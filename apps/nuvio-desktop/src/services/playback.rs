//! Applies the profile's player settings to a playback started from the UI (`stream_play`),
//! the way the original does it:
//! - video options (decoder priority, RTX VSR/HDR) go into the session's `VideoConfig`;
//! - resize mode and subtitle style are applied right after the session opens;
//! - audio/subtitle language preferences become an ordered target list
//!   (`resolvePreferredAudioLanguageTargets` / `resolvePreferredSubtitleLanguageTargets`), `alang`
//!   is set (`applyAudioLanguagePreferences`), and once the file's tracks are known the first track
//!   matching a target is selected (`languageMatchesPreference`: same code or same primary code).
//!
//! The automated player self-tests do not go through here, so their expectations (default style,
//! default decoder) do not depend on the user's settings.

use super::language_aliases::LANGUAGE_CODE_ALIASES;
use nuvio_player::{DecoderPriority, Enhancement, Motion, Picture, PlayerCommand, PlayerSnapshot, ResizeMode, SubtitleStyle, VideoConfig};
use nuvio_store::settings::PlayerSettings;
use std::time::{Duration, Instant};

/// `normalizeLanguageCode` (code path): trim, `_`→`-`, lowercase, alias table, then primary-code
/// alias with the region suffix kept.
pub fn normalize_language(code: &str) -> Option<String> {
    let raw = code.trim().replace('_', "-").to_lowercase();
    if raw.is_empty() {
        return None;
    }
    let alias = |k: &str| LANGUAGE_CODE_ALIASES.iter().find(|(a, _)| *a == k).map(|(_, v)| v.replace('_', "-").to_lowercase());
    if let Some(v) = alias(&raw) {
        return Some(v);
    }
    let (primary, suffix) = match raw.split_once('-') {
        Some((p, s)) => (p.to_string(), s.to_string()),
        None => (raw.clone(), String::new()),
    };
    let primary_alias = alias(&primary);
    Some(if suffix.is_empty() {
        primary_alias.unwrap_or(primary)
    } else {
        match primary_alias {
            Some(a) if !a.contains('-') => format!("{a}-{suffix}"),
            Some(a) => a,
            None => format!("{primary}-{suffix}"),
        }
    })
}

/// `languageMatchesPreference`.
pub fn language_matches(track: Option<&str>, target: &str) -> bool {
    let (Some(t), Some(g)) = (track.and_then(normalize_language), normalize_language(target)) else { return false };
    t == g || t.split('-').next() == g.split('-').next()
}

