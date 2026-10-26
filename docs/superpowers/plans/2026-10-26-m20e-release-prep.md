# M20e Release Preparation Implementation Plan

**Goal:** The licence notices of the Rust crates compiled into the app ship in every package and CI keeps them honest; the pinned GitHub Actions run on Node 24; the macOS dmg step survives a runner disk hiccup.

**Architecture:** `scripts/licenses.py` (Python standard library) asks `cargo metadata --filter-platform` for each packaged target, follows normal dependency edges from `app` (five native targets) and `web` (wasm32), reads each crate's licence files from the cargo registry cache and writes a generated section at the end of `THIRD_PARTY_NOTICES.md`, which every package already carries. `--check` is a CI step. Workflow pins and `scripts/package-macos.sh` are edited in place.

**Tech Stack:** Python 3 stdlib, cargo, GitHub Actions, hdiutil. No new Rust dependencies, no installed tools in CI.

## Spec notes (rulings)

1. Tool: a script, not cargo-about or cargo-deny: nothing to pin or install, and the check, the allowed list and the text grouping are about 200 lines. The generated text lives in `THIRD_PARTY_NOTICES.md` between markers, so no package script or the WiX source changes; the AppImage (which lacked the file) and `web/dist` now carry it too.
2. Scope: crates reachable through normal edges only (dev and build-only crates are not shipped); proc-macro crates are included (conservative). Workspace crates are ours; the vendored `egui-winit` is listed.
3. Texts: every licence, copying and notice file in the package outside tests and examples (ring keeps BoringSSL, ISC and fiat licences in subfolders); CRLF folded, identical texts printed once with the crates that carry them. Crates that publish no licence file take upstream copies from `packaging/licenses/`.
4. Allowed: MIT, Apache-2.0 (with LLVM-exception), BSD-2/3-Clause, ISC, Zlib, 0BSD, Unlicense, CC0-1.0, BSL-1.0, OpenSSL, Unicode-3.0, Unicode-DFS-2016, CDLA-Permissive-2.0, OFL-1.1, Ubuntu-font-1.0, MPL-2.0 (unmodified crates only). An OR needs one allowed branch, an AND all terms.
5. ring 0.17.14 declares `Apache-2.0 AND ISC`; its package also holds MIT texts and no OpenSSL text, so it is overridden to `Apache-2.0 AND ISC AND MIT`.
6. Actions: checkout v7.0.1, cache v6.1.0, upload-artifact v7.0.2 (all `node24`), pinned by commit SHA in ci.yml as well as package.yml. Release notes checked: checkout v6 stores credentials in a separate file and v7 blocks fork checkouts under `pull_request_target`/`workflow_run` (unused here); upload-artifact v7 adds an opt-in `archive: false` (default unchanged); cache v6 is an ESM rewrite.
7. dmg: `-size` is the staged folder plus 20% and 20 MB; up to four attempts, pausing 5, 10, 15 s, detaching a leftover volume and removing a partial image between them.

## Review Focus

1. Crates missing from the generated list (target coverage) and the allowed list.
2. The workflow pins against their tags.
3. The dmg retry keeps `set -euo pipefail` semantics.

## Tasks

- [x] `scripts/licenses.py`, its tests, `packaging/licenses/`, the generated section, CI step, docs.
- [x] Actions moved and pinned; dmg sizing and retry (dry run with a stand-in folder); AppImage and web build carry the notices.
