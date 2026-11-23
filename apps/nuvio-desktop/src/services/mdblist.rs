//! MDBList ratings (`features/mdblist/MdbListRatings*` in the original): Settings → Services →
//! MDBList Ratings. The key comes from the Nuvio account (`ProviderCredentialSync`, provider
//! `mdblist`), from a key saved here or from the original app's store.
//!
//! One request per title: `GET https://api.mdblist.com/imdb/{movie|show}/{imdbId}/?apikey=…`.
//! Ratings are mapped like `parseMdbListRatings` (sources, ranges, Letterboxd score/20, certified
//! marks from the keywords), then filtered by the provider switches and ordered like the original.

use nuvio_store::nextgen::AppStore;
use nuvio_store::{default_legacy_data_dir, LegacyStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const PROVIDERS: [&str; 8] = ["imdb", "tmdb", "tomatoes", "metacritic", "trakt", "letterboxd", "audience", "mal"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub api_key: String,
    pub use_imdb: bool,
    pub use_tmdb: bool,
    pub use_tomatoes: bool,
    pub use_metacritic: bool,
    pub use_trakt: bool,
    pub use_letterboxd: bool,
    pub use_audience: bool,
    pub use_mal: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: String::new(),
            use_imdb: true,
            use_tmdb: true,
            use_tomatoes: true,
            use_metacritic: true,
            use_trakt: true,
            use_letterboxd: true,
            use_audience: true,
            use_mal: true,
        }
    }
}

impl Settings {
    fn provider_enabled(&self, provider: &str) -> bool {
        match provider {
            "imdb" => self.use_imdb,
            "tmdb" => self.use_tmdb,
            "tomatoes" => self.use_tomatoes,
            "metacritic" => self.use_metacritic,
            "trakt" => self.use_trakt,
            "letterboxd" => self.use_letterboxd,
            "audience" => self.use_audience,
            "mal" => self.use_mal,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub enabled: bool,
    pub api_key_set: bool,
    pub api_key_from_account: bool,
    pub api_key_from_original: bool,
    pub use_imdb: bool,
    pub use_tmdb: bool,
    pub use_tomatoes: bool,
    pub use_metacritic: bool,
    pub use_trakt: bool,
    pub use_letterboxd: bool,
    pub use_audience: bool,
    pub use_mal: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsInput {
    pub enabled: bool,
    /// `None` keeps the key; an empty string clears Astra's own key.
    pub api_key: Option<String>,
    pub use_imdb: bool,
    pub use_tmdb: bool,
    pub use_tomatoes: bool,
    pub use_metacritic: bool,
    pub use_trakt: bool,
    pub use_letterboxd: bool,
    pub use_audience: bool,
    pub use_mal: bool,
}

impl Default for SettingsInput {
    fn default() -> Self {
        let defaults = Settings::default();
        Self {
            enabled: defaults.enabled,
            api_key: None,
            use_imdb: defaults.use_imdb,
            use_tmdb: defaults.use_tmdb,
            use_tomatoes: defaults.use_tomatoes,
            use_metacritic: defaults.use_metacritic,
            use_trakt: defaults.use_trakt,
            use_letterboxd: defaults.use_letterboxd,
            use_audience: defaults.use_audience,
            use_mal: defaults.use_mal,
        }
    }
}

/// One external rating shown on the details page (`MetaExternalRating`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rating {
    pub source: String,
    pub value: f64,
    pub certified: bool,
}

fn legacy_values() -> Option<HashMap<String, String>> {
    let dir = default_legacy_data_dir().ok()?;
    LegacyStore::open(dir).ok()?.read_store("nuvio_mdblist_settings").ok()
}

fn stored(store: &AppStore) -> std::collections::BTreeMap<i64, Settings> {
    store.read("mdblist-settings").ok().flatten().unwrap_or_default()
}

pub fn settings(store: &AppStore, profile: i64) -> Settings {
    let own = stored(store).remove(&profile);
    match own {
        Some(settings) => settings,
        None => {
            let legacy = legacy_values();
            Settings {
                enabled: legacy
                    .as_ref()
                    .and_then(|v| v.get(&format!("mdblist_enabled_{profile}")))
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("true")),
                ..Default::default()
            }
        }
    }
}

fn key(store: &AppStore, profile: i64) -> Option<(String, KeyOrigin)> {
    let own = settings(store, profile).api_key.trim().to_string();
    if !own.is_empty() {
        return Some((own, KeyOrigin::Own));
    }
    if let Some(account) = store.provider_key(profile, "mdblist").filter(|key| !key.trim().is_empty()) {
        return Some((account, KeyOrigin::Account));
    }
    let legacy = legacy_values()?;
    let key = legacy.get(&format!("mdblist_api_key_{profile}"))?.trim().to_string();
    (!key.is_empty()).then_some((key, KeyOrigin::Original))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyOrigin {
    Own,
    Account,
    Original,
}

pub fn view(store: &AppStore, profile: i64) -> View {
    let settings = settings(store, profile);
    let key = key(store, profile);
    View {
        enabled: settings.enabled,
        api_key_set: key.is_some(),
        api_key_from_account: key.as_ref().is_some_and(|(_, origin)| *origin == KeyOrigin::Account),
        api_key_from_original: key.as_ref().is_some_and(|(_, origin)| *origin == KeyOrigin::Original),
        use_imdb: settings.use_imdb,
        use_tmdb: settings.use_tmdb,
        use_tomatoes: settings.use_tomatoes,
        use_metacritic: settings.use_metacritic,
        use_trakt: settings.use_trakt,
        use_letterboxd: settings.use_letterboxd,
        use_audience: settings.use_audience,
        use_mal: settings.use_mal,
    }
}

pub fn update(store: &AppStore, profile: i64, input: SettingsInput) -> Result<View, String> {
    let edited = input.api_key.clone();
    let mut next = settings(store, profile);
    next.enabled = input.enabled;
    next.use_imdb = input.use_imdb;
    next.use_tmdb = input.use_tmdb;
    next.use_tomatoes = input.use_tomatoes;
    next.use_metacritic = input.use_metacritic;
    next.use_trakt = input.use_trakt;
    next.use_letterboxd = input.use_letterboxd;
    next.use_audience = input.use_audience;
    next.use_mal = input.use_mal;
    if let Some(key) = input.api_key {
        next.api_key = key.trim().to_string();
    }
    let mut all = stored(store);
    all.insert(profile, next);
    store.write("mdblist-settings", &all).map_err(|e| e.to_string())?;
    if let Some(key) = edited {
        // The key also travels with the account's provider credentials (as the original does).
        if let Ok(mut all) = store.provider_credentials() {
            let mut providers = all.remove(&profile).unwrap_or_default();
            if key.trim().is_empty() {
                providers.remove("mdblist");
            } else {
                providers.insert("mdblist".into(), key.trim().to_string());
            }
            let _ = store.set_provider_credentials(profile, providers);
        }
    }
    Ok(view(store, profile))
}

// ---------------------------------------------------------------- ratings

static CACHE: OnceLock<Mutex<HashMap<String, Vec<Rating>>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Vec<Rating>>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| reqwest::Client::builder().user_agent("NuvioAstra/0.1").timeout(Duration::from_secs(12)).build().expect("mdblist client"))
}

