# M3 Rectangle, Push/Pull and Measurements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Users and agents can draw rectangles and push/pull faces into solids, with typed measurements (lengths, `width,height`, distances) in the window.

**Architecture:** The kernel gains `add_polygon` (several edges as one ChangeSet), a `MoveVertex` op, and `push_pull`. Push/pull either moves the face, when every touching face runs along the push direction, or extrudes it through the existing sticky `add_edge` machinery and then orients the new faces outward. `session` adds `add_rectangle` and `push_pull` commands through one shared edit path used by `execute` and atomic batches, plus a bounding-box filter on `entities`. `tools` gains a `Tool` trait (`ToolContext`, typed `Value` events, `on_external_change`), face-aware and occlusion-aware inference, a measurements parser, and Rectangle and Push/Pull tools. `app` switches tools with L/R/P, routes typed text to the measurements box, and shows tool, prompt and measurement in the window title until the egui shell lands in M4.

**Tech Stack:** Rust 2024; no new dependencies (wgpu 30.0.1, winit 0.30.13, glam 0.34.1, serde, schemars, earcutr as pinned).

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`: milestone M3, section 1 (push_pull), section 2 (tool contract, measurements box), section 5 (agent control).

## Global Constraints

- Every new capability is a `session::Command` reachable through the API; tools only emit commands and never edit the model.
- Each modeling command is one undo step, and `ChangeSet::inverse` undoes it exactly. Kernel tests call `model.validate()` after every edit.
- Units are meters; `LENGTH_TOL = 1e-6`; command inputs and the points they produce must be finite and within +/-`MAX_COORDINATE` (1e6 m).
- Entity ids on the wire are strings (`v3`, `e7`, `f2`); errors carry the offending ids.
- No new crates or version changes.
- Git: work on `dev`; stage only the files you changed; short plain commit messages, no emoji; `cargo fmt --all` before each commit; `cargo clippy --workspace --all-targets -- -D warnings` stays clean.

## Spec notes (rulings made in this plan)

- **Push/pull rules.**
  - **Move.** If every face touching the pushed face either moves with it entirely or contains the push direction, and every edge leaving it runs along the normal, the face moves. That is the box-top case. A move that would collapse or invert those edges fails with `invalid_input`.
  - **Extrude.** Otherwise the face extrudes, creating sides and a cap.
    - A face in a flat sheet (all faces connected to it are coplanar) stays as the base and is flipped to face away from the solid.
    - A face on a solid is removed: pulling out makes a bump, pushing in cuts a recess.
- **Not in M3:**
  - Pushing through to make an opening.
  - Healing seams where a new side meets an existing coplanar face.
  - Intersecting with geometry the extrusion passes through.
- **Faces in one plane may face opposite ways.** This is needed so a flipped base keeps its orientation when its plane is rebuilt. Face reconciliation therefore matches faces by vertex set, not orientation.
- **Measurements box.**
  - Accepted input: lengths (`2.5`, `250mm`, `25cm`, `2m`, `10in`, `3ft`, `5'6"`) and comma-separated pairs (`2,3`).
  - Array syntax (`5x`, `/5`) belongs to Move/Copy and moves to M4.
  - Until egui arrives in M4, the box and the tool prompt are shown in the window title. Task 8 amends the spec.
- **Rectangle tool plane.** It lies on the face under the first click, or else on the horizontal plane through it. Agents can use any plane through `add_rectangle`'s `normal`.
- **Tool switching** (`L`, `R`, `P`) is window state, not a command. Agents use the commands directly.
- **Inference.**
  - `Inference.face` reports the face under the cursor.
  - Endpoints and edges hidden behind a face are no longer snapped.
  - Axis guides still show through faces.

## File Structure

```
crates/kernel/src/edit.rs          add_polygon, draw_segment, check_segment; new KernelError variants
crates/kernel/src/faces.rs         reconcile matches faces regardless of orientation
crates/kernel/src/model.rs         Op::MoveVertex, move_vertex, summary handles moves
crates/kernel/src/push_pull.rs     new: Model::push_pull (move or extrude)
crates/kernel/src/lib.rs           mod push_pull
crates/kernel/tests/edges.rs       add_polygon tests
crates/kernel/tests/push_pull.rs   new
crates/session/src/rectangle.rs    new: rectangle_axes, rectangle_corners
crates/session/src/command.rs      AddRectangle, PushPull, Command::changes_model
crates/session/src/query.rs        Bounds, entities `within`
crates/session/src/document.rs     shared edit path, rectangle/push-pull changes, bounds filter
crates/session/src/lib.rs          exports
crates/session/tests/commands.rs   Entities literal gains `within`
crates/session/tests/modeling.rs   new
crates/tools/src/inference.rs      Inference.face, face_under, occlusion
crates/tools/src/vcb.rs            new: parse_length(s), format_length, toward_cursor
crates/tools/src/tool.rs           new: Tool trait, ToolContext, ToolEvent, ToolOutput, ToolKind
crates/tools/src/line.rs           Line tool on the trait, typed lengths
crates/tools/src/rectangle.rs      new: Rectangle tool
crates/tools/src/push_pull.rs      new: Push/Pull tool
crates/tools/src/overlay.rs        test literals gain `face: None`
crates/tools/src/lib.rs            modules and exports
crates/app/src/editor.rs           Box<dyn Tool>, tool switching, typed values, status, cursor_left
crates/app/src/app.rs              keys to the editor, status in the title, CursorLeft
crates/api/tests/dispatch.rs       rectangle/push_pull over JSON-RPC
docs/agent-api.md                  new commands, within filter, box example
docs/superpowers/specs/...md       M3/M4 amendment
```

Workspace test totals after each task: 189, 201, 214, 223, 230, 238, 249, 250 (186 before M3).

---
### Task 1: Kernel polygons and faces of either orientation

**Files:**
- Modify: `crates/kernel/src/edit.rs`, `crates/kernel/src/faces.rs`
- Test: `crates/kernel/tests/edges.rs`, `crates/kernel/src/faces.rs` (unit test)

**Interfaces:**
- Produces:
  - `Model::add_polygon(&mut self, points: &[DVec3]) -> Result<ChangeSet, KernelError>`: the closed polygon through `points` as one ChangeSet, with `add_edge` rules (auto faces included). It checks every side first (`NotFinite`, `ZeroLength`) and changes nothing on error.
  - `pub(crate) fn draw_segment(&mut self, a: DVec3, b: DVec3)`: `add_edge` without checks and without ending the edit, for kernel operations that build one ChangeSet out of many segments.
  - `pub(crate) fn find_vertex(&self, p: DVec3) -> Option<VertexId>` (was private).
  - Face reconciliation keeps a face that matches a region by vertex set even when its normal is opposite to the plane's orientation. Its holes are wound to match its own normal.

- [ ] **Step 1: Write the failing tests**

Append to `crates/kernel/tests/edges.rs`:
```rust
#[test]
fn add_polygon_is_one_change_set_with_a_face() {
    let mut m = Model::new();
    let pts = [
        DVec3::ZERO,
        DVec3::new(2.0, 0.0, 0.0),
        DVec3::new(2.0, 1.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
    ];
    let c = m.add_polygon(&pts).unwrap();
    m.validate().unwrap();
    assert_eq!(c.summary().created.len(), 4 + 4 + 1);
    assert_eq!(m.faces().count(), 1);
    m.apply(&c.inverse());
    assert_eq!(m, Model::new());
}

#[test]
fn add_polygon_rejects_bad_points_without_changes() {
    let mut m = Model::new();
    let (a, b) = (DVec3::ZERO, DVec3::X);
    assert_eq!(
        m.add_polygon(&[a, b, b, DVec3::Y]),
        Err(KernelError::ZeroLength)
    );
    assert_eq!(
        m.add_polygon(&[a, b, DVec3::NAN]),
        Err(KernelError::NotFinite)
    );
    assert_eq!(m, Model::new());
}
```

In `crates/kernel/src/faces.rs`, add to the `tests` module, before `loop_drawn_inside_an_opening_is_filled`:
```rust
    #[test]
    fn flipped_face_keeps_its_id_and_orientation_when_its_plane_is_rebuilt() {
        let mut m = Model::new();
        square(&mut m, 0.0, 0.0, 4.0);
        let (id, f) = m.faces().map(|(id, f)| (id, f.clone())).next().unwrap();
        let mut outer = f.outer.clone();
        outer.reverse();
        let flipped = Face {
            outer,
            holes: vec![],
            normal: -f.normal,
        };
        m.replace_face(id, flipped);
        m.take_changes();
        square(&mut m, 1.0, 1.0, 2.0);
        let f = m.face(id).unwrap();
        assert_eq!(f.normal, -DVec3::Z);
        assert_eq!(f.holes.len(), 1);
        assert_eq!(m.face_area(id), 12.0);
        assert_eq!(m.faces().count(), 2);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kernel`
Expected: compile error in `tests/edges.rs`: no method named `add_polygon`. After a temporary stub, the faces test would fail because the flipped face is replaced by a new id. You do not need to write the stub.

- [ ] **Step 3: Implement**

In `crates/kernel/src/edit.rs`, replace `add_edge` with these three methods (same `impl Model` block):
```rust
    /// Adds the segment a-b with sticky-geometry rules: endpoints merge with nearby vertices,
    /// crossed edges are split, collinear overlaps merge, and closed coplanar loops become faces.
    /// Returns the changes as one ChangeSet. It is empty only when nothing changed; it can be
    /// non-empty without a new edge when an existing edge was only split.
    pub fn add_edge(&mut self, a: DVec3, b: DVec3) -> Result<ChangeSet, KernelError> {
        check_segment(a, b)?;
        self.draw_segment(a, b);
        Ok(self.take_changes())
    }

    /// Adds the closed polygon through `points` (each point joined to the next, the last to the
    /// first) as one ChangeSet, with the same rules as `add_edge`.
    pub fn add_polygon(&mut self, points: &[DVec3]) -> Result<ChangeSet, KernelError> {
        let n = points.len();
        for i in 0..n {
            check_segment(points[i], points[(i + 1) % n])?;
        }
        for i in 0..n {
            self.draw_segment(points[i], points[(i + 1) % n]);
        }
        Ok(self.take_changes())
    }

    /// `add_edge` without input checks or ending the edit; the caller checked the segment.
    pub(crate) fn draw_segment(&mut self, a: DVec3, b: DVec3) {
        let new_edges = self.insert_segment(a, b);
        self.create_faces(&new_edges);
    }
```

Make `find_vertex` crate-visible: `fn find_vertex(&self, p: DVec3)` becomes `pub(crate) fn find_vertex(&self, p: DVec3)`.

Add this free function above `fn loop_has_side`:
```rust
fn check_segment(a: DVec3, b: DVec3) -> Result<(), KernelError> {
    if !a.is_finite() || !b.is_finite() {
        return Err(KernelError::NotFinite);
    }
    if a.distance(b) < LENGTH_TOL {
        return Err(KernelError::ZeroLength);
    }
    Ok(())
}
```

In `crates/kernel/src/faces.rs`, replace `fn reconcile` with:
```rust
    /// Keeps old faces that match a wanted region, updates their holes if needed, inserts the
    /// rest, and removes old faces that match nothing. All through recorded primitives.
    fn reconcile(&mut self, old: &[(FaceId, Face)], wanted: Vec<Region>, plane: Plane) {
        let mut matched: BTreeSet<FaceId> = BTreeSet::new();
        for r in wanted {
            let outer: BTreeSet<VertexId> = r.outer.iter().copied().collect();
            // Faces in one plane may face opposite ways (push/pull flips a base face), so an
            // old face matches by vertex set and keeps its own orientation.
            let same = old.iter().find(|(id, f)| {
                !matched.contains(id) && f.outer.iter().copied().collect::<BTreeSet<_>>() == outer
            });
            match same {
                Some((id, f)) => {
                    matched.insert(*id);
                    if loop_sets(&f.holes) != loop_sets(&r.holes) {
                        let mut holes = r.holes;
                        if f.normal.dot(plane.normal) < 0.0 {
                            holes.iter_mut().for_each(|h| h.reverse());
                        }
                        let face = Face {
                            outer: f.outer.clone(),
                            holes,
                            normal: f.normal,
                        };
                        self.replace_face(*id, face);
                    }
                }
                None => {
                    self.insert_face(Face {
                        outer: r.outer,
                        holes: r.holes,
                        normal: plane.normal,
                    });
                }
            }
        }
        for (id, _) in old {
            if !matched.contains(id) {
                self.remove_face(*id);
            }
        }
    }
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 189.

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/edit.rs crates/kernel/src/faces.rs crates/kernel/tests/edges.rs
git commit -m "Add polygons as one edit and keep flipped faces on plane rebuilds"
```

---

### Task 2: Kernel push/pull

**Files:**
- Modify: `crates/kernel/src/model.rs`, `crates/kernel/src/edit.rs`, `crates/kernel/src/lib.rs`
- Create: `crates/kernel/src/push_pull.rs`, `crates/kernel/tests/push_pull.rs`

**Interfaces:**
- Consumes: `draw_segment`, `find_vertex` (Task 1); `remove_face`, `replace_face`, `take_changes` (existing).
- Produces:
  - `Op::MoveVertex(VertexId, DVec3 /* from */, DVec3 /* to */)`. Its inverse swaps the positions. `ChangeSummary` lists a moved vertex as `modified`.
  - `pub(crate) fn move_vertex(&mut self, id: VertexId, to: DVec3)` (recorded).
  - `KernelError::{FaceNotFound, ZeroDistance, Collapse}`.
  - `Model::push_pull(&mut self, id: FaceId, distance: f64) -> Result<ChangeSet, KernelError>`, with the rules in "Spec notes". It checks before mutating, so an error changes nothing.

- [ ] **Step 1: Write the failing tests**

Create `crates/kernel/tests/push_pull.rs`:
```rust
use glam::DVec3;
use kernel::{EntityId, FaceId, KernelError, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// Adds an axis-aligned rectangle at height z and returns its face.
fn rect(m: &mut Model, x0: f64, y0: f64, x1: f64, y1: f64, z: f64) -> FaceId {
    let c = m
        .add_polygon(&[p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z)])
        .unwrap();
    m.validate().unwrap();
    let s = c.summary();
    let faces: Vec<FaceId> = s
        .created
        .iter()
        .filter_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .collect();
    *faces.last().expect("rectangle made a face")
}

/// The face whose plane is z = `z` and whose normal is parallel to Z.
fn face_at_height(m: &Model, z: f64) -> FaceId {
    m.faces()
        .find(|(_, f)| {
            f.normal.cross(DVec3::Z).length() < 1e-9
                && f.outer
                    .iter()
                    .all(|v| (m.position(*v).unwrap().z - z).abs() < 1e-9)
        })
        .map(|(id, _)| id)
        .expect("face at height")
}

fn counts(m: &Model) -> (usize, usize, usize) {
    (m.vertices().count(), m.edges().count(), m.faces().count())
}

/// Number of `solid` faces crossed by the ray from `from` in a fixed, generic direction.
fn crossings(m: &Model, solid: &[FaceId], from: DVec3) -> usize {
    let dir = DVec3::new(0.2113, 0.5773, 0.7887).normalize();
    let mut n = 0;
    for id in solid {
        for [a, b, c] in m.triangulate(*id).unwrap() {
            let (e1, e2) = (b - a, c - a);
            let h = dir.cross(e2);
            let det = e1.dot(h);
            if det.abs() < 1e-12 {
                continue;
            }
            let s = from - a;
            let u = s.dot(h) / det;
            let q = s.cross(e1);
            let v = dir.dot(q) / det;
            let t = e2.dot(q) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 1e-9 {
                n += 1;
            }
        }
    }
    n
}

/// Every face of the closed surface `solid` points out of it: just in front of the face is
/// outside (even crossings), just behind is inside (odd).
fn assert_outward(m: &Model, solid: &[FaceId]) {
    for id in solid {
        let f = m.face(*id).unwrap();
        let [a, b, c] = m.triangulate(*id).unwrap()[0];
        let centre = (a + b + c) / 3.0;
        let front = crossings(m, solid, centre + f.normal * 1e-4);
        let back = crossings(m, solid, centre - f.normal * 1e-4);
        assert!(
            front.is_multiple_of(2) && !back.is_multiple_of(2),
            "face {id:?} points inward"
        );
    }
}

fn all_faces(m: &Model) -> Vec<FaceId> {
    m.faces().map(|(id, _)| id).collect()
}

/// A 4x4x2 box pulled up from the ground. Returns the model and its top face.
fn box_4x4x2() -> (Model, FaceId) {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 4.0, 4.0, 0.0);
    m.push_pull(base, 2.0).unwrap();
    let top = face_at_height(&m, 2.0);
    (m, top)
}

#[test]
fn pulling_a_free_rectangle_makes_a_closed_box() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 3.0, 0.0);
    let c = m.push_pull(base, 1.5).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 12, 6));
    assert_eq!(m.face(base).unwrap().normal, -DVec3::Z, "base faces down");
    assert_eq!(m.face(face_at_height(&m, 1.5)).unwrap().normal, DVec3::Z);
    assert_outward(&m, &all_faces(&m));
    let s = c.summary();
    assert_eq!(s.modified, vec![EntityId::Face(base)]);
    assert!(s.deleted.is_empty());
}

#[test]
fn pushing_a_free_rectangle_down_makes_a_box_below() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 3.0, 0.0);
    m.push_pull(base, -1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 12, 6));
    assert_eq!(m.face(base).unwrap().normal, DVec3::Z, "base faces up");
    assert_eq!(m.face(face_at_height(&m, -1.0)).unwrap().normal, -DVec3::Z);
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn pulling_a_box_top_stretches_the_sides() {
    let (mut m, top) = box_4x4x2();
    let c = m.push_pull(top, 1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 12, 6));
    assert!(m.face(top).is_some(), "the top keeps its id");
    assert_eq!(face_at_height(&m, 3.0), top);
    let s = c.summary();
    assert!(s.created.is_empty() && s.deleted.is_empty());
    assert_eq!(s.modified.len(), 4);
    assert!(
        s.modified
            .iter()
            .all(|id| matches!(id, EntityId::Vertex(_)))
    );
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn pushing_a_box_top_to_or_through_the_bottom_fails() {
    let (mut m, top) = box_4x4x2();
    let before = m.clone();
    assert_eq!(m.push_pull(top, -2.0), Err(KernelError::Collapse));
    assert_eq!(m.push_pull(top, -3.0), Err(KernelError::Collapse));
    assert_eq!(m, before);
    assert!(m.push_pull(top, -1.0).is_ok());
}

#[test]
fn rectangle_on_a_box_top_pulls_up_as_a_bump() {
    let (mut m, top) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, 1.0).unwrap();
    m.validate().unwrap();
    assert!(m.face(sub).is_none(), "the bump's base is not a face");
    assert_eq!(m.faces().count(), 11);
    assert_eq!(m.face(top).unwrap().holes.len(), 1);
    assert_eq!(m.face(face_at_height(&m, 3.0)).unwrap().normal, DVec3::Z);
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn rectangle_on_a_box_top_pushed_down_cuts_a_recess() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, -1.0).unwrap();
    m.validate().unwrap();
    assert!(m.face(sub).is_none());
    assert_eq!(m.faces().count(), 11);
    assert_eq!(m.face(face_at_height(&m, 1.0)).unwrap().normal, DVec3::Z);
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn face_with_a_hole_extrudes_a_tube() {
    let mut m = Model::new();
    let ring = rect(&mut m, 0.0, 0.0, 4.0, 4.0, 0.0);
    let inner = rect(&mut m, 1.0, 1.0, 3.0, 3.0, 0.0);
    assert_eq!(m.face(ring).unwrap().holes.len(), 1);
    m.push_pull(ring, 2.0).unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 11);
    let cap = face_at_height(&m, 2.0);
    assert_eq!(m.face(cap).unwrap().holes.len(), 1, "the hole stays open");
    assert!((m.area(cap).unwrap() - 12.0).abs() < 1e-9);
    let tube: Vec<FaceId> = all_faces(&m).into_iter().filter(|f| *f != inner).collect();
    assert_outward(&m, &tube);
}

#[test]
fn pulling_one_of_two_adjacent_rectangles_keeps_the_neighbor() {
    let mut m = Model::new();
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    let b = rect(&mut m, 2.0, 0.0, 4.0, 2.0, 0.0);
    let neighbor = m.face(b).unwrap().clone();
    m.push_pull(a, 1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(m.face(b), Some(&neighbor));
    assert_eq!(m.face(a).unwrap().normal, -DVec3::Z);
    assert_eq!(m.faces().count(), 7);
}

#[test]
fn walls_at_any_angle_extrude_into_boxes() {
    for (u, w) in [
        (DVec3::X, DVec3::Z),
        (DVec3::new(1.0, 1.0, 0.0).normalize(), DVec3::Z),
        (DVec3::X, DVec3::new(0.0, 1.0, 1.0).normalize()),
    ] {
        let mut m = Model::new();
        let o = p(0.3, 0.7, 0.0);
        let c = m
            .add_polygon(&[o, o + u * 2.0, o + u * 2.0 + w * 1.5, o + w * 1.5])
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
        m.push_pull(face, 0.8).unwrap();
        m.validate().unwrap();
        assert_eq!(counts(&m), (8, 12, 6), "wall along {u} and {w}");
        assert_outward(&m, &all_faces(&m));
    }
}

#[test]
fn push_pull_undoes_and_redoes_exactly() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    let flat = m.clone();
    let extrude = m.push_pull(base, 1.0).unwrap();
    let boxed = m.clone();
    let top = face_at_height(&m, 1.0);
    let stretch = m.push_pull(top, 0.5).unwrap();
    let taller = m.clone();
    m.apply(&stretch.inverse());
    assert_eq!(m, boxed);
    m.apply(&extrude.inverse());
    assert_eq!(m, flat);
    m.apply(&extrude);
    m.apply(&stretch);
    assert_eq!(m, taller);
}

#[test]
fn bad_push_pull_input_changes_nothing() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    let before = m.clone();
    let missing = FaceId(999);
    assert_eq!(m.push_pull(missing, 1.0), Err(KernelError::FaceNotFound));
    assert_eq!(m.push_pull(base, 0.0), Err(KernelError::ZeroDistance));
    assert_eq!(m.push_pull(base, f64::NAN), Err(KernelError::NotFinite));
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
}
```

