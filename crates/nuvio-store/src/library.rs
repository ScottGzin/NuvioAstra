//! Library (saved movies/series), watch progress and watched marks, per profile.
//!
//! Port of the original's local mode:
//! - `LibraryRepository` / `LibraryItem` (store `nuvio_library`, key `library_<profile>`): an item is
//!   identified by `(type, id)` exactly as the addon gave them (`localState.contains(id, type)`).
//! - `WatchProgressRepository` / `WatchProgressEntry` (store `nuvio_watch_progress`, key
//!   `watch_progress_<profile>`): one entry per movie or episode, identified by its progress key
//!   `<parentMetaId>` or `<parentMetaId>_s<season>e<episode>` (`buildWatchProgressKey`).
//! - `WatchingPolicies`: progress is stored once the position reaches 1 s and the duration is not a
//!   placeholder clip (< 121 s); an entry is complete when playback ended or reached 90 %.
//! - `SeriesContinuity.continueWatchingProgressEntries`: latest entry per series, movies not
//!   completed, newest first, at most 20.
//! - `WatchedRepository` / `WatchedItem` (store `nuvio_watched`, key `watched_<profile>`): a
//!   completed entry also marks the movie/episode as watched (`WatchingActions.onProgressEntryUpdated`).
//!
//! NextGen keeps each profile in its own documents (`library-<p>.json`, `progress-<p>.json`,
//! `watched-<p>.json`), written atomically like every NextGen document. Stream URLs, tracking ids
//! and anything that could carry a token are not stored (the original's `lastSourceUrl` is dropped).

use crate::nextgen::AppStore;
use crate::sync_state::Change;
use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// The original's fixed rule, and the default of "Considerar assistido".
pub const COMPLETION_PERCENT: i64 = 90;
/// With 100 %, the last frames: a file rarely reports exactly its duration.
const END_TOLERANCE: f64 = 0.0005;
pub const PROGRESS_STORE_THRESHOLD_MS: i64 = 1_000;
pub const MIN_REAL_CONTENT_DURATION_MS: i64 = 121_000;
pub const CONTINUE_WATCHING_LIMIT: usize = 20;
/// `PlaybackProgressPersistIntervalMs`: periodic save while playing.
pub const PROGRESS_PERSIST_INTERVAL_MS: i64 = 60_000;
/// `PlayerSeekProgressSyncDebounceMs`: save after a seek settles.
pub const SEEK_SAVE_DEBOUNCE_MS: i64 = 700;

/// Serializes read-modify-write of the library documents (UI commands and the progress writer).
pub(crate) static LOCK: Mutex<()> = Mutex::new(());

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// `isSeriesLikeWatchingContentType`.
pub fn is_series_like(content_type: &str) -> bool {
    matches!(content_type.trim().to_ascii_lowercase().as_str(), "series" | "tv" | "show" | "tvshow")
}

/// `isShortPlaceholderDuration`: a known duration under 121 s is an error/placeholder clip.
pub fn is_short_placeholder(duration_ms: i64) -> bool {
    (1..MIN_REAL_CONTENT_DURATION_MS).contains(&duration_ms)
}

/// `shouldStoreProgress`.
pub fn should_store(position_ms: i64, duration_ms: i64) -> bool {
    !is_short_placeholder(duration_ms) && position_ms >= PROGRESS_STORE_THRESHOLD_MS
}

/// `isProgressComplete`: ended, or at least `percent` watched (90 in the original, the profile's
/// choice here). Never for placeholder clips; closing the player is not "ended".
pub fn is_complete(position_ms: i64, duration_ms: i64, ended: bool, percent: i64) -> bool {
    if is_short_placeholder(duration_ms) {
        return false;
    }
    if ended {
        return true;
    }
    if duration_ms <= 0 {
        return false;
    }
    let needed = if percent >= 100 { 1.0 - END_TOLERANCE } else { percent as f64 / 100.0 };
    position_ms as f64 / duration_ms as f64 >= needed
}

/// `buildWatchProgressKey`.
pub fn progress_key(parent_meta_id: &str, season: Option<i32>, episode: Option<i32>) -> String {
    match (season, episode) {
        (Some(s), Some(e)) => format!("{}_s{s}e{e}", parent_meta_id.trim()),
        _ => parent_meta_id.trim().to_string(),
    }
}

fn poster_shape() -> String {
    "Poster".into()
}

fn completion_percent() -> i64 {
    COMPLETION_PERCENT
}

/// A saved movie/series (`LibraryItem`, display fields only). Reads the original's JSON too
/// (`type` → `contentType`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub id: String,
    #[serde(alias = "type")]
    pub content_type: String,
    pub name: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub release_info: Option<String>,
    #[serde(default)]
    pub imdb_rating: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default = "poster_shape")]
    pub poster_shape: String,
    #[serde(default)]
    pub saved_at_epoch_ms: i64,
    /// Addon the item came from (account sync field `addon_base_url`), kept as received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addon_base_url: Option<String>,
}

