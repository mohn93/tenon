# M7b Materials Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Named materials can be added, renamed, recolored, deleted and painted on face sides and groups, by agents (commands) and in the window (Paint tool `b`, Materials list); paint follows the geometry, is saved in `.skr` and exported to OBJ (.mtl) and glTF.

**Architecture:**
- kernel: `material.rs` holds `Material`, `Side`, name rules and the paint operations; `Scene` owns the material table (steps for add/remove/change) and checks references in `validate`; faces carry `front`/`back`, instances `material`; two new `Op`s make painting undo-exact; paint is propagated by the existing region rebuild (container face), stencil drawing (copies, groups, explode) and push/pull extrusion; `Triangle` resolves materials for export (face, else nearest painted instance).
- session: four commands and a `materials` query in `document/materials.rs`; `ChangeSummary.materials` reports material edits (new id of `add_material`); `.skr` schema 5; export groups triangles by front material: OBJ + `.mtl`, glTF primitives/materials.
- render: per-vertex front/back colors resolved in `scene_geometry` with instance inheritance; shader chooses by facing; defaults live in Rust.
- tools: `PaintTool` (`b`) emits only `Command::Paint`; sampling reported through `ToolOutput.sampled`; `ToolContext` carries the current material and the sampling modifier.
- app: `Editor` keeps the current material and runs material commands; Materials list in the right panel under Entity Info; Cmd/Ctrl modifier forwarded; Entity Info shows materials.

**Tech Stack:** Rust 2024; no dependency changes.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md`: materials (section 2), commands and queries (section 5), export. Task 2 adds the M7b milestone line.

## Global Constraints

- Every new capability is a `session::Command` reachable through the API, covered by a command-driven test. Tools only emit commands.
- Each modeling command is one undo step. Command inputs and the points they produce must be finite and within +/-`MAX_COORDINATE` (1e6 m).
- No dependency changes.
- Git:
  - work on `dev`;
  - stage only the files you changed;
  - short plain commit messages, no emoji;
  - run `cargo fmt --all` before each commit;
  - `cargo clippy --workspace --all-targets -- -D warnings` stays clean.
- Do not launch the app window.

## Spec notes (rulings made in this plan)

1. **Where materials live.** `Scene` owns `materials: BTreeMap<MaterialId, Material>`, shared by the root and every definition (components share them; one list per model). Not per mesh, not in `Document`.
2. **Material ids.** `MaterialId(u64)`, shown and parsed as `"m<n>"` (canonical digits, like `d<n>`), JSON Schema pattern `^m(0|[1-9][0-9]*)$`. Allocated from the scene's entity id counter (`Scene::alloc_material`), so a material id never equals an entity or definition id and is never reused; `Scene::from_meshes` counts material ids when computing `next_id`.
3. **Material = name + sRGB color.** `Material { name: String, color: [u8; 3] }`. No opacity, no textures (deferred: translucency needs sorted blending). JSON colors are `[r,g,b]` integers 0-255; serde's `u8` rejects 256, 0.5 and wrong lengths (bad params, -32602 over the API).
4. **Names.** Not blank (after trim), at most 256 characters (`MAX_MATERIAL_NAME`), distinct (exact comparison). Checked in the kernel (`KernelError::MaterialName(String)` with the reason) and in `Scene::validate` (so files with bad names are refused). `add_material` without a name takes `Scene::free_material_name("Material")` -> "Material", "Material 2", ...
5. **Face paint.** `Face` gets `front: Option<MaterialId>` and `back: Option<MaterialId>` (None = default material); `front` is the side the normal points to. Helpers `Face::new(outer, holes, normal)` (unpainted), `Face::reversed()` (loops reversed, normal negated, front/back swapped), `Face::paint() -> [Option<MaterialId>; 2]`.
6. **Instance material** is in scope: `Instance.material: Option<MaterialId>` (SketchUp behavior). It shows on every face side inside the instance, at any depth, that has no material of its own; the nearest painted instance wins (innermost). Painting an instance does not touch the definition's faces, so other placements of a component keep their look. `Instance::new(definition, transform)` is unpainted.
7. **Undo-exact ops.** New `Op::PaintFace(FaceId, [Option<MaterialId>;2] from, to)` and `Op::PaintInstance(InstanceId, Option from, Option to)` (summary: `modified`). New `Step::AddMaterial`, `RemoveMaterial`, `ChangeMaterial(id, from, to)`. `Model::set_paint`/`set_instance_material` record only when something differs, so painting what is already painted is an empty change (no undo step, no revision bump).
8. **`ChangeSummary.materials: Vec<MaterialId>`** (serde default, skipped when empty): materials added, removed or changed by a change (net: added-then-removed omitted). This is how `add_material` reports the new id (`changes.materials[0]`) and how subscribers/events see material edits. Chosen over a separate `Outcome.material` so `changed` notifications and `events` carry it too.
9. **Delete** = `Scene::delete_material`: first unpaints every face side and instance using it in every mesh (recorded paint ops), then removes it; one undo step restores both. No separate purge-unused-materials command (deferred).
10. **Paint follows geometry:**
    - split (edge across a face, loop drawn on a face): every piece keeps the face's front/back (`rebuild_plane` gives a new region the paint of the old face containing its interior point, as it already did for the normal);
    - merge by erasing the seam: the merged face gets the paint of the old face its interior point lies in (one of the two, as for orientation);
    - a face drawn inside an opening (no containing face) is unpainted;
    - a face whose holes change keeps its paint (`..f.clone()`); weld and heal keep paint (they edit the face struct);
    - move/rotate keep paint (faces keep id/data);
    - copies (`copy`, `copy_placed`), `make_group`, `duplicate` (copied groups) keep paint: `StencilFace` carries `paint`, set after `orient` on every new piece of a stencil face;
    - copies of instances keep the instance material;
    - push/pull extrusion: the cap and every side face (also recess floor and walls) get the pushed face's front/back paint (SketchUp gives new extrusion faces the pushed face's material); a base that turns over (`orient` via `Face::reversed`) keeps each material on its physical side, so a red-front sheet pulled up has a base with back = red, front = default; a moved (stretched) face keeps its paint;
    - explode: the instance material passes to the face sides that come out without one (`Stencil::painted_over`) and to inner instances without one (`i.material.or(placed.material)`).
11. **Commands** (each one undo step, through `Document::edit`, so atomic batches work): `add_material {name?, color}`, `change_material {material, name?, color?}` (unchanged = no step), `delete_material {material}`, `paint {ids, material?, side?}`. `paint` takes faces and instances of the open context; `side` is `front` (default), `back` or `both` (ignored for instances); `material` absent or null = default material. Errors: empty ids, vertex ids, edge or guide ids -> `invalid_input` (listing the edge/guide/vertex ids); unknown entity -> `not_found` with ids; unknown material -> `not_found` "no material m999"; bad name -> `invalid_input` with the reason.
12. **Queries:** `{"type":"materials"}` -> `{materials: [{id, name, color, faces, instances}], revision}` (counts over the whole model, each definition once). `FaceInfo` gains `front_material`/`back_material`, `InstanceInfo` gains `material` (all omitted when None).
13. **Files:** `.skr` schema 5. `SceneData.materials: Vec<MaterialData {id, name, color}>` (omitted when empty), `FaceData.front/back: Option<u64>`, `InstanceData.material: Option<u64>`, all serde-default, so v1-v4 files open unpainted. Validation: material ids <= MAX_ID and not repeated ("appears twice"), names valid and distinct, every painted face side/instance names an existing material ("painted with a missing material"). The existing `deny_unknown_fields` keeps rejecting stray fields.
14. **Render:** `FaceVertex` gains `front: [f32;3]`, `back: [f32;3]` (vertex attributes 3 and 4); the shader picks by `front_facing`, keeps the headlight shading and the 50% blue selection tint on top. Default colors moved from WGSL to `render::scene::DEFAULT_FRONT`/`DEFAULT_BACK` (same values). Material colors are used as-is (`u8/255`): the targets are non-sRGB `Unorm`, the same space as the old constants. Recoloring bumps the revision, so the app's existing revision check rebuilds buffers.
15. **Export:** `kernel::Triangle` gains `front`/`back` (resolved: face's own, else nearest painted instance). Only the front side is exported (formats have one material per triangle). OBJ: when any triangle has a material, writes `<stem>.mtl` beside the OBJ (same safe `replace_file`, written before the OBJ), `mtllib <stem>.mtl`, `usemtl <name>` before each material's faces, `newmtl <name>` + `Kd r g b` (linear, 6 decimals); names sanitized (non-alphanumeric except `-_.` -> `_`, numbered `_2` on collision); unpainted faces use `default` (front default color). Without painted faces the OBJ is byte-for-byte as before and no `.mtl` is written. Refused (`invalid_input`) when the `.mtl` path is the export path itself (e.g. `x.mtl` with format obj) or the open model's file. glTF/GLB: one primitive per material (groups in order of first use), shared position/normal buffer views with per-primitive accessors (`byteOffset` always present), `materials[k]` = `{name, doubleSided: true, pbrMetallicRoughness: {metallicFactor: 0, baseColorFactor: [linear r,g,b,1]}}` (default material has no baseColorFactor and is named `default`). STL unchanged.
16. **Paint tool:** `ToolKind::Paint`, label/name "Paint" (SketchUp's "Paint Bucket" is too wide for the toolbar), shortcut `b` (free), last toolbar button. Target = `face_under_in` only (edges are not targets): an open-context face paints the side facing the viewer (`camera.ray(cursor).dir . normal > 0` -> back); an instance under the cursor paints the instance; outside geometry is ignored. A click on a selected face/instance paints every selected face and instance (on the clicked side). Hover outlines the face (`face_outline`) or the instance box (`box_outline`). Typed values are refused with a message.
17. **Sampling (eyedropper):** Cmd (macOS) / Ctrl held + click picks up the clicked side's own material (instance: its material; None = default) into the current material; the status says "Painting with <name>". Alt is not used because Alt+left-drag already orbits (`Navigation`); Cmd is SketchUp-for-Mac's sampling modifier. Ctrl's tap-to-toggle-copy is untouched (returns false outside Move/Rotate/Tape).
18. **Current material** is app state (`Editor.material`), passed to tools as `ToolContext.material`; `Editor::current_material()` returns None when the chosen material no longer exists (deleted, undone). Tools report a sample through `ToolOutput.sampled: Option<Option<MaterialId>>`.
19. **UI:** no new menu; a Materials section in the existing right panel under Entity Info: "Default" row plus one row per material (16 pt swatch + selectable name; clicking either chooses it and switches to Paint, as choosing in SketchUp's Materials browser does), a scroll area capped at 220 pt, Add (name "Material N", next color of a 12-color palette `MATERIAL_PALETTE`, chosen at once) and Delete (enabled for a non-default current material); for the current material a name field (kept in egui temp memory while focused; renames when focus is lost with a changed name, e.g. Enter) and the 12-color palette (click = recolor, one undo step). No free color picker (a drag-picker would make an undo step per frame; deferred). Entity Info shows "Front: X"/"Back: Y" for one face and "Material: X" for one painted instance.
20. **Toolbar width:** with Paint the toolbar ends at about 1240 pt of 1280 (measured: Close group's right edge clickable at x = 1239). Still fits; about 40 pt left. The next button needs the deferred toolbar wrapping.

## File Structure

| File | Responsibility |
|---|---|
| `crates/kernel/src/material.rs` (new) | Material, Side, name rules; Model paint_faces/paint_instances/unpaint; Scene add/change/delete/paint materials, free_material_name |
| `crates/kernel/src/ids.rs` | MaterialId ("m<n>") |
| `crates/kernel/src/model.rs` | Face front/back (new, reversed, paint), Instance material (new), Op::PaintFace/PaintInstance, ChangeSummary.materials, set_paint, set_instance_material |
| `crates/kernel/src/scene.rs` | material table, Step::Add/Remove/ChangeMaterial, summary materials, validation of materials and references |
| `crates/kernel/src/faces.rs` | new regions take the containing face's paint |
| `crates/kernel/src/push_pull.rs` | cap and sides take the pushed face's paint; orient keeps paint on its side |
| `crates/kernel/src/transform.rs` | stencil faces carry paint; `Stencil::painted_over` for explode |
| `crates/kernel/src/group.rs` | instance material kept by copies, passed on by explode |
| `crates/kernel/src/data.rs` | MaterialData, face/instance material fields |
| `crates/kernel/src/triangles.rs` | Triangle front/back with instance inheritance |
| `crates/kernel/tests/materials.rs` (new) | kernel material tests |
| `crates/session/src/document/materials.rs` (new) | add/change/delete material and paint commands, materials query |
| `crates/session/src/command.rs`, `query.rs` | material commands, Materials query/result, face/instance material fields |
| `crates/session/src/file.rs` | schema 5 |
| `crates/session/src/export.rs`, `document/files.rs` | per-material OBJ/MTL and glTF |
| `crates/session/tests/materials.rs` (new) | command-driven material tests |
| `crates/render/src/scene.rs`, `renderer.rs`, `shaders/scene.wgsl` | per-side colors |
| `crates/tools/src/paint.rs` (new) | Paint tool |
| `crates/tools/src/tool.rs` | ToolContext material/sample, ToolOutput sampled, ToolKind::Paint |
| `crates/app/src/editor.rs` | current material, material actions, palette |
| `crates/app/src/ui.rs` | Materials list in the right panel |
| `crates/app/src/info.rs` | face and instance materials in Entity Info |
| `crates/app/src/app.rs` | material rows into the view, UI actions, sampling modifier |

The code below was written and verified in a scratch copy of `dev` at 67871a7. Each task was checked on its own with `cargo fmt --all --check`, clippy with `-D warnings`, and the full test suite. Workspace test totals after each task: 693, 700, 705, 711 (from 684).

Code for new files is given in full. Changes to existing files are given as unified diffs against the state the previous task leaves.

---

### Task 1: Kernel materials

**Files:**
- Create: `crates/kernel/src/material.rs` (Material, Side, check_name, Model::paint_faces/paint_instances/unpaint, Scene::add_material/change_material/delete_material/paint/free_material_name/check_free_name)
- Create (test): `crates/kernel/tests/materials.rs` (9 tests)
- Modify: `crates/kernel/src/ids.rs` (MaterialId + Display/FromStr/JsonSchema/String conversions; id test hunk adds m-id asserts), `lib.rs` (mod material; exports MaterialId, Material, Side, MAX_MATERIAL_NAME, MaterialData), `model.rs` (Face front/back + new/reversed/paint; Instance material + new; Op::PaintFace/PaintInstance in inverse/entity/apply_op; ChangeSummary.materials; set_paint, set_instance_material; test literals -> Face::new), `scene.rs` (materials field, Step::Add/Remove/ChangeMaterial, summary materials, from_meshes(materials), material(), materials(), alloc_material(), apply_step pub(crate), validate materials and references, Instance::new in instance_guide_anchors), `faces.rs` (wanted regions carry paint; reconcile sets it; `..f.clone()` for hole updates; test uses `f.reversed()`), `push_pull.rs` (orient uses `Face::reversed`; cap and sides `set_paint(f, face.paint())`; doc), `transform.rs` (StencilFace.paint; set_paint after orient; `Stencil::painted_over`), `group.rs` (Instance::new; explode stencil painted_over + child material; copies keep material), `weld.rs` (`..f`), `edit.rs` (KernelError::MaterialNotFound, MaterialName(String)), `data.rs` (MaterialData, SceneData.materials, FaceData.front/back, InstanceData.material, to_data/from_data)
- Modify (test hunk): `crates/kernel/tests/data.rs` (literals get `material: None` / `materials: vec![]`)
- Modify: `crates/session/src/document.rs` (one `Instance { .. }` literal -> `Instance::new(i.definition, *t * i.transform)`; needed to compile)

**Interfaces:**
- Produces:
  - `pub struct MaterialId(pub u64)` (serde as "m<n>"), `pub struct Material { pub name: String, pub color: [u8; 3] }`, `pub enum Side { Front (default), Back, Both }` (serde snake_case, JsonSchema), `pub const MAX_MATERIAL_NAME: usize = 256`
  - `Face { outer, holes, normal, front: Option<MaterialId>, back: Option<MaterialId> }`; `Face::new(Vec<VertexId>, Vec<Vec<VertexId>>, DVec3) -> Face`; `Face::reversed(&self) -> Face`; `Face::paint(&self) -> [Option<MaterialId>; 2]`
  - `Instance { definition, transform, material: Option<MaterialId> }`; `Instance::new(DefinitionId, DAffine3) -> Instance`
  - `Op::PaintFace(FaceId, [Option<MaterialId>; 2], [Option<MaterialId>; 2])`, `Op::PaintInstance(InstanceId, Option<MaterialId>, Option<MaterialId>)`
  - `Step::AddMaterial(MaterialId, Material)`, `Step::RemoveMaterial(MaterialId, Material)`, `Step::ChangeMaterial(MaterialId, Material, Material)`
  - `ChangeSummary.materials: Vec<MaterialId>`
  - `Model::paint_faces(&mut self, faces: &[FaceId], side: Side, material: Option<MaterialId>) -> ChangeSet`; `Model::paint_instances(&mut self, instances: &[InstanceId], material: Option<MaterialId>) -> ChangeSet`; `pub(crate) Model::set_paint(&mut self, FaceId, [Option<MaterialId>; 2])`, `set_instance_material(&mut self, InstanceId, Option<MaterialId>)`, `unpaint(&mut self, MaterialId) -> ChangeSet`
  - `Scene::material(&self, MaterialId) -> Option<&Material>`; `Scene::materials(&self) -> impl Iterator<Item = (MaterialId, &Material)>`; `Scene::add_material(&mut self, name: &str, color: [u8; 3]) -> Result<(MaterialId, SceneChange), KernelError>`; `Scene::change_material(&mut self, id: MaterialId, name: Option<&str>, color: Option<[u8; 3]>) -> Result<SceneChange, KernelError>`; `Scene::delete_material(&mut self, id: MaterialId) -> Result<SceneChange, KernelError>`; `Scene::paint(&mut self, mesh: MeshId, faces: &[FaceId], instances: &[InstanceId], side: Side, material: Option<MaterialId>) -> Result<SceneChange, KernelError>`; `Scene::free_material_name(&self, base: &str) -> String`
  - `KernelError::MaterialNotFound`, `KernelError::MaterialName(String)`
  - `MaterialData { id: u64, name: String, color: [u8; 3] }`; `SceneData.materials`, `FaceData.front/back: Option<u64>`, `InstanceData.material: Option<u64>`

Consumes: `Scene::edit`, `Model::record`/`take_changes`, `rebuild_plane`, `draw_stencil`, `orient`.

- [ ] **Step 1: Write the failing tests**

Tests: new integration file `crates/kernel/tests/materials.rs`: `materials_are_added_changed_and_deleted_as_exact_undo_steps`, `material_names_must_be_present_short_and_distinct`, `painting_faces_and_instances_undoes_exactly`, `deleting_a_material_unpaints_everything_painted_with_it`, `pieces_of_a_split_face_keep_its_paint_and_a_merge_keeps_it_too`, `a_face_drawn_in_an_opening_is_not_painted`, `pulled_faces_give_their_paint_to_the_cap_and_sides`, `copies_groups_and_explodes_carry_paint`, `materials_survive_files_and_bad_material_data_is_rejected`. Test-module hunk in `ids.rs` (m-id parse/display).

Test-module hunks are part of the Step 3 diffs; apply them first, along with the test files below.

Apply to `crates/kernel/tests/data.rs`:
```diff
--- a/crates/kernel/tests/data.rs
+++ b/crates/kernel/tests/data.rs
@@ -130,8 +130,9 @@ fn broken_data_is_rejected() {
             group.mesh.instances.push(kernel::InstanceData {
                 id: 999_999,
                 definition: component,
                 transform: glam::DAffine3::IDENTITY.to_cols_array(),
+                material: None,
             });
         },
         "places itself",
     );
@@ -185,8 +186,9 @@ fn loose_vertices(next_id: u64, vertices: &[(u64, [f64; 3])]) -> SceneData {
                 .collect(),
             ..Default::default()
         },
         definitions: vec![],
+        materials: vec![],
     }
 }
 
 #[test]
@@ -260,8 +262,9 @@ fn nested(n: u64, cycle: bool) -> SceneData {
     let instance = |id: u64, definition: u64| kernel::InstanceData {
         id,
         definition,
         transform: glam::DAffine3::IDENTITY.to_cols_array(),
+        material: None,
     };
     let definitions = (0..n)
         .map(|k| kernel::DefinitionData {
             id: k + 1,
@@ -285,8 +288,9 @@ fn nested(n: u64, cycle: bool) -> SceneData {
             instances: vec![instance(n + 1, 1)],
             ..Default::default()
         },
         definitions,
+        materials: vec![],
     }
 }
 
 #[test]
```

Create `crates/kernel/tests/materials.rs`:
```rust
//! Materials: adding, changing and deleting them, painting faces and instances, and how paint
//! follows geometry through splits, merges, push/pull, copies, groups, explodes and files.

use glam::DVec3;
use kernel::{
    DefinitionKind, EntityId, FaceId, InstanceId, KernelError, MaterialData, MaterialId, MeshId,
    Model, Scene, SceneData, Side,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

const RED: [u8; 3] = [200, 30, 30];
const BLUE: [u8; 3] = [30, 30, 200];

/// A 2 x 2 square on the ground (facing up), in a scene of its own.
fn square_scene() -> (Scene, FaceId) {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
    ])
    .unwrap();
    let face = m.faces().next().unwrap().0;
    (Scene::from_root(m), face)
}

fn red(s: &mut Scene) -> MaterialId {
    s.add_material("Red", RED).unwrap().0
}

#[test]
fn materials_are_added_changed_and_deleted_as_exact_undo_steps() {
    let (mut s, _) = square_scene();
    let empty = s.clone();
    let (id, added) = s.add_material("Brick", RED).unwrap();
    assert_eq!(s.material(id).unwrap().name, "Brick");
    assert_eq!(s.material(id).unwrap().color, RED);
    assert_eq!(added.summary().materials, vec![id]);
    assert!(added.summary().created.is_empty());
    // Material ids come from the entity id counter: never one an entity has.
    assert!(s.root().vertices().all(|(v, _)| v.0 != id.0));

    let before = s.clone();
    let changed = s
        .change_material(id, Some("Old brick"), Some(BLUE))
        .unwrap();
    assert_eq!(s.material(id).unwrap().name, "Old brick");
    assert_eq!(s.material(id).unwrap().color, BLUE);
    assert_eq!(changed.summary().materials, vec![id]);
    assert!(s.change_material(id, None, Some(BLUE)).unwrap().is_empty());
    s.apply(&changed.inverse());
    assert_eq!(s, before);

    let deleted = s.delete_material(id).unwrap();
    assert!(s.material(id).is_none());
    s.apply(&deleted.inverse());
    assert_eq!(s, before);
    s.apply(&added.inverse());
    assert_eq!(s, empty);
}

#[test]
fn material_names_must_be_present_short_and_distinct() {
    let (mut s, _) = square_scene();
    let id = red(&mut s);
    for bad in ["", "   ", "Red"] {
        assert!(
            matches!(s.add_material(bad, BLUE), Err(KernelError::MaterialName(_))),
            "{bad:?} accepted"
        );
    }
    let long = "x".repeat(kernel::MAX_MATERIAL_NAME + 1);
    assert!(matches!(
        s.add_material(&long, BLUE),
        Err(KernelError::MaterialName(_))
    ));
    let blue = s.add_material("Blue", BLUE).unwrap().0;
    assert!(matches!(
        s.change_material(blue, Some("Red"), None),
        Err(KernelError::MaterialName(_))
    ));
    // Keeping its own name is fine.
    assert!(s.change_material(id, Some("Red"), None).unwrap().is_empty());
    assert_eq!(s.free_material_name("Red"), "Red 2");
    assert_eq!(s.free_material_name("Green"), "Green");
    assert_eq!(
        s.change_material(MaterialId(999), None, None),
        Err(KernelError::MaterialNotFound)
    );
    assert_eq!(
        s.delete_material(MaterialId(999)),
        Err(KernelError::MaterialNotFound)
    );
}

#[test]
fn painting_faces_and_instances_undoes_exactly() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    let before = s.clone();
    let front = s
        .paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    assert_eq!(s.root().face(face).unwrap().paint(), [Some(id), None]);
    assert_eq!(front.summary().modified, vec![EntityId::Face(face)]);
    let back = s
        .paint(MeshId::Root, &[face], &[], Side::Back, Some(id))
        .unwrap();
    assert_eq!(s.root().face(face).unwrap().paint(), [Some(id), Some(id)]);
    assert!(
        s.paint(MeshId::Root, &[face], &[], Side::Both, Some(id))
            .unwrap()
            .is_empty(),
        "already painted"
    );
    let cleared = s
        .paint(MeshId::Root, &[face], &[], Side::Both, None)
        .unwrap();
    assert_eq!(s.root().face(face).unwrap().paint(), [None, None]);
    s.apply(&cleared.inverse());
    s.apply(&back.inverse());
    s.apply(&front.inverse());
    assert_eq!(s, before);
    assert_eq!(
        s.paint(
            MeshId::Root,
            &[face],
            &[],
            Side::Front,
            Some(MaterialId(999))
        ),
        Err(KernelError::MaterialNotFound)
    );

    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    let (group, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[face],
            &[],
            DefinitionKind::Group,
            "",
        )
        .unwrap();
    let painted = s
        .paint(MeshId::Root, &[], &[group], Side::Back, Some(id))
        .unwrap();
    assert_eq!(s.root().instance(group).unwrap().material, Some(id));
    assert_eq!(painted.summary().modified, vec![EntityId::Instance(group)]);
    s.validate().unwrap();
}

#[test]
fn deleting_a_material_unpaints_everything_painted_with_it() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    let blue = s.add_material("Blue", BLUE).unwrap().0;
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    s.paint(MeshId::Root, &[face], &[], Side::Back, Some(blue))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    let (group, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[face],
            &[],
            DefinitionKind::Group,
            "",
        )
        .unwrap();
    s.paint(MeshId::Root, &[], &[group], Side::Front, Some(id))
        .unwrap();
    let before = s.clone();
    let change = s.delete_material(id).unwrap();
    s.validate().unwrap();
    assert_eq!(s.root().instance(group).unwrap().material, None);
    let d = s.root().instance(group).unwrap().definition;
    let inner = s.definition(d).unwrap().mesh.faces().next().unwrap().1;
    assert_eq!(
        inner.paint(),
        [None, Some(blue)],
        "the other side keeps blue"
    );
    assert!(change.summary().materials == vec![id]);
    s.apply(&change.inverse());
    assert_eq!(s, before);
}

#[test]
fn pieces_of_a_split_face_keep_its_paint_and_a_merge_keeps_it_too() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    let split = s
        .edit(MeshId::Root, |m| {
            m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0))
        })
        .unwrap();
    assert_eq!(s.root().faces().count(), 2);
    assert!(s.root().faces().all(|(_, f)| f.paint() == [Some(id), None]));
    let seam = split
        .summary()
        .created
        .iter()
        .find_map(|e| match e {
            EntityId::Edge(e)
                if s.root().edge(*e).is_some_and(|e| {
                    e.v.iter().all(|v| s.root().position(*v).unwrap().x == 1.0)
                }) =>
            {
                Some(*e)
            }
            _ => None,
        })
        .unwrap();
    s.edit(MeshId::Root, |m| Ok(m.erase(&[seam], &[]))).unwrap();
    assert_eq!(s.root().faces().count(), 1);
    assert_eq!(s.root().faces().next().unwrap().1.paint(), [Some(id), None]);
    s.validate().unwrap();
}

#[test]
fn a_face_drawn_in_an_opening_is_not_painted() {
    // A loop drawn on a painted face splits it: the inner piece keeps the paint. Erasing that
    // piece and drawing inside the opening gives a new, unpainted face.
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&[
            p(0.5, 0.5, 0.0),
            p(1.5, 0.5, 0.0),
            p(1.5, 1.5, 0.0),
            p(0.5, 1.5, 0.0),
        ])
    })
    .unwrap();
    assert!(s.root().faces().all(|(_, f)| f.front == Some(id)));
    let inner = s
        .root()
        .faces()
        .find(|(_, f)| f.holes.is_empty())
        .unwrap()
        .0;
    s.edit(MeshId::Root, |m| Ok(m.erase(&[], &[inner])))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&[
            p(0.75, 0.75, 0.0),
            p(1.25, 0.75, 0.0),
            p(1.25, 1.25, 0.0),
            p(0.75, 1.25, 0.0),
        ])
    })
    .unwrap();
    let fresh = s
        .root()
        .faces()
        .find(|(_, f)| f.holes.is_empty())
        .unwrap()
        .1;
    assert_eq!(fresh.paint(), [None, None]);
}

