//! Pure source-selection rules ported from NuvioDesktop:
//! `StreamAutoPlayPolicy`, `StreamAutoPlaySelector` and `snapToAllowedTimeout`.
//!
//! The UI only holds display-safe data (addon names, labels, opaque keys), so the same rules can
//! be applied to the sources the desktop app actually has without exposing stream URLs to it.

use regex::{Regex, RegexBuilder};

/// `STREAM_AUTO_PLAY_TIMEOUT_VALUES` of the original; `i64::MAX` means "unlimited".
pub const STREAM_AUTO_PLAY_TIMEOUT_VALUES: &[i64] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 20, 25, 30, i64::MAX];

/// `snapToAllowedTimeout`: nearest allowed value, ties go to the lower one.
pub fn snap_auto_play_timeout(value: i64) -> i64 {
    if value <= 0 { return 0; }
    STREAM_AUTO_PLAY_TIMEOUT_VALUES
        .iter()
        .copied()
        .min_by_key(|allowed| (value - *allowed).abs())
        .unwrap_or(3)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoPlayMode {
    Manual,
    FirstStream,
    RegexMatch,
}

impl AutoPlayMode {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "firstStream" => Self::FirstStream,
            "regexMatch" => Self::RegexMatch,
            _ => Self::Manual,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoPlaySource {
    AllSources,
    InstalledAddonsOnly,
    EnabledPluginsOnly,
}

impl AutoPlaySource {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "installedAddonsOnly" => Self::InstalledAddonsOnly,
            "enabledPluginsOnly" => Self::EnabledPluginsOnly,
            _ => Self::AllSources,
        }
    }
}

/// `isRegexSelectionConfigured`: a non-blank pattern with at least one letter or digit that the
/// regex engine accepts (case-insensitive, like the original's `RegexOption.IGNORE_CASE`).
pub fn is_regex_selection_configured(pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() || !pattern.chars().any(|c| c.is_alphanumeric()) {
        return false;
    }
    RegexBuilder::new(pattern).case_insensitive(true).build().is_ok()
}

/// `StreamAutoPlayPolicy.isEffectivelyEnabled` (reuse link / binge-only / mode).
pub fn is_effectively_enabled(reuse_last_link: bool, reuse_binge_group: bool, mode: AutoPlayMode, regex: &str) -> bool {
    if reuse_last_link || reuse_binge_group {
        return true;
    }
    match mode {
        AutoPlayMode::Manual => false,
        AutoPlayMode::FirstStream => true,
        AutoPlayMode::RegexMatch => is_regex_selection_configured(regex),
    }
}

/// `PlayerNextEpisodeAutoPlay.manual-mode promotion`: when the next episode should be auto-selected
/// even though the general mode is MANUAL. Also returns the preferred binge group to keep.
pub fn promote_next_episode_mode(
    mode: AutoPlayMode,
    next_episode_enabled: bool,
    fallback_enabled: bool,
    prefer_binge_group: bool,
    current_binge_group: Option<&str>,
) -> (AutoPlayMode, Option<&str>) {
    let should = mode == AutoPlayMode::Manual && (next_episode_enabled || prefer_binge_group);
    if !should {
        return (mode, None);
    }
    let binge_only = (!next_episode_enabled || !fallback_enabled) && prefer_binge_group;
    (AutoPlayMode::FirstStream, if binge_only { current_binge_group } else { None })
}

/// One source the selection rules can look at. `text` is the same concatenation as the original
/// (`addonName + name + streamLabel + description + url`), with URLs redacted by the caller.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub key: &'a str,
    pub addon_name: &'a str,
    pub text: String,
    /// Direct URL / server target / debrid source, i.e. `isAutoPlayable`.
    pub ready: bool,
    pub binge_group: Option<&'a str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The source to play, if any.
    pub index: Option<usize>,
    /// Every ready candidate that matched, in preference order.
    pub ready: Vec<usize>,
    /// A matching candidate that is not ready yet (debrid still resolving).
    pub has_pending: bool,
}

