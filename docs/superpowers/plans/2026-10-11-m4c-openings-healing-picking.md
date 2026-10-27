# M4c Openings, Flush Healing, Intersection Snaps and BVH Picking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Push/pull cuts openings through walls and slabs and closes recesses pushed back flush. The cursor snaps where edges cross on screen. Picking stops scanning the whole model on every mouse move.

**Architecture:**
- **kernel:**
  - `push_pull` lets a push into a solid end exactly on a parallel face that holds the pushed face. The cap and the part of the back face it covers are removed, leaving an opening.
  - A stretch that shrinks the edges around a face exactly to nothing (a recess or bump pushed back flush) welds the face onto the geometry it reaches (new `weld.rs`):
    - collapsed edges and faces go;
    - seams between two coplanar faces facing the same way merge;
    - collinear vertices heal.
  - A bounding volume hierarchy over vertices, edges and face triangles (new `index.rs`). `Model::index()` builds it on first use after a change. Push/pull's obstruction ray uses it.
- **tools:**
  - Inference and edge/face picking take their candidates from the index: a ray query for faces, a cone around the cursor ray for vertices and edges.
  - A new `SnapKind::Intersection` snaps where two edges cross on screen, to the point on the nearer edge.
- **api/docs:** a JSON-RPC test for openings and flush. `agent-api.md` and the spec are updated.

**Tech Stack:** Rust 2024; no dependency changes.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md`:
- section 1: sticky geometry and the operations API;
- section 2: inference engine (point snaps include intersection; picking via BVH);
- the technology table: picking acceleration.

Task 6 amends the M4c milestone line and the picking row.

## Global Constraints

- Every new capability is a `session::Command` reachable through the API; tools only emit commands. (No new commands here: `push_pull` gains behavior.)
- Each modeling command is one undo step, and `ChangeSet::inverse` undoes it exactly. Kernel tests call `model.validate()` after edits.
- Command inputs and the points they produce must be finite and within +/-`MAX_COORDINATE` (1e6 m). Errors carry the offending ids.
- No dependency changes.
- Git: work on `dev`; stage only the files you changed; short plain commit messages, no emoji; `cargo fmt --all` before each commit; `cargo clippy --workspace --all-targets -- -D warnings` stays clean.

## Spec notes (rulings made in this plan)

- **Openings.**
  - **When a push cuts through:**
    - the pushed face is part of a solid and is pushed in;
    - the push ends exactly (within `LENGTH_TOL`) on one face parallel to it;
    - that face holds the moved outer loop: every corner and every side midpoint lies inside it or on its boundary.
  - **What it leaves:** every face in that plane inside the moved loops is removed, including one drawn there beforehand. The rest of the back face keeps its orientation. A pushed face with holes leaves columns standing in the opening.
  - **Still refused as `Obstructed`:** anything else in the way, including a cap that overlaps the back face's edge or an existing opening, and pushing past the back face.
  - **Obstruction test:** one ray from a point inside the face, as since M3.
- **Push/pull back to flush ("seam healing").**
  - **Scope:** only the stretch path (a face whose neighbors all run along its normal: a recess floor, a bump top, a notch floor).
  - **When it welds:** an edge leaving the face shrinks exactly to nothing.
  - **What happens then:**
    - each moved vertex that lands on the far end of such an edge merges into it;
    - collapsed edges and faces are removed, and faces drop the merged vertex;
    - every edge at a merged vertex between exactly two coplanar faces facing the same way is removed, and those faces merge (`rebuild_plane`);
    - vertices left between two collinear edges heal.
  - **Seams that stay:** an edge also used by an internal face (the wall between two boxes) keeps its seam, as in SketchUp.
  - **Still refused as `Collapse`:**
    - past flush;
    - landing on a vertex that is not the far end of a collapsing edge (M4b's no-merge ruling stands for unrelated geometry);
    - a face pushed flush onto another face (a box top onto its bottom).
  - General healing of extrusion seams beside coplanar faces is not done. Every such case found has an internal third face on the seam.
- **Index (BVH).**
  - **Where it lives:** in-house, in the kernel.
  - **How it is kept:** `Model::index()` builds it lazily; every recorded or applied op drops it. Model equality and cloning ignore it.
  - **What still scans:** box select (once per release) and sticky edge insertion (`draw_segment`). Copy speed and `MAX_COPIES` (100) are unchanged.
- **Picking through the index.**
  - **Faces:** ray hits use the face triangles. Ties at a shared triangle edge may resolve differently from M4b's polygon test, but always to a face that contains the point.
  - **Vertices and edges:**
    - they come from a cone around the cursor ray, with twice the spread between the rays through the cursor and a point `radius_px` beside it;
    - they are then tested exactly on screen as before;
    - candidates are sorted by id, so ties go the same way as before.
- **Intersection snaps.**
  - **Where:** two candidate edges that cross on screen within the pick radius, away from both ends. The point is on the edge nearer the camera.
  - **Ranking:** below endpoint, midpoint and origin; above on-edge.
  - **Marker:** orange. Push/pull measures from it like other point snaps.
  - **Real crossings** are already shared vertices through sticky splitting, so they snap as endpoints.
  - **Not offered:** edges piercing faces.

## File Structure

```
crates/kernel/src/push_pull.rs     openings (check_path, holds, hits_along); flush detection in move_face; obstruction ray via the index
crates/kernel/src/weld.rs          new: Model::weld (merge landed vertices, drop collapsed geometry, merge seams, heal)
crates/kernel/src/erase.rs         shared_plane and heal become pub(crate)
crates/kernel/src/index.rs         new: Index, Shape, BVH build and ray / cone queries
crates/kernel/src/model.rs         Model.index (OnceLock), Model::index(), dropped in apply_op
crates/kernel/src/lib.rs           mod weld, mod index, export Index and Shape
crates/kernel/tests/push_pull.rs   opening and flush tests
crates/kernel/tests/index.rs       new: index against brute force, cone coverage, updates
crates/tools/src/inference.rs      Near (cone candidates), index-based picking, SnapKind::Intersection, apparent_crossing
crates/tools/src/overlay.rs        INTERSECTION marker color
crates/tools/src/push_pull.rs      measure treats Intersection as a point snap
crates/api/tests/dispatch.rs       opening and flush over JSON-RPC
docs/agent-api.md                  push_pull row and notes
docs/superpowers/specs/...md       status, picking row, M4c line
```

Workspace test totals after each task: 357, 363, 367, 368, 370, 371 (351 before M4c).

---

### Task 1: Push through to openings

**Files:**
- Modify: `crates/kernel/src/push_pull.rs`
- Test: `crates/kernel/tests/push_pull.rs`

**Interfaces:**
- Consumes: `in_solid`, `polygon_in`, `interior_point`, `Polygon`, `remove_strays` (all in push_pull.rs); `segment_distance_2d` (geom.rs).
- Produces:
  - `Model::push_pull` cuts openings as ruled above.
  - Private helpers `check_path(id, &Face, distance) -> Result<bool, KernelError>`, `hits_along(id, distance) -> Vec<(FaceId, f64)>`, `holds(g, &Face, offset) -> bool`, and `Polygon::holds(p) -> bool`.
  - `extrude` gains a `through: bool` parameter.

- [ ] **Step 1: Write the failing tests**

Append to `crates/kernel/tests/push_pull.rs` (they use the file's existing helpers `rect`, `faces_where`, `push_pull_undoable`, `assert_closed`, `assert_outward`, `all_faces`, `side_uses`, `box_4x4x2`):
```rust
/// A wall 4 m long, 0.3 m thick and 3 m high, pulled up from the ground.
fn wall() -> Model {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 4.0, 0.3, 0.0);
    m.push_pull(base, 3.0).unwrap();
    m.take_changes();
    m
}

/// Draws the rectangle x0..x1, z0..z1 on the wall's front face (y = 0) and returns its face.
fn window(m: &mut Model, x0: f64, z0: f64, x1: f64, z1: f64) -> FaceId {
    m.add_polygon(&[
        p(x0, 0.0, z0),
        p(x1, 0.0, z0),
        p(x1, 0.0, z1),
        p(x0, 0.0, z1),
    ])
    .unwrap();
    m.validate().unwrap();
    let e = 1e-9;
    let mut found = faces_where(m, |q| {
        q.y.abs() < e && q.x > x0 - e && q.x < x1 + e && q.z > z0 - e && q.z < z1 + e
    });
    assert_eq!(found.len(), 1, "one face in the window");
    found.pop().unwrap()
}

/// The faces a ray along +y through (x, z) meets.
fn faces_on_line(m: &Model, x: f64, z: f64) -> usize {
    m.faces()
        .filter(|(id, _)| {
            m.triangulate(*id).unwrap().iter().any(|[a, b, c]| {
                let n = (*b - *a).cross(*c - *a);
                if n.y.abs() < 1e-12 {
                    return false;
                }
                let q = p(x, 0.0, z);
                let t = n.dot(*a - q) / n.y;
                let hit = q + DVec3::Y * t;
                let inside = |u: DVec3, v: DVec3| (v - u).cross(hit - u).dot(n) >= 0.0;
                inside(*a, *b) && inside(*b, *c) && inside(*c, *a)
            })
        })
        .count()
}

