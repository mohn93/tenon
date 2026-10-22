# SketchUp Alternative — Design

Date: 2026-10-10
Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides, M6d curves, M7a camera, M7b materials, M7c outliner, M8 split into M8a-d, M8a import, M8b golden images, M8c clean-up); 2026-10-13 (M8d polish); 2026-10-14 (M9a soften and smooth, M9b textures, M9c lights); 2026-10-15 (M9d texture positioning, M9e light shadows; M10a-c full agent control); 2026-10-18 (M12a MCP adapter); 2026-10-18 (M12b batch references); 2026-10-19 (M13a split; M13a1 Polygon and Scale); 2026-10-20 (M13a2 Offset, M13a3 Follow Me, M13b split; M13b1 tags; M13b2 scenes; M16a clipboard; M16b autosave); 2026-10-22 (M13c dimensions and text; M13d split; M13d1 section planes; M13d2 print; M16c library; M15c web)

## Goal

A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.

The app is fully controllable by agents: every action a user can take is a serializable command available through a local API (except window chrome such as panel sizes and open menus, listed in `docs/agent-api.md`), the model can be queried as structured data, and the app can run headless (see section 5).

## Non-goals (v1)

- Precise B-rep/NURBS solid modeling, booleans, fillets.
- True curve geometry (arcs/circles are edge chains tagged as curves).
- Texture mapping beyond planar projection with per-face offset, rotation and scale (M9b, M9d): no UV unwrapping, no distortion (pinning) of textures.
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

