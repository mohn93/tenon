# M8b Golden Images and Toolbar Wrapping Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eight canonical scenes are rendered headless and compared with stored golden PNGs within a tolerance (UPDATE_GOLDEN=1 regenerates them), and the toolbar wraps onto more rows in narrow windows so no button is clipped.

**Architecture:**
- Golden tests live entirely in `crates/render/tests/golden.rs`; the library is unchanged.
- Rendering goes through `Snapshotter::capture` (the `render_view` path), so the tests cover what agents see; scenes come from `session::Document` commands, so they track kernel behavior rather than hand-made models.
- Tolerant comparison: per-channel threshold 24 plus at most 0.5% differing pixels, alpha ignored; actual and diff PNGs in `target/tmp/golden/` on failure; `UPDATE_GOLDEN=1` regenerates.
- PNG I/O uses the existing workspace `png` crate as a render dev-dependency; no new crates.
- The toolbar is a single `horizontal_wrapped` layout inside the top panel; the 3D view's rect already comes from what the panels leave, so a taller toolbar needs no other change.

**Tech Stack:** Rust 2024; `png` added to `render` as a dev-dependency only (already a workspace dependency used by `api`; no new crate).

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`: the Testing section and the milestone list (Task 1 adds the M8b line).

## Global Constraints

- Every new capability is a `session::Command` reachable through the API, covered by a command-driven test. Tools only emit commands.
- No new crates (the `png` dev-dependency reuses the workspace crate).
- Git:
  - work on `dev`;
  - stage only the files you changed;
  - short plain commit messages, no emoji;
  - run `cargo fmt --all` before each commit;
  - `cargo clippy --workspace --all-targets -- -D warnings` stays clean.
- Do not launch the app window.

## Spec notes (rulings made in this plan)

- **Where**: one integration test file, `crates/render/tests/golden.rs`, with its helpers (compare, diff image, PNG read/write) in the file, not in the library: nothing outside the tests needs them.
- **Capture path**: scenes are rendered with the existing `render::Snapshotter` (`new_headless` + `capture`), the same path as the API's `render_view`, with the document's camera, selection and open context. A test without a GPU adapter prints "no GPU adapter, skipping" and passes, as the other headless tests do.
- **Scenes are built with `session::Document` commands** (render already depends on session), not hand-made models: `add_rectangle`, `push_pull`, `add_circle`, `paint`, `make_component`, `copy`, `rotate`, `add_guide_line/point`, `make_group`, `open_instance`, `select`, `set_camera`, `standard_view`, `set_projection`, `zoom_extents`.
- **Eight scenes, 160 x 120** (each a `#[test]`): `box` (2 x 1.5 x 1 m), `painted_box` (top erased, every face front "Brick" [190,70,50], back "Leaf" [60,170,80]; seen from above, outside brick, inside green), `rotated_component` (component plus a copy turned 35 degrees about its center, selection cleared), `guides` (ground rectangle, two guide lines, one guide point), `soft_cylinder` (24-segment circle pushed 1.5 m; soft sides hidden), `parallel_top` (box and circle, Top view, parallel, zoom extents), `selection` (top face and one vertical edge selected), `open_group` (a grouped box opened, a loose box beside it faded, context box outline). PNGs are 1.6-8.9 KB each, about 56 KB total.
- **Boxes are raised whichever way the base faces**: the helper reads the face normal and push-pulls by +h or -h, so the scenes do not depend on the rectangle's orientation rule.
- **Tolerance**: a pixel differs when any of R, G, B differs by more than `CHANNEL_TOLERANCE` = 24; alpha is ignored; an image matches when at most `MAX_DIFFERING` = 0.5% of pixels differ (96 of 19,200). Measured: showing the cylinder's soft edges makes 785 pixels (4.1%) differ, so real regressions fail; renders on this machine are identical run to run. Small color shifts (under 24 levels) pass by design.
- **Stored format**: RGBA8 PNG written with the `png` crate (already a workspace dependency used by `api`), added to `render` as a dev-dependency only. Cargo.lock gains one line (`png` in render's dependency list); no new crate. Reading expands palette/grayscale and accepts RGB or RGBA 8-bit.
- **UPDATE_GOLDEN=1** (exactly "1") makes each scene test write its PNG to `crates/render/tests/golden/<name>.png` and pass. Command: `UPDATE_GOLDEN=1 cargo test -p render --test golden`.
- **Failure output**: on a mismatch, `<name>.actual.png` and `<name>.diff.png` (render in dimmed gray, differing pixels red) go to `$CARGO_TARGET_TMPDIR/golden/` (i.e. `target/tmp/golden/`), and the panic message names both paths, the comparison counts and the UPDATE_GOLDEN hint. A missing or unreadable golden image, or one of another size, also fails (writing the actual image) rather than passing silently.
- **Stale images**: `every_golden_image_has_a_test` checks that the PNGs in `tests/golden/` are exactly the eight scene names (it returns early under UPDATE_GOLDEN, while the other tests write).
- **Developer doc**: there is no README; a new short `docs/testing.md` explains GPU skipping, the tolerance, the failure images and how to regenerate and review golden images. The spec's Testing section points at it.
- **Toolbar wrapping**: the toolbar's `ui.horizontal` becomes `ui.horizontal_wrapped`; nothing else changes (menus, separators, order). At 1280 points it stays one row (same height as at 2400); at 900 and 640 points it wraps and every tool, Undo, Redo, Make group, Make component, Explode and Close group stays clickable. No minimum window size is set.
- **Spec**: the status amendment line gains "M8b golden images"; the M8b milestone line gets the full scope (scenes, size, tolerance, failure images, UPDATE_GOLDEN, skip without GPU; toolbar wrapping in Task 2).

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/render/tests/golden.rs` | scenes, render, compare, diff image, PNG read/write, UPDATE_GOLDEN | 1 |
| `crates/render/tests/golden/*.png` | eight stored 160 x 120 renders (binary, generated with UPDATE_GOLDEN=1) | 1 |
| `crates/render/Cargo.toml` | `png` dev-dependency (workspace) | 1 |
| `Cargo.lock` | `png` in render's dependency list | 1 |
| `docs/testing.md` | how tests skip without a GPU; golden tolerance, failure images, regenerating | 1 |
| `crates/app/src/ui.rs` | toolbar `horizontal_wrapped`; toolbar click tests at several widths | 2 |
| `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md` | status line, Testing pointer, M8b line | 1, 2 |


The code below was written and verified in a scratch copy of `dev` at bf371de. Each task was checked on its own with `cargo fmt --all --check`, clippy with `-D warnings`, and the full test suite. Workspace test totals after each task: 800, 801 (from 788). The golden PNGs are binary and are not reproduced here: Task 1 regenerates them on the implementer's machine with `UPDATE_GOLDEN=1`.

Code for new files is given in full. Changes to existing files are given as unified diffs against the state the previous task leaves.

---

### Task 1: Golden-image render tests

**Files:**
- Create `crates/render/tests/golden.rs`
- Create (binary, generated) `crates/render/tests/golden/box.png`, `painted_box.png`, `rotated_component.png`, `guides.png`, `soft_cylinder.png`, `parallel_top.png`, `selection.png`, `open_group.png` (160 x 120 RGBA PNGs). The plan cannot embed them: the implementer generates them with `UPDATE_GOLDEN=1 cargo test -p render --test golden` (on a machine with a GPU adapter), then runs `cargo test -p render --test golden` without the variable to confirm they match, and commits them. The reviewer opens each PNG and checks it by eye against the scene description in the Spec notes (box shaded gray with dark edges; painted box brick outside, green inside; two boxes, the right one turned; dashed guide lines and a guide point cross; cylinder with no vertical side edges; flat top view of a rectangle and circle; blue top face and blue vertical edge; two boxes, the loose one with faded edges).
- Modify `crates/render/Cargo.toml` (`[dev-dependencies] png.workspace = true`)
- Modify `Cargo.lock` (one line: `png` under `render`)
- Create `docs/testing.md`
- Modify `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md` (status line, Testing section pointer, M8b milestone line)

**Interfaces produced:**
- `const WIDTH: u32 = 160; const HEIGHT: u32 = 120; const CHANNEL_TOLERANCE: u8 = 24; const MAX_DIFFERING: f64 = 0.005;`
- `enum Comparison { Match { differing: usize }, SizeMismatch, Differ { differing: usize, total: usize } }`
- `fn compare(expected: &[u8], actual: &[u8], width: u32, height: u32) -> (Comparison, Vec<bool>)`
- `fn diff_image(actual: &[u8], mask: &[bool]) -> Vec<u8>`
- `fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32)`, `fn read_png(path: &Path) -> Option<(Vec<u8>, u32, u32)>`
- `fn check(name: &str, doc: &Document)`: render, then compare or (UPDATE_GOLDEN=1) store
- scene helpers: `run`, `doc`, `look`, `created_face`, `add_box(doc, [x, y]) -> top face`, `raise(doc, face, height)`, `top_face(doc, z)`, `add_material`, `all_faces`

- [ ] **Step 1: Write the failing tests**

Tests: `compare_accepts_small_differences_and_rejects_large_ones`, `diff_image_marks_differing_pixels_red`, `png_round_trip_keeps_pixels`, `golden_box`, `golden_painted_box_front_and_back`, `golden_rotated_component`, `golden_guides_are_dashed`, `golden_cylinder_hides_soft_edges`, `golden_parallel_top_view`, `golden_selection_highlight`, `golden_open_group_fades_the_rest`, `every_golden_image_has_a_test` (12; workspace 788 -> 800).

Test-module hunks are part of the Step 3 diffs; apply them first, along with the test files below.

Create `crates/render/tests/golden.rs`:
```rust
//! Golden-image tests: canonical scenes rendered headless at a fixed size and compared with
//! the PNGs in `tests/golden/`, within a tolerance for GPU and driver differences.
//!
//! `UPDATE_GOLDEN=1 cargo test -p render --test golden` rewrites the stored images from this
//! machine's renders (check them by eye before committing). On a mismatch the actual image
//! and a diff image are written under the target's tmp directory and their paths reported.

use std::path::{Path, PathBuf};

use kernel::EntityId;
use render::Snapshotter;
use session::{Command, Document, MaterialId, Projection, Side, StandardView};

const WIDTH: u32 = 160;
const HEIGHT: u32 = 120;

/// A pixel differs when one of its color channels differs by more than this.
const CHANNEL_TOLERANCE: u8 = 24;

/// An image matches when at most this fraction of its pixels differ.
const MAX_DIFFERING: f64 = 0.005;

/// How two images compare.
#[derive(Debug, PartialEq)]
enum Comparison {
    Match { differing: usize },
    SizeMismatch,
    Differ { differing: usize, total: usize },
}

/// Compares two tightly packed RGBA8 images of `width` x `height` on their color channels
/// (alpha is ignored), returning the outcome and a mask of the differing pixels.
fn compare(expected: &[u8], actual: &[u8], width: u32, height: u32) -> (Comparison, Vec<bool>) {
    let total = (width * height) as usize;
    if expected.len() != total * 4 || actual.len() != total * 4 {
        return (Comparison::SizeMismatch, Vec::new());
    }
    let mask: Vec<bool> = expected
        .chunks(4)
        .zip(actual.chunks(4))
        .map(|(e, a)| (0..3).any(|c| e[c].abs_diff(a[c]) > CHANNEL_TOLERANCE))
        .collect();
    let differing = mask.iter().filter(|d| **d).count();
    let outcome = if differing as f64 <= MAX_DIFFERING * total as f64 {
        Comparison::Match { differing }
    } else {
        Comparison::Differ { differing, total }
    };
    (outcome, mask)
}

/// The actual image dimmed to gray, with differing pixels in red.
fn diff_image(actual: &[u8], mask: &[bool]) -> Vec<u8> {
    actual
        .chunks(4)
        .zip(mask)
        .flat_map(|(p, &d)| {
            if d {
                [255, 0, 0, 255]
            } else {
                let gray = ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 6 + 64) as u8;
                [gray, gray, gray, 255]
            }
        })
        .collect()
}

fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("create image folder");
    }
    let file = std::fs::File::create(path).expect("create image");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(rgba).expect("png data");
}

/// Reads a PNG as RGBA8 pixels with its size; None if it cannot be read.
fn read_png(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => buf,
        (png::ColorType::Rgb, png::BitDepth::Eight) => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => return None,
    };
    Some((rgba, info.width, info.height))
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.png"))
}

fn updating() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1")
}

/// Renders `doc` with its camera, selection and open context, and checks the image against
/// `tests/golden/<name>.png` (or rewrites that file with `UPDATE_GOLDEN=1`). Skips without
/// a GPU adapter.
fn check(name: &str, doc: &Document) {
    let Some(mut snap) = Snapshotter::new_headless() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let actual = snap
        .capture(
            doc.scene(),
            doc.context(),
            doc.selection(),
            &doc.camera,
            WIDTH,
            HEIGHT,
        )
        .expect("capture");
    let golden = golden_path(name);
    if updating() {
        write_png(&golden, &actual, WIDTH, HEIGHT);
        eprintln!("wrote {}", golden.display());
        return;
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden");
    let actual_path = out.join(format!("{name}.actual.png"));
    let Some((expected, w, h)) = read_png(&golden) else {
        write_png(&actual_path, &actual, WIDTH, HEIGHT);
        panic!(
            "{name}: no readable golden image at {}; actual written to {}; \
             run with UPDATE_GOLDEN=1 to store it",
            golden.display(),
            actual_path.display()
        );
    };
    if (w, h) != (WIDTH, HEIGHT) {
        write_png(&actual_path, &actual, WIDTH, HEIGHT);
        panic!(
            "{name}: golden image is {w}x{h}, expected {WIDTH}x{HEIGHT}; actual written to {}",
            actual_path.display()
        );
    }
    match compare(&expected, &actual, WIDTH, HEIGHT) {
        (Comparison::Match { .. }, _) => {}
        (outcome, mask) => {
            let diff_path = out.join(format!("{name}.diff.png"));
            write_png(&actual_path, &actual, WIDTH, HEIGHT);
            if !mask.is_empty() {
                write_png(&diff_path, &diff_image(&actual, &mask), WIDTH, HEIGHT);
            }
            panic!(
                "{name}: render differs from {} ({outcome:?}); actual: {}, diff: {}; \
                 if the change is intended, rerun with UPDATE_GOLDEN=1",
                golden.display(),
                actual_path.display(),
                diff_path.display()
            );
        }
    }
}

fn run(doc: &mut Document, command: Command) -> session::Outcome {
    doc.execute(command.clone())
        .unwrap_or_else(|e| panic!("{command:?}: {e}"))
}

fn doc() -> Document {
    Document::new(glam::DVec2::new(WIDTH as f64, HEIGHT as f64))
}

fn look(doc: &mut Document, eye: [f64; 3], target: [f64; 3]) {
    run(
        doc,
        Command::SetCamera {
            eye,
            target,
            fov_y_deg: None,
            up: None,
            projection: None,
        },
    );
}

fn created_face(outcome: &session::Outcome) -> EntityId {
    *outcome
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Face(_)))
        .expect("a face was made")
}

/// A 2 x 1.5 x 1 m box with a corner at the origin; returns the top face.
fn add_box(doc: &mut Document, origin: [f64; 2]) -> EntityId {
    let [x, y] = origin;
    let base = run(
        doc,
        Command::AddRectangle {
            corner: [x, y, 0.0],
            opposite: [x + 2.0, y + 1.5, 0.0],
            normal: None,
        },
    );
    let face = created_face(&base);
    raise(doc, face, 1.0);
    top_face(doc, 1.0)
}

/// Push-pulls a ground face of the open context up by `height`, whichever way it faces.
fn raise(doc: &mut Document, face: EntityId, height: f64) {
    let EntityId::Face(id) = face else {
        panic!("{face} is not a face")
    };
    let up = doc.model().face(id).expect("face").normal.z > 0.0;
    let distance = if up { height } else { -height };
    run(doc, Command::PushPull { face, distance });
}

/// The face of the open context whose vertices all lie at height `z`.
fn top_face(doc: &Document, z: f64) -> EntityId {
    let model = doc.model();
    let (id, _) = model
        .faces()
        .find(|(_, f)| {
            f.outer
                .iter()
                .all(|v| model.position(*v).is_some_and(|p| (p.z - z).abs() < 1e-9))
        })
        .expect("top face");
    EntityId::Face(id)
}

fn add_material(doc: &mut Document, name: &str, color: [u8; 3]) -> MaterialId {
    let outcome = run(
        doc,
        Command::AddMaterial {
            name: Some(name.into()),
            color,
        },
    );
    outcome.changes.materials[0]
}

fn all_faces(doc: &Document) -> Vec<EntityId> {
    doc.model()
        .faces()
        .map(|(id, _)| EntityId::Face(id))
        .collect()
}

#[test]
fn compare_accepts_small_differences_and_rejects_large_ones() {
    let (w, h) = (20, 10);
    let base = vec![100u8; (w * h * 4) as usize];
    assert_eq!(
        compare(&base, &base, w, h).0,
        Comparison::Match { differing: 0 }
    );
    // Every channel off by the tolerance still matches.
    let near: Vec<u8> = base.iter().map(|v| v + CHANNEL_TOLERANCE).collect();
    assert_eq!(
        compare(&base, &near, w, h).0,
        Comparison::Match { differing: 0 }
    );
    // Alpha is ignored.
    let mut alpha = base.clone();
    alpha[3] = 0;
    assert_eq!(
        compare(&base, &alpha, w, h).0,
        Comparison::Match { differing: 0 }
    );
    // One pixel of 200 is 0.5%: allowed; two are not.
    let mut one = base.clone();
    one[0] = 200;
    assert_eq!(
        compare(&base, &one, w, h).0,
        Comparison::Match { differing: 1 }
    );
    let mut two = one.clone();
    two[4 * 57 + 2] = 0;
    let (outcome, mask) = compare(&base, &two, w, h);
    assert_eq!(
        outcome,
        Comparison::Differ {
            differing: 2,
            total: 200
        }
    );
    assert!(mask[0] && mask[57] && mask.iter().filter(|d| **d).count() == 2);
    assert_eq!(compare(&base, &base[4..], w, h).0, Comparison::SizeMismatch);
}

#[test]
fn diff_image_marks_differing_pixels_red() {
    let actual = [10, 20, 30, 255, 200, 200, 200, 255];
    let diff = diff_image(&actual, &[false, true]);
    assert_eq!(&diff[4..], &[255, 0, 0, 255]);
    assert_eq!(diff[0], diff[1]);
    assert_ne!(&diff[..3], &[255, 0, 0]);
}

#[test]
fn png_round_trip_keeps_pixels() {
    let (w, h) = (3, 2);
    let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 11) as u8).collect();
    let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("golden")
        .join(format!("round_trip_{}.png", std::process::id()));
    write_png(&path, &rgba, w, h);
    assert_eq!(read_png(&path), Some((rgba, w, h)));
    let _ = std::fs::remove_file(&path);
    assert_eq!(read_png(&path.with_extension("missing")), None);
}

#[test]
fn golden_box() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    look(&mut d, [5.0, -4.5, 3.5], [1.0, 0.75, 0.4]);
    check("box", &d);
}

#[test]
fn golden_painted_box_front_and_back() {
    let mut d = doc();
    let top = add_box(&mut d, [0.0, 0.0]);
    run(&mut d, Command::Erase { ids: vec![top] });
    let brick = add_material(&mut d, "Brick", [190, 70, 50]);
    let leaf = add_material(&mut d, "Leaf", [60, 170, 80]);
    let faces = all_faces(&d);
    for (material, side) in [(brick, Side::Front), (leaf, Side::Back)] {
        run(
            &mut d,
            Command::Paint {
                ids: faces.clone(),
                material: Some(material),
                side,
            },
        );
    }
    // From above, into the open box: outside walls are brick, the inside leaf green.
    look(&mut d, [4.0, -3.5, 4.5], [1.0, 0.75, 0.3]);
    check("painted_box", &d);
}

#[test]
fn golden_rotated_component() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let faces = all_faces(&d);
    let made = run(
        &mut d,
        Command::MakeComponent {
            ids: faces,
            name: "Crate".into(),
        },
    );
    let first = *made
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(_)))
        .expect("an instance");
    let copied = run(
        &mut d,
        Command::Copy {
            ids: vec![first],
            offset: [3.0, 0.5, 0.0],
            count: None,
        },
    );
    let second = *copied
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(_)))
        .expect("a copy");
    run(
        &mut d,
        Command::Rotate {
            ids: vec![second],
            center: [4.0, 1.25, 0.0],
            axis: None,
            angle: 35.0,
            copies: None,
        },
    );
    run(
        &mut d,
        Command::Select {
            ids: Vec::new(),
            mode: Default::default(),
        },
    );
    look(&mut d, [7.0, -6.0, 5.0], [2.5, 1.0, 0.4]);
    check("rotated_component", &d);
}

#[test]
fn golden_guides_are_dashed() {
    let mut d = doc();
    run(
        &mut d,
        Command::AddRectangle {
            corner: [-1.5, -1.0, 0.0],
            opposite: [1.5, 1.0, 0.0],
            normal: None,
        },
    );
    run(
        &mut d,
        Command::AddGuideLine {
            point: [0.0, 1.0, 0.0],
            direction: [1.0, 0.0, 0.0],
        },
    );
    run(
        &mut d,
        Command::AddGuideLine {
            point: [-1.5, 0.0, 0.0],
            direction: [1.0, 1.0, 0.0],
        },
    );
    run(
        &mut d,
        Command::AddGuidePoint {
            position: [1.5, -1.0, 0.5],
        },
    );
    look(&mut d, [3.0, -4.5, 3.5], [0.0, 0.0, 0.0]);
    check("guides", &d);
}

#[test]
fn golden_cylinder_hides_soft_edges() {
    let mut d = doc();
    let circle = run(
        &mut d,
        Command::AddCircle {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
            normal: None,
            segments: Some(24),
        },
    );
    let face = created_face(&circle);
    raise(&mut d, face, 1.5);
    look(&mut d, [3.5, -4.0, 3.0], [0.0, 0.0, 0.6]);
    check("soft_cylinder", &d);
}

#[test]
fn golden_parallel_top_view() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    run(
        &mut d,
        Command::AddCircle {
            center: [3.5, 0.75, 0.0],
            radius: 0.75,
            normal: None,
            segments: Some(24),
        },
    );
    run(
        &mut d,
        Command::StandardView {
            view: StandardView::Top,
        },
    );
    run(
        &mut d,
        Command::SetProjection {
            projection: Projection::Parallel,
        },
    );
    run(&mut d, Command::ZoomExtents);
    check("parallel_top", &d);
}

#[test]
fn golden_selection_highlight() {
    let mut d = doc();
    let top = add_box(&mut d, [0.0, 0.0]);
    // One vertical edge: the one standing at the box's corner nearest the camera.
    let model = d.model();
    let (edge, _) = model
        .edges()
        .find(|(_, e)| {
            let [a, b] = e.v.map(|v| model.position(v).unwrap());
            (a.x - 2.0).abs() < 1e-9
                && a.y.abs() < 1e-9
                && (b.x - 2.0).abs() < 1e-9
                && b.y.abs() < 1e-9
        })
        .expect("corner edge");
    run(
        &mut d,
        Command::Select {
            ids: vec![top, EntityId::Edge(edge)],
            mode: Default::default(),
        },
    );
    look(&mut d, [5.0, -4.5, 3.5], [1.0, 0.75, 0.4]);
    check("selection", &d);
}

#[test]
fn golden_open_group_fades_the_rest() {
    let mut d = doc();
    add_box(&mut d, [0.0, 0.0]);
    let faces = all_faces(&d);
    let group = run(&mut d, Command::MakeGroup { ids: faces });
    let instance = *group
        .changes
        .created
        .iter()
        .find(|e| matches!(e, EntityId::Instance(_)))
        .expect("an instance");
    add_box(&mut d, [3.0, 0.5]);
    run(&mut d, Command::OpenInstance { id: instance });
    look(&mut d, [6.5, -5.5, 4.5], [2.5, 1.0, 0.4]);
    check("open_group", &d);
}

/// Every stored golden image belongs to a test here, so stale ones do not pile up.
#[test]
fn every_golden_image_has_a_test() {
    if updating() {
        return; // the other tests are writing the images
    }
    let names = [
        "box",
        "painted_box",
        "rotated_component",
        "guides",
        "soft_cylinder",
        "parallel_top",
        "selection",
        "open_group",
    ];
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut stored: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter_map(|n| n.strip_suffix(".png").map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    stored.sort();
    let mut expected: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    expected.sort();
    assert_eq!(stored, expected, "golden images in {}", dir.display());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

before the PNGs exist, `cargo test -p render --test golden` fails the eight scene tests with "no readable golden image at .../tests/golden/<name>.png; actual written to .../target/tmp/golden/<name>.actual.png; run with UPDATE_GOLDEN=1 to store it" and `every_golden_image_has_a_test` fails (3 passed, 9 failed). Regression check: making `scene_geometry` draw soft edges fails `golden_cylinder_hides_soft_edges` with `Differ { differing: 785, total: 19200 }` and a diff image showing the vertical side edges in red (revert afterwards).

- [ ] **Step 3: Implement**

Apply to `Cargo.lock`:
```diff
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -2320,8 +2320,9 @@ version = "0.1.0"
 dependencies = [
  "bytemuck",
  "glam",
  "kernel",
+ "png",
  "pollster 1.0.1",
  "session",
  "wgpu",
 ]
```

Apply to `crates/render/Cargo.toml`:
```diff
--- a/crates/render/Cargo.toml
+++ b/crates/render/Cargo.toml
@@ -9,4 +9,7 @@ kernel = { path = "../kernel" }
 glam.workspace = true
 bytemuck.workspace = true
 wgpu.workspace = true
 pollster.workspace = true
+
+[dev-dependencies]
+png.workspace = true
```

Apply to `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`:
```diff
--- a/docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
+++ b/docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
@@ -1,8 +1,8 @@
 # SketchUp Alternative — Design
 
 Date: 2026-10-10
-Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides, M6d curves, M7a camera, M7b materials, M7c outliner, M8 split into M8a-d, M8a import)
+Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides, M6d curves, M7a camera, M7b materials, M7c outliner, M8 split into M8a-d, M8a import, M8b golden images)
 
 ## Goal
 
 A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.
@@ -134,9 +134,9 @@ Headless inference tests (fixed camera and cursor, assert snap kind and point).
 - 4x MSAA.
 
 ### Testing
 
-Unit tests for buffer building and camera math; a small set of headless golden-image tests.
+Unit tests for buffer building and camera math; a small set of headless golden-image tests (`crates/render/tests/golden.rs`, see `docs/testing.md`).
 
 ## 4. App shell and files (`app`)
 
 - winit event loop; egui for toolbar, VCB, entity info, outliner, materials.
@@ -202,9 +202,9 @@ Every milestone's acceptance includes: new capabilities are reachable as `sessio
 - **M7b**: materials: named sRGB colors (`m<n>` ids from the entity id counter) belonging to the whole model; each face side and each instance carries an optional material (none is the default material); an instance's material shows on face sides inside it without one (the nearest painted instance wins), and exploding bakes it onto them. Commands `add_material` (optional name, default "Material", "Material 2", ...; names distinct, not blank, at most 256 characters), `change_material` (name and/or color), `delete_material` (unpaints everything painted with it, one undo step) and `paint` (faces and instances of the open context, `side` front, back or both, null for the default); a `materials` query (with face and instance use counts); `front_material`/`back_material` on faces and `material` on instances in queries; `changes.materials` lists materials a command added, removed or changed. Paint follows geometry: split pieces keep it, merges keep the containing face's, copies/groups/components keep it, push/pull gives the cap and sides the pushed face's materials (swapped for a sheet pushed against its normal, so the side facing the pull ends up outside), a face turned over keeps each material on its side. `.skr` schema version 5 (versions 1-4 open unpainted); files painting with missing materials or with bad material names are refused. Rendering shows each side's color (selection tint on top); export writes OBJ with a `.mtl` beside it, glTF with one material per color (front side only; STL has no colors). Paint tool (`b`, last toolbar button: click a face to paint the side facing the viewer with the current material, a click on something selected paints the whole selection's faces and instances, an instance paints the instance; Cmd-click or Ctrl-click (either key on every system; Alt is the orbit modifier) picks up the clicked side's material) and a Materials list in the right panel under Entity Info (Default plus every material with a swatch; clicking one makes it current and switches to Paint; Add, Delete, a name field renaming on Enter, and a 12-color palette to recolor); Entity Info shows a face's front and back materials and a painted instance's. Opacity, textures and a free color picker are deferred.
 - **M7c**: outliner: an `outliner` query (the tree of placed groups and components, the root's instances down, each level in id order: `id`, `definition`, `kind`, `name` (the definition's; empty for an unnamed group), `material`, `children`; `open` marks the nodes on the open context's path and `selected` the selected instances of the open context; a component's contents appear under each of its instances with the same ids, so a node is named by its path from the root) and an `open_path` command (makes a path of instances from the root the open context from wherever it is, `[]` being the root; clears the selection unless the path is already open; not an undo step; non-instances `invalid_input`, misplaced instances `not_found`). In the window, an Outliner section in the right panel under Materials: a Model row (click: back to the root) and the tree with expand and collapse (nodes on the open path start expanded), groups shown by name or as "Group", components as "<Name>", selected rows highlighted and open ones bold. A click selects the node, first opening the context it is in (as SketchUp does); Shift-click or Cmd/Ctrl-click adds it to or removes it from the selection when it is in the open context; a double-click opens it for editing. Instance names, a filter box, sorting by name and drag-to-reparent are deferred.
 
 - **M8a**: import: an `import` command (path; `format` obj or stl, from the extension; `as` group (default), component or context; `scale` meters per file unit, default 1; `up` y or z, default y for OBJ and z for STL, so export then import gives back the same geometry; `merge_coplanar`, default true) adding the file's faces to the open context as one undo step. A group or component is named after the file, placed at the context's origin and selected; `context` makes the group and explodes it, so the faces merge with what they touch (at most 2,000 faces). Faces are built straight from the polygons, not drawn: corners within 1e-6 m weld, a polygon that is bent or crosses itself becomes triangles, degenerate and repeated polygons are skipped and counted, neighbouring faces in one plane with one material merge (holes kept), and nothing is split where faces cross. OBJ: `v`, `f` (negative indices), `usemtl`, `mtllib` (a name the model has is its material, a library `Kd` color makes a new one, `default` is the default material); normals, texture coordinates, lines and groups are ignored. STL: binary (by its size) or ASCII. The outcome's `import` reports the instance, definition and polygon, face, vertex, skipped and new material counts. Files over 128 MiB, 200,000 polygons or 1,000,000 positions, broken lines, non-finite numbers, positions beyond +/-1e6 m and files making no face are `file` errors that change nothing. File > Import... (Cmd+I) picks an OBJ or STL and imports it as a group; the status reports faces and skipped polygons or the error. OBJ groups as separate groups, texture coordinates and a units choice in the window are deferred.
-- **M8b**: golden-image render tests (headless renders compared with stored images) and toolbar wrapping.
+- **M8b**: golden-image render tests and toolbar wrapping. Eight canonical scenes (a box, a painted open box showing front and back colors, a rotated component copy, dashed guides, a cylinder with soft sides hidden, a parallel top view, a selected face and edge, an open group fading the rest), built with commands, are rendered headless at 160 x 120 and compared with PNGs in `crates/render/tests/golden/`: a pixel differs when a color channel differs by more than 24, and at most 0.5% of pixels may differ. On a mismatch the actual and a diff image are written under the target's tmp directory; `UPDATE_GOLDEN=1` rewrites the stored images; without a GPU adapter the tests skip.
 - **M8c**: clean-up of deferred correctness and performance items.
 - **M8d**: clean-up of deferred UX and API polish items.
 
 ## Risks
```

Create `docs/testing.md`:
~~~~markdown
# Testing

`cargo test --workspace` runs everything. Rendering tests need a GPU adapter; without one they
print "no GPU adapter, skipping" and pass.

## Golden images

`crates/render/tests/golden.rs` renders a few canonical scenes headless at 160 x 120 and compares
each with its PNG in `crates/render/tests/golden/`. A pixel differs when one of its color channels
differs by more than 24 (out of 255); a test fails when more than 0.5% of its pixels differ, which
leaves room for GPU and driver differences in antialiasing.

On a failure the test prints two paths under `target/tmp/golden/`: `<name>.actual.png` (this
render) and `<name>.diff.png` (the render in gray with differing pixels in red).

When a rendering change is intended, regenerate the stored images and look at every changed one
before committing it:

```
UPDATE_GOLDEN=1 cargo test -p render --test golden
git diff --stat crates/render/tests/golden/
```

A new scene needs a test calling `check("<name>", &doc)` and its name in
`every_golden_image_has_a_test`; run with `UPDATE_GOLDEN=1` once to store its image.
~~~~

- [ ] **Step 4: Run the tests to verify they pass**

Generate the golden images (binary, not shown in this plan): `UPDATE_GOLDEN=1 cargo test -p render --test golden`, then run `cargo test -p render --test golden` again without the variable; it must pass. Open each PNG and check it shows the scene its test describes. Images: `crates/render/tests/golden/box.png`, `crates/render/tests/golden/guides.png`, `crates/render/tests/golden/open_group.png`, `crates/render/tests/golden/painted_box.png`, `crates/render/tests/golden/parallel_top.png`, `crates/render/tests/golden/rotated_component.png`, `crates/render/tests/golden/selection.png`, `crates/render/tests/golden/soft_cylinder.png`.

after `UPDATE_GOLDEN=1 cargo test -p render --test golden`, `cargo test -p render --test golden` passes 12 of 12, repeatedly; workspace 800 passed, fmt and clippy clean.

Expected workspace totals after each task: 800, 801.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/render/tests/golden.rs Cargo.lock crates/render/Cargo.toml docs/superpowers/specs/2026-10-10-sketchup-alt-design.md docs/testing.md crates/render/tests/golden/ Cargo.lock
git commit -m "Golden-image render tests: eight canonical scenes rendered headless at 160x120 and compared with stored PNGs within a channel and pixel-fraction tolerance; actual and diff images on failure, UPDATE_GOLDEN=1 regenerates; docs"
```

### Task 2: Toolbar wrapping

**Files:**
- Modify `crates/app/src/ui.rs` (toolbar `horizontal` -> `horizontal_wrapped` with a comment; tests: `screen_of`, `toolbar_clicks`, `assert_every_toolbar_button`, rewritten `clicking_a_tool_button_reports_it`, new `a_narrow_window_wraps_the_toolbar_and_keeps_every_button`; `screen()` doc comment)
- Modify `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md` (M8b line: toolbar sentence)

**Interfaces produced:**
- `fn screen_of(width: f32) -> egui::RawInput`
- `fn toolbar_clicks(width: f32) -> (Vec<UiAction>, f32)`: clicks the whole toolbar area every 4 points across and 6 down; returns the actions and the toolbar height (central rect's top)
- `fn assert_every_toolbar_button(clicked: &[UiAction], width: f32)`: every `ToolKind::ALL` tool, Undo, Redo, MakeGroup, MakeComponent, Explode, CloseInstance

- [ ] **Step 1: Write the failing tests**

Tests: `clicking_a_tool_button_reports_it` (1280: every button; toolbar height equal to that at 2400, i.e. one row), `a_narrow_window_wraps_the_toolbar_and_keeps_every_button` (900 and 640: every button; height more than one row + 10 and under 200) (1 new; workspace 800 -> 801).

Test-module hunks are part of the Step 3 diffs; apply them first, along with the test files below.

- [ ] **Step 2: Run the tests to verify they fail**

with the test changes alone, `a_narrow_window_wraps_the_toolbar_and_keeps_every_button` fails: "MakeComponent at 900 points: [...]" (the buttons past 900 points are clipped).

- [ ] **Step 3: Implement**

Apply to `crates/app/src/ui.rs`:
```diff
--- a/crates/app/src/ui.rs
+++ b/crates/app/src/ui.rs
@@ -99,9 +99,10 @@ const BELOW_MATERIALS_LIST: f32 = 140.0 + OUTLINER_MIN_HEIGHT;
 /// the 3D view, in points.
 pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
     let mut actions = Vec::new();
     Panel::top("toolbar").show(ui, |ui| {
-        ui.horizontal(|ui| {
+        // Wraps onto more rows when the window is narrower than the buttons (about 1240 points).
+        ui.horizontal_wrapped(|ui| {
             ui.menu_button("File", |ui| {
                 let items = [
                     ("New", "Cmd+N", UiAction::NewModel),
                     ("Open...", "Cmd+O", UiAction::OpenFile),
@@ -480,9 +481,9 @@ mod tests {
             .drop_without_applying_deltas();
         out.unwrap()
     }
 
-    /// The window's default size (see `app.rs`); the toolbar needs about 1200 points.
+    /// The window's default size (see `app.rs`); the toolbar fits one row of it.
     fn screen() -> egui::RawInput {
         egui::RawInput {
             screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
             ..Default::default()
@@ -707,10 +708,19 @@ mod tests {
         assert!(rect.max.y < 800.0, "status bar below: {rect:?}");
         assert!(rect.width() > 600.0 && rect.height() > 500.0, "{rect:?}");
     }
 
-    #[test]
-    fn clicking_a_tool_button_reports_it() {
+    /// A `width` x 800 point window.
+    fn screen_of(width: f32) -> egui::RawInput {
+        egui::RawInput {
+            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 800.0))),
+            ..Default::default()
+        }
+    }
+
+    /// Clicks over the whole toolbar of a `width`-point window, every 4 points across and 6
+    /// down. Returns the actions and the toolbar's height (the 3D view's top).
+    fn toolbar_clicks(width: f32) -> (Vec<UiAction>, f32) {
         let ctx = egui::Context::default();
         let status = status();
         let info = Vec::new();
         let view = View {
@@ -727,39 +737,73 @@ mod tests {
             outliner: &NO_ROWS,
             context: &[],
             selection: &NO_SELECTION,
         };
-        let (_, central) = run(&ctx, screen(), &view);
-        let row = central.min.y / 2.0;
-        // Click along the toolbar row until a press lands on the Rectangle button.
+        let (_, central) = run(&ctx, screen_of(width), &view);
         let mut clicked = Vec::new();
-        for x in (2..1280).step_by(4) {
-            let pos = Pos2::new(x as f32, row);
-            let pointer = |pressed: bool| egui::Event::PointerButton {
-                pos,
-                button: egui::PointerButton::Primary,
-                pressed,
-                modifiers: Default::default(),
-            };
-            let mut press = screen();
-            press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
-            let _ = run(&ctx, press, &view);
-            let mut release = screen();
-            release.events = vec![pointer(false)];
-            clicked.extend(run(&ctx, release, &view).0);
+        for y in (3..central.min.y as i32).step_by(6) {
+            for x in (2..width as i32).step_by(4) {
+                let pos = Pos2::new(x as f32, y as f32);
+                let pointer = |pressed: bool| egui::Event::PointerButton {
+                    pos,
+                    button: egui::PointerButton::Primary,
+                    pressed,
+                    modifiers: Default::default(),
+                };
+                let mut press = screen_of(width);
+                press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
+                let _ = run(&ctx, press, &view);
+                let mut release = screen_of(width);
+                release.events = vec![pointer(false)];
+                clicked.extend(run(&ctx, release, &view).0);
+            }
         }
-        // Paint is the last tool: the toolbar still fits a 1280-point window.
-        for kind in [ToolKind::Rectangle, ToolKind::Arc, ToolKind::Paint] {
-            assert!(clicked.contains(&UiAction::Tool(kind)), "{clicked:?}");
+        (clicked, central.min.y)
+    }
+
+    /// Every toolbar button (not the menus) is in `clicked`.
+    fn assert_every_toolbar_button(clicked: &[UiAction], width: f32) {
+        for kind in ToolKind::ALL {
+            assert!(
+                clicked.contains(&UiAction::Tool(kind)),
+                "{kind:?} at {width} points: {clicked:?}"
+            );
         }
-        assert!(clicked.contains(&UiAction::Undo));
         for action in [
+            UiAction::Undo,
+            UiAction::Redo,
             UiAction::MakeGroup,
             UiAction::MakeComponent,
             UiAction::Explode,
             UiAction::CloseInstance,
         ] {
-            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
+            assert!(
+                clicked.contains(&action),
+                "{action:?} at {width} points: {clicked:?}"
+            );
+        }
+    }
+
+    #[test]
+    fn clicking_a_tool_button_reports_it() {
+        let (clicked, height) = toolbar_clicks(1280.0);
+        assert_every_toolbar_button(&clicked, 1280.0);
+        // The default window keeps the toolbar on one row, as a very wide one does.
+        let (_, wide) = toolbar_clicks(2400.0);
+        assert_eq!(height, wide, "one toolbar row at 1280 points");
+    }
+
+    #[test]
+    fn a_narrow_window_wraps_the_toolbar_and_keeps_every_button() {
+        let (_, one_row) = toolbar_clicks(2400.0);
+        for width in [900.0, 640.0] {
+            let (clicked, height) = toolbar_clicks(width);
+            assert_every_toolbar_button(&clicked, width);
+            assert!(
+                height > one_row + 10.0,
+                "the toolbar wraps at {width} points: {height} vs {one_row}"
+            );
+            assert!(height < 200.0, "{height} points of toolbar at {width}");
         }
     }
 
     /// Clicks down the right panel at `x`, every 4 points; after each click, types `typed`
```

Apply to `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`:
```diff
--- a/docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
+++ b/docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
@@ -202,9 +202,9 @@ Every milestone's acceptance includes: new capabilities are reachable as `sessio
 - **M7b**: materials: named sRGB colors (`m<n>` ids from the entity id counter) belonging to the whole model; each face side and each instance carries an optional material (none is the default material); an instance's material shows on face sides inside it without one (the nearest painted instance wins), and exploding bakes it onto them. Commands `add_material` (optional name, default "Material", "Material 2", ...; names distinct, not blank, at most 256 characters), `change_material` (name and/or color), `delete_material` (unpaints everything painted with it, one undo step) and `paint` (faces and instances of the open context, `side` front, back or both, null for the default); a `materials` query (with face and instance use counts); `front_material`/`back_material` on faces and `material` on instances in queries; `changes.materials` lists materials a command added, removed or changed. Paint follows geometry: split pieces keep it, merges keep the containing face's, copies/groups/components keep it, push/pull gives the cap and sides the pushed face's materials (swapped for a sheet pushed against its normal, so the side facing the pull ends up outside), a face turned over keeps each material on its side. `.skr` schema version 5 (versions 1-4 open unpainted); files painting with missing materials or with bad material names are refused. Rendering shows each side's color (selection tint on top); export writes OBJ with a `.mtl` beside it, glTF with one material per color (front side only; STL has no colors). Paint tool (`b`, last toolbar button: click a face to paint the side facing the viewer with the current material, a click on something selected paints the whole selection's faces and instances, an instance paints the instance; Cmd-click or Ctrl-click (either key on every system; Alt is the orbit modifier) picks up the clicked side's material) and a Materials list in the right panel under Entity Info (Default plus every material with a swatch; clicking one makes it current and switches to Paint; Add, Delete, a name field renaming on Enter, and a 12-color palette to recolor); Entity Info shows a face's front and back materials and a painted instance's. Opacity, textures and a free color picker are deferred.
 - **M7c**: outliner: an `outliner` query (the tree of placed groups and components, the root's instances down, each level in id order: `id`, `definition`, `kind`, `name` (the definition's; empty for an unnamed group), `material`, `children`; `open` marks the nodes on the open context's path and `selected` the selected instances of the open context; a component's contents appear under each of its instances with the same ids, so a node is named by its path from the root) and an `open_path` command (makes a path of instances from the root the open context from wherever it is, `[]` being the root; clears the selection unless the path is already open; not an undo step; non-instances `invalid_input`, misplaced instances `not_found`). In the window, an Outliner section in the right panel under Materials: a Model row (click: back to the root) and the tree with expand and collapse (nodes on the open path start expanded), groups shown by name or as "Group", components as "<Name>", selected rows highlighted and open ones bold. A click selects the node, first opening the context it is in (as SketchUp does); Shift-click or Cmd/Ctrl-click adds it to or removes it from the selection when it is in the open context; a double-click opens it for editing. Instance names, a filter box, sorting by name and drag-to-reparent are deferred.
 
 - **M8a**: import: an `import` command (path; `format` obj or stl, from the extension; `as` group (default), component or context; `scale` meters per file unit, default 1; `up` y or z, default y for OBJ and z for STL, so export then import gives back the same geometry; `merge_coplanar`, default true) adding the file's faces to the open context as one undo step. A group or component is named after the file, placed at the context's origin and selected; `context` makes the group and explodes it, so the faces merge with what they touch (at most 2,000 faces). Faces are built straight from the polygons, not drawn: corners within 1e-6 m weld, a polygon that is bent or crosses itself becomes triangles, degenerate and repeated polygons are skipped and counted, neighbouring faces in one plane with one material merge (holes kept), and nothing is split where faces cross. OBJ: `v`, `f` (negative indices), `usemtl`, `mtllib` (a name the model has is its material, a library `Kd` color makes a new one, `default` is the default material); normals, texture coordinates, lines and groups are ignored. STL: binary (by its size) or ASCII. The outcome's `import` reports the instance, definition and polygon, face, vertex, skipped and new material counts. Files over 128 MiB, 200,000 polygons or 1,000,000 positions, broken lines, non-finite numbers, positions beyond +/-1e6 m and files making no face are `file` errors that change nothing. File > Import... (Cmd+I) picks an OBJ or STL and imports it as a group; the status reports faces and skipped polygons or the error. OBJ groups as separate groups, texture coordinates and a units choice in the window are deferred.
-- **M8b**: golden-image render tests and toolbar wrapping. Eight canonical scenes (a box, a painted open box showing front and back colors, a rotated component copy, dashed guides, a cylinder with soft sides hidden, a parallel top view, a selected face and edge, an open group fading the rest), built with commands, are rendered headless at 160 x 120 and compared with PNGs in `crates/render/tests/golden/`: a pixel differs when a color channel differs by more than 24, and at most 0.5% of pixels may differ. On a mismatch the actual and a diff image are written under the target's tmp directory; `UPDATE_GOLDEN=1` rewrites the stored images; without a GPU adapter the tests skip.
+- **M8b**: golden-image render tests and toolbar wrapping. Eight canonical scenes (a box, a painted open box showing front and back colors, a rotated component copy, dashed guides, a cylinder with soft sides hidden, a parallel top view, a selected face and edge, an open group fading the rest), built with commands, are rendered headless at 160 x 120 and compared with PNGs in `crates/render/tests/golden/`: a pixel differs when a color channel differs by more than 24, and at most 0.5% of pixels may differ. On a mismatch the actual and a diff image are written under the target's tmp directory; `UPDATE_GOLDEN=1` rewrites the stored images; without a GPU adapter the tests skip. The toolbar wraps onto more rows when the window is narrower than its buttons (one row at the default 1,280 points), so every button stays reachable.
 - **M8c**: clean-up of deferred correctness and performance items.
 - **M8d**: clean-up of deferred UX and API polish items.
 
 ## Risks
```

- [ ] **Step 4: Run the tests to verify they pass**

with `horizontal_wrapped`, `cargo test -p app --bin app ui::` passes 16 of 16; workspace 801 passed, fmt and clippy clean. The ui tests take about 8 s in debug (the two toolbar tests click ~10,000 points).

Expected workspace totals after each task: 800, 801.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/app/src/ui.rs docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
git commit -m "Toolbar wraps onto more rows in windows narrower than its buttons; ui tests click every toolbar button at 1280, 900 and 640 points; spec"
```

## Self-review notes

- **Spec coverage:** golden-image tests (Task 1), toolbar wrapping (Task 2).
- **Deferred** (from the scratch build):

- Golden images are generated on one machine (Apple GPU, Metal). Other adapters (llvmpipe/lavapipe in CI, other vendors) may exceed the 0.5% budget on antialiased edges; untested. If CI with a software adapter is added, it may need its own tolerance or its own image set.
- No golden image at a high DPI scale or for the overlay pass (tool previews, inference markers, snap points); `render_view` passes no overlay.
- Color shifts under 24 levels per channel (for example a slightly different default face color) pass the golden tests; the existing headless tests still check specific colors.
- No minimum window size; at very narrow widths (under ~400 points) the right panel and wrapped toolbar crowd the 3D view.
- The toolbar click tests are slow-ish (~8 s debug for ui tests); recording button rects in the test (as the outliner tests do) would be faster.
- `docs/testing.md` is new; there is still no top-level README.
