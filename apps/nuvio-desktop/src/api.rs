//! Typed commands used by the React UI for app data (the player commands are in `main.rs`).
//! Every command returns display data only: addon URLs, stream URLs and tokens stay in Rust.

use crate::services::{addons, collections, fonts, home, livetv, meta, playback, search};
use crate::streams::{self, StreamGroup};
use crate::AppState;
use nuvio_core::catalog::CatalogPage;
use nuvio_store::library;
use nuvio_store::nextgen::MigrationRecord;
use nuvio_store::settings::ProfileSettings;
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
}

/// Home rows of the active profile, with its Home settings from the NextGen store (imported
/// from the original by the migration, edited in Settings → Início).
#[tauri::command]
pub async fn home_rows(state: State<'_, AppState>) -> Result<HomeView, String> {
    let loaded = state.addons.loaded().await?;
    let settings = state.app_store.profile_settings(loaded.profile).map_err(|e| e.to_string())?.home;
    let collections = collections::views_with(&collections::stored(&state.app_store, loaded.profile), &loaded.manifests, settings.show_catalog_type, crate::tmdb::collections_access(&state).is_some());
    Ok(HomeView {
        hero_enabled: settings.hero_enabled,
        hero_sources: home::hero_sources(&loaded.manifests, &settings),
        rows: home::home_rows(&loaded.manifests, Some(&settings), &collections),
    })
}

