//! Downloads of any source (HTTP, TorBox or torrent through the P2P engine) into a folder the
//! user picks.
//!
//! Fast and sturdy where the original stalls: the file is split in 8 MiB parts fetched over
//! several connections at once (8 for HTTP/TorBox, 4 for the local P2P route); a part that fails
//! (dropped connection, HTTP 503/429/5xx, timeout) is retried by itself with backoff, from the byte
//! it reached, honoring `Retry-After`. Progress is kept next to the file (`.part` + `.part.json`),
//! so a pause, a crash or closing the app resumes where it stopped. A server without ranges gets
//! one connection, restarted on failure.
//!
//! Stored: the list (`downloads.json`, no address) and each source (`downloads-sources.dat`,
//! encrypted with DPAPI: links may carry tokens).

use crate::{p2p, AppState};
use nuvio_core::torbox::Wanted;
use nuvio_store::library::PlaybackItem;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::os::windows::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

const PART: u64 = 8 << 20;
const HTTP_CONNECTIONS: usize = 8;
const P2P_CONNECTIONS: usize = 4;
const RUNNING_AT_ONCE: usize = 2;
const PART_RETRIES: u32 = 12;
const SINGLE_RETRIES: u32 = 4;
const LIST_DOC: &str = "downloads";
const SOURCES_FILE: &str = "downloads-sources.dat";
const SETTINGS_DOC: &str = "downloads-settings";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Source {
    Http { url: String, headers: Vec<(String, String)> },
    /// A debrid service's link. `service`: which one (absent in downloads saved before the other
    /// services existed: TorBox).
    Torbox {
        magnet: String,
        filename: Option<String>,
        file_idx: Option<i64>,
        season: Option<i64>,
        episode: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        service: Option<String>,
    },
    P2p { info_hash: String, file_index: Option<i32>, filename: Option<String>, trackers: Vec<String> },
}

impl Source {
    fn kind(&self) -> &'static str {
        match self {
            Source::Http { .. } => "http",
            Source::Torbox { .. } => "torbox",
            Source::P2p { .. } => "p2p",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub poster: Option<String>,
    pub folder: String,
    /// Final name; chosen when the transfer starts.
    pub file_name: Option<String>,
    /// `queued`, `preparing`, `downloading`, `paused`, `done`, `failed`.
    pub status: String,
    /// `http`, `torbox`, `p2p`.
    pub kind: String,
    pub source_name: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    #[serde(default)]
    pub speed: u64,
    #[serde(default)]
    pub connections: usize,
    pub error: Option<String>,
    pub created_at: i64,
    pub playback: Option<PlaybackItem>,
}

impl Item {
    pub fn path(&self) -> Option<PathBuf> {
        Some(Path::new(&self.folder).join(self.file_name.as_ref()?))
    }
}

struct Running {
    stop: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct Downloads {
    items: Mutex<Vec<Item>>,
    sources: Mutex<BTreeMap<String, Source>>,
    running: Mutex<BTreeMap<String, Running>>,
    queue: Mutex<VecDeque<String>>,
    loaded: AtomicBool,
    /// A download owns the P2P engine (no torrent can start meanwhile).
    pub p2p_owner: Mutex<Option<String>>,
}

fn now_ms() -> i64 {
    nuvio_store::library::now_ms()
}

impl Downloads {
    fn load(&self, state: &AppState) {
        if self.loaded.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut items: Vec<Item> = state.app_store.read(LIST_DOC).ok().flatten().unwrap_or_default();
        // What was running when the app closed waits for the user.
        for item in items.iter_mut().filter(|i| matches!(i.status.as_str(), "queued" | "preparing" | "downloading")) {
            item.status = "paused".into();
            item.speed = 0;
        }
        *self.items.lock().expect("downloads") = items;
        let sources = std::fs::read(state.app_store.dir().join(SOURCES_FILE))
            .ok()
            .and_then(|c| nuvio_sync::crypto::unprotect(&c).ok())
            .and_then(|p| serde_json::from_slice(&p).ok())
            .unwrap_or_default();
        *self.sources.lock().expect("download sources") = sources;
    }

    fn save(&self, state: &AppState) {
        let items = self.items.lock().expect("downloads").clone();
        let _ = state.app_store.write(LIST_DOC, &items);
        let sources = self.sources.lock().expect("download sources").clone();
        if let Ok(cipher) = serde_json::to_vec(&sources).map_err(|e| e.to_string()).and_then(|p| nuvio_sync::crypto::protect(&p)) {
            let path = state.app_store.dir().join(SOURCES_FILE);
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, cipher).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Item)) -> Option<Item> {
        let mut items = self.items.lock().expect("downloads");
        let item = items.iter_mut().find(|i| i.id == id)?;
        change(item);
        Some(item.clone())
    }

