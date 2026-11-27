//! Simkl (`SimklApiClient`, `SimklMutationRepository`, `SimklSyncRemote` of the original), on the
//! AUTH V2 API: the device sign-in (`/oauth2/device`, no secret: the app is a "TV, devices &
//! command line" client), token refresh and revoke, scrobbles, history marks and the list of what
//! was watched. The account (when to send, where the token lives) is `crate::simkl`. Tokens never
//! appear in an error text, a log or an event.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

/// Simkl API root; tests point it at a local fake server.
pub fn api_root() -> &'static str {
    option_env!("NUVIO_SIMKL_API_URL").unwrap_or("https://api.simkl.com")
}

/// `app-name` sent with every request (a short lowercase name, as the API rules ask).
const APP_NAME: &str = "nuvio-astra";

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("nuvio-astra/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("simkl http client")
    })
}

pub struct Answer {
    pub status: u16,
    pub body: Value,
}

impl Answer {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn error(&self) -> &str {
        self.body.get("error").and_then(Value::as_str).unwrap_or("")
    }
}

pub enum Body<'a> {
    None,
    Json(&'a Value),
    Form(&'a [(&'a str, &'a str)]),
}

/// One request: `client_id`, `app-name` and `app-version` go in the address of every call.
pub async fn request(method: reqwest::Method, client_id: &str, token: Option<&str>, path: &str, query: &[(&str, String)], body: Body<'_>) -> Result<Answer, String> {
    let mut url = reqwest::Url::parse(&format!("{}/{}", api_root(), path.trim_start_matches('/'))).map_err(|_| "Endereço do Simkl inválido.".to_string())?;
    {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            pairs.append_pair(k, v);
        }
        pairs.append_pair("client_id", client_id);
        pairs.append_pair("app-name", APP_NAME);
        pairs.append_pair("app-version", env!("CARGO_PKG_VERSION"));
    }
    let mut req = http().request(method, url).header("Accept", "application/json");
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    req = match body {
        Body::None => req,
        Body::Json(v) => req.header("Content-Type", "application/json").body(v.to_string()),
        Body::Form(pairs) => req.form(pairs),
    };
    let response = req.send().await.map_err(|e| if e.is_timeout() { "O Simkl demorou demais para responder.".to_string() } else { "Sem conexão com o Simkl.".to_string() })?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let body = if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::Null) };
    Ok(Answer { status, body })
}

/// The message of a failed answer (never with the address or the token).
pub fn failure(answer: &Answer) -> String {
    match (answer.status, answer.error()) {
        (412, _) => "O Simkl recusou o app do Nuvio Astra.".into(),
        (401, "user_token_required") | (401, "invalid_token") => "Conecte o Simkl de novo.".into(),
        (429, "user_limit_exceeded") => "Limite diário do Simkl atingido; volta a funcionar amanhã.".into(),
        (429, _) | (400, "RATE_LIMIT") => "Muitos pedidos ao Simkl; tente de novo em instantes.".into(),
        (404, _) => "Não encontrado no Simkl.".into(),
        (500..=599, _) => "O Simkl está fora do ar agora.".into(),
        (status, _) => format!("O Simkl respondeu com o código {status}."),
    }
}

pub async fn get(client_id: &str, token: Option<&str>, path: &str, query: &[(&str, String)]) -> Result<Answer, String> {
    let answer = request(reqwest::Method::GET, client_id, token, path, query, Body::None).await?;
    if answer.ok() { Ok(answer) } else { Err(failure(&answer)) }
}

// ---------------------------------------------------------------- sign-in (device flow, V2)

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds when the access token stops working.
    pub expires_at: i64,
}

