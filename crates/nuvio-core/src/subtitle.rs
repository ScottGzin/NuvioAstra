//! Addon subtitles (`/subtitles/{type}/{id}[/extras].json`) and the ones a stream carries.
//!
//! Port of `SubtitleRepository.kt`: the language is the first of `lang`, `language`,
//! `languageCode`, `locale`, `label` (else `unknown`); entries without an HTTP(S) URL are dropped.

use crate::manifest::optional_string;
use crate::url::encode_path_segment;
use crate::CoreError;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct SubtitleItem {
    pub id: String,
    pub url: String,
    pub language: String,
}

/// The items of a `subtitles` array.
pub(crate) fn items(list: Option<&Value>, source: &str) -> Vec<SubtitleItem> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, element)| {
            let entry = element.as_object()?;
            let text = |name: &str| optional_string(entry, name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
            let url = text("url").filter(|u| {
                let lower = u.to_ascii_lowercase();
                lower.starts_with("https://") || lower.starts_with("http://")
            })?;
            let language = ["lang", "language", "languageCode", "locale", "label"].iter().find_map(|name| text(name));
            Some(SubtitleItem {
                id: text("id").unwrap_or_else(|| format!("{source}_{index}")),
                language: language.unwrap_or_else(|| "unknown".into()),
                url,
            })
        })
        .collect()
}

pub fn parse_subtitles(payload: &str, source: &str) -> Result<Vec<SubtitleItem>, CoreError> {
    let root: Value = serde_json::from_str(payload)?;
    Ok(items(root.get("subtitles"), source))
}

/// The path extra that tells the addon which file is playing (`videoHash`, `videoSize`, `filename`).
pub fn subtitle_extra(hash: Option<&str>, size: Option<i64>, filename: Option<&str>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(hash) = hash.map(str::trim).filter(|v| !v.is_empty()) {
        parts.push(format!("videoHash={}", encode_path_segment(hash)));
    }
    if let Some(size) = size.filter(|&v| v > 0) {
        parts.push(format!("videoSize={size}"));
    }
    if let Some(name) = filename.map(str::trim).filter(|v| !v.is_empty()) {
        parts.push(format!("filename={}", encode_path_segment(name)));
    }
    (!parts.is_empty()).then(|| parts.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_subtitle_repository() {
        let payload = r#"{"subtitles":[
            {"id":"1","url":"https://s.test/a.srt","lang":"por"},
            {"url":"https://s.test/b.vtt","label":"English"},
            {"url":"C:/private/file.srt","lang":"eng"},
            {"url":"https://s.test/c.srt"}
        ]}"#;
        let found = parse_subtitles(payload, "addon").unwrap();
        let short: Vec<_> = found.iter().map(|s| (s.id.as_str(), s.language.as_str())).collect();
        assert_eq!(short, vec![("1", "por"), ("addon_1", "English"), ("addon_3", "unknown")]);
        assert_eq!(subtitle_extra(Some("ab"), Some(5), Some("My File.mkv")).as_deref(), Some("videoHash=ab&videoSize=5&filename=My%20File.mkv"));
        assert_eq!(subtitle_extra(None, Some(0), Some(" ")), None);
    }
}
