# M4b Move/Copy and Inference Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Users and agents can move and copy geometry, including array copies, and the cursor snaps parallel or perpendicular to edges and can be locked to a direction.

**Architecture:**
- **kernel:**
  - `move_vertices` moves a vertex set. Partly moved faces stretch and may tilt, but a move that would bend or squash a face is refused.
  - `copy` redraws edges and face loops at k times the offset through the sticky edge machinery, then restores each copied face's orientation.
- **session:** `move` (accepts vertex, edge and face ids) and `copy` (`count` for arrays) commands.
- **tools:**
  - The tool context gains the selection and a copy-toggle event. The measurements box parses `5x` and `/5`.
  - A Move tool: two clicks or a typed distance; Ctrl toggles copy; typed arrays replace the last copy as one undo step.
  - Inference gains `Guides`: an anchor, a reference edge direction (parallel and perpendicular snaps) and a direction lock.
- **app:**
  - The editor learns the reference from the edge under the cursor.
  - Shift locks the current direction and the arrow keys lock axes; the lock shows in the status bar.
  - Copies made in the window select themselves.
  - A release is forwarded only when its press reached the tool.

**Tech Stack:** Rust 2024; no dependency changes.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md`: section 1 (move/copy operations), section 2 (inference engine, VCB array syntax, Move/Copy tool), section 5 (agent control). Task 8 amends milestones M4b and M4c.

## Global Constraints

- Every new capability is a `session::Command` reachable through the API; tools only emit commands.
- Each modeling command is one undo step, and `ChangeSet::inverse` undoes it exactly. Kernel tests call `model.validate()` after edits.
- Command inputs and the points they produce must be finite and within +/-`MAX_COORDINATE` (1e6 m). Errors carry the offending ids.
- No dependency changes.
- Git: work on `dev`; stage only the files you changed; short plain commit messages, no emoji; `cargo fmt --all` before each commit; `cargo clippy --workspace --all-targets -- -D warnings` stays clean.

## Spec notes (rulings made in this plan)

- **Move.**
  - **What moves:** the union of the vertices of the given ids. Agents may pass vertex ids; the UI moves edges and faces.
  - **Partly moved faces:** a face with only some vertices moved must stay flat. It may tilt; its normal is recomputed and must keep its sign. A move that would bend it is `invalid_input` naming that face.
  - **Refused as `invalid_input`:** collapsing an edge, turning a face over, or landing a vertex on a fixed one.
  - **No merging:** moved geometry does not merge with what it ends up touching.
- **Copy.**
  - Copy k is shifted by `k * offset`, for k = 1..=count, with `count` from 1 to 1000.
  - Copies go through the sticky edge code, so they merge with what they touch, and closed loops become faces.
  - A copied face keeps its original's orientation.
  - Vertex ids are not copyable.
- **Move tool (M).**
  - **First click:** the base point. It takes the selection, or the edge or face under the click when nothing is selected.
  - **Second click:** the destination. Typing a length moves that far toward the cursor.
  - **Ctrl tap:** toggles copy. Option is the orbit modifier, so Ctrl is used on every platform.
  - **Arrays:** after a copy, `5x` (also `x5`, `*5`) makes 5 copies at that spacing, and `/5` puts 5 copies evenly up to the copy. Both replace the single copy: the tool sends Undo plus Copy, and the result is still one undo step.
  - **Selection:** copies made in the window become the selection, as in SketchUp. Agent copies leave the selection alone.
- **Inference.**
  - **Reference edge:** the last edge the cursor rested on.
  - **Parallel and perpendicular snaps:** offered from the anchor along the reference, and along two perpendiculars (one horizontal, one in the reference's vertical plane), unless a candidate lies along an axis.
  - **Ranking:** directions (axes and these) rank between on-edge and on-face, and the nearest on screen wins. They draw in magenta.
  - **Shift lock:** pressing Shift during an operation locks the direction under the cursor (axis, parallel or perpendicular); releasing it unlocks.
  - **Arrow locks:** Right, Left and Up lock red, green and blue; pressing the same key again unlocks, and Down unlocks. They work only during an operation.
  - **While locked:** point snaps project onto the locked line.
  - **When locks end:** when the tool's operation ends (no anchor) or the window loses focus (Shift lock only).
- **Not in M4b:** apparent-intersection snaps and BVH picking. Crossings in 3D are already vertices, through sticky splitting, so endpoint snaps cover them. Task 8 moves both to M4c.
- **M4a leftovers fixed here:**
  - the stray doc comment above `SelectTool`;
  - the selection is now in `ToolContext`;
  - releases whose press never reached the tool are dropped.

## File Structure

```
crates/kernel/src/transform.rs     new: Model::move_vertices, Model::copy
crates/kernel/src/edit.rs          KernelError::{Bend(FaceId), Squash}
crates/kernel/src/push_pull.rs     orient becomes pub(crate)
crates/kernel/src/lib.rs           mod transform
crates/kernel/tests/transform.rs   new
crates/session/src/command.rs      Command::{Move, Copy}
crates/session/src/document.rs     move/copy changes, vertex_ids, check_moved, kernel_error, MAX_COPIES
crates/session/src/lib.rs          export MAX_COPIES
crates/session/tests/transform.rs  new
crates/tools/src/tool.rs           ToolContext.selection, ToolEvent::ToggleCopy, ToolKind::Move
crates/tools/src/vcb.rs            Array, parse_array
crates/tools/src/move_tool.rs      new: Move tool
crates/tools/src/inference.rs      Guides, Lock, infer_with, snap_point, directions, SnapKind::{Parallel, Perpendicular}
crates/tools/src/overlay.rs        magenta for parallel/perpendicular
crates/tools/src/{line,rectangle,push_pull,select,erase}.rs  ToggleCopy arms, test contexts
crates/tools/src/lib.rs            modules and exports
crates/app/src/editor.rs           release gating, copy toggle, copy selects, reference, locks, Status.lock
crates/app/src/ui.rs               show the lock
crates/app/src/app.rs              Ctrl tap, Shift lock result, arrow keys
crates/api/tests/dispatch.rs       move/copy over JSON-RPC
docs/agent-api.md                  move, copy
docs/superpowers/specs/...md       M4b/M4c scope
```

Workspace test totals after each task: 317, 322, 324, 329, 331, 335, 338, 339 (308 before M4b).

---

### Task 1: Kernel move and copy

**Files:**
- Create: `crates/kernel/src/transform.rs`, `crates/kernel/tests/transform.rs`
- Modify: `crates/kernel/src/edit.rs`, `crates/kernel/src/push_pull.rs`, `crates/kernel/src/lib.rs`

**Interfaces:**
- Consumes: `draw_segment`, `find_vertex`, `move_vertex`, `replace_face`, and `orient` (push_pull.rs, made `pub(crate)`).
- Produces:
  - `Model::move_vertices(&mut self, vertices: &BTreeSet<VertexId>, offset: DVec3) -> Result<ChangeSet, KernelError>`.
  - `Model::copy(&mut self, edges: &[EdgeId], faces: &[FaceId], offset: DVec3, count: u32) -> Result<ChangeSet, KernelError>`.
  - `KernelError::Bend(FaceId)`, `KernelError::Squash`. `ZeroDistance` and `NotFinite` apply to `offset`.
  - Both check everything before they mutate, so an error changes nothing.

- [ ] **Step 1: Write the failing tests**

Create `crates/kernel/tests/transform.rs`:
```rust
use std::collections::BTreeSet;

use glam::DVec3;
use kernel::{EntityId, FaceId, KernelError, Model, VertexId};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn square(m: &mut Model, x0: f64, y0: f64, size: f64) -> FaceId {
    let c = m
        .add_polygon(&[
            p(x0, y0, 0.0),
            p(x0 + size, y0, 0.0),
            p(x0 + size, y0 + size, 0.0),
            p(x0, y0 + size, 0.0),
        ])
        .unwrap();
    c.summary()
        .created
        .iter()
        .find_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .unwrap()
}

/// A 2 m cube on the ground, built with push/pull.
fn cube() -> Model {
    let mut m = Model::new();
    let base = square(&mut m, 0.0, 0.0, 2.0);
    m.push_pull(base, 2.0).unwrap();
    m
}

fn vertices_where(m: &Model, f: impl Fn(DVec3) -> bool) -> BTreeSet<VertexId> {
    m.vertices()
        .filter(|(_, v)| f(v.pos))
        .map(|(id, _)| id)
        .collect()
}

fn face_with_normal(m: &Model, n: DVec3) -> FaceId {
    m.faces()
        .find(|(_, f)| f.normal.distance(n) < 1e-9)
        .map(|(id, _)| id)
        .unwrap()
}

fn counts(m: &Model) -> (usize, usize, usize) {
    (m.vertices().count(), m.edges().count(), m.faces().count())
}

#[test]
fn moving_every_vertex_moves_a_solid_rigidly() {
    let mut m = cube();
    let before: Vec<DVec3> = m.faces().map(|(_, f)| f.normal).collect();
    let all = vertices_where(&m, |_| true);
    let c = m.move_vertices(&all, p(1.0, 2.0, 0.5)).unwrap();
    m.validate().unwrap();
    let after: Vec<DVec3> = m.faces().map(|(_, f)| f.normal).collect();
    assert_eq!(before, after);
    assert!(m.vertices().any(|(_, v)| v.pos == p(3.0, 4.0, 2.5)));
    let s = c.summary();
    assert_eq!(s.modified.len(), 8);
    assert!(s.created.is_empty() && s.deleted.is_empty());
}

#[test]
fn moving_a_top_edge_out_tilts_the_face_below_it() {
    let mut m = cube();
    let front = face_with_normal(&m, -DVec3::Y);
    let side = face_with_normal(&m, DVec3::X);
    let top_front = vertices_where(&m, |q| q.y == 0.0 && q.z == 2.0);
    assert_eq!(top_front.len(), 2);
    m.move_vertices(&top_front, p(0.0, -1.0, 0.0)).unwrap();
    m.validate().unwrap();
    let n = m.face(front).unwrap().normal;
    assert!(n.y < -0.5 && n.z < -0.1, "front face leans out: {n}");
    assert_eq!(
        m.face(side).unwrap().normal,
        DVec3::X,
        "side faces stay in their planes"
    );
    let top = face_with_normal(&m, DVec3::Z);
    assert!((m.area(top).unwrap() - 6.0).abs() < 1e-9);
}

