//! Library items: saving the selection's component, a material or the model to a `.skr` file
//! with a `library.json` entry, and inserting an item.

use std::path::{Path, PathBuf};

use glam::{DAffine3, DVec3};
use kernel::{DefinitionKind, EntityId, Material, MaterialId, Scene, SceneChange};

use super::{Document, check_coordinates};
use crate::clip;
use crate::command::{CommandError, ErrorKind};
use crate::document::clip::new_token;
use crate::file::{self, Access};
use crate::fspath::{canonical, same};
use crate::library::{self, LibraryKind, LibraryRef};

impl Document {
    /// Writes a library item to the `.skr` file at `path`: a component (the instance `item`,
    /// at the origin with its rotation and scale kept), a material (`item`, alone, texture
    /// included) or the whole model. `name` defaults to the component's or material's name,
    /// or the file's. Refuses a path that is not a `.skr` file, the open model's own file and
    /// an existing file unless `overwrite`. Changes nothing in the model.
    pub fn save_to_library(
        &self,
        path: &str,
        kind: LibraryKind,
        item: Option<LibraryRef>,
        name: Option<&str>,
        overwrite: bool,
    ) -> Result<(), CommandError> {
        let invalid = |m: String| CommandError::new(ErrorKind::InvalidInput, m);
        let target = Path::new(path);
        super::files::check_nul(target)?;
        let file_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !library::is_skr(file_name) {
            return Err(invalid(format!(
                "{path} is not a library file: it must be named <name>.skr"
            )));
        }
        if let Some(name) = name {
            library::check_name(name)?;
        }
        let (scene, default_name) = match (kind, item) {
            (LibraryKind::Model, None) => (
                None,
                file_name[..file_name.len() - 4].chars().take(256).collect(),
            ),
            (LibraryKind::Model, Some(_)) => {
                return Err(invalid("a model item takes no item id".into()));
            }
            (LibraryKind::Component, Some(LibraryRef::Entity(EntityId::Instance(id)))) => {
                let (scene, name) = self.component_item(id)?;
                (Some(scene), name)
            }
            (LibraryKind::Component, _) => {
                return Err(invalid(
                    "a component item needs the id of an instance".into(),
                ));
            }
            (LibraryKind::Material, Some(LibraryRef::Material(id))) => {
                let (scene, name) = self.material_item(id)?;
                (Some(scene), name)
            }
            (LibraryKind::Material, _) => {
                return Err(invalid("a material item needs the id of a material".into()));
            }
        };
        let name = name.map_or(default_name, str::to_string);
        library::check_name(&name)?;

        let own = self.path.as_deref().map(resolved);
        if own.is_some_and(|own| same(&own, &resolved(target))) {
            return Err(invalid(format!(
                "{path} is the open model's own file: use save, not save_to_library"
            )));
        }
        if !overwrite && std::fs::symlink_metadata(target).is_ok() {
            return Err(invalid(format!(
                "{path} already exists: pass \"overwrite\": true to replace it"
            )));
        }
        let info = library::encode_info(kind, &name)?;
        let extra: [(&str, &[u8]); 1] = [(library::INFO_ENTRY, &info)];
        let bytes = file::write_bytes(
            scene.as_ref().unwrap_or(&self.scene),
            &self.camera,
            self.view_look(),
            file::ENTRY,
            &extra,
        )
        .map_err(|e| file_error(path, e))?;
        if bytes.len() as u64 > library::MAX_ITEM_BYTES {
            return Err(invalid(format!(
                "the item is too large for a library: over {} bytes",
                library::MAX_ITEM_BYTES
            )));
        }
        file::replace_file(target, &bytes, Access::Default).map_err(|e| file_error(path, e))
    }