/// One page of a catalog identified by its home key `<manifestId>:<type>:<catalogId>`; `genre`
/// for the catalogs of a collection folder filtered by genre.
#[tauri::command]
pub async fn catalog_page(state: State<'_, AppState>, key: String, skip: Option<u32>, genre: Option<String>) -> Result<CatalogPage, String> {
    let loaded = state.addons.loaded().await?;
    // A TMDB source of a collection folder (`@tmdb/<collection>/<folder>/<source>`).
    if key.starts_with(collections::TMDB_KEY) {
        let access = crate::tmdb::collections_access(&state).ok_or("TMDB desligado.")?;
        let source = collections::tmdb_source(&collections::stored(&state.app_store, loaded.profile), &key).ok_or("Fonte TMDB não encontrada.")?;
        return crate::services::tmdb::source_page(&access, &source, skip.unwrap_or(1)).await;
    }
    let mut parts = key.splitn(3, ':');
    let (manifest_id, content_type, catalog_id) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let manifest = loaded.manifests.iter().find(|m| m.id == manifest_id).ok_or("addon not enabled")?;
    let mut page = state
        .addons
        .client
        .fetch_catalog_page(manifest, content_type, catalog_id, genre.as_deref().filter(|g| !g.trim().is_empty()), skip)
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

/// Search with results as they arrive: `search-part` events (one per catalog with results), then
/// `search-done`. The interface ignores the events of an older `request_id`.
#[tauri::command]
pub async fn search_stream(app: tauri::AppHandle, state: State<'_, AppState>, query: String, request_id: u64) -> Result<(), String> {
    use tauri::Emitter;
    let loaded = state.addons.loaded().await?;
    let disabled = state.app_store.profile_settings(loaded.profile).map_err(|e| e.to_string())?.search.disabled_addons;
    let manifests = search::searched(&loaded.manifests, &disabled);
    let emitter = app.clone();
    let done = search::search_streamed(&state.addons.client, &manifests, &query, request_id, move |part| {
        let _ = emitter.emit("search-part", part);
    })
    .await;
    let _ = app.emit("search-done", done);
    Ok(())
}

/// Addons with search catalogs and whether the search asks them ("Pesquisar em").
#[tauri::command]
pub async fn search_addons(state: State<'_, AppState>) -> Result<Vec<search::SearchAddon>, String> {
    let loaded = state.addons.loaded().await?;
    let disabled = state.app_store.profile_settings(loaded.profile).map_err(|e| e.to_string())?.search.disabled_addons;
    Ok(search::search_addons(&loaded.manifests, &disabled))
}

/// Turns one addon on or off in the search (this profile, this computer).
#[tauri::command]
pub async fn search_addon_set(state: State<'_, AppState>, key: String, enabled: bool) -> Result<Vec<search::SearchAddon>, String> {
    let loaded = state.addons.loaded().await?;
    let saved = state
        .app_store
        .update_profile_settings(loaded.profile, |s| {
            s.search.disabled_addons.retain(|k| k != &key);
            if !enabled {
                s.search.disabled_addons.push(key.clone());
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(search::search_addons(&loaded.manifests, &saved.search.disabled_addons))
}

/// Catalogs that can be browsed in Descobrir (pages come from `catalog_page`).
#[tauri::command]
pub async fn discover_catalogs(state: State<'_, AppState>) -> Result<Vec<search::DiscoverCatalog>, String> {
    let loaded = state.addons.loaded().await?;
    Ok(search::discover_catalogs(&loaded.manifests))
}

#[tauri::command]
pub async fn meta_details(state: State<'_, AppState>, content_type: String, id: String) -> Result<meta::Details, String> {
    crate::tmdb::details(&state, &content_type, &id).await
}

#[tauri::command]
pub async fn livetv_channels(state: State<'_, AppState>) -> Result<livetv::ChannelList, String> {
    let loaded = state.addons.loaded().await?;
    let list = livetv::load(&state.addons.client, &loaded.manifests).await;
    Ok(crate::playlists::with_playlists(&state, list, false).await)
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
    let list = livetv::load(&state.addons.client, &loaded.manifests).await;
    // The M3U playlists are read again too.
    Ok(crate::playlists::with_playlists(&state, list, true).await)
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
/// `refresh` ("Recarregar" in the sources panel): asks the addons again, past any HTTP cache.
pub async fn title_streams(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    content_type: String,
    id: String,
    refresh: Option<bool>,
    request: Option<u64>,
) -> Result<Vec<StreamGroup>, String> {
    let loaded = state.addons.loaded().await?;
    // `streams-part`: each addon's answer as it arrives, so the list fills in while slower addons
    // are still replying (the complete list in the addons' order is the command's result).
    let on_part = |index: usize, group: &StreamGroup| {
        if let Some(request) = request {
            use tauri::Emitter;
            let _ = app.emit("streams-part", serde_json::json!({ "request": request, "index": index, "group": group }));
        }
    };
    // Scraper plugins run while the addons are still answering; their groups come after them.
    let addons = streams::search_with_parts(&state.addons.client, &state.streams, &loaded.manifests, &content_type, &id, refresh.unwrap_or(false), &on_part);
    let (mut groups, plugins) = tokio::join!(addons, plugin_streams(&state, loaded.profile, &content_type, &id, &on_part));
    groups.extend(plugins);
    Ok(groups)
}

/// Index of the first plugin group in `streams-part` (after any addon).
const PLUGIN_PART: usize = 10_000;

/// The enabled scraper plugins for a title, all at once; each one's sources are announced as soon
/// as it answers. Plugins need the TMDB id: an IMDb id is converted through TMDB when it is set up.
async fn plugin_streams(state: &AppState, profile: i64, content_type: &str, id: &str, on_part: &(dyn Fn(usize, &StreamGroup) + Sync)) -> Vec<StreamGroup> {
    use crate::services::{plugins, tmdb};
    let scrapers = plugins::runnable(&state.app_store, profile, content_type);
    if scrapers.is_empty() {
        return Vec::new();
    }
    let (base, season, episode) = plugins::split_video_id(id);
    let access = crate::tmdb::access(state).map(|(a, _)| a);
    let media = tmdb::MediaType::parse(Some(content_type));
    let tmdb_id = match (tmdb::tmdb_prefix_id(&base), &access) {
        (Some(n), _) => Some(n),
        (None, Some(a)) => tmdb::tmdb_id(a, media, &base, None).await,
        (None, None) => None,
    };
    let Some(tmdb_id) = tmdb_id else { return Vec::new() };
    let key = access.as_ref().map(|a| a.key().to_string()).unwrap_or_default();
    let search = streams::open_search(&state.streams);
    let mut set = tokio::task::JoinSet::new();
    for (index, scraper) in scrapers.into_iter().enumerate() {
        let (tmdb, kind, key) = (tmdb_id.to_string(), content_type.to_string(), key.clone());
        set.spawn(async move {
            let items = plugins::run(&scraper, &tmdb, &kind, season, episode, &key).await;
            (index, scraper.name, items)
        });
    }
    let mut found = Vec::new();
    while let Some(Ok((index, name, items))) = set.join_next().await {
        if items.is_empty() {
            continue;
        }
        let group = streams::add_group(&state.streams, search, PLUGIN_PART + index, format!("{name} · Plugin"), items);
        on_part(PLUGIN_PART + index, &group);
        found.push((index, group));
    }
    found.sort_by_key(|(i, _)| *i);
    found.into_iter().map(|(_, g)| g).collect()
}

// ---------------------------------------------------------------- Settings → Plugins

#[tauri::command]
pub fn plugins_list(state: State<'_, AppState>) -> crate::services::plugins::PluginsView {
    crate::services::plugins::view(&state.app_store, state.active_profile(), None)
}

#[tauri::command]
pub async fn plugins_add(state: State<'_, AppState>, url: String) -> Result<crate::services::plugins::PluginsView, String> {
    state.ensure_unlocked()?;
    crate::services::plugins::add(&state.app_store, &state.addons.client, state.active_profile(), &url).await
}

#[tauri::command]
pub async fn plugins_refresh(state: State<'_, AppState>, key: String) -> Result<crate::services::plugins::PluginsView, String> {
    state.ensure_unlocked()?;
    crate::services::plugins::refresh(&state.app_store, &state.addons.client, state.active_profile(), &key).await
}

#[tauri::command]
pub fn plugins_set_enabled(state: State<'_, AppState>, enabled: bool) -> Result<crate::services::plugins::PluginsView, String> {
    state.ensure_unlocked()?;
    crate::services::plugins::set_enabled(&state.app_store, state.active_profile(), enabled)
}

#[tauri::command]
pub fn plugins_set_scraper_enabled(state: State<'_, AppState>, key: String, enabled: bool) -> Result<crate::services::plugins::PluginsView, String> {
    state.ensure_unlocked()?;
    crate::services::plugins::set_scraper_enabled(&state.app_store, state.active_profile(), &key, enabled)
}

#[tauri::command]
pub fn plugins_remove(state: State<'_, AppState>, key: String) -> Result<crate::services::plugins::PluginsView, String> {
    state.ensure_unlocked()?;
    crate::services::plugins::remove(&state.app_store, state.active_profile(), &key)
}

/// Resolves one channel, on click only: just the addon that listed it is asked (no other addon,
/// no prefetch). The UI then plays the chosen stream key with `stream_play`.
#[tauri::command]
pub async fn livetv_resolve(state: State<'_, AppState>, key: String) -> Result<Vec<StreamGroup>, String> {
    if let Some(result) = crate::playlists::resolve(&state, &key) {
        return result;
    }
    let loaded = state.addons.loaded().await?;
    let (instance, content_type, meta_id) = livetv::split_key(&key).ok_or("invalid channel key")?;
    let owner: Vec<_> = loaded.manifests.iter().filter(|m| addons::addon_instance_id(m) == instance).cloned().collect();
    if owner.is_empty() {
        return Err("the addon of this channel is not enabled".into());
    }
    Ok(streams::search_in(&state.addons.client, &state.streams, &owner, content_type, meta_id).await)
}

/// "Jogos de hoje" (ESPN, nothing from the addons).
#[tauri::command]
pub async fn livetv_matches(tz_offset_minutes: i64) -> Result<crate::services::matches::MatchesView, String> {
    Ok(crate::services::matches::today(library::now_ms(), tz_offset_minutes.clamp(-14 * 60, 14 * 60)).await)
}

/// "Onde passa": for one game only, on click (at most 16 guides of sports channels).
#[tauri::command]
pub async fn livetv_match_channels(
    state: State<'_, AppState>,
    match_id: String,
    channels: Vec<crate::services::matches::ChannelLite>,
    tz_offset_minutes: i64,
) -> Result<Vec<crate::services::matches::ChannelRef>, String> {
    let game = crate::services::matches::cached_match(&match_id).ok_or("Jogo não encontrado; atualize a lista.")?;
    let loaded = state.addons.loaded().await?;
    let tz = tz_offset_minutes.clamp(-14 * 60, 14 * 60);
    Ok(crate::services::matches::where_to_watch(&state.addons.client, &loaded.manifests, &state.epg, &game, &channels, library::now_ms(), tz).await)
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
    let collections = collections::views_with(&collections::stored(&state.app_store, loaded.profile), &loaded.manifests, settings.home.show_catalog_type, crate::tmdb::collections_access(&state).is_some());
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

/// The collection editor (Settings → Coleções): the profile's collections in the editor's shape
/// and the catalogs of the installed addons a folder can use.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionsEdit {
    pub collections: Vec<collections::EditCollection>,
    pub catalogs: Vec<collections::CatalogOption>,
}

#[tauri::command]
pub async fn collections_edit(state: State<'_, AppState>) -> Result<CollectionsEdit, String> {
    state.ensure_unlocked()?;
    let loaded = state.addons.loaded().await?;
    let stored = collections::stored(&state.app_store, loaded.profile);
    Ok(CollectionsEdit { collections: collections::editable(&stored, &loaded.manifests), catalogs: collections::catalog_options(&loaded.manifests) })
}

/// Saves the edited collections (unknown fields kept) and sends them to the account.
#[tauri::command]
pub async fn collections_save(state: State<'_, AppState>, collections_list: Vec<collections::EditCollection>) -> Result<(), String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let stored = collections::stored(&state.app_store, profile);
    let next = collections::apply(&stored, &collections_list);
    state.app_store.set_collections(profile, &next).map_err(|e| e.to_string())?;
    state.request_sync();
    Ok(())
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
            let views = collections::views(&collections::stored(&state.app_store, profile), &loaded.manifests, before.home.show_catalog_type);
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
    let owner = window.hwnd().map(|h| h.0 as isize).unwrap_or_default();
    let picked = tauri::async_runtime::spawn_blocking(move || fonts::choose_file(owner)).await.map_err(|e| e.to_string())?;
    picked.map(|path| fonts::import(&state.app_store, &path)).transpose()
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

/// A movie (no season/episode) or an episode to mark.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchedTarget {
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub video_id: Option<String>,
}

/// "Marcar como assistido" / "Desmarcar" for a movie, episodes or a whole season (`markWatched` /
/// `unmarkWatched` of the original); synced with the account like the automatic marks.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn library_set_watched<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    content_type: String,
    id: String,
    name: String,
    poster: Option<String>,
    release_info: Option<String>,
    targets: Vec<WatchedTarget>,
    watched: bool,
) -> Result<usize, String> {
    state.ensure_unlocked()?;
    let profile = state.active_profile();
    let marks: Vec<crate::services::simkl::Mark> = targets
        .iter()
        .map(|t| crate::services::simkl::Mark { content_type: content_type.clone(), id: id.clone(), season: t.season, episode: t.episode })
        .collect();
    let changed = if watched {
        let now = library::now_ms();
        let items = targets
            .into_iter()
            .map(|t| library::WatchedItem {
                id: id.clone(),
                content_type: content_type.clone(),
                name: name.clone(),
                poster: poster.clone(),
                release_info: release_info.clone(),
                season: t.season,
                episode: t.episode,
                video_id: t.video_id,
                marked_at_epoch_ms: now,
            })
            .collect();
        state.app_store.mark_watched(profile, items).map_err(|e| e.to_string())?
    } else {
        let keys: Vec<(String, Option<i32>, Option<i32>)> = targets.into_iter().map(|t| (id.clone(), t.season, t.episode)).collect();
        state.app_store.unmark_watched(profile, &keys).map_err(|e| e.to_string())?
    };
    if changed > 0 {
        use tauri::Emitter;
        let _ = app.emit("library-changed", ());
        state.request_sync();
        crate::simkl::send_marks(&app, profile, marks, watched);
    }
    Ok(changed)
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

    const MOVIE: &str = r#"{"meta":{"id":"tt0100","type":"movie","name":"Filme Teste","releaseInfo":"2021","runtime":"1h 40min",
        "genres":["Drama"],"director":["D"],"cast":["A"],"poster":"p.jpg","background":"b.jpg"}}"#;
    const SERIES: &str = r#"{"meta":{"id":"tt0200","type":"series","name":"Série Teste","videos":[
        {"id":"tt0200:1:1","title":"Piloto","season":1,"episode":1},
        {"id":"tt0200:1:2","title":"Dois","season":1,"episode":2},
        {"id":"tt0200:2:1","title":"Volta","season":2,"episode":1}]}}"#;

    /// Minimal addon over HTTP on 127.0.0.1: serves the two metas, 404 otherwise.
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
            r#"{"id":"test.meta","name":"Addon de teste","version":"1","resources":["meta"],"types":["movie","series"],"idPrefixes":["tt"],"catalogs":[]}"#,
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
            torbox: nuvio_core::torbox::Torbox::new().unwrap(),
            debrid_client: nuvio_core::debrid::Debrid::new().unwrap(),
            debrid: Default::default(),
            downloads: Default::default(),
            parental: Default::default(),
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
                library_title_progress
            ])
            .build(mock_context(noop_assets()))
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
}

/// Content warnings of a title for the start of playback (Settings → Reprodução can turn them off).
#[tauri::command]
pub async fn parental_guide(state: State<'_, AppState>, id: String) -> Result<Vec<crate::services::parental::Warning>, String> {
    crate::services::parental::guide(&state, id).await
}
