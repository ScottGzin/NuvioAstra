//! Live TV → "Jogos de hoje" (from aelrased's proposal): the day's games with scores, from ESPN's
//! public scoreboard (no key, nothing from the channel addons). "Onde passa" — only when the user
//! asks, for one game — reads the guide of up to 16 sports channels (reusing guides already read)
//! and lists the ones whose programs name both teams around kick-off. Nothing is guessed from a
//! channel's name alone.

use super::epg::{self, EpgStore};
use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_core::url::build_resource_url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MINUTE: i64 = 60_000;
const CACHE_FOR: Duration = Duration::from_secs(5 * 60);
/// Guides read for one "Onde passa".
const MAX_GUIDES: usize = 16;

fn espn_root() -> &'static str {
    option_env!("NUVIO_ESPN_API_URL").unwrap_or("https://site.api.espn.com/apis/site/v2/sports")
}

/// ESPN league path, name, and whether the event is one race/card (F1, UFC) rather than two teams.
const LEAGUES: &[(&str, &str, bool)] = &[
    ("soccer/bra.1", "Brasileirão Série A", false),
    ("soccer/bra.2", "Brasileirão Série B", false),
    ("soccer/bra.copa_do_brazil", "Copa do Brasil", false),
    ("soccer/conmebol.libertadores", "Libertadores", false),
    ("soccer/conmebol.sudamericana", "Sul-Americana", false),
    ("soccer/uefa.champions", "Champions League", false),
    ("soccer/uefa.europa", "Europa League", false),
    ("soccer/eng.1", "Premier League", false),
    ("soccer/esp.1", "LaLiga", false),
    ("soccer/ita.1", "Serie A", false),
    ("soccer/ger.1", "Bundesliga", false),
    ("soccer/fra.1", "Ligue 1", false),
    ("soccer/ksa.1", "Saudi Pro League", false),
    ("basketball/nba", "NBA", false),
    ("football/nfl", "NFL", false),
    ("hockey/nhl", "NHL", false),
    ("mma/ufc", "UFC", true),
    ("racing/f1", "Fórmula 1", true),
];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub name: String,
    pub short: String,
    pub logo: Option<String>,
    pub score: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub id: String,
    pub league: String,
    pub league_logo: Option<String>,
    pub name: String,
    pub start_ms: i64,
    /// `upcoming`, `live` or `finished`.
    pub state: &'static str,
    /// ESPN's status ("67'", "HT", "FT"…).
    pub status: String,
    pub home: Option<Team>,
    pub away: Option<Team>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchesView {
    pub matches: Vec<Match>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------- dates (no date crate)

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}")
}

/// `2026-11-24T20:00Z` / `2026-11-24T20:00:00Z` (ESPN) as Unix ms.
fn parse_espn_date(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> { std::str::from_utf8(b.get(from..from + len)?).ok()?.parse().ok() };
    let (y, mo, d, h, mi) = (num(0, 4)?, num(5, 2)?, num(8, 2)?, num(11, 2)?, num(14, 2)?);
    let s = if b.get(16) == Some(&b':') { num(17, 2).unwrap_or(0) } else { 0 };
    Some(days_from_civil(y, mo, d) * 86_400_000 + (h * 3600 + mi * 60 + s) * 1000)
}

// ---------------------------------------------------------------- ESPN

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

fn parse_team(team: &Value) -> Option<Team> {
    let name = text(team.get("shortDisplayName")).or_else(|| text(team.get("displayName"))).or_else(|| text(team.get("name")))?;
    let short = text(team.get("abbreviation")).unwrap_or_else(|| name.clone());
    let logo = text(team.get("logo")).or_else(|| team.pointer("/logos/0/href").and_then(|v| text(Some(v))));
    Some(Team { name, short, logo, score: None })
}

fn parse_event(league: (&str, &str, bool), event: &Value, league_logo: Option<&str>) -> Option<Match> {
    let comp = event.get("competitions")?.as_array()?.first()?;
    let kind = event.get("status").or_else(|| comp.get("status"))?.get("type")?;
    let state = match text(kind.get("state")).as_deref() {
        Some("in") => "live",
        Some("post") => "finished",
        _ => "upcoming",
    };
    let competitors = comp.get("competitors").and_then(Value::as_array).cloned().unwrap_or_default();
    let side = |home: bool| -> Option<Team> {
        let entry = competitors.iter().find(|c| c.get("homeAway").and_then(Value::as_str) == Some(if home { "home" } else { "away" }))?;
        let mut team = parse_team(entry.get("team")?)?;
        if state != "upcoming" {
            team.score = entry.get("score").and_then(|s| s.as_str().map(str::to_string).or_else(|| s.as_i64().map(|v| v.to_string())));
        }
        Some(team)
    };
    let (home, away) = if league.2 { (None, None) } else { (side(true), side(false)) };
    Some(Match {
        id: format!("{}/{}", league.0, text(event.get("id")).unwrap_or_default()),
        league: league.1.to_string(),
        league_logo: league_logo.map(str::to_string),
        name: text(event.get("name")).or_else(|| text(event.get("shortName"))).unwrap_or_default(),
        start_ms: text(event.get("date")).and_then(|d| parse_espn_date(&d)).unwrap_or(0),
        state,
        status: text(kind.get("shortDetail")).or_else(|| text(kind.get("detail"))).unwrap_or_default(),
        home,
        away,
    })
}