#[test]
fn pushing_a_window_to_the_back_of_a_wall_opens_it() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    assert_eq!(faces_on_line(&m, 1.5, 1.5), 2, "front and back before");
    push_pull_undoable(&mut m, win, -0.3);
    m.validate().unwrap();
    assert_eq!(
        faces_on_line(&m, 1.5, 1.5),
        0,
        "nothing left in the opening"
    );
    assert_eq!(faces_on_line(&m, 0.5, 1.5), 2, "the wall beside it stays");
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    // The front and back keep a hole each; four reveals line the opening.
    assert_eq!(m.faces().count(), 6 + 4);
}

#[test]
fn pushing_a_window_at_the_top_of_a_wall_cuts_a_slot() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 2.0, 2.0, 3.0);
    push_pull_undoable(&mut m, win, -0.3);
    m.validate().unwrap();
    assert_eq!(faces_on_line(&m, 1.5, 2.5), 0, "the slot is open");
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    let tops = faces_where(&m, |q| (q.z - 3.0).abs() < 1e-9).len();
    assert_eq!(tops, 2, "the slot splits the top in two");
    for (id, e) in m.edges() {
        assert!(
            side_uses(&m, e.v[0], e.v[1], &[]) > 0,
            "edge {id:?} borders no face"
        );
    }
}

#[test]
fn pushing_a_window_past_the_back_of_a_wall_is_refused() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    m.take_changes();
    let before = m.clone();
    assert_eq!(m.push_pull(win, -0.5), Err(KernelError::Obstructed));
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
}

#[test]
fn pushing_a_hole_through_a_box_from_the_top() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    push_pull_undoable(&mut m, sub, -2.0);
    m.validate().unwrap();
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    let caps = faces_where(&m, |q| {
        (q.x - 1.5).abs() < 0.5 + 1e-9 && (q.y - 1.5).abs() < 0.5 + 1e-9 && q.z.abs() < 1e-9
    });
    assert!(caps.is_empty(), "no face closes the bottom of the hole");
    assert_eq!(m.faces().count(), 6 + 4);
}

#[test]
fn a_window_drawn_on_both_sides_opens_both() {
    let mut m = wall();
    let back = [
        p(1.0, 0.3, 1.0),
        p(2.0, 0.3, 1.0),
        p(2.0, 0.3, 2.0),
        p(1.0, 0.3, 2.0),
    ];
    m.add_polygon(&back).unwrap();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, win, -0.3);
    m.validate().unwrap();
    assert_eq!(
        faces_on_line(&m, 1.5, 1.5),
        0,
        "nothing left in the opening"
    );
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn pushing_a_ring_through_a_box_leaves_a_column() {
    let (mut m, _) = box_4x4x2();
    rect(&mut m, 1.0, 1.0, 3.0, 3.0, 2.0);
    rect(&mut m, 1.5, 1.5, 2.5, 2.5, 2.0);
    let ring = m
        .faces()
        .find(|(_, f)| {
            f.holes.len() == 1 && f.outer.len() == 4 && {
                let q = m.position(f.outer[0]).unwrap();
                (q.z - 2.0).abs() < 1e-9 && q.x > 0.5 && q.x < 3.5 && q.y > 0.5 && q.y < 3.5
            }
        })
        .map(|(id, _)| id)
        .expect("ring face");
    push_pull_undoable(&mut m, ring, -2.0);
    m.validate().unwrap();
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    // The column keeps its top and bottom.
    assert_eq!(
        faces_where(&m, |q| (q.x - 2.0).abs() < 0.5 + 1e-9
            && (q.y - 2.0).abs() < 0.5 + 1e-9
            && q.z.abs() < 1e-9)
        .len(),
        1
    );
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kernel --test push_pull`
Expected: 5 of the 6 new tests fail with ``called `Result::unwrap()` on an `Err` value: Obstructed``. `pushing_a_window_past_the_back_of_a_wall_is_refused` passes: it pins behavior that must not change.

- [ ] **Step 3: Implement**

In `crates/kernel/src/push_pull.rs`:

1. The `geom` import becomes:
```rust
use crate::geom::{
    LENGTH_TOL, PLANE_ANGLE_TOL, Plane, point_in_polygon, project_on_segment, segment_distance_2d,
};
```

2. Replace `push_pull` (with its doc comment) by:
```rust
    /// Pushes or pulls face `id` by `distance` meters along its normal (negative: against it).
    ///
    /// If every face touching it runs along the push direction (the top of a box), the face
    /// moves and those faces stretch. Otherwise it is extruded: a side face for every side of
    /// every loop, plus a cap. The original face stays as the base when it lies in a flat sheet
    /// and is removed when it is part of a solid's surface. Extruded faces point out of the solid.
    /// A push into a solid that ends exactly on a parallel face holding the whole pushed face
    /// (the back of a wall) cuts an opening through to it.
    pub fn push_pull(&mut self, id: FaceId, distance: f64) -> Result<ChangeSet, KernelError> {
        let face = self.face(id).ok_or(KernelError::FaceNotFound)?.clone();
        if !distance.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if distance.abs() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        if self.stretches_around(id, &face) {
            self.move_face(id, &face, face.normal * distance)?;
        } else {
            let through = self.check_path(id, &face, distance)?;
            self.extrude(id, &face, distance, through);
        }
        Ok(self.take_changes())
    }
```

3. Replace `extrude` by:
```rust
    /// Extrudes face `id` by `distance`. With `through`, the push ends on the back of the solid:
    /// the cap and the part of the back face it covers are removed, leaving an opening.
    fn extrude(&mut self, id: FaceId, face: &Face, distance: f64, through: bool) {
        let n = face.normal;
        let offset = n * distance;
        let in_solid = self.in_solid(id);
        let loops: Vec<&Vec<VertexId>> = std::iter::once(&face.outer).chain(&face.holes).collect();
        let points: Vec<Vec<DVec3>> = loops
            .iter()
            .map(|l| l.iter().map(|v| self.pos(*v)).collect())
            .collect();
        // Every loop side (a, b) of the face with its positions; each sweeps the rectangle
        // a, b, b + offset, a + offset.
        let sides: Vec<(VertexId, VertexId, DVec3, DVec3)> = loops
            .iter()
            .zip(&points)
            .flat_map(|(l, ps)| {
                (0..l.len()).map(move |i| {
                    let j = (i + 1) % l.len();
                    (l[i], l[j], ps[i], ps[j])
                })
            })
            .collect();
        // The moved loops, to tell the cap apart from the rest of a back face in its plane.
        let cap_plane = Plane::through(points[0][0] + offset, n);
        let ring = |l: &Vec<DVec3>| -> Vec<DVec2> {
            l.iter().map(|p| cap_plane.project(*p + offset)).collect()
        };
        let cap = Polygon {
            plane: cap_plane,
            outer: ring(&points[0]),
            holes: points[1..].iter().map(ring).collect(),
        };
        let before: BTreeSet<FaceId> = self.faces().map(|(f, _)| f).collect();
        let vertices_before: BTreeSet<VertexId> = self.vertices().map(|(v, _)| v).collect();
        // A cut opens a piece of a side only where a face that existed before covered it (a
        // notch at a box edge). The faces in each side's plane are recorded before the
        // extrusion splits them.
        let covering: Vec<Vec<Polygon>> = sides
            .iter()
            .map(|&(_, _, pa, pb)| {
                let Some(sp) = Plane::from_points(pa, pb, pa + offset) else {
                    return Vec::new();
                };
                self.faces()
                    .filter(|(f, _)| *f != id)
                    .filter_map(|(_, g)| self.polygon_in(g, &sp))
                    .collect()
            })
            .collect();

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
        // face; that fill is removed so the hole stays open. Through a solid, the back face
        // already fills it.
        let hole_caps: Vec<BTreeSet<VertexId>> = points[1..]
            .iter()
            .filter(|_| !through)
            .filter_map(|l| l.iter().map(|p| self.find_vertex(*p + offset)).collect())
            .collect();
        if through {
            // The cap lands on the back face: every face there inside the cap opens.
            let inside: Vec<FaceId> = self
                .faces()
                .map(|(f, _)| f)
                .filter(|f| {
                    let g = self.face(*f).unwrap();
                    g.normal.cross(n).length() < PLANE_ANGLE_TOL
                        && cap_plane.contains(self.pos(g.outer[0]))
                        && self.interior_point(*f).is_some_and(|p| cap.contains(p))
                })
                .collect();
            for f in inside {
                self.remove_face(f);
            }
        }
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
            if in_cap && through {
                // The rest of the back face keeps its orientation.
                continue;
            }
            if in_cap {
                self.orient(f, cap_normal);
                continue;
            }
            // A side face lies inside a side's swept rectangle. Other new faces in a side's
            // plane (the rest of a wall split by the extrusion) keep their orientation.
            let Some(inside) = self.interior_point(f) else {
                continue;
            };
            let side = sides
                .iter()
                .position(|&(_, _, pa, pb)| in_swept(inside, pa, pb, offset));
            let Some(i) = side else {
                continue;
            };
            if cut && covering[i].iter().any(|poly| poly.contains(inside)) {
                // The cut reaches an existing face in this side's plane: leave it open.
                self.remove_face(f);
                continue;
            }
            let (_, _, pa, pb) = sides[i];
            let out = (pb - pa).cross(n).normalize();
            self.orient(f, if cut { -out } else { out });
        }
        if cut {
            self.remove_strays(&vertices_before, &sides, offset);
        }
        if !in_solid {
            self.orient(id, if distance > 0.0 { -n } else { n });
        }
    }
