//! Watch progress of the current playback, saved like the original player does
//! (`persistPlaybackProgressTick`, `flushWatchProgress`, `scheduleProgressSyncAfterSeek`):
//! - first save as soon as the position is worth storing (≥ 1 s, not a placeholder clip), then
//!   every 60 s while playing;
//! - immediately on pause and on end of file; 700 ms after a seek settles;
//! - on close, on switching to another title and on app exit (`AppState::close`).
//!
//! The player thread only compares a few numbers per snapshot (4 per second); writes go to a
//! background thread, so playback never waits for the disk. The final save on close is written
//! directly (playback is already stopping). Only titles started from Details/Library carry a
//! `PlaybackItem`; live TV and the player self-tests are never recorded.

use nuvio_player::PlayerSnapshot;
use nuvio_store::library::{self, PlaybackItem, ProgressEntry, PROGRESS_PERSIST_INTERVAL_MS, SEEK_SAVE_DEBOUNCE_MS};
use nuvio_store::nextgen::AppStore;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

/// How close to the duration an end of file must be to be the end of the episode: a stream whose
/// connection drops also reports one, wherever it stopped.
const END_WINDOW_MS: i64 = 10_000;
const END_WINDOW_FRACTION: f64 = 0.02;

/// Tolerance before a position jump counts as a seek (playback itself advances ~250 ms per snapshot).
const SEEK_JUMP_MS: i64 = 3_000;

/// Called after a save with `flush`: true for pause / end / seek / close (the moments the original
/// sends progress to the account, `flushWatchProgress`), false for the periodic save.
type Notifier = Arc<Mutex<Option<Box<dyn Fn(bool) + Send>>>>;

struct Tracked {
    session: u64,
    profile: i64,
    item: PlaybackItem,
    watched_percent: i64,
    position_ms: i64,
    duration_ms: i64,
    paused: bool,
    ended: bool,
    /// The next episode was started from this one's credits.
    finished: bool,
    observed_at_ms: i64,
    last_save_ms: Option<i64>,
    seek_at_ms: Option<i64>,
}

impl Tracked {
    /// Completed whatever the percentage.
    fn completed(&self) -> bool {
        let window = END_WINDOW_MS.max((self.duration_ms as f64 * END_WINDOW_FRACTION) as i64);
        self.finished || (self.ended && (self.duration_ms <= 0 || self.position_ms + window >= self.duration_ms))
    }
}

pub struct Job {
    pub profile: i64,
    pub entry: ProgressEntry,
    pub release_info: Option<String>,
    pub flush: bool,
}

pub struct ProgressTracker {
    dir: PathBuf,
    current: Mutex<Option<Tracked>>,
    writer: Mutex<Sender<Job>>,
    notifier: Notifier,
}

impl ProgressTracker {
    /// `dir`: the NextGen data folder. Starts the background writer.
    pub fn new(dir: PathBuf) -> Self {
        let (tx, rx) = channel::<Job>();
        let notifier: Notifier = Arc::new(Mutex::new(None));
        let (thread_dir, thread_notifier) = (dir.clone(), notifier.clone());
        std::thread::Builder::new()
            .name("nuvio-progress".into())
            .spawn(move || {
                let Ok(store) = AppStore::open(thread_dir) else { return };
                for job in rx {
                    write(&store, job, &thread_notifier);
                }
            })
            .expect("progress writer thread");
        Self { dir, current: Mutex::new(None), writer: Mutex::new(tx), notifier }
    }

    /// Called after a save (e.g. to tell the UI that Continue watching changed).
    pub fn set_notifier(&self, notify: impl Fn(bool) + Send + 'static) {
        *self.notifier.lock().expect("notifier lock") = Some(Box::new(notify));
    }

