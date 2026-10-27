# M11c Viewport Feedback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The 3D view explains itself. Snap markers get a shape as well as a color, with a halo. Preview lines are 2 pt, or 2.5 pt when locked, in the shared axis colors. A chip by the cursor names the snap and shows the live value or the typed text, and an invalid typed value flashes there. What a click would act on is highlighted for Select, Push/Pull, Paint and Erase. Push/Pull previews a translucent solid. Each tool has its own cursor. A 72 pt orientation gizmo sets the standard views and the projection. The `editor` query and `tool_input` results report the hovered entity and the snap label.

**Architecture:**
- `tools/src/overlay.rs` becomes pure screen-space math: the marker for each snap kind, the snap label, and the preview lines clipped and projected to view pixels with their colors and widths. The tools gain `Tool::hovered`, and Push/Pull reports a `Solid`.
- The renderer gains three window-only inputs, none of which `render_view` or the goldens ever set: `set_hover` (a hover flag buffer beside the selection flags, select-guarded in WGSL), `set_preview` (a blended solid pipeline) and two `ViewStyle` fields for the axes (`axes`, `wide_axes`). `ViewStyle::CLASSIC` keeps today's axes.
- The app draws the markers, preview lines, chip, flash and gizmo with egui, over the 3D view, from a `feedback::Hud` the editor builds each frame. `feedback::Timing` holds the 150 ms rest and the 2.5 s flash as repaint deadlines. The window passes an empty overlay to the renderer.
- `api::EditorState` gains `hovered` and `snap_label`.

**Tech Stack:** Rust 2024; egui, egui-winit, egui-wgpu, egui_kittest `=0.36.2`; wgpu `=30.0.1`; egui-phosphor `=0.14.0`; glam. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md`, "Interface milestones", M11c. The previous plans, `docs/superpowers/plans/2026-10-17-m11b-layout.md` and `2026-10-16-m11a-design-foundation.md`, define what this one consumes: theme tokens and `theme::AXIS_*`, `ViewStyle`, the kittest harness, `ui::draw` and `View`. The design audits (`interaction.md` P1-P4 and P10, `visual.md` P4 and P7, in `/private/tmp/claude-501/-Users-mohn93-Desktop-side-projects-sketchup/7b020704-4344-4767-b2d2-50b34442d2ad/scratchpad/design/`) only explain where values came from; the executor reads the spec bullet and this plan.

## Global Constraints

- No emoji anywhere (code, comments, docs, commit messages). Keep comments succinct and in the style of the surrounding code.
- Before any cargo command, run `export PATH="$HOME/.cargo/bin:$PATH"`. After every task, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` must pass. The suite has 1253 tests at the start (`cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'`). No task may leave fewer tests. A test replaced by its migrated twin keeps the count; a deleted test needs a replacement in the same task.
- Pinned versions: egui, egui-winit, egui-wgpu and egui_kittest `=0.36.2`; wgpu `=30.0.1`; egui-phosphor `=0.14.0`. Add no dependency.
- **CLASSIC renders stay byte-identical.** `render_view`, headless renders and the golden images use `ViewStyle::CLASSIC` and never call `set_hover` or `set_preview`. Every new per-fragment or per-vertex effect in `scene.wgsl` is guarded by `select(...)` on a value that is 0 under CLASSIC, or drawn by a new pipeline that CLASSIC never binds, so the Metal code for the CLASSIC path computes exactly what it did. Task 13 proves it with two worktrees.
- Values from the spec:
  - Markers are 12 pt with a 1.5 pt halo, white by day and black by night: endpoint a green filled circle, midpoint a cyan filled circle, intersection an orange X, on edge a red hollow square, on face a blue diamond, a guide point a cross.
  - Preview lines are 2 pt, and 2.5 pt when locked. The axis colors are shared by the axes, the preview lines and the locks: X `#E5484D`, Y `#30A46C`, Z `#3E63DD`. The axes are 1.5 pt, faded on their negative side.
  - After the cursor rests for 150 ms, a chip near it names the snap ("Endpoint", "On Red Axis", "Parallel to Edge", "Midpoint in Group"). A line under the label shows the measurement or, while typing, the typed text. An invalid typed value flashes its message at the cursor for 2.5 s and also stays in the status bar.
  - Hover highlight for Select, Push/Pull, Paint and Erase: faces mix 25% toward the accent and edges are drawn 2 pt in the accent. Erase uses the danger color, and Paint previews the material.
  - Push/Pull previews a solid in the face's material at 70% opacity.
  - Cursors: a crosshair for the drawing tools and Tape, move arrows for Move, grabbing while orbiting.
  - Select draws no snap markers.
  - A 72 pt gizmo in the view's top right: clicking an axis end sets that standard view; clicking the centre switches between perspective and parallel.
- Idle stays at 0% CPU: no unconditional `request_repaint` or `request_redraw`. The rest and the flash wake the window through `repaint_at` deadlines only.
- Icons draw only in the "phosphor" family (`fonts::icons()`); labels use Inter. This milestone adds no icon glyphs (the gizmo letters are Inter).
- egui 0.36.2: a selected button reports AccessKit `toggled` (use `ui::tests::is_on`); a tooltip repeats its widget's name, so query before hovering; `Window::anchor` makes a window immovable; `ui::draw` appends actions on every pass, so anything applied from an action must be harmless twice (`StandardView` and `SetProjection` are).
- Agent API parity: hover and the snap label are in `editor` and in `tool_input` results; the gizmo maps to `standard_view` and `set_projection`. `docs/agent-api.md` is updated in Task 12.
- Git: work on `dev`. Other agents may share the checkout, so run `git status` and `git diff` first and stage only the files you changed (never `git add -A` or `git add .`). Short plain commit messages, no attribution lines. Never push or merge.
- Clippy denies dead code in the `app` binary: an item lands in the task that first uses it. Public items of the library crates (`tools`, `render`, `api`) may land before their first user.
- Do not launch the app window in tests (no `cargo run` of the GUI). GPU tests in `crates/render` run on this Mac. Tests never write under `~/.tenon`.

## Review Focus

- **A window left alone must go idle.** After the cursor stops, exactly one frame is drawn at the 150 ms rest; after an invalid value, one more at 2.5 s; then none. Tests: `timing_asks_for_one_frame_per_change_then_none` (Task 9), `the_hud_settles_in_one_frame` (Task 9).
- **A preview line that runs behind the eye** (a long Line chain, a Tape guide, an orbit that puts the start point behind the camera; in parallel, a point outside the depth range) must be clipped, not flipped across the screen or dropped whole. Task 1 test: `lines_crossing_the_near_plane_are_clipped_not_flipped`.
- **The hovered entity disappears or moves out of reach**: undo, an agent's edit, a group opened or closed. No stale highlight and no write past a buffer. Tests: `external_changes_clear_the_hover` (Task 2), `a_new_model_drops_the_hover` (Task 4), `an_agent_edit_drops_the_highlight_until_the_next_frame` (Task 7).
- **A Retina display.** The editor works in physical pixels, egui in points: the marker, lines and chip must land on the cursor's pixel at a scale of 2, and the marker must be 12 pt, not 12 px. Task 9 test: `the_hud_lands_on_its_pixels_at_2x`.
- **The gizmo seen head-on, and presses on it.** In Top view the +Z end sits on the centre: the centre must stay clickable, and no press on the gizmo may reach the tools. Task 11 tests: `an_axis_pointing_at_the_viewer_leaves_the_centre_clickable`, `the_gizmo_takes_the_pointer_from_the_tools`.

## Spec notes (rulings made in this plan)

- **Units.** The spec's "px" for markers, halos and lines means points (egui strokes are in points, and the audits measured logical pixels). The editor's positions are physical view pixels; the UI divides by `pixels_per_point` and adds the view's top-left.
- **Who draws what.**
  - Markers, preview lines, the chip, the flash and the gizmo are drawn by egui, after the panels, with the root `Ui`'s painter clipped to the view rectangle (the gizmo in its own `Area`). The renderer's overlay pass already draws with `CompareFunction::Always`, so drawing on top in egui keeps the depth behaviour exactly, and adds anti-aliased fills, halos and exact widths. The window passes `&[]` as the overlay. `Renderer::render`/`render_in` keep their `overlay` parameter, so the renderer's CLASSIC path and the golden tests do not change.
  - Hover highlight, the Push/Pull solid and the styled axes are drawn by the renderer, because they need the depth buffer (a highlighted face behind another stays hidden; the solid is hidden by geometry in front of it).
  - **None of them appear in `render_view`.** They are window feedback: the offscreen renderer behind `render_view` never calls `set_hover` or `set_preview` and keeps `ViewStyle::CLASSIC`. Agents read them from `editor.hovered`, `editor.snap_label`, `editor.measurement`, and `tool_input`'s `inference` and `preview`.
- **Markers** (`tools::overlay::marker`), 12 pt across, each drawn first in the halo color 1.5 pt larger on every side (strokes 3 pt wider), then in its color:
  - Endpoint, Origin: filled circle `#2FBF4A`.
  - Midpoint: filled circle `#12B5CB`.
  - Intersection: an X of two 2.5 pt strokes, `#FF8A00`.
  - OnEdge, OnGuide: hollow square, 2 pt stroke, `#E5484D`.
  - OnFace: filled diamond `#3E63DD`.
  - GuidePoint: an upright cross (+) of two 2.5 pt strokes, `#2FBF4A`.
  - Axis, Parallel, Perpendicular, Ground, Free: no marker, as today.
  - The halo follows the lighting: white by day, black by night.
  - No marker in Select, while flying, or while the cursor is outside the view.
