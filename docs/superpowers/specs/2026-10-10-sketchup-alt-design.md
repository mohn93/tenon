# SketchUp Alternative — Design

Date: 2026-10-10
Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides, M6d curves, M7a camera, M7b materials, M7c outliner, M8 split into M8a-d, M8a import, M8b golden images, M8c clean-up); 2026-10-13 (M8d polish); 2026-10-14 (M9a soften and smooth, M9b textures, M9c lights)

## Goal

A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.

The app is fully controllable by agents: every action a user can take is a serializable command available through a local API, the model can be queried as structured data, and the app can run headless (see section 5).

## Non-goals (v1)

- Precise B-rep/NURBS solid modeling, booleans, fillets.
- True curve geometry (arcs/circles are edge chains tagged as curves).
- Texture mapping beyond basic projection (M9b projects textures in each face's plane; no per-face positioning).
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
- **Edge**: two vertex ids; an optional curve id (M6d: the edges of a circle or arc share one; the curve's shape is worked out from its edges, never stored); a soft flag (M6d: hidden, between faces of one smooth surface); smooth and hidden flags added when needed.
- **Face**: outer vertex loop (counter-clockwise around the normal) + hole loops (clockwise); unit normal; front/back material (M7b: each side an optional material id, none being the default material). Consecutive loop vertices are always joined by an edge.
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

Select, Line, Rectangle, Circle, Arc, Push/Pull, Move/Copy, Rotate, Erase, Orbit/Pan/Zoom, Tape Measure (guides), Make Group/Component.

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

Unit tests for buffer building and camera math; a small set of headless golden-image tests (`crates/render/tests/golden.rs`, see `docs/testing.md`).

## 4. App shell and files (`app`)

- winit event loop; egui for toolbar, VCB, entity info, outliner, materials.
- Viewport occupies the egui central area.
- Input routing: egui first if it wants the input, otherwise camera navigation, then the active tool.
- Native format `.skr`: zip with versioned JSON/bincode of definitions, instance tree, materials; schema version field for migrations.
- Export: OBJ, STL, glTF. Import: OBJ, STL.

## 5. Agent control (`session`, `api`)

Design rule for every milestone: the interface is one client of the command layer, and the API is another. No feature may exist only in the interface.

### Commands

- `session::Command` is a serde-serializable enum covering every state change: modeling (add edge, draw rectangle/circle, push/pull face by distance, move/copy/rotate entities, erase, make group/component, explode), guides (add a guide line or point, delete guides), editing context (open/close an instance, open a path of instances), selection, camera (set, orbit, pan, zoom, zoom extents, standard views, projection), undo/redo, file (new, open, save, import, export).
- Commands take explicit values (points, distances, entity IDs), never screen coordinates or cursor state.
- `Document::execute(Command) -> Result<Outcome, CommandError>`, where `Outcome` carries the net `ChangeSummary` (created/deleted/modified ids as strings). Each modeling command is one undo step; camera commands are not recorded. Errors are structured (`kind` + `message` + offending `ids`), never panics. Raw `ChangeSet`s stay internal.
- Entity IDs are the kernel's stable ids, serialized as strings (`"v12"`, `"e3"`, `"f7"`, `"i4"` for an instance, `"g9"` for a guide); definitions are `"d<n>"`.

### Queries

Read-only, structured: list/get entities (vertices, edges, faces, groups, instances, guides) with geometry, bounding-box and type filters, selection, camera, model stats, undo history, the outliner tree of groups and components. Agents do not need screenshots to know the model state.

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
- **M6d**: curves: edges carry an optional curve id; the edges of a curve always form one unbranched, non-straight chain (sticky splits keep both pieces on the curve, erasing leaves the rest a curve, a piece that falls off gets a new id, a lone edge is plain), and copies, groups and explodes get new curve ids; the shape (circle or arc, center, normal, radius, angle, segments, length) is derived from the edges, so moves and rotations need no upkeep; `add_circle` tags its edges; an `add_arc` command (center, start, signed angle in degrees, plane normal, 2-360 segments, default 12); a `curve` query and `curve`/`soft` fields on edges; `.skr` schema version 3 (versions 1 and 2 still open). Push/pull of a face bounded by a curve gives the cap a curve of its own and makes the swept edges between curve segments soft; a soft edge bounding fewer than two faces hardens. In the window, soft edges are not drawn, picked or snapped to; clicking a curve edge with Select selects the whole curve; Entity Info shows circles and arcs with radius, angle, segments and length. Arc tool (`a`, SketchUp's 2-point arc: start, end, bulge; typed chord and bulge lengths, "12s" segments). Smooth shading across soft edges is deferred.
- **M7a**: camera: commands `orbit` (yaw and pitch in degrees about the target), `pan` (meters along the screen), `zoom` (factor), `zoom_extents` (the whole model in root coordinates, guides excluded; an empty model leaves the camera), `standard_view` (top, bottom, front, back, left, right, iso; same target and distance) and `set_projection` (perspective or parallel; the view height at the target is kept, so a parallel view's zoom is the eye's distance); `set_camera` takes `up` (the screen's up in views straight down or up, which are now allowed) and `projection`; the `camera` query reports `up`, `projection` and `view_height`; camera commands are not undo steps and do not change the revision; `.skr` schema version 4 saves `up` and `projection` (versions 1-3 open in perspective). A parallel view sees 100 eye-to-target distances either side of the target, behind the eye too: rays start there, and picking, inference, overlay sizes (snap markers, guide crosses, pick radius, protractor), guide line spans, render depth offsets and shading work in both projections; `render_view` takes `up` and `projection`. Camera menu: Zoom Extents (Shift+Z), the standard views, Perspective and Parallel Projection; camera moves from the menu keep the tool's operation going. Erase Selection moves from the toolbar to the Edit menu so the toolbar keeps fitting a 1280-point window.
- **M7b**: materials: named sRGB colors (`m<n>` ids from the entity id counter) belonging to the whole model; each face side and each instance carries an optional material (none is the default material); an instance's material shows on face sides inside it without one (the nearest painted instance wins), and exploding bakes it onto them. Commands `add_material` (optional name, default "Material", "Material 2", ...; names distinct, not blank, at most 256 characters), `change_material` (name and/or color), `delete_material` (unpaints everything painted with it, one undo step) and `paint` (faces and instances of the open context, `side` front, back or both, null for the default); a `materials` query (with face and instance use counts); `front_material`/`back_material` on faces and `material` on instances in queries; `changes.materials` lists materials a command added, removed or changed. Paint follows geometry: split pieces keep it, merges keep the containing face's, copies/groups/components keep it, push/pull gives the cap and sides the pushed face's materials (swapped for a sheet pushed against its normal, so the side facing the pull ends up outside), a face turned over keeps each material on its side. `.skr` schema version 5 (versions 1-4 open unpainted); files painting with missing materials or with bad material names are refused. Rendering shows each side's color (selection tint on top); export writes OBJ with a `.mtl` beside it, glTF with one material per color (front side only; STL has no colors). Paint tool (`b`, last toolbar button: click a face to paint the side facing the viewer with the current material, a click on something selected paints the whole selection's faces and instances, an instance paints the instance; Cmd-click or Ctrl-click (either key on every system; Alt is the orbit modifier) picks up the clicked side's material) and a Materials list in the right panel under Entity Info (Default plus every material with a swatch; clicking one makes it current and switches to Paint; Add, Delete, a name field renaming on Enter, and a 12-color palette to recolor); Entity Info shows a face's front and back materials and a painted instance's. Opacity, textures and a free color picker are deferred.
- **M7c**: outliner: an `outliner` query (the tree of placed groups and components, the root's instances down, each level in id order: `id`, `definition`, `kind`, `name` (the definition's; empty for an unnamed group), `material`, `children`; `open` marks the nodes on the open context's path and `selected` the selected instances of the open context; a component's contents appear under each of its instances with the same ids, so a node is named by its path from the root) and an `open_path` command (makes a path of instances from the root the open context from wherever it is, `[]` being the root; clears the selection unless the path is already open; not an undo step; non-instances `invalid_input`, misplaced instances `not_found`). In the window, an Outliner section in the right panel under Materials: a Model row (click: back to the root) and the tree with expand and collapse (nodes on the open path start expanded), groups shown by name or as "Group", components as "<Name>", selected rows highlighted and open ones bold. A click selects the node, first opening the context it is in (as SketchUp does); Shift-click or Cmd/Ctrl-click adds it to or removes it from the selection when it is in the open context; a double-click opens it for editing. Instance names, a filter box, sorting by name and drag-to-reparent are deferred.

- **M8a**: import: an `import` command (path; `format` obj or stl, from the extension; `as` group (default), component or context; `scale` meters per file unit, default 1; `up` y or z, default y for OBJ and z for STL, so export then import gives back the same geometry; `merge_coplanar`, default true) adding the file's faces to the open context as one undo step. A group or component is named after the file, placed at the context's origin and selected; `context` makes the group and explodes it, so the faces merge with what they touch (at most 2,000 faces). Faces are built straight from the polygons, not drawn: corners within 1e-6 m weld, a polygon that is bent or crosses itself becomes triangles, degenerate and repeated polygons are skipped and counted, neighbouring faces in one plane with one material merge (holes kept), and nothing is split where faces cross. OBJ: `v`, `f` (negative indices), `usemtl`, `mtllib` (a name the model has is its material, a library `Kd` color makes a new one, `default` is the default material); normals, texture coordinates, lines and groups are ignored. STL: binary (by its size) or ASCII. The outcome's `import` reports the instance, definition and polygon, face, vertex, skipped and new material counts. Files over 128 MiB, 200,000 polygons or 1,000,000 positions, broken lines, non-finite numbers, positions beyond +/-1e6 m and files making no face are `file` errors that change nothing. File > Import... (Cmd+I) picks an OBJ or STL and imports it as a group; the status reports faces and skipped polygons or the error. OBJ groups as separate groups, texture coordinates and a units choice in the window are deferred.
- **M8b**: golden-image render tests and toolbar wrapping. Eight canonical scenes (a box, a painted open box showing front and back colors, a rotated component copy, dashed guides, a cylinder with soft sides hidden, a parallel top view, a selected face and edge, an open group fading the rest), built with commands, are rendered headless at 160 x 120 and compared with PNGs in `crates/render/tests/golden/`: a pixel differs when a color channel differs by more than 24, and at most 0.5% of pixels may differ. On a mismatch the actual and a diff image are written under the target's tmp directory; `UPDATE_GOLDEN=1` rewrites the stored images; without a GPU adapter the tests skip. The toolbar wraps onto more rows when the window is narrower than its buttons (one row at the default 1,280 points), so every button stays reachable.
- **M8c**: clean-up of deferred correctness and performance items. Performance: each mesh keeps vertex-to-edge and vertex-to-face maps and a vertex grid in step with every change (undo and redo included; `validate` checks them); copies, groups and explodes draw their segments as one batch and rebuild the faces of each plane they reach once, finding a plane's edges with an edge index when many planes are reached; the spatial index survives paint, curve and soft tags, instances and guides. Copying 30 circles of 24 sides 3 times takes under a second in release (25 s before); `crates/kernel/tests/perf.rs` times it and other big edits. Files: numbers reopen bit for bit; ids in a file are below 2^53 and `next_id` at most 2^53, so whatever opens saves and opens again (saving a model that has given out ids past that fails); no number is the id of two things (entities in any mesh, curves, definitions, materials). Export: the outcome's `export` counts triangles written and faces left out because they cannot be triangulated; a binary STL over 2^32 - 1 triangles or a `.glb` over 4 GiB is refused. Also: a straight polyline spends no curve id; copying or erasing instances of a missing mesh is an error or no change rather than a panic; an OBJ library path that differs from the export path only in case is refused; test scratch paths carry the process id.
- **M8d**: clean-up of deferred UX and API polish items, plus the drawing speed left from M8c. Performance: drawing a batch of segments (copies, groups, explodes, context imports) finds the edges and vertices near each segment with indexes over the mesh, kept as a few levels rebuilt as the batch adds geometry, instead of scanning the mesh; rebuilding the faces of one plane finds the edges near a side and the old faces around a point with indexes when there are many; copying keeps a face's samples and the copied segments in indexes. Results are exactly those of scanning (same faces, ids and undo steps). In a release build, copying an exploded 1,800-face terrain 3 times takes under a second (9.3 s before), exploding a 20,000-face import a few seconds (86 s before), and copying 30 circles 100 times in one plane a few seconds (over 9 minutes before). So `copy` and `rotate` take up to 1,000 copies again (100 before) and an import `as` context up to 10,000 faces (2,000 before). Window: the Circle and Arc tools keep a typed side or segment count for the session; letters are lowercase unless Shift is held (Caps Lock does not make "g" into "G"); holding a letter shortcut acts once; the right panel scrolls when the window is too short for it; File > Import Units (meters, centimeters, millimeters, inches, feet) sets the import scale for the session; the export dialog suggests the format exported to last; a count like "0s" says what a count is. API: the `outliner` query takes an optional `depth` and marks nodes whose children it left out `truncated`; directions and normals of any finite non-zero length are accepted (a very long or very short one was refused as zero).

Post-v1 milestones (from the first real models: a house built through the API looked faceted and flat):

- **M9a**: soften and smooth. Commands `soften_edges {ids, soft}` (set or clear the soft flag of the given edges, and of the edges of given faces; only an edge between exactly two faces can be soft, others are left as they are and counted) and `soften_by_angle {ids, max_angle_deg}` (among the given edges and the edges of the given faces, or every edge of the open context when `ids` is empty, soften each edge between two faces whose normals differ by at most the angle; others unchanged), each one undo step reporting how many edges changed. Rendering shades faces smoothly across soft edges: a face corner's normal is the area-weighted average of the face normals around that corner reachable through soft edges, so a lathed vase or a dome reads as one surface; edges stay hidden as before. glTF export writes the same smooth normals. Window: Edit > Soften Edges... (an angle slider and Soften/Harden applying to the selection, or the open context when nothing is selected); Entity Info shows and toggles an edge's Soft flag.
- **M9b**: textures. A material may carry an image texture: PNG or JPEG read from a path given to `add_material`/`change_material` (`texture: {path, width, height}`, the image's size in meters on the model; `texture: null` removes it), decoded and kept in the document and saved inside `.skr` (schema 7; older files open without textures). Limits: 16 MiB per file, 4096 px per side, 256 textures. A face shows the texture projected in its plane in its definition's coordinates (u along the face's horizontal direction, or x on level faces; v across), so neighbouring faces continue the pattern and a component carries its texture with it. The material's color stays its average color for edges, the Materials list and formats without textures. Rendering samples a texture array with mipmaps and repeat wrapping. Export: glTF embeds textures with UVs; OBJ writes `map_Kd` images beside the `.mtl`. The `materials` query reports a texture's size in meters and pixels. Window: the Materials list shows a texture swatch, Texture... loads one, and width/height fields set its size.
- **M9c**: lights. A material may glow: `emission` (0 to 10, default 0) on `add_material`/`change_material`, saved in `.skr`. Glowing faces are drawn unshaded at their color, and each placed instance (and the open context's loose geometry) whose faces glow lights its surroundings as one point light at the area-weighted center of its glowing faces, colored like them and scaled by emission and area, with a falloff to zero at a range set by its strength; no shadows. The renderer uses up to the 128 strongest lights at the camera. A lighting mode `day` (the current headlight and ambient, lights adding to them) or `night` (headlight and ambient dimmed, a dark background, lights carrying the scene) is a view setting: `set_lighting {mode}`, a `lighting` field on `render_view`, View > Night lighting, saved with the camera in `.skr`.

## Risks

- **Sticky geometry robustness**: edge merging and face detection under floating-point error is the main risk. Mitigation: robust predicates, explicit tolerances, invariant checks in every test, fuzz tests on random edge sequences.
- **Inference feel**: hard to specify; plan for iteration after M2 with manual testing.
- **Line rendering quality**: custom wide lines need care at grazing angles; acceptable fallback is thinner lines.
- **Interface-only features creeping in**: mitigated by the per-milestone acceptance rule and command-driven tests.
