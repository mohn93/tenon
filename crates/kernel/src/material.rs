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

/// A named color, possibly see-through.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// sRGB red, green and blue.
    pub color: [u8; 3],
    /// How much of what is behind it the material hides: 1 is opaque, 0 invisible.
    pub opacity: f32,
}

impl Material {
    /// True when the material lets anything behind it show through.
    pub fn is_translucent(&self) -> bool {
        self.opacity < 1.0
    }
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

/// Fails, saying why, when an opacity is not a finite number from 0 to 1.
pub(crate) fn check_opacity(opacity: f32) -> Result<(), String> {
    if opacity.is_finite() && (0.0..=1.0).contains(&opacity) {
        Ok(())
    } else {
        Err(format!(
            "opacity must be a number from 0 to 1, not {opacity}"
        ))
    }
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
    /// Adds an opaque material. The name must be valid (see [`MAX_MATERIAL_NAME`]) and not
    /// taken.
    pub fn add_material(
        &mut self,
        name: &str,
        color: [u8; 3],
    ) -> Result<(MaterialId, SceneChange), KernelError> {
        self.add_material_with_opacity(name, color, 1.0)
    }

    /// Adds a material of `opacity` (finite, 0 to 1). The name must be valid (see
    /// [`MAX_MATERIAL_NAME`]) and not taken.
    pub fn add_material_with_opacity(
        &mut self,
        name: &str,
        color: [u8; 3],
        opacity: f32,
    ) -> Result<(MaterialId, SceneChange), KernelError> {
        self.check_free_name(name, None)?;
        check_opacity(opacity).map_err(KernelError::MaterialOpacity)?;
        let id = self.alloc_material();
        let step = Step::AddMaterial(
            id,
            Material {
                name: name.to_string(),
                color,
                opacity,
            },
        );
        self.apply_step(&step);
        Ok((id, step.into()))
    }

    /// Renames, recolors and/or changes the opacity of a material (None keeps that part).
    /// Nothing to change changes nothing.
    pub fn change_material(
        &mut self,
        id: MaterialId,
        name: Option<&str>,
        color: Option<[u8; 3]>,
        opacity: Option<f32>,
    ) -> Result<SceneChange, KernelError> {
        let from = self
            .material(id)
            .ok_or(KernelError::MaterialNotFound)?
            .clone();
        if let Some(name) = name {
            self.check_free_name(name, Some(id))?;
        }
        if let Some(opacity) = opacity {
            check_opacity(opacity).map_err(KernelError::MaterialOpacity)?;
        }
        let to = Material {
            name: name.map_or_else(|| from.name.clone(), str::to_string),
            color: color.unwrap_or(from.color),
            opacity: opacity.unwrap_or(from.opacity),
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
