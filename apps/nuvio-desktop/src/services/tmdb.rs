//! TMDB (Settings → TMDB; `TmdbMetadataService`, `TmdbService`, `TmdbCollectionSourceResolver`
//! and `TmdbEntityBrowseScreen` of the original).
//!
//! With a key of the user's own (kept encrypted by `crate::tmdb`), the details of a title are
//! completed from `api.themoviedb.org`: artwork, localized title and synopsis, genres, age rating,
//! cast with photos, studios and networks, episode titles/pictures, season posters, "More like
//! this", the movie's collection and the trailers. Each part follows its switch in the settings,
//! as `applyEnrichment` does. Titles that only TMDB knows (`tmdb:<id>`) get a page of their own
//! (`fetchStandaloneMeta`), and collection folders may list TMDB sources (discover, company,
//! network, list, collection, person, director).
//!
//! The key goes only to TMDB; it is never part of an error text, a log or an event.

use nuvio_core::catalog::{CatalogPage, MetaPreview, PosterShape};
use nuvio_core::meta::{MetaDetails, MetaPerson, MetaTrailer, MetaVideo};
use nuvio_store::settings::TmdbSettings;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// TMDB API root; tests point it at a local fake server.
fn api_root() -> &'static str {
    option_env!("NUVIO_TMDB_API_URL").unwrap_or("https://api.themoviedb.org/3")
}

const IMAGES: &str = "https://image.tmdb.org/t/p/";
const CACHE_FOR: Duration = Duration::from_secs(6 * 3600);
const CACHE_MAX: usize = 600;

// ---------------------------------------------------------------- access and requests

/// Key and language of one request set.
#[derive(Clone)]
pub struct Access {
    key: String,
    pub language: String,
}

impl Access {
    pub fn new(key: String, language: String) -> Self {
        Self { key, language: normalize_language(Some(&language)) }
    }

    /// The key itself (scraper plugins get it, like in the original).
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl std::fmt::Debug for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Access").field("language", &self.language).finish_non_exhaustive()
    }
}

struct Client {
    http: reqwest::Client,
    cache: Mutex<HashMap<String, (Instant, Arc<str>)>>,
}

fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| Client {
        http: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(25))
            .user_agent(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("tmdb http client"),
        cache: Mutex::new(HashMap::new()),
    })
}

/// A v4 "read access token" (a JWT) goes in the header; a v3 key in the address.
fn is_token(key: &str) -> bool {
    key.starts_with("eyJ") && key.matches('.').count() == 2
}

/// `GET {root}/{endpoint}?{query}`: the text of a 2xx answer, cached for a few hours.
async fn get(access: &Access, endpoint: &str, query: &[(&str, String)]) -> Result<Arc<str>, String> {
    let mut url = reqwest::Url::parse(&format!("{}/{}", api_root(), endpoint.trim_start_matches('/'))).map_err(|_| "Endereço TMDB inválido.".to_string())?;
    {
        let mut pairs = url.query_pairs_mut();
        for (name, value) in query {
            if !value.trim().is_empty() {
                pairs.append_pair(name, value);
            }
        }
    }
    // The cache is keyed by the address without the key, plus a fingerprint of the key.
    let cache_key = format!("{}|{}", &nuvio_sync::crypto::sha256_hex(&access.key)[..10], url);
    let c = client();
    if let Some((at, text)) = c.cache.lock().expect("tmdb cache").get(&cache_key) {
        if at.elapsed() < CACHE_FOR {
            return Ok(text.clone());
        }
    }
    let mut request = c.http.get(url.clone()).header("Accept", "application/json");
    if is_token(&access.key) {
        request = request.bearer_auth(&access.key);
    } else {
        url.query_pairs_mut().append_pair("api_key", &access.key);
        request = c.http.get(url).header("Accept", "application/json");
    }
    let response = request.send().await.map_err(|e| if e.is_timeout() { "O TMDB demorou demais para responder.".to_string() } else { "Sem conexão com o TMDB.".to_string() })?;
    let status = response.status().as_u16();
    if status == 401 {
        return Err("Chave do TMDB inválida.".into());
    }
    if status == 404 {
        return Err("Não encontrado no TMDB.".into());
    }
    if !(200..300).contains(&status) {
        return Err(format!("O TMDB respondeu com o código {status}."));
    }
    let text: Arc<str> = Arc::from(response.text().await.map_err(|_| "Resposta do TMDB incompleta.".to_string())?);
    let mut cache = c.cache.lock().expect("tmdb cache");
    if cache.len() >= CACHE_MAX {
        cache.retain(|_, (at, _)| at.elapsed() < CACHE_FOR);
        if cache.len() >= CACHE_MAX {
            cache.clear();
        }
    }
    cache.insert(cache_key, (Instant::now(), text.clone()));
    Ok(text)
}

async fn get_json<T: for<'de> Deserialize<'de>>(access: &Access, endpoint: &str, query: &[(&str, String)]) -> Result<T, String> {
    let text = get(access, endpoint, query).await?;
    serde_json::from_str(&text).map_err(|_| "Resposta do TMDB em formato inesperado.".to_string())
}

/// Checks a key (Settings → TMDB → Salvar): `GET /configuration`.
pub async fn check_key(key: &str) -> Result<(), String> {
    let access = Access::new(key.trim().to_string(), "en".into());
    get(&access, "configuration", &[]).await.map(|_| ())
}

// ---------------------------------------------------------------- small helpers

/// `normalizeTmdbLanguage`: `xx-YY`, `en` when unset, `es-419` as `es-MX`.
pub fn normalize_language(value: Option<&str>) -> String {
    let raw = match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => v.replace('_', "-"),
        None => return "en".into(),
    };
    let parts: Vec<&str> = raw.split('-').collect();
    let normalized = if parts.len() == 2 { format!("{}-{}", parts[0].to_lowercase(), parts[1].to_uppercase()) } else { raw.to_lowercase() };
    if normalized == "es-419" { "es-MX".into() } else { normalized }
}

/// The language used when the setting follows the app: its interface language (`pt` is Brazil's
/// Portuguese, `pt-PT` Portugal's), with the region TMDB uses for it.
pub fn app_language_tag(ui: &str) -> String {
    match ui {
        "pt" | "pt-BR" => "pt-BR".into(),
        "pt-PT" => "pt-PT".into(),
        "en" => "en-US".into(),
        "ar" => "ar-SA".into(),
        "es" => "es-ES".into(),
        "zh" => "zh-CN".into(),
        "nb" => "no-NO".into(),
        other => {
            let base = other.split('-').next().unwrap_or("en").to_lowercase();
            let region = DEFAULT_REGIONS.iter().find(|(c, _)| *c == base).map(|(_, r)| *r);
            match region {
                Some(r) => format!("{base}-{r}"),
                None if base.len() == 2 => base,
                None => "en-US".into(),
            }
        }
    }
}

fn image(path: Option<&str>, size: &str) -> Option<String> {
    let path = path.map(str::trim).filter(|p| !p.is_empty())?;
    Some(format!("{IMAGES}{size}{path}"))
}

fn text(value: &Option<String>) -> Option<String> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

/// `formatRating`: one decimal.
fn rating(value: f64) -> String {
    format!("{:.1}", (value * 10.0).round() / 10.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Movie,
    Tv,
}

impl MediaType {
    /// "tv"/"series" (and the original's other names for shows) is TV, anything else a movie.
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_lowercase()).as_deref() {
            Some("tv") | Some("series") | Some("show") | Some("tvshow") => Self::Tv,
            _ => Self::Movie,
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "tv",
        }
    }
    /// The app's content type.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "series",
        }
    }
}

