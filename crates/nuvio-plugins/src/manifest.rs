//! Repository manifest parsing and URL resolution, matching the original app's
//! `PluginManifestParser` and `PluginRepository` rules.

use crate::error::PluginError;
use crate::model::{PluginManifest, PluginRepositoryItem, PluginScraper};
use url::Url;

/// Parses a `manifest.json` and applies the original's three validity checks.
pub fn parse_manifest(payload: &str) -> Result<PluginManifest, PluginError> {
    let manifest: PluginManifest = serde_json::from_str(payload)?;
    if manifest.name.trim().is_empty() {
        return Err(PluginError::EmptyName);
    }
    if manifest.version.trim().is_empty() {
        return Err(PluginError::EmptyVersion);
    }
    if manifest.scrapers.is_empty() {
        return Err(PluginError::NoScrapers);
    }
    for scraper in &manifest.scrapers {
        if scraper.id.trim().is_empty() || scraper.filename.trim().is_empty() {
            return Err(PluginError::IncompleteScraper);
        }
    }
    Ok(manifest)
}

/// Resolves a scraper's `filename` against the manifest URL. Absolute http(s) URLs are kept.
/// `javascript:`, `file:` and other schemes are refused — plugin code is only fetched over HTTP.
pub fn resolve_code_url(manifest_url: &str, filename: &str) -> Result<String, PluginError> {
    let base = Url::parse(manifest_url.trim())?;
    let joined = base.join(filename.trim())?;
    match joined.scheme() {
        "http" | "https" => Ok(joined.to_string()),
        _ => Err(PluginError::UnsupportedUrl),
    }
}

pub fn repository_item(manifest_url: String, manifest: &PluginManifest, now_epoch_ms: i64) -> PluginRepositoryItem {
    PluginRepositoryItem {
        manifest_url,
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        version: Some(manifest.version.clone()),
        scraper_count: manifest.scrapers.len(),
        last_updated: now_epoch_ms,
    }
}

/// Builds the stored scraper record from a manifest entry. `code` is filled after the download.
pub fn scraper_from_manifest(manifest_url: &str, entry: &crate::model::PluginManifestScraper) -> Result<PluginScraper, PluginError> {
    Ok(PluginScraper {
        id: entry.id.trim().to_string(),
        repository_url: manifest_url.trim().to_string(),
        name: entry.name.trim().to_string(),
        description: entry.description.clone().unwrap_or_default(),
        version: entry.version.trim().to_string(),
        filename: entry.filename.trim().to_string(),
        supported_types: entry.supported_types.clone(),
        enabled: entry.enabled,
        manifest_enabled: entry.enabled,
        has_settings: entry.has_settings,
        logo: entry.logo.clone(),
        content_language: entry.content_language.clone(),
        formats: entry.formats.clone(),
        code: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST_URL: &str = "https://raw.githubusercontent.com/michat88/nuvio-providers/main/manifest.json";
    const MANIFEST: &str = include_str!("../fixtures/michat88-manifest.json");
    const KISSKH: &str = include_str!("../fixtures/kisskh.js");

    #[test]
    fn a_real_nuvio_repository_parses() {
        let manifest = parse_manifest(MANIFEST).unwrap();
        assert_eq!(manifest.name, "Michat88 Repo");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.scrapers.len(), 30);

        let first = &manifest.scrapers[0];
        assert_eq!(first.id, "kisskh");
        assert_eq!(first.filename, "providers/kisskh.js");
        assert_eq!(first.supported_types, ["movie", "tv"]);
        assert!(first.enabled);

        let item = repository_item(MANIFEST_URL.into(), &manifest, 1_700_000_000_000);
        assert_eq!(item.scraper_count, 30);
        assert_eq!(item.last_updated, 1_700_000_000_000);

        let scraper = scraper_from_manifest(MANIFEST_URL, first).unwrap();
        assert_eq!(scraper.repository_url, MANIFEST_URL);
        assert!(scraper.supports_type("tv"));
    }

    #[test]
    fn a_real_scraper_shows_the_contract_we_must_support() {
        // This is the exact shape of the script the original runtime evaluates: CommonJS export,
        // Promise-based `getStreams(tmdbId, mediaType, seasonNum, episodeNum)`, global `fetch`,
        // `require('cheerio-without-node-native')`, and a tolerant stream object.
        assert!(KISSKH.contains("function getStreams(tmdbId, mediaType, seasonNum, episodeNum)"));
        assert!(KISSKH.contains("module.exports = { getStreams }"));
        assert!(KISSKH.contains("require("));
        assert!(KISSKH.contains("fetch("));
        assert!(KISSKH.contains("headers:"));
    }

    #[test]
    fn relative_and_absolute_code_urls_are_resolved_safely() {
        assert_eq!(
            resolve_code_url(MANIFEST_URL, "providers/kisskh.js").unwrap(),
            "https://raw.githubusercontent.com/michat88/nuvio-providers/main/providers/kisskh.js"
        );
        assert_eq!(
            resolve_code_url(MANIFEST_URL, "https://cdn.example/x.js").unwrap(),
            "https://cdn.example/x.js"
        );
        assert_eq!(
            resolve_code_url("https://host.test/plugins/manifest.json", "../shared/x.js").unwrap(),
            "https://host.test/shared/x.js"
        );
        assert_eq!(resolve_code_url(MANIFEST_URL, "javascript:alert(1)").unwrap_err(), PluginError::UnsupportedUrl);
    }

    #[test]
    fn invalid_manifests_are_rejected() {
        assert_eq!(parse_manifest("not json").unwrap_err(), PluginError::Json);
        assert_eq!(parse_manifest(r#"{"name":"","version":"1","scrapers":[]}"#).unwrap_err(), PluginError::EmptyName);
        assert_eq!(parse_manifest(r#"{"name":"x","version":"","scrapers":[]}"#).unwrap_err(), PluginError::EmptyVersion);
        assert_eq!(parse_manifest(r#"{"name":"x","version":"1","scrapers":[]}"#).unwrap_err(), PluginError::NoScrapers);
        assert_eq!(parse_manifest(r#"{"name":"x","version":"1","scrapers":[{"id":"a"}]}"#).unwrap_err(), PluginError::IncompleteScraper);
        // Unknown fields must be ignored, as in the original's `Json { ignoreUnknownKeys = true }`.
        assert!(parse_manifest(r#"{"name":"x","version":"1","futureField":7,"scrapers":[{"id":"a","name":"A","version":"1","filename":"a.js"}]}"#).is_ok());
    }
}
