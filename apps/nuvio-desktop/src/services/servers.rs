//! Media servers: Jellyfin and Emby (the original's MediaBrowser provider). Credentials belong to
//! the user's own server: the address, username and password are sent only to that server, and the
//! resulting access token stays in a Rust-only document (never returned to the UI).

use nuvio_core::catalog::{CatalogPage, MetaPreview, PosterShape};
use nuvio_store::nextgen::AppStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Connection {
    pub id: String,
    /// `jellyfin` or `emby`. Plex and Silo use different login flows and are not wired yet.
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

fn auth_header(provider: &str, token: Option<&str>) -> String {
    let token = token.filter(|t| !t.is_empty()).map(|t| format!(", Token=\"{t}\"")).unwrap_or_default();
    format!("MediaBrowser Client=\"Nuvio\", Device=\"NuvioDesktop\", DeviceId=\"nuvio-desktop\", Version=\"{}\"{token}", env!("CARGO_PKG_VERSION"))
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
    let response = client
        .post(&url)
        .header("X-Emby-Authorization", auth_header(&provider, None))
        .header("Authorization", auth_header(&provider, None))
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

async fn get_json(client: &reqwest::Client, url: &str, provider: &str, token: &str) -> Result<Value, String> {
    let response = client.get(url)
        .header("X-Emby-Authorization", auth_header(provider, Some(token)))
        .header("Authorization", auth_header(provider, Some(token)))
        .header("Accept", "application/json")
        .send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.json().await.map_err(|e| e.to_string())
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
    let body = get_json(&client, &url, &connection.provider, &token).await?;
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
    })
}

/// `GET /Items` of one library, mapped to the same shape addon catalogs use.
pub async fn catalog(app: &AppStore, profile: i64, connection_id: &str, library_id: &str, skip: u32) -> Result<CatalogPage, String> {
    let connection = connections(app, profile)?.into_iter().find(|c| c.id == connection_id).ok_or("servidor não encontrado")?;
    // The token is used only here; `preview` never receives it, so image URLs stay token-free.
    let token = tokens(app).remove(connection_id).ok_or("sessão expirada; conecte novamente")?;
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let url = if connection.provider == "emby" {
        format!("{}{}/Users/{}/Items?ParentId={library_id}&StartIndex={skip}&Limit=60&Recursive=true", connection.address, api_path(&connection.provider), connection.user_id)
    } else {
        format!("{}/Items?ParentId={library_id}&StartIndex={skip}&Limit=60&Recursive=true", connection.address)
    };
    let body = get_json(&client, &url, &connection.provider, &token).await?;
    let raw = body.get("TotalRecordCount").and_then(Value::as_u64).unwrap_or(0) as usize;
    let items = body.get("Items").and_then(Value::as_array).cloned().unwrap_or_default();
    let previews: Vec<MetaPreview> = items.iter().filter_map(|item| preview(&connection, item)).collect();
    let next = (skip as usize + previews.len() < raw).then_some(skip + 60);
    Ok(CatalogPage { items: previews, raw_item_count: raw, next_skip: next })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_normalized_like_the_original() {
        assert_eq!(normalize_address(" 192.168.1.2:8096/web "), "http://192.168.1.2:8096");
        assert_eq!(normalize_address("https://media.test/emby/"), "https://media.test");
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
