//! Minimal runtime binding to `libmpv-2.dll` (the same DLL the current Nuvio app ships).
//!
//! The library is loaded with `libloading`, exactly like `player_bridge.cpp` loads it with
//! `LoadLibrary`, so no import library or build-time mpv SDK is needed. Only the client API calls
//! the player uses are bound. Values come from mpv's `client.h`.

use libloading::Library;
use std::ffi::{c_char, c_double, c_int, c_ulong, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::PlayerError;

#[repr(C)]
pub struct MpvHandle {
    _private: [u8; 0],
}

pub const FORMAT_NONE: c_int = 0;
pub const FORMAT_STRING: c_int = 1;
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_INT64: c_int = 4;
pub const FORMAT_DOUBLE: c_int = 5;
pub const FORMAT_NODE: c_int = 6;
pub const FORMAT_NODE_ARRAY: c_int = 7;

pub const EVENT_NONE: c_int = 0;
pub const EVENT_SHUTDOWN: c_int = 1;
pub const EVENT_LOG_MESSAGE: c_int = 2;
pub const EVENT_START_FILE: c_int = 6;
pub const EVENT_END_FILE: c_int = 7;
pub const EVENT_FILE_LOADED: c_int = 8;
pub const EVENT_VIDEO_RECONFIG: c_int = 17;
pub const EVENT_PLAYBACK_RESTART: c_int = 21;
pub const EVENT_PROPERTY_CHANGE: c_int = 22;

pub const END_FILE_EOF: c_int = 0;
pub const END_FILE_STOP: c_int = 2;
pub const END_FILE_QUIT: c_int = 3;
pub const END_FILE_ERROR: c_int = 4;

/// `mpv_node` (client.h): a tagged union; only strings and arrays are used here.
#[repr(C)]
pub struct MpvNode {
    pub u: MpvNodeValue,
    pub format: c_int,
}

#[repr(C)]
pub union MpvNodeValue {
    pub string: *mut c_char,
    pub list: *mut MpvNodeList,
    pub int64: i64,
    pub double: f64,
}

/// `mpv_node_list`.
#[repr(C)]
pub struct MpvNodeList {
    pub num: c_int,
    pub values: *mut MpvNode,
    pub keys: *mut *mut c_char,
}

#[repr(C)]
pub struct MpvEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct MpvEventProperty {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct MpvEventLogMessage {
    pub prefix: *const c_char,
    pub level: *const c_char,
    pub text: *const c_char,
    pub log_level: c_int,
}

#[repr(C)]
pub struct MpvEventEndFile {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

type FnCreate = unsafe extern "C" fn() -> *mut MpvHandle;
type FnInitialize = unsafe extern "C" fn(*mut MpvHandle) -> c_int;
type FnSetOptionString = unsafe extern "C" fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int;
type FnSetOption = unsafe extern "C" fn(*mut MpvHandle, *const c_char, c_int, *mut c_void) -> c_int;
type FnSetPropertyString = unsafe extern "C" fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int;
type FnSetProperty = unsafe extern "C" fn(*mut MpvHandle, *const c_char, c_int, *mut c_void) -> c_int;
type FnGetProperty = unsafe extern "C" fn(*mut MpvHandle, *const c_char, c_int, *mut c_void) -> c_int;
type FnGetPropertyString = unsafe extern "C" fn(*mut MpvHandle, *const c_char) -> *mut c_char;
type FnCommand = unsafe extern "C" fn(*mut MpvHandle, *mut *const c_char) -> c_int;
type FnObserveProperty = unsafe extern "C" fn(*mut MpvHandle, u64, *const c_char, c_int) -> c_int;
type FnWaitEvent = unsafe extern "C" fn(*mut MpvHandle, c_double) -> *mut MpvEvent;
type FnWakeup = unsafe extern "C" fn(*mut MpvHandle);
type FnTerminateDestroy = unsafe extern "C" fn(*mut MpvHandle);
type FnFree = unsafe extern "C" fn(*mut c_void);
type FnErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type FnClientApiVersion = unsafe extern "C" fn() -> c_ulong;
type FnRequestLogMessages = unsafe extern "C" fn(*mut MpvHandle, *const c_char) -> c_int;

/// Function table resolved from the DLL. Kept alive by `Arc` for as long as any handle exists.
pub struct MpvApi {
    _library: Library,
    pub path: PathBuf,
    create: FnCreate,
    initialize: FnInitialize,
    set_option_string: FnSetOptionString,
    set_option: FnSetOption,
    set_property_string: FnSetPropertyString,
    set_property: FnSetProperty,
    get_property: FnGetProperty,
    get_property_string: FnGetPropertyString,
    command: FnCommand,
    observe_property: FnObserveProperty,
    wait_event: FnWaitEvent,
    wakeup: FnWakeup,
    terminate_destroy: FnTerminateDestroy,
    free: FnFree,
    error_string: FnErrorString,
    client_api_version: FnClientApiVersion,
    request_log_messages: FnRequestLogMessages,
}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        // SAFETY: the symbol type matches the C prototype in mpv's client.h.
        *unsafe { $lib.get(concat!($name, "\0").as_bytes()) }
            .map_err(|e| PlayerError::Library(format!("missing symbol {}: {e}", $name)))?
    };
}

impl MpvApi {
    pub fn load(path: &Path) -> Result<Arc<Self>, PlayerError> {
        // SAFETY: loading libmpv runs its DLL initialization, which has no preconditions.
        let library = unsafe { Library::new(path) }
            .map_err(|e| PlayerError::Library(format!("cannot load {}: {e}", path.display())))?;
        let api = Self {
            create: sym!(library, "mpv_create"),
            initialize: sym!(library, "mpv_initialize"),
            set_option_string: sym!(library, "mpv_set_option_string"),
            set_option: sym!(library, "mpv_set_option"),
            set_property_string: sym!(library, "mpv_set_property_string"),
            set_property: sym!(library, "mpv_set_property"),
            get_property: sym!(library, "mpv_get_property"),
            get_property_string: sym!(library, "mpv_get_property_string"),
            command: sym!(library, "mpv_command"),
            observe_property: sym!(library, "mpv_observe_property"),
            wait_event: sym!(library, "mpv_wait_event"),
            wakeup: sym!(library, "mpv_wakeup"),
            terminate_destroy: sym!(library, "mpv_terminate_destroy"),
            free: sym!(library, "mpv_free"),
            error_string: sym!(library, "mpv_error_string"),
            client_api_version: sym!(library, "mpv_client_api_version"),
            request_log_messages: sym!(library, "mpv_request_log_messages"),
            path: path.to_path_buf(),
            _library: library,
        };
        Ok(Arc::new(api))
    }

    /// `(major, minor)` of the client API.
    pub fn client_api_version(&self) -> (u32, u32) {
        // SAFETY: no arguments, returns a plain integer.
        let v = unsafe { (self.client_api_version)() } as u32;
        (v >> 16, v & 0xFFFF)
    }

    fn error_text(&self, code: c_int) -> String {
        // SAFETY: mpv returns a static NUL-terminated string for any error code.
        unsafe { CStr::from_ptr((self.error_string)(code)) }.to_string_lossy().into_owned()
    }
}

/// One mpv core. All calls are thread-safe per the client API; the handle is destroyed once.
pub struct Mpv {
    api: Arc<MpvApi>,
    handle: *mut MpvHandle,
}

// SAFETY: the mpv client API is fully thread-safe for a handle (documented in client.h).
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

fn cstr(value: &str) -> Result<CString, PlayerError> {
    CString::new(value).map_err(|_| PlayerError::InvalidArgument(format!("NUL byte in {value:?}")))
}

impl Mpv {
    pub fn create(api: Arc<MpvApi>) -> Result<Self, PlayerError> {
        // libmpv 2.5 refuses to initialise when LC_NUMERIC is not "C" (GTK sets the user locale).
        #[cfg(not(windows))]
        unsafe {
            libc::setlocale(libc::LC_NUMERIC, b"C\0".as_ptr().cast());
        }
        // SAFETY: plain constructor; null means failure.
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err(PlayerError::Mpv("mpv_create failed".into()));
        }
        Ok(Self { api, handle })
    }

    fn check(&self, what: &str, code: c_int) -> Result<(), PlayerError> {
        if code < 0 { Err(PlayerError::Mpv(format!("{what}: {}", self.api.error_text(code)))) } else { Ok(()) }
    }

    pub fn initialize(&self) -> Result<(), PlayerError> {
        // SAFETY: valid handle.
        self.check("mpv_initialize", unsafe { (self.api.initialize)(self.handle) })
    }

    pub fn set_option(&self, name: &str, value: &str) -> Result<(), PlayerError> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        // SAFETY: valid handle and NUL-terminated strings.
        self.check(name, unsafe { (self.api.set_option_string)(self.handle, n.as_ptr(), v.as_ptr()) })
    }

    pub fn set_option_i64(&self, name: &str, mut value: i64) -> Result<(), PlayerError> {
        let n = cstr(name)?;
        // SAFETY: FORMAT_INT64 expects a pointer to int64_t.
        self.check(name, unsafe {
            (self.api.set_option)(self.handle, n.as_ptr(), FORMAT_INT64, (&mut value as *mut i64).cast())
        })
    }

    /// Sets a string-list option from separate strings (`MPV_FORMAT_NODE_ARRAY`), so values with
    /// commas or backslashes need no escaping.
    pub fn set_option_string_list(&self, name: &str, values: &[String]) -> Result<(), PlayerError> {
        let n = cstr(name)?;
        let owned = values.iter().map(|v| cstr(v)).collect::<Result<Vec<_>, _>>()?;
        let mut nodes: Vec<MpvNode> = owned
            .iter()
            .map(|c| MpvNode { u: MpvNodeValue { string: c.as_ptr() as *mut c_char }, format: FORMAT_STRING })
            .collect();
        let mut list = MpvNodeList { num: nodes.len() as c_int, values: nodes.as_mut_ptr(), keys: std::ptr::null_mut() };
        let mut node = MpvNode { u: MpvNodeValue { list: &mut list }, format: FORMAT_NODE_ARRAY };
        // SAFETY: the node tree and its strings outlive the call; mpv copies the data.
        self.check(name, unsafe { (self.api.set_option)(self.handle, n.as_ptr(), FORMAT_NODE, (&mut node as *mut MpvNode).cast()) })
    }

    pub fn set_property(&self, name: &str, value: &str) -> Result<(), PlayerError> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        // SAFETY: valid handle and strings.
        self.check(name, unsafe { (self.api.set_property_string)(self.handle, n.as_ptr(), v.as_ptr()) })
    }

    pub fn set_property_f64(&self, name: &str, mut value: f64) -> Result<(), PlayerError> {
        let n = cstr(name)?;
        // SAFETY: FORMAT_DOUBLE expects a pointer to double.
        self.check(name, unsafe {
            (self.api.set_property)(self.handle, n.as_ptr(), FORMAT_DOUBLE, (&mut value as *mut f64).cast())
        })
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        let n = cstr(name).ok()?;
        let mut value = 0.0f64;
        // SAFETY: FORMAT_DOUBLE writes a double.
        let code = unsafe { (self.api.get_property)(self.handle, n.as_ptr(), FORMAT_DOUBLE, (&mut value as *mut f64).cast()) };
        (code >= 0).then_some(value)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let n = cstr(name).ok()?;
        let mut value = 0i64;
        // SAFETY: FORMAT_INT64 writes an int64_t.
        let code = unsafe { (self.api.get_property)(self.handle, n.as_ptr(), FORMAT_INT64, (&mut value as *mut i64).cast()) };
        (code >= 0).then_some(value)
    }

    pub fn get_flag(&self, name: &str) -> Option<bool> {
        let n = cstr(name).ok()?;
        let mut value: c_int = 0;
        // SAFETY: FORMAT_FLAG writes an int.
        let code = unsafe { (self.api.get_property)(self.handle, n.as_ptr(), FORMAT_FLAG, (&mut value as *mut c_int).cast()) };
        (code >= 0).then_some(value != 0)
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        let n = cstr(name).ok()?;
        // SAFETY: returns an mpv-allocated string or null; freed with mpv_free.
        unsafe {
            let raw = (self.api.get_property_string)(self.handle, n.as_ptr());
            if raw.is_null() {
                return None;
            }
            let value = CStr::from_ptr(raw).to_string_lossy().into_owned();
            (self.api.free)(raw.cast());
            Some(value)
        }
    }

    pub fn command(&self, args: &[&str]) -> Result<(), PlayerError> {
        let owned = args.iter().map(|a| cstr(a)).collect::<Result<Vec<_>, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: NULL-terminated array of NUL-terminated strings, alive for the call.
        self.check(args.first().copied().unwrap_or("command"), unsafe { (self.api.command)(self.handle, ptrs.as_mut_ptr()) })
    }

    pub fn observe(&self, id: u64, name: &str, format: c_int) -> Result<(), PlayerError> {
        let n = cstr(name)?;
        // SAFETY: valid handle; mpv copies the name.
        self.check(name, unsafe { (self.api.observe_property)(self.handle, id, n.as_ptr(), format) })
    }

    /// Waits for the next event. The returned pointer is valid until the next call on this handle.
    ///
    /// # Safety
    /// Only one thread may wait for events on a handle.
    pub unsafe fn wait_event(&self, timeout_seconds: f64) -> *mut MpvEvent {
        unsafe { (self.api.wait_event)(self.handle, timeout_seconds) }
    }

    /// Enables log messages at `level` ("v", "debug"…) as EVENT_LOG_MESSAGE. Diagnostics only:
    /// mpv log lines can contain URLs, so they must never be shown for addon streams.
    pub fn request_log_messages(&self, level: &str) -> Result<(), PlayerError> {
        let l = cstr(level)?;
        // SAFETY: valid handle and string.
        self.check("mpv_request_log_messages", unsafe { (self.api.request_log_messages)(self.handle, l.as_ptr()) })
    }

    pub fn wakeup(&self) {
        // SAFETY: valid handle; wakes a thread blocked in mpv_wait_event.
        unsafe { (self.api.wakeup)(self.handle) }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // SAFETY: last owner; destroys the core and waits for its threads, releasing the window.
        unsafe { (self.api.terminate_destroy)(self.handle) }
    }
}
