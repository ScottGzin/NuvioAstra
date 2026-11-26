//! Automatic subtitle sync by ear: align a downloaded subtitle file with when people speak in a
//! video that has no subtitles of its own. Port of VyseTV's `sound_sync.dart`.
//!
//! A `Heard` spot is 10 ms speech flags (1/0) decoded from a short stretch of the film's sound
//! (`voice_activity::detect_speech`). The file's lines are turned into a line-or-not signal at the
//! same rate; Pearson correlation over every shift finds where the two agree far better than
//! anywhere else. Several stretches must agree on their own before a match is trusted.

use crate::autosync::SyncCue;

/// A stretch of a film's sound: film seconds where it starts, and 100 speech flags per second.
#[derive(Debug, Clone)]
pub struct Heard {
    pub start: f64,
    pub speech: Vec<u8>,
}

impl Heard {
    pub fn end(&self) -> f64 {
        self.start + self.speech.len() as f64 / 100.0
    }
}

/// A confident affine match: subtitle time × `scale` + `shift` = film time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioMatch {
    pub scale: f64,
    /// Seconds.
    pub shift: f64,
    /// Pearson correlation at the best shift.
    pub score: f64,
    /// Best correlation more than 2.5 s away.
    pub elsewhere: f64,
    /// Stretches whose own best shift agrees.
    pub agree: usize,
}

const SCALES: [f64; 7] = [1.0, 24.0 / 23.976, 23.976 / 24.0, 25.0 / 23.976, 23.976 / 25.0, 25.0 / 24.0, 24.0 / 25.0];
const REACH: usize = 120; // seconds
const TENTHS: usize = 10;

/// Filters a subtitle file down to lines that are someone speaking: not a sound description or a
/// song, which the audio has no speech for.
pub fn spoken_lines(cues: &[SyncCue]) -> Vec<(f64, f64)> {
    cues.iter()
        .filter(|cue| {
            let text = cue.text.to_lowercase();
            let stripped = text
                .chars()
                .filter(|c| *c != '<' && *c != '>' && *c != '{' && *c != '}')
                .collect::<String>();
            let t = stripped.trim();
            if t.is_empty() || t.chars().all(|c| "♪♫♬♩🎵🎶#*- ".contains(c)) {
                return false;
            }
            if t.starts_with('♪') || t.ends_with('♪') {
                return false;
            }
            let first = t.chars().next().unwrap();
            let last = t.chars().last().unwrap();
            if matches!(first, '[' | '(' | '（' | '【' | '「') && matches!(last, ']' | ')' | '）' | '】' | '」') {
                return false;
            }
            cue.end_ms > cue.start_ms
        })
        .map(|cue| (cue.start_ms as f64 / 1000.0, cue.end_ms as f64 / 1000.0))
        .collect()
}

/// Where to listen: `count` stretches of `length` seconds where the file has the most dialogue,
/// spread over the film, the first soon after `now` so subtitles are in time early.
pub fn listen_at(lines: &[(f64, f64)], duration: f64, now: f64, count: usize, len: f64) -> Vec<f64> {
    if duration < len * 2.0 {
        return Vec::new();
    }
    let first = (now + 15.0).max(180.0_f64.min(duration * 0.06));
    let last = duration - 360.0_f64.min(duration * 0.08) - len;
    if last <= first {
        return vec![0.0f64.max((now + 15.0).min(duration - len))];
    }
    let dialogue = |at: f64| -> f64 {
        lines
            .iter()
            .map(|(a, b)| {
                let x = a.max(at);
                let y = b.min(at + len);
                (y - x).max(0.0)
            })
            .sum()
    };
    let best_in = |from: f64, to: f64| -> f64 {
        let mut best = from;
        let mut top = -1.0;
        let mut at = from;
        while at <= to {
            let d = dialogue(at);
            if d > top {
                best = at;
                top = d;
            }
            at += 5.0;
        }
        best
    };
    let mut picked = vec![best_in(first, last.min(first + 600.0))];
    let sections = count - 1;
    let mut candidates = Vec::new();
    for k in 0..sections {
        candidates.push(best_in(
            first + (last - first) * k as f64 / sections as f64,
            first + (last - first) * (k + 1) as f64 / sections as f64,
        ));
    }
    while picked.len() < count && !candidates.is_empty() {
        candidates.sort_by(|a, b| {
            let da = picked.iter().map(|p| (a - p).abs()).fold(f64::INFINITY, f64::min);
            let db = picked.iter().map(|p| (b - p).abs()).fold(f64::INFINITY, f64::min);
            db.total_cmp(&da)
        });
        let c = candidates.remove(0);
        if picked.iter().all(|p| (c - p).abs() >= len) {
            picked.push(c);
        }
    }
    picked
}

