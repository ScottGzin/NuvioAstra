//! Discord Rich Presence (`DiscordIpcClient` / `DiscordPresenceManager` of the original desktop app).
//!
//! The activity goes to the Discord app running on this computer through its local named pipe
//! (`\\.\pipe\discord-ipc-N`); nothing goes to the internet from here. The application id is the
//! original's public one (`NUVIO_DISCORD_CLIENT_ID` in local.properties picks another). Off by
//! default, like the original; when Discord is closed the app keeps trying every 15 s, silently.
//! Every presence is a "Watching" one: the tab, the title being viewed, or what is playing with a
//! progress bar.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::State;

/// `DiscordConfig.CLIENT_ID` of the original (its public default).
fn client_id() -> &'static str {
    option_env!("NUVIO_DISCORD_CLIENT_ID").filter(|v| !v.is_empty()).unwrap_or("1538974392376369212")
}

const RECONNECT: Duration = Duration::from_secs(15);
/// Discord activity type 3: "Watching".
const WATCHING: i64 = 3;

/// What the interface shows (`PresenceSnapshot` of the original).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Presence {
    /// `tab`, `details` or `player`.
    pub kind: String,
    /// Tab name, title viewed, or title playing.
    pub title: String,
    /// The episode (`S1E2 - Nome`) of what is playing.
    pub episode: Option<String>,
    pub poster: Option<String>,
    pub playing: bool,
    pub position_ms: i64,
    pub duration_ms: i64,
}

/// `S1E2 - Title` as `S1, E2: Title` (`toDiscordEpisodeLabel`).
fn episode_label(value: &str) -> String {
    let v = value.trim();
    let Some(rest) = v.strip_prefix('S') else { return v.into() };
    let Some((season, after)) = rest.split_once('E') else { return v.into() };
    if season.is_empty() || !season.chars().all(|c| c.is_ascii_digit()) {
        return v.into();
    }
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return v.into();
    }
    let title = after[digits.len()..].trim().trim_start_matches('-').trim();
    if title.is_empty() { format!("S{season}, E{digits}") } else { format!("S{season}, E{digits}: {title}") }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `toDiscordActivity`. Strings are cut to Discord's 128 characters.
pub fn activity(p: &Presence, now: i64) -> Value {
    let cut = |s: &str| s.chars().take(128).collect::<String>();
    match p.kind.as_str() {
        "player" => {
            let episode = p.episode.as_deref().map(episode_label);
            let state = if p.playing { episode } else { Some(episode.map_or("Pausado".to_string(), |e| format!("{e} • Pausado"))) };
            let mut a = json!({ "type": WATCHING, "name": cut(&p.title), "details": cut(&p.title) });
            if let Some(state) = state {
                a["state"] = json!(cut(&state));
            }
            if p.playing {
                let start = now - p.position_ms / 1000;
                let mut ts = json!({ "start": start });
                if p.duration_ms > 0 {
                    ts["end"] = json!(start + p.duration_ms / 1000);
                }
                a["timestamps"] = ts;
            }
            if let Some(poster) = p.poster.as_deref().filter(|u| u.starts_with("https://")) {
                a["assets"] = json!({ "large_image": poster, "large_text": cut(&p.title) });
            }
            a
        }
        "details" => json!({ "type": WATCHING, "details": cut(&format!("Vendo {}", p.title)) }),
        "tab" => json!({ "type": WATCHING, "details": cut(&format!("Navegando em {}", p.title)) }),
        _ => json!({ "type": WATCHING, "details": "Navegando no Nuvio Astra" }),
    }
}

