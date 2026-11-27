//! Settings → Integrações → MDBList: the user's own API key (free, in mdblist.com → Preferences),
//! kept encrypted with DPAPI in `mdblist.dat` for every profile of this computer (updates never
//! touch the data folder), and the ratings it brings to the details page. The key never goes back
//! to the interface. Which ratings are shown is a profile setting (`ProfileSettings.mdblist`).

use crate::services::mdblist::{self, Rating};
use crate::AppState;
use nuvio_store::settings::MdbListSettings;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize, Deserialize)]
struct Saved {
    key: String,
    username: Option<String>,
}

fn key_file(state: &AppState) -> std::path::PathBuf {
    state.app_store.dir().join("mdblist.dat")
}

fn saved(state: &AppState) -> Option<Saved> {
    let cipher = std::fs::read(key_file(state)).ok()?;
    serde_json::from_slice::<Saved>(&nuvio_sync::crypto::unprotect(&cipher).ok()?).ok().filter(|s| !s.key.trim().is_empty())
}

fn settings_of(state: &AppState) -> MdbListSettings {
    state.app_store.profile_settings(state.active_profile()).map(|s| s.mdblist).unwrap_or_default()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    has_key: bool,
    username: Option<String>,
}

#[tauri::command]
pub fn mdblist_status(state: State<'_, AppState>) -> Status {
    let s = saved(&state);
    Status { has_key: s.is_some(), username: s.and_then(|s| s.username) }
}

/// Checks the key with MDBList, then keeps it (encrypted).
#[tauri::command]
pub async fn mdblist_save_key(state: State<'_, AppState>, key: String) -> Result<Status, String> {
    let key = key.trim().to_string();
    if key.len() < 12 || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Isso não parece uma chave do MDBList.".into());
    }
    let username = mdblist::check_key(&key).await?;
    let plain = serde_json::to_vec(&Saved { key, username }).map_err(|e| e.to_string())?;
    let cipher = nuvio_sync::crypto::protect(&plain)?;
    let path = key_file(&state);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, cipher).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())?;
    mdblist::clear_cache();
    Ok(mdblist_status(state))
}

#[tauri::command]
pub fn mdblist_remove_key(state: State<'_, AppState>) -> Result<Status, String> {
    let path = key_file(&state);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    mdblist::clear_cache();
    Ok(mdblist_status(state))
}

/// "Pegar minha chave": the MDBList page where the key is shown.
#[tauri::command]
pub fn mdblist_open_key_page() -> Result<(), String> {
    crate::manage::open_in_browser("https://mdblist.com/preferences/")
}

fn wanted(settings: &MdbListSettings, source: &str) -> bool {
    match source {
        "imdb" => settings.use_imdb,
        "tmdb" => settings.use_tmdb,
        "tomatoes" => settings.use_tomatoes,
        "metacritic" => settings.use_metacritic,
        "trakt" => settings.use_trakt,
        "letterboxd" => settings.use_letterboxd,
        "audience" => settings.use_audience,
        "mal" => settings.use_mal,
        _ => false,
    }
}

/// Ratings of a title for the details page (only the sources switched on). An error when MDBList
/// is off, has no key or does not know the title.
#[tauri::command]
pub async fn mdblist_ratings(state: State<'_, AppState>, content_type: String, id: String) -> Result<Vec<Rating>, String> {
    let settings = settings_of(&state);
    if !settings.enabled {
        return Err("MDBList desligado.".into());
    }
    let key = saved(&state).ok_or("Sem chave do MDBList.")?.key;
    let list = mdblist::ratings(&key, &content_type, &id).await?;
    Ok(list.into_iter().filter(|r| wanted(&settings, &r.source)).collect())
}
