//! Nuvio Astra plugin contract (UI-free).
//!
//! Phase 1 port: the data model, repository manifest parsing and tolerant parsing of the JSON a
//! plugin returns. The QuickJS runtime and its host bridges come next; keeping the contract here
//! lets us test compatibility against real repositories before executing any JavaScript.

pub mod error;
pub mod host;
pub mod manifest;
pub mod model;
pub mod polyfill;
pub mod runtime;

pub use error::PluginError;
pub use model::{
    is_refresh_due, normalize_type, parse_runtime_results, PluginManifest, PluginManifestScraper,
    PluginRepositoryItem, PluginRuntimeResult, PluginScraper, PluginSubtitleResult, StoredPluginsState,
    REPOSITORY_REFRESH_INTERVAL_MS,
};
