# Host API contract for phase 2

Source (GPL-3.0): Nuvio Desktop `Dev` branch,
`composeApp/src/fullCommonMain/kotlin/com/nuvio/app/features/plugins/runtime/js/JsBindings.kt`
and `runtime/host/HostFunctions.kt`, `runtime/network/FetchBridge.kt`, `runtime/dom/DomBridge.kt`,
`runtime/crypto/CryptoBridge.kt`, `runtime/wasm/WasmBridge.kt`.

The original wraps every plugin in `var module = { exports: {} }; var exports = module.exports;`
and then calls:

```js
var getStreams = module.exports.getStreams || globalThis.getStreams;
var args = JSON.parse(__get_call_args());          // { tmdbId, mediaType, season, episode }
var result = await getStreams(args.tmdbId, args.mediaType, args.season, args.episode);
__capture_result(JSON.stringify(result || []));
```

The settings layout is asked similarly through `module.exports.onSettings` / `globalThis.onSettings`
with `__capture_result`.

Globals the plugin may rely on, supplied by the runtime before the script is evaluated:

| Global | Native bridge in phase 2 |
|---|---|
| `__get_scraper_id()`, `__get_scraper_settings()`, `__get_tmdb_api_key()`, `__get_call_args()` | Rust host functions; TMDB key comes from our existing settings |
| `__capture_result(string)` | receives the plugin's JSON answer |
| `__plugin_sleep(ms)` + `setTimeout` / `setInterval` / `clearTimeout` / `clearInterval` | timers backed by Rust (QuickJS has no event loop) |
| `fetch` / `__native_fetch` | our DoH-aware HTTP client, with `ok`, `status`, `url`, `body`, `bodyBase64`, `headers`, and `res.text()` / `res.json()` |
| `require('cheerio')`, `require('cheerio-without-node-native')`, `require('react-native-cheerio')` | `scraper` / `html5ever` DOM bridge with CSS selectors |
| `require('axios')`, `require('crypto-js')` | thin shims over the host fetch/crypto |
| `URL`, `URLSearchParams`, `atob`, `btoa`, `TextEncoder`, `TextDecoder` | polyfills |
| `crypto` / WebAssembly bridge | `crypto.getRandomValues`/`randomUUID` exist; `require('crypto-js')` is implemented through a native bridge: WordArray, `enc.Utf8/Hex/Base64/Latin1`, MD5/SHA1/SHA256/SHA512, HmacSHA256, PBKDF2, AES-CBC with PKCS#7 (including OpenSSL `Salted__` envelopes). WebAssembly is still deferred |
| `SCRAPER_ID`, `SCRAPER_SETTINGS`, `CALL_ARGS`, `TMDB_API_KEY`, `global`, `window`, `self` | injected into every fresh context; `__get_scraper_id`, `__get_scraper_settings`, `__get_tmdb_api_key`, `__get_call_args` are JS aliases for compatibility |

Security additions from VyseTV (not in the original): refuse `fetch` to loopback, private ranges,
CGNAT, link-local and `*.local`/`*.internal`, **after every redirect** as well; the validated IP is
pinned on the request so a second DNS answer cannot change the destination; fresh engine per call;
96 MB memory cap, 25 s CPU interrupt and 60 s wall-clock timeout. This guard is mandatory in our
runtime because the process holds debrid keys and the Nuvio session.

Not ported yet: `onSettings` layout, the Vega and `repo.json` adapters, WebAssembly and catalogue
plugins. Regular `manifest.json` scraper lists — the format Nuvio's community uses — are supported.