fn scope_allowed(
    candidate: &Candidate<'_>,
    source: AutoPlaySource,
    installed: &[String],
    selected_addons: &[String],
    selected_plugins: &[String],
) -> bool {
    let is_installed = installed.iter().any(|name| name == candidate.addon_name);
    let in_scope = match source {
        AutoPlaySource::AllSources => true,
        AutoPlaySource::InstalledAddonsOnly => is_installed,
        AutoPlaySource::EnabledPluginsOnly => !is_installed,
    };
    if !in_scope {
        return false;
    }
    if !selected_addons.is_empty() && !selected_addons.iter().any(|name| name == candidate.addon_name) {
        return false;
    }
    if !selected_plugins.is_empty() && !selected_plugins.iter().any(|name| name == candidate.addon_name) {
        return false;
    }
    true
}

/// Kotlin's exclusion-word extraction: `(?!word|other)` inside the user's pattern means "match
/// unless the text contains word/other" (a convenience on top of the user's regex).
fn exclusion_words(pattern: &str) -> Vec<String> {
    let Some(re) = Regex::new(r"\(\?![^)]*?\(([^)]+)\)").ok() else { return Vec::new() };
    re.captures_iter(pattern)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .flat_map(|group| group.split('|').map(|word| word.trim().to_string()).collect::<Vec<_>>())
        .filter(|word| !word.is_empty())
        .collect()
}

/// `evaluateAutoPlayStream`: scope filter → regex match → preferred binge group first → ready
/// source. `mode` MANUAL yields no selection.
#[allow(clippy::too_many_arguments)]
pub fn select(
    candidates: &[Candidate<'_>],
    mode: AutoPlayMode,
    source: AutoPlaySource,
    installed: &[String],
    selected_addons: &[String],
    selected_plugins: &[String],
    regex: &str,
    preferred_binge_group: Option<&str>,
) -> Selection {
    let mut out = Selection::default();
    if mode == AutoPlayMode::Manual {
        return out;
    }
    let user = if mode == AutoPlayMode::RegexMatch {
        match RegexBuilder::new(regex.trim()).case_insensitive(true).build() {
            Ok(re) => Some(re),
            Err(_) => return out,
        }
    } else {
        None
    };
    let exclusions = exclusion_words(regex);
    let exclude = if exclusions.is_empty() {
        None
    } else {
        let alternation = exclusions
            .iter()
            .map(|word| regex::escape(word))
            .collect::<Vec<_>>()
            .join("|");
        RegexBuilder::new(&format!(r"\b(?:{alternation})\b"))
            .case_insensitive(true)
            .build()
            .ok()
    };

    let mut matching: Vec<usize> = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if !scope_allowed(candidate, source, installed, selected_addons, selected_plugins) {
            continue;
        }
        if let Some(re) = &user {
            if !re.is_match(&candidate.text) {
                continue;
            }
        }
        if exclude.as_ref().is_some_and(|re| re.is_match(&candidate.text)) {
            continue;
        }
        matching.push(index);
    }

    let is_ready = |i: usize| candidates[i].ready;
    let preferred = |i: usize| {
        preferred_binge_group.is_some()
            && candidates[i].binge_group.is_some()
            && candidates[i].binge_group == preferred_binge_group
    };
    for i in matching.iter().copied().filter(|i| preferred(*i) && is_ready(*i)) {
        out.ready.push(i);
    }
    for i in matching.iter().copied().filter(|i| !preferred(*i) && is_ready(*i)) {
        out.ready.push(i);
    }
    out.has_pending = matching.iter().copied().any(|i| !is_ready(i));
    out.index = out.ready.first().copied();
    out
}