fn distinct(items: impl IntoIterator<Item = Option<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items.into_iter().flatten() {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

/// `resolvePreferredAudioLanguageTargets`. The content's original language is not known to
/// NextGen yet, so `original` falls back to the device languages, as the original does when the
/// original language is unknown.
pub fn audio_targets(settings: &PlayerSettings, device: &[String]) -> Vec<String> {
    let norm = |v: Option<&str>| {
        v.and_then(normalize_language).filter(|c| !matches!(c.as_str(), "default" | "device" | "none" | "forced" | "original"))
    };
    let secondary = || norm(settings.secondary_preferred_audio_language.as_deref());
    let primary = normalize_language(&settings.preferred_audio_language).unwrap_or_else(|| "device".into());
    match primary.as_str() {
        "default" => distinct([secondary()]),
        "device" | "original" => distinct(device.iter().map(|d| norm(Some(d))).chain([secondary()])),
        _ => distinct([norm(Some(&settings.preferred_audio_language)), secondary()]),
    }
}

/// `resolvePreferredSubtitleLanguageTargets` (`forced` stays a target: it selects a forced track).
pub fn subtitle_targets(settings: &PlayerSettings, device: &[String]) -> Vec<String> {
    let norm = |v: Option<&str>| v.and_then(normalize_language).filter(|c| !matches!(c.as_str(), "none" | "default"));
    let secondary = || norm(settings.secondary_preferred_subtitle_language.as_deref());
    let primary = normalize_language(&settings.preferred_subtitle_language).unwrap_or_else(|| "none".into());
    match primary.as_str() {
        "none" => distinct([secondary()]),
        "device" => distinct(device.iter().map(|d| norm(Some(d))).chain([secondary()])),
        _ => distinct([norm(Some(&settings.preferred_subtitle_language)), secondary()]),
    }
}

/// Track ids to select once the tracks are known: (audio, subtitle). `None` = leave as is.
pub fn choose_tracks(snapshot: &PlayerSnapshot, audio: &[String], subtitles: &[String]) -> (Option<String>, Option<String>) {
    let audio_tracks: Vec<_> = snapshot.tracks.iter().filter(|t| t.kind == "audio").collect();
    let sub_tracks: Vec<_> = snapshot.tracks.iter().filter(|t| t.kind == "sub").collect();
    let audio_id = audio
        .iter()
        .find_map(|target| audio_tracks.iter().find(|t| language_matches(t.lang.as_deref(), target)))
        .map(|t| t.id.to_string());
    let forced = sub_tracks.iter().find(|t| t.forced).map(|t| t.id.to_string());
    let matched = subtitles.iter().find_map(|target| {
        if target == "forced" {
            sub_tracks.iter().find(|t| t.forced)
        } else {
            sub_tracks.iter().find(|t| !t.forced && language_matches(t.lang.as_deref(), target))
        }
    });
    // No preferred subtitle: forced subtitles only (`useForcedSubtitles` default on), else off.
    let sub_id = matched.map(|t| t.id.to_string()).or(forced).or_else(|| (!sub_tracks.is_empty()).then(|| "no".to_string()));
    (audio_id, sub_id)
}

pub fn video_config(settings: &PlayerSettings) -> VideoConfig {
    VideoConfig {
        decoder_priority: match settings.decoder_priority {
            0 => DecoderPriority::HardwareOnly,
            2 => DecoderPriority::SoftwareOnly,
            _ => DecoderPriority::HardwarePreferred,
        },
        nvidia_rtx_super_resolution: settings.nvidia_rtx_super_resolution,
        nvidia_rtx_hdr: settings.nvidia_rtx_hdr,
        motion: Motion::from_name(&settings.motion_smoothing),
        enhancement: Enhancement::from_name(&settings.video_enhancement),
        picture: picture(settings),
        audio_passthrough: settings.audio_passthrough,
        subtitle_codepage: settings.subtitle_codepage.clone(),
    }
}

/// The player menu and the watchdog both save in the profile (as the choice for the next videos).
pub fn save_improvements(store: &nuvio_store::nextgen::AppStore, profile: i64, motion: Option<Motion>, enhancement: Option<Enhancement>, picture: Option<Picture>) {
    let result = store.update_profile_settings(profile, |settings| {
        // The two structs share their camelCase names.
        if let Some(value) = picture.and_then(|p| serde_json::to_value(p).ok()).and_then(|v| serde_json::from_value(v).ok()) {
            settings.player.picture = value;
        }
        if let Some(motion) = motion {
            settings.player.motion_smoothing = motion.name().into();
        }
        if let Some(enhancement) = enhancement {
            settings.player.video_enhancement = enhancement.name().into();
        }
    });
    if let Err(error) = result {
        eprintln!("[player] improvements not saved: {error}");
    }
}

/// Live TV and trailers: the video as it comes. `display-resample` follows the display's clock,
/// which would drift from a live stream's.
pub fn video_config_unenhanced(settings: &PlayerSettings) -> VideoConfig {
    VideoConfig { motion: Motion::Off, enhancement: Enhancement::Off, picture: Picture::default(), ..video_config(settings) }
}

pub fn resize_mode(settings: &PlayerSettings) -> ResizeMode {
    match settings.resize_mode.as_str() {
        "Fill" => ResizeMode::Fill,
        "Zoom" => ResizeMode::Zoom,
        "Stretch" => ResizeMode::Stretch,
        _ => ResizeMode::Fit,
    }
}

fn picture(settings: &PlayerSettings) -> Picture {
    serde_json::to_value(&settings.picture).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

/// `font`: `fonts::mpv_font` of the chosen id.
pub fn subtitle_style(settings: &PlayerSettings, font: (String, Option<String>)) -> SubtitleStyle {
    let s = &settings.subtitle;
    SubtitleStyle {
        text_color: s.text_color.clone(),
        background_color: s.background_color.clone(),
        outline_color: s.outline_color.clone(),
        outline_enabled: s.outline_enabled,
        outline_width: s.outline_width as f64,
        bold: s.bold,
        font_size_sp: s.font_size_sp as f64,
        bottom_offset: s.bottom_offset,
        strip_sdh: s.strip_sdh,
        shadow_enabled: s.shadow_enabled,
        shadow_opacity: s.shadow_opacity,
        shadow_size: s.shadow_size as f64,
        italic: s.italic,
        spacing: s.spacing as f64,
        blur: s.blur as f64,
        align: s.align.clone(),
        override_ass: s.override_ass,
        font: font.0,
        fonts_dir: font.1,
        ..SubtitleStyle::default()
    }
}

/// Commands applied right after the session opens.
pub fn startup_commands(settings: &PlayerSettings, audio: &[String], font: (String, Option<String>)) -> Vec<PlayerCommand> {
    let mut commands = vec![
        PlayerCommand::SetResizeMode { mode: resize_mode(settings) },
        PlayerCommand::ApplySubtitleStyle { style: subtitle_style(settings, font) },
    ];
    if !audio.is_empty() {
        commands.push(PlayerCommand::SetAudioLanguages { languages: audio.to_vec() });
    }
    // Audio gain: each session starts at 0 dB unless the user asked to keep it (`persistAudioAmplification`).
    commands.push(PlayerCommand::SetAudioAmplification {
        db: if settings.persist_audio_amplification { settings.audio_amplification_db } else { 0 },
    });
    // The original's `effectiveDownmixEnabled`: only with software decoding.
    if settings.downmix_enabled && settings.decoder_priority == 2 {
        let layout = if settings.audio_output_channels == "2.0" { "stereo" } else { settings.audio_output_channels.as_str() };
        commands.push(PlayerCommand::SetAudioDownmix { layout: layout.to_string() });
    }
    if settings.buffer_enabled {
        commands.push(PlayerCommand::SetBuffer {
            max_seconds: settings.max_buffer_ms as f64 / 1000.0,
            back_seconds: settings.back_buffer_duration_ms as f64 / 1000.0,
            max_bytes: settings.target_buffer_size_mb * 1024 * 1024,
        });
    }
    commands
}

/// User locale (`Locale.getDefault().toLanguageTag()` of the original on Windows; `LANG` and
/// friends on Linux), normalised to the BCP-47 style the audio/subtitle preferences expect.
#[cfg(windows)]
pub fn device_languages() -> Vec<String> {
    let mut buffer = [0u16; 85];
    // SAFETY: the buffer is LOCALE_NAME_MAX_LENGTH wide, as the API requires.
    let len = unsafe { windows::Win32::Globalization::GetUserDefaultLocaleName(&mut buffer) };
    if len <= 1 {
        return Vec::new();
    }
    vec![String::from_utf16_lossy(&buffer[..(len as usize - 1)])]
}

#[cfg(not(windows))]
pub fn device_languages() -> Vec<String> {
    let mut out = Vec::new();
    for key in ["LC_ALL", "LANGUAGE", "LANG"] {
        let Ok(value) = std::env::var(key) else { continue };
        let value = value.trim();
        if value.is_empty() || value == "C" || value == "POSIX" {
            continue;
        }
        // "pt_PT.UTF-8" → "pt-PT"; "pt:en" → "pt", "en".
        for part in value.split([':', ',']) {
            let tag = part.split('.').next().unwrap_or(part).replace('_', "-");
            if !tag.is_empty() && !out.contains(&tag) {
                out.push(tag);
            }
        }
    }
    out
}

/// Waits (up to `timeout`) for the tracks of `session`, then selects the preferred ones once.
pub fn select_tracks_when_loaded(
    snapshot: impl Fn() -> Option<PlayerSnapshot>,
    execute: impl Fn(PlayerCommand),
    session: u64,
    audio: Vec<String>,
    subtitles: Vec<String>,
    timeout: Duration,
) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        match snapshot() {
            Some(s) if s.session != session => return, // another session took over
            Some(s) if s.loaded && !s.tracks.is_empty() => {
                let (a, sub) = choose_tracks(&s, &audio, &subtitles);
                if let Some(id) = a.filter(|id| s.aid.as_deref() != Some(id)) {
                    execute(PlayerCommand::SetAudioTrack { id });
                }
                if let Some(id) = sub.filter(|id| s.sid.as_deref().unwrap_or("no") != id) {
                    execute(PlayerCommand::SetSubtitleTrack { id });
                }
                return;
            }
            None => return,
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_player::Track;

    fn track(id: i64, kind: &str, lang: &str, forced: bool) -> Track {
        Track { id, kind: kind.into(), title: None, lang: Some(lang.into()), codec: None, selected: false, external: false, default: false, forced }
    }

    #[test]
    fn language_codes_normalize_like_the_original() {
        assert_eq!(normalize_language("pt_BR").as_deref(), Some("pt-br"));
        assert_eq!(normalize_language("por").as_deref(), Some("pt"));
        assert_eq!(normalize_language("eng").as_deref(), Some("en"));
        assert_eq!(normalize_language("en-US").as_deref(), Some("en-us"));
        assert!(language_matches(Some("por"), "pt-br"));
        assert!(language_matches(Some("eng"), "en"));
        assert!(!language_matches(Some("spa"), "pt-br"));
    }

    #[test]
    fn targets_follow_the_original_rules() {
        let device = vec!["pt-BR".to_string()];
        let mut s = PlayerSettings::default(); // audio: device, subtitles: none
        assert_eq!(audio_targets(&s, &device), vec!["pt-br"]);
        assert!(subtitle_targets(&s, &device).is_empty());
        s.preferred_audio_language = "default".into();
        s.secondary_preferred_audio_language = Some("en".into());
        assert_eq!(audio_targets(&s, &device), vec!["en"]);
        s.preferred_audio_language = "ja".into();
        assert_eq!(audio_targets(&s, &device), vec!["ja", "en"]);
        s.preferred_subtitle_language = "device".into();
        assert_eq!(subtitle_targets(&s, &device), vec!["pt-br"]);
        s.preferred_subtitle_language = "forced".into();
        assert_eq!(subtitle_targets(&s, &device), vec!["forced"]);
    }

    #[test]
    fn tracks_are_chosen_by_preference_order() {
        let snapshot = PlayerSnapshot {
            loaded: true,
            tracks: vec![
                track(1, "audio", "eng", false),
                track(2, "audio", "por", false),
                track(1, "sub", "eng", false),
                track(2, "sub", "por", true),
                track(3, "sub", "por", false),
            ],
            ..Default::default()
        };
        assert_eq!(choose_tracks(&snapshot, &["pt-br".into()], &["pt-br".into()]), (Some("2".into()), Some("3".into())));
        assert_eq!(choose_tracks(&snapshot, &["fr".into()], &[]), (None, Some("2".into()))); // forced only
        let no_forced = PlayerSnapshot { tracks: vec![track(1, "sub", "eng", false)], ..snapshot.clone() };
        assert_eq!(choose_tracks(&no_forced, &[], &[]).1.as_deref(), Some("no"));
    }
}
