//! Addon meta responses (`/meta/{type}/{id}.json`): details, cast and videos (episodes).
//!
//! Port of the core of `features/details/MetaDetailsParser.kt`: `meta` object required (`id`,
//! `type`, `name`), `background` falls back to `landscapePoster`, people come from the top-level
//! list (strings or `{name, character, photo}` objects) plus `links` of the matching category,
//! videos need `id` and `title`/`name`, `description` is the overview fallback, `available`
//! defaults to true; `trailers` as `MetaDetailsParser.trailers()`. Not ported yet: `app_extras`
//! (certifications, season posters),
//! embedded video streams, TMDB id lookup.

use crate::manifest::{array, optional_string, primitive_content, string_list};
use crate::CoreError;
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPerson {
    pub name: String,
    pub role: Option<String>,
    pub photo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaVideo {
    pub id: String,
    pub title: String,
    pub released: Option<String>,
    pub available: bool,
    pub thumbnail: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub overview: Option<String>,
}

/// `MetaTrailer`: `key` is a YouTube video id (or URL) when `site` is YouTube.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaTrailer {
    pub id: String,
    pub key: String,
    pub name: String,
    pub site: String,
    pub size: Option<i64>,
    #[serde(rename = "type")]
    pub kind: String,
    pub official: bool,
    pub published_at: Option<String>,
    pub season_number: Option<i64>,
}

impl MetaTrailer {
    /// `heroTrailerPriority`.
    fn priority(&self) -> i32 {
        let series = self.season_number.is_some();
        let trailer = self.kind.eq_ignore_ascii_case("Trailer");
        match (series, trailer, self.official) {
            (false, true, true) => 70,
            (false, true, false) => 60,
            (false, false, true) => 50,
            (false, false, false) => 40,
            (true, true, true) => 30,
            (true, true, false) => 20,
            (true, false, true) => 10,
            _ => 0,
        }
    }

    /// `youtubePlaybackUrl`.
    pub fn youtube_url(&self) -> String {
        if self.key.starts_with("http://") || self.key.starts_with("https://") {
            self.key.clone()
        } else {
            format!("https://www.youtube.com/watch?v={}", self.key)
        }
    }
}

/// `selectHeroTrailer`: the YouTube trailer the details page offers.
pub fn select_trailer(trailers: &[MetaTrailer]) -> Option<&MetaTrailer> {
    let mut seen = std::collections::HashSet::new();
    trailers
        .iter()
        .filter(|t| !t.key.is_empty() && t.site.eq_ignore_ascii_case("YouTube"))
        .filter(|t| seen.insert(t.key.clone()))
        .max_by(|a, b| {
            (a.priority(), a.published_at.as_deref().unwrap_or(""), a.size.unwrap_or(0), &a.name)
                .cmp(&(b.priority(), b.published_at.as_deref().unwrap_or(""), b.size.unwrap_or(0), &b.name))
        })
}

fn trailers(meta: &Map<String, Value>) -> Vec<MetaTrailer> {
    let non_blank = |v: Option<String>| v.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
    array(meta, "trailers")
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|t| {
            let s = |n: &str| non_blank(optional_string(t, n));
            let key = s("key").or_else(|| s("source")).or_else(|| s("ytId")).or_else(|| s("ytid"))?;
            Some(MetaTrailer {
                id: s("id").unwrap_or_else(|| key.clone()),
                name: s("name").unwrap_or_else(|| "Trailer".into()),
                site: s("site").unwrap_or_else(|| "YouTube".into()),
                size: int_like(t, "size"),
                kind: s("type").unwrap_or_else(|| "Trailer".into()),
                official: t.get("official").and_then(Value::as_bool) == Some(true),
                published_at: s("published_at").or_else(|| s("publishedAt")),
                season_number: int_like(t, "seasonNumber").or_else(|| int_like(t, "season_number")),
                key,
            })
        })
        .collect()
}

