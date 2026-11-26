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
//! - Linux: the same signed `latest.json` is used (platform key `linux-x86_64`; the bundles built by
//!   `packaging/linux/` are not bundle-patched, so format-specific keys are not consulted). In
//!   addition, `update_check` does a read-only `GET` to GitHub Releases to find `.AppImage` / `.deb`
//!   assets of the latest release: their
//!   URLs are appended to the release notes shown in the existing update card, and if the signed
//!   manifest has no Linux entry the check reports the direct download links instead of claiming
//!   the app is up to date. These links are never downloaded or installed by the app itself.

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

/// Read-only discovery of the Linux artifacts attached to the newest GitHub release.
///
/// Deliberately no new Tauri command / IPC type is needed: the URLs are appended to the release
/// notes that the existing update card already renders as text. Nothing here installs anything.
#[cfg(target_os = "linux")]
mod linux_releases {
    use super::repository;

    #[derive(Debug, Clone)]
    pub struct Asset {
        pub name: String,
        pub url: String,
    }

    pub struct Latest {
        pub tag: String,
        pub assets: Vec<Asset>,
    }

    fn arch_names() -> &'static [&'static str] {
        #[cfg(target_arch = "aarch64")]
        {
            &["aarch64", "arm64"]
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            &["x86_64", "amd64", "x64"]
        }
    }

    fn tag(release: &serde_json::Value) -> String {
        release.get("tag_name").and_then(|t| t.as_str()).unwrap_or("").trim().trim_start_matches('v').to_string()
    }

    /// Keeps only https assets for this architecture whose file name looks like a Linux package.
    fn select_assets(release: &serde_json::Value) -> Vec<Asset> {
        let Some(list) = release.get("assets").and_then(|a| a.as_array()) else { return Vec::new() };
        let mut out = Vec::new();
        for asset in list {
            let (Some(name), Some(url)) = (
                asset.get("name").and_then(|n| n.as_str()),
                asset.get("browser_download_url").and_then(|u| u.as_str()),
            ) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            let linux_package = lower.ends_with(".appimage") || lower.ends_with(".deb");
            if !linux_package || !arch_names().iter().any(|a| lower.contains(a)) || !url.starts_with("https://") {
                continue;
            }
            out.push(Asset { name: name.to_string(), url: url.to_string() });
        }
        out
    }

    /// `GET /repos/{owner}/{repo}/releases/latest` — no token, no writes, no side effects.
    pub async fn fetch() -> Result<Latest, String> {
        let repo = repository().ok_or_else(|| "Atualizações automáticas não estão configuradas nesta compilação.".to_string())?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("nuvio-astra/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("Sem conexão com o GitHub ({e})."))?;
        let release: serde_json::Value = client
            .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Sem conexão com o servidor de versões do GitHub ({e})."))?
            .json()
            .await
            .map_err(|e| format!("Não foi possível ler a versão publicada no GitHub ({e})."))?;
        Ok(Latest { tag: tag(&release), assets: select_assets(&release) })
    }

    pub fn download_lines(latest: &Latest) -> Option<String> {
        if latest.assets.is_empty() {
            return None;
        }
        let mut text = String::from("Linux — downloads diretos (use se a atualização automática não estiver disponível):");
        for asset in &latest.assets {
            text.push_str("\n- ");
            text.push_str(&asset.name);
            text.push_str(": ");
            text.push_str(&asset.url);
        }
        Some(text)
    }

    pub fn no_signed_update_error(latest: &Latest, cause: &str) -> String {
        let mut text = if latest.tag.is_empty() {
            "Há uma versão publicada no GitHub, mas este pacote não tem uma atualização assinada para Linux.".to_string()
        } else {
            format!("Há uma versão nova no GitHub ({}), mas este pacote não tem uma atualização assinada para Linux.", latest.tag)
        };
        if let Some(lines) = download_lines(latest) {
            text.push_str("\n\n");
            text.push_str(&lines);
        }
        text.push_str("\n\n");
        text.push_str(cause);
        text
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn selects_only_matching_linux_assets() {
            let release: serde_json::Value = serde_json::from_str(
                r#"{"tag_name":"v0.3.0","assets":[
                    {"name":"Nuvio-Astra-0.3.0-x86_64.AppImage","browser_download_url":"https://example.test/a.AppImage"},
                    {"name":"nuvio-astra_0.3.0_amd64.deb","browser_download_url":"https://example.test/a.deb"},
                    {"name":"Nuvio-Astra-0.3.0-aarch64.AppImage","browser_download_url":"https://example.test/b.AppImage"},
                    {"name":"Nuvio-Astra-0.3.0-x64-setup.exe","browser_download_url":"https://example.test/a.exe"},
                    {"name":"Nuvio-Astra-0.3.0-x86_64.AppImage","browser_download_url":"http://example.test/insecure.AppImage"}
                ]}"#,
            )
            .expect("release JSON parsed");
            let latest = Latest { tag: tag(&release), assets: select_assets(&release) };
            assert_eq!(latest.tag, "0.3.0");
            let names: Vec<_> = latest.assets.iter().map(|a| a.name.as_str()).collect();
            assert_eq!(names, ["Nuvio-Astra-0.3.0-x86_64.AppImage", "nuvio-astra_0.3.0_amd64.deb"]);
            assert_eq!(download_lines(&latest).unwrap().matches("https://example.test").count(), 2);
        }
    }
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
        E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) => "A assinatura da atualização não confere: a instalação foi cancelada por segurança.".into(),
        E::Network(m) => format!("Sem conexão com o servidor de atualizações ({m})."),
        E::Reqwest(e) => format!("Sem conexão com o servidor de atualizações ({e})."),
        E::ReleaseNotFound => "Nenhuma versão publicada ainda.".into(),
        other => format!("Não foi possível atualizar: {other}"),
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
    let url = endpoint().ok_or("Atualizações automáticas não estão configuradas nesta compilação.")?;
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
    let update = match updater.check().await {
        Ok(update) => update,
        Err(e) => {
            #[cfg(target_os = "linux")]
            {
                use tauri_plugin_updater::Error as UpdaterError;
                // A signed manifest with only Windows entries is the current situation for many
                // builds. Do not tell Linux users they are up to date: show where the Linux
                // artifacts are instead. This is still a read-only check (no install attempt).
                if matches!(e, UpdaterError::TargetsNotFound(_) | UpdaterError::TargetNotFound(_) | UpdaterError::ReleaseNotFound) {
                    if let Ok(latest) = linux_releases::fetch().await {
                        if !latest.assets.is_empty() {
                            return Err(linux_releases::no_signed_update_error(&latest, &e.to_string()));
                        }
                    }
                }
            }
            return Err(pt_error(e));
        }
    };
    #[allow(unused_mut)]
    let mut info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone().filter(|b| !b.trim().is_empty()),
        date: u.date.map(|d| d.to_string()),
    });
    #[cfg(target_os = "linux")]
    {
        if info.is_some() {
            if let Ok(latest) = linux_releases::fetch().await {
                if let Some(lines) = linux_releases::download_lines(&latest) {
                    let found = info.as_mut().expect("update info");
                    found.notes = Some(match found.notes.take() {
                        Some(notes) => format!("{notes}\n\n{lines}"),
                        None => lines,
                    });
                }
            }
        }
    }
    *state.updates.pending.lock().expect("update lock") = update;
    Ok(info)
}

/// Downloads the update found by `update_check` (progress in `update-progress` events), verifies
/// its signature and runs the installer; the app then closes and the installer reopens it.
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let update = state.updates.pending.lock().expect("update lock").clone().ok_or("Nenhuma atualização pendente: verifique de novo.")?;
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
