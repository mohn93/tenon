#!/usr/bin/env bash
# Builds target/package/linux/sketchup-alt-<version>-linux-<arch>.tar.gz and, when APPIMAGETOOL
# names an appimagetool, the matching .AppImage. The AppImage holds only the program: the
# graphics driver and window libraries come from the system (docs/linux.md).
#
#   scripts/package-linux.sh
#   APPIMAGETOOL=/path/to/appimagetool-x86_64.AppImage scripts/package-linux.sh
#
# APPIMAGE_RUNTIME names the AppImage runtime to use (CI pins and checks one); without it
# appimagetool downloads the latest.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

version="$(awk -F'"' '/^version *=/ { print $2; exit }' Cargo.toml)"
arch="$(uname -m)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
out="$target_dir/package/linux"
name="sketchup-alt-$version-linux-$arch"

export SKETCHUP_ALT_GIT_HASH="${SKETCHUP_ALT_GIT_HASH:-$(git rev-parse --short=7 HEAD 2>/dev/null || true)}"

# APP_BINARY skips the build (to try the layout with a program built earlier).
binary="${APP_BINARY:-$target_dir/release/app}"
if [ -z "${APP_BINARY:-}" ]; then
    cargo build --release --locked -p app
fi

rm -rf "$out"
mkdir -p "$out/$name"
cp "$binary" "$out/$name/app"
cp packaging/linux/sketchup-alt.desktop "$out/$name/"
cp packaging/icons/app-256.png "$out/$name/sketchup-alt.png"
cp THIRD_PARTY_NOTICES.md docs/linux.md "$out/$name/"
tar -C "$out" -czf "$out/$name.tar.gz" "$name"
rm -rf "$out/$name"

if [ -n "${APPIMAGETOOL:-}" ]; then
    appdir="$out/sketchup-alt.AppDir"
    mkdir -p "$appdir/usr/bin"
    cp "$binary" "$appdir/usr/bin/app"
    cp packaging/linux/AppRun "$appdir/AppRun"
    cp packaging/linux/sketchup-alt.desktop "$appdir/"
    cp packaging/icons/app-256.png "$appdir/sketchup-alt.png"
    cp THIRD_PARTY_NOTICES.md "$appdir/"
    # No FUSE is needed to run the tool itself (CI has none).
    runtime=()
    if [ -n "${APPIMAGE_RUNTIME:-}" ]; then
        runtime=(--runtime-file "$APPIMAGE_RUNTIME")
    fi
    APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" "$APPIMAGETOOL" ${runtime[@]+"${runtime[@]}"} "$appdir" "$out/$name.AppImage"
    rm -rf "$appdir"
fi

ls -l "$out"