```

4. In `remove_strays`, the doc comment and the `segments` list become:
```rust
    /// After a cut, removes the edges that border no face and lie on a swept vertical segment
    /// (p to p + offset for a loop point p), on a loop side of the pushed face, or on one of its
    /// moved sides (the rim of an opening on the back face). Then removes the loop vertices,
    /// their offset copies and the vertices this push/pull created that no edge uses any more.
    /// User lines the extrusion split lie on none of these, so they stay.
    fn remove_strays(
```
and
```rust
        let segments: Vec<(DVec3, DVec3)> = sides
            .iter()
            .flat_map(|&(_, _, pa, pb)| [(pa, pa + offset), (pa, pb), (pa + offset, pb + offset)])
            .collect();
```

5. Replace `check_unobstructed` by these four functions:
```rust
    /// Fails with `Obstructed` if face `id`, moved `distance` along its normal, would meet another
    /// face: a ray from a point inside it hits one within that distance.
    fn check_unobstructed(&self, id: FaceId, distance: f64) -> Result<(), KernelError> {
        if self.hits_along(id, distance).is_empty() {
            Ok(())
        } else {
            Err(KernelError::Obstructed)
        }
    }

    /// Checks the path of face `id` extruded by `distance`, like `check_unobstructed`, except
    /// that a push into a solid may end exactly on one face parallel to it that holds the whole
    /// moved face (the back of a wall). Returns true in that case: the push cuts through.
    fn check_path(&self, id: FaceId, face: &Face, distance: f64) -> Result<bool, KernelError> {
        let hits = self.hits_along(id, distance);
        let [(g, t)] = hits[..] else {
            return if hits.is_empty() {
                Ok(false)
            } else {
                Err(KernelError::Obstructed)
            };
        };
        let ends_on_back = distance < 0.0
            && (t - distance.abs()).abs() < LENGTH_TOL
            && self.holds(g, face, face.normal * distance)
            && self.in_solid(id);
        if ends_on_back {
            Ok(true)
        } else {
            Err(KernelError::Obstructed)
        }
    }

    /// The faces a ray from a point inside face `id` meets beyond `LENGTH_TOL` and within
    /// `distance` along its normal, each with the distance to its nearest hit.
    fn hits_along(&self, id: FaceId, distance: f64) -> Vec<(FaceId, f64)> {
        let Some(tris) = self.triangulate(id) else {
            return Vec::new();
        };
        let Some([a, b, c]) = tris.first().copied() else {
            return Vec::new();
        };
        let from = (a + b + c) / 3.0;
        let dir = self.face(id).unwrap().normal * distance.signum();
        let reach = distance.abs() + LENGTH_TOL;
        self.faces()
            .filter(|(f, _)| *f != id)
            .filter_map(|(f, _)| {
                self.triangulate(f)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|t| ray_hits_triangle(from, dir, t))
                    .filter(|t| *t > LENGTH_TOL && *t <= reach)
                    .min_by(f64::total_cmp)
                    .map(|t| (f, t))
            })
            .collect()
    }

    /// True if face `g` is parallel to `face` and, in its plane, holds `face` moved by `offset`:
    /// every corner and side midpoint of the moved outer loop lies inside `g` or on its boundary.
    fn holds(&self, g: FaceId, face: &Face, offset: DVec3) -> bool {
        let gf = self.face(g).unwrap();
        let moved: Vec<DVec3> = face.outer.iter().map(|v| self.pos(*v) + offset).collect();
        let plane = Plane::through(self.pos(gf.outer[0]), gf.normal);
        let Some(poly) = self.polygon_in(gf, &plane) else {
            return false;
        };
        let parallel = gf.normal.cross(face.normal).length() < PLANE_ANGLE_TOL;
        parallel
            && (0..moved.len()).all(|i| {
                let (a, b) = (moved[i], moved[(i + 1) % moved.len()]);
                plane.contains(a) && poly.holds(a) && poly.holds((a + b) * 0.5)
            })
    }
```

6. Replace `impl Polygon` by:
```rust
impl Polygon {
    /// True if `p` (assumed in the plane) is inside the outer loop and outside every hole.
    fn contains(&self, p: DVec3) -> bool {
        let q = self.plane.project(p);
        point_in_polygon(q, &self.outer) && !self.holes.iter().any(|h| point_in_polygon(q, h))
    }

    /// True if `p` (assumed in the plane) is inside or on the boundary.
    fn holds(&self, p: DVec3) -> bool {
        let q = self.plane.project(p);
        let on = |l: &Vec<DVec2>| {
            (0..l.len()).any(|i| segment_distance_2d(q, l[i], l[(i + 1) % l.len()]) < LENGTH_TOL)
        };
        on(&self.outer) || self.holes.iter().any(on) || self.contains(p)
    }
}
```

Why each piece is there:
- **The cap polygon** is computed before anything is drawn, so the faces inside the moved loops can be told apart from the rest of the back face.
- **Hole caps are kept when cutting through:** the back face already fills a pushed face's hole, and that piece stays as the back of the column.
- **`remove_strays` also covers the moved sides:** the back face's old boundary along a slot cut from its edge borders nothing afterwards.

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 357.

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/push_pull.rs crates/kernel/tests/push_pull.rs
git commit -m "Push through to openings"
```

---

### Task 2: Push/pull back to flush welds and heals

**Files:**
- Create: `crates/kernel/src/weld.rs`
- Modify: `crates/kernel/src/push_pull.rs`, `crates/kernel/src/erase.rs`, `crates/kernel/src/lib.rs`
- Test: `crates/kernel/tests/push_pull.rs`

**Interfaces:**
- Consumes:
  - `loop_has_side` (edit.rs);
  - `rebuild_plane` (faces.rs);
  - `shared_plane` and `heal` (erase.rs), both made `pub(crate)` here;
  - `vector_area` (geom.rs).
- Produces: `Model::weld(&mut self, moved: &BTreeSet<VertexId>)` (`pub(crate)`). `move_face` allows an exact collapse and welds.

- [ ] **Step 1: Write the failing tests**

Append to `crates/kernel/tests/push_pull.rs`:
```rust
/// The model is exactly a plain box: 8 vertices, 12 edges and 6 faces pointing out.
fn assert_plain_box(m: &Model) {
    m.validate().unwrap();
    assert_eq!(counts(m), (8, 12, 6));
    assert!(
        m.faces()
            .all(|(_, f)| f.holes.is_empty() && f.outer.len() == 4)
    );
    assert_outward(m, &all_faces(m));
}

#[test]
fn pushing_a_recess_back_flush_closes_it() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, -1.0).unwrap();
    let floor = face_in(&m, 1.0, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, floor, 1.0);
    assert_plain_box(&m);
}

#[test]
fn pushing_a_bump_back_flush_removes_it() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, 1.0).unwrap();
    let top = face_in(&m, 3.0, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, top, -1.0);
    assert_plain_box(&m);
}

#[test]
fn pushing_notches_and_steps_back_flush_restores_the_box() {
    for (x1, y1) in [(1.0, 1.0), (4.0, 1.0), (2.0, 1.0)] {
        let (mut m, _) = box_4x4x2();
        rect(&mut m, 0.0, 0.0, x1, y1, 2.0);
        let sub = face_in(&m, 2.0, 0.0, 0.0, x1, y1);
        m.push_pull(sub, -1.0).unwrap();
        let floor = face_in(&m, 1.0, 0.0, 0.0, x1, y1);
        push_pull_undoable(&mut m, floor, 1.0);
        assert_plain_box(&m);
    }
}

#[test]
fn pushing_a_recess_past_flush_is_refused() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, -1.0).unwrap();
    let floor = face_in(&m, 1.0, 1.0, 1.0, 2.0, 2.0);
    m.take_changes();
    let before = m.clone();
    assert_eq!(m.push_pull(floor, 1.5), Err(KernelError::Collapse));
    assert_eq!(m, before);
}

#[test]
fn pulling_a_lower_box_level_with_its_neighbour_removes_the_step() {
    let mut m = Model::new();
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    m.push_pull(a, 2.0).unwrap();
    let b = rect(&mut m, 2.0, 0.0, 4.0, 2.0, 0.0);
    m.push_pull(b, 1.0).unwrap();
    let top = face_in(&m, 1.0, 2.0, 0.0, 4.0, 2.0);
    push_pull_undoable(&mut m, top, 1.0);
    m.validate().unwrap();
    let step = faces_where(&m, |q| (q.x - 2.0).abs() < 1e-9 && q.z > 1.0 - 1e-9);
    assert!(step.is_empty(), "no step wall is left");
    let internal = faces_where(&m, |q| (q.x - 2.0).abs() < 1e-9);
    assert_eq!(internal.len(), 1, "one internal wall");
    let solid: Vec<FaceId> = all_faces(&m)
        .into_iter()
        .filter(|f| !internal.contains(f))
        .collect();
    assert_outward(&m, &solid);
    assert_closed(&m, &internal);
}

#[test]
fn a_window_recess_pushed_back_flush_closes() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    m.push_pull(win, -0.1).unwrap();
    let floor = faces_where(&m, |q| (q.y - 0.1).abs() < 1e-9)[0];
    push_pull_undoable(&mut m, floor, 0.1);
    assert_plain_box(&m);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kernel --test push_pull`
