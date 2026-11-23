//! One playback session embedded in a host window (Windows x64).
//!
//! Window model (same as the shipping `player_bridge.cpp`): a container child window is created
//! in the host window at the bottom of the z-order; mpv receives it as `wid` and creates its own
//! video window inside; the UI webview (a sibling with a transparent background) overlays it.
//! The video area is set with `video-margin-ratio-*`, so preview ⇄ fullscreen and resizing never
//! recreate the player, and picture-in-picture moves the container into its own window.
//!
//! Every feature mirrors the original Windows player (`NativePlayerController.kt` +
//! `player_bridge.cpp`); differences are documented on each item.
//!
//! Threading: `open`, `shutdown`, `fit_to_host` and the PiP calls create, move or destroy Win32
//! windows and must run on the host window's thread (the Tauri main thread). Property commands are
//! thread-safe (mpv client API).

use crate::enhance::{Enhancement, Lowered, Motion, Tuner};
use crate::picture::Picture;
use crate::mpv::*;
use crate::{win, PlayerError};
use serde::{Deserialize, Serialize};
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetClassNameW, SetWindowPos, SetWindowTextW, HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
};

/// Decoder preference, as `decoderPriority` in the original app (0 / 1 / 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DecoderPriority {
    /// 0: hardware decoding, no software fallback.
    HardwareOnly,
    /// 1 (default): hardware decoding with software fallback.
    #[default]
    HardwarePreferred,
    /// 2: software decoding only.
    SoftwareOnly,
}

/// Per-session video configuration (the original player's settings).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoConfig {
    pub decoder_priority: DecoderPriority,
    /// NVIDIA RTX Video Super Resolution (`d3d11vpp` scaling-mode=nvidia).
    pub nvidia_rtx_super_resolution: bool,
    /// NVIDIA RTX Video HDR (`d3d11vpp` nvidia-true-hdr).
    pub nvidia_rtx_hdr: bool,
    pub motion: Motion,
    pub enhancement: Enhancement,
    pub picture: Picture,
}

/// What to play.
#[derive(Debug, Clone)]
pub struct LoadRequest {
    pub url: String,
    /// HTTP request headers for the stream (addon `behaviorHints.proxyHeaders.request`).
    pub headers: Vec<(String, String)>,
    pub start_seconds: f64,
    pub paused: bool,
    pub video: VideoConfig,
    /// Separate audio of the same media (YouTube trailers: MP4 video + M4A audio), as the
    /// original's `activeSourceAudioUrl`.
    pub audio_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResizeMode {
    #[default]
    Fit,
    Fill,
    Zoom,
    Stretch,
}

/// Subtitle style, as `SubtitleStyleState` mapped by `NativePlayerController.applySubtitleStyle`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleStyle {
    /// `#AARRGGBB`.
    pub text_color: String,
    pub background_color: String,
    pub outline_color: String,
    pub outline_enabled: bool,
    pub outline_width: f64,
    pub bold: bool,
    /// UI size in sp; mpv size = sp × 3, clamped 18–96.
    pub font_size_sp: f64,
    /// UI bottom offset; mpv `sub-pos` = 100 − offset / 2, clamped 0–150.
    pub bottom_offset: i64,
    pub strip_sdh: bool,
    pub use_libass: bool,
    /// Formatted (ASS) subtitles get this style too, instead of their own.
    pub override_ass: bool,
    pub italic: bool,
    /// Extra space between letters.
    pub spacing: f64,
    /// Softened edge.
    pub blur: f64,
    /// `left`, `center` or `right`.
    pub align: String,
    pub font: String,
    pub fonts_dir: Option<String>,
    pub shadow_enabled: bool,
    /// 0–100.
    pub shadow_opacity: i64,
    pub shadow_size: f64,
}

impl Default for SubtitleStyle {
    /// Defaults of `SubtitleStyleState` (white text, black outline, 18 sp, offset 20).
    fn default() -> Self {
        Self {
            text_color: "#FFFFFFFF".into(),
            background_color: "#00000000".into(),
            outline_color: "#FF000000".into(),
            outline_enabled: true,
            outline_width: 2.0,
            bold: false,
            font_size_sp: 18.0,
            bottom_offset: 20,
            strip_sdh: false,
            use_libass: false,
            override_ass: false,
            italic: false,
            spacing: 0.0,
            blur: 0.0,
            align: "center".into(),
            font: "sans-serif".into(),
            fonts_dir: None,
            shadow_enabled: false,
            shadow_opacity: 0,
            shadow_size: 0.0,
        }
    }
}

const SUBTITLE_BOX_PADDING: f64 = 4.0;

impl SubtitleStyle {
    pub fn mpv_font_size(&self) -> f64 {
        (self.font_size_sp * 3.0).clamp(18.0, 96.0)
    }
    pub fn mpv_sub_pos(&self) -> i64 {
        (100 - self.bottom_offset / 2).clamp(0, 150)
    }
    pub fn mpv_outline_size(&self) -> f64 {
        if self.outline_enabled { self.outline_width.clamp(0.0, 8.0) } else { 0.0 }
    }
    pub fn mpv_shadow_color(&self) -> String {
        let alpha = if self.shadow_enabled { (self.shadow_opacity.clamp(0, 100) * 255 / 100) as u8 } else { 0 };
        format!("#{alpha:02X}000000")
    }
    /// The background box's color, when there is one.
    pub fn mpv_box(&self) -> Option<&str> {
        let color = self.background_color.as_str();
        (color.len() == 9 && !color.starts_with("#00")).then_some(color)
    }

    pub fn mpv_shadow_offset(&self) -> f64 {
        if self.shadow_enabled { self.shadow_size } else { 0.0 }
    }
}

pub const SUBTITLE_DELAY_LIMIT_MS: i64 = 60_000;
pub const AUDIO_DELAY_LIMIT_MS: i64 = 60_000;
pub const SPEED_MIN: f64 = 0.25;
pub const SPEED_MAX: f64 = 4.0;
pub const VOLUME_MAX: f64 = 200.0;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: i64,
    pub kind: String,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub codec: Option<String>,
    pub selected: bool,
    pub external: bool,
    pub default: bool,
    pub forced: bool,
}

