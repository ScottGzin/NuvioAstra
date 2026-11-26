//! Discord Rich Presence (the original's `DiscordIpcClient` / `DiscordPresenceManager`).
//!
//! The public client id comes from NuvioDesktop's `build.gradle.kts` and is not a secret. Activity
//! is sent over Discord's local IPC socket (or named pipe on Windows). Failures are silent: the
//! app keeps working when Discord is not running.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const CLIENT_ID: &str = "1538974392376369212";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Activity {
    /// Media title (`name` and `details` in the original).
    pub title: String,
    /// Episode label or "Browsing X" / "Viewing X".
    pub state: Option<String>,
    pub poster: Option<String>,
    pub playing: bool,
    /// Playback position and duration in milliseconds (Unix start/end timestamps are derived).
    pub position_ms: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[cfg(unix)]
type Connection = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Connection = std::fs::File;

#[cfg(unix)]
fn connect() -> Option<Connection> {
    for base in ["XDG_RUNTIME_DIR", "TMPDIR"] {
        if let Ok(dir) = std::env::var(base) {
            for i in 0..10 {
                let path = PathBuf::from(&dir).join(format!("discord-ipc-{i}"));
                if path.exists() {
                    if let Ok(stream) = Connection::connect(&path) { return Some(stream); }
                }
            }
        }
    }
    for dir in ["/tmp", "/var/run"] {
        for i in 0..10 {
            let path = PathBuf::from(dir).join(format!("discord-ipc-{i}"));
            if path.exists() {
                if let Ok(stream) = Connection::connect(&path) { return Some(stream); }
            }
        }
    }
    None
}

#[cfg(windows)]
fn connect() -> Option<Connection> {
    for i in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{i}");
        if let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) {
            return Some(file);
        }
    }
    None
}

fn frame(opcode: u32, payload: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload.as_bytes());
    out
}

fn write_frame(connection: &mut Connection, opcode: u32, payload: &str) -> Result<(), String> {
    connection.write_all(&frame(opcode, payload)).map_err(|e| e.to_string())?;
    connection.flush().map_err(|e| e.to_string())
}

fn read_frame(connection: &mut Connection) -> Result<(), String> {
    let mut header = [0u8; 8];
    connection.read_exact(&mut header).map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(header[4..8].try_into().expect("4 bytes")) as usize;
    let mut payload = vec![0u8; length.min(1 << 20)];
    if length > 0 {
        connection.read_exact(&mut payload).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One reusable IPC connection. Kept behind a mutex so a stale socket can be dropped and reopened.
#[derive(Default)]
pub struct DiscordRpc {
    connection: Mutex<Option<Connection>>,
    nonce: AtomicU64,
}

impl DiscordRpc {
    pub fn new() -> Self { Self::default() }

    fn with_connection<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, String>) -> Result<T, String> {
        let mut guard = self.connection.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            let mut connection = connect().ok_or("Discord is not running")?;
            write_frame(&mut connection, 0, &format!(r#"{{"v":1,"client_id":"{CLIENT_ID}"}}"#))?;
            read_frame(&mut connection)?; // READY
            *guard = Some(connection);
        }
        let result = f(guard.as_mut().expect("connection"))?;
        Ok(result)
    }

    fn drop_connection(&self) {
        if let Ok(mut guard) = self.connection.lock() { *guard = None; }
    }

    /// `DiscordPresenceManager.setActivity`: type 3 = "Watching". A `None` activity clears it.
    pub fn publish(&self, activity: Option<&Activity>) -> Result<(), String> {
        let payload = match activity {
            None => serde_json::json!({
                "cmd": "SET_ACTIVITY",
                "args": { "pid": std::process::id(), "activity": null },
                "nonce": self.next_nonce(),
            }),
            Some(a) => {
                let timestamps = if a.playing { a.position_ms.map(|start| {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
                    let begin = now - a.position_ms.unwrap_or(0).max(0);
                    let end = a.duration_ms.filter(|d| *d > 0).map(|d| begin + d / 1000);
                    serde_json::json!({ "start": begin / 1000, "end": end })
                }) } else { None };
                let mut json = serde_json::json!({
                    "type": 3,
                    "name": a.title,
                    "details": a.title,
                    "state": a.state,
                    "timestamps": timestamps,
                });
                if let Some(poster) = &a.poster {
                    json["assets"] = serde_json::json!({ "large_image": poster, "large_text": a.title });
                }
                serde_json::json!({
                    "cmd": "SET_ACTIVITY",
                    "args": { "pid": std::process::id(), "activity": json },
                    "nonce": self.next_nonce(),
                })
            }
        };
        // A command that fails closes the socket; the next publish reconnects.
        if self.with_connection(|connection| write_frame(connection, 1, &payload.to_string())).is_err() {
            self.drop_connection();
        }
        Ok(())
    }

    fn next_nonce(&self) -> String {
        let n = self.nonce.fetch_add(1, Ordering::Relaxed);
        format!("{}-{n}-{}", std::process::id(), n)
    }
}

/// The single IPC connection of the process; no AppState wiring is needed.
static RPC: LazyLock<DiscordRpc> = LazyLock::new(DiscordRpc::new);

/// Publishes activity when the setting is on. An absent activity clears the presence.
pub fn publish(activity: Option<&Activity>) {
    let _ = RPC.publish(activity);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_little_endian_and_activity_serializes() {
        let bytes = frame(1, "{}");
        assert_eq!(&bytes[0..4], &1u32.to_le_bytes());
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        let activity = Activity { title: "Title".into(), state: Some("S1, E2".into()), poster: None, playing: true, position_ms: None, duration_ms: None };
        let value = serde_json::to_value(activity).unwrap();
        assert_eq!(value["title"], "Title");
        assert_eq!(value["state"], "S1, E2");
    }
}