    pub fn list(&self) -> Vec<Item> {
        self.items.lock().expect("downloads").clone()
    }
}

fn emit(app: &AppHandle) {
    let state = app.state::<AppState>();
    let _ = app.emit("downloads", state.downloads.list());
}

// ---------------------------------------------------------------- folder

#[derive(Serialize, Deserialize, Default)]
struct Settings {
    folder: Option<String>,
}

fn default_folder() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Downloads").join("Nuvio Astra")
}

fn folder(state: &AppState) -> PathBuf {
    state
        .app_store
        .read::<Settings>(SETTINGS_DOC)
        .ok()
        .flatten()
        .and_then(|s| s.folder)
        .map(PathBuf::from)
        .unwrap_or_else(default_folder)
}

/// Free bytes on the disk of `path` (its first existing parent).
fn free_space(path: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let existing = path.ancestors().find(|p| p.exists())?;
    let mut free = 0u64;
    // SAFETY: valid string and out pointer.
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(existing.as_os_str()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

/// Windows "choose folder" dialog.
fn choose_folder(owner: isize, start: &Path) -> Option<PathBuf> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH};
    // SAFETY: COM calls on this thread, released in order.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let picked = (|| -> Option<PathBuf> {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let options = dialog.GetOptions().ok()?;
            dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
            let _ = dialog.SetTitle(&HSTRING::from("Pasta dos downloads do Nuvio Astra"));
            if let Some(existing) = start.ancestors().find(|p| p.exists()) {
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(existing.as_os_str()), None) {
                    let _ = dialog.SetFolder(&item);
                }
            }
            dialog.Show(Some(HWND(owner as *mut _))).ok()?;
            let result = dialog.GetResult().ok()?;
            let name = result.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let path = name.to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(name.0 as *const _));
            path.map(PathBuf::from)
        })();
        if initialized {
            CoUninitialize();
        }
        picked
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    pub folder: String,
    pub free_bytes: Option<u64>,
}

#[tauri::command]
pub fn downloads_folder(state: State<'_, AppState>) -> FolderView {
    let folder = folder(&state);
    FolderView { free_bytes: free_space(&folder), folder: folder.display().to_string() }
}

#[tauri::command]
pub async fn downloads_choose_folder<R: tauri::Runtime>(window: tauri::WebviewWindow<R>, state: State<'_, AppState>) -> Result<FolderView, String> {
    let owner = window.hwnd().map(|h| h.0 as isize).unwrap_or_default();
    let start = folder(&state);
    let picked = tauri::async_runtime::spawn_blocking(move || choose_folder(owner, &start)).await.map_err(|e| e.to_string())?;
    if let Some(path) = picked {
        state.app_store.write(SETTINGS_DOC, &Settings { folder: Some(path.display().to_string()) }).map_err(|e| e.to_string())?;
    }
    Ok(downloads_folder(state))
}

// ---------------------------------------------------------------- names

fn clean(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "LPT1"].iter().any(|r| trimmed.split('.').next().unwrap_or_default().eq_ignore_ascii_case(r));
    let safe = if trimmed.is_empty() || reserved { "video".to_string() } else { trimmed };
    // Room for " (99).part.json" under the 255 limit of a name.
    safe.chars().take(180).collect()
}

fn extension_of(name: &str) -> Option<String> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    (["mkv", "mp4", "webm", "avi", "mov", "m4v", "ts", "m2ts", "wmv", "flv"].contains(&ext.as_str())).then_some(ext)
}

fn name_from_url(url: &str) -> Option<String> {
    let last = url.split(['?', '#']).next()?.rsplit('/').next()?;
    let decoded = percent_decode(last);
    extension_of(&decoded).map(|_| decoded)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| text.get(i + 1..i + 3)).flatten().and_then(|h| u8::from_str_radix(h, 16).ok());
        out.push(hex.unwrap_or(if bytes[i] == b'+' { b' ' } else { bytes[i] }));
        i += if hex.is_some() { 3 } else { 1 };
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn name_from_disposition(value: &str) -> Option<String> {
    if let Some(rest) = value.split("filename*=").nth(1) {
        let raw = rest.split(';').next()?.trim().trim_matches('"');
        let encoded = raw.split("''").nth(1).unwrap_or(raw);
        return Some(percent_decode(encoded)).filter(|n| extension_of(n).is_some());
    }
    let rest = value.split("filename=").nth(1)?;
    Some(rest.split(';').next()?.trim().trim_matches('"').to_string()).filter(|n| extension_of(n).is_some())
}

