//! TorBox: which torrents are already cached on its servers ("Instant") and the HTTP link of a
//! cached file. Port of the original's `TorboxApiClient`, `LocalDebridService`,
//! `LocalDebridAddonStreamResolver` and `TorboxFileSelector`.
//!
//! The key is the user's own and goes only to TorBox, in the `Authorization` header (and in the
//! `token` query of `requestdl`, which TorBox requires). It never appears in an error text.

use crate::stream::StreamItem;
use crate::url::encode_path_segment;
use crate::CoreError;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Hashes per `checkcached` request.
const CHECK_BATCH: usize = 100;
/// Single-hash retries when TorBox answers with another (canonical) hash.
const ALIAS_RETRIES: usize = 12;

fn base_url() -> &'static str {
    option_env!("NUVIO_TORBOX_API_URL").unwrap_or("https://api.torbox.app")
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TorboxError {
    #[error("A chave do TorBox foi recusada. Confira em Configurações → TorBox.")]
    Unauthorized,
    #[error("Este torrent não está no cache do TorBox.")]
    NotCached,
    #[error("O TorBox não encontrou um vídeo desse episódio no torrent.")]
    NoFile,
    #[error("Sem conexão com o TorBox.")]
    Offline,
    #[error("O TorBox respondeu com erro ({0}).")]
    Failed(String),
}

impl From<reqwest::Error> for TorboxError {
    fn from(error: reqwest::Error) -> Self {
        match CoreError::from(error) {
            CoreError::HttpStatus { status: 401 | 403 } => TorboxError::Unauthorized,
            CoreError::HttpStatus { status } => TorboxError::Failed(status.to_string()),
            _ => TorboxError::Offline,
        }
    }
}

/// What TorBox says about a cached torrent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cached {
    pub name: Option<String>,
    pub size: Option<u64>,
}

/// A playable file of a cached torrent.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub url: String,
    pub filename: Option<String>,
    pub size: Option<u64>,
}

/// What identifies the file wanted inside a torrent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Wanted {
    pub magnet: String,
    pub filename: Option<String>,
    pub file_idx: Option<i64>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
}

#[derive(Clone)]
pub struct Torbox {
    http: reqwest::Client,
}

