//! Home rows: `buildHomeCatalogDefinitions` + the user's `HomeCatalogSettings`.
//!
//! Definitions: catalogs of enabled addons (addon order) with no required extra, key
//! `<manifestId>:<type>:<catalogId>`, first occurrence wins. Settings: hidden rows dropped, stored
//! order applied (rows without a stored preference keep definition order after the stored ones),
//! custom titles used, otherwise "<name> - <type label>" or just the name when the user turned
//! the catalog type off. Items are not loaded here: the UI asks for each row's page lazily.

use super::addons::media_type_label;
use super::collections::CollectionView;
use nuvio_core::manifest::AddonManifest;
use nuvio_store::HomeCatalogSettings;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HomeRow {
    pub key: String,
    pub title: String,
    pub addon_name: String,
    pub content_type: String,
    pub catalog_id: String,
    pub supports_pagination: bool,
    /// Set for a collection row (key `collection_<id>`): its folders instead of a catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionView>,
}

/// Local date `YYYY-MM-DD` (`CurrentDateProvider.todayIsoDate`).
#[cfg(windows)]
pub fn today_iso() -> String {
    // SAFETY: plain Win32 call without arguments.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

#[cfg(not(windows))]
pub fn today_iso() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d").to_string()
}

/// `YYYY-MM-DD` at the start of an ISO date/time, if any.
fn iso_date(value: &str) -> Option<&str> {
    let v = value.trim().get(..10)?;
    let b = v.as_bytes();
    (b[4] == b'-' && b[7] == b'-' && v.chars().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())).then_some(v)
}

/// `MetaPreview.isUnreleased`: release date after today (from `released`, then `releaseInfo`),
/// else a 4-digit year in `releaseInfo` after the current year.
pub fn is_unreleased(item: &nuvio_core::catalog::MetaPreview, today: &str) -> bool {
    if let Some(date) = item.raw_release_date.as_deref().and_then(iso_date) {
        return date > today;
    }
    let Some(info) = item.release_info.as_deref() else { return false };
    if let Some(date) = iso_date(info) {
        return date > today;
    }
    let year = info
        .split(|c: char| !c.is_ascii_digit())
        .find(|w| w.len() == 4 && (w.starts_with("19") || w.starts_with("20")))
        .and_then(|w| w.parse::<i32>().ok());
    match (year, today.get(..4).and_then(|y| y.parse::<i32>().ok())) {
        (Some(y), Some(current)) => y > current,
        _ => false,
    }
}

/// One catalog that can appear on Home, with the profile's preference (Settings → Início).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogChoice {
    pub key: String,
    /// Title the row gets without a custom title (`<name> - <type>` when the type is shown).
    pub default_title: String,
    pub addon_name: String,
    pub content_type: String,
    pub enabled: bool,
    pub custom_title: String,
}

/// Every Home-eligible catalog in the profile's order (saved order first, new catalogs after).
pub fn catalog_choices(manifests: &[AddonManifest], settings: &HomeCatalogSettings, collections: &[CollectionView]) -> Vec<CatalogChoice> {
    let all_visible = HomeCatalogSettings { items: Vec::new(), ..settings.clone() };
    let mut choices: Vec<(i64, usize, CatalogChoice)> = home_rows(manifests, Some(&all_visible), collections)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let pref = settings.items.iter().find(|p| p.key == row.key);
            (
                pref.map_or(i64::MAX, |p| p.order),
                index,
                CatalogChoice {
                    key: row.key,
                    default_title: row.title,
                    addon_name: row.addon_name,
                    content_type: row.content_type,
                    enabled: pref.is_none_or(|p| p.enabled),
                    custom_title: pref.map(|p| p.custom_title.clone()).unwrap_or_default(),
                },
            )
        })
        .collect();
    choices.sort_by_key(|(order, index, _)| (*order, *index));
    choices.into_iter().map(|(_, _, c)| c).collect()
}

/// `HERO_SOURCE_SELECTION_LIMIT` of `HomeCatalogSettingsRepository`.
pub const HERO_SOURCE_LIMIT: usize = 2;

