//! Per-profile settings of NextGen, with the original's keys, defaults and ranges.
//!
//! Only settings that NextGen applies for real are here (see docs/FEATURE_PARITY.md):
//! - player (`PlayerSettingsRepository` / `PlayerSettingsStorage.desktop.kt`, store
//!   `nuvio_player_settings`, keys `<name>_<profile>`): resize mode, decoder priority, NVIDIA RTX
//!   VSR/HDR, audio/subtitle language preferences, subtitle style;
//! - home (`HomeCatalogSettingsRepository`): hero, catalog type in titles, unreleased content,
//!   catalog visibility/order/custom titles.
//! In the original all of them are profile-scoped (`ProfileScopedKey`); so are they here.

use crate::{HomeCatalogSettings, LegacyStore, StoreError};
use serde::{Deserialize, Serialize};

pub const RESIZE_MODES: &[&str] = &["Fit", "Fill", "Zoom", "Stretch"];
pub const FONT_SIZE_RANGE: (i64, i64) = (6, 40); // desktop `subtitleFontSizeRangeSp`
pub const OUTLINE_WIDTH_RANGE: (i64, i64) = (1, 5);
pub const BOTTOM_OFFSET_RANGE: (i64, i64) = (0, 200);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleSettings {
    /// `#AARRGGBB`, as `Color.toStorageHexString`.
    pub text_color: String,
    pub background_color: String,
    pub outline_color: String,
    pub outline_enabled: bool,
    pub outline_width: i64,
    pub bold: bool,
    pub font_size_sp: i64,
    pub bottom_offset: i64,
    pub strip_sdh: bool,
    /// `SubtitleAppearance`: `default`, `sans`, `serif`, `mono`, `custom-<uuid>` (imported here) or,
    /// Astra only, `system:<family>`.
    pub font_id: String,
    pub shadow_enabled: bool,
    pub shadow_opacity: i64,
    pub shadow_size: i64,
    /// Astra only: the next five.
    pub italic: bool,
    pub spacing: i64,
    pub blur: i64,
    pub align: String,
    pub override_ass: bool,
}

/// Astra only: the player's picture options (`nuvio_player::Picture`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PictureSettings {
    pub mode: String,
    pub denoise: i64,
    pub deblur: i64,
    pub thin: i64,
    pub darken: i64,
    pub deband: i64,
    pub brightness: i64,
    pub contrast: i64,
    pub saturation: i64,
    pub gamma: i64,
    pub hue: i64,
}

impl Default for PictureSettings {
    fn default() -> Self {
        Self { mode: "auto".into(), denoise: 0, deblur: 0, thin: 0, darken: 0, deband: 0, brightness: 0, contrast: 0, saturation: 0, gamma: 0, hue: 0 }
    }
}

impl PictureSettings {
    pub fn sanitized(mut self) -> Self {
        if !ANIME_MODES.contains(&self.mode.as_str()) {
            self.mode = Self::default().mode;
        }
        for effect in [&mut self.denoise, &mut self.deblur, &mut self.thin, &mut self.darken, &mut self.deband] {
            *effect = (*effect).clamp(0, 3);
        }
        for color in [&mut self.brightness, &mut self.contrast, &mut self.saturation, &mut self.gamma, &mut self.hue] {
            *color = (*color).clamp(-50, 50);
        }
        self
    }
}

pub const SUBTITLE_FONTS: &[&str] = &["default", "sans", "serif", "mono"];
pub const ANIME_MODES: &[&str] = &["auto", "a", "b", "c", "aa", "bb", "ca"];

fn valid_font(id: &str) -> bool {
    SUBTITLE_FONTS.contains(&id)
        || id.strip_prefix("custom-").is_some_and(|u| u.len() == 36 && u.chars().all(|c| matches!(c, 'a'..='f' | '0'..='9' | '-')))
        || id.strip_prefix("system:").is_some_and(|f| !f.trim().is_empty() && f.len() <= 100 && !f.chars().any(char::is_control))
}