`assert_outward` casts a ray from just in front of and just behind each face and counts crossings; it applies only to closed surfaces, so the tube test leaves out the inner ground face.

In `crates/kernel/src/model.rs`, add to the `tests` module, before `ids_are_never_reused_after_undo`:
```rust
    #[test]
    fn moved_vertices_undo_exactly_and_count_as_modified() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        m.take_changes();
        let before = m.clone();
        m.move_vertex(a, DVec3::X);
        m.move_vertex(a, DVec3::Y);
        let c = m.take_changes();
        assert_eq!(m.position(a), Some(DVec3::Y));
        assert_eq!(c.summary().modified, vec![EntityId::Vertex(a)]);
        assert!(c.summary().created.is_empty() && c.summary().deleted.is_empty());
        m.apply(&c.inverse());
        assert_eq!(m, before);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kernel`
Expected: compile errors: no method `push_pull`, no variant `KernelError::Collapse`, no method `move_vertex`.

- [ ] **Step 3: Implement**

`crates/kernel/src/model.rs`:

Replace `enum Op` and its `impl Op` with:
```rust
/// One primitive, invertible change to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    AddVertex(VertexId, Vertex),
    RemoveVertex(VertexId, Vertex),
    AddEdge(EdgeId, Edge),
    RemoveEdge(EdgeId, Edge),
    AddFace(FaceId, Face),
    RemoveFace(FaceId, Face),
    /// A vertex moved from the first position to the second.
    MoveVertex(VertexId, DVec3, DVec3),
}

impl Op {
    fn inverse(&self) -> Op {
        match self.clone() {
            Op::AddVertex(id, v) => Op::RemoveVertex(id, v),
            Op::RemoveVertex(id, v) => Op::AddVertex(id, v),
            Op::AddEdge(id, e) => Op::RemoveEdge(id, e),
            Op::RemoveEdge(id, e) => Op::AddEdge(id, e),
            Op::AddFace(id, f) => Op::RemoveFace(id, f),
            Op::RemoveFace(id, f) => Op::AddFace(id, f),
            Op::MoveVertex(id, from, to) => Op::MoveVertex(id, to, from),
        }
    }

    /// The entity this op touches and whether it exists before and after the op.
    fn entity(&self) -> (EntityId, bool, bool) {
        match self {
            Op::AddVertex(id, _) => (EntityId::Vertex(*id), false, true),
            Op::RemoveVertex(id, _) => (EntityId::Vertex(*id), true, false),
            Op::MoveVertex(id, _, _) => (EntityId::Vertex(*id), true, true),
            Op::AddEdge(id, _) => (EntityId::Edge(*id), false, true),
            Op::RemoveEdge(id, _) => (EntityId::Edge(*id), true, false),
            Op::AddFace(id, _) => (EntityId::Face(*id), false, true),
            Op::RemoveFace(id, _) => (EntityId::Face(*id), true, false),
        }
    }
}
```

Change the doc comment on `ChangeSummary` to:
```rust
/// Net effect of a ChangeSet per entity. A moved vertex is `modified`; the edges and faces using
/// it change shape without being listed.
```

Replace `ChangeSet::summary` with:
```rust
    pub fn summary(&self) -> ChangeSummary {
        // entity -> (existed before, exists after)
        let mut state: BTreeMap<EntityId, (bool, bool)> = BTreeMap::new();
        for op in &self.ops {
            let (id, before, after) = op.entity();
            state.entry(id).or_insert((before, after)).1 = after;
        }
        let mut s = ChangeSummary::default();
        for (id, (before, after)) in state {
            match (before, after) {
                (false, true) => s.created.push(id),
                (true, false) => s.deleted.push(id),
                (true, true) => s.modified.push(id),
                (false, false) => {}
            }
        }
        s
    }
```

In `fn apply_op`, add this arm after `Op::RemoveFace`:
```rust
            Op::MoveVertex(id, _, to) => {
                if let Some(v) = self.vertices.get_mut(&id) {
                    v.pos = to;
                }
            }
```

Add before `pub(crate) fn insert_edge`:
```rust
    pub(crate) fn move_vertex(&mut self, id: VertexId, to: DVec3) {
        let from = self.pos(id);
        self.record(Op::MoveVertex(id, from, to));
    }
```

`crates/kernel/src/edit.rs`, replace `KernelError` and its `Display` impl with:
```rust
#[derive(Clone, Debug, PartialEq)]
pub enum KernelError {
    NotFinite,
    ZeroLength,
    FaceNotFound,
    ZeroDistance,
    /// A push/pull would shrink a connected edge to nothing or turn it around.
    Collapse,
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelError::NotFinite => write!(f, "coordinates must be finite numbers"),
            KernelError::ZeroLength => write!(f, "edge endpoints coincide"),
            KernelError::FaceNotFound => write!(f, "no such face"),
            KernelError::ZeroDistance => write!(f, "distance is too small"),
            KernelError::Collapse => {
                write!(
                    f,
                    "distance would collapse or invert the edges joining the face"
                )
            }
        }
    }
}
```

`crates/kernel/src/lib.rs`: add `mod push_pull;` after `mod model;`.

Create `crates/kernel/src/push_pull.rs`:
```rust
//! Push/pull: move a face along its normal, extruding new sides or stretching the faces around it.

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec3;

use crate::edit::KernelError;
use crate::geom::{LENGTH_TOL, PLANE_ANGLE_TOL};
use crate::ids::{FaceId, VertexId};
use crate::model::{ChangeSet, Face, Model};

impl Model {
    /// Pushes or pulls face `id` by `distance` meters along its normal (negative: against it).
    ///
    /// If every face touching it runs along the push direction (the top of a box), the face
    /// moves and those faces stretch. Otherwise it is extruded: a side face for every side of
    /// every loop, plus a cap. The original face stays as the base when it lies in a flat sheet
    /// and is removed when it is part of a solid's surface. Extruded faces point out of the solid.
    pub fn push_pull(&mut self, id: FaceId, distance: f64) -> Result<ChangeSet, KernelError> {
        let face = self.face(id).ok_or(KernelError::FaceNotFound)?.clone();
        if !distance.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if distance.abs() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        if self.stretches_around(id, &face) {
            self.move_face(&face, face.normal * distance)?;
        } else {
            self.extrude(id, &face, distance);
        }
        Ok(self.take_changes())
    }

    /// True if moving the face along its normal keeps every touching face planar: each face
    /// sharing a vertex either moves with it entirely or contains the normal direction, every
    /// edge leaving the face runs along the normal, and at least one face stretches.
    fn stretches_around(&self, id: FaceId, face: &Face) -> bool {
        let moving = loop_vertices(face);
        let n = face.normal;
        let mut stretching = false;
        for (fid, f) in self.faces() {
            if fid == id {
                continue;
            }
            let verts = loop_vertices(f);
            if verts.is_disjoint(&moving) || verts.is_subset(&moving) {
                continue;
            }
            if f.normal.dot(n).abs() > PLANE_ANGLE_TOL {
                return false;
            }
            stretching = true;
        }
        let along_normal = self.edges().all(|(_, e)| {
            let [a, b] = e.v;
            if moving.contains(&a) == moving.contains(&b) {
                return true;
            }
            (self.pos(b) - self.pos(a)).normalize().cross(n).length() < PLANE_ANGLE_TOL
        });
        stretching && along_normal
    }

    /// Moves every vertex of the face by `offset`, after checking that no edge leaving the face
    /// collapses or flips and no vertex lands on another.
    fn move_face(&mut self, face: &Face, offset: DVec3) -> Result<(), KernelError> {
        let moving = loop_vertices(face);
        for (_, e) in self.edges() {
            let [a, b] = e.v;
            let (m, fixed) = match (moving.contains(&a), moving.contains(&b)) {
                (true, false) => (a, b),
                (false, true) => (b, a),
                _ => continue,
            };
            let before = self.pos(m) - self.pos(fixed);
            let after = before + offset;
            if after.length() < LENGTH_TOL || after.dot(before) <= 0.0 {
                return Err(KernelError::Collapse);
            }
        }
        for &m in &moving {
            let to = self.pos(m) + offset;
            let lands_on_other = self
                .vertices()
                .any(|(v, vert)| !moving.contains(&v) && vert.pos.distance(to) < LENGTH_TOL);
            if lands_on_other {
                return Err(KernelError::Collapse);
            }
        }
        for m in moving {
            let to = self.pos(m) + offset;
            self.move_vertex(m, to);
        }
        Ok(())
    }

    fn extrude(&mut self, id: FaceId, face: &Face, distance: f64) {
        let n = face.normal;
        let offset = n * distance;
        let in_solid = self.in_solid(id);
        let loops: Vec<&Vec<VertexId>> = std::iter::once(&face.outer).chain(&face.holes).collect();
        let points: Vec<Vec<DVec3>> = loops
            .iter()
            .map(|l| l.iter().map(|v| self.pos(*v)).collect())
            .collect();
        let before: BTreeSet<FaceId> = self.faces().map(|(f, _)| f).collect();

        if in_solid {
            self.remove_face(id);
        }
        for l in &points {
            for p in l {
                self.draw_segment(*p, *p + offset);
            }
        }
        for l in &points {
            for i in 0..l.len() {
                self.draw_segment(l[i] + offset, l[(i + 1) % l.len()] + offset);
            }
        }

        // Each hole's cap loop closes inside the cap, which fills it like any loop drawn on a
        // face; that fill is removed so the hole stays open.
        let hole_caps: Vec<BTreeSet<VertexId>> = points[1..]
            .iter()
            .filter_map(|l| l.iter().map(|p| self.find_vertex(*p + offset)).collect())
            .collect();
        // A solid grows outward when pulled along the normal and is cut when pushed against it.
        let cut = in_solid && distance < 0.0;
        let cap_normal = if in_solid || distance > 0.0 { n } else { -n };
        let cap_origin = points[0][0] + offset;
        let created: Vec<FaceId> = self
            .faces()
            .map(|(f, _)| f)
            .filter(|f| !before.contains(f))
            .collect();
        for f in created {
            let g = self.face(f).unwrap().clone();
            let outer: BTreeSet<VertexId> = g.outer.iter().copied().collect();
            if hole_caps.contains(&outer) {
                self.remove_face(f);
                continue;
            }
            let in_cap = g.normal.cross(n).length() < PLANE_ANGLE_TOL
                && (self.pos(g.outer[0]) - cap_origin).dot(n).abs() < LENGTH_TOL;
            let want = if in_cap {
                Some(cap_normal)
            } else {
                loops.iter().find_map(|l| {
                    (0..l.len()).find_map(|i| {
                        let (a, b) = (l[i], l[(i + 1) % l.len()]);
                        (outer.contains(&a) && outer.contains(&b)).then(|| {
                            let out = (self.pos(b) - self.pos(a)).cross(n).normalize();
                            if cut { -out } else { out }
                        })
                    })
                })
            };
            if let Some(want) = want {
                self.orient(f, want);
            }
        }
        if !in_solid {
            self.orient(id, if distance > 0.0 { -n } else { n });
        }
    }

    /// Flips face `id` if its normal points away from `want`.
    fn orient(&mut self, id: FaceId, want: DVec3) {
        let f = self.face(id).unwrap();
        if f.normal.dot(want) >= 0.0 {
            return;
        }
        let reversed = |l: &Vec<VertexId>| l.iter().rev().copied().collect::<Vec<_>>();
        let flipped = Face {
            outer: reversed(&f.outer),
            holes: f.holes.iter().map(reversed).collect(),
            normal: -f.normal,
        };
        self.replace_face(id, flipped);
    }

    /// True if the face is connected through shared sides to a face in another plane, i.e. it
    /// is part of a solid's surface rather than a flat sheet.
    fn in_solid(&self, id: FaceId) -> bool {
        let mut by_side: BTreeMap<(VertexId, VertexId), Vec<FaceId>> = BTreeMap::new();
        for (fid, f) in self.faces() {
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    by_side
                        .entry(side(l[i], l[(i + 1) % l.len()]))
                        .or_default()
                        .push(fid);
                }
            }
        }
        let n = self.face(id).unwrap().normal;
        let mut seen = BTreeSet::from([id]);
        let mut stack = vec![id];
        while let Some(fid) = stack.pop() {
            let f = self.face(fid).unwrap();
            if f.normal.cross(n).length() >= PLANE_ANGLE_TOL {
                return true;
            }
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    for &next in &by_side[&side(l[i], l[(i + 1) % l.len()])] {
                        if seen.insert(next) {
                            stack.push(next);
                        }
                    }
                }
            }
        }
        false
    }
}

fn loop_vertices(f: &Face) -> BTreeSet<VertexId> {
    std::iter::once(&f.outer)
        .chain(&f.holes)
        .flatten()
        .copied()
        .collect()
}

fn side(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    (a.min(b), a.max(b))
}
```

