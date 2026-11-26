//! Nuvio Astra desktop shell (Windows x64): Tauri 2 window + embedded libmpv.
//!
//! The UI talks to Rust only through the typed commands below (no file system, shell or raw
//! network access is exposed to the webview). Stream URLs, headers and tokens stay in Rust.
//! Player state flows back as `player-state` events.
//!
//! Sync commands run on the main thread, which owns the window: every command that creates,
//! moves or destroys a Win32 window is sync for that reason.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account;
mod api;
mod audio_sync;
mod bench;
mod capture;
mod manage;
mod p2p;
mod pip;
mod selftest;
mod services;
mod streams;
mod watching;
mod trailer;
mod updater;
mod uitest;

use nuvio_core::client::AddonClient;
use nuvio_player::mpv::MpvApi;
use nuvio_player::{LoadRequest, Player, PlayerCommand, PlayerSnapshot, VideoConfig};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub struct AppState {
    api: Mutex<Option<Arc<MpvApi>>>,
    player: Mutex<Option<Player>>,
    session: AtomicU64,
    hwnd: Mutex<isize>,
    pub client: AddonClient,
    pub streams: streams::StreamCache,
    /// Direct-download transfers and their cancellation flags.
    pub downloads: services::downloads::Manager,
    pub video: Mutex<VideoConfig>,
    pub addons: services::addons::AddonContext,
    pub app_store: nuvio_store::nextgen::AppStore,
    /// Watch progress of the current title (saved off the player thread).
    pub progress: services::progress::ProgressTracker,
    /// Manifest loaded by the install preview, installed by `addon_install` without a new request.
    pub pending_addon: Mutex<Option<nuvio_core::manifest::AddonManifest>>,
    /// Nuvio account (session in Rust only) and its sync engine.
    pub account: Arc<nuvio_sync::Engine>,
    /// Wakes the background sync (local changes, profile switch).
    pub sync_wake: Arc<tokio::sync::Notify>,
    pub sync_signals: Arc<account::SyncSignals>,
    /// The picture-in-picture window while it exists (`pip.rs`).
    pub pip: Mutex<Option<tauri::WebviewWindow>>,
    /// Local relay for streams that need request headers on every HLS request.
    pub relay: Mutex<Option<nuvio_core::relay::Relay>>,
    pub pin_gate: Mutex<account::PinGate>,
    /// P2P session (original's Nuvio Engine) of torrent streams.
    pub p2p: Arc<nuvio_p2p::P2pService>,
    /// Stream key and file of the prepared torrent (`torrent_prepare` → `stream_play`).
    pub p2p_prepared: Mutex<Option<(String, nuvio_p2p::PreparedStream)>>,
    /// Player session that plays the P2P stream (its close ends the P2P session).
    pub p2p_session: AtomicU64,
    /// YouTube trailers (resolved on click only).
    pub youtube: nuvio_core::youtube::Extractor,
    /// Update found by the last check (official Tauri updater).
    pub updates: updater::UpdateState,
    /// Program guides of the live channels opened (memory only, bounded).
    pub epg: services::epg::EpgStore,
    /// Nuvio scraper plugins (repositories, validated scripts and their runs).
    pub plugins: services::plugins::Manager,
    /// The movie or episode playing (subtitles, skip buttons).
    pub playing: Mutex<Option<Arc<watching::Playing>>>,
    pub subtitle_choice: Mutex<Option<watching::Choice>>,
    /// Episode lists behind "next episode" in Continue watching.
    pub next_up: services::next_up::Lists,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRequest {
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub start_seconds: f64,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct CssRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerInfo {
    pub session: u64,
    pub libmpv_api: String,
}

impl AppState {
    pub fn api(&self) -> Result<Arc<MpvApi>, String> {
        let mut slot = self.api.lock().expect("api lock");
        if let Some(api) = slot.as_ref() {
            return Ok(api.clone());
        }
        let path = nuvio_player::locate_libmpv().ok_or_else(|| {
            #[cfg(windows)]
            { "libmpv-2.dll not found (next to the exe or NUVIO_LIBMPV)".to_string() }
            #[cfg(not(windows))]
            { "A reprodução de vídeo ainda não está disponível no Linux (defina NUVIO_LIBMPV para testar)".to_string() }
        })?;
        let api = MpvApi::load(&path).map_err(|e| e.to_string())?;
        *slot = Some(api.clone());
        Ok(api)
    }

    /// Opens a new session, closing the previous one first (one player at a time). Main thread.
    pub fn open(&self, app: &AppHandle, url: String, headers: Vec<(String, String)>, start_seconds: f64) -> Result<PlayerInfo, String> {
        self.open_with_audio(app, url, None, headers, start_seconds)
    }

    /// `open` with a separate audio URL (trailers).
    pub fn open_with_audio(&self, app: &AppHandle, url: String, audio_url: Option<String>, headers: Vec<(String, String)>, start_seconds: f64) -> Result<PlayerInfo, String> {
        let api = self.api()?;
        self.close();
        let session = self.session.fetch_add(1, Ordering::SeqCst) + 1;
        let emitter = app.clone();
        let profile = self.active_profile();
        let saved_changes = std::sync::atomic::AtomicU32::new(0);
        let sink: nuvio_player::SnapshotSink = Arc::new(move |snapshot: &PlayerSnapshot| {
            let state = emitter.state::<AppState>();
            state.progress.observe(snapshot);
            crate::services::servers::observe_snapshot(
                snapshot.loaded,
                snapshot.paused,
                snapshot.buffering,
                snapshot.ended,
                snapshot.time_pos,
                snapshot.duration,
            );
            if saved_changes.swap(snapshot.auto_changes, Ordering::SeqCst) != snapshot.auto_changes {
                // Only what the watchdog lowered.
                let (motion, enhancement, picture) = match snapshot.auto_reason {
                    Some("motion") => (Some(snapshot.motion), None, None),
                    Some("effects") => (None, None, Some(snapshot.picture)),
                    _ => (None, Some(snapshot.enhancement), None),
                };
                services::playback::save_improvements(&state.app_store, profile, motion, enhancement, picture);
            }
            let _ = emitter.emit("player-state", snapshot);
        });
        let load = LoadRequest {
            url,
            headers,
            start_seconds,
            paused: false,
            video: self.video.lock().expect("video lock").clone(),
            audio_url,
        };
        let hwnd = *self.hwnd.lock().expect("hwnd lock");
        let player = Player::open(api.clone(), hwnd, session, &load, sink).map_err(|e| e.to_string())?;
        *self.player.lock().expect("player lock") = Some(player);
        let (major, minor) = api.client_api_version();
        Ok(PlayerInfo { session, libmpv_api: format!("{major}.{minor}") })
    }

    /// Main thread (destroys the video container).
    pub fn close(&self) {
        *self.playing.lock().expect("playing lock") = None;
        // Take it out first so shutdown (which joins threads) runs without holding the lock.
        let player = self.player.lock().expect("player lock").take();
        // Last position of a tracked title (close, switch to another title, app exit).
        if let Some(player) = &player {
            let snapshot = player.snapshot();
            self.progress.finish(Some(&snapshot));
            services::servers::end_playback();
            p2p::on_player_closed(self, snapshot.session);
        }
        // Dropping moves the video back out of the PiP window, which can then go.
        drop(player);
        // The local relay only exists for the stream that was playing.
        *self.relay.lock().expect("relay lock") = None;
        pip::close_window(self);
    }

    /// Active profile (NextGen `profiles.json`; changed by `profile_switch`).
    pub fn active_profile(&self) -> i64 {
        self.app_store.profiles().map(|p| p.active).unwrap_or(1)
    }

    pub fn with_player<T>(&self, f: impl FnOnce(&Player) -> Result<T, String>) -> Result<T, String> {
        let guard = self.player.lock().expect("player lock");
        let player = guard.as_ref().ok_or_else(|| "no active player".to_string())?;
        f(player)
    }
}

/// Plays an arbitrary URL (test UI). Main thread.
#[tauri::command]
fn player_open(app: AppHandle, state: tauri::State<'_, AppState>, request: OpenRequest) -> Result<PlayerInfo, String> {
    state.open(&app, request.url, request.headers, request.start_seconds)
}

/// Streams of one title from the installed addons (on demand, never in bulk).
#[tauri::command]
async fn streams_search(
    state: tauri::State<'_, AppState>,
    content_type: String,
    id: String,
    profile: Option<i64>,
) -> Result<Vec<streams::StreamGroup>, String> {
    streams::search(&state.client, &state.streams, &content_type, &id, profile).await
}

/// Plays a stream returned by `streams_search`, with its URL and headers as given by the addon.
/// Main thread.
#[tauri::command]
///
/// `progress`: the movie/episode being played (Details, Library), so its position is saved for the
/// active profile; absent for live TV.
async fn stream_play(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    key: String,
    start_seconds: Option<f64>,
    progress: Option<nuvio_store::library::PlaybackItem>,
    previous_finished: Option<bool>,
) -> Result<PlayerInfo, String> {
    let stream = state.streams.get(&key).ok_or("unknown stream (search again)")?;
    let kind = streams::kind_of(&stream);
    // With debrid services connected, torrent sources are resolved to direct links instead of
    // starting P2P. Every connected provider is tried in turn (preferred one first), so a torrent
    // that is not cached on TorBox can still play from Real-Debrid.
    let profile = state.active_profile();
    let direct = stream.playable_direct_url().unwrap_or_default();
    let is_share = services::debrid::is_share_url(direct);
    let use_debrid = matches!(kind, streams::StreamKind::Torrent | streams::StreamKind::Debrid) || is_share;
    let mut providers = if use_debrid { services::debrid::available(&state.app_store, profile) } else { Vec::new() };
    // A duplicated card is bound to one provider; a normal card tries them all in order.
    if let Some(id) = stream.forced_debrid.as_deref() {
        providers.retain(|active| active.provider.id() == id);
    }

    let mut resolved: Option<String> = None;
    let mut last_error: Option<String> = None;
    for active in &providers {
        let attempt = if matches!(kind, streams::StreamKind::Torrent | streams::StreamKind::Debrid) {
            services::debrid::resolve(active, &stream).await
        } else {
            services::debrid::unrestrict_share(active, direct).await.ok_or_else(|| "Não é uma ligação de partilha deste serviço.".to_string())
        };
        match attempt {
            Ok(link) => {
                eprintln!("[debrid] {} resolved", active.provider.code());
                resolved = Some(link);
                break;
            }
            Err(error) => {
                eprintln!("[debrid] {} failed: {error}", active.provider.code());
                last_error = Some(error);
            }
        }
    }
    if use_debrid && resolved.is_none() && (kind == streams::StreamKind::Debrid || is_share) {
        return Err(last_error.unwrap_or_else(|| "Nenhum serviço de debrid conseguiu resolver esta fonte.".into()));
    }
    let torrent = kind == streams::StreamKind::Torrent && resolved.is_none();
    // `ytId` sources are resolved by the in-app YouTube extractor, like a trailer, and played
    // with their separate audio track when the extractor found one.
    let mut youtube_audio: Option<String> = None;
    let mut url = if let Some(link) = resolved {
        link
    } else if torrent {
        p2p::prepared_url(&state, &key)?
    } else if kind == streams::StreamKind::YouTube {
        let id = stream
            .yt_id
            .as_deref()
            .and_then(nuvio_core::youtube::video_id)
            .ok_or("invalid YouTube source")?;
        let source = state.youtube.resolve(&id).await?;
        youtube_audio = source.audio_url;
        source.video_url
    } else {
        stream.playable_direct_url().ok_or("stream has no playable URL")?.to_string()
    };

    // The active profile's player settings (Settings → Reprodução / Áudio / Legendas).
    let profile = state.active_profile();
    let settings = state.app_store.profile_settings(profile).map(|s| s.player).unwrap_or_default();
    // Only movies and episodes (they carry `progress`) get the picture improvements.
    *state.video.lock().expect("video lock") = match progress {
        Some(_) => services::playback::video_config(&settings),
        None => services::playback::video_config_unenhanced(&settings),
    };
    let url_host = url.split("://").nth(1).unwrap_or(&url).split('/').next().unwrap_or("").to_string();
    eprintln!("[play] kind={:?} forced={:?} providers={:?} url_host={url_host:?}", kind, stream.forced_debrid, services::debrid::available_codes(&state.app_store, profile));
    let mut headers = if torrent { Vec::new() } else { stream.playback_request_headers() };
    // A player applies headers to the main URL only. When the addon needs them, put them on every
    // request through a local relay (HLS playlists and segments included). `NUVIO_RELAY=0` disables.
    *state.relay.lock().expect("relay lock") = None;
    if !headers.is_empty() && std::env::var("NUVIO_RELAY").ok().as_deref() != Some("0") {
        let relay = nuvio_core::relay::Relay::start(url.clone(), headers.clone()).await.map_err(|e| e.to_string())?;
        url = relay.url().to_string();
        *state.relay.lock().expect("relay lock") = Some(relay);
        headers = Vec::new();
    }
    if previous_finished == Some(true) {
        state.progress.mark_finished();
    }
    let source_url = url.clone();
    let info = state.open_with_audio(&app, url, youtube_audio, headers, start_seconds.unwrap_or(0.0))?;
    let parent_id = progress.as_ref().map(|item| item.parent_meta_id.clone());
    if let Some(item) = &progress {
        let playing = watching::Playing::new(info.session, item, &stream, &source_url);
        *state.playing.lock().expect("playing lock") = Some(Arc::new(playing));
    }
    if torrent {
        state.p2p_session.store(info.session, Ordering::SeqCst);
    }
    if let Some(mut item) = progress {
        // Display names only (no URL): which addon and source were used last time.
        item.provider_name = Some(stream.addon_name.clone());
        item.last_stream_title = stream.name.clone().or_else(|| stream.title.clone());
        state.progress.start(info.session, profile, item, settings.watched_percent);
    }

    let font = services::fonts::mpv_font(&state.app_store, &settings.subtitle.font_id);
    let device = services::playback::device_languages();
    let audio = services::playback::audio_targets(&settings, &device);
    let subtitles = watching::subtitle_languages(&state, parent_id.as_deref(), services::playback::subtitle_targets(&settings, &device));
    let wanted: Vec<String> = subtitles.iter().filter(|language| *language != "forced").cloned().collect();
    for command in services::playback::startup_commands(&settings, &audio, font) {
        let _ = state.with_player(|p| p.execute(&command).map_err(|e| e.to_string()));
    }
    let handle = app.clone();
    let session = info.session;
    std::thread::spawn(move || {
        let state = handle.state::<AppState>();
        services::playback::select_tracks_when_loaded(
            || state.with_player(|p| Ok(p.snapshot())).ok(),
            |command| {
                let _ = state.with_player(|p| p.execute(&command).map_err(|e| e.to_string()));
            },
            session,
            audio,
            subtitles,
            std::time::Duration::from_secs(20),
        );
        if parent_id.is_some() && !wanted.is_empty() {
            // The selection above takes a moment to show in the player's state.
            std::thread::sleep(std::time::Duration::from_millis(700));
            tauri::async_runtime::spawn(watching::select_from_addons(handle.clone(), session, wanted));
        }
    });
    Ok(info)
}

/// Shows the main window maximized the first time (called by the interface after its first paint).
fn show_main_window(window: &tauri::WebviewWindow) {
    if !window.is_visible().unwrap_or(true) {
        let _ = window.maximize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    // The WM reparents the X11 host into its frame and unmaps child windows; remap the video
    // container once the window is really visible.
    #[cfg(target_os = "linux")]
    {
        let _ = window.app_handle().state::<AppState>().with_player(|p| {
            p.fit_to_host();
            Ok(())
        });
    }
}

#[tauri::command]
fn window_ready(window: tauri::WebviewWindow) {
    show_main_window(&window);
}

/// Gives keyboard focus back to the WebView. On Linux the video is a native X11 window and the
/// player controls live in the page, so the page must stay the key-focus owner while it plays.
#[tauri::command]
fn player_focus(window: tauri::WebviewWindow) {
    let _ = window.set_focus();
}

/// `"selftest"` when the window is driven by the automated player tests (the UI then shows only
/// the video overlay and loads no catalogs), `"uitest"` for the automated UI test (the normal UI
/// plus its driver), `"app"` otherwise.
#[tauri::command]
fn app_mode() -> &'static str {
    if selftest::requested().is_some() {
        "selftest"
    } else if uitest::requested().is_some() {
        "uitest"
    } else {
        "app"
    }
}

/// The host OS (`linux`, `windows`, `macos`…). The player needs it because on Linux the video is a
/// native X11 window above the WebView: the page keeps the video out of the player chrome instead
/// of drawing the chrome over it (`web/src/player/FullPlayer.tsx`).
#[tauri::command]
fn platform() -> &'static str {
    std::env::consts::OS
}

#[tauri::command]
fn player_command(state: tauri::State<'_, AppState>, command: PlayerCommand) -> Result<(), String> {
    state.with_player(|p| p.execute(&command).map_err(|e| e.to_string()))?;
    if let PlayerCommand::SetSubtitleTrack { id } = &command {
        watching::track_chosen(&state, id);
    }
    let (motion, enhancement, picture) = match &command {
        PlayerCommand::SetMotion { motion } => (Some(*motion), None, None),
        PlayerCommand::SetEnhancement { enhancement } => (None, Some(*enhancement), None),
        PlayerCommand::SetPicture { picture } => (None, None, Some(*picture)),
        _ => return Ok(()),
    };
    services::playback::save_improvements(&state.app_store, state.active_profile(), motion, enhancement, picture);
    Ok(())
}

/// `rect` in CSS pixels of the webview; `None` = whole window. `holes` are regions of that
/// rectangle the page needs for its chrome (Linux cuts them out of the native video window).
/// `scale` = devicePixelRatio.
#[tauri::command]
fn player_video_rect(state: tauri::State<'_, AppState>, rect: Option<CssRect>, holes: Option<Vec<CssRect>>, scale: f64) -> Result<(), String> {
    let holes = holes.unwrap_or_default();
    state.with_player(|p| {
        p.set_video_rect(
            rect.map(|r| (r.x * scale, r.y * scale, r.width * scale, r.height * scale)),
            holes.into_iter().map(|r| (r.x * scale, r.y * scale, r.width * scale, r.height * scale)).collect(),
        )
        .map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn player_snapshot(state: tauri::State<'_, AppState>) -> Result<PlayerSnapshot, String> {
    state.with_player(|p| Ok(p.snapshot()))
}

#[tauri::command]
fn player_close(state: tauri::State<'_, AppState>) {
    state.close();
}

/// Video settings for the next session (decoder priority, RTX filters), like the original's
/// player settings.
#[tauri::command]
fn player_video_config(state: tauri::State<'_, AppState>, config: VideoConfig) {
    *state.video.lock().expect("video lock") = config;
}

pub fn set_picture_in_picture(app: &AppHandle, state: &AppState, enabled: bool, title: &str) -> Result<(), String> {
    pip::set(app, state, enabled, title)
}

/// Borderless fullscreen driven directly by Win32. `tao`'s `set_fullscreen` posts the style/position
/// change to the event loop and, with a maximized WebView2 window plus the native mpv child, blocks
/// the UI thread in this build (the keyboard stops answering). Driving the HWND directly keeps the
/// message pump free and restores the exact previous placement on exit.
#[cfg(windows)]
static FULLSCREEN_STATE: std::sync::Mutex<Option<(windows::Win32::UI::WindowsAndMessaging::WINDOWPLACEMENT, i32, i32)>> = std::sync::Mutex::new(None);

#[tauri::command]
fn window_set_fullscreen(window: tauri::WebviewWindow, fullscreen: bool) -> Result<bool, String> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
        use windows::Win32::UI::WindowsAndMessaging::*;

        eprintln!("[fs] request={fullscreen}");
        let hwnd = HWND(window.hwnd().map_err(|e| e.to_string())?.0 as *mut std::ffi::c_void);
        unsafe {
            if fullscreen {
                if FULLSCREEN_STATE.lock().expect("fullscreen state").is_some() {
                    return Ok(true);
                }
                let mut placement = WINDOWPLACEMENT::default();
                placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
                GetWindowPlacement(hwnd, &mut placement).map_err(|e| e.to_string())?;
                let style = GetWindowLongW(hwnd, GWL_STYLE);
                let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
                *FULLSCREEN_STATE.lock().expect("fullscreen state") = Some((placement, style, ex_style));
                // No ShowWindow(SW_RESTORE): it sends WM_SIZE re-entrantly on this (main) thread.
                let borderless = (style
                    & !((WS_CAPTION.0 | WS_THICKFRAME.0 | WS_MAXIMIZEBOX.0 | WS_MINIMIZEBOX.0
                        | WS_SYSMENU.0 | WS_MAXIMIZE.0 | WS_BORDER.0 | WS_DLGFRAME.0) as i32))
                    | WS_POPUP.0 as i32;
                SetWindowLongW(hwnd, GWL_STYLE, borderless);
                SetWindowLongW(
                    hwnd,
                    GWL_EXSTYLE,
                    ex_style & !((WS_EX_WINDOWEDGE.0 | WS_EX_CLIENTEDGE.0 | WS_EX_STATICEDGE.0) as i32),
                );
                let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                    return Err("GetMonitorInfoW failed".into());
                }
                let r = info.rcMonitor;
                // Synchronous (no SWP_ASYNCWINDOWPOS): the style/size change must be finished
                // before we move focus back into WebView2 or resize the video container.
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_FRAMECHANGED | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
                )
                .map_err(|e| e.to_string())?;
                eprintln!("[fs] SetWindowPos ok");
                let _ = SetForegroundWindow(hwnd);
                let _ = window.set_focus();
                let _ = window.eval("window.focus();");
                eprintln!("[fs] focus restored");
                let _ = window.app_handle().state::<AppState>().with_player(|p| {
                    p.fit_container_to_host();
                    Ok(())
                });
                eprintln!("[fs] container resized");
                return Ok(true);
            }

            let saved = FULLSCREEN_STATE.lock().expect("fullscreen state").take();
            let Some((placement, style, ex_style)) = saved else {
                return Ok(window.is_fullscreen().unwrap_or(false));
            };
            SetWindowLongW(hwnd, GWL_STYLE, style);
            SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style);
            SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(|e| e.to_string())?;
            SetWindowPlacement(hwnd, &placement).map_err(|e| e.to_string())?;
            eprintln!("[fs] restored placement");
            let _ = SetForegroundWindow(hwnd);
            let _ = window.set_focus();
            let _ = window.eval("window.focus();");
            let _ = window.app_handle().state::<AppState>().with_player(|p| {
                p.fit_container_to_host();
                Ok(())
            });
            eprintln!("[fs] left fullscreen");
            return Ok(false);
        }
    }
    #[cfg(not(windows))]
    {
        window.set_fullscreen(fullscreen).map_err(|e| e.to_string())?;
        Ok(window.is_fullscreen().unwrap_or(false))
    }
}

