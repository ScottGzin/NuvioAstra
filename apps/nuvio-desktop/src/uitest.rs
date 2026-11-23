//! Automated UI test (`--uitest <report.json>`): the real React UI drives itself with DOM clicks
//! against the real addons (`web/src/uitest/`), and uses these two commands to capture the
//! composed window and to hand in its report. Commands refuse to run outside this mode.

use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub fn requested() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--uitest")?;
    args.get(i + 1).map(PathBuf::from)
}

fn report_path() -> Result<PathBuf, String> {
    requested().ok_or_else(|| "not running in --uitest mode".to_string())
}

/// Captures the composed window next to the report (`<name>.bmp`) and returns its pixel stats.
#[tauri::command]
pub async fn uitest_capture(window: tauri::WebviewWindow, name: String) -> Result<crate::capture::RegionStats, String> {
    let dir = report_path()?.parent().map(PathBuf::from).unwrap_or_default();
    let safe: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    tauri::async_runtime::spawn_blocking(move || crate::capture::capture_and_measure(&window, &dir.join(format!("{safe}.png")), None))
        .await
        .map_err(|e| e.to_string())?
}

/// Writes the report and closes the app (the player is released by the window's Destroyed event).
#[tauri::command]
pub fn uitest_report(app: AppHandle, report: Value) -> Result<(), String> {
    let path = report_path()?;
    std::fs::write(&path, serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    app.state::<crate::AppState>().close();
    app.exit(0);
    Ok(())
}

/// Optional phase given after the report path (`--uitest <report> <phase>`), for checks that need
/// a restart (settings persistence).
#[tauri::command]
pub fn uitest_phase() -> Result<String, String> {
    report_path()?;
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--uitest").unwrap_or(0);
    Ok(args.get(i + 2).cloned().unwrap_or_default())
}

/// Reads an mpv property of the current session (checks that settings reach the player).
#[tauri::command]
pub fn uitest_player_property(app: AppHandle, name: String) -> Result<Option<String>, String> {
    report_path()?;
    app.state::<crate::AppState>().with_player(|p| Ok(p.property(&name)))
}

/// Data folder of a UI test run: a copy of the real NextGen data (`*.json`, not the backups), so the
/// test works with the user's real library, settings and history but never changes them. The copy
/// is refreshed at the start of a run and kept for `*-verify*` phases (checks after a restart).
pub fn data_dir(real: &std::path::Path) -> Option<PathBuf> {
    requested()?;
    let dir = std::env::temp_dir().join("nuvio-nextgen-uitest-data");
    let phase = uitest_phase().unwrap_or_default();
    if !phase.contains("verify") || !dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(entries) = std::fs::read_dir(real) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|e| e == "json") {
                    let _ = std::fs::copy(&path, dir.join(entry.file_name()));
                }
            }
        }
    }
    Some(dir)
}

/// Saved items / progress / watched counts per profile in the test data folder (profile separation).
#[tauri::command]
pub fn uitest_store_counts(app: AppHandle) -> Result<std::collections::BTreeMap<i64, [usize; 3]>, String> {
    report_path()?;
    let store = &app.state::<crate::AppState>().app_store;
    let mut out = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(store.dir()).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let profile = ["library-", "progress-", "watched-"]
            .iter()
            .find_map(|p| name.strip_prefix(p)?.strip_suffix(".json")?.parse::<i64>().ok());
        if let Some(p) = profile {
            let counts = [
                store.favorites(p).map_err(|e| e.to_string())?.len(),
                store.progress(p).map_err(|e| e.to_string())?.len(),
                store.watched(p).map_err(|e| e.to_string())?.len(),
            ];
            out.insert(p, counts);
        }
    }
    Ok(out)
}

/// Phase `p2p`: the sources of any title get one extra group with a freely licensed torrent
/// (Big Buck Bunny, © Blender Foundation, CC BY 3.0; WebTorrent's public test torrent), so the real
/// UI path (sources → P2P notice → engine → player) is tested without the user's addons' content.
pub fn p2p_test_streams() -> Option<(&'static str, &'static str)> {
    (uitest_phase().ok()? == "p2p").then_some((
        "Teste P2P (conteúdo livre)",
        r#"{"streams":[{"name":"Big Buck Bunny (CC BY)","title":"Blender Foundation · torrent de teste","infoHash":"dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c","sources":["tracker:udp://tracker.opentrackr.org:1337/announce","tracker:udp://tracker.leechers-paradise.org:6969"],"behaviorHints":{"filename":"Big Buck Bunny.mp4"}}]}"#,
    ))
}

