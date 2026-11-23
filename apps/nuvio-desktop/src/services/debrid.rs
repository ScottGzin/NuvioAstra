//! Debrid services (`features/debrid` in the original): Settings → Services → Connected Services.
//!
//! Providers of the original: TorBox and Premiumize (device code), Real-Debrid (API key, hidden in
//! the providers list). This port covers **TorBox**, the provider configured on this computer, with
//! an API key: it resolves a `clientResolve` stream (`Comet`/`Torrentio`-style) by adding the magnet
//! to the account (`add_only_if_cached`), picking the file the addon pointed at, and asking for a
//! direct download link. Streams whose addon already resolved a URL are untouched.
//!
//! Premiumize needs the client id of its device flow (not in the public repository) and Real-Debrid
//! is hidden in the original's UI, so both stay out for now.

use nuvio_core::stream::StreamItem;
use nuvio_store::{default_legacy_data_dir, LegacyStore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

const TORBOX_BASE: &str = "https://api.torbox.app/v1/api";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub torbox_api_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        // `DebridSettings()` in the original: off until a provider is connected and enabled.
        Self { enabled: false, torbox_api_key: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub enabled: bool,
    pub torbox_connected: bool,
    /// The Nuvio account's key (`debrid:torbox` provider credential) is the one in use.
    pub torbox_from_account: bool,
    /// The original app's key is the one in use.
    pub torbox_from_original: bool,
    /// Account e-mail when TorBox answered; `None` means the key could not be checked now.
    pub account: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsInput {
    pub enabled: bool,
    /// `None` keeps the key; an empty string clears Astra's key.
    pub torbox_api_key: Option<String>,
}

impl Default for SettingsInput {
    fn default() -> Self {
        Self { enabled: false, torbox_api_key: None }
    }
}

fn legacy_values() -> Option<std::collections::HashMap<String, String>> {
    let dir = default_legacy_data_dir().ok()?;
    LegacyStore::open(dir).ok()?.read_store("nuvio_debrid_settings").ok()
}

fn stored(store: &nuvio_store::nextgen::AppStore) -> std::collections::BTreeMap<i64, Settings> {
    store.read("debrid-settings").ok().flatten().unwrap_or_default()
}

pub fn settings(store: &nuvio_store::nextgen::AppStore, profile: i64) -> Settings {
    stored(store).remove(&profile).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOrigin {
    Own,
    /// From the Nuvio account (`ProviderCredentialSync`: provider `debrid:torbox`).
    Account,
    Original,
}

/// Effective key: Astra's own, else the account's, else the original's
/// `debrid_torbox_api_key_<profile>`.
pub fn torbox_key(store: &nuvio_store::nextgen::AppStore, profile: i64) -> Option<(String, KeyOrigin)> {
    let own = settings(store, profile).torbox_api_key.trim().to_string();
    if !own.is_empty() {
        return Some((own, KeyOrigin::Own));
    }
    if let Some(account) = store.provider_key(profile, "debrid:torbox").filter(|key| !key.trim().is_empty()) {
        return Some((account, KeyOrigin::Account));
    }
    let legacy = legacy_values()?;
    let key = legacy.get(&format!("debrid_torbox_api_key_{profile}"))?.trim().to_string();
    (!key.is_empty()).then_some((key, KeyOrigin::Original))
}

/// Key used to resolve playable links: only when the profile enabled debrid and TorBox is connected.
pub fn access(store: &nuvio_store::nextgen::AppStore, profile: i64) -> Option<String> {
    let own = settings(store, profile);
    let enabled = if stored(store).contains_key(&profile) {
        own.enabled
    } else {
        legacy_values()
            .and_then(|v| v.get(&format!("debrid_enabled_{profile}")).map(|v| v.trim().eq_ignore_ascii_case("true")))
            .unwrap_or(false)
    };
    if !enabled {
        return None;
    }
    torbox_key(store, profile).map(|(key, _)| key)
}

pub async fn view(store: &nuvio_store::nextgen::AppStore, profile: i64) -> View {
    let key = torbox_key(store, profile);
    let account = match &key {
        Some((key, _)) => torbox_account(key).await,
        None => None,
    };
    View {
        enabled: if stored(store).contains_key(&profile) {
            settings(store, profile).enabled
        } else {
            legacy_values()
                .and_then(|v| v.get(&format!("debrid_enabled_{profile}")).map(|v| v.trim().eq_ignore_ascii_case("true")))
                .unwrap_or(false)
        },
        torbox_connected: key.is_some(),
        torbox_from_account: key.as_ref().is_some_and(|(_, origin)| *origin == KeyOrigin::Account),
        torbox_from_original: key.as_ref().is_some_and(|(_, origin)| *origin == KeyOrigin::Original),
        account,
    }
}

/// Validates a key before saving it (the provider card does the same).
pub async fn update(store: &nuvio_store::nextgen::AppStore, profile: i64, input: SettingsInput) -> Result<View, String> {
    let edited = input.torbox_api_key.clone();
    let mut next = settings(store, profile);
    if let Some(key) = input.torbox_api_key {
        let key = key.trim().to_string();
        if !key.is_empty() && torbox_account(&key).await.is_none() {
            return Err("O TorBox recusou esta chave da API.".into());
        }
        next.torbox_api_key = key;
    }
    next.enabled = input.enabled;
    let mut all = stored(store);
    all.insert(profile, next);
    store.write("debrid-settings", &all).map_err(|e| e.to_string())?;
    if let Some(key) = edited {
        // Same as `ProviderCredentialSync`: the key is also sent with the account's credentials.
        if let Ok(mut all) = store.provider_credentials() {
            let mut providers = all.remove(&profile).unwrap_or_default();
            if key.trim().is_empty() {
                providers.remove("debrid:torbox");
            } else {
                providers.insert("debrid:torbox".into(), key.trim().to_string());
            }
            let _ = store.set_provider_credentials(profile, providers);
        }
    }
    Ok(view(store, profile).await)
}

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent("NuvioAstra/0.1")
            .timeout(Duration::from_secs(30))
            .build()
            .expect("debrid http client")
    })
}

