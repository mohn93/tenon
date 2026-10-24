# M18d Mass and Measuring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Weigh and measure a model precisely: material density, the mass, centre of gravity and inertia of solid parts and whole assemblies, and exact distance, angle and round-face queries an agent can use to check an engine's dimensions.

**Architecture:** `Material` gains `density` (kg/m³, optional). A kernel `mass.rs` integrates mass properties of a closed mesh (volume, centroid, inertia tensor by the divergence theorem over triangles), transforms them by instance transforms (including scale and mirroring), and sums them over nested instances. A kernel `measure.rs` gives distances and angles between entities and fits round faces (a set of soft-connected facets of one cylinder, or a circular edge loop) to report centre, axis and radius. Session queries expose both; Entity Info shows mass.

**Tech Stack:** Rust 2024, glam. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M18d.

## Global Constraints

- No new dependencies. Existing behaviour unchanged; golden images unchanged.
- New fields and queries at every registration site; `.skr` schema bump only when a material has a density (this branch: the next free number after dev's current `SCHEMA_VERSION`; the controller renumbers at merge if another branch took it); `docs/agent-api.md`.
- Gate per task (project rules): native gate + the wasm32 `cargo check` line.
- No emoji. Short commit messages, no trailers. Never `git add -A`.

## Spec notes (rulings)

1. **Density.** `add_material`/`change_material` take `density` (kg/m³, 1 to 30,000; `change_material { density: null }` clears it); `materials` query lists it when set; Materials panel number field; `.skr` saves it; `.skp` ignores it. Common values for the docs: steel 7850, cast iron 7200, aluminium 2700, copper 8960, brass 8500, rubber 1200.
2. **`mass_properties { ids }`** (instances of the open context; an empty list means every root instance): for each instance and for the total, `{ volume (m³), mass (kg, absent if any contributing solid has no density), centroid (open-context coordinates), inertia (3x3 about the centroid, kg·m², absent without mass), principal: { moments, axes } }`. A part's density is the instance's own material's, else the material most of its faces show on their front, else none. Nested instances count with their own material (an instance inside a group contributes its own mass; loose geometry of a group must be a solid to count; non-solid loose geometry is reported in `skipped` with the reason and ignored). Mirrored and scaled instances transform volume (|det|), centroid and inertia correctly.
3. **`measure`** query, one of:
   - `{ distance: [a, b] }` between two entities (vertices, edges, faces, instances, guides; in the open context or by instance path `[i1, i2, f3]`): the minimum distance and the two closest points (exact for points, segments and planar polygons; instances: between their meshes, accelerated by bounding boxes);
   - `{ angle: [a, b] }` between two edges (directions) or two faces (normals), in degrees 0-180;
   - `{ round: id }` for a face or edge on a round feature: centre, axis, radius and (for a cylinder) length, fitted from the soft-connected facets or the curve's points (least squares; reports the fit's largest deviation so faceting error is visible).
4. **Entity Info** shows Mass (with units kg) and the centre of gravity for a selected solid with density, using the cached solid result pattern from M17c.

## Review Focus

1. Mass property integration exact for boxes, cylinders (faceted analytic), spheres; parallel-axis and rotation transforms of inertia under instance transforms (incl. mirrors and non-uniform scale).
2. Measure: closest points correct for skew segments, segment-to-polygon including inside and holes; round fits on primitives and boolean results.
3. Validation, units, docs.

---

### Task 1: Density and mass properties

**Files:** `crates/kernel/src/{material.rs, data.rs, (new) mass.rs, lib.rs}`, session commands/queries, Materials panel, Entity Info, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for rulings 1, 2 and 4.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Density and mass properties"`.

### Task 2: Measuring

**Files:** `crates/kernel/src/(new) measure.rs`, session query, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for ruling 3.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Measure distances, angles and round features"`.

### After the tasks (controller)

- Final review; merge into dev; CI; roadmap row M18d done; spec bullet; memory.