Expected: 5 of the 6 new tests fail with ``called `Result::unwrap()` on an `Err` value: Collapse``. `pushing_a_recess_past_flush_is_refused` passes; it pins behavior.

- [ ] **Step 3: Implement**

`crates/kernel/src/erase.rs`: `fn shared_plane(` becomes `pub(crate) fn shared_plane(`, and `fn heal(` becomes `pub(crate) fn heal(`.

`crates/kernel/src/lib.rs`: add `mod weld;` after `mod transform;`.

Create `crates/kernel/src/weld.rs`:
```rust
//! Welding: after a push/pull brings a face flush with the geometry around it, the vertices
//! that met merge, collapsed edges and faces go, and the seams left in a flat surface heal.

use std::collections::{BTreeMap, BTreeSet};

use crate::edit::loop_has_side;
use crate::geom::{LENGTH_TOL, Plane, vector_area};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::{Face, Model};

impl Model {
    /// Merges every vertex of `moved` that now lies on another vertex into it. Edges that
    /// collapse are removed and the others re-joined; faces drop the merged vertex and are
    /// removed when nothing of them is left (the walls of a recess pushed back flush). Then
    /// every edge at a merged vertex between exactly two faces in one plane facing the same way
    /// is removed and those faces merge, and vertices left between two collinear edges heal.
    pub(crate) fn weld(&mut self, moved: &BTreeSet<VertexId>) {
        let mut target: BTreeMap<VertexId, VertexId> = BTreeMap::new();
        for &m in moved {
            let p = self.pos(m);
            let other = self
                .vertices()
                .find(|(v, vert)| !moved.contains(v) && vert.pos.distance(p) < LENGTH_TOL)
                .map(|(v, _)| v);
            if let Some(f) = other {
                target.insert(m, f);
            }
        }
        if target.is_empty() {
            return;
        }
        let to = |v: &VertexId| *target.get(v).unwrap_or(v);

        let users: Vec<(FaceId, Face)> = self
            .faces()
            .filter(|(_, f)| loops(f).flatten().any(|v| target.contains_key(v)))
            .map(|(id, f)| (id, f.clone()))
            .collect();
        for (id, f) in users {
            let fix = |l: &Vec<VertexId>| -> Vec<VertexId> {
                let mut out: Vec<VertexId> = l.iter().map(to).collect();
                out.dedup();
                while out.len() > 1 && out.first() == out.last() {
                    out.pop();
                }
                out
            };
            let outer = fix(&f.outer);
            let flat = |l: &Vec<VertexId>| {
                let points: Vec<_> = l.iter().map(|v| self.pos(*v)).collect();
                l.len() < 3 || vector_area(&points).length() < LENGTH_TOL * LENGTH_TOL
            };
            if flat(&outer) {
                self.remove_face(id);
                continue;
            }
            let holes = f.holes.iter().map(fix).filter(|h| !flat(h)).collect();
            self.replace_face(
                id,
                Face {
                    outer,
                    holes,
                    normal: f.normal,
                },
            );
        }

        let joined: Vec<(EdgeId, [VertexId; 2])> = self
            .edges()
            .filter(|(_, e)| e.v.iter().any(|v| target.contains_key(v)))
            .map(|(id, e)| (id, e.v))
            .collect();
        for (e, [a, b]) in joined {
            self.remove_edge(e);
            let (a, b) = (to(&a), to(&b));
            if a != b && self.edge_between(a, b).is_none() {
                self.insert_edge(a, b);
            }
        }
        for &m in target.keys() {
            self.remove_vertex(m);
        }

        let met: BTreeSet<VertexId> = target.values().copied().collect();
        let seams: Vec<(EdgeId, [VertexId; 2], Plane)> = self
            .edges()
            .filter(|(_, e)| e.v.iter().any(|v| met.contains(v)))
            .filter_map(|(id, e)| {
                let [a, b] = e.v;
                let users: Vec<FaceId> = self
                    .faces()
                    .filter(|(_, f)| loops(f).any(|l| loop_has_side(l, a, b)))
                    .map(|(f, _)| f)
                    .collect();
                let [f, g] = users[..] else {
                    return None;
                };
                let facing = self.face(f)?.normal.dot(self.face(g)?.normal) > 0.0;
                let plane = self.shared_plane(f, g).filter(|_| facing)?;
                Some((id, e.v, plane))
            })
            .collect();
        let mut touched = met;
        let mut planes: Vec<Plane> = Vec::new();
        for (e, ends, plane) in seams {
            self.remove_edge(e);
            touched.extend(ends);
            if !planes.iter().any(|p| p.same_as(&plane)) {
                planes.push(plane);
            }
        }
        for plane in planes {
            self.rebuild_plane(plane, &[]);
        }
        for &v in &touched {
            if self.vertex(v).is_some() && self.edges_at(v).is_empty() {
                self.remove_vertex(v);
            }
        }
        for v in touched {
            if self.vertex(v).is_some() {
                self.heal(v);
            }
        }
    }
}

fn loops(f: &Face) -> impl Iterator<Item = &Vec<VertexId>> {
    std::iter::once(&f.outer).chain(&f.holes)
}
```

`crates/kernel/src/push_pull.rs`, replace `move_face` by:
```rust
    /// Moves every vertex of face `id` by `offset`, after checking that no edge leaving the face
    /// flips, no vertex lands on another and the face passes through no other face. An edge
    /// leaving the face may shrink to nothing exactly (a recess pushed back flush): the face
    /// then lands on the geometry around it and is welded to it.
    fn move_face(&mut self, id: FaceId, face: &Face, offset: DVec3) -> Result<(), KernelError> {
        let moving = loop_vertices(face);
        // Moving vertices that land on the far end of an edge leaving the face.
        let mut meets: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        for (_, e) in self.edges() {
            let [a, b] = e.v;
            let (m, fixed) = match (moving.contains(&a), moving.contains(&b)) {
                (true, false) => (a, b),
                (false, true) => (b, a),
                _ => continue,
            };
            let before = self.pos(m) - self.pos(fixed);
            let after = before + offset;
            if after.length() < LENGTH_TOL {
                meets.insert((m, fixed));
            } else if after.dot(before) <= 0.0 {
                return Err(KernelError::Collapse);
            }
        }
        let flush = !meets.is_empty();
        // Every face that stretches must keep its fixed vertices on the side of the face they
        // started on; otherwise its loops fold over (a hole could end up outside its outer loop).
        let n = face.normal;
        let origin = self.pos(face.outer[0]);
        let d = offset.dot(n);
        for (_, f) in self.faces() {
            let verts = loop_vertices(f);
            if verts.is_disjoint(&moving) || verts.is_subset(&moving) {
                continue;
            }
            for v in verts.difference(&moving) {
                let h = (self.pos(*v) - origin).dot(n);
                let level = (h - d).abs() < LENGTH_TOL;
                if (level && !flush) || (!level && h * (h - d) <= 0.0) {
                    return Err(KernelError::Collapse);
                }
            }
        }
        for &m in &moving {
            let to = self.pos(m) + offset;
            let lands_on_other = self.vertices().any(|(v, vert)| {
                !moving.contains(&v)
                    && vert.pos.distance(to) < LENGTH_TOL
                    && !meets.contains(&(m, v))
            });
            if lands_on_other {
                return Err(KernelError::Collapse);
            }
        }
        // A face carried flush onto another face (a box top pushed down to its bottom) would
        // flatten the solid: the edges joining them collapse.
        self.check_unobstructed(id, offset.dot(n))
            .map_err(|e| if flush { KernelError::Collapse } else { e })?;
        for &m in &moving {
            let to = self.pos(m) + offset;
            self.move_vertex(m, to);
        }
        if flush {
            self.weld(&moving);
        }
        Ok(())
    }
```

