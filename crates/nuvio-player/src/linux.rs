//! Linux player backend: libmpv embedded in an X11 child window of the Tauri (GTK/XWayland)
//! window. Mirrors the proven model of the upstream Nuvio Desktop Linux player bridge:
//! `wid = <X11 window>` + `vo=gpu` + `gpu-context=x11egl` (fallback `x11vk`), X11 embedding even on
//! Wayland sessions, and the same mpv options / events / snapshots as the Windows backend.
//!
//! Kept in this crate so the desktop app and the web UI use the exact same public API on both OSes.

use crate::mpv::*;
use crate::PlayerError;
use serde::{Deserialize, Serialize};
use std::ffi::{c_char, c_int, CStr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Decoder preference, as `decoderPriority` in the original app (0 / 1 / 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DecoderPriority {
    HardwareOnly,
    #[default]
    HardwarePreferred,
    SoftwareOnly,
}

/// Motion interpolation (Windows backend drives display-sync). Placeholder on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Motion { #[default] Off, Steady, Fluid }

/// Image enhancement level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Enhancement { #[default] Off, Movies, AnimeFast, AnimeQuality }

impl Motion {
    pub fn name(self) -> &'static str {
        match self { Motion::Off => "off", Motion::Steady => "steady", Motion::Fluid => "fluid" }
    }
    pub fn from_name(name: &str) -> Self {
        [Motion::Steady, Motion::Fluid].into_iter().find(|m| m.name() == name).unwrap_or_default()
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
        [Enhancement::Movies, Enhancement::AnimeFast, Enhancement::AnimeQuality]
            .into_iter().find(|e| e.name() == name).unwrap_or_default()
    }
}

/// Anime4K mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AnimeMode { #[default] Auto, A, B, C, Aa, Bb, Ca }

/// Fine-tuning of the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Picture {
    pub mode: AnimeMode,
    pub denoise: i64,
    pub deblur: i64,
    pub thin: i64,
    pub darken: i64,
    pub deband: i64,
    pub brightness: i64,
    pub contrast: i64,
    pub saturation: i64,
    pub gamma: i64,
    pub hue: i64,
}

/// Per-session video configuration (the original player's settings).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoConfig {
    pub decoder_priority: DecoderPriority,
    pub nvidia_rtx_super_resolution: bool,
    pub nvidia_rtx_hdr: bool,
    pub motion: Motion,
    pub enhancement: Enhancement,
    pub picture: Picture,
    pub audio_passthrough: bool,
    pub subtitle_codepage: String,
}

/// What to play.
#[derive(Debug, Clone)]
pub struct LoadRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub start_seconds: f64,
    pub paused: bool,
    pub video: VideoConfig,
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleStyle {
    pub text_color: String,
    pub background_color: String,
    pub outline_color: String,
    pub outline_enabled: bool,
    pub outline_width: f64,
    pub bold: bool,
    pub font_size_sp: f64,
    pub bottom_offset: i64,
    pub strip_sdh: bool,
    pub use_libass: bool,
    pub override_ass: bool,
    pub italic: bool,
    pub spacing: f64,
    pub blur: f64,
    pub align: String,
    pub font: String,
    pub fonts_dir: Option<String>,
    pub shadow_enabled: bool,
    pub shadow_opacity: i64,
    pub shadow_size: f64,
}

impl Default for SubtitleStyle {
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
            font: String::new(),
            fonts_dir: None,
            shadow_enabled: false,
            shadow_opacity: 0,
            shadow_size: 0.0,
        }
    }
}

impl SubtitleStyle {
    pub fn mpv_font_size(&self) -> f64 { (self.font_size_sp * 3.0).clamp(18.0, 96.0) }
    pub fn mpv_sub_pos(&self) -> i64 { (100 - self.bottom_offset / 2).clamp(0, 150) }
    pub fn mpv_outline_size(&self) -> f64 { if self.outline_enabled { self.outline_width.clamp(0.0, 8.0) } else { 0.0 } }
    pub fn mpv_shadow_color(&self) -> String {
        let alpha = if self.shadow_enabled { (self.shadow_opacity.clamp(0, 100) * 255 / 100) as u8 } else { 0 };
        format!("#{alpha:02X}000000")
    }
    pub fn mpv_shadow_offset(&self) -> f64 { if self.shadow_enabled { self.shadow_size } else { 0.0 } }
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

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Chapter { pub time: f64, pub title: String }

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
    pub display_width: i64,
    pub display_height: i64,
    pub fps: f64,
    pub av_sync: f64,
    pub dropped_frames: i64,
    pub decoder_dropped_frames: i64,
    pub aid: Option<String>,
    pub sid: Option<String>,
    pub tracks: Vec<Track>,
    pub chapters: Vec<Chapter>,
    pub picture_in_picture: bool,
    pub motion: Motion,
    pub enhancement: Enhancement,
    pub picture: Picture,
    pub auto_changes: u32,
    pub auto_reason: Option<&'static str>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PlayerCommand {
    SetPaused { paused: bool },
    TogglePause,
    Seek { seconds: f64, relative: bool },
    SetVolume { volume: f64 },
    SetMute { mute: bool },
    SetSpeed { speed: f64 },
    SetAudioTrack { id: String },
    SetAudioLanguages { languages: Vec<String> },
    SetSubtitleTrack { id: String },
    AddSubtitle { url: String, #[serde(default)] title: Option<String>, #[serde(default)] lang: Option<String> },
    RemoveExternalSubtitles { select_id: Option<i64> },
    SetSubtitleVisible { visible: bool },
    SetSubtitleDelay { ms: i64 },
    SetAudioDelay { ms: i64 },
    SetAudioAmplification { db: i64 },
    SetAudioDownmix { layout: String },
    SetBuffer { max_seconds: f64, back_seconds: f64, max_bytes: i64 },
    ApplySubtitleStyle { style: SubtitleStyle },
    SetResizeMode { mode: ResizeMode },
    SetMotion { motion: Motion },
    SetEnhancement { enhancement: Enhancement },
    SetPicture { picture: Picture },
    SetCompare { active: bool },
}

pub type SnapshotSink = Arc<dyn Fn(&PlayerSnapshot) + Send + Sync>;

const OBSERVED: &[(u64, &str, c_int)] = &[
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
];

const EMIT_INTERVAL: Duration = Duration::from_millis(250);

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
    previous_rect: Option<(f64, f64, f64, f64)>,
    previous_mode: ResizeMode,
}

