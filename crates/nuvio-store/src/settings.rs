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

    // ---- Sources / automatic playback (PlayerSettingsUiState of the original) ----
    /// `streamReuseLastLinkEnabled`: replay the cached stream link (default false).
    pub reuse_last_link_enabled: bool,
    /// `streamReuseLastLinkCacheHours`, one of 1,2,3,6,12,24,48,72,168.
    pub reuse_last_link_cache_hours: i64,
    /// `streamAutoPlayFallbackOnFailure`: try the next candidate if a source fails to start.
    pub auto_play_fallback_on_failure: bool,
    pub auto_play_fallback_max_sources: i64,
    pub auto_play_fallback_startup_timeout_seconds: i64,
    /// `streamAutoPlayMode`: `manual`, `firstStream` or `regexMatch`.
    pub auto_play_mode: String,
    /// `streamAutoPlaySource`: `allSources`, `installedAddonsOnly` or `enabledPluginsOnly`.
    pub auto_play_source: String,
    pub auto_play_selected_addons: Vec<String>,
    pub auto_play_selected_plugins: Vec<String>,
    pub auto_play_regex: String,
    /// `streamAutoPlayTimeoutSeconds`, snapped to `STREAM_AUTO_PLAY_TIMEOUT_VALUES`.
    pub auto_play_timeout_seconds: i64,

    // ---- Next episode, chapters, hold-to-speed, content warnings ----
    pub auto_play_next_episode_enabled: bool,
    pub auto_play_next_episode_fallback_enabled: bool,
    pub auto_play_prefer_binge_group: bool,
    pub auto_play_reuse_binge_group: bool,
    pub preload_next_episode_sources: bool,
    pub chapter_title_enabled: bool,
    pub hold_to_speed_enabled: bool,
    /// Clamped to 1.0–4.0 (the original's `holdToSpeedValue`, default 2).
    pub hold_to_speed_value: f64,
    pub show_parental_guide: bool,

    // ---- Audio (SubtitleAudioModels / PlayerSettingsRepository) ----
    pub remember_audio_delay_per_device: bool,
    pub persist_audio_amplification: bool,
    /// 0–10 dB.
    pub audio_amplification_db: i64,
    pub downmix_enabled: bool,
    /// One of `2.0`, `2.1`, `3.0`, `3.1`, `4.0`, `4.1`, `5.0`, `5.1`, `7.0`, `7.1`.
    pub audio_output_channels: String,
    pub maintain_original_audio_on_downmix: bool,
    /// -10–30 dB, used by the downmix center filter.
    pub center_mix_level_db: i64,
    /// Bitstream AC3/E-AC3/DTS/DTS-HD/TrueHD to an AV receiver instead of decoding to PCM.
    #[serde(default)]
    pub audio_passthrough: bool,
    /// Encoding for subtitle files without a BOM (`mpv sub-codepage`), e.g. `cp1256` for Arabic SRT.
    #[serde(default)]
    pub subtitle_codepage: String,

    // ---- Buffer / disk cache (PlaybackBufferSettings / VodCacheSizing) ----
    pub buffer_enabled: bool,
    pub min_buffer_ms: i64,
    pub max_buffer_ms: i64,
    pub buffer_for_playback_ms: i64,
    pub buffer_for_playback_after_rebuffer_ms: i64,
    pub back_buffer_duration_ms: i64,
    pub target_buffer_size_mb: i64,
    pub vod_cache_enabled: bool,
    /// `auto` or `manual`.
    pub vod_cache_size_mode: String,
    /// 100–65536 MB (the original's default is 500 MB).
    pub vod_cache_size_mb: i64,
    pub cards: CardStyle,
    pub app_language: Option<String>,
    /// Tracking page: source that feeds the Library (`nuvio`, `trakt`, `simkl`, `mdblist`).
    #[serde(default)]
    pub library_source: String,
    /// Tracking page: source that powers resume / Continue Watching.
    #[serde(default)]
    pub progress_source: String,
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
            cards: CardStyle::default(),
            app_language: None,

            reuse_last_link_enabled: false,
            reuse_last_link_cache_hours: 24,
            auto_play_fallback_on_failure: false,
            auto_play_fallback_max_sources: 3,
            auto_play_fallback_startup_timeout_seconds: 15,
            auto_play_mode: "manual".into(),
            auto_play_source: "allSources".into(),
            auto_play_selected_addons: Vec::new(),
            auto_play_selected_plugins: Vec::new(),
            auto_play_regex: String::new(),
            auto_play_timeout_seconds: 3,

            auto_play_next_episode_enabled: false,
            auto_play_next_episode_fallback_enabled: true,
            auto_play_prefer_binge_group: true,
            auto_play_reuse_binge_group: false,
            preload_next_episode_sources: false,
            chapter_title_enabled: true,
            hold_to_speed_enabled: true,
            hold_to_speed_value: 2.0,
            show_parental_guide: true,

            remember_audio_delay_per_device: true,
            persist_audio_amplification: false,
            audio_amplification_db: 0,
            downmix_enabled: false,
            audio_output_channels: "2.0".into(),
            maintain_original_audio_on_downmix: true,
            center_mix_level_db: 0,
            audio_passthrough: false,
            subtitle_codepage: String::new(),

            buffer_enabled: false,
            min_buffer_ms: 15_000,
            max_buffer_ms: 45_000,
            buffer_for_playback_ms: 5_000,
            buffer_for_playback_after_rebuffer_ms: 3_000,
            back_buffer_duration_ms: 10_000,
            target_buffer_size_mb: 100,
            vod_cache_enabled: false,
            vod_cache_size_mode: "auto".into(),
            vod_cache_size_mb: 500,

            library_source: "nuvio".into(),
            progress_source: "nuvio".into(),
        }
    }
}

