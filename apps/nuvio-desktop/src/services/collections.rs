//! Home collections (`CollectionModels.kt`, `HomeCollectionRowSection.kt`, `FolderDetailRepository.kt`).
//!
//! A collection is a Home row of folders; each folder has a cover (image, animated GIF/WebP or a
//! short video, or an emoji) and a list of sources. Sources of type `addon` are catalogs of the
//! profile's addons (resolved as `findCollectionCatalog`: the declared addon first, then any
//! enabled addon with the same catalog); TMDB sources are listed when TMDB is set up
//! (`services/tmdb.rs`); Trakt sources are skipped. The collections come from the account (`sync_pull_collections`, read only) or,
//! before the first sync, from the original app's own store (read only).

use super::addons::media_type_label;
use nuvio_core::manifest::{AddonCatalog, AddonManifest};
use nuvio_store::nextgen::AppStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------- stored format (lenient)

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct StoredCollection {
    id: String,
    title: String,
    backdrop_image_url: Option<String>,
    pin_to_top: bool,
    view_mode: String,
    show_all_tab: bool,
    folders: Vec<StoredFolder>,
}

impl Default for StoredCollection {
    fn default() -> Self {
        Self { id: String::new(), title: String::new(), backdrop_image_url: None, pin_to_top: false, view_mode: "TABBED_GRID".into(), show_all_tab: true, folders: Vec::new() }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct StoredFolder {
    id: String,
    title: String,
    cover_image_url: Option<String>,
    focus_gif_url: Option<String>,
    focus_gif_enabled: bool,
    cover_emoji: Option<String>,
    tile_shape: String,
    hide_title: bool,
    sources: Vec<StoredSource>,
    catalog_sources: Vec<StoredSource>,
    hero_backdrop_url: Option<String>,
    title_logo_url: Option<String>,
}

impl Default for StoredFolder {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            cover_image_url: None,
            focus_gif_url: None,
            focus_gif_enabled: true,
            cover_emoji: None,
            tile_shape: "poster".into(),
            hide_title: false,
            sources: Vec::new(),
            catalog_sources: Vec::new(),
            hero_backdrop_url: None,
            title_logo_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
struct StoredSource {
    provider: Option<String>,
    addon_id: Option<String>,
    #[serde(rename = "type")]
    content_type: Option<String>,
    catalog_id: Option<String>,
    genre: Option<String>,
    // TMDB sources (`TmdbCollectionSourceResolver`)
    title: Option<String>,
    tmdb_source_type: Option<String>,
    tmdb_id: Option<Value>,
    media_type: Option<String>,
    sort_by: Option<String>,
    filters: Option<Value>,
}

impl StoredSource {
    /// The TMDB source this is, if any.
    fn tmdb(&self) -> Option<super::tmdb::Source> {
        super::tmdb::Source::from_json(&serde_json::json!({
            "provider": self.provider, "tmdbSourceType": self.tmdb_source_type, "tmdbId": self.tmdb_id,
            "mediaType": self.media_type, "sortBy": self.sort_by, "filters": self.filters,
        }))
    }
}

/// Catalog key of a TMDB folder source: positions of the collection, folder and source in the
/// stored list (`catalog_page` reads it back with `tmdb_source`).
pub const TMDB_KEY: &str = "@tmdb/";

/// The TMDB source a `@tmdb/<collection>/<folder>/<source>` key points to.
pub fn tmdb_source(value: &Value, key: &str) -> Option<super::tmdb::Source> {
    stored_source(value, key.strip_prefix(TMDB_KEY)?)?.tmdb()
}

fn stored_source(value: &Value, at: &str) -> Option<StoredSource> {
    let mut parts = at.split('/').map(|p| p.parse::<usize>().ok());
    let (c, f, i) = (parts.next()??, parts.next()??, parts.next()??);
    let collection = parse(value).into_iter().nth(c)?;
    let folder = collection.folders.get(f)?;
    let sources = if folder.sources.is_empty() { &folder.catalog_sources } else { &folder.sources };
    sources.get(i).cloned()
}

// ---------------------------------------------------------------- what the interface gets

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionView {
    pub id: String,
    pub title: String,
    pub backdrop: Option<String>,
    pub pin_to_top: bool,
    /// `TABBED_GRID`, `ROWS` or `FOLLOW_LAYOUT`.
    pub view_mode: String,
    pub show_all_tab: bool,
    pub folders: Vec<FolderView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    pub id: String,
    pub title: String,
    /// What the card shows: the GIF (when enabled) or the cover. GIF/WebP play by themselves; a
    /// video URL (`.mp4`, `.webm`) is played muted in a loop.
    pub image: Option<String>,
    /// The still cover, shown while the animated one loads or if it fails.
    pub still_image: Option<String>,
    pub emoji: Option<String>,
    /// `poster`, `landscape` or `square`.
    pub shape: String,
    pub hide_title: bool,
    pub backdrop: Option<String>,
    pub logo: Option<String>,
    pub sources: Vec<FolderSource>,
    /// TMDB/Trakt sources, or catalogs of addons that are not installed (not shown).
    pub skipped_sources: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FolderSource {
    /// Catalog key `<manifestId>:<type>:<catalogId>` (as the Home rows).
    pub key: String,
    pub genre: Option<String>,
    pub label: String,
    pub addon_name: String,
    pub content_type: String,
    pub supports_pagination: bool,
}

fn clean(value: &Option<String>) -> Option<String> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

fn genre_of(value: &Option<String>) -> Option<String> {
    clean(value).filter(|g| !g.eq_ignore_ascii_case("none"))
}

/// Collections of the profile: the copy received from the account, else the original app's store.
pub fn stored(app: &AppStore, profile: i64) -> Value {
    if let Ok(Some(value)) = app.collections(profile) {
        return value;
    }
    super::addons::legacy_store().ok().and_then(|l| l.collections(profile).ok().flatten()).unwrap_or(Value::Array(Vec::new()))
}

/// Parses the stored array; a collection that does not parse, or has no id, is skipped (as
/// `normalizeCollections`), and repeated ids keep the first one.
fn parse(value: &Value) -> Vec<StoredCollection> {
    let mut seen = std::collections::HashSet::new();
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| serde_json::from_value::<StoredCollection>(v.clone()).ok())
                .filter(|c| !c.id.trim().is_empty() && seen.insert(c.id.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn find_catalog<'a>(catalogs: &'a [AddonCatalog], content_type: &str, catalog_id: &str) -> Option<&'a AddonCatalog> {
    let short = catalog_id.split(',').next().unwrap_or(catalog_id);
    catalogs
        .iter()
        .find(|c| c.id == catalog_id && c.content_type == content_type)
        .or_else(|| catalogs.iter().find(|c| c.id == short && c.content_type == content_type))
}

/// `findCollectionCatalog`: the declared addon, then any enabled addon with the same catalog.
fn resolve<'a>(manifests: &'a [AddonManifest], addon_id: &str, content_type: &str, catalog_id: &str) -> Option<(&'a AddonManifest, &'a AddonCatalog)> {
    manifests
        .iter()
        .find(|m| m.id == addon_id)
        .and_then(|m| find_catalog(&m.catalogs, content_type, catalog_id).map(|c| (m, c)))
        .or_else(|| manifests.iter().find_map(|m| m.catalogs.iter().find(|c| c.id == catalog_id && c.content_type == content_type).map(|c| (m, c))))
}

fn folder_view(folder: &StoredFolder, manifests: &[AddonManifest], show_catalog_type: bool, at: Option<(usize, usize)>, ready: Ready) -> FolderView {
    let sources = if folder.sources.is_empty() { &folder.catalog_sources } else { &folder.sources };
    let mut resolved = Vec::new();
    let mut skipped = 0;
    for (index, source) in sources.iter().enumerate() {
        let provider = clean(&source.provider).unwrap_or_else(|| "addon".into());
        // TMDB sources, when TMDB is set up (`at` is given).
        if provider.eq_ignore_ascii_case("tmdb") {
            match (at.filter(|_| ready.tmdb), source.tmdb()) {
                (Some((c, f)), Some(tmdb)) => resolved.push(FolderSource {
                    key: format!("{TMDB_KEY}{c}/{f}/{index}"),
                    genre: None,
                    label: clean(&source.title).unwrap_or_else(|| tmdb.default_label()),
                    addon_name: "TMDB".into(),
                    content_type: tmdb.media_type.content_type().into(),
                    supports_pagination: tmdb.paged(),
                }),
                _ => skipped += 1,
            }
            continue;
        }
        let (Some(addon_id), Some(content_type), Some(catalog_id)) = (clean(&source.addon_id), clean(&source.content_type), clean(&source.catalog_id)) else {
            skipped += 1;
            continue;
        };
        if provider.eq_ignore_ascii_case("tmdb") || provider.eq_ignore_ascii_case("trakt") {
            skipped += 1;
            continue;
        }
        let Some((manifest, catalog)) = resolve(manifests, &addon_id, &content_type, &catalog_id) else {
            skipped += 1;
            continue;
        };
        let genre = genre_of(&source.genre);
        // `folderCatalogTabLabel`: "<catalog> · <genre>", then " - <type>" when the type is shown.
        let name = match &genre {
            Some(g) => format!("{} · {g}", catalog.name),
            None => catalog.name.clone(),
        };
        let label = if show_catalog_type { format!("{name} - {}", media_type_label(&catalog.content_type)) } else { name };
        resolved.push(FolderSource {
            key: format!("{}:{}:{}", manifest.id, catalog.content_type, catalog.id),
            genre,
            label,
            addon_name: manifest.name.clone(),
            content_type: catalog.content_type.clone(),
            supports_pagination: catalog.supports_pagination(),
        });
    }
    let cover = clean(&folder.cover_image_url);
    let gif = clean(&folder.focus_gif_url).filter(|_| folder.focus_gif_enabled);
    let shape = match folder.tile_shape.to_lowercase().as_str() {
        "landscape" | "wide" => "landscape",
        "square" => "square",
        _ => "poster",
    };
    FolderView {
        id: folder.id.clone(),
        title: folder.title.clone(),
        image: gif.clone().or_else(|| cover.clone()),
        still_image: cover,
        emoji: clean(&folder.cover_emoji),
        shape: shape.into(),
        hide_title: folder.hide_title,
        backdrop: clean(&folder.hero_backdrop_url),
        logo: clean(&folder.title_logo_url),
        sources: resolved,
        skipped_sources: skipped,
    }
}

/// Collections that can appear on Home (at least one folder), in the stored order.
pub fn views(value: &Value, manifests: &[AddonManifest], show_catalog_type: bool) -> Vec<CollectionView> {
    views_with(value, manifests, show_catalog_type, false)
}

/// Which services can resolve folder sources.
#[derive(Clone, Copy)]
struct Ready {
    tmdb: bool,
}

/// `views`; with `tmdb`, the TMDB sources of the folders are listed too.
pub fn views_with(value: &Value, manifests: &[AddonManifest], show_catalog_type: bool, tmdb: bool) -> Vec<CollectionView> {
    let ready = Ready { tmdb };
    parse(value)
        .into_iter()
        .enumerate()
        .filter(|(_, c)| !c.folders.is_empty())
        .map(|(ci, c)| CollectionView {
            folders: c
                .folders
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.id.trim().is_empty())
                .map(|(fi, f)| folder_view(f, manifests, show_catalog_type, Some((ci, fi)), ready))
                .collect(),
            id: c.id,
            title: c.title,
            backdrop: clean(&c.backdrop_image_url),
            pin_to_top: c.pin_to_top,
            view_mode: c.view_mode.to_uppercase(),
            show_all_tab: c.show_all_tab,
        })
        .filter(|c| !c.folders.is_empty())
        .collect()
}


// ---------------------------------------------------------------- editor (`CollectionEditorRepository`)
// The stored JSON is edited in place (`CollectionJsonPreserver`): fields Astra does not know stay
// as they were, and TMDB/Trakt sources are kept untouched behind the addon catalogs edited here.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EditSource {
    pub addon_id: String,
    pub content_type: String,
    pub catalog_id: String,
    pub genre: Option<String>,
    /// Display only (filled when listed).
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EditFolder {
    pub id: String,
    pub title: String,
    pub cover_image: Option<String>,
    pub cover_emoji: Option<String>,
    /// `poster`, `landscape` or `square`.
    pub tile_shape: String,
    pub hide_title: bool,
    pub sources: Vec<EditSource>,
    /// TMDB / Trakt sources kept as they are (not editable in Astra).
    #[serde(default)]
    pub other_sources: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EditCollection {
    pub id: String,
    pub title: String,
    pub backdrop: Option<String>,
    /// `TABBED_GRID` or `ROWS`.
    pub view_mode: String,
    pub show_all_tab: bool,
    pub pin_to_top: bool,
    pub folders: Vec<EditFolder>,
}

fn is_addon_source(source: &StoredSource) -> bool {
    let provider = clean(&source.provider).unwrap_or_else(|| "addon".into());
    !(provider.eq_ignore_ascii_case("tmdb") || provider.eq_ignore_ascii_case("trakt"))
}

/// The stored collections in the editor's shape.
pub fn editable(value: &Value, manifests: &[AddonManifest]) -> Vec<EditCollection> {
    parse(value)
        .into_iter()
        .map(|c| EditCollection {
            folders: c
                .folders
                .iter()
                .map(|f| {
                    let list = if f.sources.is_empty() { &f.catalog_sources } else { &f.sources };
                    let (addon, other): (Vec<&StoredSource>, Vec<&StoredSource>) = list.iter().partition(|s| is_addon_source(s));
                    EditFolder {
                        id: f.id.clone(),
                        title: f.title.clone(),
                        cover_image: clean(&f.cover_image_url),
                        cover_emoji: clean(&f.cover_emoji),
                        tile_shape: f.tile_shape.clone(),
                        hide_title: f.hide_title,
                        other_sources: other.len(),
                        sources: addon
                            .into_iter()
                            .filter_map(|s| {
                                let (addon_id, content_type, catalog_id) = (clean(&s.addon_id)?, clean(&s.content_type)?, clean(&s.catalog_id)?);
                                let label = resolve(manifests, &addon_id, &content_type, &catalog_id)
                                    .map(|(m, cat)| format!("{} · {} ({})", cat.name, media_type_label(&cat.content_type), m.name))
                                    .unwrap_or_else(|| format!("{catalog_id} ({addon_id})"));
                                Some(EditSource { addon_id, content_type, catalog_id, genre: genre_of(&s.genre), label })
                            })
                            .collect(),
                    }
                })
                .collect(),
            id: c.id,
            title: c.title,
            backdrop: clean(&c.backdrop_image_url),
            view_mode: c.view_mode,
            show_all_tab: c.show_all_tab,
            pin_to_top: c.pin_to_top,
        })
        .collect()
}

fn set(obj: &mut serde_json::Map<String, Value>, key: &str, value: Value) {
    obj.insert(key.to_string(), value);
}

fn text_or_null(v: &Option<String>) -> Value {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map_or(Value::Null, |s| Value::String(s.to_string()))
}

/// The stored JSON with the editor's collections, in the editor's order. Ids not in `edits` are
/// removed; new ids are added; unknown fields of the existing ones are kept.
pub fn apply(value: &Value, edits: &[EditCollection]) -> Value {
    let old: Vec<Value> = value.as_array().cloned().unwrap_or_default();
    let find = |list: &[Value], id: &str| list.iter().find(|v| v.get("id").and_then(Value::as_str) == Some(id)).cloned();
    let out: Vec<Value> = edits
        .iter()
        .filter(|c| !c.id.trim().is_empty())
        .map(|c| {
            let mut obj = find(&old, &c.id).and_then(|v| v.as_object().cloned()).unwrap_or_default();
            let old_folders: Vec<Value> = obj.get("folders").and_then(Value::as_array).cloned().unwrap_or_default();
            set(&mut obj, "id", Value::String(c.id.clone()));
            set(&mut obj, "title", Value::String(c.title.trim().to_string()));
            set(&mut obj, "backdropImageUrl", text_or_null(&c.backdrop));
            set(&mut obj, "viewMode", Value::String(if c.view_mode == "ROWS" { "ROWS".into() } else { "TABBED_GRID".into() }));
            set(&mut obj, "showAllTab", Value::Bool(c.show_all_tab));
            set(&mut obj, "pinToTop", Value::Bool(c.pin_to_top));
            let folders: Vec<Value> = c
                .folders
                .iter()
                .filter(|f| !f.id.trim().is_empty())
                .map(|f| {
                    let mut fo = find(&old_folders, &f.id).and_then(|v| v.as_object().cloned()).unwrap_or_default();
                    let previous: Vec<Value> = fo
                        .get("sources")
                        .and_then(Value::as_array)
                        .filter(|a| !a.is_empty())
                        .or_else(|| fo.get("catalogSources").and_then(Value::as_array))
                        .cloned()
                        .unwrap_or_default();
                    // TMDB / Trakt sources stay, after the addon catalogs.
                    let kept: Vec<Value> = previous
                        .into_iter()
                        .filter(|v| serde_json::from_value::<StoredSource>(v.clone()).is_ok_and(|s| !is_addon_source(&s)))
                        .collect();
                    let mut sources: Vec<Value> = f
                        .sources
                        .iter()
                        .map(|s| serde_json::json!({ "provider": "addon", "addonId": s.addon_id, "type": s.content_type, "catalogId": s.catalog_id, "genre": s.genre }))
                        .collect();
                    sources.extend(kept);
                    set(&mut fo, "id", Value::String(f.id.clone()));
                    set(&mut fo, "title", Value::String(f.title.trim().to_string()));
                    set(&mut fo, "coverImageUrl", text_or_null(&f.cover_image));
                    set(&mut fo, "coverEmoji", text_or_null(&f.cover_emoji));
                    set(&mut fo, "tileShape", Value::String(match f.tile_shape.as_str() { "landscape" | "square" => f.tile_shape.clone(), _ => "poster".into() }));
                    set(&mut fo, "hideTitle", Value::Bool(f.hide_title));
                    set(&mut fo, "sources", Value::Array(sources));
                    set(&mut fo, "catalogSources", Value::Array(Vec::new()));
                    Value::Object(fo)
                })
                .collect();
            set(&mut obj, "folders", Value::Array(folders));
            Value::Object(obj)
        })
        .collect();
    Value::Array(out)
}

/// A catalog of the installed addons that a folder can use.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogOption {
    pub addon_id: String,
    pub addon_name: String,
    pub content_type: String,
    pub catalog_id: String,
    pub label: String,
    pub genres: Vec<String>,
}

pub fn catalog_options(manifests: &[AddonManifest]) -> Vec<CatalogOption> {
    manifests
        .iter()
        .flat_map(|m| {
            m.catalogs.iter().filter(|c| !c.extra.iter().any(|e| e.name == "search" && e.is_required)).map(move |c| CatalogOption {
                addon_id: m.id.clone(),
                addon_name: m.name.clone(),
                content_type: c.content_type.clone(),
                catalog_id: c.id.clone(),
                label: format!("{} · {}", c.name, media_type_label(&c.content_type)),
                genres: c.extra.iter().find(|e| e.name == "genre").map(|e| e.options.clone()).unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_core::manifest::parse_manifest;
    use serde_json::json;

    fn manifests() -> Vec<AddonManifest> {
        vec![
            parse_manifest(
                "https://a.test/manifest.json",
                r#"{"id":"a","name":"Addon A","version":"1","resources":["catalog"],"types":["movie"],
                   "catalogs":[{"type":"movie","id":"top","name":"Top"},{"type":"series","id":"pop","name":"Popular","extra":[{"name":"genre"}]}]}"#,
            )
            .unwrap(),
            parse_manifest(
                "https://b.test/manifest.json",
                r#"{"id":"b","name":"Addon B","version":"1","resources":["catalog"],"types":["movie"],"catalogs":[{"type":"movie","id":"new","name":"New"}]}"#,
            )
            .unwrap(),
        ]
    }

    #[test]
    fn folders_resolve_addon_sources_and_skip_the_rest() {
        let stored = json!([
            {"id":"c1","title":"Estúdios","folders":[
                {"id":"f1","title":"Marvel","coverImageUrl":"https://img/marvel.jpg","focusGifUrl":"https://img/marvel.gif","tileShape":"WIDE",
                 "sources":[
                    {"provider":"addon","addonId":"a","type":"movie","catalogId":"top"},
                    {"provider":"addon","addonId":"gone","type":"movie","catalogId":"new"},
                    {"provider":"addon","addonId":"a","type":"series","catalogId":"pop,extra","genre":"Drama"},
                    {"provider":"tmdb","tmdbId":420,"title":"Marvel"},
                    {"provider":"addon","addonId":"x","type":"movie","catalogId":"missing"}]},
                {"id":"f2","title":"Sem GIF","coverImageUrl":"https://img/a.jpg","focusGifUrl":"https://img/a.gif","focusGifEnabled":false,
                 "catalogSources":[{"addonId":"b","type":"movie","catalogId":"new","genre":"None"}]}
            ]},
            {"id":"empty","title":"Vazia","folders":[]},
            {"id":"c1","title":"Repetida","folders":[{"id":"x","title":"x"}]},
            {"title":"Sem id","folders":[{"id":"y","title":"y"}]},
            "lixo"
        ]);
        let views = views(&stored, &manifests(), true);
        assert_eq!(views.len(), 1);
        let c = &views[0];
        assert_eq!((c.view_mode.as_str(), c.show_all_tab, c.pin_to_top), ("TABBED_GRID", true, false));
        let f1 = &c.folders[0];
        assert_eq!(f1.image.as_deref(), Some("https://img/marvel.gif"));
        assert_eq!(f1.still_image.as_deref(), Some("https://img/marvel.jpg"));
        assert_eq!(f1.shape, "landscape");
        let keys: Vec<_> = f1.sources.iter().map(|s| (s.key.as_str(), s.label.as_str(), s.genre.as_deref())).collect();
        assert_eq!(
            keys,
            vec![("a:movie:top", "Top - Filmes", None), ("b:movie:new", "New - Filmes", None), ("a:series:pop", "Popular · Drama - Séries", Some("Drama"))]
        );
        assert_eq!(f1.skipped_sources, 2);
        let f2 = &c.folders[1];
        assert_eq!(f2.image.as_deref(), Some("https://img/a.jpg"), "GIF turned off in the folder");
        assert_eq!(f2.sources[0].genre, None, "\"None\" is no genre");
        assert_eq!(f2.shape, "poster");
    }

    #[test]
    fn the_editor_keeps_what_astra_does_not_know() {
        let stored = serde_json::json!([{
            "id": "c1", "title": "Velha", "viewMode": "ROWS", "futureField": 7,
            "folders": [{"id": "f1", "title": "Pasta", "tileShape": "poster", "focusGifUrl": "https://g.gif", "sources": [
                {"provider": "addon", "addonId": "a", "type": "movie", "catalogId": "top"},
                {"provider": "tmdb", "tmdbListId": 5}
            ]}]
        }]);
        let mut edits = editable(&stored, &[]);
        assert_eq!(edits[0].folders[0].other_sources, 1);
        edits[0].title = "Nova".into();
        edits[0].folders[0].sources.push(EditSource { addon_id: "b".into(), content_type: "series".into(), catalog_id: "pop".into(), genre: Some("Drama".into()), label: String::new() });
        edits.push(EditCollection { id: "c2".into(), title: "Outra".into(), backdrop: None, view_mode: "TABBED_GRID".into(), show_all_tab: true, pin_to_top: false, folders: vec![] });
        let out = apply(&stored, &edits);
        assert_eq!(out[0]["title"], "Nova");
        assert_eq!(out[0]["futureField"], 7, "unknown fields stay");
        assert_eq!(out[0]["viewMode"], "ROWS");
        let folder = &out[0]["folders"][0];
        assert_eq!(folder["focusGifUrl"], "https://g.gif");
        let sources = folder["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[1]["genre"], "Drama");
        assert_eq!(sources[2]["provider"], "tmdb", "TMDB source kept after the addon catalogs");
        assert_eq!(out[1]["id"], "c2");
        let removed = apply(&stored, &[]);
        assert_eq!(removed.as_array().unwrap().len(), 0);
    }
}
