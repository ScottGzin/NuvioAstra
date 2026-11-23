//! NextGen's own storage and the one-time migration from the current app.
//!
//! - Data lives in `%APPDATA%\NuvioNextGen` as small JSON documents, written atomically (temporary
//!   file + rename) with the previous version kept as `<name>.json.bak`, so a crash during a write
//!   never leaves a half-written document.
//! - Migration reads the current app's stores **read-only**, first copies them to
//!   `NuvioNextGen\backups\<label>` (verified byte for byte), imports what NextGen uses, validates the
//!   imported counts against the source, and records the result in `migration.json`. If anything
//!   fails before `migration.json` is written, the next start simply runs the migration again; the
//!   original files are never modified.

use crate::settings::ProfileSettings;
use crate::{LegacyStore, StoreError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 1: backup + Live TV favorites. 2: + per-profile settings (player and home).
/// 3: + per-profile library (saved items), watch progress and watched marks.
/// 4: + profiles (list, active profile) and each profile's installed addons with enabled state.
pub const MIGRATION_VERSION: u32 = 4;

/// `settings.json` holds every profile and is written by the UI, the player and the sync.
static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::Io { path: path.to_path_buf(), source }
}

pub fn default_app_dir() -> Result<PathBuf, StoreError> {
    // Windows keeps roaming data in %APPDATA%; Linux follows the XDG base directory spec,
    // falling back to ~/.local/share (the same layout used by the legacy store resolver).
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from).ok_or(StoreError::DataDirNotFound)?;
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or(StoreError::DataDirNotFound)?;
    Ok(base.join("NuvioNextGen"))
}

#[derive(Serialize, Deserialize)]
struct SyncBase<T> {
    account: String,
    value: T,
}

pub struct AppStore {
    dir: PathBuf,
}