    /// Starts tracking a session opened for `item`.
    pub fn start(&self, session: u64, profile: i64, item: PlaybackItem, watched_percent: i64) {
        *self.current.lock().expect("tracker lock") = Some(Tracked {
            session,
            profile,
            item,
            watched_percent,
            position_ms: 0,
            duration_ms: 0,
            paused: false,
            ended: false,
            finished: false,
            observed_at_ms: library::now_ms(),
            last_save_ms: None,
            seek_at_ms: None,
        });
    }

    /// The user went on to the next episode from its card: the tracked one counts as finished.
    pub fn mark_finished(&self) {
        if let Some(t) = self.current.lock().expect("tracker lock").as_mut() {
            t.finished = true;
        }
    }

    /// Player thread, every snapshot: decides whether this moment must be saved.
    pub fn observe(&self, snapshot: &PlayerSnapshot) {
        let job = self.decide(snapshot, library::now_ms());
        if let Some(job) = job {
            let _ = self.writer.lock().expect("writer lock").send(job);
        }
    }

    fn decide(&self, s: &PlayerSnapshot, now: i64) -> Option<Job> {
        let mut guard = self.current.lock().expect("tracker lock");
        let t = guard.as_mut().filter(|t| t.session == s.session)?;
        let position = (s.time_pos * 1000.0) as i64;
        let duration = (s.duration * 1000.0) as i64;
        if !s.loaded && !s.ended {
            return None;
        }
        // Keep the last real values (mpv may report zeros once the file is unloaded).
        let previous = t.position_ms;
        if position > 0 {
            t.position_ms = position;
        }
        if duration > 0 {
            t.duration_ms = duration;
        }
        let elapsed = (now - t.observed_at_ms).max(0);
        t.observed_at_ms = now;

        let mut save = false;
        let mut flush = false;
        if s.ended && !t.ended {
            save = true;
            flush = true;
        }
        t.ended = s.ended;
        if s.paused && !t.paused {
            save = true;
            flush = true;
        }
        t.paused = s.paused;
        if position > 0 && previous > 0 && (position - previous).abs() > SEEK_JUMP_MS + elapsed * 2 {
            t.seek_at_ms = Some(now);
        }
        if let Some(at) = t.seek_at_ms {
            if now - at >= SEEK_SAVE_DEBOUNCE_MS {
                t.seek_at_ms = None;
                save = true;
                flush = true;
            }
        }
        if !s.paused && t.last_save_ms.is_none_or(|last| now - last >= PROGRESS_PERSIST_INTERVAL_MS) {
            save = true;
        }
        if !save {
            return None;
        }
        let entry = t.item.entry(t.position_ms, t.duration_ms, t.completed(), now, t.watched_percent)?;
        t.last_save_ms = Some(now);
        Some(Job { profile: t.profile, entry, release_info: t.item.release_info.clone(), flush })
    }

    /// Final save of the tracked session (close, switch, exit), written before returning.
    pub fn finish(&self, last: Option<&PlayerSnapshot>) {
        let tracked = self.current.lock().expect("tracker lock").take();
        let Some(mut t) = tracked else { return };
        if let Some(s) = last.filter(|s| s.session == t.session) {
            if s.time_pos > 0.0 {
                t.position_ms = (s.time_pos * 1000.0) as i64;
            }
            if s.duration > 0.0 {
                t.duration_ms = (s.duration * 1000.0) as i64;
            }
            t.ended |= s.ended;
        }
        let Some(entry) = t.item.entry(t.position_ms, t.duration_ms, t.completed(), library::now_ms(), t.watched_percent) else { return };
        if let Ok(store) = AppStore::open(&self.dir) {
            write(&store, Job { profile: t.profile, entry, release_info: t.item.release_info, flush: true }, &self.notifier);
        }
    }
}

