# M14a Intersect Faces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SketchUp's Intersect Faces: With Model, With Selection and With Context. New edges are made wherever the chosen faces cross (faces inside groups and components included, read through their placements), the new edges go into the open context, and faces of the open context that the new edges cross are split so the pieces can be pushed, pulled or erased; coplanar overlaps and touching edges are handled; one undo step whose outcome lists the added edges and the split faces; Edit menu and context menu items and an agent command.

**Architecture:** A new kernel module `intersect.rs` computes intersection segments between two planar faces (each given as world-space outer loop + holes and plane): for non-parallel planes, the line of the two planes is clipped to each face's polygon (2D, holes respected) and the overlapping intervals are the segments; for coplanar faces, each face's boundary edges are clipped to the other face's area. Candidate face pairs come from the per-mesh BVH (`Index::near_box`) in the local frame of the indexed mesh. The session gathers the source faces for the chosen scope (open-context faces and placed instance faces, each with its transform into root space), computes all segments, maps them into the open context's coordinates, and draws them in one batch through a new public `Model::add_segments` (which uses the existing `draw_segments`, so faces split exactly as a drawn line splits them). Geometry inside groups and components is never modified.

**Tech Stack:** Rust 2024, egui 0.36.2. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M14a.

## Global Constraints

- No new dependencies. Existing behaviour unchanged; golden images unchanged; `.skr` schema unchanged.
- New command variants get arms in `Document::execute`/`edit` and `session::batch::id_slots` (no `_` arm).
- Tolerances: reuse `LENGTH_TOL`, `PLANE_ANGLE_TOL`; segments shorter than `LENGTH_TOL` are dropped; points within `LENGTH_TOL` weld (the existing `draw_segments` behaviour).
- Gate per task (project rules in the SDD rules files): `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --lib --bins --tests`; Windows clippy and the wasm check run in CI (run `cargo check --target wasm32-unknown-unknown -p web` too).
- Docs (`docs/agent-api.md`) updated in the task that changes behaviour. No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Scopes** (as SketchUp): **With Model**: the selected faces (and faces of selected groups/components, through their placements) are intersected with every shown face of the model (root and all placed instances, any depth, hidden tags and hidden instances excluded, section cuts ignored). **With Selection**: the selected faces/instances are intersected only with each other. **With Context**: the selected faces/instances are intersected with the shown faces of the open context's own mesh only (not with other groups). An empty selection is `invalid_input` ("Select the faces or groups to intersect").
2. **Where edges go:** every new edge goes into the open context's mesh, in its coordinates (mapped through the inverse of the context transform); faces of the open context crossed by them are split (draw_segments rules). Faces inside groups and components are never modified, even when they are the ones crossed. A face is never intersected with itself or with faces that share its mesh and plane and touch only along existing edges (no new edges where they already exist).
3. **Coplanar overlaps:** for two coplanar faces (planes equal within `PLANE_ANGLE_TOL`/`LENGTH_TOL`), each face's boundary segments that lie inside the other face's area (including its boundary) are added; touching edges that already exist are no-ops.
4. **Command:** `intersect_faces { ids: [entity id], with: "model" | "selection" | "context" }` (ids default to the current selection when empty), one undo step labelled `intersect_faces`; outcome `intersect: { added_edges: [ids], split_faces: [ids of faces that existed before and were split], created_faces: [new face ids] }`; nothing to intersect (no crossings) is a success with an empty report and no undo step, and the window says "No faces crossed".
5. **Window:** Edit > Intersect Faces > With Model / With Selection / With Context (enabled with a selection). The viewport right-click menu (M13d1 has one for a section plane) shows Intersect Faces > With Model / With Selection / With Context when the selection has faces or groups. Status line after: "Added N edges, split M faces" or "No faces crossed".
6. **Limits:** at most 200,000 face pairs tested after the broad phase and 100,000 new segments; beyond that `invalid_input` ("Too many faces cross to intersect at once"); a pathological input never hangs.

## Review Focus

1. Two boxes overlapping at the root, Intersect With Model on one: both boxes are split along their crossing lines; erasing the overlapping piece leaves a clean union-like shell (all faces valid, `validate()` passes).
2. A box crossing a group's box: new edges appear in the open context only; the group's mesh is byte-for-byte unchanged; inside an open group, edges go into that group in its coordinates (rotated, scaled, mirrored placements).
3. Coplanar overlap: two overlapping rectangles in one plane give the shared region's boundary; touching boxes (sharing a face plane but not overlapping) add nothing.
4. Faces with holes: a line through a hole is split so no edge crosses the hole.
5. Undo restores the model exactly; the outcome ids are correct and usable as batch references.

---

### Task 1: Face-face intersection in the kernel

**Files:** `crates/kernel/src/{intersect.rs (new), edit.rs (pub add_segments), lib.rs}`; tests `crates/kernel/tests/integration/intersect.rs` and an `#[ignore]` perf test in `perf.rs`.

**Interfaces (produces):** `kernel::intersect::{FaceShape { outer: Vec<DVec3>, holes: Vec<Vec<DVec3>>, plane: Plane }, face_shape(model: &Model, face: FaceId, to_world: DAffine3) -> Option<FaceShape>, face_segments(a: &FaceShape, b: &FaceShape) -> Vec<(DVec3, DVec3)>}`, plus `Model::add_segments(&mut self, segments: &[(DVec3, DVec3)]) -> Result<ChangeSet, KernelError>` (validates and draws in one batch).

- [ ] **Step 1: Write the failing tests** for `face_segments` (crossing quads; T-shaped; parallel planes -> none; coplanar overlap; coplanar touch -> none; holes; faces meeting along an existing edge -> none; tiny segments dropped; mirrored/scaled transforms through face_shape) and `add_segments` splitting faces (a chord across a face splits it; validate passes).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Face intersections in the kernel"`.

### Task 2: The intersect_faces command

**Files:** `crates/session/src/{command.rs, document.rs (+ document/intersect.rs new), batch.rs}`, `crates/api/{src/dispatch.rs description, tests/integration/dispatch.rs}`, `docs/agent-api.md`; tests `crates/session/tests/integration/intersect.rs`.

- [ ] **Step 1: Write the failing tests** for rulings 1-4 and 6, and Review Focus 1-5.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Intersect faces command"`.

### Task 3: Edit menu, context menu and status

**Files:** `crates/app/src/{menus.rs, ui/mod.rs, ui/dialogs.rs (context menu generalized), app.rs, editor.rs (+ editor/intersect.rs new)}`, `docs/agent-api.md` (window table); UI tests in `crates/app/src/ui/tests/`, editor tests.

- [ ] **Step 1: Write the failing tests** for ruling 5 (menu items and enabled states, the right-click menu for a face/group selection and the existing section-plane menu still working, status messages, native menu ids stable).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Intersect Faces in the window"`.

### After the tasks (controller)

- Push `dev`; CI green; roadmap row M14a done; spec bullet; memory; merge and push; next milestone (M14b).
