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

    pub(crate) fn pos(&self, id: VertexId) -> DVec3 {
        self.vertices[&id].pos
    }

    /// Position of a vertex, or None if it does not exist.
    pub fn position(&self, id: VertexId) -> Option<DVec3> {
        self.vertices.get(&id).map(|v| v.pos)
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

    /// Area of a face (outer area minus holes), or None if it does not exist.
    pub fn area(&self, id: FaceId) -> Option<f64> {
        self.faces.contains_key(&id).then(|| self.face_area(id))
    }

    pub(crate) fn face_area(&self, id: FaceId) -> f64 {
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
            if f.outer.is_empty() {
                return Err(format!("face {id:?} has an empty outer loop"));
            }
            if f.outer
                .iter()
                .chain(f.holes.iter().flatten())
                .any(|v| !self.vertices.contains_key(v))
            {
                return Err(format!("face {id:?} references a missing vertex"));
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
    fn validate_checks_face_invariants() {
        // Unit right triangle on the ground with its three sides as edges.
        let base = || {
            let mut m = Model::new();
            let a = m.insert_vertex(DVec3::ZERO);
            let b = m.insert_vertex(DVec3::X);
            let c = m.insert_vertex(DVec3::Y);
            m.insert_edge(a, b);
            m.insert_edge(b, c);
            m.insert_edge(c, a);
            (m, a, b, c)
        };

        let (mut m, a, b, c) = base();
        m.insert_face(Face {
            outer: vec![a, b, c],
            holes: vec![],
            normal: DVec3::Z,
        });
        assert!(m.validate().is_ok());

        let bad_faces: Vec<(&str, Face)> = vec![
            (
                "empty outer loop",
                Face {
                    outer: vec![],
                    holes: vec![],
                    normal: DVec3::Z,
                },
            ),
            (
                "missing vertex",
                Face {
                    outer: vec![a, b, VertexId(999)],
                    holes: vec![],
                    normal: DVec3::Z,
                },
            ),
            (
                "wrong winding",
                Face {
                    outer: vec![a, c, b],
                    holes: vec![],
                    normal: DVec3::Z,
                },
            ),
            (
                "non-unit normal",
                Face {
                    outer: vec![a, b, c],
                    holes: vec![],
                    normal: DVec3::new(0.0, 0.0, 2.0),
                },
            ),
        ];
        for (what, face) in bad_faces {
            let (mut m, _, _, _) = base();
            m.insert_face(face);
            assert!(m.validate().is_err(), "{what} was accepted");
        }
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