/// The final name: the file's own when known, else "<title> <episode>.<extension>".
fn final_name(item: &Item, hints: &[Option<String>], content_type: Option<&str>) -> String {
    if let Some(own) = hints.iter().flatten().map(|h| h.rsplit(['/', '\\']).next().unwrap_or(h)).find(|h| extension_of(h).is_some()) {
        return clean(own);
    }
    let ext = match content_type.unwrap_or_default() {
        t if t.contains("mp4") => "mp4",
        t if t.contains("webm") => "webm",
        _ => "mkv",
    };
    let base = [Some(item.title.clone()), item.subtitle.clone()].into_iter().flatten().collect::<Vec<_>>().join(" - ");
    format!("{}.{ext}", clean(&base))
}

/// `name.mkv` → `name (2).mkv` when taken.
fn unique(folder: &Path, name: &str) -> String {
    let (stem, ext) = name.rsplit_once('.').map_or((name.to_string(), String::new()), |(s, e)| (s.to_string(), format!(".{e}")));
    let taken = |n: &str| folder.join(n).exists() || folder.join(format!("{n}.part")).exists();
    if !taken(name) {
        return name.to_string();
    }
    (2..1000).map(|i| format!("{stem} ({i}){ext}")).find(|n| !taken(n)).unwrap_or_else(|| format!("{stem} {}{ext}", now_ms()))
}

// ---------------------------------------------------------------- transfer

#[derive(Serialize, Deserialize, Default)]
struct Progress {
    total: u64,
    part: u64,
    /// Bytes written per part, for the parts started.
    done: BTreeMap<u64, u64>,
}

enum Failure {
    Stopped,
    /// Worth retrying (dropped, 5xx, 429, timeout).
    Retry(String, Option<Duration>),
    Fatal(String),
}

fn status_failure(status: u16, retry_after: Option<Duration>) -> Failure {
    match status {
        408 | 425 | 429 | 500..=599 => Failure::Retry(format!("HTTP {status}"), retry_after),
        401 | 403 | 404 | 410 => Failure::Fatal(format!("O servidor recusou o arquivo (HTTP {status}). O link pode ter expirado: baixe de novo pela fonte.")),
        _ => Failure::Fatal(format!("HTTP {status}")),
    }
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let seconds: u64 = response.headers().get("retry-after")?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(seconds.min(120)))
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis((750u64 << attempt.min(6)).min(30_000))
}

struct Transfer {
    client: reqwest::Client,
    url: String,
    headers: Vec<(String, String)>,
    stop: Arc<AtomicBool>,
    written: Arc<AtomicU64>,
}

impl Transfer {
    fn request(&self, range: Option<(u64, Option<u64>)>) -> reqwest::RequestBuilder {
        let mut request = self.client.get(&self.url);
        for (k, v) in &self.headers {
            request = request.header(k, v);
        }
        if let Some((from, to)) = range {
            request = request.header("Range", match to {
                Some(to) => format!("bytes={from}-{to}"),
                None => format!("bytes={from}-"),
            });
        }
        request
    }

    /// Size, whether ranges work, the type and the name the server gives.
    async fn probe(&self) -> Result<(Option<u64>, bool, Option<String>, Option<String>), Failure> {
        let response = self.request(Some((0, Some(0)))).send().await.map_err(|e| Failure::Retry(nuvio_core::CoreError::from(e).to_string(), None))?;
        let status = response.status().as_u16();
        let header = |n: &str| response.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
        let kind = header("content-type");
        let name = header("content-disposition").as_deref().and_then(name_from_disposition);
        match status {
            206 => {
                let total = header("content-range").and_then(|r| r.rsplit('/').next()?.trim().parse().ok());
                Ok((total, total.is_some(), kind, name))
            }
            200 => Ok((header("content-length").and_then(|l| l.parse().ok()), false, kind, name)),
            other => Err(status_failure(other, retry_after(&response))),
        }
    }

    /// Writes `[from, to]` at its place, resuming from what this part already has.
    async fn part(&self, file: &File, from: u64, to: u64, already: &AtomicU64) -> Result<(), Failure> {
        let start = from + already.load(Ordering::SeqCst);
        if start > to {
            return Ok(());
        }
        let mut response = self.request(Some((start, Some(to)))).send().await.map_err(|e| Failure::Retry(nuvio_core::CoreError::from(e).to_string(), None))?;
        match response.status().as_u16() {
            206 => {}
            200 => return Err(Failure::Fatal("O servidor deixou de aceitar download em partes.".into())),
            other => return Err(status_failure(other, retry_after(&response))),
        }
        let mut at = start;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Err(Failure::Stopped);
            }
            let chunk = match tokio::time::timeout(Duration::from_secs(45), response.chunk()).await {
                Ok(Ok(Some(chunk))) => chunk,
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(Failure::Retry(nuvio_core::CoreError::from(e).to_string(), None)),
                Err(_) => return Err(Failure::Retry("sem resposta por 45 s".into(), None)),
            };
            let room = (to + 1 - at) as usize;
            let data = &chunk[..chunk.len().min(room)];
            file.seek_write(data, at).map_err(|e| Failure::Fatal(format!("Não foi possível gravar o arquivo: {e}")))?;
            at += data.len() as u64;
            already.fetch_add(data.len() as u64, Ordering::SeqCst);
            self.written.fetch_add(data.len() as u64, Ordering::SeqCst);
            if at > to {
                break;
            }
        }
        if at <= to {
            return Err(Failure::Retry("conexão encerrada antes do fim da parte".into(), None));
        }
        Ok(())
    }
}

