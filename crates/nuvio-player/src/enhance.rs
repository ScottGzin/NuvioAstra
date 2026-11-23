//! "Melhorar imagem" and motion, done by mpv on the GPU, with a watchdog.
//!
//! Levels:
//! - Movies: stronger debanding, sharper scaler, better HDR → SDR tone mapping.
//! - Anime fast / quality: Anime4K v4 in six modes, plus effects and colors usable with any level
//!   (`picture.rs`). Restore always runs, so it also works for 1080p on a 1080p screen; the
//!   upscalers only run when the output is at least 1.2× the video (their own `//!WHEN`). Above
//!   1080p no shader is loaded (too heavy; the choice is kept for the next video). While Anime is
//!   on it replaces RTX Video Super Resolution: both together ran Anime4K at 4K and dropped 18% of
//!   the frames.
//!
//! Motion (`display-resample` + `interpolation`) needs a present that waits for the monitor. In a
//! child window the D3D11 flip model does not wait (measured on an RTX 5050 at 165 Hz: video at
//! 3.0× speed, 12 s away from the audio after a few seconds; Vulkan is unthrottled too). The blit
//! model (`d3d11-flip=no`) waits (1.000×), so motion always uses it. It cannot output HDR, which
//! is why it is used only for motion. A minimized window never paces the video: motion is
//! suspended meanwhile.
//!
//! The `Tuner` lives on the player's event thread: a change can recreate mpv's video output, which
//! must not block the window's thread.

use crate::mpv::Mpv;
use crate::picture::{self, Picture, Weight};
use crate::PlayerError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, IsIconic, IsWindowVisible, GA_ROOT};

/// Options a level may change; their start values come back when the level is left.
const TUNED: &[&str] = &[
    "deband", "deband-iterations", "deband-threshold", "deband-range", "deband-grain", "scale",
    "tone-mapping", "hdr-compute-peak", "hdr-contrast-recovery",
    "brightness", "contrast", "saturation", "gamma", "hue",
];
const MOVIES: &[(&str, &str)] = &[
    ("deband", "yes"),
    ("deband-iterations", "4"),
    ("deband-threshold", "64"),
    ("deband-range", "20"),
    ("deband-grain", "24"),
    ("scale", "ewa_lanczossharp"),
    ("tone-mapping", "spline"),
    ("hdr-compute-peak", "yes"),
    ("hdr-contrast-recovery", "0.30"),
];

/// Wider sources get no Anime4K.
const MAX_ANIME_WIDTH: i64 = 2048;
const POLL: Duration = Duration::from_millis(500);
/// After a seek, a switch or a video output restart.
const MOTION_GRACE: Duration = Duration::from_secs(2);
/// Also covers the first compilation of the shaders.
const DROPS_GRACE: Duration = Duration::from_secs(6);
const DROPS_WINDOW: Duration = Duration::from_secs(5);
const MAX_DROP_SHARE: f64 = 0.12;
const MAX_AV_OFFSET: f64 = 0.25;
const MAX_RATE_EXCESS: f64 = 1.2;
const BAD_POLLS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Motion {
    #[default]
    Off,
    /// Even cadence, the original look.
    Steady,
    /// Neighboring frames blended up to the monitor's refresh rate.
    Fluid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Enhancement {
    #[default]
    Off,
    Movies,
    AnimeFast,
    AnimeQuality,
}

impl Motion {
    pub fn name(self) -> &'static str {
        match self {
            Motion::Off => "off",
            Motion::Steady => "steady",
            Motion::Fluid => "fluid",
        }
    }

    pub fn from_name(name: &str) -> Self {
        [Motion::Steady, Motion::Fluid].into_iter().find(|m| m.name() == name).unwrap_or_default()
    }

    /// Every level sets every property, so switching never leaves values of the previous one.
    fn properties(self) -> [(&'static str, &'static str); 4] {
        let (sync, interpolation, tscale, threshold) = match self {
            Motion::Off => ("audio", "no", "oversample", "0.01"),
            Motion::Steady => ("display-resample", "yes", "oversample", "0.01"),
            // -1: blend also when the monitor rate is a multiple of the video's.
            Motion::Fluid => ("display-resample", "yes", "mitchell", "-1"),
        };
        [("video-sync", sync), ("interpolation", interpolation), ("tscale", tscale), ("interpolation-threshold", threshold)]
    }
}