#[test]
fn pulled_faces_give_their_paint_to_the_cap_and_sides() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    let blue = s.add_material("Blue", BLUE).unwrap().0;
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    s.paint(MeshId::Root, &[face], &[], Side::Back, Some(blue))
        .unwrap();
    s.edit(MeshId::Root, |m| m.push_pull(face, 1.0)).unwrap();
    s.validate().unwrap();
    let m = s.root();
    assert_eq!(m.faces().count(), 6);
    // The base turned over to face down: each material stays on its side, so blue (what was
    // below) is now its front.
    let base = m.face(face).unwrap();
    assert_eq!(base.normal, -DVec3::Z);
    assert_eq!(base.paint(), [Some(blue), Some(id)]);
    for (fid, f) in m.faces().filter(|(f, _)| *f != face) {
        assert_eq!(f.paint(), [Some(id), Some(blue)], "{fid:?}");
    }

    // A recess pushed into the painted top: its floor and walls take the top's paint.
    let top = m.faces().find(|(_, f)| f.normal == DVec3::Z).unwrap().0;
    let green = s.add_material("Green", [0, 200, 0]).unwrap().0;
    s.paint(MeshId::Root, &[top], &[], Side::Both, Some(green))
        .unwrap();
    s.edit(MeshId::Root, |m| {
        m.add_polygon(&[
            p(0.5, 0.5, 1.0),
            p(1.5, 0.5, 1.0),
            p(1.5, 1.5, 1.0),
            p(0.5, 1.5, 1.0),
        ])
    })
    .unwrap();
    let inner = s
        .root()
        .faces()
        .find(|(_, f)| f.normal == DVec3::Z && f.holes.is_empty() && f.outer.len() == 4)
        .filter(|(id, _)| *id != top)
        .unwrap()
        .0;
    s.edit(MeshId::Root, |m| m.push_pull(inner, -0.5)).unwrap();
    s.validate().unwrap();
    let m = s.root();
    let recess: Vec<_> = m
        .faces()
        .filter(|(_, f)| {
            f.outer
                .iter()
                .all(|v| m.position(*v).unwrap().z >= 0.5 - 1e-9)
                && f.outer
                    .iter()
                    .any(|v| m.position(*v).unwrap().z < 1.0 - 1e-9)
        })
        .collect();
    assert_eq!(recess.len(), 5, "floor and four walls");
    assert!(
        recess
            .iter()
            .all(|(_, f)| f.paint() == [Some(green), Some(green)])
    );
}

#[test]
fn copies_groups_and_explodes_carry_paint() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    s.paint(MeshId::Root, &[face], &[], Side::Front, Some(id))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    s.edit(MeshId::Root, |m| {
        m.copy(&edges, &[face], p(5.0, 0.0, 0.0), 1)
    })
    .unwrap();
    assert_eq!(s.root().faces().count(), 2);
    assert!(s.root().faces().all(|(_, f)| f.front == Some(id)));

    // Group the copy (the face stays painted inside), paint the group blue, copy the group:
    // the copy is blue too.
    let copy = s
        .root()
        .faces()
        .map(|(f, _)| f)
        .find(|f| *f != face)
        .unwrap();
    let copy_edges: Vec<_> = {
        let m = s.root();
        m.edges()
            .filter(|(_, e)| e.v.iter().all(|v| m.position(*v).unwrap().x >= 5.0))
            .map(|(e, _)| e)
            .collect()
    };
    let (group, _) = s
        .make_group(
            MeshId::Root,
            &copy_edges,
            &[copy],
            &[],
            DefinitionKind::Group,
            "",
        )
        .unwrap();
    let d = s.root().instance(group).unwrap().definition;
    assert!(
        s.definition(d)
            .unwrap()
            .mesh
            .faces()
            .all(|(_, f)| f.front == Some(id))
    );
    let blue = s.add_material("Blue", BLUE).unwrap().0;
    s.paint(MeshId::Root, &[], &[group], Side::Front, Some(blue))
        .unwrap();
    s.copy_instances(MeshId::Root, &[group], p(0.0, 5.0, 0.0), 1)
        .unwrap();
    let copied: InstanceId = s
        .root()
        .instances()
        .map(|(i, _)| i)
        .find(|i| *i != group)
        .unwrap();
    assert_eq!(s.root().instance(copied).unwrap().material, Some(blue));

    // Exploding the copy: its face keeps red on the front and takes the group's blue on the
    // back, which had no material.
    s.explode(MeshId::Root, copied).unwrap();
    s.validate().unwrap();
    let exploded = s
        .root()
        .faces()
        .find(|(_, f)| {
            f.outer
                .iter()
                .all(|v| s.root().position(*v).unwrap().y >= 5.0)
        })
        .unwrap()
        .1;
    assert_eq!(exploded.paint(), [Some(id), Some(blue)]);
}

