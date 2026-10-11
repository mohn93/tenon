//! Moving and copying geometry.

use std::collections::BTreeSet;

use glam::DVec3;

use crate::edit::KernelError;
use crate::geom::{LENGTH_TOL, vector_area};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::{ChangeSet, Face, Model};

impl Model {
    /// Moves `vertices` by `offset` as one ChangeSet. Faces that use some of them stretch and
    /// may tilt, but must stay flat and keep their winding; edges between moved and fixed
    /// vertices must keep a length; no moved vertex may land on a fixed one. Moved geometry is
    /// not merged with what it ends up touching.
    pub fn move_vertices(
        &mut self,
        vertices: &BTreeSet<VertexId>,
        offset: DVec3,
    ) -> Result<ChangeSet, KernelError> {
        if !offset.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if offset.length() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        let moved: BTreeSet<VertexId> = vertices
            .iter()
            .copied()
            .filter(|v| self.vertex(*v).is_some())
            .collect();
        let at = |m: &Model, v: VertexId| {
            m.pos(v)
                + if moved.contains(&v) {
                    offset
                } else {
                    DVec3::ZERO
                }
        };

        for (_, e) in self.edges() {
            let [a, b] = e.v;
            if moved.contains(&a) != moved.contains(&b)
                && at(self, a).distance(at(self, b)) < LENGTH_TOL
            {
                return Err(KernelError::Squash);
            }
        }
        for &v in &moved {
            let to = self.pos(v) + offset;
            let lands = self
                .vertices()
                .any(|(w, vert)| !moved.contains(&w) && vert.pos.distance(to) < LENGTH_TOL);
            if lands {
                return Err(KernelError::Squash);
            }
        }
        // Faces that move partly get a new plane; check it and compute their new normals.
        let mut retilted: Vec<(FaceId, Face)> = Vec::new();
        for (id, f) in self.faces() {
            let all: Vec<VertexId> = loops(f).flatten().copied().collect();
            let touched = all.iter().filter(|v| moved.contains(v)).count();
            if touched == 0 || touched == all.len() {
                continue;
            }
            let outer: Vec<DVec3> = f.outer.iter().map(|v| at(self, *v)).collect();
            let area = vector_area(&outer);
            if area.dot(f.normal) <= LENGTH_TOL * LENGTH_TOL {
                return Err(KernelError::Squash);
            }
            let normal = area.normalize();
            let origin = outer[0];
            if all
                .iter()
                .any(|v| (at(self, *v) - origin).dot(normal).abs() > LENGTH_TOL)
            {
                return Err(KernelError::Bend(id));
            }
            for h in &f.holes {
                let pts: Vec<DVec3> = h.iter().map(|v| at(self, *v)).collect();
                if vector_area(&pts).dot(normal) >= 0.0 {
                    return Err(KernelError::Squash);
                }
            }
            if normal.distance(f.normal) > 1e-12 {
                let mut g = f.clone();
                g.normal = normal;
                retilted.push((id, g));
            }
        }
        for &v in &moved {
            let to = self.pos(v) + offset;
            self.move_vertex(v, to);
        }
        for (id, f) in retilted {
            self.replace_face(id, f);
        }
        Ok(self.take_changes())
    }

    /// Copies `edges` and `faces` (with their edges) `count` times, copy k shifted by
    /// `k * offset`, as one ChangeSet. Copies are drawn like new edges, so they merge with
    /// geometry they touch and close loops into faces; each copied face keeps the orientation
    /// of its original.
    pub fn copy(
        &mut self,
        edges: &[EdgeId],
        faces: &[FaceId],
        offset: DVec3,
        count: u32,
    ) -> Result<ChangeSet, KernelError> {
        if !offset.is_finite() {
            return Err(KernelError::NotFinite);
        }
        if offset.length() < LENGTH_TOL {
            return Err(KernelError::ZeroDistance);
        }
        let mut segments: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        for e in edges.iter().filter_map(|e| self.edge(*e)) {
            segments.insert((e.v[0].min(e.v[1]), e.v[0].max(e.v[1])));
        }
        let mut originals: Vec<(Vec<DVec3>, DVec3)> = Vec::new();
        for f in faces.iter().filter_map(|f| self.face(*f)) {
            for l in loops(f) {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    segments.insert((a.min(b), a.max(b)));
                }
            }
            originals.push((f.outer.iter().map(|v| self.pos(*v)).collect(), f.normal));
        }
        let points: Vec<(DVec3, DVec3)> = segments
            .iter()
            .map(|(a, b)| (self.pos(*a), self.pos(*b)))
            .collect();
        for k in 1..=count {
            let shift = offset * k as f64;
            for (a, b) in &points {
                self.draw_segment(*a + shift, *b + shift);
            }
        }
        for k in 1..=count {
            let shift = offset * k as f64;
            for (outer, normal) in &originals {
                let ids: Option<BTreeSet<VertexId>> =
                    outer.iter().map(|p| self.find_vertex(*p + shift)).collect();
                let Some(ids) = ids else { continue };
                let copy = self
                    .faces()
                    .find(|(_, f)| f.outer.iter().copied().collect::<BTreeSet<_>>() == ids)
                    .map(|(id, _)| id);
                if let Some(id) = copy {
                    self.orient(id, *normal);
                }
            }
        }
        Ok(self.take_changes())
    }
}

fn loops(f: &Face) -> impl Iterator<Item = &Vec<VertexId>> {
    std::iter::once(&f.outer).chain(&f.holes)
}
