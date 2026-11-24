<p align="center">
  <img src="packaging/branding/nuvio-astra.png" width="120" alt="Nuvio Astra" />
</p>

<h1 align="center">Nuvio Astra</h1>

<p align="center">
  A lightweight, unofficial fork of <a href="https://github.com/NuvioMedia/NuvioDesktop">Nuvio Desktop</a>,
  rewritten in <b>Rust + Tauri</b>. It uses the same Stremio-compatible addons and the same Nuvio account.
</p>

<p align="center">
  <a href="../../releases/latest"><b>Download the latest release</b></a>
</p>

---

## What it is

Nuvio Astra is a desktop player for catalogs, streams, and subtitles supplied by addons you install yourself.
The app ships **no media**. It reads the same addon manifests, catalogs, streams, and subtitle resources as
Nuvio Desktop, and can sync your profiles, addons, favorites, watch progress, Home layout, collections, and
compatible settings with your Nuvio account.

The UI is React/TypeScript; the data layer, Addon protocol, player integration, P2P engine bridge, sync engine,
and all secret handling live in Rust. Secrets (API keys, tokens, credentials) stay in Rust-owned documents and
are never returned to the webview.

| Home | Details |
|---|---|
| ![Home](docs/screenshots/home.jpg) | ![Title details](docs/screenshots/details.jpg) |
| **Search** | **Live TV** |
| ![Search](docs/screenshots/search.jpg) | ![Live TV guide](docs/screenshots/tvaovivo.jpg) |

## Features

### Discovery

- Hero carousel built from the two catalogs you choose.
- Catalog rows you can rename, reorder, or hide, with endless scrolling.
- Account collections with folders, animated GIF covers, and TMDB/Trakt list sources.
- Instant search with history, plus a Discover screen with type/catalog/genre filters from your addons.
- Details pages with seasons, episodes, cast, trailers, similar titles, parental guide, and Trakt comments.
- Hover previews on poster cards (configurable delay, optional muted trailer).
- Custom poster source: a URL template with `{id}`, `{id_type}`, `{typed_id}`, `{type}`, `{shape}`,
  `{imdb_id}`, `{tmdb_id}`, `{tvdb_id}`, `{mal_id}`, `{kitsu_id}`, `{anilist_id}`, `{anidb_id}`,
  optional `{token?}`, and `{first|second}` fallbacks, enabled per screen.

### Playback

- Embedded **libmpv** player with hardware decoding, audio/subtitle tracks, speed control, chapter markers,
  and a source picker overlay.
- Skip intro / recap / credits from video chapters, **IntroDB**, **AniSkip**, and (with your own client id)
  **Anime-Skip**. You can also submit new timings back to IntroDB.
- Automatic subtitle sync with audio for addon/external subtitles, plus manual delay, styling, embedded fonts,
  and legacy subtitle code pages.
- Picture-in-picture, automatic next episode, and a watched threshold from 90% to 100%.
- Direct HTTP, HLS with custom headers (through a local relay), and torrent/P2P playback through the bundled
  Nuvio Engine — playback starts before the download finishes.
- Image quality: Anime4K, sharpening/line-art/denoise shaders, color controls, judder-free or smooth motion,
  and NVIDIA RTX Video Super Resolution/HDR on supported hardware.

### Sources and libraries

- **Debrid:** TorBox, Real-Debrid, AllDebrid, Premiumize, and Debrid-Link. Cached torrents play instantly as
  direct links, without P2P. Keys are per profile and synced with your account.
- **Downloads** from HTTP, TorBox, or torrent sources, with parallel connections and resume.
- **Source badges:** quality, HDR/Dolby Vision, codec, audio, size, and seeds.
- **Scraper plugins:** community Nuvio `manifest.json` repositories, executed in a sandboxed QuickJS engine
  with no file, local-network, or session access.
- **Media servers:** Jellyfin, Emby, Plex (PIN sign-in), and Silo (password or device code). Libraries open with
  the same poster cards and paging as Home.

### Account and sharing

- Profiles with PINs and avatars, favorites, history, and Continue Watching.
- Account sync of profiles, addons, favorites, progress, Home order, collections, and compatible settings.
- **Watch Party:** create or join a room by code and watch together through Supabase Realtime, with Syncplay-style
  drift correction. Requires a Supabase project with Realtime enabled.
- Interface languages: the same 29 as Nuvio Desktop (Portuguese and Brazilian Portuguese, English, Arabic,
  Hebrew, Urdu and 24 more), with right-to-left layouts for Arabic, Hebrew, and Urdu.

### System

- Live TV with categories, favorites, addon filtering, and EPG when the addon provides one.
- In-app automatic updates with signature verification.
- DNS-over-HTTPS fallback for addon/debrid requests, and a local header relay for streams that need custom
  request headers.

## Requirements

- **Windows 10/11 (x64)** with Microsoft Edge WebView2 (included in Windows 11).
- **Linux (experimental):** WebKitGTK 4.1, libmpv 2 (`libmpv.so.2`), and X11/XWayland.

## Install

1. Download `Nuvio-Astra-<version>-x64-setup.exe` from [Releases](../../releases/latest).
2. Run the installer. Administrator rights are not required.
3. Updates arrive inside the app, with a banner showing the version and changelog.

The installer is not yet digitally signed; SmartScreen may ask for confirmation on first run
(**More info → Run anyway**). Release checksums are in `SHA256SUMS.txt`.

On first launch you can sign in with your Nuvio account or continue offline. Existing Nuvio Desktop data is
imported automatically (profiles, addons, library, and settings); the original files are never modified and your
Nuvio password/session is not copied.

