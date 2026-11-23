//! TMDB sources of collection folders (`TmdbCollectionSourceResolver` in the original).
//!
//! A folder of an account collection can list a TMDB source instead of an addon catalog: a discover
//! query with filters (watch provider, genres, exclusions, sort), a company or a network. The
//! original resolves them against `api.themoviedb.org` with the key configured in Settings → TMDB;
//! Astra reads the same key from the original app's store, read only, until it has its own field.
//!
//! Covered here: `DISCOVER`, `NETWORK` and `COMPANY` (the same discover endpoint). `LIST`,
//! `COLLECTION`, `PERSON` and `DIRECTOR` sources are still skipped by `collections`.

use nuvio_core::catalog::{CatalogPage, MetaPreview, PosterShape};
use nuvio_core::meta::{MetaDetails, MetaPerson, MetaTrailer};
use nuvio_core::client::AddonClient;
use nuvio_store::nextgen::AppStore;
use nuvio_store::{default_legacy_data_dir, LegacyStore};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Movie,
    Tv,
}

impl MediaType {
    /// `TmdbCollectionMediaType.fromString`: "tv"/"series" is TV, anything else a movie.
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_lowercase()).as_deref() {
            Some("tv") | Some("series") => Self::Tv,
            _ => Self::Movie,
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            Self::Movie => "discover/movie",
            Self::Tv => "discover/tv",
        }
    }

    /// Content type of the UI (`MetaPreview.type`): series for TV, movie otherwise.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "series",
        }
    }
}

/// `TmdbCollectionFilters` as stored by the account.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filters {
    pub with_genres: Option<String>,
    pub without_genres: Option<String>,
    pub release_date_gte: Option<String>,
    pub release_date_lte: Option<String>,
    pub vote_average_gte: Option<f64>,
    pub vote_average_lte: Option<f64>,
    pub vote_count_gte: Option<i64>,
    pub with_original_language: Option<String>,
    pub with_origin_country: Option<String>,
    pub with_keywords: Option<String>,
    pub without_keywords: Option<String>,
    pub with_companies: Option<String>,
    pub without_companies: Option<String>,
    pub with_networks: Option<String>,
    pub year: Option<i64>,
    pub watch_region: Option<String>,
    pub with_watch_providers: Option<String>,
    pub without_watch_providers: Option<String>,
}

/// One TMDB source of a folder, ready to be queried.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// `DISCOVER`, `NETWORK` or `COMPANY`.
    pub source_type: String,
    pub media_type: MediaType,
    pub sort_by: Option<String>,
    /// Company or network id (`tmdbId` of the source).
    pub tmdb_id: Option<i64>,
    pub filters: Filters,
}

/// Every TMDB source kind of the original (`TmdbCollectionSourceType`).
pub fn supported(source_type: &str) -> bool {
    matches!(source_type.trim().to_uppercase().as_str(), "DISCOVER" | "NETWORK" | "COMPANY" | "LIST" | "COLLECTION" | "PERSON" | "DIRECTOR")
}

/// Per-profile TMDB settings of Astra (`tmdb-settings.json`). Until they are edited here, the
/// original app's key and language are used, so an existing install keeps working.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub api_key: String,
    pub language: String,
    pub use_more_like_this: bool,
    pub use_collections: bool,
    pub use_artwork: bool,
    pub use_basic_info: bool,
    pub use_details: bool,
    pub use_credits: bool,
    pub use_episodes: bool,
    pub use_trailers: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: String::new(),
            language: String::new(),
            use_more_like_this: true,
            use_collections: true,
            use_artwork: true,
            use_basic_info: true,
            use_details: true,
            use_credits: true,
            use_episodes: true,
            use_trailers: true,
        }
    }
}

/// What the settings page sees: never the key itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub enabled: bool,
    pub language: String,
    pub use_more_like_this: bool,
    pub use_collections: bool,
    pub use_artwork: bool,
    pub use_basic_info: bool,
    pub use_details: bool,
    pub use_credits: bool,
    pub use_episodes: bool,
    pub use_trailers: bool,
    /// A key is available (Astra's own or the original app's).
    pub api_key_set: bool,
    /// The key in use came from the Nuvio account (`provider-credentials`).
    pub api_key_from_account: bool,
    /// The key in use is the original app's, not one saved in Astra.
    pub api_key_from_original: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsInput {
    pub enabled: bool,
    /// `None` keeps the current key; an empty string clears Astra's key (the original's is used).
    pub api_key: Option<String>,
    pub language: String,
    pub use_more_like_this: bool,
    pub use_collections: bool,
    pub use_artwork: bool,
    pub use_basic_info: bool,
    pub use_details: bool,
    pub use_credits: bool,
    pub use_episodes: bool,
    pub use_trailers: bool,
}

impl Default for SettingsInput {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: None,
            language: String::new(),
            use_more_like_this: true,
            use_collections: true,
            use_artwork: true,
            use_basic_info: true,
            use_details: true,
            use_credits: true,
            use_episodes: true,
            use_trailers: true,
        }
    }
}

/// TMDB access of a profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Access {
    api_key: String,
    pub language: String,
}

fn legacy_settings() -> Option<LegacyStore> {
    LegacyStore::open(default_legacy_data_dir().ok()?).ok()
}

/// Key and language of the original app for this profile (read only).
fn legacy_access(profile: i64) -> Option<Access> {
    let store = legacy_settings()?;
    let values = store.read_store("nuvio_tmdb_settings").ok()?;
    let api_key = values.get(&format!("tmdb_api_key_{profile}"))?.trim().to_string();
    if api_key.is_empty() {
        return None;
    }
    let language = normalize_language(values.get(&format!("tmdb_language_{profile}")).map(String::as_str));
    Some(Access { api_key, language })
}

fn stored(store: &AppStore) -> std::collections::BTreeMap<i64, Settings> {
    store.read("tmdb-settings").ok().flatten().unwrap_or_default()
}

/// Astra's settings of the profile (defaults when it never saved any).
pub fn settings(store: &AppStore, profile: i64) -> Settings {
    stored(store).remove(&profile).unwrap_or_default()
}

fn write(store: &AppStore, profile: i64, settings: &Settings) -> Result<(), String> {
    let mut all = stored(store);
    all.insert(profile, settings.clone());
    store.write("tmdb-settings", &all).map_err(|e| e.to_string())
}

/// Saves the page's settings (the key only when it was edited) and returns what is in use.
pub fn update(store: &AppStore, profile: i64, input: SettingsInput) -> Result<SettingsView, String> {
    let edited_key = input.api_key.clone();
    let mut next = settings(store, profile);
    next.enabled = input.enabled;
    next.use_more_like_this = input.use_more_like_this;
    next.use_collections = input.use_collections;
    next.use_artwork = input.use_artwork;
    next.use_basic_info = input.use_basic_info;
    next.use_details = input.use_details;
    next.use_credits = input.use_credits;
    next.use_episodes = input.use_episodes;
    next.use_trailers = input.use_trailers;
    next.language = input.language.trim().to_string();
    if let Some(key) = input.api_key {
        next.api_key = key.trim().to_string();
    }
    write(store, profile, &next)?;
    if let Some(key) = edited_key {
        // The account's credentials are separate from the settings blob (`ProviderCredentialSync`);
        // keeping this copy lets the next sync send the key to the other devices.
        if let Ok(mut all) = store.provider_credentials() {
            let mut providers = all.remove(&profile).unwrap_or_default();
            if key.trim().is_empty() {
                providers.remove("tmdb");
            } else {
                providers.insert("tmdb".into(), key.trim().to_string());
            }
            let _ = store.set_provider_credentials(profile, providers);
        }
    }
    Ok(view(store, profile))
}

