//! Nuvio account in the app: commands, the PIN gate of profiles and the background sync loop.
//!
//! - Session, sync and PIN logic live in `nuvio-sync` (Rust only): the UI receives the account
//!   state (e-mail, sync state, dates, counts), never a token.
//! - PIN gate: a PIN-protected profile is locked when the app starts with it active and when
//!   switching to it; its personal data commands refuse to answer until `profile_unlock` validated
//!   the PIN (account first, offline cache otherwise).
//! - Sync loop (`SyncManager`): 2.5 s after start, then on local changes (3 s debounce: favorites,
//!   history, addons, profiles, and the progress saved on pause / end / seek / close), on profile
//!   switch, every 15 minutes, and on demand. Runs never overlap.
//! - First sync with an account: nothing is written (locally or remotely) until the user reviewed
//!   the read-only preview (`account_sync_preview`) and started it (`account_sync_now(true)`).

use crate::AppState;
use nuvio_sync::{AccountError, Avatar, PinResult, Preview, Report, Status};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

const START_DELAY: Duration = Duration::from_millis(2500);
const CHANGE_DEBOUNCE: Duration = Duration::from_secs(3);
const PERIODIC: Duration = Duration::from_secs(15 * 60);

#[derive(Default)]
pub struct PinGate {
    /// Profiles whose PIN was just validated (consumed by the next switch to them).
    pub unlocked: HashSet<i64>,
    /// Active profile locked until its PIN is entered (app started with a protected profile).
    pub locked_active: Option<i64>,
}

/// Account server of this build (`build.rs`); `None` builds have no account features.
pub fn server_config() -> Option<nuvio_sync::ServerConfig> {
    nuvio_sync::ServerConfig::new(option_env!("NUVIO_SUPABASE_URL")?, option_env!("NUVIO_SUPABASE_FALLBACK_URL").filter(|f| !f.is_empty()), option_env!("NUVIO_SUPABASE_ANON_KEY")?)
}

impl AppState {
    pub fn ensure_unlocked(&self) -> Result<(), String> {
        let gate = self.pin_gate.lock().expect("pin gate");
        if gate.locked_active.is_some_and(|p| p == self.active_profile()) {
            return Err("This profile is locked. Enter the PIN to continue.".into());
        }
        Ok(())
    }

    /// Asks the sync loop for a run (debounced).
    pub fn request_sync(&self) {
        self.sync_wake.notify_one();
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncApplied {
    pub profile: i64,
    pub changed: Vec<String>,
}

/// One run for the active profile, with the events the UI listens to.
pub async fn run_sync<R: Runtime>(app: &AppHandle<R>, confirm_link: bool) -> Result<Report, AccountError> {
    let state = app.state::<AppState>();
    if state.pin_gate.lock().expect("pin gate").locked_active.is_some() {
        return Err(AccountError::Busy);
    }
    let active = state.active_profile();
    let engine = state.account.clone();
    let _ = app.emit("sync-status", Status { state: "syncing".into(), ..engine.status() });
    let result = engine.sync(active, confirm_link).await;
    if let Ok(report) = &result {
        if report.changed.iter().any(|k| k == "profiles" || k == "addons") {
            state.addons.invalidate();
        }
        if !report.changed.is_empty() {
            let _ = app.emit("sync-applied", SyncApplied { profile: report.profile, changed: report.changed.clone() });
            if report.changed.iter().any(|k| matches!(k.as_str(), "library" | "progress" | "watched")) {
                let _ = app.emit("library-changed", ());
            }
        }
    }
    let _ = app.emit("sync-status", engine.status());
    result
}

/// Background sync (signed in only).
pub fn start_loop<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(START_DELAY).await;
        loop {
            let state = app.state::<AppState>();
            if state.account.status().signed_in {
                let _ = run_sync(&app, false).await;
            }
            let wake = state.sync_wake.clone();
            tokio::select! {
                _ = wake.notified() => tokio::time::sleep(CHANGE_DEBOUNCE).await,
                _ = tokio::time::sleep(PERIODIC) => {}
            }
        }
    });
}

fn err(e: AccountError) -> String {
    e.to_string()
}

#[tauri::command]
pub fn account_status(state: State<'_, AppState>) -> Status {
    state.account.status()
}

#[tauri::command]
pub async fn account_sign_in<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, email: String, password: String) -> Result<Status, String> {
    let engine: Arc<nuvio_sync::Engine> = state.account.clone();
    let status = engine.sign_in(&email, &password).await.map_err(err)?;
    state.request_sync();
    let _ = app.emit("sync-status", status.clone());
    Ok(status)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignUpResult {
    pub signed_in: bool,
    pub message: String,
}

#[tauri::command]
pub async fn account_sign_up(state: State<'_, AppState>, email: String, password: String) -> Result<SignUpResult, String> {
    let signed_in = state.account.sign_up(&email, &password).await.map_err(err)?;
    if signed_in {
        state.request_sync();
    }
    Ok(SignUpResult {
        signed_in,
        message: if signed_in { "Account created and signed in.".into() } else { "Account created. Confirm the email sent by Nuvio, then sign in with your password.".into() },
    })
}

/// Signs out: local data and changes not sent yet stay on this computer.
#[tauri::command]
pub async fn account_sign_out<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) -> Result<Status, String> {
    let status = state.account.sign_out().await;
    state.pin_gate.lock().expect("pin gate").unlocked.clear();
    let _ = app.emit("sync-status", status.clone());
    Ok(status)
}

/// "Sincronizar agora" (and "Mesclar" for data of another account): the result once the backend
/// finished.
#[tauri::command]
pub async fn account_sync_now<R: Runtime>(app: AppHandle<R>, confirm_link: bool) -> Result<Status, String> {
    match run_sync(&app, confirm_link).await {
        Ok(_) | Err(AccountError::NeedsReview) => Ok(app.state::<AppState>().account.status()),
        Err(e) => Err(err(e)),
    }
}

/// Read-only comparison of this computer with the account (before the first sync, and as a check).
#[tauri::command]
pub async fn account_sync_preview(state: State<'_, AppState>) -> Result<Preview, String> {
    let engine: Arc<nuvio_sync::Engine> = state.account.clone();
    engine.preview().await.map_err(err)
}

#[tauri::command]
pub async fn avatars_catalog(state: State<'_, AppState>) -> Result<Vec<Avatar>, String> {
    state.account.avatars().await.map_err(err)
}

/// Validates a profile's PIN (before switching to it, or to open the app on it).
#[tauri::command]
pub async fn profile_unlock(state: State<'_, AppState>, index: i64, pin: String) -> Result<PinResult, String> {
    let result = state.account.verify_pin(index, &pin).await;
    if result.unlocked {
        let mut gate = state.pin_gate.lock().expect("pin gate");
        gate.unlocked.insert(index);
        if gate.locked_active == Some(index) {
            gate.locked_active = None;
            gate.unlocked.remove(&index);
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn profile_pin_set(state: State<'_, AppState>, index: i64, pin: String, current_pin: Option<String>) -> Result<crate::manage::ProfilesView, String> {
    state.ensure_unlocked()?;
    state.account.set_pin(index, &pin, current_pin.as_deref()).await.map_err(err)?;
    crate::manage::profiles_view(&state)
}

#[tauri::command]
pub async fn profile_pin_clear(state: State<'_, AppState>, index: i64, current_pin: Option<String>) -> Result<crate::manage::ProfilesView, String> {
    state.ensure_unlocked()?;
    state.account.clear_pin(index, current_pin.as_deref()).await.map_err(err)?;
    crate::manage::profiles_view(&state)
}
