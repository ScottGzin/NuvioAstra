//! Addon streams for one title, resolved on demand.
//!
//! Mirrors `StreamsRepository.load` for installed addons: enabled addons of the effective profile
//! (read-only from the current app's data), manifests loaded, only addons whose manifest supports
//! `stream` for the type and id are asked, each once. Stream URLs and headers never leave Rust:
//! the UI gets an opaque key plus display-safe fields.
//!
//! Every addon is asked at once and its group is handed to the caller as soon as it answers
//! (`search_in_streamed`), so the Sources screen fills up while the slowest addons are still
//! answering; one addon that hangs is cut off by `ADDON_TIMEOUT`.

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
use std::time::Duration;

/// Time one addon gets to answer. The Sources screen shows the other addons meanwhile, and a stuck
/// addon must not keep the final answer waiting past this (the HTTP client alone waits 60 s).
const ADDON_TIMEOUT: Duration = Duration::from_secs(30);
/// Error shown for an addon that did not answer within `ADDON_TIMEOUT`.
const ADDON_TIMEOUT_ERROR: &str = "Tempo esgotado ao buscar as fontes.";

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
    /// Display name of the addon that served the stream.
    pub addon_name: String,
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    /// Torrent/file name (`behaviorHints.filename`), used by the badge rules.
    pub filename: Option<String>,
    /// `behaviorHints.videoSize` in bytes (the "SIZE" chip).
    pub video_size: Option<i64>,
    pub kind: StreamKind,
    /// A debrid service is connected: this source is played through it (no local P2P), even when
    /// the addon only left a magnet (`DirectDebridPlaybackResolver` in the original).
    pub debrid_playable: bool,
    /// Name of the connected provider that resolves torrent/debrid sources (display only).
    pub debrid_provider: Option<String>,
    /// Header names only (values may be secrets).
    pub header_names: Vec<String>,
    pub not_web_ready: bool,
    /// `externalUrl`: opened in the browser (the original's external-link source).
    pub external_url: Option<String>,
    /// `ytId`: resolved by the in-app YouTube extractor when played.
    pub youtube_id: Option<String>,
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

/// One group of the streamed answer (`streams-part` event): sent as soon as its addon or scraper
/// answers, so the fastest sources show up without waiting for the slowest one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamPart {
    pub request_id: u64,
    /// Position of the group in the full answer (addons in the user's order, then the scrapers).
    pub order: usize,
    pub group: StreamGroup,
}

#[derive(Default)]
pub struct StreamCache {
    /// `(search id, keys)`; newest first. Keys are `"{search}:{order}:{index}"`.
    searches: Mutex<VecDeque<(u64, HashMap<String, StreamItem>)>>,
    last_search: AtomicU64,
}

impl StreamCache {
    pub fn get(&self, key: &str) -> Option<StreamItem> {
        self.searches.lock().expect("stream cache lock").iter().find_map(|(_, search)| search.get(key).cloned())
    }
    /// Registers one externally resolved stream (a media-server item) under its own key so the
    /// regular `stream_play` path, headers and local relay all work unchanged.
    pub fn remember(&self, key: String, item: StreamItem) {
        self.keep(HashMap::from([(key, item)]));
    }

