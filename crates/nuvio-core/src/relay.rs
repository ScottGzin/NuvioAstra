//! Small local HTTP relay (`127.0.0.1`, random port) for streams that need request headers.
//!
//! mpv sends the headers of the main URL, but not of every HLS segment or playlist inside it. A
//! relay puts those headers on every upstream request instead. The player only sees a plain local
//! URL. The relay keeps no data beyond the connection and is aborted when the player is closed.
//! Upstream requests go through [`crate::net::http_client`], so the DoH fallback applies too.

use crate::net;
use crate::CoreError;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// Which URL an incoming path maps to. `/` is the target itself; `/segment.ts` is resolved next to
/// it, so relative playlist entries keep working.
pub fn upstream_url(target: &str, suffix: &str) -> String {
    let suffix = suffix.trim_start_matches('/');
    if suffix.is_empty() {
        return target.to_string();
    }
    let without_query = target.split(['?', '#']).next().unwrap_or(target);
    let after_scheme = without_query.find("://").map(|i| i + 3).unwrap_or(0);
    let base = match without_query[after_scheme..].rfind('/') {
        Some(index) => &without_query[..after_scheme + index + 1],
        None => return format!("{without_query}/{suffix}"),
    };
    format!("{base}{suffix}")
}

struct Head {
    method: String,
    path: String,
}

fn parse_head(bytes: &[u8]) -> Option<Head> {
    let text = String::from_utf8_lossy(bytes);
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_ascii_uppercase();
    let path = parts.next()?.to_string();
    if path.is_empty() || !path.starts_with('/') {
        return None;
    }
    Some(Head { method, path })
}

fn header_value(map: &HeaderMap, name: &reqwest::header::HeaderName) -> Option<String> {
    map.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

async fn serve(mut socket: TcpStream, target: String, headers: Vec<(String, String)>, client: reqwest::Client) {
    let mut buffer = vec![0u8; 16 * 1024];
    let read = match tokio::time::timeout(std::time::Duration::from_secs(15), socket.read(&mut buffer)).await {
        Ok(Ok(read)) => read,
        _ => return,
    };
    if read == 0 {
        return;
    }
    let Some(head) = parse_head(&buffer[..read]) else {
        let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    };
    if head.method != "GET" && head.method != "HEAD" {
        let _ = socket.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    }
    let url = upstream_url(&target, &head.path);
    let mut request_headers = HeaderMap::new();
    let text = String::from_utf8_lossy(&buffer[..read]);
    for line in text.lines().skip(1) {
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.trim().as_bytes()), HeaderValue::from_str(value.trim())) {
                    request_headers.insert(name, value);
                }
            }
        }
    }
    request_headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    for (name, value) in &headers {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.trim().as_bytes()), HeaderValue::from_str(value.trim())) {
            request_headers.insert(name, value);
        }
    }

    let method = if head.method == "HEAD" { reqwest::Method::HEAD } else { reqwest::Method::GET };
    let Ok(mut response) = client.request(method, &url).headers(request_headers).send().await else {
        let _ = socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return;
    };

    let status = response.status();
    let host = url.split("://").nth(1).unwrap_or(&url).split('/').next().unwrap_or("?");
    eprintln!("[relay] {} {} → {}", head.method, host, status.as_u16());
    let mut out = format!("HTTP/1.1 {} {}\r\n", status.as_u16(), status.canonical_reason().unwrap_or("OK"));
    for name in [CONTENT_TYPE, CONTENT_LENGTH, CONTENT_RANGE] {
        if let Some(value) = header_value(response.headers(), &name) {
            out.push_str(&format!("{}: {}\r\n", name.as_str(), value));
        }
    }
    if header_value(response.headers(), &RANGE).is_some() || header_value(response.headers(), &CONTENT_RANGE).is_some() {
        out.push_str("Accept-Ranges: bytes\r\n");
    }
    out.push_str("Cache-Control: no-store\r\nConnection: close\r\n\r\n");
    if socket.write_all(out.as_bytes()).await.is_err() {
        return;
    }
    if head.method == "HEAD" {
        return;
    }
    while let Ok(Some(chunk)) = response.chunk().await {
        if socket.write_all(&chunk).await.is_err() {
            return;
        }
    }
    let _ = socket.flush().await;
}

/// A running relay. Dropping it closes the local port.
pub struct Relay {
    url: String,
    task: JoinHandle<()>,
}

impl Relay {
    pub async fn start(target: String, headers: Vec<(String, String)>) -> Result<Self, CoreError> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| CoreError::InvalidPayload(format!("relay bind: {e}")))?;
        let addr: SocketAddr = listener
            .local_addr()
            .map_err(|e| CoreError::InvalidPayload(format!("relay address: {e}")))?;
        let client = net::http_client()?;
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else { break };
                let target = target.clone();
                let headers = headers.clone();
                let client = client.clone();
                tokio::spawn(serve(socket, target, headers, client));
            }
        });
        Ok(Self { url: format!("http://{addr}/"), task })
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_resolved_next_to_the_target() {
        let target = "https://host.test/a/b/master.m3u8?token=1";
        assert_eq!(upstream_url(target, "/"), target);
        assert_eq!(upstream_url(target, "/seg1.ts"), "https://host.test/a/b/seg1.ts");
        assert_eq!(upstream_url("https://host.test", "/v.mkv"), "https://host.test/v.mkv");
    }

    #[test]
    fn request_head_is_parsed() {
        let head = parse_head(b"GET /seg.ts HTTP/1.1\r\nRange: bytes=0-\r\n\r\n").unwrap();
        assert_eq!(head.method, "GET");
        assert_eq!(head.path, "/seg.ts");
        assert!(parse_head(b"garbage\r\n\r\n").is_none());
    }

    #[tokio::test]
    async fn headers_reach_every_upstream_request() {
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_url = format!("http://{}/list.m3u8", origin.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = origin.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            assert!(request.to_ascii_lowercase().contains("x-test: yes"), "{request}");
            let body = b"#EXTM3U\nseg.ts\n";
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        });

        let relay = Relay::start(origin_url, vec![("X-Test".into(), "yes".into())]).await.unwrap();
        let response = reqwest::get(relay.url()).await.unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.text().await.unwrap(), "#EXTM3U\nseg.ts\n");
        server.await.unwrap();
    }
}
