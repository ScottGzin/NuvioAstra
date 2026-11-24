# Linux packaging and updates (experimental)

This document covers the two scripts in `packaging/linux/` and how the in-app updater behaves on Linux.
The active CI workflow (`.github/workflows/ci.yml`) is Windows-only; `packaging/linux/ci-linux.yml` is a
**template** to copy into `.github/workflows/` when you are ready to publish Linux artifacts.

Packages are built **from a binary that already exists** — the scripts never compile, and they never read
or write the application data directory (`$XDG_DATA_HOME/NuvioNextGen`).

## Prerequisites

On Debian 12 / Ubuntu 24.04:

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
    libssl-dev libayatana-appindicator3-dev librsvg2-dev dpkg-dev fakeroot
```

Additional inputs (not downloaded by the scripts):

| Input | Where to get it | Used for |
|---|---|---|
| Release binary | `cargo build --release -p nuvio-desktop` | both scripts |
| `libmpv.so.2` | A system install (`libmpv2`), or the same upstream build used on Windows, rebuilt for Linux | both scripts (bundled) |
| `libnuvio_engine.so` | The Nuvio Engine (libtorrent) rebuilt as a Linux shared library with the C ABI from `crates/nuvio-p2p/src/ffi.rs` | both scripts (bundled) |
| `appimagetool` | [AppImageKit continuous release](https://github.com/AppImage/AppImageKit/releases/tag/continuous) | AppImage only |
| `dpkg-deb` / `fakeroot` | `dpkg-dev`, `fakeroot` | `.deb` only |

`build-appimage.sh` exits with the exact `curl` + `chmod` commands when `appimagetool` is not on
`PATH`, at `$APPIMAGETOOL`, or at `<repo>/tools/appimagetool`. Nothing is downloaded automatically.

## Build and package

```bash
# 1. Frontend + Rust release build (from the repository root)
cd apps/nuvio-desktop/web && npm ci && npm run build && cd ../../..
cargo build --release -p nuvio-desktop

# 2. Provide the native libraries (example: system libmpv, engine from the dev tree)
export NUVIO_LIBMPV=/usr/lib/x86_64-linux-gnu/libmpv.so.2
export NUVIO_ENGINE_DLL=apps/nuvio-desktop/native/linux/engine/libnuvio_engine.so