/// Makes the webview background transparent so the video container below shows through
/// (`ICoreWebView2Controller2::put_DefaultBackgroundColor`, as in player_bridge.cpp).
/// On Linux WebKitGTK already honours the transparent window background; there is no video
/// container yet (phase 2 of the port), so this is a no-op.
#[cfg(windows)]
fn make_webview_transparent(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.with_webview(|webview| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Controller2, COREWEBVIEW2_COLOR};
        use windows::core::Interface;
        let controller = webview.controller();
        if let Ok(controller2) = controller.cast::<ICoreWebView2Controller2>() {
            // SAFETY: COM call on the UI thread (with_webview runs on it).
            let _ = unsafe { controller2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { R: 0, G: 0, B: 0, A: 0 }) };
        }
    })
}

/// Asks WebView2 to lower its memory use (`ICoreWebView2_19::put_MemoryUsageTargetLevel`).
/// Measured in the player self-test before being used by the app.
#[cfg(windows)]
pub fn set_webview_low_memory(window: &tauri::WebviewWindow, low: bool) -> tauri::Result<()> {
    window.with_webview(move |webview| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
        };
        use windows::core::Interface;
        // SAFETY: COM calls on the UI thread (with_webview runs on it).
        unsafe {
            if let Ok(core) = webview.controller().CoreWebView2() {
                if let Ok(core19) = core.cast::<ICoreWebView2_19>() {
                    let level = if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
                    let _ = core19.SetMemoryUsageTargetLevel(level);
                }
            }
        }
    })
}

