//! TorBox for the active profile: its key (encrypted with DPAPI, never sent to the UI), which
//! torrent sources are cached there ("TB Instant") and the HTTP link a cached source plays from.
//!
//! As in the original, torrents of the installed addons are checked by info hash, and the
//! addons' own TorBox streams (`clientResolve`) play with the same key. A source that is not cached
//! keeps playing through P2P.

use crate::streams::{kind_of, StreamKind};
use crate::AppState;
use nuvio_core::stream::StreamItem;
use nuvio_core::torbox::{self, Resolved, TorboxError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::State;

/// A link stays valid at TorBox for hours; this keeps it for a replay or a download.
const RESOLVED_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Serialize, Deserialize)]
struct Saved {
    key: String,
    account: String,
}

#[derive(Default)]
pub struct Debrid {
    /// Stream key → link of the cached file.
    resolved: Mutex<HashMap<String, (Instant, Resolved)>>,
    /// Info hash → cached at TorBox, per key fingerprint (another key asks again).
    cached: Mutex<HashMap<(String, String), bool>>,
}

impl Debrid {
    pub fn link(&self, key: &str) -> Option<Resolved> {
        let resolved = self.resolved.lock().expect("torbox links");
        resolved.get(key).filter(|(at, _)| at.elapsed() < RESOLVED_TTL).map(|(_, r)| r.clone())
    }

    fn forget(&self) {
        self.resolved.lock().expect("torbox links").clear();
        self.cached.lock().expect("torbox cache").clear();
    }
}

fn file(state: &AppState, profile: i64) -> std::path::PathBuf {
    state.app_store.dir().join(format!("torbox-{profile}.dat"))
}

fn saved(state: &AppState, profile: i64) -> Option<Saved> {
    let cipher = std::fs::read(file(state, profile)).ok()?;
    serde_json::from_slice(&nuvio_sync::crypto::unprotect(&cipher).ok()?).ok()
}

/// The active profile's key when TorBox is on.
pub fn active_key(state: &AppState) -> Option<String> {
    let profile = state.active_profile();
    let enabled = state.app_store.profile_settings(profile).map(|s| s.debrid.enabled).unwrap_or(true);
    saved(state, profile).filter(|_| enabled).map(|s| s.key)
}

fn fingerprint(key: &str) -> String {
    nuvio_sync::crypto::sha256_hex(key)[..12].to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorboxStatus {
    pub configured: bool,
    pub enabled: bool,
    /// E-mail of the TorBox account, as TorBox answered when the key was saved.
    pub account: Option<String>,
}

fn status(state: &AppState) -> TorboxStatus {
    let profile = state.active_profile();
    let saved = saved(state, profile);
    TorboxStatus {
        configured: saved.is_some(),
        enabled: state.app_store.profile_settings(profile).map(|s| s.debrid.enabled).unwrap_or(true),
        account: saved.map(|s| s.account),
    }
}

#[tauri::command]
pub fn torbox_status(state: State<'_, AppState>) -> Result<TorboxStatus, String> {
    state.ensure_unlocked()?;
    Ok(status(&state))
}

/// Validates the key with TorBox, then keeps it encrypted for the active profile.
#[tauri::command]
pub async fn torbox_save_key(state: State<'_, AppState>, key: String) -> Result<TorboxStatus, String> {
    state.ensure_unlocked()?;
    let key = key.trim().to_string();
    if key.len() < 8 || key.chars().any(char::is_whitespace) {
        return Err("Cole a chave de API completa do TorBox.".into());
    }
    let account = state.torbox.account(&key).await.map_err(|e| e.to_string())?;
    let plain = serde_json::to_vec(&Saved { key, account }).map_err(|e| e.to_string())?;
    let cipher = nuvio_sync::crypto::protect(&plain)?;
    let path = file(&state, state.active_profile());
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, cipher).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())?;
    state.debrid.forget();
    Ok(status(&state))
}

#[tauri::command]
pub fn torbox_remove_key(state: State<'_, AppState>) -> Result<TorboxStatus, String> {
    state.ensure_unlocked()?;
    let _ = std::fs::remove_file(file(&state, state.active_profile()));
    state.debrid.forget();
    Ok(status(&state))
}

