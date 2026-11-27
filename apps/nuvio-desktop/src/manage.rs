//! Addon management and profiles (Settings → Addons, profile switcher in the sidebar).
//!
//! Addons follow `AddonRepository`: install from a manifest address (normalized, validated, no
//! duplicates, tracking addons need consent), enable/disable, reorder, refresh one manifest,
//! remove, and "Configurar" opens the addon's `/configure` page in the browser like the original.
//! Manifest URLs never reach the UI (they may carry configuration secrets): each addon is
//! identified by an opaque key and shown by host only.
//!
//! Profiles follow `ProfileRepository` (local): list, switch, create, edit (name, color, use the
//! primary profile's addons), delete (not profile 1, with backup). A switch closes the player first
//! (its progress is saved for the profile it was started in) and invalidates the addon list and the
//! stream keys of the previous profile.

use crate::services::addons::{self, fetch_manifest, summary_of};
use crate::AppState;
use nuvio_core::manifest::AddonManifest;
use nuvio_store::profiles::{addon_key, AddonError, ProfileError, MAX_PROFILES, PROFILE_COLORS};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub name: String,
    pub content_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonView {
    /// Opaque handle (the manifest URL stays in Rust).
    pub key: String,
    pub enabled: bool,
    /// `ok` (manifest loaded), `error` (enabled, did not load), `disabled` (not asked).
    pub status: &'static str,
    pub error: Option<String>,
    pub name: String,
    pub description: String,
    pub version: String,
    pub logo: Option<String>,
    pub types: Vec<String>,
    pub catalogs: Vec<CatalogView>,
    pub catalog_count: usize,
    pub resources: Vec<String>,
    pub configurable: bool,
    pub configuration_required: bool,
    pub tracking: bool,
    /// Host of the manifest address; `configured` when the address carries a configuration.
    pub host: String,
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonsView {
    pub profile: i64,
    /// Profile whose list is shown (1 for a profile that uses the primary profile's addons).
    pub addon_profile: i64,
    pub read_only: bool,
    pub addons: Vec<AddonView>,
}

/// `https://host/a/b/manifest.json?x` → (`host`, configured = path or query beyond the manifest).
fn host_of(url: &str) -> (String, bool) {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.rsplit('@').next().unwrap_or(host).to_string();
    let configured = url.contains('?') || path.trim_end_matches("manifest.json").trim_matches('/').contains('/') || path.trim_end_matches("manifest.json").trim_matches('/').len() > 24;
    (host, configured)
}

fn view_of(url: &str, enabled: bool, manifest: Option<&AddonManifest>, summary: Option<&nuvio_store::profiles::AddonSummary>, error: Option<&String>) -> AddonView {
    let (host, configured) = host_of(url);
    let status = if !enabled { "disabled" } else if manifest.is_some() { "ok" } else { "error" };
    match manifest {
        Some(m) => AddonView {
            key: addon_key(url),
            enabled,
            status,
            error: None,
            name: m.name.clone(),
            description: m.description.clone(),
            version: m.version.clone(),
            logo: m.logo_url.clone(),
            types: m.types.clone(),
            catalogs: m.catalogs.iter().map(|c| CatalogView { name: c.name.clone(), content_type: c.content_type.clone() }).collect(),
            catalog_count: m.catalogs.len(),
            resources: m.resources.iter().map(|r| r.name.clone()).collect(),
            configurable: m.behavior_hints.configurable || m.behavior_hints.configuration_required,
            configuration_required: m.behavior_hints.configuration_required,
            tracking: m.is_tracking_addon(),
            host,
            configured,
        },
        None => {
            let s = summary.cloned().unwrap_or_default();
            AddonView {
                key: addon_key(url),
                enabled,
                status,
                error: if enabled { Some(error.cloned().unwrap_or_else(|| "Não foi possível carregar o manifesto.".into())) } else { None },
                // `ManagedAddon.displayTitle` without a manifest: the last path segment.
                name: if s.name.is_empty() { host.clone() } else { s.name },
                description: s.description,
                version: s.version,
                logo: s.logo,
                types: s.types,
                catalogs: vec![],
                catalog_count: s.catalogs,
                resources: vec![],
                configurable: false,
                configuration_required: false,
                tracking: false,
                host,
                configured,
            }
        }
    }
}

async fn build_view(state: &AppState) -> Result<AddonsView, String> {
    state.ensure_unlocked()?;
    let loaded = state.addons.loaded().await?;
    let all = state.app_store.profiles().map_err(|e| e.to_string())?;
    let items = state.app_store.addons(loaded.addon_profile).map_err(|e| e.to_string())?;
    let by_url: std::collections::HashMap<&str, &AddonManifest> = loaded.manifests.iter().map(|m| (m.transport_url.as_str(), m)).collect();
    Ok(AddonsView {
        profile: loaded.profile,
        addon_profile: loaded.addon_profile,
        read_only: all.addons_read_only(loaded.profile),
        addons: items
            .iter()
            .map(|a| {
                let manifest = if a.enabled { by_url.get(a.manifest_url.as_str()).copied() } else { None };
                view_of(&a.manifest_url, a.enabled, manifest, a.summary.as_ref(), loaded.errors.get(&a.manifest_url))
            })
            .collect(),
    })
}

fn addon_result(r: Result<Result<(), AddonError>, nuvio_store::StoreError>) -> Result<(), String> {
    r.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

/// Installed addons of the active profile (enabled ones with their loaded manifest).
#[tauri::command]
pub async fn addons_list(state: State<'_, AppState>) -> Result<AddonsView, String> {
    build_view(&state).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonPreview {
    pub key: String,
    pub addon: AddonView,
    /// `player`/`library` resources: installing needs the user's consent (`TrackingAddonConsentDialog`).
    pub tracking: bool,
}

/// Step 1 of the install: normalize the address, refuse duplicates, load and validate the manifest
/// (one request, no cache). The manifest is kept for step 2, which makes no new request.
#[tauri::command]
pub async fn addon_preview(state: State<'_, AppState>, address: String) -> Result<AddonPreview, String> {
    let url = nuvio_core::url::normalize_manifest_url(&address).ok_or("Insira uma URL de addon.")?;
    if !(url.starts_with("https://") || url.starts_with("http://")) || host_of(&url).0.is_empty() {
        return Err("Insira uma URL de addon válida".into());
    }
    let profile = state.active_profile();
    let all = state.app_store.profiles().map_err(|e| e.to_string())?;
    if all.addons_read_only(profile) {
        return Err(AddonError::ReadOnlyProfile.to_string());
    }
    if state.app_store.addons(profile).map_err(|e| e.to_string())?.iter().any(|a| a.manifest_url == url) {
        return Err(AddonError::AlreadyInstalled.to_string());
    }
    let manifest = fetch_manifest(&state.addons.client, &url, true).await?;
    let addon = view_of(&url, true, Some(&manifest), None, None);
    let tracking = manifest.is_tracking_addon();
    *state.pending_addon.lock().expect("pending lock") = Some(manifest);
    Ok(AddonPreview { key: addon_key(&url), addon, tracking })
}

/// Step 2: installs the previewed addon for the active profile (enabled, at the end of the list).
#[tauri::command]
pub async fn addon_install(state: State<'_, AppState>, key: String, tracking_consent: bool) -> Result<AddonsView, String> {
    let manifest = {
        let pending = state.pending_addon.lock().expect("pending lock");
        pending.as_ref().filter(|m| addon_key(&m.transport_url) == key).cloned()
    }
    .ok_or("Verifique o endereço do addon novamente.")?;
    if manifest.is_tracking_addon() && !tracking_consent {
        return Err("Este addon de rastreamento só pode ser instalado com o seu consentimento.".into());
    }
    let profile = state.active_profile();
    addon_result(state.app_store.install_addon(profile, &manifest.transport_url, summary_of(&manifest)))?;
    state.pending_addon.lock().expect("pending lock").take();
    state.addons.remember(manifest).await;
    state.addons.invalidate();
    state.request_sync();
    build_view(&state).await
}

#[tauri::command]
pub async fn addon_set_enabled(state: State<'_, AppState>, key: String, enabled: bool) -> Result<AddonsView, String> {
    addon_result(state.app_store.set_addon_enabled(state.active_profile(), &key, enabled))?;
    state.addons.invalidate();
    state.request_sync();
    build_view(&state).await
}

/// Removes the addon from the profile's list only (library, history and progress are kept).
#[tauri::command]
pub async fn addon_remove(state: State<'_, AppState>, key: String) -> Result<AddonsView, String> {
    addon_result(state.app_store.remove_addon(state.active_profile(), &key))?;
    state.addons.invalidate();
    state.request_sync();
    build_view(&state).await
}

#[tauri::command]
pub async fn addon_move(state: State<'_, AppState>, key: String, delta: i64) -> Result<AddonsView, String> {
    addon_result(state.app_store.move_addon(state.active_profile(), &key, delta))?;
    state.addons.invalidate();
    state.request_sync();
    build_view(&state).await
}

fn url_of(state: &AppState, key: &str) -> Result<(i64, String), String> {
    let all = state.app_store.profiles().map_err(|e| e.to_string())?;
    let addon_profile = all.addon_profile(all.active);
    state
        .app_store
        .addons(addon_profile)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| addon_key(&a.manifest_url) == key)
        .map(|a| (addon_profile, a.manifest_url))
        .ok_or_else(|| AddonError::NotFound.to_string())
}

/// Manual "Atualizar": this addon's manifest only, without cache (`refreshAddon(forceRefresh)`).
#[tauri::command]
pub async fn addon_refresh(state: State<'_, AppState>, key: String) -> Result<AddonsView, String> {
    let (addon_profile, url) = url_of(&state, &key)?;
    let manifest = state.addons.refresh_one(&url).await?;
    let _ = state.app_store.update_addon_summary(addon_profile, &url, summary_of(&manifest));
    build_view(&state).await
}

/// "Configurar addon": opens `<addon>/configure` in the default browser, like the original. The
/// page gives a new manifest address, installed with the normal flow.
#[tauri::command]
pub fn addon_configure(state: State<'_, AppState>, key: String) -> Result<(), String> {
    let (_, url) = url_of(&state, &key)?;
    open_in_browser(&nuvio_core::url::configure_url(&url))
}

#[cfg(windows)]
pub(crate) fn open_in_browser(url: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("endereço inválido".into());
    }
    // SAFETY: plain Win32 call with owned, NUL-terminated strings.
    let result = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL) };
    if result.0 as isize > 32 { Ok(()) } else { Err("Não foi possível abrir o navegador.".into()) }
}