const DEFAULT_REGIONS: &[(&str, &str)] = &[
    ("ar", "SA"), ("bg", "BG"), ("bs", "BA"), ("cs", "CZ"), ("da", "DK"), ("de", "DE"), ("el", "GR"), ("es", "ES"), ("et", "EE"), ("fi", "FI"),
    ("fr", "FR"), ("he", "IL"), ("hi", "IN"), ("hr", "HR"), ("hu", "HU"), ("id", "ID"), ("it", "IT"), ("ja", "JP"), ("ko", "KR"), ("lt", "LT"),
    ("lv", "LV"), ("nb", "NO"), ("nl", "NL"), ("no", "NO"), ("pl", "PL"), ("pt", "PT"), ("ro", "RO"), ("ru", "RU"), ("sk", "SK"), ("sl", "SI"),
    ("sr", "RS"), ("sv", "SE"), ("th", "TH"), ("tr", "TR"), ("uk", "UA"), ("ur", "PK"), ("vi", "VN"), ("zh", "CN"),
];

fn split_language(language: &str) -> (String, Option<String>) {
    let code = language.split('-').next().unwrap_or("").to_lowercase();
    let region = language.split_once('-').map(|(_, r)| r.to_uppercase()).filter(|r| r.len() == 2);
    (code, region)
}

/// `preferredAgeRatingRegions`: the language's region, then US and GB.
fn age_regions(language: &str) -> Vec<String> {
    let (code, region) = split_language(language);
    let region = region.or_else(|| DEFAULT_REGIONS.iter().find(|(c, _)| *c == code).map(|(_, r)| r.to_string()));
    let mut out: Vec<String> = region.into_iter().collect();
    for r in ["US", "GB"] {
        if !out.iter().any(|x| x == r) {
            out.push(r.into());
        }
    }
    out
}