/// How a source can use TorBox.
fn eligible(stream: &StreamItem) -> Option<String> {
    match kind_of(stream) {
        StreamKind::Torrent => torbox::info_hash(stream),
        StreamKind::Debrid if torbox::is_torbox_client_stream(stream) => Some(String::new()),
        _ => None,
    }
}

/// Which of the given sources (keys of the last searches) TorBox has ready: `cached`, `notCached`.
/// The addon's own TorBox sources are ready by definition. Without a key: nothing.
#[tauri::command]
pub async fn torbox_check(state: State<'_, AppState>, keys: Vec<String>) -> Result<HashMap<String, String>, String> {
    state.ensure_unlocked()?;
    let Some(api_key) = active_key(&state) else { return Ok(HashMap::new()) };
    let print = fingerprint(&api_key);
    let mut out = HashMap::new();
    let mut by_hash: HashMap<String, Vec<String>> = HashMap::new();
    for key in keys {
        let Some(stream) = state.streams.get(&key) else { continue };
        match eligible(&stream) {
            Some(hash) if hash.is_empty() => {
                out.insert(key, "cached".to_string());
            }
            Some(hash) => by_hash.entry(hash).or_default().push(key),
            None => {}
        }
    }
    let unknown: Vec<String> = {
        let known = state.debrid.cached.lock().expect("torbox cache");
        by_hash.keys().filter(|h| !known.contains_key(&(print.clone(), (*h).clone()))).cloned().collect()
    };
    if !unknown.is_empty() {
        let found = state.torbox.check_cached(&api_key, &unknown).await.map_err(|e| e.to_string())?;
        let mut known = state.debrid.cached.lock().expect("torbox cache");
        for hash in unknown {
            let cached = found.contains_key(&hash);
            known.insert((print.clone(), hash), cached);
        }
    }
    let known = state.debrid.cached.lock().expect("torbox cache");
    for (hash, keys) in by_hash {
        let cached = known.get(&(print.clone(), hash)).copied().unwrap_or(false);
        for key in keys {
            out.insert(key, if cached { "cached" } else { "notCached" }.to_string());
        }
    }
    Ok(out)
}

/// Gets the TorBox link of a source before it plays or downloads. `season`/`episode` pick the file
/// of a season pack.
pub async fn prepare(state: &AppState, key: &str, season: Option<i64>, episode: Option<i64>) -> Result<Resolved, String> {
    if let Some(link) = state.debrid.link(key) {
        return Ok(link);
    }
    let api_key = active_key(state).ok_or("Configure o TorBox em Configurações → TorBox.")?;
    let stream = state.streams.get(key).ok_or("unknown stream (search again)")?;
    eligible(&stream).ok_or("Esta fonte não pode ser aberta pelo TorBox.")?;
    let wanted = torbox::wanted(&stream, season, episode).ok_or("Esta fonte não informa o torrent.")?;
    let resolved = state.torbox.resolve(&api_key, &wanted).await.map_err(|e| {
        if e == TorboxError::NotCached {
            if let Some(hash) = torbox::info_hash(&stream) {
                state.debrid.cached.lock().expect("torbox cache").insert((fingerprint(&api_key), hash), false);
            }
        }
        e.to_string()
    })?;
    state.debrid.resolved.lock().expect("torbox links").insert(key.to_string(), (Instant::now(), resolved.clone()));
    Ok(resolved)
}

/// TorBox has this source ready (already linked, checked as cached, or the addon's own TorBox source).
pub fn ready(state: &AppState, key: &str) -> bool {
    let Some(api_key) = active_key(state) else { return false };
    if state.debrid.link(key).is_some() {
        return true;
    }
    let Some(stream) = state.streams.get(key) else { return false };
    if torbox::is_torbox_client_stream(&stream) {
        return true;
    }
    let Some(hash) = torbox::info_hash(&stream) else { return false };
    state.debrid.cached.lock().expect("torbox cache").get(&(fingerprint(&api_key), hash)).copied().unwrap_or(false)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub filename: Option<String>,
    pub size: Option<u64>,
}

#[tauri::command]
pub async fn torbox_prepare(state: State<'_, AppState>, key: String, season: Option<i64>, episode: Option<i64>) -> Result<Prepared, String> {
    state.ensure_unlocked()?;
    let resolved = prepare(&state, &key, season, episode).await?;
    Ok(Prepared { filename: resolved.filename, size: resolved.size })
}