#[cfg(not(windows))]
pub(crate) fn open_in_browser(_: &str) -> Result<(), String> {
    Err("not supported".into())
}

// ---------------- Profiles ----------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesView {
    pub active: i64,
    /// The active profile is PIN-protected and was not unlocked yet (app start).
    pub locked: bool,
    pub profiles: Vec<addons::ProfileInfo>,
    pub max: i64,
    pub colors: Vec<&'static str>,
}

pub(crate) fn profiles_view(state: &AppState) -> Result<ProfilesView, String> {
    let (active, profiles) = addons::profiles(&state.app_store)?;
    let locked = state.pin_gate.lock().expect("pin gate").locked_active == Some(active);
    Ok(ProfilesView { active, locked, profiles, max: MAX_PROFILES, colors: PROFILE_COLORS.to_vec() })
}

fn profile_result<T>(r: Result<Result<T, ProfileError>, nuvio_store::StoreError>) -> Result<T, String> {
    r.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

/// Everything tied to the previous profile is dropped: the player (its progress is saved for that
/// profile first), the addon list, the stream keys and a pending install.
fn after_profile_change(state: &AppState) {
    state.close();
    state.addons.invalidate();
    state.streams.clear();
    state.pending_addon.lock().expect("pending lock").take();
}

#[tauri::command]
pub fn profiles_list(state: State<'_, AppState>) -> Result<ProfilesView, String> {
    profiles_view(&state)
}

/// Switches the active profile without restarting. Main thread (closes the player window).
#[tauri::command]
pub fn profile_switch(state: State<'_, AppState>, index: i64) -> Result<ProfilesView, String> {
    let target = state.app_store.profiles().map_err(|e| e.to_string())?.get(index).map(|p| p.pin_enabled);
    let protected = target.ok_or("Esse perfil não existe mais.")?;
    {
        // A protected profile opens only with the PIN just validated by `profile_unlock` (backend check).
        let mut gate = state.pin_gate.lock().expect("pin gate");
        let unlocked = gate.unlocked.remove(&index);
        if protected && !unlocked && !(state.active_profile() == index && gate.locked_active.is_none()) {
            return Err("Este perfil é protegido por PIN.".into());
        }
    }
    if state.active_profile() != index {
        // Player first, so its last position is saved for the profile it belongs to.
        state.close();
        profile_result(state.app_store.set_active_profile(index))?;
        after_profile_change(&state);
        state.request_sync();
    }
    // Only now: a switch that failed never opens the locked profile.
    state.pin_gate.lock().expect("pin gate").locked_active = None;
    profiles_view(&state)
}

#[tauri::command]
pub fn profile_create(state: State<'_, AppState>, name: String, color: String, uses_primary_addons: bool) -> Result<ProfilesView, String> {
    state.ensure_unlocked()?;
    profile_result(state.app_store.create_profile(&name, &color, uses_primary_addons))?;
    state.request_sync();
    profiles_view(&state)
}

#[tauri::command]
pub fn profile_update(state: State<'_, AppState>, index: i64, name: String, color: String, uses_primary_addons: bool, avatar_id: Option<String>) -> Result<ProfilesView, String> {
    state.ensure_unlocked()?;
    let before = state.app_store.profiles().map_err(|e| e.to_string())?;
    profile_result(state.app_store.update_profile(index, &name, &color, uses_primary_addons))?;
    if let Some(avatar) = avatar_id {
        // "" = back to color and initial.
        profile_result(state.app_store.set_profile_avatar(index, Some(avatar)))?;
    }
    state.request_sync();
    let after = state.app_store.profiles().map_err(|e| e.to_string())?;
    if index == after.active && before.addon_profile(index) != after.addon_profile(index) {
        state.addons.invalidate(); // the active profile now uses another addon list
        state.streams.clear();
    }
    profiles_view(&state)
}

/// Deletes a profile (not 1) and its data after a verified backup. Main thread (may close the player).
#[tauri::command]
pub fn profile_delete(state: State<'_, AppState>, index: i64) -> Result<ProfilesView, String> {
    state.ensure_unlocked()?;
    let was_active = state.active_profile() == index;
    if was_active {
        state.close();
    }
    profile_result(state.app_store.delete_profile(index))?;
    if was_active {
        after_profile_change(&state);
    }
    state.request_sync();
    profiles_view(&state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_only_and_configured_flag() {
        assert_eq!(host_of("https://addon.test/manifest.json"), ("addon.test".into(), false));
        let (host, configured) = host_of("https://torrentio.test/providers=a|b|c,qualityfilter=x/manifest.json");
        assert_eq!((host.as_str(), configured), ("torrentio.test", true));
        assert!(host_of("https://a.test/manifest.json?token=1").1);
        assert_eq!(host_of("https://user:pw@a.test/manifest.json").0, "a.test", "credentials never shown");
    }
}

/// Addon management and profile switching through Tauri's real IPC (mock runtime), against a
/// local addon server that counts manifest requests.
#[cfg(test)]
mod ipc_tests {
    use crate::services::addons::AddonContext;
    use crate::AppState;
    use nuvio_core::client::AddonClient;
    use serde_json::{json, Value};
    use std::io::{BufRead, BufReader, Write};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
    use tauri::webview::InvokeRequest;

    const GOOD: &str = r#"{"id":"test.addon","name":"Addon Bom","version":"1.2.0","description":"Catálogos de teste",
        "resources":["catalog","meta","stream"],"types":["movie","series"],"catalogs":[{"type":"movie","id":"top","name":"Populares"}],
        "behaviorHints":{"configurable":true}}"#;
    const OTHER: &str = r#"{"id":"test.other","name":"Outro","version":"1","resources":["stream"],"types":["movie"],"catalogs":[]}"#;
    const TRACKING: &str = r#"{"id":"test.track","name":"Rastreador","version":"1","resources":["player"],"types":["movie"],"catalogs":[]}"#;
    const MISSING_NAME: &str = r#"{"id":"x","version":"1","resources":[],"types":[]}"#;

    fn server(hits: Arc<AtomicUsize>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, body) = match path.as_str() {
                    "/good/manifest.json" => ("200 OK", GOOD),
                    "/other/manifest.json" => ("200 OK", OTHER),
                    "/track/manifest.json" => ("200 OK", TRACKING),
                    "/bad/manifest.json" => ("200 OK", MISSING_NAME),
                    "/html/manifest.json" => ("200 OK", "<html>not json</html>"),
                    _ => ("404 Not Found", "{}"),
                };
                if path.ends_with("manifest.json") {
                    hits.fetch_add(1, Ordering::SeqCst);
                }
                let mut out = stream;
                let _ = write!(out, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn call(window: &tauri::WebviewWindow<tauri::test::MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
        get_ipc_response(
            window,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|b| b.deserialize::<Value>().unwrap())
    }

    #[test]
    fn addons_and_profiles_contract() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = server(hits.clone());
        let dir = std::env::temp_dir().join(format!("nuvio-manage-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let client = AddonClient::new().unwrap();
        let state = AppState {
            api: Mutex::new(None),
            player: Mutex::new(None),
            session: AtomicU64::new(0),
            hwnd: Mutex::new(0),
            addons: AddonContext::new(client.clone(), dir.clone()),
            client,
            streams: crate::streams::StreamCache::default(),
            video: Mutex::new(nuvio_player::VideoConfig::default()),
            app_store: nuvio_store::nextgen::AppStore::open(&dir).unwrap(),
            progress: crate::services::progress::ProgressTracker::new(dir.clone()),
            pending_addon: Mutex::new(None),
            account: Arc::new(nuvio_sync::Engine::new(dir.clone(), None, None)),
            sync_wake: Arc::new(tokio::sync::Notify::new()),
            sync_signals: Default::default(),
            pip: Default::default(),
            pin_gate: Mutex::new(Default::default()),
            p2p: Default::default(),
            p2p_prepared: Mutex::new(None),
            p2p_session: Default::default(),
            youtube: nuvio_core::youtube::Extractor::new().unwrap(),
            updates: Default::default(),
            epg: Default::default(),
            playing: Default::default(),
            subtitle_choice: Default::default(),
            next_up: Default::default(),
            torbox: nuvio_core::torbox::Torbox::new().unwrap(),
            debrid_client: nuvio_core::debrid::Debrid::new().unwrap(),
            debrid: Default::default(),
            downloads: Default::default(),
            parental: Default::default(),
        };
        let app = mock_builder()
            .manage(state)
            .invoke_handler(tauri::generate_handler![
                super::addons_list,
                super::addon_preview,
                super::addon_install,
                super::addon_set_enabled,
                super::addon_remove,
                super::addon_move,
                super::addon_refresh,
                super::profiles_list,
                super::profile_switch,
                super::profile_create,
                super::profile_update,
                super::profile_delete,
                crate::api::library_toggle_favorite,
                crate::api::library_favorite_keys,
                crate::account::profile_unlock
            ])
            .build(mock_context(noop_assets()))
            .unwrap();
        let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();

        assert_eq!(call(&w, "addons_list", json!({})).unwrap()["addons"], json!([]));
        // Install: preview (one request) then install (no new request).
        let preview = call(&w, "addon_preview", json!({ "address": format!("{base}/good/") })).unwrap();
        assert_eq!(preview["addon"]["name"], "Addon Bom");
        assert_eq!(preview["addon"]["catalogs"][0]["name"], "Populares");
        assert_eq!(preview["addon"]["configurable"], true);
        assert!(!preview.to_string().contains("/good/"), "no manifest URL in the UI data: {preview}");
        let list = call(&w, "addon_install", json!({ "key": preview["key"], "trackingConsent": false })).unwrap();
        assert_eq!(list["addons"][0]["status"], "ok");
        assert_eq!(list["addons"][0]["version"], "1.2.0");
        assert_eq!(hits.load(Ordering::SeqCst), 1, "install reused the previewed manifest");
        // Duplicate (same address typed differently), invalid manifests, unreachable address.
        let dup = call(&w, "addon_preview", json!({ "address": format!("{base}/good/manifest.json#x") })).unwrap_err();
        assert_eq!(dup, "Esse addon já está instalado.");
        assert!(call(&w, "addon_preview", json!({ "address": format!("{base}/bad") })).unwrap_err().as_str().unwrap().contains("Campo ausente no manifesto"));
        assert!(call(&w, "addon_preview", json!({ "address": format!("{base}/html") })).unwrap_err().as_str().unwrap().contains("não é um manifesto"));
        assert!(call(&w, "addon_preview", json!({ "address": format!("{base}/nothing") })).unwrap_err().as_str().unwrap().contains("HTTP 404"));
        assert!(call(&w, "addon_preview", json!({ "address": "http://127.0.0.1:1/x" })).unwrap_err().as_str().unwrap().contains("conectar"));
        assert_eq!(call(&w, "addon_preview", json!({ "address": "  " })).unwrap_err(), "Insira uma URL de addon.");
        // Tracking addon: consent required.
        let track = call(&w, "addon_preview", json!({ "address": format!("{base}/track") })).unwrap();
        assert_eq!(track["tracking"], true);
        assert!(call(&w, "addon_install", json!({ "key": track["key"], "trackingConsent": false })).is_err());
        call(&w, "addon_install", json!({ "key": track["key"], "trackingConsent": true })).unwrap();
        let other = call(&w, "addon_preview", json!({ "address": format!("{base}/other") })).unwrap();
        let list = call(&w, "addon_install", json!({ "key": other["key"], "trackingConsent": false })).unwrap();
        let keys: Vec<String> = list["addons"].as_array().unwrap().iter().map(|a| a["key"].as_str().unwrap().to_string()).collect();
        assert_eq!(keys.len(), 3);

        // Disable: kept, shown from the saved summary, no request; enable again: nothing asked.
        let before = hits.load(Ordering::SeqCst);
        let list = call(&w, "addon_set_enabled", json!({ "key": keys[0], "enabled": false })).unwrap();
        assert_eq!((list["addons"][0]["status"].clone(), list["addons"][0]["name"].clone()), (json!("disabled"), json!("Addon Bom")));
        assert_eq!(hits.load(Ordering::SeqCst), before, "disabling asks nothing");
        call(&w, "addon_set_enabled", json!({ "key": keys[0], "enabled": true })).unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), before, "manifest already known: still nothing asked");
        let list = call(&w, "addon_refresh", json!({ "key": keys[0] })).unwrap();
        assert_eq!(list["addons"][0]["status"], "ok");
        assert_eq!(hits.load(Ordering::SeqCst), before + 1, "manual refresh asks that addon only");
        let list = call(&w, "addon_move", json!({ "key": keys[2], "delta": -1 })).unwrap();
        assert_eq!(list["addons"][1]["key"], keys[2].as_str());
        let list = call(&w, "addon_remove", json!({ "key": keys[1] })).unwrap();
        assert_eq!(list["addons"].as_array().unwrap().len(), 2);

        // Profiles: create, switch (own addons and favorites), shared primary addons are read-only.
        let p = call(&w, "profile_create", json!({ "name": "Dois", "color": "#E53935", "usesPrimaryAddons": false })).unwrap();
        assert_eq!(p["profiles"].as_array().unwrap().len(), 2);
        call(&w, "library_toggle_favorite", json!({ "item": { "id": "tt1", "contentType": "movie", "name": "Um" } })).unwrap();
        // PIN gate (backend): a protected profile does not open without its PIN.
        {
            use tauri::Manager;
            app.state::<AppState>().app_store.set_profile_pin_state(2, true).unwrap();
        }
        assert_eq!(call(&w, "profile_switch", json!({ "index": 2 })).unwrap_err(), "Este perfil é protegido por PIN.");
        let unlock = call(&w, "profile_unlock", json!({ "index": 2, "pin": "1234" })).unwrap();
        assert_eq!(unlock["unlocked"], false, "no account and no verified PIN on this computer");
        assert!(unlock["message"].as_str().unwrap().contains("conta Nuvio"));
        {
            use tauri::Manager;
            app.state::<AppState>().app_store.set_profile_pin_state(2, false).unwrap();
        }
        let p = call(&w, "profile_switch", json!({ "index": 2 })).unwrap();
        assert_eq!(p["active"], 2);
        assert_eq!(call(&w, "addons_list", json!({})).unwrap()["addons"], json!([]), "profile 2 has its own (empty) list");
        assert_eq!(call(&w, "library_favorite_keys", json!({})).unwrap(), json!([]), "no favorite of profile 1");
        call(&w, "profile_update", json!({ "index": 2, "name": "Dois", "color": "#E53935", "usesPrimaryAddons": true })).unwrap();
        let shared = call(&w, "addons_list", json!({})).unwrap();
        assert_eq!((shared["readOnly"].clone(), shared["addons"].as_array().unwrap().len()), (json!(true), 2));
        assert_eq!(call(&w, "addon_remove", json!({ "key": keys[0] })).unwrap_err(), "Este perfil usa addons primários.");
        // Delete: profile 1 refused; profile 2 removed with its data, profile 1 active again.
        assert!(call(&w, "profile_delete", json!({ "index": 1 })).is_err());
        let p = call(&w, "profile_delete", json!({ "index": 2 })).unwrap();
        assert_eq!((p["active"].clone(), p["profiles"].as_array().unwrap().len()), (json!(1), 1));
        assert_eq!(call(&w, "library_favorite_keys", json!({})).unwrap(), json!(["movie/tt1"]), "profile 1 data untouched");
        assert_eq!(call(&w, "addons_list", json!({})).unwrap()["addons"].as_array().unwrap().len(), 2);
    }
}
