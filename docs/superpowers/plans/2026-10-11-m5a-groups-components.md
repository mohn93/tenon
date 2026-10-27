# M5a Groups and Components Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Groups and components through commands and queries: make them, explode them, copy, move, erase and select instances, open an instance to edit what it places, and see all of it drawn.

**Architecture:**
- **kernel:**
  - `EntityId::Instance` (`"i4"`) and `DefinitionId` (`"d5"`).
  - A `Model` (one mesh) also holds `Instance`s: a definition id plus a `DAffine3` transform, with recorded, invertible ops.
  - A new `Scene` (`scene.rs`) holds the root mesh and the `Definition`s (name, kind, mesh):
    - one id counter across all meshes, so ids stay unique model-wide and are never reused;
    - `SceneChange` = ordered `Step`s (an edit inside one mesh, or adding/removing a definition), with an exact inverse.
  - Scene operations (`group.rs`): `make_group`, `explode`, `copy_instances`, `erase_instances`, plus bounds and path resolution.
  - Copy's drawing code becomes a reusable `Stencil` (geometry captured by position and drawn again under a transform).
- **session:**
  - `Document` keeps a `Scene` and an open context: a path of instances.
  - Commands and queries address the open context's mesh, in its own coordinates.
  - New commands: `make_group`, `make_component`, `explode`, `open_instance`, `close_instance`. `move`, `copy`, `erase` and `select` accept instance ids.
  - New queries: `definitions`, `context`. `entities` lists instances; `stats` counts them.
- **render:** `scene_geometry` draws every instance placed by its transform:
  - a selected instance lights up everything it places;
  - while an instance is open, the edges outside it fade and its bounding box is outlined.
- **app/api/docs:**
  - the window and `render_view` draw the scene;
  - Entity Info names a selected group or component;
  - a JSON-RPC test, `agent-api.md`, and the spec amendment splitting M5 into M5a-c.

**Tech Stack:** Rust 2024; glam 0.34.1 (`DAffine3`, already with the `serde` feature in the kernel); no dependency changes.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md`:
- section 1, "Groups and components";
- section 5, Commands and Queries;
- the Milestones list. Task 5 amends section 1 and the milestone lines: M5 becomes M5a (this plan), M5b (tools) and M5c (files).

## Global Constraints

- Every new capability is a `session::Command` or `Query` reachable through the API, covered by a command-driven test. Tools only emit commands.
- Each modeling command is one undo step. `SceneChange::inverse` undoes it exactly. Kernel tests call `scene.validate()` (or `model.validate()`) after edits.
- Command inputs and the points they produce must be finite and within +/-`MAX_COORDINATE` (1e6 m). Errors carry the offending ids.
- No dependency changes.
- Git:
  - work on `dev`;
  - stage only the files you changed;
  - short plain commit messages, no emoji;
  - run `cargo fmt --all` before each commit;
  - `cargo clippy --workspace --all-targets -- -D warnings` stays clean.
- Do not launch the app window; everything is tested headless.

## Spec notes (rulings made in this plan)

- **Scope split.** M5 ("groups and components; save/load; export") is split like M4:
  - **M5a (this plan):** the model, commands, queries and drawing.
  - **M5b:** tools — picking and box-selecting instances, double-click to open/close, Move on instances, inference on geometry inside instances, tools inside a moved instance, toolbar and shortcuts.
  - **M5c:** `.skr` save/load and OBJ/STL/glTF export.

  M5a is fully usable by agents. In the window it shows groups and highlights instance selections, but tools still see only the open context's own geometry (see "Known gaps").
- **Ids.** One counter serves every mesh, so `"f12"` names one entity anywhere in the model. A component's entities are shared by all its instances: `"f12"` inside a component is one face, drawn once per instance. Instances are `"i<n>"`; definitions are `"d<n>"` (a separate `DefinitionId`, not an `EntityId`, since nothing selects or erases a definition directly).
- **Groups vs components.**
  - Both are definitions with their own mesh.
  - Copying a component instance places the same definition.
  - Copying a group copies its definition (and the groups inside it; components inside are placed again).
  - A group's definition is removed with its last instance (erase, explode). A component's definition stays with no instances; purging comes with files in M5c.
- **Making a group.** The given edges, faces (with their edges) and instances move into a new definition, placed by one new instance with an identity transform. The group keeps the coordinates of the context it was made in.
  - Edges that still bound a face left outside stay outside too (and are copied in).
  - Edges and vertices left without a face or edge are removed outside.
  - Nothing heals or merges outside.
  - The instance becomes the selection.
  - A component needs a non-empty name; groups are unnamed (`""`).
- **Exploding.** The definition's geometry is drawn into the parent under the instance's transform, like new edges, so it merges with what it touches (a grouped box top exploded back onto the box closes it again).
  - Instances inside come out with composed transforms and new ids.
  - What comes out (created edges, faces, instances) becomes the selection.
- **Drawing faces onto existing outlines.** Copies and explodes now close a face where every side already existed (an empty outline, an opening), oriented like the original. Before, such a copy made no face. This is the one behavior change to an existing command (`copy`), pinned by a test.
- **Open context.**
  - `open_instance` / `close_instance` push and pop a path of instances. They are not undo steps and do not change the revision, like `select`, and they clear the selection.
  - All commands and queries address the open context's mesh. Points, offsets and positions are in that mesh's own coordinates (the SketchUp Ruby API convention). Ids from other meshes are `not_found`.
  - Undo and redo apply wherever a change was made. After any change, open instances that no longer exist are closed.
  - A failed atomic batch also restores the open context.
  - `Command::changes_model` is true for open/close, so the editor drops a tool's operation in progress when an agent opens or closes an instance.
- **Transforms.** `DAffine3`. Only translations are produced in M5a (moving and copying instances); rotation and scaling come with the Rotate and Scale tools. Bounds of an instance are its definition's box corners under its transform.
- **Queries.**
  - `entity` on an instance returns `InstanceInfo`: id, definition, name, kind, `transform` (12 numbers, column-major: x axis, y axis, z axis, translation) and `bounds` in the open context's coordinates.
  - `entities` filters instances by `within` using their bounding box.
  - `stats` counts the open context plus the number of definitions. This changes the stats JSON; one existing test is updated.
- **Rendering.**
  - Selection ids belong to the open context's own placement: a face selected inside an open component is lit in that instance only, not in the others.
  - Faded edges use `FADED_EDGE_COLOR`. The open instance's box is drawn in `CONTEXT_BOX_COLOR` (12 lines; it can coincide with the geometry's own edges for a box).
  - Rebuild triggers: the app rebuilds model buffers when the revision, the selection or the open context changes.

## Known gaps (left for M5b, recorded in the ledger)

- Tools get `Document::model()`, the open context's mesh. In an instance with a non-identity transform they work in the wrong frame. Tools do not pick instances, and inference does not snap to geometry inside instances.
- The Move tool ignores instance ids in the selection. Erase (Delete key) works on a selected instance.

## File Structure

| File | Responsibility |
|---|---|
| `crates/kernel/src/ids.rs` | `EntityId::Instance` ("i"), `DefinitionId` ("d"), shared prefix parser |
| `crates/kernel/src/model.rs` | `Instance`, instance ops, `move_instances`, id reservation |
| `crates/kernel/src/scene.rs` (new) | `Scene`, `Definition`, `DefinitionKind`, `MeshId`, `Step`, `SceneChange`; edit/apply/validate, `resolve`, bounds |
| `crates/kernel/src/group.rs` (new) | `make_group`, `explode`, `copy_instances`, `erase_instances`, prune; `Model::detach` |
| `crates/kernel/src/transform.rs` | `Stencil` (capture and draw), `copy` rewritten on it |
| `crates/kernel/src/edit.rs` | `KernelError::InstanceNotFound` |
| `crates/session/src/command.rs`, `query.rs` | new commands, queries and result types |
| `crates/session/src/document.rs` | Document over `Scene` with an open context |
| `crates/session/src/document/groups.rs` (new) | group/explode/open/close command handling, `InstanceInfo` |
| `crates/render/src/scene.rs` | `scene_geometry` |
| `crates/render/src/renderer.rs`, `snapshot.rs` | `set_scene`, `capture(scene, context, selection, ...)` |
| `crates/app/src/{app,host,info}.rs`, `crates/api/src/headless.rs` | callers; Entity Info for instances |
| `docs/agent-api.md`, spec | documentation |

The code below was written and verified in a scratch copy of `dev` at 371f9b2. Each task was checked on its own with `cargo fmt --all --check`, clippy with `-D warnings`, and the full test suite. Workspace test totals after each task: 396, 407, 418, 422, 423 (from 386).

Code for new files is given in full. Changes to existing files are given as unified diffs against the state the previous task leaves (apply them by hand or with `git apply`; the context lines show where each change goes).

---

### Task 1: Instances, scenes and make_group in the kernel

**Files:**
- Create: `crates/kernel/src/scene.rs`, `crates/kernel/src/group.rs`
- Modify: `crates/kernel/src/ids.rs`, `crates/kernel/src/model.rs`, `crates/kernel/src/transform.rs`, `crates/kernel/src/lib.rs`
- Modify (new `EntityId` arm only): `crates/session/src/document.rs`, `crates/app/src/editor.rs`, `crates/app/src/info.rs`, `crates/tools/src/erase.rs`, `crates/tools/src/move_tool.rs`
- Test: `crates/kernel/tests/groups.rs` (new), unit test in `ids.rs`

**Interfaces:**
- Consumes:
  - `Model::draw_segment`, `find_vertex`, `orient`, `face_with_outer` (existing, crate-private);
  - `loop_has_side` (edit.rs).
- Produces:
  - **ids:** `kernel::{InstanceId, DefinitionId}`; `EntityId::Instance(InstanceId)` serialized `"i<n>"`; `DefinitionId` serialized `"d<n>"`.
  - **instances in a mesh:**
    - `kernel::Instance { definition: DefinitionId, transform: DAffine3 }`;
    - `Op::{AddInstance, RemoveInstance, TransformInstance}`;
    - `Model::{instance, instances, is_empty}`;
    - `Model::move_instances(&mut self, &[InstanceId], DVec3) -> ChangeSet`;
    - crate-private: `insert_instance`, `insert_instance_as`, `remove_instance`, `next_id`, `reserve_ids`.
  - **scene types:**
    - `kernel::{Scene, Definition, DefinitionKind, MeshId, Step, SceneChange}`;
    - `SceneChange::{is_empty, inverse, join, summary}`.
  - **scene methods:**
    - `Scene::{new, from_root, root, mesh, definition, definitions, meshes, instance_count, apply, validate}`;
    - `Scene::edit(&mut self, MeshId, impl FnOnce(&mut Model) -> Result<ChangeSet, KernelError>) -> Result<SceneChange, KernelError>`;
    - `Scene::make_group(&mut self, parent: MeshId, &[EdgeId], &[FaceId], &[InstanceId], DefinitionKind, name: &str) -> Result<(InstanceId, SceneChange), KernelError>`.
  - **crate-private helpers:** `Stencil`, `Model::{stencil, draw_stencil, detach}`, `Scene::add_definition`.

- [ ] **Step 1: Write the failing tests**

Add a test to the `tests` module of `crates/kernel/src/ids.rs` and create the kernel test file. The `ids.rs` change appears in Step 3's diff of that file: `entity_ids_round_trip_as_strings` gains the instance and definition cases. Apply that test part now.

Create `crates/kernel/tests/groups.rs`:
```rust
use glam::{DAffine3, DVec3};
use kernel::{
    DefinitionKind, EdgeId, EntityId, FaceId, InstanceId, MeshId, Model, Scene, SceneChange,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A scene whose root holds a 2 x 3 x 1 box with its corner at `at`.
fn box_scene(at: DVec3) -> Scene {
    let mut m = Model::new();
    let c = m
        .add_polygon(&[
            at,
            at + p(2.0, 0.0, 0.0),
            at + p(2.0, 3.0, 0.0),
            at + p(0.0, 3.0, 0.0),
        ])
        .unwrap();
    let face = c
        .summary()
        .created
        .iter()
        .find_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .unwrap();
    m.push_pull(face, 1.0).unwrap();
    m.validate().unwrap();
    Scene::from_root(m)
}

fn all(m: &Model) -> (Vec<EdgeId>, Vec<FaceId>) {
    (
        m.edges().map(|(id, _)| id).collect(),
        m.faces().map(|(id, _)| id).collect(),
    )
}

fn counts(m: &Model) -> (usize, usize, usize, usize) {
    (
        m.vertices().count(),
        m.edges().count(),
        m.faces().count(),
        m.instances().count(),
    )
}

fn group_all(s: &mut Scene, kind: DefinitionKind) -> (InstanceId, SceneChange) {
    let (edges, faces) = all(s.root());
    s.make_group(MeshId::Root, &edges, &faces, &[], kind, "Box")
        .unwrap()
}

fn definition_of(s: &Scene, i: InstanceId) -> kernel::DefinitionId {
    s.root().instance(i).unwrap().definition
}

#[test]
fn grouping_a_box_moves_it_into_a_definition() {
    let mut s = box_scene(DVec3::ZERO);
    let before = s.clone();
    let (i, change) = group_all(&mut s, DefinitionKind::Group);
    s.validate().unwrap();

    assert_eq!(counts(s.root()), (0, 0, 0, 1));
    let instance = s.root().instance(i).unwrap();
    assert_eq!(instance.transform, DAffine3::IDENTITY);
    let d = s.definition(instance.definition).unwrap();
    assert_eq!((d.name.as_str(), d.kind), ("Box", DefinitionKind::Group));
    assert_eq!(counts(&d.mesh), (8, 12, 6, 0));
    assert_eq!(s.instance_count(instance.definition), 1);

    let summary = change.summary();
    assert!(summary.created.contains(&EntityId::Instance(i)));
    assert_eq!(summary.deleted.len(), 8 + 12 + 6);
    assert_eq!(summary.created.len(), 1 + 8 + 12 + 6);

    let after = s.clone();
    s.apply(&change.inverse());
    assert_eq!(s, before);
    s.apply(&change);
    assert_eq!(s, after);
}

#[test]
fn grouped_faces_keep_their_orientation() {
    let mut s = box_scene(DVec3::ZERO);
    let normals = |m: &Model| {
        let mut n: Vec<[i64; 3]> = m
            .faces()
            .map(|(_, f)| f.normal.to_array().map(|c| c.round() as i64))
            .collect();
        n.sort();
        n
    };
    let before = normals(s.root());
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    let d = definition_of(&s, i);
    assert_eq!(normals(&s.definition(d).unwrap().mesh), before);
}

#[test]
fn ids_stay_unique_across_meshes_and_are_never_reused() {
    let mut s = box_scene(DVec3::ZERO);
    let (_, change) = group_all(&mut s, DefinitionKind::Group);
    let used = change.summary().created;
    s.apply(&change.inverse());
    let again = s
        .edit(MeshId::Root, |m| {
            m.add_edge(p(5.0, 0.0, 0.0), p(6.0, 0.0, 0.0))
        })
        .unwrap()
        .summary()
        .created;
    for id in again {
        assert!(!used.contains(&id), "{id} reused");
    }
    s.validate().unwrap();
}

#[test]
fn grouping_one_face_of_a_box_leaves_its_shared_edges() {
    let mut s = box_scene(DVec3::ZERO);
    let top = s
        .root()
        .faces()
        .find(|(_, f)| f.normal.z > 0.5)
        .map(|(id, _)| id)
        .unwrap();
    let (i, change) = s
        .make_group(MeshId::Root, &[], &[top], &[], DefinitionKind::Group, "Top")
        .unwrap();
    s.validate().unwrap();
    assert_eq!(counts(s.root()), (8, 12, 5, 1));
    let d = definition_of(&s, i);
    assert_eq!(counts(&s.definition(d).unwrap().mesh), (4, 4, 1, 0));
    assert_eq!(change.summary().deleted, vec![EntityId::Face(top)]);
}

#[test]
fn grouping_a_lone_face_takes_its_edges_along() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let face = m.faces().next().unwrap().0;
    let mut s = Scene::from_root(m);
    let (i, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[face],
            &[],
            DefinitionKind::Group,
            "Sheet",
        )
        .unwrap();
    s.validate().unwrap();
    assert_eq!(counts(s.root()), (0, 0, 0, 1));
    let d = definition_of(&s, i);
    assert_eq!(counts(&s.definition(d).unwrap().mesh), (4, 4, 1, 0));
}

#[test]
fn a_selected_edge_still_bounding_a_face_stays_behind() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let edge = m.edges().next().unwrap().0;
    let mut s = Scene::from_root(m);
    let (i, _) = s
        .make_group(
            MeshId::Root,
            &[edge],
            &[],
            &[],
            DefinitionKind::Group,
            "Edge",
        )
        .unwrap();
    s.validate().unwrap();
    assert_eq!(counts(s.root()), (4, 4, 1, 1));
    let d = definition_of(&s, i);
    assert_eq!(counts(&s.definition(d).unwrap().mesh), (2, 1, 0, 0));
}

#[test]
fn an_opening_stays_open_in_the_group() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(4.0, 0.0, 0.0),
        p(4.0, 4.0, 0.0),
        p(0.0, 4.0, 0.0),
    ])
    .unwrap();
    m.add_polygon(&[
        p(1.0, 1.0, 0.0),
        p(2.0, 1.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(1.0, 2.0, 0.0),
    ])
    .unwrap();
    let (small, big): (Vec<FaceId>, Vec<FaceId>) = m
        .faces()
        .map(|(id, _)| id)
        .partition(|id| m.area(*id).unwrap() < 2.0);
    m.erase(&[], &small);
    let mut s = Scene::from_root(m);
    let (edges, _) = all(s.root());
    let (i, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &big,
            &[],
            DefinitionKind::Group,
            "Frame",
        )
        .unwrap();
    s.validate().unwrap();
    let d = definition_of(&s, i);
    let mesh = &s.definition(d).unwrap().mesh;
    assert_eq!(counts(mesh), (8, 8, 1, 0));
    assert_eq!(mesh.faces().next().unwrap().1.holes.len(), 1);
}

#[test]
fn grouping_an_instance_with_geometry_nests_it() {
    let mut s = box_scene(DVec3::ZERO);
    let (inner, _) = group_all(&mut s, DefinitionKind::Component);
    s.edit(MeshId::Root, |m| {
        m.add_edge(p(5.0, 0.0, 0.0), p(6.0, 0.0, 0.0))
    })
    .unwrap();
    let (edges, faces) = all(s.root());
    let (outer, change) = s
        .make_group(
            MeshId::Root,
            &edges,
            &faces,
            &[inner],
            DefinitionKind::Group,
            "Both",
        )
        .unwrap();
    s.validate().unwrap();
    assert_eq!(counts(s.root()), (0, 0, 0, 1));
    let d = definition_of(&s, outer);
    let mesh = &s.definition(d).unwrap().mesh;
    assert_eq!(counts(mesh), (2, 1, 0, 1));
    assert!(mesh.instance(inner).is_some());
    assert!(
        change
            .summary()
            .modified
            .contains(&EntityId::Instance(inner))
    );
}

#[test]
fn an_instance_listed_twice_moves_once() {
    let mut s = box_scene(DVec3::ZERO);
    let (inner, _) = group_all(&mut s, DefinitionKind::Group);
    let (outer, _) = s
        .make_group(
            MeshId::Root,
            &[],
            &[],
            &[inner, inner],
            DefinitionKind::Group,
            "Outer",
        )
        .unwrap();
    s.validate().unwrap();
    let d = definition_of(&s, outer);
    assert_eq!(counts(&s.definition(d).unwrap().mesh), (0, 0, 0, 1));
}

#[test]
fn moving_an_instance_translates_it_and_undoes_exactly() {
    let mut s = box_scene(DVec3::ZERO);
    let (i, _) = group_all(&mut s, DefinitionKind::Group);
    let before = s.clone();
    let change = s
        .edit(MeshId::Root, |m| {
            Ok(m.move_instances(&[i], p(1.0, 2.0, 3.0)))
        })
        .unwrap();
    assert_eq!(
        s.root().instance(i).unwrap().transform,
        DAffine3::from_translation(p(1.0, 2.0, 3.0))
    );
    assert_eq!(change.summary().modified, vec![EntityId::Instance(i)]);
    s.apply(&change.inverse());
    assert_eq!(s, before);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kernel --test groups`
Expected: compile errors: `unresolved imports` for `kernel::DefinitionKind`, `InstanceId`, `MeshId`, `Scene`, `SceneChange`.

- [ ] **Step 3: Implement**

The session, app and tools changes only add the new `EntityId::Instance` arm to exhaustive matches; Task 3 replaces the session ones properly.

Apply to `crates/kernel/src/ids.rs`:
```diff
--- a/crates/kernel/src/ids.rs
+++ b/crates/kernel/src/ids.rs
@@ -15,67 +15,73 @@ macro_rules! id_type {
 
 id_type!(VertexId);
 id_type!(EdgeId);
 id_type!(FaceId);
+id_type!(InstanceId);
 
-/// Any kernel entity. Serialized as a short string: "v12", "e3", "f7".
+/// Any kernel entity. Serialized as a short string: "v12", "e3", "f7", "i4".
 #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
 #[serde(try_from = "String", into = "String")]
 pub enum EntityId {
     Vertex(VertexId),
     Edge(EdgeId),
     Face(FaceId),
+    /// A placed group or component.
+    Instance(InstanceId),
 }
 
 impl fmt::Display for EntityId {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
         match self {
             EntityId::Vertex(id) => write!(f, "v{}", id.0),
             EntityId::Edge(id) => write!(f, "e{}", id.0),
             EntityId::Face(id) => write!(f, "f{}", id.0),
+            EntityId::Instance(id) => write!(f, "i{}", id.0),
         }
     }
 }
 
 impl FromStr for EntityId {
     type Err = String;
 
     fn from_str(s: &str) -> Result<Self, Self::Err> {
-        let mut chars = s.chars();
-        let kind = chars
-            .next()
-            .ok_or_else(|| format!("invalid entity id {s:?}"))?;
-        // Canonical decimal only: digits, no sign, no leading zeros ("v0" is fine, "v007" is not).
-        let digits = chars.as_str();
-        let canonical = !digits.is_empty()
-            && digits.bytes().all(|b| b.is_ascii_digit())
-            && (digits == "0" || !digits.starts_with('0'));
-        if !canonical {
-            return Err(format!("invalid entity id {s:?}"));
-        }
-        let n: u64 = digits
-            .parse()
-            .map_err(|_| format!("invalid entity id {s:?}"))?;
+        let (kind, n) = parse_prefixed(s).ok_or_else(|| format!("invalid entity id {s:?}"))?;
         match kind {
             'v' => Ok(EntityId::Vertex(VertexId(n))),
             'e' => Ok(EntityId::Edge(EdgeId(n))),
             'f' => Ok(EntityId::Face(FaceId(n))),
+            'i' => Ok(EntityId::Instance(InstanceId(n))),
             _ => Err(format!("invalid entity id {s:?}")),
         }
     }
 }
 
-/// Ids appear in JSON as strings: "v12" (vertex), "e3" (edge), "f7" (face).
+/// A letter followed by a canonical decimal number: digits, no sign, no leading zeros ("v0" is
+/// fine, "v007" is not).
+fn parse_prefixed(s: &str) -> Option<(char, u64)> {
+    let mut chars = s.chars();
+    let kind = chars.next()?;
+    let digits = chars.as_str();
+    let canonical = !digits.is_empty()
+        && digits.bytes().all(|b| b.is_ascii_digit())
+        && (digits == "0" || !digits.starts_with('0'));
+    if !canonical {
+        return None;
+    }
+    Some((kind, digits.parse().ok()?))
+}
+
+/// Ids appear in JSON as strings: "v12" (vertex), "e3" (edge), "f7" (face), "i4" (instance).
 impl schemars::JsonSchema for EntityId {
     fn schema_name() -> std::borrow::Cow<'static, str> {
         "EntityId".into()
     }
 
     fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
         schemars::json_schema!({
             "type": "string",
-            "pattern": "^[vef](0|[1-9][0-9]*)$",
-            "description": "Entity id: v<n> vertex, e<n> edge, f<n> face"
+            "pattern": "^[vefi](0|[1-9][0-9]*)$",
+            "description": "Entity id: v<n> vertex, e<n> edge, f<n> face, i<n> instance"
         })
     }
 }
 