impl LibraryItem {
    pub fn key(&self) -> String {
        favorite_key(&self.content_type, &self.id)
    }
}

/// Identity of a saved item: the addon's type and id (same id with another type is another item).
pub fn favorite_key(content_type: &str, id: &str) -> String {
    format!("{}/{}", content_type.trim(), id.trim())
}

/// One movie or episode with its last position (`WatchProgressEntry` without stream URLs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEntry {
    pub content_type: String,
    pub parent_meta_id: String,
    pub parent_meta_type: String,
    /// Playback identity (the id the streams were asked with).
    pub video_id: String,
    pub title: String,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub season_number: Option<i32>,
    #[serde(default)]
    pub episode_number: Option<i32>,
    #[serde(default)]
    pub episode_title: Option<String>,
    #[serde(default)]
    pub episode_thumbnail: Option<String>,
    pub last_position_ms: i64,
    pub duration_ms: i64,
    pub last_updated_epoch_ms: i64,
    /// Addon that served the stream (display name only).
    #[serde(default)]
    pub provider_name: Option<String>,
    #[serde(default)]
    pub last_stream_title: Option<String>,
    #[serde(default)]
    pub is_completed: bool,
    #[serde(default)]
    pub progress_percent: Option<f32>,
    #[serde(default)]
    pub progress_key: Option<String>,
    /// "Considerar assistido" of the profile it was read for. Never stored: positions are what is
    /// kept and synced, so changing the setting also reclassifies what was watched before.
    #[serde(skip, default = "completion_percent")]
    pub watched_percent: i64,
}

impl ProgressEntry {
    pub fn key(&self) -> String {
        self.progress_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .unwrap_or_else(|| progress_key(&self.parent_meta_id, self.season_number, self.episode_number))
    }

    fn percent(&self) -> Option<f32> {
        self.progress_percent.map(|p| p.clamp(0.0, 100.0))
    }

    /// `isEffectivelyCompleted`.
    pub fn effectively_completed(&self) -> bool {
        if let Some(percent) = self.percent() {
            return self.is_completed || percent >= self.watched_percent as f32;
        }
        // A known position decides alone: finishing stores the position at the end, and a flag
        // computed with the original's 90 % must not outweigh a higher choice.
        if self.last_position_ms > 0 && self.duration_ms >= MIN_REAL_CONTENT_DURATION_MS {
            return is_complete(self.last_position_ms, self.duration_ms, false, self.watched_percent);
        }
        self.is_completed
    }

    /// `normalizedCompletion` + `withResolvedProgressKey` (position kept as reported).
    pub fn normalized(mut self) -> Self {
        let completed = self.effectively_completed();
        self.last_position_ms = self.last_position_ms.max(0);
        if self.percent().is_none() && completed && self.duration_ms <= 0 {
            self.progress_percent = Some(100.0);
        }
        self.is_completed = completed;
        self.progress_key = Some(self.key());
        self
    }

