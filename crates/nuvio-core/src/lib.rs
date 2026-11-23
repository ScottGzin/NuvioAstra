//! Nuvio Astra core.
//!
//! UI-free crate holding the Stremio addon protocol as implemented by the current Kotlin app:
//! URL building ([`url`]), manifests ([`manifest`]), catalog pages ([`catalog`]) and an HTTP
//! client with in-flight de-duplication ([`client`]), plus addon streams ([`stream`]). It is
//! UI-free so the Tauri desktop app (Windows x64) uses it directly through typed commands.

pub mod auto_play;
pub mod catalog;
pub mod downloads;
pub mod client;
pub mod error;
pub mod manifest;
pub mod meta;
pub mod shuffle;
pub mod skip;
pub mod stream;
pub mod subtitle;
pub mod url;
pub mod youtube;

pub use error::CoreError;