/// A chapter of the file (`chapter-list`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Chapter {
    pub time: f64,
    pub title: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    pub session: u64,
    pub loaded: bool,
    pub ended: bool,
    pub paused: bool,
    pub buffering: bool,
    pub time_pos: f64,
    pub duration: f64,
    pub cache_seconds: f64,
    pub volume: f64,
    pub mute: bool,
    pub speed: f64,
    pub sub_delay_ms: i64,
    pub audio_delay_ms: i64,
    pub sub_visible: bool,
    pub hwdec_current: Option<String>,
    pub video_codec: Option<String>,
    pub width: i64,
    pub height: i64,
    /// Display size after aspect correction (`video-out-params/dw,dh`), used for PiP.
    pub display_width: i64,
    pub display_height: i64,
    pub fps: f64,
    /// Last A/V difference in seconds (`avsync`).
    pub av_sync: f64,
    pub dropped_frames: i64,
    pub decoder_dropped_frames: i64,
    pub aid: Option<String>,
    pub sid: Option<String>,
    pub tracks: Vec<Track>,
    pub chapters: Vec<Chapter>,
    pub picture_in_picture: bool,
    /// Improvements in use (`enhance`).
    pub motion: Motion,
    pub enhancement: Enhancement,
    pub picture: Picture,
    /// How many times the watchdog lowered something in this session, and what the last time
    /// (`motion` or `enhancement`).
    pub auto_changes: u32,
    pub auto_reason: Option<&'static str>,
    /// Error text without URLs (mpv error names only).
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PlayerCommand {
    SetPaused { paused: bool },
    TogglePause,
    /// Absolute or relative seek in seconds, `+exact` like the original.
    Seek { seconds: f64, relative: bool },
    /// 0–200 %.
    SetVolume { volume: f64 },
    SetMute { mute: bool },
    /// 0.25–4.0.
    SetSpeed { speed: f64 },
    /// `"no"`, `"auto"` or a track id.
    SetAudioTrack { id: String },
    /// Preferred audio languages (`alang`), then `aid=auto` like `setAudioLanguages`.
    SetAudioLanguages { languages: Vec<String> },
    SetSubtitleTrack { id: String },
    /// `sub-add <url> select [<title> [<lang>]]`, as `addSubtitleUrl`.
    AddSubtitle {
        url: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        lang: Option<String>,
    },
    /// Removes external subtitle tracks, then selects `select_id` or disables subtitles.
    RemoveExternalSubtitles { select_id: Option<i64> },
    SetSubtitleVisible { visible: bool },
    /// ±60 000 ms.
    SetSubtitleDelay { ms: i64 },
    /// ±60 000 ms. Not exposed by the original Windows player (Android only); added on request.
    SetAudioDelay { ms: i64 },
    /// 0–10 dB of gain, expressed as mpv's `volume` percentage (100 × 10^(dB/20)).
    SetAudioAmplification { db: i64 },
    /// Downmix to an ffmpeg layout name (`stereo`, `5.1`, …); empty restores the default.
    SetAudioDownmix { layout: String },
    /// HTTP stream buffer: `cache-secs`, `demuxer-readahead-secs` and `demuxer-max-bytes`.
    SetBuffer { max_seconds: f64, back_seconds: f64, max_bytes: i64 },
    ApplySubtitleStyle { style: SubtitleStyle },
    SetResizeMode { mode: ResizeMode },
    SetMotion { motion: Motion },
    SetEnhancement { enhancement: Enhancement },
    SetPicture { picture: Picture },
    /// The original picture while `active` (never saved).
    SetCompare { active: bool },
}

/// Observed properties: (reply id, name, format).
const OBSERVED: &[(u64, &str, std::ffi::c_int)] = &[
    (1, "time-pos", FORMAT_DOUBLE),
    (2, "duration", FORMAT_DOUBLE),
    (3, "pause", FORMAT_FLAG),
    (4, "paused-for-cache", FORMAT_FLAG),
    (5, "demuxer-cache-duration", FORMAT_DOUBLE),
    (6, "volume", FORMAT_DOUBLE),
    (7, "mute", FORMAT_FLAG),
    (8, "hwdec-current", FORMAT_STRING),
    (9, "video-codec", FORMAT_STRING),
    (10, "width", FORMAT_INT64),
    (11, "height", FORMAT_INT64),
    (12, "estimated-vf-fps", FORMAT_DOUBLE),
    (13, "frame-drop-count", FORMAT_INT64),
    (14, "decoder-frame-drop-count", FORMAT_INT64),
    (15, "aid", FORMAT_STRING),
    (16, "sid", FORMAT_STRING),
    (17, "track-list", FORMAT_NONE),
    (18, "eof-reached", FORMAT_FLAG),
    (19, "speed", FORMAT_DOUBLE),
    (20, "sub-delay", FORMAT_DOUBLE),
    (21, "audio-delay", FORMAT_DOUBLE),
    (22, "sub-visibility", FORMAT_FLAG),
    (23, "avsync", FORMAT_DOUBLE),
    (24, "video-out-params/dw", FORMAT_INT64),
    (25, "video-out-params/dh", FORMAT_INT64),
    (26, "chapter-list", FORMAT_NONE),
];

/// Minimum interval between two snapshots sent to the UI (position updates are frequent).
const EMIT_INTERVAL: Duration = Duration::from_millis(250);

pub type SnapshotSink = Arc<dyn Fn(&PlayerSnapshot) + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
struct AppliedSubtitleStyle {
    use_libass: bool,
    text_color: String,
    background_color: String,
    outline_color: String,
    outline_size: f64,
    bold: bool,
    font_size: f64,
    position: i64,
    strip_sdh: bool,
}

struct PipSession {
    window: isize,
    /// `win::enter_pip`'s state.
    state: usize,
    previous_rect: Option<(f64, f64, f64, f64)>,
    previous_mode: ResizeMode,
}

/// Improvement changes asked by the UI, not applied yet.
#[derive(Default)]
struct Wanted {
    motion: Option<Motion>,
    enhancement: Option<Enhancement>,
    picture: Option<Picture>,
    compare: Option<bool>,
}

pub struct Player {
    mpv: Option<Arc<Mpv>>,
    host: isize,
    container: isize,
    stop: Arc<AtomicBool>,
    events: Option<JoinHandle<()>>,
    snapshot: Arc<Mutex<PlayerSnapshot>>,
    applied_style: Mutex<Option<AppliedSubtitleStyle>>,
    video_rect: Mutex<Option<(f64, f64, f64, f64)>>,
    resize_mode: Mutex<ResizeMode>,
    pip: Mutex<Option<PipSession>>,
    /// Changes for the `Tuner`, applied by the event thread.
    wanted: Arc<Mutex<Wanted>>,
}

