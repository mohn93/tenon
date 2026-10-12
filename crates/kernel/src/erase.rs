//! Erasing edges and faces, with SketchUp-style healing.

use std::collections::BTreeSet;

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
            let users = self.faces_on_side(a, b);
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
    pub(crate) fn shared_plane(&self, f: FaceId, g: FaceId) -> Option<Plane> {
        let (ff, gg) = (self.face(f)?, self.face(g)?);
        let plane = Plane::through(self.pos(ff.outer[0]), ff.normal);
        let same = ff.normal.cross(gg.normal).length() < PLANE_ANGLE_TOL
            && plane.contains(self.pos(gg.outer[0]));
        same.then_some(plane)
    }

    /// Joins the two edges at `v` into one if they are collinear and nothing else meets there,
    /// removing `v` from the loops of the faces around it.
    pub(crate) fn heal(&mut self, v: VertexId) {
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
            .faces_at(v)
            .map(|id| (id, self.face(id).unwrap().clone()))
            .collect();
        let (first, second) = (self.remove_edge(e1), self.remove_edge(e2));
        // The joined edge keeps what the two had in common: their curve, and being soft.
        let curve = first.curve.filter(|c| second.curve == Some(*c));
        self.insert_edge_like(p, q, curve, first.soft && second.soft);
        for (id, mut f) in users {
            f.outer.retain(|x| *x != v);
            f.holes.iter_mut().for_each(|h| h.retain(|x| *x != v));
            self.replace_face(id, f);
        }
        self.remove_vertex(v);
    }
}