- **Preview lines** (`tools::overlay::preview_lines`): 2 pt; 2.5 pt while any lock is in force (axis, parallel or perpendicular). Color: the axis color when the hit is on an axis (`#E5484D`, `#30A46C`, `#3E63DD`), magenta `#CC1ACC` for parallel and perpendicular, otherwise `#1A1A1A` by day and `#E8E9EC` by night. One color for all of a frame's segments, as today. Segments are clipped to the camera's visible depths before projecting.
- **The grid's axes** (renderer, themed only): `ViewStyle::axes` holds the three colors and `ViewStyle::wide_axes` draws them about 1.5 pt wide (each axis drawn twice more, offset half a pixel across and half a pixel up; under 4x MSAA that reads 1.5 px). Negative halves mix 60% toward the style's day background, as CLASSIC mixes toward its own. `theme::view_style` sets the shared colors and `wide_axes: true`. CLASSIC keeps `grid::AXIS_*` and `wide_axes: false`, and `grid::axis_lines` returns exactly what it does now.
- **Snap label** (`tools::overlay::snap_label`): "Endpoint", "Midpoint", "Origin", "Guide Point", "Intersection", "On Edge", "On Guide", "On Red Axis", "On Green Axis", "On Blue Axis", "Parallel to Edge", "Perpendicular to Edge", "On Face"; none for Ground and Free.
  - A snap on geometry inside a group or component of the open context adds " in Group" or " in Component", by the definition kind of the instance that places it (`Owner::Instance`). Geometry outside the open context (`Owner::Outside`) adds nothing.
  - The owner is that of the first entity `infer_detail` reports. Snaps with none (origin, axes, parallel, perpendicular, ground) add nothing.
  - No label in Select, as there is no marker there.
- **The chip.**
  - It shows once the cursor has rested 150 ms in the view. Its first line is the snap label (if any) in 11 pt Inter `text_primary`. Its second line is `Status::value` (the live measurement, or the typed text with its trailing `_`) in 13 pt JetBrains Mono: `text_primary`, or `accent` while typing.
  - It is drawn only when it has a line. It is hidden while navigating, while flying, and while the cursor is outside the view or over a window.
  - Its top-left sits at cursor + (14, 18) pt. It flips to the cursor's left or above it so it stays inside the view.
  - It has a `bg_raised` fill, a 1 pt `border` stroke, radius 4, and padding 6 x 4.
