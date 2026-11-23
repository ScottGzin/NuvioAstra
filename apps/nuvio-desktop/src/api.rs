//! Typed commands used by the React UI for app data (the player commands are in `main.rs`).
//! Every command returns display data only: addon URLs, stream URLs and tokens stay in Rust.

use crate::services::{addons, badges, collections, debrid, discord, discover, downloads, fonts, home, livetv, mdblist, meta, parental, playback, search, tmdb};
use crate::streams::{self, StreamGroup};
use crate::AppState;
use nuvio_core::catalog::{CatalogPage, MetaPreview};
use nuvio_core::downloads::{DownloadItem, DownloadStatus, is_supported_download_url};
use nuvio_core::meta::MetaVideo;
use nuvio_core::shuffle::{Picker, ShuffleSettings};
use nuvio_store::library;
use nuvio_store::nextgen::MigrationRecord;
use nuvio_store::settings::{ContinueWatchingSettings, ProfileSettings};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub active_profile: i64,
    pub profiles: Vec<addons::ProfileInfo>,
    pub addons_enabled: usize,
    pub addons_failed: usize,
    pub migration: Option<MigrationRecord>,
}

#[tauri::command]
pub async fn app_bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    let (active_profile, profiles) = addons::profiles(&state.app_store)?;
    let loaded = state.addons.loaded().await?;
    let migration = state.app_store.migration().ok().flatten();
    Ok(Bootstrap { active_profile, profiles, addons_enabled: loaded.enabled, addons_failed: loaded.failed, migration })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeView {
    pub hero_enabled: bool,
    /// Catalog keys whose first page feeds the hero (may include catalogs hidden from Home).
    pub hero_sources: Vec<String>,
    pub rows: Vec<home::HomeRow>,
    /// Continue watching preferences (Settings → Início → Continuar assistindo).
    pub continue_watching: ContinueWatchingSettings,
}

/// Home rows of the active profile, with its Home settings from the NextGen store (imported
/// from the original by the migration, edited in Settings → Início).
#[tauri::command]
pub async fn home_rows(state: State<'_, AppState>) -> Result<HomeView, String> {
    let loaded = state.addons.loaded().await?;
    let profile_settings = state.app_store.profile_settings(loaded.profile).map_err(|e| e.to_string())?;
    let settings = profile_settings.home;
    let tmdb_available = tmdb::collections_enabled(&state.app_store, loaded.profile);
    let collections = collections::views(&collections::stored(&state.app_store, loaded.profile), &loaded.manifests, settings.show_catalog_type, tmdb_available);
    Ok(HomeView {
        hero_enabled: settings.hero_enabled,
        hero_sources: home::hero_sources(&loaded.manifests, &settings),
        rows: home::home_rows(&loaded.manifests, Some(&settings), &collections),
        continue_watching: profile_settings.continue_watching,
    })
}

/// One page of a catalog identified by its home key `<manifestId>:<type>:<catalogId>`; `genre`
/// for the catalogs of a collection folder filtered by genre.
#[tauri::command]
pub async fn catalog_page(state: State<'_, AppState>, key: String, skip: Option<u32>, genre: Option<String>) -> Result<CatalogPage, String> {
    let loaded = state.addons.loaded().await?;
    let mut parts = key.splitn(3, ':');
    let (manifest_id, content_type, catalog_id) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    // Several installed addons can share a manifest id: take the declared one when it has this
    // catalog, else any enabled addon that does (`resolveCatalog` in the original).
    let (manifest, catalog) = collections::resolve_catalog(&loaded.manifests, manifest_id, content_type, catalog_id)
        .ok_or("catalog not found")?;
    let mut page = state
        .addons
        .client
        .fetch_catalog_page(manifest, &catalog.content_type, &catalog.id, genre.as_deref().filter(|g| !g.trim().is_empty()), skip)
        .await
        .map_err(|e| e.to_string())?;
    // `hideUnreleasedContent` (Settings → Início); paging still follows the raw page size.
    if state.app_store.profile_settings(loaded.profile).map_err(|e| e.to_string())?.home.hide_unreleased_content {
        let today = home::today_iso();
        page.items.retain(|item| !home::is_unreleased(item, &today));
    }
    Ok(page)
}

#[tauri::command]
pub async fn search_all(state: State<'_, AppState>, query: String) -> Result<Vec<search::SearchSection>, String> {
    let loaded = state.addons.loaded().await?;
    Ok(search::search(&state.addons.client, &loaded.manifests, &query).await)
}

/// Genre options of a Home catalog key (`<manifestId>:<type>:<catalogId>`), used by "See all".
#[derive(Debug, Clone, Serialize)]
pub struct CatalogGenres {
    pub options: Vec<String>,
    pub required: bool,
}

#[tauri::command]
pub async fn catalog_genres(state: State<'_, AppState>, key: String) -> Result<CatalogGenres, String> {
    let loaded = state.addons.loaded().await?;
    let mut parts = key.splitn(3, ':');
    let (manifest_id, content_type, catalog_id) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    // The same resolver the Home rows and `catalog_page` use, so the keys always agree.
    let (_, catalog) = collections::resolve_catalog(&loaded.manifests, manifest_id, content_type, catalog_id)
        .ok_or("catalog not found")?;
    Ok(match catalog.genre_extra() {
        Some(extra) => CatalogGenres { options: extra.options.clone(), required: extra.is_required },
        None => CatalogGenres { options: Vec::new(), required: false },
    })
}

/// One page of a TMDB collection source (`TmdbCollectionSourceResolver`). The source's kind,
/// media type, sort and filters are read from the account's copy of the collections, never taken
/// from the UI; `page` is 1-based, as the original's `skip` for these sources.
#[tauri::command]
pub async fn tmdb_source_page(
    state: State<'_, AppState>,
    collection_id: String,
    folder_id: String,
    source_id: String,
    page: u32,
) -> Result<CatalogPage, String> {
    let loaded = state.addons.loaded().await?;
    let access = tmdb::access(&state.app_store, loaded.profile).ok_or("Chave da API do TMDB não configurada.")?;
    let stored = collections::stored(&state.app_store, loaded.profile);
    let source = collections::tmdb_source(&stored, &collection_id, &folder_id, &source_id).ok_or("Fonte do TMDB não encontrada.")?;
    tmdb::page(&state.addons.client, &access, &source, page).await
}

/// Discover picker of the active profile: the addons' listable catalogs, the type options and the
/// catalog remembered from the last visit (`services::discover`).
#[tauri::command]
pub async fn discover_sources(state: State<'_, AppState>) -> Result<discover::View, String> {
    let loaded = state.addons.loaded().await?;
    let sources = discover::sources(&loaded.manifests);
    let saved = state.app_store.discover_selection().map_err(|e| e.to_string())?.remove(&loaded.profile);
    let selected_key = discover::resolve_catalog(&sources, saved.as_deref(), None).map(|s| s.key.clone());
    Ok(discover::View { type_options: discover::type_options(&sources), sources, selected_key })
}

