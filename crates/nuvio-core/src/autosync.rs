//! Automatic subtitle sync (ported from the original Nuvio `features/autosync`).
//!
//! Despite the name, the original does no audio analysis: it compares the timing of the selected
//! subtitle with one or more other subtitle timelines and finds a single, validated constant
//! delay. This module ports the keyless, player-independent part of that subsystem:
//! `AutoSyncSubtitleCueParser`, `AutoSyncTimelineRetimer` (delay-only path) and
//! `AutomaticDelaySyncEngine.computeAlignment`.
//!
//! Heavy parts are intentionally not ported yet: embedded MP4/MKV index parsing, affine FPS drift
//! retiming and the banded group DP. The result is applied by the desktop app as `sub-delay`.

use regex::Regex;

/// One timed subtitle line. `end_ms` is an estimate for formats without explicit ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// A confident constant shift, in milliseconds, with the evidence that qualified it.
#[derive(Debug, Clone, PartialEq)]
pub struct Alignment {
    pub offset_ms: i64,
    pub score: f64,
    pub margin: f64,
    pub segments_passed: usize,
    pub references: usize,
}

const MIN_CUES: usize = 4;
const COARSE_BIN_MS: i64 = 500;
const FINE_BIN_MS: i64 = 100;
const MAX_OFFSET_MS: i64 = 180_000;
const FINE_RADIUS_MS: i64 = 750;
const COARSE_PLATEAU_EPSILON: f64 = 0.01;
const COARSE_PLATEAU_MAX_MS: i64 = 3_000;
const FINE_TIE_EPSILON: f64 = 0.005;
const ONSET_TOLERANCE_MS: i64 = 250;
const MAX_CUE_DURATION_MS: i64 = 20_000;
const MAX_TIMELINE_MS: i64 = 8 * 60 * 60 * 1_000;
const MIN_SCALE: f64 = 0.94;
const MAX_SCALE: f64 = 1.06;

const DELAY_ONLY_MIN_SCORE: f64 = 0.78;
const DELAY_ONLY_MIN_MARGIN: f64 = 0.02;
const DELAY_ONLY_MIN_SEGMENT_SCORE: f64 = 0.68;
const DELAY_ONLY_SEGMENT_RADIUS_MS: i64 = 1_000;
const DELAY_ONLY_MAX_SEGMENT_DELTA_MS: i64 = 500;
const DELAY_ONLY_DISTINCT_OFFSET_MS: i64 = 3_000;
const SMALL_SAMPLE_CUE_LIMIT: usize = 8;

// ---------------------------------------------------------------- cue parsing

/// Cleans BOM/line endings and detects SRT, WebVTT, ASS/SSA or TTML from the URL/contents.
pub fn parse_cues(text: &str, source: &str) -> Vec<SyncCue> {
    let cleaned = text.replace('\u{feff}', "").replace("\r\n", "\n").replace('\r', "\n");
    let path = source.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    let head = cleaned.trim_start();
    if path.ends_with(".vtt") || path.ends_with(".webvtt") || head.starts_with("WEBVTT") {
        parse_vtt(&cleaned)
    } else if path.ends_with(".ass") || path.ends_with(".ssa") || has_ass_events(&cleaned) {
        parse_ass(&cleaned)
    } else {
        parse_srt(&cleaned)
    }
}

fn has_ass_events(text: &str) -> bool {
    text.lines().any(|line| line.trim().eq_ignore_ascii_case("[Events]"))
}

fn parse_timestamp(raw: &str) -> Option<i64> {
    let value = raw.trim();
    let (clock, fraction) = match value.split_once(['.', ',']) {
        Some((clock, fraction)) => (clock, fraction),
        None => (value, ""),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let mut seconds: i64 = 0;
    for part in parts {
        seconds = seconds.saturating_mul(60).saturating_add(part.parse::<i64>().ok()?);
    }
    let millis = if fraction.is_empty() {
        0
    } else if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    } else {
        let digits: String = fraction.chars().take(3).collect();
        let mut value = digits.parse::<i64>().ok()?;
        for _ in digits.len()..3 {
            value *= 10;
        }
        value
    };
    Some(seconds.saturating_mul(1_000).saturating_add(millis))
}