/// Settings the page shows, with the language that is really used and whether a key exists.
pub fn view(store: &AppStore, profile: i64) -> SettingsView {
    let settings = settings(store, profile);
    let legacy = legacy_access(profile);
    let account = store.provider_key(profile, "tmdb");
    let original_key = legacy.as_ref().is_some_and(|l| !l.api_key.is_empty());
    let own = !settings.api_key.trim().is_empty();
    let api_key_from_account = !own && account.is_some();
    let api_key_from_original = !own && account.is_none() && original_key;
    SettingsView {
        enabled: settings.enabled,
        language: if settings.language.trim().is_empty() {
            legacy.map(|l| l.language).unwrap_or_else(|| "en".into())
        } else {
            normalize_language(Some(&settings.language))
        },
        use_more_like_this: settings.use_more_like_this,
        use_collections: settings.use_collections,
        use_artwork: settings.use_artwork,
        use_basic_info: settings.use_basic_info,
        use_details: settings.use_details,
        use_credits: settings.use_credits,
        use_episodes: settings.use_episodes,
        use_trailers: settings.use_trailers,
        api_key_set: own || account.is_some() || original_key,
        api_key_from_account,
        api_key_from_original,
    }
}

/// Effective access: disabled settings or no key at all mean no TMDB requests.
pub fn access(store: &AppStore, profile: i64) -> Option<Access> {
    resolve(settings(store, profile), store.provider_key(profile, "tmdb"), legacy_access(profile))
}

fn resolve(settings: Settings, account_key: Option<String>, legacy: Option<Access>) -> Option<Access> {
    if !settings.enabled {
        return None;
    }
    let api_key = if !settings.api_key.trim().is_empty() {
        settings.api_key.trim().to_string()
    } else if let Some(account) = account_key.filter(|key| !key.trim().is_empty()) {
        account
    } else {
        legacy.as_ref()?.api_key.clone()
    };
    let language = if settings.language.trim().is_empty() {
        legacy.map(|l| l.language).unwrap_or_else(|| "en".into())
    } else {
        normalize_language(Some(&settings.language))
    };
    Some(Access { api_key, language })
}

/// Whether collection folders may list TMDB sources.
pub fn collections_enabled(store: &AppStore, profile: i64) -> bool {
    settings(store, profile).use_collections && access(store, profile).is_some()
}

/// Whether the details screen may fetch "More Like This".
pub fn more_like_this_enabled(store: &AppStore, profile: i64) -> bool {
    settings(store, profile).use_more_like_this && access(store, profile).is_some()
}

/// Whether this profile can list TMDB collection sources (a key is configured).
pub fn has_key(store: &AppStore, profile: i64) -> bool {
    access(store, profile).is_some()
}

/// `normalizeTmdbLanguage`: `xx-YY`, `en` when unset.
fn normalize_language(value: Option<&str>) -> String {
    let raw = match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => v.replace('_', "-"),
        None => return "en".into(),
    };
    let parts: Vec<&str> = raw.split('-').collect();
    if parts.len() == 2 {
        format!("{}-{}", parts[0].to_lowercase(), parts[1].to_uppercase())
    } else {
        raw.to_lowercase()
    }
}

/// `movieSort`/`tvSort`: "original" starts from popularity, and the date sorts swap by media type.
fn sort_for(sort_by: Option<&str>, media: MediaType) -> String {
    match (sort_by.map(str::trim).filter(|v| !v.is_empty()), media) {
        (None, _) | (Some("original"), _) => "popularity.desc".into(),
        (Some("first_air_date.desc"), MediaType::Movie) => "primary_release_date.desc".into(),
        (Some("primary_release_date.desc"), MediaType::Tv) => "first_air_date.desc".into(),
        (Some(other), _) => other.to_string(),
    }
}

/// `buildDiscoverQuery`: the pairs sent to `discover/{movie|tv}`.
pub fn discover_query(source: &Source, page: u32, language: &str) -> Vec<(String, String)> {
    let media = source.media_type;
    let filters = &source.filters;
    let mut query: Vec<(String, String)> = Vec::new();
    let mut put = |name: &str, value: Option<String>| {
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            query.push((name.to_string(), value));
        }
    };
    put("language", Some(language.to_string()));
    put("page", Some(page.to_string()));
    put("sort_by", Some(sort_for(source.sort_by.as_deref(), media)));
    let source_type = source.source_type.trim().to_uppercase();
    let company = source.tmdb_id.map(|id| id.to_string()).filter(|_| source_type == "COMPANY");
    let network = source.tmdb_id.map(|id| id.to_string()).filter(|_| source_type == "NETWORK");
    put("with_companies", company.or_else(|| filters.with_companies.clone()));
    put("without_companies", filters.without_companies.clone());
    put("with_networks", network.or_else(|| filters.with_networks.clone()));
    put("with_genres", filters.with_genres.clone());
    put("without_genres", filters.without_genres.clone());
    put("vote_count.gte", filters.vote_count_gte.map(|v| v.to_string()));
    put("vote_average.gte", filters.vote_average_gte.map(format_number));
    put("vote_average.lte", filters.vote_average_lte.map(format_number));
    put("with_original_language", filters.with_original_language.clone());
    put("with_origin_country", filters.with_origin_country.clone());
    put("with_keywords", filters.with_keywords.clone());
    put("without_keywords", filters.without_keywords.clone());
    let with_providers = filters.with_watch_providers.clone().filter(|v| !v.trim().is_empty());
    let without_providers = filters.without_watch_providers.clone().filter(|v| !v.trim().is_empty());
    if with_providers.is_some() || without_providers.is_some() {
        put("with_watch_providers", with_providers.clone());
        put("without_watch_providers", without_providers);
        put("watch_region", Some(filters.watch_region.clone().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "US".into())));
        if with_providers.is_some() {
            put("with_watch_monetization_types", Some("flatrate|free|ads|rent|buy".into()));
        }
    }
    put("year", filters.year.filter(|_| media == MediaType::Movie).map(|v| v.to_string()));
    put("first_air_date_year", filters.year.filter(|_| media == MediaType::Tv).map(|v| v.to_string()));
    let date_gte = if media == MediaType::Movie { "primary_release_date.gte" } else { "first_air_date.gte" };
    let date_lte = if media == MediaType::Movie { "primary_release_date.lte" } else { "first_air_date.lte" };
    put(date_gte, filters.release_date_gte.clone());
    put(date_lte, filters.release_date_lte.clone());
    query
}

/// Same text the original sends for a `Double` (`7.0`, `6.5`).
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

/// `buildTmdbUrl`: the original appends the raw values (they are numbers, ids and sort keys).
fn tmdb_url(access: &Access, endpoint: &str, query: &[(String, String)]) -> String {
    let mut url = format!("https://api.themoviedb.org/3/{endpoint}?api_key={}", access.api_key);
    for (name, value) in query {
        url.push('&');
        url.push_str(name);
        url.push('=');
        url.push_str(value);
    }
    url
}

fn discover_url(access: &Access, source: &Source, page: u32) -> String {
    tmdb_url(access, source.media_type.endpoint(), &discover_query(source, page, &access.language))
}

#[derive(Debug, Deserialize, Default)]
struct FindResponse {
    #[serde(default)]
    movie_results: Vec<FindItem>,
    #[serde(default)]
    tv_results: Vec<FindItem>,
}

