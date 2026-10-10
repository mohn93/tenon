//! Automatic face creation: after edges are added, the faces of every affected plane are rebuilt
//! from the bounded regions of that plane's edge graph.

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec2;

use crate::geom::{ANGLE_TOL, Plane, point_in_polygon};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::{Face, Model};
use crate::regions::{PlaneGraph, Region};

impl Model {
    /// Updates the faces of every plane spanned by one of `new_edges` and an edge touching it.
    pub(crate) fn create_faces(&mut self, new_edges: &[EdgeId]) {
        let mut done: Vec<Plane> = Vec::new();
        for &e in new_edges {
            for plane in self.candidate_planes(e) {
                if done.iter().any(|p| p.same_as(&plane)) {
                    continue;
                }
                done.push(plane);
                self.rebuild_plane(plane, new_edges);
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

    /// The plane's orientation taken from an existing face in it, so all its faces agree.
    fn oriented(&self, plane: Plane) -> Plane {
        self.faces()
            .find(|(_, f)| self.face_in_plane(f, &plane))
            .map(|(_, f)| Plane {
                normal: f.normal,
                d: f.normal.dot(self.pos(f.outer[0])),
            })
            .unwrap_or(plane)
    }

    fn face_in_plane(&self, f: &Face, plane: &Plane) -> bool {
        f.normal.cross(plane.normal).length() < ANGLE_TOL * 1e3
            && plane.contains(self.pos(f.outer[0]))
    }

    /// Recomputes the regions of `plane` and makes the face set match them.
    fn rebuild_plane(&mut self, plane: Plane, new_edges: &[EdgeId]) {
        let plane = self.oriented(plane);
        let old: Vec<(FaceId, Face)> = self
            .faces()
            .filter(|(_, f)| self.face_in_plane(f, &plane))
            .map(|(id, f)| (id, f.clone()))
            .collect();

        let edges: Vec<[VertexId; 2]> = self
            .edges()
            .map(|(_, e)| e.v)
            .filter(|v| plane.contains(self.pos(v[0])) && plane.contains(self.pos(v[1])))
            .collect();
        let pos: BTreeMap<VertexId, DVec2> = edges
            .iter()
            .flatten()
            .map(|v| (*v, plane.project(self.pos(*v))))
            .collect();
        let graph = PlaneGraph { pos, edges };
        let project = |l: &[VertexId]| -> Vec<DVec2> {
            l.iter().map(|v| plane.project(self.pos(*v))).collect()
        };
        let old_polys: Vec<(Vec<DVec2>, Vec<Vec<DVec2>>)> = old
            .iter()
            .map(|(_, f)| {
                (
                    project(&f.outer),
                    f.holes.iter().map(|h| project(h)).collect(),
                )
            })
            .collect();
        let new_sides: BTreeSet<(VertexId, VertexId)> = new_edges
            .iter()
            .filter_map(|e| self.edge(*e))
            .map(|e| side(e.v[0], e.v[1]))
            .collect();

        let wanted: Vec<Region> = graph
            .regions()
            .into_iter()
            .filter(|r| {
                let Some(p) = graph.interior_point(r) else {
                    return false;
                };
                let in_old_face = old_polys.iter().any(|(o, hs)| {
                    point_in_polygon(p, o) && !hs.iter().any(|h| point_in_polygon(p, h))
                });
                let in_old_hole = old_polys
                    .iter()
                    .any(|(_, hs)| hs.iter().any(|h| point_in_polygon(p, h)));
                let uses_new = std::iter::once(&r.outer).chain(&r.holes).any(|l| {
                    (0..l.len()).any(|i| new_sides.contains(&side(l[i], l[(i + 1) % l.len()])))
                });
                in_old_face || (uses_new && !in_old_hole)
            })
            .collect();

        self.reconcile(&old, wanted, plane);
    }

    /// Keeps old faces that match a wanted region, updates their holes if needed, inserts the
    /// rest, and removes old faces that match nothing. All through recorded primitives.
    fn reconcile(&mut self, old: &[(FaceId, Face)], wanted: Vec<Region>, plane: Plane) {
        let mut matched: BTreeSet<FaceId> = BTreeSet::new();
        for r in wanted {
            let outer: BTreeSet<VertexId> = r.outer.iter().copied().collect();
            let same = old.iter().find(|(id, f)| {
                !matched.contains(id)
                    && f.normal.dot(plane.normal) > 0.0
                    && f.outer.iter().copied().collect::<BTreeSet<_>>() == outer
            });
            match same {
                Some((id, f)) => {
                    matched.insert(*id);
                    if loop_sets(&f.holes) != loop_sets(&r.holes) {
                        let face = Face {
                            outer: f.outer.clone(),
                            holes: r.holes,
                            normal: f.normal,
                        };
                        self.replace_face(*id, face);
                    }
                }
                None => {
                    self.insert_face(Face {
                        outer: r.outer,
                        holes: r.holes,
                        normal: plane.normal,
                    });
                }
            }
        }
        for (id, _) in old {
            if !matched.contains(id) {
                self.remove_face(*id);
            }
        }
    }
}

fn side(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    (a.min(b), a.max(b))
}

fn loop_sets(loops: &[Vec<VertexId>]) -> BTreeSet<BTreeSet<VertexId>> {
    loops.iter().map(|l| l.iter().copied().collect()).collect()
}
