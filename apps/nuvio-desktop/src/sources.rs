//! Source behaviours of Settings → Fontes and the source kinds that do not play as a plain link:
//! - "Reutilizar último link" (`stream_reuse_last_link_*` of the original): the link a movie or
//!   episode last played from is kept (encrypted with DPAPI, URLs never reach the UI) and played
//!   again while it is fresh, without asking the addons.
//! - YouTube sources (`ytId`): resolved by the in-app extractor, like trailers.
//! - External links (`externalUrl`): opened in the default browser.

use crate::{play_url, AppState, PlayerInfo};
use nuvio_core::stream::StreamItem;
use nuvio_store::library::PlaybackItem;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{AppHandle, State};

const FILE: &str = "last-links.dat";
/// Entries kept (the newest).
const KEPT: usize = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct LastLink {
    url: String,
    headers: Vec<(String, String)>,
    name: Option<String>,
    addon_name: String,
    filename: Option<String>,
    at_ms: i64,
}

fn entry_key(profile: i64, item: &PlaybackItem) -> String {
    format!("{profile}|{}|{}", item.content_type, item.video_id)
}

fn load(state: &AppState) -> HashMap<String, LastLink> {
    std::fs::read(state.app_store.dir().join(FILE))
        .ok()
        .and_then(|cipher| nuvio_sync::crypto::unprotect(&cipher).ok())
        .and_then(|plain| serde_json::from_slice(&plain).ok())
        .unwrap_or_default()
}

fn save(state: &AppState, mut links: HashMap<String, LastLink>) {
    if links.len() > KEPT {
        let mut by_age: Vec<(String, i64)> = links.iter().map(|(k, v)| (k.clone(), v.at_ms)).collect();
        by_age.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        for (key, _) in by_age.into_iter().skip(KEPT) {
            links.remove(&key);
        }
    }
    let Ok(plain) = serde_json::to_vec(&links) else { return };
    let Ok(cipher) = nuvio_sync::crypto::protect(&plain) else { return };
    let path = state.app_store.dir().join(FILE);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, cipher).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Called when a movie or episode starts from a link (not a P2P session, whose local address
/// changes every time).
pub fn remember(state: &AppState, item: &PlaybackItem, url: &str, headers: &[(String, String)], stream: &StreamItem) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    let mut links = load(state);
    links.insert(
        entry_key(state.active_profile(), item),
        LastLink {
            url: url.to_string(),
            headers: headers.to_vec(),
            name: stream.name.clone().or_else(|| stream.title.clone()),
            addon_name: stream.addon_name.clone(),
            filename: stream.behavior_hints.filename.clone(),
            at_ms: nuvio_store::library::now_ms(),
        },
    );
    save(state, links);
}

fn fresh(link: &LastLink, max_age_hours: i64, now_ms: i64) -> bool {
    now_ms - link.at_ms <= max_age_hours.clamp(1, 24 * 7) * 3_600_000
}

/// Whether "Reutilizar último link" would play this movie/episode right away.
#[tauri::command]
pub fn last_link_has(state: State<'_, AppState>, progress: PlaybackItem) -> bool {
    let settings = state.app_store.profile_settings(state.active_profile()).map(|s| s.sources).unwrap_or_default();
    settings.reuse_last_link
        && load(&state)
            .get(&entry_key(state.active_profile(), &progress))
            .is_some_and(|l| fresh(l, settings.reuse_last_link_hours, nuvio_store::library::now_ms()))
}

/// Plays the last link of this movie/episode when "Reutilizar último link" is on and the link is
/// fresh. `NO_LAST_LINK` otherwise: the UI then asks the addons as usual. Main thread.
#[tauri::command]
pub fn last_link_play(app: AppHandle, state: State<'_, AppState>, start_seconds: Option<f64>, progress: PlaybackItem) -> Result<PlayerInfo, String> {
    state.ensure_unlocked()?;
    let settings = state.app_store.profile_settings(state.active_profile()).map(|s| s.sources).unwrap_or_default();
    if !settings.reuse_last_link {
        return Err("NO_LAST_LINK".into());
    }
    let links = load(&state);
    let link = links
        .get(&entry_key(state.active_profile(), &progress))
        .filter(|l| fresh(l, settings.reuse_last_link_hours, nuvio_store::library::now_ms()))
        .cloned()
        .ok_or("NO_LAST_LINK")?;
    let mut stream = StreamItem { name: link.name.clone(), addon_name: link.addon_name.clone(), ..Default::default() };
    stream.behavior_hints.filename = link.filename.clone();
    play_url(&app, &state, link.url, link.headers, &stream, false, start_seconds, Some(progress), None)
}

/// Plays a YouTube source (`ytId`) of the addons through the in-app extractor.
#[tauri::command]
pub async fn stream_play_youtube(app: AppHandle, state: State<'_, AppState>, key: String, start_seconds: Option<f64>, progress: Option<PlaybackItem>) -> Result<PlayerInfo, String> {
    state.ensure_unlocked()?;
    let stream = state.streams.get(&key).ok_or("unknown stream (search again)")?;
    let id = stream.yt_id.as_deref().and_then(nuvio_core::youtube::video_id).ok_or("Esta fonte não tem um vídeo do YouTube válido.")?;
    let source = state.youtube.resolve(&id).await?;
    crate::selftest::on_main(&app, move |app| {
        use tauri::Manager;
        let state = app.state::<AppState>();
        match source.audio_url {
            // Separate audio (high resolutions): played as one file with an extra audio track.
            Some(audio) => {
                let profile = state.active_profile();
                let settings = state.app_store.profile_settings(profile).map(|s| s.player).unwrap_or_default();
                *state.video.lock().expect("video lock") = crate::services::playback::video_config_unenhanced(&settings);
                state.open_with_audio(app, source.video_url, Some(audio), Vec::new(), start_seconds.unwrap_or(0.0))
            }
            None => play_url(app, &state, source.video_url, Vec::new(), &stream, false, start_seconds, progress, None),
        }
    })
}

/// Opens an external source (`externalUrl`) in the default browser.
#[tauri::command]
pub fn stream_open_external(state: State<'_, AppState>, key: String) -> Result<(), String> {
    let stream = state.streams.get(&key).ok_or("unknown stream (search again)")?;
    let url = stream.external_url.as_deref().map(str::trim).filter(|u| u.starts_with("https://") || u.starts_with("http://")).ok_or("Esta fonte não tem um link externo válido.")?;
    crate::manage::open_in_browser(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_expire_after_the_chosen_hours() {
        let link = LastLink { url: "https://a".into(), headers: vec![], name: None, addon_name: "A".into(), filename: None, at_ms: 0 };
        assert!(fresh(&link, 24, 23 * 3_600_000));
        assert!(!fresh(&link, 24, 25 * 3_600_000));
        assert!(fresh(&link, 168, 100 * 3_600_000));
    }
}
