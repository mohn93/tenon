# M17b Solid Primitives and Generators Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an agent make a whole part in one command: boxes, cylinders, tubes, cones, prisms, spheres and tori as closed solids in their own group or component (painted and named), plus `revolve`, `add_helix` and `copy_along_path`, so detailed mechanical models need far fewer commands and never fight sticky geometry.

**Architecture:** A new kernel module `crates/kernel/src/primitives.rs` builds each primitive as a closed, outward-facing mesh in its own local frame (a fresh `Model`), with soft edges between curved side faces. A session helper turns a built mesh into a new group definition, a named component, or loose geometry in the open context, places the instance, paints it and names it, in one undo step. `revolve` and `copy_along_path` work in the open context like `follow_me` and `copy` do.

**Tech Stack:** Rust 2024, glam. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M17b. Builds on M17a (`Transform`, outcome `placed`).

## Global Constraints

- No new dependencies. Existing commands unchanged; golden images unchanged.
- New command variants get arms in `Document::execute`/`edit`, the undoable list, `session::batch::id_slots` (no `_` arm), api dispatch and `docs/agent-api.md` in the task that adds them.
- Gate per task (project rules): native gate + the wasm32 `cargo check` line.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Common primitive options.** Every `add_<primitive>` takes `as` (`"group"` default, `"component"`, `"geometry"`), `name` (optional; the definition's name; for a component without one, the primitive's kind capitalised: "Box", "Cylinder", ...; component names follow the existing uniqueness rule, so a taken name gets "Box#2" as `make_component` does, or whatever `make_component` does today), `material` (optional MaterialId; paints the instance for group/component, the faces' front sides for geometry). As group or component the solid is built in a new definition and is never merged with the open context's geometry; one instance is placed and is the `placed`/`created` instance (it becomes the selection, as `make_group` does). As geometry it is added to the open context with the usual merge rules (it may join touching geometry) and the outcome's `created` lists its faces and edges. One undo step. Every primitive is a closed 2-manifold whose faces point outward; the edges between neighbouring curved side faces are soft (as on a pulled circle), cap and prism edges hard. Batch `$N:i0` names the instance.
2. **Local frame and placement.** Curved primitives are built about the local +z axis with the base center at the local origin; the instance transform maps local +z onto `axis` (normalised; zero or non-finite is `invalid_input`) and local +x onto the plane's u axis for that normal (the same u axis `add_circle` uses for its first corner), with the origin at `base` (or `center`). A box's definition has its corner at the local origin and the model axes. So `entities` reports a meaningful transform and `set_transform` / `rotate` behave as expected. As geometry, the same placement is applied to the points.
3. **Shapes and limits** (lengths > 1e-6 m and within the coordinate limit; `segments` 3 to 360, default 24; invalid values are `invalid_input` with the field named):
   - `add_box { corner, size: [sx, sy, sz] }`: all sizes > 0, along the model axes (inside a group, its axes).
   - `add_cylinder { base, axis, radius, height }`, `segments`.
   - `add_tube { base, axis, radius, inner_radius, height }`: 0 < inner_radius < radius; a closed solid with an inner and outer wall and two ring caps.
   - `add_cone { base, axis, radius, top_radius (default 0), height }`: top_radius 0 is a point (triangle sides meeting at the apex, soft between them); > 0 a frustum with a top cap; top_radius may exceed radius.
   - `add_prism { base, axis, radius, sides (3..360, default 6), height, direction? }`: a pulled regular polygon (inscribed radius; first corner along `direction` projected into the plane, default u axis); all edges hard.
   - `add_sphere { center, radius, segments }`: a UV sphere, `segments` around (default 24), segments/2 rings (at least 2), triangles at the poles, quads elsewhere, every edge between side faces soft.
   - `add_torus { center, axis, radius, tube_radius, segments, tube_segments (3..360, default 12) }`: 0 < tube_radius < radius; quads, all edges soft.
   Quads must be planar (they are, by construction, for sphere and torus quads between two rings and two meridians: check this and triangulate any that are not within the kernel's planarity tolerance).
4. **`revolve { face, center, axis, angle (default 360), segments (default 24) }`**: turns face `face` of the open context about the line through `center` along `axis`, sweeping solid faces, as one undo step; `angle` in (0, 360] degrees (counter-clockwise looking down the axis); `segments` is the count for a full turn, scaled by angle/360 and rounded up (at least 1). The face must not cross the axis (touching it along a side or a point is allowed: the swept faces collapse to a point or edge there without zero-area faces); the face plane must contain the axis or the face lies wholly on one side of the axis line (a face whose plane is perpendicular to the axis is `invalid_input`: use push_pull). The face itself is removed when the angle is 360 (as `follow_me` does); for a partial angle it stays as the start cap and a copy closes the end. Edges between consecutive swept faces of one profile side are soft; edges from profile corners are hard unless the profile edge was soft/on a curve, matching `follow_me` conventions. Outcome `created`. Implementing it through the existing `follow_me` machinery with a generated circular path is acceptable if the path is not left in the model and the results meet these rules.
5. **`add_helix { center, axis, radius, pitch, turns, segments (per turn, default 24), start_direction?, left_handed (default false) }`**: draws a helical polyline on one curve (not a polygon), starting at center + radius along start_direction (default u axis), rising `pitch` per turn along axis; turns in (0, 1000], total points <= 10,000 (else `invalid_input`); one undo step; outcome `created` (edges). Follow Me of a small circle along the helix makes a spring; test that it works (`follow_me` with the helix edges as path) and the result is a closed solid if the profile is a closed face.
6. **`copy_along_path { ids, path, count?, spacing?, align (default true) }`**: exactly one of `count` (>= 1) or `spacing` (> 0). `path` is a chain of edges (as `follow_me`'s path: connected, no branches, open or closed). Positions by arc length from the path's start vertex (open: the end of the chain nearest the centroid of `ids`' bounding box; closed: the path vertex nearest it): with `count` N on an open path, N positions evenly from start to end inclusive (N = 1: the start); on a closed path N positions evenly around (k * L / N). With `spacing` s: 0, s, 2s, ... up to the length (inclusive within 1e-9). Copy k is the originals moved by p(t_k) - p(0) and, with `align`, then turned about p(t_k) by the smallest rotation taking the path tangent at 0 to the tangent at t_k (tangent at a vertex: the outgoing edge's direction; at the end of an open path: the incoming edge). A position at t = 0 makes no copy (the originals are there). At most 10,000 copies (`invalid_input` beyond). Copies follow `copy`'s rules; one undo step; `placed` lists instances made.

## Review Focus

1. Every primitive is closed and oriented outward (check with an Euler characteristic / every edge shared by exactly two faces test and the signed volume being positive and near the analytic volume within the facet error); soft edges where ruled; placement transform as ruled.
2. Validation: degenerate radii, zero axis, NaN, segment limits, huge counts; never panic.
3. `revolve` touching the axis creates no zero-area faces and stays manifold; partial angles cap correctly.
4. `copy_along_path` positions and rotations for open and closed paths, `spacing` end inclusion, and the spring (follow_me along a helix) works.

---

### Task 1: Primitive builder and box, cylinder, tube, cone, prism

**Files:** `crates/kernel/src/{(new) primitives.rs, lib.rs}`, `crates/session/src/{command.rs, document.rs or (new) document/primitives.rs, batch.rs}`, `docs/agent-api.md`; tests in kernel (unit, mesh closure and volume) and session integration.

- [ ] **Step 1: Write the failing tests** for rulings 1-3 (these five shapes; as group, component, geometry; material and name; transform in `entities`; soft edges on the curved sides).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Solid primitives: box, cylinder, tube, cone, prism"`.

### Task 2: Sphere and torus

**Files:** `crates/kernel/src/primitives.rs`, session command/handler, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for the sphere and torus parts of ruling 3 (closure, orientation, volume within facet error, all side edges soft, planar quads).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Sphere and torus primitives"`.

### Task 3: Revolve

**Files:** `crates/kernel/src/{sweep.rs or (new) revolve.rs}`, session command/handler, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for ruling 4 (a rectangle beside the axis gives a tube; a profile touching the axis gives a closed solid with no zero-area faces; 90 degree partial with caps; crossing the axis and perpendicular plane refused; undo).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Revolve"`.

### Task 4: Helix and copy along a path

**Files:** session commands/handlers (helix points in a new `crates/session/src/helix.rs` like `circle.rs`), kernel copy helpers if needed, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for rulings 5-6 (helix point count, pitch, handedness; spring via follow_me is closed; copy_along_path count and spacing on open and closed paths with and without align; placed order; limits).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Helix and copy along a path"`.

### After the tasks (controller)

- Final review; push `dev`; CI green; roadmap row M17b done; spec bullet; memory; merge and push; next milestone (M17c).
