//! C ABI of the original's Nuvio Engine (`nuvio_engine.dll`, libtorrent inside), loaded at runtime
//! like libmpv. Struct layouts, enum values and the call protocol were taken from the original's
//! JVM binding (`nuvio-engine-jvm-0.1.6.jar`: `JnaNativeTypes.kt`, `JnaNuvioNativeApi`,
//! `NuvioEngine`): every struct starts with its size and is initialized by the engine's
//! `*_init_sized` function before use; commands return a request id and complete by an event.

use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::path::Path;

pub const STATUS_OK: c_int = 0;
pub const STATUS_NO_EVENT: c_int = 7;
pub const SOURCE_MAGNET: c_int = 0;

pub const EVENT_TORRENT_ADDED: c_int = 1;
pub const EVENT_METADATA_READY: c_int = 2;
pub const EVENT_TORRENT_ERROR: c_int = 3;
pub const EVENT_STREAM_PREPARED: c_int = 4;
pub const EVENT_TORRENT_REMOVED: c_int = 5;
pub const EVENT_STREAM_STOPPED: c_int = 6;
pub const EVENT_DISK_CACHE_RECLAIMED: c_int = 7;

#[repr(C)]
pub struct Config {
    pub struct_size: i32,
    pub data_directory: *const c_char,
    pub cache_directory: *const c_char,
    pub memory_cache_capacity_bytes: u64,
    pub disk_cache_capacity_bytes: u64,
    pub listen_port: u16,
    pub reserved0: u16,
    pub upload_mode: i32,
    pub upload_limit_bytes_per_second: u64,
    pub stream_inactivity_timeout_ms: i32,
    pub reserved1: i32,
    pub warm_torrent_timeout_ms: i32,
    pub reserved2: i32,
    pub tls_ca_bundle_path: *const c_char,
    pub torrent_profile: i32,
    pub reserved3: i32,
}

#[repr(C)]
pub struct TorrentRequest {
    pub struct_size: i32,
    pub magnet_uri: *const c_char,
    pub source_type: i32,
    pub reserved0: i32,
    pub torrent_data: *const c_void,
    pub torrent_data_size: usize,
}

#[repr(C)]
pub struct StreamRequest {
    pub struct_size: i32,
    pub torrent_id: *const c_char,
    pub file_index: i32,
    pub reserved0: i32,
    pub filename_hint: *const c_char,
}

#[repr(C)]
pub struct Event {
    pub struct_size: i32,
    pub kind: i32,
    pub sequence: u64,
    pub request_id: u64,
    pub dropped_events: u64,
    pub torrent_id: [u8; 65],
    pub message: [u8; 256],
    pub file_index: i32,
    pub reserved0: i32,
    pub file_size: u64,
    pub stream_id: [u8; 65],
    pub stream_url: [u8; 512],
}

#[repr(C)]
pub struct File {
    pub struct_size: i32,
    pub index: i32,
    pub offset: u64,
    pub size: u64,
    pub path_truncated: u8,
    pub reserved0: [u8; 7],
    pub path: [u8; 1024],
}

#[repr(C)]
pub struct Stats {
    pub struct_size: i32,
    pub active_torrents: i32,
    pub active_streams: i32,
    pub active_http_requests: i32,
    pub connected_peers: i32,
    pub connected_seeds: i32,
    pub pending_piece_reads: i32,
    pub reserved0: i32,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub total_payload_download: u64,
    pub total_payload_upload: u64,
    pub memory_cache_capacity: u64,
    pub memory_cache_used: u64,
    pub memory_cache_hits: u64,
    pub memory_cache_misses: u64,
    pub memory_cache_evictions: u64,
    pub memory_cache_entries: u64,
    pub warm_torrents: i32,
    pub quiesced_torrents: i32,
    pub disk_cache_capacity: u64,
    pub disk_cache_used: u64,
    pub disk_cache_protected: u64,
    pub disk_cache_evictions: u64,
    pub disk_cache_reclaimed: u64,
    pub disk_cache_over_budget: u8,
    pub reserved1: [u8; 7],
    pub known_peers: i32,
    pub connect_candidates: i32,
    pub interested_peers: i32,
    pub unchoked_peers: i32,
    pub downloading_peers: i32,
    pub snubbed_peers: i32,
    pub pending_block_requests: i32,
    pub target_block_requests: i32,
    pub timed_out_block_requests: i32,
    pub connecting_peers: i32,
    pub handshaking_peers: i32,
    pub target_piece_peers: i32,
    pub target_piece_unchoked_peers: i32,
    pub target_piece_downloading_peers: i32,
    pub off_target_downloading_peers: i32,
    pub tracker_reply_events: i32,
    pub tracker_error_events: i32,
    pub dht_reply_events: i32,
    pub reserved2: i32,
    pub tracker_peers_returned: u64,
    pub dht_peers_returned: u64,
    pub peer_connect_events: u64,
    pub peer_disconnect_events: u64,
    pub peer_disconnect_timeouts: u64,
    pub peer_disconnect_connect_failures: u64,
    pub peer_disconnect_redundant: u64,
    pub peer_disconnect_turnover: u64,
    pub peer_disconnect_other: u64,
    pub torrent_finished_events: u64,
}

