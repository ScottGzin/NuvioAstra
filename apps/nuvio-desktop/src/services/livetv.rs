//! Live TV channels from the addons' `tv` catalogs (port of the Kotlin `LiveTvCatalogLoader`).
//!
//! Listable `tv` catalogs only (no required extra besides `skip`), up to 5 pages per catalog,
//! channel identity = `<addon instance id>|<type>|<meta id>` (same key as the original, so
//! favorites migrate as is), duplicates of one addon merged (genres united), channels without
//! genre grouped as "Outros". Only catalog requests: no stream is resolved here.

use super::addons::addon_instance_id;
use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_sync::crypto::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

pub const MAX_PAGES_PER_CATALOG: usize = 5;
pub const FALLBACK_CATEGORY: &str = "Outros";
pub const M3U_PREFIX: &str = "m3u|";

/// In-memory direct URLs of M3U channels (imported for a session, never written to disk).
static M3U_URLS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub fn remember_m3u(channels: &[Channel]) {
    let mut map = M3U_URLS.lock().expect("m3u lock");
    let urls = map.get_or_insert_with(HashMap::new);
    for channel in channels {
        if let Some(url) = &channel.url {
            urls.insert(channel.key.clone(), url.clone());
        }
    }
}

pub fn resolve_m3u(key: &str) -> Option<String> {
    M3U_URLS.lock().expect("m3u lock").as_ref()?.get(key).cloned()
}

/// Seven public Arabic HLS feeds found in the user's AelatnaTV Tizen app. Used as a temporary
/// source for testing Live TV / matches when no channel addon is installed.
pub fn fallback_channels() -> Vec<Channel> {
    const FEEDS: &[(&str, &str, &str, &str)] = &[
        ("الجزيرة الإخبارية", "https://live-hls-web-aje.getaj.net/AJE/03.m3u8", "أخبار", "https://upload.wikimedia.org/wikipedia/en/thumb/f/f2/Aljazeera_eng.svg/512px-Aljazeera_eng.svg.png"),
        ("العربية الإخبارية", "https://mbc1-enc.akamaized.net/out/v1/a292671542f74dda4d693521b343888/index.m3u8", "أخبار", "https://upload.wikimedia.org/wikipedia/commons/thumb/6/6f/Al_Arabiya_Logo.svg/512px-Al_Arabiya_Logo.svg.png"),
        ("سكاي نيوز عربية", "https://stream.skynewsarabia.com/hls/sna.m3u8", "أخبار", "https://upload.wikimedia.org/wikipedia/commons/thumb/d/d4/Sky_News_Arabia_Logo.svg/512px-Sky_News_Arabia_Logo.svg.png"),
        ("TRT عربي", "https://tv-trtarabic.medya.trt.com.tr/master.m3u8", "أخبار", "https://upload.wikimedia.org/wikipedia/commons/thumb/5/52/TRT_Arabi_logo.svg/512px-TRT_Arabi_logo.svg.png"),
        ("فرانس 24 عربي", "https://f24hls-i.akamaihd.net/hls/live/221193/F24_AR_HI_HLS/master_5000.m3u8", "أخبار", "https://upload.wikimedia.org/wikipedia/commons/thumb/b/b3/France_24_logo.svg/512px-France_24_logo.svg.png"),
        ("القرآن الكريم", "https://win.holol.com/live/quran/playlist.m3u8", "إسلامية", ""),
        ("السنة النبوية", "https://win.holol.com/live/sunnah/playlist.m3u8", "إسلامية", ""),
        // Sports feeds found in the BabElMoshahdTV project; verified reachable from here.
        ("Sports One", "https://5c7b683162943.streamlock.net/live/ngrp:sportsone_all/playlist.m3u8", "رياضة", ""),
        ("Bahrain Sports 2", "https://5c7b683162943.streamlock.net/live/ngrp:bahrainsportstwo_all/playlist.m3u8", "رياضة", ""),
    ];
    FEEDS
        .iter()
        .map(|(name, url, group, logo)| Channel {
            key: format!("{M3U_PREFIX}{group}|{}", &sha256_hex(url)[..10]),
            meta_id: (*url).to_string(),
            content_type: "tv".into(),
            name: (*name).to_string(),
            logo: if logo.is_empty() { None } else { Some((*logo).to_string()) },
            genres: vec![(*group).to_string()],
            addon_name: "M3U".into(),
            url: Some((*url).to_string()),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub key: String,
    pub meta_id: String,
    pub content_type: String,
    pub name: String,
    pub logo: Option<String>,
    pub genres: Vec<String>,
    pub addon_name: String,
    /// Direct stream URL, only for channels imported from an M3U playlist (never sent to the UI).
    #[serde(default, skip_serializing)]
    pub url: Option<String>,
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

/// Parses a public M3U playlist into the same `Channel` shape addons produce.
/// Keys are stable for a given URL, so favorites and playback keys keep working during a session.
pub fn parse_m3u(text: &str, limit: usize) -> Vec<Channel> {
    let attr = |line: &str, name: &str| -> Option<String> {
        let needle = format!("{name}=\"");
        let at = line.find(&needle)? + needle.len();
        let rest = &line[at..];
        let end = rest.find('"')?;
        let value = &rest[..end];
        (!value.trim().is_empty()).then(|| value.trim().to_string())
    };
    let mut out: Vec<Channel> = Vec::new();
    let mut pending: Option<(String, Option<String>, Vec<String>)> = None;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.starts_with("#EXTINF") {
            let comma = line.find(',').unwrap_or(line.len());
            let name = line[comma + 1..].trim().to_string();
            if name.is_empty() {
                pending = None;
                continue;
            }
            let group = attr(line, "group-title").unwrap_or_else(|| FALLBACK_CATEGORY.to_string());
            let logo = attr(line, "tvg-logo").or_else(|| attr(line, "logo"));
            pending = Some((name, logo, vec![group]));
        } else if line.starts_with('#') || line.is_empty() {
            continue;
        } else if let Some((name, logo, genres)) = pending.take() {
            if !line.starts_with("http://") && !line.starts_with("https://") {
                continue;
            }
            let key = format!("{M3U_PREFIX}{}|{}", genres[0], &sha256_hex(line)[..10]);
            out.push(Channel {
                key,
                meta_id: line.to_string(),
                content_type: "tv".into(),
                name,
                logo,
                genres,
                addon_name: "M3U".into(),
                url: Some(line.to_string()),
            });
            if out.len() >= limit {
                break;
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
                        url: None,
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
            url: None,
        }
    }

    #[test]
    fn duplicates_merge_and_fallback_category_is_last() {
        let merged = merge(vec![ch("a|tv|1", &["News"]), ch("a|tv|1", &["Sports"]), ch("b|tv|1", &[]), ch("a|tv|2", &["Anime"])]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].genres, vec!["News", "Sports"]);
        let cats = categories(&merged);
        assert_eq!(cats.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec!["Anime", "News", "Sports", "Outros"]);
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