impl Token {
    /// Refreshed a day before it expires (7-day access tokens).
    pub fn expiring(&self, now_secs: i64) -> bool {
        now_secs >= self.expires_at - 86_400
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn token_of(body: &Value) -> Option<Token> {
    Some(Token {
        access_token: body.get("access_token")?.as_str()?.to_string(),
        refresh_token: body.get("refresh_token").and_then(Value::as_str).unwrap_or("").to_string(),
        expires_at: now_secs() + body.get("expires_in").and_then(Value::as_i64).unwrap_or(604_800),
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    #[serde(skip)]
    pub device_code: String,
    pub user_code: String,
    /// The page with the code already filled in (`verification_uri_complete`).
    pub verification_url: String,
    pub expires_in: i64,
    pub interval: i64,
}

const SCOPE: &str = "media:read media:write";

pub async fn device_code(client_id: &str) -> Result<DeviceCode, String> {
    let form = [("client_id", client_id), ("scope", SCOPE)];
    let answer = request(reqwest::Method::POST, client_id, None, "oauth2/device", &[], Body::Form(&form)).await?;
    if !answer.ok() {
        return Err(failure(&answer));
    }
    let b = &answer.body;
    let text = |k: &str| b.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(DeviceCode {
        device_code: text("device_code").ok_or("Resposta do Simkl incompleta.")?,
        user_code: text("user_code").ok_or("Resposta do Simkl incompleta.")?,
        verification_url: text("verification_uri_complete").or_else(|| text("verification_uri")).unwrap_or_else(|| "https://simkl.com/pin".into()),
        expires_in: b.get("expires_in").and_then(Value::as_i64).unwrap_or(900),
        interval: b.get("interval").and_then(Value::as_i64).unwrap_or(5).max(1),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevicePoll {
    Authorized(Token),
    /// Waiting (Simkl also answers this while the user declines: the code just expires).
    Pending,
    SlowDown,
    Expired,
    Failed(String),
}

pub async fn device_poll(client_id: &str, device_code: &str) -> DevicePoll {
    let form = [("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("client_id", client_id), ("device_code", device_code)];
    let answer = match request(reqwest::Method::POST, client_id, None, "oauth2/token", &[], Body::Form(&form)).await {
        Ok(a) => a,
        Err(e) => return DevicePoll::Failed(e),
    };
    if answer.ok() {
        return token_of(&answer.body).map_or_else(|| DevicePoll::Failed("Resposta do Simkl incompleta.".into()), DevicePoll::Authorized);
    }
    match answer.error() {
        "authorization_pending" => DevicePoll::Pending,
        "slow_down" => DevicePoll::SlowDown,
        "expired_token" | "access_denied" => DevicePoll::Expired,
        _ => DevicePoll::Failed(failure(&answer)),
    }
}

/// `Ok(None)` when Simkl refused the refresh token (connect again).
pub async fn refresh(client_id: &str, refresh_token: &str) -> Result<Option<Token>, String> {
    let form = [("grant_type", "refresh_token"), ("client_id", client_id), ("refresh_token", refresh_token)];
    let answer = request(reqwest::Method::POST, client_id, None, "oauth2/token", &[], Body::Form(&form)).await?;
    if answer.ok() {
        let mut token = token_of(&answer.body).ok_or("Resposta do Simkl incompleta.")?;
        if token.refresh_token.is_empty() {
            token.refresh_token = refresh_token.to_string();
        }
        return Ok(Some(token));
    }
    match (answer.status, answer.error()) {
        (400, "invalid_grant") | (401, _) => Ok(None),
        _ => Err(failure(&answer)),
    }
}

pub async fn revoke(client_id: &str, token: &str) {
    let form = [("client_id", client_id), ("token", token)];
    let _ = request(reqwest::Method::POST, client_id, None, "oauth2/revoke", &[], Body::Form(&form)).await;
}

/// The signed-in user's name (`/users/settings`).
pub async fn username(client_id: &str, token: &str) -> Option<String> {
    let answer = get(client_id, Some(token), "users/settings", &[]).await.ok()?;
    answer.body.pointer("/user/name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).map(str::to_string)
}

// ---------------------------------------------------------------- ids, scrobble, history

/// Simkl ids of a title id: `tt…` is IMDb, `tmdb:<n>` TMDB.
pub fn ids_of(id: &str) -> Option<Value> {
    let head = id.trim().split(':').next().unwrap_or("");
    if head.starts_with("tt") && head.len() > 2 {
        return Some(json!({ "imdb": head }));
    }
    let tmdb = id.trim().strip_prefix("tmdb:")?.split(':').next()?.trim().parse::<i64>().ok()?;
    Some(json!({ "tmdb": tmdb }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scrobble {
    Start,
    Pause,
    Stop,
}

impl Scrobble {
    pub fn path(self) -> &'static str {
        match self {
            Self::Start => "scrobble/start",
            Self::Pause => "scrobble/pause",
            Self::Stop => "scrobble/stop",
        }
    }
}

/// `buildSimklScrobbleBody`: a movie, or the show and the episode (season, number).
pub fn scrobble_body(content_type: &str, id: &str, season: Option<i32>, episode: Option<i32>, progress: f64) -> Option<Value> {
    let ids = ids_of(id)?;
    let progress = (progress.clamp(0.0, 100.0) * 100.0).round() / 100.0;
    if content_type == "movie" {
        return Some(json!({ "progress": progress, "movie": { "ids": ids } }));
    }
    let (season, number) = (season?, episode?);
    Some(json!({ "progress": progress, "show": { "ids": ids }, "episode": { "season": season, "number": number } }))
}

/// One mark of the user (`library_set_watched`).
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub content_type: String,
    pub id: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
}

/// `/sync/history` body (`buildHistoryRequest`): movies, and shows with their seasons and episodes
/// (a whole show when no episode is given).
pub fn history_body(marks: &[Mark]) -> Value {
    let mut movies = Vec::new();
    let mut shows: BTreeMap<String, (Value, BTreeMap<i32, Vec<i32>>, bool)> = BTreeMap::new();
    for m in marks {
        let Some(ids) = ids_of(&m.id) else { continue };
        if m.content_type == "movie" {
            movies.push(json!({ "ids": ids }));
            continue;
        }
        let entry = shows.entry(ids.to_string()).or_insert_with(|| (ids.clone(), BTreeMap::new(), false));
        match (m.season, m.episode) {
            (Some(s), Some(e)) => entry.1.entry(s).or_default().push(e),
            _ => entry.2 = true,
        }
    }
    let shows: Vec<Value> = shows
        .into_values()
        .map(|(ids, seasons, whole)| {
            let mut show = json!({ "ids": ids });
            if !whole {
                show["seasons"] = Value::Array(seasons.into_iter().map(|(s, eps)| json!({ "number": s, "episodes": eps.into_iter().map(|e| json!({ "number": e })).collect::<Vec<_>>() })).collect());
            }
            show
        })
        .collect();
    json!({ "movies": movies, "shows": shows })
}

/// What the user watched on Simkl (`/sync/all-items`), as marks with the title.
#[derive(Debug, Clone, PartialEq)]
pub struct Watched {
    pub content_type: String,
    pub id: String,
    pub name: String,
    pub year: Option<i64>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
}

fn best_id(ids: &Value) -> Option<String> {
    let imdb = ids.get("imdb").and_then(Value::as_str).filter(|v| v.starts_with("tt"));
    if let Some(imdb) = imdb {
        return Some(imdb.to_string());
    }
    let tmdb = ids.get("tmdb").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))?;
    Some(format!("tmdb:{tmdb}"))
}

/// `/sync/all-items` (`episode_watched_at=yes`): finished movies, and the watched episodes of shows
/// and anime that carry an IMDb or TMDB id.
pub fn parse_all_items(body: &Value) -> Vec<Watched> {
    let mut out = Vec::new();
    for entry in body.get("movies").and_then(Value::as_array).into_iter().flatten() {
        let Some(movie) = entry.get("movie") else { continue };
        let finished = entry.get("status").and_then(Value::as_str) == Some("completed") || entry.get("last_watched_at").and_then(Value::as_str).is_some_and(|v| !v.is_empty());
        let Some(id) = movie.get("ids").and_then(best_id) else { continue };
        if finished {
            out.push(Watched { content_type: "movie".into(), id, name: movie.get("title").and_then(Value::as_str).unwrap_or("").into(), year: movie.get("year").and_then(Value::as_i64), season: None, episode: None });
        }
    }
    for key in ["shows", "anime"] {
        for entry in body.get(key).and_then(Value::as_array).into_iter().flatten() {
            let Some(show) = entry.get("show") else { continue };
            let Some(id) = show.get("ids").and_then(best_id) else { continue };
            let name = show.get("title").and_then(Value::as_str).unwrap_or("").to_string();
            let year = show.get("year").and_then(Value::as_i64);
            for season in entry.get("seasons").and_then(Value::as_array).into_iter().flatten() {
                let Some(s) = season.get("number").and_then(Value::as_i64) else { continue };
                for ep in season.get("episodes").and_then(Value::as_array).into_iter().flatten() {
                    let watched = ep.get("watched_at").and_then(Value::as_str).is_some_and(|v| !v.is_empty());
                    let Some(e) = ep.get("number").and_then(Value::as_i64).filter(|_| watched) else { continue };
                    out.push(Watched { content_type: "series".into(), id: id.clone(), name: name.clone(), year, season: Some(s as i32), episode: Some(e as i32) });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrobble_and_history_bodies() {
        assert_eq!(scrobble_body("movie", "tt1", None, None, 33.333).unwrap(), json!({ "progress": 33.33, "movie": { "ids": { "imdb": "tt1" } } }));
        assert_eq!(scrobble_body("series", "tmdb:5:1:2", Some(1), Some(2), 90.0).unwrap(), json!({ "progress": 90.0, "show": { "ids": { "tmdb": 5 } }, "episode": { "season": 1, "number": 2 } }));
        assert!(scrobble_body("series", "kitsu:1", Some(1), Some(1), 1.0).is_none());
        let m = |t: &str, id: &str, s: Option<i32>, e: Option<i32>| Mark { content_type: t.into(), id: id.into(), season: s, episode: e };
        let body = history_body(&[m("movie", "tt1", None, None), m("series", "tt2", Some(1), Some(3)), m("series", "tt2", Some(1), Some(4))]);
        assert_eq!(body, json!({ "movies": [{ "ids": { "imdb": "tt1" } }], "shows": [{ "ids": { "imdb": "tt2" }, "seasons": [{ "number": 1, "episodes": [{ "number": 3 }, { "number": 4 }] }] }] }));
    }

    #[test]
    fn all_items_keep_only_watched() {
        let body = json!({
            "movies": [
                { "status": "completed", "movie": { "title": "A", "year": 2001, "ids": { "imdb": "tt9" } } },
                { "status": "plantowatch", "movie": { "title": "B", "ids": { "imdb": "tt8" } } }
            ],
            "shows": [{ "show": { "title": "S", "ids": { "tmdb": "77" } }, "seasons": [{ "number": 1, "episodes": [{ "number": 1, "watched_at": "2024-01-01T00:00:00Z" }, { "number": 2, "watched_at": null }] }] }],
            "anime": [{ "show": { "title": "Só MAL", "ids": { "mal": 5 } }, "seasons": [{ "number": 1, "episodes": [{ "number": 1, "watched_at": "x" }] }] }]
        });
        let list = parse_all_items(&body);
        assert_eq!(list.iter().map(|w| (w.id.as_str(), w.episode)).collect::<Vec<_>>(), vec![("tt9", None), ("tmdb:77", Some(1))]);
    }

    #[test]
    fn token_refresh_a_day_early() {
        let t = Token { access_token: "a".into(), refresh_token: "r".into(), expires_at: 1_000_000 };
        assert!(!t.expiring(1_000_000 - 90_000));
        assert!(t.expiring(1_000_000 - 80_000));
    }
}
