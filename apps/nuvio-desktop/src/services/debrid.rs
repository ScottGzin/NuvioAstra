//! Debrid services (`features/debrid` in the original): Settings → Services → Connected Services.
//!
//! Five providers are supported, all with the user's own API key: TorBox, Real-Debrid, AllDebrid,
//! Premiumize and Debrid-Link. The key stays in Rust (and in the Nuvio account as a
//! `debrid:<provider>` credential); the UI only sees which providers are connected. A torrent
//! source is asked of the active provider and plays from the direct link it returns. When the
//! torrent is not cached the source keeps playing through the local P2P engine instead.
//!
//! The port keeps the original's `selectDebridFile` order: the filename the addon named, then the
//! episode, then the file index, then the largest video.

use nuvio_core::net;
use nuvio_core::stream::StreamItem;
use nuvio_store::{default_legacy_data_dir, LegacyStore};
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;
use std::time::Duration;

const TORBOX_BASE: &str = "https://api.torbox.app/v1/api";
const RD_BASE: &str = "https://api.real-debrid.com/rest/1.0";
const AD_BASE: &str = "https://api.alldebrid.com";
const PM_BASE: &str = "https://www.premiumize.me/api";
const DL_BASE: &str = "https://debrid-link.com/api/v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Torbox,
    RealDebrid,
    AllDebrid,
    Premiumize,
    DebridLink,
}

impl Provider {
    pub const ALL: [Provider; 5] = [Self::Torbox, Self::RealDebrid, Self::AllDebrid, Self::Premiumize, Self::DebridLink];

