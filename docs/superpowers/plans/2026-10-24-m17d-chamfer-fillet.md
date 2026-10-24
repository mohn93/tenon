# M17d Chamfer and Fillet Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Round or bevel the straight edges of a solid part in one command, so machined parts (blocks, caps, brackets) stop looking like raw boxes.

**Architecture:** Built on the M17c boolean core. For each chosen edge between two planar faces of a solid's definition, a cutter (convex edge) or filler (concave edge) prism is built whose cross-section is the region between the two faces and the chamfer line or fillet arc, running along the edge; cutters are subtracted and fillers added with the boolean core, all in one undo step, in the definition the edges belong to (the open context must be the solid's definition, i.e. the agent opens the group first, or passes the instance: see ruling 1).

**Tech Stack:** Rust 2024. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M17d. Depends on M17c (`solid.rs`, boolean core).

## Global Constraints

- No new dependencies. Existing behaviour unchanged; golden images unchanged.
- New commands: every registration site in the task that adds them; `docs/agent-api.md`.
- Gate per task (project rules): native gate + the wasm32 `cargo check` line.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Commands.** `chamfer_edges { edges, distance }` and `fillet_edges { edges, radius, segments (default 8 per quarter turn, 1 to 90) }`. `edges` are edges of the open context, whose loose geometry must be a solid (M17c rule 1) - so an agent opens the group (`open_instance`) first - or `{ instance, edges }` form: `instance` an instance of the open context and `edges` ids inside its definition (the definition is edited; a component's other instances change too, as editing it does). Each edge must be straight, bound exactly two planar faces, and not be tangent (the angle between the faces' normals at least 1 degree); else `invalid_input` naming the edge. Distance/radius > 1e-6 m and small enough that the cut stays within both faces next to the edge (the cut line must lie inside each face along the whole edge: otherwise `invalid_input` "radius too large for edge eN").
2. **Shape.** Convex edge (the solid's interior angle < 180°): material is removed; a chamfer leaves a flat face at equal setback `distance` on both faces; a fillet leaves an arc of `radius` tangent to both faces, made of `segments * (turn angle / 90°)` (at least 1) faces whose shared edges are soft and smooth, with the edges where the fillet meets the planar faces soft too (the surface reads as one smooth round). Concave edge: material is added the same way (a cove).
3. **Ends and corners.** Along an edge, the cutter runs the edge's whole length and is extended past an end only where that end vertex is shared with another chosen edge (so meeting fillets overlap and leave a closed corner); at an end not shared with another chosen edge, the cutter's end is trimmed to the plane of the face at that end (the third face meeting the vertex) so no neighbouring face is notched. The corner where three filleted edges meet is the cutters' combined result (not a spherical patch); it must be closed. The whole operation is one undo step; if any boolean fails, nothing changes.
4. **Outcome.** `created`/`deleted`/`modified` as for the boolean commands; the faces of each fillet are listed in `created`.

## Review Focus

1. Geometry: setbacks exact, arc tangent to both faces, soft and smooth edges set, concave coves correct.
2. Ends and corners: no notching of neighbouring faces; a box with all 12 edges filleted stays a closed solid with volume between the sharp-corner and spherical-corner analytic values.
3. Validation and all-or-nothing failure.

---

### Task 1: Chamfer

**Files:** `crates/kernel/src/(new) fillet.rs` (cutter profiles, edge analysis), session command, `docs/agent-api.md`; tests (kernel unit and session integration).

- [ ] **Step 1: Write the failing tests** for rules 1, 3, 4 with chamfers (one edge of a box, three edges at a corner, a concave edge of an L-shaped solid, validation).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Chamfer edges"`.

### Task 2: Fillet

**Files:** `crates/kernel/src/fillet.rs`, session command, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for rules 2-4 with fillets (box edge, all 12 edges, concave cove, soft/smooth flags, segments).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Fillet edges"`.

### After the tasks (controller)

- Final review; push `dev`; CI green; roadmap row M17d done; spec bullet; memory; merge and push; next milestone (M17e).