@@ -92,8 +98,58 @@ impl TryFrom<String> for EntityId {
         s.parse()
     }
 }
 
+/// A group or component definition. Serialized as "d<n>".
+#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
+#[serde(try_from = "String", into = "String")]
+pub struct DefinitionId(pub u64);
+
+impl fmt::Display for DefinitionId {
+    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
+        write!(f, "d{}", self.0)
+    }
+}
+
+impl FromStr for DefinitionId {
+    type Err = String;
+
+    fn from_str(s: &str) -> Result<Self, Self::Err> {
+        match parse_prefixed(s) {
+            Some(('d', n)) => Ok(DefinitionId(n)),
+            _ => Err(format!("invalid definition id {s:?}")),
+        }
+    }
+}
+
+impl schemars::JsonSchema for DefinitionId {
+    fn schema_name() -> std::borrow::Cow<'static, str> {
+        "DefinitionId".into()
+    }
+
+    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
+        schemars::json_schema!({
+            "type": "string",
+            "pattern": "^d(0|[1-9][0-9]*)$",
+            "description": "Group or component definition id: d<n>"
+        })
+    }
+}
+
+impl From<DefinitionId> for String {
+    fn from(id: DefinitionId) -> String {
+        id.to_string()
+    }
+}
+
+impl TryFrom<String> for DefinitionId {
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
 
@@ -102,12 +158,19 @@ mod tests {
         for id in [
             EntityId::Vertex(VertexId(12)),
             EntityId::Edge(EdgeId(0)),
             EntityId::Face(FaceId(7)),
+            EntityId::Instance(InstanceId(4)),
         ] {
             assert_eq!(id.to_string().parse::<EntityId>().unwrap(), id);
         }
         assert_eq!(EntityId::Face(FaceId(7)).to_string(), "f7");
+        assert_eq!(EntityId::Instance(InstanceId(4)).to_string(), "i4");
+        assert_eq!("d3".parse::<DefinitionId>(), Ok(DefinitionId(3)));
+        assert_eq!(DefinitionId(3).to_string(), "d3");
+        for s in ["", "d", "f3", "d03", "d-1"] {
+            assert!(s.parse::<DefinitionId>().is_err(), "{s:?} parsed");
+        }
     }
 
     #[test]
     fn malformed_entity_ids_are_rejected() {
```

Apply to `crates/kernel/src/model.rs`:
```diff
--- a/crates/kernel/src/model.rs
+++ b/crates/kernel/src/model.rs
@@ -1,12 +1,12 @@
 use std::collections::{BTreeMap, BTreeSet};
 use std::sync::OnceLock;
 
-use glam::DVec3;
+use glam::{DAffine3, DVec3};
 use serde::{Deserialize, Serialize};
 
 use crate::geom::{LENGTH_TOL, vector_area};
-use crate::ids::{EdgeId, EntityId, FaceId, VertexId};
+use crate::ids::{DefinitionId, EdgeId, EntityId, FaceId, InstanceId, VertexId};
 use crate::index::Index;
 
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 pub struct Vertex {
@@ -26,8 +26,16 @@ pub struct Face {
     pub holes: Vec<Vec<VertexId>>,
     pub normal: DVec3,
 }
 
+/// A placed copy of a definition's geometry (a group or component).
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
+pub struct Instance {
+    pub definition: DefinitionId,
+    /// Maps the definition's coordinates into the coordinates of the mesh holding the instance.
+    pub transform: DAffine3,
+}
+
 /// One primitive, invertible change to the model.
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 pub enum Op {
     AddVertex(VertexId, Vertex),
@@ -37,8 +45,12 @@ pub enum Op {
     AddFace(FaceId, Face),
     RemoveFace(FaceId, Face),
     /// A vertex moved from the first position to the second.
     MoveVertex(VertexId, DVec3, DVec3),
+    AddInstance(InstanceId, Instance),
+    RemoveInstance(InstanceId, Instance),
+    /// An instance's transform changed from the first to the second.
+    TransformInstance(InstanceId, DAffine3, DAffine3),
 }
 
 impl Op {
     fn inverse(&self) -> Op {
@@ -49,8 +61,11 @@ impl Op {
             Op::RemoveEdge(id, e) => Op::AddEdge(id, e),
             Op::AddFace(id, f) => Op::RemoveFace(id, f),
             Op::RemoveFace(id, f) => Op::AddFace(id, f),
             Op::MoveVertex(id, from, to) => Op::MoveVertex(id, to, from),
+            Op::AddInstance(id, i) => Op::RemoveInstance(id, i),
+            Op::RemoveInstance(id, i) => Op::AddInstance(id, i),
+            Op::TransformInstance(id, from, to) => Op::TransformInstance(id, to, from),
         }
     }
 
     /// The entity this op touches and whether it exists before and after the op.
@@ -62,8 +77,11 @@ impl Op {
             Op::AddEdge(id, _) => (EntityId::Edge(*id), false, true),
             Op::RemoveEdge(id, _) => (EntityId::Edge(*id), true, false),
             Op::AddFace(id, _) => (EntityId::Face(*id), false, true),
             Op::RemoveFace(id, _) => (EntityId::Face(*id), true, false),
+            Op::AddInstance(id, _) => (EntityId::Instance(*id), false, true),
+            Op::RemoveInstance(id, _) => (EntityId::Instance(*id), true, false),
+            Op::TransformInstance(id, _, _) => (EntityId::Instance(*id), true, true),
         }
     }
 }
 
@@ -73,9 +91,9 @@ pub struct ChangeSet {
     pub ops: Vec<Op>,
 }
 
 /// Net effect of a ChangeSet per entity. A moved vertex is `modified`; the edges and faces using
-/// it change shape without being listed.
+/// it change shape without being listed. A moved instance is `modified` too.
 #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
 pub struct ChangeSummary {
     pub created: Vec<EntityId>,
     pub deleted: Vec<EntityId>,
@@ -112,14 +130,16 @@ impl ChangeSet {
         s
     }
 }
 
-/// The geometry of one mesh. Every mutation is recorded as an `Op` until `take_changes`.
+/// The geometry of one mesh, plus the instances placed in it. Every mutation is recorded as an
+/// `Op` until `take_changes`.
 #[derive(Clone, Debug, Default)]
 pub struct Model {
     vertices: BTreeMap<VertexId, Vertex>,
     edges: BTreeMap<EdgeId, Edge>,
     faces: BTreeMap<FaceId, Face>,
+    instances: BTreeMap<InstanceId, Instance>,
     next_id: u64,
     pending: Vec<Op>,
     /// Built on first use after a change; see `Model::index`.
     index: OnceLock<Index>,
@@ -127,9 +147,12 @@ pub struct Model {
 
 /// Two models are equal when they hold the same entities under the same ids.
 impl PartialEq for Model {
     fn eq(&self, other: &Self) -> bool {
-        self.vertices == other.vertices && self.edges == other.edges && self.faces == other.faces
+        self.vertices == other.vertices
+            && self.edges == other.edges
+            && self.faces == other.faces
+            && self.instances == other.instances
     }
 }
 
 impl Model {
@@ -167,8 +190,21 @@ impl Model {
     pub fn faces(&self) -> impl Iterator<Item = (FaceId, &Face)> {
         self.faces.iter().map(|(k, v)| (*k, v))
     }
 
+    pub fn instance(&self, id: InstanceId) -> Option<&Instance> {
+        self.instances.get(&id)
+    }
+
+    pub fn instances(&self) -> impl Iterator<Item = (InstanceId, &Instance)> {
+        self.instances.iter().map(|(k, v)| (*k, v))
+    }
+
+    /// True when the mesh holds no vertices and no instances.
+    pub fn is_empty(&self) -> bool {
+        self.vertices.is_empty() && self.instances.is_empty()
+    }
+
     pub(crate) fn pos(&self, id: VertexId) -> DVec3 {
         self.vertices[&id].pos
     }
 
@@ -249,15 +285,38 @@ impl Model {
                 if let Some(v) = self.vertices.get_mut(&id) {
                     v.pos = to;
                 }
             }
+            Op::AddInstance(id, i) => {
+                self.bump(id.0);
+                self.instances.insert(id, i);
+            }
+            Op::RemoveInstance(id, _) => {
+                self.instances.remove(&id);
+            }
+            Op::TransformInstance(id, _, to) => {
+                if let Some(i) = self.instances.get_mut(&id) {
+                    i.transform = to;
+                }
+            }
         }
     }
 
     fn bump(&mut self, used: u64) {
         self.next_id = self.next_id.max(used + 1);
     }
 
+    /// The id the next new entity gets.
+    pub(crate) fn next_id(&self) -> u64 {
+        self.next_id
+    }
+
+    /// Makes new entities get ids from `first` on (or later), so that ids stay unique across
+    /// the meshes of a scene.
+    pub(crate) fn reserve_ids(&mut self, first: u64) {
+        self.next_id = self.next_id.max(first);
+    }
+
     fn record(&mut self, op: Op) {
         self.apply_op(op.clone());
         self.pending.push(op);
     }
@@ -310,8 +369,37 @@ impl Model {
         self.record(Op::RemoveFace(id, f.clone()));
         f
     }
 
+    pub(crate) fn insert_instance(&mut self, instance: Instance) -> InstanceId {
+        let id = InstanceId(self.alloc());
+        self.record(Op::AddInstance(id, instance));
+        id
+    }
+
+    /// Adds an instance under an id it had elsewhere (an instance moved between meshes).
+    pub(crate) fn insert_instance_as(&mut self, id: InstanceId, instance: Instance) {
+        self.record(Op::AddInstance(id, instance));
+    }
+
+    pub(crate) fn remove_instance(&mut self, id: InstanceId) -> Instance {
+        let i = self.instances[&id].clone();
+        self.record(Op::RemoveInstance(id, i.clone()));
+        i
+    }
+
+    /// Moves instances by `offset` as one ChangeSet. Unknown ids are skipped.
+    pub fn move_instances(&mut self, ids: &[InstanceId], offset: DVec3) -> ChangeSet {
+        for id in ids {
+            if let Some(i) = self.instances.get(id) {
+                let from = i.transform;
+                let to = DAffine3::from_translation(offset) * from;
+                self.record(Op::TransformInstance(*id, from, to));
+            }
+        }
+        self.take_changes()
+    }
+
     /// Replaces a face's data, keeping its id.
     pub(crate) fn replace_face(&mut self, id: FaceId, face: Face) {
         self.remove_face(id);
         self.record(Op::AddFace(id, face));
@@ -341,8 +429,13 @@ impl Model {
                     return Err(format!("vertices {ia:?} and {ib:?} coincide"));
                 }
             }
         }
+        for (id, i) in &self.instances {
+            if !i.transform.is_finite() {
+                return Err(format!("instance {id:?} has a non-finite transform"));
+            }
+        }
         for (id, f) in &self.faces {
             if (f.normal.length() - 1.0).abs() > 1e-9 {
                 return Err(format!("face {id:?} normal is not unit length"));
             }
```

Apply to `crates/kernel/src/transform.rs`:
```diff
--- a/crates/kernel/src/transform.rs
+++ b/crates/kernel/src/transform.rs
@@ -1,9 +1,9 @@
 //! Moving and copying geometry.
 
 use std::collections::BTreeSet;
 
-use glam::DVec3;
+use glam::{DAffine3, DVec3};
 
 use crate::edit::KernelError;
 use crate::geom::{LENGTH_TOL, Plane, point_in_polygon, segment_distance_2d, vector_area};
 use crate::ids::{EdgeId, FaceId, VertexId};
@@ -120,63 +120,80 @@ impl Model {
         }
         if offset.length() < LENGTH_TOL {
             return Err(KernelError::ZeroDistance);
         }
-        let mut segments: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
+        let stencil = self.stencil(edges, faces);
+        let places: Vec<DAffine3> = (1..=count)
+            .map(|k| DAffine3::from_translation(offset * k as f64))
+            .collect();
+        self.draw_stencil(&stencil, &places);
+        Ok(self.take_changes())
+    }
+
+    /// Captures `edges` and `faces` (with their edges) by position.
+    pub(crate) fn stencil(&self, edges: &[EdgeId], faces: &[FaceId]) -> Stencil {
+        let mut sides: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
         for e in edges.iter().filter_map(|e| self.edge(*e)) {
-            segments.insert((e.v[0].min(e.v[1]), e.v[0].max(e.v[1])));
+            sides.insert((e.v[0].min(e.v[1]), e.v[0].max(e.v[1])));
         }
-        let mut originals: Vec<(Vec<DVec3>, DVec3)> = Vec::new();
-        // Holes of copied faces that no face fills.
-        let mut openings: Vec<Vec<DVec3>> = Vec::new();
+        let mut s = Stencil::default();
         for f in faces.iter().filter_map(|f| self.face(*f)) {
             for l in loops(f) {
                 for i in 0..l.len() {
                     let (a, b) = (l[i], l[(i + 1) % l.len()]);
-                    segments.insert((a.min(b), a.max(b)));
+                    sides.insert((a.min(b), a.max(b)));
                 }
             }
-            originals.push((f.outer.iter().map(|v| self.pos(*v)).collect(), f.normal));
+            s.faces
+                .push((f.outer.iter().map(|v| self.pos(*v)).collect(), f.normal));
             for h in &f.holes {
                 if self.face_with_outer(&h.iter().copied().collect()).is_none() {
-                    openings.push(h.iter().map(|v| self.pos(*v)).collect());
+                    s.openings.push(h.iter().map(|v| self.pos(*v)).collect());
                 }
             }
         }
-        let existing: BTreeSet<FaceId> = self.faces().map(|(id, _)| id).collect();
-        let points: Vec<(DVec3, DVec3)> = segments
+        s.segments = sides
             .iter()
             .map(|(a, b)| (self.pos(*a), self.pos(*b)))
             .collect();
-        for k in 1..=count {
-            let shift = offset * k as f64;
-            for (a, b) in &points {
-                self.draw_segment(*a + shift, *b + shift);
+        s
+    }
+
+    /// Draws `stencil` once per placement, without ending the edit. Copies are drawn like new
+    /// edges, so they merge with geometry they touch and close loops into faces; each face
+    /// keeps the orientation of its original, and openings stay open.
+    pub(crate) fn draw_stencil(&mut self, stencil: &Stencil, places: &[DAffine3]) {
+        let existing: BTreeSet<FaceId> = self.faces().map(|(id, _)| id).collect();
+        for t in places {
+            for (a, b) in &stencil.segments {
+                self.draw_segment(t.transform_point3(*a), t.transform_point3(*b));
             }
         }
-        for k in 1..=count {
-            let shift = offset * k as f64;
-            for (outer, normal) in &originals {
-                let ids: Option<BTreeSet<VertexId>> =
-                    outer.iter().map(|p| self.find_vertex(*p + shift)).collect();
+        for t in places {
+            for (outer, normal) in &stencil.faces {
+                let ids: Option<BTreeSet<VertexId>> = outer
+                    .iter()
+                    .map(|p| self.find_vertex(t.transform_point3(*p)))
+                    .collect();
                 let Some(ids) = ids else { continue };
                 if let Some(id) = self.face_with_outer(&ids) {
-                    self.orient(id, *normal);
+                    self.orient(id, t.transform_vector3(*normal).normalize());
                 }
             }
             // Drawing an opening's loop closes it into a face; take that face out again.
-            for hole in &openings {
-                let ids: Option<BTreeSet<VertexId>> =
-                    hole.iter().map(|p| self.find_vertex(*p + shift)).collect();
+            for hole in &stencil.openings {
+                let ids: Option<BTreeSet<VertexId>> = hole
+                    .iter()
+                    .map(|p| self.find_vertex(t.transform_point3(*p)))
+                    .collect();
                 let Some(ids) = ids else { continue };
                 if let Some(id) = self.face_with_outer(&ids)
                     && !existing.contains(&id)
                 {
                     self.remove_face(id);
                 }
             }
         }
-        Ok(self.take_changes())
     }
 
     /// The face whose outer loop is exactly these vertices.
     fn face_with_outer(&self, ids: &BTreeSet<VertexId>) -> Option<FaceId> {
@@ -185,8 +202,18 @@ impl Model {
             .map(|(id, _)| id)
     }
 }
 
+/// Edges and faces captured by position, to be drawn again elsewhere (copies, groups).
+#[derive(Clone, Debug, Default)]
+pub(crate) struct Stencil {
+    segments: Vec<(DVec3, DVec3)>,
+    /// Outer loop and normal of each face.
+    faces: Vec<(Vec<DVec3>, DVec3)>,
+    /// Holes of the faces that no face fills.
+    openings: Vec<Vec<DVec3>>,
+}
+
 fn loops(f: &Face) -> impl Iterator<Item = &Vec<VertexId>> {
     std::iter::once(&f.outer).chain(&f.holes)
 }
 
```

Create `crates/kernel/src/scene.rs`:
```rust
//! Groups and components: definitions with a mesh of their own, placed as instances.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{DefinitionId, EntityId};
use crate::model::{ChangeSet, ChangeSummary, Model};

/// A group has one instance, and a copy of it gets a definition of its own. A component is
/// shared by all its instances: editing it changes them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionKind {
    Group,
    Component,
}

/// Geometry that instances place in other meshes.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub name: String,
    pub kind: DefinitionKind,
    pub mesh: Model,
}

/// One of a scene's meshes: the root or a definition's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MeshId {
    Root,
    Definition(DefinitionId),
}

/// One invertible change to a scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Changes inside one mesh.
    Edit(MeshId, ChangeSet),
    AddDefinition(DefinitionId, Definition),
    RemoveDefinition(DefinitionId, Definition),
}

impl Step {
    fn inverse(&self) -> Step {
        match self.clone() {
            Step::Edit(mesh, c) => Step::Edit(mesh, c.inverse()),
            Step::AddDefinition(id, d) => Step::RemoveDefinition(id, d),
            Step::RemoveDefinition(id, d) => Step::AddDefinition(id, d),
        }
    }
}

/// The ordered steps of one edit. Applying `inverse()` undoes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneChange {
    pub steps: Vec<Step>,
}

impl SceneChange {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn inverse(&self) -> SceneChange {
        SceneChange {
            steps: self.steps.iter().rev().map(Step::inverse).collect(),
        }
    }

    /// Joins changes made one after another into one.
    pub fn join(changes: impl IntoIterator<Item = SceneChange>) -> SceneChange {
        SceneChange {
            steps: changes.into_iter().flat_map(|c| c.steps).collect(),
        }
    }

    /// Net effect per entity across all meshes (ids are unique in a scene). Definitions are not
    /// listed; the instances that place them are.
    pub fn summary(&self) -> ChangeSummary {
        let ops = self
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::Edit(_, c) => Some(c.ops.iter().cloned()),
                _ => None,
            })
            .flatten()
            .collect();
        ChangeSet { ops }.summary()
    }
}

impl From<Step> for SceneChange {
    fn from(step: Step) -> Self {
        SceneChange { steps: vec![step] }
    }
}

/// The whole model: a root mesh plus the definitions its instances place.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    root: Model,
    definitions: BTreeMap<DefinitionId, Definition>,
    /// The next id to hand out, in any mesh. Never decreases, so ids are never reused.
    next_id: u64,
}

/// Two scenes are equal when they hold the same entities and definitions under the same ids.
impl PartialEq for Scene {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.definitions == other.definitions
    }
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    /// A scene whose root mesh is `model`.
    pub fn from_root(model: Model) -> Self {
        Scene {
            next_id: model.next_id(),
            root: model,
            definitions: BTreeMap::new(),
        }
    }

    pub fn root(&self) -> &Model {
        &self.root
    }

    pub fn mesh(&self, id: MeshId) -> Option<&Model> {
        match id {
            MeshId::Root => Some(&self.root),
            MeshId::Definition(d) => self.definitions.get(&d).map(|d| &d.mesh),
        }
    }

    fn mesh_mut(&mut self, id: MeshId) -> Option<&mut Model> {
        match id {
            MeshId::Root => Some(&mut self.root),
            MeshId::Definition(d) => self.definitions.get_mut(&d).map(|d| &mut d.mesh),
        }
    }

    pub fn definition(&self, id: DefinitionId) -> Option<&Definition> {
        self.definitions.get(&id)
    }

    pub fn definitions(&self) -> impl Iterator<Item = (DefinitionId, &Definition)> {
        self.definitions.iter().map(|(k, v)| (*k, v))
    }

    /// Every mesh: the root first, then the definitions'.
    pub fn meshes(&self) -> impl Iterator<Item = (MeshId, &Model)> {
        std::iter::once((MeshId::Root, &self.root)).chain(
            self.definitions
                .iter()
                .map(|(id, d)| (MeshId::Definition(*id), &d.mesh)),
        )
    }

    /// How many instances of a definition the scene holds, in all meshes.
    pub fn instance_count(&self, id: DefinitionId) -> usize {
        self.meshes()
            .flat_map(|(_, m)| m.instances())
            .filter(|(_, i)| i.definition == id)
            .count()
    }

    /// Runs one kernel edit on a mesh and returns it as a scene change. New entities get ids
    /// unique across the scene. Panics if the mesh does not exist.
    pub fn edit(
        &mut self,
        mesh: MeshId,
        f: impl FnOnce(&mut Model) -> Result<ChangeSet, KernelError>,
    ) -> Result<SceneChange, KernelError> {
        let first = self.next_id;
        let m = self.mesh_mut(mesh).expect("edit of a missing mesh");
        m.reserve_ids(first);
        let result = f(m);
        let next = m.next_id();
        self.next_id = self.next_id.max(next);
        let changes = result?;
        Ok(if changes.is_empty() {
            SceneChange::default()
        } else {
            Step::Edit(mesh, changes).into()
        })
    }

    /// Adds an empty definition and returns its id and the step that added it.
    pub(crate) fn add_definition(
        &mut self,
        name: &str,
        kind: DefinitionKind,
    ) -> (DefinitionId, Step) {
        let id = DefinitionId(self.next_id);
        let step = Step::AddDefinition(
            id,
            Definition {
                name: name.to_string(),
                kind,
                mesh: Model::new(),
            },
        );
        self.apply_step(&step);
        (id, step)
    }

    /// Applies a change (e.g. an inverse for undo).
    pub fn apply(&mut self, change: &SceneChange) {
        for step in &change.steps {
            self.apply_step(step);
        }
    }

    fn apply_step(&mut self, step: &Step) {
        match step {
            Step::Edit(mesh, c) => {
                let m = self.mesh_mut(*mesh).expect("change to a missing mesh");
                m.apply(c);
                let next = m.next_id();
                self.next_id = self.next_id.max(next);
            }
            Step::AddDefinition(id, d) => {
                self.next_id = self.next_id.max(id.0 + 1).max(d.mesh.next_id());
                self.definitions.insert(*id, d.clone());
            }
            Step::RemoveDefinition(id, _) => {
                self.definitions.remove(id);
            }
        }
    }

    /// Checks every mesh, that instances place existing definitions, that no definition places
    /// itself (directly or deeper), and that no id is used in two meshes.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for (mesh, m) in self.meshes() {
            m.validate().map_err(|e| format!("{mesh:?}: {e}"))?;
            for (id, i) in m.instances() {
                if !self.definitions.contains_key(&i.definition) {
                    return Err(format!("instance {id:?} places a missing definition"));
                }
            }
            for id in entity_ids(m) {
                if !seen.insert(id) {
                    return Err(format!("{id} is used in two meshes"));
                }
            }
        }
        for id in self.definitions.keys() {
            if self.places(*id, *id) {
                return Err(format!("definition {id} places itself"));
            }
        }
        Ok(())
    }

    /// True when `inside`'s mesh places `target`, directly or through other definitions.
    fn places(&self, inside: DefinitionId, target: DefinitionId) -> bool {
        let mut stack = vec![inside];
        let mut visited = BTreeSet::new();
        while let Some(d) = stack.pop() {
            if !visited.insert(d) {
                continue;
            }
            for (_, i) in self.definitions[&d].mesh.instances() {
                if i.definition == target {
                    return true;
                }
                stack.push(i.definition);
            }
        }
        false
    }
}