#[derive(Debug, Deserialize)]
struct FindItem {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct RecommendationsResponse {
    #[serde(default)]
    results: Vec<DiscoverItem>,
}

#[derive(Debug, Deserialize)]
struct DiscoverResponse {
    page: Option<u32>,
    total_pages: Option<u32>,
    results: Option<Vec<DiscoverItem>>,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    page: Option<u32>,
    total_pages: Option<u32>,
    items: Option<Vec<DiscoverItem>>,
}

#[derive(Debug, Deserialize)]
struct CollectionResponse {
    parts: Option<Vec<DiscoverItem>>,
}

#[derive(Debug, Deserialize)]
struct PersonResponse {
    cast: Option<Vec<DiscoverItem>>,
    crew: Option<Vec<DiscoverItem>>,
}

/// `TmdbListItem` and its siblings: a discover result, a list/collection item or a person credit.
#[derive(Debug, Deserialize)]
struct DiscoverItem {
    id: i64,
    #[serde(default)]
    media_type: Option<String>,
    title: Option<String>,
    name: Option<String>,
    original_title: Option<String>,
    original_name: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    overview: Option<String>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    vote_average: Option<f64>,
    #[serde(default)]
    vote_count: Option<i64>,
    #[serde(default)]
    popularity: Option<f64>,
    #[serde(default)]
    job: Option<String>,
}

impl DiscoverItem {
    /// `mediaType` of a list item: "tv" is a series, anything else a movie.
    fn media(&self) -> MediaType {
        match self.media_type.as_deref().map(str::to_lowercase).as_deref() {
            Some("tv") => MediaType::Tv,
            _ => MediaType::Movie,
        }
    }
}

fn image_url(path: Option<&str>, size: &str) -> Option<String> {
    let path = path.map(str::trim).filter(|p| !p.is_empty())?;
    Some(format!("https://image.tmdb.org/t/p/{size}{path}"))
}

fn non_blank(value: Option<&String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl DiscoverItem {
    /// `TmdbListItem.toPreview`: `id` is `tmdb:<id>`, series for TV, the year as release info and
    /// the vote average with one decimal as the rating.
    fn to_preview(&self, media: MediaType) -> Option<MetaPreview> {
        let name = non_blank(self.title.as_ref())
            .or_else(|| non_blank(self.name.as_ref()))
            .or_else(|| non_blank(self.original_title.as_ref()))
            .or_else(|| non_blank(self.original_name.as_ref()))?;
        let (release_info, raw_release_date) = match media {
            MediaType::Movie => (self.release_date.as_deref(), self.release_date.clone()),
            MediaType::Tv => (self.first_air_date.as_deref(), self.first_air_date.clone()),
        };
        let banner = image_url(self.backdrop_path.as_deref(), "w1280");
        Some(MetaPreview {
            id: format!("tmdb:{}", self.id),
            content_type: media.content_type().into(),
            name,
            poster: image_url(self.poster_path.as_deref(), "w500").or_else(|| image_url(self.backdrop_path.as_deref(), "w780")),
            banner,
            logo: None,
            poster_shape: PosterShape::Poster,
            description: non_blank(self.overview.as_ref()),
            release_info: release_info.map(|d| d.chars().take(4).collect()).filter(|y: &String| !y.trim().is_empty()),
            raw_release_date,
            imdb_rating: self.vote_average.map(|v| format!("{:.1}", (v * 10.0).round() / 10.0)),
            genres: Vec::new(),
            landscape_poster: None,
            runtime: None,
            status: None,
        })
    }

    /// `TmdbPersonCredit*.toPreview`: only credits of the source's media type.
    fn to_credit_preview(&self, media: MediaType) -> Option<MetaPreview> {
        if self.media() != media {
            return None;
        }
        self.to_preview(media)
    }
}

/// `sortedFor`: only these sorts reorder; "original"/popularity keep the API order.
fn sort_items(mut items: Vec<(MetaPreview, i64)>, sort_by: Option<&str>) -> Vec<MetaPreview> {
    let rating = |item: &MetaPreview| item.imdb_rating.as_deref().and_then(|r| r.parse::<f64>().ok()).unwrap_or(-1.0);
    let date = |item: &MetaPreview| item.raw_release_date.clone().or_else(|| item.release_info.clone()).unwrap_or_default();
    match sort_by.map(str::trim).unwrap_or("") {
        "vote_average.desc" => items.sort_by(|a, b| rating(&b.0).partial_cmp(&rating(&a.0)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| date(&b.0).cmp(&date(&a.0)))),
        "vote_count.desc" => items.sort_by_key(|(_, votes)| std::cmp::Reverse(*votes)),
        "primary_release_date.desc" | "first_air_date.desc" => items.sort_by(|a, b| date(&b.0).cmp(&date(&a.0))),
        _ => {}
    }
    items.into_iter().map(|(item, _)| item).collect()
}

fn dedupe(items: Vec<MetaPreview>) -> Vec<MetaPreview> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|item| seen.insert(format!("{}:{}", item.content_type, item.id))).collect()
}

fn dedupe_votes(items: Vec<(MetaPreview, i64)>) -> Vec<(MetaPreview, i64)> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|(item, _)| seen.insert(format!("{}:{}", item.content_type, item.id))).collect()
}

/// `resolveList`: `/list/{id}`, sorted then deduplicated, paged with the list's page number.
pub fn parse_list(payload: &str, page: u32, sort_by: Option<&str>) -> Result<CatalogPage, String> {
    let body: ListResponse = serde_json::from_str(payload).map_err(|e| e.to_string())?;
    let items: Vec<(MetaPreview, i64)> = body
        .items
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.to_preview(item.media()).map(|preview| (preview, item.vote_count.unwrap_or(0))))
        .collect();
    let items = dedupe(sort_items(items, sort_by));
    let current = body.page.unwrap_or(page);
    let total = body.total_pages.unwrap_or(page);
    let next_skip = if current < total && !items.is_empty() { Some(page + 1) } else { None };
    Ok(CatalogPage { raw_item_count: items.len(), items, next_skip })
}

/// `resolveCollection`: `/collection/{id}`, its parts, no paging.
pub fn parse_collection(payload: &str, sort_by: Option<&str>) -> Result<CatalogPage, String> {
    let body: CollectionResponse = serde_json::from_str(payload).map_err(|e| e.to_string())?;
    let items: Vec<(MetaPreview, i64)> = body
        .parts
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.to_preview(MediaType::Movie).map(|preview| (preview, item.vote_count.unwrap_or(0))))
        .collect();
    let items = dedupe(sort_items(items, sort_by));
    Ok(CatalogPage { raw_item_count: items.len(), items, next_skip: None })
}

/// `resolvePersonCredits`: `/person/{id}/combined_credits`, cast (or crew "Director" for DIRECTOR
/// sources) of the source's media type, deduplicated then sorted; no paging.
pub fn parse_person(payload: &str, media: MediaType, director: bool, sort_by: Option<&str>) -> Result<CatalogPage, String> {
    let body: PersonResponse = serde_json::from_str(payload).map_err(|e| e.to_string())?;
    let credits: Vec<DiscoverItem> = if director {
        body.crew
            .unwrap_or_default()
            .into_iter()
            .filter(|credit| credit.job.as_deref().is_some_and(|job| job.eq_ignore_ascii_case("director")))
            .collect()
    } else {
        body.cast.unwrap_or_default()
    };
    let items: Vec<(MetaPreview, i64)> = credits
        .iter()
        .filter_map(|credit| credit.to_credit_preview(media).map(|preview| (preview, credit.vote_count.unwrap_or(0))))
        .collect();
    let items = sort_items(dedupe_votes(items), sort_by);
    Ok(CatalogPage { raw_item_count: items.len(), items, next_skip: None })
}

/// One page of a TMDB source (`resolve`): discover/network/company, list, collection or the credits
/// of a person/director. `page` is 1-based, as the original's `skip`.
pub async fn page(client: &AddonClient, access: &Access, source: &Source, page: u32) -> Result<CatalogPage, String> {
    let page = page.max(1);
    let source_type = source.source_type.trim().to_uppercase();
    match source_type.as_str() {
        "LIST" => {
            let id = source.tmdb_id.ok_or("ID da lista TMDB ausente.")?;
            let query = vec![("language".to_string(), access.language.clone()), ("page".to_string(), page.to_string())];
            let payload = client.get_text(&tmdb_url(access, &format!("list/{id}"), &query), false).await.map_err(|e| e.to_string())?;
            parse_list(&payload, page, source.sort_by.as_deref())
        }
        "COLLECTION" => {
            let id = source.tmdb_id.ok_or("ID da coleção TMDB ausente.")?;
            let query = vec![("language".to_string(), access.language.clone())];
            let payload = client.get_text(&tmdb_url(access, &format!("collection/{id}"), &query), false).await.map_err(|e| e.to_string())?;
            parse_collection(&payload, source.sort_by.as_deref())
        }
        "PERSON" | "DIRECTOR" => {
            let id = source.tmdb_id.ok_or("ID da pessoa TMDB ausente.")?;
            let query = vec![("language".to_string(), access.language.clone())];
            let payload = client
                .get_text(&tmdb_url(access, &format!("person/{id}/combined_credits"), &query), false)
                .await
                .map_err(|e| e.to_string())?;
            parse_person(&payload, source.media_type, source_type == "DIRECTOR", source.sort_by.as_deref())
        }
        _ => {
            let payload = client.get_text(&discover_url(access, source, page), false).await.map_err(|e| e.to_string())?;
            parse_page(&payload, page, source.media_type)
        }
    }
}