The extrusion draws every side's vertical edge, then every cap edge, through `draw_segment`. Existing auto-face code therefore creates the side and cap faces and merges with nearby geometry. Afterwards the code:
- removes the fill that auto-faces put inside each hole's cap loop;
- flips each new face to point out of the solid: the cap along the push direction (for a sheet) or the normal (for a solid); each side along `side x normal`, reversed for a recess;
- flips a kept base face to point away from the solid.

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 201 (kernel push_pull 11, model unit tests +1).

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/model.rs crates/kernel/src/edit.rs crates/kernel/src/lib.rs crates/kernel/src/push_pull.rs crates/kernel/tests/push_pull.rs
git commit -m "Add push/pull: extrude faces outward or move them when their neighbors stretch"
```

---
### Task 3: Session commands: add_rectangle, push_pull, bounds filter

**Files:**
- Create: `crates/session/src/rectangle.rs`, `crates/session/tests/modeling.rs`
- Modify: `crates/session/src/command.rs`, `crates/session/src/query.rs`, `crates/session/src/document.rs`, `crates/session/src/lib.rs`, `crates/session/tests/commands.rs`

**Interfaces:**
- Consumes: `Model::add_polygon`, `Model::push_pull`, `KernelError` (Tasks 1-2).
- Produces:
  - `Command::AddRectangle { corner: [f64; 3], opposite: [f64; 3], normal: Option<[f64; 3]> }` (wire: `add_rectangle`, `normal` optional).
  - `Command::PushPull { face: EntityId, distance: f64 }` (wire: `push_pull`).
  - `Command::changes_model(&self) -> bool`: false only for `SetCamera`.
  - `session::rectangle_axes(normal: DVec3) -> (DVec3, DVec3)`.
  - `session::rectangle_corners(corner: DVec3, opposite: DVec3, normal: Option<DVec3>) -> Result<[DVec3; 4], CommandError>`, shared by the command and the Rectangle tool (Task 6).
  - `Query::Entities` gains `within: Option<Bounds>`; `session::Bounds { min: [f64; 3], max: [f64; 3] }` with `contains(p)`. An entity matches when all its vertices are inside, bounds included.
  - `Document::edit(&Command) -> Option<Result<ChangeSet, CommandError>>` (private) is the single place editing commands become ChangeSets. `execute` and atomic batches both use it, so a new command added there works in both.
  - Errors:
    - `push_pull` given a non-face id: `invalid_input` with `ids`.
    - Unknown face: `not_found` with `ids`.
    - Zero distance, collapse, or results beyond `MAX_COORDINATE`: `invalid_input`.
    - Bad rectangles: `invalid_input`.

- [ ] **Step 1: Write the failing tests**

Create `crates/session/tests/modeling.rs`:
```rust
//! Rectangles, push/pull and the entities filter, driven through JSON commands.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn cmd(v: Value) -> Command {
    serde_json::from_value(v).expect("valid command json")
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, Vec<String>)> {
    d.execute(cmd(v))
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.ids.iter().map(|i| i.to_string()).collect()))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn counts(d: &Document) -> (u64, u64, u64) {
    let s = ask(d, json!({"type": "stats"}));
    let n = |k: &str| s[k].as_u64().unwrap();
    (n("vertices"), n("edges"), n("faces"))
}

/// The id of the one face an outcome created.
fn created_face(outcome: &Value) -> String {
    let faces: Vec<&str> = outcome["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .filter(|id| id.starts_with('f'))
        .collect();
    assert_eq!(faces.len(), 1, "{outcome}");
    faces[0].to_string()
}

fn rectangle(d: &mut Document, corner: [f64; 3], opposite: [f64; 3]) -> String {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": corner, "opposite": opposite}),
    )
    .unwrap();
    created_face(&out)
}

#[test]
fn rectangle_on_the_ground_is_one_undo_step() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 3.0, 0.0]);
    assert_eq!(counts(&d), (4, 4, 1));
    let info = ask(&d, json!({"type": "entity", "id": face}));
    assert_eq!(info["area"], json!(6.0));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (0, 0, 0));
}

#[test]
fn rectangle_on_a_tilted_plane_follows_its_normal() {
    let mut d = doc();
    let a = 0.5f64.sqrt();
    // In the plane with normal (0, a, a): u = -X, v = (0, -a, a).
    let out = run(
        &mut d,
        json!({
            "type": "add_rectangle",
            "corner": [1.0, 0.0, 0.0],
            "opposite": [-1.0, -1.5 * a, 1.5 * a],
            "normal": [0.0, 1.0, 1.0]
        }),
    )
    .unwrap();
    let info = ask(&d, json!({"type": "entity", "id": created_face(&out)}));
    assert!((info["area"].as_f64().unwrap() - 3.0).abs() < 1e-9);
    let n: Vec<f64> = serde_json::from_value(info["normal"].clone()).unwrap();
    assert!((n[0]).abs() < 1e-9 && (n[1] - a).abs() < 1e-9 && (n[2] - a).abs() < 1e-9);
}

#[test]
fn bad_rectangles_change_nothing() {
    let mut d = doc();
    for bad in [
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 1]}),
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 0, 0]}),
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 1], "normal": [0, 0, 1]}),
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2e6, 1, 0]}),
    ] {
        assert_eq!(
            run(&mut d, bad.clone()).unwrap_err().0,
            ErrorKind::InvalidInput,
            "{bad}"
        );
    }
    assert_eq!(counts(&d), (0, 0, 0));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 0);
}

#[test]
fn push_pull_makes_a_box_in_one_undo_step() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 1.5}),
    )
    .unwrap();
    assert_eq!(counts(&d), (8, 12, 6));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (4, 4, 1));
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(counts(&d), (8, 12, 6));
}

#[test]
fn pulling_a_box_top_reports_moved_vertices() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    let out = run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 1.0}),
    )
    .unwrap();
    let top = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .find(|id| {
            id.starts_with('f')
                && ask(&d, json!({"type": "entity", "id": id}))["normal"] == json!([0.0, 0.0, 1.0])
        })
        .unwrap();
    let out = run(
        &mut d,
        json!({"type": "push_pull", "face": top, "distance": 0.5}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"].as_array().unwrap().len(), 4);
    assert_eq!(counts(&d), (8, 12, 6));
}

#[test]
fn bad_push_pulls_name_the_entity() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    let pp =
        |id: &str, distance: f64| json!({"type": "push_pull", "face": id, "distance": distance});
    assert_eq!(
        run(&mut d, pp("e2", 1.0)).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["e2".to_string()])
    );
    assert_eq!(
        run(&mut d, pp("f999", 1.0)).unwrap_err(),
        (ErrorKind::NotFound, vec!["f999".to_string()])
    );
    assert_eq!(
        run(&mut d, pp(&face, 0.0)).unwrap_err().0,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        run(&mut d, pp(&face, 2e6)).unwrap_err().0,
        ErrorKind::InvalidInput
    );
    assert!(
        serde_json::from_value::<Command>(json!({"type": "push_pull", "face": face, "dist": 1}))
            .is_err()
    );
    assert_eq!(counts(&d), (4, 4, 1));
}

#[test]
fn atomic_batch_of_rectangle_and_push_pull_is_one_undo_step() {
    let mut d = doc();
    // Ids are allocated in order, so on an empty model the rectangle's face is f8.
    let cmds = vec![
        cmd(json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]})),
        cmd(json!({"type": "push_pull", "face": "f8", "distance": 1})),
    ];
    d.execute_batch(cmds, true).unwrap();
    assert_eq!(counts(&d), (8, 12, 6));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (0, 0, 0));
}

#[test]
fn atomic_batches_without_model_changes_add_no_undo_step() {
    let mut d = doc();
    let camera_only = vec![cmd(
        json!({"type": "set_camera", "eye": [5, 5, 5], "target": [0, 0, 0]}),
    )];
    d.execute_batch(camera_only, true).unwrap();
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 0);
    assert_eq!(d.revision(), 0);

    rectangle(&mut d, [0.0, 0.0, 0.0], [1.0, 1.0, 0.0]);
    let existing = vec![cmd(
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    )];
    let out = d.execute_batch(existing, true).unwrap();
    assert!(out[0].changes.created.is_empty());
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
    assert_eq!(d.revision(), 1);
}

#[test]
fn mixed_atomic_batch_rolls_back_the_camera_too() {
    let mut d = doc();
    let camera = d.camera.clone();
    let cmds = vec![
        cmd(json!({"type": "set_camera", "eye": [5, 5, 5], "target": [0, 0, 0]})),
        cmd(json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]})),
        cmd(json!({"type": "push_pull", "face": "f999", "distance": 1})),
    ];
    let err = d.execute_batch(cmds, true).unwrap_err();
    assert_eq!(err.index, 2);
    assert!(err.rolled_back);
    assert_eq!(d.camera, camera);
    assert_eq!(counts(&d), (0, 0, 0));
}

#[test]
fn entities_within_bounds() {
    let mut d = doc();
    rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    rectangle(&mut d, [5.0, 5.0, 0.0], [6.0, 6.0, 0.0]);
    let r = ask(
        &d,
        json!({"type": "entities", "within": {"min": [-1, -1, -1], "max": [3, 3, 1]}}),
    );
    assert_eq!(r["vertices"].as_array().unwrap().len(), 4);
    assert_eq!(r["edges"].as_array().unwrap().len(), 4);
    assert_eq!(r["faces"].as_array().unwrap().len(), 1);
    let edge_on_boundary = ask(
        &d,
        json!({"type": "entities", "kinds": ["edge"], "within": {"min": [0, 0, 0], "max": [2, 0, 0]}}),
    );
    assert_eq!(edge_on_boundary["edges"].as_array().unwrap().len(), 1);
}
```

In `crates/session/tests/commands.rs` test `query_results_round_trip_through_json`, the `Query::Entities` literal gains `within: None,` after `kinds: vec![],`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p session`
Expected: `commands.rs` fails to compile (`Query::Entities` has no field `within`). Without that change, `modeling.rs` fails at runtime on the unknown `add_rectangle` command.

- [ ] **Step 3: Implement**

Create `crates/session/src/rectangle.rs`:
```rust
//! Rectangle geometry shared by `Command::AddRectangle` and the Rectangle tool.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;

use crate::command::{CommandError, ErrorKind};

/// In-plane side directions of rectangles drawn on a plane with unit normal `normal`: `u` is
/// horizontal when the plane is not (X on horizontal planes), and `v = normal x u`.
pub fn rectangle_axes(normal: DVec3) -> (DVec3, DVec3) {
    let u = if normal.cross(DVec3::Z).length() < 1e-9 {
        DVec3::X
    } else {
        DVec3::Z.cross(normal).normalize()
    };
    (u, normal.cross(u))
}

/// The four corners, in order, of the rectangle with opposite corners `corner` and `opposite`
/// whose sides follow `rectangle_axes(normal)`. Without a normal, the corners must differ in
/// exactly two coordinates and the rectangle lies in that axis plane.
pub fn rectangle_corners(
    corner: DVec3,
    opposite: DVec3,
    normal: Option<DVec3>,
) -> Result<[DVec3; 4], CommandError> {
    let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
    let d = opposite - corner;
    let normal = match normal {
        Some(n) if n.is_finite() && n.length() > 1e-9 => n.normalize(),
        Some(_) => return Err(invalid("normal must be a non-zero vector")),
        None => {
            let flat: Vec<DVec3> = [DVec3::X, DVec3::Y, DVec3::Z]
                .into_iter()
                .filter(|axis| d.dot(*axis).abs() < LENGTH_TOL)
                .collect();
            match flat[..] {
                [axis] => axis,
                _ => {
                    return Err(invalid(
                        "corners must differ in exactly two coordinates unless normal is given",
                    ));
                }
            }
        }
    };
    if d.dot(normal).abs() > LENGTH_TOL {
        return Err(invalid("opposite must lie in the plane through corner"));
    }
    let (u, v) = rectangle_axes(normal);
    let (du, dv) = (d.dot(u), d.dot(v));
    if du.abs() < LENGTH_TOL || dv.abs() < LENGTH_TOL {
        return Err(invalid("rectangle must have non-zero width and height"));
    }
    Ok([
        corner,
        corner + u * du,
        corner + u * du + v * dv,
        corner + v * dv,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_follow_the_world_on_axis_planes() {
        assert_eq!(rectangle_axes(DVec3::Z), (DVec3::X, DVec3::Y));
        assert_eq!(rectangle_axes(DVec3::X), (DVec3::Y, DVec3::Z));
        let (u, v) = rectangle_axes(DVec3::Y);
        assert!(u.abs_diff_eq(-DVec3::X, 1e-12) && v.abs_diff_eq(DVec3::Z, 1e-12));
    }

    #[test]
    fn corners_on_the_ground_without_a_normal() {
        let c = rectangle_corners(DVec3::ZERO, DVec3::new(2.0, -3.0, 0.0), None).unwrap();
        assert_eq!(
            c,
            [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, -3.0, 0.0),
                DVec3::new(0.0, -3.0, 0.0)
            ]
        );
    }

    #[test]
    fn ambiguous_or_flat_rectangles_are_rejected() {
        let o = DVec3::ZERO;
        assert!(rectangle_corners(o, DVec3::new(1.0, 1.0, 1.0), None).is_err());
        assert!(rectangle_corners(o, DVec3::new(1.0, 0.0, 0.0), None).is_err());
        assert!(rectangle_corners(o, DVec3::new(1.0, 1.0, 1.0), Some(DVec3::Z)).is_err());
        assert!(rectangle_corners(o, DVec3::new(1.0, 0.0, 0.0), Some(DVec3::Z)).is_err());
        assert!(rectangle_corners(o, DVec3::new(1.0, 1.0, 0.0), Some(DVec3::ZERO)).is_err());
    }
}
```

`crates/session/src/command.rs`, add these variants to `Command` after `AddEdge`:
```rust
    /// Draw a rectangle from one corner to the opposite one, as one undo step. Its sides follow
    /// the world axes on axis planes. `normal` picks the plane; without it the corners must
    /// differ in exactly two coordinates.
    AddRectangle {
        corner: [f64; 3],
        opposite: [f64; 3],
        #[serde(default)]
        normal: Option<[f64; 3]>,
    },
    /// Move a face along its normal by `distance` meters (negative: against the normal),
    /// extruding new sides or stretching the faces around it.
    PushPull {
        face: EntityId,
        distance: f64,
    },
```
and add after the enum:
```rust
impl Command {
    /// True for commands that can change the model (everything except camera moves).
    pub fn changes_model(&self) -> bool {
        !matches!(self, Command::SetCamera { .. })
    }
}
```

`crates/session/src/query.rs`, replace the `Entities` variant (with its doc comment) with:
```rust
    /// Vertices, edges and faces. An entity is returned when `ids` is empty or lists it,
    /// `kinds` is empty or lists its kind, and `within` is absent or contains all its vertices;
    /// unknown ids are simply absent from the result.
    Entities {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        kinds: Vec<EntityKind>,
        #[serde(default)]
        within: Option<Bounds>,
    },
```
and add before `/// The three kinds of entity.`:
```rust
/// An axis-aligned box, bounds included.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    pub fn contains(&self, p: [f64; 3]) -> bool {
        (0..3).all(|i| self.min[i] <= p[i] && p[i] <= self.max[i])
    }
}
```

`crates/session/src/lib.rs`:
```rust
//! The editable document and the command/query layer every client (UI, API, tests) goes through.

pub mod camera;
mod command;
mod document;
mod query;
mod rectangle;

pub use camera::{Camera, Ray};
pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome};
pub use document::{Document, MAX_COORDINATE};
pub use query::{
    Bounds, ChangeEvent, EdgeInfo, EntityKind, FaceInfo, Query, QueryResult, VertexInfo,
};
pub use rectangle::{rectangle_axes, rectangle_corners};
```

`crates/session/src/document.rs`:

Add the import `use crate::rectangle::rectangle_corners;` after the `crate::query` import.

Replace `execute` with `execute` plus `edit`:
```rust
    /// Runs one command. Each modeling command is one undo step; camera commands are not recorded.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        if let Some(changes) = self.edit(&command) {
            return Ok(self.record(changes?));
        }
        match command {
            Command::Undo => {
                let changes = self.undo.pop().ok_or_else(|| {
                    CommandError::new(ErrorKind::NothingToUndo, "nothing to undo")
                })?;
                let inverse = changes.inverse();
                self.model.apply(&inverse);
                self.redo.push(changes);
                Ok(self.changed(&inverse))
            }
            Command::Redo => {
                let changes = self.redo.pop().ok_or_else(|| {
                    CommandError::new(ErrorKind::NothingToRedo, "nothing to redo")
                })?;
                self.model.apply(&changes);
                let outcome = self.changed(&changes);
                self.undo.push(changes);
                Ok(outcome)
            }
            Command::SetCamera {
                eye,
                target,
                fov_y_deg,
            } => {
                self.camera = self.camera_for(eye, target, fov_y_deg)?;
                Ok(Outcome::default())
            }
            Command::AddEdge { .. } | Command::AddRectangle { .. } | Command::PushPull { .. } => {
                unreachable!("edits are handled by Document::edit")
            }
        }
    }

    /// Applies an editing command to the model and returns its changes, not yet recorded in
    /// history. None for commands that are not edits (undo, redo, camera). Every editing
    /// command goes through here, so `execute` and atomic batches stay in step.
    fn edit(&mut self, command: &Command) -> Option<Result<ChangeSet, CommandError>> {
        Some(match command {
            Command::AddEdge { start, end } => self.add_edge_changes(*start, *end),
            Command::AddRectangle {
                corner,
                opposite,
                normal,
            } => self.add_rectangle_changes(*corner, *opposite, *normal),
            Command::PushPull { face, distance } => self.push_pull_changes(*face, *distance),
            Command::Undo | Command::Redo | Command::SetCamera { .. } => return None,
        })
    }
```

In `execute_batch`, replace the atomic loop's `let result = match command { ... };` with:
```rust
            let result = match self.edit(&command) {
                Some(changes) => changes.map(|c| {
                    let summary = c.summary();
                    applied.push(c);
                    Outcome { changes: summary }
                }),
                None if matches!(command, Command::Undo | Command::Redo) => Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    "undo and redo are not allowed in an atomic batch",
                )),
                None => self.execute(command),
            };
```

Add after `add_edge_changes`:
```rust
    fn add_rectangle_changes(
        &mut self,
        corner: [f64; 3],
        opposite: [f64; 3],
        normal: Option<[f64; 3]>,
    ) -> Result<ChangeSet, CommandError> {
        check_coordinates(&[corner, opposite])?;
        let corners = rectangle_corners(
            DVec3::from(corner),
            DVec3::from(opposite),
            normal.map(DVec3::from),
        )?;
        check_coordinates(&corners.map(|c| c.to_array()))?;
        self.model
            .add_polygon(&corners)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }

    fn push_pull_changes(
        &mut self,
        face: EntityId,
        distance: f64,
    ) -> Result<ChangeSet, CommandError> {
        let EntityId::Face(id) = face else {
            return Err(
                CommandError::new(ErrorKind::InvalidInput, "push_pull needs a face id")
                    .with_ids(vec![face]),
            );
        };
        let Some(f) = self.model.face(id) else {
            return Err(
                CommandError::new(ErrorKind::NotFound, format!("no entity {face}"))
                    .with_ids(vec![face]),
            );
        };
        if !distance.is_finite() || distance.abs() > MAX_COORDINATE {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("distance must be finite and within +/-{MAX_COORDINATE} m"),
            ));
        }
        let moved: Vec<[f64; 3]> = std::iter::once(&f.outer)
            .chain(&f.holes)
            .flatten()
            .map(|v| (self.model.position(*v).unwrap() + f.normal * distance).to_array())
            .collect();
        check_coordinates(&moved)?;
        self.model.push_pull(id, distance).map_err(|e| {
            CommandError::new(ErrorKind::InvalidInput, e.to_string()).with_ids(vec![face])
        })
    }
```

