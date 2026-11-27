//! MDBList (`MdbListRatingsClient`, `MdbListRatingsDecoder`, `MdbListMetadataService` of the
//! original): the ratings of a title from IMDb, TMDB, Rotten Tomatoes (critics and audience),
//! Metacritic, Trakt, Letterboxd and MyAnimeList, in one request with the user's own API key.
//! Answers are kept in memory for a few hours: the free plan allows 1000 requests a day.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub fn api_root() -> &'static str {
    option_env!("NUVIO_MDBLIST_API_URL").unwrap_or("https://api.mdblist.com")
}

const CACHE_FOR: Duration = Duration::from_secs(6 * 3600);

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("mdblist http client")
    })
}

/// `MdbListMetadataService.PROVIDER_PRIORITY_ORDER`.
pub const PROVIDERS: &[&str] = &["imdb", "tmdb", "tomatoes", "metacritic", "trakt", "letterboxd", "audience", "mal"];

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Rating {
    /// One of `PROVIDERS`.
    pub source: String,
    pub value: f64,
    /// 10 (IMDb, MAL), 5 (Letterboxd) or 100.
    pub max: f64,
    /// Certified Fresh / Verified Hot.
    pub certified: bool,
}

/// `MediaRatings.toRatings`.
pub fn parse_ratings(body: &Value) -> Vec<Rating> {
    let keywords: Vec<String> = body
        .get("keywords")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|k| k.get("name").and_then(Value::as_str).or_else(|| k.as_str()))
        .map(|n| n.rsplit('.').next().unwrap_or(n).to_string())
        .collect();
    let mut out: Vec<Rating> = Vec::new();
    for r in body.get("ratings").and_then(Value::as_array).into_iter().flatten() {
        let raw = r.get("source").and_then(Value::as_str).unwrap_or("");
        let source = match raw {
            "popcorn" | "audience" | "tomatoesaudience" => "audience",
            "myanimelist" | "mal" => "mal",
            s if PROVIDERS.contains(&s) => s,
            _ => continue,
        };
        let max = match source {
            "imdb" | "mal" => 10.0,
            "letterboxd" => 5.0,
            _ => 100.0,
        };
        let Some(raw_value) = r.get("value").and_then(Value::as_f64).filter(|v| *v >= 0.0) else { continue };
        let value = match (source, r.get("score").and_then(Value::as_f64)) {
            ("letterboxd", Some(score)) => score / 20.0,
            _ => raw_value,
        };
        if value > max || out.iter().any(|o| o.source == source) {
            continue;
        }
        let certified = match source {
            "tomatoes" => keywords.iter().any(|k| k == "certified-fresh"),
            "audience" => keywords.iter().any(|k| k == "certified-hot"),
            _ => false,
        };
        out.push(Rating { source: source.into(), value, max, certified });
    }
    out.sort_by_key(|r| PROVIDERS.iter().position(|p| *p == r.source).unwrap_or(99));
    out
}

fn errors(status: u16) -> String {
    match status {
        401 | 403 => "Chave do MDBList inválida.".into(),
        404 => "O MDBList não conhece este título.".into(),
        429 => "Limite diário do MDBList atingido; volta a funcionar amanhã.".into(),
        500..=599 => "O MDBList está fora do ar agora.".into(),
        other => format!("O MDBList respondeu com o código {other}."),
    }
}

async fn get(path: &str, key: &str, query: &[(&str, &str)]) -> Result<Value, String> {
    let mut url = reqwest::Url::parse(&format!("{}/{}", api_root(), path.trim_start_matches('/'))).map_err(|_| "Endereço do MDBList inválido.".to_string())?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("apikey", key);
        for (k, v) in query {
            pairs.append_pair(k, v);
        }
    }
    let response = http().get(url).header("Accept", "application/json").send().await.map_err(|e| if e.is_timeout() { "O MDBList demorou demais para responder.".to_string() } else { "Sem conexão com o MDBList.".to_string() })?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(errors(status));
    }
    response.json().await.map_err(|_| "Resposta do MDBList em formato inesperado.".to_string())
}

/// The account of a key (Settings → MDBList → Salvar): its name, or why it was refused.
pub async fn check_key(key: &str) -> Result<Option<String>, String> {
    let body = get("user", key, &[]).await?;
    if body.get("error").is_some() {
        return Err("Chave do MDBList inválida.".into());
    }
    Ok(body.get("username").or_else(|| body.get("name")).and_then(Value::as_str).map(str::to_string))
}

/// Where a title is asked: `imdb/{movie|show}/tt…` or `tmdb/{movie|show}/<n>`.
pub fn path_of(content_type: &str, id: &str) -> Option<String> {
    let media = if content_type == "movie" { "movie" } else { "show" };
    let head = id.trim().split(':').next().unwrap_or("");
    if head.starts_with("tt") && head.len() > 2 {
        return Some(format!("imdb/{media}/{head}/"));
    }
    let tmdb = id.trim().strip_prefix("tmdb:")?.split(':').next()?.trim().parse::<i64>().ok()?;
    Some(format!("tmdb/{media}/{tmdb}/"))
}

static CACHE: Mutex<Option<HashMap<String, (Instant, Vec<Rating>)>>> = Mutex::new(None);

/// Ratings of a title (cached for a few hours, per title).
pub async fn ratings(key: &str, content_type: &str, id: &str) -> Result<Vec<Rating>, String> {
    let path = path_of(content_type, id).ok_or("Este título não tem um identificador que o MDBList conheça.")?;
    if let Some((at, list)) = CACHE.lock().expect("mdblist cache").get_or_insert_with(HashMap::new).get(&path) {
        if at.elapsed() < CACHE_FOR {
            return Ok(list.clone());
        }
    }
    let body = get(&path, key, &[("append_to_response", "keyword")]).await?;
    let list = parse_ratings(&body);
    let mut cache = CACHE.lock().expect("mdblist cache");
    let map = cache.get_or_insert_with(HashMap::new);
    if map.len() > 500 {
        map.clear();
    }
    map.insert(path, (Instant::now(), list.clone()));
    Ok(list)
}

/// A new key: the cached answers of the previous one go.
pub fn clear_cache() {
    *CACHE.lock().expect("mdblist cache") = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ratings_are_normalized_and_ordered() {
        let body = json!({
            "ratings": [
                { "source": "letterboxd", "value": 3.9, "score": 78 },
                { "source": "tomatoes", "value": 93 },
                { "source": "imdb", "value": 8.1 },
                { "source": "popcorn", "value": 88 },
                { "source": "imdb", "value": 1.0 },
                { "source": "metacritic", "value": null },
                { "source": "rogerebert", "value": 4 },
                { "source": "myanimelist", "value": 11.0 }
            ],
            "keywords": [{ "name": "certified-fresh" }]
        });
        let list = parse_ratings(&body);
        assert_eq!(list.iter().map(|r| r.source.as_str()).collect::<Vec<_>>(), vec!["imdb", "tomatoes", "letterboxd", "audience"]);
        assert_eq!(list[0].value, 8.1);
        assert!(list[1].certified && !list[3].certified);
        assert!((list[2].value - 3.9).abs() < 1e-9 && list[2].max == 5.0);
    }

    #[test]
    fn paths() {
        assert_eq!(path_of("movie", "tt123").as_deref(), Some("imdb/movie/tt123/"));
        assert_eq!(path_of("series", "tt123:1:2").as_deref(), Some("imdb/show/tt123/"));
        assert_eq!(path_of("series", "tmdb:55").as_deref(), Some("tmdb/show/55/"));
        assert_eq!(path_of("movie", "kitsu:1"), None);
    }
}
