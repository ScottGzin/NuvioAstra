//! Settings → Integrações → Simkl: "Conectar" signs in by code (AUTH V2 device flow) with the Nuvio
//! Astra app registered at Simkl, built in like the original's `SimklConfig.CLIENT_ID`
//! (`NUVIO_SIMKL_CLIENT_ID`, see build.rs; a public id, there is no secret). The account's token is
//! kept encrypted with DPAPI in `simkl-<profile>.dat` in the data folder, which updates never
//! touch, so the profile stays connected from one version to the next. Then what Astra does with
//! the account: scrobbling the playback, sending the watched marks and bringing what was watched
//! on Simkl. Requests run in the background and never hold the player or the interface.

use crate::services::progress::{ScrobbleAction, ScrobbleEvent};
use crate::services::simkl::{self as api, DeviceCode, DevicePoll, Mark, Token};
use crate::AppState;
use nuvio_store::library::{self, WatchedItem};
use nuvio_store::settings::SimklSettings;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// The Nuvio Astra app at Simkl, built in (none in a build made without it).
fn client_id() -> Option<&'static str> {
    option_env!("NUVIO_SIMKL_CLIENT_ID").filter(|v| !v.is_empty())
}

#[derive(Serialize, Deserialize, Clone)]
struct Session {
    token: Token,
    username: Option<String>,
}

fn session_file(state: &AppState, profile: i64) -> std::path::PathBuf {
    state.app_store.dir().join(format!("simkl-{profile}.dat"))
}

fn session(state: &AppState, profile: i64) -> Option<Session> {
    let cipher = std::fs::read(session_file(state, profile)).ok()?;
    serde_json::from_slice(&nuvio_sync::crypto::unprotect(&cipher).ok()?).ok()
}

fn save_session(state: &AppState, profile: i64, value: &Session) -> Result<(), String> {
    let plain = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let cipher = nuvio_sync::crypto::protect(&plain)?;
    let path = session_file(state, profile);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, cipher).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())
}

fn settings_of(state: &AppState, profile: i64) -> SimklSettings {
    state.app_store.profile_settings(profile).map(|s| s.simkl).unwrap_or_default()
}

/// One refresh at a time: a refresh invalidates the previous access token at once.
static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A valid access token of the profile, refreshed a day before it expires.
async fn access_token(state: &AppState, profile: i64) -> Option<(&'static str, String)> {
    let id = client_id()?;
    let _guard = REFRESH.lock().await;
    let mut current = session(state, profile)?;
    if current.token.expiring(api::now_secs()) {
        match api::refresh(id, &current.token.refresh_token).await {
            Ok(Some(token)) => {
                current.token = token;
                let _ = save_session(state, profile, &current);
            }
            // Simkl refused the refresh token: signed out.
            Ok(None) => {
                let _ = std::fs::remove_file(session_file(state, profile));
                return None;
            }
            // Offline: the current token may still work.
            Err(_) if current.token.expires_at > api::now_secs() => {}
            Err(_) => return None,
        }
    }
    Some((id, current.token.access_token))
}

// ---------------------------------------------------------------- settings page

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// This build has the Simkl app (false only in builds made without it).
    configured: bool,
    connected: bool,
    username: Option<String>,
    last_import_ms: Option<i64>,
}

fn last_import_file(state: &AppState, profile: i64) -> std::path::PathBuf {
    state.app_store.dir().join(format!("simkl-import-{profile}.json"))
}

fn last_import(state: &AppState, profile: i64) -> Option<i64> {
    std::fs::read_to_string(last_import_file(state, profile)).ok()?.trim().parse().ok()
}

#[tauri::command]
pub fn simkl_status(state: State<'_, AppState>) -> Status {
    let profile = state.active_profile();
    let s = session(&state, profile);
    Status { configured: client_id().is_some(), connected: s.is_some(), username: s.and_then(|s| s.username), last_import_ms: last_import(&state, profile) }
}

