#!/usr/bin/env bash
# Builds the Linux image and runs fmt, clippy and the tests in it, with the repo mounted
# read-write at /work. Build output goes to a Docker volume, never the host's target/, so the
# script can be run repeatedly and does not disturb a macOS build.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="${SKETCHUP_LINUX_IMAGE:-sketchup-alt-linux}"
volume="${SKETCHUP_LINUX_TARGET_VOLUME:-sketchup-alt-linux-target}"
rust_version="$(rustc --version 2>/dev/null | awk '{print $2}' || true)"

docker build -f "$root/scripts/linux.Dockerfile" \
    ${rust_version:+--build-arg "RUST_VERSION=$rust_version"} -t "$image" "$root/scripts"

docker run --rm \
    -v "$root:/work:ro" \
    -v "$volume:/target" \
    -e CARGO_TARGET_DIR=/target \
    -e CARGO_TERM_COLOR=always \
    -e RUST_BACKTRACE=1 \
    "$image" \
    bash -euxc '
        cargo fmt --all --check
        cargo clippy --workspace --all-targets -- -D warnings
        cargo test --workspace
    '
