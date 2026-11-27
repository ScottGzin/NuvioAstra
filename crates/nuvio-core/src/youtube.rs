//! YouTube trailers, resolved in the app like the original (`InAppYouTubeExtractor` +
//! `TrailerExtractionPlatform.desktop.kt`): the watch page gives the InnerTube key and visitor
//! data; the `player` API is asked as the visionOS, Android and iOS clients; the best adaptive MP4
//! video + M4A audio (original audio track, visionOS preferred), else the best HLS master or
//! progressive format, is chosen; `googlevideo` URLs are probed with small range requests (and the
//! alternate servers of `mn`) before use. Nothing is fetched before the user asks for the trailer.

use crate::CoreError;
use serde_json::{json, Value};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const PREFERRED_SEPARATE_CLIENT: &str = "visionos";
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36";

struct Client {
    key: &'static str,
    id: &'static str,
    version: &'static str,
    user_agent: &'static str,
    priority: i32,
    context: fn() -> Value,
}

const CLIENTS: &[Client] = &[
    Client {
        key: "visionos",
        id: "101",
        version: "1.02",
        user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
        priority: 0,
        context: || json!({"clientName":"VISIONOS","clientVersion":"1.02","deviceMake":"Apple","deviceModel":"RealityDevice17,1","osName":"visionOS","osVersion":"26.5.23O471","hl":"en","gl":"US"}),
    },
    Client {
        key: "android",
        id: "3",
        version: "20.10.35",
        user_agent: "com.google.android.youtube/20.10.35 (Linux; U; Android 14; en_US) gzip",
        priority: 1,
        context: || json!({"clientName":"ANDROID","clientVersion":"20.10.35","osName":"Android","osVersion":"14","platform":"MOBILE","androidSdkVersion":34,"hl":"en","gl":"US"}),
    },
    Client {
        key: "ios",
        id: "5",
        version: "20.10.1",
        user_agent: "com.google.ios.youtube/20.10.1 (iPhone16,2; U; CPU iOS 17_4 like Mac OS X)",
        priority: 2,
        context: || json!({"clientName":"IOS","clientVersion":"20.10.1","deviceModel":"iPhone16,2","osName":"iPhone","osVersion":"17.4.0.21E219","platform":"MOBILE","hl":"en","gl":"US"}),
    },
];

/// What the player opens: one URL, or video + separate audio.
#[derive(Debug, Clone, PartialEq)]
pub struct TrailerSource {
    pub video_url: String,
    pub audio_url: Option<String>,
    /// `adaptive_separate`, `hls`, `combined_fallback`, `adaptive_video_only` (diagnostics only).
    pub mode: &'static str,
    pub height: i64,
}

#[derive(Debug, Clone)]
struct Candidate {
    client: &'static str,
    priority: i32,
    url: String,
    score: f64,
    has_n: bool,
    height: i64,
    ext: &'static str,
    default_audio: bool,
}

#[derive(Debug, Clone)]
struct Manifest {
    url: String,
    height: i64,
    bandwidth: i64,
}

/// `extractVideoId`: an 11-character id, or a youtube.com / youtu.be URL.
pub fn video_id(input: &str) -> Option<String> {
    let valid = |s: &str| s.len() == 11 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let t = input.trim();
    if valid(t) {
        return Some(t.to_string());
    }
    let without_scheme = t.split_once("://").map_or(t, |(_, r)| r);
    let without_fragment = without_scheme.split('#').next().unwrap_or("");
    let (host_path, query) = without_fragment.split_once('?').unwrap_or((without_fragment, ""));
    let (host, path) = host_path.split_once('/').unwrap_or((host_path, ""));
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if host.to_ascii_lowercase().ends_with("youtu.be") {
        if let Some(id) = segments.first().filter(|s| valid(s)) {
            return Some(id.to_string());
        }
    }
    if let Some(v) = query.split('&').find_map(|p| p.strip_prefix("v=")).filter(|v| valid(v)) {
        return Some(v.to_string());
    }
    if segments.len() >= 2 && ["embed", "shorts", "live"].contains(&segments[0]) && valid(segments[1]) {
        return Some(segments[1].to_string());
    }
    None
}

