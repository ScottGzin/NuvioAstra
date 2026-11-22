//! Live TV channels from the addons' `tv` catalogs (port of the Kotlin `LiveTvCatalogLoader`).
//!
//! Listable `tv` catalogs only (no required extra besides `skip`), up to 5 pages per catalog,
//! channel identity = `<addon instance id>|<type>|<meta id>` (same key as the original, so
//! favorites migrate as is), duplicates of one addon merged (genres united), channels without
//! genre grouped as "Outros". Only catalog requests: no stream is resolved here.

use super::addons::addon_instance_id;
use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use serde::Serialize;

pub const MAX_PAGES_PER_CATALOG: usize = 5;
pub const FALLBACK_CATEGORY: &str = "Other";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub key: String,
    pub meta_id: String,
    pub content_type: String,
    pub name: String,
    pub logo: Option<String>,
    pub genres: Vec<String>,
    pub addon_name: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChannelList {
    pub channels: Vec<Channel>,
    pub categories: Vec<(String, usize)>,
    pub failed_catalogs: usize,
}

pub fn listable_tv_catalogs(manifests: &[AddonManifest]) -> Vec<(&AddonManifest, String, bool)> {
    manifests
        .iter()
        .flat_map(|m| {
            m.catalogs
                .iter()
                .filter(|c| c.content_type.eq_ignore_ascii_case("tv"))
                .filter(|c| !c.extra.iter().any(|e| e.is_required && !e.name.eq_ignore_ascii_case("skip")))
                .map(move |c| (m, c.id.clone(), c.supports_pagination()))
        })
        .collect()
}

/// `<addon instance id>|<type>|<meta id>` → its parts (the instance id may contain `|`-free URLs
/// with `:`; the meta id is the last segment).
pub fn split_key(key: &str) -> Option<(&str, &str, &str)> {
    let mut parts = key.rsplitn(3, '|');
    let meta_id = parts.next()?;
    let content_type = parts.next()?;
    let instance = parts.next()?;
    (!meta_id.is_empty() && !instance.is_empty()).then_some((instance, content_type, meta_id))
}

pub fn merge(channels: Vec<Channel>) -> Vec<Channel> {
    let mut out: Vec<Channel> = Vec::with_capacity(channels.len());
    let mut index = std::collections::HashMap::new();
    for channel in channels {
        match index.get(&channel.key) {
            Some(&i) => {
                let existing: &mut Channel = &mut out[i];
                existing.logo = existing.logo.take().or(channel.logo);
                for g in channel.genres {
                    if !existing.genres.contains(&g) {
                        existing.genres.push(g);
                    }
                }
            }
            None => {
                index.insert(channel.key.clone(), out.len());
                out.push(channel);
            }
        }
    }
    out
}

pub fn categories(channels: &[Channel]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for c in channels {
        let names = if c.genres.is_empty() { vec![FALLBACK_CATEGORY.to_string()] } else { c.genres.clone() };
        for n in names {
            match counts.iter_mut().find(|(k, _)| *k == n) {
                Some((_, count)) => *count += 1,
                None => counts.push((n, 1)),
            }
        }
    }
    counts.sort_by(|a, b| (a.0 == FALLBACK_CATEGORY).cmp(&(b.0 == FALLBACK_CATEGORY)).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    counts
}

pub async fn load(client: &AddonClient, manifests: &[AddonManifest]) -> ChannelList {
    let mut handles = Vec::new();
    for (manifest, catalog_id, paginated) in listable_tv_catalogs(manifests) {
        let (client, manifest) = (client.clone(), manifest.clone());
        handles.push(tauri::async_runtime::spawn(async move {
            let mut channels = Vec::new();
            let mut skip: Option<u32> = None;
            for _ in 0..MAX_PAGES_PER_CATALOG {
                let page = client.fetch_catalog_page(&manifest, "tv", &catalog_id, None, skip).await.map_err(|e| e.to_string())?;
                let before = channels.len();
                for item in page.items.into_iter().filter(|i| i.content_type.eq_ignore_ascii_case("tv") || i.content_type.is_empty()) {
                    channels.push(Channel {
                        key: format!("{}|{}|{}", addon_instance_id(&manifest), item.content_type, item.id),
                        meta_id: item.id,
                        content_type: item.content_type,
                        name: item.name.trim().to_string(),
                        logo: item.logo.or(item.poster),
                        genres: item.genres,
                        addon_name: manifest.name.clone(),
                    });
                }
                match page.next_skip {
                    Some(next) if paginated && page.raw_item_count > 0 && channels.len() > before && Some(next) != skip => skip = Some(next),
                    _ => break,
                }
            }
            Ok::<_, String>(channels)
        }));
    }
    let mut all = Vec::new();
    let mut failed = 0;
    for handle in handles {
        match handle.await {
            Ok(Ok(mut c)) => all.append(&mut c),
            _ => failed += 1,
        }
    }
    let channels = merge(all);
    let categories = categories(&channels);
    ChannelList { channels, categories, failed_catalogs: failed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(key: &str, genres: &[&str]) -> Channel {
        Channel {
            key: key.into(),
            meta_id: key.into(),
            content_type: "tv".into(),
            name: key.into(),
            logo: None,
            genres: genres.iter().map(|g| g.to_string()).collect(),
            addon_name: "A".into(),
        }
    }

    #[test]
    fn duplicates_merge_and_fallback_category_is_last() {
        let merged = merge(vec![ch("a|tv|1", &["News"]), ch("a|tv|1", &["Sports"]), ch("b|tv|1", &[]), ch("a|tv|2", &["Anime"])]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].genres, vec!["News", "Sports"]);
        let cats = categories(&merged);
        assert_eq!(cats.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec!["Anime", "News", "Sports", "Other"]);
    }

    #[test]
    fn channel_keys_split_back_into_addon_type_and_id() {
        assert_eq!(
            split_key("addon:org.iptv:https://a.test/x/manifest.json|tv|iptv:ch:1"),
            Some(("addon:org.iptv:https://a.test/x/manifest.json", "tv", "iptv:ch:1"))
        );
        assert_eq!(split_key("nope"), None);
    }
}
