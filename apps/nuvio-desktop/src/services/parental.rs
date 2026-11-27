//! Content warnings shown when a movie or episode starts (the original's `ParentalGuideRepository`):
//! IMDb's parents guide through the same public service the original uses, asked once per title
//! (only its IMDb id is sent) and kept in memory.

use crate::AppState;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

const BASE: &str = "https://api.tiffara.com";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    /// `nudity`, `violence`, `profanity`, `alcohol`, `frightening`.
    pub category: &'static str,
    /// `severe`, `moderate`, `mild`.
    pub severity: String,
}

#[derive(Default)]
pub struct Guides(Mutex<HashMap<String, Vec<Warning>>>);

/// `resolveParentalGuideSeverity`: the most voted level above "none", if it beats "none".
fn severity(category: Option<&Value>) -> Option<String> {
    let rows = category?.get("severityBreakdowns")?.as_array()?;
    let votes = |r: &Value| r.get("voteCount").and_then(Value::as_i64).unwrap_or(0);
    let level = |r: &Value| r.get("severityLevel").and_then(Value::as_str).unwrap_or_default().to_lowercase();
    let none = rows.iter().find(|r| level(r) == "none").map(votes).unwrap_or(0);
    let top = rows.iter().filter(|r| level(r) != "none").max_by_key(|r| votes(r))?;
    (votes(top) > none).then(|| level(top))
}

/// `mapParentalGuideCategoriesToResult` + `buildParentalWarnings` (severe first, at most 5).
pub fn warnings(body: &Value) -> Vec<Warning> {
    let categories = body.get("parentsGuide").and_then(Value::as_array).cloned().unwrap_or_default();
    let find = |name: &str| categories.iter().find(|c| c.get("category").and_then(Value::as_str).is_some_and(|v| v.eq_ignore_ascii_case(name)));
    let mut out: Vec<Warning> = [
        ("nudity", "SEXUAL_CONTENT"),
        ("violence", "VIOLENCE"),
        ("profanity", "PROFANITY"),
        ("alcohol", "ALCOHOL_DRUGS"),
        ("frightening", "FRIGHTENING_INTENSE_SCENES"),
    ]
    .into_iter()
    .filter_map(|(category, name)| severity(find(name)).map(|severity| Warning { category, severity }))
    .collect();
    let rank = |s: &str| match s {
        "severe" => 0,
        "moderate" => 1,
        "mild" => 2,
        _ => 3,
    };
    out.sort_by_key(|w| rank(&w.severity));
    out.truncate(5);
    out
}

fn imdb_id(value: &str) -> Option<String> {
    let at = value.find("tt")?;
    let digits: String = value[at + 2..].chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then(|| format!("tt{digits}"))
}

/// The warnings of a title (empty when off in Settings, without an IMDb id or without data).
pub async fn guide(state: &AppState, id: String) -> Result<Vec<Warning>, String> {
    let enabled = state.app_store.profile_settings(state.active_profile()).map(|s| s.playback.show_parental_guide).unwrap_or(true);
    let Some(id) = imdb_id(&id).filter(|_| enabled) else { return Ok(Vec::new()) };
    if let Some(found) = state.parental.0.lock().expect("guides").get(&id) {
        return Ok(found.clone());
    }
    let url = format!("{}/titles/{id}/parentsGuide", option_env!("NUVIO_PARENTAL_API_URL").unwrap_or(BASE));
    let found = match state.addons.client.get_text(&url, false).await {
        Ok(text) => serde_json::from_str::<Value>(&text).map(|body| warnings(&body)).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    state.parental.0.lock().expect("guides").insert(id, found.clone());
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_follow_the_original_votes() {
        let body = serde_json::json!({"parentsGuide": [
            {"category": "VIOLENCE", "severityBreakdowns": [{"severityLevel": "none", "voteCount": 2}, {"severityLevel": "severe", "voteCount": 30}, {"severityLevel": "mild", "voteCount": 5}]},
            {"category": "PROFANITY", "severityBreakdowns": [{"severityLevel": "none", "voteCount": 40}, {"severityLevel": "moderate", "voteCount": 10}]},
            {"category": "ALCOHOL_DRUGS", "severityBreakdowns": [{"severityLevel": "mild", "voteCount": 8}]}
        ]});
        let w = warnings(&body);
        assert_eq!(w, vec![Warning { category: "violence", severity: "severe".into() }, Warning { category: "alcohol", severity: "mild".into() }]);
        assert_eq!(imdb_id("tt0133093:1:2").as_deref(), Some("tt0133093"));
        assert_eq!(imdb_id("kitsu:1"), None);
    }
}
