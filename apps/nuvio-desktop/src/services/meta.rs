//! Details of one title: the enabled addons whose manifest serves `meta` for the type and id are
//! asked in order until one answers (as `MetaDetailsRepository.findMetaManifests`).

use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_core::meta::{parse_meta, select_trailer, MetaDetails, MetaTrailer};
use nuvio_core::url::build_resource_url;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    pub meta: MetaDetails,
    pub seasons: Vec<i64>,
    pub addon_name: String,
    /// The YouTube trailer "Assistir trailer" plays (`selectHeroTrailer`), if the title has one.
    pub trailer: Option<MetaTrailer>,
    /// MDBList ratings (Settings → Services → MDBList Ratings); empty when off.
    pub ratings: Vec<super::mdblist::Rating>,
    /// Watched marks of this title (movie, or its episodes) for the manual toggle.
    pub watched_keys: Vec<String>,
}

pub async fn details(client: &AddonClient, manifests: &[AddonManifest], content_type: &str, id: &str) -> Result<Details, String> {
    let mut last_error = "no installed addon provides details for this title".to_string();
    for manifest in manifests.iter().filter(|m| m.supports_resource("meta", content_type, id)) {
        let url = build_resource_url(&manifest.transport_url, "meta", content_type, id, None);
        match client.get_text(&url, false).await {
            Ok(payload) => match parse_meta(&payload) {
                Ok(meta) => {
                    let seasons = meta.seasons();
                    let trailer = select_trailer(&meta.trailers).cloned();
                    return Ok(Details { meta, seasons, addon_name: manifest.name.clone(), trailer, ratings: Vec::new(), watched_keys: Vec::new() });
                }
                Err(e) => last_error = e.to_string(),
            },
            Err(e) => last_error = e.to_string(),
        }
    }
    Err(last_error)
}
