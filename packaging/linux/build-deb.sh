#!/usr/bin/env bash
# Build a Debian/Ubuntu .deb from an already built release binary.
#
# Usage (from any folder):
#   packaging/linux/build-deb.sh [--binary PATH] [--libmpv PATH] [--engine PATH]
#                                [--icon PATH] [--version X.Y.Z] [--out DIR] [--arch amd64]
#
# Layout of the package:
#   /usr/bin/nuvio-astra                 launcher (sets LD_LIBRARY_PATH + native library overrides)
#   /usr/lib/nuvio-astra/nuvio-desktop   release binary
#   /usr/lib/nuvio-astra/libmpv.so.2     bundled player
#   /usr/lib/nuvio-astra/libnuvio_engine.so  bundled P2P engine
#   /usr/share/applications/nuvio-astra.desktop
#   /usr/share/icons/hicolor/256x256/apps/nuvio-astra.png
#   /usr/share/doc/nuvio-astra/...       license + notices
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
PKG=nuvio-astra
LIBDIR="/usr/lib/$PKG"

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Build a Debian/Ubuntu .deb from an already built release binary.

  --binary PATH    release binary (default: $CARGO_TARGET_DIR/release/nuvio-desktop)
  --libmpv PATH    libmpv.so.2 to bundle (default: NUVIO_LIBMPV / system / target)
  --engine PATH    libnuvio_engine.so to bundle (default: NUVIO_ENGINE_DLL / target)
  --icon PATH      PNG icon, ideally 256x256 (default: 128x128@2x.png)
  --version X.Y.Z  version for the package (default: tauri.conf.json version)
  --out DIR        output directory (default: <repo>/dist)
  --arch ARCH      Debian architecture (default: dpkg --print-architecture, else uname -m)
  -h, --help       show this help

Environment:
  NUVIO_DEB_DEPENDS  override the Depends field (space/comma separated package list).
  NUVIO_DEB_MAINTAINER  override the Maintainer field.
EOF
}

binary="" libmpv="" engine="" icon="" version="" out="" arch=""
while (($#)); do
  case "$1" in
    --binary)  binary="${2:-}"; shift 2 ;;
    --libmpv)  libmpv="${2:-}"; shift 2 ;;
    --engine)  engine="${2:-}"; shift 2 ;;
    --icon)    icon="${2:-}"; shift 2 ;;
    --version) version="${2:-}"; shift 2 ;;
    --out)     out="${2:-}"; shift 2 ;;
    --arch)    arch="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (use --help)" ;;
  esac
done

if [[ -z "$arch" ]]; then
  if command -v dpkg >/dev/null 2>&1; then
    arch="$(dpkg --print-architecture)"
  else
    case "$(uname -m)" in
      x86_64|amd64) arch=amd64 ;;
      aarch64|arm64) arch=arm64 ;;
      *) die "unsupported architecture: $(uname -m)" ;;
    esac
  fi
fi

binary="${binary:-$TARGET_DIR/release/nuvio-desktop}"
[[ -f "$binary" ]] || die "release binary not found: $binary (run 'cargo build --release -p nuvio-desktop' or pass --binary)"

if [[ -z "$libmpv" ]]; then
  for candidate in \
    "${NUVIO_LIBMPV:-}" \
    "$TARGET_DIR/release/libmpv.so.2" \
    "/usr/lib/x86_64-linux-gnu/libmpv.so.2" \
    "/usr/lib/aarch64-linux-gnu/libmpv.so.2" \
    "/usr/lib64/libmpv.so.2"; do
    [[ -n "$candidate" && -f "$candidate" ]] && { libmpv="$candidate"; break; }
  done
fi
[[ -f "$libmpv" ]] || die "libmpv.so.2 not found (pass --libmpv or set NUVIO_LIBMPV)"

if [[ -z "$engine" ]]; then
  for candidate in \
    "${NUVIO_ENGINE_DLL:-}" \
    "$TARGET_DIR/release/libnuvio_engine.so" \
    "$ROOT/apps/nuvio-desktop/native/linux/engine/libnuvio_engine.so"; do
    [[ -n "$candidate" && -f "$candidate" ]] && { engine="$candidate"; break; }
  done
fi
[[ -f "$engine" ]] || die "libnuvio_engine.so not found (pass --engine or set NUVIO_ENGINE_DLL)"

icon="${icon:-$ROOT/apps/nuvio-desktop/icons/128x128@2x.png}"
[[ -f "$icon" ]] || icon="$ROOT/apps/nuvio-desktop/icons/icon.png"
[[ -f "$icon" ]] || die "PNG icon not found (pass --icon)"
[[ "${icon,,}" == *.png ]] || die "desktop icon must be a PNG: $icon"

