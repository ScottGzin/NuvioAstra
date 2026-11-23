//! Pure download rules ported from NuvioDesktop's `DownloadsRepository` / `DownloadsModels`.
//! The actual HTTP transfer lives in the desktop service; this module owns naming, validation and
//! the item shape shared with the UI.

use serde::{Deserialize, Serialize};

pub const DEFAULT_EXTENSION: &str = "mp4";
pub const MAX_NAME_CHARS: usize = 92;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadStatus { Downloading, Paused, Completed, Failed }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadItem {
    pub id: String,
    pub content_type: String,
    pub parent_meta_id: String,
    pub parent_meta_type: String,
    pub video_id: String,
    pub title: String,
    pub season_number: Option<i64>,
    pub episode_number: Option<i64>,
    pub episode_title: Option<String>,
    pub stream_title: String,
    pub stream_subtitle: Option<String>,
    pub provider_name: String,
    pub source_url: String,
    pub file_name: String,
    pub local_file_uri: Option<String>,
    pub status: DownloadStatus,
    pub downloaded_bytes: i64,
    pub total_bytes: Option<i64>,
    pub error_message: Option<String>,
    pub created_at_epoch_ms: i64,
    pub updated_at_epoch_ms: i64,
}

impl Default for DownloadItem {
    fn default() -> Self {
        Self {
            id: String::new(), content_type: String::new(), parent_meta_id: String::new(), parent_meta_type: String::new(),
            video_id: String::new(), title: String::new(), season_number: None, episode_number: None, episode_title: None,
            stream_title: String::new(), stream_subtitle: None, provider_name: String::new(), source_url: String::new(),
            file_name: "download.mp4".into(), local_file_uri: None, status: DownloadStatus::Paused,
            downloaded_bytes: 0, total_bytes: None, error_message: None, created_at_epoch_ms: 0, updated_at_epoch_ms: 0,
        }
    }
}

impl DownloadItem {
    pub fn is_episode(&self) -> bool { self.season_number.is_some() && self.episode_number.is_some() }
    pub fn is_playable(&self) -> bool {
        self.status == DownloadStatus::Completed && self.local_file_uri.as_deref().is_some_and(|p| !p.trim().is_empty())
    }
    pub fn logical_content_key(&self) -> String {
        if self.is_episode() {
            format!("{}|{}|{}", self.parent_meta_id, self.season_number.unwrap(), self.episode_number.unwrap())
        } else {
            format!("{}|movie", self.parent_meta_id)
        }
    }
    pub fn progress_fraction(&self) -> f64 {
        match self.total_bytes {
            Some(total) if total > 0 => (self.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

/// `fileExtensionFromUrl`: suffix after the last `.` before `?`/`#`, 2–5 alphanumeric chars.
pub fn file_extension_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let name = path.rsplit('/').next().unwrap_or("");
    let Some((_, extension)) = name.rsplit_once('.') else { return DEFAULT_EXTENSION.into() };
    let valid = extension.len() >= 2 && extension.len() <= 5 && extension.chars().all(|c| c.is_ascii_alphanumeric());
    if valid { extension.to_ascii_lowercase() } else { DEFAULT_EXTENSION.into() }
}

/// `isSupportedDownloadUrl`: plain HTTP(S) files only; no torrents or adaptive manifests.
pub fn is_supported_download_url(url: &str) -> bool {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) { return false; }
    let lower = trimmed.to_ascii_lowercase();
    !lower.ends_with(".m3u8") && !lower.ends_with(".mpd") && !lower.ends_with(".torrent") && !lower.contains("magnet:")
}

fn sanitize_name(value: &str) -> String {
    let cleaned: String = value.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ' ' | '-') { c } else { '_' }).collect();
    let trimmed = cleaned.trim();
    let truncated: String = trimmed.chars().take(MAX_NAME_CHARS).collect();
    if truncated.trim().is_empty() { "download".into() } else { truncated }
}

/// `buildFileName`: `<title> S<SS>E<EE>[ <episode title>]_<id>.<ext>` or `<title>_<id>.<ext>`.
pub fn build_file_name(title: &str, season: Option<i64>, episode: Option<i64>, episode_title: Option<&str>, download_id: &str, url: &str) -> String {
    let base = if let (Some(season), Some(episode)) = (season, episode) {
        let mut value = format!("{} S{:02}E{:02}", title, season, episode);
        if let Some(name) = episode_title.filter(|n| !n.trim().is_empty()) {
            value.push(' ');
            value.push_str(name);
        }
        value
    } else if !title.trim().is_empty() {
        title.to_string()
    } else {
        "download".into()
    };
    format!("{}_{}.{}", sanitize_name(&base), sanitize_name(download_id), file_extension_from_url(url))
}

/// `nextDownloadId`: base-36 timestamp + ordinal, base-36.
pub fn next_download_id(now_epoch_ms: i64, ordinal: u64) -> String {
    format!("{}_{}", radix36(now_epoch_ms as u128), radix36(ordinal as u128))
}

fn radix36(mut value: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 { return "0".into(); }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("ascii digits")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_follow_the_original() {
        assert_eq!(build_file_name("Movie: One", None, None, None, "abc", "https://x/y.MKV?a=1"), "Movie_ One_abc.mkv");
        assert_eq!(build_file_name("Show", Some(2), Some(7), Some("The Episode!"), "abc", "https://x/y.mp4"), "Show S02E07 The Episode__abc.mp4");
        assert_eq!(file_extension_from_url("https://x/a.b/c"), "mp4");
        assert_eq!(file_extension_from_url("https://x/a.m3u8"), "m3u8");
        assert_eq!(file_extension_from_url("https://x/a.toolongext"), DEFAULT_EXTENSION);
        assert_eq!(next_download_id(36, 0), "10_0");
    }

    #[test]
    fn url_validation_and_item_rules() {
        assert!(is_supported_download_url("https://x.test/a.mp4"));
        assert!(!is_supported_download_url("magnet:?xt=urn:btih:1"));
        assert!(!is_supported_download_url("https://x.test/a.m3u8"));
        let mut item = DownloadItem {
            parent_meta_id: "tt1".into(), season_number: Some(1), episode_number: Some(2),
            source_url: "https://x/a.mp4".into(), status: DownloadStatus::Completed,
            local_file_uri: Some("/tmp/a.mp4".into()), downloaded_bytes: 50, total_bytes: Some(100), ..Default::default()
        };
        assert!(item.is_episode());
        assert!(item.is_playable());
        assert_eq!(item.logical_content_key(), "tt1|1|2");
        assert_eq!(item.progress_fraction(), 0.5);
        item.local_file_uri = None;
        assert!(!item.is_playable());
    }
}
