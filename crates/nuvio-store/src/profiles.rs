//! Profiles and installed addons, owned by NextGen (imported once from the original by migration v4).
//!
//! Port of the original's local rules:
//! - `ProfileRepository` / `NuvioProfile`: at most 6 profiles (`MAX_PROFILES`), indexes 1–6, a new
//!   profile takes the lowest free index; profile 1 (primary) cannot be deleted
//!   (`ProfileEditScreen`: delete only when `profileIndex > 1`); deleting removes all the profile's
//!   data; `uses_primary_addons` makes a secondary profile use profile 1's addons (and then it cannot
//!   change them: `profile_primary_addons_required`). Avatar colors from `PROFILE_COLORS`.
//! - `AddonRepository`: per profile an ordered list of manifest URLs plus an enabled map; URLs are
//!   normalized (`normalizeManifestUrl`) and de-duplicated (`dedupeManifestUrls`); installing an
//!   existing URL is refused (`addon_already_installed`); disabling keeps the addon; removing only
//!   drops it from the list (no library/history data is touched).
//!
//! Files: `profiles.json` (list + active index) and `addons-<effective profile>.json` (ordered
//! `{ manifestUrl, enabled, summary }`). Manifest URLs can carry addon configuration secrets: they
//! stay in these files and in Rust, and are never logged.

use crate::nextgen::AppStore;
use crate::sync_state::Change;
use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub const MAX_PROFILES: i64 = 6;
pub const PROFILE_COLORS: [&str; 16] = [
    "#1E88E5", "#E53935", "#43A047", "#FB8C00", "#8E24AA", "#00ACC1", "#F4511E", "#3949AB", "#C0CA33", "#D81B60", "#00897B",
    "#5E35B1", "#7CB342", "#039BE5", "#FFB300", "#6D4C41",
];

pub(crate) static LOCK: Mutex<()> = Mutex::new(());

fn default_color() -> String {
    PROFILE_COLORS[0].into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub index: i64,
    pub name: String,
    #[serde(default = "default_color")]
    pub avatar_color_hex: String,
    /// Avatar of the original's catalog (image served by the account backend; shown as color + initial).
    #[serde(default)]
    pub avatar_id: Option<String>,
    #[serde(default)]
    pub uses_primary_addons: bool,
    // Fields owned by the Nuvio account (`NuvioProfile`): kept as received so a push never wipes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_background_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_background_url: Option<String>,
    #[serde(default)]
    pub uses_primary_plugins: bool,
    /// Set by the account (`set_profile_pin`); the PIN itself is never stored, only a salted hash.
    #[serde(default)]
    pub pin_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_locked_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            index: 1,
            name: String::new(),
            avatar_color_hex: default_color(),
            avatar_id: None,
            uses_primary_addons: false,
            avatar_url: None,
            profile_background_id: None,
            profile_background_url: None,
            uses_primary_plugins: false,
            pin_enabled: false,
            pin_locked_until: None,
            updated_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Profiles {
    pub active: i64,
    pub profiles: Vec<Profile>,
}

impl Default for Profiles {
    fn default() -> Self {
        Self { active: 1, profiles: vec![Profile { index: 1, name: "Profile 1".into(), ..Profile::default() }] }
    }
}

impl Profiles {
    pub fn get(&self, index: i64) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.index == index)
    }

    /// `resolveEffectiveProfileId`: profile whose addon list a profile uses.
    pub fn addon_profile(&self, index: i64) -> i64 {
        match self.get(index) {
            Some(p) if p.index != 1 && p.uses_primary_addons => 1,
            _ => index,
        }
    }

    /// `isUsingPrimaryAddonsFromSecondaryProfile`: addon changes are refused for this profile.
    pub fn addons_read_only(&self, index: i64) -> bool {
        self.addon_profile(index) != index
    }

    fn sanitized(mut self) -> Self {
        self.profiles.sort_by_key(|p| p.index);
        self.profiles.dedup_by_key(|p| p.index);
        self.profiles.retain(|p| (1..=MAX_PROFILES).contains(&p.index));
        if self.profiles.is_empty() {
            return Self::default();
        }
        if self.get(self.active).is_none() {
            self.active = self.profiles[0].index;
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    Limit,
    NotFound,
    PrimaryCannotBeDeleted,
    EmptyName,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Limit => "The limit of 6 profiles has been reached.",
            Self::NotFound => "Profile not found.",
            Self::PrimaryCannotBeDeleted => "The primary profile cannot be deleted.",
            Self::EmptyName => "Enter a name for the profile.",
        })
    }
}