    pub fn id(self) -> &'static str {
        match self {
            Self::Torbox => "torbox",
            Self::RealDebrid => "realdebrid",
            Self::AllDebrid => "alldebrid",
            Self::Premiumize => "premiumize",
            Self::DebridLink => "debridlink",
        }
    }

    /// Short mark shown on a source card: TB / RD / AD / PM / DL.
    pub fn code(self) -> &'static str {
        match self {
            Self::Torbox => "TB",
            Self::RealDebrid => "RD",
            Self::AllDebrid => "AD",
            Self::Premiumize => "PM",
            Self::DebridLink => "DL",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Torbox => "TorBox",
            Self::RealDebrid => "Real-Debrid",
            Self::AllDebrid => "AllDebrid",
            Self::Premiumize => "Premiumize",
            Self::DebridLink => "Debrid-Link",
        }
    }

    /// The Nuvio account credential name (`ProviderCredentialSync`).
    pub fn credential(self) -> String {
        format!("debrid:{}", self.id())
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase().replace(['-', '_'], "");
        let value = value.strip_prefix("debrid:").unwrap_or(&value);
        Self::ALL.into_iter().find(|p| p.id().replace(['-', '_'], "") == value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    /// Kept for data saved by earlier versions. New keys live in `keys["torbox"]`.
    pub torbox_api_key: String,
    pub keys: BTreeMap<String, String>,
    pub active: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        // `DebridSettings()` in the original: off until a provider is connected and enabled.
        Self { enabled: false, torbox_api_key: String::new(), keys: BTreeMap::new(), active: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub id: &'static str,
    pub label: &'static str,
    pub connected: bool,
    pub from_account: bool,
    pub from_original: bool,
    pub account: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub enabled: bool,
    pub providers: Vec<ProviderView>,
    /// Provider that will resolve a source, `None` when no key is connected.
    pub active: Option<&'static str>,
    // Kept so older builds of the UI keep working.
    pub torbox_connected: bool,
    pub torbox_from_account: bool,
    pub torbox_from_original: bool,
    pub account: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsInput {
    pub enabled: bool,
    /// Older UI field: TorBox key. `None` keeps it, `""` clears it.
    pub torbox_api_key: Option<String>,
    /// Provider id → key. `None` keeps the key, `Some("")` clears it.
    pub keys: BTreeMap<String, Option<String>>,
    /// Provider id to use; an empty string clears the choice.
    pub active: Option<String>,
}

impl Default for SettingsInput {
    fn default() -> Self {
        Self { enabled: false, torbox_api_key: None, keys: BTreeMap::new(), active: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOrigin {
    Own,
    /// From the Nuvio account (`ProviderCredentialSync`).
    Account,
    /// From the original Nuvio Desktop app (TorBox only).
    Original,
}

#[derive(Debug, Clone)]
pub struct Active {
    pub provider: Provider,
    pub key: String,
}

fn legacy_values() -> Option<HashMap<String, String>> {
    let dir = default_legacy_data_dir().ok()?;
    LegacyStore::open(dir).ok()?.read_store("nuvio_debrid_settings").ok()
}

fn stored(store: &nuvio_store::nextgen::AppStore) -> BTreeMap<i64, Settings> {
    store.read("debrid-settings").ok().flatten().unwrap_or_default()
}

pub fn settings(store: &nuvio_store::nextgen::AppStore, profile: i64) -> Settings {
    stored(store).remove(&profile).unwrap_or_default()
}

fn own_key(settings: &Settings, provider: Provider) -> String {
    let from_map = settings.keys.get(provider.id()).map(|key| key.trim()).unwrap_or_default();
    if !from_map.is_empty() {
        return from_map.to_string();
    }
    if provider == Provider::Torbox {
        return settings.torbox_api_key.trim().to_string();
    }
    String::new()
}

/// Effective key: Astra's own, else the account's, else the original app's TorBox key.
pub fn key_for(store: &nuvio_store::nextgen::AppStore, profile: i64, provider: Provider) -> Option<(String, KeyOrigin)> {
    let own = own_key(&settings(store, profile), provider);
    if !own.is_empty() {
        return Some((own, KeyOrigin::Own));
    }
    if let Some(account) = store.provider_key(profile, &provider.credential()).filter(|key| !key.trim().is_empty()) {
        return Some((account.trim().to_string(), KeyOrigin::Account));
    }
    if provider == Provider::Torbox {
        let legacy = legacy_values()?;
        let key = legacy.get(&format!("debrid_torbox_api_key_{profile}"))?.trim().to_string();
        return (!key.is_empty()).then_some((key, KeyOrigin::Original));
    }
    None
}

fn enabled(store: &nuvio_store::nextgen::AppStore, profile: i64) -> bool {
    let all = stored(store);
    if all.contains_key(&profile) {
        return all.get(&profile).is_some_and(|s| s.enabled);
    }
    legacy_values()
        .and_then(|v| v.get(&format!("debrid_enabled_{profile}")).map(|v| v.trim().eq_ignore_ascii_case("true")))
        .unwrap_or(false)
}

pub fn active_provider(store: &nuvio_store::nextgen::AppStore, profile: i64) -> Option<Provider> {
    let saved = settings(store, profile);
    if let Some(chosen) = saved.active.as_deref().and_then(Provider::parse) {
        if key_for(store, profile, chosen).is_some() {
            return Some(chosen);
        }
    }
    Provider::ALL.into_iter().find(|provider| key_for(store, profile, *provider).is_some())
}

/// The provider and key used to resolve playable links, only when debrid is enabled.
pub fn access(store: &nuvio_store::nextgen::AppStore, profile: i64) -> Option<Active> {
    available(store, profile).into_iter().next()
}

/// Every connected provider, preferred one first. Used to try them in turn: if a torrent is not
/// cached on TorBox but is on Real-Debrid, playback still succeeds.
pub fn available(store: &nuvio_store::nextgen::AppStore, profile: i64) -> Vec<Active> {
    if !enabled(store, profile) {
        return Vec::new();
    }
    let mut order = Vec::new();
    if let Some(preferred) = active_provider(store, profile) {
        order.push(preferred);
    }
    for provider in Provider::ALL {
        if !order.contains(&provider) {
            order.push(provider);
        }
    }
    order
        .into_iter()
        .filter_map(|provider| key_for(store, profile, provider).map(|(key, _)| Active { provider, key }))
        .collect()
}

/// Short codes of every connected provider, for the badge on a source card.
pub fn available_codes(store: &nuvio_store::nextgen::AppStore, profile: i64) -> Vec<&'static str> {
    available(store, profile).into_iter().map(|active| active.provider.code()).collect()
}

async fn provider_view(store: &nuvio_store::nextgen::AppStore, profile: i64, provider: Provider) -> ProviderView {
    let key = key_for(store, profile, provider);
    let account = match &key {
        Some((key, _)) => provider_account(provider, key).await,
        None => None,
    };
    ProviderView {
        id: provider.id(),
        label: provider.label(),
        connected: key.is_some(),
        from_account: key.as_ref().is_some_and(|(_, origin)| *origin == KeyOrigin::Account),
        from_original: key.as_ref().is_some_and(|(_, origin)| *origin == KeyOrigin::Original),
        account,
    }
}

pub async fn view(store: &nuvio_store::nextgen::AppStore, profile: i64) -> View {
    // Validate the keys concurrently: a provider that is offline must not hold up the page.
    let (torbox, realdebrid, alldebrid, premiumize, debridlink) = tokio::join!(
        provider_view(store, profile, Provider::Torbox),
        provider_view(store, profile, Provider::RealDebrid),
        provider_view(store, profile, Provider::AllDebrid),
        provider_view(store, profile, Provider::Premiumize),
        provider_view(store, profile, Provider::DebridLink),
    );
    View {
        enabled: enabled(store, profile),
        active: active_provider(store, profile).map(Provider::id),
        torbox_connected: torbox.connected,
        torbox_from_account: torbox.from_account,
        torbox_from_original: torbox.from_original,
        account: torbox.account.clone(),
        providers: vec![torbox, realdebrid, alldebrid, premiumize, debridlink],
    }
}

/// Saves the Connected Services page. A key entered here is also sent with the account's
/// credentials on the next sync; keys are validated with their provider before being stored.
/// Applies a settings-page edit to the stored document, without any network call.
fn merge_input(mut saved: Settings, input: &SettingsInput) -> Settings {
    // A key saved by an older version lives in `torbox_api_key`; move it into the map so saving
    // an unrelated setting (e.g. the enabled toggle) cannot silently drop it.
    if !saved.torbox_api_key.trim().is_empty() && !saved.keys.contains_key(Provider::Torbox.id()) {
        saved.keys.insert(Provider::Torbox.id().to_string(), saved.torbox_api_key.clone());
    }
    for (id, key) in &input.keys {
        let Some(provider) = Provider::parse(id) else { continue };
        if let Some(key) = key {
            saved.keys.insert(provider.id().to_string(), key.trim().to_string());
        }
    }
    if let Some(key) = &input.torbox_api_key {
        saved.keys.entry(Provider::Torbox.id().to_string()).or_insert_with(|| key.trim().to_string());
    }
    if let Some(active) = &input.active {
        saved.active = Some(active.trim().to_string()).filter(|v| !v.is_empty());
    }
    saved.enabled = input.enabled;
    // Keep the old field in step so a downgrade still finds the TorBox key.
    if let Some(key) = saved.keys.get(Provider::Torbox.id()) {
        saved.torbox_api_key = key.clone();
    }
    saved
}

pub async fn update(store: &nuvio_store::nextgen::AppStore, profile: i64, input: SettingsInput) -> Result<View, String> {
    let mut changed: Vec<Provider> = Vec::new();
    for (id, key) in &input.keys {
        let Some(provider) = Provider::parse(id) else { continue };
        let Some(key) = key else { continue };
        let key = key.trim();
        if !key.is_empty() && provider_account(provider, key).await.is_none() {
            return Err(format!("{} recusou esta chave da API.", provider.label()));
        }
        changed.push(provider);
    }
    if let Some(key) = &input.torbox_api_key {
        let key = key.trim();
        if !key.is_empty() && !input.keys.contains_key(Provider::Torbox.id()) && provider_account(Provider::Torbox, key).await.is_none() {
            return Err(format!("{} recusou esta chave da API.", Provider::Torbox.label()));
        }
        changed.push(Provider::Torbox);
    }
    let next = merge_input(settings(store, profile), &input);
    let mut all = stored(store);
    all.insert(profile, next.clone());
    store.write("debrid-settings", &all).map_err(|e| e.to_string())?;

    if let Ok(mut all) = store.provider_credentials() {
        let mut providers = all.remove(&profile).unwrap_or_default();
        for provider in changed {
            let key = own_key(&next, provider);
            if key.is_empty() {
                providers.remove(&provider.credential());
            } else {
                providers.insert(provider.credential(), key);
            }
        }
        let _ = store.set_provider_credentials(profile, providers);
    }
    Ok(view(store, profile).await)
}

// ---------------------------------------------------------------- provider APIs

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| net::http_client().expect("debrid http client"))
}

async fn json(request: RequestBuilder) -> Result<(u16, Value), String> {
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|e| e.to_string())?;
    let body = serde_json::from_str(&body).unwrap_or(Value::Null);
    Ok((status, body))
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

fn id_text(value: Option<&Value>) -> Option<String> {
    text(value).or_else(|| value.and_then(Value::as_i64).map(|id| id.to_string()))
}

fn as_objects(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(object @ Value::Object(_)) => vec![object],
        _ => Vec::new(),
    }
}

fn api_error(body: &Value) -> String {
    ["error", "message", "detail"]
        .iter()
        .find_map(|key| text(body.get(*key)))
        .unwrap_or_default()
}

fn api_error_or(body: &Value, fallback: &str) -> String {
    let detail = api_error(body);
    if detail.is_empty() { fallback.to_string() } else { detail }
}

pub async fn provider_account(provider: Provider, key: &str) -> Option<String> {
    let key = key.trim().to_string();
    // The settings page checks up to five providers at once; never make the UI wait for a timeout.
    tokio::time::timeout(Duration::from_secs(12), async {
        match provider {
            Provider::Torbox => torbox_account(&key).await,
            Provider::RealDebrid => {
                let (_, body) = json(http().get(format!("{RD_BASE}/user")).bearer_auth(&key)).await.ok()?;
                if body.get("error").is_some() {
                    return None;
                }
                text(body.get("email")).or_else(|| text(body.get("username")))
            }
            Provider::AllDebrid => {
                let (_, body) = json(http().get(format!("{AD_BASE}/v4/user")).query(&[("apikey", &key)]).bearer_auth(&key)).await.ok()?;
                if body.get("status").and_then(Value::as_str) != Some("success") {
                    return None;
                }
                let data = body.get("data")?;
                text(data.get("email")).or_else(|| text(data.get("username")))
            }
            Provider::Premiumize => {
                let (_, body) = json(http().get(format!("{PM_BASE}/account/info")).bearer_auth(&key)).await.ok()?;
                if body.get("status").and_then(Value::as_str) != Some("success") {
                    return None;
                }
                text(body.get("email")).or_else(|| text(body.get("customer_id"))).or_else(|| Some("conta Premiumize".into()))
            }
            Provider::DebridLink => {
                let (_, body) = json(http().get(format!("{DL_BASE}/account/infos")).bearer_auth(&key)).await.ok()?;
                if body.get("success").and_then(Value::as_bool) != Some(true) {
                    return None;
                }
                let data = body.get("value")?;
                text(data.get("email")).or_else(|| text(data.get("login")))
            }
        }
    })
    .await
    .ok()
    .flatten()
}

/// `TorboxApiClient.validateApiKey`: `/user/me`; the e-mail is shown on the provider card.
pub async fn torbox_account(key: &str) -> Option<String> {
    let (status, body) = json(http().get(format!("{TORBOX_BASE}/user/me")).bearer_auth(key.trim())).await.ok()?;
    if status != 200 || body.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    text(body.get("data")?.get("email"))
}

// ---------------------------------------------------------------- stream resolution

#[derive(Debug, Clone, Default, PartialEq)]
struct ResolveInfo {
    magnet: Option<String>,
    hash: Option<String>,
    file_idx: Option<usize>,
    filenames: Vec<String>,
    season: Option<i64>,
    episode: Option<i64>,
    trackers: Vec<String>,
}

fn resolve_info(stream: &StreamItem) -> ResolveInfo {
    let resolve = stream.client_resolve.as_ref();
    let raw = resolve.and_then(|r| r.pointer("/stream/raw"));
    let mut filenames: Vec<String> = Vec::new();
    for name in [stream.behavior_hints.filename.as_deref(), resolve.and_then(|r| r.get("filename")).and_then(Value::as_str), raw.and_then(|r| r.get("filename")).and_then(Value::as_str)] {
        if let Some(name) = name.map(str::trim).filter(|v| !v.is_empty()) {
            if !filenames.iter().any(|existing| existing == name) {
                filenames.push(name.to_string());
            }
        }
    }
    let mut trackers: Vec<String> = stream.sources.clone();
    if let Some(sources) = resolve.and_then(|r| r.get("sources")).and_then(Value::as_array) {
        for source in sources.iter().filter_map(Value::as_str) {
            if !trackers.iter().any(|t| t == source) {
                trackers.push(source.to_string());
            }
        }
    }
    ResolveInfo {
        magnet: [stream.url.as_deref(), stream.external_url.as_deref(), resolve.and_then(|r| r.get("magnetUri")).and_then(Value::as_str)]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|v| v.to_ascii_lowercase().starts_with("magnet:"))
            .map(str::to_string),
        hash: stream
            .info_hash
            .clone()
            .filter(|h| !h.trim().is_empty())
            .or_else(|| resolve.and_then(|r| r.get("infoHash")).and_then(Value::as_str).map(str::to_string)),
        file_idx: resolve.and_then(|r| r.get("fileIdx")).and_then(Value::as_u64).map(|v| v as usize),
        filenames,
        season: resolve.and_then(|r| r.get("season")).and_then(Value::as_i64),
        episode: resolve.and_then(|r| r.get("episode")).and_then(Value::as_i64),
        trackers,
    }
}

/// `DebridMagnetBuilder.fromStream`: an existing magnet, else the info hash with the file name and
/// the trackers the addon listed.
fn magnet_uri(info: &ResolveInfo) -> Option<String> {
    if let Some(magnet) = &info.magnet {
        return Some(magnet.clone());
    }
    let hash = info.hash.as_deref()?.trim();
    if hash.is_empty() {
        return None;
    }
    let mut magnet = format!("magnet:?xt=urn:btih:{}", encode(hash));
    if let Some(name) = info.filenames.first() {
        magnet.push_str("&dn=");
        magnet.push_str(&encode(name));
    }
    for tracker in info.trackers.iter().map(|t| t.trim()).filter(|t| !t.is_empty() && !t.to_ascii_lowercase().starts_with("dht:")) {
        let tracker = tracker.strip_prefix("tracker:").unwrap_or(tracker).trim();
        if !tracker.is_empty() {
            magnet.push_str("&tr=");
            magnet.push_str(&encode(tracker));
        }
    }
    Some(magnet)
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
struct ProviderFile {
    id: String,
    name: String,
    size: i64,
    playable: bool,
}

impl ProviderFile {
    fn parse(value: &Value) -> Option<Self> {
        let id = id_text(value.get("id"))?;
        let name = text(value.get("name"))
            .or_else(|| text(value.get("path")))
            .or_else(|| text(value.get("filename")))
            .unwrap_or_default();
        let mime = text(value.get("mimetype")).unwrap_or_default().to_ascii_lowercase();
        let playable = mime.starts_with("video/") || has_video_extension(&name);
        Some(Self { id, name, size: value.get("size").or_else(|| value.get("bytes")).and_then(Value::as_i64).unwrap_or(0), playable })
    }
}

fn has_video_extension(name: &str) -> bool {
    const EXTENSIONS: [&str; 10] = [".mp4", ".mkv", ".webm", ".avi", ".mov", ".m4v", ".ts", ".m2ts", ".wmv", ".flv"];
    let lower = name.to_ascii_lowercase();
    EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

fn episode_matches(name: &str, season: i64, episode: i64) -> bool {
    let lower = name.to_ascii_lowercase();
    let patterns = [format!("s{season:02}e{episode:02}"), format!("s{season}e{episode}"), format!("{season}x{episode}")];
    patterns.iter().any(|pattern| {
        lower.match_indices(pattern).any(|(index, _)| !lower[index + pattern.len()..].chars().next().is_some_and(|c| c.is_ascii_digit()))
    })
}

/// `selectDebridFile`: the addon's filename, then the episode, then the index, then the largest
/// video. `None` when a named file or episode is not in the torrent.
fn select_file<'a>(files: &'a [ProviderFile], info: &ResolveInfo) -> Option<&'a ProviderFile> {
    let playable: Vec<&ProviderFile> = files.iter().filter(|f| f.playable).collect();
    if playable.is_empty() {
        return None;
    }
    let normalized = |name: &str| name.trim().replace('\\', "/");
    let names: Vec<String> = info.filenames.iter().map(|n| normalized(n)).collect();
    for name in &names {
        let basename = name.rsplit('/').next().unwrap_or(name);
        let found: Vec<&ProviderFile> = playable
            .iter()
            .copied()
            .filter(|file| {
                let path = normalized(&file.name);
                path == *name || path.rsplit('/').next() == Some(basename)
            })
            .collect();
        if !found.is_empty() {
            return found.first().copied();
        }
    }
    if let (Some(season), Some(episode)) = (info.season, info.episode) {
        let found: Vec<&ProviderFile> = playable
            .iter()
            .copied()
            .filter(|file| episode_matches(file.name.rsplit('/').next().unwrap_or(&file.name), season, episode))
            .collect();
        if !found.is_empty() {
            return found.first().copied();
        }
    }
    if !names.is_empty() || info.season.zip(info.episode).is_some() {
        return None;
    }
    if let Some(index) = info.file_idx {
        if let Some(file) = playable.get(index) {
            return Some(*file);
        }
    }
    playable.into_iter().max_by_key(|file| file.size)
}

fn not_cached(label: &str) -> String {
    format!("Esta fonte não está em cache no {label}.")
}

// ---------------------------------------------------------------- TorBox

fn multipart(fields: &[(&str, &str)]) -> (String, Vec<u8>) {
    let boundary = format!("NuvioAstra{}", fields.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>());
    let mut body: Vec<u8> = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn data_value(response: &Value) -> Option<&Value> {
    match response.get("data") {
        Some(Value::Array(items)) => items.first(),
        Some(value) => Some(value),
        None => None,
    }
}

fn tb_success(response: &Value) -> bool {
    response.get("success").and_then(Value::as_bool) != Some(false)
}

fn tb_detail(response: &Value) -> String {
    ["detail", "error"].iter().find_map(|key| text(response.get(*key))).unwrap_or_default()
}

async fn create_torrent(key: &str, magnet: &str) -> Result<i64, String> {
    let (content_type, body) = multipart(&[("magnet", magnet), ("add_only_if_cached", "true"), ("allow_zip", "false")]);
    let (status, body) = json(
        http()
            .post(format!("{TORBOX_BASE}/torrents/createtorrent"))
            .bearer_auth(key)
            .header("Content-Type", content_type)
            .body(body),
    )
    .await?;
    if !tb_success(&body) {
        let detail = tb_detail(&body);
        return Err(if detail.to_ascii_lowercase().contains("cache") { not_cached("TorBox") } else if detail.is_empty() { format!("TorBox HTTP {status}") } else { detail });
    }
    id_text(data_value(&body).and_then(|d| d.get("torrent_id").or_else(|| d.get("id"))))
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| not_cached("TorBox"))
}

async fn wait_torbox_files(key: &str, torrent_id: i64) -> Result<Vec<ProviderFile>, String> {
    for attempt in 0..12 {
        let (_, body) = json(http().get(format!("{TORBOX_BASE}/torrents/mylist?id={torrent_id}&bypass_cache=true")).bearer_auth(key)).await?;
        if !tb_success(&body) {
            return Err(if tb_detail(&body).is_empty() { "O TorBox não devolveu os ficheiros.".into() } else { tb_detail(&body) });
        }
        let files: Vec<ProviderFile> = data_value(&body)
            .and_then(|d| d.get("files"))
            .map(|files| as_objects(Some(files)).into_iter().filter_map(ProviderFile::parse).collect())
            .unwrap_or_default();
        if !files.is_empty() {
            return Ok(files);
        }
        if attempt < 11 {
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
    }
    Err("O TorBox ainda não preparou os ficheiros desta fonte.".into())
}

async fn resolve_torbox_stream(key: &str, info: &ResolveInfo) -> Result<String, String> {
    let magnet = magnet_uri(info).ok_or("Esta fonte não tem magnet para o TorBox.")?;
    let torrent_id = create_torrent(key, &magnet).await?;
    let files = wait_torbox_files(key, torrent_id).await?;
    let file = select_file(&files, info).ok_or("O ficheiro desta fonte não está no TorBox.")?;
    let (_, body) = json(
        http()
            .get(format!("{TORBOX_BASE}/torrents/requestdl"))
            .query(&[("token", key), ("torrent_id", &torrent_id.to_string()), ("file_id", &file.id), ("zip_link", "false"), ("redirect", "false"), ("append_name", "false")])
            .bearer_auth(key),
    )
    .await?;
    if !tb_success(&body) {
        return Err(if tb_detail(&body).is_empty() { "O TorBox não devolveu a ligação.".into() } else { tb_detail(&body) });
    }
    data_value(&body).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).ok_or_else(|| "O TorBox não devolveu a ligação.".into())
}

// ---------------------------------------------------------------- Real-Debrid

async fn rd_add_magnet(key: &str, magnet: &str) -> Result<String, String> {
    let (_, created) = json(http().post(format!("{RD_BASE}/torrents/addMagnet")).bearer_auth(key).form(&[("magnet", magnet)])).await?;
    id_text(created.get("id")).ok_or_else(|| {
        let detail = api_error(&created);
        if detail.to_ascii_lowercase().contains("infringing") {
            "O Real-Debrid recusou este torrent (ficheiro considerado infrator).".to_string()
        } else if detail.is_empty() {
            not_cached("Real-Debrid")
        } else {
            detail
        }
    })
}

/// A bare `magnet:?xt=urn:btih:<hash>`. Some services reject a reconstructed magnet because of its
/// trackers or display name even when the hash itself is available.
fn bare_magnet(info: &ResolveInfo) -> Option<String> {
    let hash = info.hash.clone().or_else(|| {
        info.magnet.as_deref()?.to_ascii_lowercase().find("xt=urn:btih:").and_then(|at| {
            let rest = info.magnet.as_deref()?[at + 12..].split('&').next()?;
            Some(rest.to_string())
        })
    })?;
    let hash = hash.trim();
    let valid = (hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit())) || (hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_alphanumeric()));
    valid.then(|| format!("magnet:?xt=urn:btih:{hash}"))
}