pub fn parse_scoreboard(league: (&str, &str, bool), body: &Value) -> Vec<Match> {
    let logo = body.pointer("/leagues/0/logos/0/href").and_then(|v| text(Some(v)));
    body.get("events").and_then(Value::as_array).into_iter().flatten().filter_map(|e| parse_event(league, e, logo.as_deref())).collect()
}

struct Cache {
    day: String,
    at: Instant,
    matches: Vec<Match>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(15)).build().expect("espn client"))
}

/// Every league for one ESPN day, at the same time; a league that fails is left out.
async fn fetch_day(day: String) -> (Vec<Match>, usize) {
    let mut set = tokio::task::JoinSet::new();
    for &league in LEAGUES {
        let day = day.clone();
        set.spawn(async move {
            let url = format!("{}/{}/scoreboard?dates={day}", espn_root(), league.0);
            let body: Value = http().get(&url).send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
            Some(parse_scoreboard(league, &body))
        });
    }
    let (mut out, mut failed) = (Vec::new(), 0);
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Some(mut list)) => out.append(&mut list),
            _ => failed += 1,
        }
    }
    (out, failed)
}

/// The games of the user's local day (`tz_offset_min` as JavaScript's `getTimezoneOffset`).
pub async fn today(now_ms: i64, tz_offset_min: i64) -> MatchesView {
    let local = now_ms - tz_offset_min * MINUTE;
    let day_index = local.div_euclid(86_400_000);
    let day = civil_from_days(day_index);
    if let Some(c) = CACHE.lock().expect("matches").as_ref().filter(|c| c.day == day && c.at.elapsed() < CACHE_FOR) {
        return MatchesView { matches: c.matches.clone(), error: None };
    }
    // ESPN days follow US time: the next day carries this evening's late games in Brazil.
    let ((mut a, fa), (mut b, fb)) = tokio::join!(fetch_day(day.clone()), fetch_day(civil_from_days(day_index + 1)));
    a.append(&mut b);
    let start = day_index * 86_400_000 + tz_offset_min * MINUTE;
    let end = start + 86_400_000;
    let mut seen = std::collections::HashSet::new();
    let mut matches: Vec<Match> = a.into_iter().filter(|m| m.start_ms >= start && m.start_ms < end && seen.insert(m.id.clone())).collect();
    matches.sort_by_key(|m| (match m.state { "live" => 0, "upcoming" => 1, _ => 2 }, m.start_ms));
    if matches.is_empty() && fa + fb == 2 * LEAGUES.len() {
        let stale = CACHE.lock().expect("matches").as_ref().filter(|c| c.day == day).map(|c| c.matches.clone()).unwrap_or_default();
        return MatchesView { matches: stale, error: Some("Não foi possível buscar os jogos agora.".into()) };
    }
    *CACHE.lock().expect("matches") = Some(Cache { day, at: Instant::now(), matches: matches.clone() });
    MatchesView { matches, error: None }
}

pub fn cached_match(id: &str) -> Option<Match> {
    CACHE.lock().expect("matches").as_ref()?.matches.iter().find(|m| m.id == id).cloned()
}

// ---------------------------------------------------------------- "Onde passa"

/// A channel of the Live TV list, as the interface sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelLite {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub logo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRef {
    pub key: String,
    pub name: String,
    pub logo: Option<String>,
    /// The program that named the game (why this channel).
    pub program: String,
}

const GENERIC: &[&str] = &["united", "city", "real", "club", "clube", "sporting", "athletic", "atletico", "inter", "state", "university", "town", "county", "rovers", "wanderers", "olympique", "racing", "deportivo", "sport", "sports", "team", "new", "saint", "esporte"];

fn fold(value: &str) -> String {
    let mapped: String = value
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c => c,
        })
        .collect();
    mapped.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' }).collect()
}

fn tokens(value: &str) -> Vec<String> {
    fold(value).split_whitespace().filter(|t| t.len() >= 4 && !GENERIC.contains(t)).map(str::to_string).collect()
}

fn mentions(haystack: &str, needles: &[String]) -> bool {
    needles.iter().any(|t| if t.len() >= 6 { haystack.contains(t.as_str()) } else { haystack.contains(&format!(" {t} ")) })
}

/// How many sides of the game a program names (both teams, or the event for F1/UFC).
pub fn match_score(m: &Match, program: &str) -> usize {
    let haystack = format!(" {} ", fold(program));
    let sides: Vec<Vec<String>> = match (&m.home, &m.away) {
        (Some(h), Some(a)) => vec![[tokens(&h.name), tokens(&h.short)].concat(), [tokens(&a.name), tokens(&a.short)].concat()],
        _ => vec![[tokens(&m.name), tokens(&m.league)].concat()],
    };
    sides.iter().filter(|side| !side.is_empty() && mentions(&haystack, side)).count()
}

