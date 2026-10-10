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
    /// Returns the changes as one ChangeSet. It is empty only when nothing changed; it can be
    /// non-empty without a new edge when an existing edge was only split.
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