/// `ContinueWatchingPreferencesUiState` of the original (`features/watchprogress`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContinueWatchingSettings {
    pub visible: bool,
    /// `card`, `wide` or `poster`.
    pub style: String,
    pub up_next_from_furthest_episode: bool,
    pub use_episode_thumbnails: bool,
    pub show_unaired_next_up: bool,
    pub blur_next_up: bool,
    pub show_resume_prompt_on_launch: bool,
    /// `default`, `streaming` or `splitUpcoming`.
    pub sort_mode: String,
    /// `stream`, `details` or `detailsForUpcoming`.
    pub click_action: String,
}

impl Default for ContinueWatchingSettings {
    fn default() -> Self {
        Self {
            visible: true,
            style: "wide".into(),
            up_next_from_furthest_episode: true,
            use_episode_thumbnails: true,
            show_unaired_next_up: true,
            blur_next_up: false,
            show_resume_prompt_on_launch: true,
            sort_mode: "default".into(),
            click_action: "stream".into(),
        }
    }
}

impl ContinueWatchingSettings {
    pub fn sanitized(mut self) -> Self {
        if !["card", "wide", "poster"].contains(&self.style.as_str()) {
            self.style = "card".into();
        }
        if !["default", "streaming", "splitUpcoming"].contains(&self.sort_mode.as_str()) {
            self.sort_mode = "default".into();
        }
        if !["stream", "details", "detailsForUpcoming"].contains(&self.click_action.as_str()) {
            self.click_action = "stream".into();
        }
        self
    }
}

/// One section of the details screen (`MetaScreenSectionItem` in the original). All sections are
/// enabled by default, in the order declared by `MetaScreenSettingsRepository.definitions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MetaSectionSetting {
    pub key: String,
    pub enabled: bool,
    pub order: i32,
}

/// `MetaScreenSettingsUiState` of the original (`features/details`). Sections Astra does not
/// render yet (comments, collection, trailers as a section) are still stored for account parity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MetaScreenSettings {
    pub sections: Vec<MetaSectionSetting>,
    /// `normal`, `cinematic` or `dominant_color` (the original's default).
    pub background_mode: String,
    pub hero_trailer_playback: bool,
    pub tab_layout: bool,
    /// `horizontal` or `list`.
    pub episode_card_style: String,
    pub blur_unwatched_episodes: bool,
    pub poster_transition_enabled: bool,
    pub show_overall_ratings: bool,
    /// `SHOW_ALL`, `HIDE_WATCHED` or `HIDE_ALL`.
    pub episode_ratings_visibility: String,
}