#[test]
fn moving_a_corner_out_of_its_plane_is_refused() {
    let mut m = Model::new();
    let f = square(&mut m, 0.0, 0.0, 2.0);
    let before = m.clone();
    let corner = vertices_where(&m, |q| q == p(0.0, 0.0, 0.0));
    assert_eq!(
        m.move_vertices(&corner, p(0.0, 0.0, 1.0)),
        Err(KernelError::Bend(f))
    );
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
}

#[test]
fn moves_that_collapse_or_overlap_are_refused() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    let before = m.clone();
    let corner = vertices_where(&m, |q| q == p(0.0, 0.0, 0.0));
    // Onto the neighbouring corner: the edge between them collapses.
    assert_eq!(
        m.move_vertices(&corner, p(2.0, 0.0, 0.0)),
        Err(KernelError::Squash)
    );
    // Across the square: the face turns inside out.
    assert_eq!(
        m.move_vertices(&corner, p(3.0, 3.0, 0.0)),
        Err(KernelError::Squash)
    );
    assert_eq!(
        m.move_vertices(&corner, p(0.0, 0.0, 0.0)),
        Err(KernelError::ZeroDistance)
    );
    assert_eq!(m, before);
}

#[test]
fn move_undoes_and_redoes_exactly() {
    let mut m = cube();
    let before = m.clone();
    let top_front = vertices_where(&m, |q| q.y == 0.0 && q.z == 2.0);
    let c = m.move_vertices(&top_front, p(0.0, -1.0, 0.0)).unwrap();
    let after = m.clone();
    m.apply(&c.inverse());
    assert_eq!(m, before);
    m.apply(&c);
    assert_eq!(m, after);
}

#[test]
fn copying_a_face_keeps_its_orientation() {
    let mut m = cube();
    let bottom = face_with_normal(&m, -DVec3::Z);
    m.copy(&[], &[bottom], p(5.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (12, 16, 7));
    let copies: Vec<_> = m
        .faces()
        .filter(|(_, f)| f.outer.iter().all(|v| m.position(*v).unwrap().x >= 5.0))
        .collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].1.normal, -DVec3::Z);
}

#[test]
fn array_copies_make_count_copies() {
    let mut m = Model::new();
    let f = square(&mut m, 0.0, 0.0, 1.0);
    let before = m.clone();
    let c = m.copy(&[], &[f], p(2.0, 0.0, 0.0), 3).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (16, 16, 4));
    assert!(m.vertices().any(|(_, v)| v.pos == p(7.0, 1.0, 0.0)));
    m.apply(&c.inverse());
    assert_eq!(m, before);
}

#[test]
fn copies_that_touch_merge_with_the_original() {
    let mut m = Model::new();
    let f = square(&mut m, 0.0, 0.0, 2.0);
    m.copy(&[], &[f], p(2.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (6, 7, 2));
}

#[test]
fn copying_loose_edges_copies_only_edges() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    let e = m.edges().next().unwrap().0;
    m.copy(&[e], &[], p(0.0, 1.0, 0.0), 2).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (6, 3, 0));
    assert_eq!(
        m.copy(&[e], &[], DVec3::ZERO, 1),
        Err(KernelError::ZeroDistance)
    );
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kernel --test transform`
Expected: compile errors (`move_vertices`, `copy`, `KernelError::Bend` not found).

- [ ] **Step 3: Implement**

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
    /// A push/pull would carry the face through other faces.
    Obstructed,
    /// A move would bend this face out of its plane.
    Bend(FaceId),
    /// A move would shrink an edge to nothing, turn a face inside out, or land a vertex on
    /// another.
    Squash,
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
            KernelError::Obstructed => {
                write!(f, "distance would push the face through other geometry")
            }
            KernelError::Bend(face) => {
                write!(f, "the move would bend face f{} out of its plane", face.0)
            }
            KernelError::Squash => write!(
                f,
                "the move would collapse an edge, turn a face over, or land a vertex on another"
            ),
        }
    }
}
```

`crates/kernel/src/push_pull.rs`: `fn orient(` becomes `pub(crate) fn orient(`.
`crates/kernel/src/lib.rs`: add `mod transform;` after `mod regions;`.

Create `crates/kernel/src/transform.rs`:
```rust
//! Moving and copying geometry.

use std::collections::BTreeSet;

use glam::DVec3;

use crate::edit::KernelError;
use crate::geom::{LENGTH_TOL, vector_area};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::{ChangeSet, Face, Model};

impl Model {
    /// Moves `vertices` by `offset` as one ChangeSet. Faces that use some of them stretch and
    /// may tilt, but must stay flat and keep their winding; edges between moved and fixed
    /// vertices must keep a length; no moved vertex may land on a fixed one. Moved geometry is
    /// not merged with what it ends up touching.
    pub fn move_vertices(
        &mut self,
        vertices: &BTreeSet<VertexId>,
        offset: DVec3,
    ) -> Result<ChangeSet, KernelError> {
        if !offset.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if offset.length() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        let moved: BTreeSet<VertexId> = vertices
            .iter()
            .copied()
            .filter(|v| self.vertex(*v).is_some())
            .collect();
        let at = |m: &Model, v: VertexId| {
            m.pos(v)
                + if moved.contains(&v) {
                    offset
                } else {
                    DVec3::ZERO
                }
        };

        for (_, e) in self.edges() {
            let [a, b] = e.v;
            if moved.contains(&a) != moved.contains(&b)
                && at(self, a).distance(at(self, b)) < LENGTH_TOL
            {
                return Err(KernelError::Squash);
            }
        }
        for &v in &moved {
            let to = self.pos(v) + offset;
            let lands = self
                .vertices()
                .any(|(w, vert)| !moved.contains(&w) && vert.pos.distance(to) < LENGTH_TOL);
            if lands {
                return Err(KernelError::Squash);
            }
        }
        // Faces that move partly get a new plane; check it and compute their new normals.
        let mut retilted: Vec<(FaceId, Face)> = Vec::new();
        for (id, f) in self.faces() {
            let all: Vec<VertexId> = loops(f).flatten().copied().collect();
            let touched = all.iter().filter(|v| moved.contains(v)).count();
            if touched == 0 || touched == all.len() {
                continue;
            }
            let outer: Vec<DVec3> = f.outer.iter().map(|v| at(self, *v)).collect();
            let area = vector_area(&outer);
            if area.dot(f.normal) <= LENGTH_TOL * LENGTH_TOL {
                return Err(KernelError::Squash);
            }
            let normal = area.normalize();
            let origin = outer[0];
            if all
                .iter()
                .any(|v| (at(self, *v) - origin).dot(normal).abs() > LENGTH_TOL)
            {
                return Err(KernelError::Bend(id));
            }
            for h in &f.holes {
                let pts: Vec<DVec3> = h.iter().map(|v| at(self, *v)).collect();
                if vector_area(&pts).dot(normal) >= 0.0 {
                    return Err(KernelError::Squash);
                }
            }
            if normal.distance(f.normal) > 1e-12 {
                let mut g = f.clone();
                g.normal = normal;
                retilted.push((id, g));
            }
        }
        for &v in &moved {
            let to = self.pos(v) + offset;
            self.move_vertex(v, to);
        }
        for (id, f) in retilted {
            self.replace_face(id, f);
        }
        Ok(self.take_changes())
    }

    /// Copies `edges` and `faces` (with their edges) `count` times, copy k shifted by
    /// `k * offset`, as one ChangeSet. Copies are drawn like new edges, so they merge with
    /// geometry they touch and close loops into faces; each copied face keeps the orientation
    /// of its original.
    pub fn copy(
        &mut self,
        edges: &[EdgeId],
        faces: &[FaceId],
        offset: DVec3,
        count: u32,
    ) -> Result<ChangeSet, KernelError> {
        if !offset.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if offset.length() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        let mut segments: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        for e in edges.iter().filter_map(|e| self.edge(*e)) {
            segments.insert((e.v[0].min(e.v[1]), e.v[0].max(e.v[1])));
        }
        let mut originals: Vec<(Vec<DVec3>, DVec3)> = Vec::new();
        for f in faces.iter().filter_map(|f| self.face(*f)) {
            for l in loops(f) {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    segments.insert((a.min(b), a.max(b)));
                }
            }
            originals.push((f.outer.iter().map(|v| self.pos(*v)).collect(), f.normal));
        }
        let points: Vec<(DVec3, DVec3)> = segments
            .iter()
            .map(|(a, b)| (self.pos(*a), self.pos(*b)))
            .collect();
        for k in 1..=count {
            let shift = offset * k as f64;
            for (a, b) in &points {
                self.draw_segment(*a + shift, *b + shift);
            }
        }
        for k in 1..=count {
            let shift = offset * k as f64;
            for (outer, normal) in &originals {
                let ids: Option<BTreeSet<VertexId>> =
                    outer.iter().map(|p| self.find_vertex(*p + shift)).collect();
                let Some(ids) = ids else { continue };
                let copy = self
                    .faces()
                    .find(|(_, f)| f.outer.iter().copied().collect::<BTreeSet<_>>() == ids)
                    .map(|(id, _)| id);
                if let Some(id) = copy {
                    self.orient(id, *normal);
                }
            }
        }
        Ok(self.take_changes())
    }
}

fn loops(f: &Face) -> impl Iterator<Item = &Vec<VertexId>> {
    std::iter::once(&f.outer).chain(&f.holes)
}
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 317.

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/transform.rs crates/kernel/tests/transform.rs crates/kernel/src/edit.rs crates/kernel/src/push_pull.rs crates/kernel/src/lib.rs
git commit -m "Add vertex moves that keep faces flat and copies with arrays"
```

---

### Task 2: Session move and copy commands

**Files:**
- Create: `crates/session/tests/transform.rs`
- Modify: `crates/session/src/command.rs`, `crates/session/src/document.rs`, `crates/session/src/lib.rs`

**Interfaces:**
- Consumes: `Model::move_vertices`, `Model::copy`, `KernelError::{Bend, Squash}` (Task 1).
- Produces:
  - `Command::Move { ids: Vec<EntityId>, offset: [f64; 3] }` (wire `move`).
  - `Command::Copy { ids: Vec<EntityId>, offset: [f64; 3], count: Option<u32> }` (wire `copy`).
  - `session::MAX_COPIES = 1000`.
  - Errors:
    - empty `ids` → `invalid_input`;
    - unknown ids → `not_found` with ids;
    - copy given a vertex id → `invalid_input` with ids;
    - count outside 1..=1000 → `invalid_input`;
    - any resulting coordinate beyond `MAX_COORDINATE` → `invalid_input`;
    - `Bend(f)` → `invalid_input` with `[f]`;
    - other kernel errors → `invalid_input` with the given ids.

