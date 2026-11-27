//! Application services: data for the UI, built on `nuvio-core` and `nuvio-store`.
//! No UI code and no Tauri commands here (those live in `api.rs`).

pub mod addons;
pub mod collections;
pub mod enrich;
pub mod epg;
pub mod fonts;
pub mod home;
pub mod language_aliases;
pub mod livetv;
pub mod meta;
pub mod next_up;
pub mod parental;
pub mod playback;
pub mod plugins;
pub mod progress;
pub mod search;
pub mod segments;
pub mod subtitles;
pub mod m3u;
pub mod matches;
pub mod mdblist;
pub mod simkl;
pub mod tmdb;