The two changed checks:
- **Fixed vertices at the target level** (a recess rim) used to fail. They are now allowed when the move is flush, because they are where the face lands.
- **Obstruction:** a flush move that would also land on a face (a box top pushed to its bottom) reports `Collapse` rather than `Obstructed`, so `pushing_a_box_top_to_or_through_the_bottom_fails` keeps its expectation.

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 363.

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/weld.rs crates/kernel/src/push_pull.rs crates/kernel/src/erase.rs crates/kernel/src/lib.rs crates/kernel/tests/push_pull.rs
git commit -m "Weld faces pushed back flush"
```

---

### Task 3: Spatial index (BVH) in the kernel

**Files:**
- Create: `crates/kernel/src/index.rs`, `crates/kernel/tests/index.rs`
- Modify: `crates/kernel/src/model.rs`, `crates/kernel/src/lib.rs`, `crates/kernel/src/push_pull.rs`

**Interfaces:**
- Consumes: `Model::{vertices, edges, faces, triangulate, pos}`.
- Produces:
  - `kernel::Shape::{Point(VertexId, DVec3), Segment(EdgeId, DVec3, DVec3), Triangle(FaceId, [DVec3; 3])}`.
  - `kernel::Index` with:
    - `ray_hit(origin, dir, min_t, max_t) -> Option<(FaceId, f64)>`: the nearest triangle hit with `min_t < t <= max_t`;
    - `ray_hits(origin, dir, min_t, max_t) -> Vec<(FaceId, f64)>`: every face hit in that range, each at its nearest `t`, in face order;
    - `near_ray(origin, dir, r0, slope, visit: impl FnMut(&Shape))`: visits every vertex and edge that may lie within `r0 + slope * t` of the ray, plus some outside it.
  - `Model::index(&self) -> &Index`: built on first use, dropped by every op.
  - `push_pull`'s `hits_along` uses `ray_hits`. The private `ray_hits_triangle` moves from push_pull.rs to index.rs.

- [ ] **Step 1: Write the failing tests**

Create `crates/kernel/tests/index.rs`:
```rust
use glam::DVec3;
use kernel::{EntityId, FaceId, Model, Shape};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 5 x 5 grid of boxes of different heights, with a loose edge above each.
fn town() -> Model {
    let mut m = Model::new();
    for i in 0..5 {
        for j in 0..5 {
            let (x, y) = (i as f64 * 3.0, j as f64 * 3.0);
            let c = m
                .add_polygon(&[
                    p(x, y, 0.0),
                    p(x + 2.0, y, 0.0),
                    p(x + 2.0, y + 2.0, 0.0),
                    p(x, y + 2.0, 0.0),
                ])
                .unwrap();
            let base = c
                .summary()
                .created
                .into_iter()
                .find_map(|id| match id {
                    EntityId::Face(f) => Some(f),
                    _ => None,
                })
                .unwrap();
            m.push_pull(base, 1.0 + ((i * 7 + j * 3) % 5) as f64 * 0.5)
                .unwrap();
            m.add_edge(p(x, y, 6.0), p(x + 1.0, y + 1.5, 6.5)).unwrap();
        }
    }
    m.take_changes();
    m
}

/// Rays from an eye above one corner of the town towards points spread over it.
fn rays() -> Vec<(DVec3, DVec3)> {
    let eye = p(-6.1, -7.3, 9.7);
    let mut out = Vec::new();
    for i in 0..23 {
        for j in 0..19 {
            let target = p(
                i as f64 * 0.731 - 1.0,
                j as f64 * 0.853 - 1.0,
                (i + j) as f64 * 0.05,
            );
            out.push((eye, (target - eye).normalize()));
        }
    }
    out
}

fn hit_triangle(from: DVec3, dir: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let n = (b - a).cross(c - a);
    let denom = n.dot(dir);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = n.dot(a - from) / denom;
    let q = from + dir * t;
    let inside = |u: DVec3, v: DVec3| (v - u).cross(q - u).dot(n) >= 0.0;
    (inside(a, b) && inside(b, c) && inside(c, a)).then_some(t)
}

/// The nearest face hit with t > 0, by testing every triangle.
fn brute_hit(m: &Model, from: DVec3, dir: DVec3) -> Option<(FaceId, f64)> {
    let mut best: Option<(FaceId, f64)> = None;
    for (f, _) in m.faces() {
        for tri in m.triangulate(f).unwrap() {
            if let Some(t) = hit_triangle(from, dir, tri)
                && t > 0.0
                && best.is_none_or(|(_, bt)| t < bt)
            {
                best = Some((f, t));
            }
        }
    }
    best
}

#[test]
fn ray_hits_match_testing_every_face() {
    let m = town();
    let mut hits = 0;
    for (from, dir) in rays() {
        let want = brute_hit(&m, from, dir);
        let got = m.index().ray_hit(from, dir, 0.0, f64::INFINITY);
        assert_eq!(got.map(|(f, _)| f), want.map(|(f, _)| f), "ray {dir:?}");
        if let (Some((_, a)), Some((_, b))) = (got, want) {
            assert!((a - b).abs() < 1e-9);
            hits += 1;
        }
    }
    assert!(hits > 100, "most rays hit something ({hits})");
}

#[test]
fn ray_hits_list_every_face_within_reach() {
    let m = town();
    let (from, dir) = (p(1.0, 1.0, 10.0), DVec3::NEG_Z);
    let hits = m.index().ray_hits(from, dir, 0.0, 20.0);
    // The box top and its base below.
    assert_eq!(hits.len(), 2);
    let short = m.index().ray_hits(from, dir, 0.0, 9.5);
    assert_eq!(short.len(), 1, "the base is out of reach");
}

#[test]
fn near_ray_visits_everything_inside_the_cone() {
    let m = town();
    let (r0, slope) = (0.05, 0.02);
    for (from, dir) in rays() {
        let mut seen = Vec::new();
        m.index().near_ray(from, dir, r0, slope, |s| seen.push(*s));
        let within = |q: DVec3| {
            let t = (q - from).dot(dir);
            (q - from - dir * t).length() <= r0 + slope * t.max(0.0)
        };
        for (v, vert) in m.vertices() {
            if within(vert.pos) {
                assert!(
                    seen.contains(&Shape::Point(v, vert.pos)),
                    "vertex {v:?} missed"
                );
            }
        }
        for (e, edge) in m.edges() {
            let (a, b) = (
                m.position(edge.v[0]).unwrap(),
                m.position(edge.v[1]).unwrap(),
            );
            let near = (0..=20).any(|k| within(a + (b - a) * (k as f64 / 20.0)));
            if near {
                assert!(
                    seen.iter()
                        .any(|s| matches!(s, Shape::Segment(id, ..) if *id == e)),
                    "edge {e:?} missed"
                );
            }
        }
        assert!(
            seen.len() < m.vertices().count() + m.edges().count(),
            "the cone prunes"
        );
    }
}

#[test]
fn the_index_follows_edits_undo_and_clones() {
    let mut m = town();
    let (from, dir) = (p(20.0, 20.0, 10.0), DVec3::NEG_Z);
    assert_eq!(m.index().ray_hit(from, dir, 0.0, f64::INFINITY), None);
    let c = m
        .add_polygon(&[
            p(19.0, 19.0, 0.0),
            p(21.0, 19.0, 0.0),
            p(21.0, 21.0, 0.0),
            p(19.0, 21.0, 0.0),
        ])
        .unwrap();
    let hit = m.index().ray_hit(from, dir, 0.0, f64::INFINITY);
    assert_eq!(hit.map(|(_, t)| t), Some(10.0));
    let copy = m.clone();
    assert_eq!(copy, m);
    assert_eq!(copy.index().ray_hit(from, dir, 0.0, f64::INFINITY), hit);
    m.apply(&c.inverse());
    assert_eq!(m.index().ray_hit(from, dir, 0.0, f64::INFINITY), None);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kernel --test index`
Expected: compile errors (`kernel::Shape` and `Model::index` not found).

- [ ] **Step 3: Implement**

Create `crates/kernel/src/index.rs`:
```rust
//! A bounding volume hierarchy over the model's vertices, edges and face triangles, so ray
//! picks and cursor snaps visit only the geometry near a ray instead of the whole model.

use glam::DVec3;

use crate::geom::LENGTH_TOL;
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::Model;

/// A piece of geometry held by the index, with its position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Point(VertexId, DVec3),
    Segment(EdgeId, DVec3, DVec3),
    Triangle(FaceId, [DVec3; 3]),
}

impl Shape {
    /// Bounding box, padded by `LENGTH_TOL` so flat shapes have some thickness.
    fn bounds(&self) -> (DVec3, DVec3) {
        let (min, max) = match *self {
            Shape::Point(_, p) => (p, p),
            Shape::Segment(_, a, b) => (a.min(b), a.max(b)),
            Shape::Triangle(_, [a, b, c]) => (a.min(b).min(c), a.max(b).max(c)),
        };
        (
            min - DVec3::splat(LENGTH_TOL),
            max + DVec3::splat(LENGTH_TOL),
        )
    }
}

#[derive(Clone, Debug)]
enum Kids {
    /// Shapes `start..end`.
    Leaf(usize, usize),
    /// Node positions of the two halves.
    Inner(usize, usize),
}

#[derive(Clone, Debug)]
struct Node {
    min: DVec3,
    max: DVec3,
    kids: Kids,
}

/// Most shapes in a leaf.
const LEAF: usize = 4;

/// Spatial index over a model's geometry. Get it with [`Model::index`].
#[derive(Clone, Debug, Default)]
pub struct Index {
    nodes: Vec<Node>,
    shapes: Vec<Shape>,
}

impl Index {
    pub(crate) fn build(model: &Model) -> Index {
        let mut shapes: Vec<Shape> = model
            .vertices()
            .map(|(id, v)| Shape::Point(id, v.pos))
            .collect();
        for (id, e) in model.edges() {
            shapes.push(Shape::Segment(id, model.pos(e.v[0]), model.pos(e.v[1])));
        }
        for (id, _) in model.faces() {
            for t in model.triangulate(id).unwrap_or_default() {
                shapes.push(Shape::Triangle(id, t));
            }
        }
        let mut index = Index {
            nodes: Vec::new(),
            shapes,
        };
        if !index.shapes.is_empty() {
            index.split(0, index.shapes.len());
        }
        index
    }

