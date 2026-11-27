//! M3U playlists in Live TV (an addition of Nuvio Astra): the user adds the address of a playlist
//! (or a local `.m3u` file) and its channels appear next to the addons' ones, with the categories
//! of `group-title`, the logos of `tvg-logo` and the headers of `#EXTVLCOPT` / `#EXTHTTP`.
//!
//! The playlists are kept encrypted (IPTV addresses usually carry a user and a password) and the
//! channels' addresses stay in Rust: the interface only gets names, logos and keys.

use super::livetv::Channel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};



/// Channels a playlist may bring (a few huge IPTV lists have hundreds of thousands of VOD lines).
const MAX_CHANNELS: usize = 20_000;
/// Largest playlist read (bytes).
pub const MAX_BYTES: usize = 40 * 1024 * 1024;

/// A playlist the user added.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    /// `http(s)://…` or a local file path.
    pub source: String,
}

/// One channel of a playlist, with what playing it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub logo: Option<String>,
    pub group: Option<String>,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

fn attr(line: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = line.find(&needle)? + needle.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].trim().to_string()).filter(|v| !v.is_empty())
}

/// The display name: what follows the first comma outside the quoted attributes (the name itself
/// may have commas).
fn title(line: &str) -> String {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => return line[i + 1..].trim().to_string(),
            _ => {}
        }
    }
    String::new()
}

