//! Welding: after a push/pull brings a face flush with the geometry around it, the vertices
//! that met merge, collapsed edges and faces go, and the seams left in a flat surface heal.

use std::collections::{BTreeMap, BTreeSet};

use crate::edit::{insert_between, loop_has_side};
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
            self.replace_face(id, Face { outer, holes, ..f });
        }

        let joined: Vec<(EdgeId, [VertexId; 2])> = self
            .edges()
            .filter(|(_, e)| e.v.iter().any(|v| target.contains_key(v)))
            .map(|(id, e)| (id, e.v))
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
                let users: Vec<FaceId> = self
                    .faces()
                    .filter(|(_, f)| loops(f).any(|l| loop_has_side(l, a, b)))
                    .map(|(id, _)| id)
                    .collect();
                for id in users {
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
