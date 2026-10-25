#!/usr/bin/env bash
# Builds target/package/macos/sketchup-alt-<version>-macos-<arch>.dmg: the release build in a
# sketchup-alt.app bundle, ad-hoc signed (`codesign -s -`), in a disk image. It opens no window
# and installs nothing outside target/. Developer ID signing and notarization are not done here
# (docs/packaging.md says what they need).
#
#   scripts/package-macos.sh              the host's architecture only
#   UNIVERSAL=1 scripts/package-macos.sh  aarch64 and x86_64, joined with lipo
#                                         (needs `rustup target add` for both)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# The oldest macOS the build runs on: the app's own binary and its Info.plist agree on it.
min_macos=11.0
export MACOSX_DEPLOYMENT_TARGET="$min_macos"

version="$(awk -F'"' '/^version *=/ { print $2; exit }' Cargo.toml)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
out="$target_dir/package/macos"
app="$out/sketchup-alt.app"

# The commit About shows; a source tarball without git sets SKETCHUP_ALT_GIT_HASH itself.
export SKETCHUP_ALT_GIT_HASH="${SKETCHUP_ALT_GIT_HASH:-$(git rev-parse --short=7 HEAD 2>/dev/null || true)}"

rm -rf "$out"
mkdir -p "$out"

if [ "${UNIVERSAL:-0}" = 1 ]; then
    arch_name=universal
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
        cargo build --release --locked -p app --target "$t"
    done
    binary="$out/app"
    lipo -create -output "$binary" \
        "$target_dir/aarch64-apple-darwin/release/app" \
        "$target_dir/x86_64-apple-darwin/release/app"
else
    arch_name="$(uname -m)"
    [ "$arch_name" = arm64 ] && arch_name=aarch64
    cargo build --release --locked -p app
    binary="$target_dir/release/app"
fi

mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/app"
cp packaging/icons/app.icns "$app/Contents/Resources/app.icns"
cp THIRD_PARTY_NOTICES.md "$app/Contents/Resources/THIRD_PARTY_NOTICES.md"
sed -e "s/@VERSION@/$version/g" -e "s/@MIN_MACOS@/$min_macos/g" \
    packaging/macos/Info.plist.in > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"

# Ad hoc: the bundle is sealed (so a changed file is noticed) but names no developer, so
# Gatekeeper still asks the first time (right-click, Open).
codesign --force --deep -s - "$app"
codesign --verify --deep --strict "$app"

# The image holds the app and a link to /Applications to drag it onto.
stage="$out/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
dmg="$out/sketchup-alt-$version-macos-$arch_name.dmg"
hdiutil create -volname "sketchup-alt $version" -srcfolder "$stage" -ov -format UDZO "$dmg"
rm -rf "$stage"

lipo -archs "$app/Contents/MacOS/app"
echo "$dmg"
