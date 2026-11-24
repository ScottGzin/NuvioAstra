//! Catalog responses (`/catalog/{type}/{id}[/{extra}].json`).
//!
//! Port of `HomeCatalogParser.parseCatalogResponse` and `fetchCatalogPage` paging rules:
//! items without id/type/name are skipped, duplicates (`type:id`) are dropped, and the next
//! `skip` is `requested skip + raw item count` when the page had items.

use crate::manifest::{array, optional_string, string_list};
use crate::CoreError;
use serde::Serialize;
use serde_json::Value;

/// Serialized in camelCase: this is the contract read by the UI (`web/src/types.ts`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPreview {
    pub id: String,
    pub content_type: String,
    pub name: String,
    pub poster: Option<String>,
    pub banner: Option<String>,
    pub logo: Option<String>,
    pub poster_shape: PosterShape,
    pub description: Option<String>,
    pub release_info: Option<String>,
    pub raw_release_date: Option<String>,
    pub imdb_rating: Option<String>,
    pub genres: Vec<String>,
    pub landscape_poster: Option<String>,
    pub runtime: Option<String>,
    pub status: Option<String>,
    /// Original artwork kept when a custom poster source replaces it, for image-error fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_poster_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_landscape_poster_url: Option<String>,
}

/// IDs extracted from an item id, for the original's `{imdb_id}` / `{id_type}` poster tokens.
struct PosterIds {
    full: String,
    id_type: String,
    typed: String,
    ids: std::collections::HashMap<String, String>,
}

fn poster_ids(id: &str, content_type: &str) -> PosterIds {
    let mut ids = std::collections::HashMap::new();
    let mut id_type = "imdb".to_string();
    if id.starts_with("tt") && id[2..].bytes().all(|b| b.is_ascii_digit()) {
        ids.insert("imdb".into(), id.to_string());
    } else if id.starts_with("srv1:") {
        // Media-server items have no IMDb/TMDB id: only `{id}` / `{type}` / `{shape}` can resolve.
        id_type = "server".to_string();
        ids.insert("server".into(), id.to_string());
        return PosterIds { full: id.to_string(), id_type, typed: id.to_string(), ids };
    } else {
        let parts: Vec<&str> = id.split(':').collect();
        let known = ["imdb", "tmdb", "tvdb", "mal", "kitsu", "anilist", "anidb"];
        let mut first_known = false;
        for part in &parts {
            if !first_known && known.contains(part) {
                id_type = (*part).to_string();
                first_known = true;
                continue;
            }
            if first_known && !ids.contains_key(&id_type) {
                ids.insert(id_type.clone(), (*part).to_string());
                break;
            }
        }
    }
    let typed = if let Some(imdb) = ids.get("imdb") {
        imdb.clone()
    } else if let Some(value) = ids.get("tmdb").or_else(|| ids.get("tvdb")) {
        format!("{}-{}", if content_type == "movie" { "movie" } else { "series" }, value)
    } else {
        id.to_string()
    };
    PosterIds { full: id.to_string(), id_type, typed, ids }
}

/// Resolves one custom poster template. Port of `CustomPosterUrlResolver.resolve`: `{id}`,
/// `{id_type}`, `{typed_id}`, `{type}` and `{shape}`, per-service ids, optional `id?` and pipe
/// alternates (`{imdb_id|kitsu_id}`). A missing required id returns `None` (keep the original URL).
pub fn resolve_poster_template(pattern: &str, id: &str, content_type: &str, shape: &str) -> Option<String> {
    if pattern.trim().is_empty() {
        return None;
    }
    let decoded = pattern.replace("%7B", "{").replace("%7D", "}").replace("%7C", "|")
        .replace("%7b", "{").replace("%7d", "}").replace("%7c", "|");
    let ids = poster_ids(id, content_type);
    let mut out = String::with_capacity(decoded.len());
    let bytes = decoded.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'{' {
            let ch = decoded[index..].chars().next()?;
            out.push(ch);
            index += ch.len_utf8();
            continue;
        }
        let Some(relative_end) = decoded[index + 1..].find('}') else {
            out.push('{');
            index += 1;
            continue;
        };
        let end = index + 1 + relative_end;
        let expression = &decoded[index + 1..end];
        let mut replaced: Option<String> = None;
        for candidate in expression.split('|') {
            let optional = candidate.ends_with('?');
            let name = candidate.trim_end_matches('?').trim();
            let value = match name {
                "id" => Some(ids.full.clone()),
                "id_type" => Some(ids.id_type.clone()),
                "typed_id" => Some(ids.typed.clone()),
                "type" => Some(content_type.to_string()),
                "shape" => Some(shape.to_string()),
                other => ids.ids.get(other.strip_suffix("_id").unwrap_or(other)).cloned(),
            };
            if let Some(value) = value.filter(|v| !v.is_empty()) {
                replaced = Some(value);
                break;
            }
            if optional {
                replaced = Some(String::new());
                break;
            }
        }
        match replaced {
            Some(value) => out.push_str(&value),
            None => {
                let known = |name: &str| {
                    let name = name.trim_end_matches('?').trim().strip_suffix("_id").unwrap_or(name.trim_end_matches('?').trim());
                    matches!(name, "id" | "id_type" | "typed_id" | "type" | "shape" | "imdb" | "tmdb" | "tvdb" | "mal" | "kitsu" | "anilist" | "anidb")
                };
                if expression.split('|').all(known) {
                    return None;
                }
                out.push('{');
                out.push_str(expression);
                out.push('}');
            }
        }
        index = end + 1;
    }
    Some(out)
}