/// Catalogs whose items feed the hero, at most two, hidden rows included, collections never: the
/// chosen ones that exist. Nothing chosen (or none of them installed any more): `normalizePreferences`,
/// so the first in the saved order with `heroSourceEnabled` (on unless turned off).
pub fn hero_sources(manifests: &[AddonManifest], settings: &HomeCatalogSettings) -> Vec<String> {
    let catalogs = catalog_choices(manifests, settings, &[]);
    if let Some(chosen) = &settings.hero_catalogs {
        let installed: Vec<String> =
            chosen.iter().filter(|key| catalogs.iter().any(|c| c.key == **key)).take(HERO_SOURCE_LIMIT).cloned().collect();
        if chosen.is_empty() || !installed.is_empty() {
            return installed;
        }
    }
    catalogs
        .into_iter()
        .filter(|c| settings.items.iter().find(|p| p.key == c.key).is_none_or(|p| p.hero_source_enabled))
        .take(HERO_SOURCE_LIMIT)
        .map(|c| c.key)
        .collect()
}

/// Catalog rows then collection rows (`buildCollectionDefinitions`), ordered by the saved
/// preferences; collections marked "fixar no topo" come first (`enforcePinnedCollectionsAtTop`).
pub fn home_rows(manifests: &[AddonManifest], settings: Option<&HomeCatalogSettings>, collections: &[CollectionView]) -> Vec<HomeRow> {
    let default_settings = HomeCatalogSettings::default();
    let settings = settings.unwrap_or(&default_settings);
    let mut seen = std::collections::HashSet::new();
    let definitions: Vec<(usize, HomeRow)> = manifests
        .iter()
        .flat_map(|manifest| {
            manifest
                .catalogs
                .iter()
                .filter(|c| !c.extra.iter().any(|e| e.is_required))
                .map(move |catalog| (manifest, catalog))
        })
        .filter_map(|(manifest, catalog)| {
            let key = format!("{}:{}:{}", manifest.id, catalog.content_type, catalog.id);
            if !seen.insert(key.clone()) {
                return None;
            }
            let title = if settings.show_catalog_type {
                format!("{} - {}", catalog.name, media_type_label(&catalog.content_type))
            } else {
                catalog.name.clone()
            };
            Some(HomeRow {
                key,
                title,
                addon_name: manifest.name.clone(),
                content_type: catalog.content_type.clone(),
                catalog_id: catalog.id.clone(),
                supports_pagination: catalog.supports_pagination(),
                collection: None,
            })
        })
        .chain(collections.iter().map(|c| HomeRow {
            key: format!("collection_{}", c.id),
            title: c.title.clone(),
            addon_name: "Coleção".into(),
            content_type: "collection".into(),
            catalog_id: String::new(),
            supports_pagination: false,
            collection: Some(c.clone()),
        }))
        .enumerate()
        .collect();

    let mut rows: Vec<(i64, usize, HomeRow)> = definitions
        .into_iter()
        .filter_map(|(index, mut row)| {
            match settings.items.iter().find(|p| p.key == row.key) {
                Some(pref) if !pref.enabled => None,
                Some(pref) => {
                    if !pref.custom_title.trim().is_empty() {
                        row.title = pref.custom_title.clone();
                    }
                    Some((pref.order, index, row))
                }
                None => Some((i64::MAX, index, row)),
            }
        })
        .collect();
    rows.sort_by_key(|(order, index, row)| (!row.collection.as_ref().is_some_and(|c| c.pin_to_top), *order, *index));
    rows.into_iter().map(|(_, _, row)| row).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_core::manifest::parse_manifest;
    use nuvio_store::HomeCatalogPreference;

    fn manifest() -> AddonManifest {
        parse_manifest(
            "https://a.test/manifest.json",
            r#"{"id":"a","name":"Addon A","version":"1","resources":["catalog"],"types":["movie"],
               "catalogs":[{"type":"movie","id":"top","name":"Top"},{"type":"series","id":"new","name":"New"},
                           {"type":"movie","id":"genre","name":"By genre","extra":[{"name":"genre","isRequired":true}]},
                           {"type":"tv","id":"ch","name":"Channels","extra":[{"name":"skip"}]}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn default_rows_skip_required_extras_and_use_type_labels() {
        let rows = home_rows(&[manifest()], None, &[]);
        assert_eq!(rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), vec!["Top - Filmes", "New - Séries", "Channels - TV"]);
        assert!(rows[2].supports_pagination);
    }

    #[test]
    fn stored_settings_order_hide_and_rename() {
        let settings = HomeCatalogSettings {
            show_catalog_type: false,
            items: vec![
                HomeCatalogPreference { key: "a:tv:ch".into(), order: 0, ..Default::default() },
                HomeCatalogPreference { key: "a:series:new".into(), enabled: false, ..Default::default() },
                HomeCatalogPreference { key: "a:movie:top".into(), order: 1, custom_title: "Mais vistos".into(), ..Default::default() },
            ],
            ..Default::default()
        };
        let rows = home_rows(&[manifest()], Some(&settings), &[]);
        assert_eq!(rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), vec!["Channels", "Mais vistos"]);
    }

    #[test]
    fn collections_follow_the_saved_order_and_pinned_ones_lead() {
        let stored = serde_json::json!([
            {"id":"c1","title":"Estúdios","folders":[{"id":"f","title":"F","sources":[{"addonId":"a","type":"movie","catalogId":"top"}]}]},
            {"id":"c2","title":"Fixa","pinToTop":true,"folders":[{"id":"g","title":"G"}]},
            {"id":"c3","title":"Oculta","folders":[{"id":"h","title":"H"}]}
        ]);
        let collections = crate::services::collections::views(&stored, &[manifest()], true, false);
        let settings = HomeCatalogSettings {
            items: vec![
                HomeCatalogPreference { key: "collection_c1".into(), order: 0, ..Default::default() },
                HomeCatalogPreference { key: "a:movie:top".into(), order: 1, hero_source_enabled: false, ..Default::default() },
                HomeCatalogPreference { key: "collection_c3".into(), enabled: false, ..Default::default() },
            ],
            ..Default::default()
        };
        let rows = home_rows(&[manifest()], Some(&settings), &collections);
        let keys: Vec<_> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["collection_c2", "collection_c1", "a:movie:top", "a:series:new", "a:tv:ch"]);
        assert_eq!(hero_sources(&[manifest()], &settings), vec!["a:series:new", "a:tv:ch"], "turned off one skipped, two at most");
        let hidden = HomeCatalogSettings {
            items: vec![HomeCatalogPreference { key: "a:movie:top".into(), enabled: false, ..Default::default() }],
            ..Default::default()
        };
        assert_eq!(hero_sources(&[manifest()], &hidden), vec!["a:movie:top", "a:series:new"], "a hidden row still feeds the hero");
        let chosen = |keys: &[&str]| HomeCatalogSettings { hero_catalogs: Some(keys.iter().map(|k| k.to_string()).collect()), ..settings.clone() };
        assert_eq!(hero_sources(&[manifest()], &chosen(&["gone:movie:x", "a:tv:ch", "a:movie:top"])), vec!["a:tv:ch", "a:movie:top"], "the choice wins over flags and order");
        assert!(hero_sources(&[manifest()], &chosen(&[])).is_empty(), "none chosen");
        assert_eq!(hero_sources(&[manifest()], &chosen(&["gone:movie:x"])), vec!["a:series:new", "a:tv:ch"], "chosen addon removed");
        assert_eq!(rows[1].collection.as_ref().unwrap().folders[0].sources[0].key, "a:movie:top");
    }

    #[test]
    fn unreleased_items_follow_the_original_rule() {
        let item = |released: Option<&str>, info: Option<&str>| nuvio_core::catalog::parse_catalog_page(
            &serde_json::json!({"metas":[{"id":"x","type":"movie","name":"X","released":released,"releaseInfo":info}]}).to_string(),
            None,
            None,
        )
        .unwrap()
        .items
        .remove(0);
        let today = "2026-11-19";
        assert!(is_unreleased(&item(Some("2026-12-01T00:00:00.000Z"), None), today));
        assert!(!is_unreleased(&item(Some("2026-11-19T00:00:00.000Z"), Some("2027")), today));
        assert!(is_unreleased(&item(None, Some("2027")), today));
        assert!(!is_unreleased(&item(None, Some("2019-2026")), today));
        assert!(!is_unreleased(&item(None, None), today));
    }

}
