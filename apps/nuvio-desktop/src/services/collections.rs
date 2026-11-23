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
    id: Option<String>,
    provider: Option<String>,
    addon_id: Option<String>,
    #[serde(rename = "type")]
    content_type: Option<String>,
    catalog_id: Option<String>,
    genre: Option<String>,
    /// TMDB sources: media type, source kind, sort and discover filters.
    media_type: Option<String>,
    tmdb_source_type: Option<String>,
    sort_by: Option<String>,
    tmdb_id: Option<i64>,
    filters: Option<super::tmdb::Filters>,
    name: Option<String>,
    title: Option<String>,
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
    /// `addon` (a catalog of an installed addon) or `tmdb` (a discover query resolved in Rust).
    pub provider: String,
    /// Catalog key `<manifestId>:<type>:<catalogId>` (addon sources, as the Home rows).
    pub key: String,
    /// Stored source id of a TMDB source (the page is asked with the collection and folder ids).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
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

/// `findCollectionCatalog`: a catalog of the declared addon when it still exists, else any enabled
/// addon with the same catalog. Several installed manifests can share an id (more than one
/// AIOMetadata configuration, for instance), so every manifest with that id is searched, not only
/// the first one.
pub fn resolve_catalog<'a>(
    manifests: &'a [AddonManifest],
    manifest_id: &str,
    content_type: &str,
    catalog_id: &str,
) -> Option<(&'a AddonManifest, &'a AddonCatalog)> {
    let find = |m: &'a AddonManifest| find_catalog(&m.catalogs, content_type, catalog_id).map(|c| (m, c));
    manifests.iter().filter(|m| m.id == manifest_id).find_map(find).or_else(|| manifests.iter().find_map(find))
}