    /// Drops the keys of the last search (profile switch: no stream of another profile is playable).
    pub fn clear(&self) {
        self.searches.lock().expect("stream cache lock").clear();
    }
    /// Starts one answer and returns its id: every key of this search is `"{id}:{order}:{index}"`.
    /// Sources are registered with [`insert`](Self::insert) as each addon or scraper answers, so a
    /// card shown early is already playable.
    pub fn begin(&self) -> u64 {
        let id = self.next_search();
        let mut searches = self.searches.lock().expect("stream cache lock");
        searches.push_front((id, HashMap::new()));
        searches.truncate(KEPT_SEARCHES);
        id
    }
    /// Registers a source under a key already handed to the UI.
    pub fn insert(&self, search: u64, key: String, item: StreamItem) {
        let mut searches = self.searches.lock().expect("stream cache lock");
        if let Some((_, items)) = searches.iter_mut().find(|(id, _)| *id == search) {
            items.insert(key, item);
        }
    }
    fn next_search(&self) -> u64 {
        self.last_search.fetch_add(1, Ordering::Relaxed) + 1
    }
    /// Registers a direct stream (an imported M3U channel) and returns its UI entry.
    pub fn put_direct(&self, key: &str, name: &str, url: &str) -> StreamEntry {
        let mut item = StreamItem {
            url: Some(url.to_string()),
            name: Some(name.to_string()),
            title: Some(name.to_string()),
            addon_name: "M3U".into(),
            addon_id: "m3u".into(),
            ..Default::default()
        };
        let entry = StreamEntry {
            key: key.to_string(),
            addon_name: "M3U".into(),
            name: item.name.clone(),
            title: item.title.clone(),
            description: None,
            filename: None,
            video_size: None,
            kind: StreamKind::Direct,
            debrid_playable: false,
            debrid_provider: None,
            header_names: Vec::new(),
            not_web_ready: false,
            external_url: None,
            youtube_id: None,
        };
        let mut cached = HashMap::new();
        cached.insert(key.to_string(), std::mem::take(&mut item));
        self.keep(cached);
        entry
    }

    fn keep(&self, items: HashMap<String, StreamItem>) {
        let id = self.next_search();
        let mut searches = self.searches.lock().expect("stream cache lock");
        searches.push_front((id, items));
        searches.truncate(KEPT_SEARCHES);
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
    Ok(search_in(client, cache, &manifests, content_type, id, false, None).await)
}

/// Addons asked for one title's streams, in the user's order.
fn targets<'a>(manifests: &'a [AddonManifest], content_type: &str, id: &str) -> Vec<&'a AddonManifest> {
    manifests.iter().filter(|m| m.supports_resource("stream", content_type, id)).collect()
}

/// Number of addons asked for one title (the scrapers' groups come after them).
pub fn target_count(manifests: &[AddonManifest], content_type: &str, id: &str) -> usize {
    targets(manifests, content_type, id).len()
}

/// UI entry of one source; the stream itself goes to `cache` under the same key.
fn entry_of(cache: &StreamCache, search: u64, order: usize, index: usize, addon_name: &str, stream: StreamItem, debrid_provider: Option<&str>) -> StreamEntry {
    let kind = kind_of(&stream);
    let key = format!("{search}:{order}:{index}");
    let entry = StreamEntry {
        key: key.clone(),
        addon_name: addon_name.to_string(),
        name: stream.name.clone(),
        title: stream.title.clone(),
        description: stream.description.clone(),
        filename: stream.behavior_hints.filename.clone(),
        video_size: stream.behavior_hints.video_size,
        kind,
        debrid_playable: debrid_provider.is_some() && matches!(kind, StreamKind::Torrent | StreamKind::Debrid),
        debrid_provider: if matches!(kind, StreamKind::Torrent | StreamKind::Debrid) { debrid_provider.map(str::to_string) } else { None },
        header_names: stream.playback_request_headers().into_iter().map(|(k, _)| k).collect(),
        not_web_ready: stream.behavior_hints.not_web_ready,
        external_url: stream.external_url.clone(),
        youtube_id: stream.yt_id.clone(),
    };
    cache.insert(search, key, stream);
    entry
}

/// One group of the answer: an addon's or scraper's streams, or its error.
fn group_of(
    cache: &StreamCache,
    search: u64,
    order: usize,
    addon_name: &str,
    result: Result<Vec<StreamItem>, String>,
    debrid_provider: Option<&str>,
) -> StreamGroup {
    match result {
        Err(error) => StreamGroup { addon_name: addon_name.to_string(), streams: Vec::new(), error: Some(error) },
        Ok(streams) => StreamGroup {
            addon_name: addon_name.to_string(),
            error: None,
            streams: streams
                .into_iter()
                .enumerate()
                .map(|(index, stream)| entry_of(cache, search, order, index, addon_name, stream, debrid_provider))
                .collect(),
        },
    }
}

/// Searches streams for one title in the given addons (those that serve `stream` for it), in order.
/// The streams go to `cache`, which keeps the last few searches.
pub async fn search_in(
    client: &AddonClient,
    cache: &StreamCache,
    manifests: &[AddonManifest],
    content_type: &str,
    id: &str,
    force: bool,
    debrid_provider: Option<&str>,
) -> Vec<StreamGroup> {
    let search = cache.begin();
    search_in_streamed(client, cache, manifests, content_type, id, force, debrid_provider, search, 0, 0, |_| {}).await
}