#[cfg(not(windows))]
fn make_webview_transparent(_window: &tauri::WebviewWindow) -> tauri::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn set_webview_low_memory(_window: &tauri::WebviewWindow, _low: bool) -> tauri::Result<()> {
    Ok(())
}

/// NextGen data store (`%APPDATA%\NuvioNextGen`), migrated once from the current app's data:
/// verified backup first, originals never modified, IDs kept. A failed migration is reported
/// and retried on the next start; the app still opens with what the legacy store provides.
fn open_app_store() -> nuvio_store::nextgen::AppStore {
    let real = nuvio_store::nextgen::default_app_dir().expect("APPDATA");
    // UI test runs work on a copy of the real data (see `uitest::data_dir`).
    let dir = uitest::data_dir(&real).unwrap_or(real);
    let store = nuvio_store::nextgen::AppStore::open(dir).expect(r"data directory (%APPDATA%\NuvioNextGen)");
    match services::addons::legacy_store() {
        Ok(legacy) => match nuvio_store::nextgen::migrate_from_legacy(&store, &legacy) {
            Ok(record) => eprintln!(
                "[nextgen] migration ok: {} backup files, {} profiles, {} live TV favorites, library {} saved / {} progress / {} watched",
                record.backup_files,
                record.profiles,
                record.live_tv_favorites,
                record.library.saved_items,
                record.library.progress_entries,
                record.library.watched_items
            ),
            Err(e) => eprintln!("[nextgen] migration failed (will retry next start): {e}"),
        },
        Err(e) => eprintln!("[nextgen] no legacy data to migrate: {e}"),
    }
    store
}

