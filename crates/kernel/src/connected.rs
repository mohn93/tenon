//! The faces along an edge, and the geometry connected to an edge or a face.

use std::collections::{BTreeSet, VecDeque};

use crate::ids::{EdgeId, EntityId, FaceId, VertexId};
use crate::model::Model;

impl Model {
    /// The faces with a loop side along edge `id` (either way round), in id order; empty for
    /// an unknown edge.
    pub fn faces_of_edge(&self, id: EdgeId) -> Vec<FaceId> {
        match self.edge(id) {
            Some(e) => self.faces_on_side(e.v[0], e.v[1]),
            None => Vec::new(),
        }
    }

    /// Every edge and face joined to `seed` (an edge or a face) through shared vertices:
    /// edges in id order, then faces in id order, `seed` among them. Empty for an unknown id,
    /// a vertex, an instance or a guide. The walk crosses faces as well as edges, so a face's
    /// holes are joined to its outer loop and an edge or a face of one piece gives the same set.
    pub fn connected(&self, seed: EntityId) -> Vec<EntityId> {
        let start: Vec<VertexId> = match seed {
            EntityId::Edge(id) => match self.edge(id) {
                Some(e) => e.v.to_vec(),
                None => return Vec::new(),
            },
            EntityId::Face(id) => match self.face(id) {
                Some(f) => f.outer.clone(),
                None => return Vec::new(),
            },
            _ => return Vec::new(),
        };
        let mut seen: BTreeSet<VertexId> = BTreeSet::new();
        let mut edges: BTreeSet<EdgeId> = BTreeSet::new();
        let mut faces: BTreeSet<FaceId> = BTreeSet::new();
        let mut queue: VecDeque<VertexId> = start.into_iter().collect();
        while let Some(v) = queue.pop_front() {
            if !seen.insert(v) {
                continue;
            }
            for e in self.edges_at(v) {
                if edges.insert(e) {
                    queue.extend(self.edge(e).expect("edge at a vertex").v);
                }
            }
            for f in self.faces_at(v) {
                if faces.insert(f) {
                    let f = self.face(f).expect("face at a vertex");
                    queue.extend(f.outer.iter().chain(f.holes.iter().flatten()));
                }
            }
        }
        edges
            .into_iter()
            .map(EntityId::Edge)
            .chain(faces.into_iter().map(EntityId::Face))
            .collect()
    }
}
