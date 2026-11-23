//! Discover: the type / catalog / genre browser of the Search screen.
//!
//! Port of `SearchRepository.refreshDiscover`, `buildDiscoverSources`, `selectDiscoverType`,
//! `selectDiscoverCatalog`, `selectDiscoverGenre`, `resolveDiscoverCatalog`,
//! `resolveGenreSelection` and the paging rules of `nextCatalogPaginationState`.
//!
//! Sources are the enabled addons' catalogs that can be listed without asking for anything the
//! picker cannot fill (`AddonCatalog::supports_discover`). A catalog's `genre` extra feeds the
//! genre picker; a required genre starts on its first option. The chosen catalog is remembered per
//! profile (`DiscoverSelectionStorage` in the original, store `nuvio_discover_selection`).
//!
//! The original also listed the libraries of media servers (Plex, Emby/Jellyfin, Silo). Media
//! servers are not ported yet, so only addon catalogs are listed here.

use nuvio_core::catalog::{CatalogPage, MetaPreview};
use nuvio_core::manifest::{AddonCatalog, AddonManifest};
use nuvio_store::profiles::addon_key;
use serde::Serialize;

/// `CATALOG_PAGE_SIZE`: a catalog that answers with a full page paginates even without a `skip`
/// extra.
pub const CATALOG_PAGE_SIZE: usize = 100;
/// `DUPLICATE_CATALOG_PAGE_ADVANCE_LIMIT`: stop after this many pages that brought nothing new.
pub const DUPLICATE_PAGE_ADVANCE_LIMIT: u32 = 3;

/// One catalog the Discover picker can list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    /// `<addon key>:<type>:<catalogId>`, the key the UI sends back to load a page. The addon key is
    /// the opaque hash of the manifest URL (`addon_key`), never the URL, so several addons that
    /// share a manifest id (more than one AIOMetadata configuration, for instance) keep their own
    /// catalogs.
    pub key: String,
    pub addon_name: String,
    pub content_type: String,
    pub catalog_id: String,
    pub catalog_name: String,
    /// Options of the catalog's `genre` extra (empty when it has none).
    pub genre_options: Vec<String>,
    pub genre_required: bool,
    /// Genre the picker starts on (`resolveGenreSelection(null)`).
    pub default_genre: Option<String>,
    pub supports_pagination: bool,
}

/// Everything the Discover picker needs to draw itself and restore the last choice.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub sources: Vec<Source>,
    /// Distinct content types in source order (the type picker).
    pub type_options: Vec<String>,
    /// Catalog to open first: the one remembered for this profile when it is still installed,
    /// otherwise the first source (`resolveDiscoverCatalog`).
    pub selected_key: Option<String>,
}

/// One page of Discover, with the pagination decision already applied.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// Items of this page (after the unreleased filter).
    pub items: Vec<MetaPreview>,
    /// `skip` of the next request, or `None` when this catalog has no more pages.
    pub next_skip: Option<u32>,
    /// Pages in a row that brought nothing new (resets when a page brings something).
    pub duplicate_pages: u32,
    /// Whether this catalog paginates (the manifest's `skip` extra or a full page).
    pub supports_pagination: bool,
}

/// Catalogs of the enabled addons that Discover can list, in addon order.
pub fn sources(manifests: &[AddonManifest]) -> Vec<Source> {
    manifests
        .iter()
        .flat_map(|manifest| {
            manifest
                .catalogs
                .iter()
                .filter(|catalog| catalog.supports_discover())
                .map(move |catalog| {
                    let genre = catalog.genre_extra();
                    let addon_key = addon_key(&manifest.transport_url);
                    let mut source = Source {
                        key: format!("{addon_key}:{}:{}", catalog.content_type, catalog.id),
                        addon_name: manifest.name.clone(),
                        content_type: catalog.content_type.clone(),
                        catalog_id: catalog.id.clone(),
                        catalog_name: catalog.name.clone(),
                        genre_options: genre.map(|g| g.options.clone()).unwrap_or_default(),
                        genre_required: genre.is_some_and(|g| g.is_required),
                        default_genre: None,
                        supports_pagination: catalog.supports_pagination(),
                    };
                    source.default_genre = resolve_genre(&source, None);
                    source
                })
        })
        .collect()
}

