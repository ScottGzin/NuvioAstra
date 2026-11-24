//! Native host functions exposed to plugin scripts.
//!
//! The two important ones are `fetch` and the DOM bridge. Both are deliberately narrow: a plugin
//! can only fetch public http(s) URLs (IPs are validated before connecting and again on every
//! redirect) and query HTML with CSS selectors. There is no filesystem, shell, account or addon
//! access from a script.

use aes::{Aes128, Aes192, Aes256};
use base64::Engine as _;
use cbc::{Decryptor, Encryptor};
use cipher::block_padding::Pkcs7;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use md5::Md5;
use pbkdf2::pbkdf2_hmac;
use rquickjs::{function::Async, prelude::Func, Ctx, Error as JsError, Result as JsResult};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::Url;

const MAX_RESPONSE_TEXT: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 20 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const MAX_HEADER_VALUE: usize = 8 * 1024;

#[derive(Default)]
pub struct HostState {
    pub captured: Mutex<Option<String>>,
    pub log: Mutex<String>,
}

pub type SharedHost = Arc<HostState>;

/// `127.0.0.1`, private ranges, CGNAT, link-local and similar targets must never be reachable
/// from a plugin: the host process holds debrid keys and the Nuvio session.
pub fn forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || (v4.octets()[0] == 100 && (64..=127).contains(&v4.octets()[1]))
                || (v4.octets()[0] == 198 && (18..=19).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || v6.is_unicast_link_local() || v6.is_multicast() || v6.is_unique_local()
        }
    }
}

fn local_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".internal")
}

/// Validates a target and returns the public IPs it resolves to. Uses the same DoH-aware resolver
/// as addons, so a poisoned DNS answer cannot smuggle a private address past the check.
pub async fn validate_target(raw: &str) -> Option<(Url, String, Vec<IpAddr>)> {
    let url = Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_string();
    if local_host(&host) {
        return None;
    }
    let addresses: Vec<IpAddr> = nuvio_core::net::resolve_host(&host)
        .await
        .ok()?
        .into_iter()
        .map(|addr| addr.ip())
        .filter(|ip| !forbidden_ip(*ip))
        .collect();
    (!addresses.is_empty()).then_some((url, host, addresses))
}

fn failed_fetch(url: &str, reason: &str) -> String {
    json!({
        "ok": false, "status": 0, "statusText": reason, "url": url,
        "body": "", "bodyBase64": "", "headers": {}
    })
    .to_string()
}

fn parse_headers(raw: &str) -> BTreeMap<String, String> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw) else { return BTreeMap::new() };
    map.into_iter()
        .filter_map(|(key, value)| {
            let text = match value {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => return None,
            };
            let key = key.trim();
            if key.is_empty() || matches!(key.to_ascii_lowercase().as_str(), "host" | "connection" | "content-length" | "proxy-authorization") {
                return None;
            }
            Some((key.to_string(), text.chars().take(MAX_HEADER_VALUE).collect()))
        })
        .collect()
}

fn decode_body(kind: &str, body: &str) -> Vec<u8> {
    match kind {
        "base64" => base64::engine::general_purpose::STANDARD.decode(body.trim()).unwrap_or_default(),
        _ => body.as_bytes().to_vec(),
    }
}

