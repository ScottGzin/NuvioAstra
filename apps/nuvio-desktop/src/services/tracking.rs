//! Trakt and Simkl OAuth + watched-history sync, ported from the original's
//! `TraktAuthRepository` / `SimklAuthRepository`. Client ids come from local credentials; tokens
//! and pending authorization material stay in Rust-owned documents.

use base64::Engine;
use sha2::Digest;
use nuvio_store::library::WatchedItem;
use nuvio_store::nextgen::AppStore;
use nuvio_store::tracking::{TrackingPending, TrackingToken};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

const TRAKT_API: &str = "https://api.trakt.tv";
const SIMKL_API: &str = "https://api.simkl.com";
const SIMKL_AUTHORIZE: &str = "https://simkl.com/oauth/authorize";

fn now_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64 }

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStart {
    pub user_code: String,
    pub verification_url: String,
    pub interval_secs: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PollResult { Pending, Authorized, Expired, Denied, SlowDown, NoSession, Error }

fn client() -> Result<reqwest::Client, String> {
    // Trakt rejects requests without a User-Agent (HTTP 403), reqwest does not send one by default.
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION"))),
    );
    reqwest::Client::builder().default_headers(headers).build().map_err(|e| e.to_string())
}

fn trakt_headers(client_id: &str, token: Option<&str>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in [
        ("trakt-api-key", client_id.to_string()),
        ("trakt-api-version", "2".to_string()),
        ("Content-Type", "application/json".to_string()),
    ] {
        if let (Ok(name), Ok(value)) = (reqwest::header::HeaderName::from_bytes(name.as_bytes()), value.parse()) {
            headers.insert(name, value);
        }
    }
    if let Some(token) = token {
        if let Ok(value) = format!("Bearer {token}").parse() { headers.insert(reqwest::header::AUTHORIZATION, value); }
    }
    headers
}

/// Trakt device flow, step 1: request a user code (`POST /oauth/device/code`).
pub async fn trakt_start(app: &AppStore, profile: i64, client_id: &str) -> Result<DeviceStart, String> {
    let response = client()?
        .post(format!("{TRAKT_API}/oauth/device/code"))
        .headers(trakt_headers(client_id, None))
        .json(&json!({ "client_id": client_id }))
        .send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Trakt recusou o pedido ({})", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let device_code = body.get("device_code").and_then(Value::as_str).unwrap_or("").to_string();
    let user_code = body.get("user_code").and_then(Value::as_str).unwrap_or("").to_string();
    if device_code.is_empty() || user_code.is_empty() { return Err("Trakt não devolveu um código de dispositivo.".into()); }
    let verification_url = body.get("verification_url").or_else(|| body.get("verification_uri"))
        .and_then(Value::as_str).unwrap_or("https://trakt.tv/activate").to_string();
    let interval = body.get("interval").and_then(Value::as_i64).unwrap_or(5).clamp(1, 30);
    let expires = now_ms() + body.get("expires_in").and_then(Value::as_i64).unwrap_or(600).max(60) * 1000;
    app.set_tracking_pending(profile, "trakt", Some(TrackingPending {
        secret: device_code, user_code: Some(user_code.clone()), verification_url: Some(verification_url.clone()),
        state: None, interval_secs: interval, expires_at_epoch_ms: expires,
    })).map_err(|e| e.to_string())?;
    Ok(DeviceStart { user_code, verification_url, interval_secs: interval })
}

/// Trakt device flow, step 2: poll `/oauth/device/token` until authorized/expired.
pub async fn trakt_poll(app: &AppStore, profile: i64, client_id: &str, client_secret: &str) -> Result<PollResult, String> {
    let pending = app.tracking_pending(profile, "trakt").map_err(|e| e.to_string())?.ok_or(PollResult::NoSession)
        .map_err(|_| "Nenhuma autorização Trakt iniciada.".to_string())?;
    if pending.expires_at_epoch_ms > 0 && now_ms() > pending.expires_at_epoch_ms {
        app.set_tracking_pending(profile, "trakt", None).map_err(|e| e.to_string())?;
        return Ok(PollResult::Expired);
    }
    let response = client()?
        .post(format!("{TRAKT_API}/oauth/device/token"))
        .headers(trakt_headers(client_id, None))
        .json(&json!({ "code": pending.secret, "client_id": client_id, "client_secret": client_secret }))
        .send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    if status.is_success() {
        let body: Value = response.json().await.map_err(|e| e.to_string())?;
        let access = body.get("access_token").and_then(Value::as_str).unwrap_or("");
        if access.is_empty() { return Ok(PollResult::Error); }
        let expires_in = body.get("expires_in").and_then(Value::as_i64).unwrap_or(0);
        app.set_tracking_token(profile, "trakt", Some(TrackingToken {
            access_token: access.to_string(),
            refresh_token: body.get("refresh_token").and_then(Value::as_str).map(str::to_string),
            expires_at_epoch_ms: if expires_in > 0 { now_ms() + expires_in * 1000 } else { 0 },
        })).map_err(|e| e.to_string())?;
        app.set_tracking_pending(profile, "trakt", None).map_err(|e| e.to_string())?;
        return Ok(PollResult::Authorized);
    }
    Ok(match status.as_u16() {
        400 => PollResult::Pending,
        410 => PollResult::Expired,
        418 => PollResult::Denied,
        429 => PollResult::SlowDown,
        _ => PollResult::Error,
    })
}