#[test]
fn materials_survive_files_and_bad_material_data_is_rejected() {
    let (mut s, face) = square_scene();
    let id = red(&mut s);
    s.paint(MeshId::Root, &[face], &[], Side::Back, Some(id))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
    let (group, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[face],
            &[],
            DefinitionKind::Group,
            "",
        )
        .unwrap();
    s.paint(MeshId::Root, &[], &[group], Side::Front, Some(id))
        .unwrap();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    let data: SceneData = serde_json::from_str(&json).unwrap();
    let back = Scene::from_data(&data).unwrap();
    assert_eq!(back, s);
    // New ids continue after the material's.
    let mut next = back.clone();
    let (later, _) = next.add_material("Blue", BLUE).unwrap();
    assert!(later.0 > id.0);

    let rejects = |f: &dyn Fn(&mut SceneData), expected: &str| {
        let mut d = s.to_data();
        f(&mut d);
        let err = Scene::from_data(&d).unwrap_err();
        assert!(err.contains(expected), "{err:?} lacks {expected:?}");
    };
    rejects(
        &|d| d.root.instances[0].material = Some(12345),
        "missing material",
    );
    rejects(
        &|d| d.definitions[0].mesh.faces[0].front = Some(12345),
        "missing material",
    );
    rejects(&|d| d.materials[0].name = " ".into(), "needs a name");
    rejects(
        &|d| {
            let m = d.materials[0].clone();
            d.materials.push(MaterialData { id: 777, ..m });
        },
        "two materials are named",
    );
    rejects(
        &|d| {
            let m = d.materials[0].clone();
            d.materials.push(MaterialData {
                name: "Other".into(),
                ..m
            });
        },
        "appears twice",
    );
    rejects(&|d| d.materials[0].id = u64::MAX, "too large");
    // A color component over 255 is not a material color.
    let bad = json.replace("[200,30,30]", "[256,30,30]");
    assert!(serde_json::from_str::<SceneData>(&bad).is_err());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

`cargo test -p kernel --test materials` fails to compile (no `MaterialId`, `Side`, `add_material`, ...). Verified semantic red: with the two `set_paint(f, face.paint())` lines in push_pull.rs and `set_paint(id, f.paint)` in transform.rs removed, 3 tests fail (`pulled_faces_give_their_paint_to_the_cap_and_sides`, `copies_groups_and_explodes_carry_paint`, `deleting_a_material_unpaints_everything_painted_with_it`).

- [ ] **Step 3: Implement**

Apply to `crates/kernel/src/data.rs`:
```diff
--- a/crates/kernel/src/data.rs
+++ b/crates/kernel/src/data.rs
@@ -4,9 +4,12 @@ use std::collections::BTreeMap;
 
 use glam::{DAffine3, DVec3};
 use serde::{Deserialize, Serialize};
 
-use crate::ids::{CurveId, DefinitionId, EdgeId, FaceId, GuideId, InstanceId, VertexId};
+use crate::ids::{
+    CurveId, DefinitionId, EdgeId, FaceId, GuideId, InstanceId, MaterialId, VertexId,
+};
+use crate::material::Material;
 use crate::model::{Edge, Face, Guide, Instance, Model, Vertex};
 use crate::scene::{Definition, DefinitionKind, Scene};
 
 /// The largest id a file may use: every id up to it is exact as a JSON (double) number.
@@ -23,8 +26,20 @@ pub struct SceneData {
     /// The id the next new entity gets; never reused.
     pub next_id: u64,
     pub root: MeshData,
     pub definitions: Vec<DefinitionData>,
+    /// Absent in files written before materials existed (and when there are none).
+    #[serde(default, skip_serializing_if = "Vec::is_empty")]
+    pub materials: Vec<MaterialData>,
+}
+
+/// A material: `color` is sRGB red, green and blue.
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
+#[serde(deny_unknown_fields)]
+pub struct MaterialData {
+    pub id: u64,
+    pub name: String,
+    pub color: [u8; 3],
 }
 
 #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
 #[serde(deny_unknown_fields)]
@@ -64,8 +79,13 @@ pub struct FaceData {
     pub id: u64,
     pub outer: Vec<u64>,
     pub holes: Vec<Vec<u64>>,
     pub normal: [f64; 3],
+    /// The materials of the front and back sides (absent: the default material).
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub front: Option<u64>,
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub back: Option<u64>,
 }
 
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 #[serde(deny_unknown_fields)]
@@ -73,8 +93,11 @@ pub struct InstanceData {
     pub id: u64,
     pub definition: u64,
     /// Column-major 3x4: x axis, y axis, z axis, translation.
     pub transform: [f64; 12],
+    /// Absent when the instance is not painted.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub material: Option<u64>,
 }
 
 /// A guide line through `point` along `direction` (a unit vector), or, without `direction`, a
 /// guide point at `point`.
@@ -110,14 +133,24 @@ impl Scene {
                     kind: d.kind,
                     mesh: mesh_data(&d.mesh),
                 })
                 .collect(),
+            materials: self
+                .materials()
+                .map(|(id, m)| MaterialData {
+                    id: id.0,
+                    name: m.name.clone(),
+                    color: m.color,
+                })
+                .collect(),
         }
     }
 
     /// Rebuilds a scene from plain data, keeping every id. Fails, naming the problem, when an
     /// id is used twice or is over [`MAX_ID`], a number is not finite, an instance's transform
-    /// cannot be inverted, or the result breaks a model invariant (see [`Scene::validate`]).
+    /// cannot be inverted, or the result breaks a model invariant (see [`Scene::validate`]:
+    /// that includes painting with a missing material and blank, overlong or repeated
+    /// material names).
     pub fn from_data(data: &SceneData) -> Result<Scene, String> {
         if data.next_id > MAX_ID {
             return Err(format!(
                 "next_id {} is too large (over {MAX_ID})",
@@ -136,9 +169,20 @@ impl Scene {
             if definitions.insert(DefinitionId(d.id), definition).is_some() {
                 return Err(format!("definition d{} appears twice", d.id));
             }
         }
-        let scene = Scene::from_meshes(root, definitions, data.next_id);
+        let mut materials = BTreeMap::new();
+        for m in &data.materials {
+            check_id("material m", m.id)?;
+            let material = Material {
+                name: m.name.clone(),
+                color: m.color,
+            };
+            if materials.insert(MaterialId(m.id), material).is_some() {
+                return Err(format!("material m{} appears twice", m.id));
+            }
+        }
+        let scene = Scene::from_meshes(root, definitions, materials, data.next_id);
         scene.validate()?;
         Ok(scene)
     }
 }
@@ -171,16 +215,19 @@ fn mesh_data(m: &Model) -> MeshData {
                     .iter()
                     .map(|h| h.iter().map(|v| v.0).collect())
                     .collect(),
                 normal: f.normal.to_array(),
+                front: f.front.map(|m| m.0),
+                back: f.back.map(|m| m.0),
             })
             .collect(),
         instances: m
             .instances()
             .map(|(id, i)| InstanceData {
                 id: id.0,
                 definition: i.definition.0,
                 transform: i.transform.to_cols_array(),
+                material: i.material.map(|m| m.0),
             })
             .collect(),
         guides: m
             .guides()
@@ -249,8 +296,10 @@ fn mesh(data: &MeshData) -> Result<Model, String> {
         let face = Face {
             outer: ring(&f.outer),
             holes: f.holes.iter().map(ring).collect(),
             normal: DVec3::from(f.normal),
+            front: f.front.map(MaterialId),
+            back: f.back.map(MaterialId),
         };
         if faces.insert(FaceId(f.id), face).is_some() {
             return Err(format!("face f{} appears twice", f.id));
         }
@@ -268,8 +317,9 @@ fn mesh(data: &MeshData) -> Result<Model, String> {
         }
         let instance = Instance {
             definition: DefinitionId(i.definition),
             transform,
+            material: i.material.map(MaterialId),
         };
         if instances.insert(InstanceId(i.id), instance).is_some() {
             return Err(format!("instance i{} appears twice", i.id));
         }
```

Apply to `crates/kernel/src/edit.rs`:
```diff
--- a/crates/kernel/src/edit.rs
+++ b/crates/kernel/src/edit.rs
@@ -24,8 +24,12 @@ pub enum KernelError {
     /// An instance id names no instance in the mesh.
     InstanceNotFound,
     /// A definition id names no definition in the scene.
     DefinitionNotFound,
+    /// A material id names no material in the scene.
+    MaterialNotFound,
+    /// A material name is empty, too long or taken; the text says which.
+    MaterialName(String),
 }
 
 impl fmt::Display for KernelError {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
@@ -51,8 +55,10 @@ impl fmt::Display for KernelError {
                 "the move would collapse an edge, turn a face over, or land a vertex on another"
             ),
             KernelError::InstanceNotFound => write!(f, "no such instance"),
             KernelError::DefinitionNotFound => write!(f, "no such definition"),
+            KernelError::MaterialNotFound => write!(f, "no such material"),
+            KernelError::MaterialName(why) => write!(f, "{why}"),
         }
     }
 }
 
```

Apply to `crates/kernel/src/faces.rs`:
```diff
--- a/crates/kernel/src/faces.rs
+++ b/crates/kernel/src/faces.rs
@@ -5,9 +5,9 @@ use std::collections::{BTreeMap, BTreeSet};
 
 use glam::{DVec2, DVec3};
 
 use crate::geom::{PLANE_ANGLE_TOL, Plane, point_in_polygon};
-use crate::ids::{EdgeId, FaceId, VertexId};
+use crate::ids::{EdgeId, FaceId, MaterialId, VertexId};
 use crate::model::{Face, Model};
 use crate::regions::{PlaneGraph, Region};
 
 impl Model {
@@ -102,11 +102,12 @@ impl Model {
             .iter()
             .flat_map(|(_, f)| std::iter::once(&f.outer).chain(&f.holes))
             .flat_map(|l| (0..l.len()).map(|i| side(l[i], l[(i + 1) % l.len()])))
             .collect();
-        // Each wanted region with the normal it gets if it is new: that of the old face it lies
-        // in, or the plane's own normal when it lies in none.
-        let wanted: Vec<(Region, DVec3)> = graph
+        // Each wanted region with the normal and materials it gets if it is new: those of the old
+        // face it lies in (a piece of a split face keeps its paint), or the plane's own normal
+        // and no materials when it lies in none.
+        let wanted: Vec<(Region, DVec3, Paint)> = graph
             .regions()
             .into_iter()
             .filter_map(|r| {
                 let p = graph.interior_point(&r)?;
@@ -126,20 +127,26 @@ impl Model {
                     !old_sides.contains(&side(r.outer[i], r.outer[(i + 1) % r.outer.len()]))
                 });
                 let keep = in_old_face || (uses_new && (!in_old_hole || apart));
                 let normal = container.map_or(plane.normal, |i| old[i].1.normal);
-                keep.then_some((r, normal))
+                let paint = container.map_or([None, None], |i| old[i].1.paint());
+                keep.then_some((r, normal, paint))
             })
             .collect();
 
         self.reconcile(&old, wanted, plane);
     }
 
     /// Keeps old faces that match a wanted region, updates their holes if needed, inserts the
     /// rest, and removes old faces that match nothing. All through recorded primitives.
-    fn reconcile(&mut self, old: &[(FaceId, Face)], wanted: Vec<(Region, DVec3)>, plane: Plane) {
+    fn reconcile(
+        &mut self,
+        old: &[(FaceId, Face)],
+        wanted: Vec<(Region, DVec3, Paint)>,
+        plane: Plane,
+    ) {
         let mut matched: BTreeSet<FaceId> = BTreeSet::new();
-        for (r, normal) in wanted {
+        for (r, normal, [front, back]) in wanted {
             let outer: BTreeSet<VertexId> = r.outer.iter().copied().collect();
             // Faces in one plane may face opposite ways (push/pull flips a base face), so an
             // old face matches by vertex set and keeps its own orientation.
             let same = old.iter().find(|(id, f)| {
@@ -155,9 +162,9 @@ impl Model {
                         }
                         let face = Face {
                             outer: f.outer.clone(),
                             holes,
-                            normal: f.normal,
+                            ..f.clone()
                         };
                         self.replace_face(*id, face);
                     }
                 }
@@ -173,8 +180,10 @@ impl Model {
                     self.insert_face(Face {
                         outer,
                         holes,
                         normal,
+                        front,
+                        back,
                     });
                 }
             }
         }
@@ -185,8 +194,11 @@ impl Model {
         }
     }
 }
 
+/// A face's front and back materials.
+type Paint = [Option<MaterialId>; 2];
+
 fn side(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
     (a.min(b), a.max(b))
 }
 
@@ -234,16 +246,9 @@ mod tests {
             .find(|(id, _)| m.face_area(*id) == 16.0)
             .map(|(id, f)| (id, f.clone()))
             .unwrap();
         assert_eq!(f.normal, DVec3::Z);
-        let mut outer = f.outer.clone();
-        outer.reverse();
-        let flipped = Face {
-            outer,
-            holes: vec![],
-            normal: -f.normal,
-        };
-        m.replace_face(id, flipped);
+        m.replace_face(id, f.reversed());
         m.take_changes();
         square(&mut m, 1.0, 1.0, 2.0);
         m.validate().unwrap();
         let f = m.face(id).unwrap();
```

Apply to `crates/kernel/src/group.rs`:
```diff
--- a/crates/kernel/src/group.rs
+++ b/crates/kernel/src/group.rs
@@ -40,12 +40,9 @@ impl Scene {
             for (id, _) in &moved {
                 m.remove_instance(*id);
             }
             m.detach(edges, faces);
-            placed = Some(m.insert_instance(Instance {
-                definition,
-                transform: DAffine3::IDENTITY,
-            }));
+            placed = Some(m.insert_instance(Instance::new(definition, DAffine3::IDENTITY)));
             Ok(m.take_changes())
         })?;
         let filled = self.edit(MeshId::Definition(definition), |m| {
             m.draw_stencil(&stencil, &[DAffine3::IDENTITY]);
@@ -63,9 +60,10 @@ impl Scene {
     /// Replaces an instance in mesh `parent` by its definition's geometry, placed by the
     /// instance's transform and drawn like new edges, so it merges with what it touches.
     /// Instances and guides inside it come out into `parent`, with new ids; a group inside a
     /// component gets a copy of its definition. A group's definition goes with its last
-    /// instance; a component's stays.
+    /// instance; a component's stays. The instance's material passes to the face sides and
+    /// instances that come out without one.
     pub fn explode(
         &mut self,
         parent: MeshId,
         instance: InstanceId,
@@ -81,9 +79,9 @@ impl Scene {
         // A definition that outlives this explode still places its groups, so the groups
         // lifted out of it get copies of their definitions: a group is placed only once.
         let survives = definition.kind == DefinitionKind::Component
             || self.instance_count(placed.definition) > 1;
-        let stencil = whole(&definition.mesh);
+        let stencil = whole(&definition.mesh).painted_over(placed.material);
         let inner: Vec<Instance> = definition
             .mesh
             .instances()
             .map(|(_, i)| i.clone())
@@ -106,8 +104,9 @@ impl Scene {
             };
             children.push(Instance {
                 definition,
                 transform: placed.transform * i.transform,
+                material: i.material.or(placed.material),
             });
         }
         let exploded = self.edit(parent, |m| {
             m.remove_instance(instance);
@@ -177,8 +176,9 @@ impl Scene {
                 };
                 copies.push(Instance {
                     definition,
                     transform: *place * original.transform,
+                    material: original.material,
                 });
             }
         }
         let placed = self.edit(parent, |m| {
@@ -211,8 +211,9 @@ impl Scene {
             };
             placed.push(Instance {
                 definition,
                 transform: child.transform,
+                material: child.material,
             });
         }
         let (copy, added) = self.add_definition(&name, kind);
         let filled = self
```

Apply to `crates/kernel/src/ids.rs`:
```diff
--- a/crates/kernel/src/ids.rs
+++ b/crates/kernel/src/ids.rs
@@ -162,8 +162,59 @@ impl TryFrom<String> for DefinitionId {
         s.parse()
     }
 }
 
+/// A material faces and instances are painted with. Serialized as "m<n>". Material ids come
+/// from the same counter as entity ids, so they never repeat one.
+#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
+#[serde(try_from = "String", into = "String")]
+pub struct MaterialId(pub u64);
+
+impl fmt::Display for MaterialId {
+    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
+        write!(f, "m{}", self.0)
+    }
+}
+
+impl FromStr for MaterialId {
+    type Err = String;
+
+    fn from_str(s: &str) -> Result<Self, Self::Err> {
+        match parse_prefixed(s) {
+            Some(('m', n)) => Ok(MaterialId(n)),
+            _ => Err(format!("invalid material id {s:?}")),
+        }
+    }
+}
+
+impl schemars::JsonSchema for MaterialId {
+    fn schema_name() -> std::borrow::Cow<'static, str> {
+        "MaterialId".into()
+    }
+
+    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
+        schemars::json_schema!({
+            "type": "string",
+            "pattern": "^m(0|[1-9][0-9]*)$",
+            "description": "Material id: m<n>"
+        })
+    }
+}
+
+impl From<MaterialId> for String {
+    fn from(id: MaterialId) -> String {
+        id.to_string()
+    }
+}
+
+impl TryFrom<String> for MaterialId {
+    type Error = String;
+
+    fn try_from(s: String) -> Result<Self, Self::Error> {
+        s.parse()
+    }
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
 
@@ -185,8 +236,13 @@ mod tests {
         assert_eq!(DefinitionId(3).to_string(), "d3");
         for s in ["", "d", "f3", "d03", "d-1"] {
             assert!(s.parse::<DefinitionId>().is_err(), "{s:?} parsed");
         }
+        assert_eq!("m5".parse::<MaterialId>(), Ok(MaterialId(5)));
+        assert_eq!(MaterialId(5).to_string(), "m5");
+        for s in ["", "m", "d5", "m05", "m+5"] {
+            assert!(s.parse::<MaterialId>().is_err(), "{s:?} parsed");
+        }
     }
 
     #[test]
     fn malformed_entity_ids_are_rejected() {
```

Apply to `crates/kernel/src/lib.rs`:
```diff
--- a/crates/kernel/src/lib.rs
+++ b/crates/kernel/src/lib.rs
@@ -10,8 +10,9 @@ pub mod geom;
 mod group;
 mod guides;
 mod ids;
 mod index;
+mod material;
 mod mesh;
 mod model;
 mod push_pull;
 mod regions;
@@ -21,13 +22,16 @@ mod triangles;
 mod weld;
 
 pub use curves::{Curve, CurveCircle, CurveKind};
 pub use data::{
-    DefinitionData, EdgeData, FaceData, GuideData, InstanceData, MAX_ID, MeshData, SceneData,
-    VertexData,
+    DefinitionData, EdgeData, FaceData, GuideData, InstanceData, MAX_ID, MaterialData, MeshData,
+    SceneData, VertexData,
 };
 pub use edit::KernelError;
-pub use ids::{CurveId, DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, VertexId};
+pub use ids::{
+    CurveId, DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, MaterialId, VertexId,
+};
 pub use index::{Index, Shape};
+pub use material::{MAX_MATERIAL_NAME, Material, Side};
 pub use model::{ChangeSet, ChangeSummary, Edge, Face, Guide, Instance, Model, Op, Vertex};
 pub use scene::{Definition, DefinitionKind, MeshId, Scene, SceneChange, Step};
 pub use triangles::Triangle;
```

Create `crates/kernel/src/material.rs`:
```rust
//! Materials: named colors shared by every mesh of a scene, painted on either side of a face or
//! on an instance (whose faces without a material of their own then show it).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{FaceId, InstanceId, MaterialId};
use crate::model::{ChangeSet, Model};
use crate::scene::{MeshId, Scene, SceneChange, Step};

/// The longest material name, in characters.
pub const MAX_MATERIAL_NAME: usize = 256;

/// A named color.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// sRGB red, green and blue.
    pub color: [u8; 3],
}

/// Which side of a face to paint.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The side the face's normal points to.
    #[default]
    Front,
    Back,
    Both,
}

/// Fails, saying why, when a material name is blank or longer than [`MAX_MATERIAL_NAME`]
/// characters.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a material needs a name".into());
    }
    if name.chars().count() > MAX_MATERIAL_NAME {
        return Err(format!(
            "a material name may have at most {MAX_MATERIAL_NAME} characters"
        ));
    }
    Ok(())
}

impl Model {
    /// Paints `side` of each face with `material` (None: the default material) as one
    /// ChangeSet. Faces already painted so, and unknown ids, are skipped.
    pub fn paint_faces(
        &mut self,
        faces: &[FaceId],
        side: Side,
        material: Option<MaterialId>,
    ) -> ChangeSet {
        for &id in faces.iter().collect::<BTreeSet<_>>() {
            let Some(f) = self.face(id) else { continue };
            let [mut front, mut back] = f.paint();
            if side != Side::Back {
                front = material;
            }
            if side != Side::Front {
                back = material;
            }
            self.set_paint(id, [front, back]);
        }
        self.take_changes()
    }

    /// Paints instances with `material` (None: none) as one ChangeSet. Unknown ids are skipped.
    pub fn paint_instances(
        &mut self,
        instances: &[InstanceId],
        material: Option<MaterialId>,
    ) -> ChangeSet {
        for &id in instances.iter().collect::<BTreeSet<_>>() {
            if self.instance(id).is_some() {
                self.set_instance_material(id, material);
            }
        }
        self.take_changes()
    }

    /// Takes `material` off every face side and instance painted with it, as one ChangeSet.
    pub(crate) fn unpaint(&mut self, material: MaterialId) -> ChangeSet {
        let off = |m: Option<MaterialId>| m.filter(|m| *m != material);
        let faces: Vec<_> = self
            .faces()
            .filter(|(_, f)| f.paint().contains(&Some(material)))
            .map(|(id, f)| (id, f.paint()))
            .collect();
        for (id, [front, back]) in faces {
            self.set_paint(id, [off(front), off(back)]);
        }
        let instances: Vec<_> = self
            .instances()
            .filter(|(_, i)| i.material == Some(material))
            .map(|(id, _)| id)
            .collect();
        for id in instances {
            self.set_instance_material(id, None);
        }
        self.take_changes()
    }
}

impl Scene {
    /// Adds a material. The name must be valid (see [`MAX_MATERIAL_NAME`]) and not taken.
    pub fn add_material(
        &mut self,
        name: &str,
        color: [u8; 3],
    ) -> Result<(MaterialId, SceneChange), KernelError> {
        self.check_free_name(name, None)?;
        let id = self.alloc_material();
        let step = Step::AddMaterial(
            id,
            Material {
                name: name.to_string(),
                color,
            },
        );
        self.apply_step(&step);
        Ok((id, step.into()))
    }

    /// Renames and/or recolors a material (None keeps that part). Nothing to change changes
    /// nothing.
    pub fn change_material(
        &mut self,
        id: MaterialId,
        name: Option<&str>,
        color: Option<[u8; 3]>,
    ) -> Result<SceneChange, KernelError> {
        let from = self
            .material(id)
            .ok_or(KernelError::MaterialNotFound)?
            .clone();
        if let Some(name) = name {
            self.check_free_name(name, Some(id))?;
        }
        let to = Material {
            name: name.map_or_else(|| from.name.clone(), str::to_string),
            color: color.unwrap_or(from.color),
        };
        if to == from {
            return Ok(SceneChange::default());
        }
        let step = Step::ChangeMaterial(id, from, to);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// Removes a material. Faces and instances painted with it go back to the default material
    /// first, so undo restores both.
    pub fn delete_material(&mut self, id: MaterialId) -> Result<SceneChange, KernelError> {
        let material = self
            .material(id)
            .ok_or(KernelError::MaterialNotFound)?
            .clone();
        let meshes: Vec<MeshId> = self.meshes().map(|(m, _)| m).collect();
        let mut changes = Vec::new();
        for mesh in meshes {
            changes.push(self.edit(mesh, |m| Ok(m.unpaint(id)))?);
        }
        let step = Step::RemoveMaterial(id, material);
        self.apply_step(&step);
        changes.push(step.into());
        Ok(SceneChange::join(changes))
    }

    /// Paints `side` of faces, and instances, of mesh `mesh` with `material` (None: the default
    /// material). Instances take the material whatever `side` says. Unknown ids are skipped.
    pub fn paint(
        &mut self,
        mesh: MeshId,
        faces: &[FaceId],
        instances: &[InstanceId],
        side: Side,
        material: Option<MaterialId>,
    ) -> Result<SceneChange, KernelError> {
        if material.is_some_and(|m| self.material(m).is_none()) {
            return Err(KernelError::MaterialNotFound);
        }
        self.edit(mesh, |m| {
            let mut c = m.paint_faces(faces, side, material);
            c.ops.extend(m.paint_instances(instances, material).ops);
            Ok(c)
        })
    }

    /// `base` if no material has that name, else `base` followed by the first free number
    /// from 2 ("Material 2").
    pub fn free_material_name(&self, base: &str) -> String {
        let taken: BTreeSet<&str> = self.materials().map(|(_, m)| m.name.as_str()).collect();
        if !taken.contains(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|n| !taken.contains(n.as_str()))
            .expect("a free name")
    }

    fn check_free_name(&self, name: &str, except: Option<MaterialId>) -> Result<(), KernelError> {
        check_name(name).map_err(KernelError::MaterialName)?;
        let taken = self
            .materials()
            .any(|(id, m)| Some(id) != except && m.name == name);
        if taken {
            return Err(KernelError::MaterialName(format!(
                "a material named {name:?} already exists"
            )));
        }
        Ok(())
    }
}
```

Apply to `crates/kernel/src/model.rs`:
```diff
--- a/crates/kernel/src/model.rs
+++ b/crates/kernel/src/model.rs
@@ -4,9 +4,11 @@ use std::sync::OnceLock;
 use glam::{DAffine3, DVec3};
 use serde::{Deserialize, Serialize};
 
 use crate::geom::{LENGTH_TOL, vector_area};
-use crate::ids::{CurveId, DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, VertexId};
+use crate::ids::{
+    CurveId, DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, MaterialId, VertexId,
+};
 use crate::index::Index;
 
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 pub struct Vertex {
@@ -43,16 +45,68 @@ impl Edge {
 pub struct Face {
     pub outer: Vec<VertexId>,
     pub holes: Vec<Vec<VertexId>>,
     pub normal: DVec3,
+    /// The material of the side `normal` points to; None is the default material.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub front: Option<MaterialId>,
+    /// The material of the other side.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub back: Option<MaterialId>,
+}
+
+impl Face {
+    /// An unpainted face.
+    pub fn new(outer: Vec<VertexId>, holes: Vec<Vec<VertexId>>, normal: DVec3) -> Face {
+        Face {
+            outer,
+            holes,
+            normal,
+            front: None,
+            back: None,
+        }
+    }
+
+    /// The same face turned over: loops reversed, normal negated, and the materials swapped so
+    /// each stays on the side it was on.
+    pub fn reversed(&self) -> Face {
+        let rev = |l: &Vec<VertexId>| l.iter().rev().copied().collect::<Vec<_>>();
+        Face {
+            outer: rev(&self.outer),
+            holes: self.holes.iter().map(rev).collect(),
+            normal: -self.normal,
+            front: self.back,
+            back: self.front,
+        }
+    }
+
+    /// The front and back materials.
+    pub fn paint(&self) -> [Option<MaterialId>; 2] {
+        [self.front, self.back]
+    }
 }
 
 /// A placed copy of a definition's geometry (a group or component).
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 pub struct Instance {
     pub definition: DefinitionId,
     /// Maps the definition's coordinates into the coordinates of the mesh holding the instance.
     pub transform: DAffine3,
+    /// The material shown on the sides of its faces (at any depth) that have none of their
+    /// own, unless an instance nearer to them has one.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub material: Option<MaterialId>,
+}
+
+impl Instance {
+    /// An unpainted instance.
+    pub fn new(definition: DefinitionId, transform: DAffine3) -> Instance {
+        Instance {
+            definition,
+            transform,
+            material: None,
+        }
+    }
 }
 
 /// Construction geometry drawn by the Tape Measure: not part of any face, never exported.
 #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
@@ -104,8 +158,12 @@ pub enum Op {
     /// An instance's transform changed from the first to the second.
     TransformInstance(InstanceId, DAffine3, DAffine3),
     AddGuide(GuideId, Guide),
     RemoveGuide(GuideId, Guide),
+    /// A face's front and back materials changed from the first pair to the second.
+    PaintFace(FaceId, [Option<MaterialId>; 2], [Option<MaterialId>; 2]),
+    /// An instance's material changed from the first to the second.
+    PaintInstance(InstanceId, Option<MaterialId>, Option<MaterialId>),
 }
 
 impl Op {
     fn inverse(&self) -> Op {
@@ -122,8 +180,10 @@ impl Op {
             Op::RemoveInstance(id, i) => Op::AddInstance(id, i),
             Op::TransformInstance(id, from, to) => Op::TransformInstance(id, to, from),
             Op::AddGuide(id, g) => Op::RemoveGuide(id, g),
             Op::RemoveGuide(id, g) => Op::AddGuide(id, g),
+            Op::PaintFace(id, from, to) => Op::PaintFace(id, to, from),
+            Op::PaintInstance(id, from, to) => Op::PaintInstance(id, to, from),
         }
     }
 
     /// The entity this op touches and whether it exists before and after the op.
@@ -141,8 +201,10 @@ impl Op {
             Op::RemoveInstance(id, _) => (EntityId::Instance(*id), true, false),
             Op::TransformInstance(id, _, _) => (EntityId::Instance(*id), true, true),
             Op::AddGuide(id, _) => (EntityId::Guide(*id), false, true),
             Op::RemoveGuide(id, _) => (EntityId::Guide(*id), true, false),
+            Op::PaintFace(id, _, _) => (EntityId::Face(*id), true, true),
+            Op::PaintInstance(id, _, _) => (EntityId::Instance(*id), true, true),
         }
     }
 }
 
@@ -158,8 +220,12 @@ pub struct ChangeSet {
 pub struct ChangeSummary {
     pub created: Vec<EntityId>,
     pub deleted: Vec<EntityId>,
     pub modified: Vec<EntityId>,
+    /// Materials added, removed, renamed or recolored (see `SceneChange::summary`). Absent
+    /// when none.
+    #[serde(default, skip_serializing_if = "Vec::is_empty")]
+    pub materials: Vec<MaterialId>,
 }
 
 impl ChangeSet {
     pub fn is_empty(&self) -> bool {
@@ -419,8 +485,19 @@ impl Model {
             }
             Op::RemoveGuide(id, _) => {
                 self.guides.remove(&id);
             }
+            Op::PaintFace(id, _, [front, back]) => {
+                if let Some(f) = self.faces.get_mut(&id) {
+                    f.front = front;
+                    f.back = back;
+                }
+            }
+            Op::PaintInstance(id, _, to) => {
+                if let Some(i) = self.instances.get_mut(&id) {
+                    i.material = to;
+                }
+            }
         }
     }
 
     fn bump(&mut self, used: u64) {
@@ -587,8 +664,24 @@ impl Model {
         }
         self.take_changes()
     }
 
+    /// Sets a face's front and back materials, recording a change only when they differ.
+    pub(crate) fn set_paint(&mut self, id: FaceId, paint: [Option<MaterialId>; 2]) {
+        let from = self.faces[&id].paint();
+        if from != paint {
+            self.record(Op::PaintFace(id, from, paint));
+        }
+    }
+
+    /// Sets an instance's material, recording a change only when it differs.
+    pub(crate) fn set_instance_material(&mut self, id: InstanceId, material: Option<MaterialId>) {
+        let from = self.instances[&id].material;
+        if from != material {
+            self.record(Op::PaintInstance(id, from, material));
+        }
+    }
+
     /// Replaces a face's data, keeping its id.
     pub(crate) fn replace_face(&mut self, id: FaceId, face: Face) {
         self.remove_face(id);
         self.record(Op::AddFace(id, face));
@@ -782,47 +875,21 @@ mod tests {
             (m, a, b, c)
         };
 
         let (mut m, a, b, c) = base();
-        m.insert_face(Face {
-            outer: vec![a, b, c],
-            holes: vec![],
-            normal: DVec3::Z,
-        });
+        m.insert_face(Face::new(vec![a, b, c], vec![], DVec3::Z));
         assert!(m.validate().is_ok());
 
         let bad_faces: Vec<(&str, Face)> = vec![
-            (
-                "empty outer loop",
-                Face {
-                    outer: vec![],
-                    holes: vec![],
-                    normal: DVec3::Z,
-                },
-            ),
+            ("empty outer loop", Face::new(vec![], vec![], DVec3::Z)),
             (
                 "missing vertex",
-                Face {
-                    outer: vec![a, b, VertexId(999)],
-                    holes: vec![],
-                    normal: DVec3::Z,
-                },
-            ),
-            (
-                "wrong winding",
-                Face {
-                    outer: vec![a, c, b],
-                    holes: vec![],
-                    normal: DVec3::Z,
-                },
+                Face::new(vec![a, b, VertexId(999)], vec![], DVec3::Z),
             ),
+            ("wrong winding", Face::new(vec![a, c, b], vec![], DVec3::Z)),
             (
                 "non-unit normal",
-                Face {
-                    outer: vec![a, b, c],
-                    holes: vec![],
-                    normal: DVec3::new(0.0, 0.0, 2.0),
-                },
+                Face::new(vec![a, b, c], vec![], DVec3::new(0.0, 0.0, 2.0)),
             ),
         ];
         for (what, face) in bad_faces {
             let (mut m, _, _, _) = base();
```

Apply to `crates/kernel/src/push_pull.rs`:
```diff
--- a/crates/kernel/src/push_pull.rs
+++ b/crates/kernel/src/push_pull.rs
@@ -18,9 +18,11 @@ impl Model {
     /// moves and those faces stretch. Otherwise it is extruded: a side face for every side of
     /// every loop, plus a cap. The original face stays as the base when it lies in a flat sheet
     /// and is removed when it is part of a solid's surface. Extruded faces point out of the solid.
     /// A push into a solid that ends exactly on a parallel face holding the whole pushed face
-    /// (the back of a wall) cuts an opening through to it.
+    /// (the back of a wall) cuts an opening through to it. Extruded faces (cap and sides) take
+    /// the face's front and back materials; a base that turns over keeps each material on the
+    /// side it was on.
     pub fn push_pull(&mut self, id: FaceId, distance: f64) -> Result<ChangeSet, KernelError> {
         let face = self.face(id).ok_or(KernelError::FaceNotFound)?.clone();
         if !distance.is_finite() {
             return Err(KernelError::NotFinite);
@@ -142,9 +144,10 @@ impl Model {
         Ok(())
     }
 
     /// Extrudes face `id` by `distance`. With `through`, the push ends on the back of the solid:
-    /// the cap and the part of the back face it covers are removed, leaving an opening.
+    /// the cap and the part of the back face it covers are removed, leaving an opening. The cap
+    /// and the side faces get the face's front and back materials.
     fn extrude(&mut self, id: FaceId, face: &Face, distance: f64, through: bool) {
         let n = face.normal;
         let offset = n * distance;
         let in_solid = self.in_solid(id);
@@ -270,8 +273,9 @@ impl Model {
                 continue;
             }
             if in_cap {
                 self.orient(f, cap_normal);
+                self.set_paint(f, face.paint());
                 continue;
             }
             // A side face lies inside a side's swept rectangle. Other new faces in a side's
             // plane (the rest of a wall split by the extrusion) keep their orientation.
@@ -291,8 +295,10 @@ impl Model {
             }
             let (_, _, pa, pb) = sides[i];
             let out = (pb - pa).cross(n).normalize();
             self.orient(f, if cut { -out } else { out });
+            // The sides take the pushed face's materials, as the cap does.
+            self.set_paint(f, face.paint());
         }
         if cut {
             self.remove_strays(&vertices_before, &sides, offset);
         }
@@ -562,20 +568,16 @@ impl Model {
         }
         true
     }
 
-    /// Flips face `id` if its normal points away from `want`.
+    /// Flips face `id` if its normal points away from `want`. Its materials stay on the sides
+    /// they were on.
     pub(crate) fn orient(&mut self, id: FaceId, want: DVec3) {
         let f = self.face(id).unwrap();
         if f.normal.dot(want) >= 0.0 {
             return;
         }
-        let reversed = |l: &Vec<VertexId>| l.iter().rev().copied().collect::<Vec<_>>();
-        let flipped = Face {
-            outer: reversed(&f.outer),
-            holes: f.holes.iter().map(reversed).collect(),
-            normal: -f.normal,
-        };
+        let flipped = f.reversed();
         self.replace_face(id, flipped);
     }
 
     /// True if the face is part of a solid's surface with the solid behind it: it is connected
```

Apply to `crates/kernel/src/scene.rs`:
```diff
--- a/crates/kernel/src/scene.rs
+++ b/crates/kernel/src/scene.rs
@@ -5,9 +5,10 @@ use std::collections::{BTreeMap, BTreeSet};
 use glam::{DAffine3, DVec3};
 use serde::{Deserialize, Serialize};
 
 use crate::edit::KernelError;
-use crate::ids::{CurveId, DefinitionId, EntityId, InstanceId};
+use crate::ids::{CurveId, DefinitionId, EntityId, InstanceId, MaterialId};
+use crate::material::Material;
 use crate::model::{ChangeSet, ChangeSummary, Instance, Model};
 
 /// A group has one instance, and a copy of it gets a definition of its own. A component is
 /// shared by all its instances: editing it changes them all.
@@ -41,8 +42,12 @@ pub enum Step {
     AddDefinition(DefinitionId, Definition),
     RemoveDefinition(DefinitionId, Definition),
     /// A definition's name changed from the first to the second.
     RenameDefinition(DefinitionId, String, String),
+    AddMaterial(MaterialId, Material),
+    RemoveMaterial(MaterialId, Material),
+    /// A material's name or color changed from the first to the second.
+    ChangeMaterial(MaterialId, Material, Material),
 }
 
 impl Step {
     fn inverse(&self) -> Step {
@@ -50,8 +55,11 @@ impl Step {
             Step::Edit(mesh, c) => Step::Edit(mesh, c.inverse()),
             Step::AddDefinition(id, d) => Step::RemoveDefinition(id, d),
             Step::RemoveDefinition(id, d) => Step::AddDefinition(id, d),
             Step::RenameDefinition(id, from, to) => Step::RenameDefinition(id, to, from),
+            Step::AddMaterial(id, m) => Step::RemoveMaterial(id, m),
+            Step::RemoveMaterial(id, m) => Step::AddMaterial(id, m),
+            Step::ChangeMaterial(id, from, to) => Step::ChangeMaterial(id, to, from),
         }
     }
 }
 
@@ -79,9 +87,10 @@ impl SceneChange {
         }
     }
 
     /// Net effect per entity across all meshes (ids are unique in a scene). Definitions are not
-    /// listed; the instances that place them are.
+    /// listed; the instances that place them are. Materials added, removed or changed are
+    /// listed in `materials` (one added and removed again is not).
     pub fn summary(&self) -> ChangeSummary {
         let ops = self
             .steps
             .iter()
@@ -90,9 +99,26 @@ impl SceneChange {
                 _ => None,
             })
             .flatten()
             .collect();
-        ChangeSet { ops }.summary()
+        let mut summary = ChangeSet { ops }.summary();
+        // material -> (existed before, exists after)
+        let mut state: BTreeMap<MaterialId, (bool, bool)> = BTreeMap::new();
+        for step in &self.steps {
+            let (id, before, after) = match step {
+                Step::AddMaterial(id, _) => (*id, false, true),
+                Step::RemoveMaterial(id, _) => (*id, true, false),
+                Step::ChangeMaterial(id, _, _) => (*id, true, true),
+                _ => continue,
+            };
+            state.entry(id).or_insert((before, after)).1 = after;
+        }
+        summary.materials = state
+            .into_iter()
+            .filter(|(_, (before, after))| *before || *after)
+            .map(|(id, _)| id)
+            .collect();
+        summary
     }
 }
 
 impl From<Step> for SceneChange {
@@ -105,16 +131,21 @@ impl From<Step> for SceneChange {
 #[derive(Clone, Debug, Default)]
 pub struct Scene {
     root: Model,
     definitions: BTreeMap<DefinitionId, Definition>,
+    /// The materials faces and instances may be painted with, shared by every mesh.
+    materials: BTreeMap<MaterialId, Material>,
     /// The next id to hand out, in any mesh. Never decreases, so ids are never reused.
     next_id: u64,
 }
 
-/// Two scenes are equal when they hold the same entities and definitions under the same ids.
+/// Two scenes are equal when they hold the same entities, definitions and materials under the
+/// same ids.
 impl PartialEq for Scene {
     fn eq(&self, other: &Self) -> bool {
-        self.root == other.root && self.definitions == other.definitions
+        self.root == other.root
+            && self.definitions == other.definitions
+            && self.materials == other.materials
     }
 }
 
 impl Scene {
@@ -127,27 +158,31 @@ impl Scene {
         Scene {
             next_id: model.next_id(),
             root: model,
             definitions: BTreeMap::new(),
+            materials: BTreeMap::new(),
         }
     }
 
     /// A scene of these meshes; new ids start at `next_id` or after the largest id used,
     /// whichever is later. The caller validates the result.
     pub(crate) fn from_meshes(
         root: Model,
         definitions: BTreeMap<DefinitionId, Definition>,
+        materials: BTreeMap<MaterialId, Material>,
         next_id: u64,
     ) -> Self {
         let used = definitions
             .iter()
             .map(|(id, d)| id.0.saturating_add(1).max(d.mesh.next_id()))
+            .chain(materials.keys().map(|id| id.0.saturating_add(1)))
             .chain([root.next_id(), next_id])
             .max()
             .unwrap_or(0);
         Scene {
             root,
             definitions,
+            materials,
             next_id: used,
         }
     }
 
@@ -166,8 +201,24 @@ impl Scene {
             MeshId::Definition(d) => self.definitions.get(&d).map(|d| &d.mesh),
         }
     }
 
+    pub fn material(&self, id: MaterialId) -> Option<&Material> {
+        self.materials.get(&id)
+    }
+
+    /// Every material, by id.
+    pub fn materials(&self) -> impl Iterator<Item = (MaterialId, &Material)> {
+        self.materials.iter().map(|(k, v)| (*k, v))
+    }
+
+    /// A material id for a new material, taken from the entity id counter.
+    pub(crate) fn alloc_material(&mut self) -> MaterialId {
+        let id = MaterialId(self.next_id);
+        self.next_id += 1;
+        id
+    }
+
     fn mesh_mut(&mut self, id: MeshId) -> Option<&mut Model> {
         match id {
             MeshId::Root => Some(&mut self.root),
             MeshId::Definition(d) => self.definitions.get_mut(&d).map(|d| &mut d.mesh),
@@ -299,12 +350,9 @@ impl Scene {
                 m.guides()
                     .map(|(_, g)| instance.transform.transform_point3(g.anchor())),
             );
             for (_, i) in m.instances() {
-                let placed = Instance {
-                    definition: i.definition,
-                    transform: instance.transform * i.transform,
-                };
+                let placed = Instance::new(i.definition, instance.transform * i.transform);
                 out.extend(self.instance_guide_anchors(&placed));
             }
         }
         out
@@ -349,9 +397,9 @@ impl Scene {
             self.apply_step(step);
         }
     }
 
-    fn apply_step(&mut self, step: &Step) {
+    pub(crate) fn apply_step(&mut self, step: &Step) {
         match step {
             Step::Edit(mesh, c) => {
                 let m = self.mesh_mut(*mesh).expect("change to a missing mesh");
                 m.apply(c);
@@ -372,22 +420,56 @@ impl Scene {
                 if let Some(d) = self.definitions.get_mut(id) {
                     d.name = to.clone();
                 }
             }
+            Step::AddMaterial(id, m) => {
+                self.next_id = self.next_id.max(id.0.saturating_add(1));
+                self.materials.insert(*id, m.clone());
+            }
+            Step::RemoveMaterial(id, _) => {
+                self.materials.remove(id);
+            }
+            Step::ChangeMaterial(id, _, to) => {
+                if let Some(m) = self.materials.get_mut(id) {
+                    *m = to.clone();
+                }
+            }
         }
     }
 
     /// Checks every mesh, that instances place existing definitions, that no definition places
-    /// itself (directly or deeper), and that no id is used in two meshes.
+    /// itself (directly or deeper), that no id is used in two meshes, that every material
+    /// painted exists, and that material names are valid and distinct.
     pub fn validate(&self) -> Result<(), String> {
+        let mut names = BTreeSet::new();
+        for (id, m) in &self.materials {
+            crate::material::check_name(&m.name).map_err(|e| format!("material {id}: {e}"))?;
+            if !names.insert(m.name.as_str()) {
+                return Err(format!(
+                    "material {id}: two materials are named {:?}",
+                    m.name
+                ));
+            }
+        }
+        let missing = |m: &Option<MaterialId>| m.is_some_and(|m| !self.materials.contains_key(&m));
         let mut seen = BTreeSet::new();
         let mut curves_seen = BTreeSet::new();
         for (mesh, m) in self.meshes() {
             m.validate().map_err(|e| format!("{mesh:?}: {e}"))?;
             for (id, i) in m.instances() {
                 if !self.definitions.contains_key(&i.definition) {
                     return Err(format!("instance {id:?} places a missing definition"));
                 }
+                if missing(&i.material) {
+                    return Err(format!(
+                        "instance {id:?} is painted with a missing material"
+                    ));
+                }
+            }
+            for (id, f) in m.faces() {
+                if missing(&f.front) || missing(&f.back) {
+                    return Err(format!("face {id:?} is painted with a missing material"));
+                }
             }
             for id in entity_ids(m) {
                 if !seen.insert(id) {
                     return Err(format!("{id} is used in two meshes"));
```

Apply to `crates/kernel/src/transform.rs`:
```diff
--- a/crates/kernel/src/transform.rs
+++ b/crates/kernel/src/transform.rs
@@ -7,9 +7,9 @@ use glam::{DAffine3, DVec3};
 use crate::edit::KernelError;
 use crate::geom::{
     LENGTH_TOL, Plane, point_in_polygon, project_on_segment, segment_distance_2d, vector_area,
 };
-use crate::ids::{CurveId, EdgeId, FaceId, VertexId};
+use crate::ids::{CurveId, EdgeId, FaceId, MaterialId, VertexId};
 use crate::model::{ChangeSet, Face, Model};
 
 impl Model {
     /// Moves `vertices` by `offset` as one ChangeSet. Faces that use some of them stretch and
@@ -206,8 +206,9 @@ impl Model {
             }
             let mut face = StencilFace {
                 outer: ring(&f.outer),
                 normal: f.normal,
+                paint: f.paint(),
                 openings: Vec::new(),
                 filled: Vec::new(),
             };
             for h in &f.holes {
@@ -295,9 +296,9 @@ impl Model {
             placed.iter().any(|f| {
                 m.face_within(id, &f.outer) && !f.openings.iter().any(|h| m.face_within(id, h))
             })
         };
-        // Give each piece of a stencil face that face's orientation.
+        // Give each piece of a stencil face that face's orientation and materials.
         for f in &placed {
             for &id in &new {
                 let in_hole = f
                     .openings
@@ -305,8 +306,9 @@ impl Model {
                     .chain(&f.filled)
                     .any(|h| self.face_within(id, h));
                 if self.face_within(id, &f.outer) && !in_hole {
                     self.orient(id, f.normal);
+                    self.set_paint(id, f.paint);
                 }
             }
         }
         // A new face within an existing face (and not within one of its holes) is a piece of
@@ -392,13 +394,28 @@ pub(crate) struct Stencil {
     marks: Vec<(Option<CurveId>, bool)>,
     faces: Vec<StencilFace>,
 }
 
+impl Stencil {
+    /// The stencil with `material` on every face side that has no material of its own (an
+    /// exploded instance's material passing to its faces).
+    pub(crate) fn painted_over(mut self, material: Option<MaterialId>) -> Stencil {
+        for f in &mut self.faces {
+            for side in &mut f.paint {
+                *side = side.or(material);
+            }
+        }
+        self
+    }
+}
+
 /// A face captured by position.
 #[derive(Clone, Debug)]
 struct StencilFace {
     outer: Vec<DVec3>,
     normal: DVec3,
+    /// Front and back materials.
+    paint: [Option<MaterialId>; 2],
     /// Holes no face fills.
     openings: Vec<Vec<DVec3>>,
     /// Holes another face fills.
     filled: Vec<Vec<DVec3>>,
@@ -409,8 +426,9 @@ impl StencilFace {
         let ring = |r: &Vec<DVec3>| r.iter().map(|p| t.transform_point3(*p)).collect();
         StencilFace {
             outer: ring(&self.outer),
             normal: t.transform_vector3(self.normal).normalize(),
+            paint: self.paint,
             openings: self.openings.iter().map(ring).collect(),
             filled: self.filled.iter().map(ring).collect(),
         }
     }
```

Apply to `crates/kernel/src/weld.rs`:
```diff
--- a/crates/kernel/src/weld.rs
+++ b/crates/kernel/src/weld.rs
@@ -54,16 +54,9 @@ impl Model {
                 self.remove_face(id);
                 continue;
             }
             let holes = f.holes.iter().map(fix).filter(|h| !flat(h)).collect();
-            self.replace_face(
-                id,
-                Face {
-                    outer,
-                    holes,
-                    normal: f.normal,
-                },
-            );
+            self.replace_face(id, Face { outer, holes, ..f });
         }
 
         let joined: Vec<(EdgeId, [VertexId; 2])> = self
             .edges()
```

Apply to `crates/session/src/document.rs`:
```diff
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -732,12 +732,9 @@ impl Document {
                     .map(|g| at(g.anchor())),
             );
             for id in instances {
                 let Some(i) = m.instance(*id) else { continue };
-                let placed = Instance {
-                    definition: i.definition,
-                    transform: *t * i.transform,
-                };
+                let placed = Instance::new(i.definition, *t * i.transform);
                 let mut points: Vec<[f64; 3]> = match self.scene.instance_bounds(&placed) {
                     Some(b) => b.map(|p| p.to_array()).to_vec(),
                     None => vec![placed.transform.translation.to_array()],
                 };
```

- [ ] **Step 4: Run the tests to verify they pass**

`cargo test -p kernel --test materials` 9 passed; `cargo test --workspace` 693 passed; fmt and clippy `-D warnings` clean.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` (expected totals: 693, 700, 705, 711 after Tasks 1-4).

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/kernel/tests/data.rs crates/kernel/tests/materials.rs crates/kernel/src/data.rs crates/kernel/src/edit.rs crates/kernel/src/faces.rs crates/kernel/src/group.rs crates/kernel/src/ids.rs crates/kernel/src/lib.rs crates/kernel/src/material.rs crates/kernel/src/model.rs crates/kernel/src/push_pull.rs crates/kernel/src/scene.rs crates/kernel/src/transform.rs crates/kernel/src/weld.rs crates/session/src/document.rs
git commit -m "Kernel materials: scene materials (m ids), front/back face paint, instance material, paint ops; paint follows splits, merges, push/pull, copies, groups and explodes; materials in scene data"
```

### Task 2: Material commands, queries and files

**Files:**
- Create: `crates/session/src/document/materials.rs` (add_material_changes, change_material_changes, delete_material_changes, paint_changes, material_infos, material_error)
- Create (test): `crates/session/tests/materials.rs` (7 tests)
- Modify: `crates/session/src/command.rs` (4 variants; imports MaterialId, Side), `query.rs` (Query::Materials, MaterialInfo, QueryResult::Materials, FaceInfo.front_material/back_material, InstanceInfo.material), `document.rs` (mod materials; edit() arms; unreachable arm list; Query::Materials; face_info fields), `document/groups.rs` (instance_info material), `file.rs` (SCHEMA_VERSION 5 + doc), `lib.rs` (re-export `MaterialId`, `Side`, `MaterialInfo`)
- Modify (test hunks): `crates/session/tests/camera.rs:439` and `crates/session/tests/curves.rs:205` (`== 4` -> `session::file::SCHEMA_VERSION`), `crates/api/tests/dispatch.rs` (describe lists add_material, change_material, delete_material, paint, query materials)
- Modify docs: `docs/agent-api.md` (queries table row, command list, `m6` ids, new "### Materials" section, Files paragraph schema 5 and refusal reasons), spec (status line "M7b materials", Face bullet, M7b milestone line after M7a)

**Interfaces:**
- Produces:
  - `Command::AddMaterial { name: Option<String>, color: [u8; 3] }`, `Command::ChangeMaterial { material: MaterialId, name: Option<String>, color: Option<[u8; 3]> }`, `Command::DeleteMaterial { material: MaterialId }`, `Command::Paint { ids: Vec<EntityId>, material: Option<MaterialId>, side: Side }` (all `#[serde(default)]` on optional fields)
  - `Query::Materials`; `QueryResult::Materials { materials: Vec<MaterialInfo>, revision: u64 }`; `pub struct MaterialInfo { id: MaterialId, name: String, color: [u8; 3], faces: usize, instances: usize }`
  - `FaceInfo.front_material`, `FaceInfo.back_material`, `InstanceInfo.material`: `Option<MaterialId>` (skip when None)
  - `session::file::SCHEMA_VERSION = 5`

Consumes: Task 1 Scene API; `Document::parts`, `active`, `edit` routing.

- [ ] **Step 1: Write the failing tests**

Tests: new integration file `crates/session/tests/materials.rs`: `materials_are_added_renamed_recolored_and_deleted`, `bad_material_commands_are_refused`, `painting_faces_sets_their_front_and_back_materials`, `bad_paint_commands_are_refused`, `instances_are_painted_inside_and_out_and_deleting_unpaints`, `a_failed_atomic_batch_unpaints_and_events_list_materials`, `materials_and_paint_are_saved_and_older_files_still_open` (writes a v4 copy without materials/front/back and a broken copy; scratch names include the pid).

Test-module hunks are part of the Step 3 diffs; apply them first, along with the test files below.

Apply to `crates/api/tests/dispatch.rs`:
```diff
--- a/crates/api/tests/dispatch.rs
+++ b/crates/api/tests/dispatch.rs
@@ -141,8 +141,12 @@ fn describe_lists_methods_and_schemas() {
         "open_instance",
         "close_instance",
         "rename_definition",
         "purge_definitions",
+        "add_material",
+        "change_material",
+        "delete_material",
+        "paint",
         "new",
         "open",
         "save",
         "export",
@@ -152,9 +156,16 @@ fn describe_lists_methods_and_schemas() {
             "Command schema lacks {variant}"
         );
     }
     let query_schema = d["schemas"]["Query"].to_string();
-    for variant in ["events", "definitions", "context", "file", "curve"] {
+    for variant in [
+        "events",
+        "definitions",
+        "context",
+        "file",
+        "curve",
+        "materials",
+    ] {
         assert!(
             query_schema.contains(variant),
             "Query schema lacks {variant}"
         );
```

Apply to `crates/session/tests/camera.rs`:
```diff
--- a/crates/session/tests/camera.rs
+++ b/crates/session/tests/camera.rs
@@ -435,9 +435,9 @@ fn the_view_and_projection_are_saved_with_the_model() {
     // A version 3 file (before projections) has neither field and opens in perspective.
     let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
     let mut document: Value =
         serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
-    assert_eq!(document["schema_version"], 4);
+    assert_eq!(document["schema_version"], session::file::SCHEMA_VERSION);
     assert_eq!(document["camera"]["projection"], "parallel");
     document["schema_version"] = json!(3);
     document["camera"] = json!({"eye": [-1, -2, 3], "target": [0, 0, 0], "fov_y_deg": 35});
     let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
```

Apply to `crates/session/tests/curves.rs`:
```diff
--- a/crates/session/tests/curves.rs
+++ b/crates/session/tests/curves.rs
@@ -201,9 +201,9 @@ fn curves_are_saved_and_older_files_still_open() {
     assert_eq!(curve(&e, &first), curve(&d, &first));
 
     // A version 2 file (before curves) opens, its edges plain.
     edit_file(&path, |doc| {
-        assert_eq!(doc["schema_version"], 4);
+        assert_eq!(doc["schema_version"], session::file::SCHEMA_VERSION);
         doc["schema_version"] = json!(2);
         let camera = doc["camera"].as_object_mut().unwrap();
         camera.remove("up");
         camera.remove("projection");
```

Create `crates/session/tests/materials.rs`:
```rust
//! Materials and painting, driven through JSON commands and queries.

use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String, Vec<String>)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| {
            (
                e.kind,
                e.message,
                e.ids.iter().map(|i| i.to_string()).collect(),
            )
        })
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

/// A 2 x 2 x 1 box: `f8` is its bottom (facing down), `f25` its top.
fn cube(d: &mut Document) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "push_pull", "face": "f8", "distance": 1.0}),
    )
    .unwrap();
}

/// Adds a material and returns its id.
fn add(d: &mut Document, name: &str, color: [u8; 3]) -> String {
    let out = run(
        d,
        json!({"type": "add_material", "name": name, "color": color}),
    )
    .unwrap();
    out["changes"]["materials"][0].as_str().unwrap().to_string()
}

fn materials(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "materials"}))["materials"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn materials_are_added_renamed_recolored_and_deleted() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_material", "color": [10, 20, 30]}),
    )
    .unwrap();
    let first = out["changes"]["materials"][0].as_str().unwrap().to_string();
    assert!(first.starts_with('m'), "{first}");
    assert!(out["changes"]["created"].as_array().unwrap().is_empty());
    assert_eq!(d.revision(), 1, "an undo step that changes the revision");
    let second = run(&mut d, json!({"type": "add_material", "color": [0, 0, 0]})).unwrap()
        ["changes"]["materials"][0]
        .clone();
    let listed = materials(&d);
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed[0],
        json!({"id": first, "name": "Material", "color": [10, 20, 30], "faces": 0, "instances": 0})
    );
    assert_eq!(listed[1]["id"], second);
    assert_eq!(listed[1]["name"], "Material 2");

    run(
        &mut d,
        json!({"type": "change_material", "material": first, "name": "Brick", "color": [180, 60, 40]}),
    )
    .unwrap();
    assert_eq!(materials(&d)[0]["name"], "Brick");
    assert_eq!(materials(&d)[0]["color"], json!([180, 60, 40]));
    run(
        &mut d,
        json!({"type": "change_material", "material": first, "color": [1, 2, 3]}),
    )
    .unwrap();
    assert_eq!(materials(&d)[0]["name"], "Brick", "absent fields stay");

    let out = run(
        &mut d,
        json!({"type": "delete_material", "material": first}),
    )
    .unwrap();
    assert_eq!(out["changes"]["materials"], json!([first]));
    assert_eq!(materials(&d).len(), 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[0]["color"], json!([1, 2, 3]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[0]["name"], "Material");
}

#[test]
fn bad_material_commands_are_refused() {
    let mut d = doc();
    let red = add(&mut d, "Red", [200, 0, 0]);
    let invalid = |r: Result<Value, (ErrorKind, String, Vec<String>)>, why: &str| {
        let (kind, message, _) = r.unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
        assert!(message.contains(why), "{message:?} lacks {why:?}");
    };
    invalid(
        run(
            &mut d,
            json!({"type": "add_material", "name": "Red", "color": [0, 0, 0]}),
        ),
        "already exists",
    );
    invalid(
        run(
            &mut d,
            json!({"type": "add_material", "name": " ", "color": [0, 0, 0]}),
        ),
        "needs a name",
    );
    invalid(
        run(
            &mut d,
            json!({"type": "add_material", "name": "x".repeat(257), "color": [0, 0, 0]}),
        ),
        "at most 256",
    );
    let (kind, message, _) = run(
        &mut d,
        json!({"type": "change_material", "material": "m999", "name": "X"}),
    )
    .unwrap_err();
    assert_eq!(
        (kind, message.as_str()),
        (ErrorKind::NotFound, "no material m999")
    );
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "delete_material", "material": "m999"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    // Colors are whole numbers 0 to 255, and material ids are "m<n>".
    for bad in [
        json!({"type": "add_material", "color": [256, 0, 0]}),
        json!({"type": "add_material", "color": [0.5, 0, 0]}),
        json!({"type": "add_material", "color": [0, 0]}),
        json!({"type": "delete_material", "material": "d3"}),
    ] {
        assert!(
            serde_json::from_value::<Command>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    assert_eq!(materials(&d).len(), 1);
    assert_eq!(materials(&d)[0]["id"], red);
}

#[test]
fn painting_faces_sets_their_front_and_back_materials() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    let blue = add(&mut d, "Blue", [0, 0, 200]);
    let revision = d.revision();
    let out = run(
        &mut d,
        json!({"type": "paint", "ids": ["f25"], "material": red}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"], json!(["f25"]));
    assert_eq!(d.revision(), revision + 1);
    let top = ask(&d, json!({"type": "entity", "id": "f25"}));
    assert_eq!(top["front_material"], red);
    assert!(top.get("back_material").is_none(), "the default is absent");

    run(
        &mut d,
        json!({"type": "paint", "ids": ["f25", "f8"], "material": blue, "side": "back"}),
    )
    .unwrap();
    let top = ask(&d, json!({"type": "entity", "id": "f25"}));
    assert_eq!(
        (&top["front_material"], &top["back_material"]),
        (&json!(red), &json!(blue))
    );
    run(
        &mut d,
        json!({"type": "paint", "ids": ["f25"], "side": "both"}),
    )
    .unwrap();
    let top = ask(&d, json!({"type": "entity", "id": "f25"}));
    assert!(top.get("front_material").is_none() && top.get("back_material").is_none());
    assert_eq!(materials(&d)[1]["faces"], 1, "f8's back is still blue");

    run(&mut d, json!({"type": "undo"})).unwrap();
    let top = ask(&d, json!({"type": "entity", "id": "f25"}));
    assert_eq!(top["front_material"], red);

    // Painting what is already painted so changes nothing and adds no undo step.
    let revision = d.revision();
    let out = run(
        &mut d,
        json!({"type": "paint", "ids": ["f25"], "material": red}),
    )
    .unwrap();
    assert!(out["changes"]["modified"].as_array().unwrap().is_empty());
    assert_eq!(d.revision(), revision);
}

#[test]
fn bad_paint_commands_are_refused() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    let cases = [
        (
            json!({"type": "paint", "ids": [], "material": red}),
            ErrorKind::InvalidInput,
            vec![],
        ),
        (
            json!({"type": "paint", "ids": ["f8", "e2"], "material": red}),
            ErrorKind::InvalidInput,
            vec!["e2"],
        ),
        (
            json!({"type": "paint", "ids": ["v0"], "material": red}),
            ErrorKind::InvalidInput,
            vec!["v0"],
        ),
        (
            json!({"type": "paint", "ids": ["f999"], "material": red}),
            ErrorKind::NotFound,
            vec!["f999"],
        ),
        (
            json!({"type": "paint", "ids": ["f8"], "material": "m999"}),
            ErrorKind::NotFound,
            vec![],
        ),
    ];
    let revision = d.revision();
    for (command, kind, ids) in cases {
        let (got, message, got_ids) = run(&mut d, command.clone()).unwrap_err();
        assert_eq!(got, kind, "{command}: {message}");
        assert_eq!(got_ids, ids, "{command}");
    }
    assert_eq!(d.revision(), revision);
    assert!(
        serde_json::from_value::<Command>(
            json!({"type": "paint", "ids": ["f8"], "material": red, "side": "inside"})
        )
        .is_err()
    );
}

/// The id of the open context's face pointing up at z = 1.
fn top_face(d: &Document) -> Value {
    let faces = ask(d, json!({"type": "entities", "kinds": ["face"]}));
    faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap()["id"]
        .clone()
}

#[test]
fn instances_are_painted_inside_and_out_and_deleting_unpaints() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    let all = ask(&d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    let ids: Vec<Value> = all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect();
    run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    run(
        &mut d,
        json!({"type": "paint", "ids": [group], "material": red, "side": "back"}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": group}))["material"],
        red
    );

    // Inside the group, its faces (with new ids) are painted in the group's own context.
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let top = top_face(&d);
    run(
        &mut d,
        json!({"type": "paint", "ids": [top], "material": red}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let listed = materials(&d);
    assert_eq!(
        (&listed[0]["faces"], &listed[0]["instances"]),
        (&json!(1), &json!(1))
    );

    run(&mut d, json!({"type": "delete_material", "material": red})).unwrap();
    assert!(
        ask(&d, json!({"type": "entity", "id": group}))
            .get("material")
            .is_none()
    );
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    assert!(
        ask(&d, json!({"type": "entity", "id": top}))
            .get("front_material")
            .is_none()
    );
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": group}))["material"],
        red
    );
    assert_eq!(materials(&d)[0]["faces"], 1);
}

#[test]
fn a_failed_atomic_batch_unpaints_and_events_list_materials() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "paint", "ids": ["f25"], "material": red},
        {"type": "paint", "ids": ["f8"], "material": "m999"}
    ]))
    .unwrap();
    let err = d.execute_batch(commands, true).unwrap_err();
    assert!(err.rolled_back);
    assert!(
        ask(&d, json!({"type": "entity", "id": "f25"}))
            .get("front_material")
            .is_none()
    );
    let events = ask(&d, json!({"type": "events", "since": 0}));
    let last = events["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["changes"]["materials"], json!([red]));
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("session-materials");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn materials_and_paint_are_saved_and_older_files_still_open() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    run(
        &mut d,
        json!({"type": "paint", "ids": ["f25"], "material": red, "side": "both"}),
    )
    .unwrap();
    let path = scratch("painted.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(materials(&e), materials(&d));
    assert_eq!(
        ask(&e, json!({"type": "entity", "id": "f25"})),
        ask(&d, json!({"type": "entity", "id": "f25"}))
    );
    // A new material after reopening gets an id never used before.
    let next = add(&mut e, "Blue", [0, 0, 200]);
    assert_ne!(next, red);

    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let document: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    assert_eq!(document["schema_version"], 5);
    assert_eq!(document["model"]["materials"][0]["name"], "Red");

    // The same model written by version 4, before materials: it opens unpainted.
    let mut old = document.clone();
    old["schema_version"] = json!(4);
    old["model"].as_object_mut().unwrap().remove("materials");
    for f in old["model"]["root"]["faces"].as_array_mut().unwrap() {
        let f = f.as_object_mut().unwrap();
        f.remove("front");
        f.remove("back");
    }
    let old_path = scratch("v4.skr");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&old_path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, &serde_json::to_vec(&old).unwrap()).unwrap();
    zip.finish().unwrap();
    let mut v4 = doc();
    run(
        &mut v4,
        json!({"type": "open", "path": old_path.to_str().unwrap()}),
    )
    .unwrap();
    assert!(materials(&v4).is_empty());
    assert_eq!(ask(&v4, json!({"type": "stats"}))["faces"], 6);

    // A face painted with a material the file lacks does not open.
    let mut broken = document;
    broken["model"]["materials"] = json!([]);
    let broken_path = scratch("broken.skr");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&broken_path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, &serde_json::to_vec(&broken).unwrap()).unwrap();
    zip.finish().unwrap();
    let (kind, message, _) = run(
        &mut doc(),
        json!({"type": "open", "path": broken_path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("missing material"), "{message}");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

`cargo test -p session --test materials` -> all 7 fail at `materials.rs:14` "valid command json" (unknown variant `add_material`).

- [ ] **Step 3: Implement**

Apply to `crates/session/src/command.rs`:
```diff
--- a/crates/session/src/command.rs
+++ b/crates/session/src/command.rs
@@ -1,7 +1,7 @@
 use std::fmt;
 
-use kernel::{ChangeSummary, DefinitionId, EntityId};
+use kernel::{ChangeSummary, DefinitionId, EntityId, MaterialId, Side};
 
 use schemars::JsonSchema;
 use serde::{Deserialize, Serialize};
 
@@ -143,8 +143,40 @@ pub enum Command {
     },
     /// Remove every definition that no instance places (components left after their last
     /// instance was erased or exploded), and what only they placed.
     PurgeDefinitions,
+    /// Add a material: a named sRGB color (each 0 to 255). Without `name` it is called
+    /// "Material" (or "Material 2", ... if taken). Names must be distinct, not blank, and at
+    /// most 256 characters. The new id is in the outcome's `changes.materials`.
+    AddMaterial {
+        #[serde(default)]
+        name: Option<String>,
+        color: [u8; 3],
+    },
+    /// Rename and/or recolor a material; absent fields stay as they are.
+    ChangeMaterial {
+        material: MaterialId,
+        #[serde(default)]
+        name: Option<String>,
+        #[serde(default)]
+        color: Option<[u8; 3]>,
+    },
+    /// Remove a material. Faces and instances painted with it, anywhere in the model, go back
+    /// to the default material.
+    DeleteMaterial {
+        material: MaterialId,
+    },
+    /// Paint faces and instances of the open context with `material` (absent or null: the
+    /// default material), as one undo step. On faces `side` (default `front`) picks the side
+    /// the normal points to, the other one, or both. An instance's material shows on the face
+    /// sides inside it (at any depth) that have no material of their own.
+    Paint {
+        ids: Vec<EntityId>,
+        #[serde(default)]
+        material: Option<MaterialId>,
+        #[serde(default)]
+        side: Side,
+    },
     /// Start an empty model with the default view. Clears undo history; not an undo step.
     /// With changes not saved, fails (`unsaved_changes`) unless `discard` is true.
     New {
         #[serde(default)]
```

Apply to `crates/session/src/document.rs`:
```diff
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -20,8 +20,9 @@ use crate::rotate::rotation;
 
 mod camera;
 mod files;
 mod groups;
+mod materials;
 
 /// Largest coordinate magnitude (meters) accepted from commands.
 pub const MAX_COORDINATE: f64 = 1.0e6;
 
@@ -236,9 +237,13 @@ impl Document {
             | Command::MakeGroup { .. }
             | Command::MakeComponent { .. }
             | Command::Explode { .. }
             | Command::RenameDefinition { .. }
-            | Command::PurgeDefinitions => {
+            | Command::PurgeDefinitions
+            | Command::AddMaterial { .. }
+            | Command::ChangeMaterial { .. }
+            | Command::DeleteMaterial { .. }
+            | Command::Paint { .. } => {
                 unreachable!("edits are handled by Document::edit")
             }
         }
     }
@@ -300,8 +305,22 @@ impl Document {
             Command::RenameDefinition { definition, name } => {
                 self.rename_changes(*definition, name)
             }
             Command::PurgeDefinitions => Ok(self.scene.purge()),
+            Command::AddMaterial { name, color } => {
+                self.add_material_changes(name.as_deref(), *color)
+            }
+            Command::ChangeMaterial {
+                material,
+                name,
+                color,
+            } => self.change_material_changes(*material, name.as_deref(), *color),
+            Command::DeleteMaterial { material } => self.delete_material_changes(*material),
+            Command::Paint {
+                ids,
+                material,
+                side,
+            } => self.paint_changes(ids, *material, *side),
             Command::New { .. }
             | Command::Open { .. }
             | Command::Save { .. }
             | Command::Export { .. } => return None,
@@ -997,8 +1016,12 @@ impl Document {
                     })
                     .collect(),
                 revision: self.revision,
             },
+            Query::Materials => QueryResult::Materials {
+                materials: self.material_infos(),
+                revision: self.revision,
+            },
             Query::Context => QueryResult::Context {
                 path: self
                     .context
                     .iter()
@@ -1145,8 +1168,10 @@ impl Document {
             outer: ids(&f.outer),
             holes: f.holes.iter().map(ids).collect(),
             normal: f.normal.to_array(),
             area: self.model().area(id).unwrap(),
+            front_material: f.front,
+            back_material: f.back,
         }
     }
 }
 
