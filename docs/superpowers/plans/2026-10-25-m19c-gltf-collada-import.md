# M19c glTF and COLLADA Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring models from other tools in through their two common exchange formats: glTF (`.gltf`, `.glb`) and COLLADA (`.dae`), with their node trees, shared meshes, colors and textures, through File > Import and the `import` command.

**Architecture:** Two readers in `crates/session` turn a file into one shared intermediate, a `ReadScene` (meshes of triangles and polygons with optional corner normals and texture coordinates, materials with a color and an optional image, and a node tree whose nodes place meshes and other nodes). One builder turns a `ReadScene` into the kernel's `ImportTree` (the tree the SketchUp import builds): a node becomes a group, a mesh placed by more than one node (glTF) or a COLLADA library node placed by `instance_node` becomes a component, and the up axis and scale are baked into points and transforms. Each mesh's polygons go through a new kernel `polygon_geometry`, which welds, merges coplanar neighbours as `import_mesh` does and marks the interior edges of smooth surfaces soft. Texture coordinates become a material texture size and a per-face texture placement fitted to them. The rest (materials kept only when used, placing, `"as": "context"`, the report) is the SketchUp import's path.

**Tech Stack:** Rust 2024. glTF JSON parsed with `serde_json` (already a dependency); `.glb` chunks and accessors read by hand. COLLADA with `roxmltree` `=0.21.2` (MIT/Apache-2.0, no dependencies).

**Spec:** `docs/superpowers/roadmap.md` row M19c.

## Global Constraints

- One new dependency (`roxmltree`). Golden images unchanged. `docs/agent-api.md`, roadmap row.
- Gate per task through the shared script. No emoji. No trailers. Never `git add -A`.
- Test fixtures are small hand-written files; never SketchUp's or Trimble's sample files.

## Spec notes (rulings)