- [ ] **Step 1: Write the failing tests**

Create `crates/session/tests/transform.rs`:
```rust
//! Move and copy, driven through JSON commands.

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

fn counts(d: &Document) -> (u64, u64, u64) {
    let s = ask(d, json!({"type": "stats"}));
    let n = |k: &str| s[k].as_u64().unwrap();
    (n("vertices"), n("edges"), n("faces"))
}

/// A 2x2 square on an empty model: vertices v0 v1 v3 v5, edges e2 e4 e6 e7, face f8.
fn square(d: &mut Document) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
}

#[test]
fn moving_a_face_moves_its_vertices_in_one_undo_step() {
    let mut d = doc();
    square(&mut d);
    let out = run(
        &mut d,
        json!({"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"].as_array().unwrap().len(), 4);
    let v = ask(&d, json!({"type": "entity", "id": "v0"}));
    assert_eq!(v["position"], json!([0.0, 0.0, 1.0]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    let v = ask(&d, json!({"type": "entity", "id": "v0"}));
    assert_eq!(v["position"], json!([0.0, 0.0, 0.0]));
}

#[test]
fn vertices_can_be_moved_within_their_face() {
    let mut d = doc();
    square(&mut d);
    run(
        &mut d,
        json!({"type": "move", "ids": ["v0"], "offset": [-1, -1, 0]}),
    )
    .unwrap();
    let area = ask(&d, json!({"type": "entity", "id": "f8"}))["area"]
        .as_f64()
        .unwrap();
    assert!((area - 6.0).abs() < 1e-9, "{area}");
}

#[test]
fn bad_moves_name_the_entities() {
    let mut d = doc();
    square(&mut d);
    let mv = |ids: Value, offset: Value| json!({"type": "move", "ids": ids, "offset": offset});
    assert_eq!(
        run(&mut d, mv(json!(["v0"]), json!([0, 0, 1]))).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["f8".to_string()]),
        "lifting one corner bends the face"
    );
    assert_eq!(
        run(&mut d, mv(json!(["v0", "e99"]), json!([1, 0, 0]))).unwrap_err(),
        (ErrorKind::NotFound, vec!["e99".to_string()])
    );
    assert_eq!(
        run(&mut d, mv(json!([]), json!([1, 0, 0]))).unwrap_err().0,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        run(&mut d, mv(json!(["f8"]), json!([2e6, 0, 0])))
            .unwrap_err()
            .0,
        ErrorKind::InvalidInput
    );
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
}

#[test]
fn copy_with_a_count_is_one_undo_step() {
    let mut d = doc();
    square(&mut d);
    run(
        &mut d,
        json!({"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 2}),
    )
    .unwrap();
    assert_eq!(counts(&d), (12, 12, 3));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (4, 4, 1));
    run(
        &mut d,
        json!({"type": "copy", "ids": ["e2"], "offset": [0, 3, 0]}),
    )
    .unwrap();
    assert_eq!(counts(&d), (6, 5, 1));
}

#[test]
fn bad_copies_change_nothing() {
    let mut d = doc();
    square(&mut d);
    let cp = |ids: Value, count: u32| json!({"type": "copy", "ids": ids, "offset": [3, 0, 0], "count": count});
    assert_eq!(
        run(&mut d, cp(json!(["v0"]), 1)).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["v0".to_string()])
    );
    assert_eq!(
        run(&mut d, cp(json!(["f8"]), 0)).unwrap_err().0,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        run(
            &mut d,
            json!({"type": "copy", "ids": ["f8"], "offset": [6e5, 0, 0], "count": 2})
        )
        .unwrap_err()
        .0,
        ErrorKind::InvalidInput
    );
    assert_eq!(counts(&d), (4, 4, 1));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p session --test transform`
Expected: the tests compile, then panic on the unknown `move`/`copy` JSON variants.

- [ ] **Step 3: Implement**

`crates/session/src/command.rs`, add before `Select`:
```rust
    /// Move vertices, edges and faces by `offset` meters as one undo step. Faces that share
    /// some of the moved vertices stretch and may tilt, but must stay flat.
    Move {
        ids: Vec<EntityId>,
        offset: [f64; 3],
    },
    /// Copy edges and faces `count` times (default 1), copy k shifted by `k * offset`, as one
    /// undo step. Copies merge with geometry they touch; copied faces keep their orientation.
    Copy {
        ids: Vec<EntityId>,
        offset: [f64; 3],
        #[serde(default)]
        count: Option<u32>,
    },
```

`crates/session/src/lib.rs`: `pub use document::{Document, MAX_COORDINATE, MAX_COPIES};`.

`crates/session/src/document.rs`:
- **Kernel import.** `use kernel::{ChangeSet, EdgeId, EntityId, FaceId, KernelError, Model, VertexId};`
- **Copy limit.** Add above `EVENT_LOG_LEN`:
```rust
/// Most copies one `copy` command may make.
pub const MAX_COPIES: u32 = 1000;
```
- **`unreachable!` arm.** In `execute`, the arm lists `Command::Move { .. }` and `Command::Copy { .. }` too.
- **`edit` arms.** In `edit`, add after the `Erase` arm:
```rust
            Command::Move { ids, offset } => self.move_changes(ids, *offset),
            Command::Copy { ids, offset, count } => {
                self.copy_changes(ids, *offset, count.unwrap_or(1))
            }
```
- **New methods.** Add before `fn erase_changes`:
```rust
    fn move_changes(
        &mut self,
        ids: &[EntityId],
        offset: [f64; 3],
    ) -> Result<ChangeSet, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one vertex, edge or face",
            ));
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        check_coordinates(&[offset])?;
        let vertices = self.vertex_ids(ids);
        let offset = DVec3::from(offset);
        self.check_moved(&vertices, &[offset])?;
        self.model
            .move_vertices(&vertices, offset)
            .map_err(|e| self.kernel_error(e, ids))
    }

    fn copy_changes(
        &mut self,
        ids: &[EntityId],
        offset: [f64; 3],
        count: u32,
    ) -> Result<ChangeSet, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge or face",
            ));
        }
        if !(1..=MAX_COPIES).contains(&count) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("count must be between 1 and {MAX_COPIES}"),
            ));
        }
        let (edges, faces) = self.edges_and_faces(ids)?;
        check_coordinates(&[offset])?;
        let offset = DVec3::from(offset);
        self.check_moved(&self.vertex_ids(ids), &[offset * count as f64])?;
        self.model
            .copy(&edges, &faces, offset, count)
            .map_err(|e| self.kernel_error(e, ids))
    }

    /// Rejects moves or copies that would put a vertex beyond `MAX_COORDINATE`.
    fn check_moved(
        &self,
        vertices: &BTreeSet<VertexId>,
        offsets: &[DVec3],
    ) -> Result<(), CommandError> {
        let moved: Vec<[f64; 3]> = vertices
            .iter()
            .flat_map(|v| {
                let p = self.model.position(*v).unwrap();
                offsets.iter().map(move |o| (p + *o).to_array())
            })
            .collect();
        check_coordinates(&moved)
    }

    /// The vertices that make up the entities (vertices themselves, edge ends, face loops).
    fn vertex_ids(&self, ids: &[EntityId]) -> BTreeSet<VertexId> {
        let m = &self.model;
        let mut out = BTreeSet::new();
        for id in ids {
            match *id {
                EntityId::Vertex(v) => {
                    out.insert(v);
                }
                EntityId::Edge(e) => out.extend(m.edge(e).map(|e| e.v).into_iter().flatten()),
                EntityId::Face(f) => {
                    if let Some(f) = m.face(f) {
                        out.extend(f.outer.iter().chain(f.holes.iter().flatten()));
                    }
                }
            }
        }
        out
    }

    /// A kernel error from moving or copying `ids`, naming the bent face when there is one.
    fn kernel_error(&self, e: KernelError, ids: &[EntityId]) -> CommandError {
        let culprits = match e {
            KernelError::Bend(f) => vec![EntityId::Face(f)],
            _ => ids.to_vec(),
        };
        CommandError::new(ErrorKind::InvalidInput, e.to_string()).with_ids(culprits)
    }
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 322.

- [ ] **Step 5: Commit**

```bash
git add crates/session/src/command.rs crates/session/src/document.rs crates/session/src/lib.rs crates/session/tests/transform.rs
git commit -m "Add move and copy commands"
```

---

### Task 3: Tool plumbing for Move

**Files:**
- Modify:
  - `crates/tools/src/tool.rs`, `crates/tools/src/vcb.rs`
  - `crates/tools/src/{line,rectangle,push_pull,select,erase}.rs`
  - `crates/app/src/editor.rs`

**Interfaces:**
- Produces:
  - `ToolContext.selection: &'a BTreeSet<EntityId>`.
  - `ToolEvent::ToggleCopy`, which existing tools ignore.
  - `tools::vcb::{Array::{Times(u32), Divide(u32)}, parse_array(&str) -> Option<Array>}` (`5x`, `x5`, `*5`, `/5`; a count of 0 is rejected).
  - The editor passes the document's selection to tools.
  - The editor sends `ToolEvent::Release` only when the matching press reached the tool.
  - The stray doc comment above `pub struct SelectTool` is gone.

- [ ] **Step 1: Write the failing tests**

In `crates/tools/src/vcb.rs` tests, add before `formats_meters`:
```rust
    #[test]
    fn array_syntax() {
        assert_eq!(parse_array("5x"), Some(Array::Times(5)));
        assert_eq!(parse_array(" X3 "), Some(Array::Times(3)));
        assert_eq!(parse_array("*2"), Some(Array::Times(2)));
        assert_eq!(parse_array("/4"), Some(Array::Divide(4)));
        for bad in ["5", "0x", "/0", "x", "/", "2.5x", "-3x", "5m"] {
            assert_eq!(parse_array(bad), None, "{bad:?}");
        }
    }
```

In `crates/app/src/editor.rs` tests, add at the end of the module:
```rust
    #[test]
    fn a_release_without_its_press_is_not_sent_to_the_tool() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        assert!(!e.button(Button::Left, false));
        assert!(e.doc().selection().is_empty());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools vcb; cargo test -p app release_without`
Expected: a compile error (`parse_array` not found). The editor test fails: the release reaches the Select tool, which clears nothing but returns true.

