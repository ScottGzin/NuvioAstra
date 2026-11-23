//! "Continue watching" goes on with the next episode (`SeriesContinuity.kt` and the next-up part of
//! `HomeScreen.kt`): the furthest finished episode of each series is the seed, the next one that
//! may be shown is picked from the addon's episode list, and a series that has an episode in
//! progress after that seed shows that episode instead.

use crate::library::{is_series_like, HistoryEntry, ProgressEntry, WatchedItem};
use crate::nextgen::AppStore;
use crate::StoreError;
use std::collections::{BTreeMap, HashMap};

/// `HomeNextUpInitialResolutionLimit`.
pub const RESOLUTION_LIMIT: usize = 32;
/// A new season that is not out yet is announced this many days before.
const SEASON_ANNOUNCE_DAYS: i64 = 7;
const DAY_MS: i64 = 86_400_000;

/// The last episode finished of a series.
#[derive(Debug, Clone, PartialEq)]
pub struct Seed {
    pub meta_id: String,
    pub meta_type: String,
    pub season: i32,
    pub episode: i32,
    pub at_epoch_ms: i64,
}

fn is_series_content(content_type: &str) -> bool {
    is_series_like(content_type) || content_type.trim().eq_ignore_ascii_case("anime")
}

/// `buildHomeNextUpSeedCandidates` (from the furthest episode, the original's default), newest first.
pub fn seeds(progress: &[ProgressEntry], watched: &[WatchedItem]) -> Vec<Seed> {
    let from_progress = progress.iter().filter(|e| e.effectively_completed()).map(|e| {
        (&e.parent_meta_id, &e.parent_meta_type, e.season_number, e.episode_number, e.last_updated_epoch_ms)
    });
    let from_watched = watched.iter().map(|w| (&w.id, &w.content_type, w.season, w.episode, w.marked_at_epoch_ms));
    let mut by_series: HashMap<String, Seed> = HashMap::new();
    for (id, content_type, season, episode, at) in from_progress.chain(from_watched) {
        let (Some(season), Some(episode)) = (season, episode) else { continue };
        let id = id.trim();
        if id.is_empty() || season == 0 || !is_series_content(content_type) {
            continue;
        }
        let seed = Seed { meta_id: id.into(), meta_type: content_type.trim().into(), season, episode, at_epoch_ms: at };
        match by_series.get(id) {
            Some(kept) if (kept.season, kept.episode, kept.at_epoch_ms) >= (season, episode, at) => {}
            _ => {
                by_series.insert(id.into(), seed);
            }
        }
    }
    let mut out: Vec<Seed> = by_series.into_values().collect();
    out.sort_by(|a, b| (b.at_epoch_ms, b.season, b.episode, &b.meta_id).cmp(&(a.at_epoch_ms, a.season, a.episode, &a.meta_id)));
    out
}

/// What the choice of the next episode needs from one entry of the addon's list.
#[derive(Debug, Clone, Copy)]
pub struct Episode<'a> {
    pub season: i32,
    pub episode: i32,
    pub released: Option<&'a str>,
}