/// Remembers the catalog chosen in Discover for the active profile (`DiscoverSelectionStorage`).
#[tauri::command]
pub fn discover_select(state: State<'_, AppState>, key: String) -> Result<(), String> {
    let profile = state.active_profile();
    state.app_store.set_discover_selection(profile, key).map_err(|e| e.to_string())
}

/// One Discover page of the catalog `key`, with the original's paging rules: a page that brought
/// nothing new advances the skip by the raw page size (at most three times in a row), and a full
/// page paginates even without a `skip` extra. `existing_keys` are the `type:id` of the items
/// already on screen; `reset` starts a new catalog/genre.
#[tauri::command]
pub async fn discover_page(
    state: State<'_, AppState>,
    key: String,
    genre: Option<String>,
    skip: u32,
    reset: bool,
    existing_keys: Vec<String>,
    duplicate_pages: u32,
) -> Result<discover::Page, String> {
    let loaded = state.addons.loaded().await?;
    let (manifest, catalog) = discover::catalog(&loaded.manifests, &key).ok_or("catalog not found")?;
    let mut page = state
        .addons
        .client
        .fetch_catalog_page(manifest, &catalog.content_type, &catalog.id, genre.as_deref().filter(|g| !g.trim().is_empty()), Some(skip).filter(|s| *s > 0))
        .await
        .map_err(|e| e.to_string())?;
    // `hideUnreleasedContent`, as on Home; paging still follows the raw page size.
    if state.app_store.profile_settings(loaded.profile).map_err(|e| e.to_string())?.home.hide_unreleased_content {
        let today = home::today_iso();
        page.items.retain(|item| !home::is_unreleased(item, &today));
    }
    Ok(discover::page(catalog.supports_pagination(), skip, reset, &existing_keys, duplicate_pages, page))
}

/// Search with results as they arrive: `search-part` events (one per catalog with results), then
/// `search-done`. The interface ignores the events of an older `request_id`.
#[tauri::command]
pub async fn search_stream(app: tauri::AppHandle, state: State<'_, AppState>, query: String, request_id: u64) -> Result<(), String> {
    use tauri::Emitter;
    let loaded = state.addons.loaded().await?;
    let emitter = app.clone();
    let done = search::search_streamed(&state.addons.client, &loaded.manifests, &query, request_id, move |part| {
        let _ = emitter.emit("search-part", part);
    })
    .await;
    let _ = app.emit("search-done", done);
    Ok(())
}

#[tauri::command]
pub async fn meta_details(state: State<'_, AppState>, content_type: String, id: String) -> Result<meta::Details, String> {
    let loaded = state.addons.loaded().await?;
    let mut details = meta::details(&state.addons.client, &loaded.manifests, &content_type, &id).await?;
    // Settings → Services → TMDB: fills what the modules are switched on for (the addon's data is
    // kept where TMDB has nothing). Nothing is fetched when TMDB is off or has no key.
    let settings = tmdb::settings(&state.app_store, loaded.profile);
    if let Some(access) = tmdb::access(&state.app_store, loaded.profile) {
        // A slow TMDB must not hold the details page: after 6 s the addon's data is shown as is.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(6), tmdb::enrich(&state.addons.client, &access, &settings, &mut details.meta, &id)).await;
        details.seasons = details.meta.seasons();
        details.trailer = nuvio_core::meta::select_trailer(&details.meta.trailers).cloned();
    }
    // MDBList ratings (the key comes from the Nuvio account); never holds the page for long.
    let imdb_id = if details.meta.id.starts_with("tt") { details.meta.id.clone() } else { id.clone() };
    let series = content_type == "series" || details.meta.content_type == "series";
    let watched = state.app_store.watched(loaded.profile).unwrap_or_default();
    details.watched_keys = watched
        .iter()
        .filter(|w| w.id == details.meta.id || w.id == id)
        .map(|w| w.key())
        .collect();
    details.ratings = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        mdblist::ratings(&state.app_store, loaded.profile, &imdb_id, series),
    )
    .await
    .ok()
    .unwrap_or_default();
    Ok(details)
}

#[tauri::command]
pub async fn livetv_channels(state: State<'_, AppState>) -> Result<livetv::ChannelList, String> {
    let loaded = state.addons.loaded().await?;
    Ok(livetv::load(&state.addons.client, &loaded.manifests).await)
}

/// "Recarregar": fetches again the manifests of the channel addons (all of them, or the ones named),
/// then their channel catalogs. Other addons are not touched.
#[tauri::command]
pub async fn livetv_reload(state: State<'_, AppState>, addon_names: Option<Vec<String>>) -> Result<livetv::ChannelList, String> {
    let loaded = state.addons.loaded().await?;
    let mut urls: Vec<String> = livetv::listable_tv_catalogs(&loaded.manifests)
        .into_iter()
        .filter(|(m, _, _)| addon_names.as_ref().is_none_or(|names| names.contains(&m.name)))
        .map(|(m, _, _)| m.transport_url.clone())
        .collect();
    urls.dedup();
    for url in &urls {
        // A failed refresh keeps the previous manifest; its channels are still listed.
        let _ = state.addons.refresh_one(url).await;
    }
    let loaded = state.addons.loaded().await?;
    Ok(livetv::load(&state.addons.client, &loaded.manifests).await)
}

#[tauri::command]
pub async fn livetv_favorites(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    Ok(state.app_store.live_tv_favorites().map_err(|e| e.to_string())?.remove(&profile).unwrap_or_default())
}

#[tauri::command]
pub async fn livetv_toggle_favorite(state: State<'_, AppState>, key: String) -> Result<Vec<String>, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let mut keys = state.app_store.live_tv_favorites().map_err(|e| e.to_string())?.remove(&profile).unwrap_or_default();
    match keys.iter().position(|k| *k == key) {
        Some(i) => {
            keys.remove(i);
        }
        None => keys.push(key),
    }
    state.app_store.set_live_tv_favorites(profile, keys.clone()).map_err(|e| e.to_string())?;
    Ok(keys)
}

/// Sources of one title from the enabled addons (cached manifests), asked when the user opens them.
#[tauri::command]
pub async fn title_streams(state: State<'_, AppState>, content_type: String, id: String, refresh: Option<bool>) -> Result<Vec<StreamGroup>, String> {
    let loaded = state.addons.loaded().await?;
    let debrid = debrid::access(&state.app_store, loaded.profile).is_some();
    Ok(streams::search_in(&state.addons.client, &state.streams, &loaded.manifests, &content_type, &id, refresh.unwrap_or(false), debrid).await)
}

/// Updates Discord Rich Presence (only when enabled in Settings → Serviços → Integrações).
#[tauri::command]
pub fn discord_activity(state: State<'_, AppState>, activity: Option<discord::Activity>) -> Result<(), String> {
    let profile = state.active_profile();
    if !state.app_store.profile_settings(profile).map(|s| s.discord_rpc_enabled).unwrap_or_default() {
        return Ok(());
    }
    discord::publish(activity.as_ref());
    Ok(())
}