impl Default for MetaScreenSettings {
    fn default() -> Self {
        let keys = [
            "actions", "overview", "episodes", "cast", "details", "production", "trailers",
            "comments", "collection", "moreLikeThis",
        ];
        Self {
            sections: keys
                .iter()
                .enumerate()
                .map(|(order, key)| MetaSectionSetting { key: (*key).into(), enabled: true, order: order as i32 })
                .collect(),
            background_mode: "dominant_color".into(),
            hero_trailer_playback: false,
            tab_layout: false,
            episode_card_style: "horizontal".into(),
            blur_unwatched_episodes: false,
            poster_transition_enabled: false,
            show_overall_ratings: true,
            episode_ratings_visibility: "SHOW_ALL".into(),
        }
    }
}

impl MetaScreenSettings {
    pub fn sanitized(mut self) -> Self {
        if !["normal", "cinematic", "dominant_color"].contains(&self.background_mode.as_str()) {
            self.background_mode = "dominant_color".into();
        }
        if !["horizontal", "list"].contains(&self.episode_card_style.as_str()) {
            self.episode_card_style = "horizontal".into();
        }
        if !["SHOW_ALL", "HIDE_WATCHED", "HIDE_ALL"].contains(&self.episode_ratings_visibility.as_str()) {
            self.episode_ratings_visibility = "SHOW_ALL".into();
        }
        if self.sections.is_empty() {
            self.sections = Self::default().sections;
        }
        self
    }

    /// Whether a section is enabled, by its key. Unknown keys are enabled (new original sections).
    pub fn section_enabled(&self, key: &str) -> bool {
        self.sections.iter().find(|s| s.key == key).map_or(true, |s| s.enabled)
    }
}

/// Interface languages Astra has (`AppLanguage` of the original).
pub const APP_LANGUAGES: &[&str] = &["pt-BR", "en", "ar"];
pub const CARD_WIDTHS: &[i64] = &[104, 112, 120, 126, 134, 140];
pub const CARD_RADII: &[i64] = &[0, 4, 8, 12, 16];

/// `PosterCardStyleRepository` preferences, kept with the player settings for sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CardStyle {
    pub width: i64,
    pub radius: i64,
    pub landscape: bool,
    pub hide_labels: bool,
}
impl Default for CardStyle {
    fn default() -> Self { Self { width: 126, radius: 12, landscape: false, hide_labels: false } }
}
impl CardStyle {
    fn sanitized(mut self) -> Self {
        let nearest = |v: i64, options: &[i64]| *options.iter().min_by_key(|o| (*o - v).abs()).expect("options");
        self.width = nearest(self.width, CARD_WIDTHS);
        self.radius = nearest(self.radius, CARD_RADII);
        self
    }
}

/// TorBox switch (`enabled`: "Resolver links reproduzíveis").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DebridSettings { pub enabled: bool }
impl Default for DebridSettings { fn default() -> Self { Self { enabled: true } } }

/// Streams page settings (the original's StreamBadgeSettingsUiState). `show_size_badges = None`
/// means "use the value imported from the original app".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamsSettings {
    pub show_size_badges: Option<bool>,
    pub show_addon_logo: bool,
    pub show_in_player: bool,
    pub player_badge_source: String,
    /// `bottom` or `top`.
    pub badge_placement: String,
    /// `normal` or `cinematic`.
    pub background_mode: String,
    /// User-added Fusion badge URLs (the original keeps at most three imports).
    pub badge_urls: Vec<String>,
}

impl Default for StreamsSettings {
    fn default() -> Self {
        Self {
            show_size_badges: None, show_addon_logo: false, show_in_player: false,
            player_badge_source: String::new(), badge_placement: "bottom".into(),
            background_mode: "normal".into(), badge_urls: Vec::new(),
        }
    }
}

