//! Triangulation of faces for rendering and picking.

use glam::DVec3;

use crate::geom::Plane;
use crate::ids::FaceId;
use crate::model::Model;

impl Model {
    /// Triangles covering a face (outer loop minus holes), each wound counter-clockwise around
    /// the face normal. None if the face does not exist or cannot be triangulated.
    pub fn triangulate(&self, id: FaceId) -> Option<Vec<[DVec3; 3]>> {
        let face = self.face(id)?;
        let origin = self.position(*face.outer.first()?)?;
        let plane = Plane {
            normal: face.normal,
            d: face.normal.dot(origin),
        };

        let (u, v) = plane.basis();
        let n = face.outer.len() + face.holes.iter().map(Vec::len).sum::<usize>();
        let mut points = Vec::with_capacity(n);
        let mut coords = Vec::with_capacity(2 * n);
        let mut hole_starts = Vec::with_capacity(face.holes.len());
        for (k, lp) in std::iter::once(&face.outer).chain(&face.holes).enumerate() {
            if k > 0 {
                hole_starts.push(points.len());
            }
            for id in lp {
                let p = self.position(*id)?;
                points.push(p);
                // As `plane.project(p)`, with the basis worked out once.
                coords.extend([p.dot(u), p.dot(v)]);
            }
        }

        let indices = earcutr::earcut(&coords, &hole_starts, 2).ok()?;
        let tris = indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| {
                let t = [points[c[0]], points[c[1]], points[c[2]]];
                if (t[1] - t[0]).cross(t[2] - t[0]).dot(face.normal) < 0.0 {
                    [t[0], t[2], t[1]]
                } else {
                    t
                }
            })
            .collect();
        Some(tris)
    }
}
