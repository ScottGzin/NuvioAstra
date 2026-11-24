//! Addon stream responses (`/stream/{type}/{id}.json`).
//!
//! Port of `features/streams/StreamParser.kt` and the playback helpers of `StreamModels.kt`:
//! entries without any source are dropped, `description` falls back to `title`, proxy headers
//! keep only non-blank values and mark the stream `notWebReady`, `type` is lowercased.
//! `clientResolve` (debrid resolution data) is kept verbatim as JSON so nothing is lost before
//! the debrid module is ported.

use crate::manifest::{optional_string, string_list};
use crate::CoreError;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct StreamItem {
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub info_hash: Option<String>,
    pub file_idx: Option<i64>,
    pub external_url: Option<String>,
    pub yt_id: Option<String>,
    pub sources: Vec<String>,
    pub addon_name: String,
    pub addon_id: String,
    pub addon_logo: Option<String>,
    pub stream_type: Option<String>,
    pub behavior_hints: StreamBehaviorHints,
    /// Subtitles the stream itself carries (HTTP(S) only).
    pub subtitles: Vec<crate::subtitle::SubtitleItem>,
    /// Raw `clientResolve` object, preserved for the debrid port.
    pub client_resolve: Option<Value>,
    /// Provider id forced by a duplicated source card (e.g. `torbox` or `realdebrid`).
    pub forced_debrid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct StreamBehaviorHints {
    pub binge_group: Option<String>,
    pub not_web_ready: bool,
    pub video_hash: Option<String>,
    pub video_size: Option<i64>,
    pub filename: Option<String>,
    pub proxy_headers: Option<StreamProxyHeaders>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct StreamProxyHeaders {
    pub request: Option<BTreeMap<String, String>>,
    pub response: Option<BTreeMap<String, String>>,
}

fn is_magnet(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("magnet:"))
}

fn is_torrent_scheme(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("torrent://"))
}

impl StreamItem {
    /// `directPlaybackUrl`.
    pub fn direct_playback_url(&self) -> Option<&str> {
        self.url.as_deref().map(str::trim).filter(|u| !u.is_empty())
    }

    /// `playableDirectUrl`: a URL a player can open directly (no magnet / torrent scheme).
    pub fn playable_direct_url(&self) -> Option<&str> {
        self.direct_playback_url().filter(|u| !is_magnet(Some(u)) && !is_torrent_scheme(Some(u)))
    }

    /// `isTorrentStream` (direct-debrid candidates excluded later, with the debrid port).
    pub fn is_torrent(&self) -> bool {
        self.info_hash.as_deref().is_some_and(|h| !h.trim().is_empty())
            || is_magnet(self.url.as_deref())
            || is_magnet(self.external_url.as_deref())
            || is_torrent_scheme(self.url.as_deref())
            || is_torrent_scheme(self.external_url.as_deref())
    }