fn entity_ids(m: &Model) -> impl Iterator<Item = EntityId> + '_ {
    m.vertices()
        .map(|(id, _)| EntityId::Vertex(id))
        .chain(m.edges().map(|(id, _)| EntityId::Edge(id)))
        .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
        .chain(m.instances().map(|(id, _)| EntityId::Instance(id)))
}
```

Create `crates/kernel/src/group.rs`:
```rust
//! Making groups and components out of geometry.

use std::collections::BTreeSet;

use glam::DAffine3;

use crate::edit::{KernelError, loop_has_side};
use crate::ids::{EdgeId, FaceId, InstanceId, VertexId};
use crate::model::{Instance, Model};
use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};

impl Scene {
    /// Moves edges, faces (with their edges) and instances of mesh `parent` into a new
    /// definition and places one instance of it, with an identity transform, where they were.
    /// Edges that still bound a face left behind stay in `parent` too. Returns the new
    /// instance.
    pub fn make_group(
        &mut self,
        parent: MeshId,
        edges: &[EdgeId],
        faces: &[FaceId],
        instances: &[InstanceId],
        kind: DefinitionKind,
        name: &str,
    ) -> Result<(InstanceId, SceneChange), KernelError> {
        let source = self.mesh(parent).expect("group in a missing mesh");
        let stencil = source.stencil(edges, faces);
        let moved: Vec<(InstanceId, Instance)> = instances
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| source.instance(*id).map(|i| (*id, i.clone())))
            .collect();

        let (definition, added) = self.add_definition(name, kind);
        let mut placed = None;
        let left = self.edit(parent, |m| {
            for (id, _) in &moved {
                m.remove_instance(*id);
            }
            m.detach(edges, faces);
            placed = Some(m.insert_instance(Instance {
                definition,
                transform: DAffine3::IDENTITY,
            }));
            Ok(m.take_changes())
        })?;
        let filled = self.edit(MeshId::Definition(definition), |m| {
            m.draw_stencil(&stencil, &[DAffine3::IDENTITY]);
            for (id, i) in moved {
                m.insert_instance_as(id, i);
            }
            Ok(m.take_changes())
        })?;
        let change = SceneChange::join([SceneChange::from(added), left, filled]);
        Ok((placed.expect("instance placed"), change))
    }
}

impl Model {
    /// Removes faces and edges without healing anything: an edge given or bounding a removed
    /// face goes unless it still bounds a remaining face, and vertices left without edges go.
    pub(crate) fn detach(&mut self, edges: &[EdgeId], faces: &[FaceId]) {
        let mut sides: BTreeSet<EdgeId> = edges
            .iter()
            .copied()
            .filter(|e| self.edge(*e).is_some())
            .collect();
        for f in faces {
            let Some(face) = self.face(*f) else { continue };
            for l in std::iter::once(&face.outer).chain(&face.holes) {
                for i in 0..l.len() {
                    sides.extend(self.edge_between(l[i], l[(i + 1) % l.len()]));
                }
            }
        }
        for f in faces {
            if self.face(*f).is_some() {
                self.remove_face(*f);
            }
        }
        let mut ends: BTreeSet<VertexId> = BTreeSet::new();
        for e in sides {
            let [a, b] = self.edge(e).expect("side exists").v;
            let bounds_a_face = self.faces().any(|(_, f)| {
                std::iter::once(&f.outer)
                    .chain(&f.holes)
                    .any(|l| loop_has_side(l, a, b))
            });
            if !bounds_a_face {
                self.remove_edge(e);
                ends.extend([a, b]);
            }
        }
        for v in ends {
            if self.edges_at(v).is_empty() {
                self.remove_vertex(v);
            }
        }
    }
}
```

Apply to `crates/kernel/src/lib.rs`:
```diff
--- a/crates/kernel/src/lib.rs
+++ b/crates/kernel/src/lib.rs
@@ -4,17 +4,20 @@
 mod edit;
 mod erase;
 mod faces;
 pub mod geom;
+mod group;
 mod ids;
 mod index;
 mod mesh;
 mod model;
 mod push_pull;
 mod regions;
+mod scene;
 mod transform;
 mod weld;
 
 pub use edit::KernelError;
-pub use ids::{EdgeId, EntityId, FaceId, VertexId};
+pub use ids::{DefinitionId, EdgeId, EntityId, FaceId, InstanceId, VertexId};
 pub use index::{Index, Shape};