impl StreamsSettings {
    pub fn sanitized(mut self) -> Self {
        if !matches!(self.badge_placement.as_str(), "bottom" | "top") { self.badge_placement = "bottom".into(); }
        if !matches!(self.background_mode.as_str(), "normal" | "cinematic") { self.background_mode = "normal".into(); }
        self.badge_urls.truncate(3);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileSettings {
    pub player: PlayerSettings,
    pub home: HomeCatalogSettings,
    pub p2p: P2pSettings,
    #[serde(default)]
    pub continue_watching: ContinueWatchingSettings,
    #[serde(default)]
    pub meta_screen: MetaScreenSettings,
    /// Discord Rich Presence (the original's `DiscordRichPresenceRepository`), off by default.
    #[serde(default)]
    pub discord_rpc_enabled: bool,
    #[serde(default)]
    pub debrid: DebridSettings,
    #[serde(default)]
    pub streams: StreamsSettings,
}

pub use nuvio_core::auto_play::{STREAM_AUTO_PLAY_TIMEOUT_VALUES, snap_auto_play_timeout};
pub const REUSE_LINK_CACHE_HOURS: &[i64] = &[1, 2, 3, 6, 12, 24, 48, 72, 168];
pub const AUDIO_OUTPUT_CHANNELS: &[&str] = &["2.0", "2.1", "3.0", "3.1", "4.0", "4.1", "5.0", "5.1", "7.0", "7.1"];
/// Values accepted for mpv's `sub-codepage` (legacy SRT encodings). Empty means mpv's default.
pub const SUBTITLE_CODEPAGES: &[&str] = &[
    "", "auto", "UTF-8", "cp1250", "cp1251", "cp1252", "cp1253", "cp1254", "cp1255", "cp1256", "cp1257", "cp1258", "iso-8859-1", "iso-8859-6", "gbk", "big5", "shift_jis", "euc-kr",
];
pub const AUTO_PLAY_MODES: &[&str] = &["manual", "firstStream", "regexMatch"];
pub const AUTO_PLAY_SOURCES: &[&str] = &["allSources", "installedAddonsOnly", "enabledPluginsOnly"];

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
        self.cards = self.cards.sanitized();
        self.app_language = self.app_language.and_then(|code| APP_LANGUAGES.iter().find(|l| l.eq_ignore_ascii_case(&code)).map(|l| l.to_string()));

        if !REUSE_LINK_CACHE_HOURS.contains(&self.reuse_last_link_cache_hours) {
            self.reuse_last_link_cache_hours = d.reuse_last_link_cache_hours;
        }
        self.auto_play_fallback_max_sources = self.auto_play_fallback_max_sources.clamp(1, 10);
        self.auto_play_fallback_startup_timeout_seconds = self.auto_play_fallback_startup_timeout_seconds.clamp(5, 45);
        if !AUTO_PLAY_MODES.contains(&self.auto_play_mode.as_str()) {
            self.auto_play_mode = d.auto_play_mode;
        }
        if !AUTO_PLAY_SOURCES.contains(&self.auto_play_source.as_str()) {
            self.auto_play_source = d.auto_play_source;
        }
        self.auto_play_timeout_seconds = snap_auto_play_timeout(self.auto_play_timeout_seconds);
        self.auto_play_regex = self.auto_play_regex.trim().to_string();
        self.hold_to_speed_value = self.hold_to_speed_value.clamp(1.0, 4.0);
        self.audio_amplification_db = self.audio_amplification_db.clamp(0, 10);
        if !AUDIO_OUTPUT_CHANNELS.contains(&self.audio_output_channels.as_str()) {
            self.audio_output_channels = d.audio_output_channels;
        }
        self.center_mix_level_db = self.center_mix_level_db.clamp(-10, 30);
        self.subtitle_codepage = SUBTITLE_CODEPAGES
            .iter()
            .find(|code| code.eq_ignore_ascii_case(self.subtitle_codepage.trim()))
            .copied()
            .unwrap_or_default()
            .to_string();
        // Buffer/caches: ms values, as in the original's sliders.
        self.min_buffer_ms = self.min_buffer_ms.clamp(5_000, 120_000);
        self.max_buffer_ms = self.max_buffer_ms.clamp(5_000, 1_200_000);
        self.buffer_for_playback_ms = self.buffer_for_playback_ms.clamp(1_000, 60_000);
        self.buffer_for_playback_after_rebuffer_ms = self.buffer_for_playback_after_rebuffer_ms.clamp(1_000, 120_000);
        self.back_buffer_duration_ms = self.back_buffer_duration_ms.clamp(0, 120_000);
        self.target_buffer_size_mb = self.target_buffer_size_mb.clamp(50, 65_536);
        if !matches!(self.vod_cache_size_mode.as_str(), "auto" | "manual") {
            self.vod_cache_size_mode = d.vod_cache_size_mode;
        }
        self.vod_cache_size_mb = self.vod_cache_size_mb.clamp(100, 65_536);
        for source in [&mut self.library_source, &mut self.progress_source] {
            if !matches!(source.as_str(), "nuvio" | "trakt" | "simkl" | "mdblist") {
                *source = "nuvio".into();
            }
        }
        self
    }
}

