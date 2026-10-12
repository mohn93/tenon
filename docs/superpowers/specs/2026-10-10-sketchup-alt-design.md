# SketchUp Alternative — Design

Date: 2026-10-10
Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides)

## Goal

A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.

The app is fully controllable by agents: every action a user can take is a serializable command available through a local API, the model can be queried as structured data, and the app can run headless (see section 5).

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
| Stable IDs | monotonic `u64` ids in `BTreeMap`s (undo restores original ids) |
| Picking acceleration | BVH (in-house, in the kernel) |
| Serialization | `serde` + JSON/`bincode`, zip container |
| Interop | `gltf`, OBJ/STL (in-house or small crates) |
| Native dialogs | `rfd` |
| Agent API | JSON-RPC 2.0 (`serde_json`), local Unix socket or stdio |

## Workspace layout

```
crates/
  kernel/   geometry only; no GPU/UI deps
  session/  Document (model, selection, camera, undo), Command/Query types and executor; no GPU/UI deps
  tools/    tool state machines, inference engine, VCB parsing
  render/   wgpu renderer
  api/      JSON-RPC server exposing session commands and queries
  app/      winit + egui shell, wiring, file IO, headless mode
```

Dependency direction: `app -> {api, tools, render} -> session -> kernel`. `tools`, `render` and `api` do not depend on each other. The camera lives in `session` (it is pure math, and `tools` needs it for inference).

## 1. Kernel (`kernel`)

### Data model: edges and vertex-loop faces

SketchUp geometry is non-manifold (an edge can border three or more faces; edges can dangle), so the kernel uses an edge list with faces as vertex loops rather than a strict half-edge mesh.

- **Vertex**: `DVec3` position.
- **Edge**: two vertex ids; flags (soft, smooth, hidden) added when needed.
- **Face**: outer vertex loop (counter-clockwise around the normal) + hole loops (clockwise); unit normal; front/back material (added with materials, M5). Consecutive loop vertices are always joined by an edge.
- Ids are monotonically increasing `u64`s, never reused, stored in `BTreeMap`s, so undo can restore entities under their original ids. Adjacency is found by scanning until model sizes require an index.

### Sticky geometry (kernel responsibility)

- `add_edge` splits crossed edges, merges with existing vertices/edges within tolerance, merges collinear overlaps.
- A closed coplanar loop produces a face automatically.
- An edge across a face splits the face.
- Geometric tolerances are explicit constants; orientation tests use robust predicates.

### Groups and components

