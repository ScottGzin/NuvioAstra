//! Local side of the account sync: what changed and still has to be sent, the event cursors of the
//! account, and the merge rules applied to what the account sends back. Network-free; the requests
//! are made by `nuvio-sync`.
//!
//! Rules ported from the original (`LibrarySyncReconciler`, `WatchProgressRepository`,
//! `WatchedRepository`, `AddonRepository`, `ProfileRepository`):
//! - Library / watched marks: a snapshot replaces the local list except the keys with a pending local
//!   change (pending upserts keep the local item, pending deletes stay deleted); delta events are
//!   applied in order and skip keys with a pending local change; the cursor is the highest event id.
//! - Progress: snapshot entries are taken unless a pending ("dirty") local entry is fresher; an upsert
//!   event is ignored when the local entry is dirty and fresher, a delete event never removes a dirty
//!   local entry.
//! - Addons and profiles: local changes are sent first, then the account's list is taken as is.
//! - First link of a profile with the account (decision of this port, see `docs/ACCOUNT_SYNC.md`): the
//!   rules above, plus every local-only item kept and queued for upload, so data of both origins is
//!   preserved (nothing local is lost because the account has a different version).
//!
//! Every local change is recorded here even without an account (offline queue). Each pending key
//! carries a revision: a push clears only what it sent, so a change made while the push was running
//! is sent next time.

use crate::library::{self, favorite_key, progress_key, LibraryItem, ProgressEntry, WatchedItem};
use crate::nextgen::AppStore;
use crate::profiles::{InstalledAddon, Profile};
use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

static SYNC_LOCK: Mutex<()> = Mutex::new(());
const DOC: &str = "sync-state";