- [ ] **Step 3: Implement**

`crates/tools/src/vcb.rs`, add before `/// A length for display`:
```rust
/// A typed array of copies: "5x", "x5" or "*5" make 5 copies at the last copy's spacing;
/// "/5" makes 5 copies evenly between the original and the last copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Array {
    Times(u32),
    Divide(u32),
}

/// Parses array syntax; None if it is not an array or the count is 0.
pub fn parse_array(s: &str) -> Option<Array> {
    let s = s.trim().to_ascii_lowercase();
    let count = |n: &str| n.trim().parse::<u32>().ok().filter(|n| *n > 0);
    if let Some(n) = s.strip_prefix('/') {
        return count(n).map(Array::Divide);
    }
    let n = s
        .strip_suffix('x')
        .or_else(|| s.strip_prefix('x'))
        .or_else(|| s.strip_prefix('*'))?;
    count(n).map(Array::Times)
}
```

`crates/tools/src/tool.rs`:
- **Imports.** `use std::collections::BTreeSet;` and `use kernel::{EntityId, Model};`.
- **Selection field.** `ToolContext` gains, after `shift`:
```rust
    /// The selected edges and faces.
    pub selection: &'a BTreeSet<EntityId>,
```
- **Copy toggle.** `ToolEvent` gains, after `Cancel`:
```rust
    /// The copy modifier was tapped (Ctrl): tools that can copy switch between moving and
    /// copying.
    ToggleCopy,
```

Existing tools ignore `ToggleCopy`:
- `line.rs`, `rectangle.rs`, `push_pull.rs`: the `ToolEvent::Release(_) => {}` arm becomes `ToolEvent::Release(_) | ToolEvent::ToggleCopy => {}`.
- `erase.rs`: `ToolEvent::Release(_) | ToolEvent::Cancel | ToolEvent::ToggleCopy => {}`.
- `select.rs`: add `ToolEvent::ToggleCopy => {}` after the `Cancel` arm. Also delete the orphaned line `/// How far (physical pixels) the cursor must move between press and release to make a box.` and the blank line after it, above `#[derive(Clone, Debug, Default)] pub struct SelectTool`.
- Every test `ToolContext { ... }` literal in these five files gains `selection: &Default::default(),` after `shift`.

`crates/app/src/editor.rs`:
- **Gating field.** `Editor` gains, after `shift`:
```rust
    /// Whether the left-button press reached the tool, so its release should too.
    tool_pressed: bool,
```
initialized `tool_pressed: false`.
- **Gate in `button`.** Insert this right after the `if button != Button::Left { return false; }` check:
```rust
        // A release reaches the tool only if its press did (not one that began on a panel or
        // in another tool).
        if !pressed && !std::mem::take(&mut self.tool_pressed) {
            return false;
        }
```
and set `self.tool_pressed = pressed;` just before `self.send(...)`.
- **Selection in the context.** The `ToolContext` in `handle` gains `selection: self.doc.selection(),`.

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 324.

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/tool.rs crates/tools/src/vcb.rs crates/tools/src/line.rs crates/tools/src/rectangle.rs crates/tools/src/push_pull.rs crates/tools/src/select.rs crates/tools/src/erase.rs crates/app/src/editor.rs
git commit -m "Pass the selection to tools, parse array copies, drop unmatched releases"
```

---

### Task 4: Move tool

**Files:**
- Create: `crates/tools/src/move_tool.rs`
- Modify: `crates/tools/src/tool.rs`, `crates/tools/src/lib.rs`

**Interfaces:**
- Consumes: `ToolContext.selection`, `ToolEvent::ToggleCopy`, `parse_array` (Task 3); `pick`, `face_outline` (select.rs); `Command::{Move, Copy, Undo}`.
- Produces:
  - `MoveTool`.
  - `ToolKind::Move` (label "Move", shortcut `m`), placed between PushPull and Erase in `ToolKind::ALL`.

- [ ] **Step 1: Write the failing tests**

Create `crates/tools/src/move_tool.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;
    use std::collections::BTreeSet;

    /// A 2 m square (face f8) and a camera looking at it.
    struct Rig {
        tool: MoveTool,
        model: Model,
        camera: Camera,
        selection: BTreeSet<EntityId>,
        cursor: DVec2,
    }

    impl Rig {
        fn new(selected: &[&str]) -> Self {
            let mut model = Model::new();
            let pts = [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ];
            model.add_polygon(&pts).unwrap();
            Self {
                tool: MoveTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                selection: selected.iter().map(|s| s.parse().unwrap()).collect(),
                cursor: DVec2::ZERO,
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                drag_threshold: 4.0,
                shift: false,
                selection: &self.selection,
            };
            self.tool.handle(event, &ctx)
        }

        /// Puts the cursor over `p` and clicks there.
        fn click(&mut self, p: DVec3) -> ToolOutput {
            self.cursor = self.camera.project(p).unwrap();
            self.send(ToolEvent::Click(at(p)))
        }
    }

    fn at(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: SnapKind::Ground,
            face: None,
        }
    }

    fn ids(s: &[&str]) -> Vec<EntityId> {
        s.iter().map(|s| s.parse().unwrap()).collect()
    }

    #[test]
    fn two_clicks_move_the_selection() {
        let mut r = Rig::new(&["f8", "e2"]);
        r.click(DVec3::ZERO);
        let moved = r.send(ToolEvent::Move(at(DVec3::new(1.0, 2.0, 0.0))));
        assert_eq!(
            moved.preview.len(),
            5 + 5 + 1,
            "outline, moved outline, guide"
        );
        let out = r.click(DVec3::new(1.0, 2.0, 0.0));
        assert_eq!(
            out.commands,
            vec![Command::Move {
                ids: ids(&["e2", "f8"]),
                offset: [1.0, 2.0, 0.0]
            }]
        );
        assert!(r.tool.anchor().is_none());
    }

    #[test]
    fn without_a_selection_the_first_click_picks_what_to_move() {
        let mut r = Rig::new(&[]);
        assert!(r.click(DVec3::new(1.0, 1.0, 0.0)).message.is_none());
        let out = r.click(DVec3::new(1.0, 1.0, 1.0));
        assert_eq!(
            out.commands,
            vec![Command::Move {
                ids: ids(&["f8"]),
                offset: [0.0, 0.0, 1.0]
            }]
        );
        let empty = r.click(DVec3::new(-3.0, 4.0, 0.0));
        assert!(empty.message.is_some() && empty.commands.is_empty());
    }

    #[test]
    fn typed_distance_moves_along_the_cursor_direction() {
        let mut r = Rig::new(&["f8"]);
        r.click(DVec3::ZERO);
        r.send(ToolEvent::Move(at(DVec3::new(0.5, 0.0, 0.0))));
        let out = r.send(ToolEvent::Value("3".into()));
        assert_eq!(
            out.commands,
            vec![Command::Move {
                ids: ids(&["f8"]),
                offset: [3.0, 0.0, 0.0]
            }]
        );
    }

    #[test]
    fn ctrl_copies_and_arrays_replace_the_copy() {
        let mut r = Rig::new(&["f8"]);
        r.send(ToolEvent::ToggleCopy);
        r.click(DVec3::ZERO);
        let out = r.click(DVec3::new(3.0, 0.0, 0.0));
        assert_eq!(
            out.commands,
            vec![Command::Copy {
                ids: ids(&["f8"]),
                offset: [3.0, 0.0, 0.0],
                count: None
            }]
        );
        let times = r.send(ToolEvent::Value("4x".into()));
        assert_eq!(
            times.commands,
            vec![
                Command::Undo,
                Command::Copy {
                    ids: ids(&["f8"]),
                    offset: [3.0, 0.0, 0.0],
                    count: Some(4)
                }
            ]
        );
        let divide = r.send(ToolEvent::Value("/3".into()));
        assert_eq!(
            divide.commands,
            vec![
                Command::Undo,
                Command::Copy {
                    ids: ids(&["f8"]),
                    offset: [1.0, 0.0, 0.0],
                    count: Some(3)
                }
            ]
        );
    }

    #[test]
    fn arrays_need_a_fresh_copy() {
        let mut r = Rig::new(&["f8"]);
        assert!(r.send(ToolEvent::Value("3x".into())).message.is_some());
        r.send(ToolEvent::ToggleCopy);
        r.click(DVec3::ZERO);
        r.click(DVec3::new(3.0, 0.0, 0.0));
        r.tool.on_external_change();
        let out = r.send(ToolEvent::Value("3x".into()));
        assert!(out.commands.is_empty() && out.message.is_some());
    }
}
```

`crates/tools/src/lib.rs`:
```rust
//! Interactive tools and inference. Tools turn pointer input into `session::Command`s; they never
//! edit the model directly.

pub mod erase;
pub mod inference;
pub mod line;
pub mod move_tool;
pub mod overlay;
pub mod push_pull;
pub mod rectangle;
pub mod select;
pub mod tool;
pub mod vcb;