// ---------------------------------------------------------------------------
// X11 window helpers (a separate connection is opened per group of operations;
// all calls happen on the UI thread except the PiP watcher, which owns its own).
// ---------------------------------------------------------------------------
mod x11 {
    use std::ffi::{c_char, c_int, CStr, CString};
    use std::ptr;
    use std::sync::{Arc, Mutex, OnceLock};
    use x11::xlib::*;

    /// The embedded player touches X11 windows that the window manager can reparent or destroy at
    /// any moment (especially on XWayland). The default Xlib handler aborts the process on
    /// BadWindow, so install a quiet handler once for our process.
    unsafe extern "C" fn ignore_x_error(_display: *mut Display, _event: *mut XErrorEvent) -> c_int {
        0
    }

    // Xlib installs error handlers per thread, so each thread that touches X11 needs its own.
    thread_local! {
        static QUIET: bool = unsafe { XSetErrorHandler(Some(ignore_x_error)); true };
    }

    fn quiet_x_errors() {
        let _ = QUIET.with(|installed| *installed);
    }

    fn alive(dpy: *mut Display, window: Window) -> bool {
        let mut attrs: XWindowAttributes = unsafe { std::mem::zeroed() };
        unsafe { XGetWindowAttributes(dpy, window, &mut attrs) != 0 }
    }

    /// One connection for the whole process. An X client's windows are destroyed when its
    /// connection closes, so this must NOT be closed until the app exits.
    struct DisplayPtr(*mut Display);
    // SAFETY: Xlib calls are serialised by the UI thread; the PiP watcher uses its own connection.
    unsafe impl Send for DisplayPtr {}
    unsafe impl Sync for DisplayPtr {}
    static DISPLAY: OnceLock<DisplayPtr> = OnceLock::new();

    pub fn display() -> Option<*mut Display> {
        quiet_x_errors();
        let p = DISPLAY.get_or_init(|| unsafe {
            let d = XOpenDisplay(ptr::null());
            DisplayPtr(if d.is_null() { XOpenDisplay(b":0\0".as_ptr().cast()) } else { d })
        });
        if p.0.is_null() { None } else { Some(p.0) }
    }

    /// A thread-owned X connection. Xlib connections must not be shared across threads.
    pub fn open_private_display() -> *mut Display {
        let dpy = unsafe { XOpenDisplay(ptr::null()) };
        if !dpy.is_null() {
            unsafe { XSetErrorHandler(Some(ignore_x_error)); }
        }
        dpy
    }

    pub fn close_private_display(dpy: *mut Display) {
        if !dpy.is_null() {
            unsafe { XCloseDisplay(dpy); }
        }
    }

    /// Finds the real Tauri window (WM_CLASS `nuvio-desktop`), not the WM's frame window that
    /// shares the same title. The app forces the X11 GDK backend at startup, so the GTK/WebKit
    /// window is an X11 window even on a Wayland session.
    pub fn find_host(title: &str) -> Option<isize> {
        let dpy = display()?;

        unsafe fn search(dpy: *mut Display, w: Window, title: &str, found: &mut Option<isize>) {
            unsafe {
            let mut name: *mut c_char = ptr::null_mut();
            let is_nuvio = if XFetchName(dpy, w, &mut name) != 0 && !name.is_null() {
                let text = CStr::from_ptr(name).to_string_lossy().into_owned();
                XFree(name.cast());
                text.contains(title)
            } else {
                false
            };
            if is_nuvio {
                let mut hint: XClassHint = std::mem::zeroed();
                if XGetClassHint(dpy, w, &mut hint) != 0 {
                    let cls = CStr::from_ptr(hint.res_name).to_string_lossy().into_owned();
                    let class = CStr::from_ptr(hint.res_class).to_string_lossy().into_owned();
                    XFree(hint.res_name.cast());
                    XFree(hint.res_class.cast());
                    if cls.contains("nuvio") || class.contains("Nuvio") {
                        *found = Some(w as isize);
                    }
                }
            }
            let (mut r, mut p, mut kids, mut n) = (0, 0, ptr::null_mut(), 0);
            if XQueryTree(dpy, w, &mut r, &mut p, &mut kids, &mut n) != 0 && !kids.is_null() {
                for i in 0..n as usize {
                    search(dpy, *kids.add(i), title, found);
                }
                XFree(kids.cast());
            }
            }
        }

        let mut found = None;
        unsafe { search(dpy, XDefaultRootWindow(dpy), title, &mut found) };
        found
    }

    fn size(dpy: *mut Display, window: Window) -> (i32, i32) {
        let mut attrs: XWindowAttributes = unsafe { std::mem::zeroed() };
        if unsafe { XGetWindowAttributes(dpy, window, &mut attrs) } == 0 {
            return (1280, 720);
        }
        (attrs.width, attrs.height)
    }

    /// Absolute position of a window's upper-left corner on the root window.
    fn origin(dpy: *mut Display, window: Window) -> (i32, i32) {
        let root = unsafe { XDefaultRootWindow(dpy) };
        let (mut x, mut y, mut child) = (0i32, 0i32, 0u64);
        let ok = unsafe { XTranslateCoordinates(dpy, window, root, 0, 0, &mut x, &mut y, &mut child) };
        if ok == 0 { (0, 0) } else { (x, y) }
    }

    #[link(name = "Xext")]
    unsafe extern "C" {
        fn XShapeCombineRectangles(
            dpy: *mut Display, window: Window, op: i32, x: i32, y: i32,
            rectangles: *mut XRectangle, n_rectangles: i32, shaping_kind: i32, op2: i32,
        ) -> i32;
        fn XShapeCombineMask(
            dpy: *mut Display, window: Window, shaping_kind: i32, x: i32, y: i32,
            mask: Pixmap, op: i32,
        ) -> i32;
    }