/// Resolves a `Source::key` (`<addon key>:<type>:<catalog id>`) to the manifest and catalog it
/// belongs to. Only enabled manifests are searched; a key of another profile or of an addon that
/// is no longer installed finds nothing.
pub fn catalog<'a>(manifests: &'a [AddonManifest], key: &str) -> Option<(&'a AddonManifest, &'a AddonCatalog)> {
    let mut parts = key.splitn(3, ':');
    let (addon, content_type, catalog_id) = (parts.next()?, parts.next()?, parts.next()?);
    let manifest = manifests.iter().find(|m| addon_key(&m.transport_url) == addon)?;
    let catalog = manifest.catalogs.iter().find(|c| c.content_type == content_type && c.id == catalog_id)?;
    Some((manifest, catalog))
}

/// Distinct content types, in source order (`sources.map { it.type }.distinct()`).
pub fn type_options(sources: &[Source]) -> Vec<String> {
    let mut types: Vec<String> = Vec::new();
    for source in sources {
        if !types.contains(&source.content_type) {
            types.push(source.content_type.clone());
        }
    }
    types
}

/// `resolveDiscoverCatalog`: the remembered catalog when it still exists, else the last one, else
/// the first.
pub fn resolve_catalog<'a>(sources: &'a [Source], preferred: Option<&str>, current: Option<&str>) -> Option<&'a Source> {
    preferred
        .and_then(|key| sources.iter().find(|s| s.key == key))
        .or_else(|| current.and_then(|key| sources.iter().find(|s| s.key == key)))
        .or_else(|| sources.first())
}

/// `resolveGenreSelection`: no options means no genre filter; a requested option that exists is
/// kept; a required genre falls back to its first option; otherwise all genres.
pub fn resolve_genre(source: &Source, requested: Option<&str>) -> Option<String> {
    if source.genre_options.is_empty() {
        return None;
    }
    if let Some(requested) = requested.filter(|r| source.genre_options.iter().any(|o| o == r)) {
        return Some(requested.to_string());
    }
    if source.genre_required {
        return source.genre_options.first().cloned();
    }
    None
}

/// `nextCatalogPaginationState`: no pagination when the catalog cannot page, answered nothing, or
/// reported no next `skip`. A page that brought new items advances to the addon's next `skip` and
/// resets the duplicate counter; a page that brought nothing advances by the raw page size, at
/// most three times in a row.
pub fn next_pagination(
    supports_pagination: bool,
    requested_skip: u32,
    raw_item_count: usize,
    page_next_skip: Option<u32>,
    loaded_new_items: bool,
    duplicate_pages: u32,
) -> (Option<u32>, u32) {
    let Some(page_next_skip) = page_next_skip.filter(|_| supports_pagination && raw_item_count > 0) else {
        return (None, 0);
    };
    if loaded_new_items {
        return (Some(page_next_skip), 0);
    }
    let duplicates = duplicate_pages + 1;
    let advanced_skip = if page_next_skip > requested_skip {
        page_next_skip
    } else {
        requested_skip + raw_item_count.max(1) as u32
    };
    if duplicates < DUPLICATE_PAGE_ADVANCE_LIMIT && advanced_skip > requested_skip {
        (Some(advanced_skip), duplicates)
    } else {
        (None, duplicates)
    }
}

