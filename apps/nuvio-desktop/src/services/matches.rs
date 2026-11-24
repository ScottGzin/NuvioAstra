//! Live matches: ESPN's public scoreboard + the channels' own EPG.
//!
//! Fixtures and scores come from ESPN (no key). To find which channel actually carries a game, we
//! read the guide of the sports channels around kick-off and look for the team names in the program
//! title/description — the same idea as VyseTV's `match_finder`, adapted to Stremio `tv` channels.
//! Only channels with a real EPG answer are reported; nothing is guessed from the channel name.

use super::epg::{self, EpgStore};
use super::livetv::Channel;
use nuvio_core::client::AddonClient;
use nuvio_core::manifest::AddonManifest;
use nuvio_core::url::build_resource_url;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CACHE_TTL: Duration = Duration::from_secs(2 * 60);
const MINUTE: i64 = 60_000;
const MAX_GUIDES: usize = 16;
const ESPN: &str = "https://site.api.espn.com/apis/site/v2/sports";

/// ESPN league path, display name and whether the "match" is one event (UFC, F1).
const LEAGUES: &[(&str, &str, bool)] = &[
    ("soccer/uefa.champions", "Champions League", false),
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
    ("racing/f1", "Formula 1", true),
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
    pub status: String,
    pub home: Option<Team>,
    pub away: Option<Team>,
    /// Installed channels whose EPG puts this match in their guide, best first.
    pub channels: Vec<ChannelRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRef {
    pub key: String,
    pub name: String,
    pub logo: Option<String>,
    /// Program title that matched, so the UI can explain the choice.
    pub program: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchesView {
    pub matches: Vec<Match>,
    pub fetched_at_ms: i64,
    pub error: Option<String>,
}

struct Cache {
    day: String,
    at: Instant,
    matches: Vec<Match>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// `Date.getTimezoneOffset()` minutes → local day as YYYYMMDD.
fn local_day(now_ms: i64, offset_min: i64) -> String {
    let local = now_ms - offset_min * MINUTE;
    let days = local.div_euclid(86_400_000);
    civil_from_days(days)
}

/// Howard Hinnant's civil-from-days, for YYYYMMDD without pulling in a date crate.
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

fn parse_espn_date(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 20 {
        return None;
    }
    let num = |from: usize, len: usize| -> Option<i64> {
        std::str::from_utf8(b.get(from..from + len)?).ok()?.parse().ok()
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, s) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let days = days_from_civil(y, mo, d);
    let mut ms = days * 86_400_000 + (h * 3600 + mi * 60 + s) * 1000;
    if let Some(dot) = b.iter().position(|c| *c == b'.') {
        let mut end = dot + 1;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
        let frac = std::str::from_utf8(&b[dot + 1..end]).ok()?;
        let mut millis: i64 = 0;
        for (i, c) in frac.chars().take(3).enumerate() {
            millis += (c as i64 - '0' as i64) * 10i64.pow(2 - i as u32);
        }
        ms += millis;
    }
    // ESPN timestamps end in Z; offsets other than Z are not used by the scoreboard.
    Some(ms)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn text(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(serde_json::Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

fn parse_team(value: Option<&serde_json::Value>, state: &str) -> Option<Team> {
    let team = value?;
    let name = text(team.get("shortDisplayName"))
        .or_else(|| text(team.get("displayName")))
        .or_else(|| text(team.get("name")))?;
    let short = text(team.get("abbreviation")).or_else(|| text(team.get("shortDisplayName"))).unwrap_or_else(|| name.clone());
    let logo = text(team.get("logo"))
        .or_else(|| team.get("logos").and_then(|l| l.get(0)).and_then(|l| text(l.get("href"))))
        .or_else(|| team.get("flag").and_then(|f| text(f.get("href"))));
    // The score lives on the competitor object, not on `team`; the caller fills it in.
    let _ = state;
    Some(Team { name, short, logo, score: None })
}

fn parse_event(league: (&str, &str, bool), event: &serde_json::Value, league_logo: Option<&str>) -> Option<Match> {
    let competitions = event.get("competitions")?.as_array()?;
    let comp = competitions.first()?;
    let status = event.get("status").or_else(|| comp.get("status"))?;
    let kind = status.get("type")?;
    let state = match text(kind.get("state")).as_deref() {
        Some("in") => "live",
        Some("post") => "finished",
        _ => "upcoming",
    };
    let competitors = comp.get("competitors")?.as_array()?;
    let side = |home: bool| -> Option<Team> {
        let entry = competitors.iter().find(|c| {
            c.get("homeAway").and_then(serde_json::Value::as_str) == Some(if home { "home" } else { "away" })
        })?;
        let mut team = parse_team(entry.get("team"), state)?;
        if state != "upcoming" {
            team.score = entry.get("score").and_then(serde_json::Value::as_str).map(str::to_string)
                .or_else(|| entry.get("score").and_then(serde_json::Value::as_i64).map(|v| v.to_string()));
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
        channels: Vec::new(),
    })
}

async fn fetch_day(day: &str) -> Result<Vec<Match>, String> {
    let http = nuvio_core::net::http_client().map_err(|e| e.to_string())?;
    let mut matches = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for league in LEAGUES {
        let url = format!("{ESPN}/{}/scoreboard?dates={day}", league.0);
        let payload = http.get(&url).send().await.map_err(|e| e.to_string())?;
        if !payload.status().is_success() {
            continue;
        }
        let body: serde_json::Value = payload.json().await.map_err(|e| e.to_string())?;
        let logo = body.get("leagues").and_then(|l| l.get(0)).and_then(|l| l.get("logos"))
            .and_then(|l| l.get(0)).and_then(|l| text(l.get("href")));
        for event in body.get("events").and_then(serde_json::Value::as_array).into_iter().flatten() {
            if let Some(m) = parse_event(*league, event, logo.as_deref()) {
                if seen.insert(m.id.clone()) {
                    matches.push(m);
                }
            }
        }
    }
    matches.sort_by_key(|m| m.start_ms);
    Ok(matches)
}

const GENERIC: &[&str] = &[
    "united", "city", "real", "club", "sporting", "athletic", "atletico", "inter", "state", "university",
    "town", "county", "rovers", "wanderers", "olympique", "racing", "deportivo", "sport", "sports",
    "team", "new", "saint",
];

fn fold(value: &str) -> String {
    const FROM: &str = "àáâãäåçèéêëìíîïñòóôõöùúûüýÿ";
    const TO: &str = "aaaaaaceeeeiiiinooooouuuuyy";
    let mut out = String::new();
    for c in value.to_lowercase().chars() {
        if let Some(i) = FROM.find(c) {
            out.push(TO.chars().nth(i / c.len_utf8()).unwrap_or(c));
        } else {
            out.push(c);
        }
    }
    out.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' }).collect()
}

fn tokens(value: &str) -> Vec<String> {
    fold(value)
        .split_whitespace()
        .filter(|t| t.len() >= 4 && !GENERIC.contains(&t))
        .map(str::to_string)
        .collect()
}

fn mentions(haystack: &str, needles: &[String]) -> bool {
    needles.iter().any(|token| {
        if token.len() >= 6 {
            haystack.contains(token)
        } else {
            haystack.contains(&format!(" {token} "))
        }
    })
}

fn match_score(m: &Match, program: &str) -> usize {
    let from = m.start_ms - 30 * MINUTE;
    let to = m.start_ms + 150 * MINUTE;
    let haystack = format!(" {}", fold(program));
    let sides: Vec<Vec<String>> = if let (Some(home), Some(away)) = (&m.home, &m.away) {
        vec![
            [tokens(&home.name), tokens(&home.short)].concat(),
            [tokens(&away.name), tokens(&away.short)].concat(),
        ]
    } else {
        vec![[tokens(&m.name), tokens(&m.league)].concat()]
    };
    sides.iter().filter(|side| !side.is_empty() && mentions(&haystack, side)).count()
}

/// Channels whose name or genre looks like a sports feed. We only fetch guides for these.
fn is_sports_channel(channel: &Channel) -> bool {
    let haystack = fold(&format!("{} {}", channel.name, channel.genres.join(" ")));
    ["sport", "espn", "bein", "sky sports", "fox sports", "tnt sports", "dazn", "nba", "nfl", "nhl", "ufc", "f1", "movistar", "canal plus", "esporte", "futebol", "fodbold"].iter().any(|k| haystack.contains(k))
}

async fn guide_for(client: &AddonClient, manifests: &[AddonManifest], epg: &EpgStore, channel: &Channel, now: i64, tz: i64) -> Vec<epg::Program> {
    if let Some(view) = epg.cached(&channel.key, now) {
        return view.programs;
    }
    let Some((instance, content_type, meta_id)) = super::livetv::split_key(&channel.key) else { return Vec::new() };
    let Some(owner) = manifests.iter().find(|m| super::addons::addon_instance_id(m) == instance) else { return Vec::new() };
    if !owner.supports_resource("meta", content_type, meta_id) {
        return Vec::new();
    }
    let url = build_resource_url(&owner.transport_url, "meta", content_type, meta_id, None);
    let Ok(text) = client.get_text(&url, true).await else { return Vec::new() };
    let description = nuvio_core::meta::parse_meta(&text).ok().and_then(|m| m.description).unwrap_or_default();
    epg.update(&channel.key, epg::parse_schedule(&description, now, tz), now).programs
}

/// Loads today's matches and attaches the installed channels that actually list them.
pub async fn load(client: &AddonClient, manifests: &[AddonManifest], channels: &[Channel], epg: &EpgStore, tz_offset_min: i64) -> MatchesView {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let day = local_day(now, tz_offset_min);
    let cached = {
        let mut cache = CACHE.lock().expect("matches cache");
        cache.as_ref().filter(|c| c.day == day && c.at.elapsed() < CACHE_TTL).map(|c| c.matches.clone())
    };
    let mut matches = match cached {
        Some(m) => m,
        None => {
            let mut all = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut last_error = None;
            // ESPN days are US-centred: ask for the neighbouring days too, then filter by the
            // interface's local day below. Single-day requests work for every league.
            for adjacent in [shift_day(&day, -1), day.clone(), shift_day(&day, 1)] {
                match fetch_day(&adjacent).await {
                    Ok(batch) => {
                        for m in batch {
                            if seen.insert(m.id.clone()) {
                                all.push(m);
                            }
                        }
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            if all.is_empty() && last_error.is_some() {
                let mut cache = CACHE.lock().expect("matches cache");
                let stale = cache.as_ref().filter(|c| c.day == day).map(|c| c.matches.clone()).unwrap_or_default();
                return MatchesView { matches: stale, fetched_at_ms: now, error: last_error };
            }
            all.sort_by_key(|m| m.start_ms);
            let mut cache = CACHE.lock().expect("matches cache");
            *cache = Some(Cache { day: day.clone(), at: Instant::now(), matches: all.clone() });
            all
        }
    };

    let day_start = day_from_string(&day, tz_offset_min);
    let day_end = day_start + 86_400_000;
    matches.retain(|m| m.start_ms >= day_start - 6 * 60 * MINUTE && m.start_ms < day_end + 6 * 60 * MINUTE);

    let candidates: Vec<&Channel> = channels.iter().filter(|c| is_sports_channel(c)).take(MAX_GUIDES).collect();
    let mut guides: HashMap<&str, Vec<epg::Program>> = HashMap::new();
    for channel in candidates {
        let programs = guide_for(client, manifests, epg, channel, now, tz_offset_min).await;
        guides.insert(channel.key.as_str(), programs);
    }

    for m in &mut matches {
        let mut found: Vec<ChannelRef> = Vec::new();
        for channel in channels.iter().filter(|c| is_sports_channel(c)) {
            let Some(programs) = guides.get(channel.key.as_str()) else { continue };
            let best = programs
                .iter()
                .filter(|p| p.end_ms > m.start_ms - 30 * MINUTE && p.start_ms < m.start_ms + 150 * MINUTE)
                .map(|p| (match_score(m, &format!("{} {}", p.title, p.description.as_deref().unwrap_or(""))), p))
                .filter(|(score, _)| *score >= if m.home.is_some() { 2 } else { 1 })
                .max_by_key(|(score, _)| *score);
            if let Some((_, program)) = best {
                found.push(ChannelRef { key: channel.key.clone(), name: channel.name.clone(), logo: channel.logo.clone(), program: program.title.clone() });
            }
        }
        m.channels = found;
    }
    MatchesView { matches, fetched_at_ms: now, error: None }
}

fn shift_day(day: &str, delta: i64) -> String {
    let y: i64 = day.get(0..4).and_then(|v| v.parse().ok()).unwrap_or(1970);
    let m: i64 = day.get(4..6).and_then(|v| v.parse().ok()).unwrap_or(1);
    let d: i64 = day.get(6..8).and_then(|v| v.parse().ok()).unwrap_or(1);
    civil_from_days(days_from_civil(y, m, d) + delta)
}

fn day_from_string(day: &str, offset_min: i64) -> i64 {
    let y: i64 = day.get(0..4).and_then(|v| v.parse().ok()).unwrap_or(1970);
    let m: i64 = day.get(4..6).and_then(|v| v.parse().ok()).unwrap_or(1);
    let d: i64 = day.get(6..8).and_then(|v| v.parse().ok()).unwrap_or(1);
    days_from_civil(y, m, d) * 86_400_000 - offset_min * MINUTE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        for (y, m, d) in [(2026, 1, 1), (2026, 11, 24), (2024, 2, 29), (2000, 3, 1)] {
            let day = format!("{y:04}{m:02}{d:02}");
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), day);
        }
    }

    #[test]
    fn espn_timestamps_parse() {
        assert_eq!(parse_espn_date("2026-11-24T20:00:00Z"), Some(days_from_civil(2026, 11, 24) * 86_400_000 + 20 * 3600_000));
        assert_eq!(parse_espn_date("bad"), None);
    }

    #[test]
    fn team_names_are_folded_and_matched() {
        let m = Match {
            id: "x".into(), league: "Premier League".into(), league_logo: None, name: "Arsenal vs Liverpool".into(),
            start_ms: 1_000_000, state: "live", status: "67'".into(),
            home: Some(Team { name: "Arsenal".into(), short: "ARS".into(), logo: None, score: Some("1".into()) }),
            away: Some(Team { name: "Liverpool".into(), short: "LIV".into(), logo: None, score: Some("2".into()) }),
            channels: Vec::new(),
        };
        assert_eq!(match_score(&m, "Arsenal 1-2 Liverpool ao vivo"), 2);
        assert_eq!(match_score(&m, "Jornal da tarde"), 0);
    }
}