    /// `origin`/`size` of the video container and the holes (window coordinates) to cut out of it,
    /// clipped to the container. The WebView cannot paint over the native video window, so the
    /// player chrome is a hole in it instead.
    pub fn local_holes(
        origin: (f64, f64),
        size: (f64, f64),
        holes: &[(f64, f64, f64, f64)],
    ) -> Vec<(i32, i32, i32, i32)> {
        let (ox, oy) = origin;
        let (sw, sh) = size;
        holes
            .iter()
            .filter_map(|&(x, y, w, h)| {
                let left = (x - ox).max(0.0);
                let top = (y - oy).max(0.0);
                let right = (x + w - ox).min(sw);
                let bottom = (y + h - oy).min(sh);
                (right - left >= 1.0 && bottom - top >= 1.0)
                    .then(|| (left as i32, top as i32, (right - left) as i32, (bottom - top) as i32))
            })
            .collect()
    }

    /// Makes the window's visible region the whole rectangle minus `holes` (its own coordinates).
    /// An empty list restores the normal rectangular window.
    pub fn set_holes(window: isize, holes: &[(i32, i32, i32, i32)]) {
        if window == 0 { return; }
        let Some(dpy) = display() else { return };
        quiet_x_errors();
        unsafe {
            const SHAPE_BOUNDING: i32 = 0;
            const SHAPE_SET: i32 = 0;
            const SHAPE_SUBTRACT: i32 = 3;
            if !alive(dpy, window as Window) { return; }
            if holes.is_empty() {
                XShapeCombineMask(dpy, window as Window, SHAPE_BOUNDING, 0, 0, 0, SHAPE_SET);
                XFlush(dpy);
                return;
            }
            let (w, h) = size(dpy, window as Window);
            let mut full = [XRectangle { x: 0, y: 0, width: w.max(1) as u16, height: h.max(1) as u16 }];
            XShapeCombineRectangles(dpy, window as Window, SHAPE_BOUNDING, 0, 0, full.as_mut_ptr(), 1, SHAPE_SET, 0);
            let mut rects: Vec<XRectangle> = holes
                .iter()
                .map(|&(x, y, w, h)| XRectangle {
                    x: x.clamp(0, i16::MAX as i32) as i16,
                    y: y.clamp(0, i16::MAX as i32) as i16,
                    width: w.clamp(0, u16::MAX as i32) as u16,
                    height: h.clamp(0, u16::MAX as i32) as u16,
                })
                .collect();
            XShapeCombineRectangles(
                dpy, window as Window, SHAPE_BOUNDING, 0, 0,
                rects.as_mut_ptr(), rects.len() as i32, SHAPE_SUBTRACT, 0,
            );
            XFlush(dpy);
        }
    }

    /// The video container is the large child of the host window that is not the WebView.
    pub fn container_of(host: isize) -> isize {
        let Some(dpy) = display() else { return 0 };
        unsafe {
            let (mut root, mut parent, mut kids, mut n) = (0, 0, std::ptr::null_mut(), 0);
            if XQueryTree(dpy, host as Window, &mut root, &mut parent, &mut kids, &mut n) != 0 && !kids.is_null() {
                let mut found = 0;
                for i in 0..n as usize {
                    let k = *kids.add(i);
                    let (w, _) = size(dpy, k);
                    if w > 200 { found = k as isize; }
                }
                XFree(kids.cast());
                return found;
            }
        }
        0
    }

    /// Makes a window (and its children) input-transparent: pointer events fall through to the
    /// WebView, which paints the player controls in the areas cut out of the video window.
    pub fn input_transparent(window: isize) {
        let Some(dpy) = display() else { return };
        input_transparent_on(dpy, window);
    }

    /// Same operation on a thread-owned display connection (never share Xlib connections across threads).
    pub fn input_transparent_on(dpy: *mut Display, window: isize) {
        if window == 0 || dpy.is_null() { return; }
        unsafe {
            XSetErrorHandler(Some(ignore_x_error));
            const SHAPE_INPUT: i32 = 2;
            const SHAPE_SET: i32 = 0;
            if !alive(dpy, window as Window) { return; }
            XShapeCombineRectangles(dpy, window as Window, SHAPE_INPUT, 0, 0, std::ptr::null_mut(), 0, SHAPE_SET, 0);
            let (mut root, mut parent, mut kids, mut n) = (0, 0, std::ptr::null_mut(), 0);
            if XQueryTree(dpy, window as Window, &mut root, &mut parent, &mut kids, &mut n) != 0 && !kids.is_null() {
                for i in 0..n as usize {
                    let child = *kids.add(i);
                    if alive(dpy, child) {
                        XShapeCombineRectangles(dpy, child, SHAPE_INPUT, 0, 0, std::ptr::null_mut(), 0, SHAPE_SET, 0);
                    }
                }
                XFree(kids.cast());
            }
            XFlush(dpy);
        }
    }

    /// Creates the video container as a child of the host, sized to the video area only so the
    /// WebView controls below it stay visible and clickable.
    pub fn create_container(host: isize) -> Result<isize, String> {
        let dpy = display().ok_or("cannot open X display (set DISPLAY / use XWayland)")?;
        unsafe {
            let host = host as Window;
            let (w, h) = size(dpy, host);
            let window = XCreateSimpleWindow(dpy, host, 0, 0, w as u32, h as u32, 0, 0, 0);
            if window == 0 { return Err("XCreateSimpleWindow failed".into()); }
            XMapWindow(dpy, window);
            XLowerWindow(dpy, window);
            XFlush(dpy);
            Ok(window as isize)
        }
    }

    /// The window manager reparents the Tauri window into its frame (and unmaps children) shortly
    /// after creation, so the video container must be mapped again once the host is visible.
    pub fn map(window: isize, host: isize) {
        let Some(dpy) = display() else { return };
        quiet_x_errors();
        unsafe {
            XMapWindow(dpy, window as Window);
            XLowerWindow(dpy, window as Window);
            // Tauri draws its WebView into the host window itself, so the video child has to be
            // input-transparent for clicks to reach the player controls painted underneath.
            input_transparent(window);
            XSetInputFocus(dpy, host as Window, RevertToParent, CurrentTime);
            XFlush(dpy);
        }
    }

