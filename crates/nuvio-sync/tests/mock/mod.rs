//! In-memory Supabase stand-in for contract tests: the GoTrue and PostgREST routes, parameters and
//! row shapes the original app uses (see `docs/ACCOUNT_SYNC.md`). Runs on 127.0.0.1.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub const EMAIL: &str = "ana@exemplo.test";
pub const PASSWORD: &str = "senha-forte-1";
pub const USER: &str = "user-a";

#[derive(Default)]
pub struct Mock {
    pub access: String,
    pub refresh: String,
    pub token_counter: u32,
    pub refresh_revoked: bool,
    /// The next REST call answers 401 "JWT expired".
    pub expire_next: bool,
    /// Connections are closed without an answer.
    pub down: bool,
    /// RPC names answering 500.
    pub failing: Vec<String>,
    pub calls: Vec<String>,
    pub event_id: i64,
    pub profiles: Vec<Value>,
    pub library: BTreeMap<(i64, String), Value>,
    pub progress: BTreeMap<(i64, String), Value>,
    pub watched: BTreeMap<(i64, String), Value>,
    /// (event id, profile, kind, op, row)
    pub events: Vec<(i64, i64, &'static str, &'static str, Value)>,
    pub addons: BTreeMap<i64, Vec<Value>>,
    /// `collections_json` per profile (`sync_pull_collections`); absent = no row.
    pub collections: BTreeMap<i64, Value>,
    /// `settings_json` per profile (`sync_pull_home_catalog_settings`); absent = no row.
    pub home: BTreeMap<i64, Value>,
    /// `settings_json` of the `desktop` blob per profile (`sync_pull_profile_settings_blob`).
    pub blobs: BTreeMap<i64, Value>,
    /// Rows of `sync_pull_provider_credentials` per profile.
    pub provider_credentials: BTreeMap<i64, Vec<Value>>,
    pub pins: BTreeMap<i64, String>,
    pub pin_failures: BTreeMap<i64, u32>,
    pub origins: Vec<String>,
}

impl Mock {
    fn tokens(&mut self) -> Value {
        self.token_counter += 1;
        self.access = format!("access-{}", self.token_counter);
        self.refresh = format!("refresh-{}", self.token_counter);
        json!({ "access_token": self.access, "refresh_token": self.refresh, "expires_in": 3600, "token_type": "bearer",
                "user": { "id": USER, "email": EMAIL } })
    }

    fn event(&mut self, profile: i64, kind: &'static str, op: &'static str, row: Value) {
        self.event_id += 1;
        let id = self.event_id;
        self.events.push((id, profile, kind, op, row));
    }

    /// A change made by another device (with its event).
    pub fn remote_library_upsert(&mut self, profile: i64, id: &str, name: &str) {
        let row = json!({ "content_id": id, "content_type": "movie", "name": name, "poster_shape": "POSTER", "genres": [], "added_at": 1000 });
        self.library.insert((profile, format!("movie/{id}")), row.clone());
        self.event(profile, "library", "upsert", row);
    }
    pub fn remote_library_delete(&mut self, profile: i64, id: &str) {
        if let Some(row) = self.library.remove(&(profile, format!("movie/{id}"))) {
            self.event(profile, "library", "delete", row);
        }
    }
    pub fn remote_progress(&mut self, profile: i64, key: &str, position: i64, last_watched: i64) {
        let row = json!({ "progress_key": key, "content_id": key, "content_type": "movie", "video_id": key, "position": position, "duration": 6000000, "last_watched": last_watched });
        self.progress.insert((profile, key.to_string()), row.clone());
        self.event(profile, "progress", "upsert", row);
    }

    fn cursor(&self, profile: i64, kind: &str) -> i64 {
        self.events.iter().filter(|e| e.1 == profile && e.2 == kind).map(|e| e.0).max().unwrap_or(0)
    }

    fn delta(&self, p: &Value, kind: &str) -> Value {
        let profile = p["p_profile_id"].as_i64().unwrap();
        let since = p["p_since_event_id"].as_i64().unwrap();
        let limit = p["p_limit"].as_i64().unwrap() as usize;
        Value::Array(
            self.events
                .iter()
                .filter(|e| e.1 == profile && e.2 == kind && e.0 > since)
                .take(limit)
                .map(|e| {
                    let mut row = e.4.clone();
                    row["event_id"] = json!(e.0);
                    row["operation"] = json!(e.3);
                    row
                })
                .collect(),
        )
    }