fn between<'a>(text: &'a str, start: &str) -> Option<&'a str> {
    let i = text.find(start)? + start.len();
    let rest = &text[i..];
    Some(&rest[..rest.find('"')?])
}

fn query_param<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    url.split_once('?')?.1.split('&').find_map(|p| p.strip_prefix(name)?.strip_prefix('='))
}

fn quality_label(label: Option<&str>) -> Option<i64> {
    let l = label?;
    let p = l.find('p')?;
    let digits: String = l[..p].chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    (2..=4).contains(&digits.len()).then(|| digits.parse().ok()).flatten()
}

fn num(v: &Value, k: &str) -> Option<f64> {
    match v.get(k)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// `sortCandidates`.
fn sort(items: &mut [Candidate]) {
    let container = |e: &str| match e {
        "mp4" | "m4a" => 0,
        "webm" => 1,
        _ => 2,
    };
    items.sort_by(|a, b| {
        (!a.default_audio)
            .cmp(&!b.default_audio)
            .then(b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.has_n.cmp(&b.has_n))
            .then(container(a.ext).cmp(&container(b.ext)))
            .then(a.priority.cmp(&b.priority))
    });
}

fn best(mut items: Vec<Candidate>, client: &str) -> Option<Candidate> {
    let same: Vec<Candidate> = items.iter().filter(|c| c.client == client).cloned().collect();
    if !same.is_empty() {
        items = same;
    }
    sort(&mut items);
    items.into_iter().next()
}

fn parse_hls_attributes(line: &str) -> Vec<(String, String)> {
    let raw = line.split_once(':').map_or("", |(_, r)| r);
    let mut out = Vec::new();
    let (mut key, mut value, mut in_key, mut quoted) = (String::new(), String::new(), true, false);
    for ch in raw.chars() {
        if in_key {
            if ch == '=' { in_key = false } else { key.push(ch) }
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
            continue;
        }
        if ch == ',' && !quoted {
            out.push((key.trim().to_string(), value.trim().to_string()));
            key.clear();
            value.clear();
            in_key = true;
            continue;
        }
        value.push(ch);
    }
    if !key.trim().is_empty() {
        out.push((key.trim().to_string(), value.trim().to_string()));
    }
    out
}

/// Best variant (height, bandwidth) of an HLS master playlist.
fn best_hls_variant(body: &str) -> Option<(i64, i64)> {
    let lines: Vec<&str> = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut best: Option<(i64, i64, i64)> = None;
    for (i, line) in lines.iter().enumerate() {
        if !line.starts_with("#EXT-X-STREAM-INF:") || lines.get(i + 1).is_none_or(|n| n.starts_with('#')) {
            continue;
        }
        let attrs = parse_hls_attributes(line);
        let get = |k: &str| attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
        let (w, h) = get("RESOLUTION").and_then(|r| r.split_once('x')).map_or((0, 0), |(w, h)| (w.parse().unwrap_or(0), h.parse().unwrap_or(0)));
        let bw = get("BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0);
        if best.is_none_or(|(bh, bb, bwid)| (h, bw, w) > (bh, bb, bwid)) {
            best = Some((h, bw, w));
        }
    }
    best.map(|(h, b, _)| (h, b))
}

pub struct Extractor {
    http: reqwest::Client,
}

impl Extractor {
    pub fn new() -> Result<Self, CoreError> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| CoreError::InvalidPayload(format!("HTTP client: {e}")))?;
        Ok(Self { http })
    }

    async fn text(&self, req: reqwest::RequestBuilder) -> Result<String, String> {
        let r = req.send().await.map_err(|e| e.to_string())?;
        let status = r.status();
        let body = r.text().await.map_err(|e| e.to_string())?;
        if status.is_success() { Ok(body) } else { Err(format!("HTTP {}", status.as_u16())) }
    }

    /// Resolves a trailer (id or URL) to what the player opens; `None`-like errors are in pt-BR.
    pub async fn resolve(&self, input: &str) -> Result<TrailerSource, String> {
        let id = video_id(input).ok_or("Endereço de trailer inválido.")?;
        let watch = self
            .text(
                self.http
                    .get(format!("https://www.youtube.com/watch?v={id}&hl=en"))
                    .header("accept-language", "en-US,en;q=0.9")
                    .header("user-agent", BROWSER_UA),
            )
            .await
            .map_err(|e| format!("YouTube indisponível ({e})"))?;
        let api_key = between(&watch, "\"INNERTUBE_API_KEY\":\"").ok_or("O YouTube não respondeu como esperado.")?.to_string();
        let visitor = between(&watch, "\"VISITOR_DATA\":\"").map(str::to_string);

        let (mut progressive, mut video, mut audio, mut manifests) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for c in CLIENTS {
            let mut req = self
                .http
                .post(format!("https://www.youtube.com/youtubei/v1/player?key={}", api_key.replace('%', "%25").replace('+', "%2B").replace('&', "%26").replace('=', "%3D")))
                .header("accept-language", "en-US,en;q=0.9")
                .header("content-type", "application/json")
                .header("origin", "https://www.youtube.com")
                .header("x-youtube-client-name", c.id)
                .header("x-youtube-client-version", c.version)
                .header("user-agent", c.user_agent);
            if let Some(v) = &visitor {
                req = req.header("x-goog-visitor-id", v);
            }
            let body = json!({
                "videoId": id, "contentCheckOk": true, "racyCheckOk": true,
                "context": {"client": (c.context)()},
                "playbackContext": {"contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS"}},
            });
            let Ok(text) = self.text(req.body(body.to_string())).await else { continue };
            let Ok(player) = serde_json::from_str::<Value>(&text) else { continue };
            let Some(sd) = player.get("streamingData") else { continue };
            if let Some(m) = sd.get("hlsManifestUrl").and_then(Value::as_str).filter(|m| !m.is_empty()) {
                manifests.push((c.priority, m.to_string()));
            }
            let candidate = |f: &Value, is_audio: bool| -> Option<Candidate> {
                let url = f.get("url")?.as_str()?.to_string();
                let mime = f.get("mimeType").and_then(Value::as_str).unwrap_or("");
                let bitrate = num(f, "bitrate").or_else(|| num(f, "averageBitrate")).unwrap_or(0.0);
                let height = num(f, "height").map(|h| h as i64).or_else(|| quality_label(f.get("qualityLabel").and_then(Value::as_str))).unwrap_or(0);
                let fps = num(f, "fps").unwrap_or(0.0);
                let webm = mime.contains("webm");
                Some(Candidate {
                    client: c.key,
                    priority: c.priority,
                    has_n: query_param(&url, "n").is_some_and(|n| !n.is_empty()),
                    url,
                    score: if is_audio { bitrate * 1_000_000.0 + num(f, "audioSampleRate").unwrap_or(0.0) } else { height as f64 * 1e9 + fps * 1e6 + bitrate },
                    height: if is_audio { 0 } else { height },
                    ext: if webm { "webm" } else if is_audio { "m4a" } else { "mp4" },
                    default_audio: !is_audio || f.get("audioTrack").and_then(|t| t.get("audioIsDefault")).and_then(Value::as_bool).unwrap_or(true),
                })
            };
            for f in sd.get("formats").and_then(Value::as_array).into_iter().flatten() {
                let mime = f.get("mimeType").and_then(Value::as_str).unwrap_or("");
                if mime.is_empty() || mime.contains("video/") {
                    progressive.extend(candidate(f, false));
                }
            }
            for f in sd.get("adaptiveFormats").and_then(Value::as_array).into_iter().flatten() {
                let mime = f.get("mimeType").and_then(Value::as_str).unwrap_or("");
                if mime.contains("video/") {
                    video.extend(candidate(f, false));
                } else if mime.contains("audio/") {
                    audio.extend(candidate(f, true));
                }
            }
        }
        if progressive.is_empty() && video.is_empty() && audio.is_empty() && manifests.is_empty() {
            return Err("Este trailer não está disponível para reprodução.".into());
        }

        let mut best_manifest: Option<Manifest> = None;
        for (_, url) in &manifests {
            let Ok(body) = self.text(self.http.get(url).header("user-agent", BROWSER_UA)).await else { continue };
            if let Some((height, bandwidth)) = best_hls_variant(&body) {
                if best_manifest.as_ref().is_none_or(|b| (height, bandwidth) > (b.height, b.bandwidth)) {
                    best_manifest = Some(Manifest { url: url.clone(), height, bandwidth });
                }
            }
        }
        sort(&mut progressive);
        let best_progressive = progressive.into_iter().next();
        let best_video = best(video.into_iter().filter(|c| c.ext == "mp4").collect(), PREFERRED_SEPARATE_CLIENT);
        let best_audio = best(audio.into_iter().filter(|c| c.ext == "m4a").collect(), PREFERRED_SEPARATE_CLIENT);

        // `buildPlaybackSource`.
        let manifest_h = best_manifest.as_ref().map(|m| m.height);
        let combined_is_manifest = manifest_h.is_some_and(|h| best_progressive.as_ref().is_none_or(|p| h > p.height));
        // Astra: libmpv does not play YouTube's HLS masters here (tested), so the separate MP4 + M4A
        // pair comes first and HLS is the fallback (the original prefers HLS when it is as tall).
        let prefer_manifest = best_video.is_none() && manifest_h.is_some();
        let combined = if combined_is_manifest { best_manifest.as_ref().map(|m| m.url.clone()) } else { best_progressive.as_ref().map(|p| p.url.clone()) };
        let separated_video = match (&best_video, prefer_manifest) {
            (Some(v), false) => self.reachable(&v.url).await,
            _ => None,
        };
        let separated_audio = match (&separated_video, &best_audio) {
            (Some(_), Some(a)) => self.reachable(&a.url).await,
            _ => None,
        };
        if let (Some(v), Some(a)) = (&separated_video, &separated_audio) {
            return Ok(TrailerSource { video_url: v.clone(), audio_url: Some(a.clone()), mode: "adaptive_separate", height: best_video.map_or(0, |b| b.height) });
        }
        if let Some(url) = match combined {
            Some(c) => self.reachable(&c).await,
            None => None,
        } {
            let (mode, height) = if combined_is_manifest { ("hls", manifest_h.unwrap_or(0)) } else { ("combined_fallback", best_progressive.map_or(0, |p| p.height)) };
            return Ok(TrailerSource { video_url: url, audio_url: None, mode, height });
        }
        if let Some(v) = separated_video {
            return Ok(TrailerSource { video_url: v, audio_url: None, mode: "adaptive_video_only", height: best_video.map_or(0, |b| b.height) });
        }
        Err("O YouTube não liberou nenhum formato reproduzível para este trailer.".into())
    }

    /// `resolveReachableUrlOrNull`: the URL, or an alternate `mn` server, that answers ranges.
    async fn reachable(&self, url: &str) -> Option<String> {
        if !url.contains("googlevideo.com") {
            return Some(url.to_string());
        }
        let mut candidates = vec![url.to_string()];
        let host = url.split_once("://").map(|(_, r)| r.split('/').next().unwrap_or("")).unwrap_or("");
        if let Some(mn) = query_param(url, "mn") {
            for (i, server) in mn.split("%2C").flat_map(|s| s.split(',')).map(str::trim).filter(|s| !s.is_empty()).enumerate() {
                let mut alt = host.to_string();
                if let Some(p) = alt.find("---") {
                    if alt.starts_with("rr") && alt[2..p].chars().all(|c| c.is_ascii_digit()) {
                        alt = format!("rr{}{}", i + 1, &alt[p..]);
                    }
                }
                if let Some(sn) = alt.find("sn-") {
                    let end = alt[sn..].find('.').map_or(alt.len(), |e| sn + e);
                    alt.replace_range(sn..end, server);
                }
                if alt != host {
                    candidates.push(url.replacen(host, &alt, 1));
                }
            }
        }
        candidates.dedup();
        for c in candidates {
            if self.probe(&c).await {
                return Some(c);
            }
        }
        None
    }

    async fn probe(&self, url: &str) -> bool {
        let size = query_param(url, "clen").and_then(|v| v.parse::<i64>().ok()).filter(|s| *s > 0);
        let ranges: Vec<(i64, i64)> = match size {
            Some(s) => {
                let mut r = vec![(0, 65_535.min(s - 1)), ((s - 65_536).max(0), s - 1)];
                r.dedup();
                r
            }
            None => vec![(0, 0)],
        };
        for (a, b) in ranges {
            let ok = self
                .http
                .get(url)
                .timeout(PROBE_TIMEOUT)
                .header("user-agent", BROWSER_UA)
                .header("accept-language", "en-US,en;q=0.9")
                .header("range", format!("bytes={a}-{b}"))
                .send()
                .await
                .map(|r| r.status().as_u16() == 206 || (size.is_none() && a == 0 && r.status().is_success()))
                .unwrap_or(false);
            if !ok {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------- trailer in a chosen language
// The addons' trailers carry no language. With a language chosen in Settings, "Assistir trailer"
// searches YouTube (only when clicked) for the title plus the usual words of that language's
// trailers ("trailer dublado", "bande annonce vf"…) and takes the best short result whose title
// names the movie.

/// Interface value → (search words, YouTube `hl`, `gl`).
pub fn trailer_language(code: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match code {
        "pt-BR" => ("trailer dublado", "pt-BR", "BR"),
        "pt-PT" => ("trailer português", "pt-PT", "PT"),
        "en" => ("official trailer", "en", "US"),
        "es" => ("tráiler español castellano", "es", "ES"),
        "es-419" => ("tráiler doblado latino", "es-419", "MX"),
        "fr" => ("bande annonce vf", "fr", "FR"),
        "de" => ("trailer deutsch", "de", "DE"),
        "it" => ("trailer italiano", "it", "IT"),
        "ja" => ("予告", "ja", "JP"),
        _ => return None,
    })
}

/// Words that mark a video in the language (dubbed) and words that mark another version.
fn language_marks(code: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match code {
        "pt-BR" => (&["dublado", "dublada", "dub"], &["legendado", "legendada", "subtitulado", "doblado", "vostfr"]),
        "pt-PT" => (&["portugues", "dobrado", "legendado"], &["dublado", "doblado"]),
        "en" => (&["official"], &["dublado", "legendado", "doblado", "subtitulado", "vf", "deutsch", "italiano"]),
        "es" => (&["espanol", "castellano"], &["latino", "subtitulado", "subtitulada", "dublado"]),
        "es-419" => (&["doblado", "latino", "latinoamerica"], &["subtitulado", "subtitulada", "castellano", "dublado"]),
        "fr" => (&["vf", "francais"], &["vostfr", "vost"]),
        "de" => (&["deutsch", "german"], &["omu"]),
        "it" => (&["italiano"], &["sottotitolato"]),
        "ja" => (&["予告", "吹替"], &[]),
        _ => (&[], &[]),
    }
}

fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect()
}

fn has_word(folded_title: &str, word: &str) -> bool {
    let word = fold(word);
    let word = word.trim();
    if word.chars().any(|c| !c.is_ascii()) {
        return folded_title.contains(word);
    }
    folded_title.split_whitespace().any(|w| w == word)
}

/// A JSON string value that starts right after `"key":"` in `text`, decoded.
fn json_string_at(text: &str, from: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return serde_json::from_str(&format!("\"{}\"", &text[from..i])).ok(),
            _ => i += 1,
        }
    }
    None
}