async fn resolve_realdebrid(key: &str, info: &ResolveInfo) -> Result<String, String> {
    let magnet = magnet_uri(info).ok_or("Esta fonte não tem magnet para o Real-Debrid.")?;
    let id = match rd_add_magnet(key, &magnet).await {
        Ok(id) => id,
        Err(first_error) => {
            if !first_error.contains("infrator") && !first_error.to_ascii_lowercase().contains("infringing") {
                return Err(first_error);
            }
            match bare_magnet(info) {
                Some(bare) if bare != magnet => {
                    eprintln!("[rd] retrying with a bare magnet");
                    rd_add_magnet(key, &bare).await?
                }
                _ => return Err(first_error),
            }
        }
    };
    let mut torrent = json(http().get(format!("{RD_BASE}/torrents/info/{id}")).bearer_auth(key)).await?.1;
    let files: Vec<ProviderFile> = as_objects(torrent.get("files")).into_iter().filter_map(ProviderFile::parse).collect();
    let file = select_file(&files, info).ok_or("O ficheiro desta fonte não está no Real-Debrid.")?;
    let needs_selection = text(torrent.get("status")).is_some_and(|s| s.eq_ignore_ascii_case("waiting_files_selection"))
        || as_objects(torrent.get("files")).iter().any(|f| !f.get("selected").and_then(Value::as_i64).is_some_and(|v| v == 1));
    if needs_selection {
        json(http().post(format!("{RD_BASE}/torrents/selectFiles/{id}")).bearer_auth(key).form(&[("files", file.id.as_str())])).await?;
        for _ in 0..8 {
            torrent = json(http().get(format!("{RD_BASE}/torrents/info/{id}")).bearer_auth(key)).await?.1;
            if text(torrent.get("status")).is_some_and(|s| s.eq_ignore_ascii_case("downloaded")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
    }
    if !text(torrent.get("status")).is_some_and(|s| s.eq_ignore_ascii_case("downloaded")) {
        return Err(not_cached("Real-Debrid"));
    }
    let links = as_objects(torrent.get("links"));
    let link = links.first().and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty()).ok_or("O Real-Debrid não devolveu a ligação.")?;
    let (_, unrestricted) = json(http().post(format!("{RD_BASE}/unrestrict/link")).bearer_auth(key).form(&[("link", link)])).await?;
    text(unrestricted.get("download")).ok_or_else(|| api_error_or(&unrestricted, "O Real-Debrid não devolveu a ligação."))
}

// ---------------------------------------------------------------- AllDebrid

async fn ad_request(key: &str, path: &str, query: Vec<(&str, &str)>) -> Result<Value, String> {
    let (_, body) = json(http().get(format!("{AD_BASE}{path}")).query(&query).bearer_auth(key)).await?;
    if body.get("status").and_then(Value::as_str) != Some("success") {
        return Err(api_error_or(body.get("error").unwrap_or(&Value::Null), "O AllDebrid recusou o pedido."));
    }
    Ok(body)
}

async fn resolve_alldebrid(key: &str, info: &ResolveInfo) -> Result<String, String> {
    let magnet = magnet_uri(info).ok_or("Esta fonte não tem magnet para o AllDebrid.")?;
    let uploaded = ad_request(key, "/v4/magnet/upload", vec![("apikey", key), ("magnets[]", magnet.as_str())]).await?;
    let magnet = as_objects(uploaded.get("data").and_then(|d| d.get("magnets"))).into_iter().next().ok_or_else(not_cached_all)?;
    let id = id_text(magnet.get("id")).ok_or_else(not_cached_all)?;
    let status = ad_request(key, "/v4.1/magnet/status", vec![("apikey", key), ("id", id.as_str())]).await?;
    let magnets = as_objects(status.get("data").and_then(|d| d.get("magnets")));
    let item = magnets.into_iter().find(|m| id_text(m.get("id")).as_deref() == Some(id.as_str())).ok_or_else(not_cached_all)?;
    if item.get("statusCode").and_then(Value::as_i64) != Some(4) && !text(item.get("status")).is_some_and(|s| s.eq_ignore_ascii_case("ready")) {
        return Err(not_cached_all());
    }
    let files: Vec<ProviderFile> = as_objects(item.get("links"))
        .into_iter()
        .enumerate()
        .filter_map(|(i, link)| {
            let mut file = ProviderFile::parse(link)?;
            if file.id.is_empty() {
                file.id = i.to_string();
            }
            Some(file)
        })
        .collect();
    let file = select_file(&files, info).ok_or_else(not_cached_all)?;
    let link = as_objects(item.get("links"))
        .get(file.id.parse::<usize>().unwrap_or(0))
        .and_then(|l| text(l.get("link")))
        .or_else(|| text(item.get("links").and_then(|l| l.get(0)).and_then(|l| l.get("link"))))
        .ok_or_else(|| "O AllDebrid não devolveu a ligação.".to_string())?;
    let unlocked = ad_request(key, "/v4/link/unlock", vec![("apikey", key), ("link", link.as_str())]).await?;
    let url = text(unlocked.get("data").and_then(|d| d.get("link"))).ok_or_else(|| "O AllDebrid não devolveu a ligação.".to_string())?;
    Ok(if url.starts_with("http") { url } else { format!("https://{url}") })
}

fn not_cached_all() -> String {
    not_cached("AllDebrid")
}

// ---------------------------------------------------------------- Premiumize

async fn pm_request(key: &str, path: &str, params: Vec<(&str, &str)>) -> Result<Value, String> {
    let (_, body) = json(http().post(format!("{PM_BASE}/{path}")).bearer_auth(key).form(&params)).await?;
    if body.get("status").and_then(Value::as_str) != Some("success") {
        return Err(api_error_or(&body, "O Premiumize recusou o pedido."));
    }
    Ok(body)
}

async fn resolve_premiumize(key: &str, info: &ResolveInfo) -> Result<String, String> {
    let magnet = magnet_uri(info).ok_or("Esta fonte não tem magnet para o Premiumize.")?;
    let body = pm_request(key, "transfer/directdl", vec![("src", magnet.as_str())]).await?;
    let files: Vec<ProviderFile> = as_objects(body.get("content"))
        .into_iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            let mut file = ProviderFile::parse(entry)?;
            if file.id.is_empty() {
                file.id = i.to_string();
            }
            Some(file)
        })
        .collect();
    let file = select_file(&files, info).ok_or_else(|| not_cached("Premiumize"))?;
    as_objects(body.get("content"))
        .get(file.id.parse::<usize>().unwrap_or(0))
        .and_then(|entry| text(entry.get("link")))
        .ok_or_else(|| "O Premiumize não devolveu a ligação.".into())
}