    /// Keeps keyboard focus on the Tauri host (where the WebView controls live). mpv can grab it
    /// back when it creates its GL child, so this is called repeatedly while playing.
    pub fn focus_host(host: isize) {
        let Some(dpy) = display() else { return };
        focus_host_on(dpy, host);
    }

    pub fn focus_host_on(dpy: *mut Display, host: isize) {
        if dpy.is_null() || host == 0 { return; }
        unsafe {
            XSetErrorHandler(Some(ignore_x_error));
            XSetInputFocus(dpy, host as Window, RevertToParent, CurrentTime);
            XFlush(dpy);
        }
    }

    /// The WM can reparent/unmap children a moment after the host first appears. Re-map and resize
    /// from a short-lived private connection (the container belongs to the long-lived connection,
    /// so closing this one does not destroy it). The size comes from the rectangle the page asked
    /// for: resizing to the host here would put the video back over the player chrome.
    pub fn ensure_visible_later(
        window: isize,
        host: isize,
        rect: Arc<Mutex<Option<(f64, f64, f64, f64)>>>,
        holes: Arc<Mutex<Vec<(f64, f64, f64, f64)>>>,
    ) {
        std::thread::spawn(move || {
            for delay in [150u64, 600, 1500, 3000] {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                unsafe {
                    let dpy = XOpenDisplay(ptr::null());
                    if dpy.is_null() {
                        continue;
                    }
                    // A fresh display connection also needs the quiet handler for *this* thread.
                    unsafe { XSetErrorHandler(Some(ignore_x_error)); }
                    if !alive(dpy, host as Window) || !alive(dpy, window as Window) {
                        XCloseDisplay(dpy);
                        break;
                    }
                    let (host_w, host_h) = size(dpy, host as Window);
                    let wanted = rect.lock().ok().and_then(|rect| *rect);
                    let (x, y, w, h) = match wanted {
                        Some((x, y, w, h)) if w >= 1.0 && h >= 1.0 => (x as i32, y as i32, w as i32, h as i32),
                        Some(_) => (-20000, -20000, 1, 1),
                        None => (0, 0, host_w.max(1), host_h.max(1)),
                    };
                    XMoveResizeWindow(dpy, window as Window, x, y, w as u32, h as u32);
                    let wanted_holes = holes.lock().map(|holes| holes.clone()).unwrap_or_default();
                    let local = local_holes((x as f64, y as f64), (w as f64, h as f64), &wanted_holes);
                    set_holes(window, &local);
                    XMapWindow(dpy, window as Window);
                    XLowerWindow(dpy, window as Window);
                    input_transparent(window as isize);
                    XSetInputFocus(dpy, host as Window, RevertToParent, CurrentTime);
                    XFlush(dpy);
                    XCloseDisplay(dpy);
                }
            }
        });
    }

    pub fn destroy(window: isize) {
        if window == 0 { return; }
        if let Some(dpy) = display() {
            unsafe {
                XDestroyWindow(dpy, window as Window);
                XFlush(dpy);
            }
        }
    }

    #[allow(dead_code)]
    pub fn move_resize(window: isize, x: i32, y: i32, w: i32, h: i32) {
        let Some(dpy) = display() else { return };
        unsafe {
            XMoveResizeWindow(dpy, window as Window, x, y, w.max(1) as u32, h.max(1) as u32);
            XFlush(dpy);
        }
    }

    pub fn client_size(window: isize) -> (i32, i32) {
        let Some(dpy) = display() else { return (1280, 720) };
        client_size_on(dpy, window)
    }

    pub fn client_size_on(dpy: *mut Display, window: isize) -> (i32, i32) {
        if dpy.is_null() { return (1280, 720); }
        size(dpy, window as Window)
    }

    fn parent(dpy: *mut Display, window: Window) -> Option<Window> {
        let (mut r, mut p, mut kids, mut n) = (0, 0, ptr::null_mut(), 0);
        if unsafe { XQueryTree(dpy, window, &mut r, &mut p, &mut kids, &mut n) } == 0 {
            return None;
        }
        if !kids.is_null() {
            unsafe { XFree(kids.cast()) };
        }
        Some(p)
    }

    pub fn fit_to_host(window: isize, host: isize) {
        let Some(dpy) = display() else { return };
        unsafe {
            let (w, h) = size(dpy, host as Window);
            XMoveResizeWindow(dpy, window as Window, 0, 0, w.max(1) as u32, h.max(1) as u32);
            XFlush(dpy);
        }
    }

    /// Sizes the X window exactly to the video rectangle (parent-relative coords).
    pub fn set_rect(window: isize, x: i32, y: i32, w: i32, h: i32) {
        let Some(dpy) = display() else { return };
        unsafe {
            XMoveResizeWindow(dpy, window as Window, x, y, w.max(1) as u32, h.max(1) as u32);
            XFlush(dpy);
        }
    }

    pub fn set_title(window: isize, title: &str) {
        let Some(dpy) = display() else { return };
        if let Ok(name) = CString::new(title) {
            unsafe {
                XStoreName(dpy, window as Window, name.as_ptr().cast());
                XFlush(dpy);
            }
        }
    }

    /// Creates an override-redirect (borderless) top-level window and reparents the video into it.
    pub fn enter_pip(container: isize, title: &str, aspect: f64) -> Result<isize, String> {
        let dpy = display().ok_or("cannot open X display")?;
        unsafe {
            let root = XDefaultRootWindow(dpy);
            let (vw, _) = client_size(container);
            let w = vw.max(320);
            let h = ((w as f64 / aspect).round() as i32).max(180);
            let x = (XDisplayWidth(dpy, XDefaultScreen(dpy)) - w) / 2;
            let y = (XDisplayHeight(dpy, XDefaultScreen(dpy)) - h) / 2;
            let pip = XCreateSimpleWindow(dpy, root, x, y, w as u32, h as u32, 0, 0, 0);
            if pip == 0 {
                return Err("cannot create PiP window".into());
            }
            let mut attrs: XSetWindowAttributes = std::mem::zeroed();
            attrs.override_redirect = 1;
            XChangeWindowAttributes(dpy, pip, CWOverrideRedirect, &mut attrs);
            XSelectInput(dpy, pip, ButtonPressMask | KeyPressMask);
            if let Ok(name) = CString::new(title) {
                XStoreName(dpy, pip, name.as_ptr().cast());
            }
            XMapRaised(dpy, pip);
            XReparentWindow(dpy, container as Window, pip, 0, 0);
            XFlush(dpy);
            Ok(pip as isize)
        }
    }