pub use erase::EraseTool;
pub use inference::{Axis, Inference, SnapKind, edge_under, face_under, infer};
pub use line::LineTool;
pub use move_tool::MoveTool;
pub use push_pull::PushPullTool;
pub use rectangle::RectangleTool;
pub use select::SelectTool;
pub use tool::{Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools move_tool`
Expected: compile errors (`MoveTool` not found).

- [ ] **Step 3: Implement**

Prepend to `crates/tools/src/move_tool.rs`:
```rust
//! The Move tool: click a base point, then the destination; the selection (or else the edge or
//! face under the first click) moves by the difference. Tapping Ctrl switches to copying.
//! After a copy, typing "5x" makes 5 copies at that spacing and "/5" puts 5 copies evenly up
//! to it, replacing the single copy.

use glam::DVec3;
use kernel::EntityId;
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::select::{face_outline, pick};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{Array, format_length, parse_array, parse_length};

/// A move in progress.
#[derive(Clone, Debug)]
struct Drag {
    base: DVec3,
    ids: Vec<EntityId>,
    /// What moves, drawn at its original position.
    outline: Vec<[DVec3; 2]>,
}

#[derive(Clone, Debug, Default)]
pub struct MoveTool {
    drag: Option<Drag>,
    hover: Option<DVec3>,
    copy: bool,
    /// The last copy made (what and how far), which a typed array replaces.
    last_copy: Option<(Vec<EntityId>, DVec3)>,
}

impl MoveTool {
    pub fn new() -> Self {
        Self::default()
    }

    fn finish(&mut self, offset: DVec3, out: &mut ToolOutput) {
        let Some(drag) = self.drag.take() else { return };
        if self.copy {
            out.commands.push(Command::Copy {
                ids: drag.ids.clone(),
                offset: offset.to_array(),
                count: None,
            });
            self.last_copy = Some((drag.ids, offset));
        } else {
            out.commands.push(Command::Move {
                ids: drag.ids,
                offset: offset.to_array(),
            });
            self.last_copy = None;
        }
    }

    /// Replaces the last copy with an array of copies.
    fn array(&mut self, array: Array, out: &mut ToolOutput) {
        let Some((ids, offset)) = self.last_copy.clone() else {
            out.message = Some("Make a copy first, then type 5x or /5".into());
            return;
        };
        let (spacing, count) = match array {
            Array::Times(n) => (offset, n),
            Array::Divide(n) => (offset / n as f64, n),
        };
        out.commands.push(Command::Undo);
        out.commands.push(Command::Copy {
            ids,
            offset: spacing.to_array(),
            count: Some(count),
        });
    }
}

/// Segments outlining edges and faces.
fn outline(ctx: &ToolContext, ids: &[EntityId]) -> Vec<[DVec3; 2]> {
    let m = ctx.model;
    let mut out = Vec::new();
    for id in ids {
        match *id {
            EntityId::Edge(e) => {
                if let Some(e) = m.edge(e)
                    && let (Some(a), Some(b)) = (m.position(e.v[0]), m.position(e.v[1]))
                {
                    out.push([a, b]);
                }
            }
            EntityId::Face(f) => {
                if let Some(f) = m.face(f) {
                    out.extend(face_outline(m, f));
                }
            }
            EntityId::Vertex(_) => {}
        }
    }
    out
}

impl Tool for MoveTool {
    fn name(&self) -> &'static str {
        "Move"
    }

    fn prompt(&self) -> &'static str {
        match (&self.drag, self.copy, &self.last_copy) {
            (Some(_), false, _) => "Click the destination, or type a distance",
            (Some(_), true, _) => "Copy: click the destination, or type a distance",
            (None, _, Some(_)) => "Type 5x or /5 for more copies, or click a new base point",
            (None, false, None) => "Select geometry, then click a base point (Ctrl copies)",
            (None, true, None) => "Copy: select geometry, then click a base point",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        let distance = self
            .drag
            .as_ref()
            .zip(self.hover)
            .map(|(d, h)| format_length(d.base.distance(h)));
        ("Distance", distance)
    }

    fn anchor(&self) -> Option<DVec3> {
        self.drag.as_ref().map(|d| d.base)
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => match &self.drag {
                None => {
                    let ids: Vec<EntityId> = if ctx.selection.is_empty() {
                        pick(ctx).into_iter().collect()
                    } else {
                        ctx.selection.iter().copied().collect()
                    };
                    if ids.is_empty() {
                        out.message = Some("Select geometry, or click an edge or face".into());
                    } else {
                        self.drag = Some(Drag {
                            base: hit.point,
                            outline: outline(ctx, &ids),
                            ids,
                        });
                        self.hover = Some(hit.point);
                    }
                }
                Some(d) => {
                    let offset = hit.point - d.base;
                    if offset.length() >= LENGTH_TOL {
                        self.finish(offset, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => match &self.drag {
                Some(d) => {
                    let dir = self
                        .hover
                        .map(|h| h - d.base)
                        .filter(|v| v.length() >= LENGTH_TOL);
                    match (dir, parse_length(&text)) {
                        (None, _) => {
                            out.message = Some("Move the cursor to set a direction first".into());
                        }
                        (_, None) => out.message = Some(format!("Not a distance: {text}")),
                        (Some(_), Some(len)) if len.abs() < LENGTH_TOL => {
                            out.message = Some("Distance must not be zero".into());
                        }
                        (Some(dir), Some(len)) => self.finish(dir.normalize() * len, &mut out),
                    }
                }
                None => match parse_array(&text) {
                    Some(array) => self.array(array, &mut out),
                    None => out.message = Some(format!("Not an array like 5x or /5: {text}")),
                },
            },
            ToolEvent::ToggleCopy => self.copy = !self.copy,
            ToolEvent::Cancel => self.drag = None,
            ToolEvent::Release(_) => {}
        }
        if let (Some(d), Some(hover)) = (&self.drag, self.hover) {
            let offset = hover - d.base;
            out.preview = d.outline.clone();
            if offset.length() >= LENGTH_TOL {
                out.preview
                    .extend(d.outline.iter().map(|[a, b]| [*a + offset, *b + offset]));
                out.preview.push([d.base, hover]);
            }
        }
        out
    }

    fn on_external_change(&mut self) {
        self.drag = None;
        self.last_copy = None;
    }
}
```

Replace `crates/tools/src/tool.rs` with:
```rust
//! What every interactive tool implements, and the tools the app can switch between.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{EntityId, Model};
use session::{Camera, Command};

use crate::erase::EraseTool;
use crate::inference::Inference;
use crate::line::LineTool;
use crate::move_tool::MoveTool;
use crate::push_pull::PushPullTool;
use crate::rectangle::RectangleTool;
use crate::select::SelectTool;

/// What a tool may look at while handling an event.
pub struct ToolContext<'a> {
    pub model: &'a Model,
    pub camera: &'a Camera,
    /// Cursor position in physical pixels.
    pub cursor: DVec2,
    /// How close (physical pixels) the cursor must be to an edge to pick it.
    pub pick_radius: f64,
    /// How far (physical pixels) the cursor must travel from a press for it to be a drag.
    pub drag_threshold: f64,
    /// Whether Shift is held.
    pub shift: bool,
    /// The selected edges and faces.
    pub selection: &'a BTreeSet<EntityId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolEvent {
    /// The cursor moved; carries the current inference.
    Move(Inference),
    /// The primary button was pressed at the current inference.
    Click(Inference),
    /// The primary button was released at the current inference.
    Release(Inference),
    /// Text typed into the measurements box, submitted with Enter.
    Value(String),
    /// Escape: abandon the operation in progress.
    Cancel,
    /// The copy modifier was tapped (Ctrl): tools that can copy switch between moving and
    /// copying.
    ToggleCopy,
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
    Select,
    Line,
    Rectangle,
    PushPull,
    Move,
    Erase,
}

impl ToolKind {
    /// Every tool, in toolbar order.
    pub const ALL: [ToolKind; 6] = [
        ToolKind::Select,
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::PushPull,
        ToolKind::Move,
        ToolKind::Erase,
    ];

    pub fn make(self) -> Box<dyn Tool> {
        match self {
            ToolKind::Select => Box::new(SelectTool::new()),
            ToolKind::Line => Box::new(LineTool::new()),
            ToolKind::Rectangle => Box::new(RectangleTool::new()),
            ToolKind::PushPull => Box::new(PushPullTool::new()),
            ToolKind::Move => Box::new(MoveTool::new()),
            ToolKind::Erase => Box::new(EraseTool::new()),
        }
    }

    /// The tool's name, as its `Tool::name` reports it.
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Select => "Select",
            ToolKind::Line => "Line",
            ToolKind::Rectangle => "Rectangle",
            ToolKind::PushPull => "Push/Pull",
            ToolKind::Move => "Move",
            ToolKind::Erase => "Erase",
        }
    }

    /// The key that selects the tool.
    pub fn shortcut(self) -> char {
        match self {
            ToolKind::Select => ' ',
            ToolKind::Line => 'l',
            ToolKind::Rectangle => 'r',
            ToolKind::PushPull => 'p',
            ToolKind::Move => 'm',
            ToolKind::Erase => 'e',
        }
    }

    pub fn from_shortcut(c: char) -> Option<ToolKind> {
        let c = c.to_ascii_lowercase();
        Self::ALL.into_iter().find(|k| k.shortcut() == c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_tool_names_and_shortcuts_round_trip() {
        for kind in ToolKind::ALL {
            assert_eq!(kind.make().name(), kind.label());
            assert_eq!(ToolKind::from_shortcut(kind.shortcut()), Some(kind));
        }
        assert_eq!(ToolKind::from_shortcut('E'), Some(ToolKind::Erase));
        assert_eq!(ToolKind::from_shortcut('z'), None);
    }
}
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 329.

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/move_tool.rs crates/tools/src/tool.rs crates/tools/src/lib.rs
git commit -m "Add the Move tool with copy and array copies"
```

---
### Task 5: Copy toggle and copies in the editor

**Files:**
- Modify: `crates/app/src/editor.rs`, `crates/app/src/app.rs`

**Interfaces:**
- Consumes: `ToolEvent::ToggleCopy`, the Move tool (Task 4), `Command::{Copy, Select}`.
- Produces:
  - `Editor::toggle_copy(&mut self) -> bool`.
  - A successful `Copy` from a tool selects the copy's edges and faces (`Select` with `Replace`).
  - `Editor::run` returns `Option<Outcome>`.
  - The window calls `toggle_copy` when Ctrl goes down (`ModifiersChanged`).

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/editor.rs` tests, add before `a_release_without_its_press_is_not_sent_to_the_tool`:
```rust
    #[test]
    fn move_tool_moves_the_selection_by_a_typed_distance() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert!(e.key(EditorKey::Char('m')));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        move_to(&mut e, DVec3::new(2.0, 4.0, 0.0));
        type_text(&mut e, "2");
        assert_eq!(stats(&e), (4, 1));
        let moved = DVec3::new(1.0, 3.0, 0.0);
        let model = e.doc().model();
        assert!(
            model
                .vertices()
                .any(|(_, v)| v.pos.abs_diff_eq(moved, 1e-6))
        );
    }

    #[test]
    fn copies_are_selected_and_arrays_replace_them() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('m'));
        assert!(e.toggle_copy());
        assert!(e.status().contains("Copy"), "{}", e.status());
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        move_to(&mut e, DVec3::new(5.0, 2.0, 0.0));
        type_text(&mut e, "3");
        assert_eq!(stats(&e), (8, 2));
        assert_eq!(
            e.doc().selection().len(),
            5,
            "the copy's four edges and face"
        );
        type_text(&mut e, "3x");
        assert_eq!(stats(&e), (16, 4));
        e.undo();
        assert_eq!(stats(&e), (4, 1), "the array is one undo step");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app`
Expected: compile error (`toggle_copy` not found).

- [ ] **Step 3: Implement**

`crates/app/src/editor.rs`:
- **Imports.** `use kernel::EntityId;` and `use session::{BatchError, Command, CommandError, Document, ErrorKind, Outcome, SelectMode};`.
- **Copy toggle.** Add after `button`:
```rust
    /// The copy modifier (Ctrl) was tapped.
    pub fn toggle_copy(&mut self) -> bool {
        self.send(ToolEvent::ToggleCopy);
        true
    }
```
- **Replace `apply` and `run` with:**
```rust
    /// Runs the tool's commands. A copy selects what it created, as in SketchUp.
    fn apply(&mut self, out: ToolOutput) {
        for command in out.commands {
            let is_copy = matches!(command, Command::Copy { .. });
            if let Some(outcome) = self.run(command)
                && is_copy
            {
                let ids = outcome
                    .changes
                    .created
                    .into_iter()
                    .filter(|id| !matches!(id, EntityId::Vertex(_)))
                    .collect();
                self.run(Command::Select {
                    ids,
                    mode: SelectMode::Replace,
                });
            }
        }
        self.preview = out.preview;
    }

    /// Runs a command; a failure becomes the status message.
    fn run(&mut self, command: Command) -> Option<Outcome> {
        match self.doc.execute(command) {
            Ok(outcome) => Some(outcome),
            Err(e) if matches!(e.kind, ErrorKind::NothingToUndo | ErrorKind::NothingToRedo) => None,
            Err(e) => {
                self.message = Some(e.message);
                None
            }
        }
    }
```

`crates/app/src/app.rs`, the `ModifiersChanged` arm becomes:
```rust
            WindowEvent::ModifiersChanged(m) => {
                let ctrl_pressed = m.state().control_key() && !s.modifiers.control_key();
                s.modifiers = m.state();
                s.editor
                    .set_modifiers(m.state().shift_key(), m.state().alt_key());
                // Tapping Ctrl switches tools that can copy (Move) between moving and copying.
                ctrl_pressed && s.editor.toggle_copy()
            }
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 331.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/editor.rs crates/app/src/app.rs
git commit -m "Toggle copying with Ctrl and select copies made in the window"
```

---

### Task 6: Parallel, perpendicular and locked inference

**Files:**
- Modify: `crates/tools/src/inference.rs`, `crates/tools/src/overlay.rs`, `crates/tools/src/lib.rs`

**Interfaces:**
- Produces:
  - `SnapKind::{Parallel, Perpendicular}`.
  - `Guides { anchor, reference, lock }` (Default).
  - `Lock { dir, kind }`.
  - `infer_with(model, camera, cursor, &Guides, radius_px) -> Inference`. The old `infer(model, camera, cursor, anchor, radius_px)` stays as a wrapper with no reference and no lock.
  - `directions(reference) -> Vec<(DVec3, SnapKind)>`.
  - `overlay::MAGENTA`, used for parallel and perpendicular previews.
  - Exports: `tools::{Guides, Lock, infer_with}`.

- [ ] **Step 1: Write the failing tests**

In `crates/tools/src/inference.rs` tests, add at the end of the module:
```rust
    fn guides(anchor: DVec3, reference: Option<DVec3>, lock: Option<Lock>) -> Guides {
        Guides {
            anchor: Some(anchor),
            reference,
            lock,
        }
    }

    #[test]
    fn follows_parallel_and_perpendicular_to_the_reference() {
        let c = cam();
        let anchor = DVec3::new(0.0, 3.0, 0.0);
        let r = DVec3::new(2.0, 1.0, 0.0).normalize();
        let g = guides(anchor, Some(r), None);
        let along = anchor + r * 2.0;
        let hit = infer_with(&Model::new(), &c, px(&c, along), &g, R);
        assert_eq!(hit.kind, SnapKind::Parallel);
        assert!(hit.point.abs_diff_eq(along, 1e-6));
        let across = anchor + DVec3::new(1.0, -2.0, 0.0).normalize() * 2.0;
        let hit = infer_with(&Model::new(), &c, px(&c, across), &g, R);
        assert_eq!(hit.kind, SnapKind::Perpendicular);
        assert!(hit.point.abs_diff_eq(across, 1e-6));
    }

    #[test]
    fn a_reference_along_an_axis_adds_nothing() {
        assert_eq!(directions(Some(DVec3::new(0.0, 3.0, 0.0))).len(), 3);
        let tilted = directions(Some(DVec3::new(1.0, 1.0, 0.0)));
        // Parallel and the horizontal perpendicular; the other perpendicular is vertical (Z).
        assert_eq!(tilted.len(), 5);
        assert_eq!(directions(None).len(), 3);
    }

    #[test]
    fn a_lock_holds_the_point_on_its_line() {
        let (c, mut m) = (cam(), Model::new());
        m.add_edge(DVec3::new(3.0, 3.0, 2.0), DVec3::new(4.0, 3.0, 2.0))
            .unwrap();
        let anchor = DVec3::new(1.0, 1.0, 0.0);
        let lock = Lock {
            dir: DVec3::Z,
            kind: SnapKind::Axis(Axis::Z),
        };
        let g = guides(anchor, None, Some(lock));
        let far = infer_with(&m, &c, px(&c, DVec3::new(-4.0, 5.0, 0.0)), &g, R);
        assert_eq!(far.kind, SnapKind::Axis(Axis::Z));
        assert!(far.point.x == 1.0 && far.point.y == 1.0);
        // An endpoint under the cursor sets the height.
        let snapped = infer_with(&m, &c, px(&c, DVec3::new(3.0, 3.0, 2.0)), &g, R);
        assert_eq!(snapped.point, DVec3::new(1.0, 1.0, 2.0));
    }
```

In `crates/tools/src/overlay.rs` tests, add before `endpoint_marker_is_a_square_of_constant_screen_size`:
```rust
    #[test]
    fn parallel_and_perpendicular_previews_are_magenta() {
        for kind in [SnapKind::Parallel, SnapKind::Perpendicular] {
            let hit = Inference {
                point: DVec3::new(1.0, 2.0, 0.0),
                kind,
                face: None,
            };
            let segs = overlay(&[[DVec3::ZERO, hit.point]], Some(&hit), &cam(), 1.0);
            assert_eq!(segs[0].color, MAGENTA);
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools`
Expected: compile errors (`Guides`, `Lock`, `infer_with`, `SnapKind::Parallel`, `MAGENTA` not found).

- [ ] **Step 3: Implement**

`crates/tools/src/inference.rs`:
- `SnapKind` gains, after `Axis(Axis)`:
```rust
    /// Parallel to the reference edge, through the point the tool is drawing from.
    Parallel,
    /// Perpendicular to the reference edge, through the point the tool is drawing from.
    Perpendicular,
```
- Replace the whole `pub fn infer(...)` function (with its doc comment) with these items:
```rust
/// What inference can follow from the point a tool is drawing from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Guides {
    /// The point a tool is drawing from; directions are measured from it.
    pub anchor: Option<DVec3>,
    /// Unit direction of the last edge the cursor rested on: snaps parallel and perpendicular
    /// to it are offered from the anchor.
    pub reference: Option<DVec3>,
    /// A locked direction (Shift or an arrow key): the point stays on the line through the
    /// anchor, and points the cursor snaps to are projected onto it.
    pub lock: Option<Lock>,
}

/// A direction inference is held to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lock {
    /// Unit direction.
    pub dir: DVec3,
    /// How the locked point is reported (an axis, parallel or perpendicular).
    pub kind: SnapKind,
}

/// Finds the 3D point under `cursor` (physical pixels) with no guides but `anchor`, which
/// enables axis inference. See [`infer_with`].
pub fn infer(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    anchor: Option<DVec3>,
    radius_px: f64,
) -> Inference {
    let guides = Guides {
        anchor,
        ..Guides::default()
    };
    infer_with(model, camera, cursor, &guides, radius_px)
}

/// Finds the 3D point under `cursor` (physical pixels). Snap candidates count only within
/// `radius_px` of the cursor.
///
/// Ranking: endpoint, midpoint and origin (nearest on screen wins) > on-edge > directions from
/// the anchor (axes, parallel or perpendicular to the reference edge; nearest wins) > on-face >
/// ground > free. Points and edges hidden behind a face are skipped. With a lock, the result is
/// always on the locked line.
pub fn infer_with(
    model: &Model,
    camera: &Camera,
    cursor: DVec2,
    guides: &Guides,
    radius_px: f64,
) -> Inference {
    let ray = camera.ray(cursor);
    let face_hit = nearest_face_hit(model, &ray);
    let face = face_hit.map(|(f, _)| f);
    let screen_dist = |p: DVec3| camera.project(p).map(|s| s.distance(cursor));
    let snapped = snap_point(model, camera, cursor, radius_px);

    if let (Some(anchor), Some(lock)) = (guides.anchor, guides.lock) {
        let point = match snapped {
            Some((p, _)) => anchor + lock.dir * (p - anchor).dot(lock.dir),
            None => closest_on_line_to_ray(anchor, lock.dir, &ray, false),
        };
        return Inference {
            point,
            kind: lock.kind,
            face,
        };
    }
    if let Some((point, kind)) = snapped {
        return Inference { point, kind, face };
    }

    // Along a direction from the anchor. Direction lines are guides, so faces do not hide them.
    if let Some(anchor) = guides.anchor {
        let mut along: Option<(f64, Inference)> = None;
        for (dir, kind) in directions(guides.reference) {
            let p = closest_on_line_to_ray(anchor, dir, &ray, false);
            if let Some(d) = screen_dist(p).filter(|d| *d <= radius_px)
                && p.distance(anchor) > 0.0
                && along.is_none_or(|(bd, _)| d < bd)
            {
                along = Some((
                    d,
                    Inference {
                        point: p,
                        kind,
                        face,
                    },
                ));
            }
        }
        if let Some((_, hit)) = along {
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
    let mut best: Option<(f64, DVec3, SnapKind)> = None;
    let mut offer = |p: DVec3, kind: SnapKind| {
        if let Some(d) = within(p)
            && best.is_none_or(|(bd, _, _)| d < bd)
        {
            best = Some((d, p, kind));
        }
    };
    for (_, v) in model.vertices() {
        offer(v.pos, SnapKind::Endpoint);
    }
    for (_, e) in model.edges() {
        if let (Some(a), Some(b)) = (model.position(e.v[0]), model.position(e.v[1])) {
            offer((a + b) * 0.5, SnapKind::Midpoint);
        }
    }
    offer(DVec3::ZERO, SnapKind::Origin);
    if let Some((_, p, kind)) = best {
        return Some((p, kind));
    }

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
    on_edge.map(|(_, p)| (p, SnapKind::OnEdge))
}

/// Unit directions offered from the anchor: the three axes, plus, for a reference direction
/// not along an axis, the direction itself (parallel) and two perpendiculars (one horizontal,
/// one in the vertical plane through the reference) when they are not along an axis either.
pub fn directions(reference: Option<DVec3>) -> Vec<(DVec3, SnapKind)> {
    let mut out: Vec<(DVec3, SnapKind)> = Axis::ALL
        .into_iter()
        .map(|a| (a.dir(), SnapKind::Axis(a)))
        .collect();
    let along_axis = |d: DVec3| Axis::ALL.iter().any(|a| d.cross(a.dir()).length() < 1e-6);
    let Some(r) = reference.and_then(|r| r.try_normalize()) else {
        return out;
    };
    if along_axis(r) {
        return out;
    }
    out.push((r, SnapKind::Parallel));
    let horizontal = r.cross(DVec3::Z).try_normalize();
    let upright = horizontal.and_then(|h| r.cross(h).try_normalize());
    for p in [horizontal, upright].into_iter().flatten() {
        if !along_axis(p) {
            out.push((p, SnapKind::Perpendicular));
        }
    }
    out
}
```
`snap_point` is the old point and on-edge code, moved so the lock can project its result. Behavior without guides is unchanged; the existing inference tests cover it.

`crates/tools/src/overlay.rs`:
- add `pub const MAGENTA: Color = [0.80, 0.10, 0.80, 1.0];` after `BLUE`;
- in `overlay`'s line-color match, add after the blue arm:
```rust
        Some(SnapKind::Parallel | SnapKind::Perpendicular) => MAGENTA,
```

`crates/tools/src/lib.rs`, the inference export becomes:
```rust
pub use inference::{
    Axis, Guides, Inference, Lock, SnapKind, edge_under, face_under, infer, infer_with,
};
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 335.

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/inference.rs crates/tools/src/overlay.rs crates/tools/src/lib.rs
git commit -m "Infer parallel and perpendicular directions and hold locked directions"
```

---

### Task 7: Reference edges and direction locks in the editor

**Files:**
- Modify: `crates/app/src/editor.rs`, `crates/app/src/ui.rs`, `crates/app/src/app.rs`

**Interfaces:**
- Consumes: `Guides`, `Lock`, `infer_with`, `edge_under`, `Axis`, `SnapKind` (Task 6).
- Produces:
  - **New keys:** `EditorKey::LockAxis(Axis)` and `EditorKey::Unlock`.
  - **`Editor::set_modifiers(shift, alt) -> bool`:**
    - pressing Shift during an operation locks the direction under the cursor;
    - releasing Shift ends a Shift lock;
    - it returns true when the lock changed.
  - **Status.** `Status` gains `lock: Option<&'static str>`, such as "Locked: red axis", "Locked: parallel" or "Locked: perpendicular". `Display` shows it after the prompt, and the status bar shows it in bold.
  - **Reference and locks.**
    - The editor keeps the reference: the direction of the last edge under the cursor.
    - The lock is cleared when the tool has no anchor.
    - `focus_lost` ends a Shift lock.
  - **Window keys:** ArrowRight, ArrowLeft and ArrowUp map to `LockAxis(X/Y/Z)`; ArrowDown maps to `Unlock`. The `ModifiersChanged` arm also redraws when the lock changed.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/editor.rs` tests, add at the end of the module:
```rust
    /// The edge from the last click's chain: both ends of every edge, as positions.
    fn edge_ends(e: &Editor) -> Vec<[DVec3; 2]> {
        let m = e.doc().model();
        m.edges()
            .map(|(_, ed)| [m.position(ed.v[0]).unwrap(), m.position(ed.v[1]).unwrap()])
            .collect()
    }

    #[test]
    fn shift_locks_the_direction_under_the_cursor() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let up = e.doc().camera.project(DVec3::new(1.0, 1.0, 2.5)).unwrap();
        e.cursor_moved(up + DVec2::new(4.0, 0.0));
        assert!(e.set_modifiers(true, false));
        assert!(e.status().contains("Locked: blue axis"), "{}", e.status());
        click_at(&mut e, DVec3::new(4.0, 4.0, 0.0));
        let [a, b] = edge_ends(&e)[0];
        let near = |v: f64, w: f64| (v - w).abs() < 1e-9;
        let vertical = near(a.x, b.x) && near(a.y, b.y) && near(a.x, 1.0) && near(a.y, 1.0);
        assert!(vertical && (a.z - b.z).abs() > 0.5, "{a} {b}");
        assert!(e.set_modifiers(false, false), "releasing Shift unlocks");
        assert!(!e.status().contains("Locked"));
    }

    #[test]
    fn arrow_keys_lock_an_axis_during_an_operation() {
        let mut e = editor();
        assert!(!e.key(EditorKey::LockAxis(Axis::X)), "nothing to lock yet");
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert!(e.key(EditorKey::LockAxis(Axis::X)));
        assert!(e.status().contains("Locked: red axis"));
        click_at(&mut e, DVec3::new(3.0, 4.0, 0.0));
        let [a, b] = edge_ends(&e)[0];
        let near = |v: f64, w: f64| (v - w).abs() < 1e-9;
        let along_red = near(a.y, b.y) && near(a.z, 0.0) && near(b.z, 0.0);
        assert!(along_red && (a.x - b.x).abs() > 0.1, "{a} {b}");
        assert!(e.key(EditorKey::LockAxis(Axis::X)), "the same key unlocks");
        assert!(!e.status().contains("Locked"));
        e.key(EditorKey::LockAxis(Axis::Y));
        assert!(e.key(EditorKey::Unlock));
        e.key(EditorKey::Escape);
        assert!(
            !e.key(EditorKey::Unlock),
            "the chain ended, so nothing is locked"
        );
    }

    #[test]
    fn hovering_an_edge_teaches_parallel_snaps() {
        let mut e = editor();
        e.execute(Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [2.0, 1.0, 0.0],
        })
        .unwrap();
        let on_edge = e.doc().camera.project(DVec3::new(1.5, 0.75, 0.0)).unwrap();
        e.cursor_moved(on_edge + DVec2::new(0.0, 2.0));
        click_at(&mut e, DVec3::new(0.0, 3.0, 0.0));
        let along = DVec3::new(0.0, 3.0, 0.0) + DVec3::new(2.0, 1.0, 0.0).normalize() * 2.0;
        e.cursor_moved(e.doc().camera.project(along).unwrap());
        let magenta = tools::overlay::MAGENTA;
        assert!(e.overlay().iter().any(|v| v.color == magenta));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app`
Expected: compile errors (`EditorKey::LockAxis`, `set_modifiers` returns `()`).

- [ ] **Step 3: Implement**

`crates/app/src/editor.rs`:
- **Imports.** The tools import becomes:
```rust
use tools::{
    Axis, Guides, Inference, Lock, SnapKind, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput,
    edge_under, infer_with,
};
```
- **`EditorKey` variants.** `EditorKey` gains, after `Escape`:
```rust
    /// Lock the direction to an axis during an operation, or unlock it if it is locked there.
    LockAxis(Axis),
    /// Remove any direction lock.
    Unlock,
```
- **Replace `Status` and its `Display` impl with:**
```rust
/// What the status bar shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub tool: &'static str,
    pub prompt: &'static str,
    /// The measurements box label, e.g. "Length"; empty when the tool measures nothing.
    pub label: &'static str,
    /// Typed text (with a trailing "_") while typing, otherwise the live measurement.
    pub value: Option<String>,
    /// Feedback from the last tool action or failed command.
    pub message: Option<String>,
    /// The direction lock in force, e.g. "Locked: red axis".
    pub lock: Option<&'static str>,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}  |  {}", self.tool, self.prompt)?;
        if let Some(lock) = self.lock {
            write!(f, "  |  {lock}")?;
        }
        if let Some(value) = &self.value {
            write!(f, "  |  {}: {value}", self.label)?;
        }
        if let Some(message) = &self.message {
            write!(f, "  |  {message}")?;
        }
        Ok(())
    }
}
```
- **State.** `Editor` gains, after `tool_pressed`:
```rust
    /// Direction of the last edge the cursor rested on, for parallel/perpendicular snaps.
    reference: Option<DVec3>,
    /// The direction lock in force during the current operation.
    lock: Option<Lock>,
    /// Whether `lock` comes from holding Shift (and ends when it is released).
    shift_lock: bool,
