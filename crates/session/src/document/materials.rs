//! Materials, their textures, and painting.

use std::path::Path;
use std::sync::Arc;

use kernel::{
    EntityId, KernelError, MAX_TEXTURE_SIZE, MIN_TEXTURE_SIZE, Material, MaterialEdit, MaterialId,
    SceneChange, Side, Texture,
};

use super::Document;
use super::import::read_limited;
use crate::command::{CommandError, ErrorKind, ExistingMaterial, TextureSource};
use crate::query::{MaterialInfo, TextureInfo};
use crate::texture::{
    MAX_TEXTURE_BYTES, average_color, check_total_pixels, dimensions, texture_from_bytes,
};

/// The name `add_material` gives when none is passed (made unique with a number).
const DEFAULT_MATERIAL_NAME: &str = "Material";

impl Document {
    /// Adds a material; without `color` it takes the texture's average color.
    pub(super) fn add_material_changes(
        &mut self,
        name: Option<&str>,
        color: Option<[u8; 3]>,
        opacity: Option<f32>,
        texture: Option<&TextureSource>,
        emission: Option<f32>,
        existing: ExistingMaterial,
    ) -> Result<SceneChange, CommandError> {
        self.existing_material = None;
        let found = match (existing, name) {
            (ExistingMaterial::Fail, _) => None,
            (_, None) => {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    "existing: reuse and update need a name",
                ));
            }
            (_, Some(n)) => self
                .scene
                .materials()
                .find(|(_, m)| m.name == n)
                .map(|(id, _)| id),
        };
        if let (Some(id), ExistingMaterial::Update) = (found, existing) {
            let change = self.change_material_changes(
                id,
                MaterialEdit {
                    name: None,
                    color,
                    opacity,
                    texture: None,
                    emission,
                },
                texture.map(Some),
            )?;
            self.existing_material = Some(id);
            return Ok(change);
        }
        if texture.is_some_and(|t| t.path.is_none()) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a new material's texture needs a path",
            ));
        }
        let others = self.texture_pixels_besides(None);
        let texture = texture.map(|t| texture_of(t, None, others)).transpose()?;
        let Some(color) = color.or_else(|| texture.as_ref().map(|t| average_color(&t.image)))
        else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "give a material a color or a texture",
            ));
        };
        let name = match name {
            Some(n) => n.to_string(),
            None => self.scene.free_material_name(DEFAULT_MATERIAL_NAME),
        };
        let material = Material {
            name,
            color,
            opacity: opacity.unwrap_or(1.0),
            texture,
            emission: emission.unwrap_or(0.0),
        };
        if let Some(id) = found {
            // Reuse: the same checks as for a new material, then nothing changes.
            self.scene
                .check_material_parts(&material)
                .map_err(|e| material_error(e, None))?;
            self.existing_material = Some(id);
            return Ok(SceneChange::default());
        }
        self.scene
            .add_material_from(material)
            .map(|(_, change)| change)
            .map_err(|e| material_error(e, None))
    }

    /// Changes the parts of a material `edit` gives (its texture aside). `texture`: None keeps
    /// it, `Some(None)` removes it. A newly read image sets the color to its average unless
    /// `edit` gives one.
    pub(super) fn change_material_changes(
        &mut self,
        id: MaterialId,
        mut edit: MaterialEdit,
        texture: Option<Option<&TextureSource>>,
    ) -> Result<SceneChange, CommandError> {
        let Some(material) = self.scene.material(id) else {
            return Err(material_error(KernelError::MaterialNotFound, Some(id)));
        };
        let read = matches!(texture, Some(Some(s)) if s.path.is_some());
        let texture = match texture {
            None => None,
            Some(None) => Some(None),
            Some(Some(source)) => Some(Some(texture_of(
                source,
                material.texture.as_ref(),
                self.texture_pixels_besides(Some(id)),
            )?)),
        };
        if let (Some(Some(t)), None) = (&texture, edit.color)
            && read
        {
            edit.color = Some(average_color(&t.image));
        }
        edit.texture = texture;
        self.scene
            .change_material_with(id, edit)
            .map_err(|e| material_error(e, Some(id)))
    }

    pub(super) fn delete_material_changes(
        &mut self,
        id: MaterialId,
    ) -> Result<SceneChange, CommandError> {
        self.scene
            .delete_material(id)
            .map_err(|e| material_error(e, Some(id)))
    }

    /// Paints faces and instances of the open context. Edges, guides and vertices are
    /// invalid input, as is an empty list.
    pub(super) fn paint_changes(
        &mut self,
        ids: &[EntityId],
        material: Option<MaterialId>,
        side: Side,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one face or instance",
            ));
        }
        let others: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| !matches!(id, EntityId::Face(_) | EntityId::Instance(_)))
            .collect();
        if !others.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "only faces and instances can be painted",
            )
            .with_ids(others));
        }
        let parts = self.parts(ids)?;
        let mesh = self.active();
        self.scene
            .paint(mesh, &parts.faces, &parts.instances, side, material)
            .map_err(|e| material_error(e, material))
    }

    /// The pixels the texture images of materials other than `except` hold, each image counted
    /// once however many materials share it.
    pub(crate) fn texture_pixels_besides(&self, except: Option<MaterialId>) -> u64 {
        let mut seen = std::collections::HashSet::new();
        self.scene
            .materials()
            .filter(|(id, _)| Some(*id) != except)
            .filter_map(|(_, m)| m.texture.as_ref())
            .filter(|t| seen.insert(Arc::as_ptr(&t.image)))
            .map(|t| u64::from(t.image.width) * u64::from(t.image.height))
            .sum()
    }

    /// Every material with how many faces and instances use it.
    pub(super) fn material_infos(&self) -> Vec<MaterialInfo> {
        self.scene
            .materials()
            .map(|(id, m)| {
                let meshes = || self.scene.meshes().map(|(_, mesh)| mesh);
                MaterialInfo {
                    id,
                    name: m.name.clone(),
                    color: m.color,
                    opacity: m.opacity,
                    emission: m.emission,
                    texture: m.texture.as_ref().map(|t| TextureInfo {
                        width: t.width,
                        height: t.height,
                        pixel_width: t.image.width,
                        pixel_height: t.image.height,
                        format: t.image.format,
                    }),
                    faces: meshes()
                        .flat_map(|mesh| mesh.faces())
                        .filter(|(_, f)| f.paint().contains(&Some(id)))
                        .count(),
                    instances: meshes()
                        .flat_map(|mesh| mesh.instances())
                        .filter(|(_, i)| i.material == Some(id))
                        .count(),
                }
            })
            .collect()
    }
}