    /// A scene holding the component instance `id` of the open context alone, at the origin
    /// with its rotation and scale, and the component's name.
    fn component_item(&self, id: kernel::InstanceId) -> Result<(Scene, String), CommandError> {
        let instance = self.model().instance(id).ok_or_else(|| {
            let id = EntityId::Instance(id);
            self.missing_error(vec![id], format!("no instance {id}"))
        })?;
        let definition = self.scene.definition(instance.definition).ok_or_else(|| {
            CommandError::new(ErrorKind::NotFound, "the instance's definition is missing")
        })?;
        if definition.kind != DefinitionKind::Component {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a group cannot be a library component: Make it a component first",
            ));
        }
        let context = self.context_transform();
        let world = context * instance.transform;
        let to_world = DAffine3::from_translation(-world.translation) * context;
        let parts = self.parts(&[EntityId::Instance(id)])?;
        let scene = self
            .scene
            .extract_with(self.active(), &parts, to_world)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
        Ok((scene, definition.name.clone()))
    }

    /// A scene holding material `id` alone, and its name.
    fn material_item(&self, id: MaterialId) -> Result<(Scene, String), CommandError> {
        let material = self
            .scene
            .material(id)
            .ok_or_else(|| CommandError::new(ErrorKind::NotFound, format!("no material {id}")))?;
        let mut scene = Scene::new();
        scene
            .add_material_from(material.clone())
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
        Ok((scene, material.name.clone()))
    }

    /// Inserts the library item at `path`. A component or model: its scene as a clip taken
    /// from no document, so every definition is new, placed with the minimum corner of its
    /// box at `at`, or at the open context's origin; what it made is selected. A material: added
    /// as it is, or reused when the model has an identical one under the name, or added as
    /// "<name> 2", "<name> 3"... (the lowest free, the name shortened to fit the length limit); its id is the outcome's `material`.
    pub(super) fn insert_library_changes(
        &mut self,
        path: &str,
        at: Option<[f64; 3]>,
    ) -> Result<SceneChange, CommandError> {
        if let Some(at) = at {
            check_coordinates(&[at])?;
        }
        super::files::check_nul(Path::new(path))?;
        let item = library::read_item(Path::new(path))?;
        if item.kind == LibraryKind::Material {
            return self.insert_material(&item.scene);
        }
        let clip = clip::from_scene(item.scene, new_token())?;
        // World orientation and size are kept: the item's world box minimum lands at the world
        // point of `at` (a point of the open context), or of the context's origin.
        let context = self.context_transform();
        let point = at.map_or(DVec3::ZERO, DVec3::from);
        let placement = context.inverse()
            * DAffine3::from_translation(context.transform_point3(point) - clip.base);
        self.paste_clip(clip, placement, false)
    }

    fn insert_material(&mut self, item: &Scene) -> Result<SceneChange, CommandError> {
        let material: Material =
            item.materials()
                .next()
                .map(|(_, m)| m.clone())
                .ok_or_else(|| {
                    CommandError::new(ErrorKind::InvalidInput, "the item holds no material")
                })?;
        for n in 1.. {
            let name = if n == 1 {
                material.name.clone()
            } else {
                // The base is shortened so the suffix still fits the kernel's name limit.
                let suffix = format!(" {n}");
                let room = kernel::MAX_MATERIAL_NAME.saturating_sub(suffix.chars().count());
                let base: String = material.name.chars().take(room).collect();
                format!("{base}{suffix}")
            };
            let existing = self
                .scene
                .materials()
                .find(|(_, m)| m.name == name)
                .map(|(id, m)| (id, library::same_material(m, &material)));
            match existing {
                Some((id, true)) => {
                    self.library_material = Some(id);
                    return Ok(SceneChange::default());
                }
                Some((_, false)) => {}
                None => {
                    let (id, change) = self
                        .scene
                        .add_material_from(Material {
                            name,
                            ..material.clone()
                        })
                        .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
                    self.library_material = Some(id);
                    return Ok(change);
                }
            }
        }
        unreachable!("materials are finite")
    }
}

/// `path` resolved after links as far as it exists: itself, else its folder with the file name.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(real) = canonical(path) {
        return real;
    }
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    match (absolute.parent(), absolute.file_name()) {
        (Some(dir), Some(name)) => canonical(dir).map_or(absolute.clone(), |d| d.join(name)),
        _ => absolute,
    }
}

fn file_error(path: &str, e: String) -> CommandError {
    CommandError::new(ErrorKind::File, format!("{path}: {e}"))
}
