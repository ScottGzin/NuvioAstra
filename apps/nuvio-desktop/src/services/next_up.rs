//! Next episodes for "Continue watching": each series' episode list comes from the meta addons
//! and stays in memory, so the row draws at once and is completed when the addons answer.

use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_core::meta::MetaDetails;
use nuvio_store::library::{self, HistoryEntry, ProgressEntry};
use nuvio_store::next_up::{self, Episode, Seed};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FRESH: Duration = Duration::from_secs(30 * 60);
const CONCURRENCY: usize = 6;

type Key = (i64, String, String);

#[derive(Default)]
pub struct Lists {
    by_series: Mutex<HashMap<Key, (Instant, Arc<MetaDetails>)>>,
}

impl Lists {
    fn known(&self, key: &Key) -> Option<(bool, Arc<MetaDetails>)> {
        self.by_series.lock().expect("next-up lists").get(key).map(|(at, meta)| (at.elapsed() < FRESH, meta.clone()))
    }

    /// Asks the addons for the lists that are missing or old. A failure keeps what was known.
    pub async fn refresh(&self, client: &AddonClient, manifests: &[AddonManifest], profile: i64, seeds: &[Seed]) {
        let wanted: Vec<Key> = seeds
            .iter()
            .map(|s| (profile, s.meta_type.clone(), s.meta_id.clone()))
            .filter(|key| !self.known(key).is_some_and(|(fresh, _)| fresh))
            .collect();
        for batch in wanted.chunks(CONCURRENCY) {
            let mut tasks = Vec::new();
            for key in batch.iter().cloned() {
                let (client, manifests) = (client.clone(), manifests.to_vec());
                tasks.push(tauri::async_runtime::spawn(async move {
                    let found = super::meta::details(&client, &manifests, &key.1, &key.2).await.ok();
                    (key, found)
                }));
            }
            for task in tasks {
                if let Ok((key, Some(details))) = task.await {
                    self.by_series.lock().expect("next-up lists").insert(key, (Instant::now(), Arc::new(details.meta)));
                }
            }
        }
    }

    /// The cards of the series whose list is known.
    pub fn cards(&self, profile: i64, seeds: &[Seed], progress: &[ProgressEntry], now_ms: i64) -> Vec<HistoryEntry> {
        seeds
            .iter()
            .filter_map(|seed| {
                let (_, meta) = self.known(&(profile, seed.meta_type.clone(), seed.meta_id.clone()))?;
                card(seed, &meta, progress, now_ms)
            })
            .collect()
    }
}

fn card(seed: &Seed, meta: &MetaDetails, progress: &[ProgressEntry], now_ms: i64) -> Option<HistoryEntry> {
    let numbered: Vec<_> = meta
        .videos
        .iter()
        .filter_map(|v| Some((i32::try_from(v.season?).ok()?, i32::try_from(v.episode?).ok()?, v)))
        .collect();
    let list: Vec<Episode<'_>> = numbered
        .iter()
        .map(|(season, episode, v)| Episode { season: *season, episode: *episode, released: v.released.as_deref() })
        .collect();
    let (season, episode, video) = numbered[next_up::next_episode(&list, seed.season, seed.episode, now_ms)?];
    let key = library::progress_key(&seed.meta_id, Some(season), Some(episode));
    let started = progress.iter().find(|e| e.key() == key).filter(|e| e.in_progress());
    Some(HistoryEntry {
        next_up: true,
        airs: video.released.clone().filter(|date| !next_up::has_aired(Some(date.as_str()), now_ms)),
        key: format!("next:{}", seed.meta_id),
        meta_id: seed.meta_id.clone(),
        meta_type: seed.meta_type.clone(),
        video_id: Some(video.id.clone()),
        title: meta.name.clone(),
        poster: meta.poster.clone(),
        background: video.thumbnail.clone().or_else(|| meta.background.clone()),
        season: Some(season),
        episode: Some(episode),
        episode_title: Some(video.title.clone()).filter(|t| !t.trim().is_empty()),
        position_ms: started.map_or(0, |e| e.resume_position_ms()),
        duration_ms: started.map_or(0, |e| e.duration_ms),
        fraction: started.map_or(0.0, |e| e.fraction()),
        completed: false,
        in_progress: started.is_some(),
        last_watched_epoch_ms: seed.at_epoch_ms,
    })
}
