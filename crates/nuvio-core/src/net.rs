//! HTTP client with a DNS-over-HTTPS fallback, for addons and debrid providers.
//!
//! Some networks answer DNS lookups with a dead address instead of refusing them, so the usual
//! "resolution failed" retry never happens. [`DohResolver`] first asks the system resolver. When it
//! fails, returns nothing, or returns an address that is known to be a poisoned answer, the name is
//! looked up over HTTPS (DNS JSON API) and those addresses are used instead. TLS still sees the
//! original hostname, so certificates and SNI are unchanged.
//!
//! Environment (all optional):
//! - `NUVIO_DOH_URL` — DNS JSON endpoint (default: Cloudflare).
//! - `NUVIO_DOH_FORCE=1` — ask DoH first for every public hostname.
//! - `NUVIO_DOH_POISONED` — comma-separated extra IPs that count as poisoned answers.

use crate::CoreError;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde_json::Value;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_DOH: &str = "https://cloudflare-dns.com/dns-query";
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

fn endpoint() -> String {
    std::env::var("NUVIO_DOH_URL").unwrap_or_else(|_| DEFAULT_DOH.to_string())
}

fn forced() -> bool {
    std::env::var("NUVIO_DOH_FORCE").is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

/// A local name (no dot, `.local`, `.localhost`) must keep using the system resolver.
fn is_local(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    !host.contains('.') || host.ends_with(".local") || host.ends_with(".localhost")
}

fn known_poison(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_unspecified()
                || v4.is_loopback()
                || v4.is_broadcast()
                || v4 == Ipv4Addr::new(127, 0, 0, 53) // systemd's stub answers every name
                || v4 == Ipv4Addr::new(0, 0, 0, 0)
        }
        IpAddr::V6(v6) => v6.is_unspecified() || v6.is_loopback() || v6 == Ipv6Addr::UNSPECIFIED,
    }
}

fn poison_extra(ip: IpAddr) -> bool {
    std::env::var("NUVIO_DOH_POISONED")
        .map(|list| list.split(',').filter_map(|s| s.trim().parse::<IpAddr>().ok()).any(|p| p == ip))
        .unwrap_or(false)
}

fn bad_answer(addrs: &[SocketAddr], host: &str) -> bool {
    addrs.is_empty()
        || (!is_local(host)
            && (forced()
                || addrs.iter().any(|a| {
                    let ip = a.ip();
                    known_poison(ip) || poison_extra(ip)
                })))
}

/// Parses the `Answer` array of a DNS JSON response into IP addresses.
pub fn parse_doh_json(body: &str) -> Vec<IpAddr> {
    let Ok(json) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let Some(answers) = json.get("Answer").and_then(Value::as_array) else { return Vec::new() };
    answers
        .iter()
        .filter_map(|answer| {
            let kind = answer.get("type").and_then(Value::as_i64).unwrap_or(0);
            let data = answer.get("data").and_then(Value::as_str)?;
            match kind {
                1 | 28 => data.trim().parse::<IpAddr>().ok(),
                _ => None,
            }
        })
        .collect()
}

#[derive(Clone)]
pub struct DohResolver {
    http: reqwest::Client,
    cache: Arc<Mutex<HashMap<String, (Instant, Vec<IpAddr>)>>>,
}

