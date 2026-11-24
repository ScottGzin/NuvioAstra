# Contract fixtures

Two real artifacts from the Nuvio plugin ecosystem are kept here so the Rust contract can be tested
against the shapes third-party repositories actually use. They are data for tests only; the app
ships no plugins.

- `michat88-manifest.json` — <https://raw.githubusercontent.com/michat88/nuvio-providers/main/manifest.json>
  Repository `Michat88 Repo`, version `1.0.0`, 30 scrapers. Shows the `name` / `version` /
  `scrapers[] { id, name, version, filename, supportedTypes, enabled, ... }` format of the original
  app. Repository licensed GPL-3.0.
- `kisskh.js` — <https://raw.githubusercontent.com/michat88/nuvio-providers/main/providers/kisskh.js>
  A real scraper using the original host API: `require('cheerio-without-node-native')`, global
  `fetch`, Promise-based `getStreams(tmdbId, mediaType, seasonNum, episodeNum)`, and a CommonJS
  export. Its stream objects carry `name`, `title`, `url`, `quality`, `headers` and `provider`.

Fetched 2026-11-23. Both come from the same GPL-3.0 repository. Do not use these files as product
branding or as bundled plugin content.