fn hwnd(value: isize) -> HWND {
    HWND(value as *mut _)
}

/// Header lines `Name: value`, trimmed with blank names or values skipped (`toHeaderLines` in
/// `NativePlayerController.kt`). They are handed to mpv as a node array, one string per line.
///
/// Difference from the original: `player_bridge.cpp` joins the lines into one escaped string
/// (a comma becomes `\,` and a backslash `\\`). Measured with a local proxy, mpv unescapes `\,`
/// but not `\\`, so a header
/// value containing a backslash reaches the server doubled. The node array avoids escaping.
/// Per-file `loadfile` options; values use mpv's `%length%value` quoting (URLs contain commas).
pub fn load_options(request: &LoadRequest) -> String {
    let mut options = Vec::new();
    if request.start_seconds > 0.0 {
        options.push(format!("start={:.3}", request.start_seconds));
    }
    if let Some(audio) = request.audio_url.as_deref().filter(|a| !a.is_empty()) {
        options.push(format!("audio-files=%{}%{audio}", audio.len()));
    }
    options.join(",")
}

pub fn http_header_lines(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let (name, value) = (name.trim(), value.trim());
            (!name.is_empty() && !value.is_empty()).then(|| format!("{name}: {value}"))
        })
        .collect()
}

fn rtx_filter(super_resolution: bool, hdr: bool) -> &'static str {
    match (super_resolution, hdr) {
        (true, true) => "d3d11vpp=scale=2:scaling-mode=nvidia:format=x2bgr10:nvidia-true-hdr,format=x2bgr10:transfer=pq:primaries=bt.2020",
        (true, false) => "d3d11vpp=scale=2:scaling-mode=nvidia",
        (false, true) => "d3d11vpp=format=x2bgr10:nvidia-true-hdr,format=x2bgr10:transfer=pq:primaries=bt.2020",
        (false, false) => "",
    }
}

/// mpv options in the order the original bridge sets them (`startMpv`).
pub(crate) fn startup_options(video: &VideoConfig, unified_memory_gpu: bool, shader_cache: Option<&str>) -> Vec<(String, String)> {
    let rtx = video.nvidia_rtx_super_resolution || video.nvidia_rtx_hdr;
    let fast_profile = !rtx && unified_memory_gpu;
    let mut o: Vec<(String, String)> = Vec::new();
    let mut set = |k: &str, v: &str| o.push((k.to_string(), v.to_string()));
    set("config", "no");
    set("terminal", "no");
    set("osc", "no");
    // The UI owns all input; mpv must not react to keys or the mouse.
    set("input-default-bindings", "no");
    set("input-vo-keyboard", "no");
    set("input-cursor", "no");
    set("cursor-autohide", "no");
    set("keep-open", "yes");
    set("idle", "yes");
    set("volume-max", "200");
    set("vo", "gpu-next,gpu,");
    if rtx {
        set("gpu-api", "d3d11");
        set("hwdec", "d3d11va");
        set("d3d11-adapter", "NVIDIA");
    } else {
        set("gpu-api", "auto");
        set("hwdec", "auto");
    }
    set("hwdec-codecs", "all");
    let filters = rtx_filter(video.nvidia_rtx_super_resolution, video.nvidia_rtx_hdr);
    if !filters.is_empty() {
        set("vf", filters);
    }
    set("target-colorspace-hint", "yes");
    match video.decoder_priority {
        DecoderPriority::HardwareOnly => set("vd-lavc-software-fallback", "no"),
        DecoderPriority::SoftwareOnly => {
            set("hwdec", "no");
            set("vd-lavc-software-fallback", "yes");
        }
        DecoderPriority::HardwarePreferred => set("vd-lavc-software-fallback", "yes"),
    }
    set("vd-lavc-threads", "4");
    set("tone-mapping", "bt.2390");
    set("dither-depth", "auto");
    if fast_profile {
        set("profile", "fast");
    } else {
        set("deband", "yes");
        set("scale", "spline36");
        set("cscale", "spline36");
    }
    set("demuxer-max-bytes", "150MiB");
    set("demuxer-max-back-bytes", "50MiB");
    set("demuxer-seekable-cache", "yes");
    set("stream-lavf-o", "reconnect=1,reconnect_streamed=1,reconnect_on_network_error=1,reconnect_on_http_error=%23%408,429,500,502,503,504,reconnect_delay_max=15");
    if let Some(dir) = shader_cache {
        set("gpu-shader-cache-dir", dir);
    }
    set("background", "color");
    set("background-color", "#000000");
    o
}

/// `unifiedMemoryGpu()` of the bridge: the default D3D11 adapter reports unified memory
/// (integrated GPU), which selects mpv's `fast` profile.
pub fn unified_memory_gpu() -> bool {
    use std::sync::OnceLock;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_FLAG, D3D11_FEATURE_D3D11_OPTIONS2,
        D3D11_FEATURE_DATA_D3D11_OPTIONS2, D3D11_SDK_VERSION,
    };
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| unsafe {
        let mut device: Option<ID3D11Device> = None;
        if D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, windows::Win32::Foundation::HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), None, D3D11_SDK_VERSION, Some(&mut device), None, None).is_err() {
            return false;
        }
        let Some(device) = device else { return false };
        let mut options = D3D11_FEATURE_DATA_D3D11_OPTIONS2::default();
        device
            .CheckFeatureSupport(
                D3D11_FEATURE_D3D11_OPTIONS2,
                (&mut options as *mut D3D11_FEATURE_DATA_D3D11_OPTIONS2).cast(),
                std::mem::size_of::<D3D11_FEATURE_DATA_D3D11_OPTIONS2>() as u32,
            )
            .is_ok()
            && options.UnifiedMemoryArchitecture.as_bool()
    })
}

/// mpv shader cache for the next-gen app (kept apart from the original app's cache folder).
pub fn shader_cache_dir() -> Option<String> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let dir = std::path::PathBuf::from(base).join("NuvioNextGen").join("Cache").join("mpv-shaders");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.to_string_lossy().into_owned())
}

impl Player {
    /// Creates the container window in `host_hwnd`, the mpv core inside it, and starts loading.
    /// Must run on the host window's thread.
    pub fn open(api: Arc<MpvApi>, host_hwnd: isize, session: u64, request: &LoadRequest, sink: SnapshotSink) -> Result<Self, PlayerError> {
        let container = win::create_container(hwnd(host_hwnd)).map_err(PlayerError::Mpv)?.0 as isize;
        match Self::start(api, host_hwnd, container, session, request, sink) {
            Ok(player) => Ok(player),
            Err(error) => {
                win::destroy(hwnd(container));
                Err(error)
            }
        }
    }