impl Torbox {
    pub fn new() -> Result<Self, CoreError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http })
    }

    async fn send(&self, request: reqwest::RequestBuilder, key: &str) -> Result<(u16, Value), TorboxError> {
        let response = request.bearer_auth(key).send().await?;
        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return Err(TorboxError::Unauthorized);
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        Ok((status, body))
    }

    fn data(status: u16, body: &Value) -> Result<&Value, TorboxError> {
        if (200..300).contains(&status) && body.get("success").and_then(Value::as_bool) != Some(false) {
            return Ok(body.get("data").unwrap_or(&Value::Null));
        }
        let detail = body.get("error").and_then(Value::as_str).unwrap_or_default();
        Err(TorboxError::Failed(if detail.is_empty() { status.to_string() } else { format!("{status} {detail}") }))
    }

    /// The account the key belongs to (its e-mail), which also validates the key.
    pub async fn account(&self, key: &str) -> Result<String, TorboxError> {
        let (status, body) = self.send(self.http.get(format!("{}/v1/api/user/me", base_url())), key).await?;
        let data = Self::data(status, &body)?;
        Ok(data.get("email").and_then(Value::as_str).unwrap_or("conta TorBox").to_string())
    }

    async fn check_batch(&self, key: &str, hashes: &[String]) -> Result<HashMap<String, Cached>, TorboxError> {
        let url = format!("{}/v1/api/torrents/checkcached?format=object", base_url());
        let (status, body) = self.send(self.http.post(url).json(&serde_json::json!({ "hashes": hashes })), key).await?;
        let data = Self::data(status, &body)?;
        Ok(data
            .as_object()
            .into_iter()
            .flatten()
            .map(|(hash, item)| {
                let cached = Cached {
                    name: item.get("name").and_then(Value::as_str).map(str::to_string),
                    size: item.get("size").and_then(Value::as_u64),
                };
                (hash.to_lowercase(), cached)
            })
            .collect())
    }

    /// Which of `hashes` TorBox has cached. TorBox answers an alias hit with its canonical hash, so
    /// unmatched hashes are asked alone (`resolveTorboxAliasMatches`).
    pub async fn check_cached(&self, key: &str, hashes: &[String]) -> Result<HashMap<String, Cached>, TorboxError> {
        let mut wanted: Vec<String> = hashes.iter().map(|h| h.trim().to_lowercase()).filter(|h| !h.is_empty()).collect();
        wanted.sort();
        wanted.dedup();
        let mut found = HashMap::new();
        let mut aliased = false;
        for batch in wanted.chunks(CHECK_BATCH) {
            let answer = self.check_batch(key, batch).await?;
            aliased |= answer.keys().any(|h| !batch.contains(h));
            if batch.len() == 1 {
                if let Some(item) = answer.into_values().next() {
                    found.insert(batch[0].clone(), item);
                }
                continue;
            }
            found.extend(answer.into_iter().filter(|(h, _)| batch.contains(h)));
        }
        if aliased {
            let missing: Vec<String> = wanted.iter().filter(|h| !found.contains_key(*h)).take(ALIAS_RETRIES).cloned().collect();
            for hash in missing {
                if let Some(item) = self.check_batch(key, std::slice::from_ref(&hash)).await?.into_values().next() {
                    found.insert(hash, item);
                }
            }
        }
        Ok(found)
    }

    /// Adds a cached torrent to the account and returns the HTTP link of the wanted file.
    pub async fn resolve(&self, key: &str, wanted: &Wanted) -> Result<Resolved, TorboxError> {
        let boundary = format!("NuvioAstra{:x}", wanted.magnet.len() * 2654435761 % 4294967291);
        let mut form = String::new();
        for (name, value) in [("magnet", wanted.magnet.as_str()), ("add_only_if_cached", "true"), ("allow_zip", "false")] {
            form.push_str(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"));
        }
        form.push_str(&format!("--{boundary}--\r\n"));
        let create = self
            .http
            .post(format!("{}/v1/api/torrents/createtorrent", base_url()))
            .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
            .body(form);
        let (status, body) = self.send(create, key).await?;
        if status == 409 || body.get("error").and_then(Value::as_str).is_some_and(|e| e.to_ascii_uppercase().contains("NOT_CACHED")) {
            return Err(TorboxError::NotCached);
        }
        let data = Self::data(status, &body)?;
        let id = data.get("torrent_id").or_else(|| data.get("id")).and_then(Value::as_i64).ok_or(TorboxError::NotCached)?;

        let list = self.http.get(format!("{}/v1/api/torrents/mylist?id={id}&bypass_cache=true", base_url()));
        let (status, body) = self.send(list, key).await?;
        let data = Self::data(status, &body)?;
        let files: Vec<File> = data.get("files").and_then(Value::as_array).into_iter().flatten().filter_map(File::from_json).collect();
        let file = select_file(&files, wanted).ok_or(TorboxError::NoFile)?;

        let link = self.http.get(format!(
            "{}/v1/api/torrents/requestdl?token={}&torrent_id={id}&file_id={}&zip_link=false&redirect=false&append_name=false",
            base_url(),
            encode_path_segment(key),
            file.id
        ));
        let (status, body) = self.send(link, key).await?;
        let url = Self::data(status, &body)?.as_str().filter(|u| u.starts_with("http")).ok_or(TorboxError::NoFile)?.to_string();
        Ok(Resolved { url, filename: Some(file.name.clone()).filter(|n| !n.is_empty()), size: file.size })
    }
}

#[derive(Debug, Clone, PartialEq)]
struct File {
    id: i64,
    name: String,
    mime: String,
    size: Option<u64>,
}

impl File {
    fn from_json(v: &Value) -> Option<Self> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Self {
            id: v.get("id")?.as_i64()?,
            name: text("name").or_else(|| text("short_name")).or_else(|| text("absolute_path")).unwrap_or_default(),
            mime: text("mimetype").unwrap_or_default(),
            size: v.get("size").and_then(Value::as_u64),
        })
    }

    fn playable(&self) -> bool {
        self.mime.to_ascii_lowercase().starts_with("video/") || has_video_extension(&self.name)
    }
}

