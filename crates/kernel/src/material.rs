//! Materials: named colors shared by every mesh of a scene, painted on either side of a face or
//! on an instance (whose faces without a material of their own then show it).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{FaceId, InstanceId, MaterialId};
use crate::model::{ChangeSet, Model};
use crate::scene::{MeshId, Scene, SceneChange, Step};

/// The longest material name, in characters.
pub const MAX_MATERIAL_NAME: usize = 256;

/// A named color.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// sRGB red, green and blue.
    pub color: [u8; 3],
}

/// Which side of a face to paint.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The side the face's normal points to.
    #[default]
    Front,
    Back,
    Both,
}

/// Fails, saying why, when a material name is blank or longer than [`MAX_MATERIAL_NAME`]
/// characters.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a material needs a name".into());
    }
    if name.chars().count() > MAX_MATERIAL_NAME {
        return Err(format!(
            "a material name may have at most {MAX_MATERIAL_NAME} characters"
        ));
    }
    Ok(())
}

impl Model {
    /// Paints `side` of each face with `material` (None: the default material) as one
    /// ChangeSet. Faces already painted so, and unknown ids, are skipped.
    pub fn paint_faces(
        &mut self,
        faces: &[FaceId],
        side: Side,
        material: Option<MaterialId>,
    ) -> ChangeSet {
        for &id in faces.iter().collect::<BTreeSet<_>>() {
            let Some(f) = self.face(id) else { continue };
            let [mut front, mut back] = f.paint();
            if side != Side::Back {
                front = material;
            }
            if side != Side::Front {
                back = material;
            }
            self.set_paint(id, [front, back]);
        }
        self.take_changes()
    }

    /// Paints instances with `material` (None: none) as one ChangeSet. Unknown ids are skipped.
    pub fn paint_instances(
        &mut self,
        instances: &[InstanceId],
        material: Option<MaterialId>,
    ) -> ChangeSet {
        for &id in instances.iter().collect::<BTreeSet<_>>() {
            if self.instance(id).is_some() {
                self.set_instance_material(id, material);
            }
        }
        self.take_changes()
    }

    /// Takes `material` off every face side and instance painted with it, as one ChangeSet.
    pub(crate) fn unpaint(&mut self, material: MaterialId) -> ChangeSet {
        let off = |m: Option<MaterialId>| m.filter(|m| *m != material);
        let faces: Vec<_> = self
            .faces()
            .filter(|(_, f)| f.paint().contains(&Some(material)))
            .map(|(id, f)| (id, f.paint()))
            .collect();
        for (id, [front, back]) in faces {
            self.set_paint(id, [off(front), off(back)]);
        }
        let instances: Vec<_> = self
            .instances()
            .filter(|(_, i)| i.material == Some(material))
            .map(|(id, _)| id)
            .collect();
        for id in instances {
            self.set_instance_material(id, None);
        }
        self.take_changes()
    }
}

impl Scene {
    /// Adds a material. The name must be valid (see [`MAX_MATERIAL_NAME`]) and not taken.
    pub fn add_material(
        &mut self,
        name: &str,
        color: [u8; 3],
    ) -> Result<(MaterialId, SceneChange), KernelError> {
        self.check_free_name(name, None)?;
        let id = self.alloc_material();
        let step = Step::AddMaterial(
            id,
            Material {
                name: name.to_string(),
                color,
            },
        );
        self.apply_step(&step);
        Ok((id, step.into()))
    }

    /// Renames and/or recolors a material (None keeps that part). Nothing to change changes
    /// nothing.
    pub fn change_material(
        &mut self,
        id: MaterialId,
        name: Option<&str>,
        color: Option<[u8; 3]>,
    ) -> Result<SceneChange, KernelError> {
        let from = self
            .material(id)
            .ok_or(KernelError::MaterialNotFound)?
            .clone();
        if let Some(name) = name {
            self.check_free_name(name, Some(id))?;
        }
        let to = Material {
            name: name.map_or_else(|| from.name.clone(), str::to_string),
            color: color.unwrap_or(from.color),
        };
        if to == from {
            return Ok(SceneChange::default());
        }
        let step = Step::ChangeMaterial(id, from, to);
        self.apply_step(&step);
        Ok(step.into())
    }

    /// Removes a material. Faces and instances painted with it go back to the default material
    /// first, so undo restores both.
    pub fn delete_material(&mut self, id: MaterialId) -> Result<SceneChange, KernelError> {
        let material = self
            .material(id)
            .ok_or(KernelError::MaterialNotFound)?
            .clone();
        let meshes: Vec<MeshId> = self.meshes().map(|(m, _)| m).collect();
        let mut changes = Vec::new();
        for mesh in meshes {
            changes.push(self.edit(mesh, |m| Ok(m.unpaint(id)))?);
        }
        let step = Step::RemoveMaterial(id, material);
        self.apply_step(&step);
        changes.push(step.into());
        Ok(SceneChange::join(changes))
    }

    /// Paints `side` of faces, and instances, of mesh `mesh` with `material` (None: the default
    /// material). Instances take the material whatever `side` says. Unknown ids are skipped.
    pub fn paint(
        &mut self,
        mesh: MeshId,
        faces: &[FaceId],
        instances: &[InstanceId],
        side: Side,
        material: Option<MaterialId>,
    ) -> Result<SceneChange, KernelError> {
        if material.is_some_and(|m| self.material(m).is_none()) {
            return Err(KernelError::MaterialNotFound);
        }
        self.edit(mesh, |m| {
            let mut c = m.paint_faces(faces, side, material);
            c.ops.extend(m.paint_instances(instances, material).ops);
            Ok(c)
        })
    }

    /// `base` if no material has that name, else `base` followed by the first free number
    /// from 2 ("Material 2").
    pub fn free_material_name(&self, base: &str) -> String {
        let taken: BTreeSet<&str> = self.materials().map(|(_, m)| m.name.as_str()).collect();
        if !taken.contains(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|n| !taken.contains(n.as_str()))
            .expect("a free name")
    }

    fn check_free_name(&self, name: &str, except: Option<MaterialId>) -> Result<(), KernelError> {
        check_name(name).map_err(KernelError::MaterialName)?;
        let taken = self
            .materials()
            .any(|(id, m)| Some(id) != except && m.name == name);
        if taken {
            return Err(KernelError::MaterialName(format!(
                "a material named {name:?} already exists"
            )));
        }
        Ok(())
    }
}