```
initialized `reference: None, lock: None, shift_lock: false`.
- **Replace `set_modifiers` and `focus_lost` with:**
```rust
    /// Pressing Shift during an operation locks the direction the cursor follows (an axis,
    /// parallel or perpendicular); releasing it unlocks. Returns true if the lock changed.
    pub fn set_modifiers(&mut self, shift: bool, alt: bool) -> bool {
        let pressed = shift && !self.shift;
        let released = !shift && self.shift;
        self.shift = shift;
        self.nav.set_modifiers(shift, alt);
        if pressed && self.lock.is_none() {
            let (Some(anchor), Some(hit)) = (self.tool.anchor(), self.hit) else {
                return false;
            };
            let dir = match hit.kind {
                SnapKind::Axis(a) => Some(a.dir()),
                SnapKind::Parallel | SnapKind::Perpendicular => {
                    (hit.point - anchor).try_normalize()
                }
                _ => None,
            };
            if let Some(dir) = dir {
                self.lock = Some(Lock {
                    dir,
                    kind: hit.kind,
                });
                self.shift_lock = true;
                self.update_hover();
                return true;
            }
        } else if released && self.shift_lock {
            self.lock = None;
            self.shift_lock = false;
            self.update_hover();
            return true;
        }
        false
    }

    pub fn focus_lost(&mut self) {
        self.shift = false;
        if std::mem::take(&mut self.shift_lock) {
            self.lock = None;
        }
        self.nav.reset();
    }
