//! QuickJS runtime for Nuvio scraper plugins.
//!
//! A fresh engine is created for every call (no state leaks between plugins or calls), with a
//! memory cap and an interrupt handler for CPU-heavy scripts. Host functions expose only a validated
//! `fetch`, a CSS-selector DOM bridge and logging.

use crate::error::PluginError;
use crate::host::{self, HostState, SharedHost};
use crate::model::parse_runtime_results;
use crate::model::PluginRuntimeResult;
use crate::polyfill::{call_script, wrap_module, POLYFILL};
use rquickjs::{AsyncContext, AsyncRuntime, Promise};
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const PLUGIN_TIMEOUT: Duration = Duration::from_secs(60);
const CPU_DEADLINE: Duration = Duration::from_secs(25);
const MEMORY_LIMIT: usize = 96 * 1024 * 1024;
const MAX_CONCURRENT_PLUGINS: usize = 10;

/// Arguments the original passes as the fourth `__get_call_args` global.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginCall {
    pub tmdb_id: String,
    pub media_type: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub scraper_id: String,
    pub settings: Value,
    pub tmdb_api_key: String,
}

impl Default for PluginCall {
    fn default() -> Self {
        Self {
            tmdb_id: String::new(),
            media_type: "movie".into(),
            season: None,
            episode: None,
            scraper_id: String::new(),
            settings: json!({}),
            tmdb_api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginOutput {
    pub results: Vec<PluginRuntimeResult>,
    pub log: String,
}

fn semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_PLUGINS))
}

/// Runs `getStreams` in a fresh sandbox and returns its streams plus the plugin log.
pub async fn get_streams(code: &str, call: &PluginCall) -> Result<PluginOutput, PluginError> {
    let _permit = semaphore().acquire().await.map_err(|_| PluginError::Timeout)?;
    match tokio::time::timeout(PLUGIN_TIMEOUT, run(code, call)).await {
        Ok(result) => result,
        Err(_) => Err(PluginError::Timeout),
    }
}

async fn run(code: &str, call: &PluginCall) -> Result<PluginOutput, PluginError> {
    let runtime = AsyncRuntime::new().map_err(|e| PluginError::Runtime(e.to_string()))?;
    runtime.set_memory_limit(MEMORY_LIMIT).await;
    let deadline = Instant::now() + CPU_DEADLINE;
    runtime
        .set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)))
        .await;

    let context = AsyncContext::full(&runtime).await.map_err(|e| PluginError::Runtime(e.to_string()))?;
    let state: SharedHost = Arc::new(HostState::default());
    let injected = inject_globals(call)?;

    let executed = context
        .async_with(async |ctx| {
            ctx.eval::<(), _>(injected)?;
            host::register(&ctx, state.clone())?;
            ctx.eval::<(), _>(POLYFILL)?;
            ctx.eval::<(), _>(wrap_module(code))?;
            let promise: Promise = ctx.eval(call_script())?;
            promise.into_future::<()>().await?;
            Ok::<(), rquickjs::Error>(())
        })
        .await;
    runtime.idle().await;
    executed.map_err(|e| PluginError::Runtime(e.to_string()))?;

    let raw = state.captured.lock().expect("plugin result").clone().unwrap_or_default();
    let log = state.log.lock().expect("plugin log").clone();
    Ok(PluginOutput { results: parse_runtime_results(&raw), log })
}