fn folder_view(folder: &StoredFolder, manifests: &[AddonManifest], show_catalog_type: bool, collection_id: &str, tmdb_available: bool) -> FolderView {
    let sources = if folder.sources.is_empty() { &folder.catalog_sources } else { &folder.sources };
    let mut resolved = Vec::new();
    let mut skipped = 0;
    for source in sources {
        let provider = clean(&source.provider).unwrap_or_else(|| "addon".into());
        if provider.eq_ignore_ascii_case("tmdb") {
            let Some(source_id) = clean(&source.id) else {
                skipped += 1;
                continue;
            };
            let source_type = clean(&source.tmdb_source_type).unwrap_or_else(|| "DISCOVER".into());
            // Without a TMDB key no TMDB source is listed (as before).
            if !tmdb_available || !super::tmdb::supported(&source_type) {
                skipped += 1;
                continue;
            }
            let media_type = if source_type.eq_ignore_ascii_case("NETWORK") {
                super::tmdb::MediaType::Tv
            } else {
                super::tmdb::MediaType::parse(source.media_type.as_deref())
            };
            let name = clean(&source.title)
                .or_else(|| clean(&source.name))
                .or_else(|| clean(&source.genre))
                .unwrap_or_else(|| "TMDB".into());
            let content_type = media_type.content_type();
            // The original's TMDB tabs show the source title only (no " - Movies" suffix).
            resolved.push(FolderSource {
                provider: "tmdb".into(),
                key: format!("tmdb:{collection_id}:{}:{source_id}", folder.id),
                source_id: Some(source_id),
                genre: None,
                label: name,
                addon_name: "TMDB".into(),
                content_type: content_type.into(),
                // The original pages discover, lists, networks and companies; not a collection or a
                // person's credits.
                supports_pagination: !matches!(source_type.to_uppercase().as_str(), "COLLECTION" | "PERSON" | "DIRECTOR"),
            });
            continue;
        }
        if provider.eq_ignore_ascii_case("trakt") {
            skipped += 1;
            continue;
        }
        let (Some(addon_id), Some(content_type), Some(catalog_id)) = (clean(&source.addon_id), clean(&source.content_type), clean(&source.catalog_id)) else {
            skipped += 1;
            continue;
        };
        let Some((manifest, catalog)) = resolve_catalog(manifests, &addon_id, &content_type, &catalog_id) else {
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
            provider: "addon".into(),
            key: format!("{}:{}:{}", manifest.id, catalog.content_type, catalog.id),
            source_id: None,
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

/// The stored TMDB source behind a folder tab (`tmdb_source_page`): its kind, media type, sort and
/// discover filters, taken from the account's copy of the collections (never from the UI).
pub fn tmdb_source(value: &Value, collection_id: &str, folder_id: &str, source_id: &str) -> Option<super::tmdb::Source> {
    let collection = parse(value).into_iter().find(|c| c.id == collection_id)?;
    let folder = collection.folders.iter().find(|f| f.id == folder_id)?;
    let sources = if folder.sources.is_empty() { &folder.catalog_sources } else { &folder.sources };
    let source = sources.iter().find(|s| clean(&s.id).as_deref() == Some(source_id))?;
    if !clean(&source.provider).is_some_and(|p| p.eq_ignore_ascii_case("tmdb")) {
        return None;
    }
    let source_type = clean(&source.tmdb_source_type).unwrap_or_else(|| "DISCOVER".into());
    if !super::tmdb::supported(&source_type) {
        return None;
    }
    let media_type = if source_type.eq_ignore_ascii_case("NETWORK") {
        super::tmdb::MediaType::Tv
    } else {
        super::tmdb::MediaType::parse(source.media_type.as_deref())
    };
    Some(super::tmdb::Source {
        source_type,
        media_type,
        sort_by: clean(&source.sort_by),
        tmdb_id: source.tmdb_id,
        filters: source.filters.clone().unwrap_or_default(),
    })
}

/// Collections that can appear on Home (at least one folder), in the stored order.
pub fn views(value: &Value, manifests: &[AddonManifest], show_catalog_type: bool, tmdb_available: bool) -> Vec<CollectionView> {
    parse(value)
        .into_iter()
        .filter(|c| !c.folders.is_empty())
        .map(|c| CollectionView {
            folders: c.folders.iter().filter(|f| !f.id.trim().is_empty()).map(|f| folder_view(f, manifests, show_catalog_type, &c.id, tmdb_available)).collect(),
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
        let views = views(&stored, &manifests(), true, false);
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

    /// TMDB sources of a folder (the Netflix tabs of "Streaming Services", the lists of Actors and
    /// Directors, the parts of Film Collections): listed when the profile has a TMDB key.
    #[test]
    fn tmdb_sources_become_folder_tabs_when_a_key_is_configured() {
        let stored = json!([{"id":"collection-1","title":"Services","folders":[{"id":"folder-1","title":"Netflix",
            "sources":[
                {"id":"src-1","provider":"tmdb","mediaType":"TV","tmdbSourceType":"DISCOVER","sortBy":"popularity.desc",
                 "name":"New Series","title":"New Series","genre":"New Series",
                 "filters":{"withoutGenres":"16,10764","watchRegion":"US","withWatchProviders":"8|1796"}},
                {"id":"src-2","provider":"tmdb","mediaType":"MOVIE","tmdbSourceType":"NETWORK","tmdbId":213,"title":"Netflix"},
                {"id":"src-3","provider":"tmdb","mediaType":"MOVIE","tmdbSourceType":"COLLECTION","tmdbId":1,"title":"Saga"},
                {"id":"src-4","provider":"tmdb","mediaType":"MOVIE","tmdbSourceType":"LIST","tmdbId":8659010,"sortBy":"vote_average.desc","title":"Top Rated"}]}]}]);
        let with_key = views(&stored, &[], true, true);
        let tabs: Vec<_> = with_key[0].folders[0]
            .sources
            .iter()
            .map(|s| (s.provider.as_str(), s.label.as_str(), s.content_type.as_str(), s.source_id.as_deref()))
            .collect();
        assert_eq!(
            tabs,
            vec![
                ("tmdb", "New Series", "series", Some("src-1")),
                ("tmdb", "Netflix", "series", Some("src-2")),
                ("tmdb", "Saga", "movie", Some("src-3")),
                ("tmdb", "Top Rated", "movie", Some("src-4")),
            ]
        );
        assert_eq!(with_key[0].folders[0].skipped_sources, 0);

        // Without a key the tabs are not listed (as before).
        assert!(views(&stored, &[], true, false)[0].folders[0].sources.is_empty());

        let source = tmdb_source(&stored, "collection-1", "folder-1", "src-1").expect("discover source");
        assert_eq!(source.media_type, super::super::tmdb::MediaType::Tv);
        assert_eq!(source.filters.with_watch_providers.as_deref(), Some("8|1796"));
        assert_eq!(source.filters.without_genres.as_deref(), Some("16,10764"));
        let network = tmdb_source(&stored, "collection-1", "folder-1", "src-2").expect("network source");
        assert_eq!(network.media_type, super::super::tmdb::MediaType::Tv, "networks are TV");
        assert_eq!(network.tmdb_id, Some(213));
        let collection = tmdb_source(&stored, "collection-1", "folder-1", "src-3").expect("collection source");
        assert_eq!(collection.tmdb_id, Some(1));
        assert!(!with_key[0].folders[0].sources[2].supports_pagination, "a collection has no pages");
        assert!(with_key[0].folders[0].sources[3].supports_pagination, "a list pages");
        let list = tmdb_source(&stored, "collection-1", "folder-1", "src-4").expect("list source");
        assert_eq!(list.sort_by.as_deref(), Some("vote_average.desc"));
    }

    /// Several AIOMetadata configurations installed at once share the manifest id `aio-metadata`;
    /// a source of the second one must resolve (and load) with its own catalog.
    #[test]
    fn addons_with_the_same_manifest_id_each_resolve_their_catalogs() {
        let manifests = vec![
            parse_manifest(
                "https://one.test/manifest.json",
                r#"{"id":"aio","name":"AIO 1","version":"1","resources":["catalog"],"types":["movie"],
                    "catalogs":[{"type":"movie","id":"shared","name":"Shared"}]}"#,
            )
            .unwrap(),
            parse_manifest(
                "https://two.test/manifest.json",
                r#"{"id":"aio","name":"AIO 2","version":"1","resources":["catalog"],"types":["movie"],
                    "catalogs":[{"type":"movie","id":"shared","name":"Shared"},{"type":"movie","id":"only-two","name":"Only two"}]}"#,
            )
            .unwrap(),
        ];
        let (manifest, catalog) = resolve_catalog(&manifests, "aio", "movie", "only-two").expect("found in the second addon");
        assert_eq!((manifest.name.as_str(), catalog.name.as_str()), ("AIO 2", "Only two"));
        // The declared addon wins for a catalog both of them have.
        let (manifest, _) = resolve_catalog(&manifests, "aio", "movie", "shared").unwrap();
        assert_eq!(manifest.name, "AIO 1");

        let stored = json!([{"id":"c","title":"C","folders":[{"id":"f","title":"F",
            "sources":[{"provider":"addon","addonId":"aio","type":"movie","catalogId":"only-two"}]}]}]);
        let views = views(&stored, &manifests, false, false);
        assert_eq!(views[0].folders[0].sources.len(), 1);
        assert_eq!(views[0].folders[0].sources[0].addon_name, "AIO 2");
        assert_eq!(views[0].folders[0].skipped_sources, 0);
    }
}