#[derive(Clone, Copy)]
struct Peak {
    scale: f64,
    shift: f64,
    r: f64,
    elsewhere: f64,
    agree: usize,
}

/// The file's line-or-not signal at `rate` steps per second from film time `from`, file lines at
/// their times × `scale`.
fn covered(file: &[(f64, f64)], scale: f64, from: f64, count: usize, rate: usize) -> Vec<u8> {
    let mut out = vec![0u8; count];
    for (a, b) in file {
        let i0 = ((a * scale - from) * rate as f64).ceil() as isize;
        let i1 = ((b * scale - from) * rate as f64).ceil() as isize;
        if i1 <= 0 || i0 >= count as isize {
            continue;
        }
        let lo = i0.max(0) as usize;
        let hi = (i1 as usize).min(count);
        for x in out.iter_mut().take(hi).skip(lo) {
            *x = 1;
        }
    }
    out
}

fn tenths(speech: &[u8]) -> Vec<f64> {
    speech.chunks(TENTHS).map(|c| c.iter().map(|x| *x as f64).sum::<f64>() / c.len() as f64).collect()
}

fn pearson(v: &[f64], lines: &[u8]) -> f64 {
    let n = v.len().min(lines.len());
    if n == 0 {
        return 0.0;
    }
    let mut sv = 0.0;
    let mut svv = 0.0;
    let mut ss = 0.0;
    let mut svs = 0.0;
    for i in 0..n {
        sv += v[i];
        svv += v[i] * v[i];
        let l = lines[i] as f64;
        ss += l;
        svs += v[i] * l;
    }
    let var_v = svv - sv * sv / n as f64;
    let var_s = ss - ss * ss / n as f64;
    if var_v <= 0.0 || var_s <= 0.0 {
        return 0.0;
    }
    (svs - sv * ss / n as f64) / (var_v * var_s).sqrt()
}

fn search(heard: &[Heard], coarse: &[Vec<f64>], file: &[(f64, f64)], scale: f64) -> Option<Peak> {
    if heard.is_empty() {
        return None;
    }
    let reach = REACH * TENTHS;
    let mut r = vec![0.0f64; 2 * reach + 1];
    let mut own = Vec::new();
    for (k, spot) in heard.iter().enumerate() {
        let v = &coarse[k];
        let n = v.len();
        if n == 0 {
            return None;
        }
        let b0 = (spot.start * TENTHS as f64).round() as isize;
        let from = (b0 - reach as isize) as f64 / TENTHS as f64;
        let lines = covered(file, scale, from, n + 2 * reach, TENTHS);
        let mut mine = vec![0.0f64; 2 * reach + 1];
        for lag in -(reach as isize)..=reach as isize {
            let start = (reach as isize - lag) as usize;
            let slice = &lines[start..start + n];
            let flags: Vec<u8> = slice.iter().map(|x| if *x > 0 { 1 } else { 0 }).collect();
            mine[(lag + reach as isize) as usize] = pearson(v, &flags);
            r[(lag + reach as isize) as usize] += mine[(lag + reach as isize) as usize] / heard.len() as f64;
        }
        own.push(mine);
    }
    let mut top = 0usize;
    for i in 1..r.len() {
        if r[i] > r[top] {
            top = i;
        }
    }
    let mut elsewhere = -1.0f64;
    for (i, x) in r.iter().enumerate() {
        if (i as isize - top as isize).abs() > 25 && *x > elsewhere {
            elsewhere = *x;
        }
    }
    let mut agree = 0usize;
    for mine in &own {
        let mut at = top.saturating_sub(30);
        for i in top.saturating_sub(30)..=(top + 30).min(mine.len() - 1) {
            if mine[i] > mine[at] {
                at = i;
            }
        }
        if (at as isize - top as isize).abs() <= 5 {
            agree += 1;
        }
    }
    Some(Peak {
        scale,
        shift: (top as isize - reach as isize) as f64 / TENTHS as f64,
        r: r[top],
        elsewhere,
        agree,
    })
}

