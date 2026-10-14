//! Materials and painting.

use kernel::{EntityId, KernelError, MaterialId, SceneChange, Side};

use super::Document;
use crate::command::{CommandError, ErrorKind};
use crate::query::MaterialInfo;

/// The name `add_material` gives when none is passed (made unique with a number).
const DEFAULT_MATERIAL_NAME: &str = "Material";

impl Document {
    pub(super) fn add_material_changes(
        &mut self,
        name: Option<&str>,
        color: [u8; 3],
        opacity: Option<f32>,
    ) -> Result<SceneChange, CommandError> {
        let name = match name {
            Some(n) => n.to_string(),
            None => self.scene.free_material_name(DEFAULT_MATERIAL_NAME),
        };
        self.scene
            .add_material_with_opacity(&name, color, opacity.unwrap_or(1.0))
            .map(|(_, change)| change)
            .map_err(|e| material_error(e, None))
    }

    pub(super) fn change_material_changes(
        &mut self,
        id: MaterialId,
        name: Option<&str>,
        color: Option<[u8; 3]>,
        opacity: Option<f32>,
    ) -> Result<SceneChange, CommandError> {
        self.scene
            .change_material(id, name, color, opacity)
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
