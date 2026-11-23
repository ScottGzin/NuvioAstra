//! Nuvio account in the app: commands, the PIN gate of profiles and the background sync loop.
//!
//! - Session, sync and PIN logic live in `nuvio-sync` (Rust only): the UI receives the account
//!   state (e-mail, sync state, dates, counts), never a token.
//! - PIN gate: a PIN-protected profile is locked when the app starts with it active and when
//!   switching to it; its personal data commands refuse to answer until `profile_unlock` validated
//!   the PIN (account first, offline cache otherwise). While locked only the PIN's state is read
//!   from the account, so removing it in Nuvio lifts the lock here; the lock screen can also sign
//!   in or change profile, so nobody stays stuck.
//! - Sync loop (`SyncManager`), all automatic, runs never overlap, always receiving before sending:
//!   - as soon as the UI is ready (every profile), on sign-in, profile switch and PIN unlock;
//!   - 0.8 s after a local change (favorites, history, addons, profiles, settings, Home order);
//!   - playback position: sent alone each time it is saved (every minute, pause, seek, end, close);
//!   - window focused again (at most every 10 s); every 5 min while focused, 15 min otherwise;
//!   - after a failure: again in 30 s, 1 min, 2 min… up to 15 min;
//!   - closing the window with changes waiting: hidden at once, sent for up to 5 s, then closed.
//! - First sync with an account: signing in starts it (`account_connect`, after a verified backup),
//!   except when this computer's data is another account's: then nothing is written (locally or
//!   remotely) until the user confirms.

use crate::AppState;
use nuvio_sync::{AccountError, Avatar, PinResult, Preview, Report, Status};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// The UI says when it listens (`app_ready`); this is only the limit of that wait.
const START_LIMIT: Duration = Duration::from_millis(2500);
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(800);
const PERIODIC_FOCUSED: Duration = Duration::from_secs(5 * 60);
const PERIODIC: Duration = Duration::from_secs(15 * 60);
const FOCUS_DELAY: Duration = Duration::from_secs(1);
const FOCUS_MIN_INTERVAL: Duration = Duration::from_secs(10);
/// While locked: how often the account is asked whether the PIN is still there.
const RETRY_LOCKED: Duration = Duration::from_secs(15);
const CONNECT_BUSY_ATTEMPTS: u32 = 120;
const WELCOME_DOC: &str = "welcome-seen";
const RETRY_FIRST: Duration = Duration::from_secs(30);
const RETRY_BUSY: Duration = Duration::from_secs(5);
const CLOSE_LIMIT: Duration = Duration::from_secs(5);

/// What wakes the sync loop besides `sync_wake` (a local change).
#[derive(Default)]
pub struct SyncSignals {
    /// A playback position was saved.
    pub progress: tokio::sync::Notify,
    /// The window got the focus.
    pub focus: tokio::sync::Notify,
    /// The UI listens to the sync events.
    pub ready: tokio::sync::Notify,
    closing: AtomicBool,
}

/// Wait before the next automatic run.
fn next_delay(failures: u32, focused: bool) -> Duration {
    match failures {
        0 if focused => PERIODIC_FOCUSED,
        0 => PERIODIC,
        n => (RETRY_FIRST * 2u32.pow((n - 1).min(5))).min(PERIODIC),
    }
}

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
            return Err("Este perfil está bloqueado. Digite o PIN para continuar.".into());
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
    let locked = state.pin_gate.lock().expect("pin gate").locked_active;
    if let Some(index) = locked {
        if !pin_removed(&state, index).await {
            return Err(AccountError::Busy);
        }
        lift_lock(app, index);
    }
    let active = state.active_profile();
    let engine = state.account.clone();
    let _ = app.emit("sync-status", Status { state: "syncing".into(), ..engine.status() });
    let result = engine.sync(active, confirm_link).await;
    if let Ok(report) = &result {
        if report.changed.iter().any(|k| k == "profiles" || k == "addons") {
            state.addons.invalidate();
        }
        // The account deleted the profile on screen: the one that replaced it keeps its protection.
        if state.active_profile() != active {
            lock_if_protected(app);
            let _ = app.emit("profile-lock", ());
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

/// Follows a PIN that was removed elsewhere: the account says so or, for profiles copied from the
/// Nuvio of this computer and not linked yet, that Nuvio does.
async fn pin_removed(state: &AppState, index: i64) -> bool {
    if let Some(protected) = state.account.refresh_pin(index).await {
        return !protected;
    }
    if state.account.linked() || state.account.status().signed_in {
        return false;
    }
    let Ok(nuvio) = crate::services::addons::legacy_store() else { return false };
    let removed = nuvio
        .profiles()
        .ok()
        .flatten()
        .is_some_and(|stored| stored.profiles.iter().any(|p| p.profile_index == index && !p.pin_enabled));
    removed && state.app_store.set_profile_pin_state(index, false).is_ok()
}

fn lift_lock<R: Runtime>(app: &AppHandle<R>, index: i64) {
    let state = app.state::<AppState>();
    {
        let mut gate = state.pin_gate.lock().expect("pin gate");
        if gate.locked_active != Some(index) {
            return;
        }
        gate.locked_active = None;
    }
    let _ = app.emit("profile-lock", ());
    state.request_sync();
}

fn lock_if_protected<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let protected = state.app_store.profiles().ok().and_then(|p| p.get(p.active).filter(|x| x.pin_enabled).map(|x| x.index));
    state.pin_gate.lock().expect("pin gate").locked_active = protected;
}

/// The hero catalogs chosen in the Nuvio of this computer (`nuvio-store/src/hero.rs`).
pub fn follow_nuvio<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let Ok(nuvio) = crate::services::addons::legacy_store() else { return };
    match state.app_store.follow_nuvio_hero(&nuvio) {
        Ok(changed) => {
            let active = state.active_profile();
            if changed.contains(&active) {
                let _ = app.emit("sync-applied", SyncApplied { profile: active, changed: vec!["home".into()] });
            }
        }
        Err(_) => eprintln!("[nextgen] Nuvio's hero choice could not be read"),
    }
}