- **The flash.** A typed value (Enter) that leaves a message, whether the tool refused it or its command failed, flashes that message for 2.5 s. It is a chip in `danger` fill with `on_accent` 11 pt text, under the chip (or at the chip's place when there is none). It shows whatever the rest. The message also stays in the status bar as today.
- **Timing and idle** (`feedback::Timing`). A cursor move in the view restarts the rest. `Timing::deadline` gives the rest's end until it passes, and the flash's end until it passes, else None. The app folds it into `repaint_at` after each frame, so a still window draws at most one frame per change and then sleeps.
- **Hover target** (`Tool::hovered`, an id of the open context; None while an operation is in progress or the cursor leaves the view):
  - Select: `select::pick` (an edge, guide, group or face; a curve's edge highlights alone).
  - Push/Pull: the open context's face under the cursor while idle.
  - Paint: the face, or the group, a click would paint.
  - Erase: `select::pick`.
  - The other tools: None.
  - Push/Pull's idle outline and Paint's and Erase's outlines are removed; the highlight replaces them. Move keeps its outlines.
- **Hover look** (`theme::hover_look`):
  - Select and Push/Pull: faces mix 25% toward `accent`, edges `accent`.
  - Erase: the same with `danger`.
  - Paint on a face: the face shows the paint material's color fully (mix 1.0; a textured material shows its color; the default material shows `render::scene::DEFAULT_FRONT`), with edges in `accent`.
  - Paint on a group: as Select.
  - A hovered group lights everything it places. A hovered guide is drawn in the edge color.
  - Hovered edges are always drawn wide (2 pt), whatever the style's `wide_selected_edges`. On a selected entity the hover wins.
- **Hover in the renderer.** Flagged meshes get a second flag buffer, `hover` (one f32 per vertex, 0 or 1), read at a new vertex location. `set_hover` rewrites only the old and new hovered ranges. The WGSL mixes and recolors with `select(old, hovered_value, hovered > 0.5)`, so a 0 flag returns the old value untouched. `LightingUniforms` gains `hover_face` (rgb, mix) and `hover_edge`. `set_model` drops the hover (the new buffers start at 0).
- **Push/Pull solid** (`tools::Solid`):
  - It holds the cap (the face's triangles moved by the distance) and the sides (two triangles per loop edge). The base is left out: it would fight the face's own depth.
  - It is shown while the distance is at least `LENGTH_TOL`. Its color is the face's front material (or `DEFAULT_FRONT`) at alpha 0.7.
  - The renderer draws it after the see-through faces, depth-tested without writing depth, alpha blended, both sides, shaded by the ambient light and the headlight.
  - The wireframe preview lines stay on top.
  - Window only: `tool_input` keeps reporting the segments.
- **Cursors** (`ui::viewport::cursor_icon`):
  - Any orbit or pan drag: `Grabbing`.
  - Select: `Default`.
  - Line, Rectangle, Circle, Arc, Rotate and Tape Measure: `Crosshair`.
  - Move: `Move`.
  - Push/Pull: `ResizeVertical`.
  - Paint and Erase: `PointingHand` over a target, `Default` otherwise.
  - Fly mode keeps hiding the pointer.
  - The cursor is set only while the pointer is over the view, on no window or gizmo, and only when no widget set one this frame (the dock's resize edge keeps its own).
  - Custom cursor images are deferred.
- **Gizmo** (`ui::viewport::gizmo`):
  - Placement: a 72 x 72 pt `egui::Area` (`Order::Middle`, not movable) whose top-right corner is inset 12 pt from the view's. The Area makes `Gui::wants_pointer_at` true over it, so no press there reaches the tools.
  - The disc: a 36 pt radius disc in `bg_panel` at alpha 160.
  - The axes: each axis end lies at 26 pt × its unit direction projected onto the camera's right and up (y down). Positive ends get a 3 pt line from the centre and a filled 8 pt radius cap in the axis color, with "X", "Y" or "Z" in 11 pt Inter SemiBold `on_accent`. Negative ends get a 5 pt radius ring of 1.5 pt in the axis color.
  - Draw order: the ends are drawn and made clickable far to near, and the centre last. An end within 10 pt of the centre (its axis points at or away from the viewer) is neither drawn nor clickable.
  - The ends' names are "Right View" (+X), "Left View" (-X), "Back View" (+Y), "Front View" (-Y), "Top View" (+Z) and "Bottom View" (-Z). Each reports `UiAction::StandardView` for the view whose `direction().0` is that axis end.
  - The centre: a 7 pt radius disc in `bg_raised` with a 1 pt `border`, named "Switch Projection". It is AccessKit-toggled while parallel and reports `SetProjection` with the other projection.
  - Hovered ends and the centre get the `PointingHand` cursor. The gizmo is hidden while flying.
- **API.** `EditorState` gains `hovered: Option<EntityId>` (an id of the open context) and `snap_label: Option<String>`, both always present and `null` when there is none. Both are as of the last hover refresh, which is what the window shows, and both are in `tool_input`'s `editor`. The `editor` notification carries them, coalesced per frame, as the measurement already is. `tool_input`'s `inference` is still reported in Select (the snap exists; the window just does not draw it).
- **Tests.** Painted text is checked with `ui::tests::drawn_texts`. Painted circles and lines are checked with a new helper, `drawn_shapes`. GPU behaviour is checked by pixel counts in `crates/render/tests/headless.rs`. The final task runs a live look in the real window through a `.app` bundle wrapper (computer use cannot see a bare binary).

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/tools/src/overlay.rs` | marker shapes, snap labels, clipped screen lines, colors | 1, 9 |
| `crates/tools/src/tool.rs` | `Tool::hovered`, `ToolOutput::solid`, `Solid` | 2, 3 |
| `crates/tools/src/{select,push_pull,paint,erase}.rs` | hover targets; Push/Pull solid | 2, 3 |
| `crates/render/src/scene.rs` | `Spans::ranges` | 4 |
| `crates/render/src/renderer.rs`, `shaders/scene.wgsl` | hover flags and colors, preview solid pipeline, wide axes | 4, 5, 6 |
| `crates/render/src/style.rs`, `grid.rs` | `ViewStyle::{axes, wide_axes}`, `axis_lines_in` | 6 |
| `crates/render/tests/headless.rs` | GPU tests | 4, 5, 6 |
| `crates/api/src/editor.rs` | `EditorState::{hovered, snap_label}` | 7 |
| `crates/app/src/editor.rs` | `hovered`, `snap_label`, `navigating`, `hud`, `take_flash`, `solid_preview` | 7, 8, 9, 10 |
| `crates/app/src/host.rs`, `host/tool_input_tests.rs` | `editor_state` fields and tests | 7 |
| `crates/app/src/feedback.rs` (new) | `Hud`, `Chip`, `Timing` | 9 |
| `crates/app/src/ui/viewport.rs` (new) | HUD painting, `cursor_icon`, gizmo | 9, 10, 11 |
| `crates/app/src/ui/mod.rs`, `ui/tests/viewport.rs` (new), `ui/tests/mod.rs` | `View::{hud, cursor, camera}`, helpers | 9, 10, 11 |
| `crates/app/src/ui/theme.rs` | `view_style` axes, `hover_look`, HUD and gizmo metrics | 6, 8, 9, 11 |
| `crates/app/src/app.rs`, `main.rs` | wiring: hover, solid, HUD, timing deadlines, cursor | 8, 9, 10 |
| `docs/agent-api.md` | editor fields, gizmo row, chrome list | 12 |
| `docs/superpowers/roadmap.md` | M11c row | 13 |

---

### Task 1: Markers, snap labels and clipped preview lines as pure functions

**Files:**
- Modify: `crates/tools/src/overlay.rs`, `crates/tools/src/lib.rs` (exports)
- Test: `crates/tools/src/overlay.rs`

**Interfaces:**
- Consumes: `Inference`, `SnapKind`, `Axis`, `session::Camera` (`depth`, `depth_range`, `near`, `projector`, `is_parallel`).
- Produces (all `pub` in `tools::overlay`; the old `overlay()` and its `Segment` stay until Task 9):

```rust
/// sRGB and alpha, 0 to 255.
pub type Rgba = [u8; 4];
pub const AXIS_X: Rgba; // #E5484D
pub const AXIS_Y: Rgba; // #30A46C
pub const AXIS_Z: Rgba; // #3E63DD
pub const PARALLEL: Rgba; // #CC1ACC
pub const LINE_DAY: Rgba; // #1A1A1A
pub const LINE_NIGHT: Rgba; // #E8E9EC
pub const ENDPOINT: Rgba; // #2FBF4A
pub const MIDPOINT: Rgba; // #12B5CB
pub const INTERSECTION: Rgba; // #FF8A00
pub const ON_EDGE: Rgba; // #E5484D
pub const ON_FACE: Rgba; // #3E63DD
/// Marker size across, halo width, line widths, in points.
pub const MARKER_PT: f32 = 12.0;
pub const HALO_PT: f32 = 1.5;
pub const LINE_PT: f32 = 2.0;
pub const LOCKED_LINE_PT: f32 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape { Dot, Cross, Square, Diamond, Plus }
pub fn marker(kind: SnapKind) -> Option<(Shape, Rgba)>;

/// Where snapped geometry lies, seen from the open context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Within { #[default] Open, Group, Component, Outside }
pub fn snap_label(kind: SnapKind, within: Within) -> Option<String>;

/// A preview segment on screen, in view pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenLine { pub a: DVec2, pub b: DVec2, pub color: Rgba, pub width: f32 }
/// The part of `a`..`b` the camera sees (perspective: in front of the near plane; parallel:
/// within `depth_range`), projected; None when none of it is seen.
pub fn clip_segment(camera: &Camera, a: DVec3, b: DVec3) -> Option<[DVec2; 2]>;
pub fn preview_lines(
    preview: &[[DVec3; 2]],
    hit: Option<&Inference>,
    locked: bool,
    night: bool,
    camera: &Camera,
) -> Vec<ScreenLine>;
```

- [ ] **Step 1: Record the baseline**

```bash
cd ~/Desktop/side_projects/sketchup && export PATH="$HOME/.cargo/bin:$PATH"
git status && cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
```
Expected: a clean tree on `dev` and 1253.

- [ ] **Step 2: Write the failing tests** (in `overlay.rs`'s test module, beside the old ones)

```rust
#[test]
fn each_snap_kind_has_its_shape_and_color() {
    use SnapKind::*;
    assert_eq!(marker(Endpoint), Some((Shape::Dot, ENDPOINT)));
    assert_eq!(marker(Origin), Some((Shape::Dot, ENDPOINT)));
    assert_eq!(marker(Midpoint), Some((Shape::Dot, MIDPOINT)));
    assert_eq!(marker(Intersection), Some((Shape::Cross, INTERSECTION)));
    assert_eq!(marker(OnEdge), Some((Shape::Square, ON_EDGE)));
    assert_eq!(marker(OnGuide), Some((Shape::Square, ON_EDGE)));
    assert_eq!(marker(OnFace), Some((Shape::Diamond, ON_FACE)));
    assert_eq!(marker(GuidePoint), Some((Shape::Plus, ENDPOINT)));
    for k in [Axis(super::Axis::X), Parallel, Perpendicular, Ground, Free] {
        assert_eq!(marker(k), None, "{k:?}");
    }
    assert_eq!(ON_EDGE, [0xE5, 0x48, 0x4D, 0xFF]);
}

#[test]
fn snap_labels_name_the_snap_and_where_it_lies() {
    use SnapKind::*;
    assert_eq!(snap_label(Endpoint, Within::Open).as_deref(), Some("Endpoint"));
    assert_eq!(snap_label(Midpoint, Within::Group).as_deref(), Some("Midpoint in Group"));
    assert_eq!(snap_label(OnFace, Within::Component).as_deref(), Some("On Face in Component"));
    assert_eq!(snap_label(OnEdge, Within::Outside).as_deref(), Some("On Edge"));
    assert_eq!(snap_label(Axis(super::Axis::X), Within::Open).as_deref(), Some("On Red Axis"));
    assert_eq!(snap_label(Axis(super::Axis::Y), Within::Open).as_deref(), Some("On Green Axis"));
    assert_eq!(snap_label(Axis(super::Axis::Z), Within::Open).as_deref(), Some("On Blue Axis"));
    assert_eq!(snap_label(Parallel, Within::Open).as_deref(), Some("Parallel to Edge"));
    assert_eq!(snap_label(Perpendicular, Within::Open).as_deref(), Some("Perpendicular to Edge"));
    assert_eq!(snap_label(GuidePoint, Within::Open).as_deref(), Some("Guide Point"));
    assert_eq!(snap_label(Ground, Within::Open), None);
    assert_eq!(snap_label(Free, Within::Open), None);
}

#[test]
fn preview_lines_take_the_axis_color_and_widen_when_locked() {
    let c = cam();
    let hit = Inference { point: DVec3::new(0.0, 3.0, 0.0), kind: SnapKind::Axis(Axis::Y), face: None };
    let seg = [[DVec3::ZERO, hit.point]];
    let free = preview_lines(&seg, Some(&hit), false, false, &c);
    assert_eq!((free.len(), free[0].color, free[0].width), (1, AXIS_Y, LINE_PT));
    let locked = preview_lines(&seg, Some(&hit), true, false, &c);
    assert_eq!(locked[0].width, LOCKED_LINE_PT);
    assert_eq!(free[0].a, c.project(DVec3::ZERO).unwrap());
    let plain = Inference { kind: SnapKind::Free, ..hit };
    assert_eq!(preview_lines(&seg, Some(&plain), false, false, &c)[0].color, LINE_DAY);
    assert_eq!(preview_lines(&seg, Some(&plain), false, true, &c)[0].color, LINE_NIGHT);
    let par = Inference { kind: SnapKind::Parallel, ..hit };
    assert_eq!(preview_lines(&seg, Some(&par), false, false, &c)[0].color, PARALLEL);
}

#[test]
fn lines_crossing_the_near_plane_are_clipped_not_flipped() {
    let c = cam();
    let ahead = c.target;
    let behind = c.eye - c.forward() * 5.0;
    let [a, b] = clip_segment(&c, ahead, behind).expect("the part in front is seen");
    assert_eq!(a, c.project(ahead).unwrap());
    // The clipped end lies on the same side of the screen as the line heads toward the eye,
    // and stays finite.
    assert!(b.is_finite() && b.distance(a) > 1.0);
    let toward = c.project(ahead + (behind - ahead) * 0.1).unwrap() - a;
    assert!((b - a).dot(toward) > 0.0, "not flipped: {a} -> {b}");
    assert_eq!(clip_segment(&c, behind, behind - c.forward()), None);
    // Parallel: clipped to the depth range the same way.
    let mut p = cam();
    p.projection = session::Projection::Parallel;
    let (_, far) = p.depth_range();
    let out = p.eye + p.forward() * (far + 10.0);
    let [a, b] = clip_segment(&p, p.target, out).unwrap();
    assert_eq!(a, p.project(p.target).unwrap());
    assert!(b.is_finite());
}
```

The old `overlay` tests stay, untouched, until Task 9 migrates them.

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p tools overlay`
Expected: compile errors for `marker`, `snap_label`, `preview_lines`, `clip_segment`.

- [ ] **Step 4: Implement**

- `marker` and `snap_label` are tables (the Spec notes give them).
- `clip_segment`:
  - Measure each end's view depth: `(p - camera.eye).dot(camera.forward())`.
  - In perspective the lower bound is `camera.near()` and there is no upper bound; in parallel the bounds are `camera.depth_range()`.
  - Shrink the bounds inward by 0.1% so the clipped end projects.
  - Cut the segment where it leaves the bounds (linear in depth), then project both ends with one `camera.projector()`. Return None when both ends lie on the same outside side.
- `preview_lines` maps each segment through `clip_segment`, with color and width chosen once per call.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p tools overlay`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git status && git diff --stat
git add crates/tools/src/overlay.rs crates/tools/src/lib.rs
git commit -m "Overlay: shaped markers, snap labels, clipped screen lines"
```

---

### Task 2: Tools say what a click would act on

**Files:**
- Modify: `crates/tools/src/tool.rs`, `select.rs`, `push_pull.rs`, `paint.rs`, `erase.rs`
- Test: the same files' test modules

**Interfaces:**
- Consumes: `select::pick`, `paint`'s `target`, `face_under_in`.
- Produces: `Tool::hovered(&self) -> Option<EntityId>` (default `None`), with the doc comment from the Spec notes ("What a click would act on now..."). Implemented by `SelectTool`, `PushPullTool`, `PaintTool` and `EraseTool`, set on `ToolEvent::Move` and cleared by Click, Cancel and `on_external_change`.

- [ ] **Step 1: Write the failing tests**

- In `push_pull.rs`, replace `hovering_a_face_outlines_it` with its twin `hovering_a_face_reports_it_without_an_outline`. Over the face: `t.hovered() == Some(EntityId::Face(face))` and `out.preview.is_empty()`. Off it: `None`. After a click starts a drag: `None`.
- In `paint.rs`, the hover test (`"the face is outlined"`) becomes `hovering_reports_the_face_or_group_a_click_would_paint`. It asserts `hovered()` is the face, then the group over an instance, and that `out.preview` is empty.
- In `erase.rs`, `hovering_outlines_and_clicking_erases` becomes `hovering_reports_the_target_and_clicking_erases`. Over the edge `hovered() == Some(EntityId::Edge(e))`, inside the face `Some(Face)`, and off everything `None`. The preview is empty in every case.
- In `select.rs`, `hovering_reports_what_a_click_would_select`: an edge, then a face, then nothing. While the button is held (a box drag) it is `None`.
- In `tool.rs`, add:

```rust
#[test]
fn external_changes_clear_the_hover() {
    // A 2 m square seen from the default camera; the cursor over its middle.
    for kind in [ToolKind::Select, ToolKind::PushPull, ToolKind::Paint, ToolKind::Erase] {
        let (model, face) = square_model();
        let mut tool = kind.make();
        moved(&mut *tool, &model, DVec3::new(1.0, 1.0, 0.0), face);
        assert_eq!(tool.hovered(), Some(EntityId::Face(face)), "{kind:?}");
        tool.on_external_change();
        assert_eq!(tool.hovered(), None, "{kind:?}");
    }
    for kind in [ToolKind::Line, ToolKind::Move, ToolKind::TapeMeasure] {
        assert_eq!(kind.make().hovered(), None);
    }
}
```
(`square_model` and `moved` are small helpers built from the existing `phases` fixture: the same square, camera and `ToolContext`, sending `ToolEvent::Move`.)

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p tools hover`
Expected: FAIL (no `hovered`; outlines still emitted).

- [ ] **Step 3: Implement `hovered` in the four tools and remove the outlines**

- `PushPullTool.hover: Vec<Vec<DVec3>>` becomes `hovered: Option<FaceId>`.
- `PaintTool.outline` and `EraseTool.outline` become `hovered: Option<EntityId>`. Erase's `outline` function goes.
- `SelectTool` gains `hovered: Option<EntityId>`.
- The module docs that say "What is under the cursor is outlined" now say it is reported as hovered.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p tools`
Expected: PASS. The app's editor tests still pass, since they never read these outlines (check `cargo test -p app editor`).

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/tools/src/tool.rs crates/tools/src/select.rs crates/tools/src/push_pull.rs crates/tools/src/paint.rs crates/tools/src/erase.rs
git commit -m "Tools: report the hovered target instead of outlining it"
```

---

### Task 3: Push/Pull reports a solid

**Files:**
- Modify: `crates/tools/src/tool.rs` (`ToolOutput::solid`, `Solid`), `crates/tools/src/push_pull.rs`, `crates/tools/src/lib.rs` (export `Solid`)
- Test: `crates/tools/src/push_pull.rs`

**Interfaces:**
- Produces:

```rust
/// A translucent solid a tool previews (Push/Pull's pulled shape), in the open context's
/// coordinates, shown in `material`'s color (None: the default material).
#[derive(Clone, Debug, PartialEq)]
pub struct Solid { pub triangles: Vec<[DVec3; 3]>, pub material: Option<MaterialId> }
// ToolOutput gains:
/// A solid to draw as a live preview.
pub solid: Option<Solid>,
```

- [ ] **Step 1: Write the failing test** (`push_pull.rs`, using the module's rig over a 2 m square at z = 0)

```rust
#[test]
fn a_pull_previews_its_cap_and_sides_in_the_faces_material() {
    let mut r = Rig::new();
    r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
    let out = r.at(DVec3::new(1.0, 1.0, 2.0), ToolEvent::Move);
    let solid = out.solid.expect("a solid while pulling");
    // A square: 2 cap triangles and 2 per side.
    assert_eq!(solid.triangles.len(), 2 + 4 * 2);
    let tops = solid.triangles.iter().flatten().filter(|p| (p.z - 2.0).abs() < 1e-9).count();
    assert!(tops >= 6, "the cap is moved by the distance");
    assert!(solid.triangles.iter().flatten().all(|p| p.z > -1e-9 && p.z < 2.0 + 1e-9));
    assert_eq!(solid.material, r.model.face(r.face).unwrap().front);
    // No pull yet: no solid.
    let mut r = Rig::new();
    r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
    assert_eq!(r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Move).solid, None);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p tools a_pull_previews`
Expected: compile error (no `solid`).

- [ ] **Step 3: Implement**

- `Drag` keeps the face's triangles (`model.triangulate(face)`) and front material from the click.
- `handle` fills `out.solid` beside the wireframe while `drag.distance.abs() >= LENGTH_TOL`.
- The sides come from `drag.loops`: for each edge `p -> q`, the triangles `[p, q, q + o]` and `[p, q + o, p + o]`, with `o = normal * distance`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p tools`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/tools/src/tool.rs crates/tools/src/push_pull.rs crates/tools/src/lib.rs
git commit -m "Push/Pull: report the pulled solid"
```

---

### Task 4: The renderer highlights a hovered entity

**Files:**
- Modify: `crates/render/src/scene.rs` (`Spans::ranges`), `crates/render/src/renderer.rs`, `crates/render/src/shaders/scene.wgsl`, `crates/render/src/lib.rs` (export `HoverLook`)
- Test: `crates/render/src/scene.rs`, `crates/render/tests/headless.rs`

**Interfaces:**
- Produces:

```rust
/// Where one entity of the open context lies in a `SceneBase`'s lists (a group: all it places).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ranges { pub faces: Range<usize>, pub edges: Range<usize>, pub guides: Range<usize> }
impl Spans { pub fn ranges(&self, id: EntityId) -> Option<Ranges>; }

/// How the hovered entity is shown: its faces mixed `face_mix` toward `face` (sRGB), its
/// edges and guides in `edge`, edges about 2 px wide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoverLook { pub face: [f32; 3], pub face_mix: f32, pub edge: [f32; 4] }
impl Renderer {
    /// Shows `hover` (an id of the open context given to the last `set_model`) from the next
    /// frame; None shows nothing hovered. Only the old and new entity's flags are rewritten.
    pub fn set_hover(&mut self, hover: Option<(EntityId, HoverLook)>);
}
```

- [ ] **Step 1: Write the failing tests**

In `scene.rs`, add `ranges_match_the_flags_of_a_one_entity_selection`. For a face, an edge, a guide and an instance of the `forest()` fixture, set the ranges' entries in zeroed flags; the result must equal `spans.flags(&BTreeSet::from([id]))`. A vertex and an unknown id give `None`.

In `crates/render/tests/headless.rs`:

```rust
const HOVER: render::HoverLook =
    render::HoverLook { face: [1.0, 0.0, 1.0], face_mix: 0.25, edge: [1.0, 0.0, 1.0, 1.0] };

/// Renders the ground square with `hover`, twice through one renderer: hovered, then cleared.
fn hovered_frames(hover: Option<EntityId>) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> { /* plain, hovered, cleared */ }

#[test]
fn hovering_a_face_tints_it_toward_the_hover_color() {
    let model = ground_square();
    let face = model.faces().next().unwrap().0;
    let Some((plain, hovered, cleared)) = hovered_frames(Some(EntityId::Face(face))) else { return };
    let at = DVec3::new(0.5, 0.5, 0.0);
    let (p, h) = (pixel_at(&plain, &camera(), at), pixel_at(&hovered, &camera(), at));
    assert!(h[0] > p[0] && h[2] > p[2] && h[1] < p[1], "toward magenta: {p:?} -> {h:?}");
    assert_eq!(plain, cleared, "clearing the hover draws what was drawn before");
}

#[test]
fn a_hovered_edge_draws_wide_in_the_hover_color_whatever_the_style() {
    // CLASSIC style (wide_selected_edges false): hover still draws about 2 px.
    let model = ground_square();
    let edge = model.edges().next().unwrap().0;
    let Some((plain, hovered, _)) = hovered_frames(Some(EntityId::Edge(edge))) else { return };
    let thin = count(&plain, magenta);
    let wide = count(&hovered, magenta);
    assert_eq!(thin, 0);
    assert!(wide > 0);
    // About twice a 1 px line along the edge's screen length.
    let len = { let c = camera(); let (a, b) = edge_ends(&model, edge);
                c.project(a).unwrap().distance(c.project(b).unwrap()) };
    assert!(wide as f64 > 1.6 * len, "{wide} pixels for {len} px of edge");
}

#[test]
fn a_new_model_drops_the_hover() {
    let Some(screen) = Offscreen::new() else { eprintln!("no GPU adapter, skipping"); return };
    let cam = camera();
    let square = ground_square();
    let face = square.faces().next().unwrap().0;
    let mut r = screen.renderer();
    r.set_scene(&Scene::from_root(square), &[], &BTreeSet::new());
    r.set_hover(Some((EntityId::Face(face), HOVER)));
    // A model without that face: nothing hovered, nothing written out of range.
    let empty = Scene::from_root(Model::new());
    r.set_model(&empty, &[]);
    let after = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    r.set_hover(None);
    let mut fresh = screen.renderer();
    fresh.set_scene(&empty, &[], &BTreeSet::new());
    let expected = screen.draw(&mut fresh, &cam, Viewport::full((W, H)), &[]);
    assert_eq!(after, expected);
    assert_eq!(screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]), expected);
}
```
(`edge_ends` reads the edge's vertex positions; `hovered_frames` builds one `Offscreen`, `set_scene`s the ground square with `ViewStyle::CLASSIC`, draws, calls `set_hover(Some((id, HOVER)))`, draws, calls `set_hover(None)`, and draws.)

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p render --test headless hover && cargo test -p render ranges`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `Spans::ranges`, with `Spans::flags` written in terms of it.
- `Mesh` gains `hover: Option<wgpu::Buffer>`, created zeroed beside `flags` by `flagged_mesh`.
- The face and edge vertex layouts gain a third buffer, one f32 at the next free location (`@location(9)` for faces, `@location(3)` for edges).
- `set_hover` writes 0.0 over the old ranges and 1.0 over the new ones with `queue.write_buffer` at the range's byte offset. It rebuilds `self.guides` with `shown_guides` (so a guide no longer hovered gets its color back), recolors the hovered guide to `look.edge`, and records `edges_hovered`.
- `LightingUniforms` and WGSL `Lighting` gain `hover_face: [f32; 4]` (rgb, mix) and `hover_edge: [f32; 4]` after `canvas_bottom`; they are zero without a hover.
- `set_model` resets the stored hover.
- WGSL:
  - `FaceOut` gains `@location(9) @interpolate(flat) hovered: f32`.
  - In `face_color`, after `base`: `let shown = select(base, mix(base, lighting.hover_face.rgb, lighting.hover_face.w), in.hovered > 0.5);` Use `shown` where `base` was used after that line.
  - `vs_edge` wraps its color: `select(<today's expression>, lighting.hover_edge, hovered > 0.5)`.
  - `vs_edge_wide` draws edges that are selected or hovered, in `select(lighting.selected_edge, lighting.hover_edge, hovered > 0.5)`, and clips the rest.
  - `render_in` binds the wide pipeline when `(style.wide_selected_edges && edges_selected) || edges_hovered`, and every flagged draw binds vertex buffer 2.

- [ ] **Step 4: Run the render tests and the goldens**

Run: `cargo test -p render`
Expected: PASS, goldens included (they compare within tolerance; Task 13 checks bytes).

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/render/src/scene.rs crates/render/src/renderer.rs crates/render/src/shaders/scene.wgsl crates/render/src/lib.rs crates/render/tests/headless.rs
git commit -m "Render: highlight the hovered entity"
```

---

### Task 5: The renderer draws a translucent preview solid

**Files:**
- Modify: `crates/render/src/renderer.rs`, `crates/render/src/shaders/scene.wgsl`, `crates/render/src/lib.rs`
- Test: `crates/render/tests/headless.rs`

**Interfaces:**
- Produces:

```rust
/// A vertex of a tool's preview solid, in root coordinates; `color` is sRGB with its alpha.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PreviewVertex { pub pos: [f32; 3], pub normal: [f32; 3], pub color: [f32; 4] }
impl Renderer {
    /// Draws `triangles` (three vertices each) from the next frame, blended over the scene and
    /// hidden by what is in front of them; empty draws none. Re-uploads only when it changed.
    pub fn set_preview(&mut self, triangles: &[PreviewVertex]);
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_preview_solid_blends_over_what_is_behind_it_and_hides_behind_faces() {
    // A red box 1 m above the ground square at 70%: over the square, the pixel is reddish but
    // shows the square through it; clearing it gives back the plain frame.
    let Some(screen) = Offscreen::new() else { eprintln!("no GPU adapter, skipping"); return };
    let mut r = screen.renderer();
    r.set_scene(&Scene::from_root(ground_square()), &[], &BTreeSet::new());
    let cam = camera();
    let plain = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    r.set_preview(&box_vertices(DVec3::new(0.2, 0.2, 0.0), 0.6, [1.0, 0.0, 0.0, 0.7]));
    let shown = screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]);
    let p = pixel_at(&shown, &cam, DVec3::new(0.5, 0.5, 0.6));
    assert!(dominant(p, 0), "red: {p:?}");
    assert!(p[1] > 30 && p[2] > 30, "the square shows through: {p:?}");
    r.set_preview(&[]);
    assert_eq!(screen.draw(&mut r, &cam, Viewport::full((W, H)), &[]), plain);
}
```
Add `a_preview_solid_under_a_face_is_hidden`: a solid under the ground square, seen from above, leaves the frame identical to the plain one. `box_vertices` is a test helper building an axis-aligned box's 12 triangles with flat normals.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p render --test headless preview_solid`
Expected: compile error.

- [ ] **Step 3: Implement**

- A "preview solid" pipeline: `vs_preview` and `fs_preview`, a triangle list, depth `Greater` without writes, `BlendState::ALPHA_BLENDING`, no culling.
- `fs_preview` returns `vec4(color.rgb * (lighting.mode.y + lighting.mode.z * abs(dot(n, to_eye))), color.a)`, with `to_eye` as in `face_color`.
- The renderer keeps `preview: Option<Mesh>` and the last uploaded vertices. It draws them after the see-through faces and before the overlay.

- [ ] **Step 4: Run the render tests**

Run: `cargo test -p render`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/render/src/renderer.rs crates/render/src/shaders/scene.wgsl crates/render/src/lib.rs crates/render/tests/headless.rs
git commit -m "Render: draw a translucent preview solid"
```

---

### Task 6: Axes in the shared colors, 1.5 pt wide, in the window's style

**Files:**
- Modify: `crates/render/src/style.rs`, `crates/render/src/grid.rs`, `crates/render/src/renderer.rs`, `crates/render/src/shaders/scene.wgsl`, `crates/app/src/ui/theme.rs`
- Test: `crates/render/src/grid.rs`, `crates/render/tests/headless.rs`, `crates/app/src/ui/theme.rs`

**Interfaces:**
- Produces:
  - `ViewStyle` gains `pub axes: [[f32; 4]; 3]` (red, green, blue; sRGB) and `pub wide_axes: bool`. In CLASSIC they are `[grid::AXIS_X, grid::AXIS_Y, grid::AXIS_Z]` and `false`.
  - `grid::axis_lines_in(length: f32, colors: [[f32; 4]; 3], toward: [f32; 4]) -> Vec<LineVertex>`, where negative halves mix 60% toward `toward`.
  - `grid::flat_ground_lines_in(colors, toward)`, `grid::vertical_axis_lines_in(colors, toward)` and `grid::flat_axis_lines_in(colors, toward)` (just the X and Y axes).
  - The old functions call these with CLASSIC's colors and `BACKGROUND`.
  - `theme::view_style` sets `axes` from `theme::AXIS_X/Y/Z` and `wide_axes: true`.

- [ ] **Step 1: Write the failing tests**

- In `grid.rs`, `the_classic_axes_are_unchanged`: `axis_lines_in(1000.0, [AXIS_X, AXIS_Y, AXIS_Z], BACKGROUND) == axis_lines(1000.0)`, and the same for `flat_ground_lines_in` and `vertical_axis_lines_in`.
- In `headless.rs`:

```rust
#[test]
fn styled_axes_take_their_colors_and_wide_axes_cover_more_pixels() {
    let cam = camera();
    let none = BTreeSet::new();
    let magenta_axes = ViewStyle { axes: [[1.0, 0.0, 1.0, 1.0]; 3], ..ViewStyle::CLASSIC };
    let wide = ViewStyle { wide_axes: true, ..magenta_axes };
    let empty = Model::new();
    let (Some(classic), Some(thin), Some(thick)) = (
        render_styled(&cam, &empty, &none, ViewStyle::CLASSIC, Lighting::Day),
        render_styled(&cam, &empty, &none, magenta_axes, Lighting::Day),
        render_styled(&cam, &empty, &none, wide, Lighting::Day),
    ) else { return };
    assert_eq!(count(&classic, magenta), 0);
    let (t, w) = (count(&thin, magenta), count(&thick, magenta));
    assert!(t > 0, "the axes in the style's colors");
    assert!(w as f64 > 1.25 * t as f64, "{t} -> {w} pixels");
}
```
- In `theme.rs`, `the_views_axes_are_the_shared_axis_colors`: `view_style(Dark).axes[0]` equals `theme::AXIS_X` as f32 sRGB (0xE5 / 255, ...), and `wide_axes` is true.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p render axes && cargo test -p app view_style`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `set_style` rebuilds `flat_ground` and `vertical_axis` (and `flat_axes`, used for the wide copies) when `axes` or `day_background()` changed.
- The wide copies are two new pipelines, "wide flat axes" (`vs_flat_ground_wide`, depth `Always`) and "wide vertical axis" (`vs_ground_wide`, tested like `vs_ground`). Each draws its mesh with `0..2` instances, offset by `select(vec2(0.0, 1.0), vec2(1.0, 0.0), k == 0u) / lighting.lines.zw` in clip xy times w (half a pixel, as `vs_edge_wide` offsets a whole one).
- They are drawn right after the 1 px axes, only when `style.wide_axes`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p render && cargo test -p app theme`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/render/src/style.rs crates/render/src/grid.rs crates/render/src/renderer.rs crates/render/src/shaders/scene.wgsl crates/render/tests/headless.rs crates/app/src/ui/theme.rs
git commit -m "Axes: shared colors, 1.5 pt in the window's style"
```

---

### Task 7: The editor and the API report the hovered entity and the snap label

**Files:**
- Modify: `crates/app/src/editor.rs`, `crates/api/src/editor.rs`, `crates/app/src/host.rs`
- Test: `crates/app/src/editor.rs`, `crates/app/src/host/tool_input_tests.rs`

**Interfaces:**
- Consumes: `Tool::hovered` (Task 2), `overlay::{snap_label, Within}` (Task 1), `infer_detail`, `Scenery::part(k).owner`.
- Produces:
  - `Editor::hovered(&self) -> Option<EntityId>`: None while flying or with the cursor outside the view (`hover_hidden`), else `tool.hovered()`.
  - `Editor::snap_label(&self) -> Option<String>`: None in Select, while flying, or without a hit, else `snap_label(hit.kind, self.within)`.
  - `update_hover` calls `infer_detail` and keeps `within: Within`, worked out from the first involved part's owner: an `Owner::Instance(i)` gives Group or Component by its definition's kind, `Owner::Outside` gives Outside, and otherwise Open.
  - `api::EditorState` gains:

```rust
/// What a click would act on now (Select, Push/Pull, Paint, Erase): an id of the open
/// context; null otherwise.
pub hovered: Option<EntityId>,
/// The snap the window names by the cursor, e.g. "Endpoint" or "Midpoint in Group"; null
/// when there is none (always in Select).
pub snap_label: Option<String>,
```

- [ ] **Step 1: Write the failing tests**

In `tool_input_tests.rs`:

```rust
#[test]
fn results_report_the_hovered_target_and_the_snap_label() {
    let mut e = editor();
    square(&mut e); // a 2 m square face at the origin, as the other tests build it
    tool(&mut e, "push_pull");
    let r = input(&mut e, json!([{"type": "move", "point": [1, 1, 0]}]));
    assert!(r["editor"]["hovered"].as_str().unwrap().starts_with('f'), "{r}");
    tool(&mut e, "line");
    let r = input(&mut e, json!([{"type": "move", "point": [0, 0, 0]}]));
    assert_eq!(r["editor"]["snap_label"], "Endpoint");
    assert_eq!(r["editor"]["hovered"], Value::Null);
    tool(&mut e, "select");
    let r = input(&mut e, json!([{"type": "move", "point": [2, 1, 0]}]));
    assert_eq!(r["editor"]["snap_label"], Value::Null, "Select names no snap");
    assert!(r["editor"]["hovered"].as_str().unwrap().starts_with('e'));
    assert_eq!(ask(&mut e, json!({"type": "editor"}))["hovered"], r["editor"]["hovered"]);
}

#[test]
fn a_snap_inside_a_group_says_so() {
    let mut e = editor();
    square(&mut e);
    select_all(&mut e);
    exec(&mut e, json!({"type": "make_group"}));
    tool(&mut e, "line");
    let r = input(&mut e, json!([{"type": "move", "point": [1, 0, 0]}]));
    assert_eq!(r["editor"]["snap_label"], "Midpoint in Group");
}
```

In `editor.rs`:

```rust
#[test]
fn an_agent_edit_drops_the_highlight_until_the_next_frame() {
    let mut e = editor();
    square(&mut e);
    e.select_tool(ToolKind::Erase);
    // `square` draws 1..3 on both axes: the face's middle.
    move_to(&mut e, DVec3::new(2.0, 2.0, 0.0));
    e.refresh_hover();
    let face = e.hovered().expect("the face");
    e.execute(Command::Erase { ids: vec![face] }).unwrap();
    assert_eq!(e.hovered(), None, "nothing stale between the edit and the next frame");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app hovered snap_label highlight`
Expected: compile errors.

- [ ] **Step 3: Implement** the editor methods and fields, and `editor_state`'s two new fields. Check that `on_external_change` reaches the tool on agent edits: it does today, and Task 2 made it clear the hover.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app && cargo test -p api`
Expected: PASS. Any test that compares a whole `EditorState` JSON gains the two keys.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/editor.rs crates/api/src/editor.rs crates/app/src/host.rs crates/app/src/host/tool_input_tests.rs
git commit -m "API: the editor reports the hovered entity and the snap label"
```

---

### Task 8: The window highlights the hover and shows the Push/Pull solid

**Files:**
- Modify: `crates/app/src/editor.rs` (`solid_preview`, keep `ToolOutput::solid`), `crates/app/src/ui/theme.rs` (`hover_look`, `HOVER_TINT`), `crates/app/src/app.rs`
- Test: `crates/app/src/editor.rs`, `crates/app/src/ui/theme.rs`

**Interfaces:**
- Consumes: `Renderer::set_hover`, `HoverLook` (Task 4); `Renderer::set_preview`, `PreviewVertex` (Task 5); `Solid` (Task 3); `Editor::hovered` (Task 7).
- Produces:
  - `theme::HOVER_TINT: f32 = 0.25`.
  - `theme::hover_look(theme: egui::Theme, tool: ToolKind, target: EntityId, paint: [u8; 3]) -> render::HoverLook`, as in the Spec notes. `paint` is the paint material's sRGB, or `DEFAULT_FRONT` × 255 for the default material.
  - `Editor::solid_preview(&self) -> Vec<render::PreviewVertex>`: the tool's last `Solid`, in root coordinates (`doc.context_transform()`), with a flat normal per triangle and the material's color, or `render::scene::DEFAULT_FRONT`, at alpha 0.7. It is empty while flying.
  - The editor keeps `solid: Option<Solid>` from each Move's output, as it keeps `preview`.

- [ ] **Step 1: Write the failing tests**

```rust
// theme.rs
#[test]
fn the_hover_look_follows_the_tool() {
    let face = EntityId::Face(kernel::FaceId(1));
    let group = EntityId::Instance(kernel::InstanceId(2));
    let t = tokens(egui::Theme::Light);
    let rgb = |c: Color32| [c.r(), c.g(), c.b()].map(|u| f32::from(u) / 255.0);
    let select = hover_look(egui::Theme::Light, ToolKind::Select, face, [0; 3]);
    assert_eq!((select.face, select.face_mix), (rgb(t.accent), HOVER_TINT));
    let erase = hover_look(egui::Theme::Light, ToolKind::Erase, face, [0; 3]);
    assert_eq!(erase.face, rgb(t.danger));
    assert_eq!(&erase.edge[..3], &rgb(t.danger)[..]);
    let paint = hover_look(egui::Theme::Light, ToolKind::Paint, face, [255, 0, 0]);
    assert_eq!((paint.face, paint.face_mix), ([1.0, 0.0, 0.0], 1.0));
    assert_eq!(hover_look(egui::Theme::Light, ToolKind::Paint, group, [255, 0, 0]), select);
}

// editor.rs
#[test]
fn pulling_a_face_previews_a_translucent_solid() {
    let mut e = editor();
    square(&mut e);
    e.select_tool(ToolKind::PushPull);
    click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
    move_to(&mut e, DVec3::new(2.0, 2.0, 1.5));
    e.refresh_hover();
    let solid = e.solid_preview();
    assert_eq!(solid.len(), 3 * (2 + 4 * 2));
    assert!(solid.iter().all(|v| v.color[3] == 0.7));
    assert!(solid.iter().all(|v| v.color[..3] == render::scene::DEFAULT_FRONT[..]));
    e.key(EditorKey::Escape);
    e.refresh_hover();
    assert!(e.solid_preview().is_empty());
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app hover_look pulling_a_face`
Expected: compile errors.

- [ ] **Step 3: Implement and wire**

In `App::render`, after the selection update:
- `self.renderer.set_hover(self.editor.hovered().map(|id| (id, theme::hover_look(self.gui.theme(), self.editor.tool_kind(), id, paint))))`, where `paint` is the current material's color.
- `self.renderer.set_preview(&self.editor.solid_preview())`.

Both are no-ops when unchanged.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/editor.rs crates/app/src/ui/theme.rs crates/app/src/app.rs
git commit -m "Window: hover highlight and the Push/Pull solid"
```

---

### Task 9: Markers, lines, the snap chip and the flash, drawn by egui

**Files:**
- Create: `crates/app/src/feedback.rs`, `crates/app/src/ui/viewport.rs`, `crates/app/src/ui/tests/viewport.rs`
- Modify: `crates/app/src/main.rs` (`mod feedback;`), `crates/app/src/editor.rs` (`hud`, `take_flash`; remove `overlay`), `crates/tools/src/overlay.rs` (remove `overlay`, `Segment` and the old `[f32; 4]` constants once unused), `crates/app/src/ui/mod.rs` (`View::hud`, `mod viewport;`, `draw` paints it), `crates/app/src/ui/theme.rs` (chip metrics), `crates/app/src/app.rs` (timing, deadlines, `render_in(.., &[])`), `crates/app/src/ui/tests/mod.rs` (`Fixture::hud`, `drawn_shapes`)
- Test: `crates/app/src/feedback.rs`, `crates/app/src/editor.rs`, `crates/app/src/ui/tests/viewport.rs`

**Interfaces:**
- Consumes: `overlay::{marker, preview_lines, snap_label, Shape, Rgba, ScreenLine, MARKER_PT, HALO_PT}`, `Editor::snap_label` (Task 7), `Status`.
- Produces:

```rust
// feedback.rs
pub const LABEL_REST: Duration = Duration::from_millis(150);
pub const FLASH_TIME: Duration = Duration::from_millis(2500);
/// What the window draws over the 3D view for the tools; positions in view pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    pub marker: Option<(DVec2, Shape, Rgba)>,
    pub lines: Vec<ScreenLine>,
    pub chip: Option<Chip>,
    pub flash: Option<String>,
    /// The cursor, while it is in the view.
    pub cursor: Option<DVec2>,
    /// Night lighting: halos are black.
    pub night: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Chip { pub label: Option<String>, pub value: Option<String>, pub typing: bool }
#[derive(Clone, Debug, Default)]
pub struct Timing { /* moved: Option<Instant>, flash: Option<(String, Instant)> */ }
impl Timing {
    pub fn moved(&mut self, now: Instant);
    pub fn flash(&mut self, message: String, now: Instant);
    /// `hud` as shown at `now`: the chip only once rested and not `navigating`, the flash
    /// while it lasts.
    pub fn dress(&self, hud: Hud, now: Instant, navigating: bool) -> Hud;
    /// When the HUD next changes by itself (the rest's end, the flash's end); None when
    /// nothing is pending.
    pub fn deadline(&self, now: Instant) -> Option<Instant>;
}
// editor.rs
/// The marker, preview lines and chip contents as of the last `refresh_hover` (chip shown
/// regardless of the rest; `Timing::dress` decides).
pub fn hud(&self, night: bool) -> Hud;
/// The message of a typed value that just failed, once.
pub fn take_flash(&mut self) -> Option<String>;
// ui/viewport.rs
pub fn hud(ui: &Ui, view_rect: Rect, hud: &Hud);
/// Where the chip of `size` goes for a cursor at `cursor` (points): (14, 18) past it, flipped
/// to stay inside `view_rect`.
pub fn chip_rect(cursor: Pos2, size: Vec2, view_rect: Rect) -> Rect;
// ui/mod.rs: View gains `pub hud: &'a Hud`.
```

- [ ] **Step 1: Write the failing tests**

`feedback.rs`:

```rust
#[test]
fn timing_asks_for_one_frame_per_change_then_none() {
    let t0 = Instant::now();
    let mut t = Timing::default();
    assert_eq!(t.deadline(t0), None, "idle from the start");
    t.moved(t0);
    assert_eq!(t.deadline(t0), Some(t0 + LABEL_REST));
    let rested = t0 + LABEL_REST;
    assert_eq!(t.deadline(rested), None, "the frame at the rest is the last");
    t.flash("Not a length: x".into(), rested);
    assert_eq!(t.deadline(rested), Some(rested + FLASH_TIME));
    assert_eq!(t.deadline(rested + FLASH_TIME), None);
}

#[test]
fn the_chip_waits_for_the_rest_and_hides_while_navigating() {
    let t0 = Instant::now();
    let mut t = Timing::default();
    t.moved(t0);
    let hud = Hud { chip: Some(Chip { label: Some("Endpoint".into()), value: None, typing: false }), ..Hud::default() };
    assert_eq!(t.dress(hud.clone(), t0, false).chip, None);
    assert!(t.dress(hud.clone(), t0 + LABEL_REST, false).chip.is_some());
    assert_eq!(t.dress(hud.clone(), t0 + LABEL_REST, true).chip, None);
    t.flash("Not a length: x".into(), t0);
    assert_eq!(t.dress(hud.clone(), t0, false).flash.as_deref(), Some("Not a length: x"));
    assert_eq!(t.dress(hud, t0 + FLASH_TIME, false).flash, None);
}
```

`editor.rs`: migrate `hovering_an_endpoint_shows_a_marker` and the overlay-length assertions (`e.overlay().len()`) as twins on `hud(false)`. Also migrate the magenta test (`tools::overlay::MAGENTA` becomes `PARALLEL`, read from `hud.lines[0].color`). Then add:

```rust
#[test]
fn select_shows_no_marker_and_a_typed_error_flashes_once() {
    let mut e = editor();
    square(&mut e);
    let corner = DVec3::new(1.0, 1.0, 0.0);
    move_to(&mut e, corner);
    e.refresh_hover();
    assert_eq!(e.hud(false).marker.map(|m| (m.1, m.2)), Some((Shape::Dot, ENDPOINT)));
    e.select_tool(ToolKind::Select);
    move_to(&mut e, corner);
    e.refresh_hover();
    assert_eq!(e.hud(false).marker, None);
    e.select_tool(ToolKind::Rectangle);
    click_at(&mut e, DVec3::ZERO);
    type_text(&mut e, "abc");
    let flash = e.take_flash().expect("the refused value");
    assert!(e.status().contains(&flash), "it stays in the status bar");
    assert_eq!(e.take_flash(), None);
}
```

`ui/tests/viewport.rs` (`Fixture` gains `hud: Hud`, default empty):

```rust
#[test]
fn the_chip_shows_the_label_and_the_value() {
    let mut f = Fixture::default();
    f.hud = Hud { cursor: Some(DVec2::new(300.0, 200.0)), chip: Some(Chip {
        label: Some("Midpoint in Group".into()), value: Some("1.5 m".into()), typing: false }),
        flash: Some("Not a length: x".into()), ..Hud::default() };
    let h = harness(f);
    let texts = drawn_texts(&h);
    for t in ["Midpoint in Group", "1.5 m", "Not a length: x"] {
        assert!(texts.iter().any(|s| s == t), "{t} in {texts:?}");
    }
}

#[test]
fn the_chip_stays_inside_the_view() {
    let view = Rect::from_min_size(Pos2::new(44.0, 36.0), Vec2::new(600.0, 400.0));
    let size = Vec2::new(120.0, 40.0);
    let r = chip_rect(Pos2::new(100.0, 100.0), size, view);
    assert_eq!(r.min, Pos2::new(114.0, 118.0));
    let r = chip_rect(Pos2::new(640.0, 430.0), size, view);
    assert!(view.contains_rect(r), "{r:?}");
    assert!(r.max.x <= 640.0 && r.max.y <= 430.0, "flipped left and up");
}

#[test]
fn the_hud_lands_on_its_pixels_at_2x() {
    // pixels_per_point 2: a marker at view pixel (200, 100) is drawn centred at
    // view.min + (100, 50) points, its dot 12 points across.
    let mut f = Fixture::default();
    f.hud = Hud { marker: Some((DVec2::new(200.0, 100.0), Shape::Dot, ENDPOINT)), ..Hud::default() };
    let h = harness_scaled(f, 2.0);
    let view = h.state().central;
    let dots = drawn_shapes(&h).into_iter().filter_map(|s| match s {
        egui::Shape::Circle(c) if c.fill == Color32::from_rgb(0x2F, 0xBF, 0x4A) => Some(c),
        _ => None }).collect::<Vec<_>>();
    assert_eq!(dots.len(), 1);
    assert_eq!(dots[0].center, view.min + Vec2::new(100.0, 50.0));
    assert_eq!(dots[0].radius, MARKER_PT / 2.0);
}

#[test]
fn the_hud_settles_in_one_frame() {
    let mut f = Fixture::default();
    f.hud = Hud { cursor: Some(DVec2::new(300.0, 200.0)),
        marker: Some((DVec2::new(300.0, 200.0), Shape::Cross, INTERSECTION)),
        chip: Some(Chip { label: Some("Intersection".into()), value: None, typing: false }),
        flash: Some("x".into()), ..Hud::default() };
    let mut h = harness(f);
    assert_eq!(h.try_run().unwrap(), 1);
}
```

`harness_scaled(f, ppp)` and `drawn_shapes(h) -> Vec<egui::Shape>` (flattening `Shape::Vec`) are new helpers in `tests/mod.rs`. `drawn_texts_of` reuses `drawn_shapes`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app feedback viewport select_shows_no_marker`
Expected: compile errors.

- [ ] **Step 3: Implement**

- `Editor::hud`:
  - The marker: no marker in Select, while flying, or with the hover hidden. Otherwise the hit, moved to root coordinates and projected with `doc.camera`, with its `marker()` shape and color.
  - The lines: `preview_lines(root preview, hit, self.lock.is_some(), night, &doc.camera)`.
  - The chip: `Chip { label: self.snap_label(), value: status.value, typing: !typed.is_empty() }`, or None when both lines are None.
  - `cursor`: `Some(self.cursor)` unless the hover is hidden.
- `take_flash`: `send` records `flash` when the event was a `Value` and `self.message` is Some afterwards.
- `ui::viewport::hud`: paints with `ui.painter_at(view_rect)` after the panels in `draw`, so windows and menus stay above it. Lines go first, then the marker (halo, then shape), then the chip and the flash at `chip_rect`. The colors and metrics are in the Spec notes; the chip's metrics are `theme::CHIP_OFFSET = Vec2::new(14.0, 18.0)`, `CHIP_PADDING = Vec2::new(6.0, 4.0)` and `CHIP_RADIUS = SWATCH_RADIUS`.
- `App`:
  - It owns `timing: Timing`.
  - `cursor_moved` calls `timing.moved(now)` when the editor reports a move inside the view.
  - Each frame: `if let Some(m) = editor.take_flash() { timing.flash(m, now) }`, then `let hud = timing.dress(editor.hud(night), now, editor.navigating())`, then `View { hud: &hud, .. }`. `Editor::navigating` is added in Task 10; until then pass `false`.
  - After presenting: `if let Some(t) = timing.deadline(now) { repaint_at = Some(earliest(repaint_at, t)) }`.
  - `render_in` gets `&[]`.
- Delete `Editor::overlay`, `tools::overlay::overlay`, `Segment` and the unused old constants. Migrate their tests as twins (marker size and display scale become `the_hud_lands_on_its_pixels_at_2x` and an editor twin checking `hud.marker` follows `scale_factor` in view pixels).

- [ ] **Step 4: Run the tests and count**

Run: `cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'`
Expected: at least the count before this task.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/feedback.rs crates/app/src/ui/viewport.rs crates/app/src/ui/tests/viewport.rs crates/app/src/ui/tests/mod.rs crates/app/src/main.rs crates/app/src/editor.rs crates/app/src/ui/mod.rs crates/app/src/ui/theme.rs crates/app/src/app.rs crates/tools/src/overlay.rs crates/tools/src/lib.rs
git commit -m "Viewport: shaped markers, wide preview lines, snap chip and flash"
```

---

### Task 10: Tool cursors

**Files:**
- Modify: `crates/app/src/ui/viewport.rs` (`cursor_icon`, applied in `draw`), `crates/app/src/ui/mod.rs` (`View::cursor`), `crates/app/src/editor.rs` (`navigating`), `crates/app/src/app.rs`, `crates/app/src/ui/tests/mod.rs` (`Fixture::cursor`)
- Test: `crates/app/src/ui/tests/viewport.rs`, `crates/app/src/editor.rs`

**Interfaces:**
- Produces:
  - `ui::viewport::cursor_icon(tool: ToolKind, hovering: bool, navigating: bool) -> egui::CursorIcon`, as in the Spec notes.
  - `View::cursor: egui::CursorIcon`.
  - `Editor::navigating(&self) -> bool`: `nav.dragging()`.
  - In `draw`, after the panels and the HUD: if the pointer is over `view_rect`, `ctx.layer_id_at(pointer)` is None or `Order::Background`, and `ctx.output(|o| o.cursor_icon) == CursorIcon::Default`, then `ctx.set_cursor_icon(view.cursor)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn each_tool_has_its_cursor() {
    use egui::CursorIcon as C;
    for (tool, icon) in [(ToolKind::Select, C::Default), (ToolKind::Line, C::Crosshair),
        (ToolKind::Rectangle, C::Crosshair), (ToolKind::Circle, C::Crosshair), (ToolKind::Arc, C::Crosshair),
        (ToolKind::TapeMeasure, C::Crosshair), (ToolKind::Rotate, C::Crosshair), (ToolKind::Move, C::Move),
        (ToolKind::PushPull, C::ResizeVertical), (ToolKind::Paint, C::Default), (ToolKind::Erase, C::Default)] {
        assert_eq!(cursor_icon(tool, false, false), icon, "{tool:?}");
    }
    assert_eq!(cursor_icon(ToolKind::Paint, true, false), C::PointingHand);
    assert_eq!(cursor_icon(ToolKind::Erase, true, false), C::PointingHand);
    assert_eq!(cursor_icon(ToolKind::Line, false, true), C::Grabbing);
}

#[test]
fn the_view_shows_the_tools_cursor_and_the_panels_keep_theirs() {
    let mut h = harness(Fixture { cursor: egui::CursorIcon::Crosshair, ..Default::default() });
    let view = h.state().central;
    h.event(egui::Event::PointerMoved(view.center()));
    h.run_ok();
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Crosshair);
    h.event(egui::Event::PointerMoved(rect_of(&h, "Line").center()));
    h.run_ok();
    assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::Crosshair);
}
```

In `editor.rs`: `a_middle_drag_is_navigating`, which checks `navigating()` across a middle press, move and release.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app cursor navigating`
Expected: compile errors.

