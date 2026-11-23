//! Home collections (`CollectionModels.kt`, `HomeCollectionRowSection.kt`, `FolderDetailRepository.kt`).
//!
//! A collection is a Home row of folders; each folder has a cover (image, animated GIF/WebP or a
//! short video, or an emoji) and a list of sources. Sources of type `addon` are catalogs of the
//! profile's addons (resolved as `findCollectionCatalog`: the declared addon first, then any
//! enabled addon with the same catalog); TMDB and Trakt sources need services Astra does not have
//! and are skipped. The collections come from the account (`sync_pull_collections`, read only) or,
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

fn folder_view(folder: &StoredFolder, manifests: &[AddonManifest], show_catalog_type: bool) -> FolderView {
    let sources = if folder.sources.is_empty() { &folder.catalog_sources } else { &folder.sources };
    let mut resolved = Vec::new();
    let mut skipped = 0;
    for source in sources {
        let provider = clean(&source.provider).unwrap_or_else(|| "addon".into());
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
    parse(value)
        .into_iter()
        .filter(|c| !c.folders.is_empty())
        .map(|c| CollectionView {
            folders: c.folders.iter().filter(|f| !f.id.trim().is_empty()).map(|f| folder_view(f, manifests, show_catalog_type)).collect(),
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
}
