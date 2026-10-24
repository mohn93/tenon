# M17c Solid Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SketchUp's Solid Tools for agents and users: tell whether a group or component is a solid and its volume, and combine solids with Union, Subtract, Intersect, Trim, Split and Outer Shell, so bores, pockets, drilled holes and cutaways can be modelled directly.

**Architecture:** A kernel module `crates/kernel/src/solid.rs`: a closedness test (every edge bounds exactly two faces, consistent orientation, no stray edges or faces), signed volume, and a boolean core. The core brings both solids into one coordinate space, cuts each one's faces along the other's surface (reusing `intersect::face_segments` and the face splitting used by Intersect Faces), classifies each resulting face as inside, outside, or on the other solid (on: coplanar, same or opposite direction) by a ray-parity test from an interior point (robust: several ray directions, majority, skipping rays that graze edges), keeps the faces the operation needs (flipping kept faces of B for Subtract), welds them into one mesh, and builds the result definition. Session commands wrap it with one undo step. Window: Tools > Solid Tools menu items acting on the selection.

**Tech Stack:** Rust 2024, glam. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M17c.

## Global Constraints

- No new dependencies. Existing behaviour unchanged; golden images unchanged.
- New commands: every registration site (execute/edit arms, undoable list, `batch::id_slots`, api, docs) in the task that adds them.
- Gate per task (project rules): native gate + the wasm32 `cargo check` line.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Solid.** A group or component instance is a solid when its definition's own geometry (nested instances ignored: a definition with nested instances or with guides only is not a solid unless its loose geometry alone is closed and it has no nested instances) has at least one face and is closed: every edge bounds exactly two faces, the faces around each edge are consistently oriented (one traverses it each way), and there are no edges bounding no face. Query `solid { id }` returns `{ solid: bool, volume: f64 (m³, in the open context's units, i.e. after the instance transform's scale; absent when not solid), reason?: string (why not, e.g. "edge e12 bounds 1 face") }`. Entity Info shows "Solid group"/"Solid component" and the volume (the existing info panel; a small addition).
2. **Operations** (each one undo step; all instances in the open context; every operand must be a solid else `invalid_input` naming it and the reason):
   - `solid_union { ids }` (2 or more): one new group whose geometry is the union.
   - `solid_subtract { target, tools }` (one or more tools): the target minus each tool, in order; tools are erased.
   - `solid_intersect { ids }` (2 or more): the common part.
   - `solid_trim { target, tools }`: as subtract but the tools stay.
   - `solid_split { a, b }`: three groups: a - b, a ∩ b, b - a (empty pieces are not made).
   - `outer_shell { ids }` (1 or more): the union, with internal faces removed (for one solid: the solid with internal faces/voids removed).
   SketchUp's convention: the result is a group (not a component), placed with the first operand's transform (target for subtract/trim, first id otherwise), with its material, tag, hidden flag and its definition name (a component target's name gets no "#n"; the result is a group named as the definition was). Operands consumed by the operation are erased (union/intersect/outer_shell: all; subtract: target and tools; trim: target only; split: both). Outcome `created`, `deleted`, `placed` (the result instances, in the order above). An empty result (disjoint intersect, a tool that removes everything) is not an error: no group is made, the consumed operands are still erased, and `placed` is empty; the outcome has `empty: true`.
3. **Faces and materials.** Kept faces keep their materials (front and back) and textures as they were in their source, transformed into the result's coordinates; faces from a subtracted tool are flipped and keep their material on the side now facing out (the tool's front material ends up on the front, as SketchUp shows the tool's material on the cut). Faces coplanar with the other solid: kept once for union (same direction), dropped for opposite directions where both solids touch along them (union) and kept as appropriate for subtract/intersect so the result is closed. Edges: soft/smooth flags kept on surviving edges; new cut edges are hard.
4. **Result validity.** The result must be closed (rule 1) when the operands are; if numerical trouble leaves it open, the operation fails with `invalid_input` "the result would not be a solid" and changes nothing (never leave a broken model). Coplanar coincident faces, shared edges and touching vertices are handled; the tests cover: box minus a cylinder through it (a bore), box minus a smaller box from a face (a pocket), two boxes sharing a face (union removes the shared face), two overlapping cylinders at right angles (intersect), a sphere minus a box (curved faces split), a stack of 20 bores in one plate (performance: under 2 s in debug).
5. **Performance.** Candidate face pairs come from bounding-box overlap (the face index), not all pairs; the classification ray test uses the face index too.
6. **Window.** Tools > Solid Tools > Union, Subtract, Intersect, Trim, Split, Outer Shell act on the selection (for subtract/trim the target is the selected instance picked first when the selection keeps pick order, else the one with the lowest id; the menu item's tooltip names the target; the others are the tools); disabled with a tooltip reason when the selection is not suitable. No new tool on the rail.

## Review Focus

1. Closedness test and volume (sign and value) are right, including holes in faces and faces with several loops.
2. The boolean core: classification robustness (rays grazing edges or vertices, coplanar faces), result closed and outward, materials on the right sides, no leftover stray edges.
3. Failure leaves the model untouched (one undo step only on success).
4. Performance with many faces (bounding-box pruning).

---

### Task 1: Solid test, volume and query

**Files:** `crates/kernel/src/{(new) solid.rs, lib.rs}`, `crates/session/src/query.rs`, api query dispatch/schema, `crates/app/src/info.rs` (Entity Info line), `docs/agent-api.md`; tests (kernel unit, session/api integration).

- [ ] **Step 1: Write the failing tests** for rule 1.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Solid test and volume"`.

### Task 2: Boolean core with Union, Subtract and Intersect

**Files:** `crates/kernel/src/{solid.rs or (new) boolean.rs}`, session commands and handler, `docs/agent-api.md`; tests (kernel unit for the core; session integration for the commands).

- [ ] **Step 1: Write the failing tests** for rules 2-5 for union, subtract and intersect.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Solid union, subtract and intersect"`.

### Task 3: Trim, Split and Outer Shell

**Files:** kernel boolean module, session commands, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for the remaining operations of rule 2.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Solid trim, split and outer shell"`.

### Task 4: Solid Tools in the window

**Files:** `crates/app/src/{menus.rs, native_menu.rs, editor.rs or flows.rs}`, tests (menu items, enabled state, a union through the menu).

- [ ] **Step 1: Write the failing tests** for rule 6.
- [ ] **Step 2-5:** run, implement, gate (+ `scripts/build-web.sh`), commit `git commit -m "Solid Tools menu"`.

### After the tasks (controller)

- Final review; push `dev`; CI green; roadmap row M17c done; spec bullet; memory; merge and push; next milestone (M17d).