    /// Request headers for the player (`sanitizePlaybackHeaders`: trimmed, blank and `Range` dropped).
    pub fn playback_request_headers(&self) -> Vec<(String, String)> {
        self.behavior_hints
            .proxy_headers
            .as_ref()
            .and_then(|h| h.request.as_ref())
            .map(|headers| {
                headers
                    .iter()
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .filter(|(k, v)| !k.is_empty() && !v.is_empty() && !k.eq_ignore_ascii_case("range"))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn string_map(obj: &Map<String, Value>) -> BTreeMap<String, String> {
    obj.iter()
        .filter_map(|(k, v)| match v {
            Value::String(s) if !s.trim().is_empty() => Some((k.clone(), s.clone())),
            Value::Number(n) => Some((k.clone(), n.to_string())),
            Value::Bool(b) => Some((k.clone(), b.to_string())),
            _ => None,
        })
        .collect()
}

fn int_like(obj: &Map<String, Value>, name: &str) -> Option<i64> {
    match obj.get(name)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn parse_streams(payload: &str, addon_name: &str, addon_id: &str, addon_logo: Option<&str>) -> Result<Vec<StreamItem>, CoreError> {
    let root: Value = serde_json::from_str(payload)?;
    let Some(streams) = root.as_object().and_then(|r| r.get("streams")).and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    Ok(streams
        .iter()
        .filter_map(|element| {
            let obj = element.as_object()?;
            let s = |n: &str| optional_string(obj, n);
            let url = s("url");
            let info_hash = s("infoHash");
            let external_url = s("externalUrl");
            let yt_id = s("ytId").map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            let client_resolve = obj.get("clientResolve").filter(|v| v.is_object()).cloned();
            if url.is_none() && info_hash.is_none() && external_url.is_none() && yt_id.is_none() && client_resolve.is_none() {
                return None;
            }
            let hints = obj.get("behaviorHints").and_then(Value::as_object);
            let proxy_headers = hints
                .and_then(|h| h.get("proxyHeaders"))
                .and_then(Value::as_object)
                .and_then(|p| {
                    let pick = |n: &str| p.get(n).and_then(Value::as_object).map(string_map).filter(|m| !m.is_empty());
                    let (request, response) = (pick("request"), pick("response"));
                    (request.is_some() || response.is_some()).then_some(StreamProxyHeaders { request, response })
                });
            let hint = |n: &str| hints.and_then(|h| optional_string(h, n));
            Some(StreamItem {
                name: s("name"),
                title: s("title"),
                description: s("description").or_else(|| s("title")),
                url,
                info_hash,
                file_idx: int_like(obj, "fileIdx"),
                external_url,
                yt_id,
                sources: string_list(obj, "sources"),
                addon_name: addon_name.to_string(),
                addon_id: addon_id.to_string(),
                addon_logo: addon_logo.map(str::to_string),
                stream_type: s("type").map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()),
                behavior_hints: StreamBehaviorHints {
                    binge_group: hint("bingeGroup"),
                    not_web_ready: hints.and_then(|h| h.get("notWebReady")).and_then(Value::as_bool).unwrap_or(false)
                        || proxy_headers.is_some(),
                    video_hash: hint("videoHash"),
                    video_size: hints.and_then(|h| int_like(h, "videoSize")),
                    filename: hint("filename"),
                    proxy_headers,
                },
                subtitles: crate::subtitle::items(obj.get("subtitles"), "stream"),
                client_resolve,
                forced_debrid: None,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = r#"{"streams": [
        {"name": "HD", "title": "Live 1080p", "url": "https://cdn.test/live.m3u8", "type": " HLS ",
         "behaviorHints": {"bingeGroup": "g1", "videoSize": "123",
            "proxyHeaders": {"request": {"Referer": "https://ref.test", "Range": "bytes=0-", "X-Empty": " "}, "response": {}}}},
        {"name": "Torrent", "infoHash": "abc", "fileIdx": "2", "sources": ["tracker:udp://t", ""]},
        {"name": "Magnet", "url": "magnet:?xt=urn:btih:abc"},
        {"name": "Nothing playable"},
        {"name": "Debrid", "clientResolve": {"type": "debrid", "infoHash": "def", "stream": {"raw": {"size": 10}}}},
        "not an object"
    ]}"#;

    #[test]
    fn parses_streams_like_stream_parser_kt() {
        let streams = parse_streams(PAYLOAD, "Addon", "addon:x:https://a/manifest.json", None).unwrap();
        assert_eq!(streams.len(), 4); // no-source and non-object entries dropped
        let hd = &streams[0];
        assert_eq!(hd.description.as_deref(), Some("Live 1080p")); // falls back to title
        assert_eq!(hd.stream_type.as_deref(), Some("hls"));
        assert!(hd.behavior_hints.not_web_ready); // proxy headers imply notWebReady
        assert_eq!(hd.behavior_hints.video_size, Some(123));
        assert_eq!(hd.playable_direct_url(), Some("https://cdn.test/live.m3u8"));
        assert_eq!(hd.playback_request_headers(), vec![("Referer".to_string(), "https://ref.test".to_string())]);
        assert!(!hd.is_torrent());

        let torrent = &streams[1];
        assert!(torrent.is_torrent());
        assert_eq!(torrent.file_idx, Some(2));
        assert_eq!(torrent.sources, vec!["tracker:udp://t"]);

        let magnet = &streams[2];
        assert!(magnet.is_torrent());
        assert_eq!(magnet.playable_direct_url(), None);

        let debrid = &streams[3];
        assert_eq!(debrid.client_resolve.as_ref().and_then(|c| c.pointer("/stream/raw/size")).and_then(Value::as_i64), Some(10));
    }

    #[test]
    fn missing_streams_array_is_empty() {
        assert!(parse_streams("{}", "a", "b", None).unwrap().is_empty());
    }
}