fn parse_start_end(timing: &str) -> Option<(i64, i64)> {
    let (start, end) = timing.split_once("-->")?;
    let start = parse_timestamp(start.trim().split_whitespace().next()?)?;
    let end = parse_timestamp(end.trim().split_whitespace().next()?)?;
    (end > start).then_some((start, end))
}

fn normalize_text(text: &str) -> String {
    let stamp = Regex::new(r#"<(?:\d+:)?\d{1,2}:\d{2}(?:[.,]\d+)?>"#).unwrap();
    let tag = Regex::new(r"</?[a-zA-Z0-9._-]+(?: [^>]*)?>").unwrap();
    let spaces = Regex::new(r"[ \t]+").unwrap();
    let text = stamp.replace_all(text, "");
    let text = tag.replace_all(&text, "");
    let text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"");
    text.lines()
        .map(|line| spaces.replace_all(line.trim(), " ").into_owned())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_srt(text: &str) -> Vec<SyncCue> {
    let blocks = Regex::new(r"\n\s*\n").unwrap();
    let mut cues = Vec::new();
    for block in blocks.split(text) {
        let lines: Vec<&str> = block.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let mut index = 0;
        if lines.first().is_some_and(|line| line.bytes().all(|b| b.is_ascii_digit())) && lines.len() > 1 {
            index += 1;
        }
        let Some(timing) = lines.get(index) else { continue };
        let Some((start_ms, end_ms)) = parse_start_end(timing) else { continue };
        let body = normalize_text(&lines[index + 1..].join("\n"));
        if !body.is_empty() {
            cues.push(SyncCue { start_ms, end_ms, text: body });
        }
    }
    cues
}

fn parse_vtt(text: &str) -> Vec<SyncCue> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut cues = Vec::new();
    let mut cursor = 0;
    while cursor < lines.len() {
        let line = lines[cursor].trim();
        if line.is_empty() || line.starts_with("WEBVTT") || line.starts_with("NOTE") || line.starts_with("STYLE") || line.starts_with("REGION") {
            cursor += 1;
            continue;
        }
        let (timing, body_start) = if line.contains("-->") {
            (line, cursor + 1)
        } else {
            (lines.get(cursor + 1).copied().unwrap_or_default().trim(), cursor + 2)
        };
        let Some((start_ms, end_ms)) = parse_start_end(timing) else {
            cursor += 1;
            continue;
        };
        let mut parts = Vec::new();
        let mut i = body_start;
        while i < lines.len() && !lines[i].trim().is_empty() {
            parts.push(lines[i].trim());
            i += 1;
        }
        let body = normalize_text(&parts.join("\n"));
        if !body.is_empty() {
            cues.push(SyncCue { start_ms, end_ms, text: body });
        }
        cursor = i.max(cursor + 1);
    }
    cues
}

fn parse_ass(text: &str) -> Vec<SyncCue> {
    let mut format: Vec<String> = Vec::new();
    let mut cues = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("Format:") {
            format = line[7..].split(',').map(|field| field.trim().to_ascii_lowercase()).collect();
        } else if line.starts_with("Dialogue:") && !format.is_empty() {
            let fields: Vec<&str> = line[9..].splitn(format.len(), ',').collect();
            let field = |name: &str| -> Option<&str> {
                format.iter().position(|f| f == name).and_then(|i| fields.get(i)).copied()
            };
            let (Some(start), Some(end)) = (field("start"), field("end")) else { continue };
            let (Some(start_ms), Some(end_ms)) = (parse_timestamp(start), parse_timestamp(end)) else { continue };
            if end_ms <= start_ms {
                continue;
            }
            let body = field("text").unwrap_or_default().replace("\\N", "\n").replace("\\n", "\n");
            let body = Regex::new(r"\{[^}]*\}").unwrap().replace_all(&body, "");
            let body = normalize_text(&body);
            if !body.is_empty() {
                cues.push(SyncCue { start_ms, end_ms, text: body });
            }
        }
    }
    cues
}