/// Same mapping and pagination rules as `resolveDiscover`, without the network call.
pub fn parse_page(payload: &str, page: u32, media: MediaType) -> Result<CatalogPage, String> {
    let body: DiscoverResponse = serde_json::from_str(payload).map_err(|e| e.to_string())?;
    let items: Vec<MetaPreview> = body
        .results
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.to_preview(media))
        .collect();
    let items = dedupe(items);
    let current = body.page.unwrap_or(page);
    let total = body.total_pages.unwrap_or(page);
    let next_skip = if current < total && !items.is_empty() { Some(page + 1) } else { None };
    let raw_item_count = items.len();
    Ok(CatalogPage { items, raw_item_count, next_skip })
}

/// Media type of a details `contentType`.
pub fn media_of(content_type: &str) -> MediaType {
    if content_type.trim().eq_ignore_ascii_case("series") {
        MediaType::Tv
    } else {
        MediaType::Movie
    }
}

/// TMDB id of a title: `tmdb:<id>` as the addons send it, a numeric id, or an IMDb id resolved
/// through `/find` (the addons mostly use IMDb ids).
async fn tmdb_id(client: &AddonClient, access: &Access, media: MediaType, meta_id: &str) -> Option<i64> {
    let meta_id = meta_id.trim();
    if let Some(rest) = meta_id.strip_prefix("tmdb:") {
        return rest.trim().parse().ok();
    }
    if !meta_id.is_empty() && meta_id.chars().all(|c| c.is_ascii_digit()) {
        return meta_id.parse().ok();
    }
    if !meta_id.starts_with("tt") {
        return None;
    }
    let query = vec![("external_source".to_string(), "imdb_id".to_string()), ("language".to_string(), access.language.clone())];
    let payload = client.get_text(&tmdb_url(access, &format!("find/{meta_id}"), &query), false).await.ok()?;
    let body: FindResponse = serde_json::from_str(&payload).ok()?;
    match media {
        MediaType::Movie => body.movie_results.first().map(|item| item.id),
        MediaType::Tv => body.tv_results.first().map(|item| item.id),
    }
}

/// `fetchMoreLikeThis`: `/movie|tv/{id}/recommendations`, at most 12 titles, the item's own media
/// type when the answer carries one.
pub async fn more_like_this(client: &AddonClient, access: &Access, content_type: &str, meta_id: &str) -> Vec<MetaPreview> {
    let media = media_of(content_type);
    let Some(id) = tmdb_id(client, access, media, meta_id).await else { return Vec::new() };
    let endpoint = format!("{}/{id}/recommendations", if media == MediaType::Tv { "tv" } else { "movie" });
    let query = vec![("language".to_string(), access.language.clone())];
    let Ok(payload) = client.get_text(&tmdb_url(access, &endpoint, &query), false).await else { return Vec::new() };
    parse_recommendations(&payload, media)
}

/// Same mapping as `fetchMoreLikeThis`, without the network calls.
pub fn parse_recommendations(payload: &str, media: MediaType) -> Vec<MetaPreview> {
    let Ok(body) = serde_json::from_str::<RecommendationsResponse>(payload) else { return Vec::new() };
    body.results
        .iter()
        .filter(|item| item.id > 0)
        .filter_map(|item| {
            let inferred = match item.media_type.as_deref().map(str::to_lowercase).as_deref() {
                Some("tv") => MediaType::Tv,
                Some("movie") => MediaType::Movie,
                _ => media,
            };
            item.to_preview(inferred)
        })
        .take(12)
        .collect()
}

// ---------------------------------------------------------------- enrichment (details page)

/// What TMDB adds to a title's details, built once and applied according to the module switches.
#[derive(Debug, Clone, Default)]
struct Enrichment {
    title: Option<String>,
    overview: Option<String>,
    genres: Vec<String>,
    poster: Option<String>,
    backdrop: Option<String>,
    logo: Option<String>,
    rating: Option<f64>,
    runtime: Option<i64>,
    status: Option<String>,
    countries: Vec<String>,
    language: Option<String>,
    age_rating: Option<String>,
    director: Vec<String>,
    writer: Vec<String>,
    cast: Vec<MetaPerson>,
    trailers: Vec<MetaTrailer>,
    episodes: HashMap<(i64, i64), EpisodeInfo>,
}