    pub fn fraction(&self) -> f32 {
        if let Some(p) = self.percent() {
            return p / 100.0;
        }
        if self.duration_ms > 0 {
            (self.last_position_ms as f32 / self.duration_ms as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// `shouldTreatAsInProgressForContinueWatching` (local source).
    pub fn in_progress(&self) -> bool {
        !self.effectively_completed() && (self.last_position_ms > 0 || self.percent().is_some_and(|p| p > 0.0))
    }

    fn is_episode_like(&self) -> bool {
        is_series_like(&self.parent_meta_type) || (self.season_number.is_some() && self.episode_number.is_some())
    }

    /// `resolveResumePosition` with the stored duration.
    pub fn resume_position_ms(&self) -> i64 {
        if self.last_position_ms > 0 || self.duration_ms > 0 {
            return self.last_position_ms.max(0);
        }
        self.percent().map_or(0, |p| (self.duration_ms as f32 * p / 100.0) as i64)
    }
}

/// `watchingProgressRecencyComparator` (newest last).
fn recency(a: &ProgressEntry, b: &ProgressEntry) -> std::cmp::Ordering {
    (a.last_updated_epoch_ms, a.season_number.unwrap_or(0), a.episode_number.unwrap_or(0), a.last_position_ms, a.effectively_completed())
        .cmp(&(b.last_updated_epoch_ms, b.season_number.unwrap_or(0), b.episode_number.unwrap_or(0), b.last_position_ms, b.effectively_completed()))
        .then_with(|| a.key().cmp(&b.key()))
}

/// Keeps the newest entry of each progress key (`newestByProgressKey`), newest first.
pub fn dedupe_progress(entries: impl IntoIterator<Item = ProgressEntry>) -> Vec<ProgressEntry> {
    let mut by_key: std::collections::HashMap<String, ProgressEntry> = std::collections::HashMap::new();
    for entry in entries {
        let entry = entry.normalized();
        let key = entry.key();
        match by_key.get(&key) {
            Some(existing) if recency(existing, &entry).is_ge() => {}
            _ => {
                by_key.insert(key, entry);
            }
        }
    }
    let mut out: Vec<ProgressEntry> = by_key.into_values().collect();
    out.sort_by(|a, b| recency(b, a));
    out
}

/// `continueWatchingEntries`: one entry per series (its latest; dropped when that one is completed),
/// movies not completed, newest first, at most `limit`.
pub fn continue_watching(entries: &[ProgressEntry], limit: usize) -> Vec<ProgressEntry> {
    let selection: Vec<&ProgressEntry> = entries.iter().filter(|e| e.effectively_completed() || e.in_progress()).collect();
    let mut latest_per_series: std::collections::HashMap<String, &ProgressEntry> = std::collections::HashMap::new();
    let mut picked: Vec<&ProgressEntry> = Vec::new();
    for entry in selection {
        if entry.is_episode_like() {
            let slot = latest_per_series.entry(entry.parent_meta_id.trim().to_string()).or_insert(entry);
            if recency(slot, entry).is_lt() {
                *slot = entry;
            }
        } else {
            picked.push(entry);
        }
    }
    picked.extend(latest_per_series.into_values());
    let mut out: Vec<ProgressEntry> = picked.into_iter().filter(|e| e.in_progress()).cloned().collect();
    out.sort_by(|a, b| b.last_updated_epoch_ms.cmp(&a.last_updated_epoch_ms));
    out.truncate(limit);
    out
}

/// `resumeProgressForSeries` / movie resume: the newest entry of a title unless it is completed.
pub fn resume_for(entries: &[ProgressEntry], meta_id: &str) -> Option<ProgressEntry> {
    entries
        .iter()
        .filter(|e| e.parent_meta_id.trim() == meta_id.trim())
        .max_by(|a, b| recency(a, b))
        .filter(|e| !e.effectively_completed())
        .cloned()
}

/// A movie or episode marked as watched (`WatchedItem`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchedItem {
    pub id: String,
    #[serde(alias = "type")]
    pub content_type: String,
    pub name: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub release_info: Option<String>,
    #[serde(default)]
    pub season: Option<i32>,
    #[serde(default)]
    pub episode: Option<i32>,
    #[serde(default)]
    pub video_id: Option<String>,
    pub marked_at_epoch_ms: i64,
}

impl WatchedItem {
    /// `watchedItemKey` with the type aliases folded (`watchedItemTypeAliases`).
    pub fn key(&self) -> String {
        watched_key(&self.content_type, &self.id, self.season, self.episode)
    }
}

pub fn watched_key(content_type: &str, id: &str, season: Option<i32>, episode: Option<i32>) -> String {
    let kind = if is_series_like(content_type) || content_type.eq_ignore_ascii_case("anime") { "series" } else { content_type.trim() };
    format!("{kind}:{}:{}:{}", id.trim(), season.unwrap_or(-1), episode.unwrap_or(-1))
}

pub fn dedupe_watched(items: impl IntoIterator<Item = WatchedItem>) -> Vec<WatchedItem> {
    let mut by_key: std::collections::HashMap<String, WatchedItem> = std::collections::HashMap::new();
    for item in items {
        match by_key.get(&item.key()) {
            Some(existing) if existing.marked_at_epoch_ms >= item.marked_at_epoch_ms => {}
            _ => {
                by_key.insert(item.key(), item);
            }
        }
    }
    let mut out: Vec<WatchedItem> = by_key.into_values().collect();
    out.sort_by(|a, b| b.marked_at_epoch_ms.cmp(&a.marked_at_epoch_ms).then_with(|| a.key().cmp(&b.key())));
    out
}

pub fn dedupe_library(items: impl IntoIterator<Item = LibraryItem>) -> Vec<LibraryItem> {
    let mut seen = std::collections::HashSet::new();
    let mut items: Vec<LibraryItem> = items.into_iter().collect();
    items.sort_by(|a, b| b.saved_at_epoch_ms.cmp(&a.saved_at_epoch_ms));
    items.retain(|i| seen.insert(i.key()));
    items
}

/// One line of the playback history: a movie or episode that was played (progress) or marked
/// watched, newest first.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// A next episode (not started), shown after the previous one was finished.
    pub next_up: bool,
    /// Release date of a next episode that is not out yet.
    pub airs: Option<String>,
    /// Progress key of the movie/episode (used to remove it).
    pub key: String,
    pub meta_id: String,
    pub meta_type: String,
    pub video_id: Option<String>,
    pub title: String,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub episode_title: Option<String>,
    /// The episode's own picture (Continue watching can show it instead of the series art).
    pub episode_thumbnail: Option<String>,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub fraction: f32,
    pub completed: bool,
    pub in_progress: bool,
    pub last_watched_epoch_ms: i64,
}

impl HistoryEntry {
    pub fn from_progress(e: &ProgressEntry) -> Self {
        Self {
            next_up: false,
            airs: None,
            key: e.key(),
            meta_id: e.parent_meta_id.clone(),
            meta_type: e.parent_meta_type.clone(),
            video_id: Some(e.video_id.clone()),
            title: e.title.clone(),
            poster: e.poster.clone(),
            background: e.background.clone(),
            season: e.season_number,
            episode: e.episode_number,
            episode_title: e.episode_title.clone(),
            episode_thumbnail: e.episode_thumbnail.clone(),
            position_ms: e.resume_position_ms(),
            duration_ms: e.duration_ms,
            fraction: if e.effectively_completed() { 1.0 } else { e.fraction() },
            completed: e.effectively_completed(),
            in_progress: e.in_progress(),
            last_watched_epoch_ms: e.last_updated_epoch_ms,
        }
    }