/// Two activities that look the same in Discord (a playing start a few seconds apart is the same:
/// the position moves with the clock).
fn same(a: &Value, b: &Value) -> bool {
    let start = |v: &Value| v.pointer("/timestamps/start").and_then(Value::as_i64);
    let strip = |v: &Value| {
        let mut v = v.clone();
        if let Some(ts) = v.get_mut("timestamps").and_then(Value::as_object_mut) {
            ts.remove("start");
            ts.remove("end");
        }
        v
    };
    strip(a) == strip(b) && match (start(a), start(b)) {
        (Some(x), Some(y)) => (x - y).abs() <= 3,
        (None, None) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------- the pipe

fn frame(opcode: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string().into_bytes();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

fn read_frame(pipe: &mut std::fs::File) -> std::io::Result<(u32, Value)> {
    let mut header = [0u8; 8];
    pipe.read_exact(&mut header)?;
    let opcode = u32::from_le_bytes(header[..4].try_into().expect("4 bytes"));
    let len = u32::from_le_bytes(header[4..].try_into().expect("4 bytes")) as usize;
    if len > 1_048_576 {
        return Err(std::io::Error::other("discord frame too large"));
    }
    let mut body = vec![0u8; len];
    pipe.read_exact(&mut body)?;
    Ok((opcode, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

/// Opens Discord's pipe and shakes hands (`connect`).
fn connect() -> Option<std::fs::File> {
    for i in 0..10 {
        let Ok(mut pipe) = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")) else { continue };
        if pipe.write_all(&frame(0, &json!({ "v": 1, "client_id": client_id() }))).is_err() {
            continue;
        }
        if let Ok((1, ready)) = read_frame(&mut pipe) {
            if ready.get("evt").and_then(Value::as_str) == Some("READY") {
                return Some(pipe);
            }
        }
    }
    None
}

/// `SET_ACTIVITY` (`null` clears it); false when Discord went away.
fn set_activity(pipe: &mut std::fs::File, activity: Option<&Value>) -> bool {
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let nonce = format!("astra-{}", NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let command = json!({ "cmd": "SET_ACTIVITY", "nonce": nonce, "args": { "pid": std::process::id(), "activity": activity } });
    if pipe.write_all(&frame(1, &command)).is_err() {
        return false;
    }
    matches!(read_frame(pipe), Ok((1, answer)) if answer.get("nonce").and_then(Value::as_str) == Some(nonce.as_str()))
}

// ---------------------------------------------------------------- the worker

enum Message {
    Presence(Presence),
    Enabled(bool),
}

fn worker() -> &'static Mutex<Sender<Message>> {
    static WORKER: OnceLock<Mutex<Sender<Message>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = channel::<Message>();
        std::thread::Builder::new()
            .name("nuvio-discord".into())
            .spawn(move || {
                let mut enabled = false;
                let mut wanted: Option<Presence> = None;
                let mut pipe: Option<std::fs::File> = None;
                let mut shown: Option<Value> = None;
                let mut last_try: Option<Instant> = None;
                loop {
                    match rx.recv_timeout(Duration::from_secs(2)) {
                        Ok(Message::Presence(p)) => wanted = Some(p),
                        Ok(Message::Enabled(on)) => {
                            enabled = on;
                            if !on {
                                if let Some(p) = pipe.as_mut() {
                                    let _ = set_activity(p, None);
                                }
                                pipe = None;
                                shown = None;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    if !enabled {
                        continue;
                    }
                    if pipe.is_none() {
                        if last_try.is_some_and(|t| t.elapsed() < RECONNECT) {
                            continue;
                        }
                        last_try = Some(Instant::now());
                        pipe = connect();
                        shown = None;
                    }
                    let Some(p) = pipe.as_mut() else { continue };
                    let next = activity(wanted.as_ref().unwrap_or(&Presence::default()), now_secs());
                    if shown.as_ref().is_some_and(|s| same(s, &next)) {
                        continue;
                    }
                    if set_activity(p, Some(&next)) {
                        shown = Some(next);
                    } else {
                        pipe = None;
                        shown = None;
                    }
                }
            })
            .expect("discord thread");
        Mutex::new(tx)
    })
}

// ---------------------------------------------------------------- settings and commands

#[derive(Serialize, Deserialize, Default)]
struct Stored {
    enabled: bool,
}

fn file(state: &crate::AppState) -> std::path::PathBuf {
    state.app_store.dir().join("discord.json")
}

fn enabled(state: &crate::AppState) -> bool {
    std::fs::read_to_string(file(state)).ok().and_then(|t| serde_json::from_str::<Stored>(&t).ok()).is_some_and(|s| s.enabled)
}

/// When the app opens: starts sending if it is on.
pub fn start(state: &crate::AppState) {
    if enabled(state) {
        let _ = worker().lock().expect("discord").send(Message::Enabled(true));
    }
}

#[tauri::command]
pub fn discord_enabled(state: State<'_, crate::AppState>) -> bool {
    enabled(&state)
}

/// Settings → Integrações → Discord (this computer).
#[tauri::command]
pub fn discord_set_enabled(state: State<'_, crate::AppState>, enabled: bool) -> Result<(), String> {
    let text = serde_json::to_string(&Stored { enabled }).map_err(|e| e.to_string())?;
    std::fs::write(file(&state), text).map_err(|e| e.to_string())?;
    let _ = worker().lock().expect("discord").send(Message::Enabled(enabled));
    Ok(())
}

/// What the interface shows now (sent on every change; repeats are dropped here).
#[tauri::command]
pub fn discord_presence(presence: Presence) {
    let _ = worker().lock().expect("discord").send(Message::Presence(presence));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episode_labels() {
        assert_eq!(episode_label("S1E2 - Piloto"), "S1, E2: Piloto");
        assert_eq!(episode_label("S10E3"), "S10, E3");
        assert_eq!(episode_label("Especial"), "Especial");
    }

    #[test]
    fn player_activity_with_progress_bar() {
        let p = Presence { kind: "player".into(), title: "Série".into(), episode: Some("S1E2 - Piloto".into()), poster: Some("https://img/p.jpg".into()), playing: true, position_ms: 60_000, duration_ms: 1_800_000 };
        let a = activity(&p, 10_000);
        assert_eq!(a["type"], 3);
        assert_eq!(a["state"], "S1, E2: Piloto");
        assert_eq!(a["timestamps"]["start"], 9_940);
        assert_eq!(a["timestamps"]["end"], 9_940 + 1_800);
        assert_eq!(a["assets"]["large_image"], "https://img/p.jpg");
        let paused = activity(&Presence { playing: false, ..p.clone() }, 10_000);
        assert_eq!(paused["state"], "S1, E2: Piloto • Pausado");
        assert!(paused.get("timestamps").is_none());
        let later = activity(&Presence { position_ms: 62_000, ..p }, 10_002);
        assert!(same(&a, &later), "the clock moving does not resend");
    }

    #[test]
    fn frames_are_little_endian() {
        let f = frame(1, &json!({ "a": 1 }));
        assert_eq!(&f[..4], &[1, 0, 0, 0]);
        assert_eq!(u32::from_le_bytes(f[4..8].try_into().unwrap()) as usize, f.len() - 8);
    }
}