/// Channels whose name or category looks like a sports feed: only their guides are read.
pub fn is_sports_channel(c: &ChannelLite) -> bool {
    let haystack = fold(&format!("{} {}", c.name, c.genres.join(" ")));
    ["sport", "espn", "bein", "fox sports", "tnt sports", "dazn", "premiere", "sportv", "combate", "band sports", "nba", "nfl", "nhl", "ufc", "f1", "esporte", "futebol", "futbol", "football", "soccer"].iter().any(|k| haystack.contains(k))
}

async fn guide_for(client: &AddonClient, manifests: &[AddonManifest], store: &EpgStore, key: &str, now: i64, tz: i64) -> Vec<epg::Program> {
    if let Some(view) = store.cached(key, now) {
        return view.programs;
    }
    let Some((instance, content_type, meta_id)) = super::livetv::split_key(key) else { return Vec::new() };
    let Some(owner) = manifests.iter().find(|m| super::addons::addon_instance_id(m) == instance) else { return Vec::new() };
    if !owner.supports_resource("meta", content_type, meta_id) {
        return Vec::new();
    }
    let url = build_resource_url(&owner.transport_url, "meta", content_type, meta_id, None);
    let Ok(text) = client.get_text(&url, false).await else { return Vec::new() };
    let description = nuvio_core::meta::parse_meta(&text).ok().and_then(|m| m.description).unwrap_or_default();
    store.update(key, epg::parse_schedule(&description, now, tz), now).programs
}

/// "Onde passa" for one game: the sports channels (at most 16 guides) whose program around
/// kick-off names it.
pub async fn where_to_watch(client: &AddonClient, manifests: &[AddonManifest], store: &EpgStore, m: &Match, channels: &[ChannelLite], now: i64, tz: i64) -> Vec<ChannelRef> {
    let needed = if m.home.is_some() { 2 } else { 1 };
    let mut found = Vec::new();
    for c in channels.iter().filter(|c| is_sports_channel(c)).take(MAX_GUIDES) {
        let programs = guide_for(client, manifests, store, &c.key, now, tz).await;
        let best = programs
            .iter()
            .filter(|p| p.end_ms > m.start_ms - 30 * MINUTE && p.start_ms < m.start_ms + 150 * MINUTE)
            .map(|p| (match_score(m, &format!("{} {}", p.title, p.description.as_deref().unwrap_or(""))), p))
            .filter(|(score, _)| *score >= needed)
            .max_by_key(|(score, _)| *score);
        if let Some((_, p)) = best {
            found.push(ChannelRef { key: c.key.clone(), name: c.name.clone(), logo: c.logo.clone(), program: p.title.clone() });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(days_from_civil(2026, 11, 26)), "20261126");
        assert_eq!(parse_espn_date("2026-11-26T22:30Z"), Some(days_from_civil(2026, 11, 26) * 86_400_000 + (22 * 60 + 30) * 60_000));
        assert_eq!(parse_espn_date("2026-11-26T22:30:15Z").unwrap() % 60_000, 15_000);
        assert_eq!(parse_espn_date("x"), None);
    }

    #[test]
    fn scoreboard_is_read() {
        let body = json!({ "leagues": [{ "logos": [{ "href": "https://l/bra.png" }] }], "events": [{
            "id": "1", "name": "Flamengo at Palmeiras", "date": "2026-11-26T22:30Z",
            "status": { "type": { "state": "in", "shortDetail": "67'" } },
            "competitions": [{ "competitors": [
                { "homeAway": "home", "score": "2", "team": { "shortDisplayName": "Palmeiras", "abbreviation": "PAL", "logo": "https://l/pal.png" } },
                { "homeAway": "away", "score": "1", "team": { "shortDisplayName": "Flamengo", "abbreviation": "FLA" } }
            ] }]
        }] });
        let list = parse_scoreboard(("soccer/bra.1", "Brasileirão Série A", false), &body);
        assert_eq!(list.len(), 1);
        let m = &list[0];
        assert_eq!((m.state, m.status.as_str(), m.id.as_str()), ("live", "67'", "soccer/bra.1/1"));
        assert_eq!(m.home.as_ref().unwrap().score.as_deref(), Some("2"));
        assert_eq!(m.league_logo.as_deref(), Some("https://l/bra.png"));
    }

    #[test]
    fn programs_must_name_both_teams() {
        let team = |n: &str, s: &str| Some(Team { name: n.into(), short: s.into(), logo: None, score: None });
        let m = Match { id: "x".into(), league: "Brasileirão".into(), league_logo: None, name: "".into(), start_ms: 0, state: "live", status: "".into(), home: team("São Paulo", "SAO"), away: team("Grêmio", "GRE") };
        assert_eq!(match_score(&m, "Futebol: São Paulo x Grêmio - ao vivo"), 2);
        assert_eq!(match_score(&m, "Gols do Grêmio"), 1);
        assert!(is_sports_channel(&ChannelLite { key: "k".into(), name: "SporTV 2".into(), genres: vec![], logo: None }));
        assert!(!is_sports_channel(&ChannelLite { key: "k".into(), name: "Globo News".into(), genres: vec!["Notícias".into()], logo: None }));
    }
}
