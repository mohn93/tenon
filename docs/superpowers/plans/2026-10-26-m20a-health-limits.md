# M20a Model Health and Cut List Limits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the limits M19b and M19e left: `make_solid` fills holes that are not flat and mends T-junctions, `model_health` reports closed shells that cross themselves, and cut list sizes of sheared or skewed parts come from the part's smallest box.

**Architecture:** A kernel `shellcross.rs` finds faces of one closed shell that cross (triangles tested through a uniform grid, shared vertices and edges excluded); `health::check` lists them as `self_crossing` and gives the part a reason. `health::repair` gains two steps: mending T-junctions (open edges split at open vertices lying on them, before the rest) and a patch for holes that are not flat (ear clipping over the loop projected on its own best-fit plane). `kernel::cutlist` takes a part's size from the smallest of the boxes aligned to its faces' normals and sides, measured on the placed points.

**Tech Stack:** Rust 2024. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M20a. Builds on M19b and M19e.

## Global Constraints

- No new dependencies; golden images unchanged; `docs/agent-api.md` and the roadmap row updated; no undo-step or report-shape change beyond the new fields named below.
- Gate per task (project rules). No emoji. No trailers. Never `git add -A`.

## Spec notes (rulings)

1. **Self-crossing.** Only closed shells are tested (every side of every face used by exactly two faces). Two faces of one shell cross when triangles of them intersect beyond what they share: triangles sharing no vertex cross when their intersection is longer than `LENGTH_TOL`; sharing a vertex, when it runs beyond that vertex; sharing an edge, beyond the edge; coplanar triangles cross when they overlap in area. Touching at a point or along a shared edge is not crossing. Faces of different shells (two solids in one mesh) are not compared. A part with crossing faces is not a solid: `reason` is "its faces cross each other", and `model_health` lists a `self_crossing` problem (`ids`: the faces involved, in id order, text "4 faces cross each other"). `make_solid` cannot fix it (the part is left, reported with the problem).
2. **Performance.** Triangles go in a grid with a level for every power of two, each in the level just larger than its box (at most eight cells); a triangle looks in its own level and the coarser ones, and a pair is tested in one cell only (the cell holding the low corner of the boxes' overlap). (Review: a uniform grid sized from the mean extent put a dense small part beside large faces in one cell.)
3. **T-junctions.** Only vertices on open edges are candidates and only open edges (one face) are split. A vertex strictly inside an open edge's ends, within the weld tolerance of its line, and not already on the loop of the face using the edge, splits it: the face's loop gains the vertices in order along the edge, new edges join them, the long edge goes. Done after welding and before everything else; counted as `tjunctions_mended` (edges split) in the report. A face that stops being flat or valid because of the split fails the part as any touched face does.
4. **Holes that are not flat.** A loop of open sides that is not flat (and has area) is patched with triangles: the ring projected on the plane through its centroid with the loop's area normal, which must be simple (no sides crossing), ear clipped (ears of no area are refused, a loop that cannot be clipped is refused: "cannot be triangulated"). Each triangle is a face with new edges for the diagonals. `holes_filled` counts holes, not triangles. A patch is refused (the part left) when any of its triangles is pierced by an edge or, after filling, crosses another face of its shell ("a patch over the hole at e12 would cross the part"); the all-or-nothing rule of M19b holds.
5. **Cut list sizes.** The placed shown points of the part (linear part of the composed placement applied) are measured in several orthonormal frames: the placement's own axes (Gram-Schmidt of its columns, which is the old answer when the placement has no shear), and for each shown face (largest areas first) its normal with each of its sides in the plane (not the facets of a smooth surface, faces with a soft edge, nor the sides of circles and arcs, so a dowel reads its diameter). The frame with the least box volume wins, then the least box area (flat parts); the own-axes frame wins ties, so ordinary boards read as before. The number of frames is capped so frames times points stays under two million. The sizes go longest to shortest.
6. **Report.** `make_solid` entries gain `tjunctions_mended` (a count, 0 when none; fixed entries only, as the other counts). `model_health` gains the `self_crossing` kind.

## Review Focus

1. No quadratic pass: crossing detection, T-junction search, patch containment.
2. Crossing tolerance: valid shells (spheres, tubes, hollow boxes, voids, nested solids) never reported.
3. All-or-nothing: a refused patch or crossing leaves the definition untouched.

---

### Task 1: Self-crossing shells

**Files:** `crates/kernel/src/shellcross.rs` (new), `crates/kernel/src/lib.rs`, `crates/kernel/src/health.rs`, `crates/session/src/query.rs`, `crates/session/src/document/health.rs`, `docs/agent-api.md`; tests (a valid cube, sphere, hollow box and tube not reported; a box with one end pushed through the opposite face; a folded-over coplanar face; two solids overlapping in one mesh not reported; a large sphere checked quickly; `model_health` lists `self_crossing` and `make_solid` leaves it).

### Task 2: T-junctions

**Files:** `crates/kernel/src/health.rs`, `crates/session/src/command.rs`, `crates/session/src/document/health.rs`, `crates/api/src/dispatch.rs`, `docs/agent-api.md`; tests (a box face split by a vertex on the neighbour's long edge is mended and a solid; two vertices on one edge; a vertex off the edge by more than the tolerance is not; undo is one step).

### Task 3: Holes that are not flat

**Files:** `crates/kernel/src/health.rs`; tests (a cube missing two triangles meeting across an edge, the old "not flat" test, is now filled; a bent hole of a non-convex ring; a patch that would cross the part is refused; a self-crossing ring is refused; a large bent hole is patched quickly).

### Task 4: Cut list smallest box

**Files:** `crates/kernel/src/cutlist.rs`, `crates/app/src/viewer/parts.rs` (uses `own_size`); tests (a sheared plank reads its true sides; a board modelled turned 30 degrees in its own definition reads its true size; an ordinary board and a scaled instance read as before; a flat part; the many-points cap).

### Task 5: Docs, client, roadmap

**Files:** `docs/agent-api.md`, `docs/superpowers/roadmap.md`, `clients/python/tests/test_model.py` (if a shape test fits).
