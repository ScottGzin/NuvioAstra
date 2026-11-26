//! Data contract of Nuvio's local scraper plugins, ported from the original app's
//! `features/plugins/PluginModels.kt` and `runtime/PluginRuntime.kt`.
//!
//! This is the compatibility surface third-party repositories are written against. Field names and
//! the tolerant parsing of a plugin's JSON answer are deliberate: a plugin may return `quality: 720`
//! (number) or `"720p"` (string), and `url` may be a string or `{ "url": "..." }`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// `PLUGIN_REPOSITORY_REFRESH_INTERVAL_MS` in the original.
pub const REPOSITORY_REFRESH_INTERVAL_MS: i64 = 6 * 60 * 60 * 1_000;

fn default_supported_types() -> Vec<String> {
    vec!["movie".into(), "tv".into()]
}

/// `manifest.json` of a plugin repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub scrapers: Vec<PluginManifestScraper>,
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self { name: String::new(), version: String::new(), description: None, author: None, scrapers: Vec::new() }
    }
}

/// One `scrapers[]` entry of the repository manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginManifestScraper {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: String,
    pub filename: String,
    #[serde(default = "default_supported_types")]
    pub supported_types: Vec<String>,
    pub enabled: bool,
    pub has_settings: bool,
    pub logo: Option<String>,
    pub content_language: Vec<String>,
    pub supported_platforms: Option<Vec<String>>,
    pub disabled_platforms: Option<Vec<String>>,
    pub formats: Option<Vec<String>>,
    pub supported_formats: Option<Vec<String>>,
    pub supports_external_player: Option<bool>,
    pub limited: Option<bool>,
}

impl Default for PluginManifestScraper {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: None,
            version: String::new(),
            filename: String::new(),
            supported_types: default_supported_types(),
            enabled: true,
            has_settings: false,
            logo: None,
            content_language: Vec::new(),
            supported_platforms: None,
            disabled_platforms: None,
            formats: None,
            supported_formats: None,
            supports_external_player: None,
            limited: None,
        }
    }
}

/// A repository as shown in Settings → Plugins.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginRepositoryItem {
    pub manifest_url: String,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub scraper_count: usize,
    pub last_updated: i64,
}

impl Default for PluginRepositoryItem {
    fn default() -> Self {
        Self { manifest_url: String::new(), name: String::new(), description: None, version: None, scraper_count: 0, last_updated: 0 }
    }
}

/// `isPluginRepositoryRefreshDue`.
pub fn is_refresh_due(last_updated_epoch_ms: i64, now_epoch_ms: i64) -> bool {
    last_updated_epoch_ms <= 0 || now_epoch_ms - last_updated_epoch_ms >= REPOSITORY_REFRESH_INTERVAL_MS
}

/// `normalizePluginType`.
pub fn normalize_type(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "series" | "show" | "other" => "tv".to_string(),
        other => other.to_string(),
    }
}

/// An installed scraper, including the code downloaded from its `filename`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginScraper {
    pub id: String,
    pub repository_url: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub filename: String,
    pub supported_types: Vec<String>,
    pub enabled: bool,
    pub manifest_enabled: bool,
    pub has_settings: bool,
    pub logo: Option<String>,
    pub content_language: Vec<String>,
    pub formats: Option<Vec<String>>,
    pub code: String,
}

impl Default for PluginScraper {
    fn default() -> Self {
        Self {
            id: String::new(),
            repository_url: String::new(),
            name: String::new(),
            description: String::new(),
            version: String::new(),
            filename: String::new(),
            supported_types: default_supported_types(),
            enabled: true,
            manifest_enabled: true,
            has_settings: false,
            logo: None,
            content_language: Vec::new(),
            formats: None,
            code: String::new(),
        }
    }
}

impl PluginScraper {
    /// `PluginScraper.supportsType`.
    pub fn supports_type(&self, content_type: &str) -> bool {
        let wanted = normalize_type(content_type);
        self.supported_types.iter().any(|t| normalize_type(t) == wanted)
    }
}

/// The whole persisted plugin state (`StoredPluginsState` in the original).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoredPluginsState {
    pub plugins_enabled: bool,
    pub group_streams_by_repository: bool,
    pub repositories: Vec<PluginRepositoryItem>,
    pub scrapers: Vec<PluginScraper>,
}