# 3. Package
packaging/linux/build-appimage.sh
packaging/linux/build-deb.sh
```

Outputs go to `dist/`:

- `Nuvio-Astra-<version>-x86_64.AppImage` (+ `.sha256`)
- `nuvio-astra_<version>_amd64.deb` (+ `.sha256`)

The AppImage contains `usr/bin/nuvio-desktop`, the two private libraries in `usr/lib/`, a desktop
entry, a 256×256 icon, the GPL/third-party notices and an `AppRun` that sets
`LD_LIBRARY_PATH`, `NUVIO_LIBMPV` and `NUVIO_ENGINE_DLL` before starting the binary.

The `.deb` installs the same files under `/usr/lib/nuvio-astra/`, the icon and desktop entry under
`/usr/share/`, and a launcher `/usr/bin/nuvio-astra` (so `Exec=nuvio-astra` works from any desktop).
Install it with `sudo apt install ./dist/nuvio-astra_<version>_amd64.deb`.

### Options and environment variables

Both scripts accept `--binary`, `--libmpv`, `--engine`, `--icon`, `--version`, `--out`.
`build-appimage.sh` also accepts `--appimage-tool` and `--arch`; `build-deb.sh` accepts `--arch`.

| Variable | Purpose |
|---|---|
| `CARGO_TARGET_DIR` | Where the release binary and native libraries are looked up (default `<repo>/target`). |
| `NUVIO_LIBMPV` | Path to `libmpv.so.2`; overrides auto-detection. Also used by the player at runtime. |
| `NUVIO_ENGINE_DLL` | Path to `libnuvio_engine.so`; overrides auto-detection. Also used by the P2P bridge at runtime. |
| `APPIMAGETOOL` | Path to the `appimagetool` AppImage. |
| `NUVIO_DEB_DEPENDS` | Replace the `.deb` `Depends` field (use with care; keep WebKitGTK, X11 and libmpv). |
| `NUVIO_DEB_MAINTAINER` | Replace the `.deb` `Maintainer` field. |
| `NUVIO_UPDATE_REPOSITORY` | `owner/repo` embedded by `build.rs` at **build time**; required for in-app updates. |

The default `.deb` dependencies are `libwebkit2gtk-4.1-37` (or the Ubuntu `t64` name), `libgtk-3-0`,
`libx11-6`, `libxcb1`, `libxrandr2`, `libxi6`, `libxkbcommon0`, `libayatana-appindicator3-1`,
`libmpv2`, `libgbm1`, `libdrm2`, `libasound2` (or `t64`) and `libc6`. Adjust for the target distro
with `NUVIO_DEB_DEPENDS` if needed.

## Caveats

- **No code signing for the packages.** The AppImage and `.deb` are not signed with a distro or
  GPG key; users must trust the download source and check `SHA256SUMS.txt` / `*.sha256` themselves.
  The in-app updater *does* verify artifacts cryptographically — see below.
- **AppImage needs FUSE** on the user's machine to mount the image. The scripts build with
  `APPIMAGE_EXTRACT_AND_RUN=1`, and users can set that variable when running too, to extract to a
  temporary directory instead of mounting. AppImages also need the same WebKitGTK/libmpv stack at
  runtime (WebKitGTK cannot reasonably be bundled).
- **Linux support is experimental.** X11/XWayland is required by the native video window; a
  pure-Wayland session without XWayland is not supported yet.
- **libmpv and the P2P engine are GPL-licensed.** Keep `packaging/licenses/` in the package and
  publish the corresponding source, as the project does for the Windows build.
- **Engine ABI matters.** `libnuvio_engine.so` must export exactly the C symbols bound in
  `crates/nuvio-p2p/src/ffi.rs`. A Windows `nuvio_engine.dll` cannot be renamed and reused.
- The `.deb` bundles its own libraries but still declares the system packages above; the launcher
  puts `/usr/lib/nuvio-astra` first in `LD_LIBRARY_PATH`, so the private copies win.

## How updates work on Linux

The updater is the official Tauri 2 updater (`apps/nuvio-desktop/src/updater.rs`), using the same
signed `latest.json` as Windows. The build scripts package a plain `cargo build --release` binary;
they do **not** run `tauri bundle`, so Tauri's compile-time bundle marker is unset. On Linux that
means:

- Target lookup uses only the generic key `linux-x86_64` (or `linux-aarch64`). Use that key in
  `latest.json`; the format-specific `linux-x86_64-appimage` / `linux-x86_64-deb` keys are only
  consulted by binaries patched by the Tauri bundler.
- The installer path inside Tauri defaults to the AppImage installer. Consequently a **signed
  in-app update is supported for the AppImage build**. For a `.deb` installation the updater will
  still announce the release and show the download links, but the automatic install is not
  reliable (it may fail to replace `/usr/lib/nuvio-astra/nuvio-desktop`); `.deb` users should
  update with `sudo apt install ./nuvio-astra_*.deb`. Adding reliable native `.deb` self-update
  would require the Tauri bundler's bundle-patched binary or a dedicated installer command in
  `updater.rs` + `main.rs` (outside the scope of these scripts).

URL downloads must be HTTPS.

`tauri.conf.json` sets `requireSignedVersion: true`, so each Linux artifact must be signed with the
same minisign key as the Windows installer (the `TAURI_SIGNING_PRIVATE_KEY` secret), with the
version in the trusted comment, and the base64 content of the `.sig` file must be stored in
`latest.json` next to the artifact URL. Without a valid signature Tauri refuses to install.

`latest.json` example (add the Linux keys to the existing Windows entry; do not replace it):

```json
{
  "version": "0.2.0",
  "notes": "- Novidades da versão",
  "pub_date": "2026-11-24T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<base64 .sig>",
      "url": "https://github.com/owner/repo/releases/download/v0.2.0/Nuvio-Astra-0.2.0-x64-setup.exe"
    },
    "linux-x86_64": {
      "signature": "<base64 .sig>",
      "url": "https://github.com/owner/repo/releases/download/v0.2.0/Nuvio-Astra-0.2.0-x86_64.AppImage"
    }
  }
}
```

Sign with the same step used for the Windows installer, e.g.
`tauri signer sign -k "$TAURI_SIGNING_PRIVATE_KEY" -p "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" <artifact>`,
then read the generated `<artifact>.sig` into the `signature` field. Never commit the private key.

With a bundle-patched Tauri `.deb`, installation goes through a privilege prompt (`pkexec`/`sudo`);
a signed AppImage update replaces the running AppImage (the file must be writable and on the same
mount point). With the scripts in this directory only the AppImage path applies. Both update paths
preserve `$XDG_DATA_HOME/NuvioNextGen`.

### Read-only GitHub artifact detection

Because existing releases only carry a signed Windows entry, `update_check()` on Linux also does a
read-only `GET https://api.github.com/repos/<owner>/<repo>/releases/latest` (no token, no writes):

- When Tauri finds a signed update, the `.AppImage` / `.deb` assets for the host architecture are
  appended as plain text to the release notes shown in the existing **Settings → Atualizações**
  card (and in the update banner). The "Atualizar agora" button still uses the signed Tauri
  artifact, never these links.
- When the signed manifest has no Linux entry, the check returns an error message containing the
  direct download URLs instead of silently reporting "up to date".
- A failed/unreachable GitHub API call never blocks the signed update check.

This was implemented entirely inside `updater.rs` so no shared file (`main.rs`, `api.rs`,
`types.ts`, `i18n.strings.ts`, ... ) had to change. The URLs are displayed as text because the
existing `ReleaseNotes` renderer does not support Markdown links.

**Optional future hook (not done here, would need shared files):** for clickable artifact cards,
add a `linux_artifacts()` command to `generate_handler!` in `apps/nuvio-desktop/src/main.rs`, add a
typed method and interface in `apps/nuvio-desktop/web/src/api.ts` and
`apps/nuvio-desktop/web/src/types.ts`, and render the new field in
`apps/nuvio-desktop/web/src/screens/Settings.tsx` (the `UpdatesSection` function). No other Rust
change is needed: `linux_releases::fetch()` already returns the data.
