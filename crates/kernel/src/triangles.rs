//! Every face of a scene as triangles in root coordinates, for export.

use glam::{DAffine3, DVec3};

use crate::ids::MaterialId;
use crate::scene::{MeshId, Scene};

/// One triangle of a face, wound counter-clockwise around `normal` (the face's front).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub corners: [DVec3; 3],
    pub normal: DVec3,
    /// The material shown on the front and on the back: the face's own, or else that of the
    /// nearest painted instance placing it; None for the default material.
    pub front: Option<MaterialId>,
    pub back: Option<MaterialId>,
}

impl Scene {
    /// Every face the scene shows, as triangles in root coordinates: the root's faces and,
    /// placed by their transforms, the faces of every instance at any depth. A component
    /// placed twice gives its faces twice. Edges without faces give nothing. The order is
    /// fixed: faces by id, then instances by id, depth first.
    pub fn triangles(&self) -> Vec<Triangle> {
        let mut out = Vec::new();
        self.collect_triangles(MeshId::Root, DAffine3::IDENTITY, None, &mut out);
        out
    }

    /// `inherited` is the material of the nearest painted instance placing the mesh.
    fn collect_triangles(
        &self,
        id: MeshId,
        place: DAffine3,
        inherited: Option<MaterialId>,
        out: &mut Vec<Triangle>,
    ) {
        let Some(model) = self.mesh(id) else { return };
        // A mirroring placement turns triangles inside out; swapping two corners restores the
        // winding. Normals follow the inverse transpose, which is right for any linear map.
        let mirrored = place.matrix3.determinant() < 0.0;
        let normal_map = place.matrix3.inverse().transpose();
        for (fid, face) in model.faces() {
            let normal = (normal_map * face.normal).normalize();
            let (front, back) = (face.front.or(inherited), face.back.or(inherited));
            for tri in model.triangulate(fid).unwrap_or_default() {
                let [a, b, c] = tri.map(|p| place.transform_point3(p));
                out.push(Triangle {
                    corners: if mirrored { [a, c, b] } else { [a, b, c] },
                    normal,
                    front,
                    back,
                });
            }
        }
        for (_, instance) in model.instances() {
            self.collect_triangles(
                MeshId::Definition(instance.definition),
                place * instance.transform,
                instance.material.or(inherited),
                out,
            );
        }
    }
}