An MCP server (M12a) on stdio exposes the API to MCP clients (Claude Code, Claude Desktop) as a small tool set rather than one tool per command. It runs its own headless model by default, or forwards to a running window's socket with `--live`. It is a client of the same dispatch as the JSON-RPC API, so it adds no behaviour of its own beyond argument defaults and result shaping.

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
- **M9b**: textures. A material may carry an image texture: PNG or JPEG read from a path given to `add_material`/`change_material` (`texture: {path, width, height}`, the image's size in meters on the model; `texture: null` removes it), decoded and kept in the document and saved inside `.skr` (schema 7; older files open without textures). Limits: 16 MiB per file, 4096 px per side, 256 textures, and 2^28 px (1 GiB decoded) across all textures, checked from image headers before decoding; over it, adding or changing a texture and opening a file are `file` errors. A face shows the texture projected in its plane in its definition's coordinates (u along the face's horizontal direction, or x on level faces; v across), so neighbouring faces continue the pattern and a component carries its texture with it. The material's color stays its average color for edges, the Materials list and formats without textures. Rendering samples a texture array with mipmaps and repeat wrapping. Export: glTF embeds textures with UVs; OBJ writes `map_Kd` images beside the `.mtl`. The `materials` query reports a texture's size in meters and pixels. Window: the Materials list shows a texture swatch, Texture... loads one, and width/height fields set its size.
- **M9c**: lights. A material may glow: `emission` (0 to 10, default 0) on `add_material`/`change_material`, saved in `.skr`. Glowing faces are drawn unshaded at their color, and each placed instance whose faces glow lights its surroundings as one point light at the area-weighted center of its glowing faces, while loose glowing faces (the root's, outside any instance; lights do not change with the open instance) give one such light per connected group of glowing faces (faces sharing an edge or vertex), each at its own area-weighted center, colored like them and scaled by emission and area, with a falloff to zero at a range set by its strength; no shadows. The renderer uses up to the 128 strongest lights at the camera. A lighting mode `day` (the current headlight and ambient, lights adding to them) or `night` (headlight and ambient dimmed, a dark background, lights carrying the scene) is a view setting: `set_lighting {mode}`, a `lighting` field on `render_view`, View > Night lighting, saved with the camera in `.skr`.
- **M9d**: texture positioning. A face may carry a texture placement applied on top of M9b's projection: `offset` [u, v] in meters, `rotation_deg` (counterclockwise in the face's plane seen from its front) and `scale` [u, v] (multipliers, default 1). Command `position_texture {ids, offset?, rotation_deg?, scale?, reset?}` sets the given fields (others kept) on the given faces in the open context; `reset: true` clears the placement. One undo step reporting how many faces changed. Faces split from a positioned face keep its placement; copied faces and faces copied with definitions keep theirs; an exploded instance's painted faces get placements that take up the instance's move, turn and even scale, so their textures stay where they were (a mirror or uneven scale is approximated); other new faces have none. The back side shows the texture as seen from behind, not mirrored. The `face` entity data reports the placement. Saved in `.skr` (schema 9; older files open without placements). glTF and OBJ export write the placed UVs. Window: Entity Info for a selected face (or several) shows texture offset, rotation and scale fields, Rotate 90 and Reset buttons, applying to all selected faces.
- **M9e**: light shadows. The lights of M9c cast shadows, so light stops at walls. Up to 64 lights cast shadows (the strongest at the camera, by M9c's ranking); each has a cube shadow map (256 px faces, 16-bit depth, one cube-map array). Shadow maps are kept between frames and redrawn only when the drawn geometry changes or a light enters the shadowed set or moves, so moving the camera costs no shadow passes once they are drawn; each map draws only the instances whose bounds reach the light's range. Glowing faces cast no shadows (a lamp's glowing shade does not hide its own light); everything else does, both sides. Lookups use a small filter to soften the edge and a bias against acne. Lights beyond the shadowed set light without shadows, as before. Shadows apply in day and night lighting. A golden-image test shows a wall shading the floor behind it from a lamp.

Agent control milestones (an audit found every model edit has a command, but not the editor around it; the goal is that an agent can do anything a user can without touching the window):

- **M10a**: agent sight and state. Queries: `pick` (a pixel of a view, given width, height and an optional camera like `render_view`'s, or a ray `{origin, direction}`) returns the hits front to back: entity id with its instance path from the root, point, normal and distance; `infer` (a pixel or a model point, optionally `from` a point as a tool's second click would) returns what the tools' inference would snap to: kind (endpoint, midpoint, on edge, on face, intersection, axis, guide, center, nothing), point, the entities involved and the axis lock; `editor` reports the editor state: active tool and its phase (idle or mid-operation with the points so far), paint material, fly mode, viewport size in pixels, status text, measurements box text, import units, soften angle, open dialog if any. Commands: `set_tool {tool}`, `set_paint_material {material}`, `set_import_units {units}`, `cancel` (abort the tool operation in progress, like Escape). The `history` query lists undo and redo step labels. `subscribe` also sends `selection` (selected ids), `camera` and `editor` notifications when they change, whoever changed them (window or API), coalesced to at most one of each per frame. Method `quit {discard}` ends the app (window or headless), refusing with unsaved changes unless `discard`. The spec's control promise is made exact: every action in the window either has a command or is window chrome (panel sizes, menus being open) listed in agent-api.md.
- **M10b**: tool driving and parity. Method `tool_input {events}` feeds the active tool the events a user would make, without the window: `move`, `down`, `up`, `click`, `double_click` at a pixel (with view width and height) or a model point, `drag` paths, `key` (with modifiers) and `type` (measurements box text then Enter), each run through the same inference, locks and typed-value handling as the window; the result reports the tool's phase, preview (inferred point, rubber-band geometry), status text and the outcome of any command it issued. So every tool behaviour (inference snapping, typed lengths and counts, Shift locks, Ctrl copy, double-click to open a group) is reachable. A model `point` means that point: inference resolves at it (the face or edge holding it, or free space) even when other geometry hides it from the camera, while pixels keep the window's front-most behaviour. Modifiers a `modifiers` event holds stay held across calls until another lets them go or `cancel` (so a Shift lock can span calls), are reported in the editor state, and never release keys the user holds in the window; Cmd or Alt with a key that has no binding, and typing under them, are `invalid_input`. Pixels outside the view are refused before anything runs; an event that fails at its turn stops the rest, and the error's data gives its index and what ran before it (editor, commands with outcomes, revision); a drag that fails partway lets go of its button without the release reaching the tool (no orbit or Select box left going). As in the window, the hover waits while a navigation drag turns the view. Tool change found by the review: a rectangle started on an edge or point rather than inside a face (a wall's foot) lies in the plane of the face the cursor moves across that holds the corner, as in SketchUp, in the window too. Parity gaps closed as commands: `orbit`/`zoom` about a given model point; `select` by screen box with window or crossing mode; `add_arc` by start, end and a third point (or bulge); `position_texture` with `rotate_by` (relative); `undo`/`redo` with a count.
- **M10c**: safe agent access. The window shows when agents are connected (a status-bar indicator with the count, and a log of the last commands in View > Agent Activity). An opt-in `--api-root DIR` (repeatable) limits every path the API reads or writes (open, save, import, export, textures) to those folders, resolved after symlinks; relative paths resolve against the first root, or are refused when no root and no absolute path. A path outside the roots gets one refusal whether it exists or not; the files an OBJ export writes beside it may not be links either. Without roots, behaviour is unchanged. API saves and exports refuse to overwrite a file that is not of the target format unless `overwrite: true`. File errors never echo file contents. When an API `open`/`new` with `discard: true` drops unsaved window work, the dropped model is first written to a private (0600) recovery file and the window says so. Saves and exports write through a fresh hidden temporary file beside the target and never delete a file they did not create.

Interface milestones (a four-part design audit on 2026-10-16 found the engine solid but the window plain: stock egui with no theme, fonts or icons; one wrapped text row mixing menus, tools and commands; a fixed 180 pt right panel that clips; snap markers told apart by color only; all feedback in the status bar; several SketchUp conventions missing; and no macOS menu bar. The goal is a window that feels like a finished design tool. Window-only preferences (theme, UI scale, panel sizes, recent files) are chrome and are listed as such in agent-api.md. Every model or editor action added below gets a command or a `tool_input` path as before):

- **M11a**: design foundation, with the layout unchanged.
  - `ui.rs` becomes a `ui/` module tree with no behaviour change.
  - The UI tests move to `egui_kittest` (0.36.2) and find widgets by label through the AccessKit tree instead of sweeping screen coordinates.
  - A theme module defines dark and light token sets: background levels, border, text primary/secondary/disabled, one accent (`#4C8DFF` dark, `#2463EB` light), warn, danger and success. It also sets a 4 pt spacing grid, 28 pt controls, a corner radius of 6 for controls, 8 for windows and 4 for swatches, and a 2 pt accent focus ring.
  - Fonts are Inter for text (13 pt body, 11 pt uppercase section titles) and JetBrains Mono for values, both OFL, plus the Phosphor icon font (`egui-phosphor`, MIT), all built into the binary.
  - A widget kit: tool button, icon button, section header, numeric field, swatch, and a menu item that takes its shortcut text from one shortcut table. The same table drives the key handling, so labels cannot drift from bindings.
  - The theme (System, Dark, Light; default System) is set from View > Theme and saved in a preferences file, `~/.sketchup-alt/prefs.json`, which is window chrome.
  - The viewport follows the theme:
    - Day lighting draws a light canvas gradient in the light theme and a mid-dark one in the dark theme.
    - `render_view` and headless renders keep their current backgrounds, so golden images and agent renders do not change.
  - The selection uses the accent everywhere: list rows, the viewport's selected edges at 2 px, and the face tint cut to 18%.
  - The right panel loses its magic heights. Sections size from measured text, no row is clipped, and the panel keeps its current place for now.
  - Undo and Redo are disabled when there is nothing to undo or redo, and their tooltips name the step (from the history labels).
  - The window has a minimum size of 960 x 600 pt.
- **M11b**: layout.
  - **Tool rail:** a 44 pt rail on the left of 32 pt icon buttons in groups, separated by 8 pt gaps:
    - Select, Erase, Paint
    - Line, Rectangle, Circle, Arc
    - Push/Pull, Move, Rotate
    - Tape Measure

    The tooltips name the tool and its key in a chip. The active tool has an accent fill. The rail never wraps.
  - **Context bar:** a 36 pt bar along the top holds:
    - Undo and Redo as icons.
    - A breadcrumb of the open path (`Model > House > Kitchen`): clicking a crumb runs `open_path` to that level, and Esc still closes one level.
    - Make Group, Make Component and Explode, shown only when they apply.
    - The day/night toggle and the agents indicator.
  - **Right dock:** a resizable dock, 280 pt by default, between 240 and 420, with its width saved in prefs. It has two areas split by a draggable divider:
    - Tabs for Entity Info and Materials.
    - The Outliner, with a filter field that matches names. Matching rows show along with their ancestors.
  - **Materials:** shown as a swatch grid with an "Edit material" section under it. Clicking a swatch chooses it for editing and as the paint material without switching tool. Double-clicking a swatch, or choosing the Paint tool, paints.
  - **Status bar:** 28 pt tall, with every element on one baseline:
    - The tool name chip and its prompt.
    - The axis lock as a pill colored for its axis.
    - The measurements box, 180 pt wide, always labelled, with JetBrains Mono values and an accent ring while typing. In Select it is hidden.
    - The selection count.
    - The agents badge.
  - **Menus:** on macOS the menus move to the native menu bar through `muda`; other systems keep egui menus built from the same table. The menu layout is:
    - **App menu:** About, Settings..., Quit.
    - **File:** New, Open..., Open Recent, Save, Save As..., Import..., Export...
    - **Edit:** Undo <label>, Redo <label>, Select All, Select None, Erase, Delete Guides, Soften Edges...
    - **Draw:** the four drawing tools.
    - **Tools:** the other tools.
    - **Group:** Make Group, Make Component, Explode, Close Group, Rename Component..., Purge Unused.
    - **Camera:** as now, plus Field of View...
    - **View:** Theme, Agent Activity.
    - **Help:** Keyboard Shortcuts, which opens a window listing every binding from the shortcut table.

    A shortcut that a native menu handles is not also handled by the editor's key path, so it never fires twice. Items whose milestone has not landed yet are left out of the menus until it does.
- **M11c**: feedback in the viewport.
  - **Snap markers:** each kind has its own shape and color: endpoint is a green filled circle, midpoint a cyan filled circle, intersection an orange X, on edge a red hollow square, on face a blue diamond, and a guide point a cross. Markers are 12 px with a 1.5 px halo (white in day, black in night).
  - **Preview lines:** 2 px, and 2.5 px when locked to an axis. The axis colors are shared by the axes and the locks: X `#E5484D`, Y `#30A46C`, Z `#3E63DD`. The axes are 1.5 px, faded on their negative side.
  - **Snap label:** after the cursor rests for 150 ms, a chip near it names the snap ("Endpoint", "On Red Axis", "Parallel to Edge", "Midpoint in Group").
  - **Live measurement:** a line under the snap label shows the measurement or, while typing, the typed text. An invalid typed value flashes its message at the cursor for 2.5 s and also stays in the status bar.
  - **Hover highlight:** the face or edge a click would act on is highlighted for Select, Push/Pull, Paint and Erase. Faces mix 25% toward the accent and edges are drawn 2 px in the accent. Erase shows the target in the danger color, and Paint previews the material.
  - **Previews:** Push/Pull previews a solid in the face's material at 70% opacity.
  - **Cursors:** each tool sets a cursor: a crosshair for the drawing tools and Tape, move arrows for Move, and grabbing while orbiting.
  - **Select:** the Select tool draws no snap markers.
  - **Orientation gizmo:** a 72 pt gizmo in the viewport's top right shows the axes. Clicking an axis end sets that standard view, and clicking the centre switches between perspective and parallel.
  - **API:** the `editor` query and `tool_input` results report the hovered entity and the snap label, so agents see what the window shows.
- **M11d**: interaction conventions.
  - **Drag to draw:** Line, Rectangle, Circle, Arc and Push/Pull treat a release more than the drag threshold from the press as the second click.
  - **Push/Pull:**
    - A typed distance right after a pull redoes it at that distance, as Move and Rotate already allow.
    - A double-click on a face repeats the last pull distance.
  - **Selection clicks:**
    - With Select, double-clicking a face selects it with its edges, and double-clicking an edge selects it with its faces.
    - Triple-clicking selects everything connected.
    - Double-clicking an instance still opens it.
  - **Select All and Select None:** Cmd+A selects everything in the open context and Cmd+Shift+A selects nothing. `select` takes `all: true`.
  - **Duplicate:** Cmd+D copies the selection in place and starts Move on the copy.
  - **Rotate:** before the first click, the arrow keys lock Rotate's protractor plane to an axis.
  - **Startup:** the app starts in Select.
  - **No silent dead ends:**
    - Clicking empty space with Push/Pull, Paint or Erase says what the tool works on.
    - Move's idle prompt describes what it actually does.
  - **Empty-model card:** an empty model shows a dismissable hint card, 360 x 200 pt, bottom left of the viewport. It lists New, Open and the five most recent files, with a one-line hint for drawing a box. It goes away at the first edit.
- **M11e**: app basics and missing UI.
  - **Recent files:** Open Recent lists the last 10 files from prefs.
  - **Unsaved changes:** the prompt reads `Do you want to save changes to "<name>"?` with Save, Don't Save and Cancel.
  - **Window title:** on macOS the title is the file name, and the window shows the edited dot instead of `*`.
  - **Import options:** before the file is read, a sheet sets units, up axis, import as (group, component or loose) and merge coplanar faces. The last values are remembered.
  - **Component names:**
    - Rename Component... renames a definition (`rename_definition`), as do F2 on a selected Outliner row and the Entity Info name field.
    - Make Component asks for a name.
  - **Purge Unused:** runs `purge_definitions` and reports the count.
  - **Field of View...:** sets the camera's field of view (`set_camera` `fov`).
  - **Settings** (Cmd+,) has two tabs:
    - Appearance: theme and UI scale (90, 100, 110 or 125%).
    - Input: trackpad scroll orbits or zooms.

    Both are saved in prefs.
  - **Accessibility:** AccessKit is switched on, so VoiceOver reads the window's controls. Every icon-only control has a label.

  Deferred:
  - the Offset, Scale and Follow Me tools (now M13a1-M13a3)
  - cut, copy and paste between models
  - display units and precision
  - custom cursor images
  - number keys for standard views

Agent integration milestones:

- **M12a**: MCP adapter.
  - **Server:** `app --mcp [--api-root DIR]... [FILE]` runs an MCP server on stdin/stdout with its own headless model, as `--headless --stdio` does. `app --mcp --live SOCKET` instead forwards every call to the window (or headless instance) listening on `SOCKET`; that program's file and `--api-root` folders apply, so `--live` takes neither. Nothing but MCP messages is written to stdout.
  - **Protocol:** MCP over stdio (newline-delimited JSON-RPC 2.0): `initialize` (protocol versions 2025-11-25, 2025-06-18, 2025-03-26 and 2024-11-05; the client's when supported, else the newest), `ping`, `tools/list` and `tools/call`; notifications get no reply.
  - **Tools:** `describe` (the API description, or one named schema), `transact` (one command, or several as one atomic batch by default), `query`, `pick`, `infer`, `render_view` (the PNG as MCP image content) and `tool_input`. Each tool's input schema is the API's JSON Schema under an object root. A failed call is a tool result with `isError` and the API's error (code, message, data), so the model can correct itself.
  - **API changes it needs**, in the JSON-RPC API too:
    - `describe` gives each method's params and result as JSON Schemas; the `execute` result schema includes `revision`; model coordinates carry their +/-1e6 m bounds; Command, Query and the other command schemas have an object root.
    - `batch` results carry the net `changes` of the whole batch.
    - Undo steps carry an origin, `user` (the window) or `agent` (the API), listed by the `history` query; `undo` takes an `expect` (label and optional origin) and refuses with `unexpected_step` when the next step differs, so an agent does not undo a person's work in a shared window.
  - **Docs:** setup for Claude Code and Claude Desktop, headless and live.
- **M12b**: references inside a batch.
  - A command in a `batch` (and in the MCP `transact` tool) can name ids an earlier command of the same batch made, by reference instead of by id: `"$N:KI"` is the I-th id of kind K (`v`, `e`, `f`, `i`, `g`, or `m` for materials) in command N's outcome (`changes.created`, or `changes.materials` for `m`), counted from 0 in the order the outcome lists them; `"$N:K*"` is all of that kind and `"$N:*"` every edge, face, instance and guide command N created (not its vertices). N counts from 0 and must be lower than the referring command's own index.
  - References are allowed only where a command takes an entity or material id (`ids`, `face`, `id`, `material`, and `open_path`'s `path`); anywhere else such a string is plain text. `*` references only in lists of ids.
  - A malformed reference, or one to the referring command or a later one, rejects the whole batch before anything runs (invalid params). A reference to an id the earlier command did not make (`"$0:f1"` when it made one face) fails that command with `invalid_input`, like any failing command: an atomic batch is rolled back.
  - `describe` and the MCP `transact` schema accept a reference wherever they accept an entity or material id in a batch.

Modeling parity milestones (M13a is split in three: M13a1 Polygon and Scale, M13a2 Offset, M13a3 Follow Me):

- **M13a1**: Polygon and Scale.
  - **Polygon:** `add_polygon {center, radius, sides?, normal?, circumscribed?, direction?}` draws a regular polygon of `sides` (default 6, 3 to 360) in one undo step: `radius` is the distance to its corners, or with `circumscribed` to the middles of its sides; `direction` (default the plane's u axis, as for circles) points from the center to the first corner, or to the middle of the first side when circumscribed. Its edges form one curve marked as a polygon: it selects as one, Entity Info reads "1 polygon" with radius and sides, the `curve` query's `kind` is `polygon`, and pulling it gives hard sides (a circle's are soft). The mark is kept through copies, splits, groups and files. The Polygon tool (key `n`) works like the Circle tool: click the center, then a corner (the cursor sets the radius and the rotation); typed `8s` sets the sides (remembered for the session), a typed length the radius; Ctrl switches inscribed and circumscribed.
  - **Scale:** `scale {ids, origin, factors}` scales edges, faces (their vertices), vertices, instances and guides by `factors` [x, y, z] along the model axes about the fixed point `origin`, in one undo step. Factors are finite, not zero, at most 1e4 in size; a negative factor mirrors; all factors 1 is `invalid_input`. Faces that move whole keep their front side out (a mirrored box stays outward); faces that move partly must stay flat and not fold; an edge shrunk below the length tolerance is refused. Instances take the scale into their placement (stretched or mirrored instances already render and pick correctly); Entity Info shows an instance's scale. The Scale tool (key `s`) shows grips on the selection's box (8 corners, 12 edge middles, 6 face middles; grips along a flat side of the box are left out): drag or click a grip and move to scale, the opposite grip staying fixed; corner grips scale uniformly and edge and face grips along their own axes, Shift switching between the two, and Ctrl switching to scaling about the box's center. A typed number is a factor (`2`, `-1`, or `2,1.5` for the grip's axes), a typed length the new size along the grip (`3m`); typed after the scale, it redoes it.

- **M13a2**: Offset (see `plans/2026-10-20-m13a2-offset.md`).
- **M13a3**: Follow Me. `follow_me {face, path}` sweeps a profile face along a chain of connected edges (open, closed, or another face's outer loop for a lathe) in one undo step: sections on each joint's miter plane carried by a rotation-minimising frame, straight runs taken as one segment, caps on an open path with the profile's materials, the profile face replaced; joints along curves and sides from curved profile edges are soft. A profile too big for a turn or degenerate against the path is refused; the sweep merges with what it touches but does not cut solids. The Follow Me tool (key `j`): select the path edges and click the profile, or select a face and click the profile to sweep around that face's edges.
- **M13b1**: Tags. A scene-level tag table (name, color, visibility; ids `t<n>`) and a tag on every edge, face, instance and guide (none = Untagged). Commands `add_tag`, `change_tag`, `delete_tag {move_to?}`, `set_tag`, query `tags`, and `tag` in entity info. A hidden tag hides what it tags, and everything inside a hidden group or component, from drawing, picking, inference, select all, Zoom Extents and OBJ/STL/glTF export; hidden entities stay selected and reachable by id. Color by tag is a view setting. New geometry from push/pull and Follow Me is Untagged except the pulled cap and Follow Me's caps, which keep the source face's tag. A Tags panel and an Entity Info tag field. `.skr` schema 10. `.skp` import maps layers to tags (Layer0 is Untagged), export writes every tag as a layer (see `plans/2026-10-20-m13b1-tags.md`).
- **M13b2**: Scenes. An ordered list of named scenes (ids `p<n>`), each storing a view (camera, projection, field of view, night lighting, color by tag) and/or the hidden tags. Commands `add_scene`, `update_scene`, `change_scene {name?, camera?, tags?}`, `move_scene`, `delete_scene`, `go_to_scene` (the view change is not undoable; tag visibility is one undo step when it changes), queries `scenes` and `scene`. Scenes cannot store a camera beyond the coordinate limit, and save checks what open checks. Scene tabs above the viewport, a Scenes panel, View menu with Page Up/Down. `.skr` schema 11. `.skp` 2021+ import brings pages in as scenes (hidden layers through the tag map, at most 1024); export reports scenes as not written (see `plans/2026-10-20-m13b2-scenes.md`).
- **M16a**: Cut, copy and paste. A clip is a standalone model (selected edges, faces, instances with the definitions they reach, guides, and the materials and tags used) in world coordinates, carried as text on the system clipboard (`sketchup-alt clip 1` header, base64 of an in-memory `.skr` archive plus base point and source document token). Paste places the clip's base point with the cursor (Move's inference) in the open context; Paste In Place keeps world coordinates. Materials and tags merge by name; components are reused within the source document and shared across repeated pastes in a session. Untrusted clips are fully validated and capped (64 MiB, 100,000 placed instances, 20M placed faces, depth 32). Edit menu, Cmd+X/C/V, editor commands `copy_selection`, `cut_selection`, `paste_clipboard`, `paste_in_place`, and the session command `paste` (see `plans/2026-10-20-m16a-clipboard.md`).
- **M16b**: Autosave and crash recovery. Each window process holds an OS lock on its own session file in the private recovery folder and autosaves the modified model there every N minutes (Settings > Files, default 5, 0 = off; held back at most one extra interval while a tool gesture is in progress), with a JSON sidecar; the user's file is never written. The autosave is removed after a save, new, open or a settled exit, and written fresh on an unprompted system quit. At start a Recover Models window offers autosaves of sessions whose lock is free (crashed); File > Recover Models... also lists files kept by agent discards. Open restores as an untitled modified model that becomes this window's autosave; Discard and Discard All confirm first. Listing is bounded (200 newest) and skips links and non-private files. Headless and MCP never autosave (see `plans/2026-10-20-m16b-autosave.md`).
- **M13c**: Dimensions and text. Per-mesh dimensions (`r<n>`: linear between two anchors with an offset, radial on an arc or circle, radius or diameter) and notes (`n<n>`: leader notes anchored to a point with explicit or automatic text, screen notes fixed to the view in the root). Vertex anchors follow edits; when a vertex or curve goes the anchor freezes to its last position in the same undoable step, and a dimension whose ends meet is removed. Display units (meters, centimeters, millimeters, inches, feet and inches with fractional precision) are saved in the model and format labels; other UI lengths stay in meters until M14f. Dimension lines are drawn by the renderer, labels by the window overlay. Dimension and Text tools, a note sheet, a Settings Model tab, and Tools > 3D Text... (Inter glyph outlines through skrifa as faces with holes, optionally extruded, touching letters kept as separate groups). `.skr` schema 12 only with annotations or non-default units; exports report annotations as not written; legacy `.skp` import counts them as not imported (see `plans/2026-10-20-m13c-dimensions-text.md`).
- **M13d1**: Section planes. A per-mesh entity (`s<n>`: a point and a unit normal) with one active plane per context; an active plane cuts away what lies behind it in its own context and everything nested there (never the parent or siblings), applied on the CPU when the scene is built, so drawing, picking, inference and box select agree. Closed cuts get a flat dark fill (regions of the planar arrangement of cut segments that a majority-of-rays parity test finds solid) and wide cut edges; open cuts get cut edges only. Active planes are drawn orange. Display toggles for planes, cuts and fill are view settings. Section Plane tool (face or axis-locked), context menu Flip and Active Cut, View menu toggles. `.skr` schema 13 only with planes or non-default display; exports ignore cuts and report planes as not written (see `plans/2026-10-22-m13d1-section-planes.md`).
- **M13d2**: Print, PDF and SVG. A CPU hidden-line line drawing of the view (shown non-soft edges split into short pieces tested by sight rays against each placement's BVH, section cuts and fills as in the renderer, dimension and note labels) exported as SVG, in pixels or in millimetres at 1:N for a parallel view. PDF 1.4 written by the app itself: the view rendered by the GPU on white paper (no grid, axes or sky) fitted on one sheet, or the drawn extent at exactly 1:N tiled across up to 100 sheets with trim marks, a scale bar in the display unit and sheet footers, label text as PDF text clipped to the printable area. `export_pdf` for agents, File > Export PDF... and File > Print... (writes a temporary PDF and opens it in the system's default viewer) with a Print sheet remembered in prefs (see `plans/2026-10-22-m13d2-print.md`).
- **M16c**: Component library. Library items are ordinary `.skr` files in a folder chosen in Settings; items saved by the app carry a `library.json` entry (kind component, material or model, and a name), other `.skr` files list as models. Save a selected component (rotation and scale kept), a material (texture included) or the whole model; list a folder (regular `.skr` files only, no links, bounded); insert through the clip path with a fresh token (always new definitions, materials and tags merged by name; a material item reused if identical, else renamed "Name 2"), keeping world size and orientation in a transformed open group. Every item read passes the clip caps. A Library dock tab with search, a kind filter, thumbnails (white paper, iso, cached), the Paste tool for components and models, and Insert, Open and Delete; save actions in Entity Info, the context bar, the Materials panel and the File menu. Commands `save_to_library`, `insert_library_item`, query `library`, editor command `place_library_item` (see `plans/2026-10-22-m16c-library.md`).
- **M15c**: Web build. The shared crates build for wasm32 (time through `web-time`, the agent transport native-only, shadows decided by adapter capability, WebGL2 downlevel limits) and the session opens, imports, saves and exports from bytes. `app` is a library plus a thin native binary; platform services (file open and save, prefs storage, the unsaved prompt, clipboard) go through a `Platform` trait, and `Features` switches off what the web lacks (library, autosave and recovery, print, open recent, quit, agent socket). A `web` crate runs the app on a page canvas (WebGPU first, WebGL2 with light shadows off), opens files by upload or the File System Access API (a handle tied to the document for Save), saves by download or write-back, keeps prefs in localStorage, asks about unsaved changes in the app and guards tab close, pastes from the page paste event, and offers a format sheet for Export. egui-winit 0.36.2 is vendored with two small web changes. `scripts/build-web.sh` and `scripts/serve-web.sh`; CI checks the wasm build (see `plans/2026-10-22-m15c-web.md`, `docs/web.md`).
- **M13e1**: SketchUp import, 2021 and newer files (M13e is split in three: M13e1 import of the 2021+ container, M13e2 the 2017-2020 container, M13e3 export; see `research/2026-10-19-skp-readers.md`).
  - **Reader:** openskp-rs's `skpcore` (MIT) is forked into the workspace as the `skp` crate and owned from then on: kept close to upstream (its own edition, clippy relaxed for its code), memory-mapped reading removed, the inflate bug fixed, allocations bounded by the file size. Files from SketchUp 2020 and older are refused with a clear message until M13e2.
  - **Import:** `import` takes `.skp` (`format: "skp"`, from the extension or the file's header): edges, faces with holes, soft, smooth and hidden edges (between two faces they become soft), front and back materials (colour, opacity, texture with its tile size; per-face texture positions are not carried yet), groups and components as our groups and components (one definition per SketchUp definition, shared by its instances, nested to any depth with their transforms, including scaled and mirrored ones; an instance's material paints what is unpainted inside it). Guides move to M13e2 (the 2021+ reader does not parse them yet). Inches become meters (`scale` multiplies on top; Z is up, `up` is ignored). As today the import lands as one new group or component named after the file, or as loose geometry (`"as": "context"`).
  - **Not carried yet, and reported:** tags and their visibility, scenes, hidden flags (hidden faces, instances and edges not between two faces are imported visible), image entities, section planes, text and dimensions. The outcome's `import` report counts definitions, instances, guides and textures and lists these as warnings; faces the reader could not build are counted as skipped.
  - **Safety:** the file is read whole within the 128 MiB limit and never memory-mapped; every coordinate is checked finite and within the model bounds after placement; faces, definitions, instances, nesting depth, materials and textures have limits; a reference cycle or a missing definition is refused or skipped, never followed. A malformed file gives an error, never a crash.
  - **Window:** File > Import lists `.skp` (units and up axis do not apply to it); File > Open and the FILE argument open a `.skp` as a new untitled model whose root is the file's root, with no group around it.
- **M13e2**: SketchUp import, 2017-2020 files, and guides. The legacy reader in the `skp` crate returns the same topology the 2021+ reader does (faces with holes and back materials, hidden faces kept, groups told from components, instance flags), reads the SketchUp 2020 container (its extra block before the component list), recovers past unreadable entities where it safely can, and is hardened like the 2021+ path; files it cannot read give a plain error, never a wrong model. Guides are imported from both containers. Files older than SketchUp 2017 are refused. Each 2017-2020 copy of a model imports like its 2024 original (the 96-file regression set, kept outside the repo; the CC BY cupboard's four copies are fixtures in the repo).
- **M13e3**: SketchUp export. File > Export and `export` write `.skp` in the SketchUp 2017 format (opened by SketchUp 2017 and every later version): the whole model with its groups and components (each definition written once, placed by its instances, nested with scaled and mirrored transforms), faces with holes and front and back materials, soft edges, loose edges, guides, and materials with opacity and textures (the stored image, its size, and each textured face's placement). A new `skpwrite` crate ports openskp's writer (MIT), splicing the model into openskp's blank 2017 document. Not written yet, and reported: glowing materials (SketchUp cannot store them), curves (their edges are written separately), tags and scenes (after M13b). Our reader checks every written file in the tests; SketchUp itself is the final check.

## Risks

- **Sticky geometry robustness**: edge merging and face detection under floating-point error is the main risk. Mitigation: robust predicates, explicit tolerances, invariant checks in every test, fuzz tests on random edge sequences.
- **Inference feel**: hard to specify; plan for iteration after M2 with manual testing.
- **Line rendering quality**: custom wide lines need care at grazing angles; acceptable fallback is thinner lines.
- **Interface-only features creeping in**: mitigated by the per-milestone acceptance rule and command-driven tests.