In `query`, the `Query::Entities` arm starts:
```rust
            Query::Entities { ids, kinds, within } => {
                let wanted = |id: EntityId, kind: EntityKind| {
                    (ids.is_empty() || ids.contains(&id))
                        && (kinds.is_empty() || kinds.contains(&kind))
                        && within.is_none_or(|b| self.vertices_of(id).all(|p| b.contains(p)))
                };
```
(the rest of the arm is unchanged), and add before `fn vertex_info`:
```rust
    /// Positions of the vertices an entity is made of.
    fn vertices_of(&self, id: EntityId) -> impl Iterator<Item = [f64; 3]> + '_ {
        let m = &self.model;
        let ids: Vec<VertexId> = match id {
            EntityId::Vertex(v) => vec![v],
            EntityId::Edge(e) => m.edge(e).map(|e| e.v.to_vec()).unwrap_or_default(),
            EntityId::Face(f) => m
                .face(f)
                .map(|f| {
                    f.outer
                        .iter()
                        .chain(f.holes.iter().flatten())
                        .copied()
                        .collect()
                })
                .unwrap_or_default(),
        };
        ids.into_iter()
            .filter_map(|v| m.position(v).map(|p| p.to_array()))
    }
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 214 (session modeling 10, rectangle unit tests 3).

- [ ] **Step 5: Commit**

```bash
git add crates/session/src/rectangle.rs crates/session/src/command.rs crates/session/src/query.rs crates/session/src/document.rs crates/session/src/lib.rs crates/session/tests/commands.rs crates/session/tests/modeling.rs
git commit -m "Add rectangle and push/pull commands and a bounds filter for entities"
```

---
### Task 4: Inference reports faces and skips hidden points; measurements parser

**Files:**
- Modify: `crates/tools/src/inference.rs`, `crates/tools/src/lib.rs`, `crates/tools/src/line.rs` (test literal), `crates/tools/src/overlay.rs` (test literals)
- Create: `crates/tools/src/vcb.rs`

**Interfaces:**
- Consumes: `Model::add_polygon`, `Model::push_pull` (tests build a box).
- Produces:
  - `Inference` gains `pub face: Option<FaceId>`: the nearest face under the cursor, set whatever the point snapped to.
  - `tools::face_under(model: &Model, camera: &Camera, cursor: DVec2) -> Option<(FaceId, DVec3)>`.
  - Inference skips endpoints, midpoints, the origin and on-edge points when a face lies between the eye and the point. Axis guides are not hidden.
  - `tools::vcb::{parse_length(&str) -> Option<f64>, parse_lengths(&str) -> Option<Vec<f64>>, format_length(f64) -> String, toward_cursor(cursor: f64, value: f64) -> f64}`.
    - Unitless input is meters. Units: `mm`, `cm`, `m`, `in`/`"`, `ft`/`'`, and `5'6"`.
    - `format_length` gives meters with up to 3 decimals: `"2.5 m"`.
    - `toward_cursor` negates a typed value when the live measurement is below `-LENGTH_TOL`.

- [ ] **Step 1: Write the failing tests**

In `crates/tools/src/inference.rs`, replace the whole `#[cfg(test)] mod tests` module with:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 10.0;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    fn px(c: &Camera, p: DVec3) -> DVec2 {
        c.project(p).unwrap()
    }

    fn square(m: &mut Model, z: f64) {
        let c = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, z), DVec3::new(b.0, b.1, z))
                .unwrap();
        }
    }

    #[test]
    fn snaps_to_a_nearby_endpoint_exactly() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 1.0, 0.0))
            .unwrap();
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(3.0, 1.0, 0.0)) + DVec2::new(4.0, -3.0),
            None,
            R,
        );
        assert_eq!(
            hit,
            Inference {
                point: DVec3::new(3.0, 1.0, 0.0),
                kind: SnapKind::Endpoint,
                face: None,
            }
        );
    }

    #[test]
    fn snaps_to_midpoints_and_the_origin() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 1.0, 0.0))
            .unwrap();
        let mid = infer(&m, &c, px(&c, DVec3::new(2.0, 1.0, 0.0)), None, R);
        assert_eq!(mid.kind, SnapKind::Midpoint);
        assert!(mid.point.abs_diff_eq(DVec3::new(2.0, 1.0, 0.0), 1e-12));
        let origin = infer(&m, &c, px(&c, DVec3::ZERO) + DVec2::new(2.0, 2.0), None, R);
        assert_eq!(origin.kind, SnapKind::Origin);
    }

    #[test]
    fn snaps_onto_an_edge_between_its_points() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(1.0, 1.0, 0.0), DVec3::new(5.0, 1.0, 0.0))
            .unwrap();
        let hit = infer(
            &m,
            &c,
            px(&c, DVec3::new(4.2, 1.0, 0.0)) + DVec2::new(0.0, 3.0),
            None,
            R,
        );
        assert_eq!(hit.kind, SnapKind::OnEdge);
        assert!((hit.point.y - 1.0).abs() < 1e-9 && hit.point.z.abs() < 1e-9);
        assert!((hit.point.x - 4.2).abs() < 0.1);
    }

    #[test]
    fn follows_an_axis_from_the_anchor() {
        let c = cam();
        let anchor = DVec3::new(1.0, 1.0, 0.0);
        let target = DVec3::new(1.0, 1.0, 2.5); // straight up (blue)
        let hit = infer(
            &Model::new(),
            &c,
            px(&c, target) + DVec2::new(5.0, 0.0),
            Some(anchor),
            R,
        );
        assert_eq!(hit.kind, SnapKind::Axis(Axis::Z));
        assert!(hit.point.x == 1.0 && hit.point.y == 1.0);
        assert!((hit.point.z - 2.5).abs() < 0.2);
    }

    #[test]
    fn points_beat_axes() {
        let (c, mut m) = (cam(), Model::new());
        let anchor = DVec3::new(0.0, 3.0, 0.0);
        m.add_edge(DVec3::new(2.0, 3.0, 0.0), DVec3::new(2.0, 5.0, 0.0))
            .unwrap();
        let hit = infer(&m, &c, px(&c, DVec3::new(2.0, 3.0, 0.0)), Some(anchor), R);
        assert_eq!(hit.kind, SnapKind::Endpoint);
    }

    #[test]
    fn lands_on_faces_then_ground_then_free() {
        let (c, mut m) = (cam(), Model::new());
        square(&mut m, 1.0);
        let on_face = infer(&m, &c, px(&c, DVec3::new(1.3, 0.7, 1.0)), None, R);
        assert_eq!(on_face.kind, SnapKind::OnFace);
        assert!((on_face.point.z - 1.0).abs() < 1e-9);

        let ground = infer(
            &Model::new(),
            &c,
            px(&c, DVec3::new(-3.0, 4.0, 0.0)),
            None,
            R,
        );
        assert_eq!(ground.kind, SnapKind::Ground);
        assert!(ground.point.z.abs() < 1e-9);

        let mut level = cam();
        level.eye = DVec3::new(0.0, -10.0, 1.0);
        level.target = DVec3::new(0.0, 0.0, 1.0);
        let sky = infer(&Model::new(), &level, DVec2::new(400.0, 0.0), None, R);
        assert_eq!(sky.kind, SnapKind::Free);
    }

    #[test]
    fn inner_face_inside_another_faces_hole_is_hit() {
        let (c, mut m) = (cam(), Model::new());
        let pts = [(-2.0, -2.0), (4.0, -2.0), (4.0, 4.0), (-2.0, 4.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, 1.0), DVec3::new(b.0, b.1, 1.0))
                .unwrap();
        }
        square(&mut m, 1.0); // inner square (0,0)-(2,2): a face and a hole of the outer face
        let inside_inner = infer(&m, &c, px(&c, DVec3::new(1.2, 0.8, 1.0)), None, R);
        assert_eq!(inside_inner.kind, SnapKind::OnFace);
        assert!((inside_inner.point.z - 1.0).abs() < 1e-9);
    }

    /// A 2 m box on the ground at the origin, built through push/pull.
    fn cube() -> Model {
        let mut m = Model::new();
        let pts = [
            DVec3::ZERO,
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(2.0, 2.0, 0.0),
            DVec3::new(0.0, 2.0, 0.0),
        ];
        m.add_polygon(&pts).unwrap();
        let base = m.faces().next().unwrap().0;
        m.push_pull(base, 2.0).unwrap();
        m
    }

    #[test]
    fn hidden_corners_are_not_snapped() {
        let (c, m) = (cam(), cube());
        // The camera looks from (-8, -12, 7): the far bottom corner is behind the box.
        let hidden = DVec3::new(2.0, 2.0, 0.0);
        let hit = infer(&m, &c, px(&c, hidden), None, R);
        assert_ne!(hit.kind, SnapKind::Endpoint);
        assert_eq!(hit.kind, SnapKind::OnFace);
        assert!(hit.face.is_some());

        let near_top = DVec3::new(2.0, 2.0, 2.0);
        let hit = infer(&m, &c, px(&c, near_top), None, R);
        assert_eq!(hit.kind, SnapKind::Endpoint);
        assert_eq!(hit.point, near_top);
    }

    #[test]
    fn reports_the_face_under_the_cursor() {
        let (c, m) = (cam(), cube());
        let top = m
            .faces()
            .find(|(_, f)| f.normal == DVec3::Z)
            .map(|(id, _)| id)
            .unwrap();
        let cursor = px(&c, DVec3::new(1.0, 1.2, 2.0));
        let hit = infer(&m, &c, cursor, None, R);
        assert_eq!((hit.kind, hit.face), (SnapKind::OnFace, Some(top)));
        let (face, point) = face_under(&m, &c, cursor).unwrap();
        assert_eq!(face, top);
        assert!((point.z - 2.0).abs() < 1e-9);
        assert!(face_under(&m, &c, px(&c, DVec3::new(-3.0, 4.0, 0.0))).is_none());
    }

    #[test]
    fn axis_guides_show_through_faces() {
        let (c, m) = (cam(), cube());
        let anchor = DVec3::new(3.0, 3.0, 0.0);
        let behind = DVec3::new(3.0, 3.0, 1.0);
        let hit = infer(&m, &c, px(&c, behind), Some(anchor), R);
        assert_eq!(hit.kind, SnapKind::Axis(Axis::Z));
    }
}
```

Create `crates/tools/src/vcb.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-12)
    }

    #[test]
    fn metric_lengths() {
        assert!(close(parse_length("2.5"), 2.5));
        assert!(close(parse_length(" 2m "), 2.0));
        assert!(close(parse_length("250mm"), 0.25));
        assert!(close(parse_length("25 cm"), 0.25));
        assert!(close(parse_length(".5"), 0.5));
        assert!(close(parse_length("-1.2M"), -1.2));
    }

    #[test]
    fn imperial_lengths() {
        assert!(close(parse_length("10in"), 0.254));
        assert!(close(parse_length("10\""), 0.254));
        assert!(close(parse_length("3ft"), 0.9144));
        assert!(close(parse_length("3'"), 0.9144));
        assert!(close(parse_length("5'6\""), 66.0 * INCH));
        assert!(close(parse_length("5' 6"), 66.0 * INCH));
    }

    #[test]
    fn rejects_non_lengths() {
        for bad in ["", "m", "abc", "2km", "1e3", "2..5", "--2", "5'x", "'6"] {
            assert_eq!(parse_length(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn comma_separated_lengths() {
        assert_eq!(parse_lengths("2,3"), Some(vec![2.0, 3.0]));
        assert_eq!(parse_lengths("200mm, 1.5m"), Some(vec![0.2, 1.5]));
        assert_eq!(parse_lengths("2,"), None);
    }

    #[test]
    fn typed_values_follow_the_cursor_direction() {
        assert_eq!(toward_cursor(0.4, 2.0), 2.0);
        assert_eq!(toward_cursor(-0.4, 2.0), -2.0);
        assert_eq!(toward_cursor(-0.4, -2.0), 2.0);
        assert_eq!(
            toward_cursor(-1e-12, 2.0),
            2.0,
            "rounding noise is not a direction"
        );
    }

    #[test]
    fn formats_meters() {
        assert_eq!(format_length(2.5), "2.5 m");
        assert_eq!(format_length(2.0), "2 m");
        assert_eq!(format_length(0.12345), "0.123 m");
        assert_eq!(format_length(-0.0001), "0 m");
        assert_eq!(format_length(-1.25), "-1.25 m");
    }
}
```

`crates/tools/src/lib.rs`: add `pub mod vcb;` after `pub mod overlay;`, and export `face_under`:
```rust
pub use inference::{Axis, Inference, SnapKind, face_under, infer};
```

In the test modules of `crates/tools/src/line.rs` (helper `at`) and `crates/tools/src/overlay.rs` (four `Inference` literals), add `face: None,` after each `kind: ...,` field.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools`
Expected: compile errors: `Inference` has no field `face`, `face_under` and `parse_length` are not found.

- [ ] **Step 3: Implement**

In `crates/tools/src/inference.rs`, replace everything above `#[cfg(test)]` with:
```rust
//! Snapping the cursor to meaningful 3D points.

use glam::{DVec2, DVec3};
use kernel::geom::{Plane, point_in_polygon};
use kernel::{FaceId, Model};
use session::{Camera, Ray};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn dir(self) -> DVec3 {
        match self {
            Axis::X => DVec3::X,
            Axis::Y => DVec3::Y,
            Axis::Z => DVec3::Z,
        }
    }
}

/// What the cursor snapped to, from strongest to weakest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Origin,
    OnEdge,
    /// Along a red/green/blue axis through the point the tool is drawing from.
    Axis(Axis),
    OnFace,
    Ground,
    /// Nothing to snap to: a point in front of the camera under the cursor.
    Free,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inference {
    pub point: DVec3,
    pub kind: SnapKind,
    /// The nearest face under the cursor, whatever the point snapped to.
    pub face: Option<FaceId>,
}

/// Finds the 3D point under `cursor` (physical pixels). `anchor` is the point a tool is drawing
/// from, enabling axis inference. Snap candidates count only within `radius_px` of the cursor.
///
/// Ranking: endpoint, midpoint and origin (nearest on screen wins) > on-edge > axis > on-face >
/// ground > free. Points and edges hidden behind a face are skipped.
pub fn infer(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    anchor: Option<DVec3>,
    radius_px: f64,
) -> Inference {
    let ray = camera.ray(cursor);
    let face_hit = nearest_face_hit(model, &ray);
    let face = face_hit.map(|(f, _)| f);
    let screen_dist = |p: DVec3| camera.project(p).map(|s| s.distance(cursor));
    let within = |p: DVec3| {
        screen_dist(p)
            .filter(|d| *d <= radius_px)
            .filter(|_| visible(model, camera.eye, p))
    };

    // Points: vertices, edge midpoints, origin.
    let mut best: Option<(f64, Inference)> = None;
    let offer = |p: DVec3, kind: SnapKind, best: &mut Option<(f64, Inference)>| {
        if let Some(d) = within(p)
            && best.as_ref().is_none_or(|(bd, _)| d < *bd)
        {
            *best = Some((
                d,
                Inference {
                    point: p,
                    kind,
                    face,
                },
            ));
        }
    };
    for (_, v) in model.vertices() {
        offer(v.pos, SnapKind::Endpoint, &mut best);
    }
    for (_, e) in model.edges() {
        if let (Some(a), Some(b)) = (model.position(e.v[0]), model.position(e.v[1])) {
            offer((a + b) * 0.5, SnapKind::Midpoint, &mut best);
        }
    }
    offer(DVec3::ZERO, SnapKind::Origin, &mut best);
    if let Some((_, hit)) = best {
        return hit;
    }

    // On an edge.
    let mut on_edge: Option<(f64, DVec3)> = None;
    for (_, e) in model.edges() {
        if let (Some(a), Some(b)) = (model.position(e.v[0]), model.position(e.v[1])) {
            let p = closest_on_line_to_ray(a, b - a, &ray, true);
            if let Some(d) = within(p)
                && on_edge.is_none_or(|(bd, _)| d < bd)
            {
                on_edge = Some((d, p));
            }
        }
    }
    if let Some((_, point)) = on_edge {
        return Inference {
            point,
            kind: SnapKind::OnEdge,
            face,
        };
    }

    // Along an axis from the anchor.
    if let Some(anchor) = anchor {
        let mut on_axis: Option<(f64, Inference)> = None;
        for axis in Axis::ALL {
            let p = closest_on_line_to_ray(anchor, axis.dir(), &ray, false);
            // Axis lines are guides, so they are not hidden by faces.
            if let Some(d) = screen_dist(p).filter(|d| *d <= radius_px)
                && p.distance(anchor) > 0.0
                && on_axis.is_none_or(|(bd, _)| d < bd)
            {
                on_axis = Some((
                    d,
                    Inference {
                        point: p,
                        kind: SnapKind::Axis(axis),
                        face,
                    },
                ));
            }
        }
        if let Some((_, hit)) = on_axis {
            return hit;
        }
    }

    // On a face (nearest along the ray).
    if let Some((_, point)) = face_hit {
        return Inference {
            point,
            kind: SnapKind::OnFace,
            face,
        };
    }

    match ray.hit_ground() {
        Some(point) if point.distance(ray.origin) <= camera.distance() * 50.0 => Inference {
            point,
            kind: SnapKind::Ground,
            face,
        },
        _ => Inference {
            point: camera.anchor_at(cursor),
            kind: SnapKind::Free,
            face,
        },
    }
}

/// The nearest face under the cursor and the point where the cursor's ray meets it.
pub fn face_under(model: &Model, camera: &Camera, cursor: DVec2) -> Option<(FaceId, DVec3)> {
    nearest_face_hit(model, &camera.ray(cursor))
}

/// True if no face lies between the eye and `p` (faces through `p` itself do not hide it).
fn visible(model: &Model, eye: DVec3, p: DVec3) -> bool {
    let to = p - eye;
    let dist = to.length();
    if dist < 1e-12 {
        return true;
    }
    let ray = Ray {
        origin: eye,
        dir: to / dist,
    };
    nearest_face_hit(model, &ray).is_none_or(|(_, q)| q.distance(eye) >= dist * (1.0 - 1e-6))
}

/// Point on the line `origin + t * dir` closest to the ray; `t` is clamped to [0, 1] when
/// `segment` is true (dir is then the segment vector).
fn closest_on_line_to_ray(origin: DVec3, dir: DVec3, ray: &Ray, segment: bool) -> DVec3 {
    let w0 = origin - ray.origin;
    let a = dir.dot(dir);
    let b = dir.dot(ray.dir);
    let c = ray.dir.dot(ray.dir);
    let d = dir.dot(w0);
    let e = ray.dir.dot(w0);
    let denom = a * c - b * b;
    let mut t = if denom.abs() < 1e-12 * a * c {
        0.0
    } else {
        (b * e - c * d) / denom
    };
    if segment {
        t = t.clamp(0.0, 1.0);
    }
    origin + dir * t
}

fn nearest_face_hit(model: &Model, ray: &Ray) -> Option<(FaceId, DVec3)> {
    let mut best: Option<(f64, FaceId, DVec3)> = None;
    for (id, f) in model.faces() {
        let Some(origin) = f.outer.first().and_then(|v| model.position(*v)) else {
            continue;
        };
        let plane = Plane {
            normal: f.normal,
            d: f.normal.dot(origin),
        };
        let denom = plane.normal.dot(ray.dir);
        if denom.abs() < 1e-12 {
            continue;
        }
        let t = -plane.distance(ray.origin) / denom;
        if t <= 0.0 || best.is_some_and(|(bt, _, _)| t >= bt) {
            continue;
        }
        let p = ray.origin + ray.dir * t;
        let project = |l: &Vec<kernel::VertexId>| -> Vec<DVec2> {
            l.iter()
                .filter_map(|v| model.position(*v))
                .map(|q| plane.project(q))
                .collect()
        };
        let q = plane.project(p);
        if point_in_polygon(q, &project(&f.outer))
            && !f.holes.iter().any(|h| point_in_polygon(q, &project(h)))
        {
            best = Some((t, id, p));
        }
    }
    best.map(|(_, id, p)| (id, p))
}
```

`nearest_face_hit` now returns the face id with the point. A candidate point is hidden when the nearest face along the ray from the eye to it is closer than `dist * (1 - 1e-6)`. Faces through the point itself therefore do not hide it.

Prepend to `crates/tools/src/vcb.rs`:
```rust
//! The measurements box: parsing typed lengths and formatting live ones.

use kernel::geom::LENGTH_TOL;

const INCH: f64 = 0.0254;

/// Parses a length in meters: "2.5" (meters), "250mm", "25cm", "2m", "10in", "10\"", "3ft",
/// "3'", "5'6\"" or "5'6". A leading "-" negates it. None if it is not a length.
pub fn parse_length(s: &str) -> Option<f64> {
    let s = s.trim();
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => (-1.0, rest.trim_start()),
        None => (1.0, s),
    };
    let meters = match s.split_once('\'') {
        Some((feet, inches)) => {
            let inches = inches.trim();
            let inches = inches.strip_suffix('"').unwrap_or(inches).trim();
            let inches = if inches.is_empty() {
                0.0
            } else {
                number(inches)?
            };
            (number(feet.trim())? * 12.0 + inches) * INCH
        }
        None => {
            let split = s
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(s.len());
            let (value, unit) = s.split_at(split);
            number(value)? * unit_scale(unit.trim())?
        }
    };
    meters.is_finite().then_some(sign * meters)
}

/// Comma-separated lengths, each with its own unit: "2,3" or "200mm, 1.5m".
pub fn parse_lengths(s: &str) -> Option<Vec<f64>> {
    s.split(',').map(parse_length).collect()
}

/// A length for display, in meters with up to millimeter precision: "2.5 m".
pub fn format_length(meters: f64) -> String {
    let s = format!("{meters:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    let s = if s == "-0" { "0" } else { s };
    format!("{s} m")
}

/// A typed `value` pointed the way the cursor is: negated when `cursor` (the live measurement)
/// is clearly negative. A cursor within tolerance of zero counts as positive.
pub fn toward_cursor(cursor: f64, value: f64) -> f64 {
    if cursor <= -LENGTH_TOL { -value } else { value }
}

fn number(s: &str) -> Option<f64> {
    let digits = !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.');
    if digits { s.parse().ok() } else { None }
}

fn unit_scale(unit: &str) -> Option<f64> {
    Some(match unit.to_ascii_lowercase().as_str() {
        "" | "m" => 1.0,
        "mm" => 0.001,
        "cm" => 0.01,
        "in" | "\"" => INCH,
        "ft" => 12.0 * INCH,
        _ => return None,
    })
}
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 223 (inference +3, vcb 6).

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/inference.rs crates/tools/src/vcb.rs crates/tools/src/lib.rs crates/tools/src/line.rs crates/tools/src/overlay.rs
git commit -m "Report the face under the cursor, skip hidden snaps, parse typed lengths"
```

---

### Task 5: Tool trait, Line tool on it, editor keys and status

**Files:**
- Create: `crates/tools/src/tool.rs`
- Modify: `crates/tools/src/line.rs`, `crates/tools/src/lib.rs`, `crates/app/src/editor.rs`, `crates/app/src/app.rs`

**Interfaces:**
- Consumes: `Inference`, `infer`, `vcb` (Task 4); `Command::changes_model` (Task 3).
- Produces (`tools`):
  - `ToolContext<'a> { model: &'a Model, camera: &'a Camera, cursor: DVec2 }`.
  - `ToolEvent::{Move(Inference), Click(Inference), Value(String), Cancel}` (now `Clone`, not `Copy`; moved from `line.rs` to `tool.rs`).
  - `ToolOutput { commands: Vec<Command>, preview: Vec<[DVec3; 2]>, message: Option<String> }`.
  - `trait Tool`:
    - `name() -> &'static str`
    - `prompt() -> &'static str`
    - `measurement() -> (&'static str, Option<String>)`
    - `anchor() -> Option<DVec3>`
    - `handle(&mut self, ToolEvent, &ToolContext) -> ToolOutput`
    - `on_external_change(&mut self)`
  - `ToolKind` with `make() -> Box<dyn Tool>` and `from_shortcut(char) -> Option<ToolKind>`. This task has only `Line` (`l`); Tasks 6 and 7 add `Rectangle` (`r`) and `PushPull` (`p`).
  - `LineTool` implements `Tool`. A typed length places the next point that far along the direction from the last point to the cursor; a negative length goes backward.
- Produces (`app`):
  - `EditorKey::{Char(char), Enter, Backspace, Escape}` and `Editor::key(EditorKey) -> bool`:
    - A digit, `.` or `-` starts the measurements text; once started, every printable character is appended.
    - With no text typed, letters are tool shortcuts.
    - Enter sends `ToolEvent::Value`.
    - Escape clears the text first, then cancels the tool.
  - `Editor::select_tool(ToolKind)`, `Editor::status() -> String`, and `Editor::cursor_left() -> bool`.
    - `status()` joins with `"  |  "`: tool name, prompt, `Label: value` (or `Label: typed_` while typing), and any message.
    - `cursor_left()` hides the inference marker.
  - `Editor::cancel` is removed (Escape goes through `key`). `execute`/`execute_batch`/`undo`/`redo` call `Tool::on_external_change` when the command can change the model.
  - Command failures from tools set the status message instead of printing to stderr.
  - Window:
    - The title shows `sketchup-alt  |  <status>`.
    - Key presses without Cmd/Ctrl go to `Editor::key`. On non-ASCII layouts they fall back to the physical key for digits, `.`, `-`, `,` and `l r p m`.
    - `CursorLeft` hides the marker.

- [ ] **Step 1: Write the failing tests**

In `crates/tools/src/line.rs`, replace the whole `#[cfg(test)] mod tests` module with:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;

    fn at(x: f64, y: f64) -> Inference {
        Inference {
            point: DVec3::new(x, y, 0.0),
            kind: SnapKind::Ground,
            face: None,
        }
    }

    fn edge(a: (f64, f64), b: (f64, f64)) -> Command {
        Command::AddEdge {
            start: [a.0, a.1, 0.0],
            end: [b.0, b.1, 0.0],
        }
    }

    /// Runs events through a Line tool with an empty model.
    struct Rig {
        tool: LineTool,
        model: Model,
        camera: Camera,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                tool: LineTool::new(),
                model: Model::new(),
                camera: Camera::new(DVec2::new(800.0, 600.0)),
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                camera: &self.camera,
                cursor: DVec2::ZERO,
            };
            self.tool.handle(event, &ctx)
        }

        fn click(&mut self, x: f64, y: f64) -> ToolOutput {
            self.send(ToolEvent::Click(at(x, y)))
        }
    }

    #[test]
    fn first_click_starts_and_moving_previews() {
        let mut r = Rig::new();
        assert!(r.click(0.0, 0.0).commands.is_empty());
        let out = r.send(ToolEvent::Move(at(2.0, 0.0)));
        assert_eq!(out.preview, vec![[DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0)]]);
        assert_eq!(r.tool.anchor(), Some(DVec3::ZERO));
        assert_eq!(r.tool.measurement(), ("Length", Some("2 m".to_string())));
    }

    #[test]
    fn clicks_chain_edges() {
        let mut r = Rig::new();
        r.click(0.0, 0.0);
        let a = r.click(2.0, 0.0);
        let b = r.click(2.0, 1.0);
        assert_eq!(a.commands, vec![edge((0.0, 0.0), (2.0, 0.0))]);
        assert_eq!(b.commands, vec![edge((2.0, 0.0), (2.0, 1.0))]);
        assert_eq!(r.tool.anchor(), Some(DVec3::new(2.0, 1.0, 0.0)));
    }

    #[test]
    fn clicking_the_first_point_closes_the_chain() {
        let mut r = Rig::new();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)] {
            r.click(x, y);
        }
        let out = r.click(0.0, 0.0);
        assert_eq!(out.commands, vec![edge((1.0, 1.0), (0.0, 0.0))]);
        assert_eq!(r.tool.anchor(), None);
        assert!(out.preview.is_empty());
    }

    #[test]
    fn clicking_the_same_point_twice_does_nothing() {
        let mut r = Rig::new();
        r.click(1.0, 1.0);
        assert!(r.click(1.0, 1.0).commands.is_empty());
        assert_eq!(r.tool.anchor(), Some(DVec3::new(1.0, 1.0, 0.0)));
    }

    #[test]
    fn cancel_drops_the_chain() {
        let mut r = Rig::new();
        r.click(0.0, 0.0);
        let out = r.send(ToolEvent::Cancel);
        assert!(out.preview.is_empty() && out.commands.is_empty());
        assert_eq!(r.tool.anchor(), None);
    }

    #[test]
    fn typed_length_places_the_point_along_the_cursor_direction() {
        let mut r = Rig::new();
        r.click(1.0, 1.0);
        r.send(ToolEvent::Move(at(1.0, 5.0)));
        let out = r.send(ToolEvent::Value("2.5".into()));
        assert_eq!(out.commands, vec![edge((1.0, 1.0), (1.0, 3.5))]);
        assert_eq!(r.tool.anchor(), Some(DVec3::new(1.0, 3.5, 0.0)));
        let back = r.send(ToolEvent::Value("-500mm".into()));
        assert_eq!(back.commands, vec![edge((1.0, 3.5), (1.0, 3.0))]);
    }

    #[test]
    fn typed_text_that_cannot_be_used_explains_why() {
        let mut r = Rig::new();
        assert!(r.send(ToolEvent::Value("2".into())).message.is_some());
        r.click(0.0, 0.0);
        r.send(ToolEvent::Move(at(3.0, 0.0)));
        let out = r.send(ToolEvent::Value("two".into()));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Not a length: two"));
    }

    #[test]
    fn external_change_drops_the_chain() {
        let mut r = Rig::new();
        r.click(0.0, 0.0);
        r.tool.on_external_change();
        assert_eq!(r.tool.anchor(), None);
    }
}
```

In `crates/app/src/editor.rs`, replace the whole `#[cfg(test)] mod tests` module with:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use session::{Query, QueryResult};

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.25)
    }

    fn click_at(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }

    fn stats(e: &Editor) -> (usize, usize) {
        match e.doc().query(&Query::Stats).unwrap() {
            QueryResult::Stats { edges, faces, .. } => (edges, faces),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn square(e: &mut Editor) {
        for (x, y) in [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0), (1.0, 1.0)] {
            click_at(e, DVec3::new(x, y, 0.0));
        }
    }

    #[test]
    fn clicking_four_corners_draws_a_face() {
        let mut e = editor();
        square(&mut e);
        assert_eq!(stats(&e), (4, 1));
        let model = e.doc().model();
        let (id, _) = model.faces().next().unwrap();
        assert!((model.area(id).unwrap() - 4.0).abs() < 1e-6);
    }

    #[test]
    fn undo_and_redo_step_through_edges() {
        let mut e = editor();
        square(&mut e);
        e.undo();
        assert_eq!(stats(&e), (3, 0));
        e.redo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn preview_follows_the_cursor_until_cancelled() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let px = e.doc().camera.project(DVec3::new(-2.0, 4.0, 0.0)).unwrap();
        assert!(e.cursor_moved(px));
        assert!(!e.overlay().is_empty(), "preview line expected");
        e.key(EditorKey::Escape);
        assert!(
            e.overlay().is_empty(),
            "ground hover has no marker and no preview after cancel"
        );
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn undo_mid_chain_cancels_the_chain() {
        let mut e = editor();
        for (x, y) in [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0)] {
            click_at(&mut e, DVec3::new(x, y, 0.0));
        }
        e.undo();
        click_at(&mut e, DVec3::new(5.0, 5.0, 0.0));
        assert_eq!(stats(&e).0, 1, "the click only starts a new chain");
    }

    #[test]
    fn alt_left_drag_orbits_without_drawing() {
        let mut e = editor();
        let before = e.doc().camera.clone();
        e.cursor_moved(DVec2::new(400.0, 300.0));
        e.set_modifiers(false, true);
        e.button(Button::Left, true);
        e.cursor_moved(DVec2::new(460.0, 300.0));
        e.button(Button::Left, false);
        assert_ne!(e.doc().camera, before);
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn hovering_an_endpoint_shows_a_marker() {
        let mut e = editor();
        square(&mut e);
        let px = e.doc().camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(px + DVec2::new(3.0, 0.0));
        assert_eq!(e.overlay().len(), 8, "a 4-sided marker, no preview");
    }

    #[test]
    fn api_edit_mid_chain_cancels_the_chain() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let out = e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [6.0, 5.0, 0.0],
        });
        assert!(out.is_ok());
        click_at(&mut e, DVec3::new(-2.0, -2.0, 0.0));
        assert_eq!(
            stats(&e),
            (1, 0),
            "the click starts a new chain instead of adding an edge"
        );
    }

    #[test]
    fn api_camera_move_keeps_the_chain() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        e.execute(Command::SetCamera {
            eye: [-6.0, -10.0, 6.0],
            target: [0.0, 0.0, 0.0],
            fov_y_deg: None,
        })
        .unwrap();
        click_at(&mut e, DVec3::new(3.0, 1.0, 0.0));
        assert_eq!(stats(&e), (1, 0), "the chain continued");
    }

    fn move_to(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
    }

    fn type_text(e: &mut Editor, text: &str) {
        for c in text.chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
    }

    #[test]
    fn typing_shows_in_the_status_and_escape_clears_it_first() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        e.key(EditorKey::Char('2'));
        e.key(EditorKey::Char('.'));
        e.key(EditorKey::Char('5'));
        e.key(EditorKey::Backspace);
        assert!(e.status().contains("Length: 2._"), "{}", e.status());
        e.key(EditorKey::Escape);
        assert!(!e.status().contains('_'));
        assert!(e.status().contains("Click the next point"), "chain kept");
        e.key(EditorKey::Escape);
        assert!(e.status().contains("Click to start a line"));
    }

    #[test]
    fn typed_length_draws_the_next_edge() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(4.0, 1.0, 0.0));
        type_text(&mut e, "2");
        assert_eq!(stats(&e), (1, 0));
        let end = DVec3::new(3.0, 1.0, 0.0);
        let model = e.doc().model();
        assert!(model.vertices().any(|(_, v)| v.pos.abs_diff_eq(end, 1e-9)));
    }

    #[test]
    fn letters_are_shortcuts_only_when_nothing_is_typed() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        assert!(
            e.key(EditorKey::Char('l')),
            "selecting Line again restarts it"
        );
        assert!(e.status().contains("Click to start a line"));
        assert!(!e.key(EditorKey::Char('z')));
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        e.key(EditorKey::Char('1'));
        e.key(EditorKey::Char('m'));
        assert!(e.status().contains("Length: 1m_"), "{}", e.status());
    }

    #[test]
    fn leaving_the_window_hides_the_marker() {
        let mut e = editor();
        square(&mut e);
        let px = e.doc().camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(px);
        assert!(!e.overlay().is_empty());
        assert!(e.cursor_left());
        assert!(e.overlay().is_empty());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools -p app`
Expected: compile errors: `ToolContext`, `Tool`, `EditorKey`, `Editor::key`, `Editor::status`, `Editor::cursor_left` not found.

- [ ] **Step 3: Implement**

Create `crates/tools/src/tool.rs`:
```rust
//! What every interactive tool implements, and the tools the app can switch between.

use glam::{DVec2, DVec3};
use kernel::Model;
use session::{Camera, Command};

use crate::inference::Inference;
use crate::line::LineTool;

/// What a tool may look at while handling an event.
pub struct ToolContext<'a> {
    pub model: &'a Model,
    pub camera: &'a Camera,
    /// Cursor position in physical pixels.
    pub cursor: DVec2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolEvent {
    /// The cursor moved; carries the current inference.
    Move(Inference),
    /// The primary button was pressed at the current inference.
    Click(Inference),
    /// Text typed into the measurements box, submitted with Enter.
    Value(String),
    /// Escape: abandon the operation in progress.
    Cancel,
}

/// What a tool wants done after an event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolOutput {
    /// Commands to execute, in order (each is one undo step).
    pub commands: Vec<Command>,
    /// Segments to draw as a live preview.
    pub preview: Vec<[DVec3; 2]>,
    /// Feedback for the user, e.g. why typed text was ignored.
    pub message: Option<String>,
}

