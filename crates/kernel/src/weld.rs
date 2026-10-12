//! Welding: after a push/pull brings a face flush with the geometry around it, the vertices
//! that met merge, collapsed edges and faces go, and the seams left in a flat surface heal.

use std::collections::{BTreeMap, BTreeSet};

use crate::edit::insert_between;
use crate::geom::{LENGTH_TOL, Plane, project_on_segment, vector_area};
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
                .vertices_near(p, LENGTH_TOL)
                .into_iter()
                .find(|v| !moved.contains(v));
            if let Some(f) = other {
                target.insert(m, f);
            }
        }
        if target.is_empty() {
            return;
        }
        let to = |v: &VertexId| *target.get(v).unwrap_or(v);

        let users: BTreeSet<FaceId> = target.keys().flat_map(|v| self.faces_at(*v)).collect();
        let users: Vec<(FaceId, Face)> = users
            .into_iter()
            .map(|id| (id, self.face(id).unwrap().clone()))
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
            self.replace_face(id, Face { outer, holes, ..f });
        }

        let joined: BTreeSet<EdgeId> = target.keys().flat_map(|v| self.edges_at(*v)).collect();
        let joined: Vec<(EdgeId, [VertexId; 2])> = joined
            .into_iter()
            .map(|id| (id, self.edge(id).unwrap().v))
            .collect();
        for (e, [a, b]) in joined {
            let old = self.remove_edge(e);
            let (a, b) = (to(&a), to(&b));
            if a == b {
                continue;
            }
            // Vertices left on the re-joined edge split it, in the edge and in the face loops.
            let (pa, pb) = (self.pos(a), self.pos(b));
            let mut along: Vec<(f64, VertexId)> = self
                .vertices()
                .filter(|(v, _)| *v != a && *v != b && !target.contains_key(v))
                .filter_map(|(v, vert)| {
                    let (t, dist) = project_on_segment(vert.pos, pa, pb);
                    (dist < LENGTH_TOL
                        && t > 0.0
                        && t * pa.distance(pb) > LENGTH_TOL
                        && (1.0 - t) * pa.distance(pb) > LENGTH_TOL)
                        .then_some((t, v))
                })
                .collect();
            along.sort_by(|x, y| x.0.total_cmp(&y.0));
            let mut chain = vec![a];
            chain.extend(along.iter().map(|(_, v)| *v));
            chain.push(b);
            if chain.len() > 2 {
                for id in self.faces_on_side(a, b) {
                    let mut f = self.face(id).unwrap().clone();
                    for l in std::iter::once(&mut f.outer).chain(f.holes.iter_mut()) {
                        for w in 1..chain.len() - 1 {
                            insert_between(l, chain[w - 1], b, chain[w]);
                        }
                    }
                    self.replace_face(id, f);
                }
            }
            for w in chain.windows(2) {
                if self.edge_between(w[0], w[1]).is_none() {
                    self.insert_edge_like(w[0], w[1], old.curve, old.soft);
                }
            }
        }
        for &m in target.keys() {
            self.remove_vertex(m);
        }
        self.merge_seams(target.values().copied().collect());
    }

    /// Removes every edge at a vertex of `met` between exactly two faces in one plane facing
    /// the same way, merging those faces; then removes the vertices left without edges and heals
    /// those left between two collinear edges.
    pub(crate) fn merge_seams(&mut self, met: BTreeSet<VertexId>) {
        let at_met: BTreeSet<EdgeId> = met.iter().flat_map(|v| self.edges_at(*v)).collect();
        let seams: Vec<(EdgeId, [VertexId; 2], Plane)> = at_met
            .into_iter()
            .filter_map(|id| {
                let e = self.edge(id)?;
                let [a, b] = e.v;
                let users = self.faces_on_side(a, b);
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