/// Names and artwork for what the account sent to "Continuar assistindo", in the background.
fn enrich_in_background<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let Ok(loaded) = state.addons.loaded().await else { return };
        if crate::services::enrich::continue_watching(&state.app_store, &state.addons.client, &loaded.manifests, loaded.profile).await {
            let _ = app.emit("library-changed", ());
        }
    });
}

/// Background sync (signed in only). Every run and every position push happens here, one at a time.
pub fn start_loop<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let _ = tokio::time::timeout(START_LIMIT, app.state::<AppState>().sync_signals.ready.notified()).await;
        let mut failures = 0u32;
        loop {
            let state = app.state::<AppState>();
            let mut busy = false;
            follow_nuvio(&app);
            let locked = state.pin_gate.lock().expect("pin gate").locked_active;
            if let Some(index) = locked.filter(|_| !state.account.status().signed_in) {
                if pin_removed(&state, index).await {
                    lift_lock(&app, index);
                }
            }
            if state.account.status().signed_in {
                match run_sync(&app, false).await {
                    Ok(report) => {
                        failures = if report.failed.is_empty() { 0 } else { failures + 1 };
                        enrich_in_background(&app);
                    }
                    // The PIN screen, or the preview / first sync running.
                    Err(AccountError::Busy) => busy = true,
                    Err(AccountError::NeedsReview | AccountError::NotConfigured | AccountError::SignedOut) => failures = 0,
                    Err(_) => failures += 1,
                }
            } else {
                failures = 0;
            }
            let focused = app.get_webview_window("main").is_some_and(|w| w.is_focused().unwrap_or(false));
            let delay = if state.pin_gate.lock().expect("pin gate").locked_active.is_some() {
                RETRY_LOCKED
            } else if busy {
                RETRY_BUSY
            } else {
                next_delay(failures, focused)
            };
            let last = tokio::time::Instant::now();
            loop {
                tokio::select! {
                    _ = state.sync_wake.notified() => {
                        tokio::time::sleep(CHANGE_DEBOUNCE).await;
                        break;
                    }
                    _ = state.sync_signals.focus.notified() => {
                        if last.elapsed() >= FOCUS_MIN_INTERVAL {
                            tokio::time::sleep(FOCUS_DELAY).await;
                            break;
                        }
                    }
                    _ = state.sync_signals.progress.notified() => {
                        // While failing, the retry sends it: no request per saved position.
                        if failures > 0 || busy || !state.account.status().signed_in {
                            continue;
                        }
                        match state.account.push_pending(state.active_profile()).await {
                            Ok(report) if report.failed.is_empty() => {
                                let _ = app.emit("sync-status", state.account.status());
                            }
                            Err(AccountError::NeedsReview) => {}
                            // A full run reports the failure and starts the retries.
                            _ => break,
                        }
                    }
                    _ = tokio::time::sleep_until(last + delay) => break,
                }
            }
        }
    });
}

