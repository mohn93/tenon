//! Push/pull: move a face along its normal, extruding new sides or stretching the faces around it.

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec3;

use crate::edit::KernelError;
use crate::geom::{LENGTH_TOL, PLANE_ANGLE_TOL};
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
            self.move_face(&face, face.normal * distance)?;
        } else {
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

    /// Moves every vertex of the face by `offset`, after checking that no edge leaving the face
    /// collapses or flips and no vertex lands on another.
    fn move_face(&mut self, face: &Face, offset: DVec3) -> Result<(), KernelError> {
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
        for &m in &moving {
            let to = self.pos(m) + offset;
            let lands_on_other = self
                .vertices()
                .any(|(v, vert)| !moving.contains(&v) && vert.pos.distance(to) < LENGTH_TOL);
            if lands_on_other {
                return Err(KernelError::Collapse);
            }
        }
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
            let want = if in_cap {
                Some(cap_normal)
            } else {
                loops.iter().find_map(|l| {
                    (0..l.len()).find_map(|i| {
                        let (a, b) = (l[i], l[(i + 1) % l.len()]);
                        (outer.contains(&a) && outer.contains(&b)).then(|| {
                            let out = (self.pos(b) - self.pos(a)).cross(n).normalize();
                            if cut { -out } else { out }
                        })
                    })
                })
            };
            if let Some(want) = want {
                self.orient(f, want);
            }
        }
        if !in_solid {
            self.orient(id, if distance > 0.0 { -n } else { n });
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

    /// True if the face is connected through shared sides to a face in another plane, i.e. it
    /// is part of a solid's surface rather than a flat sheet.
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
        let mut seen = BTreeSet::from([id]);
        let mut stack = vec![id];
        while let Some(fid) = stack.pop() {
            let f = self.face(fid).unwrap();
            if f.normal.cross(n).length() >= PLANE_ANGLE_TOL {
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

fn side(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    (a.min(b), a.max(b))
}
