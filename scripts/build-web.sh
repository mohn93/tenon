#!/usr/bin/env bash
# Builds the browser version (the `web` profile: size-optimised) into web/dist: index.html, web.js and web_bg.wasm. Needs the
# wasm32-unknown-unknown target (`rustup target add wasm32-unknown-unknown`) and wasm-bindgen-cli
# 0.2.129, the version the workspace pins (`cargo install wasm-bindgen-cli --version 0.2.129`).
# Serve the result with scripts/serve-web.sh.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

cargo build -p web --target wasm32-unknown-unknown --profile web
target_dir="${CARGO_TARGET_DIR:-$root/target}"

mkdir -p web/dist
wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
    --out-dir web/dist \
    "$target_dir/wasm32-unknown-unknown/web/web.wasm"
cp web/index.html web/dist/index.html

echo "built web/dist:"
ls -l web/dist