/// Same requests as `search_in`, all at once: every addon's group is handed to `emit` as soon as it
/// answers and registered in `cache`, so the Sources screen shows the fastest addons without
/// waiting for the slowest one. `search` is the id from [`StreamCache::begin`], `order_base` the
/// position of the first addon in the full answer (the scrapers come after them).
#[allow(clippy::too_many_arguments)]
pub async fn search_in_streamed(
    client: &AddonClient,
    cache: &StreamCache,
    manifests: &[AddonManifest],
    content_type: &str,
    id: &str,
    force: bool,
    debrid_provider: Option<&str>,
    search: u64,
    request_id: u64,
    order_base: usize,
    mut emit: impl FnMut(StreamPart),
) -> Vec<StreamGroup> {
    let targets = targets(manifests, content_type, id);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for (index, manifest) in targets.iter().enumerate() {
        let order = order_base + index;
        let client = client.clone();
        let tx = tx.clone();
        let url = build_resource_url(&manifest.transport_url, "stream", content_type, id, None);
        let (name, addon_id, logo) = (
            manifest.name.clone(),
            format!("addon:{}:{}", manifest.id, manifest.transport_url),
            manifest.logo_url.clone(),
        );
        tauri::async_runtime::spawn(async move {
            let result = match tokio::time::timeout(ADDON_TIMEOUT, client.get_text(&url, force)).await {
                Ok(Ok(payload)) => parse_streams(&payload, &name, &addon_id, logo.as_deref()).map_err(|e| e.to_string()),
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => Err(ADDON_TIMEOUT_ERROR.to_string()),
            };
            let _ = tx.send((order, name, result));
        });
    }
    drop(tx);
    let mut groups = Vec::with_capacity(targets.len() + 1);
    while let Some((order, addon_name, result)) = rx.recv().await {
        let group = group_of(cache, search, order, &addon_name, result, debrid_provider);
        emit(StreamPart { request_id, order, group: group.clone() });
        groups.push((order, group));
    }
    // UI test phase `p2p` only: one freely licensed torrent (see `uitest::p2p_test_streams`).
    if let Some((name, payload)) = crate::uitest::p2p_test_streams() {
        let order = order_base + targets.len();
        let result = parse_streams(payload, name, "uitest:p2p", None).map_err(|e| e.to_string());
        let group = group_of(cache, search, order, name, result, debrid_provider);
        emit(StreamPart { request_id, order, group: group.clone() });
        groups.push((order, group));
    }
    groups.sort_by_key(|(order, _)| *order);
    groups.into_iter().map(|(_, group)| group).collect()
}

/// Turns one scraper's results into the same stream group an addon produces, so the Sources screen,
/// playback and the local relay need no plugin-specific code. `search`/`order` come from the title's
/// answer (the scraper groups follow the addons).
pub fn plugin_group(cache: &StreamCache, search: u64, order: usize, name: &str, streams: Vec<StreamItem>, debrid_provider: Option<&str>) -> StreamGroup {
    group_of(cache, search, order, name, Ok(streams), debrid_provider)
}