```
- **Key arms.** In `key`, add after the `EditorKey::Escape => { ... }` arm:
```rust
            EditorKey::LockAxis(axis) => {
                if self.tool.anchor().is_none() {
                    return false;
                }
                let same = self.lock.is_some_and(|l| l.kind == SnapKind::Axis(axis));
                self.lock = (!same).then_some(Lock {
                    dir: axis.dir(),
                    kind: SnapKind::Axis(axis),
                });
                self.shift_lock = false;
                self.update_hover();
                true
            }
            EditorKey::Unlock => {
                let had = self.lock.take().is_some();
                self.shift_lock = false;
                self.update_hover();
                had
            }
```
- **Replace `status_parts` and `update_hover` with:**
```rust
    /// The tool, what to do next, the measurement and any feedback.
    pub fn status_parts(&self) -> Status {
        let (label, value) = self.tool.measurement();
        let value = if self.typed.is_empty() {
            value
        } else {
            Some(format!("{}_", self.typed))
        };
        let lock = self.lock.map(|l| match l.kind {
            SnapKind::Axis(Axis::X) => "Locked: red axis",
            SnapKind::Axis(Axis::Y) => "Locked: green axis",
            SnapKind::Axis(Axis::Z) => "Locked: blue axis",
            SnapKind::Parallel => "Locked: parallel",
            _ => "Locked: perpendicular",
        });
        Status {
            tool: self.tool.name(),
            prompt: self.tool.prompt(),
            label,
            value,
            message: self.message.clone(),
            lock,
        }
    }

    /// Recomputes the inference under the cursor and the tool preview. True if either changed.
    fn update_hover(&mut self) -> bool {
        let anchor = self.tool.anchor();
        if anchor.is_none() {
            // Locks last for one operation.
            self.lock = None;
            self.shift_lock = false;
        }
        let radius = PICK_RADIUS_LOGICAL_PX * self.scale_factor;
        let (model, camera) = (self.doc.model(), &self.doc.camera);
        let guides = Guides {
            anchor,
            reference: self.reference,
            lock: self.lock,
        };
        let hit = infer_with(model, camera, self.cursor, &guides, radius);
        // The edge the cursor rests on becomes the reference for later direction snaps.
        if let Some(e) = edge_under(model, camera, self.cursor, radius)
            && let Some(edge) = model.edge(e)
            && let (Some(a), Some(b)) = (model.position(edge.v[0]), model.position(edge.v[1]))
        {
            self.reference = (b - a).try_normalize();
        }
        let out = self.handle(ToolEvent::Move(hit));
        let changed = self.hit != Some(hit) || self.preview != out.preview;
        self.hit = Some(hit);
        self.preview = out.preview;
        changed
    }