/// IMDb parents-guide warnings of a title (Settings → Reprodução → Avisos de conteúdo).
#[tauri::command]
pub async fn parental_guide(state: State<'_, AppState>, id: String) -> Result<Option<parental::Guide>, String> {
    let profile = state.active_profile();
    if !state.app_store.profile_settings(profile).map(|s| s.player.show_parental_guide).unwrap_or_default() {
        return Ok(None);
    }
    Ok(parental::fetch(&state.addons.client, &id).await)
}

// ---- Collections editor (the original's CollectionRepository validation rules) ----

#[tauri::command]
pub fn collections_editor_load(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let profile = state.active_profile();
    Ok(collections::stored(&state.app_store, profile))
}

#[tauri::command]
pub fn collections_editor_save(state: State<'_, AppState>, value: serde_json::Value) -> Result<serde_json::Value, String> {
    state.ensure_unlocked()?;
    collections::validate_edited(&value)?;
    let profile = state.active_profile();
    state.app_store.set_collections(profile, &value).map_err(|e| e.to_string())?;
    state.request_sync();
    Ok(value)
}

// ---- Downloads (the original's DownloadsRepository) ----

#[tauri::command]
pub fn download_list(state: State<'_, AppState>) -> Result<Vec<DownloadItem>, String> {
    let profile = state.active_profile();
    let mut items = state.app_store.downloads(profile).map_err(|e| e.to_string())?;
    let mut changed = false;
    for item in items.iter_mut() {
        if item.status == DownloadStatus::Downloading && !state.downloads.is_active(&item.id) {
            item.status = DownloadStatus::Paused;
            changed = true;
        }
    }
    if changed {
        state.app_store.set_downloads(profile, items.clone()).map_err(|e| e.to_string())?;
    }
    Ok(items)
}

/// Queues a direct stream for download. The URL and headers stay in Rust; the UI sends only the
/// opaque stream key and the playback identity.
#[tauri::command]
pub fn download_from_stream(
    state: State<'_, AppState>,
    key: String,
    progress: nuvio_store::library::PlaybackItem,
) -> Result<DownloadItem, String> {
    state.ensure_unlocked()?;
    let stream = state.streams.get(&key).ok_or("unknown stream (search again)")?;
    let url = stream
        .playable_direct_url()
        .filter(|url| is_supported_download_url(url))
        .ok_or("this source cannot be downloaded")?
        .to_string();
    let profile = state.active_profile();
    let existing = state.app_store.downloads(profile).map_err(|e| e.to_string())?;
    let item = downloads::item_from_stream(profile, &stream, &url, &progress, &existing);
    // Same logical content replaces the previous download (with its files).
    let dir = state.app_store.dir().join("downloads");
    let replaced: Vec<DownloadItem> = existing.iter().filter(|old| old.logical_content_key() != item.logical_content_key()).cloned().collect();
    for old in existing.iter().filter(|old| old.logical_content_key() == item.logical_content_key()) {
        state.downloads.cancel(&old.id);
        downloads::remove_files(&dir, &old.file_name);
    }
    let mut saved = replaced;
    saved.push(item.clone());
    state.app_store.set_downloads(profile, saved).map_err(|e| e.to_string())?;
    state.downloads.set_headers(&item.id, stream.playback_request_headers());
    state.downloads.start(state.app_store.clone(), profile, item.id.clone(), item.source_url.clone(), dir);
    Ok(item)
}

#[tauri::command]
pub fn download_control(state: State<'_, AppState>, id: String, action: String) -> Result<Vec<DownloadItem>, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let dir = state.app_store.dir().join("downloads");
    match action.as_str() {
        "pause" => {
            state.downloads.cancel(&id);
            state.app_store.edit_downloads(profile, |items| {
                if let Some(item) = items.iter_mut().find(|item| item.id == id) { item.status = DownloadStatus::Paused; }
            }).map_err(|e| e.to_string())?;
        }
        "resume" | "retry" => {
            let item = state.app_store.downloads(profile).map_err(|e| e.to_string())?.into_iter()
                .find(|item| item.id == id).ok_or("download not found")?;
            state.app_store.edit_downloads(profile, |items| {
                if let Some(item) = items.iter_mut().find(|item| item.id == id) {
                    item.status = DownloadStatus::Downloading;
                    item.error_message = None;
                }
            }).map_err(|e| e.to_string())?;
            state.downloads.start(state.app_store.clone(), profile, id, item.source_url, dir);
        }
        "remove" => {
            state.downloads.cancel(&id);
            if let Some(item) = state.app_store.downloads(profile).map_err(|e| e.to_string())?.into_iter().find(|item| item.id == id) {
                downloads::remove_files(&dir, &item.file_name);
            }
            state.app_store.edit_downloads(profile, |items| items.retain(|item| item.id != id)).map_err(|e| e.to_string())?;
        }
        "clearCompleted" => {
            let done: Vec<DownloadItem> = state.app_store.downloads(profile).map_err(|e| e.to_string())?
                .into_iter().filter(|item| item.status == DownloadStatus::Completed).collect();
            for item in &done { downloads::remove_files(&dir, &item.file_name); }
            let ids: Vec<&str> = done.iter().map(|item| item.id.as_str()).collect();
            state.app_store.edit_downloads(profile, |items| items.retain(|item| !ids.contains(&item.id.as_str()))).map_err(|e| e.to_string())?;
        }
        _ => return Err("unknown download action".into()),
    }
    download_list(state)
}

/// Plays a finished download from its local file, without re-contacting the addon.
#[tauri::command]
pub async fn download_play(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    start_seconds: Option<f64>,
) -> Result<crate::PlayerInfo, String> {
    let profile = state.active_profile();
    let item = state.app_store.downloads(profile).map_err(|e| e.to_string())?.into_iter()
        .find(|item| item.id == id).ok_or("download not found")?;
    let path = item.local_file_uri.filter(|p| std::path::Path::new(p).exists()).ok_or("download is not available locally")?;
    let settings = state.app_store.profile_settings(profile).map(|s| s.player).unwrap_or_default();
    *state.video.lock().expect("video lock") = playback::video_config(&settings);
    Ok(state.open(&app, path, Vec::new(), start_seconds.unwrap_or(0.0))?)
}

/// Effective shuffle settings for one title (`EpisodeShuffleRepository.settings`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShuffleView {
    pub available: bool,
    pub enabled: bool,
    pub include_watched: bool,
}

fn shuffle_view_for(state: &AppState, content_id: &str, content_type: &str) -> Result<ShuffleView, String> {
    let profile = state.active_profile();
    let profile_state = state.app_store.shuffle_profile(profile).map_err(|e| e.to_string())?;
    let saved = profile_state.shows.get(content_id).cloned().unwrap_or_default();
    let effective = ShuffleSettings { enabled: saved.enabled, include_watched: saved.include_watched }
        .effective(profile_state.available, content_type);
    Ok(ShuffleView { available: profile_state.available, enabled: effective.enabled, include_watched: effective.include_watched })
}

#[tauri::command]
pub fn shuffle_view(state: State<'_, AppState>, content_id: String, content_type: String) -> Result<ShuffleView, String> {
    shuffle_view_for(&state, &content_id, &content_type)
}