/// Turns every torrent/debrid source into one card per connected debrid provider. Each copy carries
/// the provider id in its cache key, so clicking it resolves through that provider only. When there
/// is a single provider the original card is kept as is.
pub fn duplicate_for_providers(cache: &StreamCache, groups: &mut [StreamGroup], choices: &[(String, String)]) {
    if choices.len() < 2 {
        return;
    }
    let search = cache.begin();
    for (group_index, group) in groups.iter_mut().enumerate() {
        let originals: Vec<StreamEntry> = group.streams.iter().filter(|entry| matches!(entry.kind, StreamKind::Torrent | StreamKind::Debrid)).cloned().collect();
        if originals.is_empty() {
            continue;
        }
        group.streams.retain(|entry| !matches!(entry.kind, StreamKind::Torrent | StreamKind::Debrid));
        for (i, original) in originals.into_iter().enumerate() {
            let Some(item) = cache.get(&original.key) else { continue };
            for (provider_index, (code, id)) in choices.iter().enumerate() {
                let mut copy = item.clone();
                copy.forced_debrid = Some(id.clone());
                let key = format!("{search}:{group_index}:{i}:{provider_index}");
                let mut entry = original.clone();
                entry.key = key.clone();
                entry.debrid_provider = Some(code.clone());
                entry.debrid_playable = true;
                cache.insert(search, key, copy);
                group.streams.push(entry);
            }
        }
    }
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
    fn debrid_sources_are_duplicated_per_provider() {
        let cache = StreamCache::default();
        let stream = parse_streams(r#"{"streams":[{"infoHash":"abc","name":"Movie"}]}"#, "A", "a", None).unwrap().remove(0);
        cache.keep(HashMap::from([("0:0:0".to_string(), stream)]));
        let entry = StreamEntry {
            key: "0:0:0".into(),
            addon_name: "A".into(),
            name: Some("Movie".into()),
            title: None,
            description: None,
            filename: None,
            video_size: None,
            kind: StreamKind::Torrent,
            debrid_playable: true,
            debrid_provider: None,
            header_names: Vec::new(),
            not_web_ready: false,
            external_url: None,
            youtube_id: None,
        };
        let mut groups = vec![StreamGroup { addon_name: "A".into(), streams: vec![entry], error: None }];
        let choices = vec![("RD".to_string(), "realdebrid".to_string()), ("TB".to_string(), "torbox".to_string())];
        duplicate_for_providers(&cache, &mut groups, &choices);
        assert_eq!(groups[0].streams.len(), 2);
        assert_eq!(groups[0].streams[0].debrid_provider.as_deref(), Some("RD"));
        assert_eq!(groups[0].streams[1].debrid_provider.as_deref(), Some("TB"));
        assert_eq!(cache.get(&groups[0].streams[0].key).unwrap().forced_debrid.as_deref(), Some("realdebrid"));
    }

    #[test]
    fn join_all_keeps_order() {
        let out = tauri::async_runtime::block_on(join_all((0..5).map(|i| async move { i * 2 })));
        assert_eq!(out, vec![0, 2, 4, 6, 8]);
    }

    /// Two addons on a local server: `/fast` answers at once, `/slow` after 700 ms.
    fn stream_server() -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    let _ = reader.read_line(&mut request);
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                            break;
                        }
                    }
                    let path = request.split_whitespace().nth(1).unwrap_or("");
                    if path.starts_with("/slow/") {
                        std::thread::sleep(Duration::from_millis(700));
                    }
                    let body = if path.ends_with("manifest.json") {
                        String::new()
                    } else {
                        r#"{"streams":[{"name":"HD","url":"https://cdn.test/a.m3u8"}]}"#.to_string()
                    };
                    let mut out = stream;
                    let _ = write!(out, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn streamed_search_hands_over_the_fast_addon_first_and_keeps_its_keys() {
        let base = stream_server();
        let manifest = |name: &str| {
            nuvio_core::manifest::parse_manifest(
                &format!("{base}/{name}/manifest.json"),
                &format!(r#"{{"id":"{name}","name":"{name}","version":"1","resources":["stream"],"types":["movie"]}}"#),
            )
            .unwrap()
        };
        let manifests = vec![manifest("fast"), manifest("slow")];
        let cache = StreamCache::default();
        let client = AddonClient::new().unwrap();
        let search = cache.begin();
        let mut parts = Vec::new();
        let groups = tauri::async_runtime::block_on(search_in_streamed(
            &client,
            &cache,
            &manifests,
            "movie",
            "tt0100",
            false,
            None,
            search,
            7,
            0,
            |part| parts.push(part),
        ));
        // The complete answer keeps the addon order…
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].addon_name, "fast");
        assert_eq!(groups[1].addon_name, "slow");
        // …while the fast addon was handed over before the slow one answered.
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].order, 0);
        assert_eq!(parts[0].request_id, 7);
        // A card shown before the end is already playable.
        let early = &parts[0].group.streams[0].key;
        assert_eq!(cache.get(early).and_then(|s| s.url).as_deref(), Some("https://cdn.test/a.m3u8"));
    }
}
