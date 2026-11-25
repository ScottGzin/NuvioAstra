//! Search across addon search catalogs (port of `SearchRepository.buildSearchRequests`).
//!
//! A catalog is searched when it has a `search` extra and no other required extra; every such
//! catalog of the enabled addons is asked once, concurrently; sections without results are
//! dropped. The UI debounces typing, so one query = one round of requests.

use super::addons::media_type_label;
use nuvio_core::catalog::MetaPreview;
use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_core::url::build_catalog_url;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSection {
    pub key: String,
    pub title: String,
    pub addon_name: String,
    pub items: Vec<MetaPreview>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchRequest {
    pub key: String,
    pub title: String,
    pub addon_name: String,
    pub url: String,
}

pub fn requests(manifests: &[AddonManifest], query: &str) -> Vec<SearchRequest> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    manifests
        .iter()
        .flat_map(|m| m.catalogs.iter().filter(|c| c.supports_search()).map(move |c| (m, c)))
        .map(|(m, c)| SearchRequest {
            key: format!("{}:search:{}:{}:{}", m.id, c.content_type, c.id, query.to_lowercase()),
            title: format!("{} - {}", c.name, media_type_label(&c.content_type)),
            addon_name: m.name.clone(),
            url: build_catalog_url(&m.transport_url, &c.content_type, &c.id, None, Some(query), None),
        })
        .collect()
}

pub async fn search(client: &AddonClient, manifests: &[AddonManifest], query: &str) -> Vec<SearchSection> {
    let handles: Vec<_> = requests(manifests, query)
        .into_iter()
        .map(|r| {
            let client = client.clone();
            tauri::async_runtime::spawn(async move {
                let payload = client.get_text(&r.url, false).await.ok()?;
                let page = nuvio_core::catalog::parse_catalog_page(&payload, None, None).ok()?;
                (!page.items.is_empty()).then(|| SearchSection { key: r.key, title: r.title, addon_name: r.addon_name, items: page.items })
            })
        })
        .collect();
    let mut sections = Vec::new();
    for h in handles {
        if let Ok(Some(section)) = h.await {
            sections.push(section);
        }
    }
    sections
}

/// One section of a streamed search (`search_stream`): sent as soon as its catalog answers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPart {
    pub request_id: u64,
    /// Position of the catalog in the full request list (sections are shown in this order).
    pub order: usize,
    pub section: SearchSection,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchDone {
    pub request_id: u64,
    pub catalogs: usize,
}

/// Same requests as `search`, but every section is handed to `emit` as soon as it arrives, so the
/// fastest addons show their results without waiting for the slowest one. `done` runs at the end.
pub async fn search_streamed(
    client: &AddonClient,
    manifests: &[AddonManifest],
    query: &str,
    request_id: u64,
    emit: impl Fn(SearchPart) + Send + Sync + Clone + 'static,
) -> SearchDone {
    let reqs = requests(manifests, query);
    let catalogs = reqs.len();
    let handles: Vec<_> = reqs
        .into_iter()
        .enumerate()
        .map(|(order, r)| {
            let (client, emit) = (client.clone(), emit.clone());
            tauri::async_runtime::spawn(async move {
                let Ok(payload) = client.get_text(&r.url, false).await else { return };
                let Ok(page) = nuvio_core::catalog::parse_catalog_page(&payload, None, None) else { return };
                if !page.items.is_empty() {
                    emit(SearchPart { request_id, order, section: SearchSection { key: r.key, title: r.title, addon_name: r.addon_name, items: page.items } });
                }
            })
        })
        .collect();
    for h in handles {
        let _ = h.await;
    }
    SearchDone { request_id, catalogs }
}

// ---------- Addons asked by the search (Astra: each one can be left out) ----------

/// Stable key of one installed addon, without its URL (which can carry a configuration): the
/// manifest id plus a short FNV-1a hash of the URL, so two configurations of the same addon differ.
pub fn addon_key(manifest: &AddonManifest) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in manifest.transport_url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{}@{:08x}", manifest.id, hash as u32)
}

/// An addon that has search catalogs, for the "Pesquisar em" choice.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchAddon {
    pub key: String,
    pub name: String,
    pub logo: Option<String>,
    /// Search catalogs it has ("Filmes", "Séries"…).
    pub catalogs: Vec<String>,
    pub enabled: bool,
}

pub fn search_addons(manifests: &[AddonManifest], disabled: &[String]) -> Vec<SearchAddon> {
    manifests
        .iter()
        .filter(|m| m.catalogs.iter().any(|c| c.supports_search()))
        .map(|m| {
            let key = addon_key(m);
            let mut catalogs: Vec<String> = Vec::new();
            for c in m.catalogs.iter().filter(|c| c.supports_search()) {
                let label = media_type_label(&c.content_type);
                if !catalogs.contains(&label) {
                    catalogs.push(label);
                }
            }
            SearchAddon { enabled: !disabled.contains(&key), key, name: m.name.clone(), logo: m.logo_url.clone(), catalogs }
        })
        .collect()
}

/// The addons the search asks: all but the ones left out.
pub fn searched(manifests: &[AddonManifest], disabled: &[String]) -> Vec<AddonManifest> {
    manifests.iter().filter(|m| !disabled.contains(&addon_key(m))).cloned().collect()
}

// ---------- Discover (port of `buildDiscoverSources`) ----------

/// One catalog that can be browsed in Descobrir. `key` is the Home catalog key (`catalog_page`).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverCatalog {
    pub key: String,
    pub addon_name: String,
    pub content_type: String,
    pub type_label: String,
    pub name: String,
    pub genres: Vec<String>,
    pub genre_required: bool,
    pub supports_pagination: bool,
}

