//! Debrid services of the active profile: TorBox (`nuvio_core::torbox`) and Real-Debrid,
//! AllDebrid, Premiumize, Debrid-Link (`nuvio_core::debrid`). Each key is encrypted with DPAPI
//! and never sent to the UI.
//!
//! As in the original, torrents of the installed addons open as the service's HTTP link when a
//! connected service already has them, and otherwise keep playing through P2P:
//! - TorBox and Premiumize say beforehand which torrents they have ("TB Instant", "PM Instant").
//! - Real-Debrid, AllDebrid and Debrid-Link cannot: their torrents are marked "try" and asked when
//!   the source is opened (a torrent they do not have is removed from the account at once).
//! - Services are tried in the user's preferred order. The addons' own debrid sources
//!   (`clientResolve`) play through their own service.

use crate::streams::{kind_of, StreamKind};
use crate::AppState;
use nuvio_core::debrid::{client_stream_provider, DebridError, Provider};
use nuvio_core::stream::StreamItem;
use nuvio_core::torbox::{self, Resolved, TorboxError, Wanted};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::State;

/// A link stays valid at the services for hours; this keeps it for a replay or a download.
const RESOLVED_TTL: Duration = Duration::from_secs(15 * 60);

/// TorBox or one of the other services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    Torbox,
    Other(Provider),
}

impl Service {
    pub const ALL: [Service; 5] = [
        Service::Torbox,
        Service::Other(Provider::RealDebrid),
        Service::Other(Provider::AllDebrid),
        Service::Other(Provider::Premiumize),
        Service::Other(Provider::DebridLink),
    ];

    pub fn id(self) -> &'static str {
        match self {
            Service::Torbox => "torbox",
            Service::Other(p) => p.id(),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Service::Torbox => "TorBox",
            Service::Other(p) => p.label(),
        }
    }
    fn code(self) -> &'static str {
        match self {
            Service::Torbox => "TB",
            Service::Other(p) => p.code(),
        }
    }
    fn can_check(self) -> bool {
        match self {
            Service::Torbox => true,
            Service::Other(p) => p.can_check(),
        }
    }
    pub fn parse(id: &str) -> Option<Self> {
        if id.trim().eq_ignore_ascii_case("torbox") {
            return Some(Service::Torbox);
        }
        Provider::parse(id).map(Service::Other)
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    key: String,
    account: String,
}

#[derive(Default)]
pub struct Debrid {
    /// Stream key → link of the file and the service that gave it.
    resolved: Mutex<HashMap<String, (Instant, Resolved, Service)>>,
    /// (key fingerprint, info hash) → the service has it.
    cached: Mutex<HashMap<(String, String), bool>>,
}

impl Debrid {
    pub fn link(&self, key: &str) -> Option<Resolved> {
        self.entry(key).map(|(r, _)| r)
    }

    fn entry(&self, key: &str) -> Option<(Resolved, Service)> {
        let resolved = self.resolved.lock().expect("debrid links");
        resolved.get(key).filter(|(at, _, _)| at.elapsed() < RESOLVED_TTL).map(|(_, r, s)| (r.clone(), *s))
    }

    fn known(&self, print: &str, hash: &str) -> Option<bool> {
        self.cached.lock().expect("debrid cache").get(&(print.to_string(), hash.to_string())).copied()
    }

    fn remember(&self, print: &str, hash: &str, cached: bool) {
        self.cached.lock().expect("debrid cache").insert((print.to_string(), hash.to_string()), cached);
    }

    fn forget(&self) {
        self.resolved.lock().expect("debrid links").clear();
        self.cached.lock().expect("debrid cache").clear();
    }
}

// ---------------------------------------------------------------- keys

fn file(state: &AppState, service: Service, profile: i64) -> std::path::PathBuf {
    match service {
        // The TorBox file keeps its name: keys saved by earlier versions stay valid.
        Service::Torbox => state.app_store.dir().join(format!("torbox-{profile}.dat")),
        Service::Other(p) => state.app_store.dir().join(format!("debrid-{}-{profile}.dat", p.id())),
    }
}

fn saved(state: &AppState, service: Service, profile: i64) -> Option<Saved> {
    let cipher = std::fs::read(file(state, service, profile)).ok()?;
    serde_json::from_slice(&nuvio_sync::crypto::unprotect(&cipher).ok()?).ok()
}

fn enabled(state: &AppState, profile: i64) -> bool {
    state.app_store.profile_settings(profile).map(|s| s.debrid.enabled).unwrap_or(true)
}

fn order_file(state: &AppState, profile: i64) -> std::path::PathBuf {
    state.app_store.dir().join(format!("debrid-preferred-{profile}.json"))
}

