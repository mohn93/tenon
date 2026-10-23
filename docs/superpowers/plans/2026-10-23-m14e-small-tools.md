# M14e Smaller Drawing and Camera Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SketchUp's Freehand, Rotated Rectangle, Protractor, Look Around and Position Camera tools, each with a command for agents and `tool_input` coverage.

**Architecture:** New session commands where none exists: `add_curve` (a polyline, on a curve or not), `add_rotated_rectangle`, `look_around` and `position_camera` (camera commands, no undo step). The Protractor emits the existing `add_guide_line`. Five new tools in the tools crate on the existing `Tool` trait (drags arrive as Click, Move stream, Release; typed values as `ToolEvent::Value`), a Douglas-Peucker simplifier and a slope parser (`rise:run`) in the tools crate, registered in `ToolKind`, the api `ToolName`, menus, rail, icons, cursors and shortcuts.

**Tech Stack:** Rust 2024, egui 0.36.2. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M14e.

## Global Constraints

- No new dependencies. Existing tools and commands unchanged; golden images unchanged.
- New command variants get arms in `Document::execute`/`edit`, the undoable or camera lists, `session::batch::id_slots` (no `_` arm), api dispatch and `docs/agent-api.md` in the task that adds them. New tools get every registration site and `docs/agent-api.md` "Driving the tools" entries.
- Gate per task (project rules in the SDD rules files): native gate + `cargo check --target wasm32-unknown-unknown -p kernel -p session -p tools -p render -p api -p app --lib -p web`; `scripts/build-web.sh` in Task 4.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **`add_curve { points, closed, curve }`** (curve default true): draws the polyline with the usual edge rules (faces close as for drawn edges); with `curve` the edges go on one new curve (as `add_curve` in the kernel; a straight polyline gets no curve). 2 to 10,000 points, coordinate limits, consecutive duplicates dropped, a closed polyline needs 3 distinct points. One undo step; outcome `created`.
2. **Freehand tool** (no shortcut): press, drag, release. The path lies in the plane of the face under the press point (or the horizontal plane through it at the current drawing axes), sampled from the pointer, simplified with Douglas-Peucker at 2 px projected to model units at the press point's depth, at most 10,000 points. Releasing near the start (within the inference snap radius) closes it. Shift held at the press: a polyline (`curve: false`). Esc cancels. Preview while dragging.
3. **`add_rotated_rectangle { corner, edge_end, width_point }`**: the first side is corner -> edge_end; the width is the distance from width_point to the line of that side, and the rectangle lies in the plane through the side and width_point, toward width_point. Zero side or zero width (or width_point on the side's line) is `invalid_input`. Draws the closed loop (the face forms as for a drawn rectangle). One undo step; outcome `created`.
4. **Rotated Rectangle tool** (no shortcut): click the first corner, click the second (the first side, any direction, with inference and arrow-key locks), then move to set the width and plane and click. Typed after the first click: `length` or `length,angle` (angle in degrees in the drawing plane from the red axis, measured as the Protractor does); after the second click: `width` or `width,angle` (angle of the rectangle's plane about the first side, from the default plane: horizontal, or the face under the first corner). Esc steps back. Preview of the rectangle.
5. **Protractor tool** (no shortcut): click the centre (a point, with inference), click a point on the baseline (with inference to axes and edges), move to set the angle (snapping to 15 degree steps as the Rotate protractor does), click to place a guide line through the centre at that angle in the protractor's plane (the face under the centre, or the plane of the arrow-key lock as for Rotate). Typed after the baseline: an angle in degrees (`30`, `-45`) or a slope `rise:run` (`1:2` is atan(1/2)); the last guide is replaced by a typed value right after a click, as the Tape Measure does. Ctrl toggles measuring only (no guide). Emits `add_guide_line`. Draws the existing protractor visual.
6. **`look_around { yaw, pitch }`** (degrees): turns the view about the eye (yaw about the world up, pitch about the camera's right; the view's pitch stays within -89..89 degrees); camera command, no undo step. **`position_camera { point, eye_height, look_at }`**: eye = point + eye_height up (eye_height 0.01..10,000 m, default 1.676 m, SketchUp's 5'6"); the view looks level toward `look_at` when given (its height ignored), else keeps the current horizontal heading; switches to perspective; camera command.
7. **Look Around tool** (shortcut W): drag turns the view (0.0025 radian per pixel, as Fly Mode's look); typed value sets the eye height above the face directly under the eye, else above z = 0 (`position_camera` at the point under the eye). **Position Camera tool** (shortcut H): click a point to stand there at the eye height; press-drag-release from A to B stands at A looking toward B; typed value before the click sets the eye height (remembered for the session); after placing, the tool becomes Look Around (as SketchUp). Both emit only camera commands (no model change, no undo).
8. **Window:** Draw menu gains Freehand and Rotated Rectangle; Tools menu gains Protractor; Camera menu gains Look Around and Position Camera; rail buttons and icons; cursors; the phone viewer gets none.

## Review Focus

1. Commands validate input and never panic (NaN, huge, degenerate); undo for the model commands; camera commands make no undo step and keep the camera valid (no NaN up vector at pitch limits, parallel to perspective).
2. Freehand: simplification keeps the shape within tolerance, closing, Shift polyline, plane choice on a face and on the ground, a 10,000-sample drag stays fast.
3. Rotated Rectangle and Protractor: typed values and angle conventions consistent with Rotate and the drawing axes (M14c); guides at the right angle in the right plane.
4. Tools are driven by `tool_input` (drags with many points, typed values, modifiers) and listed in the api `ToolName`.

---

### Task 1: The commands

**Files:** `crates/session/src/{command.rs, document.rs, document/camera.rs, batch.rs}`, `crates/kernel/src/curves.rs` (if `add_curve` needs a `curve` flag), api dispatch, `docs/agent-api.md`; tests in session and api suites.

- [ ] **Step 1: Write the failing tests** for rulings 1, 3 and 6 (Review Focus 1).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Curve, rotated rectangle and camera commands"`.

### Task 2: Freehand, Rotated Rectangle and Protractor tools

**Files:** `crates/tools/src/{(new) freehand.rs, (new) rotated_rectangle.rs, (new) protractor.rs, (new) simplify.rs, vcb.rs (slope), tool.rs (ToolKind), lib.rs}`, `crates/api/src/editor.rs (ToolName)`, docs; tests in the tools crate (drive helpers).

- [ ] **Step 1: Write the failing tests** for rulings 2, 4 and 5 (Review Focus 2-3).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Freehand, Rotated Rectangle and Protractor"`.

### Task 3: Look Around and Position Camera tools

**Files:** `crates/tools/src/{(new) look_around.rs, (new) position_camera.rs, tool.rs (ToolKind, Remembered eye height)}`, `crates/api/src/editor.rs`, `crates/app/src/editor.rs` (tool switch after positioning, camera outputs), docs; tests.

- [ ] **Step 1: Write the failing tests** for ruling 7.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Look Around and Position Camera"`.

### Task 4: The tools in the window

**Files:** `crates/app/src/{menus.rs, native_menu.rs, shortcuts.rs, ui/rail.rs, ui/icons.rs, ui/viewport.rs, host/tool_input_tests.rs}`, `docs/agent-api.md`; tests (menus, shortcuts W and H, rail, tool_input drags for each tool, native ids stable, phone viewer).

- [ ] **Step 1: Write the failing tests** for ruling 8 and Review Focus 4.
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "New tools in the window"`.

### After the tasks (controller)

- Push `dev`; CI green; roadmap row M14e done; spec bullet; memory; merge and push; next milestone (M14f).
