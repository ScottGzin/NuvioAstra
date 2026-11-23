//! Times of intros, recaps and credits from the public services the original uses
//! (`SkipIntroRepository.kt`): IntroDB by IMDb id, then AniSkip by MyAnimeList id.
//!
//! The original maps anime ids with a Simkl key that its open-source build does not carry; ARM
//! (Fribb's anime lists) gives the same mapping without a key. When a season is split in several
//! entries the episode numbers cannot be mapped safely, so AniSkip is not asked: a wrong skip is
//! worse than none. Only the title's public id and episode are sent, and only while the skip
//! buttons are on.

use nuvio_core::client::AddonClient;
use nuvio_core::skip::{parse_aniskip, parse_introdb, Kind, Segment};
use serde_json::Value;
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

/// The segments of each service, by priority.
pub async fn fetch(client: &AddonClient, target: &Target) -> Vec<Vec<Segment>> {
    match target {
        Target::Mal { id, episode } => vec![aniskip(client, *id, *episode).await],
        Target::Kitsu { id, episode } => {
            let mapped = get(client, format!("{}/ids?source=kitsu&id={id}", mapping_base()))
                .await
                .and_then(|payload| serde_json::from_str::<Value>(&payload).ok())
                .and_then(|v| v.get("myanimelist").and_then(Value::as_u64));
            match mapped {
                Some(mal) => vec![aniskip(client, mal, *episode).await],
                None => Vec::new(),
            }
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
                if missing {
                    let mapped = get(client, format!("{}/imdb?id={id}", mapping_base())).await.and_then(|p| mal_of_season(&p, *season));
                    if let Some(mal) = mapped {
                        out.push(aniskip(client, mal, *number).await);
                    }
                }
            }
            out
        }
    }
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