-pub use model::{ChangeSet, ChangeSummary, Edge, Face, Model, Op, Vertex};
+pub use model::{ChangeSet, ChangeSummary, Edge, Face, Instance, Model, Op, Vertex};
+pub use scene::{Definition, DefinitionKind, MeshId, Scene, SceneChange, Step};
```

Apply to `crates/session/src/document.rs`:
```diff
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -397,8 +397,9 @@ impl Document {
             match *id {
                 EntityId::Vertex(v) => {
                     out.insert(v);
                 }
+                EntityId::Instance(_) => {}
                 EntityId::Edge(e) => out.extend(m.edge(e).map(|e| e.v).into_iter().flatten()),
                 EntityId::Face(f) => {
                     if let Some(f) = m.face(f) {
                         out.extend(f.outer.iter().chain(f.holes.iter().flatten()));
@@ -474,8 +475,9 @@ impl Document {
         match id {
             EntityId::Vertex(v) => self.model.vertex(v).is_some(),
             EntityId::Edge(e) => self.model.edge(e).is_some(),
             EntityId::Face(f) => self.model.face(f).is_some(),
+            EntityId::Instance(i) => self.model.instance(i).is_some(),
         }
     }
 
     fn record(&mut self, changes: ChangeSet) -> Outcome {
@@ -553,8 +555,9 @@ impl Document {
                     }
                     EntityId::Face(f) => {
                         QueryResult::Face(m.face(f).map(|_| self.face_info(f)).ok_or_else(missing)?)
                     }
+                    EntityId::Instance(_) => return Err(missing()),
                 }
             }
             Query::Camera => QueryResult::Camera {
                 eye: self.camera.eye.to_array(),
@@ -591,8 +594,9 @@ impl Document {
     fn vertices_of(&self, id: EntityId) -> impl Iterator<Item = [f64; 3]> + '_ {
         let m = &self.model;
         let ids: Vec<VertexId> = match id {
             EntityId::Vertex(v) => vec![v],
+            EntityId::Instance(_) => vec![],
             EntityId::Edge(e) => m.edge(e).map(|e| e.v.to_vec()).unwrap_or_default(),
             EntityId::Face(f) => m
                 .face(f)
                 .map(|f| {
```

Apply to `crates/app/src/editor.rs`:
```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -1016,9 +1016,9 @@ mod tests {
                     .v
                     .iter()
                     .all(|v| model.position(*v).unwrap().x > 3.5),
                 EntityId::Face(f) => model.face(f).unwrap().outer.len() == 4,
-                EntityId::Vertex(_) => false,
+                EntityId::Vertex(_) | EntityId::Instance(_) => false,
             }
         }));
         // Typing another array must not undo anything.
         type_text(&mut e, "3x");
```

Apply to `crates/app/src/info.rs`:
```diff
--- a/crates/app/src/info.rs
+++ b/crates/app/src/info.rs
@@ -11,9 +11,9 @@ pub fn entity_info(doc: &Document) -> Vec<String> {
     for id in doc.selection() {
         match id {
             EntityId::Edge(e) => edges.push(*e),
             EntityId::Face(f) => faces.push(*f),
-            EntityId::Vertex(_) => {}
+            EntityId::Vertex(_) | EntityId::Instance(_) => {}
         }
     }
     match (&edges[..], &faces[..]) {
         ([], []) => vec!["Nothing selected".into()],
```

Apply to `crates/tools/src/erase.rs`:
```diff
--- a/crates/tools/src/erase.rs
+++ b/crates/tools/src/erase.rs
@@ -27,9 +27,9 @@ fn outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
             .and_then(|e| Some([m.position(e.v[0])?, m.position(e.v[1])?]))
             .into_iter()
             .collect(),
         EntityId::Face(f) => m.face(f).map(|f| face_outline(m, f)).unwrap_or_default(),
-        EntityId::Vertex(_) => Vec::new(),
+        EntityId::Vertex(_) | EntityId::Instance(_) => Vec::new(),
     }
 }
 
 impl Tool for EraseTool {
```

Apply to `crates/tools/src/move_tool.rs`:
```diff
--- a/crates/tools/src/move_tool.rs
+++ b/crates/tools/src/move_tool.rs
@@ -93,9 +93,9 @@ fn outline(ctx: &ToolContext, ids: &[EntityId]) -> Vec<[DVec3; 2]> {
                 if let Some(f) = m.face(f) {
                     out.extend(face_outline(m, f));
                 }
             }
-            EntityId::Vertex(_) => {}
+            EntityId::Vertex(_) | EntityId::Instance(_) => {}
         }
     }
     out
 }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kernel --test groups && cargo test -p kernel --lib ids`
Expected: 10 passed (groups), ids tests pass.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 396 tests pass. Every existing copy test still passes: `copy` now goes through `Stencil` with identical results.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/kernel/tests/groups.rs crates/kernel/src/ids.rs crates/kernel/src/model.rs crates/kernel/src/transform.rs crates/kernel/src/scene.rs crates/kernel/src/group.rs crates/kernel/src/lib.rs crates/session/src/document.rs crates/app/src/editor.rs crates/app/src/info.rs crates/tools/src/erase.rs crates/tools/src/move_tool.rs
git commit -m "Add instances, scenes and make_group to the kernel"
```

### Task 2: Explode, copy and erase instances

**Files:**
- Modify: `crates/kernel/src/group.rs`, `crates/kernel/src/scene.rs`, `crates/kernel/src/transform.rs`, `crates/kernel/src/edit.rs`
- Test: `crates/kernel/tests/groups.rs`, `crates/kernel/tests/transform.rs`

**Interfaces:**
- Consumes (Task 1):
  - `Scene::{edit, add_definition}`;
  - `Model::{stencil, draw_stencil, insert_instance, remove_instance}`;
  - `Model::rebuild_plane` (faces.rs);
  - `Plane::through` (geom.rs).
- Produces:
  - `Scene::explode(&mut self, MeshId, InstanceId) -> Result<SceneChange, KernelError>`;
  - `Scene::copy_instances(&mut self, MeshId, &[InstanceId], offset: DVec3, count: u32) -> Result<SceneChange, KernelError>`;
  - `Scene::erase_instances(&mut self, MeshId, &[InstanceId]) -> SceneChange`;
  - `Scene::resolve(&self, &[InstanceId]) -> Option<(MeshId, DAffine3)>`;
  - `Scene::bounds(&self, MeshId) -> Option<[DVec3; 2]>`;
  - `Scene::instance_bounds(&self, &Instance) -> Option<[DVec3; 2]>`;
  - `KernelError::InstanceNotFound`.
  - `draw_stencil` now also closes a face whose sides all existed already.

- [ ] **Step 1: Write the failing tests**

Apply to `crates/kernel/tests/groups.rs`:
```diff
--- a/crates/kernel/tests/groups.rs
+++ b/crates/kernel/tests/groups.rs
@@ -306,4 +306,218 @@ fn moving_an_instance_translates_it_and_undoes_exactly() {
     assert_eq!(change.summary().modified, vec![EntityId::Instance(i)]);
     s.apply(&change.inverse());
     assert_eq!(s, before);
 }
+
+fn root_counts(s: &Scene) -> (usize, usize, usize, usize) {
+    counts(s.root())
+}
+
+#[test]
+fn exploding_a_group_puts_its_geometry_back() {
+    let mut s = box_scene(DVec3::ZERO);
+    let original = root_counts(&s);
+    let (i, _) = group_all(&mut s, DefinitionKind::Group);
+    let d = definition_of(&s, i);
+    let before = s.clone();
+    let change = s.explode(MeshId::Root, i).unwrap();
+    s.validate().unwrap();
+    assert_eq!(root_counts(&s), original);
+    assert!(s.definition(d).is_none(), "the group's definition goes");
+    assert!(change.summary().deleted.contains(&EntityId::Instance(i)));
+    s.apply(&change.inverse());
+    assert_eq!(s, before);
+}
+
+#[test]
+fn exploding_a_moved_instance_places_the_geometry_where_it_was_shown() {
+    let mut s = box_scene(DVec3::ZERO);
+    let (i, _) = group_all(&mut s, DefinitionKind::Group);
+    s.edit(MeshId::Root, |m| {
+        Ok(m.move_instances(&[i], p(10.0, 0.0, 0.0)))
+    })
+    .unwrap();
+    assert_eq!(
+        s.bounds(MeshId::Root),
+        Some([p(10.0, 0.0, 0.0), p(12.0, 3.0, 1.0)])
+    );
+    s.explode(MeshId::Root, i).unwrap();
+    s.validate().unwrap();
+    assert_eq!(
+        s.bounds(MeshId::Root),
+        Some([p(10.0, 0.0, 0.0), p(12.0, 3.0, 1.0)])
+    );
+}
+
+#[test]
+fn an_exploded_face_merges_with_what_it_touches() {
+    let mut s = box_scene(DVec3::ZERO);
+    let top = s
+        .root()
+        .faces()
+        .find(|(_, f)| f.normal.z > 0.5)
+        .map(|(id, _)| id)
+        .unwrap();
+    let (i, _) = s
+        .make_group(MeshId::Root, &[], &[top], &[], DefinitionKind::Group, "Top")
+        .unwrap();
+    s.explode(MeshId::Root, i).unwrap();
+    s.validate().unwrap();
+    assert_eq!(root_counts(&s), (8, 12, 6, 0));
+    let up = s.root().faces().filter(|(_, f)| f.normal.z > 0.5).count();
+    assert_eq!(up, 1);
+}
+
+#[test]
+fn a_component_definition_outlives_its_exploded_instances() {
+    let mut s = box_scene(DVec3::ZERO);
+    let (i, _) = group_all(&mut s, DefinitionKind::Component);
+    let d = definition_of(&s, i);
+    s.explode(MeshId::Root, i).unwrap();
+    s.validate().unwrap();
+    assert!(s.definition(d).is_some());
+    assert_eq!(s.instance_count(d), 0);
+}
+
+#[test]
+fn exploding_a_nested_group_lifts_the_inner_instance() {
+    let mut s = box_scene(DVec3::ZERO);
+    let (inner, _) = group_all(&mut s, DefinitionKind::Component);
+    s.edit(MeshId::Root, |m| {
+        Ok(m.move_instances(&[inner], p(0.0, 5.0, 0.0)))
+    })
+    .unwrap();
+    let (outer, _) = s
+        .make_group(
+            MeshId::Root,
+            &[],
+            &[],
+            &[inner],
+            DefinitionKind::Group,
+            "Outer",
+        )
+        .unwrap();
+    s.edit(MeshId::Root, |m| {
+        Ok(m.move_instances(&[outer], p(1.0, 0.0, 0.0)))
+    })
+    .unwrap();
+    s.explode(MeshId::Root, outer).unwrap();
+    s.validate().unwrap();
+    let lifted: Vec<_> = s.root().instances().collect();
+    assert_eq!(lifted.len(), 1);
+    assert_ne!(lifted[0].0, inner, "lifted instances get new ids");
+    assert_eq!(
+        lifted[0].1.transform,
+        DAffine3::from_translation(p(1.0, 5.0, 0.0))
+    );
+}
+
+#[test]
+fn copies_of_a_component_share_its_definition() {
+    let mut s = box_scene(DVec3::ZERO);
+    let (i, _) = group_all(&mut s, DefinitionKind::Component);
+    let d = definition_of(&s, i);
+    let change = s
+        .copy_instances(MeshId::Root, &[i], p(3.0, 0.0, 0.0), 3)
+        .unwrap();
+    s.validate().unwrap();
+    assert_eq!(s.instance_count(d), 4);
+    assert_eq!(s.definitions().count(), 1);
+    assert_eq!(change.summary().created.len(), 3);
+    assert_eq!(
+        s.bounds(MeshId::Root),
+        Some([p(0.0, 0.0, 0.0), p(11.0, 3.0, 1.0)])
+    );
+}
+
+#[test]
+fn a_copy_of_a_group_gets_its_own_definition() {
+    let mut s = box_scene(DVec3::ZERO);
+    let (inner, _) = group_all(&mut s, DefinitionKind::Group);
+    let (outer, _) = s
+        .make_group(
+            MeshId::Root,
+            &[],
+            &[],
+            &[inner],
+            DefinitionKind::Group,
+            "Outer",
+        )
+        .unwrap();
+    let before = s.clone();
+    let change = s
+        .copy_instances(MeshId::Root, &[outer], p(0.0, 4.0, 0.0), 1)
+        .unwrap();
+    s.validate().unwrap();
+    assert_eq!(s.definitions().count(), 4, "outer and inner both copied");
+    let copy = s
+        .root()
+        .instances()
+        .find(|(id, _)| *id != outer)
+        .map(|(_, i)| i.definition)
+        .unwrap();
+    assert_ne!(copy, definition_of(&s, outer));
+    assert_eq!(s.instance_count(copy), 1);
+    s.apply(&change.inverse());
+    assert_eq!(s, before);
+}
+
+#[test]
+fn erasing_a_group_removes_its_definition_and_undoes_exactly() {
+    let mut s = box_scene(DVec3::ZERO);
+    let (i, _) = group_all(&mut s, DefinitionKind::Group);
+    let d = definition_of(&s, i);
+    let before = s.clone();
+    let change = s.erase_instances(MeshId::Root, &[i]);
+    s.validate().unwrap();
+    assert_eq!(root_counts(&s), (0, 0, 0, 0));
+    assert!(s.definition(d).is_none());
+    assert_eq!(change.summary().deleted, vec![EntityId::Instance(i)]);
+    s.apply(&change.inverse());
+    assert_eq!(s, before);
+}
+
+#[test]
+fn erasing_a_component_instance_keeps_the_definition() {
+    let mut s = box_scene(DVec3::ZERO);
+    let (i, _) = group_all(&mut s, DefinitionKind::Component);
+    let d = definition_of(&s, i);
+    s.erase_instances(MeshId::Root, &[i]);
+    assert!(s.definition(d).is_some());
+}
+
+#[test]
+fn paths_resolve_to_the_open_mesh_and_its_placement() {
+    let mut s = box_scene(DVec3::ZERO);
+    let (inner, _) = group_all(&mut s, DefinitionKind::Group);
+    let (outer, _) = s
+        .make_group(
+            MeshId::Root,
+            &[],
+            &[],
+            &[inner],
+            DefinitionKind::Group,
+            "Outer",
+        )
+        .unwrap();
+    s.edit(MeshId::Root, |m| {
+        Ok(m.move_instances(&[outer], p(1.0, 2.0, 3.0)))
+    })
+    .unwrap();
+    let d_outer = definition_of(&s, outer);
+    let d_inner = s
+        .definition(d_outer)
+        .unwrap()
+        .mesh
+        .instance(inner)
+        .unwrap()
+        .definition;
+    assert_eq!(s.resolve(&[]), Some((MeshId::Root, DAffine3::IDENTITY)));
+    assert_eq!(
+        s.resolve(&[outer, inner]),
+        Some((
+            MeshId::Definition(d_inner),
+            DAffine3::from_translation(p(1.0, 2.0, 3.0))
+        ))
+    );
+    assert_eq!(s.resolve(&[inner]), None, "inner is not in the root");
+}
```

Apply to `crates/kernel/tests/transform.rs`:
```diff
--- a/crates/kernel/tests/transform.rs
+++ b/crates/kernel/tests/transform.rs
@@ -254,4 +254,24 @@ fn copying_a_face_with_a_filled_hole_does_not_remove_faces() {
     m.validate().unwrap();
     // The hole copies close into faces, as the original's hole is filled.
     assert_eq!(m.faces().count(), 6);
 }
+
+#[test]
+fn a_face_copied_onto_an_empty_outline_fills_it() {
+    let mut m = cube();
+    let top = face_with_normal(&m, DVec3::Z);
+    m.erase(&[], &[top]);
+    let bottom = face_with_normal(&m, -DVec3::Z);
+    m.copy(&[], &[bottom], p(0.0, 0.0, 2.0), 1).unwrap();
+    m.validate().unwrap();
+    assert_eq!(counts(&m), (8, 12, 6));
+    let lid = m
+        .faces()
+        .find(|(_, f)| f.outer.iter().all(|v| m.position(*v).unwrap().z > 1.0))
+        .map(|(_, f)| f.normal);
+    assert_eq!(
+        lid,
+        Some(-DVec3::Z),
+        "the copy keeps the bottom's orientation"
+    );
+}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kernel --test groups`
Expected: compile errors: no method named `explode`, `copy_instances`, `erase_instances`, `bounds`, `resolve` on `Scene`.

Run: `cargo test -p kernel --test transform a_face_copied_onto_an_empty_outline_fills_it`
Expected: FAIL, `left: (8, 12, 5)` / `right: (8, 12, 6)`.

- [ ] **Step 3: Implement**

Apply to `crates/kernel/src/edit.rs`:
```diff
--- a/crates/kernel/src/edit.rs
+++ b/crates/kernel/src/edit.rs
@@ -20,8 +20,9 @@ pub enum KernelError {
     Bend(FaceId),
     /// A move would shrink an edge to nothing, turn a face inside out, or land a vertex on
     /// another.
     Squash,
+    InstanceNotFound,
 }
 
 impl fmt::Display for KernelError {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
@@ -45,8 +46,9 @@ impl fmt::Display for KernelError {
             KernelError::Squash => write!(
                 f,
                 "the move would collapse an edge, turn a face over, or land a vertex on another"
             ),
+            KernelError::InstanceNotFound => write!(f, "no such instance"),
         }
     }
 }
 
```

Apply to `crates/kernel/src/scene.rs`:
```diff
--- a/crates/kernel/src/scene.rs
+++ b/crates/kernel/src/scene.rs
@@ -1,13 +1,14 @@
 //! Groups and components: definitions with a mesh of their own, placed as instances.
 
 use std::collections::{BTreeMap, BTreeSet};
 
+use glam::{DAffine3, DVec3};
 use serde::{Deserialize, Serialize};
 
 use crate::edit::KernelError;
-use crate::ids::{DefinitionId, EntityId};
-use crate::model::{ChangeSet, ChangeSummary, Model};
+use crate::ids::{DefinitionId, EntityId, InstanceId};
+use crate::model::{ChangeSet, ChangeSummary, Instance, Model};
 
 /// A group has one instance, and a copy of it gets a definition of its own. A component is
 /// shared by all its instances: editing it changes them all.
 #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
@@ -209,8 +210,58 @@ impl Scene {
         self.apply_step(&step);
         (id, step)
     }
 
+    /// Removes a definition and returns the step that removed it.
+    pub(crate) fn remove_definition(&mut self, id: DefinitionId) -> Step {
+        let step = Step::RemoveDefinition(id, self.definitions[&id].clone());
+        self.apply_step(&step);
+        step
+    }
+
+    /// Follows a path of instances from the root, each one in the mesh of the one before.
+    /// Returns the mesh the last one places and the transform from that mesh's coordinates to
+    /// the root's (the root itself for an empty path). None if a step is not where it should
+    /// be.
+    pub fn resolve(&self, path: &[InstanceId]) -> Option<(MeshId, DAffine3)> {
+        let mut mesh = MeshId::Root;
+        let mut transform = DAffine3::IDENTITY;
+        for id in path {
+            let i = self.mesh(mesh)?.instance(*id)?;
+            transform *= i.transform;
+            mesh = MeshId::Definition(i.definition);
+        }
+        Some((mesh, transform))
+    }
+
+    /// The box around a mesh's vertices and instances, in the mesh's coordinates. None when
+    /// the mesh is empty or missing.
+    pub fn bounds(&self, mesh: MeshId) -> Option<[DVec3; 2]> {
+        let m = self.mesh(mesh)?;
+        let mut points: Vec<DVec3> = m.vertices().map(|(_, v)| v.pos).collect();
+        for (_, i) in m.instances() {
+            points.extend(self.instance_bounds(i).into_iter().flatten());
+        }
+        enclose(&points)
+    }
+
+    /// The box around an instance's geometry, in the coordinates of the mesh holding it. None
+    /// when its definition is empty or missing.
+    pub fn instance_bounds(&self, instance: &Instance) -> Option<[DVec3; 2]> {
+        let [lo, hi] = self.bounds(MeshId::Definition(instance.definition))?;
+        let corners: Vec<DVec3> = (0..8)
+            .map(|k| {
+                let corner = DVec3::new(
+                    if k & 1 == 0 { lo.x } else { hi.x },
+                    if k & 2 == 0 { lo.y } else { hi.y },
+                    if k & 4 == 0 { lo.z } else { hi.z },
+                );
+                instance.transform.transform_point3(corner)
+            })
+            .collect();
+        enclose(&corners)
+    }
+
     /// Applies a change (e.g. an inverse for undo).
     pub fn apply(&mut self, change: &SceneChange) {
         for step in &change.steps {
             self.apply_step(step);
@@ -278,8 +329,18 @@ impl Scene {
         false
     }
 }
 
+/// The smallest box holding the points; None when there are none.
+fn enclose(points: &[DVec3]) -> Option<[DVec3; 2]> {
+    let first = *points.first()?;
+    Some(
+        points
+            .iter()
+            .fold([first, first], |[lo, hi], p| [lo.min(*p), hi.max(*p)]),
+    )
+}
+
 fn entity_ids(m: &Model) -> impl Iterator<Item = EntityId> + '_ {
     m.vertices()
         .map(|(id, _)| EntityId::Vertex(id))
         .chain(m.edges().map(|(id, _)| EntityId::Edge(id)))
```

Apply to `crates/kernel/src/group.rs`:
```diff
--- a/crates/kernel/src/group.rs
+++ b/crates/kernel/src/group.rs
@@ -1,14 +1,16 @@
 //! Making groups and components out of geometry.
 
 use std::collections::BTreeSet;
 
-use glam::DAffine3;
+use glam::{DAffine3, DVec3};
 
 use crate::edit::{KernelError, loop_has_side};
-use crate::ids::{EdgeId, FaceId, InstanceId, VertexId};
+use crate::geom::LENGTH_TOL;
+use crate::ids::{DefinitionId, EdgeId, FaceId, InstanceId, VertexId};
 use crate::model::{Instance, Model};
 use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};
+use crate::transform::Stencil;
 
 impl Scene {
     /// Moves edges, faces (with their edges) and instances of mesh `parent` into a new
     /// definition and places one instance of it, with an identity transform, where they were.
@@ -56,8 +58,168 @@ impl Scene {
         Ok((placed.expect("instance placed"), change))
     }
 }
 
+impl Scene {
+    /// Replaces an instance in mesh `parent` by its definition's geometry, placed by the
+    /// instance's transform and drawn like new edges, so it merges with what it touches.
+    /// Instances inside it become instances in `parent`, with new ids. A group's definition
+    /// goes with its last instance; a component's stays.
+    pub fn explode(
+        &mut self,
+        parent: MeshId,
+        instance: InstanceId,
+    ) -> Result<SceneChange, KernelError> {
+        let placed = self
+            .mesh(parent)
+            .and_then(|m| m.instance(instance))
+            .cloned()
+            .ok_or(KernelError::InstanceNotFound)?;
+        let mesh = &self
+            .definition(placed.definition)
+            .expect("instance of a missing definition")
+            .mesh;
+        let stencil = whole(mesh);
+        let children: Vec<Instance> = mesh
+            .instances()
+            .map(|(_, i)| Instance {
+                definition: i.definition,
+                transform: placed.transform * i.transform,
+            })
+            .collect();
+        let exploded = self.edit(parent, |m| {
+            m.remove_instance(instance);
+            m.draw_stencil(&stencil, &[placed.transform]);
+            for child in children {
+                m.insert_instance(child);
+            }
+            Ok(m.take_changes())
+        })?;
+        Ok(SceneChange::join([exploded, self.prune()]))
+    }
+
+    /// Places `count` copies of instances of mesh `parent`, copy k shifted by `k * offset`.
+    /// Copies of a component instance place the same definition; each copy of a group gets a
+    /// copy of the group's definition (and of the groups inside it).
+    pub fn copy_instances(
+        &mut self,
+        parent: MeshId,
+        instances: &[InstanceId],
+        offset: DVec3,
+        count: u32,
+    ) -> Result<SceneChange, KernelError> {
+        if !offset.is_finite() {
+            return Err(KernelError::NotFinite);
+        }
+        if offset.length() < LENGTH_TOL {
+            return Err(KernelError::ZeroDistance);
+        }
+        let source = self.mesh(parent).expect("copy in a missing mesh");
+        let originals: Vec<Instance> = instances
+            .iter()
+            .map(|id| source.instance(*id).cloned())
+            .collect::<Option<_>>()
+            .ok_or(KernelError::InstanceNotFound)?;
+        let mut steps = SceneChange::default();
+        let mut copies = Vec::new();
+        for k in 1..=count {
+            for original in &originals {
+                let definition = match self.definition(original.definition).map(|d| d.kind) {
+                    Some(DefinitionKind::Group) => {
+                        let (d, made) = self.duplicate(original.definition);
+                        steps = SceneChange::join([steps, made]);
+                        d
+                    }
+                    _ => original.definition,
+                };
+                copies.push(Instance {
+                    definition,
+                    transform: DAffine3::from_translation(offset * k as f64) * original.transform,
+                });
+            }
+        }
+        let placed = self.edit(parent, |m| {
+            for c in copies {
+                m.insert_instance(c);
+            }
+            Ok(m.take_changes())
+        })?;
+        Ok(SceneChange::join([steps, placed]))
+    }
+
+    /// A new group definition holding a copy of `id`'s geometry, with new ids; groups inside it
+    /// are copied too, components inside are placed again.
+    fn duplicate(&mut self, id: DefinitionId) -> (DefinitionId, SceneChange) {
+        let original = self.definition(id).expect("copy of a missing definition");
+        let (name, kind) = (original.name.clone(), original.kind);
+        let stencil = whole(&original.mesh);
+        let children: Vec<Instance> = original.mesh.instances().map(|(_, i)| i.clone()).collect();
+        let mut change = SceneChange::default();
+        let mut placed = Vec::new();
+        for child in children {
+            let definition = match self.definition(child.definition).map(|d| d.kind) {
+                Some(DefinitionKind::Group) => {
+                    let (d, made) = self.duplicate(child.definition);
+                    change = SceneChange::join([change, made]);
+                    d
+                }
+                _ => child.definition,
+            };
+            placed.push(Instance {
+                definition,
+                transform: child.transform,
+            });
+        }
+        let (copy, added) = self.add_definition(&name, kind);
+        let filled = self
+            .edit(MeshId::Definition(copy), |m| {
+                m.draw_stencil(&stencil, &[DAffine3::IDENTITY]);
+                for i in placed {
+                    m.insert_instance(i);
+                }
+                Ok(m.take_changes())
+            })
+            .expect("drawing a copy cannot fail");
+        (copy, SceneChange::join([change, added.into(), filled]))
+    }
+
+    /// Removes instances from mesh `parent`. A group's definition goes with its last instance.
+    pub fn erase_instances(&mut self, parent: MeshId, instances: &[InstanceId]) -> SceneChange {
+        let erased = self
+            .edit(parent, |m| {
+                for id in instances {
+                    if m.instance(*id).is_some() {
+                        m.remove_instance(*id);
+                    }
+                }
+                Ok(m.take_changes())
+            })
+            .expect("erasing cannot fail");
+        SceneChange::join([erased, self.prune()])
+    }
+
+    /// Removes group definitions that no instance places any more.
+    fn prune(&mut self) -> SceneChange {
+        let mut steps = Vec::new();
+        loop {
+            let unused = self
+                .definitions()
+                .find(|(id, d)| d.kind == DefinitionKind::Group && self.instance_count(*id) == 0)
+                .map(|(id, _)| id);
+            let Some(id) = unused else { break };
+            steps.push(self.remove_definition(id));
+        }
+        SceneChange { steps }
+    }
+}
+
+/// Every edge and face of a mesh, as a stencil.
+fn whole(m: &Model) -> Stencil {
+    let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
+    let faces: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
+    m.stencil(&edges, &faces)
+}
+
 impl Model {
     /// Removes faces and edges without healing anything: an edge given or bounding a removed
     /// face goes unless it still bounds a remaining face, and vertices left without edges go.
     pub(crate) fn detach(&mut self, edges: &[EdgeId], faces: &[FaceId]) {
```

Apply to `crates/kernel/src/transform.rs`:
```diff
--- a/crates/kernel/src/transform.rs
+++ b/crates/kernel/src/transform.rs
@@ -158,10 +158,11 @@ impl Model {
         s
     }
 
     /// Draws `stencil` once per placement, without ending the edit. Copies are drawn like new
-    /// edges, so they merge with geometry they touch and close loops into faces; each face
-    /// keeps the orientation of its original, and openings stay open.
+    /// edges, so they merge with geometry they touch and close loops into faces (also where
+    /// every side of a face was there already); each face keeps the orientation of its
+    /// original, and openings stay open.
     pub(crate) fn draw_stencil(&mut self, stencil: &Stencil, places: &[DAffine3]) {
         let existing: BTreeSet<FaceId> = self.faces().map(|(id, _)| id).collect();
         for t in places {
             for (a, b) in &stencil.segments {
@@ -169,15 +170,27 @@ impl Model {
             }
         }
         for t in places {
             for (outer, normal) in &stencil.faces {
-                let ids: Option<BTreeSet<VertexId>> = outer
+                let normal = t.transform_vector3(*normal).normalize();
+                let ring: Option<Vec<VertexId>> = outer
                     .iter()
                     .map(|p| self.find_vertex(t.transform_point3(*p)))
                     .collect();
-                let Some(ids) = ids else { continue };
+                let Some(ring) = ring else { continue };
+                let ids: BTreeSet<VertexId> = ring.iter().copied().collect();
+                if self.face_with_outer(&ids).is_none() {
+                    // Every side was there already, so drawing them closed no face: close it.
+                    let n = ring.len();
+                    let sides: Option<Vec<EdgeId>> = (0..n)
+                        .map(|i| self.edge_between(ring[i], ring[(i + 1) % n]))
+                        .collect();
+                    if let Some(sides) = sides {
+                        self.rebuild_plane(Plane::through(self.pos(ring[0]), normal), &sides);
+                    }
+                }
                 if let Some(id) = self.face_with_outer(&ids) {
-                    self.orient(id, t.transform_vector3(*normal).normalize());
+                    self.orient(id, normal);
                 }
             }
             // Drawing an opening's loop closes it into a face; take that face out again.
             for hole in &stencil.openings {
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kernel --test groups --test transform`
Expected: 20 + 14 passed.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 407 tests pass.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/kernel/tests/groups.rs crates/kernel/tests/transform.rs crates/kernel/src/edit.rs crates/kernel/src/scene.rs crates/kernel/src/group.rs crates/kernel/src/transform.rs
git commit -m "Explode, copy and erase instances"
```

### Task 3: Group, component and context commands in the session

**Files:**
- Create: `crates/session/src/document/groups.rs`
- Modify: `crates/session/src/command.rs`, `crates/session/src/query.rs`, `crates/session/src/document.rs`, `crates/session/src/lib.rs`
- Test: `crates/session/tests/groups.rs` (new), `crates/session/tests/commands.rs` (stats now has `instances` and `definitions`)

**Interfaces:**
- Consumes (Tasks 1-2): `Scene` and its operations; `SceneChange`; `MeshId`; `Instance`.
- Produces:
  - **commands:** `Command::{MakeGroup { ids }, MakeComponent { ids, name }, Explode { ids }, OpenInstance { id }, CloseInstance}`.
  - **queries:** `Query::{Definitions, Context}`; `EntityKind::Instance`.
  - **result types:**
    - `QueryResult::{Instance(InstanceInfo), Definitions { definitions, revision }, Context { path, definition }}`;
    - `QueryResult::Entities` gains `instances`;
    - `QueryResult::Stats` gains `instances`, `definitions`;
    - `session::{InstanceInfo, DefinitionInfo}`.
  - **Document:**
    - `scene(&self) -> &Scene`;
    - `context(&self) -> &[InstanceId]`;
    - `context_transform(&self) -> DAffine3`;
    - `model()` now returns the open context's mesh.
- `mod groups;` is declared inside `document.rs`, so the file lives at `document/groups.rs` and can use `Document`'s private fields and methods.

- [ ] **Step 1: Write the failing tests**

Create `crates/session/tests/groups.rs`:
```rust
//! Groups, components and the open context, driven through JSON commands.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, Vec<String>)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.ids.iter().map(|i| i.to_string()).collect()))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

/// Vertices, edges, faces and instances in the open context, and definitions in the model.
fn counts(d: &Document) -> [u64; 5] {
    let s = ask(d, json!({"type": "stats"}));
    ["vertices", "edges", "faces", "instances", "definitions"].map(|k| s[k].as_u64().unwrap())
}

/// A 2 x 2 x 1 box on an empty model.
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

/// Every edge and face id in the open context.
fn everything(d: &Document) -> Vec<Value> {
    let all = ask(d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect()
}

/// The instance id among a command's created ids.
fn created_instance(out: &Value) -> String {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .find(|id| id.starts_with('i'))
        .expect("an instance was created")
}

/// Groups the whole box and returns the instance id.
fn grouped_cube(d: &mut Document, command: Value) -> String {
    cube(d);
    let mut command = command;
    command["ids"] = Value::Array(everything(d));
    let out = run(d, command).unwrap();
    created_instance(&out)
}

/// The id of the face whose normal is +Z.
fn top_face(d: &Document) -> String {
    let faces = ask(d, json!({"type": "entities", "kinds": ["face"]}));
    faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"] == json!([0.0, 0.0, 1.0]))
        .map(|f| f["id"].as_str().unwrap().to_string())
        .unwrap()
}

#[test]
fn making_a_group_is_one_undo_step_and_selects_it() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    assert_eq!(counts(&d), [0, 0, 0, 1, 1]);
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([i]));
    let info = ask(&d, json!({"type": "entity", "id": i}));
    assert_eq!(info["kind"], "group");
    assert_eq!(
        info["bounds"],
        json!({"min": [0.0, 0.0, 0.0], "max": [2.0, 2.0, 1.0]})
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), [8, 12, 6, 0, 0]);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 1, 1]);
}

#[test]
fn an_open_instance_is_edited_in_its_own_coordinates() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(
        &mut d,
        json!({"type": "move", "ids": [i], "offset": [10, 0, 0]}),
    )
    .unwrap();
    let moved = ask(&d, json!({"type": "entity", "id": i}));
    assert_eq!(moved["bounds"]["min"], json!([10.0, 0.0, 0.0]));

    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    let context = ask(&d, json!({"type": "context"}));
    assert_eq!(context["path"], json!([i]));
    assert_eq!(context["definition"], moved["definition"]);
    assert_eq!(counts(&d), [8, 12, 6, 0, 1]);
    let inside = ask(&d, json!({"type": "entities", "kinds": ["vertex"]}));
    let xs: Vec<f64> = inside["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["position"][0].as_f64().unwrap())
        .collect();
    assert!(xs.iter().all(|x| (0.0..=2.0).contains(x)), "{xs:?}");

    let top = top_face(&d);
    run(
        &mut d,
        json!({"type": "push_pull", "face": top, "distance": 1.0}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "context"}))["path"], json!([]));
    let grown = ask(&d, json!({"type": "entity", "id": i}));
    assert_eq!(
        grown["bounds"],
        json!({"min": [10.0, 0.0, 0.0], "max": [12.0, 2.0, 2.0]})
    );
}

#[test]
fn editing_a_component_changes_every_instance() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_component", "name": "Crate"}));
    run(
        &mut d,
        json!({"type": "copy", "ids": [i], "offset": [3, 0, 0], "count": 2}),
    )
    .unwrap();
    let defs = ask(&d, json!({"type": "definitions"}));
    assert_eq!(defs["definitions"].as_array().unwrap().len(), 1);
    assert_eq!(defs["definitions"][0]["name"], "Crate");
    assert_eq!(defs["definitions"][0]["instances"], 3);

    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    let top = top_face(&d);
    run(
        &mut d,
        json!({"type": "push_pull", "face": top, "distance": 1.0}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let all = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    let tops: Vec<f64> = all["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["bounds"]["max"][2].as_f64().unwrap())
        .collect();
    assert_eq!(tops, [2.0, 2.0, 2.0]);
}

#[test]
fn a_copied_group_is_independent() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(
        &mut d,
        json!({"type": "copy", "ids": [i], "offset": [3, 0, 0]}),
    )
    .unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 2, 2]);
}

#[test]
fn erasing_a_group_removes_its_definition_until_undone() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(&mut d, json!({"type": "erase", "ids": [i]})).unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 0, 0]);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 1, 1]);
}

#[test]
fn exploding_selects_what_comes_out() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(&mut d, json!({"type": "explode", "ids": [i]})).unwrap();
    assert_eq!(counts(&d), [8, 12, 6, 0, 0]);
    let selected = ask(&d, json!({"type": "selection"}));
    assert_eq!(selected["ids"].as_array().unwrap().len(), 12 + 6);
}

#[test]
fn undoing_the_group_while_inside_it_closes_it() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "context"}))["path"], json!([]));
    assert_eq!(counts(&d), [8, 12, 6, 0, 0]);
}

#[test]
fn a_failed_atomic_batch_restores_the_open_context() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "open_instance", "id": i},
        {"type": "push_pull", "face": "f999", "distance": 1.0}
    ]))
    .unwrap();
    let err = d.execute_batch(commands, true).unwrap_err();
    assert_eq!(err.index, 1);
    assert_eq!(ask(&d, json!({"type": "context"}))["path"], json!([]));
}

#[test]
fn group_commands_reject_bad_ids() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    let invalid = |r: Result<Value, (ErrorKind, Vec<String>)>| r.unwrap_err().0;
    assert_eq!(
        invalid(run(&mut d, json!({"type": "make_group", "ids": []}))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        run(&mut d, json!({"type": "make_group", "ids": ["v0"]})).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["v0".to_string()])
    );
    assert_eq!(
        invalid(run(
            &mut d,
            json!({"type": "make_component", "ids": [i], "name": " "})
        )),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        invalid(run(&mut d, json!({"type": "explode", "ids": ["f8"]}))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        invalid(run(&mut d, json!({"type": "open_instance", "id": "f8"}))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        invalid(run(&mut d, json!({"type": "open_instance", "id": "i999"}))),
        ErrorKind::NotFound
    );
    assert_eq!(
        invalid(run(&mut d, json!({"type": "close_instance"}))),
        ErrorKind::InvalidInput
    );
    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    assert_eq!(
        run(&mut d, json!({"type": "select", "ids": [i]})).unwrap_err(),
        (ErrorKind::NotFound, vec![i.clone()]),
        "the open instance is not in its own context"
    );
}

#[test]
fn instances_filter_by_kind_and_box() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(
        &mut d,
        json!({"type": "add_edge", "start": [5, 0, 0], "end": [6, 0, 0]}),
    )
    .unwrap();
    let only = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    assert_eq!(only["instances"].as_array().unwrap().len(), 1);
    assert_eq!(only["edges"], json!([]));
    let inside = ask(
        &d,
        json!({"type": "entities", "within": {"min": [-1, -1, -1], "max": [3, 3, 3]}}),
    );
    assert_eq!(inside["instances"][0]["id"], json!(i));
    assert_eq!(inside["edges"], json!([]));
    let outside = ask(
        &d,
        json!({"type": "entities", "within": {"min": [0, 0, 0], "max": [1, 1, 1]}}),
    );
    assert_eq!(outside["instances"], json!([]));
}

