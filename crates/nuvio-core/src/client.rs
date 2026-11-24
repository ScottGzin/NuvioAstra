//! HTTP client for addon resources.
//!
//! Behavior kept from the Kotlin desktop client (`AddonPlatform.desktop.kt` / `CatalogData.kt`):
//! `Accept: application/json`, 60 s connect/read timeouts, redirects followed, and concurrent
//! requests for the same URL share a single network call (in-flight de-duplication).

use crate::catalog::{parse_catalog_page, CatalogPage};
use crate::manifest::{parse_manifest, AddonManifest};
use crate::url::build_catalog_url;
use crate::CoreError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;

type Shared = Arc<OnceCell<Result<Arc<str>, Arc<CoreError>>>>;

#[derive(Clone)]
pub struct AddonClient {
    http: reqwest::Client,
    inflight: Arc<Mutex<HashMap<String, Shared>>>,
}

impl AddonClient {
    pub fn new() -> Result<Self, CoreError> {
        let http = crate::net::http_client()?;
        Ok(Self { http, inflight: Arc::default() })
    }

    /// GETs `url` as text; simultaneous calls for the same URL share one request.
    pub async fn get_text(&self, url: &str, force_refresh: bool) -> Result<Arc<str>, Arc<CoreError>> {
        let key = format!("{force_refresh}|{url}");
        let cell = {
            let mut map = self.inflight.lock().expect("inflight lock poisoned");
            map.entry(key.clone()).or_default().clone()
        };
        let result = cell
            .get_or_init(|| async {
                let mut request = self.http.get(url).header("Accept", "application/json");
                if force_refresh {
                    request = request.header("Cache-Control", "no-cache");
                }
                let response = request.send().await.map_err(|e| Arc::new(CoreError::from(e)))?;
                let status = response.status();
                if !status.is_success() {
                    return Err(Arc::new(CoreError::HttpStatus { status: status.as_u16() }));
                }
                let body = response.text().await.map_err(|e| Arc::new(CoreError::from(e)))?;
                Ok(Arc::<str>::from(body))
            })
            .await
            .clone();
        // The shared result is only for requests that overlapped; later calls fetch again.
        let mut map = self.inflight.lock().expect("inflight lock poisoned");
        if map.get(&key).is_some_and(|current| Arc::ptr_eq(current, &cell)) {
            map.remove(&key);
        }
        result
    }

    /// GETs a small file (a subtitle); refuses one larger than `max` bytes.
    pub async fn get_bytes(&self, url: &str, max: usize) -> Result<Vec<u8>, CoreError> {
        let mut response = self.http.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(CoreError::HttpStatus { status: status.as_u16() });
        }
        let mut out = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if out.len() + chunk.len() > max {
                return Err(CoreError::InvalidPayload("file too large".into()));
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    pub async fn fetch_manifest(&self, manifest_url: &str) -> Result<AddonManifest, Arc<CoreError>> {
        let payload = self.get_text(manifest_url, false).await?;
        parse_manifest(manifest_url, &payload).map_err(Arc::new)
    }

    pub async fn fetch_catalog_page(
        &self,
        manifest: &AddonManifest,
        content_type: &str,
        catalog_id: &str,
        genre: Option<&str>,
        skip: Option<u32>,
    ) -> Result<CatalogPage, Arc<CoreError>> {
        let url = build_catalog_url(&manifest.transport_url, content_type, catalog_id, genre, None, skip);
        let payload = self.get_text(&url, false).await?;
        parse_catalog_page(&payload, skip, None).map_err(Arc::new)
    }
}
