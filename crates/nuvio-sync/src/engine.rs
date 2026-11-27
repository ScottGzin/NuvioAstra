//! The account engine: session (sign in/up/out, refresh), sync runs, profile PIN and avatars.
//!
//! A sync run (`sync`) follows the original's ordered profile sync (`SyncManager`):
//! profiles → pending pushes of every profile → pulls of the active profile (addons, library, watch
//! progress, watched marks). Each entity is a step of its own: a failing step is reported and the
//! others go on (`runOrderedProfileSync`); a lost connection stops the run and keeps everything
//! pending for the next one. Runs never overlap (single flight) and a sign-out cancels the running
//! one (generation check before every write).

use crate::crypto;
use crate::remote::{now_s, Remote, RemoteError, ServerConfig, Session, SignUp};
use nuvio_store::library::{LibraryItem, WatchedItem};
use nuvio_store::nextgen::AppStore;
use nuvio_store::HomeCatalogSettings;
use nuvio_store::profiles::{Profile, MAX_PROFILES};
use nuvio_store::settings::PlayerSettings;
use nuvio_store::sync_state::{fresher, MergeStats, RemoteEvent, RemoteProgress};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

const SESSION_FILE: &str = "account.dat";
const PAGE: i64 = 500;
const BATCH: usize = 500;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub finished_at_epoch_ms: i64,
    pub profile: i64,
    pub sent: usize,
    pub received: usize,
    /// Entities that could not be synced in this run (pt-BR names), with the reason.
    pub failed: Vec<String>,
    /// Kinds of local data changed by what the account sent (`profiles`, `addons`, `library`, `progress`, `watched`, `collections`, `home`, `settings`).
    pub changed: Vec<String>,
}

const OTHER_ACCOUNT: &str = "Os dados deste computador vieram de outra conta Nuvio. Ao confirmar, eles serão mesclados com esta conta: nada é apagado, os itens dos dois lados são mantidos.";

/// One kind of data on both sides, by key.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Compare {
    pub local: usize,
    pub remote: usize,
    pub both: usize,
    pub only_local: usize,
    pub only_remote: usize,
    /// Same key, different content (addon enabled state, progress position/date).
    pub differing: usize,
    /// Of `differing`, the local version is newer (kept and sent; progress only).
    pub local_newer: usize,
    /// Records sharing a key with another record of the same side.
    pub local_duplicates: usize,
    pub remote_duplicates: usize,
}