/// One search result: id, title and length in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub seconds: Option<u32>,
}

/// Video results of a YouTube results page (`ytInitialData`), in order.
pub fn parse_search(page: &str) -> Vec<SearchHit> {
    const MARK: &str = "\"videoRenderer\":{\"videoId\":\"";
    const TITLE: &str = "\"title\":{\"runs\":[{\"text\":\"";
    let mut out: Vec<SearchHit> = Vec::new();
    let mut at = 0;
    while let Some(found) = page[at..].find(MARK) {
        let start = at + found + MARK.len();
        let id: String = page[start..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        let end = page[start..].find(MARK).map_or(page.len(), |n| start + n);
        let block = &page[start..end.min(start + 6000)];
        let title = block.find(TITLE).and_then(|n| json_string_at(block, n + TITLE.len())).unwrap_or_default();
        let seconds = block.find("\"lengthText\":").and_then(|n| {
            let rest = &block[n..];
            let k = rest.find("\"simpleText\":\"")? + 14;
            let value = json_string_at(rest, k)?;
            value.split(':').try_fold(0u32, |acc, part| part.trim().parse::<u32>().ok().map(|v| acc * 60 + v))
        });
        if id.len() == 11 && !out.iter().any(|h| h.id == id) {
            out.push(SearchHit { id, title, seconds });
        }
        at = start;
        if out.len() >= 12 {
            break;
        }
    }
    out
}

/// The best hit: a short trailer or teaser (≤ 6 min), in the language (dubbed words count,
/// subtitled versions count against), of this title (its name or the year help; another year
/// means another movie of the series).
pub fn best_trailer(hits: &[SearchHit], name: &str, year: Option<&str>, language: &str) -> Option<String> {
    let name = fold(name);
    let tokens: Vec<&str> = name.split_whitespace().filter(|t| t.len() >= 3).collect();
    let (good, bad) = language_marks(language);
    hits.iter()
        .take(10)
        .enumerate()
        .filter(|(_, h)| h.seconds.is_none_or(|s| (20..=360).contains(&s)))
        .filter_map(|(i, h)| {
            let title = fold(&h.title);
            let trailer = ["trailer", "teaser", "trailers", "bande", "annonce", "trailer", "予告"].iter().any(|w| has_word(&title, w) || h.title.contains(w))
                || title.contains("trailer");
            if !trailer {
                return None;
            }
            let mut score: i32 = 10;
            score += 30 * good.iter().filter(|w| has_word(&title, w) || h.title.contains(*w)).count().min(2) as i32;
            score -= 40 * bad.iter().filter(|w| has_word(&title, w)).count().min(1) as i32;
            if name.trim().len() > 1 && title.contains(name.trim()) {
                score += 20;
            }
            let shared = tokens.iter().filter(|t| has_word(&title, t)).count();
            score += (10 * shared / tokens.len().max(1)) as i32;
            let years: Vec<&str> = title.split_whitespace().filter(|w| w.len() == 4 && (w.starts_with("19") || w.starts_with("20")) && w.chars().all(|c| c.is_ascii_digit())).collect();
            if let Some(year) = year {
                if years.contains(&year) {
                    score += 20;
                } else if !years.is_empty() {
                    score -= 30;
                }
            }
            Some((score * 10 - i as i32 * 3, h.id.clone()))
        })
        .max_by_key(|(score, _)| *score)
        .filter(|(score, _)| *score > 0)
        .map(|(_, id)| id)
}

impl Extractor {
    /// The trailer of `name` (`year` helps) in the given interface language, from YouTube's search.
    pub async fn search_trailer(&self, name: &str, year: Option<&str>, language: &str) -> Result<Option<String>, String> {
        let Some((words, hl, gl)) = trailer_language(language) else { return Ok(None) };
        let query = format!("{name} {} {words}", year.unwrap_or_default()).split_whitespace().collect::<Vec<_>>().join(" ");
        let page = self
            .text(
                self.http
                    .get("https://www.youtube.com/results")
                    .query(&[("search_query", query.as_str()), ("sp", "EgIQAQ=="), ("hl", hl), ("gl", gl)])
                    .header("accept-language", format!("{hl},{};q=0.8", hl.split('-').next().unwrap_or(hl)))
                    .header("user-agent", BROWSER_UA),
            )
            .await
            .map_err(|e| format!("YouTube indisponível ({e})"))?;
        let hits = parse_search(&page);
        if std::env::var_os("NUVIO_TRAILER_DEBUG").is_some() {
            for h in &hits {
                eprintln!("  {} {:?} {}", h.id, h.seconds, h.title);
            }
        }
        Ok(best_trailer(&hits, name, year, language))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ids() {
        for (input, want) in [
            ("dQw4w9WgXcQ", Some("dQw4w9WgXcQ")),
            ("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1", Some("dQw4w9WgXcQ")),
            ("https://youtu.be/dQw4w9WgXcQ", Some("dQw4w9WgXcQ")),
            ("youtube.com/embed/dQw4w9WgXcQ", Some("dQw4w9WgXcQ")),
            ("https://www.youtube.com/shorts/dQw4w9WgXcQ", Some("dQw4w9WgXcQ")),
            ("https://example.com/x", None),
            ("short", None),
        ] {
            assert_eq!(video_id(input).as_deref(), want, "{input}");
        }
    }

    #[test]
    fn hls_and_ordering() {
        let m = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100,RESOLUTION=640x360,CODECS=\"a,b\"\nv1.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=900,RESOLUTION=1920x1080\nv2.m3u8\n";
        assert_eq!(best_hls_variant(m), Some((1080, 900)));
        let c = |client, h, n, def| Candidate { client, priority: 0, url: String::new(), score: h as f64 * 1e9, has_n: n, height: h, ext: "mp4", default_audio: def };
        let b = best(vec![c("android", 1080, false, true), c("visionos", 720, false, true)], "visionos").unwrap();
        assert_eq!((b.client, b.height), ("visionos", 720), "preferred client first");
        let mut v = vec![c("ios", 720, true, true), c("ios", 720, false, true), c("ios", 1080, false, false)];
        sort(&mut v);
        assert_eq!((v[0].height, v[0].has_n), (720, false), "original audio first, then score, then no n");
        assert_eq!(quality_label(Some("1080p60")), Some(1080));
    }

    #[test]
    fn localized_trailer_search_results() {
        let page = r#"x"videoRenderer":{"videoId":"AAAAAAAAAAA","title":{"runs":[{"text":"Matrix (1999) - Trailer Dublado \u0026 HD"}]},"lengthText":{"accessibility":{},"simpleText":"2:10"}}
            "videoRenderer":{"videoId":"BBBBBBBBBBB","title":{"runs":[{"text":"Matrix explicado em 40 minutos"}]},"lengthText":{"simpleText":"41:02"}}
            "videoRenderer":{"videoId":"CCCCCCCCCCC","title":{"runs":[{"text":"Outro filme trailer"}]},"lengthText":{"simpleText":"1:59"}}"#;
        let hits = parse_search(page);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "Matrix (1999) - Trailer Dublado & HD");
        assert_eq!(hits[0].seconds, Some(130));
        assert_eq!(best_trailer(&hits, "The Matrix", Some("1999"), "pt-BR"), Some("AAAAAAAAAAA".into()));
        assert_eq!(best_trailer(&hits[1..2], "The Matrix", Some("1999"), "pt-BR"), None, "too long");
        let dub = |title: &str, id: &str| SearchHit { id: id.into(), title: title.into(), seconds: Some(140) };
        let pick = best_trailer(
            &[dub("Matrix (The Matrix 1999) - Trailer Legendado", "L"), dub("Matrix | Trailer dublado | 1999", "D"), dub("The Matrix (1999) Official Trailer", "E")],
            "The Matrix",
            Some("1999"),
            "pt-BR",
        );
        assert_eq!(pick.as_deref(), Some("D"), "dubbed wins over subtitled and English");
        let sequel = best_trailer(&[dub("DUNA 3 Tráiler Español Latino (2026)", "3"), dub("Duna: Parte Dos | Tráiler Oficial 2 | Doblado", "2")], "Dune: Part Two", Some("2024"), "es-419");
        assert_eq!(sequel.as_deref(), Some("2"), "another year is another movie");
        assert!(trailer_language("pt-BR").is_some() && trailer_language("original").is_none());
    }

    /// Real YouTube search (network): `cargo test -p nuvio-core live_trailer_search -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_trailer_search() {
        let yt = Extractor::new().unwrap();
        for (name, year, lang) in [("The Matrix", "1999", "pt-BR"), ("Inception", "2010", "pt-BR"), ("Dune: Part Two", "2024", "es-419"), ("Oppenheimer", "2023", "fr")] {
            let id = yt.search_trailer(name, Some(year), lang).await.unwrap();
            println!("{name} [{lang}] -> {id:?}");
            assert!(id.is_some(), "{name}");
        }
    }
}
