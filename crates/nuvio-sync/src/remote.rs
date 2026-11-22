//! HTTP access to the Nuvio account backend (Supabase), with the same endpoints the original reaches
//! through supabase-kt: GoTrue (`/auth/v1/...`) and PostgREST (`/rest/v1/rpc/<function>`,
//! `/rest/v1/addons`). Every request carries the public `apikey` (role `anon`) and, when signed in,
//! the user's access token. On a connection failure the request is retried once on the fallback
//! address (`SupabaseEndpointConfig`). Tokens never appear in URLs, logs or error texts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub url: String,
    pub fallback_url: Option<String>,
    /// Public key (role `anon`). Never a service key: `ServerConfig::new` refuses those.
    pub anon_key: String,
}

impl ServerConfig {
    /// `None` when not configured or when the key is not a public one.
    pub fn new(url: &str, fallback_url: Option<&str>, anon_key: &str) -> Option<Self> {
        let url = url.trim().trim_end_matches('/');
        let key = anon_key.trim();
        if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1")) || key.is_empty() || !is_public_key(key) {
            return None;
        }
        Some(Self {
            url: url.to_string(),
            fallback_url: fallback_url.map(|f| f.trim().trim_end_matches('/').to_string()).filter(|f| f.starts_with("https://") && f != url),
            anon_key: key.to_string(),
        })
    }
}

/// `sb_publishable_…` or a JWT whose `role` claim is `anon`.
pub fn is_public_key(key: &str) -> bool {
    if key.starts_with("sb_publishable_") {
        return true;
    }
    if key.starts_with("sb_secret_") {
        return false;
    }
    let Some(payload) = key.split('.').nth(1) else { return false };
    let Some(bytes) = base64url_decode(payload) else { return false };
    serde_json::from_slice::<Value>(&bytes).ok().and_then(|v| v.get("role").and_then(|r| r.as_str()).map(|r| r == "anon")).unwrap_or(false)
}

fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => continue,
            _ => return None,
        } as u32;
        bits = (bits << 6) | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    Some(out)
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RemoteError {
    #[error("no connection to the account server")]
    Offline,
    #[error("expired or invalid session")]
    Unauthorized,
    #[error("{0}")]
    Auth(String),
    #[error("the account server returned error {status}: {message}")]
    Http { status: u16, message: String },
    #[error("unexpected response from the account server")]
    Decode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub user_id: String,
    pub email: Option<String>,
}

impl Session {
    pub fn expires_soon(&self, now_s: i64) -> bool {
        self.expires_at - now_s < 60
    }
}

pub fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Clone)]
pub struct Remote {
    http: reqwest::Client,
    pub config: ServerConfig,
}

/// Outcome of a sign-up: a session, or an account waiting for e-mail confirmation.
pub enum SignUp {
    Session(Session),
    ConfirmEmail,
}

impl Remote {
    pub fn new(config: ServerConfig) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(45))
            .user_agent(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP client");
        Self { http, config }
    }

    async fn send(&self, method: reqwest::Method, path_and_query: &str, bearer: Option<&str>, body: Option<&Value>, extra: &[(&str, &str)]) -> Result<Value, RemoteError> {
        let mut bases = vec![self.config.url.clone()];
        bases.extend(self.config.fallback_url.clone());
        let mut last = RemoteError::Offline;
        for base in bases {
            let mut req = self
                .http
                .request(method.clone(), format!("{base}{path_and_query}"))
                .header("apikey", &self.config.anon_key)
                .header("Authorization", format!("Bearer {}", bearer.unwrap_or(&self.config.anon_key)))
                .header("Accept", "application/json");
            for (k, v) in extra {
                req = req.header(*k, *v);
            }
            if let Some(b) = body {
                req = req.json(b);
            }
            let response = match req.send().await {
                Ok(r) => r,
                Err(e) if e.is_connect() || e.is_timeout() => {
                    last = RemoteError::Offline;
                    continue; // fallback address, as the original
                }
                Err(_) => return Err(RemoteError::Offline),
            };
            let status = response.status().as_u16();
            let text = response.text().await.map_err(|_| RemoteError::Offline)?;
            let value: Value = if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::String(String::new())) };
            if (200..300).contains(&status) {
                return Ok(value);
            }
            return Err(error_of(status, &value, path_and_query.starts_with("/auth/")));
        }
        Err(last)
    }

    // ---------------- GoTrue

    /// `signInWith(Email)`.
    pub async fn sign_in(&self, email: &str, password: &str) -> Result<Session, RemoteError> {
        let body = serde_json::json!({ "email": email, "password": password });
        let v = self.send(reqwest::Method::POST, "/auth/v1/token?grant_type=password", None, Some(&body), &[]).await?;
        session_of(&v).ok_or(RemoteError::Decode)
    }

    /// `signUpWith(Email)`: a session when the server does not require e-mail confirmation.
    pub async fn sign_up(&self, email: &str, password: &str) -> Result<SignUp, RemoteError> {
        let body = serde_json::json!({ "email": email, "password": password });
        let v = self.send(reqwest::Method::POST, "/auth/v1/signup", None, Some(&body), &[]).await?;
        Ok(match session_of(&v) {
            Some(s) => SignUp::Session(s),
            None => SignUp::ConfirmEmail,
        })
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<Session, RemoteError> {
        let body = serde_json::json!({ "refresh_token": refresh_token });
        match self.send(reqwest::Method::POST, "/auth/v1/token?grant_type=refresh_token", None, Some(&body), &[]).await {
            Ok(v) => session_of(&v).ok_or(RemoteError::Decode),
            // A refresh token the server refuses means the session is over.
            Err(RemoteError::Auth(_)) | Err(RemoteError::Http { status: 400..=403, .. }) => Err(RemoteError::Unauthorized),
            Err(e) => Err(e),
        }
    }

    /// Ends this device's session on the server (local scope); errors are ignored by the caller.
    pub async fn sign_out(&self, access_token: &str) -> Result<(), RemoteError> {
        self.send(reqwest::Method::POST, "/auth/v1/logout?scope=local", Some(access_token), None, &[]).await.map(|_| ())
    }

    // ---------------- PostgREST

    pub async fn rpc(&self, function: &str, access_token: &str, params: &Value) -> Result<Value, RemoteError> {
        self.send(reqwest::Method::POST, &format!("/rest/v1/rpc/{function}"), Some(access_token), Some(params), &[("Content-Type", "application/json")]).await
    }

    /// `rpc` for functions without parameters.
    pub async fn rpc0(&self, function: &str, access_token: &str) -> Result<Value, RemoteError> {
        self.rpc(function, access_token, &serde_json::json!({})).await
    }

    /// `from("addons").select { eq("profile_id", p); order("sort_order") }`.
    pub async fn addon_rows(&self, access_token: &str, profile: i64) -> Result<Value, RemoteError> {
        self.send(reqwest::Method::GET, &format!("/rest/v1/addons?select=url,name,enabled,sort_order&profile_id=eq.{profile}&order=sort_order.asc"), Some(access_token), None, &[]).await
    }
}