impl DohResolver {
    pub fn new() -> Result<Self, CoreError> {
        // The DoH endpoint itself must not go through the resolver being built. Cloudflare's
        // certificate covers cloudflare-dns.com even when we connect to its public IP.
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(6));
        if endpoint() == DEFAULT_DOH {
            for ip in ["1.1.1.1", "1.0.0.1"] {
                if let Ok(addr) = format!("{ip}:443").parse() {
                    builder = builder.resolve("cloudflare-dns.com", addr);
                }
            }
        }
        Ok(Self { http: builder.build()?, cache: Arc::new(Mutex::new(HashMap::new())) })
    }

    async fn query(&self, host: &str) -> Vec<IpAddr> {
        let url = endpoint();
        let run = |kind: &'static str| {
            let url = url.clone();
            let http = self.http.clone();
            let host = host.to_string();
            async move {
                let response = http
                    .get(&url)
                    .query(&[("name", host.as_str()), ("type", kind)])
                    .header("Accept", "application/dns-json")
                    .send()
                    .await
                    .ok()?;
                let body = response.text().await.ok()?;
                Some(parse_doh_json(&body))
            }
        };
        let (a, aaaa) = tokio::join!(run("A"), run("AAAA"));
        let mut ips = a.unwrap_or_default();
        if ips.is_empty() {
            ips = aaaa.unwrap_or_default();
        }
        ips
    }

    async fn resolve_over_https(&self, host: &str) -> Vec<IpAddr> {
        {
            let cache = self.cache.lock().expect("doh cache");
            if let Some((at, ips)) = cache.get(host) {
                if at.elapsed() < CACHE_TTL {
                    return ips.clone();
                }
            }
        }
        let ips = self.query(host).await;
        if !ips.is_empty() {
            let mut cache = self.cache.lock().expect("doh cache");
            cache.insert(host.to_string(), (Instant::now(), ips.clone()));
        }
        ips
    }
}

impl Resolve for DohResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let resolver = self.clone();
        Box::pin(async move {
            let system: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map(|found| found.collect())
                .unwrap_or_default();
            let mut addrs = system;
            if bad_answer(&addrs, &host) && !is_local(&host) {
                let from_doh: Vec<SocketAddr> = resolver
                    .resolve_over_https(&host)
                    .await
                    .into_iter()
                    .map(|ip| SocketAddr::new(ip, 0))
                    .collect();
                if !from_doh.is_empty() {
                    addrs = from_doh;
                }
            }
            if addrs.is_empty() {
                return Err("no DNS answer".into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

fn client_builder() -> Result<reqwest::ClientBuilder, CoreError> {
    Ok(reqwest::Client::builder()
        .dns_resolver(DohResolver::new()?)
        .connect_timeout(Duration::from_secs(60))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION"))))
}

/// Shared client for addons, debrid APIs and the local relay.
pub fn http_client() -> Result<reqwest::Client, CoreError> {
    Ok(client_builder()?.build()?)
}

/// A builder with the same DoH resolver, for callers that need per-request settings such as
/// pinned addresses (the plugin sandbox pins the IP it validated before connecting).
pub fn http_client_builder() -> Result<reqwest::ClientBuilder, CoreError> {
    client_builder()
}

/// Resolves a host with the fallback, for diagnostics and tests.
pub async fn resolve_host(host: &str) -> Result<Vec<SocketAddr>, CoreError> {
    let resolver = DohResolver::new()?;
    let name = Name::from_str(host).map_err(|_| CoreError::InvalidPayload("invalid hostname".into()))?;
    let addrs = resolver.resolve(name).await.map_err(|_| CoreError::InvalidPayload("no DNS answer".into()))?;
    Ok(addrs.collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_json_answers_are_parsed() {
        let body = r#"{"Status":0,"Answer":[
            {"name":"a.test.","type":1,"data":"1.2.3.4"},
            {"name":"a.test.","type":28,"data":"2606:4700::1"},
            {"name":"a.test.","type":5,"data":"cname.test."}
        ]}"#;
        let ips = parse_doh_json(body);
        assert_eq!(ips.len(), 2);
        assert!(ips.contains(&"1.2.3.4".parse::<IpAddr>().unwrap()));
        assert!(ips.contains(&"2606:4700::1".parse::<IpAddr>().unwrap()));
        assert!(parse_doh_json("{}").is_empty());
    }

    #[test]
    fn local_names_keep_the_system_resolver() {
        assert!(is_local("localhost"));
        assert!(is_local("printer.local"));
        assert!(!is_local("api.torbox.app"));
    }

    #[test]
    fn poisoned_answers_trigger_the_fallback() {
        let good: Vec<SocketAddr> = vec!["1.1.1.1:443".parse().unwrap()];
        let poisoned: Vec<SocketAddr> = vec!["0.0.0.0:443".parse().unwrap()];
        assert!(!bad_answer(&good, "api.torbox.app"));
        assert!(bad_answer(&poisoned, "api.torbox.app"));
        assert!(bad_answer(&[], "api.torbox.app"));
        assert!(!bad_answer(&poisoned, "printer.local"), "local names are never sent to DoH");
    }
}