// ---------------------------------------------------------------- Debrid-Link

async fn dl_request(key: &str, request: RequestBuilder) -> Result<Value, String> {
    let (_, body) = json(request.bearer_auth(key)).await?;
    if body.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(api_error_or(&body, "O Debrid-Link recusou o pedido."));
    }
    Ok(body)
}

async fn resolve_debridlink(key: &str, info: &ResolveInfo) -> Result<String, String> {
    let magnet = magnet_uri(info).ok_or("Esta fonte não tem magnet para o Debrid-Link.")?;
    let added = dl_request(key, http().post(format!("{DL_BASE}/seedbox/add")).json(&serde_json::json!({ "url": magnet, "wait": false }))).await?;
    let id = id_text(added.get("value").and_then(|v| v.get("id"))).ok_or_else(|| not_cached("Debrid-Link"))?;
    let listed = dl_request(key, http().get(format!("{DL_BASE}/seedbox/list")).query(&[("ids", id.as_str())])).await?;
    let torrent = as_objects(listed.get("value")).into_iter().next().ok_or_else(|| not_cached("Debrid-Link"))?;
    let files: Vec<ProviderFile> = as_objects(torrent.get("files")).into_iter().filter_map(ProviderFile::parse).collect();
    let file = select_file(&files, info).ok_or_else(|| not_cached("Debrid-Link"))?;
    let complete = |f: &Value| f.get("downloadPercent").and_then(Value::as_i64) == Some(100);
    let ready = as_objects(torrent.get("files")).into_iter().any(|f| id_text(f.get("id")).as_deref() == Some(file.id.as_str()) && complete(f));
    if !ready {
        return Err(not_cached("Debrid-Link"));
    }
    as_objects(torrent.get("files"))
        .into_iter()
        .find(|f| id_text(f.get("id")).as_deref() == Some(file.id.as_str()))
        .and_then(|f| text(f.get("downloadUrl")))
        .ok_or_else(|| "O Debrid-Link não devolveu a ligação.".into())
}