Data locations:

- Windows: `%APPDATA%\NuvioNextGen`
- Linux: `$XDG_DATA_HOME/NuvioNextGen` (default `~/.local/share/NuvioNextGen`)

## Build from source

```bash
cargo test --workspace
cd apps/nuvio-desktop/web && npm ci && npm run build && cd ../../..
cargo build --release -p nuvio-desktop
```

Windows packaging (after the build):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\build-release.ps1 -SkipBuild -LibMpv <path\to\libmpv-2.dll>
```

Linux prerequisites (Debian/Ubuntu):

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
    libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Then run `./target/release/nuvio-desktop`.

Optional build configuration goes in `local.properties` (see [`local.properties.example`](local.properties.example)):
account server, update repository, and optional Trakt/Simkl/MDBList client ids. Release automation is documented in
[`GITHUB_PUBLISH.md`](GITHUB_PUBLISH.md); technical notes are in [`docs/`](docs/).

### Environment variables

| Variable | Purpose |
|---|---|
| `NUVIO_DOH_URL` | DNS-over-HTTPS endpoint (default `https://cloudflare-dns.com/dns-query`). |
| `NUVIO_DOH_FORCE=1` | Always try DoH first for public hostnames. |
| `NUVIO_DOH_POISONED` | Comma-separated extra IPs to treat as poisoned DNS answers. |
| `NUVIO_RELAY=0` | Disable the local header relay for streams with custom headers. |
| `NUVIO_MPV_AUDIO_DEVICE` | Pin the mpv audio output device. |
| `NUVIO_LIBMPV`, `NUVIO_MPV_GPU_CONTEXT`, `NUVIO_MPV_HWDEC`, `NUVIO_ENGINE_DLL` | Linux player/engine overrides. |
| `NUVIO_SUPABASE_URL`, `NUVIO_SUPABASE_ANON_KEY`, `NUVIO_SUPABASE_FALLBACK_URL` | Account/Realtime server (build-time). |
| `NUVIO_INTRODB_API_URL`, `NUVIO_ANISKIP_API_URL`, `NUVIO_ANIME_MAPPING_API_URL`, `NUVIO_TRAKT_API_URL` | Optional service endpoint overrides. |

## Project layout

| Path | Contents |
|---|---|
| `apps/nuvio-desktop/web` | React/TypeScript UI. |
| `apps/nuvio-desktop/src` | Tauri commands and application services. |
| `crates/nuvio-core` | Addon protocol, catalog/meta models, skip segments, subsync helpers. |
| `crates/nuvio-store` | Per-profile settings and NextGen migration/transaction layer. |
| `crates/nuvio-sync` | Nuvio account sync engine. |
| `crates/nuvio-player` | libmpv integration (Windows and Linux backends). |
| `crates/nuvio-p2p`, `crates/nuvio-plugins` | Torrent engine bridge and sandboxed scraper plugins. |

## Limitations

Some original features are still partial: cloud-library browsing for debrid accounts is not implemented;
account sign-in from another device (`nuvio.tv/link`) is still missing; Watch Party needs Realtime enabled in the
Supabase project; and hover trailers still render the video stream with a separately synchronized audio track.
Linux now has AppImage/.deb builds and GitHub release links in the updater, but support is still experimental.
The current parity status is tracked in
[`docs/FEATURE_PARITY.md`](docs/FEATURE_PARITY.md), and settings marked **Not yet available** list what is missing.

## Credits and licenses

**Nuvio Astra:** modifications and new code © 2026 [ScottGzin](https://github.com/ScottGzin) and contributors,
licensed under GPL-3.0.

**Original project:** a modified version of [Nuvio Desktop](https://github.com/NuvioMedia/NuvioDesktop),
© Nuvio team / [NuvioMedia](https://github.com/NuvioMedia) and contributors, GPL-3.0. The Nuvio name and brand
belong to their respective owners.

| Component | License |
|---|---|
| Nuvio Engine (libtorrent, Boost, OpenSSL, try_signal) | GPL-3.0-or-later / BSD / BSL / Apache-2.0 |
| mpv / libmpv and FFmpeg | GPL-2.0-or-later / GPL |
| Anime4K | MIT (Unlicense for `AutoDownscalePre`) |
| Tauri, React | MIT or Apache-2.0 / MIT |

Other dependencies use permissive licenses. Third-party notices are in [`packaging/licenses`](packaging/licenses).

**Third-party services:** [IntroDB](https://introdb.app), [AniSkip](https://api.aniskip.com/api-docs),
[ARM](https://github.com/BeeeQueue/arm-server), Trakt, TMDB, YouTube, and every addon author. They are not
affiliated with this project and are governed by their own terms.

## Disclaimer

Nuvio Astra is **not affiliated with, sponsored by, or endorsed by** the Nuvio team. It does not host or provide
any media: it only displays what the addons you install return. You are responsible for the addons you use and for
compliance with local law. With torrent/P2P streams, your IP is visible to other peers.

## Privacy

- Your Nuvio password is sent only to the account server and is never stored. The session is encrypted locally
  (DPAPI on Windows, an AES-256-GCM key file on Linux).
- With skip buttons enabled, the title's public id and season/episode are sent to IntroDB, AniSkip, or ARM.
- Addons receive catalog/source/subtitle requests; subtitle lookups can include the video filename, size, and hash.
- API keys and tokens are kept in Rust-owned files, never in the webview.

## License

[GNU General Public License v3.0](LICENSE). You may use, study, modify, and redistribute this project under those
terms, preserving notices and making the source available. Provided **without warranty**.
