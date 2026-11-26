//! PHASE 2 core of automatic subtitle sync: global affine (FPS) discovery and the banded
//! group DP that locally transplants reference starts onto the external timeline.
//!
//! This is a self-contained port of the player-independent part of the Kotlin original
//! `AutoSyncTimelineRetime.kt`:
//!
//! * [`find_affine_alignment`] ports `discoverActivityAlignment`,
//!   `activityScaleCandidates`, `coarsePlateauMs`, `fineActivityBest`, `onsetAgreement`
//!   and the shared activity-correlation helpers.
//! * [`retime_grouped`] ports `retimeWithSeed` (the banded DP, group shapes, local shift
//!   outlier rejection, timing transplant and final confidence gates).
//!
//! It deliberately does **not** depend on any of the private helpers in [`crate::autosync`]:
//! the activity timeline code is reimplemented here so the module compiles on its own once
//! `pub mod autosync_dp;` is added to `lib.rs`.

use crate::autosync::SyncCue;

// ---------------------------------------------------------------- activity constants

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
const SCALE_DEDUP: f64 = 0.00035;
const PARTIAL_TARGET_SPAN_RATIO: f64 = 0.75;
const DISTINCT_TRANSFORM_MS: f64 = 3_000.0;

// Confidence gates applied to the discovered affine transform. The Kotlin original applies
// these in `finalizeDiscoveredResult`; they live here so discovery itself never returns a
// weak or ambiguous transform to callers.
const ACTIVITY_MIN_SCORE: f64 = 0.55;
const ACTIVITY_MIN_MARGIN: f64 = 0.02;
const SMALL_SAMPLE_CUE_LIMIT: usize = 8;
const SMALL_SAMPLE_ACTIVITY_MIN_SCORE: f64 = 0.72;
const SMALL_SAMPLE_ACTIVITY_MIN_MARGIN: f64 = 0.03;

// -------------------------------------------------------------------- DP constants

const BAND_RADIUS_CUES: usize = 28;
const SKIP_REFERENCE_COST: f64 = 1.35;
const SKIP_TARGET_COST: f64 = 1.85;
const GROUP_COMPLEXITY_COST: f64 = 0.10;
const START_END_TOLERANCE_MS: f64 = 1_250.0;
const MIDPOINT_TOLERANCE_MS: f64 = 1_500.0;
const DURATION_TOLERANCE_MS: f64 = 2_000.0;
const MAX_GROUP_COST: f64 = 8.0;

const MIN_TARGET_COVERAGE: f64 = 0.84;
const MIN_MATCHED_TARGET_CUES: usize = 20;
const MAX_AVERAGE_GROUP_COST: f64 = 2.35;
const MAX_LONGEST_TARGET_SKIP_RUN: usize = 12;

const LOCAL_SHIFT_RADIUS_GROUPS: usize = 3;
const LOCAL_SHIFT_MAX_DEVIATION_MS: i64 = 250;

const GROUP_SHAPES: [(usize, usize); 6] = [(1, 1), (1, 2), (2, 1), (1, 3), (3, 1), (2, 2)];

const STEP_UNREACHABLE: u8 = 0;
const STEP_START: u8 = 1;
const STEP_GROUP: u8 = 2;
const STEP_SKIP_REFERENCE: u8 = 3;
const STEP_SKIP_TARGET: u8 = 4;

// ---------------------------------------------------------------------- public types

/// A globally discovered affine transform that maps **target** timestamps onto the
/// **reference** timeline (`mapped = time_ms * scale + intercept_ms`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AffineAlignment {
    pub scale: f64,
    pub intercept_ms: i64,
    /// Fine-bin correlation score after the visibility factor.
    pub score: f64,
    /// Difference from the best transform that is meaningfully distinct in time.
    pub margin: f64,
}

/// Result of the grouped retime. `cues` is in the same (start-sorted) order as the target.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupRetime {
    pub cues: Vec<RetimedCue>,
    /// Fraction of target cues consumed by a matched group.
    pub target_coverage: f64,
    /// Mean per-group alignment cost (before the complexity term).
    pub average_group_cost: f64,
    /// All structural gates of the original passed.
    pub confident: bool,
    /// Number of matched groups in the chosen DP path.
    pub groups: usize,
}

/// One retimed target cue. Original times are kept so callers can map back to the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetimedCue {
    pub original_start_ms: i64,
    pub original_end_ms: i64,
    pub start_ms: i64,
    pub end_ms: i64,
}

// ------------------------------------------------------------------- internal types

#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    scale: f64,
    intercept_ms: i64,
    score: f64,
}

#[derive(Debug, Clone)]
struct ActivityTimeline {
    /// Number of activity bins (`bins.lastIndex` in Kotlin).
    bins_len: usize,
    packed: Vec<u64>,
    first_active: usize,
    last_active: usize,
    active_count: usize,
    prefix: Vec<i64>,
}

#[derive(Debug, Clone)]
struct PreparedActivity {
    coarse: ActivityTimeline,
    fine: ActivityTimeline,
    /// Sorted cue starts, used only for onset tie-breaking.
    starts: Vec<i64>,
}

// ------------------------------------------------------------- activity preparation