pub fn has_video_extension(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".mp4", ".mkv", ".webm", ".avi", ".mov", ".m4v", ".ts", ".m2ts", ".wmv", ".flv"].iter().any(|e| lower.ends_with(e))
}

fn normalized(path: &str) -> String {
    path.trim().replace('\\', "/").trim_start_matches('/').to_string()
}

/// `sXXeYY` / `XxYY` as a whole token.
fn has_episode(name: &str, season: i64, episode: i64) -> bool {
    let lower = name.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let number_at = |mut i: usize| -> Option<(i64, usize)> {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        (i > start).then(|| (lower[start..i].parse().unwrap_or(-1), i))
    };
    for (i, &c) in bytes.iter().enumerate() {
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        let (s, next) = match (c, number_at(i + 1), number_at(i)) {
            (b's', Some(found), _) => found,
            (_, _, Some(found)) => found,
            _ => continue,
        };
        let Some(&sep) = bytes.get(next) else { continue };
        if s != season || !(sep == b'e' || sep == b'x') {
            continue;
        }
        if let Some((e, end)) = number_at(next + 1) {
            if e == episode && bytes.get(end).is_none_or(|b| !b.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

/// `selectDebridFile`: the named file, else the episode, else the index, else the largest video.
fn select_file<'a>(files: &'a [File], wanted: &Wanted) -> Option<&'a File> {
    let playable: Vec<&File> = files.iter().filter(|f| f.playable()).collect();
    if playable.is_empty() {
        return None;
    }
    if let Some(name) = wanted.filename.as_deref().map(normalized).filter(|n| !n.is_empty()) {
        let base = name.rsplit('/').next().unwrap_or(&name).to_lowercase();
        let found: Vec<&&File> = playable
            .iter()
            .filter(|f| {
                let path = normalized(&f.name).to_lowercase();
                path == name.to_lowercase() || path.rsplit('/').next() == Some(base.as_str())
            })
            .collect();
        if let [only] = found.as_slice() {
            return Some(only);
        }
    }
    if let (Some(season), Some(episode)) = (wanted.season, wanted.episode) {
        let found: Vec<&&File> = playable.iter().filter(|f| has_episode(normalized(&f.name).rsplit('/').next().unwrap_or_default(), season, episode)).collect();
        return match found.as_slice() {
            [only] => Some(only),
            [] => None,
            several => several.iter().max_by_key(|f| f.size.unwrap_or(0)).map(|f| **f),
        };
    }
    if let Some(index) = wanted.file_idx.and_then(|i| usize::try_from(i).ok()) {
        if let Some(file) = files.get(index).filter(|f| f.playable()) {
            return Some(file);
        }
    }
    playable.into_iter().max_by_key(|f| f.size.unwrap_or(0))
}

fn client_resolve_text<'a>(stream: &'a StreamItem, name: &str) -> Option<&'a str> {
    stream.client_resolve.as_ref()?.get(name)?.as_str().map(str::trim).filter(|v| !v.is_empty())
}

/// The torrent's info hash (40 hex or 32 base32 characters), lowercase.
pub fn info_hash(stream: &StreamItem) -> Option<String> {
    let from_magnet = |m: &str| -> Option<String> {
        let at = m.to_ascii_lowercase().find("xt=urn:btih:")? + 12;
        Some(m[at..].split('&').next()?.to_string())
    };
    let magnets = [stream.url.as_deref(), stream.external_url.as_deref(), client_resolve_text(stream, "magnetUri")];
    let hash = stream
        .info_hash
        .clone()
        .or_else(|| client_resolve_text(stream, "infoHash").map(str::to_string))
        .or_else(|| magnets.into_iter().flatten().filter(|m| m.to_ascii_lowercase().starts_with("magnet:")).find_map(from_magnet))?;
    let hash = hash.trim().to_lowercase();
    ((hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit())) || (hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_alphanumeric()))).then_some(hash)
}

/// A TorBox `clientResolve` stream of an addon (`isDirectDebridCandidate`).
pub fn is_torbox_client_stream(stream: &StreamItem) -> bool {
    let resolve = stream.client_resolve.as_ref();
    let text = |n: &str| resolve.and_then(|r| r.get(n)).and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase();
    text("type") == "debrid" && text("service") == "torbox" && resolve.and_then(|r| r.get("isCached")).and_then(Value::as_bool) == Some(true)
}