#[tauri::command]
pub fn shuffle_set(state: State<'_, AppState>, content_id: String, enabled: bool, include_watched: bool) -> Result<ShuffleView, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    state
        .app_store
        .set_shuffle(profile, content_id.clone(), nuvio_store::nextgen::ShuffleShowSettings { enabled, include_watched })
        .map_err(|e| e.to_string())?;
    state.request_sync();
    // The content type is not needed to read the saved toggle here.
    shuffle_view_for(&state, &content_id, "series")
}

/// Picks a random episode with the original's `RandomEpisodePicker` rules.
#[tauri::command]
pub async fn shuffle_pick(
    state: State<'_, AppState>,
    content_id: String,
    content_type: String,
    surface: String,
    current_season: Option<i64>,
    current_episode: Option<i64>,
) -> Result<Option<MetaVideo>, String> {
    let view = shuffle_view_for(&state, &content_id, &content_type)?;
    if !view.enabled {
        return Ok(None);
    }
    let loaded = state.addons.loaded().await?;
    let details = meta::details(&state.addons.client, &loaded.manifests, &content_type, &content_id).await?;
    let profile = state.active_profile();
    let mut watched: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
    for item in state.app_store.watched(profile).map_err(|e| e.to_string())? {
        if let (Some(season), Some(episode)) = (item.season, item.episode) {
            watched.insert((season as i64, episode as i64));
        }
    }
    for entry in state.app_store.progress(profile).map_err(|e| e.to_string())? {
        if entry.effectively_completed() {
            if let (Some(season), Some(episode)) = (entry.season_number, entry.episode_number) {
                watched.insert((season as i64, episode as i64));
            }
        }
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut picker = Picker::new(&details.meta.videos, &watched, &home::today_iso(), seed);
    let current = current_season.zip(current_episode);
    let consume = surface != "playback" || current.is_none();
    Ok(picker.pick(view.include_watched, current, consume))
}

/// Opens an `externalUrl` source in the browser, as the original does. Only http(s) links are
/// accepted, and the UI never sees the stored stream directly, only its opaque key.
#[tauri::command]
pub fn stream_open_external(state: State<'_, AppState>, key: String) -> Result<(), String> {
    let stream = state.streams.get(&key).ok_or("unknown stream (search again)")?;
    let url = stream
        .external_url
        .as_deref()
        .map(str::trim)
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .ok_or("external link is not an http(s) URL")?
        .to_string();
    let spawned = {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn()
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(&url).spawn()
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            std::process::Command::new("xdg-open").arg(&url).spawn()
        }
    };
    spawned.map_err(|e| format!("could not open the browser: {e}"))?;
    Ok(())
}

/// MDBList Ratings settings of the active profile (key never returned).
#[tauri::command]
pub fn mdblist_settings(state: State<'_, AppState>) -> mdblist::View {
    mdblist::view(&state.app_store, state.active_profile())
}

#[tauri::command]
pub fn mdblist_settings_set(state: State<'_, AppState>, settings: mdblist::SettingsInput) -> Result<mdblist::View, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let view = mdblist::update(&state.app_store, profile, settings)?;
    state.request_sync();
    Ok(view)
}

/// Manually marks (or unmarks) a movie/episode as watched (`WatchedRepository.markWatched`).
#[tauri::command]
pub fn library_toggle_watched(state: State<'_, AppState>, item: nuvio_store::library::WatchedItem) -> Result<bool, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let added = state.app_store.toggle_watched(profile, item).map_err(|e| e.to_string())?;
    state.request_sync();
    Ok(added)
}

/// Connected Services (debrid) of the active profile; the key itself never leaves Rust.
#[tauri::command]
pub async fn debrid_settings(state: State<'_, AppState>) -> Result<debrid::View, String> {
    Ok(debrid::view(&state.app_store, state.active_profile()).await)
}

/// Saves the Connected Services page (and sends a key entered here with the account's credentials
/// on the next sync).
#[tauri::command]
pub async fn debrid_settings_set(state: State<'_, AppState>, settings: debrid::SettingsInput) -> Result<debrid::View, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let view = debrid::update(&state.app_store, profile, settings).await?;
    state.request_sync();
    Ok(view)
}

/// TMDB settings of the active profile (the key itself never leaves Rust; `apiKeyFromOriginal`
/// says the original app's key is the one in use).
#[tauri::command]
pub fn tmdb_settings(state: State<'_, AppState>) -> tmdb::SettingsView {
    tmdb::view(&state.app_store, state.active_profile())
}

/// Saves the TMDB page: the key only when the field was edited (`apiKey` null keeps it, "" clears
/// Astra's own key so the original app's is used again).
#[tauri::command]
pub fn tmdb_settings_set(state: State<'_, AppState>, settings: tmdb::SettingsInput) -> Result<tmdb::SettingsView, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    tmdb::update(&state.app_store, profile, settings)
}

/// "More Like This" of a title: TMDB recommendations (`/movie|tv/{id}/recommendations`), resolved
/// from the title's `tmdb:<id>`, numeric or IMDb id. Empty when no TMDB key is configured or the
/// title is unknown to TMDB; unreleased titles are dropped (as the original).
#[tauri::command]
pub async fn more_like_this(state: State<'_, AppState>, content_type: String, id: String) -> Result<Vec<MetaPreview>, String> {
    let loaded = state.addons.loaded().await?;
    if !tmdb::more_like_this_enabled(&state.app_store, loaded.profile) {
        return Ok(Vec::new());
    }
    let Some(access) = tmdb::access(&state.app_store, loaded.profile) else { return Ok(Vec::new()) };
    let mut items = tmdb::more_like_this(&state.addons.client, &access, &content_type, &id).await;
    let today = home::today_iso();
    items.retain(|item| !home::is_unreleased(item, &today));
    Ok(items)
}

/// Badge rules imported in the original app for the active profile (read only). Empty when it never
/// imported a set: the sources still show each addon's own text.
#[tauri::command]
pub fn stream_badge_rules(state: State<'_, AppState>) -> badges::Rules {
    badges::rules(state.active_profile())
}

/// Resolves one channel, on click only: just the addon that listed it is asked (no other addon,
/// no prefetch). The UI then plays the chosen stream key with `stream_play`.
#[tauri::command]
pub async fn livetv_resolve(state: State<'_, AppState>, key: String) -> Result<Vec<StreamGroup>, String> {
    let loaded = state.addons.loaded().await?;
    let (instance, content_type, meta_id) = livetv::split_key(&key).ok_or("invalid channel key")?;
    let owner: Vec<_> = loaded.manifests.iter().filter(|m| addons::addon_instance_id(m) == instance).cloned().collect();
    if owner.is_empty() {
        return Err("the addon of this channel is not enabled".into());
    }
    let debrid = debrid::access(&state.app_store, loaded.profile).is_some();
    Ok(streams::search_in(&state.addons.client, &state.streams, &owner, content_type, meta_id, false, debrid).await)
}