/// Random PKCE verifier and its S256 challenge (the original's `SimklPkceMaterial`).
fn pkce_material() -> (String, String, String) {
    let bytes: [u8; 32] = std::array::from_fn(|i| (now_ms().wrapping_mul(6364136223846793005).wrapping_add(i as i64 * 0x9E37)) as u8);
    let verifier: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let digest = sha2::Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    let state = format!("{:x}", now_ms().unsigned_abs());
    (verifier, challenge, state)
}

/// Simkl browser authorization URL with PKCE and state.
pub fn simkl_authorize_url(app: &AppStore, profile: i64, client_id: &str) -> Result<String, String> {
    let (verifier, challenge, state) = pkce_material();
    app.set_tracking_pending(profile, "simkl", Some(TrackingPending {
        secret: verifier, state: Some(state.clone()), ..Default::default()
    })).map_err(|e| e.to_string())?;
    let url = format!(
        "{SIMKL_AUTHORIZE}?response_type=code&client_id={client_id}&redirect_uri=nuvio%3A%2F%2Fauth%2Fsimkl\
         &code_challenge={challenge}&code_challenge_method=S256&state={state}&app-name=nuvio&app-version={}",
        env!("CARGO_PKG_VERSION")
    );
    Ok(url)
}

/// Exchanges the code received through the `nuvio://auth/simkl` callback.
pub async fn simkl_complete(app: &AppStore, profile: i64, client_id: &str, code: &str, state: &str) -> Result<bool, String> {
    let pending = app.tracking_pending(profile, "simkl").map_err(|e| e.to_string())?
        .ok_or("Nenhuma autorização Simkl iniciada.")?;
    if pending.state.as_deref().is_some_and(|expected| expected != state) {
        return Err("Estado de autorização inválido.".into());
    }
    let response = client()?
        .post(format!("{SIMKL_API}/oauth/token"))
        .header("Content-Type", "application/json")
        .json(&json!({
            "code": code, "client_id": client_id, "code_verifier": pending.secret,
            "redirect_uri": "nuvio://auth/simkl", "grant_type": "authorization_code"
        }))
        .send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() { return Err(format!("Simkl recusou o código ({})", response.status().as_u16())); }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let access = body.get("access_token").and_then(Value::as_str).unwrap_or("");
    if access.is_empty() { return Err("Simkl não devolveu um token.".into()); }
    let expires_in = body.get("expires_in").and_then(Value::as_i64).unwrap_or(0);
    app.set_tracking_token(profile, "simkl", Some(TrackingToken {
        access_token: access.to_string(), refresh_token: None,
        expires_at_epoch_ms: if expires_in > 0 { now_ms() + expires_in * 1000 } else { 0 },
    })).map_err(|e| e.to_string())?;
    app.set_tracking_pending(profile, "simkl", None).map_err(|e| e.to_string())?;
    Ok(true)
}

pub fn connected(app: &AppStore, profile: i64, provider: &str) -> bool {
    app.tracking_token(profile, provider).ok().flatten().is_some_and(|token| !token.access_token.is_empty())
}

pub fn disconnect(app: &AppStore, profile: i64, provider: &str) -> Result<(), String> {
    app.set_tracking_token(profile, provider, None).map_err(|e| e.to_string())?;
    app.set_tracking_pending(profile, provider, None).map_err(|e| e.to_string())?;
    Ok(())
}

