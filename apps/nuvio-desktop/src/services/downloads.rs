//! Direct HTTP downloads (the original's `DownloadsRepository` + `DownloadsPlatformDownloader`).
//! Stream URLs and request headers stay in Rust; the UI only receives an item with a filename and
//! progress. Files live in `<Astra data dir>/downloads`, with a `.part` file while downloading.

use nuvio_core::downloads::{DownloadItem, DownloadStatus, build_file_name, next_download_id};
use nuvio_store::nextgen::AppStore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct Manager {
    cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    headers: Mutex<HashMap<String, Vec<(String, String)>>>,
}

impl Manager {
    pub fn new() -> Self { Self::default() }

    pub fn set_headers(&self, id: &str, headers: Vec<(String, String)>) {
        self.headers.lock().expect("download headers").insert(id.to_string(), headers);
    }

    fn headers(&self, id: &str) -> Vec<(String, String)> {
        self.headers.lock().expect("download headers").get(id).cloned().unwrap_or_default()
    }

    pub fn cancel(&self, id: &str) { self.stop_flag(id).store(true, Ordering::Relaxed); }

    /// A transfer interrupted by an app restart has no live flag and is restored as Paused.
    pub fn is_active(&self, id: &str) -> bool {
        self.cancel.lock().expect("download flags").contains_key(id)
    }

    fn stop_flag(&self, id: &str) -> Arc<AtomicBool> {
        let mut map = self.cancel.lock().expect("download flags");
        map.entry(id.to_string()).or_default().clone()
    }

    fn forget(&self, id: &str) {
        self.cancel.lock().expect("download flags").remove(id);
        self.headers.lock().expect("download headers").remove(id);
    }

    /// Starts (or restarts) the transfer. The item is already stored with status Downloading.
    pub fn start(&self, store: AppStore, profile: i64, id: String, url: String, dir: PathBuf) {
        let flag = self.stop_flag(&id);
        flag.store(false, Ordering::Relaxed);
        let headers = self.headers(&id);
        tauri::async_runtime::spawn(run(store, profile, id, url, headers, dir, flag));
    }
}

fn part_path(dir: &Path, file_name: &str) -> PathBuf { dir.join(format!("{file_name}.part")) }
fn final_path(dir: &Path, file_name: &str) -> PathBuf { dir.join(file_name) }

/// Deletes both the finished file and its partial file (`DownloadsRepository.replace`).
pub fn remove_files(dir: &Path, file_name: &str) {
    for path in [part_path(dir, file_name), final_path(dir, file_name)] {
        let _ = std::fs::remove_file(path);
    }
}

fn parse_total(headers: &reqwest::header::HeaderMap) -> Option<i64> {
    headers
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse().ok())
        .or_else(|| headers.get(reqwest::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()))
}