static PENDING: Mutex<Option<(i64, String)>> = Mutex::new(None);

/// "Conectar": a code, and the Simkl page (with the code filled in) opened in the browser.
#[tauri::command]
pub async fn simkl_device_start(state: State<'_, AppState>, open_browser: bool) -> Result<DeviceCode, String> {
    let id = client_id().ok_or("Esta versão do Nuvio Astra foi gerada sem o Simkl.")?;
    let code = api::device_code(id).await?;
    *PENDING.lock().expect("pending") = Some((state.active_profile(), code.device_code.clone()));
    if open_browser {
        let url = if code.verification_url.starts_with("https://simkl.com/") { code.verification_url.clone() } else { "https://simkl.com/pin".to_string() };
        let _ = crate::manage::open_in_browser(&url);
    }
    Ok(code)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollAnswer {
    /// `pending`, `slowDown`, `connected`, `expired`, `failed`.
    state: &'static str,
    message: Option<String>,
    username: Option<String>,
}

#[tauri::command]
pub async fn simkl_device_poll(app: AppHandle, state: State<'_, AppState>) -> Result<PollAnswer, String> {
    let id = client_id().ok_or("Esta versão do Nuvio Astra foi gerada sem o Simkl.")?;
    let Some((profile, device_code)) = PENDING.lock().expect("pending").clone() else {
        return Ok(PollAnswer { state: "expired", message: None, username: None });
    };
    let answer = |state: &'static str, message: Option<String>| Ok(PollAnswer { state, message, username: None });
    match api::device_poll(id, &device_code).await {
        DevicePoll::Pending => answer("pending", None),
        DevicePoll::SlowDown => answer("slowDown", None),
        DevicePoll::Expired => {
            *PENDING.lock().expect("pending") = None;
            answer("expired", None)
        }
        DevicePoll::Failed(message) => {
            *PENDING.lock().expect("pending") = None;
            answer("failed", Some(message))
        }
        DevicePoll::Authorized(token) => {
            *PENDING.lock().expect("pending") = None;
            let username = api::username(id, &token.access_token).await;
            save_session(&state, profile, &Session { token, username: username.clone() })?;
            if settings_of(&state, profile).import_watched {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    let _ = import(&app, &state, profile).await;
                });
            }
            Ok(PollAnswer { state: "connected", message: None, username })
        }
    }
}

#[tauri::command]
pub fn simkl_device_cancel() {
    *PENDING.lock().expect("pending") = None;
}

/// "Desconectar": the token is revoked at Simkl and removed here.
#[tauri::command]
pub async fn simkl_disconnect(state: State<'_, AppState>) -> Result<Status, String> {
    let profile = state.active_profile();
    if let (Some(id), Some(s)) = (client_id(), session(&state, profile)) {
        api::revoke(id, &s.token.access_token).await;
    }
    let _ = std::fs::remove_file(session_file(&state, profile));
    let _ = std::fs::remove_file(last_import_file(&state, profile));
    Ok(simkl_status(state))
}

// ---------------------------------------------------------------- scrobble (from the player)

/// Called by the progress tracker (player thread): queued, sent in order in the background.
pub fn scrobbler(app: AppHandle) -> impl Fn(ScrobbleEvent) + Send + 'static {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ScrobbleEvent>();
    let worker = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let state = worker.state::<AppState>();
            scrobble(&state, event).await;
        }
    });
    move |event| {
        let _ = tx.send(event);
    }
}