fn type_order(content_type: &str) -> u8 {
    match content_type.to_lowercase().as_str() {
        "movie" => 0,
        "series" => 1,
        "anime" => 2,
        _ => 9,
    }
}

/// Every discover catalog of the enabled addons, in addon order, grouped by type (movies, series,
/// anime first, as the original's `typeSortKey`). A catalog listed twice (same addon id) once.
pub fn discover_catalogs(manifests: &[AddonManifest]) -> Vec<DiscoverCatalog> {
    let mut out: Vec<DiscoverCatalog> = Vec::new();
    for m in manifests {
        for c in m.catalogs.iter().filter(|c| c.supports_discover()) {
            let key = format!("{}:{}:{}", m.id, c.content_type, c.id);
            if out.iter().any(|d| d.key == key) {
                continue;
            }
            let genre = c.genre_extra();
            out.push(DiscoverCatalog {
                key,
                addon_name: m.name.clone(),
                content_type: c.content_type.clone(),
                type_label: media_type_label(&c.content_type),
                name: c.name.clone(),
                genres: genre.map(|g| g.options.clone()).unwrap_or_default(),
                genre_required: genre.is_some_and(|g| g.is_required),
                supports_pagination: c.supports_pagination(),
            });
        }
    }
    out.sort_by_key(|d| (type_order(&d.content_type), if type_order(&d.content_type) == 9 { d.content_type.to_lowercase() } else { String::new() }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_core::manifest::parse_manifest;

    #[test]
    fn only_search_capable_catalogs_are_asked() {
        let m = parse_manifest(
            "https://a.test/cfg/manifest.json",
            r#"{"id":"a","name":"A","version":"1","resources":["catalog"],"types":["movie"],"catalogs":[
                {"type":"movie","id":"top","name":"Top"},
                {"type":"movie","id":"s","name":"Busca","extra":[{"name":"search","isRequired":true}]},
                {"type":"series","id":"g","name":"G","extra":[{"name":"search"},{"name":"genre","isRequired":true}]}]}"#,
        )
        .unwrap();
        let r = requests(&[m], " ação ");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].url, "https://a.test/cfg/catalog/movie/s/search=a%C3%A7%C3%A3o.json");
        assert_eq!(r[0].title, "Busca - Filmes");
        assert!(requests(&[], "x").is_empty());
    }

    fn addon(url: &str, json: &str) -> AddonManifest {
        parse_manifest(url, json).unwrap()
    }

    #[test]
    fn addons_can_be_left_out_of_the_search() {
        let cat = r#"{"id":"meta","name":"Meta","version":"1","resources":["catalog"],"types":["movie","series"],"catalogs":[
            {"type":"movie","id":"s","name":"Busca","extra":[{"name":"search"}]},
            {"type":"series","id":"s","name":"Busca","extra":[{"name":"search"}]}]}"#;
        let one = addon("https://a.test/cfg1/manifest.json", cat);
        let two = addon("https://a.test/cfg2/manifest.json", cat);
        let streams = addon("https://b.test/manifest.json", r#"{"id":"b","name":"B","version":"1","resources":["stream"],"types":["movie"],"catalogs":[]}"#);
        let all = vec![one.clone(), two.clone(), streams];
        // Two configurations of the same addon have different keys, and no URL in them.
        assert_ne!(addon_key(&one), addon_key(&two));
        assert!(addon_key(&one).starts_with("meta@") && !addon_key(&one).contains("cfg"));
        assert_eq!(addon_key(&one), addon_key(&one.clone()));

        let listed = search_addons(&all, &[addon_key(&two)]);
        assert_eq!(listed.len(), 2, "only addons with search catalogs");
        assert_eq!(listed[0].catalogs, vec!["Filmes".to_string(), "Séries".to_string()]);
        assert!(listed[0].enabled && !listed[1].enabled);

        let asked = searched(&all, &[addon_key(&two)]);
        assert_eq!(requests(&asked, "x").len(), 2);
        assert!(requests(&asked, "x").iter().all(|r| r.url.contains("cfg1")));
    }

    #[test]
    fn discover_follows_the_original_rules() {
        let m = addon(
            "https://a.test/manifest.json",
            r#"{"id":"a","name":"A","version":"1","resources":["catalog"],"types":["movie","series","tv"],"catalogs":[
                {"type":"tv","id":"ch","name":"Canais"},
                {"type":"series","id":"pop","name":"Populares","extra":[{"name":"genre","options":["Drama","Ação"]},{"name":"skip"}]},
                {"type":"movie","id":"top","name":"Top"},
                {"type":"movie","id":"g","name":"Por gênero","extra":[{"name":"genre","isRequired":true,"options":["Drama"]}]},
                {"type":"movie","id":"x","name":"Sem opções","extra":[{"name":"genre","isRequired":true}]},
                {"type":"movie","id":"s","name":"Busca","extra":[{"name":"search","isRequired":true}]},
                {"type":"movie","id":"o","name":"Outro extra","extra":[{"name":"lastVideosIds","isRequired":true}]}]}"#,
        );
        let d = discover_catalogs(&[m.clone(), m]);
        let keys: Vec<_> = d.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, vec!["a:movie:top", "a:movie:g", "a:series:pop", "a:tv:ch"]);
        assert!(d[1].genre_required && d[1].genres == vec!["Drama".to_string()]);
        assert!(d[2].supports_pagination && !d[2].genre_required && d[2].genres.len() == 2);
        assert_eq!(d[2].type_label, "Séries");
    }
}
