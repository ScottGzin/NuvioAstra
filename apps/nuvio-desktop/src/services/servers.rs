//! Media servers: Jellyfin/Emby (MediaBrowser), Plex (PIN device flow) and Silo (device code /
//! password). Credentials belong to the user's own server; the resulting access token stays in a
//! Rust-only document and is never returned to the UI.
//!
//! The Plex and Silo flows/mappers follow the original Nuvio `features/servers/{plex,silo}` and
//! cross-check the public endpoints against VyseTV's `media_servers` implementation.

use nuvio_core::catalog::{CatalogPage, MetaPreview, PosterShape};
use nuvio_core::stream::{StreamBehaviorHints, StreamItem, StreamProxyHeaders};
use nuvio_store::nextgen::AppStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Connection {
    pub id: String,
    /// `jellyfin`, `emby`, `plex` or `silo`. Login flows differ; tokens stay in `server_tokens`.
    pub provider: String,
    pub name: String,
    pub address: String,
    pub user_id: String,
    pub user_name: String,
    pub enabled: bool,
}

impl Default for Connection {
    fn default() -> Self {
        Self { id: String::new(), provider: "jellyfin".into(), name: String::new(), address: String::new(), user_id: String::new(), user_name: String::new(), enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Library {
    pub id: String,
    pub name: String,
    /// `movies`, `tvshows`, `music` or `mixed`, from Jellyfin's `CollectionType`.
    pub kind: String,
}

impl Default for Library { fn default() -> Self { Self { id: String::new(), name: String::new(), kind: "mixed".into() } } }

pub fn connections(app: &AppStore, profile: i64) -> Result<Vec<Connection>, String> {
    Ok(app.read::<BTreeMap<i64, Vec<Connection>>>("servers").map_err(|e| e.to_string())?.unwrap_or_default().remove(&profile).unwrap_or_default())
}

pub fn save_connections(app: &AppStore, profile: i64, items: Vec<Connection>) -> Result<(), String> {
    let mut all = app.read::<BTreeMap<i64, Vec<Connection>>>("servers").map_err(|e| e.to_string())?.unwrap_or_default();
    all.insert(profile, items);
    app.write("servers", &all).map_err(|e| e.to_string())
}

fn tokens(app: &AppStore) -> BTreeMap<String, String> {
    app.read("server_tokens").ok().flatten().unwrap_or_default()
}

fn save_token(app: &AppStore, id: &str, token: &str) -> Result<(), String> {
    let mut all: BTreeMap<String, String> = tokens(app);
    all.insert(id.to_string(), token.to_string());
    app.write("server_tokens", &all).map_err(|e| e.to_string())
}

pub fn remove(app: &AppStore, profile: i64, id: &str) -> Result<Vec<Connection>, String> {
    let mut items = connections(app, profile)?;
    items.retain(|item| item.id != id);
    save_connections(app, profile, items.clone())?;
    let mut all = tokens(app);
    all.remove(id);
    app.write("server_tokens", &all).map_err(|e| e.to_string())?;
    Ok(items)
}

/// `normalizeServerAddress` of the original: add http:// and strip `/web` / api suffixes.
pub fn normalize_address(raw: &str) -> String {
    let mut value = raw.trim().trim_end_matches('/').to_string();
    if !value.contains("://") { value = format!("http://{value}"); }
    for suffix in ["/web/index.html", "/web", "/emby", "/jellyfin"] {
        if value.to_ascii_lowercase().ends_with(suffix) {
            value.truncate(value.len() - suffix.len());
            break;
        }
    }
    value.trim_end_matches('/').to_string()
}

fn api_path(provider: &str) -> &'static str {
    if provider == "emby" { "/emby" } else { "" }
}

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Enough entropy for a media-server client id / playback session id (no `rand` dependency).
pub fn random_id(len: usize) -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut seed = nanos as u64 ^ (std::process::id() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ ID_COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let mut out = String::new();
    while out.len() < len {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        out.push(std::char::from_digit((seed & 15) as u32, 16).unwrap_or('0'));
    }
    out
}

/// Stable `X-Plex-Client-Identifier` across restarts (same namespace as the original app).
pub fn client_id(app: &AppStore) -> String {
    if let Some(saved) = app.read::<String>("server_client_id").ok().flatten() {
        if !saved.is_empty() {
            return saved;
        }
    }
    // Same identity format as Nuvio Desktop (`nuvio-mobile-` + 32 chars): Plex and Silo then treat
    // this build as the same client as the original app.
    let id = format!("nuvio-mobile-{}", random_id(32));
    let _ = app.write("server_client_id", &id);
    id
}

fn request_headers(provider: &str, token: Option<&str>, client_id: &str) -> Vec<(String, String)> {
    let token = token.filter(|t| !t.is_empty());
    match provider {
        "plex" => {
            let mut headers = vec![
                ("X-Plex-Product".into(), "Nuvio".into()),
                ("X-Plex-Version".into(), env!("CARGO_PKG_VERSION").chars().take(64).collect()),
                ("X-Plex-Client-Identifier".into(), client_id.into()),
                ("X-Plex-Platform".into(), "Linux".into()),
                ("X-Plex-Device".into(), "Desktop".into()),
                ("X-Plex-Device-Name".into(), "Nuvio Astra".into()),
                ("X-Plex-Provides".into(), "player".into()),
                ("Accept".into(), "application/json".into()),
            ];
            if let Some(token) = token {
                headers.push(("X-Plex-Token".into(), token.into()));
            }
            headers
        }
        "silo" => {
            let mut headers = vec![("Accept".into(), "application/json".into())];
            if let Some(token) = token {
                let (access, profile) = token.split_once('|').unwrap_or((token, ""));
                headers.push(("Authorization".into(), format!("Bearer {access}")));
                if !profile.is_empty() {
                    headers.push(("X-Profile-Id".into(), profile.into()));
                }
            }
            headers
        }
        _ => {
            let auth = |token: Option<&str>| {
                let token = token.map(|t| format!(", Token=\"{t}\"")).unwrap_or_default();
                format!("MediaBrowser Client=\"Nuvio\", Device=\"NuvioDesktop\", DeviceId=\"{client_id}\", Version=\"{}\"{token}", env!("CARGO_PKG_VERSION"))
            };
            vec![
                ("X-Emby-Authorization".into(), auth(token)),
                ("Authorization".into(), auth(token)),
                ("Accept".into(), "application/json".into()),
            ]
        }
    }
}

fn apply_headers(builder: reqwest::RequestBuilder, headers: &[(String, String)]) -> reqwest::RequestBuilder {
    let mut builder = builder;
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
}

/// Authenticates against the user's server and stores the returned token. Neither password nor
/// token is ever included in the returned connection.
pub async fn add(app: &AppStore, profile: i64, provider: &str, address: &str, username: &str, password: &str) -> Result<Connection, String> {
    let provider = provider.to_ascii_lowercase();
    if provider != "jellyfin" && provider != "emby" {
        return Err("Somente Jellyfin e Emby são suportados.".into());
    }
    let base = normalize_address(address);
    if username.trim().is_empty() { return Err("Informe o usuário.".into()); }
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let url = format!("{base}{}/Users/AuthenticateByName", api_path(&provider));
    let headers = request_headers(&provider, None, &client_id(app));
    let response = apply_headers(client.post(&url), &headers)
        .header("Content-Type", "application/json")
        .json(&json!({ "Username": username, "Pw": password }))
        .send()
        .await
        .map_err(|e| format!("Não foi possível conectar ao servidor: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Credenciais recusadas pelo servidor ({})", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let token = body.get("AccessToken").and_then(Value::as_str).unwrap_or("");
    let user_id = body.pointer("/SessionInfo/UserId").and_then(Value::as_str)
        .or_else(|| body.get("UserId").and_then(Value::as_str)).unwrap_or("");
    let user_name = body.pointer("/SessionInfo/UserName").and_then(Value::as_str)
        .or_else(|| body.pointer("/User/Name").and_then(Value::as_str)).unwrap_or(username);
    if token.is_empty() || user_id.is_empty() { return Err("O servidor não devolveu uma sessão válida.".into()); }
    let mut items = connections(app, profile)?;
    let id = format!("srv-{}", user_id.chars().take(12).collect::<String>());
    let connection = Connection {
        id: id.clone(), provider, name: format!("{user_name} · {base}"), address: base, user_id: user_id.into(),
        user_name: user_name.into(), enabled: true,
    };
    items.retain(|item| item.id != id);
    items.push(connection.clone());
    save_connections(app, profile, items)?;
    save_token(app, &id, token)?;
    Ok(connection)
}

pub async fn get_json(client: &reqwest::Client, url: &str, provider: &str, token: Option<&str>, app: &AppStore) -> Result<Value, String> {
    let headers = request_headers(provider, token.filter(|t| !t.is_empty()), &client_id(app));
    let response = apply_headers(client.get(url), &headers).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.json().await.map_err(|e| e.to_string())
}

pub async fn post_json(client: &reqwest::Client, url: &str, provider: &str, token: Option<&str>, app: &AppStore, body: Value) -> Result<Value, String> {
    let headers = request_headers(provider, token, &client_id(app));
    let response = apply_headers(client.post(url), &headers)
        .header("Content-Type", "application/json")
        .json(&body)
        .send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.json::<Value>().await.map_err(|e| e.to_string())
}

fn text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(*key).and_then(Value::as_str).filter(|v| !v.is_empty()))
}

fn number(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| match value.get(*key) {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    })
}

pub async fn libraries(app: &AppStore, profile: i64, id: &str) -> Result<Vec<Library>, String> {
    let connection = connections(app, profile)?.into_iter().find(|c| c.id == id).ok_or("servidor não encontrado")?;
    let token = tokens(app).remove(id).ok_or("sessão expirada; conecte novamente")?;
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let url = if connection.provider == "emby" {
        format!("{}{}/Users/{}/Views", connection.address, api_path(&connection.provider), connection.user_id)
    } else {
        format!("{}/UserViews", connection.address)
    };
    if connection.provider == "plex" {
        let url = format!("{}/library/sections", connection.address);
        let body = get_json(&client, &url, "plex", Some(&token), app).await?;
        let items = body.pointer("/MediaContainer/Directory").and_then(Value::as_array).cloned().unwrap_or_default();
        return Ok(items.into_iter().filter_map(|item| {
            let kind = match number(&item, &["type", "@attributes.type"]).unwrap_or(0) {
                1 => "movies", 2 => "tvshows", 8 | 9 | 10 => "music", _ => return None,
            };
            Some(Library {
                id: text(&item, &["key", "@attributes.key"])?.to_string(),
                name: text(&item, &["title", "@attributes.title"]).unwrap_or("Biblioteca").to_string(),
                kind: kind.into(),
            })
        }).collect());
    }
    if connection.provider == "silo" {
        let url = format!("{}/api/v2/user/libraries", connection.address);
        let body = get_json(&client, &url, "silo", Some(&token), app).await?;
        let items = body.get("items").and_then(Value::as_array)
            .or_else(|| body.as_array()).cloned().unwrap_or_default();
        return Ok(items.into_iter().filter_map(|item| {
            let raw = text(&item, &["kind", "type"]).unwrap_or("mixed");
            let kind = if raw.contains("movie") { "movies" }
                else if raw.contains("series") || raw.contains("show") || raw == "tv" { "tvshows" }
                else if raw.contains("music") { "music" } else { "mixed" };
            Some(Library {
                id: text(&item, &["id"]).or_else(|| text(&item, &["library_id"]))?.to_string(),
                name: text(&item, &["name", "title"]).unwrap_or("Biblioteca").to_string(),
                kind: kind.into(),
            })
        }).collect());
    }
    let body = get_json(&client, &url, &connection.provider, Some(&token), app).await?;
    let items = body.get("Items").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(items.into_iter().map(|item| Library {
        id: item.get("Id").and_then(Value::as_str).unwrap_or("").to_string(),
        name: item.get("Name").and_then(Value::as_str).unwrap_or("Servidor").to_string(),
        kind: match item.get("CollectionType").and_then(Value::as_str).unwrap_or("") {
            "movies" => "movies", "tvshows" => "tvshows", "music" => "music", _ => "mixed",
        }.to_string(),
    }).filter(|library| !library.id.is_empty()).collect())
}

fn preview(connection: &Connection, item: &Value) -> Option<MetaPreview> {
    let id = item.get("Id")?.as_str()?;
    let item_type = item.get("Type").and_then(Value::as_str).unwrap_or("");
    let content_type = if item_type.contains("Series") || item_type == "Episode" { "series" } else { "movie" };
    let image = |kind: &str| -> Option<String> {
        item.get("ImageTags")?.get(kind).and_then(Value::as_str)
            .map(|tag| format!("{}{}/Items/{id}/Images/{kind}?tag={tag}&Quality=90", connection.address, api_path(&connection.provider)))
    };
    let poster = image("Primary");
    Some(MetaPreview {
        id: format!("srv1:{}:{}", connection.id, id),
        content_type: content_type.into(),
        name: item.get("Name").and_then(Value::as_str).unwrap_or("Sem título").to_string(),
        poster: poster.clone(),
        banner: image("Banner"),
        logo: image("Logo"),
        poster_shape: PosterShape::Poster,
        description: item.get("Overview").and_then(Value::as_str).map(str::to_string),
        release_info: item.get("ProductionYear").and_then(Value::as_i64).map(|year| year.to_string()),
        raw_release_date: None,
        imdb_rating: item.get("CommunityRating").and_then(Value::as_f64).map(|v| format!("{v:.1}")),
        genres: item.get("Genres").and_then(Value::as_array).map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        landscape_poster: poster,
        runtime: item.get("RunTimeTicks").and_then(Value::as_i64).map(|ticks| format!("{} min", ticks / 10_000_000 / 60_000)),
        status: None,
        raw_poster_url: None, raw_landscape_poster_url: None,
    })
}

fn plex_image(base: &str, path: Option<&str>, token: &str) -> Option<String> {
    let path = path?.trim_start_matches('/');
    if path.is_empty() { return None; }
    Some(format!("{base}/{path}?X-Plex-Token={token}"))
}

fn preview_plex(connection: &Connection, item: &Value, token: &str) -> Option<MetaPreview> {
    let id = text(item, &["ratingKey", "@attributes.ratingKey"])?;
    let item_type = number(item, &["type", "@attributes.type"]).unwrap_or(0);
    let content_type = if item_type == 2 || item_type == 4 { "series" } else { "movie" };
    let poster = plex_image(&connection.address, text(item, &["thumb", "@attributes.thumb"]), token);
    let art = plex_image(&connection.address, text(item, &["art", "@attributes.art"]), token);
    let genres = item.get("Genre").and_then(Value::as_array).map(|values| {
        values.iter().filter_map(|entry| text(entry, &["tag"]).map(str::to_string)).collect()
    }).unwrap_or_default();
    Some(MetaPreview {
        id: format!("srv1:{}:{}", connection.id, id),
        content_type: content_type.into(),
        name: text(item, &["title", "@attributes.title"]).unwrap_or("Sem título").to_string(),
        poster: poster.clone(),
        banner: None,
        logo: None,
        poster_shape: PosterShape::Poster,
        description: text(item, &["summary"]).map(str::to_string),
        release_info: number(item, &["year", "@attributes.year"]).map(|year| year.to_string()),
        raw_release_date: None,
        imdb_rating: item.get("rating").and_then(Value::as_f64).map(|v| format!("{v:.1}")),
        genres,
        landscape_poster: art,
        runtime: number(item, &["duration", "@attributes.duration"])
            .map(|ms| format!("{} min", ms / 60_000)),
        status: None,
        raw_poster_url: None, raw_landscape_poster_url: None,
    })
}

fn preview_silo(connection: &Connection, item: &Value) -> Option<MetaPreview> {
    let id = text(item, &["id", "content_id", "contentId"])?;
    let raw_type = text(item, &["type", "kind", "media_kind"]).unwrap_or("movie");
    let content_type = if raw_type.contains("series") || raw_type.contains("show") { "series" } else { "movie" };
    let images = item.get("images").or_else(|| item.get("artwork"));
    let image = |kinds: &[&str]| -> Option<String> {
        let images = images?;
        for kind in kinds {
            if let Some(value) = images.get(*kind).and_then(Value::as_str) {
                if value.starts_with("http") { return Some(value.to_string()); }
                return Some(format!("{}{}", connection.address, value));
            }
        }
        None
    };
    let poster = image(&["poster", "primary", "thumb"]);
    Some(MetaPreview {
        id: format!("srv1:{}:{}", connection.id, id),
        content_type: content_type.into(),
        name: text(item, &["title", "name"]).unwrap_or("Sem título").to_string(),
        poster: poster.clone(),
        banner: image(&["banner"]),
        logo: image(&["logo"]),
        poster_shape: PosterShape::Poster,
        description: text(item, &["overview", "summary", "description"]).map(str::to_string),
        release_info: number(item, &["year", "release_year"]).map(|year| year.to_string()),
        raw_release_date: text(item, &["release_date"]).map(str::to_string),
        imdb_rating: item.get("rating").and_then(Value::as_f64).map(|v| format!("{v:.1}")),
        genres: item.get("genres").and_then(Value::as_array).map(|values| {
            values.iter().filter_map(|entry| {
                text(entry, &["name", "tag"]).map(str::to_string).or_else(|| entry.as_str().map(str::to_string))
            }).collect()
        }).unwrap_or_default(),
        landscape_poster: image(&["backdrop", "background", "fanart"]).or_else(|| poster.clone()),
        runtime: number(item, &["runtime", "duration", "duration_seconds"]).map(|seconds| format!("{} min", seconds / 60)),
        status: None,
        raw_poster_url: None, raw_landscape_poster_url: None,
    })
}

/// `GET /Items` of one library, mapped to the same shape addon catalogs use.
pub async fn catalog(app: &AppStore, profile: i64, connection_id: &str, library_id: &str, skip: u32) -> Result<CatalogPage, String> {
    let connection = connections(app, profile)?.into_iter().find(|c| c.id == connection_id).ok_or("servidor não encontrado")?;
    // The token is used only here; `preview` never receives it, so image URLs stay token-free.
    let token = tokens(app).remove(connection_id).ok_or("sessão expirada; conecte novamente")?;
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    if connection.provider == "plex" {
        let url = format!("{}/library/sections/{library_id}/all?X-Plex-Container-Start={skip}&X-Plex-Container-Size=60&sort=addedAt%3Adesc", connection.address);
        let body = get_json(&client, &url, "plex", Some(&token), app).await?;
        let container = body.get("MediaContainer").unwrap_or(&body);
        let raw = number(container, &["totalSize", "@attributes.totalSize"]).unwrap_or(0).max(0) as usize;
        let items = container.get("Metadata").and_then(Value::as_array).cloned().unwrap_or_default();
        let previews: Vec<MetaPreview> = items.iter().filter_map(|item| preview_plex(&connection, item, &token)).collect();
        let next = (skip as usize + previews.len() < raw).then_some(skip + 60);
        return Ok(CatalogPage { items: previews, raw_item_count: raw, next_skip: next });
    }
    if connection.provider == "silo" {
        let url = format!("{}/api/v2/catalog?source=query&library_id={library_id}&type=movie&sort=-added_at&image_size=medium&limit=60&offset={skip}", connection.address);
        let body = get_json(&client, &url, "silo", Some(&token), app).await?;
        let mut items = body.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        // Silo artwork URLs expire: refresh items without images from the current server address.
        for item in &mut items {
            if item.get("images").is_none() {
                let id = text(item, &["id", "content_id", "contentId"]).unwrap_or_default();
                if !id.is_empty() {
                    if let Ok(fresh) = get_json(&client, &format!("{}/api/v2/catalog/items/{id}?image_size=medium", connection.address), "silo", Some(&token), app).await {
                        if let Some(images) = fresh.get("images") {
                            item["images"] = images.clone();
                        }
                    }
                }
            }
        }
        let raw = number(&body, &["total", "total_count"]).unwrap_or(items.len() as i64).max(0) as usize;
        let previews: Vec<MetaPreview> = items.iter().filter_map(|item| preview_silo(&connection, item)).collect();
        let next = (skip as usize + previews.len() < raw).then_some(skip + 60);
        return Ok(CatalogPage { items: previews, raw_item_count: raw, next_skip: next });
    }
    let url = if connection.provider == "emby" {
        format!("{}{}/Users/{}/Items?ParentId={library_id}&StartIndex={skip}&Limit=60&Recursive=true", connection.address, api_path(&connection.provider), connection.user_id)
    } else {
        format!("{}/Items?ParentId={library_id}&StartIndex={skip}&Limit=60&Recursive=true", connection.address)
    };
    let body = get_json(&client, &url, &connection.provider, Some(&token), app).await?;
    let raw = body.get("TotalRecordCount").and_then(Value::as_u64).unwrap_or(0) as usize;
    let items = body.get("Items").and_then(Value::as_array).cloned().unwrap_or_default();
    let previews: Vec<MetaPreview> = items.iter().filter_map(|item| preview(&connection, item)).collect();
    let next = (skip as usize + previews.len() < raw).then_some(skip + 60);
    Ok(CatalogPage { items: previews, raw_item_count: raw, next_skip: next })
}

// ---------------------------------------------------------------- Plex sign-in

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexPin {
    pub id: String,
    pub code: String,
    pub auth_url: String,
}

pub async fn plex_pin_start(app: &AppStore) -> Result<PlexPin, String> {
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let cid = client_id(app);
    let response = apply_headers(client.post("https://plex.tv/api/v2/pins?strong=true"), &request_headers("plex", None, &cid))
        .header("Content-Length", "0")
        .send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("plex.tv recusou o PIN ({})", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let id = text(&body, &["id"]).ok_or("plex.tv não devolveu o id do PIN")?.to_string();
    let code = text(&body, &["code"]).ok_or("plex.tv não devolveu o código do PIN")?.to_string();
    let auth_url = format!("https://app.plex.tv/auth#?clientID={cid}&code={code}&context%5Bdevice%5D%5Bproduct%5D=Nuvio");
    Ok(PlexPin { id, code, auth_url })
}

/// `None` while the user has not claimed the PIN yet (404/410 also means "waiting").
pub async fn plex_pin_poll(app: &AppStore, pin_id: &str) -> Result<Option<String>, String> {
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let url = format!("https://plex.tv/api/v2/pins/{pin_id}");
    let response = apply_headers(client.get(&url), &request_headers("plex", None, &client_id(app))).send().await.map_err(|e| e.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND || response.status() == reqwest::StatusCode::GONE {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("plex.tv recusou a consulta ({})", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(text(&body, &["authToken"]).map(str::to_string))
}

fn pick_plex_server(resources: &Value, account_token: &str) -> Option<(String, String, String)> {
    let list = resources.as_array()?;
    let candidates: Vec<&Value> = list.iter().filter(|resource| {
        resource.get("provides").and_then(Value::as_str).is_some_and(|provides| provides.split(',').any(|p| p.trim() == "server"))
    }).collect();
    for resource in candidates.iter().copied() {
        let machine = text(resource, &["machineIdentifier"]).unwrap_or_default();
        if machine.is_empty() { continue; }
        let server_token = text(resource, &["accessToken"]).filter(|t| !t.is_empty()).unwrap_or(account_token);
        let address = resource.get("connections").and_then(Value::as_array)
            .and_then(|connections| connections.iter().find_map(|connection| {
                let uri = text(connection, &["uri"])?;
                let local = connection.get("local").and_then(Value::as_bool).unwrap_or(false);
                let relay = connection.get("relay").and_then(Value::as_bool).unwrap_or(false);
                (uri.starts_with("http://") || uri.starts_with("https://")).then_some((local, relay, uri.to_string()))
            }))
            .map(|(_, _, uri)| uri)
            .or_else(|| text(resource, &["address"]).and_then(|address| {
                number(resource, &["port"]).map(|port| format!("http://{address}:{port}"))
            }));
        if let Some(address) = address {
            return Some((machine.to_string(), server_token.to_string(), address.trim_end_matches('/').to_string()));
        }
    }
    None
}

pub async fn plex_connect(app: &AppStore, profile: i64, account_token: &str) -> Result<Connection, String> {
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let url = "https://clients.plex.tv/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1";
    let resources = get_json(&client, url, "plex", Some(account_token), app).await?;
    let (machine, server_token, address) = pick_plex_server(&resources, account_token).ok_or("Nenhum servidor Plex disponível nesta conta.")?;
    let user = get_json(&client, "https://plex.tv/api/v2/user", "plex", Some(account_token), app).await.ok();
    let user_id = user.as_ref().and_then(|u| text(u, &["id", "@attributes.id"])).unwrap_or(&machine).to_string();
    let user_name = user.as_ref().and_then(|u| text(u, &["username", "name", "@attributes.name"])).unwrap_or("Plex").to_string();
    let mut items = connections(app, profile)?;
    let id = format!("srv-{}", machine.chars().take(12).collect::<String>());
    let connection = Connection {
        id: id.clone(), provider: "plex".into(), name: format!("{user_name} · {address}"),
        address, user_id, user_name, enabled: true,
    };
    items.retain(|item| item.id != id);
    items.push(connection.clone());
    save_connections(app, profile, items)?;
    save_token(app, &id, &server_token)?;
    Ok(connection)
}

// ---------------------------------------------------------------- Silo sign-in

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiloInfo {
    pub address: String,
    pub name: String,
    pub server_id: String,
    pub password_login: bool,
    pub device_code: bool,
}

pub async fn silo_discover(app: &AppStore, raw_address: &str) -> Result<SiloInfo, String> {
    let base = normalize_address(raw_address);
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let info = get_json(&client, &format!("{base}/api/v2/system/info"), "silo", None, app).await;
    if info.is_err() {
        return Err("Não parece um servidor Silo (API v2 indisponível).".into());
    }
    let identity = get_json(&client, &format!("{base}/api/v2/system/identity"), "silo", None, app).await.ok();
    let providers = get_json(&client, &format!("{base}/api/v2/auth/providers"), "silo", None, app).await.ok();
    let branding = get_json(&client, &format!("{base}/api/v2/theme/branding"), "silo", None, app).await.ok();
    let has_password = providers.as_ref().is_some_and(|value| {
        value.get("password_login").and_then(Value::as_bool).unwrap_or(false)
            || value.as_array().is_some_and(|list| list.iter().any(|item| text(item, &["mode"]).is_some_and(|mode| mode == "credentials")))
    });
    let has_device = providers.as_ref().is_some_and(|value| {
        value.get("device_code").and_then(Value::as_bool).unwrap_or(false)
            || value.as_array().is_some_and(|list| list.iter().any(|item| text(item, &["mode"]).is_some_and(|mode| mode.contains("device"))))
    });
    Ok(SiloInfo {
        name: branding.as_ref().and_then(|v| text(v, &["server_name", "name"])).unwrap_or("Silo").to_string(),
        server_id: identity.as_ref().and_then(|v| text(v, &["server_id", "id"])).unwrap_or(&base).to_string(),
        address: base,
        password_login: has_password,
        device_code: has_device,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiloCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
}

pub async fn silo_code_start(app: &AppStore, address: &str) -> Result<SiloCode, String> {
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let body = json!({
        "device_name": format!("Nuvio on {}", std::env::consts::OS),
        "device_platform": "android-tv",
    });
    let response = post_json(&client, &format!("{address}/api/v2/auth/device/start"), "silo", None, app, body).await?;
    Ok(SiloCode {
        device_code: text(&response, &["device_code"]).ok_or("Silo não devolveu o device code")?.to_string(),
        user_code: text(&response, &["user_code"]).ok_or("Silo não devolveu o código do usuário")?.to_string(),
        verification_uri: text(&response, &["verification_uri_complete", "verification_uri"]).unwrap_or("").to_string(),
        interval: number(&response, &["interval"]).unwrap_or(5).max(1) as u64,
    })
}

fn access_token(value: &Value) -> Option<String> {
    text(value, &["access_token", "accessToken", "access"])
        .or_else(|| value.pointer("/tokens/access_token").and_then(Value::as_str))
        .or_else(|| value.pointer("/tokens/access").and_then(Value::as_str))
        .map(str::to_string)
}

async fn silo_save(app: &AppStore, profile: i64, address: &str, token: &str) -> Result<Connection, String> {
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let profiles = get_json(&client, &format!("{address}/api/v2/profiles"), "silo", Some(token), app).await.ok();
    let first = profiles.as_ref().and_then(|value| value.get("items").and_then(Value::as_array).or_else(|| value.as_array())).and_then(|items| items.first());
    let profile_id = first.and_then(|p| text(p, &["id"])).unwrap_or("default").to_string();
    let profile_name = first.and_then(|p| text(p, &["name"])).unwrap_or("Silo");
    let mut items = connections(app, profile)?;
    let id = format!("srv-{}", random_id(12));
    let connection = Connection {
        id: id.clone(), provider: "silo".into(), name: format!("Silo · {profile_name} · {address}"),
        address: address.to_string(), user_id: profile_id.clone(), user_name: profile_name.to_string(), enabled: true,
    };
    items.retain(|item| item.id != id);
    items.push(connection.clone());
    save_connections(app, profile, items)?;
    // `access|profileId` is kept in Rust only; request_headers turns it into two headers.
    save_token(app, &id, &format!("{token}|{profile_id}"))?;
    Ok(connection)
}

pub async fn silo_login(app: &AppStore, profile: i64, raw_address: &str, username: &str, password: &str) -> Result<Connection, String> {
    let address = normalize_address(raw_address);
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let body = json!({ "username": username.trim(), "password": password });
    let response = post_json(&client, &format!("{address}/api/v2/auth/login"), "silo", None, app, body).await?;
    let token = access_token(&response).ok_or("Silo não devolveu um token de acesso.")?;
    silo_save(app, profile, &address, &token).await
}

/// `None` while pending/denied; a connected session once the server approves the code.
pub async fn silo_code_connect(app: &AppStore, profile: i64, raw_address: &str, device_code: &str) -> Result<Option<Connection>, String> {
    let address = normalize_address(raw_address);
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let response = post_json(&client, &format!("{address}/api/v2/auth/device/poll"), "silo", None, app, json!({ "device_code": device_code })).await?;
    let status = text(&response, &["status"]).unwrap_or("pending");
    if status != "approved" {
        return Ok(None);
    }
    let token = access_token(&response).ok_or("Silo aprovou o código mas não devolveu o token.")?;
    silo_save(app, profile, &address, &token).await.map(Some)
}

// ---------------------------------------------------------------- server playback

/// Rust-only state of the active media-server playback. No token or id is ever sent to the UI.
#[derive(Debug, Clone)]
pub struct PlaybackDescriptor {
    pub connection_id: String,
    pub address: String,
    pub client_id: String,
    pub provider: String,
    pub item_id: String,
    pub token: String,
    pub play_session: String,
    pub rating_key: String,
    pub key_path: String,
    pub duration_ms: i64,
    pub transcoded: bool,
    pub installation_id: Option<String>,
    pub profile_id: Option<String>,
    pub timeline_offset: f64,
}

#[derive(Debug)]
struct PlaybackState {
    descriptor: PlaybackDescriptor,
    started: bool,
    ever_played: bool,
    playing: bool,
    last_position_ms: i64,
    last_report: std::time::Instant,
    sequence: u64,
}

static PLAYING: std::sync::Mutex<Option<PlaybackState>> = std::sync::Mutex::new(None);

pub fn begin_playback(descriptor: PlaybackDescriptor) {
    let mut active = PLAYING.lock().expect("server playback lock");
    *active = Some(PlaybackState {
        descriptor,
        started: false,
        ever_played: false,
        playing: false,
        last_position_ms: 0,
        last_report: std::time::Instant::now(),
        sequence: 0,
    });
}

pub fn end_playback() {
    let mut active = PLAYING.lock().expect("server playback lock");
    if let Some(state) = active.take() {
        if state.started {
            let descriptor = state.descriptor.clone();
            let position = state.last_position_ms;
            tokio::spawn(async move { let _ = report(&descriptor, "stopped", position, 0).await; });
        }
    }
}

/// Mirrors the original `ServerPlayback.ActivePlayback.onSnapshot`: START once, PAUSE/RESUME on a
/// flip, PROGRESS every 10 s or after a 5 s seek. Called from the player's 250 ms snapshot sink.
pub fn observe_snapshot(loaded: bool, paused: bool, buffering: bool, ended: bool, time_pos: f64, duration: f64) {
    let mut active = PLAYING.lock().expect("server playback lock");
    let Some(state) = active.as_mut() else { return };
    if !loaded || buffering {
        return;
    }
    let playing = !paused && !buffering;
    let position = (time_pos * 1_000.0).round() as i64;
    let now = std::time::Instant::now();
    let event = if !state.started && playing {
        state.started = true;
        state.ever_played = true;
        Some("playing")
    } else if state.playing != playing {
        state.playing = playing;
        if playing { state.ever_played = true; }
        Some(if playing { "playing" } else { "paused" })
    } else if ended {
        None
    } else if (position - state.last_position_ms).abs() > 5_000
        || now.duration_since(state.last_report).as_millis() >= 10_000
    {
        Some("playing")
    } else {
        None
    };
    if let Some(event) = event {
        state.last_position_ms = position;
        state.last_report = now;
        state.sequence += 1;
        let descriptor = state.descriptor.clone();
        let sequence = state.sequence;
        tokio::spawn(async move { let _ = report(&descriptor, event, position, sequence).await; });
    }
    if duration.is_finite() && duration > 0.0 {
        state.descriptor.duration_ms = (duration * 1_000.0).round() as i64;
    }
}

async fn report(descriptor: &PlaybackDescriptor, event: &str, position_ms: i64, sequence: u64) -> Result<(), String> {
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    if descriptor.provider == "plex" {
        let state = if event == "stopped" { "stopped" } else if event == "paused" { "paused" } else { "playing" };
        let query = vec![
            ("ratingKey".to_string(), descriptor.rating_key.clone()),
            ("key".to_string(), descriptor.key_path.clone()),
            ("state".to_string(), state.to_string()),
            ("time".to_string(), position_ms.max(0).to_string()),
            ("duration".to_string(), descriptor.duration_ms.max(0).to_string()),
            ("hasMDE".to_string(), "1".into()),
        ];
        let response = client
            .get(format!("{}/:/timeline", descriptor.address))
            .query(&query)
            .header("X-Plex-Token", &descriptor.token)
            .header("X-Plex-Client-Identifier", &descriptor.client_id)
            .header("X-Plex-Session-Identifier", &descriptor.play_session)
            .send().await;
        if let Ok(response) = response {
            let _ = response.bytes().await;
        }
        if event == "stopped" && descriptor.transcoded {
            let _ = client
                .get(format!("{}/video/:/transcode/universal/stop", descriptor.address))
                .query(&[("session", &descriptor.play_session)])
                .header("X-Plex-Token", &descriptor.token)
                .header("X-Plex-Session-Identifier", &descriptor.play_session)
                .send().await;
        }
        Ok(())
    } else if descriptor.provider == "silo" {
        let address = descriptor.address.as_str();
        let installation = descriptor.installation_id.clone().unwrap_or_default();
        let position = position_ms as f64 / 1_000.0 + descriptor.timeline_offset;
        if event == "stopped" {
            let body = json!({
                "installation_id": installation,
                "stop_id": random_id(16),
                "sequence": sequence,
                "position": position,
                "is_paused": true,
            });
            for attempt in 0..3 {
                let response = client
                    .delete(format!("{address}/api/v2/playback/{}", descriptor.play_session))
                    .header("Authorization", format!("Bearer {}", descriptor.token))
                    .header("X-Profile-Id", descriptor.profile_id.clone().unwrap_or_default())
                    .json(&body)
                    .send().await;
                match response {
                    Ok(response) if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND || response.status().as_u16() == 410 => return Ok(()),
                    Ok(_) => {}
                    Err(_) if attempt < 2 => {
                        tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt + 1))).await;
                        continue;
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            return Ok(());
        }
        let body = json!({
            "installation_id": installation,
            "sequence": sequence,
            "position": position,
            "is_paused": event == "paused",
        });
        let response = client
            .post(format!("{address}/api/v2/playback/{}/progress", descriptor.play_session))
            .header("Authorization", format!("Bearer {}", descriptor.token))
            .header("X-Profile-Id", descriptor.profile_id.clone().unwrap_or_default())
            .json(&body)
            .send().await
            .map_err(|e| e.to_string())?;
        if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND || response.status().as_u16() == 410 {
            Ok(())
        } else {
            Err(format!("Silo progress respondeu {}.", response.status().as_u16()))
        }
    } else {
        Ok(())
    }
}


/// Builds a playable `StreamItem` for a `srv1:` card. Tokens stay in the proxy headers, so the
/// local relay (not the WebView) attaches them to the media and segment requests.
pub async fn stream_item(app: &AppStore, profile: i64, connection_id: &str, item_id: &str) -> Result<(StreamItem, PlaybackDescriptor), String> {
    let connection = connections(app, profile)?.into_iter().find(|c| c.id == connection_id).ok_or("servidor não encontrado")?;
    let token = tokens(app).remove(connection_id).ok_or("sessão expirada; conecte novamente")?;
    let cid = client_id(app);
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let mut headers = BTreeMap::new();
    let mut descriptor = PlaybackDescriptor {
        connection_id: connection_id.to_string(),
        address: connection.address.clone(),
        client_id: cid.clone(),
        provider: connection.provider.clone(),
        item_id: item_id.to_string(),
        token: token.clone(),
        play_session: random_id(32),
        rating_key: item_id.to_string(),
        key_path: format!("/library/metadata/{item_id}"),
        duration_ms: 0,
        transcoded: false,
        installation_id: None,
        profile_id: None,
        timeline_offset: 0.0,
    };
    let mut url;
    if connection.provider == "plex" {
        let body = get_json(&client, &format!("{}/library/metadata/{item_id}", connection.address), "plex", Some(&token), app).await?;
        let container = body.get("MediaContainer").unwrap_or(&body);
        let meta = container.get("Metadata").and_then(Value::as_array).and_then(|items| items.first()).ok_or("item Plex não encontrado")?;
        let media = meta.get("Media").and_then(Value::as_array).and_then(|values| values.first());
        let part = media.and_then(|media| media.get("Part")).and_then(Value::as_array).and_then(|parts| parts.first());
        let key = part.and_then(|part| text(part, &["key", "@attributes.key"])).ok_or("este item Plex não tem um arquivo reproduzível")?;
        descriptor.rating_key = text(meta, &["ratingKey", "@attributes.ratingKey"]).unwrap_or(item_id).to_string();
        descriptor.key_path = text(meta, &["key", "@attributes.key"]).unwrap_or(&descriptor.key_path).to_string();
        descriptor.duration_ms = number(meta, &["duration", "@attributes.duration"]).unwrap_or(0);
        let direct_container = media.and_then(|m| text(m, &["container", "@attributes.container"])).unwrap_or("");
        let direct_video = media.and_then(|m| text(m, &["videoCodec", "@attributes.videoCodec"])).unwrap_or("");
        let direct_audio = media.and_then(|m| text(m, &["audioCodec", "@attributes.audioCodec"])).unwrap_or("");
        let allowed_container = ["mkv", "mp4", "mov", "ts", "avi", "webm"].contains(&direct_container.to_ascii_lowercase().as_str());
        let allowed_video = direct_video.is_empty() || ["h264", "hevc", "h265", "vp9", "av1"].contains(&direct_video.to_ascii_lowercase().as_str());
        let allowed_audio = direct_audio.is_empty() || ["aac", "ac3", "eac3", "mp3", "flac", "opus", "vorbis"].contains(&direct_audio.to_ascii_lowercase().as_str());
        if allowed_container && allowed_video && allowed_audio {
            let path = if key.starts_with('/') { key.to_string() } else { format!("/library/parts/{key}/file") };
            url = format!("{}{path}?X-Plex-Session-Identifier={}", connection.address, descriptor.play_session);
        } else {
            let query = vec![
                ("path".to_string(), descriptor.key_path.clone()),
                ("mediaIndex".to_string(), "0".into()),
                ("partIndex".to_string(), "0".into()),
                ("protocol".to_string(), "hls".into()),
                ("fastSeek".to_string(), "1".into()),
                ("directPlay".to_string(), "0".into()),
                ("directStream".to_string(), "1".into()),
                ("directStreamAudio".to_string(), "1".into()),
                ("subtitles".to_string(), "none".into()),
                ("session".to_string(), descriptor.play_session.clone()),
                ("copyts".to_string(), "1".into()),
                ("hasMDE".to_string(), "1".into()),
            ];
            let decision = get_json(&client, &format!("{}/video/:/transcode/universal/decision", connection.address), "plex", Some(&token), app).await.ok();
            if let Some(decision) = decision {
                let container = decision.get("MediaContainer").unwrap_or(&decision);
                let code = number(container, &["generalDecisionCode", "@attributes.generalDecisionCode"]).unwrap_or(0);
                if code >= 2_000 {
                    return Err("Este item não pode ser reproduzido pelo Plex.".into());
                }
            }
            descriptor.transcoded = true;
            url = format!("{}/video/:/transcode/universal/start.m3u8", connection.address);
            headers.insert("X-Plex-Client-Profile-Extra".into(), PROFILE_EXTRA.into());
        }
        headers.insert("X-Plex-Token".into(), token.clone());
        headers.insert("X-Plex-Client-Identifier".into(), cid);
        headers.insert("X-Plex-Session-Identifier".into(), descriptor.play_session.clone());
    } else if connection.provider == "silo" {
        let (access, profile_id) = token.split_once('|').unwrap_or((&token, &connection.user_id.as_str()));
        descriptor.profile_id = Some(profile_id.to_string());
        let installation = random_id(32);
        descriptor.installation_id = Some(installation.clone());
        let body = post_json(&client, &format!("{}/api/v2/playback/start", connection.address), "silo", Some(access), app, json!({
            "installation_id": installation,
            "protocol_version": 3,
            "client_features": ["playback_plan_v3", "direct_stream_resume_v1"],
            "file_id": item_id,
            "profile_id": profile_id,
            "playback_attempt_id": random_id(32),
            "quality_preference": "auto",
            "start_position": 0,
        })).await?;
        let plan = body.get("playback_plan").or_else(|| body.get("plan")).unwrap_or(&body);
        let session = text(&body, &["session_id", "sessionId", "playback_session_id"])
            .or_else(|| body.pointer("/session/id").and_then(Value::as_str))
            .ok_or("Silo não devolveu a sessão de reprodução")?;
        descriptor.play_session = session.to_string();
        descriptor.timeline_offset = plan.pointer("/timeline/offset_seconds").and_then(Value::as_f64)
            .or_else(|| body.pointer("/timeline/offset_seconds").and_then(Value::as_f64)).unwrap_or(0.0);
        let delivery = text(plan, &["delivery"]).unwrap_or("");
        descriptor.transcoded = !(delivery.is_empty() || delivery == "original_http" || delivery.starts_with("server_remux"));
        url = plan.pointer("/stream/url").and_then(Value::as_str)
            .or_else(|| plan.get("url").and_then(Value::as_str))
            .or_else(|| body.get("url").and_then(Value::as_str))
            .ok_or("Silo não devolveu uma URL de reprodução")?.to_string();
        if url.starts_with('/') { url = format!("{}{}", connection.address, url); }
        headers.insert("Authorization".into(), format!("Bearer {access}"));
        headers.insert("X-Profile-Id".into(), profile_id.to_string());
    } else {
        let path = api_path(&connection.provider);
        url = format!("{}{}/Items/{item_id}/Stream", connection.address, path);
        headers.insert("X-Emby-Authorization".into(), request_headers(&connection.provider, Some(&token), &cid)
            .into_iter().find(|(name, _)| name == "X-Emby-Authorization").map(|(_, value)| value).unwrap_or_default());
    }
    let item = StreamItem {
        name: Some(connection.name.clone()),
        title: Some("Servidor".into()),
        description: Some(connection.provider.clone()),
        url: Some(url),
        addon_name: "Media server".into(),
        addon_id: format!("server:{}", connection.provider),
        behavior_hints: StreamBehaviorHints {
            not_web_ready: true,
            proxy_headers: Some(StreamProxyHeaders { request: Some(headers), response: None }),
            ..Default::default()
        },
        ..Default::default()
    };
    Ok((item, descriptor))
}

const PROFILE_EXTRA: &str = "add-transcode-target(type=videoProfile&context=streaming&protocol=hls&container=mpegts&videoCodec=h264,hevc&audioCodec=aac,ac3,eac3,mp3&replace=true)+add-transcode-target-audio-codec(type=audioProfile&context=streaming&protocol=hls&container=mpegts&audioCodec=ac3,eac3)+add-direct-play-profile(type=videoProfile&container=mkv,mp4,mov,ts,avi,webm&videoCodec=*&audioCodec=*)";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_normalized_like_the_original() {
        assert_eq!(normalize_address(" 192.168.1.2:8096/web "), "http://192.168.1.2:8096");
        assert_eq!(normalize_address("https://media.test/emby/"), "https://media.test");
    }

    #[test]
    fn plex_items_map_with_the_token_only_in_the_image_url() {
        let connection = Connection { id: "srv-plex".into(), provider: "plex".into(), address: "http://plex.test:32400".into(), ..Default::default() };
        let item = json!({
            "ratingKey": "42", "type": 1, "title": "Film", "year": 2024,
            "thumb": "/library/metadata/42/thumb/123", "duration": 7_200_000,
            "summary": "Desc", "Genre": [{"tag": "Drama"}],
        });
        let preview = preview_plex(&connection, &item, "secret").unwrap();
        assert_eq!(preview.id, "srv1:srv-plex:42");
        assert_eq!(preview.content_type, "movie");
        assert_eq!(preview.release_info.as_deref(), Some("2024"));
        assert_eq!(preview.genres, vec!["Drama"]);
        assert!(preview.poster.as_deref().is_some_and(|url| url.contains("X-Plex-Token=secret")));
    }

    #[test]
    fn provider_headers_keep_tokens_out_of_the_ui() {
        let plex = request_headers("plex", Some("pk"), "client-1");
        assert!(plex.iter().any(|(name, value)| name == "X-Plex-Token" && value == "pk"));
        let silo = request_headers("silo", Some("access|profile-2"), "client-1");
        assert!(silo.iter().any(|(name, value)| name == "X-Profile-Id" && value == "profile-2"));
        assert!(silo.iter().any(|(name, value)| name == "Authorization" && value == "Bearer access"));
    }

    #[test]
    fn items_map_to_previews_without_a_token_in_the_url() {
        let connection = Connection { id: "srv-1".into(), address: "http://x".into(), ..Default::default() };
        let item = json!({ "Id": "abc", "Type": "Movie", "Name": "Film", "ProductionYear": 2020, "ImageTags": { "Primary": "tag" }, "Genres": ["Drama"] });
        let preview = preview(&connection, &item).unwrap();
        assert_eq!(preview.id, "srv1:srv-1:abc");
        assert!(preview.poster.as_deref().is_some_and(|url| !url.contains("secret-token")));
        assert_eq!(preview.release_info.as_deref(), Some("2020"));
        assert_eq!(preview.genres, vec!["Drama"]);
    }
}
