//! Materials: named colors, possibly with a texture, possibly glowing, shared by every mesh of a
//! scene, painted on either side of a face or on an instance (whose faces without a material of
//! their own then show it).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{FaceId, InstanceId, MaterialId};
use crate::model::{ChangeSet, Model};
use crate::scene::{MeshId, Scene, SceneChange, Step};
use crate::texture::{MAX_TEXTURES, Texture, check_texture};

/// The longest material name, in characters.
pub const MAX_MATERIAL_NAME: usize = 256;

/// The strongest a material glows (see `Material::emission`).
pub const MAX_EMISSION: f32 = 10.0;

/// A named color, possibly see-through, possibly with a texture, possibly glowing.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// sRGB red, green and blue. With a texture, faces show the texture and this color stands
    /// for it (edges, swatches, formats without textures).
    pub color: [u8; 3],
    /// How much of what is behind it the material hides: 1 is opaque, 0 invisible.
    pub opacity: f32,
    pub texture: Option<Texture>,
    /// How strongly faces painted with it glow, from 0 (not at all) to `MAX_EMISSION`.
    pub emission: f32,
    /// How metallic the surface is, from 0 (not at all, the default) to 1: a metal tints its
    /// highlights and reflections with its color and has no diffuse shading.
    pub metalness: f32,
    /// How rough the surface is, from 0 (a mirror) to 1 (matte, the default: no highlight and
    /// no reflection).
    pub roughness: f32,
    /// The material's density in kg/m³ (`MIN_DENSITY` to `MAX_DENSITY`), used to weigh solids
    /// painted with it; None when unknown.
    pub density: Option<f64>,
}

/// The lowest density a material may have, in kg/m³.
pub const MIN_DENSITY: f64 = 1.0;
/// The highest density a material may have, in kg/m³.
pub const MAX_DENSITY: f64 = 30_000.0;

/// A material's default metalness.
pub const DEFAULT_METALNESS: f32 = 0.0;
/// A material's default roughness.
pub const DEFAULT_ROUGHNESS: f32 = 1.0;

impl Material {
    /// True when the material lets anything behind it show through.
    pub fn is_translucent(&self) -> bool {
        self.opacity < 1.0
    }

    /// True when faces painted with it glow.
    pub fn glows(&self) -> bool {
        self.emission > 0.0
    }
}

/// The parts of a material `Scene::change_material_with` sets; None keeps that part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialEdit<'a> {
    pub name: Option<&'a str>,
    pub color: Option<[u8; 3]>,
    pub opacity: Option<f32>,
    /// `Some(None)` removes the texture.
    pub texture: Option<Option<Texture>>,
    pub emission: Option<f32>,
    pub metalness: Option<f32>,
    pub roughness: Option<f32>,
    /// `Some(None)` clears the density.
    pub density: Option<Option<f64>>,
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

/// Fails, saying why, when an emission is not a finite number from 0 to [`MAX_EMISSION`].
pub(crate) fn check_emission(emission: f32) -> Result<(), String> {
    if emission.is_finite() && (0.0..=MAX_EMISSION).contains(&emission) {
        Ok(())
    } else {
        Err(format!(
            "emission must be a number from 0 to {MAX_EMISSION}, not {emission}"
        ))
    }
}

/// Fails, saying why, when a metalness or roughness (`what` names it) is not a finite number
/// from 0 to 1.
pub(crate) fn check_unit(what: &str, value: f32) -> Result<(), String> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(format!("{what} must be a number from 0 to 1, not {value}"))
    }
}