// ------------------------------------------------------------------ alignment

#[derive(Debug, Clone)]
struct ActivityTimeline {
    prefix: Vec<i64>,
    packed: Vec<u64>,
    active_count: usize,
    first_active: usize,
    last_active: usize,
}

#[derive(Debug, Clone)]
struct PreparedActivity {
    coarse: ActivityTimeline,
    fine: ActivityTimeline,
    starts: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    scale: f64,
    intercept_ms: i64,
    score: f64,
}

#[derive(Debug, Clone)]
struct Seed {
    coarse: Candidate,
    margin: f64,
    fine: Option<Candidate>,
    plateau_start_ms: i64,
    plateau_end_ms: i64,
}

/// Sorts by start and drops exact duplicates, like `normalizeExternalTimeline`.
fn normalize_timeline(cues: &[SyncCue]) -> Vec<SyncCue> {
    if cues.len() < 2 {
        return cues.to_vec();
    }
    let mut sorted = cues.to_vec();
    sorted.sort_by_key(|cue| cue.start_ms);
    let mut seen: std::collections::HashSet<(i64, i64, String)> = std::collections::HashSet::new();
    sorted.retain(|cue| seen.insert((cue.start_ms, cue.end_ms, cue.text.clone())));
    sorted
}

fn build_activity(cues: &[SyncCue], scale: f64, bin_ms: i64) -> Option<ActivityTimeline> {
    if !scale.is_finite() || !(MIN_SCALE..=MAX_SCALE).contains(&scale) || cues.is_empty() || bin_ms <= 0 {
        return None;
    }
    let mut scaled = Vec::with_capacity(cues.len());
    let mut max_end_ms = 0i64;
    for cue in cues {
        let safe_start = cue.start_ms.max(0);
        let safe_end = (cue.end_ms).clamp(safe_start + 1, safe_start + MAX_CUE_DURATION_MS);
        let start = ((safe_start as f64 * scale).round() as i64).max(0);
        let end = ((safe_end as f64 * scale).round() as i64).max(start + 1);
        if end > MAX_TIMELINE_MS {
            return None;
        }
        scaled.push((start, end));
        max_end_ms = max_end_ms.max(end);
    }
    let bin_count = (((max_end_ms + 1) as f64 / bin_ms as f64).ceil() as usize).max(1);
    let mut bins = vec![false; bin_count];
    for (start, end) in scaled {
        let first = (start / bin_ms) as usize;
        let first = first.min(bin_count - 1);
        let last = ((end as f64 / bin_ms as f64).ceil() as usize).clamp(first + 1, bin_count);
        for bin in &mut bins[first..last] {
            *bin = true;
        }
    }
    let mut active = Vec::new();
    let mut prefix = vec![0i64; bin_count + 1];
    let mut packed = vec![0u64; (bin_count + 63) / 64];
    for (index, is_active) in bins.iter().enumerate() {
        if *is_active {
            active.push(index);
            packed[index / 64] |= 1u64 << (index % 64);
        }
        prefix[index + 1] = prefix[index] + i64::from(*is_active);
    }
    if active.is_empty() {
        return None;
    }
    Some(ActivityTimeline {
        prefix,
        packed,
        active_count: active.len(),
        first_active: *active.first().unwrap(),
        last_active: *active.last().unwrap(),
    })
}

fn prepare_activity(cues: &[SyncCue]) -> Option<PreparedActivity> {
    if cues.len() < MIN_CUES {
        return None;
    }
    let coarse = build_activity(cues, 1.0, COARSE_BIN_MS)?;
    let fine = build_activity(cues, 1.0, FINE_BIN_MS)?;
    let mut starts: Vec<i64> = cues.iter().map(|cue| cue.start_ms.max(0)).collect();
    starts.sort_unstable();
    Some(PreparedActivity { coarse, fine, starts })
}