fn write(store: &AppStore, job: Job, notifier: &Notifier) {
    match store.record_progress(job.profile, job.entry, job.release_info) {
        Ok(Some(_)) => {
            if let Some(notify) = notifier.lock().expect("notifier lock").as_ref() {
                notify(job.flush);
            }
        }
        Ok(None) => {}
        // No title or URL in the message: only the error kind.
        Err(e) => eprintln!("[nextgen] progress not saved: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(session: u64, pos: f64, dur: f64, paused: bool, ended: bool) -> PlayerSnapshot {
        PlayerSnapshot { session, loaded: true, time_pos: pos, duration: dur, paused, ended, ..Default::default() }
    }

    fn movie() -> PlaybackItem {
        serde_json::from_value(serde_json::json!({
            "contentType": "movie", "parentMetaId": "tt1", "parentMetaType": "movie", "videoId": "tt1", "title": "Filme"
        }))
        .unwrap()
    }

    #[test]
    fn saves_like_the_original_player() {
        let dir = std::env::temp_dir().join(format!("nuvio-progress-tracker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tracker = ProgressTracker::new(dir.clone());
        tracker.start(7, 1, movie(), 90);
        let d = |s: &PlayerSnapshot, now: i64| tracker.decide(s, now).map(|j| j.entry.last_position_ms);

        assert_eq!(d(&snapshot(6, 50.0, 6000.0, false, false), 0), None, "other session ignored");
        assert_eq!(d(&snapshot(7, 0.5, 6000.0, false, false), 0), None, "under 1 s");
        assert_eq!(d(&snapshot(7, 1.2, 6000.0, false, false), 250), Some(1200), "first save");
        assert_eq!(d(&snapshot(7, 1.5, 6000.0, false, false), 500), None);
        assert_eq!(d(&snapshot(7, 30.0, 6000.0, false, false), 30_000), None, "no save every update");
        assert_eq!(d(&snapshot(7, 61.0, 6000.0, false, false), 60_250), Some(61_000), "every 60 s");
        assert_eq!(d(&snapshot(7, 62.0, 6000.0, true, false), 61_250), Some(62_000), "on pause");
        assert_eq!(d(&snapshot(7, 62.0, 6000.0, true, false), 120_000), None, "paused: nothing more");
        // Seek: saved once 700 ms after the jump.
        assert_eq!(d(&snapshot(7, 900.0, 6000.0, false, false), 70_000), None);
        assert_eq!(d(&snapshot(7, 900.3, 6000.0, false, false), 70_300), None);
        assert_eq!(d(&snapshot(7, 900.8, 6000.0, false, false), 70_800), Some(900_800), "after the seek settles");
        // A connection that drops mid-file also reports an end: not watched.
        let dropped = tracker.decide(&snapshot(7, 901.0, 6000.0, false, true), 71_000).unwrap();
        assert!(!dropped.entry.is_completed);
        assert_eq!(dropped.entry.last_position_ms, 901_000);
        // End of file completes it even with zeros reported afterwards.
        tracker.decide(&snapshot(7, 5_998.0, 6000.0, false, false), 129_000);
        let ended = tracker.decide(&snapshot(7, 0.0, 0.0, false, true), 130_000).unwrap();
        assert!(ended.entry.is_completed);
        assert_eq!(ended.entry.last_position_ms, 6_000_000);
    }

    #[test]
    fn closing_saves_the_position_without_completing() {
        let dir = std::env::temp_dir().join(format!("nuvio-progress-close-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tracker = ProgressTracker::new(dir.clone());
        tracker.start(3, 2, movie(), 90);
        tracker.finish(Some(&snapshot(3, 1234.5, 6000.0, false, false)));
        let store = AppStore::open(&dir).unwrap();
        let saved = store.progress(2).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].last_position_ms, 1_234_500);
        assert!(!saved[0].is_completed);
        assert!(store.progress(1).unwrap().is_empty(), "other profile untouched");
        // Nothing tracked any more: a second close does nothing.
        tracker.finish(Some(&snapshot(3, 2000.0, 6000.0, false, false)));
        assert_eq!(store.progress(2).unwrap()[0].last_position_ms, 1_234_500);
    }
}