#[derive(Debug, Clone)]
struct EpisodeInfo {
    title: Option<String>,
    overview: Option<String>,
    thumbnail: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct TitleResponse {
    title: Option<String>,
    name: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    vote_average: Option<f64>,
    runtime: Option<i64>,
    #[serde(default)]
    episode_run_time: Vec<i64>,
    status: Option<String>,
    #[serde(default)]
    production_countries: Vec<Country>,
    #[serde(default)]
    origin_country: Vec<String>,
    #[serde(default)]
    spoken_languages: Vec<Spoken>,
    original_language: Option<String>,
    #[serde(default)]
    genres: Vec<Named>,
    credits: Option<Credits>,
    aggregate_credits: Option<AggregateCredits>,
    images: Option<Images>,
    videos: Option<Videos>,
    release_dates: Option<ReleaseDates>,
    content_ratings: Option<ContentRatings>,
}

#[derive(Debug, Deserialize, Default)]
struct Named {
    name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Country {
    iso_3166_1: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Spoken {
    english_name: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Credits {
    #[serde(default)]
    cast: Vec<CastMember>,
    #[serde(default)]
    crew: Vec<CrewMember>,
}

#[derive(Debug, Deserialize, Default)]
struct CastMember {
    name: Option<String>,
    character: Option<String>,
    profile_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CrewMember {
    name: Option<String>,
    job: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AggregateCredits {
    #[serde(default)]
    cast: Vec<AggregateCast>,
    #[serde(default)]
    crew: Vec<AggregateCrew>,
}

#[derive(Debug, Deserialize, Default)]
struct AggregateCast {
    name: Option<String>,
    profile_path: Option<String>,
    #[serde(default)]
    roles: Vec<Role>,
}

#[derive(Debug, Deserialize, Default)]
struct Role {
    character: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AggregateCrew {
    name: Option<String>,
    #[serde(default)]
    jobs: Vec<Job>,
}

#[derive(Debug, Deserialize, Default)]
struct Job {
    job: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Images {
    #[serde(default)]
    logos: Vec<ImageItem>,
}

#[derive(Debug, Deserialize, Default)]
struct ImageItem {
    file_path: Option<String>,
    iso_639_1: Option<String>,
    vote_average: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
struct Videos {
    #[serde(default)]
    results: Vec<Video>,
}

#[derive(Debug, Deserialize, Default)]
struct Video {
    key: Option<String>,
    name: Option<String>,
    site: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    official: bool,
    published_at: Option<String>,
    size: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct ReleaseDates {
    #[serde(default)]
    results: Vec<ReleaseCountry>,
}

#[derive(Debug, Deserialize, Default)]
struct ReleaseCountry {
    iso_3166_1: Option<String>,
    #[serde(default)]
    release_dates: Vec<ReleaseInfo>,
}

#[derive(Debug, Deserialize, Default)]
struct ReleaseInfo {
    certification: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ContentRatings {
    #[serde(default)]
    results: Vec<RatingCountry>,
}

#[derive(Debug, Deserialize, Default)]
struct RatingCountry {
    iso_3166_1: Option<String>,
    rating: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct SeasonResponse {
    #[serde(default)]
    episodes: Vec<SeasonEpisode>,
}

#[derive(Debug, Deserialize, Default)]
struct SeasonEpisode {
    episode_number: Option<i64>,
    name: Option<String>,
    overview: Option<String>,
    still_path: Option<String>,
}

static ENRICHMENT_CACHE: OnceLock<Mutex<HashMap<String, Arc<Enrichment>>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Arc<Enrichment>>> {
    ENRICHMENT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Builds the enrichment from a details payload (`append_to_response` credits/images/videos/
/// ratings). Pure, so the mapping is tested without the network.
fn parse_enrichment(payload: &str, media: MediaType, language: &str) -> Option<Enrichment> {
    let body: TitleResponse = serde_json::from_str(payload).ok()?;
    let mut enrichment = Enrichment {
        title: non_blank(body.title.as_ref()).or_else(|| non_blank(body.name.as_ref())),
        overview: non_blank(body.overview.as_ref()),
        genres: body.genres.iter().filter_map(|g| non_blank(g.name.as_ref())).collect(),
        poster: image_url(body.poster_path.as_deref(), "w500"),
        backdrop: image_url(body.backdrop_path.as_deref(), "w1280"),
        logo: pick_logo(body.images.as_ref(), language.split('-').next().unwrap_or("en")),
        rating: body.vote_average,
        runtime: body.runtime.or_else(|| body.episode_run_time.first().copied()),
        status: non_blank(body.status.as_ref()),
        countries: {
            let mut countries: Vec<String> = body.production_countries.iter().filter_map(|c| non_blank(c.iso_3166_1.as_ref())).collect();
            if countries.is_empty() {
                countries = body.origin_country.iter().filter_map(|c| non_blank(Some(c))).collect();
            }
            countries
        },
        language: body
            .spoken_languages
            .iter()
            .filter_map(|l| non_blank(l.english_name.as_ref()).or_else(|| non_blank(l.name.as_ref())))
            .next()
            .or_else(|| non_blank(body.original_language.as_ref())),
        age_rating: age_rating(&body, media, language),
        ..Default::default()
    };
    let (director, writer, cast) = match (body.credits.as_ref(), body.aggregate_credits.as_ref()) {
        (Some(credits), _) => credits_parts(credits),
        (_, Some(credits)) => aggregate_parts(credits),
        _ => (Vec::new(), Vec::new(), Vec::new()),
    };
    enrichment.director = director;
    enrichment.writer = writer;
    enrichment.cast = cast;
    enrichment.trailers = trailers(body.videos.as_ref());
    Some(enrichment)
}

fn pick_logo(images: Option<&Images>, language: &str) -> Option<String> {
    let logos = images?.logos.as_slice();
    let pref = |item: &ImageItem| item.iso_639_1.as_deref().is_some_and(|l| l == language);
    let en = |item: &ImageItem| item.iso_639_1.as_deref() == Some("en");
    let neutral = |item: &ImageItem| item.iso_639_1.is_none();
    let best = logos
        .iter()
        .filter(|l| pref(l) || en(l) || neutral(l))
        .max_by(|a, b| a.vote_average.unwrap_or(0.0).partial_cmp(&b.vote_average.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal))
        .or_else(|| logos.first())?;
    image_url(best.file_path.as_deref(), "w500")
}

fn age_rating(body: &TitleResponse, media: MediaType, language: &str) -> Option<String> {
    let region = language.split('-').nth(1).unwrap_or(match language.split('-').next().unwrap_or("en") {
        "pt" => "BR",
        "ar" => "SA",
        "es" => "ES",
        "fr" => "FR",
        "de" => "DE",
        "tr" => "TR",
        "ja" => "JP",
        "ko" => "KR",
        "zh" => "CN",
        _ => "US",
    });
    let pick = |country: Option<&String>, value: Option<&String>| -> Option<String> {
        if country.map(String::as_str) == Some(region) {
            non_blank(value)
        } else {
            None
        }
    };
    if media == MediaType::Tv {
        let results = &body.content_ratings.as_ref()?.results;
        results.iter().find_map(|r| pick(r.iso_3166_1.as_ref(), r.rating.as_ref())).or_else(|| {
            results.iter().find_map(|r| non_blank(r.rating.as_ref()))
        })
    } else {
        let results = &body.release_dates.as_ref()?.results;
        results
            .iter()
            .find_map(|r| {
                if r.iso_3166_1.as_deref() != Some(region) {
                    return None;
                }
                r.release_dates.iter().find_map(|d| non_blank(d.certification.as_ref()))
            })
            .or_else(|| results.iter().flat_map(|r| r.release_dates.iter()).find_map(|d| non_blank(d.certification.as_ref())))
    }
}

fn credits_parts(credits: &Credits) -> (Vec<String>, Vec<String>, Vec<MetaPerson>) {
    let director = credits
        .crew
        .iter()
        .filter(|c| c.job.as_deref().is_some_and(|job| job.eq_ignore_ascii_case("director")))
        .filter_map(|c| non_blank(c.name.as_ref()))
        .collect();
    let writer = credits
        .crew
        .iter()
        .filter(|c| c.job.as_deref().is_some_and(is_writing_job))
        .filter_map(|c| non_blank(c.name.as_ref()))
        .collect();
    let cast = credits
        .cast
        .iter()
        .take(30)
        .filter_map(|c| {
            Some(MetaPerson {
                name: non_blank(c.name.as_ref())?,
                role: non_blank(c.character.as_ref()),
                photo: image_url(c.profile_path.as_deref(), "w185"),
            })
        })
        .collect();
    (director, writer, cast)
}

fn aggregate_parts(credits: &AggregateCredits) -> (Vec<String>, Vec<String>, Vec<MetaPerson>) {
    let director = credits
        .crew
        .iter()
        .filter(|c| c.jobs.iter().any(|j| j.job.as_deref().is_some_and(|job| job.eq_ignore_ascii_case("director"))))
        .filter_map(|c| non_blank(c.name.as_ref()))
        .collect();
    let writer = credits
        .crew
        .iter()
        .filter(|c| c.jobs.iter().any(|j| j.job.as_deref().is_some_and(is_writing_job)))
        .filter_map(|c| non_blank(c.name.as_ref()))
        .collect();
    let cast = credits
        .cast
        .iter()
        .take(30)
        .filter_map(|c| {
            Some(MetaPerson {
                name: non_blank(c.name.as_ref())?,
                role: c.roles.first().and_then(|r| non_blank(r.character.as_ref())),
                photo: image_url(c.profile_path.as_deref(), "w185"),
            })
        })
        .collect();
    (director, writer, cast)
}

fn is_writing_job(job: &str) -> bool {
    matches!(job.to_lowercase().as_str(), "screenplay" | "writer" | "story" | "author" | "writing")
}

fn trailers(videos: Option<&Videos>) -> Vec<MetaTrailer> {
    let mut seen = HashSet::new();
    videos
        .map(|v| v.results.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter(|v| v.site.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("youtube")))
        .filter(|v| v.kind.as_deref().is_some_and(|k| matches!(k.to_lowercase().as_str(), "trailer" | "teaser")))
        .filter_map(|v| {
            let key = non_blank(v.key.as_ref())?;
            if !seen.insert(key.clone()) {
                return None;
            }
            Some(MetaTrailer {
                id: key.clone(),
                key,
                name: non_blank(v.name.as_ref()).unwrap_or_else(|| "Trailer".into()),
                site: "YouTube".into(),
                size: v.size,
                kind: v.kind.clone().unwrap_or_else(|| "Trailer".into()),
                official: v.official,
                published_at: non_blank(v.published_at.as_ref()),
                season_number: None,
            })
        })
        .take(12)
        .collect()
}

/// `applyEnrichment`: only the switched modules, overwriting with TMDB when it has the value (the
/// original's rules).
fn apply_enrichment(meta: &mut MetaDetails, enrichment: &Enrichment, settings: &Settings) {
    if settings.use_artwork {
        if enrichment.backdrop.is_some() {
            meta.background = enrichment.backdrop.clone();
        }
        if enrichment.poster.is_some() {
            meta.poster = enrichment.poster.clone();
        }
        if enrichment.logo.is_some() {
            meta.logo = enrichment.logo.clone();
        }
    }
    if settings.use_basic_info {
        if enrichment.title.is_some() {
            meta.name = enrichment.title.clone().unwrap();
        }
        if enrichment.overview.is_some() {
            meta.description = enrichment.overview.clone();
        }
        if meta.imdb_rating.as_deref().is_none_or(|r| r.trim().is_empty()) {
            meta.imdb_rating = enrichment.rating.map(|r| format!("{r:.1}"));
        }
        if !enrichment.genres.is_empty() {
            meta.genres = enrichment.genres.clone();
        }
    }
    if settings.use_details {
        if enrichment.status.is_some() {
            meta.status = enrichment.status.clone();
        }
        if enrichment.age_rating.is_some() {
            meta.age_rating = enrichment.age_rating.clone();
        }
        if let Some(minutes) = enrichment.runtime {
            meta.runtime = Some(format_runtime(minutes));
        }
        if !enrichment.countries.is_empty() {
            meta.country = Some(enrichment.countries.join(", "));
        }
        if enrichment.language.is_some() {
            meta.language = enrichment.language.clone();
        }
    }
    if settings.use_credits {
        if !enrichment.cast.is_empty() {
            meta.cast = enrichment.cast.clone();
        }
        if !enrichment.director.is_empty() {
            meta.director = enrichment.director.clone();
        }
        if !enrichment.writer.is_empty() {
            meta.writer = enrichment.writer.clone();
        }
    }
    if settings.use_trailers && !enrichment.trailers.is_empty() {
        meta.trailers = enrichment.trailers.clone();
    }
    if settings.use_episodes {
        for video in &mut meta.videos {
            let (Some(season), Some(episode)) = (video.season, video.episode) else { continue };
            let Some(info) = enrichment.episodes.get(&(season, episode)) else { continue };
            if let Some(title) = &info.title {
                video.title = title.clone();
            }
            if let Some(overview) = &info.overview {
                video.overview = Some(overview.clone());
            }
            if let Some(thumbnail) = &info.thumbnail {
                video.thumbnail = Some(thumbnail.clone());
            }
        }
    }
}

fn format_runtime(minutes: i64) -> String {
    if minutes >= 60 {
        format!("{}h {}min", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}min")
    }
}

/// Fills a title's details from TMDB (`enrichMeta`): the id comes from the addon id (`tmdb:<id>`,
/// numeric or IMDb through `/find`), and only the switches of the settings page are applied.
pub async fn enrich(client: &AddonClient, access: &Access, settings: &Settings, meta: &mut MetaDetails, fallback_id: &str) {
    if !settings.enabled {
        return;
    }
    let media = media_of(&meta.content_type);
    let id = match tmdb_id(client, access, media, &meta.id).await {
        Some(id) => id,
        None => match tmdb_id(client, access, media, fallback_id).await {
            Some(id) => id,
            None => return,
        },
    };
    // Episodes are only fetched for the seasons the addon lists.
    let seasons: Vec<i64> = meta.videos.iter().filter_map(|v| v.season).filter(|s| *s > 0).collect::<BTreeSet<_>>().into_iter().collect();
    let Some(enrichment) = fetch_enrichment_for(client, access, media, id, settings.use_episodes, &seasons).await else { return };
    apply_enrichment(meta, &enrichment, settings);
}

/// Same as `fetch_enrichment`, with the addon's seasons for the episode requests.
async fn fetch_enrichment_for(
    client: &AddonClient,
    access: &Access,
    media: MediaType,
    tmdb_id: i64,
    episodes_enabled: bool,
    seasons: &[i64],
) -> Option<Arc<Enrichment>> {
    let key = format!("{}:{}:{tmdb_id}:{episodes_enabled}:{seasons:?}", access.language, media.content_type());
    if let Some(hit) = cache().lock().ok().and_then(|guard| guard.get(&key).cloned()) {
        return Some(hit);
    }
    let media_type = if media == MediaType::Tv { "tv" } else { "movie" };
    let append = if media == MediaType::Tv { "aggregate_credits,images,videos,content_ratings" } else { "credits,images,videos,release_dates" };
    let language_prefix = access.language.split('-').next().unwrap_or("en").to_string();
    let query = vec![
        ("language".to_string(), access.language.clone()),
        ("append_to_response".to_string(), append.to_string()),
        ("include_image_language".to_string(), format!("{language_prefix},en,null")),
    ];
    let payload = client.get_text(&tmdb_url(access, &format!("{media_type}/{tmdb_id}"), &query), false).await.ok()?;
    let mut enrichment = parse_enrichment(&payload, media, &access.language)?;

    if episodes_enabled && media == MediaType::Tv {
        for season in seasons {
            let query = vec![("language".to_string(), access.language.clone())];
            let Ok(payload) = client.get_text(&tmdb_url(access, &format!("tv/{tmdb_id}/season/{season}"), &query), false).await else { continue };
            let Ok(season_body) = serde_json::from_str::<SeasonResponse>(&payload) else { continue };
            for episode in season_body.episodes {
                let Some(number) = episode.episode_number else { continue };
                enrichment.episodes.insert(
                    (*season, number),
                    EpisodeInfo {
                        title: non_blank(episode.name.as_ref()),
                        overview: non_blank(episode.overview.as_ref()),
                        thumbnail: image_url(episode.still_path.as_deref(), "w300"),
                    },
                );
            }
        }
    }

    let enrichment = Arc::new(enrichment);
    if let Ok(mut guard) = cache().lock() {
        if guard.len() >= 64 {
            guard.clear();
        }
        guard.insert(key, enrichment.clone());
    }
    Some(enrichment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvio_core::meta::MetaVideo;

    fn netflix_new_movies() -> Source {
        Source {
            source_type: "DISCOVER".into(),
            media_type: MediaType::Movie,
            sort_by: Some("primary_release_date.desc".into()),
            tmdb_id: None,
            filters: Filters {
                without_genres: Some("99".into()),
                without_keywords: Some("9716".into()),
                watch_region: Some("US".into()),
                with_watch_providers: Some("8|1796".into()),
                ..Default::default()
            },
        }
    }

    fn query_map(query: Vec<(String, String)>) -> std::collections::HashMap<String, String> {
        query.into_iter().collect()
    }

    #[test]
    fn discover_query_matches_the_original_for_the_netflix_sources() {
        let query = query_map(discover_query(&netflix_new_movies(), 2, "en-US"));
        assert_eq!(query["language"], "en-US");
        assert_eq!(query["page"], "2");
        assert_eq!(query["sort_by"], "primary_release_date.desc");
        assert_eq!(query["with_watch_providers"], "8|1796");
        assert!(!query.contains_key("without_watch_providers"));
        assert_eq!(query["watch_region"], "US");
        assert_eq!(query["with_watch_monetization_types"], "flatrate|free|ads|rent|buy");
        assert_eq!(query["without_genres"], "99");
        assert_eq!(query["without_keywords"], "9716");
        assert!(!query.contains_key("with_genres"));
    }

    #[test]
    fn tv_sources_use_first_air_date_and_swap_the_sort() {
        let mut source = netflix_new_movies();
        source.media_type = MediaType::Tv;
        source.sort_by = Some("primary_release_date.desc".into());
        source.filters.year = Some(2024);
        source.filters.release_date_gte = Some("2020-01-01".into());
        let query = query_map(discover_query(&source, 1, "en"));
        assert_eq!(query["sort_by"], "first_air_date.desc");
        assert_eq!(query["first_air_date_year"], "2024");
        assert_eq!(query["first_air_date.gte"], "2020-01-01");
        assert!(!query.contains_key("year"));
        assert!(!query.contains_key("primary_release_date.gte"));
    }

    #[test]
    fn company_network_and_original_sort() {
        let mut source = netflix_new_movies();
        source.source_type = "NETWORK".into();
        source.tmdb_id = Some(213);
        source.sort_by = None;
        let query = query_map(discover_query(&source, 1, "en"));
        assert_eq!(query["with_networks"], "213");
        assert_eq!(query["sort_by"], "popularity.desc");

        source.source_type = "COMPANY".into();
        source.tmdb_id = Some(420);
        let query = query_map(discover_query(&source, 1, "en"));
        assert_eq!(query["with_companies"], "420");
        assert!(!query.contains_key("with_networks"));
    }

    #[test]
    fn ratings_and_images_follow_the_original_mapping() {
        let payload = r#"{"page":1,"total_pages":2,"results":[
            {"id":123,"title":"Filme","poster_path":"/p.jpg","backdrop_path":"/b.jpg","overview":"Sinopse",
             "release_date":"2024-05-01","vote_average":7.26},
            {"id":456,"name":"Série","first_air_date":"2021-02-03","vote_average":8.0},
            {"id":123,"title":"Duplicado"},
            {"id":789}
        ]}"#;
        let page = parse_page(payload, 1, MediaType::Movie).unwrap();
        assert_eq!(page.items.len(), 2, "no title and duplicate id dropped");
        let first = &page.items[0];
        assert_eq!((first.id.as_str(), first.name.as_str()), ("tmdb:123", "Filme"));
        assert_eq!(first.content_type, "movie");
        assert_eq!(first.poster.as_deref(), Some("https://image.tmdb.org/t/p/w500/p.jpg"));
        assert_eq!(first.banner.as_deref(), Some("https://image.tmdb.org/t/p/w1280/b.jpg"));
        assert_eq!(first.release_info.as_deref(), Some("2024"));
        assert_eq!(first.raw_release_date.as_deref(), Some("2024-05-01"));
        assert_eq!(first.imdb_rating.as_deref(), Some("7.3"));
        assert_eq!(first.description.as_deref(), Some("Sinopse"));
        assert_eq!(page.next_skip, Some(2));

        let page = parse_page(payload, 1, MediaType::Tv).unwrap();
        let series = &page.items[1];
        assert_eq!(series.content_type, "series");
        assert_eq!(series.release_info.as_deref(), Some("2021"));
        assert_eq!(series.imdb_rating.as_deref(), Some("8.0"));
        assert_eq!(series.poster, None, "no poster and no backdrop");
    }

    #[test]
    fn the_last_page_has_no_next_skip() {
        let payload = r#"{"page":3,"total_pages":3,"results":[{"id":1,"title":"A"}]}"#;
        assert_eq!(parse_page(payload, 3, MediaType::Movie).unwrap().next_skip, None);
        let empty = r#"{"page":1,"total_pages":5,"results":[]}"#;
        assert_eq!(parse_page(empty, 1, MediaType::Movie).unwrap().next_skip, None);
    }

    #[test]
    fn lists_sort_dedupe_and_page_like_the_original() {
        let payload = r#"{"page":1,"total_pages":2,"items":[
            {"id":1,"media_type":"movie","title":"B","release_date":"2020-01-01","vote_average":7.0,"vote_count":10},
            {"id":2,"media_type":"tv","name":"Série","first_air_date":"2019-01-01","vote_average":9.0,"vote_count":5},
            {"id":1,"media_type":"movie","title":"B duplicado","vote_average":8.0}]}"#;
        let by_rating = parse_list(payload, 1, Some("vote_average.desc")).unwrap();
        assert_eq!(by_rating.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["tmdb:2", "tmdb:1"]);
        assert_eq!(by_rating.items[0].content_type, "series");
        assert_eq!(by_rating.items[0].release_info.as_deref(), Some("2019"));
        assert_eq!(by_rating.next_skip, Some(2), "page 1 of 2");

        let by_votes = parse_list(payload, 1, Some("vote_count.desc")).unwrap();
        assert_eq!(by_votes.items[0].id, "tmdb:1", "10 votes before 5");

        let last = r#"{"page":2,"total_pages":2,"items":[{"id":1,"title":"A"}]}"#;
        assert_eq!(parse_list(last, 2, None).unwrap().next_skip, None);
    }

    #[test]
    fn collections_use_their_parts_without_paging() {
        let payload = r#"{"id":14890,"name":"Bad Boys","parts":[
            {"id":3,"title":"Bad Boys for Life","release_date":"2020-01-17","vote_average":6.5},
            {"id":1,"title":"Bad Boys","release_date":"1995-04-07","vote_average":6.8}]}"#;
        let page = parse_collection(payload, Some("vote_average.desc")).unwrap();
        assert_eq!(page.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["tmdb:1", "tmdb:3"]);
        assert_eq!(page.items[0].content_type, "movie");
        assert_eq!(page.next_skip, None);
    }

    #[test]
    fn person_credits_filter_by_media_and_job() {
        let payload = r#"{
            "cast":[
                {"id":1,"media_type":"movie","title":"Filme","release_date":"2001-01-01","vote_average":7.0,"vote_count":100},
                {"id":2,"media_type":"tv","name":"Série","first_air_date":"2002-01-01","vote_average":8.0,"vote_count":200},
                {"id":1,"media_type":"movie","title":"Filme repetido","vote_average":9.0}],
            "crew":[
                {"id":3,"media_type":"movie","title":"Dirigido","job":"Director","vote_average":6.0},
                {"id":4,"media_type":"movie","title":"Escrito","job":"Writer","vote_average":5.0},
                {"id":5,"media_type":"tv","name":"Série dirigida","job":"Director","vote_average":7.0}]}"#;
        let movies = parse_person(payload, MediaType::Movie, false, Some("vote_count.desc")).unwrap();
        assert_eq!(movies.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["tmdb:1"], "movie cast, deduplicated");
        let series = parse_person(payload, MediaType::Tv, false, None).unwrap();
        assert_eq!(series.items.len(), 1);
        assert_eq!(series.items[0].content_type, "series");
        let directed = parse_person(payload, MediaType::Movie, true, Some("original")).unwrap();
        assert_eq!(directed.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["tmdb:3"]);
        assert_eq!(directed.next_skip, None);
    }

    #[test]
    fn recommendations_keep_the_item_media_type_and_stop_at_twelve() {
        let mut results = String::new();
        for id in 1..=13 {
            let kind = if id % 3 == 0 { r#""media_type":"tv","name":"Série","# } else { "" };
            if !results.is_empty() {
                results.push(',');
            }
            results.push_str(&format!(r#"{{"id":{id},{kind}"title":"Filme {id}","vote_average":7.0}}"#));
        }
        let payload = format!(r#"{{"page":1,"total_pages":1,"results":[{results}]}}"#);
        let items = parse_recommendations(&payload, MediaType::Movie);
        assert_eq!(items.len(), 12, "at most twelve, as the original");
        assert_eq!(items[2].content_type, "series", "the answer's own media type wins");
        assert_eq!(items[0].id, "tmdb:1");
        assert_eq!(items[0].content_type, "movie");
        assert!(parse_recommendations("{\"results\":[]}", MediaType::Tv).is_empty());
        assert!(parse_recommendations("not json", MediaType::Tv).is_empty());
    }

    fn meta() -> MetaDetails {
        MetaDetails {
            id: "tt1".into(),
            content_type: "movie".into(),
            name: "Addon title".into(),
            poster: Some("old-poster".into()),
            background: Some("old-background".into()),
            logo: None,
            description: Some("old overview".into()),
            release_info: Some("2020".into()),
            imdb_rating: None,
            age_rating: None,
            status: None,
            runtime: None,
            genres: vec!["Old".into()],
            director: Vec::new(),
            writer: Vec::new(),
            cast: Vec::new(),
            country: None,
            awards: None,
            language: None,
            website: None,
            default_video_id: None,
            videos: Vec::new(),
            trailers: Vec::new(),
        }
    }

    const MOVIE_ENRICHMENT: &str = r#"{
        "title":"Localized title","overview":"TMDB overview","poster_path":"/p.jpg","backdrop_path":"/b.jpg",
        "vote_average":8.14,"runtime":109,"status":"Released",
        "production_countries":[{"iso_3166_1":"US"}],"spoken_languages":[{"english_name":"English"}],
        "genres":[{"name":"Drama"}],
        "credits":{"cast":[{"name":"Actor","character":"Role","profile_path":"/a.jpg"}],
                   "crew":[{"name":"Director","job":"Director"},{"name":"Writer","job":"Screenplay"}]},
        "images":{"logos":[{"file_path":"/l.png","iso_639_1":"en","vote_average":5.0}]},
        "videos":{"results":[{"key":"abc","name":"Official Trailer","site":"YouTube","type":"Trailer","official":true}]},
        "release_dates":{"results":[{"iso_3166_1":"US","release_dates":[{"certification":"PG-13"}]}]}}"#;

    const TV_ENRICHMENT: &str = r#"{
        "name":"Show","overview":"Show overview","first_air_date":"2021-01-01","vote_average":7.0,
        "episode_run_time":[45],"status":"Returning Series","origin_country":["GB"],
        "spoken_languages":[{"name":"English"}],"genres":[{"name":"Mystery"}],
        "aggregate_credits":{"cast":[{"name":"Lead","roles":[{"character":"Detective"}],"profile_path":null}],
                             "crew":[{"name":"Director","jobs":[{"job":"Director"}]},{"name":"Writer","jobs":[{"job":"Writer"}]}]},
        "videos":{"results":[]},
        "content_ratings":{"results":[{"iso_3166_1":"US","rating":"TV-14"}]}}"#;

    #[test]
    fn enrichment_maps_a_movie_the_way_the_original_does() {
        let enrichment = parse_enrichment(MOVIE_ENRICHMENT, MediaType::Movie, "en-US").unwrap();
        assert_eq!(enrichment.title.as_deref(), Some("Localized title"));
        assert_eq!(enrichment.poster.as_deref(), Some("https://image.tmdb.org/t/p/w500/p.jpg"));
        assert_eq!(enrichment.logo.as_deref(), Some("https://image.tmdb.org/t/p/w500/l.png"));
        assert_eq!(enrichment.rating, Some(8.14));
        assert_eq!(enrichment.runtime, Some(109));
        assert_eq!(enrichment.countries, ["US"]);
        assert_eq!(enrichment.language.as_deref(), Some("English"));
        assert_eq!(enrichment.age_rating.as_deref(), Some("PG-13"));
        assert_eq!(enrichment.director, ["Director"]);
        assert_eq!(enrichment.writer, ["Writer"]);
        assert_eq!(enrichment.cast.len(), 1);
        assert_eq!(enrichment.cast[0].role.as_deref(), Some("Role"));
        assert_eq!(enrichment.trailers[0].key, "abc");

        let mut details = meta();
        apply_enrichment(&mut details, &enrichment, &Settings::default());
        assert_eq!(details.name, "Localized title");
        assert_eq!(details.description.as_deref(), Some("TMDB overview"));
        assert_eq!(details.poster.as_deref(), Some("https://image.tmdb.org/t/p/w500/p.jpg"));
        assert_eq!(details.logo.as_deref(), Some("https://image.tmdb.org/t/p/w500/l.png"));
        assert_eq!(details.imdb_rating.as_deref(), Some("8.1"));
        assert_eq!(details.genres, ["Drama"]);
        assert_eq!(details.age_rating.as_deref(), Some("PG-13"));
        assert_eq!(details.runtime.as_deref(), Some("1h 49min"));
        assert_eq!(details.country.as_deref(), Some("US"));
        assert_eq!(details.language.as_deref(), Some("English"));
        assert_eq!(details.director, ["Director"]);
        assert_eq!(details.writer, ["Writer"]);
        assert_eq!(details.cast[0].name, "Actor");
        assert_eq!(details.poster, enrichment.poster);
    }

    #[test]
    fn modules_that_are_off_leave_the_addon_data_alone() {
        let enrichment = parse_enrichment(MOVIE_ENRICHMENT, MediaType::Movie, "en-US").unwrap();
        let settings = Settings {
            use_artwork: false,
            use_basic_info: false,
            use_details: false,
            use_credits: false,
            use_trailers: false,
            use_episodes: false,
            ..Default::default()
        };
        let mut details = meta();
        let before = details.clone();
        apply_enrichment(&mut details, &enrichment, &settings);
        assert_eq!(details, before);
    }

    #[test]
    fn tv_uses_aggregate_credits_and_content_ratings() {
        let enrichment = parse_enrichment(TV_ENRICHMENT, MediaType::Tv, "en-US").unwrap();
        assert_eq!(enrichment.title.as_deref(), Some("Show"));
        assert_eq!(enrichment.runtime, Some(45));
        assert_eq!(enrichment.countries, ["GB"], "origin country when no production countries");
        assert_eq!(enrichment.age_rating.as_deref(), Some("TV-14"));
        assert_eq!(enrichment.cast[0].role.as_deref(), Some("Detective"));
        assert_eq!(enrichment.director, ["Director"]);
        assert_eq!(enrichment.writer, ["Writer"]);
    }

    #[test]
    fn episodes_fill_the_addons_episode_list() {
        let mut details = meta();
        details.content_type = "series".into();
        details.videos = vec![MetaVideo {
            id: "tt1:1:2".into(),
            title: "Addon episode".into(),
            released: None,
            available: true,
            thumbnail: None,
            season: Some(1),
            episode: Some(2),
            overview: None,
        }];
        let mut enrichment = Enrichment::default();
        enrichment.episodes.insert(
            (1, 2),
            EpisodeInfo { title: Some("TMDB episode".into()), overview: Some("Episode overview".into()), thumbnail: Some("https://image.tmdb.org/t/p/w300/e.jpg".into()) },
        );
        apply_enrichment(&mut details, &enrichment, &Settings::default());
        assert_eq!(details.videos[0].title, "TMDB episode");
        assert_eq!(details.videos[0].overview.as_deref(), Some("Episode overview"));
        assert_eq!(details.videos[0].thumbnail.as_deref(), Some("https://image.tmdb.org/t/p/w300/e.jpg"));
    }

    #[test]
    fn settings_fall_back_to_the_original_app_and_respect_the_switches() {
        let legacy = Access { api_key: "legacy-key".into(), language: "pt-BR".into() };
        assert!(resolve(Settings::default(), None, None).is_none(), "no key anywhere");
        let access = resolve(Settings::default(), None, Some(legacy.clone())).expect("legacy key");
        assert_eq!(access.api_key, "legacy-key");
        assert_eq!(access.language, "pt-BR");

        let own = Settings { api_key: "own-key".into(), language: "ar".into(), ..Default::default() };
        let access = resolve(own, None, Some(legacy.clone())).unwrap();
        assert_eq!(access.api_key, "own-key");
        assert_eq!(access.language, "ar", "own language wins");

        // The Nuvio account's key (provider credentials) comes before the original app's.
        let access = resolve(Settings::default(), Some("account-key".into()), Some(legacy.clone())).unwrap();
        assert_eq!(access.api_key, "account-key");

        let disabled = Settings { enabled: false, ..Default::default() };
        assert!(resolve(disabled, None, Some(legacy.clone())).is_none(), "disabled means no requests");
    }

    #[test]
    fn settings_are_saved_per_profile_without_exposing_the_key() {
        let dir = std::env::temp_dir().join(format!("nuvio-tmdb-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = AppStore::open(&dir).unwrap();
        let view = update(&store, 1, SettingsInput {
            enabled: true,
            api_key: Some("  my-key  ".into()),
            language: "en-US".into(),
            use_more_like_this: false,
            use_collections: true,
            ..Default::default()
        })
        .unwrap();
        assert!(view.api_key_set);
        assert!(!view.api_key_from_original, "Astra's own key is in use");
        assert!(!view.use_more_like_this);
        assert_eq!(settings(&store, 1).api_key, "my-key");
        assert_eq!(settings(&store, 2), Settings::default(), "other profiles untouched");
        // Keeping the key when the field is not edited.
        let view = update(&store, 1, SettingsInput { api_key: None, ..Default::default() }).unwrap();
        assert!(view.api_key_set);
        assert_eq!(settings(&store, 1).api_key, "my-key");
        // Clearing it falls back to the original app (none in this isolated directory).
        let view = update(&store, 1, SettingsInput { api_key: Some(String::new()), ..Default::default() }).unwrap();
        assert_eq!(settings(&store, 1).api_key, "", "Astra's key cleared");
        // The original app's key (when this machine has one) is the fallback again.
        assert_eq!(view.api_key_from_original, legacy_access(1).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn series_content_type_maps_to_tv() {
        assert_eq!(media_of("series"), MediaType::Tv);
        assert_eq!(media_of(" movie "), MediaType::Movie);
        assert_eq!(media_of("anime"), MediaType::Movie);
    }

    #[test]
    fn supported_source_types() {
        for kind in ["DISCOVER", "network", "COMPANY", "LIST", "COLLECTION", "PERSON", "DIRECTOR"] {
            assert!(supported(kind), "{kind}");
        }
        assert!(!supported("TRAKT"));
    }
}
