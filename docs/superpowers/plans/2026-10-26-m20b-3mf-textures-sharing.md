# M20b 3MF Textures and Shared Parts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The 3MF export writes base-color textures (Materials and Properties extension) and writes a component placed more than once as one object referenced by each build item or component, instead of copying it per placement.

**Architecture:** `Scene::shared_parts` (kernel, replacing `triangles_by_part`) walks the shown tree and returns objects (own triangles in the object's coordinates plus references to other objects with transforms) and build items. `threemf::encode` welds each object, writes `texture2d`/`texture2dgroup` resources for textured materials (texture coordinates from the same `Triangle::uvs` the OBJ and glTF exports use), the image parts, their content types and the model part's relationships.

**Tech Stack:** Rust 2024, `zip`, `quick-xml` (dev only). No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M20b; 3MF Core 1.3 and Materials and Properties 1.2.1.

## Spec notes (rulings)

1. **Sharing key.** A placement is shared with others of the same definition and the same inherited material (an instance's paint changes the triangles' materials, so differently painted placements are different geometry). A key placed at least twice (counted over the shown tree, selection respected, each key's contents counted once) becomes one object in definition coordinates. Placed once, a part is baked as today (world coordinates, one build item, no transform).
2. **Nesting.** A shared object holds its own faces and the non-shared things inside it baked in; shared definitions inside it are referenced. An object with references is written as a `components` object, its own faces going to a mesh object `<name> (body)` that is its first component; a root part that is baked but holds shared definitions is written the same way.
3. **Mirrored placements.** A placement whose composed transform has a negative determinant is never shared or referenced (a negative-determinant transform turns a mesh inside out in consumers, and the spec asks producers to avoid it). It is baked with the corners swapped as before, so the triangles stay outward facing.
4. **Closed check** stays per mesh object, reported per mesh object; component objects are not reported. `triangles` and `vertices` count what is written, so a shared object counts once. `skipped_faces` counts a shared object's faces once.
5. **Textures.** A textured material's triangles use a `texture2dgroup` (one per material) with `tex2coord` per distinct coordinate; `pid`/`p1..p3` on the triangle. v counts up from the image's bottom (3MF as OBJ), shifted by whole repeats like glTF; tile style `wrap`. Images are written as the stored PNG or JPEG bytes at `/3D/Textures/<n>.png|jpg`, shared when materials share the image; Default content types `image/png` and `image/jpeg`; relationship type `.../2013/01/3dtexture` in `3D/_rels/3dmodel.model.rels`. The extension is not listed in `requiredextensions`, so consumers without it keep the base colors (every textured material still has a base color entry and the object's own `pid`).

## Review Focus

1. Transform order (3MF is row vector, translation in the last row) and mm scaling of translations.
2. Mirrored placements baked, never referenced.
3. Ids resolve, texture parts exist with content types and relationships.

---

### Task 1: Kernel `shared_parts`

**Files:** `crates/kernel/src/triangles.rs`, `crates/session/src/document/files.rs`.

- [ ] Tests: one component twice gives one object, two items; different paint or mirrored placement not shared; nested shared definition referenced.

### Task 2: Encode sharing and textures

**Files:** `crates/session/src/threemf.rs`, `crates/session/src/export.rs` (share `texcoords`).

- [ ] Tests parse the package back: well-formed XML, ids resolve, texture parts, content types, relationships, mirrored winding.

### Task 3: Docs and roadmap

**Files:** `docs/agent-api.md`, `docs/superpowers/roadmap.md`.