fn valid_color(hex: &str) -> String {
    let h = hex.trim();
    let ok = h.len() == 7 && h.starts_with('#') && h[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok { h.to_ascii_uppercase() } else { default_color() }
}

/// What NextGen knows of an addon without fetching it (shown for disabled addons; refreshed
/// whenever its manifest is loaded).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AddonSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub logo: Option<String>,
    pub types: Vec<String>,
    pub catalogs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAddon {
    pub manifest_url: String,
    pub enabled: bool,
    #[serde(default)]
    pub summary: Option<AddonSummary>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AddonsDoc {
    #[serde(default)]
    items: Vec<InstalledAddon>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddonError {
    AlreadyInstalled,
    NotFound,
    ReadOnlyProfile,
}

impl std::fmt::Display for AddonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Texts of the original (pt-BR).
        f.write_str(match self {
            Self::AlreadyInstalled => "This addon is already installed.",
            Self::NotFound => "Addon not found.",
            Self::ReadOnlyProfile => "This profile uses the primary profile’s addons.",
        })
    }
}

fn addons_doc(profile: i64) -> String {
    format!("addons-{profile}")
}

/// Short stable id of an addon entry for the UI (the URL itself never leaves Rust).
pub fn addon_key(manifest_url: &str) -> String {
    // FNV-1a 64: not a secret-preserving hash, only an opaque handle.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in manifest_url.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

impl AppStore {
    pub fn profiles(&self) -> Result<Profiles, StoreError> {
        Ok(self.read::<Profiles>("profiles")?.unwrap_or_default().sanitized())
    }

    pub(crate) fn has_profiles(&self) -> bool {
        self.dir().join("profiles.json").exists()
    }

    pub(crate) fn write_profiles(&self, profiles: &Profiles) -> Result<(), StoreError> {
        self.write("profiles", &profiles.clone().sanitized())
    }

    /// Selects the active profile; returns the stored list.
    pub fn set_active_profile(&self, index: i64) -> Result<Result<Profiles, ProfileError>, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.profiles()?;
        if all.get(index).is_none() {
            return Ok(Err(ProfileError::NotFound));
        }
        all.active = index;
        self.write_profiles(&all)?;
        Ok(Ok(all))
    }

    /// `createProfile`: lowest free index up to 6.
    pub fn create_profile(&self, name: &str, color: &str, uses_primary_addons: bool) -> Result<Result<Profile, ProfileError>, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let name = name.trim();
        if name.is_empty() {
            return Ok(Err(ProfileError::EmptyName));
        }
        let mut all = self.profiles()?;
        let Some(index) = (1..=MAX_PROFILES).find(|i| all.get(*i).is_none()) else {
            return Ok(Err(ProfileError::Limit));
        };
        let profile = Profile { index, name: name.to_string(), avatar_color_hex: valid_color(color), uses_primary_addons: uses_primary_addons && index != 1, ..Profile::default() };
        all.profiles.push(profile.clone());
        self.write_profiles(&all)?;
        self.note(index, Change::Profiles)?;
        Ok(Ok(profile))
    }

    /// `updateProfile`: name, color, uses-primary-addons (avatar id kept).
    pub fn update_profile(&self, index: i64, name: &str, color: &str, uses_primary_addons: bool) -> Result<Result<Profile, ProfileError>, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let name = name.trim();
        if name.is_empty() {
            return Ok(Err(ProfileError::EmptyName));
        }
        let mut all = self.profiles()?;
        let Some(p) = all.profiles.iter_mut().find(|p| p.index == index) else {
            return Ok(Err(ProfileError::NotFound));
        };
        p.name = name.to_string();
        p.avatar_color_hex = valid_color(color);
        p.uses_primary_addons = uses_primary_addons && index != 1;
        let updated = p.clone();
        self.write_profiles(&all)?;
        self.note(index, Change::Profiles)?;
        Ok(Ok(updated))
    }

    /// `deleteProfile` (local): the profile and all its NextGen data are removed, after a verified
    /// copy of each document to `backups/profile-<index>-<ms>/`. Profile 1 cannot be deleted. If the
    /// deleted profile was active, the first remaining one becomes active.
    pub fn delete_profile(&self, index: i64) -> Result<Result<(Profiles, std::path::PathBuf), ProfileError>, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if index == 1 {
            return Ok(Err(ProfileError::PrimaryCannotBeDeleted));
        }
        let mut all = self.profiles()?;
        if all.get(index).is_none() {
            return Ok(Err(ProfileError::NotFound));
        }
        let now = crate::library::now_ms();
        let backup = self.dir().join("backups").join(format!("profile-{index}-{now}"));
        std::fs::create_dir_all(&backup).map_err(|source| StoreError::Io { path: backup.clone(), source })?;
        // Whole shared documents are copied too (they hold every profile's part).
        for name in [format!("library-{index}"), format!("progress-{index}"), format!("watched-{index}"), addons_doc(index), "settings".into(), "livetv-favorites".into(), "profiles".into()] {
            let src = self.dir().join(format!("{name}.json"));
            if src.exists() {
                let bytes = std::fs::read(&src).map_err(|source| StoreError::Io { path: src.clone(), source })?;
                let dest = backup.join(format!("{name}.json"));
                std::fs::write(&dest, &bytes).map_err(|source| StoreError::Io { path: dest.clone(), source })?;
                if std::fs::read(&dest).ok().as_deref() != Some(&bytes[..]) {
                    return Err(StoreError::BackupMismatch(dest));
                }
            }
        }
        for name in [format!("library-{index}"), format!("progress-{index}"), format!("watched-{index}"), addons_doc(index)] {
            for ext in ["json", "json.bak"] {
                let _ = std::fs::remove_file(self.dir().join(format!("{name}.{ext}")));
            }
        }
        let mut settings = self.all_settings()?;
        if settings.remove(&index).is_some() {
            self.write("settings", &settings)?;
        }
        let mut live = self.live_tv_favorites()?;
        if live.remove(&index).is_some() {
            self.write("livetv-favorites", &live)?;
        }
        all.profiles.retain(|p| p.index != index);
        if all.active == index {
            all.active = all.profiles.first().map_or(1, |p| p.index);
        }
        self.write_profiles(&all)?;
        self.note(index, Change::ProfileDeleted(index))?;
        Ok(Ok((self.profiles()?, backup)))
    }

    /// Avatar of the account's catalog (`None` = color and initial). A local change to send.
    pub fn set_profile_avatar(&self, index: i64, avatar_id: Option<String>) -> Result<Result<Profile, ProfileError>, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.profiles()?;
        let Some(p) = all.profiles.iter_mut().find(|p| p.index == index) else {
            return Ok(Err(ProfileError::NotFound));
        };
        p.avatar_id = avatar_id.filter(|a| !a.trim().is_empty());
        p.avatar_url = None;
        let updated = p.clone();
        self.write_profiles(&all)?;
        self.note(index, Change::Profiles)?;
        Ok(Ok(updated))
    }

    /// PIN flag set by the account (`set_profile_pin` / `clear_profile_pin`); not a local change to send.
    pub fn set_profile_pin_state(&self, index: i64, enabled: bool) -> Result<(), StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.profiles()?;
        if let Some(p) = all.profiles.iter_mut().find(|p| p.index == index) {
            p.pin_enabled = enabled;
            self.write_profiles(&all)?;
        }
        Ok(())
    }

    /// Installed addons of an (effective) profile, in the user's order.
    pub fn addons(&self, addon_profile: i64) -> Result<Vec<InstalledAddon>, StoreError> {
        Ok(self.read::<AddonsDoc>(&addons_doc(addon_profile))?.unwrap_or_default().items)
    }

    pub(crate) fn has_addons(&self, profile: i64) -> bool {
        self.dir().join(format!("{}.json", addons_doc(profile))).exists()
    }

    pub(crate) fn write_addons(&self, addon_profile: i64, items: Vec<InstalledAddon>) -> Result<(), StoreError> {
        let mut seen = std::collections::HashSet::new();
        let items: Vec<_> = items.into_iter().filter(|a| seen.insert(a.manifest_url.clone())).collect();
        self.write(&addons_doc(addon_profile), &AddonsDoc { items })
    }

    fn edit_addons<T>(&self, profile: i64, edit: impl FnOnce(&mut Vec<InstalledAddon>) -> Result<T, AddonError>) -> Result<Result<T, AddonError>, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let all = self.profiles()?;
        if all.addons_read_only(profile) {
            return Ok(Err(AddonError::ReadOnlyProfile));
        }
        let mut items = self.addons(profile)?;
        match edit(&mut items) {
            Ok(value) => {
                self.write_addons(profile, items)?;
                self.note(profile, Change::Addons)?;
                Ok(Ok(value))
            }
            Err(e) => Ok(Err(e)),
        }
    }

    /// `installManifest`: appended at the end, enabled. `manifest_url` must already be normalized.
    pub fn install_addon(&self, profile: i64, manifest_url: &str, summary: AddonSummary) -> Result<Result<(), AddonError>, StoreError> {
        self.edit_addons(profile, |items| {
            if items.iter().any(|a| a.manifest_url == manifest_url) {
                return Err(AddonError::AlreadyInstalled);
            }
            items.push(InstalledAddon { manifest_url: manifest_url.to_string(), enabled: true, summary: Some(summary) });
            Ok(())
        })
    }

    pub fn set_addon_enabled(&self, profile: i64, key: &str, enabled: bool) -> Result<Result<(), AddonError>, StoreError> {
        self.edit_addons(profile, |items| {
            let a = items.iter_mut().find(|a| addon_key(&a.manifest_url) == key).ok_or(AddonError::NotFound)?;
            a.enabled = enabled;
            Ok(())
        })
    }

    /// `removeAddon`: only the list entry; library, history and progress stay.
    pub fn remove_addon(&self, profile: i64, key: &str) -> Result<Result<(), AddonError>, StoreError> {
        self.edit_addons(profile, |items| {
            let i = items.iter().position(|a| addon_key(&a.manifest_url) == key).ok_or(AddonError::NotFound)?;
            items.remove(i);
            Ok(())
        })
    }

    /// `moveAddon` by one position (`-1` up, `1` down).
    pub fn move_addon(&self, profile: i64, key: &str, delta: i64) -> Result<Result<(), AddonError>, StoreError> {
        self.edit_addons(profile, |items| {
            let i = items.iter().position(|a| addon_key(&a.manifest_url) == key).ok_or(AddonError::NotFound)?;
            let j = i as i64 + delta.signum();
            if j >= 0 && (j as usize) < items.len() {
                items.swap(i, j as usize);
            }
            Ok(())
        })
    }

    /// Keeps what was learned from a loaded manifest (no user-visible change, so allowed for
    /// read-only profiles too).
    pub fn update_addon_summary(&self, addon_profile: i64, manifest_url: &str, summary: AddonSummary) -> Result<(), StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut items = self.addons(addon_profile)?;
        let mut changed = false;
        for a in items.iter_mut().filter(|a| a.manifest_url == manifest_url) {
            if a.summary.as_ref() != Some(&summary) {
                a.summary = Some(summary.clone());
                changed = true;
            }
        }
        if changed {
            self.write_addons(addon_profile, items)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> AppStore {
        let dir = std::env::temp_dir().join(format!("nuvio-profiles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        AppStore::open(dir).unwrap()
    }

    #[test]
    fn profiles_follow_the_original_rules() {
        let s = store("rules");
        assert_eq!(s.profiles().unwrap().profiles.len(), 1, "default primary profile");
        for i in 2..=6 {
            let p = s.create_profile(&format!("P{i}"), "#e53935", true).unwrap().unwrap();
            assert_eq!(p.index, i);
            assert_eq!(p.avatar_color_hex, "#E53935");
        }
        assert_eq!(s.create_profile("Sete", "#000000", false).unwrap(), Err(ProfileError::Limit));
        assert_eq!(s.create_profile("  ", "#000000", false).unwrap(), Err(ProfileError::EmptyName));
        assert_eq!(s.delete_profile(1).unwrap().unwrap_err(), ProfileError::PrimaryCannotBeDeleted);
        s.set_active_profile(3).unwrap().unwrap();
        let (after, backup) = s.delete_profile(3).unwrap().unwrap();
        assert_eq!(after.active, 1, "deleting the active profile selects the first one");
        assert!(backup.join("profiles.json").exists());
        // The freed index is reused (lowest free).
        assert_eq!(s.create_profile("Nova", "bad", false).unwrap().unwrap().index, 3);
        assert_eq!(s.profiles().unwrap().get(3).unwrap().avatar_color_hex, "#1E88E5", "invalid color → default");
        let u = s.update_profile(2, " Dois ", "#43A047", false).unwrap().unwrap();
        assert_eq!((u.name.as_str(), u.uses_primary_addons), ("Dois", false));
        assert!(!s.update_profile(1, "Um", "#43A047", true).unwrap().unwrap().uses_primary_addons, "primary never uses primary");
    }

    #[test]
    fn addons_per_profile_with_primary_sharing_and_no_duplicates() {
        let s = store("addons");
        s.create_profile("Dois", "#1E88E5", false).unwrap().unwrap();
        s.create_profile("Três", "#1E88E5", true).unwrap().unwrap();
        let sum = |n: &str| AddonSummary { name: n.into(), ..Default::default() };
        s.install_addon(1, "https://a.test/manifest.json", sum("A")).unwrap().unwrap();
        s.install_addon(1, "https://b.test/x/manifest.json", sum("B")).unwrap().unwrap();
        assert_eq!(s.install_addon(1, "https://a.test/manifest.json", sum("A")).unwrap(), Err(AddonError::AlreadyInstalled));
        s.install_addon(2, "https://c.test/manifest.json", sum("C")).unwrap().unwrap();
        assert_eq!(s.addons(1).unwrap().len(), 2);
        assert_eq!(s.addons(2).unwrap().len(), 1, "profiles keep separate lists");
        // Profile 3 uses profile 1's addons and cannot change them.
        let all = s.profiles().unwrap();
        assert_eq!(all.addon_profile(3), 1);
        assert_eq!(s.install_addon(3, "https://d.test/manifest.json", sum("D")).unwrap(), Err(AddonError::ReadOnlyProfile));

        let b = addon_key("https://b.test/x/manifest.json");
        s.move_addon(1, &b, -1).unwrap().unwrap();
        assert_eq!(s.addons(1).unwrap()[0].manifest_url, "https://b.test/x/manifest.json");
        s.set_addon_enabled(1, &b, false).unwrap().unwrap();
        assert!(!s.addons(1).unwrap()[0].enabled, "disabled, still installed");
        s.remove_addon(1, &b).unwrap().unwrap();
        assert_eq!(s.addons(1).unwrap().len(), 1);
        assert_eq!(s.remove_addon(1, &b).unwrap(), Err(AddonError::NotFound));
    }
}
