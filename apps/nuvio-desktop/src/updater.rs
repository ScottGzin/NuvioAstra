//! App updates through the official Tauri 2 updater (`tauri-plugin-updater`).
//!
//! - Endpoint: the `latest.json` of the GitHub repository the build was made for
//!   (`NUVIO_UPDATE_REPOSITORY=owner/repo`, set by the release workflow or `local.properties`);
//!   a build without it has updates off and says so.
//! - Public key: `plugins.updater.pubkey` in `tauri.conf.json` (the private key only exists in the
//!   repository's Actions secrets). Every downloaded installer is verified against it by the plugin
//!   before anything runs; a bad signature stops the update.
//! - Install: the plugin starts the signed NSIS installer (`/P /UPDATE /R`), which installs over the
//!   current installation folder and reopens the app. Data in `%APPDATA%\NuvioNextGen` (profiles,
//!   addons, settings, library, account session) is never touched by the installer.
//! - The player and the P2P session are closed first (watch progress saved).

use crate::AppState;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// `owner/repo` of the GitHub repository whose releases this build follows.
pub fn repository() -> Option<&'static str> {
    option_env!("NUVIO_UPDATE_REPOSITORY").map(str::trim).filter(|r| r.contains('/') && !r.contains(' '))
}

fn endpoint() -> Option<String> {
    repository().map(|r| format!("https://github.com/{r}/releases/latest/download/latest.json"))
}

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<Update>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub configured: bool,
    pub current_version: String,
    pub repository: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    /// `downloading`, `verifying`, `installing`, `error`.
    pub phase: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
}

fn pt_error(e: tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) => "The update signature does not match: installation was canceled for security.".into(),
        E::Network(m) => format!("No connection to the update server ({m})."),
        E::Reqwest(e) => format!("No connection to the update server ({e})."),
        E::ReleaseNotFound => "No version has been published yet.".into(),
        other => format!("Could not update: {other}"),
    }
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateStatus {
    UpdateStatus {
        configured: endpoint().is_some(),
        current_version: app.package_info().version.to_string(),
        repository: repository().map(str::to_string),
    }
}

/// Asks the release endpoint whether a newer version exists (nothing is downloaded).
#[tauri::command]
pub async fn update_check(app: AppHandle, state: State<'_, AppState>) -> Result<Option<UpdateInfo>, String> {
    let url = endpoint().ok_or("Automatic updates are not configured in this build.")?;
    let handle = app.clone();
    let updater = app
        .updater_builder()
        .endpoints(vec![url.parse().map_err(|e| format!("{e}"))?])
        .map_err(pt_error)?
        .timeout(std::time::Duration::from_secs(30))
        .on_before_exit(move || {
            // Last position saved, player and P2P engine released before the installer runs.
            let state = handle.state::<AppState>();
            state.close();
            state.p2p.stop();
        })
        .build()
        .map_err(pt_error)?;
    let update = updater.check().await.map_err(pt_error)?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone().filter(|b| !b.trim().is_empty()),
        date: u.date.map(|d| d.to_string()),
    });
    *state.updates.pending.lock().expect("update lock") = update;
    Ok(info)
}

/// Downloads the update found by `update_check` (progress in `update-progress` events), verifies
/// its signature and runs the installer; the app then closes and the installer reopens it.
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let update = state.updates.pending.lock().expect("update lock").clone().ok_or("No pending update: check again.")?;
    let progress = |phase: &str, downloaded: u64, total: Option<u64>, error: Option<String>| {
        let _ = app.emit("update-progress", UpdateProgress { phase: phase.into(), downloaded, total, error });
    };
    let mut downloaded = 0u64;
    let mut last_emit = std::time::Instant::now();
    let result = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                if last_emit.elapsed().as_millis() >= 150 || total.is_some_and(|t| downloaded >= t) {
                    last_emit = std::time::Instant::now();
                    progress("downloading", downloaded, total, None);
                }
            },
            || {},
        )
        .await;
    let bytes = match result {
        Ok(b) => b,
        Err(e) => {
            let text = pt_error(e);
            progress("error", 0, None, Some(text.clone()));
            return Err(text);
        }
    };
    // `download` already verified the signature; `install` verifies again before running it.
    progress("verifying", bytes.len() as u64, Some(bytes.len() as u64), None);
    progress("installing", bytes.len() as u64, Some(bytes.len() as u64), None);
    // On success the plugin exits the process right after starting the installer.
    update.install(&bytes).map_err(|e| {
        let text = pt_error(e);
        progress("error", 0, None, Some(text.clone()));
        text
    })
}

#[cfg(test)]
mod tests {
    use tauri_plugin_updater::RemoteRelease;

    /// `latest.json` exactly as the release workflow writes it (PowerShell `ConvertTo-Json` spacing,
    /// accents in the notes) is read by the official updater: version, notes, date, URL, signature.
    #[test]
    fn release_manifest_is_understood_by_the_updater() {
        let manifest = r#"{
    "version":  "0.1.1",
    "notes":  "- Primeira versão pública\n- Correção",
    "pub_date":  "2026-11-20T18:48:18Z",
    "platforms":  {
                      "windows-x86_64":  {
                                             "signature":  "c2ln",
                                             "url":  "https://github.com/dono/Nuvio-Astra/releases/download/v0.1.1/Nuvio-Astra-0.1.1-x64-setup.exe"
                                         }
                  }
}"#;
        let r: RemoteRelease = serde_json::from_str(manifest).expect("manifest parsed");
        assert_eq!(r.version.to_string(), "0.1.1");
        assert!(r.notes.as_deref().unwrap().contains("versão pública"));
        assert!(r.pub_date.is_some());
        assert!(r.download_url("windows-x86_64").unwrap().as_str().ends_with("/v0.1.1/Nuvio-Astra-0.1.1-x64-setup.exe"));
        assert_eq!(r.signature("windows-x86_64").unwrap(), "c2ln");
    }

    #[test]
    fn repository_and_endpoint() {
        // Built without NUVIO_UPDATE_REPOSITORY in tests: updates off, nothing contacted.
        if super::repository().is_none() {
            assert!(super::endpoint().is_none());
        } else {
            assert!(super::endpoint().unwrap().ends_with("/releases/latest/download/latest.json"));
        }
    }
}
