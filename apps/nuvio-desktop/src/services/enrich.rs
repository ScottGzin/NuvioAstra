//! Titles and artwork of "Continuar assistindo" entries that came from the account, which keeps
//! only ids and positions (`WatchProgressRepository.resolveRemoteMetadata`): asked to the profile's
//! metadata addons in the background. A title that could not be resolved waits before a retry.

use super::meta;
use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_core::meta::MetaDetails;
use nuvio_store::library::{self, ProgressEntry};
use nuvio_store::nextgen::AppStore;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);

static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static TRIED: Mutex<Option<HashMap<(String, String), Instant>>> = Mutex::new(None);

fn blank(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(|v| v.trim().is_empty())
}

/// `needsRemoteMetadataEnrichment`.
fn needs(entry: &ProgressEntry) -> bool {
    entry.title.trim().is_empty() || entry.title.eq_ignore_ascii_case(&entry.parent_meta_id) || blank(&entry.poster) || blank(&entry.background)
}

/// `enrichWatchProgressEntry` (display fields only).
fn apply(entry: &mut ProgressEntry, meta: &MetaDetails) {
    fn take(slot: &mut Option<String>, value: &Option<String>) {
        if !blank(value) {
            *slot = value.clone();
        }
    }
    if !meta.name.trim().is_empty() {
        entry.title = meta.name.clone();
    }
    take(&mut entry.poster, &meta.poster);
    take(&mut entry.background, &meta.background);
    take(&mut entry.logo, &meta.logo);
    if let (Some(season), Some(episode)) = (entry.season_number, entry.episode_number) {
        if let Some(video) = meta.videos.iter().find(|v| v.season == Some(season.into()) && v.episode == Some(episode.into())) {
            if !video.title.trim().is_empty() {
                entry.episode_title = Some(video.title.clone());
            }
            take(&mut entry.episode_thumbnail, &video.thumbnail);
        }
    }
}

/// Returns whether something on screen changed.
pub async fn continue_watching(store: &AppStore, client: &AddonClient, manifests: &[AddonManifest], profile: i64) -> bool {
    let Ok(_running) = RUNNING.try_lock() else { return false };
    let Ok(progress) = store.progress(profile) else { return false };
    let wanted: BTreeMap<String, String> = library::continue_watching(&progress, library::CONTINUE_WATCHING_LIMIT)
        .into_iter()
        .filter(needs)
        .map(|e| {
            let kind = if e.parent_meta_type.trim().is_empty() { e.content_type } else { e.parent_meta_type };
            (e.parent_meta_id, kind)
        })
        .collect();
    let mut changed = false;
    for (id, kind) in wanted {
        {
            // By title and addon set: a newly installed metadata addon is asked at once.
            let addons: String = manifests.iter().map(|m| m.id.as_str()).collect::<Vec<_>>().join(",");
            let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
            let tried = tried.get_or_insert_with(HashMap::new);
            let key = (id.clone(), addons);
            if tried.get(&key).is_some_and(|at| at.elapsed() < RETRY_AFTER) {
                continue;
            }
            tried.insert(key, Instant::now());
        }
        // No error text: it could carry an addon URL.
        let Ok(details) = meta::details(client, manifests, &kind, &id).await else { continue };
        changed |= store.enrich_progress(profile, &id, |entry| apply(entry, &details.meta)).unwrap_or(false);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_from_the_account_get_name_art_and_episode() {
        let mut entry: ProgressEntry = serde_json::from_value(serde_json::json!({
            "contentType": "series", "parentMetaId": "tt1", "parentMetaType": "series", "videoId": "tt1:1:2", "title": "tt1",
            "seasonNumber": 1, "episodeNumber": 2, "lastPositionMs": 60000, "durationMs": 1400000, "lastUpdatedEpochMs": 5
        }))
        .unwrap();
        assert!(needs(&entry));
        let meta = nuvio_core::meta::parse_meta(
            r#"{"meta":{"id":"tt1","type":"series","name":"Série","poster":"p.jpg","background":"b.jpg",
               "videos":[{"id":"tt1:1:2","title":"Piloto","season":1,"episode":2,"thumbnail":"t.jpg"}]}}"#,
        )
        .unwrap();
        apply(&mut entry, &meta);
        assert!(!needs(&entry));
        assert_eq!((entry.title.as_str(), entry.episode_title.as_deref(), entry.episode_thumbnail.as_deref()), ("Série", Some("Piloto"), Some("t.jpg")));
        assert_eq!((entry.last_position_ms, entry.last_updated_epoch_ms), (60000, 5), "position and date untouched");
    }
}