impl Default for SubtitleSettings {
    /// `SubtitleStyleState.DEFAULT`.
    fn default() -> Self {
        Self {
            text_color: "#FFFFFFFF".into(),
            background_color: "#00000000".into(),
            outline_color: "#FF000000".into(),
            outline_enabled: true,
            outline_width: 2,
            bold: false,
            font_size_sp: 18,
            bottom_offset: 20,
            strip_sdh: false,
            font_id: "default".into(),
            shadow_enabled: false,
            shadow_opacity: 75,
            shadow_size: 2,
            italic: false,
            spacing: 0,
            blur: 0,
            align: "center".into(),
            override_ass: false,
        }
    }
}

/// `AutoSkipSegmentType` stored values.
pub const AUTO_SKIP_SEGMENTS: &[&str] = &["intro", "movie-credits", "outro", "recap"];
/// `NextEpisodeThresholdMode` names.
pub const NEXT_EPISODE_MODES: &[&str] = &["PERCENTAGE", "MINUTES_BEFORE_END"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayerSettings {
    /// `PlayerResizeMode` name: Fit, Fill, Zoom, Stretch.
    pub resize_mode: String,
    /// 0 hardware only, 1 hardware preferred (default), 2 software only.
    pub decoder_priority: i64,
    pub nvidia_rtx_super_resolution: bool,
    pub nvidia_rtx_hdr: bool,
    /// Astra only: `off`, `steady` or `fluid`.
    pub motion_smoothing: String,
    /// Astra only: `off`, `movies`, `animeFast` or `animeQuality`.
    pub video_enhancement: String,
    /// `default`, `device`, `original` or a language code (`AudioLanguageOption`).
    pub preferred_audio_language: String,
    pub secondary_preferred_audio_language: Option<String>,
    /// `none`, `device`, `forced` or a language code (`SubtitleLanguageOption`).
    pub preferred_subtitle_language: String,
    pub secondary_preferred_subtitle_language: Option<String>,
    pub subtitle: SubtitleSettings,
    pub picture: PictureSettings,
    /// `skip_intro_enabled`: the skip buttons (and the services that give their times).
    pub skip_segments_enabled: bool,
    /// `auto_skip_segment_types`: `intro`, `recap`, `outro`, `movie-credits`.
    pub auto_skip_segments: Vec<String>,
    /// `PERCENTAGE` or `MINUTES_BEFORE_END`: when the next-episode card shows.
    pub next_episode_threshold_mode: String,
    pub next_episode_threshold_percent: f64,
    pub next_episode_threshold_minutes: f64,
    /// Astra only: 90–100, from where an episode or movie counts as watched (the original's is 90).
    pub watched_percent: i64,
}

impl Default for PlayerSettings {
    /// `PlayerSettingsUiState()` defaults.
    fn default() -> Self {
        Self {
            resize_mode: "Fit".into(),
            decoder_priority: 1,
            nvidia_rtx_super_resolution: false,
            nvidia_rtx_hdr: false,
            motion_smoothing: "off".into(),
            video_enhancement: "off".into(),
            preferred_audio_language: "device".into(),
            secondary_preferred_audio_language: None,
            preferred_subtitle_language: "none".into(),
            secondary_preferred_subtitle_language: None,
            subtitle: SubtitleSettings::default(),
            picture: PictureSettings::default(),
            skip_segments_enabled: true,
            auto_skip_segments: Vec::new(),
            next_episode_threshold_mode: "PERCENTAGE".into(),
            next_episode_threshold_percent: 99.0,
            next_episode_threshold_minutes: 2.0,
            watched_percent: crate::library::COMPLETION_PERCENT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileSettings {
    pub player: PlayerSettings,
    pub home: HomeCatalogSettings,
    pub p2p: P2pSettings,
}

pub const MOTION_SMOOTHING: &[&str] = &["off", "steady", "fluid"];
pub const VIDEO_ENHANCEMENT: &[&str] = &["off", "movies", "animeFast", "animeQuality"];
pub const P2P_PROFILES: &[&str] = &["SOFT", "BALANCED", "FAST"];
/// `P2pCacheSize` names and sizes.
pub const P2P_CACHE_SIZES: &[(&str, u64)] = &[
    ("NONE", 0),
    ("GB_2", 2 << 30),
    ("GB_5", 5 << 30),
    ("GB_10", 10 << 30),
];

/// P2P streaming (`P2pSettingsRepository`, profile-scoped in the original). Off until the user
/// accepts the P2P notice (`P2pConsentDialog`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct P2pSettings {
    pub enabled: bool,
    pub enable_upload: bool,
    pub hide_torrent_stats: bool,
    /// `P2pTorrentProfile` name.
    pub torrent_profile: String,
    /// `P2pCacheSize` name.
    pub cache_size: String,
}

impl Default for P2pSettings {
    fn default() -> Self {
        Self { enabled: false, enable_upload: true, hide_torrent_stats: false, torrent_profile: "BALANCED".into(), cache_size: "GB_2".into() }
    }
}

impl P2pSettings {
    pub fn sanitized(mut self) -> Self {
        let d = Self::default();
        if !P2P_PROFILES.contains(&self.torrent_profile.as_str()) {
            self.torrent_profile = d.torrent_profile;
        }
        if !P2P_CACHE_SIZES.iter().any(|(n, _)| *n == self.cache_size) {
            self.cache_size = d.cache_size;
        }
        self
    }

    pub fn cache_bytes(&self) -> u64 {
        P2P_CACHE_SIZES.iter().find(|(n, _)| *n == self.cache_size).map_or(2 << 30, |(_, b)| *b)
    }

    /// Native `NuvioTorrentProfile` value (Soft 0, Balanced 1, Fast 2).
    pub fn profile_value(&self) -> i32 {
        P2P_PROFILES.iter().position(|p| *p == self.torrent_profile).map_or(1, |i| i as i32)
    }
}

fn normalize_color(value: &str, fallback: &str) -> String {
    // `subtitleColorFromStorage`: 6 or 8 hex digits, optional '#'; 6 digits = opaque.
    let hex = value.trim().trim_start_matches('#');
    let ok = (hex.len() == 6 || hex.len() == 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    match (ok, hex.len()) {
        (true, 6) => format!("#FF{}", hex.to_uppercase()),
        (true, _) => format!("#{}", hex.to_uppercase()),
        _ => fallback.to_string(),
    }
}

fn optional_language(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl PlayerSettings {
    /// Brings every value into the original's ranges/sets (invalid values become the defaults),
    /// so a malformed write from the UI can never reach the player.
    pub fn sanitized(mut self) -> Self {
        let d = Self::default();
        if !RESIZE_MODES.contains(&self.resize_mode.as_str()) {
            self.resize_mode = d.resize_mode;
        }
        if !MOTION_SMOOTHING.contains(&self.motion_smoothing.as_str()) {
            self.motion_smoothing = d.motion_smoothing;
        }
        if !VIDEO_ENHANCEMENT.contains(&self.video_enhancement.as_str()) {
            self.video_enhancement = d.video_enhancement;
        }
        if !(0..=2).contains(&self.decoder_priority) {
            self.decoder_priority = d.decoder_priority;
        }
        if self.preferred_audio_language.trim().is_empty() {
            self.preferred_audio_language = d.preferred_audio_language;
        }
        if self.preferred_subtitle_language.trim().is_empty() {
            self.preferred_subtitle_language = d.preferred_subtitle_language;
        }
        self.preferred_audio_language = self.preferred_audio_language.trim().to_string();
        self.preferred_subtitle_language = self.preferred_subtitle_language.trim().to_string();
        self.secondary_preferred_audio_language = optional_language(self.secondary_preferred_audio_language);
        self.secondary_preferred_subtitle_language = optional_language(self.secondary_preferred_subtitle_language);
        let s = &mut self.subtitle;
        let ds = SubtitleSettings::default();
        s.text_color = normalize_color(&s.text_color, &ds.text_color);
        s.background_color = normalize_color(&s.background_color, &ds.background_color);
        s.outline_color = normalize_color(&s.outline_color, &ds.outline_color);
        s.outline_width = s.outline_width.clamp(OUTLINE_WIDTH_RANGE.0, OUTLINE_WIDTH_RANGE.1);
        s.font_size_sp = s.font_size_sp.clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1);
        s.bottom_offset = s.bottom_offset.clamp(BOTTOM_OFFSET_RANGE.0, BOTTOM_OFFSET_RANGE.1);
        if !valid_font(&s.font_id) {
            s.font_id = ds.font_id.clone();
        }
        // `SubtitleAppearance.normalized()`
        s.shadow_opacity = s.shadow_opacity.clamp(0, 100);
        s.shadow_size = s.shadow_size.clamp(1, 8);
        s.spacing = s.spacing.clamp(0, 10);
        s.blur = s.blur.clamp(0, 10);
        if !matches!(s.align.as_str(), "left" | "center" | "right") {
            s.align = ds.align.clone();
        }
        self.picture = self.picture.sanitized();
        self.auto_skip_segments.retain(|kind| AUTO_SKIP_SEGMENTS.contains(&kind.as_str()));
        self.auto_skip_segments.sort();
        self.auto_skip_segments.dedup();
        if !NEXT_EPISODE_MODES.contains(&self.next_episode_threshold_mode.as_str()) {
            self.next_episode_threshold_mode = d.next_episode_threshold_mode;
        }
        // Steps of 0.5, as the original's sliders.
        let half = |v: f64, min: f64, max: f64| ((v * 2.0).round() / 2.0).clamp(min, max);
        self.next_episode_threshold_percent = half(self.next_episode_threshold_percent, 97.0, 100.0);
        self.next_episode_threshold_minutes = half(self.next_episode_threshold_minutes, 0.0, 3.5);
        self.watched_percent = self.watched_percent.clamp(90, 100);
        self
    }
}

impl ProfileSettings {
    pub fn sanitized(mut self) -> Self {
        self.player = self.player.sanitized();
        self.p2p = self.p2p.sanitized();
        let mut seen = std::collections::HashSet::new();
        self.home.items.retain(|i| !i.key.trim().is_empty() && seen.insert(i.key.clone()));
        self
    }
}

impl LegacyStore {
    /// Player settings of one profile as the original stores them (missing keys = defaults).
    pub fn player_settings(&self, profile_index: i64) -> Result<PlayerSettings, StoreError> {
        let store = self.read_store("nuvio_player_settings")?;
        let get = |name: &str| store.get(&format!("{name}_{profile_index}")).map(|v| v.trim().to_string());
        let boolean = |name: &str| get(name).and_then(|v| v.parse::<bool>().ok());
        let int = |name: &str| get(name).and_then(|v| v.parse::<i64>().ok());
        let number = |name: &str| get(name).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
        let d = PlayerSettings::default();
        let ds = SubtitleSettings::default();
        let appearance: serde_json::Value = get("subtitle_appearance").and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        Ok(PlayerSettings {
            resize_mode: get("resize_mode").unwrap_or(d.resize_mode),
            decoder_priority: int("decoder_priority").unwrap_or(d.decoder_priority),
            nvidia_rtx_super_resolution: boolean("nvidia_rtx_super_resolution_enabled").unwrap_or(false),
            nvidia_rtx_hdr: boolean("nvidia_rtx_hdr_enabled").unwrap_or(false),
            // The original has no such settings.
            motion_smoothing: d.motion_smoothing,
            video_enhancement: d.video_enhancement,
            preferred_audio_language: get("preferred_audio_language").unwrap_or(d.preferred_audio_language),
            secondary_preferred_audio_language: get("secondary_preferred_audio_language"),
            preferred_subtitle_language: get("preferred_subtitle_language").unwrap_or(d.preferred_subtitle_language),
            secondary_preferred_subtitle_language: get("secondary_preferred_subtitle_language"),
            subtitle: SubtitleSettings {
                text_color: get("subtitle_text_color").unwrap_or(ds.text_color),
                background_color: get("subtitle_background_color").unwrap_or(ds.background_color),
                outline_color: get("subtitle_outline_color").unwrap_or(ds.outline_color),
                outline_enabled: boolean("subtitle_outline_enabled").unwrap_or(ds.outline_enabled),
                outline_width: int("subtitle_outline_width").unwrap_or(ds.outline_width),
                bold: boolean("subtitle_bold").unwrap_or(ds.bold),
                font_size_sp: int("subtitle_font_size_sp").unwrap_or(ds.font_size_sp),
                bottom_offset: int("subtitle_bottom_offset").unwrap_or(ds.bottom_offset),
                strip_sdh: boolean("subtitle_strip_sdh").unwrap_or(ds.strip_sdh),
                // An imported font of the original is not here: the default is used.
                font_id: appearance["fontId"].as_str().filter(|f| SUBTITLE_FONTS.contains(f)).map_or(ds.font_id, str::to_string),
                shadow_enabled: appearance["shadowEnabled"].as_bool().unwrap_or(ds.shadow_enabled),
                shadow_opacity: appearance["shadowOpacity"].as_i64().unwrap_or(ds.shadow_opacity),
                shadow_size: appearance["shadowSize"].as_i64().unwrap_or(ds.shadow_size),
                italic: ds.italic,
                spacing: ds.spacing,
                blur: ds.blur,
                align: ds.align,
                override_ass: ds.override_ass,
            },
            picture: PictureSettings::default(),
            skip_segments_enabled: boolean("skip_intro_enabled").unwrap_or(d.skip_segments_enabled),
            auto_skip_segments: AUTO_SKIP_SEGMENTS
                .iter()
                .filter(|kind| get("auto_skip_segment_types").is_some_and(|saved| saved.split(|c: char| !c.is_ascii_lowercase() && c != '-').any(|v| v == **kind)))
                .map(|kind| kind.to_string())
                .collect(),
            next_episode_threshold_mode: get("next_episode_threshold_mode").unwrap_or(d.next_episode_threshold_mode),
            next_episode_threshold_percent: number("next_episode_threshold_percent_v2").unwrap_or(d.next_episode_threshold_percent),
            next_episode_threshold_minutes: number("next_episode_threshold_minutes_before_end_v2").unwrap_or(d.next_episode_threshold_minutes),
            watched_percent: d.watched_percent,
        }
        .sanitized())
    }

    /// Everything NextGen ports for one profile, read from the original's stores.
    pub fn profile_settings(&self, profile_index: i64) -> Result<ProfileSettings, StoreError> {
        Ok(ProfileSettings {
            player: self.player_settings(profile_index)?,
            home: self.home_catalog_settings(profile_index)?.unwrap_or_default(),
            // P2P starts off in Astra: the notice is accepted again here.
            p2p: P2pSettings::default(),
        }
        .sanitized())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_original_ranges() {
        let mut p = PlayerSettings { resize_mode: "Huge".into(), decoder_priority: 7, ..Default::default() };
        p.subtitle.font_size_sp = 99;
        p.subtitle.outline_width = 0;
        p.subtitle.bottom_offset = -3;
        p.subtitle.text_color = "00ff00".into();
        p.subtitle.outline_color = "nope".into();
        p.secondary_preferred_audio_language = Some("  ".into());
        let s = p.sanitized();
        assert_eq!(s.resize_mode, "Fit");
        assert_eq!(s.decoder_priority, 1);
        assert_eq!((s.subtitle.font_size_sp, s.subtitle.outline_width, s.subtitle.bottom_offset), (40, 1, 0));
        assert_eq!(s.subtitle.text_color, "#FF00FF00");
        assert_eq!(s.subtitle.outline_color, "#FF000000");
        assert_eq!(s.secondary_preferred_audio_language, None);
    }

    #[test]
    fn serializes_in_camel_case_for_the_ui() {
        let json = serde_json::to_value(ProfileSettings::default()).unwrap();
        assert_eq!(json["player"]["resizeMode"], "Fit");
        assert_eq!(json["player"]["subtitle"]["fontSizeSp"], 18);
        assert_eq!(json["home"]["heroEnabled"], true);
    }
}