/// Resolves a torrent source through the active provider into a direct URL.
pub async fn resolve(active: &Active, stream: &StreamItem) -> Result<String, String> {
    let info = resolve_info(stream);
    match active.provider {
        Provider::Torbox => resolve_torbox_stream(&active.key, &info).await,
        Provider::RealDebrid => resolve_realdebrid(&active.key, &info).await,
        Provider::AllDebrid => resolve_alldebrid(&active.key, &info).await,
        Provider::Premiumize => resolve_premiumize(&active.key, &info).await,
        Provider::DebridLink => resolve_debridlink(&active.key, &info).await,
    }
}

/// True for an addon URL that is a debrid share/hoster page rather than a playable file.
pub fn is_share_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("real-debrid.com/d/") && !lower.contains(".download.real-debrid.com/")
        || lower.contains("alldebrid.") && lower.contains("/link/")
        || lower.contains("debrid-link.com/") && lower.contains("/dl/")
}

/// Some addons hand out a Real-Debrid *share page* (`real-debrid.com/d/CODE`) or a hoster link
/// instead of a direct stream. Turn those into a playable link with the API before the player sees
/// them. A normal `*.download.real-debrid.com/d/...` link is already direct and is left alone.
pub async fn unrestrict_share(active: &Active, url: &str) -> Option<String> {
    match active.provider {
        Provider::RealDebrid => {
            if !url.to_ascii_lowercase().contains("real-debrid.com/d/") || url.to_ascii_lowercase().contains(".download.real-debrid.com/") {
                return None;
            }
            let (_, body) = json(http().post(format!("{RD_BASE}/unrestrict/link")).bearer_auth(&active.key).form(&[("link", url)])).await.ok()?;
            text(body.get("download"))
        }
        Provider::AllDebrid => {
            if !url.to_ascii_lowercase().contains("alldebrid.") {
                return None;
            }
            let body = ad_request(&active.key, "/v4/link/unlock", vec![("apikey", &active.key), ("link", url)]).await.ok()?;
            text(body.get("data")?.get("link")).map(|link| if link.starts_with("http") { link } else { format!("https://{link}") })
        }
        Provider::Premiumize => {
            if !url.to_ascii_lowercase().contains("premiumize.") {
                return None;
            }
            let body = pm_request(&active.key, "transfer/directdl", vec![("src", url)]).await.ok()?;
            as_objects(body.get("content")).into_iter().next().and_then(|entry| text(entry.get("link")))
        }
        _ => None,
    }
}