```

Apply to `crates/session/src/document/groups.rs`:
```diff
--- a/crates/session/src/document/groups.rs
+++ b/crates/session/src/document/groups.rs
@@ -173,8 +173,9 @@ impl Document {
             bounds: self.instance_box(id).map(|[lo, hi]| Bounds {
                 min: lo.to_array(),
                 max: hi.to_array(),
             }),
+            material: i.material,
         }
     }
 }
 
```

Create `crates/session/src/document/materials.rs`:
```rust
//! Materials and painting.

use kernel::{EntityId, KernelError, MaterialId, SceneChange, Side};

use super::Document;
use crate::command::{CommandError, ErrorKind};
use crate::query::MaterialInfo;

/// The name `add_material` gives when none is passed (made unique with a number).
const DEFAULT_MATERIAL_NAME: &str = "Material";

impl Document {
    pub(super) fn add_material_changes(
        &mut self,
        name: Option<&str>,
        color: [u8; 3],
    ) -> Result<SceneChange, CommandError> {
        let name = match name {
            Some(n) => n.to_string(),
            None => self.scene.free_material_name(DEFAULT_MATERIAL_NAME),
        };
        self.scene
            .add_material(&name, color)
            .map(|(_, change)| change)
            .map_err(|e| material_error(e, None))
    }

    pub(super) fn change_material_changes(
        &mut self,
        id: MaterialId,
        name: Option<&str>,
        color: Option<[u8; 3]>,
    ) -> Result<SceneChange, CommandError> {
        self.scene
            .change_material(id, name, color)
            .map_err(|e| material_error(e, Some(id)))
    }

    pub(super) fn delete_material_changes(
        &mut self,
        id: MaterialId,
    ) -> Result<SceneChange, CommandError> {
        self.scene
            .delete_material(id)
            .map_err(|e| material_error(e, Some(id)))
    }

    /// Paints faces and instances of the open context. Edges and guides are invalid input,
    /// as are vertices and an empty list.
    pub(super) fn paint_changes(
        &mut self,
        ids: &[EntityId],
        material: Option<MaterialId>,
        side: Side,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one face or instance",
            ));
        }
        let parts = self.parts(ids)?;
        let others: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Edge(_) | EntityId::Guide(_)))
            .collect();
        if !others.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "only faces and instances can be painted",
            )
            .with_ids(others));
        }
        let mesh = self.active();
        self.scene
            .paint(mesh, &parts.faces, &parts.instances, side, material)
            .map_err(|e| material_error(e, material))
    }

    /// Every material with how many faces and instances use it.
    pub(super) fn material_infos(&self) -> Vec<MaterialInfo> {
        self.scene
            .materials()
            .map(|(id, m)| {
                let meshes = || self.scene.meshes().map(|(_, mesh)| mesh);
                MaterialInfo {
                    id,
                    name: m.name.clone(),
                    color: m.color,
                    faces: meshes()
                        .flat_map(|mesh| mesh.faces())
                        .filter(|(_, f)| f.paint().contains(&Some(id)))
                        .count(),
                    instances: meshes()
                        .flat_map(|mesh| mesh.instances())
                        .filter(|(_, i)| i.material == Some(id))
                        .count(),
                }
            })
            .collect()
    }
}

/// A kernel refusal as a command error; a missing material is `not_found`, naming `id`.
fn material_error(e: KernelError, id: Option<MaterialId>) -> CommandError {
    match (e, id) {
        (KernelError::MaterialNotFound, Some(id)) => {
            CommandError::new(ErrorKind::NotFound, format!("no material {id}"))
        }
        (KernelError::MaterialNotFound, None) => {
            CommandError::new(ErrorKind::NotFound, "no such material")
        }
        (e, _) => CommandError::new(ErrorKind::InvalidInput, e.to_string()),
    }
}
```

Apply to `crates/session/src/file.rs`:
```diff
--- a/crates/session/src/file.rs
+++ b/crates/session/src/file.rs
@@ -13,11 +13,11 @@ use crate::document::MAX_COORDINATE;
 
 /// The `format` every file records.
 pub const FORMAT: &str = "tenon";
 /// The schema version this build writes and the newest it reads. Version 2 added guides,
-/// version 3 curves and soft edges, and version 4 the camera's up direction and projection;
-/// older files still open.
-pub const SCHEMA_VERSION: u32 = 4;
+/// version 3 curves and soft edges, version 4 the camera's up direction and projection, and
+/// version 5 materials; older files still open.
+pub const SCHEMA_VERSION: u32 = 5;
 /// The archive entry holding the document.
 const ENTRY: &str = "document.json";
 /// The largest `document.json` this build reads, uncompressed.
 const MAX_DOCUMENT_BYTES: u64 = 256 * 1024 * 1024;
```

Apply to `crates/session/src/lib.rs`:
```diff
--- a/crates/session/src/lib.rs
+++ b/crates/session/src/lib.rs
@@ -15,11 +15,11 @@ pub use arc::{DEFAULT_ARC_SEGMENTS, arc_from_bulge, arc_points};
 pub use camera::{Camera, Projection, Ray, StandardView};
 pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
 pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
 pub use document::{Document, MAX_COORDINATE, MAX_COPIES};
-pub use kernel::CurveKind;
+pub use kernel::{CurveKind, MaterialId, Side};
 pub use query::{
     Bounds, ChangeEvent, CurveInfo, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo,
-    GuideKind, InstanceInfo, Query, QueryResult, VertexInfo,
+    GuideKind, InstanceInfo, MaterialInfo, Query, QueryResult, VertexInfo,
 };
 pub use rectangle::{rectangle_axes, rectangle_corners};
 pub use rotate::rotation;
```

Apply to `crates/session/src/query.rs`:
```diff
--- a/crates/session/src/query.rs
+++ b/crates/session/src/query.rs
@@ -1,5 +1,5 @@
-use kernel::{ChangeSummary, CurveKind, DefinitionId, DefinitionKind, EntityId};
+use kernel::{ChangeSummary, CurveKind, DefinitionId, DefinitionKind, EntityId, MaterialId};
 use schemars::JsonSchema;
 use serde::{Deserialize, Serialize};
 
 use crate::camera::Projection;