/// `tmdbImageLanguages`.
fn image_languages(language: &str) -> String {
    let (code, _) = split_language(language);
    let mut out: Vec<String> = Vec::new();
    for v in [code, language.to_string(), "en".into(), "null".into()] {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out.join(",")
}

// ---------------------------------------------------------------- TMDB answers

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Named {
    id: Option<i64>,
    name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct CompanyJson {
    id: Option<i64>,
    name: Option<String>,
    logo_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Country {
    iso_3166_1: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PersonRef {
    id: Option<i64>,
    name: Option<String>,
    profile_path: Option<String>,
    character: Option<String>,
    job: Option<String>,
    roles: Vec<RoleJson>,
    jobs: Vec<JobJson>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RoleJson {
    character: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct JobJson {
    job: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct DetailsJson {
    id: i64,
    title: Option<String>,
    name: Option<String>,
    original_title: Option<String>,
    original_name: Option<String>,
    overview: Option<String>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    last_air_date: Option<String>,
    status: Option<String>,
    vote_average: Option<f64>,
    runtime: Option<i64>,
    episode_run_time: Vec<i64>,
    production_countries: Vec<Country>,
    origin_country: Vec<String>,
    original_language: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    created_by: Vec<PersonRef>,
    genres: Vec<Named>,
    production_companies: Vec<CompanyJson>,
    networks: Vec<CompanyJson>,
    belongs_to_collection: Option<Named>,
    number_of_seasons: Option<i64>,
    imdb_id: Option<String>,
    external_ids: Option<ExternalIds>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ExternalIds {
    imdb_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct CreditsJson {
    cast: Vec<PersonRef>,
    crew: Vec<PersonRef>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ImageJson {
    file_path: Option<String>,
    iso_639_1: Option<String>,
    iso_3166_1: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ImagesJson {
    logos: Vec<ImageJson>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ReleaseDatesJson {
    results: Vec<ReleaseCountry>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ReleaseCountry {
    iso_3166_1: Option<String>,
    release_dates: Vec<Certification>,
    rating: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Certification {
    certification: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct VideosJson {
    results: Vec<VideoJson>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct VideoJson {
    id: Option<String>,
    key: Option<String>,
    name: Option<String>,
    site: Option<String>,
    size: Option<i64>,
    #[serde(rename = "type")]
    kind: Option<String>,
    official: Option<bool>,
    published_at: Option<String>,
    iso_639_1: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct SeasonJson {
    poster_path: Option<String>,
    episodes: Vec<EpisodeJson>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct EpisodeJson {
    episode_number: Option<i64>,
    season_number: Option<i64>,
    name: Option<String>,
    overview: Option<String>,
    still_path: Option<String>,
    air_date: Option<String>,
    runtime: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct FindJson {
    movie_results: Vec<Named>,
    tv_results: Vec<Named>,
}

/// `TmdbListItem` and its siblings: a discover/recommendation result, a list/collection item or a
/// person credit.
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
struct ItemJson {
    id: i64,
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
    vote_count: Option<i64>,
    popularity: Option<f64>,
    job: Option<String>,
    character: Option<String>,
    genre_ids: Vec<i64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PageJson {
    page: Option<u32>,
    total_pages: Option<u32>,
    results: Vec<ItemJson>,
    items: Vec<ItemJson>,
    parts: Vec<ItemJson>,
    cast: Vec<ItemJson>,
    crew: Vec<ItemJson>,
    name: Option<String>,
}

impl ItemJson {
    fn media(&self, fallback: MediaType) -> MediaType {
        match self.media_type.as_deref().map(str::to_lowercase).as_deref() {
            Some("tv") => MediaType::Tv,
            Some("movie") => MediaType::Movie,
            _ => fallback,
        }
    }

    fn title(&self) -> Option<String> {
        text(&self.title).or_else(|| text(&self.name)).or_else(|| text(&self.original_title)).or_else(|| text(&self.original_name))
    }

    /// `toPreview`: `tmdb:<id>`, the year as release info, the vote average with one decimal.
    fn preview(&self, media: MediaType, shape: PosterShape) -> Option<MetaPreview> {
        if self.id <= 0 {
            return None;
        }
        let name = self.title()?;
        let date = match media {
            MediaType::Movie => text(&self.release_date).or_else(|| text(&self.first_air_date)),
            MediaType::Tv => text(&self.first_air_date).or_else(|| text(&self.release_date)),
        };
        let (poster, banner) = match shape {
            PosterShape::Landscape => (image(self.backdrop_path.as_deref(), "w780").or_else(|| image(self.poster_path.as_deref(), "w500")), image(self.backdrop_path.as_deref(), "w1280")),
            _ => (image(self.poster_path.as_deref(), "w500").or_else(|| image(self.backdrop_path.as_deref(), "w780")), image(self.backdrop_path.as_deref(), "w1280")),
        };
        Some(MetaPreview {
            id: format!("tmdb:{}", self.id),
            content_type: media.content_type().into(),
            name,
            poster,
            banner,
            logo: None,
            poster_shape: shape,
            description: text(&self.overview),
            release_info: date.as_ref().map(|d| d.chars().take(4).collect::<String>()).filter(|y| !y.is_empty()),
            raw_release_date: date,
            imdb_rating: self.vote_average.filter(|v| *v > 0.0).map(rating),
            genres: Vec::new(),
            landscape_poster: None,
            runtime: None,
            status: None,
        })
    }
}

fn dedupe(items: Vec<MetaPreview>) -> Vec<MetaPreview> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|i| seen.insert(i.stable_key())).collect()
}

// ---------------------------------------------------------------- ids

/// `ensureTmdbId`: `tmdb:<id>`, a bare number, or an IMDb id resolved through `/find`.
pub async fn tmdb_id(access: &Access, media: MediaType, id: &str, imdb_fallback: Option<&str>) -> Option<i64> {
    let normalized = id.trim();
    let normalized = normalized.strip_prefix("tmdb:").unwrap_or(normalized);
    let head = normalized.split([':', '/']).next().unwrap_or("").trim();
    if !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()) {
        return head.parse().ok();
    }
    let imdb = if head.to_lowercase().starts_with("tt") { Some(head.to_string()) } else { imdb_fallback.map(|v| v.split(':').next().unwrap_or("").to_string()).filter(|v| v.starts_with("tt")) }?;
    let found: FindJson = get_json(access, &format!("find/{imdb}"), &[("external_source", "imdb_id".into())]).await.ok()?;
    let list = match media {
        MediaType::Movie => &found.movie_results,
        MediaType::Tv => &found.tv_results,
    };
    list.first().and_then(|n| n.id)
}

/// `tmdbToImdb`: the IMDb id of a TMDB title (stream addons mostly know only those).
pub async fn imdb_id(access: &Access, media: MediaType, tmdb: i64) -> Option<String> {
    let ids: ExternalIds = get_json(access, &format!("{}/{tmdb}/external_ids", media.path()), &[]).await.ok()?;
    text(&ids.imdb_id).filter(|v| v.starts_with("tt"))
}

/// The TMDB id written in a title id (`tmdb:<id>`, also `tmdb:<id>:<s>:<e>`).
pub fn tmdb_prefix_id(id: &str) -> Option<i64> {
    id.trim().strip_prefix("tmdb:")?.split(':').next()?.trim().parse().ok()
}

// ---------------------------------------------------------------- enrichment

/// A studio or a network (`MetaCompany`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub id: Option<i64>,
    pub name: String,
    pub logo: Option<String>,
}

/// What TMDB adds to the details page besides the fields of the title itself.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Extras {
    pub tmdb_id: Option<i64>,
    pub more_like_this: Vec<MetaPreview>,
    pub collection_id: Option<i64>,
    pub collection_name: Option<String>,
    pub collection: Vec<MetaPreview>,
    pub companies: Vec<Company>,
    pub networks: Vec<Company>,
}

/// `TmdbEnrichment`.
#[derive(Debug, Clone, Default)]
struct Enrichment {
    title: Option<String>,
    description: Option<String>,
    genres: Vec<String>,
    backdrop: Option<String>,
    logo: Option<String>,
    poster: Option<String>,
    people: Vec<MetaPerson>,
    director: Vec<String>,
    writer: Vec<String>,
    release_date: Option<String>,
    last_air_date: Option<String>,
    rating: Option<f64>,
    runtime_minutes: Option<i64>,
    age_rating: Option<String>,
    status: Option<String>,
    countries: Vec<String>,
    language: Option<String>,
    companies: Vec<Company>,
    networks: Vec<Company>,
    collection_id: Option<i64>,
    collection_name: Option<String>,
    collection: Vec<MetaPreview>,
    more_like_this: Vec<MetaPreview>,
    trailers: Vec<MetaTrailer>,
    seasons: i64,
    imdb_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct EpisodeExtra {
    title: Option<String>,
    overview: Option<String>,
    thumbnail: Option<String>,
    season_poster: Option<String>,
    air_date: Option<String>,
}

fn company(c: &CompanyJson) -> Option<Company> {
    Some(Company { id: c.id, name: text(&c.name)?, logo: image(c.logo_path.as_deref(), "w300") })
}

fn person(p: &PersonRef, role: Option<String>) -> Option<MetaPerson> {
    Some(MetaPerson { name: text(&p.name)?, role, photo: image(p.profile_path.as_deref(), "w185"), tmdb_id: p.id })
}

fn is_writer(job: Option<&str>) -> bool {
    let job = job.unwrap_or("").to_lowercase();
    job.contains("writer") || job.contains("screenplay")
}

fn job_of(p: &PersonRef) -> Option<String> {
    p.job.clone().or_else(|| p.jobs.first().and_then(|j| j.job.clone()))
}

/// `buildPeople`: the creators (series) or directors (movies) — writers when there are none —
/// then the cast, without repeats.
fn people(details: &DetailsJson, credits: &CreditsJson, media: MediaType) -> Vec<MetaPerson> {
    let creators: Vec<MetaPerson> = if media == MediaType::Tv { details.created_by.iter().filter_map(|c| person(c, Some("Criação".into()))).collect() } else { Vec::new() };
    let directors: Vec<MetaPerson> = credits.crew.iter().filter(|c| job_of(c).is_some_and(|j| j.eq_ignore_ascii_case("director"))).filter_map(|c| person(c, Some("Direção".into()))).collect();
    let writers: Vec<MetaPerson> = credits.crew.iter().filter(|c| is_writer(job_of(c).as_deref())).filter_map(|c| person(c, Some("Roteiro".into()))).collect();
    let cast = credits.cast.iter().filter_map(|c| {
        let role = text(&c.character).or_else(|| c.roles.first().and_then(|r| text(&r.character)));
        person(c, role)
    });
    let primary = if media == MediaType::Tv && !creators.is_empty() {
        creators
    } else if media == MediaType::Movie && !directors.is_empty() {
        directors
    } else {
        writers
    };
    let mut seen = HashSet::new();
    primary.into_iter().chain(cast).filter(|p| seen.insert(format!("{}|{}", p.name.to_lowercase(), p.role.clone().unwrap_or_default().to_lowercase()))).take(40).collect()
}

fn distinct(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    names.filter(|n| seen.insert(n.clone())).collect()
}

/// `selectBestLocalizedImagePath`.
fn best_logo(images: &ImagesJson, language: &str) -> Option<String> {
    let (code, region) = split_language(language);
    let region = region.or_else(|| match code.as_str() {
        "pt" => Some("PT".into()),
        "es" => Some("ES".into()),
        _ => None,
    });
    let score = |i: &ImageJson| {
        let lang = i.iso_639_1.as_deref();
        let reg = i.iso_3166_1.as_deref().map(str::to_uppercase);
        (
            lang == Some(code.as_str()) && reg == region,
            lang == Some(code.as_str()) && reg.is_none(),
            lang == Some(code.as_str()),
            lang == Some("en"),
            lang.is_none(),
        )
    };
    let mut list: Vec<&ImageJson> = images.logos.iter().filter(|i| i.file_path.as_deref().is_some_and(|p| !p.trim().is_empty())).collect();
    list.sort_by_key(|i| std::cmp::Reverse(score(i)));
    list.first().and_then(|i| image(i.file_path.as_deref(), "w500"))
}

fn movie_age(list: &[ReleaseCountry], language: &str) -> Option<String> {
    for region in age_regions(language) {
        if let Some(c) = list.iter().find(|c| c.iso_3166_1.as_deref().map(str::to_uppercase).as_deref() == Some(region.as_str())) {
            if let Some(r) = c.release_dates.iter().find_map(|d| text(&d.certification)) {
                return Some(r);
            }
        }
    }
    list.iter().flat_map(|c| c.release_dates.iter()).find_map(|d| text(&d.certification))
}

fn tv_age(list: &[ReleaseCountry], language: &str) -> Option<String> {
    for region in age_regions(language) {
        if let Some(r) = list.iter().find(|c| c.iso_3166_1.as_deref().map(str::to_uppercase).as_deref() == Some(region.as_str())).and_then(|c| text(&c.rating)) {
            return Some(r);
        }
    }
    list.iter().find_map(|c| text(&c.rating))
}

/// `fetchTrailers` (the title's own videos): YouTube only, without repeats; per type, the ones in
/// the chosen language first, then English, official, newest; the "Trailer" type first.
fn trailers(videos: &VideosJson, language: &str) -> Vec<MetaTrailer> {
    let (code, _) = split_language(language);
    let mut seen = HashSet::new();
    let mut by_kind: Vec<(String, Vec<(MetaTrailer, Option<String>)>)> = Vec::new();
    for v in &videos.results {
        let key = text(&v.key).unwrap_or_default();
        if key.is_empty() || !v.site.as_deref().unwrap_or("YouTube").eq_ignore_ascii_case("youtube") || !seen.insert(key.clone()) {
            continue;
        }
        let kind = text(&v.kind).unwrap_or_else(|| "Trailer".into());
        let trailer = MetaTrailer {
            id: text(&v.id).unwrap_or_else(|| key.clone()),
            key,
            name: text(&v.name).unwrap_or_else(|| "Trailer".into()),
            site: "YouTube".into(),
            size: v.size,
            kind: kind.clone(),
            official: v.official == Some(true),
            published_at: text(&v.published_at),
            season_number: None,
        };
        let lang = v.iso_639_1.as_deref().map(str::to_lowercase);
        match by_kind.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, list)) => list.push((trailer, lang)),
            None => by_kind.push((kind, vec![(trailer, lang)])),
        }
    }
    for (_, list) in by_kind.iter_mut() {
        list.sort_by(|(a, la), (b, lb)| {
            let rank = |l: &Option<String>| match l.as_deref() {
                Some(x) if x == code => 0,
                Some("en") => 1,
                _ => 2,
            };
            rank(la).cmp(&rank(lb)).then(b.official.cmp(&a.official)).then(b.published_at.cmp(&a.published_at))
        });
    }
    by_kind.sort_by(|(ka, la), (kb, lb)| {
        let rank = |k: &str, l: &[(MetaTrailer, Option<String>)]| {
            if k.eq_ignore_ascii_case("trailer") {
                0
            } else if l.iter().any(|(t, _)| t.official) {
                1
            } else {
                2
            }
        };
        rank(ka, la).cmp(&rank(kb, lb)).then(ka.to_lowercase().cmp(&kb.to_lowercase()))
    });
    by_kind.into_iter().flat_map(|(_, l)| l.into_iter().map(|(t, _)| t)).collect()
}

async fn fetch_enrichment(access: &Access, settings: &TmdbSettings, media: MediaType, id: i64) -> Result<Enrichment, String> {
    let lang = access.language.clone();
    let path = media.path();
    let details_q = [("language", lang.clone()), ("append_to_response", if media == MediaType::Tv { "external_ids".to_string() } else { String::new() })];
    let credits_endpoint = if media == MediaType::Tv { format!("tv/{id}/aggregate_credits") } else { format!("movie/{id}/credits") };
    let ratings_endpoint = if media == MediaType::Tv { format!("tv/{id}/content_ratings") } else { format!("movie/{id}/release_dates") };
    let want_similar = settings.use_more_like_this;
    let want_trailers = settings.use_trailers;
    let details_endpoint = format!("{path}/{id}");
    let images_endpoint = format!("{path}/{id}/images");
    let credits_q = [("language", lang.clone())];
    let images_q = [("include_image_language", image_languages(&lang))];
    let (details, credits, images, ratings, similar, videos) = tokio::join!(
        get_json::<DetailsJson>(access, &details_endpoint, &details_q),
        get_json::<CreditsJson>(access, &credits_endpoint, &credits_q),
        get_json::<ImagesJson>(access, &images_endpoint, &images_q),
        get_json::<ReleaseDatesJson>(access, &ratings_endpoint, &[]),
        async {
            if want_similar {
                get_json::<PageJson>(access, &format!("{path}/{id}/recommendations"), &[("language", lang.clone())]).await.ok()
            } else {
                None
            }
        },
        async {
            if want_trailers {
                get_json::<VideosJson>(access, &format!("{path}/{id}/videos"), &[("language", lang.clone()), ("include_video_language", format!("{},en,null", split_language(&lang).0))]).await.ok()
            } else {
                None
            }
        },
    );
    let details = details?;
    let credits = credits.unwrap_or_default();
    let directors: Vec<String> = if media == MediaType::Tv {
        distinct(details.created_by.iter().filter_map(|c| text(&c.name)))
    } else {
        distinct(credits.crew.iter().filter(|c| job_of(c).is_some_and(|j| j.eq_ignore_ascii_case("director"))).filter_map(|c| text(&c.name)))
    };
    let writers = if directors.is_empty() { distinct(credits.crew.iter().filter(|c| is_writer(job_of(c).as_deref())).filter_map(|c| text(&c.name))) } else { Vec::new() };
    let age = ratings.ok().and_then(|r| if media == MediaType::Tv { tv_age(&r.results, &lang) } else { movie_age(&r.results, &lang) });
    let more_like_this: Vec<MetaPreview> = similar
        .map(|p| dedupe(p.results.iter().filter_map(|i| i.preview(i.media(media), PosterShape::Poster)).take(12).collect()))
        .unwrap_or_default();
    let (collection_id, collection_name) = details.belongs_to_collection.as_ref().map(|c| (c.id, text(&c.name))).unwrap_or((None, None));
    let collection = match collection_id.filter(|_| settings.use_collections && media == MediaType::Movie) {
        Some(cid) => match get_json::<PageJson>(access, &format!("collection/{cid}"), &[("language", lang.clone())]).await {
            Ok(body) => {
                let mut parts = body.parts.clone();
                parts.sort_by_key(|p| text(&p.release_date).unwrap_or_else(|| "9999".into()));
                parts.iter().filter_map(|p| p.preview(MediaType::Movie, PosterShape::Landscape)).collect()
            }
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    };
    let countries: Vec<String> = {
        let list: Vec<String> = details.production_countries.iter().filter_map(|c| text(&c.iso_3166_1)).collect();
        if list.is_empty() { details.origin_country.iter().filter(|c| !c.trim().is_empty()).cloned().collect() } else { list }
    };
    Ok(Enrichment {
        title: text(&details.title).or_else(|| text(&details.name)),
        description: text(&details.overview),
        genres: details.genres.iter().filter_map(|g| text(&g.name)).collect(),
        backdrop: image(details.backdrop_path.as_deref(), "w1280"),
        logo: images.ok().and_then(|i| best_logo(&i, &lang)),
        poster: image(details.poster_path.as_deref(), "w500"),
        people: people(&details, &credits, media),
        director: directors,
        writer: writers,
        release_date: text(&details.release_date).or_else(|| text(&details.first_air_date)),
        last_air_date: text(&details.last_air_date).filter(|_| media == MediaType::Tv),
        rating: details.vote_average.filter(|v| *v > 0.0),
        runtime_minutes: details.runtime.or_else(|| details.episode_run_time.first().copied()).filter(|m| *m > 0),
        age_rating: age,
        status: text(&details.status),
        countries,
        language: text(&details.original_language),
        companies: details.production_companies.iter().filter_map(company).collect(),
        networks: details.networks.iter().filter_map(company).collect(),
        collection_name: collection_name.filter(|_| !collection.is_empty()),
        collection_id: collection_id.filter(|_| !collection.is_empty()),
        collection,
        more_like_this,
        trailers: videos.map(|v| trailers(&v, &lang)).unwrap_or_default(),
        seasons: details.number_of_seasons.unwrap_or(0),
        imdb_id: text(&details.imdb_id).or_else(|| details.external_ids.as_ref().and_then(|e| text(&e.imdb_id))).filter(|v| v.starts_with("tt")),
    })
}

/// `fetchEpisodeEnrichment`: `tv/{id}/season/{n}` for each season, in parallel.
async fn fetch_episodes(access: &Access, id: i64, seasons: &[i64]) -> HashMap<(i64, i64), EpisodeExtra> {
    let mut set = tokio::task::JoinSet::new();
    let mut seasons: Vec<i64> = seasons.to_vec();
    seasons.sort();
    seasons.dedup();
    for season in seasons.into_iter().take(60) {
        let access = access.clone();
        set.spawn(async move {
            let body: Result<SeasonJson, String> = get_json(&access, &format!("tv/{id}/season/{season}"), &[("language", access.language.clone())]).await;
            (season, body)
        });
    }
    let mut out = HashMap::new();
    while let Some(joined) = set.join_next().await {
        let Ok((season, Ok(body))) = joined else { continue };
        let poster = image(body.poster_path.as_deref(), "w500");
        for e in &body.episodes {
            let Some(number) = e.episode_number else { continue };
            out.insert(
                (e.season_number.unwrap_or(season), number),
                EpisodeExtra {
                    title: text(&e.name),
                    overview: text(&e.overview),
                    thumbnail: image(e.still_path.as_deref(), "w500"),
                    season_poster: poster.clone(),
                    air_date: text(&e.air_date),
                },
            );
        }
    }
    out
}

/// TMDB's name for an episode it has no title for ("Episode 3", "Episódio 3").
fn generic_episode_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    let mut words = lower.split_whitespace();
    matches!(words.next(), Some("episode" | "episódio" | "episodio" | "الحلقة")) && words.next().is_some_and(|n| n.chars().all(|c| c.is_ascii_digit())) && words.next().is_none()
}

/// `applyEnrichment`: each part only when its switch is on; what TMDB lacks stays as the addon sent.
fn apply(mut meta: MetaDetails, e: &Enrichment, episodes: &HashMap<(i64, i64), EpisodeExtra>, s: &TmdbSettings) -> MetaDetails {
    if s.use_artwork {
        meta.background = e.backdrop.clone().or(meta.background);
        meta.poster = e.poster.clone().or(meta.poster);
        meta.logo = e.logo.clone().or(meta.logo);
    }
    if s.use_basic_info {
        meta.name = e.title.clone().unwrap_or(meta.name);
        meta.description = e.description.clone().or(meta.description);
        if meta.imdb_rating.as_deref().is_none_or(|r| r.trim().is_empty()) {
            meta.imdb_rating = e.rating.map(rating);
        }
        if !e.genres.is_empty() {
            meta.genres = e.genres.clone();
        }
    }
    if s.use_details {
        meta.status = e.status.clone().or(meta.status);
        meta.age_rating = e.age_rating.clone().or(meta.age_rating);
        meta.runtime = e.runtime_minutes.map(|m| format!("{m} min")).or(meta.runtime);
        if !e.countries.is_empty() {
            meta.country = Some(e.countries.join(", "));
        }
        meta.language = e.language.clone().or(meta.language);
    }
    if s.use_credits {
        if !e.director.is_empty() {
            meta.director = e.director.clone();
        }
        if !e.writer.is_empty() {
            meta.writer = e.writer.clone();
        }
        if !e.people.is_empty() {
            meta.cast = e.people.clone();
        }
    }
    if !episodes.is_empty() {
        for v in meta.videos.iter_mut() {
            let (Some(season), Some(episode)) = (v.season, v.episode) else { continue };
            let Some(x) = episodes.get(&(season, episode)) else { continue };
            if s.use_episodes {
                if let Some(title) = x.title.clone().filter(|t| !generic_episode_title(t)) {
                    v.title = title;
                }
                v.overview = x.overview.clone().or(v.overview.take());
                v.thumbnail = x.thumbnail.clone().or(v.thumbnail.take());
            }
            if s.use_season_posters {
                v.season_poster = x.season_poster.clone();
            }
        }
    }
    if s.use_trailers && !e.trailers.is_empty() {
        meta.trailers = e.trailers.clone();
    }
    meta
}

fn extras(e: &Enrichment, id: i64, s: &TmdbSettings) -> Extras {
    Extras {
        tmdb_id: Some(id),
        more_like_this: if s.use_more_like_this { e.more_like_this.clone() } else { Vec::new() },
        collection_id: e.collection_id.filter(|_| s.use_collections),
        collection_name: e.collection_name.clone().filter(|_| s.use_collections),
        collection: if s.use_collections { e.collection.clone() } else { Vec::new() },
        companies: if s.use_productions { e.companies.clone() } else { Vec::new() },
        networks: if s.use_networks { e.networks.clone() } else { Vec::new() },
    }
}

/// `enrichMeta`: the addon's details completed from TMDB. `None` when TMDB does not know the title.
pub async fn enrich(access: &Access, settings: &TmdbSettings, meta: MetaDetails) -> Result<(MetaDetails, Extras), (MetaDetails, String)> {
    let media = MediaType::parse(Some(&meta.content_type));
    let Some(id) = tmdb_id(access, media, &meta.id, None).await else {
        return Err((meta, "O TMDB não conhece este título.".into()));
    };
    let wants_episodes = media == MediaType::Tv && (settings.use_episodes || settings.use_season_posters);
    let seasons = meta.seasons();
    let (enrichment, episodes) = tokio::join!(fetch_enrichment(access, settings, media, id), async {
        if wants_episodes {
            fetch_episodes(access, id, &seasons).await
        } else {
            HashMap::new()
        }
    });
    let enrichment = match enrichment {
        Ok(e) => e,
        Err(error) => return Err((meta, error)),
    };
    let extras = extras(&enrichment, id, settings);
    Ok((apply(meta, &enrichment, &episodes, settings), extras))
}

/// `fetchStandaloneMeta`: the page of a title only TMDB knows. Its id becomes the IMDb id when TMDB
/// has one (stream addons look titles up by it); series get their episodes from the seasons.
pub async fn standalone(access: &Access, settings: &TmdbSettings, content_type: &str, id: i64) -> Result<(MetaDetails, Extras), String> {
    let media = MediaType::parse(Some(content_type));
    let e = fetch_enrichment(access, settings, media, id).await?;
    let imdb = match &e.imdb_id {
        Some(v) => Some(v.clone()),
        None => imdb_id(access, media, id).await,
    };
    let base_id = imdb.clone().unwrap_or_else(|| format!("tmdb:{id}"));
    let mut videos = Vec::new();
    if media == MediaType::Tv && e.seasons > 0 {
        let seasons: Vec<i64> = (1..=e.seasons.min(60)).collect();
        let episodes = fetch_episodes(access, id, &seasons).await;
        let today = super::home::today_iso();
        let mut keys: Vec<&(i64, i64)> = episodes.keys().collect();
        keys.sort();
        for key in keys {
            let x = &episodes[key];
            videos.push(MetaVideo {
                id: format!("{base_id}:{}:{}", key.0, key.1),
                title: x.title.clone().unwrap_or_else(|| format!("Episódio {}", key.1)),
                released: x.air_date.clone(),
                available: x.air_date.as_deref().is_some_and(|d| d <= today.as_str()),
                thumbnail: x.thumbnail.clone(),
                season: Some(key.0),
                episode: Some(key.1),
                overview: x.overview.clone(),
                season_poster: x.season_poster.clone(),
            });
        }
    }
    let year = e.release_date.as_deref().map(|d| d.chars().take(4).collect::<String>());
    let release_info = match (media, &year, e.last_air_date.as_deref().map(|d| d.chars().take(4).collect::<String>())) {
        (MediaType::Tv, Some(from), Some(to)) if e.status.as_deref() == Some("Ended") && *from != to => Some(format!("{from}–{to}")),
        (MediaType::Tv, Some(from), _) if e.status.as_deref() != Some("Ended") => Some(format!("{from}–")),
        _ => year,
    };
    let meta = MetaDetails {
        id: base_id,
        content_type: media.content_type().into(),
        name: e.title.clone().unwrap_or_else(|| format!("TMDB {id}")),
        poster: e.poster.clone(),
        background: e.backdrop.clone(),
        logo: e.logo.clone(),
        description: e.description.clone(),
        release_info,
        imdb_rating: e.rating.map(rating),
        age_rating: e.age_rating.clone(),
        status: e.status.clone(),
        runtime: e.runtime_minutes.map(|m| format!("{m} min")),
        genres: e.genres.clone(),
        director: e.director.clone(),
        writer: e.writer.clone(),
        cast: e.people.clone(),
        country: (!e.countries.is_empty()).then(|| e.countries.join(", ")),
        awards: None,
        language: e.language.clone(),
        website: None,
        default_video_id: None,
        videos,
        trailers: e.trailers.clone(),
    };
    // A page built from TMDB shows everything it has, whatever the switches say.
    let all = TmdbSettings::default();
    Ok((meta, extras(&e, id, &all)))
}

// ---------------------------------------------------------------- person, company, network pages

/// Header of a person, studio or network page.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntityHeader {
    pub kind: String,
    pub id: i64,
    pub name: String,
    pub image: Option<String>,
    pub description: Option<String>,
    /// Person: "Atuação", "Direção"…; studio/network: the country.
    pub subtitle: Option<String>,
    /// Person: birth date and place.
    pub facts: Vec<String>,
    /// Media types that have titles (`movie`, `series`).
    pub media: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PersonJson {
    name: Option<String>,
    biography: Option<String>,
    birthday: Option<String>,
    deathday: Option<String>,
    place_of_birth: Option<String>,
    profile_path: Option<String>,
    known_for_department: Option<String>,
    combined_credits: Option<PageJson>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct CompanyDetailsJson {
    name: Option<String>,
    description: Option<String>,
    logo_path: Option<String>,
    origin_country: Option<String>,
    headquarters: Option<String>,
}

fn department(value: Option<&str>) -> Option<String> {
    Some(
        match value? {
            "Acting" => "Atuação",
            "Directing" => "Direção",
            "Writing" => "Roteiro",
            "Production" => "Produção",
            "Sound" => "Som",
            "Camera" => "Fotografia",
            "Editing" => "Edição",
            "Creator" => "Criação",
            other => return Some(other.to_string()),
        }
        .to_string(),
    )
}

/// `isTalkShowGuestAppearance`: talk shows, news and reality (genres 10767, 10763, 10764) where
/// the person only appears as themselves.
fn guest_appearance(item: &ItemJson) -> bool {
    let character = item.character.as_deref().unwrap_or("").to_lowercase();
    item.genre_ids.iter().any(|g| [10767, 10763, 10764].contains(g)) || character.contains("self") || character.contains("himself") || character.contains("herself")
}

/// The titles of a person (`fetchPersonDetail`): their cast credits, or the directing/writing ones
/// when they are known for those; most popular first, without repeats.
fn person_titles(body: &PageJson, department: Option<&str>) -> Vec<MetaPreview> {
    let behind = matches!(department, Some("Directing") | Some("Writing") | Some("Creator"));
    let mut list: Vec<&ItemJson> = if behind {
        body.crew.iter().filter(|c| c.job.as_deref().is_some_and(|j| j.eq_ignore_ascii_case("director") || is_writer(Some(j)) || j.eq_ignore_ascii_case("creator"))).collect()
    } else {
        body.cast.iter().filter(|c| !guest_appearance(c)).collect()
    };
    if list.is_empty() {
        list = body.cast.iter().chain(body.crew.iter()).collect();
    }
    list.sort_by(|a, b| b.popularity.unwrap_or(0.0).partial_cmp(&a.popularity.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    dedupe(list.into_iter().filter_map(|i| i.preview(i.media(MediaType::Movie), PosterShape::Poster)).collect())
}

/// A person's page: the header and all their titles (one request).
pub async fn person_page(access: &Access, id: i64) -> Result<(EntityHeader, Vec<MetaPreview>), String> {
    let body: PersonJson = get_json(access, &format!("person/{id}"), &[("language", access.language.clone()), ("append_to_response", "combined_credits".into())]).await?;
    let mut biography = text(&body.biography);
    if biography.is_none() && !access.language.starts_with("en") {
        // `fetchEnglishFallbackNames`' idea for the biography: TMDB often has it only in English.
        let english = Access { key: access.key.clone(), language: "en-US".into() };
        biography = get_json::<PersonJson>(&english, &format!("person/{id}"), &[("language", "en-US".into())]).await.ok().and_then(|b| text(&b.biography));
    }
    let credits = body.combined_credits.unwrap_or_default();
    let titles = person_titles(&credits, body.known_for_department.as_deref());
    let mut facts = Vec::new();
    if let Some(b) = text(&body.birthday) {
        facts.push(match text(&body.place_of_birth) {
            Some(place) => format!("{b} · {place}"),
            None => b,
        });
    }
    if let Some(d) = text(&body.deathday) {
        facts.push(format!("† {d}"));
    }
    let mut media: Vec<String> = Vec::new();
    for t in &titles {
        if !media.contains(&t.content_type) {
            media.push(t.content_type.clone());
        }
    }
    Ok((
        EntityHeader {
            kind: "person".into(),
            id,
            name: text(&body.name).unwrap_or_else(|| format!("TMDB {id}")),
            image: image(body.profile_path.as_deref(), "w500"),
            description: biography,
            subtitle: department(body.known_for_department.as_deref()),
            facts,
            media,
        },
        titles,
    ))
}

/// A studio's or network's page header. Studios list movies and series; networks, series.
pub async fn company_header(access: &Access, kind: &str, id: i64) -> Result<EntityHeader, String> {
    let endpoint = if kind == "network" { format!("network/{id}") } else { format!("company/{id}") };
    let body: CompanyDetailsJson = get_json(access, &endpoint, &[]).await?;
    Ok(EntityHeader {
        kind: kind.into(),
        id,
        name: text(&body.name).unwrap_or_else(|| format!("TMDB {id}")),
        image: image(body.logo_path.as_deref(), "w500"),
        description: text(&body.description),
        subtitle: text(&body.headquarters).or_else(|| text(&body.origin_country)),
        facts: Vec::new(),
        media: if kind == "network" { vec!["series".into()] } else { vec!["movie".into(), "series".into()] },
    })
}

/// One page of a studio's or network's titles (`discover`, most popular first).
pub async fn company_titles(access: &Access, kind: &str, id: i64, media: MediaType, page: u32) -> Result<CatalogPage, String> {
    let source = Source {
        source_type: if kind == "network" { "NETWORK".into() } else { "COMPANY".into() },
        media_type: if kind == "network" { MediaType::Tv } else { media },
        sort_by: None,
        tmdb_id: Some(id),
        filters: Filters::default(),
    };
    source_page(access, &source, page).await
}

// ---------------------------------------------------------------- collection folder sources

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

/// One TMDB source of a folder (`CollectionSource` with provider `tmdb`).
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// `DISCOVER`, `NETWORK`, `COMPANY`, `LIST`, `COLLECTION`, `PERSON` or `DIRECTOR`.
    pub source_type: String,
    pub media_type: MediaType,
    pub sort_by: Option<String>,
    pub tmdb_id: Option<i64>,
    pub filters: Filters,
}

impl Source {
    /// From the stored JSON of a source; `None` for another provider or an unknown kind.
    pub fn from_json(value: &Value) -> Option<Self> {
        if !value.get("provider").and_then(Value::as_str).is_some_and(|p| p.eq_ignore_ascii_case("tmdb")) {
            return None;
        }
        let source_type = value.get("tmdbSourceType").and_then(Value::as_str).unwrap_or("DISCOVER").trim().to_uppercase();
        if !supported(&source_type) {
            return None;
        }
        let tmdb_id = value.get("tmdbId").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
        Some(Self {
            source_type,
            media_type: MediaType::parse(value.get("mediaType").and_then(Value::as_str)),
            sort_by: value.get("sortBy").and_then(Value::as_str).map(str::to_string),
            tmdb_id,
            filters: value.get("filters").and_then(|f| serde_json::from_value(f.clone()).ok()).unwrap_or_default(),
        })
    }

    /// Tab label when the source has no title of its own.
    pub fn default_label(&self) -> String {
        let media = if self.media_type == MediaType::Tv { "Séries" } else { "Filmes" };
        match self.source_type.as_str() {
            "LIST" => "Lista do TMDB".into(),
            "COLLECTION" => "Coleção do TMDB".into(),
            "COMPANY" => format!("{media} do estúdio"),
            "NETWORK" => "Séries do canal".into(),
            "PERSON" => format!("{media} da pessoa"),
            "DIRECTOR" => format!("{media} do diretor"),
            _ => format!("{media} do TMDB"),
        }
    }

    pub fn paged(&self) -> bool {
        matches!(self.source_type.as_str(), "DISCOVER" | "NETWORK" | "COMPANY" | "LIST")
    }
}

/// Every TMDB source kind of the original (`TmdbCollectionSourceType`).
pub fn supported(source_type: &str) -> bool {
    matches!(source_type.trim().to_uppercase().as_str(), "DISCOVER" | "NETWORK" | "COMPANY" | "LIST" | "COLLECTION" | "PERSON" | "DIRECTOR")
}

/// `movieSort`/`tvSort`: "original" is popularity; the date sorts swap by media type.
fn sort_for(sort_by: Option<&str>, media: MediaType) -> String {
    match (sort_by.map(str::trim).filter(|v| !v.is_empty()), media) {
        (None, _) | (Some("original"), _) => "popularity.desc".into(),
        (Some("first_air_date.desc"), MediaType::Movie) => "primary_release_date.desc".into(),
        (Some("primary_release_date.desc"), MediaType::Tv) => "first_air_date.desc".into(),
        (Some(other), _) => other.to_string(),
    }
}

/// The original sends a `Double` as `7.0`, `6.5`.
fn number(value: f64) -> String {
    if value.fract() == 0.0 { format!("{value:.1}") } else { value.to_string() }
}

/// `buildDiscoverQuery`.
pub fn discover_query(source: &Source, page: u32, language: &str) -> Vec<(&'static str, String)> {
    let media = source.media_type;
    let f = &source.filters;
    let mut q: Vec<(&'static str, String)> = Vec::new();
    let mut put = |name: &'static str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            q.push((name, v));
        }
    };
    put("language", Some(language.to_string()));
    put("page", Some(page.to_string()));
    put("sort_by", Some(sort_for(source.sort_by.as_deref(), media)));
    let company = source.tmdb_id.map(|id| id.to_string()).filter(|_| source.source_type == "COMPANY");
    let network = source.tmdb_id.map(|id| id.to_string()).filter(|_| source.source_type == "NETWORK");
    put("with_companies", company.or_else(|| f.with_companies.clone()));
    put("without_companies", f.without_companies.clone());
    put("with_networks", network.or_else(|| f.with_networks.clone()));
    put("with_genres", f.with_genres.clone());
    put("without_genres", f.without_genres.clone());
    put("vote_count.gte", f.vote_count_gte.map(|v| v.to_string()));
    put("vote_average.gte", f.vote_average_gte.map(number));
    put("vote_average.lte", f.vote_average_lte.map(number));
    put("with_original_language", f.with_original_language.clone());
    put("with_origin_country", f.with_origin_country.clone());
    put("with_keywords", f.with_keywords.clone());
    put("without_keywords", f.without_keywords.clone());
    let with_providers = f.with_watch_providers.clone().filter(|v| !v.trim().is_empty());
    let without_providers = f.without_watch_providers.clone().filter(|v| !v.trim().is_empty());
    if with_providers.is_some() || without_providers.is_some() {
        put("with_watch_providers", with_providers.clone());
        put("without_watch_providers", without_providers);
        put("watch_region", Some(f.watch_region.clone().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "US".into())));
        if with_providers.is_some() {
            put("with_watch_monetization_types", Some("flatrate|free|ads|rent|buy".into()));
        }
    }
    put("year", f.year.filter(|_| media == MediaType::Movie).map(|v| v.to_string()));
    put("first_air_date_year", f.year.filter(|_| media == MediaType::Tv).map(|v| v.to_string()));
    let (gte, lte) = if media == MediaType::Movie { ("primary_release_date.gte", "primary_release_date.lte") } else { ("first_air_date.gte", "first_air_date.lte") };
    put(gte, f.release_date_gte.clone());
    put(lte, f.release_date_lte.clone());
    q
}

/// `sortedFor`: only these sorts reorder; the others keep TMDB's order.
fn sort_items(mut items: Vec<(MetaPreview, i64)>, sort_by: Option<&str>) -> Vec<MetaPreview> {
    let score = |i: &MetaPreview| i.imdb_rating.as_deref().and_then(|r| r.parse::<f64>().ok()).unwrap_or(-1.0);
    let date = |i: &MetaPreview| i.raw_release_date.clone().or_else(|| i.release_info.clone()).unwrap_or_default();
    match sort_by.map(str::trim).unwrap_or("") {
        "vote_average.desc" => items.sort_by(|a, b| score(&b.0).partial_cmp(&score(&a.0)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| date(&b.0).cmp(&date(&a.0)))),
        "vote_count.desc" => items.sort_by_key(|(_, votes)| std::cmp::Reverse(*votes)),
        "primary_release_date.desc" | "first_air_date.desc" => items.sort_by(|a, b| date(&b.0).cmp(&date(&a.0))),
        _ => {}
    }
    items.into_iter().map(|(i, _)| i).collect()
}

fn paged(body: &PageJson, items: Vec<MetaPreview>, page: u32) -> CatalogPage {
    let current = body.page.unwrap_or(page);
    let total = body.total_pages.unwrap_or(page);
    let next_skip = (current < total && !items.is_empty()).then_some(page + 1);
    CatalogPage { raw_item_count: items.len(), items, next_skip }
}

/// One page of a TMDB folder source (`resolve`); `page` is 1-based.
pub async fn source_page(access: &Access, source: &Source, page: u32) -> Result<CatalogPage, String> {
    let page = page.max(1);
    let lang = access.language.clone();
    match source.source_type.as_str() {
        "LIST" => {
            let id = source.tmdb_id.ok_or("Lista do TMDB sem número.")?;
            let body: PageJson = get_json(access, &format!("list/{id}"), &[("language", lang), ("page", page.to_string())]).await?;
            let items: Vec<(MetaPreview, i64)> = body.items.iter().filter_map(|i| i.preview(i.media(MediaType::Movie), PosterShape::Poster).map(|p| (p, i.vote_count.unwrap_or(0)))).collect();
            let items = dedupe(sort_items(items, source.sort_by.as_deref()));
            Ok(paged(&body, items, page))
        }
        "COLLECTION" => {
            let id = source.tmdb_id.ok_or("Coleção do TMDB sem número.")?;
            let body: PageJson = get_json(access, &format!("collection/{id}"), &[("language", lang)]).await?;
            let items: Vec<(MetaPreview, i64)> = body.parts.iter().filter_map(|i| i.preview(MediaType::Movie, PosterShape::Poster).map(|p| (p, i.vote_count.unwrap_or(0)))).collect();
            let items = dedupe(sort_items(items, source.sort_by.as_deref()));
            Ok(CatalogPage { raw_item_count: items.len(), items, next_skip: None })
        }
        "PERSON" | "DIRECTOR" => {
            let id = source.tmdb_id.ok_or("Pessoa do TMDB sem número.")?;
            let body: PageJson = get_json(access, &format!("person/{id}/combined_credits"), &[("language", lang)]).await?;
            let credits: Vec<&ItemJson> = if source.source_type == "DIRECTOR" {
                body.crew.iter().filter(|c| c.job.as_deref().is_some_and(|j| j.eq_ignore_ascii_case("director"))).collect()
            } else {
                body.cast.iter().collect()
            };
            let mut seen = HashSet::new();
            let items: Vec<(MetaPreview, i64)> = credits
                .into_iter()
                .filter(|c| c.media(MediaType::Movie) == source.media_type)
                .filter_map(|c| c.preview(source.media_type, PosterShape::Poster).map(|p| (p, c.vote_count.unwrap_or(0))))
                .filter(|(p, _)| seen.insert(p.stable_key()))
                .collect();
            let items = sort_items(items, source.sort_by.as_deref());
            Ok(CatalogPage { raw_item_count: items.len(), items, next_skip: None })
        }
        _ => {
            let endpoint = if source.media_type == MediaType::Tv { "discover/tv" } else { "discover/movie" };
            let query = discover_query(source, page, &lang);
            let body: PageJson = get_json(access, endpoint, &query).await?;
            let items = dedupe(body.results.iter().filter_map(|i| i.preview(source.media_type, PosterShape::Poster)).collect());
            Ok(paged(&body, items, page))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_and_regions() {
        assert_eq!(normalize_language(Some("pt_br")), "pt-BR");
        assert_eq!(normalize_language(Some("es-419")), "es-MX");
        assert_eq!(normalize_language(None), "en");
        assert_eq!(age_regions("pt-BR"), vec!["BR", "US", "GB"]);
        assert_eq!(age_regions("de"), vec!["DE", "US", "GB"]);
        assert_eq!(image_languages("pt-BR"), "pt,pt-BR,en,null");
    }

    #[test]
    fn token_or_key() {
        assert!(is_token("eyJhbGciOiJIUzI1NiJ9.eyJhdWQiOiJ4In0.c2ln"));
        assert!(!is_token("0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn logo_prefers_language_then_english() {
        let images: ImagesJson = serde_json::from_str(r#"{"logos":[{"file_path":"/x.png","iso_639_1":"fr"},{"file_path":"/en.png","iso_639_1":"en"},{"file_path":"/pt.png","iso_639_1":"pt","iso_3166_1":"BR"}]}"#).unwrap();
        assert_eq!(best_logo(&images, "pt-BR").as_deref(), Some("https://image.tmdb.org/t/p/w500/pt.png"));
        assert_eq!(best_logo(&images, "de-DE").as_deref(), Some("https://image.tmdb.org/t/p/w500/en.png"));
    }

    #[test]
    fn people_put_directors_first_and_dedupe() {
        let details = DetailsJson::default();
        let credits: CreditsJson = serde_json::from_str(r#"{"cast":[{"id":1,"name":"Ana","character":"Maria"},{"id":1,"name":"Ana","character":"Maria"},{"id":2,"name":"Beto"}],
            "crew":[{"id":9,"name":"Dir","job":"Director"},{"id":8,"name":"Esc","job":"Screenplay"}]}"#).unwrap();
        let list = people(&details, &credits, MediaType::Movie);
        assert_eq!(list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Dir", "Ana", "Beto"]);
        assert_eq!(list[0].tmdb_id, Some(9));
        assert_eq!(list[1].role.as_deref(), Some("Maria"));
    }

    #[test]
    fn age_rating_by_region() {
        let r: ReleaseDatesJson = serde_json::from_str(r#"{"results":[{"iso_3166_1":"US","release_dates":[{"certification":"PG-13"}]},{"iso_3166_1":"BR","release_dates":[{"certification":""},{"certification":"14"}]}]}"#).unwrap();
        assert_eq!(movie_age(&r.results, "pt-BR").as_deref(), Some("14"));
        assert_eq!(movie_age(&r.results, "en-US").as_deref(), Some("PG-13"));
    }

    #[test]
    fn trailers_prefer_language_and_trailer_type() {
        let v: VideosJson = serde_json::from_str(r#"{"results":[
            {"key":"t1","site":"YouTube","type":"Teaser","official":true,"iso_639_1":"en"},
            {"key":"t2","site":"YouTube","type":"Trailer","official":false,"iso_639_1":"en"},
            {"key":"t3","site":"YouTube","type":"Trailer","official":true,"iso_639_1":"pt"},
            {"key":"t3","site":"YouTube","type":"Trailer"},
            {"key":"v","site":"Vimeo","type":"Trailer"}]}"#).unwrap();
        let list = trailers(&v, "pt-BR");
        assert_eq!(list.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), vec!["t3", "t2", "t1"]);
    }

    #[test]
    fn apply_follows_switches() {
        let meta = nuvio_core::meta::parse_meta(r#"{"meta":{"id":"tt1","type":"series","name":"Addon","description":"d","videos":[{"id":"tt1:1:1","title":"Episode 1","season":1,"episode":1},{"id":"tt1:1:2","title":"Pilot B","season":1,"episode":2}]}}"#).unwrap();
        let e = Enrichment { title: Some("TMDB".into()), backdrop: Some("b".into()), genres: vec!["Drama".into()], ..Default::default() };
        let mut eps = HashMap::new();
        eps.insert((1, 1), EpisodeExtra { title: Some("O começo".into()), thumbnail: Some("th".into()), season_poster: Some("sp".into()), ..Default::default() });
        eps.insert((1, 2), EpisodeExtra { title: Some("Episódio 2".into()), ..Default::default() });
        let s = TmdbSettings { use_artwork: false, ..Default::default() };
        let out = apply(meta, &e, &eps, &s);
        assert_eq!(out.name, "TMDB");
        assert_eq!(out.background, None, "artwork switch off");
        assert_eq!(out.genres, vec!["Drama"]);
        assert_eq!(out.videos[0].title, "O começo");
        assert_eq!(out.videos[0].season_poster.as_deref(), Some("sp"));
        assert_eq!(out.videos[1].title, "Pilot B", "a generic TMDB title does not replace the addon's");
    }

    #[test]
    fn source_from_json_and_discover_query() {
        let v = serde_json::json!({"provider":"tmdb","tmdbSourceType":"company","tmdbId":"420","mediaType":"movie","sortBy":"vote_average.desc","filters":{"voteAverageGte":7.0,"withWatchProviders":"8"}});
        let s = Source::from_json(&v).unwrap();
        assert_eq!(s.source_type, "COMPANY");
        assert_eq!(s.tmdb_id, Some(420));
        let q = discover_query(&s, 2, "pt-BR");
        let get = |k: &str| q.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
        assert_eq!(get("with_companies").as_deref(), Some("420"));
        assert_eq!(get("vote_average.gte").as_deref(), Some("7.0"));
        assert_eq!(get("watch_region").as_deref(), Some("US"));
        assert_eq!(get("page").as_deref(), Some("2"));
        assert!(Source::from_json(&serde_json::json!({"provider":"addon"})).is_none());
        assert!(Source::from_json(&serde_json::json!({"provider":"tmdb","tmdbSourceType":"WEIRD"})).is_none());
    }

    #[test]
    fn person_titles_skip_talk_shows() {
        let body: PageJson = serde_json::from_str(r#"{"cast":[
            {"id":1,"media_type":"movie","title":"Filme","popularity":5},
            {"id":2,"media_type":"tv","name":"Talk","genre_ids":[10767],"popularity":99},
            {"id":3,"media_type":"tv","name":"Série","popularity":9}]}"#).unwrap();
        let list = person_titles(&body, Some("Acting"));
        assert_eq!(list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Série", "Filme"]);
        assert_eq!(list[0].content_type, "series");
    }
}
