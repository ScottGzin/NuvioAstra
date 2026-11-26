# Player Port — Inventory and Status (Windows x64)

Source of truth for the original behavior: `composeApp/src/desktopMain/native/windows/player_bridge.cpp`,
`desktopMain/.../player/desktop/NativePlayerController.kt`, `DesktopPlayerPictureInPicture.kt`,
`DesktopPlayerPipSizing.kt`, `commonMain/.../player/SubtitleAudioModels.kt`.
Next-gen implementation: `crates/nuvio-player` (`player.rs`, `win.rs`, `mpv.rs`).

Status legend: ✅ ported and verified (self-test or unit test) · 🟡 ported, partially verified ·
⏳ not ported yet · ➕ added on request (not in the original Windows player).

## Inventory

| # | Feature (original) | Original behavior | Next-gen | Verified by |
|---|---|---|---|---|
| 1 | Subtitle style | `sub-ass-override=no` + style-override reset on libass mode change; `sub-font-size` = sp × 3 (18–96), `sub-scale` 1; `sub-pos` = 100 − offset/2 (0–150); `sub-color`, `sub-back-color` + `sub-border-style` (`outline-and-shadow` if transparent, else `background-box`), `sub-outline-color`, `sub-outline-size` (0–8, 0 if disabled), `sub-bold`, change tracking | ✅ same rules and change tracking (`apply_subtitle_style`), default style applied on open | unit test `subtitle_style_mapping…`; self-test reads `sub-pos=90`, `sub-font-size=54` from mpv on all 7 sources |
| 1b | Subtitle appearance | `sub-fonts-dir`, `sub-font`, `sub-shadow-color` (`#AA000000`, alpha from opacity), `sub-shadow-offset` | ✅ | unit test (shadow color) |
| 1c | SDH filter | `sub-filter-sdh` + `sub-filter-sdh-harder` | ✅ | code review against bridge |
| 2 | Subtitle tracks | select by id or `no`; `sub-add <url> select`; remove external tracks (`sub-remove`, reverse order) then select or `no`; `sub-visibility`; `sub-delay` ±60 000 ms | ✅ | self-test: add+select, remove, visibility, delay 250 ms read back |
| 3 | Audio tracks | select by id; preferred languages `alang` then `aid=auto` | ✅ | self-test: switch on HLS (3), MKV (2), addon stream (5) |
| 4 | Audio delay | **not in the original Windows player** (`supportsAudioDelay` false; Android only, ±60 000 ms, 25 ms steps) | ➕ `audio-delay` ±60 000 ms, same limits | self-test: −100 ms read back |
| 5 | Speed | 0.25–4.0 | ✅ | self-test: 1.5 read back |
| 6 | Seek | `absolute+exact` / `relative+exact` | ✅ | self-test: exact seek on long sources |
| 7 | Volume / pause | volume 0–200 (`volume-max=200`), pause flag, toggle | ✅ | self-test: 150 read back |
| 7b | Resize modes | Fit (`keepaspect=yes`, `panscan=0`), Fill/Zoom (`panscan=1.0`), Stretch (`keepaspect=no`) | ✅ | self-test: Zoom → `panscan=1.000000` |
| 7c | Keyboard / mouse | the desktop dispatcher: Space/Enter toggle, ←/→ seek 10 s, ↑/↓ volume, M mute, F/F11 full screen, N next, S subtitles, C chapters, E sources, P PiP, Esc back (panels first). Mouse: wheel seeks 10 s, click toggles playback, double click seeks at the sides / full screen in the middle | ➕/✅ page bindings; Linux re-asserts WebView key focus over the native video window (`player_focus`) | manual Linux check (wheel + keys) |
| 8 | RTX filters | VSR `d3d11vpp=scale=2:scaling-mode=nvidia`; HDR `…nvidia-true-hdr…pq/bt.2020`; both combined; forces `gpu-api=d3d11`, `hwdec=d3d11va`, `d3d11-adapter=NVIDIA` | 🟡 options identical | unit test on generated options; **not run on video yet** (needs RTX settings exposed in UI) |
| 8b | Quality profile | `profile=fast` on unified-memory GPUs without RTX; else `deband`, `spline36` | ✅ (D3D11 `UnifiedMemoryArchitecture` check ported) | unit test; this machine: discrete GPU → deband/spline36 |
| 9 | Picture-in-picture | container moved to an always-on-top window; aspect from `video-out-params`; min 320×180; default 480 wide, bottom-right, 24 px margin; Fit while in PiP; restore | ✅ (Win32 window, aspect lock on resize, drag anywhere, double-click/close restores) | self-test: playback continues in PiP and after restore (all sources); unit test on default bounds |
| 10 | Decoder settings | priority 0 = hardware only (`vd-lavc-software-fallback=no`), 1 = hardware + fallback, 2 = software (`hwdec=no`); `hwdec-codecs=all`; `gpu-shader-cache-dir` | ✅ | unit test; self-test: `d3d11va` on H.264, HEVC Main10, 4K |
| 11 | Stream headers | `http-header-fields` from `proxyHeaders.request` (trimmed, blank skipped, `Range` dropped by `sanitizePlaybackHeaders`) | ✅ **fixed**: passed as a node array (no escaping) | local proxy test: Referer, Cookie, User-Agent, custom token with `,` and `\` all received intact; 403 without them |
| 12 | Network resilience | `stream-lavf-o` reconnect options, demuxer cache 150 MiB / 50 MiB back | ✅ unchanged (not reduced) | options unit test |
| 13 | State to UI | 500 ms polling in the bridge | ✅ property observation, ≤ 4 snapshots/s | self-test |
| — | Not ported yet | chapters / skip intro, next-episode autoplay, source fallback runs, auto-sync subtitles, external subtitle download (addon subtitles), resume position persistence, Discord/now-playing metadata, display-sleep inhibition while playing, watch-party sync | ⏳ | — |

## Original behavior corrected in the next-gen player

- **Header values with a backslash.** The bridge escapes header lines into one string; mpv
  unescapes `\,` but not `\\`, so a value containing `\` arrives doubled. Measured with
  `bench/header-proxy.py` + `examples/header_check.rs`. The next-gen player passes each header line
  as its own string (`MPV_FORMAT_NODE_ARRAY`).

## Earlier statement corrected

The player feasibility notes said the original app "moves subtitles up while controls are
visible". That is wrong: the original applies a fixed style with `sub-pos = 100 − bottomOffset/2`
(90 by default). The spike had not applied that style yet, so subtitles sat at `sub-pos=100`, under
the controls. With the style ported, the default is 90 as in the original.
