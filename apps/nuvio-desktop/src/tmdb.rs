//! Settings → TMDB and the TMDB parts of the details page (`services/tmdb.rs` does the work).
//!
//! The key is the user's own (TMDB gives one for free). It is kept encrypted with DPAPI in
//! `tmdb.dat`, for every profile of this computer, and never goes back to the interface. Until one
//! is saved here, the key the original Nuvio Desktop had for the profile is used (read only). The
//! switches and the language are profile settings (`ProfileSettings.tmdb`).

use crate::services::meta::{self, Details};
use crate::services::tmdb::{self, Access, EntityHeader, MediaType};
use crate::AppState;
use nuvio_core::catalog::{CatalogPage, MetaPreview};
use nuvio_core::meta::select_trailer;
use nuvio_store::settings::TmdbSettings;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;
use tauri::State;

/// Interface language (`pt`, `en`, `ar`): the TMDB language when the setting follows the app.
static UI_LANGUAGE: Mutex<String> = Mutex::new(String::new());

#[tauri::command]
pub fn ui_language_set(language: String) {
    *UI_LANGUAGE.lock().expect("ui language") = language.chars().take(8).collect();
}

#[derive(Serialize, Deserialize)]
struct Saved {
    key: String,
}

fn key_file(state: &AppState) -> std::path::PathBuf {
    state.app_store.dir().join("tmdb.dat")
}

fn saved_key(state: &AppState) -> Option<String> {
    let cipher = std::fs::read(key_file(state)).ok()?;
    let saved: Saved = serde_json::from_slice(&nuvio_sync::crypto::unprotect(&cipher).ok()?).ok()?;
    Some(saved.key).filter(|k| !k.trim().is_empty())
}

/// The key the original app kept for this profile (`nuvio_tmdb_settings`, read only).
fn original_key(profile: i64) -> Option<String> {
    let store = nuvio_store::LegacyStore::open(nuvio_store::default_legacy_data_dir().ok()?).ok()?;
    let values = store.read_store("nuvio_tmdb_settings").ok()?;
    values.get(&format!("tmdb_api_key_{profile}")).map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

fn settings_of(state: &AppState) -> TmdbSettings {
    state.app_store.profile_settings(state.active_profile()).map(|s| s.tmdb).unwrap_or_default()
}

/// The language in use: the setting, else the app's interface language.
fn language(state: &AppState, settings: &TmdbSettings) -> String {
    if !settings.language.trim().is_empty() {
        return tmdb::normalize_language(Some(&settings.language));
    }
    let ui = UI_LANGUAGE.lock().expect("ui language").clone();
    let ui = if ui.is_empty() {
        state.app_store.profile_settings(state.active_profile()).ok().and_then(|s| s.player.app_language).filter(|l| l != "device").map(|l| if l == "pt" { "pt-PT".to_string() } else { l }).unwrap_or_else(|| "pt".into())
    } else {
        ui
    };
    tmdb::app_language_tag(&ui)
}

/// TMDB access of the active profile: `None` when TMDB is off or there is no key.
pub fn access(state: &AppState) -> Option<(Access, TmdbSettings)> {
    let settings = settings_of(state);
    if !settings.enabled {
        return None;
    }
    let key = saved_key(state).or_else(|| original_key(state.active_profile()))?;
    Some((Access::new(key, language(state, &settings)), settings))
}

// ---------------------------------------------------------------- settings page

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `astra`, `original` or `none`.
    key_source: &'static str,
    enabled: bool,
    /// Language really used (`pt-BR`…).
    language: String,
}

#[tauri::command]
pub fn tmdb_status(state: State<'_, AppState>) -> Status {
    let settings = settings_of(&state);
    let key_source = if saved_key(&state).is_some() {
        "astra"
    } else if original_key(state.active_profile()).is_some() {
        "original"
    } else {
        "none"
    };
    Status { key_source, enabled: settings.enabled, language: language(&state, &settings) }
}

/// Checks the key with TMDB, then keeps it (encrypted).
#[tauri::command]
pub async fn tmdb_save_key(state: State<'_, AppState>, key: String) -> Result<Status, String> {
    let key = key.trim().to_string();
    if key.len() < 16 || key.chars().any(char::is_whitespace) {
        return Err("Isso não parece uma chave do TMDB.".into());
    }
    tmdb::check_key(&key).await?;
    let plain = serde_json::to_vec(&Saved { key }).map_err(|e| e.to_string())?;
    let cipher = nuvio_sync::crypto::protect(&plain)?;
    let path = key_file(&state);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, cipher).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())?;
    Ok(tmdb_status(state))
}

#[tauri::command]
pub fn tmdb_remove_key(state: State<'_, AppState>) -> Result<Status, String> {
    let path = key_file(&state);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(tmdb_status(state))
}

// ---------------------------------------------------------------- details page

/// The addon's details of the last titles opened (`meta_enrich` completes them).
static BASE: Mutex<VecDeque<(String, Details)>> = Mutex::new(VecDeque::new());