/// What to ask TorBox for (`DebridMagnetBuilder` + `toResolveMetadata`).
pub fn wanted(stream: &StreamItem, season: Option<i64>, episode: Option<i64>) -> Option<Wanted> {
    let existing = [stream.url.as_deref(), stream.external_url.as_deref(), client_resolve_text(stream, "magnetUri")]
        .into_iter()
        .flatten()
        .find(|m| m.to_ascii_lowercase().starts_with("magnet:"))
        .map(str::to_string);
    let filename = client_resolve_text(stream, "filename")
        .map(str::to_string)
        .or_else(|| stream.client_resolve.as_ref()?.pointer("/stream/raw/filename")?.as_str().map(str::to_string))
        .or_else(|| stream.behavior_hints.filename.clone());
    let magnet = match existing {
        Some(m) => m,
        None => {
            let mut m = format!("magnet:?xt=urn:btih:{}", info_hash(stream)?);
            if let Some(name) = filename.as_deref() {
                m.push_str(&format!("&dn={}", encode_path_segment(name)));
            }
            for tracker in stream.sources.iter().filter_map(|s| s.strip_prefix("tracker:").map(str::trim)).filter(|t| !t.is_empty()) {
                m.push_str(&format!("&tr={}", encode_path_segment(tracker)));
            }
            m
        }
    };
    let number = |n: &str| stream.client_resolve.as_ref()?.get(n)?.as_i64();
    Some(Wanted {
        magnet,
        filename,
        file_idx: stream.file_idx.or_else(|| number("fileIdx")),
        season: season.or_else(|| number("season")),
        episode: episode.or_else(|| number("episode")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::parse_streams;

    fn file(id: i64, name: &str, size: u64) -> File {
        File { id, name: name.into(), mime: String::new(), size: Some(size) }
    }

    #[test]
    fn picks_the_file_like_the_original() {
        let files = [file(1, "Show/Sample.mkv", 900), file(2, "Show/Show.S01E02.1080p.mkv", 500), file(3, "Show/Show.S01E12.mkv", 600), file(4, "notes.txt", 9)];
        let pick = |w: Wanted| select_file(&files, &w).map(|f| f.id);
        assert_eq!(pick(Wanted { season: Some(1), episode: Some(2), ..Default::default() }), Some(2), "not S01E12");
        assert_eq!(pick(Wanted { filename: Some("Show.S01E12.mkv".into()), ..Default::default() }), Some(3));
        assert_eq!(pick(Wanted { file_idx: Some(0), ..Default::default() }), Some(1));
        assert_eq!(pick(Wanted::default()), Some(1), "largest video");
        assert_eq!(pick(Wanted { season: Some(2), episode: Some(1), ..Default::default() }), None);
        assert!(has_episode("show 1x05 web", 1, 5) && !has_episode("show s01e050", 1, 5));
    }

    #[test]
    fn torrent_identity_of_addon_streams() {
        let payload = r#"{"streams":[
            {"infoHash":"ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD","fileIdx":3,"behaviorHints":{"filename":"A B.mkv"},"sources":["tracker:udp://t.test:1","dht:x"]},
            {"clientResolve":{"type":"debrid","service":"torbox","isCached":true,"infoHash":"1111111111111111111111111111111111111111","filename":"E.mkv","season":2,"episode":4}},
            {"url":"https://h.test/a.mkv"}
        ]}"#;
        let streams = parse_streams(payload, "a", "b", None).unwrap();
        let w = wanted(&streams[0], None, None).unwrap();
        assert_eq!(w.magnet, "magnet:?xt=urn:btih:abcdefabcdefabcdefabcdefabcdefabcdefabcd&dn=A%20B.mkv&tr=udp%3A%2F%2Ft.test%3A1");
        assert_eq!(w.file_idx, Some(3));
        assert!(is_torbox_client_stream(&streams[1]) && !is_torbox_client_stream(&streams[0]));
        let c = wanted(&streams[1], None, None).unwrap();
        assert_eq!((c.season, c.episode, c.filename.as_deref()), (Some(2), Some(4), Some("E.mkv")));
        assert_eq!(info_hash(&streams[2]), None);
    }
}
