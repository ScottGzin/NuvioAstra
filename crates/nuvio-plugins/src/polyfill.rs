//! The small web-like surface the original gives plugins. QuickJS has no DOM, fetch, timers or
//! Node modules, so these are built on the host functions in [`crate::host`].

/// Must be evaluated after the four `SCRAPER_*` / `TMDB_API_KEY` globals are injected.
pub const POLYFILL: &str = r####"
(function () {
  if (typeof globalThis.global === 'undefined') globalThis.global = globalThis;
  if (typeof globalThis.window === 'undefined') globalThis.window = globalThis;
  if (typeof globalThis.self === 'undefined') globalThis.self = globalThis;

  globalThis.__get_scraper_id = function () { return SCRAPER_ID; };
  globalThis.__get_scraper_settings = function () { return JSON.stringify(SCRAPER_SETTINGS); };
  globalThis.__get_tmdb_api_key = function () { return TMDB_API_KEY; };
  globalThis.__get_call_args = function () { return JSON.stringify(CALL_ARGS); };

  ['log', 'error', 'warn', 'info', 'debug'].forEach(function (level) {
    globalThis.console = globalThis.console || {};
    globalThis.console[level] = function () {
      var parts = Array.prototype.slice.call(arguments).map(function (v) {
        if (typeof v === 'string') return v;
        try { return JSON.stringify(v); } catch (e) { return String(v); }
      });
      __plugin_log(level, parts.join(' '));
    };
  });

  // QuickJS has no event loop; timers are backed by the host's async sleep.
  var __timer_id = 0;
  var __timers = Object.create(null);
  function __duration(value, interval) {
    var n = Number(value);
    if (!isFinite(n) || n < 0) n = 0;
    n = Math.min(Math.floor(n), 60000);
    return interval && n < 1 ? 1 : n;
  }
  globalThis.setTimeout = function (fn, delay) {
    if (typeof fn !== 'function') throw new TypeError('timer callback must be a function');
    var id = ++__timer_id;
    var args = Array.prototype.slice.call(arguments, 2);
    __timers[id] = true;
    __plugin_sleep(__duration(delay, false)).then(function () {
      if (!__timers[id]) return;
      delete __timers[id];
      fn.apply(globalThis, args);
    });
    return id;
  };
  globalThis.clearTimeout = function (id) { delete __timers[id]; };
  globalThis.setInterval = function (fn, delay) {
    var id = ++__timer_id;
    var ms = __duration(delay, true);
    __timers[id] = true;
    (function tick() {
      __plugin_sleep(ms).then(function () {
        if (!__timers[id]) return;
        fn();
        if (__timers[id]) tick();
      });
    })();
    return id;
  };
  globalThis.clearInterval = function (id) { delete __timers[id]; };

  globalThis.fetch = async function (url, options) {
    options = options || {};
    var headers = options.headers || {};
    var kind = 'none';
    var body = '';
    if (options.body != null) {
      if (typeof options.body === 'string') {
        kind = 'text';
        body = options.body;
      } else {
        kind = 'text';
        body = JSON.stringify(options.body);
        if (!headers['Content-Type'] && !headers['content-type']) headers['Content-Type'] = 'application/json';
      }
    }
    var raw = await __native_fetch(String(url), options.method || 'GET', JSON.stringify(headers), kind, body);
    var answer = JSON.parse(raw);
    return Promise.resolve({
      ok: answer.ok,
      status: answer.status,
      statusText: answer.statusText,
      url: answer.url,
      headers: answer.headers,
      body: answer.body,
      bodyBase64: answer.bodyBase64,
      text: function () { return Promise.resolve(answer.body); },
      json: function () { return Promise.resolve(JSON.parse(answer.body)); }
    });
  };

  // Minimal Cheerio-compatible set, backed by the Rust `scraper` bridge.
  function DomSet(nodes) {
    this.nodes = nodes || [];
    this.length = this.nodes.length;
    for (var i = 0; i < this.nodes.length; i++) this[i] = this.nodes[i];
  }
  DomSet.prototype.text = function () {
    return this.nodes.map(function (n) { return n.text || ''; }).join('');
  };
  DomSet.prototype.html = function () { return this.nodes.length ? this.nodes[0].html : null; };
  DomSet.prototype.attr = function (name) {
    if (!this.nodes.length || !this.nodes[0].attrs) return null;
    var value = this.nodes[0].attrs[name];
    return value === undefined ? null : value;
  };
  DomSet.prototype.data = function (name) { return this.attr('data-' + name); };
  DomSet.prototype.each = function (fn) {
    for (var i = 0; i < this.nodes.length; i++) fn.call(this.nodes[i], i, this.nodes[i]);
    return this;
  };
  DomSet.prototype.map = function (fn) { return this.nodes.map(fn); };
  DomSet.prototype.get = function (i) { return this.nodes[i]; };
  DomSet.prototype.first = function () { return new DomSet(this.nodes.slice(0, 1)); };
  DomSet.prototype.last = function () { return new DomSet(this.nodes.slice(-1)); };
  DomSet.prototype.eq = function (i) { return new DomSet(i < 0 ? [] : [this.nodes[i]]); };
  DomSet.prototype.find = function (selector) {
    var out = [];
    for (var i = 0; i < this.nodes.length; i++) {
      out = out.concat(JSON.parse(__native_dom(this.nodes[i].html || '', String(selector))));
    }
    return new DomSet(out);
  };
  DomSet.prototype.children = function () { return this.find('*'); };

  function cheerioLoad(html) {
    return function (selector) {
      if (selector && typeof selector === 'object' && selector.html) {
        return new DomSet([selector]);
      }
      if (selector && typeof selector === 'string' && selector.trim().charAt(0) === '<') {
        return new DomSet(JSON.parse(__native_dom(selector, '*')));
      }
      return new DomSet(JSON.parse(__native_dom(html || '', String(selector))));
    };
  }
  var cheerio = { load: cheerioLoad };
  var modules = {
    'cheerio': cheerio,
    'cheerio-without-node-native': cheerio,
    'react-native-cheerio': cheerio,
    'axios': {
      get: function (url, config) { return fetch(url, { headers: config && config.headers }); },
      post: function (url, body, config) { return fetch(url, { method: 'POST', body: body, headers: config && config.headers }); }
    },
    'crypto-js': (function () {
      // crypto-js compatible WordArray: the same `{ words, sigBytes }` shape real scrapers use.
      function WA(words, sigBytes) {
        this.words = words || [];
        this.sigBytes = sigBytes === undefined ? this.words.length * 4 : sigBytes;
      }
      function bytesOf(wa) {
        var out = [];
        for (var i = 0; i < wa.sigBytes; i++) out.push((wa.words[i >>> 2] >>> (24 - (i % 4) * 8)) & 255);
        return out;
      }
      function fromBytes(bytes) {
        var words = [];
        for (var i = 0; i < bytes.length; i++) words[i >>> 2] = ((words[i >>> 2] || 0) | ((bytes[i] & 255) << (24 - (i % 4) * 8))) >>> 0;
        return new WA(words, bytes.length);
      }
      function hexOf(wa) {
        return bytesOf(wa).map(function (b) { return ('0' + b.toString(16)).slice(-2); }).join('');
      }
      function ofHex(hex) {
        var h = String(hex), bytes = [];
        for (var i = 0; i + 1 < h.length; i += 2) bytes.push(parseInt(h.substr(i, 2), 16));
        return fromBytes(bytes);
      }
      WA.prototype.toString = function (encoder) {
        return encoder && encoder.stringify ? encoder.stringify(this) : hexOf(this);
      };
      function utf8Bytes(text) {
        var s = String(text), out = [];
        for (var i = 0; i < s.length; i++) {
          var c = s.charCodeAt(i);
          if (c < 128) out.push(c);
          else if (c < 2048) out.push(192 | (c >> 6), 128 | (c & 63));
          else if (c >= 55296 && c <= 56319 && i + 1 < s.length) {
            var c2 = s.charCodeAt(++i), cp = 65536 + ((c - 55296) << 10) + (c2 - 56320);
            out.push(240 | (cp >> 18), 128 | ((cp >> 12) & 63), 128 | ((cp >> 6) & 63), 128 | (cp & 63));
          } else out.push(224 | (c >> 12), 128 | ((c >> 6) & 63), 128 | (c & 63));
        }
        return out;
      }
      function utf8Text(bytes) {
        var out = '';
        for (var i = 0; i < bytes.length;) {
          var b = bytes[i++];
          if (b < 128) out += String.fromCharCode(b);
          else if (b < 224) out += String.fromCharCode(((b & 31) << 6) | (bytes[i++] & 63));
          else if (b < 240) out += String.fromCharCode(((b & 15) << 12) | ((bytes[i++] & 63) << 6) | (bytes[i++] & 63));
          else {
            var cp = ((b & 7) << 18) | ((bytes[i++] & 63) << 12) | ((bytes[i++] & 63) << 6) | (bytes[i++] & 63);
            cp -= 65536;
            out += String.fromCharCode(55296 + (cp >> 10), 56320 + (cp & 1023));
          }
        }
        return out;
      }
      var Utf8 = { parse: function (s) { return fromBytes(utf8Bytes(s)); }, stringify: function (wa) { return utf8Text(bytesOf(wa)); } };
      var Hex = { parse: ofHex, stringify: hexOf };
      var Base64 = {
        parse: function (s) {
          var bin = atob(String(s).replace(/-/g, '+').replace(/_/g, '/')), bytes = [];
          for (var i = 0; i < bin.length; i++) bytes.push(bin.charCodeAt(i) & 255);
          return fromBytes(bytes);
        },
        stringify: function (wa) {
          var bytes = bytesOf(wa), text = '';
          for (var i = 0; i < bytes.length; i++) text += String.fromCharCode(bytes[i]);
          return btoa(text);
        }
      };
      function waOf(value) {
        if (typeof value === 'string') return Utf8.parse(value);
        if (value && value.words) return value;
        if (value && value.ciphertext) return value.ciphertext;
        return new WA([], 0);
      }
      function hashed(algorithm, message) {
        return ofHex(__native_crypto(algorithm, hexOf(waOf(message)), '', ''));
      }
      // OpenSSL's `Salted__` envelope: derive key/iv with EVP_BytesToKey(MD5), as crypto-js does.
      function opensslKey(passphraseWa, saltBytes, keyBytes) {
        var material = [], previous = [], password = bytesOf(passphraseWa);
        while (material.length < keyBytes + 16) {
          var input = previous.concat(password).concat(saltBytes), h = __native_crypto('md5', hexOf(fromBytes(input)), '', '');
          previous = bytesOf(ofHex(h));
          material = material.concat(previous);
        }
        return { key: fromBytes(material.slice(0, keyBytes)), iv: fromBytes(material.slice(keyBytes, keyBytes + 16)) };
      }
      function cipherParamsOf(value) {
        if (typeof value === 'string') return Base64.parse(value);
        if (value && value.ciphertext) return value.ciphertext;
        return waOf(value);
      }
      return {
        lib: { WordArray: {
          create: function (value) { return waOf(value); },
          random: function (nBytes) {
            var bytes = [];
            for (var i = 0; i < nBytes; i++) bytes.push(Math.floor(Math.random() * 256));
            return fromBytes(bytes);
          }
        } },
        enc: { Utf8: Utf8, Hex: Hex, Base64: Base64, Latin1: {
          parse: function (s) { var out = []; for (var i = 0; i < String(s).length; i++) out.push(s.charCodeAt(i) & 255); return fromBytes(out); },
          stringify: function (wa) { return bytesOf(wa).map(function (b) { return String.fromCharCode(b); }).join(''); }
        } },
        mode: { CBC: 'CBC', CFB: 'CFB', OFB: 'OFB', CTR: 'CTR', ECB: 'ECB' },
        pad: { Pkcs7: 'Pkcs7', AnsiX923: 'AnsiX923', Iso10126: 'Iso10126', NoPadding: 'NoPadding' },
        format: { OpenSSL: {}, Hex: {}, Base64: {} },
        MD5: function (m) { return hashed('md5', m); },
        SHA1: function (m) { return hashed('sha1', m); },
        SHA256: function (m) { return hashed('sha256', m); },
        SHA512: function (m) { return hashed('sha512', m); },
        HmacSHA256: function (m, k) { return ofHex(__native_crypto('hmacSha256', hexOf(waOf(k)), hexOf(waOf(m)), '')); },
        PBKDF2: function (password, salt, cfg) {
          cfg = cfg || {};
          return ofHex(__native_crypto('pbkdf2', hexOf(waOf(password)), hexOf(waOf(salt)),
            String(cfg.keySize || 32) + ':' + String(cfg.iterations || 250000) + ':' + String((cfg.hasher && cfg.hasher.name) || 'sha256')));
        },
        AES: {
          decrypt: function (cipherParams, key, cfg) {
            cfg = cfg || {};
            var cipherWa = cipherParamsOf(cipherParams), keyWa = waOf(key), ivWa = cfg.iv ? waOf(cfg.iv) : null;
            if (!ivWa && typeof cipherParams === 'string') {
              var raw = bytesOf(cipherWa);
              if (raw.length >= 16 && utf8Text(raw.slice(0, 8)) === 'Salted__') {
                var derived = opensslKey(keyWa, raw.slice(8, 16), typeof key === 'string' ? 32 : hexOf(keyWa).length / 2);
                keyWa = derived.key;
                ivWa = derived.iv;
                cipherWa = fromBytes(raw.slice(16));
              }
            }
            if (!ivWa) ivWa = new WA([0, 0, 0, 0], 16);
            var plain = __native_crypto('aesDecrypt', hexOf(cipherWa), hexOf(keyWa), hexOf(ivWa));
            if (!plain) throw new Error('AES decrypt failed');
            return ofHex(plain);
          },
          encrypt: function (plain, key, cfg) {
            cfg = cfg || {};
            var ivWa = cfg.iv ? waOf(cfg.iv) : new WA([0, 0, 0, 0], 16);
            var cipher = __native_crypto('aesEncrypt', hexOf(waOf(plain)), hexOf(waOf(key)), hexOf(ivWa));
            if (!cipher) throw new Error('AES encrypt failed');
            return ofHex(cipher);
          }
        }
      };
    })()
  };
  globalThis.require = function (name) {
    if (Object.prototype.hasOwnProperty.call(modules, name)) return modules[name];
    throw new Error('module not available in plugin sandbox: ' + name);
  };

  // URL helpers used by most scrapers to build query strings.
  function URLSearchParams(init) {
    this._pairs = [];
    var self = this;
    if (typeof init === 'string') {
      init.replace(/^\?/, '').split('&').forEach(function (part) {
        if (!part) return;
        var at = part.indexOf('=');
        var key = at < 0 ? part : part.slice(0, at);
        var value = at < 0 ? '' : part.slice(at + 1);
        self._pairs.push([decodeURIComponent(key.replace(/\+/g, ' ')), decodeURIComponent(value.replace(/\+/g, ' '))]);
      });
    } else if (init && typeof init === 'object') {
      Object.keys(init).forEach(function (key) { self._pairs.push([key, String(init[key])]); });
    }
  }
  URLSearchParams.prototype.get = function (key) {
    for (var i = 0; i < this._pairs.length; i++) if (this._pairs[i][0] === key) return this._pairs[i][1];
    return null;
  };
  URLSearchParams.prototype.has = function (key) { return this.get(key) !== null; };
  URLSearchParams.prototype.set = function (key, value) {
    for (var i = 0; i < this._pairs.length; i++) if (this._pairs[i][0] === key) { this._pairs[i][1] = String(value); return; }
    this._pairs.push([key, String(value)]);
  };
  URLSearchParams.prototype.append = function (key, value) { this._pairs.push([key, String(value)]); };
  URLSearchParams.prototype.forEach = function (fn) { this._pairs.forEach(function (p) { fn(p[1], p[0]); }); };
  URLSearchParams.prototype.toString = function () {
    return this._pairs.map(function (p) {
      return encodeURIComponent(p[0]) + '=' + encodeURIComponent(p[1]);
    }).join('&');
  };
  globalThis.URLSearchParams = URLSearchParams;

  function URL(input) {
    var text = String(input);
    this.href = text;
    var match = /^([a-zA-Z][a-zA-Z0-9+.-]*:\/\/)?([^\/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/.exec(text) || [];
    this.protocol = match[1] || '';
    this.host = match[2] || '';
    this.hostname = this.host.replace(/:.*$/, '');
    this.pathname = match[3] || '/';
    this.search = match[4] || '';
    this.hash = match[5] || '';
    this.searchParams = new URLSearchParams(this.search);
  }
  URL.prototype.toString = function () { return this.href; };
  globalThis.URL = URL;

  var B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  globalThis.btoa = function (value) {
    var bytes = String(value);
    var out = '';
    for (var i = 0; i < bytes.length; i += 3) {
      var a = bytes.charCodeAt(i), b = bytes.charCodeAt(i + 1), c = bytes.charCodeAt(i + 2);
      out += B64.charAt(a >> 2);
      out += B64.charAt(((a & 3) << 4) | ((b || 0) >> 4));
      out += isNaN(b) ? '=' : B64.charAt(((b & 15) << 2) | ((c || 0) >> 6));
      out += (isNaN(b) || isNaN(c)) ? '=' : B64.charAt(c & 63);
    }
    return out;
  };
  globalThis.atob = function (value) {
    var text = String(value).replace(/=+$/, '');
    var out = '';
    for (var i = 0; i < text.length; i += 4) {
      var a = B64.indexOf(text.charAt(i));
      var b = B64.indexOf(text.charAt(i + 1));
      var c = B64.indexOf(text.charAt(i + 2));
      var d = B64.indexOf(text.charAt(i + 3));
      out += String.fromCharCode((a << 2) | (b >> 4));
      if (c >= 0) out += String.fromCharCode(((b & 15) << 4) | (c >> 2));
      if (d >= 0) out += String.fromCharCode(((c & 3) << 6) | d);
    }
    return out;
  };
  globalThis.TextEncoder = function () {};
  globalThis.TextEncoder.prototype.encode = function (text) { return String(text); };
  globalThis.TextDecoder = function () {};
  globalThis.TextDecoder.prototype.decode = function (text) { return String(text); };
  globalThis.crypto = globalThis.crypto || {
    randomUUID: function () { return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function (c) {
      var r = Math.random() * 16 | 0;
      return (c === 'x' ? r : (r & 3) | 8).toString(16);
    }); },
    getRandomValues: function (array) { for (var i = 0; i < array.length; i++) array[i] = Math.floor(Math.random() * 256); return array; }
  };
})();
"####;

/// Call wrapper matching `JsBindings.staticCallCode` in the original.
pub fn call_script() -> &'static str {
    r####"
(async function () {
  try {
    var getStreams = (typeof module !== 'undefined' && module.exports && module.exports.getStreams) || globalThis.getStreams;
    if (typeof getStreams !== 'function') {
      __capture_result('[]');
      return;
    }
    var args = JSON.parse(__get_call_args());
    var result = await getStreams(
      args.tmdbId,
      args.mediaType,
      args.season == null ? undefined : args.season,
      args.episode == null ? undefined : args.episode
    );
    __capture_result(JSON.stringify(result || []));
  } catch (error) {
    __plugin_log('error', String(error && error.stack ? error.stack : (error && error.message ? error.message : error)));
    __capture_result('[]');
  }
})();
"####
}

/// `var module = { exports: {} }` wrapper from the original.
pub fn wrap_module(code: &str) -> String {
    format!("var module = {{ exports: {{}} }}; var exports = module.exports;\n(function() {{\n{code}\n}})();")
}
