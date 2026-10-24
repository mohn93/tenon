# Testing

`cargo test --workspace` runs everything. Rendering tests need a GPU adapter; without one they
print "no GPU adapter, skipping" and pass. There are no doc tests, so
`cargo test --workspace --lib --bins --tests` runs the same tests without building doc-test
runners. Dependencies are built optimized in dev and test builds (`[profile.dev.package."*"]` in
the root `Cargo.toml`); the workspace crates stay unoptimized. Run tests with `cargo test`, not a
process-per-test runner such as cargo-nextest: the rendering tests then each create their own GPU
device and the run takes longer.

Each crate's integration tests are modules of one test binary, `tests/integration/main.rs`, so a
full run starts one test program per crate (on macOS every new program is scanned before it runs,
which made one binary per file slow). Run one file's tests by module path:
`cargo test -p session --test integration scale::`. A new test file goes in
`tests/integration/` with a `mod` line in `main.rs`; tests in different files now run in parallel
in one process, so they must not share fixed temp paths or global state.

The `app` crate is a library with a thin binary: its unit tests are in the library, so run them
with `cargo test -p app --lib <filter>`; the binary's integration tests (the headless and MCP
modes) are `cargo test -p app --test integration`. The shared crates and the app library also
build for the browser: `cargo check --target wasm32-unknown-unknown -p kernel -p session -p tools
-p render -p api -p app --lib -p web` (needs `rustup target add wasm32-unknown-unknown`); `scripts/build-web.sh`
builds the page (docs/web.md).

## Keeping test runs fast

A full `cargo test --workspace --lib --bins --tests` runs in about 80 s on an M1 Max once built
(an incremental test build after a change in `session` is about 15 s). Two things kept it near
five minutes before 2026-10-24:

- On macOS, Metal reads the folder of the program that starts it (`target/debug/deps` for a test
  binary) before the first GPU device exists. Cargo never deletes old artifacts, and with debug
  info each build left hundreds of new object files there; at 700,000 files every GPU test
  binary waited about 25 s before its first render. The workspace crates now build without debug
  info (`[profile.dev] debug = false`), which leaves no object files behind, and
  `python3 scripts/prune-target.py` removes stale artifacts while keeping what the current build
  uses. If GPU tests are slow again, count the files in `target/debug/deps` first.
- Tests that build very large meshes (a 360 by 360 torus, 10,000-point curves) take seconds in an
  unoptimized test build. Test the largest accepted sizes by building the description or checking
  the limit, and keep full model builds and validation to moderate sizes; put timing checks of
  big models in the ignored `perf` modules.

While working on one crate, run its tests (`cargo test -p session --test integration`,
`cargo test -p kernel --lib`) and clippy for the crates you touched; run the full gate once before
committing a task.

## Golden images

`crates/render/tests/integration/golden.rs` renders a few canonical scenes headless at 160 x 120 and compares
each with its PNG in `crates/render/tests/golden/`. A pixel differs when one of its color channels
differs by more than 24 (out of 255); a test fails when more than 0.5% of its pixels differ, which
leaves room for GPU and driver differences in antialiasing. Changes under that tolerance, such as
small shading shifts, are not caught.

On a failure the test prints two paths under `target/tmp/golden/`: `<name>.actual.png` (this
render) and `<name>.diff.png` (the render in gray with differing pixels in red).

When a rendering change is intended, regenerate the stored images and look at every changed one
before committing it. Regenerate only with the intended rendering change applied (no experimental
or probe edits): `UPDATE_GOLDEN` accepts whatever renders. With a test filter, only the matching
images are rewritten.

```
UPDATE_GOLDEN=1 cargo test -p render --test integration golden::
git diff --stat crates/render/tests/golden/
```

A new scene needs a test calling `check("<name>", &doc)` and its name in
`every_golden_image_has_a_test`; run with `UPDATE_GOLDEN=1` once to store its image.

## Timings

`crates/kernel/tests/integration/perf.rs` times big edits (copying 30 circles three and a hundred times,
copying an exploded 1,800-face terrain, edits inside a 20,000-face imported group, exploding
imported groups of 1,800 and 20,000 faces, softening and shading a 20,000-face terrain, intersecting two 20,000-face terrains and two 4,000-edge coplanar faces, drawing a 10,000-corner polygon). Its tests are ignored by default;
run them in release, where they also check loose upper bounds:

```
cargo test --release -p kernel --test integration perf:: -- --ignored --nocapture
```

`crates/session/tests/integration/perf.rs` times saving and opening a big model, purging many definitions,
Intersect Faces over 22,500 placed boxes with Select All (With Model, about 0.1 s):

```
cargo test --release -p session --test integration perf:: -- --ignored --nocapture
```

`crates/session/tests/integration/small_tools.rs` has ignored release timings of `add_curve`, among them
`two_serpentines_are_refused_or_drawn_in_under_a_second` and a self-crossing star (each under a second):

```
cargo test --release -p session --test integration small_tools:: -- --ignored --nocapture
```