/// Program guide of one channel (see `services::epg`): asked only for the channel the user picked,
/// only to the addon that listed it, reused until the current program ends.
#[tauri::command]
pub async fn livetv_epg(state: State<'_, AppState>, key: String, tz_offset_minutes: i64) -> Result<crate::services::epg::EpgView, String> {
    use crate::services::epg;
    let now = library::now_ms();
    if let Some(view) = state.epg.cached(&key, now) {
        return Ok(view);
    }
    let (instance, content_type, meta_id) = livetv::split_key(&key).ok_or("invalid channel key")?;
    let loaded = state.addons.loaded().await?;
    let Some(owner) = loaded.manifests.iter().find(|m| addons::addon_instance_id(m) == instance) else {
        return Ok(state.epg.update(&key, Vec::new(), now));
    };
    if !owner.supports_resource("meta", content_type, meta_id) {
        return Ok(state.epg.update(&key, Vec::new(), now));
    }
    let url = nuvio_core::url::build_resource_url(&owner.transport_url, "meta", content_type, meta_id, None);
    let text = state.addons.client.get_text(&url, true).await.map_err(|e| e.to_string())?;
    let description = nuvio_core::meta::parse_meta(&text).ok().and_then(|m| m.description).unwrap_or_default();
    Ok(state.epg.update(&key, epg::parse_schedule(&description, now, tz_offset_minutes.clamp(-14 * 60, 14 * 60)), now))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub profile: addons::ProfileInfo,
    pub settings: ProfileSettings,
    pub defaults: ProfileSettings,
    pub catalogs: Vec<home::CatalogChoice>,
    /// What "Idioma do dispositivo" resolves to on this PC.
    pub device_languages: Vec<String>,
    pub data_dir: String,
    pub migration: Option<MigrationRecord>,
    pub app_version: &'static str,
}

/// Settings of the active profile (Settings page). Everything here is applied by NextGen.
#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> Result<SettingsView, String> {
    state.ensure_unlocked()?;
    let loaded = state.addons.loaded().await?;
    let (_, profiles) = addons::profiles(&state.app_store)?;
    let profile = profiles
        .into_iter()
        .find(|p| p.index == loaded.profile)
        .unwrap_or(addons::ProfileInfo {
            index: loaded.profile,
            name: format!("Perfil {}", loaded.profile),
            avatar_id: None,
            avatar_color_hex: nuvio_store::profiles::PROFILE_COLORS[0].into(),
            uses_primary_addons: false,
            pin_enabled: false,
        });
    let settings = state.app_store.profile_settings(loaded.profile).map_err(|e| e.to_string())?;
    let collections = collections::views(&collections::stored(&state.app_store, loaded.profile), &loaded.manifests, settings.home.show_catalog_type, false);
    let catalogs = home::catalog_choices(&loaded.manifests, &settings.home, &collections);
    Ok(SettingsView {
        profile,
        settings,
        defaults: ProfileSettings::default(),
        catalogs,
        device_languages: playback::device_languages(),
        data_dir: state.app_store.dir().display().to_string(),
        migration: state.app_store.migration().ok().flatten(),
        app_version: env!("CARGO_PKG_VERSION"),
    })
}

/// Saves settings (validated into the original's ranges) and returns what was stored. Player
/// settings apply from the next playback; Home settings on the next Home load.
///
/// `previous`: what the editor had loaded; only what differs from it is written, so a screen left
/// open never undoes what the account sent. `profile`: the profile being edited (a save still
/// waiting when the profile is switched must not land on the new one). The change is then sent.
#[tauri::command]
pub async fn settings_set(state: State<'_, AppState>, settings: ProfileSettings, previous: Option<ProfileSettings>, profile: Option<i64>) -> Result<ProfileSettings, String> {
    state.ensure_unlocked()?;
    let active = state.active_profile();
    let profile = profile.unwrap_or(active);
    let mut settings = settings;
    if let Some(before) = previous.as_ref().filter(|p| profile == active && settings.home.items.is_empty() && !p.home.items.is_empty()) {
        // "Restaurar padrões" of Início: the default order is written out (`resetToDefaults`
        // normalizes it), or the account would keep the old one.
        if let Ok(loaded) = state.addons.loaded().await {
            let views = collections::views(&collections::stored(&state.app_store, profile), &loaded.manifests, before.home.show_catalog_type, false);
            settings.home.items = home::catalog_choices(&loaded.manifests, &settings.home, &views)
                .into_iter()
                .zip(0..)
                .map(|(choice, order)| nuvio_store::HomeCatalogPreference { key: choice.key, order, ..Default::default() })
                .collect();
        }
    }
    let saved = match &previous {
        Some(previous) => state.app_store.edit_profile_settings(profile, previous, &settings),
        None => state.app_store.set_profile_settings(profile, settings),
    }
    .map_err(|e| e.to_string())?;
    state.request_sync();
    Ok(saved)
}

// ---- Subtitle fonts (services::fonts)

#[tauri::command]
pub async fn subtitle_fonts(state: State<'_, AppState>) -> Result<fonts::FontList, String> {
    Ok(fonts::list(&state.app_store))
}

/// Opens the Windows file dialog. `None` = cancelled.
#[tauri::command]
pub async fn subtitle_font_import<R: tauri::Runtime>(window: tauri::WebviewWindow<R>, state: State<'_, AppState>) -> Result<Option<fonts::Font>, String> {
    #[cfg(windows)]
    {
        let owner = window.hwnd().map(|h| h.0 as isize).unwrap_or_default();
        let picked = tauri::async_runtime::spawn_blocking(move || fonts::choose_file(owner)).await.map_err(|e| e.to_string())?;
        picked.map(|path| fonts::import(&state.app_store, &path)).transpose()
    }
    #[cfg(not(windows))]
    {
        let _ = (window, state);
        Ok(None)
    }
}

#[tauri::command]
pub async fn subtitle_font_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    fonts::remove(&state.app_store, &id)
}

#[tauri::command]
pub async fn subtitle_font_data(state: State<'_, AppState>, id: String) -> Result<tauri::ipc::Response, String> {
    fonts::data(&state.app_store, &id).map(tauri::ipc::Response::new)
}

// ---- Library (saved items, Continue watching, history), active profile only ----
// Everything below reads NextGen's own documents: no addon request is made.

/// `"<type>/<id>"` of every saved item, for the favorite marks on cards (small, read once).
#[tauri::command]
pub async fn library_favorite_keys(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    Ok(state.app_store.favorites(profile).map_err(|e| e.to_string())?.iter().map(|i| i.key()).collect())
}

/// Saved movies and series, newest first (the display data saved with them).
#[tauri::command]
pub async fn library_favorites(state: State<'_, AppState>) -> Result<Vec<library::LibraryItem>, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    state.app_store.favorites(profile).map_err(|e| e.to_string())
}

/// Saves or removes a title (its type and id as the addon gave them); returns whether it is saved.
#[tauri::command]
pub async fn library_toggle_favorite(state: State<'_, AppState>, item: library::LibraryItem) -> Result<bool, String> {
    if item.id.trim().is_empty() || item.content_type.trim().is_empty() {
        return Err("item without type or id".into());
    }
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let saved = state.app_store.toggle_favorite(profile, item).map_err(|e| e.to_string())?;
    state.request_sync();
    Ok(saved)
}