    fn rpc(&mut self, name: &str, p: &Value) -> Result<Value, (u16, Value)> {
        if let Some(o) = p.get("p_origin_client_id").and_then(Value::as_str) {
            self.origins.push(o.to_string());
        }
        let profile = p.get("p_profile_id").and_then(Value::as_i64).unwrap_or(0);
        Ok(match name {
            "sync_pull_profiles" => Value::Array(self.profiles.clone()),
            "sync_push_profiles" => {
                let incoming = p["p_profiles"].as_array().unwrap().clone();
                let old = std::mem::take(&mut self.profiles);
                self.profiles = incoming
                    .into_iter()
                    .map(|mut row| {
                        let idx = row["profile_index"].as_i64().unwrap();
                        let prev = old.iter().find(|o| o["profile_index"] == idx);
                        row["pin_enabled"] = json!(self.pins.contains_key(&idx));
                        row["updated_at"] = prev.and_then(|o| o.get("updated_at").cloned()).unwrap_or(json!("2026-01-01T00:00:00Z"));
                        row
                    })
                    .collect();
                Value::Null
            }
            "sync_delete_profile_data" => {
                self.profiles.retain(|r| r["profile_index"].as_i64() != Some(profile));
                self.library.retain(|k, _| k.0 != profile);
                self.progress.retain(|k, _| k.0 != profile);
                self.watched.retain(|k, _| k.0 != profile);
                self.addons.remove(&profile);
                Value::Null
            }
            "sync_get_library_delta_cursor" => json!(self.cursor(profile, "library")),
            "sync_pull_library" => {
                let (limit, offset) = (p["p_limit"].as_i64().unwrap() as usize, p["p_offset"].as_i64().unwrap() as usize);
                Value::Array(self.library.iter().filter(|(k, _)| k.0 == profile).map(|(_, v)| v.clone()).skip(offset).take(limit).collect())
            }
            "sync_pull_library_delta" => self.delta(p, "library"),
            "sync_push_library_items" => {
                for row in p["p_items"].as_array().unwrap().clone() {
                    let key = format!("{}/{}", row["content_type"].as_str().unwrap(), row["content_id"].as_str().unwrap());
                    self.library.insert((profile, key), row.clone());
                    self.event(profile, "library", "upsert", row);
                }
                Value::Null
            }
            "sync_delete_library_items" => {
                for k in p["p_keys"].as_array().unwrap().clone() {
                    let key = format!("{}/{}", k["content_type"].as_str().unwrap(), k["content_id"].as_str().unwrap());
                    if let Some(row) = self.library.remove(&(profile, key)) {
                        self.event(profile, "library", "delete", row);
                    }
                }
                Value::Null
            }
            "sync_get_watch_progress_delta_cursor" => json!(self.cursor(profile, "progress")),
            "sync_pull_watch_progress" => Value::Array(self.progress.iter().filter(|(k, _)| k.0 == profile).map(|(_, v)| v.clone()).collect()),
            "sync_pull_watch_progress_delta" => self.delta(p, "progress"),
            "sync_push_watch_progress" => {
                for row in p["p_entries"].as_array().unwrap().clone() {
                    let key = row["progress_key"].as_str().unwrap().to_string();
                    self.progress.insert((profile, key), row.clone());
                    self.event(profile, "progress", "upsert", row);
                }
                Value::Null
            }
            "sync_delete_watch_progress" => {
                for k in p["p_keys"].as_array().unwrap().clone() {
                    if let Some(row) = self.progress.remove(&(profile, k.as_str().unwrap().to_string())) {
                        self.event(profile, "progress", "delete", row);
                    }
                }
                Value::Null
            }
            "sync_get_watched_items_delta_cursor" => json!(self.cursor(profile, "watched")),
            "sync_pull_watched_items" => {
                let (page, size) = (p["p_page"].as_i64().unwrap() as usize, p["p_page_size"].as_i64().unwrap() as usize);
                assert!(page >= 1, "pages start at 1");
                Value::Array(self.watched.iter().filter(|(k, _)| k.0 == profile).map(|(_, v)| v.clone()).skip((page - 1) * size).take(size).collect())
            }
            "sync_pull_watched_items_delta" => self.delta(p, "watched"),
            "sync_push_watched_items" => {
                for row in p["p_items"].as_array().unwrap().clone() {
                    let key = format!("{}:{}:{}", row["content_id"].as_str().unwrap(), row["season"], row["episode"]);
                    self.watched.insert((profile, key), row.clone());
                    self.event(profile, "watched", "upsert", row);
                }
                Value::Null
            }
            "sync_delete_watched_items" => {
                for k in p["p_keys"].as_array().unwrap().clone() {
                    let key = format!("{}:{}:{}", k["content_id"].as_str().unwrap(), k["season"], k["episode"]);
                    if let Some(row) = self.watched.remove(&(profile, key)) {
                        self.event(profile, "watched", "delete", row);
                    }
                }
                Value::Null
            }
            "sync_pull_collections" => match self.collections.get(&profile) {
                Some(c) => json!([{ "profile_id": profile, "collections_json": c, "updated_at": "2026-11-22T00:00:00Z" }]),
                None => json!([]),
            },
            "sync_pull_home_catalog_settings" => {
                assert_eq!(p["p_platform"], "home_catalog_shared");
                match self.home.get(&profile) {
                    Some(h) => json!([{ "profile_id": profile, "settings_json": h }]),
                    None => json!([]),
                }
            }
            "sync_push_home_catalog_settings" => {
                assert_eq!(p["p_platform"], "home_catalog_shared");
                assert!(p["p_origin_client_id"].is_string() && p["p_settings_json"]["items"].is_array());
                self.home.insert(profile, p["p_settings_json"].clone());
                Value::Null
            }
            "sync_pull_profile_settings_blob" => {
                assert_eq!(p["p_platform"], "desktop");
                match self.blobs.get(&profile) {
                    Some(b) => json!([{ "profile_id": profile, "settings_json": b }]),
                    None => json!([]),
                }
            }
            "sync_push_profile_settings_blob" => {
                assert_eq!(p["p_platform"], "desktop");
                assert!(p["p_origin_client_id"].is_string());
                self.blobs.insert(profile, p["p_settings_json"].clone());
                Value::Null
            }
            "sync_pull_provider_credentials" => {
                Value::Array(self.provider_credentials.get(&profile).cloned().unwrap_or_default())
            }
            "sync_push_provider_credentials" => {
                assert!(p["p_origin_client_id"].is_string());
                self.provider_credentials.insert(profile, p["p_credentials"].as_array().unwrap().clone());
                Value::Null
            }
            "sync_push_addons" => {
                self.addons.insert(profile, p["p_addons"].as_array().unwrap().clone());
                Value::Null
            }
            "verify_profile_pin" => {
                let fails = self.pin_failures.get(&profile).copied().unwrap_or(0);
                if fails >= 3 {
                    return Ok(json!([{ "unlocked": false, "retry_after_seconds": 30 }]));
                }
                let ok = self.pins.get(&profile).map(String::as_str) == p["p_pin"].as_str();
                if !ok {
                    self.pin_failures.insert(profile, fails + 1);
                }
                json!([{ "unlocked": ok, "retry_after_seconds": 0 }])
            }
            "set_profile_pin" => {
                if let Some(current) = self.pins.get(&profile) {
                    if p.get("p_current_pin").and_then(Value::as_str) != Some(current.as_str()) {
                        return Err((400, json!({ "message": "PIN atual incorreto", "code": "P0001" })));
                    }
                }
                self.pins.insert(profile, p["p_pin"].as_str().unwrap().to_string());
                for r in self.profiles.iter_mut().filter(|r| r["profile_index"] == profile) {
                    r["pin_enabled"] = json!(true);
                    r["updated_at"] = json!("2026-02-02T00:00:00Z");
                }
                Value::Null
            }
            "clear_profile_pin" => {
                self.pins.remove(&profile);
                for r in self.profiles.iter_mut().filter(|r| r["profile_index"] == profile) {
                    r["pin_enabled"] = json!(false);
                }
                Value::Null
            }
            "get_avatar_catalog" => json!([
                { "id": "avatar_b", "display_name": "B", "storage_path": "b.png", "category": "character", "sort_order": 2, "is_active": true },
                { "id": "avatar_a", "display_name": "A", "storage_path": "a.png", "category": "character", "sort_order": 1, "is_active": true, "bg_color": "#112233" },
                { "id": "avatar_old", "display_name": "Old", "storage_path": "o.png", "category": "character", "sort_order": 0, "is_active": false }
            ]),
            other => return Err((404, json!({ "message": format!("function {other} not found"), "code": "PGRST202" }))),
        })
    }