if [[ -z "$version" ]]; then
  version="$(grep -m1 -o '"version"[[:space:]]*:[[:space:]]*"[^"]*"' "$ROOT/apps/nuvio-desktop/tauri.conf.json" | cut -d'"' -f4 || true)"
fi
[[ -n "$version" ]] || die "could not read the version from tauri.conf.json (pass --version)"
command -v dpkg-deb >/dev/null 2>&1 || die "dpkg-deb not found (install dpkg-dev)"

# Runtime package names differ between distributions: Debian/Ubuntu 24.04 ship the 4.1 WebKitGTK
# runtime as `libwebkit2gtk-4.1-0` (the `-37` suffix belongs to the older 4.0 series), and Ubuntu's
# t64 transition renamed some libraries. The alternatives cover Debian 12, Ubuntu 24.04 and PikaOS.
depends="${NUVIO_DEB_DEPENDS:-libwebkit2gtk-4.1-0 | libwebkit2gtk-4.1-0t64, libgtk-3-0 | libgtk-3-0t64, libx11-6, libxcb1, libxrandr2, libxi6, libxkbcommon0, libayatana-appindicator3-1, libmpv2, libgbm1, libdrm2, libasound2 | libasound2t64, libc6}"
maintainer="${NUVIO_DEB_MAINTAINER:-Nuvio Astra <maintainer@nuvio-astra.invalid>}"

work="$(mktemp -d "${TMPDIR:-/tmp}/nuvio-deb.XXXXXX")"
trap 'rm -rf "$work"' EXIT
pkgdir="$work/$PKG"
mkdir -p "$pkgdir/DEBIAN" "$pkgdir/usr/bin" "$pkgdir$LIBDIR" \
  "$pkgdir/usr/share/applications" \
  "$pkgdir/usr/share/icons/hicolor/256x256/apps" \
  "$pkgdir/usr/share/doc/$PKG"

install -m 755 "$binary" "$pkgdir$LIBDIR/nuvio-desktop"
install -m 644 "$libmpv" "$pkgdir$LIBDIR/libmpv.so.2"
install -m 644 "$engine" "$pkgdir$LIBDIR/libnuvio_engine.so"
install -m 644 "$icon" "$pkgdir/usr/share/icons/hicolor/256x256/apps/nuvio-astra.png"

cat > "$pkgdir/usr/share/applications/nuvio-astra.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Nuvio Astra
GenericName=Media Player
Comment=Stremio-compatible addon media player
Exec=/usr/bin/nuvio-astra %u
Icon=nuvio-astra
Terminal=false
Categories=AudioVideo;Video;Player;
Keywords=stream;video;addon;stremio;
StartupNotify=true
StartupWMClass=nuvio-desktop
MimeType=x-scheme-handler/nuvio;
EOF

cat > "$pkgdir/usr/bin/nuvio-astra" <<EOF
#!/bin/sh
# Prefer the private copies of libmpv / the P2P engine shipped with this package.
export LD_LIBRARY_PATH="$LIBDIR\${LD_LIBRARY_PATH:+\$LD_LIBRARY_PATH}"
export NUVIO_LIBMPV="\${NUVIO_LIBMPV:-$LIBDIR/libmpv.so.2}"
export NUVIO_ENGINE_DLL="\${NUVIO_ENGINE_DLL:-$LIBDIR/libnuvio_engine.so}"
exec "$LIBDIR/nuvio-desktop" "\$@"
EOF
chmod 755 "$pkgdir/usr/bin/nuvio-astra"

{
  printf 'Package: %s\n' "$PKG"
  printf 'Version: %s\n' "$version"
  printf 'Section: video\n'
  printf 'Priority: optional\n'
  printf 'Architecture: %s\n' "$arch"
  printf 'Depends: %s\n' "$depends"
  printf 'Maintainer: %s\n' "$maintainer"
  printf 'Description: Stremio-compatible addon media player\n'
  printf ' Nuvio Astra plays catalogs, streams and subtitles supplied by addons you install.\n'
  printf ' It bundles libmpv and the Nuvio P2P engine; no media is shipped with the app.\n'
} > "$pkgdir/DEBIAN/control"

cat > "$pkgdir/usr/share/doc/$PKG/copyright" <<'EOF'
Nuvio Astra — GPL-3.0-only
See /usr/share/doc/nuvio-astra/licenses/ for the full license and third-party notices.
The bundled libmpv/mpv is GPL; the Nuvio Engine is GPL with bundled third-party licenses.
EOF
if [[ -d "$ROOT/packaging/licenses" ]]; then
  cp -R "$ROOT/packaging/licenses" "$pkgdir/usr/share/doc/$PKG/licenses"
fi

out="${out:-$ROOT/dist}"
mkdir -p "$out"
deb="$out/${PKG}_${version}_${arch}.deb"
printf 'Building %s from %s\n' "$deb" "$pkgdir"
if command -v fakeroot >/dev/null 2>&1; then
  fakeroot dpkg-deb --build "$pkgdir" "$deb"
else
  # --root-owner-group gives root ownership without fakeroot (dpkg-dev >= 1.19).
  dpkg-deb --build --root-owner-group "$pkgdir" "$deb"
fi
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$out" && sha256sum "$(basename "$deb")" > "$(basename "$deb").sha256")
fi
printf 'Debian package: %s\n' "$deb"