fn main() {
    // The Linux player embeds libmpv into an X11 `wid`. Force the X11 GDK backend before Tauri
    // initialises GTK, even on a Wayland session (XWayland always provides X11). Same approach as
    // the upstream Nuvio Desktop Linux player bridge.
    #[cfg(target_os = "linux")]
    unsafe {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    let selftest = selftest::requested();
    tauri::Builder::default()
        .manage({
            let client = AddonClient::new().expect("HTTP client");
            let store = open_app_store();
            AppState {
                api: Mutex::new(None),
                player: Mutex::new(None),
                session: AtomicU64::new(0),
                hwnd: Mutex::new(0),
                addons: services::addons::AddonContext::new(client.clone(), store.dir().to_path_buf()),
                plugins: services::plugins::Manager::new(store.dir().to_path_buf(), client.clone()),
                client,
                streams: streams::StreamCache::default(),
                downloads: services::downloads::Manager::new(),
                video: Mutex::new(VideoConfig::default()),
                progress: services::progress::ProgressTracker::new(store.dir().to_path_buf()),
                pending_addon: Mutex::new(None),
                account: Arc::new(nuvio_sync::Engine::new(
                    store.dir().to_path_buf(),
                    account::server_config(),
                    // Account the original app's data belongs to (read-only), for the first link.
                    services::addons::legacy_store().ok().and_then(|l| l.profiles().ok().flatten()).map(|p| p.user_id).filter(|u| !u.is_empty()),
                )),
                sync_wake: Arc::new(tokio::sync::Notify::new()),
                sync_signals: Default::default(),
                pip: Default::default(),
                relay: Mutex::new(None),
                pin_gate: Mutex::new(account::PinGate {
                    locked_active: store.profiles().ok().and_then(|p| p.get(p.active).filter(|x| x.pin_enabled).map(|x| x.index)),
                    ..Default::default()
                }),
                app_store: store,
                p2p: Arc::new(nuvio_p2p::P2pService::default()),
                p2p_prepared: Mutex::new(None),
                p2p_session: AtomicU64::new(0),
                youtube: nuvio_core::youtube::Extractor::new().expect("HTTP client"),
                updates: Default::default(),
                epg: Default::default(),
                playing: Mutex::new(None),
                subtitle_choice: Mutex::new(None),
                next_up: Default::default(),
            }
        })
        // Official Tauri 2 updater: public key from tauri.conf.json, endpoint set per build.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // `nuvio://auth/...` callbacks from Trakt/Simkl sign-in. A second instance forwards the
        // URL to the running one instead of opening another window.
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            use tauri::Emitter;
            let urls: Vec<String> = args.into_iter().skip(1).filter(|arg| arg.starts_with("nuvio://")).collect();
            if !urls.is_empty() {
                let _ = app.emit("deep-link://new-url", urls);
            }
        }))
        .invoke_handler(tauri::generate_handler![
            player_open,
            streams_search,
            stream_play,
            player_command,
            player_video_rect,
            player_snapshot,
            player_close,
            player_video_config,
            account::app_ready,
            account::welcome_needed,
            account::welcome_done,
            account::account_connect,
            account::profile_pin_refresh,
            watching::player_subtitles,
            watching::player_subtitle_load,
            watching::player_subtitle_autosync,
            watching::player_subtitle_style,
            watching::player_moments,
            api::library_next_up,
            pip::player_picture_in_picture,
            pip::pip_ready,
            pip::pip_drag,
            api::subtitle_fonts,
            api::subtitle_font_import,
            api::subtitle_font_remove,
            api::subtitle_font_data,
            window_set_fullscreen,
            api::app_bootstrap,
            api::home_rows,
            api::catalog_page,
            api::catalog_genres,
            api::stream_badge_rules,
            api::streams_settings,
            api::streams_settings_set,
            api::debrid_settings,
            api::debrid_settings_set,
            api::plugins_list,
            api::plugins_add,
            api::plugins_refresh,
            api::plugins_set_enabled,
            api::plugins_set_scraper_enabled,
            api::plugins_remove,
            api::mdblist_settings,
            api::mdblist_settings_set,
            api::more_like_this,
            api::tmdb_settings,
            api::tmdb_settings_set,
            api::tmdb_source_page,
            api::trakt_source_page,
            api::trakt_search_lists,
            api::trakt_trending_lists,
            api::trakt_popular_lists,
            api::trakt_resolve_list_input,
            api::trakt_my_lists,
            api::trakt_comments,
            api::discover_sources,
            api::discover_select,
            api::discover_page,
            api::search_all,
            api::search_stream,
            api::search_addons,
            api::search_addon_set,
            api::discover_catalogs,
            api::meta_details,
            api::livetv_channels,
            api::livetv_matches,
            api::livetv_import_m3u,
            api::livetv_fallback_channels,
            api::livetv_favorites,
            api::livetv_toggle_favorite,
            api::title_streams,
            api::livetv_resolve,
            api::livetv_epg,
            api::livetv_reload,
            api::settings_get,
            api::settings_set,
            api::library_favorite_keys,
            api::library_favorites,
            api::library_toggle_favorite,
            api::library_remove_favorite,
            api::library_continue_watching,
            api::library_history,
            api::library_remove_history,
            api::library_title_progress,
            api::library_toggle_watched,
            api::stream_open_external,
            api::open_url,
            api::parental_guide,
            api::discord_activity,
            api::shuffle_view,
            api::download_list,
            api::collections_editor_load,
            api::server_list,
            api::tracking_credentials,
            api::trakt_device_start,
            api::trakt_device_poll,
            api::simkl_authorize_url,
            api::simkl_complete,
            api::tracking_disconnect,
            api::tracking_resume,
            api::mdblist_device_start,
            api::mdblist_device_poll,
            api::tracking_sync,
            api::tracking_credential_set,
            api::server_add,
            api::server_remove,
            api::server_libraries,
            api::server_catalog,
            api::server_plex_pin_start,
            api::server_plex_pin_poll,
            api::server_plex_connect,
            api::server_silo_discover,
            api::server_silo_login,
            api::server_silo_code_start,
            api::server_silo_code_connect,
            api::server_stream,
            api::party_config,
            api::skip_credentials,
            api::skip_credentials_set,
            api::skip_submit_timing,
            api::collections_editor_save,
            api::download_from_stream,
            api::download_control,
            api::download_play,
            api::shuffle_set,
            api::shuffle_pick,
            manage::addons_list,
            manage::addon_preview,
            manage::addon_install,
            manage::addon_set_enabled,
            manage::addon_remove,
            manage::addon_move,
            manage::addon_refresh,
            manage::addon_configure,
            manage::profiles_list,
            manage::profile_switch,
            manage::profile_create,
            manage::profile_update,
            manage::profile_delete,
            account::account_status,
            account::account_sign_in,
            account::account_sign_up,
            account::account_sign_out,
            account::account_sync_now, account::account_sync_preview,
            account::avatars_catalog,
            account::profile_unlock,
            account::profile_pin_set,
            account::profile_pin_clear,
            trailer::trailer_play,
            trailer::trailer_resolve,
            updater::update_status,
            updater::update_check,
            updater::update_install,
            p2p::torrent_prepare,
            p2p::torrent_stop,
            p2p::p2p_status,
            p2p::p2p_clear_cache,
            p2p::p2p_cache_size,
            app_mode,
            platform,
            window_ready,
            player_focus,
            uitest::uitest_capture,
            uitest::uitest_report,
            uitest::uitest_phase,
            uitest::uitest_player_property,
            uitest::uitest_store_counts,
            uitest::uitest_process_resources,
            uitest::uitest_native_libraries,
            uitest::uitest_seed_collections,
            uitest::uitest_break_display_sync
        ])
        .setup(move |app| {
            // On Linux/Windows a deep link starts a new instance with the URL as its only argument.
            app.state::<tauri_plugin_deep_link::DeepLink<tauri::Wry>>()
                .handle_cli_arguments(std::env::args());
            let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Nuvio Astra")
                .inner_size(1280.0, 760.0)
                .min_inner_size(800.0, 480.0);
            // Black, never white, in the 1px seams a borderless transition can leave.
            #[cfg(windows)]
            {
                builder = builder.background_color(tauri::window::Color(0, 0, 0, 255));
            }
            // Transparent so the mpv child window below the webview is visible through the UI.
            #[cfg(target_os = "linux")]
            {
                builder = builder.transparent(true);
            }
            // Normal use: created hidden and shown maximized by `window_ready` once the interface
            // has painted (no white/black frame while the webview starts). The automated tests
            // keep the visible fixed-size window they measure.
            let window = builder
                .visible(selftest.is_some() || uitest::requested().is_some())
                .build()?;
            #[cfg(target_os = "linux")]
            {
                // WebKitGTK must not paint an opaque background over the embedded video.
                let _ = window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)));
            }
            if selftest.is_none() && uitest::requested().is_none() {
                // Safety net: show the window even if the interface never reports.
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    let h = handle.clone();
                    let _ = handle.run_on_main_thread(move || {
                        if let Some(w) = h.get_webview_window("main") {
                            show_main_window(&w);
                        }
                    });
                });
            }
            make_webview_transparent(&window)?;
            let handle = app.handle().clone();
            let signals = app.state::<AppState>().sync_signals.clone();
            app.state::<AppState>().progress.set_notifier(move |_flush| {
                let _ = handle.emit("library-changed", ());
                // Every saved position goes to the account, like the original.
                signals.progress.notify_one();
            });
            if selftest.is_none() && uitest::requested().is_none() {
                account::start_loop(app.handle().clone());
            }
            #[cfg(windows)]
            {
                let hwnd = window.hwnd()?.0 as isize;
                *app.state::<AppState>().hwnd.lock().expect("hwnd lock") = hwnd;
            }
            if uitest::requested().is_some() {
                // Screen captures need the window in front.
                let _ = window.set_always_on_top(true);
            }
            if let Some(request) = selftest.clone() {
                selftest::start(app.handle().clone(), request);
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // The video container is resized by the fullscreen command itself, after the window
            // transition completes. Doing it here (inside WM_SIZE) blocks the UI thread.
            tauri::WindowEvent::Resized(_) => {}
            // Back to the app: what other devices changed is fetched.
            tauri::WindowEvent::Focused(true) => {
                let app = window.app_handle().clone();
                app.state::<AppState>().sync_signals.focus.notify_one();
                // Coming back from Nuvio: its hero choice shows at once.
                tauri::async_runtime::spawn_blocking(move || account::follow_nuvio(&app));
            }
            // Alt+F4 on PiP: back to the main window (the container must leave it first).
            tauri::WindowEvent::CloseRequested { api, .. } if window.label() == pip::LABEL => {
                api.prevent_close();
                let app = window.app_handle();
                let state = app.state::<AppState>();
                if pip::set(app, &state, false, "").is_err() {
                    pip::close_window(&state);
                }
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if window.label() == "main" && uitest::requested().is_none() && account::send_before_close(window) {
                    api.prevent_close();
                }
            }
            // Release the player (mpv threads, windows, GPU resources) before the app exits.
            // The P2P session (peers, listening socket, engine threads) ends with the app.
            tauri::WindowEvent::Destroyed if window.label() == "main" => {
                let state = window.app_handle().state::<AppState>();
                state.close();
                state.p2p.stop();
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running Nuvio Astra");
}