impl Default for StoredPluginsState {
    fn default() -> Self {
        Self { plugins_enabled: true, group_streams_by_repository: false, repositories: Vec::new(), scrapers: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginSubtitleResult {
    pub url: String,
    pub language: String,
    pub name: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
}

impl Default for PluginSubtitleResult {
    fn default() -> Self {
        Self { url: String::new(), language: "Unknown".into(), name: None, headers: None }
    }
}

/// One stream returned by `getStreams` (`PluginRuntimeResult`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PluginRuntimeResult {
    pub title: String,
    pub name: Option<String>,
    pub url: String,
    pub quality: Option<String>,
    pub size: Option<String>,
    pub language: Option<String>,
    pub provider: Option<String>,
    #[serde(rename = "type")]
    pub stream_type: Option<String>,
    pub seeders: Option<i64>,
    pub peers: Option<i64>,
    pub info_hash: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub subtitles: Option<Vec<PluginSubtitleResult>>,
}

impl Default for PluginRuntimeResult {
    fn default() -> Self {
        Self {
            title: String::new(),
            name: None,
            url: String::new(),
            quality: None,
            size: None,
            language: None,
            provider: None,
            stream_type: None,
            seeders: None,
            peers: None,
            info_hash: None,
            headers: None,
            subtitles: None,
        }
    }
}

/// `JsonObject.stringOrNull`: numbers and booleans become text; blanks and "[object …]" are dropped.
fn string_or_null(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty() && !text.contains("[object")).then(|| text.to_string())
}

fn headers(value: Option<&Value>) -> Option<BTreeMap<String, String>> {
    let object = value?.as_object()?;
    let map: BTreeMap<String, String> = object.iter().filter_map(|(key, value)| string_or_null(Some(value)).map(|v| (key.clone(), v))).collect();
    (!map.is_empty()).then_some(map)
}

/// Mirrors `parseJsonResults`: drop non-objects and entries without a usable URL, keep everything
/// else as text so that numeric `quality`/`size` from real plugins do not fail the parse.
pub fn parse_runtime_results(raw: &str) -> Vec<PluginRuntimeResult> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(raw) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let url = string_or_null(object.get("url"))
                .or_else(|| string_or_null(object.get("url").and_then(|v| v.get("url"))))?;
            let name = string_or_null(object.get("name"));
            let subtitles: Vec<PluginSubtitleResult> = object
                .get("subtitles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|sub| {
                    let sub = sub.as_object()?;
                    Some(PluginSubtitleResult {
                        url: string_or_null(sub.get("url"))?,
                        language: string_or_null(sub.get("language")).unwrap_or_else(|| "Unknown".into()),
                        name: string_or_null(sub.get("name")),
                        headers: headers(sub.get("headers")),
                    })
                })
                .collect();
            Some(PluginRuntimeResult {
                title: string_or_null(object.get("title")).or_else(|| name.clone()).unwrap_or_else(|| "Desconhecido".into()),
                name,
                url,
                quality: string_or_null(object.get("quality")),
                size: string_or_null(object.get("size")),
                language: string_or_null(object.get("language")),
                provider: string_or_null(object.get("provider")),
                stream_type: string_or_null(object.get("type")),
                seeders: object.get("seeders").and_then(Value::as_i64),
                peers: object.get("peers").and_then(Value::as_i64),
                info_hash: string_or_null(object.get("infoHash")),
                headers: headers(object.get("headers")),
                subtitles: (!subtitles.is_empty()).then_some(subtitles),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_match_the_original() {
        assert_eq!(normalize_type("Series"), "tv");
        assert_eq!(normalize_type("SHOW"), "tv");
        assert_eq!(normalize_type("other"), "tv");
        assert_eq!(normalize_type("Movie"), "movie");
        let scraper = PluginScraper { supported_types: vec!["series".into()], ..Default::default() };
        assert!(scraper.supports_type("tv"));
        assert!(!scraper.supports_type("movie"));
    }

    #[test]
    fn refresh_uses_the_six_hour_window() {
        assert!(is_refresh_due(0, 1_000));
        assert!(!is_refresh_due(1_000, 1_000 + REPOSITORY_REFRESH_INTERVAL_MS - 1));
        assert!(is_refresh_due(1_000, 1_000 + REPOSITORY_REFRESH_INTERVAL_MS));
    }

    #[test]
    fn tolerant_result_parsing_matches_real_plugins() {
        let raw = r#"[
          {"title":"Kisskh HLS","name":"Kisskh HLS","url":"https://cdn/x.m3u8","quality":"Auto",
           "headers":{"Referer":"https://kisskh.ovh"},"provider":"kisskh"},
          {"name":"4K","url":{"url":"https://cdn/y.mkv"},"quality":2160,"size":123,"seeders":42,"infoHash":"abc",
           "subtitles":[{"url":"https://cdn/s.vtt","language":"ar","name":"Arabic"}]},
          {"name":"broken","url":"  "},
          {"name":"no url"},
          "not an object"
        ]"#;
        let parsed = parse_runtime_results(raw);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].title, "Kisskh HLS");
        assert_eq!(parsed[0].headers.as_ref().and_then(|h| h.get("Referer")).map(String::as_str), Some("https://kisskh.ovh"));
        assert_eq!(parsed[1].url, "https://cdn/y.mkv");
        assert_eq!(parsed[1].quality.as_deref(), Some("2160"), "numeric quality becomes text");
        assert_eq!(parsed[1].seeders, Some(42));
        assert_eq!(parsed[1].subtitles.as_ref().and_then(|s| s.first()).map(|s| s.language.as_str()), Some("ar"));
        assert!(parse_runtime_results("{}").is_empty());
    }
}
