//! Every face of a scene as triangles in root coordinates, for export.

use glam::{DAffine3, DVec3};

use crate::scene::{MeshId, Scene};

/// One triangle of a face, wound counter-clockwise around `normal` (the face's front).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub corners: [DVec3; 3],
    pub normal: DVec3,
}

impl Scene {
    /// Every face the scene shows, as triangles in root coordinates: the root's faces and,
    /// placed by their transforms, the faces of every instance at any depth. A component
    /// placed twice gives its faces twice. Edges without faces give nothing. The order is
    /// fixed: faces by id, then instances by id, depth first.
    pub fn triangles(&self) -> Vec<Triangle> {
        let mut out = Vec::new();
        self.collect_triangles(MeshId::Root, DAffine3::IDENTITY, &mut out);
        out
    }

    fn collect_triangles(&self, id: MeshId, place: DAffine3, out: &mut Vec<Triangle>) {
        let Some(model) = self.mesh(id) else { return };
        for (fid, face) in model.faces() {
            let normal = place.transform_vector3(face.normal).normalize();
            for tri in model.triangulate(fid).unwrap_or_default() {
                out.push(Triangle {
                    corners: tri.map(|p| place.transform_point3(p)),
                    normal,
                });
            }
        }
        for (_, instance) in model.instances() {
            self.collect_triangles(
                MeshId::Definition(instance.definition),
                place * instance.transform,
                out,
            );
        }
    }
}
