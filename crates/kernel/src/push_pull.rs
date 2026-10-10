//! Push/pull: move a face along its normal, extruding new sides or stretching the faces around it.

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec3;

use crate::edit::KernelError;
use crate::geom::{LENGTH_TOL, PLANE_ANGLE_TOL, Plane};
use crate::ids::{FaceId, VertexId};
use crate::model::{ChangeSet, Face, Model};

impl Model {
    /// Pushes or pulls face `id` by `distance` meters along its normal (negative: against it).
    ///
    /// If every face touching it runs along the push direction (the top of a box), the face
    /// moves and those faces stretch. Otherwise it is extruded: a side face for every side of
    /// every loop, plus a cap. The original face stays as the base when it lies in a flat sheet
    /// and is removed when it is part of a solid's surface. Extruded faces point out of the solid.
    pub fn push_pull(&mut self, id: FaceId, distance: f64) -> Result<ChangeSet, KernelError> {
        let face = self.face(id).ok_or(KernelError::FaceNotFound)?.clone();
        if !distance.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if distance.abs() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        if self.stretches_around(id, &face) {
            self.move_face(id, &face, face.normal * distance)?;
        } else {
            self.check_unobstructed(id, distance)?;
            self.extrude(id, &face, distance);
        }
        Ok(self.take_changes())
    }

    /// True if moving the face along its normal keeps every touching face planar: each face
    /// sharing a vertex either moves with it entirely or contains the normal direction, every
    /// edge leaving the face runs along the normal, and at least one face stretches.
    fn stretches_around(&self, id: FaceId, face: &Face) -> bool {
        let moving = loop_vertices(face);
        let n = face.normal;
        let mut stretching = false;
        for (fid, f) in self.faces() {
            if fid == id {
                continue;
            }
            let verts = loop_vertices(f);
            if verts.is_disjoint(&moving) || verts.is_subset(&moving) {
                continue;
            }
            if f.normal.dot(n).abs() > PLANE_ANGLE_TOL {
                return false;
            }
            stretching = true;
        }
        let along_normal = self.edges().all(|(_, e)| {
            let [a, b] = e.v;
            if moving.contains(&a) == moving.contains(&b) {
                return true;
            }
            (self.pos(b) - self.pos(a)).normalize().cross(n).length() < PLANE_ANGLE_TOL
        });
        stretching && along_normal
    }

    /// Moves every vertex of face `id` by `offset`, after checking that no edge leaving the face
    /// collapses or flips, no vertex lands on another and the face passes through no other face.
    fn move_face(&mut self, id: FaceId, face: &Face, offset: DVec3) -> Result<(), KernelError> {
        let moving = loop_vertices(face);
        for (_, e) in self.edges() {
            let [a, b] = e.v;
            let (m, fixed) = match (moving.contains(&a), moving.contains(&b)) {
                (true, false) => (a, b),
                (false, true) => (b, a),
                _ => continue,
            };
            let before = self.pos(m) - self.pos(fixed);
            let after = before + offset;
            if after.length() < LENGTH_TOL || after.dot(before) <= 0.0 {
                return Err(KernelError::Collapse);
            }
        }
        // Every face that stretches must keep its fixed vertices on the side of the face they
        // started on; otherwise its loops fold over (a hole could end up outside its outer loop).
        let n = face.normal;
        let origin = self.pos(face.outer[0]);
        let d = offset.dot(n);
        for (_, f) in self.faces() {
            let verts = loop_vertices(f);
            if verts.is_disjoint(&moving) || verts.is_subset(&moving) {
                continue;
            }
            for v in verts.difference(&moving) {
                let h = (self.pos(*v) - origin).dot(n);
                if (h - d).abs() < LENGTH_TOL || h * (h - d) <= 0.0 {
                    return Err(KernelError::Collapse);
                }
            }
        }
        for &m in &moving {
            let to = self.pos(m) + offset;
            let lands_on_other = self
                .vertices()
                .any(|(v, vert)| !moving.contains(&v) && vert.pos.distance(to) < LENGTH_TOL);
            if lands_on_other {
                return Err(KernelError::Collapse);
            }
        }
        self.check_unobstructed(id, offset.dot(n))?;
        for m in moving {
            let to = self.pos(m) + offset;
            self.move_vertex(m, to);
        }
        Ok(())
    }