fn preferred(state: &AppState, profile: i64) -> Option<Service> {
    let text = std::fs::read_to_string(order_file(state, profile)).ok()?;
    Service::parse(serde_json::from_str::<String>(&text).ok()?.as_str())
}

/// Connected services of the active profile, preferred first (empty when debrid is off).
fn connected(state: &AppState) -> Vec<(Service, String)> {
    let profile = state.active_profile();
    if !enabled(state, profile) {
        return Vec::new();
    }
    let first = preferred(state, profile);
    let order = first.into_iter().chain(Service::ALL.into_iter().filter(|s| Some(*s) != first));
    order.into_iter().filter_map(|s| saved(state, s, profile).map(|saved| (s, saved.key))).collect()
}

fn key_of(state: &AppState, service: Service) -> Option<String> {
    connected(state).into_iter().find(|(s, _)| *s == service).map(|(_, k)| k)
}

fn fingerprint(service: Service, key: &str) -> String {
    format!("{}:{}", service.id(), &nuvio_sync::crypto::sha256_hex(key)[..12])
}

async fn account(state: &AppState, service: Service, key: &str) -> Result<String, String> {
    match service {
        Service::Torbox => state.torbox.account(key).await.map_err(|e| e.to_string()),
        Service::Other(p) => state.debrid_client.account(p, key).await.map_err(|e| e.to_string()),
    }
}