#[test]
fn repeated_ids_count_once() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    let out = run(&mut d, json!({"type": "make_group", "ids": [i, i]})).unwrap();
    let outer = created_instance(&out);
    run(&mut d, json!({"type": "explode", "ids": [outer, outer]})).unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 1, 1]);
}
```

Apply to `crates/session/tests/commands.rs`:
```diff
--- a/crates/session/tests/commands.rs
+++ b/crates/session/tests/commands.rs
@@ -45,9 +45,17 @@ fn square_via_commands_makes_a_queryable_face() {
     draw_square(&mut d);
     let stats = ask(&d, json!({"type": "stats"}));
     assert_eq!(
         stats,
-        json!({"type": "stats", "vertices": 4, "edges": 4, "faces": 1, "revision": 4})
+        json!({
+            "type": "stats",
+            "vertices": 4,
+            "edges": 4,
+            "faces": 1,
+            "instances": 0,
+            "definitions": 0,
+            "revision": 4
+        })
     );
     let all = ask(&d, json!({"type": "entities"}));
     let face = &all["faces"][0];
     assert_eq!(face["area"], json!(4.0));
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p session --test groups --test commands`
Expected:
- `groups`: all 11 tests fail with ``valid command json: ... unknown variant `make_group` `` (one with `make_component`);
- `commands`: `square_via_commands_makes_a_queryable_face` fails (the stats JSON lacks `instances` and `definitions`).

- [ ] **Step 3: Implement**

`document.rs` is the largest change:
- `model: Model` becomes `scene: Scene` plus `context: Vec<InstanceId>`; undo and redo hold `SceneChange`s;
- every edit runs through `edit_mesh` on the open context's mesh;
- `edges_and_faces` becomes `parts` (edges, faces and instances, each id once);
- `changed` closes open instances that stopped existing.

Apply to `crates/session/src/command.rs`:
```diff
--- a/crates/session/src/command.rs
+++ b/crates/session/src/command.rs
@@ -4,9 +4,11 @@ use kernel::{ChangeSummary, EntityId};
 use schemars::JsonSchema;
 use serde::{Deserialize, Serialize};
 
 /// Every state change a client can make. Values are explicit (meters, degrees, entity ids);
-/// commands never refer to screen positions or cursor state.
+/// commands never refer to screen positions or cursor state. Points, offsets and entity ids are
+/// those of the open context (see `OpenInstance`): the root, or the definition of the instance
+/// being edited, in its own coordinates.
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
 pub enum Command {
     /// Draw a straight edge. Sticky-geometry rules apply (merging, splitting, auto faces).
@@ -28,29 +30,58 @@ pub enum Command {
     PushPull {
         face: EntityId,
         distance: f64,
     },
-    /// Erase edges and faces as one undo step. An erased edge takes the faces it bounds with
-    /// it, except that two faces in one plane meeting there merge into one. Vertices go away
-    /// with their last edge and cannot be erased directly.
+    /// Erase edges, faces and instances as one undo step. An erased edge takes the faces it
+    /// bounds with it, except that two faces in one plane meeting there merge into one.
+    /// Vertices go away with their last edge and cannot be erased directly. A group's
+    /// definition goes with its last instance.
     Erase {
         ids: Vec<EntityId>,
     },
-    /// Move vertices, edges and faces by `offset` meters as one undo step. Faces that share
-    /// some of the moved vertices stretch and may tilt, but must stay flat.
+    /// Move vertices, edges, faces and instances by `offset` meters as one undo step. Faces
+    /// that share some of the moved vertices stretch and may tilt, but must stay flat.
     Move {
         ids: Vec<EntityId>,
         offset: [f64; 3],
     },
-    /// Copy edges and faces `count` times (default 1), copy k shifted by `k * offset`, as one
-    /// undo step. Copies merge with geometry they touch; copied faces keep their orientation.
+    /// Copy edges, faces and instances `count` times (default 1), copy k shifted by
+    /// `k * offset`, as one undo step. Copies merge with geometry they touch; copied faces keep
+    /// their orientation. A copied component instance places the same definition; a copied
+    /// group gets a definition of its own.
     Copy {
         ids: Vec<EntityId>,
         offset: [f64; 3],
         #[serde(default)]
         count: Option<u32>,
     },
-    /// Change the selection of edges and faces. Not recorded in undo history.
+    /// Turn edges, faces (with their edges) and instances into a group: they move into a new
+    /// definition, placed by one new instance where they were. The instance becomes the
+    /// selection.
+    MakeGroup {
+        ids: Vec<EntityId>,
+    },
+    /// Like `MakeGroup`, but the definition is a named component: copies of its instances
+    /// share it, so editing one edits them all.
+    MakeComponent {
+        ids: Vec<EntityId>,
+        name: String,
+    },
+    /// Replace instances by their geometry, which merges with what it touches. Instances
+    /// inside them come out as instances. What comes out becomes the selection.
+    Explode {
+        ids: Vec<EntityId>,
+    },
+    /// Edit the definition an instance of the open context places: its entities become the
+    /// ones commands and queries see, in its own coordinates. Clears the selection. Not
+    /// recorded in undo history.
+    OpenInstance {
+        id: EntityId,
+    },
+    /// Go back to editing the context the open instance is in. Clears the selection. Not
+    /// recorded in undo history.
+    CloseInstance,
+    /// Change the selection of edges, faces and instances. Not recorded in undo history.
     Select {
         ids: Vec<EntityId>,
         #[serde(default)]
         mode: SelectMode,
@@ -65,9 +96,10 @@ pub enum Command {
     },
 }
 
 impl Command {
-    /// True for commands that can change the model (everything except camera and selection).
+    /// True for commands that can change the model or the open context (everything except
+    /// camera and selection).
     pub fn changes_model(&self) -> bool {
         !matches!(self, Command::SetCamera { .. } | Command::Select { .. })
     }
 }
```

Apply to `crates/session/src/query.rs`:
```diff
--- a/crates/session/src/query.rs
+++ b/crates/session/src/query.rs
@@ -1,32 +1,37 @@
-use kernel::{ChangeSummary, EntityId};
+use kernel::{ChangeSummary, DefinitionId, DefinitionKind, EntityId};
 use schemars::JsonSchema;
 use serde::{Deserialize, Serialize};
 
-/// Read-only questions about the document.
+/// Read-only questions about the document. Entities and coordinates are those of the open
+/// context (see `Command::OpenInstance`).
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
 pub enum Query {
-    /// Vertices, edges and faces. An entity is returned when `ids` is empty or lists it,
-    /// `kinds` is empty or lists its kind, and `within` is absent or contains all its vertices;
-    /// unknown ids are simply absent from the result.
+    /// Vertices, edges, faces and instances. An entity is returned when `ids` is empty or lists
+    /// it, `kinds` is empty or lists its kind, and `within` is absent or contains all its
+    /// vertices (an instance: its bounding box); unknown ids are simply absent from the result.
     Entities {
         #[serde(default)]
         ids: Vec<EntityId>,
         #[serde(default)]
         kinds: Vec<EntityKind>,
         #[serde(default)]
         within: Option<Bounds>,
     },
-    /// One entity by id ("v3", "e7", "f2").
+    /// One entity by id ("v3", "e7", "f2", "i4").
     Entity {
         id: EntityId,
     },
     Camera,
     Stats,
     History,
-    /// The selected edges and faces.
+    /// The selected edges, faces and instances.
     Selection,
+    /// Every group and component definition.
+    Definitions,
+    /// The open context: the path of instances being edited, from the root.
+    Context,
     /// Model changes after `since` (a revision number), oldest first.
     Events {
         since: u64,
     },
@@ -45,15 +50,16 @@ impl Bounds {
         (0..3).all(|i| self.min[i] <= p[i] && p[i] <= self.max[i])
     }
 }
 
-/// The three kinds of entity.
+/// The kinds of entity.
 #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
 #[serde(rename_all = "snake_case")]
 pub enum EntityKind {
     Vertex,
     Edge,
     Face,
+    Instance,
 }
 
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 pub struct VertexInfo {
@@ -77,29 +83,60 @@ pub struct FaceInfo {
     pub normal: [f64; 3],
     pub area: f64,
 }
 
+/// A placed group or component.
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
+pub struct InstanceInfo {
+    pub id: EntityId,
+    pub definition: DefinitionId,
+    pub name: String,
+    pub kind: DefinitionKind,
+    /// Column-major 3x4 matrix mapping the definition's coordinates into the open context's:
+    /// x axis, y axis, z axis, translation.
+    pub transform: [f64; 12],
+    /// The box around the instance's geometry, in the open context's coordinates. Absent for
+    /// an empty definition.
+    pub bounds: Option<Bounds>,
+}
+
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
+pub struct DefinitionInfo {
+    pub id: DefinitionId,
+    pub name: String,
+    pub kind: DefinitionKind,
+    /// How many instances place it, in all contexts.
+    pub instances: usize,
+    pub edges: usize,
+    pub faces: usize,
+}
+
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 #[serde(tag = "type", rename_all = "snake_case")]
 pub enum QueryResult {
     Entities {
         vertices: Vec<VertexInfo>,
         edges: Vec<EdgeInfo>,
         faces: Vec<FaceInfo>,
+        instances: Vec<InstanceInfo>,
         revision: u64,
     },
     Vertex(VertexInfo),
     Edge(EdgeInfo),
     Face(FaceInfo),
+    Instance(InstanceInfo),
     Camera {
         eye: [f64; 3],
         target: [f64; 3],
         fov_y_deg: f64,
     },
+    /// Counts for the open context, plus the number of definitions.
     Stats {
         vertices: usize,
         edges: usize,
         faces: usize,
+        instances: usize,
+        definitions: usize,
         revision: u64,
     },
     History {
         undo_depth: usize,
@@ -109,8 +146,18 @@ pub enum QueryResult {
     Selection {
         ids: Vec<EntityId>,
         revision: u64,
     },
+    Definitions {
+        definitions: Vec<DefinitionInfo>,
+        revision: u64,
+    },
+    /// `path` lists the open instances from the root; empty when the root is open.
+    /// `definition` is the definition being edited.
+    Context {
+        path: Vec<EntityId>,
+        definition: Option<DefinitionId>,
+    },
     /// `truncated` is true when changes after `since` were dropped from the log; the client
     /// should re-read the model (e.g. with `entities`) instead of applying `events`.
     Events {
         revision: u64,
```

Apply to `crates/session/src/lib.rs`:
```diff
--- a/crates/session/src/lib.rs
+++ b/crates/session/src/lib.rs
@@ -9,7 +9,8 @@ mod rectangle;
 pub use camera::{Camera, Ray};
 pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
 pub use document::{Document, MAX_COORDINATE, MAX_COPIES};
 pub use query::{
-    Bounds, ChangeEvent, EdgeInfo, EntityKind, FaceInfo, Query, QueryResult, VertexInfo,
+    Bounds, ChangeEvent, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, InstanceInfo, Query,
+    QueryResult, VertexInfo,
 };
 pub use rectangle::{rectangle_axes, rectangle_corners};
```

Apply to `crates/session/src/document.rs`:
```diff
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -1,14 +1,21 @@
 use std::collections::{BTreeSet, VecDeque};
 
-use glam::{DVec2, DVec3};
-use kernel::{ChangeSet, EdgeId, EntityId, FaceId, KernelError, Model, VertexId};
+use glam::{DAffine3, DVec2, DVec3};
+use kernel::{
+    ChangeSet, EdgeId, EntityId, FaceId, InstanceId, KernelError, MeshId, Model, Scene,
+    SceneChange, VertexId,
+};
 
 use crate::camera::Camera;
 use crate::command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
-use crate::query::{ChangeEvent, EdgeInfo, EntityKind, FaceInfo, Query, QueryResult, VertexInfo};
+use crate::query::{
+    ChangeEvent, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, Query, QueryResult, VertexInfo,
+};
 use crate::rectangle::rectangle_corners;
 
+mod groups;
+
 /// Largest coordinate magnitude (meters) accepted from commands.
 pub const MAX_COORDINATE: f64 = 1.0e6;
 
 /// Most copies one `copy` command may make.
@@ -16,23 +23,27 @@ pub const MAX_COPIES: u32 = 100;
 
 /// How many recent model changes `Query::Events` can return.
 const EVENT_LOG_LEN: usize = 1024;
 
-/// The open model plus everything needed to edit it: camera, selection and undo history.
+/// The open model plus everything needed to edit it: camera, selection, the open context and
+/// undo history.
 pub struct Document {
-    model: Model,
+    scene: Scene,
+    /// The instances being edited, from the root; empty when the root is open.
+    context: Vec<InstanceId>,
     pub camera: Camera,
     selection: BTreeSet<EntityId>,
-    undo: Vec<ChangeSet>,
-    redo: Vec<ChangeSet>,
+    undo: Vec<SceneChange>,
+    redo: Vec<SceneChange>,
     revision: u64,
     events: VecDeque<ChangeEvent>,
 }
 
 impl Document {
     pub fn new(viewport: DVec2) -> Self {
         Self {
-            model: Model::new(),
+            scene: Scene::new(),
+            context: Vec::new(),
             camera: Camera::new(viewport),
             selection: BTreeSet::new(),
             undo: Vec::new(),
             redo: Vec::new(),
@@ -40,13 +51,50 @@ impl Document {
             events: VecDeque::new(),
         }
     }
 
+    /// The whole model: the root mesh and every definition.
+    pub fn scene(&self) -> &Scene {
+        &self.scene
+    }
+
+    /// The mesh being edited: the root, or the definition of the open instance.
     pub fn model(&self) -> &Model {
-        &self.model
+        self.scene
+            .mesh(self.active())
+            .expect("the open context exists")
+    }
+
+    /// The instances being edited, from the root; empty when the root is open.
+    pub fn context(&self) -> &[InstanceId] {
+        &self.context
+    }
+
+    /// Maps the open context's coordinates to the root's.
+    pub fn context_transform(&self) -> DAffine3 {
+        self.scene
+            .resolve(&self.context)
+            .expect("the open context exists")
+            .1
     }
 
-    /// The selected edges and faces. Entities leave it when they stop existing.
+    fn active(&self) -> MeshId {
+        self.scene
+            .resolve(&self.context)
+            .expect("the open context exists")
+            .0
+    }
+
+    /// Runs one kernel edit on the open context's mesh.
+    fn edit_mesh(
+        &mut self,
+        f: impl FnOnce(&mut Model) -> Result<ChangeSet, KernelError>,
+    ) -> Result<SceneChange, KernelError> {
+        let mesh = self.active();
+        self.scene.edit(mesh, f)
+    }
+
+    /// The selected edges, faces and instances. Entities leave it when they stop existing.
     pub fn selection(&self) -> &BTreeSet<EntityId> {
         &self.selection
     }
 
@@ -81,17 +129,17 @@ impl Document {
                 let changes = self.undo.pop().ok_or_else(|| {
                     CommandError::new(ErrorKind::NothingToUndo, "nothing to undo")
                 })?;
                 let inverse = changes.inverse();
-                self.model.apply(&inverse);
+                self.scene.apply(&inverse);
                 self.redo.push(changes);
                 Ok(self.changed(&inverse))
             }
             Command::Redo => {
                 let changes = self.redo.pop().ok_or_else(|| {
                     CommandError::new(ErrorKind::NothingToRedo, "nothing to redo")
                 })?;
-                self.model.apply(&changes);
+                self.scene.apply(&changes);
                 let outcome = self.changed(&changes);
                 self.undo.push(changes);
                 Ok(outcome)
             }
@@ -102,10 +150,18 @@ impl Document {
             } => {
                 self.camera = self.camera_for(eye, target, fov_y_deg)?;
                 Ok(Outcome::default())
             }
+            Command::OpenInstance { id } => {
+                self.open_instance(id)?;
+                Ok(Outcome::default())
+            }
+            Command::CloseInstance => {
+                self.close_instance()?;
+                Ok(Outcome::default())
+            }
             Command::Select { ids, mode } => {
-                self.edges_and_faces(&ids)?;
+                self.parts(&ids)?;
                 let ids: BTreeSet<EntityId> = ids.into_iter().collect();
                 match mode {
                     SelectMode::Replace => self.selection = ids,
                     SelectMode::Add => self.selection.extend(ids),
@@ -121,18 +177,21 @@ impl Document {
             | Command::AddRectangle { .. }
             | Command::PushPull { .. }
             | Command::Erase { .. }
             | Command::Move { .. }
-            | Command::Copy { .. } => {
+            | Command::Copy { .. }
+            | Command::MakeGroup { .. }
+            | Command::MakeComponent { .. }
+            | Command::Explode { .. } => {
                 unreachable!("edits are handled by Document::edit")
             }
         }
     }
 
     /// Applies an editing command to the model and returns its changes, not yet recorded in
-    /// history. None for commands that are not edits (undo, redo, camera, selection). Every
-    /// editing command goes through here, so `execute` and atomic batches stay in step.
-    fn edit(&mut self, command: &Command) -> Option<Result<ChangeSet, CommandError>> {
+    /// history. None for commands that are not edits (undo, redo, camera, selection, context).
+    /// Every editing command goes through here, so `execute` and atomic batches stay in step.
+    fn edit(&mut self, command: &Command) -> Option<Result<SceneChange, CommandError>> {
         Some(match command {
             Command::AddEdge { start, end } => self.add_edge_changes(*start, *end),
             Command::AddRectangle {
                 corner,
@@ -144,9 +203,17 @@ impl Document {
             Command::Move { ids, offset } => self.move_changes(ids, *offset),
             Command::Copy { ids, offset, count } => {
                 self.copy_changes(ids, *offset, count.unwrap_or(1))
             }
-            Command::Undo | Command::Redo | Command::SetCamera { .. } | Command::Select { .. } => {
+            Command::MakeGroup { ids } => self.group_changes(ids, None),
+            Command::MakeComponent { ids, name } => self.group_changes(ids, Some(name)),
+            Command::Explode { ids } => self.explode_changes(ids),
+            Command::Undo
+            | Command::Redo
+            | Command::SetCamera { .. }
+            | Command::Select { .. }
+            | Command::OpenInstance { .. }
+            | Command::CloseInstance => {
                 return None;
             }
         })
     }
@@ -191,10 +258,11 @@ impl Document {
     ///
     /// A failed atomic batch still consumes entity ids: ids are never reused.
     ///
     /// Non-atomic: each command is its own undo step and earlier commands stay applied.
-    /// Atomic: all modeling commands become one undo step, and on error everything (model and
-    /// camera) is rolled back. Undo and redo are not allowed inside an atomic batch.
+    /// Atomic: all modeling commands become one undo step, and on error everything (model,
+    /// camera, selection and open context) is rolled back. Undo and redo are not allowed inside
+    /// an atomic batch.
     pub fn execute_batch(
         &mut self,
         commands: Vec<Command>,
         atomic: bool,
@@ -218,9 +286,10 @@ impl Document {
         }
 
         let camera_before = self.camera.clone();
         let selection_before = self.selection.clone();
-        let mut applied: Vec<ChangeSet> = Vec::new();
+        let context_before = self.context.clone();
+        let mut applied: Vec<SceneChange> = Vec::new();
         for (index, command) in commands.into_iter().enumerate() {
             let result = match self.edit(&command) {
                 Some(changes) => changes.map(|c| {
                     let summary = c.summary();
@@ -236,12 +305,13 @@ impl Document {
             match result {
                 Ok(o) => outcomes.push(o),
                 Err(error) => {
                     for c in applied.iter().rev() {
-                        self.model.apply(&c.inverse());
+                        self.scene.apply(&c.inverse());
                     }
                     self.camera = camera_before;
                     self.selection = selection_before;
+                    self.context = context_before;
                     return Err(BatchError {
                         index,
                         error,
                         completed: outcomes,
@@ -249,56 +319,51 @@ impl Document {
                     });
                 }
             }
         }
-        let merged = ChangeSet {
-            ops: applied.into_iter().flat_map(|c| c.ops).collect(),
-        };
-        self.record(merged);
+        self.record(SceneChange::join(applied));
         Ok(outcomes)
     }
 
     fn add_edge_changes(
         &mut self,
         start: [f64; 3],
         end: [f64; 3],
-    ) -> Result<ChangeSet, CommandError> {
+    ) -> Result<SceneChange, CommandError> {
         check_coordinates(&[start, end])?;
-        self.model
-            .add_edge(DVec3::from(start), DVec3::from(end))
+        self.edit_mesh(|m| m.add_edge(DVec3::from(start), DVec3::from(end)))
             .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
     }
 
     fn add_rectangle_changes(
         &mut self,
         corner: [f64; 3],
         opposite: [f64; 3],
         normal: Option<[f64; 3]>,
-    ) -> Result<ChangeSet, CommandError> {
+    ) -> Result<SceneChange, CommandError> {
         check_coordinates(&[corner, opposite])?;
         let corners = rectangle_corners(
             DVec3::from(corner),
             DVec3::from(opposite),
             normal.map(DVec3::from),
         )?;
         check_coordinates(&corners.map(|c| c.to_array()))?;
-        self.model
-            .add_polygon(&corners)
+        self.edit_mesh(|m| m.add_polygon(&corners))
             .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
     }
 
     fn push_pull_changes(
         &mut self,
         face: EntityId,
         distance: f64,
-    ) -> Result<ChangeSet, CommandError> {
+    ) -> Result<SceneChange, CommandError> {
         let EntityId::Face(id) = face else {
             return Err(
                 CommandError::new(ErrorKind::InvalidInput, "push_pull needs a face id")
                     .with_ids(vec![face]),
             );
         };
-        let Some(f) = self.model.face(id) else {
+        let Some(f) = self.model().face(id) else {
             return Err(
                 CommandError::new(ErrorKind::NotFound, format!("no entity {face}"))
                     .with_ids(vec![face]),
             );
@@ -311,25 +376,25 @@ impl Document {
         }
         let moved: Vec<[f64; 3]> = std::iter::once(&f.outer)
             .chain(&f.holes)
             .flatten()
-            .map(|v| (self.model.position(*v).unwrap() + f.normal * distance).to_array())
+            .map(|v| (self.model().position(*v).unwrap() + f.normal * distance).to_array())
             .collect();
         check_coordinates(&moved)?;
-        self.model.push_pull(id, distance).map_err(|e| {
+        self.edit_mesh(|m| m.push_pull(id, distance)).map_err(|e| {
             CommandError::new(ErrorKind::InvalidInput, e.to_string()).with_ids(vec![face])
         })
     }
 
     fn move_changes(
         &mut self,
         ids: &[EntityId],
         offset: [f64; 3],
-    ) -> Result<ChangeSet, CommandError> {
+    ) -> Result<SceneChange, CommandError> {
         if ids.is_empty() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
-                "ids must list at least one vertex, edge or face",
+                "ids must list at least one vertex, edge, face or instance",
             ));
         }
         let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
         if !missing.is_empty() {
@@ -338,61 +403,105 @@ impl Document {
             );
         }
         check_coordinates(&[offset])?;
         let vertices = self.vertex_ids(ids);
+        let instances: Vec<InstanceId> = ids
+            .iter()
+            .filter_map(|id| match id {
+                EntityId::Instance(i) => Some(*i),
+                _ => None,
+            })
+            .collect();
         let offset = DVec3::from(offset);
-        self.check_moved(&vertices, &[offset])?;
-        self.model
-            .move_vertices(&vertices, offset)
-            .map_err(|e| self.kernel_error(e, ids))
+        self.check_moved(&vertices, &instances, &[offset])?;
+        self.edit_mesh(|m| {
+            let mut c = m.move_vertices(&vertices, offset)?;
+            c.ops.extend(m.move_instances(&instances, offset).ops);
+            Ok(c)
+        })
+        .map_err(|e| self.kernel_error(e, ids))
     }
 
     fn copy_changes(
         &mut self,
         ids: &[EntityId],
         offset: [f64; 3],
         count: u32,
-    ) -> Result<ChangeSet, CommandError> {
+    ) -> Result<SceneChange, CommandError> {
         if ids.is_empty() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
-                "ids must list at least one edge or face",
+                "ids must list at least one edge, face or instance",
             ));
         }
         if !(1..=MAX_COPIES).contains(&count) {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
                 format!("count must be between 1 and {MAX_COPIES}"),
             ));
         }
-        let (edges, faces) = self.edges_and_faces(ids)?;
+        let parts = self.parts(ids)?;
         check_coordinates(&[offset])?;
         let offset = DVec3::from(offset);
-        self.check_moved(&self.vertex_ids(ids), &[offset * count as f64])?;
-        self.model
-            .copy(&edges, &faces, offset, count)
-            .map_err(|e| self.kernel_error(e, ids))
+        self.check_moved(
+            &self.vertex_ids(ids),
+            &parts.instances,
+            &[offset * count as f64],
+        )?;
+        let geometry = if parts.edges.is_empty() && parts.faces.is_empty() {
+            SceneChange::default()
+        } else {
+            self.edit_mesh(|m| m.copy(&parts.edges, &parts.faces, offset, count))
+                .map_err(|e| self.kernel_error(e, ids))?
+        };
+        let placed = if parts.instances.is_empty() {
+            SceneChange::default()
+        } else {
+            let mesh = self.active();
+            match self
+                .scene
+                .copy_instances(mesh, &parts.instances, offset, count)
+            {
+                Ok(c) => c,
+                Err(e) => {
+                    self.scene.apply(&geometry.inverse());
+                    return Err(self.kernel_error(e, ids));
+                }
+            }
+        };
+        Ok(SceneChange::join([geometry, placed]))
     }
 
-    /// Rejects moves or copies that would put a vertex beyond `MAX_COORDINATE`.
+    /// Rejects moves or copies that would put a vertex or an instance's bounding box beyond
+    /// `MAX_COORDINATE`.
     fn check_moved(
         &self,
         vertices: &BTreeSet<VertexId>,
+        instances: &[InstanceId],
         offsets: &[DVec3],
     ) -> Result<(), CommandError> {
-        let moved: Vec<[f64; 3]> = vertices
+        let mut points: Vec<DVec3> = vertices
             .iter()
-            .flat_map(|v| {
-                let p = self.model.position(*v).unwrap();
-                offsets.iter().map(move |o| (p + *o).to_array())
-            })
+            .map(|v| self.model().position(*v).unwrap())
+            .collect();
+        for i in instances {
+            points.extend(self.instance_box(*i).into_iter().flatten());
+        }
+        let moved: Vec<[f64; 3]> = points
+            .iter()
+            .flat_map(|p| offsets.iter().map(move |o| (*p + *o).to_array()))
             .collect();
         check_coordinates(&moved)
     }
 
+    /// The box around an instance of the open context, in the context's coordinates.
+    fn instance_box(&self, id: InstanceId) -> Option<[DVec3; 2]> {
+        self.scene.instance_bounds(self.model().instance(id)?)
+    }
+
     /// The vertices that make up the entities (vertices themselves, edge ends, face loops).
     fn vertex_ids(&self, ids: &[EntityId]) -> BTreeSet<VertexId> {
-        let m = &self.model;
+        let m = self.model();
         let mut out = BTreeSet::new();
         for id in ids {
             match *id {
                 EntityId::Vertex(v) => {
@@ -418,34 +527,37 @@ impl Document {
         };
         CommandError::new(ErrorKind::InvalidInput, e.to_string()).with_ids(culprits)
     }
 
-    fn erase_changes(&mut self, ids: &[EntityId]) -> Result<ChangeSet, CommandError> {
+    fn erase_changes(&mut self, ids: &[EntityId]) -> Result<SceneChange, CommandError> {
         if ids.is_empty() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
-                "ids must list at least one edge or face",
+                "ids must list at least one edge, face or instance",
             ));
         }
-        let (edges, faces) = self.edges_and_faces(ids)?;
-        Ok(self.model.erase(&edges, &faces))
+        let parts = self.parts(ids)?;
+        let erased = self
+            .edit_mesh(|m| Ok(m.erase(&parts.edges, &parts.faces)))
+            .expect("erase cannot fail");
+        let mesh = self.active();
+        let removed = self.scene.erase_instances(mesh, &parts.instances);
+        Ok(SceneChange::join([erased, removed]))
     }
 
-    /// Splits ids into edges and faces. Vertex ids are invalid input and unknown ids are not
-    /// found; both errors list the offending ids.
-    fn edges_and_faces(
-        &self,
-        ids: &[EntityId],
-    ) -> Result<(Vec<EdgeId>, Vec<FaceId>), CommandError> {
+    /// Splits ids (each counted once) into edges, faces and instances of the open context.
+    /// Vertex ids are invalid input and unknown ids are not found; both errors list the
+    /// offending ids.
+    fn parts(&self, ids: &[EntityId]) -> Result<Parts, CommandError> {
         let vertices: Vec<EntityId> = ids
             .iter()
             .copied()
             .filter(|id| matches!(id, EntityId::Vertex(_)))
             .collect();
         if !vertices.is_empty() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
-                "only edges and faces can be selected or erased",
+                "only edges, faces and instances are accepted here, not vertices",
             )
             .with_ids(vertices));
         }
         let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
@@ -453,35 +565,32 @@ impl Document {
             return Err(
                 CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
             );
         }
-        let edges = ids
-            .iter()
-            .filter_map(|id| match id {
-                EntityId::Edge(e) => Some(*e),
-                _ => None,
-            })
-            .collect();
-        let faces = ids
-            .iter()
-            .filter_map(|id| match id {
-                EntityId::Face(f) => Some(*f),
-                _ => None,
-            })
-            .collect();
-        Ok((edges, faces))
+        let mut parts = Parts::default();
+        for id in ids.iter().collect::<BTreeSet<_>>() {
+            match *id {
+                EntityId::Edge(e) => parts.edges.push(e),
+                EntityId::Face(f) => parts.faces.push(f),
+                EntityId::Instance(i) => parts.instances.push(i),
+                EntityId::Vertex(_) => {}
+            }
+        }
+        Ok(parts)
     }
 
+    /// Whether the open context holds the entity.
     fn exists(&self, id: EntityId) -> bool {
+        let m = self.model();
         match id {
-            EntityId::Vertex(v) => self.model.vertex(v).is_some(),
-            EntityId::Edge(e) => self.model.edge(e).is_some(),
-            EntityId::Face(f) => self.model.face(f).is_some(),
-            EntityId::Instance(i) => self.model.instance(i).is_some(),
+            EntityId::Vertex(v) => m.vertex(v).is_some(),
+            EntityId::Edge(e) => m.edge(e).is_some(),
+            EntityId::Face(f) => m.face(f).is_some(),
+            EntityId::Instance(i) => m.instance(i).is_some(),
         }
     }
 
-    fn record(&mut self, changes: ChangeSet) -> Outcome {
+    fn record(&mut self, changes: SceneChange) -> Outcome {
         if changes.is_empty() {
             return Outcome::default();
         }
         let outcome = self.changed(&changes);
@@ -489,12 +598,15 @@ impl Document {
         self.redo.clear();
         outcome
     }
 
-    /// Bumps the revision, logs the change and drops entities that no longer exist from the
-    /// selection.
-    fn changed(&mut self, changes: &ChangeSet) -> Outcome {
+    /// Bumps the revision, logs the change, closes open instances that no longer exist and
+    /// drops entities that are no longer in the open context from the selection.
+    fn changed(&mut self, changes: &SceneChange) -> Outcome {
         self.revision += 1;
+        while self.scene.resolve(&self.context).is_none() {
+            self.context.pop();
+        }
         self.selection = self
             .selection
             .iter()
             .copied()
@@ -511,15 +623,16 @@ impl Document {
         Outcome { changes: summary }
     }
 
     pub fn query(&self, query: &Query) -> Result<QueryResult, CommandError> {
-        let m = &self.model;
+        let m = self.model();
         Ok(match query {
             Query::Entities { ids, kinds, within } => {
                 let wanted = |id: EntityId, kind: EntityKind| {
                     (ids.is_empty() || ids.contains(&id))
                         && (kinds.is_empty() || kinds.contains(&kind))
-                        && within.is_none_or(|b| self.vertices_of(id).all(|p| b.contains(p)))
+                        && within
+                            .is_none_or(|b| self.vertices_of(id).into_iter().all(|p| b.contains(p)))
                 };
                 QueryResult::Entities {
                     vertices: m
                         .vertices()
@@ -535,8 +648,13 @@ impl Document {
                         .faces()
                         .filter(|(id, _)| wanted(EntityId::Face(*id), EntityKind::Face))
                         .map(|(id, _)| self.face_info(id))
                         .collect(),
+                    instances: m
+                        .instances()
+                        .filter(|(id, _)| wanted(EntityId::Instance(*id), EntityKind::Instance))
+                        .map(|(id, _)| self.instance_info(id))
+                        .collect(),
                     revision: self.revision,
                 }
             }
             Query::Entity { id } => {
@@ -555,9 +673,13 @@ impl Document {
                     }
                     EntityId::Face(f) => {
                         QueryResult::Face(m.face(f).map(|_| self.face_info(f)).ok_or_else(missing)?)
                     }
-                    EntityId::Instance(_) => return Err(missing()),
+                    EntityId::Instance(i) => QueryResult::Instance(
+                        m.instance(i)
+                            .map(|_| self.instance_info(i))
+                            .ok_or_else(missing)?,
+                    ),
                 }
             }
             Query::Camera => QueryResult::Camera {
                 eye: self.camera.eye.to_array(),
@@ -567,8 +689,10 @@ impl Document {
             Query::Stats => QueryResult::Stats {
                 vertices: m.vertices().count(),
                 edges: m.edges().count(),
                 faces: m.faces().count(),
+                instances: m.instances().count(),
+                definitions: self.scene.definitions().count(),
                 revision: self.revision,
             },
             Query::History => QueryResult::History {
                 undo_depth: self.undo.len(),
@@ -578,8 +702,34 @@ impl Document {
             Query::Selection => QueryResult::Selection {
                 ids: self.selection.iter().copied().collect(),
                 revision: self.revision,
             },
+            Query::Definitions => QueryResult::Definitions {
+                definitions: self
+                    .scene
+                    .definitions()
+                    .map(|(id, d)| DefinitionInfo {
+                        id,
+                        name: d.name.clone(),
+                        kind: d.kind,
+                        instances: self.scene.instance_count(id),
+                        edges: d.mesh.edges().count(),
+                        faces: d.mesh.faces().count(),
+                    })
+                    .collect(),
+                revision: self.revision,
+            },
+            Query::Context => QueryResult::Context {
+                path: self
+                    .context
+                    .iter()
+                    .map(|i| EntityId::Instance(*i))
+                    .collect(),
+                definition: match self.active() {
+                    MeshId::Root => None,
+                    MeshId::Definition(d) => Some(d),
+                },
+            },
             Query::Events { since } => {
                 let (events, truncated) = self.events_since(*since);
                 QueryResult::Events {
                     revision: self.revision,
@@ -589,14 +739,22 @@ impl Document {
             }
         })
     }
 
-    /// Positions of the vertices an entity is made of.
-    fn vertices_of(&self, id: EntityId) -> impl Iterator<Item = [f64; 3]> + '_ {
-        let m = &self.model;
+    /// Positions of the vertices an entity is made of; for an instance, the corners of its
+    /// bounding box.
+    fn vertices_of(&self, id: EntityId) -> Vec<[f64; 3]> {
+        let m = self.model();
         let ids: Vec<VertexId> = match id {
             EntityId::Vertex(v) => vec![v],
-            EntityId::Instance(_) => vec![],
+            EntityId::Instance(i) => {
+                return self
+                    .instance_box(i)
+                    .into_iter()
+                    .flatten()
+                    .map(|p| p.to_array())
+                    .collect();
+            }
             EntityId::Edge(e) => m.edge(e).map(|e| e.v.to_vec()).unwrap_or_default(),
             EntityId::Face(f) => m
                 .face(f)
                 .map(|f| {
@@ -609,40 +767,49 @@ impl Document {
                 .unwrap_or_default(),
         };
         ids.into_iter()
             .filter_map(|v| m.position(v).map(|p| p.to_array()))
+            .collect()
     }
 
     fn vertex_info(&self, id: VertexId) -> VertexInfo {
         VertexInfo {
             id: EntityId::Vertex(id),
-            position: self.model.position(id).unwrap().to_array(),
+            position: self.model().position(id).unwrap().to_array(),
         }
     }
 
     fn edge_info(&self, id: EdgeId) -> EdgeInfo {
-        let [a, b] = self.model.edge(id).unwrap().v;
+        let [a, b] = self.model().edge(id).unwrap().v;
         EdgeInfo {
             id: EntityId::Edge(id),
             vertices: [EntityId::Vertex(a), EntityId::Vertex(b)],
-            start: self.model.position(a).unwrap().to_array(),
-            end: self.model.position(b).unwrap().to_array(),
+            start: self.model().position(a).unwrap().to_array(),
+            end: self.model().position(b).unwrap().to_array(),
         }
     }
 
     fn face_info(&self, id: FaceId) -> FaceInfo {
-        let f = self.model.face(id).unwrap();
+        let f = self.model().face(id).unwrap();
         let ids = |l: &Vec<VertexId>| l.iter().map(|v| EntityId::Vertex(*v)).collect();
         FaceInfo {
             id: EntityId::Face(id),
             outer: ids(&f.outer),
             holes: f.holes.iter().map(ids).collect(),
             normal: f.normal.to_array(),
-            area: self.model.area(id).unwrap(),
+            area: self.model().area(id).unwrap(),
         }
     }
 }
 
+/// Edges, faces and instances named by a command.
+#[derive(Default)]
+struct Parts {
+    edges: Vec<EdgeId>,
+    faces: Vec<FaceId>,
+    instances: Vec<InstanceId>,
+}
+
 /// Rejects non-finite or out-of-range coordinates before they reach the kernel or camera.
 fn check_coordinates(points: &[[f64; 3]]) -> Result<(), CommandError> {
     let ok = points
         .iter()
```

Create `crates/session/src/document/groups.rs`:
```rust
//! Groups, components and the open context.

use std::collections::BTreeSet;

use kernel::{DefinitionKind, EntityId, KernelError, SceneChange};

use super::Document;
use crate::command::{CommandError, ErrorKind};
use crate::query::{Bounds, InstanceInfo};

impl Document {
    /// Makes a group (`name` None) or a named component from `ids` and selects its instance.
    pub(super) fn group_changes(
        &mut self,
        ids: &[EntityId],
        name: Option<&String>,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge, face or instance",
            ));
        }
        let (kind, name) = match name {
            None => (DefinitionKind::Group, ""),
            Some(n) if n.trim().is_empty() => {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    "a component needs a name",
                ));
            }
            Some(n) => (DefinitionKind::Component, n.as_str()),
        };
        let parts = self.parts(ids)?;
        let mesh = self.active();
        let (instance, change) = self
            .scene
            .make_group(
                mesh,
                &parts.edges,
                &parts.faces,
                &parts.instances,
                kind,
                name,
            )
            .map_err(|e| invalid(e, ids))?;
        self.selection = BTreeSet::from([EntityId::Instance(instance)]);
        Ok(change)
    }

    /// Explodes instances and selects what comes out of them.
    pub(super) fn explode_changes(
        &mut self,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one instance",
            ));
        }
        let others: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| !matches!(id, EntityId::Instance(_)))
            .collect();
        if !others.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "only instances can be exploded",
            )
            .with_ids(others));
        }
        let parts = self.parts(ids)?;
        let mesh = self.active();
        let mut done = Vec::new();
        for i in &parts.instances {
            match self.scene.explode(mesh, *i) {
                Ok(c) => done.push(c),
                Err(e) => {
                    for c in done.iter().rev() {
                        self.scene.apply(&c.inverse());
                    }
                    return Err(invalid(e, ids));
                }
            }
        }
        let change = SceneChange::join(done);
        self.selection = change
            .summary()
            .created
            .into_iter()
            .filter(|id| !matches!(id, EntityId::Vertex(_)) && self.exists(*id))
            .collect();
        Ok(change)
    }

    /// Opens an instance of the open context for editing.
    pub(super) fn open_instance(&mut self, id: EntityId) -> Result<(), CommandError> {
        let EntityId::Instance(i) = id else {
            return Err(
                CommandError::new(ErrorKind::InvalidInput, "only instances can be opened")
                    .with_ids(vec![id]),
            );
        };
        if self.model().instance(i).is_none() {
            return Err(
                CommandError::new(ErrorKind::NotFound, format!("no entity {id}"))
                    .with_ids(vec![id]),
            );
        }
        self.context.push(i);
        self.selection.clear();
        Ok(())
    }

    /// Closes the open instance.
    pub(super) fn close_instance(&mut self) -> Result<(), CommandError> {
        if self.context.pop().is_none() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "no instance is open",
            ));
        }
        self.selection.clear();
        Ok(())
    }

    pub(super) fn instance_info(&self, id: kernel::InstanceId) -> InstanceInfo {
        let i = self.model().instance(id).unwrap();
        let d = self.scene.definition(i.definition).unwrap();
        InstanceInfo {
            id: EntityId::Instance(id),
            definition: i.definition,
            name: d.name.clone(),
            kind: d.kind,
            transform: i.transform.to_cols_array(),
            bounds: self.instance_box(id).map(|[lo, hi]| Bounds {
                min: lo.to_array(),
                max: hi.to_array(),
            }),
        }
    }
}

fn invalid(e: KernelError, ids: &[EntityId]) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, e.to_string()).with_ids(ids.to_vec())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p session`
Expected: all pass, including 11 in `groups`.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 418 tests pass. The app, tools and api need no changes here: `Document::model()` keeps its signature.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/session/tests/groups.rs crates/session/tests/commands.rs crates/session/src/command.rs crates/session/src/query.rs crates/session/src/lib.rs crates/session/src/document.rs crates/session/src/document/groups.rs
git commit -m "Group, component and context commands in the session"
```

### Task 4: Draw the scene, the selection and the open context

**Files:**
- Modify: `crates/render/src/scene.rs`, `crates/render/src/renderer.rs`, `crates/render/src/snapshot.rs`, `crates/app/src/app.rs`, `crates/app/src/host.rs`, `crates/app/src/info.rs`, `crates/api/src/headless.rs`
- Test: unit tests in `crates/render/src/scene.rs` and `crates/app/src/info.rs`; `crates/render/tests/headless.rs`, `crates/render/tests/snapshot.rs` follow the new signatures

**Interfaces:**
- Consumes (Tasks 1-3):
  - `Scene::{resolve, bounds, mesh, definition}`;
  - `Document::{scene, context, selection}`.
- Produces:
  - `render::scene::scene_geometry(&Scene, context: &[InstanceId], selection: &BTreeSet<EntityId>) -> ModelGeometry`, replacing `model_geometry`;
  - `render::scene::{FADED_EDGE_COLOR, CONTEXT_BOX_COLOR}`;
  - `Renderer::set_scene(&mut self, &Scene, &[InstanceId], &BTreeSet<EntityId>)`, replacing `set_model`;
  - `Snapshotter::capture(&mut self, &Scene, &[InstanceId], &BTreeSet<EntityId>, &Camera, u32, u32)`.

- [ ] **Step 1: Write the failing tests**

The new render tests replace the `tests` module at the end of `crates/render/src/scene.rs`. The three existing tests move to a `geometry` helper; three are new:
- `every_instance_is_drawn_where_it_is_placed`;
- `a_selected_instance_lights_up_what_it_places`;
- `an_open_instance_fades_the_rest_and_is_boxed`.

The new Entity Info test is `describes_groups_and_components` in `crates/app/src/info.rs`. Both test modules are part of the Step 3 diffs of those files. Apply the test-module parts first.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p render --lib`
Expected: compile errors: cannot find function `scene_geometry`, values `FADED_EDGE_COLOR` and `CONTEXT_BOX_COLOR`, types `Scene`, `InstanceId`, `MeshId`.

Run: `cargo test -p app --bin app info` (before changing `entity_info`)
Expected: `describes_groups_and_components` fails: `left: ["Nothing selected"]`, `right: ["1 group"]`.

- [ ] **Step 3: Implement**

Apply to `crates/render/src/scene.rs`:
```diff
--- a/crates/render/src/scene.rs
+++ b/crates/render/src/scene.rs
@@ -1,10 +1,11 @@
-//! GPU vertex data built from a kernel model.
+//! GPU vertex data built from a kernel scene.
 
 use std::collections::BTreeSet;
 
 use bytemuck::{Pod, Zeroable};
-use kernel::{EntityId, Model};
+use glam::{DAffine3, DVec3};
+use kernel::{EntityId, InstanceId, MeshId, Scene};
 
 use crate::grid::LineVertex;
 
 #[repr(C)]
@@ -18,76 +19,203 @@ pub struct FaceVertex {
 
 pub const EDGE_COLOR: [f32; 4] = [0.08, 0.08, 0.08, 1.0];
 pub const SELECTED_EDGE_COLOR: [f32; 4] = [0.10, 0.35, 0.95, 1.0];
 
+/// Edges outside the open instance, while one is open.
+pub const FADED_EDGE_COLOR: [f32; 4] = [0.62, 0.62, 0.62, 1.0];
+/// The box around the open instance.
+pub const CONTEXT_BOX_COLOR: [f32; 4] = [0.45, 0.45, 0.45, 1.0];
+
 /// Triangles for every face and line pairs for every edge.
 #[derive(Debug, Default, PartialEq)]
 pub struct ModelGeometry {
     pub faces: Vec<FaceVertex>,
     pub edges: Vec<LineVertex>,
 }
 
-/// Geometry for `model`, with the entities in `selection` highlighted.
-pub fn model_geometry(model: &Model, selection: &BTreeSet<EntityId>) -> ModelGeometry {
+/// Geometry for the whole scene in root coordinates, every instance placed by its transform.
+/// `context` is the path of open instances (see `session::Document::context`) and `selection`
+/// holds ids of the open context: selected entities there are highlighted, and a selected
+/// instance highlights everything it places. While an instance is open, edges outside it are
+/// faded and its bounding box is outlined.
+pub fn scene_geometry(
+    scene: &Scene,
+    context: &[InstanceId],
+    selection: &BTreeSet<EntityId>,
+) -> ModelGeometry {
     let mut g = ModelGeometry::default();
-    for (id, face) in model.faces() {
-        let normal = face.normal.as_vec3().to_array();
-        let selected = if selection.contains(&EntityId::Face(id)) {
-            1.0
-        } else {
-            0.0
+    let mut walk = Walk {
+        scene,
+        context,
+        selection,
+        path: Vec::new(),
+        g: &mut g,
+    };
+    walk.mesh(MeshId::Root, DAffine3::IDENTITY, false);
+    if !context.is_empty()
+        && let Some((mesh, t)) = scene.resolve(context)
+        && let Some([lo, hi]) = scene.bounds(mesh)
+    {
+        let corner = |k: usize| {
+            let c = DVec3::new(
+                if k & 1 == 0 { lo.x } else { hi.x },
+                if k & 2 == 0 { lo.y } else { hi.y },
+                if k & 4 == 0 { lo.z } else { hi.z },
+            );
+            t.transform_point3(c).as_vec3().to_array()
         };
-        for tri in model.triangulate(id).unwrap_or_default() {
-            for p in tri {
-                g.faces.push(FaceVertex {
-                    pos: p.as_vec3().to_array(),
-                    normal,
-                    selected,
+        for (a, b) in BOX_EDGES {
+            for k in [a, b] {
+                g.edges.push(LineVertex {
+                    pos: corner(k),
+                    color: CONTEXT_BOX_COLOR,
                 });
             }
         }
     }
-    for (id, edge) in model.edges() {
-        let color = if selection.contains(&EntityId::Edge(id)) {
-            SELECTED_EDGE_COLOR
-        } else {
-            EDGE_COLOR
+    g
+}
+
+/// Corner pairs of a box's edges; corner k has bit 0 for x, bit 1 for y, bit 2 for z.
+const BOX_EDGES: [(usize, usize); 12] = [
+    (0, 1),
+    (2, 3),
+    (4, 5),
+    (6, 7),
+    (0, 2),
+    (1, 3),
+    (4, 6),
+    (5, 7),
+    (0, 4),
+    (1, 5),
+    (2, 6),
+    (3, 7),
+];
+
+struct Walk<'a> {
+    scene: &'a Scene,
+    context: &'a [InstanceId],
+    selection: &'a BTreeSet<EntityId>,
+    /// Instances from the root down to the mesh being walked.
+    path: Vec<InstanceId>,
+    g: &'a mut ModelGeometry,
+}
+
+impl Walk<'_> {
+    /// Adds a mesh placed by `t`; `lit` highlights all of it (it is inside a selected instance).
+    fn mesh(&mut self, id: MeshId, t: DAffine3, lit: bool) {
+        let Some(model) = self.scene.mesh(id) else {
+            return;
         };
-        for v in edge.v {
-            if let Some(p) = model.position(v) {
-                g.edges.push(LineVertex {
-                    pos: p.as_vec3().to_array(),
-                    color,
-                });
+        // Selection ids refer to the open context's placement only, not to other placements
+        // of the same component.
+        let open = self.path == self.context;
+        let inside = self.path.starts_with(self.context);
+        let selected = |e: EntityId| lit || (open && self.selection.contains(&e));
+        for (fid, face) in model.faces() {
+            let normal = t
+                .transform_vector3(face.normal)
+                .normalize()
+                .as_vec3()
+                .to_array();
+            let flag = if selected(EntityId::Face(fid)) {
+                1.0
+            } else {
+                0.0
+            };
+            for tri in model.triangulate(fid).unwrap_or_default() {
+                for p in tri {
+                    self.g.faces.push(FaceVertex {
+                        pos: t.transform_point3(p).as_vec3().to_array(),
+                        normal,
+                        selected: flag,
+                    });
+                }
+            }
+        }
+        for (eid, edge) in model.edges() {
+            let color = if selected(EntityId::Edge(eid)) {
+                SELECTED_EDGE_COLOR
+            } else if inside {
+                EDGE_COLOR
+            } else {
+                FADED_EDGE_COLOR
+            };
+            for v in edge.v {
+                if let Some(p) = model.position(v) {
+                    self.g.edges.push(LineVertex {
+                        pos: t.transform_point3(p).as_vec3().to_array(),
+                        color,
+                    });
+                }
             }
         }
+        for (iid, instance) in model.instances() {
+            let lit = selected(EntityId::Instance(iid));
+            self.path.push(iid);
+            self.mesh(
+                MeshId::Definition(instance.definition),
+                t * instance.transform,
+                lit,
+            );
+            self.path.pop();
+        }
     }
-    g
 }
 
 #[cfg(test)]
 mod tests {
     use super::*;
-    use glam::DVec3;
+    use kernel::{DefinitionKind, Model};
 
     fn square() -> Model {
         let mut m = Model::new();
         let pts = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
         m.add_polygon(&pts).unwrap();
         m
     }
 
+    fn geometry(m: Model, selection: &BTreeSet<EntityId>) -> ModelGeometry {
+        scene_geometry(&Scene::from_root(m), &[], selection)
+    }
+
+    /// The square made a component, copied 3 m along x. Returns the scene and both instances.
+    fn two_squares() -> (Scene, InstanceId, InstanceId) {
+        let mut s = Scene::from_root(square());
+        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
+        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
+        let (first, _) = s
+            .make_group(
+                MeshId::Root,
+                &edges,
+                &faces,
+                &[],
+                DefinitionKind::Component,
+                "Tile",
+            )
+            .unwrap();
+        s.copy_instances(MeshId::Root, &[first], DVec3::new(3.0, 0.0, 0.0), 1)
+            .unwrap();
+        let second = s
+            .root()
+            .instances()
+            .map(|(id, _)| id)
+            .find(|id| *id != first)
+            .unwrap();
+        (s, first, second)
+    }
+
     #[test]
     fn empty_model_has_no_geometry() {
         assert_eq!(
-            model_geometry(&Model::new(), &BTreeSet::new()),
+            geometry(Model::new(), &BTreeSet::new()),
             ModelGeometry::default()
         );
     }
 
     #[test]
     fn square_gives_two_triangles_and_four_edges() {
-        let g = model_geometry(&square(), &BTreeSet::new());
+        let g = geometry(square(), &BTreeSet::new());
         assert_eq!(g.faces.len(), 6);
         assert!(
             g.faces
                 .iter()
@@ -102,14 +230,56 @@ mod tests {
         let m = square();
         let face = m.faces().next().unwrap().0;
         let edge = m.edges().next().unwrap().0;
         let selection = BTreeSet::from([EntityId::Face(face), EntityId::Edge(edge)]);
-        let g = model_geometry(&m, &selection);
+        let g = geometry(m, &selection);
         assert!(g.faces.iter().all(|v| v.selected == 1.0));
         let blue = g
             .edges
             .iter()
             .filter(|v| v.color == SELECTED_EDGE_COLOR)
             .count();
         assert_eq!(blue, 2, "one edge, two vertices");
     }
+
+    #[test]
+    fn every_instance_is_drawn_where_it_is_placed() {
+        let (s, _, _) = two_squares();
+        let g = scene_geometry(&s, &[], &BTreeSet::new());
+        assert_eq!(g.faces.len(), 12);
+        assert_eq!(g.edges.len(), 16);
+        let max_x = g.faces.iter().map(|v| v.pos[0]).fold(f32::MIN, f32::max);
+        assert_eq!(max_x, 4.0);
+    }
+
+    #[test]
+    fn a_selected_instance_lights_up_what_it_places() {
+        let (s, first, _) = two_squares();
+        let selection = BTreeSet::from([EntityId::Instance(first)]);
+        let g = scene_geometry(&s, &[], &selection);
+        let lit: Vec<_> = g.faces.iter().filter(|v| v.selected == 1.0).collect();
+        assert_eq!(lit.len(), 6);
+        assert!(lit.iter().all(|v| v.pos[0] <= 1.0), "only the first square");
+        let blue = g
+            .edges
+            .iter()
+            .filter(|v| v.color == SELECTED_EDGE_COLOR)
+            .count();
+        assert_eq!(blue, 8);
+    }
+
+    #[test]
+    fn an_open_instance_fades_the_rest_and_is_boxed() {
+        let (s, first, _) = two_squares();
+        let d = s.root().instance(first).unwrap().definition;
+        let face = s.definition(d).unwrap().mesh.faces().next().unwrap().0;
+        let selection = BTreeSet::from([EntityId::Face(face)]);
+        let g = scene_geometry(&s, &[first], &selection);
+        let lit: Vec<_> = g.faces.iter().filter(|v| v.selected == 1.0).collect();
+        assert_eq!(lit.len(), 6, "the face is lit in the open placement only");
+        assert!(lit.iter().all(|v| v.pos[0] <= 1.0));
+        let count = |c: [f32; 4]| g.edges.iter().filter(|v| v.color == c).count();
+        assert_eq!(count(EDGE_COLOR), 8, "the open square's edges");
+        assert_eq!(count(FADED_EDGE_COLOR), 8, "the other square's edges");
+        assert_eq!(count(CONTEXT_BOX_COLOR), 24, "twelve box edges");
+    }
 }
```

Apply to `crates/render/src/renderer.rs`:
```diff
--- a/crates/render/src/renderer.rs
+++ b/crates/render/src/renderer.rs
@@ -1,12 +1,12 @@
 use std::collections::BTreeSet;
 
-use kernel::{EntityId, Model};
+use kernel::{EntityId, InstanceId, Scene};
 use session::Camera;
 use wgpu::util::DeviceExt;
 
 use crate::grid::{BACKGROUND, LineVertex, flat_ground_lines, vertical_axis_lines};
-use crate::scene::{FaceVertex, model_geometry};
+use crate::scene::{FaceVertex, scene_geometry};
 
 pub const SAMPLE_COUNT: u32 = 4;
 pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
 
@@ -253,12 +253,18 @@ impl Renderer {
             targets: None,
         }
     }
 
-    /// Rebuilds the face and edge buffers from the model, highlighting `selection`. Call when
-    /// the model or the selection changes.
-    pub fn set_model(&mut self, model: &Model, selection: &BTreeSet<EntityId>) {
-        let g = model_geometry(model, selection);
+    /// Rebuilds the face and edge buffers from the scene, with the open context `context` and
+    /// `selection` shown (see `scene_geometry`). Call when the model, the open context or the
+    /// selection changes.
+    pub fn set_scene(
+        &mut self,
+        scene: &Scene,
+        context: &[InstanceId],
+        selection: &BTreeSet<EntityId>,
+    ) {
+        let g = scene_geometry(scene, context, selection);
         self.faces = mesh(&self.device, "faces", &g.faces);
         self.edges = mesh(&self.device, "edges", &g.edges);
     }
 
```

Apply to `crates/render/src/snapshot.rs`:
```diff
--- a/crates/render/src/snapshot.rs
+++ b/crates/render/src/snapshot.rs
@@ -1,9 +1,9 @@
 //! Offscreen rendering to RGBA pixels, for headless mode and the API's `render_view`.
 
 use std::collections::BTreeSet;
 
-use kernel::{EntityId, Model};
+use kernel::{EntityId, InstanceId, Scene};
 use session::Camera;
 
 use crate::Renderer;
 
@@ -36,14 +36,15 @@ impl Snapshotter {
             renderer: Renderer::new(device, queue, FORMAT),
         }
     }
 
-    /// Renders `model` as seen by `camera` (its viewport is replaced by the image size) into
-    /// tightly packed RGBA8 pixels, top row first, with `selection` highlighted. None if the
-    /// GPU readback fails.
+    /// Renders `scene` as seen by `camera` (its viewport is replaced by the image size) into
+    /// tightly packed RGBA8 pixels, top row first, with the open context and `selection` shown.
+    /// None if the GPU readback fails.
     pub fn capture(
         &mut self,
-        model: &Model,
+        scene: &Scene,
+        context: &[InstanceId],
         selection: &BTreeSet<EntityId>,
         camera: &Camera,
         width: u32,
         height: u32,
@@ -67,9 +68,9 @@ impl Snapshotter {
             usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
             view_formats: &[],
         });
         let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
-        self.renderer.set_model(model, selection);
+        self.renderer.set_scene(scene, context, selection);
         self.renderer.render(&view, (width, height), &camera, &[]);
 
         // Texture-to-buffer copies need rows padded to a multiple of 256 bytes.
         let row = width * 4;
```

Apply to `crates/render/tests/headless.rs`:
```diff
--- a/crates/render/tests/headless.rs
+++ b/crates/render/tests/headless.rs
@@ -1,8 +1,8 @@
 use std::collections::BTreeSet;
 
 use glam::{DVec2, DVec3};
-use kernel::{EntityId, Model};
+use kernel::{EntityId, Model, Scene};
 use render::grid::{BACKGROUND, LineVertex};
 use render::{Renderer, Viewport};
 use session::Camera;
 
@@ -60,9 +60,9 @@ fn render_with(
         view_formats: &[],
     });
     let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
     let mut renderer = Renderer::new(&device, &queue, format);
-    renderer.set_model(model, selection);
+    renderer.set_scene(&Scene::from_root(model.clone()), &[], selection);
     renderer.render_in(&view, (W, H), viewport, camera, overlay);
 
     let buffer = device.create_buffer(&wgpu::BufferDescriptor {
         label: Some("readback"),
```

Apply to `crates/render/tests/snapshot.rs`:
```diff
--- a/crates/render/tests/snapshot.rs
+++ b/crates/render/tests/snapshot.rs
@@ -1,8 +1,8 @@
 use std::collections::BTreeSet;
 
 use glam::{DVec2, DVec3};
-use kernel::Model;
+use kernel::{Model, Scene};
 use render::Snapshotter;
 use render::grid::BACKGROUND;
 use session::Camera;
 
@@ -29,9 +29,16 @@ fn capture_returns_tightly_packed_rows_for_any_width() {
     let mut camera = Camera::new(DVec2::new(10.0, 10.0));
     camera.eye = DVec3::new(0.0, -6.0, 1.5);
     camera.target = DVec3::new(0.0, 0.0, 1.5);
     let px = snap
-        .capture(&ground_square(), &BTreeSet::new(), &camera, w, h)
+        .capture(
+            &Scene::from_root(ground_square()),
+            &[],
+            &BTreeSet::new(),
+            &camera,
+            w,
+            h,
+        )
         .expect("capture");
     assert_eq!(px.len(), (w * h * 4) as usize);
 
     let at = |x: u32, y: u32| {
@@ -69,9 +76,9 @@ fn capture_handles_padding_edge_widths() {
     let h = 8;
     // Widths around the 64-pixel (256-byte) row alignment boundary.
     for w in [1u32, 63, 64, 65] {
         let px = snap
-            .capture(&Model::new(), &BTreeSet::new(), &camera, w, h)
+            .capture(&Scene::new(), &[], &BTreeSet::new(), &camera, w, h)
             .expect("capture");
         assert_eq!(px.len(), (w * h * 4) as usize, "width {w}");
         // At width 1 the only column is the image center, where the renderer draws the
         // axis lines, so the sky check only applies to wider images.
```

Apply to `crates/app/src/app.rs`:
```diff
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -4,9 +4,9 @@ use std::sync::Arc;
 use std::time::{Duration, Instant};
 
 use api::{Hub, listen_unix};
 use glam::DVec2;
-use kernel::EntityId;
+use kernel::{EntityId, InstanceId};
 use render::{Renderer, Snapshotter, Viewport};
 use tools::Axis;
 use winit::application::ApplicationHandler;
 use winit::dpi::LogicalSize;
@@ -74,10 +74,11 @@ struct State {
     renderer: Renderer,
     snapshotter: Snapshotter,
     editor: Editor,
     gui: Gui,
-    /// Document revision and selection the renderer's model buffers were built from.
-    drawn: Option<(u64, BTreeSet<EntityId>)>,
+    /// Document revision, selection and open context the renderer's model buffers were built
+    /// from.
+    drawn: Option<(u64, BTreeSet<EntityId>, Vec<InstanceId>)>,
     modifiers: ModifiersState,
     /// Where the 3D view sits in the window (physical pixels), from the last frame's layout.
     view: Viewport,
     /// Last cursor position in window coordinates (physical pixels).
@@ -229,11 +230,16 @@ impl State {
                 .resize(DVec2::new(v.width.max(1) as f64, v.height.max(1) as f64));
         }
 
         let doc = self.editor.doc();
-        let drawn = (doc.revision(), doc.selection().clone());
+        let drawn = (
+            doc.revision(),
+            doc.selection().clone(),
+            doc.context().to_vec(),
+        );
         if self.drawn.as_ref() != Some(&drawn) {
-            self.renderer.set_model(doc.model(), doc.selection());
+            self.renderer
+                .set_scene(doc.scene(), doc.context(), doc.selection());
             self.drawn = Some(drawn);
         }
         self.renderer.render_in(
             &target,
@@ -263,13 +269,15 @@ impl State {
         }
         acted
     }
 
-    /// Handles waiting API requests; redraws if they changed the model, selection or camera.
+    /// Handles waiting API requests; redraws if they changed the model, selection, open
+    /// context or camera.
     fn process_api(&mut self, hub: &mut Hub) {
         let before = (
             self.editor.doc().revision(),
             self.editor.doc().selection().clone(),
+            self.editor.doc().context().to_vec(),
             self.editor.doc().camera.clone(),
             self.editor.overlay(),
         );
         let mut host = WindowHost {
@@ -279,8 +287,9 @@ impl State {
         hub.process(&mut host);
         if (
             self.editor.doc().revision(),
             self.editor.doc().selection().clone(),
+            self.editor.doc().context().to_vec(),
             self.editor.doc().camera.clone(),
             self.editor.overlay(),
         ) != before
         {
```

Apply to `crates/app/src/host.rs`:
```diff
--- a/crates/app/src/host.rs
+++ b/crates/app/src/host.rs
@@ -30,8 +30,14 @@ impl Host for WindowHost<'_> {
     }
 
     fn render_view(&mut self, camera: &Camera, width: u32, height: u32) -> Option<Vec<u8>> {
         let doc = self.editor.doc();
-        self.snapshotter
-            .capture(doc.model(), doc.selection(), camera, width, height)
+        self.snapshotter.capture(
+            doc.scene(),
+            doc.context(),
+            doc.selection(),
+            camera,
+            width,
+            height,
+        )
     }
 }
```

Apply to `crates/app/src/info.rs`:
```diff
--- a/crates/app/src/info.rs
+++ b/crates/app/src/info.rs
@@ -1,28 +1,41 @@
 //! Text for the Entity Info panel: what is selected and its measurements.
 
-use kernel::{EntityId, Model};
+use kernel::{DefinitionKind, EntityId, Model};
 use session::Document;
 use tools::vcb::format_length;
 
-/// Lines describing the selection: counts, plus length, area or normal for a single entity.
+/// Lines describing the selection: counts, plus length, area or normal for a single entity,
+/// or the kind and name of a single instance.
 pub fn entity_info(doc: &Document) -> Vec<String> {
     let m = doc.model();
-    let (mut edges, mut faces) = (Vec::new(), Vec::new());
+    let (mut edges, mut faces, mut instances) = (Vec::new(), Vec::new(), Vec::new());
     for id in doc.selection() {
         match id {
             EntityId::Edge(e) => edges.push(*e),
             EntityId::Face(f) => faces.push(*f),
-            EntityId::Vertex(_) | EntityId::Instance(_) => {}
+            EntityId::Instance(i) => instances.push(*i),
+            EntityId::Vertex(_) => {}
         }
     }
+    if let ([], [], [i]) = (&edges[..], &faces[..], &instances[..]) {
+        let d = m
+            .instance(*i)
+            .and_then(|i| doc.scene().definition(i.definition));
+        return match d {
+            Some(d) if d.kind == DefinitionKind::Component => {
+                vec!["1 component".into(), format!("Name: {}", d.name)]
+            }
+            _ => vec!["1 group".into()],
+        };
+    }
     match (&edges[..], &faces[..]) {
-        ([], []) => vec!["Nothing selected".into()],
-        ([e], []) => vec![
+        ([], []) if instances.is_empty() => vec!["Nothing selected".into()],
+        ([e], []) if instances.is_empty() => vec![
             "1 edge".into(),
             format!("Length: {}", format_length(edge_length(m, *e))),
         ],
-        ([], [f]) => {
+        ([], [f]) if instances.is_empty() => {
             let face = m.face(*f).unwrap();
             let n = face.normal.to_array().map(number);
             vec![
                 "1 face".into(),
@@ -37,8 +50,11 @@ pub fn entity_info(doc: &Document) -> Vec<String> {
             }
             if !faces.is_empty() {
                 parts.push(count(faces.len(), "face"));
             }
+            if !instances.is_empty() {
+                parts.push(count(instances.len(), "instance"));
+            }
             vec![parts.join(", ")]
         }
     }
 }
@@ -101,5 +117,40 @@ mod tests {
         assert_eq!(entity_info(&d), ["1 edge", "Length: 2 m"]);
         select(&mut d, &["e2", "e4", "f8"]);
         assert_eq!(entity_info(&d), ["2 edges, 1 face"]);
     }
+
+    #[test]
+    fn describes_groups_and_components() {
+        let mut d = doc_with_square();
+        let ids = ["e2", "e4", "e6", "e7", "f8"]
+            .map(|s| s.parse().unwrap())
+            .to_vec();
+        d.execute(Command::MakeGroup { ids }).unwrap();
+        assert_eq!(entity_info(&d), ["1 group"]);
+        let group = *d.selection().first().unwrap();
+        d.execute(Command::MakeComponent {
+            ids: vec![group],
+            name: "Table".into(),
+        })
+        .unwrap();
+        assert_eq!(entity_info(&d), ["1 component", "Name: Table"]);
+        d.execute(Command::AddEdge {
+            start: [5.0, 0.0, 0.0],
+            end: [6.0, 0.0, 0.0],
+        })
+        .unwrap();
+        let edge = d
+            .model()
+            .edges()
+            .map(|(id, _)| EntityId::Edge(id))
+            .next()
+            .unwrap();
+        let component = *d.selection().first().unwrap();
+        d.execute(Command::Select {
+            ids: vec![edge, component],
+            mode: SelectMode::Replace,
+        })
+        .unwrap();
+        assert_eq!(entity_info(&d), ["1 edge, 1 instance"]);
+    }
 }
```

Apply to `crates/api/src/headless.rs`:
```diff
--- a/crates/api/src/headless.rs
+++ b/crates/api/src/headless.rs
@@ -49,10 +49,17 @@ impl Host for HeadlessHost {
         if !self.gpu_checked {
             self.gpu_checked = true;
             self.snapshotter = Snapshotter::new_headless();
         }
-        let (model, selection) = (self.doc.model(), self.doc.selection());
-        self.snapshotter
-            .as_mut()
-            .and_then(|s| s.capture(model, selection, camera, width, height))
+        let doc = &self.doc;
+        self.snapshotter.as_mut().and_then(|s| {
+            s.capture(
+                doc.scene(),
+                doc.context(),
+                doc.selection(),
+                camera,
+                width,
+                height,
+            )
+        })
     }
 }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p render && cargo test -p app --bin app info`
Expected: all pass. The GPU tests skip with "no GPU adapter" where there is none.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 422 tests pass.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/render/src/scene.rs crates/render/src/renderer.rs crates/render/src/snapshot.rs crates/render/tests/headless.rs crates/render/tests/snapshot.rs crates/app/src/app.rs crates/app/src/host.rs crates/app/src/info.rs crates/api/src/headless.rs
git commit -m "Draw groups and components and the open context"
```

### Task 5: API test, agent docs, spec amendment, visual check

**Files:**
- Modify: `docs/agent-api.md`, `docs/superpowers/specs/2026-10-10-tenon-design.md`
- Test: `crates/api/tests/dispatch.rs`

**Interfaces:**
- Consumes: everything above, over JSON-RPC (`HeadlessHost`).
- Produces: documentation only.

- [ ] **Step 1: Write the failing tests**

Apply to `crates/api/tests/dispatch.rs`:
```diff
--- a/crates/api/tests/dispatch.rs
+++ b/crates/api/tests/dispatch.rs
@@ -128,15 +128,26 @@ fn describe_lists_methods_and_schemas() {
         "select",
         "undo",
         "redo",
         "set_camera",
+        "make_group",
+        "make_component",
+        "explode",
+        "open_instance",
+        "close_instance",
     ] {
         assert!(
             command_schema.contains(variant),
             "Command schema lacks {variant}"
         );
     }
-    assert!(d["schemas"]["Query"].to_string().contains("events"));
+    let query_schema = d["schemas"]["Query"].to_string();
+    for variant in ["events", "definitions", "context"] {
+        assert!(
+            query_schema.contains(variant),
+            "Query schema lacks {variant}"
+        );
+    }
 }
 
 #[test]
 fn rectangle_and_push_pull_build_a_box() {
@@ -377,4 +388,107 @@ fn ids_must_be_string_number_or_null() {
         let r: Value = serde_json::from_str(&handle(&mut h, &line).unwrap()).unwrap();
         assert!(r.get("result").is_some());
     }
 }
+
+#[test]
+fn components_over_json_rpc() {
+    let mut h = HeadlessHost::new();
+    call(
+        &mut h,
+        "execute",
+        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
+    );
+    call(
+        &mut h,
+        "execute",
+        json!({"type": "push_pull", "face": "f8", "distance": 1.0}),
+    );
+    let all = call(
+        &mut h,
+        "query",
+        json!({"type": "entities", "kinds": ["edge", "face"]}),
+    )["result"]
+        .clone();
+    let ids: Vec<Value> = all["edges"]
+        .as_array()
+        .unwrap()
+        .iter()
+        .chain(all["faces"].as_array().unwrap())
+        .map(|e| e["id"].clone())
+        .collect();
+    let r = call(
+        &mut h,
+        "execute",
+        json!({"type": "make_component", "ids": ids, "name": "Crate"}),
+    );
+    let first = call(&mut h, "query", json!({"type": "selection"}))["result"]["ids"][0].clone();
+    assert!(
+        r["result"]["changes"]["created"]
+            .as_array()
+            .unwrap()
+            .contains(&first)
+    );
+    call(
+        &mut h,
+        "execute",
+        json!({"type": "copy", "ids": [first], "offset": [3, 0, 0]}),
+    );
+
+    call(
+        &mut h,
+        "execute",
+        json!({"type": "open_instance", "id": first}),
+    );
+    let faces = call(
+        &mut h,
+        "query",
+        json!({"type": "entities", "kinds": ["face"]}),
+    );
+    let top = faces["result"]["faces"]
+        .as_array()
+        .unwrap()
+        .iter()
+        .find(|f| f["normal"] == json!([0.0, 0.0, 1.0]))
+        .unwrap()["id"]
+        .clone();
+    let r = call(
+        &mut h,
+        "execute",
+        json!({"type": "push_pull", "face": top, "distance": 1.0}),
+    );
+    assert!(r["result"].is_object(), "{r}");
+    let png = call(&mut h, "render_view", json!({"width": 64, "height": 64}));
+    assert!(
+        png["result"]["png_base64"].is_string()
+            || png["error"]["code"] == codes::RENDER_UNAVAILABLE,
+        "{png}"
+    );
+    call(&mut h, "execute", json!({"type": "close_instance"}));
+
+    let placed = call(
+        &mut h,
+        "query",
+        json!({"type": "entities", "kinds": ["instance"]}),
+    );
+    let tops: Vec<Value> = placed["result"]["instances"]
+        .as_array()
+        .unwrap()
+        .iter()
+        .map(|i| i["bounds"]["max"][2].clone())
+        .collect();
+    assert_eq!(tops, [json!(2.0), json!(2.0)], "both copies grew");
+    let defs = call(&mut h, "query", json!({"type": "definitions"}));
+    assert_eq!(defs["result"]["definitions"][0]["instances"], 2);
+
+    let r = call(
+        &mut h,
+        "execute",
+        json!({"type": "explode", "ids": [first]}),
+    );
+    assert!(r["result"].is_object(), "{r}");
+    let stats = call(&mut h, "query", json!({"type": "stats"}))["result"].clone();
+    assert_eq!(
+        (stats["faces"].clone(), stats["instances"].clone()),
+        (json!(6), json!(1))
+    );
+}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p api --test dispatch`
Expected: PASS. Tasks 3-4 already made these reachable; this test pins the wire format end to end: the `describe` schemas, a component edited through an open instance, `render_view` with an instance open, and explode.

- [ ] **Step 3: Implement**

Apply to `docs/agent-api.md`:
~~~~diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -37,37 +37,45 @@ all commands and results.
 Queries:
 
 | Query | Result |
 |---|---|
-| `{"type": "entities"}` | every vertex, edge and face, plus `revision` |
-| `{"type": "entities", "kinds": ["face"]}` | only the listed kinds (`vertex`, `edge`, `face`) |
+| `{"type": "entities"}` | every vertex, edge, face and instance of the open context, plus `revision` |
+| `{"type": "entities", "kinds": ["face"]}` | only the listed kinds (`vertex`, `edge`, `face`, `instance`) |
 | `{"type": "entities", "ids": ["v0", "e2"]}` | only the listed ids; unknown ids are absent, not an error |
-| `{"type": "entities", "within": {"min": [0,0,0], "max": [2,2,1]}}` | only entities whose vertices all lie in the box (bounds included). `ids`, `kinds` and `within` combine (all must match); empty lists mean no filter |
+| `{"type": "entities", "within": {"min": [0,0,0], "max": [2,2,1]}}` | only entities whose vertices (an instance: its bounding box) all lie in the box, bounds included. `ids`, `kinds` and `within` combine (all must match); empty lists mean no filter |
 | `{"type": "entity", "id": "f7"}` | one entity, or a not-found error |
-| `{"type": "stats"}`, `{"type": "camera"}`, `{"type": "history"}` | counts and revision, camera, undo/redo depth |
-| `{"type": "selection"}` | the selected edge and face ids, plus `revision` |
+| `{"type": "stats"}`, `{"type": "camera"}`, `{"type": "history"}` | counts for the open context (plus the number of definitions) and revision, camera, undo/redo depth |
+| `{"type": "selection"}` | the selected edge, face and instance ids, plus `revision` |
+| `{"type": "definitions"}` | every group and component definition: `id`, `name`, `kind`, how many `instances` place it, `edges`, `faces` |
+| `{"type": "context"}` | the open context: `path` (open instances from the root, empty at the root) and the `definition` being edited |
 | `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |
 
 Unknown fields in commands, queries and params are rejected with `-32602` (a typo fails instead
 of silently doing nothing).
 
-Commands: `add_edge`, `add_rectangle`, `push_pull`, `erase`, `move`, `copy`, `select`, `undo`,
-`redo`, `set_camera`. Units are
-meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m. Entity ids are strings: `v12`
-(vertex), `e3` (edge), `f7` (face).
+Commands: `add_edge`, `add_rectangle`, `push_pull`, `erase`, `move`, `copy`, `make_group`,
+`make_component`, `explode`, `open_instance`, `close_instance`, `select`, `undo`, `redo`,
+`set_camera`. Units are meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m.
+Entity ids are strings: `v12` (vertex), `e3` (edge), `f7` (face), `i4` (instance of a group or
+component). Definitions are `d5`. Ids are unique across the whole model, including inside
+groups and components.
 
 Modeling commands, each one undo step:
 
 | Command | Effect |
 |---|---|
 | `{"type": "add_edge", "start": [x,y,z], "end": [x,y,z]}` | A straight edge. It merges with nearby vertices, splits edges it crosses, and closed coplanar loops become faces. |
 | `{"type": "add_rectangle", "corner": [0,0,0], "opposite": [2,3,0]}` | A rectangle with sides along the axes. Without `normal` the corners must differ in exactly two coordinates. With `"normal": [nx,ny,nz]` it lies in the plane through `corner` with that normal; `opposite` must be in that plane. Sides then run along u = `Z x normal` (horizontal; X on horizontal planes) and v = `normal x u`, so directions are fixed only up to sign: on a plane with normal +Y, u = -X and v = +Z; with normal -Y, u = +X. |
-| `{"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}` | Moves vertices, edges and faces (their vertices) by `offset`. Faces that share only some moved vertices stretch and may tilt but must stay flat: a move that would bend one is `invalid_input` naming that face. A move that would collapse an edge, turn a face over, make a face cross itself, push a hole out of its face or land a vertex on another is `invalid_input` naming the ids. Moved geometry does not merge with what it ends up touching. Unknown ids are `not_found`. |
-| `{"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 4}` | Copies edges and faces `count` times (default 1, at most 100); copy k is shifted by `k * offset`. Copies are drawn like new edges, so they merge with what they touch and closed loops become faces (an opening in a copied face stays open); copied faces keep the orientation of their originals. Vertex ids are `invalid_input`. |
-| `{"type": "erase", "ids": ["e2", "f8"]}` | Erases edges and faces. An erased face leaves its edges. An erased edge takes the faces it bounds with it, except that two faces in one plane meeting at it merge into one. Vertices go with their last edge; a vertex left between two collinear edges is healed away. Faces are erased before edges, so erasing a face together with the seam it shares with a coplanar neighbour removes the neighbour too: the seam then bounds only one face. Vertex ids are `invalid_input` and unknown ids are `not_found`, both with `ids`; empty `ids` is `invalid_input`. |
+| `{"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}` | Moves vertices, edges and faces (their vertices) and instances by `offset`. Faces that share only some moved vertices stretch and may tilt but must stay flat: a move that would bend one is `invalid_input` naming that face. A move that would collapse an edge, turn a face over, make a face cross itself, push a hole out of its face or land a vertex on another is `invalid_input` naming the ids. Moved geometry does not merge with what it ends up touching. Unknown ids are `not_found`. |
+| `{"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 4}` | Copies edges, faces and instances `count` times (default 1, at most 100); copy k is shifted by `k * offset`. Copies are drawn like new edges, so they merge with what they touch and closed loops become faces, also on an outline that had no face (an opening in a copied face stays open); copied faces keep the orientation of their originals. A copied component instance places the same definition; a copied group gets a definition of its own. Vertex ids are `invalid_input`. |
+| `{"type": "erase", "ids": ["e2", "f8"]}` | Erases edges, faces and instances (a group's definition goes with its last instance). An erased face leaves its edges. An erased edge takes the faces it bounds with it, except that two faces in one plane meeting at it merge into one. Vertices go with their last edge; a vertex left between two collinear edges is healed away. Faces are erased before edges, so erasing a face together with the seam it shares with a coplanar neighbour removes the neighbour too: the seam then bounds only one face. Vertex ids are `invalid_input` and unknown ids are `not_found`, both with `ids`; empty `ids` is `invalid_input`. |
+| `{"type": "make_group", "ids": ["e2", "f8", "i4"]}` | Moves edges, faces (with their edges) and instances into a new group definition and places one instance of it where they were; the instance becomes the selection. Edges that still bound a face left behind stay outside too. Vertex ids are `invalid_input`. |
+| `{"type": "make_component", "ids": ["f8"], "name": "Chair"}` | Like `make_group`, but makes a named component: its copies share the definition, so editing one edits them all. An empty name is `invalid_input`. |
+| `{"type": "explode", "ids": ["i4"]}` | Replaces instances by their geometry, which merges with what it touches; instances inside come out as instances (with new ids). What comes out becomes the selection. Ids that are not instances are `invalid_input`. |
 | `{"type": "push_pull", "face": "f8", "distance": 1.5}` | Moves a face along its normal (negative: against it). A face on a flat sheet extrudes into a solid and stays as its base. A face on a solid extrudes a bump, or cuts a recess when pushed in; pushed in exactly as far as a parallel face that holds it (the back of a wall), it cuts an opening through. A face whose neighbors all run along the normal, like a box top, moves and stretches them; moved back flush with the surface around it (a recess or bump pushed back), it closes up and the seams heal. |
 
-`{"type": "select", "ids": ["f8"], "mode": "replace"}` changes the selection (edges and faces;
+`{"type": "select", "ids": ["f8"], "mode": "replace"}` changes the selection (edges, faces and
+instances;
 `mode` is `replace` (default), `add`, `remove` or `toggle`). It is not an undo step and does not
 change the revision. The window shows the selection and agents see what the user selected with
 the `selection` query. Vertex ids are `invalid_input` and unknown ids are `not_found`, both with
 `ids`. Empty `ids` with mode `replace` clears the selection.
@@ -88,8 +96,27 @@ pushed face, with nothing else on it inside that area: that cuts an opening (a w
 wall, a hole through a slab). The floor of a recess pushed on to the back opens the same way.
 When a face moves, its vertices are reported as `modified`; the edges and faces using them changed
 shape too.
 
+### Groups and components
+
+Commands and queries work on the open context: the root at first. `{"type": "open_instance",
+"id": "i4"}` opens an instance of the open context for editing, and `{"type": "close_instance"}`
+goes back out one level. Neither is an undo step or changes the revision; both clear the
+selection. While an instance is open:
+
+- `entities`, `entity`, `stats` and `selection` see only the definition's entities, and ids
+  from elsewhere are `not_found`;
+- points and offsets in commands, and positions in results, are in the definition's own
+  coordinates (`instance.transform` maps them into the coordinates around it);
+- edits change the definition, so every instance of a component changes with it.
+
+An instance's `entity` result has its `definition`, `name`, `kind` (`group` or `component`),
+`transform` (column-major 3x4: x axis, y axis, z axis, translation) and `bounds` in the open
+context's coordinates. Undo and redo apply to the whole model wherever the change was made; an
+open instance that stops existing is closed. Groups keep the coordinates of the context they
+were made in (the instance's transform starts as the identity).
+
 Behaviors worth knowing:
 
 - Replies come before the `changed` notifications they cause, and your own edits are notified too.
 - A request without an `id` runs but gets no reply. JSON-RPC batch arrays are rejected; use the
@@ -104,9 +131,9 @@ Behaviors worth knowing:
 ```
 -> {"jsonrpc":"2.0","id":1,"method":"execute","params":{"type":"add_edge","start":[0,0,0],"end":[2,0,0]}}
 <- {"jsonrpc":"2.0","id":1,"result":{"changes":{"created":["v0","v1","e2"],"deleted":[],"modified":[]},"revision":1}}
 -> {"jsonrpc":"2.0","id":2,"method":"query","params":{"type":"stats"}}
-<- {"jsonrpc":"2.0","id":2,"result":{"type":"stats","vertices":2,"edges":1,"faces":0,"revision":1}}
+<- {"jsonrpc":"2.0","id":2,"result":{"type":"stats","vertices":2,"edges":1,"faces":0,"instances":0,"definitions":0,"revision":1}}
 ```
 
 Building a 2 x 3 x 1.5 m box on an empty model:
 
~~~~

Apply to `docs/superpowers/specs/2026-10-10-tenon-design.md`:
```diff
--- a/docs/superpowers/specs/2026-10-10-tenon-design.md
+++ b/docs/superpowers/specs/2026-10-10-tenon-design.md
@@ -1,8 +1,8 @@
 # Tenon — Design
 
 Date: 2026-10-10
-Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope)
+Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c)
 
 ## Goal
 
 A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.
@@ -70,9 +70,11 @@ SketchUp geometry is non-manifold (an edge can border three or more faces; edges
 ### Groups and components
 
 - A `Definition` owns its own mesh. An `Instance` references a definition with a transform.
 - Model = root mesh + tree of instances. Geometry in different meshes never merges.
-- Groups are single-use definitions; components are shared.
+- Groups are single-use definitions; components are shared. Copying a group copies its definition; copying a component instance places the same definition. A group's definition goes with its last instance.
+- Entity ids are unique across all meshes, so an id alone names an entity anywhere in the model.
+- Editing happens in an open context: the root or an instance opened for editing (a path of instances). Commands and queries address the open context's mesh, in its own coordinates.
 
 ### Operations API
 
 `add_edge`, `push_pull(face, distance)`, `move_vertices`, `erase`, `split_face`, `make_group`, etc. Each returns a `ChangeSet` (created / deleted / modified element keys plus data needed to invert). ChangeSets drive undo and render-cache invalidation.
@@ -147,9 +149,9 @@ Unit tests for buffer building and camera math; a small set of headless golden-i
 Design rule for every milestone: the interface is one client of the command layer, and the API is another. No feature may exist only in the interface.
 
 ### Commands
 
-- `session::Command` is a serde-serializable enum covering every state change: modeling (add edge, draw rectangle/circle, push/pull face by distance, move/copy/rotate entities, erase, make group/component), selection, camera (set, orbit, zoom extents, standard views), undo/redo, file (new, open, save, import, export).
+- `session::Command` is a serde-serializable enum covering every state change: modeling (add edge, draw rectangle/circle, push/pull face by distance, move/copy/rotate entities, erase, make group/component, explode), editing context (open/close an instance), selection, camera (set, orbit, zoom extents, standard views), undo/redo, file (new, open, save, import, export).
 - Commands take explicit values (points, distances, entity IDs), never screen coordinates or cursor state.
 - `Document::execute(Command) -> Result<Outcome, CommandError>`, where `Outcome` carries the net `ChangeSummary` (created/deleted/modified ids as strings). Each modeling command is one undo step; camera commands are not recorded. Errors are structured (`kind` + `message` + offending `ids`), never panics. Raw `ChangeSet`s stay internal.
 - Entity IDs are the kernel's stable ids, serialized as strings (`"v12"`, `"e3"`, `"f7"`).
 
@@ -186,9 +188,11 @@ Every milestone's acceptance includes: new capabilities are reachable as `sessio
 - **M3**: Rectangle, Push/Pull, VCB (lengths and `width,height`; shown in the window title until the egui shell lands in M4).
 - **M4a**: egui shell (toolbar, measurements box, entity info); selection in the session; Select tool (click, box); Erase tool and command (with face merging and edge healing).
 - **M4b**: Move/Copy (with VCB array syntax); parallel/perpendicular inference; direction locks (Shift, arrow keys).
 - **M4c**: push/pull through to openings; push/pull back to flush welds the face and heals the seams; apparent-intersection snaps; BVH picking (a kernel index used by tools and push/pull).
-- **M5**: groups and components; save/load; export.
+- **M5a**: groups and components in the kernel and session: make group/component, explode, copy/move/erase/select instances, open/close an instance for editing (commands and queries), drawn with the selection and open context shown.
+- **M5b**: group and component tools: picking and box-selecting instances, double-click to open and close, Move on instances, inference on geometry inside instances, editing a moved instance with the tools, toolbar and shortcuts for make group/component and explode.
+- **M5c**: save/load (`.skr`); export OBJ, STL, glTF; file commands.
 
 ## Risks
 
 - **Sticky geometry robustness**: edge merging and face detection under floating-point error is the main risk. Mitigation: robust predicates, explicit tolerances, invariant checks in every test, fuzz tests on random edge sequences.
```

- [ ] **Step 4: Run the tests to verify they pass**

Visual check, headless (never open the window). Build with `cargo build -p app`, then drive `target/debug/app --headless --stdio` with JSON-RPC lines:
1. `add_rectangle` (0,0,0)-(2,2,0), then `push_pull` `f8` by 1;
2. `make_component` of every edge and face, named `Crate`;
3. `copy` the instance with offset [3,0,0] and count 2;
4. `open_instance` the first instance, `push_pull` its top face (normal [0,0,1]) by 1, and `select` that face;
5. `render_view` 800x500 with camera eye [-4,-7,6], target [4,1,0.5]; decode `png_base64` to a file;
6. `close_instance`, `select` the first instance, and render again.

Expected:
- all three boxes are 2 m tall;
- with the instance open, the other two have light gray edges and only the open instance's top is blue;
- after closing, the whole first box is blue with blue edges.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 423 tests pass.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/api/tests/dispatch.rs docs/agent-api.md docs/superpowers/specs/2026-10-10-tenon-design.md
git commit -m "Document groups and components in the agent API"
```

## Self-review notes

- **Spec coverage (M5a line):**

  | Requirement | Task |
  |---|---|
  | make group/component | 1 (kernel), 3 (commands) |
  | explode | 2, 3 |
  | copy, move, erase and select instances | 1-3 |
  | open/close an instance (commands and queries) | 3 |
  | drawn with the selection and the open context | 4 |
  | API reachability and docs | 5 |

  M5b and M5c items are listed in the spec amendment and in "Known gaps".
- **Section 1 invariants:**
  - geometry in different meshes never merges: each mesh is edited on its own, and only explode draws into the parent;
  - undo restores original ids: `SceneChange` steps carry full data, and a removed definition is stored whole;
  - ids are never reused: the `Scene` counter never decreases, and undoing a definition does not lower it. This is pinned by `ids_stay_unique_across_meshes_and_are_never_reused`.
- **Type names used across tasks:**
  - `Scene`, `SceneChange`, `Step`, `MeshId`, `Definition`, `DefinitionKind`, `Instance`, `InstanceId`, `DefinitionId`;
  - `Scene::{edit, make_group, explode, copy_instances, erase_instances, resolve, bounds, instance_bounds, instance_count}`;
  - `Document::{scene, context, context_transform, model}`;
  - `scene_geometry`, `set_scene`, `capture`.
