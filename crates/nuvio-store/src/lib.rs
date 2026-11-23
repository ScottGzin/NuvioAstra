//! Access to the data of the current Nuvio Desktop app, for migration.
//!
//! The Kotlin app keeps one Java `.properties` file per store in the app data directory
//! (`%APPDATA%\Nuvio`, `~/Library/Application Support/Nuvio`, `$XDG_CONFIG_HOME/nuvio`), with JSON
//! values and keys suffixed by the profile index (see `DesktopStorage.kt`, `ProfileScopedKey.kt`).
//!
//! Everything here is **read-only** on the original files. Migration first copies the stores to
//! a timestamped backup and verifies the copy byte for byte; the originals are never modified,
//! so the old app keeps working and is the rollback path.

use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("app data directory not found")]
    DataDirNotFound,
    #[error("I/O error on {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("invalid .properties file {path}: {message}")]
    Properties { path: PathBuf, message: String },
    #[error("invalid JSON in store `{store}` key `{key}`: {source}")]
    Json { store: String, key: String, source: serde_json::Error },
    #[error("backup verification failed for {0}")]
    BackupMismatch(PathBuf),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::Io { path: path.to_path_buf(), source }
}

/// Default data directory of the current app, resolved exactly like `DesktopStorage.resolveAppDataDir`.
pub fn default_legacy_data_dir() -> Result<PathBuf, StoreError> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or(StoreError::DataDirNotFound)?;
    let dir = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join("Nuvio")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Nuvio")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("nuvio")
    };
    if dir.is_dir() { Ok(dir) } else { Err(StoreError::DataDirNotFound) }
}

/// Read-only view of the legacy stores.
pub struct LegacyStore {
    dir: PathBuf,
}

