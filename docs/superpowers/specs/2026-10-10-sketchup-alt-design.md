# SketchUp Alternative — Design

Date: 2026-10-10
Status: Draft for review

## Goal

A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.

## Non-goals (v1)

- Precise B-rep/NURBS solid modeling, booleans, fillets.
- True curve geometry (arcs/circles are edge chains tagged as curves).
- Texture mapping beyond basic projection.
- `.skp` import/export (closed SDK).
- Profile/silhouette edge rendering.

## Stack

| Concern | Choice |
|---|---|
| Language | Rust (stable) |
| Windowing / input | winit |
| GPU | wgpu (Metal / Vulkan / DX12) |
| UI | egui (egui-wgpu, egui-winit) |
| Math | glam, f64 (`DVec3`) for model geometry, f32 on the GPU |
| Robust predicates | `robust` |
| Triangulation | `earcutr` (or `spade` if constrained triangulation is needed) |
| Stable IDs | `slotmap` |
| Picking acceleration | BVH (`bvh` crate or in-house) |
| Serialization | `serde` + JSON/`bincode`, zip container |
| Interop | `gltf`, OBJ/STL (in-house or small crates) |
| Native dialogs | `rfd` |

## Workspace layout

```
crates/
  kernel/   geometry only; no GPU/UI deps
  tools/    tool state machines, inference engine, VCB parsing
  render/   wgpu renderer
  app/      winit + egui shell, wiring, file IO
```

Dependency direction: `app -> {tools, render} -> kernel`. `tools` and `render` do not depend on each other.

## 1. Kernel (`kernel`)

### Data model: half-edge mesh

- **Vertex**: `DVec3` position, one outgoing half-edge.
- **HalfEdge**: twin, next, prev, origin vertex, face (optional).
- **Edge**: pair of twin half-edges; flags: soft, smooth, hidden.
- **Face**: outer loop + inner loops (holes); cached plane (normal, d); front/back material.
- All elements stored in `slotmap`s with stable keys that survive edits.

### Sticky geometry (kernel responsibility)

- `add_edge` splits crossed edges, merges with existing vertices/edges within tolerance, merges collinear overlaps.
- A closed coplanar loop produces a face automatically.
- An edge across a face splits the face.
- Geometric tolerances are explicit constants; orientation tests use robust predicates.

### Groups and components

- A `Definition` owns its own mesh. An `Instance` references a definition with a transform.
- Model = root mesh + tree of instances. Geometry in different meshes never merges.
- Groups are single-use definitions; components are shared.

### Operations API

`add_edge`, `push_pull(face, distance)`, `move_vertices`, `erase`, `split_face`, `make_group`, etc. Each returns a `ChangeSet` (created / deleted / modified element keys plus data needed to invert). ChangeSets drive undo and render-cache invalidation.

### Testing

Pure unit tests per operation plus a `validate()` invariant check run after every operation in tests: twin symmetry, closed loops, face planarity, no dangling references.

## 2. Tools and inference (`tools`)

### Tool contract

```rust
trait Tool {
    fn on_event(&mut self, ev: &ToolEvent, ctx: &ToolContext) -> ToolResponse;
}
// ToolEvent: pointer move/down/up (with InferenceResult), key, VCB text
// ToolResponse: Preview(geometry) | Commit(Vec<Op>) | Cancel | Status(text)
```

Tools never mutate the mesh directly. A `Commit` is applied atomically as one undo step.

### Inference engine

- Point snaps: endpoint, midpoint, on-edge, on-face, intersection, origin.
- Direction snaps: red/green/blue axes, parallel/perpendicular to an edge, along the current drawing direction.
- Locks: Shift locks the current inference; arrow keys lock an axis.
- Ranking: points over directions, nearer over farther; tolerance in screen pixels.
- Output: snapped `DVec3`, snap kind, guide geometry, tooltip label.
- Picking via BVH over faces and edges; edges get a pixel pick radius.

### Measurements box (VCB)

Typed input routed to the active tool. Unit parser supports mm, cm, m, in, ft, `5'6"`, comma-separated dimensions (`300,200`), and array syntax (`5x`, `/5`).

### Undo/redo

Stack of ChangeSets grouped per commit.

### v1 tools

Select, Line, Rectangle, Circle, Push/Pull, Move/Copy, Rotate, Erase, Orbit/Pan/Zoom, Tape Measure (guides), Make Group/Component.

### Testing

Headless inference tests (fixed camera and cursor, assert snap kind and point). Tool tests feed scripted event sequences and assert the emitted ops.

## 3. Rendering (`render`)

- Per-definition GPU buffers rebuilt incrementally from ChangeSets; components drawn instanced.
- Pass 1, faces: front/back material colors, camera-attached headlight + ambient, polygon offset to avoid edge z-fighting.
- Pass 2, edges: screen-space quad-expanded lines with constant pixel width; soft/hidden edges skipped.
- Pass 3, overlay: tool previews, inference guides (colored, dashed), snap markers, selection highlight, axes, ground grid; selective depth testing.
- Camera: perspective and orthographic, orbit about the point under the cursor, zoom to cursor, standard views, zoom extents.
- Redraw on demand only (input, edits, camera animation).
- 4x MSAA.

### Testing

Unit tests for buffer building and camera math; a small set of headless golden-image tests.

## 4. App shell and files (`app`)

- winit event loop; egui for toolbar, VCB, entity info, outliner, materials.
- Viewport occupies the egui central area.
- Input routing: egui first if it wants the input, otherwise camera navigation, then the active tool.
- Native format `.skr`: zip with versioned JSON/bincode of definitions, instance tree, materials; schema version field for migrations.
- Export: OBJ, STL, glTF. Import: OBJ, STL.

## Milestones

1. **M1**: workspace, window, empty viewport with orbit/pan/zoom, grid, axes.
2. **M2**: kernel `add_edge` and auto-face creation; Line tool with endpoint and axis inference; undo/redo.
3. **M3**: Rectangle, Push/Pull, VCB.
4. **M4**: Select, Move, Erase; full inference set.
5. **M5**: groups and components; save/load; export.

## Risks

- **Sticky geometry robustness**: edge merging and face detection under floating-point error is the main risk. Mitigation: robust predicates, explicit tolerances, invariant checks in every test, fuzz tests on random edge sequences.
- **Inference feel**: hard to specify; plan for iteration after M2 with manual testing.
- **Line rendering quality**: custom wide lines need care at grazing angles; acceptable fallback is thinner lines.
