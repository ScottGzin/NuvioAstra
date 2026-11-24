//! Times of intros, recaps and credits from the public services the original uses
//! (`SkipIntroRepository.kt`): IntroDB by IMDb id, then AniSkip by MyAnimeList id.
//!
//! The original maps anime ids with a Simkl key that its open-source build does not carry; ARM
//! (Fribb's anime lists) gives the same mapping without a key. When a season is split in several
//! entries the episode numbers cannot be mapped safely, so AniSkip is not asked: a wrong skip is
//! worse than none. Only the title's public id and episode are sent, and only while the skip
//! buttons are on.

use nuvio_core::client::AddonClient;
use nuvio_core::skip::{parse_animeskip, parse_aniskip, parse_introdb, Kind, Segment};
use serde_json::{json, Value};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(8);

fn introdb_base() -> &'static str {
    option_env!("NUVIO_INTRODB_API_URL").unwrap_or("https://api.introdb.app")
}

fn aniskip_base() -> &'static str {
    option_env!("NUVIO_ANISKIP_API_URL").unwrap_or("https://api.aniskip.com/v2")
}

fn mapping_base() -> &'static str {
    option_env!("NUVIO_ANIME_MAPPING_API_URL").unwrap_or("https://arm.haglund.dev/api/v2")
}

fn anime_skip_base() -> &'static str {
    option_env!("NUVIO_ANIMESKIP_API_URL").unwrap_or("https://api.anime-skip.com")
}

/// Personal keys of the original's Anime-Skip / IntroDB services. They stay in Rust.
#[derive(Debug, Clone, Default)]
pub struct SkipConfig {
    pub anime_skip_enabled: bool,
    pub anime_skip_client_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Imdb { id: String, episode: Option<(i64, i64)> },
    Mal { id: u64, episode: i64 },
    Kitsu { id: u64, episode: i64 },
}

/// `SkipIntroRepository` lookup by video id; the season and episode of the list win.
pub fn target(parent_id: &str, video_id: &str, season: Option<i64>, episode: Option<i64>, movie: bool) -> Option<Target> {
    let parts: Vec<&str> = video_id.trim().split(':').collect();
    if let ("mal" | "kitsu", Some(Ok(id))) = (parts[0], parts.get(1).map(|v| v.parse::<u64>())) {
        // These ids number the episodes of their own entry.
        let own = if parts.len() >= 3 { parts[parts.len() - 1].parse::<i64>().ok() } else { None };
        let episode = own.or(episode).or(movie.then_some(1)).filter(|&e| e > 0)?;
        return Some(if parts[0] == "mal" { Target::Mal { id, episode } } else { Target::Kitsu { id, episode } });
    }
    let is_imdb = |id: &str| id.strip_prefix("tt").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let id = [parts[0], parent_id.trim().split(':').next().unwrap_or_default()].into_iter().find(|id| is_imdb(id))?.to_string();
    if movie {
        return Some(Target::Imdb { id, episode: None });
    }
    let part = |i: usize| parts.get(i).and_then(|v| v.parse::<i64>().ok());
    let season = season.or_else(|| part(1)).filter(|&s| s > 0)?;
    let episode = episode.or_else(|| part(2)).filter(|&e| e > 0)?;
    Some(Target::Imdb { id, episode: Some((season, episode)) })
}

/// The MyAnimeList id of one season in ARM's answer for an IMDb id, when only one entry can be it.
fn mal_of_season(payload: &str, season: i64) -> Option<u64> {
    let root: Value = serde_json::from_str(payload).ok()?;
    let entries = root.as_array()?;
    let season_of = |e: &Value| e.get("thetvdb-season").and_then(Value::as_i64).or_else(|| e.get("themoviedb-season").and_then(Value::as_i64));
    let mal = |e: &Value| e.get("myanimelist").and_then(Value::as_u64);
    if season == 1 && entries.len() == 1 {
        return mal(&entries[0]);
    }
    let same: Vec<&Value> = entries.iter().filter(|e| season_of(e) == Some(season)).collect();
    if let [only] = same.as_slice() {
        return mal(only);
    }
    let series: Vec<&&Value> = same.iter().filter(|e| e.get("media").and_then(Value::as_str) == Some("TV")).collect();
    match series.as_slice() {
        [only] => mal(only),
        _ => None,
    }
}

/// The AniList id of one season in ARM's answer, only when a single entry can be it.
fn anilist_of_season(root: &Value, season: i64) -> Option<u64> {
    let entries = root.as_array()?;
    let season_of = |e: &Value| e.get("thetvdb-season").and_then(Value::as_i64).or_else(|| e.get("themoviedb-season").and_then(Value::as_i64));
    let anilist = |e: &Value| e.get("anilist").and_then(Value::as_u64);
    if season == 1 && entries.len() == 1 {
        return anilist(&entries[0]);
    }
    let same: Vec<&Value> = entries.iter().filter(|e| season_of(e) == Some(season)).collect();
    // A split season has several independent AniList entries: do not guess.
    let candidates: Vec<&Value> = same.iter().copied().filter(|e| e.get("media").and_then(Value::as_str) == Some("TV")).collect();
    match (candidates.as_slice(), same.as_slice()) {
        ([only], _) => anilist(only),
        ([], [only]) => anilist(only),
        _ => None,
    }
}

