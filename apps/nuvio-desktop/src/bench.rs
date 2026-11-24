//! Real-use benchmark (`--bench-addon <addon name> <report.json> [cycles]`).
//!
//! Reproduces what the user does in the original app, without test-only actions: idle after
//! start-up, then `cycles` times: search the streams of the same live channel (first item of the
//! addon's first listable catalog), play it fullscreen (window fullscreen, video over the whole
//! window, controls overlay visible), keep playing, close, stay idle. The report holds phase
//! timestamps for the external sampler (`bench/sample-run.ps1`) and what mpv reported.
//! No URLs or header values are written.

use crate::selftest::{on_main, snapshot, wait_for};
use crate::AppState;
use serde::Serialize;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// Start-up settle before the idle window, then idle length (same as the original's guide).
pub const IDLE_SETTLE: Duration = Duration::from_secs(15);
pub const IDLE_WINDOW: Duration = Duration::from_secs(45);
/// After the first frame: stabilization, then the measured playback window.
pub const PLAY_SETTLE: Duration = Duration::from_secs(15);
pub const PLAY_WINDOW: Duration = Duration::from_secs(60);
/// After closing: release time, then the measured retained-memory window.
pub const CLOSE_SETTLE: Duration = Duration::from_secs(15);
pub const CLOSE_WINDOW: Duration = Duration::from_secs(30);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Phase {
    pub name: String,
    pub start_ms: u128,
    pub end_ms: u128,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Cycle {
    pub cycle: usize,
    /// From the start of the stream search (user's click) to the first decoded frame.
    pub ttff_ms: Option<u128>,
    pub hwdec: Option<String>,
    pub resolution: String,
    pub fps: f64,
    pub dropped_frames: i64,
    pub decoder_dropped_frames: i64,
    pub max_abs_av_sync_ms: f64,
    pub error: Option<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BenchReport {
    pub app: String,
    pub content: String,
    pub phases: Vec<Phase>,
    pub cycles: Vec<Cycle>,
    pub error: Option<String>,
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn phase(report: &mut BenchReport, name: String, length: Duration) {
    let start = now_ms();
    std::thread::sleep(length);
    report.phases.push(Phase { name, start_ms: start, end_ms: now_ms() });
}

pub fn start(app: AppHandle, addon: String, report_path: PathBuf, cycles: usize) {
    std::thread::spawn(move || {
        let report = run(&app, &addon, cycles);
        let _ = std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap_or_default());
        app.exit(0);
    });
}

fn run(app: &AppHandle, addon: &str, cycles: usize) -> BenchReport {
    let mut report = BenchReport { app: "nuvio-nextgen".into(), ..Default::default() };
    let window = app.get_webview_window("main").expect("main window");
    std::thread::sleep(IDLE_SETTLE);
    phase(&mut report, "idle".into(), IDLE_WINDOW);

    // The channel: first item of the addon's first listable catalog (as picked earlier).
    let client = app.state::<AppState>().client.clone();
    let wanted = addon.to_lowercase();
    let picked = tauri::async_runtime::block_on(async move {
        let manifests = crate::streams::load_manifests(&client, None).await?;
        let manifest = manifests.into_iter().filter_map(Result::ok).find(|m| m.name.to_lowercase().contains(&wanted)).ok_or("addon not enabled")?;
        let catalog = manifest
            .catalogs
            .iter()
            .find(|c| !c.extra.iter().any(|e| e.is_required && !e.name.eq_ignore_ascii_case("skip")))
            .ok_or("no listable catalog")?
            .clone();
        let page = client.fetch_catalog_page(&manifest, &catalog.content_type, &catalog.id, None, None).await.map_err(|e| e.to_string())?;
        let item = page.items.into_iter().next().ok_or("empty catalog")?;
        Ok::<_, String>((item.content_type, item.id, item.name))
    });
    let (content_type, id, title) = match picked {
        Ok(v) => v,
        Err(e) => {
            report.error = Some(e);
            return report;
        }
    };
    report.content = format!("{addon} — \"{title}\" [{content_type}]");

    // Fullscreen player, controls overlay visible, video over the whole window.
    let _ = app.emit("selftest", true);
    let _ = app.emit("selftest-source", report.content.clone());
    let _ = window.set_fullscreen(true);
    let _ = app.emit("selftest-rect", Option::<[f64; 4]>::None);
    std::thread::sleep(Duration::from_secs(1));

    for cycle in 1..=cycles {
        let mut c = Cycle { cycle, ..Default::default() };
        let clicked = Instant::now();
        let handle = app.clone();
        let (ct, item) = (content_type.clone(), id.clone());
        let opened = tauri::async_runtime::block_on(async move {
            let state = handle.state::<AppState>();
            let groups = crate::streams::search(&state.client, &state.streams, &ct, &item, None).await?;
            groups
                .into_iter()
                .flat_map(|g| g.streams)
                .find(|s| s.kind == crate::streams::StreamKind::Direct)
                .map(|s| s.key)
                .ok_or_else(|| "no direct stream".to_string())
        })
        .and_then(|key| {
            on_main(app, move |app| {
                let state = app.state::<AppState>();
                let stream = state.streams.get(&key).ok_or("stream vanished")?;
                let url = stream.playable_direct_url().ok_or("no url")?.to_string();
                state.open(app, url, stream.playback_request_headers(), 0.0)
            })
        });
        if let Err(e) = opened {
            c.error = Some(e);
            report.cycles.push(c);
            continue;
        }
        let _ = app.state::<AppState>().with_player(|p| p.set_video_rect(None, Vec::new()).map_err(|e| e.to_string()));
        let first = wait_for(app, Duration::from_secs(45), |s| s.loaded && s.time_pos > 0.0 && s.width > 0);
        c.ttff_ms = first.as_ref().map(|_| clicked.elapsed().as_millis());
        if first.is_none() {
            c.error = snapshot(app).and_then(|s| s.error).or(Some("no frame within 45 s".into()));
            on_main(app, |app| app.state::<AppState>().close());
            report.cycles.push(c);
            continue;
        }
        std::thread::sleep(PLAY_SETTLE);
        // Measured playback window, sampling A/V sync meanwhile.
        let start = now_ms();
        let until = Instant::now() + PLAY_WINDOW;
        let mut max_av = 0.0f64;
        while Instant::now() < until {
            if let Some(s) = snapshot(app) {
                max_av = max_av.max(s.av_sync.abs());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        report.phases.push(Phase { name: format!("playback-{cycle}"), start_ms: start, end_ms: now_ms() });
        let s = snapshot(app).unwrap_or_default();
        c.hwdec = s.hwdec_current;
        c.resolution = format!("{}x{}", s.width, s.height);
        c.fps = s.fps;
        c.dropped_frames = s.dropped_frames;
        c.decoder_dropped_frames = s.decoder_dropped_frames;
        c.max_abs_av_sync_ms = (max_av * 10_000.0).round() / 10.0;
        on_main(app, |app| app.state::<AppState>().close());
        std::thread::sleep(CLOSE_SETTLE);
        phase(&mut report, format!("after-close-{cycle}"), CLOSE_WINDOW);
        report.cycles.push(c);
    }
    let _ = window.set_fullscreen(false);
    report
}