/// An interactive way to build commands. Tools never edit the model themselves.
pub trait Tool {
    fn name(&self) -> &'static str;
    /// What the user should do next.
    fn prompt(&self) -> &'static str;
    /// The measurements box label and the live value, e.g. ("Length", Some("2.5 m")).
    fn measurement(&self) -> (&'static str, Option<String>);
    /// The point axis inference starts from, if any.
    fn anchor(&self) -> Option<DVec3>;
    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput;
    /// The model changed outside the tool (agent edit, undo, redo): drop any operation in
    /// progress, since it may refer to geometry that no longer exists.
    fn on_external_change(&mut self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Line,
}

impl ToolKind {
    pub fn make(self) -> Box<dyn Tool> {
        match self {
            ToolKind::Line => Box::new(LineTool::new()),
        }
    }

    /// The keyboard shortcut that selects the tool.
    pub fn from_shortcut(c: char) -> Option<ToolKind> {
        match c.to_ascii_lowercase() {
            'l' => Some(ToolKind::Line),
            _ => None,
        }
    }
}
```

In `crates/tools/src/line.rs`, replace everything above `#[cfg(test)]` with:
```rust
//! The Line tool: click to start, click again to place each edge; edges chain from the last point.
//! Typing a length places the next point that far along the current direction.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length};

#[derive(Clone, Debug, Default)]
pub struct LineTool {
    /// First point of the current chain; clicking it again closes the chain.
    first: Option<DVec3>,
    /// Point the next edge starts from.
    last: Option<DVec3>,
    /// Latest inferred cursor point.
    hover: Option<DVec3>,
}

impl LineTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the edge from the chain's last point to `p` and continues (or closes) the chain.
    fn place(&mut self, last: DVec3, p: DVec3, out: &mut ToolOutput) {
        out.commands.push(Command::AddEdge {
            start: last.to_array(),
            end: p.to_array(),
        });
        if self.first.is_some_and(|f| f.distance(p) < LENGTH_TOL) {
            self.reset();
        } else {
            self.last = Some(p);
        }
    }

    fn reset(&mut self) {
        self.first = None;
        self.last = None;
    }
}

impl Tool for LineTool {
    fn name(&self) -> &'static str {
        "Line"
    }

    fn prompt(&self) -> &'static str {
        match self.last {
            None => "Click to start a line",
            Some(_) => "Click the next point, or type a length",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        let length = self
            .last
            .zip(self.hover)
            .map(|(a, b)| format_length(a.distance(b)));
        ("Length", length)
    }

    fn anchor(&self) -> Option<DVec3> {
        self.last
    }

    fn handle(&mut self, event: ToolEvent, _ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => {
                let p = hit.point;
                self.hover = Some(p);
                match self.last {
                    None => {
                        self.first = Some(p);
                        self.last = Some(p);
                    }
                    Some(last) if last.distance(p) < LENGTH_TOL => {}
                    Some(last) => self.place(last, p, &mut out),
                }
            }
            ToolEvent::Value(text) => {
                let dir = self
                    .last
                    .zip(self.hover)
                    .map(|(a, b)| b - a)
                    .filter(|d| d.length() >= LENGTH_TOL);
                match (self.last, dir, parse_length(&text)) {
                    (None, _, _) | (_, None, _) => {
                        out.message = Some("Move the cursor to set a direction first".into());
                    }
                    (_, _, None) => out.message = Some(format!("Not a length: {text}")),
                    (Some(_), Some(_), Some(len)) if len.abs() < LENGTH_TOL => {
                        out.message = Some("Length must not be zero".into());
                    }
                    (Some(last), Some(dir), Some(len)) => {
                        let p = last + dir.normalize() * len;
                        self.place(last, p, &mut out);
                    }
                }
            }
            ToolEvent::Cancel => self.reset(),
        }
        if let (Some(a), Some(b)) = (self.last, self.hover)
            && a.distance(b) >= LENGTH_TOL
        {
            out.preview.push([a, b]);
        }
        out
    }

    fn on_external_change(&mut self) {
        self.reset();
    }
}
```

`crates/tools/src/lib.rs`:
```rust
//! Interactive tools and inference. Tools turn pointer input into `session::Command`s; they never
//! edit the model directly.

pub mod inference;
pub mod line;
pub mod overlay;
pub mod tool;
pub mod vcb;

pub use inference::{Axis, Inference, SnapKind, face_under, infer};
pub use line::LineTool;
pub use tool::{Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};
```

In `crates/app/src/editor.rs`, replace everything above `#[cfg(test)]` with:
```rust
//! Window-independent editing state: routes input to navigation or the active tool, runs the
//! resulting commands on the document, and produces the overlay and status text to show.

use glam::{DVec2, DVec3};
use render::grid::LineVertex;
use session::{BatchError, Command, CommandError, Document, ErrorKind, Outcome};
use tools::overlay::overlay;
use tools::{Inference, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput, infer};

use crate::navigation::{Button, Navigation};

/// Snap radius in logical pixels (scaled to physical pixels by the display scale factor).
const PICK_RADIUS_LOGICAL_PX: f64 = 8.0;

/// Keys the editor handles itself (navigation and undo shortcuts are handled elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKey {
    Char(char),
    Enter,
    Backspace,
    Escape,
}

pub struct Editor {
    doc: Document,
    nav: Navigation,
    tool: Box<dyn Tool>,
    cursor: DVec2,
    hit: Option<Inference>,
    preview: Vec<[DVec3; 2]>,
    /// Text typed into the measurements box, not yet submitted.
    typed: String,
    /// Feedback from the last tool action or failed command.
    message: Option<String>,
    /// Physical pixels per logical pixel.
    scale_factor: f64,
}

impl Editor {
    pub fn new(viewport: DVec2, scale_factor: f64) -> Self {
        Self {
            doc: Document::new(viewport),
            nav: Navigation::default(),
            tool: ToolKind::Line.make(),
            cursor: DVec2::ZERO,
            hit: None,
            preview: Vec::new(),
            typed: String::new(),
            message: None,
            scale_factor,
        }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// Runs a command from outside the UI (the agent API). A command that can change the model
    /// ends the tool's operation in progress, which may refer to geometry that changed.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        if command.changes_model() {
            self.reset_tool();
        }
        let result = self.doc.execute(command);
        self.update_hover();
        result
    }

    /// Batch version of `execute`.
    pub fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        if commands.iter().any(Command::changes_model) {
            self.reset_tool();
        }
        let result = self.doc.execute_batch(commands, atomic);
        self.update_hover();
        result
    }

    /// Switches to another tool, dropping the current tool's operation and any typed text.
    pub fn select_tool(&mut self, kind: ToolKind) {
        self.tool = kind.make();
        self.preview.clear();
        self.typed.clear();
        self.message = None;
        self.update_hover();
    }

    pub fn resize(&mut self, viewport: DVec2) {
        self.doc.camera.viewport = viewport;
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    pub fn set_modifiers(&mut self, shift: bool, alt: bool) {
        self.nav.set_modifiers(shift, alt);
    }

    pub fn focus_lost(&mut self) {
        self.nav.reset();
    }

    /// Returns true if anything visible changed.
    pub fn cursor_moved(&mut self, pos: DVec2) -> bool {
        self.cursor = pos;
        let camera_moved = self.nav.cursor_moved(&mut self.doc.camera, pos);
        self.update_hover() || camera_moved
    }

    /// The cursor left the window: hide the inference marker.
    pub fn cursor_left(&mut self) -> bool {
        self.hit.take().is_some()
    }

    pub fn button(&mut self, button: Button, pressed: bool) -> bool {
        if self.nav.button(&self.doc.camera, button, pressed) {
            return false;
        }
        if button != Button::Left || !pressed {
            return false;
        }
        self.update_hover();
        let Some(hit) = self.hit else { return false };
        self.send(ToolEvent::Click(hit));
        true
    }

    pub fn scroll(&mut self, steps: f64) -> bool {
        let zoomed = self.nav.scroll(&mut self.doc.camera, steps);
        if zoomed {
            self.update_hover();
        }
        zoomed
    }

    /// Typing goes to the measurements box once it starts with a digit, "." or "-"; other
    /// letters are tool shortcuts. Enter submits the typed text to the tool. Escape clears the
    /// typed text, or else cancels the tool's operation. Returns true if anything changed.
    pub fn key(&mut self, key: EditorKey) -> bool {
        match key {
            EditorKey::Char(c) if self.typed.is_empty() => {
                if c.is_ascii_digit() || c == '.' || c == '-' {
                    self.typed.push(c);
                    true
                } else if let Some(kind) = ToolKind::from_shortcut(c) {
                    self.select_tool(kind);
                    true
                } else {
                    false
                }
            }
            EditorKey::Char(c) => {
                if !c.is_control() {
                    self.typed.push(c);
                }
                true
            }
            EditorKey::Backspace => self.typed.pop().is_some(),
            EditorKey::Enter if self.typed.is_empty() => false,
            EditorKey::Enter => {
                let text = std::mem::take(&mut self.typed);
                self.send(ToolEvent::Value(text));
                true
            }
            EditorKey::Escape if !self.typed.is_empty() => {
                self.typed.clear();
                true
            }
            EditorKey::Escape => {
                self.send(ToolEvent::Cancel);
                true
            }
        }
    }

    pub fn undo(&mut self) -> bool {
        self.reset_tool();
        self.run(Command::Undo);
        self.update_hover();
        true
    }

    pub fn redo(&mut self) -> bool {
        self.reset_tool();
        self.run(Command::Redo);
        self.update_hover();
        true
    }

    /// One line describing the tool, what to do next, the measurement and any feedback.
    pub fn status(&self) -> String {
        let (label, value) = self.tool.measurement();
        let mut parts = vec![self.tool.name().to_string(), self.tool.prompt().to_string()];
        if !self.typed.is_empty() {
            parts.push(format!("{label}: {}_", self.typed));
        } else if let Some(value) = value {
            parts.push(format!("{label}: {value}"));
        }
        parts.extend(self.message.clone());
        parts.join("  |  ")
    }

    /// Line pairs for the renderer's overlay pass.
    pub fn overlay(&self) -> Vec<LineVertex> {
        overlay(
            &self.preview,
            self.hit.as_ref(),
            &self.doc.camera,
            self.scale_factor,
        )
        .into_iter()
        .flat_map(|s| {
            [s.a, s.b].map(|p| LineVertex {
                pos: p.as_vec3().to_array(),
                color: s.color,
            })
        })
        .collect()
    }

    /// Sends a click, typed value or cancel to the tool and runs what it asks for.
    fn send(&mut self, event: ToolEvent) {
        let out = self.handle(event);
        self.message = out.message.clone();
        self.apply(out);
        self.update_hover();
    }

    fn handle(&mut self, event: ToolEvent) -> ToolOutput {
        let ctx = ToolContext {
            model: self.doc.model(),
            camera: &self.doc.camera,
            cursor: self.cursor,
        };
        self.tool.handle(event, &ctx)
    }

    fn apply(&mut self, out: ToolOutput) {
        for command in out.commands {
            self.run(command);
        }
        self.preview = out.preview;
    }

    /// Drops the tool's operation in progress so it can't refer to geometry that changed.
    fn reset_tool(&mut self) {
        self.tool.on_external_change();
        self.preview.clear();
    }

    fn run(&mut self, command: Command) {
        match self.doc.execute(command) {
            Ok(_) => {}
            Err(e) if matches!(e.kind, ErrorKind::NothingToUndo | ErrorKind::NothingToRedo) => {}
            Err(e) => self.message = Some(e.message),
        }
    }

    /// Recomputes the inference under the cursor and the tool preview. True if either changed.
    fn update_hover(&mut self) -> bool {
        let hit = infer(
            self.doc.model(),
            &self.doc.camera,
            self.cursor,
            self.tool.anchor(),
            PICK_RADIUS_LOGICAL_PX * self.scale_factor,
        );
        let out = self.handle(ToolEvent::Move(hit));
        let changed = self.hit != Some(hit) || self.preview != out.preview;
        self.hit = Some(hit);
        self.preview = out.preview;
        changed
    }
}
```

`crates/app/src/app.rs`:

1. Import: `use crate::editor::{Editor, EditorKey};`.
2. `State` gains a field after `modifiers`:
```rust
    /// The window title last set, which shows the editor status.
    title: String,
```
initialized with `title: String::new(),` in `State::new`, and a method:
```rust
    /// Shows the editor status (tool, prompt, measurement) in the window title.
    fn sync_title(&mut self) {
        let title = format!("sketchup-alt  |  {}", self.editor.status());
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }
    }
```
3. In `process_api`, call `self.sync_title();` right after `hub.process(&mut host);`.
4. In `resumed`, call `state.sync_title();` right after `let mut state = State::new(window);`.
5. In `window_event`, add the arm `WindowEvent::CursorLeft { .. } => s.editor.cursor_left(),` after the `CursorMoved` arm, and replace the body of the `KeyboardInput` arm after the `is_z` binding with:
```rust
                if command_key && is_z {
                    if s.modifiers.shift_key() {
                        s.editor.redo()
                    } else {
                        s.editor.undo()
                    }
                } else if command_key {
                    false
                } else {
                    editor_key(&event.logical_key, event.physical_key)
                        .is_some_and(|k| s.editor.key(k))
                }
```
6. In the `if changed { ... }` block at the end of `window_event`, call `s.sync_title();` first.
7. Add above `fn map_button`:
```rust
/// The editor key for a key press. Characters from non-ASCII layouts (e.g. Arabic) fall back to
/// the physical key so shortcuts and digits work on any layout.
fn editor_key(logical: &Key, physical: PhysicalKey) -> Option<EditorKey> {
    match logical {
        Key::Named(NamedKey::Enter) => Some(EditorKey::Enter),
        Key::Named(NamedKey::Backspace) => Some(EditorKey::Backspace),
        Key::Named(NamedKey::Escape) => Some(EditorKey::Escape),
        Key::Named(NamedKey::Space) => Some(EditorKey::Char(' ')),
        Key::Character(text) => {
            let c = text.chars().next()?;
            if c.is_ascii() {
                Some(EditorKey::Char(c))
            } else {
                physical_char(physical).map(EditorKey::Char)
            }
        }
        _ => None,
    }
}

/// The US-layout character of a physical key, for the keys the editor uses.
fn physical_char(key: PhysicalKey) -> Option<char> {
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    Some(match code {
        KeyCode::Digit0 => '0',
        KeyCode::Digit1 => '1',
        KeyCode::Digit2 => '2',
        KeyCode::Digit3 => '3',
        KeyCode::Digit4 => '4',
        KeyCode::Digit5 => '5',
        KeyCode::Digit6 => '6',
        KeyCode::Digit7 => '7',
        KeyCode::Digit8 => '8',
        KeyCode::Digit9 => '9',
        KeyCode::Period => '.',
        KeyCode::Minus => '-',
        KeyCode::Comma => ',',
        KeyCode::KeyL => 'l',
        KeyCode::KeyR => 'r',
        KeyCode::KeyP => 'p',
        KeyCode::KeyM => 'm',
        _ => return None,
    })
}
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 230 (line +3, editor +4).

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/tool.rs crates/tools/src/line.rs crates/tools/src/lib.rs crates/app/src/editor.rs crates/app/src/app.rs
git commit -m "Add the Tool trait, typed values and a status line in the window title"
```

---
### Task 6: Rectangle tool

**Files:**
- Create: `crates/tools/src/rectangle.rs`
- Modify: `crates/tools/src/tool.rs`, `crates/tools/src/lib.rs`, `crates/app/src/editor.rs` (tests)

**Interfaces:**
- Consumes: `Tool`, `ToolContext`, `ToolKind` (Task 5); `rectangle_axes`, `rectangle_corners` (Task 3); `parse_lengths`, `format_length`, `toward_cursor` (Task 4).
- Produces:
  - `RectangleTool` (`ToolKind::Rectangle`, shortcut `r`).
    - First click: the corner. The plane is the clicked face's if the point lies on it, otherwise horizontal.
    - The opposite corner is the inferred point when it lies on that plane, otherwise where the cursor ray meets the plane.
    - A second click emits one `Command::AddRectangle` with `normal: Some(..)`. A click that gives no area is ignored.
  - Typed `w,h` extends toward the cursor on each axis. A negative value flips the side, and a zero gives a message.
  - Measurement: `("Dimensions", "2 m, 3 m")`.

- [ ] **Step 1: Write the failing tests**

Create `crates/tools/src/rectangle.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::SnapKind;
    use glam::DVec2;
    use kernel::{FaceId, Model};
    use session::Camera;

    fn ground(x: f64, y: f64) -> Inference {
        Inference {
            point: DVec3::new(x, y, 0.0),
            kind: SnapKind::Ground,
            face: None,
        }
    }

    struct Rig {
        tool: RectangleTool,
        model: Model,
        camera: Camera,
        cursor: DVec2,
    }

    impl Rig {
        fn new(model: Model) -> Self {
            Self {
                tool: RectangleTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                camera: &self.camera,
                cursor: self.cursor,
            };
            self.tool.handle(event, &ctx)
        }
    }

    fn rect(corner: [f64; 3], opposite: [f64; 3], normal: DVec3) -> Command {
        Command::AddRectangle {
            corner,
            opposite,
            normal: Some(normal.to_array()),
        }
    }

    #[test]
    fn two_clicks_on_the_ground_draw_one_rectangle() {
        let mut r = Rig::new(Model::new());
        assert!(
            r.send(ToolEvent::Click(ground(1.0, 1.0)))
                .commands
                .is_empty()
        );
        let moved = r.send(ToolEvent::Move(ground(3.0, 4.0)));
        assert_eq!(moved.preview.len(), 4);
        assert_eq!(
            r.tool.measurement(),
            ("Dimensions", Some("2 m, 3 m".to_string()))
        );
        let out = r.send(ToolEvent::Click(ground(3.0, 4.0)));
        assert_eq!(
            out.commands,
            vec![rect([1.0, 1.0, 0.0], [3.0, 4.0, 0.0], DVec3::Z)]
        );
        assert!(out.preview.is_empty());
        assert_eq!(r.tool.anchor(), None);
    }

    #[test]
    fn a_click_without_area_is_ignored() {
        let mut r = Rig::new(Model::new());
        r.send(ToolEvent::Click(ground(1.0, 1.0)));
        assert!(
            r.send(ToolEvent::Click(ground(3.0, 1.0)))
                .commands
                .is_empty()
        );
        assert!(
            r.tool.anchor().is_some(),
            "still waiting for the opposite corner"
        );
    }

    #[test]
    fn typed_size_extends_toward_the_cursor() {
        let mut r = Rig::new(Model::new());
        r.send(ToolEvent::Click(ground(1.0, 1.0)));
        r.send(ToolEvent::Move(ground(0.0, 3.0)));
        let out = r.send(ToolEvent::Value("2,500mm".into()));
        assert_eq!(
            out.commands,
            vec![rect([1.0, 1.0, 0.0], [-1.0, 1.5, 0.0], DVec3::Z)]
        );
    }

    #[test]
    fn bad_typed_sizes_explain_why() {
        let mut r = Rig::new(Model::new());
        assert!(r.send(ToolEvent::Value("2,3".into())).message.is_some());
        r.send(ToolEvent::Click(ground(1.0, 1.0)));
        for bad in ["2", "2,x", "2,0"] {
            let out = r.send(ToolEvent::Value(bad.into()));
            assert!(out.commands.is_empty() && out.message.is_some(), "{bad}");
        }
    }

    #[test]
    fn starts_on_the_plane_of_the_face_under_the_cursor() {
        // A wall in the plane x = 0.
        let mut m = Model::new();
        let pts = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(0.0, 4.0, 0.0),
            DVec3::new(0.0, 4.0, 3.0),
            DVec3::new(0.0, 0.0, 3.0),
        ];
        m.add_polygon(&pts).unwrap();
        let wall: FaceId = m.faces().next().unwrap().0;
        let normal = m.face(wall).unwrap().normal;
        let mut r = Rig::new(m);
        let on_wall = |y: f64, z: f64| Inference {
            point: DVec3::new(0.0, y, z),
            kind: SnapKind::OnFace,
            face: Some(wall),
        };
        r.send(ToolEvent::Click(on_wall(1.0, 1.0)));
        let out = r.send(ToolEvent::Click(on_wall(2.0, 2.5)));
        assert_eq!(
            out.commands,
            vec![rect([0.0, 1.0, 1.0], [0.0, 2.0, 2.5], normal)]
        );
    }

    #[test]
    fn off_plane_cursor_is_projected_through_the_view_ray() {
        let mut r = Rig::new(Model::new());
        r.send(ToolEvent::Click(ground(0.0, 0.0)));
        let target = DVec3::new(2.0, 1.0, 0.0);
        r.cursor = r.camera.project(target).unwrap();
        // An axis inference above the plane: the corner comes from the cursor ray instead.
        let above = Inference {
            point: DVec3::new(0.0, 0.0, 2.0),
            kind: SnapKind::Axis(crate::Axis::Z),
            face: None,
        };
        r.send(ToolEvent::Move(above));
        let (w, h) = r.tool.size().unwrap();
        assert!((w - 2.0).abs() < 1e-6 && (h - 1.0).abs() < 1e-6);
    }
}
```

`crates/tools/src/lib.rs`: add `pub mod rectangle;` after `pub mod overlay;` and `pub use rectangle::RectangleTool;` after `pub use line::LineTool;`.

In `crates/app/src/editor.rs` tests, add before `leaving_the_window_hides_the_marker`:
```rust
    #[test]
    fn rectangle_tool_draws_one_undo_step_per_rectangle() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('r')));
        assert!(e.status().starts_with("Rectangle"), "{}", e.status());
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert_eq!(stats(&e), (4, 1));
        e.undo();
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn typed_size_finishes_a_rectangle() {
        let mut e = editor();
        e.key(EditorKey::Char('r'));
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        move_to(&mut e, DVec3::new(1.0, 1.0, 0.0));
        type_text(&mut e, "3,2");
        let model = e.doc().model();
        let (face, _) = model.faces().next().unwrap();
        assert!((model.area(face).unwrap() - 6.0).abs() < 1e-9);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools -p app`
Expected: compile errors in `rectangle.rs` (`RectangleTool` not found). Once it compiles, the editor tests fail until `r` selects the tool.

- [ ] **Step 3: Implement**

Prepend to `crates/tools/src/rectangle.rs`:
```rust
//! The Rectangle tool: click a corner, then the opposite corner (or type "width,height"). The
//! rectangle lies on the face under the first click, or on a horizontal plane.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::{Command, rectangle_axes, rectangle_corners};

use crate::inference::Inference;
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_lengths, toward_cursor};

#[derive(Clone, Debug, Default)]
pub struct RectangleTool {
    /// First corner and the normal of the plane the rectangle lies in.
    start: Option<(DVec3, DVec3)>,
    /// Opposite corner under the cursor, on that plane.
    hover: Option<DVec3>,
}

impl RectangleTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The command for the rectangle from the first corner to `opposite`, if it has an area.
    fn finish(&mut self, opposite: DVec3, out: &mut ToolOutput) {
        let Some((corner, normal)) = self.start else {
            return;
        };
        if rectangle_corners(corner, opposite, Some(normal)).is_ok() {
            out.commands.push(Command::AddRectangle {
                corner: corner.to_array(),
                opposite: opposite.to_array(),
                normal: Some(normal.to_array()),
            });
            self.start = None;
            self.hover = None;
        }
    }

    /// Width and height along the rectangle axes from the first corner to the cursor.
    fn size(&self) -> Option<(f64, f64)> {
        let ((corner, normal), hover) = (self.start?, self.hover?);
        let (u, v) = rectangle_axes(normal);
        Some(((hover - corner).dot(u), (hover - corner).dot(v)))
    }
}

/// The plane a rectangle started at `hit` lies in: the face under the cursor if the point is
/// on it, otherwise horizontal.
fn plane_normal(hit: &Inference, ctx: &ToolContext) -> DVec3 {
    hit.face
        .and_then(|id| ctx.model.face(id))
        .filter(|f| {
            let origin = ctx.model.position(f.outer[0]).unwrap();
            (hit.point - origin).dot(f.normal).abs() < LENGTH_TOL
        })
        .map_or(DVec3::Z, |f| f.normal)
}

/// The cursor's point on the plane through `origin` with `normal`: the inferred point if it is
/// on the plane, otherwise where the cursor ray crosses the plane.
fn on_plane(origin: DVec3, normal: DVec3, hit: &Inference, ctx: &ToolContext) -> Option<DVec3> {
    if (hit.point - origin).dot(normal).abs() < LENGTH_TOL {
        return Some(hit.point);
    }
    let ray = ctx.camera.ray(ctx.cursor);
    let denom = ray.dir.dot(normal);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = (origin - ray.origin).dot(normal) / denom;
    (t > 0.0).then(|| ray.origin + ray.dir * t)
}

impl Tool for RectangleTool {
    fn name(&self) -> &'static str {
        "Rectangle"
    }

    fn prompt(&self) -> &'static str {
        match self.start {
            None => "Click the first corner",
            Some(_) => "Click the opposite corner, or type width,height",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        let size = self
            .size()
            .map(|(w, h)| format!("{}, {}", format_length(w.abs()), format_length(h.abs())));
        ("Dimensions", size)
    }

    fn anchor(&self) -> Option<DVec3> {
        self.start.map(|(corner, _)| corner)
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => {
                if let Some((corner, normal)) = self.start {
                    self.hover = on_plane(corner, normal, &hit, ctx);
                }
            }
            ToolEvent::Click(hit) => match self.start {
                None => self.start = Some((hit.point, plane_normal(&hit, ctx))),
                Some((corner, normal)) => {
                    if let Some(p) = on_plane(corner, normal, &hit, ctx) {
                        self.finish(p, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => match (self.start, parse_lengths(&text).as_deref()) {
                (None, _) => out.message = Some("Click the first corner first".into()),
                (Some((corner, normal)), Some(&[w, h])) => {
                    // Typed sizes extend toward the cursor; a negative size flips the side.
                    let (u, v) = rectangle_axes(normal);
                    let (sw, sh) = self.size().unwrap_or((0.0, 0.0));
                    let (w, h) = (toward_cursor(sw, w), toward_cursor(sh, h));
                    self.finish(corner + u * w + v * h, &mut out);
                    if self.start.is_some() {
                        out.message = Some("Width and height must not be zero".into());
                    }
                }
                (Some(_), _) => out.message = Some(format!("Not width,height: {text}")),
            },
            ToolEvent::Cancel => self.on_external_change(),
        }
        if let (Some((corner, normal)), Some(hover)) = (self.start, self.hover)
            && let Ok(c) = rectangle_corners(corner, hover, Some(normal))
        {
            out.preview = (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect();
        }
        out
    }

    fn on_external_change(&mut self) {
        self.start = None;
        self.hover = None;
    }
}
```

`crates/tools/src/tool.rs`:
- import `use crate::rectangle::RectangleTool;`
- add `Rectangle` to `ToolKind`
- `make`: `ToolKind::Rectangle => Box::new(RectangleTool::new()),`
- `from_shortcut`: `'r' => Some(ToolKind::Rectangle),`

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 238 (rectangle 6, editor +2).

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/rectangle.rs crates/tools/src/tool.rs crates/tools/src/lib.rs crates/app/src/editor.rs
git commit -m "Add the Rectangle tool"
```

---

### Task 7: Push/Pull tool

**Files:**
- Create: `crates/tools/src/push_pull.rs`
- Modify: `crates/tools/src/inference.rs`, `crates/tools/src/tool.rs`, `crates/tools/src/lib.rs`, `crates/app/src/editor.rs` (tests)

**Interfaces:**
- Consumes: `face_under` (Task 4), `Tool` (Task 5), `Command::PushPull` (Task 3).
- Produces:
  - `closest_on_line_to_ray` in `inference.rs` becomes `pub(crate)`.
  - `PushPullTool` (`ToolKind::PushPull`, shortcut `p`):
    - **Hover.** Outlines the face under the cursor.
    - **First click.** Starts on the face under the cursor.
    - **Distance.** Measured from the face's plane along its normal. A snapped point (endpoint, midpoint, origin, on-edge) uses its height; otherwise the point on the normal line nearest the cursor ray is used.
    - **Second click.** Emits one `Command::PushPull` unless the distance is below `LENGTH_TOL`.
    - **Typed distance.** Follows the drag direction (`toward_cursor`).
    - **Preview.** The face outline, the offset outline and the connecting edges.
    - **Measurement.** `("Distance", "1.5 m")`.

- [ ] **Step 1: Write the failing tests**

Create `crates/tools/src/push_pull.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;
    use session::Camera;

    /// A 2x2 rectangle on the ground and a camera looking at it.
    struct Rig {
        tool: PushPullTool,
        model: Model,
        face: FaceId,
        camera: Camera,
        cursor: DVec2,
    }

    impl Rig {
        fn new() -> Self {
            let mut model = Model::new();
            let pts = [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ];
            model.add_polygon(&pts).unwrap();
            let face = model.faces().next().unwrap().0;
            Self {
                tool: PushPullTool::new(),
                model,
                face,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
            }
        }

        /// Puts the cursor over `p` and sends the event built from the inference there.
        fn at(&mut self, p: DVec3, make: fn(Inference) -> ToolEvent) -> ToolOutput {
            self.cursor = self.camera.project(p).unwrap();
            let hit = Inference {
                point: p,
                kind: SnapKind::OnFace,
                face: face_under(&self.model, &self.camera, self.cursor).map(|(f, _)| f),
            };
            self.send(make(hit))
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                camera: &self.camera,
                cursor: self.cursor,
            };
            self.tool.handle(event, &ctx)
        }
    }