- A `Definition` owns its own mesh. An `Instance` references a definition with a transform.
- Model = root mesh + tree of instances. Geometry in different meshes never merges.
- Groups are single-use definitions; components are shared. Copying a group copies its definition; copying a component instance places the same definition. A group's definition goes with its last instance.
- Entity ids are unique across all meshes, so an id alone names an entity anywhere in the model.
- Editing happens in an open context: the root or an instance opened for editing (a path of instances). Commands and queries address the open context's mesh, in its own coordinates.

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
// ToolResponse: Preview(geometry) | Commit(Command) | Cancel | Status(text)
```

Tools never mutate the mesh directly. A `Commit` carries a `session::Command`, executed through the same path as API commands and applied atomically as one undo step. A tool is an interactive way to build a command; the command is always reachable without the tool.

### Inference engine

- Point snaps: endpoint, midpoint, on-edge, on-face, intersection, origin.
- Direction snaps: red/green/blue axes, parallel/perpendicular to an edge, along the current drawing direction.
- Locks: Shift locks the current inference; arrow keys lock an axis.
- Ranking: points over directions, nearer over farther; tolerance in screen pixels.
- Output: snapped `DVec3`, snap kind, guide geometry, tooltip label.
- Picking via BVH over faces and edges; edges get a pixel pick radius.
- Tools work in the open context's coordinates. Inference sees all geometry (inside instances and outside the open instance) and faces anywhere hide what is behind them; clicking geometry inside an instance picks the instance, and geometry outside the open instance is never picked.

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

## 5. Agent control (`session`, `api`)

Design rule for every milestone: the interface is one client of the command layer, and the API is another. No feature may exist only in the interface.

### Commands

- `session::Command` is a serde-serializable enum covering every state change: modeling (add edge, draw rectangle/circle, push/pull face by distance, move/copy/rotate entities, erase, make group/component, explode), guides (add a guide line or point, delete guides), editing context (open/close an instance), selection, camera (set, orbit, zoom extents, standard views), undo/redo, file (new, open, save, import, export).
- Commands take explicit values (points, distances, entity IDs), never screen coordinates or cursor state.
- `Document::execute(Command) -> Result<Outcome, CommandError>`, where `Outcome` carries the net `ChangeSummary` (created/deleted/modified ids as strings). Each modeling command is one undo step; camera commands are not recorded. Errors are structured (`kind` + `message` + offending `ids`), never panics. Raw `ChangeSet`s stay internal.
- Entity IDs are the kernel's stable ids, serialized as strings (`"v12"`, `"e3"`, `"f7"`, `"i4"` for an instance, `"g9"` for a guide); definitions are `"d<n>"`.

### Queries

Read-only, structured: list/get entities (vertices, edges, faces, groups, instances) with geometry, bounding-box and type filters, selection, camera, model stats, undo history. Agents do not need screenshots to know the model state.

### API server (`api`)

- JSON-RPC 2.0 over a Unix domain socket (default, socket file mode 0600, path printed at startup) or stdio (headless). No network listener.
- Methods: `execute` (one command), `batch` (list of commands, optionally atomic as one undo step), `query`, `render_view` (camera + size -> PNG bytes, uses the headless renderer), `subscribe` (stream of ChangeSets), `describe` (command/query catalogue with JSON Schemas, schema version).
- Efficiency: batching, compact responses (IDs and changed entities only unless requested), no polling needed thanks to `subscribe`.
- Commands from the API and from the interface go through one queue on the main thread, so the open window reflects agent edits live.

### Headless mode

`app --headless [--stdio | --socket PATH] [FILE]` runs the session and API without a window; rendering for `render_view` is offscreen.

### MCP

A thin MCP adapter that forwards to the JSON-RPC API is optional and out of scope for v1 milestones.

### Testing

Every milestone adds, for each new feature, at least one test that drives it through `session::Command`/queries without the interface. M2.5 adds end-to-end tests that talk JSON-RPC to a headless instance.

## Milestones

Every milestone's acceptance includes: new capabilities are reachable as `session` commands/queries and covered by a command-driven test.

- **M1**: workspace, window, empty viewport with orbit/pan/zoom, grid, axes.
- **M2**: `session` crate (Document, Command/Query, executor; camera moved here); kernel `add_edge` and auto-face creation; Line tool with endpoint and axis inference; undo/redo.
- **M2.5**: `api` crate: JSON-RPC server (socket + stdio), `execute`/`batch`/`query`/`render_view`/`subscribe`/`describe`; headless mode; end-to-end API tests.
- **M3**: Rectangle, Push/Pull, VCB (lengths and `width,height`; shown in the window title until the egui shell lands in M4).
- **M4a**: egui shell (toolbar, measurements box, entity info); selection in the session; Select tool (click, box); Erase tool and command (with face merging and edge healing).
- **M4b**: Move/Copy (with VCB array syntax); parallel/perpendicular inference; direction locks (Shift, arrow keys).
- **M4c**: push/pull through to openings; push/pull back to flush welds the face and heals the seams; apparent-intersection snaps; BVH picking (a kernel index used by tools and push/pull).
- **M5a**: groups and components in the kernel and session: make group/component, explode, copy/move/erase/select instances, open/close an instance for editing (commands and queries), drawn with the selection and open context shown.
- **M5b**: group and component tools: picking and box-selecting instances, double-click to open and close, Move on instances, inference on geometry inside instances, editing a moved instance with the tools, toolbar and shortcuts for make group/component and explode.
- **M5c**: files: `.skr` save/open/new as commands (zip with versioned JSON keeping every id), opening a file given on the command line, File menu with native dialogs and shortcuts, unsaved-changes prompts; rename and purge definitions.
- **M5d**: export OBJ, STL, glTF as commands and from the File menu (faces as triangles in root coordinates; OBJ and glTF Y-up, STL Z-up).
- **M6a**: circles: an `add_circle` command (center, radius, plane normal, 3-360 sides, default 24) and a Circle tool (center, then rim or typed radius; "24s" sets the side count). `c` selects the Circle tool and Shift+G makes a component. Circles are plain edge chains until M6d tags them as curves.
- **M6b**: rotate: a `rotate` command (vertices, edges, faces and instances about an axis through a center, angle in degrees, exact quarter turns; optional `copies` for radial arrays) and a Rotate tool (`q`: center on a face turns about its normal, else about Z; reference click, then angle click with 15-degree snapping, or a typed angle; Ctrl copies, "5x" and "/5" arrays). Turned placements work everywhere: render normals, picking, box select, inference and the tools' camera inside an open turned instance, bounds, Entity Info (rotation line), explode, files and export.
- **M6c**: guides and the Tape Measure: guide lines (infinite) and guide points are model entities (`g<n>` ids) of the context they are drawn in; `add_guide_line`, `add_guide_point` and `delete_guides` commands, and `erase`, `select`, `move`, `rotate`, `copy`, `make_group` and `explode` take guides; saved in `.skr` (schema version 2; version 1 still opens), never exported. Inference snaps to guide points, onto guide lines, and where guide lines cross edges or each other. Guides are drawn dashed (guide points as small crosses), picked, box-selected and erased like edges, and shown in Entity Info. Tape Measure tool (`t`): from a point the second click adds a guide point, from an edge or guide line a parallel guide line; typed distances, retyped right after to adjust; Ctrl switches to measuring only. Edit > Delete Guides.

## Risks

- **Sticky geometry robustness**: edge merging and face detection under floating-point error is the main risk. Mitigation: robust predicates, explicit tolerances, invariant checks in every test, fuzz tests on random edge sequences.
- **Inference feel**: hard to specify; plan for iteration after M2 with manual testing.
- **Line rendering quality**: custom wide lines need care at grazing angles; acceptable fallback is thinner lines.
- **Interface-only features creeping in**: mitigated by the per-milestone acceptance rule and command-driven tests.