/// Sorts by start and drops exact duplicates (`normalizeExternalTimeline`).
fn normalize_timeline(cues: &[SyncCue]) -> Vec<SyncCue> {
    if cues.len() < 2 {
        return cues.to_vec();
    }
    let mut sorted = cues.to_vec();
    sorted.sort_by_key(|cue| cue.start_ms);
    let mut seen = std::collections::HashSet::new();
    sorted.retain(|cue| seen.insert((cue.start_ms, cue.end_ms, cue.text.clone())));
    sorted
}

/// Port of `buildActivityTimeline` (Kotlin lines 1349-1407). Note the prefix array is
/// indexed directly in `score_offset` rather than through the Rust port's `prefix_count`
/// helper, preserving Kotlin's treatment of the final bin.
fn build_activity(cues: &[SyncCue], scale: f64, bin_ms: i64) -> Option<ActivityTimeline> {
    if !scale.is_finite() || !(MIN_SCALE..=MAX_SCALE).contains(&scale) || cues.is_empty() || bin_ms <= 0 {
        return None;
    }
    let mut scaled = Vec::with_capacity(cues.len());
    let mut max_end_ms = 0i64;
    for cue in cues {
        let safe_start = cue.start_ms.max(0);
        let safe_end = cue.end_ms.clamp(safe_start + 1, safe_start + MAX_CUE_DURATION_MS);
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
        let first = ((start / bin_ms) as usize).min(bin_count - 1);
        let last = (end as f64 / bin_ms as f64).ceil() as usize;
        let last = last.clamp(first + 1, bin_count);
        for bin in &mut bins[first..last] {
            *bin = true;
        }
    }

    let mut active = Vec::new();
    let mut prefix = vec![0i64; bin_count + 1];
    let mut packed = vec![0u64; (bin_count + 63) >> 6];
    for (index, is_active) in bins.iter().enumerate() {
        if *is_active {
            active.push(index);
            packed[index >> 6] |= 1u64 << (index & 63);
        }
        prefix[index + 1] = prefix[index] + i64::from(*is_active);
    }
    if active.is_empty() {
        return None;
    }
    Some(ActivityTimeline {
        bins_len: bin_count,
        packed,
        first_active: active[0],
        last_active: *active.last().unwrap(),
        active_count: active.len(),
        prefix,
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
    let word = start_bit >> 6;
    let offset = (start_bit & 63) as u32;
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

fn shifted_intersection(
    reference: &ActivityTimeline,
    target: &ActivityTimeline,
    offset_bins: i64,
    source_start: usize,
    source_end: usize,
) -> usize {
    if source_end < source_start {
        return 0;
    }
    let first_word = (source_start as i64) >> 6;
    let last_word = (source_end as i64) >> 6;
    let mut intersection = 0usize;
    for word in first_word..=last_word {
        let aligned = packed_window(&reference.packed, (word << 6) + offset_bins);
        let first_bit = if word == first_word { source_start as i64 - word * 64 } else { 0 };
        let last_bit = if word == last_word { source_end as i64 - word * 64 } else { 63 };
        let mask = range_mask(first_bit, last_bit);
        let own = usize::try_from(word).ok().and_then(|i| target.packed.get(i).copied()).unwrap_or(0);
        intersection += (own & aligned & mask).count_ones() as usize;
    }
    intersection
}

/// Port of `scoreActivityOffset`: `precision * 0.72 + recall * 0.28` with the visibility
/// factor `(0.85 + 0.15 * visibility)`.
fn score_offset(reference: &ActivityTimeline, target: &ActivityTimeline, offset_bins: i64) -> Option<f64> {
    let reference_last = reference.bins_len as i64 - 1;
    let target_last = target.bins_len as i64 - 1;
    let source_start = (-offset_bins).max(0);
    let source_end = target_last.min(reference_last - offset_bins);
    if source_end < source_start {
        return None;
    }
    let visible = target.prefix[source_end as usize + 1] - target.prefix[source_start as usize];
    if visible <= 0 {
        return None;
    }
    let intersection =
        shifted_intersection(reference, target, offset_bins, source_start as usize, source_end as usize);

    let reference_start = (target.first_active as i64 + offset_bins).max(0);
    let reference_end = reference_last.min(target.last_active as i64 + offset_bins);
    if reference_end < reference_start {
        return None;
    }
    let in_window = reference.prefix[reference_end as usize + 1] - reference.prefix[reference_start as usize];
    if in_window <= 0 {
        return None;
    }
    let precision = intersection as f64 / visible as f64;
    let recall = intersection as f64 / in_window as f64;
    let visibility = (visible as f64 / target.active_count as f64).clamp(0.0, 1.0);
    Some((precision * 0.72 + recall * 0.28) * (0.85 + 0.15 * visibility))
}

/// Port of the activity `coarseOffsetRange` (Kotlin lines 1527-1538).
fn coarse_offset_range(reference: &ActivityTimeline, target: &ActivityTimeline) -> std::ops::RangeInclusive<i64> {
    let base = MAX_OFFSET_MS / COARSE_BIN_MS;
    let reference_span = (reference.last_active - reference.first_active) as i64;
    let target_span = (target.last_active - target.first_active) as i64;
    if target_span as f64 >= reference_span as f64 * PARTIAL_TARGET_SPAN_RATIO {
        return -base..=base;
    }
    let start = (-base).min(reference.first_active as i64 - target.first_active as i64 - base);
    let end = base.max(reference.last_active as i64 - target.last_active as i64 + base);
    start..=end
}

/// Contiguous coarse offsets around `best_index` (same scale) within the plateau epsilon,
/// capped on either side (`coarsePlateauMs`).
fn coarse_plateau_ms(candidates: &[Candidate], best_index: usize) -> (i64, i64) {
    let best = candidates[best_index];
    let floor = best.score - COARSE_PLATEAU_EPSILON;
    let extent = |step: i64| {
        let mut edge = best.intercept_ms;
        let mut index = best_index as i64 + step;
        while index >= 0 && (index as usize) < candidates.len() {
            let candidate = candidates[index as usize];
            if candidate.scale != best.scale
                || candidate.intercept_ms != edge + step * COARSE_BIN_MS
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

/// How well transformed target starts land on reference starts (linearly falling to zero at
/// `ONSET_TOLERANCE_MS`). Port of `onsetAgreement`.
fn onset_agreement(reference_starts: &[i64], target_starts: &[i64], scale: f64, intercept_ms: i64) -> f64 {
    if reference_starts.is_empty() {
        return 0.0;
    }
    let tolerance = ONSET_TOLERANCE_MS;
    let mut agreement = 0.0;
    let mut cursor = 0usize;
    for &start in target_starts {
        let mapped = (start as f64 * scale).round() as i64 + intercept_ms;
        while cursor < reference_starts.len() && reference_starts[cursor] < mapped - tolerance {
            cursor += 1;
        }
        if cursor == reference_starts.len() {
            break;
        }
        let mut nearest = i64::MAX;
        let mut index = cursor;
        while index < reference_starts.len() && reference_starts[index] <= mapped + tolerance {
            nearest = nearest.min((reference_starts[index] - mapped).abs());
            index += 1;
        }
        if nearest <= tolerance {
            agreement += (tolerance - nearest) as f64 / tolerance as f64;
        }
    }
    agreement
}

/// Port of `fineActivityBest`: refine across the whole coarse plateau plus the fine radius,
/// then break score ties with onset agreement.
fn fine_best(
    reference: &ActivityTimeline,
    target: &ActivityTimeline,
    reference_starts: &[i64],
    target_starts: &[i64],
    scale: f64,
    plateau_start_ms: i64,
    plateau_end_ms: i64,
) -> Option<Candidate> {
    let mut candidates = Vec::new();
    let mut best: Option<Candidate> = None;
    let mut offset_ms = plateau_start_ms - FINE_RADIUS_MS;
    while offset_ms <= plateau_end_ms + FINE_RADIUS_MS {
        let offset_bins = (offset_ms as f64 / FINE_BIN_MS as f64).round() as i64;
        if let Some(score) = score_offset(reference, target, offset_bins) {
            let candidate = Candidate { scale, intercept_ms: offset_bins * FINE_BIN_MS, score };
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
        let onsets = onset_agreement(reference_starts, target_starts, scale, candidate.intercept_ms);
        if onsets > chosen_onsets || (onsets == chosen_onsets && candidate.score > chosen.score) {
            chosen = candidate;
            chosen_onsets = onsets;
        }
    }
    Some(chosen)
}

/// Port of `activityScaleCandidates`.
fn activity_scale_candidates(reference: &[SyncCue], target: &[SyncCue]) -> Vec<f64> {
    let mut candidates: Vec<f64> = vec![
        1.0,
        25.0 / 23.976,
        23.976 / 25.0,
        25.0 / 24.0,
        24.0 / 25.0,
        24.0 / 23.976,
        23.976 / 24.0,
    ];
    let reference_span = reference.last().unwrap().start_ms - reference.first().unwrap().start_ms;
    let target_span = target.last().unwrap().start_ms - target.first().unwrap().start_ms;
    if reference_span > 0 && target_span > 0 {
        let span_ratio = reference_span as f64 / target_span as f64;
        if (MIN_SCALE..=MAX_SCALE).contains(&span_ratio) {
            candidates.push(span_ratio);
        }
    }
    candidates.retain(|candidate| (*candidate).is_finite() && (MIN_SCALE..=MAX_SCALE).contains(candidate));
    candidates.sort_by(|a, b| a.total_cmp(b));
    let mut unique: Vec<f64> = Vec::new();
    for candidate in candidates {
        if unique.iter().all(|kept| (kept - candidate).abs() >= SCALE_DEDUP) {
            unique.push(candidate);
        }
    }
    unique
}

fn is_distinct_transform(first: Candidate, second: Candidate, target_end_ms: f64) -> bool {
    let start_difference = (first.intercept_ms - second.intercept_ms).abs() as f64;
    let end_difference = ((target_end_ms * first.scale + first.intercept_ms as f64)
        - (target_end_ms * second.scale + second.intercept_ms as f64))
        .abs();
    start_difference.max(end_difference) >= DISTINCT_TRANSFORM_MS
}

/// Discovers a global affine (FPS drift + offset) transform by correlating cue activity.
///
/// `target` is the timeline that will be retimed; `reference` is the trusted timeline.
/// Returns `None` when there are too few cues, no scale/offset corridor scores well, or the
/// best transform is ambiguous (margin below the configured threshold).
pub fn find_affine_alignment(target: &[SyncCue], reference: &[SyncCue]) -> Option<AffineAlignment> {
    let target = normalize_timeline(target);
    let reference = normalize_timeline(reference);
    if target.len() < MIN_CUES || reference.len() < MIN_CUES {
        return None;
    }
    let reference_activity = prepare_activity(&reference)?;
    let target_activity = prepare_activity(&target)?;
    let target_end_ms = target.iter().map(|cue| cue.end_ms).max()? as f64;

    let mut coarse_candidates: Vec<Candidate> = Vec::new();
    for scale in activity_scale_candidates(&reference, &target) {
        let target_coarse = if scale == 1.0 {
            target_activity.coarse.clone()
        } else {
            match build_activity(&target, scale, COARSE_BIN_MS) {
                Some(timeline) => timeline,
                None => continue,
            }
        };
        for offset_bins in coarse_offset_range(&reference_activity.coarse, &target_coarse) {
            if let Some(score) = score_offset(&reference_activity.coarse, &target_coarse, offset_bins) {
                let candidate = Candidate { scale, intercept_ms: offset_bins * COARSE_BIN_MS, score };
                coarse_candidates.push(candidate);
            }
        }
    }
    if coarse_candidates.is_empty() {
        return None;
    }

    // First maximum wins, matching Kotlin `maxByOrNull`.
    let mut best_index = 0usize;
    for index in 1..coarse_candidates.len() {
        if coarse_candidates[index].score > coarse_candidates[best_index].score {
            best_index = index;
        }
    }
    let coarse_best = coarse_candidates[best_index];

    let mut second_score = 0.0f64;
    let mut has_second = false;
    for (index, candidate) in coarse_candidates.iter().enumerate() {
        if index == best_index || !is_distinct_transform(coarse_best, *candidate, target_end_ms) {
            continue;
        }
        if !has_second || candidate.score > second_score {
            second_score = candidate.score;
            has_second = true;
        }
    }
    let margin = coarse_best.score - second_score;

    let target_fine = if coarse_best.scale == 1.0 {
        target_activity.fine.clone()
    } else {
        build_activity(&target, coarse_best.scale, FINE_BIN_MS)?
    };
    let (plateau_start, plateau_end) = coarse_plateau_ms(&coarse_candidates, best_index);
    let best = fine_best(
        &reference_activity.fine,
        &target_fine,
        &reference_activity.starts,
        &target_activity.starts,
        coarse_best.scale,
        plateau_start,
        plateau_end,
    )
    .unwrap_or(coarse_best);

    let small_sample = target.len() < SMALL_SAMPLE_CUE_LIMIT;
    let min_score = if small_sample { SMALL_SAMPLE_ACTIVITY_MIN_SCORE } else { ACTIVITY_MIN_SCORE };
    let min_margin = if small_sample { SMALL_SAMPLE_ACTIVITY_MIN_MARGIN } else { ACTIVITY_MIN_MARGIN };
    if best.score < min_score || margin < min_margin {
        return None;
    }

    Some(AffineAlignment { scale: best.scale, intercept_ms: best.intercept_ms, score: best.score, margin })
}

// -------------------------------------------------------------------- banded DP core

#[derive(Debug, Clone, Copy)]
struct Group {
    reference_start_index: usize,
    reference_count: usize,
    target_start_index: usize,
    target_count: usize,
    cost: f64,
}

/// Compact per-row storage mirroring Kotlin `BandedDpRows`.
struct BandedRows {
    row_count: usize,
    row_starts: Vec<usize>,
    row_ends: Vec<usize>,
    row_offsets: Vec<usize>,
    min_reachable: Vec<Option<usize>>,
    costs: Vec<f64>,
    previous_reference: Vec<usize>,
    previous_target: Vec<usize>,
    reference_counts: Vec<u8>,
    target_counts: Vec<u8>,
    steps: Vec<u8>,
    local_group_costs: Vec<f64>,
}

impl BandedRows {
    fn new(target_size: usize, reference_size: usize, centers: &[usize]) -> Self {
        let row_count = target_size + 1;
        let mut row_starts = vec![0usize; row_count];
        let mut row_ends = vec![0usize; row_count];
        let mut row_offsets = vec![0usize; row_count + 1];
        let mut total = 0usize;
        for target_index in 0..=target_size {
            let start = if target_index == 0 { 0 } else { centers[target_index].saturating_sub(BAND_RADIUS_CUES) };
            let end = (centers[target_index] + BAND_RADIUS_CUES).min(reference_size);
            row_starts[target_index] = start;
            row_ends[target_index] = end;
            row_offsets[target_index] = total;
            total += end - start + 1;
        }
        row_offsets[row_count] = total;
        Self {
            row_count,
            row_starts,
            row_ends,
            row_offsets,
            min_reachable: vec![None; row_count],
            costs: vec![f64::INFINITY; total],
            previous_reference: vec![0usize; total],
            previous_target: vec![0usize; total],
            reference_counts: vec![0u8; total],
            target_counts: vec![0u8; total],
            steps: vec![STEP_UNREACHABLE; total],
            local_group_costs: vec![0.0; total],
        }
    }

    fn set_start(&mut self) {
        let index = self.index_of(0, 0).expect("row 0 always contains reference 0");
        self.costs[index] = 0.0;
        self.steps[index] = STEP_START;
        self.min_reachable[0] = Some(0);
    }

    fn index_of(&self, target_index: usize, reference_index: usize) -> Option<usize> {
        if target_index >= self.row_count {
            return None;
        }
        let start = self.row_starts[target_index];
        let end = self.row_ends[target_index];
        if reference_index < start || reference_index > end {
            return None;
        }
        Some(self.row_offsets[target_index] + reference_index - start)
    }

    fn is_reachable(&self, cell: usize) -> bool {
        self.steps[cell] != STEP_UNREACHABLE
    }

    #[allow(clippy::too_many_arguments)]
    fn write(
        &mut self,
        cell: usize,
        target_index: usize,
        reference_index: usize,
        cost: f64,
        previous_reference_index: usize,
        previous_target_index: usize,
        reference_count: usize,
        target_count: usize,
        step: u8,
        local_group_cost: f64,
    ) {
        self.costs[cell] = cost;
        self.previous_reference[cell] = previous_reference_index;
        self.previous_target[cell] = previous_target_index;
        self.reference_counts[cell] = reference_count as u8;
        self.target_counts[cell] = target_count as u8;
        self.steps[cell] = step;
        self.local_group_costs[cell] = local_group_cost;
        if self.min_reachable[target_index].is_none_or(|current| reference_index < current) {
            self.min_reachable[target_index] = Some(reference_index);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn relax(
        &mut self,
        to_target_index: usize,
        to_reference_index: usize,
        candidate_cost: f64,
        previous_reference_index: usize,
        previous_target_index: usize,
        reference_count: usize,
        target_count: usize,
        step: u8,
        local_group_cost: f64,
    ) {
        let Some(cell) = self.index_of(to_target_index, to_reference_index) else { return };
        let current_cost = self.costs[cell];
        if !self.is_reachable(cell) || candidate_cost + 1e-9 < current_cost {
            self.write(
                cell,
                to_target_index,
                to_reference_index,
                candidate_cost,
                previous_reference_index,
                previous_target_index,
                reference_count,
                target_count,
                step,
                local_group_cost,
            );
        }
    }

    fn backtrack(&self, final_reference_index: usize, final_target_index: usize) -> Option<Vec<Group>> {
        let mut groups = Vec::new();
        let mut reference_index = final_reference_index;
        let mut target_index = final_target_index;
        let mut guard = 0usize;
        let guard_limit = self.row_count * 8 + final_reference_index * 2 + 32;
        while reference_index != 0 || target_index != 0 {
            guard += 1;
            if guard > guard_limit {
                return None;
            }
            let cell = self.index_of(target_index, reference_index)?;
            if !self.is_reachable(cell) {
                return None;
            }
            let step = self.steps[cell];
            if step == STEP_START {
                return None;
            }
            if step == STEP_GROUP {
                groups.push(Group {
                    reference_start_index: self.previous_reference[cell],
                    reference_count: self.reference_counts[cell] as usize,
                    target_start_index: self.previous_target[cell],
                    target_count: self.target_counts[cell] as usize,
                    cost: self.local_group_costs[cell],
                });
            }
            reference_index = self.previous_reference[cell];
            target_index = self.previous_target[cell];
        }
        groups.reverse();
        Some(groups)
    }

    /// Cheapest reachable cell of the final (all-targets-consumed) row.
    fn best_final_reference(&self) -> Option<usize> {
        let target_index = self.row_count - 1;
        let start = self.row_starts[target_index];
        let end = self.row_ends[target_index];
        let mut best: Option<(usize, f64)> = None;
        for reference_index in start..=end {
            let cell = self.index_of(target_index, reference_index)?;
            if !self.is_reachable(cell) {
                continue;
            }
            if best.is_none_or(|(_, cost)| self.costs[cell] < cost) {
                best = Some((reference_index, self.costs[cell]));
            }
        }
        best.map(|(reference_index, _)| reference_index)
    }
}

fn transform_time_double(time_ms: i64, scale: f64, intercept_ms: f64) -> f64 {
    time_ms as f64 * scale + intercept_ms
}

fn transform_time(time_ms: i64, scale: f64, intercept_ms: f64) -> i64 {
    (transform_time_double(time_ms, scale, intercept_ms).round() as i64).max(0)
}

fn lower_bound_reference(reference: &[SyncCue], time_ms: i64) -> usize {
    let mut low = 0usize;
    let mut high = reference.len();
    while low < high {
        let mid = (low + high) >> 1;
        if reference[mid].start_ms < time_ms {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

fn is_within_band(reference_index: usize, target_index: usize, centers: &[usize], reference_size: usize) -> bool {
    if reference_index > reference_size {
        return false;
    }
    let center = centers[target_index.min(centers.len() - 1)];
    reference_index.abs_diff(center) <= BAND_RADIUS_CUES || target_index == 0
}

#[allow(clippy::too_many_arguments)]
fn group_cost(
    reference_starts: &[f64],
    reference_ends: &[f64],
    transformed_target_starts: &[f64],
    transformed_target_ends: &[f64],
    reference_index: usize,
    reference_count: usize,
    target_index: usize,
    target_count: usize,
) -> f64 {
    let reference_start = reference_starts[reference_index];
    let reference_end = reference_ends[reference_index + reference_count - 1];
    let target_start = transformed_target_starts[target_index];
    let target_end = transformed_target_ends[target_index + target_count - 1];

    let reference_duration = (reference_end - reference_start).max(1.0);
    let target_duration = (target_end - target_start).max(1.0);
    let reference_mid = (reference_start + reference_end) * 0.5;
    let target_mid = (target_start + target_end) * 0.5;

    let start_error = (reference_start - target_start).abs() / START_END_TOLERANCE_MS;
    let end_error = (reference_end - target_end).abs() / START_END_TOLERANCE_MS;
    let midpoint_error = (reference_mid - target_mid).abs() / MIDPOINT_TOLERANCE_MS;
    let duration_error = (reference_duration - target_duration).abs() / DURATION_TOLERANCE_MS;
    start_error * 0.34 + end_error * 0.34 + midpoint_error * 0.18 + duration_error * 0.14
}

/// Replaces group shifts deviating from the local median by more than 250 ms (`Kotlin
/// rejectLocalShiftOutliers`).
fn reject_local_shift_outliers(shifts: &[i64]) -> Vec<i64> {
    let window_size = LOCAL_SHIFT_RADIUS_GROUPS * 2 + 1;
    if shifts.len() < window_size {
        return shifts.to_vec();
    }
    let mut window = vec![0i64; window_size];
    (0..shifts.len())
        .map(|index| {
            let from = index.saturating_sub(LOCAL_SHIFT_RADIUS_GROUPS);
            let to = (index + LOCAL_SHIFT_RADIUS_GROUPS).min(shifts.len() - 1);
            let count = to - from + 1;
            window[..count].copy_from_slice(&shifts[from..=to]);
            window[..count].sort_unstable();
            let median = window[count / 2];
            if (shifts[index] - median).abs() <= LOCAL_SHIFT_MAX_DEVIATION_MS {
                shifts[index]
            } else {
                median
            }
        })
        .collect()
}

fn longest_false_run(values: &[bool]) -> usize {
    let mut longest = 0usize;
    let mut current = 0usize;
    for &value in values {
        if value {
            current = 0;
        } else {
            current += 1;
            longest = longest.max(current);
        }
    }
    longest
}

fn affine_retimed(target: &[SyncCue], scale: f64, intercept_ms: f64) -> Vec<RetimedCue> {
    target
        .iter()
        .map(|cue| {
            let start = transform_time(cue.start_ms, scale, intercept_ms);
            let end = transform_time(cue.end_ms, scale, intercept_ms).max(start + 1);
            RetimedCue { original_start_ms: cue.start_ms, original_end_ms: cue.end_ms, start_ms: start, end_ms: end }
        })
        .collect()
}

fn failure_result(target: &[SyncCue], affine: &AffineAlignment) -> GroupRetime {
    GroupRetime {
        cues: affine_retimed(target, affine.scale, affine.intercept_ms as f64),
        target_coverage: 0.0,
        average_group_cost: f64::INFINITY,
        confident: false,
        groups: 0,
    }
}

/// Runs the banded DP that resolves local 1:1 / 1:2 / 2:1 / 1:3 / 3:1 / 2:2 groupings
/// plus skips, then transplants matched group starts from the reference while preserving
/// every target cue's duration.
///
/// When no valid group path exists, the cues still receive the affine transform and a
/// non-confident result with zero coverage is returned (this API is infallible by design).
pub fn retime_grouped(target: &[SyncCue], reference: &[SyncCue], affine: &AffineAlignment) -> GroupRetime {
    if target.len() < MIN_CUES || reference.len() < MIN_CUES
        || !affine.scale.is_finite() || !(0.85..=1.15).contains(&affine.scale)
    {
        return failure_result(target, affine);
    }
    let target = normalize_timeline(target);
    let reference = normalize_timeline(reference);

    let reference_starts: Vec<f64> = reference.iter().map(|cue| cue.start_ms as f64).collect();
    let reference_ends: Vec<f64> = reference.iter().map(|cue| cue.end_ms as f64).collect();
    let transformed_target_starts: Vec<f64> = target
        .iter()
        .map(|cue| transform_time_double(cue.start_ms, affine.scale, affine.intercept_ms as f64))
        .collect();
    let transformed_target_ends: Vec<f64> = target
        .iter()
        .map(|cue| transform_time_double(cue.end_ms, affine.scale, affine.intercept_ms as f64))
        .collect();

    let mut centers = Vec::with_capacity(target.len() + 1);
    for &start in &transformed_target_starts {
        let predicted = (start.round() as i64).max(0);
        centers.push(lower_bound_reference(&reference, predicted));
    }
    let predicted_end = (transformed_target_ends[target.len() - 1].round() as i64).max(0);
    centers.push(lower_bound_reference(&reference, predicted_end));

    let mut rows = BandedRows::new(target.len(), reference.len(), &centers);
    rows.set_start();

    for target_index in 0..=target.len() {
        let Some(mut reference_index) = rows.min_reachable[target_index] else { continue };
        let center = centers[target_index];
        let row_max = (center + BAND_RADIUS_CUES).min(reference.len());
        while reference_index <= row_max {
            if let Some(cell) = rows.index_of(target_index, reference_index) {
                if rows.is_reachable(cell) {
                    let cell_cost = rows.costs[cell];

                    if reference_index < reference.len() && reference_index + 1 <= row_max {
                        rows.relax(
                            target_index,
                            reference_index + 1,
                            cell_cost + SKIP_REFERENCE_COST,
                            reference_index,
                            target_index,
                            1,
                            0,
                            STEP_SKIP_REFERENCE,
                            0.0,
                        );
                    }
                    if target_index < target.len()
                        && is_within_band(reference_index, target_index + 1, &centers, reference.len())
                    {
                        rows.relax(
                            target_index + 1,
                            reference_index,
                            cell_cost + SKIP_TARGET_COST,
                            reference_index,
                            target_index,
                            0,
                            1,
                            STEP_SKIP_TARGET,
                            0.0,
                        );
                    }
                    for &(reference_count, target_count) in &GROUP_SHAPES {
                        let next_reference_index = reference_index + reference_count;
                        let next_target_index = target_index + target_count;
                        if next_reference_index > reference.len() || next_target_index > target.len() {
                            continue;
                        }
                        if !is_within_band(next_reference_index, next_target_index, &centers, reference.len()) {
                            continue;
                        }
                        let cost = group_cost(
                            &reference_starts,
                            &reference_ends,
                            &transformed_target_starts,
                            &transformed_target_ends,
                            reference_index,
                            reference_count,
                            target_index,
                            target_count,
                        );
                        if !cost.is_finite() || cost > MAX_GROUP_COST {
                            continue;
                        }
                        rows.relax(
                            next_target_index,
                            next_reference_index,
                            cell_cost + cost + GROUP_COMPLEXITY_COST * (reference_count + target_count - 2) as f64,
                            reference_index,
                            target_index,
                            reference_count,
                            target_count,
                            STEP_GROUP,
                            cost,
                        );
                    }
                }
            }
            reference_index += 1;
        }
    }

    let Some(final_reference_index) = rows.best_final_reference() else { return failure_result(&target, affine) };
    let Some(groups) = rows.backtrack(final_reference_index, target.len()) else { return failure_result(&target, affine) };
    if groups.is_empty() {
        return failure_result(&target, affine);
    }

    let mut retimed = affine_retimed(&target, affine.scale, affine.intercept_ms as f64);
    let shifts = reject_local_shift_outliers(
        &groups
            .iter()
            .map(|group| {
                reference[group.reference_start_index].start_ms - retimed[group.target_start_index].start_ms
            })
            .collect::<Vec<_>>(),
    );
    let mut matched_target = vec![false; target.len()];
    let mut matched_reference = vec![false; reference.len()];
    for (group_index, group) in groups.iter().enumerate() {
        for index in group.target_start_index..group.target_start_index + group.target_count {
            matched_target[index] = true;
        }
        for index in group.reference_start_index..group.reference_start_index + group.reference_count {
            matched_reference[index] = true;
        }
        for index in group.target_start_index..group.target_start_index + group.target_count {
            let duration = (retimed[index].end_ms - retimed[index].start_ms).max(1);
            let new_start = (retimed[index].start_ms + shifts[group_index]).max(0);
            retimed[index].start_ms = new_start;
            retimed[index].end_ms = new_start + duration;
        }
    }

    // Keep the output monotonic even for malformed, backwards-overlapping source cues.
    for index in 0..retimed.len() {
        let previous_start = if index == 0 { i64::MIN } else { retimed[index - 1].start_ms };
        let start = retimed[index].start_ms.max(previous_start);
        let end = retimed[index].end_ms.max(start + 1);
        retimed[index].start_ms = start;
        retimed[index].end_ms = end;
    }
    // Preserve overlaps that already existed, but never create a new one between cues that
    // were sequential before AutoSync.
    for index in 0..retimed.len().saturating_sub(1) {
        if target[index].end_ms > target[index + 1].start_ms {
            continue;
        }
        if retimed[index].end_ms > retimed[index + 1].start_ms
            && retimed[index + 1].start_ms > retimed[index].start_ms
        {
            retimed[index].end_ms = retimed[index + 1].start_ms;
        }
    }

    let matched_target_count = matched_target.iter().filter(|&&value| value).count();
    let target_coverage = matched_target_count as f64 / target.len() as f64;
    let average_group_cost = groups.iter().map(|group| group.cost).sum::<f64>() / groups.len() as f64;
    let longest_target_skip_run = longest_false_run(&matched_target);
    let confident = matched_target_count >= MIN_MATCHED_TARGET_CUES.min(target.len())
        && target_coverage >= MIN_TARGET_COVERAGE
        && average_group_cost <= MAX_AVERAGE_GROUP_COST
        && longest_target_skip_run <= MAX_LONGEST_TARGET_SKIP_RUN;

    GroupRetime {
        cues: retimed,
        target_coverage,
        average_group_cost,
        confident,
        groups: groups.len(),
    }
}

// ----------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Irregular spacing avoids the periodic ambiguity that legitimately defeats correlation.
    fn irregular_cues(count: usize, step_ms: i64) -> Vec<SyncCue> {
        (0..count)
            .map(|index| {
                let start = 2_000 + index as i64 * step_ms + (index % 5) as i64 * 137;
                let duration = 1_600 + (index % 3) as i64 * 120;
                SyncCue { start_ms: start, end_ms: start + duration, text: format!("line {index}") }
            })
            .collect()
    }

    fn scaled_cues(cues: &[SyncCue], scale: f64, offset_ms: i64) -> Vec<SyncCue> {
        cues.iter()
            .map(|cue| SyncCue {
                start_ms: (cue.start_ms as f64 * scale).round() as i64 + offset_ms,
                end_ms: (cue.end_ms as f64 * scale).round() as i64 + offset_ms,
                text: cue.text.clone(),
            })
            .collect()
    }

    fn identity_affine(offset_ms: i64) -> AffineAlignment {
        AffineAlignment { scale: 1.0, intercept_ms: offset_ms, score: 1.0, margin: 1.0 }
    }

    #[test]
    fn detects_fps_drift() {
        let target = irregular_cues(20, 4_000);
        let reference = scaled_cues(&target, 1.04, 1_000);
        let alignment = find_affine_alignment(&target, &reference).expect("FPS drift must be detected");
        assert!((alignment.scale - 1.04).abs() <= 0.003, "unexpected scale {}", alignment.scale);
        assert!((alignment.intercept_ms - 1_000).abs() <= 200, "unexpected offset {}", alignment.intercept_ms);
        assert!(alignment.score >= ACTIVITY_MIN_SCORE, "score {}", alignment.score);
        assert!(alignment.margin >= ACTIVITY_MIN_MARGIN, "margin {}", alignment.margin);
    }

    #[test]
    fn finds_a_pure_offset_without_drift() {
        let target = irregular_cues(18, 4_000);
        let reference = scaled_cues(&target, 1.0, 3_000);
        let alignment = find_affine_alignment(&target, &reference).expect("constant offset must be found");
        assert!((alignment.scale - 1.0).abs() <= 0.001, "scale {}", alignment.scale);
        assert!((alignment.intercept_ms - 3_000).abs() <= 200, "offset {}", alignment.intercept_ms);
    }

    #[test]
    fn rejects_ambiguous_offset() {
        // Perfectly periodic cues: every multiple of the period scores identically, so the
        // margin between the best and second distinct transform collapses to zero.
        let target: Vec<SyncCue> = (0..12)
            .map(|index| {
                let start = 1_000 + index as i64 * 4_000;
                SyncCue { start_ms: start, end_ms: start + 2_000, text: format!("line {index}") }
            })
            .collect();
        let reference = scaled_cues(&target, 1.0, 4_000);
        assert!(find_affine_alignment(&target, &reference).is_none());
    }

    #[test]
    fn rejects_tiny_samples() {
        let target = irregular_cues(3, 4_000);
        let reference = scaled_cues(&target, 1.0, 1_000);
        assert!(find_affine_alignment(&target, &reference).is_none());
    }

    #[test]
    fn grouped_retiming_preserves_durations_and_monotonic_order() {
        let target = irregular_cues(24, 4_000);
        let offset_ms = 2_500;
        // One merged reference cue spans target cues 5 and 6 (a 2:1 group); one reference
        // cue is split into two at target index 15 (a 1:2 group).
        let mut reference = Vec::new();
        for (index, cue) in target.iter().enumerate() {
            let start = cue.start_ms + offset_ms;
            let end = cue.end_ms + offset_ms;
            if index == 6 {
                continue;
            }
            if index == 5 {
                reference.push(SyncCue {
                    start_ms: start,
                    end_ms: target[6].end_ms + offset_ms,
                    text: "merged".into(),
                });
            } else if index == 15 {
                let middle = start + (end - start) / 2;
                reference.push(SyncCue { start_ms: start, end_ms: middle, text: "split-a".into() });
                reference.push(SyncCue { start_ms: middle, end_ms: end, text: "split-b".into() });
            } else {
                reference.push(SyncCue { start_ms: start, end_ms: end, text: cue.text.clone() });
            }
        }
        assert_eq!(reference.len(), target.len());
        let result = retime_grouped(&target, &reference, &identity_affine(offset_ms));

        assert!(result.confident, "clean split/merge must be confident");
        assert!(result.groups >= 20, "groups: {}", result.groups);
        assert!((result.target_coverage - 1.0).abs() < f64::EPSILON, "coverage {}", result.target_coverage);
        assert_eq!(result.cues.len(), target.len());
        for (original, retimed) in target.iter().zip(result.cues.iter()) {
            assert_eq!(retimed.original_start_ms, original.start_ms);
            assert_eq!(retimed.original_end_ms, original.end_ms);
            assert_eq!(
                retimed.end_ms - retimed.start_ms,
                original.end_ms - original.start_ms,
                "duration changed for cue {}",
                original.text,
            );
        }
        for pair in result.cues.windows(2) {
            assert!(pair[0].start_ms <= pair[1].start_ms, "starts out of order");
            assert!(pair[0].end_ms > pair[0].start_ms);
        }
    }

    #[test]
    fn confidence_gates_reject_unrelated_timelines() {
        let target = irregular_cues(12, 4_000);
        let reference: Vec<SyncCue> = target
            .iter()
            .enumerate()
            .map(|(index, cue)| SyncCue {
                start_ms: cue.start_ms + 250_000 + index as i64 * 7,
                end_ms: cue.end_ms + 250_000 + index as i64 * 7,
                text: cue.text.clone(),
            })
            .collect();
        let result = retime_grouped(&target, &reference, &identity_affine(0));
        assert!(!result.confident);
        assert!(result.target_coverage < MIN_TARGET_COVERAGE, "coverage {}", result.target_coverage);
        assert_eq!(result.groups, 0);
    }

    #[test]
    fn confidence_gates_require_minimum_coverage() {
        // Only the first half of the target exists in the reference. The trailing target cues
        // are too far from any reference cue to form a group below MAX_GROUP_COST, so the
        // coverage and matched-cue gates both fail.
        let target = irregular_cues(24, 4_000);
        let reference: Vec<SyncCue> = target[..12]
            .iter()
            .map(|cue| SyncCue {
                start_ms: cue.start_ms + 2_000,
                end_ms: cue.end_ms + 2_000,
                text: cue.text.clone(),
            })
            .collect();
        let result = retime_grouped(&target, &reference, &identity_affine(2_000));
        assert!(result.target_coverage < MIN_TARGET_COVERAGE, "coverage {}", result.target_coverage);
        assert!(!result.confident);
        assert!(result.groups >= 12);
    }
}
