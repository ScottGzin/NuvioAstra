//! Nuvio scraper plugins: repositories of `manifest.json` scraper lists, stored per profile and run
//! in the sandbox implemented by `nuvio-plugins`.
//!
//! This is the Nuvio Desktop plugin format (not VyseTV's `repo.json`/Vega formats). Repositories are
//! fetched and validated on install and on manual refresh; the downloaded script is validated before
//! it is stored, so a broken script can never be run. Plugin scripts are kept locally, like in the
//! original, and are not synchronized with the account (see docs/ACCOUNT_SYNC.md).

use nuvio_core::client::AddonClient;
use nuvio_core::stream::{StreamBehaviorHints, StreamItem, StreamProxyHeaders};
use nuvio_plugins::manifest::{parse_manifest, repository_item, resolve_code_url, scraper_from_manifest};
use nuvio_plugins::model::StoredPluginsState;
use nuvio_plugins::runtime::{get_streams, validate_module, PluginCall};
use nuvio_sync::crypto::sha256_hex;
use nuvio_store::nextgen::AppStore;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STORE_NAME: &str = "plugins";
/// One scraper gets this much wall clock. The Sources screen shows the other answers meanwhile, and
/// a scraper that hangs (or burns its CPU deadline) must not hold the whole answer past this.
const SCRAPER_TIMEOUT: Duration = Duration::from_secs(20);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn all(store: &AppStore) -> BTreeMap<i64, StoredPluginsState> {
    store.read(STORE_NAME).ok().flatten().unwrap_or_default()
}

fn state_of(store: &AppStore, profile: i64) -> StoredPluginsState {
    all(store).remove(&profile).unwrap_or_default()
}

fn save(store: &AppStore, profile: i64, state: StoredPluginsState) -> Result<(), String> {
    let mut all = all(store);
    all.insert(profile, state);
    store.write(STORE_NAME, &all).map_err(|e| e.to_string())
}

/// Opaque handle: the manifest URL may carry a token and is never sent to the UI.
pub fn repo_key(url: &str) -> String {
    sha256_hex(&normalize_url(url))[..12].to_string()
}