impl Enhancement {
    pub fn name(self) -> &'static str {
        match self {
            Enhancement::Off => "off",
            Enhancement::Movies => "movies",
            Enhancement::AnimeFast => "animeFast",
            Enhancement::AnimeQuality => "animeQuality",
        }
    }

    pub fn from_name(name: &str) -> Self {
        [Enhancement::Movies, Enhancement::AnimeFast, Enhancement::AnimeQuality].into_iter().find(|e| e.name() == name).unwrap_or_default()
    }

    /// The level the watchdog falls back to.
    fn step_down(self) -> Self {
        match self {
            Enhancement::AnimeQuality => Enhancement::AnimeFast,
            _ => Enhancement::Off,
        }
    }

    fn weight(self) -> Option<Weight> {
        match self {
            Enhancement::AnimeFast => Some(Weight::Fast),
            Enhancement::AnimeQuality => Some(Weight::Quality),
            Enhancement::Off | Enhancement::Movies => None,
        }
    }
}

/// mpv loads shaders only from files: they are written to the app's cache.
fn write_shaders(list: &[(String, String)]) -> std::io::Result<Vec<String>> {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let dir = base.join("NuvioNextGen").join("Cache").join("shaders").join("anime4k-v4");
    let mut files = Vec::new();
    for (name, text) in list {
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(name);
        if std::fs::read_to_string(&path).map_or(true, |current| current != *text) {
            std::fs::write(&path, text)?;
        }
        files.push(path.to_string_lossy().into_owned());
    }
    Ok(files)
}

fn motion_is_broken(rate: f64, speed: f64, av_offset: f64) -> bool {
    av_offset.abs() > MAX_AV_OFFSET || rate > speed * MAX_RATE_EXCESS
}

fn drops_too_many(dropped: i64, fps: f64, seconds: f64) -> bool {
    dropped as f64 > fps.max(1.0) * seconds * MAX_DROP_SHARE
}

/// What the watchdog changed on its own.
pub(crate) enum Lowered {
    Motion,
    Enhancement(Enhancement),
    Picture(Picture),
}

pub(crate) struct Tuner {
    mpv: Arc<Mpv>,
    container: isize,
    base: Vec<(&'static str, String)>,
    /// `vf` with and without RTX Video Super Resolution, when that is on.
    rtx_filters: Option<(&'static str, &'static str)>,
    /// File names of the shaders in use.
    loaded: Vec<String>,
    without_vsr: bool,
    picture: Picture,
    /// "Segurar para comparar": the original picture meanwhile.
    compare: bool,
    motion: Motion,
    enhancement: Enhancement,
    blit: bool,
    suspended: bool,
    next_poll: Instant,
    motion_after: Instant,
    drops_after: Instant,
    last_position: Option<(Instant, f64)>,
    bad_polls: u32,
    drops: Option<(Instant, i64)>,
}

impl Tuner {
    /// After `mpv_initialize`, before the file is loaded.
    pub(crate) fn new(mpv: Arc<Mpv>, container: isize, rtx_filters: Option<(&'static str, &'static str)>) -> Self {
        let base = TUNED.iter().filter_map(|&name| mpv.get_string(name).map(|value| (name, value))).collect();
        let now = Instant::now();
        Self {
            mpv,
            container,
            base,
            rtx_filters,
            loaded: Vec::new(),
            without_vsr: false,
            picture: Picture::default(),
            compare: false,
            motion: Motion::Off,
            enhancement: Enhancement::Off,
            blit: false,
            suspended: false,
            next_poll: now,
            motion_after: now,
            drops_after: now,
            last_position: None,
            bad_polls: 0,
            drops: None,
        }
    }

    /// Playback restarted: the video's size is known now.
    pub(crate) fn playback_restarted(&mut self) {
        self.restart();
        if let Err(error) = self.load_shaders() {
            eprintln!("[player] anime shaders: {error}");
        }
    }