#[tauri::command]
pub async fn library_remove_favorite(state: State<'_, AppState>, content_type: String, id: String) -> Result<bool, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let removed = state.app_store.remove_favorite(profile, &content_type, &id).map_err(|e| e.to_string())?;
    state.request_sync();
    Ok(removed)
}

/// Continue watching (original rule: latest episode per series, unfinished movies, at most 20).
#[tauri::command]
pub async fn library_continue_watching(state: State<'_, AppState>) -> Result<Vec<library::HistoryEntry>, String> {
    state.ensure_unlocked()?;
    continue_watching(&state, state.active_profile())
}

/// In progress + next episodes whose lists are known (nothing is asked to the addons).
fn continue_watching(state: &AppState, profile: i64) -> Result<Vec<library::HistoryEntry>, String> {
    let progress = state.app_store.progress(profile).map_err(|e| e.to_string())?;
    let seeds = state.app_store.next_up_seeds(profile, &progress).map_err(|e| e.to_string())?;
    let next = state.next_up.cards(profile, &seeds, &progress, library::now_ms());
    let current = library::continue_watching(&progress, library::CONTINUE_WATCHING_LIMIT).iter().map(library::HistoryEntry::from_progress).collect();
    Ok(nuvio_store::next_up::merge(current, next, library::CONTINUE_WATCHING_LIMIT))
}

/// Asks the addons for the episode lists behind the next episodes and returns the completed row.
#[tauri::command]
pub async fn library_next_up(state: State<'_, AppState>) -> Result<Vec<library::HistoryEntry>, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let progress = state.app_store.progress(profile).map_err(|e| e.to_string())?;
    let seeds = state.app_store.next_up_seeds(profile, &progress).map_err(|e| e.to_string())?;
    if !seeds.is_empty() {
        let loaded = state.addons.loaded().await?;
        state.next_up.refresh(&state.addons.client, &loaded.manifests, profile, &seeds).await;
    }
    continue_watching(&state, profile)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<library::HistoryEntry>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

/// One page of the playback history (newest first).
#[tauri::command]
pub async fn library_history(state: State<'_, AppState>, offset: usize, limit: usize) -> Result<HistoryPage, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let progress = state.app_store.progress(profile).map_err(|e| e.to_string())?;
    let watched = state.app_store.watched(profile).map_err(|e| e.to_string())?;
    let all = library::history(&progress, &watched);
    let total = all.len();
    let limit = limit.clamp(1, 200);
    let items: Vec<_> = all.into_iter().skip(offset).take(limit).collect();
    let end = offset + items.len();
    Ok(HistoryPage { items, total, next_offset: (end < total).then_some(end) })
}

/// Removes a movie/episode from the history (its progress and its watched mark).
#[tauri::command]
pub async fn library_remove_history<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: State<'_, AppState>, key: String) -> Result<bool, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let removed = match key.strip_prefix("next:") {
        // A next episode has nothing stored: the series leaves the row until more of it is watched.
        Some(series) => state.app_store.dismiss_next_up(profile, series).map_err(|e| e.to_string())?,
        None => {
            let started = state.app_store.progress(profile).map_err(|e| e.to_string())?.into_iter().find(|e| e.key() == key && e.in_progress());
            let removed = state.app_store.remove_history(profile, &key).map_err(|e| e.to_string())?;
            // Without this the episode just removed would return as that series' next one.
            if let Some(entry) = started.filter(|_| removed) {
                state.app_store.dismiss_next_up(profile, &entry.parent_meta_id).map_err(|e| e.to_string())?;
            }
            removed
        }
    };
    if removed {
        use tauri::Emitter;
        let _ = app.emit("library-changed", ());
        state.request_sync();
    }
    Ok(removed)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleProgress {
    /// Where "Continuar" resumes (newest unfinished entry of the title), if any.
    pub resume: Option<library::HistoryEntry>,
    /// Every movie/episode of the title with progress or watched mark.
    pub entries: Vec<library::HistoryEntry>,
}

/// Progress of one title for the details page (per episode for series).
#[tauri::command]
pub async fn library_title_progress(state: State<'_, AppState>, id: String) -> Result<TitleProgress, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let progress: Vec<_> = state.app_store.progress(profile).map_err(|e| e.to_string())?.into_iter().filter(|e| e.parent_meta_id == id).collect();
    let watched: Vec<_> = state.app_store.watched(profile).map_err(|e| e.to_string())?.into_iter().filter(|w| w.id == id).collect();
    Ok(TitleProgress {
        resume: library::resume_for(&progress, &id).as_ref().map(library::HistoryEntry::from_progress),
        entries: library::history(&progress, &watched),
    })
}