fn normalize_url(url: &str) -> String {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{url}")
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryView {
    pub key: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub scraper_count: usize,
    pub last_updated: i64,
    pub refresh_due: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScraperView {
    pub key: String,
    pub repository_key: String,
    pub name: String,
    pub version: String,
    pub supported_types: Vec<String>,
    pub enabled: bool,
    pub manifest_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginsView {
    pub profile: i64,
    pub enabled: bool,
    pub repositories: Vec<RepositoryView>,
    pub scrapers: Vec<ScraperView>,
    pub error: Option<String>,
}

fn view_of(store: &AppStore, profile: i64, error: Option<String>) -> PluginsView {
    let state = state_of(store, profile);
    let now = now_ms();
    PluginsView {
        profile,
        enabled: state.plugins_enabled,
        repositories: state
            .repositories
            .iter()
            .map(|r| RepositoryView {
                key: repo_key(&r.manifest_url),
                name: r.name.clone(),
                version: r.version.clone().unwrap_or_default(),
                description: r.description.clone(),
                scraper_count: state.scrapers.iter().filter(|s| s.repository_url == r.manifest_url).count(),
                last_updated: r.last_updated,
                refresh_due: nuvio_plugins::model::is_refresh_due(r.last_updated, now),
            })
            .collect(),
        scrapers: state
            .scrapers
            .iter()
            .map(|s| ScraperView {
                key: format!("{}:{}", repo_key(&s.repository_url), s.id),
                repository_key: repo_key(&s.repository_url),
                name: s.name.clone(),
                version: s.version.clone(),
                supported_types: s.supported_types.clone(),
                enabled: s.enabled,
                manifest_enabled: s.manifest_enabled,
            })
            .collect(),
        error,
    }
}

pub struct Manager {
    dir: PathBuf,
    client: AddonClient,
}

impl Manager {
    pub fn new(dir: PathBuf, client: AddonClient) -> Self {
        Self { dir, client }
    }

    fn store(&self) -> Result<AppStore, String> {
        AppStore::open(&self.dir).map_err(|e| e.to_string())
    }

    pub fn view(&self, profile: i64) -> PluginsView {
        match self.store() {
            Ok(store) => view_of(&store, profile, None),
            Err(_) => PluginsView {
                profile,
                enabled: false,
                repositories: Vec::new(),
                scrapers: Vec::new(),
                error: Some("Não foi possível ler o estado dos plugins.".into()),
            },
        }
    }

    /// Adds a repository: validates its manifest and every script before anything is stored.
    pub async fn add(&self, profile: i64, url: &str) -> Result<PluginsView, String> {
        let url = normalize_url(url);
        let store = self.store()?;
        let mut state = state_of(&store, profile);
        if state.repositories.iter().any(|r| r.manifest_url == url) {
            return Ok(view_of(&store, profile, Some("Repositório já adicionado.".into())));
        }
        let (mut repository, scrapers, error) = self.fetch(&url).await?;
        let scrapers = scrapers.unwrap_or_default();
        repository.last_updated = now_ms();
        state.repositories.push(repository);
        state.scrapers.extend(scrapers);
        save(&store, profile, state)?;
        Ok(view_of(&store, profile, error))
    }

    /// Re-fetches one repository; only scrapers whose `version` changed are downloaded again.
    pub async fn refresh(&self, profile: i64, key: &str) -> Result<PluginsView, String> {
        let store = self.store()?;
        let mut state = state_of(&store, profile);
        let old = state
            .repositories
            .iter()
            .find(|r| repo_key(&r.manifest_url) == key)
            .cloned()
            .ok_or("Repositório não encontrado.")?;
        let (mut repository, scrapers, error) = self.fetch(&old.manifest_url).await?;
        let scrapers = scrapers.unwrap_or_default();
        let mut fresh_all: Vec<nuvio_plugins::model::PluginScraper> = Vec::new();
        for scraper in scrapers {
            let previous = state.scrapers.iter().find(|s| s.repository_url == old.manifest_url && s.id == scraper.id);
            let mut item = scraper;
            if let Some(prev) = previous {
                if prev.version == item.version {
                    item.code = prev.code.clone();
                    item.enabled = prev.enabled;
                }
            }
            fresh_all.push(item);
        }
        state.scrapers.retain(|s| s.repository_url != old.manifest_url);
        state.scrapers.extend(fresh_all);
        repository.last_updated = now_ms();
        if let Some(slot) = state.repositories.iter_mut().find(|r| r.manifest_url == old.manifest_url) {
            *slot = repository;
        }
        save(&store, profile, state)?;
        Ok(view_of(&store, profile, error))
    }

    async fn fetch(&self, url: &str) -> Result<(nuvio_plugins::model::PluginRepositoryItem, Option<Vec<nuvio_plugins::model::PluginScraper>>, Option<String>), String> {
        let payload = self.client.get_text(url, true).await.map_err(|e| format!("Não foi possível ler o manifesto: {}", e))?;
        let manifest = parse_manifest(&payload).map_err(|e| e.to_string())?;
        let repository = repository_item(url.to_string(), &manifest, now_ms());
        let mut scrapers = Vec::new();
        let mut errors = Vec::new();
        for entry in &manifest.scrapers {
            let code_url = match resolve_code_url(url, &entry.filename) {
                Ok(url) => url,
                Err(e) => {
                    errors.push(format!("{}: {}", entry.id, e));
                    continue;
                }
            };
            let code = match self.client.get_text(&code_url, true).await {
                Ok(code) => code.to_string(),
                Err(e) => {
                    errors.push(format!("{}: {}", entry.id, e));
                    continue;
                }
            };
            if let Err(e) = validate_module(&code).await {
                errors.push(format!("{}: {}", entry.id, e));
                continue;
            }
            match scraper_from_manifest(url, entry) {
                Ok(mut scraper) => {
                    scraper.code = code;
                    scrapers.push(scraper);
                }
                Err(e) => errors.push(format!("{}: {}", entry.id, e)),
            }
        }
        let error = (!errors.is_empty()).then(|| errors.join(" · "));
        Ok((repository, Some(scrapers), error))
    }

    pub fn set_enabled(&self, profile: i64, enabled: bool) -> Result<PluginsView, String> {
        let store = self.store()?;
        let mut state = state_of(&store, profile);
        state.plugins_enabled = enabled;
        save(&store, profile, state)?;
        Ok(view_of(&store, profile, None))
    }

    pub fn set_scraper_enabled(&self, profile: i64, key: &str, enabled: bool) -> Result<PluginsView, String> {
        let store = self.store()?;
        let mut state = state_of(&store, profile);
        let Some(scraper) = state.scrapers.iter_mut().find(|s| format!("{}:{}", repo_key(&s.repository_url), s.id) == key) else {
            return Err("Scraper não encontrado.".into());
        };
        scraper.enabled = enabled;
        save(&store, profile, state)?;
        Ok(view_of(&store, profile, None))
    }

    pub fn remove(&self, profile: i64, key: &str) -> Result<PluginsView, String> {
        let store = self.store()?;
        let mut state = state_of(&store, profile);
        let Some(repository) = state.repositories.iter().find(|r| repo_key(&r.manifest_url) == key).cloned() else {
            return Err("Repositório não encontrado.".into());
        };
        state.repositories.retain(|r| r.manifest_url != repository.manifest_url);
        state.scrapers.retain(|s| s.repository_url != repository.manifest_url);
        save(&store, profile, state)?;
        Ok(view_of(&store, profile, None))
    }

    /// Runs the enabled scrapers for a title and returns streams as ordinary addon streams, so the
    /// player, headers and the relay work exactly the same way.
    pub async fn search(&self, profile: i64, content_type: &str, id: &str) -> Vec<(String, Vec<StreamItem>)> {
        self.search_streamed(profile, content_type, id, |_, _, _| {})
            .await
            .into_iter()
            .map(|(_, name, streams)| (name, streams))
            .collect()
    }

    /// Same scrapers as `search`, all at once: every scraper's streams are handed to `emit` as soon
    /// as it answers (and returned in the configured order), so the fastest scraper shows its
    /// sources without waiting for the slowest one.
    pub async fn search_streamed<F>(&self, profile: i64, content_type: &str, id: &str, mut emit: F) -> Vec<(usize, String, Vec<StreamItem>)>
    where
        F: FnMut(usize, &str, &[StreamItem]),
    {
        let store = match self.store() {
            Ok(store) => store,
            Err(_) => return Vec::new(),
        };
        let state = state_of(&store, profile);
        if !state.plugins_enabled || state.scrapers.is_empty() {
            return Vec::new();
        }
        let (tmdb_id, season, episode) = parse_content_id(id);
        if tmdb_id.is_empty() {
            return Vec::new();
        }
        let scrapers: Vec<_> = state.scrapers.into_iter().filter(|s| s.enabled && s.supports_type(content_type)).collect();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for (index, scraper) in scrapers.iter().enumerate() {
            let tx = tx.clone();
            let call = PluginCall {
                tmdb_id: tmdb_id.clone(),
                media_type: content_type.to_string(),
                season,
                episode,
                scraper_id: scraper.id.clone(),
                settings: serde_json::json!({}),
                tmdb_api_key: String::new(),
            };
            let code = scraper.code.clone();
            let name = scraper.name.clone();
            let scraper_id = scraper.id.clone();
            let logo = scraper.logo.clone();
            tauri::async_runtime::spawn(async move {
                let streams = match tokio::time::timeout(SCRAPER_TIMEOUT, get_streams(&code, &call)).await {
                    Ok(Ok(run)) => run.results.into_iter().map(|r| plugin_stream(&name, &scraper_id, logo.clone(), r)).collect(),
                    // A scraper that fails or times out simply has nothing to offer.
                    _ => Vec::new(),
                };
                let _ = tx.send((index, name, streams));
            });
        }
        drop(tx);
        let mut found = Vec::with_capacity(scrapers.len());
        while let Some((index, name, streams)) = rx.recv().await {
            // The UI shows only groups with sources: an empty answer is not announced.
            if !streams.is_empty() {
                emit(index, &name, &streams);
            }
            found.push((index, name, streams));
        }
        found.sort_by_key(|(index, _, _)| *index);
        found
    }
}

/// `tmdb:123` / `tt123:2:5` → (id, season, episode). Only numeric TMDB ids can be passed to a
/// scraper (the plugins call TMDB themselves), which is what the Stremio details ids use here.
pub fn parse_content_id(id: &str) -> (String, Option<i64>, Option<i64>) {
    let body = id.strip_prefix("tmdb:").unwrap_or(id);
    let mut parts = body.split(':');
    let raw = parts.next().unwrap_or_default().trim();
    let numbers: Vec<i64> = parts.filter_map(|p| p.trim().parse().ok()).collect();
    let (season, episode) = match numbers.as_slice() {
        [s, e, ..] => (Some(*s), Some(*e)),
        [s] => (Some(*s), None),
        _ => (None, None),
    };
    let tmdb = if !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()) { raw.to_string() } else { String::new() };
    (tmdb, season, episode)
}

fn plugin_stream(addon: &str, id: &str, logo: Option<String>, result: nuvio_plugins::model::PluginRuntimeResult) -> StreamItem {
    let headers = result.headers.map(|headers| StreamProxyHeaders {
        request: Some(headers.into_iter().collect()),
        response: None,
    });
    StreamItem {
        name: result.name.clone().or_else(|| Some(result.title.clone())),
        title: Some(result.title.clone()),
        description: Some(result.title.clone()),
        url: Some(result.url),
        info_hash: result.info_hash,
        addon_name: format!("{addon} · Plugins"),
        addon_id: format!("plugin:{id}"),
        addon_logo: logo,
        stream_type: result.stream_type,
        behavior_hints: StreamBehaviorHints {
            filename: result.name.clone(),
            proxy_headers: headers,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_ids_are_parsed_for_plugins() {
        assert_eq!(parse_content_id("tmdb:550"), ("550".into(), None, None));
        assert_eq!(parse_content_id("tmdb:1396:2:5"), ("1396".into(), Some(2), Some(5)));
        assert_eq!(parse_content_id("tt0111161"), (String::new(), None, None));
    }

    #[test]
    fn repository_keys_are_stable_and_opaque() {
        assert_eq!(repo_key("raw.githubusercontent.com/x/manifest.json"), repo_key("https://raw.githubusercontent.com/x/manifest.json"));
        assert_ne!(repo_key("https://a.test/manifest.json"), repo_key("https://b.test/manifest.json"));
    }

    #[tokio::test]
    async fn a_repository_is_downloaded_validated_and_persisted() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let manifest = serde_json::json!({
            "name": "Local test repo", "version": "1.0",
            "scrapers": [{
                "id": "local", "name": "Local", "version": "1", "filename": "local.js",
                "supportedTypes": ["movie", "tv"], "enabled": true
            }]
        })
        .to_string();
        let script = "module.exports.getStreams = function () { return []; };";
        std::thread::spawn(move || {
            for body in [manifest.clone(), script.to_string()] {
                let (mut socket, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 4096];
                let _ = socket.read(&mut buffer).unwrap();
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).unwrap();
            }
        });

        let dir = std::env::temp_dir().join(format!("nuvio-plugins-manager-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let manager = Manager::new(dir.clone(), AddonClient::new().unwrap());
        let view = manager.add(1, &format!("http://127.0.0.1:{port}/manifest.json")).await.unwrap();
        assert_eq!(view.repositories.len(), 1);
        assert_eq!(view.repositories[0].name, "Local test repo");
        assert_eq!(view.scrapers.len(), 1);
        assert_eq!(view.scrapers[0].name, "Local");

        // The downloaded code is stored, so a new manager (new process) can run it offline.
        let reopened = Manager::new(dir.clone(), AddonClient::new().unwrap());
        let saved = reopened.view(1);
        assert_eq!(saved.scrapers.len(), 1);
        assert!(saved.scrapers[0].enabled);
        assert!(reopened.search(1, "movie", "tmdb:550").await.len() == 1, "the scraper must be runnable after a restart");

        let removed = reopened.remove(1, &view.repositories[0].key).unwrap();
        assert!(removed.repositories.is_empty() && removed.scrapers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