/// Parses an M3U / M3U8 playlist (extended or plain).
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut pending: Option<Entry> = None;
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut group_line: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let name = attr(rest, "tvg-name").filter(|_| title(rest).is_empty()).unwrap_or_else(|| title(rest));
            pending = Some(Entry { name, logo: attr(rest, "tvg-logo"), group: attr(rest, "group-title"), url: String::new(), headers: Vec::new() });
            headers.clear();
            group_line = None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTVLCOPT:") {
            if let Some((k, v)) = rest.split_once('=') {
                let name = match k.trim().to_lowercase().as_str() {
                    "http-user-agent" => "User-Agent",
                    "http-referrer" | "http-referer" => "Referer",
                    "http-origin" => "Origin",
                    _ => continue,
                };
                headers.push((name.into(), v.trim().to_string()));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTHTTP:") {
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(rest) {
                for (k, v) in map {
                    if let Some(v) = v.as_str() {
                        headers.push((k, v.to_string()));
                    }
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTGRP:") {
            group_line = Some(rest.trim().to_string()).filter(|g| !g.is_empty());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if !(line.starts_with("http://") || line.starts_with("https://") || line.starts_with("rtmp://") || line.starts_with("rtsp://")) {
            pending = None;
            continue;
        }
        let mut entry = pending.take().unwrap_or(Entry { name: String::new(), logo: None, group: None, url: String::new(), headers: Vec::new() });
        // `url|User-Agent=…&Referer=…` (Kodi style)
        let (url, kodi) = line.split_once('|').unwrap_or((line, ""));
        for pair in kodi.split('&').filter(|p| !p.is_empty()) {
            if let Some((k, v)) = pair.split_once('=') {
                headers.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
        entry.url = url.trim().to_string();
        if entry.name.is_empty() {
            entry.name = format!("Canal {}", out.len() + 1);
        }
        entry.group = entry.group.or_else(|| group_line.take());
        entry.headers = std::mem::take(&mut headers);
        out.push(entry);
        if out.len() >= MAX_CHANNELS {
            break;
        }
    }
    out
}

/// The `#EXTM3U` header (or at least one `#EXTINF`) tells a playlist from something else.
pub fn looks_like_playlist(text: &str) -> bool {
    let head: String = text.chars().take(4096).collect();
    head.trim_start_matches('\u{feff}').trim_start().starts_with("#EXTM3U") || head.contains("#EXTINF")
}

/// Channel key: `m3u:<playlist>|tv|<index>` (the addon channels' `<instance>|<type>|<id>` shape).
pub fn key(playlist: &str, index: usize) -> String {
    format!("m3u:{playlist}|tv|{index}")
}

pub fn split(key: &str) -> Option<(&str, usize)> {
    let rest = key.strip_prefix("m3u:")?;
    let (playlist, tail) = rest.split_once('|')?;
    let index = tail.strip_prefix("tv|")?.parse().ok()?;
    Some((playlist, index))
}

/// The playlist's channels as Live TV channels.
pub fn channels(playlist: &Playlist, entries: &[Entry]) -> Vec<Channel> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| Channel {
            key: key(&playlist.id, i),
            meta_id: i.to_string(),
            content_type: "tv".into(),
            name: e.name.clone(),
            logo: e.logo.clone().filter(|l| l.starts_with("http")),
            genres: e.group.clone().into_iter().collect(),
            addon_name: playlist.name.clone(),
        })
        .collect()
}

/// The channel as an addon `streams` answer (so it plays through the normal path).
pub fn stream_payload(entry: &Entry) -> String {
    let mut stream = json!({ "name": "M3U", "title": entry.name, "url": entry.url });
    if !entry.headers.is_empty() {
        let request: serde_json::Map<String, Value> = entry.headers.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        stream["behaviorHints"] = json!({ "notWebReady": true, "proxyHeaders": { "request": request } });
    }
    json!({ "streams": [stream] }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}#EXTM3U url-tvg=\"http://epg\"\n\
        #EXTINF:-1 tvg-id=\"a\" tvg-logo=\"https://l/a.png\" group-title=\"Notícias\",Canal A, HD\n\
        #EXTVLCOPT:http-user-agent=VLC/3\n\
        #EXTVLCOPT:http-referrer=https://site/\n\
        http://srv/a.m3u8\n\
        #EXTINF:-1 group-title=\"Esportes\",Canal B\n\
        #EXTGRP:Ignorado\n\
        https://srv/b.ts|User-Agent=Kodi&Referer=https://r/\n\
        #EXTINF:-1,Sem endereço\n\
        not-a-url\n\
        #EXTINF:-1 tvg-name=\"Nome TVG\",\n\
        #EXTHTTP:{\"Cookie\":\"x=1\"}\n\
        https://srv/c\n";

    #[test]
    fn parses_extended_playlists() {
        let list = parse(SAMPLE);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "Canal A, HD");
        assert_eq!(list[0].logo.as_deref(), Some("https://l/a.png"));
        assert_eq!(list[0].group.as_deref(), Some("Notícias"));
        assert_eq!(list[0].headers, vec![("User-Agent".into(), "VLC/3".into()), ("Referer".into(), "https://site/".into())]);
        assert_eq!(list[1].url, "https://srv/b.ts");
        assert_eq!(list[1].group.as_deref(), Some("Esportes"));
        assert_eq!(list[1].headers, vec![("User-Agent".into(), "Kodi".into()), ("Referer".into(), "https://r/".into())]);
        assert_eq!(list[2].name, "Nome TVG");
        assert_eq!(list[2].headers, vec![("Cookie".into(), "x=1".into())]);
        assert!(looks_like_playlist(SAMPLE));
        assert!(!looks_like_playlist("<html>"));
    }

    #[test]
    fn keys_and_payload() {
        assert_eq!(split(&key("p1", 7)), Some(("p1", 7)));
        assert_eq!(split("addon|tv|x"), None);
        let list = parse(SAMPLE);
        let payload: Value = serde_json::from_str(&stream_payload(&list[0])).unwrap();
        assert_eq!(payload["streams"][0]["url"], "http://srv/a.m3u8");
        assert_eq!(payload["streams"][0]["behaviorHints"]["proxyHeaders"]["request"]["User-Agent"], "VLC/3");
        let items = nuvio_core::stream::parse_streams(&stream_payload(&list[0]), "Lista", "m3u:p1", None).unwrap();
        assert_eq!(items[0].playback_request_headers().len(), 2);
    }
}
