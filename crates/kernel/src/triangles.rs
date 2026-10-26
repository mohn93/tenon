//! Every face of a scene as triangles in root coordinates, for export.

use std::collections::{BTreeSet, HashMap};

use glam::{DAffine3, DVec2, DVec3};

use crate::ids::{DefinitionId, EntityId, FaceId, MaterialId};
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

/// An object of a printable model: triangles in the object's own coordinates and the other
/// objects it places.
#[derive(Clone, Debug, Default)]
pub struct PartObject {
    /// The definition the object is made from; None for the root's loose faces.
    pub definition: Option<DefinitionId>,
    pub triangles: Vec<Triangle>,
    /// Indices into `SharedParts::objects` (always lower than this object's own) and where
    /// each is placed. Never a mirroring transform.
    pub refs: Vec<(usize, DAffine3)>,
}

/// The shown model as objects and build items, for formats that keep parts apart and can
/// reference an object more than once (3MF).
#[derive(Clone, Debug, Default)]
pub struct SharedParts {
    pub objects: Vec<PartObject>,
    /// Which object is built, and with what transform.
    pub items: Vec<(usize, DAffine3)>,
    /// How many faces gave no triangles (a shared object's once).
    pub skipped: usize,
}

/// What two placements must have alike to share an object: the definition and the paint they
/// inherit.
type ShareKey = (DefinitionId, Option<MaterialId>);

struct PartBuilder<'a> {
    scene: &'a Scene,
    counts: HashMap<ShareKey, usize>,
    built: HashMap<ShareKey, Option<usize>>,
    out: SharedParts,
}

fn mirrors(place: &DAffine3) -> bool {
    place.matrix3.determinant() < 0.0
}

impl PartBuilder<'_> {
    /// Counts the placements of each key below `model`. A key's own contents are counted on
    /// its first placement only, as they are built once. Mirrored placements are baked, not
    /// counted.
    fn count(&mut self, model: &Model, place: DAffine3, inherited: Option<MaterialId>) {
        for (_, instance) in model
            .instances()
            .filter(|(_, i)| self.scene.placed_instance_shown(i))
        {
            let composed = place * instance.transform;
            if mirrors(&composed) {
                continue;
            }
            let material = instance.material.or(inherited);
            let n = self
                .counts
                .entry((instance.definition, material))
                .or_default();
            *n += 1;
            if *n == 1
                && let Some(inner) = self.scene.mesh(MeshId::Definition(instance.definition))
            {
                self.count(inner, composed, material);
            }
        }
    }

    /// The object holding `key`'s definition in its own coordinates, built once; None when it
    /// holds nothing.
    fn shared(&mut self, key: ShareKey) -> Option<usize> {
        if let Some(i) = self.built.get(&key) {
            return *i;
        }
        let mut object = PartObject {
            definition: Some(key.0),
            ..PartObject::default()
        };
        if let Some(model) = self.scene.mesh(MeshId::Definition(key.0)) {
            self.walk(model, DAffine3::IDENTITY, key.1, &mut object);
        }
        let index = self.push(object);
        self.built.insert(key, index);
        index
    }

    fn push(&mut self, object: PartObject) -> Option<usize> {
        if object.triangles.is_empty() && object.refs.is_empty() {
            return None;
        }
        self.out.objects.push(object);
        Some(self.out.objects.len() - 1)
    }

    /// Adds the faces of `model` and of everything it places to `object`, placed by `place`:
    /// baked in, except shared definitions, which are referenced.
    fn walk(
        &mut self,
        model: &Model,
        place: DAffine3,
        inherited: Option<MaterialId>,
        object: &mut PartObject,
    ) {
        let scene = self.scene;
        scene.collect_faces(
            model,
            place,
            inherited,
            &|_| true,
            &mut object.triangles,
            &mut self.out.skipped,
        );
        for (_, instance) in model
            .instances()
            .filter(|(_, i)| scene.placed_instance_shown(i))
        {
            let composed = place * instance.transform;
            let material = instance.material.or(inherited);
            let key = (instance.definition, material);
            if mirrors(&composed) {
                scene.collect_triangles(
                    MeshId::Definition(instance.definition),
                    composed,
                    material,
                    &mut object.triangles,
                    &mut self.out.skipped,
                );
            } else if self.counts.get(&key).copied().unwrap_or(0) >= 2 {
                if let Some(i) = self.shared(key) {
                    object.refs.push((i, composed));
                }
            } else if let Some(inner) = scene.mesh(MeshId::Definition(instance.definition)) {
                self.walk(inner, composed, material, object);
            }
        }
    }
}

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

    /// The shown model as objects and build items: the root's loose faces first (one object
    /// in root coordinates), then each shown root instance. A definition placed more than once
    /// with the same inherited paint is one object in its own coordinates, built by each
    /// placement's transform (and referenced by the objects placing it deeper); anything
    /// else, and every placement that mirrors (a mirroring transform turns a mesh inside out
    /// in consumers), is baked in at its world transform with the winding restored. Parts
    /// without triangles are left out. With `only`, just the instances and faces it holds.
    pub fn shared_parts(&self, only: Option<&BTreeSet<EntityId>>) -> SharedParts {
        let root = self.root();
        let mut b = PartBuilder {
            scene: self,
            counts: HashMap::new(),
            built: HashMap::new(),
            out: SharedParts::default(),
        };
        let wanted = |iid| only.is_none_or(|s| s.contains(&EntityId::Instance(iid)));
        let shown = |i: &crate::model::Instance| self.placed_instance_shown(i);
        for (_, instance) in root.instances().filter(|(i, n)| wanted(*i) && shown(n)) {
            if mirrors(&instance.transform) {
                continue;
            }
            let key = (instance.definition, instance.material);
            let n = b.counts.entry(key).or_default();
            *n += 1;
            if *n == 1
                && let Some(inner) = self.mesh(MeshId::Definition(instance.definition))
            {
                b.count(inner, instance.transform, instance.material);
            }
        }
        let mut loose = PartObject::default();
        let keep = |f: FaceId| only.is_none_or(|s| s.contains(&EntityId::Face(f)));
        self.collect_faces(
            root,
            DAffine3::IDENTITY,
            None,
            &keep,
            &mut loose.triangles,
            &mut b.out.skipped,
        );
        if let Some(i) = b.push(loose) {
            b.out.items.push((i, DAffine3::IDENTITY));
        }
        for (_, instance) in root.instances().filter(|(i, n)| wanted(*i) && shown(n)) {
            let key = (instance.definition, instance.material);
            let item = if mirrors(&instance.transform) {
                let mut object = PartObject {
                    definition: Some(instance.definition),
                    ..PartObject::default()
                };
                self.collect_triangles(
                    MeshId::Definition(instance.definition),
                    instance.transform,
                    instance.material,
                    &mut object.triangles,
                    &mut b.out.skipped,
                );
                b.push(object).map(|i| (i, DAffine3::IDENTITY))
            } else if b.counts.get(&key).copied().unwrap_or(0) >= 2 {
                b.shared(key).map(|i| (i, instance.transform))
            } else {
                let mut object = PartObject {
                    definition: Some(instance.definition),
                    ..PartObject::default()
                };
                if let Some(inner) = self.mesh(MeshId::Definition(instance.definition)) {
                    b.walk(inner, instance.transform, instance.material, &mut object);
                }
                b.push(object).map(|i| (i, DAffine3::IDENTITY))
            };
            b.out.items.extend(item);
        }
        b.out
    }
}
