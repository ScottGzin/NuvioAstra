//! Public Trakt list sources of collection folders (`TraktPublicListSourceResolver.kt`).
//!
//! Only the public API is used: a client id (no OAuth token) plus `trakt-api-version: 2` and the
//! required `User-Agent`. Personal list ids, not slugs or user names, are stored in the collection.

use nuvio_core::catalog::{CatalogPage, MetaPreview, PosterShape};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(12);
const PAGE_LIMIT: usize = 50;
const LIST_LIMIT: usize = 20;

fn base() -> &'static str {
    option_env!("NUVIO_TRAKT_API_URL").unwrap_or("https://api.trakt.tv")
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Source {
    pub list_id: i64,
    #[serde(default = "default_media")]
    pub media_type: String,
    #[serde(default = "default_sort")]
    pub sort_by: String,
    #[serde(default = "default_how")]
    pub sort_how: String,
}

fn default_media() -> String { "MOVIE".into() }
fn default_sort() -> String { "rank".into() }
fn default_how() -> String { "asc".into() }

impl Source {
    pub fn api_type(&self) -> &'static str {
        if self.media_type.eq_ignore_ascii_case("TV") { "show" } else { "movie" }
    }
    pub fn content_type(&self) -> &'static str {
        if self.media_type.eq_ignore_ascii_case("TV") { "series" } else { "movie" }
    }
    pub fn sort_by(&self) -> String {
        let value = self.sort_by.trim().to_ascii_lowercase();
        if ["rank", "added", "title", "released", "runtime", "popularity", "percentage", "votes"].contains(&value.as_str()) {
            value
        } else {
            "rank".into()
        }
    }
    pub fn sort_how(&self) -> String {
        let value = self.sort_how.trim().to_ascii_lowercase();
        if value == "desc" { "desc".into() } else { "asc".into() }
    }
    /// Stable tab key of the original (`catalogRouteKey`), used by the folder view and the page IPC.
    pub fn key(&self) -> String {
        format!("trakt_{}_{}_{}_{}", self.list_id, self.api_type(), self.sort_by(), self.sort_how())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListResult {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub cover: Option<String>,
    pub item_count: i64,
    pub likes: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedList {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub cover: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TraktComment {
    pub id: i64,
    pub author: String,
    pub username: String,
    pub text: String,
    pub spoiler: bool,
    pub review: bool,
    pub likes: i64,
    pub rating: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TraktCommentsPage {
    pub items: Vec<TraktComment>,
    pub page: u32,
    pub page_count: u32,
    pub item_count: u32,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(format!("NuvioAstra/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

async fn get_authed_optional(client_id: &str, token: Option<&str>, path: &str, query: &[(String, String)]) -> Result<(Value, HeaderMap), String> {
    if let Some(token) = token.filter(|value| !value.is_empty()) {
        return get_authed(client_id, token, path, query).await;
    }
    get(client_id, path, query).await
}

async fn get(client_id: &str, path: &str, query: &[(String, String)]) -> Result<(Value, HeaderMap), String> {
    if client_id.trim().is_empty() {
        return Err("Credenciais do Trakt não configuradas.".into());
    }
    let response = client()?
        .get(format!("{}{}", base(), path))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("trakt-api-version", "2")
        .header("trakt-api-key", client_id.trim())
        .query(query)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let headers = response.headers().clone();
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND {
        return Err("Lista do Trakt não encontrada ou não é pública.".into());
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("Limite de requisições do Trakt atingido.".into());
    }
    if !status.is_success() {
        return Err(format!("Trakt respondeu {}.", status.as_u16()));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    Ok((body, headers))
}

fn text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(*key).and_then(Value::as_str).filter(|v| !v.is_empty()))
}

fn integer(value: &Value, keys: &[&str]) -> i64 {
    keys.iter().find_map(|key| match value.get(*key) {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }).unwrap_or(0)
}

fn image(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        match value.get(*key) {
            Some(Value::String(url)) if !url.is_empty() => return Some(https(url)),
            Some(Value::Array(list)) => {
                if let Some(url) = list.iter().filter_map(Value::as_str).find(|url| !url.is_empty()) {
                    return Some(https(url));
                }
            }
            _ => {}
        }
    }
    None
}

fn https(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("//") {
        format!("https:{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        url.to_string()
    }
}

fn list_result(value: &Value) -> Option<(ListResult, String)> {
    let list = if value.get("list").is_some() { value.get("list")? } else { value };
    let ids = list.get("ids")?;
    let id = integer(ids, &["trakt"]);
    if id <= 0 {
        return None;
    }
    let slug = text(ids, &["slug"]).unwrap_or_default().to_string();
    let images = list.get("images").unwrap_or(&Value::Null);
    Some((ListResult {
        id,
        slug: slug.clone(),
        title: text(list, &["name"]).unwrap_or("Trakt").to_string(),
        cover: image(images, &["poster", "posters"]),
        item_count: integer(list, &["item_count"]),
        likes: {
            let likes = integer(list, &["likes"]);
            if likes == 0 { integer(value, &["like_count"]) } else { likes }
        },
    }, slug))
}

/// `parseTraktListPath` / `parseTraktListInput`: numeric id, `?id=`, `trakt.tv/lists/…`,
/// `trakt.tv/users/…/lists/…` or a bare slug (resolved through the API).
pub fn parse_trakt_list_input(input: &str) -> Option<(String, bool)> {
    let value = input.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(id) = value.parse::<i64>() {
        return Some((id.to_string(), true));
    }
    if let Some(start) = value.find("id=") {
        let id = value[start + 3..].split(['&', '#', '/']).next().unwrap_or_default();
        if let Ok(id) = id.parse::<i64>() {
            return Some((id.to_string(), true));
        }
    }
    let lower = value.to_ascii_lowercase();
    for marker in ["trakt.tv/lists/", "trakt.tv/users/"] {
        if let Some(at) = lower.find(marker) {
            let rest = &value[at + marker.len()..];
            let segment = rest.split(['/', '?', '#']).next().unwrap_or_default();
            if marker.ends_with("users/") {
                // user name consumed above; find the following `lists/` segment.
                let after = rest.split_once('/').map(|(_, tail)| tail).unwrap_or_default();
                let slug = after.strip_prefix("lists/").or_else(|| after.strip_prefix("lists/")).unwrap_or(after);
                let slug = slug.split(['?', '#']).next().unwrap_or_default();
                if !slug.is_empty() { return Some((slug.to_string(), slug.parse::<i64>().is_ok())); }
            } else if !segment.is_empty() {
                return Some((segment.to_string(), segment.parse::<i64>().is_ok()));
            }
        }
    }
    if value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Some((value.to_string(), false));
    }
    None
}

pub async fn resolve_list_input(client_id: &str, input: &str) -> Result<ResolvedList, String> {
    let (path, numeric) = parse_trakt_list_input(input).ok_or("Informe um id, URL ou slug de lista do Trakt.".to_string())?;
    let (body, _) = get(client_id, &format!("/lists/{path}"), &[("extended".into(), "full,images".into())]).await?;
    let (found, _) = list_result(&body).ok_or("A lista do Trakt não tem um id numérico.".to_string())?;
    let title = text(&body, &["name"]).filter(|name| !name.is_empty()).unwrap_or(&found.title).to_string();
    Ok(ResolvedList { id: found.id, slug: if numeric { found.slug } else { path }, title, cover: found.cover })
}

pub async fn search_lists(client_id: &str, query: &str) -> Result<Vec<ListResult>, String> {
    let (body, _) = get(client_id, "/search/list", &[
        ("query".into(), query.to_string()),
        ("extended".into(), "full,images".into()),
        ("page".into(), "1".into()),
        ("limit".into(), LIST_LIMIT.to_string()),
    ]).await?;
    let empty = Vec::new();
    Ok(body.as_array().unwrap_or(&empty).iter()
        .filter(|item| text(item, &["type"]) == Some("list"))
        .filter_map(list_result)
        .map(|(list, _)| list)
        .collect())
}

pub async fn prominent_lists(client_id: &str, kind: &str) -> Result<Vec<ListResult>, String> {
    let (body, _) = get(client_id, &format!("/lists/{kind}"), &[
        ("extended".into(), "full,images".into()),
        ("page".into(), "1".into()),
        ("limit".into(), LIST_LIMIT.to_string()),
    ]).await?;
    let empty = Vec::new();
    Ok(body.as_array().unwrap_or(&empty).iter().filter_map(list_result).map(|(list, _)| list).collect())
}

/// Same picker calls with an optional account token: private lists become visible when signed in.
pub async fn resolve_list_input_authed(client_id: &str, token: Option<&str>, input: &str) -> Result<ResolvedList, String> {
    let (path, numeric) = parse_trakt_list_input(input).ok_or("Informe um id, URL ou slug de lista do Trakt.".to_string())?;
    let (body, _) = get_authed_optional(client_id, token, &format!("/lists/{path}"), &[("extended".into(), "full,images".into())]).await?;
    let (found, _) = list_result(&body).ok_or("A lista do Trakt não tem um id numérico.".to_string())?;
    let title = text(&body, &["name"]).filter(|name| !name.is_empty()).unwrap_or(&found.title).to_string();
    Ok(ResolvedList { id: found.id, slug: if numeric { found.slug } else { path }, title, cover: found.cover })
}

pub async fn search_lists_authed(client_id: &str, token: Option<&str>, query: &str) -> Result<Vec<ListResult>, String> {
    let (body, _) = get_authed_optional(client_id, token, "/search/list", &[
        ("query".into(), query.to_string()),
        ("extended".into(), "full,images".into()),
        ("page".into(), "1".into()),
        ("limit".into(), LIST_LIMIT.to_string()),
    ]).await?;
    let empty = Vec::new();
    Ok(body.as_array().unwrap_or(&empty).iter()
        .filter(|item| text(item, &["type"]) == Some("list"))
        .filter_map(list_result).map(|(list, _)| list).collect())
}

pub async fn prominent_lists_authed(client_id: &str, token: Option<&str>, kind: &str) -> Result<Vec<ListResult>, String> {
    let (body, _) = get_authed_optional(client_id, token, &format!("/lists/{kind}"), &[
        ("extended".into(), "full,images".into()),
        ("page".into(), "1".into()),
        ("limit".into(), LIST_LIMIT.to_string()),
    ]).await?;
    let empty = Vec::new();
    Ok(body.as_array().unwrap_or(&empty).iter().filter_map(list_result).map(|(list, _)| list).collect())
}

/// Lists of the signed-in user (`GET /users/me/lists`), including private ones.
pub async fn my_lists(client_id: &str, token: &str) -> Result<Vec<ListResult>, String> {
    let (body, _) = get_authed(client_id, token, "/users/me/lists", &[
        ("extended".into(), "full,images".into()),
        ("limit".into(), LIST_LIMIT.to_string()),
    ]).await?;
    let empty = Vec::new();
    Ok(body.as_array().unwrap_or(&empty).iter().filter_map(list_result).map(|(list, _)| list).collect())
}

// ---------------------------------------------------------------- comments (OAuth required)

/// The original's comments are NOT public: an authenticated Trakt account is required.
async fn get_authed(client_id: &str, token: &str, path: &str, query: &[(String, String)]) -> Result<(Value, HeaderMap), String> {
    let response = client()?
        .get(format!("{}{}", base(), path))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("trakt-api-version", "2")
        .header("trakt-api-key", client_id.trim())
        .header("Authorization", format!("Bearer {token}"))
        .query(query)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let headers = response.headers().clone();
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok((Value::Array(Vec::new()), headers));
    }
    if !status.is_success() {
        return Err(format!("Trakt respondeu {}.", status.as_u16()));
    }
    Ok((response.json().await.map_err(|e| e.to_string())?, headers))
}

fn strip_spoilers(text_value: &str) -> (String, bool) {
    let lower = text_value.to_ascii_lowercase();
    let mut output = String::with_capacity(text_value.len());
    let mut cursor = 0;
    let mut found = false;
    while let Some(start_rel) = lower[cursor..].find("[spoiler]") {
        let start = cursor + start_rel;
        output.push_str(&text_value[cursor..start]);
        let after = start + "[spoiler]".len();
        match lower[after..].find("[/spoiler]") {
            Some(end_rel) => {
                cursor = after + end_rel + "[/spoiler]".len();
            }
            None => {
                cursor = text_value.len();
            }
        }
        found = true;
    }
    output.push_str(&text_value[cursor..]);
    (output.trim().to_string(), found)
}

fn header_number(headers: &HeaderMap, name: &str, fallback: u32) -> u32 {
    headers.get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(fallback)
}

/// Resolves a title id to Trakt's path id: IMDb is used directly, TMDB ids go through
/// `/search/tmdb/{id}` and the best id is imdb → numeric trakt → slug (`bestPathId`).
async fn comments_path_id(client_id: &str, token: &str, content_type: &str, id: &str) -> Result<String, String> {
    if id.starts_with("tt") && id.len() >= 7 {
        return Ok(id.to_string());
    }
    let tmdb = id.strip_prefix("tmdb:").unwrap_or(id);
    if !tmdb.bytes().all(|byte| byte.is_ascii_digit()) || tmdb.is_empty() {
        return Err("Este título não tem um identificador IMDb/TMDB para comentários.".into());
    }
    let kind = if content_type == "movie" { "movie" } else { "show" };
    let (body, _) = get_authed(client_id, token, &format!("/search/tmdb/{tmdb}"), &[("type".into(), kind.into())]).await?;
    let empty = Vec::new();
    let results = body.as_array().unwrap_or(&empty);
    for result in results {
        if text(result, &["type"]) != Some(kind) {
            continue;
        }
        let item = if kind == "movie" { result.get("movie") } else { result.get("show") };
        if let Some(ids) = item.and_then(|item| item.get("ids")) {
            if let Some(imdb) = text(ids, &["imdb"]) {
                return Ok(imdb.to_string());
            }
            let trakt = integer(ids, &["trakt"]);
            if trakt > 0 {
                return Ok(trakt.to_string());
            }
            if let Some(slug) = text(ids, &["slug"]) {
                return Ok(slug.to_string());
            }
        }
    }
    Err("O Trakt não encontrou este título.".into())
}

pub async fn comments_page(client_id: &str, token: &str, content_type: &str, id: &str, page: u32) -> Result<TraktCommentsPage, String> {
    let page = page.max(1);
    let path_id = comments_path_id(client_id, token, content_type, id).await?;
    let kind = if content_type == "movie" { "movies" } else { "shows" };
    let (body, headers) = get_authed(
        client_id,
        token,
        &format!("/{kind}/{path_id}/comments/likes"),
        &[("page".into(), page.to_string()), ("limit".into(), "100".into())],
    ).await?;
    let empty = Vec::new();
    let comments = body.as_array().unwrap_or(&empty);
    let mut items = Vec::new();
    for comment in comments {
        let Some(raw) = text(comment, &["comment"]) else { continue };
        let (cleaned, inline_spoiler) = strip_spoilers(raw);
        if cleaned.is_empty() {
            continue;
        }
        let user = comment.get("user").unwrap_or(&Value::Null);
        let spoiler = comment.get("spoiler").and_then(Value::as_bool).unwrap_or(false) || inline_spoiler;
        items.push(TraktComment {
            id: integer(comment, &["id"]),
            author: text(user, &["name"]).or_else(|| text(user, &["username"])).unwrap_or("Trakt user").to_string(),
            username: text(user, &["username"]).unwrap_or_default().to_string(),
            text: cleaned,
            spoiler,
            review: comment.get("review").and_then(Value::as_bool).unwrap_or(false),
            likes: integer(comment, &["likes"]),
            rating: comment.pointer("/user_stats/rating").and_then(Value::as_i64)
                .or_else(|| comment.pointer("/user_stats/rating").and_then(Value::as_f64).map(|v| v.round() as i64)),
            created_at: text(comment, &["created_at"]).unwrap_or_default().to_string(),
            updated_at: text(comment, &["updated_at"]).unwrap_or_default().to_string(),
        });
    }
    Ok(TraktCommentsPage {
        items,
        page,
        page_count: header_number(&headers, "x-pagination-page-count", page).max(page),
        item_count: header_number(&headers, "x-pagination-item-count", comments.len() as u32),
    })
}

fn preview(item: &Value, source: &Source) -> Option<MetaPreview> {
    let movie = if source.api_type() == "show" { item.get("show") } else { item.get("movie") }?;
    let title = text(movie, &["title"])?.to_string();
    let ids = movie.get("ids")?;
    let id = text(ids, &["imdb"])
        .map(str::to_string)
        .or_else(|| integer(ids, &["tmdb"]).checked_add(0).filter(|v| *v > 0).map(|v| format!("tmdb:{v}")))
        .or_else(|| integer(ids, &["trakt"]).checked_add(0).filter(|v| *v > 0).map(|v| format!("trakt:{v}")))
        .or_else(|| text(ids, &["slug"]).map(str::to_string))?;
    let images = movie.get("images").unwrap_or(&Value::Null);
    let poster = image(images, &["poster"]);
    let fanart = image(images, &["fanart"]);
    let banner = image(images, &["banner"]);
    let thumb = image(images, &["thumb"]);
    let release = text(movie, &["released"]).or_else(|| text(movie, &["first_aired"]));
    let rating = movie.get("rating").and_then(Value::as_f64)
        .map(|value| format!("{:.1}", (value * 10.0).round() / 10.0));
    Some(MetaPreview {
        id,
        content_type: source.content_type().into(),
        name: title,
        poster: poster.clone().or_else(|| fanart.clone()),
        banner: fanart.or(banner).or(thumb).or_else(|| poster.clone()),
        logo: image(images, &["logo", "clearart"]),
        poster_shape: PosterShape::Poster,
        description: text(movie, &["overview"]).map(str::to_string),
        release_info: integer(movie, &["year"]).checked_add(0).filter(|v| *v > 0).map(|v| v.to_string())
            .or_else(|| release.map(|value| value.chars().take(4).collect())),
        raw_release_date: release.map(str::to_string),
        imdb_rating: rating,
        genres: movie.get("genres").and_then(Value::as_array).map(|values| {
            values.iter().filter_map(Value::as_str).map(str::to_string).collect()
        }).unwrap_or_default(),
        landscape_poster: poster,
        runtime: None,
        status: None,
        raw_poster_url: None, raw_landscape_poster_url: None,
    })
}

fn page_count(headers: &HeaderMap) -> usize {
    headers.get("x-pagination-page-count")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(1)
}

pub async fn list_items_page(client_id: &str, source: &Source, page: u32) -> Result<CatalogPage, String> {
    let page = page.max(1);
    let (body, headers) = get(client_id, &format!("/lists/{}/items/{}", source.list_id, source.api_type()), &[
        ("extended".into(), "full,images".into()),
        ("page".into(), page.to_string()),
        ("limit".into(), PAGE_LIMIT.to_string()),
        ("sort_by".into(), source.sort_by()),
        ("sort_how".into(), source.sort_how()),
    ]).await?;
    let empty = Vec::new();
    let items = body.as_array().unwrap_or(&empty);
    let mut previews = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in items {
        if let Some(preview) = preview(item, source) {
            if seen.insert(preview.stable_key()) {
                previews.push(preview);
            }
        }
    }
    let raw = previews.len();
    let next = ((page as usize) < page_count(&headers) && !previews.is_empty()).then_some(page + 1);
    Ok(CatalogPage { items: previews, raw_item_count: raw, next_skip: next })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ids_urls_and_slugs() {
        assert_eq!(parse_trakt_list_input("123456"), Some(("123456".into(), true)));
        assert_eq!(parse_trakt_list_input("https://trakt.tv/lists/123456"), Some(("123456".into(), true)));
        assert_eq!(parse_trakt_list_input("https://trakt.tv/users/nuvio/lists/123456"), Some(("123456".into(), true)));
        assert_eq!(parse_trakt_list_input("https://example.com/import?id=123456"), Some(("123456".into(), true)));
        assert_eq!(parse_trakt_list_input("my-list"), Some(("my-list".into(), false)));
        assert_eq!(parse_trakt_list_input(""), None);
    }

    #[test]
    fn sorts_and_media_are_normalized() {
        let source = Source { list_id: 1, media_type: "TV".into(), sort_by: "unknown".into(), sort_how: "DESC".into() };
        assert_eq!(source.api_type(), "show");
        assert_eq!(source.content_type(), "series");
        assert_eq!(source.sort_by(), "rank");
        assert_eq!(source.sort_how(), "desc");
        assert_eq!(source.key(), "trakt_1_show_rank_desc");
    }

    #[test]
    fn strips_inline_spoilers() {
        assert_eq!(
            strip_spoilers("Nice [SPOILER]he dies[/spoiler] end"),
            ("Nice  end".to_string(), true)
        );
        assert_eq!(strip_spoilers("plain text"), ("plain text".to_string(), false));
    }

    #[test]
    fn maps_list_items_to_previews() {
        let source = Source { list_id: 1, media_type: "MOVIE".into(), sort_by: "rank".into(), sort_how: "asc".into() };
        let item: Value = serde_json::from_str(r#"{"rank":1,"movie":{"title":"Film","year":2024,"ids":{"trakt":10,"imdb":"tt123","tmdb":55,"slug":"film"},"overview":"Desc","rating":8.5,"genres":["Drama"],"images":{"poster":"http://img/p.jpg","fanart":"//img/f.jpg"}}}"#).unwrap();
        let preview = preview(&item, &source).unwrap();
        assert_eq!(preview.id, "tt123");
        assert_eq!(preview.content_type, "movie");
        assert_eq!(preview.poster.as_deref(), Some("https://img/p.jpg"));
        assert_eq!(preview.imdb_rating.as_deref(), Some("8.5"));
        assert_eq!(preview.genres, vec!["Drama"]);
    }
}