    pub fn exit_pip(container: isize, host: isize, pip: isize) {
        let Some(dpy) = display() else { return };
        unsafe {
            XReparentWindow(dpy, container as Window, host as Window, 0, 0);
            XDestroyWindow(dpy, pip as Window);
            let (w, h) = size(dpy, host as Window);
            XMoveResizeWindow(dpy, container as Window, 0, 0, w.max(1) as u32, h.max(1) as u32);
            XLowerWindow(dpy, container as Window);
            XFlush(dpy);
        }
    }

    /// Watches a PiP window for a click in its top-right corner or Escape and calls `on_close`.
    pub fn watch_pip_close(pip: isize, on_close: Arc<Mutex<Box<dyn Fn() + Send>>>) {
        std::thread::spawn(move || {
            let Some(dpy) = display() else { return };
            let (pw, _) = client_size(pip);
            let mut event: XEvent = unsafe { std::mem::zeroed() };
            loop {
                unsafe { XNextEvent(dpy, &mut event) };
                let close = unsafe {
                    if event.type_ == ButtonPress {
                        let b = &event.button;
                        b.x >= pw - 48 && b.y <= 48
                    } else if event.type_ == KeyPress {
                        true
                    } else {
                        false
                    }
                };
                if close {
                    if let Ok(cb) = on_close.lock() {
                        cb();
                    }
                    break;
                }
            }
            unsafe { XCloseDisplay(dpy) };
        });
    }

    /// Self-test helper: number of X windows whose class/name mentions mpv under `root`.
    pub fn count_mpv_windows(_root: isize) -> usize {
        let Some(dpy) = display() else { return 0 };
        let mut count = 0;
        unsafe {
            let (mut r, mut p, mut kids, mut n) = (0, 0, ptr::null_mut(), 0);
            if XQueryTree(dpy, _root as Window, &mut r, &mut p, &mut kids, &mut n) != 0 && !kids.is_null() {
                for i in 0..n as usize {
                    let mut name: *mut c_char = ptr::null_mut();
                    if XFetchName(dpy, *kids.add(i), &mut name) != 0 && !name.is_null() {
                        if CStr::from_ptr(name).to_string_lossy().to_lowercase().contains("mpv") {
                            count += 1;
                        }
                        XFree(name.cast());
                    }
                }
                XFree(kids.cast());
            }
        }
        count
    }
}

fn http_header_lines(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let (name, value) = (name.trim(), value.trim());
            (!name.is_empty() && !value.is_empty()).then(|| format!("{name}: {value}"))
        })
        .collect()
}

fn load_options(request: &LoadRequest) -> String {
    let mut options = Vec::new();
    if request.start_seconds > 0.0 {
        options.push(format!("start={:.3}", request.start_seconds));
    }
    if let Some(audio) = request.audio_url.as_deref().filter(|a| !a.is_empty()) {
        options.push(format!("audio-files=%{}%{audio}", audio.len()));
    }
    options.join(",")
}

/// Linux mpv options, adapted from the Windows `startup_options` and the upstream Linux bridge.
fn startup_options(video: &VideoConfig, gpu_context: &str, hwdec: &str) -> Vec<(String, String)> {
    let mut o: Vec<(String, String)> = Vec::new();
    let mut set = |k: &str, v: &str| o.push((k.to_string(), v.to_string()));
    set("config", "no");
    set("terminal", "no");
    set("osc", "no");
    set("osd-level", "1");
    set("input-default-bindings", "yes");
    set("input-vo-keyboard", "no");
    set("input-cursor", "yes");
    set("cursor-autohide", "no");
    set("keep-open", "yes");
    set("idle", "yes");
    set("volume-max", "200");
    set("vo", "gpu");
    set("gpu-context", gpu_context);
    set("force-window", "immediate");
    set("force-seekable", "yes");
    set("audio-channels", "auto");
    if video.audio_passthrough {
        set("audio-spdif", "ac3,eac3,dts,dts-hd,truehd");
        if let Ok(device) = std::env::var("NUVIO_MPV_AUDIO_DEVICE") {
            let device = device.trim();
            if !device.is_empty() {
                set("audio-device", device);
            }
        }
    }
    if !video.subtitle_codepage.trim().is_empty() {
        set("sub-codepage", video.subtitle_codepage.trim());
    }
    set("hwdec", hwdec);
    set("gpu-hwdec-interop", "auto");
    set("hwdec-codecs", "all");
    set("target-colorspace-hint", "yes");
    set("target-colorspace-hint-mode", "source");
    match video.decoder_priority {
        DecoderPriority::HardwareOnly => {
            set("vd-lavc-software-fallback", "no");
        }
        DecoderPriority::SoftwareOnly => {
            set("hwdec", "no");
            set("vd-lavc-software-fallback", "yes");
        }
        DecoderPriority::HardwarePreferred => set("vd-lavc-software-fallback", "yes"),
    }
    set("vd-lavc-threads", "4");
    set("tone-mapping", "bt.2390");
    set("dither-depth", "auto");
    set("deband", "yes");
    set("scale", "spline36");
    set("cscale", "spline36");
    set("demuxer-max-bytes", "150MiB");
    set("demuxer-max-back-bytes", "50MiB");
    set("demuxer-seekable-cache", "yes");
    set("stream-lavf-o", "reconnect=1,reconnect_streamed=1,reconnect_on_network_error=1,reconnect_on_http_error=%23%408,429,500,502,503,504,reconnect_delay_max=15");
    if let Some(dir) = shader_cache_dir() {
        set("gpu-shader-cache-dir", &dir);
    }
    set("background", "color");
    set("background-color", "#000000");
    o
}