```

`crates/app/src/ui.rs`:
- after `ui.label(view.status.prompt);` add:
```rust
            if let Some(lock) = view.status.lock {
                ui.label(RichText::new(lock).strong());
            }
```
- the test `Status` literal gains `lock: None,`.

`crates/app/src/app.rs`:
- **Import.** Add `use tools::Axis;` above the `crate::` imports.
- **`ModifiersChanged` arm** becomes:
```rust
            WindowEvent::ModifiersChanged(m) => {
                let ctrl_pressed = m.state().control_key() && !s.modifiers.control_key();
                s.modifiers = m.state();
                let locked = s
                    .editor
                    .set_modifiers(m.state().shift_key(), m.state().alt_key());
                // Tapping Ctrl switches tools that can copy (Move) between moving and copying.
                let toggled = ctrl_pressed && s.editor.toggle_copy();
                locked || toggled
            }
```
- **Arrow keys.** In `editor_key`, add after the `Delete` arm:
```rust
        // Arrow keys lock the drawing direction: right red, left green, up blue, down unlocks.
        Key::Named(NamedKey::ArrowRight) => Some(EditorKey::LockAxis(Axis::X)),
        Key::Named(NamedKey::ArrowLeft) => Some(EditorKey::LockAxis(Axis::Y)),
        Key::Named(NamedKey::ArrowUp) => Some(EditorKey::LockAxis(Axis::Z)),
        Key::Named(NamedKey::ArrowDown) => Some(EditorKey::Unlock),
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 338.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/editor.rs crates/app/src/ui.rs crates/app/src/app.rs
git commit -m "Learn reference edges and lock directions with Shift and arrow keys"
```

---

### Task 8: Agent docs, API test, spec amendment

**Files:**
- Modify: `crates/api/tests/dispatch.rs`, `docs/agent-api.md`, `docs/superpowers/specs/2026-10-10-tenon-design.md`

- [ ] **Step 1: Write the test**

In `crates/api/tests/dispatch.rs`:
- in `describe_lists_methods_and_schemas`, add `"move"` and `"copy"` to the variant list after `"erase"`;
- add before `render_view_returns_a_png_or_reports_no_gpu`:
```rust
#[test]
fn move_and_copy_over_json_rpc() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    let r = call(
        &mut h,
        "execute",
        json!({"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}),
    );
    assert_eq!(
        r["result"]["changes"]["modified"].as_array().unwrap().len(),
        4
    );
    call(
        &mut h,
        "execute",
        json!({"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 2}),
    );
    let stats = call(&mut h, "query", json!({"type": "stats"}));
    assert_eq!(stats["result"]["faces"], 3);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "move", "ids": ["v0"], "offset": [0, 0, 1]}),
    );
    assert_eq!(
        r["error"]["data"]["ids"],
        json!(["f8"]),
        "lifting one corner bends f8"
    );
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p api --test dispatch`
Expected: PASS (14 tests).

- [ ] **Step 3: Docs and spec**

Replace `docs/agent-api.md` with:
````markdown
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

The default socket is `tenon-<pid>.sock` in the temp directory, or in `~/.tenon/`
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
| `{"type": "selection"}` | the selected edge and face ids, plus `revision` |
| `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |

Unknown fields in commands, queries and params are rejected with `-32602` (a typo fails instead
of silently doing nothing).

Commands: `add_edge`, `add_rectangle`, `push_pull`, `erase`, `move`, `copy`, `select`, `undo`,
`redo`, `set_camera`. Units are
meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m. Entity ids are strings: `v12`
(vertex), `e3` (edge), `f7` (face).

Modeling commands, each one undo step:

| Command | Effect |
|---|---|
| `{"type": "add_edge", "start": [x,y,z], "end": [x,y,z]}` | A straight edge. It merges with nearby vertices, splits edges it crosses, and closed coplanar loops become faces. |
| `{"type": "add_rectangle", "corner": [0,0,0], "opposite": [2,3,0]}` | A rectangle with sides along the axes. Without `normal` the corners must differ in exactly two coordinates. With `"normal": [nx,ny,nz]` it lies in the plane through `corner` with that normal; `opposite` must be in that plane. Sides then run along u = `Z x normal` (horizontal; X on horizontal planes) and v = `normal x u`, so directions are fixed only up to sign: on a plane with normal +Y, u = -X and v = +Z; with normal -Y, u = +X. |
| `{"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}` | Moves vertices, edges and faces (their vertices) by `offset`. Faces that share only some moved vertices stretch and may tilt but must stay flat: a move that would bend one is `invalid_input` naming that face. A move that would collapse an edge, turn a face over or land a vertex on another is `invalid_input` naming the ids. Moved geometry does not merge with what it ends up touching. Unknown ids are `not_found`. |
| `{"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 4}` | Copies edges and faces `count` times (default 1, at most 1000); copy k is shifted by `k * offset`. Copies are drawn like new edges, so they merge with what they touch and closed loops become faces; copied faces keep the orientation of their originals. Vertex ids are `invalid_input`. |
| `{"type": "erase", "ids": ["e2", "f8"]}` | Erases edges and faces. An erased face leaves its edges. An erased edge takes the faces it bounds with it, except that two faces in one plane meeting at it merge into one. Vertices go with their last edge; a vertex left between two collinear edges is healed away. Faces are erased before edges, so erasing a face together with the seam it shares with a coplanar neighbour removes the neighbour too: the seam then bounds only one face. Vertex ids are `invalid_input` and unknown ids are `not_found`, both with `ids`; empty `ids` is `invalid_input`. |
| `{"type": "push_pull", "face": "f8", "distance": 1.5}` | Moves a face along its normal (negative: against it). A face on a flat sheet extrudes into a solid and stays as its base. A face on a solid extrudes a bump, or cuts a recess when pushed in. A face whose neighbors all run along the normal, like a box top, moves and stretches them. |

`{"type": "select", "ids": ["f8"], "mode": "replace"}` changes the selection (edges and faces;
`mode` is `replace` (default), `add`, `remove` or `toggle`). It is not an undo step and does not
change the revision. The window shows the selection and agents see what the user selected with
the `selection` query. Vertex ids are `invalid_input` and unknown ids are `not_found`, both with
`ids`. Empty `ids` with mode `replace` clears the selection.

The selection drops any entity that stops existing, through erase, merges, undo or redo. Undo does
not restore the selection. The `selection` query's `revision` is the model revision: it does not
change when only the selection changes, and there is no notification for selection changes, so
compare ids to detect changes.

Find the face to push from the `created` ids of the command that made it, or with
`{"type": "entities", "kinds": ["face"]}`. Faces created by push/pull point out of the solid.
`distance` is measured along the face's own normal, and faces in one plane can point either way
(a pulled-up rectangle's base faces down), so read the face's `normal` with
`{"type": "entity", "id": "f8"}` before choosing the sign. A push/pull that would collapse or
invert the edges around a moved face fails with `invalid_input`. So does one that would carry the
face through other geometry (pushing a recess through the bottom of a box); push-through
openings come in M4c.
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
````

In `docs/superpowers/specs/2026-10-10-tenon-design.md`:
- `Status:` line: `Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope)`.
- Replace the M4b and M4c lines with:
```markdown
- **M4b**: Move/Copy (with VCB array syntax); parallel/perpendicular inference; direction locks (Shift, arrow keys).
- **M4c**: push/pull through to openings; seam healing; apparent-intersection snaps; BVH picking.
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 339.

- [ ] **Step 5: Commit**

```bash
git add crates/api/tests/dispatch.rs docs/agent-api.md docs/superpowers/specs/2026-10-10-tenon-design.md
git commit -m "Document move and copy for agents and amend the M4b/M4c scope"
```

---

## Self-review notes

- **Spec coverage:**
  - Move and copy operations: Tasks 1 and 2.
  - Move/Copy tool and VCB arrays: Tasks 3 to 5.
  - Direction snaps (parallel, perpendicular, along the drawing direction via the hovered edge) and locks (Shift, arrows): Tasks 6 and 7.
  - Agent reachability: Tasks 2 and 8.
  - Apparent-intersection snaps and BVH picking are moved to M4c in Task 8.
- **M4a follow-ups folded in:** the selection in `ToolContext`, unmatched releases, and the stray doc comment (Task 3).
- **Every intermediate state was built and tested.** The totals in each Verify step come from replaying the tasks in order.
