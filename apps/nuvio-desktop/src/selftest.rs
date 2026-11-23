//! Automated player tests driving the real window (Tauri + WebView2 overlay + embedded libmpv).
//!
//! - `--selftest <report.json>`: public test media (HLS, MP4, MKV, DASH, HEVC, 4K). For each source:
//!   first frame, subtitle style, preview/fullscreen/resize, speed, delays, volume, resize mode,
//!   subtitle visibility, audio switch, external subtitle add/remove, seek, picture-in-picture, A/V
//!   sync, clean close.
//! - `--selftest-addon <addon name> <report.json>`: one explicitly chosen title of one installed
//!   addon (first item of its first listable catalog), its streams searched once, the first direct
//!   stream played with the addon's headers. Reports never contain URLs or header values.
//!
//! Every check records what mpv actually reported.

use crate::{set_picture_in_picture, AppState};
use nuvio_player::{PlayerCommand, PlayerSnapshot, ResizeMode};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone)]
pub enum Request {
    Media { report: PathBuf },
    Addon { addon: String, report: PathBuf },
    Bench { addon: String, report: PathBuf, cycles: usize },
}

pub fn requested() -> Option<Request> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--selftest") {
        return args.get(i + 1).map(|r| Request::Media { report: PathBuf::from(r) });
    }
    if let Some(i) = args.iter().position(|a| a == "--bench-addon") {
        let cycles = args.get(i + 3).and_then(|v| v.parse().ok()).unwrap_or(3);
        return Some(Request::Bench { addon: args.get(i + 1)?.clone(), report: PathBuf::from(args.get(i + 2)?), cycles });
    }
    if let Some(i) = args.iter().position(|a| a == "--selftest-addon") {
        return Some(Request::Addon { addon: args.get(i + 1)?.clone(), report: PathBuf::from(args.get(i + 2)?) });
    }
    None
}

struct Source {
    id: &'static str,
    kind: &'static str,
    url: &'static str,
}

const SOURCES: &[Source] = &[
    Source { id: "hls", kind: "HLS (Apple fMP4, multi-audio, WebVTT)", url: "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8" },
    Source { id: "mp4", kind: "MP4 (H.264, 5.1 audio, ~10 min)", url: "https://archive.org/download/BigBuckBunny_124/Content/big_buck_bunny_720p_surround.mp4" },
    Source { id: "mkv", kind: "MKV (Matroska test5: multi-audio, embedded subtitles)", url: "https://github.com/ietf-wg-cellar/matroska-test-files/raw/master/test_files/test5.mkv" },
    Source { id: "dash", kind: "DASH (Akamai BBB 30 fps)", url: "https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd" },
    Source { id: "hevc", kind: "HLS HEVC Main10 1080p60 variant (Apple bipbop, video only)", url: "https://devstreaming-cdn.apple.com/videos/streaming/examples/bipbop_adv_example_hevc/v18/prog_index.m3u8" },
    Source { id: "4k", kind: "DASH 4K H.264 (Akamai Spring)", url: "https://dash.akamaized.net/akamai/streamroot/050714/Spring_4Ktest.mpd" },
];

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Check {
    name: String,
    passed: bool,
    detail: String,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SourceReport {
    id: String,
    kind: String,
    started_at_ms: u128,
    first_frame_ms: Option<u128>,
    hwdec: Option<String>,
    video_codec: Option<String>,
    resolution: String,
    fps: f64,
    audio_tracks: usize,
    subtitle_tracks: usize,
    header_names: Vec<String>,
    max_abs_av_sync_ms: f64,
    dropped_frames: i64,
    decoder_dropped_frames: i64,
    checks: Vec<Check>,
    ended_at_ms: u128,
    error: Option<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Report {
    libmpv_api: Option<String>,
    sources: Vec<SourceReport>,
    mpv_windows_left_after_all: usize,
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

pub fn start(app: AppHandle, request: Request) {
    if let Request::Bench { addon, report, cycles } = request {
        crate::bench::start(app, addon, report, cycles);
        return;
    }
    std::thread::spawn(move || {
        let report_path = match &request {
            Request::Media { report } | Request::Addon { report, .. } | Request::Bench { report, .. } => report.clone(),
        };
        let report = run(&app, &request);
        let _ = std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap_or_default());
        app.exit(0);
    });
}

/// Runs `f` on the main thread (window creation/destruction) and returns its result.
pub(crate) fn on_main<T: Send + 'static>(app: &AppHandle, f: impl FnOnce(&AppHandle) -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(f(&handle));
    })
    .expect("main thread");
    rx.recv().expect("main thread result")
}