/// Evaluates a downloaded script without calling it, to reject broken repositories at install time.
pub async fn validate_module(code: &str) -> Result<bool, PluginError> {
    let runtime = AsyncRuntime::new().map_err(|e| PluginError::Runtime(e.to_string()))?;
    runtime.set_memory_limit(MEMORY_LIMIT).await;
    let context = AsyncContext::full(&runtime).await.map_err(|e| PluginError::Runtime(e.to_string()))?;
    let state: SharedHost = Arc::new(HostState::default());
    let injected = inject_globals(&PluginCall::default())?;
    let checked = context
        .async_with(async |ctx| {
            ctx.eval::<(), _>(injected)?;
            host::register(&ctx, state.clone())?;
            ctx.eval::<(), _>(POLYFILL)?;
            ctx.eval::<(), _>(wrap_module(code))?;
            ctx.eval::<bool, _>("typeof module.exports.getStreams === 'function' || typeof globalThis.getStreams === 'function'")
        })
        .await;
    runtime.idle().await;
    checked.map_err(|e| PluginError::Runtime(e.to_string()))
}

fn inject_globals(call: &PluginCall) -> Result<String, PluginError> {
    let settings = serde_json::to_string(&call.settings).map_err(|_| PluginError::Runtime("settings".into()))?;
    let args = json!({
        "tmdbId": call.tmdb_id,
        "mediaType": call.media_type,
        "season": call.season,
        "episode": call.episode,
    });
    let args = serde_json::to_string(&args).map_err(|_| PluginError::Runtime("call args".into()))?;
    let key = serde_json::to_string(&call.tmdb_api_key).map_err(|_| PluginError::Runtime("tmdb key".into()))?;
    let id = serde_json::to_string(&call.scraper_id).map_err(|_| PluginError::Runtime("scraper id".into()))?;
    Ok(format!(
        "globalThis.SCRAPER_ID = {id};\nglobalThis.SCRAPER_SETTINGS = {settings};\nglobalThis.CALL_ARGS = {args};\nglobalThis.TMDB_API_KEY = {key};\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run_code(code: &str) -> PluginOutput {
        get_streams(code, &PluginCall::default()).await.expect("plugin should run")
    }

    #[tokio::test]
    async fn original_commonjs_contract_returns_streams() {
        let output = run_code(
            r####"
            module.exports.getStreams = async function (tmdbId, mediaType, season, episode) {
              return [{
                title: 'A film', name: 'Mirror', url: 'https://cdn.test/a.m3u8',
                quality: 720, headers: { Referer: 'https://site.test' }, provider: 'example'
              }];
            };
            "####,
        )
        .await;
        assert_eq!(output.results.len(), 1);
        let stream = &output.results[0];
        assert_eq!(stream.url, "https://cdn.test/a.m3u8");
        assert_eq!(stream.title, "A film");
        assert_eq!(stream.quality.as_deref(), Some("720"));
        assert_eq!(stream.provider.as_deref(), Some("example"));
    }

    #[tokio::test]
    async fn globals_and_call_arguments_match_the_original() {
        let call = PluginCall {
            tmdb_id: "550".into(),
            media_type: "tv".into(),
            season: Some(2),
            episode: Some(4),
            scraper_id: "kisskh".into(),
            settings: json!({"lang": "ar"}),
            tmdb_api_key: "key-123".into(),
        };
        let code = r####"
          module.exports.getStreams = function (tmdbId, mediaType, season, episode) {
            return [{
              title: [SCRAPER_ID, TMDB_API_KEY, SCRAPER_SETTINGS.lang, tmdbId, mediaType, season, episode].join('|'),
              url: 'https://cdn.test/x.mp4'
            }];
          };
        "####;
        let output = get_streams(code, &call).await.unwrap();
        assert_eq!(output.results.len(), 1);
        assert_eq!(
            output.results[0].title,
            "kisskh|key-123|ar|550|tv|2|4"
        );
    }

    #[tokio::test]
    async fn a_script_without_getstreams_returns_empty() {
        let output = run_code("var nothing = 1;").await;
        assert!(output.results.is_empty());
    }

    #[tokio::test]
    async fn the_dom_bridge_supports_cheerio_style_selectors() {
        let output = run_code(
            r####"
            var cheerio = require('cheerio-without-node-native');
            module.exports.getStreams = function () {
              var $ = cheerio.load('<html><body><a class="v" href="https://cdn/x.m3u8">Watch</a><a class="v" href="https://cdn/y.m3u8">More</a></body></html>');
              var out = [];
              $('a.v').each(function (i, el) {
                out.push({ title: $(el).text(), url: $(el).attr('href') });
              });
              return out;
            };
            "####,
        )
        .await;
        assert_eq!(output.results.len(), 2, "log: {}", output.log);
        assert_eq!(output.results[0].title, "Watch");
        assert_eq!(output.results[0].url, "https://cdn/x.m3u8");
        assert_eq!(output.results[1].title, "More");
    }

    #[tokio::test]
    async fn fetch_to_private_addresses_is_blocked() {
        let output = run_code(
            r####"
            module.exports.getStreams = async function () {
              var a = await fetch('http://127.0.0.1:8080/secret');
              var b = await fetch('http://192.168.1.1/secret');
              var c = await fetch('http://router.local/secret');
              return [a, b, c].map(function (r) {
                return { title: String(r.status), url: 'https://cdn.test/' + r.status };
              });
            };
            "####,
        )
        .await;
        assert_eq!(output.results.len(), 3, "log: {}", output.log);
        assert!(output.results.iter().all(|r| r.title == "0"), "private targets must never be fetched: {}", output.log);
    }

    #[tokio::test]
    async fn console_log_goes_to_the_plugin_log() {
        let output = run_code(
            r####"
            module.exports.getStreams = function () {
              console.log('hello', 42);
              return [];
            };
            "####,
        )
        .await;
        assert!(output.log.contains("hello 42"), "{}", output.log);
    }

    #[tokio::test]
    async fn crypto_js_api_works_inside_a_plugin() {
        let output = run_code(
            r####"
            var C = require('crypto-js');
            module.exports.getStreams = function () {
              var md5 = C.MD5('abc').toString(C.enc.Hex);
              var sha = C.SHA256('abc').toString(C.enc.Hex);
              var hmac = C.HmacSHA256('Hi There', 'key').toString(C.enc.Hex);
              var key = C.enc.Hex.parse('000102030405060708090a0b0c0d0e0f');
              var iv = C.enc.Hex.parse('0f0e0d0c0b0a09080706050403020100');
              var cfg = { iv: iv, mode: C.mode.CBC, padding: C.pad.Pkcs7 };
              var encrypted = C.AES.encrypt(C.enc.Utf8.parse('hello world'), key, cfg);
              var decrypted = C.AES.decrypt(encrypted, key, cfg).toString(C.enc.Utf8);
              var b64 = C.enc.Base64.stringify(C.enc.Utf8.parse('hello'));
              var hex = C.enc.Hex.stringify(C.enc.Utf8.parse('abc'));
              return [{ title: [md5, sha.slice(0, 8), hmac.slice(0, 8), decrypted, b64, hex].join('|'), url: 'https://cdn/x.mp4' }];
            };
            "####,
        )
        .await;
        assert_eq!(output.results.len(), 1, "log: {}", output.log);
        let title = &output.results[0].title;
        assert!(title.starts_with("900150983cd24fb0d6963f7d28e17f72|ba7816bf|"), "{title}");
        assert!(title.contains("|hello world|aGVsbG8=|616263"), "{title}");
    }

    #[tokio::test]
    async fn the_real_scraper_fixture_loads_and_exports_getstreams() {
        let code = include_str!("../fixtures/kisskh.js");
        assert!(validate_module(code).await.unwrap(), "the real Nuvio scraper must be accepted");
    }

    #[test]
    fn private_ip_ranges_are_rejected() {
        for ip in ["127.0.0.1", "0.0.0.0", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.1.1", "100.64.0.1", "::1", "fd00::1"] {
            assert!(host::forbidden_ip(ip.parse().unwrap()), "{ip} must be blocked");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700::1111"] {
            assert!(!host::forbidden_ip(ip.parse().unwrap()), "{ip} is public");
        }
    }
}