impl AppStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    pub fn read<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, StoreError> {
        let path = self.path(name);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(StoreError::Io { path, source: e }),
        };
        match serde_json::from_slice(&bytes) {
            Ok(v) => Ok(Some(v)),
            // A corrupt document falls back to its last good version.
            Err(_) => {
                let backup = path.with_extension("json.bak");
                match fs::read(&backup) {
                    Ok(b) => Ok(serde_json::from_slice(&b).ok()),
                    Err(_) => Ok(None),
                }
            }
        }
    }

    /// Atomic write: `<name>.json.tmp` → rename over `<name>.json`; the previous file is kept as `.bak`.
    pub fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<(), StoreError> {
        let path = self.path(name);
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(value).map_err(|e| StoreError::Json { store: name.into(), key: String::new(), source: e })?;
        {
            use std::io::Write;
            let mut file = fs::File::create(&tmp).map_err(io(&tmp))?;
            file.write_all(&bytes).map_err(io(&tmp))?;
            // On disk before it replaces the previous version (power loss never leaves an empty file).
            file.sync_all().map_err(io(&tmp))?;
        }
        if path.exists() {
            let _ = fs::copy(&path, path.with_extension("json.bak"));
        }
        fs::rename(&tmp, &path).map_err(io(&path))
    }

    /// Collections of a profile as the account sent them (`collections_json`, kept as is: the Home
    /// reads the fields it knows and ignores the rest). `None` = never received on this computer.
    pub fn collections(&self, profile: i64) -> Result<Option<serde_json::Value>, StoreError> {
        self.read(&format!("collections-{profile}"))
    }

    pub fn set_collections(&self, profile: i64, value: &serde_json::Value) -> Result<(), StoreError> {
        self.write(&format!("collections-{profile}"), value)
    }

    /// The preferences of kind `kind` (`home`, `player`) both this computer and `account` last
    /// agreed on: a difference from them is a local change still to send.
    pub fn sync_base<T: DeserializeOwned>(&self, kind: &str, profile: i64, account: &str) -> Result<Option<T>, StoreError> {
        let Some(doc) = self.read::<SyncBase<serde_json::Value>>(&format!("sync-base-{kind}-{profile}"))? else { return Ok(None) };
        if doc.account != account {
            return Ok(None);
        }
        Ok(serde_json::from_value(doc.value).ok())
    }

    pub fn set_sync_base<T: Serialize>(&self, kind: &str, profile: i64, account: &str, value: &T) -> Result<(), StoreError> {
        self.write(&format!("sync-base-{kind}-{profile}"), &SyncBase { account: account.to_string(), value })
    }

    /// Copy of every profile's settings, made once, before the account's preferences first replace
    /// them on this computer (`settings-before-sync.json`).
    pub fn keep_settings_before_sync(&self) -> Result<(), StoreError> {
        const NAME: &str = "settings-before-sync";
        if self.path(NAME).exists() {
            return Ok(());
        }
        self.write(NAME, &self.all_settings()?)
    }

    /// A deleted profile: what it agreed on with the account no longer counts.
    pub fn forget_sync_bases(&self, profile: i64) -> Result<(), StoreError> {
        for kind in ["home", "player"] {
            let name = format!("sync-base-{kind}-{profile}");
            if self.path(&name).exists() {
                self.write(&name, &SyncBase { account: String::new(), value: () })?;
            }
        }
        Ok(())
    }

    /// Saves an edit made over `previous` (what the editor had loaded): only what the edit changed
    /// is written, so a screen that was open never undoes what the sync brought meanwhile.
    pub fn edit_profile_settings(&self, profile: i64, previous: &ProfileSettings, edited: &ProfileSettings) -> Result<ProfileSettings, StoreError> {
        fn overlay(current: &mut serde_json::Value, previous: &serde_json::Value, edited: &serde_json::Value) {
            use serde_json::Value;
            if previous == edited {
                return;
            }
            match (current, previous, edited) {
                (Value::Object(current), Value::Object(previous), Value::Object(edited)) => {
                    for (key, value) in edited {
                        overlay(current.entry(key.clone()).or_insert(Value::Null), previous.get(key).unwrap_or(&Value::Null), value);
                    }
                }
                (slot, _, _) => *slot = edited.clone(),
            }
        }
        let json = |settings: &ProfileSettings| serde_json::to_value(settings).unwrap_or_default();
        let (previous, edited_json) = (json(previous), json(edited));
        self.update_profile_settings(profile, |current| {
            let mut value = json(current);
            overlay(&mut value, &previous, &edited_json);
            *current = serde_json::from_value(value).unwrap_or_else(|_| edited.clone());
        })
    }

    /// Favorite live TV channel keys per profile.
    pub fn live_tv_favorites(&self) -> Result<BTreeMap<i64, Vec<String>>, StoreError> {
        Ok(self.read("livetv-favorites")?.unwrap_or_default())
    }

    pub fn set_live_tv_favorites(&self, profile: i64, keys: Vec<String>) -> Result<(), StoreError> {
        let mut all = self.live_tv_favorites()?;
        all.insert(profile, keys);
        self.write("livetv-favorites", &all)
    }

    /// Settings of every profile that has its own (`settings.json`).
    pub fn all_settings(&self) -> Result<BTreeMap<i64, ProfileSettings>, StoreError> {
        Ok(self.read("settings")?.unwrap_or_default())
    }

    /// Settings of one profile; the original's defaults when it never had any.
    pub fn profile_settings(&self, profile: i64) -> Result<ProfileSettings, StoreError> {
        Ok(self.all_settings()?.remove(&profile).unwrap_or_default())
    }

    /// Validates (original ranges) and saves one profile's settings; other profiles untouched.
    pub fn set_profile_settings(&self, profile: i64, settings: ProfileSettings) -> Result<ProfileSettings, StoreError> {
        self.update_profile_settings(profile, |current| *current = settings)
    }

    /// Read-modify-write of one profile's settings under their lock.
    pub fn update_profile_settings(&self, profile: i64, change: impl FnOnce(&mut ProfileSettings)) -> Result<ProfileSettings, StoreError> {
        let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.all_settings()?;
        let mut settings = all.remove(&profile).unwrap_or_default();
        change(&mut settings);
        let settings = settings.sanitized();
        all.insert(profile, settings.clone());
        self.write("settings", &all)?;
        Ok(settings)
    }

    pub fn migration(&self) -> Result<Option<MigrationRecord>, StoreError> {
        self.read("migration")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MigrationRecord {
    pub version: u32,
    pub source_dir: String,
    pub backup_dir: String,
    pub backup_files: usize,
    pub profiles: usize,
    pub live_tv_favorites: usize,
    /// Profiles whose settings were imported (version 2; profiles already set in NextGen are kept).
    #[serde(default)]
    pub settings_profiles: usize,
    /// Version 3: what was imported from the original (profiles that already had NextGen data
    /// are kept as they are and not counted).
    #[serde(default)]
    pub library: LibraryImport,
    /// Version 4: profiles imported (0 when NextGen already had its own list) and addons per profile.
    #[serde(default)]
    pub imported_profiles: usize,
    #[serde(default)]
    pub imported_addons: usize,
    /// Verified copy of NextGen's own documents taken before this run changed anything.
    #[serde(default)]
    pub destination_backup_dir: String,
    pub completed_at_epoch_ms: u128,
}

/// Recovery journal of a migration run (`migration-journal.json`), removed when the run completes.
/// A document listed in `created` but not in `validated` was written by a run that stopped before
/// checking it: the next run removes it and imports it again (it never existed before that run,
/// so nothing of the user's is lost).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MigrationJournal {
    pub target_version: u32,
    pub started_at_epoch_ms: u128,
    pub destination_backup_dir: String,
    pub created: Vec<String>,
    pub validated: Vec<String>,
}

impl MigrationJournal {
    fn save(&self, app: &AppStore) -> Result<(), StoreError> {
        app.write("migration-journal", self)
    }
    /// Before writing a document that does not exist yet.
    fn begin(&mut self, app: &AppStore, file: &str) -> Result<(), StoreError> {
        self.created.push(file.to_string());
        self.save(app)
    }
    /// After the document was read back and matched its source.
    fn done(&mut self, app: &AppStore, file: &str) -> Result<(), StoreError> {
        self.validated.push(file.to_string());
        self.save(app)
    }
}

/// Undoes what an interrupted run left unvalidated. Returns the files removed.
fn recover_interrupted(app: &AppStore) -> Result<Vec<String>, StoreError> {
    let Some(journal) = app.read::<MigrationJournal>("migration-journal")? else { return Ok(vec![]) };
    let mut removed = Vec::new();
    for file in journal.created.iter().filter(|f| !journal.validated.contains(f)) {
        let path = app.dir().join(file);
        if path.exists() {
            fs::remove_file(&path).map_err(io(&path))?;
            let _ = fs::remove_file(path.with_extension("json.bak"));
            removed.push(file.clone());
        }
    }
    Ok(removed)
}

