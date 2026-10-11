# M5b Group and Component Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The window's tools work with groups and components:
- clicking or box-selecting picks them, and the Move tool moves and copies them;
- double-clicking opens one, and Escape or a double-click outside closes it;
- tools inside a moved group draw where the cursor is;
- inference snaps to geometry inside groups and outside the open one;
- toolbar buttons and the G and C shortcuts make groups and components and explode them.

**Architecture:**
- **tools:**
  - A new `Scenery` (`scenery.rs`) lists every placed mesh of the scene in the open context's coordinates. Each part is tagged with who owns it: the open context, an instance of it, or outside it.
  - Inference, edge picking and face picking go through all parts:
    - snaps come from any geometry, and faces anywhere hide what is behind them;
    - `Inference::face` stays the open context's face only.
  - `ToolContext` carries the scenery. Picking an instance's geometry picks the instance. A new `ToolEvent::DoubleClick` lets the Select tool open and close instances.
- **app:**
  - The editor builds the scenery for every event and gives tools a camera moved into the open context's coordinates (placements are moves). It maps previews and markers back into the root's coordinates for drawing.
  - The window turns two quick releases into a double-click.
  - The toolbar gets Make group, Make component, Explode and Close group buttons. `g` and `c` are shortcuts.
  - Entity Info says what is open.
- **No session or API changes:** every action here is an existing command (`make_group`, `make_component`, `explode`, `open_instance`, `close_instance`, `select`, `move`, `copy`).

**Tech Stack:** Rust 2024; glam 0.34.1; egui 0.36.2; winit 0.30.13; no dependency changes.

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`:
- section 2 (inference engine; tools include Make Group/Component);
- section 4 (input routing);
- the M5b milestone line. Task 4 adds an inference note to section 2.

## Global Constraints

- Tools never edit the model directly: every action is a `session::Command` run through the editor, the same path the API uses.
- Each modeling command is one undo step; opening and closing instances are not undo steps (as in M5a).
- No dependency changes.
- Git:
  - work on `dev`;
  - stage only the files you changed;
  - short plain commit messages, no emoji;
  - run `cargo fmt --all` before each commit;
  - `cargo clippy --workspace --all-targets -- -D warnings` stays clean.
- Do not launch the app window; everything is tested headless (editor tests drive the editor directly).

## Spec notes (rulings made in this plan)

- **Coordinates.**
  - Tools see the open context's mesh and a camera expressed in its coordinates. Placements are moves (no rotate or scale yet), so the camera only shifts.
  - Commands built by tools therefore carry the open context's coordinates, which is what the session expects.
  - The editor maps previews and the inference marker back into root coordinates for the overlay.
  - The ground used for inference inside an instance is its own z = 0 plane, as in SketchUp, where tools inside a group use its axes.
- **Seeing vs editing.** Inference sees every placed mesh: the open context, its instances at any depth, and everything outside the open instance.
  - Endpoint, midpoint, on-edge and intersection snaps can come from any of them, and faces anywhere hide points behind them.
  - `Inference::face` (used by Push/Pull and Rectangle) is given only for the open context's own faces. Drawing a rectangle on a group's face from outside therefore starts on a horizontal plane; see the gaps below.
  - The reference edge for parallel and perpendicular snaps can be any visible edge.
- **Picking.** A click picks the open context's edge under the cursor, else an instance's edge (which picks the instance), else the nearest face:
  - the open context's face picks the face;
  - an instance's face picks the instance;
  - a face outside the open instance picks nothing.

  The same `pick` serves Select, Move (with an empty selection) and Erase, so Erase now erases a group under the cursor.
- **Box selection** treats an instance as its bounding box. A window (drag right) needs all 8 projected corners inside. A crossing (drag left) needs the projected corners' screen rectangle to overlap the box. Instances with a corner behind the camera are skipped.
- **Opening and closing.**
  - Double-clicking an instance of the open context opens it.
  - Double-clicking where nothing of the open context or its instances lies closes the open instance.
  - Escape in the Select tool with nothing in progress closes it too.
  - A double-click is two left releases within 500 ms and 6 logical pixels. It is detected in the window and sent after the second release, so the Select tool has already selected what was clicked.
- **Move and copy** take instance ids like any others. The outline drawn for an instance is its bounding box. After a copy, the editor selects only what was created in the open context; a copied group's new contents live in its new definition.
- **Shortcuts and buttons.**
  - `g` makes a group of the selection.
  - `c` makes a component named "Component N", with the first N not already used by a definition. Renaming comes later, with an API command.
  - The toolbar has Make group (G), Make component (C), Explode (enabled when a group or component is selected) and Close group (Escape, enabled while one is open).
  - Explode applies to the selected instances only.
- **Entity Info** starts with "Editing group" or "Editing component <name>" while one is open.

## Known gaps (record in the ledger; later milestones)

- A rectangle drawn on a group's face from outside lies on a horizontal plane, not the face's.
- Rotated or scaled placements are not supported by the camera shift; nothing creates them yet.
- `Scenery` is rebuilt for every event and every part is searched. That is fine at current model sizes, but needs caching or a scene-level index for big scenes.

## File Structure

| File | Responsibility |
|---|---|
| `crates/tools/src/scenery.rs` (new) | `Scenery`, `Part`, `Owner`: placed meshes in the open context's coordinates, `instance_box` |
| `crates/tools/src/inference.rs` | inference and picking over a `Scenery` (`infer_in`, `edge_under_in`, `face_under_in`); the one-mesh functions stay as wrappers |
| `crates/tools/src/tool.rs` | `ToolContext::scenery`, `ToolEvent::DoubleClick` |
| `crates/tools/src/select.rs` | picking and box-selecting instances, box outlines, double-click and Escape |
| `crates/tools/src/{move_tool,erase,line,rectangle,push_pull}.rs` | scenery in test rigs, instance outlines, the new event |
| `crates/app/src/editor.rs` | local camera, scenery per event, overlay mapping, `double_click`, group/component/explode/close actions and keys |
| `crates/app/src/app.rs` | double-click detection, toolbar actions, `g`/`c` physical keys |
| `crates/app/src/ui.rs`, `info.rs` | toolbar buttons; "Editing ..." line |

The code below was written and verified in a scratch copy of `dev` at 79fe19f. Each task was checked on its own with `cargo fmt --all --check`, clippy with `-D warnings`, and the full test suite. Workspace test totals after each task: 450, 453, 460, 463 (from 443).

Code for new files is given in full. Changes to existing files are given as unified diffs against the state the previous task leaves (apply them by hand or with `git apply`; the context lines show where each change goes).

---

### Task 1: See and snap to geometry in every placed mesh

**Files:**
- Create: `crates/tools/src/scenery.rs` (with unit tests)
- Modify: `crates/tools/src/inference.rs`, `crates/tools/src/lib.rs`
- Test: `crates/tools/tests/scenery.rs` (new)

**Interfaces:**
- Consumes (M5a, kernel):
  - `Scene::{resolve, mesh}`;
  - `Model::{instances, vertices, index}`;
  - `Index::{ray_hit, near_ray}`;
  - `MeshId`, `InstanceId`.
- Produces (re-exported from `tools`):
  - `Owner { Open, Instance(InstanceId), Outside }`;
  - `Part<'a> { model: &'a Model, place: DAffine3, owner: Owner }`;
  - `Scenery<'a> { parts: Vec<Part<'a>> }`.
- `Scenery` methods:
  - `single(&Model) -> Scenery`;
  - `of(&Scene, context: &[InstanceId]) -> Option<Scenery>`, with the open part first;
  - `open() -> &Model`;
  - `inside_instance() -> bool`;
  - `instance_box(InstanceId) -> Option<[DVec3; 2]>`.
- Inference and picking functions:
  - `infer_in(&Scenery, &Camera, DVec2, &Guides, f64) -> Inference`;
  - `edge_under_in(&Scenery, &Camera, DVec2, f64) -> Option<(Owner, EdgeId, DVec3, DVec3)>`;
  - `face_under_in(&Scenery, &Camera, DVec2) -> Option<(Owner, FaceId, DVec3)>`.
- `infer`, `infer_with`, `edge_under` and `face_under` keep their one-mesh signatures as wrappers.

- [ ] **Step 1: Write the failing tests**

Create `crates/tools/tests/scenery.rs`:
```rust
//! Inference and picking across groups: geometry inside instances and outside the open
//! instance is seen and snapped to, in the open context's coordinates.

use glam::{DVec2, DVec3};
use kernel::{DefinitionKind, InstanceId, MeshId, Model, Scene};
use session::Camera;
use tools::{Guides, Owner, Scenery, SnapKind, edge_under_in, face_under_in, infer_in};

const R: f64 = 10.0;

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 1 x 1 x 1 box at the origin, grouped and moved by (3, 0, 0), plus a loose root edge
/// from (0, -2, 0) to (1, -2, 0). Returns the scene and the group's instance.
fn scene() -> (Scene, InstanceId) {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let base = m.faces().next().unwrap().0;
    m.push_pull(base, 1.0).unwrap();
    let mut s = Scene::from_root(m);
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (i, _) = s
        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
        .unwrap();
    s.edit(MeshId::Root, |m| {
        let mut c = m.add_edge(p(0.0, -2.0, 0.0), p(1.0, -2.0, 0.0))?;
        c.ops.extend(m.move_instances(&[i], p(3.0, 0.0, 0.0)).ops);
        Ok(c)
    })
    .unwrap();
    s.validate().unwrap();
    (s, i)
}

/// The default camera, moved into the coordinates of an instance placed by `shift`.
fn camera(shift: DVec3) -> Camera {
    let mut c = Camera::new(DVec2::new(800.0, 600.0));
    c.eye -= shift;
    c.target -= shift;
    c
}

fn px(c: &Camera, q: DVec3) -> DVec2 {
    c.project(q).unwrap()
}

#[test]
fn from_the_root_a_groups_corner_is_an_endpoint() {
    let (s, _) = scene();
    let view = Scenery::of(&s, &[]).unwrap();
    let c = camera(DVec3::ZERO);
    let corner = p(4.0, 0.0, 1.0);
    let hit = infer_in(
        &view,
        &c,
        px(&c, corner) + DVec2::new(3.0, 2.0),
        &Guides::default(),
        R,
    );
    assert_eq!(hit.kind, SnapKind::Endpoint);
    assert!(hit.point.abs_diff_eq(corner, 1e-9), "{:?}", hit.point);
}

#[test]
fn a_groups_face_is_seen_but_not_given_as_the_open_contexts_face() {
    let (s, i) = scene();
    let view = Scenery::of(&s, &[]).unwrap();
    let c = camera(DVec3::ZERO);
    let top = p(3.5, 0.5, 1.0);
    let hit = infer_in(&view, &c, px(&c, top), &Guides::default(), R);
    assert_eq!(hit.kind, SnapKind::OnFace);
    assert!(hit.point.abs_diff_eq(top, 1e-9));
    assert_eq!(hit.face, None);
    let (owner, _, at) = face_under_in(&view, &c, px(&c, top)).unwrap();
    assert_eq!(owner, Owner::Instance(i));
    assert!(at.abs_diff_eq(top, 1e-9));
}

#[test]
fn a_groups_faces_hide_what_is_behind_them() {
    let (mut s, _) = scene();
    let c = camera(DVec3::ZERO);
    // A root edge starting right behind the middle of the group's box, as seen from the eye.
    let hidden = c.eye + (p(3.5, 0.5, 0.5) - c.eye) * 1.2;
    s.edit(MeshId::Root, |m| {
        m.add_edge(hidden, hidden + p(0.5, 0.0, 0.0))
    })
    .unwrap();
    let view = Scenery::of(&s, &[]).unwrap();
    let hit = infer_in(&view, &c, px(&c, hidden), &Guides::default(), R);
    assert_ne!(hit.kind, SnapKind::Endpoint, "{hit:?}");
    // From the root alone (no group), the same end is snapped to.
    let open = Scenery::single(s.root());
    let bare = infer_in(&open, &c, px(&c, hidden), &Guides::default(), R);
    assert_eq!(bare.kind, SnapKind::Endpoint);
}

#[test]
fn edges_under_the_cursor_report_whose_they_are() {
    let (s, i) = scene();
    let view = Scenery::of(&s, &[]).unwrap();
    let c = camera(DVec3::ZERO);
    let (owner, _, _, _) = edge_under_in(&view, &c, px(&c, p(3.5, 0.0, 0.0)), R).unwrap();
    assert_eq!(owner, Owner::Instance(i));
    let (owner, _, _, _) = edge_under_in(&view, &c, px(&c, p(0.5, -2.0, 0.0)), R).unwrap();
    assert_eq!(owner, Owner::Open);
}

