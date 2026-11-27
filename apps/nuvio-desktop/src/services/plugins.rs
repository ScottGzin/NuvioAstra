//! Scraper plugins (`PluginRepository` / `PluginRuntime` of the original, ported from aelrased's
//! proposal): repositories of `manifest.json` scraper lists, kept per profile with the downloaded
//! scripts, and run in the `nuvio-plugins` sandbox (a fresh QuickJS engine per call, no access to
//! this computer or the local network, memory and time limits).
//!
//! The scrapers get the TMDB id of the title, like in the original: titles from the addons (IMDb
//! ids) are converted through TMDB when it is set up (Settings → Integrações → TMDB). Their streams
//! become one more group per scraper in the sources panel, playable like any addon stream.

use nuvio_core::client::AddonClient;
use nuvio_core::stream::{StreamBehaviorHints, StreamItem, StreamProxyHeaders};
use nuvio_plugins::manifest::{parse_manifest, repository_item, resolve_code_url, scraper_from_manifest};
use nuvio_plugins::model::{normalize_type, PluginRepositoryItem, PluginRuntimeResult, PluginScraper, StoredPluginsState};
use nuvio_plugins::runtime::{get_streams, validate_module, PluginCall};
use nuvio_store::nextgen::AppStore;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STORE_NAME: &str = "plugins";
/// One scraper's time: the sources panel shows the others meanwhile.
const SCRAPER_TIMEOUT: Duration = Duration::from_secs(20);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn all(store: &AppStore) -> BTreeMap<i64, StoredPluginsState> {
    store.read(STORE_NAME).ok().flatten().unwrap_or_default()
}

pub fn state_of(store: &AppStore, profile: i64) -> StoredPluginsState {
    all(store).remove(&profile).unwrap_or_default()
}

fn save(store: &AppStore, profile: i64, state: StoredPluginsState) -> Result<(), String> {
    let mut all = all(store);
    all.insert(profile, state);
    store.write(STORE_NAME, &all).map_err(|e| e.to_string())
}

fn normalize_url(url: &str) -> String {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") { url.to_string() } else { format!("https://{url}") }
}

/// Opaque handle: the manifest address may carry a token and never goes to the interface.
pub fn repo_key(url: &str) -> String {
    nuvio_sync::crypto::sha256_hex(&normalize_url(url))[..12].to_string()
}