async fn get(client: &AddonClient, url: String) -> Option<std::sync::Arc<str>> {
    // Not an error for the user: the player just has no button.
    tokio::time::timeout(TIMEOUT, client.get_text(&url, false)).await.ok()?.ok()
}

async fn aniskip(client: &AddonClient, mal: u64, episode: i64) -> Vec<Segment> {
    let url = format!(
        "{}/skip-times/{mal}/{episode}?types=op&types=ed&types=recap&types=mixed-op&types=mixed-ed&episodeLength=0",
        aniskip_base()
    );
    get(client, url).await.and_then(|payload| parse_aniskip(&payload).ok()).unwrap_or_default()
}

async fn graphql(client: &reqwest::Client, query: &str, client_id: &str) -> Option<Value> {
    let response = tokio::time::timeout(
        TIMEOUT,
        client
            .post(format!("{}/graphql", anime_skip_base()))
            .header("X-Client-ID", client_id)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({ "query": query }))
            .send(),
    )
    .await
    .ok()?
    .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().await.ok()
}

/// Anime-Skip as an extra source: AniList id → show id → this episode's timestamp markers.
/// Both queries need the user's own client id; no mapping means no segments, never a guess.
async fn anime_skip(client_id: &str, anilist: u64, season: Option<i64>, episode: i64) -> Vec<Segment> {
    if client_id.trim().is_empty() {
        return Vec::new();
    }
    let Some(http) = reqwest::Client::builder().build().ok() else { return Vec::new() };
    let show_query = format!("{{ findShowsByExternalId(service: ANILIST, serviceId: \"{anilist}\") {{ id }} }}");
    let Some(shows_response) = graphql(&http, &show_query, client_id).await else { return Vec::new() };
    let Some(shows) = shows_response
        .pointer("/data/findShowsByExternalId")
        .and_then(Value::as_array)
    else { return Vec::new() };
    shows
        .iter()
        .filter_map(|show| show.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    for show_id in shows {
        let episodes_query = format!(
            "{{ findEpisodesByShowId(showId: \"{show_id}\") {{ season number baseDuration timestamps {{ at type {{ name }} }} }} }}"
        );
        if let Some(body) = graphql(&http, &episodes_query, client_id).await {
            let segments = parse_animeskip(&body, season, episode);
            if !segments.is_empty() {
                return segments;
            }
        }
    }
    Vec::new()
}

/// The segments of each service, by priority.
pub async fn fetch(client: &AddonClient, target: &Target, config: &SkipConfig) -> Vec<Vec<Segment>> {
    let anime_skip = |anilist: Option<u64>, season: Option<i64>, episode: i64| async move {
        if !config.anime_skip_enabled {
            return Vec::new();
        }
        match anilist {
            Some(anilist) => anime_skip(&config.anime_skip_client_id, anilist, season, episode).await,
            None => Vec::new(),
        }
    };
    match target {
        Target::Mal { id, episode } => {
            let mapped = get(client, format!("{}/ids?source=myanimelist&id={id}", mapping_base())).await;
            let anilist = mapped.as_ref().and_then(|p| serde_json::from_str::<Value>(p).ok())
                .and_then(|v| v.get("anilist").and_then(Value::as_u64));
            vec![
                aniskip(client, *id, *episode).await,
                anime_skip(anilist, None, *episode).await,
            ]
        }
        Target::Kitsu { id, episode } => {
            let mapped = get(client, format!("{}/ids?source=kitsu&id={id}", mapping_base())).await;
            let value = mapped.as_ref().and_then(|p| serde_json::from_str::<Value>(p).ok());
            let mal = value.as_ref().and_then(|v| v.get("myanimelist").and_then(Value::as_u64));
            let anilist = value.as_ref().and_then(|v| v.get("anilist").and_then(Value::as_u64));
            let mut out = Vec::new();
            if let Some(mal) = mal {
                out.push(aniskip(client, mal, *episode).await);
            }
            let extra = anime_skip(anilist, None, *episode).await;
            if !extra.is_empty() {
                out.push(extra);
            }
            out
        }
        Target::Imdb { id, episode } => {
            let query = match episode {
                Some((season, episode)) => format!("season={season}&episode={episode}"),
                None => "is_movie=true".into(),
            };
            let introdb = get(client, format!("{}/segments?imdb_id={id}&{query}", introdb_base()))
                .await
                .and_then(|payload| parse_introdb(&payload, episode.is_none()).ok())
                .unwrap_or_default();
            let mut out = vec![introdb];
            if let Some((season, number)) = episode {
                let missing = [Kind::Intro, Kind::Outro].iter().any(|kind| !out[0].iter().any(|s| s.kind == *kind));
                if missing || config.anime_skip_enabled {
                    let mapping = get(client, format!("{}/imdb?id={id}", mapping_base())).await;
                    let value = mapping.as_ref().and_then(|p| serde_json::from_str::<Value>(p).ok());
                    if missing {
                        if let Some(mal) = mapping.as_deref().and_then(|p| mal_of_season(p, *season)) {
                            out.push(aniskip(client, mal, *number).await);
                        }
                    }
                    let anilist = value.as_ref().and_then(|v| anilist_of_season(v, *season));
                    let extra = anime_skip(anilist, Some(*season), *number).await;
                    if !extra.is_empty() {
                        out.push(extra);
                    }
                }
            }
            out
        }
    }
}

/// IntroDB `POST /submit` (`SkipIntroRepository.submitIntro`). Times are sent in both seconds and
/// milliseconds, exactly as the original does.
pub async fn submit_intro(
    api_key: &str,
    imdb_id: &str,
    season: i64,
    episode: i64,
    segment_type: &str,
    start: f64,
    end: f64,
) -> Result<bool, String> {
    if api_key.trim().is_empty() || !start.is_finite() || !end.is_finite() || end <= start {
        return Err("Tempos inválidos ou chave da IntroDB ausente.".into());
    }
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let response = tokio::time::timeout(
        TIMEOUT,
        client
            .post(format!("{}/submit", introdb_base()))
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&json!({
                "imdb_id": imdb_id,
                "season": season,
                "episode": episode,
                "start_sec": start,
                "end_sec": end,
                "start_ms": (start * 1000.0).round() as i64,
                "end_ms": (end * 1000.0).round() as i64,
                "segment_type": segment_type,
            }))
            .send(),
    )
    .await
    .map_err(|_| "A IntroDB demorou demais para responder.".to_string())?
    .map_err(|e| e.to_string())?;
    Ok(response.status() == reqwest::StatusCode::OK || response.status() == reqwest::StatusCode::CREATED)
}

