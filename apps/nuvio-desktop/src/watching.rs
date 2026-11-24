//! What the player knows about the movie or episode that is playing, and its commands: subtitles
//! to choose from, the skip buttons and the next-episode moment.

use crate::services::{self, segments, subtitles};
use crate::AppState;
use nuvio_core::autosync::{compute_alignment_v2, parse_cues};
use nuvio_core::skip::{self, Chapter, Moments, Segment, Threshold};
use nuvio_core::stream::StreamItem;
use nuvio_player::PlayerCommand;
use nuvio_store::library::PlaybackItem;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

pub struct Playing {
    session: u64,
    parent_id: String,
    movie: bool,
    video: subtitles::Video,
    /// Video URL + headers of this playback, used to read embedded subtitle tracks for auto-sync.
    video_url: String,
    video_headers: std::collections::BTreeMap<String, String>,
    target: Option<segments::Target>,
    /// The last list shown (an option's index points into it).
    found: Mutex<Vec<subtitles::Found>>,
    /// Complete answers of this session: addons and services are asked once.
    listed: tokio::sync::Mutex<Option<subtitles::SubtitleList>>,
    segments: tokio::sync::OnceCell<Vec<Vec<Segment>>>,
    /// Subtitle loaded from `found` (auto sync needs its original cues); `None` for file tracks.
    selected: Mutex<Option<usize>>,
    /// One sync at a time per playback.
    syncing: tokio::sync::Mutex<()>,
}

/// Outcome of a subtitle auto-sync run, shown in the subtitle menu.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoSyncStatus {
    pub status: &'static str,
    pub offset_ms: Option<i64>,
    pub references: usize,
}

impl AutoSyncStatus {
    fn of(status: &'static str) -> Self {
        Self { status, offset_ms: None, references: 0 }
    }
}

impl Playing {
    pub fn new(session: u64, item: &PlaybackItem, stream: &StreamItem, url: &str) -> Self {
        let (season, episode) = (item.season_number.map(i64::from), item.episode_number.map(i64::from));
        let movie = item.content_type.eq_ignore_ascii_case("movie") || (season.is_none() && episode.is_none() && item.video_id == item.parent_meta_id);
        let hints = &stream.behavior_hints;
        Self {
            session,
            parent_id: item.parent_meta_id.clone(),
            movie,
            video: subtitles::Video {
                content_type: item.content_type.clone(),
                id: item.video_id.clone(),
                hash: hints.video_hash.clone(),
                size: hints.video_size,
                filename: hints.filename.clone().or_else(|| subtitles::filename_from_url(url)),
                own: stream.subtitles.clone(),
            },
            video_url: url.to_string(),
            video_headers: stream.playback_request_headers().into_iter().collect(),
            target: segments::target(&item.parent_meta_id, &item.video_id, season, episode, movie),
            found: Mutex::default(),
            listed: tokio::sync::Mutex::new(None),
            segments: tokio::sync::OnceCell::new(),
            selected: Mutex::new(None),
            syncing: tokio::sync::Mutex::new(()),
        }
    }
}

/// The subtitle language picked by hand, kept for the next episodes of that title while the app
/// runs (`None`: turned off).
#[derive(Debug, Clone)]
pub struct Choice {
    parent_id: String,
    language: Option<String>,
}

fn playing(state: &AppState, session: u64) -> Result<Arc<Playing>, String> {
    let current = state.playing.lock().expect("playing lock").clone();
    current.filter(|p| p.session == session).ok_or_else(|| "this playback has ended".to_string())
}

/// Subtitle languages to select at start: the one picked for this title, else the settings'.
pub fn subtitle_languages(state: &AppState, parent_id: Option<&str>, preferred: Vec<String>) -> Vec<String> {
    let choice = state.subtitle_choice.lock().expect("subtitle choice").clone();
    match choice.filter(|c| Some(c.parent_id.as_str()) == parent_id) {
        Some(choice) => choice.language.into_iter().collect(),
        None => preferred,
    }
}

fn remember(state: &AppState, language: Option<String>) {
    let parent = state.playing.lock().expect("playing lock").as_ref().map(|p| p.parent_id.clone());
    if let Some(parent_id) = parent {
        *state.subtitle_choice.lock().expect("subtitle choice") = Some(Choice { parent_id, language });
    }
}

/// `SetSubtitleTrack` sent by the UI.
pub fn track_chosen(state: &AppState, id: &str) {
    // A track from the video itself has no downloadable original for auto sync.
    if let Some(current) = state.playing.lock().expect("playing lock").clone() {
        *current.selected.lock().expect("subtitle selected") = None;
    }
    if id == "no" {
        return remember(state, None);
    }
    let Ok(snapshot) = state.with_player(|p| Ok(p.snapshot())) else { return };
    if let Some(track) = snapshot.tracks.iter().find(|t| t.kind == "sub" && t.id.to_string() == id) {
        if let Some(language) = track.lang.clone() {
            remember(state, Some(language));
        }
    }
}

