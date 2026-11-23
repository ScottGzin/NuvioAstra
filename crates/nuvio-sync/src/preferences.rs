//! Player preferences kept by the account (`ProfileSettingsSync`, platform `desktop`).
//!
//! The account keeps one blob with every preference of Nuvio Desktop (theme, cards, player, debrid,
//! TMDB…) and a desktop replaces all of its own with it. So Astra never writes a blob of its own:
//! - a push is the blob just read, with only the fields changed here patched in;
//! - when the account has no blob, nothing is pushed (a partial one would reset those preferences
//!   on the user's other computers);
//! - each field merges three-way with the values both sides last agreed on.
//!
//! The fields are the ones of `PlayerSettingsStorage.exportToSyncPayload()` that Astra has. Its own
//! options (motion, picture, P2P, the extra subtitle options) have no place in the blob and stay on
//! the computer, and so do fonts: only the built-in ids mean the same on every device, another
//! choice leaves the account's as it is.

use nuvio_store::settings::{PlayerSettings, AUTO_SKIP_SEGMENTS, SUBTITLE_FONTS};
use serde_json::{json, Map, Value};

pub(crate) const PLATFORM: &str = "desktop";
const FEATURE: &str = "player_settings";

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Flag,
    Int,
    /// `encodeSyncFloat`; a whole number may come without decimals.
    Float,
    /// `encodeSyncStringSet` of the values Astra knows; the others are kept as the account has them.
    Choices,
    /// A member of `subtitle_appearance` (a JSON text; a missing member is the default).
    Member(&'static str, &'static str),
}

const APPEARANCE: &str = "subtitle_appearance";

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Text | Kind::Member(..) => "string",
            Kind::Flag => "boolean",
            Kind::Int => "int",
            Kind::Float => "float",
            Kind::Choices => "string_set",
        }
    }
}

/// (blob key, path in `PlayerSettings`' JSON, `encodeSync*` type)
const FIELDS: &[(&str, &str, Kind)] = &[
    ("resize_mode", "/resizeMode", Kind::Text),
    ("decoder_priority", "/decoderPriority", Kind::Int),
    ("nvidia_rtx_super_resolution_enabled", "/nvidiaRtxSuperResolution", Kind::Flag),
    ("nvidia_rtx_hdr_enabled", "/nvidiaRtxHdr", Kind::Flag),
    ("preferred_audio_language", "/preferredAudioLanguage", Kind::Text),
    ("secondary_preferred_audio_language", "/secondaryPreferredAudioLanguage", Kind::Text),
    ("preferred_subtitle_language", "/preferredSubtitleLanguage", Kind::Text),
    ("secondary_preferred_subtitle_language", "/secondaryPreferredSubtitleLanguage", Kind::Text),
    ("subtitle_text_color", "/subtitle/textColor", Kind::Text),
    ("subtitle_background_color", "/subtitle/backgroundColor", Kind::Text),
    ("subtitle_outline_color", "/subtitle/outlineColor", Kind::Text),
    ("subtitle_outline_enabled", "/subtitle/outlineEnabled", Kind::Flag),
    ("subtitle_outline_width", "/subtitle/outlineWidth", Kind::Int),
    ("subtitle_bold", "/subtitle/bold", Kind::Flag),
    ("subtitle_font_size_sp", "/subtitle/fontSizeSp", Kind::Int),
    ("subtitle_bottom_offset", "/subtitle/bottomOffset", Kind::Int),
    ("subtitle_strip_sdh", "/subtitle/stripSdh", Kind::Flag),
    ("skip_intro_enabled", "/skipSegmentsEnabled", Kind::Flag),
    ("auto_skip_segment_types", "/autoSkipSegments", Kind::Choices),
    ("next_episode_threshold_mode", "/nextEpisodeThresholdMode", Kind::Text),
    ("next_episode_threshold_percent_v2", "/nextEpisodeThresholdPercent", Kind::Float),
    ("next_episode_threshold_minutes_before_end_v2", "/nextEpisodeThresholdMinutes", Kind::Float),
    (APPEARANCE, "/subtitle/fontId", Kind::Member("fontId", "text")),
    (APPEARANCE, "/subtitle/shadowEnabled", Kind::Member("shadowEnabled", "flag")),
    (APPEARANCE, "/subtitle/shadowOpacity", Kind::Member("shadowOpacity", "int")),
    (APPEARANCE, "/subtitle/shadowSize", Kind::Member("shadowSize", "int")),
];

/// A value that means nothing on another device (a font of this computer).
fn local_only(path: &str, value: &Value) -> bool {
    path == "/subtitle/fontId" && value.as_str().is_some_and(|id| !SUBTITLE_FONTS.contains(&id))
}

fn as_json(settings: &PlayerSettings) -> Value {
    serde_json::to_value(settings).unwrap_or_default()
}

fn field(settings: &Value, path: &str) -> Value {
    settings.pointer(path).cloned().unwrap_or(Value::Null)
}