    fn push_pull(face: FaceId, distance: f64) -> Command {
        Command::PushPull {
            face: EntityId::Face(face),
            distance,
        }
    }

    #[test]
    fn hovering_a_face_outlines_it() {
        let mut r = Rig::new();
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Move);
        assert_eq!(out.preview.len(), 4);
        let off = r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::Move);
        assert!(off.preview.is_empty());
    }

    #[test]
    fn click_drag_click_pulls_by_the_height_under_the_cursor() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        // The cursor over the point 1.5 m above the click lies on the normal line.
        let up = DVec3::new(1.0, 1.0, 1.5);
        let preview = r.at(up, ToolEvent::Move);
        assert_eq!(preview.preview.len(), 4 + 4 + 4);
        let (label, value) = r.tool.measurement();
        assert_eq!((label, value.as_deref()), ("Distance", Some("1.5 m")));
        let out = r.at(up, ToolEvent::Click);
        let [Command::PushPull { face, distance }] = out.commands.as_slice() else {
            panic!("expected one push_pull, got {:?}", out.commands);
        };
        assert_eq!(*face, EntityId::Face(r.face));
        assert!((distance - 1.5).abs() < 1e-6);
        assert!(r.tool.measurement().1.is_none(), "the drag ended");
    }

    #[test]
    fn snapped_points_set_the_distance_exactly() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        let corner = Inference {
            point: DVec3::new(5.0, 5.0, 0.75),
            kind: SnapKind::Endpoint,
            face: None,
        };
        let out = r.send(ToolEvent::Click(corner));
        assert_eq!(out.commands, vec![push_pull(r.face, 0.75)]);
    }

    #[test]
    fn typed_distance_follows_the_drag_direction() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.at(DVec3::new(1.0, 1.0, -0.5), ToolEvent::Move);
        let out = r.send(ToolEvent::Value("2".into()));
        assert_eq!(out.commands, vec![push_pull(r.face, -2.0)]);
    }

    #[test]
    fn typed_distance_without_dragging_goes_along_the_normal() {
        let mut r = Rig::new();
        for x in [0.3, 0.7, 1.1, 1.3, 1.9] {
            r.at(DVec3::new(x, 1.0, 0.0), ToolEvent::Click);
            r.at(DVec3::new(x, 1.0, 0.0), ToolEvent::Move);
            let out = r.send(ToolEvent::Value("1".into()));
            assert_eq!(
                out.commands,
                vec![push_pull(r.face, 1.0)],
                "clicked at x = {x}"
            );
        }
    }

    #[test]
    fn clicking_off_any_face_or_typing_first_does_nothing() {
        let mut r = Rig::new();
        assert!(
            r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::Click)
                .commands
                .is_empty()
        );
        assert!(r.tool.measurement().1.is_none());
        let out = r.send(ToolEvent::Value("1".into()));
        assert_eq!(out.message.as_deref(), Some("Click a face first"));
    }

    #[test]
    fn cancel_and_external_changes_end_the_drag() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.send(ToolEvent::Cancel);
        assert!(r.tool.measurement().1.is_none());
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.tool.on_external_change();
        assert!(r.tool.measurement().1.is_none());
    }
}
```

`crates/tools/src/lib.rs`:
```rust
//! Interactive tools and inference. Tools turn pointer input into `session::Command`s; they never
//! edit the model directly.

