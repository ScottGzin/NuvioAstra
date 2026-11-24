//! Pure episode-shuffle rules ported from NuvioDesktop's `RandomEpisodePicker` /
//! `EpisodeShuffle`. No UI, no wall-clock access inside `pick`: the caller passes today's date.

use crate::meta::MetaVideo;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShuffleSettings {
    pub enabled: bool,
    pub include_watched: bool,
}

impl ShuffleSettings {
    /// `EpisodeShuffleRepository.settings`: enabled only for available series-like content.
    pub fn effective(self, available: bool, content_type: &str) -> Self {
        let series = matches!(content_type.trim().to_ascii_lowercase().as_str(), "series" | "tv" | "show" | "tvshow");
        Self { enabled: self.enabled && available && series, ..self }
    }
}

/// `isReleasedBy`: the original treats a missing/invalid date as "not aired" when `available` is
/// false, and as aired when there is no date to check.
fn released_by(video: &MetaVideo, today: &str) -> bool {
    if !video.available {
        return false;
    }
    let Some(date) = video.released.as_deref().map(str::trim) else { return true };
    let day = date.get(..10).unwrap_or(date);
    if day.len() != 10 || !day.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() }) {
        return true;
    }
    day <= today
}

/// A tiny deterministic PRNG so tests can reproduce a pick; the app seeds it from the clock.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n <= 1 { return 0; }
        (self.next() % n as u64) as usize
    }
}

/// `RandomEpisodePicker`: keeps the episodes already shown for one show so a shuffle session does
/// not repeat an episode until it has exhausted the pool.
pub struct Picker {
    episodes: Vec<MetaVideo>,
    unwatched: Vec<MetaVideo>,
    shown: HashSet<String>,
    last_picked: Option<String>,
    rng: Rng,
}

impl Picker {
    pub fn new(videos: &[MetaVideo], watched: &HashSet<(i64, i64)>, today: &str, seed: u64) -> Self {
        let mut episodes: Vec<MetaVideo> = videos
            .iter()
            .filter(|v| v.season.unwrap_or(0) > 0 && v.episode.is_some())
            .filter(|v| released_by(v, today))
            .filter(|v| !v.id.trim().is_empty() && v.episode.unwrap_or(0) > 0)
            .cloned()
            .collect();
        let mut seen_pairs = HashSet::new();
        episodes.retain(|v| seen_pairs.insert((v.season.unwrap(), v.episode.unwrap())));
        let mut seen_ids = HashSet::new();
        episodes.retain(|v| seen_ids.insert(v.id.clone()));
        let unwatched = episodes
            .iter()
            .filter(|v| !watched.contains(&(v.season.unwrap(), v.episode.unwrap())))
            .cloned()
            .collect();
        // Carry over the previous session's state (EpisodeShuffle.select inheritance).
        Self { episodes, unwatched, shown: HashSet::new(), last_picked: None, rng: Rng(seed) }
    }

    pub fn with_history(mut self, shown: HashSet<String>, last_picked: Option<String>) -> Self {
        self.shown = shown;
        self.last_picked = last_picked;
        self
    }

    pub fn into_history(self) -> (HashSet<String>, Option<String>) { (self.shown, self.last_picked) }

    /// `pick(includeWatched, current, consumeSelection)`.
    pub fn pick(&mut self, include_watched: bool, current: Option<(i64, i64)>, consume_selection: bool) -> Option<MetaVideo> {
        let pool_source = if include_watched { &self.episodes } else { &self.unwatched };
        let candidates: Vec<&MetaVideo> = pool_source
            .iter()
            .filter(|v| Some((v.season.unwrap(), v.episode.unwrap())) != current)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let unseen: Vec<&MetaVideo> = candidates.iter().copied().filter(|v| !self.shown.contains(&v.id)).collect();
        let chosen = if !unseen.is_empty() {
            unseen[self.rng.below(unseen.len())]
        } else {
            if consume_selection {
                self.shown.retain(|id| candidates.iter().any(|v| v.id == *id));
            } else {
                self.shown.clear();
            }
            let rest: Vec<&MetaVideo> = candidates.iter().copied().filter(|v| self.last_picked.as_deref() != Some(v.id.as_str())).collect();
            let rest = if rest.is_empty() { candidates.as_slice() } else { rest.as_slice() };
            rest[self.rng.below(rest.len())]
        };
        if consume_selection {
            self.shown.insert(chosen.id.clone());
        }
        self.last_picked = Some(chosen.id.clone());
        Some(chosen.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str, season: i64, episode: i64, released: &str) -> MetaVideo {
        MetaVideo {
            id: id.into(), title: format!("E{episode}"), released: Some(released.into()), available: true,
            thumbnail: None, season: Some(season), episode: Some(episode), overview: None,
        }
    }

    #[test]
    fn filtered_episodes_and_released_rule() {
        let videos = vec![
            video("e1", 1, 1, "2020-01-01"),
            video("e2", 1, 2, "2099-01-01"), // unaired
            video("s0", 0, 1, "2020-01-01"), // specials excluded
            video("dup", 1, 1, "2020-01-01"),
            video("e3", 1, 3, "2020-01-01"),
        ];
        let watched = HashSet::new();
        let mut picker = Picker::new(&videos, &watched, "2026-01-01", 1);
        let first = picker.pick(false, None, true).unwrap();
        assert!(["e1", "e3"].contains(&first.id.as_str()));
        let second = picker.pick(false, None, true).unwrap();
        assert_ne!(first.id, second.id, "shown episodes are not repeated before the pool ends");
    }

    #[test]
    fn watched_and_include_watched_settings() {
        let videos = vec![video("a", 1, 1, "2020-01-01"), video("b", 1, 2, "2020-01-01")];
        let watched = HashSet::from([(1i64, 1i64)]);
        let mut picker = Picker::new(&videos, &watched, "2026-01-01", 7);
        assert_eq!(picker.pick(false, None, true).unwrap().id, "b");
        let mut picker = Picker::new(&videos, &watched, "2026-01-01", 7);
        assert!(["a", "b"].contains(&picker.pick(true, None, true).unwrap().id.as_str()));
        let settings = ShuffleSettings { enabled: true, include_watched: false };
        assert!(settings.effective(true, "series").enabled);
        assert!(!settings.effective(true, "movie").enabled);
        assert!(!settings.effective(false, "series").enabled);
    }

    #[test]
    fn current_episode_and_history_are_respected() {
        let videos = vec![video("a", 1, 1, "2020-01-01"), video("b", 1, 2, "2020-01-01")];
        let mut picker = Picker::new(&videos, &HashSet::new(), "2026-01-01", 3)
            .with_history(HashSet::from(["a".to_string()]), Some("a".into()));
        let picked = picker.pick(false, Some((1, 1)), false).unwrap();
        assert_eq!(picked.id, "b");
        let (shown, last) = picker.into_history();
        assert!(!shown.contains("b"), "consumeSelection=false must not add the pick to history");
        assert_eq!(last.as_deref(), Some("b"));
    }
}