fn remember(content_type: &str, id: &str, details: &Details) {
    let key = format!("{content_type}/{id}");
    let mut base = BASE.lock().expect("details base");
    base.retain(|(k, _)| *k != key);
    base.push_back((key, details.clone()));
    while base.len() > 12 {
        base.pop_front();
    }
}

fn remembered(content_type: &str, id: &str) -> Option<Details> {
    let key = format!("{content_type}/{id}");
    BASE.lock().expect("details base").iter().find(|(k, _)| *k == key).map(|(_, d)| d.clone())
}

/// Details of a title (`MetaDetailsRepository.load`): the addons first — a `tmdb:` title is asked
/// by its IMDb id — then, when no addon has it and TMDB is on, the page TMDB alone can build.
pub async fn details(state: &AppState, content_type: &str, id: &str) -> Result<Details, String> {
    let loaded = state.addons.loaded().await?;
    let tmdb_access = access(state);
    let mut lookup = id.to_string();
    if let (Some(tid), Some((a, _))) = (tmdb::tmdb_prefix_id(id), &tmdb_access) {
        if let Some(imdb) = tmdb::imdb_id(a, MediaType::parse(Some(content_type)), tid).await {
            lookup = imdb;
        }
    }
    let first = meta::details(&state.addons.client, &loaded.manifests, content_type, &lookup).await;
    let first = match first {
        Err(_) if lookup != id => meta::details(&state.addons.client, &loaded.manifests, content_type, id).await,
        other => other,
    };
    match first {
        Ok(d) => {
            remember(content_type, id, &d);
            Ok(d)
        }
        Err(error) => {
            let (Some(tid), Some((a, s))) = (tmdb::tmdb_prefix_id(id), tmdb_access) else { return Err(error) };
            let (meta, extras) = tmdb::standalone(&a, &s, content_type, tid).await.map_err(|_| error)?;
            let seasons = meta.seasons();
            let trailer = select_trailer(&meta.trailers).cloned();
            Ok(Details { meta, seasons, addon_name: "TMDB".into(), trailer, extras: Some(extras), enriched: true })
        }
    }
}

/// The details just shown, completed from TMDB (`enrichMeta`). An error when TMDB is off, has no
/// key or does not know the title: the page keeps the addon's details.
#[tauri::command]
pub async fn meta_enrich(state: State<'_, AppState>, content_type: String, id: String) -> Result<Details, String> {
    let (a, s) = access(&state).ok_or("TMDB desligado.")?;
    let base = match remembered(&content_type, &id) {
        Some(d) => d,
        None => details(&state, &content_type, &id).await?,
    };
    if base.enriched {
        return Ok(base);
    }
    let Details { meta, addon_name, .. } = base;
    let (meta, extras) = tmdb::enrich(&a, &s, meta).await.map_err(|(_, e)| e)?;
    let seasons = meta.seasons();
    let trailer = select_trailer(&meta.trailers).cloned();
    Ok(Details { meta, seasons, addon_name, trailer, extras: Some(extras), enriched: true })
}

// ---------------------------------------------------------------- person, studio, network

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityPage {
    header: EntityHeader,
    /// A person's titles (all of them); empty for studios and networks (paged).
    titles: Vec<MetaPreview>,
}

fn kind_of(kind: &str) -> Result<&'static str, String> {
    match kind {
        "person" => Ok("person"),
        "company" => Ok("company"),
        "network" => Ok("network"),
        _ => Err("Tipo de página TMDB desconhecido.".into()),
    }
}

#[tauri::command]
pub async fn tmdb_entity(state: State<'_, AppState>, kind: String, id: i64) -> Result<EntityPage, String> {
    let (a, _) = access(&state).ok_or("Configure o TMDB para ver esta página.")?;
    match kind_of(&kind)? {
        "person" => {
            let (header, titles) = tmdb::person_page(&a, id).await?;
            Ok(EntityPage { header, titles })
        }
        k => Ok(EntityPage { header: tmdb::company_header(&a, k, id).await?, titles: Vec::new() }),
    }
}

/// One page of a studio's or network's titles (`media`: `movie` or `series`; `page` 1-based).
#[tauri::command]
pub async fn tmdb_entity_titles(state: State<'_, AppState>, kind: String, id: i64, media: String, page: u32) -> Result<CatalogPage, String> {
    let (a, _) = access(&state).ok_or("Configure o TMDB para ver esta página.")?;
    let kind = kind_of(&kind)?;
    if kind == "person" {
        return Err("Pessoas não têm páginas de títulos.".into());
    }
    tmdb::company_titles(&a, kind, id, MediaType::parse(Some(&media)), page).await
}

/// Whether collection folders may show TMDB sources (TMDB on, a key, "Coleções" on).
pub fn collections_access(state: &AppState) -> Option<Access> {
    access(state).filter(|(_, s)| s.use_collections).map(|(a, _)| a)
}