pub mod inference;
pub mod line;
pub mod overlay;
pub mod push_pull;
pub mod rectangle;
pub mod tool;
pub mod vcb;

pub use inference::{Axis, Inference, SnapKind, face_under, infer};
pub use line::LineTool;
pub use push_pull::PushPullTool;
pub use rectangle::RectangleTool;
pub use tool::{Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};
```

In `crates/app/src/editor.rs` tests, add a helper after `move_to`:
```rust
    fn click(e: &mut Editor) {
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }
```
and before `leaving_the_window_hides_the_marker`:
```rust
    /// Draws a 2 m square on the ground with the Rectangle tool and selects Push/Pull.
    fn square_then_push_pull(e: &mut Editor) {
        e.key(EditorKey::Char('r'));
        click_at(e, DVec3::new(0.0, 0.0, 0.0));
        click_at(e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('p'));
        assert!(e.status().starts_with("Push/Pull"), "{}", e.status());
    }

    #[test]
    fn push_pull_turns_a_rectangle_into_a_box() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(1.0, 1.0, 1.5));
        assert!(e.status().contains("Distance: 1.5 m"), "{}", e.status());
        click(&mut e);
        assert_eq!(stats(&e), (12, 6));
        e.undo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn typed_distance_finishes_a_push_pull() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(1.0, 1.0, 0.5));
        type_text(&mut e, "750mm");
        assert_eq!(stats(&e), (12, 6));
        let top = e
            .doc()
            .model()
            .vertices()
            .map(|(_, v)| v.pos.z)
            .fold(0.0, f64::max);
        assert!((top - 0.75).abs() < 1e-9);
    }

    #[test]
    fn failed_tool_commands_show_why() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        type_text(&mut e, "1");
        assert_eq!(stats(&e), (12, 6));
        // Pushing the top 3 m down would turn the 1 m sides inside out.
        click_at(&mut e, DVec3::new(1.0, 1.0, 1.0));
        type_text(&mut e, "-3");
        assert_eq!(stats(&e), (12, 6));
        assert!(e.status().contains("collapse"), "{}", e.status());
    }

    #[test]
    fn api_edit_ends_a_push_pull_in_progress() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert!(e.status().contains("Distance"), "{}", e.status());
        e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [6.0, 5.0, 0.0],
        })
        .unwrap();
        assert!(e.status().contains("Click a face"), "{}", e.status());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools -p app`
Expected: compile errors in `push_pull.rs` (`PushPullTool` not found).

- [ ] **Step 3: Implement**

Prepend to `crates/tools/src/push_pull.rs`:
```rust
//! The Push/Pull tool: click a face, move to set the distance along its normal, click again (or
//! type the distance).

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use kernel::{EntityId, FaceId, Model};
use session::Command;

use crate::inference::{Inference, SnapKind, closest_on_line_to_ray, face_under};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length, toward_cursor};

/// A push/pull in progress.
#[derive(Clone, Debug)]
struct Drag {
    face: FaceId,
    normal: DVec3,
    /// Where the face was clicked; the cursor is tracked along the normal line through it.
    start: DVec3,
    /// The face's loops (outer first), for the preview.
    loops: Vec<Vec<DVec3>>,
    distance: f64,
}

#[derive(Clone, Debug, Default)]
pub struct PushPullTool {
    drag: Option<Drag>,
    /// Outline of the face under the cursor while no push/pull is in progress.
    hover: Vec<Vec<DVec3>>,
}