impl LegacyStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let dir = dir.into();
        if !dir.is_dir() {
            return Err(StoreError::DataDirNotFound);
        }
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Store names (file names without `.properties`), sorted.
    pub fn store_names(&self) -> Result<Vec<String>, StoreError> {
        let mut names: Vec<String> = fs::read_dir(&self.dir)
            .map_err(io(&self.dir))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".properties").map(str::to_string)
            })
            .collect();
        names.sort();
        Ok(names)
    }

    /// All key/value pairs of one store; a missing store is empty, like in the Kotlin app.
    pub fn read_store(&self, name: &str) -> Result<HashMap<String, String>, StoreError> {
        let path = self.dir.join(format!("{name}.properties"));
        if !path.exists() {
            return Ok(HashMap::new());
        }
        // Java writes .properties as ISO-8859-1 with \uXXXX escapes (`properties` follows the JDK).
        let bytes = fs::read(&path).map_err(io(&path))?;
        Ok(properties::parse(&bytes))
    }

    fn json<T: for<'de> Deserialize<'de>>(&self, store: &str, key: &str) -> Result<Option<T>, StoreError> {
        let values = self.read_store(store)?;
        values
            .get(key)
            .map(|raw| {
                serde_json::from_str(raw).map_err(|source| StoreError::Json {
                    store: store.to_string(),
                    key: key.to_string(),
                    source,
                })
            })
            .transpose()
    }

    /// Profiles as stored by `ProfileRepository` (store `nuvio_profiles`, key `profiles`).
    pub fn profiles(&self) -> Result<Option<StoredProfiles>, StoreError> {
        self.json("nuvio_profiles", "profiles")
    }

    /// Installed addon manifest URLs of a profile, in the user's order (`AddonPlatform.desktop.kt`).
    /// The URLs may contain addon configuration secrets: never log them.
    pub fn installed_addon_urls(&self, profile_index: i64) -> Result<Vec<String>, StoreError> {
        Ok(self
            .json::<Vec<String>>("nuvio_addons", &format!("installed_addon_urls_{profile_index}"))?
            .unwrap_or_default())
    }

    /// Enabled flag per manifest URL; addons missing from the map are enabled (Kotlin default).
    pub fn addon_enabled_states(&self, profile_index: i64) -> Result<BTreeMap<String, bool>, StoreError> {
        Ok(self
            .json::<BTreeMap<String, bool>>("nuvio_addons", &format!("addon_enabled_states_{profile_index}"))?
            .unwrap_or_default())
    }

    /// Addons of a profile with their enabled state, normalized exactly like
    /// `AddonRepository.initialize`: URLs pass through `dedupeManifestUrls`, enabled-state keys
    /// through `ensureManifestSuffix`, and an addon missing from the state map is enabled.
    pub fn installed_addons(&self, profile_index: i64) -> Result<Vec<InstalledAddon>, StoreError> {
        let states: BTreeMap<String, bool> = self
            .addon_enabled_states(profile_index)?
            .into_iter()
            .map(|(url, enabled)| (nuvio_core::url::ensure_manifest_suffix(&url), enabled))
            .collect();
        Ok(nuvio_core::url::dedupe_manifest_urls(self.installed_addon_urls(profile_index)?)
            .into_iter()
            .map(|url| InstalledAddon { enabled: states.get(&url).copied().unwrap_or(true), manifest_url: url })
            .collect())
    }

    /// Profile whose addons a profile uses: a secondary profile with `uses_primary_addons` reads
    /// profile 1's addons (`AddonRepository.resolveEffectiveProfileId`).
    pub fn effective_addon_profile(profiles: &StoredProfiles, profile_index: i64) -> i64 {
        let uses_primary = profiles
            .profiles
            .iter()
            .any(|p| p.profile_index == profile_index && p.profile_index != 1 && p.uses_primary_addons);
        if uses_primary { 1 } else { profile_index }
    }

    /// Home rows preferences (`HomeCatalogSettingsRepository`: store `nuvio_home_catalog_settings`,
    /// key `home_catalog_settings_<profile>`).
    pub fn home_catalog_settings(&self, profile_index: i64) -> Result<Option<HomeCatalogSettings>, StoreError> {
        self.json("nuvio_home_catalog_settings", &format!("home_catalog_settings_{profile_index}"))
    }

    /// Collections of a profile (`CollectionStorage`: store `nuvio_collections`, key
    /// `collections_<profile>`, the JSON array the account also keeps as `collections_json`).
    pub fn collections(&self, profile_index: i64) -> Result<Option<serde_json::Value>, StoreError> {
        self.json("nuvio_collections", &format!("collections_{profile_index}"))
    }

    /// The PIN hash Nuvio keeps for offline checks (`ProfilePinCacheStorage`): (salt, digest, profile date).
    pub fn pin_cache(&self, profile_index: i64) -> Result<Option<(String, String, String)>, StoreError> {
        let Some(saved) = self.json::<serde_json::Value>("nuvio_profile_pin_cache", &format!("profile_pin_{profile_index}"))? else { return Ok(None) };
        let text = |name: &str| saved.get(name).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let (salt, digest) = (text("salt"), text("digest"));
        Ok((!salt.is_empty() && !digest.is_empty()).then(|| (salt, digest, text("profileUpdatedAt"))))
    }

    /// Favorite live TV channel keys (`LiveTvFavoritesStorage`: store `nuvio_live_tv`, key
    /// `live_tv_favorite_keys_<profile>`, JSON array of `addon:<id>:<manifestUrl>|<type>|<metaId>`).
    pub fn live_tv_favorites(&self, profile_index: i64) -> Result<Vec<String>, StoreError> {
        Ok(self
            .json::<Vec<String>>("nuvio_live_tv", &format!("live_tv_favorite_keys_{profile_index}"))?
            .unwrap_or_default())
    }

    /// Items of a `{ "<field>": [...] }` payload; entries that do not parse are skipped (counted).
    fn payload_list<T: for<'de> Deserialize<'de>>(&self, store: &str, key: &str, field: &str) -> Result<LegacyList<T>, StoreError> {
        let Some(payload) = self.json::<serde_json::Value>(store, key)? else {
            return Ok(LegacyList { items: vec![], skipped: 0 });
        };
        let raw = payload.get(field).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let total = raw.len();
        let items: Vec<T> = raw.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect();
        Ok(LegacyList { skipped: total - items.len(), items })
    }

    /// Saved movies/series (`LibraryStorage`: store `nuvio_library`, key `library_<profile>`).
    pub fn library_items(&self, profile_index: i64) -> Result<LegacyList<library::LibraryItem>, StoreError> {
        self.payload_list("nuvio_library", &format!("library_{profile_index}"), "items")
    }

    /// Watch progress (`WatchProgressStorage`: store `nuvio_watch_progress`, key `watch_progress_<profile>`).
    /// Stream URLs in the entries are not kept by the parsed type.
    pub fn watch_progress(&self, profile_index: i64) -> Result<LegacyList<library::ProgressEntry>, StoreError> {
        self.payload_list("nuvio_watch_progress", &format!("watch_progress_{profile_index}"), "entries")
    }

    /// Watched marks (`WatchedStorage`: store `nuvio_watched`, key `watched_<profile>`).
    pub fn watched_items(&self, profile_index: i64) -> Result<LegacyList<library::WatchedItem>, StoreError> {
        self.payload_list("nuvio_watched", &format!("watched_{profile_index}"), "items")
    }

    /// Copies every `.properties` store into `backup_root/<label>/` and verifies each copy byte for
    /// byte. The originals are only read. Returns the backup directory.
    pub fn backup_to(&self, backup_root: &Path, label: &str) -> Result<BackupReport, StoreError> {
        let target = backup_root.join(label);
        fs::create_dir_all(&target).map_err(io(&target))?;
        let mut files = Vec::new();
        for name in self.store_names()? {
            let source = self.dir.join(format!("{name}.properties"));
            let dest = target.join(format!("{name}.properties"));
            let bytes = fs::read(&source).map_err(io(&source))?;
            fs::write(&dest, &bytes).map_err(io(&dest))?;
            let written = fs::read(&dest).map_err(io(&dest))?;
            if written != bytes {
                return Err(StoreError::BackupMismatch(dest));
            }
            files.push(BackupEntry { store: name, bytes: bytes.len() as u64 });
        }
        Ok(BackupReport { dir: target, files })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyList<T> {
    pub items: Vec<T>,
    /// Entries present in the original that could not be read (not imported).
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledAddon {
    pub manifest_url: String,
    pub enabled: bool,
}

#[derive(Debug)]
pub struct BackupReport {
    pub dir: PathBuf,
    pub files: Vec<BackupEntry>,
}

#[derive(Debug)]
pub struct BackupEntry {
    pub store: String,
    pub bytes: u64,
}

/// `StoredProfilePayload` in `ProfileRepository.kt`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredProfiles {
    #[serde(default)]
    pub user_id: String,
    #[serde(default = "first_profile")]
    pub active_profile_index: i64,
    #[serde(default)]
    pub profiles: Vec<StoredProfile>,
}

fn first_profile() -> i64 {
    1
}

/// Subset of `NuvioProfile` needed by the next-gen app; unknown fields are ignored, not lost
/// (the original file is untouched).
#[derive(Debug, Clone, Deserialize)]
pub struct StoredProfile {
    #[serde(default)]
    pub id: String,
    #[serde(default = "first_profile")]
    pub profile_index: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub avatar_id: Option<String>,
    #[serde(default)]
    pub avatar_color_hex: Option<String>,
    #[serde(default)]
    pub uses_primary_addons: bool,
    #[serde(default)]
    pub pin_enabled: bool,
}

/// `StoredHomeCatalogSettingsPayload` of `HomeCatalogSettingsRepository.kt`.
#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct HomeCatalogSettings {
    pub hero_enabled: bool,
    pub show_catalog_type: bool,
    pub hide_unreleased_content: bool,
    pub items: Vec<HomeCatalogPreference>,
    /// Catalogs of the hero, in order (`Some([])`: none). `None`: never chosen, so the original's
    /// rule over `hero_source_enabled` decides. Local, as in the original (see `hero.rs`).
    pub hero_catalogs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct HomeCatalogPreference {
    pub key: String,
    pub custom_title: String,
    pub enabled: bool,
    pub hero_source_enabled: bool,
    pub order: i64,
}

/// Kotlin defaults: hero on, catalog type shown, unreleased content visible.
impl Default for HomeCatalogSettings {
    fn default() -> Self {
        Self { hero_enabled: true, show_catalog_type: true, hide_unreleased_content: false, items: Vec::new(), hero_catalogs: None }
    }
}

impl Default for HomeCatalogPreference {
    fn default() -> Self {
        Self { key: String::new(), custom_title: String::new(), enabled: true, hero_source_enabled: true, order: 0 }
    }
}

mod hero;
pub mod library;
mod properties;
pub mod next_up;
pub mod nextgen;
pub mod profiles;
pub mod settings;
pub mod sync_state;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nuvio-store-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Written the way java.util.Properties.store() escapes values (':' '=' and non-ASCII).
    const ADDONS: &str = "#Thu Nov 19 10:00:00 BRT 2026\n\
installed_addon_urls_1=[\"https\\://a.example/cfg\\=1/manifest.json\",\"https\\://b.example/\",\"https\\://b.example/manifest.json\"]\n\
addon_enabled_states_1={\"https\\://b.example\"\\:false}\n";

    const PROFILES: &str = "profiles={\"userId\"\\:\"u1\",\"activeProfileIndex\"\\:2,\"profiles\"\\:[{\"id\"\\:\"p1\",\"profile_index\"\\:1,\"name\"\\:\"Mo\\u00EDs\",\"extra\"\\:true},{\"id\"\\:\"p2\",\"profile_index\"\\:2,\"name\"\\:\"Kids\",\"uses_primary_addons\"\\:true}]}\n";

    #[test]
    fn reads_addons_with_java_escapes_and_default_enabled_state() {
        let dir = temp_dir("addons");
        fs::write(dir.join("nuvio_addons.properties"), ADDONS).unwrap();
        let store = LegacyStore::open(&dir).unwrap();
        let addons = store.installed_addons(1).unwrap();
        assert_eq!(
            addons,
            vec![
                InstalledAddon { manifest_url: "https://a.example/cfg=1/manifest.json".into(), enabled: true },
                InstalledAddon { manifest_url: "https://b.example/manifest.json".into(), enabled: false },
            ]
        );
        assert!(store.installed_addons(2).unwrap().is_empty());
    }

    #[test]
    fn reads_profiles_payload_and_unicode_escapes() {
        let dir = temp_dir("profiles");
        fs::write(dir.join("nuvio_profiles.properties"), PROFILES).unwrap();
        let profiles = LegacyStore::open(&dir).unwrap().profiles().unwrap().unwrap();
        assert_eq!(profiles.active_profile_index, 2);
        assert_eq!(profiles.profiles.len(), 2);
        assert_eq!(profiles.profiles[0].name, "Moís");
        assert!(profiles.profiles[1].uses_primary_addons);
        assert_eq!(LegacyStore::effective_addon_profile(&profiles, 2), 1);
        assert_eq!(LegacyStore::effective_addon_profile(&profiles, 1), 1);
        assert_eq!(LegacyStore::effective_addon_profile(&profiles, 3), 3);
    }

    #[test]
    fn backup_copies_and_verifies_without_touching_originals() {
        let dir = temp_dir("backup-src");
        fs::write(dir.join("nuvio_addons.properties"), ADDONS).unwrap();
        fs::write(dir.join("nuvio_profiles.properties"), PROFILES).unwrap();
        let before = fs::read(dir.join("nuvio_addons.properties")).unwrap();
        let backup_root = temp_dir("backup-dst");
        let report = LegacyStore::open(&dir).unwrap().backup_to(&backup_root, "snapshot").unwrap();
        assert_eq!(report.files.len(), 2);
        assert_eq!(fs::read(report.dir.join("nuvio_addons.properties")).unwrap(), before);
        assert_eq!(fs::read(dir.join("nuvio_addons.properties")).unwrap(), before);
    }

    #[test]
    fn reads_home_settings_and_live_tv_favorites() {
        let dir = temp_dir("home");
        fs::write(
            dir.join("nuvio_home_catalog_settings.properties"),
            // As java.util.Properties writes it: ':' escaped as '\:'.
            r#"home_catalog_settings_1={"heroEnabled"\:false,"items"\:[{"key"\:"a\:movie\:top","enabled"\:false,"order"\:2}]}"#,
        )
        .unwrap();
        fs::write(dir.join("nuvio_live_tv.properties"), r#"live_tv_favorite_keys_1=["addon\:x\:https\://a/manifest.json|tv|c1"]"#).unwrap();
        let store = LegacyStore::open(&dir).unwrap();
        let home = store.home_catalog_settings(1).unwrap().unwrap();
        assert!(!home.hero_enabled);
        assert!(home.show_catalog_type); // missing → true, the Kotlin default
        assert_eq!(home.items[0].key, "a:movie:top");
        assert!(!home.items[0].enabled);
        assert!(home.items[0].hero_source_enabled);
        assert_eq!(store.live_tv_favorites(1).unwrap(), vec!["addon:x:https://a/manifest.json|tv|c1"]);
        assert!(store.live_tv_favorites(2).unwrap().is_empty());
    }

    #[test]
    fn missing_store_is_empty() {
        let dir = temp_dir("empty");
        let store = LegacyStore::open(&dir).unwrap();
        assert!(store.read_store("nuvio_watched").unwrap().is_empty());
        assert!(store.profiles().unwrap().is_none());
    }
}