/// Every part of the file over `connections` connections; returns when all are written.
async fn ranged(transfer: Arc<Transfer>, part_path: &Path, state_path: &Path, total: u64, connections: usize) -> Result<(), Failure> {
    let file = Arc::new(OpenOptions::new().create(true).write(true).truncate(false).open(part_path).map_err(|e| Failure::Fatal(e.to_string()))?);
    let mut progress: Progress = std::fs::read(state_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).filter(|p: &Progress| p.total == total && p.part == PART).unwrap_or(Progress { total, part: PART, done: BTreeMap::new() });
    if file.metadata().map(|m| m.len()).unwrap_or(0) != total {
        file.set_len(total).map_err(|e| Failure::Fatal(format!("Não há espaço para reservar o arquivo: {e}")))?;
        if progress.done.values().any(|&d| d > 0) && progress.total != total {
            progress.done.clear();
        }
    }
    let parts = total.div_ceil(PART);
    let counters: Arc<Vec<AtomicU64>> = Arc::new((0..parts).map(|i| AtomicU64::new(progress.done.get(&i).copied().unwrap_or(0))).collect());
    transfer.written.store(counters.iter().map(|c| c.load(Ordering::SeqCst)).sum(), Ordering::SeqCst);
    let pending: Arc<Mutex<VecDeque<u64>>> = Arc::new(Mutex::new(
        (0..parts).filter(|&i| counters[i as usize].load(Ordering::SeqCst) < (PART.min(total - i * PART))).collect(),
    ));
    let failure: Arc<Mutex<Option<Failure>>> = Arc::new(Mutex::new(None));

    // Keeps the progress file current, so any stop resumes.
    let saver = {
        let (counters, path, stop) = (counters.clone(), state_path.to_path_buf(), transfer.stop.clone());
        tokio::spawn(async move {
            loop {
                let snapshot = Progress { total, part: PART, done: counters.iter().enumerate().filter_map(|(i, c)| Some((i as u64, c.load(Ordering::SeqCst))).filter(|(_, v)| *v > 0)).collect() };
                let _ = std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap_or_default());
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        })
    };

    let mut workers = Vec::new();
    for _ in 0..connections.max(1) {
        let (transfer, file, counters, pending, failure) = (transfer.clone(), file.clone(), counters.clone(), pending.clone(), failure.clone());
        workers.push(tokio::spawn(async move {
            loop {
                if failure.lock().expect("failure").is_some() {
                    return;
                }
                let Some(index) = pending.lock().expect("parts").pop_front() else { return };
                let from = index * PART;
                let to = (from + PART).min(total) - 1;
                let mut attempt = 0;
                loop {
                    match transfer.part(&file, from, to, &counters[index as usize]).await {
                        Ok(()) => break,
                        Err(Failure::Retry(reason, wait)) if attempt < PART_RETRIES && !transfer.stop.load(Ordering::SeqCst) => {
                            attempt += 1;
                            eprintln!("[downloads] part {index}: {reason}, retry {attempt}");
                            tokio::time::sleep(wait.unwrap_or_else(|| backoff(attempt))).await;
                        }
                        Err(Failure::Retry(reason, _)) => {
                            failure.lock().expect("failure").get_or_insert(Failure::Fatal(format!("O servidor falhou várias vezes seguidas ({reason}). Retome para tentar de novo.")));
                            return;
                        }
                        Err(other) => {
                            failure.lock().expect("failure").get_or_insert(other);
                            return;
                        }
                    }
                }
            }
        }));
    }
    for worker in workers {
        let _ = worker.await;
    }
    let outcome = failure.lock().expect("failure").take();
    let complete = counters.iter().enumerate().all(|(i, c)| c.load(Ordering::SeqCst) >= PART.min(total - i as u64 * PART));
    transfer.stop.store(true, Ordering::SeqCst);
    let _ = saver.await;
    if transfer.stop.load(Ordering::SeqCst) && !complete && outcome.is_none() {
        return Err(Failure::Stopped);
    }
    match outcome {
        Some(failure) if !complete => Err(failure),
        _ if complete => {
            let _ = file.sync_all();
            Ok(())
        }
        _ => Err(Failure::Stopped),
    }
}