fn fine(heard: &[Heard], file: &[(f64, f64)], scale: f64, around: f64) -> f64 {
    let mut best = around;
    let mut top = -2.0f64;
    for k in -30..=30 {
        let shift = around + k as f64 / 100.0;
        let mut r = 0.0;
        for spot in heard {
            let n = spot.speech.len();
            let lines = covered(file, scale, spot.start - shift, n, 100);
            let flags: Vec<u8> = lines.iter().map(|x| if *x > 0 { 1 } else { 0 }).collect();
            r += pearson(
                &spot.speech.iter().map(|x| *x as f64).collect::<Vec<_>>(),
                &flags,
            );
        }
        if r > top {
            top = r;
            best = shift;
        }
    }
    best
}

/// Where the file's lines belong, judged by the speech heard in `spots`. `None` when nothing is
/// clear (waiting for more sound or the film has no usable speech).
pub fn fit_to_speech(spots: &[Heard], file: &[(f64, f64)]) -> Option<AudioMatch> {
    let heard: Vec<&Heard> = spots
        .iter()
        .filter(|s| {
            s.speech.len() >= 1000 && s.speech.iter().map(|x| *x as usize).sum::<usize>() as f64 / s.speech.len() as f64 >= 0.1
        })
        .collect();
    if heard.len() < 2 || file.is_empty() {
        return None;
    }
    let coarse: Vec<Vec<f64>> = heard.iter().map(|s| tenths(&s.speech)).collect();
    let owned: Vec<Heard> = heard.into_iter().cloned().collect();
    let mut best: Option<Peak> = None;
    for scale in SCALES {
        let Some(p) = search(&owned, &coarse, file, scale) else { continue };
        let better = match best {
            None => true,
            Some(b) if scale == 1.0 => p.r > b.r,
            Some(b) => p.r > b.r + 0.02,
        };
        if better {
            best = Some(p);
        }
    }
    let p = best?;
    let shift = fine(&owned, file, p.scale, p.shift);
    let clear = p.r >= 0.12 && p.r - p.elsewhere >= 0.06 && p.r >= p.elsewhere * 1.6 && p.agree == owned.len();
    if !clear {
        return None;
    }
    Some(AudioMatch {
        scale: p.scale,
        shift,
        score: p.r,
        elsewhere: p.elsewhere,
        agree: p.agree,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoken_lines_skips_music_and_sound_descriptions() {
        let cues = vec![
            SyncCue { start_ms: 0, end_ms: 1000, text: "♪ music ♪".into() },
            SyncCue { start_ms: 1000, end_ms: 2000, text: "[door shuts]".into() },
            SyncCue { start_ms: 2000, end_ms: 3000, text: "Hello there".into() },
        ];
        assert_eq!(spoken_lines(&cues), vec![(2.0, 3.0)]);
    }

    #[test]
    fn fit_to_speech_finds_a_shift_from_two_spots() {
        // A file with a line every second; speech in the film is the same pattern moved by 2.5 s.
        // Irregular spacing, like real dialogue: a periodic pattern would match at many shifts.
        let mut file = Vec::new();
        let mut at = 0.0f64;
        let mut seed = 1u64;
        for _ in 0..50 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            at += 0.5 + ((seed >> 33) % 1500) as f64 / 1000.0; // 0.5..2.0 s, irregular
            file.push((at + 0.07, at + 0.42 + ((seed >> 13) % 250) as f64 / 1000.0));
        }
        let mut spots = Vec::new();
        for start in [0.0, 40.0] {
            let mut speech = vec![0u8; 2000]; // 20 s at 100 flags/s
            for (a, b) in &file {
                let from = ((a + 2.5 - start) * 100.0).round() as isize;
                let to = ((b + 2.5 - start) * 100.0).round() as isize;
                for idx in from.max(0)..to.min(speech.len() as isize) {
                    speech[idx as usize] = 1;
                }
            }
            spots.push(Heard { start, speech });
        }
        let found = fit_to_speech(&spots, &file).expect("a clear shift must be found");
        assert!((found.shift - 2.5).abs() < 0.15, "shift {}", found.shift);
    }
}