    /// Returns whether the shader list changed.
    fn load_shaders(&mut self) -> Result<bool, PlayerError> {
        let size = self.mpv.get_i64("width").zip(self.mpv.get_i64("height"));
        let plain = self.compare || size.is_some_and(|(width, _)| width > MAX_ANIME_WIDTH);
        let weight = self.enhancement.weight().filter(|_| !plain);
        let list = if plain {
            Vec::new()
        } else {
            // A cropped 1920×800 is still 1080p.
            picture::chain(&self.picture, weight, size.map(|(width, height)| height.max(width * 9 / 16)))
        };
        if let Some((with_vsr, without_vsr)) = self.rtx_filters {
            if weight.is_some() != self.without_vsr {
                self.mpv.set_property("vf", if weight.is_some() { without_vsr } else { with_vsr })?;
                self.without_vsr = weight.is_some();
            }
        }
        if list.iter().map(|(name, _)| name).eq(self.loaded.iter()) {
            return Ok(false);
        }
        let files = write_shaders(&list).map_err(|e| PlayerError::Mpv(format!("shader file: {e}")))?;
        self.mpv.command(&["change-list", "glsl-shaders", "clr", ""])?;
        for file in &files {
            self.mpv.command(&["change-list", "glsl-shaders", "append", file])?;
        }
        self.loaded = list.into_iter().map(|(name, _)| name).collect();
        Ok(true)
    }

    /// Playback restarted (load, seek, video output recreated): measurements start again.
    pub(crate) fn restart(&mut self) {
        let now = Instant::now();
        self.motion_after = now + MOTION_GRACE;
        self.drops_after = now + DROPS_GRACE;
        self.clear_samples();
    }

    fn clear_samples(&mut self) {
        self.last_position = None;
        self.bad_polls = 0;
        self.drops = None;
    }

    fn set_motion_properties(&self, motion: Motion) -> Result<(), PlayerError> {
        for (name, value) in motion.properties() {
            self.mpv.set_property(name, value)?;
        }
        Ok(())
    }

    pub(crate) fn set_motion(&mut self, motion: Motion) -> Result<(), PlayerError> {
        if motion != Motion::Off && !self.blit {
            self.mpv.set_property("gpu-api", "d3d11")?;
            self.mpv.set_property("d3d11-flip", "no")?;
            // Recreates the video output with the new swapchain (kept until this video closes).
            self.mpv.set_property("vo", "gpu-next,gpu")?;
            self.blit = true;
        }
        self.set_motion_properties(if self.suspended { Motion::Off } else { motion })?;
        self.motion = motion;
        self.restart();
        Ok(())
    }

    pub(crate) fn set_enhancement(&mut self, level: Enhancement) -> Result<(), PlayerError> {
        self.enhancement = level;
        self.load_shaders()?;
        self.apply_properties()?;
        self.restart();
        Ok(())
    }

    pub(crate) fn set_picture(&mut self, picture: Picture) -> Result<(), PlayerError> {
        self.picture = picture.sanitized();
        let reloaded = self.load_shaders()?;
        self.apply_properties()?;
        if reloaded {
            self.restart();
        }
        Ok(())
    }

    pub(crate) fn set_compare(&mut self, compare: bool) -> Result<(), PlayerError> {
        self.compare = compare;
        self.load_shaders()?;
        self.apply_properties()?;
        self.restart();
        Ok(())
    }

    /// Every tuned option: its start value, then the level, the banding level and the colors.
    fn apply_properties(&self) -> Result<(), PlayerError> {
        let mut values: Vec<(&'static str, String)> = self.base.clone();
        let mut put = |name: &'static str, value: String| match values.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => values.push((name, value)),
        };
        if !self.compare {
            if self.enhancement == Enhancement::Movies {
                for (name, value) in MOVIES {
                    put(name, (*value).to_string());
                }
            }
            for (name, value) in self.picture.debanding().into_iter().flatten() {
                put(name, value.to_string());
            }
            for (name, value) in self.picture.colors() {
                put(name, value.to_string());
            }
        }
        for (name, value) in &values {
            self.mpv.set_property(name, value)?;
        }
        Ok(())
    }

    fn on_screen(&self) -> bool {
        // SAFETY: plain queries; a destroyed window just answers false.
        unsafe {
            let root = GetAncestor(HWND(self.container as *mut _), GA_ROOT);
            IsWindowVisible(root).as_bool() && !IsIconic(root).as_bool()
        }
    }

    pub(crate) fn poll(&mut self) -> Option<Lowered> {
        let now = Instant::now();
        if now < self.next_poll || (self.motion == Motion::Off && self.enhancement == Enhancement::Off && self.loaded.is_empty()) {
            return None;
        }
        self.next_poll = now + POLL;

        let on_screen = self.on_screen();
        if self.motion != Motion::Off && on_screen == self.suspended {
            self.suspended = !on_screen;
            if let Err(error) = self.set_motion_properties(if on_screen { self.motion } else { Motion::Off }) {
                eprintln!("[player] motion suspend/resume: {error}");
            }
            self.restart();
            return None;
        }
        // `core-idle`: paused, buffering, seeking or ended.
        if !on_screen || self.mpv.get_flag("core-idle").unwrap_or(true) {
            self.clear_samples();
            return None;
        }
        self.check_motion(now).or_else(|| self.check_drops(now))
    }