fn packed_window(packed: &[u64], start_bit: i64) -> u64 {
    let word = start_bit.div_euclid(64);
    let offset = start_bit.rem_euclid(64) as u32;
    let low = usize::try_from(word).ok().and_then(|i| packed.get(i).copied()).unwrap_or(0);
    if offset == 0 {
        return low;
    }
    let high = usize::try_from(word + 1).ok().and_then(|i| packed.get(i).copied()).unwrap_or(0);
    (low >> offset) | (high << (64 - offset))
}

fn range_mask(first_bit: i64, last_bit: i64) -> u64 {
    let lower = u64::MAX << first_bit;
    let upper = if last_bit == 63 { u64::MAX } else { (1u64 << (last_bit + 1)) - 1 };
    lower & upper
}

fn shifted_intersection(reference: &ActivityTimeline, target: &ActivityTimeline, offset_bins: i64, source_start: usize, source_end: usize) -> usize {
    if source_end < source_start {
        return 0;
    }
    let first_word = (source_start as i64).div_euclid(64);
    let last_word = (source_end as i64).div_euclid(64);
    let mut intersection = 0usize;
    for word in first_word..=last_word {
        let aligned = packed_window(&reference.packed, word * 64 + offset_bins);
        let first_bit = if word == first_word { source_start as i64 - word * 64 } else { 0 };
        let last_bit = if word == last_word { source_end as i64 - word * 64 } else { 63 };
        let mask = range_mask(first_bit, last_bit);
        let own = usize::try_from(word).ok().and_then(|i| target.packed.get(i).copied()).unwrap_or(0);
        intersection += (own & aligned & mask).count_ones() as usize;
    }
    intersection
}

fn prefix_count(timeline: &ActivityTimeline, from: usize, to_inclusive: usize) -> Option<i64> {
    if to_inclusive < from || to_inclusive + 1 >= timeline.prefix.len() {
        return None;
    }
    Some(timeline.prefix[to_inclusive + 1] - timeline.prefix[from])
}

fn score_offset(reference: &ActivityTimeline, target: &ActivityTimeline, offset_bins: i64) -> Option<f64> {
    let target_last = target.prefix.len() as i64 - 2;
    let reference_last = reference.prefix.len() as i64 - 2;
    let source_start = (-offset_bins).max(0);
    let source_end = target_last.min(reference_last - offset_bins);
    if source_end < source_start {
        return None;
    }
    let (source_start, source_end) = (source_start as usize, source_end as usize);
    let visible = prefix_count(target, source_start, source_end)?;
    if visible <= 0 {
        return None;
    }
    let intersection = shifted_intersection(reference, target, offset_bins, source_start, source_end) as f64;
    let reference_start = (target.first_active as i64 + offset_bins).max(0);
    let reference_end = reference_last.min(target.last_active as i64 + offset_bins);
    if reference_end < reference_start {
        return None;
    }
    let in_window = prefix_count(reference, reference_start as usize, reference_end as usize)?;
    if in_window <= 0 {
        return None;
    }
    let precision = intersection / visible as f64;
    let recall = intersection / in_window as f64;
    let visibility = (visible as f64 / target.active_count as f64).clamp(0.0, 1.0);
    Some((precision * 0.72 + recall * 0.28) * (0.85 + 0.15 * visibility))
}

fn coverage_at_segment(reference: &ActivityTimeline, target: &ActivityTimeline, offset_bins: i64, segment: usize) -> Option<f64> {
    if segment > 2 {
        return None;
    }
    let span = target.last_active as i64 - target.first_active as i64 + 1;
    if span <= 0 {
        return None;
    }
    let start = target.first_active as i64 + span * segment as i64 / 3;
    let end = if segment == 2 {
        target.last_active as i64
    } else {
        target.first_active as i64 + span * (segment as i64 + 1) / 3 - 1
    };
    if end < start {
        return None;
    }
    let visible = prefix_count(target, start as usize, end as usize)?;
    if visible <= 0 {
        return None;
    }
    Some(shifted_intersection(reference, target, offset_bins, start as usize, end as usize) as f64 / visible as f64)
}