/// Ratings of a title from its IMDb id. Empty when MDBList is off, there is no key, the id is not
/// an IMDb id, or the request fails (the details page is never held back by ratings).
pub async fn ratings(store: &AppStore, profile: i64, imdb_id: &str, is_series: bool) -> Vec<Rating> {
    let settings = settings(store, profile);
    let providers: Vec<&str> = PROVIDERS.iter().copied().filter(|p| settings.provider_enabled(p)).collect();
    let Some((key, _)) = key(store, profile) else { return Vec::new() };
    if !settings.enabled || providers.is_empty() {
        return Vec::new();
    }
    let imdb_id = imdb_id.trim();
    if !imdb_id.starts_with("tt") {
        return Vec::new();
    }
    let cache_key = format!("{imdb_id}:{}", is_series);
    if let Some(hit) = cache().lock().ok().and_then(|guard| guard.get(&cache_key).cloned()) {
        return select(hit, &providers);
    }
    let media = if is_series { "show" } else { "movie" };
    let url = format!(
        "https://api.mdblist.com/imdb/{media}/{imdb_id}/?apikey={}",
        encode(&key)
    );
    let response = match http().get(&url).header("Accept", "application/json").send().await {
        Ok(response) if response.status().is_success() => response,
        _ => return Vec::new(),
    };
    let payload = match response.text().await {
        Ok(payload) => payload,
        Err(_) => return Vec::new(),
    };
    let parsed = parse_ratings(&payload).unwrap_or_default();
    if let Ok(mut guard) = cache().lock() {
        if guard.len() >= 128 {
            guard.clear();
        }
        guard.insert(cache_key, parsed.clone());
    }
    select(parsed, &providers)
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn select(ratings: Vec<Rating>, providers: &[&str]) -> Vec<Rating> {
    providers
        .iter()
        .filter_map(|provider| ratings.iter().find(|rating| rating.source == *provider).cloned())
        .collect()
}

#[derive(Debug, Deserialize, Default)]
struct MediaRatings {
    #[serde(default)]
    ratings: Vec<SourceRating>,
    #[serde(default)]
    keywords: Vec<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct SourceRating {
    source: String,
    value: Option<f64>,
    score: Option<f64>,
}

fn parse_ratings(payload: &str) -> Option<Vec<Rating>> {
    let media: MediaRatings = serde_json::from_str(payload).ok()?;
    let keywords: Vec<String> = media
        .keywords
        .iter()
        .filter_map(|keyword| match keyword {
            Value::Object(map) => map.get("name").and_then(Value::as_str).map(str::to_string),
            Value::String(name) => Some(name.clone()),
            _ => None,
        })
        .map(|name| name.rsplit('.').next().unwrap_or(&name).to_string())
        .collect();
    let mut seen = std::collections::HashSet::new();
    let ratings = media
        .ratings
        .into_iter()
        .filter_map(|rating| {
            let source = match rating.source.as_str() {
                "popcorn" | "audience" | "tomatoesaudience" => "audience",
                "myanimelist" | "mal" => "mal",
                "imdb" | "tmdb" | "tomatoes" | "metacritic" | "trakt" | "letterboxd" => rating.source.as_str(),
                _ => return None,
            };
            let maximum = match source {
                "imdb" | "mal" => 10.0,
                "letterboxd" => 5.0,
                _ => 100.0,
            };
            let raw = rating.value.filter(|value| *value >= 0.0)?;
            let value = if source == "letterboxd" {
                rating.score.map(|score| score / 20.0).unwrap_or(raw)
            } else {
                raw
            };
            if !(0.0..=maximum).contains(&value) || !seen.insert(source.to_string()) {
                return None;
            }
            let certified = match source {
                "tomatoes" => keywords.iter().any(|k| k == "certified-fresh"),
                "audience" => keywords.iter().any(|k| k == "certified-hot"),
                _ => false,
            };
            Some(Rating { source: source.to_string(), value, certified })
        })
        .collect();
    Some(ratings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_are_mapped_like_the_original() {
        let payload = r#"{"ratings":[
            {"source":"imdb","value":8.5},
            {"source":"tomatoes","value":92},
            {"source":"popcorn","value":88},
            {"source":"metacritic","value":74},
            {"source":"letterboxd","value":4.2,"score":84},
            {"source":"myanimelist","value":9.1},
            {"source":"trakt","value":81},
            {"source":"unknown","value":50},
            {"source":"tmdb","value":-1}],
            "keywords":[{"name":"certified.certified-fresh"},{"name":"certified-hot"}]}"#;
        let ratings = parse_ratings(payload).unwrap();
        let by = |source: &str| ratings.iter().find(|r| r.source == source).cloned();
        assert_eq!(by("imdb").unwrap().value, 8.5);
        assert_eq!(by("tomatoes").unwrap().value, 92.0);
        assert!(by("tomatoes").unwrap().certified);
        assert_eq!(by("audience").unwrap().value, 88.0);
        assert!(by("audience").unwrap().certified);
        assert_eq!(by("metacritic").unwrap().value, 74.0);
        assert_eq!(by("letterboxd").unwrap().value, 4.2, "score/20 when present");
        assert_eq!(by("mal").unwrap().value, 9.1);
        assert_eq!(by("trakt").unwrap().value, 81.0);
        assert!(by("unknown").is_none());
        assert!(by("tmdb").is_none(), "negative value dropped");
    }

    #[test]
    fn providers_are_filtered_in_priority_order() {
        let ratings = vec![
            Rating { source: "imdb".into(), value: 8.0, certified: false },
            Rating { source: "tomatoes".into(), value: 90.0, certified: false },
            Rating { source: "trakt".into(), value: 80.0, certified: false },
        ];
        let selected = select(ratings, &["trakt", "imdb"]);
        assert_eq!(selected.iter().map(|r| r.source.as_str()).collect::<Vec<_>>(), ["trakt", "imdb"]);
    }
}

/// Playback progress from the user's MDBList API key (`GET /sync/playback?apikey=`), used when
/// MDBList is selected as the Watch Progress source without an OAuth device connection.
pub async fn playback(store: &AppStore, profile: i64, content_id: &str) -> Vec<crate::services::tracking::RemoteResume> {
    use crate::services::tracking::RemoteResume;
    let Some((key, _)) = key(store, profile) else { return Vec::new() };
    let response = match http()
        .get(format!("https://api.mdblist.com/sync/playback?apikey={}", encode(&key)))
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response,
        _ => return Vec::new(),
    };
    let Ok(body) = response.text().await else { return Vec::new() };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&body) else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(items) = payload.as_array() {
        for item in items {
            let id = item
                .pointer("/media/ids/imdb")
                .or_else(|| item.pointer("/ids/imdb"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if !content_id.is_empty() && !id.eq_ignore_ascii_case(content_id) { continue; }
            out.push(RemoteResume {
                season: item.get("season").and_then(serde_json::Value::as_i64),
                episode: item.get("episode").and_then(serde_json::Value::as_i64),
                progress_percent: item.get("progress").and_then(serde_json::Value::as_f64).unwrap_or(0.0),
                paused_at: item.get("updatedAt").and_then(serde_json::Value::as_str).map(str::to_string),
            });
        }
    }
    out
}