/// Fails, saying why, when a density is not a finite number from [`MIN_DENSITY`] to
/// [`MAX_DENSITY`] (kg/m³).
pub fn check_density(density: f64) -> Result<(), String> {
    if density.is_finite() && (MIN_DENSITY..=MAX_DENSITY).contains(&density) {
        Ok(())
    } else {
        Err(format!(
            "density must be a number from {MIN_DENSITY} to {MAX_DENSITY} kg/m³, not {density}"
        ))
    }
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
        self.add_material_with_texture(name, color, opacity, None)
    }

    /// Adds a material of `opacity` (finite, 0 to 1), with `texture` if given (see
    /// [`MAX_TEXTURES`] and [`crate::MIN_TEXTURE_SIZE`]). The name must be valid (see
    /// [`MAX_MATERIAL_NAME`]) and not taken.
    pub fn add_material_with_texture(
        &mut self,
        name: &str,
        color: [u8; 3],
        opacity: f32,
        texture: Option<Texture>,
    ) -> Result<(MaterialId, SceneChange), KernelError> {
        self.add_material_from(Material {
            name: name.to_string(),
            color,
            opacity,
            texture,
            emission: 0.0,
            metalness: DEFAULT_METALNESS,
            roughness: DEFAULT_ROUGHNESS,
            density: None,
        })
    }

    /// Checks the ranges of the opacity, emission, metalness and roughness given, as
    /// `add_material_from` does.
    pub fn check_material_ranges(
        opacity: Option<f32>,
        emission: Option<f32>,
        metalness: Option<f32>,
        roughness: Option<f32>,
    ) -> Result<(), KernelError> {
        if let Some(o) = opacity {
            check_opacity(o).map_err(KernelError::MaterialOpacity)?;
        }
        if let Some(e) = emission {
            check_emission(e).map_err(KernelError::MaterialEmission)?;
        }
        if let Some(m) = metalness {
            check_unit("metalness", m).map_err(KernelError::MaterialFinish)?;
        }
        if let Some(r) = roughness {
            check_unit("roughness", r).map_err(KernelError::MaterialFinish)?;
        }
        Ok(())
    }

    /// Adds `material` after checking every part of it: the name (valid, see
    /// [`MAX_MATERIAL_NAME`], and not taken), the opacity (0 to 1), the texture (see
    /// [`MAX_TEXTURES`]) and the emission (0 to [`MAX_EMISSION`]).
    pub fn add_material_from(
        &mut self,
        material: Material,
    ) -> Result<(MaterialId, SceneChange), KernelError> {
        self.check_free_name(&material.name, None)?;
        check_opacity(material.opacity).map_err(KernelError::MaterialOpacity)?;
        check_emission(material.emission).map_err(KernelError::MaterialEmission)?;
        check_unit("metalness", material.metalness).map_err(KernelError::MaterialFinish)?;
        check_unit("roughness", material.roughness).map_err(KernelError::MaterialFinish)?;
        if let Some(d) = material.density {
            check_density(d).map_err(KernelError::MaterialDensity)?;
        }
        if let Some(t) = &material.texture {
            self.check_new_texture(t, None)?;
        }
        let id = self.alloc_material();
        let step = Step::AddMaterial(id, material);
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
        self.change_material_with_texture(id, name, color, opacity, None)
    }

    /// `change_material`, and `texture` sets (`Some(Some(_))`) or removes (`Some(None)`) the
    /// material's texture; None keeps it. One step, so undo restores every part.
    pub fn change_material_with_texture(
        &mut self,
        id: MaterialId,
        name: Option<&str>,
        color: Option<[u8; 3]>,
        opacity: Option<f32>,
        texture: Option<Option<Texture>>,
    ) -> Result<SceneChange, KernelError> {
        self.change_material_with(
            id,
            MaterialEdit {
                name,
                color,
                opacity,
                texture,
                emission: None,
                metalness: None,
                roughness: None,
                density: None,
            },
        )
    }

    /// Changes the parts of a material `edit` gives, after checking them as
    /// `add_material_from` does. One step, so undo restores every part; nothing to change
    /// changes nothing.
    pub fn change_material_with(
        &mut self,
        id: MaterialId,
        edit: MaterialEdit,
    ) -> Result<SceneChange, KernelError> {
        let from = self
            .material(id)
            .ok_or(KernelError::MaterialNotFound)?
            .clone();
        if let Some(name) = edit.name {
            self.check_free_name(name, Some(id))?;
        }
        if let Some(opacity) = edit.opacity {
            check_opacity(opacity).map_err(KernelError::MaterialOpacity)?;
        }
        if let Some(emission) = edit.emission {
            check_emission(emission).map_err(KernelError::MaterialEmission)?;
        }
        if let Some(m) = edit.metalness {
            check_unit("metalness", m).map_err(KernelError::MaterialFinish)?;
        }
        if let Some(r) = edit.roughness {
            check_unit("roughness", r).map_err(KernelError::MaterialFinish)?;
        }
        if let Some(Some(d)) = edit.density {
            check_density(d).map_err(KernelError::MaterialDensity)?;
        }
        if let Some(Some(t)) = &edit.texture {
            self.check_new_texture(t, Some(id))?;
        }
        let to = Material {
            name: edit.name.map_or_else(|| from.name.clone(), str::to_string),
            color: edit.color.unwrap_or(from.color),
            opacity: edit.opacity.unwrap_or(from.opacity),
            texture: edit.texture.unwrap_or_else(|| from.texture.clone()),
            emission: edit.emission.unwrap_or(from.emission),
            metalness: edit.metalness.unwrap_or(from.metalness),
            roughness: edit.roughness.unwrap_or(from.roughness),
            density: edit.density.unwrap_or(from.density),
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

    /// How many materials have a texture.
    pub fn texture_count(&self) -> usize {
        self.materials()
            .filter(|(_, m)| m.texture.is_some())
            .count()
    }

    /// Fails when `texture` is not valid, or when giving it to material `to` (None: a new one)
    /// would make more than [`MAX_TEXTURES`] textures.
    fn check_new_texture(
        &self,
        texture: &Texture,
        to: Option<MaterialId>,
    ) -> Result<(), KernelError> {
        check_texture(texture).map_err(KernelError::MaterialTexture)?;
        let others = self
            .materials()
            .filter(|(id, m)| Some(*id) != to && m.texture.is_some())
            .count();
        if others >= MAX_TEXTURES {
            return Err(KernelError::MaterialTexture(format!(
                "a model holds at most {MAX_TEXTURES} textures"
            )));
        }
        Ok(())
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