    fn handle(&mut self, method: &str, path: &str, auth: &str, body: &Value) -> (u16, Value) {
        self.calls.push(path.split('?').next().unwrap_or(path).trim_start_matches("/rest/v1/rpc/").to_string());
        if path.starts_with("/auth/v1/token?grant_type=password") {
            return if body["email"] == EMAIL && body["password"] == PASSWORD {
                self.refresh_revoked = false;
                (200, self.tokens())
            } else {
                (400, json!({ "code": 400, "error_code": "invalid_credentials", "msg": "Invalid login credentials" }))
            };
        }
        if path.starts_with("/auth/v1/token?grant_type=refresh_token") {
            return if !self.refresh_revoked && body["refresh_token"] == self.refresh.as_str() {
                (200, self.tokens())
            } else {
                (400, json!({ "error_code": "refresh_token_not_found", "msg": "Invalid Refresh Token" }))
            };
        }
        if path.starts_with("/auth/v1/signup") {
            return (200, json!({ "id": "user-new", "email": body["email"], "confirmation_sent_at": "now" }));
        }
        if path.starts_with("/auth/v1/logout") {
            return (204, Value::Null);
        }
        // REST: the user's token is required.
        if auth != format!("Bearer {}", self.access) && !path.contains("get_avatar_catalog") {
            return (401, json!({ "code": "PGRST301", "message": "JWT invalid" }));
        }
        if std::mem::take(&mut self.expire_next) {
            return (401, json!({ "code": "PGRST303", "message": "JWT expired" }));
        }
        if let Some(name) = path.strip_prefix("/rest/v1/rpc/") {
            if self.failing.iter().any(|f| f == name) {
                return (500, json!({ "message": "internal error" }));
            }
            return match self.rpc(name, body) {
                Ok(v) => (200, v),
                Err(e) => e,
            };
        }
        if method == "GET" && path.starts_with("/rest/v1/addons?") {
            let profile: i64 = path.split("profile_id=eq.").nth(1).and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok()).unwrap();
            assert!(path.contains("order=sort_order.asc"));
            let mut rows = self.addons.get(&profile).cloned().unwrap_or_default();
            rows.sort_by_key(|r| r["sort_order"].as_i64().unwrap_or(0));
            return (200, Value::Array(rows));
        }
        (404, json!({ "message": "not found" }))
    }
}

pub fn start() -> (String, Arc<Mutex<Mock>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mock = Arc::new(Mutex::new(Mock::default()));
    let m = mock.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let m = m.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let (mut len, mut auth, mut apikey) = (0usize, String::new(), String::new());
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                    let lower = h.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    if lower.starts_with("authorization:") {
                        auth = h["authorization:".len()..].trim().to_string();
                    }
                    if lower.starts_with("apikey:") {
                        apikey = h["apikey:".len()..].trim().to_string();
                    }
                }
                let mut body = vec![0u8; len];
                let _ = reader.read_exact(&mut body);
                let mut parts = line.split_whitespace();
                let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                let (status, value) = {
                    let mut mock = m.lock().unwrap();
                    if mock.down {
                        return; // connection dropped without an answer
                    }
                    assert!(!apikey.is_empty(), "apikey header on every request");
                    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    mock.handle(&method, &path, &auth, &body)
                };
                let text = if value.is_null() && status == 204 { String::new() } else { value.to_string() };
                let mut out = stream;
                let _ = write!(out, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
            });
        }
    });
    (base, mock)
}
