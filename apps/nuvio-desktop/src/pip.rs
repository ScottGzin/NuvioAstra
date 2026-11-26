//! Picture-in-picture (`DesktopPlayerPictureInPicture` of the original): an undecorated,
//! always-on-top window with a transparent webview (`web/src/player/PipView.tsx`: timeline, play,
//! subtitles, audio, volume, time, lock, return). The player's video container moves into it, below
//! the webview, exactly like in the main window.
//!
//! The window is destroyed only after the container went back (its child windows would go with
//! it), and it never outlives the player session or the main window.

use crate::AppState;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "pip";
/// Shown by `pip_ready`; this is the fallback when the page never reports.
const SHOW_FALLBACK: std::time::Duration = std::time::Duration::from_secs(3);

/// Main thread, and never inside a synchronous command (creating a webview there deadlocks).
pub fn set(app: &AppHandle, state: &AppState, enabled: bool, title: &str) -> Result<(), String> {
    if !enabled {
        state.with_player(|p| p.exit_picture_in_picture().map_err(|e| e.to_string()))?;
        close_window(state);
        let _ = app.emit("player-pip", false);
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.unminimize();
            let _ = main.set_focus();
        }
        return Ok(());
    }
    if state.with_player(|p| Ok(p.is_picture_in_picture()))? {
        return Ok(());
    }
    // A window left by a session that ended has no container in it.
    close_window(state);
    #[cfg(not(windows))]
    {
        // The Linux backend creates its own X11 PiP window; no Tauri webview needed here.
        state.with_player(|p| p.enter_picture_in_picture(0).map_err(|e| e.to_string()))?;
        let _ = app.emit("player-pip", true);
        return Ok(());
    }
#[cfg(windows)]
    {
        let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
            .initialization_script("window.__NUVIO_PIP__ = true;")
            .title(title)
            .decorations(false)
            // Without it an undecorated window keeps a 1 px light line on top.
            .shadow(false)
            .resizable(true)
            .maximizable(false)
            .always_on_top(true)
            .visible(false)
            .inner_size(480.0, 270.0)
            .build()
            .map_err(|e| format!("janela do picture-in-picture: {e}"))?;
        let entered = crate::make_webview_transparent(&window)
            .and_then(|()| window.hwnd())
            .map_err(|e| e.to_string())
            .and_then(|hwnd| state.with_player(|p| p.enter_picture_in_picture(hwnd.0 as isize).map_err(|e| e.to_string())));
        if let Err(error) = entered {
            let _ = window.destroy();
            return Err(error);
        }
        *state.pip.lock().expect("pip window lock") = Some(window.clone());
        let _ = app.emit("player-pip", true);
        std::thread::spawn(move || {
            std::thread::sleep(SHOW_FALLBACK);
            if !window.is_visible().unwrap_or(true) {
                let _ = window.show();
            }
        });
    }
    Ok(())
}

/// After the container left it.
pub fn close_window(state: &AppState) {
    if let Some(window) = state.pip.lock().expect("pip window lock").take() {
        let _ = window.emit("player-pip", false);
        let _ = window.destroy();
    }
}

#[tauri::command]
pub async fn player_picture_in_picture(app: AppHandle, enabled: bool, title: Option<String>) -> Result<(), String> {
    crate::selftest::on_main(&app, move |app| set(app, &app.state::<AppState>(), enabled, title.as_deref().unwrap_or("Nuvio Astra")))
}

/// The controls painted: shown now, so no empty frame is seen.
#[tauri::command]
pub fn pip_ready(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != LABEL {
        return Ok(());
    }
    window.show().and_then(|()| window.set_focus()).map_err(|e| e.to_string())
}

/// Dragging anywhere outside the controls moves the window.
#[tauri::command]
pub fn pip_drag(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != LABEL {
        return Ok(());
    }
    window.start_dragging().map_err(|e| e.to_string())
}