fn session_of(v: &Value) -> Option<Session> {
    let access_token = v.get("access_token")?.as_str()?.to_string();
    let refresh_token = v.get("refresh_token")?.as_str()?.to_string();
    let expires_at = v.get("expires_at").and_then(Value::as_i64).unwrap_or_else(|| now_s() + v.get("expires_in").and_then(Value::as_i64).unwrap_or(3600));
    let user = v.get("user")?;
    Some(Session {
        access_token,
        refresh_token,
        expires_at,
        user_id: user.get("id")?.as_str()?.to_string(),
        email: user.get("email").and_then(Value::as_str).map(str::to_string),
    })
}

/// Server error → message for the user (pt-BR, never containing tokens).
fn error_of(status: u16, body: &Value, auth: bool) -> RemoteError {
    let text = ["error_description", "msg", "message", "error"].iter().find_map(|k| body.get(*k).and_then(Value::as_str)).unwrap_or("").to_string();
    let code = body.get("error_code").or_else(|| body.get("code")).and_then(Value::as_str).unwrap_or("").to_string();
    if auth {
        let lower = format!("{code} {text}").to_lowercase();
        let msg = if lower.contains("invalid_credentials") || lower.contains("invalid login credentials") {
            "Incorrect email or password."
        } else if lower.contains("email_not_confirmed") || lower.contains("email not confirmed") {
            "Confirm your email before signing in (check the message sent by Nuvio)."
        } else if lower.contains("user_already_exists") || lower.contains("already registered") {
            "An account with this email already exists."
        } else if lower.contains("weak_password") || lower.contains("password should") {
            "Weak password: use at least 6 characters."
        } else if lower.contains("over_request_rate_limit") || status == 429 {
            "Too many attempts. Wait a moment and try again."
        } else if lower.contains("signup_disabled") || lower.contains("signups not allowed") {
            "The server is not accepting new registrations."
        } else if status == 401 || status == 403 {
            return RemoteError::Unauthorized;
        } else {
            return RemoteError::Http { status, message: text };
        };
        return RemoteError::Auth(msg.to_string());
    }
    if status == 401 || code == "PGRST301" || code == "PGRST303" || text.to_lowercase().contains("jwt expired") {
        return RemoteError::Unauthorized;
    }
    RemoteError::Http { status, message: text }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(role: &str) -> String {
        fn b64(s: &str) -> String {
            const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let bytes = s.as_bytes();
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
                let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
                for i in 0..=chunk.len() {
                    out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
                }
            }
            out
        }
        format!("{}.{}.sig", b64(r#"{"alg":"HS256"}"#), b64(&format!(r#"{{"role":"{role}","iss":"supabase"}}"#)))
    }

    #[test]
    fn only_public_keys_are_accepted() {
        assert!(is_public_key(&jwt("anon")));
        assert!(!is_public_key(&jwt("service_role")));
        assert!(is_public_key("sb_publishable_abc"));
        assert!(!is_public_key("sb_secret_abc"));
        assert!(ServerConfig::new("https://x.test/", None, &jwt("service_role")).is_none());
        assert!(ServerConfig::new("http://evil.test", None, &jwt("anon")).is_none(), "https only");
        assert_eq!(ServerConfig::new("https://x.test/", Some("https://y.test"), &jwt("anon")).unwrap().url, "https://x.test");
    }

    #[test]
    fn auth_errors_become_portuguese_messages() {
        let e = error_of(400, &serde_json::json!({ "error_code": "invalid_credentials", "msg": "Invalid login credentials" }), true);
        assert_eq!(e, RemoteError::Auth("Incorrect email or password.".into()));
        assert_eq!(error_of(401, &serde_json::json!({ "code": "PGRST303", "message": "JWT expired" }), false), RemoteError::Unauthorized);
    }
}