/// The original's key check quirk: an empty body returns 400 for a *valid* key (auth passes,
/// payload is rejected), 200/201 also means valid, 401/403 means invalid.
pub async fn verify_introdb_key(api_key: &str) -> Result<bool, String> {
    if api_key.trim().is_empty() {
        return Ok(false);
    }
    let client = reqwest::Client::builder().build().map_err(|e| e.to_string())?;
    let response = tokio::time::timeout(
        TIMEOUT,
        client
            .post(format!("{}/submit", introdb_base()))
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .body("{}")
            .send(),
    )
    .await
    .map_err(|_| "A IntroDB demorou demais para responder.".to_string())?
    .map_err(|e| e.to_string())?;
    Ok(matches!(
        response.status(),
        reqwest::StatusCode::OK | reqwest::StatusCode::CREATED | reqwest::StatusCode::BAD_REQUEST
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_of_the_addons() {
        let imdb = |s, e| Some(Target::Imdb { id: "tt2560140".into(), episode: Some((s, e)) });
        assert_eq!(target("tt2560140", "tt2560140:1:3", None, None, false), imdb(1, 3));
        assert_eq!(target("tt2560140", "tt2560140:1:3", Some(2), Some(4), false), imdb(2, 4));
        assert_eq!(target("kitsu:7442", "kitsu:7442:3", Some(1), Some(3), false), Some(Target::Kitsu { id: 7442, episode: 3 }));
        assert_eq!(target("mal:16498", "mal:16498:5", None, None, false), Some(Target::Mal { id: 16498, episode: 5 }));
        assert_eq!(target("tmdb:1", "tmdb:1:1:1", Some(1), Some(1), false), None);
    }

    #[test]
    fn anilist_ids_are_only_used_for_unambiguous_seasons() {
        let list: Value = serde_json::from_str(r#"[
            {"anilist":16498,"media":"TV","thetvdb-season":1},
            {"anilist":18397,"media":"OVA","thetvdb-season":0},
            {"anilist":35760,"media":"TV","thetvdb-season":3},
            {"anilist":38524,"media":"TV","thetvdb-season":3}
        ]"#).unwrap();
        assert_eq!(anilist_of_season(&list, 1), Some(16498));
        assert_eq!(anilist_of_season(&list, 3), None);
    }

    #[test]
    fn a_split_season_is_not_guessed() {
        let list = r#"[
            {"myanimelist":16498,"media":"TV","thetvdb-season":1},
            {"myanimelist":18397,"media":"OVA","thetvdb-season":0},
            {"myanimelist":35760,"media":"TV","thetvdb-season":3},
            {"myanimelist":38524,"media":"TV","thetvdb-season":3}
        ]"#;
        assert_eq!((mal_of_season(list, 1), mal_of_season(list, 3)), (Some(16498), None));
    }
}
