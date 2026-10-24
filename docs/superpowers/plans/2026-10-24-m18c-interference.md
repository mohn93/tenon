# M18c Interference and Clearance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Know whether an engine works: which parts collide and by how much, how close parts come, and whether anything hits anything while the crank turns.

**Architecture:** A kernel `interfere.rs`: candidate pairs from world bounding boxes, then for each pair of solid parts an exact overlap test (triangle-triangle intersection; containment by ray parity) and an overlap volume from the M17c boolean core's intersect run on copies (no model change); a minimum-distance routine between two meshes (triangle-triangle distance with a bounding-volume hierarchy per part, built once per definition and reused across poses). A sweep driver steps an M18a/M18b joint through a range on a cloned scene, re-checking the pairs whose bounds come close. Session queries; the window highlights colliding parts.

**Tech Stack:** Rust 2024. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M18c. Depends on M18a (sweep uses joints).

## Global Constraints

- No new dependencies; golden images unchanged; queries only (no model change, no undo step); `docs/agent-api.md`.
- Gate per task (project rules). No emoji. No trailers. Never `git add -A`.

## Spec notes (rulings)

1. **`interference { ids? }`** query (root instances; default all solid root instances, nested instances counted as parts of their root instance): pairs that overlap, each `{ a, b, volume (m³), point (a point inside the overlap) }`, sorted by volume; touching (shared faces or edges, zero volume) is not interference but is listed in `touching` when asked (`touching: true`). Parts that are not solids are listed in `skipped` with the reason. Pairs joined by a joint are still checked (a pin in its bore with zero clearance touches; with negative clearance it interferes).
2. **`clearance { a, b }`** and **`clearance { ids }`** (all pairs among them): the minimum distance between two parts' surfaces with the closest points; 0 when touching; negative is not reported (use `interference`).
3. **`sweep_check { joint, from, to, steps (2..10,000), ids?, slide? }`**: drives the joint through the range on a copy of the scene (couplings and cams included), and for each pair reports the first and last value where it interferes, the largest overlap volume, and the smallest clearance with the joint value where it occurs. Pairs that never come within `report_within` meters (default 0.005) are omitted. Never changes the document. A step the solver cannot meet stops the sweep and reports the value.
4. **Performance.** BVH per definition reused across steps; the V6-sized test (about 60 parts) sweeping 720 steps finishes in under 5 s in release (ignored perf test) and a 36-step version in the normal suite.
5. **Window.** Tools > Check Interference: runs `interference` on the selection (or all), selects the colliding parts and shows a list in a panel or the status line with volumes; clicking a pair selects both and zooms to them.

## Review Focus

1. Exactness of overlap detection (no false positives for touching parts, coplanar faces), overlap volumes against analytic values, containment cases (one part fully inside another).
2. Distance correctness and BVH pruning; sweep reporting the right first and last values.
3. No document change; performance.

---

### Task 1: Interference and clearance queries

**Files:** `crates/kernel/src/(new) interfere.rs`, `crates/kernel/src/(new) bvh.rs`, session queries, `docs/agent-api.md`; tests (overlapping boxes volume; a pin in a bore with clearance 0.1 mm; a part inside another; touching faces; clearance between skew cylinders).

- [ ] **Step 1-5:** tests, implement, gate, commit `git commit -m "Interference and clearance"`.

### Task 2: Sweep check

**Files:** `crates/kernel/src/interfere.rs`, session query, `docs/agent-api.md`; tests (a slider-crank whose piston hits a valve between two known angles; min clearance value and angle).

- [ ] **Step 1-5:** tests, implement, gate, commit `git commit -m "Sweep check"`.

### Task 3: Check Interference in the window

**Files:** app menus, editor, a small results panel; tests.

- [ ] **Step 1-5:** tests, implement, gate (+ build-web), commit `git commit -m "Check Interference"`.

### After the tasks (controller)

- Final review; push; CI; roadmap row M18c done; spec bullet; memory; merge.