fn store_key(state: &AppState, service: Service, key: String, account: String) -> Result<(), String> {
    let plain = serde_json::to_vec(&Saved { key, account }).map_err(|e| e.to_string())?;
    let cipher = nuvio_sync::crypto::protect(&plain)?;
    let path = file(state, service, state.active_profile());
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, cipher).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- settings page

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
    let saved = saved(state, Service::Torbox, profile);
    TorboxStatus { configured: saved.is_some(), enabled: enabled(state, profile), account: saved.map(|s| s.account) }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceView {
    pub id: &'static str,
    pub label: &'static str,
    pub code: &'static str,
    pub configured: bool,
    pub account: Option<String>,
    /// Can say beforehand which torrents it has.
    pub can_check: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebridStatus {
    pub enabled: bool,
    pub services: Vec<ServiceView>,
    /// The service tried first (the first connected one when none was chosen).
    pub preferred: Option<&'static str>,
}

fn debrid_status_of(state: &AppState) -> DebridStatus {
    let profile = state.active_profile();
    let services: Vec<ServiceView> = Service::ALL
        .into_iter()
        .map(|s| {
            let saved = saved(state, s, profile);
            ServiceView { id: s.id(), label: s.label(), code: s.code(), configured: saved.is_some(), account: saved.map(|x| x.account), can_check: s.can_check() }
        })
        .collect();
    let chosen = preferred(state, profile).filter(|s| saved(state, *s, profile).is_some());
    let first = services.iter().find(|v| v.configured).map(|v| v.id);
    DebridStatus { enabled: enabled(state, profile), preferred: chosen.map(Service::id).or(first), services }
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
    save_key(&state, Service::Torbox, key).await?;
    Ok(status(&state))
}

#[tauri::command]
pub fn torbox_remove_key(state: State<'_, AppState>) -> Result<TorboxStatus, String> {
    state.ensure_unlocked()?;
    let _ = std::fs::remove_file(file(&state, Service::Torbox, state.active_profile()));
    state.debrid.forget();
    Ok(status(&state))
}

async fn save_key(state: &AppState, service: Service, key: String) -> Result<(), String> {
    let key = key.trim().to_string();
    if key.len() < 8 || key.chars().any(char::is_whitespace) {
        return Err(format!("Cole a chave de API completa do {}.", service.label()));
    }
    let account = account(state, service, &key).await?;
    store_key(state, service, key, account)?;
    state.debrid.forget();
    Ok(())
}

#[tauri::command]
pub fn debrid_status(state: State<'_, AppState>) -> Result<DebridStatus, String> {
    state.ensure_unlocked()?;
    Ok(debrid_status_of(&state))
}

/// Validates the key with the service, then keeps it encrypted for the active profile.
#[tauri::command]
pub async fn debrid_save_key(state: State<'_, AppState>, service: String, key: String) -> Result<DebridStatus, String> {
    state.ensure_unlocked()?;
    let service = Service::parse(&service).ok_or("unknown service")?;
    save_key(&state, service, key).await?;
    Ok(debrid_status_of(&state))
}

#[tauri::command]
pub fn debrid_remove_key(state: State<'_, AppState>, service: String) -> Result<DebridStatus, String> {
    state.ensure_unlocked()?;
    let service = Service::parse(&service).ok_or("unknown service")?;
    let _ = std::fs::remove_file(file(&state, service, state.active_profile()));
    state.debrid.forget();
    Ok(debrid_status_of(&state))
}

/// The service tried first.
#[tauri::command]
pub fn debrid_set_preferred(state: State<'_, AppState>, service: String) -> Result<DebridStatus, String> {
    state.ensure_unlocked()?;
    let service = Service::parse(&service).ok_or("unknown service")?;
    let text = serde_json::to_string(service.id()).map_err(|e| e.to_string())?;
    std::fs::write(order_file(&state, state.active_profile()), text).map_err(|e| e.to_string())?;
    Ok(debrid_status_of(&state))
}

// ---------------------------------------------------------------- sources

/// How a source can use a debrid service: a torrent (its hash), or an addon's own debrid source.
enum Use {
    Torrent(String),
    Client(Service),
}

fn eligible(stream: &StreamItem) -> Option<Use> {
    match kind_of(stream) {
        StreamKind::Torrent => torbox::info_hash(stream).map(Use::Torrent),
        StreamKind::Debrid if torbox::is_torbox_client_stream(stream) => Some(Use::Client(Service::Torbox)),
        StreamKind::Debrid => client_stream_provider(stream).map(|p| Use::Client(Service::Other(p))),
        _ => None,
    }
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    /// `cached` (opens through a service), `try` (a service will be asked when opened),
    /// `notCached` (P2P).
    pub states: HashMap<String, String>,
    /// The service of a `cached` / `try` source: TB, RD, AD, PM, DL.
    pub codes: HashMap<String, String>,
}

async fn check_service(state: &AppState, service: Service, key: &str, hashes: &[String]) -> Result<Vec<String>, String> {
    match service {
        Service::Torbox => Ok(state.torbox.check_cached(key, hashes).await.map_err(|e| e.to_string())?.into_keys().collect()),
        Service::Other(p) => Ok(state.debrid_client.check_cached(p, key, hashes).await.map_err(|e| e.to_string())?.into_iter().collect()),
    }
}

/// What the connected services can do with the given sources (keys of the last searches).
/// Without a connected service: nothing.
#[tauri::command]
pub async fn torbox_check(state: State<'_, AppState>, keys: Vec<String>) -> Result<CheckResult, String> {
    state.ensure_unlocked()?;
    let services = connected(&state);
    let mut out = CheckResult::default();
    if services.is_empty() {
        return Ok(out);
    }
    let mut by_hash: HashMap<String, Vec<String>> = HashMap::new();
    for key in keys {
        let Some(stream) = state.streams.get(&key) else { continue };
        match eligible(&stream) {
            Some(Use::Client(service)) if services.iter().any(|(s, _)| *s == service) => {
                out.states.insert(key.clone(), "cached".into());
                out.codes.insert(key, service.code().into());
            }
            Some(Use::Torrent(hash)) => by_hash.entry(hash).or_default().push(key),
            _ => {}
        }
    }
    if by_hash.is_empty() {
        return Ok(out);
    }
    // Services that can check: asked once per hash and key.
    for (service, key) in services.iter().filter(|(s, _)| s.can_check()) {
        let print = fingerprint(*service, key);
        let unknown: Vec<String> = by_hash.keys().filter(|h| state.debrid.known(&print, h).is_none()).cloned().collect();
        if unknown.is_empty() {
            continue;
        }
        match check_service(&state, *service, key, &unknown).await {
            Ok(found) => {
                for hash in unknown {
                    let cached = found.contains(&hash);
                    state.debrid.remember(&print, &hash, cached);
                }
            }
            Err(error) => eprintln!("[debrid] {} check failed: {error}", service.id()),
        }
    }
    for (hash, keys) in by_hash {
        let mut verdict: Option<(&str, &str)> = None;
        for (service, key) in &services {
            match state.debrid.known(&fingerprint(*service, key), &hash) {
                Some(true) => {
                    verdict = Some(("cached", service.code()));
                    break;
                }
                Some(false) => {}
                None if !service.can_check() && verdict.is_none() => verdict = Some(("try", service.code())),
                None => {}
            }
        }
        for key in keys {
            match verdict {
                Some((state_name, code)) => {
                    out.states.insert(key.clone(), state_name.into());
                    out.codes.insert(key, code.into());
                }
                None => {
                    out.states.insert(key, "notCached".into());
                }
            }
        }
    }
    Ok(out)
}

/// Asks one service for the wanted file.
pub async fn resolve_with(state: &AppState, service: Service, key: &str, wanted: &Wanted) -> Result<Resolved, Failure> {
    match service {
        Service::Torbox => state.torbox.resolve(key, wanted).await.map_err(|e| match e {
            TorboxError::NotCached | TorboxError::NoFile => Failure::NotThere(e.to_string()),
            other => Failure::Error(other.to_string()),
        }),
        Service::Other(p) => state.debrid_client.resolve(p, key, wanted).await.map_err(|e| match e {
            DebridError::NotCached(_) | DebridError::NoFile(_) => Failure::NotThere(e.to_string()),
            other => Failure::Error(other.to_string()),
        }),
    }
}

pub enum Failure {
    /// The service does not have this torrent (or the episode inside it).
    NotThere(String),
    Error(String),
}

/// Gets the link of a source before it plays or downloads, from the first connected service that
/// has it. `season`/`episode` pick the file of a season pack.
pub async fn prepare(state: &AppState, key: &str, season: Option<i64>, episode: Option<i64>) -> Result<Resolved, String> {
    if let Some(link) = state.debrid.link(key) {
        return Ok(link);
    }
    let services = connected(state);
    if services.is_empty() {
        return Err("Configure um serviço em Configurações → Debrid.".into());
    }
    let stream = state.streams.get(key).ok_or("unknown stream (search again)")?;
    let usage = eligible(&stream).ok_or("Esta fonte não pode ser aberta por um serviço debrid.")?;
    let wanted = torbox::wanted(&stream, season, episode).ok_or("Esta fonte não informa o torrent.")?;
    let candidates: Vec<(Service, String)> = match &usage {
        Use::Client(service) => services.into_iter().filter(|(s, _)| s == service).collect(),
        Use::Torrent(hash) => services.into_iter().filter(|(s, k)| state.debrid.known(&fingerprint(*s, k), hash) != Some(false)).collect(),
    };
    let mut last_error: Option<String> = None;
    let mut not_there: Option<String> = None;
    for (service, api_key) in candidates {
        match resolve_with(state, service, &api_key, &wanted).await {
            Ok(resolved) => {
                if let Use::Torrent(hash) = &usage {
                    state.debrid.remember(&fingerprint(service, &api_key), hash, true);
                }
                state.debrid.resolved.lock().expect("debrid links").insert(key.to_string(), (Instant::now(), resolved.clone(), service));
                return Ok(resolved);
            }
            Err(Failure::NotThere(text)) => {
                if let Use::Torrent(hash) = &usage {
                    state.debrid.remember(&fingerprint(service, &api_key), hash, false);
                }
                not_there.get_or_insert(text);
            }
            Err(Failure::Error(text)) => {
                eprintln!("[debrid] {}: {text}", service.id());
                last_error.get_or_insert(text);
            }
        }
    }
    Err(last_error.or(not_there).unwrap_or_else(|| "Nenhum serviço debrid conectado tem esta fonte.".into()))
}

/// The service that has this source ready (already linked, or checked as cached), if any.
pub fn ready_service(state: &AppState, key: &str) -> Option<Service> {
    if let Some((_, service)) = state.debrid.entry(key) {
        return Some(service);
    }
    let stream = state.streams.get(key)?;
    let services = connected(state);
    match eligible(&stream)? {
        Use::Client(service) => services.iter().any(|(s, _)| *s == service).then_some(service),
        Use::Torrent(hash) => services.into_iter().find(|(s, k)| state.debrid.known(&fingerprint(*s, k), &hash) == Some(true)).map(|(s, _)| s),
    }
}

/// The key of a connected service (downloads resume through the service that started them).
pub fn connected_key(state: &AppState, service: Service) -> Option<String> {
    key_of(state, service)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub filename: Option<String>,
    pub size: Option<u64>,
    /// TB, RD, AD, PM or DL.
    pub code: Option<&'static str>,
}

#[tauri::command]
pub async fn torbox_prepare(state: State<'_, AppState>, key: String, season: Option<i64>, episode: Option<i64>) -> Result<Prepared, String> {
    state.ensure_unlocked()?;
    let resolved = prepare(&state, &key, season, episode).await?;
    let code = state.debrid.entry(&key).map(|(_, s)| s.code());
    Ok(Prepared { filename: resolved.filename, size: resolved.size, code })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_parse_and_keep_the_torbox_file_name() {
        for s in Service::ALL {
            assert_eq!(Service::parse(s.id()), Some(s));
        }
        assert_eq!(Service::parse("TorBox"), Some(Service::Torbox));
        assert_eq!(Service::ALL.iter().filter(|s| s.can_check()).count(), 2, "TorBox and Premiumize");
        assert_ne!(fingerprint(Service::Torbox, "k"), fingerprint(Service::Other(Provider::RealDebrid), "k"));
    }
}
