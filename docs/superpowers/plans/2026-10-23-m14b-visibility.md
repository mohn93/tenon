# M14b Visibility and Face Orientation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SketchUp's Hide and Unhide (selected, last, all) for edges, faces, groups and components and guides; View > Hidden Geometry (hidden entities drawn ghosted and selectable); X-ray (see-through faces) as a view setting saved with the camera and scenes; Reverse Faces and Orient Faces. Saved in `.skr`, with commands and queries for agents, and in the window.

**Architecture:** A persistent `hidden: bool` on kernel `Edge`, `Face`, `Instance` and both guide kinds (serde default, written only when set), changed through `ChangeSet` like `soft`, so hiding is one undo step. Every walk that decides what is shown (render scene build, picking, box select, inference, shown bounds, line drawing and print, triangles, intersect, the phone viewer's parts) treats a hidden entity like one on a hidden tag, except that View > Hidden Geometry draws hidden entities ghosted and lets them be picked. Hidden Geometry and X-ray are two new `PageView` flags following `color_by_tag` exactly (document state, camera query, scenes, `.skr`, renderer setter). X-ray and ghosting reuse the renderer's glass path (per-side alpha, sorted, no depth write). Reverse Faces uses the existing `Face::reversed()`; Orient Faces is a new kernel walk across edges shared by exactly two faces (the side-keyed map pattern of `push_pull.rs` `in_solid`).

**Tech Stack:** Rust 2024, wgpu 30, egui 0.36.2. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M14b.

## Global Constraints

- No new dependencies. Existing behaviour unchanged when nothing is hidden and both new view flags are off; golden images unchanged.
- `.skr` schema 14: written only when some entity is hidden or a camera or scene view has `x_ray` or `hidden_geometry` on (the writer's lowest-sufficient-version ladder in `crates/session/src/file.rs`); older files open unchanged; schema 13 files written exactly as before.
- New command variants get arms in `Document::execute`/`edit`, the undoable or camera command lists, and `session::batch::id_slots` (no `_` arm); api dispatch and `docs/agent-api.md` updated in the task that adds them.
- Gate per task (project rules in the SDD rules files): native gate + `cargo check --target wasm32-unknown-unknown -p kernel -p session -p tools -p render -p api -p app --lib -p web`; `scripts/build-web.sh` in Task 5.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Hidden flag:** `hidden: bool` on `Edge`, `Face`, `Instance`, `Guide::Line`, `Guide::Point` (and their `data.rs` forms). Hiding a face hides only the face (its edges stay); hiding an edge never changes faces (a hidden edge still bounds them); hiding an instance hides everything placed through it. Hidden state is independent of tags: shown means not hidden and on a shown tag. Copy, paste, clip, library items and `.skr` keep it; component instances each have their own flag; geometry inside a definition is shared by every instance.
2. **Commands:** `hide { ids }` (entities of the open context: edges, faces, instances, guides; already hidden ones ignored); `unhide { ids }` (selected), `unhide_last {}` (the entities hidden by the most recent `hide` of this session that are still hidden; document state, not saved, cleared on open/new), `unhide_all {}` (every hidden entity of the open context, not inside its instances). Each is one undo step labelled by its name; doing nothing is a success with no undo step. Outcome `changed: [ids]`. Selection: hiding removes the hidden entities from the selection unless Hidden Geometry is on.
3. **Queries:** entity queries and the selection info report `hidden`; `outliner` rows' `hidden` is true for a hidden instance or one on a hidden tag (existing semantics extended), plus a new `own_hidden` for the instance's own flag.
4. **What hidden means (Hidden Geometry off):** not drawn, not picked, not in box select, no inference snaps, not in shown bounds or Zoom Extents, not in the hidden-line drawing, SVG, PDF or print, not in intersect sources or targets, not a part in the phone viewer. Mesh exports (`.obj`, `.stl`, `.gltf`) leave hidden entities out wherever they leave out hidden tags today (check each exporter; if one exports hidden-tag geometry, do the same for hidden entities and document it). `.skp` export writes the hidden flag where the writer has the field (edges, faces, instances), else skips the entity as for hidden tags (documented). `.skp` import: the HIDDEN edge flag becomes `hidden` (it is soft today; soft and smooth stay soft), the face hidden bit and hidden instances become `hidden` where the reader exposes them, hidden guides are imported hidden instead of skipped.
5. **Hidden Geometry (`hidden_geometry`, View menu toggle):** hidden entities (own flag only; hidden tags stay hidden as in SketchUp) are drawn ghosted: faces as glass at alpha 0.25 in their material colour, edges in a light grey; they can be picked and box-selected so they can be unhidden; inference still ignores them. Saved with the camera and scenes (`PageView.hidden_geometry`), command `set_hidden_geometry { on }` (camera command, no undo step, does not mark the document modified, like `set_color_by_tag`), `camera` query field.
6. **X-ray (`x_ray`):** every shown face is drawn as glass at alpha 0.5 (both sides, existing translucent faces keep their lower alpha), edges behind faces visible; picking unchanged. `PageView.x_ray`, command `set_x_ray { on }`, `camera` query field, scenes. Print, PDF, SVG and the line drawing ignore both flags (documented).
7. **Reverse Faces:** `reverse_faces { ids }` reverses each face in ids (faces of the open context; non-faces ignored) with `Face::reversed()` (ids kept, each side keeps its material and texture look). One undo step; outcome `changed`.
8. **Orient Faces:** `orient_faces { id, ids }`: from face `id`, walk faces connected across edges used by exactly two faces (edges with three or more faces stop the walk), within `ids` when non-empty (else all connected faces), flipping each neighbour so the two faces run their shared edge in opposite directions. Breadth-first from `id`, deterministic; a face reached with a conflict (non-orientable surface) keeps the first orientation given. One undo step; outcome `orient: { flipped: [ids], conflicts: n }`; nothing to flip is success with no undo step.
9. **Window:** Edit > Hide; Edit > Unhide > Selected / Last / All; View > Hidden Geometry and View > X-ray (checked toggles); Entity Info shows a Hidden checkbox for the selection (mixed state as for Soft); the Outliner greys hidden instances and its context or row toggle hides/unhides; the viewport right-click menu (M14a `ViewportMenu`) gains Hide, Unhide (when the selection has hidden entities), Reverse Faces and Orient Faces (when it has faces). None of it in the phone viewer (read-only), which still honours hidden entities.

## Review Focus

1. Nothing hidden is drawn, picked, snapped to, counted in bounds or printed when Hidden Geometry is off, at any instance depth (hidden instance, hidden face inside a definition, hidden edge at the root) and through the phone viewer.
2. Hide, unhide (each form) and undo restore exactly; unhide_last after undo/redo and across contexts behaves sensibly; selection updates.
3. `.skr` round trip; schema 14 only when needed; schema 13 output byte-identical when nothing is hidden and flags are off; `.skp` import mapping (hidden edges no longer soft).
4. Orient Faces on a box with one flipped face, a box with several, an open surface, a non-manifold T junction (stops), a Moebius strip (conflict reported, terminates).
5. X-ray and Hidden Geometry rendering with the glass path (sorting, both sides, shadows unaffected?), saved in scenes, and no golden image changes with both off.

---

### Task 1: Hidden flag, hide and unhide commands, file and import

**Files:** `crates/kernel/src/{model.rs, data.rs, (new) hide.rs, lib.rs}`, `crates/session/src/{command.rs, document.rs (+ document/hide.rs new), batch.rs, file.rs, query.rs, skp.rs, (skpwrite mapping)}`, `crates/skpwrite` if it has hidden fields, `crates/api/src/dispatch.rs`, `docs/agent-api.md`; tests in kernel, session and api integration suites.

- [ ] **Step 1: Write the failing tests** for rulings 1-3 and the `.skr`/`.skp` parts of ruling 4 (Review Focus 2, 3).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Hidden flag and hide commands"`.

### Task 2: Hidden entities left out everywhere

**Files:** `crates/kernel/src/{tag.rs (shown checks, shown_bounds fast path), triangles.rs}`, `crates/tools/src/{scenery.rs, inference.rs, probe.rs, select.rs}`, `crates/render/src/scene.rs`, `crates/session/src/{drawing.rs, svg.rs/pdf.rs inputs, document/intersect.rs, export paths}`, `crates/app/src/viewer/parts.rs`; tests per site.

- [ ] **Step 1: Write the failing tests** for ruling 4 (each listed site, nested instances; Review Focus 1).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Hidden entities left out of view, picking and output"`.

### Task 3: Hidden Geometry and X-ray view settings

**Files:** `crates/kernel/src/page.rs`, `crates/session/src/{camera.rs, command.rs, document.rs, document/scenes.rs, document/files.rs, file.rs, query.rs}`, `crates/render/src/{scene.rs, renderer.rs, snapshot.rs, print.rs}`, `crates/tools` (pick hidden when shown), `crates/api/src/{dispatch.rs, headless.rs}`, `docs/agent-api.md`; tests including a render test comparing glass counts / alpha and a golden test that both-off is unchanged.

- [ ] **Step 1: Write the failing tests** for rulings 5-6 (Review Focus 5).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Hidden Geometry and X-ray"`.

### Task 4: Reverse Faces and Orient Faces

**Files:** `crates/kernel/src/(new) orient.rs, lib.rs`, `crates/session/src/{command.rs, document.rs, batch.rs}`, api dispatch, `docs/agent-api.md`; tests in kernel and session suites.

- [ ] **Step 1: Write the failing tests** for rulings 7-8 (Review Focus 4).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Reverse and Orient Faces"`.

### Task 5: Visibility and orientation in the window

**Files:** `crates/app/src/{menus.rs, native_menu.rs (if needed), ui/mod.rs, ui/right.rs, ui/outliner.rs, ui/dialogs.rs, app.rs, editor.rs}`, `docs/agent-api.md` window table; UI and editor tests.

- [ ] **Step 1: Write the failing tests** for ruling 9 (menu items and toggles, Entity Info checkbox, Outliner, viewport menu entries, phone viewer unaffected, native menu ids stable).
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Hide, X-ray and face orientation in the window"`.

### After the tasks (controller)

- Push `dev`; CI green; roadmap row M14b done; spec bullet; memory; merge and push; next milestone (M14c).