fn put(settings: &mut Value, path: &str, value: Value) {
    if let Some(slot) = settings.pointer_mut(path) {
        *slot = value;
    }
}

/// A value of the blob (`decodeSync*`): another type or an unreadable value counts as absent.
fn decode(blob: &Value, key: &str, kind: Kind) -> Option<Value> {
    let entry = blob.get("features")?.get(FEATURE)?.get(key)?;
    if entry.get("type")?.as_str()? != kind.name() {
        return None;
    }
    let value = entry.get("value")?;
    let readable = match kind {
        Kind::Text => value.is_string(),
        Kind::Flag => value.is_boolean(),
        Kind::Int => value.is_i64(),
        Kind::Float => return value.as_f64().map(|v| json!(v)),
        Kind::Choices => {
            let mut known: Vec<&str> =
                value.as_array()?.iter().filter_map(Value::as_str).filter(|v| AUTO_SKIP_SEGMENTS.contains(v)).collect();
            known.sort_unstable();
            known.dedup();
            return Some(json!(known));
        }
        Kind::Member(member, shape) => {
            let object: Value = serde_json::from_str(value.as_str()?).ok()?;
            let default = match member {
                "fontId" => json!("default"),
                "shadowEnabled" => json!(false),
                "shadowOpacity" => json!(75),
                _ => json!(2),
            };
            let found = object.as_object()?.get(member).cloned().unwrap_or_else(|| default.clone());
            let ok = match shape {
                "text" => found.is_string() && !local_only("/subtitle/fontId", &found),
                "flag" => found.is_boolean(),
                _ => found.is_i64(),
            };
            return Some(if ok { found } else { default });
        }
    };
    readable.then(|| value.clone())
}

fn read_over(start: &PlayerSettings, blob: &Value) -> PlayerSettings {
    let mut out = as_json(start);
    for &(key, path, kind) in FIELDS {
        if let Some(value) = decode(blob, key, kind) {
            put(&mut out, path, value);
        }
    }
    serde_json::from_value::<PlayerSettings>(out).map_or_else(|_| start.clone(), PlayerSettings::sanitized)
}

/// What the account says: an absent field is the original's default.
pub(crate) fn from_blob(blob: &Value) -> PlayerSettings {
    read_over(&PlayerSettings::default(), blob)
}

/// `local` with the account's value in every field not changed here since `base`. Without a base
/// the fields the account has win and the others keep the local value.
pub(crate) fn merge(local: &PlayerSettings, base: Option<&PlayerSettings>, blob: &Value) -> PlayerSettings {
    let Some(base) = base else { return read_over(local, blob) };
    let (here, agreed, theirs) = (as_json(local), as_json(base), as_json(&from_blob(blob)));
    let mut out = here.clone();
    for &(_, path, _) in FIELDS {
        if field(&here, path) == field(&agreed, path) {
            put(&mut out, path, field(&theirs, path));
        }
    }
    serde_json::from_value::<PlayerSettings>(out).map_or_else(|_| local.clone(), PlayerSettings::sanitized)
}

pub(crate) fn differs(local: &PlayerSettings, base: &PlayerSettings) -> bool {
    let (here, agreed) = (as_json(local), as_json(base));
    FIELDS.iter().any(|(_, path, _)| {
        let value = field(&here, path);
        !local_only(path, &value) && value != field(&agreed, path)
    })
}