- [ ] **Step 3: Implement.** In `App::render`, set `cursor: viewport::cursor_icon(tool, editor.hovered().is_some(), editor.navigating())`, and replace Task 9's `false` with `editor.navigating()`. Fly mode's `hide_cursor` still wins in `Gui::paint`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/ui/viewport.rs crates/app/src/ui/mod.rs crates/app/src/editor.rs crates/app/src/app.rs crates/app/src/ui/tests/mod.rs crates/app/src/ui/tests/viewport.rs
git commit -m "Viewport: a cursor per tool, grabbing while navigating"
```

---

### Task 11: The orientation gizmo

**Files:**
- Modify: `crates/app/src/ui/viewport.rs` (`gizmo`, `gizmo_ends`), `crates/app/src/ui/mod.rs` (`View::camera`; `draw` calls `gizmo` unless flying), `crates/app/src/ui/theme.rs` (gizmo metrics), `crates/app/src/app.rs`, `crates/app/src/ui/tests/mod.rs` (`Fixture::camera`)
- Test: `crates/app/src/ui/tests/viewport.rs`

**Interfaces:**
- Produces:
  - `View::camera: &'a session::Camera`. In the fixture the default is `Camera::new(DVec2::new(1280.0, 800.0))`.
  - `ui::viewport::gizmo_ends(camera: &Camera) -> [(StandardView, Vec2, f32); 6]`: each end's view, its offset from the centre in points (y down), and how far it points toward the viewer (`-dir.dot(forward)`).
  - `ui::viewport::gizmo(ui: &Ui, view_rect: Rect, camera: &Camera, actions: &mut Vec<UiAction>)`.
  - Theme metrics: `GIZMO = 72.0`, `GIZMO_INSET = 12.0`, `GIZMO_AXIS = 26.0`, `GIZMO_CAP = 8.0`, `GIZMO_NEG_CAP = 5.0`, `GIZMO_CENTRE = 7.0`, `GIZMO_HIDE = 10.0`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_gizmo_sets_standard_views_and_switches_projection() {
    let mut h = harness(Fixture::default());
    assert_eq!(click(&mut h, "Front View"), [UiAction::StandardView(StandardView::Front)]);
    assert_eq!(is_on(&h, "Switch Projection"), Some(false));
    assert_eq!(click(&mut h, "Switch Projection"), [UiAction::SetProjection(Projection::Parallel)]);
    let mut h = harness(Fixture { projection: Projection::Parallel, ..Default::default() });
    assert_eq!(is_on(&h, "Switch Projection"), Some(true));
    assert_eq!(click(&mut h, "Switch Projection"), [UiAction::SetProjection(Projection::Perspective)]);
}