impl AppStore {
    /// Verified copy (byte-for-byte check) of every NextGen document to `backups/<label>/`, e.g.
    /// before the first sync with an account. The encrypted session file is not a document and is
    /// never copied.
    pub fn backup_documents(&self, label: &str) -> Result<PathBuf, StoreError> {
        backup_destination(self, label)
    }
}

/// Verified copy of every NextGen document (`*.json`, not the backups) to `backups/<label>/`.
fn backup_destination(app: &AppStore, label: &str) -> Result<PathBuf, StoreError> {
    let target = app.dir().join("backups").join(label);
    fs::create_dir_all(&target).map_err(io(&target))?;
    for entry in fs::read_dir(app.dir()).map_err(io(app.dir()))?.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !path.is_file() || !name.ends_with(".json") || name == "migration-journal.json" {
            continue;
        }
        let bytes = fs::read(&path).map_err(io(&path))?;
        let dest = target.join(&name);
        fs::write(&dest, &bytes).map_err(io(&dest))?;
        if fs::read(&dest).map_err(io(&dest))? != bytes {
            return Err(StoreError::BackupMismatch(dest));
        }
    }
    Ok(target)
}

/// A document just imported from the original that did not validate is removed, so the next start
/// imports it again instead of keeping (and then skipping) a bad copy. Only documents created by
/// this migration step go through here; documents NextGen already had are never removed.
fn discard(app: &AppStore, file: impl AsRef<str>) -> StoreError {
    let path = app.dir().join(file.as_ref());
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("json.bak"));
    StoreError::BackupMismatch(path)
}