#[test]
fn inside_the_group_the_root_is_snapped_to_in_the_groups_coordinates() {
    let (s, i) = scene();
    let view = Scenery::of(&s, &[i]).unwrap();
    let shift = p(3.0, 0.0, 0.0);
    let c = camera(shift);
    // The root edge's end (1, -2, 0) is (-2, -2, 0) inside the group.
    let end = p(-2.0, -2.0, 0.0);
    let hit = infer_in(
        &view,
        &c,
        px(&c, end) + DVec2::new(2.0, 2.0),
        &Guides::default(),
        R,
    );
    assert_eq!(hit.kind, SnapKind::Endpoint);
    assert!(hit.point.abs_diff_eq(end, 1e-9), "{:?}", hit.point);
    let (owner, _, _, _) = edge_under_in(&view, &c, px(&c, p(-2.5, -2.0, 0.0)), R).unwrap();
    assert_eq!(owner, Owner::Outside);
    // The group's own top face is the open context's face.
    let hit = infer_in(&view, &c, px(&c, p(0.5, 0.5, 1.0)), &Guides::default(), R);
    assert!(hit.face.is_some());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p tools --test scenery`
Expected: compile errors: unresolved imports `tools::Owner`, `tools::Scenery`, `tools::edge_under_in`, `tools::face_under_in`, `tools::infer_in`.

- [ ] **Step 3: Implement**

Create `crates/tools/src/scenery.rs`:
```rust
//! What tools see: the open context's mesh plus every other placed mesh of the scene, all in
//! the open context's coordinates.

use glam::{DAffine3, DVec3};
use kernel::{InstanceId, MeshId, Model, Scene};

/// Whose geometry a part is, seen from the open context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The open context's own mesh: editable, its entities can be picked.
    Open,
    /// Geometry placed (at any depth) by this instance of the open context: picking it picks
    /// the instance.
    Instance(InstanceId),
    /// Geometry outside the open context: seen and snapped to, never picked.
    Outside,
}

/// One placed mesh.
#[derive(Clone, Copy, Debug)]
pub struct Part<'a> {
    pub model: &'a Model,
    /// Maps the mesh's coordinates into the open context's. Rigid (placements are moves).
    pub place: DAffine3,
    pub owner: Owner,
}

/// Every placed mesh, the open context's first.
#[derive(Clone, Debug)]
pub struct Scenery<'a> {
    pub parts: Vec<Part<'a>>,
}

impl<'a> Scenery<'a> {
    /// Just one mesh, open, in its own coordinates.
    pub fn single(model: &'a Model) -> Self {
        Scenery {
            parts: vec![Part {
                model,
                place: DAffine3::IDENTITY,
                owner: Owner::Open,
            }],
        }
    }

    /// The whole scene seen from the open context `context` (a path of instances from the
    /// root; see `session::Document::context`). None if the path does not resolve.
    pub fn of(scene: &'a Scene, context: &[InstanceId]) -> Option<Self> {
        let (_, to_root) = scene.resolve(context)?;
        let from_root = to_root.inverse();
        let mut parts = Vec::new();
        let mut path = Vec::new();
        collect(
            scene,
            MeshId::Root,
            from_root,
            context,
            &mut path,
            &mut parts,
        );
        // The open mesh first, then the others in walk order.
        parts.sort_by_key(|p: &Part| p.owner != Owner::Open);
        Some(Scenery { parts })
    }

    /// The open context's own mesh.
    pub fn open(&self) -> &'a Model {
        self.parts[0].model
    }

    /// True when an instance is open (there is geometry outside the open context or the root
    /// is not what is open).
    pub fn inside_instance(&self) -> bool {
        self.parts.iter().any(|p| p.owner == Owner::Outside)
    }

    /// The box around everything an instance of the open context places, in the open
    /// context's coordinates. None when it places nothing.
    pub fn instance_box(&self, id: InstanceId) -> Option<[DVec3; 2]> {
        let mut points = self
            .parts
            .iter()
            .filter(|p| p.owner == Owner::Instance(id))
            .flat_map(|p| {
                p.model
                    .vertices()
                    .map(move |(_, v)| p.place.transform_point3(v.pos))
            });
        let first = points.next()?;
        Some(points.fold([first, first], |[lo, hi], q| [lo.min(q), hi.max(q)]))
    }
}