    fn extrude(&mut self, id: FaceId, face: &Face, distance: f64) {
        let n = face.normal;
        let offset = n * distance;
        let in_solid = self.in_solid(id);
        let loops: Vec<&Vec<VertexId>> = std::iter::once(&face.outer).chain(&face.holes).collect();
        let points: Vec<Vec<DVec3>> = loops
            .iter()
            .map(|l| l.iter().map(|v| self.pos(*v)).collect())
            .collect();
        let before: BTreeSet<FaceId> = self.faces().map(|(f, _)| f).collect();
        // Planes of the faces each loop side already bordered, so a cut can tell which of its
        // sides open onto an existing face (a notch at a box edge).
        let bordered: Vec<Plane> = self
            .faces()
            .filter(|(f, _)| *f != id)
            .filter_map(|(_, g)| {
                let vs = loop_vertices(g);
                loops
                    .iter()
                    .any(|l| {
                        (0..l.len())
                            .any(|i| vs.contains(&l[i]) && vs.contains(&l[(i + 1) % l.len()]))
                    })
                    .then(|| {
                        let o = self.pos(g.outer[0]);
                        Plane {
                            normal: g.normal,
                            d: g.normal.dot(o),
                        }
                    })
            })
            .collect();

        if in_solid {
            self.remove_face(id);
        }
        for l in &points {
            for p in l {
                self.draw_segment(*p, *p + offset);
            }
        }
        for l in &points {
            for i in 0..l.len() {
                self.draw_segment(l[i] + offset, l[(i + 1) % l.len()] + offset);
            }
        }

        // Each hole's cap loop closes inside the cap, which fills it like any loop drawn on a
        // face; that fill is removed so the hole stays open.
        let hole_caps: Vec<BTreeSet<VertexId>> = points[1..]
            .iter()
            .filter_map(|l| l.iter().map(|p| self.find_vertex(*p + offset)).collect())
            .collect();
        // A solid grows outward when pulled along the normal and is cut when pushed against it.
        let cut = in_solid && distance < 0.0;
        let cap_normal = if in_solid || distance > 0.0 { n } else { -n };
        let cap_origin = points[0][0] + offset;
        let created: Vec<FaceId> = self
            .faces()
            .map(|(f, _)| f)
            .filter(|f| !before.contains(f))
            .collect();
        for f in created {
            let g = self.face(f).unwrap().clone();
            let outer: BTreeSet<VertexId> = g.outer.iter().copied().collect();
            if hole_caps.contains(&outer) {
                self.remove_face(f);
                continue;
            }
            let in_cap = g.normal.cross(n).length() < PLANE_ANGLE_TOL
                && (self.pos(g.outer[0]) - cap_origin).dot(n).abs() < LENGTH_TOL;
            if in_cap {
                self.orient(f, cap_normal);
                continue;
            }
            // A side face holds a loop side of the pushed face as consecutive outer vertices.
            let side_of = loops.iter().find_map(|l| {
                (0..l.len()).find_map(|i| {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    has_side(&g.outer, a, b).then_some((a, b))
                })
            });
            let Some((a, b)) = side_of else {
                continue;
            };
            let (pa, pb) = (self.pos(a), self.pos(b));
            if cut {
                let opens = Plane::from_points(pa, pb, pa + offset)
                    .is_some_and(|sp| bordered.iter().any(|q| q.same_as(&sp)));
                if opens {
                    // The cut reaches an existing face in this side's plane: leave it open.
                    self.remove_face(f);
                    let stray = self.edge_between(a, b).filter(|_| {
                        !self.faces().any(|(_, h)| {
                            std::iter::once(&h.outer)
                                .chain(&h.holes)
                                .any(|l| has_side(l, a, b))
                        })
                    });
                    if let Some(e) = stray {
                        self.remove_edge(e);
                    }
                    continue;
                }
            }
            let out = (pb - pa).cross(n).normalize();
            self.orient(f, if cut { -out } else { out });
        }
        if !in_solid {
            self.orient(id, if distance > 0.0 { -n } else { n });
        }
    }

