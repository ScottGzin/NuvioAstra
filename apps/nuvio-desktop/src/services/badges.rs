//! Stream badge rules imported in the original app (`StreamBadgeRules` / `FR-Badges` sets),
//! read only from its settings store. The UI matches them against each stream's name/description.
//!
//! The original keeps up to three imported rule sets per profile and uses the active one; when the
//! stream badge store has none it falls back to the legacy debrid rules. A missing store simply
//! means no badges (the cards still show the addon's own text).

use nuvio_store::{default_legacy_data_dir, LegacyStore};
use serde::{Deserialize, Serialize};

/// One badge filter (`StreamBadgeFilter`): a regex and the chip it produces.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub name: String,
    pub pattern: String,
    pub image_url: String,
    pub tag_color: String,
    pub tag_style: String,
    pub text_color: String,
    pub border_color: String,
    pub group_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    pub filters: Vec<Filter>,
    /// `showFileSizeBadges` (default true): add the "SIZE 4.4 GB" chip.
    pub show_file_size_badges: bool,
    /// `BOTTOM` or `TOP` (`stream_badge_placement`).
    pub placement: String,
}

impl Rules {
    fn empty() -> Self {
        Self { filters: Vec::new(), show_file_size_badges: true, placement: "BOTTOM".into() }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct StoredRules {
    imports: Vec<StoredImport>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct StoredImport {
    filters: Vec<StoredFilter>,
    is_active: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct StoredFilter {
    name: String,
    pattern: String,
    #[serde(rename = "imageURL", alias = "imageUrl")]
    image_url: String,
    is_enabled: Option<bool>,
    tag_color: String,
    tag_style: String,
    text_color: String,
    border_color: String,
    group_id: String,
}

/// Badge rules of the profile (the active import; disabled and unusable filters dropped).
pub fn rules(profile: i64) -> Rules {
    let Some(store) = default_legacy_data_dir().ok().and_then(|dir| LegacyStore::open(dir).ok()) else { return Rules::empty() };
    let settings = store.read_store("nuvio_stream_badge_settings").unwrap_or_default();
    let payload: Option<String> = settings.get(&format!("stream_badge_rules_{profile}")).cloned().or_else(|| {
        // `StreamBadgeSettingsRepository`: the legacy debrid import.
        store
            .read_store("nuvio_debrid_settings")
            .ok()
            .and_then(|debrid| debrid.get(&format!("debrid_stream_badge_rules_{profile}")).cloned())
    });
    let show_file_size_badges = settings
        .get(&format!("show_file_size_badges_{profile}"))
        .is_none_or(|v| !v.trim().eq_ignore_ascii_case("false"));
    let placement = settings
        .get(&format!("stream_badge_placement_{profile}"))
        .map(|v| v.trim().to_uppercase())
        .filter(|v| v == "TOP" || v == "BOTTOM")
        .unwrap_or_else(|| "BOTTOM".into());
    let filters = payload.as_deref().map(parse_filters).unwrap_or_default();
    Rules { filters, show_file_size_badges, placement }
}

/// Parses the stored payload and keeps the active import's usable filters.
fn parse_filters(payload: &str) -> Vec<Filter> {
    let Ok(stored) = serde_json::from_str::<StoredRules>(payload) else { return Vec::new() };
    let Some(import) = stored.imports.iter().find(|i| i.is_active).or_else(|| stored.imports.first()) else { return Vec::new() };
    import
        .filters
        .iter()
        .filter(|f| f.is_enabled.unwrap_or(true))
        .filter(|f| !f.name.trim().is_empty() && !f.pattern.trim().is_empty())
        .map(|f| Filter {
            name: f.name.trim().to_string(),
            pattern: f.pattern.trim().to_string(),
            image_url: f.image_url.trim().to_string(),
            tag_color: f.tag_color.trim().to_string(),
            tag_style: f.tag_style.trim().to_lowercase(),
            text_color: f.text_color.trim().to_string(),
            border_color: f.border_color.trim().to_string(),
            group_id: f.group_id.trim().to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = r##"{"imports":[
        {"sourceUrl":"https://example/first.json","isActive":false,"filters":[{"name":"Ignored","pattern":"x"}]},
        {"sourceUrl":"https://example/neon.json","isActive":true,"filters":[
            {"id":"1","name":"1080p","pattern":"\\b1080p\\b","imageURL":"https://img/1080.png","tagColor":"#123456","tagStyle":"filled","textColor":"#fff","borderColor":"#000","groupId":"res"},
            {"id":"2","name":"Disabled","pattern":"720p","isEnabled":false},
            {"id":"3","name":"","pattern":"blank name"},
            {"id":"4","name":"No pattern","pattern":"  "},
            {"id":"5","name":"AVC","pattern":"\\bAVC\\b","tagStyle":"OUTLINE"}]}]}"##;

    #[test]
    fn active_import_only_and_usable_filters() {
        let filters = parse_filters(PAYLOAD);
        assert_eq!(filters.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["1080p", "AVC"]);
        assert_eq!(filters[0].image_url, "https://img/1080.png");
        assert_eq!(filters[0].tag_style, "filled");
        assert_eq!(filters[1].tag_style, "outline");
        assert_eq!(filters[0].group_id, "res");
    }

    #[test]
    fn bad_payloads_give_no_badges() {
        assert!(parse_filters("not json").is_empty());
        assert!(parse_filters(r#"{"imports":[]}"#).is_empty());
    }
}