    fn check_motion(&mut self, now: Instant) -> Option<Lowered> {
        if self.motion == Motion::Off || self.suspended || now < self.motion_after {
            return None;
        }
        let position = self.mpv.get_f64("time-pos")?;
        let (then, before) = self.last_position.replace((now, position))?;
        let rate = (position - before) / now.duration_since(then).as_secs_f64().max(0.001);
        let speed = self.mpv.get_f64("speed").unwrap_or(1.0);
        let offset = self.mpv.get_f64("avsync").unwrap_or(0.0);
        self.bad_polls = if motion_is_broken(rate, speed, offset) { self.bad_polls + 1 } else { 0 };
        if self.bad_polls < BAD_POLLS {
            return None;
        }
        eprintln!("[player] display sync not working here (rate {rate:.2}, A/V {offset:.2} s): motion off");
        // The video ran ahead: return to where the sound is, otherwise it freezes until audio catches up.
        let heard = self.mpv.get_f64("audio-pts");
        if let Err(error) = self.set_motion(Motion::Off) {
            eprintln!("[player] motion off: {error}");
        }
        if let Some(seconds) = heard {
            let _ = self.mpv.command(&["seek", &format!("{seconds:.3}"), "absolute+exact"]);
        }
        Some(Lowered::Motion)
    }

    fn check_drops(&mut self, now: Instant) -> Option<Lowered> {
        if self.compare || (self.enhancement == Enhancement::Off && self.loaded.is_empty()) || now < self.drops_after {
            return None;
        }
        let total = self.mpv.get_i64("frame-drop-count")?;
        let (since, start) = *self.drops.get_or_insert((now, total));
        let seconds = now.duration_since(since).as_secs_f64();
        if seconds < DROPS_WINDOW.as_secs_f64() {
            return None;
        }
        self.drops = Some((now, total));
        let fps = self.mpv.get_f64("container-fps").unwrap_or(24.0);
        if !drops_too_many(total - start, fps, seconds) {
            return None;
        }
        // The effects go first: the level is what matters most.
        if self.picture.has_effects() {
            let lighter = self.picture.without_effects();
            eprintln!("[player] {} frames dropped in {seconds:.0} s: effects off", total - start);
            return self.set_picture(lighter).map(|()| Lowered::Picture(lighter)).map_err(|e| eprintln!("[player] effects not lowered: {e}")).ok();
        }
        let lower = self.enhancement.step_down();
        eprintln!("[player] {} frames dropped in {seconds:.0} s: image improvement lowered to {}", total - start, lower.name());
        match self.set_enhancement(lower) {
            Ok(()) => Some(Lowered::Enhancement(lower)),
            Err(error) => {
                eprintln!("[player] image improvement not lowered: {error}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchdog_rules() {
        assert!(motion_is_broken(3.008, 1.0, -12.67), "the reported bug");
        assert!(motion_is_broken(22.33, 1.0, 0.0), "too fast with A/V looking fine");
        assert!(!motion_is_broken(1.001, 1.0, 0.004));
        assert!(!motion_is_broken(2.01, 2.0, 0.01), "2x chosen by the user");
        assert!(drops_too_many(30, 23.976, 5.0));
        assert!(!drops_too_many(3, 23.976, 5.0));
        assert_eq!(Enhancement::AnimeQuality.step_down(), Enhancement::AnimeFast);
        assert_eq!(Enhancement::AnimeFast.step_down(), Enhancement::Off);
        assert_eq!(Enhancement::Movies.step_down(), Enhancement::Off);
    }

    #[test]
    fn names_round_trip_and_shaders_are_embedded() {
        for level in [Enhancement::Off, Enhancement::Movies, Enhancement::AnimeFast, Enhancement::AnimeQuality] {
            assert_eq!(Enhancement::from_name(level.name()), level);
            let list = picture::chain(&Picture::default(), level.weight(), Some(1080));
            assert!(list.iter().all(|(_, text)| text.contains("//!HOOK")));
            assert_eq!(list.is_empty(), level.weight().is_none());
        }
        for motion in [Motion::Off, Motion::Steady, Motion::Fluid] {
            assert_eq!(Motion::from_name(motion.name()), motion);
        }
    }
}