fn score_at_segment(reference: &ActivityTimeline, target: &ActivityTimeline, offset_bins: i64, segment: usize) -> Option<f64> {
    if segment > 2 {
        return None;
    }
    let span = target.last_active as i64 - target.first_active as i64 + 1;
    if span <= 0 {
        return None;
    }
    let start = target.first_active as i64 + span * segment as i64 / 3;
    let end = if segment == 2 {
        target.last_active as i64
    } else {
        target.first_active as i64 + span * (segment as i64 + 1) / 3 - 1
    };
    if end < start {
        return None;
    }
    let visible = prefix_count(target, start as usize, end as usize)?;
    if visible <= 0 {
        return None;
    }
    let intersection = shifted_intersection(reference, target, offset_bins, start as usize, end as usize) as f64;
    let reference_start = (start + offset_bins).max(0);
    let reference_end = (reference.prefix.len() as i64 - 2).min(end + offset_bins);
    if reference_end < reference_start {
        return None;
    }
    let in_window = prefix_count(reference, reference_start as usize, reference_end as usize)?;
    if in_window <= 0 {
        return None;
    }
    let precision = intersection / visible as f64;
    let recall = intersection / in_window as f64;
    Some(precision * 0.72 + recall * 0.28)
}

fn coarse_offset_range(reference: &ActivityTimeline, target: &ActivityTimeline) -> std::ops::RangeInclusive<i64> {
    let base = MAX_OFFSET_MS / COARSE_BIN_MS;
    let reference_span = (reference.last_active - reference.first_active) as i64;
    let target_span = (target.last_active - target.first_active) as i64;
    if target_span * 4 >= reference_span * 3 {
        return -base..=base;
    }
    (reference.first_active as i64 - target.first_active as i64 - base).min(-base)
        ..=(reference.last_active as i64 - target.last_active as i64 + base).max(base)
}

fn plateau_ms(candidates: &[Candidate], best: &Candidate) -> (i64, i64) {
    let Some(best_index) = candidates.iter().position(|candidate| candidate.intercept_ms == best.intercept_ms) else {
        return (best.intercept_ms, best.intercept_ms);
    };
    let floor = best.score - COARSE_PLATEAU_EPSILON;
    let extent = |step: i64| {
        let mut edge = best.intercept_ms;
        let mut index = best_index as i64 + step;
        while index >= 0 && (index as usize) < candidates.len() {
            let candidate = candidates[index as usize];
            if candidate.intercept_ms != edge + step * COARSE_BIN_MS
                || candidate.score < floor
                || (candidate.intercept_ms - best.intercept_ms).abs() > COARSE_PLATEAU_MAX_MS
            {
                break;
            }
            edge = candidate.intercept_ms;
            index += step;
        }
        edge
    };
    (extent(-1), extent(1))
}

fn onset_agreement(reference_starts: &[i64], target_starts: &[i64], intercept_ms: i64) -> f64 {
    if reference_starts.is_empty() {
        return 0.0;
    }
    let mut agreement = 0.0;
    let mut cursor = 0usize;
    for &start in target_starts {
        let mapped = start + intercept_ms;
        while cursor < reference_starts.len() && reference_starts[cursor] < mapped - ONSET_TOLERANCE_MS {
            cursor += 1;
        }
        if cursor == reference_starts.len() {
            break;
        }
        let mut nearest = i64::MAX;
        let mut index = cursor;
        while index < reference_starts.len() && reference_starts[index] <= mapped + ONSET_TOLERANCE_MS {
            nearest = nearest.min((reference_starts[index] - mapped).abs());
            index += 1;
        }
        if nearest <= ONSET_TOLERANCE_MS {
            agreement += (ONSET_TOLERANCE_MS - nearest) as f64 / ONSET_TOLERANCE_MS as f64;
        }
    }
    agreement
}