fn shader_cache_dir() -> Option<String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?;
    let dir = base.join("NuvioNextGen").join("mpv-shaders");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.to_string_lossy().into_owned())
}

pub struct Player {
    mpv: Option<Arc<Mpv>>,
    host: isize,
    container: isize,
    stop: Arc<AtomicBool>,
    events: Option<JoinHandle<()>>,
    snapshot: Arc<Mutex<PlayerSnapshot>>,
    applied_style: Mutex<Option<AppliedSubtitleStyle>>,
    /// The video rectangle the page asked for; shared with the delayed re-map so a window manager
    /// reparent does not put the video back over the controls.
    video_rect: Arc<Mutex<Option<(f64, f64, f64, f64)>>>,
    /// Areas of the video window the page needs for the chrome (window coordinates, physical px).
    video_holes: Arc<Mutex<Vec<(f64, f64, f64, f64)>>>,
    resize_mode: Mutex<ResizeMode>,
    pip: Mutex<Option<PipSession>>,
    /// Keeps mpv's recreated video children input-transparent so the HTML controls stay clickable.
    input_watcher: Option<JoinHandle<()>>,
}

impl Player {
    pub fn open(api: Arc<MpvApi>, host_xid: isize, session: u64, request: &LoadRequest, sink: SnapshotSink) -> Result<Self, PlayerError> {
        let host = if host_xid == 0 {
            x11::find_host("Nuvio Astra").ok_or_else(|| PlayerError::Mpv("main window not found (X11 required)".into()))?
        } else {
            host_xid
        };
        let container = x11::create_container(host).map_err(PlayerError::Mpv)?;
        match Self::start(api, host, container, session, request, sink) {
            Ok(player) => Ok(player),
            Err(error) => {
                x11::destroy(container);
                Err(error)
            }
        }
    }

