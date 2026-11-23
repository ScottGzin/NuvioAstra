//! Nuvio Astra account and cloud sync, compatible with Nuvio Desktop's Supabase backend.
//! Architecture and rules: `docs/ACCOUNT_SYNC.md`.

pub mod crypto;
pub mod engine;
mod home;
mod preferences;
pub mod remote;

pub use engine::{AccountError, Avatar, Compare, Engine, PinResult, Preview, ProfilePreview, Report, Status};
pub use remote::{RemoteError, ServerConfig};