/// Days since 1970-01-01 of an ISO date or date-time (`days_from_civil`, proleptic Gregorian).
pub fn iso_day(value: &str) -> Option<i64> {
    let date = value.trim().get(..10)?;
    let mut parts = date.split('-');
    let (y, m, d): (i64, i64, i64) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// `hasEpisodeAired`: a date up to today; without a date it counts as out.
pub fn has_aired(released: Option<&str>, now_ms: i64) -> bool {
    released.and_then(iso_day).is_none_or(|day| day <= now_ms.div_euclid(DAY_MS))
}

/// `nextReleasedEpisodeAfter`: index in `episodes` of the one that follows (`season`, `episode`).
/// An episode of the same season shows even before it airs (`showUnairedNextUp`, default on); a
/// new season needs a date that passed or comes within a week.
pub fn next_episode(episodes: &[Episode<'_>], season: i32, episode: i32, now_ms: i64) -> Option<usize> {
    let mut order: Vec<usize> = (0..episodes.len()).filter(|&i| episodes[i].season > 0).collect();
    order.sort_by_key(|&i| (episodes[i].season, episodes[i].episode));
    order.dedup_by_key(|i| (episodes[*i].season, episodes[*i].episode));
    let listed = order.iter().position(|&i| (episodes[i].season, episodes[i].episode) == (season, episode));
    let seed = listed.or_else(|| {
        // Absolute numbering of anime: "season 1, episode 30" of an addon that splits seasons.
        let several_seasons = order.first().map(|&i| episodes[i].season) != order.last().map(|&i| episodes[i].season);
        let index = usize::try_from(episode.checked_sub(1)?).ok()?;
        (season == 1 && several_seasons && index < order.len()).then_some(index)
    })?;
    let seed_season = episodes[order[seed]].season;
    let today = now_ms.div_euclid(DAY_MS);
    order[seed + 1..].iter().copied().find(|&i| {
        episodes[i].season == seed_season
            || episodes[i].released.and_then(iso_day).is_some_and(|day| day <= today + SEASON_ANNOUNCE_DAYS)
    })
}

/// `buildHomeContinueWatchingItems`: episodes in progress and next episodes by recency, one per
/// title, the next episode left out when one is in progress since it.
pub fn merge(in_progress: Vec<HistoryEntry>, next: Vec<HistoryEntry>, limit: usize) -> Vec<HistoryEntry> {
    let mut out = in_progress;
    for item in next {
        match out.iter().position(|e| e.meta_id.trim() == item.meta_id.trim()) {
            Some(i) if out[i].last_watched_epoch_ms >= item.last_watched_epoch_ms => {}
            Some(i) => out[i] = item,
            None => out.push(item),
        }
    }
    out.sort_by(|a, b| b.last_watched_epoch_ms.cmp(&a.last_watched_epoch_ms).then_with(|| b.in_progress.cmp(&a.in_progress)));
    out.truncate(limit);
    out
}

fn dismissed_doc(profile: i64) -> String {
    format!("next-up-dismissed-{profile}")
}

impl AppStore {
    /// Series whose next episode was removed from Continue watching, and the date of the seed it
    /// had then: watching more of that series brings it back.
    pub fn dismissed_next_up(&self, profile: i64) -> Result<BTreeMap<String, i64>, StoreError> {
        Ok(self.read(&dismissed_doc(profile))?.unwrap_or_default())
    }

    pub fn dismiss_next_up(&self, profile: i64, meta_id: &str) -> Result<bool, StoreError> {
        let _guard = crate::library::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let meta_id = meta_id.trim();
        let Some(seed) = seeds(&self.progress(profile)?, &self.watched(profile)?).into_iter().find(|s| s.meta_id == meta_id) else {
            return Ok(false);
        };
        let mut dismissed = self.dismissed_next_up(profile)?;
        dismissed.insert(seed.meta_id, seed.at_epoch_ms);
        self.write(&dismissed_doc(profile), &dismissed)?;
        Ok(true)
    }

    /// The seeds whose next episode may be shown.
    pub fn next_up_seeds(&self, profile: i64, progress: &[ProgressEntry]) -> Result<Vec<Seed>, StoreError> {
        let dismissed = self.dismissed_next_up(profile)?;
        let mut out = seeds(progress, &self.watched(profile)?);
        out.retain(|s| dismissed.get(&s.meta_id).is_none_or(|&at| s.at_epoch_ms > at));
        out.truncate(RESOLUTION_LIMIT);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watched(id: &str, season: i32, episode: i32, at: i64) -> WatchedItem {
        WatchedItem {
            id: id.into(),
            content_type: "series".into(),
            name: id.into(),
            poster: None,
            release_info: None,
            season: Some(season),
            episode: Some(episode),
            video_id: None,
            marked_at_epoch_ms: at,
        }
    }

    #[test]
    fn the_seed_is_the_furthest_episode() {
        let marks = [watched("a", 1, 9, 10), watched("a", 1, 2, 90), watched("b", 0, 1, 50), watched("c", 2, 1, 20)];
        let found = seeds(&[], &marks);
        let short: Vec<_> = found.iter().map(|s| (s.meta_id.as_str(), s.season, s.episode)).collect();
        assert_eq!(short, vec![("c", 2, 1), ("a", 1, 9)], "specials are not seeds");
    }

    #[test]
    fn next_follows_the_original_release_rules() {
        let now = iso_day("2026-11-22").unwrap() * DAY_MS;
        assert_eq!((iso_day("1970-01-01"), iso_day("2000-03-01T00:00:00Z")), (Some(0), Some(11_017)));
        let e = |season, episode, released| Episode { season, episode, released };
        let list = [e(0, 1, None), e(1, 2, None), e(1, 1, None), e(1, 3, Some("2027-01-01")), e(2, 1, Some("2026-11-28")), e(3, 1, None)];
        assert_eq!(next_episode(&list, 1, 1, now), Some(1));
        assert_eq!(next_episode(&list, 1, 2, now), Some(3), "same season shows before airing");
        assert_eq!(next_episode(&list, 1, 3, now), Some(4), "new season within a week");
        assert_eq!(next_episode(&list, 2, 1, now), None, "a new season without a date");
        let split = [e(1, 1, None), e(1, 2, None), e(2, 1, Some("2020-01-01")), e(2, 2, None)];
        assert_eq!(next_episode(&split, 1, 3, now), Some(3), "absolute numbering");
        assert!(!has_aired(Some("2026-11-23"), now) && has_aired(Some("2026-11-22T20:00:00Z"), now) && has_aired(None, now));
    }
}
