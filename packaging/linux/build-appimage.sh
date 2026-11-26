#!/usr/bin/env bash
# Build a Nuvio Astra AppImage from an already built release binary.
#
# Usage (from any folder):
#   packaging/linux/build-appimage.sh [--binary PATH] [--libmpv PATH] [--engine PATH]
#                                     [--icon PATH] [--version X.Y.Z] [--out DIR]
#                                     [--appimage-tool PATH] [--arch x86_64|aarch64]
#
# Defaults:
#   binary   $CARGO_TARGET_DIR/release/nuvio-desktop (else <repo>/target/release/nuvio-desktop)
#   libmpv   $NUVIO_LIBMPV, then common system locations, then <target>/release/libmpv.so.2
#   engine   $NUVIO_ENGINE_DLL, then <target>/release/libnuvio_engine.so, then the dev tree
#   icon     apps/nuvio-desktop/icons/128x128@2x.png (256x256)
#   out      <repo>/dist
#   tool     $APPIMAGETOOL, then `appimagetool` on PATH, then <repo>/tools/appimagetool
#
# Nothing is downloaded by this script. If appimagetool is missing it prints the exact
# curl/chmod commands for AppImageKit's continuous release and exits.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Build a Nuvio Astra AppImage from an already built release binary.

  --binary PATH         release binary (default: $CARGO_TARGET_DIR/release/nuvio-desktop)
  --libmpv PATH         libmpv.so.2 to bundle (default: NUVIO_LIBMPV / system / target)
  --engine PATH         libnuvio_engine.so to bundle (default: NUVIO_ENGINE_DLL / target)
  --icon PATH           PNG icon, ideally 256x256 (default: 128x128@2x.png)
  --version X.Y.Z       version for the file name (default: tauri.conf.json version)
  --out DIR             output directory (default: <repo>/dist)
  --appimage-tool PATH  appimagetool binary (default: APPIMAGETOOL / PATH / <repo>/tools)
  --arch ARCH           x86_64 or aarch64 (default: uname -m)
  -h, --help            show this help
EOF
}

binary="" libmpv="" engine="" icon="" version="" out="" tool="" arch=""
while (($#)); do
  case "$1" in
    --binary)         binary="${2:-}"; shift 2 ;;
    --libmpv)         libmpv="${2:-}"; shift 2 ;;
    --engine)         engine="${2:-}"; shift 2 ;;
    --icon)           icon="${2:-}"; shift 2 ;;
    --version)        version="${2:-}"; shift 2 ;;
    --out)            out="${2:-}"; shift 2 ;;
    --appimage-tool)  tool="${2:-}"; shift 2 ;;
    --arch)           arch="${2:-}"; shift 2 ;;
    -h|--help)        usage; exit 0 ;;
    *) die "unknown argument: $1 (use --help)" ;;
  esac
done

arch="${arch:-$(uname -m)}"
case "$arch" in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) die "unsupported architecture: $arch" ;;
esac

binary="${binary:-$TARGET_DIR/release/nuvio-desktop}"
[[ -f "$binary" ]] || die "release binary not found: $binary (run 'cargo build --release -p nuvio-desktop' or pass --binary)"

if [[ -z "$libmpv" ]]; then
  for candidate in \
    "${NUVIO_LIBMPV:-}" \
    "$TARGET_DIR/release/libmpv.so.2" \
    "/usr/lib/${arch}-linux-gnu/libmpv.so.2" \
    "/usr/lib64/libmpv.so.2" \
    "$ROOT/apps/nuvio-desktop/native/linux/libmpv.so.2"; do
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
[[ "${icon,,}" == *.png ]] || die "AppImage icon must be a PNG: $icon"