/// One connection from the start (servers without ranges).
async fn single(transfer: Arc<Transfer>, part_path: &Path) -> Result<(), Failure> {
    let mut attempt = 0;
    loop {
        transfer.written.store(0, Ordering::SeqCst);
        let result = async {
            let file = File::create(part_path).map_err(|e| Failure::Fatal(e.to_string()))?;
            let mut response = transfer.request(None).send().await.map_err(|e| Failure::Retry(nuvio_core::CoreError::from(e).to_string(), None))?;
            if !response.status().is_success() {
                return Err(status_failure(response.status().as_u16(), retry_after(&response)));
            }
            let mut at = 0u64;
            loop {
                if transfer.stop.load(Ordering::SeqCst) {
                    return Err(Failure::Stopped);
                }
                match tokio::time::timeout(Duration::from_secs(45), response.chunk()).await {
                    Ok(Ok(Some(chunk))) => {
                        file.seek_write(&chunk, at).map_err(|e| Failure::Fatal(e.to_string()))?;
                        at += chunk.len() as u64;
                        transfer.written.store(at, Ordering::SeqCst);
                    }
                    Ok(Ok(None)) => return Ok(()),
                    Ok(Err(e)) => return Err(Failure::Retry(nuvio_core::CoreError::from(e).to_string(), None)),
                    Err(_) => return Err(Failure::Retry("sem resposta por 45 s".into(), None)),
                }
            }
        }
        .await;
        match result {
            Err(Failure::Retry(_, wait)) if attempt < SINGLE_RETRIES => {
                attempt += 1;
                tokio::time::sleep(wait.unwrap_or_else(|| backoff(attempt))).await;
            }
            other => return other,
        }
    }
}

// ---------------------------------------------------------------- jobs

/// Where the bytes come from now: a link for HTTP and TorBox, the engine's local route for P2P.
async fn open_source(app: &AppHandle, id: &str, source: &Source) -> Result<(String, Vec<(String, String)>, Vec<Option<String>>, usize), String> {
    let state = app.state::<AppState>();
    match source {
        Source::Http { url, headers } => Ok((url.clone(), headers.clone(), vec![name_from_url(url)], HTTP_CONNECTIONS)),
        Source::Torbox { magnet, filename, file_idx, season, episode, service } => {
            let service = service.as_deref().and_then(crate::debrid::Service::parse).unwrap_or(crate::debrid::Service::Torbox);
            let key = crate::debrid::connected_key(&state, service).ok_or("Conecte o serviço debrid deste download em Configurações → Debrid para continuar.")?;
            let wanted = Wanted { magnet: magnet.clone(), filename: filename.clone(), file_idx: *file_idx, season: *season, episode: *episode };
            let resolved = crate::debrid::resolve_with(&state, service, &key, &wanted).await.map_err(|e| match e {
                crate::debrid::Failure::NotThere(t) | crate::debrid::Failure::Error(t) => t,
            })?;
            let hints = vec![resolved.filename.clone(), filename.clone(), name_from_url(&resolved.url)];
            Ok((resolved.url, Vec::new(), hints, HTTP_CONNECTIONS))
        }
        Source::P2p { info_hash, file_index, filename, trackers } => {
            let request = nuvio_p2p::StreamRequest { info_hash: info_hash.clone(), file_index: *file_index, filename: filename.clone(), trackers: trackers.clone() };
            let prepared = p2p::prepare_for_download(&state, id, request).await?;
            Ok((prepared.url, Vec::new(), vec![Some(prepared.file_name), filename.clone()], P2P_CONNECTIONS))
        }
    }
}

