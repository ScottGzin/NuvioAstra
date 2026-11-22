//! P2P streaming (torrent sources from addons) with the original's own engine.
//!
//! The original's desktop app (`P2pStreamingEngine.desktop.kt`) runs the Nuvio Engine
//! (`nuvio_engine.dll`, libtorrent) and plays the local HTTP URL it serves for the selected file;
//! the engine downloads the pieces the player asks for first, so playback starts long before the
//! download ends and seeking works (HTTP ranges). This crate does the same from Rust:
//! [`Engine`] is the thin, thread-safe binding (commands complete through the engine's events) and
//! [`P2pService`] is the single playback session: start (magnet with the addon's trackers + the
//! original's default ones, metadata, file selection by index / filename / engine's choice),
//! statistics, and a full stop (stream stopped, torrent removed, engine destroyed: no peer
//! connection, listening socket or thread is left).

pub mod ffi;

use ffi::{Api, Handle};
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const ENGINE_DLL: &str = "nuvio_engine.dll";

/// Trackers the original always adds (`DEFAULT_TRACKERS`).
pub const DEFAULT_TRACKERS: &[&str] = &[
    "udp://zer0day.ch:1337/announce",
    "udp://tracker.publictracker.xyz:6969/announce",
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://open.stealth.si:80/announce",
    "http://tracker.renfei.net:8080/announce",
    "udp://udp.tracker.projectk.org:23333/announce",
    "udp://tracker.tryhackx.org:6969/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://tracker.theoks.net:6969/announce",
    "udp://tracker.startwork.cv:1337/announce",
    "udp://tracker.qu.ax:6969/announce",
    "udp://tracker.plx.im:6969/announce",
    "udp://tracker.nyaa.vc:6969/announce",
    "udp://tracker.iperson.xyz:6969/announce",
    "udp://tracker.gmi.gd:6969/announce",
    "udp://tracker.fnix.net:6969/announce",
    "udp://tracker.flatuslifir.is:6969/announce",
    "udp://tracker.ducks.party:1984/announce",
    "udp://tracker.bluefrog.pw:2710/announce",
];

const METADATA_TIMEOUT: Duration = Duration::from_secs(120);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(60);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(40);

/// `canonicalP2pInfoHash`: 40 (v1) or 64 (v2) hex characters, lowercase.
pub fn canonical_info_hash(hash: &str) -> Result<String, String> {
    let h = hash.trim().to_ascii_lowercase();
    if (h.len() == 40 || h.len() == 64) && h.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(h)
    } else {
        Err("The torrent hash must have 40 or 64 hexadecimal characters.".into())
    }
}

/// Info hash of a `magnet:?xt=urn:btih:<hex>` link (base32 v1 hashes are converted).
pub fn info_hash_from_magnet(magnet: &str) -> Option<String> {
    let lower = magnet.trim();
    let start = lower.to_ascii_lowercase().find("urn:btih:")? + "urn:btih:".len();
    let value: String = lower[start..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    match value.len() {
        40 => canonical_info_hash(&value).ok(),
        32 => base32_to_hex(&value),
        _ => None,
    }
}

fn base32_to_hex(value: &str) -> Option<String> {
    let mut bits: u64 = 0;
    let mut count = 0;
    let mut out = String::new();
    for c in value.chars() {
        let v = match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u64 - 'A' as u64,
            c @ '2'..='7' => c as u64 - '2' as u64 + 26,
            _ => return None,
        };
        bits = (bits << 5) | v;
        count += 5;
        if count >= 8 {
            count -= 8;
            out.push_str(&format!("{:02x}", (bits >> count) & 0xff));
        }
    }
    (out.len() == 40).then_some(out)
}

/// Trackers of a magnet link (`tr=` parameters, URL-decoded).
pub fn trackers_from_magnet(magnet: &str) -> Vec<String> {
    magnet
        .split(['?', '&'])
        .filter_map(|p| p.strip_prefix("tr="))
        .map(percent_decode)
        .filter(|t| !t.is_empty())
        .collect()
}