async fn native_fetch(state: SharedHost, raw_url: String, method: String, headers_json: String, body_kind: String, body: String) -> JsResult<String> {
    let mut url = match validate_target(&raw_url).await {
        Some((url, _, _)) => url,
        None => return Ok(failed_fetch(&raw_url, "blocked by sandbox")),
    };
    let headers = parse_headers(&headers_json);
    let body_bytes = if body_kind == "none" { Vec::new() } else { decode_body(&body_kind, &body) };
    let mut verb = if method.trim().is_empty() { "GET".to_string() } else { method.trim().to_ascii_uppercase() };

    for _ in 0..=MAX_REDIRECTS {
        let Some((validated, host, addresses)) = validate_target(url.as_str()).await else {
            return Ok(failed_fetch(url.as_str(), "blocked by sandbox"));
        };
        url = validated;
        let mut builder = match nuvio_core::net::http_client_builder() {
            Ok(builder) => builder.redirect(reqwest::redirect::Policy::none()),
            Err(_) => return Ok(failed_fetch(url.as_str(), "http client unavailable")),
        };
        for ip in addresses {
            builder = builder.resolve(&host, SocketAddr::new(ip, 0));
        }
        let client = match builder.build() {
            Ok(client) => client,
            Err(_) => return Ok(failed_fetch(url.as_str(), "http client unavailable")),
        };
        let mut request = client.request(verb.parse().unwrap_or(reqwest::Method::GET), url.clone());
        let has_agent = headers.keys().any(|name| name.eq_ignore_ascii_case("user-agent"));
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        if !has_agent {
            request = request.header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36");
        }
        request = request.timeout(Duration::from_secs(20));
        if !body_bytes.is_empty() && verb != "GET" && verb != "HEAD" {
            request = request.body(body_bytes.clone());
        }
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(error) => return Ok(failed_fetch(url.as_str(), &error.to_string())),
        };
        let status = response.status();
        if status.is_redirection() {
            let target = response.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).and_then(|v| url.join(v).ok());
            if let Some(target) = target {
                if status == reqwest::StatusCode::PERMANENT_REDIRECT || status == reqwest::StatusCode::TEMPORARY_REDIRECT {
                    // keep the method and body for 307/308
                } else {
                    verb = "GET".into();
                }
                url = target;
                continue;
            }
        }

        let status_text = status.canonical_reason().unwrap_or("").to_string();
        let response_headers: BTreeMap<String, String> = response
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str().to_ascii_lowercase(), value.to_str().unwrap_or_default().chars().take(MAX_HEADER_VALUE).collect()))
            .collect();
        let final_url = response.url().to_string();
        let mut bytes = Vec::new();
        while let Ok(Some(chunk)) = response.chunk().await {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        let text_bytes = bytes.len().min(MAX_RESPONSE_TEXT);
        let text = String::from_utf8_lossy(&bytes[..text_bytes]).into_owned();
        let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let mut state = state.log.lock().expect("plugin log");
        state.push_str(&format!("fetch {status} {final_url}\n"));
        drop(state);
        return Ok(json!({
            "ok": status.is_success(), "status": status.as_u16(), "statusText": status_text,
            "url": final_url, "body": text, "bodyBase64": base64, "headers": response_headers,
        })
        .to_string());
    }
    Ok(failed_fetch(url.as_str(), "too many redirects"))
}

/// `__native_dom(html, selector)` → JSON array of `{ html, text, attrs }`, driven by `scraper`.
fn native_dom(html: String, selector: String) -> JsResult<String> {
    let Ok(parsed) = scraper::Selector::parse(&selector) else { return Ok("[]".into()) };
    let document = scraper::Html::parse_document(&html);
    let nodes: Vec<Value> = document
        .select(&parsed)
        .map(|element| {
            let attrs: BTreeMap<String, String> = element.value().attrs().map(|(key, value)| (key.to_string(), value.to_string())).collect();
            json!({ "html": element.html(), "text": element.text().collect::<String>(), "attrs": attrs })
        })
        .collect();
    serde_json::to_string(&nodes).map_err(|_| JsError::Exception)
}

// ---------------------------------------------------------------- crypto-js bridge