async fn run(app: AppHandle, id: String, stop: Arc<AtomicBool>) {
    let state = app.state::<AppState>();
    let downloads = &state.downloads;
    let Some(source) = downloads.sources.lock().expect("download sources").get(&id).cloned() else { return };
    downloads.update(&id, |i| {
        i.status = "preparing".into();
        i.error = None;
    });
    emit(&app);

    let outcome: Result<(), String> = async {
        let (url, headers, hints, connections) = open_source(&app, &id, &source).await?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(10))
            .user_agent(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        let written = Arc::new(AtomicU64::new(0));
        let transfer = Arc::new(Transfer { client, url, headers, stop: stop.clone(), written: written.clone() });
        let mut attempt = 0;
        let (total, ranges, kind, served_name) = loop {
            match transfer.probe().await {
                Ok(found) => break found,
                Err(Failure::Retry(reason, wait)) if attempt < 4 => {
                    attempt += 1;
                    eprintln!("[downloads] probe: {reason}, retry {attempt}");
                    tokio::time::sleep(wait.unwrap_or_else(|| backoff(attempt))).await;
                }
                Err(Failure::Retry(reason, _) | Failure::Fatal(reason)) => return Err(reason),
                Err(Failure::Stopped) => return Err(String::new()),
            }
        };
        let item = downloads.items.lock().expect("downloads").iter().find(|i| i.id == id).cloned().ok_or("download removido")?;
        let folder = PathBuf::from(&item.folder);
        std::fs::create_dir_all(&folder).map_err(|e| format!("Não foi possível usar a pasta: {e}"))?;
        let name = match item.file_name.clone() {
            Some(name) => name,
            None => {
                let mut all = hints.clone();
                all.insert(0, served_name);
                unique(&folder, &final_name(&item, &all, kind.as_deref()))
            }
        };
        if let (Some(total), Some(free)) = (total, free_space(&folder)) {
            let part_size = std::fs::metadata(folder.join(format!("{name}.part"))).map(|m| m.len()).unwrap_or(0);
            if total.saturating_sub(part_size) > free {
                return Err(format!("Falta espaço na pasta: o vídeo tem {} e há {} livres.", size_text(total), size_text(free)));
            }
        }
        let use_ranges = ranges && total.is_some_and(|t| t > PART / 2);
        downloads.update(&id, |i| {
            i.file_name = Some(name.clone());
            i.total = total;
            i.status = "downloading".into();
            i.connections = if use_ranges { connections } else { 1 };
        });
        downloads.save(&state);
        emit(&app);

        // Progress and speed for the screen, once a second.
        let ticker = {
            let (app, id, written, stop) = (app.clone(), id.clone(), written.clone(), stop.clone());
            tokio::spawn(async move {
                let mut last = (Instant::now(), written.load(Ordering::SeqCst));
                let mut speed = 0f64;
                while !stop.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let now = (Instant::now(), written.load(Ordering::SeqCst));
                    let sample = now.1.saturating_sub(last.1) as f64 / now.0.duration_since(last.0).as_secs_f64().max(0.001);
                    speed = if speed == 0.0 { sample } else { speed * 0.6 + sample * 0.4 };
                    last = now;
                    app.state::<AppState>().downloads.update(&id, |i| {
                        i.downloaded = now.1;
                        i.speed = speed as u64;
                    });
                    emit(&app);
                }
            })
        };
        let part_path = folder.join(format!("{name}.part"));
        let state_path = folder.join(format!("{name}.part.json"));
        let result = match total.filter(|_| use_ranges) {
            Some(total) => ranged(transfer.clone(), &part_path, &state_path, total, connections).await,
            None => single(transfer.clone(), &part_path).await,
        };
        let finished = result.is_ok();
        stop.store(true, Ordering::SeqCst);
        let _ = ticker.await;
        match result {
            Ok(()) => {
                std::fs::rename(&part_path, folder.join(&name)).map_err(|e| format!("Não foi possível concluir o arquivo: {e}"))?;
                let _ = std::fs::remove_file(&state_path);
                let size = std::fs::metadata(folder.join(&name)).map(|m| m.len()).unwrap_or(0);
                downloads.update(&id, |i| {
                    i.downloaded = size;
                    i.total = Some(size);
                });
                Ok(())
            }
            Err(Failure::Stopped) if !finished => Err(String::new()),
            Err(Failure::Retry(reason, _) | Failure::Fatal(reason)) => Err(reason),
            Err(Failure::Stopped) => Err(String::new()),
        }
    }
    .await;

    if matches!(source, Source::P2p { .. }) {
        p2p::release_download(&state, &id);
    }
    downloads.running.lock().expect("running").remove(&id);
    downloads.update(&id, |i| {
        i.speed = 0;
        match &outcome {
            Ok(()) => i.status = "done".into(),
            Err(reason) if reason.is_empty() => {
                if i.status != "removed" {
                    i.status = "paused".into();
                }
            }
            Err(reason) => {
                i.status = "failed".into();
                i.error = Some(reason.clone());
            }
        }
    });
    downloads.items.lock().expect("downloads").retain(|i| i.status != "removed");
    downloads.save(&state);
    emit(&app);
    start_next(&app);
}

