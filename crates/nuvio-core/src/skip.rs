//! Intro / recap / credits segments and the next-episode moment.
//!
//! Port of the original's `features/player/skip`: `SkipModels.kt` (IntroDB and AniSkip answers),
//! `ChapterSkipIntervals`, the merge per category, the skip action (never over a post-credits
//! scene) and `PlayerNextEpisodeRules.shouldShowNextEpisodeCard`.

use crate::CoreError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A scene after the credits is assumed when more than this remains after them.
const POST_CREDITS_GAP: f64 = 5.0;
/// Chapters shorter or longer than this are not an opening, ending or recap.
const CHAPTER_SECONDS: std::ops::RangeInclusive<f64> = 5.0..=240.0;
/// Clips shorter than this are placeholders: nothing is offered on them.
const MIN_CONTENT_SECONDS: f64 = 121.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Intro,
    Recap,
    Outro,
    MovieCredits,
    PostCredits,
}

impl Kind {
    /// Merge category; a post-credits scene has none (kept from every source).
    fn category(self) -> Option<Kind> {
        match self {
            Kind::PostCredits => None,
            Kind::MovieCredits => Some(Kind::Outro),
            other => Some(other),
        }
    }

    /// `AutoSkipSegmentType` stored value.
    pub fn setting(self) -> Option<&'static str> {
        match self {
            Kind::Intro => Some("intro"),
            Kind::Recap => Some("recap"),
            Kind::Outro => Some("outro"),
            Kind::MovieCredits => Some("movie-credits"),
            Kind::PostCredits => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub kind: Kind,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Chapter {
    pub time: f64,
    #[serde(default)]
    pub title: String,
}

/// IntroDB `GET /segments`.
pub fn parse_introdb(payload: &str, movie: bool) -> Result<Vec<Segment>, CoreError> {
    let root: Value = serde_json::from_str(payload)?;
    let bound = |part: &Value, seconds: &str, millis: &str| {
        part.get(seconds).and_then(Value::as_f64).or_else(|| part.get(millis).and_then(Value::as_f64).map(|v| v / 1000.0))
    };
    let read = |name: &str, kind: Kind| -> Option<Segment> {
        let part = root.get(name).filter(|v| v.is_object())?;
        let start = bound(part, "start_sec", "start_ms")?;
        let end = bound(part, "end_sec", "end_ms").or((kind == Kind::PostCredits).then_some(f64::MAX))?;
        (start >= 0.0 && end > start).then_some(Segment { kind, start, end })
    };
    let post = read("post_credits", Kind::PostCredits);
    let mut out = Vec::new();
    if movie {
        if let Some(mut credits) = read("outro", Kind::MovieCredits) {
            if let Some(scene) = post.filter(|p| p.start > credits.start && p.start < credits.end) {
                credits.end = scene.start;
            }
            out.push(credits);
        }
    } else {
        out.extend([read("intro", Kind::Intro), read("recap", Kind::Recap), read("outro", Kind::Outro)].into_iter().flatten());
    }
    out.extend(post);
    Ok(out)
}

/// AniSkip `GET /v2/skip-times/{mal}/{episode}`.
pub fn parse_aniskip(payload: &str) -> Result<Vec<Segment>, CoreError> {
    let root: Value = serde_json::from_str(payload)?;
    let results = root.get("results").and_then(Value::as_array);
    Ok(results
        .into_iter()
        .flatten()
        .filter_map(|result| {
            let kind = match result.get("skipType")?.as_str()? {
                "op" | "mixed-op" => Kind::Intro,
                "ed" | "mixed-ed" => Kind::Outro,
                "recap" => Kind::Recap,
                _ => return None,
            };
            let start = result.pointer("/interval/startTime")?.as_f64()?;
            let end = result.pointer("/interval/endTime")?.as_f64()?;
            (start >= 0.0 && end > start).then_some(Segment { kind, start, end })
        })
        .collect())
}

/// `word( credits| theme)? number?` as a whole title (spaces optional).
fn is_titled(title: &str, word: &str, suffixes: &[&str]) -> bool {
    let Some(rest) = title.strip_prefix(word).map(str::trim_start) else { return false };
    let rest = suffixes.iter().find_map(|s| rest.strip_prefix(*s)).unwrap_or(rest).trim_start();
    rest.chars().all(|c| c.is_ascii_digit())
}

/// `ChapterSkipIntervals`: what the file's own chapter names say.
pub fn from_chapters(chapters: &[Chapter], duration: f64, movie: bool) -> Vec<Segment> {
    if chapters.len() < 2 {
        return Vec::new();
    }
    let mut sorted: Vec<(f64, String)> = chapters.iter().map(|c| (c.time.max(0.0), c.title.trim().to_lowercase())).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let opening = |t: &str| ["opening", "op"].iter().any(|w| is_titled(t, w, &["credits", "theme"]));
    let has_opening = sorted.iter().any(|(_, t)| opening(t));
    let mut out = Vec::new();
    for (i, (start, title)) in sorted.iter().enumerate() {
        let end = sorted.get(i + 1).map_or(duration, |next| next.0);
        let t = title.as_str();
        let kind = if ["post", "mid", "after"].iter().any(|w| [" credits", "-credits", "credits"].iter().any(|c| t.contains(&format!("{w}{c}")))) {
            Kind::PostCredits
        } else if opening(t) || (!has_opening && t == "intro") {
            Kind::Intro
        } else if ["ending", "ed"].iter().any(|w| is_titled(t, w, &["credits", "theme"]))
            || t == "credits"
            || ["end", "closing"].iter().any(|w| t.strip_prefix(w).is_some_and(|rest| rest.trim_start() == "credits"))
        {
            if movie { Kind::MovieCredits } else { Kind::Outro }
        } else if t == "recap" || t == "previously" || t.starts_with("previously on") {
            Kind::Recap
        } else {
            continue;
        };
        let length = end - start;
        if length <= 0.0 || (kind != Kind::PostCredits && !CHAPTER_SECONDS.contains(&length)) {
            continue;
        }
        out.push(Segment { kind, start: *start, end });
    }
    out
}

/// Chapters win per category over the services, which win in the given order; a movie with
/// segments from a service ignores its chapters.
pub fn merge(chapters: Vec<Segment>, services: Vec<Vec<Segment>>, movie: bool) -> Vec<Segment> {
    let from_services = services.iter().any(|list| !list.is_empty());
    let mut sources = services;
    if !(movie && from_services) {
        sources.insert(0, chapters);
    }
    let mut out: Vec<Segment> = Vec::new();
    for source in sources {
        let taken: Vec<Kind> = out.iter().filter_map(|s| s.kind.category()).collect();
        let fresh: Vec<Segment> = source
            .into_iter()
            .filter(|s| match s.kind.category() {
                Some(category) => !taken.contains(&category),
                None => !out.iter().any(|o| o.kind == Kind::PostCredits && s.start < o.end && o.start < s.end),
            })
            .collect();
        out.extend(fresh);
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// When the next-episode card shows without a known ending.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Threshold {
    Percent(f64),
    MinutesBeforeEnd(f64),
}

impl Threshold {
    fn seconds_before_end(self, duration: f64) -> f64 {
        match self {
            Threshold::Percent(percent) => (1.0 - percent.clamp(97.0, 100.0) / 100.0) * duration,
            Threshold::MinutesBeforeEnd(minutes) => minutes.clamp(0.0, 3.5) * 60.0,
        }
    }
}

/// One skippable moment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
    pub kind: Kind,
    pub start: f64,
    pub end: f64,
    pub target: f64,
    /// A scene follows the credits ("Pular para pós-créditos").
    pub to_post_credits: bool,
    /// Skipped by itself, as chosen in the settings.
    pub auto: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Moments {
    pub segments: Vec<Moment>,
    /// `None`: only when playback ends.
    pub next_episode_at: Option<f64>,
}

fn following_scene(segments: &[Segment], credits: &Segment, duration: f64) -> Option<Segment> {
    segments
        .iter()
        .filter(|s| s.kind == Kind::PostCredits && s.start >= credits.end && s.start < duration)
        .min_by(|a, b| a.start.total_cmp(&b.start))
        .copied()
}

fn next_episode_at(segments: &[Segment], duration: f64, threshold: Threshold) -> Option<f64> {
    let window = threshold.seconds_before_end(duration);
    let user = (duration - window).max(0.0);
    let endings: Vec<&Segment> = segments.iter().filter(|s| s.kind == Kind::Outro).collect();
    let Some(latest) = endings.iter().max_by(|a, b| a.end.total_cmp(&b.end)) else { return Some(user) };
    if let Some(scene) = following_scene(segments, latest, duration) {
        return Some(scene.end.min(duration).max(user));
    }
    let after = duration - latest.end;
    if after > POST_CREDITS_GAP {
        // Probably a scene: the card never covers one that is shorter than the chosen window.
        return (after >= window).then_some(user);
    }
    if after > window {
        Some(user)
    } else {
        endings.iter().map(|s| s.start).reduce(f64::min)
    }
}

/// What the player offers, for a known duration. `auto`: the kinds the user skips automatically.
pub fn plan(segments: &[Segment], duration: f64, threshold: Threshold, auto: &[String]) -> Moments {
    if !duration.is_finite() || duration < MIN_CONTENT_SECONDS {
        return Moments::default();
    }
    let segments: Vec<Segment> = segments
        .iter()
        .filter(|s| s.start < duration)
        .map(|s| Segment { end: s.end.min(duration), ..*s })
        .collect();
    let moments = segments
        .iter()
        .filter(|s| s.kind != Kind::PostCredits)
        .map(|s| {
            let credits = matches!(s.kind, Kind::Outro | Kind::MovieCredits);
            let scene = if credits { following_scene(&segments, s, duration) } else { None };
            Moment {
                kind: s.kind,
                start: s.start,
                end: s.end,
                target: scene.map_or(s.end, |p| p.start),
                to_post_credits: credits && (scene.is_some() || duration - s.end > POST_CREDITS_GAP),
                auto: s.kind.setting().is_some_and(|name| auto.iter().any(|a| a == name)),
            }
        })
        .collect();
    Moments { segments: moments, next_episode_at: next_episode_at(&segments, duration, threshold) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(kind: Kind, start: f64, end: f64) -> Segment {
        Segment { kind, start, end }
    }

    #[test]
    fn service_answers() {
        let introdb = r#"{"intro":{"start_sec":4,"end_sec":92},"recap":null,"outro":{"start_ms":1344990,"end_ms":1434950},"post_credits":null}"#;
        assert_eq!(parse_introdb(introdb, false).unwrap(), vec![seg(Kind::Intro, 4.0, 92.0), seg(Kind::Outro, 1344.99, 1434.95)]);
        let movie = r#"{"outro":{"start_sec":7000,"end_sec":7300},"post_credits":{"start_sec":7200,"end_sec":null}}"#;
        assert_eq!(parse_introdb(movie, true).unwrap()[0], seg(Kind::MovieCredits, 7000.0, 7200.0));
        let aniskip = r#"{"found":true,"results":[{"interval":{"startTime":4,"endTime":96},"skipType":"op"},{"interval":{"startTime":1345,"endTime":1435},"skipType":"mixed-ed"},{"interval":{"startTime":1,"endTime":2},"skipType":"other"}]}"#;
        assert_eq!(parse_aniskip(aniskip).unwrap(), vec![seg(Kind::Intro, 4.0, 96.0), seg(Kind::Outro, 1345.0, 1435.0)]);
    }

    #[test]
    fn chapter_names() {
        let titled = |pairs: &[(f64, &str)]| pairs.iter().map(|&(time, t)| Chapter { time, title: t.into() }).collect::<Vec<_>>();
        let found = from_chapters(&titled(&[(0.0, "Recap"), (30.0, "Intro"), (40.0, "Opening Theme 2"), (130.0, "Part A"), (1300.0, " ED "), (1390.0, "Preview")]), 1420.0, false);
        assert_eq!(found, vec![seg(Kind::Recap, 0.0, 30.0), seg(Kind::Intro, 40.0, 130.0), seg(Kind::Outro, 1300.0, 1390.0)]);
        let plain = from_chapters(&titled(&[(0.0, "Intro"), (60.0, "Episode"), (900.0, "Edward"), (1000.0, "End Credits")]), 1100.0, false);
        assert_eq!(plain, vec![seg(Kind::Intro, 0.0, 60.0), seg(Kind::Outro, 1000.0, 1100.0)]);
        assert!(from_chapters(&titled(&[(0.0, "Opening")]), 90.0, false).is_empty());
    }

    #[test]
    fn chapters_win_per_category() {
        let merged = merge(
            vec![seg(Kind::Intro, 10.0, 100.0)],
            vec![vec![seg(Kind::Intro, 12.0, 99.0), seg(Kind::Outro, 1300.0, 1390.0)], vec![seg(Kind::Outro, 1.0, 2.0)]],
            false,
        );
        assert_eq!(merged, vec![seg(Kind::Intro, 10.0, 100.0), seg(Kind::Outro, 1300.0, 1390.0)]);
    }

    #[test]
    fn next_episode_and_skip_follow_the_credits() {
        let at = |segments: &[Segment]| plan(segments, 1440.0, Threshold::Percent(99.0), &[]).next_episode_at.map(f64::round);
        assert_eq!(at(&[]), Some(1426.0));
        assert_eq!(at(&[seg(Kind::Outro, 1347.0, 1437.0)]), Some(1347.0), "ending at the end: from its start");
        assert_eq!(at(&[seg(Kind::Outro, 1300.0, 1390.0)]), Some(1426.0), "a preview after it");
        assert_eq!(at(&[seg(Kind::Outro, 1340.0, 1430.0)]), None, "a short scene is never covered");
        let scene = [seg(Kind::Outro, 1300.0, 1380.0), seg(Kind::PostCredits, 1390.0, 1438.0)];
        assert_eq!(at(&scene), Some(1438.0));
        let moment = plan(&scene, 1440.0, Threshold::Percent(99.0), &["outro".into()]).segments[0];
        assert_eq!((moment.target, moment.to_post_credits, moment.auto), (1390.0, true, true));
        assert_eq!(plan(&scene, 100.0, Threshold::Percent(99.0), &[]), Moments::default(), "placeholder clip");
    }
}