    fn start(api: Arc<MpvApi>, host: isize, container: isize, session: u64, request: &LoadRequest, sink: SnapshotSink) -> Result<Self, PlayerError> {
        let mpv = Arc::new(Mpv::create(api)?);
        for (name, value) in startup_options(&request.video, unified_memory_gpu(), shader_cache_dir().as_deref()) {
            mpv.set_option(&name, &value)?;
        }
        let headers = http_header_lines(&request.headers);
        if !headers.is_empty() {
            mpv.set_option_string_list("http-header-fields", &headers)?;
        }
        mpv.set_option_i64("wid", container as i64)?;
        mpv.initialize()?;
        // Before the file loads there is no video output yet, so this is immediate. An improvement
        // that cannot be applied never stops the video from playing.
        let rtx = &request.video;
        let rtx_filters = rtx
            .nvidia_rtx_super_resolution
            .then(|| (rtx_filter(true, rtx.nvidia_rtx_hdr), rtx_filter(false, rtx.nvidia_rtx_hdr)));
        let mut tuner = Tuner::new(mpv.clone(), container, rtx_filters);
        let (mut motion, mut enhancement) = (request.video.motion, request.video.enhancement);
        if motion != Motion::Off {
            if let Err(error) = tuner.set_motion(motion) {
                eprintln!("[player] motion not applied: {error}");
                motion = Motion::Off;
            }
        }
        if enhancement != Enhancement::Off {
            if let Err(error) = tuner.set_enhancement(enhancement) {
                eprintln!("[player] image improvement not applied: {error}");
                enhancement = Enhancement::Off;
            }
        }
        let mut picture = request.video.picture.sanitized();
        if picture != Picture::default() {
            if let Err(error) = tuner.set_picture(picture) {
                eprintln!("[player] picture options not applied: {error}");
                picture = Picture::default();
            }
        }
        let wanted = Arc::new(Mutex::new(Wanted::default()));
        for &(id, name, format) in OBSERVED {
            mpv.observe(id, name, format)?;
        }

        let snapshot = Arc::new(Mutex::new(PlayerSnapshot { session, sub_visible: true, speed: 1.0, motion, enhancement, picture, ..Default::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        let events = {
            let (mpv, stop, snapshot, wanted) = (mpv.clone(), stop.clone(), snapshot.clone(), wanted.clone());
            std::thread::Builder::new()
                .name("nuvio-mpv-events".into())
                .spawn(move || event_loop(mpv, container, stop, snapshot, sink, tuner, wanted))
                .map_err(|e| PlayerError::Mpv(format!("event thread: {e}")))?
        };
        let player = Self {
            mpv: Some(mpv),
            host,
            container,
            stop,
            events: Some(events),
            snapshot,
            applied_style: Mutex::new(None),
            video_rect: Mutex::new(None),
            resize_mode: Mutex::new(ResizeMode::Fit),
            pip: Mutex::new(None),
            wanted,
        };
        // The original applies the default subtitle style on attach (pendingSubtitleStyle).
        player.apply_subtitle_style(&SubtitleStyle::default())?;
        player.load(request)?;
        Ok(player)
    }

    /// UI test only: puts back the flip swapchain, under which display sync runs too fast, so the
    /// watchdog can be checked against the real failure.
    #[doc(hidden)]
    pub fn break_display_sync(&self) -> Result<(), PlayerError> {
        self.mpv().set_property("d3d11-flip", "yes")?;
        self.mpv().set_property("vo", "gpu-next,gpu,")
    }

    fn mpv(&self) -> &Mpv {
        self.mpv.as_deref().expect("player used after shutdown")
    }

    pub fn load(&self, request: &LoadRequest) -> Result<(), PlayerError> {
        let mpv = self.mpv();
        mpv.set_property("pause", if request.paused { "yes" } else { "no" })?;
        let options = load_options(request);
        if options.is_empty() {
            mpv.command(&["loadfile", &request.url])
        } else {
            mpv.command(&["loadfile", &request.url, "replace", "-1", &options])
        }
    }

    pub fn snapshot(&self) -> PlayerSnapshot {
        self.snapshot.lock().expect("snapshot lock").clone()
    }

    pub fn execute(&self, command: &PlayerCommand) -> Result<(), PlayerError> {
        let mpv = self.mpv();
        match command {
            PlayerCommand::SetPaused { paused } => mpv.set_property("pause", if *paused { "yes" } else { "no" }),
            PlayerCommand::TogglePause => mpv.command(&["cycle", "pause"]),
            PlayerCommand::Seek { seconds, relative } => mpv.command(&[
                "seek",
                &format!("{seconds:.3}"),
                if *relative { "relative+exact" } else { "absolute+exact" },
            ]),
            PlayerCommand::SetVolume { volume } => mpv.set_property_f64("volume", volume.clamp(0.0, VOLUME_MAX)),
            PlayerCommand::SetMute { mute } => mpv.set_property("mute", if *mute { "yes" } else { "no" }),
            PlayerCommand::SetSpeed { speed } => mpv.set_property_f64("speed", speed.clamp(SPEED_MIN, SPEED_MAX)),
            PlayerCommand::SetAudioTrack { id } => mpv.set_property("aid", id),
            PlayerCommand::SetAudioLanguages { languages } => {
                mpv.set_property("alang", &languages.join(","))?;
                mpv.set_property("aid", "auto")
            }
            PlayerCommand::SetSubtitleTrack { id } => mpv.set_property("sid", id),
            PlayerCommand::AddSubtitle { url, title, lang } => {
                let mut args = vec!["sub-add", url.as_str(), "select"];
                args.extend(title.as_deref());
                if title.is_some() {
                    args.extend(lang.as_deref());
                }
                if url.is_empty() { Ok(()) } else { mpv.command(&args) }
            }
            PlayerCommand::RemoveExternalSubtitles { select_id } => {
                let count = mpv.get_i64("track-list/count").unwrap_or(0);
                for index in (0..count).rev() {
                    let p = format!("track-list/{index}");
                    let is_external_sub = mpv.get_string(&format!("{p}/type")).as_deref() == Some("sub")
                        && mpv.get_flag(&format!("{p}/external")).unwrap_or(false);
                    if is_external_sub {
                        if let Some(id) = mpv.get_i64(&format!("{p}/id")).filter(|id| *id >= 0) {
                            mpv.command(&["sub-remove", &id.to_string()])?;
                        }
                    }
                }
                match select_id {
                    Some(id) if *id >= 0 => mpv.set_property("sid", &id.to_string()),
                    _ => mpv.set_property("sid", "no"),
                }
            }
            PlayerCommand::SetSubtitleVisible { visible } => mpv.set_property("sub-visibility", if *visible { "yes" } else { "no" }),
            PlayerCommand::SetSubtitleDelay { ms } => {
                mpv.set_property_f64("sub-delay", (*ms).clamp(-SUBTITLE_DELAY_LIMIT_MS, SUBTITLE_DELAY_LIMIT_MS) as f64 / 1000.0)
            }
            PlayerCommand::SetAudioDelay { ms } => {
                mpv.set_property_f64("audio-delay", (*ms).clamp(-AUDIO_DELAY_LIMIT_MS, AUDIO_DELAY_LIMIT_MS) as f64 / 1000.0)
            }
            PlayerCommand::SetAudioAmplification { db } => {
                let gain = 100.0 * 10f64.powf((*db).clamp(0, 10) as f64 / 20.0);
                mpv.set_property_f64("volume", gain.min(1000.0))
            }
            PlayerCommand::SetAudioDownmix { layout } => {
                mpv.set_property("audio-channels", if layout.is_empty() { "auto" } else { layout })
            }
            PlayerCommand::SetBuffer { max_seconds, back_seconds, max_bytes } => {
                mpv.set_property_f64("cache-secs", max_seconds.max(0.0))?;
                mpv.set_property_f64("demuxer-readahead-secs", back_seconds.max(0.0))?;
                mpv.set_property("demuxer-max-bytes", &(*max_bytes).max(0).to_string())
            }
            PlayerCommand::ApplySubtitleStyle { style } => self.apply_subtitle_style(style),
            PlayerCommand::SetResizeMode { mode } => {
                *self.resize_mode.lock().expect("mode lock") = *mode;
                // PiP always uses Fit (desktopEffectiveResizeMode).
                let in_pip = self.pip.lock().expect("pip lock").is_some();
                self.apply_resize_mode(if in_pip { ResizeMode::Fit } else { *mode })
            }
            PlayerCommand::SetMotion { motion } => {
                self.wanted.lock().expect("wanted lock").motion = Some(*motion);
                Ok(())
            }
            PlayerCommand::SetEnhancement { enhancement } => {
                self.wanted.lock().expect("wanted lock").enhancement = Some(*enhancement);
                Ok(())
            }
            PlayerCommand::SetPicture { picture } => {
                self.wanted.lock().expect("wanted lock").picture = Some(picture.sanitized());
                Ok(())
            }
            PlayerCommand::SetCompare { active } => {
                self.wanted.lock().expect("wanted lock").compare = Some(*active);
                Ok(())
            }
        }
    }

    /// `setResizeMode` of the bridge.
    fn apply_resize_mode(&self, mode: ResizeMode) -> Result<(), PlayerError> {
        let mpv = self.mpv();
        let (keepaspect, panscan) = match mode {
            ResizeMode::Fit => ("yes", "0.0"),
            ResizeMode::Fill | ResizeMode::Zoom => ("yes", "1.0"),
            ResizeMode::Stretch => ("no", "0.0"),
        };
        mpv.set_property("keepaspect", keepaspect)?;
        mpv.set_property("panscan", panscan)?;
        mpv.set_property("video-unscaled", "no")
    }

    /// `applySubtitleStyle` + `setSubtitleAppearance` of the bridge, including its change tracking
    /// (`sub-ass-override` and the style-override reset only when the libass mode changes).
    pub fn apply_subtitle_style(&self, style: &SubtitleStyle) -> Result<(), PlayerError> {
        let mpv = self.mpv();
        let next = AppliedSubtitleStyle {
            use_libass: style.use_libass,
            text_color: Some(style.text_color.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| "#FFFFFFFF".into()),
            background_color: Some(style.background_color.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| "#00000000".into()),
            outline_color: Some(style.outline_color.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| "#FF000000".into()),
            outline_size: style.mpv_outline_size(),
            bold: style.bold,
            font_size: style.mpv_font_size(),
            position: style.mpv_sub_pos(),
            strip_sdh: style.strip_sdh,
        };
        let mut applied = self.applied_style.lock().expect("style lock");
        let prev = applied.as_ref();
        let mode_changed = prev.is_none_or(|p| p.use_libass != next.use_libass);
        let changed = |f: fn(&AppliedSubtitleStyle, &AppliedSubtitleStyle) -> bool| mode_changed || prev.is_none_or(|p| f(p, &next));

        if mode_changed {
            mpv.set_property("sub-ass-override", "no")?;
            mpv.command(&["change-list", "sub-ass-style-overrides", "clr", ""])?;
        }
        if changed(|p, n| p.font_size != n.font_size) {
            mpv.set_property_f64("sub-scale", 1.0)?;
            mpv.set_property_f64("sub-font-size", next.font_size)?;
        }
        if changed(|p, n| p.position != n.position) {
            mpv.set_property("sub-pos", &next.position.to_string())?;
        }
        if changed(|p, n| p.text_color != n.text_color) {
            mpv.set_property("sub-color", &next.text_color)?;
        }
        if changed(|p, n| p.background_color != n.background_color) {
            let border = if next.background_color.starts_with("#00") { "outline-and-shadow" } else { "background-box" };
            mpv.set_property("sub-border-style", border)?;
        }
        if changed(|p, n| p.outline_color != n.outline_color) {
            mpv.set_property("sub-outline-color", &next.outline_color)?;
        }
        if changed(|p, n| p.bold != n.bold) {
            mpv.set_property("sub-bold", if next.bold { "yes" } else { "no" })?;
        }
        if changed(|p, n| p.outline_size != n.outline_size) {
            mpv.set_property_f64("sub-outline-size", next.outline_size)?;
        }
        if prev.is_none_or(|p| p.strip_sdh != next.strip_sdh) {
            let flag = if next.strip_sdh { "yes" } else { "no" };
            mpv.set_property("sub-filter-sdh", flag)?;
            mpv.set_property("sub-filter-sdh-harder", flag)?;
        }
        *applied = Some(next);
        drop(applied);

        mpv.set_property("sub-ass-override", if style.override_ass { "force" } else { "no" })?;
        mpv.set_property("sub-italic", if style.italic { "yes" } else { "no" })?;
        mpv.set_property_f64("sub-spacing", style.spacing.clamp(0.0, 10.0))?;
        mpv.set_property_f64("sub-blur", style.blur.clamp(0.0, 20.0))?;
        mpv.set_property("sub-align-x", if matches!(style.align.as_str(), "left" | "right") { style.align.as_str() } else { "center" })?;

        // setSubtitleAppearance: font, fonts dir, shadow.
        if let Some(dir) = style.fonts_dir.as_deref().filter(|d| !d.is_empty()) {
            mpv.set_property("sub-fonts-dir", dir)?;
        }
        mpv.set_property("sub-font", &style.font)?;
        // mpv's `sub-shadow-color` is an alias of `sub-back-color`: with a box, the color is the box's
        // and the shadow offset its padding, so a box has no shadow.
        let (color, offset) = match style.mpv_box() {
            Some(color) => (color.to_string(), SUBTITLE_BOX_PADDING),
            None => (style.mpv_shadow_color(), style.mpv_shadow_offset()),
        };
        mpv.set_property("sub-back-color", &color)?;
        mpv.set_property_f64("sub-shadow-offset", offset)
    }

    /// Places the video inside `rect` (physical pixels, host client coordinates); `None` = whole
    /// window. Ignored while in PiP (the PiP window always shows the whole video).
    pub fn set_video_rect(&self, rect: Option<(f64, f64, f64, f64)>) -> Result<(), PlayerError> {
        *self.video_rect.lock().expect("rect lock") = rect;
        if self.pip.lock().expect("pip lock").is_some() {
            return Ok(());
        }
        self.apply_margins(rect)
    }

    fn apply_margins(&self, rect: Option<(f64, f64, f64, f64)>) -> Result<(), PlayerError> {
        let mpv = self.mpv();
        let (w, h) = win::client_size(hwnd(self.container));
        let (w, h) = (w as f64, h as f64);
        let (left, top, right, bottom) = match rect {
            None => (0.0, 0.0, 0.0, 0.0),
            Some((x, y, rw, rh)) => (
                (x / w).clamp(0.0, 1.0),
                (y / h).clamp(0.0, 1.0),
                ((w - x - rw) / w).clamp(0.0, 1.0),
                ((h - y - rh) / h).clamp(0.0, 1.0),
            ),
        };
        mpv.set_property_f64("video-margin-ratio-left", left)?;
        mpv.set_property_f64("video-margin-ratio-top", top)?;
        mpv.set_property_f64("video-margin-ratio-right", right)?;
        mpv.set_property_f64("video-margin-ratio-bottom", bottom)
    }

    /// Keeps the container the size of the host window (call on host resize). UI thread.
    pub fn fit_to_host(&self) {
        if self.pip.lock().expect("pip lock").is_none() {
            win::fit_to_parent(hwnd(self.container));
            let rect = *self.video_rect.lock().expect("rect lock");
            let _ = self.apply_margins(rect);
        }
    }

    pub fn is_picture_in_picture(&self) -> bool {
        self.pip.lock().expect("pip lock").is_some()
    }

    /// Moves the video into `window`, the app's PiP window (original `DesktopPlayerPictureInPicture`):
    /// aspect from `video-out-params`, Fit resize mode, no margins. UI thread.
    pub fn enter_picture_in_picture(&self, window: isize) -> Result<(), PlayerError> {
        let mut pip = self.pip.lock().expect("pip lock");
        if pip.is_some() {
            return Ok(());
        }
        let s = self.snapshot();
        let aspect = if s.display_width > 0 && s.display_height > 0 {
            s.display_width as f64 / s.display_height as f64
        } else if s.width > 0 && s.height > 0 {
            s.width as f64 / s.height as f64
        } else {
            16.0 / 9.0
        };
        let state = win::enter_pip(hwnd(self.container), hwnd(self.host), hwnd(window), aspect).map_err(PlayerError::Mpv)?;
        let previous_mode = *self.resize_mode.lock().expect("mode lock");
        *pip = Some(PipSession { window, state, previous_rect: *self.video_rect.lock().expect("rect lock"), previous_mode });
        drop(pip);
        self.apply_margins(None)?;
        self.apply_resize_mode(ResizeMode::Fit)?;
        self.snapshot.lock().expect("snapshot lock").picture_in_picture = true;
        Ok(())
    }

    /// Restores the video into the host window. UI thread.
    pub fn exit_picture_in_picture(&self) -> Result<(), PlayerError> {
        let Some(session) = self.pip.lock().expect("pip lock").take() else { return Ok(()) };
        win::exit_pip(hwnd(self.container), hwnd(self.host), hwnd(session.window), session.state);
        self.apply_resize_mode(session.previous_mode)?;
        self.apply_margins(session.previous_rect)?;
        self.snapshot.lock().expect("snapshot lock").picture_in_picture = false;
        Ok(())
    }

    pub fn set_window_title(&self, title: &str) {
        if let Some(session) = self.pip.lock().expect("pip lock").as_ref() {
            let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                let _ = SetWindowTextW(hwnd(session.window), windows::core::PCWSTR(wide.as_ptr()));
            }
        }
    }

    pub fn container_hwnd(&self) -> isize {
        self.container
    }

    /// Reads an mpv property as text (verification in tests; never used to expose URLs).
    pub fn property(&self, name: &str) -> Option<String> {
        self.mpv().get_string(name)
    }

    /// Stops playback and destroys the mpv core, its threads and windows. Idempotent. UI thread.
    pub fn shutdown(&mut self) {
        if self.mpv.is_none() {
            return;
        }
        let _ = self.exit_picture_in_picture();
        self.stop.store(true, Ordering::SeqCst);
        if let Some(mpv) = &self.mpv {
            mpv.wakeup();
        }
        if let Some(events) = self.events.take() {
            let _ = events.join();
        }
        // Last reference: mpv_terminate_destroy runs here and waits for mpv's own threads and window.
        self.mpv.take();
        win::destroy(hwnd(self.container));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn read_chapters(mpv: &Mpv) -> Vec<Chapter> {
    let count = mpv.get_i64("chapter-list/count").unwrap_or(0).min(200);
    (0..count)
        .filter_map(|i| {
            Some(Chapter {
                time: mpv.get_f64(&format!("chapter-list/{i}/time"))?,
                title: mpv.get_string(&format!("chapter-list/{i}/title")).unwrap_or_default(),
            })
        })
        .collect()
}

fn read_tracks(mpv: &Mpv) -> Vec<Track> {
    let count = mpv.get_i64("track-list/count").unwrap_or(0);
    (0..count)
        .map(|i| {
            let p = |field: &str| mpv.get_string(&format!("track-list/{i}/{field}"));
            let f = |field: &str| mpv.get_flag(&format!("track-list/{i}/{field}")).unwrap_or(false);
            Track {
                id: mpv.get_i64(&format!("track-list/{i}/id")).unwrap_or(-1),
                kind: p("type").unwrap_or_default(),
                title: p("title"),
                lang: p("lang"),
                codec: p("codec"),
                selected: f("selected"),
                external: f("external"),
                default: f("default"),
                forced: f("forced"),
            }
        })
        .collect()
}

/// Keeps mpv's video window at the bottom and disabled: the webview above handles all input,
/// and a disabled child lets clicks reach the PiP window (drag) instead of mpv.
fn adopt_mpv_window(container: isize) {
    unsafe extern "system" fn visit(child: HWND, _: LPARAM) -> BOOL {
        let mut class = [0u16; 32];
        let len = unsafe { GetClassNameW(child, &mut class) } as usize;
        if String::from_utf16_lossy(&class[..len]) == "mpv" {
            unsafe {
                let _ = EnableWindow(child, false);
                let _ = SetWindowPos(child, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        BOOL(1)
    }
    unsafe {
        let _ = EnumChildWindows(Some(hwnd(container)), Some(visit), LPARAM(0));
    }
}

fn error_text(code: i32) -> String {
    // mpv error codes (client.h); the text never includes the URL.
    match code {
        -13 => "loading failed (network, format or protocol)".into(),
        -14 => "unsupported or unknown format".into(),
        -15 => "nothing to play (no audio or video stream)".into(),
        -16 => "video output initialization failed".into(),
        -17 => "audio output initialization failed".into(),
        other => format!("playback failed (mpv error {other})"),
    }
}

fn event_loop(
    mpv: Arc<Mpv>,
    container: isize,
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<PlayerSnapshot>>,
    sink: SnapshotSink,
    mut tuner: Tuner,
    wanted: Arc<Mutex<Wanted>>,
) {
    let mut last_emit = Instant::now() - EMIT_INTERVAL;
    let mut dirty = false;
    while !stop.load(Ordering::SeqCst) {
        // SAFETY: this thread is the only one waiting for events on this handle.
        let event = unsafe { &*mpv.wait_event(0.25) };
        let mut urgent = false;
        if event.event_id == EVENT_PLAYBACK_RESTART {
            tuner.playback_restarted();
        }
        // Outside the snapshot lock: a change may recreate the video output.
        let asked = std::mem::take(&mut *wanted.lock().expect("wanted lock"));
        let motion = asked.motion.and_then(|m| tuner.set_motion(m).map(|()| m).map_err(|e| eprintln!("[player] motion: {e}")).ok());
        let enhancement = asked
            .enhancement
            .and_then(|l| tuner.set_enhancement(l).map(|()| l).map_err(|e| eprintln!("[player] image improvement: {e}")).ok());
        let picture = asked.picture.and_then(|p| tuner.set_picture(p).map(|()| p).map_err(|e| eprintln!("[player] picture options: {e}")).ok());
        if let Some(compare) = asked.compare {
            let _ = tuner.set_compare(compare).map_err(|e| eprintln!("[player] compare: {e}"));
        }
        let lowered = tuner.poll();
        {
            let mut s = snapshot.lock().expect("snapshot lock");
            match event.event_id {
                EVENT_NONE => {}
                EVENT_SHUTDOWN => break,
                EVENT_START_FILE => {
                    s.loaded = false;
                    s.ended = false;
                    s.error = None;
                    urgent = true;
                }
                EVENT_FILE_LOADED | EVENT_VIDEO_RECONFIG | EVENT_PLAYBACK_RESTART => {
                    if event.event_id == EVENT_FILE_LOADED {
                        s.loaded = true;
                    }
                    adopt_mpv_window(container);
                    urgent = true;
                }
                EVENT_END_FILE if !event.data.is_null() => {
                    // SAFETY: END_FILE carries an mpv_event_end_file.
                    let end = unsafe { &*(event.data as *const MpvEventEndFile) };
                    match end.reason {
                        END_FILE_EOF => s.ended = true,
                        END_FILE_ERROR => s.error = Some(error_text(end.error)),
                        _ => {}
                    }
                    urgent = true;
                }
                EVENT_PROPERTY_CHANGE if !event.data.is_null() => {
                    // SAFETY: PROPERTY_CHANGE carries an mpv_event_property.
                    let prop = unsafe { &*(event.data as *const MpvEventProperty) };
                    let name = unsafe { CStr::from_ptr(prop.name) }.to_string_lossy();
                    // SAFETY: data matches the format reported by mpv (or is null when unavailable).
                    let f64v = || (prop.format == FORMAT_DOUBLE && !prop.data.is_null()).then(|| unsafe { *(prop.data as *const f64) });
                    let i64v = || (prop.format == FORMAT_INT64 && !prop.data.is_null()).then(|| unsafe { *(prop.data as *const i64) });
                    let flag = || (prop.format == FORMAT_FLAG && !prop.data.is_null()).then(|| unsafe { *(prop.data as *const i32) } != 0);
                    let string = || {
                        (prop.format == FORMAT_STRING && !prop.data.is_null()).then(|| unsafe {
                            CStr::from_ptr(*(prop.data as *const *const std::ffi::c_char)).to_string_lossy().into_owned()
                        })
                    };
                    match name.as_ref() {
                        "time-pos" => s.time_pos = f64v().unwrap_or(0.0),
                        "duration" => s.duration = f64v().unwrap_or(0.0),
                        "pause" => { s.paused = flag().unwrap_or(false); urgent = true; }
                        "paused-for-cache" => { s.buffering = flag().unwrap_or(false); urgent = true; }
                        "demuxer-cache-duration" => s.cache_seconds = f64v().unwrap_or(0.0),
                        "volume" => { s.volume = f64v().unwrap_or(0.0); urgent = true; }
                        "mute" => { s.mute = flag().unwrap_or(false); urgent = true; }
                        "speed" => { s.speed = f64v().unwrap_or(1.0); urgent = true; }
                        "sub-delay" => { s.sub_delay_ms = (f64v().unwrap_or(0.0) * 1000.0).round() as i64; urgent = true; }
                        "audio-delay" => { s.audio_delay_ms = (f64v().unwrap_or(0.0) * 1000.0).round() as i64; urgent = true; }
                        "sub-visibility" => { s.sub_visible = flag().unwrap_or(true); urgent = true; }
                        "avsync" => s.av_sync = f64v().unwrap_or(0.0),
                        "hwdec-current" => s.hwdec_current = string(),
                        "video-codec" => s.video_codec = string(),
                        "width" => s.width = i64v().unwrap_or(0),
                        "height" => s.height = i64v().unwrap_or(0),
                        "video-out-params/dw" => s.display_width = i64v().unwrap_or(0),
                        "video-out-params/dh" => s.display_height = i64v().unwrap_or(0),
                        "estimated-vf-fps" => s.fps = f64v().unwrap_or(0.0),
                        "frame-drop-count" => s.dropped_frames = i64v().unwrap_or(0),
                        "decoder-frame-drop-count" => s.decoder_dropped_frames = i64v().unwrap_or(0),
                        "aid" => { s.aid = string(); urgent = true; }
                        "sid" => { s.sid = string(); urgent = true; }
                        "track-list" => { s.tracks = read_tracks(&mpv); urgent = true; }
                        "chapter-list" => { s.chapters = read_chapters(&mpv); urgent = true; }
                        // Back to false when the user seeks back from the end.
                        "eof-reached" => { if let Some(end) = flag() { urgent |= end != s.ended; s.ended = end; } }
                        _ => {}
                    }
                }
                _ => {}
            }
            if event.event_id != EVENT_NONE {
                dirty = true;
            }
            if asked.motion.is_some() || asked.enhancement.is_some() || asked.picture.is_some() || lowered.is_some() {
                s.motion = motion.unwrap_or(s.motion);
                s.enhancement = enhancement.unwrap_or(s.enhancement);
                s.picture = picture.unwrap_or(s.picture);
                s.auto_changes += u32::from(lowered.is_some());
                match lowered {
                    Some(Lowered::Motion) => (s.motion, s.auto_reason) = (Motion::Off, Some("motion")),
                    Some(Lowered::Enhancement(level)) => (s.enhancement, s.auto_reason) = (level, Some("enhancement")),
                    Some(Lowered::Picture(lighter)) => (s.picture, s.auto_reason) = (lighter, Some("effects")),
                    None => {}
                }
                (dirty, urgent) = (true, true);
            }
            if dirty && (urgent || last_emit.elapsed() >= EMIT_INTERVAL) {
                sink(&s);
                last_emit = Instant::now();
                dirty = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value<'a>(opts: &'a [(String, String)], key: &str) -> Vec<&'a str> {
        opts.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
    }

    #[test]
    fn header_lines_trim_and_skip_blank_like_to_header_lines() {
        let lines = http_header_lines(&[
            ("Referer".into(), " https://a.test/x,y ".into()),
            ("User-Agent".into(), r"A\B".into()),
            ("X-Empty".into(), "  ".into()),
            (" ".into(), "value".into()),
            ("Cookie".into(), "k=v; t=1".into()),
        ]);
        // Commas and backslashes stay literal: each line is passed to mpv as its own string.
        assert_eq!(lines, vec!["Referer: https://a.test/x,y", r"User-Agent: A\B", "Cookie: k=v; t=1"]);
    }

    #[test]
    fn startup_options_follow_the_bridge_for_each_configuration() {
        let default = startup_options(&VideoConfig::default(), false, Some("C:/cache"));
        assert_eq!(value(&default, "hwdec"), vec!["auto"]);
        assert_eq!(value(&default, "vd-lavc-software-fallback"), vec!["yes"]);
        assert_eq!(value(&default, "deband"), vec!["yes"]);
        assert!(value(&default, "vf").is_empty());
        assert_eq!(value(&default, "gpu-shader-cache-dir"), vec!["C:/cache"]);

        let integrated = startup_options(&VideoConfig::default(), true, None);
        assert_eq!(value(&integrated, "profile"), vec!["fast"]);
        assert!(value(&integrated, "deband").is_empty());

        let software = startup_options(&VideoConfig { decoder_priority: DecoderPriority::SoftwareOnly, ..Default::default() }, false, None);
        assert_eq!(value(&software, "hwdec"), vec!["auto", "no"]); // last one wins, like the bridge
        let hardware = startup_options(&VideoConfig { decoder_priority: DecoderPriority::HardwareOnly, ..Default::default() }, false, None);
        assert_eq!(value(&hardware, "vd-lavc-software-fallback"), vec!["no"]);

        let rtx = startup_options(&VideoConfig { nvidia_rtx_super_resolution: true, nvidia_rtx_hdr: true, ..Default::default() }, true, None);
        assert_eq!(value(&rtx, "gpu-api"), vec!["d3d11"]);
        assert_eq!(value(&rtx, "hwdec"), vec!["d3d11va"]);
        assert_eq!(value(&rtx, "d3d11-adapter"), vec!["NVIDIA"]);
        assert!(value(&rtx, "vf")[0].contains("scaling-mode=nvidia") && value(&rtx, "vf")[0].contains("nvidia-true-hdr"));
        assert!(value(&rtx, "profile").is_empty()); // RTX disables the fast profile
    }

    #[test]
    fn subtitle_style_mapping_matches_native_player_controller() {
        let style = SubtitleStyle::default();
        assert_eq!(style.mpv_font_size(), 54.0);
        assert_eq!(style.mpv_sub_pos(), 90);
        assert_eq!(style.mpv_outline_size(), 2.0);
        assert_eq!(style.mpv_shadow_color(), "#00000000");
        let big = SubtitleStyle { font_size_sp: 40.0, bottom_offset: -400, outline_enabled: false, shadow_enabled: true, shadow_opacity: 50, ..Default::default() };
        assert_eq!(big.mpv_font_size(), 96.0);
        assert_eq!(big.mpv_sub_pos(), 150);
        assert_eq!(big.mpv_outline_size(), 0.0);
        assert_eq!(big.mpv_shadow_color(), "#7F000000");
        assert_eq!(big.mpv_box(), None);
        assert_eq!(SubtitleStyle { background_color: "#B3000000".into(), ..Default::default() }.mpv_box(), Some("#B3000000"));
    }
}
