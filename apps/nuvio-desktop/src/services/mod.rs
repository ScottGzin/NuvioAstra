//! Application services: data for the UI, built on `nuvio-core` and `nuvio-store`.
//! No UI code and no Tauri commands here (those live in `api.rs`).

pub mod addons;
pub mod badges;
pub mod collections;
pub mod debrid;
pub mod discord;
pub mod discover;
pub mod downloads;
pub mod enrich;
pub mod epg;
pub mod fonts;
pub mod home;
pub mod language_aliases;
pub mod livetv;
pub mod meta;
pub mod mdblist;
pub mod next_up;
pub mod parental;
pub mod playback;
pub mod progress;
pub mod search;
pub mod segments;
pub mod subtitles;
pub mod tmdb;