    fn start(api: Arc<MpvApi>, host: isize, container: isize, session: u64, request: &LoadRequest, sink: SnapshotSink) -> Result<Self, PlayerError> {
        // GPU context attempts: x11egl on Mesa, x11vk on NVIDIA proprietary drivers (upstream bridge).
        let env_ctx = std::env::var("NUVIO_MPV_GPU_CONTEXT").ok();
        let env_hwdec = std::env::var("NUVIO_MPV_HWDEC").ok();
        let attempts: Vec<(&str, &str)> = match (env_ctx.as_deref(), env_hwdec.as_deref()) {
            (Some(ctx), hw) => vec![(ctx, hw.unwrap_or("auto"))],
            (None, Some(hw)) => vec![("x11egl", hw), ("x11vk", hw)],
            _ => vec![("x11egl", "auto"), ("x11vk", "auto")],
        };

        let mut last_error = None;
        let mut attempt = 0usize;
        let video_rect: Arc<Mutex<Option<(f64, f64, f64, f64)>>> = Arc::new(Mutex::new(None));
        let video_holes: Arc<Mutex<Vec<(f64, f64, f64, f64)>>> = Arc::new(Mutex::new(Vec::new()));
        let mpv = loop {
            let Some((ctx, hwdec)) = attempts.get(attempt).copied() else {
                return Err(PlayerError::Mpv(format!("mpv initialization failed: {}", last_error.unwrap_or_default())));
            };
            attempt += 1;
            let candidate = Arc::new(Mpv::create(api.clone())?);
            for (name, value) in startup_options(&request.video, ctx, hwdec) {
                candidate.set_option(&name, &value)?;
            }
            let headers = http_header_lines(&request.headers);
            if !headers.is_empty() {
                candidate.set_option_string_list("http-header-fields", &headers)?;
            }
            candidate.set_option_i64("wid", container as i64)?;
            match candidate.initialize() {
                Ok(()) => {
                    // mpv/WM may have left the foreign `wid` unmapped; make it visible now and
                    // again after the WM finishes reparenting the host window.
                    x11::map(container, host);
                    x11::ensure_visible_later(container, host, video_rect.clone(), video_holes.clone());
                    break candidate;
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    eprintln!("[player] gpu-context={ctx} failed ({e}), retrying");
                }
            }
        };

        for &(id, name, format) in OBSERVED {
            mpv.observe(id, name, format)?;
        }

        let snapshot = Arc::new(Mutex::new(PlayerSnapshot { session, sub_visible: true, speed: 1.0, ..Default::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        let events = {
            let (mpv, stop, snapshot) = (mpv.clone(), stop.clone(), snapshot.clone());
            std::thread::Builder::new()
                .name("nuvio-mpv-events".into())
                .spawn(move || event_loop(mpv, stop, snapshot, sink))
                .map_err(|e| PlayerError::Mpv(format!("event thread: {e}")))?
        };
        // mpv recreates its GL child on every file load; reapply the empty input shape so pointer
        // events keep falling through to the Tauri WebView that paints the player controls.
        let input_watcher = {
            let stop = stop.clone();
            std::thread::Builder::new().name("nuvio-x11-input".into()).spawn(move || {
                let dpy = x11::open_private_display();
                while !stop.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(250));
                    x11::input_transparent_on(dpy, container);
                    x11::focus_host_on(dpy, host);
                }
                x11::close_private_display(dpy);
            }).ok()
        };
        // The page reserves the video rectangle on Linux (the WebView cannot paint over the
        // native video window): the container is kept inside it by `set_video_rect`.
        let player = Self {
            mpv: Some(mpv),
            host,
            container,
            stop,
            events: Some(events),
            snapshot,
            applied_style: Mutex::new(None),
            video_rect,
            video_holes,
            resize_mode: Mutex::new(ResizeMode::Fit),
            pip: Mutex::new(None),
            input_watcher,
        };
        player.apply_subtitle_style(&SubtitleStyle::default())?;
        player.load(request)?;
        Ok(player)
    }

    fn mpv(&self) -> &Mpv { self.mpv.as_deref().expect("player used after shutdown") }

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

    pub fn snapshot(&self) -> PlayerSnapshot { self.snapshot.lock().expect("snapshot lock").clone() }

    pub fn execute(&self, command: &PlayerCommand) -> Result<(), PlayerError> {
        let mpv = self.mpv();
        match command {
            PlayerCommand::SetPaused { paused } => mpv.set_property("pause", if *paused { "yes" } else { "no" }),
            PlayerCommand::TogglePause => mpv.command(&["cycle", "pause"]),
            PlayerCommand::Seek { seconds, relative } => mpv.command(&[
                "seek", &format!("{seconds:.3}"), if *relative { "relative+exact" } else { "absolute+exact" },
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
                if url.is_empty() { Ok(()) } else { mpv.command(&["sub-add", url, "select", title.as_deref().unwrap_or(""), lang.as_deref().unwrap_or("")]) }
            }
            PlayerCommand::RemoveExternalSubtitles { select_id } => {
                let count = mpv.get_i64("track-list/count").unwrap_or(0);
                for index in (0..count).rev() {
                    let p = format!("track-list/{index}");
                    let external = mpv.get_string(&format!("{p}/type")).as_deref() == Some("sub")
                        && mpv.get_flag(&format!("{p}/external")).unwrap_or(false);
                    if external {
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
            PlayerCommand::SetSubtitleDelay { ms } => mpv.set_property_f64("sub-delay", (*ms).clamp(-SUBTITLE_DELAY_LIMIT_MS, SUBTITLE_DELAY_LIMIT_MS) as f64 / 1000.0),
            PlayerCommand::SetAudioDelay { ms } => mpv.set_property_f64("audio-delay", (*ms).clamp(-AUDIO_DELAY_LIMIT_MS, AUDIO_DELAY_LIMIT_MS) as f64 / 1000.0),
            PlayerCommand::SetAudioAmplification { db } => {
                let gain = 100.0 * 10f64.powf((*db).clamp(0, 10) as f64 / 20.0);
                mpv.set_property_f64("volume", gain.min(1000.0))
            }
            PlayerCommand::SetAudioDownmix { layout } => mpv.set_property("audio-channels", if layout.is_empty() { "auto" } else { layout }),
            PlayerCommand::SetBuffer { max_seconds, back_seconds, max_bytes } => {
                mpv.set_property_f64("cache-secs", max_seconds.max(0.0))?;
                mpv.set_property_f64("demuxer-readahead-secs", back_seconds.max(0.0))?;
                mpv.set_property("demuxer-max-bytes", &(*max_bytes).max(0).to_string())
            }
            PlayerCommand::ApplySubtitleStyle { style } => self.apply_subtitle_style(style),
            PlayerCommand::SetMotion { .. } | PlayerCommand::SetEnhancement { .. } | PlayerCommand::SetPicture { .. } | PlayerCommand::SetCompare { .. } => Ok(()),
            PlayerCommand::SetResizeMode { mode } => {
                *self.resize_mode.lock().expect("mode lock") = *mode;
                let in_pip = self.pip.lock().expect("pip lock").is_some();
                self.apply_resize_mode(if in_pip { ResizeMode::Fit } else { *mode })
            }
        }
    }

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
        if changed(|p, n| p.position != n.position) { mpv.set_property("sub-pos", &next.position.to_string())?; }
        if changed(|p, n| p.text_color != n.text_color) { mpv.set_property("sub-color", &next.text_color)?; }
        if changed(|p, n| p.background_color != n.background_color) {
            mpv.set_property("sub-back-color", &next.background_color)?;
            let border = if next.background_color.starts_with("#00") { "outline-and-shadow" } else { "background-box" };
            mpv.set_property("sub-border-style", border)?;
        }
        if changed(|p, n| p.outline_color != n.outline_color) { mpv.set_property("sub-outline-color", &next.outline_color)?; }
        if changed(|p, n| p.bold != n.bold) { mpv.set_property("sub-bold", if next.bold { "yes" } else { "no" })?; }
        if changed(|p, n| p.outline_size != n.outline_size) { mpv.set_property_f64("sub-outline-size", next.outline_size)?; }
        if prev.is_none_or(|p| p.strip_sdh != next.strip_sdh) {
            let flag = if next.strip_sdh { "yes" } else { "no" };
            mpv.set_property("sub-filter-sdh", flag)?;
            mpv.set_property("sub-filter-sdh-harder", flag)?;
        }
        *applied = Some(next);
        drop(applied);
        if let Some(dir) = style.fonts_dir.as_deref().filter(|d| !d.is_empty()) {
            mpv.set_property("sub-fonts-dir", dir)?;
        }
        mpv.set_property("sub-font", &style.font)?;
        mpv.set_property("sub-shadow-color", &style.mpv_shadow_color())?;
        mpv.set_property_f64("sub-shadow-offset", style.mpv_shadow_offset())
    }

    pub fn set_video_rect(&self, rect: Option<(f64, f64, f64, f64)>, holes: Vec<(f64, f64, f64, f64)>) -> Result<(), PlayerError> {
        *self.video_rect.lock().expect("rect lock") = rect;
        *self.video_holes.lock().expect("holes lock") = holes;
        if self.pip.lock().expect("pip lock").is_some() { return Ok(()); }
        self.place_video(rect)
    }

    /// Moves the video container to `rect` (physical pixels, host client coordinates); `None` is
    /// the whole client area. The WebView paints below the X11 children, so it cannot draw the
    /// player chrome over the video: the page either reserves the video rectangle (letterbox) or
    /// covers the whole window and asks for `holes` where the chrome goes (`FullPlayer.tsx`).
    /// A zero-size rectangle parks mpv off-screen — an X window cannot be 0×0 and unmapping it can
    /// break the GPU surface on some X servers.
    fn place_video(&self, rect: Option<(f64, f64, f64, f64)>) -> Result<(), PlayerError> {
        let (host_w, host_h) = x11::client_size(self.host);
        let (x, y, w, h) = rect.unwrap_or((0.0, 0.0, host_w as f64, host_h as f64));
        let holes = self.video_holes.lock().map(|holes| holes.clone()).unwrap_or_default();
        if w < 1.0 || h < 1.0 {
            x11::set_rect(self.container, -20000, -20000, 1, 1);
            x11::set_holes(self.container, &[]);
        } else {
            x11::set_rect(self.container, x as i32, y as i32, w as i32, h as i32);
            let local = x11::local_holes((x, y), (w, h), &holes);
            x11::set_holes(self.container, &local);
        }
        self.apply_margins(None)
    }

    fn apply_margins(&self, rect: Option<(f64, f64, f64, f64)>) -> Result<(), PlayerError> {
        let mpv = self.mpv();
        let (w, h) = x11::client_size(self.container);
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

    pub fn fit_to_host(&self) {
        if self.pip.lock().expect("pip lock").is_none() {
            // Re-map in case the WM reparented the host after the container was created.
            x11::map(self.container, self.host);
            let rect = *self.video_rect.lock().expect("rect lock");
            let _ = self.place_video(rect);
        }
    }

    pub fn is_picture_in_picture(&self) -> bool { self.pip.lock().expect("pip lock").is_some() }

    pub fn enter_picture_in_picture(&self, _window: isize) -> Result<(), PlayerError> {
        let mut pip = self.pip.lock().expect("pip lock");
        if pip.is_some() { return Ok(()); }
        let s = self.snapshot();
        let aspect = if s.display_width > 0 && s.display_height > 0 {
            s.display_width as f64 / s.display_height as f64
        } else if s.width > 0 && s.height > 0 {
            s.width as f64 / s.height as f64
        } else { 16.0 / 9.0 };
        let window = x11::enter_pip(self.container, "Nuvio Astra", aspect).map_err(PlayerError::Mpv)?;
        let previous_mode = *self.resize_mode.lock().expect("mode lock");
        *pip = Some(PipSession {
            window,
            previous_rect: *self.video_rect.lock().expect("rect lock"),
            previous_mode,
        });
        x11::watch_pip_close(window, Arc::new(Mutex::new(Box::new(|| {}) as Box<dyn Fn() + Send>)));
        drop(pip);
        self.apply_margins(None)?;
        self.apply_resize_mode(ResizeMode::Fit)?;
        self.snapshot.lock().expect("snapshot lock").picture_in_picture = true;
        Ok(())
    }

    pub fn exit_picture_in_picture(&self) -> Result<(), PlayerError> {
        let Some(session) = self.pip.lock().expect("pip lock").take() else { return Ok(()) };
        x11::exit_pip(self.container, self.host, session.window);
        self.apply_resize_mode(session.previous_mode)?;
        // Back in the main window: the container has to return to the page's video rectangle
        // (`exit_pip` leaves it at the host size, which would cover the controls).
        self.place_video(session.previous_rect)?;
        self.snapshot.lock().expect("snapshot lock").picture_in_picture = false;
        Ok(())
    }

    pub fn set_window_title(&self, title: &str) {
        if let Some(session) = self.pip.lock().expect("pip lock").as_ref() {
            x11::set_title(session.window, title);
        }
    }

    pub fn break_display_sync(&self) -> Result<(), PlayerError> { Ok(()) }

    pub fn container_hwnd(&self) -> isize { self.container }

    pub fn property(&self, name: &str) -> Option<String> { self.mpv().get_string(name) }

    pub fn shutdown(&mut self) {
        if self.mpv.is_none() { return; }
        let _ = self.exit_picture_in_picture();
        self.stop.store(true, Ordering::SeqCst);
        if let Some(mpv) = &self.mpv { mpv.wakeup(); }
        if let Some(events) = self.events.take() { let _ = events.join(); }
        if let Some(watcher) = self.input_watcher.take() { let _ = watcher.join(); }
        self.mpv.take();
        x11::destroy(self.container);
    }
}

impl Drop for Player {
    fn drop(&mut self) { self.shutdown(); }
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

fn error_text(code: i32) -> String {
    match code {
        -13 => "loading failed (network, format or protocol)".into(),
        -14 => "unsupported or unknown format".into(),
        -15 => "nothing to play (no audio or video stream)".into(),
        -16 => "video output initialization failed".into(),
        -17 => "audio output initialization failed".into(),
        other => format!("playback failed (mpv error {other})"),
    }
}

fn event_loop(mpv: Arc<Mpv>, stop: Arc<AtomicBool>, snapshot: Arc<Mutex<PlayerSnapshot>>, sink: SnapshotSink) {
    let mut last_emit = Instant::now() - EMIT_INTERVAL;
    let mut dirty = false;
    while !stop.load(Ordering::SeqCst) {
        // SAFETY: this thread is the only one waiting for events on this handle.
        let event = unsafe { &*mpv.wait_event(0.25) };
        let mut urgent = false;
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
                    if event.event_id == EVENT_FILE_LOADED { s.loaded = true; }
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
                    let f64v = || (prop.format == FORMAT_DOUBLE && !prop.data.is_null()).then(|| unsafe { *(prop.data as *const f64) });
                    let i64v = || (prop.format == FORMAT_INT64 && !prop.data.is_null()).then(|| unsafe { *(prop.data as *const i64) });
                    let flag = || (prop.format == FORMAT_FLAG && !prop.data.is_null()).then(|| unsafe { *(prop.data as *const i32) } != 0);
                    let string = || {
                        (prop.format == FORMAT_STRING && !prop.data.is_null()).then(|| unsafe {
                            CStr::from_ptr(*(prop.data as *const *const c_char)).to_string_lossy().into_owned()
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
                        "eof-reached" => { if flag() == Some(true) { s.ended = true; urgent = true; } }
                        _ => {}
                    }
                }
                _ => {}
            }
            if event.event_id != EVENT_NONE { dirty = true; }
            if dirty && (urgent || last_emit.elapsed() >= EMIT_INTERVAL) {
                sink(&s);
                last_emit = Instant::now();
                dirty = false;
            }
        }
    }
}

/// Helper used by the Windows self-test only; kept for API parity.
pub mod win {
    pub fn count_mpv_windows(root: isize) -> usize {
        super::x11::count_mpv_windows(root)
    }
}