#[test]
fn the_gizmo_sits_in_the_views_top_right() {
    let h = harness(Fixture::default());
    let (view, centre) = (h.state().central, rect_of(&h, "Switch Projection").center());
    let expected = Pos2::new(view.max.x - 12.0 - 36.0, view.min.y + 12.0 + 36.0);
    assert!(centre.distance(expected) < 1.0, "{centre:?} vs {expected:?}");
}

#[test]
fn the_axis_ends_set_the_views_they_look_from() {
    let ends = gizmo_ends(&Camera::new(DVec2::new(800.0, 600.0)));
    let end = |v: StandardView| *ends.iter().find(|e| e.0 == v).unwrap();
    for (pos, neg) in [(StandardView::Right, StandardView::Left),
                       (StandardView::Back, StandardView::Front),
                       (StandardView::Top, StandardView::Bottom)] {
        let (p, n) = (end(pos), end(neg));
        assert!((p.1 + n.1).length() < 1e-3, "{pos:?} and {neg:?} are opposite");
        assert!((p.2 + n.2).abs() < 1e-3);
        assert!(p.1.length() <= GIZMO_AXIS + 1e-3);
    }
    assert!(end(StandardView::Right).1.x > 0.0, "+X on the right in the default view");
    assert!(end(StandardView::Top).1.y < 0.0, "+Z up the screen");
}

