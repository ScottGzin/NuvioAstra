//! Torrent sources from addons, played in the app's own player (as the original's desktop app):
//! `torrent_prepare` starts the original's Nuvio Engine for the stream (metadata, file selection,
//! local HTTP route), then `stream_play` opens that local URL in libmpv like any direct stream.
//! The P2P session belongs to the player session that opened it: closing that player (or the app)
//! stops the stream, removes the torrent and destroys the engine. While a session is active,
//! `p2p-status` events carry peers, speeds, downloaded bytes and progress (no addresses).
//!
//! P2P is off until the user accepts the original's P2P notice (per profile, `settings.p2p`).

use crate::AppState;
use nuvio_core::stream::StreamItem;
use nuvio_p2p::{EngineConfig, P2pStatus, PreparedStream, StreamRequest};
use nuvio_store::settings::P2pSettings;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

/// Error code the UI answers with the P2P notice (consent), then retries.
pub const NEEDS_CONSENT: &str = "P2P_CONSENT_REQUIRED";

pub fn engine_dll() -> Result<PathBuf, String> {
    nuvio_p2p::locate_engine().ok_or_else(|| {
        #[cfg(windows)]
        { "O motor P2P (nuvio_engine.dll) não está instalado com esta versão.".to_string() }
        #[cfg(not(windows))]
        { "O P2P ainda não está disponível no Linux (o motor libtorrent será portado na fase 3).".to_string() }
    })
}

/// Engine folders: state with the app data, the (large) piece cache in the local, non-roaming
/// profile folder.
pub fn engine_config(state: &AppState, settings: &P2pSettings) -> EngineConfig {
    let local = if crate::uitest::requested().is_some() {
        // UI tests never touch the user's own cache.
        std::env::temp_dir().join("nuvio-nextgen-uitest-p2p")
    } else {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| state.app_store.dir().to_path_buf())
    };
    EngineConfig {
        data_dir: state.app_store.dir().join("p2p"),
        cache_dir: local.join("NuvioNextGen").join("p2p-cache"),
        disk_cache_bytes: settings.cache_bytes(),
        upload: settings.enable_upload,
        profile: settings.profile_value(),
    }
}

/// What the original passes to the engine for an addon stream: info hash (or the magnet's),
/// `fileIdx`, `behaviorHints.filename`, and the stream's trackers (`tracker:` sources, magnet `tr`).
pub fn request_for(stream: &StreamItem) -> Result<StreamRequest, String> {
    let magnet = [stream.url.as_deref(), stream.external_url.as_deref()]
        .into_iter()
        .flatten()
        .find(|u| u.trim_start().to_ascii_lowercase().starts_with("magnet:"));
    let info_hash = stream
        .info_hash
        .clone()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| magnet.and_then(nuvio_p2p::info_hash_from_magnet))
        .ok_or("Esta fonte torrent não informa o hash do torrent.")?;
    let mut trackers: Vec<String> = stream
        .sources
        .iter()
        .filter_map(|s| {
            let s = s.trim();
            match s.strip_prefix("tracker:") {
                Some(t) => Some(t.trim().to_string()),
                None if s.contains("://") => Some(s.to_string()),
                None => None,
            }
        })
        .collect();
    if let Some(m) = magnet {
        trackers.extend(nuvio_p2p::trackers_from_magnet(m));
    }
    Ok(StreamRequest {
        info_hash: nuvio_p2p::canonical_info_hash(&info_hash)?,
        file_index: stream.file_idx.and_then(|i| i32::try_from(i).ok()),
        filename: stream.behavior_hints.filename.clone(),
        trackers,
    })
}

/// Starts the P2P session of a torrent stream from the last search; returns the selected file.
#[tauri::command]
pub async fn torrent_prepare(app: AppHandle, state: State<'_, AppState>, key: String) -> Result<PreparedStream, String> {
    let stream = state.streams.get(&key).ok_or("unknown stream (search again)")?;
    let settings = state.app_store.profile_settings(state.active_profile()).map_err(|e| e.to_string())?.p2p;
    if !settings.enabled {
        return Err(NEEDS_CONSENT.into());
    }
    let request = request_for(&stream)?;
    if state.downloads.p2p_owner.lock().expect("p2p owner").is_some() {
        return Err("Um download por torrent está usando o P2P. Pause-o em Biblioteca → Downloads para assistir outro torrent.".into());
    }
    let config = engine_config(&state, &settings);
    let dll = engine_dll()?;
    // A new session replaces the previous one (its player keeps nothing playable).
    state.p2p_session.store(0, Ordering::SeqCst);
    *state.p2p_prepared.lock().expect("p2p prepared") = None;
    let service = state.p2p.clone();
    start_status_events(app.clone());
    let prepared = tauri::async_runtime::spawn_blocking(move || service.start(&dll, &config, &request))
        .await
        .map_err(|e| e.to_string())??;
    *state.p2p_prepared.lock().expect("p2p prepared") = Some((key, prepared.clone()));
    Ok(prepared)
}