    fn from_watched(w: &WatchedItem) -> Self {
        Self {
            next_up: false,
            airs: None,
            key: progress_key(&w.id, w.season, w.episode),
            meta_id: w.id.clone(),
            meta_type: w.content_type.clone(),
            video_id: w.video_id.clone(),
            title: w.name.clone(),
            poster: w.poster.clone(),
            background: None,
            season: w.season,
            episode: w.episode,
            episode_title: None,
            episode_thumbnail: None,
            position_ms: 0,
            duration_ms: 0,
            fraction: 1.0,
            completed: true,
            in_progress: false,
            last_watched_epoch_ms: w.marked_at_epoch_ms,
        }
    }
}

/// Progress of every movie/episode, then watched marks without progress; newest first.
pub fn history(progress: &[ProgressEntry], watched: &[WatchedItem]) -> Vec<HistoryEntry> {
    let mut out: Vec<HistoryEntry> = progress.iter().map(HistoryEntry::from_progress).collect();
    let keys: std::collections::HashSet<String> = out.iter().map(|h| h.key.clone()).collect();
    out.extend(watched.iter().filter(|w| !keys.contains(&progress_key(&w.id, w.season, w.episode))).map(HistoryEntry::from_watched));
    out.sort_by(|a, b| b.last_watched_epoch_ms.cmp(&a.last_watched_epoch_ms).then_with(|| a.key.cmp(&b.key)));
    out
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LibraryDoc {
    #[serde(default)]
    items: Vec<LibraryItem>,
}
#[derive(Debug, Default, Serialize, Deserialize)]
struct ProgressDoc {
    #[serde(default)]
    entries: Vec<ProgressEntry>,
}
#[derive(Debug, Default, Serialize, Deserialize)]
struct WatchedDoc {
    #[serde(default)]
    items: Vec<WatchedItem>,
}

fn library_doc(profile: i64) -> String {
    format!("library-{profile}")
}
fn progress_doc(profile: i64) -> String {
    format!("progress-{profile}")
}
fn watched_doc(profile: i64) -> String {
    format!("watched-{profile}")
}

/// What a playback reports to be saved (`WatchProgressPlaybackSession` without stream URLs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackItem {
    pub content_type: String,
    pub parent_meta_id: String,
    pub parent_meta_type: String,
    pub video_id: String,
    pub title: String,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub season_number: Option<i32>,
    #[serde(default)]
    pub episode_number: Option<i32>,
    #[serde(default)]
    pub episode_title: Option<String>,
    #[serde(default)]
    pub episode_thumbnail: Option<String>,
    #[serde(default)]
    pub release_info: Option<String>,
    #[serde(default)]
    pub provider_name: Option<String>,
    #[serde(default)]
    pub last_stream_title: Option<String>,
}

impl PlaybackItem {
    /// `WatchProgressRepository.upsert`: `None` when the position is not worth storing yet.
    pub fn entry(&self, position_ms: i64, duration_ms: i64, ended: bool, at_epoch_ms: i64, watched_percent: i64) -> Option<ProgressEntry> {
        let position_ms = position_ms.max(0);
        let duration_ms = duration_ms.max(0);
        let completed = is_complete(position_ms, duration_ms, ended, watched_percent);
        if !completed && !should_store(position_ms, duration_ms) {
            return None;
        }
        Some(
            ProgressEntry {
                content_type: self.content_type.clone(),
                parent_meta_id: self.parent_meta_id.clone(),
                parent_meta_type: self.parent_meta_type.clone(),
                video_id: self.video_id.clone(),
                title: self.title.clone(),
                logo: self.logo.clone(),
                poster: self.poster.clone(),
                background: self.background.clone(),
                season_number: self.season_number,
                episode_number: self.episode_number,
                episode_title: self.episode_title.clone(),
                episode_thumbnail: self.episode_thumbnail.clone(),
                last_position_ms: if completed && duration_ms > 0 { duration_ms } else { position_ms },
                duration_ms,
                last_updated_epoch_ms: at_epoch_ms,
                provider_name: self.provider_name.clone(),
                last_stream_title: self.last_stream_title.clone(),
                is_completed: completed,
                progress_percent: None,
                progress_key: None,
                watched_percent,
            }
            .normalized(),
        )
    }
}

impl AppStore {
    pub fn favorites(&self, profile: i64) -> Result<Vec<LibraryItem>, StoreError> {
        Ok(self.read::<LibraryDoc>(&library_doc(profile))?.unwrap_or_default().items)
    }