/// Public title id and episode used by the IntroDB submission button in the player.
pub fn submit_target(state: &AppState, session: u64) -> Result<(String, i64, i64), String> {
    let playing = playing(state, session)?;
    match playing.target.as_ref() {
        Some(segments::Target::Imdb { id, episode: Some((season, episode)) }) => Ok((id.clone(), *season, *episode)),
        _ => Err("Este vídeo não tem uma série IMDb com temporada e episódio.".into()),
    }
}

fn credential(state: &AppState, provider: &str) -> String {
    state.app_store.provider_credentials().ok()
        .and_then(|all| all.get(&state.active_profile()).and_then(|p| p.get(provider)).cloned())
        .unwrap_or_default()
}

async fn list(state: &AppState, playing: &Playing) -> Result<subtitles::SubtitleList, String> {
    let mut listed = playing.listed.lock().await;
    if let Some(known) = listed.as_ref() {
        return Ok(known.clone());
    }
    let loaded = state.addons.loaded().await?;
    let (found, list) = subtitles::search(&state.addons.client, &loaded.manifests, &playing.video).await;
    *playing.found.lock().expect("subtitles found") = found;
    // An addon that failed is asked again when the menu opens again.
    if list.failed == 0 {
        *listed = Some(list.clone());
    }
    Ok(list)
}

async fn load(state: &AppState, playing: &Playing, index: usize) -> Result<Option<String>, String> {
    let chosen = playing.found.lock().expect("subtitles found").get(index).cloned().ok_or("unknown subtitle")?;
    let file = subtitles::download(&state.addons.client, &chosen.url, &format!("{}-{index}", playing.session)).await?;
    let language = services::playback::normalize_language(&chosen.language).filter(|l| l != "unknown");
    let command = PlayerCommand::AddSubtitle {
        url: file.to_string_lossy().into_owned(),
        title: Some(chosen.title),
        lang: language.clone(),
    };
    state.with_player(|p| {
        if p.snapshot().session != playing.session {
            return Err("this playback has ended".into());
        }
        p.execute(&command).map_err(|e| e.to_string())?;
        p.execute(&PlayerCommand::SetSubtitleVisible { visible: true }).map_err(|e| e.to_string())
    })?;
    *playing.selected.lock().expect("subtitle selected") = Some(index);
    Ok(language)
}

/// Reads a downloaded subtitle into cues for the sync engine (UTF-8, BOM tolerant for now).
async fn cues_of(client: &nuvio_core::client::AddonClient, url: &str, name: &str) -> Result<Vec<nuvio_core::autosync::SyncCue>, String> {
    let file = subtitles::download(client, url, name).await?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    Ok(parse_cues(&String::from_utf8_lossy(&bytes), url))
}

/// Port of the original's `AutomaticDelaySyncEngine`: compares the loaded subtitle with the other
/// timelines of this title and applies one validated constant delay. No confident match means no
/// change — the original rejects ambiguous or drifting timelines instead of guessing.
async fn auto_sync(state: &AppState, session: u64, manual: bool) -> Result<AutoSyncStatus, String> {
    let profile = state.app_store.profile_settings(state.active_profile()).map_err(|e| e.to_string())?;
    if !profile.player.subtitle.auto_sync_enabled && !manual {
        return Ok(AutoSyncStatus::of("disabled"));
    }
    let playing = playing(state, session)?;
    let Ok(_guard) = playing.syncing.try_lock() else {
        return Ok(AutoSyncStatus::of("busy"));
    };
    let snapshot = state.with_player(|p| Ok(p.snapshot()))?;
    if snapshot.session != session {
        return Err("this playback has ended".into());
    }
    let Some(index) = *playing.selected.lock().expect("subtitle selected") else {
        return Ok(AutoSyncStatus::of("not-applicable"));
    };
    let found: Vec<subtitles::Found> = playing.found.lock().expect("subtitles found").clone();
    let Some(chosen) = found.get(index) else {
        return Ok(AutoSyncStatus::of("not-applicable"));
    };
    let language = services::playback::normalize_language(&chosen.language);
    let target = cues_of(&state.addons.client, &chosen.url, &format!("{session}-autosync-target-{index}")).await?;
    if target.len() < 4 {
        return Ok(AutoSyncStatus::of("not-applicable"));
    }
    let mut references = Vec::new();
    // Embedded subtitle tracks of the video itself are the strongest references (no extra download
    // of a subtitle file); the original reads them from the MP4/MKV index via HTTP ranges.
    if playing.video_url.starts_with("http://") || playing.video_url.starts_with("https://") {
        if let Ok(tracks) = nuvio_core::embedded_subs::load_embedded_tracks(&playing.video_url, &playing.video_headers).await {
            for track in tracks {
                if track.cues.len() >= 4 {
                    references.push(track.cues);
                }
            }
        }
    }
    for (other, option) in found.iter().enumerate() {
        if references.len() >= 3 {
            break;
        }
        if other == index {
            continue;
        }
        if language.is_some() && services::playback::normalize_language(&option.language) != language {
            continue;
        }
        if let Ok(cues) = cues_of(&state.addons.client, &option.url, &format!("{session}-autosync-ref-{other}")).await {
            if cues.len() >= 4 {
                references.push(cues);
            }
        }
        if references.len() >= 3 {
            break;
        }
    }
    if references.is_empty() {
        return Ok(AutoSyncStatus::of("not-applicable"));
    }
    let Some(alignment) = compute_alignment_v2(&target, &references) else {
        return Ok(AutoSyncStatus::of("no-match"));
    };
    let ms = alignment.offset_ms.clamp(-60_000, 60_000);
    state.with_player(|p| p.execute(&PlayerCommand::SetSubtitleDelay { ms }).map_err(|e| e.to_string()))?;
    Ok(AutoSyncStatus { status: "applied", offset_ms: Some(ms), references: references.len() })
}