/// Sockets and threads of this process (`netstat -ano`, thread count): checks that a P2P session
/// leaves no listening socket, UDP socket, peer connection or thread behind.
#[tauri::command]
pub fn uitest_process_resources() -> Result<Value, String> {
    report_path()?;
    let pid = std::process::id().to_string();
    let mut cmd = std::process::Command::new("netstat");
    cmd.arg("-ano");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (mut listening, mut established, mut other_tcp, mut udp) = (0, 0, 0, 0);
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.last() != Some(&pid.as_str()) {
            continue;
        }
        match (cols.first().copied(), cols.get(3).copied()) {
            (Some("UDP"), _) => udp += 1,
            (Some("TCP"), Some("LISTENING")) => listening += 1,
            (Some("TCP"), Some("ESTABLISHED")) => established += 1,
            (Some("TCP"), _) => other_tcp += 1,
            _ => {}
        }
    }
    let threads = thread_count();
    Ok(serde_json::json!({ "tcpListening": listening, "tcpEstablished": established, "tcpOther": other_tcp, "udp": udp, "threads": threads }))
}

#[cfg(not(windows))]
fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task").map(|d| d.count()).unwrap_or(0)
}

#[cfg(windows)]
fn thread_count() -> usize {
    {
        // Toolhelp snapshot of this process's threads.
        // Windows: Toolhelp snapshot of this process's threads.
        use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
        let pid = std::process::id();
        // SAFETY: plain Toolhelp calls with a sized entry; the handle is closed below.
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else { return 0 };
            let mut e = THREADENTRY32 { dwSize: std::mem::size_of::<THREADENTRY32>() as u32, ..Default::default() };
            let mut n = 0;
            let mut ok = Thread32First(snap, &mut e).is_ok();
            while ok {
                if e.th32OwnerProcessID == pid {
                    n += 1;
                }
                ok = Thread32Next(snap, &mut e).is_ok();
            }
            let _ = windows::Win32::Foundation::CloseHandle(snap);
            n
        }
    }
}

/// Phase `install-check`: where the app found its native libraries and whether they load —
/// libmpv (API version) and the P2P engine (version; one engine created in a test folder and
/// destroyed, no torrent added, no network use, the user's P2P cache untouched).
#[tauri::command]
pub fn uitest_native_libraries(app: AppHandle) -> Result<Value, String> {
    report_path()?;
    let state = app.state::<crate::AppState>();
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default();
    let mpv = state.api().map(|api| {
        let (major, minor) = api.client_api_version();
        format!("{major}.{minor}")
    });
    let engine_path = nuvio_p2p::locate_engine();
    let engine = engine_path.as_ref().ok_or_else(|| "nuvio_engine.dll not found".to_string()).and_then(|p| {
        let api = nuvio_p2p::load_api(p)?;
        let dir = std::env::temp_dir().join("nuvio-nextgen-uitest-install-check");
        let config = nuvio_p2p::EngineConfig { data_dir: dir.join("state"), cache_dir: dir.join("cache"), disk_cache_bytes: 0, upload: false, profile: 1 };
        let engine = nuvio_p2p::Engine::create(api, &config)?;
        let version = engine.version();
        drop(engine);
        Ok(version)
    });
    Ok(serde_json::json!({
        "exeDir": exe_dir,
        "engineDll": engine_path,
        "engineNextToExe": engine_path.as_ref().is_some_and(|p| p.parent() == Some(exe_dir.as_path())),
        "engine": engine,
        "libmpv": mpv,
    }))
}

/// Collections of the active profile in the test data folder (phase `collections`): the copy only,
/// never the real data nor the account.
#[tauri::command]
pub fn uitest_seed_collections(app: AppHandle, collections: Value) -> Result<(), String> {
    report_path()?;
    let state = app.state::<crate::AppState>();
    state.app_store.set_collections(state.active_profile(), &collections).map_err(|e| e.to_string())
}

/// Phase `enhance`: breaks display sync on purpose (see `Player::break_display_sync`).
#[tauri::command]
pub async fn uitest_break_display_sync(app: AppHandle) -> Result<(), String> {
    report_path()?;
    app.state::<crate::AppState>().with_player(|p| p.break_display_sync().map_err(|e| e.to_string()))
}