/// The account's blob with only the fields where `local` differs from it; everything else (other
/// features, unknown keys, values Astra cannot represent) is left exactly as received. `None` when
/// the blob has a shape that cannot be patched safely.
pub(crate) fn payload(blob: &Value, local: &PlayerSettings) -> Option<Value> {
    let (here, theirs) = (as_json(local), as_json(&from_blob(blob)));
    let mut out = blob.clone();
    let features = out.as_object_mut()?.entry("features").or_insert_with(|| Value::Object(Map::new())).as_object_mut()?;
    let player = features.entry(FEATURE).or_insert_with(|| Value::Object(Map::new())).as_object_mut()?;
    for &(key, path, kind) in FIELDS {
        let value = field(&here, path);
        if value == field(&theirs, path) || local_only(path, &value) {
            continue;
        }
        if let Kind::Member(member, _) = kind {
            let current = player.get(key).and_then(|e| e.get("value")).and_then(Value::as_str);
            let mut object: Map<String, Value> = current.and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default();
            object.insert(member.into(), value);
            player.insert(key.into(), json!({ "type": kind.name(), "value": Value::Object(object).to_string() }));
            continue;
        }
        if let Kind::Choices = kind {
            let current = player.get(key).and_then(|e| e.get("value")).and_then(Value::as_array);
            let unknown = current.into_iter().flatten().filter_map(Value::as_str).filter(|v| !AUTO_SKIP_SEGMENTS.contains(v));
            let mut all: Vec<&str> = unknown.chain(value.as_array().into_iter().flatten().filter_map(Value::as_str)).collect();
            all.sort_unstable();
            let written = json!({ "type": kind.name(), "value": all });
            player.insert(key.into(), written);
            continue;
        }
        if value.is_null() {
            player.remove(key); // cleared, as the original stores it
        } else {
            player.insert(key.into(), json!({ "type": kind.name(), "value": value }));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_exists_in_player_settings() {
        let settings = as_json(&PlayerSettings::default());
        for (key, path, _) in FIELDS {
            assert!(settings.pointer(path).is_some(), "{key}");
        }
    }

    #[test]
    fn only_changed_fields_are_written() {
        let blob = json!({ "version": 4, "features": {
            "theme_settings": { "amoled": { "type": "boolean", "value": true } },
            "player_settings": {
                "skip_intro_enabled": { "type": "boolean", "value": true },
                "resize_mode": { "type": "string", "value": "SomethingNewer" },
                "subtitle_font_size_sp": { "type": "int", "value": 22 }
            }
        }});
        let base = from_blob(&blob);
        assert_eq!((base.resize_mode.as_str(), base.subtitle.font_size_sp), ("Fit", 22));

        let mut local = base.clone();
        local.subtitle.bold = !local.subtitle.bold;
        local.motion_smoothing = "fluid".into();
        let sent = payload(&blob, &local).unwrap();
        let player = &sent["features"]["player_settings"];
        assert_eq!(player["subtitle_bold"], json!({ "type": "boolean", "value": local.subtitle.bold }));
        assert_eq!(player["resize_mode"]["value"], "SomethingNewer", "unknown value not overwritten");
        assert_eq!(player["skip_intro_enabled"]["value"], true);
        assert_eq!(player.as_object().unwrap().len(), 4);
        assert_eq!(sent["features"]["theme_settings"], blob["features"]["theme_settings"]);
        assert!(!sent.to_string().contains("fluid"));
    }

    #[test]
    fn subtitle_appearance_members_and_fonts_of_this_computer() {
        let custom = "custom-01234567-89ab-cdef-0123-456789abcdef";
        let blob = json!({ "features": { "player_settings": { "subtitle_appearance":
            { "type": "string", "value": format!(r#"{{"backgroundRadius":6,"fontId":"{custom}","shadowEnabled":true}}"#) } } } });
        let base = from_blob(&blob);
        assert_eq!((base.subtitle.font_id.as_str(), base.subtitle.shadow_enabled, base.subtitle.shadow_size), ("default", true, 2));

        let mut local = base.clone();
        local.subtitle.font_id = "system:Arial".into();
        assert!(!differs(&local, &base), "a font of this computer is not a change to send");
        local.subtitle.shadow_size = 5;
        let sent = payload(&blob, &local).unwrap();
        let text = sent["features"]["player_settings"]["subtitle_appearance"]["value"].as_str().unwrap();
        let object: Value = serde_json::from_str(text).unwrap();
        assert_eq!(object, json!({ "backgroundRadius": 6, "fontId": custom, "shadowEnabled": true, "shadowSize": 5 }));

        // After that push the local font survives the next sync.
        let agreed = from_blob(&sent);
        let merged = merge(&local, Some(&agreed), &sent);
        assert_eq!(merged.subtitle.font_id, "system:Arial");
    }

    #[test]
    fn skip_and_next_episode_settings_use_the_original_types() {
        let blob = json!({ "features": { "player_settings": {
            "auto_skip_segment_types": { "type": "string_set", "value": ["something-newer", "outro"] },
            "next_episode_threshold_percent_v2": { "type": "float", "value": 98 }
        }}});
        let base = from_blob(&blob);
        assert_eq!((base.auto_skip_segments.clone(), base.next_episode_threshold_percent), (vec!["outro".to_string()], 98.0));
        assert!(!differs(&base, &from_blob(&blob)));
        let mut local = base.clone();
        local.auto_skip_segments = vec!["intro".into(), "outro".into()];
        local.next_episode_threshold_minutes = 1.5;
        local.watched_percent = 97;
        let sent = payload(&blob, &local).unwrap();
        let player = &sent["features"]["player_settings"];
        assert_eq!(player["auto_skip_segment_types"], json!({ "type": "string_set", "value": ["intro", "outro", "something-newer"] }));
        assert_eq!(player["next_episode_threshold_percent_v2"]["value"], json!(98), "untouched");
        assert_eq!(player["next_episode_threshold_minutes_before_end_v2"], json!({ "type": "float", "value": 1.5 }));
        assert_eq!(player.as_object().unwrap().len(), 3, "the watched percentage stays on this computer");
    }

    #[test]
    fn three_way_by_field() {
        let base = PlayerSettings::default();
        let mut local = base.clone();
        local.subtitle.bold = !base.subtitle.bold;
        let blob = json!({ "features": { "player_settings": { "preferred_audio_language": { "type": "string", "value": "ja" } } } });
        let merged = merge(&local, Some(&base), &blob);
        assert_eq!(merged.subtitle.bold, local.subtitle.bold, "changed here");
        assert_eq!(merged.preferred_audio_language, "ja", "changed on the account");
    }
}