    /// Adds the node over shapes `start..end` and its subtree; returns the node's position.
    fn split(&mut self, start: usize, end: usize) -> usize {
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        let mut cmin = min;
        let mut cmax = max;
        for s in &self.shapes[start..end] {
            let (a, b) = s.bounds();
            min = min.min(a);
            max = max.max(b);
            cmin = cmin.min((a + b) * 0.5);
            cmax = cmax.max((a + b) * 0.5);
        }
        let at = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            kids: Kids::Leaf(start, end),
        });
        if end - start <= LEAF {
            return at;
        }
        // Halve at the median centre along the axis where the centres spread most.
        let axis = (cmax - cmin).max_position();
        let centre = |s: &Shape| {
            let (a, b) = s.bounds();
            (a[axis] + b[axis]) * 0.5
        };
        let mid = (start + end) / 2;
        self.shapes[start..end]
            .select_nth_unstable_by(mid - start, |a, b| centre(a).total_cmp(&centre(b)));
        let left = self.split(start, mid);
        let right = self.split(mid, end);
        self.nodes[at].kids = Kids::Inner(left, right);
        at
    }

    /// The nearest face triangle that the ray `origin + t * dir` meets with
    /// `min_t < t <= max_t`: its face and `t`.
    pub fn ray_hit(
        &self,
        origin: DVec3,
        dir: DVec3,
        min_t: f64,
        max_t: f64,
    ) -> Option<(FaceId, f64)> {
        let mut best: Option<(FaceId, f64)> = None;
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            let Some(node) = self.nodes.get(i) else {
                break;
            };
            let reach = best.map_or(max_t, |(_, t)| t);
            if !ray_meets_box(origin, dir, node.min, node.max, reach) {
                continue;
            }
            match node.kids {
                Kids::Inner(l, r) => stack.extend([l, r]),
                Kids::Leaf(s, e) => {
                    for shape in &self.shapes[s..e] {
                        if let Shape::Triangle(f, tri) = *shape
                            && let Some(t) = ray_hits_triangle(origin, dir, tri)
                            && t > min_t
                            && t <= best.map_or(max_t, |(_, bt)| bt)
                            && best.is_none_or(|(_, bt)| t < bt)
                        {
                            best = Some((f, t));
                        }
                    }
                }
            }
        }
        best
    }

    /// Every face the ray `origin + t * dir` meets with `min_t < t <= max_t`, each with the
    /// `t` of its nearest hit, in face order.
    pub fn ray_hits(
        &self,
        origin: DVec3,
        dir: DVec3,
        min_t: f64,
        max_t: f64,
    ) -> Vec<(FaceId, f64)> {
        let mut hits: std::collections::BTreeMap<FaceId, f64> = Default::default();
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            let Some(node) = self.nodes.get(i) else {
                break;
            };
            if !ray_meets_box(origin, dir, node.min, node.max, max_t) {
                continue;
            }
            match node.kids {
                Kids::Inner(l, r) => stack.extend([l, r]),
                Kids::Leaf(s, e) => {
                    for shape in &self.shapes[s..e] {
                        if let Shape::Triangle(f, tri) = *shape
                            && let Some(t) = ray_hits_triangle(origin, dir, tri)
                            && t > min_t
                            && t <= max_t
                        {
                            let at = hits.entry(f).or_insert(t);
                            *at = at.min(t);
                        }
                    }
                }
            }
        }
        hits.into_iter().collect()
    }

    /// Calls `visit` for every vertex and edge that may lie within `r0 + slope * t` of the ray
    /// `origin + t * dir` (unit `dir`; `t` counted from 0): a cone around the ray, such as the
    /// pixels around a cursor. Some shapes outside it are visited too; callers test exactly.
    pub fn near_ray(
        &self,
        origin: DVec3,
        dir: DVec3,
        r0: f64,
        slope: f64,
        mut visit: impl FnMut(&Shape),
    ) {
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            let Some(node) = self.nodes.get(i) else {
                break;
            };
            let centre = (node.min + node.max) * 0.5;
            let radius = (node.max - node.min).length() * 0.5;
            let rel = centre - origin;
            let t = rel.dot(dir);
            let off = (rel - dir * t).length();
            if off > radius + r0 + slope * (t + radius).max(0.0) {
                continue;
            }
            match node.kids {
                Kids::Inner(l, r) => stack.extend([l, r]),
                Kids::Leaf(s, e) => self.shapes[s..e]
                    .iter()
                    .filter(|s| !matches!(s, Shape::Triangle(..)))
                    .for_each(&mut visit),
            }
        }
    }
}

/// True if the ray meets the box at some `t` in `[0, reach]`.
fn ray_meets_box(origin: DVec3, dir: DVec3, min: DVec3, max: DVec3, reach: f64) -> bool {
    let (mut lo, mut hi) = (0.0_f64, reach);
    for k in 0..3 {
        if dir[k].abs() < 1e-300 {
            if origin[k] < min[k] || origin[k] > max[k] {
                return false;
            }
            continue;
        }
        let (a, b) = ((min[k] - origin[k]) / dir[k], (max[k] - origin[k]) / dir[k]);
        lo = lo.max(a.min(b));
        hi = hi.min(a.max(b));
        if lo > hi {
            return false;
        }
    }
    true
}

/// Distance along unit `dir` from `from` to triangle `t`, if the ray meets it.
fn ray_hits_triangle(from: DVec3, dir: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let (e1, e2) = (b - a, c - a);
    let h = dir.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = from - a;
    let u = s.dot(h) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    let eps = 1e-12;
    (u >= -eps && v >= -eps && u + v <= 1.0 + eps).then(|| e2.dot(q) / det)
}
```

`crates/kernel/src/model.rs`:
- imports: add `use std::sync::OnceLock;` after the `std::collections` import, and `use crate::index::Index;` after the `crate::ids` import.
- `struct Model` gains a last field:
```rust
    /// Built on first use after a change; see `Model::index`.
    index: OnceLock<Index>,
```
  The hand-written `PartialEq` compares only vertices, edges and faces, so the index never affects equality.
- Add as the first method under `// ---- read access ----`:
```rust
    /// The spatial index over the current geometry, built on first use after a change.
    pub fn index(&self) -> &Index {
        self.index.get_or_init(|| Index::build(self))
    }
```
- `fn apply_op(&mut self, op: Op) {` starts with `self.index.take();`. Every recorded change and every applied ChangeSet (undo, redo) passes through it.

`crates/kernel/src/lib.rs`:
- add `mod index;` after `mod ids;`;
- add `pub use index::{Index, Shape};` after the `ids` re-export.

`crates/kernel/src/push_pull.rs`:
- replace `hits_along` by:
```rust
    /// The faces a ray from a point inside face `id` meets beyond `LENGTH_TOL` and within
    /// `distance` along its normal, each with the distance to its nearest hit.
    fn hits_along(&self, id: FaceId, distance: f64) -> Vec<(FaceId, f64)> {
        let Some(tris) = self.triangulate(id) else {
            return Vec::new();
        };
        let Some([a, b, c]) = tris.first().copied() else {
            return Vec::new();
        };
        let from = (a + b + c) / 3.0;
        let dir = self.face(id).unwrap().normal * distance.signum();
        let reach = distance.abs() + LENGTH_TOL;
        let mut hits = self.index().ray_hits(from, dir, LENGTH_TOL, reach);
        hits.retain(|(f, _)| *f != id);
        hits
    }
```
- delete the free function `ray_hits_triangle` at the end of the file. It now lives in index.rs.

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 367.

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/index.rs crates/kernel/tests/index.rs crates/kernel/src/model.rs crates/kernel/src/lib.rs crates/kernel/src/push_pull.rs
git commit -m "Index geometry for picking"
```

---

### Task 4: Pick through the index

**Files:**
- Modify: `crates/tools/src/inference.rs`

**Interfaces:**
- Consumes: `Model::index`, `Index::{ray_hit, near_ray}`, `Shape` (Task 3).
- Produces:
  - Private `Near { points: Vec<(VertexId, DVec3)>, segments: Vec<(EdgeId, DVec3, DVec3)> }` with `Near::around(model, camera, cursor, radius_px)`.
  - `snap_point`, `edge_under`, `visible` and `nearest_face_hit` use the index.
  - Public signatures are unchanged.

- [ ] **Step 1: Write the test**

In the `tests` module of `crates/tools/src/inference.rs`, add before `fn follows_an_axis_from_the_anchor`:
```rust
    #[test]
    fn snaps_find_their_edge_among_many() {
        let (c, mut m) = (cam(), Model::new());
        for i in -10..10 {
            for j in -10..10 {
                let a = DVec3::new(i as f64, j as f64, 0.0);
                m.add_edge(a, a + DVec3::new(0.4, 0.0, 0.0)).unwrap();
            }
        }
        let end = DVec3::new(2.4, 1.0, 0.0);
        let hit = infer(&m, &c, px(&c, end) + DVec2::new(2.0, -1.0), None, R);
        assert_eq!((hit.point, hit.kind), (end, SnapKind::Endpoint));
        let mid = infer(&m, &c, px(&c, DVec3::new(2.2, 1.0, 0.0)), None, R);
        assert_eq!(mid.kind, SnapKind::Midpoint);
        let e = edge_under(&m, &c, px(&c, DVec3::new(2.1, 1.0, 0.0)), R).unwrap();
        let ends = m.edge(e).unwrap().v.map(|v| m.position(v).unwrap());
        assert_eq!(ends, [DVec3::new(2.0, 1.0, 0.0), end]);
    }
