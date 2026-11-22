//! Addons of the active profile, loaded once and shared by every service.
//!
//! Same rules as the original (`AddonRepository.initialize`): the profile's own list, or profile 1's
//! for a secondary profile with `uses_primary_addons`; enabled addons only; user order kept.
//! Source: NextGen's store (`profiles.json`, `addons-<p>.json`, imported once from the original).
//!
//! Manifests are fetched concurrently and cached by URL. `invalidate` (profile switch, addon
//! change) only bumps an epoch: the next `loaded` rebuilds the list from the store and fetches just
//! the manifests it does not have yet (new, re-enabled or previously failed addons), so no change
//! makes every addon be asked again.

use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_store::nextgen::AppStore;
use nuvio_store::profiles::AddonSummary;
use nuvio_store::{default_legacy_data_dir, LegacyStore};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    pub index: i64,
    pub name: String,
    pub avatar_id: Option<String>,
    pub avatar_color_hex: String,
    pub uses_primary_addons: bool,
    /// Protected by a PIN set with the account.
    pub pin_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedAddons {
    pub profile: i64,
    /// Effective profile whose addons are used (1 when `uses_primary_addons`).
    pub addon_profile: i64,
    /// Enabled manifests that loaded, in the user's order.
    #[serde(skip)]
    pub manifests: Vec<AddonManifest>,
    /// Manifest URL → load error (enabled addons that did not load; text without URL).
    #[serde(skip)]
    pub errors: HashMap<String, String>,
    pub enabled: usize,
    pub failed: usize,
}

