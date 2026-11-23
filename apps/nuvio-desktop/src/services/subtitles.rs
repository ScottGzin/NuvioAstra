//! Subtitles offered during playback: the stream's own and the addons' (`SubtitleRepository.kt`).
//!
//! URLs stay in Rust. A chosen subtitle is downloaded here and handed to mpv as a local file: mpv
//! would send the stream's request headers (which may be secrets) to the subtitle's server.

use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_core::subtitle::{parse_subtitles, subtitle_extra, SubtitleItem};
use nuvio_core::url::build_resource_url;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

const ADDON_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BYTES: usize = 10 * 1024 * 1024;
const KEEP_FILES: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone)]
pub struct Found {
    pub source: String,
    pub language: String,
    /// Track title in mpv, unique in the session: how the menu knows it was loaded.
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleOption {
    pub index: usize,
    pub source: String,
    pub language: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleList {
    pub options: Vec<SubtitleOption>,
    /// Addons that were asked.
    pub addons: usize,
    /// Addons that did not answer.
    pub failed: usize,
}

/// What the subtitle addons are told about the file.
#[derive(Debug, Clone, Default)]
pub struct Video {
    pub content_type: String,
    pub id: String,
    pub hash: Option<String>,
    pub size: Option<i64>,
    pub filename: Option<String>,
    pub own: Vec<SubtitleItem>,
}

/// The file name in a URL, only when it is plainly a video file (never a token).
pub fn filename_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
    if !["mkv", "mp4", "avi", "mov", "webm", "m4v", "ts"].contains(&extension.as_str()) {
        return None;
    }
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%').then(|| name.get(i + 1..i + 3)).flatten().and_then(|hex| u8::from_str_radix(hex, 16).ok());
        out.push(escaped.unwrap_or(bytes[i]));
        i += if escaped.is_some() { 3 } else { 1 };
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

pub async fn search(client: &AddonClient, manifests: &[AddonManifest], video: &Video) -> (Vec<Found>, SubtitleList) {
    let lowered = video.content_type.trim().to_lowercase();
    let content_type = if lowered == "tv" { "series".to_string() } else { lowered };
    let extra = subtitle_extra(video.hash.as_deref(), video.size, video.filename.as_deref());
    let mut requests = Vec::new();
    for manifest in manifests {
        let resource = ["subtitles", "subtitle"].into_iter().find(|r| manifest.supports_resource(r, &content_type, &video.id));
        if resource.is_none() {
            continue;
        }
        let url = build_resource_url(&manifest.transport_url, "subtitles", &content_type, &video.id, extra.as_deref());
        let (client, name, addon_id) = (client.clone(), manifest.name.clone(), manifest.id.clone());
        requests.push(tauri::async_runtime::spawn(async move {
            let items = match tokio::time::timeout(ADDON_TIMEOUT, client.get_text(&url, false)).await {
                Ok(Ok(payload)) => parse_subtitles(&payload, &addon_id).ok(),
                _ => None,
            };
            (name, items)
        }));
    }

    let addons = requests.len();
    let mut sources: Vec<(String, Vec<SubtitleItem>)> = vec![("Fonte".to_string(), video.own.clone())];
    let mut failed = 0;
    for request in requests {
        match request.await {
            Ok((name, Some(items))) => sources.push((name, items)),
            _ => failed += 1,
        }
    }
    let mut seen = HashSet::new();
    let mut found: Vec<Found> = Vec::new();
    for (source, items) in sources {
        for item in items {
            if seen.insert((item.id, item.url.clone())) {
                let title = format!("{source} · {}", found.len() + 1);
                found.push(Found { source: source.clone(), language: item.language, title, url: item.url });
            }
        }
    }
    let options = found
        .iter()
        .enumerate()
        .map(|(index, f)| SubtitleOption {
            index,
            source: f.source.clone(),
            language: super::playback::normalize_language(&f.language).unwrap_or_else(|| "unknown".into()),
            title: f.title.clone(),
        })
        .collect();
    (found, SubtitleList { options, addons, failed })
}

fn extension_of(url: &str, bytes: &[u8]) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    if let Some(known) = ["srt", "vtt", "ass", "ssa"].into_iter().find(|ext| path.ends_with(&format!(".{ext}"))) {
        return known;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("WEBVTT") {
        "vtt"
    } else if head.starts_with("[Script Info]") {
        "ass"
    } else {
        "srt"
    }
}

/// Downloads a subtitle to this app's temporary folder and returns the file.
pub async fn download(client: &AddonClient, url: &str, name: &str) -> Result<PathBuf, String> {
    let bytes = tokio::time::timeout(DOWNLOAD_TIMEOUT, client.get_bytes(url, MAX_BYTES))
        .await
        .map_err(|_| "a legenda demorou demais para responder".to_string())?
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("a legenda veio vazia".into());
    }
    let dir = std::env::temp_dir().join("nuvio-astra-subtitles");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Only this folder's own files, and only the old ones.
    for old in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let aged = old.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > KEEP_FILES);
        if aged {
            let _ = std::fs::remove_file(old.path());
        }
    }
    let file = dir.join(format!("{}-{name}.{}", std::process::id(), extension_of(url, &bytes)));
    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_and_formats() {
        assert_eq!(filename_from_url("https://h.test/d/My%20Show%20E01.mkv?token=secret").as_deref(), Some("My Show E01.mkv"));
        assert_eq!(filename_from_url("https://h.test/play/abc123secret"), None);
        assert_eq!(extension_of("https://h.test/sub/1", b"\xEF\xBB\xBFWEBVTT\n"), "vtt");
        assert_eq!(extension_of("https://h.test/a.ASS?x", b""), "ass");
    }
}