/// Kept for callers that only have a TorBox key.
pub async fn resolve_torbox(key: &str, stream: &StreamItem) -> Result<String, String> {
    resolve(&Active { provider: Provider::Torbox, key: key.to_string() }, stream).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_core::stream::parse_streams;
    use serde_json::json;

    fn stream(payload: &str) -> StreamItem {
        parse_streams(payload, "Addon", "addon:1", None).unwrap().remove(0)
    }

    fn file(id: &str, name: &str, size: i64) -> ProviderFile {
        ProviderFile { id: id.into(), name: name.into(), size, playable: has_video_extension(name) }
    }

    #[test]
    fn providers_round_trip_and_use_account_credentials() {
        for provider in Provider::ALL {
            assert_eq!(Provider::parse(provider.id()), Some(provider));
            assert_eq!(Provider::parse(&provider.credential()), Some(provider));
        }
        assert_eq!(Provider::parse("real-debrid"), Some(Provider::RealDebrid));
        assert_eq!(Provider::parse("debrid_link"), Some(Provider::DebridLink));
        assert_eq!(Provider::DebridLink.credential(), "debrid:debridlink");
    }

    #[test]
    fn magnet_comes_from_the_addon_or_the_hash_with_trackers() {
        let with_magnet = stream(r#"{"streams":[{"url":"magnet:?xt=urn:btih:abc&dn=Movie"}]}"#);
        assert_eq!(resolve_info(&with_magnet).magnet.as_deref(), Some("magnet:?xt=urn:btih:abc&dn=Movie"));

        let by_hash = stream(
            r#"{"streams":[{"infoHash":"HASH","behaviorHints":{"filename":"Movie 2024.mkv"},"sources":["tracker:udp://tracker.test:80/announce","dht:xyz"]}]}"#,
        );
        let magnet = magnet_uri(&resolve_info(&by_hash)).unwrap();
        assert!(magnet.starts_with("magnet:?xt=urn:btih:HASH"));
        assert!(magnet.contains("&dn=Movie%202024.mkv"), "{magnet}");
        assert!(magnet.contains("&tr=udp%3A%2F%2Ftracker.test%3A80%2Fannounce"), "{magnet}");
        assert!(!magnet.contains("dht%3A"), "DHT entries are not trackers");
    }

    #[test]
    fn file_selection_follows_the_original_order() {
        let files = vec![
            file("1", "Show.S02E04.mkv", 900),
            file("2", "Show.S02E05.mkv", 800),
            file("3", "Show.S02E05.sample.mkv", 10),
            file("4", "notes.txt", 1),
        ];
        let info = ResolveInfo { season: Some(2), episode: Some(5), ..Default::default() };
        assert_eq!(select_file(&files, &info).unwrap().id, "2", "episode pattern");

        let info = ResolveInfo { filenames: vec!["Show.S02E04.mkv".into()], ..Default::default() };
        assert_eq!(select_file(&files, &info).unwrap().id, "1", "named file");

        let info = ResolveInfo { file_idx: Some(1), ..Default::default() };
        assert_eq!(select_file(&files, &info).unwrap().id, "2", "index when nothing else is named");

        assert_eq!(select_file(&files, &ResolveInfo::default()).unwrap().id, "1", "largest file");
        let missing = ResolveInfo { filenames: vec!["absent.mkv".into()], ..Default::default() };
        assert!(select_file(&files, &missing).is_none());
    }

    #[test]
    fn provider_payloads_are_understood() {
        let created = json!({"success":true,"data":{"torrent_id":42}});
        assert_eq!(id_text(data_value(&created).and_then(|d| d.get("torrent_id").or_else(|| d.get("id")))), Some("42".into()));
        let list = json!({"success":true,"data":{"files":[
            {"id":10,"name":"a.mkv","size":5,"mimetype":"video/x-matroska"},
            {"id":11,"name":"b.nfo","size":1,"mimetype":"text/plain"}]}});
        let files: Vec<ProviderFile> = as_objects(data_value(&list).unwrap().get("files")).into_iter().filter_map(ProviderFile::parse).collect();
        assert_eq!(files.len(), 2);
        assert!(files[0].playable && !files[1].playable);
    }

    #[test]
    fn saving_a_toggle_keeps_a_legacy_torbox_key() {
        let saved = Settings { enabled: true, torbox_api_key: "legacy-key".into(), ..Default::default() };
        let merged = merge_input(saved, &SettingsInput { enabled: false, ..Default::default() });
        assert!(!merged.enabled);
        assert_eq!(own_key(&merged, Provider::Torbox), "legacy-key");
        let cleared = merge_input(merged, &SettingsInput { enabled: false, keys: BTreeMap::from([("torbox".into(), Some(String::new()))]), ..Default::default() });
        assert!(own_key(&cleared, Provider::Torbox).is_empty());
    }

    #[test]
    fn the_account_key_comes_before_the_original_app() {
        let dir = std::env::temp_dir().join(format!("nuvio-debrid-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = nuvio_store::nextgen::AppStore::open(&dir).unwrap();
        let mut credentials = std::collections::BTreeMap::new();
        credentials.insert("debrid:realdebrid".to_string(), "rd-account-key".to_string());
        store.set_provider_credentials(2, credentials).unwrap();
        assert_eq!(key_for(&store, 2, Provider::RealDebrid).map(|(key, origin)| (key, origin)), Some(("rd-account-key".to_string(), KeyOrigin::Account)));
        assert!(key_for(&store, 3, Provider::RealDebrid).is_none(), "another profile does not see this account key");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