/// Pagination decision for one Discover page, using the page before the unreleased filter (paging
/// follows the addon's raw page size).
pub fn page(
    catalog_supports_pagination: bool,
    requested_skip: u32,
    reset: bool,
    existing_keys: &[String],
    duplicate_pages: u32,
    page: CatalogPage,
) -> Page {
    let supports_pagination = catalog_supports_pagination || page.raw_item_count >= CATALOG_PAGE_SIZE;
    let loaded_new_items = reset || page.items.iter().any(|item| !existing_keys.contains(&item.stable_key()));
    let (next_skip, duplicate_pages) = next_pagination(
        supports_pagination,
        requested_skip,
        page.raw_item_count,
        page.next_skip,
        loaded_new_items,
        duplicate_pages,
    );
    Page { items: page.items, next_skip, duplicate_pages, supports_pagination }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_core::manifest::parse_manifest;

    fn manifests() -> Vec<AddonManifest> {
        vec![
            parse_manifest(
                "https://a.test/manifest.json",
                r#"{"id":"a","name":"Addon A","version":"1","resources":["catalog"],"types":["movie","series"],
                    "catalogs":[
                        {"type":"movie","id":"top","name":"Top"},
                        {"type":"movie","id":"search","name":"Search","extra":[{"name":"search","isRequired":true}]},
                        {"type":"movie","id":"genres","name":"Por gênero","extra":[{"name":"genre","isRequired":true,"options":["Action","Drama"]}]},
                        {"type":"movie","id":"paged","name":"Paged","extra":[{"name":"skip"}]},
                        {"type":"series","id":"new","name":"New"}]}"#,
            )
            .unwrap(),
            parse_manifest(
                "https://b.test/manifest.json",
                r#"{"id":"b","name":"Addon B","version":"1","resources":["catalog"],"types":["movie"],
                    "catalogs":[{"type":"movie","id":"popular","name":"Popular","extra":[{"name":"genre","options":["Comedy"]}]}]}"#,
            )
            .unwrap(),
        ]
    }

    #[test]
    fn sources_are_the_listable_catalogs_of_the_enabled_addons_in_order() {
        let sources = sources(&manifests());
        let suffixes: Vec<&str> = sources
            .iter()
            .map(|s| s.key.split_once(':').map(|(_, rest)| rest).unwrap_or(""))
            .collect();
        assert_eq!(suffixes, ["movie:top", "movie:genres", "movie:paged", "series:new", "movie:popular"]);
        assert!(sources.iter().all(|s| s.key.split(':').next().is_some_and(|addon| addon.len() == 16)));
        assert_eq!(sources[0].addon_name, "Addon A");
        assert_eq!(sources[1].genre_options, ["Action", "Drama"]);
        assert!(sources[1].genre_required);
        assert_eq!(sources[1].default_genre.as_deref(), Some("Action"));
        assert!(!sources[0].genre_required);
        assert_eq!(sources[0].default_genre, None);
        assert!(sources[2].supports_pagination);
        assert!(!sources[0].supports_pagination);
        assert_eq!(sources[4].default_genre, None, "optional genre starts on all genres");
    }

    /// Two addons with the same manifest id (several AIOMetadata configurations, for instance) are
    /// told apart by the opaque key of their manifest URL, so each catalog loads from its own addon.
    #[test]
    fn addons_with_the_same_manifest_id_keep_their_own_catalogs() {
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
        let sources = sources(&manifests);
        let keys: Vec<&str> = sources.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys.len(), 3);
        assert_ne!(keys[0], keys[1], "same catalog id and type, different addons");
        assert_eq!(keys[0].split(':').nth(1), Some("movie"));
        assert_eq!(keys[0].rsplit(':').next(), Some("shared"));

        // Every source resolves to the addon it came from.
        for source in &sources {
            let (manifest, catalog) = catalog(&manifests, &source.key).expect("catalog resolves");
            assert_eq!(manifest.name, source.addon_name);
            assert_eq!(catalog.id, source.catalog_id);
            assert_eq!(catalog.content_type, source.content_type);
        }
        assert!(catalog(&manifests, "0000000000000000:movie:only-two").is_none());
    }

    #[test]
    fn type_options_are_distinct_and_keep_the_source_order() {
        assert_eq!(type_options(&sources(&manifests())), ["movie", "series"]);
        assert!(type_options(&[]).is_empty());
    }

    #[test]
    fn remembered_catalog_wins_then_the_previous_one_then_the_first() {
        let sources = sources(&manifests());
        let key = |s: Option<&Source>| s.map(|s| s.key.clone());
        assert_eq!(key(resolve_catalog(&sources, Some(sources[2].key.as_str()), Some(sources[0].key.as_str()))), Some(sources[2].key.clone()));
        assert_eq!(key(resolve_catalog(&sources, Some("gone:movie:x"), Some(sources[3].key.as_str()))), Some(sources[3].key.clone()));
        assert_eq!(key(resolve_catalog(&sources, None, None)), Some(sources[0].key.clone()));
        assert!(resolve_catalog(&[], Some(sources[0].key.as_str()), None).is_none());
    }

    #[test]
    fn genre_selection_follows_the_original_rules() {
        let source = |genre_required: bool, options: &[&str]| Source {
            key: "0123456789abcdef:movie:x".into(),
            addon_name: "A".into(),
            content_type: "movie".into(),
            catalog_id: "x".into(),
            catalog_name: "X".into(),
            genre_options: options.iter().map(|o| o.to_string()).collect(),
            genre_required,
            default_genre: None,
            supports_pagination: false,
        };
        let all = source(false, &["Action", "Drama"]);
        assert_eq!(resolve_genre(&all, None), None);
        assert_eq!(resolve_genre(&all, Some("Drama")).as_deref(), Some("Drama"));
        assert_eq!(resolve_genre(&all, Some("Comedy")), None, "unknown option → all genres");
        let required = source(true, &["Action", "Drama"]);
        assert_eq!(resolve_genre(&required, None).as_deref(), Some("Action"));
        assert_eq!(resolve_genre(&required, Some("Comedy")).as_deref(), Some("Action"));
        assert_eq!(resolve_genre(&source(true, &[]), Some("Action")), None);
    }

    #[test]
    fn a_page_that_brings_new_items_advances_and_resets_the_duplicate_counter() {
        assert_eq!(next_pagination(true, 0, 20, Some(20), true, 2), (Some(20), 0));
    }

    #[test]
    fn duplicate_pages_advance_the_skip_and_stop_after_the_third() {
        // The addon answered the same page: advance by the raw page size, up to three times.
        assert_eq!(next_pagination(true, 0, 20, Some(20), false, 0), (Some(20), 1));
        assert_eq!(next_pagination(true, 20, 20, Some(40), false, 1), (Some(40), 2));
        assert_eq!(next_pagination(true, 40, 20, Some(60), false, 2), (None, 3));
    }

    #[test]
    fn pagination_stops_when_the_addon_answers_nothing_or_cannot_page() {
        assert_eq!(next_pagination(false, 0, 20, Some(20), true, 0), (None, 0));
        assert_eq!(next_pagination(true, 0, 0, Some(0), true, 0), (None, 0));
        assert_eq!(next_pagination(true, 0, 20, None, true, 0), (None, 0));
    }

    #[test]
    fn full_pages_paginate_even_without_a_skip_extra() {
        let page = CatalogPage { items: Vec::new(), raw_item_count: CATALOG_PAGE_SIZE, next_skip: Some(100) };
        let state = super::page(false, 0, true, &[], 0, page);
        assert!(state.supports_pagination);
        assert_eq!(state.next_skip, Some(100));

        let small = CatalogPage { items: Vec::new(), raw_item_count: 10, next_skip: Some(10) };
        let state = super::page(false, 0, true, &[], 0, small);
        assert!(!state.supports_pagination);
        assert_eq!(state.next_skip, None);
    }

    #[test]
    fn new_items_are_counted_by_stable_key() {
        let item = |id: &str| MetaPreview {
            id: id.into(),
            content_type: "movie".into(),
            name: id.into(),
            poster: None,
            banner: None,
            logo: None,
            poster_shape: nuvio_core::catalog::PosterShape::Poster,
            description: None,
            release_info: None,
            raw_release_date: None,
            imdb_rating: None,
            genres: Vec::new(),
            landscape_poster: None,
            runtime: None,
            status: None,
        };
        let page = CatalogPage { items: vec![item("tt1"), item("tt2")], raw_item_count: 2, next_skip: Some(2) };
        let state = super::page(true, 0, false, &["movie:tt1".to_string()], 0, page);
        assert_eq!(state.next_skip, Some(2), "tt2 is new");
        assert_eq!(state.duplicate_pages, 0);
        let page = CatalogPage { items: vec![item("tt1")], raw_item_count: 1, next_skip: Some(1) };
        let state = super::page(true, 0, false, &["movie:tt1".to_string()], 0, page);
        assert_eq!(state.next_skip, Some(1), "advanced as a duplicate page");
        assert_eq!(state.duplicate_pages, 1);
    }
}