1. **Formats.** `ImportFormat` gains `gltf` (reads `.gltf` JSON and `.glb` binary alike, told apart by the `glTF` magic; `glb` is accepted as an alias) and `dae`. Extensions `.gltf`, `.glb`, `.dae` select them.
2. **Units and axes.** glTF is meters and Y-up by its spec: `scale` (default 1) and `up` (default `y`) apply as for OBJ. COLLADA names its own unit (`<unit meter>`, default 1) and up axis (`<up_axis>`, default `Y_UP`; `X_UP` and `Z_UP` also read): the length is the file's unit times `scale`, and `up` given overrides the file's axis. The window sends its import units and up axis for glTF (as for OBJ) but neither for COLLADA, whose file names both. The axis change and scale are baked into points and node transforms (`T' = M T M^-1`), so definitions hold real-size geometry.
3. **Hierarchy.** The wrapper group or component (named after the file) holds the scene's root nodes. A node with a mesh or children becomes a group named after the node (`Group` when unnamed) placed with its transform; a node with neither is skipped. glTF: a mesh used by one node goes into that node's group; a mesh used by two or more nodes becomes one component (named after the mesh, `Component` when unnamed) placed in each of them, and a node holding only that mesh is placed as the component itself (no group around it). A scene that is a single node with a mesh used once and no children is flattened into the wrapper (its transform baked into the points), so a plain exported model is not wrapped twice. COLLADA: `library_nodes` nodes reached by `instance_node` become components (each once, shared); visual scene nodes become groups as above; `instance_geometry` of one geometry with the same material binding in two or more places becomes a component as glTF meshes do. glTF `scenes[scene]` (else scene 0, else every node no node names as a child) gives the roots; COLLADA's `scene/instance_visual_scene` gives the visual scene (else the first).
4. **Faces.** Triangles (glTF modes 4, 5, 6; COLLADA `triangles`, `polylist`, `polygons`, with a `<ph>` hole kept when the face with it is flat and simple, `trifans`/`tristrips` read too) go through `polygon_geometry`: corners closer than 1e-6 m weld, with `merge_coplanar` (default true) coplanar neighbours with one material and texture placement merge, and an edge between two faces that are not merged is soft when the file's normals at both its ends agree on the two faces (within 1e-3) — a smooth surface; without normals (glTF says flat shading) edges stay hard. Lines and points are skipped and counted in the report's warnings.
5. **Materials.** One material per file material a made face uses: name as in the file (`Material` when unnamed, numbered against the model's names as SketchUp import does: `Brick 2`; existing materials never change), color from glTF `baseColorFactor` (linear to sRGB) or COLLADA `diffuse` `<color>` (linear to sRGB; a `transparent`/`transparency` or alpha below 1 gives opacity), and the base-color / diffuse texture image (PNG or JPEG, under `add_material`'s limits; a texture that cannot be used leaves the color, with a warning as SketchUp import words it). A textured material's color is the image's average color times the factor. At most 1,024 materials are added. Faces without a material are unpainted.
6. **Texture coordinates.** The material's texture size is fitted from the first textured triangle's coordinates (meters per repeat along the image's width and height, clamped to 0.001-1000 m); each face gets the texture placement (offset, rotation, scale) that maps its plane onto its coordinates; a mapping with shear or a mirror takes the nearest rotation and scale; a placement past the placement limits is dropped (the projection shows). glTF `v` runs down the image, COLLADA's up.
7. **Side files.** Buffers and images are `data:` URIs (base64), the `.glb` BIN chunk, buffer views, or relative file names (percent-decoded) read beside the file: never absolute, never with a `..` part or a scheme (`http:` and the like); under `--api-root` the resolved file must lie within the roots (as OBJ material libraries are). A buffer is at most 128 MiB, an image at most 16 MiB; an import from bytes (the web build) reads no side files. A buffer that cannot be read is a `file` error; an image that cannot be read is a warning.
8. **Limits.** The readers cap points at 1,000,000, polygons at 200,000, nodes at 100,000, meshes and materials at 10,000 and nesting at 32 (COLLADA `instance_node` cycles are refused); accessors are bounds-checked against their buffer views and buffers (no reads past the end, no overflow); indices out of range skip the polygon. The kernel's tree limits apply on top.
9. **Skipped, reported.** Animations, skins, cameras, lights, morph targets, glTF extensions the model needs (`extensionsRequired`, e.g. Draco compression: a `file` error naming it), COLLADA controllers, cameras and lights: each a warning sentence ("2 animations were not imported"). The report counts `definitions`, `instances`, `textures` and `warnings` as SketchUp import does.
10. **Window.** File > Import's filters add glTF and COLLADA; the status after an import of either is the SketchUp-like summary (faces, definitions, instances, warnings).

## Review Focus

1. Accessor and buffer reading: bounds, strides, normalized and integer component types, sparse accessors refused cleanly; path escapes refused.
2. The node tree: shared meshes become one component, transforms with the axis change and scale baked, mirrored nodes.
3. Merging and softening: coplanar triangles back to the model's faces (round trip with `export` to glb/gltf), smooth surfaces soft, hard creases hard.
4. Texture placement fit against the exporter's coordinates.

---

### Task 1: Kernel polygon geometry with texture placement and soft edges

**Files:** `crates/kernel/src/import.rs` (MeshPolygon and BuiltFace gain `texture`; merging needs equal placements; `polygon_geometry`), `crates/kernel/src/import_tree.rs` (ImportFace gains `texture`, carried to the face), call sites; tests (two coplanar triangles merge into one face; a cylinder band of quads with shared normals comes out with soft sides and hard caps; placements carried through `import_tree`).

- [ ] **Step 1-5:** tests, implement, gate, commit `Kernel: polygon geometry for imports with texture placements and soft edges`.

### Task 2: glTF import

**Files:** `crates/session/src/(new) scene_read.rs` (ReadScene, the builder to ImportTree, texture fit), `crates/session/src/(new) gltf_read.rs`, `crates/session/src/import.rs` (formats), `crates/session/src/document/import.rs` (dispatch), fixtures `crates/session/tests/fixtures/gltf/`; tests (a glTF box; a glb with a texture; a node tree with a mesh used twice becoming one component placed twice; export to glb and gltf then import compares faces and area; path escapes refused; data URIs; malformed accessors refused).

- [ ] **Step 1-5:** tests, implement, gate, commit `glTF import`.

### Task 3: COLLADA import

**Files:** `Cargo.toml`, `crates/session/Cargo.toml` (roxmltree), `crates/session/src/(new) dae_read.rs`, fixtures `crates/session/tests/fixtures/dae/`; tests (a SketchUp-like DAE with Y_UP and centimeters, library_nodes as a component placed twice, bind_material colors, a texture side file; polylist with a hole; refusals).

- [ ] **Step 1-5:** tests, implement, gate, commit `COLLADA import`.

### Task 4: Window, docs and roadmap

**Files:** `crates/app/src/flows.rs` (filters), `crates/app/src/editor.rs` (units and summary), `docs/agent-api.md`, roadmap; tests (filters list the new extensions; status summary).

- [ ] **Step 1-5:** tests, implement, gate, commit `glTF and COLLADA in File > Import; docs`.