/// IMDb id of a watched item, when the addon id already carries one.
fn imdb_id(item: &WatchedItem) -> Option<String> {
    [item.id.as_str(), item.video_id.as_deref().unwrap_or("")]
        .into_iter().find(|id| id.starts_with("tt") && id[2..].chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult { pub movies: usize, pub episodes: usize }

/// Pushes local watched marks missing from the remote watched history.
pub async fn sync_watched(
    app: &AppStore,
    profile: i64,
    provider: &str,
    client_id: &str,
) -> Result<SyncResult, String> {
    let token = app.tracking_token(profile, provider).map_err(|e| e.to_string())?
        .ok_or("Serviço não conectado.")?;
    let local: Vec<WatchedItem> = app.watched(profile).map_err(|e| e.to_string())?;
    let mut movies: Vec<String> = Vec::new();
    let mut episodes: Vec<(String, i32, i32)> = Vec::new();
    for item in local {
        let Some(id) = imdb_id(&item) else { continue };
        if let (Some(season), Some(episode)) = (item.season, item.episode) {
            episodes.push((id, season, episode));
        } else if !movies.contains(&id) {
            movies.push(id);
        }
    }
    let (remote_movies, remote_episodes) = match provider {
        "trakt" => trakt_watched(client_id, &token.access_token).await?,
        "simkl" => simkl_watched(&token.access_token).await?,
        _ => return Err("unknown provider".into()),
    };
    movies.retain(|id| !remote_movies.contains(id));
    episodes.retain(|entry| !remote_episodes.contains(entry));
    let result = SyncResult { movies: movies.len(), episodes: episodes.len() };
    if result.movies == 0 && result.episodes == 0 { return Ok(result); }
    let payload = match provider {
        "trakt" => json!({
            "movies": movies.iter().map(|id| json!({ "ids": { "imdb": id } })).collect::<Vec<_>>(),
            "episodes": episodes.iter().map(|(id, season, episode)| json!({
                "ids": { "imdb": id }, "season": season, "number": episode
            })).collect::<Vec<_>>(),
        }),
        _ => json!({
            "movies": movies.iter().map(|id| json!({ "ids": { "imdb": id } })).collect::<Vec<_>>(),
            "shows": episodes.iter().map(|(id, season, episode)| json!({
                "ids": { "imdb": id }, "seasons": [{ "number": season, "episodes": [{ "number": episode }] }]
            })).collect::<Vec<_>>(),
        }),
    };
    let path = match provider { "trakt" => format!("{TRAKT_API}/sync/history"), _ => format!("{SIMKL_API}/sync/history") };
    let mut request = client()?.post(path).header("Content-Type", "application/json");
    if provider == "trakt" { request = request.headers(trakt_headers(client_id, Some(&token.access_token))); }
    else { request = request.bearer_auth(&token.access_token); }
    let response = request.json(&payload).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() { return Err(format!("Falha ao sincronizar ({})", response.status().as_u16())); }
    Ok(result)
}

async fn trakt_watched(client_id: &str, token: &str) -> Result<(HashSet<String>, HashSet<(String, i32, i32)>), String> {
    let http = client()?;
    let movies: Value = http.get(format!("{TRAKT_API}/sync/watched/movies"))
        .headers(trakt_headers(client_id, Some(token))).send().await.map_err(|e| e.to_string())?
        .json().await.unwrap_or(Value::Null);
    let shows: Value = http.get(format!("{TRAKT_API}/sync/watched/shows"))
        .headers(trakt_headers(client_id, Some(token))).send().await.map_err(|e| e.to_string())?
        .json().await.unwrap_or(Value::Null);
    let mut movie_ids = HashSet::new();
    if let Some(items) = movies.as_array() {
        for entry in items {
            if let Some(id) = entry.pointer("/ids/imdb").and_then(Value::as_str) { movie_ids.insert(id.to_string()); }
        }
    }
    let mut episode_ids = HashSet::new();
    if let Some(items) = shows.as_array() {
        for show in items {
            let Some(id) = show.pointer("/ids/imdb").and_then(Value::as_str) else { continue };
            if let Some(seasons) = show.get("seasons").and_then(Value::as_array) {
                for season in seasons {
                    let Some(number) = season.get("number").and_then(Value::as_i64) else { continue };
                    if let Some(episodes) = season.get("episodes").and_then(Value::as_array) {
                        for episode in episodes {
                            if let Some(episode_number) = episode.get("number").and_then(Value::as_i64) {
                                episode_ids.insert((id.to_string(), number as i32, episode_number as i32));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok((movie_ids, episode_ids))
}

async fn simkl_watched(token: &str) -> Result<(HashSet<String>, HashSet<(String, i32, i32)>), String> {
    let http = client()?;
    let body: Value = http.get(format!("{SIMKL_API}/sync/history"))
        .bearer_auth(token).header("Content-Type", "application/json")
        .send().await.map_err(|e| e.to_string())?.json().await.unwrap_or(Value::Null);
    let mut movies = HashSet::new();
    let mut episodes = HashSet::new();
    if let Some(items) = body.as_array() {
        for entry in items {
            let Some(id) = entry.pointer("/movie/ids/imdb").or_else(|| entry.pointer("/show/ids/imdb")).and_then(Value::as_str) else { continue };
            if entry.get("movie").is_some() { movies.insert(id.to_string()); continue; }
            if let (Some(season), Some(episode)) = (
                entry.pointer("/episode/season").and_then(Value::as_i64),
                entry.pointer("/episode/number").and_then(Value::as_i64),
            ) {
                episodes.insert((id.to_string(), season as i32, episode as i32));
            }
        }
    }
    Ok((movies, episodes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imdb_ids_only() {
        let base = || WatchedItem {
            id: String::new(), content_type: "movie".into(), name: "Title".into(), poster: None,
            release_info: None, season: None, episode: None, video_id: None, marked_at_epoch_ms: 0,
        };
        let mut item = base();
        item.id = "tt1234567".into();
        assert_eq!(imdb_id(&item).as_deref(), Some("tt1234567"));
        item.id = "ts:123".into();
        item.video_id = Some("tt7654321".into());
        assert_eq!(imdb_id(&item).as_deref(), Some("tt7654321"));
        item.video_id = None;
        assert_eq!(imdb_id(&item), None);
    }

    #[test]
    fn pkce_material_is_stable_length() {
        let (verifier, challenge, state) = pkce_material();
        assert_eq!(verifier.len(), 64);
        assert!(!challenge.is_empty() && !state.is_empty());
    }
}

const MDBLIST_API: &str = "https://api.mdblist.com";

/// MDBList device flow (`MdbListAuthRepository.startDeviceAuthorization`). Form POST, not JSON.
pub async fn mdblist_start(app: &AppStore, profile: i64, client_id: &str) -> Result<DeviceStart, String> {
    let response = client()?
        .post(format!("{MDBLIST_API}/oauth/device-authorization/"))
        .form(&[("scope", "write"), ("client_id", client_id)])
        .send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("MDBList recusou o pedido ({})", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let device_code = body.get("device_code").and_then(Value::as_str).unwrap_or("").to_string();
    let user_code = body.get("user_code").and_then(Value::as_str).unwrap_or("").to_string();
    if device_code.is_empty() || user_code.is_empty() { return Err("MDBList não devolveu um código de dispositivo.".into()); }
    let verification_url = body.get("verification_uri_complete").or_else(|| body.get("verification_uri"))
        .and_then(Value::as_str).unwrap_or("https://mdblist.com/activate").to_string();
    let interval = body.get("interval").and_then(Value::as_i64).unwrap_or(5).clamp(1, 3600);
    let expires = now_ms() + body.get("expires_in").and_then(Value::as_i64).unwrap_or(600).max(60) * 1000;
    app.set_tracking_pending(profile, "mdblist", Some(TrackingPending {
        secret: device_code, user_code: Some(user_code.clone()), verification_url: Some(verification_url.clone()),
        state: None, interval_secs: interval, expires_at_epoch_ms: expires,
    })).map_err(|e| e.to_string())?;
    Ok(DeviceStart { user_code, verification_url, interval_secs: interval })
}

/// MDBList device flow, step 2 (`POST /oauth/token/` with the device_code grant).
pub async fn mdblist_poll(app: &AppStore, profile: i64, client_id: &str) -> Result<PollResult, String> {
    let pending = app.tracking_pending(profile, "mdblist").map_err(|e| e.to_string())?
        .ok_or("Nenhuma autorização MDBList iniciada.")?;
    if pending.expires_at_epoch_ms > 0 && now_ms() > pending.expires_at_epoch_ms {
        app.set_tracking_pending(profile, "mdblist", None).map_err(|e| e.to_string())?;
        return Ok(PollResult::Expired);
    }
    let response = client()?
        .post(format!("{MDBLIST_API}/oauth/token/"))
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", pending.secret.as_str()),
            ("scope", "write"),
            ("client_id", client_id),
        ])
        .send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        let access = body.get("access_token").and_then(Value::as_str).unwrap_or("");
        if access.is_empty() { return Ok(PollResult::Error); }
        let expires_in = body.get("expires_in").and_then(Value::as_i64).unwrap_or(0);
        app.set_tracking_token(profile, "mdblist", Some(TrackingToken {
            access_token: access.to_string(),
            refresh_token: body.get("refresh_token").and_then(Value::as_str).map(str::to_string),
            expires_at_epoch_ms: if expires_in > 0 { now_ms() + expires_in * 1000 } else { 0 },
        })).map_err(|e| e.to_string())?;
        app.set_tracking_pending(profile, "mdblist", None).map_err(|e| e.to_string())?;
        return Ok(PollResult::Authorized);
    }
    Ok(match body.get("error").and_then(Value::as_str).unwrap_or("") {
        "authorization_pending" => PollResult::Pending,
        "slow_down" => PollResult::SlowDown,
        "expired_token" => PollResult::Expired,
        "access_denied" => PollResult::Denied,
        _ => PollResult::Error,
    })
}
