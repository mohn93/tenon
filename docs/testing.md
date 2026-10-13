# Testing

`cargo test --workspace` runs everything. Rendering tests need a GPU adapter; without one they
print "no GPU adapter, skipping" and pass.

## Golden images

`crates/render/tests/golden.rs` renders a few canonical scenes headless at 160 x 120 and compares
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
UPDATE_GOLDEN=1 cargo test -p render --test golden
git diff --stat crates/render/tests/golden/
```

A new scene needs a test calling `check("<name>", &doc)` and its name in
`every_golden_image_has_a_test`; run with `UPDATE_GOLDEN=1` once to store its image.

## Timings

`crates/kernel/tests/perf.rs` times big edits (copying 30 circles three and a hundred times,
copying an exploded 1,800-face terrain, edits inside a 20,000-face imported group, exploding
imported groups of 1,800 and 20,000 faces). Its tests are ignored by default;
run them in release, where they also check loose upper bounds:

```
cargo test --release -p kernel --test perf -- --ignored --nocapture
```
