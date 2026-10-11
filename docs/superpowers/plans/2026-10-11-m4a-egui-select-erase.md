# M4a egui Shell, Selection and Erase Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The window gets a real interface (toolbar, measurements box, Entity Info panel). Users and agents can select edges and faces and erase them, with SketchUp-style merging of coplanar faces and healing of collinear edges.

**Architecture:**
- **kernel** gains `Model::erase`:
  - An erased face leaves its edges.
  - An erased edge takes its faces with it, except that two faces in one plane merge, rebuilt through the existing region code.
  - Orphaned vertices are removed, and a vertex left between two collinear edges is healed away.
- **session** keeps a selection set with a `select` command (replace/add/remove/toggle; not an undo step), an `erase` command and a `selection` query. Entities that stop existing drop out of the selection.
- **render** highlights selected faces and edges in blue and can draw the scene into a sub-rectangle of the window.
- **tools** gains:
  - release events, a pick radius and the Shift state in `ToolContext`;
  - edge picking;
  - a Select tool (click, Shift-toggle, window and crossing box) and an Erase tool.
- **app** wraps the 3D view in egui 0.36 panels (egui-winit for input, egui-wgpu for painting). The status moves from the window title to a status bar, and Delete erases the selection.

**Tech Stack:** Rust 2024; new pins egui, egui-wgpu and egui-winit `=0.36.2`. They use wgpu 30 and winit 0.30, so the existing `wgpu =30.0.1` and `winit =0.30.13` pins are shared and no second copy is built.

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`: section 1 (erase), section 2 (Select and Erase tools), section 3 (selection highlight), section 4 (egui shell, input routing), section 5 (agent control). Milestone M4 is split into M4a/M4b/M4c in Task 8.

## Global Constraints

- Every new capability is a `session::Command` or `Query` reachable through the API. Tools only emit commands; panel buttons call the same editor paths as keys.
- Each modeling command is one undo step, and `ChangeSet::inverse` undoes it exactly. Kernel tests call `model.validate()` after edits.
- Selection is not undo history. It never changes the revision, and it holds only edges and faces.
- Entity ids on the wire are strings (`v3`, `e7`, `f2`); errors carry the offending ids.
- Input routing: egui first (pointer over a panel, or a widget using the keyboard), then navigation, then the active tool. Pointer presses reach the editor only inside the 3D view, but releases always do.
- New pins: `egui = "=0.36.2"`, `egui-wgpu = "=0.36.2"`, `egui-winit = "=0.36.2"`. No other dependency changes.
- Git: work on `dev`; stage only the files you changed; short plain commit messages, no emoji; `cargo fmt --all` before each commit; `cargo clippy --workspace --all-targets -- -D warnings` stays clean.

## Spec notes (rulings made in this plan)

- **Erase.**
  - **Faces:** erasing a face keeps its edges.
  - **Edges:** erasing an edge removes every face it bounds. The exception is exactly two faces in one plane: they merge, and the merged face takes the normal of the face it grows from.
  - **Vertices:** a vertex left without edges is removed. A vertex left with exactly two collinear edges is healed: the edges join and the vertex leaves every face loop. Healing only applies to vertices of erased edges.
  - **Errors:** vertex ids are `invalid_input`, and unknown ids are `not_found`; both carry ids.
- **Selection.** `select` takes `ids` and `mode` (`replace` default, `add`, `remove`, `toggle`).
  - Errors use the same id rules as erase.
  - Undo does not restore the selection.
  - A failed atomic batch restores the selection along with the camera.
- **Select tool.**
  - A click picks the visible edge under the cursor (within the pick radius), else the face under it. Clicking empty space clears the selection.
  - Dragging right selects what lies entirely inside the box; dragging left selects what the box touches. Box selection ignores occlusion.
  - Shift toggles. Shift-clicking empty space leaves the selection alone.
- **Erase tool:** clicking erases the edge or face under the cursor, which is outlined while hovered. Delete (or Backspace with nothing typed) erases the selection in any tool.
- **Shortcuts:** Space is Select and E is Erase. The default tool stays Line, so the existing editor tests are unchanged.
- **Status:** the window title goes back to `sketchup-alt`. Status, prompt and measurements box move to the bottom bar. The measurements box shows typed text, or the live value, read-only; typing still goes to it without focus, as in SketchUp.
- **Selected entities render blue:** faces tinted 50% toward blue, edges drawn blue. `render_view` shows the selection too.
- **Deferred to M4b:**
  - Move/Copy and VCB array syntax.
  - The full inference set.
  - Double-click to select a face with its edges.
  - Ctrl-add and Ctrl+Shift-subtract selection modifiers.
  - Erase-by-drag.
- **Deferred to M4c:** push-through openings and seam healing.

## File Structure

```
crates/kernel/src/erase.rs         new: Model::erase, merge via rebuild_plane, heal
crates/kernel/src/geom.rs          Plane::through
crates/kernel/src/faces.rs         rebuild_plane becomes pub(crate)
crates/kernel/src/edit.rs          loop_has_side becomes pub(crate)
crates/kernel/src/lib.rs           mod erase
crates/kernel/tests/erase.rs       new
crates/session/src/command.rs      Command::Erase, Command::Select, SelectMode
crates/session/src/query.rs        Query::Selection, QueryResult::Selection
crates/session/src/document.rs     selection set, select/erase, pruning, batch restore
crates/session/src/lib.rs          export SelectMode
crates/session/tests/selection.rs  new
crates/render/src/scene.rs         FaceVertex.selected, selection-aware model_geometry
crates/render/src/renderer.rs      Viewport, render_in, set_model(model, selection)
crates/render/src/shaders/scene.wgsl  selected tint
crates/render/src/snapshot.rs      capture(model, selection, ...)
crates/render/src/lib.rs           export Viewport
crates/render/tests/*.rs           updated calls, highlight and viewport tests
crates/api/src/headless.rs         pass the selection to capture
crates/tools/src/tool.rs           ToolEvent::Release, ToolContext.pick_radius/shift, ToolKind (Select, Erase, ALL, label, shortcut)
crates/tools/src/inference.rs      edge_under
crates/tools/src/select.rs         new: Select tool
crates/tools/src/erase.rs          new: Erase tool
crates/tools/src/{line,rectangle,push_pull}.rs  Release arms, test contexts
crates/tools/src/lib.rs            modules and exports
crates/app/src/editor.rs           shift, release, Delete/Backspace erase, Status, tool_kind
crates/app/src/ui.rs               new: panels (toolbar, status bar, Entity Info)
crates/app/src/info.rs             new: Entity Info text
crates/app/src/gui.rs              new: egui-winit input, egui-wgpu painting
crates/app/src/app.rs              egui wiring, view rectangle, routing, Delete/E keys
crates/app/src/host.rs             pass the selection to capture
crates/app/src/main.rs             modules
Cargo.toml, crates/app/Cargo.toml  egui pins
crates/api/tests/dispatch.rs       select/erase over JSON-RPC
docs/agent-api.md                  select, erase, selection query
docs/superpowers/specs/...md       M4 split into M4a/M4b/M4c
```

Workspace test totals after each task: 274, 281, 284, 285, 292, 296, 299, 300 (265 before M4a).

---

### Task 1: Kernel erase with merging and healing

**Files:**
- Create: `crates/kernel/src/erase.rs`, `crates/kernel/tests/erase.rs`
- Modify: `crates/kernel/src/geom.rs`, `crates/kernel/src/faces.rs`, `crates/kernel/src/edit.rs`, `crates/kernel/src/lib.rs`

**Interfaces:**
- Consumes: `rebuild_plane(plane, new_edges)` (faces.rs; with no new edges only regions inside old faces are kept), `loop_has_side`, recorded primitives `remove_face`, `remove_edge`, `insert_edge`, `replace_face`, `remove_vertex`.
- Produces:
  - `Model::erase(&mut self, edges: &[EdgeId], faces: &[FaceId]) -> ChangeSet` (ids that do not exist are ignored; never fails).
  - `Plane::through(p: DVec3, normal: DVec3) -> Plane` (canonical orientation).

- [ ] **Step 1: Write the failing tests**

Create `crates/kernel/tests/erase.rs`:
```rust
use glam::DVec3;
use kernel::{EdgeId, EntityId, FaceId, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn rect(m: &mut Model, x0: f64, y0: f64, x1: f64, y1: f64) -> FaceId {
    let c = m
        .add_polygon(&[
            p(x0, y0, 0.0),
            p(x1, y0, 0.0),
            p(x1, y1, 0.0),
            p(x0, y1, 0.0),
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

fn edge_at(m: &Model, a: DVec3, b: DVec3) -> EdgeId {
    m.edges()
        .find(|(_, e)| {
            let (pa, pb) = (m.position(e.v[0]).unwrap(), m.position(e.v[1]).unwrap());
            (pa.distance(a) < 1e-9 && pb.distance(b) < 1e-9)
                || (pa.distance(b) < 1e-9 && pb.distance(a) < 1e-9)
        })
        .map(|(id, _)| id)
        .expect("edge")
}

fn counts(m: &Model) -> (usize, usize, usize) {
    (m.vertices().count(), m.edges().count(), m.faces().count())
}

#[test]
fn erasing_a_face_keeps_its_edges() {
    let mut m = Model::new();
    let f = rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    let c = m.erase(&[], &[f]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (4, 4, 0));
    assert_eq!(c.summary().deleted, vec![EntityId::Face(f)]);
}

#[test]
fn erasing_a_box_edge_removes_the_two_faces_it_bounds() {
    let mut m = Model::new();
    let f = rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    m.push_pull(f, 1.0).unwrap();
    let e = edge_at(&m, p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    m.erase(&[e], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 11, 4));
}

#[test]
fn erasing_the_seam_between_coplanar_faces_merges_and_heals() {
    let mut m = Model::new();
    rect(&mut m, 0.0, 0.0, 4.0, 2.0);
    m.add_edge(p(2.0, 0.0, 0.0), p(2.0, 2.0, 0.0)).unwrap();
    assert_eq!(counts(&m), (6, 7, 2));
    let seam = edge_at(&m, p(2.0, 0.0, 0.0), p(2.0, 2.0, 0.0));
    m.erase(&[seam], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (4, 4, 1));
    let (id, f) = m.faces().next().unwrap();
    assert_eq!(f.outer.len(), 4);
    assert_eq!(m.area(id), Some(8.0));
}

#[test]
fn a_merged_face_keeps_the_orientation_of_the_faces_it_joins() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    m.push_pull(base, 1.0).unwrap();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0)).unwrap();
    let seam = edge_at(&m, p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0));
    m.erase(&[seam], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 12, 6));
    let bottom: Vec<_> = m
        .faces()
        .filter(|(_, f)| f.outer.iter().all(|v| m.position(*v).unwrap().z == 0.0))
        .collect();
    assert_eq!(bottom.len(), 1);
    assert_eq!(bottom[0].1.normal, -DVec3::Z);
}

#[test]
fn erasing_a_side_of_an_inner_square_opens_it_into_the_outer_face() {
    let mut m = Model::new();
    rect(&mut m, 0.0, 0.0, 4.0, 4.0);
    rect(&mut m, 1.0, 1.0, 3.0, 3.0);
    assert_eq!(m.faces().count(), 2);
    let side = edge_at(&m, p(1.0, 1.0, 0.0), p(3.0, 1.0, 0.0));
    m.erase(&[side], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 7, 1));
    let (id, f) = m.faces().next().unwrap();
    assert!(f.holes.is_empty());
    assert_eq!(m.area(id), Some(16.0));
}

#[test]
fn erasing_a_loose_edge_removes_its_free_ends() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    let e = m.edges().next().unwrap().0;
    m.erase(&[e], &[]);
    assert_eq!(m, Model::new());
}

#[test]
fn erasing_a_spur_heals_the_side_it_split() {
    let mut m = Model::new();
    rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    assert_eq!(counts(&m), (6, 6, 1));
    let spur = edge_at(&m, p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0));
    m.erase(&[spur], &[]);
    m.validate().unwrap();
    assert_eq!(counts(&m), (4, 4, 1));
    assert_eq!(m.faces().next().unwrap().1.outer.len(), 4);
}

#[test]
fn erase_undoes_and_redoes_exactly() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 4.0, 2.0);
    m.push_pull(base, 1.0).unwrap();
    m.add_edge(p(2.0, 0.0, 1.0), p(2.0, 2.0, 1.0)).unwrap();
    let before = m.clone();
    let seam = edge_at(&m, p(2.0, 0.0, 1.0), p(2.0, 2.0, 1.0));
    let corner = edge_at(&m, p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
    let c = m.erase(&[seam, corner], &[]);
    m.validate().unwrap();
    let after = m.clone();
    m.apply(&c.inverse());
    assert_eq!(m, before);
    m.apply(&c);
    assert_eq!(m, after);
}

#[test]
fn missing_ids_are_ignored() {
    let mut m = Model::new();
    rect(&mut m, 0.0, 0.0, 2.0, 2.0);
    let before = m.clone();
    assert!(m.erase(&[EdgeId(999)], &[FaceId(998)]).is_empty());
    assert_eq!(m, before);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p kernel --test erase`
Expected: compile errors: no method named `erase`.

- [ ] **Step 3: Implement**

`crates/kernel/src/geom.rs`, add to `impl Plane` before `fn distance`:
```rust
    /// The plane through `p` with normal `normal` (unit length), in canonical orientation.
    pub fn through(p: DVec3, normal: DVec3) -> Plane {
        let normal = canonical(normal);
        Plane {
            normal,
            d: normal.dot(p),
        }
    }
```

`crates/kernel/src/faces.rs`: `fn rebuild_plane(` becomes `pub(crate) fn rebuild_plane(`.
`crates/kernel/src/edit.rs`: `fn loop_has_side(` becomes `pub(crate) fn loop_has_side(`.
`crates/kernel/src/lib.rs`: add `mod erase;` after `mod edit;`.

Create `crates/kernel/src/erase.rs`:
```rust
//! Erasing edges and faces, with SketchUp-style healing.

use std::collections::BTreeSet;

use crate::edit::loop_has_side;
use crate::geom::{ANGLE_TOL, PLANE_ANGLE_TOL, Plane};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::{ChangeSet, Face, Model};

impl Model {
    /// Erases faces and edges as one ChangeSet; ids that do not exist are ignored.
    ///
    /// An erased face leaves its edges. An erased edge takes the faces it bounds with it, except
    /// when exactly two faces in one plane meet there: they merge into one face. Vertices left
    /// without edges are removed, and a vertex left between two collinear edges is healed so
    /// the two edges become one.
    pub fn erase(&mut self, edges: &[EdgeId], faces: &[FaceId]) -> ChangeSet {
        for &f in faces {
            if self.face(f).is_some() {
                self.remove_face(f);
            }
        }
        let mut touched: BTreeSet<VertexId> = BTreeSet::new();
        for &e in edges {
            let Some(edge) = self.edge(e) else { continue };
            let [a, b] = edge.v;
            touched.extend([a, b]);
            let users: Vec<FaceId> = self
                .faces()
                .filter(|(_, f)| loops(f).any(|l| loop_has_side(l, a, b)))
                .map(|(id, _)| id)
                .collect();
            let merge = match users[..] {
                [f, g] => self.shared_plane(f, g),
                _ => None,
            };
            if merge.is_none() {
                for f in &users {
                    self.remove_face(*f);
                }
            }
            self.remove_edge(e);
            if let Some(plane) = merge {
                // The two faces still exist; rebuilding their plane without the edge replaces
                // them with the merged region, which keeps the orientation of the face it lies in.
                self.rebuild_plane(plane, &[]);
            }
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
        self.take_changes()
    }

    /// The plane of faces `f` and `g` if they lie in the same plane.
    fn shared_plane(&self, f: FaceId, g: FaceId) -> Option<Plane> {
        let (ff, gg) = (self.face(f)?, self.face(g)?);
        let plane = Plane::through(self.pos(ff.outer[0]), ff.normal);
        let same = ff.normal.cross(gg.normal).length() < PLANE_ANGLE_TOL
            && plane.contains(self.pos(gg.outer[0]));
        same.then_some(plane)
    }

    /// Joins the two edges at `v` into one if they are collinear and nothing else meets there,
    /// removing `v` from the loops of the faces around it.
    fn heal(&mut self, v: VertexId) {
        let [e1, e2] = self.edges_at(v)[..] else {
            return;
        };
        let other = |e: EdgeId| {
            let [a, b] = self.edge(e).unwrap().v;
            if a == v { b } else { a }
        };
        let (p, q) = (other(e1), other(e2));
        let (dp, dq) = (self.pos(p) - self.pos(v), self.pos(q) - self.pos(v));
        let straight =
            dp.cross(dq).length() <= ANGLE_TOL * dp.length() * dq.length() && dp.dot(dq) < 0.0;
        if !straight || self.edge_between(p, q).is_some() {
            return;
        }
        let users: Vec<(FaceId, Face)> = self
            .faces()
            .filter(|(_, f)| loops(f).any(|l| l.contains(&v)))
            .map(|(id, f)| (id, f.clone()))
            .collect();
        self.remove_edge(e1);
        self.remove_edge(e2);
        self.insert_edge(p, q);
        for (id, mut f) in users {
            f.outer.retain(|x| *x != v);
            f.holes.iter_mut().for_each(|h| h.retain(|x| *x != v));
            self.replace_face(id, f);
        }
        self.remove_vertex(v);
    }
}

fn loops(f: &Face) -> impl Iterator<Item = &Vec<VertexId>> {
    std::iter::once(&f.outer).chain(&f.holes)
}
```

The merge does not insert the merged face itself. It removes the edge while the two faces still exist, then calls `rebuild_plane(plane, &[])`. With no new edges, only regions lying inside old faces are kept. The merged region's interior point lies inside one of the two faces, so reconcile replaces both faces with one, taking that face's normal (the M3 rule).

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 274.

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/erase.rs crates/kernel/tests/erase.rs crates/kernel/src/geom.rs crates/kernel/src/faces.rs crates/kernel/src/edit.rs crates/kernel/src/lib.rs
git commit -m "Add erase with coplanar face merging and collinear edge healing"
```

---

### Task 2: Session selection, select and erase commands

**Files:**
- Create: `crates/session/tests/selection.rs`
- Modify: `crates/session/src/command.rs`, `crates/session/src/query.rs`, `crates/session/src/document.rs`, `crates/session/src/lib.rs`

**Interfaces:**
- Consumes: `Model::erase` (Task 1).
- Produces:
  - `Command::Erase { ids: Vec<EntityId> }` (wire `erase`): one undo step. Empty `ids`, or vertex ids, are `invalid_input`; unknown ids are `not_found` with `ids`.
  - `Command::Select { ids: Vec<EntityId>, mode: SelectMode }` (wire `select`; `mode` defaults to `replace`).
  - `session::SelectMode::{Replace, Add, Remove, Toggle}` (snake_case on the wire).
  - `Command::changes_model()` is false for `SetCamera` and `Select`.
  - `Document::selection(&self) -> &BTreeSet<EntityId>`.
  - `Query::Selection` returns `QueryResult::Selection { ids: Vec<EntityId>, revision: u64 }`.
  - Every model change (edit, undo, redo, atomic batch) drops ids that no longer exist from the selection. A failed atomic batch restores the selection.

- [ ] **Step 1: Write the failing tests**

Create `crates/session/tests/selection.rs`:
```rust
//! Selection and erase, driven through JSON commands.

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

fn selection(d: &Document) -> Value {
    ask(d, json!({"type": "selection"}))["ids"].clone()
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
fn select_modes_combine_with_the_current_selection() {
    let mut d = doc();
    square(&mut d);
    let revision = d.revision();
    run(&mut d, json!({"type": "select", "ids": ["f8"]})).unwrap();
    assert_eq!(selection(&d), json!(["f8"]));
    run(
        &mut d,
        json!({"type": "select", "ids": ["e2", "e4"], "mode": "add"}),
    )
    .unwrap();
    assert_eq!(selection(&d), json!(["e2", "e4", "f8"]));
    run(
        &mut d,
        json!({"type": "select", "ids": ["f8", "e6"], "mode": "toggle"}),
    )
    .unwrap();
    assert_eq!(selection(&d), json!(["e2", "e4", "e6"]));
    run(
        &mut d,
        json!({"type": "select", "ids": ["e2"], "mode": "remove"}),
    )
    .unwrap();
    assert_eq!(selection(&d), json!(["e4", "e6"]));
    run(&mut d, json!({"type": "select", "ids": []})).unwrap();
    assert_eq!(selection(&d), json!([]));
    assert_eq!(
        d.revision(),
        revision,
        "selection does not change the model"
    );
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
}

#[test]
fn only_existing_edges_and_faces_can_be_selected() {
    let mut d = doc();
    square(&mut d);
    run(&mut d, json!({"type": "select", "ids": ["f8"]})).unwrap();
    assert_eq!(
        run(&mut d, json!({"type": "select", "ids": ["v0", "e2"]})).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["v0".to_string()])
    );
    assert_eq!(
        run(&mut d, json!({"type": "select", "ids": ["e2", "f99"]})).unwrap_err(),
        (ErrorKind::NotFound, vec!["f99".to_string()])
    );
    assert_eq!(selection(&d), json!(["f8"]), "unchanged after errors");
}

#[test]
fn erase_is_one_undo_step() {
    let mut d = doc();
    square(&mut d);
    let out = run(&mut d, json!({"type": "erase", "ids": ["e2"]})).unwrap();
    assert_eq!(
        out["changes"]["deleted"].as_array().unwrap().len(),
        2,
        "{out}"
    );
    assert_eq!(counts(&d), (4, 3, 0));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (4, 4, 1));
    run(&mut d, json!({"type": "erase", "ids": ["f8"]})).unwrap();
    assert_eq!(counts(&d), (4, 4, 0));
}

#[test]
fn bad_erases_name_the_entities_and_change_nothing() {
    let mut d = doc();
    square(&mut d);
    assert_eq!(
        run(&mut d, json!({"type": "erase", "ids": ["v0"]})).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["v0".to_string()])
    );
    assert_eq!(
        run(&mut d, json!({"type": "erase", "ids": ["e2", "e99"]})).unwrap_err(),
        (ErrorKind::NotFound, vec!["e99".to_string()])
    );
    assert_eq!(
        run(&mut d, json!({"type": "erase", "ids": []}))
            .unwrap_err()
            .0,
        ErrorKind::InvalidInput
    );
    assert_eq!(counts(&d), (4, 4, 1));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
}

#[test]
fn entities_that_stop_existing_leave_the_selection() {
    let mut d = doc();
    square(&mut d);
    run(&mut d, json!({"type": "select", "ids": ["f8", "e2", "e4"]})).unwrap();
    run(&mut d, json!({"type": "erase", "ids": ["e2"]})).unwrap();
    assert_eq!(selection(&d), json!(["e4"]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(
        selection(&d),
        json!(["e4"]),
        "undo does not restore the selection"
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(selection(&d), json!([]));
}

#[test]
fn failed_atomic_batch_restores_the_selection() {
    let mut d = doc();
    square(&mut d);
    run(&mut d, json!({"type": "select", "ids": ["f8"]})).unwrap();
    let cmds = vec![
        cmd(json!({"type": "select", "ids": ["e2"]})),
        cmd(json!({"type": "erase", "ids": ["e4"]})),
        cmd(json!({"type": "erase", "ids": ["e99"]})),
    ];
    let err = d.execute_batch(cmds, true).unwrap_err();
    assert!(err.rolled_back);
    assert_eq!(selection(&d), json!(["f8"]));
    assert_eq!(counts(&d), (4, 4, 1));
}

#[test]
fn select_and_erase_are_classified_for_tools() {
    assert!(!cmd(json!({"type": "select", "ids": []})).changes_model());
    assert!(cmd(json!({"type": "erase", "ids": ["e1"]})).changes_model());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p session --test selection`
Expected: the tests compile and fail at runtime, because `select`, `erase` and `selection` are unknown JSON variants.

- [ ] **Step 3: Implement**

`crates/session/src/command.rs`, add these variants before `Undo`:
```rust
    /// Erase edges and faces as one undo step. An erased edge takes the faces it bounds with
    /// it, except that two faces in one plane meeting there merge into one. Vertices go away
    /// with their last edge and cannot be erased directly.
    Erase {
        ids: Vec<EntityId>,
    },
    /// Change the selection of edges and faces. Not recorded in undo history.
    Select {
        ids: Vec<EntityId>,
        #[serde(default)]
        mode: SelectMode,
    },
```
Replace `impl Command` with this, and add `SelectMode` after it:
```rust
impl Command {
    /// True for commands that can change the model (everything except camera and selection).
    pub fn changes_model(&self) -> bool {
        !matches!(self, Command::SetCamera { .. } | Command::Select { .. })
    }
}

/// How `Command::Select` combines `ids` with the current selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SelectMode {
    /// The selection becomes exactly `ids`.
    #[default]
    Replace,
    Add,
    Remove,
    /// Selected ids are removed, the others added.
    Toggle,
}
```

`crates/session/src/lib.rs`: export `SelectMode` from `command`:
```rust
pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
```

`crates/session/src/query.rs`:
- add `Selection` to `Query` before `Events`:
```rust
    /// The selected edges and faces.
    Selection,
```
- add to `QueryResult` before `Events`:
```rust
    Selection {
        ids: Vec<EntityId>,
        revision: u64,
    },
```

`crates/session/src/document.rs`:
- **Imports.** `use std::collections::{BTreeSet, VecDeque};` and `use crate::command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};`.
- **State.** `Document` gains `selection: BTreeSet<EntityId>` after `camera`, initialized with `selection: BTreeSet::new()`. Its doc comment becomes `/// The open model plus everything needed to edit it: camera, selection and undo history.`
- **Accessor.** Add before `pub fn revision`:
```rust
    /// The selected edges and faces. Entities leave it when they stop existing.
    pub fn selection(&self) -> &BTreeSet<EntityId> {
        &self.selection
    }
```
- **Replace `execute` and `edit` with:**
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
            Command::Select { ids, mode } => {
                self.edges_and_faces(&ids)?;
                let ids: BTreeSet<EntityId> = ids.into_iter().collect();
                match mode {
                    SelectMode::Replace => self.selection = ids,
                    SelectMode::Add => self.selection.extend(ids),
                    SelectMode::Remove => self.selection.retain(|id| !ids.contains(id)),
                    SelectMode::Toggle => {
                        self.selection =
                            self.selection.symmetric_difference(&ids).copied().collect();
                    }
                }
                Ok(Outcome::default())
            }
            Command::AddEdge { .. }
            | Command::AddRectangle { .. }
            | Command::PushPull { .. }
            | Command::Erase { .. } => {
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
            Command::Erase { ids } => self.erase_changes(ids),
            Command::Undo | Command::Redo | Command::SetCamera { .. } | Command::Select { .. } => {
                return None;
            }
        })
    }
```
- **Atomic batch restore.** In `execute_batch`, next to `let camera_before = self.camera.clone();` add `let selection_before = self.selection.clone();`. Next to `self.camera = camera_before;` add `self.selection = selection_before;`.
- **New methods.** Add before `fn record`:
```rust
    fn erase_changes(&mut self, ids: &[EntityId]) -> Result<ChangeSet, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge or face",
            ));
        }
        let (edges, faces) = self.edges_and_faces(ids)?;
        Ok(self.model.erase(&edges, &faces))
    }

    /// Splits ids into edges and faces. Vertex ids are invalid input and unknown ids are not
    /// found; both errors list the offending ids.
    fn edges_and_faces(
        &self,
        ids: &[EntityId],
    ) -> Result<(Vec<EdgeId>, Vec<FaceId>), CommandError> {
        let vertices: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Vertex(_)))
            .collect();
        if !vertices.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "only edges and faces can be selected or erased",
            )
            .with_ids(vertices));
        }
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(
                CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
            );
        }
        let edges = ids
            .iter()
            .filter_map(|id| match id {
                EntityId::Edge(e) => Some(*e),
                _ => None,
            })
            .collect();
        let faces = ids
            .iter()
            .filter_map(|id| match id {
                EntityId::Face(f) => Some(*f),
                _ => None,
            })
            .collect();
        Ok((edges, faces))
    }

    fn exists(&self, id: EntityId) -> bool {
        match id {
            EntityId::Vertex(v) => self.model.vertex(v).is_some(),
            EntityId::Edge(e) => self.model.edge(e).is_some(),
            EntityId::Face(f) => self.model.face(f).is_some(),
        }
    }
```
- **Replace `fn changed` with:**
```rust
    /// Bumps the revision, logs the change and drops entities that no longer exist from the
    /// selection.
    fn changed(&mut self, changes: &ChangeSet) -> Outcome {
        self.revision += 1;
        self.selection = self
            .selection
            .iter()
            .copied()
            .filter(|id| self.exists(*id))
            .collect();
        let summary = changes.summary();
        self.events.push_back(ChangeEvent {
            revision: self.revision,
            changes: summary.clone(),
        });
        if self.events.len() > EVENT_LOG_LEN {
            self.events.pop_front();
        }
        Outcome { changes: summary }
    }
```
- **Selection query.** In `query`, add before the `Query::Events` arm:
```rust
            Query::Selection => QueryResult::Selection {
                ids: self.selection.iter().copied().collect(),
                revision: self.revision,
            },
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 281.

- [ ] **Step 5: Commit**

```bash
git add crates/session/src/command.rs crates/session/src/query.rs crates/session/src/document.rs crates/session/src/lib.rs crates/session/tests/selection.rs
git commit -m "Add selection with select and erase commands"
```

---

### Task 3: Selection highlight and drawing into a viewport

**Files:**
- Modify: `crates/render/src/scene.rs`, `crates/render/src/renderer.rs`, `crates/render/src/shaders/scene.wgsl`, `crates/render/src/snapshot.rs`, `crates/render/src/lib.rs`, `crates/render/tests/headless.rs`, `crates/render/tests/snapshot.rs`, `crates/api/src/headless.rs`, `crates/app/src/host.rs`, `crates/app/src/app.rs`

**Interfaces:**
- Consumes: `Document::selection` (Task 2).
- Produces:
  - `render::scene::model_geometry(model, selection: &BTreeSet<EntityId>)`.
  - `FaceVertex { pos, normal, selected: f32 }`.
  - `SELECTED_EDGE_COLOR`.
  - `Renderer::set_model(&mut self, model: &Model, selection: &BTreeSet<EntityId>)`.
  - `render::Viewport { x, y, width, height }` (physical pixels, origin top-left) with `Viewport::full(size)`.
  - `Renderer::render_in(target, size, viewport, camera, overlay)`: clears the whole target and draws the scene only inside `viewport`, clamped. `render(...)` is `render_in` over the full target.
  - `Snapshotter::capture(model, selection, camera, width, height)`.

- [ ] **Step 1: Write the failing tests**

In `crates/render/tests/headless.rs`:
- **Imports:**
```rust
use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{EntityId, Model};
use render::{Renderer, Viewport};
```
- **Helpers.** Replace `render_scene` with these two functions:
```rust
fn render_scene(camera: &Camera, model: &Model, overlay: &[LineVertex]) -> Option<Vec<u8>> {
    render_with(
        camera,
        model,
        &BTreeSet::new(),
        Viewport::full((W, H)),
        overlay,
    )
}

/// Renders `model` with `selection` highlighted into `viewport` of a W x H target.
fn render_with(
    camera: &Camera,
    model: &Model,
    selection: &BTreeSet<EntityId>,
    viewport: Viewport,
    overlay: &[LineVertex],
) -> Option<Vec<u8>> {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut renderer = Renderer::new(&device, &queue, format);
    renderer.set_model(model, selection);
    renderer.render_in(&view, (W, H), viewport, camera, overlay);

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (W * H * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    let pixels = buffer
        .slice(..)
        .get_mapped_range()
        .expect("mapped range")
        .to_vec();
    Some(pixels)
}
```
- **Tests.** Append:
```rust
#[test]
fn selected_faces_are_tinted_blue() {
    let cam = camera();
    let model = ground_square();
    let face = model.faces().next().unwrap().0;
    let selection = BTreeSet::from([EntityId::Face(face)]);
    let Some(px) = render_with(&cam, &model, &selection, Viewport::full((W, H)), &[]) else {
        return;
    };
    let p = pixel_at(&px, &cam, DVec3::new(0.5, 0.5, 0.0));
    assert!(
        p[2] as i32 > p[0] as i32 + 30,
        "selected face should be blue, got {p:?}"
    );
}

#[test]
fn scene_stays_inside_its_viewport() {
    // The left half holds the scene, sized to match; the right half is only background.
    let mut cam = camera();
    cam.viewport = DVec2::new((W / 2) as f64, H as f64);
    let viewport = Viewport {
        x: 0,
        y: 0,
        width: W / 2,
        height: H,
    };
    let Some(px) = render_with(&cam, &ground_square(), &BTreeSet::new(), viewport, &[]) else {
        return;
    };
    let expected = BACKGROUND.map(|c| (c * 255.0).round() as i32);
    for y in (0..H).step_by(7) {
        for x in (W / 2..W).step_by(5) {
            let p = pixel(&px, x, y);
            assert!(
                (0..3).all(|ch| (p[ch] as i32 - expected[ch]).abs() <= 1),
                "pixel {x},{y} = {p:?} outside the viewport"
            );
        }
    }
    let face = cam.project(DVec3::new(0.5, 0.5, 0.0)).unwrap();
    assert!(face.x < (W / 2) as f64);
    assert!(grayish(pixel(&px, face.x as u32, face.y as u32)));
}
```

In `crates/render/tests/snapshot.rs`:
- add `use std::collections::BTreeSet;` above the other imports;
- both `capture(` calls gain `&BTreeSet::new(),` as the second argument.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p render`
Expected: compile errors (`Viewport`, `render_in`, `set_model` arity, `capture` arity).

- [ ] **Step 3: Implement**

Replace `crates/render/src/scene.rs` with:
```rust
//! GPU vertex data built from a kernel model.

use std::collections::BTreeSet;

use bytemuck::{Pod, Zeroable};
use kernel::{EntityId, Model};

use crate::grid::LineVertex;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct FaceVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// 1 for selected faces, 0 otherwise.
    pub selected: f32,
}

pub const EDGE_COLOR: [f32; 4] = [0.08, 0.08, 0.08, 1.0];
pub const SELECTED_EDGE_COLOR: [f32; 4] = [0.10, 0.35, 0.95, 1.0];

/// Triangles for every face and line pairs for every edge.
#[derive(Debug, Default, PartialEq)]
pub struct ModelGeometry {
    pub faces: Vec<FaceVertex>,
    pub edges: Vec<LineVertex>,
}

/// Geometry for `model`, with the entities in `selection` highlighted.
pub fn model_geometry(model: &Model, selection: &BTreeSet<EntityId>) -> ModelGeometry {
    let mut g = ModelGeometry::default();
    for (id, face) in model.faces() {
        let normal = face.normal.as_vec3().to_array();
        let selected = if selection.contains(&EntityId::Face(id)) {
            1.0
        } else {
            0.0
        };
        for tri in model.triangulate(id).unwrap_or_default() {
            for p in tri {
                g.faces.push(FaceVertex {
                    pos: p.as_vec3().to_array(),
                    normal,
                    selected,
                });
            }
        }
    }
    for (id, edge) in model.edges() {
        let color = if selection.contains(&EntityId::Edge(id)) {
            SELECTED_EDGE_COLOR
        } else {
            EDGE_COLOR
        };
        for v in edge.v {
            if let Some(p) = model.position(v) {
                g.edges.push(LineVertex {
                    pos: p.as_vec3().to_array(),
                    color,
                });
            }
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn square() -> Model {
        let mut m = Model::new();
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        m.add_polygon(&pts).unwrap();
        m
    }

    #[test]
    fn empty_model_has_no_geometry() {
        assert_eq!(
            model_geometry(&Model::new(), &BTreeSet::new()),
            ModelGeometry::default()
        );
    }

    #[test]
    fn square_gives_two_triangles_and_four_edges() {
        let g = model_geometry(&square(), &BTreeSet::new());
        assert_eq!(g.faces.len(), 6);
        assert!(
            g.faces
                .iter()
                .all(|v| v.normal == [0.0, 0.0, 1.0] && v.selected == 0.0)
        );
        assert_eq!(g.edges.len(), 8);
        assert!(g.edges.iter().all(|v| v.color == EDGE_COLOR));
    }

    #[test]
    fn selected_entities_are_marked() {
        let m = square();
        let face = m.faces().next().unwrap().0;
        let edge = m.edges().next().unwrap().0;
        let selection = BTreeSet::from([EntityId::Face(face), EntityId::Edge(edge)]);
        let g = model_geometry(&m, &selection);
        assert!(g.faces.iter().all(|v| v.selected == 1.0));
        let blue = g
            .edges
            .iter()
            .filter(|v| v.color == SELECTED_EDGE_COLOR)
            .count();
        assert_eq!(blue, 2, "one edge, two vertices");
    }
}
```

`crates/render/src/renderer.rs`:
- **Imports.** `use std::collections::BTreeSet;` and `use kernel::{EntityId, Model};`.
- **Face attributes.** `FACE_ATTRIBS` becomes:
```rust
const FACE_ATTRIBS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32];
```
- **Viewport type.** Add before `/// Multisampled color and depth attachments`:
```rust
/// A rectangle of the render target in physical pixels, origin top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    /// The whole target.
    pub fn full(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        }
    }
}
```
- **Replace `set_model` with:**
```rust
    /// Rebuilds the face and edge buffers from the model, highlighting `selection`. Call when
    /// the model or the selection changes.
    pub fn set_model(&mut self, model: &Model, selection: &BTreeSet<EntityId>) {
        let g = model_geometry(model, selection);
        self.faces = mesh(&self.device, "faces", &g.faces);
        self.edges = mesh(&self.device, "edges", &g.edges);
    }
```
- **Replace `render` with `render` plus `render_in`:**
```rust
    /// Draws grid, axes, model and `overlay` (line pairs drawn on top of everything) over the
    /// whole target.
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        camera: &Camera,
        overlay: &[LineVertex],
    ) {
        self.render_in(target, size, Viewport::full(size), camera, overlay);
    }

    /// Like `render`, but the scene fills only `viewport`; the rest of the target is cleared to
    /// the background. `camera.viewport` should match the viewport's size.
    pub fn render_in(
        &mut self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        viewport: Viewport,
        camera: &Camera,
        overlay: &[LineVertex],
    ) {
        if self.targets.as_ref().map(|t| t.size) != Some(size) {
            self.targets = Some(self.make_targets(size));
        }
        let targets = self.targets.as_ref().expect("targets just created");

        let eye = camera.eye.as_vec3();
        let uniforms = Uniforms {
            view_proj: camera.view_proj_f32().to_cols_array(),
            eye: [eye.x, eye.y, eye.z, 1.0],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let overlay = mesh(&self.device, "overlay", overlay);

        let [r, g, b, a] = BACKGROUND.map(f64::from);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa,
                    depth_slice: None,
                    resolve_target: Some(target),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            // Clamp the viewport to the target; an empty one leaves just the cleared background.
            let (x, y) = (viewport.x.min(size.0), viewport.y.min(size.1));
            let (w, h) = (
                viewport.width.min(size.0 - x),
                viewport.height.min(size.1 - y),
            );
            if w > 0 && h > 0 {
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                pass.set_scissor_rect(x, y, w, h);
                pass.set_bind_group(0, &self.bind_group, &[]);
                let layers = [
                    (&self.flat_ground_pipeline, Some(&self.flat_ground)),
                    (&self.ground_pipeline, Some(&self.vertical_axis)),
                    (&self.face_pipeline, self.faces.as_ref()),
                    (&self.edge_pipeline, self.edges.as_ref()),
                    (&self.overlay_pipeline, overlay.as_ref()),
                ];
                for (pipeline, mesh) in layers {
                    if let Some(mesh) = mesh {
                        pass.set_pipeline(pipeline);
                        pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                        pass.draw(0..mesh.count, 0..1);
                    }
                }
            }
        }
        self.queue.submit([encoder.finish()]);
    }
```

`crates/render/src/lib.rs`: `pub use renderer::{Renderer, Viewport};`.

`crates/render/src/shaders/scene.wgsl`:
- `FaceOut` gains `@location(2) selected: f32,`.
- Add `const SELECTED: vec3<f32> = vec3<f32>(0.25, 0.45, 0.95);` after `BACK`.
- Replace `vs_face` and `fs_face` with:
```wgsl
@vertex
fn vs_face(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) selected: f32,
) -> FaceOut {
    var out: FaceOut;
    out.pos = u.view_proj * vec4<f32>(pos, 1.0);
    out.world = pos;
    out.normal = normal;
    out.selected = selected;
    return out;
}

// Front sides are light, back sides blue-gray; a headlight at the eye shades both. Selected
// faces are tinted blue.
@fragment
fn fs_face(in: FaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(in.normal) * select(-1.0, 1.0, front);
    let to_eye = normalize(u.eye.xyz - in.world);
    let shade = 0.6 + 0.4 * max(dot(n, to_eye), 0.0);
    let base = mix(select(BACK, FRONT, front), SELECTED, 0.5 * in.selected);
    return vec4<f32>(base * shade, 1.0);
}
```

`crates/render/src/snapshot.rs`:
- imports: `use std::collections::BTreeSet;` and `use kernel::{EntityId, Model};`;
- replace `capture`'s doc comment and signature with:
```rust
    /// Renders `model` as seen by `camera` (its viewport is replaced by the image size) into
    /// tightly packed RGBA8 pixels, top row first, with `selection` highlighted. None if the
    /// GPU readback fails.
    pub fn capture(
        &mut self,
        model: &Model,
        selection: &BTreeSet<EntityId>,
        camera: &Camera,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
```
- replace its `self.renderer.set_model(model);` with `self.renderer.set_model(model, selection);`.

`crates/api/src/headless.rs`, in `render_view`:
```rust
        let (model, selection) = (self.doc.model(), self.doc.selection());
        self.snapshotter
            .as_mut()
            .and_then(|s| s.capture(model, selection, camera, width, height))
```

`crates/app/src/host.rs`, `render_view` becomes:
```rust
    fn render_view(&mut self, camera: &Camera, width: u32, height: u32) -> Option<Vec<u8>> {
        let doc = self.editor.doc();
        self.snapshotter
            .capture(doc.model(), doc.selection(), camera, width, height)
    }
```

`crates/app/src/app.rs`: in `render`, the model upload becomes `self.renderer.set_model(self.editor.doc().model(), self.editor.doc().selection());`. Task 7 also re-uploads when the selection alone changes.

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 284 (scene +1, headless +2).

- [ ] **Step 5: Commit**

```bash
git add crates/render/src/scene.rs crates/render/src/renderer.rs crates/render/src/shaders/scene.wgsl crates/render/src/snapshot.rs crates/render/src/lib.rs crates/render/tests/headless.rs crates/render/tests/snapshot.rs crates/api/src/headless.rs crates/app/src/host.rs crates/app/src/app.rs
git commit -m "Highlight the selection and draw the scene into a viewport"
```

---
### Task 4: Tool events for release, Shift and picking

**Files:**
- Modify: `crates/tools/src/tool.rs`, `crates/tools/src/inference.rs`, `crates/tools/src/lib.rs`, `crates/tools/src/line.rs`, `crates/tools/src/rectangle.rs`, `crates/tools/src/push_pull.rs`, `crates/app/src/editor.rs`

**Interfaces:**
- Produces:
  - `ToolEvent::Release(Inference)`: the primary button went up. Line, Rectangle and Push/Pull ignore it.
  - `ToolContext` gains `pick_radius: f64` (physical pixels) and `shift: bool`.
  - `tools::edge_under(model, camera, cursor, radius_px) -> Option<EdgeId>`: the visible edge nearest the cursor on screen within the radius.
  - The editor tracks Shift and sends `Release` on left-button up. Its tool context carries `pick_radius = 8 logical px * scale` and `shift`.

- [ ] **Step 1: Write the failing test**

In `crates/tools/src/inference.rs` tests, add at the end of the module:
```rust
    #[test]
    fn edge_under_picks_the_nearest_visible_edge() {
        let (c, m) = (cam(), cube());
        let edge_at = |a: DVec3, b: DVec3| {
            m.edges()
                .find(|(_, e)| {
                    let (p, q) = (m.position(e.v[0]).unwrap(), m.position(e.v[1]).unwrap());
                    (p == a && q == b) || (p == b && q == a)
                })
                .unwrap()
                .0
        };
        let near_top = edge_at(DVec3::new(0.0, 0.0, 2.0), DVec3::new(2.0, 0.0, 2.0));
        let cursor = px(&c, DVec3::new(1.0, 0.0, 2.0)) + DVec2::new(0.0, 3.0);
        assert_eq!(edge_under(&m, &c, cursor, R), Some(near_top));
        // The far bottom edge lies behind the box.
        let far_bottom = edge_at(DVec3::new(0.0, 2.0, 0.0), DVec3::new(2.0, 2.0, 0.0));
        let behind = px(&c, DVec3::new(1.0, 2.0, 0.0));
        assert_ne!(edge_under(&m, &c, behind, R), Some(far_bottom));
        assert_eq!(
            edge_under(&m, &c, px(&c, DVec3::new(-3.0, 4.0, 0.0)), R),
            None
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools edge_under`
Expected: compile error: `edge_under` not found.

- [ ] **Step 3: Implement**

`crates/tools/src/tool.rs`:
- `ToolContext` gains, after `cursor`:
```rust
    /// How close (physical pixels) the cursor must be to an edge to pick it.
    pub pick_radius: f64,
    /// Whether Shift is held.
    pub shift: bool,
```
- `ToolEvent` gains, after `Click`:
```rust
    /// The primary button was released at the current inference.
    Release(Inference),
```

`crates/tools/src/inference.rs`:
- the kernel import becomes `use kernel::{EdgeId, FaceId, Model};`;
- add before `fn visible`:
```rust
/// The visible edge nearest the cursor on screen, if one lies within `radius_px`.
pub fn edge_under(model: &Model, camera: &Camera, cursor: DVec2, radius_px: f64) -> Option<EdgeId> {
    let ray = camera.ray(cursor);
    let mut best: Option<(f64, EdgeId)> = None;
    for (id, e) in model.edges() {
        let (Some(a), Some(b)) = (model.position(e.v[0]), model.position(e.v[1])) else {
            continue;
        };
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

`crates/tools/src/lib.rs`: `pub use inference::{Axis, Inference, SnapKind, edge_under, face_under, infer};`.

In `line.rs`, `rectangle.rs` and `push_pull.rs`:
- add the arm `ToolEvent::Release(_) => {}` before the `ToolEvent::Cancel` arm of `handle`;
- in each test `ToolContext { ... }` literal, add `pick_radius: 10.0,` and `shift: false,` after `cursor`.

`crates/app/src/editor.rs`:
- **Shift state.** `Editor` gains `shift: bool` after `tool`, initialized `shift: false`. `set_modifiers` stores it: `self.shift = shift;` before `self.nav.set_modifiers(shift, alt);`.
- **Release.** In `button`, replace everything after the navigation check with:
```rust
        if button != Button::Left {
            return false;
        }
        self.update_hover();
        let Some(hit) = self.hit else { return false };
        self.send(if pressed {
            ToolEvent::Click(hit)
        } else {
            ToolEvent::Release(hit)
        });
        true
```
- **Tool context.** The `ToolContext` built in `handle` gains:
```rust
            pick_radius: PICK_RADIUS_LOGICAL_PX * self.scale_factor,
            shift: self.shift,
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 285.

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/tool.rs crates/tools/src/inference.rs crates/tools/src/lib.rs crates/tools/src/line.rs crates/tools/src/rectangle.rs crates/tools/src/push_pull.rs crates/app/src/editor.rs
git commit -m "Give tools release events, Shift state and edge picking"
```

---

### Task 5: Select and Erase tools

**Files:**
- Create: `crates/tools/src/select.rs`, `crates/tools/src/erase.rs`
- Modify: `crates/tools/src/tool.rs`, `crates/tools/src/lib.rs`, `crates/app/src/app.rs`

**Interfaces:**
- Consumes: `ToolEvent::Release`, `ToolContext.{pick_radius, shift}`, `edge_under`, `face_under` (Task 4); `Command::{Select, Erase}`, `SelectMode` (Task 2).
- Produces:
  - `SelectTool`, `EraseTool`.
  - `pub(crate) fn pick(ctx) -> Option<EntityId>`: the edge under the cursor, else the face.
  - `pub(crate) fn face_outline(model, face)`.
  - `ToolKind::{Select, Line, Rectangle, PushPull, Erase}`, with `ToolKind::ALL` in toolbar order, `label()` (equal to `Tool::name`), `shortcut()` (`' '`, `l`, `r`, `p`, `e`) and `from_shortcut` (case-insensitive).
  - On non-ASCII layouts the window's physical-key fallback also maps `KeyE` to `'e'`.

- [ ] **Step 1: Write the failing tests**

Create `crates/tools/src/select.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use session::Camera;

    /// A 2 m square on the ground (face f8) seen by the default camera.
    struct Rig {
        tool: SelectTool,
        model: Model,
        camera: Camera,
        cursor: DVec2,
        shift: bool,
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
            Self {
                tool: SelectTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
                shift: false,
            }
        }

        fn px(&self, p: DVec3) -> DVec2 {
            self.camera.project(p).unwrap()
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                shift: self.shift,
            };
            self.tool.handle(event, &ctx)
        }

        fn hit() -> Inference {
            Inference {
                point: DVec3::ZERO,
                kind: SnapKind::Free,
                face: None,
            }
        }

        /// Presses at `a`, moves to `b` and releases there.
        fn drag(&mut self, a: DVec2, b: DVec2) -> ToolOutput {
            self.cursor = a;
            self.send(ToolEvent::Click(Self::hit()));
            self.cursor = b;
            self.send(ToolEvent::Move(Self::hit()));
            self.send(ToolEvent::Release(Self::hit()))
        }

        fn click(&mut self, p: DVec3) -> ToolOutput {
            let at = self.px(p);
            self.drag(at, at)
        }
    }

    fn select(ids: &[&str], mode: SelectMode) -> Vec<Command> {
        vec![Command::Select {
            ids: ids.iter().map(|s| s.parse().unwrap()).collect(),
            mode,
        }]
    }

    #[test]
    fn clicking_picks_edges_before_faces() {
        let mut r = Rig::new();
        let edge = r.click(DVec3::new(1.0, 0.0, 0.0));
        assert_eq!(edge.commands, select(&["e2"], SelectMode::Replace));
        let face = r.click(DVec3::new(1.0, 1.0, 0.0));
        assert_eq!(face.commands, select(&["f8"], SelectMode::Replace));
    }

    #[test]
    fn clicking_nothing_clears_unless_shift_is_held() {
        let mut r = Rig::new();
        let off = DVec3::new(-3.0, 4.0, 0.0);
        assert_eq!(r.click(off).commands, select(&[], SelectMode::Replace));
        r.shift = true;
        assert!(r.click(off).commands.is_empty());
        let face = r.click(DVec3::new(1.0, 1.0, 0.0));
        assert_eq!(face.commands, select(&["f8"], SelectMode::Toggle));
    }

    #[test]
    fn dragging_right_selects_what_is_inside_the_box() {
        let mut r = Rig::new();
        let pts: Vec<DVec2> = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]
            .iter()
            .map(|&(x, y)| r.px(DVec3::new(x, y, 0.0)))
            .collect();
        let lo = pts.iter().fold(DVec2::INFINITY, |a, p| a.min(*p)) - 5.0;
        let hi = pts.iter().fold(DVec2::NEG_INFINITY, |a, p| a.max(*p)) + 5.0;
        let all = r.drag(lo, hi);
        assert_eq!(
            all.commands,
            select(&["e2", "e4", "e6", "e7", "f8"], SelectMode::Replace)
        );
        // A box around only the edge from (0,0) to (2,0) holds just that edge.
        let (a, b) = (pts[0].min(pts[1]) - 5.0, pts[0].max(pts[1]) + 5.0);
        let one = r.drag(a, b);
        assert_eq!(one.commands, select(&["e2"], SelectMode::Replace));
    }

    #[test]
    fn dragging_left_selects_what_the_box_touches() {
        let mut r = Rig::new();
        // A small box around the middle of the edge from (0,0) to (2,0), dragged right to left.
        let mid = r.px(DVec3::new(1.0, 0.0, 0.0));
        let (a, b) = (mid + DVec2::new(6.0, -6.0), mid + DVec2::new(-6.0, 6.0));
        let out = r.drag(a, b);
        assert_eq!(out.commands, select(&["e2", "f8"], SelectMode::Replace));
        let window = r.drag(b, a);
        assert_eq!(window.commands, select(&[], SelectMode::Replace));
    }

    #[test]
    fn dragging_draws_a_box_until_release() {
        let mut r = Rig::new();
        r.cursor = DVec2::new(100.0, 100.0);
        r.send(ToolEvent::Click(Rig::hit()));
        r.cursor = DVec2::new(200.0, 160.0);
        let out = r.send(ToolEvent::Move(Rig::hit()));
        assert_eq!(out.preview.len(), 4);
        let corner = r.camera.project(out.preview[0][0]).unwrap();
        assert!(corner.distance(DVec2::new(100.0, 100.0)) < 1e-6);
        let cancelled = r.send(ToolEvent::Cancel);
        assert!(cancelled.preview.is_empty());
        assert!(r.send(ToolEvent::Release(Rig::hit())).commands.is_empty());
    }
}
```

Create `crates/tools/src/erase.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;

    fn hit() -> Inference {
        Inference {
            point: DVec3::ZERO,
            kind: SnapKind::Free,
            face: None,
        }
    }

    fn send(
        tool: &mut EraseTool,
        model: &Model,
        camera: &Camera,
        at: DVec3,
        event: ToolEvent,
    ) -> ToolOutput {
        let ctx = ToolContext {
            model,
            camera,
            cursor: camera.project(at).unwrap(),
            pick_radius: 10.0,
            shift: false,
        };
        tool.handle(event, &ctx)
    }

    #[test]
    fn hovering_outlines_and_clicking_erases() {
        let mut m = Model::new();
        let pts = [
            DVec3::ZERO,
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(2.0, 2.0, 0.0),
            DVec3::new(0.0, 2.0, 0.0),
        ];
        m.add_polygon(&pts).unwrap();
        let cam = Camera::new(DVec2::new(800.0, 600.0));
        let mut t = EraseTool::new();
        let edge = DVec3::new(1.0, 0.0, 0.0);
        assert_eq!(
            send(&mut t, &m, &cam, edge, ToolEvent::Move(hit()))
                .preview
                .len(),
            1
        );
        let inside = DVec3::new(1.0, 1.0, 0.0);
        assert_eq!(
            send(&mut t, &m, &cam, inside, ToolEvent::Move(hit()))
                .preview
                .len(),
            4
        );
        let out = send(&mut t, &m, &cam, inside, ToolEvent::Click(hit()));
        assert_eq!(
            out.commands,
            vec![Command::Erase {
                ids: vec!["f8".parse().unwrap()]
            }]
        );
        let off = DVec3::new(-3.0, 4.0, 0.0);
        assert!(
            send(&mut t, &m, &cam, off, ToolEvent::Click(hit()))
                .commands
                .is_empty()
        );
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
pub mod overlay;
pub mod push_pull;
pub mod rectangle;
pub mod select;
pub mod tool;
pub mod vcb;

pub use erase::EraseTool;
pub use inference::{Axis, Inference, SnapKind, edge_under, face_under, infer};
pub use line::LineTool;
pub use push_pull::PushPullTool;
pub use rectangle::RectangleTool;
pub use select::SelectTool;
pub use tool::{Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p tools`
Expected: compile errors (`SelectTool`, `EraseTool`, `pick`, `face_outline` not found).

- [ ] **Step 3: Implement**

Prepend to `crates/tools/src/select.rs`:
```rust
//! The Select tool: click an edge or face to select it, or drag a box. Dragging right selects
//! what lies entirely inside the box; dragging left also selects what the box touches. With
//! Shift held, clicked or boxed entities are toggled instead of replacing the selection.

use glam::{DVec2, DVec3};
use kernel::{EntityId, Face, Model};
use session::{Command, SelectMode};

use crate::inference::{edge_under, face_under};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};

/// How far (physical pixels) the cursor must move between press and release to make a box.
const DRAG_PX: f64 = 4.0;

#[derive(Clone, Debug, Default)]
pub struct SelectTool {
    /// Cursor position where the button went down, while it is held.
    press: Option<DVec2>,
    cursor: DVec2,
}

impl SelectTool {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The edge under the cursor, or else the face under it.
pub(crate) fn pick(ctx: &ToolContext) -> Option<EntityId> {
    edge_under(ctx.model, ctx.camera, ctx.cursor, ctx.pick_radius)
        .map(EntityId::Edge)
        .or_else(|| face_under(ctx.model, ctx.camera, ctx.cursor).map(|(f, _)| EntityId::Face(f)))
}

/// Edges and faces selected by the screen box from `a` to `b`: inside it entirely when
/// dragging right (`b.x >= a.x`), touching it when dragging left. Entities with a point behind
/// the camera are skipped.
fn boxed(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<EntityId> {
    let (lo, hi) = (a.min(b), a.max(b));
    let window = b.x >= a.x;
    let m = ctx.model;
    let screen = |points: &[kernel::VertexId]| -> Option<Vec<DVec2>> {
        points
            .iter()
            .map(|v| m.position(*v).and_then(|p| ctx.camera.project(p)))
            .collect()
    };
    let inside = |s: DVec2| s.cmpge(lo).all() && s.cmple(hi).all();
    let touches = |p: DVec2, q: DVec2| {
        let corners = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
        inside(p)
            || inside(q)
            || (0..4).any(|i| segments_cross(p, q, corners[i], corners[(i + 1) % 4]))
    };
    let chosen = |closed: bool, pts: &[DVec2]| {
        if window {
            pts.iter().all(|p| inside(*p))
        } else {
            let n = pts.len();
            let sides = if closed { n } else { n - 1 };
            (0..sides).any(|i| touches(pts[i], pts[(i + 1) % n]))
        }
    };
    let mut ids = Vec::new();
    for (id, e) in m.edges() {
        if screen(&e.v).is_some_and(|pts| chosen(false, &pts)) {
            ids.push(EntityId::Edge(id));
        }
    }
    for (id, f) in m.faces() {
        let all = face_loops(f).all(|l| screen(l).is_some_and(|pts| !pts.is_empty()));
        let hit = if window {
            all && face_loops(f).all(|l| screen(l).is_some_and(|pts| chosen(true, &pts)))
        } else {
            face_loops(f).any(|l| screen(l).is_some_and(|pts| chosen(true, &pts)))
        };
        if hit {
            ids.push(EntityId::Face(id));
        }
    }
    ids
}

fn face_loops(f: &Face) -> impl Iterator<Item = &Vec<kernel::VertexId>> {
    std::iter::once(&f.outer).chain(&f.holes)
}

/// True if the 2D segments p-q and r-s cross or touch.
fn segments_cross(p: DVec2, q: DVec2, r: DVec2, s: DVec2) -> bool {
    let orient = |a: DVec2, b: DVec2, c: DVec2| (b - a).perp_dot(c - a);
    let (d1, d2) = (orient(r, s, p), orient(r, s, q));
    let (d3, d4) = (orient(p, q, r), orient(p, q, s));
    d1 * d2 <= 0.0 && d3 * d4 <= 0.0 && (d1 != 0.0 || d2 != 0.0 || d3 != 0.0 || d4 != 0.0)
}

/// World-space segments that draw the screen box from `a` to `b`, just in front of the camera.
fn rubber_band(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<[DVec3; 2]> {
    let depth = ctx.camera.distance() * 0.5;
    let at = |px: DVec2| {
        let ray = ctx.camera.ray(px);
        ray.origin + ray.dir * depth
    };
    let c = [a, DVec2::new(b.x, a.y), b, DVec2::new(a.x, b.y)].map(at);
    (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect()
}

impl Tool for SelectTool {
    fn name(&self) -> &'static str {
        "Select"
    }

    fn prompt(&self) -> &'static str {
        match self.press {
            None => "Click an edge or face, or drag a box; Shift adds or removes",
            Some(_) => "Release to select: drag right for inside the box, left for touching",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        self.cursor = ctx.cursor;
        match event {
            ToolEvent::Move(_) => {}
            ToolEvent::Click(_) => self.press = Some(ctx.cursor),
            ToolEvent::Release(_) => {
                if let Some(start) = self.press.take() {
                    let ids = if start.distance(ctx.cursor) < DRAG_PX {
                        pick(ctx).into_iter().collect()
                    } else {
                        boxed(ctx, start, ctx.cursor)
                    };
                    let mode = if ctx.shift {
                        SelectMode::Toggle
                    } else {
                        SelectMode::Replace
                    };
                    // Shift with nothing picked leaves the selection alone.
                    if !(ctx.shift && ids.is_empty()) {
                        out.commands.push(Command::Select { ids, mode });
                    }
                }
            }
            ToolEvent::Value(_) => {
                out.message = Some("The Select tool takes no typed values".into());
            }
            ToolEvent::Cancel => self.press = None,
        }
        if let Some(start) = self.press
            && start.distance(self.cursor) >= DRAG_PX
        {
            out.preview = rubber_band(ctx, start, self.cursor);
        }
        out
    }

    fn on_external_change(&mut self) {
        self.press = None;
    }
}

/// Positions of the vertices of a face's loops, for outlines.
pub(crate) fn face_outline(model: &Model, f: &Face) -> Vec<[DVec3; 2]> {
    let mut out = Vec::new();
    for l in face_loops(f) {
        let pts: Vec<DVec3> = l.iter().filter_map(|v| model.position(*v)).collect();
        for i in 0..pts.len() {
            out.push([pts[i], pts[(i + 1) % pts.len()]]);
        }
    }
    out
}
```

The box tests check each entity's projected points: all inside the box (window) or any side touching it (crossing). This is screen-space work only; no occlusion is applied.

Prepend to `crates/tools/src/erase.rs`:
```rust
//! The Erase tool: click an edge or face to erase it. The entity under the cursor is outlined.

use glam::DVec3;
use kernel::EntityId;
use session::Command;

use crate::select::{face_outline, pick};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};

#[derive(Clone, Debug, Default)]
pub struct EraseTool {
    /// Outline of the entity under the cursor.
    outline: Vec<[DVec3; 2]>,
}

impl EraseTool {
    pub fn new() -> Self {
        Self::default()
    }
}

fn outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
    let m = ctx.model;
    match id {
        EntityId::Edge(e) => m
            .edge(e)
            .and_then(|e| Some([m.position(e.v[0])?, m.position(e.v[1])?]))
            .into_iter()
            .collect(),
        EntityId::Face(f) => m.face(f).map(|f| face_outline(m, f)).unwrap_or_default(),
        EntityId::Vertex(_) => Vec::new(),
    }
}

impl Tool for EraseTool {
    fn name(&self) -> &'static str {
        "Erase"
    }

    fn prompt(&self) -> &'static str {
        "Click an edge or face to erase it"
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
                self.outline = pick(ctx).map(|id| outline(ctx, id)).unwrap_or_default()
            }
            ToolEvent::Click(_) => {
                if let Some(id) = pick(ctx) {
                    out.commands.push(Command::Erase { ids: vec![id] });
                }
                self.outline.clear();
            }
            ToolEvent::Value(_) => {
                out.message = Some("The Erase tool takes no typed values".into());
            }
            ToolEvent::Release(_) | ToolEvent::Cancel => {}
        }
        out.preview = self.outline.clone();
        out
    }

    fn on_external_change(&mut self) {
        self.outline.clear();
    }
}
```

Replace `crates/tools/src/tool.rs` with:
```rust
//! What every interactive tool implements, and the tools the app can switch between.

use glam::{DVec2, DVec3};
use kernel::Model;
use session::{Camera, Command};

use crate::erase::EraseTool;
use crate::inference::Inference;
use crate::line::LineTool;
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
    /// Whether Shift is held.
    pub shift: bool,
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
    Erase,
}

impl ToolKind {
    /// Every tool, in toolbar order.
    pub const ALL: [ToolKind; 5] = [
        ToolKind::Select,
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::PushPull,
        ToolKind::Erase,
    ];

    pub fn make(self) -> Box<dyn Tool> {
        match self {
            ToolKind::Select => Box::new(SelectTool::new()),
            ToolKind::Line => Box::new(LineTool::new()),
            ToolKind::Rectangle => Box::new(RectangleTool::new()),
            ToolKind::PushPull => Box::new(PushPullTool::new()),
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

`crates/app/src/app.rs`, in `physical_char`: add `KeyCode::KeyE => 'e',` after `KeyCode::KeyP => 'p',`.

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 292 (select 5, erase 1, tool 1).

- [ ] **Step 5: Commit**

```bash
git add crates/tools/src/select.rs crates/tools/src/erase.rs crates/tools/src/tool.rs crates/tools/src/lib.rs crates/app/src/app.rs
git commit -m "Add the Select and Erase tools"
```

---

### Task 6: Erase the selection from the keyboard

**Files:**
- Modify: `crates/app/src/editor.rs`, `crates/app/src/app.rs`

**Interfaces:**
- Consumes: `Command::Erase`, `Document::selection` (Task 2); `ToolKind` shortcuts (Task 5).
- Produces:
  - `EditorKey::Delete`.
  - `Editor::erase_selection(&mut self) -> bool`: erases the selection as one undo step; false when nothing is selected.
  - `Delete` erases the selection, and so does `Backspace` when nothing is typed. With text typed, Backspace still deletes a character.
  - The window maps `NamedKey::Delete` to `EditorKey::Delete`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/editor.rs` tests, add at the end of the module:
```rust
    #[test]
    fn select_tool_clicks_pick_and_delete_erases_them() {
        let mut e = editor();
        square(&mut e);
        assert!(e.key(EditorKey::Char(' ')));
        assert!(e.status().starts_with("Select"), "{}", e.status());
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert_eq!(e.doc().selection().len(), 1);
        assert!(e.key(EditorKey::Delete));
        assert_eq!(stats(&e), (4, 0));
        assert!(e.doc().selection().is_empty());
        e.undo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn backspace_erases_the_selection_only_when_nothing_is_typed() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('1'));
        assert!(e.key(EditorKey::Backspace));
        assert_eq!(
            stats(&e),
            (4, 1),
            "the first backspace deletes the typed digit"
        );
        assert!(e.key(EditorKey::Backspace));
        assert_eq!(stats(&e), (4, 0));
        assert!(!e.key(EditorKey::Delete), "nothing left to erase");
    }

    #[test]
    fn dragging_a_box_selects_what_is_inside() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        let corners: Vec<DVec2> = [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)]
            .iter()
            .map(|&(x, y)| e.doc().camera.project(DVec3::new(x, y, 0.0)).unwrap())
            .collect();
        let lo = corners.iter().fold(DVec2::INFINITY, |a, p| a.min(*p)) - 10.0;
        let hi = corners.iter().fold(DVec2::NEG_INFINITY, |a, p| a.max(*p)) + 10.0;
        e.cursor_moved(lo);
        e.button(Button::Left, true);
        e.cursor_moved(hi);
        assert_eq!(e.overlay().len(), 8, "the box is drawn while dragging");
        e.button(Button::Left, false);
        assert_eq!(e.doc().selection().len(), 5, "four edges and the face");
    }

    #[test]
    fn erase_tool_click_erases_an_edge_and_its_face() {
        let mut e = editor();
        square(&mut e);
        assert!(e.key(EditorKey::Char('e')));
        click_at(&mut e, DVec3::new(3.0, 2.0, 0.0));
        assert_eq!(stats(&e), (3, 0));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app`
Expected: compile error (`EditorKey::Delete` not found).

- [ ] **Step 3: Implement**

`crates/app/src/editor.rs`:
- `EditorKey` gains `Delete` after `Backspace`.
- `key`'s doc comment gains: "Delete, and Backspace with nothing typed, erase the selection."
- Its `EditorKey::Backspace => self.typed.pop().is_some(),` arm becomes:
```rust
            EditorKey::Backspace if self.typed.is_empty() => self.erase_selection(),
            EditorKey::Backspace => self.typed.pop().is_some(),
            EditorKey::Delete => self.erase_selection(),
```
- Add before `pub fn undo`:
```rust
    /// Erases the selected edges and faces as one undo step. False if nothing is selected.
    pub fn erase_selection(&mut self) -> bool {
        if self.doc.selection().is_empty() {
            return false;
        }
        let ids = self.doc.selection().iter().copied().collect();
        self.reset_tool();
        self.message = None;
        self.run(Command::Erase { ids });
        self.update_hover();
        true
    }
```

`crates/app/src/app.rs`, in `editor_key`: add `Key::Named(NamedKey::Delete) => Some(EditorKey::Delete),` after the Backspace arm.

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 296.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/editor.rs crates/app/src/app.rs
git commit -m "Erase the selection with Delete or Backspace"
```

---

### Task 7: egui shell

**Files:**
- Create: `crates/app/src/ui.rs`, `crates/app/src/info.rs`, `crates/app/src/gui.rs`
- Modify: `Cargo.toml`, `crates/app/Cargo.toml`, `crates/app/src/main.rs`, `crates/app/src/editor.rs`, `crates/app/src/app.rs`, `Cargo.lock`

**Interfaces:**
- Consumes: `render::Viewport`, `Renderer::render_in`, `set_model(model, selection)` (Task 3); `ToolKind::{ALL, label, shortcut}` (Task 5); `Editor::erase_selection` (Task 6).
- Produces:
  - **`editor.rs`:**
    - `Status { tool, prompt, label, value, message }`, which implements `Display` as the old one-line status.
    - `Editor::status_parts() -> Status`; `Editor::status()` is now test-only.
    - `Editor::tool_kind() -> ToolKind`.
  - **`ui.rs`:**
    - `UiAction::{Tool(ToolKind), Undo, Redo, EraseSelection}`.
    - `View { tool, status, info, has_selection }`.
    - `draw(ui, &View) -> (Vec<UiAction>, egui::Rect)`:
      - top toolbar: tool buttons with shortcut tooltips, Undo, Redo, Erase selection (disabled when nothing is selected);
      - bottom status bar: prompt, message in the warning color, and the measurements box on the right;
      - right Entity Info panel;
      - returns the remaining central rect in points.
  - **`info.rs`:** `entity_info(doc) -> Vec<String>`:
    - "Nothing selected";
    - one edge: "1 edge", "Length: 2 m";
    - one face: "1 face", "Area: 6 m²", "Normal: 0, 0, 1";
    - otherwise counts, e.g. "2 edges, 1 face".
  - **`gui.rs`:** `Gui` holds the egui context, egui-winit state and egui-wgpu renderer:
    - `on_event(window, event) -> bool` (repaint);
    - `wants_pointer()` and `wants_keyboard()`;
    - `frame(window, view) -> GuiFrame { actions, viewport }`;
    - `paint(window, device, queue, target, size, frame) -> bool` (repaint now).
  - **`app.rs`:**
    - The title is `sketchup-alt`.
    - Each frame lays out the panels, applies clicked actions, sizes the camera to the central rect, re-uploads model buffers when the revision or the selection changed, draws the scene with `render_in` into that rect, then paints egui with `LoadOp::Load`.
    - Cursor positions reach the editor relative to the view's origin. Outside the view the marker is hidden.
    - A press reaches the editor only if it lands in the view and not on a panel; releases always do.
    - The wheel is used only inside the view. Keys are skipped while egui wants the keyboard.
    - Edits made through panel buttons notify API subscribers.
    - API requests also redraw when only the selection changed.

- [ ] **Step 1: Dependencies**

Root `Cargo.toml`, `[workspace.dependencies]`, append:
```toml
egui = "=0.36.2"
egui-wgpu = "=0.36.2"
egui-winit = "=0.36.2"
```

`crates/app/Cargo.toml`:
```toml
[package]
name = "app"
version.workspace = true
edition.workspace = true

[dependencies]
render = { path = "../render" }
session = { path = "../session" }
tools = { path = "../tools" }
api = { path = "../api" }
glam.workspace = true
wgpu.workspace = true
winit.workspace = true
pollster.workspace = true
env_logger.workspace = true
egui.workspace = true
egui-wgpu.workspace = true
egui-winit.workspace = true
kernel = { path = "../kernel" }

[dev-dependencies]
serde_json.workspace = true
```

`crates/app/src/main.rs`: the module list becomes `mod app; mod cli; mod editor; mod gui; mod headless; mod host; mod info; mod navigation; mod ui;`, one per line in that order.

Run `cargo build -p app` once to fetch and lock the new crates. Run `cargo tree -i wgpu@30.0.1 --depth 1`: it should list `egui-wgpu`, and `Cargo.lock` must contain a single `name = "wgpu"` entry.

- [ ] **Step 2: Write the failing tests**

Create `crates/app/src/info.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;
    use session::{Command, SelectMode};

    fn doc_with_square() -> Document {
        let mut d = Document::new(DVec2::new(800.0, 600.0));
        d.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 3.0, 0.0],
            normal: None,
        })
        .unwrap();
        d
    }

    fn select(d: &mut Document, ids: &[&str]) {
        d.execute(Command::Select {
            ids: ids.iter().map(|s| s.parse().unwrap()).collect(),
            mode: SelectMode::Replace,
        })
        .unwrap();
    }

    #[test]
    fn describes_the_selection() {
        let mut d = doc_with_square();
        assert_eq!(entity_info(&d), ["Nothing selected"]);
        select(&mut d, &["f8"]);
        assert_eq!(entity_info(&d), ["1 face", "Area: 6 m²", "Normal: 0, 0, 1"]);
        select(&mut d, &["e2"]);
        assert_eq!(entity_info(&d), ["1 edge", "Length: 2 m"]);
        select(&mut d, &["e2", "e4", "f8"]);
        assert_eq!(entity_info(&d), ["2 edges, 1 face"]);
    }
}
```

Create `crates/app/src/ui.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2};

    fn run(ctx: &egui::Context, input: egui::RawInput, view: &View) -> (Vec<UiAction>, Rect) {
        let mut out = None;
        ctx.run_ui(input, |ui| out = Some(draw(ui, view)))
            .drop_without_applying_deltas();
        out.unwrap()
    }

    fn screen() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
            ..Default::default()
        }
    }

    fn status() -> Status {
        Status {
            tool: "Line",
            prompt: "Click to start a line",
            label: "Length",
            value: Some("2.5_".into()),
            message: Some("Not a length: x".into()),
        }
    }

    #[test]
    fn panels_leave_a_central_view_inside_the_window() {
        let ctx = egui::Context::default();
        let status = status();
        let info = vec!["Nothing selected".to_string()];
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: false,
        };
        let (actions, rect) = run(&ctx, screen(), &view);
        assert!(actions.is_empty());
        assert!(
            rect.min.x == 0.0 && rect.min.y > 0.0,
            "toolbar above: {rect:?}"
        );
        assert!(rect.max.x < 1000.0, "info panel on the right: {rect:?}");
        assert!(rect.max.y < 700.0, "status bar below: {rect:?}");
        assert!(rect.width() > 600.0 && rect.height() > 500.0, "{rect:?}");
    }

    #[test]
    fn clicking_a_tool_button_reports_it() {
        let ctx = egui::Context::default();
        let status = status();
        let info = Vec::new();
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: true,
        };
        let (_, central) = run(&ctx, screen(), &view);
        let row = central.min.y / 2.0;
        // Click along the toolbar row until a press lands on the Rectangle button.
        let mut clicked = Vec::new();
        for x in (2..600).step_by(4) {
            let pos = Pos2::new(x as f32, row);
            let pointer = |pressed: bool| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            let mut press = screen();
            press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
            let _ = run(&ctx, press, &view);
            let mut release = screen();
            release.events = vec![pointer(false)];
            clicked.extend(run(&ctx, release, &view).0);
        }
        assert!(
            clicked.contains(&UiAction::Tool(ToolKind::Rectangle)),
            "{clicked:?}"
        );
        assert!(clicked.contains(&UiAction::Undo));
        assert!(clicked.contains(&UiAction::EraseSelection));
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p app`
Expected: compile errors (`entity_info`, `draw`, `View`, `UiAction`, `Status` not found).

- [ ] **Step 4: Implement**

Prepend to `crates/app/src/info.rs`:
```rust
//! Text for the Entity Info panel: what is selected and its measurements.

use kernel::{EntityId, Model};
use session::Document;
use tools::vcb::format_length;

/// Lines describing the selection: counts, plus length, area or normal for a single entity.
pub fn entity_info(doc: &Document) -> Vec<String> {
    let m = doc.model();
    let (mut edges, mut faces) = (Vec::new(), Vec::new());
    for id in doc.selection() {
        match id {
            EntityId::Edge(e) => edges.push(*e),
            EntityId::Face(f) => faces.push(*f),
            EntityId::Vertex(_) => {}
        }
    }
    match (&edges[..], &faces[..]) {
        ([], []) => vec!["Nothing selected".into()],
        ([e], []) => vec![
            "1 edge".into(),
            format!("Length: {}", format_length(edge_length(m, *e))),
        ],
        ([], [f]) => {
            let face = m.face(*f).unwrap();
            let n = face.normal.to_array().map(number);
            vec![
                "1 face".into(),
                format!("Area: {} m²", number(m.area(*f).unwrap_or(0.0))),
                format!("Normal: {}, {}, {}", n[0], n[1], n[2]),
            ]
        }
        _ => {
            let mut parts = Vec::new();
            if !edges.is_empty() {
                parts.push(count(edges.len(), "edge"));
            }
            if !faces.is_empty() {
                parts.push(count(faces.len(), "face"));
            }
            vec![parts.join(", ")]
        }
    }
}

fn edge_length(m: &Model, e: kernel::EdgeId) -> f64 {
    let v = m.edge(e).unwrap().v;
    match (m.position(v[0]), m.position(v[1])) {
        (Some(a), Some(b)) => a.distance(b),
        _ => 0.0,
    }
}

fn count(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// Rounded to 3 decimals, without trailing zeros or a negative zero.
fn number(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}
```

Prepend to `crates/app/src/ui.rs`:
```rust
//! The window's panels: toolbar, status bar with the measurements box, and Entity Info.

use egui::{Align, Layout, Panel, RichText, Ui};
use tools::ToolKind;

use crate::editor::Status;

/// What the user asked for through the panels this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    Tool(ToolKind),
    Undo,
    Redo,
    EraseSelection,
}

/// What the panels show.
pub struct View<'a> {
    pub tool: ToolKind,
    pub status: &'a Status,
    pub info: &'a [String],
    pub has_selection: bool,
}

/// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
/// the 3D view, in points.
pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
    let mut actions = Vec::new();
    Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal(|ui| {
            for kind in ToolKind::ALL {
                let key = match kind.shortcut() {
                    ' ' => "Space".to_string(),
                    c => c.to_ascii_uppercase().to_string(),
                };
                let button = ui
                    .selectable_label(view.tool == kind, kind.label())
                    .on_hover_text(format!("{} ({key})", kind.label()));
                if button.clicked() {
                    actions.push(UiAction::Tool(kind));
                }
            }
            ui.separator();
            if ui.button("Undo").on_hover_text("Cmd+Z").clicked() {
                actions.push(UiAction::Undo);
            }
            if ui.button("Redo").on_hover_text("Shift+Cmd+Z").clicked() {
                actions.push(UiAction::Redo);
            }
            let erase = ui
                .add_enabled(view.has_selection, egui::Button::new("Erase selection"))
                .on_hover_text("Delete");
            if erase.clicked() {
                actions.push(UiAction::EraseSelection);
            }
        });
    });
    Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(view.status.prompt);
            if let Some(message) = &view.status.message {
                ui.label(RichText::new(message).color(ui.visuals().warn_fg_color));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The measurements box: typed text, or the tool's live measurement.
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(120.0);
                    ui.monospace(view.status.value.clone().unwrap_or_default());
                });
                ui.label(view.status.label);
            });
        });
    });
    Panel::right("entity_info")
        .resizable(false)
        .default_size(180.0)
        .show(ui, |ui| {
            ui.heading("Entity Info");
            for line in view.info {
                ui.label(line);
            }
        });
    (actions, ui.available_rect_before_wrap())
}
```

Create `crates/app/src/gui.rs`:
```rust
//! egui around the 3D view: window events in, panel layout per frame, painting on top.

use egui::ViewportId;
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use render::Viewport;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::ui::{UiAction, View, draw};

pub struct Gui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

/// One laid-out frame of the panels, ready to paint.
pub struct GuiFrame {
    pub actions: Vec<UiAction>,
    /// Where the 3D view goes, in physical pixels.
    pub viewport: Viewport,
    output: egui::FullOutput,
}

impl Gui {
    pub fn new(window: &Window, device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(device, format, RendererOptions::default());
        Self {
            ctx,
            state,
            renderer,
        }
    }

    /// Feeds a window event to egui. Returns true if egui needs a repaint.
    pub fn on_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).repaint
    }

    /// True while the pointer is over a panel or egui is using it (e.g. a button press).
    pub fn wants_pointer(&self) -> bool {
        self.ctx.is_pointer_over_egui() || self.ctx.egui_is_using_pointer()
    }

    /// True while an egui widget takes typing.
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// Lays out the panels for this frame.
    pub fn frame(&mut self, window: &Window, view: &View) -> GuiFrame {
        let input = self.state.take_egui_input(window);
        let mut laid_out = (Vec::new(), egui::Rect::NOTHING);
        let output = self.ctx.run_ui(input, |ui| laid_out = draw(ui, view));
        let (actions, central) = laid_out;
        let ppp = output.pixels_per_point;
        let px = |v: f32| (v * ppp).round().max(0.0) as u32;
        let viewport = Viewport {
            x: px(central.min.x),
            y: px(central.min.y),
            width: px(central.width()),
            height: px(central.height()),
        };
        GuiFrame {
            actions,
            viewport,
            output,
        }
    }

    /// Paints the frame's panels over `target` (whose size is `size`), handles clipboard and
    /// cursor output, and returns true if egui asks to be repainted right away.
    pub fn paint(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        size: (u32, u32),
        frame: GuiFrame,
    ) -> bool {
        let mut output = frame.output;
        self.state
            .handle_platform_output(window, output.platform_output);
        let screen = ScreenDescriptor {
            size_in_pixels: [size.0, size.1],
            pixels_per_point: output.pixels_per_point,
        };
        let jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui"),
        });
        let uploads = self
            .renderer
            .update_buffers(device, queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.renderer
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        queue.submit(uploads.into_iter().chain([encoder.finish()]));
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        output.textures_delta.clear();
        output
            .viewport_output
            .get(&ViewportId::ROOT)
            .is_some_and(|v| v.repaint_delay.is_zero())
    }
}
```

**egui 0.36 details used here:**
- `Context::run_ui(input, |ui| ...)` with `egui::Panel::{top, bottom, right}(id).show(ui, ...)`. In 0.36, `show_inside` is the deprecated name.
- `textures_delta.set` holds a list of deltas per texture id.
- `Renderer::render` needs a `RenderPass<'static>`; `forget_lifetime()` provides one.
- A `FullOutput` whose texture deltas are dropped unhandled panics in debug builds. That is why `paint` clears the deltas and the tests call `drop_without_applying_deltas`.

`crates/app/src/editor.rs`:
- **`Status`.** Add after `EditorKey`:
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
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}  |  {}", self.tool, self.prompt)?;
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
- **Tool kind.** `Editor` gains `kind: ToolKind` after `tool`, initialized `kind: ToolKind::Line`. `select_tool` sets `self.kind = kind;` right after `self.tool = kind.make();`. Add after `select_tool`:
```rust
    pub fn tool_kind(&self) -> ToolKind {
        self.kind
    }
```
- **Status methods.** Replace `pub fn status(&self) -> String { ... }` with:
```rust
    /// The status as one line: tool, prompt, measurement and feedback separated by bars.
    #[cfg(test)]
    pub fn status(&self) -> String {
        self.status_parts().to_string()
    }

    /// The tool, what to do next, the measurement and any feedback.
    pub fn status_parts(&self) -> Status {
        let (label, value) = self.tool.measurement();
        let value = if self.typed.is_empty() {
            value
        } else {
            Some(format!("{}_", self.typed))
        };
        Status {
            tool: self.tool.name(),
            prompt: self.tool.prompt(),
            label,
            value,
            message: self.message.clone(),
        }
    }
```

Replace `crates/app/src/app.rs` with:
```rust
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use api::{Hub, listen_unix};
use glam::DVec2;
use kernel::EntityId;
use render::{Renderer, Snapshotter, Viewport};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::editor::{Editor, EditorKey};
use crate::gui::Gui;
use crate::host::WindowHost;
use crate::info::entity_info;
use crate::navigation::Button;
use crate::ui::{UiAction, View};

/// Sent to the event loop when an agent API request arrives.
#[derive(Debug)]
pub struct ApiWake;

/// Trackpad pixel deltas per zoom step.
const PIXELS_PER_SCROLL_STEP: f64 = 50.0;

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    snapshotter: Snapshotter,
    editor: Editor,
    gui: Gui,
    /// Document revision and selection the renderer's model buffers were built from.
    drawn: Option<(u64, BTreeSet<EntityId>)>,
    modifiers: ModifiersState,
    /// Where the 3D view sits in the window (physical pixels), from the last frame's layout.
    view: Viewport,
    /// Last cursor position in window coordinates (physical pixels).
    cursor: DVec2,
}

impl State {
    fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no compatible GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        // Colors are authored as sRGB values, so prefer a non-sRGB target that writes them as-is.
        // Keep the default format's channel layout when its non-sRGB twin is available.
        let caps = surface.get_capabilities(&adapter);
        let linear = config.format.remove_srgb_suffix();
        config.format = if caps.formats.contains(&linear) {
            linear
        } else {
            caps.formats
                .iter()
                .copied()
                .find(|f| !f.is_srgb())
                .unwrap_or(config.format)
        };
        surface.configure(&device, &config);

        let renderer = Renderer::new(&device, &queue, config.format);
        let gui = Gui::new(&window, &device, config.format);
        let view = Viewport::full((config.width, config.height));
        let snapshotter = Snapshotter::with_device(&device, &queue);
        let editor = Editor::new(
            DVec2::new(config.width as f64, config.height as f64),
            window.scale_factor(),
        );
        Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            snapshotter,
            editor,
            gui,
            drawn: None,
            modifiers: ModifiersState::empty(),
            view,
            cursor: DVec2::ZERO,
        }
    }

    /// True if `p` (window coordinates) lies in the 3D view.
    fn in_view(&self, p: DVec2) -> bool {
        let v = self.view;
        p.x >= v.x as f64
            && p.y >= v.y as f64
            && p.x < (v.x + v.width) as f64
            && p.y < (v.y + v.height) as f64
    }

    /// The cursor moved to `p` in window coordinates; the editor works in view coordinates.
    fn cursor_moved(&mut self, p: DVec2) -> bool {
        self.cursor = p;
        let origin = DVec2::new(self.view.x as f64, self.view.y as f64);
        let moved = self.editor.cursor_moved(p - origin);
        if self.in_view(p) {
            moved
        } else {
            self.editor.cursor_left() || moved
        }
    }

    fn apply(&mut self, action: UiAction) {
        match action {
            UiAction::Tool(kind) => self.editor.select_tool(kind),
            UiAction::Undo => {
                self.editor.undo();
            }
            UiAction::Redo => {
                self.editor.redo();
            }
            UiAction::EraseSelection => {
                self.editor.erase_selection();
            }
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Lays out the panels, applies what was clicked in them, and draws the 3D view and the
    /// panels. Returns true if a panel action ran (so subscribers should hear about edits).
    fn render(&mut self) -> bool {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return false;
            }
            _ => return false, // Timeout or occluded: skip this frame.
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let size = (self.config.width, self.config.height);

        let status = self.editor.status_parts();
        let info = entity_info(self.editor.doc());
        let panels = View {
            tool: self.editor.tool_kind(),
            status: &status,
            info: &info,
            has_selection: !self.editor.doc().selection().is_empty(),
        };
        let gui_frame = self.gui.frame(&self.window, &panels);
        let acted = !gui_frame.actions.is_empty();
        for action in gui_frame.actions.clone() {
            self.apply(action);
        }
        if gui_frame.viewport != self.view {
            self.view = gui_frame.viewport;
            let v = self.view;
            self.editor
                .resize(DVec2::new(v.width.max(1) as f64, v.height.max(1) as f64));
        }

        let doc = self.editor.doc();
        let drawn = (doc.revision(), doc.selection().clone());
        if self.drawn.as_ref() != Some(&drawn) {
            self.renderer.set_model(doc.model(), doc.selection());
            self.drawn = Some(drawn);
        }
        self.renderer.render_in(
            &target,
            size,
            self.view,
            &self.editor.doc().camera,
            &self.editor.overlay(),
        );
        let again = self.gui.paint(
            &self.window,
            &self.device,
            &self.queue,
            &target,
            size,
            gui_frame,
        );
        self.window.pre_present_notify();
        self.queue.present(frame);
        if again || acted {
            self.window.request_redraw();
        }
        acted
    }

    /// Handles waiting API requests; redraws if they changed the model, selection or camera.
    fn process_api(&mut self, hub: &mut Hub) {
        let before = (
            self.editor.doc().revision(),
            self.editor.doc().selection().clone(),
            self.editor.doc().camera.clone(),
            self.editor.overlay(),
        );
        let mut host = WindowHost {
            editor: &mut self.editor,
            snapshotter: &mut self.snapshotter,
        };
        hub.process(&mut host);
        if (
            self.editor.doc().revision(),
            self.editor.doc().selection().clone(),
            self.editor.doc().camera.clone(),
            self.editor.overlay(),
        ) != before
        {
            self.window.request_redraw();
        }
    }

    /// Tells API subscribers about edits made in the window.
    fn notify_api(&mut self, hub: &mut Hub) {
        let host = WindowHost {
            editor: &mut self.editor,
            snapshotter: &mut self.snapshotter,
        };
        hub.notify(&host);
    }
}

/// The editor key for a key press. Characters from non-ASCII layouts (e.g. Arabic) fall back to
/// the physical key so shortcuts and digits work on any layout.
fn editor_key(logical: &Key, physical: PhysicalKey) -> Option<EditorKey> {
    match logical {
        Key::Named(NamedKey::Enter) => Some(EditorKey::Enter),
        Key::Named(NamedKey::Backspace) => Some(EditorKey::Backspace),
        Key::Named(NamedKey::Delete) => Some(EditorKey::Delete),
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
        KeyCode::KeyE => 'e',
        KeyCode::KeyM => 'm',
        _ => return None,
    })
}

fn map_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        _ => Button::Other,
    }
}

pub struct App {
    state: Option<State>,
    hub: Hub,
    /// Socket the agent API listens on, removed on exit (None if listening failed).
    socket: Option<PathBuf>,
}

impl App {
    /// `socket` is where the agent API listens; None runs without it.
    pub fn new(event_loop: &EventLoop<ApiWake>, socket: Option<PathBuf>) -> Self {
        let proxy = event_loop.create_proxy();
        let hub = Hub::new(move || {
            let _ = proxy.send_event(ApiWake);
        });
        let socket = socket.and_then(|socket| match listen_unix(&socket, hub.inbox()) {
            Ok(()) => {
                eprintln!("agent API listening on {}", socket.display());
                Some(socket)
            }
            Err(e) => {
                eprintln!(
                    "agent API disabled: cannot listen on {}: {e}",
                    socket.display()
                );
                None
            }
        });
        Self {
            state: None,
            hub,
            socket,
        }
    }
}

impl ApplicationHandler<ApiWake> for App {
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(path) = self.socket.take() {
            let _ = std::fs::remove_file(path);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ApiWake) {
        if let Some(s) = self.state.as_mut() {
            s.process_api(&mut self.hub);
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("sketchup-alt")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let mut state = State::new(window);
        state.window.request_redraw();
        // Requests that arrived before the window existed.
        state.process_api(&mut self.hub);
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        if s.gui.on_event(&s.window, &event) {
            s.window.request_redraw();
        }
        let changed = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                false
            }
            WindowEvent::Resized(size) => {
                s.resize(size.width, size.height);
                true
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                s.editor.set_scale_factor(scale_factor);
                false
            }
            WindowEvent::RedrawRequested => {
                if s.render() {
                    s.notify_api(&mut self.hub);
                }
                false
            }
            WindowEvent::ModifiersChanged(m) => {
                s.modifiers = m.state();
                s.editor
                    .set_modifiers(m.state().shift_key(), m.state().alt_key());
                false
            }
            WindowEvent::Focused(false) => {
                s.editor.focus_lost();
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                s.cursor_moved(DVec2::new(position.x, position.y))
            }
            WindowEvent::CursorLeft { .. } => s.editor.cursor_left(),
            // Presses go to the view only when they land in it; releases always do, so a drag
            // that ends over a panel still finishes.
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                if pressed && (s.gui.wants_pointer() || !s.in_view(s.cursor)) {
                    false
                } else {
                    s.editor.button(map_button(button), pressed)
                }
            }
            WindowEvent::MouseWheel { .. } if s.gui.wants_pointer() || !s.in_view(s.cursor) => {
                false
            }
            WindowEvent::KeyboardInput { .. } if s.gui.wants_keyboard() => false,
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / PIXELS_PER_SCROLL_STEP,
                };
                s.editor.scroll(steps)
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command_key = s.modifiers.super_key() || s.modifiers.control_key();
                // Use the physical Z key only when the layout's character is not ASCII (e.g.
                // Arabic or Cyrillic), so Cmd+W on AZERTY does not undo.
                let is_z = match &event.logical_key {
                    Key::Character(c) if c.is_ascii() => c.eq_ignore_ascii_case("z"),
                    _ => event.physical_key == PhysicalKey::Code(KeyCode::KeyZ),
                };
                if command_key && is_z {
                    if s.modifiers.shift_key() {
                        s.editor.redo()
                    } else {
                        s.editor.undo()
                    }
                } else if command_key || s.modifiers.alt_key() {
                    // Alt/Option is the orbit modifier; Option+letter must not type or switch
                    // tools through its physical-key fallback.
                    false
                } else {
                    editor_key(&event.logical_key, event.physical_key)
                        .is_some_and(|k| s.editor.key(k))
                }
            }
            _ => false,
        };
        if changed {
            s.window.request_redraw();
            s.notify_api(&mut self.hub);
        }
    }
}
```

- [ ] **Step 5: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 299 (info 1, ui 2).

- [ ] **Step 6: Window check**

Start the window from a short-path directory and drive it over the socket:
```bash
./target/debug/app --socket m4a.sock &
```
After the window appears, build a box and select its top face and one edge:
```bash
python3 -c 'import socket,json; s=socket.socket(socket.AF_UNIX); s.connect("m4a.sock"); f=s.makefile("rw")
def call(i,m,p): f.write(json.dumps({"jsonrpc":"2.0","id":i,"method":m,"params":p})+"\n"); f.flush(); return json.loads(f.readline())
call(1,"execute",{"type":"add_rectangle","corner":[0,0,0],"opposite":[3,2,0]}); r=call(2,"execute",{"type":"push_pull","face":"f8","distance":1.5})
top=[i for i in r["result"]["changes"]["created"] if i.startswith("f")][-1]; print(call(3,"execute",{"type":"select","ids":[top,"e2"]}))'
```
Then take a screenshot with `screencapture -x /tmp/m4a.png` and read it.

Expected:
- a toolbar (Select, Line highlighted, Rectangle, Push/Pull, Erase, Undo, Redo, Erase selection);
- an Entity Info panel on the right saying "1 edge, 1 face";
- a status bar with "Click to start a line" and the measurements box;
- the box with a blue-tinted top face and one blue edge.

Close the window; the socket file is removed. Report what you saw and delete the screenshot.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/app/Cargo.toml crates/app/src/main.rs crates/app/src/ui.rs crates/app/src/info.rs crates/app/src/gui.rs crates/app/src/editor.rs crates/app/src/app.rs
git commit -m "Wrap the view in an egui toolbar, status bar and Entity Info panel"
```

---

### Task 8: Agent docs, API test, spec amendment

**Files:**
- Modify: `crates/api/tests/dispatch.rs`, `docs/agent-api.md`, `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`

**Interfaces:**
- Consumes: everything above. `describe` picks up `select`, `erase` and `selection` through the existing schemas.

- [ ] **Step 1: Write the test**

In `crates/api/tests/dispatch.rs`:
- in `describe_lists_methods_and_schemas`, add `"erase"` and `"select"` to the variant list after `"push_pull"`;
- add before `render_view_returns_a_png_or_reports_no_gpu`:
```rust
#[test]
fn select_then_erase_the_selection() {
    let mut h = HeadlessHost::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    call(
        &mut h,
        "execute",
        json!({"type": "select", "ids": ["f8", "e2"]}),
    );
    let sel = call(&mut h, "query", json!({"type": "selection"}));
    assert_eq!(sel["result"]["ids"], json!(["e2", "f8"]));
    let ids = sel["result"]["ids"].clone();
    let r = call(&mut h, "execute", json!({"type": "erase", "ids": ids}));
    assert_eq!(r["result"]["revision"], 2);
    let sel = call(&mut h, "query", json!({"type": "selection"}));
    assert_eq!(sel["result"]["ids"], json!([]));
    let r = call(&mut h, "execute", json!({"type": "erase", "ids": ["v0"]}));
    assert_eq!(r["error"]["data"]["ids"], json!(["v0"]));
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p api --test dispatch`
Expected: PASS (13 tests).

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
| `{"type": "selection"}` | the selected edge and face ids, plus `revision` |
| `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |

Unknown fields in commands, queries and params are rejected with `-32602` (a typo fails instead
of silently doing nothing).

Commands: `add_edge`, `add_rectangle`, `push_pull`, `erase`, `select`, `undo`, `redo`,
`set_camera`. Units are
meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m. Entity ids are strings: `v12`
(vertex), `e3` (edge), `f7` (face).

Modeling commands, each one undo step:

| Command | Effect |
|---|---|
| `{"type": "add_edge", "start": [x,y,z], "end": [x,y,z]}` | A straight edge. It merges with nearby vertices, splits edges it crosses, and closed coplanar loops become faces. |
| `{"type": "add_rectangle", "corner": [0,0,0], "opposite": [2,3,0]}` | A rectangle with sides along the axes. Without `normal` the corners must differ in exactly two coordinates. With `"normal": [nx,ny,nz]` it lies in the plane through `corner` with that normal; `opposite` must be in that plane. Sides then run along u = `Z x normal` (horizontal; X on horizontal planes) and v = `normal x u`, so directions are fixed only up to sign: on a plane with normal +Y, u = -X and v = +Z; with normal -Y, u = +X. |
| `{"type": "erase", "ids": ["e2", "f8"]}` | Erases edges and faces. An erased face leaves its edges. An erased edge takes the faces it bounds with it, except that two faces in one plane meeting at it merge into one. Vertices go with their last edge; a vertex left between two collinear edges is healed away. Vertex ids are `invalid_input`. |
| `{"type": "push_pull", "face": "f8", "distance": 1.5}` | Moves a face along its normal (negative: against it). A face on a flat sheet extrudes into a solid and stays as its base. A face on a solid extrudes a bump, or cuts a recess when pushed in. A face whose neighbors all run along the normal, like a box top, moves and stretches them. |

`{"type": "select", "ids": ["f8"], "mode": "replace"}` changes the selection (edges and faces;
`mode` is `replace` (default), `add`, `remove` or `toggle`). It is not an undo step and does not
change the revision. The window shows the selection and agents see what the user selected with
the `selection` query. Erased entities leave the selection.

Find the face to push from the `created` ids of the command that made it, or with
`{"type": "entities", "kinds": ["face"]}`. Faces created by push/pull point out of the solid.
`distance` is measured along the face's own normal, and faces in one plane can point either way
(a pulled-up rectangle's base faces down), so read the face's `normal` with
`{"type": "entity", "id": "f8"}` before choosing the sign. A push/pull that would collapse or
invert the edges around a moved face fails with `invalid_input`. So does one that would carry the
face through other geometry (pushing a recess through the bottom of a box); push-through
openings come in M4.
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

In `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`:
- `Status:` line: `Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c)`.
- Replace the M4 milestone line with:
```markdown
- **M4a**: egui shell (toolbar, measurements box, entity info); selection in the session; Select tool (click, box); Erase tool and command (with face merging and edge healing).
- **M4b**: Move/Copy (with VCB array syntax); full inference set.
- **M4c**: push/pull through to openings; seam healing.
```

- [ ] **Step 4: Verify**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; workspace total 300.

- [ ] **Step 5: Commit**

```bash
git add crates/api/tests/dispatch.rs docs/agent-api.md docs/superpowers/specs/2026-10-10-sketchup-alt-design.md
git commit -m "Document select and erase for agents and split M4 into M4a-c"
```

---

## Self-review notes

- **Spec coverage:**
  - Erase with merging: Task 1.
  - Selection: Task 2.
  - Selection highlight: Task 3.
  - Select and Erase tools: Tasks 4 and 5.
  - Keyboard erase: Task 6.
  - egui shell (toolbar, measurements box, Entity Info, input routing with egui first): Task 7.
  - Agent reachability (`select`, `erase`, `selection`, docs, `describe`): Tasks 2 and 8.
- **Deferred.** The Spec notes list everything deferred to M4b and M4c.
- **Every intermediate state was built and tested.** The totals in each Verify step come from replaying the tasks in order, and the egui window was checked by screenshot during planning.
