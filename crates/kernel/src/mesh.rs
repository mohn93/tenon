//! Triangulation of faces for rendering and picking.

use glam::DVec3;

use crate::geom::Plane;
use crate::ids::{FaceId, VertexId};
use crate::model::Model;

impl Model {
    /// Triangles covering a face (outer loop minus holes), each wound counter-clockwise around
    /// the face normal. None if the face does not exist or cannot be triangulated.
    pub fn triangulate(&self, id: FaceId) -> Option<Vec<[DVec3; 3]>> {
        let tris = self.triangle_vertices(id)?;
        Some(tris.iter().map(|t| t.map(|v| self.pos(v))).collect())
    }

    /// `triangulate`, giving each corner as the face's vertex there.
    pub fn triangle_vertices(&self, id: FaceId) -> Option<Vec<[VertexId; 3]>> {
        let face = self.face(id)?;
        let origin = self.position(*face.outer.first()?)?;
        if !face.normal.is_finite() || face.normal.length_squared() < 0.5 {
            return None;
        }
        let plane = Plane {
            normal: face.normal,
            d: face.normal.dot(origin),
        };

        let (u, v) = plane.basis();
        let n = face.outer.len() + face.holes.iter().map(Vec::len).sum::<usize>();
        let mut ids = Vec::with_capacity(n);
        let mut points = Vec::with_capacity(n);
        let mut coords = Vec::with_capacity(2 * n);
        let mut hole_starts = Vec::with_capacity(face.holes.len());
        for (k, lp) in std::iter::once(&face.outer).chain(&face.holes).enumerate() {
            if k > 0 {
                hole_starts.push(points.len());
            }
            for id in lp {
                let p = self.position(*id)?;
                ids.push(*id);
                points.push(p);
                // As `plane.project(p)`, with the basis worked out once.
                // Relative to the face's first vertex: far from the origin the absolute
                // coordinates of a small face lose their digits and collapse into collinear
                // points, where the triangulator panics.
                let q = p - origin;
                coords.extend([q.dot(u), q.dot(v)]);
            }
        }

        if coords.iter().any(|c| !c.is_finite()) {
            return None;
        }
        let indices = earcutr::earcut(&coords, &hole_starts, 2).ok()?;
        let tris = indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| {
                let t = [points[c[0]], points[c[1]], points[c[2]]];
                if (t[1] - t[0]).cross(t[2] - t[0]).dot(face.normal) < 0.0 {
                    [ids[c[0]], ids[c[2]], ids[c[1]]]
                } else {
                    [ids[c[0]], ids[c[1]], ids[c[2]]]
                }
            })
            .collect();
        Some(tris)
    }
}
