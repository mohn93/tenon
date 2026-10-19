# Ported: openskp's SketchUp 2017 writer

This crate is a Rust port of the `.skp` writer in openskp, with the changes listed below.

- **Upstream:** https://github.com/iamahsanmehmood/openskp
- **Commit:** `13ae161fa0108647a04ad5730776120bb6fe8c83`
- **Licence:** MIT, copyright (c) 2024-2026 Ahsan Mehmood; the notice is in the
  repository's `THIRD_PARTY_NOTICES.md`.

## Files ported

| Upstream | Here |
|---|---|
| `packages/python/src/openskp/create.py`: `_ArchiveWriter` (slots, class declarations and references, back-references, persistent ids, preambles, drawbase, strings, and the vertex, edge, face, loop, edge-use, face-texture-coords, material, layer, definition, thumbnail, instance, group, construction line and point records) | `src/archive.rs` |
| `create.py`: `SkpBuilder.__init__` and `to_bytes` (the splice), `_shift_ref`, `_plane_from_polygon`, `_solve_uv_matrix`, `_solve3x3`, `_scale_pins`, `_detect_image_subtype` | `src/write.rs` |
| `packages/python/src/openskp/_face_groups.py`: `face_uv_basis`, `VERTICAL_TOLERANCE` | `src/write.rs` (`uv_basis`) |
| `packages/cpp/src/create.cpp`: the scaffold offsets as constants (lines 58-95), the Iso camera patches and the camera template | `src/write.rs`, `src/archive.rs` |
| `packages/python/src/openskp/_scaffold/blank_v17.skp` (5,796 bytes, sha256 `809a1ab73a20a192ab13aaff197afb1c67d0e9352f6a353a9cd8030919f8a6c3`, the same file as `packages/cpp/tests/fixtures/blank_v17.skp`) | `src/blank_v17.skp`, unchanged |

The persistent ids follow the Python writer, the only port that runs them in one sequence
for the whole file from the scaffold's counter (CHANGELOG, "pids must run in one
sequence"); the C++ and TypeScript ports restart them per section.

## Deviations

1. **Whole document, one pass.** openskp builds a file call by call, one writer per
   section, and shifts each section's slots when it splices them. Here the whole `Doc` is
   known up front, so one slot counter runs through the materials, the scaffold's layer
   section (two slots), the definitions and the root; the shifts it splices with are the
   same numbers. The scaffold's offsets are constants (as the C++ port has them) rather
   than found by walking the scaffold at run time, and the scaffold's hash is pinned in a
   test rather than checked at run time.
2. **Edge flags per edge.** Each `Edge` carries its own soft, smooth and hidden flags.
   openskp applies one set of flags to every edge a face call newly declares.
3. **Loose edges.** An edge no face uses is a top-level `CEdge` with no curve. openskp
   writes free edges only as polylines or arcs, with a curve object.
4. **Edges by index.** A face's edges are found in the caller's `edges` by vertex pair,
   and keep the caller's direction; each edge use's sense follows from it. openskp shares
   vertices and edges by exact coordinates and declares a new edge in the direction the
   face first walks it.
5. **Entity order.** In each entity list: every face, after the edges it is the first to
   use (vertices inline at first use, as openskp does); then the loose edges; then the
   instances; then the guides. Definitions are written in an order where each comes after
   every definition it places, computed by the writer; a definition placed by nothing is
   still written. A definition that places itself is an error.
6. **Empty definitions** (no edges, faces, instances or guides) are not written, and an
   instance of one is an error. openskp raises on closing an empty definition.
7. **Deterministic output.** GUIDs are a 128-bit FNV-1a hash (two streams) of the
   definition's index and content, or of the instance's owner, index and definition GUID,
   shaped as version 4; openskp uses `uuid4()`. Every definition's timestamp is
   2026-01-01T00:00:00Z; openskp uses the current time. The same document gives the same
   bytes.
8. **Plane sign.** A face's fourth plane term is written as `-(n . centroid)`, so that
   `n . p + d = 0` on the face. That is what SketchUp's own files hold, measured on
   `crates/skp/tests/fixtures/legacy/cupboard_v2017.skp` (every face: `d = -n . p`);
   openskp writes `+n . centroid`. The normal is Newell's over the outer loop, as openskp
   computes it.
9. **Textured materials.** The average colour is the material's own red, green and blue
   with openskp's alpha 254 (openskp writes 255, 255, 255, 254), so the colour reads back.
   The stored file name is the material's name with the image's extension, cut to fit
   (openskp stores the source file's path).
10. **Opacity.** A material with opacity 1 is written in openskp's "no opacity" form
    (stored 1.0, flag 0); below 1 as `1 - opacity` with the flag set, as openskp writes an
    explicit opacity. Negative opacities are written as 0.
11. **Instance names.** A component instance is written with an empty name, as SketchUp
    stores a placement nobody renamed; a group with its definition's name. openskp names
    both after the definition unless told otherwise.
12. **Checks.** Faces are not tested for planarity (the callers' faces are planar by
    construction) and are never triangulated; holes are not tested against the outer
    loop's plane. Invalid input is a `WriteError` and never a panic: a short or missing
    loop, a vertex, definition or material index out of range, a face edge missing from
    `edges`, a recursive or empty definition, a texture neither PNG nor JPEG, a string over
    254 UTF-16 units, a NaN or infinity, a degenerate face, edge, guide line or set of
    texture pins, an empty root, and more materials than a drawbase's `u16` material
    reference can name (`TooManyMaterials`, a check openskp does not make).
13. **Guides.** Guide lines are always infinite (start and end at -1e30 and 1e30) through
    the given point along the normalized direction; openskp also writes bounded segments.
14. **Texture basis.** Pinned textures are solved in openskp's latest `face_uv_basis`
    (vertical tolerance 1e-3; a face looking down takes -X and +Y). The project's reader
    (`skp::geom::uv_axes`) uses 1e-9 and takes +X and -Y for a face looking down, so the
    two disagree on downward horizontal faces and on normals within 1e-3 of vertical; the
    round-trip tests pin faces both agree on.
15. **Not ported:** layers other than Layer0, attribute dictionaries, curves (`CArcCurve`,
    `CCurve`), images, dimensions, text, section planes, hidden faces and instances,
    component behaviours, opening and editing an existing file.
