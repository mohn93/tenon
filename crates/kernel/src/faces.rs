//! Automatic face creation: after edges are added, the faces of every affected plane are rebuilt
//! from the bounded regions of that plane's edge graph.

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec2;

use crate::geom::{PLANE_ANGLE_TOL, Plane, point_in_polygon};
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

    /// Planes spanned by edge e and each non-collinear edge reachable from its endpoints,
    /// walking through vertices whose edges continue along e's line.
    fn candidate_planes(&self, e: EdgeId) -> Vec<Plane> {
        let [a, b] = self.edge(e).unwrap().v;
        let (pa, pb) = (self.pos(a), self.pos(b));
        let mut planes = Vec::new();
        for start in [a, b] {
            let mut visited: BTreeSet<VertexId> = BTreeSet::from([a, b]);
            let mut stack = vec![start];
            while let Some(v) = stack.pop() {
                for f in self.edges_at(v) {
                    if f == e {
                        continue;
                    }
                    let fv = self.edge(f).unwrap().v;
                    let c = if fv[0] == v { fv[1] } else { fv[0] };
                    match Plane::from_points(pa, pb, self.pos(c)) {
                        Some(p) => planes.push(p),
                        None => {
                            if visited.insert(c) {
                                stack.push(c);
                            }
                        }
                    }
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
        f.normal.cross(plane.normal).length() < PLANE_ANGLE_TOL
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

        let old_sides: BTreeSet<(VertexId, VertexId)> = old
            .iter()
            .flat_map(|(_, f)| std::iter::once(&f.outer).chain(&f.holes))
            .flat_map(|l| (0..l.len()).map(|i| side(l[i], l[(i + 1) % l.len()])))
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
                // Inside an opening, only a loop drawn apart from every old face's boundary
                // gets a face; a line across the opening leaves its pieces open.
                let apart = (0..r.outer.len()).all(|i| {
                    !old_sides.contains(&side(r.outer[i], r.outer[(i + 1) % r.outer.len()]))
                });
                in_old_face || (uses_new && (!in_old_hole || apart))
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

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;

    fn square(m: &mut Model, x0: f64, y0: f64, s: f64) {
        let c = [(x0, y0), (x0 + s, y0), (x0 + s, y0 + s), (x0, y0 + s)];
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            m.add_edge(DVec3::new(a.0, a.1, 0.0), DVec3::new(b.0, b.1, 0.0))
                .unwrap();
            m.validate().unwrap();
        }
    }

    /// A 4x4 face with a 2x2 opening whose inner face was removed. Returns the 4x4 face.
    fn face_with_opening() -> (Model, FaceId, Face) {
        let mut m = Model::new();
        square(&mut m, 0.0, 0.0, 4.0);
        square(&mut m, 1.0, 1.0, 2.0);
        let inner = m.faces().find(|(_, f)| f.holes.is_empty()).unwrap().0;
        m.remove_face(inner);
        m.take_changes();
        let (id, f) = m.faces().next().unwrap();
        let f = f.clone();
        assert_eq!(m.faces().count(), 1);
        assert_eq!(m.face_area(id), 12.0);
        (m, id, f)
    }

    #[test]
    fn loop_drawn_inside_an_opening_is_filled() {
        let (mut m, outer, before) = face_with_opening();
        square(&mut m, 1.5, 1.5, 0.5);
        assert_eq!(m.faces().count(), 2);
        assert_eq!(m.face(outer), Some(&before));
        let small = m.faces().find(|(id, _)| *id != outer).unwrap().0;
        assert!((m.face_area(small) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn line_across_an_opening_leaves_it_open() {
        let (mut m, outer, _) = face_with_opening();
        m.add_edge(DVec3::new(1.0, 1.5, 0.0), DVec3::new(3.0, 1.5, 0.0))
            .unwrap();
        m.validate().unwrap();
        assert_eq!(m.faces().count(), 1);
        assert_eq!(m.face_area(outer), 12.0);
    }
}
