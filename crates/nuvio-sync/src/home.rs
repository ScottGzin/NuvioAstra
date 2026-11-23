//! Home layout kept by the account (`HomeCatalogSettingsSyncService`, platform
//! `home_catalog_shared`): position, visibility and title of each catalog and collection row, and
//! the two switches next to them. The hero choices are local, as in the original.
//!
//! Three-way merge with `base`, the layout both sides last agreed on: what changed here wins, the
//! rest follows the account. A push is the account's own object with those values patched in, so
//! fields and rows Astra does not know are kept. Media-server rows (`server:`) are never sent.

use nuvio_store::{HomeCatalogPreference, HomeCatalogSettings};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub(crate) const PLATFORM: &str = "home_catalog_shared";
const SERVER_KEY: &str = "server:";

/// `SyncCatalogItem.preferenceKey()`.
fn key_of(item: &Value) -> Option<String> {
    let text = |name: &str| item.get(name).and_then(Value::as_str).unwrap_or_default();
    if !text("key").trim().is_empty() {
        return Some(text("key").to_string());
    }
    if item.get("is_collection").and_then(Value::as_bool).unwrap_or(false) {
        return (!text("collection_id").is_empty()).then(|| format!("collection_{}", text("collection_id")));
    }
    let (addon, kind, catalog) = (text("addon_id"), text("type"), text("catalog_id"));
    (!addon.is_empty() && !kind.is_empty() && !catalog.is_empty()).then(|| format!("{addon}:{kind}:{catalog}"))
}

fn by_key(settings: &HomeCatalogSettings) -> BTreeMap<&str, &HomeCatalogPreference> {
    settings.items.iter().map(|p| (p.key.as_str(), p)).collect()
}

/// `local` with the account's values wherever nothing changed here since `base`. Without a base
/// (first sync of the layout) the account's values win and rows only here stay.
pub(crate) fn merge(local: &HomeCatalogSettings, base: Option<&HomeCatalogSettings>, remote: &Value) -> HomeCatalogSettings {
    let mut merged = local.clone();
    let switch = |name: &str, current: bool, agreed: Option<bool>| match remote.get(name).and_then(Value::as_bool) {
        Some(theirs) if agreed.is_none_or(|b| b == current) => theirs,
        _ => current,
    };
    merged.show_catalog_type = switch("show_catalog_type", local.show_catalog_type, base.map(|b| b.show_catalog_type));
    merged.hide_unreleased_content = switch("hide_unreleased_content", local.hide_unreleased_content, base.map(|b| b.hide_unreleased_content));

    let agreed = base.map(by_key);
    // The order is one value: moving a row renumbers the others, as in the original.
    let moved = agreed.as_ref().is_some_and(|b| local.items.iter().any(|p| b.get(p.key.as_str()).map(|old| old.order) != Some(p.order)));
    let mine = by_key(local);
    let mut rows: BTreeMap<String, HomeCatalogPreference> = BTreeMap::new();
    for item in remote.get("items").and_then(Value::as_array).into_iter().flatten() {
        let Some(key) = key_of(item) else { continue };
        let mut row = HomeCatalogPreference {
            key: key.clone(),
            custom_title: item.get("custom_title").and_then(Value::as_str).unwrap_or_default().to_string(),
            enabled: item.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            hero_source_enabled: true,
            order: item.get("order").and_then(Value::as_i64).unwrap_or(0),
        };
        if let Some(here) = mine.get(key.as_str()) {
            row.hero_source_enabled = here.hero_source_enabled;
            if let Some(agreed) = &agreed {
                let old = agreed.get(key.as_str());
                if moved {
                    row.order = here.order;
                }
                if here.enabled != old.is_none_or(|o| o.enabled) {
                    row.enabled = here.enabled;
                }
                if here.custom_title != old.map(|o| o.custom_title.as_str()).unwrap_or_default() {
                    row.custom_title = here.custom_title.clone();
                }
            }
        }
        // A repeated key keeps its last entry (`associate`).
        rows.insert(key, row);
    }
    if rows.is_empty() {
        return merged;
    }
    let mut items: Vec<HomeCatalogPreference> = local.items.iter().filter(|p| !rows.contains_key(&p.key)).cloned().collect();
    items.extend(rows.into_values());
    items.sort_by_key(|p| p.order);
    merged.items = items;
    merged
}

/// Whether `local` has something the account does not (hero choices and server rows aside).
pub(crate) fn differs(local: &HomeCatalogSettings, base: &HomeCatalogSettings) -> bool {
    if local.show_catalog_type != base.show_catalog_type || local.hide_unreleased_content != base.hide_unreleased_content {
        return true;
    }
    let agreed = by_key(base);
    // A row missing here is never a change: it is not deleted on the account.
    local.items.iter().filter(|p| !p.key.starts_with(SERVER_KEY)).any(|p| {
        agreed
            .get(p.key.as_str())
            .is_none_or(|b| (b.order, b.enabled, &b.custom_title) != (p.order, p.enabled, &p.custom_title))
    })
}

