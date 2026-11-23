//! Stremio addon manifest model and parser.
//!
//! Port of `features/addons/AddonManifestParser.kt` and `AddonModels.kt`. The parsing rules are
//! the same: resources may be plain strings or objects, objects inherit the manifest's `types`
//! and `idPrefixes` when theirs are empty, blank names are dropped, and relative logos resolve
//! against the manifest URL.

use crate::error::CoreError;
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddonManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub logo_url: Option<String>,
    pub resources: Vec<AddonResource>,
    pub types: Vec<String>,
    pub id_prefixes: Vec<String>,
    pub catalogs: Vec<AddonCatalog>,
    pub behavior_hints: AddonBehaviorHints,
    /// URL the manifest was loaded from; every resource URL is built from it.
    pub transport_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddonResource {
    pub name: String,
    pub types: Vec<String>,
    pub id_prefixes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddonCatalog {
    pub content_type: String,
    pub id: String,
    pub name: String,
    pub extra: Vec<AddonExtraProperty>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AddonExtraProperty {
    pub name: String,
    pub is_required: bool,
    pub options: Vec<String>,
    pub options_limit: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AddonBehaviorHints {
    pub configurable: bool,
    pub configuration_required: bool,
    pub adult: bool,
    pub p2p: bool,
}

impl AddonManifest {
    /// Same rule as `AddonManifest.supportsResource` in Kotlin.
    pub fn supports_resource(&self, name: &str, content_type: &str, id: &str) -> bool {
        self.resources.iter().any(|r| {
            r.name == name
                && r.types.iter().any(|t| t == content_type)
                && (r.id_prefixes.is_empty() || r.id_prefixes.iter().any(|p| id.starts_with(p.as_str())))
        })
    }

    /// Player/library resources mark "tracking" addons, as in Kotlin `isTrackingAddon`.
    pub fn is_tracking_addon(&self) -> bool {
        self.resources.iter().any(|r| r.name == "player" || r.name == "library")
    }
}

impl AddonCatalog {
    pub fn supports_pagination(&self) -> bool {
        self.extra.iter().any(|e| e.name.eq_ignore_ascii_case("skip"))
    }

    /// Search catalogs: have a `search` extra and no other required extra.
    pub fn supports_search(&self) -> bool {
        self.extra.iter().any(|e| e.name == "search")
            && !self.extra.iter().any(|e| e.is_required && e.name != "search")
    }

    /// Discover catalogs (`AddonCatalog.supportsDiscover`): a catalog can be listed without asking
    /// for anything the picker cannot provide. A required `search` excludes it, any other required
    /// extra except `genre`/`skip` excludes it, and a required `genre` without options excludes it
    /// (there would be nothing to choose).
    pub fn supports_discover(&self) -> bool {
        if self.extra.iter().any(|e| e.name == "search" && e.is_required) {
            return false;
        }
        !self.extra.iter().any(|e| match e.name.as_str() {
            "genre" => e.is_required && e.options.is_empty(),
            "skip" | "search" => false,
            _ => e.is_required,
        })
    }

    /// The `genre` extra, whose options feed the Discover genre picker.
    pub fn genre_extra(&self) -> Option<&AddonExtraProperty> {
        self.extra.iter().find(|e| e.name == "genre")
    }
}

pub fn parse_manifest(manifest_url: &str, payload: &str) -> Result<AddonManifest, CoreError> {
    let root: Value = serde_json::from_str(payload)?;
    let root = root
        .as_object()
        .ok_or_else(|| CoreError::InvalidPayload("manifest is not a JSON object".into()))?;
    let default_types = string_list(root, "types");
    let default_prefixes = string_list(root, "idPrefixes");

    let resources = array(root, "resources")
        .iter()
        .map(|resource| -> Result<AddonResource, CoreError> {
            match resource {
                Value::Object(obj) => {
                    let types = string_list(obj, "types");
                    let prefixes = string_list(obj, "idPrefixes");
                    Ok(AddonResource {
                        name: required_string(obj, "name")?,
                        types: if types.is_empty() { default_types.clone() } else { types },
                        id_prefixes: if prefixes.is_empty() { default_prefixes.clone() } else { prefixes },
                    })
                }
                other => Ok(AddonResource {
                    name: primitive_content(other).unwrap_or_default(),
                    types: default_types.clone(),
                    id_prefixes: default_prefixes.clone(),
                }),
            }
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|r| !r.name.trim().is_empty())
        .collect();

    let catalogs = array(root, "catalogs")
        .iter()
        .map(|element| {
            let catalog = element
                .as_object()
                .ok_or_else(|| CoreError::InvalidPayload("catalog is not an object".into()))?;
            let id = required_string(catalog, "id")?;
            let name = optional_string(catalog, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| id.clone());
            let extra = array(catalog, "extra")
                .iter()
                .filter_map(|extra| {
                    let obj = extra.as_object()?;
                    let name = optional_string(obj, "name").filter(|n| !n.trim().is_empty())?;
                    Some(AddonExtraProperty {
                        name,
                        is_required: obj.get("isRequired").and_then(Value::as_bool) == Some(true),
                        options: string_list(obj, "options"),
                        options_limit: obj.get("optionsLimit").and_then(primitive_content).and_then(|v| v.parse().ok()),
                    })
                })
                .collect();
            Ok(AddonCatalog { content_type: required_string(catalog, "type")?, id, name, extra })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;

    let behavior_hints = root
        .get("behaviorHints")
        .and_then(Value::as_object)
        .map(|hints| {
            let flag = |name: &str| hints.get(name).and_then(Value::as_bool) == Some(true);
            AddonBehaviorHints {
                configurable: flag("configurable"),
                configuration_required: flag("configurationRequired"),
                adult: flag("adult"),
                p2p: flag("p2p"),
            }
        })
        .unwrap_or_default();

    Ok(AddonManifest {
        id: required_string(root, "id")?,
        name: required_string(root, "name")?,
        description: optional_string(root, "description").unwrap_or_default(),
        version: required_string(root, "version")?,
        logo_url: optional_string(root, "logo").map(|logo| resolve_against_manifest(&logo, manifest_url)),
        resources,
        types: default_types,
        id_prefixes: default_prefixes,
        catalogs,
        behavior_hints,
        transport_url: manifest_url.to_string(),
    })
}

fn resolve_against_manifest(value: &str, manifest_url: &str) -> String {
    if value.starts_with("http://") || value.starts_with("https://") || value.starts_with("data:") {
        return value.to_string();
    }
    if let Some(rest) = value.strip_prefix("//") {
        return format!("https://{rest}");
    }
    let without_query = manifest_url.split_once('?').map_or(manifest_url, |(b, _)| b);
    let manifest_base = without_query.rsplit_once('/').map_or("", |(b, _)| b);
    if value.starts_with('/') {
        match manifest_base.split_once("://") {
            Some((scheme, after)) => {
                let host = after.split_once('/').map_or(after, |(h, _)| h);
                format!("{scheme}://{host}{value}")
            }
            None => format!("{manifest_base}{value}"),
        }
    } else {
        format!("{manifest_base}/{value}")
    }
}

// --- JSON helpers mirroring the Kotlin extension functions ---

pub(crate) fn primitive_content(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub(crate) fn optional_string(obj: &Map<String, Value>, name: &str) -> Option<String> {
    obj.get(name).and_then(primitive_content)
}

fn required_string(obj: &Map<String, Value>, name: &str) -> Result<String, CoreError> {
    optional_string(obj, name)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| CoreError::MissingField(name.to_string()))
}

pub(crate) fn array<'a>(obj: &'a Map<String, Value>, name: &str) -> &'a [Value] {
    obj.get(name).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

pub(crate) fn string_list(obj: &Map<String, Value>, name: &str) -> Vec<String> {
    array(obj, name)
        .iter()
        .filter_map(primitive_content)
        .filter(|v| !v.trim().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "id": "org.example.tv", "name": "Example TV", "version": "1.2.0",
        "logo": "/static/logo.png",
        "types": ["tv", "movie"], "idPrefixes": ["ex:"],
        "resources": ["catalog", {"name": "stream", "types": ["tv"]}, {"name": "meta", "idPrefixes": ["tt"]}, ""],
        "catalogs": [
            {"type": "tv", "id": "all", "name": "All channels",
             "extra": [{"name": "genre", "options": ["News", "Sports"]}, {"name": "skip"}, {"name": ""}]},
            {"type": "movie", "id": "search", "extra": [{"name": "search", "isRequired": true}]}
        ],
        "behaviorHints": {"configurable": true, "p2p": false},
        "unknownField": 42
    }"#;

    #[test]
    fn parses_resources_with_inherited_types_and_prefixes() {
        let m = parse_manifest("https://addon.example/cfg=1/manifest.json?x=1", MANIFEST).unwrap();
        assert_eq!(m.resources.len(), 3); // blank resource dropped
        assert_eq!(m.resources[0].types, vec!["tv", "movie"]);
        assert_eq!(m.resources[1].types, vec!["tv"]);
        assert_eq!(m.resources[1].id_prefixes, vec!["ex:"]);
        assert_eq!(m.resources[2].id_prefixes, vec!["tt"]);
        assert!(m.supports_resource("stream", "tv", "ex:globo"));
        assert!(!m.supports_resource("stream", "movie", "ex:1"));
        assert!(!m.supports_resource("stream", "tv", "other:1"));
        assert_eq!(m.logo_url.as_deref(), Some("https://addon.example/static/logo.png"));
        assert!(m.behavior_hints.configurable);
    }

    #[test]
    fn parses_catalogs_and_extras() {
        let m = parse_manifest("https://addon.example/manifest.json", MANIFEST).unwrap();
        assert_eq!(m.catalogs.len(), 2);
        let all = &m.catalogs[0];
        assert_eq!(all.extra.len(), 2); // blank extra dropped
        assert!(all.supports_pagination());
        assert!(!all.supports_search());
        assert_eq!(all.extra[0].options, vec!["News", "Sports"]);
        assert_eq!(m.catalogs[1].name, "search"); // name falls back to id
        assert!(m.catalogs[1].supports_search());
    }

    #[test]
    fn discover_catalogs_need_no_extra_the_picker_cannot_fill() {
        const DISCOVER: &str = r#"{
            "id": "a", "name": "A", "version": "1", "resources": ["catalog"], "types": ["movie"],
            "catalogs": [
                {"type": "movie", "id": "top", "name": "Top"},
                {"type": "movie", "id": "search", "name": "Search",
                 "extra": [{"name": "search", "isRequired": true}]},
                {"type": "movie", "id": "optionalSearch", "name": "Optional",
                 "extra": [{"name": "search"}]},
                {"type": "movie", "id": "genres", "name": "Genres",
                 "extra": [{"name": "genre", "isRequired": true, "options": ["Action", "Drama"]}]},
                {"type": "movie", "id": "emptyGenre", "name": "Empty genre",
                 "extra": [{"name": "genre", "isRequired": true}]},
                {"type": "movie", "id": "year", "name": "Year",
                 "extra": [{"name": "year", "isRequired": true}]},
                {"type": "movie", "id": "paged", "name": "Paged", "extra": [{"name": "skip"}]}
            ]
        }"#;
        let m = parse_manifest("https://a.test/manifest.json", DISCOVER).unwrap();
        let eligible: Vec<&str> = m.catalogs.iter().filter(|c| c.supports_discover()).map(|c| c.id.as_str()).collect();
        assert_eq!(eligible, vec!["top", "optionalSearch", "genres", "paged"]);
        let genre = m.catalogs[3].genre_extra().unwrap();
        assert!(genre.is_required);
        assert_eq!(genre.options, vec!["Action", "Drama"]);
        assert!(m.catalogs[0].genre_extra().is_none());
    }

    #[test]
    fn missing_required_fields_are_errors() {
        let err = parse_manifest("https://a/manifest.json", r#"{"id":"x","name":"y"}"#).unwrap_err();
        assert!(matches!(err, CoreError::MissingField(f) if f == "version"));
    }

    #[test]
    fn relative_logo_resolution_matches_kotlin() {
        assert_eq!(resolve_against_manifest("logo.png", "https://h.test/a/manifest.json"), "https://h.test/a/logo.png");
        assert_eq!(resolve_against_manifest("//cdn.test/l.png", "https://h.test/manifest.json"), "https://cdn.test/l.png");
        assert_eq!(resolve_against_manifest("data:image/png;base64,AA", "https://h.test/manifest.json"), "data:image/png;base64,AA");
    }
}