fn hex_decode(text: &str) -> Vec<u8> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return Vec::new();
    }
    (0..text.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

fn aes_decrypt(cipher: &[u8], key: &[u8], iv: &[u8]) -> Option<Vec<u8>> {
    if iv.len() != 16 {
        return None;
    }
    match key.len() {
        16 => Decryptor::<Aes128>::new_from_slices(key, iv).ok()?.decrypt_padded_vec_mut::<Pkcs7>(cipher).ok(),
        24 => Decryptor::<Aes192>::new_from_slices(key, iv).ok()?.decrypt_padded_vec_mut::<Pkcs7>(cipher).ok(),
        32 => Decryptor::<Aes256>::new_from_slices(key, iv).ok()?.decrypt_padded_vec_mut::<Pkcs7>(cipher).ok(),
        _ => None,
    }
}

fn aes_encrypt(plain: &[u8], key: &[u8], iv: &[u8]) -> Option<Vec<u8>> {
    if iv.len() != 16 {
        return None;
    }
    match key.len() {
        16 => Encryptor::<Aes128>::new_from_slices(key, iv).ok()?.encrypt_padded_vec_mut::<Pkcs7>(plain).into(),
        24 => Encryptor::<Aes192>::new_from_slices(key, iv).ok()?.encrypt_padded_vec_mut::<Pkcs7>(plain).into(),
        32 => Encryptor::<Aes256>::new_from_slices(key, iv).ok()?.encrypt_padded_vec_mut::<Pkcs7>(plain).into(),
        _ => None,
    }
}

/// Synchronous primitives for the `crypto-js` shim. Inputs and outputs are hex, so nothing opaque
/// crosses the JS/Rust boundary. An empty answer means the plugin's call was invalid.
pub fn crypto(op: &str, a: &str, b: &str, c: &str) -> String {
    let digest = |algorithm: &str, data: Vec<u8>| -> String {
        match algorithm {
            "md5" => hex::encode(Md5::digest(data)),
            "sha1" => hex::encode(Sha1::digest(data)),
            "sha256" => hex::encode(Sha256::digest(data)),
            "sha512" => hex::encode(Sha512::digest(data)),
            _ => String::new(),
        }
    };
    match op {
        "md5" | "sha1" | "sha256" | "sha512" => digest(op, hex_decode(a)),
        "hmacSha256" => {
            let key = hex_decode(a);
            let data = hex_decode(b);
            let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&key) else { return String::new() };
            mac.update(&data);
            hex::encode(mac.finalize().into_bytes())
        }
        "pbkdf2" => {
            let parts: Vec<&str> = c.split(':').collect();
            let size = parts.first().and_then(|v| v.parse::<usize>().ok()).unwrap_or(32);
            let rounds = parts.get(1).and_then(|v| v.parse::<u32>().ok()).unwrap_or(250_000);
            let algorithm = parts.get(2).copied().unwrap_or("sha256");
            let mut output = vec![0u8; size.min(256)];
            match algorithm {
                "sha1" => pbkdf2_hmac::<Sha1>(&hex_decode(a), &hex_decode(b), rounds, &mut output),
                "sha512" => pbkdf2_hmac::<Sha512>(&hex_decode(a), &hex_decode(b), rounds, &mut output),
                _ => pbkdf2_hmac::<Sha256>(&hex_decode(a), &hex_decode(b), rounds, &mut output),
            }
            hex::encode(output)
        }
        "aesDecrypt" => aes_decrypt(&hex_decode(a), &hex_decode(b), &hex_decode(c)).map(hex::encode).unwrap_or_default(),
        "aesEncrypt" => aes_encrypt(&hex_decode(a), &hex_decode(b), &hex_decode(c)).map(hex::encode).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Registers every host global on a fresh context.
pub fn register(ctx: &Ctx<'_>, state: SharedHost) -> JsResult<()> {
    let globals = ctx.globals();

    let capture = state.clone();
    globals.set("__capture_result", Func::from(move |text: String| {
        *capture.captured.lock().expect("plugin result") = Some(text);
    }))?;

    let log = state.clone();
    globals.set("__plugin_log", Func::from(move |level: String, message: String| {
        let mut out = log.log.lock().expect("plugin log");
        out.push_str(&format!("[{level}] {message}\n"));
    }))?;

    globals.set("__plugin_sleep", Func::from(Async(move |millis: f64| async move {
        let millis = millis.max(0.0).min(60_000.0);
        tokio::time::sleep(Duration::from_secs_f64(millis / 1000.0)).await;
        Ok::<(), JsError>(())
    })))?;

    let fetch_state = state.clone();
    globals.set("__native_fetch", Func::from(Async(move |url: String, method: String, headers: String, kind: String, body: String| {
        let state = fetch_state.clone();
        async move { native_fetch(state, url, method, headers, kind, body).await }
    })))?;

    globals.set("__native_dom", Func::from(native_dom))?;
    globals.set("__native_crypto", Func::from(|op: String, a: String, b: String, c: String| crypto(&op, &a, &b, &c)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_known_vectors() {
        let abc = hex::encode(b"abc");
        assert_eq!(crypto("md5", &abc, "", ""), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(crypto("sha256", &abc, "", ""), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        // RFC 4231, test case 1.
        let key = hex::encode(b"\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b\x0b");
        let data = hex::encode(b"Hi There");
        assert_eq!(crypto("hmacSha256", &key, &data, ""), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    }

    #[test]
    fn aes_cbc_pkcs7_matches_nist_vector() {
        // NIST SP 800-38A, AES-128-CBC first block, with PKCS#7 padding added by the shim.
        let key = "2b7e151628aed2a6abf7158809cf4f3c";
        let iv = "000102030405060708090a0b0c0d0e0f";
        let plain = "6bc1bee22e409f96e93d7e117393172a";
        let cipher = crypto("aesEncrypt", plain, key, iv);
        assert_eq!(&cipher[..32], "7649abac8119b246cee98e9b12e9197d");
        assert_eq!(crypto("aesDecrypt", &cipher, key, iv), plain);
        assert!(crypto("aesDecrypt", &cipher, key, "00").is_empty(), "a bad IV must fail");
    }
}