impl MetaDetails {
    /// Custom poster source of the original (`MetaDetails.withCustomPosterUrls`): poster and
    /// background are replaced, the logo keeps the title's own artwork.
    pub fn apply_custom_poster(&mut self, pattern: &str, screen: &str, screens: &[String]) {
        if pattern.trim().is_empty() || !screens.iter().any(|enabled| enabled == screen) {
            return;
        }
        if let Some(url) = crate::catalog::resolve_poster_template(pattern, &self.id, &self.content_type, "poster") {
            self.poster = Some(url);
        }
        if pattern.contains("{shape}") {
            if let Some(url) = crate::catalog::resolve_poster_template(pattern, &self.id, &self.content_type, "landscape") {
                self.background = Some(url);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaDetails {
    pub id: String,
    pub content_type: String,
    pub name: String,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub logo: Option<String>,
    pub description: Option<String>,
    pub release_info: Option<String>,
    pub imdb_rating: Option<String>,
    pub age_rating: Option<String>,
    pub status: Option<String>,
    pub runtime: Option<String>,
    pub genres: Vec<String>,
    pub director: Vec<String>,
    pub writer: Vec<String>,
    pub cast: Vec<MetaPerson>,
    pub country: Option<String>,
    pub awards: Option<String>,
    pub language: Option<String>,
    pub website: Option<String>,
    /// `behaviorHints.defaultVideoId`: the video to play for this title (movies with a video id).
    pub default_video_id: Option<String>,
    pub videos: Vec<MetaVideo>,
    pub trailers: Vec<MetaTrailer>,
}

impl MetaDetails {
    /// Seasons in ascending order (season 0 = specials last, as most Stremio clients show it).
    pub fn seasons(&self) -> Vec<i64> {
        let mut seasons: Vec<i64> = self.videos.iter().filter_map(|v| v.season).collect();
        seasons.sort_by_key(|s| if *s == 0 { i64::MAX } else { *s });
        seasons.dedup();
        seasons
    }
}

fn int_like(obj: &Map<String, Value>, name: &str) -> Option<i64> {
    match obj.get(name)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn people(obj: &Map<String, Value>, name: &str) -> Vec<MetaPerson> {
    array(obj, name)
        .iter()
        .filter_map(|element| match element {
            Value::Object(p) => {
                let name = optional_string(p, "name")?.trim().to_string();
                (!name.is_empty()).then(|| MetaPerson {
                    name,
                    role: optional_string(p, "character").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
                    photo: optional_string(p, "photo").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
                })
            }
            other => primitive_content(other)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .map(|name| MetaPerson { name, role: None, photo: None }),
        })
        .collect()
}

/// `writer: ["A"]` or `writer: "A, B"` (Kotlin `stringListOrCsv`).
fn names_or_csv(obj: &Map<String, Value>, name: &str) -> Vec<String> {
    match obj.get(name) {
        Some(Value::String(v)) => v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => people(obj, name).into_iter().map(|p| p.name).collect(),
    }
}

fn link_names(obj: &Map<String, Value>, categories: &[&str]) -> Vec<String> {
    array(obj, "links")
        .iter()
        .filter_map(Value::as_object)
        .filter(|l| optional_string(l, "category").is_some_and(|c| categories.iter().any(|w| c.eq_ignore_ascii_case(w))))
        .filter_map(|l| optional_string(l, "name"))
        .collect()
}

fn dedupe(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && seen.insert(n.to_lowercase()))
        .collect()
}

pub fn parse_meta(payload: &str) -> Result<MetaDetails, CoreError> {
    let root: Value = serde_json::from_str(payload)?;
    let meta = root
        .as_object()
        .and_then(|r| r.get("meta"))
        .and_then(Value::as_object)
        .ok_or_else(|| CoreError::InvalidPayload("response has no meta object".into()))?;
    let s = |n: &str| optional_string(meta, n);
    let required = |n: &str| s(n).filter(|v| !v.trim().is_empty()).ok_or_else(|| CoreError::MissingField(n.to_string()));

    let extras = meta.get("app_extras").and_then(Value::as_object);
    let empty = Map::new();
    let extras_or_empty = extras.unwrap_or(&empty);
    // Order as `mergePeople(appExtraCast, topLevelCast, linkedCast)`.
    let mut cast = people(extras_or_empty, "cast");
    for person in people(meta, "cast") {
        if !cast.iter().any(|p| p.name.eq_ignore_ascii_case(&person.name)) {
            cast.push(person);
        }
    }
    for name in link_names(meta, &["cast", "actors", "actor"]) {
        if !cast.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
            cast.push(MetaPerson { name, role: None, photo: None });
        }
    }
    let director = dedupe(
        names_or_csv(meta, "director")
            .into_iter()
            .chain(people(extras_or_empty, "directors").into_iter().map(|p| p.name))
            .chain(link_names(meta, &["director", "directors"])),
    );
    let writer = dedupe(
        names_or_csv(meta, "writer")
            .into_iter()
            .chain(people(extras_or_empty, "writers").into_iter().map(|p| p.name))
            .chain(link_names(meta, &["writer", "writers", "screenplay"])),
    );
    let non_blank = |v: Option<String>| v.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
    let age_rating = non_blank(s("ageRating"))
        .or_else(|| extras.and_then(|e| non_blank(optional_string(e, "certificationLocal"))))
        .or_else(|| extras.and_then(|e| non_blank(optional_string(e, "certification"))));
    let default_video_id = meta
        .get("behaviorHints")
        .and_then(Value::as_object)
        .and_then(|h| non_blank(optional_string(h, "defaultVideoId")));
    let videos = array(meta, "videos")
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|v| {
            let vs = |n: &str| optional_string(v, n);
            Some(MetaVideo {
                id: vs("id")?,
                title: vs("title").or_else(|| vs("name"))?,
                released: vs("released"),
                available: v.get("available").and_then(Value::as_bool).unwrap_or(true),
                thumbnail: vs("thumbnail"),
                season: int_like(v, "season"),
                episode: int_like(v, "episode"),
                overview: vs("overview").or_else(|| vs("description")),
            })
        })
        .collect();

    Ok(MetaDetails {
        id: required("id")?,
        content_type: required("type")?,
        name: required("name")?,
        poster: s("poster"),
        background: s("background").or_else(|| s("landscapePoster").filter(|v| !v.trim().is_empty())),
        logo: s("logo"),
        description: s("description"),
        release_info: s("releaseInfo"),
        imdb_rating: s("imdbRating"),
        age_rating,
        status: non_blank(s("status")),
        runtime: s("runtime"),
        genres: string_list(meta, "genres"),
        director,
        writer,
        cast,
        country: non_blank(s("country")),
        awards: non_blank(s("awards")),
        language: non_blank(s("language")),
        website: non_blank(s("website")),
        default_video_id,
        videos,
        trailers: trailers(meta),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailers_like_the_original() {
        let m = parse_meta(r#"{"meta":{"id":"tt1","type":"movie","name":"X","trailers":[
            {"source":"aaaaaaaaaaa","type":"Teaser"},
            {"key":"bbbbbbbbbbb","type":"Trailer","official":true,"site":"YouTube"},
            {"key":"ccccccccccc","type":"Trailer","site":"Vimeo"},
            {"ytId":"ddddddddddd","type":"Trailer","seasonNumber":2,"official":true},
            {"type":"Trailer"}
        ]}}"#).unwrap();
        assert_eq!(m.trailers.len(), 4);
        let t = select_trailer(&m.trailers).unwrap();
        assert_eq!(t.key, "bbbbbbbbbbb");
        assert_eq!(t.youtube_url(), "https://www.youtube.com/watch?v=bbbbbbbbbbb");
        assert!(select_trailer(&m.trailers[2..3]).is_none(), "not YouTube");
    }

    #[test]
    fn parses_series_with_people_links_and_videos() {
        let payload = r#"{"meta": {
            "id": "tt1", "type": "series", "name": "Show", "landscapePoster": "bg.jpg",
            "genres": ["Drama", ""], "cast": ["Ana", {"name": "Bob", "character": "Chef", "photo": " p.jpg "}],
            "director": ["Dee"],
            "links": [{"name": "Carl", "category": "Cast", "url": "x"}, {"name": "Dee", "category": "Directors", "url": "y"},
                      {"name": "Eve", "category": "Writers", "url": "z"}],
            "videos": [
                {"id": "tt1:1:1", "title": "Pilot", "season": 1, "episode": 1, "description": "first"},
                {"id": "tt1:0:1", "name": "Special", "season": "0", "episode": 1, "available": false},
                {"id": "tt1:2:1", "title": "Back", "season": 2, "episode": 1},
                {"title": "no id"}
            ]}}"#;
        let m = parse_meta(payload).unwrap();
        assert_eq!(m.background.as_deref(), Some("bg.jpg"));
        assert_eq!(m.genres, vec!["Drama"]);
        assert_eq!(m.cast.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Ana", "Bob", "Carl"]);
        assert_eq!(m.cast[1].photo.as_deref(), Some("p.jpg"));
        assert_eq!(m.director, vec!["Dee"]);
        assert_eq!(m.videos.len(), 3);
        assert_eq!(m.videos[0].overview.as_deref(), Some("first"));
        assert!(!m.videos[1].available);
        assert_eq!(m.seasons(), vec![1, 2, 0]);
        assert_eq!(m.writer, vec!["Eve"]);
    }

    #[test]
    fn parses_movie_extras_like_the_original() {
        let payload = r#"{"meta": {
            "id": "tt9", "type": "movie", "name": "Film", "releaseInfo": "2024", "runtime": "1h 50min",
            "writer": "Wa, Wb", "country": "BR", "awards": " ", "language": "pt",
            "app_extras": {"certification": "16", "cast": [{"name": "Zed", "character": "Lead"}], "directors": [{"name": "Dee"}]},
            "behaviorHints": {"defaultVideoId": "tt9"}}}"#;
        let m = parse_meta(payload).unwrap();
        assert_eq!(m.content_type, "movie");
        assert_eq!(m.writer, vec!["Wa", "Wb"]);
        assert_eq!(m.age_rating.as_deref(), Some("16"));
        assert_eq!(m.cast[0].role.as_deref(), Some("Lead"));
        assert_eq!(m.director, vec!["Dee"]);
        assert_eq!(m.awards, None);
        assert_eq!(m.default_video_id.as_deref(), Some("tt9"));
        assert!(m.seasons().is_empty());
    }

    /// Contract with the UI (`web/src/types.ts`): camelCase keys.
    #[test]
    fn serializes_in_camel_case_for_the_ui() {
        let m = parse_meta(r#"{"meta":{"id":"a","type":"series","name":"S","videos":[{"id":"a:1:1","title":"E","season":1,"episode":1}]}}"#).unwrap();
        let json = serde_json::to_value(&m).unwrap();
        for key in ["contentType", "releaseInfo", "imdbRating", "ageRating", "defaultVideoId", "videos"] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
        assert!(json.get("content_type").is_none());
    }

    #[test]
    fn missing_meta_or_required_fields_fail() {
        assert!(parse_meta("{}").is_err());
        assert!(matches!(parse_meta(r#"{"meta":{"id":"x","type":"movie"}}"#), Err(CoreError::MissingField(f)) if f == "name"));
    }
}
