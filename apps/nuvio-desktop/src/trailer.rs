//! "Assistir trailer" on the details page: the YouTube trailer of the title (addon meta
//! `trailers`, chosen as the original's `selectHeroTrailer`) is resolved only when the user asks
//! (`nuvio_core::youtube`, the original's in-app extractor) and opened in the app's own player.
//! A trailer is never tracked: no watch progress, history, Continue Watching or account change.

use crate::AppState;
use crate::PlayerInfo;
use tauri::{AppHandle, Manager, State};

/// "Idioma dos trailers": the trailer of `name` in the chosen language, searched on YouTube when
/// clicked. `TRAILER_ORIGINAL` when no language is chosen, `TRAILER_NOT_FOUND` when the search has
/// no fitting video (the UI then plays the addon's trailer, if any).
#[tauri::command]
pub async fn trailer_play_localized(app: AppHandle, state: State<'_, AppState>, name: String, year: Option<String>) -> Result<PlayerInfo, String> {
    let language = state.app_store.profile_settings(state.active_profile()).map(|s| s.trailer.language).unwrap_or_default();
    if nuvio_core::youtube::trailer_language(&language).is_none() {
        return Err("TRAILER_ORIGINAL".into());
    }
    let year = year.as_deref().map(|y| y.chars().take_while(char::is_ascii_digit).collect::<String>()).filter(|y| y.len() == 4);
    let id = state.youtube.search_trailer(&name, year.as_deref(), &language).await?.ok_or("TRAILER_NOT_FOUND")?;
    eprintln!("[trailer] {language}: {id}");
    trailer_play(app, state, id).await
}

#[tauri::command]
pub async fn trailer_play(app: AppHandle, state: State<'_, AppState>, key: String) -> Result<PlayerInfo, String> {
    let id = nuvio_core::youtube::video_id(&key).ok_or("Endereço de trailer inválido.")?;
    let source = state.youtube.resolve(&id).await?;
    eprintln!("[trailer] {} {}p{}", source.mode, source.height, if source.audio_url.is_some() { " + áudio separado" } else { "" });
    let profile = state.active_profile();
    let settings = state.app_store.profile_settings(profile).map(|s| s.player).unwrap_or_default();
    *state.video.lock().expect("video lock") = crate::services::playback::video_config_unenhanced(&settings);
    // Windows are created on the main thread.
    crate::selftest::on_main(&app, move |app| {
        let state = app.state::<AppState>();
        state.open_with_audio(app, source.video_url, source.audio_url, Vec::new(), 0.0)
    })
}