/// The site of a repository (shown instead of its address).
fn origin(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryView {
    pub key: String,
    pub name: String,
    pub origin: String,
    pub version: String,
    pub description: Option<String>,
    pub scraper_count: usize,
    pub last_updated: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScraperView {
    pub key: String,
    pub repository_key: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub logo: Option<String>,
    pub supported_types: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginsView {
    pub enabled: bool,
    pub repositories: Vec<RepositoryView>,
    pub scrapers: Vec<ScraperView>,
    /// Scrapers of the last add/refresh that could not be installed (name: reason).
    pub warning: Option<String>,
}

fn scraper_key(s: &PluginScraper) -> String {
    format!("{}:{}", repo_key(&s.repository_url), s.id)
}

pub fn view(store: &AppStore, profile: i64, warning: Option<String>) -> PluginsView {
    let state = state_of(store, profile);
    PluginsView {
        enabled: state.plugins_enabled,
        repositories: state
            .repositories
            .iter()
            .map(|r| RepositoryView {
                key: repo_key(&r.manifest_url),
                name: r.name.clone(),
                origin: origin(&r.manifest_url),
                version: r.version.clone().unwrap_or_default(),
                description: r.description.clone(),
                scraper_count: state.scrapers.iter().filter(|s| s.repository_url == r.manifest_url).count(),
                last_updated: r.last_updated,
            })
            .collect(),
        scrapers: state
            .scrapers
            .iter()
            .map(|s| ScraperView {
                key: scraper_key(s),
                repository_key: repo_key(&s.repository_url),
                name: s.name.clone(),
                version: s.version.clone(),
                description: Some(s.description.clone()).filter(|d| !d.trim().is_empty()),
                logo: s.logo.clone(),
                supported_types: s.supported_types.clone(),
                enabled: s.enabled,
            })
            .collect(),
        warning,
    }
}

/// Downloads a repository and every script, validating each one in the sandbox before keeping it.
async fn fetch(client: &AddonClient, url: &str) -> Result<(PluginRepositoryItem, Vec<PluginScraper>, Option<String>), String> {
    let payload = client.get_text(url, true).await.map_err(|_| "Não foi possível ler o repositório (manifest.json).".to_string())?;
    let manifest = parse_manifest(&payload).map_err(|_| "Isso não é um repositório de plugins do Nuvio.".to_string())?;
    let repository = repository_item(url.to_string(), &manifest, now_ms());
    let mut scrapers = Vec::new();
    let mut errors = Vec::new();
    for entry in &manifest.scrapers {
        let name = if entry.name.trim().is_empty() { entry.id.clone() } else { entry.name.clone() };
        let Ok(code_url) = resolve_code_url(url, &entry.filename) else {
            errors.push(format!("{name}: endereço do script inválido"));
            continue;
        };
        let Ok(code) = client.get_text(&code_url, true).await else {
            errors.push(format!("{name}: script não baixou"));
            continue;
        };
        if !matches!(validate_module(&code).await, Ok(true)) {
            errors.push(format!("{name}: script inválido"));
            continue;
        }
        match scraper_from_manifest(url, entry) {
            Ok(mut scraper) => {
                scraper.code = code.to_string();
                scrapers.push(scraper);
            }
            Err(_) => errors.push(format!("{name}: dados incompletos")),
        }
    }
    Ok((repository, scrapers, (!errors.is_empty()).then(|| errors.join(" · "))))
}

pub async fn add(store: &AppStore, client: &AddonClient, profile: i64, url: &str) -> Result<PluginsView, String> {
    let url = normalize_url(url);
    let mut state = state_of(store, profile);
    if state.repositories.iter().any(|r| r.manifest_url == url) {
        return Err("Esse repositório já foi adicionado.".into());
    }
    let (repository, scrapers, warning) = fetch(client, &url).await?;
    if scrapers.is_empty() {
        return Err(warning.map_or("O repositório não tem plugins.".to_string(), |w| format!("Nenhum plugin pôde ser instalado: {w}")));
    }
    state.repositories.push(repository);
    state.scrapers.extend(scrapers);
    // The first repository turns plugins on.
    if state.repositories.len() == 1 {
        state.plugins_enabled = true;
    }
    save(store, profile, state)?;
    Ok(view(store, profile, warning))
}

/// Downloads a repository again; a scraper keeps its on/off choice.
pub async fn refresh(store: &AppStore, client: &AddonClient, profile: i64, key: &str) -> Result<PluginsView, String> {
    let mut state = state_of(store, profile);
    let old = state.repositories.iter().find(|r| repo_key(&r.manifest_url) == key).cloned().ok_or("Repositório não encontrado.")?;
    let (repository, scrapers, warning) = fetch(client, &old.manifest_url).await?;
    let fresh: Vec<PluginScraper> = scrapers
        .into_iter()
        .map(|mut s| {
            if let Some(prev) = state.scrapers.iter().find(|p| p.repository_url == old.manifest_url && p.id == s.id) {
                s.enabled = prev.enabled;
            }
            s
        })
        .collect();
    state.scrapers.retain(|s| s.repository_url != old.manifest_url);
    state.scrapers.extend(fresh);
    if let Some(slot) = state.repositories.iter_mut().find(|r| r.manifest_url == old.manifest_url) {
        *slot = repository;
    }
    save(store, profile, state)?;
    Ok(view(store, profile, warning))
}

pub fn set_enabled(store: &AppStore, profile: i64, enabled: bool) -> Result<PluginsView, String> {
    let mut state = state_of(store, profile);
    state.plugins_enabled = enabled;
    save(store, profile, state)?;
    Ok(view(store, profile, None))
}

pub fn set_scraper_enabled(store: &AppStore, profile: i64, key: &str, enabled: bool) -> Result<PluginsView, String> {
    let mut state = state_of(store, profile);
    let scraper = state.scrapers.iter_mut().find(|s| scraper_key(s) == key).ok_or("Plugin não encontrado.")?;
    scraper.enabled = enabled;
    save(store, profile, state)?;
    Ok(view(store, profile, None))
}

pub fn remove(store: &AppStore, profile: i64, key: &str) -> Result<PluginsView, String> {
    let mut state = state_of(store, profile);
    let repository = state.repositories.iter().find(|r| repo_key(&r.manifest_url) == key).cloned().ok_or("Repositório não encontrado.")?;
    state.repositories.retain(|r| r.manifest_url != repository.manifest_url);
    state.scrapers.retain(|s| s.repository_url != repository.manifest_url);
    save(store, profile, state)?;
    Ok(view(store, profile, None))
}

/// `tt123:2:5` / `tmdb:123:2:5` → (title id, season, episode).
pub fn split_video_id(id: &str) -> (String, Option<i64>, Option<i64>) {
    let (head, rest) = match id.strip_prefix("tmdb:") {
        Some(rest) => ("tmdb:", rest),
        None => ("", id),
    };
    let mut parts = rest.split(':');
    let base = format!("{head}{}", parts.next().unwrap_or_default().trim());
    let numbers: Vec<i64> = parts.filter_map(|p| p.trim().parse().ok()).collect();
    match numbers.as_slice() {
        [s, e, ..] => (base, Some(*s), Some(*e)),
        _ => (base, None, None),
    }
}

/// The scrapers to run for a title type (none when plugins are off).
pub fn runnable(store: &AppStore, profile: i64, content_type: &str) -> Vec<PluginScraper> {
    let state = state_of(store, profile);
    if !state.plugins_enabled {
        return Vec::new();
    }
    let kind = normalize_type(content_type);
    state.scrapers.into_iter().filter(|s| s.enabled && s.supports_type(&kind)).collect()
}

/// Runs one scraper; a failure or a timeout is simply no sources.
pub async fn run(scraper: &PluginScraper, tmdb_id: &str, content_type: &str, season: Option<i64>, episode: Option<i64>, tmdb_key: &str) -> Vec<StreamItem> {
    let call = PluginCall {
        tmdb_id: tmdb_id.to_string(),
        media_type: normalize_type(content_type),
        season,
        episode,
        scraper_id: scraper.id.clone(),
        settings: serde_json::json!({}),
        tmdb_api_key: tmdb_key.to_string(),
    };
    match tokio::time::timeout(SCRAPER_TIMEOUT, get_streams(&scraper.code, &call)).await {
        Ok(Ok(output)) => output.results.into_iter().filter(|r| r.url.starts_with("http") || r.url.starts_with("magnet:") || r.info_hash.is_some()).map(|r| stream_item(scraper, r)).collect(),
        _ => Vec::new(),
    }
}

/// A plugin result as an addon stream (headers, quality and size as the sources panel shows them).
fn stream_item(scraper: &PluginScraper, r: PluginRuntimeResult) -> StreamItem {
    let headers = r.headers.clone().filter(|h| !h.is_empty()).map(|h| StreamProxyHeaders { request: Some(h.into_iter().collect()), response: None });
    let quality = r.quality.clone().filter(|q| !q.trim().is_empty());
    let name = match &quality {
        Some(q) if !r.name.as_deref().unwrap_or("").contains(q.as_str()) => Some(format!("{}\n{q}", r.name.clone().unwrap_or_else(|| scraper.name.clone()))),
        _ => r.name.clone().or_else(|| Some(scraper.name.clone())),
    };
    StreamItem {
        name,
        title: Some(r.title.clone()),
        description: Some(r.title.clone()),
        url: (!r.url.is_empty() && !r.url.starts_with("magnet:")).then(|| r.url.clone()),
        info_hash: r.info_hash.clone(),
        addon_name: scraper.name.clone(),
        addon_id: format!("plugin:{}", scraper.id),
        addon_logo: scraper.logo.clone(),
        stream_type: r.stream_type.clone(),
        behavior_hints: StreamBehaviorHints { proxy_headers: headers, not_web_ready: true, ..Default::default() },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ids_split() {
        assert_eq!(split_video_id("tt0111161"), ("tt0111161".into(), None, None));
        assert_eq!(split_video_id("tt1:2:5"), ("tt1".into(), Some(2), Some(5)));
        assert_eq!(split_video_id("tmdb:1396:2:5"), ("tmdb:1396".into(), Some(2), Some(5)));
    }

    #[test]
    fn repository_keys_are_stable_and_opaque() {
        assert_eq!(repo_key("raw.githubusercontent.com/x/manifest.json"), repo_key("https://raw.githubusercontent.com/x/manifest.json"));
        assert_ne!(repo_key("https://a.test/manifest.json"), repo_key("https://b.test/manifest.json"));
        assert_eq!(origin("https://raw.githubusercontent.com/x/manifest.json?token=abc"), "raw.githubusercontent.com");
    }
}
