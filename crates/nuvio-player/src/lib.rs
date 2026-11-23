//! Embedded libmpv player for the desktop app.
//!
//! - [`mpv`]: runtime binding to libmpv (no build-time SDK), loaded with `libloading`.
//! - [`player`] (Windows): one playback session embedded in a Win32 host window.
//! - [`linux`] backend (phase 1 of the Linux port): API-compatible placeholder; video playback is
//!   implemented in phase 2 with X11/Wayland embedding. Keeping the types identical lets the
//!   desktop app and the UI work unmodified on both platforms.

pub mod mpv;

#[cfg(windows)]
pub mod player;
#[cfg(windows)]
pub mod win;

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::*;

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlayerError {
    #[error("libmpv: {0}")]
    Library(String),
    #[error("mpv: {0}")]
    Mpv(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

#[cfg(windows)]
pub use player::{
    DecoderPriority, LoadRequest, Player, PlayerCommand, PlayerSnapshot, ResizeMode, SnapshotSink, SubtitleStyle, Track,
    VideoConfig,
};

/// Finds the libmpv shared library on Windows: `NUVIO_LIBMPV` (full path) first, then next to the
/// executable.
#[cfg(windows)]
pub fn locate_libmpv() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NUVIO_LIBMPV").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    Some(exe_dir.join("libmpv-2.dll")).filter(|p| p.is_file())
}

/// Finds libmpv on Linux. `NUVIO_LIBMPV` overrides everything; otherwise the system soname is
/// used (resolved by the dynamic loader, e.g. `/usr/lib/x86_64-linux-gnu/libmpv.so.2`).
#[cfg(not(windows))]
pub fn locate_libmpv() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NUVIO_LIBMPV").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    const CANDIDATES: &[&str] = &[
        "libmpv.so.2",
        "/usr/lib/x86_64-linux-gnu/libmpv.so.2",
        "/usr/lib/libmpv.so.2",
        "/usr/local/lib/libmpv.so.2",
    ];
    CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file() || p.to_str() == Some("libmpv.so.2"))
}