#[test]
fn an_axis_pointing_at_the_viewer_leaves_the_centre_clickable() {
    let mut camera = Camera::new(DVec2::new(1280.0, 800.0));
    camera.look_from(StandardView::Top);
    let mut h = harness(Fixture { camera, ..Default::default() });
    assert!(h.query_by_label("Top View").is_none());
    assert!(h.query_by_label("Bottom View").is_none());
    assert_eq!(click(&mut h, "Switch Projection"), [UiAction::SetProjection(Projection::Parallel)]);
}

#[test]
fn the_gizmo_takes_the_pointer_from_the_tools() {
    let h = harness(Fixture::default());
    let at = rect_of(&h, "Front View").center();
    let layer = h.ctx.layer_id_at(at).expect("the gizmo's area");
    assert_ne!(layer.order, egui::Order::Background, "Gui::wants_pointer_at is true there");
    assert!(h.query_by_label("Switch Projection").is_some());
    let flying = harness(Fixture { flying: true, ..Default::default() });
    assert!(flying.query_by_label("Switch Projection").is_none());
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app gizmo axis_ends`
Expected: compile errors.

- [ ] **Step 3: Implement** as in the Spec notes. Each end and the centre are `ui.interact` click widgets named with `Response::widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, name))` (the centre's `selected` is "parallel"; the ends' `false`). They are added far to near, then the centre. In `App`, `camera: &self.editor.doc().camera`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p app`
Expected: PASS, including `the_panels_settle_in_one_frame`.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/ui/viewport.rs crates/app/src/ui/mod.rs crates/app/src/ui/theme.rs crates/app/src/app.rs crates/app/src/ui/tests/mod.rs crates/app/src/ui/tests/viewport.rs
git commit -m "Viewport: orientation gizmo for standard views and projection"
```

---

### Task 12: Docs: hover, snap label and the gizmo in the agent API

**Files:**
- Modify: `docs/agent-api.md`

- [ ] **Step 1: Edit**

- **Editor:** in the example JSON add `"hovered": "f3", "snap_label": "Endpoint"`. Add a bullet: `hovered` is what a click would act on now with Select, Push/Pull, Paint or Erase (an id of the open context, `null` otherwise), and `snap_label` is the snap the window names by the cursor ("Endpoint", "On Red Axis", "Parallel to Edge", "Midpoint in Group"; `null` when none, and always in Select). Both are as the window last drew them, and change as the cursor moves.
- **Notifications:** the `editor` row's "When" adds "what is under the cursor".
- **Driving the tools:** after the result example, add one sentence: `editor.hovered` and `editor.snap_label` in the result say what the window highlights and names after the events. `inference` is reported in Select too, though the window draws no marker there.
- **Window actions table:**
  - Change the row "Inference snapping, Shift and arrow-key locks, the marker under the cursor" to "..., the marker and the snap label by the cursor" and add `editor.snap_label`.
  - Add a row: "Hover highlight (Select, Push/Pull, Paint, Erase)" -> `editor.hovered`; `pick` for any pixel.
  - Add a row: "The orientation gizmo: an axis end, the centre" -> `standard_view`, `set_projection`.
- **Chrome paragraph:** add the cursors, when the chip appears (the 150 ms rest), the 2.5 s flash of a refused typed value (its message is `editor.message`), the Push/Pull solid preview (`tool_input`'s `preview` has its edges), and the axes' window colors. Then add: `render_view` never shows the highlight, markers, preview lines, solid or gizmo.

- [ ] **Step 2: Check** that `cargo test -p api` still passes (doc tests over method lists, if any) and that the file has no emoji.

- [ ] **Step 3: Commit**

```bash
git status && git diff --stat
git add docs/agent-api.md
git commit -m "Docs: hover, snap label and gizmo in the agent API"
```

---

### Task 13: Roadmap, whole-branch review, byte-identical goldens, live look

- [ ] **Step 1: Full gate**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo build --release -p app
git status --short crates/render/tests/golden/
```
Expected:
- clean;
- at least 1253 plus this milestone's new tests;
- a release build;
- no golden image changed.