/// Profiles (once) and each profile's addon list (when NextGen has none for it), validated.
fn migrate_profiles_and_addons(app: &AppStore, legacy: &LegacyStore, indexes: &[i64], journal: &mut MigrationJournal) -> Result<(usize, usize), StoreError> {
    use crate::profiles::{InstalledAddon, Profile, Profiles};
    let mut imported_profiles = 0;
    if !app.has_profiles() {
        if let Some(stored) = legacy.profiles()? {
            let profiles = Profiles {
                active: stored.active_profile_index,
                profiles: stored
                    .profiles
                    .iter()
                    .map(|p| Profile {
                        index: p.profile_index,
                        name: p.name.clone(),
                        avatar_color_hex: p.avatar_color_hex.clone().unwrap_or_else(|| crate::profiles::PROFILE_COLORS[0].into()),
                        avatar_id: p.avatar_id.clone(),
                        uses_primary_addons: p.uses_primary_addons,
                        pin_enabled: p.pin_enabled,
                        ..Profile::default()
                    })
                    .collect(),
            };
            journal.begin(app, "profiles.json")?;
            app.write_profiles(&profiles)?;
            let written = app.profiles()?;
            for p in &profiles.profiles {
                if (1..=crate::profiles::MAX_PROFILES).contains(&p.index) && written.get(p.index).map(|w| &w.name) != Some(&p.name) {
                    return Err(discard(app, "profiles.json"));
                }
            }
            journal.done(app, "profiles.json")?;
            imported_profiles = written.profiles.len();
        }
    }
    let mut imported_addons = 0;
    for index in indexes {
        if app.has_addons(*index) {
            continue;
        }
        let source: Vec<InstalledAddon> = legacy
            .installed_addons(*index)?
            .into_iter()
            .map(|a| InstalledAddon { manifest_url: a.manifest_url, enabled: a.enabled, summary: None })
            .collect();
        journal.begin(app, &format!("addons-{index}.json"))?;
        app.write_addons(*index, source.clone())?;
        let written = app.addons(*index)?;
        if written.iter().map(|a| (&a.manifest_url, a.enabled)).ne(source.iter().map(|a| (&a.manifest_url, a.enabled))) {
            return Err(discard(app, format!("addons-{index}.json")));
        }
        journal.done(app, &format!("addons-{index}.json"))?;
        imported_addons += written.len();
    }
    Ok((imported_profiles, imported_addons))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryImport {
    pub saved_items: usize,
    pub progress_entries: usize,
    pub watched_items: usize,
    /// Entries of the original that could not be read (left in the backup, not imported).
    pub skipped: usize,
}

/// Imports one profile's library, progress and watched marks, each only when NextGen has no
/// document for it yet, and checks that every source key is present after the write.
fn migrate_library(app: &AppStore, legacy: &LegacyStore, profile: i64, total: &mut LibraryImport, journal: &mut MigrationJournal) -> Result<(), StoreError> {
    use crate::library::{dedupe_library, dedupe_progress, dedupe_watched};
    let [has_library, has_progress, has_watched] = app.has_library_docs(profile);
    if !has_library {
        let source = legacy.library_items(profile)?;
        total.skipped += source.skipped;
        let expected: Vec<String> = dedupe_library(source.items.clone()).iter().map(|i| i.key()).collect();
        journal.begin(app, &format!("library-{profile}.json"))?;
        app.import_library(profile, source.items)?;
        let written: Vec<String> = app.favorites(profile)?.iter().map(|i| i.key()).collect();
        if written != expected {
            return Err(discard(app, format!("library-{profile}.json")));
        }
        journal.done(app, &format!("library-{profile}.json"))?;
        total.saved_items += written.len();
    }
    if !has_progress {
        let source = legacy.watch_progress(profile)?;
        total.skipped += source.skipped;
        let expected = dedupe_progress(source.items.clone());
        journal.begin(app, &format!("progress-{profile}.json"))?;
        app.import_progress(profile, source.items)?;
        if app.progress(profile)? != expected {
            return Err(discard(app, format!("progress-{profile}.json")));
        }
        journal.done(app, &format!("progress-{profile}.json"))?;
        total.progress_entries += expected.len();
    }
    if !has_watched {
        let source = legacy.watched_items(profile)?;
        total.skipped += source.skipped;
        let expected = dedupe_watched(source.items.clone());
        journal.begin(app, &format!("watched-{profile}.json"))?;
        app.import_watched(profile, source.items)?;
        if app.watched(profile)? != expected {
            return Err(discard(app, format!("watched-{profile}.json")));
        }
        journal.done(app, &format!("watched-{profile}.json"))?;
        total.watched_items += expected.len();
    }
    Ok(())
}

/// Runs the migration once (or again if a previous attempt did not finish). Returns the record.
pub fn migrate_from_legacy(app: &AppStore, legacy: &LegacyStore) -> Result<MigrationRecord, StoreError> {
    if let Some(record) = app.migration()? {
        if record.version >= MIGRATION_VERSION {
            return Ok(record);
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    // 1. A previous run that stopped half-way: drop the documents it created without validating.
    let recovered = recover_interrupted(app)?;
    if !recovered.is_empty() {
        eprintln!("[nextgen] migration: {} unvalidated document(s) of an interrupted run removed for re-import", recovered.len());
    }
    // 2. Verified backups: the original's stores, and NextGen's own documents before any change.
    let backup = legacy.backup_to(&app.dir().join("backups"), &format!("legacy-{now}"))?;
    let destination = backup_destination(app, &format!("nextgen-{now}"))?;
    let mut journal = MigrationJournal {
        target_version: MIGRATION_VERSION,
        started_at_epoch_ms: now,
        destination_backup_dir: destination.display().to_string(),
        created: vec![],
        validated: vec![],
    };
    journal.save(app)?;

    let profiles = legacy.profiles()?.map(|p| p.profiles).unwrap_or_default();
    let indexes: Vec<i64> = if profiles.is_empty() { vec![1] } else { profiles.iter().map(|p| p.profile_index).collect() };
    let mut favorites: BTreeMap<i64, Vec<String>> = app.live_tv_favorites()?;
    let mut source_count = 0;
    for index in &indexes {
        let keys = legacy.live_tv_favorites(*index)?;
        source_count += keys.len();
        let target = favorites.entry(*index).or_default();
        for key in keys {
            if !target.contains(&key) {
                target.push(key);
            }
        }
    }
    app.write("livetv-favorites", &favorites)?;

    // Validation: everything that was read must be present after the write.
    let written = app.live_tv_favorites()?;
    for index in &indexes {
        for key in legacy.live_tv_favorites(*index)? {
            if !written.get(index).is_some_and(|keys| keys.contains(&key)) {
                return Err(StoreError::BackupMismatch(app.dir().join("livetv-favorites.json")));
            }
        }
    }

    // Settings (version 2): imported only for profiles that have none in NextGen yet, so a change
    // made in NextGen is never overwritten by the original's value.
    let mut settings = app.all_settings()?;
    let mut imported = Vec::new();
    for index in &indexes {
        if !settings.contains_key(index) {
            settings.insert(*index, legacy.profile_settings(*index)?);
            imported.push(*index);
        }
    }
    app.write("settings", &settings)?;
    let written = app.all_settings()?;
    for index in &imported {
        if written.get(index) != Some(&legacy.profile_settings(*index)?) {
            return Err(StoreError::BackupMismatch(app.dir().join("settings.json")));
        }
    }

    // Library, progress, watched (version 3).
    let mut library = LibraryImport::default();
    for index in &indexes {
        migrate_library(app, legacy, *index, &mut library, &mut journal)?;
    }

    // Profiles and addons (version 4).
    let (imported_profiles, imported_addons) = migrate_profiles_and_addons(app, legacy, &indexes, &mut journal)?;

    let record = MigrationRecord {
        version: MIGRATION_VERSION,
        source_dir: legacy.dir().display().to_string(),
        backup_dir: backup.dir.display().to_string(),
        backup_files: backup.files.len(),
        profiles: profiles.len(),
        live_tv_favorites: source_count,
        settings_profiles: imported.len(),
        library,
        imported_profiles,
        imported_addons,
        destination_backup_dir: destination.display().to_string(),
        completed_at_epoch_ms: now,
    };
    // 3. Only now, with every step validated, the version is recorded; then the journal goes.
    app.write("migration", &record)?;
    if app.migration()?.as_ref() != Some(&record) {
        return Err(StoreError::BackupMismatch(app.dir().join("migration.json")));
    }
    let journal_path = app.dir().join("migration-journal.json");
    let _ = fs::remove_file(&journal_path);
    let _ = fs::remove_file(journal_path.with_extension("json.bak"));
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nuvio-nextgen-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_keeps_previous_version_and_recovers_from_corruption() {
        let store = AppStore::open(temp("atomic")).unwrap();
        store.set_live_tv_favorites(1, vec!["a".into()]).unwrap();
        store.set_live_tv_favorites(1, vec!["a".into(), "b".into()]).unwrap();
        fs::write(store.dir().join("livetv-favorites.json"), b"{broken").unwrap();
        // Falls back to the .bak written before the last successful write.
        assert_eq!(store.live_tv_favorites().unwrap().get(&1), Some(&vec!["a".to_string()]));
    }

    #[test]
    fn migration_backs_up_imports_validates_and_runs_once() {
        let legacy_dir = temp("legacy");
        fs::write(
            legacy_dir.join("nuvio_profiles.properties"),
            "profiles={\"profiles\"\\:[{\"profile_index\"\\:1,\"name\"\\:\"A\"},{\"profile_index\"\\:2,\"name\"\\:\"B\"}]}\n",
        )
        .unwrap();
        fs::write(legacy_dir.join("nuvio_live_tv.properties"), "live_tv_favorite_keys_2=[\"k1\",\"k2\"]\n").unwrap();
        let original = fs::read(legacy_dir.join("nuvio_live_tv.properties")).unwrap();
        let app = AppStore::open(temp("app")).unwrap();
        let legacy = LegacyStore::open(&legacy_dir).unwrap();

        let record = migrate_from_legacy(&app, &legacy).unwrap();
        assert_eq!(record.profiles, 2);
        assert_eq!(record.live_tv_favorites, 2);
        assert_eq!(record.backup_files, 2);
        assert_eq!(app.live_tv_favorites().unwrap().get(&2), Some(&vec!["k1".to_string(), "k2".to_string()]));
        assert_eq!(fs::read(legacy_dir.join("nuvio_live_tv.properties")).unwrap(), original); // untouched
        // Second run: recorded, nothing redone.
        let again = migrate_from_legacy(&app, &legacy).unwrap();
        assert_eq!(again, record);
    }

    #[test]
    fn migration_v2_imports_settings_per_profile_without_overwriting_nextgen_changes() {
        let legacy_dir = temp("legacy-settings");
        fs::write(
            legacy_dir.join("nuvio_profiles.properties"),
            r#"profiles={"profiles"\:[{"profile_index"\:1,"name"\:"A"},{"profile_index"\:2,"name"\:"B"}]}
"#,
        )
        .unwrap();
        // Written as java.util.Properties escapes them ('#' and ':' escaped).
        fs::write(
            legacy_dir.join("nuvio_player_settings.properties"),
            r#"resize_mode_1=Zoom
decoder_priority_1=2
preferred_audio_language_1=pt-br
subtitle_font_size_sp_1=24
subtitle_text_color_1=\#FFFFFF00
resize_mode_2=Fill
preferred_audio_language_2=en
"#,
        )
        .unwrap();
        fs::write(
            legacy_dir.join("nuvio_home_catalog_settings.properties"),
            r#"home_catalog_settings_1={"heroEnabled"\:false,"items"\:[{"key"\:"a\:movie\:top","enabled"\:false}]}
"#,
        )
        .unwrap();
        let app = AppStore::open(temp("app-settings")).unwrap();
        let legacy = LegacyStore::open(&legacy_dir).unwrap();
        // A previous version-1 migration and a change already made in NextGen for profile 2.
        app.write("migration", &MigrationRecord {
            version: 1, source_dir: String::new(), backup_dir: String::new(), backup_files: 0, profiles: 2,
            live_tv_favorites: 0, settings_profiles: 0, library: LibraryImport::default(), imported_profiles: 0, imported_addons: 0, destination_backup_dir: String::new(), completed_at_epoch_ms: 0,
        })
        .unwrap();
        let mut mine = ProfileSettings::default();
        mine.player.resize_mode = "Stretch".into();
        app.set_profile_settings(2, mine).unwrap();

        let record = migrate_from_legacy(&app, &legacy).unwrap();
        assert_eq!(record.version, MIGRATION_VERSION);
        assert_eq!(record.settings_profiles, 1);
        let p1 = app.profile_settings(1).unwrap();
        assert_eq!(p1.player.resize_mode, "Zoom");
        assert_eq!(p1.player.decoder_priority, 2);
        assert_eq!(p1.player.preferred_audio_language, "pt-br");
        assert_eq!(p1.player.subtitle.font_size_sp, 24);
        assert_eq!(p1.player.subtitle.text_color, "#FFFFFF00");
        assert!(!p1.home.hero_enabled);
        assert_eq!(p1.home.items[0].key, "a:movie:top");
        assert!(!p1.home.items[0].enabled);
        assert_eq!(app.profile_settings(2).unwrap().player.resize_mode, "Stretch"); // kept
        assert_eq!(app.profile_settings(9).unwrap(), ProfileSettings::default()); // never had any
    }

    #[test]
    fn migration_v3_imports_library_progress_and_watched_per_profile() {
        let legacy_dir = temp("legacy-library");
        fs::write(
            legacy_dir.join("nuvio_profiles.properties"),
            r#"profiles={"profiles"\:[{"profile_index"\:1,"name"\:"A"},{"profile_index"\:2,"name"\:"B"}]}
"#,
        )
        .unwrap();
        // As the original writes them: JSON payloads with the extra sync fields, ':' escaped.
        fs::write(
            legacy_dir.join("nuvio_library.properties"),
            r#"library_1={"items"\:[{"id"\:"tt1","type"\:"series","name"\:"Série","savedAtEpochMs"\:5,"traktId"\:null},{"id"\:"tt2","type"\:"movie","name"\:"Filme","savedAtEpochMs"\:9}],"deltaCursorEventId"\:0}
library_2={"items"\:[]}
"#,
        )
        .unwrap();
        fs::write(
            legacy_dir.join("nuvio_watch_progress.properties"),
            r#"watch_progress_1={"entries"\:[{"contentType"\:"series","parentMetaId"\:"tt1","parentMetaType"\:"series","videoId"\:"tt1\:1\:1","title"\:"Série","seasonNumber"\:1,"episodeNumber"\:1,"lastPositionMs"\:60000,"durationMs"\:2400000,"lastUpdatedEpochMs"\:10,"lastSourceUrl"\:"https\://cdn.test/x?token\=secret","progressKey"\:"tt1_s1e1"},{"contentType"\:"series","parentMetaId"\:"tt1","parentMetaType"\:"series","videoId"\:"tt1\:1\:1","title"\:"Série","seasonNumber"\:1,"episodeNumber"\:1,"lastPositionMs"\:30000,"durationMs"\:2400000,"lastUpdatedEpochMs"\:4},{"broken"\:true}]}
watch_progress_2={"entries"\:[{"contentType"\:"movie","parentMetaId"\:"tt7","parentMetaType"\:"movie","videoId"\:"tt7","title"\:"Outro","lastPositionMs"\:6000000,"durationMs"\:6000000,"lastUpdatedEpochMs"\:3,"isCompleted"\:true}]}
"#,
        )
        .unwrap();
        fs::write(
            legacy_dir.join("nuvio_watched.properties"),
            r#"watched_2={"items"\:[{"id"\:"tt7","type"\:"movie","name"\:"Outro","markedAtEpochMs"\:3}]}
"#,
        )
        .unwrap();
        let originals: Vec<Vec<u8>> = ["nuvio_library", "nuvio_watch_progress", "nuvio_watched"]
            .iter()
            .map(|n| fs::read(legacy_dir.join(format!("{n}.properties"))).unwrap())
            .collect();
        let app = AppStore::open(temp("app-library")).unwrap();
        let legacy = LegacyStore::open(&legacy_dir).unwrap();
        // Profile 2's favorites were already changed in NextGen: kept as they are.
        app.import_library(2, vec![]).unwrap();

        let record = migrate_from_legacy(&app, &legacy).unwrap();
        assert_eq!(record.library, LibraryImport { saved_items: 2, progress_entries: 2, watched_items: 1, skipped: 1 });
        assert_eq!(app.favorites(1).unwrap().iter().map(|i| i.key()).collect::<Vec<_>>(), vec!["movie/tt2", "series/tt1"]);
        let p1 = app.progress(1).unwrap();
        assert_eq!(p1.len(), 1, "duplicate key keeps the newest");
        assert_eq!(p1[0].last_position_ms, 60_000);
        assert_eq!(app.progress(2).unwrap()[0].parent_meta_id, "tt7");
        assert!(app.watched(1).unwrap().is_empty());
        assert_eq!(app.watched(2).unwrap().len(), 1);
        let stored = fs::read_to_string(app.dir().join("progress-1.json")).unwrap();
        assert!(!stored.contains("secret"), "stream URLs are not imported");
        for (n, bytes) in ["nuvio_library", "nuvio_watch_progress", "nuvio_watched"].iter().zip(originals) {
            assert_eq!(fs::read(legacy_dir.join(format!("{n}.properties"))).unwrap(), bytes, "original untouched");
        }
        // NextGen changes are never overwritten by a later run.
        app.remove_favorite(1, "movie", "tt2").unwrap();
        app.write("migration", &MigrationRecord { version: 2, ..record }).unwrap();
        let again = migrate_from_legacy(&app, &legacy).unwrap();
        assert_eq!(again.library, LibraryImport::default());
        assert_eq!(app.favorites(1).unwrap().len(), 1);
    }
}

#[cfg(test)]
mod v4_tests {
    use super::*;

    #[test]
    fn migration_v4_imports_profiles_and_addons_once() {
        let legacy_dir = std::env::temp_dir().join(format!("nuvio-v4-legacy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&legacy_dir);
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("nuvio_profiles.properties"),
            r#"profiles={"activeProfileIndex"\:2,"profiles"\:[{"profile_index"\:1,"name"\:"A","avatar_color_hex"\:"\#E9E8F2","avatar_id"\:"avatar_katara"},{"profile_index"\:2,"name"\:"B","uses_primary_addons"\:true}]}
"#,
        )
        .unwrap();
        fs::write(
            legacy_dir.join("nuvio_addons.properties"),
            r#"installed_addon_urls_1=["https\://a.test","https\://b.test/cfg/manifest.json","https\://a.test/manifest.json"]
addon_enabled_states_1={"https\://b.test/cfg/manifest.json"\:false}
"#,
        )
        .unwrap();
        let app_dir = std::env::temp_dir().join(format!("nuvio-v4-app-{}", std::process::id()));
        let _ = fs::remove_dir_all(&app_dir);
        let app = AppStore::open(&app_dir).unwrap();
        let legacy = LegacyStore::open(&legacy_dir).unwrap();
        let record = migrate_from_legacy(&app, &legacy).unwrap();
        assert_eq!((record.imported_profiles, record.imported_addons), (2, 2));
        let profiles = app.profiles().unwrap();
        assert_eq!(profiles.active, 2);
        assert_eq!(profiles.get(1).unwrap().avatar_color_hex, "#E9E8F2");
        assert_eq!(profiles.addon_profile(2), 1, "uses primary addons");
        let addons = app.addons(1).unwrap();
        assert_eq!(addons.iter().map(|a| (a.manifest_url.as_str(), a.enabled)).collect::<Vec<_>>(),
            vec![("https://a.test/manifest.json", true), ("https://b.test/cfg/manifest.json", false)], "normalized, de-duplicated, order and state kept");
        // Changes made in NextGen survive a later run.
        app.set_active_profile(1).unwrap().unwrap();
        app.write("migration", &MigrationRecord { version: 3, ..record }).unwrap();
        let again = migrate_from_legacy(&app, &legacy).unwrap();
        assert_eq!(again.imported_profiles, 0);
        assert_eq!(app.profiles().unwrap().active, 1);
    }
}

/// Dry run against a copy of real data: `NUVIO_LEGACY_DIR=<backup copy> cargo test -p nuvio-store real_data -- --ignored`.
#[cfg(test)]
mod real_data {
    use super::*;
    #[test]
    #[ignore]
    fn real_data_migration_dry_run() {
        let Some(dir) = std::env::var_os("NUVIO_LEGACY_DIR") else { return };
        let legacy = LegacyStore::open(PathBuf::from(&dir)).unwrap();
        for name in legacy.store_names().unwrap() {
            legacy.read_store(&name).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let target = std::env::temp_dir().join(format!("nuvio-real-dry-run-{}", std::process::id()));
        let _ = fs::remove_dir_all(&target);
        // Optionally start from a copy of an existing NextGen folder (its *.json only).
        if let Some(existing) = std::env::var_os("NUVIO_APP_COPY") {
            fs::create_dir_all(&target).unwrap();
            for e in fs::read_dir(PathBuf::from(existing)).unwrap().flatten() {
                if e.path().extension().is_some_and(|x| x == "json") {
                    fs::copy(e.path(), target.join(e.file_name())).unwrap();
                }
            }
        }
        let app = AppStore::open(&target).unwrap();
        let record = migrate_from_legacy(&app, &legacy).unwrap();
        println!("{:?}", record.library);
        for p in [1, 2] {
            let progress = app.progress(p).unwrap();
            let cw = crate::library::continue_watching(&progress, crate::library::CONTINUE_WATCHING_LIMIT);
            println!(
                "profile {p}: saved {}, progress {}, watched {}, continue watching {}",
                app.favorites(p).unwrap().len(),
                progress.len(),
                app.watched(p).unwrap().len(),
                cw.len()
            );
        }
        let _ = fs::remove_dir_all(&target);
    }
}

/// Transaction rules of the migration: destination backup, recovery journal, version recorded
/// last, idempotence, NextGen data never replaced by the original's.
#[cfg(test)]
mod transaction_tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nuvio-tx-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Original data: 2 profiles, progress for both, one saved item, one watched mark.
    fn legacy(name: &str, profile2_progress: &str) -> PathBuf {
        let d = dir(name);
        fs::write(d.join("nuvio_profiles.properties"), r#"profiles={"activeProfileIndex"\:1,"profiles"\:[{"profile_index"\:1,"name"\:"A"},{"profile_index"\:2,"name"\:"B"}]}
"#).unwrap();
        fs::write(d.join("nuvio_library.properties"), r#"library_1={"items"\:[{"id"\:"tt1","type"\:"movie","name"\:"Um","savedAtEpochMs"\:5}]}
"#).unwrap();
        let p1 = r#"{"entries"\:[{"contentType"\:"movie","parentMetaId"\:"tt1","parentMetaType"\:"movie","videoId"\:"tt1","title"\:"Um","lastPositionMs"\:60000,"durationMs"\:6000000,"lastUpdatedEpochMs"\:10}]}"#;
        fs::write(d.join("nuvio_watch_progress.properties"), format!("watch_progress_1={p1}\nwatch_progress_2={profile2_progress}\n")).unwrap();
        fs::write(d.join("nuvio_watched.properties"), r#"watched_1={"items"\:[{"id"\:"tt9","type"\:"movie","name"\:"Nove","markedAtEpochMs"\:3}]}
"#).unwrap();
        fs::write(d.join("nuvio_addons.properties"), r#"installed_addon_urls_1=["https\://a.test/manifest.json"]
"#).unwrap();
        d
    }

    const P2: &str = r#"{"entries"\:[{"contentType"\:"movie","parentMetaId"\:"tt2","parentMetaType"\:"movie","videoId"\:"tt2","title"\:"Dois","lastPositionMs"\:90000,"durationMs"\:6000000,"lastUpdatedEpochMs"\:20}]}"#;

    fn snapshot(app: &AppStore) -> std::collections::BTreeMap<String, Vec<u8>> {
        fs::read_dir(app.dir())
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap()))
            .collect()
    }

    #[test]
    fn backs_up_destination_records_last_and_is_idempotent() {
        let legacy_dir = legacy("legacy-idem", P2);
        let app = AppStore::open(dir("app-idem")).unwrap();
        // NextGen data that already exists and is newer than the original's: kept.
        app.import_library(1, vec![]).unwrap();
        app.set_live_tv_favorites(1, vec!["mine".into()]).unwrap();
        let before = snapshot(&app);
        let originals: Vec<Vec<u8>> = ["nuvio_library", "nuvio_watch_progress", "nuvio_watched", "nuvio_profiles", "nuvio_addons"]
            .iter()
            .map(|n| fs::read(legacy_dir.join(format!("{n}.properties"))).unwrap())
            .collect();

        let record = migrate_from_legacy(&app, &LegacyStore::open(&legacy_dir).unwrap()).unwrap();
        assert_eq!(record.version, MIGRATION_VERSION);
        // Destination backup holds the documents as they were before the run, byte for byte.
        let backup = PathBuf::from(&record.destination_backup_dir);
        for (name, bytes) in &before {
            assert_eq!(&fs::read(backup.join(name)).unwrap(), bytes, "{name} backed up");
        }
        assert!(!app.dir().join("migration-journal.json").exists(), "journal removed after success");
        assert!(app.favorites(1).unwrap().is_empty(), "NextGen's (empty) library is not replaced by the original's");
        assert_eq!(app.live_tv_favorites().unwrap()[&1], vec!["mine".to_string()]);
        assert_eq!(app.progress(1).unwrap().len(), 1);
        assert_eq!(app.progress(2).unwrap()[0].parent_meta_id, "tt2", "profiles kept apart");
        assert_eq!(app.watched(1).unwrap().len(), 1);
        assert_eq!(app.addons(1).unwrap().len(), 1);

        // Second run: nothing changes at all.
        let after_first = snapshot(&app);
        let again = migrate_from_legacy(&app, &LegacyStore::open(&legacy_dir).unwrap()).unwrap();
        assert_eq!(again, record);
        assert_eq!(snapshot(&app), after_first, "second run is a no-op");
        for (n, bytes) in ["nuvio_library", "nuvio_watch_progress", "nuvio_watched", "nuvio_profiles", "nuvio_addons"].iter().zip(originals) {
            assert_eq!(fs::read(legacy_dir.join(format!("{n}.properties"))).unwrap(), bytes, "original {n} untouched");
        }
    }

    #[test]
    fn failure_mid_way_keeps_validated_documents_and_does_not_record_the_version() {
        // Profile 2's progress store is unreadable: profile 1's documents are imported and
        // validated, then the run stops without recording the version.
        let legacy_dir = legacy("legacy-fail", "{broken");
        let app = AppStore::open(dir("app-fail")).unwrap();
        assert!(migrate_from_legacy(&app, &LegacyStore::open(&legacy_dir).unwrap()).is_err());
        assert!(app.migration().unwrap().is_none(), "version not recorded");
        assert_eq!(app.progress(1).unwrap().len(), 1, "validated document kept");
        let journal: MigrationJournal = app.read("migration-journal").unwrap().unwrap();
        assert!(journal.validated.contains(&"progress-1.json".to_string()));

        // The original is fixed (e.g. the old app rewrote it): the next start completes.
        fs::write(legacy_dir.join("nuvio_watch_progress.properties"), format!(
            "watch_progress_1={}\nwatch_progress_2={P2}\n",
            r#"{"entries"\:[{"contentType"\:"movie","parentMetaId"\:"tt1","parentMetaType"\:"movie","videoId"\:"tt1","title"\:"Um","lastPositionMs"\:60000,"durationMs"\:6000000,"lastUpdatedEpochMs"\:10}]}"#
        )).unwrap();
        let record = migrate_from_legacy(&app, &LegacyStore::open(&legacy_dir).unwrap()).unwrap();
        assert_eq!(record.version, MIGRATION_VERSION);
        assert_eq!(app.progress(1).unwrap().len(), 1, "not duplicated");
        assert_eq!(app.progress(2).unwrap().len(), 1, "imported on the retry");
        assert!(!app.dir().join("migration-journal.json").exists());
    }

    #[test]
    fn interrupted_run_is_recovered_without_touching_existing_data() {
        let legacy_dir = legacy("legacy-int", P2);
        let app = AppStore::open(dir("app-int")).unwrap();
        // State left by a run killed right after writing progress-2.json (not validated yet):
        // a truncated document, plus the user's own progress-1.json that existed before.
        app.write("progress-1", &serde_json::json!({ "entries": [] })).unwrap();
        fs::write(app.dir().join("progress-2.json"), b"{\"entries\": [").unwrap();
        MigrationJournal { target_version: MIGRATION_VERSION, created: vec!["progress-2.json".into()], ..Default::default() }.save(&app).unwrap();

        let record = migrate_from_legacy(&app, &LegacyStore::open(&legacy_dir).unwrap()).unwrap();
        assert_eq!(record.version, MIGRATION_VERSION);
        assert_eq!(app.progress(2).unwrap().len(), 1, "unvalidated document re-imported");
        assert!(app.progress(1).unwrap().is_empty(), "document not created by the interrupted run is kept as it was");
        assert!(!app.dir().join("migration-journal.json").exists());
    }
}
