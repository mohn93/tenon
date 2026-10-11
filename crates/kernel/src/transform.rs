//! Moving and copying geometry.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec3};

use crate::edit::KernelError;
use crate::geom::{LENGTH_TOL, Plane, point_in_polygon, segment_distance_2d, vector_area};
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
            let mut rings = vec![outer];
            for h in &f.holes {
                let pts: Vec<DVec3> = h.iter().map(|v| at(self, *v)).collect();
                if vector_area(&pts).dot(normal) >= 0.0 {
                    return Err(KernelError::Squash);
                }
                rings.push(pts);
            }
            if !rings_are_valid(&rings, normal) {
                return Err(KernelError::Squash);
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
    /// of its original, and an opening (a hole with no face in it) stays open.
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
        let stencil = self.stencil(edges, faces);
        let places: Vec<DAffine3> = (1..=count)
            .map(|k| DAffine3::from_translation(offset * k as f64))
            .collect();
        self.draw_stencil(&stencil, &places);
        Ok(self.take_changes())
    }

    /// Captures `edges` and `faces` (with their edges) by position.
    pub(crate) fn stencil(&self, edges: &[EdgeId], faces: &[FaceId]) -> Stencil {
        let mut sides: BTreeSet<(VertexId, VertexId)> = BTreeSet::new();
        for e in edges.iter().filter_map(|e| self.edge(*e)) {
            sides.insert((e.v[0].min(e.v[1]), e.v[0].max(e.v[1])));
        }
        let mut s = Stencil::default();
        for f in faces.iter().filter_map(|f| self.face(*f)) {
            for l in loops(f) {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    sides.insert((a.min(b), a.max(b)));
                }
            }
            s.faces
                .push((f.outer.iter().map(|v| self.pos(*v)).collect(), f.normal));
            for h in &f.holes {
                if self.face_with_outer(&h.iter().copied().collect()).is_none() {
                    s.openings.push(h.iter().map(|v| self.pos(*v)).collect());
                }
            }
        }
        s.segments = sides
            .iter()
            .map(|(a, b)| (self.pos(*a), self.pos(*b)))
            .collect();
        s
    }

    /// Draws `stencil` once per placement, without ending the edit. Copies are drawn like new
    /// edges, so they merge with geometry they touch and close loops into faces; each face
    /// keeps the orientation of its original, and openings stay open.
    pub(crate) fn draw_stencil(&mut self, stencil: &Stencil, places: &[DAffine3]) {
        let existing: BTreeSet<FaceId> = self.faces().map(|(id, _)| id).collect();
        for t in places {
            for (a, b) in &stencil.segments {
                self.draw_segment(t.transform_point3(*a), t.transform_point3(*b));
            }
        }
        for t in places {
            for (outer, normal) in &stencil.faces {
                let ids: Option<BTreeSet<VertexId>> = outer
                    .iter()
                    .map(|p| self.find_vertex(t.transform_point3(*p)))
                    .collect();
                let Some(ids) = ids else { continue };
                if let Some(id) = self.face_with_outer(&ids) {
                    self.orient(id, t.transform_vector3(*normal).normalize());
                }
            }
            // Drawing an opening's loop closes it into a face; take that face out again.
            for hole in &stencil.openings {
                let ids: Option<BTreeSet<VertexId>> = hole
                    .iter()
                    .map(|p| self.find_vertex(t.transform_point3(*p)))
                    .collect();
                let Some(ids) = ids else { continue };
                if let Some(id) = self.face_with_outer(&ids)
                    && !existing.contains(&id)
                {
                    self.remove_face(id);
                }
            }
        }
    }

    /// The face whose outer loop is exactly these vertices.
    fn face_with_outer(&self, ids: &BTreeSet<VertexId>) -> Option<FaceId> {
        self.faces()
            .find(|(_, f)| f.outer.len() == ids.len() && f.outer.iter().all(|v| ids.contains(v)))
            .map(|(id, _)| id)
    }
}

/// Edges and faces captured by position, to be drawn again elsewhere (copies, groups).
#[derive(Clone, Debug, Default)]
pub(crate) struct Stencil {
    segments: Vec<(DVec3, DVec3)>,
    /// Outer loop and normal of each face.
    faces: Vec<(Vec<DVec3>, DVec3)>,
    /// Holes of the faces that no face fills.
    openings: Vec<Vec<DVec3>>,
}

fn loops(f: &Face) -> impl Iterator<Item = &Vec<VertexId>> {
    std::iter::once(&f.outer).chain(&f.holes)
}

/// True when no two non-adjacent edges of the rings touch or cross (in the plane with this
/// normal) and every hole (ring 1..) lies inside the outer ring (ring 0).
fn rings_are_valid(rings: &[Vec<DVec3>], normal: DVec3) -> bool {
    let plane = Plane::through(rings[0][0], normal);
    let flat: Vec<Vec<glam::DVec2>> = rings
        .iter()
        .map(|r| r.iter().map(|p| plane.project(*p)).collect())
        .collect();
    let mut edges = Vec::new();
    for (r, ring) in flat.iter().enumerate() {
        for i in 0..ring.len() {
            edges.push((r, i, ring[i], ring[(i + 1) % ring.len()]));
        }
    }
    for (x, e) in edges.iter().enumerate() {
        for g in &edges[x + 1..] {
            if e.0 == g.0 {
                let n = flat[e.0].len();
                if (e.1 + 1) % n == g.1 || (g.1 + 1) % n == e.1 {
                    continue; // adjacent edges share a vertex
                }
            }
            let tol = LENGTH_TOL;
            let near = segment_distance_2d(e.2, g.2, g.3) < tol
                || segment_distance_2d(e.3, g.2, g.3) < tol
                || segment_distance_2d(g.2, e.2, e.3) < tol
                || segment_distance_2d(g.3, e.2, e.3) < tol;
            if near || segments_cross(e.2, e.3, g.2, g.3) {
                return false;
            }
        }
    }
    flat[1..].iter().all(|h| point_in_polygon(h[0], &flat[0]))
}

fn segments_cross(p: glam::DVec2, q: glam::DVec2, a: glam::DVec2, b: glam::DVec2) -> bool {
    let side = |o: glam::DVec2, u: glam::DVec2, w: glam::DVec2| (u - o).perp_dot(w - o);
    side(p, q, a) * side(p, q, b) < 0.0 && side(a, b, p) * side(a, b, q) < 0.0
}