/// `exportToSyncPayload` written over the account's object (`mergeHomeCatalogSettingsJson`).
pub(crate) fn payload(remote: Option<&Value>, merged: &HomeCatalogSettings) -> Value {
    let mut object: Map<String, Value> = remote.and_then(Value::as_object).cloned().unwrap_or_default();
    let mine = by_key(merged);
    let mut items: Vec<Value> = object.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut present = std::collections::BTreeSet::new();
    for item in &mut items {
        let Some(key) = key_of(item) else { continue };
        if let Some(row) = mine.get(key.as_str()).filter(|_| !key.starts_with(SERVER_KEY)) {
            item["order"] = json!(row.order);
            item["enabled"] = json!(row.enabled);
            item["custom_title"] = json!(row.custom_title);
        }
        present.insert(key);
    }
    for row in merged.items.iter().filter(|p| !p.key.starts_with(SERVER_KEY) && !present.contains(&p.key)) {
        let collection = row.key.strip_prefix("collection_");
        let mut parts = if collection.is_some() { "" } else { row.key.as_str() }.splitn(3, ':');
        let mut part = || parts.next().unwrap_or_default();
        items.push(json!({
            "addon_id": part(), "type": part(), "catalog_id": part(),
            "enabled": row.enabled, "order": row.order, "custom_title": row.custom_title,
            "is_collection": collection.is_some(), "collection_id": collection.unwrap_or_default(), "key": row.key,
        }));
    }
    items.sort_by_key(|item| item.get("order").and_then(Value::as_i64).unwrap_or(0));
    object.insert("items".into(), Value::Array(items));
    object.insert("show_catalog_type".into(), json!(merged.show_catalog_type));
    object.insert("hide_unreleased_content".into(), json!(merged.hide_unreleased_content));
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pref(key: &str, order: i64) -> HomeCatalogPreference {
        HomeCatalogPreference { key: key.into(), order, ..Default::default() }
    }

    fn keys(settings: &HomeCatalogSettings) -> Vec<&str> {
        settings.items.iter().map(|p| p.key.as_str()).collect()
    }

    #[test]
    fn first_sync_takes_the_account_layout() {
        let mut hero_off = pref("a:movie:top", 0);
        hero_off.hero_source_enabled = false;
        let local = HomeCatalogSettings { items: vec![hero_off, pref("a:series:new", 1), pref("b:movie:only-here", 50), pref("collection_c1", 90)], ..Default::default() };
        let remote = json!({ "show_catalog_type": false, "items": [
            { "addon_id": "a", "type": "movie", "catalog_id": "top", "order": 0, "custom_title": "Em alta" },
            { "is_collection": true, "collection_id": "c1", "order": 1 },
            { "key": "a:series:new", "order": 2, "enabled": false },
            { "type": "movie", "order": 3 }
        ]});
        let merged = merge(&local, None, &remote);
        assert_eq!(keys(&merged), ["a:movie:top", "collection_c1", "a:series:new", "b:movie:only-here"]);
        assert_eq!((merged.items[0].custom_title.as_str(), merged.items[0].hero_source_enabled), ("Em alta", false), "hero choice stays local");
        assert!(!merged.items[2].enabled);
        assert!(!merged.show_catalog_type && !merged.hide_unreleased_content);
        assert_eq!(merge(&local, None, &json!({ "items": [] })), local, "empty list keeps the local order");
    }

    #[test]
    fn changes_of_both_sides_are_combined_and_nothing_unknown_is_lost() {
        let base = HomeCatalogSettings { items: vec![pref("a:movie:top", 0), pref("a:series:new", 1)], ..Default::default() };
        let mut local = base.clone();
        local.items[0].order = 1; // moved here
        local.items[1].order = 0;
        let remote = json!({ "future_field": 7, "items": [
            { "key": "a:movie:top", "order": 0, "custom_title": "Lá", "extra": "x" },
            { "key": "a:series:new", "order": 1 },
            { "key": "server:plex:movies", "order": 4 }
        ]});
        let merged = merge(&local, Some(&base), &remote);
        assert_eq!(keys(&merged), ["a:series:new", "a:movie:top", "server:plex:movies"]);
        assert_eq!(merged.items[1].custom_title, "Lá", "renamed on the account, moved here");
        assert!(differs(&merged, &merge(&base, Some(&base), &remote)));

        let sent = payload(Some(&remote), &merged);
        assert_eq!(sent["future_field"], 7);
        let items = sent["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert!(items.iter().any(|i| i["extra"] == "x" && i["order"] == 1 && i["custom_title"] == "Lá"));
        assert!(items.iter().any(|i| i["key"] == "server:plex:movies" && i["order"] == 4 && i.get("enabled").is_none()), "untouched");

        let fresh = payload(None, &HomeCatalogSettings { items: vec![pref("collection_c1", 0), pref("a:movie:x:y", 1)], ..Default::default() });
        assert_eq!((fresh["items"][0]["collection_id"].as_str(), fresh["items"][0]["addon_id"].as_str()), (Some("c1"), Some("")));
        assert_eq!(fresh["items"][1]["catalog_id"], "x:y");
    }
}