fn fine_best(
    reference: &ActivityTimeline,
    target: &ActivityTimeline,
    reference_starts: &[i64],
    target_starts: &[i64],
    plateau_start_ms: i64,
    plateau_end_ms: i64,
) -> Option<Candidate> {
    let mut candidates = Vec::new();
    let mut best: Option<Candidate> = None;
    let mut offset_ms = plateau_start_ms - FINE_RADIUS_MS;
    while offset_ms <= plateau_end_ms + FINE_RADIUS_MS {
        let offset_bins = (offset_ms as f64 / FINE_BIN_MS as f64).round() as i64;
        if let Some(score) = score_offset(reference, target, offset_bins) {
            let candidate = Candidate { scale: 1.0, intercept_ms: offset_bins * FINE_BIN_MS, score };
            candidates.push(candidate);
            if best.is_none_or(|current| candidate.score > current.score) {
                best = Some(candidate);
            }
        }
        offset_ms += FINE_BIN_MS;
    }
    let best = best?;
    let mut chosen = best;
    let mut chosen_onsets = -1.0;
    for candidate in candidates {
        if candidate.score < best.score - FINE_TIE_EPSILON {
            continue;
        }
        let onsets = onset_agreement(reference_starts, target_starts, candidate.intercept_ms);
        if onsets > chosen_onsets || (onsets == chosen_onsets && candidate.score > chosen.score) {
            chosen = candidate;
            chosen_onsets = onsets;
        }
    }
    Some(chosen)
}

fn build_seed(candidates: &[Candidate]) -> Option<Seed> {
    let coarse = *candidates.iter().max_by(|a, b| a.score.total_cmp(&b.score))?;
    let second = candidates
        .iter()
        .filter(|candidate| (candidate.intercept_ms - coarse.intercept_ms).abs() >= DELAY_ONLY_DISTINCT_OFFSET_MS)
        .max_by(|a, b| a.score.total_cmp(&b.score));
    let plateau = plateau_ms(candidates, &coarse);
    Some(Seed {
        coarse,
        margin: coarse.score - second.map(|candidate| candidate.score).unwrap_or(0.0),
        fine: None,
        plateau_start_ms: plateau.0,
        plateau_end_ms: plateau.1,
    })
}

fn find_alignment(reference: &PreparedActivity, target: &PreparedActivity, target_size: usize, allow_ambiguous_margin: bool) -> Option<Alignment> {
    let coarse_range = coarse_offset_range(&reference.coarse, &target.coarse);
    let mut candidates = Vec::new();
    for offset_bins in coarse_range {
        if let Some(score) = score_offset(&reference.coarse, &target.coarse, offset_bins) {
            candidates.push(Candidate { scale: 1.0, intercept_ms: offset_bins * COARSE_BIN_MS, score });
        }
    }
    let mut seed = build_seed(&candidates)?;
    seed.fine = fine_best(
        &reference.fine,
        &target.fine,
        &reference.starts,
        &target.starts,
        seed.plateau_start_ms,
        seed.plateau_end_ms,
    );

    let margin_accepted = allow_ambiguous_margin || seed.margin >= DELAY_ONLY_MIN_MARGIN;
    if !margin_accepted {
        return None;
    }
    let best = seed.fine.unwrap_or(seed.coarse);
    if best.score < DELAY_ONLY_MIN_SCORE {
        return None;
    }
    let global_bins = (best.intercept_ms as f64 / FINE_BIN_MS as f64).round() as i64;
    let radius = DELAY_ONLY_SEGMENT_RADIUS_MS / FINE_BIN_MS;
    let max_delta = DELAY_ONLY_MAX_SEGMENT_DELTA_MS / FINE_BIN_MS;
    let mut available = 0usize;
    let mut passed = 0usize;
    for segment in 0..3 {
        if allow_ambiguous_margin {
            let Some(coverage) = coverage_at_segment(&reference.fine, &target.fine, global_bins, segment) else { continue };
            available += 1;
            if coverage >= DELAY_ONLY_MIN_SEGMENT_SCORE {
                passed += 1;
            }
            continue;
        }
        let mut segment_best = f64::NEG_INFINITY;
        let mut segment_near = f64::NEG_INFINITY;
        let mut has_score = false;
        for delta in -radius..=radius {
            let Some(score) = score_at_segment(&reference.fine, &target.fine, global_bins + delta, segment) else { continue };
            has_score = true;
            segment_best = segment_best.max(score);
            if delta.abs() <= max_delta {
                segment_near = segment_near.max(score);
            }
        }
        if !has_score {
            continue;
        }
        available += 1;
        if segment_best >= DELAY_ONLY_MIN_SEGMENT_SCORE && segment_near >= segment_best - FINE_TIE_EPSILON {
            passed += 1;
        }
    }
    let required = if target_size < SMALL_SAMPLE_CUE_LIMIT { 2 } else { 3 };
    if available < required || passed < required {
        return None;
    }
    Some(Alignment {
        offset_ms: best.intercept_ms,
        score: best.score,
        margin: seed.margin,
        segments_passed: passed,
        references: 1,
    })
}