/// A watched mark to delete on the account (`WatchedDeleteKey`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct WatchedRef {
    pub content_id: String,
    pub content_type: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Cursor {
    /// The first snapshot was merged (afterwards only delta events are pulled).
    pub initialized: bool,
    pub event_id: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileSync {
    /// favorite key (`type/id`) → revision.
    pub library_upserts: BTreeMap<String, u64>,
    pub library_deletes: BTreeMap<String, u64>,
    /// progress key → revision.
    pub progress_dirty: BTreeMap<String, u64>,
    pub progress_deletes: BTreeMap<String, u64>,
    /// watched key → revision.
    pub watched_upserts: BTreeMap<String, u64>,
    pub watched_deletes: BTreeMap<String, (u64, WatchedRef)>,
    /// Revision of the last local change to this profile's addon list (0 = nothing pending).
    pub addons_dirty: u64,
    pub library: Cursor,
    pub progress: Cursor,
    pub watched: Cursor,
    /// Addon list taken from the account at least once.
    pub addons_linked: bool,
    pub last_sync_epoch_ms: i64,
}

impl ProfileSync {
    pub fn pending_count(&self) -> usize {
        self.library_upserts.len()
            + self.library_deletes.len()
            + self.progress_dirty.len()
            + self.progress_deletes.len()
            + self.watched_upserts.len()
            + self.watched_deletes.len()
            + usize::from(self.addons_dirty > 0)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncState {
    pub revision: u64,
    pub profiles: BTreeMap<i64, ProfileSync>,
    /// Revision of the last local change to the profile list (0 = nothing pending).
    pub profiles_dirty: u64,
    /// Profiles deleted locally whose account data must be deleted too (`sync_delete_profile_data`).
    pub profile_deletes: BTreeMap<i64, u64>,
    /// Account whose data this computer's data was merged with (first link done).
    pub linked_user_id: Option<String>,
    pub profiles_linked: bool,
    pub last_sync_epoch_ms: i64,
}

impl SyncState {
    pub fn pending_count(&self) -> usize {
        self.profiles.values().map(ProfileSync::pending_count).sum::<usize>()
            + usize::from(self.profiles_dirty > 0)
            + self.profile_deletes.len()
    }
}

/// A local change to record.
pub(crate) enum Change<'a> {
    FavoriteSaved(&'a str),
    FavoriteRemoved(&'a str),
    Progress { key: &'a str, completed: Option<WatchedItem> },
    HistoryRemoved { progress: Option<&'a str>, watched: Vec<WatchedItem> },
    /// Marked / unmarked by hand (`markWatched` / `unmarkWatched`), with the progress entries removed.
    Watched { marked: Vec<WatchedItem>, unmarked: Vec<WatchedItem>, progress_removed: Vec<String> },
    Addons,
    Profiles,
    ProfileDeleted(i64),
}

impl AppStore {
    pub fn sync_state(&self) -> Result<SyncState, StoreError> {
        Ok(self.read::<SyncState>(DOC)?.unwrap_or_default())
    }

    /// Read-modify-write of the sync state under its lock.
    pub fn update_sync_state<T>(&self, f: impl FnOnce(&mut SyncState) -> T) -> Result<T, StoreError> {
        let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.sync_state()?;
        let out = f(&mut state);
        self.write(DOC, &state)?;
        Ok(out)
    }

    pub(crate) fn note(&self, profile: i64, change: Change<'_>) -> Result<(), StoreError> {
        self.update_sync_state(|s| {
            s.revision += 1;
            let rev = s.revision;
            match change {
                Change::Profiles => s.profiles_dirty = rev,
                Change::ProfileDeleted(index) => {
                    s.profiles.remove(&index);
                    s.profile_deletes.insert(index, rev);
                    s.profiles_dirty = rev;
                }
                change => {
                    let p = s.profiles.entry(profile).or_default();
                    match change {
                        Change::FavoriteSaved(key) => {
                            p.library_deletes.remove(key);
                            p.library_upserts.insert(key.to_string(), rev);
                        }
                        Change::FavoriteRemoved(key) => {
                            p.library_upserts.remove(key);
                            p.library_deletes.insert(key.to_string(), rev);
                        }
                        Change::Progress { key, completed } => {
                            p.progress_deletes.remove(key);
                            p.progress_dirty.insert(key.to_string(), rev);
                            if let Some(w) = completed {
                                p.watched_deletes.remove(&w.key());
                                p.watched_upserts.insert(w.key(), rev);
                            }
                        }
                        Change::HistoryRemoved { progress, watched } => {
                            if let Some(key) = progress {
                                p.progress_dirty.remove(key);
                                p.progress_deletes.insert(key.to_string(), rev);
                            }
                            for w in watched {
                                p.watched_upserts.remove(&w.key());
                                let r = WatchedRef { content_id: w.id.clone(), content_type: w.content_type.clone(), season: w.season, episode: w.episode };
                                p.watched_deletes.insert(w.key(), (rev, r));
                            }
                        }
                        Change::Watched { marked, unmarked, progress_removed } => {
                            for key in progress_removed {
                                p.progress_dirty.remove(&key);
                                p.progress_deletes.insert(key, rev);
                            }
                            for w in marked {
                                p.watched_deletes.remove(&w.key());
                                p.watched_upserts.insert(w.key(), rev);
                            }
                            for w in unmarked {
                                p.watched_upserts.remove(&w.key());
                                let r = WatchedRef { content_id: w.id.clone(), content_type: w.content_type.clone(), season: w.season, episode: w.episode };
                                p.watched_deletes.insert(w.key(), (rev, r));
                            }
                        }
                        Change::Addons => p.addons_dirty = rev,
                        _ => {}
                    }
                }
            }
        })
    }

    /// After a successful push: removes the pending keys that were sent, unless changed again since.
    pub fn clear_sent(&self, profile: i64, sent: &PendingSnapshot) -> Result<(), StoreError> {
        self.update_sync_state(|s| {
            let p = s.profiles.entry(profile).or_default();
            fn clear(map: &mut BTreeMap<String, u64>, sent: &BTreeMap<String, u64>) {
                for (k, rev) in sent {
                    if map.get(k) == Some(rev) {
                        map.remove(k);
                    }
                }
            }
            clear(&mut p.library_upserts, &sent.library_upserts);
            clear(&mut p.library_deletes, &sent.library_deletes);
            clear(&mut p.progress_dirty, &sent.progress_dirty);
            clear(&mut p.progress_deletes, &sent.progress_deletes);
            clear(&mut p.watched_upserts, &sent.watched_upserts);
            for (k, (rev, _)) in &sent.watched_deletes {
                if p.watched_deletes.get(k).map(|(r, _)| r) == Some(rev) {
                    p.watched_deletes.remove(k);
                }
            }
            if sent.addons_dirty != 0 && p.addons_dirty == sent.addons_dirty {
                p.addons_dirty = 0;
            }
        })
    }

    /// What a push of `profile` has to send now, with the data of each pending key.
    pub fn pending_for(&self, profile: i64) -> Result<(PendingSnapshot, PendingData), StoreError> {
        let state = self.sync_state()?;
        let p = state.profiles.get(&profile).cloned().unwrap_or_default();
        let favorites = self.favorites(profile)?;
        let progress = self.progress(profile)?;
        let watched = self.watched(profile)?;
        let data = PendingData {
            library_upserts: favorites.into_iter().filter(|i| p.library_upserts.contains_key(&i.key())).collect(),
            library_deletes: p.library_deletes.keys().filter_map(|k| k.split_once('/').map(|(t, id)| (t.to_string(), id.to_string()))).collect(),
            progress_upserts: progress.into_iter().filter(|e| p.progress_dirty.contains_key(&e.key())).collect(),
            progress_deletes: p.progress_deletes.keys().cloned().collect(),
            watched_upserts: watched.into_iter().filter(|w| p.watched_upserts.contains_key(&w.key())).collect(),
            watched_deletes: p.watched_deletes.values().map(|(_, r)| r.clone()).collect(),
            addons: if p.addons_dirty > 0 { Some(self.addons(profile)?) } else { None },
        };
        let snap = PendingSnapshot {
            library_upserts: p.library_upserts,
            library_deletes: p.library_deletes,
            progress_dirty: p.progress_dirty,
            progress_deletes: p.progress_deletes,
            watched_upserts: p.watched_upserts,
            watched_deletes: p.watched_deletes,
            addons_dirty: p.addons_dirty,
        };
        Ok((snap, data))
    }
}

/// Revisions of what a push sends (to clear exactly that afterwards).
#[derive(Debug, Clone, Default)]
pub struct PendingSnapshot {
    pub library_upserts: BTreeMap<String, u64>,
    pub library_deletes: BTreeMap<String, u64>,
    pub progress_dirty: BTreeMap<String, u64>,
    pub progress_deletes: BTreeMap<String, u64>,
    pub watched_upserts: BTreeMap<String, u64>,
    pub watched_deletes: BTreeMap<String, (u64, WatchedRef)>,
    pub addons_dirty: u64,
}

#[derive(Debug, Clone, Default)]
pub struct PendingData {
    pub library_upserts: Vec<LibraryItem>,
    /// (content type, content id)
    pub library_deletes: Vec<(String, String)>,
    pub progress_upserts: Vec<ProgressEntry>,
    pub progress_deletes: Vec<String>,
    pub watched_upserts: Vec<WatchedItem>,
    pub watched_deletes: Vec<WatchedRef>,
    pub addons: Option<Vec<InstalledAddon>>,
}

impl PendingData {
    pub fn is_empty(&self) -> bool {
        self.library_upserts.is_empty()
            && self.library_deletes.is_empty()
            && self.progress_upserts.is_empty()
            && self.progress_deletes.is_empty()
            && self.watched_upserts.is_empty()
            && self.watched_deletes.is_empty()
            && self.addons.is_none()
    }
}

// ---------------------------------------------------------------- remote data (account format)

/// A progress row of the account (`WatchProgressSyncEntry`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteProgress {
    pub progress_key: String,
    pub content_id: String,
    pub content_type: String,
    pub video_id: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub position: i64,
    pub duration: i64,
    pub last_watched: i64,
}

impl RemoteProgress {
    pub fn key(&self) -> String {
        if self.progress_key.trim().is_empty() { progress_key(&self.content_id, self.season, self.episode) } else { self.progress_key.clone() }
    }

    /// `ProgressSyncRecord.toWatchProgressEntry`: display fields from the local entry when known.
    pub fn to_entry(&self, cached: Option<&ProgressEntry>, watched_percent: i64) -> ProgressEntry {
        ProgressEntry {
            content_type: self.content_type.clone(),
            parent_meta_id: self.content_id.clone(),
            parent_meta_type: cached.map(|c| c.parent_meta_type.clone()).unwrap_or_else(|| self.content_type.clone()),
            video_id: self.video_id.clone(),
            title: cached.map(|c| c.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| self.content_id.clone()),
            logo: cached.and_then(|c| c.logo.clone()),
            poster: cached.and_then(|c| c.poster.clone()),
            background: cached.and_then(|c| c.background.clone()),
            season_number: self.season,
            episode_number: self.episode,
            episode_title: cached.and_then(|c| c.episode_title.clone()),
            episode_thumbnail: cached.and_then(|c| c.episode_thumbnail.clone()),
            last_position_ms: self.position,
            duration_ms: self.duration,
            last_updated_epoch_ms: self.last_watched,
            provider_name: cached.and_then(|c| c.provider_name.clone()),
            last_stream_title: cached.and_then(|c| c.last_stream_title.clone()),
            is_completed: library::is_complete(self.position, self.duration, false, watched_percent),
            watched_percent,
            progress_percent: None,
            progress_key: Some(self.key()),
        }
        .normalized()
    }
}

pub struct RemoteEvent<T> {
    pub event_id: i64,
    /// `upsert` or `delete`.
    pub operation: String,
    pub item: T,
}

/// `isFresherThan`: the newest update wins (then the furthest position).
pub fn fresher(a: &ProgressEntry, b: &ProgressEntry) -> bool {
    (a.last_updated_epoch_ms, a.last_position_ms) > (b.last_updated_epoch_ms, b.last_position_ms)
}

/// What a merge changed (to tell the UI and the report).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeStats {
    pub received: usize,
    pub changed: bool,
    /// Local-only items queued for upload by the first link.
    pub queued_local: usize,
}

impl AppStore {
    // ---------------- library

    pub fn apply_library_snapshot(&self, profile: i64, server: Vec<LibraryItem>, cursor: i64) -> Result<MergeStats, StoreError> {
        let _g = library::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let local = self.favorites(profile)?;
        let state = self.sync_state()?;
        let p = state.profiles.get(&profile).cloned().unwrap_or_default();
        let first_link = !p.library.initialized;
        let received = server.len();
        let mut merged: BTreeMap<String, LibraryItem> = server.into_iter().map(|i| (i.key(), i)).collect();
        for key in p.library_deletes.keys() {
            merged.remove(key);
        }
        let mut queued = Vec::new();
        for item in &local {
            let key = item.key();
            if p.library_upserts.contains_key(&key) {
                merged.insert(key, item.clone());
            } else if first_link && !merged.contains_key(&key) && !p.library_deletes.contains_key(&key) {
                merged.insert(key.clone(), item.clone());
                queued.push(key);
            }
        }
        let new = library::dedupe_library(merged.into_values());
        let changed = new != local;
        if changed {
            self.import_library(profile, new)?;
        }
        self.update_sync_state(|s| {
            s.revision += 1;
            let rev = s.revision;
            let p = s.profiles.entry(profile).or_default();
            for key in &queued {
                p.library_upserts.insert(key.clone(), rev);
            }
            p.library = Cursor { initialized: true, event_id: cursor.max(p.library.event_id) };
        })?;
        Ok(MergeStats { received, changed, queued_local: queued.len() })
    }

    pub fn apply_library_delta(&self, profile: i64, events: Vec<RemoteEvent<LibraryItem>>) -> Result<MergeStats, StoreError> {
        let _g = library::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let local = self.favorites(profile)?;
        let p = self.sync_state()?.profiles.get(&profile).cloned().unwrap_or_default();
        let mut items: BTreeMap<String, LibraryItem> = local.iter().map(|i| (i.key(), i.clone())).collect();
        let mut events = events;
        events.sort_by_key(|e| e.event_id);
        let mut cursor = p.library.event_id;
        let received = events.len();
        for e in events {
            cursor = cursor.max(e.event_id);
            let key = e.item.key();
            if p.library_upserts.contains_key(&key) || p.library_deletes.contains_key(&key) {
                continue;
            }
            match e.operation.trim().to_ascii_lowercase().as_str() {
                "upsert" => {
                    items.insert(key, e.item);
                }
                "delete" => {
                    items.remove(&key);
                }
                _ => {}
            }
        }
        let new = library::dedupe_library(items.into_values());
        let changed = new != local;
        if changed {
            self.import_library(profile, new)?;
        }
        self.update_sync_state(|s| s.profiles.entry(profile).or_default().library.event_id = cursor)?;
        Ok(MergeStats { received, changed, queued_local: 0 })
    }

    // ---------------- progress

    pub fn apply_progress_snapshot(&self, profile: i64, server: Vec<RemoteProgress>, cursor: i64) -> Result<MergeStats, StoreError> {
        let _g = library::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let local = self.progress(profile)?;
        let percent = self.watched_percent(profile);
        let p = self.sync_state()?.profiles.get(&profile).cloned().unwrap_or_default();
        let first_link = !p.progress.initialized;
        let local_by_key: BTreeMap<String, ProgressEntry> = local.iter().map(|e| (e.key(), e.clone())).collect();
        let received = server.len();
        let mut merged: BTreeMap<String, ProgressEntry> = BTreeMap::new();
        for r in &server {
            let candidate = r.to_entry(local_by_key.get(&r.key()), percent);
            match merged.get(&candidate.key()) {
                Some(existing) if !fresher(&candidate, existing) => {}
                _ => {
                    merged.insert(candidate.key(), candidate);
                }
            }
        }
        let mut queued = Vec::new();
        // First link: every local entry is compared with the account's (local-only ones and local
        // ones that are fresher are kept and sent), so a newer local position is never replaced.
        for (key, entry) in &local_by_key {
            let dirty = p.progress_dirty.contains_key(key);
            if dirty || first_link {
                if merged.get(key).is_none_or(|remote| fresher(entry, remote)) {
                    merged.insert(key.clone(), entry.clone());
                    if !dirty {
                        queued.push(key.clone());
                    }
                }
            }
        }
        for key in p.progress_deletes.keys() {
            merged.remove(key);
        }
        let new = library::dedupe_progress(merged.into_values());
        let changed = new != local;
        if changed {
            self.import_progress(profile, new)?;
        }
        self.update_sync_state(|s| {
            s.revision += 1;
            let rev = s.revision;
            let p = s.profiles.entry(profile).or_default();
            for key in &queued {
                p.progress_dirty.insert(key.clone(), rev);
            }
            p.progress = Cursor { initialized: true, event_id: cursor.max(p.progress.event_id) };
        })?;
        Ok(MergeStats { received, changed, queued_local: queued.len() })
    }

    pub fn apply_progress_delta(&self, profile: i64, events: Vec<RemoteEvent<RemoteProgress>>) -> Result<MergeStats, StoreError> {
        let _g = library::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let local = self.progress(profile)?;
        let percent = self.watched_percent(profile);
        let p = self.sync_state()?.profiles.get(&profile).cloned().unwrap_or_default();
        let mut items: BTreeMap<String, ProgressEntry> = local.iter().map(|e| (e.key(), e.clone())).collect();
        let mut events = events;
        events.sort_by_key(|e| e.event_id);
        let mut cursor = p.progress.event_id;
        let received = events.len();
        let mut cleared = Vec::new();
        for e in events {
            cursor = cursor.max(e.event_id);
            let key = e.item.key();
            let dirty = p.progress_dirty.contains_key(&key);
            let current = items.get(&key).cloned();
            match e.operation.trim().to_ascii_lowercase().as_str() {
                // `decideWatchProgressDeltaEvent`
                "upsert" => {
                    if e.item.video_id.trim().is_empty() {
                        continue;
                    }
                    let updated = e.item.to_entry(current.as_ref(), percent);
                    match &current {
                        Some(cur) if dirty && fresher(cur, &updated) => {}
                        Some(cur) if *cur == updated => cleared.push(key),
                        _ => {
                            items.insert(key.clone(), updated);
                            cleared.push(key);
                        }
                    }
                }
                "delete" => {
                    if current.is_some() && !dirty {
                        items.remove(&key);
                        cleared.push(key);
                    }
                }
                _ => {}
            }
        }
        let new = library::dedupe_progress(items.into_values());
        let changed = new != local;
        if changed {
            self.import_progress(profile, new)?;
        }
        self.update_sync_state(|s| {
            let p = s.profiles.entry(profile).or_default();
            for key in cleared {
                p.progress_dirty.remove(&key);
            }
            p.progress.event_id = cursor;
        })?;
        Ok(MergeStats { received, changed, queued_local: 0 })
    }

    // ---------------- watched marks

    pub fn apply_watched_snapshot(&self, profile: i64, server: Vec<WatchedItem>, cursor: i64) -> Result<MergeStats, StoreError> {
        let _g = library::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let local = self.watched(profile)?;
        let p = self.sync_state()?.profiles.get(&profile).cloned().unwrap_or_default();
        let first_link = !p.watched.initialized;
        let received = server.len();
        let mut merged: BTreeMap<String, WatchedItem> = server.into_iter().map(|w| (w.key(), w)).collect();
        for key in p.watched_deletes.keys() {
            merged.remove(key);
        }
        let mut queued = Vec::new();
        for w in &local {
            let key = w.key();
            if p.watched_upserts.contains_key(&key) {
                merged.insert(key, w.clone());
            } else if first_link && !merged.contains_key(&key) && !p.watched_deletes.contains_key(&key) {
                merged.insert(key.clone(), w.clone());
                queued.push(key);
            }
        }
        let new = library::dedupe_watched(merged.into_values());
        let changed = new != local;
        if changed {
            self.import_watched(profile, new)?;
        }
        self.update_sync_state(|s| {
            s.revision += 1;
            let rev = s.revision;
            let p = s.profiles.entry(profile).or_default();
            for key in &queued {
                p.watched_upserts.insert(key.clone(), rev);
            }
            p.watched = Cursor { initialized: true, event_id: cursor.max(p.watched.event_id) };
        })?;
        Ok(MergeStats { received, changed, queued_local: queued.len() })
    }

    pub fn apply_watched_delta(&self, profile: i64, events: Vec<RemoteEvent<WatchedItem>>) -> Result<MergeStats, StoreError> {
        let _g = library::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let local = self.watched(profile)?;
        let p = self.sync_state()?.profiles.get(&profile).cloned().unwrap_or_default();
        let mut items: BTreeMap<String, WatchedItem> = local.iter().map(|w| (w.key(), w.clone())).collect();
        let mut events = events;
        events.sort_by_key(|e| e.event_id);
        let mut cursor = p.watched.event_id;
        let received = events.len();
        for e in events {
            cursor = cursor.max(e.event_id);
            let key = e.item.key();
            if p.watched_upserts.contains_key(&key) || p.watched_deletes.contains_key(&key) {
                continue;
            }
            match e.operation.trim().to_ascii_lowercase().as_str() {
                "upsert" => {
                    items.insert(key, e.item);
                }
                "delete" => {
                    items.remove(&key);
                }
                _ => {}
            }
        }
        let new = library::dedupe_watched(items.into_values());
        let changed = new != local;
        if changed {
            self.import_watched(profile, new)?;
        }
        self.update_sync_state(|s| s.profiles.entry(profile).or_default().watched.event_id = cursor)?;
        Ok(MergeStats { received, changed, queued_local: 0 })
    }

    // ---------------- addons

    /// The account's addon list of `profile` (`addons` rows by `sort_order`): `(manifest url, enabled)`.
    /// Skipped while a local change is pending (sent first) — except on the first link, where the
    /// whole-list push must not replace the account's list: the lists are joined first (account
    /// order, then local-only addons) and the result is queued for upload.
    pub fn apply_addons_remote(&self, profile: i64, remote: Vec<(String, bool)>) -> Result<MergeStats, StoreError> {
        let _g = crate::profiles::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let state = self.sync_state()?;
        let p = state.profiles.get(&profile).cloned().unwrap_or_default();
        if p.addons_dirty > 0 && p.addons_linked {
            return Ok(MergeStats::default());
        }
        let local = self.addons(profile)?;
        let received = remote.len();
        let mut seen = BTreeSet::new();
        let mut merged: Vec<InstalledAddon> = Vec::new();
        for (url, enabled) in remote {
            let url = nuvio_core::url::ensure_manifest_suffix(&url);
            if seen.insert(url.clone()) {
                let summary = local.iter().find(|a| a.manifest_url == url).and_then(|a| a.summary.clone());
                merged.push(InstalledAddon { manifest_url: url, enabled, summary });
            }
        }
        let mut queued = 0;
        if !p.addons_linked {
            for a in &local {
                if seen.insert(a.manifest_url.clone()) {
                    merged.push(a.clone());
                    queued += 1;
                }
            }
        }
        let changed = merged != local;
        if changed {
            self.write_addons(profile, merged)?;
        }
        self.update_sync_state(|s| {
            s.revision += 1;
            let rev = s.revision;
            let p = s.profiles.entry(profile).or_default();
            if queued > 0 {
                p.addons_dirty = rev;
            }
            p.addons_linked = true;
        })?;
        Ok(MergeStats { received, changed, queued_local: queued })
    }

    // ---------------- profiles

    /// The account's profiles (`sync_pull_profiles`). Skipped while a local change is pending (sent
    /// first) — except on the first link: local-only profiles (and their data) are kept and the
    /// joined list queued for upload. The active profile stays selected when it still exists.
    pub fn apply_profiles_remote(&self, remote: Vec<Profile>) -> Result<MergeStats, StoreError> {
        let _g = crate::profiles::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let state = self.sync_state()?;
        if state.profiles_linked && (state.profiles_dirty > 0 || !state.profile_deletes.is_empty()) {
            return Ok(MergeStats::default());
        }
        let local = self.profiles()?;
        let received = remote.len();
        let mut merged: BTreeMap<i64, Profile> = remote.into_iter().filter(|p| (1..=crate::profiles::MAX_PROFILES).contains(&p.index)).map(|p| (p.index, p)).collect();
        let mut queued = 0;
        if !state.profiles_linked {
            for p in &local.profiles {
                if let std::collections::btree_map::Entry::Vacant(slot) = merged.entry(p.index) {
                    slot.insert(p.clone());
                    queued += 1;
                }
            }
        }
        if merged.is_empty() {
            // An account without profiles: keep the local list and send it.
            for p in &local.profiles {
                merged.insert(p.index, p.clone());
                queued += 1;
            }
        }
        let mut new = local.clone();
        new.profiles = merged.into_values().collect();
        if new.get(new.active).is_none() {
            new.active = new.profiles.first().map_or(1, |p| p.index);
        }
        let changed = new != local;
        if changed {
            self.write_profiles(&new)?;
        }
        self.update_sync_state(|s| {
            s.revision += 1;
            if queued > 0 {
                s.profiles_dirty = s.revision;
            }
            s.profiles_linked = true;
        })?;
        Ok(MergeStats { received, changed, queued_local: queued })
    }

    /// Records the account link (first sync done for `user_id`).
    pub fn set_linked_user(&self, user_id: Option<String>) -> Result<(), StoreError> {
        self.update_sync_state(|s| s.linked_user_id = user_id)
    }

    /// Forgets the account cursors and link (another account): the next sync starts with a first
    /// link again. Pending local changes are kept.
    pub fn reset_account_link(&self, user_id: &str) -> Result<(), StoreError> {
        self.update_sync_state(|s| {
            s.linked_user_id = Some(user_id.to_string());
            s.profiles_linked = false;
            for p in s.profiles.values_mut() {
                p.library = Cursor::default();
                p.progress = Cursor::default();
                p.watched = Cursor::default();
                p.addons_linked = false;
            }
        })
    }
}

/// Favorite key from the account fields.
pub fn library_key(content_type: &str, content_id: &str) -> String {
    favorite_key(content_type, content_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> AppStore {
        let dir = std::env::temp_dir().join(format!("nuvio-sync-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        AppStore::open(dir).unwrap()
    }

    fn item(id: &str, at: i64) -> LibraryItem {
        serde_json::from_value(serde_json::json!({ "id": id, "contentType": "movie", "name": id, "savedAtEpochMs": at })).unwrap()
    }

    fn rp(id: &str, pos: i64, at: i64) -> RemoteProgress {
        RemoteProgress { progress_key: id.into(), content_id: id.into(), content_type: "movie".into(), video_id: id.into(), position: pos, duration: 6_000_000, last_watched: at, ..Default::default() }
    }

    #[test]
    fn local_changes_are_queued_per_profile_with_revisions() {
        let s = store("queue");
        s.toggle_favorite(1, item("tt1", 0)).unwrap();
        s.toggle_favorite(2, item("tt2", 0)).unwrap();
        s.toggle_favorite(1, item("tt1", 0)).unwrap(); // removed again
        let st = s.sync_state().unwrap();
        assert!(st.profiles[&1].library_upserts.is_empty());
        assert!(st.profiles[&1].library_deletes.contains_key("movie/tt1"));
        assert!(st.profiles[&2].library_upserts.contains_key("movie/tt2"));
        // A push clears only what it sent.
        let (sent, data) = s.pending_for(2).unwrap();
        assert_eq!(data.library_upserts.len(), 1);
        s.remove_favorite(2, "movie", "tt2").unwrap(); // changed while the push runs
        s.toggle_favorite(2, item("tt2", 0)).unwrap();
        s.clear_sent(2, &sent).unwrap();
        assert!(s.sync_state().unwrap().profiles[&2].library_upserts.contains_key("movie/tt2"), "newer change kept");
    }

    #[test]
    fn library_snapshot_first_link_keeps_both_origins_and_pending_wins() {
        let s = store("lib");
        s.import_library(1, vec![item("local-only", 5), item("both", 5)]).unwrap();
        s.toggle_favorite(1, item("pending", 6)).unwrap();
        s.remove_favorite(1, "movie", "both").unwrap(); // pending delete
        let stats = s.apply_library_snapshot(1, vec![item("both", 1), item("remote-only", 1), item("pending", 1)], 40).unwrap();
        let keys: BTreeSet<String> = s.favorites(1).unwrap().iter().map(|i| i.key()).collect();
        assert_eq!(keys, ["movie/local-only", "movie/pending", "movie/remote-only"].iter().map(|s| s.to_string()).collect());
        assert_eq!(stats.queued_local, 1, "local-only queued for upload");
        let st = s.sync_state().unwrap().profiles[&1].clone();
        assert!(st.library.initialized && st.library.event_id == 40);
        assert!(st.library_upserts.contains_key("movie/local-only"));
        // Delta: pending keys are skipped; deletes from another device apply; cursor moves.
        s.apply_library_delta(1, vec![
            RemoteEvent { event_id: 42, operation: "delete".into(), item: item("remote-only", 0) },
            RemoteEvent { event_id: 41, operation: "upsert".into(), item: item("new", 0) },
            RemoteEvent { event_id: 43, operation: "delete".into(), item: item("pending", 0) },
        ]).unwrap();
        let keys: BTreeSet<String> = s.favorites(1).unwrap().iter().map(|i| i.key()).collect();
        assert!(keys.contains("movie/new") && keys.contains("movie/pending") && !keys.contains("movie/remote-only"));
        assert_eq!(s.sync_state().unwrap().profiles[&1].library.event_id, 43);
        // Later snapshots (not first link) no longer add local-only items as new uploads.
        let st2 = s.apply_library_snapshot(1, vec![], 50).unwrap();
        assert_eq!(st2.queued_local, 0);
    }

    #[test]
    fn progress_rules_keep_the_fresher_dirty_entry() {
        let s = store("prog");
        let mine = rp("tt1", 900_000, 2_000).to_entry(None, 90);
        s.record_progress(1, mine, None).unwrap(); // dirty, newer
        s.apply_progress_snapshot(1, vec![rp("tt1", 100_000, 1_000), rp("tt2", 50_000, 1_000)], 7).unwrap();
        let p = s.progress(1).unwrap();
        assert_eq!(p.iter().find(|e| e.key() == "tt1").unwrap().last_position_ms, 900_000, "dirty and fresher kept");
        assert!(p.iter().any(|e| e.key() == "tt2"));
        // Delta: older upsert ignored for the dirty key; a delete never removes a dirty entry.
        s.apply_progress_delta(1, vec![
            RemoteEvent { event_id: 8, operation: "upsert".into(), item: rp("tt1", 10, 1_500) },
            RemoteEvent { event_id: 9, operation: "delete".into(), item: rp("tt1", 0, 0) },
            RemoteEvent { event_id: 10, operation: "upsert".into(), item: rp("tt2", 70_000, 3_000) },
        ]).unwrap();
        let p = s.progress(1).unwrap();
        assert_eq!(p.iter().find(|e| e.key() == "tt1").unwrap().last_position_ms, 900_000);
        assert_eq!(p.iter().find(|e| e.key() == "tt2").unwrap().last_position_ms, 70_000);
        // A newer remote position replaces a dirty older one (freshness rule of the original).
        s.apply_progress_delta(1, vec![RemoteEvent { event_id: 11, operation: "upsert".into(), item: rp("tt1", 950_000, 9_000) }]).unwrap();
        assert_eq!(s.progress(1).unwrap().iter().find(|e| e.key() == "tt1").unwrap().last_position_ms, 950_000);
        assert!(!s.sync_state().unwrap().profiles[&1].progress_dirty.contains_key("tt1"), "dirty cleared by the newer remote");
    }

    #[test]
    fn profiles_and_addons_first_link_union_then_account_wins() {
        let s = store("prof");
        s.create_profile("Local", "#1E88E5", false).unwrap().unwrap();
        s.update_sync_state(|st| st.profiles_dirty = 0).unwrap();
        let remote = vec![Profile { index: 1, name: "Conta".into(), avatar_id: Some("avatar_x".into()), ..Profile::default() }];
        let stats = s.apply_profiles_remote(remote.clone()).unwrap();
        let names: Vec<_> = s.profiles().unwrap().profiles.iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, vec!["Conta", "Local"]);
        assert_eq!(stats.queued_local, 1);
        assert!(s.sync_state().unwrap().profiles_dirty > 0, "local-only profile queued");
        s.update_sync_state(|st| st.profiles_dirty = 0).unwrap();
        s.apply_profiles_remote(remote).unwrap();
        assert_eq!(s.profiles().unwrap().profiles.len(), 1, "after the link the account list is taken as is");

        s.install_addon(1, "https://local.test/manifest.json", Default::default()).unwrap().unwrap();
        s.update_sync_state(|st| st.profiles.entry(1).or_default().addons_dirty = 0).unwrap();
        s.apply_addons_remote(1, vec![("https://remote.test".into(), false)]).unwrap();
        let urls: Vec<_> = s.addons(1).unwrap().iter().map(|a| (a.manifest_url.clone(), a.enabled)).collect();
        assert_eq!(urls, vec![("https://remote.test/manifest.json".to_string(), false), ("https://local.test/manifest.json".to_string(), true)]);
        // A pending local change blocks the remote list until it is sent.
        s.set_addon_enabled(1, &crate::profiles::addon_key("https://local.test/manifest.json"), false).unwrap().unwrap();
        let skipped = s.apply_addons_remote(1, vec![]).unwrap();
        assert_eq!(skipped.received, 0);
        assert_eq!(s.addons(1).unwrap().len(), 2);
    }
}