impl Compare {
    fn of_keys<K: Ord>(local: &BTreeSet<K>, local_rows: usize, remote: &BTreeSet<K>, remote_rows: usize) -> Self {
        let both = local.intersection(remote).count();
        Compare {
            local: local.len(),
            remote: remote.len(),
            both,
            only_local: local.len() - both,
            only_remote: remote.len() - both,
            local_duplicates: local_rows.saturating_sub(local.len()),
            remote_duplicates: remote_rows.saturating_sub(remote.len()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePreview {
    pub index: i64,
    pub local_name: Option<String>,
    pub remote_name: Option<String>,
    /// Same profile number on both sides with another name, color or avatar (the account's is kept).
    pub profile_differs: bool,
    pub shares_primary_addons: bool,
    pub addons: Compare,
    pub addon_order_differs: bool,
    pub favorites: Compare,
    pub progress: Compare,
    pub watched: Compare,
    /// Local changes of this profile waiting to be sent.
    pub pending: usize,
}

/// Read-only comparison of this computer with the account (`Engine::preview`).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub generated_at_epoch_ms: i64,
    /// The first sync with this account was not confirmed yet.
    pub first_sync: bool,
    /// This computer's data came from another account.
    pub other_account: bool,
    pub profiles: Compare,
    pub entries: Vec<ProfilePreview>,
    pub pending: usize,
    pub to_send: usize,
    pub to_receive: usize,
    pub conflicts: usize,
}

impl Preview {
    /// Without profile names (for the log).
    fn sanitized(&self) -> Preview {
        let mut p = self.clone();
        for e in &mut p.entries {
            e.local_name = None;
            e.remote_name = None;
        }
        p
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub configured: bool,
    pub signed_in: bool,
    pub email: Option<String>,
    /// `notConfigured`, `signedOut`, `expired`, `idle`, `syncing`, `pending`, `offline`, `error`,
    /// `needsReview` (signed in, first sync with this account not confirmed yet: nothing is written).
    pub state: String,
    pub pending: usize,
    pub last_sync_epoch_ms: i64,
    pub last_error: Option<String>,
    pub last_report: Option<Report>,
    /// Shown with `needsReview` when this computer's data came from another account.
    pub link_question: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AccountError {
    #[error("A conta Nuvio não está configurada nesta versão do Nuvio Astra.")]
    NotConfigured,
    #[error("Entre na sua conta Nuvio para usar este recurso.")]
    SignedOut,
    #[error("Uma sincronização já está em andamento.")]
    Busy,
    #[error("Revise a prévia da primeira sincronização e confirme para começar.")]
    NeedsReview,
    #[error("{0}")]
    Remote(#[from] RemoteError),
    #[error("Erro ao gravar os dados locais: {0}")]
    Store(String),
}

impl From<nuvio_store::StoreError> for AccountError {
    fn from(e: nuvio_store::StoreError) -> Self {
        AccountError::Store(e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PinResult {
    pub unlocked: bool,
    pub retry_after_seconds: i64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Avatar {
    pub id: String,
    pub display_name: String,
    pub category: String,
    pub sort_order: i64,
    pub bg_color: Option<String>,
    /// Public image of the account's avatar storage (no token in it).
    pub image_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PinCacheEntry {
    salt: String,
    digest: String,
    profile_updated_at: String,
}

pub struct Engine {
    dir: PathBuf,
    remote: Option<Remote>,
    session: tokio::sync::Mutex<Option<Session>>,
    run: tokio::sync::Mutex<()>,
    status: Mutex<Status>,
    generation: AtomicU64,
    /// Account id stored with the original app's profiles (`StoredProfilePayload.userId`), if any.
    legacy_owner: Option<String>,
    client_id: String,
    /// A run received the account's data since the app opened or signed in: until then nothing
    /// waiting here is sent alone, and that run reads every profile.
    received: AtomicBool,
    /// Account id of the current session (to tell whether its first sync was confirmed).
    account_user: Mutex<Option<String>>,
    /// Addon names of the account's lists (manifest url → name), so a list sent back never blanks
    /// the name of an addon whose manifest was not loaded on this computer yet.
    addon_names: Mutex<BTreeMap<String, String>>,
}

fn store(dir: &PathBuf) -> AppStore {
    AppStore::open(dir).expect("data directory")
}

impl Engine {
    pub fn new(dir: PathBuf, config: Option<ServerConfig>, legacy_owner: Option<String>) -> Self {
        let st = store(&dir);
        let session = std::fs::read(dir.join(SESSION_FILE)).ok().and_then(|c| crypto::unprotect(&c).ok()).and_then(|p| serde_json::from_slice::<Session>(&p).ok());
        let client_id = match st.read::<String>("sync-client").ok().flatten().filter(|c| (16..=96).contains(&c.len())) {
            Some(id) => id,
            None => {
                // Same shape as the original's `SyncClientIdentity` (`nuvio-mobile-` + 32 chars).
                let id = format!("nuvio-astra-{}", crypto::random_token(32));
                let _ = st.write("sync-client", &id);
                id
            }
        };
        let configured = config.is_some();
        let status = Status {
            configured,
            signed_in: session.is_some() && configured,
            email: session.as_ref().and_then(|s| s.email.clone()),
            state: if !configured { "notConfigured" } else if session.is_some() { "idle" } else { "signedOut" }.into(),
            pending: 0,
            last_sync_epoch_ms: st.sync_state().map(|s| s.last_sync_epoch_ms).unwrap_or(0),
            last_error: None,
            last_report: None,
            link_question: None,
        };
        let account_user = session.as_ref().filter(|_| configured).map(|s| s.user_id.clone());
        Self {
            dir,
            remote: config.map(Remote::new),
            session: tokio::sync::Mutex::new(session.filter(|_| configured)),
            run: tokio::sync::Mutex::new(()),
            status: Mutex::new(status),
            generation: AtomicU64::new(0),
            legacy_owner,
            client_id,
            received: AtomicBool::new(false),
            account_user: Mutex::new(account_user),
            addon_names: Mutex::new(BTreeMap::new()),
        }
    }

    fn st(&self) -> AppStore {
        store(&self.dir)
    }

    pub fn status(&self) -> Status {
        let mut s = self.status.lock().expect("status").clone();
        let state = self.st().sync_state().unwrap_or_default();
        let preferences = state.linked_user_id.as_deref().map_or(0, |account| self.preference_changes(&self.st(), account).len());
        s.pending = state.pending_count() + preferences;
        s.last_sync_epoch_ms = state.last_sync_epoch_ms;
        let user = self.account_user.lock().expect("account user").clone();
        let confirmed = user.is_some() && state.linked_user_id == user;
        if s.signed_in && !confirmed && matches!(s.state.as_str(), "idle" | "pending" | "needsReview") {
            s.state = "needsReview".into();
            s.link_question = self.other_owner(&state, user.as_deref()).map(|_| OTHER_ACCOUNT.to_string());
        } else if s.signed_in && s.state == "idle" && s.pending > 0 {
            s.state = "pending".into();
        }
        s
    }

    /// Account the local data came from when it is not `user` (previous link, or the original
    /// app's profiles), if any.
    fn other_owner(&self, state: &nuvio_store::sync_state::SyncState, user: Option<&str>) -> Option<String> {
        let user = user?;
        match &state.linked_user_id {
            Some(previous) if previous != user => Some(previous.clone()),
            Some(_) => None,
            None => self.legacy_owner.clone().filter(|owner| owner != user),
        }
    }

    /// Sanitized history of previews and runs (`sync-log.json`): dates, counts, entity names and
    /// error texts only — no token, e-mail, title or address.
    fn log(&self, kind: &str, mut entry: Value) {
        let st = self.st();
        let mut log: Vec<Value> = st.read("sync-log").ok().flatten().unwrap_or_default();
        if !entry.is_object() {
            entry = json!({});
        }
        entry["kind"] = json!(kind);
        entry["atEpochMs"] = json!(nuvio_store::library::now_ms());
        log.push(entry);
        let excess = log.len().saturating_sub(50);
        log.drain(..excess);
        let _ = st.write("sync-log", &log);
    }

    fn set_status(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.status.lock().expect("status"));
    }

    fn remote(&self) -> Result<&Remote, AccountError> {
        self.remote.as_ref().ok_or(AccountError::NotConfigured)
    }

    fn save_session(&self, session: Option<&Session>) {
        let path = self.dir.join(SESSION_FILE);
        match session {
            Some(s) => {
                if let Ok(cipher) = crypto::protect(&serde_json::to_vec(s).expect("session json")) {
                    let tmp = path.with_extension("tmp");
                    if std::fs::write(&tmp, cipher).is_ok() {
                        let _ = std::fs::rename(&tmp, &path);
                    }
                }
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    // ---------------------------------------------------------------- session

    pub async fn sign_in(&self, email: &str, password: &str) -> Result<Status, AccountError> {
        let session = self.remote()?.sign_in(email.trim(), password).await?;
        self.start_session(session).await;
        Ok(self.status())
    }

    /// Creates the account (`signUpWith(Email)`); `Ok(false)` when an e-mail confirmation is needed.
    pub async fn sign_up(&self, email: &str, password: &str) -> Result<bool, AccountError> {
        match self.remote()?.sign_up(email.trim(), password).await? {
            SignUp::Session(s) => {
                self.start_session(s).await;
                Ok(true)
            }
            SignUp::ConfirmEmail => Ok(false),
        }
    }

    async fn start_session(&self, session: Session) {
        self.received.store(false, Ordering::SeqCst);
        self.save_session(Some(&session));
        self.set_status(|s| {
            s.signed_in = true;
            s.email = session.email.clone();
            s.state = "idle".into();
            s.last_error = None;
            s.link_question = None;
        });
        *self.account_user.lock().expect("account user") = Some(session.user_id.clone());
        *self.session.lock().await = Some(session);
    }

    /// Signs out this computer: the running sync is cancelled, the session file removed. Local data
    /// and changes still waiting to be sent are kept (sent after the next sign-in to this account).
    pub async fn sign_out(&self) -> Status {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let session = self.session.lock().await.take();
        *self.account_user.lock().expect("account user") = None;
        self.save_session(None);
        if let (Some(remote), Some(s)) = (self.remote.as_ref(), session) {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), remote.sign_out(&s.access_token)).await;
        }
        self.set_status(|s| {
            s.signed_in = false;
            s.email = None;
            s.state = "signedOut".into();
            s.last_error = None;
            s.link_question = None;
        });
        self.status()
    }

    /// A valid access token (refreshed when about to expire). A refused refresh ends the session.
    async fn token(&self, force_refresh: bool) -> Result<(String, String), AccountError> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.clone() else { return Err(AccountError::SignedOut) };
        if !force_refresh && !session.expires_soon(now_s()) {
            return Ok((session.access_token, session.user_id));
        }
        match self.remote()?.refresh(&session.refresh_token).await {
            Ok(fresh) => {
                self.save_session(Some(&fresh));
                *guard = Some(fresh.clone());
                Ok((fresh.access_token, fresh.user_id))
            }
            Err(RemoteError::Unauthorized) => {
                *guard = None;
                self.save_session(None);
                self.set_status(|s| {
                    s.signed_in = false;
                    s.state = "expired".into();
                    s.last_error = Some("Sua sessão expirou. Entre novamente; nada do que está neste computador foi apagado.".into());
                });
                Err(AccountError::SignedOut)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// RPC with the user's token; an expired token is refreshed once and the call retried.
    async fn call(&self, function: &str, params: Value) -> Result<Value, AccountError> {
        let (token, _) = self.token(false).await?;
        match self.remote()?.rpc(function, &token, &params).await {
            Err(RemoteError::Unauthorized) => {
                let (token, _) = self.token(true).await?;
                Ok(self.remote()?.rpc(function, &token, &params).await?)
            }
            other => Ok(other?),
        }
    }

    async fn addon_rows(&self, profile: i64) -> Result<Value, AccountError> {
        let (token, _) = self.token(false).await?;
        match self.remote()?.addon_rows(&token, profile).await {
            Err(RemoteError::Unauthorized) => {
                let (token, _) = self.token(true).await?;
                Ok(self.remote()?.addon_rows(&token, profile).await?)
            }
            other => Ok(other?),
        }
    }

    fn with_origin(&self, mut params: Value) -> Value {
        params["p_origin_client_id"] = json!(self.client_id);
        params
    }

    // ---------------------------------------------------------------- sync

    /// One sync run for the active profile. `confirm_link`: the user reviewed the preview of the first
    /// sync with this account and started it. Until then nothing is written, locally or remotely.
    pub async fn sync(&self, active_profile: i64, confirm_link: bool) -> Result<Report, AccountError> {
        let Ok(_run) = self.run.try_lock() else { return Err(AccountError::Busy) };
        if self.remote.is_none() {
            return Err(AccountError::NotConfigured);
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let (_, user_id) = self.token(false).await?;
        let st = self.st();

        // Link of this computer's data with the account: only after the user reviewed the preview.
        let state = st.sync_state()?;
        let first_link = state.linked_user_id.as_deref() != Some(user_id.as_str());
        if first_link {
            if !confirm_link {
                let other = self.other_owner(&state, Some(&user_id));
                self.set_status(|s| {
                    s.state = "needsReview".into();
                    s.link_question = other.map(|_| OTHER_ACCOUNT.to_string());
                });
                return Err(AccountError::NeedsReview);
            }
            // Verified copy of every local document before the first write.
            let backup = st.backup_documents(&format!("before-first-sync-{}", nuvio_store::library::now_ms()))?;
            self.log("backup", json!({ "folder": backup.file_name().map(|n| n.to_string_lossy().into_owned()) }));
            // Deletions made before this link are local only: they never delete account data.
            st.update_sync_state(|s| {
                for p in s.profiles.values_mut() {
                    p.library_deletes.clear();
                    p.progress_deletes.clear();
                    p.watched_deletes.clear();
                }
                s.profile_deletes.clear();
            })?;
            if state.linked_user_id.is_some() {
                st.reset_account_link(&user_id)?;
            } else {
                st.set_linked_user(Some(user_id.clone()))?;
            }
        }

        self.set_status(|s| {
            s.state = "syncing".into();
            s.link_question = None;
        });
        let mut report = Report { profile: active_profile, ..Default::default() };
        let everything = first_link || !self.received.load(Ordering::SeqCst);
        let result = self.run_steps(&st, active_profile, generation, &mut report, everything).await;
        let cancelled = self.generation.load(Ordering::SeqCst) != generation;
        report.finished_at_epoch_ms = nuvio_store::library::now_ms();
        match result {
            Ok(()) if !cancelled => {
                if report.failed.is_empty() {
                    self.received.store(true, Ordering::SeqCst);
                    let pulled: Vec<i64> = if everything { st.profiles()?.profiles.iter().map(|p| p.index).collect() } else { vec![active_profile] };
                    st.update_sync_state(|s| {
                        s.last_sync_epoch_ms = report.finished_at_epoch_ms;
                        for p in &pulled {
                            s.profiles.entry(*p).or_default().last_sync_epoch_ms = report.finished_at_epoch_ms;
                        }
                    })?;
                }
                self.log("sync", json!({ "firstLink": first_link, "profile": report.profile, "sent": report.sent, "received": report.received, "failed": report.failed, "changed": report.changed, "pendingAfter": st.sync_state()?.pending_count() }));
                let failed = report.failed.clone();
                self.set_status(|s| {
                    s.state = if failed.is_empty() { "idle" } else { "error" }.into();
                    s.last_error = (!failed.is_empty()).then(|| format!("Não sincronizado: {}", failed.join("; ")));
                    s.last_report = Some(report.clone());
                });
                Ok(report)
            }
            Ok(()) => Ok(report),
            Err(e) => {
                let offline = matches!(e, AccountError::Remote(RemoteError::Offline));
                self.log("sync", json!({ "firstLink": first_link, "profile": report.profile, "error": if offline { "offline".to_string() } else { e.to_string() } }));
                self.set_status(|s| {
                    if s.signed_in {
                        s.state = if offline { "offline" } else { "error" }.into();
                    }
                    s.last_error = Some(if offline { "Sem conexão. As alterações ficam guardadas e serão enviadas quando a conexão voltar.".into() } else { e.to_string() });
                });
                Err(e)
            }
        }
    }

    /// The account's data is received and merged first; only then what waits here is sent.
    /// `everything`: every profile (first run since the app opened, or first link).
    async fn run_steps(&self, st: &AppStore, active: i64, generation: u64, report: &mut Report, everything: bool) -> Result<(), AccountError> {
        let alive = || self.generation.load(Ordering::SeqCst) == generation;

        // 1. Profiles: local deletions, the account's list (a local change waiting survives it),
        // then that change. Before the first link the whole-list push waits for the join (it would
        // replace the account's list).
        let state = st.sync_state()?;
        for (&index, &rev) in &state.profile_deletes {
            self.call("sync_delete_profile_data", self.with_origin(json!({ "p_profile_id": index }))).await?;
            st.forget_sync_bases(index)?;
            st.update_sync_state(|s| {
                if s.profile_deletes.get(&index) == Some(&rev) {
                    s.profile_deletes.remove(&index);
                }
            })?;
            report.sent += 1;
        }
        if let Some(rows) = self.step(report, "perfis", self.call("sync_pull_profiles", json!({}))).await? {
            if !alive() {
                return Ok(());
            }
            let remote: Vec<Profile> = rows.as_array().map(|a| a.iter().filter_map(profile_of).collect()).unwrap_or_default();
            let stats = st.apply_profiles_remote(remote)?;
            note(report, "profiles", &stats);
            self.sync_pin_cache(st)?;
            let after = st.sync_state()?;
            if after.profiles_dirty > 0 {
                let local = st.profiles()?;
                let payload: Vec<Value> = local.profiles.iter().map(profile_push).collect();
                if self.step(report, "perfis", self.call("sync_push_profiles", self.with_origin(json!({ "p_client_max_profiles": MAX_PROFILES, "p_profiles": payload })))).await?.is_some() {
                    let rev = after.profiles_dirty;
                    st.update_sync_state(|s| {
                        if s.profiles_dirty == rev {
                            s.profiles_dirty = 0;
                        }
                    })?;
                    report.sent += local.profiles.len();
                }
            }
        }

        // 2. The account's data, then what waits here: every profile when the app opens (and on the
        // first link, so the whole computer is joined at once), afterwards the active one.
        let profiles: Vec<i64> = st.profiles()?.profiles.iter().map(|p| p.index).collect();
        let active = st.profiles()?.get(active).map_or_else(|| profiles.first().copied().unwrap_or(1), |p| p.index);
        // The profile on screen first.
        let mut pulled = vec![active];
        if everything {
            pulled.extend(profiles.iter().copied().filter(|&p| p != active));
        }
        let account = st.sync_state()?.linked_user_id.unwrap_or_default();
        let mut addon_profiles = BTreeSet::new();
        for &profile in &pulled {
            if !alive() {
                return Ok(());
            }
            let addon_profile = st.profiles()?.addon_profile(profile);
            if addon_profiles.insert(addon_profile) {
                if let Some(rows) = self.step(report, "addons", self.addon_rows(addon_profile)).await? {
                    let remote = self.remember_addon_rows(&rows);
                    if alive() {
                        let stats = st.apply_addons_remote(addon_profile, remote.into_iter().map(|(url, enabled, _)| (url, enabled)).collect())?;
                        note(report, "addons", &stats);
                    }
                }
            }
            self.pull_library(st, profile, report, &alive).await?;
            self.pull_progress(st, profile, report, &alive).await?;
            self.pull_watched(st, profile, report, &alive).await?;
            self.pull_collections(st, profile, &account, report, &alive).await?;
            self.sync_home(st, profile, &account, report, &alive).await?;
            self.sync_player_settings(st, profile, &account, report, &alive).await?;
            // What changed here, and what the merge above queued.
            self.push_profile(st, profile, report).await?;
        }

        // 3. What changed in the other profiles (preferences merge with the account's first).
        for profile in self.preference_changes(st, &account).into_iter().filter(|p| !pulled.contains(p)) {
            if !alive() {
                return Ok(());
            }
            self.sync_home(st, profile, &account, report, &alive).await?;
            self.sync_player_settings(st, profile, &account, report, &alive).await?;
        }
        for &profile in profiles.iter().filter(|p| !pulled.contains(p)) {
            if !alive() {
                return Ok(());
            }
            self.push_profile(st, profile, report).await?;
        }
        Ok(())
    }

    /// Runs one step: an account error is reported and the run goes on (`runStep`); a lost
    /// connection or session stops the run.
    async fn step<T>(&self, report: &mut Report, entity: &str, fut: impl std::future::Future<Output = Result<T, AccountError>>) -> Result<Option<T>, AccountError> {
        match fut.await {
            Ok(v) => Ok(Some(v)),
            Err(e @ AccountError::Remote(RemoteError::Offline)) | Err(e @ AccountError::SignedOut) => Err(e),
            Err(e) => {
                let label = format!("{entity} ({e})");
                if !report.failed.contains(&label) {
                    report.failed.push(label);
                }
                Ok(None)
            }
        }
    }

    async fn push_profile(&self, st: &AppStore, profile: i64, report: &mut Report) -> Result<(), AccountError> {
        let (mut sent, data) = st.pending_for(profile)?;
        if data.is_empty() {
            return Ok(());
        }
        let mut ok = true;
        for batch in data.library_upserts.chunks(BATCH) {
            let items: Vec<Value> = batch.iter().map(library_push).collect();
            ok &= self.step(report, "biblioteca", self.call("sync_push_library_items", self.with_origin(json!({ "p_profile_id": profile, "p_items": items })))).await?.is_some();
        }
        for batch in data.library_deletes.chunks(BATCH) {
            let keys: Vec<Value> = batch.iter().map(|(t, id)| json!({ "content_id": id, "content_type": t })).collect();
            ok &= self.step(report, "biblioteca", self.call("sync_delete_library_items", self.with_origin(json!({ "p_profile_id": profile, "p_keys": keys })))).await?.is_some();
        }
        for batch in data.progress_upserts.chunks(BATCH) {
            let entries: Vec<Value> = batch.iter().map(progress_push).collect();
            ok &= self.step(report, "progresso", self.call("sync_push_watch_progress", self.with_origin(json!({ "p_profile_id": profile, "p_entries": entries })))).await?.is_some();
        }
        if !data.progress_deletes.is_empty() {
            ok &= self.step(report, "progresso", self.call("sync_delete_watch_progress", self.with_origin(json!({ "p_profile_id": profile, "p_keys": data.progress_deletes })))).await?.is_some();
        }
        for batch in data.watched_upserts.chunks(BATCH) {
            let items: Vec<Value> = batch.iter().map(watched_push).collect();
            ok &= self.step(report, "assistidos", self.call("sync_push_watched_items", self.with_origin(json!({ "p_profile_id": profile, "p_items": items })))).await?.is_some();
        }
        if !data.watched_deletes.is_empty() {
            let keys: Vec<Value> = data.watched_deletes.iter().map(|r| json!({ "content_id": r.content_id, "season": r.season, "episode": r.episode })).collect();
            ok &= self.step(report, "assistidos", self.call("sync_delete_watched_items", self.with_origin(json!({ "p_profile_id": profile, "p_keys": keys })))).await?.is_some();
        }
        let read_only = st.profiles()?.addons_read_only(profile);
        let addons_linked = st.sync_state()?.profiles.get(&profile).is_some_and(|p| p.addons_linked);
        // Before the first link the whole-list push waits for the join with the account's list.
        let push_addons = data.addons.is_some() && !read_only && addons_linked;
        if !push_addons {
            sent.addons_dirty = 0; // stays pending
        }
        if let Some(addons) = data.addons.as_ref().filter(|_| push_addons) {
            let rows: Vec<Value> = addons
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let known = self.addon_names.lock().expect("addon names").get(&a.manifest_url).cloned();
                    let name = a.summary.as_ref().map(|s| s.name.clone()).filter(|n| !n.trim().is_empty()).or(known).unwrap_or_default();
                    json!({ "url": a.manifest_url, "name": name, "enabled": a.enabled, "sort_order": i })
                })
                .collect();
            ok &= self.step(report, "addons", self.call("sync_push_addons", self.with_origin(json!({ "p_profile_id": profile, "p_addons": rows })))).await?.is_some();
        }
        if ok {
            st.clear_sent(profile, &sent)?;
            report.sent += data.library_upserts.len() + data.library_deletes.len() + data.progress_upserts.len() + data.progress_deletes.len() + data.watched_upserts.len() + data.watched_deletes.len() + usize::from(push_addons);
        }
        Ok(())
    }

    /// `CollectionSyncService`: pull, and push the whole list when it was edited here (the
    /// original sends the complete JSON with `sync_push_collections`). Edited here = different from
    /// what this computer and the account last agreed on; an edit here wins over a remote one made
    /// meanwhile, as in the original (its push replaces the list). No row = the account has no
    /// collections for the profile.
    async fn pull_collections(&self, st: &AppStore, profile: i64, account: &str, report: &mut Report, alive: &(dyn Fn() -> bool + Sync)) -> Result<(), AccountError> {
        let Some(rows) = self.step(report, "coleções", self.call("sync_pull_collections", json!({ "p_profile_id": profile }))).await? else { return Ok(()) };
        if !alive() {
            return Ok(());
        }
        let remote = match rows.as_array().and_then(|a| a.first()).and_then(|row| row.get("collections_json")) {
            Some(v) if v.is_array() => Some(v.clone()),
            // A JSON string (older rows) or null.
            Some(Value::String(text)) => Some(serde_json::from_str::<Value>(text).ok().filter(Value::is_array).unwrap_or_else(|| json!([]))),
            Some(_) => Some(json!([])),
            None => None,
        };
        let local = st.collections(profile)?;
        let base: Option<Value> = st.sync_base("collections", profile, account)?;
        // Also collections made here for an account that has none yet.
        let first_here = remote.is_none() && local.as_ref().and_then(Value::as_array).is_some_and(|a| !a.is_empty());
        let edited_here = first_here || matches!((&base, &local), (Some(b), Some(l)) if b != l);
        if edited_here {
            let Some(local) = local else { return Ok(()) };
            let params = self.with_origin(json!({ "p_profile_id": profile, "p_collections_json": local }));
            if self.step(report, "coleções", self.call("sync_push_collections", params)).await?.is_some() {
                st.set_sync_base("collections", profile, account, &local)?;
                report.sent += 1;
            }
            return Ok(());
        }
        let Some(remote) = remote else { return Ok(()) };
        if local.as_ref() != Some(&remote) {
            st.set_collections(profile, &remote)?;
            report.received += remote.as_array().map_or(0, Vec::len);
            mark_changed(report, "collections");
        }
        st.set_sync_base("collections", profile, account, &remote)?;
        Ok(())
    }

    /// Profiles whose preferences changed here since the account last agreed on them.
    fn preference_changes(&self, st: &AppStore, account: &str) -> Vec<i64> {
        let Ok(profiles) = st.profiles() else { return Vec::new() };
        let changed = |profile: i64| -> Option<bool> {
            let now = st.profile_settings(profile).ok()?;
            let home: Option<HomeCatalogSettings> = st.sync_base("home", profile, account).ok()?;
            let player: Option<PlayerSettings> = st.sync_base("player", profile, account).ok()?;
            Some(home.is_some_and(|b| crate::home::differs(&now.home, &b)) || player.is_some_and(|b| crate::preferences::differs(&now.player, &b)))
        };
        profiles.profiles.iter().map(|p| p.index).filter(|&p| changed(p) == Some(true)).collect()
    }

    /// `HomeCatalogSettingsSyncService` (pull, merge, push): see `home.rs`.
    async fn sync_home(&self, st: &AppStore, profile: i64, account: &str, report: &mut Report, alive: &(dyn Fn() -> bool + Sync)) -> Result<(), AccountError> {
        let params = json!({ "p_profile_id": profile, "p_platform": crate::home::PLATFORM });
        let Some(rows) = self.step(report, "ordem do Início", self.call("sync_pull_home_catalog_settings", params)).await? else { return Ok(()) };
        if !alive() {
            return Ok(());
        }
        let remote = blob_of(&rows);
        let base: Option<HomeCatalogSettings> = st.sync_base("home", profile, account)?;
        let nothing = json!({});
        let theirs = remote.unwrap_or(&nothing);
        if base.is_none() && remote.is_some() {
            st.keep_settings_before_sync()?;
        }
        let mut changed = false;
        let merged = st
            .update_profile_settings(profile, |settings| {
                let merged = crate::home::merge(&settings.home, base.as_ref(), theirs);
                changed = merged != settings.home;
                settings.home = merged;
            })?
            .home;
        if changed {
            report.received += 1;
            mark_changed(report, "home");
        }
        // What the account has now; what differs from it is this computer's to send.
        let agreed = match &base {
            Some(base) => crate::home::merge(base, Some(base), theirs),
            None => merged.clone(),
        };
        st.set_sync_base("home", profile, account, &agreed)?;
        if !crate::home::differs(&merged, &agreed) || !alive() {
            return Ok(());
        }
        let params = self.with_origin(json!({ "p_profile_id": profile, "p_platform": crate::home::PLATFORM, "p_settings_json": crate::home::payload(remote, &merged) }));
        if self.step(report, "ordem do Início", self.call("sync_push_home_catalog_settings", params)).await?.is_some() {
            st.set_sync_base("home", profile, account, &merged)?;
            report.sent += 1;
        }
        Ok(())
    }

    /// `ProfileSettingsSync` (pull, merge, push of the changed fields only): see `preferences.rs`.
    async fn sync_player_settings(&self, st: &AppStore, profile: i64, account: &str, report: &mut Report, alive: &(dyn Fn() -> bool + Sync)) -> Result<(), AccountError> {
        let params = json!({ "p_profile_id": profile, "p_platform": crate::preferences::PLATFORM });
        let Some(rows) = self.step(report, "configurações", self.call("sync_pull_profile_settings_blob", params)).await? else { return Ok(()) };
        if !alive() {
            return Ok(());
        }
        let base: Option<PlayerSettings> = st.sync_base("player", profile, account)?;
        let Some(remote) = blob_of(&rows) else {
            // Never creates a partial blob; with nothing there, nothing waits to be sent.
            if base.is_some() {
                st.set_sync_base("player", profile, "", &Value::Null)?;
            }
            return Ok(());
        };
        if base.is_none() {
            st.keep_settings_before_sync()?;
        }
        let mut changed = false;
        let merged = st
            .update_profile_settings(profile, |settings| {
                let merged = crate::preferences::merge(&settings.player, base.as_ref(), remote);
                changed = merged != settings.player;
                settings.player = merged;
            })?
            .player;
        if changed {
            report.received += 1;
            mark_changed(report, "settings");
        }
        let agreed = crate::preferences::from_blob(remote);
        st.set_sync_base("player", profile, account, &agreed)?;
        if !crate::preferences::differs(&merged, &agreed) || !alive() {
            return Ok(());
        }
        let Some(patched) = crate::preferences::payload(remote, &merged) else { return Ok(()) };
        // What the account will hold, not `merged`: a font of this computer is never part of it.
        let sent = crate::preferences::from_blob(&patched);
        let params = self.with_origin(json!({ "p_profile_id": profile, "p_platform": crate::preferences::PLATFORM, "p_settings_json": patched }));
        if self.step(report, "configurações", self.call("sync_push_profile_settings_blob", params)).await?.is_some() {
            st.set_sync_base("player", profile, account, &sent)?;
            report.sent += 1;
        }
        Ok(())
    }

    /// Sends what waits in the queue of `profile` without reading anything back: for the position
    /// saved while playing (`pushScrobbleToServer`), every minute and on pause / seek / close.
    pub async fn push_pending(&self, profile: i64) -> Result<Report, AccountError> {
        let Ok(_run) = self.run.try_lock() else { return Err(AccountError::Busy) };
        if self.remote.is_none() {
            return Err(AccountError::NotConfigured);
        }
        let (_, user) = self.token(false).await?;
        let st = self.st();
        if st.sync_state()?.linked_user_id.as_deref() != Some(user.as_str()) {
            return Err(AccountError::NeedsReview);
        }
        if !self.received.load(Ordering::SeqCst) {
            return Err(AccountError::Busy); // a full run receives first, then sends this too
        }
        let mut report = Report { profile, ..Default::default() };
        self.push_profile(&st, profile, &mut report).await?;
        Ok(report)
    }

    async fn pull_library(&self, st: &AppStore, profile: i64, report: &mut Report, alive: &(dyn Fn() -> bool + Sync)) -> Result<(), AccountError> {
        let p = json!({ "p_profile_id": profile });
        let state = st.sync_state()?.profiles.get(&profile).cloned().unwrap_or_default();
        if !state.library.initialized {
            let Some(cursor) = self.step(report, "biblioteca", self.call("sync_get_library_delta_cursor", p.clone())).await? else { return Ok(()) };
            let mut items = Vec::new();
            let mut offset = 0;
            loop {
                let Some(page) = self.step(report, "biblioteca", self.call("sync_pull_library", json!({ "p_profile_id": profile, "p_limit": PAGE, "p_offset": offset }))).await? else { return Ok(()) };
                let rows = page.as_array().cloned().unwrap_or_default();
                items.extend(rows.iter().filter_map(library_of));
                if (rows.len() as i64) < PAGE {
                    break;
                }
                offset += PAGE;
            }
            if !alive() {
                return Ok(());
            }
            let stats = st.apply_library_snapshot(profile, items, cursor.as_i64().unwrap_or(0))?;
            note(report, "library", &stats);
        }
        loop {
            let since = st.sync_state()?.profiles.get(&profile).map(|p| p.library.event_id).unwrap_or(0);
            let Some(page) = self.step(report, "biblioteca", self.call("sync_pull_library_delta", json!({ "p_profile_id": profile, "p_since_event_id": since, "p_limit": PAGE }))).await? else { return Ok(()) };
            let rows = page.as_array().cloned().unwrap_or_default();
            if rows.is_empty() || !alive() {
                return Ok(());
            }
            let events: Vec<RemoteEvent<LibraryItem>> = rows.iter().filter_map(|r| Some(RemoteEvent { event_id: r.get("event_id")?.as_i64()?, operation: r.get("operation")?.as_str()?.to_string(), item: library_of(r)? })).collect();
            let stats = st.apply_library_delta(profile, events)?;
            note(report, "library", &stats);
            if (rows.len() as i64) < PAGE {
                return Ok(());
            }
        }
    }

    async fn pull_progress(&self, st: &AppStore, profile: i64, report: &mut Report, alive: &(dyn Fn() -> bool + Sync)) -> Result<(), AccountError> {
        let state = st.sync_state()?.profiles.get(&profile).cloned().unwrap_or_default();
        if !state.progress.initialized {
            let cursor = self.step(report, "progresso", self.call("sync_get_watch_progress_delta_cursor", json!({ "p_profile_id": profile }))).await?;
            let Some(rows) = self.step(report, "progresso", self.call("sync_pull_watch_progress", json!({ "p_profile_id": profile }))).await? else { return Ok(()) };
            if !alive() {
                return Ok(());
            }
            let remote: Vec<RemoteProgress> = rows.as_array().map(|a| a.iter().filter_map(progress_of).collect()).unwrap_or_default();
            let stats = st.apply_progress_snapshot(profile, remote, cursor.and_then(|c| c.as_i64()).unwrap_or(0))?;
            note(report, "progress", &stats);
        }
        loop {
            let since = st.sync_state()?.profiles.get(&profile).map(|p| p.progress.event_id).unwrap_or(0);
            let Some(page) = self.step(report, "progresso", self.call("sync_pull_watch_progress_delta", json!({ "p_profile_id": profile, "p_since_event_id": since, "p_limit": PAGE }))).await? else { return Ok(()) };
            let rows = page.as_array().cloned().unwrap_or_default();
            if rows.is_empty() || !alive() {
                return Ok(());
            }
            let events: Vec<RemoteEvent<RemoteProgress>> = rows.iter().filter_map(|r| Some(RemoteEvent { event_id: r.get("event_id")?.as_i64()?, operation: r.get("operation")?.as_str()?.to_string(), item: progress_of(r)? })).collect();
            let stats = st.apply_progress_delta(profile, events)?;
            note(report, "progress", &stats);
            if (rows.len() as i64) < PAGE {
                return Ok(());
            }
        }
    }

    async fn pull_watched(&self, st: &AppStore, profile: i64, report: &mut Report, alive: &(dyn Fn() -> bool + Sync)) -> Result<(), AccountError> {
        let state = st.sync_state()?.profiles.get(&profile).cloned().unwrap_or_default();
        if !state.watched.initialized {
            let Some(cursor) = self.step(report, "assistidos", self.call("sync_get_watched_items_delta_cursor", json!({ "p_profile_id": profile }))).await? else { return Ok(()) };
            let mut items = Vec::new();
            let mut page_no = 1;
            loop {
                let Some(page) = self.step(report, "assistidos", self.call("sync_pull_watched_items", json!({ "p_profile_id": profile, "p_page": page_no, "p_page_size": PAGE }))).await? else { return Ok(()) };
                let rows = page.as_array().cloned().unwrap_or_default();
                items.extend(rows.iter().filter_map(watched_of));
                if (rows.len() as i64) < PAGE {
                    break;
                }
                page_no += 1;
            }
            if !alive() {
                return Ok(());
            }
            let stats = st.apply_watched_snapshot(profile, items, cursor.as_i64().unwrap_or(0))?;
            note(report, "watched", &stats);
        }
        loop {
            let since = st.sync_state()?.profiles.get(&profile).map(|p| p.watched.event_id).unwrap_or(0);
            let Some(page) = self.step(report, "assistidos", self.call("sync_pull_watched_items_delta", json!({ "p_profile_id": profile, "p_since_event_id": since, "p_limit": PAGE }))).await? else { return Ok(()) };
            let rows = page.as_array().cloned().unwrap_or_default();
            if rows.is_empty() || !alive() {
                return Ok(());
            }
            let events: Vec<RemoteEvent<WatchedItem>> = rows.iter().filter_map(|r| Some(RemoteEvent { event_id: r.get("event_id")?.as_i64()?, operation: r.get("operation")?.as_str()?.to_string(), item: watched_of(r)? })).collect();
            let stats = st.apply_watched_delta(profile, events)?;
            note(report, "watched", &stats);
            if (rows.len() as i64) < PAGE {
                return Ok(());
            }
        }
    }

    /// `(manifest url, enabled, name)` of the account's addon rows; names remembered for later pushes.
    fn remember_addon_rows(&self, rows: &Value) -> Vec<(String, bool, String)> {
        let list: Vec<(String, bool, String)> = rows
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| {
                        let url = nuvio_core::url::ensure_manifest_suffix(r.get("url")?.as_str()?);
                        Some((url, r.get("enabled").and_then(Value::as_bool).unwrap_or(true), s(r, "name").unwrap_or_default()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut names = self.addon_names.lock().expect("addon names");
        for (url, _, name) in &list {
            if !name.trim().is_empty() {
                names.insert(url.clone(), name.clone());
            }
        }
        list
    }

    // ---------------------------------------------------------------- preview (read-only)

    /// Compares this computer's data with the account's, profile by profile, with the account's
    /// read functions only (`sync_pull_*`, the `addons` table). Nothing is written to the account
    /// nor to the local data (only the sanitized `sync-log`). Used before the first sync (what will
    /// be sent, received, and which versions differ) and afterwards as a check.
    pub async fn preview(&self) -> Result<Preview, AccountError> {
        let Ok(_run) = self.run.try_lock() else { return Err(AccountError::Busy) };
        let (_, user_id) = self.token(false).await?;
        let st = self.st();
        let state = st.sync_state()?;
        let local_profiles = st.profiles()?;
        let rows = self.call("sync_pull_profiles", json!({})).await?;
        let remote_profiles: Vec<Profile> = rows.as_array().map(|a| a.iter().filter_map(profile_of).collect()).unwrap_or_default();
        let local_idx: BTreeSet<i64> = local_profiles.profiles.iter().map(|p| p.index).collect();
        let remote_idx: BTreeSet<i64> = remote_profiles.iter().map(|p| p.index).collect();
        let mut preview = Preview {
            generated_at_epoch_ms: nuvio_store::library::now_ms(),
            first_sync: state.linked_user_id.as_deref() != Some(user_id.as_str()),
            other_account: self.other_owner(&state, Some(&user_id)).is_some(),
            profiles: Compare::of_keys(&local_idx, local_profiles.profiles.len(), &remote_idx, remote_profiles.len()),
            pending: state.pending_count(),
            ..Default::default()
        };
        for index in local_idx.union(&remote_idx).copied() {
            let local = local_profiles.get(index);
            let remote = remote_profiles.iter().find(|p| p.index == index);
            let mut e = ProfilePreview {
                index,
                local_name: local.map(|p| p.name.clone()),
                remote_name: remote.map(|p| p.name.clone()),
                profile_differs: matches!((local, remote), (Some(l), Some(r)) if l.name != r.name || !l.avatar_color_hex.eq_ignore_ascii_case(&r.avatar_color_hex) || l.avatar_id != r.avatar_id),
                shares_primary_addons: local.or(remote).is_some_and(|p| p.index != 1 && p.uses_primary_addons),
                pending: state.profiles.get(&index).map_or(0, |p| p.pending_count()),
                ..Default::default()
            };
            // Addons (own list; a profile using profile 1's addons has none of its own).
            if !e.shares_primary_addons {
                let local_addons = st.addons(index)?;
                let remote_addons = self.remember_addon_rows(&self.addon_rows(index).await?);
                let l: BTreeMap<String, bool> = local_addons.iter().map(|a| (a.manifest_url.clone(), a.enabled)).collect();
                let r: BTreeMap<String, bool> = remote_addons.iter().map(|(u, en, _)| (u.clone(), *en)).collect();
                e.addons = Compare::of_keys(&l.keys().cloned().collect(), local_addons.len(), &r.keys().cloned().collect(), remote_addons.len());
                e.addons.differing = l.iter().filter(|(u, en)| r.get(*u).is_some_and(|x| x != *en)).count();
                let mut seen = BTreeSet::new();
                let lo: Vec<&String> = local_addons.iter().map(|a| &a.manifest_url).filter(|u| r.contains_key(*u)).collect();
                let ro: Vec<&String> = remote_addons.iter().map(|(u, _, _)| u).filter(|u| l.contains_key(*u) && seen.insert((*u).clone())).collect();
                e.addon_order_differs = lo != ro;
            }
            // Saved items.
            let local_fav = st.favorites(index)?;
            let remote_fav = self.fetch_library(index).await?;
            e.favorites = Compare::of_keys(&local_fav.iter().map(|i| i.key()).collect(), local_fav.len(), &remote_fav.iter().map(|i| i.key()).collect(), remote_fav.len());
            // Watch progress: same key with another position or date, and which side is newer.
            let local_prog = st.progress(index)?;
            let remote_prog = self.fetch_progress(index).await?;
            let lp: BTreeMap<String, &nuvio_store::library::ProgressEntry> = local_prog.iter().map(|p| (p.key(), p)).collect();
            let rp: BTreeMap<String, &RemoteProgress> = remote_prog.iter().map(|r| (r.key(), r)).collect();
            e.progress = Compare::of_keys(&lp.keys().cloned().collect(), local_prog.len(), &rp.keys().cloned().collect(), remote_prog.len());
            for (key, l) in &lp {
                if let Some(r) = rp.get(key) {
                    let r = r.to_entry(Some(l), l.watched_percent);
                    if (l.last_position_ms, l.duration_ms, l.last_updated_epoch_ms) != (r.last_position_ms, r.duration_ms, r.last_updated_epoch_ms) {
                        e.progress.differing += 1;
                        if fresher(l, &r) {
                            e.progress.local_newer += 1;
                        }
                    }
                }
            }
            // Watched marks.
            let local_w = st.watched(index)?;
            let remote_w = self.fetch_watched(index).await?;
            e.watched = Compare::of_keys(&local_w.iter().map(|w| w.key()).collect(), local_w.len(), &remote_w.iter().map(|w| w.key()).collect(), remote_w.len());
            preview.entries.push(e);
        }
        for e in &preview.entries {
            for c in [&e.addons, &e.favorites, &e.progress, &e.watched] {
                preview.to_send += c.only_local + c.local_newer;
                preview.to_receive += c.only_remote + c.differing - c.local_newer;
            }
            preview.to_receive += usize::from(e.profile_differs);
            preview.conflicts += usize::from(e.profile_differs) + e.addons.differing + e.progress.differing;
        }
        preview.to_send += preview.profiles.only_local;
        preview.to_receive += preview.profiles.only_remote;
        self.log("preview", serde_json::to_value(preview.sanitized()).unwrap_or(Value::Null));
        Ok(preview)
    }

    async fn fetch_library(&self, profile: i64) -> Result<Vec<LibraryItem>, AccountError> {
        let mut items = Vec::new();
        let mut offset = 0;
        loop {
            let page = self.call("sync_pull_library", json!({ "p_profile_id": profile, "p_limit": PAGE, "p_offset": offset })).await?;
            let rows = page.as_array().cloned().unwrap_or_default();
            items.extend(rows.iter().filter_map(library_of));
            if (rows.len() as i64) < PAGE {
                return Ok(items);
            }
            offset += PAGE;
        }
    }

    async fn fetch_progress(&self, profile: i64) -> Result<Vec<RemoteProgress>, AccountError> {
        let rows = self.call("sync_pull_watch_progress", json!({ "p_profile_id": profile })).await?;
        Ok(rows.as_array().map(|a| a.iter().filter_map(progress_of).collect()).unwrap_or_default())
    }

    async fn fetch_watched(&self, profile: i64) -> Result<Vec<WatchedItem>, AccountError> {
        let mut items = Vec::new();
        let mut page_no = 1;
        loop {
            let page = self.call("sync_pull_watched_items", json!({ "p_profile_id": profile, "p_page": page_no, "p_page_size": PAGE })).await?;
            let rows = page.as_array().cloned().unwrap_or_default();
            items.extend(rows.iter().filter_map(watched_of));
            if (rows.len() as i64) < PAGE {
                return Ok(items);
            }
            page_no += 1;
        }
    }

    // ---------------------------------------------------------------- PIN

    fn pin_cache(&self) -> std::collections::BTreeMap<i64, PinCacheEntry> {
        self.st().read("pin-cache").ok().flatten().unwrap_or_default()
    }

    fn save_pin_cache(&self, cache: &std::collections::BTreeMap<i64, PinCacheEntry>) -> Result<(), AccountError> {
        Ok(self.st().write("pin-cache", cache)?)
    }

    /// `rememberVerifiedPin`: salted SHA-256 of `profile:<index>:<salt>:<pin>` (the PIN is never stored).
    fn remember_pin(&self, index: i64, pin: &str) -> Result<(), AccountError> {
        let updated_at = self.st().profiles()?.get(index).and_then(|p| p.updated_at.clone()).unwrap_or_default();
        let salt = crypto::random_token(32);
        let digest = crypto::sha256_hex(&format!("profile:{index}:{salt}:{pin}"));
        let mut cache = self.pin_cache();
        cache.insert(index, PinCacheEntry { salt, digest, profile_updated_at: updated_at });
        self.save_pin_cache(&cache)
    }

    /// `syncPinCache`: drops entries of profiles without PIN or whose PIN changed elsewhere.
    fn sync_pin_cache(&self, st: &AppStore) -> Result<(), AccountError> {
        let profiles = st.profiles()?;
        let mut cache = self.pin_cache();
        let before = cache.len();
        cache.retain(|index, entry| match profiles.get(*index) {
            Some(p) if p.pin_enabled => entry.profile_updated_at.is_empty() || p.updated_at.as_deref().is_none_or(|u| u.is_empty() || u == entry.profile_updated_at),
            _ => false,
        });
        if cache.len() != before {
            self.save_pin_cache(&cache)?;
        }
        Ok(())
    }

    /// Whether this computer's data was linked to an account.
    pub fn linked(&self) -> bool {
        self.st().sync_state().is_ok_and(|s| s.linked_user_id.is_some())
    }

    /// The session belongs to the account the profiles (and their PINs) came from. Another account's
    /// PIN must never open them.
    async fn session_owns_profiles(&self) -> bool {
        let Some(user) = self.session.lock().await.as_ref().map(|s| s.user_id.clone()) else { return false };
        let linked = self.st().sync_state().ok().and_then(|s| s.linked_user_id);
        linked.or_else(|| self.legacy_owner.clone()).is_none_or(|owner| owner == user)
    }

    /// Asks the account whether a profile still has a PIN and follows a removal, also while the
    /// profile is locked (nothing else is read or written). `None`: the account could not tell.
    pub async fn refresh_pin(&self, index: i64) -> Option<bool> {
        if !self.session_owns_profiles().await {
            return None;
        }
        let rows = self.call("sync_pull_profiles", json!({})).await.ok()?;
        let protected = rows.as_array()?.iter().filter_map(profile_of).any(|p| p.index == index && p.pin_enabled);
        if !protected {
            let st = self.st();
            st.set_profile_pin_state(index, false).ok()?;
            let _ = self.sync_pin_cache(&st);
        }
        Some(protected)
    }

    /// The offline PIN hash Nuvio keeps on this computer, for profiles copied from it and not
    /// linked yet (same algorithm).
    pub fn adopt_pin_cache(&self, index: i64, (salt, digest, profile_updated_at): (String, String, String)) {
        let mut cache = self.pin_cache();
        if let std::collections::btree_map::Entry::Vacant(slot) = cache.entry(index) {
            slot.insert(PinCacheEntry { salt, digest, profile_updated_at });
            let _ = self.save_pin_cache(&cache);
        }
    }

    /// `verifyPin`: the account checks it (and applies its lockout); offline, signed out or with
    /// another account, the cached hash of the last successful check is used.
    pub async fn verify_pin(&self, index: i64, pin: &str) -> PinResult {
        if self.session_owns_profiles().await {
            match self.call("verify_profile_pin", json!({ "p_profile_id": index, "p_pin": pin })).await {
                Ok(v) => {
                    let row = if v.is_array() { v.get(0).cloned().unwrap_or(Value::Null) } else { v };
                    let unlocked = row.get("unlocked").and_then(Value::as_bool).unwrap_or(false);
                    let retry = row.get("retry_after_seconds").and_then(Value::as_i64).unwrap_or(0);
                    if unlocked {
                        let _ = self.remember_pin(index, pin);
                        return PinResult { unlocked: true, retry_after_seconds: 0, message: None };
                    }
                    // Removed on another device: nothing to type any more.
                    if retry == 0 && self.refresh_pin(index).await == Some(false) {
                        return PinResult { unlocked: true, retry_after_seconds: 0, message: None };
                    }
                    let message = if retry > 0 { format!("Bloqueado. Tente novamente em {retry}s") } else { row.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| "PIN incorreto".into()) };
                    return PinResult { unlocked: false, retry_after_seconds: retry, message: Some(message) };
                }
                Err(AccountError::Remote(RemoteError::Offline)) | Err(AccountError::SignedOut) => {}
                Err(_) => {}
            }
        }
        self.verify_pin_locally(index, pin)
    }

    fn verify_pin_locally(&self, index: i64, pin: &str) -> PinResult {
        let profile = self.st().profiles().ok().and_then(|p| p.get(index).cloned());
        if !profile.as_ref().is_some_and(|p| p.pin_enabled) {
            return PinResult { unlocked: true, retry_after_seconds: 0, message: None };
        }
        let profile = profile.expect("checked");
        let Some(cached) = self.pin_cache().get(&index).cloned() else {
            return PinResult { unlocked: false, retry_after_seconds: 0, message: Some("Este computador ainda não consegue conferir esse PIN. Entre na conta Nuvio dona deste perfil, com internet, para continuar.".into()) };
        };
        if !cached.profile_updated_at.is_empty() && profile.updated_at.as_deref().is_some_and(|u| !u.is_empty() && u != cached.profile_updated_at) {
            let mut cache = self.pin_cache();
            cache.remove(&index);
            let _ = self.save_pin_cache(&cache);
            return PinResult { unlocked: false, retry_after_seconds: 0, message: Some("O PIN deste perfil mudou. Conecte-se uma vez para atualizar o bloqueio neste dispositivo.".into()) };
        }
        if crypto::sha256_hex(&format!("profile:{index}:{}:{pin}", cached.salt)) == cached.digest {
            PinResult { unlocked: true, retry_after_seconds: 0, message: None }
        } else {
            PinResult { unlocked: false, retry_after_seconds: 0, message: Some("PIN incorreto".into()) }
        }
    }

    /// `setPin`: needs the account (`set_profile_pin`); `current_pin` when changing an existing one.
    pub async fn set_pin(&self, index: i64, pin: &str, current_pin: Option<&str>) -> Result<(), AccountError> {
        if !is_pin(pin) {
            return Err(AccountError::Store("O PIN deve ter 4 dígitos.".into()));
        }
        let mut params = json!({ "p_profile_id": index, "p_pin": pin });
        if let Some(c) = current_pin {
            params["p_current_pin"] = json!(c);
        }
        self.call("set_profile_pin", params).await.map_err(require_online("Conecte-se à internet para definir o bloqueio por PIN."))?;
        self.st().set_profile_pin_state(index, true)?;
        self.remember_pin(index, pin)?;
        Ok(())
    }

    /// `clearPin` (`clear_profile_pin`).
    pub async fn clear_pin(&self, index: i64, current_pin: Option<&str>) -> Result<(), AccountError> {
        let mut params = json!({ "p_profile_id": index });
        if let Some(c) = current_pin {
            params["p_current_pin"] = json!(c);
        }
        self.call("clear_profile_pin", params).await.map_err(require_online("Conecte-se à internet para remover o bloqueio por PIN."))?;
        self.st().set_profile_pin_state(index, false)?;
        let mut cache = self.pin_cache();
        cache.remove(&index);
        self.save_pin_cache(&cache)
    }

    // ---------------------------------------------------------------- avatars

    /// `get_avatar_catalog` (active items by category and order), cached for offline use.
    pub async fn avatars(&self) -> Result<Vec<Avatar>, AccountError> {
        let remote = self.remote()?;
        let token = match self.token(false).await {
            Ok((t, _)) => t,
            Err(_) => remote.config.anon_key.clone(),
        };
        let fresh = remote.rpc("get_avatar_catalog", &token, &json!({})).await;
        let st = self.st();
        match fresh {
            Ok(v) => {
                let base = remote.config.url.clone();
                let mut items: Vec<(bool, Avatar)> = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|r| {
                                let path = r.get("storage_path")?.as_str()?.to_string();
                                Some((
                                    r.get("is_active").and_then(Value::as_bool).unwrap_or(true),
                                    Avatar {
                                        id: r.get("id")?.as_str()?.to_string(),
                                        display_name: r.get("display_name").and_then(Value::as_str).unwrap_or("").to_string(),
                                        category: r.get("category").and_then(Value::as_str).unwrap_or("character").to_string(),
                                        sort_order: r.get("sort_order").and_then(Value::as_i64).unwrap_or(0),
                                        bg_color: r.get("bg_color").and_then(Value::as_str).map(str::to_string),
                                        // `avatarStorageUrl`
                                        image_url: if path.starts_with("https://") || path.starts_with("http://") { path } else { format!("{base}/storage/v1/object/public/avatars/{path}") },
                                    },
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                items.retain(|(active, _)| *active);
                let mut list: Vec<Avatar> = items.into_iter().map(|(_, a)| a).collect();
                list.sort_by(|a, b| (a.category.as_str(), a.sort_order).cmp(&(b.category.as_str(), b.sort_order)));
                let _ = st.write("avatars", &list);
                Ok(list)
            }
            Err(e) => st.read::<Vec<Avatar>>("avatars").ok().flatten().filter(|c| !c.is_empty()).ok_or(AccountError::Remote(e)),
        }
    }
}

fn require_online(message: &'static str) -> impl Fn(AccountError) -> AccountError {
    move |e| match e {
        AccountError::Remote(RemoteError::Offline) | AccountError::SignedOut => AccountError::Store(message.into()),
        other => other,
    }
}

pub fn is_pin(pin: &str) -> bool {
    pin.len() == 4 && pin.chars().all(|c| c.is_ascii_digit())
}

fn mark_changed(report: &mut Report, kind: &str) {
    if !report.changed.iter().any(|k| k == kind) {
        report.changed.push(kind.to_string());
    }
}

/// `settings_json` of a `sync_pull_*settings*` answer.
fn blob_of(rows: &Value) -> Option<&Value> {
    rows.as_array()?.first()?.get("settings_json").filter(|v| v.is_object())
}

fn note(report: &mut Report, kind: &str, stats: &MergeStats) {
    report.received += stats.received;
    if stats.changed && !report.changed.iter().any(|k| k == kind) {
        report.changed.push(kind.to_string());
    }
}

// ---------------------------------------------------------------- account format ↔ local types

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}
fn i32_of(v: &Value, k: &str) -> Option<i32> {
    v.get(k).and_then(Value::as_i64).map(|n| n as i32)
}

/// `NuvioProfile` row.
fn profile_of(v: &Value) -> Option<Profile> {
    Some(Profile {
        index: v.get("profile_index")?.as_i64()?,
        name: s(v, "name").unwrap_or_default(),
        avatar_color_hex: s(v, "avatar_color_hex").unwrap_or_else(|| "#1E88E5".into()),
        avatar_id: s(v, "avatar_id"),
        uses_primary_addons: v.get("uses_primary_addons").and_then(Value::as_bool).unwrap_or(false),
        avatar_url: s(v, "avatar_url"),
        profile_background_id: s(v, "profile_background_id"),
        profile_background_url: s(v, "profile_background_url"),
        uses_primary_plugins: v.get("uses_primary_plugins").and_then(Value::as_bool).unwrap_or(false),
        pin_enabled: v.get("pin_enabled").and_then(Value::as_bool).unwrap_or(false),
        pin_locked_until: s(v, "pin_locked_until"),
        updated_at: s(v, "updated_at"),
    })
}

/// `ProfilePushPayload`.
fn profile_push(p: &Profile) -> Value {
    json!({
        "profile_index": p.index, "name": p.name, "avatar_color_hex": p.avatar_color_hex,
        "uses_primary_addons": p.uses_primary_addons, "uses_primary_plugins": p.uses_primary_plugins,
        "avatar_id": p.avatar_id, "avatar_url": p.avatar_url,
        "profile_background_id": p.profile_background_id, "profile_background_url": p.profile_background_url,
    })
}

/// `LibrarySyncItem` → local item.
fn library_of(v: &Value) -> Option<LibraryItem> {
    let shape = match s(v, "poster_shape").unwrap_or_default().to_ascii_uppercase().as_str() {
        "LANDSCAPE" => "Landscape",
        "SQUARE" => "Square",
        _ => "Poster",
    };
    serde_json::from_value(json!({
        "id": s(v, "content_id")?, "contentType": s(v, "content_type")?, "name": s(v, "name").unwrap_or_default(),
        "poster": s(v, "poster"), "banner": s(v, "background"), "logo": s(v, "logo"), "description": s(v, "description"),
        "releaseInfo": s(v, "release_info"),
        "imdbRating": v.get("imdb_rating").and_then(Value::as_f64).map(|r| { let t = format!("{r}"); if t.contains('.') { t } else { format!("{t}.0") } }),
        "genres": v.get("genres").cloned().unwrap_or(json!([])), "posterShape": shape,
        "savedAtEpochMs": v.get("added_at").and_then(Value::as_i64).unwrap_or(0), "addonBaseUrl": s(v, "addon_base_url"),
    }))
    .ok()
}

fn library_push(i: &LibraryItem) -> Value {
    json!({
        "content_id": i.id, "content_type": i.content_type, "name": i.name, "poster": i.poster,
        "poster_shape": i.poster_shape.to_ascii_uppercase(), "background": i.banner, "logo": i.logo,
        "description": i.description, "release_info": i.release_info,
        "imdb_rating": i.imdb_rating.as_deref().and_then(|r| r.parse::<f64>().ok()), "genres": i.genres,
        "addon_base_url": i.addon_base_url, "added_at": i.saved_at_epoch_ms,
    })
}

fn progress_of(v: &Value) -> Option<RemoteProgress> {
    Some(RemoteProgress {
        progress_key: s(v, "progress_key").unwrap_or_default(),
        content_id: s(v, "content_id").unwrap_or_default(),
        content_type: s(v, "content_type").unwrap_or_default(),
        video_id: s(v, "video_id").unwrap_or_default(),
        season: i32_of(v, "season"),
        episode: i32_of(v, "episode"),
        position: v.get("position").and_then(Value::as_i64).unwrap_or(0),
        duration: v.get("duration").and_then(Value::as_i64).unwrap_or(0),
        last_watched: v.get("last_watched").and_then(Value::as_i64).unwrap_or(0),
    })
    .filter(|p| !p.content_id.is_empty() || !p.progress_key.is_empty())
}

fn progress_push(e: &nuvio_store::library::ProgressEntry) -> Value {
    json!({
        "content_id": e.parent_meta_id, "content_type": e.content_type, "video_id": e.video_id,
        "season": e.season_number, "episode": e.episode_number, "position": e.last_position_ms,
        "duration": e.duration_ms, "last_watched": e.last_updated_epoch_ms, "progress_key": e.key(),
    })
}

/// `normalizeWatchedMarkedAtEpochMs`: compact `yyyyMMddHHmmss` values become epoch milliseconds.
pub fn normalize_watched_at(value: i64) -> i64 {
    if !(19000101000000..=29991231235959).contains(&value) {
        return value;
    }
    let t = format!("{value:014}");
    let n = |a: usize, b: usize| t[a..b].parse::<i64>().unwrap_or(-1);
    let (y, mo, d, h, mi, se) = (n(0, 4), n(4, 6), n(6, 8), n(8, 10), n(10, 12), n(12, 14));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || !(0..=23).contains(&h) || !(0..=59).contains(&mi) || !(0..=59).contains(&se) {
        return value;
    }
    // Days from civil (UTC).
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    ((days * 86400) + h * 3600 + mi * 60 + se) * 1000
}

fn watched_of(v: &Value) -> Option<WatchedItem> {
    Some(WatchedItem {
        id: s(v, "content_id")?,
        content_type: s(v, "content_type")?,
        name: s(v, "title").unwrap_or_default(),
        poster: None,
        release_info: None,
        season: i32_of(v, "season"),
        episode: i32_of(v, "episode"),
        video_id: None,
        marked_at_epoch_ms: normalize_watched_at(v.get("watched_at").and_then(Value::as_i64).unwrap_or(0)),
    })
}

fn watched_push(w: &WatchedItem) -> Value {
    json!({
        "content_id": w.id, "content_type": w.content_type, "title": w.name,
        "season": w.season, "episode": w.episode, "watched_at": normalize_watched_at(w.marked_at_epoch_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_watched_timestamps_are_normalized() {
        assert_eq!(normalize_watched_at(20240102030405), 1704164645000);
        assert_eq!(normalize_watched_at(1704164645000), 1704164645000);
        assert!(is_pin("0123") && !is_pin("12a4") && !is_pin("12345"));
    }
}
