//! Every face of a scene as triangles in root coordinates, for export.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec2, DVec3};

use crate::ids::{EntityId, FaceId, InstanceId, MaterialId};
use crate::model::Model;
use crate::scene::{MeshId, Scene};

/// One triangle of a face, wound counter-clockwise around `normal` (the face's front).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub corners: [DVec3; 3],
    pub normal: DVec3,
    /// The normal at each corner: the face's own, or across soft edges the smooth one (see
    /// `Model::corner_normal`).
    pub normals: [DVec3; 3],
    /// Where each corner lies on the front's texture, in meters (see `Face::texture_position`:
    /// the projection with the face's placement on top): in the plane of the face as its
    /// definition holds it, so a placed copy carries its pattern.
    pub uvs: [DVec2; 3],
    /// The material shown on the front and on the back: the face's own, or else that of the
    /// nearest painted instance placing it; None for the default material.
    pub front: Option<MaterialId>,
    pub back: Option<MaterialId>,
}

/// A root part's triangles: the root instance (None for the root's loose faces) and what it
/// shows.
pub type Part = (Option<InstanceId>, Vec<Triangle>);

impl Scene {
    /// Every face the scene shows, as triangles in root coordinates: the root's faces and,
    /// placed by their transforms, the faces of every instance at any depth. Hidden faces and
    /// instances, and those on hidden tags, are left out (see `Scene::face_shown`), with all they
    /// place. A component placed twice gives its faces twice. Edges without faces give nothing.
    /// The order is fixed: faces by id, then instances by id, depth first.
    pub fn triangles(&self) -> Vec<Triangle> {
        self.triangles_and_skipped().0
    }

    /// `triangles`, and how many faces gave none (a face that cannot be triangulated, such as
    /// one whose hole covers it; counted once per placement).
    pub fn triangles_and_skipped(&self) -> (Vec<Triangle>, usize) {
        let mut out = Vec::new();
        let mut skipped = 0;
        self.collect_triangles(
            MeshId::Root,
            DAffine3::IDENTITY,
            None,
            &mut out,
            &mut skipped,
        );
        (out, skipped)
    }

    /// `inherited` is the material of the nearest painted instance placing the mesh.
    fn collect_triangles(
        &self,
        id: MeshId,
        place: DAffine3,
        inherited: Option<MaterialId>,
        out: &mut Vec<Triangle>,
        skipped: &mut usize,
    ) {
        let Some(model) = self.mesh(id) else { return };
        self.collect_faces(model, place, inherited, &|_| true, out, skipped);
        for (_, instance) in model
            .instances()
            .filter(|(_, i)| self.placed_instance_shown(i))
        {
            self.collect_triangles(
                MeshId::Definition(instance.definition),
                place * instance.transform,
                instance.material.or(inherited),
                out,
                skipped,
            );
        }
    }

    /// The shown faces of `model` that `keep` accepts, as triangles placed by `place`.
    fn collect_faces(
        &self,
        model: &Model,
        place: DAffine3,
        inherited: Option<MaterialId>,
        keep: &dyn Fn(FaceId) -> bool,
        out: &mut Vec<Triangle>,
        skipped: &mut usize,
    ) {
        // A mirroring placement turns triangles inside out; swapping two corners restores the
        // winding. Normals follow the inverse transpose, which is right for any linear map.
        let mirrored = place.matrix3.determinant() < 0.0;
        let normal_map = place.matrix3.inverse().transpose();
        let smooth = model.smooth_normals();
        for (fid, face) in model
            .faces()
            .filter(|(fid, f)| self.face_shown(f) && keep(*fid))
        {
            let normal = (normal_map * face.normal).normalize();
            let (front, back) = (face.front.or(inherited), face.back.or(inherited));
            let tris = model.shaded_triangles(fid, &smooth).unwrap_or_default();
            if tris.is_empty() {
                *skipped += 1;
            }
            for tri in tris {
                let [a, b, c] = tri.corners.map(|p| place.transform_point3(p));
                let [na, nb, nc] = tri.normals.map(|n| (normal_map * n).normalize());
                let [ua, ub, uc] = tri.corners.map(|p| face.texture_position(p));
                out.push(Triangle {
                    corners: if mirrored { [a, c, b] } else { [a, b, c] },
                    normal,
                    normals: if mirrored { [na, nc, nb] } else { [na, nb, nc] },
                    uvs: if mirrored { [ua, uc, ub] } else { [ua, ub, uc] },
                    front,
                    back,
                });
            }
        }
    }

    /// The shown triangles grouped by root part, for formats that keep parts apart (3MF): the
    /// root's loose faces first (None), then each shown root instance with everything it
    /// places, all in root coordinates. Parts without triangles are left out. With `only`,
    /// just the instances and faces it holds. Also how many faces gave no triangles.
    pub fn triangles_by_part(&self, only: Option<&BTreeSet<EntityId>>) -> (Vec<Part>, usize) {
        let mut parts = Vec::new();
        let mut skipped = 0;
        let root = self.root();
        let mut loose = Vec::new();
        let keep = |f: FaceId| only.is_none_or(|s| s.contains(&EntityId::Face(f)));
        self.collect_faces(
            root,
            DAffine3::IDENTITY,
            None,
            &keep,
            &mut loose,
            &mut skipped,
        );
        if !loose.is_empty() {
            parts.push((None, loose));
        }
        for (iid, instance) in root.instances() {
            if !self.placed_instance_shown(instance)
                || only.is_some_and(|s| !s.contains(&EntityId::Instance(iid)))
            {
                continue;
            }
            let mut tris = Vec::new();
            self.collect_triangles(
                MeshId::Definition(instance.definition),
                instance.transform,
                instance.material,
                &mut tris,
                &mut skipped,
            );
            if !tris.is_empty() {
                parts.push((Some(iid), tris));
            }
        }
        (parts, skipped)
    }
}