fn start_next(app: &AppHandle) {
    let state = app.state::<AppState>();
    let downloads = &state.downloads;
    loop {
        if downloads.running.lock().expect("running").len() >= RUNNING_AT_ONCE {
            return;
        }
        let Some(id) = downloads.queue.lock().expect("queue").pop_front() else { return };
        let still_queued = downloads.items.lock().expect("downloads").iter().any(|i| i.id == id && i.status == "queued");
        if !still_queued {
            continue;
        }
        let stop = Arc::new(AtomicBool::new(false));
        downloads.running.lock().expect("running").insert(id.clone(), Running { stop: stop.clone() });
        tauri::async_runtime::spawn(run(app.clone(), id, stop));
    }
}

fn enqueue(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    state.downloads.update(id, |i| {
        i.status = "queued".into();
        i.error = None;
    });
    state.downloads.queue.lock().expect("queue").push_back(id.to_string());
    state.downloads.save(&state);
    emit(app);
    start_next(app);
}

pub fn size_text(bytes: u64) -> String {
    let gb = bytes as f64 / 1_073_741_824.0;
    if gb >= 1.0 { format!("{:.2} GB", gb).replace('.', ",") } else { format!("{:.0} MB", bytes as f64 / 1_048_576.0) }
}

// ---------------------------------------------------------------- commands

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub key: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub poster: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub playback: Option<PlaybackItem>,
}

/// Starts downloading a source of the last searches into the chosen folder.
#[tauri::command]
pub async fn download_start(app: AppHandle, state: State<'_, AppState>, request: Request) -> Result<Item, String> {
    state.ensure_unlocked()?;
    state.downloads.load(&state);
    let stream = state.streams.get(&request.key).ok_or("unknown stream (search again)")?;
    let debrid_service = crate::debrid::ready_service(&state, &request.key);
    let source = match crate::streams::kind_of(&stream) {
        crate::streams::StreamKind::Direct => {
            let url = stream.playable_direct_url().ok_or("Esta fonte não tem link.")?.to_string();
            let path = url.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
            if path.ends_with(".m3u8") || path.ends_with(".mpd") || stream.stream_type.as_deref().is_some_and(|t| t == "hls" || t == "dash") {
                return Err("Transmissões HLS ou DASH são feitas de pedaços e ainda não podem ser baixadas. Escolha uma fonte de arquivo (MKV ou MP4).".into());
            }
            Source::Http { url, headers: stream.playback_request_headers() }
        }
        kind @ (crate::streams::StreamKind::Torrent | crate::streams::StreamKind::Debrid) => {
            match (debrid_service, nuvio_core::torbox::wanted(&stream, request.season, request.episode)) {
                (Some(service), Some(w)) => Source::Torbox {
                    magnet: w.magnet,
                    filename: w.filename,
                    file_idx: w.file_idx,
                    season: w.season,
                    episode: w.episode,
                    service: (service != crate::debrid::Service::Torbox).then(|| service.id().to_string()),
                },
                _ if kind == crate::streams::StreamKind::Debrid => return Err("Conecte o serviço debrid desta fonte em Configurações → Debrid para baixá-la.".into()),
                _ => {
                    let p2p = state.app_store.profile_settings(state.active_profile()).map_err(|e| e.to_string())?.p2p;
                    if !p2p.enabled {
                        return Err(p2p::NEEDS_CONSENT.into());
                    }
                    let r = p2p::request_for(&stream)?;
                    Source::P2p { info_hash: r.info_hash, file_index: r.file_index, filename: r.filename, trackers: r.trackers }
                }
            }
        }
        _ => return Err("Esta fonte não pode ser baixada.".into()),
    };
    let id = format!("d{}{}", now_ms(), nuvio_sync::crypto::random_token(4));
    let item = Item {
        id: id.clone(),
        title: request.title,
        subtitle: request.subtitle,
        poster: request.poster,
        folder: folder(&state).display().to_string(),
        file_name: None,
        status: "queued".into(),
        kind: source.kind().into(),
        source_name: stream.addon_name.clone(),
        downloaded: 0,
        total: None,
        speed: 0,
        connections: 0,
        error: None,
        created_at: now_ms(),
        playback: request.playback,
    };
    state.downloads.items.lock().expect("downloads").insert(0, item.clone());
    state.downloads.sources.lock().expect("download sources").insert(id.clone(), source);
    enqueue(&app, &id);
    Ok(item)
}

#[tauri::command]
pub fn downloads_list(state: State<'_, AppState>) -> Result<Vec<Item>, String> {
    state.ensure_unlocked()?;
    state.downloads.load(&state);
    Ok(state.downloads.list())
}

#[tauri::command]
pub fn download_pause(state: State<'_, AppState>, id: String) {
    if let Some(running) = state.downloads.running.lock().expect("running").get(&id) {
        running.stop.store(true, Ordering::SeqCst);
    }
    state.downloads.queue.lock().expect("queue").retain(|q| q != &id);
    if let Some(item) = state.downloads.update(&id, |i| {
        if i.status == "queued" {
            i.status = "paused".into();
        }
    }) {
        let _ = item;
    }
}