fn percent_decode(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(x) = u8::from_str_radix(&v[i + 1..i + 3], 16) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `buildP2pMagnetUri`.
pub fn magnet_uri(info_hash: &str, trackers: &[String]) -> Result<String, String> {
    let hash = canonical_info_hash(info_hash)?;
    let topic = if hash.len() == 40 { format!("urn:btih:{hash}") } else { format!("urn:btmh:1220{hash}") };
    let mut seen = std::collections::HashSet::new();
    let mut uri = format!("magnet:?xt={topic}");
    for t in DEFAULT_TRACKERS.iter().map(|s| s.to_string()).chain(trackers.iter().cloned()) {
        let t = t.trim().to_string();
        if !t.is_empty() && seen.insert(t.clone()) {
            uri.push_str("&tr=");
            for byte in t.bytes() {
                if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                    uri.push(byte as char);
                } else {
                    uri.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    Ok(uri)
}

/// Engine settings (`buildNuvioEngineConfig`).
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub disk_cache_bytes: u64,
    pub upload: bool,
    /// Soft 0, Balanced 1, Fast 2.
    pub profile: i32,
}

#[derive(Debug, Clone)]
pub struct EventData {
    pub kind: i32,
    pub torrent_id: String,
    pub message: String,
    pub file_index: i32,
    pub file_size: u64,
    pub stream_id: String,
    pub stream_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFile {
    pub index: i32,
    pub size: u64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedStream {
    #[serde(skip)]
    pub stream_id: String,
    #[serde(skip)]
    pub torrent_id: String,
    /// Local HTTP URL served by the engine (127.0.0.1), what the player opens.
    #[serde(skip)]
    pub url: String,
    pub file_index: i32,
    pub file_size: u64,
    /// Name of the selected file inside the torrent.
    pub file_name: String,
    pub file_count: usize,
}

static API: OnceLock<Result<Arc<Api>, String>> = OnceLock::new();

/// Loads the engine DLL once per process (it is never unloaded: its threads must not outlive it).
pub fn load_api(path: &Path) -> Result<Arc<Api>, String> {
    API.get_or_init(|| Api::load(path).map(Arc::new)).clone()
}

/// `nuvio_engine.dll` next to the executable, or `NUVIO_ENGINE_DLL`.
pub fn locate_engine() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NUVIO_ENGINE_DLL").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join(ENGINE_DLL);
    p.is_file().then_some(p)
}

struct Native {
    api: Arc<Api>,
    handle: Handle,
    /// The original serializes every native call (`nativeLock`).
    lock: Mutex<()>,
}
// SAFETY: the handle is only used under `lock`, as the original's binding does.
unsafe impl Send for Native {}
unsafe impl Sync for Native {}

type Waiters = HashMap<u64, (i32, Sender<Result<EventData, String>>)>;

/// One engine instance and its event loop.
pub struct Engine {
    native: Arc<Native>,
    waiters: Arc<Mutex<Waiters>>,
    /// Last error the engine reported for no command (torrent failed while streaming).
    async_error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    poller: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn create(api: Arc<Api>, config: &EngineConfig) -> Result<Self, String> {
        std::fs::create_dir_all(&config.data_dir).map_err(|e| format!("P2P engine folder: {e}"))?;
        std::fs::create_dir_all(&config.cache_dir).map_err(|e| format!("P2P engine cache: {e}"))?;
        let data = path_cstring(&config.data_dir)?;
        let cache = path_cstring(&config.cache_dir)?;
        let mut c: ffi::Config = ffi::init(api.config_init);
        c.data_directory = data.as_ptr();
        c.cache_directory = cache.as_ptr();
        c.memory_cache_capacity_bytes = 64 << 20;
        c.disk_cache_capacity_bytes = config.disk_cache_bytes;
        c.listen_port = 0;
        c.upload_mode = if config.upload { 1 } else { 0 };
        c.upload_limit_bytes_per_second = 0;
        // The player decides when the stream ends (the original's desktop value).
        c.stream_inactivity_timeout_ms = 0;
        c.warm_torrent_timeout_ms = 60_000;
        c.torrent_profile = config.profile;
        let mut handle: Handle = std::ptr::null_mut();
        // SAFETY: `c` was initialized by the engine and its strings outlive the call.
        let status = unsafe { (api.create)(&c, &mut handle) };
        if status != ffi::STATUS_OK || handle.is_null() {
            return Err(format!("Could not start the P2P engine: {}", message(&api, status)));
        }
        let native = Arc::new(Native { api, handle, lock: Mutex::new(()) });
        let waiters: Arc<Mutex<Waiters>> = Arc::default();
        let async_error: Arc<Mutex<Option<String>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let poller = {
            let (native, waiters, async_error, stop) = (native.clone(), waiters.clone(), async_error.clone(), stop.clone());
            std::thread::Builder::new()
                .name("nuvio-p2p-events".into())
                .spawn(move || event_loop(&native, &waiters, &async_error, &stop))
                .map_err(|e| e.to_string())?
        };
        Ok(Self { native, waiters, async_error, stop, poller: Some(poller) })
    }

    pub fn version(&self) -> String {
        // SAFETY: static strings of the engine.
        unsafe { format!("{} ({})", ffi::c_text((self.native.api.version_string)()), ffi::c_text((self.native.api.backend_version)())) }
    }

    /// Issues a command and waits for its completion event (`awaitCommand`).
    fn command(
        &self,
        expected: i32,
        timeout: Duration,
        cancel: &dyn Fn() -> bool,
        call: impl FnOnce(&Api, Handle, &mut u64) -> i32,
    ) -> Result<EventData, String> {
        let (tx, rx) = channel();
        {
            // Registered before the poller can dispatch the answer (it needs this lock).
            let mut waiters = self.waiters.lock().expect("p2p waiters");
            let mut request = 0u64;
            let status = {
                let _g = self.native.lock.lock().expect("p2p native");
                call(&self.native.api, self.native.handle, &mut request)
            };
            if status != ffi::STATUS_OK {
                return Err(message(&self.native.api, status));
            }
            waiters.insert(request, (expected, tx));
        }
        let deadline = Instant::now() + timeout;
        loop {
            match rx.recv_timeout(Duration::from_millis(150)) {
                Ok(r) => return r,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err("P2P engine stopped".into()),
                Err(_) if cancel() => return Err(CANCELLED.into()),
                Err(_) if Instant::now() >= deadline => return Err(TIMEOUT.into()),
                Err(_) => {}
            }
        }
    }

    /// Adds a magnet link and waits for its metadata; returns the torrent id.
    pub fn add_magnet(&self, magnet: &str, cancel: &dyn Fn() -> bool) -> Result<String, String> {
        let uri = CString::new(magnet).map_err(|_| "invalid magnet link".to_string())?;
        let e = self.command(ffi::EVENT_METADATA_READY, METADATA_TIMEOUT, cancel, |api, h, req| {
            let mut r: ffi::TorrentRequest = ffi::init(api.torrent_request_init);
            r.source_type = ffi::SOURCE_MAGNET;
            r.magnet_uri = uri.as_ptr();
            // SAFETY: initialized request; `uri` lives through the call.
            unsafe { (api.add_torrent)(h, &r, req) }
        })?;
        Ok(e.torrent_id)
    }

    pub fn files(&self, torrent_id: &str) -> Result<Vec<TorrentFile>, String> {
        let tid = CString::new(torrent_id).map_err(|e| e.to_string())?;
        let api = &self.native.api;
        let _g = self.native.lock.lock().expect("p2p native");
        let mut count = 0usize;
        // SAFETY: valid handle and NUL-terminated id.
        let status = unsafe { (api.get_file_count)(self.native.handle, tid.as_ptr(), &mut count) };
        if status != ffi::STATUS_OK {
            return Err(message(api, status));
        }
        let mut out = Vec::with_capacity(count.min(100_000));
        for i in 0..count.min(100_000) {
            let mut f: ffi::File = ffi::init(api.file_init);
            // SAFETY: `f` initialized by the engine.
            if unsafe { (api.get_file)(self.native.handle, tid.as_ptr(), i, &mut f) } != ffi::STATUS_OK {
                break;
            }
            out.push(TorrentFile { index: f.index, size: f.size, path: ffi::text(&f.path) });
        }
        Ok(out)
    }

    /// Prepares the HTTP route of one file (`fileIdx` from the addon, else the filename hint, else
    /// the engine's own choice: the main video file).
    pub fn prepare_stream(&self, torrent_id: &str, file_index: Option<i32>, filename: Option<&str>, cancel: &dyn Fn() -> bool) -> Result<EventData, String> {
        let tid = CString::new(torrent_id).map_err(|e| e.to_string())?;
        let hint = filename.filter(|f| !f.trim().is_empty()).and_then(|f| CString::new(f).ok());
        self.command(ffi::EVENT_STREAM_PREPARED, PREPARE_TIMEOUT, cancel, |api, h, req| {
            let mut r: ffi::StreamRequest = ffi::init(api.stream_request_init);
            r.torrent_id = tid.as_ptr();
            if let Some(i) = file_index.filter(|i| *i >= 0) {
                r.file_index = i;
            }
            r.filename_hint = hint.as_ref().map_or(std::ptr::null(), |h| h.as_ptr());
            // SAFETY: initialized request; strings live through the call.
            unsafe { (api.prepare_stream)(h, &r, req) }
        })
    }

    pub fn stop_stream(&self, stream_id: &str) -> Result<(), String> {
        let sid = CString::new(stream_id).map_err(|e| e.to_string())?;
        // SAFETY: valid handle and id.
        self.command(ffi::EVENT_STREAM_STOPPED, STOP_TIMEOUT, &|| false, |api, h, req| unsafe { (api.stop_stream)(h, sid.as_ptr(), req) }).map(drop)
    }

    pub fn remove_torrent(&self, torrent_id: &str) -> Result<(), String> {
        let tid = CString::new(torrent_id).map_err(|e| e.to_string())?;
        // SAFETY: valid handle and id.
        self.command(ffi::EVENT_TORRENT_REMOVED, STOP_TIMEOUT, &|| false, |api, h, req| unsafe { (api.remove_torrent)(h, tid.as_ptr(), req) }).map(drop)
    }

    /// Frees the disk cache down to `target` bytes (0 = everything not in use).
    pub fn reclaim_disk_cache(&self, target: u64) -> Result<(), String> {
        // SAFETY: valid handle.
        self.command(ffi::EVENT_DISK_CACHE_RECLAIMED, Duration::from_secs(30), &|| false, |api, h, req| unsafe { (api.reclaim_disk_cache)(h, target, req) }).map(drop)
    }

    pub fn stats(&self) -> Option<ffi::Stats> {
        let api = &self.native.api;
        let mut s: ffi::Stats = ffi::init(api.stats_init);
        let _g = self.native.lock.lock().expect("p2p native");
        // SAFETY: initialized struct, valid handle.
        (unsafe { (api.get_stats)(self.native.handle, &mut s) } == ffi::STATUS_OK).then_some(s)
    }

    pub fn stream_stats(&self, stream_id: &str) -> Option<ffi::StreamStats> {
        let sid = CString::new(stream_id).ok()?;
        let api = &self.native.api;
        let mut s: ffi::StreamStats = ffi::init(api.stream_stats_init);
        let _g = self.native.lock.lock().expect("p2p native");
        // SAFETY: initialized struct, valid handle and id.
        (unsafe { (api.get_stream_stats)(self.native.handle, sid.as_ptr(), &mut s) } == ffi::STATUS_OK).then_some(s)
    }

    pub fn take_async_error(&self) -> Option<String> {
        self.async_error.lock().expect("p2p error").take()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(p) = self.poller.take() {
            let _ = p.join();
        }
        self.waiters.lock().expect("p2p waiters").clear();
        let _g = self.native.lock.lock().expect("p2p native");
        // SAFETY: last use of the handle; the poller has ended.
        unsafe { (self.native.api.destroy)(self.native.handle) };
    }
}

const CANCELLED: &str = "canceled";
const TIMEOUT: &str = "The torrent did not respond in time (no peers or metadata).";

fn event_loop(native: &Native, waiters: &Mutex<Waiters>, async_error: &Mutex<Option<String>>, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        let mut got = false;
        loop {
            let mut e: ffi::Event = ffi::init(native.api.event_init);
            let status = {
                let _g = native.lock.lock().expect("p2p native");
                // SAFETY: initialized event struct, valid handle.
                unsafe { (native.api.poll_event)(native.handle, &mut e) }
            };
            if status != ffi::STATUS_OK {
                break;
            }
            got = true;
            let data = EventData {
                kind: e.kind,
                torrent_id: ffi::text(&e.torrent_id),
                message: ffi::text(&e.message),
                file_index: e.file_index,
                file_size: e.file_size,
                stream_id: ffi::text(&e.stream_id),
                stream_url: ffi::text(&e.stream_url),
            };
            if e.request_id == 0 {
                if data.kind == ffi::EVENT_TORRENT_ERROR || (data.kind == ffi::EVENT_STREAM_STOPPED && !data.message.is_empty()) {
                    *async_error.lock().expect("p2p error") = Some(if data.message.is_empty() { "unknown torrent error".into() } else { data.message });
                }
                continue;
            }
            let mut w = waiters.lock().expect("p2p waiters");
            if let Some((expected, _)) = w.get(&e.request_id) {
                if data.kind == ffi::EVENT_TORRENT_ERROR {
                    let (_, tx) = w.remove(&e.request_id).expect("present");
                    let _ = tx.send(Err(if data.message.is_empty() { "torrent operation failed".into() } else { data.message }));
                } else if data.kind == *expected {
                    let (_, tx) = w.remove(&e.request_id).expect("present");
                    let _ = tx.send(Ok(data));
                }
            }
        }
        if !got {
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

fn message(api: &Api, status: i32) -> String {
    // SAFETY: static string of the engine.
    let m = unsafe { ffi::c_text((api.status_message)(status)) };
    if m.is_empty() { format!("code {status}") } else { m }
}

fn path_cstring(p: &Path) -> Result<CString, String> {
    let s = p.to_str().ok_or("P2P engine path contains invalid characters")?;
    if s.len() >= 4096 {
        return Err("P2P engine path is too long".into());
    }
    CString::new(s).map_err(|e| e.to_string())
}

/// What the player starts.
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub info_hash: String,
    pub file_index: Option<i32>,
    pub filename: Option<String>,
    pub trackers: Vec<String>,
}

/// Session state for the UI (`P2pStreamingState`).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct P2pStatus {
    /// `idle`, `connecting`, `streaming`, `error`.
    pub state: String,
    /// `engine`, `metadata`, `preparing` while connecting.
    pub phase: String,
    pub peers: i32,
    pub seeds: i32,
    pub download_speed: u64,
    pub upload_speed: u64,
    /// Payload downloaded by this session.
    pub downloaded_bytes: u64,
    pub file_name: Option<String>,
    pub file_size: u64,
    /// Verified bytes of the file / file size.
    pub total_progress: f64,
    /// Contiguous bytes ready ahead of the read position.
    pub ready_ahead_bytes: u64,
    pub error: Option<String>,
}

struct Active {
    engine: Arc<Engine>,
    torrent_id: Option<String>,
    stream: Option<PreparedStream>,
    download_baseline: u64,
}

/// The single P2P playback session of the app.
#[derive(Default)]
pub struct P2pService {
    active: Mutex<Option<Active>>,
    generation: AtomicU64,
    status: Mutex<P2pStatus>,
}

impl P2pService {
    pub fn is_active(&self) -> bool {
        self.active.lock().expect("p2p active").is_some()
    }

    fn set_status(&self, f: impl FnOnce(&mut P2pStatus)) {
        f(&mut self.status.lock().expect("p2p status"));
    }

    /// Starts a stream (stopping any previous one) and returns the prepared file. Blocking; a
    /// `stop()` from another thread cancels it.
    pub fn start(&self, dll: &Path, config: &EngineConfig, request: &StreamRequest) -> Result<PreparedStream, String> {
        self.stop();
        let generation = self.generation.load(Ordering::SeqCst);
        let cancelled = || self.generation.load(Ordering::SeqCst) != generation;
        *self.status.lock().expect("p2p status") = P2pStatus { state: "connecting".into(), phase: "engine".into(), ..Default::default() };
        let result = (|| {
            let magnet = magnet_uri(&request.info_hash, &request.trackers)?;
            let api = load_api(dll)?;
            let engine = Arc::new(Engine::create(api, config)?);
            let baseline = engine.stats().map_or(0, |s| s.total_payload_download);
            {
                let mut active = self.active.lock().expect("p2p active");
                if cancelled() {
                    return Err(CANCELLED.to_string());
                }
                *active = Some(Active { engine: engine.clone(), torrent_id: None, stream: None, download_baseline: baseline });
            }
            self.set_status(|s| s.phase = "metadata".into());
            let torrent_id = engine.add_magnet(&magnet, &cancelled)?;
            self.with_active(generation, |a| a.torrent_id = Some(torrent_id.clone()))?;
            let files = engine.files(&torrent_id).unwrap_or_default();
            self.set_status(|s| s.phase = "preparing".into());
            let e = engine.prepare_stream(&torrent_id, request.file_index, request.filename.as_deref(), &cancelled)?;
            let file_name = files
                .iter()
                .find(|f| f.index == e.file_index)
                .map(|f| f.path.rsplit(['/', '\\']).next().unwrap_or(&f.path).to_string())
                .unwrap_or_default();
            let prepared = PreparedStream {
                stream_id: e.stream_id,
                torrent_id,
                url: e.stream_url,
                file_index: e.file_index,
                file_size: e.file_size,
                file_name,
                file_count: files.len(),
            };
            if !prepared.url.starts_with("http://127.0.0.1") && !prepared.url.starts_with("http://localhost") && !prepared.url.starts_with("http://[::1]") {
                return Err("the P2P engine returned a non-local address".into());
            }
            self.with_active(generation, |a| a.stream = Some(prepared.clone()))?;
            self.set_status(|s| {
                s.state = "streaming".into();
                s.phase.clear();
                s.file_name = Some(prepared.file_name.clone());
                s.file_size = prepared.file_size;
            });
            Ok(prepared)
        })();
        if let Err(e) = &result {
            if !cancelled() {
                self.stop();
                let text = e.clone();
                self.set_status(|s| {
                    *s = P2pStatus { state: "error".into(), error: Some(text), ..Default::default() };
                });
            }
        }
        result
    }

    fn with_active(&self, generation: u64, f: impl FnOnce(&mut Active)) -> Result<(), String> {
        let mut active = self.active.lock().expect("p2p active");
        match active.as_mut() {
            Some(a) if self.generation.load(Ordering::SeqCst) == generation => {
                f(a);
                Ok(())
            }
            _ => Err(CANCELLED.into()),
        }
    }

    /// Current statistics (also detects an error the engine reported while streaming).
    pub fn refresh(&self) -> P2pStatus {
        let active = self.active.lock().expect("p2p active");
        if let Some(a) = active.as_ref() {
            let stats = a.engine.stats();
            let stream = a.stream.as_ref().and_then(|s| a.engine.stream_stats(&s.stream_id));
            let error = a.engine.take_async_error();
            drop(active);
            self.set_status(|s| {
                if let Some(st) = &stats {
                    s.peers = st.connected_peers;
                    s.seeds = st.connected_seeds;
                    s.download_speed = st.download_rate;
                    s.upload_speed = st.upload_rate;
                }
                if let Some(ss) = &stream {
                    s.total_progress = if ss.file_size > 0 { ss.verified_file_bytes as f64 / ss.file_size as f64 } else { 0.0 };
                    s.ready_ahead_bytes = ss.contiguous_ready_bytes;
                }
                if let Some(e) = error {
                    s.state = "error".into();
                    s.error = Some(e);
                }
            });
            let base = self.active.lock().expect("p2p active").as_ref().map_or(0, |a| a.download_baseline);
            if let Some(st) = stats {
                self.set_status(|s| s.downloaded_bytes = st.total_payload_download.saturating_sub(base));
            }
        }
        self.status.lock().expect("p2p status").clone()
    }

    pub fn status(&self) -> P2pStatus {
        self.status.lock().expect("p2p status").clone()
    }

    /// Ends the session: stream route stopped, torrent removed, engine destroyed (its sockets and
    /// threads end with it). Cancels a start in progress. Safe to call any time.
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let active = self.active.lock().expect("p2p active").take();
        if let Some(a) = active {
            if let Some(s) = &a.stream {
                let _ = a.engine.stop_stream(&s.stream_id);
            }
            if let Some(t) = &a.torrent_id {
                let _ = a.engine.remove_torrent(t);
            }
            // Dropped here unless a cancelled start still holds it (then it drops when that ends).
            drop(a);
        }
        *self.status.lock().expect("p2p status") = P2pStatus { state: "idle".into(), ..Default::default() };
    }

    /// Clears the torrent cache (not while streaming), like "Limpar cache" in the original.
    pub fn clear_cache(&self, dll: &Path, config: &EngineConfig) -> Result<u64, String> {
        if self.is_active() {
            return Err("The cache cannot be cleared during P2P playback.".into());
        }
        let before = dir_size(&config.cache_dir);
        let engine = Engine::create(load_api(dll)?, config)?;
        engine.reclaim_disk_cache(0)?;
        drop(engine);
        Ok(before.saturating_sub(dir_size(&config.cache_dir)))
    }
}

pub fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnet_like_the_original() {
        let m = magnet_uri("DD8255ECDC7CA55FB0BBF81323D87062DB1F6D1C", &["udp://x:1/announce".into(), DEFAULT_TRACKERS[0].into()]).unwrap();
        assert!(m.starts_with("magnet:?xt=urn:btih:dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c&tr=udp%3A%2F%2Fzer0day.ch%3A1337%2Fannounce"));
        assert_eq!(m.matches("zer0day").count(), 1, "deduplicated");
        assert!(m.ends_with("&tr=udp%3A%2F%2Fx%3A1%2Fannounce"));
        assert!(magnet_uri("xyz", &[]).is_err());
        let v2 = "a".repeat(64);
        assert!(magnet_uri(&v2, &[]).unwrap().contains("urn:btmh:1220"));
    }

    #[test]
    fn magnet_parsing() {
        let m = "magnet:?xt=urn:btih:DD8255ECDC7CA55FB0BBF81323D87062DB1F6D1C&dn=x&tr=udp%3A%2F%2Ft%3A1&tr=wss%3A%2F%2Fw";
        assert_eq!(info_hash_from_magnet(m).as_deref(), Some("dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c"));
        assert_eq!(trackers_from_magnet(m), vec!["udp://t:1", "wss://w"]);
        // base32 (RFC 4648) form: 20 bytes 0x00..0x13
        assert_eq!(info_hash_from_magnet("magnet:?xt=urn:btih:AAAQEAYEAUDAOCAJBIFQYDIOB4IBCEQT").as_deref(), Some("000102030405060708090a0b0c0d0e0f10111213"));
    }

    #[test]
    fn struct_sizes_match_the_jvm_binding() {
        // JNA lays these out with the platform's C alignment, as repr(C) does.
        assert_eq!(std::mem::size_of::<ffi::Event>(), 960);
        assert_eq!(std::mem::size_of::<ffi::File>(), 1056);
        assert_eq!(std::mem::size_of::<ffi::StreamRequest>(), 32);
        assert_eq!(std::mem::size_of::<ffi::TorrentRequest>(), 40);
    }
}