/// Nothing embedded in a wanted language: the first addon subtitle that has one, unless the user
/// chose something meanwhile (`AddonSubtitleStartupMode`).
pub async fn select_from_addons(app: AppHandle, session: u64, wanted: Vec<String>) {
    let state = app.state::<AppState>();
    let still_off = || {
        state
            .with_player(|p| Ok(p.snapshot()))
            .is_ok_and(|s| s.session == session && s.sid.as_deref().is_none_or(|sid| sid == "no"))
    };
    let choice = |state: &AppState| state.subtitle_choice.lock().expect("subtitle choice").as_ref().map(|c| c.language.clone());
    let before = choice(&state);
    if !still_off() {
        return;
    }
    let Ok(playing) = playing(&state, session) else { return };
    let Ok(list) = list(&state, &playing).await else { return };
    let matched = wanted
        .iter()
        .find_map(|language| list.options.iter().find(|o| services::playback::language_matches(Some(&o.language), language)));
    if let Some(option) = matched {
        if still_off() && choice(&state) == before {
            if load(&state, &playing, option.index).await.is_ok() {
                // Same as the original: a preferred add-on subtitle is synced as soon as it loads.
                let _ = auto_sync(&state, session, false).await;
            }
        }
    }
}

#[tauri::command]
pub async fn player_subtitles(state: State<'_, AppState>, session: u64) -> Result<subtitles::SubtitleList, String> {
    state.ensure_unlocked()?;
    match playing(&state, session) {
        Ok(playing) => list(&state, &playing).await,
        // Live TV and trailers: only the tracks of the file.
        Err(_) => Ok(Default::default()),
    }
}

#[tauri::command]
pub async fn player_subtitle_load(state: State<'_, AppState>, session: u64, index: usize) -> Result<(), String> {
    state.ensure_unlocked()?;
    let playing = playing(&state, session)?;
    let language = load(&state, &playing, index).await?;
    if language.is_some() {
        remember(&state, language);
    }
    Ok(())
}

/// Syncs the loaded add-on subtitle against the other timelines of this title.
#[tauri::command]
pub async fn player_subtitle_autosync(state: State<'_, AppState>, session: u64) -> Result<AutoSyncStatus, String> {
    state.ensure_unlocked()?;
    auto_sync(&state, session, true).await
}

/// Applies the profile's subtitle style to what is playing (the quick adjustments of the menu).
#[tauri::command]
pub async fn player_subtitle_style(state: State<'_, AppState>) -> Result<(), String> {
    let settings = state.app_store.profile_settings(state.active_profile()).map_err(|e| e.to_string())?.player;
    let font = services::fonts::mpv_font(&state.app_store, &settings.subtitle.font_id);
    let style = services::playback::subtitle_style(&settings, font);
    state.with_player(|p| p.execute(&PlayerCommand::ApplySubtitleStyle { style }).map_err(|e| e.to_string()))
}

#[tauri::command]
pub async fn player_moments(state: State<'_, AppState>, session: u64) -> Result<Moments, String> {
    let Ok(playing) = playing(&state, session) else { return Ok(Moments::default()) };
    let settings = state.app_store.profile_settings(state.active_profile()).map_err(|e| e.to_string())?.player;
    let snapshot = state.with_player(|p| Ok(p.snapshot()))?;
    if snapshot.session != session {
        return Ok(Moments::default());
    }
    let mut found = Vec::new();
    if settings.skip_segments_enabled {
        let services = match &playing.target {
            Some(target) => {
                let config = segments::SkipConfig {
                    anime_skip_enabled: settings.skip_segments_enabled && settings.anime_skip_enabled,
                    anime_skip_client_id: credential(&state, "animeskip"),
                };
                playing.segments.get_or_init(|| segments::fetch(&state.addons.client, target, &config)).await.clone()
            }
            None => Vec::new(),
        };
        let chapters: Vec<Chapter> = snapshot.chapters.iter().map(|c| Chapter { time: c.time, title: c.title.clone() }).collect();
        found = skip::merge(skip::from_chapters(&chapters, snapshot.duration, playing.movie), services, playing.movie);
    }
    let threshold = if settings.next_episode_threshold_mode == "MINUTES_BEFORE_END" {
        Threshold::MinutesBeforeEnd(settings.next_episode_threshold_minutes)
    } else {
        Threshold::Percent(settings.next_episode_threshold_percent)
    };
    Ok(skip::plan(&found, snapshot.duration, threshold, &settings.auto_skip_segments))
}
