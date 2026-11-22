//! "Assistir trailer" on the details page: the YouTube trailer of the title (addon meta
//! `trailers`, chosen as the original's `selectHeroTrailer`) is resolved only when the user asks
//! (`nuvio_core::youtube`, the original's in-app extractor) and opened in the app's own player.
//! A trailer is never tracked: no watch progress, history, Continue Watching or account change.

use crate::AppState;
use crate::PlayerInfo;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub async fn trailer_play(app: AppHandle, state: State<'_, AppState>, key: String) -> Result<PlayerInfo, String> {
    let id = nuvio_core::youtube::video_id(&key).ok_or("Invalid trailer address.")?;
    let source = state.youtube.resolve(&id).await?;
    eprintln!("[trailer] {} {}p{}", source.mode, source.height, if source.audio_url.is_some() { " + áudio separado" } else { "" });
    let profile = state.active_profile();
    let settings = state.app_store.profile_settings(profile).map(|s| s.player).unwrap_or_default();
    *state.video.lock().expect("video lock") = crate::services::playback::video_config(&settings);
    // Windows are created on the main thread.
    crate::selftest::on_main(&app, move |app| {
        let state = app.state::<AppState>();
        state.open_with_audio(app, source.video_url, source.audio_url, Vec::new(), 0.0)
    })
}