/// Ports `AutomaticDelaySyncEngine.computeAlignment`: the best confident constant offset among
/// the supplied reference timelines. `None` means "no confident match" — never guess.
pub fn compute_alignment(target_cues: &[SyncCue], references: &[Vec<SyncCue>]) -> Option<Alignment> {
    if references.is_empty() {
        return None;
    }
    let target = normalize_timeline(target_cues);
    let target_activity = prepare_activity(&target)?;
    let mut best: Option<Alignment> = None;
    for reference_cues in references {
        let cues = normalize_timeline(reference_cues);
        if cues.len() < MIN_CUES {
            continue;
        }
        let Some(reference_activity) = prepare_activity(&cues) else { continue };
        // SDH/over-segmented references legitimately have ambiguous margins.
        let allow_ambiguous = cues.len() * 2 >= target.len() * 3;
        let Some(mut alignment) = find_alignment(&reference_activity, &target_activity, target.len(), allow_ambiguous) else { continue };
        alignment.references = references.len();
        if best.as_ref().is_none_or(|current| alignment.score > current.score) {
            best = Some(alignment);
        }
    }
    best
}

/// Phase 2 result: either the validated constant offset, or an affine/group retiming that needs a
/// rewritten subtitle file. `offset_ms` is the equivalent shift at the middle of the timeline and
/// can always be applied with `sub-delay` when a file rewrite is not possible.
#[derive(Debug, Clone)]
pub struct AlignmentV2 {
    pub offset_ms: i64,
    pub scale: f64,
    pub confident: bool,
    pub coverage: f64,
    pub cues: Vec<crate::autosync_dp::RetimedCue>,
}

