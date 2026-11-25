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
}