impl ProfileSettings {
    pub fn sanitized(mut self) -> Self {
        self.player = self.player.sanitized();
        self.p2p = self.p2p.sanitized();
        let mut seen = std::collections::HashSet::new();
        self.home.items.retain(|i| !i.key.trim().is_empty() && seen.insert(i.key.clone()));
        self.continue_watching = self.continue_watching.sanitized();
        self.meta_screen = self.meta_screen.sanitized();
        self.streams = self.streams.sanitized();
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
        let list = |name: &str| -> Vec<String> {
            match get(name) {
                Some(value) => serde_json::from_str::<Vec<String>>(&value).unwrap_or_else(|_| {
                    value.split(',').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()
                }),
                None => Vec::new(),
            }
        };
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
            cards: CardStyle {
                width: int("cards_width_dp").unwrap_or(126),
                radius: int("cards_corner_radius_dp").unwrap_or(12),
                landscape: boolean("cards_landscape").unwrap_or(false),
                hide_labels: boolean("cards_hide_labels").unwrap_or(false),
            },
            app_language: get("selected_app_language"),

            reuse_last_link_enabled: boolean("stream_reuse_last_link_enabled").unwrap_or(d.reuse_last_link_enabled),
            reuse_last_link_cache_hours: int("stream_reuse_last_link_cache_hours").unwrap_or(d.reuse_last_link_cache_hours),
            auto_play_fallback_on_failure: boolean("stream_auto_play_fallback_on_failure").unwrap_or(d.auto_play_fallback_on_failure),
            auto_play_fallback_max_sources: int("stream_auto_play_fallback_max_sources").unwrap_or(d.auto_play_fallback_max_sources),
            auto_play_fallback_startup_timeout_seconds: int("stream_auto_play_fallback_startup_timeout_seconds").unwrap_or(d.auto_play_fallback_startup_timeout_seconds),
            auto_play_mode: get("stream_auto_play_mode").unwrap_or(d.auto_play_mode),
            auto_play_source: get("stream_auto_play_source").unwrap_or(d.auto_play_source),
            auto_play_selected_addons: list("stream_auto_play_selected_addons"),
            auto_play_selected_plugins: list("stream_auto_play_selected_plugins"),
            auto_play_regex: get("stream_auto_play_regex").unwrap_or_default(),
            auto_play_timeout_seconds: int("stream_auto_play_timeout_seconds").unwrap_or(d.auto_play_timeout_seconds),

            auto_play_next_episode_enabled: boolean("stream_auto_play_next_episode_enabled").unwrap_or(d.auto_play_next_episode_enabled),
            auto_play_next_episode_fallback_enabled: boolean("stream_auto_play_next_episode_fallback_enabled").unwrap_or(d.auto_play_next_episode_fallback_enabled),
            auto_play_prefer_binge_group: boolean("stream_auto_play_prefer_binge_group").unwrap_or(d.auto_play_prefer_binge_group),
            auto_play_reuse_binge_group: boolean("stream_auto_play_reuse_binge_group").unwrap_or(d.auto_play_reuse_binge_group),
            preload_next_episode_sources: boolean("preload_next_episode_sources").unwrap_or(d.preload_next_episode_sources),
            chapter_title_enabled: boolean("chapter_title_enabled").unwrap_or(d.chapter_title_enabled),
            hold_to_speed_enabled: boolean("hold_to_speed_enabled").unwrap_or(d.hold_to_speed_enabled),
            hold_to_speed_value: number("hold_to_speed_value").unwrap_or(d.hold_to_speed_value),
            show_parental_guide: boolean("show_parental_guide").unwrap_or(d.show_parental_guide),

            remember_audio_delay_per_device: boolean("remember_audio_delay_per_device").unwrap_or(d.remember_audio_delay_per_device),
            persist_audio_amplification: boolean("persist_audio_amplification").unwrap_or(d.persist_audio_amplification),
            audio_amplification_db: int("audio_amplification_db").unwrap_or(d.audio_amplification_db),
            downmix_enabled: boolean("downmix_enabled").unwrap_or(d.downmix_enabled),
            audio_output_channels: get("audio_output_channels").unwrap_or(d.audio_output_channels),
            maintain_original_audio_on_downmix: boolean("maintain_original_audio_on_downmix").unwrap_or(d.maintain_original_audio_on_downmix),
            center_mix_level_db: int("center_mix_level_db").unwrap_or(d.center_mix_level_db),
            audio_passthrough: boolean("audio_passthrough").unwrap_or(d.audio_passthrough),
            subtitle_codepage: get("subtitle_codepage").unwrap_or(d.subtitle_codepage),

            buffer_enabled: boolean("buffer_engine_enabled").unwrap_or(d.buffer_enabled),
            min_buffer_ms: int("min_buffer_ms").unwrap_or(d.min_buffer_ms),
            max_buffer_ms: int("max_buffer_ms").unwrap_or(d.max_buffer_ms),
            buffer_for_playback_ms: int("buffer_for_playback_ms").unwrap_or(d.buffer_for_playback_ms),
            buffer_for_playback_after_rebuffer_ms: int("buffer_for_playback_after_rebuffer_ms").unwrap_or(d.buffer_for_playback_after_rebuffer_ms),
            back_buffer_duration_ms: int("back_buffer_duration_ms").unwrap_or(d.back_buffer_duration_ms),
            target_buffer_size_mb: int("target_buffer_size_mb").unwrap_or(d.target_buffer_size_mb),
            vod_cache_enabled: boolean("vod_cache_enabled").unwrap_or(d.vod_cache_enabled),
            vod_cache_size_mode: get("vod_cache_size_mode").unwrap_or(d.vod_cache_size_mode),
            vod_cache_size_mb: int("vod_cache_size_mb").unwrap_or(d.vod_cache_size_mb),
            library_source: get("library_source").unwrap_or(d.library_source),
            progress_source: get("progress_source").unwrap_or(d.progress_source),
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
            continue_watching: ContinueWatchingSettings::default(),
            meta_screen: MetaScreenSettings::default(),
            debrid: DebridSettings::default(),
            streams: StreamsSettings::default(),
            discord_rpc_enabled: self.read_store("nuvio_discord_rich_presence")
                .ok()
                .and_then(|store| store.get(&format!("enabled_{profile_index}")).or_else(|| store.get("enabled")).cloned())
                .and_then(|value| value.trim().parse::<bool>().ok())
                .unwrap_or(false),
        }
        .sanitized())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continue_watching_defaults_match_the_original_and_are_sanitized() {
        let cw = ContinueWatchingSettings::default();
        assert_eq!((cw.visible, cw.up_next_from_furthest_episode, cw.use_episode_thumbnails, cw.show_unaired_next_up, cw.show_resume_prompt_on_launch), (true, true, true, true, true));
        assert_eq!((cw.style.as_str(), cw.sort_mode.as_str(), cw.click_action.as_str()), ("card", "default", "stream"));
        assert!(!cw.blur_next_up);
        let bad = ContinueWatchingSettings { style: "x".into(), sort_mode: "y".into(), click_action: "z".into(), ..Default::default() }.sanitized();
        assert_eq!((bad.style.as_str(), bad.sort_mode.as_str(), bad.click_action.as_str()), ("card", "default", "stream"));
        assert_eq!(ProfileSettings::default().continue_watching, ContinueWatchingSettings::default());
    }

    #[test]
    fn advanced_audio_and_codepage_defaults_are_sanitized() {
        let p = PlayerSettings::default();
        assert!(!p.audio_passthrough);
        assert!(p.subtitle_codepage.is_empty());
        let bad = PlayerSettings { subtitle_codepage: "not;a;codepage".into(), ..Default::default() }.sanitized();
        assert!(bad.subtitle_codepage.is_empty());
        let arabic = PlayerSettings { subtitle_codepage: "CP1256".into(), audio_passthrough: true, ..Default::default() }.sanitized();
        assert_eq!(arabic.subtitle_codepage, "cp1256");
        assert!(arabic.audio_passthrough);
    }

    #[test]
    fn phase2_defaults_and_ranges() {
        let p = PlayerSettings::default();
        assert!(!p.reuse_last_link_enabled && p.reuse_last_link_cache_hours == 24);
        assert!(!p.auto_play_fallback_on_failure && p.auto_play_fallback_max_sources == 3 && p.auto_play_fallback_startup_timeout_seconds == 15);
        assert_eq!((p.auto_play_mode.as_str(), p.auto_play_source.as_str(), p.auto_play_timeout_seconds), ("manual", "allSources", 3));
        assert!(!p.auto_play_next_episode_enabled && p.auto_play_next_episode_fallback_enabled);
        assert!(p.auto_play_prefer_binge_group && !p.auto_play_reuse_binge_group && !p.preload_next_episode_sources);
        assert!(p.chapter_title_enabled && p.hold_to_speed_enabled && p.show_parental_guide);
        assert_eq!(p.hold_to_speed_value, 2.0);
        assert_eq!((p.audio_amplification_db, p.center_mix_level_db, p.audio_output_channels.as_str()), (0, 0, "2.0"));
        assert!(!p.buffer_enabled && p.min_buffer_ms == 15_000 && p.max_buffer_ms == 45_000 && p.target_buffer_size_mb == 100);
        assert!(!p.vod_cache_enabled && p.vod_cache_size_mode == "auto" && p.vod_cache_size_mb == 500);

        let bad = PlayerSettings {
            auto_play_mode: "bad".into(), auto_play_source: "bad".into(), auto_play_timeout_seconds: 11,
            hold_to_speed_value: 9.0, audio_amplification_db: 99, center_mix_level_db: 99,
            audio_output_channels: "9.9".into(), buffer_enabled: true, min_buffer_ms: 1,
            max_buffer_ms: 9_000_000, target_buffer_size_mb: 1, vod_cache_size_mode: "bad".into(),
            vod_cache_size_mb: 1, ..Default::default()
        }.sanitized();
        assert_eq!((bad.auto_play_mode.as_str(), bad.auto_play_source.as_str()), ("manual", "allSources"));
        assert_eq!(bad.auto_play_timeout_seconds, 10);
        assert_eq!(bad.hold_to_speed_value, 4.0);
        assert_eq!((bad.audio_amplification_db, bad.center_mix_level_db, bad.audio_output_channels.as_str()), (10, 30, "2.0"));
        assert_eq!((bad.min_buffer_ms, bad.max_buffer_ms, bad.target_buffer_size_mb), (5_000, 1_200_000, 50));
        assert_eq!((bad.vod_cache_size_mode.as_str(), bad.vod_cache_size_mb), ("auto", 100));
    }

    #[test]
    fn meta_screen_defaults_and_section_lookup() {
        let meta = MetaScreenSettings::default();
        assert_eq!(meta.sections.len(), 10);
        assert!(meta.sections.iter().all(|s| s.enabled));
        assert_eq!(meta.sections[0].key, "actions");
        assert_eq!(meta.background_mode, "dominant_color");
        assert_eq!(meta.episode_card_style, "horizontal");
        assert!(meta.show_overall_ratings);
        assert!(meta.section_enabled("overview"));
        assert!(meta.section_enabled("aSectionTheOriginalAddsLater"));
        let off = MetaSectionSetting { key: "cast".into(), enabled: false, order: 3 };
        let meta = MetaScreenSettings { sections: vec![off], background_mode: "bad".into(), episode_card_style: "bad".into(), episode_ratings_visibility: "bad".into(), ..Default::default() }.sanitized();
        assert!(!meta.section_enabled("cast"));
        assert_eq!(meta.background_mode, "dominant_color");
        assert_eq!(meta.episode_card_style, "horizontal");
        assert_eq!(meta.episode_ratings_visibility, "SHOW_ALL");
    }
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