#[repr(C)]
pub struct StreamStats {
    pub struct_size: i32,
    pub file_index: i32,
    pub file_size: u64,
    pub contiguous_ready_bytes: u64,
    pub verified_file_bytes: u64,
    pub delivered_bytes: u64,
    pub active_demands: i32,
    pub scheduled_pieces: i32,
    pub blocking_pieces: i32,
    pub primary_blocking_piece: i32,
    pub secondary_blocking_piece: i32,
    pub last_ready_piece: i32,
    pub primary_demand_start: u64,
    pub primary_demand_end: u64,
    pub secondary_demand_start: u64,
    pub secondary_demand_end: u64,
    pub schedule_revision: u64,
}

pub type Handle = *mut c_void;

macro_rules! api {
    ($($name:ident : $sym:literal => fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        pub struct Api {
            _lib: Library,
            $(pub $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }
        impl Api {
            pub fn load(path: &Path) -> Result<Self, String> {
                // SAFETY: loading the engine runs its DLL initialization, which has no preconditions.
                let lib = unsafe { Library::new(path) }.map_err(|e| format!("could not load the P2P engine ({}): {e}", path.display()))?;
                $(
                    // SAFETY: the symbol has this C signature in the engine's ABI (see module docs).
                    let $name = *unsafe { lib.get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(concat!($sym, "\0").as_bytes()) }
                        .map_err(|e| format!("incompatible P2P engine ({}): {e}", $sym))?;
                )*
                Ok(Self { _lib: lib, $($name,)* })
            }
        }
    };
}

api! {
    api_version: "nuvio_engine_api_version" => fn() -> c_int;
    version_string: "nuvio_engine_version_string" => fn() -> *const c_char;
    backend_version: "nuvio_engine_protocol_backend_version" => fn() -> *const c_char;
    status_message: "nuvio_engine_status_message" => fn(c_int) -> *const c_char;
    config_init: "nuvio_engine_config_init_sized" => fn(*mut Config, c_int);
    torrent_request_init: "nuvio_engine_torrent_request_init_sized" => fn(*mut TorrentRequest, c_int);
    event_init: "nuvio_engine_event_init_sized" => fn(*mut Event, c_int);
    file_init: "nuvio_engine_file_init_sized" => fn(*mut File, c_int);
    stream_request_init: "nuvio_engine_stream_request_init_sized" => fn(*mut StreamRequest, c_int);
    stats_init: "nuvio_engine_stats_init_sized" => fn(*mut Stats, c_int);
    stream_stats_init: "nuvio_engine_stream_stats_init_sized" => fn(*mut StreamStats, c_int);
    create: "nuvio_engine_create" => fn(*const Config, *mut Handle) -> c_int;
    destroy: "nuvio_engine_destroy" => fn(Handle);
    add_torrent: "nuvio_engine_add_torrent" => fn(Handle, *const TorrentRequest, *mut u64) -> c_int;
    poll_event: "nuvio_engine_poll_event" => fn(Handle, *mut Event) -> c_int;
    get_file_count: "nuvio_engine_get_file_count" => fn(Handle, *const c_char, *mut usize) -> c_int;
    get_file: "nuvio_engine_get_file" => fn(Handle, *const c_char, usize, *mut File) -> c_int;
    prepare_stream: "nuvio_engine_prepare_stream" => fn(Handle, *const StreamRequest, *mut u64) -> c_int;
    remove_torrent: "nuvio_engine_remove_torrent" => fn(Handle, *const c_char, *mut u64) -> c_int;
    stop_stream: "nuvio_engine_stop_stream" => fn(Handle, *const c_char, *mut u64) -> c_int;
    get_stats: "nuvio_engine_get_stats" => fn(Handle, *mut Stats) -> c_int;
    get_stream_stats: "nuvio_engine_get_stream_stats" => fn(Handle, *const c_char, *mut StreamStats) -> c_int;
    reclaim_disk_cache: "nuvio_engine_reclaim_disk_cache" => fn(Handle, u64, *mut u64) -> c_int;
}

/// A zeroed struct initialized by the engine (`*_init_sized` with the struct's own size).
pub fn init<T>(f: unsafe extern "C" fn(*mut T, c_int)) -> T {
    // SAFETY: every engine struct is plain data (integers, byte arrays, nullable pointers), so the
    // all-zero value is valid; the init function then writes its defaults within `size_of::<T>()`.
    let mut v: T = unsafe { std::mem::zeroed() };
    unsafe { f(&mut v, std::mem::size_of::<T>() as c_int) };
    v
}

pub fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// # Safety
/// `p` must be null or a NUL-terminated string owned by the engine (static strings).
pub unsafe fn c_text(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}