/// Cancels a start in progress or ends the session (the player's close does it too).
#[tauri::command]
pub async fn torrent_stop(state: State<'_, AppState>) -> Result<(), String> {
    *state.p2p_prepared.lock().expect("p2p prepared") = None;
    state.p2p_session.store(0, Ordering::SeqCst);
    let service = state.p2p.clone();
    tauri::async_runtime::spawn_blocking(move || service.stop()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn p2p_status(state: State<'_, AppState>) -> P2pStatus {
    state.p2p.status()
}

/// "Limpar cache" (not during a P2P playback). Returns the bytes freed.
#[tauri::command]
pub async fn p2p_clear_cache(state: State<'_, AppState>) -> Result<u64, String> {
    let settings = state.app_store.profile_settings(state.active_profile()).map_err(|e| e.to_string())?.p2p;
    let config = engine_config(&state, &settings);
    let dll = engine_dll()?;
    let service = state.p2p.clone();
    tauri::async_runtime::spawn_blocking(move || service.clear_cache(&dll, &config)).await.map_err(|e| e.to_string())?
}

/// Size of the torrent cache on disk.
#[tauri::command]
pub fn p2p_cache_size(state: State<'_, AppState>) -> u64 {
    let settings = state.app_store.profile_settings(state.active_profile()).map(|s| s.p2p).unwrap_or_default();
    nuvio_p2p::dir_size(&engine_config(&state, &settings).cache_dir)
}

/// Local URL of the prepared torrent stream for `stream_play`.
pub fn prepared_url(state: &AppState, key: &str) -> Result<String, String> {
    match state.p2p_prepared.lock().expect("p2p prepared").as_ref() {
        Some((k, p)) if k == key && state.p2p.is_active() => Ok(p.url.clone()),
        _ => Err("A fonte torrent não está pronta (inicie novamente).".into()),
    }
}

/// One `p2p-status` per second while a session exists (the thread ends with it).
fn start_status_events(app: AppHandle) {
    std::thread::Builder::new()
        .name("nuvio-p2p-status".into())
        .spawn(move || {
            let state = app.state::<AppState>();
            std::thread::sleep(std::time::Duration::from_millis(300));
            loop {
                let status = state.p2p.refresh();
                let _ = app.emit("p2p-status", &status);
                if status.state == "idle" || status.state == "error" {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        })
        .ok();
}

/// Called when a player session ends: its P2P session ends too (off the calling thread).
pub fn on_player_closed(state: &AppState, session: u64) {
    if session != 0 && state.p2p_session.compare_exchange(session, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
        *state.p2p_prepared.lock().expect("p2p prepared") = None;
        // A download of this torrent goes on; it stops the engine when it ends.
        if state.downloads.p2p_owner.lock().expect("p2p owner").is_some() {
            return;
        }
        let service = state.p2p.clone();
        std::thread::spawn(move || service.stop());
    }
}

/// The engine's local route for a download: the torrent playing now is shared, otherwise the engine
/// starts for the download alone (one torrent download at a time, none while another plays).
pub async fn prepare_for_download(state: &AppState, id: &str, request: StreamRequest) -> Result<PreparedStream, String> {
    {
        let mut owner = state.downloads.p2p_owner.lock().expect("p2p owner");
        if owner.as_deref().is_some_and(|o| o != id) {
            return Err("Outro download por torrent está em andamento; este continua quando ele terminar ou for pausado.".into());
        }
        *owner = Some(id.to_string());
    }
    let playing = state.p2p_prepared.lock().expect("p2p prepared").clone();
    if let Some((key, prepared)) = playing.filter(|_| state.p2p.is_active()) {
        let same = state.streams.get(&key).and_then(|s| request_for(&s).ok()).is_some_and(|r| r.info_hash == request.info_hash);
        if same {
            return Ok(prepared);
        }
        release_download(state, id);
        return Err("Feche o torrent que está tocando para baixar outro por P2P.".into());
    }
    let settings = state.app_store.profile_settings(state.active_profile()).map_err(|e| e.to_string())?.p2p;
    let config = engine_config(state, &settings);
    let dll = engine_dll()?;
    let service = state.p2p.clone();
    let started = tauri::async_runtime::spawn_blocking(move || service.start(&dll, &config, &request)).await.map_err(|e| e.to_string())?;
    if started.is_err() {
        release_download(state, id);
    }
    started
}

/// The download ended (done, paused, failed): the engine stops unless a player uses it.
pub fn release_download(state: &AppState, id: &str) {
    let mut owner = state.downloads.p2p_owner.lock().expect("p2p owner");
    if owner.as_deref() != Some(id) {
        return;
    }
    *owner = None;
    if state.p2p_session.load(Ordering::SeqCst) == 0 {
        *state.p2p_prepared.lock().expect("p2p prepared") = None;
        let service = state.p2p.clone();
        std::thread::spawn(move || service.stop());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_core::stream::parse_streams;

    #[test]
    fn request_from_addon_streams() {
        let payload = r#"{"streams":[
            {"name":"T","infoHash":"DD8255ECDC7CA55FB0BBF81323D87062DB1F6D1C","fileIdx":1,"sources":["tracker:udp://a:1/announce","dht:dd82"],"behaviorHints":{"filename":"Big Buck Bunny.mp4"}},
            {"name":"M","url":"magnet:?xt=urn:btih:dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c&tr=udp%3A%2F%2Fb%3A2"}
        ]}"#;
        let s = parse_streams(payload, "A", "addon:a", None).unwrap();
        let r = request_for(&s[0]).unwrap();
        assert_eq!(r.info_hash, "dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c");
        assert_eq!((r.file_index, r.filename.as_deref()), (Some(1), Some("Big Buck Bunny.mp4")));
        assert_eq!(r.trackers, vec!["udp://a:1/announce"]);
        let m = request_for(&s[1]).unwrap();
        assert_eq!((m.info_hash.as_str(), m.file_index), ("dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c", None));
        assert_eq!(m.trackers, vec!["udp://b:2"]);
    }
}
