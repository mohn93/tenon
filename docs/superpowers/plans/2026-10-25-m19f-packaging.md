# M19f Packaging Implementation Plan

**Goal:** Installable packages built by CI as workflow artifacts (macOS dmg, Windows MSI and zip, Linux AppImage and tarball), the version and commit in About, and a manual Help > Check for Updates that reads the latest GitHub release and links to it.

**Architecture:** `crates/app/src/update.rs` holds version comparison, response parsing and a blocking `check()` (ureq, native only); the window spawns it on a thread and gets `AppEvent::Update`. `Sheet::Update` shows the result. `crates/app/build.rs` embeds the git hash and, on Windows with `SKETCHUP_ALT_WINRES` set, the icon resource. Packaging lives in `packaging/` (icons, Info.plist template, WiX source, desktop file), `scripts/package-{macos,linux}.sh`, `scripts/make-icons.py` and `.github/workflows/package.yml`.

**Tech Stack:** Rust 2024. New: `ureq` 3.4.2 (MIT OR Apache-2.0, rustls, native targets only), `winresource` 0.1.34 (MIT, build dependency on Windows hosts only).

## Spec notes (rulings)

1. Name: product "sketchup-alt" (the repo's existing name); the binary stays `app`; bundle id `io.github.mohn93.sketchup-alt`.
2. Icon: drawn by `scripts/make-icons.py` (Pillow) from vector shapes; outputs committed so CI needs no Pillow.
3. macOS: universal binary via lipo in CI (macos-14 can build both targets); minimum macOS 11.0, set in both `MACOSX_DEPLOYMENT_TARGET` and `LSMinimumSystemVersion`; ad-hoc `codesign -s -`; `.skr` document type declared.
4. Windows: hand-written WiX v4 source, `wix` installed as a dotnet tool at a pinned version. The version resource is embedded only when `SKETCHUP_ALT_WINRES` is set, so ordinary builds and CI never depend on the resource compiler.
5. Linux: appimagetool 1.9.0 downloaded and SHA-256 checked; the AppImage bundles only the program.
6. Updates: manual only; `ureq` with a 10 s timeout and 1 MiB body limit; a release link outside `https://github.com/mohn93/sketchup/` falls back to the releases page; a pre-release is older than its release; 404 is "No releases published yet". Hidden on the web via `Features::update_check`.
7. Workflow: separate `package.yml`, `workflow_dispatch` and pushes to main; actions pinned to commit SHAs; `ci.yml` untouched.

## Review Focus

1. Version comparison and response parsing; the no-network guarantee until the item is chosen.
2. The workflow's triggers, pins and artifact names; the WiX source (not buildable locally).
3. Web build still compiles.

## Tasks

- [x] `update.rs` with tests (version order, pre-release, unreadable, parse outcomes, foreign link); sheet, menu and event wiring with sheet tests; About shows version and hash.
- [x] Icons script and outputs; `package-macos.sh` run locally; `package-linux.sh` layout checked with a stand-in binary.
- [x] WiX source, `build.rs` resource, `package.yml`, `docs/packaging.md`, roadmap row.