    /// Fails with `Obstructed` if face `id`, moved `distance` along its normal, would meet another
    /// face: a ray from a point inside it hits one within that distance.
    fn check_unobstructed(&self, id: FaceId, distance: f64) -> Result<(), KernelError> {
        let Some(tris) = self.triangulate(id) else {
            return Ok(());
        };
        let Some([a, b, c]) = tris.first().copied() else {
            return Ok(());
        };
        let from = (a + b + c) / 3.0;
        let dir = self.face(id).unwrap().normal * distance.signum();
        let reach = distance.abs() + LENGTH_TOL;
        let hit = self
            .faces()
            .filter(|(f, _)| *f != id)
            .flat_map(|(f, _)| self.triangulate(f).unwrap_or_default())
            .filter_map(|t| ray_hits_triangle(from, dir, t))
            .any(|t| t > LENGTH_TOL && t <= reach);
        if hit {
            Err(KernelError::Obstructed)
        } else {
            Ok(())
        }
    }

    /// Flips face `id` if its normal points away from `want`.
    fn orient(&mut self, id: FaceId, want: DVec3) {
        let f = self.face(id).unwrap();
        if f.normal.dot(want) >= 0.0 {
            return;
        }
        let reversed = |l: &Vec<VertexId>| l.iter().rev().copied().collect::<Vec<_>>();
        let flipped = Face {
            outer: reversed(&f.outer),
            holes: f.holes.iter().map(reversed).collect(),
            normal: -f.normal,
        };
        self.replace_face(id, flipped);
    }

    /// True if the face is part of a solid's surface with the solid behind it: it is connected
    /// through shared sides to a face with a vertex behind its plane. Faces connected only to
    /// faces in its plane or in front of it (a flat sheet, the ground beside a box) are not.
    fn in_solid(&self, id: FaceId) -> bool {
        let mut by_side: BTreeMap<(VertexId, VertexId), Vec<FaceId>> = BTreeMap::new();
        for (fid, f) in self.faces() {
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    by_side
                        .entry(side(l[i], l[(i + 1) % l.len()]))
                        .or_default()
                        .push(fid);
                }
            }
        }
        let n = self.face(id).unwrap().normal;
        let origin = self.pos(self.face(id).unwrap().outer[0]);
        let mut seen = BTreeSet::from([id]);
        let mut stack = vec![id];
        while let Some(fid) = stack.pop() {
            let f = self.face(fid).unwrap();
            let behind = loop_vertices(f)
                .iter()
                .any(|v| (self.pos(*v) - origin).dot(n) < -LENGTH_TOL);
            if behind {
                return true;
            }
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                for i in 0..l.len() {
                    for &next in &by_side[&side(l[i], l[(i + 1) % l.len()])] {
                        if seen.insert(next) {
                            stack.push(next);
                        }
                    }
                }
            }
        }
        false
    }
}

fn loop_vertices(f: &Face) -> BTreeSet<VertexId> {
    std::iter::once(&f.outer)
        .chain(&f.holes)
        .flatten()
        .copied()
        .collect()
}

/// True if `a` and `b` are consecutive in loop `l`, in either order (wrapping).
fn has_side(l: &[VertexId], a: VertexId, b: VertexId) -> bool {
    (0..l.len()).any(|i| {
        let (x, y) = (l[i], l[(i + 1) % l.len()]);
        (x, y) == (a, b) || (x, y) == (b, a)
    })
}

/// Distance along unit `dir` from `from` to triangle `t`, if the ray meets it.
fn ray_hits_triangle(from: DVec3, dir: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let (e1, e2) = (b - a, c - a);
    let h = dir.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = from - a;
    let u = s.dot(h) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    let eps = 1e-12;
    (u >= -eps && v >= -eps && u + v <= 1.0 + eps).then(|| e2.dot(q) / det)
}

fn side(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    (a.min(b), a.max(b))
}
