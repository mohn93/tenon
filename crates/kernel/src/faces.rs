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