```

- [ ] **Step 2: Run it**

Run: `cargo test -p tools snaps_find_their_edge_among_many`
Expected: PASS already. This task changes how candidates are found, not which one wins. The test, the existing inference tests and the editor and tool tests all pin the results across the change.

- [ ] **Step 3: Implement**

In `crates/tools/src/inference.rs`:

1. The kernel imports become:
```rust
use kernel::{EdgeId, FaceId, Model, Shape, VertexId};
```
(`kernel::geom::{Plane, point_in_polygon}` is no longer used.)

2. Replace `snap_point` by:
```rust
/// The visible point snap under the cursor, if any: endpoint, midpoint or origin (nearest on
/// screen), else the nearest point on an edge.
fn snap_point(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
) -> Option<(DVec3, SnapKind)> {
    let ray = camera.ray(cursor);
    let within = |p: DVec3| {
        camera
            .project(p)
            .map(|s| s.distance(cursor))
            .filter(|d| *d <= radius_px)
            .filter(|_| visible(model, camera.eye, p))
    };
    let near = Near::around(model, camera, cursor, radius_px);
    let mut best: Option<(f64, DVec3, SnapKind)> = None;
    let mut offer = |p: DVec3, kind: SnapKind| {
        if let Some(d) = within(p)
            && best.is_none_or(|(bd, _, _)| d < bd)
        {
            best = Some((d, p, kind));
        }
    };
    for &(_, p) in &near.points {
        offer(p, SnapKind::Endpoint);
    }
    for &(_, a, b) in &near.segments {
        offer((a + b) * 0.5, SnapKind::Midpoint);
    }
    offer(DVec3::ZERO, SnapKind::Origin);
    if let Some((_, p, kind)) = best {
        return Some((p, kind));
    }

    let mut on_edge: Option<(f64, DVec3)> = None;
    for &(_, a, b) in &near.segments {
        let p = closest_on_line_to_ray(a, b - a, &ray, true);
        if let Some(d) = within(p)
            && on_edge.is_none_or(|(bd, _)| d < bd)
        {
            on_edge = Some((d, p));
        }
    }
    on_edge.map(|(_, p)| (p, SnapKind::OnEdge))
}
```

3. Replace `edge_under` by:
```rust
/// The visible edge nearest the cursor on screen, if one lies within `radius_px`.
pub fn edge_under(model: &Model, camera: &Camera, cursor: DVec2, radius_px: f64) -> Option<EdgeId> {
    let ray = camera.ray(cursor);
    let mut best: Option<(f64, EdgeId)> = None;
    for (id, a, b) in Near::around(model, camera, cursor, radius_px).segments {
        let p = closest_on_line_to_ray(a, b - a, &ray, true);
        let Some(d) = camera.project(p).map(|s| s.distance(cursor)) else {
            continue;
        };
        if d <= radius_px && best.is_none_or(|(bd, _)| d < bd) && visible(model, camera.eye, p) {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id)
}
```

4. Replace `visible` by these three items:
```rust
/// The vertices and edges that may lie within `radius_px` of the cursor on screen, in id order
/// so that ties between equally near candidates always go the same way.
struct Near {
    points: Vec<(VertexId, DVec3)>,
    segments: Vec<(EdgeId, DVec3, DVec3)>,
}

impl Near {
    fn around(model: &Model, camera: &Camera, cursor: DVec2, radius_px: f64) -> Near {
        let ray = camera.ray(cursor);
        // The rays through the cursor and a point `radius_px` beside it spread apart by the
        // cone's slope; doubling it covers the diagonal of the pixel square around the cursor.
        let beside = camera.ray(cursor + DVec2::new(radius_px, 0.0));
        let slope = 2.0 * beside.dir.distance(ray.dir);
        let mut near = Near {
            points: Vec::new(),
            segments: Vec::new(),
        };
        model
            .index()
            .near_ray(ray.origin, ray.dir, 0.0, slope, |s| match *s {
                Shape::Point(v, p) => near.points.push((v, p)),
                Shape::Segment(e, a, b) => near.segments.push((e, a, b)),
                Shape::Triangle(..) => {}
            });
        near.points.sort_by_key(|(v, _)| *v);
        near.segments.sort_by_key(|(e, _, _)| *e);
        near
    }
}

/// True if no face lies between the eye and `p` (faces through `p` itself do not hide it).
fn visible(model: &Model, eye: DVec3, p: DVec3) -> bool {
    let to = p - eye;
    let dist = to.length();
    if dist < 1e-12 {
        return true;
    }
    model
        .index()
        .ray_hit(eye, to / dist, 0.0, dist * (1.0 - 1e-6))
        .is_none()
}
```

5. Replace `nearest_face_hit` by:
```rust
/// The nearest face the ray meets in front of its origin, and where.
fn nearest_face_hit(model: &Model, ray: &Ray) -> Option<(FaceId, DVec3)> {
    model
        .index()
        .ray_hit(ray.origin, ray.dir, 0.0, f64::INFINITY)
        .map(|(f, t)| (f, ray.origin + ray.dir * t))
}
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 368. Every existing tools and app test passes unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/inference.rs
git commit -m "Pick through the index"
```

---

### Task 5: Apparent-intersection snaps

**Files:**
- Modify: `crates/tools/src/inference.rs`, `crates/tools/src/overlay.rs`, `crates/tools/src/push_pull.rs`

**Interfaces:**
- Consumes: `Near` (Task 4), `closest_on_line_to_ray`, `Camera::{project, ray}`.
- Produces:
  - `SnapKind::Intersection` (between `Origin` and `OnEdge`).
  - Private `apparent_crossing(camera, (a, b), (c, d)) -> Option<DVec3>`.
  - `overlay::INTERSECTION` (orange).
  - The Push/Pull tool's `measure` treats `Intersection` as a point snap.

- [ ] **Step 1: Write the failing tests**

In the `tests` module of `crates/tools/src/inference.rs`, add before `fn snaps_find_their_edge_among_many`:
```rust
    #[test]
    fn snaps_where_edges_cross_on_screen_to_the_nearer_edge() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(0.0, 1.0, 0.0), DVec3::new(4.0, 1.0, 0.0))
            .unwrap();
        // A second edge hovering between the eye and the first, across it, crossing it on
        // screen away from both midpoints.
        let behind = DVec3::new(2.7, 1.0, 0.0);
        let front = behind + (c.eye - behind) * 0.3;
        let across = DVec3::Z.cross(c.eye - behind).normalize() + DVec3::Z;
        m.add_edge(front - across * 0.5, front + across * 1.5)
            .unwrap();
        let hit = infer(&m, &c, px(&c, behind) + DVec2::new(1.0, 2.0), None, R);
        assert_eq!(hit.kind, SnapKind::Intersection);
        assert!(hit.point.distance(front) < 1e-6, "{:?}", hit.point);
        // Next to an end, the endpoint wins.
        let end = infer(&m, &c, px(&c, DVec3::new(4.0, 1.0, 0.0)), None, R);
        assert_eq!(end.kind, SnapKind::Endpoint);
    }
