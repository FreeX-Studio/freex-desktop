#!/usr/bin/env bash
#
# Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
#
# This file is part of the MirraNET project and is licensed under the
# GNU Lesser General Public License v3.0 (LGPLv3).
#
# You may use, distribute and modify this code under the terms
# of the LGPLv3 license. You should have received a copy of the
# license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
# or contact: niklas.linz@mirranet.de
#
# Builds FreeX Desktop for Windows on Linux: the app (freex-desktop.exe, cross-compiled with
# MinGW-w64) and its installer (dist/freex-desktop-<version>-setup.exe).
#
#   windows/build.sh            app and installer
#   windows/build.sh --app      only the app (dist/freex-desktop.exe)
#
# Needs: cargo (Rust 1.90+), the Rust target x86_64-pc-windows-gnu (rustup target add
# x86_64-pc-windows-gnu), gcc-mingw-w64-x86-64, nsis, curl, unzip, sha256sum.
# A toolchain without the Windows standard library but with rust-src (e.g. the Ubuntu packages
# rustc-1.91 and rust-1.91-src) also works: the standard library is then built from source.
#
# mpv and the WebView2 bootstrapper are downloaded once into a cache (FREEX_DOWNLOAD_CACHE,
# default ~/.cache/freex-build). Set MPV_SHA256 / WEBVIEW2_SHA256 (e.g. in the Jenkins job) to
# pin the downloaded files.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
DIST="$ROOT/dist"
TARGET="x86_64-pc-windows-gnu"

# Bundled programs. To update mpv, change the version.
MPV_VERSION="0.41.0"
MPV_FILE="mpv-v$MPV_VERSION-x86_64-w64-mingw32.zip"
MPV_URL="https://github.com/mpv-player/mpv/releases/download/v$MPV_VERSION/$MPV_FILE"
MPV_SHA256="${MPV_SHA256:-}"
# Evergreen bootstrapper: small, installs the current runtime from Microsoft when needed.
WEBVIEW2_URL="https://go.microsoft.com/fwlink/p/?LinkId=2124703"
WEBVIEW2_SHA256="${WEBVIEW2_SHA256:-}"

CACHE="${FREEX_DOWNLOAD_CACHE:-$HOME/.cache/freex-build}"
mkdir -p "$CACHE" "$DIST"

need() {
    for tool in "$@"; do
        command -v "$tool" >/dev/null || { echo "build: $tool is missing" >&2; exit 1; }
    done
}

# ------------------------------------------------------------------ the app

need cargo rustc x86_64-w64-mingw32-gcc
VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/src-tauri/Cargo.toml" | head -1)"

export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
cargo_args=(build --release --locked --features custom-protocol --target "$TARGET")
libdir="$(rustc --print target-libdir --target "$TARGET" 2>/dev/null || true)"
if ! ls "$libdir"/libstd-*.rlib >/dev/null 2>&1; then
    if [ -d "$(rustc --print sysroot)/lib/rustlib/src/rust/library" ]; then
        echo "build: no prebuilt standard library for $TARGET, building it from rust-src" >&2
        export RUSTC_BOOTSTRAP=1
        cargo_args+=(-Zbuild-std=std,panic_abort)
    else
        echo "build: the Rust target $TARGET is missing (rustup target add $TARGET)" >&2
        exit 1
    fi
fi
(cd "$ROOT/src-tauri" && cargo "${cargo_args[@]}")
EXE="$ROOT/src-tauri/target/$TARGET/release/freex-desktop.exe"
# With the GNU toolchain the WebView2 loader is a DLL next to the exe (from the WebView2 SDK).
LOADER="$ROOT/src-tauri/target/$TARGET/release/WebView2Loader.dll"
cp "$EXE" "$LOADER" "$DIST/"
echo "build: $DIST/freex-desktop.exe (with WebView2Loader.dll)"
[ "${1:-}" = "--app" ] && exit 0

# ------------------------------------------------------------------ the installer

need makensis curl unzip sha256sum

# fetch <url> <file name> -> path in the cache (downloaded once)
fetch() {
    local target="$CACHE/$2"
    if [ ! -s "$target" ]; then
        echo "build: downloading $1" >&2
        curl -fsSL --retry 3 -o "$target.part" "$1"
        mv "$target.part" "$target"
    fi
    echo "$target"
}

# verify <file> <expected sha256 or empty>
verify() {
    [ -n "$2" ] || return 0
    local actual
    actual="$(sha256sum "$1" | cut -d' ' -f1)"
    if [ "$actual" != "$2" ]; then
        rm -f "$1"
        echo "build: checksum of $(basename "$1") does not match (got $actual, expected $2)" >&2
        exit 1
    fi
}

mpv="$(fetch "$MPV_URL" "$MPV_FILE")"
verify "$mpv" "$MPV_SHA256"
webview2="$(fetch "$WEBVIEW2_URL" "MicrosoftEdgeWebview2Setup.exe")"
verify "$webview2" "$WEBVIEW2_SHA256"

# Everything the installer puts into the program folder.
STAGE="$DIST/windows"
rm -rf "$STAGE"
mkdir -p "$STAGE/mpv"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

unzip -q "$mpv" -d "$TMP/mpv"
# The release asset is the CI artifact: a zip that holds the actual build as another zip.
find "$TMP/mpv" -name '*.zip' -print0 | while IFS= read -r -d '' inner; do
    unzip -q "$inner" -d "${inner%.zip}"
done
mpv_exe="$(find "$TMP/mpv" -iname mpv.exe | head -1)"
[ -n "$mpv_exe" ] || { echo "build: mpv.exe not found in $MPV_FILE" >&2; exit 1; }
cp -r "$(dirname "$mpv_exe")"/. "$STAGE/mpv/"
# The console helper is not needed; the app starts mpv.exe directly.
rm -f "$STAGE/mpv/mpv.com" "$STAGE/mpv/updater.bat" "$STAGE/mpv/"*.bat

cp "$EXE" "$LOADER" "$STAGE/"
cp "$ROOT/LICENSE" "$STAGE/LICENSE.txt"
cp "$HERE/THIRD-PARTY.txt" "$STAGE/"

# Windows wants four numbers: 1.0.0 -> 1.0.0.0 (a -SNAPSHOT suffix is dropped).
numeric="${VERSION%%-*}"
while [ "$(tr -cd . <<<"$numeric" | wc -c)" -lt 3 ]; do numeric="$numeric.0"; done

SETUP="$DIST/freex-desktop-$VERSION-setup.exe"
(cd "$HERE" && makensis -V2 -INPUTCHARSET UTF8 \
    -DVERSION="$VERSION" -DFILEVERSION="$numeric" -DSTAGE="$STAGE" -DOUTFILE="$SETUP" \
    -DWEBVIEW2_SETUP="$webview2" freex-desktop.nsi)
echo "build: $SETUP"