/// The texture `source` asks for: the image at its path, or else `current`'s. A new image
/// without a width is as wide as `current` (or 1 m), and without a height keeps its
/// proportions; a kept image keeps any size not given. A file that cannot be read or decoded,
/// or whose image would take the textures' pixels (`others` besides it) over the total budget,
/// is a `file` error naming it.
fn texture_of(
    source: &TextureSource,
    current: Option<&Texture>,
    others: u64,
) -> Result<Texture, CommandError> {
    // A new image without a width is as wide as `current` (or 1 m), and without a height
    // keeps its proportions (kept within the limits a given one must meet).
    let width = source.width.or(current.map(|t| t.width)).unwrap_or(1.0);
    let height_of = |pixels_wide: u32, pixels_high: u32| {
        source.height.unwrap_or_else(|| {
            (width * f64::from(pixels_high) / f64::from(pixels_wide))
                .clamp(MIN_TEXTURE_SIZE, MAX_TEXTURE_SIZE)
        })
    };
    match &source.path {
        Some(path) => {
            super::files::check_nul(Path::new(path))?;
            let file_error = |m: String| CommandError::new(ErrorKind::File, format!("{path}: {m}"));
            let bytes = read_limited(Path::new(path), MAX_TEXTURE_BYTES).map_err(file_error)?;
            let (_, w, h) = dimensions(&bytes).map_err(file_error)?;
            check_total_pixels(others, u64::from(w) * u64::from(h)).map_err(file_error)?;
            texture_from_bytes(bytes, width, height_of(w, h)).map_err(file_error)
        }
        None => match current {
            Some(t) => Ok(Texture {
                image: t.image.clone(),
                width,
                height: source.height.unwrap_or(t.height),
            }),
            None => Err(CommandError::new(
                ErrorKind::InvalidInput,
                "the material has no texture: give a texture path",
            )),
        },
    }
}

/// A kernel refusal as a command error; a missing material is `not_found`, naming `id`.
fn material_error(e: KernelError, id: Option<MaterialId>) -> CommandError {
    match (e, id) {
        (KernelError::MaterialNotFound, Some(id)) => {
            CommandError::new(ErrorKind::NotFound, format!("no material {id}"))
        }
        (KernelError::MaterialNotFound, None) => {
            CommandError::new(ErrorKind::NotFound, "no such material")
        }
        (e, _) => CommandError::new(ErrorKind::InvalidInput, e.to_string()),
    }
}