```

In the `tests` module of `crates/tools/src/overlay.rs`, add before `fn endpoint_marker_is_a_square_of_constant_screen_size`:
```rust
    #[test]
    fn intersections_get_their_own_marker_color() {
        let hit = Inference {
            point: DVec3::new(1.0, 2.0, 0.0),
            kind: SnapKind::Intersection,
            face: None,
        };
        let segs = overlay(&[], Some(&hit), &cam(), 1.0);
        assert_eq!(segs.len(), 4);
        assert!(segs.iter().all(|s| s.color == INTERSECTION));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools`
Expected: compile errors (`SnapKind::Intersection` and `INTERSECTION` not found).

- [ ] **Step 3: Implement**

`crates/tools/src/inference.rs`:

1. In `enum SnapKind`, add between `Origin,` and `OnEdge,`:
```rust
    /// Where two edges cross on screen without meeting in space: the point on the nearer one.
    Intersection,
```

2. The `Ranking:` paragraph of `infer_with`'s doc comment becomes:
```rust
/// Ranking: endpoint, midpoint and origin (nearest on screen wins) > intersection > on-edge >
/// directions from the anchor (axes, parallel or perpendicular to the reference edge; nearest
/// wins) > on-face > ground > free. Points and edges hidden behind a face are skipped. With a
/// lock, the result is always on the locked line.
```

3. Replace `snap_point` by:
```rust
/// The visible point snap under the cursor, if any: endpoint, midpoint or origin (nearest on
/// screen), else the nearest apparent intersection of two edges, else the nearest point on an
/// edge.
fn snap_point(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    radius_px: f64,
) -> Option<(DVec3, SnapKind)> {
    let ray = camera.ray(cursor);
    let within = |p: DVec3| {
        camera
            .project(p)
            .map(|s| s.distance(cursor))
            .filter(|d| *d <= radius_px)
            .filter(|_| visible(model, camera.eye, p))
    };
    let near = Near::around(model, camera, cursor, radius_px);
    let mut best: Option<(f64, DVec3, SnapKind)> = None;
    let mut offer = |p: DVec3, kind: SnapKind| {
        if let Some(d) = within(p)
            && best.is_none_or(|(bd, _, _)| d < bd)
        {
            best = Some((d, p, kind));
        }
    };
    for &(_, p) in &near.points {
        offer(p, SnapKind::Endpoint);
    }
    for &(_, a, b) in &near.segments {
        offer((a + b) * 0.5, SnapKind::Midpoint);
    }
    offer(DVec3::ZERO, SnapKind::Origin);
    if let Some((_, p, kind)) = best {
        return Some((p, kind));
    }

    let mut crossing: Option<(f64, DVec3)> = None;
    for (i, &(_, a, b)) in near.segments.iter().enumerate() {
        for &(_, c, d) in &near.segments[i + 1..] {
            if let Some(p) = apparent_crossing(camera, (a, b), (c, d))
                && let Some(dist) = within(p)
                && crossing.is_none_or(|(bd, _)| dist < bd)
            {
                crossing = Some((dist, p));
            }
        }
    }
    if let Some((_, p)) = crossing {
        return Some((p, SnapKind::Intersection));
    }

    let mut on_edge: Option<(f64, DVec3)> = None;
    for &(_, a, b) in &near.segments {
        let p = closest_on_line_to_ray(a, b - a, &ray, true);
        if let Some(d) = within(p)
            && on_edge.is_none_or(|(bd, _)| d < bd)
        {
            on_edge = Some((d, p));
        }
    }
    on_edge.map(|(_, p)| (p, SnapKind::OnEdge))
}
```

4. Add before `pub fn directions(`:
```rust
/// Where segments a-b and c-d cross on screen away from their ends: the point under the
/// crossing on whichever segment is nearer the camera. Edges that meet in space share a vertex
/// (the kernel splits crossing edges), so their crossing is an endpoint, not this.
fn apparent_crossing(
    camera: &Camera,
    (a, b): (DVec3, DVec3),
    (c, d): (DVec3, DVec3),
) -> Option<DVec3> {
    let (sa, sb) = (camera.project(a)?, camera.project(b)?);
    let (sc, sd) = (camera.project(c)?, camera.project(d)?);
    let (r, q) = (sb - sa, sd - sc);
    let denom = r.perp_dot(q);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = (sc - sa).perp_dot(q) / denom;
    let u = (sc - sa).perp_dot(r) / denom;
    let inside = |k: f64| k > 1e-6 && k < 1.0 - 1e-6;
    if !inside(t) || !inside(u) {
        return None;
    }
    let ray = camera.ray(sa + r * t);
    let on_ab = closest_on_line_to_ray(a, b - a, &ray, true);
    let on_cd = closest_on_line_to_ray(c, d - c, &ray, true);
    Some(
        if on_ab.distance(ray.origin) <= on_cd.distance(ray.origin) {
            on_ab
        } else {
            on_cd
        },
    )
}
```

`crates/tools/src/overlay.rs`:
- add the constant after `MIDPOINT`:
```rust
pub const INTERSECTION: Color = [0.95, 0.55, 0.05, 1.0];
```
- in `overlay`'s marker `match hit.kind`, add after the `SnapKind::Midpoint` arm:
```rust
        SnapKind::Intersection => INTERSECTION,
```

`crates/tools/src/push_pull.rs`, in `measure`, the `snapped` match becomes:
```rust
    let snapped = matches!(
        hit.kind,
        SnapKind::Endpoint
            | SnapKind::Midpoint
            | SnapKind::Origin
            | SnapKind::Intersection
            | SnapKind::OnEdge
    );
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 370.

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/inference.rs crates/tools/src/overlay.rs crates/tools/src/push_pull.rs
git commit -m "Snap to apparent intersections"
```

---

### Task 6: API test, agent docs, spec amendment, visual check

**Files:**
- Modify: `crates/api/tests/dispatch.rs`, `docs/agent-api.md`, `docs/superpowers/specs/2026-10-10-tenon-design.md`

**Interfaces:**
- Consumes: the `push_pull` command with Tasks 1-2 behavior. No code changes.

- [ ] **Step 1: Write the test**

In `crates/api/tests/dispatch.rs`, add before `fn render_view_returns_a_png_or_reports_no_gpu`:
```rust
/// The id of the first face in a command result's `created` list.
fn created_face(r: &Value) -> Value {
    r["result"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone()
}

#[test]
fn push_pull_opens_a_window_and_closes_a_recess_over_json_rpc() {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 0.3, 0]}),
    );
    let base = created_face(&r);
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": base, "distance": 3}),
    );
    let faces = |h: &mut HeadlessHost| {
        call(h, "query", json!({"type": "stats"}))["result"]["faces"].clone()
    };
    assert_eq!(faces(&mut h), json!(6));

    // A window on the front (y = 0, facing -y) pushed back by the wall's thickness opens.
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [1, 0, 1], "opposite": [2, 0, 2], "normal": [0, -1, 0]}),
    );
    let window = created_face(&r);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": window, "distance": -0.3}),
    );
    assert!(r["error"].is_null(), "{r}");
    assert_eq!(faces(&mut h), json!(10));
    call(&mut h, "execute", json!({"type": "undo"}));

    // Pushed in part way it is a recess; pushed back flush, the wall is whole again.
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": window, "distance": -0.1}),
    );
    let floor = call(
        &mut h,
        "query",
        json!({"type": "entities", "kinds": ["face"], "within": {"min": [1, 0.05, 1], "max": [2, 0.15, 2]}}),
    );
    let floor = floor["result"]["faces"][0]["id"].clone();
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": floor, "distance": 0.1}),
    );
    assert_eq!(faces(&mut h), json!(6));
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p api --test dispatch`
Expected: PASS (15 tests). The behavior exists already; this pins it at the JSON-RPC level.

- [ ] **Step 3: Docs**

`docs/agent-api.md`:
- **The `push_pull` row's description becomes:**

  > Moves a face along its normal (negative: against it). A face on a flat sheet extrudes into a solid and stays as its base. A face on a solid extrudes a bump, or cuts a recess when pushed in; pushed in exactly as far as a parallel face that holds it (the back of a wall), it cuts an opening through. A face whose neighbors all run along the normal, like a box top, moves and stretches them; moved back flush with the surface around it (a recess or bump pushed back), it closes up and the seams heal.

- **In the notes below the table, replace** "So does one that would carry the face through other geometry (pushing a recess through the bottom of a box); push-through openings come in M4c." **with:**

  > So does one that would carry the face into or past other geometry, unless it ends exactly on a parallel face that holds the whole pushed face: that cuts an opening (a window through a wall, a hole through a slab).

  Keep the line breaks around it as the file has them (about 100 columns).

`docs/superpowers/specs/2026-10-10-tenon-design.md`:
- **`Status:` line:**
  ```
  Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope)
  ```
- **Technology table, picking row:**
  ```
  | Picking acceleration | BVH (in-house, in the kernel) |
  ```
- **Milestones, the M4c line:**
  ```
  - **M4c**: push/pull through to openings; push/pull back to flush welds the face and heals the seams; apparent-intersection snaps; BVH picking (a kernel index used by tools and push/pull).
  ```

- [ ] **Step 4: Visual check**

Render a wall with a window opening headlessly and look at it:
```bash
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"execute","params":{"type":"add_rectangle","corner":[0,0,0],"opposite":[4,0.3,0]}}' \
 '{"jsonrpc":"2.0","id":2,"method":"execute","params":{"type":"push_pull","face":"f8","distance":3}}' \
 '{"jsonrpc":"2.0","id":3,"method":"execute","params":{"type":"add_rectangle","corner":[1,0,1],"opposite":[2,0,2],"normal":[0,-1,0]}}' \
 '{"jsonrpc":"2.0","id":4,"method":"execute","params":{"type":"push_pull","face":"f34","distance":-0.3}}' \
 '{"jsonrpc":"2.0","id":5,"method":"render_view","params":{"width":800,"height":500,"camera":{"eye":[-3,-6,4],"target":[2,0.15,1.5]}}}' \
 | cargo run -q -p app -- --headless --stdio | tail -1 \
 | python3 -c "import json,sys,base64; open('m4c-wall.png','wb').write(base64.b64decode(json.load(sys.stdin)['result']['png_base64']))"
```
Expected: `m4c-wall.png` shows a wall with a square opening through which the grid behind is visible. The wall faces and the four reveals are light gray (front side), not blue-gray. Report what you see, then delete `m4c-wall.png` (do not commit it). `f8` and `f34` are the ids an empty model produces. If request 4 returns an error, run requests 1-3 and look up the window face with `{"type": "entities", "kinds": ["face"], "within": {"min": [1,0,1], "max": [2,0,2]}}`.

- [ ] **Step 5: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 371.

- [ ] **Step 6: Commit**

```bash
git add crates/api/tests/dispatch.rs docs/agent-api.md docs/superpowers/specs/2026-10-10-tenon-design.md
git commit -m "Document openings and flush push/pull"
```

---

## Self-review notes

- **Spec coverage (M4c line):**
  - push/pull through to openings: Task 1.
  - seam healing: Task 2 (flush welding, with the scope ruling above).
  - apparent-intersection snaps: Task 5.
  - BVH picking: Tasks 3 and 4.
  - agent reachability: Task 6. There is no new command; `push_pull` is documented and tested over JSON-RPC.
- **Deferred, unchanged from the M4b follow-ups:**
  - sticky-geometry and copy speed (`draw_segment` still scans), with `MAX_COPIES` 100;
  - direction locks in the Rectangle tool;
  - overlapping coplanar faces after moves;
  - edge-through-face intersection snaps;
  - obstruction judged by one ray.
- **Every intermediate state was built and tested.** The totals in each Verify step come from replaying the tasks in order.