/// Tries the cheap delay-only path first, then FPS-drift affine alignment and the banded group DP.
pub fn compute_alignment_v2(target_cues: &[SyncCue], references: &[Vec<SyncCue>]) -> Option<AlignmentV2> {
    if let Some(fast) = compute_alignment(target_cues, references) {
        let cues = target_cues.iter().map(|cue| crate::autosync_dp::RetimedCue {
            original_start_ms: cue.start_ms,
            original_end_ms: cue.end_ms,
            start_ms: cue.start_ms + fast.offset_ms,
            end_ms: cue.end_ms + fast.offset_ms,
        }).collect();
        return Some(AlignmentV2 { offset_ms: fast.offset_ms, scale: 1.0, confident: true, coverage: 1.0, cues });
    }
    let target = normalize_timeline(target_cues);
    if target.len() < MIN_CUES {
        return None;
    }
    let middle = (target.first().map(|cue| cue.start_ms).unwrap_or(0) + target.last().map(|cue| cue.start_ms).unwrap_or(0)) / 2;
    for reference_cues in references {
        let reference = normalize_timeline(reference_cues);
        if reference.len() < MIN_CUES {
            continue;
        }
        let Some(affine) = crate::autosync_dp::find_affine_alignment(&target, &reference) else { continue };
        let retimed = crate::autosync_dp::retime_grouped(&target, &reference, &affine);
        if !retimed.confident {
            continue;
        }
        let offset = affine.intercept_ms + ((affine.scale - 1.0) * middle as f64).round() as i64;
        return Some(AlignmentV2 {
            offset_ms: offset,
            scale: affine.scale,
            confident: true,
            coverage: retimed.target_coverage,
            cues: retimed.cues,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cues(start: i64, step: i64, count: usize) -> Vec<SyncCue> {
        (0..count)
            .map(|index| {
                let at = start + index as i64 * step;
                SyncCue { start_ms: at, end_ms: at + 2_000, text: format!("line {index}") }
            })
            .collect()
    }

    #[test]
    fn parses_srt_and_webvtt_timings() {
        let srt = "\u{feff}1\r\n00:00:01,250 --> 00:00:03,500\r\n<i>Hello</i>\r\n\r\n2\r\n00:00:04,000 --> 00:00:06,000\r\nWorld\r\n";
        let parsed = parse_cues(srt, "test.srt");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].start_ms, 1_250);
        assert_eq!(parsed[0].end_ms, 3_500);
        assert_eq!(parsed[0].text, "Hello");
        let vtt = "WEBVTT\n\n00:01.000 --> 00:03.000\nHi there\n\n";
        let parsed = parse_cues(vtt, "test.vtt");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].start_ms, 1_000);
    }

    #[test]
    fn finds_a_constant_offset() {
        // Irregular spacing: a purely periodic cue list is ambiguous by nature (several offsets
        // fit equally well) and is correctly rejected by the margin gate.
        let starts = [1_000, 4_200, 9_100, 14_500, 20_300, 25_800, 32_100, 37_200, 44_000, 49_500, 56_700, 63_000];
        let target: Vec<SyncCue> = starts
            .iter()
            .enumerate()
            .map(|(index, at)| SyncCue { start_ms: *at, end_ms: at + 2_000 + (index % 3) as i64 * 700, text: format!("line {index}") })
            .collect();
        let reference: Vec<SyncCue> = target
            .iter()
            .map(|cue| SyncCue { start_ms: cue.start_ms + 2_000, end_ms: cue.end_ms + 2_000, text: cue.text.clone() })
            .collect();
        let alignment = compute_alignment(&target, &[reference]).expect("expected a confident match");
        assert!((alignment.offset_ms - 2_000).abs() <= 100, "offset was {}", alignment.offset_ms);
        assert!(alignment.score >= DELAY_ONLY_MIN_SCORE);
    }

    #[test]
    fn rejects_a_drifted_timeline() {
        let target = cues(1_000, 4_000, 16);
        let reference: Vec<SyncCue> = target
            .iter()
            .map(|cue| SyncCue {
                start_ms: (cue.start_ms as f64 * 1.06) as i64 + 2_000,
                end_ms: (cue.end_ms as f64 * 1.06) as i64 + 2_000,
                text: cue.text.clone(),
            })
            .collect();
        assert!(compute_alignment(&target, &[reference]).is_none());
    }

    #[test]
    fn phase_two_finds_fps_drift() {
        let target: Vec<SyncCue> = (0..16)
            .map(|index| {
                let at = 1_000 + index * 4_000;
                SyncCue { start_ms: at, end_ms: at + 2_000, text: format!("l{index}") }
            })
            .collect();
        let reference: Vec<SyncCue> = target
            .iter()
            .map(|cue| SyncCue {
                start_ms: (cue.start_ms as f64 * 1.04) as i64 + 2_000,
                end_ms: (cue.end_ms as f64 * 1.04) as i64 + 2_000,
                text: cue.text.clone(),
            })
            .collect();
        let result = compute_alignment_v2(&target, &[reference]).expect("affine alignment");
        assert!(result.confident);
        assert!((result.scale - 1.04).abs() < 0.005, "scale {}", result.scale);
    }

    #[test]
    fn small_sample_is_supported() {
        let target = cues(5_000, 6_000, 4);
        let reference = cues(7_500, 6_000, 4);
        let alignment = compute_alignment(&target, &[reference]).expect("small sample should still match");
        assert!((alignment.offset_ms - 2_500).abs() <= 100);
    }
}