#[tauri::command]
pub fn download_resume(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.ensure_unlocked()?;
    if !state.downloads.sources.lock().expect("download sources").contains_key(&id) {
        return Err("A origem deste download não foi guardada. Baixe de novo pela fonte.".into());
    }
    if state.downloads.running.lock().expect("running").contains_key(&id) {
        return Ok(());
    }
    enqueue(&app, &id);
    Ok(())
}

/// Removes from the list; `delete_file` also deletes what was downloaded (partial or complete).
#[tauri::command]
pub fn download_remove(app: AppHandle, state: State<'_, AppState>, id: String, delete_file: bool) -> Result<(), String> {
    let item = state.downloads.list().into_iter().find(|i| i.id == id).ok_or("download não encontrado")?;
    let running = state.downloads.running.lock().expect("running").get(&id).map(|r| r.stop.clone());
    state.downloads.queue.lock().expect("queue").retain(|q| q != &id);
    if let Some(path) = item.path() {
        let partial = PathBuf::from(format!("{}.part", path.display()));
        let progress = PathBuf::from(format!("{}.part.json", path.display()));
        if item.status != "done" || delete_file {
            // The worker closes the file first; then the leftovers go.
            let (stop, paths) = (running.clone(), vec![partial, progress, if delete_file && item.status == "done" { path } else { PathBuf::new() }]);
            std::thread::spawn(move || {
                if let Some(stop) = stop {
                    stop.store(true, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(1500));
                }
                for p in paths.iter().filter(|p| !p.as_os_str().is_empty()) {
                    let _ = std::fs::remove_file(p);
                }
            });
        }
    }
    if running.is_some() {
        state.downloads.update(&id, |i| i.status = "removed".into());
    } else {
        state.downloads.items.lock().expect("downloads").retain(|i| i.id != id);
    }
    state.downloads.sources.lock().expect("download sources").remove(&id);
    state.downloads.save(&state);
    emit(&app);
    Ok(())
}

/// Shows the file (or the folder) in Windows Explorer.
#[tauri::command]
pub fn download_show(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let item = state.downloads.list().into_iter().find(|i| i.id == id).ok_or("download não encontrado")?;
    let file = item.path().filter(|p| p.exists());
    let mut command = std::process::Command::new("explorer.exe");
    match file {
        Some(path) => command.arg(format!("/select,{}", path.display())),
        None => command.arg(&item.folder),
    };
    command.spawn().map(drop).map_err(|e| e.to_string())
}

/// The finished file, for the player.
pub fn playable(state: &AppState, id: &str) -> Result<(PathBuf, Option<PlaybackItem>), String> {
    state.downloads.load(state);
    let item = state.downloads.list().into_iter().find(|i| i.id == id && i.status == "done").ok_or("download não concluído")?;
    let path = item.path().filter(|p| p.exists()).ok_or("O arquivo não está mais na pasta.")?;
    Ok((path, item.playback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_on_windows() {
        assert_eq!(clean(r#"Show: "A/B"? <1>*.mkv"#), "Show A B 1 .mkv");
        assert_eq!(clean("CON.mkv"), "video");
        assert_eq!(name_from_url("https://h.test/d/My%20Show%20E01.mkv?t=1").as_deref(), Some("My Show E01.mkv"));
        assert_eq!(name_from_url("https://h.test/play/abc"), None);
        assert_eq!(name_from_disposition(r#"attachment; filename*=UTF-8''A%C3%A7%C3%A3o.mp4"#).as_deref(), Some("Ação.mp4"));
        let item = Item {
            id: "x".into(), title: "Série".into(), subtitle: Some("T1 E2".into()), poster: None, folder: String::new(), file_name: None,
            status: String::new(), kind: String::new(), source_name: String::new(), downloaded: 0, total: None, speed: 0, connections: 0,
            error: None, created_at: 0, playback: None,
        };
        assert_eq!(final_name(&item, &[None, Some("pack/Episode.02.mkv".into())], None), "Episode.02.mkv");
        assert_eq!(final_name(&item, &[None], Some("video/mp4")), "Série - T1 E2.mp4");
    }

    #[test]
    fn retries_only_what_can_recover() {
        assert!(matches!(status_failure(503, None), Failure::Retry(..)));
        assert!(matches!(status_failure(429, Some(Duration::from_secs(3))), Failure::Retry(_, Some(_))));
        assert!(matches!(status_failure(403, None), Failure::Fatal(_)));
        assert_eq!(backoff(1), Duration::from_millis(1500));
        assert_eq!(backoff(20), Duration::from_secs(30));
    }
}