impl MetaPreview {
    /// Applies the custom poster template when the screen is enabled. Originals are left untouched
    /// when the pattern has no usable id, exactly like `CustomPosterOverlay`.
    pub fn apply_custom_poster(&mut self, pattern: &str, screen: &str, screens: &[String]) {
        if pattern.trim().is_empty() || !screens.iter().any(|enabled| enabled == screen) {
            return;
        }
        let shape = match self.poster_shape {
            PosterShape::Landscape => "landscape",
            PosterShape::Square => "square",
            _ => "poster",
        };
        if let Some(url) = resolve_poster_template(pattern, &self.id, &self.content_type, shape) {
            self.raw_poster_url = self.poster.take();
            self.poster = Some(url);
        }
        if pattern.contains("{shape}") {
            if let Some(url) = resolve_poster_template(pattern, &self.id, &self.content_type, "landscape") {
                self.raw_landscape_poster_url = self.landscape_poster.take();
                self.landscape_poster = Some(url);
            }
        }
    }

    /// Same identity as Kotlin `MetaPreview.stableKey()`.
    pub fn stable_key(&self) -> String {
        format!("{}:{}", self.content_type, self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PosterShape {
    Poster,
    Square,
    Landscape,
}

impl PosterShape {
    fn parse(value: Option<&str>) -> Self {
        // Kotlin: `when (this?.lowercase())` — no trimming.
        match value.map(str::to_lowercase).as_deref() {
            Some("square") => Self::Square,
            Some("landscape") => Self::Landscape,
            _ => Self::Poster,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPage {
    pub items: Vec<MetaPreview>,
    /// Number of entries in `metas`, before filtering; drives pagination.
    pub raw_item_count: usize,
    pub next_skip: Option<u32>,
}

pub fn parse_catalog_page(payload: &str, requested_skip: Option<u32>, max_items: Option<usize>) -> Result<CatalogPage, CoreError> {
    let root: Value = serde_json::from_str(payload)?;
    let root = root
        .as_object()
        .ok_or_else(|| CoreError::InvalidPayload("catalog response is not a JSON object".into()))?;
    let metas = array(root, "metas");
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for element in metas {
        if max_items.is_some_and(|max| items.len() >= max) {
            break;
        }
        let Some(meta) = element.as_object() else { continue };
        let string = |name: &str| optional_string(meta, name);
        let (Some(id), Some(content_type), Some(name)) = (
            string("id").filter(|v| !v.trim().is_empty()),
            string("type").filter(|v| !v.trim().is_empty()),
            string("name").filter(|v| !v.trim().is_empty()),
        ) else {
            continue;
        };
        let landscape_poster = string("landscapePoster").filter(|v| !v.trim().is_empty());
        let trimmed = |name: &str| string(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let item = MetaPreview {
            id,
            content_type,
            name,
            poster: string("poster"),
            banner: string("banner").or_else(|| string("background")).or_else(|| landscape_poster.clone()),
            logo: string("logo"),
            poster_shape: PosterShape::parse(string("posterShape").as_deref()),
            description: string("description"),
            release_info: string("releaseInfo"),
            raw_release_date: string("released"),
            imdb_rating: string("imdbRating"),
            genres: string_list(meta, "genres"),
            landscape_poster,
            runtime: trimmed("runtime"),
            status: trimmed("status"),
        raw_poster_url: None, raw_landscape_poster_url: None,
    };
        if seen.insert(item.stable_key()) {
            items.push(item);
        }
    }
    let raw_item_count = metas.len();
    let next_skip = (raw_item_count > 0).then(|| requested_skip.unwrap_or(0) + raw_item_count as u32);
    Ok(CatalogPage { items, raw_item_count, next_skip })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_poster_tokens_and_fallback() {
        let pattern = "https://img.test/{id_type}/{typed_id}?t={type}&s={shape}&im={imdb_id|kitsu_id}&x={missing?}&f=%7Bbad%7D";
        assert_eq!(
            resolve_poster_template(pattern, "tt123", "movie", "poster").as_deref(),
            Some("https://img.test/imdb/tt123?t=movie&s=poster&im=tt123&x=&f={bad}")
        );
        let tmdb = resolve_poster_template("{tmdb_id}", "tmdb:55", "series", "poster");
        assert_eq!(tmdb.as_deref(), Some("55"));
        assert_eq!(resolve_poster_template("{imdb_id}", "tmdb:55", "movie", "poster"), None);
        let required_id_type = resolve_poster_template("{mal_id}", "mal:42:3", "movie", "poster");
        assert_eq!(required_id_type.as_deref(), Some("42"));
    }

    #[test]
    fn poster_shape_gating() {
        let mut item = MetaPreview {
            id: "tt123".into(), content_type: "movie".into(), name: "Film".into(),
            poster: Some("orig".into()), banner: None, logo: None, poster_shape: PosterShape::Landscape,
            description: None, release_info: None, raw_release_date: None, imdb_rating: None,
            genres: Vec::new(), landscape_poster: Some("orig-landscape".into()), runtime: None, status: None,
        raw_poster_url: None, raw_landscape_poster_url: None,
    };
        let screens = vec!["home".to_string()];
        item.apply_custom_poster("https://img.test/{id}", "home", &screens);
        assert_eq!(item.poster.as_deref(), Some("https://img.test/tt123"));
        assert_eq!(item.landscape_poster.as_deref(), Some("orig-landscape"));
        item.apply_custom_poster("https://img.test/{id}", "search", &screens);
        assert_eq!(item.poster.as_deref(), Some("https://img.test/tt123"));
    }

    use super::*;

    /// Contract with the UI (`web/src/types.ts`, `MetaPreview`/`CatalogPage`): camelCase keys.
    /// Without it `item.contentType` is undefined in the UI and `meta_details` gets no type.
    #[test]
    fn serializes_in_camel_case_for_the_ui() {
        let page = parse_catalog_page(
            r#"{"metas":[{"id":"tt1","type":"movie","name":"A","releaseInfo":"2020","posterShape":"landscape"}]}"#,
            Some(0),
            None,
        )
        .unwrap();
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(json["nextSkip"], 1);
        assert_eq!(json["rawItemCount"], 1);
        let item = &json["items"][0];
        assert_eq!(item["contentType"], "movie");
        assert_eq!(item["releaseInfo"], "2020");
        assert_eq!(item["posterShape"], "Landscape");
        for snake in ["content_type", "release_info", "poster_shape", "landscape_poster"] {
            assert!(item.get(snake).is_none(), "{snake} must not be serialized");
        }
    }

    #[test]
    fn skips_invalid_items_dedupes_and_computes_next_skip() {
        let payload = r#"{"metas": [
            {"id": "tt1", "type": "movie", "name": "One", "genres": ["Drama", ""], "posterShape": "landscape", "background": "b.jpg"},
            {"id": "tt1", "type": "movie", "name": "One again"},
            {"id": "", "type": "movie", "name": "No id"},
            {"id": "tt2", "type": "movie"},
            "not an object",
            {"id": "tt3", "type": "series", "name": "Three", "runtime": "  45 min ", "status": "  "}
        ]}"#;
        let page = parse_catalog_page(payload, Some(100), None).unwrap();
        assert_eq!(page.raw_item_count, 6);
        assert_eq!(page.next_skip, Some(106));
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].genres, vec!["Drama"]);
        assert_eq!(page.items[0].poster_shape, PosterShape::Landscape);
        assert_eq!(page.items[0].banner.as_deref(), Some("b.jpg"));
        assert_eq!(page.items[1].runtime.as_deref(), Some("45 min"));
        assert_eq!(page.items[1].status, None);
    }

    #[test]
    fn empty_page_has_no_next_skip_and_max_items_is_respected() {
        assert_eq!(parse_catalog_page(r#"{"metas": []}"#, None, None).unwrap().next_skip, None);
        assert_eq!(parse_catalog_page(r#"{}"#, None, None).unwrap().raw_item_count, 0);
        let payload = r#"{"metas":[{"id":"a","type":"tv","name":"A"},{"id":"b","type":"tv","name":"B"}]}"#;
        assert_eq!(parse_catalog_page(payload, None, Some(1)).unwrap().items.len(), 1);
    }
}