@@ -35,8 +35,10 @@ pub enum Query {
     /// The selected edges, faces, instances and guides.
     Selection,
     /// Every group and component definition.
     Definitions,
+    /// Every material.
+    Materials,
     /// The open context: the path of instances being edited, from the root.
     Context,
     /// The file the model was last opened from or saved to, and whether it changed since.
     File,
@@ -126,8 +128,14 @@ pub struct FaceInfo {
     pub outer: Vec<EntityId>,
     pub holes: Vec<Vec<EntityId>>,
     pub normal: [f64; 3],
     pub area: f64,
+    /// The material of the side the normal points to; absent for the default material.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub front_material: Option<MaterialId>,
+    /// The material of the other side; absent for the default material.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub back_material: Option<MaterialId>,
 }
 
 /// A placed group or component.
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
@@ -141,8 +149,11 @@ pub struct InstanceInfo {
     pub transform: [f64; 12],
     /// The box around the instance's geometry, in the open context's coordinates. Absent for
     /// an empty definition.
     pub bounds: Option<Bounds>,
+    /// The material shown on face sides inside it without one of their own; absent for none.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub material: Option<MaterialId>,
 }
 
 /// Whether a guide is a line or a point.
 #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
@@ -173,8 +184,21 @@ pub struct DefinitionInfo {
     pub edges: usize,
     pub faces: usize,
 }
 
+/// A material and how much of the model uses it.
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
+pub struct MaterialInfo {
+    pub id: MaterialId,
+    pub name: String,
+    /// sRGB red, green and blue, 0 to 255.
+    pub color: [u8; 3],
+    /// Faces painted with it on either side, in the whole model (each definition counted once).
+    pub faces: usize,
+    /// Instances painted with it, in the whole model.
+    pub instances: usize,
+}
+
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 #[serde(tag = "type", rename_all = "snake_case")]
 pub enum QueryResult {
     Entities {
@@ -223,8 +247,12 @@ pub enum QueryResult {
     Definitions {
         definitions: Vec<DefinitionInfo>,
         revision: u64,
     },
+    Materials {
+        materials: Vec<MaterialInfo>,
+        revision: u64,
+    },
     /// `path` lists the open instances from the root; empty when the root is open.
     /// `definition` is the definition being edited.
     Context {
         path: Vec<EntityId>,
```

Apply to `docs/agent-api.md`:
```diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -51,8 +51,9 @@ Queries:
 | `{"type": "stats"}`, `{"type": "history"}` | counts for the open context (plus the number of definitions) and revision; undo/redo depth |
 | `{"type": "camera"}` | the view: `eye`, `target`, `up`, `fov_y_deg`, `projection`, `view_height`; see Camera below |
 | `{"type": "selection"}` | the selected edge, face, instance and guide ids, plus `revision` |
 | `{"type": "definitions"}` | every group and component definition: `id`, `name`, `kind`, how many `instances` place it, `edges`, `faces` |
+| `{"type": "materials"}` | every material: `id`, `name`, `color`, and how many `faces` and `instances` in the whole model are painted with it; see Materials below |
 | `{"type": "context"}` | the open context: `path` (open instances from the root, empty at the root) and the `definition` being edited |
 | `{"type": "file"}` | `path` of the file last opened or saved (`null` if never), `modified` since then, and `revision` |
 | `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |
 
@@ -60,14 +61,15 @@ Unknown fields in commands, queries and params are rejected with `-32602` (a typ
 of silently doing nothing).
 
 Commands: `add_edge`, `add_rectangle`, `add_circle`, `add_arc`, `push_pull`, `erase`, `move`, `copy`, `rotate`, `make_group`,
 `make_component`, `explode`, `open_instance`, `close_instance`, `rename_definition`,
-`purge_definitions`, `add_guide_line`, `add_guide_point`, `delete_guides`, `select`, `undo`,
+`purge_definitions`, `add_guide_line`, `add_guide_point`, `delete_guides`, `add_material`,
+`change_material`, `delete_material`, `paint`, `select`, `undo`,
 `redo`, `set_camera`, `orbit`, `pan`, `zoom`, `zoom_extents`, `standard_view`, `set_projection`,
 `new`, `open`, `save`, `export`.
 Units are meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m.
 Entity ids are strings: `v12` (vertex), `e3` (edge), `f7` (face), `i4` (instance of a group or
-component), `g9` (guide line or guide point). Definitions are `d5`. Ids are unique across the whole model, including inside
+component), `g9` (guide line or guide point). Definitions are `d5`, materials `m6`. Ids are unique across the whole model, including inside
 groups and components.
 
 Modeling commands, each one undo step:
 
@@ -163,8 +165,43 @@ definition) and are saved with the model.
 guide keeps its id and is reported as `modified`). A guide's `entity` result, and each entry of
 the `guides` list in `entities`, is `{"id": "g9", "kind": "line", "point": [...], "direction":
 [...]}` (`kind` `point` has no `direction`). `stats` counts no guides.
 
+### Materials
+
+A material is a named color. Faces have two sides, each painted with a material or with none
+(the default material: light on the front, blue-gray on the back, as in the window); instances
+can be painted too. Materials belong to the whole model, not to the open context, and are saved
+with it.
+
+| Command | Effect |
+|---|---|
+| `{"type": "add_material", "name": "Brick", "color": [180, 60, 40]}` | Adds a material. `color` is sRGB red, green and blue, whole numbers 0 to 255. Without `name` it is called `Material` (`Material 2`, ... when taken). A name must not be blank, must have at most 256 characters and must differ from every other material's (`invalid_input`). The new id is the outcome's `changes.materials`, e.g. `["m31"]`. |
+| `{"type": "change_material", "material": "m31", "name": "Old brick", "color": [150, 50, 30]}` | Renames and/or recolors a material; an absent `name` or `color` stays. |
+| `{"type": "delete_material", "material": "m31"}` | Removes a material. Every face side and instance painted with it, anywhere in the model, goes back to the default material (and comes back with undo). |
+| `{"type": "paint", "ids": ["f8", "i4"], "material": "m31", "side": "front"}` | Paints faces and instances of the open context. `side` is `front` (default: the side the face's `normal` points to), `back` or `both`; it does not matter for instances. An absent or null `material` paints with the default material. Edge and guide ids are `invalid_input` (naming them), as are vertex ids and an empty list; unknown ids and an unknown material are `not_found`. Painting what is already painted so changes nothing. |
+
+Each is one undo step. A command that adds, removes, renames or recolors a material lists it in
+its `changes.materials` (also in `changed` notifications and `events`); painting reports the
+faces and instances as `modified`. Unknown material ids are `not_found`.
+
+The `entity` result of a face (and its entry in `entities`) has `front_material` and
+`back_material`, an instance's has `material`; each is absent for the default material.
+
+An instance's material shows on every face side inside it, at any depth, that has no material
+of its own; when instances inside each other are both painted, the one nearer the face wins.
+Painting an instance does not change the faces of its definition, so other instances of a
+component keep their own look. Exploding an instance paints its material onto the face sides
+and inner instances that come out without one.
+
+Paint follows the geometry: pieces of a face split by a new edge keep its materials, two faces
+merged by erasing the edge between them keep the materials of one of them (the one a point
+inside the merged face lies in), and copies, groups and components keep them. A
+face filling an opening is unpainted. Push/pull gives the extruded faces (the cap and the
+sides, also the walls and floor of a recess) the pushed face's front and back materials; a base
+that turns over (a pulled flat sheet) keeps each material on the side it was on, so its old
+front is now its back.
+
 ### Groups and components
 
 Commands and queries work on the open context: the root at first. `{"type": "open_instance",
 "id": "i4"}` opens an instance of the open context for editing, and `{"type": "close_instance"}`
@@ -232,12 +269,13 @@ projections; it sends these same commands.
 
 ### Files
 
 Models are saved as `.skr` files: a zip archive holding `document.json`, which records `format`
-(`"tenon"`), `schema_version` (4; version 3 files, written before camera projections,
-open in perspective, and versions 2 and 1, written before curves and guides, still open), the
-camera (eye, target, field of view, up and projection) and the model with every id, edges with
-their curve and soft flag.
+(`"tenon"`), `schema_version` (5; version 4 files, written before materials, open
+unpainted, version 3 files, written before camera projections, open in perspective, and
+versions 2 and 1, written before curves and guides, still open), the camera (eye, target, field
+of view, up and projection) and the model with every id, edges with their curve and soft flag,
+the materials and what is painted with them.
 
 | Command | Effect |
 |---|---|
 | `{"type": "save", "path": "/abs/house.skr"}` | Writes the file (the old file is replaced only once the new one is complete). Without `path` it saves to the file last opened or saved; a model never saved then gives `invalid_input`. |
@@ -250,10 +288,11 @@ subscribers get a `resync` notification and should read the model again. Ids in
 are those it was saved with, and new ids continue after the largest one ever used. A file that
 cannot be read or written gives the error kind `file`, with a message naming the path and the
 reason (a missing file, not a tenon file, a missing or newer schema version, a document
 over 256 MiB, ids over 2^53, a transform that cannot be inverted, a coordinate beyond the
-+/-1e6 m limit, a curve whose edges branch, fall apart or are straight, or a soft edge bounding
-fewer than two faces); the model is left as it was. `open` and `new` never drop changes the user has
++/-1e6 m limit, a curve whose edges branch, fall apart or are straight, a soft edge bounding
+fewer than two faces, a face or instance painted with a material the file lacks, or a material
+name that is blank, too long or used twice); the model is left as it was. `open` and `new` never drop changes the user has
 not saved (`modified` in the `file` query) unless told to: they fail with `unsaved_changes`, and
 a client that means it, or has asked the user, passes `"discard": true`. The window asks the user
 itself before its own New and Open. `new`, `open`, `save` and `export` are not allowed in an
 atomic batch.
```

Apply to `docs/superpowers/specs/2026-10-10-tenon-design.md`:
```diff
--- a/docs/superpowers/specs/2026-10-10-tenon-design.md
+++ b/docs/superpowers/specs/2026-10-10-tenon-design.md
@@ -1,8 +1,8 @@
 # Tenon — Design
 
 Date: 2026-10-10
-Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides, M6d curves, M7a camera)
+Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides, M6d curves, M7a camera, M7b materials)
 
 ## Goal
 
 A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.
@@ -56,9 +56,9 @@ Dependency direction: `app -> {api, tools, render} -> session -> kernel`. `tools
 SketchUp geometry is non-manifold (an edge can border three or more faces; edges can dangle), so the kernel uses an edge list with faces as vertex loops rather than a strict half-edge mesh.
 
 - **Vertex**: `DVec3` position.
 - **Edge**: two vertex ids; an optional curve id (M6d: the edges of a circle or arc share one; the curve's shape is worked out from its edges, never stored); a soft flag (M6d: hidden, between faces of one smooth surface); smooth and hidden flags added when needed.
-- **Face**: outer vertex loop (counter-clockwise around the normal) + hole loops (clockwise); unit normal; front/back material (added with materials, M5). Consecutive loop vertices are always joined by an edge.
+- **Face**: outer vertex loop (counter-clockwise around the normal) + hole loops (clockwise); unit normal; front/back material (M7b: each side an optional material id, none being the default material). Consecutive loop vertices are always joined by an edge.
 - Ids are monotonically increasing `u64`s, never reused, stored in `BTreeMap`s, so undo can restore entities under their original ids. Adjacency is found by scanning until model sizes require an index.
 
 ### Sticky geometry (kernel responsibility)
 
@@ -198,8 +198,9 @@ Every milestone's acceptance includes: new capabilities are reachable as `sessio
 - **M6b**: rotate: a `rotate` command (vertices, edges, faces and instances about an axis through a center, angle in degrees, exact quarter turns; optional `copies` for radial arrays) and a Rotate tool (`q`: center on a face turns about its normal, else about Z; reference click, then angle click with 15-degree snapping, or a typed angle; Ctrl copies, "5x" and "/5" arrays). Turned placements work everywhere: render normals, picking, box select, inference and the tools' camera inside an open turned instance, bounds, Entity Info (rotation line), explode, files and export.
 - **M6c**: guides and the Tape Measure: guide lines (infinite) and guide points are model entities (`g<n>` ids) of the context they are drawn in; `add_guide_line`, `add_guide_point` and `delete_guides` commands, and `erase`, `select`, `move`, `rotate`, `copy`, `make_group` and `explode` take guides; saved in `.skr` (schema version 2; version 1 still opens), never exported. Inference snaps to guide points, onto guide lines, and where guide lines cross edges or each other. Guides are drawn dashed (guide points as small crosses), picked, box-selected and erased like edges, and shown in Entity Info. Tape Measure tool (`t`): from a point the second click adds a guide point, from an edge or guide line a parallel guide line; typed distances, retyped right after to adjust; Ctrl switches to measuring only. Edit > Delete Guides.
 - **M6d**: curves: edges carry an optional curve id; the edges of a curve always form one unbranched, non-straight chain (sticky splits keep both pieces on the curve, erasing leaves the rest a curve, a piece that falls off gets a new id, a lone edge is plain), and copies, groups and explodes get new curve ids; the shape (circle or arc, center, normal, radius, angle, segments, length) is derived from the edges, so moves and rotations need no upkeep; `add_circle` tags its edges; an `add_arc` command (center, start, signed angle in degrees, plane normal, 2-360 segments, default 12); a `curve` query and `curve`/`soft` fields on edges; `.skr` schema version 3 (versions 1 and 2 still open). Push/pull of a face bounded by a curve gives the cap a curve of its own and makes the swept edges between curve segments soft; a soft edge bounding fewer than two faces hardens. In the window, soft edges are not drawn, picked or snapped to; clicking a curve edge with Select selects the whole curve; Entity Info shows circles and arcs with radius, angle, segments and length. Arc tool (`a`, SketchUp's 2-point arc: start, end, bulge; typed chord and bulge lengths, "12s" segments). Smooth shading across soft edges is deferred.
 - **M7a**: camera: commands `orbit` (yaw and pitch in degrees about the target), `pan` (meters along the screen), `zoom` (factor), `zoom_extents` (the whole model in root coordinates, guides excluded; an empty model leaves the camera), `standard_view` (top, bottom, front, back, left, right, iso; same target and distance) and `set_projection` (perspective or parallel; the view height at the target is kept, so a parallel view's zoom is the eye's distance); `set_camera` takes `up` (the screen's up in views straight down or up, which are now allowed) and `projection`; the `camera` query reports `up`, `projection` and `view_height`; camera commands are not undo steps and do not change the revision; `.skr` schema version 4 saves `up` and `projection` (versions 1-3 open in perspective). A parallel view sees 100 eye-to-target distances either side of the target, behind the eye too: rays start there, and picking, inference, overlay sizes (snap markers, guide crosses, pick radius, protractor), guide line spans, render depth offsets and shading work in both projections; `render_view` takes `up` and `projection`. Camera menu: Zoom Extents (Shift+Z), the standard views, Perspective and Parallel Projection; camera moves from the menu keep the tool's operation going. Erase Selection moves from the toolbar to the Edit menu so the toolbar keeps fitting a 1280-point window.
+- **M7b**: materials: named sRGB colors (`m<n>` ids from the entity id counter) belonging to the whole model; each face side and each instance carries an optional material (none is the default material); an instance's material shows on face sides inside it without one (the nearest painted instance wins), and exploding bakes it onto them. Commands `add_material` (optional name, default "Material", "Material 2", ...; names distinct, not blank, at most 256 characters), `change_material` (name and/or color), `delete_material` (unpaints everything painted with it, one undo step) and `paint` (faces and instances of the open context, `side` front, back or both, null for the default); a `materials` query (with face and instance use counts); `front_material`/`back_material` on faces and `material` on instances in queries; `changes.materials` lists materials a command added, removed or changed. Paint follows geometry: split pieces keep it, merges keep the containing face's, copies/groups/components keep it, push/pull gives the cap and sides the pushed face's materials, a face turned over keeps each material on its side. `.skr` schema version 5 (versions 1-4 open unpainted); files painting with missing materials or with bad material names are refused. Rendering shows each side's color (selection tint on top); export writes OBJ with a `.mtl` beside it, glTF with one material per color (front side only; STL has no colors). Paint Bucket tool (`b`: click a face to paint the side facing the viewer, a selected face paints the whole selection's faces and instances, an instance paints the instance; Cmd/Ctrl-click samples a material) and a Materials window (Window menu) listing materials with swatches, add, rename, recolor, delete and the current material. Opacity and textures are deferred.
 
 ## Risks
 
 - **Sticky geometry robustness**: edge merging and face detection under floating-point error is the main risk. Mitigation: robust predicates, explicit tolerances, invariant checks in every test, fuzz tests on random edge sequences.
```

- [ ] **Step 4: Run the tests to verify they pass**

same command 7 passed; `cargo test -p api --test dispatch describe` passes; workspace 700 passed; fmt and clippy clean.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` (expected totals: 693, 700, 705, 711 after Tasks 1-4).

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/api/tests/dispatch.rs crates/session/tests/camera.rs crates/session/tests/curves.rs crates/session/tests/materials.rs crates/session/src/command.rs crates/session/src/document.rs crates/session/src/document/groups.rs crates/session/src/document/materials.rs crates/session/src/file.rs crates/session/src/lib.rs crates/session/src/query.rs docs/agent-api.md docs/superpowers/specs/2026-10-10-tenon-design.md
git commit -m "Material commands: add_material, change_material, delete_material, paint; materials query and face/instance materials in queries; .skr schema 5; spec M7b"
```

### Task 3: Materials in rendering and export

**Files:**
- Modify: `crates/kernel/src/triangles.rs` (Triangle.front/back; collect_triangles takes `inherited`), `crates/render/src/scene.rs` (FaceVertex.front/back, DEFAULT_FRONT/DEFAULT_BACK, side_color, Walk::mesh takes `inherited`; unit test), `crates/render/src/renderer.rs` (`FACE_ATTRIBS: [wgpu::VertexAttribute; 5]`, attributes 3 and 4 `Float32x3`), `crates/render/src/shaders/scene.wgsl` (FaceOut front_color/back_color at locations 3/4, constants FRONT/BACK removed), `crates/session/src/export.rs` (Encoded, encode(scene, triangles, format, mtl_name), Groups/Group, linear, obj_names, obj returns (obj, Option<mtl>), gltf_parts per group), `crates/session/src/document/files.rs` (own-file closure; mtl path = target.with_extension("mtl"); write .mtl first; refusals)
- Modify (tests): `crates/kernel/tests/triangles.rs` (new test appended), `crates/render/tests/headless.rs` (render_with delegates to new `render_scene_with(camera, &Scene, selection, viewport, overlay)`; new GPU test; imports MeshId, Side), `crates/session/tests/export.rs` (helper `painted_boxes` + 2 tests appended)
- Docs: `docs/agent-api.md` Export table rows for obj/stl/glb and a paragraph on colors and refusals

**Interfaces:**
- Produces:
  - `kernel::Triangle { corners, normal, front: Option<MaterialId>, back: Option<MaterialId> }`
  - `render::scene::FaceVertex { pos, normal, selected, front: [f32; 3], back: [f32; 3] }`; `pub const DEFAULT_FRONT: [f32; 3] = [0.93, 0.93, 0.90]`, `DEFAULT_BACK = [0.60, 0.65, 0.76]`
  - `pub(crate) struct Encoded { bytes: Vec<u8>, mtl: Option<Vec<u8>> }`; `pub(crate) fn encode(scene: &Scene, triangles: &[Triangle], format: ExportFormat, mtl_name: &str) -> Encoded`

- [ ] **Step 1: Write the failing tests**

Tests: kernel `triangles_carry_face_materials_and_inherit_the_nearest_painted_instance` (integration file, appended); render unit test hunk `faces_take_their_material_colors_or_the_nearest_painted_instance` in `scene.rs` tests module; render GPU test `painted_sides_show_their_colors_and_selection_still_tints` (headless.rs; skips without a GPU like the others); session `obj_of_painted_faces_names_a_material_library_beside_it`, `gltf_has_one_primitive_and_material_per_color` (export.rs, appended).

Test-module hunks are part of the Step 3 diffs; apply them first, along with the test files below.

Apply to `crates/kernel/tests/triangles.rs`:
```diff
--- a/crates/kernel/tests/triangles.rs
+++ b/crates/kernel/tests/triangles.rs
@@ -107,4 +107,65 @@ fn a_mirrored_instance_keeps_its_triangles_wound_around_outward_normals() {
         let mid = (a + b + c) / 3.0;
         assert!((mid - centre).dot(t.normal) > 0.0, "faces out of the box");
     }
 }
+
+#[test]
+fn triangles_carry_face_materials_and_inherit_the_nearest_painted_instance() {
+    let mut s = Scene::from_root(box_model());
+    let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
+    let (blue, _) = s.add_material("Blue", [0, 0, 255]).unwrap();
+    let (green, _) = s.add_material("Green", [0, 255, 0]).unwrap();
+    let top = s
+        .root()
+        .faces()
+        .find(|(_, f)| f.normal == DVec3::Z)
+        .unwrap()
+        .0;
+    s.paint(MeshId::Root, &[top], &[], kernel::Side::Front, Some(red))
+        .unwrap();
+    let tris = s.triangles();
+    assert_eq!(
+        tris.iter().filter(|t| t.front == Some(red)).count(),
+        2,
+        "the top's two triangles"
+    );
+    assert!(tris.iter().all(|t| t.back.is_none()));
+
+    // Group the box, then group the group: paint the inner instance blue and the outer green.
+    let edges: Vec<_> = s.root().edges().map(|(e, _)| e).collect();
+    let faces: Vec<_> = s.root().faces().map(|(f, _)| f).collect();
+    let (inner, _) = s
+        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
+        .unwrap();
+    let (outer, _) = s
+        .make_group(MeshId::Root, &[], &[], &[inner], DefinitionKind::Group, "")
+        .unwrap();
+    let d = s.root().instance(outer).unwrap().definition;
+    s.paint(
+        MeshId::Root,
+        &[],
+        &[outer],
+        kernel::Side::Front,
+        Some(green),
+    )
+    .unwrap();
+    let tris = s.triangles();
+    assert_eq!(tris.iter().filter(|t| t.front == Some(green)).count(), 10);
+    assert!(tris.iter().all(|t| t.back == Some(green)));
+    s.paint(
+        MeshId::Definition(d),
+        &[],
+        &[inner],
+        kernel::Side::Front,
+        Some(blue),
+    )
+    .unwrap();
+    let tris = s.triangles();
+    assert_eq!(tris.iter().filter(|t| t.front == Some(red)).count(), 2);
+    assert_eq!(
+        tris.iter().filter(|t| t.front == Some(blue)).count(),
+        10,
+        "the nearer instance wins"
+    );
+    assert!(tris.iter().all(|t| t.back == Some(blue)));
+}
```

Apply to `crates/render/tests/headless.rs`:
```diff
--- a/crates/render/tests/headless.rs
+++ b/crates/render/tests/headless.rs
@@ -1,8 +1,8 @@
 use std::collections::BTreeSet;
 
 use glam::{DVec2, DVec3};
-use kernel::{EntityId, Model, Scene};
+use kernel::{EntityId, MeshId, Model, Scene, Side};
 use render::grid::{BACKGROUND, LineVertex};
 use render::{Renderer, Viewport};
 use session::Camera;
 
@@ -38,8 +38,20 @@ fn render_with(
     model: &Model,
     selection: &BTreeSet<EntityId>,
     viewport: Viewport,
     overlay: &[LineVertex],
+) -> Option<Vec<u8>> {
+    let scene = Scene::from_root(model.clone());
+    render_scene_with(camera, &scene, selection, viewport, overlay)
+}
+
+/// Renders a whole scene (with its materials) with `selection` highlighted into `viewport`.
+fn render_scene_with(
+    camera: &Camera,
+    scene: &Scene,
+    selection: &BTreeSet<EntityId>,
+    viewport: Viewport,
+    overlay: &[LineVertex],
 ) -> Option<Vec<u8>> {
     let Some((device, queue)) = gpu() else {
         eprintln!("no GPU adapter, skipping");
         return None;
@@ -60,9 +72,9 @@ fn render_with(
         view_formats: &[],
     });
     let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
     let mut renderer = Renderer::new(&device, &queue, format);
-    renderer.set_scene(&Scene::from_root(model.clone()), &[], selection);
+    renderer.set_scene(scene, &[], selection);
     renderer.render_in(&view, (W, H), viewport, camera, overlay);
 
     let buffer = device.create_buffer(&wgpu::BufferDescriptor {
         label: Some("readback"),
@@ -351,8 +363,48 @@ fn selected_faces_are_tinted_blue() {
         "selected face should be blue, got {p:?}"
     );
 }
 
+#[test]
+fn painted_sides_show_their_colors_and_selection_still_tints() {
+    let mut scene = Scene::from_root(ground_square());
+    let (red, _) = scene.add_material("Red", [220, 20, 20]).unwrap();
+    let (green, _) = scene.add_material("Green", [20, 200, 20]).unwrap();
+    let face = scene.root().faces().next().unwrap().0;
+    scene
+        .paint(MeshId::Root, &[face], &[], Side::Front, Some(red))
+        .unwrap();
+    scene
+        .paint(MeshId::Root, &[face], &[], Side::Back, Some(green))
+        .unwrap();
+    let cam = camera();
+    let at = DVec3::new(0.5, 0.5, 0.0);
+    let full = Viewport::full((W, H));
+    let Some(px) = render_scene_with(&cam, &scene, &BTreeSet::new(), full, &[]) else {
+        return;
+    };
+    let p = pixel_at(&px, &cam, at);
+    assert!(
+        p[0] as i32 > p[1] as i32 + 80,
+        "front should be red, got {p:?}"
+    );
+    let mut below = camera();
+    below.eye = DVec3::new(-8.0, -12.0, -7.0);
+    let px = render_scene_with(&below, &scene, &BTreeSet::new(), full, &[]).unwrap();
+    let p = pixel_at(&px, &below, at);
+    assert!(
+        p[1] as i32 > p[0] as i32 + 80,
+        "back should be green, got {p:?}"
+    );
+    let selection = BTreeSet::from([EntityId::Face(face)]);
+    let px = render_scene_with(&cam, &scene, &selection, full, &[]).unwrap();
+    let p = pixel_at(&px, &cam, at);
+    assert!(
+        p[2] as i32 > 60 && (p[2] as i32) > p[1] as i32,
+        "selected red face should be tinted blue, got {p:?}"
+    );
+}
+
 #[test]
 fn scene_stays_inside_its_viewport() {
     // The left half holds the scene, sized to match; the right half is only background.
     let mut cam = camera();
```

Apply to `crates/session/tests/export.rs`:
```diff
--- a/crates/session/tests/export.rs
+++ b/crates/session/tests/export.rs
@@ -373,4 +373,90 @@ fn a_face_with_a_hole_in_nested_groups_exports_whole_and_undo_still_works() {
         top["instances"][0]["transform"][9], 0.0,
         "undo undid the move, not an export: {top}"
     );
 }
+
+/// `two_boxes` with the first crate painted with a new material named `name` and `color`.
+fn painted_boxes(d: &mut Document, name: &str, color: [u8; 3]) {
+    two_boxes(d);
+    let out = run(
+        d,
+        json!({"type": "add_material", "name": name, "color": color}),
+    )
+    .unwrap();
+    let material = out["changes"]["materials"][0].clone();
+    let instances = ask(d, json!({"type": "entities", "kinds": ["instance"]}));
+    let first = instances["instances"][0]["id"].clone();
+    run(
+        d,
+        json!({"type": "paint", "ids": [first], "material": material}),
+    )
+    .unwrap();
+}
+
+#[test]
+fn obj_of_painted_faces_names_a_material_library_beside_it() {
+    let mut d = doc();
+    two_boxes(&mut d);
+    let mtl = scratch("plain-boxes.mtl");
+    let obj = String::from_utf8(export(&mut d, "plain-boxes.obj")).unwrap();
+    assert!(!obj.contains("mtllib") && !obj.contains("usemtl"));
+    assert!(!mtl.exists(), "no library without painted faces");
+
+    let mut d = doc();
+    painted_boxes(&mut d, "Red brick", [255, 0, 0]);
+    let mtl = scratch("painted-boxes.mtl");
+    let obj = String::from_utf8(export(&mut d, "painted-boxes.obj")).unwrap();
+    assert!(obj.contains("mtllib painted-boxes.mtl\n"), "{obj}");
+    let uses: Vec<&str> = obj.lines().filter(|l| l.starts_with("usemtl")).collect();
+    assert_eq!(uses, ["usemtl Red_brick", "usemtl default"]);
+    assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 24);
+    let library = std::fs::read_to_string(&mtl).unwrap();
+    assert!(
+        library.contains("newmtl Red_brick\nKd 1.000000 0.000000 0.000000\n"),
+        "{library}"
+    );
+    assert!(library.contains("newmtl default\n"), "{library}");
+
+    // The library must not overwrite the OBJ itself.
+    let path = scratch("painted.mtl");
+    let (kind, message) = run(
+        &mut d,
+        json!({"type": "export", "path": path.to_str().unwrap(), "format": "obj"}),
+    )
+    .unwrap_err();
+    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
+    assert!(!path.exists());
+}
+
+#[test]
+fn gltf_has_one_primitive_and_material_per_color() {
+    let mut d = doc();
+    painted_boxes(&mut d, "Clay", [128, 64, 255]);
+    let doc: Value = serde_json::from_slice(&export(&mut d, "painted.gltf")).unwrap();
+    let primitives = doc["meshes"][0]["primitives"].as_array().unwrap();
+    assert_eq!(primitives.len(), 2);
+    assert_eq!(doc["materials"][0]["name"], "Clay");
+    assert_eq!(doc["materials"][1]["name"], "default");
+    assert!(doc["materials"][1]["pbrMetallicRoughness"]["baseColorFactor"].is_null());
+    // glTF colors are linear: sRGB 128 is about 0.216.
+    let factor = doc["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"].clone();
+    let f: Vec<f64> = factor
+        .as_array()
+        .unwrap()
+        .iter()
+        .map(|x| x.as_f64().unwrap())
+        .collect();
+    assert!(
+        (f[0] - 0.2158).abs() < 1e-3 && (f[1] - 0.0513).abs() < 1e-3,
+        "{f:?}"
+    );
+    assert_eq!((f[2], f[3]), (1.0, 1.0));
+    for (k, p) in primitives.iter().enumerate() {
+        assert_eq!(p["material"], k);
+        let pos = &doc["accessors"][p["attributes"]["POSITION"].as_u64().unwrap() as usize];
+        assert_eq!(pos["count"], 36);
+        assert_eq!(pos["byteOffset"], 36 * 12 * k);
+    }
+    // The second box (the unpainted one) is the default primitive: x from 3 to 5.
+    assert_eq!(doc["accessors"][2]["min"][0], 3.0);
+}
```

- [ ] **Step 2: Run the tests to verify they fail**

kernel/render tests fail to compile (no `Triangle.front`, `FaceVertex.front`, `render_scene_with`). Verified (session/src at p2, kernel/render at p3): `cargo test -p session --test export` -> 2 failed (`gltf_has_one_primitive_and_material_per_color` at the primitive count, `obj_of_painted_faces_names_a_material_library_beside_it` at the mtllib assert), 12 passed.

- [ ] **Step 3: Implement**

Apply to `crates/kernel/src/triangles.rs`:
```diff
--- a/crates/kernel/src/triangles.rs
+++ b/crates/kernel/src/triangles.rs
@@ -1,15 +1,20 @@
 //! Every face of a scene as triangles in root coordinates, for export.
 
 use glam::{DAffine3, DVec3};
 
+use crate::ids::MaterialId;
 use crate::scene::{MeshId, Scene};
 
 /// One triangle of a face, wound counter-clockwise around `normal` (the face's front).
 #[derive(Clone, Copy, Debug, PartialEq)]
 pub struct Triangle {
     pub corners: [DVec3; 3],
     pub normal: DVec3,
+    /// The material shown on the front and on the back: the face's own, or else that of the
+    /// nearest painted instance placing it; None for the default material.
+    pub front: Option<MaterialId>,
+    pub back: Option<MaterialId>,
 }
 
 impl Scene {
     /// Every face the scene shows, as triangles in root coordinates: the root's faces and,
@@ -17,32 +22,43 @@ impl Scene {
     /// placed twice gives its faces twice. Edges without faces give nothing. The order is
     /// fixed: faces by id, then instances by id, depth first.
     pub fn triangles(&self) -> Vec<Triangle> {
         let mut out = Vec::new();
-        self.collect_triangles(MeshId::Root, DAffine3::IDENTITY, &mut out);
+        self.collect_triangles(MeshId::Root, DAffine3::IDENTITY, None, &mut out);
         out
     }
 
-    fn collect_triangles(&self, id: MeshId, place: DAffine3, out: &mut Vec<Triangle>) {
+    /// `inherited` is the material of the nearest painted instance placing the mesh.
+    fn collect_triangles(
+        &self,
+        id: MeshId,
+        place: DAffine3,
+        inherited: Option<MaterialId>,
+        out: &mut Vec<Triangle>,
+    ) {
         let Some(model) = self.mesh(id) else { return };
         // A mirroring placement turns triangles inside out; swapping two corners restores the
         // winding. Normals follow the inverse transpose, which is right for any linear map.
         let mirrored = place.matrix3.determinant() < 0.0;
         let normal_map = place.matrix3.inverse().transpose();
         for (fid, face) in model.faces() {
             let normal = (normal_map * face.normal).normalize();
+            let (front, back) = (face.front.or(inherited), face.back.or(inherited));
             for tri in model.triangulate(fid).unwrap_or_default() {
                 let [a, b, c] = tri.map(|p| place.transform_point3(p));
                 out.push(Triangle {
                     corners: if mirrored { [a, c, b] } else { [a, b, c] },
                     normal,
+                    front,
+                    back,
                 });
             }
         }
         for (_, instance) in model.instances() {
             self.collect_triangles(
                 MeshId::Definition(instance.definition),
                 place * instance.transform,
+                instance.material.or(inherited),
                 out,
             );
         }
     }
```

Apply to `crates/render/src/renderer.rs`:
```diff
--- a/crates/render/src/renderer.rs
+++ b/crates/render/src/renderer.rs
@@ -12,10 +12,9 @@ pub const SAMPLE_COUNT: u32 = 4;
 pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
 
 const LINE_ATTRIBS: [wgpu::VertexAttribute; 2] =
     wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
-const FACE_ATTRIBS: [wgpu::VertexAttribute; 3] =
-    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32];
+const FACE_ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32, 3 => Float32x3, 4 => Float32x3];
 
 /// Camera data shared by every pipeline (matches `Uniforms` in scene.wgsl).
 #[repr(C)]
 #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
```

Apply to `crates/render/src/scene.rs`:
```diff
--- a/crates/render/src/scene.rs
+++ b/crates/render/src/scene.rs
@@ -3,9 +3,9 @@
 use std::collections::BTreeSet;
 
 use bytemuck::{Pod, Zeroable};
 use glam::{DAffine3, DVec3};
-use kernel::{EntityId, Guide, InstanceId, MeshId, Scene};
+use kernel::{EntityId, Guide, InstanceId, MaterialId, MeshId, Scene};
 
 use crate::grid::LineVertex;
 use crate::guides::{FADED_GUIDE_COLOR, GUIDE_COLOR, GuideShape};
 
@@ -15,8 +15,23 @@ pub struct FaceVertex {
     pub pos: [f32; 3],
     pub normal: [f32; 3],
     /// 1 for selected faces, 0 otherwise.
     pub selected: f32,
+    /// The colors of the front and back sides.
+    pub front: [f32; 3],
+    pub back: [f32; 3],
+}
+
+/// The default material's color on a face's front.
+pub const DEFAULT_FRONT: [f32; 3] = [0.93, 0.93, 0.90];
+/// The default material's color on a face's back.
+pub const DEFAULT_BACK: [f32; 3] = [0.60, 0.65, 0.76];
+
+/// The color a side shows: its material's (sRGB, as the targets store it), or `default`.
+fn side_color(scene: &Scene, material: Option<MaterialId>, default: [f32; 3]) -> [f32; 3] {
+    material
+        .and_then(|m| scene.material(m))
+        .map_or(default, |m| m.color.map(|c| f32::from(c) / 255.0))
 }
 
 pub const EDGE_COLOR: [f32; 4] = [0.08, 0.08, 0.08, 1.0];
 pub const SELECTED_EDGE_COLOR: [f32; 4] = [0.10, 0.35, 0.95, 1.0];
@@ -38,9 +53,10 @@ pub struct ModelGeometry {
 /// Geometry for the whole scene in root coordinates, every instance placed by its transform.
 /// `context` is the path of open instances (see `session::Document::context`) and `selection`
 /// holds ids of the open context: selected entities there are highlighted, and a selected
 /// instance highlights everything it places. While an instance is open, edges and guides
-/// outside it are faded and its bounding box is outlined.
+/// outside it are faded and its bounding box is outlined. Each face side gets its material's
+/// color, or else that of the nearest painted instance placing it, or else the default.
 pub fn scene_geometry(
     scene: &Scene,
     context: &[InstanceId],
     selection: &BTreeSet<EntityId>,
@@ -52,9 +68,9 @@ pub fn scene_geometry(
         selection,
         path: Vec::new(),
         g: &mut g,
     };
-    walk.mesh(MeshId::Root, DAffine3::IDENTITY, false);
+    walk.mesh(MeshId::Root, DAffine3::IDENTITY, false, None);
     if !context.is_empty()
         && let Some((mesh, t)) = scene.resolve(context)
         && let Some([lo, hi]) = scene.bounds(mesh)
     {
@@ -103,10 +119,11 @@ struct Walk<'a> {
     g: &'a mut ModelGeometry,
 }
 
 impl Walk<'_> {
-    /// Adds a mesh placed by `t`; `lit` highlights all of it (it is inside a selected instance).
-    fn mesh(&mut self, id: MeshId, t: DAffine3, lit: bool) {
+    /// Adds a mesh placed by `t`; `lit` highlights all of it (it is inside a selected instance)
+    /// and `inherited` is the material of the nearest painted instance placing it.
+    fn mesh(&mut self, id: MeshId, t: DAffine3, lit: bool, inherited: Option<MaterialId>) {
         let Some(model) = self.scene.mesh(id) else {
             return;
         };
         // Selection ids refer to the open context's placement only, not to other placements
@@ -123,14 +140,18 @@ impl Walk<'_> {
                 1.0
             } else {
                 0.0
             };
+            let front = side_color(self.scene, face.front.or(inherited), DEFAULT_FRONT);
+            let back = side_color(self.scene, face.back.or(inherited), DEFAULT_BACK);
             for tri in model.triangulate(fid).unwrap_or_default() {
                 for p in tri {
                     self.g.faces.push(FaceVertex {
                         pos: t.transform_point3(p).as_vec3().to_array(),
                         normal,
                         selected: flag,
+                        front,
+                        back,
                     });
                 }
             }
         }
@@ -176,8 +197,9 @@ impl Walk<'_> {
             self.mesh(
                 MeshId::Definition(instance.definition),
                 t * instance.transform,
                 lit,
+                instance.material.or(inherited),
             );
             self.path.pop();
         }
     }
@@ -268,8 +290,47 @@ mod tests {
         let g = geometry(m, &all);
         assert_eq!(g.edges.len(), 2 * 6, "the two rims, selected or not");
     }
 
+    #[test]
+    fn faces_take_their_material_colors_or_the_nearest_painted_instance() {
+        let (mut s, first, second) = two_squares();
+        let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
+        let (blue, _) = s.add_material("Blue", [0, 0, 255]).unwrap();
+        let g = scene_geometry(&s, &[], &BTreeSet::new());
+        assert!(
+            g.faces
+                .iter()
+                .all(|v| v.front == DEFAULT_FRONT && v.back == DEFAULT_BACK)
+        );
+        // The first square's instance is blue; the component's face is red on its back only.
+        s.paint(MeshId::Root, &[], &[first], kernel::Side::Front, Some(blue))
+            .unwrap();
+        let d = s.root().instance(first).unwrap().definition;
+        let face = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
+        s.paint(
+            MeshId::Definition(d),
+            &[face],
+            &[],
+            kernel::Side::Back,
+            Some(red),
+        )
+        .unwrap();
+        let g = scene_geometry(&s, &[], &BTreeSet::new());
+        let (one, two): (Vec<&FaceVertex>, Vec<&FaceVertex>) =
+            g.faces.iter().partition(|v| v.pos[0] <= 1.0);
+        assert!(
+            one.iter()
+                .all(|v| v.front == [0.0, 0.0, 1.0] && v.back == [1.0, 0.0, 0.0])
+        );
+        assert!(
+            two.iter()
+                .all(|v| v.front == DEFAULT_FRONT && v.back == [1.0, 0.0, 0.0]),
+            "the second placement shares the face but not the instance's blue"
+        );
+        let _ = second;
+    }
+
     #[test]
     fn selected_entities_are_marked() {
         let m = square();
         let face = m.faces().next().unwrap().0;
```

Apply to `crates/render/src/shaders/scene.wgsl`:
```diff
--- a/crates/render/src/shaders/scene.wgsl
+++ b/crates/render/src/shaders/scene.wgsl
@@ -56,34 +56,39 @@ struct FaceOut {
     @builtin(position) pos: vec4<f32>,
     @location(0) world: vec3<f32>,
     @location(1) normal: vec3<f32>,
     @location(2) selected: f32,
+    @location(3) front_color: vec3<f32>,
+    @location(4) back_color: vec3<f32>,
 };
 
-const FRONT: vec3<f32> = vec3<f32>(0.93, 0.93, 0.90);
-const BACK: vec3<f32> = vec3<f32>(0.60, 0.65, 0.76);
 const SELECTED: vec3<f32> = vec3<f32>(0.25, 0.45, 0.95);
 
 @vertex
 fn vs_face(
     @location(0) pos: vec3<f32>,
     @location(1) normal: vec3<f32>,
     @location(2) selected: f32,
+    @location(3) front_color: vec3<f32>,
+    @location(4) back_color: vec3<f32>,
 ) -> FaceOut {
     var out: FaceOut;
     out.pos = u.view_proj * vec4<f32>(pos, 1.0);
     out.world = pos;
     out.normal = normal;
     out.selected = selected;
+    out.front_color = front_color;
+    out.back_color = back_color;
     return out;
 }
 
-// Front sides are light, back sides blue-gray; a headlight at the eye (from the viewer, in a
-// parallel view) shades both. Selected faces are tinted blue.
+// Each side shows its own color (its material's, or the default: light in front, blue-gray
+// behind); a headlight at the eye (from the viewer, in a parallel view) shades both. Selected
+// faces are tinted blue.
 @fragment
 fn fs_face(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
     let n = normalize(in.normal) * select(-1.0, 1.0, front);
     let to_eye = normalize(u.eye.xyz - in.world * u.eye.w);
     let shade = 0.6 + 0.4 * max(dot(n, to_eye), 0.0);
-    let base = mix(select(BACK, FRONT, front), SELECTED, 0.5 * in.selected);
+    let base = mix(select(in.back_color, in.front_color, front), SELECTED, 0.5 * in.selected);
     return vec4<f32>(base * shade, 1.0);
 }
```

Apply to `crates/session/src/document/files.rs`:
```diff
--- a/crates/session/src/document/files.rs
+++ b/crates/session/src/document/files.rs
@@ -81,29 +81,50 @@ impl Document {
                 ErrorKind::InvalidInput,
                 "a .skr path is a model file: use save, not export",
             ));
         }
-        if let Some(own) = &self.path {
-            let same = match (own.canonicalize(), target.canonicalize()) {
-                (Ok(a), Ok(b)) => a == b,
-                _ => own == target,
-            };
-            if same {
-                return Err(CommandError::new(
-                    ErrorKind::InvalidInput,
-                    "that is the open model's own file: export to another path",
-                ));
-            }
+        let own = |p: &Path| {
+            self.path
+                .as_ref()
+                .is_some_and(|own| match (own.canonicalize(), p.canonicalize()) {
+                    (Ok(a), Ok(b)) => a == b,
+                    _ => own == p,
+                })
+        };
+        if own(target) {
+            return Err(CommandError::new(
+                ErrorKind::InvalidInput,
+                "that is the open model's own file: export to another path",
+            ));
         }
         let triangles = self.scene.triangles();
         if triangles.is_empty() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
                 "nothing to export: the model has no faces",
             ));
         }
-        file::replace_file(Path::new(path), &export::encode(&triangles, format))
-            .map_err(|e| file_error(path, e))
+        // An OBJ with painted faces names a material library beside it: same name, `.mtl`.
+        let mtl_path = target.with_extension("mtl");
+        let mtl_name = mtl_path
+            .file_name()
+            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
+        let encoded = export::encode(&self.scene, &triangles, format, &mtl_name);
+        if let Some(mtl) = &encoded.mtl {
+            if mtl_path == target || own(&mtl_path) {
+                return Err(CommandError::new(
+                    ErrorKind::InvalidInput,
+                    format!(
+                        "an OBJ of painted faces writes its materials to {}, which is not \
+                         free: export to another path",
+                        mtl_path.display()
+                    ),
+                ));
+            }
+            file::replace_file(&mtl_path, mtl)
+                .map_err(|e| file_error(&mtl_path.display().to_string(), e))?;
+        }
+        file::replace_file(target, &encoded.bytes).map_err(|e| file_error(path, e))
     }
 
     /// Swaps in another model: history, selection and the open context start over, and event
     /// subscribers are told to read the model again.
```

Apply to `crates/session/src/export.rs`:
```diff
--- a/crates/session/src/export.rs
+++ b/crates/session/src/export.rs
@@ -1,15 +1,16 @@
-//! Exporting the model's faces as triangles: OBJ, STL, and glTF (binary `.glb` or `.gltf`
-//! with its buffer embedded). Units are meters. OBJ and glTF are Y-up (x, z, -y); STL keeps
-//! the model's Z-up coordinates.
+//! Exporting the model's faces as triangles: OBJ (with a `.mtl` beside it when faces are
+//! painted), STL, and glTF (binary `.glb` or `.gltf` with its buffer embedded). Units are
+//! meters. OBJ and glTF are Y-up (x, z, -y); STL keeps the model's Z-up coordinates. Colors are
+//! those of the faces' front sides; STL has none.
 
-use std::collections::HashMap;
+use std::collections::{BTreeSet, HashMap};
 use std::fmt::Write as _;
 use std::path::Path;
 
 use base64::Engine;
 use glam::DVec3;
-use kernel::Triangle;
+use kernel::{MaterialId, Scene, Triangle};
 use schemars::JsonSchema;
 use serde::{Deserialize, Serialize};
 use serde_json::json;
 
@@ -40,55 +41,196 @@ impl ExportFormat {
         })
     }
 }
 
-/// The bytes of `triangles` in `format`.
-pub(crate) fn encode(triangles: &[Triangle], format: ExportFormat) -> Vec<u8> {
+/// The default material's front color (sRGB), as the window shows it.
+const DEFAULT_COLOR: [f64; 3] = [0.93, 0.93, 0.90];
+
+/// What an export writes: the file's bytes and, for an OBJ whose faces are painted, the bytes
+/// of the material library (`.mtl`) it names.
+pub(crate) struct Encoded {
+    pub bytes: Vec<u8>,
+    pub mtl: Option<Vec<u8>>,
+}
+
+/// The bytes of `triangles` in `format`. Materials are looked up in `scene`; an OBJ refers to
+/// its material library as `mtl_name`.
+pub(crate) fn encode(
+    scene: &Scene,
+    triangles: &[Triangle],
+    format: ExportFormat,
+    mtl_name: &str,
+) -> Encoded {
+    let groups = Groups::of(scene, triangles);
     match format {
-        ExportFormat::Obj => obj(triangles).into_bytes(),
-        ExportFormat::Stl => stl(triangles),
-        ExportFormat::Glb => glb(triangles),
-        ExportFormat::Gltf => gltf(triangles).into_bytes(),
+        ExportFormat::Obj => {
+            let (obj, mtl) = obj(&groups, mtl_name);
+            Encoded {
+                bytes: obj.into_bytes(),
+                mtl: mtl.map(String::into_bytes),
+            }
+        }
+        ExportFormat::Stl => Encoded {
+            bytes: stl(triangles),
+            mtl: None,
+        },
+        ExportFormat::Glb => Encoded {
+            bytes: glb(&groups),
+            mtl: None,
+        },
+        ExportFormat::Gltf => Encoded {
+            bytes: gltf(&groups).into_bytes(),
+            mtl: None,
+        },
+    }
+}
+
+/// The triangles grouped by the material of their front side, in the order each material is
+/// first used, with the name and color each group is exported with.
+struct Groups<'a> {
+    groups: Vec<Group<'a>>,
+}
+
+struct Group<'a> {
+    /// None for the default material.
+    material: Option<MaterialId>,
+    name: String,
+    /// sRGB, 0 to 1.
+    color: [f64; 3],
+    triangles: Vec<&'a Triangle>,
+}
+
+impl<'a> Groups<'a> {
+    fn of(scene: &Scene, triangles: &'a [Triangle]) -> Groups<'a> {
+        let mut groups: Vec<Group<'a>> = Vec::new();
+        for t in triangles {
+            let material = t.front.filter(|m| scene.material(*m).is_some());
+            let i = match groups.iter().position(|g| g.material == material) {
+                Some(i) => i,
+                None => {
+                    let (name, color) = match material.and_then(|m| scene.material(m)) {
+                        Some(m) => (m.name.clone(), m.color.map(|c| f64::from(c) / 255.0)),
+                        None => ("default".to_string(), DEFAULT_COLOR),
+                    };
+                    groups.push(Group {
+                        material,
+                        name,
+                        color,
+                        triangles: Vec::new(),
+                    });
+                    groups.len() - 1
+                }
+            };
+            groups[i].triangles.push(t);
+        }
+        Groups { groups }
+    }
+
+    /// True when some triangle has a material of its own.
+    fn painted(&self) -> bool {
+        self.groups.iter().any(|g| g.material.is_some())
+    }
+
+    fn triangles(&self) -> impl Iterator<Item = &&'a Triangle> {
+        self.groups.iter().flat_map(|g| &g.triangles)
+    }
+}
+
+/// An sRGB component (0 to 1) as linear light, as glTF colors and most `.mtl` readers take it.
+fn linear(c: f64) -> f64 {
+    if c <= 0.04045 {
+        c / 12.92
+    } else {
+        ((c + 0.055) / 1.055).powf(2.4)
     }
 }
 
+/// Material names for OBJ and MTL: no spaces or other characters those files split on, and no
+/// two alike.
+fn obj_names(groups: &Groups) -> Vec<String> {
+    let mut taken: BTreeSet<String> = BTreeSet::new();
+    groups
+        .groups
+        .iter()
+        .map(|g| {
+            let base: String = g
+                .name
+                .chars()
+                .map(|c| {
+                    if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
+                        c
+                    } else {
+                        '_'
+                    }
+                })
+                .collect();
+            let name = std::iter::once(base.clone())
+                .chain((2..).map(|n| format!("{base}_{n}")))
+                .find(|n| !taken.contains(n))
+                .expect("a free name");
+            taken.insert(name.clone());
+            name
+        })
+        .collect()
+}
+
 /// Z-up model coordinates to Y-up.
 fn y_up(p: DVec3) -> DVec3 {
     // Adding 0.0 turns a negative zero into a positive one.
     DVec3::new(p.x + 0.0, p.z + 0.0, -p.y + 0.0)
 }
 
-/// OBJ with shared positions and normals; faces are triangles.
-fn obj(triangles: &[Triangle]) -> String {
-    let mut out = String::from("# tenon export: meters, Y up\no model\n");
+/// OBJ with shared positions and normals; faces are triangles. When some faces are painted,
+/// each material's faces follow a `usemtl` line and the material library is returned too.
+fn obj(groups: &Groups, mtl_name: &str) -> (String, Option<String>) {
+    let mut out = String::from("# tenon export: meters, Y up\n");
+    let painted = groups.painted();
+    if painted {
+        let _ = writeln!(out, "mtllib {mtl_name}");
+    }
+    out.push_str("o model\n");
+    let names = obj_names(groups);
     let mut positions: HashMap<[u64; 3], usize> = HashMap::new();
     let mut normals: HashMap<[u64; 3], usize> = HashMap::new();
     let mut faces = String::new();
     let key = |p: DVec3| p.to_array().map(|c| (c + 0.0).to_bits());
-    for t in triangles {
-        let n = y_up(t.normal);
-        let next = normals.len() + 1;
-        let ni = *normals.entry(key(n)).or_insert_with(|| {
-            let _ = writeln!(out, "vn {} {} {}", n.x, n.y, n.z);
-            next
-        });
-        let mut ids = [0; 3];
-        for (k, c) in t.corners.iter().enumerate() {
-            let p = y_up(*c);
-            let next = positions.len() + 1;
-            ids[k] = *positions.entry(key(p)).or_insert_with(|| {
-                let _ = writeln!(out, "v {} {} {}", p.x, p.y, p.z);
+    for (g, name) in groups.groups.iter().zip(&names) {
+        if painted {
+            let _ = writeln!(faces, "usemtl {name}");
+        }
+        for t in &g.triangles {
+            let n = y_up(t.normal);
+            let next = normals.len() + 1;
+            let ni = *normals.entry(key(n)).or_insert_with(|| {
+                let _ = writeln!(out, "vn {} {} {}", n.x, n.y, n.z);
                 next
             });
+            let mut ids = [0; 3];
+            for (k, c) in t.corners.iter().enumerate() {
+                let p = y_up(*c);
+                let next = positions.len() + 1;
+                ids[k] = *positions.entry(key(p)).or_insert_with(|| {
+                    let _ = writeln!(out, "v {} {} {}", p.x, p.y, p.z);
+                    next
+                });
+            }
+            let _ = writeln!(
+                faces,
+                "f {}//{ni} {}//{ni} {}//{ni}",
+                ids[0], ids[1], ids[2]
+            );
         }
-        let _ = writeln!(
-            faces,
-            "f {}//{ni} {}//{ni} {}//{ni}",
-            ids[0], ids[1], ids[2]
-        );
     }
     out.push_str(&faces);
-    out
+    let mtl = painted.then(|| {
+        let mut mtl = String::from("# tenon materials: diffuse colors, linear\n");
+        for (g, name) in groups.groups.iter().zip(&names) {
+            let [r, gr, b] = g.color.map(linear);
+            let _ = writeln!(mtl, "newmtl {name}\nKd {r:.6} {gr:.6} {b:.6}");
+        }
+        mtl
+    });
+    (out, mtl)
 }
 
 /// Binary STL: an 80-byte header, the triangle count, then per triangle its normal, its three
 /// corners (f32, little-endian) and a zero attribute word.
@@ -109,61 +251,88 @@ fn stl(triangles: &[Triangle]) -> Vec<u8> {
     }
     out
 }
 
-/// The vertex buffer (positions then normals, f32 little-endian, Y-up) and the glTF document
-/// describing it, its buffer without a `uri` (a `.glb`'s own chunk).
-fn gltf_parts(triangles: &[Triangle]) -> (Vec<u8>, serde_json::Value) {
-    let count = triangles.len() * 3;
+/// The vertex buffer (positions then normals, f32 little-endian, Y-up, each material's
+/// triangles together) and the glTF document describing it: one primitive per material, its
+/// buffer without a `uri` (a `.glb`'s own chunk).
+fn gltf_parts(groups: &Groups) -> (Vec<u8>, serde_json::Value) {
+    let count: usize = groups.groups.iter().map(|g| g.triangles.len() * 3).sum();
     let mut buffer = Vec::with_capacity(count * 24);
-    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
-    for t in triangles {
-        for c in t.corners {
-            // The bounds must hold the stored f32 values, so round before comparing.
-            let p = y_up(c).as_vec3();
-            lo = lo.min(p.as_dvec3());
-            hi = hi.max(p.as_dvec3());
-            for x in p.to_array() {
-                buffer.extend_from_slice(&x.to_le_bytes());
+    let mut bounds = Vec::new();
+    for g in &groups.groups {
+        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
+        for t in &g.triangles {
+            for c in t.corners {
+                // The bounds must hold the stored f32 values, so round before comparing.
+                let p = y_up(c).as_vec3();
+                lo = lo.min(p.as_dvec3());
+                hi = hi.max(p.as_dvec3());
+                for x in p.to_array() {
+                    buffer.extend_from_slice(&x.to_le_bytes());
+                }
             }
         }
+        bounds.push((lo, hi));
     }
-    for t in triangles {
+    for t in groups.triangles() {
         let n = y_up(t.normal);
         for _ in 0..3 {
             for x in n.to_array() {
                 buffer.extend_from_slice(&(x as f32).to_le_bytes());
             }
         }
     }
     let half = count * 12;
+    let mut accessors = Vec::new();
+    let mut primitives = Vec::new();
+    let mut materials = Vec::new();
+    let mut start = 0;
+    for (k, (g, (lo, hi))) in groups.groups.iter().zip(&bounds).enumerate() {
+        let n = g.triangles.len() * 3;
+        accessors.push(
+            json!({"bufferView": 0, "byteOffset": start * 12, "componentType": 5126,
+            "count": n, "type": "VEC3",
+            "min": lo.to_array().map(|x| x as f32), "max": hi.to_array().map(|x| x as f32)}),
+        );
+        accessors.push(
+            json!({"bufferView": 1, "byteOffset": start * 12, "componentType": 5126,
+            "count": n, "type": "VEC3"}),
+        );
+        primitives.push(
+            json!({"attributes": {"POSITION": 2 * k, "NORMAL": 2 * k + 1},
+            "mode": 4, "material": k}),
+        );
+        let mut pbr = json!({"metallicFactor": 0});
+        if g.material.is_some() {
+            let [r, gr, b] = g.color.map(linear);
+            pbr["baseColorFactor"] = json!([r, gr, b, 1.0]);
+        }
+        materials.push(json!({"name": g.name, "doubleSided": true,
+            "pbrMetallicRoughness": pbr}));
+        start += n;
+    }
     let doc = json!({
         "asset": {"version": "2.0", "generator": "tenon"},
         "scene": 0,
         "scenes": [{"nodes": [0]}],
         "nodes": [{"mesh": 0, "name": "model"}],
-        "materials": [{"name": "default", "doubleSided": true,
-                       "pbrMetallicRoughness": {"metallicFactor": 0}}],
-        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "mode": 4,
-                                    "material": 0}]}],
+        "materials": materials,
+        "meshes": [{"primitives": primitives}],
         "buffers": [{"byteLength": buffer.len()}],
         "bufferViews": [
             {"buffer": 0, "byteOffset": 0, "byteLength": half, "target": 34962},
             {"buffer": 0, "byteOffset": half, "byteLength": half, "target": 34962}
         ],
-        "accessors": [
-            {"bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
-             "min": lo.to_array().map(|x| x as f32), "max": hi.to_array().map(|x| x as f32)},
-            {"bufferView": 1, "componentType": 5126, "count": count, "type": "VEC3"}
-        ]
+        "accessors": accessors
     });
     (buffer, doc)
 }
 
 /// Binary glTF: a 12-byte header, a JSON chunk padded with spaces and a BIN chunk padded with
 /// zeros, each to a multiple of 4 bytes.
-fn glb(triangles: &[Triangle]) -> Vec<u8> {
-    let (mut bin, doc) = gltf_parts(triangles);
+fn glb(groups: &Groups) -> Vec<u8> {
+    let (mut bin, doc) = gltf_parts(groups);
     let mut json = serde_json::to_vec(&doc).expect("a JSON value serializes");
     while json.len() % 4 != 0 {
         json.push(b' ');
     }
@@ -184,10 +353,10 @@ fn glb(triangles: &[Triangle]) -> Vec<u8> {
     out
 }
 
 /// glTF JSON with the buffer embedded as a base64 data URI.
-fn gltf(triangles: &[Triangle]) -> String {
-    let (bytes, mut doc) = gltf_parts(triangles);
+fn gltf(groups: &Groups) -> String {
+    let (bytes, mut doc) = gltf_parts(groups);
     doc["buffers"][0]["uri"] = json!(format!(
         "data:application/octet-stream;base64,{}",
         base64::engine::general_purpose::STANDARD.encode(&bytes)
     ));
```

Apply to `docs/agent-api.md`:
```diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -308,13 +308,17 @@ A `.skr` path (any case) and the open model's own file are refused with `invalid
 never overwrites a model; use `save`.
 
 | Format | Content |
 |---|---|
-| `obj` | Text OBJ: shared `v` positions and `vn` normals, one `f` triangle per line; Y up (x, z, -y) |
-| `stl` | Binary STL: one record per triangle with its normal; Z up, as in the model |
-| `glb` | Binary glTF 2.0: one mesh of flat-shaded triangles (`POSITION`, `NORMAL`) with a double-sided default material, so lone faces show from behind; Y up |
+| `obj` | Text OBJ: shared `v` positions and `vn` normals, one `f` triangle per line; Y up (x, z, -y). When faces are painted, it names a material library (`mtllib house.mtl`) written beside it under the same name with `.mtl` (replacing a file there), and each material's triangles follow a `usemtl` line; the library gives each material its diffuse color (`Kd`, linear). Material names have spaces and other separators replaced by `_` (and a number added when two become alike); faces without a material use `default`. Without painted faces no `.mtl` is written. |
+| `stl` | Binary STL: one record per triangle with its normal; Z up, as in the model; no colors |
+| `glb` | Binary glTF 2.0: one mesh of flat-shaded triangles (`POSITION`, `NORMAL`), one primitive per material; materials are double-sided, so lone faces show from behind; a painted one has its name and its color as `baseColorFactor` (linear), the default one is named `default`; Y up |
 | `gltf` | The same glTF document as JSON, its buffer embedded as a base64 data URI |
 
+Colors come from each face's front side (its own material, else that of the nearest painted
+instance placing it); back-side materials are not exported. An OBJ export whose `.mtl` would
+be the export path itself or the open model's file is refused with `invalid_input`.
+
 Units are meters. Export writes the file the same safe way as `save`, is not an undo step, and
 does not change the revision or the `file` query.
 
 Behaviors worth knowing:
```

- [ ] **Step 4: Run the tests to verify they pass**

`cargo test -p kernel --test triangles` 4 passed; `cargo test -p render` (18 unit, 15 headless, 2 snapshot); `cargo test -p session --test export` 14 passed; workspace 705; fmt and clippy clean.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` (expected totals: 693, 700, 705, 711 after Tasks 1-4).

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/kernel/tests/triangles.rs crates/render/tests/headless.rs crates/session/tests/export.rs crates/kernel/src/triangles.rs crates/render/src/renderer.rs crates/render/src/scene.rs crates/render/src/shaders/scene.wgsl crates/session/src/document/files.rs crates/session/src/export.rs docs/agent-api.md
git commit -m "Materials in rendering and export: per-side face colors with instance inheritance; OBJ with a .mtl beside it, glTF primitive and material per color"
```

### Task 4: Paint tool and Materials list

**Files:**
- Create: `crates/tools/src/paint.rs` (PaintTool, Target, target(); tests module with 2 tests)
- Modify: `crates/tools/src/tool.rs` (ToolContext.material, ToolContext.sample, ToolOutput.sampled, ToolKind::Paint in ALL (11), make/label "Paint"/shortcut 'b'), `crates/tools/src/lib.rs` (pub mod paint; pub use PaintTool)
- Modify (test hunks, every ToolContext literal adds `material: None, sample: false,`): `arc.rs`, `circle.rs`, `erase.rs`, `line.rs`, `move_tool.rs`, `push_pull.rs`, `rectangle.rs`, `rotate.rs`, `select.rs` (2), `tape.rs`
- Modify: `crates/app/src/editor.rs` (fields material/sample; `MATERIAL_PALETTE`; methods; ToolContext fields in handle(); `send` applies `out.sampled`; focus_lost clears sample; 2 tests + `face_paint` helper), `crates/app/src/app.rs` (MaterialRow list into View; UiAction arms; `set_sample_modifier(super || control)` in ModifiersChanged), `crates/app/src/ui.rs` (UiAction no longer Copy, 5 variants; MaterialRow; View.materials/material; `materials()` section, swatch, rgb; tests: View literals get `materials: &[], material: None`; toolbar test also expects `ToolKind::Paint`; `panel_actions` helper + 1 test), `crates/app/src/info.rs` (Front/Back lines for one face, Material line for a painted instance, `material_name`; describes_the_selection expectation; 1 test)
- Docs: `docs/agent-api.md` (window paragraph at the end of Materials), spec M7b line (final UI wording)

**Interfaces:**
- Produces:
  - `ToolContext { .., material: Option<MaterialId>, sample: bool }`; `ToolOutput { .., sampled: Option<Option<MaterialId>> }`; `ToolKind::Paint` (label "Paint", shortcut 'b'); `tools::PaintTool::new()`
  - `app::editor::MATERIAL_PALETTE: [[u8; 3]; 12]`; `Editor::set_sample_modifier(&mut self, held: bool)`, `current_material(&self) -> Option<MaterialId>`, `choose_material(&mut self, Option<MaterialId>) -> bool`, `add_material(&mut self) -> bool`, `rename_material(&mut self, MaterialId, String) -> bool`, `recolor_material(&mut self, MaterialId, [u8; 3]) -> bool`, `delete_material(&mut self, MaterialId) -> bool`
  - `UiAction::{ChooseMaterial(Option<MaterialId>), AddMaterial, DeleteMaterial(MaterialId), RenameMaterial(MaterialId, String), RecolorMaterial(MaterialId, [u8; 3])}` (derive `Clone, Debug, PartialEq, Eq`, no longer `Copy`); `pub struct MaterialRow { id, name, color }`; `View.materials: &'a [MaterialRow]`, `View.material: Option<MaterialId>`

Consumes: `Command::Paint/AddMaterial/ChangeMaterial/DeleteMaterial`, `ChangeSummary.materials`, `face_under_in`, `face_outline`, `box_outline`, `Camera::ray`.

Tests (all test-module hunks): `tools::paint::tests::{clicking_paints_the_side_facing_the_viewer_and_groups_whole, the_sampling_modifier_picks_up_the_material_clicked}`; `editor::tests::{b_paints_faces_with_the_chosen_material_and_cmd_click_picks_one_up, clicking_a_selected_face_paints_the_whole_selection}`; `ui::tests::the_materials_list_chooses_adds_renames_recolors_and_deletes` (+ Paint in `clicking_a_tool_button_reports_it`); `info::tests::names_the_materials_of_a_face_and_a_painted_group` (+ 2 lines in `describes_the_selection`). Note: egui selects the field's text on focus, so the rename test accepts any name containing the typed "Old".

Red: compile errors (no `ToolKind::Paint`, `PaintTool`, `Editor::add_material`, `UiAction::ChooseMaterial`, `View.materials`).
Green: `cargo test -p tools paint`, `cargo test -p app` (118 tests in the bin); workspace 711 passed; fmt and clippy clean.

`$S/verify.sh p1 p2 p3 p4`: see the report (all fmt-ok clippy-ok, 0 failed).

- [ ] **Step 1: Write the failing tests**

Tests: 

Test-module hunks are part of the Step 3 diffs; apply them first, along with the test files below.

- [ ] **Step 2: Run the tests to verify they fail**

compile errors (no `ToolKind::Paint`, `PaintTool`, `Editor::add_material`, `UiAction::ChooseMaterial`, `View.materials`).

- [ ] **Step 3: Implement**

Apply to `crates/app/src/app.rs`:
```diff
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -20,9 +20,9 @@ use crate::editor::{Editor, EditorKey};
 use crate::gui::Gui;
 use crate::host::WindowHost;
 use crate::info::entity_info;
 use crate::navigation::Button;
-use crate::ui::{UiAction, View};
+use crate::ui::{MaterialRow, UiAction, View};
 
 /// Sent to the event loop when an agent API request arrives.
 #[derive(Debug)]
 pub struct ApiWake;
@@ -306,8 +306,23 @@ impl State {
             }
             UiAction::SetProjection(projection) => {
                 self.editor.set_projection(projection);
             }
+            UiAction::ChooseMaterial(material) => {
+                self.editor.choose_material(material);
+            }
+            UiAction::AddMaterial => {
+                self.editor.add_material();
+            }
+            UiAction::DeleteMaterial(material) => {
+                self.editor.delete_material(material);
+            }
+            UiAction::RenameMaterial(material, name) => {
+                self.editor.rename_material(material, name);
+            }
+            UiAction::RecolorMaterial(material, color) => {
+                self.editor.recolor_material(material, color);
+            }
         }
     }
 
     fn resize(&mut self, width: u32, height: u32) {
@@ -345,8 +360,19 @@ impl State {
             self.title = title;
         }
         let status = self.editor.status_parts();
         let info = entity_info(self.editor.doc());
+        let materials: Vec<MaterialRow> = self
+            .editor
+            .doc()
+            .scene()
+            .materials()
+            .map(|(id, m)| MaterialRow {
+                id,
+                name: m.name.clone(),
+                color: m.color,
+            })
+            .collect();
         let panels = View {
             tool: self.editor.tool_kind(),
             status: &status,
             info: &info,
@@ -359,8 +385,10 @@ impl State {
                 .any(|id| matches!(id, EntityId::Instance(_))),
             inside_instance: !self.editor.doc().context().is_empty(),
             has_guides: self.editor.doc().model().guides().next().is_some(),
             projection: self.editor.doc().camera.projection,
+            materials: &materials,
+            material: self.editor.current_material(),
         };
         let mut gui_frame = self.gui.frame(&self.window, &panels);
         // Applied once the frame is presented: some open modal dialogs, which must not run
         // while this frame's surface texture is held.
@@ -692,8 +720,11 @@ impl ApplicationHandler<ApiWake> for App {
                 s.modifiers = m.state();
                 let locked = s
                     .editor
                     .set_modifiers(m.state().shift_key(), m.state().alt_key());
+                // Cmd (Ctrl elsewhere) held makes a Paint click pick up a material.
+                s.editor
+                    .set_sample_modifier(m.state().super_key() || m.state().control_key());
                 // Tapping Ctrl switches tools that can copy (Move, Rotate) between moving and
                 // copying, and the Tape Measure between adding guides and measuring only.
                 let toggled = ctrl_pressed && s.editor.toggle_copy();
                 locked || toggled
```

Apply to `crates/app/src/editor.rs`:
```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -3,9 +3,9 @@
 
 use std::path::Path;
 
 use glam::{DVec2, DVec3};
-use kernel::EntityId;
+use kernel::{EntityId, MaterialId};
 use render::grid::LineVertex;
 use session::{
     BatchError, Camera, Command, CommandError, Document, ErrorKind, Outcome, Projection,
     SelectMode, StandardView,
@@ -91,10 +91,30 @@ pub struct Editor {
     /// Feedback from the last tool action or failed command.
     message: Option<String>,
     /// Physical pixels per logical pixel.
     scale_factor: f64,
+    /// The material the Paint tool paints with (None: the default material).
+    material: Option<MaterialId>,
+    /// Whether the sampling modifier (Cmd on macOS, Ctrl elsewhere) is held.
+    sample: bool,
 }
 
+/// The colors the Materials panel offers; new materials get them in turn.
+pub const MATERIAL_PALETTE: [[u8; 3]; 12] = [
+    [196, 84, 64],
+    [222, 184, 135],
+    [112, 146, 80],
+    [70, 120, 180],
+    [150, 150, 150],
+    [240, 200, 80],
+    [140, 90, 60],
+    [120, 90, 160],
+    [250, 250, 250],
+    [40, 40, 40],
+    [90, 170, 170],
+    [230, 130, 160],
+];
+
 impl Editor {
     pub fn new(viewport: DVec2, scale_factor: f64) -> Self {
         Self {
             doc: Document::new(viewport),
@@ -111,8 +131,10 @@ impl Editor {
             preview: Vec::new(),
             typed: String::new(),
             message: None,
             scale_factor,
+            material: None,
+            sample: false,
         }
     }
 
     pub fn doc(&self) -> &Document {
@@ -201,11 +223,77 @@ impl Editor {
         }
         false
     }
 
+    /// Whether the sampling modifier (Cmd on macOS, Ctrl elsewhere) is held: a Paint click then
+    /// picks up a material instead of painting.
+    pub fn set_sample_modifier(&mut self, held: bool) {
+        self.sample = held;
+    }
+
+    /// The material the Paint tool paints with; None (the default material) also when the
+    /// one chosen no longer exists.
+    pub fn current_material(&self) -> Option<MaterialId> {
+        self.material
+            .filter(|m| self.doc.scene().material(*m).is_some())
+    }
+
+    /// Makes `material` the one to paint with and switches to the Paint tool.
+    pub fn choose_material(&mut self, material: Option<MaterialId>) -> bool {
+        self.material = material;
+        if self.kind != ToolKind::Paint {
+            self.select_tool(ToolKind::Paint);
+        }
+        true
+    }
+
+    /// Adds a material named "Material" (numbered when taken) in the next palette color and
+    /// chooses it. False if it could not be added (the status says why).
+    pub fn add_material(&mut self) -> bool {
+        let count = self.doc.scene().materials().count();
+        let color = MATERIAL_PALETTE[count % MATERIAL_PALETTE.len()];
+        self.message = None;
+        let Some(outcome) = self.run(Command::AddMaterial { name: None, color }) else {
+            return true;
+        };
+        let added = outcome.changes.materials.first().copied();
+        self.choose_material(added)
+    }
+
+    /// Renames a material. A refused name leaves it and says why in the status.
+    pub fn rename_material(&mut self, material: MaterialId, name: String) -> bool {
+        self.message = None;
+        self.run(Command::ChangeMaterial {
+            material,
+            name: Some(name),
+            color: None,
+        });
+        true
+    }
+
+    /// Recolors a material.
+    pub fn recolor_material(&mut self, material: MaterialId, color: [u8; 3]) -> bool {
+        self.message = None;
+        self.run(Command::ChangeMaterial {
+            material,
+            name: None,
+            color: Some(color),
+        });
+        true
+    }
+
+    /// Deletes a material; what was painted with it goes back to the default material.
+    pub fn delete_material(&mut self, material: MaterialId) -> bool {
+        self.message = None;
+        self.run(Command::DeleteMaterial { material });
+        self.update_hover();
+        true
+    }
+
     /// Returns true if a Shift lock ended, so the view needs a redraw.
     pub fn focus_lost(&mut self) -> bool {
         self.shift = false;
+        self.sample = false;
         let unlocked = std::mem::take(&mut self.shift_lock);
         if unlocked {
             self.lock = None;
             self.update_hover();
@@ -608,8 +696,15 @@ impl Editor {
         let keep = is_release && out.commands.is_empty() && out.message.is_none();
         if !keep {
             self.message = out.message.clone();
         }
+        if let Some(sampled) = out.sampled {
+            self.material = sampled;
+            let name = sampled
+                .and_then(|m| self.doc.scene().material(m))
+                .map_or("the default material", |m| m.name.as_str());
+            self.message = Some(format!("Painting with {name}"));
+        }
         self.apply(out);
         self.update_hover();
     }
 
@@ -638,8 +733,10 @@ impl Editor {
             pick_radius: PICK_RADIUS_LOGICAL_PX * self.scale_factor,
             drag_threshold: DRAG_LOGICAL_PX * self.scale_factor,
             shift: self.shift,
             selection: self.doc.selection(),
+            material: self.current_material(),
+            sample: self.sample,
         };
         self.tool.handle(event, &ctx)
     }
 
@@ -2320,5 +2417,89 @@ mod tests {
             circle.iter().all(|p| p.y.abs() < 1e-9),
             "in the wall's plane"
         );
     }
+
+    /// The front and back materials of the open context's only face.
+    fn face_paint(e: &Editor) -> [Option<MaterialId>; 2] {
+        e.doc().model().faces().next().unwrap().1.paint()
+    }
+
+    #[test]
+    fn b_paints_faces_with_the_chosen_material_and_cmd_click_picks_one_up() {
+        let mut e = editor();
+        square(&mut e);
+        assert!(e.key(EditorKey::Char('b')));
+        assert_eq!(e.tool_kind(), ToolKind::Paint);
+        assert!(e.add_material());
+        let first = e.current_material().expect("the new material is chosen");
+        assert_eq!(
+            e.doc().scene().material(first).unwrap().color,
+            MATERIAL_PALETTE[0]
+        );
+        let inside = DVec3::new(2.0, 2.0, 0.0);
+        click_at(&mut e, inside);
+        assert_eq!(face_paint(&e), [Some(first), None]);
+        e.undo();
+        assert_eq!(face_paint(&e), [None, None]);
+        e.redo();
+
+        // A second material, then pick the first back up with Cmd held: nothing is painted.
+        e.add_material();
+        let second = e.current_material().unwrap();
+        assert_ne!(first, second);
+        let revision = e.doc().revision();
+        e.set_sample_modifier(true);
+        click_at(&mut e, inside);
+        e.set_sample_modifier(false);
+        assert_eq!(e.doc().revision(), revision);
+        assert_eq!(e.current_material(), Some(first));
+        assert!(
+            e.status().contains("Painting with Material"),
+            "{}",
+            e.status()
+        );
+
+        // Choosing the default material in the list paints faces back to default.
+        e.choose_material(None);
+        click_at(&mut e, inside);
+        assert_eq!(face_paint(&e), [None, None]);
+
+        // Renaming, recoloring and deleting go through commands; deleting the chosen
+        // material leaves the default chosen.
+        e.choose_material(Some(second));
+        e.rename_material(second, "Brick".into());
+        e.recolor_material(second, [1, 2, 3]);
+        let m = e.doc().scene().material(second).unwrap();
+        assert_eq!((m.name.as_str(), m.color), ("Brick", [1, 2, 3]));
+        e.rename_material(second, "  ".into());
+        assert!(e.status().contains("needs a name"), "{}", e.status());
+        e.delete_material(second);
+        assert_eq!(e.current_material(), None);
+    }
+
+    #[test]
+    fn clicking_a_selected_face_paints_the_whole_selection() {
+        let mut e = editor();
+        square(&mut e);
+        for (x, y) in [(5.0, 1.0), (7.0, 1.0), (7.0, 3.0), (5.0, 3.0), (5.0, 1.0)] {
+            click_at(&mut e, DVec3::new(x, y, 0.0));
+        }
+        let faces: Vec<EntityId> = e
+            .doc()
+            .model()
+            .faces()
+            .map(|(f, _)| EntityId::Face(f))
+            .collect();
+        assert_eq!(faces.len(), 2);
+        e.execute(Command::Select {
+            ids: faces,
+            mode: SelectMode::Replace,
+        })
+        .unwrap();
+        e.select_tool(ToolKind::Paint);
+        e.add_material();
+        let m = e.current_material();
+        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
+        assert!(e.doc().model().faces().all(|(_, f)| f.front == m));
+    }
 }
```

Apply to `crates/app/src/info.rs`:
```diff
--- a/crates/app/src/info.rs
+++ b/crates/app/src/info.rs
@@ -6,10 +6,11 @@ use session::Document;
 use tools::vcb::format_length;
 
 /// Lines describing the selection: counts, plus length, area or normal for a single entity,
 /// the radius, segments, angle and length of a whole curve (a circle or arc), the direction
-/// or position of a single guide, or the kind, name and rotation of a single instance. While a
-/// group or component is open, a first line says which.
+/// or position of a single guide, or the kind, name and rotation of a single instance; a single
+/// face's front and back materials and a painted instance's material. While a group or
+/// component is open, a first line says which.
 pub fn entity_info(doc: &Document) -> Vec<String> {
     let mut lines = editing(doc).into_iter().collect::<Vec<_>>();
     lines.extend(selection_info(doc));
     lines
@@ -78,8 +79,11 @@ fn selection_info(doc: &Document) -> Vec<String> {
             }
             _ => vec!["1 group".into()],
         };
         lines.extend(instance.and_then(|i| rotation(i.transform)));
+        if let Some(material) = instance.and_then(|i| i.material) {
+            lines.push(format!("Material: {}", material_name(doc, Some(material))));
+        }
         return lines;
     }
     match (&edges[..], &faces[..]) {
         ([], []) if instances.is_empty() && guides == 0 => vec!["Nothing selected".into()],
@@ -93,8 +97,10 @@ fn selection_info(doc: &Document) -> Vec<String> {
             vec![
                 "1 face".into(),
                 format!("Area: {} m²", number(m.area(*f).unwrap_or(0.0))),
                 format!("Normal: {}, {}, {}", n[0], n[1], n[2]),
+                format!("Front: {}", material_name(doc, face.front)),
+                format!("Back: {}", material_name(doc, face.back)),
             ]
         }
         _ => {
             let mut parts = Vec::new();
@@ -114,8 +120,15 @@ fn selection_info(doc: &Document) -> Vec<String> {
         }
     }
 }
 
+/// A material's name, or "Default" for none.
+fn material_name(doc: &Document, material: Option<kernel::MaterialId>) -> String {
+    material
+        .and_then(|m| doc.scene().material(m))
+        .map_or_else(|| "Default".to_string(), |m| m.name.clone())
+}
+
 /// "1 circle" (or arc, or curve) with its radius, angle, segments and length when `edges` are
 /// exactly the edges of one curve.
 fn curve_info(m: &Model, edges: &[EdgeId]) -> Option<Vec<String>> {
     let curve = m.curve_of(*edges.first()?)?;
@@ -214,15 +227,57 @@ mod tests {
     fn describes_the_selection() {
         let mut d = doc_with_square();
         assert_eq!(entity_info(&d), ["Nothing selected"]);
         select(&mut d, &["f8"]);
-        assert_eq!(entity_info(&d), ["1 face", "Area: 6 m²", "Normal: 0, 0, 1"]);
+        assert_eq!(
+            entity_info(&d),
+            [
+                "1 face",
+                "Area: 6 m²",
+                "Normal: 0, 0, 1",
+                "Front: Default",
+                "Back: Default"
+            ]
+        );
         select(&mut d, &["e2"]);
         assert_eq!(entity_info(&d), ["1 edge", "Length: 2 m"]);
         select(&mut d, &["e2", "e4", "f8"]);
         assert_eq!(entity_info(&d), ["2 edges, 1 face"]);
     }
 
+    #[test]
+    fn names_the_materials_of_a_face_and_a_painted_group() {
+        let mut d = doc_with_square();
+        let added = d
+            .execute(Command::AddMaterial {
+                name: Some("Brick".into()),
+                color: [180, 60, 40],
+            })
+            .unwrap();
+        let brick = added.changes.materials[0];
+        d.execute(Command::Paint {
+            ids: vec!["f8".parse().unwrap()],
+            material: Some(brick),
+            side: kernel::Side::Back,
+        })
+        .unwrap();
+        select(&mut d, &["f8"]);
+        let lines = entity_info(&d);
+        assert_eq!(lines[3..], ["Front: Default", "Back: Brick"]);
+        d.execute(Command::MakeGroup {
+            ids: vec!["f8".parse().unwrap()],
+        })
+        .unwrap();
+        let group = *d.selection().iter().next().unwrap();
+        d.execute(Command::Paint {
+            ids: vec![group],
+            material: Some(brick),
+            side: kernel::Side::Front,
+        })
+        .unwrap();
+        assert_eq!(entity_info(&d), ["1 group", "Material: Brick"]);
+    }
+
     #[test]
     fn describes_a_whole_curve() {
         let mut d = Document::new(DVec2::new(800.0, 600.0));
         d.execute(Command::AddCircle {
```

Apply to `crates/app/src/ui.rs`:
```diff
--- a/crates/app/src/ui.rs
+++ b/crates/app/src/ui.rs
@@ -1,14 +1,15 @@
-//! The window's panels: toolbar, status bar with the measurements box, and Entity Info.
+//! The window's panels: toolbar, status bar with the measurements box, and on the right Entity
+//! Info and the Materials list.
 
-use egui::{Align, Layout, Panel, RichText, Ui};
-use session::{Projection, StandardView};
+use egui::{Align, Color32, Layout, Panel, RichText, Sense, Ui, Vec2};
+use session::{MaterialId, Projection, StandardView};
 use tools::ToolKind;
 
-use crate::editor::Status;
+use crate::editor::{MATERIAL_PALETTE, Status};
 
 /// What the user asked for through the panels this frame.
-#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+#[derive(Clone, Debug, PartialEq, Eq)]
 pub enum UiAction {
     Tool(ToolKind),
     Undo,
     Redo,
@@ -25,10 +26,27 @@ pub enum UiAction {
     DeleteGuides,
     ZoomExtents,
     StandardView(StandardView),
     SetProjection(Projection),
+    /// Paint with this material (None: the default material), switching to the Paint tool.
+    ChooseMaterial(Option<MaterialId>),
+    AddMaterial,
+    DeleteMaterial(MaterialId),
+    RenameMaterial(MaterialId, String),
+    RecolorMaterial(MaterialId, [u8; 3]),
 }
 
+/// A material as the Materials list shows it.
+#[derive(Clone, Debug, PartialEq)]
+pub struct MaterialRow {
+    pub id: MaterialId,
+    pub name: String,
+    pub color: [u8; 3],
+}
+
+/// The default material's swatch: its front color.
+const DEFAULT_SWATCH: Color32 = Color32::from_rgb(237, 237, 230);
+
 /// What the panels show.
 pub struct View<'a> {
     pub tool: ToolKind,
     pub status: &'a Status,
@@ -41,8 +59,12 @@ pub struct View<'a> {
     /// Whether the open context has guides to delete.
     pub has_guides: bool,
     /// The camera's projection, checked in the Camera menu.
     pub projection: Projection,
+    /// Every material, in id order.
+    pub materials: &'a [MaterialRow],
+    /// The material the Paint tool paints with (None: the default material).
+    pub material: Option<MaterialId>,
 }
 
 /// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
 /// the 3D view, in points.
@@ -179,12 +201,91 @@ pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
             ui.heading("Entity Info");
             for line in view.info {
                 ui.label(line);
             }
+            ui.separator();
+            materials(ui, view, &mut actions);
         });
     (actions, ui.available_rect_before_wrap())
 }
 
+/// A small square of `color` that can be clicked.
+fn swatch(ui: &mut Ui, color: Color32) -> egui::Response {
+    let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
+    ui.painter().rect_filled(rect, 2.0, color);
+    response
+}
+
+fn rgb(c: [u8; 3]) -> Color32 {
+    Color32::from_rgb(c[0], c[1], c[2])
+}
+
+/// The Materials list: the default material and every material, the one painted with
+/// highlighted (clicking one chooses it and the Paint tool); Add and Delete; and for the
+/// chosen material its name (renamed on Enter or leaving the field) and colors to pick from.
+fn materials(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
+    ui.heading("Materials");
+    egui::ScrollArea::vertical()
+        .max_height(220.0)
+        .show(ui, |ui| {
+            let rows = std::iter::once((None, "Default", DEFAULT_SWATCH)).chain(
+                view.materials
+                    .iter()
+                    .map(|m| (Some(m.id), m.name.as_str(), rgb(m.color))),
+            );
+            for (id, name, color) in rows {
+                ui.horizontal(|ui| {
+                    let picked = swatch(ui, color).clicked();
+                    let label = ui.selectable_label(view.material == id, name);
+                    if picked || label.clicked() {
+                        actions.push(UiAction::ChooseMaterial(id));
+                    }
+                });
+            }
+        });
+    ui.horizontal(|ui| {
+        if ui.button("Add").clicked() {
+            actions.push(UiAction::AddMaterial);
+        }
+        let current = view.material;
+        if ui
+            .add_enabled(current.is_some(), egui::Button::new("Delete"))
+            .clicked()
+            && let Some(id) = current
+        {
+            actions.push(UiAction::DeleteMaterial(id));
+        }
+    });
+    let Some(m) = view
+        .material
+        .and_then(|id| view.materials.iter().find(|m| m.id == id))
+    else {
+        return;
+    };
+    // The name being typed lives in egui's memory until the field loses focus.
+    let key = egui::Id::new(("material-name", m.id.0));
+    let mut text = ui
+        .data(|d| d.get_temp::<String>(key))
+        .unwrap_or_else(|| m.name.clone());
+    let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
+    if edit.lost_focus() {
+        if text != m.name {
+            actions.push(UiAction::RenameMaterial(m.id, text));
+        }
+        ui.data_mut(|d| d.remove::<String>(key));
+    } else if edit.has_focus() {
+        ui.data_mut(|d| d.insert_temp(key, text));
+    }
+    ui.horizontal_wrapped(|ui| {
+        for color in MATERIAL_PALETTE {
+            let response = swatch(ui, rgb(color));
+            if response.clicked() && color != m.color {
+                actions.push(UiAction::RecolorMaterial(m.id, color));
+            }
+        }
+    });
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
     use egui::{Pos2, Rect, Vec2};
@@ -245,8 +346,10 @@ mod tests {
             has_instance_selected: false,
             inside_instance: false,
             has_guides: false,
             projection: Projection::Perspective,
+            materials: &[],
+            material: None,
         };
         let (_, central) = run(&ctx, screen(), &view);
         let file = Pos2::new(14.0, central.min.y / 2.0);
         let mut clicked = Vec::new();
@@ -323,8 +426,10 @@ mod tests {
             has_instance_selected: false,
             inside_instance: false,
             has_guides: false,
             projection: Projection::Perspective,
+            materials: &[],
+            material: None,
         };
         // The Camera menu sits right after File and Edit.
         let clicked = menu_actions(&view, 80..140);
         assert!(clicked.contains(&UiAction::ZoomExtents), "{clicked:?}");
@@ -356,8 +461,10 @@ mod tests {
             has_instance_selected: false,
             inside_instance: false,
             has_guides,
             projection: Projection::Perspective,
+            materials: &[],
+            material: None,
         };
         // The Edit button sits right after File (which ends before x = 40).
         menu_actions(&view, 40..80)
     }
@@ -389,8 +496,10 @@ mod tests {
             has_instance_selected: false,
             inside_instance: false,
             has_guides: false,
             projection: Projection::Perspective,
+            materials: &[],
+            material: None,
         };
         let (actions, rect) = run(&ctx, screen(), &view);
         assert!(actions.is_empty());
         assert!(
@@ -415,8 +524,10 @@ mod tests {
             has_instance_selected: true,
             inside_instance: true,
             has_guides: true,
             projection: Projection::Perspective,
+            materials: &[],
+            material: None,
         };
         let (_, central) = run(&ctx, screen(), &view);
         let row = central.min.y / 2.0;
         // Click along the toolbar row until a press lands on the Rectangle button.
@@ -435,9 +546,10 @@ mod tests {
             let mut release = screen();
             release.events = vec![pointer(false)];
             clicked.extend(run(&ctx, release, &view).0);
         }
-        for kind in [ToolKind::Rectangle, ToolKind::Arc] {
+        // Paint is the last tool: the toolbar still fits a 1280-point window.
+        for kind in [ToolKind::Rectangle, ToolKind::Arc, ToolKind::Paint] {
             assert!(clicked.contains(&UiAction::Tool(kind)), "{clicked:?}");
         }
         assert!(clicked.contains(&UiAction::Undo));
         for action in [
@@ -448,5 +560,97 @@ mod tests {
         ] {
             assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
         }
     }
+
+    /// Clicks down the right panel at `x`, every 4 points; after each click, types `typed`
+    /// and presses Enter. Returns every action.
+    fn panel_actions(view: &View, x: f32, typed: Option<&str>) -> Vec<UiAction> {
+        let ctx = egui::Context::default();
+        let mut clicked = Vec::new();
+        for y in (0..800).step_by(4) {
+            clicked.extend(click(&ctx, view, Pos2::new(x, y as f32)));
+            if let Some(text) = typed {
+                let mut typing = screen();
+                typing.events = vec![
+                    egui::Event::Text(text.into()),
+                    egui::Event::Key {
+                        key: egui::Key::Enter,
+                        physical_key: None,
+                        pressed: true,
+                        repeat: false,
+                        modifiers: Default::default(),
+                    },
+                ];
+                clicked.extend(run(&ctx, typing, view).0);
+                clicked.extend(run(&ctx, screen(), view).0);
+            }
+        }
+        clicked
+    }
+
+    #[test]
+    fn the_materials_list_chooses_adds_renames_recolors_and_deletes() {
+        let status = status();
+        let info = vec!["Nothing selected".to_string()];
+        let rows = [
+            MaterialRow {
+                id: MaterialId(31),
+                name: "Brick".into(),
+                color: [180, 60, 40],
+            },
+            MaterialRow {
+                id: MaterialId(32),
+                name: "Glass".into(),
+                color: [90, 170, 170],
+            },
+        ];
+        let mut view = View {
+            tool: ToolKind::Line,
+            status: &status,
+            info: &info,
+            has_selection: false,
+            has_instance_selected: false,
+            inside_instance: false,
+            has_guides: false,
+            projection: Projection::Perspective,
+            materials: &rows,
+            material: Some(MaterialId(31)),
+        };
+        // Down the panel's left part: swatches and names, Add, the name field, the palette.
+        let clicked = panel_actions(&view, 1280.0 - 160.0, None);
+        for action in [
+            UiAction::ChooseMaterial(None),
+            UiAction::ChooseMaterial(Some(MaterialId(31))),
+            UiAction::ChooseMaterial(Some(MaterialId(32))),
+            UiAction::AddMaterial,
+            UiAction::RecolorMaterial(MaterialId(31), MATERIAL_PALETTE[0]),
+        ] {
+            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
+        }
+        // Further right: Delete, and typing into the name field renames on Enter.
+        let clicked = panel_actions(&view, 1280.0 - 120.0, Some("Old"));
+        assert!(
+            clicked.contains(&UiAction::DeleteMaterial(MaterialId(31))),
+            "{clicked:?}"
+        );
+        assert!(
+            clicked.iter().any(|a| matches!(
+                a,
+                UiAction::RenameMaterial(MaterialId(31), name) if name.contains("Old")
+            )),
+            "{clicked:?}"
+        );
+        // With the default material chosen there is nothing to delete, rename or recolor.
+        view.material = None;
+        let clicked = panel_actions(&view, 1280.0 - 120.0, Some("Old"));
+        assert!(
+            !clicked.iter().any(|a| matches!(
+                a,
+                UiAction::DeleteMaterial(_)
+                    | UiAction::RenameMaterial(..)
+                    | UiAction::RecolorMaterial(..)
+            )),
+            "{clicked:?}"
+        );
+    }
 }
```

Apply to `crates/tools/src/arc.rs`:
```diff
--- a/crates/tools/src/arc.rs
+++ b/crates/tools/src/arc.rs
@@ -260,8 +260,10 @@ mod tests {
             pick_radius: 10.0,
             drag_threshold: 4.0,
             shift: false,
             selection: &Default::default(),
+            material: None,
+            sample: false,
         };
         tool.handle(event, &ctx)
     }
 
```

Apply to `crates/tools/src/circle.rs`:
```diff
--- a/crates/tools/src/circle.rs
+++ b/crates/tools/src/circle.rs
@@ -168,8 +168,10 @@ mod tests {
             pick_radius: 10.0,
             drag_threshold: 4.0,
             shift: false,
             selection: &Default::default(),
+            material: None,
+            sample: false,
         };
         tool.handle(event, &ctx)
     }
 
```

Apply to `crates/tools/src/erase.rs`:
```diff
--- a/crates/tools/src/erase.rs
+++ b/crates/tools/src/erase.rs
@@ -115,8 +115,10 @@ mod tests {
             pick_radius: 10.0,
             drag_threshold: 4.0,
             shift: false,
             selection: &Default::default(),
+            material: None,
+            sample: false,
         };
         tool.handle(event, &ctx)
     }
 
```

Apply to `crates/tools/src/lib.rs`:
```diff
--- a/crates/tools/src/lib.rs
+++ b/crates/tools/src/lib.rs
@@ -7,8 +7,9 @@ pub mod erase;
 pub mod inference;
 pub mod line;
 pub mod move_tool;
 pub mod overlay;
+pub mod paint;
 pub mod push_pull;
 pub mod rectangle;
 pub mod rotate;
 pub mod scenery;
@@ -25,8 +26,9 @@ pub use inference::{
     guide_under_where, infer, infer_in, infer_with,
 };
 pub use line::LineTool;
 pub use move_tool::MoveTool;
+pub use paint::PaintTool;
 pub use push_pull::PushPullTool;
 pub use rectangle::RectangleTool;
 pub use rotate::RotateTool;
 pub use scenery::{Owner, Part, Scenery};
```

Apply to `crates/tools/src/line.rs`:
```diff
--- a/crates/tools/src/line.rs
+++ b/crates/tools/src/line.rs
@@ -166,8 +166,10 @@ mod tests {
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
                 shift: false,
                 selection: &Default::default(),
+                material: None,
+                sample: false,
             };
             self.tool.handle(event, &ctx)
         }
 
```

Apply to `crates/tools/src/move_tool.rs`:
```diff
--- a/crates/tools/src/move_tool.rs
+++ b/crates/tools/src/move_tool.rs
@@ -255,8 +255,10 @@ mod tests {
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
                 shift: false,
                 selection: &self.selection,
+                material: None,
+                sample: false,
             };
             self.tool.handle(event, &ctx)
         }
 
```

Create `crates/tools/src/paint.rs`:
```rust
//! The Paint tool (Paint Bucket): click a face to paint the side facing you with the current
//! material, or a group to paint the group. Clicking something selected paints every selected
//! face and group. With the sampling modifier (Cmd on macOS, Ctrl elsewhere) held, a click
//! picks up the material clicked instead. What is under the cursor is outlined.

use glam::DVec3;
use kernel::{EntityId, FaceId, Side};
use session::Command;

use crate::inference::face_under_in;
use crate::scenery::Owner;
use crate::select::{box_outline, face_outline};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};

#[derive(Clone, Debug, Default)]
pub struct PaintTool {
    /// Outline of the face or group under the cursor.
    outline: Vec<[DVec3; 2]>,
}

impl PaintTool {
    pub fn new() -> Self {
        Self::default()
    }
}

/// What a click reaches: the open context's face under the cursor (with the side facing the
/// viewer), or the instance whose geometry is under it. Geometry outside the open instance
/// hides what is behind it but is not painted.
enum Target {
    Face(FaceId, Side),
    Instance(EntityId),
}

fn target(ctx: &ToolContext) -> Option<Target> {
    match face_under_in(ctx.scenery, ctx.camera, ctx.cursor)? {
        (Owner::Open, f, _) => {
            let normal = ctx.model.face(f)?.normal;
            let side = if ctx.camera.ray(ctx.cursor).dir.dot(normal) > 0.0 {
                Side::Back
            } else {
                Side::Front
            };
            Some(Target::Face(f, side))
        }
        (Owner::Instance(i), _, _) => Some(Target::Instance(EntityId::Instance(i))),
        (Owner::Outside, _, _) => None,
    }
}

impl Tool for PaintTool {
    fn name(&self) -> &'static str {
        "Paint"
    }

    fn prompt(&self) -> &'static str {
        "Click a face or group to paint it; Cmd/Ctrl-click picks up a material"
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(_) => {
                self.outline = match target(ctx) {
                    Some(Target::Face(f, _)) => ctx
                        .model
                        .face(f)
                        .map(|face| face_outline(ctx.model, face))
                        .unwrap_or_default(),
                    Some(Target::Instance(id)) => box_outline(ctx, id),
                    None => Vec::new(),
                };
            }
            ToolEvent::Click(_) => {
                let Some(target) = target(ctx) else {
                    return out;
                };
                let (id, side) = match target {
                    Target::Face(f, side) => (EntityId::Face(f), side),
                    Target::Instance(id) => (id, Side::Front),
                };
                if ctx.sample {
                    out.sampled = Some(match target {
                        Target::Face(f, side) => ctx.model.face(f).and_then(|face| match side {
                            Side::Back => face.back,
                            _ => face.front,
                        }),
                        Target::Instance(EntityId::Instance(i)) => {
                            ctx.model.instance(i).and_then(|i| i.material)
                        }
                        Target::Instance(_) => None,
                    });
                } else {
                    // A click on something selected paints the whole selection's faces (on the
                    // side clicked) and groups.
                    let ids = if ctx.selection.contains(&id) {
                        ctx.selection
                            .iter()
                            .copied()
                            .filter(|e| matches!(e, EntityId::Face(_) | EntityId::Instance(_)))
                            .collect()
                    } else {
                        vec![id]
                    };
                    out.commands.push(Command::Paint {
                        ids,
                        material: ctx.material,
                        side,
                    });
                }
            }
            ToolEvent::Value(_) => {
                out.message = Some("The Paint tool takes no typed values".into());
            }
            ToolEvent::Release(_)
            | ToolEvent::DoubleClick(_)
            | ToolEvent::Cancel
            | ToolEvent::ToggleCopy => {}
        }
        out.preview = self.outline.clone();
        out
    }

    fn on_external_change(&mut self) {
        self.outline.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::{DefinitionKind, MaterialId, MeshId, Model, Scene};
    use session::Camera;
    use std::collections::BTreeSet;

    fn hit() -> Inference {
        Inference {
            point: DVec3::ZERO,
            kind: SnapKind::Free,
            face: None,
        }
    }

    /// A 2 x 2 square on the ground at x 0..2, and a grouped one at x 3..5.
    fn scene() -> Scene {
        let mut m = Model::new();
        let square = |x: f64| {
            [
                DVec3::new(x, 0.0, 0.0),
                DVec3::new(x + 2.0, 0.0, 0.0),
                DVec3::new(x + 2.0, 2.0, 0.0),
                DVec3::new(x, 2.0, 0.0),
            ]
        };
        m.add_polygon(&square(0.0)).unwrap();
        m.add_polygon(&square(3.0)).unwrap();
        let mut s = Scene::from_root(m);
        let far = |s: &Scene| {
            let m = s.root();
            let edges: Vec<_> = m
                .edges()
                .filter(|(_, e)| e.v.iter().all(|v| m.position(*v).unwrap().x >= 3.0))
                .map(|(e, _)| e)
                .collect();
            let faces: Vec<_> = m
                .faces()
                .filter(|(_, f)| m.position(f.outer[0]).unwrap().x >= 3.0)
                .map(|(f, _)| f)
                .collect();
            (edges, faces)
        };
        let (edges, faces) = far(&s);
        s.make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        s
    }

    fn send(
        tool: &mut PaintTool,
        scene: &Scene,
        camera: &Camera,
        at: DVec3,
        sample: bool,
        selection: &BTreeSet<EntityId>,
        event: ToolEvent,
    ) -> ToolOutput {
        let scenery = Scenery::of(scene, &[]).unwrap();
        let ctx = ToolContext {
            model: scene.root(),
            scenery: &scenery,
            camera,
            cursor: camera.project(at).unwrap(),
            pick_radius: 10.0,
            drag_threshold: 4.0,
            shift: false,
            selection,
            material: Some(MaterialId(77)),
            sample,
        };
        tool.handle(event, &ctx)
    }

    fn face_at(scene: &Scene, x: f64) -> EntityId {
        let m = scene.root();
        let f = m
            .faces()
            .find(|(_, f)| (m.position(f.outer[0]).unwrap().x - x).abs() < 2.1)
            .unwrap()
            .0;
        EntityId::Face(f)
    }

    #[test]
    fn clicking_paints_the_side_facing_the_viewer_and_groups_whole() {
        let s = scene();
        let none = BTreeSet::new();
        let mut t = PaintTool::new();
        let above = Camera::new(DVec2::new(800.0, 600.0));
        let on_square = DVec3::new(1.0, 1.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            on_square,
            false,
            &none,
            ToolEvent::Move(hit()),
        );
        assert_eq!(out.preview.len(), 4, "the face is outlined");
        let out = send(
            &mut t,
            &s,
            &above,
            on_square,
            false,
            &none,
            ToolEvent::Click(hit()),
        );
        let face = face_at(&s, 0.0);
        assert_eq!(
            out.commands,
            vec![Command::Paint {
                ids: vec![face],
                material: Some(MaterialId(77)),
                side: Side::Front,
            }]
        );
        // From below, the back is painted.
        let mut below = Camera::new(DVec2::new(800.0, 600.0));
        below.eye = DVec3::new(-8.0, -12.0, -7.0);
        let out = send(
            &mut t,
            &s,
            &below,
            on_square,
            false,
            &none,
            ToolEvent::Click(hit()),
        );
        assert!(matches!(
            &out.commands[..],
            [Command::Paint {
                side: Side::Back,
                ..
            }]
        ));
        // The grouped square paints its group.
        let group = EntityId::Instance(s.root().instances().next().unwrap().0);
        let on_group = DVec3::new(4.0, 1.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            on_group,
            false,
            &none,
            ToolEvent::Click(hit()),
        );
        assert!(matches!(&out.commands[..], [Command::Paint { ids, .. }] if *ids == vec![group]));
        // Clicking a selected face paints the whole selection's faces and groups.
        let selection = BTreeSet::from([face, group]);
        let out = send(
            &mut t,
            &s,
            &above,
            on_square,
            false,
            &selection,
            ToolEvent::Click(hit()),
        );
        assert!(
            matches!(&out.commands[..], [Command::Paint { ids, .. }] if *ids == vec![face, group])
        );
        // Nothing under the cursor: nothing happens.
        let off = DVec3::new(-3.0, 6.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            off,
            false,
            &none,
            ToolEvent::Click(hit()),
        );
        assert!(out.commands.is_empty() && out.sampled.is_none());
    }

    #[test]
    fn the_sampling_modifier_picks_up_the_material_clicked() {
        let mut s = scene();
        let (red, _) = s.add_material("Red", [255, 0, 0]).unwrap();
        let (blue, _) = s.add_material("Blue", [0, 0, 255]).unwrap();
        let EntityId::Face(face) = face_at(&s, 0.0) else {
            unreachable!()
        };
        s.paint(MeshId::Root, &[face], &[], Side::Back, Some(red))
            .unwrap();
        let group = s.root().instances().next().unwrap().0;
        s.paint(MeshId::Root, &[], &[group], Side::Front, Some(blue))
            .unwrap();
        let none = BTreeSet::new();
        let mut t = PaintTool::new();
        let above = Camera::new(DVec2::new(800.0, 600.0));
        let on_square = DVec3::new(1.0, 1.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            on_square,
            true,
            &none,
            ToolEvent::Click(hit()),
        );
        assert_eq!(
            out.sampled,
            Some(None),
            "the front has the default material"
        );
        assert!(out.commands.is_empty());
        let mut below = Camera::new(DVec2::new(800.0, 600.0));
        below.eye = DVec3::new(-8.0, -12.0, -7.0);
        let out = send(
            &mut t,
            &s,
            &below,
            on_square,
            true,
            &none,
            ToolEvent::Click(hit()),
        );
        assert_eq!(out.sampled, Some(Some(red)));
        let on_group = DVec3::new(4.0, 1.0, 0.0);
        let out = send(
            &mut t,
            &s,
            &above,
            on_group,
            true,
            &none,
            ToolEvent::Click(hit()),
        );
        assert_eq!(out.sampled, Some(Some(blue)));
    }
}
```

Apply to `crates/tools/src/push_pull.rs`:
```diff
--- a/crates/tools/src/push_pull.rs
+++ b/crates/tools/src/push_pull.rs
@@ -227,8 +227,10 @@ mod tests {
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
                 shift: false,
                 selection: &Default::default(),
+                material: None,
+                sample: false,
             };
             self.tool.handle(event, &ctx)
         }
     }
```

Apply to `crates/tools/src/rectangle.rs`:
```diff
--- a/crates/tools/src/rectangle.rs
+++ b/crates/tools/src/rectangle.rs
@@ -190,8 +190,10 @@ mod tests {
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
                 shift: false,
                 selection: &Default::default(),
+                material: None,
+                sample: false,
             };
             self.tool.handle(event, &ctx)
         }
     }
```

Apply to `crates/tools/src/rotate.rs`:
```diff
--- a/crates/tools/src/rotate.rs
+++ b/crates/tools/src/rotate.rs
@@ -372,8 +372,10 @@ mod tests {
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
                 shift: false,
                 selection: &self.selection,
+                material: None,
+                sample: false,
             };
             self.tool.handle(event, &ctx)
         }
 
```

Apply to `crates/tools/src/select.rs`:
```diff
--- a/crates/tools/src/select.rs
+++ b/crates/tools/src/select.rs
@@ -395,8 +395,10 @@ mod tests {
                 pick_radius: 10.0,
                 drag_threshold: self.drag_threshold,
                 shift: self.shift,
                 selection: &self.selection,
+                material: None,
+                sample: false,
             };
             self.tool.handle(event, &ctx)
         }
 
@@ -654,8 +656,10 @@ mod tests {
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
                 shift: false,
                 selection: &Default::default(),
+                material: None,
+                sample: false,
             };
             pick(&ctx)
         };
         let picked = click(DVec3::new(3.0, 1.0, 0.0));
```

Apply to `crates/tools/src/tape.rs`:
```diff
--- a/crates/tools/src/tape.rs
+++ b/crates/tools/src/tape.rs
@@ -286,8 +286,10 @@ mod tests {
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
                 shift: false,
                 selection: &Default::default(),
+                material: None,
+                sample: false,
             };
             self.tool.handle(event, &ctx)
         }
 
```

Apply to `crates/tools/src/tool.rs`:
```diff
--- a/crates/tools/src/tool.rs
+++ b/crates/tools/src/tool.rs
@@ -2,17 +2,18 @@
 
 use std::collections::BTreeSet;
 
 use glam::{DVec2, DVec3};
-use kernel::{EntityId, Model};
+use kernel::{EntityId, MaterialId, Model};
 use session::{Camera, Command};
 
 use crate::arc::ArcTool;
 use crate::circle::CircleTool;
 use crate::erase::EraseTool;
 use crate::inference::Inference;
 use crate::line::LineTool;
 use crate::move_tool::MoveTool;
+use crate::paint::PaintTool;
 use crate::push_pull::PushPullTool;
 use crate::rectangle::RectangleTool;
 use crate::rotate::RotateTool;
 use crate::scenery::Scenery;
@@ -36,8 +37,13 @@ pub struct ToolContext<'a> {
     /// Whether Shift is held.
     pub shift: bool,
     /// The selected edges and faces.
     pub selection: &'a BTreeSet<EntityId>,
+    /// The material the Paint tool paints with (None: the default material).
+    pub material: Option<MaterialId>,
+    /// Whether the sampling modifier is held (Cmd on macOS, Ctrl elsewhere): the Paint tool
+    /// then picks up the material clicked instead of painting.
+    pub sample: bool,
 }
 
 #[derive(Clone, Debug, PartialEq)]
 pub enum ToolEvent {
@@ -67,8 +73,10 @@ pub struct ToolOutput {
     /// Segments to draw as a live preview.
     pub preview: Vec<[DVec3; 2]>,
     /// Feedback for the user, e.g. why typed text was ignored.
     pub message: Option<String>,
+    /// A material the tool picked up to paint with next (Some(None): the default material).
+    pub sampled: Option<Option<MaterialId>>,
 }
 
 /// An interactive way to build commands. Tools never edit the model themselves.
 pub trait Tool {
@@ -96,13 +104,14 @@ pub enum ToolKind {
     Move,
     Rotate,
     TapeMeasure,
     Erase,
+    Paint,
 }
 
 impl ToolKind {
     /// Every tool, in toolbar order.
-    pub const ALL: [ToolKind; 10] = [
+    pub const ALL: [ToolKind; 11] = [
         ToolKind::Select,
         ToolKind::Line,
         ToolKind::Rectangle,
         ToolKind::Circle,
@@ -111,8 +120,9 @@ impl ToolKind {
         ToolKind::Move,
         ToolKind::Rotate,
         ToolKind::TapeMeasure,
         ToolKind::Erase,
+        ToolKind::Paint,
     ];
 
     pub fn make(self) -> Box<dyn Tool> {
         match self {
@@ -125,8 +135,9 @@ impl ToolKind {
             ToolKind::Move => Box::new(MoveTool::new()),
             ToolKind::Rotate => Box::new(RotateTool::new()),
             ToolKind::TapeMeasure => Box::new(TapeMeasureTool::new()),
             ToolKind::Erase => Box::new(EraseTool::new()),
+            ToolKind::Paint => Box::new(PaintTool::new()),
         }
     }
 
     /// The tool's name, as its `Tool::name` reports it.
@@ -141,8 +152,9 @@ impl ToolKind {
             ToolKind::Move => "Move",
             ToolKind::Rotate => "Rotate",
             ToolKind::TapeMeasure => "Tape Measure",
             ToolKind::Erase => "Erase",
+            ToolKind::Paint => "Paint",
         }
     }
 
     /// The key that selects the tool.
@@ -157,8 +169,9 @@ impl ToolKind {
             ToolKind::Move => 'm',
             ToolKind::Rotate => 'q',
             ToolKind::TapeMeasure => 't',
             ToolKind::Erase => 'e',
+            ToolKind::Paint => 'b',
         }
     }
 
     pub fn from_shortcut(c: char) -> Option<ToolKind> {
```

Apply to `docs/agent-api.md`:
```diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -200,8 +200,14 @@ face filling an opening is unpainted. Push/pull gives the extruded faces (the ca
 sides, also the walls and floor of a recess) the pushed face's front and back materials; a base
 that turns over (a pulled flat sheet) keeps each material on the side it was on, so its old
 front is now its back.
 
+In the window, the Paint tool (`b`) paints the side of a face facing the viewer with the
+current material (a click on something selected paints the whole selection's faces and
+groups, a click on a group paints the group), and Cmd-click (Ctrl-click on other systems)
+picks up the clicked side's material instead. The Materials list under Entity Info chooses the
+current material and adds, renames, recolors and deletes materials, all through these commands.
+
 ### Groups and components
 
 Commands and queries work on the open context: the root at first. `{"type": "open_instance",
 "id": "i4"}` opens an instance of the open context for editing, and `{"type": "close_instance"}`
```

Apply to `docs/superpowers/specs/2026-10-10-tenon-design.md`:
```diff
--- a/docs/superpowers/specs/2026-10-10-tenon-design.md
+++ b/docs/superpowers/specs/2026-10-10-tenon-design.md
@@ -198,9 +198,9 @@ Every milestone's acceptance includes: new capabilities are reachable as `sessio
 - **M6b**: rotate: a `rotate` command (vertices, edges, faces and instances about an axis through a center, angle in degrees, exact quarter turns; optional `copies` for radial arrays) and a Rotate tool (`q`: center on a face turns about its normal, else about Z; reference click, then angle click with 15-degree snapping, or a typed angle; Ctrl copies, "5x" and "/5" arrays). Turned placements work everywhere: render normals, picking, box select, inference and the tools' camera inside an open turned instance, bounds, Entity Info (rotation line), explode, files and export.
 - **M6c**: guides and the Tape Measure: guide lines (infinite) and guide points are model entities (`g<n>` ids) of the context they are drawn in; `add_guide_line`, `add_guide_point` and `delete_guides` commands, and `erase`, `select`, `move`, `rotate`, `copy`, `make_group` and `explode` take guides; saved in `.skr` (schema version 2; version 1 still opens), never exported. Inference snaps to guide points, onto guide lines, and where guide lines cross edges or each other. Guides are drawn dashed (guide points as small crosses), picked, box-selected and erased like edges, and shown in Entity Info. Tape Measure tool (`t`): from a point the second click adds a guide point, from an edge or guide line a parallel guide line; typed distances, retyped right after to adjust; Ctrl switches to measuring only. Edit > Delete Guides.
 - **M6d**: curves: edges carry an optional curve id; the edges of a curve always form one unbranched, non-straight chain (sticky splits keep both pieces on the curve, erasing leaves the rest a curve, a piece that falls off gets a new id, a lone edge is plain), and copies, groups and explodes get new curve ids; the shape (circle or arc, center, normal, radius, angle, segments, length) is derived from the edges, so moves and rotations need no upkeep; `add_circle` tags its edges; an `add_arc` command (center, start, signed angle in degrees, plane normal, 2-360 segments, default 12); a `curve` query and `curve`/`soft` fields on edges; `.skr` schema version 3 (versions 1 and 2 still open). Push/pull of a face bounded by a curve gives the cap a curve of its own and makes the swept edges between curve segments soft; a soft edge bounding fewer than two faces hardens. In the window, soft edges are not drawn, picked or snapped to; clicking a curve edge with Select selects the whole curve; Entity Info shows circles and arcs with radius, angle, segments and length. Arc tool (`a`, SketchUp's 2-point arc: start, end, bulge; typed chord and bulge lengths, "12s" segments). Smooth shading across soft edges is deferred.
 - **M7a**: camera: commands `orbit` (yaw and pitch in degrees about the target), `pan` (meters along the screen), `zoom` (factor), `zoom_extents` (the whole model in root coordinates, guides excluded; an empty model leaves the camera), `standard_view` (top, bottom, front, back, left, right, iso; same target and distance) and `set_projection` (perspective or parallel; the view height at the target is kept, so a parallel view's zoom is the eye's distance); `set_camera` takes `up` (the screen's up in views straight down or up, which are now allowed) and `projection`; the `camera` query reports `up`, `projection` and `view_height`; camera commands are not undo steps and do not change the revision; `.skr` schema version 4 saves `up` and `projection` (versions 1-3 open in perspective). A parallel view sees 100 eye-to-target distances either side of the target, behind the eye too: rays start there, and picking, inference, overlay sizes (snap markers, guide crosses, pick radius, protractor), guide line spans, render depth offsets and shading work in both projections; `render_view` takes `up` and `projection`. Camera menu: Zoom Extents (Shift+Z), the standard views, Perspective and Parallel Projection; camera moves from the menu keep the tool's operation going. Erase Selection moves from the toolbar to the Edit menu so the toolbar keeps fitting a 1280-point window.
-- **M7b**: materials: named sRGB colors (`m<n>` ids from the entity id counter) belonging to the whole model; each face side and each instance carries an optional material (none is the default material); an instance's material shows on face sides inside it without one (the nearest painted instance wins), and exploding bakes it onto them. Commands `add_material` (optional name, default "Material", "Material 2", ...; names distinct, not blank, at most 256 characters), `change_material` (name and/or color), `delete_material` (unpaints everything painted with it, one undo step) and `paint` (faces and instances of the open context, `side` front, back or both, null for the default); a `materials` query (with face and instance use counts); `front_material`/`back_material` on faces and `material` on instances in queries; `changes.materials` lists materials a command added, removed or changed. Paint follows geometry: split pieces keep it, merges keep the containing face's, copies/groups/components keep it, push/pull gives the cap and sides the pushed face's materials, a face turned over keeps each material on its side. `.skr` schema version 5 (versions 1-4 open unpainted); files painting with missing materials or with bad material names are refused. Rendering shows each side's color (selection tint on top); export writes OBJ with a `.mtl` beside it, glTF with one material per color (front side only; STL has no colors). Paint Bucket tool (`b`: click a face to paint the side facing the viewer, a selected face paints the whole selection's faces and instances, an instance paints the instance; Cmd/Ctrl-click samples a material) and a Materials window (Window menu) listing materials with swatches, add, rename, recolor, delete and the current material. Opacity and textures are deferred.
+- **M7b**: materials: named sRGB colors (`m<n>` ids from the entity id counter) belonging to the whole model; each face side and each instance carries an optional material (none is the default material); an instance's material shows on face sides inside it without one (the nearest painted instance wins), and exploding bakes it onto them. Commands `add_material` (optional name, default "Material", "Material 2", ...; names distinct, not blank, at most 256 characters), `change_material` (name and/or color), `delete_material` (unpaints everything painted with it, one undo step) and `paint` (faces and instances of the open context, `side` front, back or both, null for the default); a `materials` query (with face and instance use counts); `front_material`/`back_material` on faces and `material` on instances in queries; `changes.materials` lists materials a command added, removed or changed. Paint follows geometry: split pieces keep it, merges keep the containing face's, copies/groups/components keep it, push/pull gives the cap and sides the pushed face's materials, a face turned over keeps each material on its side. `.skr` schema version 5 (versions 1-4 open unpainted); files painting with missing materials or with bad material names are refused. Rendering shows each side's color (selection tint on top); export writes OBJ with a `.mtl` beside it, glTF with one material per color (front side only; STL has no colors). Paint tool (`b`, last toolbar button: click a face to paint the side facing the viewer with the current material, a click on something selected paints the whole selection's faces and instances, an instance paints the instance; Cmd-click (Ctrl elsewhere; Alt is the orbit modifier) picks up the clicked side's material) and a Materials list in the right panel under Entity Info (Default plus every material with a swatch; clicking one makes it current and switches to Paint; Add, Delete, a name field renaming on Enter, and a 12-color palette to recolor); Entity Info shows a face's front and back materials and a painted instance's. Opacity, textures and a free color picker are deferred.
 
 ## Risks
 
 - **Sticky geometry robustness**: edge merging and face detection under floating-point error is the main risk. Mitigation: robust predicates, explicit tolerances, invariant checks in every test, fuzz tests on random edge sequences.
```

- [ ] **Step 4: Run the tests to verify they pass**

`cargo test -p tools paint`, `cargo test -p app` (118 tests in the bin); workspace 711 passed; fmt and clippy clean.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` (expected totals: 693, 700, 705, 711 after Tasks 1-4).

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/app/src/app.rs crates/app/src/editor.rs crates/app/src/info.rs crates/app/src/ui.rs crates/tools/src/arc.rs crates/tools/src/circle.rs crates/tools/src/erase.rs crates/tools/src/lib.rs crates/tools/src/line.rs crates/tools/src/move_tool.rs crates/tools/src/paint.rs crates/tools/src/push_pull.rs crates/tools/src/rectangle.rs crates/tools/src/rotate.rs crates/tools/src/select.rs crates/tools/src/tape.rs crates/tools/src/tool.rs docs/agent-api.md docs/superpowers/specs/2026-10-10-tenon-design.md
git commit -m "Paint tool (b) and Materials list: paint the side facing the viewer, whole selections and groups, Cmd/Ctrl-click picks up a material; add, rename, recolor and delete materials in the right panel; face and group materials in Entity Info"
```

## Self-review notes

- **Spec coverage:** materials data and commands (Tasks 1-2), rendering and export (Task 3), Paint tool and Materials list (Task 4).
- **Deferred** (from the scratch build):

No split needed: the four tasks are each green on their own and total about 3300 changed lines (about half tests). If a plan must be shorter, split after p2 (M7b1 data + commands + files; M7b2 render, export, tool, UI).

Deferred (record in memory for M8 or later):
- Opacity/translucent materials and textures (spec non-goal says "basic projection" only); a free color picker in the UI (needs commit-on-close to avoid one undo step per drag frame); hex color entry.
- Purge unused materials command; material list sorting/search; material thumbnails in the Paint tool cursor.
- Back-side materials are not exported (front only); OBJ `Kd` only (no `Ka`/`Ks`/`d`); the `.mtl` replaces any file of that name beside the OBJ without asking (the export dialog does not mention it).
- Paint Bucket modifiers from SketchUp not done: Shift (paint all faces of the same material), Ctrl fill-connected, Alt replace-in-object. Painting edges (SketchUp edge colors) not supported.
- Material names: stored untrimmed, uniqueness is exact (case-sensitive); no limit on the number of materials beyond the 256 MiB document limit.
- `Editor.material` is not saved with the model and resets on New/Open (current-material persistence across sessions not done).
- Toolbar now about 1240/1280 pt: the next button needs toolbar wrapping (already deferred to M8).
- Render rebuilds all buffers on any revision change, including recolors (pre-existing whole-scene rebuild).
- Merge of two differently painted coplanar faces takes the paint of the face containing the merged region's interior point (arbitrary but deterministic), as orientation already did.
- Explode bakes the instance material onto default face sides; the inverse (make_group lifting a common material into the instance) is not done.
- `paint` on instance ids ignores `side` silently.
