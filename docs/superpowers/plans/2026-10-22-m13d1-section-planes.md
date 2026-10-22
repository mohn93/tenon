# M13d1 Section Planes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SketchUp-style section planes: a per-context entity placed with a tool on a face or along an axis, moved, rotated and flipped like other entities, one active per context (the root, or each group/component definition) cutting away the geometry behind it in that context and everything nested in it, with a section fill on closed cuts and thick cut edges; display toggles for planes, cuts and fill; in `.skr`, commands and queries.

**Architecture:** A new per-mesh kernel entity `SectionPlane { point, normal (unit), tag }` (`EntityId::Section`, written `s<n>`) following the M13c annotation template (ids from the mesh counter, ops `AddSection`/`RemoveSection`, tags, transform, copy, erase, group/explode/duplicate, clip). Each `Model` also stores `active_section: Option<SectionId>` changed by an undoable op. Geometry on the negative side of an active plane (opposite its normal) is cut away; a placement is cut by its own mesh's active plane and by the active planes of every ancestor placement (all transformed into that placement). The cut is done on the CPU in the render crate's scene walk (`Walk::mesh`): triangles and edges are clipped by the half-spaces as they are emitted, and the section fill is computed per placement and plane from the clipped closed meshes (face/plane intersection segments chained into loops, nested even-odd, triangulated with earcut through a kernel helper), uploaded as extra triangles with a flat fill colour, plus the loops as always-wide cut edges. No shader or vertex-format change. Picking and inference ignore cut-away geometry by testing hit points against the planes. Exports ignore cuts (as SketchUp) and warn "N section planes were not written".

**Tech Stack:** Rust 2024, egui 0.36.2, wgpu 30. No new dependencies (`earcutr` stays in the kernel; render uses a kernel helper).

**Spec:** `docs/superpowers/roadmap.md` row M13d (split: M13d1 section planes, M13d2 print, PDF and SVG).

## Global Constraints