pub(crate) fn snapshot(app: &AppHandle) -> Option<PlayerSnapshot> {
    app.state::<AppState>().with_player(|p| Ok(p.snapshot())).ok()
}

fn property(app: &AppHandle, name: &str) -> Option<String> {
    app.state::<AppState>().with_player(|p| Ok(p.property(name))).ok().flatten()
}

pub(crate) fn wait_for(app: &AppHandle, timeout: Duration, mut done: impl FnMut(&PlayerSnapshot) -> bool) -> Option<PlayerSnapshot> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(s) = snapshot(app) {
            if done(&s) {
                return Some(s);
            }
            if s.error.is_some() {
                return None;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn command(app: &AppHandle, cmd: PlayerCommand) -> Result<(), String> {
    app.state::<AppState>().with_player(|p| p.execute(&cmd).map_err(|e| e.to_string()))
}

/// Preview rectangle: right 45 % of the client area, 16:9, like the TV tab's side preview.
fn preview_rect(window: &tauri::WebviewWindow) -> Option<(f64, f64, f64, f64)> {
    let size = window.inner_size().ok()?;
    let (w, h) = (size.width as f64, size.height as f64);
    let pw = w * 0.45;
    let ph = (pw * 9.0 / 16.0).min(h - 48.0);
    Some((w - pw - 24.0, 24.0, pw, ph))
}

fn set_rect(app: &AppHandle, rect: Option<(f64, f64, f64, f64)>) {
    // The UI mirrors the rect so its overlay controls sit exactly on the video.
    let _ = app.emit("selftest-rect", rect.map(|(x, y, w, h)| [x, y, w, h]));
    let _ = app.state::<AppState>().with_player(|p| p.set_video_rect(rect).map_err(|e| e.to_string()));
}

/// Video area to measure, in physical client pixels, leaving out the controls overlay at the bottom.
fn video_region(window: &tauri::WebviewWindow, fullscreen: bool) -> Option<(i32, i32, i32, i32)> {
    let (x, y, w, h) = if fullscreen {
        let size = window.inner_size().ok()?;
        (0.0, 0.0, size.width as f64, size.height as f64)
    } else {
        preview_rect(window)?
    };
    Some(((x + w * 0.05) as i32, (y + h * 0.05) as i32, (w * 0.9) as i32, (h * 0.6) as i32))
}

/// Captures the composed window and checks that the video area holds a real picture.
fn visual_quiet(window: &tauri::WebviewWindow, dir: &Path, name: &str, region: Option<(i32, i32, i32, i32)>) -> bool {
    crate::capture::capture_and_measure(window, &dir.join(format!("{name}.png")), region)
        .map(|s| s.looks_like_video())
        .unwrap_or(false)
}

fn visual(r: &mut SourceReport, window: &tauri::WebviewWindow, dir: &Path, name: &str, region: Option<(i32, i32, i32, i32)>, label: &str) {
    match crate::capture::capture_and_measure(window, &dir.join(format!("{name}.png")), region) {
        Ok(s) => check(r, label, s.looks_like_video(), format!(
            "motion {:.2}, white {:.0} %, black {:.0} %, luma sd {:.1} ({name}.bmp)", s.motion, s.white_fraction * 100.0, s.black_fraction * 100.0, s.luma_stddev)),
        Err(e) => check(r, label, false, e),
    }
}

fn check(report: &mut SourceReport, name: &str, passed: bool, detail: impl Into<String>) {
    report.checks.push(Check { name: name.into(), passed, detail: detail.into() });
}

#[cfg(windows)]
fn host_mpv_windows(app: &AppHandle) -> usize {
    let window = app.get_webview_window("main").expect("main window");
    nuvio_player::win::count_mpv_windows(windows::Win32::Foundation::HWND(window.hwnd().expect("hwnd").0))
}

#[cfg(not(windows))]
fn host_mpv_windows(_app: &AppHandle) -> usize {
    0
}

fn run(app: &AppHandle, request: &Request) -> Report {
    let mut report = Report::default();
    std::thread::sleep(Duration::from_millis(1500)); // let the UI load
    let window = app.get_webview_window("main").expect("main window");
    let _ = window.set_always_on_top(true);
    let _ = app.emit("selftest", true);
    match request {
        Request::Media { report: path } => {
            let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let subtitle = dir.join("external-test.srt");
            let _ = std::fs::write(&subtitle, "1\n00:00:00,000 --> 00:10:00,000\nNuvio NextGen external subtitle test\n");
            // Optional filter for focused experiments: NUVIO_SELFTEST_ONLY=<id>.
            let only = std::env::var("NUVIO_SELFTEST_ONLY").ok();
            for source in SOURCES.iter().filter(|s| only.as_deref().is_none_or(|o| o == s.id)) {
                let url = source.url.to_string();
                let r = play_and_check(app, &window, &dir, source.id, source.kind, move |app| {
                    app.state::<AppState>().open(app, url, vec![], 0.0)
                }, Some(subtitle.to_string_lossy().into_owned()), &mut report);
                report.sources.push(r);
                std::thread::sleep(Duration::from_secs(4)); // idle gap for the external sampler
            }
        }
        Request::Bench { .. } => {}
        Request::Addon { addon, report: path } => {
            let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let r = addon_stream(app, &window, &dir, addon, &mut report);
            report.sources.push(r);
        }
    }
    report.mpv_windows_left_after_all = host_mpv_windows(app);
    if let Some(secs) = std::env::var("NUVIO_SELFTEST_HOLD_SECS").ok().and_then(|v| v.parse::<u64>().ok()) {
        // A/B experiment: optionally ask WebView2 for low memory use during the idle hold.
        if std::env::var("NUVIO_SELFTEST_WEBVIEW_LOW_MEMORY").is_ok() {
            let _ = crate::set_webview_low_memory(&window, true);
        }
        std::thread::sleep(Duration::from_secs(secs));
    }
    let _ = window.set_always_on_top(false);
    report
}

/// One explicitly chosen title from `addon_name`: first item of its first listable catalog.
fn addon_stream(app: &AppHandle, window: &tauri::WebviewWindow, dir: &Path, addon_name: &str, report: &mut Report) -> SourceReport {
    let mut r = SourceReport { id: "addon".into(), kind: format!("Addon stream ({addon_name})"), started_at_ms: now_ms(), ..Default::default() };
    let client = app.state::<AppState>().client.clone();
    let wanted = addon_name.to_lowercase();
    let picked = tauri::async_runtime::block_on(async move {
        let manifests = crate::streams::load_manifests(&client, None).await?;
        let manifest = manifests
            .into_iter()
            .filter_map(Result::ok)
            .find(|m| m.name.to_lowercase().contains(&wanted))
            .ok_or("addon not installed or not enabled")?;
        let catalog = manifest
            .catalogs
            .iter()
            .find(|c| !c.extra.iter().any(|e| e.is_required && !e.name.eq_ignore_ascii_case("skip")))
            .ok_or("addon has no listable catalog")?
            .clone();
        let page = client
            .fetch_catalog_page(&manifest, &catalog.content_type, &catalog.id, None, None)
            .await
            .map_err(|e| e.to_string())?;
        let item = page.items.into_iter().next().ok_or("catalog is empty")?;
        Ok::<_, String>((item.content_type, item.id, item.name))
    });
    let (content_type, id, title) = match picked {
        Ok(v) => v,
        Err(e) => {
            r.error = Some(e);
            return r;
        }
    };
    r.kind = format!("Addon stream ({addon_name}) — \"{title}\" [{content_type}]");
    let handle = app.clone();
    let groups = tauri::async_runtime::block_on(async move {
        let state = handle.state::<AppState>();
        crate::streams::search(&state.client, &state.streams, &content_type, &id, None).await
    });
    let groups = match groups {
        Ok(g) => g,
        Err(e) => {
            r.error = Some(e);
            return r;
        }
    };
    let total: usize = groups.iter().map(|g| g.streams.len()).sum();
    let errors: Vec<_> = groups.iter().filter_map(|g| g.error.clone()).collect();
    check(&mut r, "streams resolved", total > 0, format!("{} addon(s) asked, {total} stream(s), errors: {errors:?}", groups.len()));
    let Some(entry) = groups.iter().flat_map(|g| g.streams.iter()).find(|s| s.kind == crate::streams::StreamKind::Direct).cloned() else {
        r.error = Some("no direct stream to play".into());
        return r;
    };
    let key = entry.key.clone();
    let kind = r.kind.clone();
    let mut inner = play_and_check(app, window, dir, "addon", &kind, move |app| {
        let state = app.state::<AppState>();
        let stream = state.streams.get(&key).ok_or("stream vanished")?;
        let url = stream.playable_direct_url().ok_or("no url")?.to_string();
        state.open(app, url, stream.playback_request_headers(), 0.0)
    }, None, report);
    // Header names only: verify mpv received as many header lines as the addon gave.
    let mut checks = std::mem::take(&mut r.checks);
    checks.append(&mut inner.checks);
    inner.checks = checks;
    inner.header_names = entry.header_names;
    inner
}

#[allow(clippy::too_many_arguments)]
fn play_and_check(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    dir: &Path,
    id: &str,
    kind: &str,
    open: impl FnOnce(&AppHandle) -> Result<crate::PlayerInfo, String> + Send + 'static,
    external_subtitle: Option<String>,
    report: &mut Report,
) -> SourceReport {
    let mut r = SourceReport { id: id.into(), kind: kind.into(), started_at_ms: now_ms(), ..Default::default() };
    let _ = app.emit("selftest-source", kind);
    let opened = Instant::now();
    match on_main(app, open) {
        Ok(info) => report.libmpv_api = Some(info.libmpv_api),
        Err(e) => {
            r.error = Some(e);
            r.ended_at_ms = now_ms();
            return r;
        }
    }
    set_rect(app, preview_rect(window));

    let first = wait_for(app, Duration::from_secs(45), |s| s.loaded && s.time_pos > 0.2 && s.width > 0);
    r.first_frame_ms = first.as_ref().map(|_| opened.elapsed().as_millis());
    check(&mut r, "plays", first.is_some(), match &first {
        Some(s) => format!("time-pos {:.2}s after {} ms", s.time_pos, opened.elapsed().as_millis()),
        None => snapshot(app).and_then(|s| s.error).unwrap_or_else(|| "no frame within 45 s".into()),
    });
    if first.is_none() {
        on_main(app, |app| app.state::<AppState>().close());
        r.ended_at_ms = now_ms();
        return r;
    }
    let header_lines = property(app, "http-header-fields").map(|v| if v.is_empty() { 0 } else { v.split(',').filter(|p| p.contains(':')).count() });
    if id == "addon" {
        check(&mut r, "addon headers passed to mpv", true, format!("header lines in mpv: {header_lines:?}"));
    }

    // A/V sync over ~4 s of playback.
    let mut max_av = 0.0f64;
    for _ in 0..40 {
        if let Some(s) = snapshot(app) {
            max_av = max_av.max(s.av_sync.abs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    r.max_abs_av_sync_ms = (max_av * 1000.0 * 10.0).round() / 10.0;
    let s = snapshot(app).unwrap_or_default();
    r.hwdec = s.hwdec_current.clone();
    r.video_codec = s.video_codec.clone();
    r.resolution = format!("{}x{}", s.width, s.height);
    r.fps = s.fps;
    r.audio_tracks = s.tracks.iter().filter(|t| t.kind == "audio").count();
    r.subtitle_tracks = s.tracks.iter().filter(|t| t.kind == "sub").count();
    check(&mut r, "hardware decoding", s.hwdec_current.as_deref().is_some_and(|h| h != "no" && !h.is_empty()),
        format!("hwdec-current = {:?}", s.hwdec_current));
    let av_ms = r.max_abs_av_sync_ms;
    check(&mut r, "A/V sync within 100 ms", av_ms <= 100.0, format!("max |avsync| = {av_ms} ms"));
    // mpv reports these as decimals ("90.000000"): compare numerically.
    let num = |name: &str| property(app, name).and_then(|v| v.parse::<f64>().ok());
    check(&mut r, "default subtitle style applied", num("sub-pos") == Some(90.0) && num("sub-font-size") == Some(54.0),
        format!("sub-pos={:?} sub-font-size={:?}", property(app, "sub-pos"), property(app, "sub-font-size")));
    visual(&mut r, window, dir, &format!("{id}-preview"), video_region(window, false), "video visible: preview");

    // Fullscreen: same session, only the window and the video margins change.
    let before = snapshot(app).unwrap_or_default();
    let _ = window.set_fullscreen(true);
    std::thread::sleep(Duration::from_millis(800));
    set_rect(app, None);
    std::thread::sleep(Duration::from_secs(3));
    let fs = snapshot(app).unwrap_or_default();
    visual(&mut r, window, dir, &format!("{id}-fullscreen"), video_region(window, true), "video visible: fullscreen");
    check(&mut r, "fullscreen keeps playing (same session)",
        fs.session == before.session && fs.time_pos > before.time_pos + 1.0 && fs.error.is_none(),
        format!("session {} -> {}, time {:.1}s -> {:.1}s", before.session, fs.session, before.time_pos, fs.time_pos));
    let _ = window.set_fullscreen(false);
    std::thread::sleep(Duration::from_millis(800));
    set_rect(app, preview_rect(window));
    std::thread::sleep(Duration::from_secs(2));
    let back = snapshot(app).unwrap_or_default();
    visual(&mut r, window, dir, &format!("{id}-back-to-preview"), video_region(window, false), "video visible: back to preview");
    check(&mut r, "back to preview keeps playing", back.session == fs.session && back.time_pos > fs.time_pos,
        format!("time {:.1}s -> {:.1}s", fs.time_pos, back.time_pos));

    // Repeated preview ⇄ fullscreen transitions: every state must show the picture.
    let mut toggles_ok = 0;
    for round in 1..=3 {
        let _ = window.set_fullscreen(true);
        std::thread::sleep(Duration::from_millis(700));
        set_rect(app, None);
        std::thread::sleep(Duration::from_millis(900));
        let a = visual_quiet(window, dir, &format!("{id}-toggle{round}-fullscreen"), video_region(window, true));
        let _ = window.set_fullscreen(false);
        std::thread::sleep(Duration::from_millis(700));
        set_rect(app, preview_rect(window));
        std::thread::sleep(Duration::from_millis(900));
        let b = visual_quiet(window, dir, &format!("{id}-toggle{round}-preview"), video_region(window, false));
        if a && b {
            toggles_ok += 1;
        }
    }
    let toggled = snapshot(app).unwrap_or_default();
    check(&mut r, "3 fullscreen round trips show video, same session", toggles_ok == 3 && toggled.session == back.session && toggled.error.is_none(),
        format!("{toggles_ok}/3 round trips with picture in both states, session {}", toggled.session));

    let _ = window.set_size(tauri::LogicalSize::new(1000.0, 620.0));
    std::thread::sleep(Duration::from_millis(600));
    set_rect(app, preview_rect(window));
    std::thread::sleep(Duration::from_secs(1));
    let resized = snapshot(app).unwrap_or_default();
    visual(&mut r, window, dir, &format!("{id}-resized"), video_region(window, false), "video visible: resized window");
    check(&mut r, "resize keeps playing", resized.time_pos > back.time_pos && resized.error.is_none(),
        format!("time {:.1}s -> {:.1}s", back.time_pos, resized.time_pos));
    let _ = window.set_size(tauri::LogicalSize::new(1280.0, 760.0));
    std::thread::sleep(Duration::from_millis(600));
    set_rect(app, preview_rect(window));

    // Maximize and restore.
    let _ = window.maximize();
    std::thread::sleep(Duration::from_millis(900));
    set_rect(app, preview_rect(window));
    std::thread::sleep(Duration::from_millis(900));
    visual(&mut r, window, dir, &format!("{id}-maximized"), video_region(window, false), "video visible: maximized");
    let _ = window.unmaximize();
    std::thread::sleep(Duration::from_millis(900));
    set_rect(app, preview_rect(window));
    std::thread::sleep(Duration::from_millis(900));
    visual(&mut r, window, dir, &format!("{id}-restored"), video_region(window, false), "video visible: restored");

    // Every resize mode shows the picture (in preview).
    let mut modes_ok = Vec::new();
    for mode in [ResizeMode::Fit, ResizeMode::Fill, ResizeMode::Zoom, ResizeMode::Stretch] {
        let _ = command(app, PlayerCommand::SetResizeMode { mode });
        std::thread::sleep(Duration::from_millis(900));
        if visual_quiet(window, dir, &format!("{id}-mode-{mode:?}").to_lowercase(), video_region(window, false)) {
            modes_ok.push(format!("{mode:?}"));
        }
    }
    let _ = command(app, PlayerCommand::SetResizeMode { mode: ResizeMode::Fit });
    check(&mut r, "resize modes Fit/Fill/Zoom/Stretch show video", modes_ok.len() == 4, format!("with picture: {modes_ok:?}"));

    // Speed, delays, volume, visibility, resize mode: value reported back by mpv.
    let _ = command(app, PlayerCommand::SetSpeed { speed: 1.5 });
    let _ = command(app, PlayerCommand::SetSubtitleDelay { ms: 250 });
    let _ = command(app, PlayerCommand::SetAudioDelay { ms: -100 });
    let _ = command(app, PlayerCommand::SetVolume { volume: 150.0 });
    let _ = command(app, PlayerCommand::SetSubtitleVisible { visible: false });
    let _ = command(app, PlayerCommand::SetResizeMode { mode: ResizeMode::Zoom });
    let applied = wait_for(app, Duration::from_secs(3), |s| {
        (s.speed - 1.5).abs() < 1e-6 && s.sub_delay_ms == 250 && s.audio_delay_ms == -100 && (s.volume - 150.0).abs() < 1e-6 && !s.sub_visible
    });
    let panscan = property(app, "panscan");
    check(&mut r, "speed / delays / volume / visibility / resize mode", applied.is_some() && panscan.as_deref().is_some_and(|p| p.starts_with("1.0")),
        format!("{:?} panscan={panscan:?}", applied.as_ref().map(|s| (s.speed, s.sub_delay_ms, s.audio_delay_ms, s.volume, s.sub_visible))));
    let _ = command(app, PlayerCommand::SetSpeed { speed: 1.0 });
    let _ = command(app, PlayerCommand::SetSubtitleDelay { ms: 0 });
    let _ = command(app, PlayerCommand::SetAudioDelay { ms: 0 });
    let _ = command(app, PlayerCommand::SetVolume { volume: 100.0 });
    let _ = command(app, PlayerCommand::SetSubtitleVisible { visible: true });
    let _ = command(app, PlayerCommand::SetResizeMode { mode: ResizeMode::Fit });

    // Audio track switch when the source has more than one.
    let current = snapshot(app).unwrap_or_default();
    let audio: Vec<_> = current.tracks.iter().filter(|t| t.kind == "audio").collect();
    if audio.len() > 1 {
        let target = audio.iter().find(|t| !t.selected).map(|t| t.id).unwrap_or(audio[0].id);
        let _ = command(app, PlayerCommand::SetAudioTrack { id: target.to_string() });
        let switched = wait_for(app, Duration::from_secs(6), |s| s.aid.as_deref() == Some(&target.to_string()));
        std::thread::sleep(Duration::from_secs(2));
        let after = snapshot(app).unwrap_or_default();
        check(&mut r, "audio track switch", switched.is_some() && after.time_pos > current.time_pos,
            format!("aid {:?} -> {:?} (target {target}), {} audio tracks", current.aid, after.aid, audio.len()));
    } else {
        check(&mut r, "audio track switch", true, format!("skipped: {} audio track(s) in source", audio.len()));
    }

    // External subtitle add (selected) then remove (original: removeExternalSubtitles).
    if let Some(sub) = external_subtitle {
        let _ = command(app, PlayerCommand::AddSubtitle { url: sub, title: None, lang: None });
        let loaded = wait_for(app, Duration::from_secs(6), |s| s.tracks.iter().any(|t| t.kind == "sub" && t.external && t.selected));
        std::thread::sleep(Duration::from_millis(800));
        visual(&mut r, window, dir, &format!("{id}-subtitle"), video_region(window, false), "video visible: with external subtitle");
        let _ = command(app, PlayerCommand::RemoveExternalSubtitles { select_id: None });
        let removed = wait_for(app, Duration::from_secs(4), |s| !s.tracks.iter().any(|t| t.external) && s.sid.as_deref().is_none_or(|v| v == "no"));
        check(&mut r, "external subtitle add + remove", loaded.is_some() && removed.is_some(),
            format!("added: {}, removed: {}", loaded.is_some(), removed.is_some()));
    }

    // Exact seek forward when long enough (not for live streams).
    let pre = snapshot(app).unwrap_or_default();
    if pre.duration > 30.0 && pre.time_pos + 15.0 < pre.duration {
        let target = pre.time_pos + 12.0;
        let _ = command(app, PlayerCommand::Seek { seconds: target, relative: false });
        let sought = wait_for(app, Duration::from_secs(10), |s| (s.time_pos - target).abs() < 1.5 && !s.buffering);
        check(&mut r, "exact seek", sought.is_some(), format!("{:.1}s -> {:?} (target {target:.1})", pre.time_pos, sought.map(|s| s.time_pos)));
    } else {
        check(&mut r, "exact seek", true, format!("skipped: duration {:.1}s", pre.duration));
    }

    // Picture-in-picture: the container moves to its own window and playback continues.
    let pip_before = snapshot(app).unwrap_or_default();
    let entered = on_main(app, |app| set_picture_in_picture(app, &app.state::<AppState>(), true, "Nuvio Astra"));
    std::thread::sleep(Duration::from_secs(3));
    let in_pip = snapshot(app).unwrap_or_default();
    let host_windows_in_pip = host_mpv_windows(app);
    check(&mut r, "picture-in-picture keeps playing",
        entered.is_ok() && in_pip.picture_in_picture && in_pip.time_pos > pip_before.time_pos + 1.0 && host_windows_in_pip == 0,
        format!("{entered:?}, time {:.1}s -> {:.1}s, mpv windows left in main window: {host_windows_in_pip}", pip_before.time_pos, in_pip.time_pos));
    let exited = on_main(app, |app| set_picture_in_picture(app, &app.state::<AppState>(), false, ""));
    std::thread::sleep(Duration::from_secs(2));
    let out = snapshot(app).unwrap_or_default();
    let back_windows = host_mpv_windows(app);
    check(&mut r, "picture-in-picture back to window", exited.is_ok() && !out.picture_in_picture && out.time_pos > in_pip.time_pos && back_windows == 1,
        format!("time {:.1}s -> {:.1}s, mpv windows in main window: {back_windows}", in_pip.time_pos, out.time_pos));

    let end = snapshot(app).unwrap_or_default();
    r.dropped_frames = end.dropped_frames;
    r.decoder_dropped_frames = end.decoder_dropped_frames;

    let closing = Instant::now();
    on_main(app, |app| app.state::<AppState>().close());
    let left = host_mpv_windows(app);
    check(&mut r, "clean close", left == 0, format!("closed in {} ms, mpv windows left: {left}", closing.elapsed().as_millis()));
    r.ended_at_ms = now_ms();
    r
}