/// `nextSourceFallbackCandidate` / `resolveWithSourceFallback`: the next ready candidate that has
/// not been tried and is not another copy of the failed source.
pub fn next_fallback(candidates: &[Candidate<'_>], tried: &[&str], failed_key: &str, max_sources: usize) -> Option<usize> {
    if max_sources == 0 {
        return None;
    }
    candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.ready && c.key != failed_key && !tried.iter().any(|k| *k == c.key))
        .take(max_sources)
        .map(|(i, _)| i)
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> Vec<Candidate<'static>> {
        vec![
            Candidate { key: "a", addon_name: "Torrentio", text: "Torrentio 1080p WEBRip".into(), ready: true, binge_group: Some("group-a") },
            Candidate { key: "b", addon_name: "Torrentio", text: "Torrentio 720p WEBRip".into(), ready: true, binge_group: Some("group-a") },
            Candidate { key: "c", addon_name: "Plugin", text: "Plugin 2160p 4K".into(), ready: true, binge_group: None },
            Candidate { key: "d", addon_name: "Addon", text: "Addon 1080p CAM".into(), ready: false, binge_group: None },
        ]
    }

    #[test]
    fn regex_configuration_matches_the_original() {
        assert!(!is_regex_selection_configured("   "));
        assert!(!is_regex_selection_configured("???"));
        assert!(!is_regex_selection_configured("["));
        assert!(is_regex_selection_configured("(?i)1080p"));
    }

    #[test]
    fn policy_and_timeout_snap() {
        assert!(!is_effectively_enabled(false, false, AutoPlayMode::Manual, "1080p"));
        assert!(is_effectively_enabled(false, false, AutoPlayMode::FirstStream, ""));
        assert!(is_effectively_enabled(false, false, AutoPlayMode::RegexMatch, "1080p"));
        assert!(is_effectively_enabled(true, false, AutoPlayMode::Manual, ""));
        assert!(is_effectively_enabled(false, true, AutoPlayMode::Manual, ""));
        assert_eq!(snap_auto_play_timeout(4), 4);
        assert_eq!(snap_auto_play_timeout(12), 10);
        assert_eq!(snap_auto_play_timeout(-5), 0);
        assert_eq!(snap_auto_play_timeout(11), 10);
        assert_eq!(snap_auto_play_timeout(i64::MAX), i64::MAX);
    }

    #[test]
    fn first_stream_and_regex_selection() {
        let c = candidates();
        let installed = vec!["Torrentio".to_string(), "Addon".to_string()];
        let first = select(&c, AutoPlayMode::FirstStream, AutoPlaySource::AllSources, &installed, &[], &[], "", None);
        assert_eq!(first.index, Some(0));
        assert_eq!(first.ready, vec![0, 1, 2]);
        assert!(first.has_pending);

        let regex = select(&c, AutoPlayMode::RegexMatch, AutoPlaySource::AllSources, &installed, &[], &[], "4K", None);
        assert_eq!(regex.index, Some(2));
        let scoped = select(&c, AutoPlayMode::FirstStream, AutoPlaySource::InstalledAddonsOnly, &installed, &[], &[], "", None);
        assert_eq!(scoped.ready, vec![0, 1]);
        let plugins = select(&c, AutoPlayMode::FirstStream, AutoPlaySource::EnabledPluginsOnly, &installed, &[], &[], "", None);
        assert_eq!(plugins.ready, vec![2]);
    }

    #[test]
    fn preferred_binge_group_and_exclusion_words() {
        let c = candidates();
        let s = select(&c, AutoPlayMode::FirstStream, AutoPlaySource::AllSources, &[], &[], &[], "", Some("group-a"));
        assert_eq!(s.index, Some(0));
        let excluded = select(&c, AutoPlayMode::RegexMatch, AutoPlaySource::AllSources, &[], &[], &[], "1080p (?!WEBRip|CAM)", None);
        assert_eq!(excluded.index, None, "all 1080p candidates are excluded");
    }

    #[test]
    fn manual_mode_promotion_and_fallback() {
        let (mode, group) = promote_next_episode_mode(AutoPlayMode::Manual, true, true, true, Some("keep"));
        assert_eq!(mode, AutoPlayMode::FirstStream);
        assert_eq!(group, None);
        let (mode, group) = promote_next_episode_mode(AutoPlayMode::Manual, false, true, true, Some("keep"));
        assert_eq!(mode, AutoPlayMode::FirstStream);
        assert_eq!(group, Some("keep"));
        let (mode, _) = promote_next_episode_mode(AutoPlayMode::RegexMatch, true, true, true, None);
        assert_eq!(mode, AutoPlayMode::RegexMatch);
        let c = candidates();
        assert_eq!(next_fallback(&c, &["a"], "a", 3), Some(1));
        assert_eq!(next_fallback(&c, &["a", "b", "c"], "a", 3), None);
    }
}