- No new dependencies; `Cargo.lock` unchanged.
- Existing behaviour unchanged; golden images unchanged for models without section planes; files without section planes or non-default section view settings save at their current schema; schema 13 only when a model has a section plane or non-default section display settings.
- New command variants get arms in `Document::execute`/`edit` and `session::batch::id_slots` (no `_` arm); the new `EntityId` variant is handled in every exhaustive match (no `_` arms added to hide it).
- Gate per task (project rules in the SDD rules files): `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --lib --bins --tests`; Windows clippy runs in CI.
- Docs (`docs/agent-api.md`) updated in the task that changes behaviour. No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Model:** `SectionPlane { point: DVec3, normal: DVec3, tag: Option<TagId> }`, `normal` unit length (refused otherwise), `point` within the coordinate limit. `Model::active_section() -> Option<SectionId>`; activating a plane of another mesh is refused. Erasing the active plane clears the active field in the same change set (undo restores both). Copying keeps copies inactive; grouping moves planes with the geometry and keeps a moved active plane active in the new definition; explode clears the definition's active plane (the parent's stays).
2. **Cut rule:** a point `p` is kept by a plane when `(p - point) · normal >= -1e-9`. A placement is cut by the active plane of its own mesh and of every ancestor mesh on its placement path (each transformed to world); a plane in a group never cuts its parent or siblings. Faces on the plane are kept. Hidden tags hide planes like other entities; a hidden plane still cuts (as SketchUp).
3. **Display settings** (document view settings like `color_by_tag`, saved in `.skr` with the view, not undoable): `section_planes` (draw plane entities, default on), `section_cuts` (apply cuts, default on), `section_fill` (default on). Commands `set_section_display {planes?, cuts?, fill?}`; query `section_display`.
4. **Commands:** `add_section_plane { point: [f64;3], normal: [f64;3], activate?: bool (default true) }` (in the open context); `flip_section_plane { plane }` (negate the normal, same point); `activate_section_plane { plane }`; `deactivate_section { }` (clears the open context's active plane). Move/Rotate/Scale/Erase/Copy/Select/group/explode/clip accept `s` ids. Each edit one undo step. Queries: `section_planes` (planes in the open context with `active`), `entity` info for `s` ids. Exports (OBJ/STL/glTF/.skp) warn "N section planes were not written"; `.skp` import does not read planes (documented).
5. **Drawing:** a plane entity is drawn as a square outline centred on `point` in the plane, side = max(1 m, 1.2 x the diagonal of its context's shown bounds), with four inward arrows on the edges pointing along the normal side; the active plane's outline is drawn with the accent colour, inactive ones in the guide colour; selection and hover like guides. Cut edges: the section loops, drawn wide (2 x the normal wide-edge width) in the edge colour. Fill: the loops of closed cuts filled with a neutral dark grey (theme-independent, `#3a3a3a`) at the plane; open chains (cuts through non-closed meshes) get cut edges but no fill. Cuts off: geometry drawn whole, planes still drawn when `section_planes` is on.
6. **Window:** Section Plane tool (Tools menu and rail; no key): the preview square follows the face under the cursor (normal = the face normal facing the camera) or, with no face, the ground plane; arrow keys lock the normal to red (Right), green (Left) or blue (Up) axis as in other tools; a click places and activates it (`add_section_plane`). Context menu on a selected plane: Flip, Active Cut (toggle), and the window View menu gets Section Planes, Section Cuts and Section Fill checkboxes. Entity Info for a plane shows "Section Plane", active or not, and its point and normal.
7. **Picking and inference:** in each placement, faces, edges and points on the cut-away side of any of its planes are not picked, hovered, box-selected or inferred (test the hit point for faces, the vertex for points, the hit point on the segment for edges). Section planes are picked by their outline. With `section_cuts` off nothing is excluded.

## Review Focus

1. A plane inside an opened group cuts only that group's geometry and its nested instances, not the parent or siblings, and the cut follows the group when it is moved, rotated or scaled (non-uniform included).
2. A box cut through its middle shows a filled square at the cut and wide cut edges; a single face (not closed) cut shows only a cut edge; a cut through a box with a hole shows a fill with a hole.
3. Undo of erasing the active plane restores it active; flip/activate/deactivate undo exactly; copy keeps copies inactive.
4. Save and reopen keep planes, the active plane per context and the display settings; a schema 12 file opens with none.
5. With the root plane active, clicking where only cut-away geometry would be selects nothing behind it and inference snaps only to kept geometry.

---

### Task 1: Section planes in the kernel

**Files:** `crates/kernel/src/{ids.rs, section.rs (new), model.rs, data.rs, scene.rs, group.rs, clip.rs, tag.rs, lib.rs}`, plus a kernel helper `section_loops(triangles_local: &[[DVec3;3]], plane: (DVec3, DVec3)) -> SectionLoops { closed: Vec<Vec<Vec<DVec3>>> /* outer + holes */, open: Vec<Vec<DVec3>> }` and `triangulate_loops` reusing earcutr; minimal arms elsewhere so the workspace compiles; tests `crates/kernel/tests/integration/sections.rs`.

- [ ] **Step 1: Write the failing tests** for ruling 1 (ops/inverses, active field, erase clears active, copy inactive, group/explode, clip, tags, data round trip and validation), and `section_loops` (box -> one square loop; box with a hole -> outer + hole; open face -> open chain; plane through a vertex or along an edge handled without panics).
- [ ] **Step 2: Run** — expected: failures.
- [ ] **Step 3: Implement** following the M13c annotation template.
- [ ] **Step 4: Run** the gate.
- [ ] **Step 5: Commit** `git commit -m "Section planes in the kernel"`.

### Task 2: Commands, queries, display settings and files

**Files:** `crates/session/src/{command.rs, document.rs (+ document/sections.rs new), query.rs, batch.rs, file.rs (schema 13), export.rs, skp_export.rs, camera.rs or wherever ViewLook lives}`, `crates/api/tests/integration/dispatch.rs`, `docs/agent-api.md`; tests `crates/session/tests/integration/sections.rs`.

- [ ] **Step 1: Write the failing tests** for rulings 3-4 and Review Focus 3-4, plus Move/Rotate/Scale/Erase/Copy/Select/group/explode/clip of `s` ids and export warnings.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Section plane commands, display settings and files"`.

### Task 3: Cutting, fill and drawing

**Files:** `crates/render/src/{scene.rs (Walk::mesh clipping, fill, cut edges), sections.rs (new: plane outline shapes), renderer.rs}`, `crates/render/tests/integration/{golden.rs, headless.rs}` + new golden images; `crates/session/src/annotation_shape.rs` or a new `section_shape.rs` for the plane outline shared with tools.

- [ ] **Step 1: Write the failing tests:** clipping of triangles and edges at a plane (unit level), fill and cut edges for Review Focus 1-2 (headless pixel checks and two new golden images `section_cut.png`, `section_group.png`), display toggles, plane outline/selection/hover, golden images unchanged without planes.
- [ ] **Step 2-5:** run, implement rulings 2 and 5, gate, commit `git commit -m "Section cuts, fill and planes drawn"`.

### Task 4: Picking, the Section Plane tool and the window

**Files:** `crates/tools/src/{scenery.rs, inference.rs, probe.rs, select.rs, move_tool.rs, section.rs (new tool), tool.rs, lib.rs}`, `crates/api/src/editor.rs` (`ToolName`), `crates/app/src/{shortcuts.rs, menus.rs, ui/icons.rs, ui/rail.rs, ui/viewport.rs, editor.rs, info.rs, app.rs}`, `docs/agent-api.md`; tests in `crates/tools/tests/scenery.rs`, tools unit tests, app UI and tool_input tests.

- [ ] **Step 1: Write the failing tests** for rulings 6-7 and Review Focus 5 (tool placement on a face and with axis locks via tool_input, Esc, context menu Flip/Active Cut, View menu toggles, Entity Info, picking/inference/box select excluding cut-away geometry, nothing excluded with cuts off).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Section Plane tool and section picking"`.

### After the tasks (controller)

- Push `dev`; CI green. Roadmap: split M13d into M13d1 (done) and M13d2 (todo); spec bullet; memory; merge and push; next: M13d2.