async fn get_json(url: &str, key: &str) -> Result<Value, String> {
    let response = http()
        .get(url)
        .bearer_auth(key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("TorBox HTTP {}", status.as_u16()));
    }
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

/// `TorboxApiClient.validateApiKey`: `/user/me`; the e-mail is shown on the provider card.
pub async fn torbox_account(key: &str) -> Option<String> {
    let body = get_json(&format!("{TORBOX_BASE}/user/me"), key.trim()).await.ok()?;
    if body.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    body.get("data")
        .and_then(|d| d.get("email"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
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

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
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
struct TorboxFile {
    id: i64,
    name: String,
    size: i64,
    mimetype: String,
}

impl TorboxFile {
    fn display_name(&self) -> &str {
        self.name.trim()
    }
    fn playable(&self) -> bool {
        self.mimetype.to_ascii_lowercase().starts_with("video/") || has_video_extension(self.display_name())
    }
}

fn has_video_extension(name: &str) -> bool {
    const EXTENSIONS: [&str; 10] = [".mp4", ".mkv", ".webm", ".avi", ".mov", ".m4v", ".ts", ".m2ts", ".wmv", ".flv"];
    let lower = name.to_ascii_lowercase();
    EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

fn parse_file(value: &Value) -> Option<TorboxFile> {
    Some(TorboxFile {
        id: value.get("id").and_then(Value::as_i64)?,
        name: value
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| value.get("short_name").and_then(Value::as_str))
            .or_else(|| value.get("absolute_path").and_then(Value::as_str))
            .unwrap_or_default()
            .trim()
            .to_string(),
        size: value.get("size").and_then(Value::as_i64).unwrap_or(0),
        mimetype: value.get("mimetype").and_then(Value::as_str).unwrap_or_default().to_string(),
    })
}

fn episode_matches(name: &str, season: i64, episode: i64) -> bool {
    let lower = name.to_ascii_lowercase();
    let patterns = [
        format!("s{season:02}e{episode:02}"),
        format!("s{season}e{episode}"),
        format!("{season}x{episode}"),
    ];
    patterns.iter().any(|pattern| {
        lower.match_indices(pattern).any(|(index, _)| {
            let rest = &lower[index + pattern.len()..];
            !rest.chars().next().is_some_and(|c| c.is_ascii_digit())
        })
    })
}

/// `selectDebridFile`: the file the addon named, else the episode it named, else its index, else the
/// largest playable file. `None` when the addon named a file/episode that is not in the torrent.
fn select_file<'a>(files: &'a [TorboxFile], info: &ResolveInfo) -> Option<&'a TorboxFile> {
    let playable: Vec<&TorboxFile> = files.iter().filter(|f| f.playable()).collect();
    if playable.is_empty() {
        return None;
    }
    let normalized = |name: &str| name.trim().replace('\\', "/");
    let names: Vec<String> = info.filenames.iter().map(|n| normalized(n)).collect();
    for name in &names {
        let basename = name.rsplit('/').next().unwrap_or(name);
        let matches: Vec<&TorboxFile> = playable
            .iter()
            .copied()
            .filter(|file| {
                let path = normalized(file.display_name());
                path == *name || (name.contains('/') && path.ends_with(&format!("/{name}"))) || path.rsplit('/').next() == Some(basename)
            })
            .collect();
        if !matches.is_empty() {
            return matches.first().copied();
        }
    }
    let season = info.season;
    let episode = info.episode;
    let episode_pattern = season.zip(episode);
    if let Some((season, episode)) = episode_pattern {
        let matches: Vec<&TorboxFile> = playable
            .iter()
            .copied()
            .filter(|file| episode_matches(file.display_name().rsplit('/').next().unwrap_or(file.display_name()), season, episode))
            .collect();
        if !matches.is_empty() {
            return matches.first().copied();
        }
    }
    if !names.is_empty() || episode_pattern.is_some() {
        return None;
    }
    if let Some(index) = info.file_idx {
        return files.get(index).filter(|f| f.playable());
    }
    playable.into_iter().max_by_key(|file| file.size)
}

fn multipart(fields: &[(&str, &str)]) -> (String, Vec<u8>) {
    let boundary = format!("NuvioAstra{}", fields.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>());
    let mut body: Vec<u8> = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn success(response: &Value) -> bool {
    response.get("success").and_then(Value::as_bool) != Some(false)
}

fn detail(response: &Value) -> String {
    ["detail", "error"]
        .iter()
        .find_map(|key| response.get(*key).and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

fn data_value(response: &Value) -> Option<&Value> {
    match response.get("data") {
        Some(Value::Array(items)) => items.first(),
        Some(value) => Some(value),
        None => None,
    }
}

fn parse_created_id(response: &Value) -> Option<i64> {
    let data = data_value(response)?;
    data.get("torrent_id").and_then(Value::as_i64).or_else(|| data.get("id").and_then(Value::as_i64))
}

fn parse_files(response: &Value) -> Vec<TorboxFile> {
    data_value(response)
        .and_then(|data| data.get("files"))
        .and_then(Value::as_array)
        .map(|files| files.iter().filter_map(parse_file).collect())
        .unwrap_or_default()
}

fn parse_link(response: &Value) -> Option<String> {
    data_value(response).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

async fn create_torrent(key: &str, magnet: &str) -> Result<i64, String> {
    let (content_type, body) = multipart(&[("magnet", magnet), ("add_only_if_cached", "true"), ("allow_zip", "false")]);
    let response = http()
        .post(format!("{TORBOX_BASE}/torrents/createtorrent"))
        .bearer_auth(key)
        .header("Content-Type", content_type)
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let payload = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("TorBox HTTP {}", status.as_u16()));
    }
    let body: Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
    if !success(&body) {
        let detail = detail(&body);
        return Err(if detail.to_ascii_lowercase().contains("cache") {
            "Esta fonte não está em cache no TorBox.".into()
        } else if detail.is_empty() {
            "O TorBox recusou esta fonte.".into()
        } else {
            detail
        });
    }
    parse_created_id(&body).ok_or_else(|| "O TorBox não devolveu o torrent.".to_string())
}

async fn wait_files(key: &str, torrent_id: i64) -> Result<Vec<TorboxFile>, String> {
    for attempt in 0..12 {
        let body = get_json(&format!("{TORBOX_BASE}/torrents/mylist?id={torrent_id}&bypass_cache=true"), key).await?;
        if !success(&body) {
            return Err(if detail(&body).is_empty() { "O TorBox não devolveu os ficheiros.".into() } else { detail(&body) });
        }
        let files = parse_files(&body);
        if !files.is_empty() {
            return Ok(files);
        }
        if attempt < 11 {
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
    }
    Err("O TorBox ainda não preparou os ficheiros desta fonte.".into())
}

async fn request_link(key: &str, torrent_id: i64, file_id: i64) -> Result<String, String> {
    let url = format!(
        "{TORBOX_BASE}/torrents/requestdl?token={}&torrent_id={torrent_id}&file_id={file_id}&zip_link=false&redirect=false&append_name=false",
        encode(key)
    );
    let body = get_json(&url, key).await?;
    if !success(&body) {
        return Err(if detail(&body).is_empty() { "O TorBox não devolveu a ligação.".into() } else { detail(&body) });
    }
    parse_link(&body).ok_or_else(|| "O TorBox não devolveu a ligação.".to_string())
}

/// Resolves a debrid stream into a direct URL (TorBox, cached torrents only).
pub async fn resolve_torbox(key: &str, stream: &StreamItem) -> Result<String, String> {
    // `shouldResolveToPlayableStream`: only the provider the addon named (TorBox here).
    let service = stream.client_resolve.as_ref().and_then(|r| r.get("service")).and_then(Value::as_str).unwrap_or("torbox");
    if !service.eq_ignore_ascii_case("torbox") {
        return Err(format!("O serviço de debrid “{service}” não é suportado nesta versão."));
    }
    let info = resolve_info(stream);
    let magnet = magnet_uri(&info).ok_or("Esta fonte não tem magnet para o TorBox.")?;
    let torrent_id = create_torrent(key, &magnet).await?;
    let files = wait_files(key, torrent_id).await?;
    let file = select_file(&files, &info).ok_or("O ficheiro desta fonte não está no TorBox.")?;
    request_link(key, torrent_id, file.id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_core::stream::parse_streams;
    use serde_json::json;

    fn stream(payload: &str) -> StreamItem {
        parse_streams(payload, "Addon", "addon:1", None).unwrap().remove(0)
    }

    fn file(id: i64, name: &str, size: i64, mime: &str) -> TorboxFile {
        TorboxFile { id, name: name.into(), size, mimetype: mime.into() }
    }

    #[test]
    fn magnet_comes_from_the_addon_or_the_hash_with_trackers() {
        let with_magnet = stream(r#"{"streams":[{"url":"magnet:?xt=urn:btih:abc&dn=Movie"}]}"#);
        assert_eq!(resolve_info(&with_magnet).magnet.as_deref(), Some("magnet:?xt=urn:btih:abc&dn=Movie"));

        let by_hash = stream(
            r#"{"streams":[{"infoHash":"HASH","behaviorHints":{"filename":"Movie 2024.mkv"},"sources":["tracker:udp://tracker.test:80/announce","dht:xyz"]}]}"#,
        );
        let info = resolve_info(&by_hash);
        let magnet = magnet_uri(&info).unwrap();
        assert!(magnet.starts_with("magnet:?xt=urn:btih:HASH"));
        assert!(magnet.contains("&dn=Movie%202024.mkv"), "{magnet}");
        assert!(magnet.contains("&tr=udp%3A%2F%2Ftracker.test%3A80%2Fannounce"), "{magnet}");
        assert!(!magnet.contains("dht%3A"), "DHT entries are not trackers");
    }

    #[test]
    fn client_resolve_magnet_and_season_are_read() {
        let stream = stream(
            r#"{"streams":[{"infoHash":"H","clientResolve":{"type":"debrid","service":"torbox","magnetUri":"magnet:?xt=urn:btih:H","season":2,"episode":5,"filename":"Show.S02E05.mkv","stream":{"raw":{"filename":"Show.S02E05.1080p.mkv"}}}}]}"#,
        );
        let info = resolve_info(&stream);
        assert_eq!(info.magnet.as_deref(), Some("magnet:?xt=urn:btih:H"));
        assert_eq!((info.season, info.episode), (Some(2), Some(5)));
        assert_eq!(info.filenames, ["Show.S02E05.mkv", "Show.S02E05.1080p.mkv"]);
    }

    #[test]
    fn file_selection_follows_the_original_order() {
        let files = vec![
            file(1, "Show.S02E04.mkv", 900, "video/x-matroska"),
            file(2, "Show.S02E05.mkv", 800, "video/x-matroska"),
            file(3, "Show.S02E05.sample.mkv", 10, "video/x-matroska"),
            file(4, "notes.txt", 1, "text/plain"),
        ];
        let info = ResolveInfo { season: Some(2), episode: Some(5), ..Default::default() };
        assert_eq!(select_file(&files, &info).unwrap().id, 2, "episode pattern");

        let info = ResolveInfo { filenames: vec!["Show.S02E04.mkv".into()], ..Default::default() };
        assert_eq!(select_file(&files, &info).unwrap().id, 1, "named file");

        let info = ResolveInfo { file_idx: Some(1), ..Default::default() };
        assert_eq!(select_file(&files, &info).unwrap().id, 2, "index when nothing else is named");

        let info = ResolveInfo::default();
        assert_eq!(select_file(&files, &info).unwrap().id, 1, "largest file by default");

        let missing = ResolveInfo { filenames: vec!["absent.mkv".into()], ..Default::default() };
        assert!(select_file(&files, &missing).is_none(), "a named file that is not there is not guessed");
    }

    #[test]
    fn the_account_key_comes_before_the_original_app() {
        let dir = std::env::temp_dir().join(format!("nuvio-debrid-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = nuvio_store::nextgen::AppStore::open(&dir).unwrap();
        let mut credentials = std::collections::BTreeMap::new();
        credentials.insert("debrid:torbox".to_string(), "account-key".to_string());
        store.set_provider_credentials(2, credentials).unwrap();
        assert_eq!(torbox_key(&store, 2).map(|(key, origin)| (key, origin)), Some(("account-key".to_string(), KeyOrigin::Account)));
        assert_ne!(torbox_key(&store, 3).map(|(_, origin)| origin), Some(KeyOrigin::Account), "another profile does not see this account key");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn torbox_payloads_are_understood() {
        let created = json!({"success":true,"data":{"torrent_id":42}});
        assert_eq!(parse_created_id(&created), Some(42));
        let created = json!({"success":true,"data":[{"id":7}]});
        assert_eq!(parse_created_id(&created), Some(7));
        let list = json!({"success":true,"data":{"files":[
            {"id":10,"name":"a.mkv","size":5,"mimetype":"video/x-matroska"},
            {"id":11,"name":"b.nfo","size":1,"mimetype":"text/plain"}]}});
        let files = parse_files(&list);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].id, 10);
        assert!(files[0].playable());
        assert!(!files[1].playable());
        assert_eq!(parse_link(&json!({"success":true,"data":"https://cdn/x"})).as_deref(), Some("https://cdn/x"));
        assert_eq!(parse_link(&json!({"success":false,"detail":"No link"})), None);
    }
}