/// Contract of `meta_details` through Tauri's real IPC path (mock runtime): the JSON the UI sends
/// (`{ contentType, id }`, see `web/src/api.ts`) is deserialized by the generated command, the
/// service asks a local addon server over HTTP, and the answer is checked in the UI's casing.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::addons::{AddonContext, LoadedAddons};
    use nuvio_core::client::AddonClient;
    use serde_json::{json, Value};
    use std::io::{BufRead, BufReader, Write};
    use std::sync::atomic::AtomicU64;
    use std::sync::Mutex;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use tauri::Manager;

    /// `mock_context` ships an empty ACL; allow the commands these IPC tests exercise.
    fn mock_ctx() -> tauri::Context<tauri::test::MockRuntime> {
        let mut ctx = mock_context(noop_assets());
        for cmd in [
            "meta_details", "settings_set", "home_rows", "library_favorite_keys", "library_favorites",
            "library_toggle_favorite", "library_remove_favorite", "library_continue_watching",
            "library_history", "library_remove_history", "library_title_progress",
            "discover_sources", "discover_select", "discover_page",
        ] {
            let authority = ctx.runtime_authority_mut();
            authority.__allow_command(cmd.into(), tauri::utils::acl::ExecutionContext::Local);
            authority.__allow_command(
                cmd.into(),
                tauri::utils::acl::ExecutionContext::Remote { url: "http://tauri.localhost/".parse().unwrap() },
            );
        }
        ctx
    }

    const MOVIE: &str = r#"{"meta":{"id":"tt0100","type":"movie","name":"Filme Teste","releaseInfo":"2021","runtime":"1h 40min",
        "genres":["Drama"],"director":["D"],"cast":["A"],"poster":"p.jpg","background":"b.jpg"}}"#;
    const SERIES: &str = r#"{"meta":{"id":"tt0200","type":"series","name":"Série Teste","videos":[
        {"id":"tt0200:1:1","title":"Piloto","season":1,"episode":1},
        {"id":"tt0200:1:2","title":"Dois","season":1,"episode":2},
        {"id":"tt0200:2:1","title":"Volta","season":2,"episode":1}]}}"#;
    const CATALOG: &str = r#"{"metas":[{"id":"tt0100","type":"movie","name":"Filme Teste","releaseInfo":"2021"},
        {"id":"tt0300","type":"movie","name":"Outro Filme"}]}"#;

    /// Minimal addon over HTTP on 127.0.0.1: serves the two metas and the Discover catalog pages
    /// (the same page again for `skip=2`, for the duplicate-page rule), 404 otherwise.
    fn addon_server() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("");
                let (status, body) = match path {
                    "/meta/movie/tt0100.json" => ("200 OK", MOVIE),
                    "/meta/series/tt0200.json" => ("200 OK", SERIES),
                    "/catalog/movie/top.json" | "/catalog/movie/top/skip=2.json" => ("200 OK", CATALOG),
                    "/catalog/movie/genres/genre=Action.json" => ("200 OK", CATALOG),
                    _ => ("404 Not Found", "{}"),
                };
                let mut out = stream;
                let _ = write!(out, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        format!("http://127.0.0.1:{port}/manifest.json")
    }

    fn app(name: &str) -> (tauri::App<tauri::test::MockRuntime>, tauri::WebviewWindow<tauri::test::MockRuntime>, std::path::PathBuf) {
        let manifest_url = addon_server();
        let manifest = nuvio_core::manifest::parse_manifest(
            &manifest_url,
            r#"{"id":"test.meta","name":"Addon de teste","version":"1","resources":["meta"],"types":["movie","series"],"idPrefixes":["tt"],"catalogs":[
                {"type":"movie","id":"top","name":"Populares","extra":[{"name":"skip"}]},
                {"type":"movie","id":"genres","name":"Por gênero","extra":[{"name":"genre","isRequired":true,"options":["Action","Drama"]}]},
                {"type":"movie","id":"search","name":"Busca","extra":[{"name":"search","isRequired":true}]}]}"#,
        )
        .unwrap();
        let client = AddonClient::new().unwrap();
        let loaded = LoadedAddons { profile: 1, addon_profile: 1, manifests: vec![manifest], errors: Default::default(), enabled: 1, failed: 0 };
        let dir = std::env::temp_dir().join(format!("nuvio-api-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState {
            api: Mutex::new(None),
            player: Mutex::new(None),
            session: AtomicU64::new(0),
            hwnd: Mutex::new(0),
            addons: AddonContext::preloaded(client.clone(), loaded),
            client,
            streams: crate::streams::StreamCache::default(),
            downloads: crate::services::downloads::Manager::new(),
            video: Mutex::new(nuvio_player::VideoConfig::default()),
            app_store: nuvio_store::nextgen::AppStore::open(&dir).unwrap(),
            progress: crate::services::progress::ProgressTracker::new(dir.clone()),
            pending_addon: Mutex::new(None),
            account: std::sync::Arc::new(nuvio_sync::Engine::new(dir.clone(), None, None)),
            sync_wake: std::sync::Arc::new(tokio::sync::Notify::new()),
            sync_signals: Default::default(),
            pip: Default::default(),
            pin_gate: Mutex::new(Default::default()),
            p2p: Default::default(),
            p2p_prepared: Mutex::new(None),
            p2p_session: Default::default(),
            youtube: nuvio_core::youtube::Extractor::new().unwrap(),
            updates: Default::default(),
            epg: Default::default(),
            playing: Default::default(),
            subtitle_choice: Default::default(),
            next_up: Default::default(),
        };
        let app = mock_builder()
            .manage(state)
            .invoke_handler(tauri::generate_handler![
                meta_details,
                settings_set,
                home_rows,
                library_favorite_keys,
                library_favorites,
                library_toggle_favorite,
                library_remove_favorite,
                library_continue_watching,
                library_history,
                library_remove_history,
                library_title_progress,
                discover_sources,
                discover_select,
                discover_page
            ])
            .build(mock_ctx())
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        (app, window, dir)
    }

    fn invoke(window: &tauri::WebviewWindow<tauri::test::MockRuntime>, args: Value) -> Result<Value, Value> {
        call(window, "meta_details", args)
    }

    fn call(window: &tauri::WebviewWindow<tauri::test::MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
        get_ipc_response(
            window,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<Value>().unwrap())
    }

    #[test]
    fn meta_details_contract_for_movies_series_and_bad_arguments() {
        let (_app, window, _) = app("meta");

        let movie = invoke(&window, json!({ "contentType": "movie", "id": "tt0100" })).expect("movie details");
        assert_eq!(movie["meta"]["contentType"], "movie");
        assert_eq!(movie["meta"]["name"], "Filme Teste");
        assert_eq!(movie["meta"]["releaseInfo"], "2021");
        assert_eq!(movie["seasons"], json!([]));
        assert_eq!(movie["addonName"], "Addon de teste");

        let series = invoke(&window, json!({ "contentType": "series", "id": "tt0200" })).expect("series details");
        assert_eq!(series["meta"]["contentType"], "series");
        assert_eq!(series["seasons"], json!([1, 2]));
        assert_eq!(series["meta"]["videos"].as_array().unwrap().len(), 3);

        // The original bug: the key absent from the arguments (it was `undefined` in the UI).
        let missing = invoke(&window, json!({ "id": "tt0100" })).unwrap_err();
        assert!(missing.as_str().unwrap().contains("missing required key contentType"), "{missing}");
        // snake_case is not accepted: the UI must send camelCase.
        assert!(invoke(&window, json!({ "content_type": "movie", "id": "tt0100" })).is_err());
        // A type the addon does not serve gives a controlled error, not a guess.
        let wrong = invoke(&window, json!({ "contentType": "tv", "id": "tt0100" })).unwrap_err();
        assert!(wrong.as_str().unwrap().contains("no installed addon"), "{wrong}");
    }

    /// `settings_set` contract: camelCase JSON from the UI, validated into the original's ranges,
    /// persisted for the active profile, and used by `home_rows`.
    #[test]
    fn settings_set_validates_persists_and_reaches_home() {
        let (_app, window, dir) = app("settings");
        let mut settings = serde_json::to_value(ProfileSettings::default()).unwrap();
        settings["player"]["resizeMode"] = json!("Zoom");
        settings["player"]["decoderPriority"] = json!(9); // invalid → default 1
        settings["player"]["subtitle"]["fontSizeSp"] = json!(80); // → 40 (desktop max)
        settings["player"]["preferredAudioLanguage"] = json!("ja");
        settings["home"]["heroEnabled"] = json!(false);
        let saved = call(&window, "settings_set", json!({ "settings": settings })).expect("saved");
        assert_eq!(saved["player"]["resizeMode"], "Zoom");
        assert_eq!(saved["player"]["decoderPriority"], 1);
        assert_eq!(saved["player"]["subtitle"]["fontSizeSp"], 40);

        // Persisted: a fresh store on the same folder reads the same values for profile 1 only.
        let store = nuvio_store::nextgen::AppStore::open(&dir).unwrap();
        let stored = store.profile_settings(1).unwrap();
        assert_eq!(stored.player.preferred_audio_language, "ja");
        assert!(!stored.home.hero_enabled);
        assert_eq!(store.profile_settings(2).unwrap(), ProfileSettings::default());

        let home = call(&window, "home_rows", json!({})).expect("home");
        assert_eq!(home["heroEnabled"], false);
        assert!(call(&window, "settings_set", json!({ "profile_settings": {} })).is_err());
    }

    /// Library commands with the JSON the UI sends (`web/src/library.ts`): favorites by type+id,
    /// progress recorded by the player tracker, Continue watching, paged history and removal.
    #[test]
    fn library_commands_contract() {
        let (app, window, _dir) = app("library");
        let card = json!({ "id": "tt0100", "contentType": "movie", "name": "Filme Teste", "poster": "p.jpg",
            "releaseInfo": "2021", "genres": [], "posterShape": "Poster" });
        assert_eq!(call(&window, "library_toggle_favorite", json!({ "item": card })).unwrap(), true);
        // Same name from another addon (other id) is another item.
        let other = json!({ "id": "tmdb:100", "contentType": "movie", "name": "Filme Teste" });
        assert_eq!(call(&window, "library_toggle_favorite", json!({ "item": other })).unwrap(), true);
        assert_eq!(call(&window, "library_favorite_keys", json!({})).unwrap(), json!(["movie/tmdb:100", "movie/tt0100"]));
        let favorites = call(&window, "library_favorites", json!({})).unwrap();
        assert_eq!(favorites[1]["releaseInfo"], "2021");
        assert_eq!(call(&window, "library_toggle_favorite", json!({ "item": card })).unwrap(), false);
        assert_eq!(call(&window, "library_remove_favorite", json!({ "contentType": "movie", "id": "tmdb:100" })).unwrap(), true);
        assert_eq!(call(&window, "library_favorite_keys", json!({})).unwrap(), json!([]));
        assert!(call(&window, "library_toggle_favorite", json!({ "item": { "id": "x", "contentType": "", "name": "" } })).is_err());

        // Two episodes of one series through the tracker (as `stream_play` + player snapshots do).
        let state = app.state::<AppState>();
        for (session, episode, pos) in [(1, 1, 600.0), (2, 2, 300.0)] {
            let item: library::PlaybackItem = serde_json::from_value(json!({
                "contentType": "series", "parentMetaId": "tt0200", "parentMetaType": "series",
                "videoId": format!("tt0200:1:{episode}"), "title": "Série Teste", "seasonNumber": 1, "episodeNumber": episode
            }))
            .unwrap();
            state.progress.start(session, 1, item, 90);
            state.progress.finish(Some(&nuvio_player::PlayerSnapshot { session, loaded: true, time_pos: pos, duration: 2400.0, ..Default::default() }));
            // History sorts by millisecond timestamp: keep the two finishes distinct.
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let cw = call(&window, "library_continue_watching", json!({})).unwrap();
        assert_eq!(cw.as_array().unwrap().len(), 1, "one entry per series");
        assert_eq!(cw[0]["key"], "tt0200_s1e2");
        assert_eq!(cw[0]["positionMs"], 300_000);
        let title = call(&window, "library_title_progress", json!({ "id": "tt0200" })).unwrap();
        assert_eq!(title["resume"]["episode"], 2);
        assert_eq!(title["entries"].as_array().unwrap().len(), 2, "episodes not mixed");
        let page = call(&window, "library_history", json!({ "offset": 0, "limit": 1 })).unwrap();
        assert_eq!((page["total"].clone(), page["nextOffset"].clone()), (json!(2), json!(1)));
        let page2 = call(&window, "library_history", json!({ "offset": 1, "limit": 1 })).unwrap();
        assert_eq!(page2["items"][0]["key"], "tt0200_s1e1");
        assert_eq!(page2["nextOffset"], Value::Null);
        assert_eq!(call(&window, "library_remove_history", json!({ "key": "tt0200_s1e2" })).unwrap(), true);
        assert_eq!(call(&window, "library_continue_watching", json!({}))
            .unwrap()[0]["key"], "tt0200_s1e1");
    }

    /// Discover commands with the JSON the UI sends (`web/src/screens/Search.tsx`): the listable
    /// catalogs of the enabled addons, the catalog remembered per profile, and a page with the
    /// original's duplicate-page rule.
    #[test]
    fn discover_commands_contract() {
        let (_app, window, dir) = app("discover");

        let view = call(&window, "discover_sources", json!({})).expect("discover sources");
        let sources = view["sources"].as_array().unwrap();
        let keys: Vec<String> = sources.iter().map(|s| s["key"].as_str().unwrap().to_string()).collect();
        assert_eq!(keys.len(), 2, "catalogs with a required search are not listed");
        assert_eq!(sources[0]["catalogId"], "top");
        assert!(keys[0].ends_with(":movie:top"), "{keys:?}");
        assert_eq!(sources[1]["catalogId"], "genres");
        assert_eq!(view["typeOptions"], json!(["movie"]));
        assert_eq!(view["selectedKey"], json!(keys[0]));
        assert_eq!(view["sources"][1]["genreOptions"], json!(["Action", "Drama"]));
        assert_eq!(view["sources"][1]["defaultGenre"], "Action");
        assert!(!view.to_string().contains("/manifest.json"), "no addon URL reaches the UI: {view}");

        // The chosen catalog is remembered per profile.
        call(&window, "discover_select", json!({ "key": keys[1] })).expect("select");
        let store = nuvio_store::nextgen::AppStore::open(&dir).unwrap();
        assert_eq!(store.discover_selection().unwrap()[&1], keys[1]);
        assert_eq!(call(&window, "discover_sources", json!({})).unwrap()["selectedKey"], json!(keys[1]));

        // First page: two items, the catalog paginates (skip extra), next skip is the raw page size.
        let page = call(&window, "discover_page", json!({
            "key": keys[0], "genre": Value::Null, "skip": 0, "reset": true,
            "existingKeys": [], "duplicatePages": 0
        }))
        .expect("first page");
        assert_eq!(page["items"].as_array().unwrap().len(), 2);
        assert_eq!(page["nextSkip"], 2);
        assert_eq!(page["duplicatePages"], 0);
        assert_eq!(page["supportsPagination"], true);

        // The addon answered the same page again: it advances the skip and counts one duplicate.
        let page = call(&window, "discover_page", json!({
            "key": keys[0], "genre": Value::Null, "skip": 2, "reset": false,
            "existingKeys": ["movie:tt0100", "movie:tt0300"], "duplicatePages": 0
        }))
        .unwrap();
        assert_eq!(page["duplicatePages"], 1);
        assert_eq!(page["nextSkip"], 4, "duplicate page advanced by the raw page size");

        // A genre-filtered catalog is asked with the genre in the URL.
        let page = call(&window, "discover_page", json!({
            "key": keys[1], "genre": "Action", "skip": 0, "reset": true,
            "existingKeys": [], "duplicatePages": 0
        }))
        .expect("genre page");
        assert_eq!(page["items"].as_array().unwrap().len(), 2);
        assert_eq!(page["nextSkip"], Value::Null, "no skip extra and a short page: nothing more");

        // A catalog that is no longer installed is refused.
        let gone = call(&window, "discover_page", json!({
            "key": "0000000000000000:movie:gone", "genre": Value::Null, "skip": 0, "reset": true,
            "existingKeys": [], "duplicatePages": 0
        }))
        .unwrap_err();
        assert!(gone.as_str().unwrap().contains("catalog not found"), "{gone}");
    }
}
