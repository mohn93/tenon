# M13a1 Polygon and Scale Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two SketchUp tools, each as a command and a tool that agents can also drive: Polygon (`add_polygon`, a regular polygon drawn as a curve marked as a polygon) and Scale (`scale`, factors along the model axes about a fixed point, with box grips in the window).

**Architecture:** Polygons reuse the curve machinery (`Model::add_curve`) with a new per-edge `polygon` flag that travels wherever an edge's curve id travels; `Curve::kind` reads it, and Push/Pull leaves polygon sides hard. Scale generalises `Model::transform_vertices` from rigid motions to any invertible affine map (normals by the inverse transpose, loops reversed under a mirror) and reuses `transform_instances` and `transform_guides`, which already accept any affine map. The tools follow the Circle and Rotate tools; grips are drawn with the existing line preview.

**Tech Stack:** Rust 2024, glam (DAffine3/DMat3), egui 0.36.2, egui-phosphor =0.14.0. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md`, the M13a1 bullet under "Modeling parity milestones".

## Global Constraints

- No new crates or dependencies.
- Units are meters, Z up, angles in degrees. Model coordinates are bounded by `session::MAX_COORDINATE` (1.0e6); new points go through `check_coordinates` / `check_placed` like the existing commands.
- Existing behaviour is unchanged: circles, arcs, rotate and move keep their results; golden images unchanged; every existing test keeps passing (tests that enumerate tools are updated to include the new ones).
- `.skr` files written before this milestone open unchanged; the new edge field is `#[serde(default, skip_serializing_if = "std::ops::Not::not")]` so files without polygons are byte-for-byte as before.
- New command variants get an arm in `session::batch::id_slots` (no `_` arm) and in `Document::edit` / `execute`; `reference_keys_cover_every_id_field` must keep passing.
- Gate per task: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Use `export PATH="$HOME/.cargo/bin:$PATH"`. Full-suite runs take 10-25 minutes; run focused tests while iterating.
- `docs/agent-api.md` is updated in the task that changes the API. No emoji. Commit messages short, no attribution lines.
- Never `git add -A`; stage the files you changed.

## Spec notes (rulings)

