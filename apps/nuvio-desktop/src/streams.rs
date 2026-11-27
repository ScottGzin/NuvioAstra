//! Addon streams for one title, resolved on demand.
//!
//! Mirrors `StreamsRepository.load` for installed addons: enabled addons of the effective profile
//! (read-only from the current app's data), manifests loaded, only addons whose manifest supports
//! `stream` for the type and id are asked, each once. Stream URLs and headers never leave Rust:
//! the UI gets an opaque key plus display-safe fields.

use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_core::stream::{parse_streams, StreamItem};
use nuvio_core::url::build_resource_url;
use nuvio_store::{default_legacy_data_dir, LegacyStore};
use serde::Serialize;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// What the player can do with a stream in this stage.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StreamKind {
    /// Direct media URL (HTTP(S) file, HLS, DASH…): playable now.
    Direct,
    /// Torrent / magnet: needs the P2P engine (not ported yet).
    Torrent,
    /// Debrid `clientResolve`: needs the debrid resolver (not ported yet).
    Debrid,
    /// `externalUrl`: opened outside the player in the original app.
    External,
    /// `ytId` only: needs the YouTube resolver (not ported yet).
    YouTube,
}

pub fn kind_of(stream: &StreamItem) -> StreamKind {
    if stream.client_resolve.is_some() && stream.playable_direct_url().is_none() {
        StreamKind::Debrid
    } else if stream.is_torrent() {
        StreamKind::Torrent
    } else if stream.playable_direct_url().is_some() {
        StreamKind::Direct
    } else if stream.yt_id.is_some() {
        StreamKind::YouTube
    } else {
        StreamKind::External
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamEntry {
    pub key: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub kind: StreamKind,
    /// Header names only (values may be secrets).
    pub header_names: Vec<String>,
    pub not_web_ready: bool,
    /// A TorBox source of the addon itself (`clientResolve`): plays with the TorBox key.
    pub torbox: bool,
    /// The file's name and size when the addon says (badges, downloads).
    pub filename: Option<String>,
    pub size: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamGroup {
    pub addon_name: String,
    pub streams: Vec<StreamEntry>,
    /// Error without URL (status code or network error kind).
    pub error: Option<String>,
}

/// Searches whose keys still work: the Details list stays playable while the player searches the
/// next episode, and a key never means a stream of another search.
const KEPT_SEARCHES: usize = 6;

#[derive(Default)]
pub struct StreamCache {
    /// (search id, its streams), newest first.
    searches: Mutex<VecDeque<(u64, HashMap<String, StreamItem>)>>,
    last_search: AtomicU64,
}

impl StreamCache {
    pub fn get(&self, key: &str) -> Option<StreamItem> {
        self.searches.lock().expect("stream cache lock").iter().find_map(|(_, search)| search.get(key).cloned())
    }
    /// Drops the keys of the last search (profile switch: no stream of another profile is playable).
    pub fn clear(&self) {
        self.searches.lock().expect("stream cache lock").clear();
    }
    fn next_search(&self) -> u64 {
        self.last_search.fetch_add(1, Ordering::Relaxed) + 1
    }
    fn begin(&self, search: u64) {
        let mut searches = self.searches.lock().expect("stream cache lock");
        searches.push_front((search, HashMap::new()));
        searches.truncate(KEPT_SEARCHES);
    }
    /// The streams of one addon's answer: playable at once, before the other addons answer.
    fn add(&self, search: u64, items: HashMap<String, StreamItem>) {
        let mut searches = self.searches.lock().expect("stream cache lock");
        if let Some((_, map)) = searches.iter_mut().find(|(id, _)| *id == search) {
            map.extend(items);
        }
    }
}

/// Enabled addon manifest URLs of the active (or given) profile, normalized like the original app.
fn enabled_addon_urls(profile: Option<i64>) -> Result<Vec<String>, String> {
    let store = LegacyStore::open(default_legacy_data_dir().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let profiles = store.profiles().map_err(|e| e.to_string())?;
    let index = profile.or(profiles.as_ref().map(|p| p.active_profile_index)).unwrap_or(1);
    let effective = profiles.as_ref().map_or(index, |p| LegacyStore::effective_addon_profile(p, index));
    Ok(store
        .installed_addons(effective)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|a| a.enabled)
        .map(|a| a.manifest_url)
        .collect())
}

pub async fn load_manifests(client: &AddonClient, profile: Option<i64>) -> Result<Vec<Result<AddonManifest, String>>, String> {
    let urls = enabled_addon_urls(profile)?;
    let tasks = urls.into_iter().map(|url| {
        let client = client.clone();
        async move { client.fetch_manifest(&url).await.map_err(|e| e.to_string()) }
    });
    Ok(join_all(tasks).await)
}

/// Runs every future concurrently on the async runtime and returns the outputs in order.
async fn join_all<F>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let handles: Vec<_> = futures.into_iter().map(tauri::async_runtime::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        out.push(handle.await.expect("task panicked"));
    }
    out
}

/// Searches streams for one title in every enabled addon that serves `stream` for it.
pub async fn search(
    client: &AddonClient,
    cache: &StreamCache,
    content_type: &str,
    id: &str,
    profile: Option<i64>,
) -> Result<Vec<StreamGroup>, String> {
    let manifests: Vec<AddonManifest> = load_manifests(client, profile).await?.into_iter().filter_map(Result::ok).collect();
    Ok(search_in(client, cache, &manifests, content_type, id).await)
}

/// Searches streams for one title in the given addons (those that serve `stream` for it), in order.
/// The streams go to `cache`, which keeps the last few searches.
pub async fn search_in(
    client: &AddonClient,
    cache: &StreamCache,
    manifests: &[AddonManifest],
    content_type: &str,
    id: &str,
) -> Vec<StreamGroup> {
    search_with(client, cache, manifests, content_type, id, false).await
}

/// `search_in`; with `fresh`, the addons are asked with `Cache-Control: no-cache` (reload).
pub async fn search_with(
    client: &AddonClient,
    cache: &StreamCache,
    manifests: &[AddonManifest],
    content_type: &str,
    id: &str,
    fresh: bool,
) -> Vec<StreamGroup> {
    search_with_parts(client, cache, manifests, content_type, id, fresh, &|_, _| {}).await
}

/// `search_with`, calling `on_part(position, group)` as soon as each addon answers (its streams
/// are playable from then on). The result keeps the addons' order.
pub async fn search_with_parts(
    client: &AddonClient,
    cache: &StreamCache,
    manifests: &[AddonManifest],
    content_type: &str,
    id: &str,
    fresh: bool,
    on_part: &(dyn Fn(usize, &StreamGroup) + Sync),
) -> Vec<StreamGroup> {
    let targets: Vec<&AddonManifest> = manifests.iter().filter(|m| m.supports_resource("stream", content_type, id)).collect();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, String, Result<Vec<StreamItem>, String>)>();
    for (index, manifest) in targets.iter().enumerate() {
        let client = client.clone();
        let url = build_resource_url(&manifest.transport_url, "stream", content_type, id, None);
        let (name, addon_id, logo) = (
            manifest.name.clone(),
            format!("addon:{}:{}", manifest.id, manifest.transport_url),
            manifest.logo_url.clone(),
        );
        let tx = tx.clone();
        tauri::async_runtime::spawn(async move {
            let result = client
                .get_text(&url, fresh)
                .await
                .map_err(|e| e.to_string())
                .and_then(|payload| parse_streams(&payload, &name, &addon_id, logo.as_deref()).map_err(|e| e.to_string()));
            let _ = tx.send((index, name, result));
        });
    }
    let mut total = targets.len();
    // UI test phase `p2p` only: one freely licensed torrent (see `uitest::p2p_test_streams`).
    if let Some((name, payload)) = crate::uitest::p2p_test_streams() {
        let _ = tx.send((total, name.to_string(), parse_streams(payload, name, "uitest:p2p", None).map_err(|e| e.to_string())));
        total += 1;
    }
    drop(tx);

    let search = cache.next_search();
    cache.begin(search);
    let mut slots: Vec<Option<StreamGroup>> = vec![None; total];
    while let Some((index, addon_name, result)) = rx.recv().await {
        let (group, items) = group_of(search, index, addon_name, result);
        cache.add(search, items);
        on_part(index, &group);
        if let Some(slot) = slots.get_mut(index) {
            *slot = Some(group);
        }
    }
    slots.into_iter().flatten().collect()
}

/// A search that is not an addon request (the scraper plugins of a title): its groups are added
/// one by one as they arrive, all under one search id.
pub fn open_search(cache: &StreamCache) -> u64 {
    let search = cache.next_search();
    cache.begin(search);
    search
}

pub fn add_group(cache: &StreamCache, search: u64, index: usize, name: String, items: Vec<StreamItem>) -> StreamGroup {
    let (group, map) = group_of(search, index, name, Ok(items));
    cache.add(search, map);
    group
}

/// Streams that do not come from an addon request (an M3U channel): kept like one addon's answer,
/// so they play through the same path.
pub fn register(cache: &StreamCache, group_name: String, payload: &str, addon_id: &str) -> Vec<StreamGroup> {
    let result = parse_streams(payload, &group_name, addon_id, None).map_err(|e| e.to_string());
    let search = cache.next_search();
    cache.begin(search);
    let (group, items) = group_of(search, 0, group_name, result);
    cache.add(search, items);
    vec![group]
}

fn group_of(search: u64, group_index: usize, addon_name: String, result: Result<Vec<StreamItem>, String>) -> (StreamGroup, HashMap<String, StreamItem>) {
    let mut cached = HashMap::new();
    let group = match result {
        Err(error) => StreamGroup { addon_name, streams: vec![], error: Some(error) },
        Ok(streams) => StreamGroup {
            addon_name,
            error: None,
            streams: streams
                .into_iter()
                .enumerate()
                .map(|(i, stream)| {
                    let key = format!("{search}:{group_index}:{i}");
                    let entry = StreamEntry {
                        key: key.clone(),
                        name: stream.name.clone(),
                        description: stream.description.clone(),
                        kind: kind_of(&stream),
                        header_names: stream.playback_request_headers().into_iter().map(|(k, _)| k).collect(),
                        not_web_ready: stream.behavior_hints.not_web_ready,
                        torbox: nuvio_core::torbox::is_torbox_client_stream(&stream),
                        filename: stream.behavior_hints.filename.clone(),
                        size: stream.behavior_hints.video_size,
                    };
                    cached.insert(key, stream);
                    entry
                })
                .collect(),
        },
    };
    (group, cached)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_core::stream::parse_streams;

    #[test]
    fn kinds_follow_the_original_playback_rules() {
        let payload = r#"{"streams":[
            {"url":"https://cdn.test/a.m3u8"},
            {"infoHash":"abc"},
            {"url":"magnet:?xt=urn:btih:abc"},
            {"clientResolve":{"type":"debrid"}},
            {"externalUrl":"https://site.test/watch"},
            {"ytId":"dQw4w9WgXcQ"}
        ]}"#;
        let kinds: Vec<_> = parse_streams(payload, "a", "b", None).unwrap().iter().map(kind_of).collect();
        assert_eq!(kinds, vec![StreamKind::Direct, StreamKind::Torrent, StreamKind::Torrent, StreamKind::Debrid, StreamKind::External, StreamKind::YouTube]);
    }

    #[test]
    fn join_all_keeps_order() {
        let out = tauri::async_runtime::block_on(join_all((0..5).map(|i| async move { i * 2 })));
        assert_eq!(out, vec![0, 2, 4, 6, 8]);
    }
}
