//! Content warnings from the original's `ParentalGuideRepository`: IMDb's parents guide served by
//! api.tiffara.com. Only non-blank severity labels whose votes beat "none" are reported.

use nuvio_core::client::AddonClient;
use serde::Serialize;
use serde_json::Value;

const BASE_URL: &str = "https://api.tiffara.com";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Guide {
    pub nudity: Option<String>,
    pub violence: Option<String>,
    pub profanity: Option<String>,
    pub alcohol: Option<String>,
    pub frightening: Option<String>,
}

impl Guide {
    pub fn warnings(&self) -> Vec<(&'static str, &str)> {
        [
            ("nudity", self.nudity.as_deref()),
            ("violence", self.violence.as_deref()),
            ("profanity", self.profanity.as_deref()),
            ("alcohol", self.alcohol.as_deref()),
            ("frightening", self.frightening.as_deref()),
        ]
        .into_iter()
        .filter_map(|(label, severity)| severity.map(|s| (label, s)))
        .collect()
    }
}

/// `extractParentalGuideImdbId`: the first `tt\d+` in the given id.
pub fn extract_imdb_id(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    for start in 0..bytes.len().saturating_sub(1) {
        if bytes[start] == b't' && bytes[start + 1] == b't' {
            let mut end = start + 2;
            while end < bytes.len() && bytes[end].is_ascii_digit() { end += 1; }
            if end > start + 2 {
                return Some(input[start..end].to_string());
            }
        }
    }
    None
}

/// `resolveParentalGuideSeverity`: the most voted non-"none" level, but only when it beats the
/// votes for "none".
fn severity(category: Option<&Value>) -> Option<String> {
    let breakdowns = category?.get("severityBreakdowns")?.as_array()?;
    let mut dominant: Option<(i64, &str)> = None;
    let mut none_votes = 0i64;
    for entry in breakdowns {
        let level = entry.get("severityLevel")?.as_str()?;
        let votes = entry.get("voteCount").and_then(Value::as_i64).unwrap_or(0);
        if level.eq_ignore_ascii_case("none") {
            none_votes = votes;
        } else if dominant.is_none_or(|(count, _)| votes > count) {
            dominant = Some((votes, level));
        }
    }
    let (votes, level) = dominant?;
    (votes > none_votes).then(|| level.to_ascii_lowercase())
}

/// `mapParentalGuideCategoriesToResult`, with the category names of the original.
pub fn map_guide(payload: &str) -> Option<Guide> {
    let categories = serde_json::from_str::<Value>(payload).ok()?;
    let categories = categories.get("parentsGuide")?.as_array()?;
    if categories.is_empty() {
        return None;
    }
    let find = |name: &str| categories.iter().find(|c| c.get("category").and_then(Value::as_str).is_some_and(|v| v.eq_ignore_ascii_case(name)));
    Some(Guide {
        nudity: severity(find("SEXUAL_CONTENT")),
        violence: severity(find("VIOLENCE")),
        profanity: severity(find("PROFANITY")),
        alcohol: severity(find("ALCOHOL_DRUGS")),
        frightening: severity(find("FRIGHTENING_INTENSE_SCENES")),
    })
}

/// Fetches and maps a guide. A network or parsing error simply means "no warning available".
pub async fn fetch(client: &AddonClient, imdb_id: &str) -> Option<Guide> {
    let id = extract_imdb_id(imdb_id)?;
    let payload = client.get_text(&format!("{BASE_URL}/titles/{id}/parentsGuide"), false).await.ok()?;
    map_guide(&payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imdb_id_and_severity_rules() {
        assert_eq!(extract_imdb_id("https://imdb.com/title/tt1234567/"), Some("tt1234567".into()));
        assert_eq!(extract_imdb_id("series-123"), None);
        let payload = r#"{"parentsGuide":[
            {"category":"VIOLENCE","severityBreakdowns":[{"severityLevel":"none","voteCount":2},{"severityLevel":"severe","voteCount":5}]},
            {"category":"PROFANITY","severityBreakdowns":[{"severityLevel":"none","voteCount":8},{"severityLevel":"mild","voteCount":3}]},
            {"category":"ALCOHOL_DRUGS","severityBreakdowns":[]}
        ]}"#;
        let guide = map_guide(payload).unwrap();
        assert_eq!(guide.violence.as_deref(), Some("severe"));
        assert_eq!(guide.profanity, None, "none has more votes");
        assert_eq!(guide.alcohol, None);
        assert_eq!(guide.warnings(), vec![("violence", "severe")]);
    }
}