/// The window is closing: with changes waiting, it hides at once and closes after sending them (at
/// most 5 s; what is left goes on the next start). Returns whether it took over the close.
pub fn send_before_close<R: Runtime>(window: &tauri::Window<R>) -> bool {
    let app = window.app_handle();
    let state = app.state::<AppState>();
    let status = state.account.status();
    if !status.signed_in || status.state == "needsReview" || state.pin_gate.lock().expect("pin gate").locked_active.is_some() {
        return false;
    }
    if state.sync_signals.closing.swap(true, Ordering::SeqCst) {
        return true; // already closing
    }
    state.close(); // the last position joins the queue
    if state.account.status().pending == 0 {
        state.sync_signals.closing.store(false, Ordering::SeqCst);
        return false;
    }
    let _ = window.hide();
    let (app, window) = (app.clone(), window.clone());
    tauri::async_runtime::spawn(async move {
        let send = async {
            while app.state::<AppState>().account.status().pending > 0 {
                match run_sync(&app, false).await {
                    Err(AccountError::Busy) => tokio::time::sleep(Duration::from_millis(250)).await,
                    _ => break,
                }
            }
        };
        let _ = tokio::time::timeout(CLOSE_LIMIT, send).await;
        let _ = window.destroy();
    });
    true
}

/// The UI listens to the sync events: the first run can start.
#[tauri::command]
pub fn app_ready(state: State<'_, AppState>) {
    state.sync_signals.ready.notify_one();
}

/// The account screen at opening: only on a computer never linked to an account, until the user
/// signs in or chooses to go on without one. An install that is linked never sees it.
#[tauri::command]
pub fn welcome_needed(state: State<'_, AppState>) -> bool {
    let seen = || state.app_store.read::<bool>(WELCOME_DOC).ok().flatten().unwrap_or(false);
    state.account.status().configured && !state.account.linked() && !seen()
}

#[tauri::command]
pub fn welcome_done(state: State<'_, AppState>) -> Result<(), String> {
    state.app_store.write(WELCOME_DOC, &true).map_err(|e| e.to_string())
}

/// What follows a sign-in: the account's data is received and joined with this computer's at once.
/// Only data of another account waits for `confirm_other` (the status then carries the question).
#[tauri::command]
pub async fn account_connect<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, confirm_other: bool) -> Result<Status, String> {
    state.ensure_unlocked()?;
    let status = state.account.status();
    if !status.signed_in {
        return Err(err(AccountError::SignedOut));
    }
    if status.link_question.is_some() && !confirm_other {
        return Ok(status);
    }
    let linking = status.state == "needsReview";
    let mut attempts = 0;
    let result = loop {
        match run_sync(&app, true).await {
            // The background loop's own run ends in a moment.
            Err(AccountError::Busy) if attempts < CONNECT_BUSY_ATTEMPTS => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            other => break other,
        }
    };
    if linking && state.account.linked() {
        // A protected profile that just arrived asks for its PIN like at every start.
        lock_if_protected(&app);
        let _ = app.emit("profile-lock", ());
    }
    result.map_err(err)?;
    enrich_in_background(&app);
    state.request_sync();
    Ok(state.account.status())
}

/// "Já removi o PIN": asks again now.
#[tauri::command]
pub async fn profile_pin_refresh<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, index: i64) -> Result<crate::manage::ProfilesView, String> {
    if pin_removed(&state, index).await {
        lift_lock(&app, index);
    }
    crate::manage::profiles_view(&state)
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
        message: if signed_in { "Conta criada e conectada.".into() } else { "Conta criada. Confirme o e-mail enviado pela Nuvio e depois entre com sua senha.".into() },
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

/// Starts the first sync with an account (confirmed by the user on the preview). Every other run is
/// automatic.
#[tauri::command]
pub async fn account_sync_now<R: Runtime>(app: AppHandle<R>, confirm_link: bool) -> Result<Status, String> {
    match run_sync(&app, confirm_link).await {
        Ok(_) => {
            enrich_in_background(&app);
            Ok(app.state::<AppState>().account.status())
        }
        Err(AccountError::NeedsReview) => Ok(app.state::<AppState>().account.status()),
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
    if !state.account.linked() {
        if let Some(cached) = crate::services::addons::legacy_store().ok().and_then(|nuvio| nuvio.pin_cache(index).ok().flatten()) {
            state.account.adopt_pin_cache(index, cached);
        }
    }
    let result = state.account.verify_pin(index, &pin).await;
    if result.unlocked {
        let mut gate = state.pin_gate.lock().expect("pin gate");
        gate.unlocked.insert(index);
        if gate.locked_active == Some(index) {
            gate.locked_active = None;
            gate.unlocked.remove(&index);
            state.request_sync(); // it waited for the PIN
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_runs_follow_focus_and_back_off_after_failures() {
        assert_eq!(next_delay(0, true), PERIODIC_FOCUSED);
        assert_eq!(next_delay(0, false), PERIODIC);
        assert_eq!(next_delay(1, true), Duration::from_secs(30));
        assert_eq!(next_delay(3, false), Duration::from_secs(120));
        assert_eq!(next_delay(40, true), PERIODIC);
    }
}
