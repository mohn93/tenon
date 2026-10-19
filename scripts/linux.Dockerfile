# Linux build-and-test image for sketchup-alt. Used by scripts/linux-test.sh; mirrors the CI job.
FROM ubuntu:24.04

ARG RUST_VERSION=1.99.0
ENV DEBIAN_FRONTEND=noninteractive

# build-essential gives the C linker; the rest is what the window and the software Vulkan
# adapter (Mesa lavapipe) need at run time. zenity is not needed for tests.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl \
        mesa-vulkan-drivers libvulkan1 libxkbcommon0 libwayland-client0 \
        libx11-6 libxcursor1 libxrandr2 libxi6 libxkbcommon-x11-0 libwayland-cursor0 libwayland-egl1 \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION" -c rustfmt -c clippy

WORKDIR /work
