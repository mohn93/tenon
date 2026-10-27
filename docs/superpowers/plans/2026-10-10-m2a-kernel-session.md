# M2a Kernel and Session Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A geometry kernel with SketchUp-style sticky edges and automatic faces, and a `session` layer where every edit is a serializable command with undo/redo and structured queries, all testable without a window.

**Architecture:** New `kernel` crate (pure geometry: vertices, edges, faces with vertex loops; every mutation recorded as invertible `Op`s grouped into `ChangeSet`s). New `session` crate (`Document` = model + camera + undo history; `Command`/`Query` enums serialized as tagged JSON). The camera moves from `render` into `session`. The interactive Line tool, model rendering and input routing are M2b (separate plan).

**Tech Stack:** Rust 2024, glam 0.34.1 (with `serde` feature in `kernel`), serde 1.0.229, serde_json 1.0.151 (dev), existing wgpu/winit crates unchanged.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md` (milestone M2, first half; section 1 Kernel, section 5 Agent control).

## Global Constraints

- `kernel` and `session` have no GPU or UI dependencies.
- Dependency direction: `app -> {render} -> session -> kernel`; `render` and `app` use `session::Camera`.
- Model units are meters; Z-up; model math in f64.
- `LENGTH_TOL = 1e-6` m: points closer than this are the same point.
- Entity ids are monotonically increasing `u64`s, never reused, serialized as strings `"v12"`, `"e3"`, `"f7"`.
- Every model mutation is recorded as an `Op`; a `ChangeSet` undoes exactly via `inverse()`.
- One modeling command = one undo step; an edit that changes nothing adds no undo step; camera commands are not recorded in history.
- Commands and queries are serde enums tagged by `"type"` in snake_case; errors are structured (`kind` + `message`), never panics.
- Every capability added is covered by a test that drives it through `session::Command`/`Query` (spec acceptance rule).
- Pinned versions: serde `=1.0.229`, serde_json `=1.0.151`; existing pins unchanged.
- Git: work on `dev`; stage only files you changed; short plain commit messages, no emoji.
- Run `cargo fmt --all` before each commit (code below is already rustfmt-formatted).

## Spec notes (deviations, to be reflected in the spec)

- **Face representation:** faces store vertex loops (`outer` counter-clockwise around `normal`, `holes` clockwise) over a plain edge list, instead of a strict half-edge mesh. SketchUp geometry is non-manifold (an edge can border three or more faces, and edges can dangle), which a half-edge mesh cannot represent.
- **Ids:** monotonic `u64` keys in `BTreeMap`s instead of `slotmap`: undo must restore entities under their original ids, and slotmap cannot re-insert a specific key.
- **Adjacency** is found by scanning (O(n) per lookup). Fine for M2 model sizes; a spatial index comes with picking in M2b/M4.
- Face materials are not modeled yet (M5).

## Out of scope (M2b)

Line tool, inference snapping, drawing faces/edges in the viewport, input dispatcher, egui, renderer reshaping, reversed-Z depth.

## File Structure

```
Cargo.toml                         + serde, serde_json workspace deps
crates/session/Cargo.toml          new
crates/session/src/lib.rs          exports
crates/session/src/camera.rs       moved from crates/render/src/camera.rs (unchanged)
crates/session/src/command.rs      Command, Outcome, CommandError, ErrorKind
crates/session/src/query.rs        Query, QueryResult, *Info structs
crates/session/src/document.rs     Document: execute, query, undo/redo
crates/session/tests/commands.rs   JSON-driven command/query tests
crates/kernel/Cargo.toml           new
crates/kernel/src/lib.rs           exports
crates/kernel/src/ids.rs           VertexId/EdgeId/FaceId, EntityId ("v12")
crates/kernel/src/geom.rs          tolerances, Plane, segment/polygon helpers
crates/kernel/src/model.rs         Vertex/Edge/Face, Op, ChangeSet, Model storage, validate
crates/kernel/src/edit.rs          add_edge: merging and splitting
crates/kernel/src/faces.rs         automatic face creation
crates/kernel/tests/edges.rs       edge sticky-geometry tests
crates/kernel/tests/faces.rs       face tests, cube, undo, randomized invariants
crates/render/*, crates/app/*      import Camera from session
```

---

### Task 1: `session` crate with the camera moved out of `render`

**Files:**
- Create: `crates/session/Cargo.toml`, `crates/session/src/lib.rs`
- Move: `crates/render/src/camera.rs` -> `crates/session/src/camera.rs` (content unchanged)
- Modify: `crates/render/Cargo.toml`, `crates/render/src/lib.rs`, `crates/render/src/renderer.rs`, `crates/render/tests/headless.rs`, `crates/app/Cargo.toml`, `crates/app/src/app.rs`, `crates/app/src/navigation.rs`

**Interfaces:**
- Produces: `session::Camera`, `session::Ray`, `session::camera::UP` and the camera constants, identical to the old `render::camera` items. `render` no longer exports `Camera`/`Ray`.

This is a pure move; the existing tests are the safety net (no new tests).

- [ ] **Step 1: Record the baseline**

Run: `cargo test --workspace 2>&1 | grep "test result"`
Expected: all pass (app 10, render 16 + 4 + headless 2).

- [ ] **Step 2: Create the crate and move the file**

```bash
mkdir -p crates/session/src
git mv crates/render/src/camera.rs crates/session/src/camera.rs
```

`crates/session/Cargo.toml`:
```toml
[package]
name = "session"
version.workspace = true
edition.workspace = true

[dependencies]
glam.workspace = true
```

`crates/session/src/lib.rs`:
```rust
//! The editable document and the command/query layer every client (UI, API, tests) goes through.

pub mod camera;

pub use camera::{Camera, Ray};
```

- [ ] **Step 3: Rewire `render`**

`crates/render/Cargo.toml`: add as the first line under `[dependencies]`:
```toml
session = { path = "../session" }
```

`crates/render/src/lib.rs` becomes:
```rust
pub mod grid;
pub mod renderer;

pub use renderer::Renderer;
```

`crates/render/src/renderer.rs`: replace `use crate::Camera;` with `use session::Camera;`.

`crates/render/tests/headless.rs`: replace `use render::{Camera, Renderer, grid::BACKGROUND};` with:
```rust
use render::{Renderer, grid::BACKGROUND};
use session::Camera;
```

- [ ] **Step 4: Rewire `app`**

`crates/app/Cargo.toml`: add under `[dependencies]` after the `render` line:
```toml
session = { path = "../session" }
```

`crates/app/src/app.rs`: replace `use render::{Camera, Renderer};` with:
```rust
use render::Renderer;
use session::Camera;
```

`crates/app/src/navigation.rs`: replace `use render::Camera;` with `use session::Camera;`.

- [ ] **Step 5: Verify**

Run: `cargo fmt --all && cargo test --workspace 2>&1 | grep "test result" && cargo clippy --workspace --all-targets -- -D warnings`
Expected: same totals as Step 1, now split as session 16, render 4 + headless 2, app 10. No clippy warnings.

- [ ] **Step 6: Commit**

```bash
git add Cargo.lock crates/session crates/render/Cargo.toml crates/render/src/lib.rs crates/render/src/renderer.rs crates/render/tests/headless.rs crates/app/Cargo.toml crates/app/src/app.rs crates/app/src/navigation.rs
git commit -m "Move camera into new session crate"
```

(`git mv` already staged the deletion of `crates/render/src/camera.rs`.)

---

### Task 2: `kernel` crate: ids and geometry helpers

**Files:**
- Modify: `Cargo.toml` (workspace deps)
- Create: `crates/kernel/Cargo.toml`, `crates/kernel/src/lib.rs`, `crates/kernel/src/ids.rs`, `crates/kernel/src/geom.rs`

**Interfaces:**
- Produces:
  - `kernel::{VertexId, EdgeId, FaceId}` (`pub struct X(pub u64)`, Copy, Ord, serde transparent)
  - `kernel::EntityId { Vertex(VertexId), Edge(EdgeId), Face(FaceId) }` with `Display`/`FromStr` and serde as `"v12"`/`"e3"`/`"f7"`
  - `kernel::geom::{LENGTH_TOL, ANGLE_TOL, Plane, project_on_segment, segment_crossing, signed_area2, point_in_polygon, vector_area}`
  - `Plane { normal, d }`: `from_points(a, b, c) -> Option<Plane>` (canonical normal: largest-magnitude component positive), `distance`, `contains`, `same_as`, `basis() -> (u, v)` with `u x v = normal`, `project(p) -> DVec2`

- [ ] **Step 1: Workspace dependencies**

In the root `Cargo.toml`, add to `[workspace.dependencies]`:
```toml
serde = { version = "=1.0.229", features = ["derive"] }
serde_json = "=1.0.151"
```

- [ ] **Step 2: Crate skeleton**

`crates/kernel/Cargo.toml`:
```toml
[package]
name = "kernel"
version.workspace = true
edition.workspace = true

[dependencies]
glam = { workspace = true, features = ["serde"] }
serde.workspace = true
```

`crates/kernel/src/lib.rs`:
```rust
//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

pub mod geom;
mod ids;

pub use ids::{EdgeId, EntityId, FaceId, VertexId};
```

- [ ] **Step 3: Write the failing tests**

Create `crates/kernel/src/ids.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ids_round_trip_as_strings() {
        for id in [
            EntityId::Vertex(VertexId(12)),
            EntityId::Edge(EdgeId(0)),
            EntityId::Face(FaceId(7)),
        ] {
            assert_eq!(id.to_string().parse::<EntityId>().unwrap(), id);
        }
        assert_eq!(EntityId::Face(FaceId(7)).to_string(), "f7");
    }

    #[test]
    fn malformed_entity_ids_are_rejected() {
        for s in ["", "x1", "v", "v-1", "e1.5"] {
            assert!(s.parse::<EntityId>().is_err(), "{s:?} parsed");
        }
    }
}
```

Create `crates/kernel/src/geom.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_normal_is_canonical() {
        let p = Plane::from_points(DVec3::ZERO, DVec3::Y, DVec3::X).unwrap();
        assert_eq!(p.normal, DVec3::Z);
        assert!(Plane::from_points(DVec3::ZERO, DVec3::X, DVec3::X * 2.0).is_none());
    }

    #[test]
    fn basis_is_right_handed() {
        let p = Plane::from_points(DVec3::ZERO, DVec3::new(1.0, 0.0, 1.0), DVec3::Y).unwrap();
        let (u, v) = p.basis();
        assert!(u.cross(v).abs_diff_eq(p.normal, 1e-12));
        assert!(u.dot(p.normal).abs() < 1e-12);
    }

    #[test]
    fn crossing_segments_meet_at_interior_point() {
        let x = segment_crossing(
            DVec3::new(0.0, -1.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0),
        );
        assert!(x.unwrap().abs_diff_eq(DVec3::ZERO, 1e-12));
    }

    #[test]
    fn touching_parallel_and_skew_segments_do_not_cross() {
        let (o, x, y) = (DVec3::ZERO, DVec3::X, DVec3::Y);
        assert!(segment_crossing(o, x, o, y).is_none(), "shared endpoint");
        assert!(segment_crossing(o, x, y, x + y).is_none(), "parallel");
        assert!(
            segment_crossing(-x, x, -y + DVec3::Z, y + DVec3::Z).is_none(),
            "skew"
        );
        assert!(
            segment_crossing(-x, x, DVec3::new(2.0, -1.0, 0.0), DVec3::new(2.0, 1.0, 0.0))
                .is_none(),
            "beyond end"
        );
    }

    #[test]
    fn polygon_helpers() {
        let sq = [DVec2::ZERO, DVec2::X, DVec2::ONE, DVec2::Y];
        assert_eq!(signed_area2(&sq), 2.0);
        assert!(point_in_polygon(DVec2::splat(0.5), &sq));
        assert!(!point_in_polygon(DVec2::splat(1.5), &sq));
        let pts = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        assert!(vector_area(&pts).abs_diff_eq(DVec3::Z, 1e-12));
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p kernel`
Expected: compile errors (`cannot find type EntityId`, `cannot find type Plane`, ...).

- [ ] **Step 5: Implement**

Prepend to `crates/kernel/src/ids.rs` (above the test module):
```rust
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}

id_type!(VertexId);
id_type!(EdgeId);
id_type!(FaceId);

/// Any kernel entity. Serialized as a short string: "v12", "e3", "f7".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EntityId {
    Vertex(VertexId),
    Edge(EdgeId),
    Face(FaceId),
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityId::Vertex(id) => write!(f, "v{}", id.0),
            EntityId::Edge(id) => write!(f, "e{}", id.0),
            EntityId::Face(id) => write!(f, "f{}", id.0),
        }
    }
}

impl FromStr for EntityId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, num) = s.split_at(s.len().min(1));
        let n: u64 = num
            .parse()
            .map_err(|_| format!("invalid entity id {s:?}"))?;
        match kind {
            "v" => Ok(EntityId::Vertex(VertexId(n))),
            "e" => Ok(EntityId::Edge(EdgeId(n))),
            "f" => Ok(EntityId::Face(FaceId(n))),
            _ => Err(format!("invalid entity id {s:?}")),
        }
    }
}

impl From<EntityId> for String {
    fn from(id: EntityId) -> String {
        id.to_string()
    }
}

impl TryFrom<String> for EntityId {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}
```

Prepend to `crates/kernel/src/geom.rs` (above the test module):
```rust
//! Tolerances and small geometric helpers used by the kernel.

use glam::{DVec2, DVec3};

/// Points closer than this (meters) are the same point.
pub const LENGTH_TOL: f64 = 1e-6;
/// Sine of the largest angle treated as zero (parallel or collinear).
pub const ANGLE_TOL: f64 = 1e-9;

/// Plane `normal . p = d` with a unit normal in canonical orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub normal: DVec3,
    pub d: f64,
}

impl Plane {
    /// Plane through three points, or None if they are collinear.
    pub fn from_points(a: DVec3, b: DVec3, c: DVec3) -> Option<Plane> {
        let ab = b - a;
        let ac = c - a;
        let n = ab.cross(ac);
        if n.length() <= ANGLE_TOL * ab.length() * ac.length() {
            return None;
        }
        let normal = canonical(n.normalize());
        Some(Plane {
            normal,
            d: normal.dot(a),
        })
    }

    pub fn distance(&self, p: DVec3) -> f64 {
        self.normal.dot(p) - self.d
    }

    pub fn contains(&self, p: DVec3) -> bool {
        self.distance(p).abs() < LENGTH_TOL
    }

    pub fn same_as(&self, other: &Plane) -> bool {
        self.normal.cross(other.normal).length() < ANGLE_TOL * 1e3
            && self.normal.dot(other.normal) > 0.0
            && (self.d - other.d).abs() < LENGTH_TOL
    }

    /// Orthonormal in-plane axes (u, v) with u x v = normal.
    pub fn basis(&self) -> (DVec3, DVec3) {
        let helper = if self.normal.x.abs() < 0.9 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let u = helper.cross(self.normal).normalize();
        let v = self.normal.cross(u);
        (u, v)
    }

    pub fn project(&self, p: DVec3) -> DVec2 {
        let (u, v) = self.basis();
        DVec2::new(p.dot(u), p.dot(v))
    }
}

/// Flip a unit normal so its largest-magnitude component is positive.
fn canonical(n: DVec3) -> DVec3 {
    let a = n.abs();
    let major = if a.x >= a.y && a.x >= a.z {
        n.x
    } else if a.y >= a.z {
        n.y
    } else {
        n.z
    };
    if major < 0.0 { -n } else { n }
}

/// Parameter t of the point on segment a-b closest to p (unclamped) and its distance.
pub fn project_on_segment(p: DVec3, a: DVec3, b: DVec3) -> (f64, f64) {
    let ab = b - a;
    let t = (p - a).dot(ab) / ab.length_squared();
    (t, (a + ab * t).distance(p))
}

/// The point where open segments p-q and a-b cross, if they cross away from all four endpoints.
pub fn segment_crossing(p: DVec3, q: DVec3, a: DVec3, b: DVec3) -> Option<DVec3> {
    let d1 = q - p;
    let d2 = b - a;
    let r = p - a;
    let a11 = d1.dot(d1);
    let a12 = d1.dot(d2);
    let a22 = d2.dot(d2);
    let c1 = d1.dot(r);
    let c2 = d2.dot(r);
    let denom = a11 * a22 - a12 * a12;
    if denom <= ANGLE_TOL * ANGLE_TOL * a11 * a22 {
        return None; // parallel
    }
    let s = (a12 * c2 - a22 * c1) / denom;
    let t = (a11 * c2 - a12 * c1) / denom;
    if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
        return None;
    }
    let x1 = p + d1 * s;
    let x2 = a + d2 * t;
    if x1.distance(x2) >= LENGTH_TOL {
        return None;
    }
    let x = (x1 + x2) * 0.5;
    let away = |e: DVec3| e.distance(x) >= LENGTH_TOL;
    (away(p) && away(q) && away(a) && away(b)).then_some(x)
}

/// Twice the signed area of a 2D polygon (positive when counter-clockwise).
pub fn signed_area2(poly: &[DVec2]) -> f64 {
    (0..poly.len())
        .map(|i| poly[i].perp_dot(poly[(i + 1) % poly.len()]))
        .sum()
}

/// Even-odd point-in-polygon test. Points on the boundary may go either way.
pub fn point_in_polygon(p: DVec2, poly: &[DVec2]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Vector area of a closed 3D polygon (direction = normal by right-hand rule, length = area).
pub fn vector_area(points: &[DVec3]) -> DVec3 {
    (0..points.len())
        .map(|i| points[i].cross(points[(i + 1) % points.len()]))
        .sum::<DVec3>()
        * 0.5
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p kernel`
Expected: 7 passed (2 ids + 5 geom).

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add Cargo.toml Cargo.lock crates/kernel
git commit -m "Add kernel crate with ids and geometry helpers"
```

---

### Task 3: Model storage, recorded ops and change sets

**Files:**
- Create: `crates/kernel/src/model.rs`
- Modify: `crates/kernel/src/lib.rs`

**Interfaces:**
- Consumes: ids and `geom::{LENGTH_TOL, vector_area}` from Task 2.
- Produces:
  - `Vertex { pos: DVec3 }`, `Edge { v: [VertexId; 2] }`, `Face { outer: Vec<VertexId>, holes: Vec<Vec<VertexId>>, normal: DVec3 }` (serde)
  - `Op` (Add/Remove for Vertex/Edge/Face, carrying id + data); `ChangeSet { ops }` with `is_empty`, `inverse`, `summary() -> ChangeSummary { created, deleted, modified: Vec<EntityId> }`
  - `Model`: `new`, `vertex/edge/face(id) -> Option<&_>`, `vertices()/edges()/faces()` iterators of `(id, &data)`, `pos(VertexId) -> DVec3`, `edge_between(a, b)`, `edges_at(v)`, `face_area(FaceId)`, `take_changes() -> ChangeSet`, `apply(&ChangeSet)`, `validate() -> Result<(), String>`; `PartialEq` compares entities only
  - Crate-internal recorded mutations: `insert_vertex`, `insert_edge`, `remove_edge`, `insert_face`, `remove_face`, `replace_face`

- [ ] **Step 1: Export the module**

`crates/kernel/src/lib.rs` becomes:
```rust
//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

pub mod geom;
mod ids;
mod model;

pub use ids::{EdgeId, EntityId, FaceId, VertexId};
pub use model::{ChangeSet, ChangeSummary, Edge, Face, Model, Op, Vertex};
```

- [ ] **Step 2: Write the failing tests**

Create `crates/kernel/src/model.rs` containing only its test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_changes_undo_and_redo_exactly() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        let b = m.insert_vertex(DVec3::X);
        m.insert_edge(a, b);
        let first = m.take_changes();
        let after_first = m.clone();

        let e = m.edge_between(a, b).unwrap();
        m.remove_edge(e);
        let second = m.take_changes();

        m.apply(&second.inverse());
        assert_eq!(m, after_first);
        m.apply(&first.inverse());
        assert_eq!(m, Model::new());
        m.apply(&first);
        m.apply(&second);
        assert_eq!(m.edges().count(), 0);
        assert_eq!(m.vertices().count(), 2);
    }

    #[test]
    fn summary_reports_net_effect() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        let b = m.insert_vertex(DVec3::X);
        let c = m.insert_vertex(DVec3::Y);
        let e = m.insert_edge(a, b);
        m.take_changes();

        m.remove_edge(e);
        let e2 = m.insert_edge(a, c);
        let s = m.take_changes().summary();
        assert_eq!(s.created, vec![EntityId::Edge(e2)]);
        assert_eq!(s.deleted, vec![EntityId::Edge(e)]);
        assert!(s.modified.is_empty());
    }

    #[test]
    fn ids_are_never_reused_after_undo() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        let c = m.take_changes();
        m.apply(&c.inverse());
        let b = m.insert_vertex(DVec3::X);
        assert_ne!(a, b);
    }

    #[test]
    fn validate_catches_duplicate_edges() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        let b = m.insert_vertex(DVec3::X);
        m.insert_edge(a, b);
        assert!(m.validate().is_ok());
        m.insert_edge(b, a);
        assert!(m.validate().unwrap_err().contains("duplicates"));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p kernel model`
Expected: compile errors (`cannot find type Model`, ...).

- [ ] **Step 4: Implement**

Prepend to `crates/kernel/src/model.rs` (above the test module):
```rust
use std::collections::{BTreeMap, BTreeSet};

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::geom::{LENGTH_TOL, vector_area};
use crate::ids::{EdgeId, EntityId, FaceId, VertexId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub pos: DVec3,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub v: [VertexId; 2],
}

/// A planar face. `outer` winds counter-clockwise around `normal`; each hole winds clockwise.
/// Consecutive loop vertices (wrapping) are always joined by an edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub outer: Vec<VertexId>,
    pub holes: Vec<Vec<VertexId>>,
    pub normal: DVec3,
}

/// One primitive, invertible change to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    AddVertex(VertexId, Vertex),
    RemoveVertex(VertexId, Vertex),
    AddEdge(EdgeId, Edge),
    RemoveEdge(EdgeId, Edge),
    AddFace(FaceId, Face),
    RemoveFace(FaceId, Face),
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
        }
    }

    fn entity(&self) -> (EntityId, bool) {
        match self {
            Op::AddVertex(id, _) => (EntityId::Vertex(*id), true),
            Op::RemoveVertex(id, _) => (EntityId::Vertex(*id), false),
            Op::AddEdge(id, _) => (EntityId::Edge(*id), true),
            Op::RemoveEdge(id, _) => (EntityId::Edge(*id), false),
            Op::AddFace(id, _) => (EntityId::Face(*id), true),
            Op::RemoveFace(id, _) => (EntityId::Face(*id), false),
        }
    }
}

/// The ordered primitive ops of one edit. Applying `inverse()` undoes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub ops: Vec<Op>,
}

/// Net effect of a ChangeSet per entity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangeSummary {
    pub created: Vec<EntityId>,
    pub deleted: Vec<EntityId>,
    pub modified: Vec<EntityId>,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn inverse(&self) -> ChangeSet {
        ChangeSet {
            ops: self.ops.iter().rev().map(Op::inverse).collect(),
        }
    }

    pub fn summary(&self) -> ChangeSummary {
        // entity -> (existed before, exists after)
        let mut state: BTreeMap<EntityId, (bool, bool)> = BTreeMap::new();
        for op in &self.ops {
            let (id, added) = op.entity();
            state.entry(id).or_insert((!added, added)).1 = added;
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
}

/// The geometry of one mesh. Every mutation is recorded as an `Op` until `take_changes`.
#[derive(Clone, Debug, Default)]
pub struct Model {
    vertices: BTreeMap<VertexId, Vertex>,
    edges: BTreeMap<EdgeId, Edge>,
    faces: BTreeMap<FaceId, Face>,
    next_id: u64,
    pending: Vec<Op>,
}

/// Two models are equal when they hold the same entities under the same ids.
impl PartialEq for Model {
    fn eq(&self, other: &Self) -> bool {
        self.vertices == other.vertices && self.edges == other.edges && self.faces == other.faces
    }
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- read access ----

    pub fn vertex(&self, id: VertexId) -> Option<&Vertex> {
        self.vertices.get(&id)
    }

    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(&id)
    }

    pub fn face(&self, id: FaceId) -> Option<&Face> {
        self.faces.get(&id)
    }

    pub fn vertices(&self) -> impl Iterator<Item = (VertexId, &Vertex)> {
        self.vertices.iter().map(|(k, v)| (*k, v))
    }

    pub fn edges(&self) -> impl Iterator<Item = (EdgeId, &Edge)> {
        self.edges.iter().map(|(k, v)| (*k, v))
    }

    pub fn faces(&self) -> impl Iterator<Item = (FaceId, &Face)> {
        self.faces.iter().map(|(k, v)| (*k, v))
    }

    pub fn pos(&self, id: VertexId) -> DVec3 {
        self.vertices[&id].pos
    }

    pub fn edge_between(&self, a: VertexId, b: VertexId) -> Option<EdgeId> {
        self.edges
            .iter()
            .find(|(_, e)| e.v == [a, b] || e.v == [b, a])
            .map(|(id, _)| *id)
    }

    pub fn edges_at(&self, v: VertexId) -> Vec<EdgeId> {
        self.edges
            .iter()
            .filter(|(_, e)| e.v.contains(&v))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Area of a face (outer area minus holes).
    pub fn face_area(&self, id: FaceId) -> f64 {
        let f = &self.faces[&id];
        let area = |l: &Vec<VertexId>| {
            vector_area(&l.iter().map(|v| self.pos(*v)).collect::<Vec<_>>()).dot(f.normal)
        };
        area(&f.outer) + f.holes.iter().map(area).sum::<f64>()
    }

    // ---- change tracking ----

    /// Ends the current edit and returns everything recorded since the last call.
    pub fn take_changes(&mut self) -> ChangeSet {
        ChangeSet {
            ops: std::mem::take(&mut self.pending),
        }
    }

    /// Applies a ChangeSet (e.g. an inverse for undo) without recording it.
    pub fn apply(&mut self, changes: &ChangeSet) {
        for op in &changes.ops {
            self.apply_op(op.clone());
        }
    }

    fn apply_op(&mut self, op: Op) {
        match op {
            Op::AddVertex(id, v) => {
                self.bump(id.0);
                self.vertices.insert(id, v);
            }
            Op::RemoveVertex(id, _) => {
                self.vertices.remove(&id);
            }
            Op::AddEdge(id, e) => {
                self.bump(id.0);
                self.edges.insert(id, e);
            }
            Op::RemoveEdge(id, _) => {
                self.edges.remove(&id);
            }
            Op::AddFace(id, f) => {
                self.bump(id.0);
                self.faces.insert(id, f);
            }
            Op::RemoveFace(id, _) => {
                self.faces.remove(&id);
            }
        }
    }

    fn bump(&mut self, used: u64) {
        self.next_id = self.next_id.max(used + 1);
    }

    fn record(&mut self, op: Op) {
        self.apply_op(op.clone());
        self.pending.push(op);
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ---- recorded primitive mutations (used by edit and face code) ----

    pub(crate) fn insert_vertex(&mut self, pos: DVec3) -> VertexId {
        let id = VertexId(self.alloc());
        self.record(Op::AddVertex(id, Vertex { pos }));
        id
    }

    pub(crate) fn insert_edge(&mut self, a: VertexId, b: VertexId) -> EdgeId {
        let id = EdgeId(self.alloc());
        self.record(Op::AddEdge(id, Edge { v: [a, b] }));
        id
    }

    pub(crate) fn remove_edge(&mut self, id: EdgeId) -> Edge {
        let e = self.edges[&id].clone();
        self.record(Op::RemoveEdge(id, e.clone()));
        e
    }

    pub(crate) fn insert_face(&mut self, face: Face) -> FaceId {
        let id = FaceId(self.alloc());
        self.record(Op::AddFace(id, face));
        id
    }

    pub(crate) fn remove_face(&mut self, id: FaceId) -> Face {
        let f = self.faces[&id].clone();
        self.record(Op::RemoveFace(id, f.clone()));
        f
    }

    /// Replaces a face's data, keeping its id.
    pub(crate) fn replace_face(&mut self, id: FaceId, face: Face) {
        self.remove_face(id);
        self.record(Op::AddFace(id, face));
    }

    // ---- invariants ----

    /// Checks structural invariants. Used by tests after every edit.
    pub fn validate(&self) -> Result<(), String> {
        let mut pairs = BTreeSet::new();
        for (id, e) in &self.edges {
            let [a, b] = e.v;
            if !self.vertices.contains_key(&a) || !self.vertices.contains_key(&b) {
                return Err(format!("edge {id:?} references a missing vertex"));
            }
            if a == b || self.pos(a).distance(self.pos(b)) < LENGTH_TOL {
                return Err(format!("edge {id:?} has zero length"));
            }
            if !pairs.insert((a.min(b), a.max(b))) {
                return Err(format!("edge {id:?} duplicates another edge"));
            }
        }
        let verts: Vec<_> = self.vertices.iter().collect();
        for (i, (ia, a)) in verts.iter().enumerate() {
            for (ib, b) in &verts[i + 1..] {
                if a.pos.distance(b.pos) < LENGTH_TOL {
                    return Err(format!("vertices {ia:?} and {ib:?} coincide"));
                }
            }
        }
        for (id, f) in &self.faces {
            if (f.normal.length() - 1.0).abs() > 1e-9 {
                return Err(format!("face {id:?} normal is not unit length"));
            }
            let origin = self.pos(f.outer[0]);
            for (k, lp) in std::iter::once(&f.outer).chain(&f.holes).enumerate() {
                if lp.len() < 3 {
                    return Err(format!("face {id:?} has a loop with fewer than 3 vertices"));
                }
                if lp.iter().collect::<BTreeSet<_>>().len() != lp.len() {
                    return Err(format!("face {id:?} has a loop that repeats a vertex"));
                }
                for i in 0..lp.len() {
                    let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
                    if self.edge_between(a, b).is_none() {
                        return Err(format!("face {id:?} loop side {a:?}-{b:?} has no edge"));
                    }
                    if (self.pos(a) - origin).dot(f.normal).abs() > LENGTH_TOL {
                        return Err(format!("face {id:?} vertex {a:?} is off the face plane"));
                    }
                }
                let pts: Vec<_> = lp.iter().map(|v| self.pos(*v)).collect();
                let signed = vector_area(&pts).dot(f.normal);
                let ok = if k == 0 { signed > 0.0 } else { signed < 0.0 };
                if !ok {
                    return Err(format!("face {id:?} loop {k} winds the wrong way"));
                }
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kernel`
Expected: 11 passed. `cargo build -p kernel` warns that the crate-internal mutation helpers are unused; expected until Tasks 4-5 use them (do not silence them).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/kernel/src/lib.rs crates/kernel/src/model.rs
git commit -m "Add kernel model with recorded, invertible changes"
```

---

### Task 4: `add_edge` with merging and splitting

**Files:**
- Create: `crates/kernel/src/edit.rs`, `crates/kernel/src/faces.rs` (stub), `crates/kernel/tests/edges.rs`
- Modify: `crates/kernel/src/lib.rs`

**Interfaces:**
- Consumes: `Model` internals from Task 3, `geom::{project_on_segment, segment_crossing}`.
- Produces:
  - `kernel::KernelError { NotFinite, ZeroLength }` (Display + Error)
  - `Model::add_edge(a: DVec3, b: DVec3) -> Result<ChangeSet, KernelError>`: endpoints merge with vertices within `LENGTH_TOL`; an endpoint inside an existing edge splits it; edges crossing the new segment away from endpoints are split; vertices lying on the segment split it; collinear overlaps merge; returns one ChangeSet (empty if nothing new).
  - Crate-internal `Model::create_faces(&mut self, new_edges: &[EdgeId])`: a no-op stub in this task, implemented in Task 5. Splitting an edge already updates every face loop that used it.

- [ ] **Step 1: Export modules and add the stub**

`crates/kernel/src/lib.rs` becomes:
```rust
//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

mod edit;
mod faces;
pub mod geom;
mod ids;
mod model;

pub use edit::KernelError;
pub use ids::{EdgeId, EntityId, FaceId, VertexId};
pub use model::{ChangeSet, ChangeSummary, Edge, Face, Model, Op, Vertex};
```

`crates/kernel/src/faces.rs`:
```rust
//! Automatic face creation (implemented in Task 5).

use crate::ids::EdgeId;
use crate::model::Model;

impl Model {
    /// Creates faces for closed coplanar loops that use at least one of `new_edges`.
    pub(crate) fn create_faces(&mut self, _new_edges: &[EdgeId]) {}
}
```

- [ ] **Step 2: Write the failing tests**

`crates/kernel/tests/edges.rs`:
```rust
use glam::DVec3;
use kernel::{KernelError, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

#[test]
fn rejects_degenerate_edges() {
    let mut m = Model::new();
    assert_eq!(
        m.add_edge(DVec3::ZERO, p(0.0, 0.0, 1e-9)),
        Err(KernelError::ZeroLength)
    );
    assert_eq!(
        m.add_edge(DVec3::ZERO, p(f64::NAN, 0.0, 0.0)),
        Err(KernelError::NotFinite)
    );
    assert_eq!(m, Model::new());
}

#[test]
fn endpoints_merge_with_nearby_vertices() {
    let mut m = Model::new();
    m.add_edge(DVec3::ZERO, p(1.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(1.0 + 1e-8, 0.0, 0.0), p(1.0, 1.0, 0.0))
        .unwrap();
    assert_eq!(m.vertices().count(), 3);
    assert_eq!(m.edges().count(), 2);
    m.validate().unwrap();
}

#[test]
fn adding_an_existing_edge_changes_nothing() {
    let mut m = Model::new();
    m.add_edge(DVec3::ZERO, p(1.0, 0.0, 0.0)).unwrap();
    let c = m.add_edge(p(1.0, 0.0, 0.0), DVec3::ZERO).unwrap();
    assert!(c.is_empty());
}

#[test]
fn endpoint_on_an_edge_splits_it() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 3);
    assert_eq!(m.vertices().count(), 4);
}

#[test]
fn crossing_edges_split_each_other() {
    let mut m = Model::new();
    m.add_edge(p(-1.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(0.0, -1.0, 0.0), p(0.0, 1.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 4);
    assert_eq!(m.vertices().count(), 5);
}

#[test]
fn skew_edges_do_not_split() {
    let mut m = Model::new();
    m.add_edge(p(-1.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(0.0, -1.0, 1.0), p(0.0, 1.0, 1.0)).unwrap();
    assert_eq!(m.edges().count(), 2);
}

#[test]
fn collinear_overlap_merges() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(1.0, 0.0, 0.0), p(3.0, 0.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 3);
    assert_eq!(m.vertices().count(), 4);
}

#[test]
fn edge_covering_existing_vertices_is_split_at_them() {
    let mut m = Model::new();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    m.add_edge(p(2.0, 0.0, 0.0), p(2.0, 1.0, 0.0)).unwrap();
    m.add_edge(p(0.0, 0.0, 0.0), p(3.0, 0.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 5);
}

#[test]
fn open_polyline_creates_no_face() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    m.add_edge(p(1.0, 1.0, 0.0), p(0.0, 1.0, 0.0)).unwrap();
    assert_eq!(m.faces().count(), 0);
}

#[test]
fn edits_undo_exactly() {
    let mut m = Model::new();
    m.add_edge(p(-1.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    let before = m.clone();
    let c = m.add_edge(p(0.0, -1.0, 0.0), p(0.0, 1.0, 0.0)).unwrap();
    m.apply(&c.inverse());
    assert_eq!(m, before);
    m.apply(&c);
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 4);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p kernel --test edges`
Expected: compile errors (`unresolved import kernel::KernelError`, no method `add_edge`).

- [ ] **Step 4: Implement**

`crates/kernel/src/edit.rs`:
```rust
use std::fmt;

use glam::DVec3;

use crate::geom::{LENGTH_TOL, project_on_segment, segment_crossing};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::{ChangeSet, Model};

#[derive(Clone, Debug, PartialEq)]
pub enum KernelError {
    NotFinite,
    ZeroLength,
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelError::NotFinite => write!(f, "coordinates must be finite numbers"),
            KernelError::ZeroLength => write!(f, "edge endpoints coincide"),
        }
    }
}

impl std::error::Error for KernelError {}

impl Model {
    /// Adds the segment a-b with sticky-geometry rules: endpoints merge with nearby vertices,
    /// crossed edges are split, collinear overlaps merge, and closed coplanar loops become faces.
    /// Returns the changes as one ChangeSet (empty if the edge already existed).
    pub fn add_edge(&mut self, a: DVec3, b: DVec3) -> Result<ChangeSet, KernelError> {
        if !a.is_finite() || !b.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if a.distance(b) < LENGTH_TOL {
            return Err(KernelError::ZeroLength);
        }
        let new_edges = self.insert_segment(a, b);
        self.create_faces(&new_edges);
        Ok(self.take_changes())
    }

    fn find_vertex(&self, p: DVec3) -> Option<VertexId> {
        self.vertices()
            .find(|(_, v)| v.pos.distance(p) < LENGTH_TOL)
            .map(|(id, _)| id)
    }

    /// Existing vertex at p, or a new one (splitting an edge if p lies inside it).
    fn vertex_at(&mut self, p: DVec3) -> VertexId {
        if let Some(v) = self.find_vertex(p) {
            return v;
        }
        let hit = self.edges().find_map(|(id, e)| {
            let (t, dist) = project_on_segment(p, self.pos(e.v[0]), self.pos(e.v[1]));
            (dist < LENGTH_TOL && t > 0.0 && t < 1.0).then_some(id)
        });
        match hit {
            Some(e) => self.split_edge(e, p),
            None => self.insert_vertex(p),
        }
    }

    /// Splits edge e at p, inserting the new vertex into every face loop that used the edge.
    fn split_edge(&mut self, e: EdgeId, p: DVec3) -> VertexId {
        let [a, b] = self.remove_edge(e).v;
        let m = self.insert_vertex(p);
        self.insert_edge(a, m);
        self.insert_edge(m, b);
        let users: Vec<FaceId> = self
            .faces()
            .filter(|(_, f)| {
                std::iter::once(&f.outer)
                    .chain(&f.holes)
                    .any(|l| loop_has_side(l, a, b))
            })
            .map(|(id, _)| id)
            .collect();
        for id in users {
            let mut f = self.face(id).unwrap().clone();
            for l in std::iter::once(&mut f.outer).chain(f.holes.iter_mut()) {
                insert_between(l, a, b, m);
            }
            self.replace_face(id, f);
        }
        m
    }

    /// Inserts edges covering a-b, splitting at every vertex and edge crossing along the way.
    /// Returns only the edges that did not exist before.
    fn insert_segment(&mut self, a: DVec3, b: DVec3) -> Vec<EdgeId> {
        let va = self.vertex_at(a);
        let vb = self.vertex_at(b);
        let (pa, pb) = (self.pos(va), self.pos(vb));

        let others: Vec<EdgeId> = self
            .edges()
            .filter(|(_, e)| !e.v.contains(&va) && !e.v.contains(&vb))
            .map(|(id, _)| id)
            .collect();
        for e in others {
            let [p, q] = self.edge(e).unwrap().v;
            if let Some(x) = segment_crossing(self.pos(p), self.pos(q), pa, pb) {
                self.split_edge(e, x);
            }
        }

        let mut along: Vec<(f64, VertexId)> = self
            .vertices()
            .filter_map(|(id, v)| {
                let (t, dist) = project_on_segment(v.pos, pa, pb);
                (dist < LENGTH_TOL && (-1e-12..=1.0 + 1e-12).contains(&t)).then_some((t, id))
            })
            .collect();
        along.sort_by(|x, y| x.0.total_cmp(&y.0));

        let mut created = Vec::new();
        for w in along.windows(2) {
            let (u, v) = (w[0].1, w[1].1);
            if self.edge_between(u, v).is_none() {
                created.push(self.insert_edge(u, v));
            }
        }
        created
    }
}

fn loop_has_side(l: &[VertexId], a: VertexId, b: VertexId) -> bool {
    (0..l.len()).any(|i| {
        let (x, y) = (l[i], l[(i + 1) % l.len()]);
        (x, y) == (a, b) || (x, y) == (b, a)
    })
}

fn insert_between(l: &mut Vec<VertexId>, a: VertexId, b: VertexId, m: VertexId) {
    for i in 0..l.len() {
        let (x, y) = (l[i], l[(i + 1) % l.len()]);
        if (x, y) == (a, b) || (x, y) == (b, a) {
            l.insert(i + 1, m);
            return;
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kernel`
Expected: 11 unit + 10 edges passed. A dead-code warning for `insert_face` remains until Task 5.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/kernel/src/lib.rs crates/kernel/src/edit.rs crates/kernel/src/faces.rs crates/kernel/tests/edges.rs
git commit -m "Add sticky add_edge with merging and splitting"
```

---

### Task 5: Automatic faces

**Files:**
- Modify: `crates/kernel/src/faces.rs` (replace the stub)
- Create: `crates/kernel/tests/faces.rs`

**Interfaces:**
- Consumes: everything from Tasks 2-4.
- Produces: `create_faces` behavior used by `add_edge`:
  - A closed loop of coplanar edges that includes at least one newly added edge becomes a face (outer loop counter-clockwise around the plane's canonical normal).
  - A new edge joining two points on an existing face's boundary replaces that face with the two pieces (existing holes go to the piece that contains them).
  - A closed loop floating inside a face becomes a new face and a hole of the surrounding face.
  - A closed loop drawn around existing faces in the same plane gets them as holes.
  - Dangling edges and non-coplanar loops create nothing.

Algorithm (for reviewers): for each new edge, candidate planes are spanned by it and each non-collinear edge touching it; per distinct plane, collect the in-plane edges connected to the new edges, project to 2D, trace all faces of the planar graph by always turning as far left as possible, and keep simple counter-clockwise cycles that use a new edge and are not already faces. The container face is found by testing a point just left of each cycle's first side.

- [ ] **Step 1: Write the failing tests**

`crates/kernel/tests/faces.rs`:
```rust
use glam::DVec3;
use kernel::{ChangeSet, EntityId, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// Adds a closed polyline and checks invariants after every edge.
fn polygon(m: &mut Model, pts: &[DVec3]) -> Vec<ChangeSet> {
    (0..pts.len())
        .map(|i| {
            let c = m.add_edge(pts[i], pts[(i + 1) % pts.len()]).unwrap();
            m.validate().unwrap();
            c
        })
        .collect()
}

fn square(m: &mut Model, x0: f64, y0: f64, size: f64) -> Vec<ChangeSet> {
    polygon(
        m,
        &[
            p(x0, y0, 0.0),
            p(x0 + size, y0, 0.0),
            p(x0 + size, y0 + size, 0.0),
            p(x0, y0 + size, 0.0),
        ],
    )
}

fn face_areas(m: &Model) -> Vec<f64> {
    let mut a: Vec<f64> = m.faces().map(|(id, _)| m.face_area(id)).collect();
    a.sort_by(f64::total_cmp);
    a
}

fn count_faces(ids: &[EntityId]) -> usize {
    ids.iter()
        .filter(|id| matches!(id, EntityId::Face(_)))
        .count()
}

#[test]
fn closing_a_square_creates_one_face() {
    let mut m = Model::new();
    let changes = square(&mut m, 0.0, 0.0, 2.0);
    assert!(
        changes[..3]
            .iter()
            .all(|c| count_faces(&c.summary().created) == 0)
    );
    assert_eq!(count_faces(&changes[3].summary().created), 1);
    let (_, f) = m.faces().next().unwrap();
    assert_eq!(f.normal, DVec3::Z);
    assert_eq!(face_areas(&m), [4.0]);
}

#[test]
fn non_coplanar_loop_creates_no_face() {
    let mut m = Model::new();
    polygon(
        &mut m,
        &[
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 0.0),
        ],
    );
    assert_eq!(m.faces().count(), 0);
}

#[test]
fn tilted_triangle_gets_a_face_in_its_own_plane() {
    let mut m = Model::new();
    polygon(
        &mut m,
        &[p(0.0, 0.0, 0.0), p(2.0, 0.0, 1.0), p(0.0, 2.0, 1.0)],
    );
    assert_eq!(m.faces().count(), 1);
    let (id, f) = m.faces().next().unwrap();
    assert!((f.normal.length() - 1.0).abs() < 1e-12);
    assert!((m.face_area(id) - 6f64.sqrt()).abs() < 1e-9);
}

#[test]
fn diagonal_splits_a_face_in_two() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    let c = m.add_edge(p(0.0, 0.0, 0.0), p(2.0, 2.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(face_areas(&m), [2.0, 2.0]);
    let s = c.summary();
    assert_eq!(count_faces(&s.deleted), 1);
    assert_eq!(count_faces(&s.created), 2);
}

#[test]
fn edge_between_side_midpoints_splits_the_sides_and_the_face() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 7);
    assert_eq!(face_areas(&m), [2.0, 2.0]);
}

#[test]
fn dangling_edge_inside_a_face_keeps_the_face() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    m.add_edge(p(0.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert_eq!(face_areas(&m), [4.0]);
}

#[test]
fn loop_drawn_inside_a_face_becomes_a_face_and_a_hole() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    square(&mut m, 1.0, 1.0, 1.0);
    assert_eq!(face_areas(&m), [1.0, 15.0]);
    let outer = m.faces().find(|(_, f)| !f.holes.is_empty()).unwrap().1;
    assert_eq!(outer.holes.len(), 1);
}

#[test]
fn loop_drawn_around_a_face_encloses_it_as_a_hole() {
    let mut m = Model::new();
    square(&mut m, 1.0, 1.0, 1.0);
    square(&mut m, 0.0, 0.0, 4.0);
    assert_eq!(face_areas(&m), [1.0, 15.0]);
}

#[test]
fn twelve_cube_edges_make_six_faces() {
    let mut m = Model::new();
    let c = |i: u8| p((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64);
    let pairs = [
        (0, 1),
        (1, 3),
        (3, 2),
        (2, 0),
        (4, 5),
        (5, 7),
        (7, 6),
        (6, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (a, b) in pairs {
        m.add_edge(c(a), c(b)).unwrap();
        m.validate().unwrap();
    }
    assert_eq!(face_areas(&m), [1.0; 6]);
}

#[test]
fn undoing_face_edits_restores_each_state() {
    let mut m = Model::new();
    let mut states = vec![m.clone()];
    let mut history = Vec::new();
    for c in square(&mut m, 0.0, 0.0, 2.0) {
        history.push(c);
    }
    history.push(m.add_edge(p(0.0, 0.0, 0.0), p(2.0, 2.0, 0.0)).unwrap());
    let mut replay = Model::new();
    for c in &history {
        replay.apply(c);
        states.push(replay.clone());
    }
    assert_eq!(replay, m);
    for (i, c) in history.iter().enumerate().rev() {
        m.apply(&c.inverse());
        m.validate().unwrap();
        assert_eq!(m, states[i]);
    }
}

/// Deterministic pseudo-random edits on a coarse grid in a few planes; invariants must always hold
/// and undoing everything must return to an empty model.
#[test]
fn random_edits_keep_invariants_and_fully_undo() {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut faces_seen = 0;
    for _round in 0..20 {
        let mut m = Model::new();
        let mut history = Vec::new();
        for _ in 0..25 {
            let r: Vec<f64> = (0..6).map(|_| (next() % 4) as f64).collect();
            let z = if r[4] < 1.0 { r[5] } else { 0.0 };
            let a = p(r[0], r[1], z);
            let b = p(r[2], r[3], if r[5] < 2.0 { z } else { 0.0 });
            if let Ok(c) = m.add_edge(a, b) {
                if let Err(e) = m.validate() {
                    panic!("after adding {a} -> {b}: {e}");
                }
                history.push(c);
            }
        }
        faces_seen += m.faces().count();
        let built = m.clone();
        for c in history.iter().rev() {
            m.apply(&c.inverse());
        }
        assert_eq!(m, Model::new());
        for c in &history {
            m.apply(c);
        }
        assert_eq!(m, built);
    }
    assert!(faces_seen > 0, "random edits never formed a face");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kernel --test faces`
Expected: most tests FAIL (no faces are created by the stub), e.g. `closing_a_square_creates_one_face` panics on the face count; `non_coplanar_loop_creates_no_face` passes.

- [ ] **Step 3: Implement**

Replace `crates/kernel/src/faces.rs` with:
```rust
//! Automatic face creation: after edges are added, every new closed coplanar loop becomes a face.

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec2;

use crate::geom::{LENGTH_TOL, Plane, point_in_polygon, signed_area2};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::{Face, Model};

impl Model {
    /// Creates faces for closed coplanar loops that use at least one of `new_edges`.
    pub(crate) fn create_faces(&mut self, new_edges: &[EdgeId]) {
        let mut done: Vec<Plane> = Vec::new();
        for &e in new_edges {
            for plane in self.candidate_planes(e) {
                if done.iter().any(|p| p.same_as(&plane)) {
                    continue;
                }
                done.push(plane);
                self.fill_plane(&plane, new_edges);
            }
        }
    }

    /// Planes spanned by edge e and each non-collinear edge touching it.
    fn candidate_planes(&self, e: EdgeId) -> Vec<Plane> {
        let [a, b] = self.edge(e).unwrap().v;
        let (pa, pb) = (self.pos(a), self.pos(b));
        let mut planes = Vec::new();
        for v in [a, b] {
            for f in self.edges_at(v) {
                if f == e {
                    continue;
                }
                let fv = self.edge(f).unwrap().v;
                let c = if fv[0] == v { fv[1] } else { fv[0] };
                if let Some(p) = Plane::from_points(pa, pb, self.pos(c)) {
                    planes.push(p);
                }
            }
        }
        planes
    }

    fn fill_plane(&mut self, plane: &Plane, new_edges: &[EdgeId]) {
        let in_plane = |m: &Model, e: EdgeId| {
            let v = m.edge(e).unwrap().v;
            plane.contains(m.pos(v[0])) && plane.contains(m.pos(v[1]))
        };
        let seeds: BTreeSet<EdgeId> = new_edges
            .iter()
            .copied()
            .filter(|&e| in_plane(self, e))
            .collect();
        let edges = self.connected_in_plane(&seeds, plane);
        let cycles = self.bounded_cycles(&edges, plane);

        let seed_sides: BTreeSet<(VertexId, VertexId)> = seeds
            .iter()
            .map(|e| {
                let [a, b] = self.edge(*e).unwrap().v;
                (a.min(b), a.max(b))
            })
            .collect();
        let uses_seed = |c: &Vec<VertexId>| {
            (0..c.len()).any(|i| {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                seed_sides.contains(&(a.min(b), a.max(b)))
            })
        };
        let new_cycles: Vec<Vec<VertexId>> = cycles
            .into_iter()
            .filter(|c| uses_seed(c) && !self.face_with_loop(c))
            .collect();
        if new_cycles.is_empty() {
            return;
        }

        // Existing faces in this plane, and which one (if any) contains each new cycle.
        let plane_faces: Vec<FaceId> = self
            .faces()
            .filter(|(_, f)| {
                f.normal.cross(plane.normal).length() < 1e-9 && plane.contains(self.pos(f.outer[0]))
            })
            .map(|(id, _)| id)
            .collect();
        let mut by_container: BTreeMap<Option<FaceId>, Vec<Vec<VertexId>>> = BTreeMap::new();
        for c in new_cycles {
            let inside = self.interior_point(&c, plane);
            let container = plane_faces
                .iter()
                .copied()
                .find(|&f| self.face_contains(f, inside, plane));
            by_container.entry(container).or_default().push(c);
        }

        for (container, cycles) in by_container {
            match container {
                Some(f) if cycles.iter().any(|c| self.shares_side_with_outer(f, c)) => {
                    // The new edges cut face f: replace it with the new pieces.
                    let old = self.remove_face(f);
                    for c in cycles {
                        let poly = self.project_loop(&c, plane);
                        let holes = old
                            .holes
                            .iter()
                            .filter(|h| point_in_polygon(plane.project(self.pos(h[0])), &poly))
                            .cloned()
                            .collect();
                        self.insert_face(Face {
                            outer: c,
                            holes,
                            normal: plane.normal,
                        });
                    }
                }
                Some(f) => {
                    // Loops floating inside face f: new faces that are also holes of f.
                    let mut outer = self.face(f).unwrap().clone();
                    for c in cycles {
                        outer.holes.push(c.iter().rev().copied().collect());
                        self.insert_face(Face {
                            outer: c,
                            holes: Vec::new(),
                            normal: plane.normal,
                        });
                    }
                    self.replace_face(f, outer);
                }
                None => {
                    for c in cycles {
                        // Existing faces enclosed by the new loop become its holes.
                        let poly = self.project_loop(&c, plane);
                        let holes = plane_faces
                            .iter()
                            .filter_map(|&f| self.face(f))
                            .filter(|f| {
                                !f.outer.iter().any(|v| c.contains(v))
                                    && f.outer.iter().all(|v| {
                                        point_in_polygon(plane.project(self.pos(*v)), &poly)
                                    })
                            })
                            .map(|f| f.outer.iter().rev().copied().collect())
                            .collect();
                        self.insert_face(Face {
                            outer: c,
                            holes,
                            normal: plane.normal,
                        });
                    }
                }
            }
        }
    }

    /// Edges lying in `plane` that are connected to the seeds through other in-plane edges.
    fn connected_in_plane(&self, seeds: &BTreeSet<EdgeId>, plane: &Plane) -> BTreeSet<EdgeId> {
        let mut found: BTreeSet<EdgeId> = seeds.clone();
        let mut stack: Vec<VertexId> = seeds
            .iter()
            .flat_map(|e| self.edge(*e).unwrap().v)
            .collect();
        let mut seen: BTreeSet<VertexId> = BTreeSet::new();
        while let Some(v) = stack.pop() {
            if !seen.insert(v) {
                continue;
            }
            for e in self.edges_at(v) {
                let [a, b] = self.edge(e).unwrap().v;
                if plane.contains(self.pos(a)) && plane.contains(self.pos(b)) && found.insert(e) {
                    stack.push(if a == v { b } else { a });
                }
            }
        }
        found
    }

    /// Simple counter-clockwise cycles (bounded regions) of the planar graph formed by `edges`.
    /// Each cycle is listed once, wound counter-clockwise around `plane.normal`.
    fn bounded_cycles(&self, edges: &BTreeSet<EdgeId>, plane: &Plane) -> Vec<Vec<VertexId>> {
        // Neighbors of each vertex sorted counter-clockwise by angle.
        let mut nbrs: BTreeMap<VertexId, Vec<VertexId>> = BTreeMap::new();
        for e in edges {
            let [a, b] = self.edge(*e).unwrap().v;
            nbrs.entry(a).or_default().push(b);
            nbrs.entry(b).or_default().push(a);
        }
        let p2: BTreeMap<VertexId, DVec2> = nbrs
            .keys()
            .map(|v| (*v, plane.project(self.pos(*v))))
            .collect();
        for (v, list) in nbrs.iter_mut() {
            let o = p2[v];
            list.sort_by(|x, y| {
                let ax = (p2[x] - o).to_angle();
                let ay = (p2[y] - o).to_angle();
                ax.total_cmp(&ay)
            });
        }

        // Walk every directed side once; at each vertex turn as far left as possible.
        let mut visited: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        let mut cycles = Vec::new();
        for (&start, list) in &nbrs {
            for &next in list {
                if visited.contains(&(start, next)) {
                    continue;
                }
                let mut cycle = Vec::new();
                let (mut u, mut w) = (start, next);
                loop {
                    visited.insert((u, w));
                    cycle.push(u);
                    let around = &nbrs[&w];
                    let i = around.iter().position(|x| *x == u).unwrap();
                    let turn = around[(i + around.len() - 1) % around.len()];
                    (u, w) = (w, turn);
                    if (u, w) == (start, next) {
                        break;
                    }
                }
                let unique: BTreeSet<_> = cycle.iter().collect();
                let poly: Vec<DVec2> = cycle.iter().map(|v| p2[v]).collect();
                if unique.len() == cycle.len() && signed_area2(&poly) > LENGTH_TOL * LENGTH_TOL {
                    cycles.push(cycle);
                }
            }
        }
        cycles
    }

    fn face_with_loop(&self, c: &[VertexId]) -> bool {
        let set: BTreeSet<_> = c.iter().collect();
        self.faces().any(|(_, f)| {
            f.outer.len() == c.len() && f.outer.iter().collect::<BTreeSet<_>>() == set
        })
    }

    fn project_loop(&self, l: &[VertexId], plane: &Plane) -> Vec<DVec2> {
        l.iter().map(|v| plane.project(self.pos(*v))).collect()
    }

    /// A point strictly inside a counter-clockwise cycle: just left of the middle of its first side.
    fn interior_point(&self, c: &[VertexId], plane: &Plane) -> DVec2 {
        let poly = self.project_loop(c, plane);
        let (a, b) = (poly[0], poly[1]);
        let side = b - a;
        let left = side.perp().normalize();
        (a + b) * 0.5 + left * (side.length() * 1e-4).max(LENGTH_TOL * 10.0)
    }

    fn face_contains(&self, f: FaceId, p: DVec2, plane: &Plane) -> bool {
        let face = self.face(f).unwrap();
        point_in_polygon(p, &self.project_loop(&face.outer, plane))
            && !face
                .holes
                .iter()
                .any(|h| point_in_polygon(p, &self.project_loop(h, plane)))
    }

    fn shares_side_with_outer(&self, f: FaceId, c: &[VertexId]) -> bool {
        let outer = &self.face(f).unwrap().outer;
        let sides = |l: &[VertexId]| -> BTreeSet<(VertexId, VertexId)> {
            (0..l.len())
                .map(|i| {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    (a.min(b), a.max(b))
                })
                .collect()
        };
        !sides(outer).is_disjoint(&sides(c))
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kernel && cargo clippy -p kernel --all-targets -- -D warnings`
Expected: 11 unit + 10 edges + 11 faces passed; no warnings (the dead-code warning is gone).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/kernel/src/faces.rs crates/kernel/tests/faces.rs
git commit -m "Add automatic face creation"
```

---

### Task 6: Commands, queries and undo in `session`

**Files:**
- Modify: `crates/session/Cargo.toml`, `crates/session/src/lib.rs`
- Create: `crates/session/src/command.rs`, `crates/session/src/query.rs`, `crates/session/src/document.rs`, `crates/session/tests/commands.rs`

**Interfaces:**
- Consumes: `kernel::{Model, ChangeSet, ChangeSummary, EntityId, VertexId, EdgeId, FaceId}`, `session::Camera`.
- Produces (spec section 5, the surface M2.5 serves over JSON-RPC):
  - `Command` (serde, `"type"` tag, snake_case): `add_edge { start, end }`, `undo`, `redo`, `set_camera { eye, target, fov_y_deg: Option }`
  - `Outcome { changes: ChangeSummary }`; `CommandError { kind: ErrorKind, message }`; `ErrorKind`: `invalid_input`, `nothing_to_undo`, `nothing_to_redo`, `not_found`
  - `Query`: `entities`, `entity { id }`, `camera`, `stats`, `history`; `QueryResult` variants `entities`, `vertex`, `edge`, `face`, `camera`, `stats`, `history`; `VertexInfo`, `EdgeInfo`, `FaceInfo` (with `area`)
  - `Document { pub model, pub camera }`: `new(viewport: DVec2)`, `execute(Command) -> Result<Outcome, CommandError>`, `query(&Query) -> Result<QueryResult, CommandError>`

- [ ] **Step 1: Dependencies and exports**

`crates/session/Cargo.toml`:
```toml
[package]
name = "session"
version.workspace = true
edition.workspace = true

[dependencies]
kernel = { path = "../kernel" }
glam.workspace = true
serde.workspace = true

[dev-dependencies]
serde_json.workspace = true
```

`crates/session/src/lib.rs`:
```rust
//! The editable document and the command/query layer every client (UI, API, tests) goes through.

pub mod camera;
mod command;
mod document;
mod query;

pub use camera::{Camera, Ray};
pub use command::{Command, CommandError, ErrorKind, Outcome};
pub use document::Document;
pub use query::{EdgeInfo, FaceInfo, Query, QueryResult, VertexInfo};
```

- [ ] **Step 2: Write the failing tests**

`crates/session/tests/commands.rs`:
```rust
//! Drives the document only through serialized commands and queries, the way the API will.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query, QueryResult};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

/// Parses a JSON command, runs it, and returns the outcome as JSON.
fn run(d: &mut Document, cmd: Value) -> Result<Value, ErrorKind> {
    let cmd: Command = serde_json::from_value(cmd).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| e.kind)
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).expect("valid query json");
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn edge(d: &mut Document, a: [f64; 3], b: [f64; 3]) -> Value {
    run(d, json!({"type": "add_edge", "start": a, "end": b})).unwrap()
}

fn draw_square(d: &mut Document) {
    edge(d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
    edge(d, [2.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    edge(d, [2.0, 2.0, 0.0], [0.0, 2.0, 0.0]);
    edge(d, [0.0, 2.0, 0.0], [0.0, 0.0, 0.0]);
}

#[test]
fn add_edge_reports_created_ids_as_strings() {
    let mut d = doc();
    let out = edge(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    assert_eq!(out["changes"]["created"], json!(["v0", "v1", "e2"]));
}

#[test]
fn square_via_commands_makes_a_queryable_face() {
    let mut d = doc();
    draw_square(&mut d);
    let stats = ask(&d, json!({"type": "stats"}));
    assert_eq!(
        stats,
        json!({"type": "stats", "vertices": 4, "edges": 4, "faces": 1})
    );
    let all = ask(&d, json!({"type": "entities"}));
    let face = &all["faces"][0];
    assert_eq!(face["area"], json!(4.0));
    assert_eq!(face["normal"], json!([0.0, 0.0, 1.0]));
    let one = ask(&d, json!({"type": "entity", "id": face["id"]}));
    assert_eq!(one["type"], "face");
}

#[test]
fn undo_and_redo_walk_history() {
    let mut d = doc();
    draw_square(&mut d);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 0);
    assert_eq!(
        ask(&d, json!({"type": "history"})),
        json!({"type": "history", "undo_depth": 3, "redo_depth": 1})
    );
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 1);
    for _ in 0..4 {
        run(&mut d, json!({"type": "undo"})).unwrap();
    }
    assert_eq!(
        run(&mut d, json!({"type": "undo"})),
        Err(ErrorKind::NothingToUndo)
    );
    assert_eq!(ask(&d, json!({"type": "stats"}))["edges"], 0);
}

#[test]
fn new_edit_clears_redo() {
    let mut d = doc();
    edge(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    run(&mut d, json!({"type": "undo"})).unwrap();
    edge(&mut d, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    assert_eq!(
        run(&mut d, json!({"type": "redo"})),
        Err(ErrorKind::NothingToRedo)
    );
}

#[test]
fn duplicate_edge_adds_no_undo_step() {
    let mut d = doc();
    edge(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    edge(&mut d, [1.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
}

#[test]
fn invalid_input_is_a_structured_error() {
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [0, 0, 0]}),
    );
    assert_eq!(r, Err(ErrorKind::InvalidInput));
    let q: Query = serde_json::from_value(json!({"type": "entity", "id": "f99"})).unwrap();
    assert_eq!(d.query(&q).unwrap_err().kind, ErrorKind::NotFound);
    assert!(serde_json::from_value::<Query>(json!({"type": "entity", "id": "zz"})).is_err());
}

#[test]
fn set_camera_moves_camera_without_history() {
    let mut d = doc();
    run(&mut d, json!({"type": "set_camera", "eye": [10.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0], "fov_y_deg": 50.0})).unwrap();
    let cam = ask(&d, json!({"type": "camera"}));
    assert_eq!(cam["eye"], json!([10.0, 0.0, 5.0]));
    assert!((cam["fov_y_deg"].as_f64().unwrap() - 50.0).abs() < 1e-9);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 0);
    let vertical = run(
        &mut d,
        json!({"type": "set_camera", "eye": [0.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0], "fov_y_deg": null}),
    );
    assert_eq!(vertical, Err(ErrorKind::InvalidInput));
}

#[test]
fn query_results_round_trip_through_json() {
    let mut d = doc();
    draw_square(&mut d);
    let r = d.query(&Query::Entities).unwrap();
    let back: QueryResult = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
    assert_eq!(back, r);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p session --test commands`
Expected: compile errors (`file not found for module command`, ...).

- [ ] **Step 4: Implement**

`crates/session/src/command.rs`:
```rust
use std::fmt;

use kernel::ChangeSummary;
use serde::{Deserialize, Serialize};

/// Every state change a client can make. Values are explicit (meters, degrees, entity ids);
/// commands never refer to screen positions or cursor state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Draw a straight edge. Sticky-geometry rules apply (merging, splitting, auto faces).
    AddEdge {
        start: [f64; 3],
        end: [f64; 3],
    },
    Undo,
    Redo,
    /// Place the camera. Not recorded in undo history.
    SetCamera {
        eye: [f64; 3],
        target: [f64; 3],
        fov_y_deg: Option<f64>,
    },
}

/// What a successful command changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub changes: ChangeSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    NothingToUndo,
    NothingToRedo,
    NotFound,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandError {
    pub kind: ErrorKind,
    pub message: String,
}

impl CommandError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for CommandError {}
```

`crates/session/src/query.rs`:
```rust
use kernel::EntityId;
use serde::{Deserialize, Serialize};

/// Read-only questions about the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Query {
    /// Every vertex, edge and face.
    Entities,
    /// One entity by id ("v3", "e7", "f2").
    Entity {
        id: EntityId,
    },
    Camera,
    Stats,
    History,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VertexInfo {
    pub id: EntityId,
    pub position: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeInfo {
    pub id: EntityId,
    pub vertices: [EntityId; 2],
    pub start: [f64; 3],
    pub end: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceInfo {
    pub id: EntityId,
    pub outer: Vec<EntityId>,
    pub holes: Vec<Vec<EntityId>>,
    pub normal: [f64; 3],
    pub area: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryResult {
    Entities {
        vertices: Vec<VertexInfo>,
        edges: Vec<EdgeInfo>,
        faces: Vec<FaceInfo>,
    },
    Vertex(VertexInfo),
    Edge(EdgeInfo),
    Face(FaceInfo),
    Camera {
        eye: [f64; 3],
        target: [f64; 3],
        fov_y_deg: f64,
    },
    Stats {
        vertices: usize,
        edges: usize,
        faces: usize,
    },
    History {
        undo_depth: usize,
        redo_depth: usize,
    },
}
```

`crates/session/src/document.rs`:
```rust
use glam::{DVec2, DVec3};
use kernel::{ChangeSet, EdgeId, EntityId, FaceId, Model, VertexId};

use crate::camera::Camera;
use crate::command::{Command, CommandError, ErrorKind, Outcome};
use crate::query::{EdgeInfo, FaceInfo, Query, QueryResult, VertexInfo};

/// The open model plus everything needed to edit it: camera and undo history.
pub struct Document {
    pub model: Model,
    pub camera: Camera,
    undo: Vec<ChangeSet>,
    redo: Vec<ChangeSet>,
}

impl Document {
    pub fn new(viewport: DVec2) -> Self {
        Self {
            model: Model::new(),
            camera: Camera::new(viewport),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Runs one command. Each modeling command is one undo step; camera commands are not recorded.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        match command {
            Command::AddEdge { start, end } => {
                let changes = self
                    .model
                    .add_edge(DVec3::from(start), DVec3::from(end))
                    .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
                Ok(self.record(changes))
            }
            Command::Undo => {
                let changes = self.undo.pop().ok_or_else(|| {
                    CommandError::new(ErrorKind::NothingToUndo, "nothing to undo")
                })?;
                let inverse = changes.inverse();
                self.model.apply(&inverse);
                self.redo.push(changes);
                Ok(Outcome {
                    changes: inverse.summary(),
                })
            }
            Command::Redo => {
                let changes = self.redo.pop().ok_or_else(|| {
                    CommandError::new(ErrorKind::NothingToRedo, "nothing to redo")
                })?;
                self.model.apply(&changes);
                let summary = changes.summary();
                self.undo.push(changes);
                Ok(Outcome { changes: summary })
            }
            Command::SetCamera {
                eye,
                target,
                fov_y_deg,
            } => {
                let (eye, target) = (DVec3::from(eye), DVec3::from(target));
                let fov = fov_y_deg.unwrap_or(self.camera.fov_y.to_degrees());
                if !eye.is_finite() || !target.is_finite() || eye.distance(target) < 1e-6 {
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "eye and target must be distinct finite points",
                    ));
                }
                if (target - eye).normalize().cross(DVec3::Z).length() < 1e-9 {
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "view direction must not be vertical",
                    ));
                }
                if !(1.0..=170.0).contains(&fov) {
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "fov_y_deg must be between 1 and 170",
                    ));
                }
                self.camera.eye = eye;
                self.camera.target = target;
                self.camera.fov_y = fov.to_radians();
                Ok(Outcome::default())
            }
        }
    }

    fn record(&mut self, changes: ChangeSet) -> Outcome {
        let summary = changes.summary();
        if !changes.is_empty() {
            self.undo.push(changes);
            self.redo.clear();
        }
        Outcome { changes: summary }
    }

    pub fn query(&self, query: &Query) -> Result<QueryResult, CommandError> {
        let m = &self.model;
        Ok(match query {
            Query::Entities => QueryResult::Entities {
                vertices: m.vertices().map(|(id, _)| self.vertex_info(id)).collect(),
                edges: m.edges().map(|(id, _)| self.edge_info(id)).collect(),
                faces: m.faces().map(|(id, _)| self.face_info(id)).collect(),
            },
            Query::Entity { id } => {
                let missing = || CommandError::new(ErrorKind::NotFound, format!("no entity {id}"));
                match *id {
                    EntityId::Vertex(v) => QueryResult::Vertex(
                        m.vertex(v)
                            .map(|_| self.vertex_info(v))
                            .ok_or_else(missing)?,
                    ),
                    EntityId::Edge(e) => {
                        QueryResult::Edge(m.edge(e).map(|_| self.edge_info(e)).ok_or_else(missing)?)
                    }
                    EntityId::Face(f) => {
                        QueryResult::Face(m.face(f).map(|_| self.face_info(f)).ok_or_else(missing)?)
                    }
                }
            }
            Query::Camera => QueryResult::Camera {
                eye: self.camera.eye.to_array(),
                target: self.camera.target.to_array(),
                fov_y_deg: self.camera.fov_y.to_degrees(),
            },
            Query::Stats => QueryResult::Stats {
                vertices: m.vertices().count(),
                edges: m.edges().count(),
                faces: m.faces().count(),
            },
            Query::History => QueryResult::History {
                undo_depth: self.undo.len(),
                redo_depth: self.redo.len(),
            },
        })
    }

    fn vertex_info(&self, id: VertexId) -> VertexInfo {
        VertexInfo {
            id: EntityId::Vertex(id),
            position: self.model.pos(id).to_array(),
        }
    }

    fn edge_info(&self, id: EdgeId) -> EdgeInfo {
        let [a, b] = self.model.edge(id).unwrap().v;
        EdgeInfo {
            id: EntityId::Edge(id),
            vertices: [EntityId::Vertex(a), EntityId::Vertex(b)],
            start: self.model.pos(a).to_array(),
            end: self.model.pos(b).to_array(),
        }
    }

    fn face_info(&self, id: FaceId) -> FaceInfo {
        let f = self.model.face(id).unwrap();
        let ids = |l: &Vec<VertexId>| l.iter().map(|v| EntityId::Vertex(*v)).collect();
        FaceInfo {
            id: EntityId::Face(id),
            outer: ids(&f.outer),
            holes: f.holes.iter().map(ids).collect(),
            normal: f.normal.to_array(),
            area: self.model.face_area(id),
        }
    }
}
```

- [ ] **Step 5: Run the full suite and lints**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | grep "test result"`
Expected: no warnings; 72 tests pass (kernel 11 + 10 + 11, session 16 + 8, render 4 + 2, app 10).

- [ ] **Step 6: Smoke-check the app still runs**

Run `RUST_LOG=warn cargo run -p app` in the background for about 8 seconds; confirm the process is alive with no panic or wgpu validation error, then stop it. (The app does not use `Document` yet; that is M2b.)

- [ ] **Step 7: Commit**

```bash
git add crates/session/Cargo.toml crates/session/src/lib.rs crates/session/src/command.rs crates/session/src/query.rs crates/session/src/document.rs crates/session/tests/commands.rs Cargo.lock
git commit -m "Add session commands, queries and undo"
```