if [[ -z "$version" ]]; then
  version="$(grep -m1 -o '"version"[[:space:]]*:[[:space:]]*"[^"]*"' "$ROOT/apps/nuvio-desktop/tauri.conf.json" | cut -d'"' -f4 || true)"
fi
[[ -n "$version" ]] || die "could not read the version from tauri.conf.json (pass --version)"

if [[ -z "$tool" ]]; then
  if [[ -n "${APPIMAGETOOL:-}" && -x "$APPIMAGETOOL" ]]; then
    tool="$APPIMAGETOOL"
  elif command -v appimagetool >/dev/null 2>&1; then
    tool="$(command -v appimagetool)"
  elif [[ -x "$ROOT/tools/appimagetool" ]]; then
    tool="$ROOT/tools/appimagetool"
  else
    cat >&2 <<EOF
error: appimagetool not found.

Download it once (continuous build, ${arch}):

  mkdir -p "$ROOT/tools"
  curl -fL -o "$ROOT/tools/appimagetool" \\
    "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-${arch}.AppImage"
  chmod +x "$ROOT/tools/appimagetool"

Then re-run this script (or put appimagetool on PATH / set APPIMAGETOOL=/path/to/appimagetool).
appimagetool itself needs FUSE; on machines without it run with APPIMAGE_EXTRACT_AND_RUN=1
(this script already sets it).
EOF
    exit 1
  fi
fi
[[ -x "$tool" ]] || die "appimagetool is not executable: $tool"

if command -v file >/dev/null 2>&1; then
  file "$binary" | grep -q 'ELF' || die "not an ELF binary: $binary"
  # -L: the system libmpv.so.2 is usually a symlink to libmpv.so.2.x.
  [[ "$(file -Lb "$libmpv")" == *ELF* ]] || die "not an ELF shared library: $libmpv"
  [[ "$(file -Lb "$engine")" == *ELF* ]] || die "not an ELF shared library: $engine"
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/nuvio-appimage.XXXXXX")"
trap 'rm -rf "$work"' EXIT
appdir="$work/AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" \
  "$appdir/usr/share/applications" \
  "$appdir/usr/share/icons/hicolor/256x256/apps" \
  "$appdir/usr/share/doc/nuvio-astra"

install -m 755 "$binary" "$appdir/usr/bin/nuvio-desktop"
install -m 644 "$libmpv" "$appdir/usr/lib/libmpv.so.2"
install -m 644 "$engine" "$appdir/usr/lib/libnuvio_engine.so"
install -m 644 "$icon" "$appdir/usr/share/icons/hicolor/256x256/apps/nuvio-astra.png"

cat > "$appdir/usr/share/applications/nuvio-astra.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Nuvio Astra
GenericName=Media Player
Comment=Stremio-compatible addon media player
Exec=nuvio-astra %u
Icon=nuvio-astra
Terminal=false
Categories=AudioVideo;Video;Player;
Keywords=stream;video;addon;stremio;
StartupNotify=true
StartupWMClass=nuvio-desktop
MimeType=x-scheme-handler/nuvio;
EOF
# Older appimagetool builds look for the entry at the AppDir root; newer ones accept the copy in
# usr/share/applications. Keep both so the image can be built with either.
cp "$appdir/usr/share/applications/nuvio-astra.desktop" "$appdir/nuvio-astra.desktop"
cp "$appdir/usr/share/icons/hicolor/256x256/apps/nuvio-astra.png" "$appdir/nuvio-astra.png"
cp "$appdir/usr/share/icons/hicolor/256x256/apps/nuvio-astra.png" "$appdir/.DirIcon"

cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
# Keep the bundled native libraries first: libmpv.so.2 and the P2P engine are private to the app.
HERE="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
export LD_LIBRARY_PATH="$HERE/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export NUVIO_LIBMPV="${NUVIO_LIBMPV:-$HERE/usr/lib/libmpv.so.2}"
export NUVIO_ENGINE_DLL="${NUVIO_ENGINE_DLL:-$HERE/usr/lib/libnuvio_engine.so}"
exec "$HERE/usr/bin/nuvio-desktop" "$@"
EOF
chmod 755 "$appdir/AppRun"
ln -sf AppRun "$appdir/nuvio-astra"

if [[ -d "$ROOT/packaging/licenses" ]]; then
  cp -R "$ROOT/packaging/licenses" "$appdir/usr/share/doc/nuvio-astra/licenses"
fi

out="${out:-$ROOT/dist}"
mkdir -p "$out"
image="$out/Nuvio-Astra-${version}-${arch}.AppImage"
printf 'Building %s from %s\n' "$image" "$appdir"
ARCH="$arch" APPIMAGE_EXTRACT_AND_RUN=1 "$tool" "$appdir" "$image"
chmod 755 "$image"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$out" && sha256sum "$(basename "$image")" > "$(basename "$image").sha256")
fi
printf 'AppImage: %s\n' "$image"