async fn run(store: AppStore, profile: i64, id: String, url: String, headers: Vec<(String, String)>, dir: PathBuf, flag: Arc<AtomicBool>) {
    let _ = std::fs::create_dir_all(&dir);
    let file_name = store
        .downloads(profile)
        .ok()
        .and_then(|items| items.into_iter().find(|item| item.id == id).map(|item| item.file_name))
        .unwrap_or_else(|| "download.mp4".into());
    let part = part_path(&dir, &file_name);
    let finished = final_path(&dir, &file_name);
    if finished.exists() {
        store
            .edit_downloads(profile, |items| {
                if let Some(item) = items.iter_mut().find(|item| item.id == id) {
                    item.status = DownloadStatus::Completed;
                    item.local_file_uri = Some(finished.to_string_lossy().to_string());
                }
            })
            .ok();
        return;
    }
    let mut offset = std::fs::metadata(&part).map(|m| m.len() as i64).unwrap_or(0);
    let client = reqwest::Client::builder().build();
    let client = match client {
        Ok(client) => client,
        Err(e) => { mark_failed(&store, profile, &id, e.to_string()); return; }
    };
    let mut request = client.get(&url);
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("range") || name.eq_ignore_ascii_case("accept-encoding") { continue; }
        request = request.header(&name, &value);
    }
    if offset > 0 { request = request.header(reqwest::header::RANGE, format!("bytes={offset}-")); }
    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => { mark_failed(&store, profile, &id, e.to_string()); return; }
    };
    if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let _ = std::fs::remove_file(&part);
        offset = 0;
    } else if !response.status().is_success() {
        mark_failed(&store, profile, &id, format!("HTTP {}", response.status().as_u16()));
        return;
    }
    let total = parse_total(response.headers()).or_else(|| {
        // A 206 response carries only the remaining length in content-length.
        response.headers().get(reqwest::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<i64>().ok()).map(|n| n + offset)
    });
    let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut file = match std::fs::OpenOptions::new().create(true).write(true).open(&part) {
        Ok(file) => file,
        Err(e) => { mark_failed(&store, profile, &id, e.to_string()); return; }
    };
    use std::io::{Seek, Write};
    if !resumed || offset == 0 {
        if let Err(e) = file.set_len(0) { mark_failed(&store, profile, &id, e.to_string()); return; }
        offset = 0;
    } else if let Err(e) = file.seek(std::io::SeekFrom::Start(offset as u64)) {
        mark_failed(&store, profile, &id, e.to_string());
        return;
    }
    let mut written = offset;
    let mut response = response;
    let mut last_save = Instant::now();
    loop {
        if flag.load(Ordering::Relaxed) {
            set_status(&store, profile, &id, DownloadStatus::Paused, written, total, None);
            return;
        }
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(e) => { mark_failed(&store, profile, &id, e.to_string()); return; }
        };
        if let Err(e) = file.write_all(&chunk).and_then(|_| file.flush()) {
            mark_failed(&store, profile, &id, e.to_string());
            return;
        }
        written += chunk.len() as i64;
        if last_save.elapsed() >= Duration::from_secs(1) {
            set_status(&store, profile, &id, DownloadStatus::Downloading, written, total, None);
            last_save = Instant::now();
        }
    }
    if let Some(expected) = total {
        if written < expected && written > 0 {
            // Keep the partial file and retry from where we stopped on the next resume.
            set_status(&store, profile, &id, DownloadStatus::Paused, written, total, Some("Conexão interrompida".into()));
            return;
        }
    }
    drop(file);
    let _ = std::fs::remove_file(&finished);
    if let Err(e) = std::fs::rename(&part, &finished) {
        mark_failed(&store, profile, &id, e.to_string());
        return;
    }
    store
        .edit_downloads(profile, |items| {
            if let Some(item) = items.iter_mut().find(|item| item.id == id) {
                item.status = DownloadStatus::Completed;
                item.downloaded_bytes = written;
                item.total_bytes = total.or(Some(written));
                item.local_file_uri = Some(finished.to_string_lossy().to_string());
                item.error_message = None;
                item.updated_at_epoch_ms = now_ms();
            }
        })
        .ok();
}

fn now_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64 }

fn set_status(store: &AppStore, profile: i64, id: &str, status: DownloadStatus, bytes: i64, total: Option<i64>, error: Option<String>) {
    store
        .edit_downloads(profile, |items| {
            if let Some(item) = items.iter_mut().find(|item| item.id == id) {
                item.status = status;
                item.downloaded_bytes = bytes;
                item.total_bytes = total;
                item.error_message = error;
                item.updated_at_epoch_ms = now_ms();
            }
        })
        .ok();
}

fn mark_failed(store: &AppStore, profile: i64, id: &str, message: String) {
    set_status(store, profile, id, DownloadStatus::Failed, 0, None, Some(message));
}

/// Builds a new item from a stream key (URL/headers never leave Rust) with the original's rules.
pub fn item_from_stream(
    profile: i64,
    stream: &nuvio_core::stream::StreamItem,
    url: &str,
    progress: &nuvio_store::library::PlaybackItem,
    existing: &[DownloadItem],
) -> DownloadItem {
    let now = now_ms();
    let ordinal = existing.len() as u64 + 1;
    let id = next_download_id(now, ordinal);
    let title = if progress.season_number.is_some() && progress.episode_number.is_some() {
        format!("{} - {}", progress.title, progress.episode_title.clone().unwrap_or_default())
    } else {
        progress.title.clone()
    };
    DownloadItem {
        id: id.clone(),
        content_type: progress.content_type.clone(),
        parent_meta_id: progress.parent_meta_id.clone(),
        parent_meta_type: progress.parent_meta_type.clone(),
        video_id: progress.video_id.clone(),
        title: title.trim_end_matches(" - ").to_string(),
        season_number: progress.season_number.map(i64::from),
        episode_number: progress.episode_number.map(i64::from),
        episode_title: progress.episode_title.clone(),
        stream_title: stream.name.clone().or_else(|| stream.title.clone()).unwrap_or_default(),
        stream_subtitle: stream.description.clone(),
        provider_name: stream.addon_name.clone(),
        source_url: url.to_string(),
        file_name: build_file_name(&title, progress.season_number.map(i64::from), progress.episode_number.map(i64::from), progress.episode_title.as_deref(), &id, url),
        local_file_uri: None,
        status: DownloadStatus::Downloading,
        downloaded_bytes: 0,
        total_bytes: None,
        error_message: None,
        created_at_epoch_ms: now,
        updated_at_epoch_ms: now,
    }
}