/// The original app's data, read-only (migration only).
pub fn legacy_store() -> Result<LegacyStore, String> {
    LegacyStore::open(default_legacy_data_dir().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Active profile and the list, from NextGen's `profiles.json`.
pub fn profiles(store: &AppStore) -> Result<(i64, Vec<ProfileInfo>), String> {
    let all = store.profiles().map_err(|e| e.to_string())?;
    Ok((
        all.active,
        all.profiles
            .iter()
            .map(|p| ProfileInfo {
                index: p.index,
                name: p.name.clone(),
                avatar_id: p.avatar_id.clone(),
                avatar_color_hex: p.avatar_color_hex.clone(),
                uses_primary_addons: p.uses_primary_addons,
                pin_enabled: p.pin_enabled,
            })
            .collect(),
    ))
}

pub fn summary_of(m: &AddonManifest) -> AddonSummary {
    AddonSummary {
        id: m.id.clone(),
        name: m.name.clone(),
        description: m.description.clone(),
        version: m.version.clone(),
        logo: m.logo_url.clone(),
        types: m.types.clone(),
        catalogs: m.catalogs.len(),
    }
}

struct Cached {
    epoch: u64,
    loaded: Arc<LoadedAddons>,
    /// Every manifest loaded so far, by URL (kept across rebuilds).
    by_url: HashMap<String, AddonManifest>,
}

pub struct AddonContext {
    pub client: AddonClient,
    dir: PathBuf,
    epoch: AtomicU64,
    cache: Mutex<Option<Cached>>,
}

impl AddonContext {
    pub fn new(client: AddonClient, dir: PathBuf) -> Self {
        Self { client, dir, epoch: AtomicU64::new(0), cache: Mutex::new(None) }
    }

    /// Context with manifests already loaded (contract tests against a local addon server).
    #[cfg(test)]
    pub fn preloaded(client: AddonClient, loaded: LoadedAddons) -> Self {
        let by_url = loaded.manifests.iter().map(|m| (m.transport_url.clone(), m.clone())).collect();
        Self { client, dir: PathBuf::new(), epoch: AtomicU64::new(0), cache: Mutex::new(Some(Cached { epoch: 0, loaded: Arc::new(loaded), by_url })) }
    }

    /// Marks the list as stale (profile switch, addon installed/removed/enabled/moved). Never waits.
    pub fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Addons of the active profile; rebuilt after `invalidate`, fetching only missing manifests.
    pub async fn loaded(&self) -> Result<Arc<LoadedAddons>, String> {
        let mut cache = self.cache.lock().await;
        let epoch = self.epoch();
        if let Some(c) = cache.as_ref().filter(|c| c.epoch == epoch) {
            return Ok(c.loaded.clone());
        }
        let mut by_url = cache.take().map(|c| c.by_url).unwrap_or_default();
        let result = self.build(&mut by_url).await;
        match result {
            Ok(loaded) => {
                let loaded = Arc::new(loaded);
                *cache = Some(Cached { epoch, loaded: loaded.clone(), by_url });
                Ok(loaded)
            }
            Err(e) => Err(e),
        }
    }

    /// Fetches one addon's manifest again, bypassing caches (manual "Atualizar"), and keeps it.
    pub async fn refresh_one(&self, manifest_url: &str) -> Result<AddonManifest, String> {
        let manifest = fetch_manifest(&self.client, manifest_url, true).await?;
        if let Some(c) = self.cache.lock().await.as_mut() {
            c.by_url.insert(manifest_url.to_string(), manifest.clone());
        }
        self.invalidate(); // the list is rebuilt with the new manifest on next use
        Ok(manifest)
    }

    /// Adds a manifest fetched by the install preview to the cache (no second request).
    pub async fn remember(&self, manifest: AddonManifest) {
        let mut cache = self.cache.lock().await;
        if let Some(c) = cache.as_mut() {
            c.by_url.insert(manifest.transport_url.clone(), manifest);
        }
    }

    async fn build(&self, by_url: &mut HashMap<String, AddonManifest>) -> Result<LoadedAddons, String> {
        let store = AppStore::open(&self.dir).map_err(|e| e.to_string())?;
        let all = store.profiles().map_err(|e| e.to_string())?;
        let profile = all.active;
        let addon_profile = all.addon_profile(profile);
        let urls: Vec<String> = store
            .addons(addon_profile)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|a| a.enabled)
            .map(|a| a.manifest_url)
            .collect();
        let missing: Vec<String> = urls.iter().filter(|u| !by_url.contains_key(*u)).cloned().collect();
        let handles: Vec<_> = missing
            .iter()
            .cloned()
            .map(|url| {
                let client = self.client.clone();
                tauri::async_runtime::spawn(async move { (url.clone(), fetch_manifest(&client, &url, false).await) })
            })
            .collect();
        let mut errors = HashMap::new();
        for handle in handles {
            if let Ok((url, result)) = handle.await {
                match result {
                    Ok(manifest) => {
                        let _ = store.update_addon_summary(addon_profile, &url, summary_of(&manifest));
                        by_url.insert(url, manifest);
                    }
                    Err(e) => {
                        errors.insert(url, e);
                    }
                }
            }
        }
        let manifests: Vec<AddonManifest> = urls.iter().filter_map(|u| by_url.get(u).cloned()).collect();
        Ok(LoadedAddons { profile, addon_profile, enabled: urls.len(), failed: urls.len() - manifests.len(), manifests, errors })
    }
}

/// Loads and validates a manifest; the error says what went wrong without the URL (it may carry
/// the addon's configuration secrets). Texts follow the original's messages.
pub async fn fetch_manifest(client: &AddonClient, url: &str, force: bool) -> Result<AddonManifest, String> {
    use nuvio_core::CoreError;
    let payload = client.get_text(url, force).await.map_err(|e| describe(&e))?;
    return nuvio_core::manifest::parse_manifest(url, &payload).map_err(|e| describe(&e));

    fn describe(e: &CoreError) -> String {
        match e {
            CoreError::MissingField(field) => format!("Missing field in the manifest: \"{field}\""),
            CoreError::Json(_) | CoreError::InvalidPayload(_) => "The response is not a valid addon manifest.".into(),
            CoreError::HttpStatus { status } => format!("The address returned an HTTP {status} error."),
            CoreError::Network { kind } => match kind {
                nuvio_core::error::NetworkErrorKind::Timeout => "Timed out while loading the manifest.".into(),
                nuvio_core::error::NetworkErrorKind::Connect => "Could not connect to the addon address.".into(),
                _ => "Could not load the manifest.".into(),
            },
        }
    }
}

/// Stream-addon instance id, as `ManagedAddon.streamAddonInstanceId` (`addon:<id>:<manifestUrl>`).
pub fn addon_instance_id(manifest: &AddonManifest) -> String {
    format!("addon:{}:{}", manifest.id, manifest.transport_url)
}

/// `localizedMediaTypeLabel` (pt-BR strings of the original).
pub fn media_type_label(content_type: &str) -> String {
    match content_type.trim().to_lowercase().as_str() {
        "movie" => "Movies".into(),
        "series" => "Series".into(),
        "anime" => "Anime".into(),
        "channel" => "Channels".into(),
        "tv" => "TV".into(),
        _ => {
            let mut chars = content_type.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        }
    }
}