    /// Saves or removes an item (`toggleLocalSaved`); returns whether it is saved now.
    pub fn toggle_favorite(&self, profile: i64, mut item: LibraryItem) -> Result<bool, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut items = self.favorites(profile)?;
        let key = item.key();
        let saved = if let Some(i) = items.iter().position(|x| x.key() == key) {
            items.remove(i);
            false
        } else {
            item.saved_at_epoch_ms = now_ms();
            items.insert(0, item);
            true
        };
        self.write(&library_doc(profile), &LibraryDoc { items })?;
        self.note(profile, if saved { Change::FavoriteSaved(&key) } else { Change::FavoriteRemoved(&key) })?;
        Ok(saved)
    }

    pub fn remove_favorite(&self, profile: i64, content_type: &str, id: &str) -> Result<bool, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut items = self.favorites(profile)?;
        let key = favorite_key(content_type, id);
        let before = items.len();
        items.retain(|x| x.key() != key);
        if items.len() == before {
            return Ok(false);
        }
        self.write(&library_doc(profile), &LibraryDoc { items })?;
        self.note(profile, Change::FavoriteRemoved(&key))?;
        Ok(true)
    }

    pub fn progress(&self, profile: i64) -> Result<Vec<ProgressEntry>, StoreError> {
        let percent = self.watched_percent(profile);
        let mut entries = self.read::<ProgressDoc>(&progress_doc(profile))?.unwrap_or_default().entries;
        for entry in &mut entries {
            entry.watched_percent = percent;
            entry.is_completed = entry.effectively_completed();
        }
        Ok(entries)
    }

    pub fn watched(&self, profile: i64) -> Result<Vec<WatchedItem>, StoreError> {
        Ok(self.read::<WatchedDoc>(&watched_doc(profile))?.unwrap_or_default().items)
    }

    /// "Considerar assistido" of a profile (the original's 90 % when the settings cannot be read).
    pub fn watched_percent(&self, profile: i64) -> i64 {
        self.profile_settings(profile).map_or(COMPLETION_PERCENT, |s| s.player.watched_percent)
    }

    /// Stores one playback position. An older report than the stored one is ignored (saves may
    /// arrive out of order from the background writer). A completed entry marks the movie/episode
    /// watched, like the original. Returns the stored entry, if any.
    pub fn record_progress(&self, profile: i64, entry: ProgressEntry, release_info: Option<String>) -> Result<Option<ProgressEntry>, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entry.normalized();
        let key = entry.key();
        let mut entries = self.progress(profile)?;
        let entry = ProgressEntry { watched_percent: self.watched_percent(profile), ..entry }.normalized();
        if let Some(i) = entries.iter().position(|e| e.key() == key) {
            if entries[i].last_updated_epoch_ms > entry.last_updated_epoch_ms {
                return Ok(None);
            }
            entries.remove(i);
        }
        entries.insert(0, entry.clone());
        self.write(&progress_doc(profile), &ProgressDoc { entries })?;
        let mut completed = None;
        if entry.is_completed {
            let mut watched = self.watched(profile)?;
            let mark = WatchedItem {
                id: entry.parent_meta_id.clone(),
                content_type: entry.parent_meta_type.clone(),
                name: entry.title.clone(),
                poster: entry.poster.clone(),
                release_info,
                season: entry.season_number,
                episode: entry.episode_number,
                video_id: Some(entry.video_id.clone()),
                marked_at_epoch_ms: entry.last_updated_epoch_ms,
            };
            watched.push(mark.clone());
            completed = Some(mark);
            self.write(&watched_doc(profile), &WatchedDoc { items: dedupe_watched(watched) })?;
        }
        self.note(profile, Change::Progress { key: &key, completed })?;
        Ok(Some(entry))
    }

    /// Marks movies/episodes as watched by hand (`markWatched`): their unfinished progress leaves
    /// Continue watching. Returns how many were not marked before.
    pub fn mark_watched(&self, profile: i64, items: Vec<WatchedItem>) -> Result<usize, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut watched = self.watched(profile)?;
        let known: std::collections::HashSet<String> = watched.iter().map(WatchedItem::key).collect();
        let added: Vec<WatchedItem> = items.into_iter().filter(|w| !known.contains(&w.key())).collect();
        if added.is_empty() {
            return Ok(0);
        }
        let keys: std::collections::HashSet<String> = added.iter().map(|w| progress_key(&w.id, w.season, w.episode)).collect();
        let mut entries = self.progress(profile)?;
        let removed: Vec<String> = entries.iter().filter(|e| keys.contains(&e.key()) && !e.is_completed).map(ProgressEntry::key).collect();
        if !removed.is_empty() {
            entries.retain(|e| !removed.contains(&e.key()));
            self.write(&progress_doc(profile), &ProgressDoc { entries })?;
        }
        watched.extend(added.iter().cloned());
        self.write(&watched_doc(profile), &WatchedDoc { items: dedupe_watched(watched) })?;
        let count = added.len();
        self.note(profile, Change::Watched { marked: added, unmarked: Vec::new(), progress_removed: removed })?;
        Ok(count)
    }

    /// Unmarks movies/episodes (`unmarkWatched`), and drops their finished progress so they stop
    /// counting as watched. Returns how many were removed.
    pub fn unmark_watched(&self, profile: i64, keys: &[(String, Option<i32>, Option<i32>)]) -> Result<usize, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let wanted: std::collections::HashSet<String> = keys.iter().map(|(id, s, e)| progress_key(id, *s, *e)).collect();
        let watched = self.watched(profile)?;
        let (removed, kept): (Vec<WatchedItem>, Vec<WatchedItem>) = watched.into_iter().partition(|w| wanted.contains(&progress_key(&w.id, w.season, w.episode)));
        let mut entries = self.progress(profile)?;
        let finished: Vec<String> = entries.iter().filter(|e| wanted.contains(&e.key()) && e.is_completed).map(ProgressEntry::key).collect();
        if removed.is_empty() && finished.is_empty() {
            return Ok(0);
        }
        if !finished.is_empty() {
            entries.retain(|e| !finished.contains(&e.key()));
            self.write(&progress_doc(profile), &ProgressDoc { entries })?;
        }
        self.write(&watched_doc(profile), &WatchedDoc { items: kept })?;
        let count = removed.len().max(finished.len());
        self.note(profile, Change::Watched { marked: Vec::new(), unmarked: removed, progress_removed: finished })?;
        Ok(count)
    }

    /// Removes a movie/episode from the history: its progress (`removeProgress`) and its watched
    /// mark (`unmarkWatched`). Returns whether anything was removed.
    pub fn remove_history(&self, profile: i64, key: &str) -> Result<bool, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.progress(profile)?;
        let before = entries.len();
        entries.retain(|e| e.key() != key);
        let removed_progress = entries.len() != before;
        if removed_progress {
            self.write(&progress_doc(profile), &ProgressDoc { entries })?;
        }
        let watched = self.watched(profile)?;
        let (removed, kept): (Vec<WatchedItem>, Vec<WatchedItem>) = watched.into_iter().partition(|w| progress_key(&w.id, w.season, w.episode) == key);
        let removed_watched = !removed.is_empty();
        if removed_watched {
            self.write(&watched_doc(profile), &WatchedDoc { items: kept })?;
        }
        if removed_progress || removed_watched {
            self.note(profile, Change::HistoryRemoved { progress: removed_progress.then_some(key), watched: removed })?;
        }
        Ok(removed_progress || removed_watched)
    }

    pub(crate) fn has_library_docs(&self, profile: i64) -> [bool; 3] {
        [library_doc(profile), progress_doc(profile), watched_doc(profile)].map(|n| self.dir().join(format!("{n}.json")).exists())
    }

    pub(crate) fn import_library(&self, profile: i64, items: Vec<LibraryItem>) -> Result<(), StoreError> {
        self.write(&library_doc(profile), &LibraryDoc { items: dedupe_library(items) })
    }
    /// Titles and artwork for entries that came from the account, which keeps only ids and
    /// positions (`enrichWatchProgressEntry`). Not a change to send: positions and dates stay.
    pub fn enrich_progress(&self, profile: i64, meta_id: &str, enrich: impl Fn(&mut ProgressEntry)) -> Result<bool, StoreError> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.progress(profile)?;
        let mut changed = false;
        for entry in entries.iter_mut().filter(|e| e.parent_meta_id == meta_id) {
            let before = entry.clone();
            enrich(entry);
            changed |= *entry != before;
        }
        if changed {
            self.import_progress(profile, entries)?;
        }
        Ok(changed)
    }

    pub(crate) fn import_progress(&self, profile: i64, entries: Vec<ProgressEntry>) -> Result<(), StoreError> {
        self.write(&progress_doc(profile), &ProgressDoc { entries: dedupe_progress(entries) })
    }
    pub(crate) fn import_watched(&self, profile: i64, items: Vec<WatchedItem>) -> Result<(), StoreError> {
        self.write(&watched_doc(profile), &WatchedDoc { items: dedupe_watched(items) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nuvio-nextgen-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn item(kind: &str, id: &str, s: Option<i32>, e: Option<i32>) -> PlaybackItem {
        PlaybackItem {
            content_type: kind.into(),
            parent_meta_id: id.into(),
            parent_meta_type: kind.into(),
            video_id: match (s, e) {
                (Some(s), Some(e)) => format!("{id}:{s}:{e}"),
                _ => id.into(),
            },
            title: format!("Título {id}"),
            logo: None,
            poster: None,
            background: None,
            season_number: s,
            episode_number: e,
            episode_title: None,
            episode_thumbnail: None,
            release_info: None,
            provider_name: Some("Addon".into()),
            last_stream_title: Some("1080p".into()),
        }
    }

    #[test]
    fn rules_match_the_original() {
        assert!(!should_store(999, 3_600_000));
        assert!(should_store(1_000, 3_600_000));
        assert!(should_store(5_000, 0), "unknown duration still stores");
        assert!(!should_store(60_000, 120_000), "placeholder clip");
        assert!(is_complete(3_240_000, 3_600_000, false, 90), "90 %");
        assert!(!is_complete(3_239_000, 3_600_000, false, 90));
        assert!(is_complete(10_000, 3_600_000, true, 90), "ended");
        assert!(!is_complete(119_000, 120_000, true, 90), "placeholder never completes");
        assert!(!is_complete(10_000, 0, false, 90));
        assert!(!is_complete(3_480_000, 3_600_000, false, 97), "97 % is not reached at 96.7 %");
        assert!(is_complete(3_492_000, 3_600_000, false, 97) && !is_complete(3_590_000, 3_600_000, false, 100) && is_complete(3_599_000, 3_600_000, false, 100));
        assert_eq!(progress_key("tt1", Some(2), Some(3)), "tt1_s2e3");
        assert_eq!(progress_key("tt1", None, None), "tt1");
        assert!(item("movie", "tt1", None, None).entry(500, 3_600_000, false, 1, 90).is_none());
        let closed = item("movie", "tt1", None, None).entry(600_000, 3_600_000, false, 1, 90).unwrap();
        assert!(!closed.is_completed, "closing the player does not complete");
        let ended = item("movie", "tt1", None, None).entry(3_590_000, 3_600_000, true, 1, 90).unwrap();
        assert!(ended.is_completed && ended.last_position_ms == 3_600_000);
    }

    #[test]
    fn episodes_keep_separate_progress_and_continue_watching_follows_series_rule() {
        let dir = temp("cw");
        let store = AppStore::open(&dir).unwrap();
        let e1 = item("series", "tt9", Some(1), Some(1)).entry(600_000, 2_400_000, false, 1_000, 90).unwrap();
        let e2 = item("series", "tt9", Some(1), Some(2)).entry(300_000, 2_400_000, false, 2_000, 90).unwrap();
        let movie = item("movie", "tt5", None, None).entry(1_200_000, 6_000_000, false, 1_500, 90).unwrap();
        for e in [e1, e2, movie] {
            store.record_progress(1, e, None).unwrap();
        }
        let all = store.progress(1).unwrap();
        assert_eq!(all.len(), 3);
        let ep1 = all.iter().find(|e| e.key() == "tt9_s1e1").unwrap();
        let ep2 = all.iter().find(|e| e.key() == "tt9_s1e2").unwrap();
        assert_eq!((ep1.last_position_ms, ep2.last_position_ms), (600_000, 300_000), "not mixed");
        let cw = continue_watching(&all, CONTINUE_WATCHING_LIMIT);
        assert_eq!(cw.iter().map(|e| e.key()).collect::<Vec<_>>(), vec!["tt9_s1e2", "tt5"], "latest episode per series, newest first");
        assert_eq!(resume_for(&all, "tt9").unwrap().key(), "tt9_s1e2");

        // Completing the latest episode: the series leaves Continue watching and the episode is watched.
        let done = item("series", "tt9", Some(1), Some(2)).entry(2_200_000, 2_400_000, false, 3_000, 90).unwrap();
        store.record_progress(1, done, None).unwrap();
        let all = store.progress(1).unwrap();
        assert_eq!(continue_watching(&all, 20).iter().map(|e| e.key()).collect::<Vec<_>>(), vec!["tt5"]);
        assert!(resume_for(&all, "tt9").is_none());
        let watched = store.watched(1).unwrap();
        assert_eq!(watched.len(), 1);
        assert_eq!((watched[0].season, watched[0].episode), (Some(1), Some(2)));

        // Replaying the same episode updates its entry (no duplicate) and its date.
        let again = item("series", "tt9", Some(1), Some(1)).entry(900_000, 2_400_000, false, 4_000, 90).unwrap();
        store.record_progress(1, again, None).unwrap();
        let all = store.progress(1).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].key(), "tt9_s1e1");
        assert_eq!(all[0].last_updated_epoch_ms, 4_000);
        // A late (older) save does not overwrite a newer one.
        let stale = item("series", "tt9", Some(1), Some(1)).entry(100_000, 2_400_000, false, 3_500, 90).unwrap();
        assert!(store.record_progress(1, stale, None).unwrap().is_none());
        assert_eq!(store.progress(1).unwrap()[0].last_position_ms, 900_000);

        // History: newest first, removable; other profiles untouched.
        let h = history(&store.progress(1).unwrap(), &store.watched(1).unwrap());
        assert_eq!(h.iter().map(|h| h.key.as_str()).collect::<Vec<_>>(), vec!["tt9_s1e1", "tt9_s1e2", "tt5"]);
        assert!(store.progress(2).unwrap().is_empty());
        assert!(store.remove_history(1, "tt9_s1e2").unwrap());
        assert!(store.watched(1).unwrap().is_empty());
        assert_eq!(store.progress(1).unwrap().len(), 2);
    }

    #[test]
    fn favorites_are_per_profile_and_identified_by_type_and_id() {
        let dir = temp("fav");
        let store = AppStore::open(&dir).unwrap();
        let mk = |kind: &str, id: &str, name: &str| LibraryItem {
            id: id.into(),
            content_type: kind.into(),
            name: name.into(),
            poster: None,
            banner: None,
            logo: None,
            description: None,
            release_info: None,
            imdb_rating: None,
            genres: vec![],
            poster_shape: "Poster".into(),
            saved_at_epoch_ms: 0,
            addon_base_url: None,
        };
        // Same name, different ids (two addons) are two items; same id with another type too.
        assert!(store.toggle_favorite(1, mk("movie", "tt1", "Duna")).unwrap());
        assert!(store.toggle_favorite(1, mk("movie", "tmdb:438631", "Duna")).unwrap());
        assert!(store.toggle_favorite(1, mk("series", "tt1", "Duna")).unwrap());
        assert_eq!(store.favorites(1).unwrap().len(), 3);
        assert!(store.favorites(2).unwrap().is_empty());
        assert!(!store.toggle_favorite(1, mk("movie", "tt1", "Outro nome")).unwrap(), "same type+id toggles off");
        assert_eq!(store.favorites(1).unwrap().len(), 2);
        assert!(store.remove_favorite(1, "series", "tt1").unwrap());
        assert!(!store.remove_favorite(1, "series", "tt1").unwrap());
        assert_eq!(store.favorites(1).unwrap().iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["tmdb:438631"]);
    }

    #[test]
    fn reads_the_original_json_and_drops_stream_urls() {
        let raw = r#"{"contentType":"series","parentMetaId":"tt3","parentMetaType":"series","videoId":"tt3:1:4","title":"S",
            "seasonNumber":1,"episodeNumber":4,"lastPositionMs":1000,"durationMs":2000000,"lastUpdatedEpochMs":5,
            "lastSourceUrl":"https://cdn.test/v.mkv?token=secret","providerAddonId":"addon:x:https://a.test/tok/manifest.json",
            "isCompleted":false,"progressPercent":95.0,"source":"local"}"#;
        let entry: ProgressEntry = serde_json::from_str(raw).unwrap();
        let entry = entry.normalized();
        assert!(entry.is_completed, "95 % from the original counts as completed");
        assert_eq!(entry.key(), "tt3_s1e4");
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("secret") && !json.contains("https://"), "{json}");
        let lib: LibraryItem = serde_json::from_str(r#"{"id":"tt4","type":"series","name":"N","savedAtEpochMs":9,"traktId":1}"#).unwrap();
        assert_eq!(lib.content_type, "series");
        assert_eq!(serde_json::to_value(&lib).unwrap()["contentType"], "series");
    }
}
