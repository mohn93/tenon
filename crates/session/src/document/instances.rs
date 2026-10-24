//! Placing instances and setting their transforms.

use kernel::{DefinitionId, EntityId, SceneChange};

use super::Document;
use crate::command::{CommandError, ErrorKind, Transform};

impl Document {
    /// Places a new instance of `definition` in the open context and selects it.
    pub(super) fn place_instance_changes(
        &mut self,
        definition: DefinitionId,
        transform: &Transform,
    ) -> Result<SceneChange, CommandError> {
        let t = transform.to_affine()?;
        if self.scene.definition(definition).is_none() {
            return Err(CommandError::new(
                ErrorKind::NotFound,
                format!("no definition {definition}"),
            ));
        }
        let mesh = self.active();
        let (id, change) = self
            .scene
            .place_instance(mesh, definition, t)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
        self.selection = std::collections::BTreeSet::from([EntityId::Instance(id)]);
        Ok(change)
    }

    /// Replaces the transform of an instance of the open context.
    pub(super) fn set_transform_changes(
        &mut self,
        id: EntityId,
        transform: &Transform,
    ) -> Result<SceneChange, CommandError> {
        let t = transform.to_affine()?;
        let EntityId::Instance(instance) = id else {
            return Err(
                CommandError::new(ErrorKind::InvalidInput, "id must name an instance")
                    .with_ids(vec![id]),
            );
        };
        if !self.exists(id) {
            return Err(self.no_such_entities(vec![id]));
        }
        let mesh = self.active();
        self.scene
            .set_instance_transform(mesh, instance, t)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
    }
}