/// Adds the mesh `id`, placed by `place` (into the open context's coordinates) and reached by
/// `path`, and everything it places.
fn collect<'a>(
    scene: &'a Scene,
    id: MeshId,
    place: DAffine3,
    context: &[InstanceId],
    path: &mut Vec<InstanceId>,
    parts: &mut Vec<Part<'a>>,
) {
    let Some(model) = scene.mesh(id) else { return };
    let owner = if path.as_slice() == context {
        Owner::Open
    } else if path.starts_with(context) {
        Owner::Instance(path[context.len()])
    } else {
        Owner::Outside
    };
    parts.push(Part {
        model,
        place,
        owner,
    });
    for (iid, instance) in model.instances() {
        path.push(iid);
        collect(
            scene,
            MeshId::Definition(instance.definition),
            place * instance.transform,
            context,
            path,
            parts,
        );
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::DefinitionKind;

    /// A root holding a square at the origin, grouped, plus a loose edge; the group is moved
    /// by (5, 0, 0). Returns the scene and the instance.
    fn scene() -> (Scene, InstanceId) {
        let mut m = Model::new();
        let p = |x, y| DVec3::new(x, y, 0.0);
        m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
            .unwrap();
        let mut s = Scene::from_root(m);
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (i, _) = s
            .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        s.edit(MeshId::Root, |m| {
            let mut c = m.add_edge(p(0.0, 3.0), p(1.0, 3.0))?;
            c.ops
                .extend(m.move_instances(&[i], DVec3::new(5.0, 0.0, 0.0)).ops);
            Ok(c)
        })
        .unwrap();
        (s, i)
    }

    #[test]
    fn from_the_root_instances_are_placed_where_they_are_drawn() {
        let (s, i) = scene();
        let v = Scenery::of(&s, &[]).unwrap();
        assert_eq!(v.parts.len(), 2);
        assert_eq!(v.parts[0].owner, Owner::Open);
        assert_eq!(v.parts[1].owner, Owner::Instance(i));
        assert!(!v.inside_instance());
        assert_eq!(
            v.instance_box(i),
            Some([DVec3::new(5.0, 0.0, 0.0), DVec3::new(6.0, 1.0, 0.0)])
        );
    }

    #[test]
    fn inside_an_instance_the_rest_is_outside_and_shifted() {
        let (s, i) = scene();
        let v = Scenery::of(&s, &[i]).unwrap();
        assert_eq!(v.parts[0].owner, Owner::Open);
        assert_eq!(v.parts[0].place, DAffine3::IDENTITY);
        assert_eq!(v.parts[1].owner, Owner::Outside);
        // The root's loose edge at x 0..1 lies at x -5..-4 in the group's coordinates.
        assert_eq!(
            v.parts[1].place,
            DAffine3::from_translation(DVec3::new(-5.0, 0.0, 0.0))
        );
        assert!(v.inside_instance());
        assert!(Scenery::of(&s, &[InstanceId(999)]).is_none());
    }
}
```

Apply to `crates/tools/src/inference.rs`:
```diff
--- a/crates/tools/src/inference.rs
+++ b/crates/tools/src/inference.rs
@@ -3,8 +3,10 @@
 use glam::{DVec2, DVec3};
 use kernel::{EdgeId, FaceId, Model, Shape, VertexId};
 use session::{Camera, Ray};
 
+use crate::scenery::{Owner, Scenery};
+
 #[derive(Clone, Copy, Debug, PartialEq, Eq)]
 pub enum Axis {
     X,
     Y,
@@ -47,9 +49,10 @@ pub enum SnapKind {
 #[derive(Clone, Copy, Debug, PartialEq)]
 pub struct Inference {
     pub point: DVec3,
     pub kind: SnapKind,
-    /// The nearest face under the cursor, whatever the point snapped to.
+    /// The nearest face under the cursor, whatever the point snapped to, if it belongs to the
+    /// open context (a face inside an instance or outside the open instance is not given).
     pub face: Option<FaceId>,
 }
 
 /// What inference can follow from the point a tool is drawing from.
@@ -73,10 +76,10 @@ pub struct Lock {
     /// How the locked point is reported (an axis, parallel or perpendicular).
     pub kind: SnapKind,
 }
 
-/// Finds the 3D point under `cursor` (physical pixels) with no guides but `anchor`, which
-/// enables axis inference. See [`infer_with`].
+/// Finds the 3D point under `cursor` (physical pixels) on one mesh, with no guides but
+/// `anchor`, which enables axis inference. See [`infer_in`].
 pub fn infer(
     model: &Model,
     camera: &Camera,
     cursor: DVec2,
@@ -89,27 +92,39 @@ pub fn infer(
     };
     infer_with(model, camera, cursor, &guides, radius_px)
 }
 
-/// Finds the 3D point under `cursor` (physical pixels). Snap candidates count only within
-/// `radius_px` of the cursor.
+/// [`infer_in`] on one mesh.
+pub fn infer_with(
+    model: &Model,
+    camera: &Camera,
+    cursor: DVec2,
+    guides: &Guides,
+    radius_px: f64,
+) -> Inference {
+    infer_in(&Scenery::single(model), camera, cursor, guides, radius_px)
+}
+
+/// Finds the 3D point under `cursor` (physical pixels) among everything `scenery` holds, in
+/// the open context's coordinates (`camera` must be in them too). Snap candidates count only
+/// within `radius_px` of the cursor.
 ///
 /// Ranking: endpoint, midpoint and origin (nearest on screen wins) > intersection > on-edge >
 /// directions from the anchor (axes, parallel or perpendicular to the reference edge; nearest
 /// wins) > on-face > ground > free. Points and edges hidden behind a face are skipped. With a
 /// lock, the result is always on the locked line.
-pub fn infer_with(
-    model: &Model,
+pub fn infer_in(
+    scenery: &Scenery,
     camera: &Camera,
     cursor: DVec2,
     guides: &Guides,
     radius_px: f64,
 ) -> Inference {
     let ray = camera.ray(cursor);
-    let face_hit = nearest_face_hit(model, &ray);
-    let face = face_hit.map(|(f, _)| f);
+    let face_hit = nearest_face_hit(scenery, &ray);
+    let face = face_hit.and_then(|(owner, f, _)| (owner == Owner::Open).then_some(f));
     let screen_dist = |p: DVec3| camera.project(p).map(|s| s.distance(cursor));
-    let snapped = snap_point(model, camera, cursor, radius_px);
+    let snapped = snap_point(scenery, camera, cursor, radius_px);
 
     if let (Some(anchor), Some(lock)) = (guides.anchor, guides.lock) {
         let point = match snapped {
             Some((p, _)) => anchor + lock.dir * (p - anchor).dot(lock.dir),
@@ -149,9 +164,9 @@ pub fn infer_with(
         }
     }
 
     // On a face (nearest along the ray).
-    if let Some((_, point)) = face_hit {
+    if let Some((_, _, point)) = face_hit {
         return Inference {
             point,
             kind: SnapKind::OnFace,
             face,
@@ -175,9 +190,9 @@ pub fn infer_with(
 /// The visible point snap under the cursor, if any: endpoint, midpoint or origin (nearest on
 /// screen), else the nearest apparent intersection of two edges both visible there, else the
 /// nearest point on an edge.
 fn snap_point(
-    model: &Model,
+    scenery: &Scenery,
     camera: &Camera,
     cursor: DVec2,
     radius_px: f64,
 ) -> Option<(DVec3, SnapKind)> {
@@ -186,35 +201,35 @@ fn snap_point(
         camera
             .project(p)
             .map(|s| s.distance(cursor))
             .filter(|d| *d <= radius_px)
-            .filter(|_| visible(model, camera.eye, p))
+            .filter(|_| visible(scenery, camera.eye, p))
     };
-    let near = Near::around(model, camera, cursor, radius_px);
+    let near = Near::around(scenery, camera, cursor, radius_px);
     let mut best: Option<(f64, DVec3, SnapKind)> = None;
     let mut offer = |p: DVec3, kind: SnapKind| {
         if let Some(d) = within(p)
             && best.is_none_or(|(bd, _, _)| d < bd)
         {
             best = Some((d, p, kind));
         }
     };
-    for &(_, p) in &near.points {
+    for &(_, _, p) in &near.points {
         offer(p, SnapKind::Endpoint);
     }
-    for &(_, a, b) in &near.segments {
+    for &(_, _, a, b) in &near.segments {
         offer((a + b) * 0.5, SnapKind::Midpoint);
     }
     offer(DVec3::ZERO, SnapKind::Origin);
     if let Some((_, p, kind)) = best {
         return Some((p, kind));
     }
 
     let mut crossing: Option<(f64, DVec3)> = None;
-    for (i, &(_, a, b)) in near.segments.iter().enumerate() {
-        for &(_, c, d) in &near.segments[i + 1..] {
+    for (i, &(_, _, a, b)) in near.segments.iter().enumerate() {
+        for &(_, _, c, d) in &near.segments[i + 1..] {
             if let Some((p, far)) = apparent_crossing(camera, (a, b), (c, d))
-                && visible(model, camera.eye, far)
+                && visible(scenery, camera.eye, far)
                 && let Some(dist) = within(p)
                 && crossing.is_none_or(|(bd, _)| dist < bd)
             {
                 crossing = Some((dist, p));
@@ -225,9 +240,9 @@ fn snap_point(
         return Some((p, SnapKind::Intersection));
     }
 
     let mut on_edge: Option<(f64, DVec3)> = None;
-    for &(_, a, b) in &near.segments {
+    for &(_, _, a, b) in &near.segments {
         let p = closest_on_line_to_ray(a, b - a, &ray, true);
         if let Some(d) = within(p)
             && on_edge.is_none_or(|(bd, _)| d < bd)
         {
@@ -295,38 +310,60 @@ pub fn directions(reference: Option<DVec3>) -> Vec<(DVec3, SnapKind)> {
     }
     out
 }
 
-/// The nearest face under the cursor and the point where the cursor's ray meets it.
+/// The nearest face of one mesh under the cursor and the point where the cursor's ray meets it.
 pub fn face_under(model: &Model, camera: &Camera, cursor: DVec2) -> Option<(FaceId, DVec3)> {
-    nearest_face_hit(model, &camera.ray(cursor))
+    face_under_in(&Scenery::single(model), camera, cursor).map(|(_, f, p)| (f, p))
+}
+
+/// The nearest face under the cursor among everything `scenery` holds: whose it is, the face
+/// (an id in its own mesh) and where the cursor's ray meets it.
+pub fn face_under_in(
+    scenery: &Scenery,
+    camera: &Camera,
+    cursor: DVec2,
+) -> Option<(Owner, FaceId, DVec3)> {
+    nearest_face_hit(scenery, &camera.ray(cursor))
 }
 
-/// The visible edge nearest the cursor on screen, if one lies within `radius_px`.
+/// The visible edge of one mesh nearest the cursor on screen, if one lies within `radius_px`.
 pub fn edge_under(model: &Model, camera: &Camera, cursor: DVec2, radius_px: f64) -> Option<EdgeId> {
+    edge_under_in(&Scenery::single(model), camera, cursor, radius_px).map(|(_, e, _, _)| e)
+}
+
+/// The visible edge nearest the cursor on screen among everything `scenery` holds, if one
+/// lies within `radius_px`: whose it is, the edge (an id in its own mesh) and its ends.
+pub fn edge_under_in(
+    scenery: &Scenery,
+    camera: &Camera,
+    cursor: DVec2,
+    radius_px: f64,
+) -> Option<(Owner, EdgeId, DVec3, DVec3)> {
     let ray = camera.ray(cursor);
-    let mut best: Option<(f64, EdgeId)> = None;
-    for (id, a, b) in Near::around(model, camera, cursor, radius_px).segments {
+    let mut best: Option<(f64, (Owner, EdgeId, DVec3, DVec3))> = None;
+    for (part, id, a, b) in Near::around(scenery, camera, cursor, radius_px).segments {
         let p = closest_on_line_to_ray(a, b - a, &ray, true);
         let Some(d) = camera.project(p).map(|s| s.distance(cursor)) else {
             continue;
         };
-        if d <= radius_px && best.is_none_or(|(bd, _)| d < bd) && visible(model, camera.eye, p) {
-            best = Some((d, id));
+        if d <= radius_px && best.is_none_or(|(bd, _)| d < bd) && visible(scenery, camera.eye, p) {
+            best = Some((d, (scenery.parts[part].owner, id, a, b)));
         }
     }
-    best.map(|(_, id)| id)
+    best.map(|(_, hit)| hit)
 }
 
-/// The vertices and edges that may lie within `radius_px` of the cursor on screen, in id order
-/// so that ties between equally near candidates always go the same way.
+/// The vertices and edges that may lie within `radius_px` of the cursor on screen, with the
+/// index of the part each comes from and positions in the open context's coordinates, in
+/// part and id order so that ties between equally near candidates always go the same way.
 struct Near {
-    points: Vec<(VertexId, DVec3)>,
-    segments: Vec<(EdgeId, DVec3, DVec3)>,
+    points: Vec<(usize, VertexId, DVec3)>,
+    segments: Vec<(usize, EdgeId, DVec3, DVec3)>,
 }
 
 impl Near {
-    fn around(model: &Model, camera: &Camera, cursor: DVec2, radius_px: f64) -> Near {
+    fn around(scenery: &Scenery, camera: &Camera, cursor: DVec2, radius_px: f64) -> Near {
         let ray = camera.ray(cursor);
         // The rays through the cursor and a point `radius_px` beside it spread apart by the
         // cone's slope; doubling it covers the diagonal of the pixel square around the cursor.
         let beside = camera.ray(cursor + DVec2::new(radius_px, 0.0));
@@ -334,32 +371,49 @@ impl Near {
         let mut near = Near {
             points: Vec::new(),
             segments: Vec::new(),
         };
-        model
-            .index()
-            .near_ray(ray.origin, ray.dir, 0.0, slope, |s| match *s {
-                Shape::Point(v, p) => near.points.push((v, p)),
-                Shape::Segment(e, a, b) => near.segments.push((e, a, b)),
-                Shape::Triangle(..) => {}
-            });
-        near.points.sort_by_key(|(v, _)| *v);
-        near.segments.sort_by_key(|(e, _, _)| *e);
+        for (k, part) in scenery.parts.iter().enumerate() {
+            let back = part.place.inverse();
+            let (origin, dir) = (
+                back.transform_point3(ray.origin),
+                back.transform_vector3(ray.dir),
+            );
+            let at = |p: DVec3| part.place.transform_point3(p);
+            part.model
+                .index()
+                .near_ray(origin, dir, 0.0, slope, |s| match *s {
+                    Shape::Point(v, p) => near.points.push((k, v, at(p))),
+                    Shape::Segment(e, a, b) => near.segments.push((k, e, at(a), at(b))),
+                    Shape::Triangle(..) => {}
+                });
+        }
+        near.points.sort_by_key(|(k, v, _)| (*k, *v));
+        near.segments.sort_by_key(|(k, e, _, _)| (*k, *e));
         near
     }
 }
 
 /// True if no face lies between the eye and `p` (faces through `p` itself do not hide it).
-fn visible(model: &Model, eye: DVec3, p: DVec3) -> bool {
+fn visible(scenery: &Scenery, eye: DVec3, p: DVec3) -> bool {
     let to = p - eye;
     let dist = to.length();
     if dist < 1e-12 {
         return true;
     }
-    model
-        .index()
-        .ray_hit(eye, to / dist, 0.0, dist * (1.0 - 1e-6))
-        .is_none()
+    let dir = to / dist;
+    scenery.parts.iter().all(|part| {
+        let back = part.place.inverse();
+        part.model
+            .index()
+            .ray_hit(
+                back.transform_point3(eye),
+                back.transform_vector3(dir),
+                0.0,
+                dist * (1.0 - 1e-6),
+            )
+            .is_none()
+    })
 }
 
 /// Point on the line `origin + t * dir` closest to the ray; `t` is clamped to [0, 1] when
 /// `segment` is true (dir is then the segment vector).
@@ -381,14 +435,25 @@ pub(crate) fn closest_on_line_to_ray(origin: DVec3, dir: DVec3, ray: &Ray, segme
     }
     origin + dir * t
 }
 
-/// The nearest face the ray meets in front of its origin, and where.
-fn nearest_face_hit(model: &Model, ray: &Ray) -> Option<(FaceId, DVec3)> {
-    model
-        .index()
-        .ray_hit(ray.origin, ray.dir, 0.0, f64::INFINITY)
-        .map(|(f, t)| (f, ray.origin + ray.dir * t))
+/// The nearest face the ray meets in front of its origin: whose it is, the face, and where.
+fn nearest_face_hit(scenery: &Scenery, ray: &Ray) -> Option<(Owner, FaceId, DVec3)> {
+    let mut best: Option<(f64, Owner, FaceId)> = None;
+    for part in &scenery.parts {
+        let back = part.place.inverse();
+        let reach = best.map_or(f64::INFINITY, |(t, _, _)| t);
+        if let Some((f, t)) = part.model.index().ray_hit(
+            back.transform_point3(ray.origin),
+            back.transform_vector3(ray.dir),
+            0.0,
+            reach,
+        ) && best.is_none_or(|(bt, _, _)| t < bt)
+        {
+            best = Some((t, part.owner, f));
+        }
+    }
+    best.map(|(t, owner, f)| (owner, f, ray.origin + ray.dir * t))
 }
 
 #[cfg(test)]
 mod tests {
```

Apply to `crates/tools/src/lib.rs`:
```diff
--- a/crates/tools/src/lib.rs
+++ b/crates/tools/src/lib.rs
@@ -7,18 +7,21 @@ pub mod line;
 pub mod move_tool;
 pub mod overlay;
 pub mod push_pull;
 pub mod rectangle;
+pub mod scenery;
 pub mod select;
 pub mod tool;
 pub mod vcb;
 
 pub use erase::EraseTool;
 pub use inference::{
-    Axis, Guides, Inference, Lock, SnapKind, edge_under, face_under, infer, infer_with,
+    Axis, Guides, Inference, Lock, SnapKind, edge_under, edge_under_in, face_under, face_under_in,
+    infer, infer_in, infer_with,
 };
 pub use line::LineTool;
 pub use move_tool::MoveTool;
 pub use push_pull::PushPullTool;
 pub use rectangle::RectangleTool;
+pub use scenery::{Owner, Part, Scenery};
 pub use select::SelectTool;
 pub use tool::{Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p tools`
Expected: all pass, including 5 in `tests/scenery.rs` and 2 in `scenery::tests`. Every existing inference test still passes through the one-mesh wrappers.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 450 tests pass.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/tools/tests/scenery.rs crates/tools/src/scenery.rs crates/tools/src/inference.rs crates/tools/src/lib.rs
git commit -m "See and snap to geometry in every placed mesh"
```

### Task 2: Give tools the open context's view and coordinates

**Files:**
- Modify: `crates/tools/src/tool.rs` (`ToolContext::scenery`), `crates/tools/src/select.rs` (`pick` through the scenery, open context only for now), `crates/tools/src/push_pull.rs` (grab only the open context's faces)
- Modify (test rigs gain `scenery`): `crates/tools/src/{erase,line,move_tool,rectangle}.rs`
- Modify: `crates/app/src/editor.rs`
- Test: three new tests at the end of `editor::tests` in `crates/app/src/editor.rs`

**Interfaces:**
- Consumes (Task 1): `Scenery::{of, single}`, `infer_in`, `edge_under_in`, `face_under_in`, `Owner`.
- Consumes (M5a): `Document::{scene, context, context_transform, model}`.
- Produces:
  - `ToolContext::scenery: &'a Scenery<'a>`, whose `open()` is `model`;
  - editor helpers `local_camera()` and `scenery()` (private);
  - `Editor::overlay()` draws in root coordinates.

- [ ] **Step 1: Write the failing tests**

The new editor tests and their helpers (`moved_group`, `has_edge`) are added at the end of the `tests` module of `crates/app/src/editor.rs`; they are the last hunk of that file's diff in Step 3. Apply that hunk first:
- `tools_inside_a_moved_group_work_in_its_coordinates`;
- `the_marker_follows_the_cursor_inside_a_moved_group`;
- `lines_from_the_root_snap_to_a_groups_corner`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app --bin app editor::tests`
Expected: the three new tests FAIL. Tools see only the open mesh with the root camera, so the clicks land in the wrong place, the marker is not at the corner, and nothing snaps to the group. The other editor tests pass.

- [ ] **Step 3: Implement**

Apply to `crates/tools/src/tool.rs`:
```diff
--- a/crates/tools/src/tool.rs
+++ b/crates/tools/src/tool.rs
@@ -11,13 +11,18 @@ use crate::inference::Inference;
 use crate::line::LineTool;
 use crate::move_tool::MoveTool;
 use crate::push_pull::PushPullTool;
 use crate::rectangle::RectangleTool;
+use crate::scenery::Scenery;
 use crate::select::SelectTool;
 
-/// What a tool may look at while handling an event.
+/// What a tool may look at while handling an event. Points, the camera and entity ids are in
+/// the open context's coordinates (see `session::Document::context`).
 pub struct ToolContext<'a> {
+    /// The open context's mesh: what tools edit.
     pub model: &'a Model,
+    /// Everything visible, for inference and picking; `scenery.open()` is `model`.
+    pub scenery: &'a Scenery<'a>,
     pub camera: &'a Camera,
     /// Cursor position in physical pixels.
     pub cursor: DVec2,
     /// How close (physical pixels) the cursor must be to an edge to pick it.
```

Apply to `crates/tools/src/select.rs`:
```diff
--- a/crates/tools/src/select.rs
+++ b/crates/tools/src/select.rs
@@ -6,9 +6,10 @@ use glam::{DVec2, DVec3};
 use kernel::geom::point_in_polygon;
 use kernel::{EntityId, Face, Model};
 use session::{Command, SelectMode};
 
-use crate::inference::{edge_under, face_under};
+use crate::inference::{edge_under_in, face_under_in};
+use crate::scenery::Owner;
 use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
 
 #[derive(Clone, Debug, Default)]
 pub struct SelectTool {
@@ -22,13 +23,20 @@ impl SelectTool {
         Self::default()
     }
 }
 
-/// The edge under the cursor, or else the face under it.
+/// The open context's edge under the cursor, or else its face under it. Geometry elsewhere
+/// (inside instances, outside the open instance) hides what is behind it but is not picked.
 pub(crate) fn pick(ctx: &ToolContext) -> Option<EntityId> {
-    edge_under(ctx.model, ctx.camera, ctx.cursor, ctx.pick_radius)
-        .map(EntityId::Edge)
-        .or_else(|| face_under(ctx.model, ctx.camera, ctx.cursor).map(|(f, _)| EntityId::Face(f)))
+    let open = |o: Owner| o == Owner::Open;
+    edge_under_in(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius)
+        .filter(|(o, ..)| open(*o))
+        .map(|(_, e, _, _)| EntityId::Edge(e))
+        .or_else(|| {
+            face_under_in(ctx.scenery, ctx.camera, ctx.cursor)
+                .filter(|(o, ..)| open(*o))
+                .map(|(_, f, _)| EntityId::Face(f))
+        })
 }
 
 /// Edges and faces selected by the screen box from `a` to `b`: inside it entirely when
 /// dragging right (`b.x >= a.x`), touching it when dragging left. Entities with a point behind
@@ -231,8 +239,9 @@ mod tests {
 
         fn send(&mut self, event: ToolEvent) -> ToolOutput {
             let ctx = ToolContext {
                 model: &self.model,
+                scenery: &crate::scenery::Scenery::single(&self.model),
                 camera: &self.camera,
                 cursor: self.cursor,
                 pick_radius: 10.0,
                 drag_threshold: self.drag_threshold,
```

Apply to `crates/tools/src/push_pull.rs`:
```diff
--- a/crates/tools/src/push_pull.rs
+++ b/crates/tools/src/push_pull.rs
@@ -5,9 +5,10 @@ use glam::DVec3;
 use kernel::geom::LENGTH_TOL;
 use kernel::{EntityId, FaceId, Model};
 use session::Command;
 
-use crate::inference::{Inference, SnapKind, closest_on_line_to_ray, face_under};
+use crate::inference::{Inference, SnapKind, closest_on_line_to_ray, face_under_in};
+use crate::scenery::Owner;
 use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{format_length, parse_length, toward_cursor};
 
 /// A push/pull in progress.
@@ -111,9 +112,12 @@ impl Tool for PushPullTool {
                 None => self.hover = hit.face.map(|f| loops(ctx.model, f)).unwrap_or_default(),
             },
             ToolEvent::Click(hit) => match self.drag.as_ref() {
                 None => {
-                    if let Some((face, start)) = face_under(ctx.model, ctx.camera, ctx.cursor) {
+                    let under = face_under_in(ctx.scenery, ctx.camera, ctx.cursor)
+                        .filter(|(owner, ..)| *owner == Owner::Open)
+                        .map(|(_, f, p)| (f, p));
+                    if let Some((face, start)) = under {
                         self.drag = Some(Drag {
                             face,
                             normal: ctx.model.face(face).unwrap().normal,
                             start,
@@ -169,8 +173,9 @@ impl Tool for PushPullTool {
 
 #[cfg(test)]
 mod tests {
     use super::*;
+    use crate::inference::face_under;
     use glam::DVec2;
     use session::Camera;
 
     /// A 2x2 rectangle on the ground and a camera looking at it.
@@ -215,8 +220,9 @@ mod tests {
 
         fn send(&mut self, event: ToolEvent) -> ToolOutput {
             let ctx = ToolContext {
                 model: &self.model,
+                scenery: &crate::scenery::Scenery::single(&self.model),
                 camera: &self.camera,
                 cursor: self.cursor,
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
```

Apply to `crates/tools/src/erase.rs`:
```diff
--- a/crates/tools/src/erase.rs
+++ b/crates/tools/src/erase.rs
@@ -99,8 +99,9 @@ mod tests {
         event: ToolEvent,
     ) -> ToolOutput {
         let ctx = ToolContext {
             model,
+            scenery: &crate::scenery::Scenery::single(model),
             camera,
             cursor: camera.project(at).unwrap(),
             pick_radius: 10.0,
             drag_threshold: 4.0,
```

Apply to `crates/tools/src/line.rs`:
```diff
--- a/crates/tools/src/line.rs
+++ b/crates/tools/src/line.rs
@@ -159,8 +159,9 @@ mod tests {
 
         fn send(&mut self, event: ToolEvent) -> ToolOutput {
             let ctx = ToolContext {
                 model: &self.model,
+                scenery: &crate::scenery::Scenery::single(&self.model),
                 camera: &self.camera,
                 cursor: DVec2::ZERO,
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
```

Apply to `crates/tools/src/move_tool.rs`:
```diff
--- a/crates/tools/src/move_tool.rs
+++ b/crates/tools/src/move_tool.rs
@@ -239,8 +239,9 @@ mod tests {
 
         fn send(&mut self, event: ToolEvent) -> ToolOutput {
             let ctx = ToolContext {
                 model: &self.model,
+                scenery: &crate::scenery::Scenery::single(&self.model),
                 camera: &self.camera,
                 cursor: self.cursor,
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
```

Apply to `crates/tools/src/rectangle.rs`:
```diff
--- a/crates/tools/src/rectangle.rs
+++ b/crates/tools/src/rectangle.rs
@@ -178,8 +178,9 @@ mod tests {
 
         fn send(&mut self, event: ToolEvent) -> ToolOutput {
             let ctx = ToolContext {
                 model: &self.model,
+                scenery: &crate::scenery::Scenery::single(&self.model),
                 camera: &self.camera,
                 cursor: self.cursor,
                 pick_radius: 10.0,
                 drag_threshold: 4.0,
```

Apply to `crates/app/src/editor.rs`:
```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -3,13 +3,15 @@
 
 use glam::{DVec2, DVec3};
 use kernel::EntityId;
 use render::grid::LineVertex;
-use session::{BatchError, Command, CommandError, Document, ErrorKind, Outcome, SelectMode};
+use session::{
+    BatchError, Camera, Command, CommandError, Document, ErrorKind, Outcome, SelectMode,
+};
 use tools::overlay::overlay;
 use tools::{
-    Axis, Guides, Inference, Lock, SnapKind, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput,
-    edge_under, infer_with,
+    Axis, Guides, Inference, Lock, Scenery, SnapKind, Tool, ToolContext, ToolEvent, ToolKind,
+    ToolOutput, edge_under_in, infer_in,
 };
 
 use crate::navigation::{Button, Navigation};
 
@@ -395,24 +397,30 @@ impl Editor {
             lock,
         }
     }
 
-    /// Line pairs for the renderer's overlay pass.
+    /// Line pairs for the renderer's overlay pass. Tools work in the open context's
+    /// coordinates; the overlay is drawn in the root's.
     pub fn overlay(&self) -> Vec<LineVertex> {
-        overlay(
-            &self.preview,
-            self.hit.as_ref(),
-            &self.doc.camera,
-            self.scale_factor,
-        )
-        .into_iter()
-        .flat_map(|s| {
-            [s.a, s.b].map(|p| LineVertex {
-                pos: p.as_vec3().to_array(),
-                color: s.color,
+        let t = self.doc.context_transform();
+        let preview: Vec<[DVec3; 2]> = self
+            .preview
+            .iter()
+            .map(|[a, b]| [t.transform_point3(*a), t.transform_point3(*b)])
+            .collect();
+        let hit = self.hit.map(|h| Inference {
+            point: t.transform_point3(h.point),
+            ..h
+        });
+        overlay(&preview, hit.as_ref(), &self.doc.camera, self.scale_factor)
+            .into_iter()
+            .flat_map(|s| {
+                [s.a, s.b].map(|p| LineVertex {
+                    pos: p.as_vec3().to_array(),
+                    color: s.color,
+                })
             })
-        })
-        .collect()
+            .collect()
     }
 
     /// Sends a click, release, typed value or cancel to the tool and runs what it asks for.
     fn send(&mut self, event: ToolEvent) {
@@ -426,12 +434,31 @@ impl Editor {
         self.apply(out);
         self.update_hover();
     }
 
+    /// The camera in the open context's coordinates. Placements are moves, so only the eye
+    /// and the target change.
+    fn local_camera(&self) -> Camera {
+        let back = self.doc.context_transform().inverse();
+        let mut camera = self.doc.camera.clone();
+        camera.eye = back.transform_point3(camera.eye);
+        camera.target = back.transform_point3(camera.target);
+        camera
+    }
+
+    /// Everything tools can see, in the open context's coordinates.
+    fn scenery(&self) -> Scenery<'_> {
+        Scenery::of(self.doc.scene(), self.doc.context()).expect("the open context exists")
+    }
+
     fn handle(&mut self, event: ToolEvent) -> ToolOutput {
+        let camera = self.local_camera();
+        let scenery =
+            Scenery::of(self.doc.scene(), self.doc.context()).expect("the open context exists");
         let ctx = ToolContext {
             model: self.doc.model(),
-            camera: &self.doc.camera,
+            scenery: &scenery,
+            camera: &camera,
             cursor: self.cursor,
             pick_radius: PICK_RADIUS_LOGICAL_PX * self.scale_factor,
             drag_threshold: DRAG_LOGICAL_PX * self.scale_factor,
             shift: self.shift,
@@ -511,21 +538,21 @@ impl Editor {
             self.lock = None;
             self.shift_lock = false;
         }
         let radius = PICK_RADIUS_LOGICAL_PX * self.scale_factor;
-        let (model, camera) = (self.doc.model(), &self.doc.camera);
+        let camera = self.local_camera();
+        let scenery = self.scenery();
         let guides = Guides {
             anchor,
             reference: self.reference,
             lock: self.lock,
         };
-        let hit = infer_with(model, camera, self.cursor, &guides, radius);
-        // The edge the cursor rests on becomes the reference for later direction snaps.
-        if let Some(e) = edge_under(model, camera, self.cursor, radius)
-            && let Some(edge) = model.edge(e)
-            && let (Some(a), Some(b)) = (model.position(edge.v[0]), model.position(edge.v[1]))
-        {
-            self.reference = (b - a).try_normalize();
+        let hit = infer_in(&scenery, &camera, self.cursor, &guides, radius);
+        // The edge the cursor rests on (anywhere) becomes the reference for direction snaps.
+        let reference = edge_under_in(&scenery, &camera, self.cursor, radius)
+            .and_then(|(_, _, a, b)| (b - a).try_normalize());
+        if reference.is_some() {
+            self.reference = reference;
         }
         let out = self.handle(ToolEvent::Move(hit));
         let changed = self.hit != Some(hit) || self.preview != out.preview;
         self.hit = Some(hit);
@@ -1181,5 +1208,97 @@ mod tests {
         assert!(e.status().contains("Locked: red axis"), "{}", e.status());
         assert!(!e.key_repeat(EditorKey::Unlock));
         assert!(e.status().contains("Locked: red axis"));
     }
+
+    /// A 2 x 2 x 1 box at the origin, grouped and moved by (10, 0, 0). Returns the instance.
+    fn moved_group(e: &mut Editor) -> EntityId {
+        e.execute(Command::AddRectangle {
+            corner: [0.0, 0.0, 0.0],
+            opposite: [2.0, 2.0, 0.0],
+            normal: None,
+        })
+        .unwrap();
+        e.execute(Command::PushPull {
+            face: "f8".parse().unwrap(),
+            distance: 1.0,
+        })
+        .unwrap();
+        let m = e.doc().model();
+        let ids = m
+            .edges()
+            .map(|(id, _)| EntityId::Edge(id))
+            .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
+            .collect();
+        e.execute(Command::MakeGroup { ids }).unwrap();
+        let group = *e.doc().selection().first().unwrap();
+        e.execute(Command::Move {
+            ids: vec![group],
+            offset: [10.0, 0.0, 0.0],
+        })
+        .unwrap();
+        group
+    }
+
+    /// True if the open context has an edge between these two points.
+    fn has_edge(e: &Editor, a: DVec3, b: DVec3) -> bool {
+        let m = e.doc().model();
+        m.edges().any(|(_, edge)| {
+            let (p, q) = (
+                m.position(edge.v[0]).unwrap(),
+                m.position(edge.v[1]).unwrap(),
+            );
+            (p.abs_diff_eq(a, 1e-9) && q.abs_diff_eq(b, 1e-9))
+                || (p.abs_diff_eq(b, 1e-9) && q.abs_diff_eq(a, 1e-9))
+        })
+    }
+
+    #[test]
+    fn tools_inside_a_moved_group_work_in_its_coordinates() {
+        let mut e = editor();
+        let group = moved_group(&mut e);
+        e.execute(Command::OpenInstance { id: group }).unwrap();
+        e.select_tool(ToolKind::Line);
+        click_at(&mut e, DVec3::new(10.0, 0.0, 1.0));
+        click_at(&mut e, DVec3::new(12.0, 2.0, 1.0));
+        e.key(EditorKey::Escape);
+        assert!(
+            has_edge(&e, DVec3::new(0.0, 0.0, 1.0), DVec3::new(2.0, 2.0, 1.0)),
+            "the diagonal is drawn in the group's own coordinates"
+        );
+        assert_eq!(stats(&e), (13, 7), "it splits the group's top");
+    }
+
+    #[test]
+    fn the_marker_follows_the_cursor_inside_a_moved_group() {
+        let mut e = editor();
+        let group = moved_group(&mut e);
+        e.execute(Command::OpenInstance { id: group }).unwrap();
+        e.select_tool(ToolKind::Line);
+        let corner = DVec3::new(10.0, 0.0, 1.0);
+        let px = e.doc().camera.project(corner).unwrap();
+        e.cursor_moved(px + DVec2::new(3.0, 2.0));
+        let overlay = e.overlay();
+        assert!(!overlay.is_empty());
+        assert!(
+            overlay
+                .iter()
+                .all(|v| DVec3::from(v.pos.map(f64::from)).distance(corner) < 0.5),
+            "the endpoint marker is drawn at the corner in root coordinates"
+        );
+    }
+
+    #[test]
+    fn lines_from_the_root_snap_to_a_groups_corner() {
+        let mut e = editor();
+        moved_group(&mut e);
+        e.select_tool(ToolKind::Line);
+        let corner = DVec3::new(10.0, 0.0, 0.0);
+        let px = e.doc().camera.project(corner).unwrap();
+        e.cursor_moved(px + DVec2::new(3.0, -2.0));
+        e.button(Button::Left, true);
+        e.button(Button::Left, false);
+        click_at(&mut e, DVec3::new(10.0, -3.0, 0.0));
+        e.key(EditorKey::Escape);
+        assert!(has_edge(&e, corner, DVec3::new(10.0, -3.0, 0.0)));
+    }
 }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p app --bin app editor::tests && cargo test -p tools`
Expected: all pass.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 453 tests pass.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/tools/src/tool.rs crates/tools/src/select.rs crates/tools/src/push_pull.rs crates/tools/src/erase.rs crates/tools/src/line.rs crates/tools/src/move_tool.rs crates/tools/src/rectangle.rs crates/app/src/editor.rs
git commit -m "Give tools the open context's view and coordinates"
```

### Task 3: Pick, box-select, move and open groups with the tools

**Files:**
- Modify: `crates/tools/src/tool.rs` (`ToolEvent::DoubleClick`), `crates/tools/src/select.rs` (instances in `pick` and `boxed`, `box_outline`, double-click, Escape), `crates/tools/src/move_tool.rs` and `crates/tools/src/erase.rs` (instance outlines, prompts), `crates/tools/src/{line,rectangle,push_pull}.rs` (ignore the new event)
- Modify: `crates/app/src/editor.rs` (`double_click`, `select_created` keeps only open-context ids), `crates/app/src/app.rs` (double-click detection)
- Test: six new tests at the end of `editor::tests`, and `double_clicks_are_two_quick_releases_in_one_place` in `app::tests`

**Interfaces:**
- Consumes (Tasks 1-2): `ToolContext::scenery`, `Scenery::{instance_box, inside_instance}`, `edge_under_in`, `face_under_in`.
- Produces:
  - `ToolEvent::DoubleClick(Inference)`;
  - `select::box_outline(&ToolContext, EntityId) -> Vec<[DVec3; 2]>` (`pub(crate)`);
  - `Editor::double_click(&mut self) -> bool`;
  - in `app.rs`: `is_double_click(Option<(Instant, DVec2)>, Instant, DVec2, f64) -> bool`, `DOUBLE_CLICK`, `DOUBLE_CLICK_LOGICAL_PX`.

- [ ] **Step 1: Write the failing tests**

The new tests are the last hunks of the Step 3 diffs of `crates/app/src/editor.rs` (helpers `clear_selection`, `double_click_at` and `box_min`, plus six tests) and `crates/app/src/app.rs` (`double_clicks_are_two_quick_releases_in_one_place`). Apply those test hunks first. The editor tests are:
- `clicking_a_group_selects_it`;
- `double_clicking_a_group_opens_it_and_escape_closes_it`;
- `double_clicking_outside_the_open_group_closes_it`;
- `a_window_around_a_group_selects_it`;
- `the_move_tool_moves_a_selected_group`;
- `copying_a_group_selects_the_copy`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app --bin app`
Expected: compile errors: no method named `double_click` on `Editor`, cannot find function `is_double_click`.

- [ ] **Step 3: Implement**

Apply to `crates/tools/src/tool.rs`:
```diff
--- a/crates/tools/src/tool.rs
+++ b/crates/tools/src/tool.rs
@@ -42,8 +42,11 @@ pub enum ToolEvent {
     /// The primary button was pressed at the current inference.
     Click(Inference),
     /// The primary button was released at the current inference.
     Release(Inference),
+    /// The primary button was double-clicked at the current inference (sent after the second
+    /// release).
+    DoubleClick(Inference),
     /// Text typed into the measurements box, submitted with Enter.
     Value(String),
     /// Escape: abandon the operation in progress.
     Cancel,
```

Apply to `crates/tools/src/select.rs`:
```diff
--- a/crates/tools/src/select.rs
+++ b/crates/tools/src/select.rs
@@ -1,7 +1,9 @@
-//! The Select tool: click an edge or face to select it, or drag a box. Dragging right selects
-//! what lies entirely inside the box; dragging left also selects what the box touches. With
-//! Shift held, clicked or boxed entities are toggled instead of replacing the selection.
+//! The Select tool: click an edge, face or group to select it, or drag a box. Dragging right
+//! selects what lies entirely inside the box; dragging left also selects what the box touches.
+//! With Shift held, clicked or boxed entities are toggled instead of replacing the selection.
+//! Double-clicking a group opens it for editing; double-clicking outside it, or Escape, closes
+//! it.
 
 use glam::{DVec2, DVec3};
 use kernel::geom::point_in_polygon;
 use kernel::{EntityId, Face, Model};
@@ -23,25 +25,55 @@ impl SelectTool {
         Self::default()
     }
 }
 
-/// The open context's edge under the cursor, or else its face under it. Geometry elsewhere
-/// (inside instances, outside the open instance) hides what is behind it but is not picked.
+/// What a click picks: the open context's edge under the cursor, or else its face under it.
+/// An edge or face inside an instance of the open context picks the instance. Geometry outside
+/// the open instance hides what is behind it but is not picked.
 pub(crate) fn pick(ctx: &ToolContext) -> Option<EntityId> {
-    let open = |o: Owner| o == Owner::Open;
-    edge_under_in(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius)
-        .filter(|(o, ..)| open(*o))
-        .map(|(_, e, _, _)| EntityId::Edge(e))
-        .or_else(|| {
-            face_under_in(ctx.scenery, ctx.camera, ctx.cursor)
-                .filter(|(o, ..)| open(*o))
-                .map(|(_, f, _)| EntityId::Face(f))
-        })
+    let edge = edge_under_in(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius);
+    match edge {
+        Some((Owner::Open, e, _, _)) => return Some(EntityId::Edge(e)),
+        Some((Owner::Instance(i), ..)) => return Some(EntityId::Instance(i)),
+        _ => {}
+    }
+    match face_under_in(ctx.scenery, ctx.camera, ctx.cursor)? {
+        (Owner::Open, f, _) => Some(EntityId::Face(f)),
+        (Owner::Instance(i), _, _) => Some(EntityId::Instance(i)),
+        (Owner::Outside, _, _) => None,
+    }
+}
+
+/// The corners of an instance's bounding box (bit 0 picks x, bit 1 y, bit 2 z).
+fn box_corners(ctx: &ToolContext, id: EntityId) -> Option<[DVec3; 8]> {
+    let EntityId::Instance(i) = id else {
+        return None;
+    };
+    let [lo, hi] = ctx.scenery.instance_box(i)?;
+    Some(std::array::from_fn(|k| {
+        DVec3::new(
+            if k & 1 == 0 { lo.x } else { hi.x },
+            if k & 2 == 0 { lo.y } else { hi.y },
+            if k & 4 == 0 { lo.z } else { hi.z },
+        )
+    }))
+}
+
+/// The twelve edges of an instance's bounding box, for outlines.
+pub(crate) fn box_outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
+    let Some(c) = box_corners(ctx, id) else {
+        return Vec::new();
+    };
+    (0..8)
+        .flat_map(|k| [1, 2, 4].map(|bit| (k, k | bit)))
+        .filter(|(a, b)| a != b)
+        .map(|(a, b)| [c[a], c[b]])
+        .collect()
 }
 
-/// Edges and faces selected by the screen box from `a` to `b`: inside it entirely when
-/// dragging right (`b.x >= a.x`), touching it when dragging left. Entities with a point behind
-/// the camera are skipped.
+/// Edges, faces and instances selected by the screen box from `a` to `b`: inside it entirely
+/// when dragging right (`b.x >= a.x`), touching it when dragging left. Entities with a point
+/// behind the camera are skipped.
 fn boxed(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<EntityId> {
     let (lo, hi) = (a.min(b), a.max(b));
     let window = b.x >= a.x;
     let m = ctx.model;
@@ -83,8 +115,34 @@ fn boxed(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<EntityId> {
         corners
             .iter()
             .any(|c| point_in_polygon(*c, &outer) && !holes.iter().any(|h| point_in_polygon(*c, h)))
     };
+    // Instances by their bounding boxes: inside when every corner is, touching when the box's
+    // screen rectangle overlaps the selection box.
+    for (id, _) in m.instances() {
+        let id = EntityId::Instance(id);
+        let Some(corners) = box_corners(ctx, id) else {
+            continue;
+        };
+        let Some(pts) = corners
+            .iter()
+            .map(|p| ctx.camera.project(*p))
+            .collect::<Option<Vec<DVec2>>>()
+        else {
+            continue;
+        };
+        let hit = if window {
+            pts.iter().all(|p| inside(*p))
+        } else {
+            let (bl, bh) = pts
+                .iter()
+                .fold((pts[0], pts[0]), |(l, h), p| (l.min(*p), h.max(*p)));
+            bl.cmple(hi).all() && bh.cmpge(lo).all()
+        };
+        if hit {
+            ids.push(id);
+        }
+    }
     for (id, f) in m.faces() {
         let all = face_loops(f).all(|l| screen(l).is_some_and(|pts| !pts.is_empty()));
         let hit = if window {
             all && face_loops(f).all(|l| screen(l).is_some_and(|pts| chosen(true, &pts)))
@@ -128,9 +186,11 @@ impl Tool for SelectTool {
     }
 
     fn prompt(&self) -> &'static str {
         match self.press {
-            None => "Click an edge or face, or drag a box; Shift adds or removes",
+            None => {
+                "Click an edge, face or group, or drag a box; Shift adds or removes; double-click a group to edit it"
+            }
             Some(_) => "Release to select: drag right for inside the box, left for touching",
         }
     }
 
@@ -168,9 +228,23 @@ impl Tool for SelectTool {
             }
             ToolEvent::Value(_) => {
                 out.message = Some("The Select tool takes no typed values".into());
             }
-            ToolEvent::Cancel => self.press = None,
+            ToolEvent::DoubleClick(_) => match pick(ctx) {
+                Some(id @ EntityId::Instance(_)) => {
+                    out.commands.push(Command::OpenInstance { id });
+                }
+                None if ctx.scenery.inside_instance() => {
+                    out.commands.push(Command::CloseInstance);
+                }
+                _ => {}
+            },
+            // Escape with nothing in progress closes the open instance.
+            ToolEvent::Cancel => {
+                if self.press.take().is_none() && ctx.scenery.inside_instance() {
+                    out.commands.push(Command::CloseInstance);
+                }
+            }
             ToolEvent::ToggleCopy => {}
         }
         if let Some(start) = self.press
             && start.distance(self.cursor) >= ctx.drag_threshold
```

Apply to `crates/tools/src/move_tool.rs`:
```diff
--- a/crates/tools/src/move_tool.rs
+++ b/crates/tools/src/move_tool.rs
@@ -7,9 +7,9 @@ use glam::DVec3;
 use kernel::EntityId;
 use kernel::geom::LENGTH_TOL;
 use session::{Command, MAX_COPIES};
 
-use crate::select::{face_outline, pick};
+use crate::select::{box_outline, face_outline, pick};
 use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{Array, format_length, parse_array, parse_length};
 
 /// A move in progress.
@@ -75,9 +75,9 @@ impl MoveTool {
         });
     }
 }
 
-/// Segments outlining edges and faces.
+/// Segments outlining edges, faces and instances (their bounding boxes).
 fn outline(ctx: &ToolContext, ids: &[EntityId]) -> Vec<[DVec3; 2]> {
     let m = ctx.model;
     let mut out = Vec::new();
     for id in ids {
@@ -93,9 +93,10 @@ fn outline(ctx: &ToolContext, ids: &[EntityId]) -> Vec<[DVec3; 2]> {
                 if let Some(f) = m.face(f) {
                     out.extend(face_outline(m, f));
                 }
             }
-            EntityId::Vertex(_) | EntityId::Instance(_) => {}
+            EntityId::Instance(_) => out.extend(box_outline(ctx, *id)),
+            EntityId::Vertex(_) => {}
         }
     }
     out
 }
@@ -109,10 +110,12 @@ impl Tool for MoveTool {
         match (&self.drag, self.copy, &self.last_copy) {
             (Some(_), false, _) => "Click the destination, or type a distance",
             (Some(_), true, _) => "Copy: click the destination, or type a distance",
             (None, _, Some(_)) => "Type 5x or /5 for more copies, or click a new base point",
-            (None, false, None) => "Select geometry, then click a base point (Ctrl copies)",
-            (None, true, None) => "Copy: select geometry, then click a base point",
+            (None, false, None) => {
+                "Select geometry or a group, then click a base point (Ctrl copies)"
+            }
+            (None, true, None) => "Copy: select geometry or a group, then click a base point",
         }
     }
 
     fn measurement(&self) -> (&'static str, Option<String>) {
@@ -139,9 +142,10 @@ impl Tool for MoveTool {
                     } else {
                         ctx.selection.iter().copied().collect()
                     };
                     if ids.is_empty() {
-                        out.message = Some("Select geometry, or click an edge or face".into());
+                        out.message =
+                            Some("Select geometry, or click an edge, face or group".into());
                     } else {
                         self.drag = Some(Drag {
                             base: hit.point,
                             outline: outline(ctx, &ids),
@@ -180,9 +184,9 @@ impl Tool for MoveTool {
                 },
             },
             ToolEvent::ToggleCopy => self.copy = !self.copy,
             ToolEvent::Cancel => self.drag = None,
-            ToolEvent::Release(_) => {}
+            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) => {}
         }
         if let (Some(d), Some(hover)) = (&self.drag, self.hover) {
             let offset = hover - d.base;
             out.preview = d.outline.clone();
```

Apply to `crates/tools/src/erase.rs`:
```diff
--- a/crates/tools/src/erase.rs
+++ b/crates/tools/src/erase.rs
@@ -1,11 +1,12 @@
-//! The Erase tool: click an edge or face to erase it. The entity under the cursor is outlined.
+//! The Erase tool: click an edge, face or group to erase it. What is under the cursor is
+//! outlined.
 
 use glam::DVec3;
 use kernel::EntityId;
 use session::Command;
 
-use crate::select::{face_outline, pick};
+use crate::select::{box_outline, face_outline, pick};
 use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
 
 #[derive(Clone, Debug, Default)]
 pub struct EraseTool {
@@ -27,9 +28,10 @@ fn outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
             .and_then(|e| Some([m.position(e.v[0])?, m.position(e.v[1])?]))
             .into_iter()
             .collect(),
         EntityId::Face(f) => m.face(f).map(|f| face_outline(m, f)).unwrap_or_default(),
-        EntityId::Vertex(_) | EntityId::Instance(_) => Vec::new(),
+        EntityId::Instance(_) => box_outline(ctx, id),
+        EntityId::Vertex(_) => Vec::new(),
     }
 }
 
 impl Tool for EraseTool {
@@ -37,9 +39,9 @@ impl Tool for EraseTool {
         "Erase"
     }
 
     fn prompt(&self) -> &'static str {
-        "Click an edge or face to erase it"
+        "Click an edge, face or group to erase it"
     }
 
     fn measurement(&self) -> (&'static str, Option<String>) {
         ("", None)
@@ -63,9 +65,12 @@ impl Tool for EraseTool {
             }
             ToolEvent::Value(_) => {
                 out.message = Some("The Erase tool takes no typed values".into());
             }
-            ToolEvent::Release(_) | ToolEvent::Cancel | ToolEvent::ToggleCopy => {}
+            ToolEvent::Release(_)
+            | ToolEvent::DoubleClick(_)
+            | ToolEvent::Cancel
+            | ToolEvent::ToggleCopy => {}
         }
         out.preview = self.outline.clone();
         out
     }
```

Apply to `crates/tools/src/line.rs`:
```diff
--- a/crates/tools/src/line.rs
+++ b/crates/tools/src/line.rs
@@ -101,9 +101,9 @@ impl Tool for LineTool {
                         self.place(last, p, &mut out);
                     }
                 }
             }
-            ToolEvent::Release(_) | ToolEvent::ToggleCopy => {}
+            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::ToggleCopy => {}
             ToolEvent::Cancel => self.reset(),
         }
         if let (Some(a), Some(b)) = (self.last, self.hover)
             && a.distance(b) >= LENGTH_TOL
```

Apply to `crates/tools/src/rectangle.rs`:
```diff
--- a/crates/tools/src/rectangle.rs
+++ b/crates/tools/src/rectangle.rs
@@ -125,9 +125,9 @@ impl Tool for RectangleTool {
                     }
                 }
                 (Some(_), _) => out.message = Some(format!("Not width,height: {text}")),
             },
-            ToolEvent::Release(_) | ToolEvent::ToggleCopy => {}
+            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::ToggleCopy => {}
             ToolEvent::Cancel => self.on_external_change(),
         }
         if let (Some((corner, normal)), Some(hover)) = (self.start, self.hover)
             && let Ok(c) = rectangle_corners(corner, hover, Some(normal))
```

Apply to `crates/tools/src/push_pull.rs`:
```diff
--- a/crates/tools/src/push_pull.rs
+++ b/crates/tools/src/push_pull.rs
@@ -145,9 +145,9 @@ impl Tool for PushPullTool {
                     let v = toward_cursor(d.distance, v);
                     self.finish(v, &mut out);
                 }
             },
-            ToolEvent::Release(_) | ToolEvent::ToggleCopy => {}
+            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::ToggleCopy => {}
             ToolEvent::Cancel => self.on_external_change(),
         }
         match &self.drag {
             Some(d) => {
```

Apply to `crates/app/src/editor.rs`:
```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -266,8 +266,18 @@ impl Editor {
         });
         true
     }
 
+    /// The primary button was double-clicked (call it after the second release). The Select
+    /// tool opens the group under the cursor, or closes the open one when there is nothing of
+    /// it there.
+    pub fn double_click(&mut self) -> bool {
+        self.update_hover();
+        let Some(hit) = self.hit else { return false };
+        self.send(ToolEvent::DoubleClick(hit));
+        true
+    }
+
     pub fn scroll(&mut self, steps: f64) -> bool {
         let zoomed = self.nav.scroll(&mut self.doc.camera, steps);
         if zoomed {
             self.update_hover();
@@ -497,15 +507,22 @@ impl Editor {
         }
         self.preview = out.preview;
     }
 
-    /// Selects the edges and faces a command created.
+    /// Selects the edges, faces and instances a command created in the open context (a copied
+    /// group's new contents live in its new definition and are left out).
     fn select_created(&mut self, outcome: Outcome) {
+        let m = self.doc.model();
         let ids = outcome
             .changes
             .created
             .into_iter()
-            .filter(|id| !matches!(id, EntityId::Vertex(_)))
+            .filter(|id| match *id {
+                EntityId::Edge(e) => m.edge(e).is_some(),
+                EntityId::Face(f) => m.face(f).is_some(),
+                EntityId::Instance(i) => m.instance(i).is_some(),
+                EntityId::Vertex(_) => false,
+            })
             .collect();
         self.run(Command::Select {
             ids,
             mode: SelectMode::Replace,
@@ -1300,5 +1317,131 @@ mod tests {
         click_at(&mut e, DVec3::new(10.0, -3.0, 0.0));
         e.key(EditorKey::Escape);
         assert!(has_edge(&e, corner, DVec3::new(10.0, -3.0, 0.0)));
     }
+
+    fn clear_selection(e: &mut Editor) {
+        e.execute(Command::Select {
+            ids: Vec::new(),
+            mode: SelectMode::Replace,
+        })
+        .unwrap();
+    }
+
+    fn double_click_at(e: &mut Editor, p: DVec3) {
+        click_at(e, p);
+        click_at(e, p);
+        e.double_click();
+    }
+
+    /// The minimum corner of an instance's bounding box in the open context.
+    fn box_min(e: &Editor, id: EntityId) -> DVec3 {
+        match e.doc().query(&Query::Entity { id }).unwrap() {
+            QueryResult::Instance(info) => DVec3::from(info.bounds.unwrap().min),
+            other => panic!("unexpected {other:?}"),
+        }
+    }
+
+    #[test]
+    fn clicking_a_group_selects_it() {
+        let mut e = editor();
+        let group = moved_group(&mut e);
+        clear_selection(&mut e);
+        e.select_tool(ToolKind::Select);
+        click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
+        assert_eq!(
+            e.doc().selection().iter().copied().collect::<Vec<_>>(),
+            [group]
+        );
+    }
+
+    #[test]
+    fn double_clicking_a_group_opens_it_and_escape_closes_it() {
+        let mut e = editor();
+        let group = moved_group(&mut e);
+        e.select_tool(ToolKind::Select);
+        double_click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
+        let open: Vec<EntityId> = e
+            .doc()
+            .context()
+            .iter()
+            .map(|i| EntityId::Instance(*i))
+            .collect();
+        assert_eq!(open, [group]);
+        e.key(EditorKey::Escape);
+        assert!(e.doc().context().is_empty());
+    }
+
+    #[test]
+    fn double_clicking_outside_the_open_group_closes_it() {
+        let mut e = editor();
+        let group = moved_group(&mut e);
+        e.execute(Command::OpenInstance { id: group }).unwrap();
+        e.select_tool(ToolKind::Select);
+        double_click_at(&mut e, DVec3::new(-3.0, -3.0, 0.0));
+        assert!(e.doc().context().is_empty());
+    }
+
+    #[test]
+    fn a_window_around_a_group_selects_it() {
+        let mut e = editor();
+        let group = moved_group(&mut e);
+        clear_selection(&mut e);
+        e.select_tool(ToolKind::Select);
+        let corners: Vec<DVec2> = [0.0, 2.0]
+            .into_iter()
+            .flat_map(|x| [0.0, 2.0].map(move |y| (x, y)))
+            .flat_map(|(x, y)| [0.0, 1.0].map(move |z| DVec3::new(10.0 + x, y, z)))
+            .map(|p| e.doc().camera.project(p).unwrap())
+            .collect();
+        let lo = corners.iter().fold(corners[0], |a, b| a.min(*b)) - DVec2::splat(20.0);
+        let hi = corners.iter().fold(corners[0], |a, b| a.max(*b)) + DVec2::splat(20.0);
+        e.cursor_moved(lo);
+        e.button(Button::Left, true);
+        e.cursor_moved(hi);
+        e.button(Button::Left, false);
+        assert_eq!(
+            e.doc().selection().iter().copied().collect::<Vec<_>>(),
+            [group]
+        );
+    }
+
+    #[test]
+    fn the_move_tool_moves_a_selected_group() {
+        let mut e = editor();
+        let group = moved_group(&mut e);
+        e.select_tool(ToolKind::Move);
+        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
+        let px = e.doc().camera.project(DVec3::new(10.0, 3.0, 0.0)).unwrap();
+        e.cursor_moved(px);
+        for c in "5".chars() {
+            e.key(EditorKey::Char(c));
+        }
+        e.key(EditorKey::Enter);
+        assert!(
+            box_min(&e, group).abs_diff_eq(DVec3::new(10.0, 5.0, 0.0), 1e-6),
+            "{:?}",
+            box_min(&e, group)
+        );
+    }
+
+    #[test]
+    fn copying_a_group_selects_the_copy() {
+        let mut e = editor();
+        let group = moved_group(&mut e);
+        e.select_tool(ToolKind::Move);
+        e.toggle_copy();
+        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
+        let px = e.doc().camera.project(DVec3::new(10.0, 3.0, 0.0)).unwrap();
+        e.cursor_moved(px);
+        for c in "4".chars() {
+            e.key(EditorKey::Char(c));
+        }
+        e.key(EditorKey::Enter);
+        let selected: Vec<EntityId> = e.doc().selection().iter().copied().collect();
+        assert_eq!(selected.len(), 1);
+        assert_ne!(selected[0], group);
+        assert!(matches!(selected[0], EntityId::Instance(_)));
+        assert_eq!(e.status_parts().message, None);
+        assert!(box_min(&e, selected[0]).abs_diff_eq(DVec3::new(10.0, 4.0, 0.0), 1e-6));
+    }
 }
```

Apply to `crates/app/src/app.rs`:
```diff
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -64,8 +64,19 @@ fn wake_decision(now: Instant, deadline: Option<Instant>) -> Wake {
 
 /// Trackpad pixel deltas per zoom step.
 const PIXELS_PER_SCROLL_STEP: f64 = 50.0;
 
+/// Longest time between the two releases of a double-click.
+const DOUBLE_CLICK: Duration = Duration::from_millis(500);
+/// Farthest the cursor may move between them, in logical pixels.
+const DOUBLE_CLICK_LOGICAL_PX: f64 = 6.0;
+
+/// True if a release at `at` (physical pixels) at time `now` completes a double-click begun
+/// by the release `last`.
+fn is_double_click(last: Option<(Instant, DVec2)>, now: Instant, at: DVec2, max_px: f64) -> bool {
+    last.is_some_and(|(t, p)| now.duration_since(t) <= DOUBLE_CLICK && p.distance(at) <= max_px)
+}
+
 struct State {
     window: Arc<Window>,
     surface: wgpu::Surface<'static>,
     device: wgpu::Device,
@@ -84,8 +95,10 @@ struct State {
     /// Last cursor position in window coordinates (physical pixels).
     cursor: DVec2,
     /// When egui wants its next frame without any input (e.g. a tooltip delay).
     repaint_at: Option<Instant>,
+    /// When and where the left button was last released in the view, for double-clicks.
+    last_release: Option<(Instant, DVec2)>,
 }
 
 impl State {
     fn new(window: Arc<Window>) -> Self {
@@ -143,8 +156,9 @@ impl State {
             modifiers: ModifiersState::empty(),
             view,
             cursor: DVec2::ZERO,
             repaint_at: None,
+            last_release: None,
         }
     }
 
     /// True if `p` (window coordinates) lies in the 3D view.
@@ -492,9 +506,20 @@ impl ApplicationHandler<ApiWake> for App {
                 let pressed = state == ElementState::Pressed;
                 if pressed && (s.gui.wants_pointer() || !s.in_view(s.cursor)) {
                     false
                 } else {
-                    s.editor.button(map_button(button), pressed)
+                    let handled = s.editor.button(map_button(button), pressed);
+                    if handled && !pressed && button == MouseButton::Left {
+                        let now = Instant::now();
+                        let max = DOUBLE_CLICK_LOGICAL_PX * s.window.scale_factor();
+                        if is_double_click(s.last_release, now, s.cursor, max) {
+                            s.last_release = None;
+                            s.editor.double_click();
+                        } else {
+                            s.last_release = Some((now, s.cursor));
+                        }
+                    }
+                    handled
                 }
             }
             WindowEvent::MouseWheel { .. } if s.gui.wants_pointer() || !s.in_view(s.cursor) => {
                 false
@@ -563,8 +588,30 @@ mod tests {
             Repaint::Idle
         );
     }
 
+    #[test]
+    fn double_clicks_are_two_quick_releases_in_one_place() {
+        let now = Instant::now();
+        let at = DVec2::new(100.0, 100.0);
+        let soon = now + Duration::from_millis(300);
+        assert!(!is_double_click(None, now, at, 6.0));
+        assert!(is_double_click(
+            Some((now, at)),
+            soon,
+            at + DVec2::new(3.0, 3.0),
+            6.0
+        ));
+        assert!(!is_double_click(
+            Some((now, at)),
+            soon,
+            at + DVec2::new(10.0, 0.0),
+            6.0
+        ));
+        let late = now + Duration::from_millis(800);
+        assert!(!is_double_click(Some((now, at)), late, at, 6.0));
+    }
+
     #[test]
     fn the_loop_wakes_at_the_deadline() {
         let now = Instant::now();
         let later = now + Duration::from_millis(300);
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p app --bin app && cargo test -p tools`
Expected: all pass.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 460 tests pass.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/tools/src/tool.rs crates/tools/src/select.rs crates/tools/src/move_tool.rs crates/tools/src/erase.rs crates/tools/src/line.rs crates/tools/src/rectangle.rs crates/tools/src/push_pull.rs crates/app/src/editor.rs crates/app/src/app.rs
git commit -m "Pick, box-select, move and open groups with the tools"
```

### Task 4: Group and component buttons, shortcuts and editing line

**Files:**
- Modify: `crates/app/src/editor.rs` (`make_group`, `make_component`, `explode_selection`, `close_instance`, `g`/`c` keys), `crates/app/src/ui.rs` (buttons, `View` fields, `UiAction` variants), `crates/app/src/app.rs` (actions, view fields, physical `g`/`c`), `crates/app/src/info.rs` ("Editing ..." line)
- Modify: `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md` (section 2 note, status line)
- Test: two new editor tests, `says_what_is_being_edited` in `info::tests`, and the extended `clicking_a_tool_button_reports_it` in `ui::tests`

**Interfaces:**
- Consumes: the M5a commands `MakeGroup`, `MakeComponent`, `Explode`, `CloseInstance`; `Document::{scene, context, selection}`; `DefinitionKind`.
- Produces:
  - `Editor::{make_group, make_component, explode_selection, close_instance}(&mut self) -> bool`;
  - `UiAction::{MakeGroup, MakeComponent, Explode, CloseInstance}`;
  - `ui::View::{has_instance_selected, inside_instance}`.

- [ ] **Step 1: Write the failing tests**

The tests are the test hunks of the Step 3 diffs:
- `editor.rs`: helper `definition_names`, `g_and_c_group_the_selection_and_explode_takes_it_apart`, `close_instance_goes_back_out`;
- `info.rs`: `says_what_is_being_edited`;
- `ui.rs`: `clicking_a_tool_button_reports_it` scans up to x = 1000, the `View` literals gain the two new fields, and the new actions are asserted.

Apply them first.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app --bin app`
Expected: compile errors: no method `explode_selection` / `close_instance` on `Editor`, no variant `UiAction::MakeGroup` (and the other three), no fields `has_instance_selected` / `inside_instance` on `View`.

- [ ] **Step 3: Implement**

Apply to `crates/app/src/editor.rs`:
```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -285,19 +285,25 @@ impl Editor {
         zoomed
     }
 
     /// Typing goes to the measurements box once it starts with a digit, ".", "-", "/" or "*"
-    /// (or "x" in the Move tool, for arrays like "x5"); other letters are tool shortcuts. Enter submits the typed text to the tool. Escape clears the
-    /// typed text, or else cancels the tool's operation. Delete, and Backspace with nothing
-    /// typed, erase the selection. Returns true if anything changed.
+    /// (or "x" in the Move tool, for arrays like "x5"); "g" makes a group and "c" a component
+    /// of the selection; other letters are tool shortcuts. Enter submits the typed text to the
+    /// tool. Escape clears the typed text, or else cancels the tool's operation (the Select
+    /// tool then closes the open group). Delete, and Backspace with nothing typed, erase the
+    /// selection. Returns true if anything changed.
     pub fn key(&mut self, key: EditorKey) -> bool {
         match key {
             EditorKey::Char(c) if self.typed.is_empty() => {
                 let starts_value = c.is_ascii_digit() || matches!(c, '.' | '-' | '/' | '*');
                 let starts_array = matches!(c, 'x' | 'X') && self.kind == ToolKind::Move;
                 if starts_value || starts_array {
                     self.typed.push(c);
                     true
+                } else if c.eq_ignore_ascii_case(&'g') {
+                    self.make_group()
+                } else if c.eq_ignore_ascii_case(&'c') {
+                    self.make_component()
                 } else if let Some(kind) = ToolKind::from_shortcut(c) {
                     self.select_tool(kind);
                     true
                 } else {
@@ -362,8 +368,74 @@ impl Editor {
         self.update_hover();
         true
     }
 
+    /// Makes a group of the selection. False if nothing is selected.
+    pub fn make_group(&mut self) -> bool {
+        let ids: Vec<EntityId> = self.doc.selection().iter().copied().collect();
+        if ids.is_empty() {
+            self.message = Some("Select geometry to make a group".into());
+            return false;
+        }
+        self.run_selection_command(Command::MakeGroup { ids })
+    }
+
+    /// Makes a component of the selection, named "Component N" with the first free N. False
+    /// if nothing is selected.
+    pub fn make_component(&mut self) -> bool {
+        let ids: Vec<EntityId> = self.doc.selection().iter().copied().collect();
+        if ids.is_empty() {
+            self.message = Some("Select geometry to make a component".into());
+            return false;
+        }
+        let names: Vec<&str> = self
+            .doc
+            .scene()
+            .definitions()
+            .map(|(_, d)| d.name.as_str())
+            .collect();
+        let name = (1..)
+            .map(|n| format!("Component {n}"))
+            .find(|n| !names.contains(&n.as_str()))
+            .expect("a free name");
+        self.run_selection_command(Command::MakeComponent { ids, name })
+    }
+
+    /// Explodes the selected groups and components. False if none is selected.
+    pub fn explode_selection(&mut self) -> bool {
+        let ids: Vec<EntityId> = self
+            .doc
+            .selection()
+            .iter()
+            .copied()
+            .filter(|id| matches!(id, EntityId::Instance(_)))
+            .collect();
+        if ids.is_empty() {
+            self.message = Some("Select a group or component to explode".into());
+            return false;
+        }
+        self.run_selection_command(Command::Explode { ids })
+    }
+
+    /// Closes the open group or component. False if none is open.
+    pub fn close_instance(&mut self) -> bool {
+        if self.doc.context().is_empty() {
+            return false;
+        }
+        self.run_selection_command(Command::CloseInstance)
+    }
+
+    /// Runs a command that works on the selection or the open context, dropping the tool's
+    /// operation and any typed text first.
+    fn run_selection_command(&mut self, command: Command) -> bool {
+        self.reset_tool();
+        self.typed.clear();
+        self.message = None;
+        self.run(command);
+        self.update_hover();
+        true
+    }
+
     pub fn undo(&mut self) -> bool {
         self.reset_tool();
         self.run(Command::Undo);
         self.update_hover();
@@ -1443,5 +1515,58 @@ mod tests {
         assert!(matches!(selected[0], EntityId::Instance(_)));
         assert_eq!(e.status_parts().message, None);
         assert!(box_min(&e, selected[0]).abs_diff_eq(DVec3::new(10.0, 4.0, 0.0), 1e-6));
     }
+
+    /// Names of the definitions in the model, in id order.
+    fn definition_names(e: &Editor) -> Vec<String> {
+        e.doc()
+            .scene()
+            .definitions()
+            .map(|(_, d)| d.name.clone())
+            .collect()
+    }
+
+    #[test]
+    fn g_and_c_group_the_selection_and_explode_takes_it_apart() {
+        let mut e = editor();
+        assert!(!e.key(EditorKey::Char('g')), "nothing selected");
+        assert!(e.status().contains("Select geometry to make a group"));
+        square(&mut e);
+        e.select_tool(ToolKind::Select);
+        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
+        assert!(e.key(EditorKey::Char('g')));
+        let group = *e.doc().selection().first().unwrap();
+        assert!(matches!(group, EntityId::Instance(_)));
+        assert_eq!(definition_names(&e), [""]);
+        assert!(e.key(EditorKey::Char('c')));
+        assert!(e.key(EditorKey::Char('C')));
+        assert_eq!(
+            definition_names(&e),
+            ["", "Component 1", "Component 2"],
+            "each component gets the next free name"
+        );
+        assert!(e.explode_selection());
+        assert!(e.explode_selection());
+        assert!(e.explode_selection());
+        assert_eq!(
+            definition_names(&e).len(),
+            3,
+            "the components stay, and so does the group inside Component 1"
+        );
+        assert_eq!(stats(&e), (4, 1), "the square is back in the root");
+        assert!(
+            !e.explode_selection(),
+            "the square's edges and face are no group"
+        );
+    }
+
+    #[test]
+    fn close_instance_goes_back_out() {
+        let mut e = editor();
+        assert!(!e.close_instance());
+        let group = moved_group(&mut e);
+        e.execute(Command::OpenInstance { id: group }).unwrap();
+        assert!(e.close_instance());
+        assert!(e.doc().context().is_empty());
+    }
 }
```

Apply to `crates/app/src/ui.rs`:
```diff
--- a/crates/app/src/ui.rs
+++ b/crates/app/src/ui.rs
@@ -11,16 +11,24 @@ pub enum UiAction {
     Tool(ToolKind),
     Undo,
     Redo,
     EraseSelection,
+    MakeGroup,
+    MakeComponent,
+    Explode,
+    CloseInstance,
 }
 
 /// What the panels show.
 pub struct View<'a> {
     pub tool: ToolKind,
     pub status: &'a Status,
     pub info: &'a [String],
     pub has_selection: bool,
+    /// Whether a group or component is selected.
+    pub has_instance_selected: bool,
+    /// Whether a group or component is open for editing.
+    pub inside_instance: bool,
 }
 
 /// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
 /// the 3D view, in points.
@@ -52,8 +60,34 @@ pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
                 .on_hover_text("Delete");
             if erase.clicked() {
                 actions.push(UiAction::EraseSelection);
             }
+            ui.separator();
+            let buttons = [
+                (view.has_selection, "Make group", "G", UiAction::MakeGroup),
+                (
+                    view.has_selection,
+                    "Make component",
+                    "C",
+                    UiAction::MakeComponent,
+                ),
+                (view.has_instance_selected, "Explode", "", UiAction::Explode),
+                (
+                    view.inside_instance,
+                    "Close group",
+                    "Escape",
+                    UiAction::CloseInstance,
+                ),
+            ];
+            for (enabled, label, key, action) in buttons {
+                let mut button = ui.add_enabled(enabled, egui::Button::new(label));
+                if !key.is_empty() {
+                    button = button.on_hover_text(key);
+                }
+                if button.clicked() {
+                    actions.push(action);
+                }
+            }
         });
     });
     Panel::bottom("status").show(ui, |ui| {
         ui.horizontal(|ui| {
@@ -125,8 +159,10 @@ mod tests {
             tool: ToolKind::Line,
             status: &status,
             info: &info,
             has_selection: false,
+            has_instance_selected: false,
+            inside_instance: false,
         };
         let (actions, rect) = run(&ctx, screen(), &view);
         assert!(actions.is_empty());
         assert!(
@@ -147,14 +183,16 @@ mod tests {
             tool: ToolKind::Line,
             status: &status,
             info: &info,
             has_selection: true,
+            has_instance_selected: true,
+            inside_instance: true,
         };
         let (_, central) = run(&ctx, screen(), &view);
         let row = central.min.y / 2.0;
         // Click along the toolbar row until a press lands on the Rectangle button.
         let mut clicked = Vec::new();
-        for x in (2..600).step_by(4) {
+        for x in (2..1000).step_by(4) {
             let pos = Pos2::new(x as f32, row);
             let pointer = |pressed: bool| egui::Event::PointerButton {
                 pos,
                 button: egui::PointerButton::Primary,
@@ -173,6 +211,14 @@ mod tests {
             "{clicked:?}"
         );
         assert!(clicked.contains(&UiAction::Undo));
         assert!(clicked.contains(&UiAction::EraseSelection));
+        for action in [
+            UiAction::MakeGroup,
+            UiAction::MakeComponent,
+            UiAction::Explode,
+            UiAction::CloseInstance,
+        ] {
+            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
+        }
     }
 }
```

Apply to `crates/app/src/app.rs`:
```diff
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -193,8 +193,20 @@ impl State {
             }
             UiAction::EraseSelection => {
                 self.editor.erase_selection();
             }
+            UiAction::MakeGroup => {
+                self.editor.make_group();
+            }
+            UiAction::MakeComponent => {
+                self.editor.make_component();
+            }
+            UiAction::Explode => {
+                self.editor.explode_selection();
+            }
+            UiAction::CloseInstance => {
+                self.editor.close_instance();
+            }
         }
     }
 
     fn resize(&mut self, width: u32, height: u32) {
@@ -230,8 +242,15 @@ impl State {
             tool: self.editor.tool_kind(),
             status: &status,
             info: &info,
             has_selection: !self.editor.doc().selection().is_empty(),
+            has_instance_selected: self
+                .editor
+                .doc()
+                .selection()
+                .iter()
+                .any(|id| matches!(id, EntityId::Instance(_))),
+            inside_instance: !self.editor.doc().context().is_empty(),
         };
         let gui_frame = self.gui.frame(&self.window, &panels);
         let acted = !gui_frame.actions.is_empty();
         for action in gui_frame.actions.clone() {
@@ -370,8 +389,10 @@ fn physical_char(key: PhysicalKey) -> Option<char> {
         KeyCode::KeyR => 'r',
         KeyCode::KeyP => 'p',
         KeyCode::KeyE => 'e',
         KeyCode::KeyM => 'm',
+        KeyCode::KeyG => 'g',
+        KeyCode::KeyC => 'c',
         _ => return None,
     })
 }
 
```

Apply to `crates/app/src/info.rs`:
```diff
--- a/crates/app/src/info.rs
+++ b/crates/app/src/info.rs
@@ -4,10 +4,30 @@ use kernel::{DefinitionKind, EntityId, Model};
 use session::Document;
 use tools::vcb::format_length;
 
 /// Lines describing the selection: counts, plus length, area or normal for a single entity,
-/// or the kind and name of a single instance.
+/// or the kind and name of a single instance. While a group or component is open, a first line
+/// says which.
 pub fn entity_info(doc: &Document) -> Vec<String> {
+    let mut lines = editing(doc).into_iter().collect::<Vec<_>>();
+    lines.extend(selection_info(doc));
+    lines
+}
+
+/// "Editing group" or "Editing component <name>" while one is open.
+fn editing(doc: &Document) -> Option<String> {
+    let (scene, path) = (doc.scene(), doc.context());
+    let (last, outer) = path.split_last()?;
+    let (parent, _) = scene.resolve(outer)?;
+    let instance = scene.mesh(parent)?.instance(*last)?;
+    let d = scene.definition(instance.definition)?;
+    Some(match d.kind {
+        DefinitionKind::Group => "Editing group".into(),
+        DefinitionKind::Component => format!("Editing component {}", d.name),
+    })
+}
+
+fn selection_info(doc: &Document) -> Vec<String> {
     let m = doc.model();
     let (mut edges, mut faces, mut instances) = (Vec::new(), Vec::new(), Vec::new());
     for id in doc.selection() {
         match id {
@@ -152,5 +172,29 @@ mod tests {
         })
         .unwrap();
         assert_eq!(entity_info(&d), ["1 edge, 1 instance"]);
     }
+
+    #[test]
+    fn says_what_is_being_edited() {
+        let mut d = doc_with_square();
+        let ids = ["e2", "e4", "e6", "e7", "f8"]
+            .map(|s| s.parse().unwrap())
+            .to_vec();
+        d.execute(Command::MakeComponent {
+            ids,
+            name: "Table".into(),
+        })
+        .unwrap();
+        let table = *d.selection().first().unwrap();
+        d.execute(Command::OpenInstance { id: table }).unwrap();
+        assert_eq!(
+            entity_info(&d),
+            ["Editing component Table", "Nothing selected"]
+        );
+        let ids = d.model().faces().map(|(f, _)| EntityId::Face(f)).collect();
+        d.execute(Command::MakeGroup { ids }).unwrap();
+        let group = *d.selection().first().unwrap();
+        d.execute(Command::OpenInstance { id: group }).unwrap();
+        assert_eq!(entity_info(&d), ["Editing group", "Nothing selected"]);
+    }
 }
```

Apply to `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`:
```diff
--- a/docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
+++ b/docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
@@ -1,8 +1,8 @@
 # SketchUp Alternative — Design
 
 Date: 2026-10-10
-Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c)
+Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools)
 
 ## Goal
 
 A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.
@@ -104,8 +104,9 @@ Tools never mutate the mesh directly. A `Commit` carries a `session::Command`, e
 - Locks: Shift locks the current inference; arrow keys lock an axis.
 - Ranking: points over directions, nearer over farther; tolerance in screen pixels.
 - Output: snapped `DVec3`, snap kind, guide geometry, tooltip label.
 - Picking via BVH over faces and edges; edges get a pixel pick radius.
+- Tools work in the open context's coordinates. Inference sees all geometry (inside instances and outside the open instance) and faces anywhere hide what is behind them; clicking geometry inside an instance picks the instance, and geometry outside the open instance is never picked.
 
 ### Measurements box (VCB)
 
 Typed input routed to the active tool. Unit parser supports mm, cm, m, in, ft, `5'6"`, comma-separated dimensions (`300,200`), and array syntax (`5x`, `/5`).
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p app --bin app`
Expected: all pass.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 463 tests pass.

The window is not launched; the egui test clicks the new buttons headlessly and the editor tests drive the shortcuts.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/app/src/editor.rs crates/app/src/ui.rs crates/app/src/app.rs crates/app/src/info.rs docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
git commit -m "Group and component buttons, shortcuts and editing line"
```

## Self-review notes

- **Spec coverage (M5b line):**

  | Requirement | Task |
  |---|---|
  | picking and box-selecting instances | 3 |
  | double-click to open and close | 3 (plus Escape) |
  | Move on instances | 3 |
  | inference on geometry inside instances | 1-2 |
  | editing a moved instance with the tools | 2 |
  | toolbar and shortcuts for make group/component and explode | 4 (plus Close group) |
- **Section 2 contract:** tools only emit commands: open, close, make group/component, explode, select, move and copy are all commands, so every new interaction has an API equivalent from M5a.
- **Type names used across tasks:**
  - `Scenery`, `Part`, `Owner`, `infer_in`, `edge_under_in`, `face_under_in`;
  - `ToolContext::scenery`, `ToolEvent::DoubleClick`, `box_outline`;
  - `Editor::{double_click, make_group, make_component, explode_selection, close_instance}`;
  - `UiAction::{MakeGroup, MakeComponent, Explode, CloseInstance}`.