async fn scrobble(state: &AppState, e: ScrobbleEvent) {
    if !settings_of(state, e.profile).scrobble {
        return;
    }
    let Some((id, token)) = access_token(state, e.profile).await else { return };
    let item = &e.item;
    let Some(body) = api::scrobble_body(&item.content_type, &item.parent_meta_id, item.season_number, item.episode_number, e.percent) else { return };
    let action = match e.action {
        ScrobbleAction::Start => api::Scrobble::Start,
        ScrobbleAction::Pause => api::Scrobble::Pause,
        ScrobbleAction::Stop => api::Scrobble::Stop,
    };
    // A stop is tried again; a 20 s write lock (`RATE_LIMIT`) is waited out once.
    let attempts = if action == api::Scrobble::Stop { 3 } else { 1 };
    for attempt in 0..attempts {
        match api::request(reqwest::Method::POST, id, Some(&token), action.path(), &[], api::Body::Json(&body)).await {
            Ok(answer) if answer.ok() || answer.status == 409 || answer.status == 404 => return,
            Ok(answer) if answer.status == 400 && attempt + 1 < attempts => {
                let _ = answer;
                tokio::time::sleep(std::time::Duration::from_secs(21)).await
            }
            _ if attempt + 1 < attempts => tokio::time::sleep(std::time::Duration::from_millis(1500 * (attempt as u64 + 1))).await,
            _ => return,
        }
    }
}

// ---------------------------------------------------------------- watched marks

/// "Marcar como assistido" (or unmarking) of the profile, sent to the Simkl history.
pub fn send_marks<R: tauri::Runtime>(app: &tauri::AppHandle<R>, profile: i64, marks: Vec<Mark>, watched: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if marks.is_empty() || !settings_of(&state, profile).sync_history {
            return;
        }
        let Some((id, token)) = access_token(&state, profile).await else { return };
        let body = api::history_body(&marks);
        let path = if watched { "sync/history" } else { "sync/history/remove" };
        let _ = api::request(reqwest::Method::POST, id, Some(&token), path, &[], api::Body::Json(&body)).await;
    });
}

const IMPORT_EVERY_MS: i64 = 6 * 3600 * 1000;

/// What was watched on Simkl and is not marked here yet becomes watched in the profile.
async fn import(app: &AppHandle, state: &AppState, profile: i64) -> Result<usize, String> {
    let (id, token) = access_token(state, profile).await.ok_or("Conecte o Simkl primeiro.")?;
    let query = [("extended", "full_anime_seasons".to_string()), ("episode_watched_at", "yes".to_string()), ("include_all_episodes", "yes".to_string())];
    let answer = api::get(id, Some(&token), "sync/all-items", &query).await?;
    let now = library::now_ms();
    let items: Vec<WatchedItem> = api::parse_all_items(&answer.body)
        .into_iter()
        .map(|w| WatchedItem {
            video_id: match (w.season, w.episode) {
                (Some(s), Some(e)) => Some(format!("{}:{s}:{e}", w.id)),
                _ => Some(w.id.clone()),
            },
            id: w.id,
            content_type: w.content_type,
            name: w.name,
            poster: None,
            release_info: w.year.map(|y| y.to_string()),
            season: w.season,
            episode: w.episode,
            marked_at_epoch_ms: now,
        })
        .collect();
    let added = state.app_store.mark_watched(profile, items).map_err(|e| e.to_string())?;
    let _ = std::fs::write(last_import_file(state, profile), now.to_string());
    if added > 0 {
        let _ = app.emit("library-changed", ());
        state.request_sync();
    }
    Ok(added)
}

/// "Importar agora".
#[tauri::command]
pub async fn simkl_import(app: AppHandle, state: State<'_, AppState>) -> Result<usize, String> {
    let profile = state.active_profile();
    import(&app, &state, profile).await
}

/// When the app opens: the import, if it is on and the last one is old enough.
pub fn import_on_start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let state = app.state::<AppState>();
        let profile = state.active_profile();
        if !settings_of(&state, profile).import_watched || session(&state, profile).is_none() {
            return;
        }
        if last_import(&state, profile).is_some_and(|at| library::now_ms() - at < IMPORT_EVERY_MS) {
            return;
        }
        let _ = import(&app, &state, profile).await;
    });
}