impl PushPullTool {
    pub fn new() -> Self {
        Self::default()
    }

    fn finish(&mut self, distance: f64, out: &mut ToolOutput) {
        if let Some(d) = self.drag.take() {
            out.commands.push(Command::PushPull {
                face: EntityId::Face(d.face),
                distance,
            });
        }
    }
}

fn loops(model: &Model, id: FaceId) -> Vec<Vec<DVec3>> {
    let f = model.face(id).unwrap();
    std::iter::once(&f.outer)
        .chain(&f.holes)
        .map(|l| l.iter().filter_map(|v| model.position(*v)).collect())
        .collect()
}

fn outline(loops: &[Vec<DVec3>], offset: DVec3, out: &mut Vec<[DVec3; 2]>) {
    for l in loops {
        for i in 0..l.len() {
            out.push([l[i] + offset, l[(i + 1) % l.len()] + offset]);
        }
    }
}

/// Distance from the face's plane along its normal for the cursor: the height of a snapped
/// point, otherwise of the point on the normal line nearest the cursor ray.
fn measure(drag: &Drag, hit: &Inference, ctx: &ToolContext) -> f64 {
    let snapped = matches!(
        hit.kind,
        SnapKind::Endpoint | SnapKind::Midpoint | SnapKind::Origin | SnapKind::OnEdge
    );
    let p = if snapped {
        hit.point
    } else {
        let ray = ctx.camera.ray(ctx.cursor);
        closest_on_line_to_ray(drag.start, drag.normal, &ray, false)
    };
    (p - drag.loops[0][0]).dot(drag.normal)
}

impl Tool for PushPullTool {
    fn name(&self) -> &'static str {
        "Push/Pull"
    }

    fn prompt(&self) -> &'static str {
        match self.drag {
            None => "Click a face to push or pull",
            Some(_) => "Move to set the distance, then click or type it",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        (
            "Distance",
            self.drag.as_ref().map(|d| format_length(d.distance)),
        )
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => match self.drag.as_mut() {
                Some(d) => d.distance = measure(d, &hit, ctx),
                None => self.hover = hit.face.map(|f| loops(ctx.model, f)).unwrap_or_default(),
            },
            ToolEvent::Click(hit) => match self.drag.as_ref() {
                None => {
                    if let Some((face, start)) = face_under(ctx.model, ctx.camera, ctx.cursor) {
                        self.drag = Some(Drag {
                            face,
                            normal: ctx.model.face(face).unwrap().normal,
                            start,
                            loops: loops(ctx.model, face),
                            distance: 0.0,
                        });
                        self.hover.clear();
                    }
                }
                Some(d) => {
                    let distance = measure(d, &hit, ctx);
                    if distance.abs() >= LENGTH_TOL {
                        self.finish(distance, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => match (self.drag.as_ref(), parse_length(&text)) {
                (None, _) => out.message = Some("Click a face first".into()),
                (Some(_), None) => out.message = Some(format!("Not a length: {text}")),
                (Some(_), Some(v)) if v.abs() < LENGTH_TOL => {
                    out.message = Some("Distance must not be zero".into());
                }
                (Some(d), Some(v)) => {
                    // A typed distance goes the way the cursor is pulling; negative reverses it.
                    let v = toward_cursor(d.distance, v);
                    self.finish(v, &mut out);
                }
            },
            ToolEvent::Cancel => self.on_external_change(),
        }
        match &self.drag {
            Some(d) => {
                let offset = d.normal * d.distance;
                outline(&d.loops, DVec3::ZERO, &mut out.preview);
                if d.distance.abs() >= LENGTH_TOL {
                    outline(&d.loops, offset, &mut out.preview);
                    for p in d.loops.iter().flatten() {
                        out.preview.push([*p, *p + offset]);
                    }
                }
            }
            None => outline(&self.hover, DVec3::ZERO, &mut out.preview),
        }
        out
    }

    fn on_external_change(&mut self) {
        self.drag = None;
        self.hover.clear();
    }
}
```

The distance is measured from a face vertex's plane, not from the clicked point, which may be a hair off the plane. Typing right after the click therefore pulls along the normal, not backward. The `typed_distance_without_dragging_goes_along_the_normal` test pins this.

`crates/tools/src/inference.rs`: `fn closest_on_line_to_ray(` becomes `pub(crate) fn closest_on_line_to_ray(`.

`crates/tools/src/tool.rs` (final form):
```rust
//! What every interactive tool implements, and the tools the app can switch between.

use glam::{DVec2, DVec3};
use kernel::Model;
use session::{Camera, Command};

use crate::inference::Inference;
use crate::line::LineTool;
use crate::push_pull::PushPullTool;
use crate::rectangle::RectangleTool;

/// What a tool may look at while handling an event.
pub struct ToolContext<'a> {
    pub model: &'a Model,
    pub camera: &'a Camera,
    /// Cursor position in physical pixels.
    pub cursor: DVec2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolEvent {
    /// The cursor moved; carries the current inference.
    Move(Inference),
    /// The primary button was pressed at the current inference.
    Click(Inference),
    /// Text typed into the measurements box, submitted with Enter.
    Value(String),
    /// Escape: abandon the operation in progress.
    Cancel,
}

/// What a tool wants done after an event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolOutput {
    /// Commands to execute, in order (each is one undo step).
    pub commands: Vec<Command>,
    /// Segments to draw as a live preview.
    pub preview: Vec<[DVec3; 2]>,
    /// Feedback for the user, e.g. why typed text was ignored.
    pub message: Option<String>,
}

/// An interactive way to build commands. Tools never edit the model themselves.
pub trait Tool {
    fn name(&self) -> &'static str;
    /// What the user should do next.
    fn prompt(&self) -> &'static str;
    /// The measurements box label and the live value, e.g. ("Length", Some("2.5 m")).
    fn measurement(&self) -> (&'static str, Option<String>);
    /// The point axis inference starts from, if any.
    fn anchor(&self) -> Option<DVec3>;
    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput;
    /// The model changed outside the tool (agent edit, undo, redo): drop any operation in
    /// progress, since it may refer to geometry that no longer exists.
    fn on_external_change(&mut self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Line,
    Rectangle,
    PushPull,
}

impl ToolKind {
    pub fn make(self) -> Box<dyn Tool> {
        match self {
            ToolKind::Line => Box::new(LineTool::new()),
            ToolKind::Rectangle => Box::new(RectangleTool::new()),
            ToolKind::PushPull => Box::new(PushPullTool::new()),
        }
    }

    /// The keyboard shortcut that selects the tool.
    pub fn from_shortcut(c: char) -> Option<ToolKind> {
        match c.to_ascii_lowercase() {
            'l' => Some(ToolKind::Line),
            'r' => Some(ToolKind::Rectangle),
            'p' => Some(ToolKind::PushPull),
            _ => None,
        }
    }
}
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 249 (push_pull tool 7, editor +4).

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/push_pull.rs crates/tools/src/inference.rs crates/tools/src/tool.rs crates/tools/src/lib.rs crates/app/src/editor.rs
git commit -m "Add the Push/Pull tool"
```

---

### Task 8: Agent docs, API test, spec amendment, visual check

**Files:**
- Modify: `crates/api/tests/dispatch.rs`, `docs/agent-api.md`, `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`

**Interfaces:**
- Consumes: everything above. `describe` picks up the new commands and the `within` filter through the existing schemas, with no code change.

- [ ] **Step 1: Write the test**

In `crates/api/tests/dispatch.rs`, test `describe_lists_methods_and_schemas`, the variant list becomes:
```rust
    for variant in [
        "add_edge",
        "add_rectangle",
        "push_pull",
        "undo",
        "redo",
        "set_camera",
    ] {
```
and add before `render_view_returns_a_png_or_reports_no_gpu`:
```rust
#[test]
fn rectangle_and_push_pull_build_a_box() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 3, 0]}),
    );
    let face = r["result"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": face, "distance": 1.5}),
    );
    assert_eq!(r["result"]["revision"], 2);
    let stats = call(&mut h, "query", json!({"type": "stats"}));
    assert_eq!(
        (&stats["result"]["vertices"], &stats["result"]["faces"]),
        (&json!(8), &json!(6))
    );
    let r = call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": "e2", "distance": 1}),
    );
    assert_eq!(r["error"]["data"]["kind"], "invalid_input");
    assert_eq!(r["error"]["data"]["ids"], json!(["e2"]));
    let top = call(
        &mut h,
        "query",
        json!({"type": "entities", "kinds": ["face"], "within": {"min": [-1, -1, 1.4], "max": [3, 4, 1.6]}}),
    );
    assert_eq!(top["result"]["faces"].as_array().unwrap().len(), 1);
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p api --test dispatch`
Expected: PASS (12 tests). The behavior exists already; this pins it at the JSON-RPC level.

- [ ] **Step 3: Docs**

Replace `docs/agent-api.md` with:
```markdown
# Agent API

Every edit the window can make is also a command agents can send. The API is JSON-RPC 2.0 with one
JSON object per line (UTF-8, `\n`-terminated).

## Starting

| Command | Where the API is |
|---|---|
| `cargo run -p app` | Window, plus a Unix socket (path printed on stderr: `agent API listening on ...`) |
| `cargo run -p app -- --socket PATH` | Window, socket at `PATH` |
| `cargo run -p app -- --headless --stdio` | No window; requests on stdin, replies on stdout; exits when stdin closes |
| `cargo run -p app -- --headless` | No window; socket at the default path (printed on stderr) |
| `cargo run -p app -- --headless --socket PATH` | No window; socket at `PATH` |

The default socket is `sketchup-alt-<pid>.sock` in the temp directory, or in `~/.sketchup-alt/`
(created with mode 0700) when the temp directory path is too long for a Unix socket. The socket is
readable only by the current user. Try it by hand with `nc -U PATH`. The window removes the
socket file on a normal quit. The default name includes the process id, so a socket left behind
by a killed app is never reused; agents that start the app should pass `--socket PATH` (a fixed
path is replaced on the next start).

## Methods

Call `describe` first: it returns every method, notification, error code and the JSON Schemas of
all commands and results.

| Method | Params | Result |
|---|---|---|
| `execute` | a Command, e.g. `{"type": "add_edge", "start": [0,0,0], "end": [1,0,0]}` | `{changes, revision}` |
| `batch` | `{"commands": [...], "atomic": true}` | `{outcomes, revision}`; atomic = one undo step, all or nothing |
| `query` | a Query, see the table below | the QueryResult |
| `render_view` | `{"width": 640, "height": 400, "camera": {"eye": [...], "target": [...]}}` (camera optional) | `{width, height, png_base64}` |
| `subscribe` | none | `{revision}`, then `changed` notifications after every model change; a `resync` notification (`{revision}`) means changes were missed, so re-read with `{"type": "entities"}` (its `revision` tells you where you are) |
| `describe` | none | the API description |

Queries:

| Query | Result |
|---|---|
| `{"type": "entities"}` | every vertex, edge and face, plus `revision` |
| `{"type": "entities", "kinds": ["face"]}` | only the listed kinds (`vertex`, `edge`, `face`) |
| `{"type": "entities", "ids": ["v0", "e2"]}` | only the listed ids; unknown ids are absent, not an error |
| `{"type": "entities", "within": {"min": [0,0,0], "max": [2,2,1]}}` | only entities whose vertices all lie in the box (bounds included). `ids`, `kinds` and `within` combine (all must match); empty lists mean no filter |
| `{"type": "entity", "id": "f7"}` | one entity, or a not-found error |
| `{"type": "stats"}`, `{"type": "camera"}`, `{"type": "history"}` | counts and revision, camera, undo/redo depth |
| `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |

Unknown fields in commands, queries and params are rejected with `-32602` (a typo fails instead
of silently doing nothing).

Commands: `add_edge`, `add_rectangle`, `push_pull`, `undo`, `redo`, `set_camera`. Units are
meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m. Entity ids are strings: `v12`
(vertex), `e3` (edge), `f7` (face).

Modeling commands, each one undo step:

| Command | Effect |
|---|---|
| `{"type": "add_edge", "start": [x,y,z], "end": [x,y,z]}` | A straight edge. It merges with nearby vertices, splits edges it crosses, and closed coplanar loops become faces. |
| `{"type": "add_rectangle", "corner": [0,0,0], "opposite": [2,3,0]}` | A rectangle with sides along the axes. Without `normal` the corners must differ in exactly two coordinates. With `"normal": [nx,ny,nz]` it lies in the plane through `corner` with that normal; `opposite` must be in that plane. Sides then run horizontally (X on horizontal planes) and along `normal x horizontal`. |
| `{"type": "push_pull", "face": "f8", "distance": 1.5}` | Moves a face along its normal (negative: against it). A face on a flat sheet extrudes into a solid and stays as its base. A face on a solid extrudes a bump, or cuts a recess when pushed in. A face whose neighbors all run along the normal, like a box top, moves and stretches them. |

Find the face to push from the `created` ids of the command that made it, or with
`{"type": "entities", "kinds": ["face"]}`. Faces created by push/pull point out of the solid. A
push/pull that would collapse or invert the edges around a moved face fails with `invalid_input`.
When a face moves, its vertices are reported as `modified`; the edges and faces using them changed
shape too.

Behaviors worth knowing:

- Replies come before the `changed` notifications they cause, and your own edits are notified too.
- A request without an `id` runs but gets no reply. JSON-RPC batch arrays are rejected; use the
  `batch` method.
- `set_camera` returns empty changes and does not change the revision. `render_view`'s camera
  accepts `fov_y_deg` and never moves the document camera.
- A failed atomic batch is rolled back but still consumes entity ids, so later ids may skip numbers.
- An `id` that is not a string, number or null is an invalid request (`-32600`, reply id null).

## Example

```
-> {"jsonrpc":"2.0","id":1,"method":"execute","params":{"type":"add_edge","start":[0,0,0],"end":[2,0,0]}}
<- {"jsonrpc":"2.0","id":1,"result":{"changes":{"created":["v0","v1","e2"],"deleted":[],"modified":[]},"revision":1}}
-> {"jsonrpc":"2.0","id":2,"method":"query","params":{"type":"stats"}}
<- {"jsonrpc":"2.0","id":2,"result":{"type":"stats","vertices":2,"edges":1,"faces":0,"revision":1}}
```

Building a 2 x 3 x 1.5 m box on an empty model:

```
-> {"jsonrpc":"2.0","id":3,"method":"execute","params":{"type":"add_rectangle","corner":[0,0,0],"opposite":[2,3,0]}}
<- {"jsonrpc":"2.0","id":3,"result":{"changes":{"created":["v0","v1","v3","v5","e2","e4","e6","e7","f8"],"deleted":[],"modified":[]},"revision":1}}
-> {"jsonrpc":"2.0","id":4,"method":"execute","params":{"type":"push_pull","face":"f8","distance":1.5}}
<- {"jsonrpc":"2.0","id":4,"result":{"changes":{"created":["v9","v11","v13","v15","e10","e12","e14","e16","e17","e19","e21","e23","f18","f20","f22","f24","f25"],"deleted":[],"modified":["f8"]},"revision":2}}
```

`f8` is now the box's bottom, flipped to face down; `f25` is the top.

## Errors

Standard JSON-RPC codes (-32700 parse, -32600 invalid request, -32601 unknown method, -32602 bad
params) plus `-32000` command failed (`data` is `{kind, message, ids?}`), `-32001` batch failed
(`data` is `{index, error, completed, rolled_back}`) and `-32002` rendering unavailable.
```

The ids in the box example are what an empty model produces. Check them by running the two requests through `cargo run -p app -- --headless --stdio`.

In `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`:
- `Status:` line: `Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope)`.
- Milestones, replace the M3 and M4 lines with:
```markdown
- **M3**: Rectangle, Push/Pull, VCB (lengths and `width,height`; shown in the window title until the egui shell lands in M4).
- **M4**: egui shell (toolbar, measurements box, entity info); Select, Move/Copy (with VCB array syntax), Erase; full inference set; push/pull through to openings.
```

- [ ] **Step 4: Visual check**

Render a box with a recess headlessly and look at it:
```bash
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"execute","params":{"type":"add_rectangle","corner":[0,0,0],"opposite":[3,2,0]}}' \
 '{"jsonrpc":"2.0","id":2,"method":"execute","params":{"type":"push_pull","face":"f8","distance":1.5}}' \
 '{"jsonrpc":"2.0","id":3,"method":"execute","params":{"type":"add_rectangle","corner":[1,0.5,1.5],"opposite":[2,1.5,1.5]}}' \
 '{"jsonrpc":"2.0","id":4,"method":"execute","params":{"type":"push_pull","face":"f34","distance":-0.8}}' \
 '{"jsonrpc":"2.0","id":5,"method":"render_view","params":{"width":800,"height":500,"camera":{"eye":[-4,-6,5],"target":[1.5,1,0.5]}}}' \
 | cargo run -q -p app -- --headless --stdio | tail -1 \
 | python3 -c "import json,sys,base64; open('m3-box.png','wb').write(base64.b64decode(json.load(sys.stdin)['result']['png_base64']))"
```
Expected: `m3-box.png` shows a box with a square recess in its top. Every visible face, including the recess walls, is light gray (front side), not blue-gray. Report what you see, then delete `m3-box.png` (do not commit it).

- [ ] **Step 5: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 250.

- [ ] **Step 6: Commit**

```bash
git add crates/api/tests/dispatch.rs docs/agent-api.md docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
git commit -m "Document rectangle and push/pull for agents and amend the M3/M4 scope"
```

---

## Self-review notes

- **Spec coverage.**
  - M3 Rectangle: Tasks 3 and 6.
  - Push/Pull: Tasks 2, 3 and 7.
  - VCB: Tasks 4 to 7.
  - Agent reachability: Tasks 3 and 8 (`describe` schemas, API test, docs).
  - Section 2's tool contract: Task 5 (`Tool`, `ToolEvent::Value`).
  - The `on_external_change` hook and the shared edit path come from the M2.5 follow-ups.
- **Deferred:**
  - Array syntax, egui panels, push-through openings and seam healing (spec amended in Task 8).
  - Translucent face previews (wireframe previews instead).
  - Per-method params schemas for an MCP adapter.
- **Every intermediate state was built and tested.** The totals in each Verify step come from replaying the tasks in order.