1. **Polygon flag representation:** `kernel::Edge` gets `pub polygon: bool` (serde default false, skipped when false). Every edge of a curve has the same flag (checked in `validate_curves`). Wherever an edge's `curve` is carried to a new edge (edge splits in edit.rs, stencil marks in transform.rs used by copy/group/explode, Push/Pull's moved cap in push_pull.rs), its `polygon` flag is carried with it. A plain edge (no curve) is never `polygon`.
2. **Curve kind:** `CurveKind::Polygon` is added. `Curve` gets `pub polygon: bool`. `kind()`: polygon and closed and on a circle → `Polygon`; polygon otherwise (opened, or corners moved off the circle) → `Curve`; non-polygon curves as today. JSON `"polygon"`.
3. **Pulled polygons:** `mark_swept_curves` softens a swept edge only between two sides of one non-polygon curve. The moved copy of a polygon curve is a polygon curve.
4. **`add_polygon`:** `{center, radius, sides?, normal?, circumscribed?, direction?}`. `sides` default 6, 3 to 360 (`MAX_CIRCLE_SEGMENTS`), out of range `invalid_input`. `normal` default Z (via `unit_vector`). `direction` default the plane's u axis from `rectangle_axes(normal)`; otherwise projected into the plane, and `invalid_input` when its in-plane part is shorter than 1e-9 or not finite. Corner k (inscribed) is at `center + R (cos a_k * d + sin a_k * (n x d))` with `a_k = TAU k / sides`, `R = radius`, `d` the unit in-plane direction; circumscribed: `R = radius / cos(PI / sides)` and `a_k = TAU k / sides + PI / sides` (so the middle of the first side lies along `d`). Corners counter-clockwise around the normal. Side length below `10 * LENGTH_TOL` is `invalid_input` (as for circles). One undo step, label `add_polygon`; drawn with sticky geometry (merges, auto face) like `add_circle`.
5. **Polygon tool:** key `n`, label "Polygon", icon `ph::HEXAGON`, in the Draw group after Circle (rail, Draw menu, `ToolKind` order after `Circle`). First click: center and plane (as the Circle tool). Second click (or release after a drag): corner point; radius = distance from center to the point projected into the plane, direction = that projected offset. Typed `Ns` sets sides (remembered across tools in `Remembered::polygon_sides`, default 6; message "Polygons get N sides"); a typed length with a center set finishes with that radius along the current hover direction (u axis if none). Ctrl tap (`ToolEvent::ToggleCopy`) switches inscribed and circumscribed (message "Polygons are drawn circumscribed" / "Polygons are drawn inscribed"; remembered for the session in `Remembered::polygon_circumscribed`). Preview: the polygon loop and the center-to-cursor segment.
6. **`scale`:** `{ids, origin, factors}` (`factors: [f64; 3]`, along model x, y, z, about `origin`). Each factor finite, not zero, `|f| <= 1e4` and `|f| >= 1e-4`, else `invalid_input` "scale factors must be finite and between 1e-4 and 1e4 in size"; all three exactly 1 → `invalid_input` "nothing to scale: every factor is 1". `ids` as for `rotate` (edges, faces, vertices, instances, guides; empty → `invalid_input`; unknown → `not_found`). Transform `t = translate(origin) * scale(factors) * translate(-origin)`, applied with `transform_vertices`, `transform_instances`, `transform_guides`, after `check_placed`. One undo step, label `scale`. No copies.
7. **Affine `transform_vertices`:** the rigid-motion `debug_assert` becomes a check that `|det(t.matrix3)| >= 1e-12` (else `KernelError::Squash`). Faces whose vertices all move get `normal = (M^-1)^T n`, normalised; when `det < 0` their outer loop and each hole are reversed (keep the first vertex first) so winding stays counter-clockwise around the new normal. Faces that move partly keep today's code (a fold under a mirror is `Squash`). Every edge — not only those between moved and fixed vertices — shorter than `LENGTH_TOL` after the transform is `Squash`. Rotations and translations give exactly today's results (the existing tests pin this). Texture placements of moved faces are left as `transform_vertices` leaves them today.
8. **Entity Info for instances:** when an instance's placement is not a pure rotation, show `Scale: sx, sy, sz` (column lengths of `matrix3`, the x value negated when `det < 0`, formatted with the existing `number`), and compute the shown rotation from the columns divided by those lengths (x column negated when `det < 0`). A pure rotation shows exactly what it shows today.
9. **Scale tool:** key `s`, label "Scale", icon `ph::ARROWS_OUT`, in the modify group after Rotate (rail, Tools menu, `ToolKind` order after `Rotate`).
   - The box is the model-axis bounds of the selection: vertex positions of selected edges, faces (outer and hole loops) and vertices, `scenery.instance_box` of instances, and guide anchors (new `pub(crate) fn selection_bounds(ctx: &ToolContext, ids: &BTreeSet<EntityId>) -> Option<[DVec3; 2]>` in `crates/tools/src/select.rs`).
   - An axis is flat when the box's extent along it is below `LENGTH_TOL`; its factor is always 1. With every axis flat the tool shows "Nothing to scale: select something with size" and does nothing.
   - Grips: for every sign vector `s` in `{-1,0,1}^3` other than zero whose non-zero entries are all on non-flat axes, a grip at `center + s * half`. Its active axes are the non-zero entries of `s`; its opposite grip is `center - s * half`. Corner grips (3 active axes) scale uniformly by default; edge (2) and face (1) grips along their active axes; Shift held flips this for the current move. "Uniform" scales every non-flat axis by one factor.
   - Grip hover: the grip whose projection (`camera.project`) is nearest the cursor within 10 physical pixels. Drawn as a camera-facing square of 4 segments with side `0.012 * camera.eye_distance(grip)` (1.5 times that when hovered), plus the 12 box edges.
   - With nothing selected, a click on geometry selects what the Select tool's click would (`Command::Select`, whole curve for a curve edge), and grips appear for the new selection.
   - A click on a grip (or a press there and release after a drag, via `DragStart`) starts scaling: the fixed point is the opposite grip, or the box center when "about center" is on. Ctrl tap (`ToggleCopy`) switches "about center" (message "Scaling about the center" / "Scaling about the opposite grip"), also while scaling.
   - While scaling, the cursor point `p` is the inference point when it snapped to a point (`is_point`), else for uniform or one-axis grips the point on the line from the fixed point through the grip nearest the cursor ray, and for two-axis non-uniform grips the cursor ray's intersection with the plane through the grip spanned by the two axes (falling back to the line when the ray is parallel to it). With `d = grip - fixed`: uniform factor `dot(p - fixed, d) / dot(d, d)`; non-uniform factor per active axis `(p - fixed)_i / d_i`, others 1.
   - Preview: the box and the selection outline (`move_tool::outline`) carried through the current scale. `measurement()` shows the factors, e.g. `1.5` or `1.5, 2`.
   - The next click finishes with `Command::Scale { ids: selection, origin: fixed, factors }`. Any active factor with `|f| < 1e-4` instead shows "Scale must not be zero" and keeps scaling.
   - Typed values (new `vcb::parse_scale(&str) -> Option<Vec<ScaleValue>>`, `enum ScaleValue { Factor(f64), Length(f64) }`): comma-separated; a plain number (optional `-`, decimals) is a factor; a number with a unit (anything `parse_length` accepts that is not a plain number) is a length; mixing kinds is `None`. One value applies to every axis that moves (uniform mode: all non-flat axes; otherwise the grip's active axes); several values must match the active axes in x, y, z order, else the message "Type 1 or N values". A length becomes the factor `length / |d|` (one value) or `length_i / |d_i|` (per axis), signed like the current factor. While scaling a typed value finishes; after a scale (nothing in progress) it redoes the last one as `[Command::Undo { count: None, expect: None }, Command::Scale {…}]` with the same ids, origin and moving axes, as the Rotate tool's retype does.
   - `anchor()` returns None (Shift is the uniform toggle, not an inference lock). `phase()` is `Active` while scaling. Esc (`Cancel`) drops the scaling in progress.

## Review Focus

1. Mirroring a closed box (`factors [-1, 1, 1]`) must leave every face's normal pointing out of the box and every loop counter-clockwise around its normal; the box must still push/pull and render the right way out. Test in Task 4.
2. A polygon copied, grouped, exploded, split by a line to one side's middle, pulled, saved and reopened must still read as a polygon (or, when opened up, as a curve) and its pulled sides must stay hard. Test in Task 1.
3. Scaling an instance non-uniformly then querying Entity Info must not show a wrong rotation; scaling a rotated group along model axes is allowed and stays pickable. Test in Task 4.
4. Typing a scale right after a scale made by a click redoes it (undo + scale); typing with no previous scale and nothing in progress shows a message and runs nothing. Test in Task 5.
5. Polygon side counts and circumscribed mode persist across tool switches but not into a fresh session; `8s` before the first click and `3s`/`361s`/`2.5s` refusals behave like the Circle tool's. Test in Task 3.

---

### Task 1: Polygon curves in the kernel

**Files:**
- Modify: `crates/kernel/src/model.rs` (`Edge`, `Edge::new`, `insert_edge_like`, `set_edge`, `validate_curves`)
- Modify: `crates/kernel/src/curves.rs` (`Curve.polygon`, `CurveKind::Polygon`, `kind()`, `add_curve` signature, `tag_along`)
- Modify: `crates/kernel/src/edit.rs` (split keeps the flag), `crates/kernel/src/transform.rs` (stencil marks carry it), `crates/kernel/src/push_pull.rs` (`side_curves`, `mark_swept_curves`)
- Modify: `crates/kernel/src/data.rs` (persist the flag)
- Modify: `crates/session/src/query.rs` (`CurveInfo` docs: kind `polygon`), `crates/app/src/info.rs` (`curve_info`)
- Test: `crates/kernel/tests/curves.rs`, `crates/session/tests/curves.rs`

**Interfaces:**
- Produces: `pub enum CurveKind { Circle, Arc, Curve, Polygon }`; `Curve { …, pub polygon: bool }`; `Model::add_curve(&mut self, points: &[DVec3], closed: bool, polygon: bool) -> Result<ChangeSet, KernelError>` (existing callers pass `false`); `Edge { v, curve, soft, polygon }`.
- Consumed by Task 2: `add_curve(&points, true, true)`.

- [ ] **Step 1: Write the failing tests** in `crates/kernel/tests/curves.rs` (build a hexagon with a local helper giving 6 points on a circle of radius 1 around (0,0,0), and `m.add_curve(&pts, true, true)`):
  - `a_polygon_curve_reads_as_a_polygon`: the curve of one of its edges has `kind() == CurveKind::Polygon`, `polygon == true`, `segments == 6`, `circle.unwrap().radius` within 1e-9 of 1.0. The same points with `polygon: false` read as `Circle`.
  - `a_pulled_polygon_has_hard_sides`: after push-pulling the hexagon's face by 1, no edge is soft and the top loop's edges form one curve of kind `Polygon`; a 6-segment circle pulled the same way has 6 soft edges (today's behaviour).
  - `copies_splits_and_files_keep_the_polygon_mark`: copy the polygon's edges and face with `copy` (offset (3,0,0)): the copy's curve is a `Polygon` with a different id; draw an edge from the middle of one side to the center (splitting that side): the curve still has `segments == 6` and kind `Polygon`; round-trip the model through `data` (the same serialize/deserialize helpers the existing file tests use): kinds unchanged. Erase one side edge: the remaining chain's kind is `Curve`.
  - `polygon_flags_agree_along_a_curve`: `validate` rejects a model whose curve has one edge with `polygon: false` and the others `true` (construct through `data`'s deserializing path the way existing `validate_curves` tests do).
- [ ] **Step 2: Run** `cargo test -p kernel --test curves` — expected: compile failure (`add_curve` arity, `CurveKind::Polygon`).
- [ ] **Step 3: Implement** rulings 1-3. `Curve::kind` per ruling 2; `curve()` sets `polygon` from its edges. `curve_info` in info.rs: first line `"1 polygon"` for `Polygon`, then `Radius: …`, `Sides: N` (in place of `Segments:`), `Length: …`. Update `CurveInfo`'s doc comment and the `### Curves` section of `docs/agent-api.md` (kind `polygon`, radius is the corner radius).
- [ ] **Step 4: Run** the gate — expected: all pass; golden images unchanged.
- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/model.rs crates/kernel/src/curves.rs crates/kernel/src/edit.rs crates/kernel/src/transform.rs crates/kernel/src/push_pull.rs crates/kernel/src/data.rs crates/kernel/tests/curves.rs crates/session/src/query.rs crates/app/src/info.rs docs/agent-api.md
git commit -m "Kernel: curves marked as polygons, hard sides when pulled"
```
(Add any other file the `add_curve` signature change touched.)

### Task 2: `add_polygon` command

**Files:**
- Create: `crates/session/src/polygon.rs` (`polygon_points`)
- Modify: `crates/session/src/command.rs` (`Command::AddPolygon`), `crates/session/src/document.rs` (`execute`/`edit` arms, `add_polygon_changes`), `crates/session/src/batch.rs` (`id_slots` arm), `crates/session/src/lib.rs`
- Modify: `crates/api/tests/dispatch.rs` (the describe command list), `docs/agent-api.md` (command list line ~88, a row after `add_circle`)
- Test: `crates/session/tests/polygon.rs` (new), `crates/session/tests/curves.rs`

**Interfaces:**
- Consumes: `Model::add_curve(points, true, true)` (Task 1).
- Produces: `pub fn polygon_points(center: DVec3, radius: f64, sides: u32, normal: Option<DVec3>, circumscribed: bool, direction: Option<DVec3>) -> Result<Vec<DVec3>, CommandError>` and `pub const DEFAULT_POLYGON_SIDES: u32 = 6;` exported from `session`. `Command::AddPolygon { center: [f64;3] (ModelPoint schema), radius: f64, sides: Option<u32>, normal: Option<[f64;3]>, circumscribed: bool (serde default), direction: Option<[f64;3]> }`.

- [ ] **Step 1: Write the failing tests** in `crates/session/tests/polygon.rs`:
  - `polygon_points_inscribed_and_circumscribed`: `polygon_points((0,0,0), 1, 4, None, false, None)` gives (1,0,0), (0,1,0), (-1,0,0), (0,-1,0) within 1e-12; with `circumscribed: true` the first corner is (1,1,0) (radius / cos(45°) at 45°) within 1e-12; with `direction: Some((0,2,0))` the inscribed first corner is (0,1,0).
  - `polygon_points_refuses_bad_input`: sides 2 and 361 → `InvalidInput`; direction (0,0,5) with the default normal → `InvalidInput`; radius 1e-7 → `InvalidInput`.
  - `add_polygon_makes_one_face_and_one_polygon_curve`: `execute(AddPolygon {center (0,0,0), radius 1, sides None, …})` → outcome creates 1 face and 6 edges; the `curve` query of one created edge has `kind == "polygon"` and `segments == 6`; one undo step labelled `add_polygon` (`history` query).
  - `add_polygon_over_json`: `{"type":"add_polygon","center":[0,0,0],"radius":2,"sides":8,"circumscribed":true}` deserializes and executes; unknown keys are refused.
- [ ] **Step 2: Add** `groups_and_explode_keep_the_polygon_mark` to `crates/session/tests/curves.rs` (Review Focus 2): `add_polygon`, `make_group` of everything, `explode` the group: the `curve` query still says `polygon`; save to a temp `.skr` and `open` it: still `polygon`.
- [ ] **Step 3: Run** `cargo test -p session --test polygon --test curves` — expected: compile failure.
- [ ] **Step 4: Implement** ruling 4. `add_polygon_changes` mirrors `add_circle_changes` (document.rs ~688). Add `"add_polygon"` to the describe command list in `crates/api/tests/dispatch.rs`. Document the command in `docs/agent-api.md` (command list and a table row in the style of `add_circle`, with the defaults, ranges and the curve kind).
- [ ] **Step 5: Run** the gate — expected: all pass.
- [ ] **Step 6: Commit**

```bash
git add crates/session/src/polygon.rs crates/session/src/command.rs crates/session/src/document.rs crates/session/src/batch.rs crates/session/src/lib.rs crates/session/tests/polygon.rs crates/session/tests/curves.rs crates/api/tests/dispatch.rs docs/agent-api.md
git commit -m "Session: add_polygon command"
```

### Task 3: Polygon tool

**Files:**
- Create: `crates/tools/src/polygon.rs` (`PolygonTool`)
- Modify: `crates/tools/src/lib.rs`, `crates/tools/src/tool.rs` (`ToolKind::Polygon` after `Circle`, `ALL` length, `make_with`, `label`, `shortcut` `'n'`, `Remembered { polygon_sides, polygon_circumscribed }` and `Default`), tool.rs tests that enumerate drawing kinds
- Modify: `crates/app/src/shortcuts.rs` (`TABLE` entry after Circle, length), `crates/app/src/menus.rs` (Draw after Circle), `crates/app/src/ui/rail.rs` (Draw group after Circle), `crates/app/src/ui/icons.rs` (`ph::HEXAGON`), `crates/app/src/ui/viewport.rs` (`cursor_icon`: as Circle), `crates/app/src/editor.rs` (`toggle_copy` also for Polygon)
- Modify: `crates/api/src/editor.rs` (`ToolName::Polygon` and both `From` impls)
- Modify: UI tests listing tools: `crates/app/src/ui/tests/menus.rs` (Draw list), `crates/app/src/ui/tests/viewport.rs` (cursor table), `crates/app/src/ui/tests/layout.rs` (rail pairs if they change)
- Modify: `docs/agent-api.md` (tool names in the editor query, the `drag` row, the window-feature table)
- Test: `crates/tools/src/polygon.rs` unit tests, `crates/app/src/host/tool_input_tests.rs`

**Interfaces:**
- Consumes: `session::polygon_points`, `DEFAULT_POLYGON_SIDES`, `Command::AddPolygon` (Task 2); `vcb::{parse_segments, meant_as_count, parse_length}`; `rectangle::{plane_normal, on_plane}`; `DragStart`.
- Produces: `pub struct PolygonTool` with `with_settings(sides: u32, circumscribed: bool) -> PolygonTool`; `ToolKind::Polygon`; `ToolName::Polygon` (JSON `"polygon"`).

- [ ] **Step 1: Write the failing tests.**
  - Unit tests in `crates/tools/src/polygon.rs`, following `circle.rs`'s tests: `two_clicks_draw_a_hexagon` (center (0,0,0), corner (2,0,0) → one `AddPolygon` with radius 2, sides 6, direction (2,0,0) projected, circumscribed false); `typed_sides_and_radius` (`"8s"` then center then `"1.5"` → sides 8, radius 1.5; `"2s"`, `"361s"`, `"2.5s"` give the Circle tool's bad-sides message style and change nothing); `ctrl_switches_circumscribed` (ToggleCopy then two clicks → `circumscribed: true`, message "Polygons are drawn circumscribed"); `remember_keeps_sides_and_mode` (after `remember`, `Remembered` holds them; `ToolKind::Polygon.make_with(&r)` starts with them).
  - In `crates/app/src/host/tool_input_tests.rs`: `polygon_draws_from_clicks_and_typed_sides` — `tool(&mut e, "polygon")`, events `type "5s"`, `click [0,0,0]`, `move [1,0,0]`, `type "2"` → `ran(&r) == ["add_polygon"]`, the command's `sides == 5`, `radius == 2`; then `set_tool select`, `set_tool polygon`, two clicks → sides still 5 (Review Focus 5).
- [ ] **Step 2: Run** `cargo test -p tools polygon` — expected: compile failure.
- [ ] **Step 3: Implement** ruling 5 and the registrations listed under Files. Update the enumerating tests (`ALL` length, shortcut table length, rail/menu/cursor lists).
- [ ] **Step 4: Run** the gate — expected: all pass.
- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/polygon.rs crates/tools/src/lib.rs crates/tools/src/tool.rs crates/app/src/shortcuts.rs crates/app/src/menus.rs crates/app/src/ui/rail.rs crates/app/src/ui/icons.rs crates/app/src/ui/viewport.rs crates/app/src/editor.rs crates/api/src/editor.rs crates/app/src/ui/tests crates/app/src/host/tool_input_tests.rs docs/agent-api.md
git commit -m "Polygon tool"
```

### Task 4: Affine transforms and the `scale` command

**Files:**
- Modify: `crates/kernel/src/transform.rs` (`transform_vertices`)
- Modify: `crates/session/src/command.rs` (`Command::Scale`), `crates/session/src/document.rs` (`scale_changes`, `execute`/`edit` arms), `crates/session/src/batch.rs` (`id_slots`: `Slot::List(ids)`)
- Modify: `crates/app/src/info.rs` (`rotation` and a scale line, ruling 8)
- Modify: `crates/api/tests/dispatch.rs` (describe list), `docs/agent-api.md` (command list, a row after `rotate`)
- Test: `crates/kernel/tests/transform.rs` (or the existing kernel test file that covers `transform_vertices`; create `crates/kernel/tests/scale.rs` if none fits), `crates/session/tests/scale.rs` (new), info.rs unit tests

**Interfaces:**
- Produces: `Command::Scale { ids: Vec<EntityId>, origin: [f64;3] (ModelPoint schema), factors: [f64;3] }`; `transform_vertices` accepts any invertible affine map.

- [ ] **Step 1: Write the failing tests.**
  - Kernel (`scale.rs`): `scaling_a_box_doubles_it` (a 1 m box's vertices scaled by (2,2,2) about the origin: bounds (0,0,0)-(2,2,2), every face normal unchanged); `stretching_keeps_faces_flat_and_normals_right` (box scaled (3,1,1): side normals still axis-aligned unit vectors); `mirroring_keeps_faces_outward` (Review Focus 1: box scaled (-1,1,1) about its center: for every face, its normal points away from the box center and `vector_area(outer)` is along the normal; then push-pull the top face by 1 succeeds and adds volume upward); `shrinking_an_edge_below_tolerance_is_refused` (scale (1e-9,1,1) on a whole box → `KernelError::Squash`); `rotations_are_unchanged` (an existing rotate scenario gives identical positions and normals to before — reuse the rotate test inputs).
  - Session (`scale.rs`): `scale_command_scales_geometry_instances_and_guides` (a rectangle, a group and a guide point scaled by (2,1,1) about (0,0,0): positions/bounds doubled in x; the instance's `transform` x column is doubled; one undo step labelled `scale`; undo restores everything); `scale_refuses_bad_factors` (`[0,1,1]`, `[1,1,1]`, `[1e5,1,1]`, `[NaN…]` via JSON → `invalid_input`; empty ids → `invalid_input`; unknown id → `not_found`); `scale_a_rotated_group_along_model_axes` (Review Focus 3: rotate a group 30° about Z, scale it (2,1,1): ok, and a `pick` at its new extent hits it — use the existing pick helpers in the session/api tests).
  - info.rs: `entity_info_shows_scale_and_true_rotation` — an instance with transform rotation 90° about Z times scale (2,1,1) shows `Scale: 2, 1, 1` and `Rotation: 90° about 0, 0, 1`; a mirrored one (-1,1,1) shows `Scale: -1, 1, 1`; a pure rotation's lines are as before.
- [ ] **Step 2: Run** the new tests — expected: compile failures / failures.
- [ ] **Step 3: Implement** rulings 6-8. `scale_changes` follows `rotate_changes` without copies (document.rs ~810). Document `scale` in `docs/agent-api.md` (command list and a row with origin, factors, limits, mirror behaviour).
- [ ] **Step 4: Run** the gate — expected: all pass, golden images unchanged.
- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/transform.rs crates/kernel/tests crates/session/src/command.rs crates/session/src/document.rs crates/session/src/batch.rs crates/session/tests/scale.rs crates/app/src/info.rs crates/api/tests/dispatch.rs docs/agent-api.md
git commit -m "Scale command: affine vertex transforms, mirrored faces stay outward"
```

### Task 5: Scale tool

**Files:**
- Create: `crates/tools/src/scale.rs` (`ScaleTool`)
- Modify: `crates/tools/src/vcb.rs` (`ScaleValue`, `parse_scale`), `crates/tools/src/select.rs` (`selection_bounds`), `crates/tools/src/lib.rs`, `crates/tools/src/tool.rs` (`ToolKind::Scale` after `Rotate`, `ALL`, `make_with`, `label`, `shortcut` `'s'`)
- Modify: `crates/app/src/shortcuts.rs`, `crates/app/src/menus.rs` (Tools after Rotate), `crates/app/src/ui/rail.rs` (modify group after Rotate), `crates/app/src/ui/icons.rs` (`ph::ARROWS_OUT`), `crates/app/src/ui/viewport.rs` (`cursor_icon`: as Rotate), `crates/app/src/editor.rs` (`toggle_copy` also for Scale; `apply` needs no special message)
- Modify: `crates/api/src/editor.rs` (`ToolName::Scale`)
- Modify: UI tests listing tools (menus Tools list, cursor table, rail pair `(Rotate, TapeMeasure)` → `(Scale, TapeMeasure)`)
- Modify: `docs/agent-api.md` (tool names, how to drive Scale with `tool_input`: click a grip by its model point, `key ctrl`, `modifiers shift`, typed values; window-feature table)
- Test: unit tests in `crates/tools/src/scale.rs` and `vcb.rs`, `crates/app/src/host/tool_input_tests.rs`

**Interfaces:**
- Consumes: `Command::Scale` (Task 4); `move_tool::outline`; `select::{pick, whole_curve}`; `DragStart`; `Camera::{ray, project, eye_distance}`.
- Produces: `pub struct ScaleTool`; `ToolKind::Scale`; `ToolName::Scale` (JSON `"scale"`); `pub enum ScaleValue { Factor(f64), Length(f64) }`, `pub fn parse_scale(text: &str) -> Option<Vec<ScaleValue>>`.

- [ ] **Step 1: Write the failing tests.**
  - `vcb.rs`: `parse_scale_reads_factors_and_lengths`: `"2"` → `[Factor(2)]`; `"-1"` → `[Factor(-1)]`; `"1.5, 2"` → two factors; `"3m"` → `[Length(3)]`; `"250mm,1m"` → two lengths; `"2,3m"`, `""`, `"x"` → None.
  - `scale.rs` unit tests (selection: a 2x2x2 box at the origin, set through `ctx.selection`): `grips_cover_a_box` (26 grips; for a flat 2x2 rectangle in z=0, 8 grips: 4 corners and 4 edge middles); `corner_grip_scales_uniformly_about_the_opposite_corner` (click the grip at (2,2,2), move to (3,3,3) along the diagonal, click → `Scale { origin: [0,0,0], factors: [1.5,1.5,1.5] }`); `face_grip_scales_one_axis_and_shift_makes_it_uniform` (grip (2,1,1) moved to (4,1,1) → factors (2,1,1); same with `shift: true` → (2,2,2)); `ctrl_scales_about_the_center` (ToggleCopy then corner grip to (3,3,3) → origin (1,1,1), factors (2,2,2)); `typed_values` (while scaling a corner grip, `"2"` → (2,2,2); on a face grip `"4m"` → factor 2 on that axis; `"1,2,3"` on an edge grip → message "Type 1 or 2 values", nothing run); `zero_is_refused` (moving the face grip onto the fixed face → message "Scale must not be zero", no command); `nothing_selected_click_selects` (empty selection, click on a face → one `Command::Select`).
  - `tool_input_tests.rs`: `scale_a_box_by_its_corner_and_retype` (Review Focus 4) — build a box with `exec` (`add_rectangle` + `push_pull` with a reference or created face), `select all`, `tool(&mut e, "scale")`, `click` at the top corner's model point, `move` along the diagonal, `type "2"` → `ran == ["scale"]`, factors `[2,2,2]`, origin the opposite corner; then `type "3"` → `ran == ["undo", "scale"]` with factors `[3,3,3]`; a fresh `set_tool scale` with nothing in progress and `type "2"` after another tool's command runs nothing and reports a message. Also `scale_about_center_with_ctrl`: `key ctrl` before the grip click → origin is the box center.
- [ ] **Step 2: Run** `cargo test -p tools scale` — expected: compile failure.
- [ ] **Step 3: Implement** ruling 9 and the registrations under Files.
- [ ] **Step 4: Run** the gate — expected: all pass.
- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/scale.rs crates/tools/src/vcb.rs crates/tools/src/select.rs crates/tools/src/lib.rs crates/tools/src/tool.rs crates/app/src/shortcuts.rs crates/app/src/menus.rs crates/app/src/ui/rail.rs crates/app/src/ui/icons.rs crates/app/src/ui/viewport.rs crates/app/src/editor.rs crates/api/src/editor.rs crates/app/src/ui/tests crates/app/src/host/tool_input_tests.rs docs/agent-api.md
git commit -m "Scale tool with box grips"
```

### After the tasks (controller)

- Live check through `app --mcp` with `claude -p` (temporary `--mcp-config`, `--strict-mcp-config`): draw a hexagon, pull it, scale it (2,1,1) and mirror a copy, query the curve kind and render.
- Roadmap row M13a1 → `done <first>..<last>`, memory note, merge `dev` into `main` and push (standing permission), then plan M13a2.
