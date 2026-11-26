//! Subtitle sync by ear for streams with no subtitles of their own.
//!
//! A second, silent mpv core decodes a few short stretches of the film's sound to 16 kHz mono PCM
//! files (`ao=pcm`, far faster than real time). The WebRTC voice detector marks speech in each
//! stretch, and `nuvio-core::autosync_audio` decides where the downloaded subtitle belongs.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nuvio_core::autosync::{AlignmentV2, SyncCue};
use nuvio_core::autosync_audio::{fit_to_speech, listen_at, spoken_lines, Heard};
use nuvio_core::voice_activity::detect_speech;
use nuvio_player::mpv::{Mpv, MpvApi, EVENT_END_FILE, EVENT_SHUTDOWN};

/// One decoded stretch: a little file that is removed as soon as it has been heard.
fn hear(api: &Arc<MpvApi>, url: &str, headers: &[(String, String)], at: f64, length: f64, index: usize) -> Option<Heard> {
    let file = std::env::temp_dir().join(format!("nuvio-ear-{}-{}-{}", std::process::id(), index, Instant::now().elapsed().as_nanos()));
    let core = Mpv::create(api.clone()).ok()?;
    for (name, value) in [
        ("config", "no"),
        ("terminal", "no"),
        ("ao", "pcm"),
        ("ao-pcm-file", file.to_string_lossy().as_ref()),
        ("ao-pcm-waveheader", "no"),
        ("audio-format", "s16"),
        ("audio-samplerate", "16000"),
        ("audio-channels", "mono"),
        ("hr-seek", "yes"),
        ("vid", "no"),
        ("sid", "no"),
        ("mute", "no"),
        ("volume", "100"),
        ("start", &format!("{at:.3}")),
        ("end", &format!("{:.3}", at + length)),
    ] {
        core.set_option(name, value).ok()?;
    }
    if !headers.is_empty() {
        let fields: Vec<String> = headers
            .iter()
            .filter(|(name, value)| !name.is_empty() && !value.is_empty())
            .map(|(name, value)| format!("{name}: {value}"))
            .collect();
        if !fields.is_empty() {
            core.set_option_string_list("http-header-fields", &fields).ok()?;
        }
    }
    core.initialize().ok()?;
    let started = Instant::now();
    if core.command(&["loadfile", url]).is_err() {
        return None;
    }
    while started.elapsed() < Duration::from_secs(90) {
        // SAFETY: only this thread waits on this handle.
        let event = unsafe { &*core.wait_event(0.05) };
        if event.event_id == EVENT_END_FILE || event.event_id == EVENT_SHUTDOWN {
            break;
        }
    }
    let _ = core.command(&["stop"]);
    let bytes = std::fs::read(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    if bytes.len() < 16_000 * 2 * 10 {
        return None;
    }
    let pcm: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let speech = detect_speech(&pcm);
    Some(Heard { start: at, speech })
}

/// Runs the by-ear fallback. `None` means there was not enough sound to decide.
pub fn sync_by_ear(
    url: &str,
    headers: &[(String, String)],
    target: &[SyncCue],
    duration: f64,
    now: f64,
) -> Option<AlignmentV2> {
    let lib = nuvio_player::locate_libmpv()?;
    let api = MpvApi::load(&lib).ok()?;
    let file = spoken_lines(target);
    if file.len() < 4 || duration <= 90.0 {
        return None;
    }
    let spots = listen_at(&file, duration, now, 4, 45.0);
    let mut heard = Vec::new();
    for (index, at) in spots.iter().enumerate() {
        if let Some(s) = hear(&api, url, headers, *at, 45.0, index) {
            heard.push(s);
        }
    }
    let found = fit_to_speech(&heard, &file)?;
    let offset_ms = (found.shift * 1000.0).round().clamp(-60_000.0, 60_000.0) as i64;
    Some(AlignmentV2 {
        offset_ms,
        scale: found.scale,
        confident: true,
        coverage: found.score,
        cues: Vec::new(),
    })
}