- [ ] **Step 2: Roadmap row**

In `docs/superpowers/roadmap.md`, change the M11c row's last cell from `planned` to `done <first>..<last>`, using the short hashes of Task 1's commit and Task 12's commit.

```bash
git status && git diff --stat
git add docs/superpowers/roadmap.md
git commit -m "Roadmap: M11c done"
```

- [ ] **Step 3: Whole-branch review**

Use superpowers:requesting-code-review over `<base>..HEAD`, where `<base>` is the commit before Task 1, against the spec's M11c bullet and this plan's Spec notes and Review Focus. Ask the reviewer to check in particular:
- the WGSL changes keep CLASSIC's arithmetic: every new effect sits behind `select` on a value that is 0 there, or in a pipeline CLASSIC never binds;
- `render_view` never calls `set_hover` or `set_preview` and stays CLASSIC;
- there is no unconditional repaint: `Timing::deadline` is the only new wake, and `the_hud_settles_in_one_frame` holds;
- preview lines are clipped at the near plane and the depth range;
- `set_hover` writes only inside the current buffers after model changes;
- the gizmo's Area keeps presses from the tools;
- every Spec note value matches the code (colors, sizes, offsets, cursor table);
- no test was lost (compare with Task 1's baseline).

Fix findings in follow-up commits and rerun Step 1.

- [ ] **Step 4: Byte-identical CLASSIC goldens**

```bash
git worktree add ../sketchup-m11c-base <base>
git worktree add ../sketchup-m11c-head HEAD
(cd ../sketchup-m11c-base && UPDATE_GOLDEN=1 cargo test -p render --test golden)
(cd ../sketchup-m11c-head && UPDATE_GOLDEN=1 cargo test -p render --test golden)
for f in ../sketchup-m11c-base/crates/render/tests/golden/*.png; do
  cmp "$f" "../sketchup-m11c-head/crates/render/tests/golden/$(basename "$f")" || echo "DIFF $f"
done
git worktree remove --force ../sketchup-m11c-base && git worktree remove --force ../sketchup-m11c-head
```
Expected: no `DIFF` line. A difference means a CLASSIC path changed: find it, fix it, and repeat.

- [ ] **Step 5: Live look in the real window**

1. Build a `.app` bundle wrapper in the scratchpad around the release binary. Give it its own bundle id and socket, as the M11b memory note describes; a bare binary is invisible to computer use.
2. Open it and check, by screenshot:
   - On a box: the endpoint dot with its halo and the "Endpoint" chip after a rest.
   - A red-axis Line preview at 2 pt, and 2.5 pt with Shift held.
   - The typed "abc" flash for 2.5 s.
   - The Push/Pull highlight and the translucent solid.
   - Erase in red and Paint previewing the material.
   - Crosshair and grabbing cursors.
   - The gizmo: Front View, then Switch Projection.
   - Activity Monitor showing about 0% CPU once the window is still.
3. Report what was seen. Fix anything wrong in follow-up commits.

Report the milestone's summary with the rulings in "Spec notes".
