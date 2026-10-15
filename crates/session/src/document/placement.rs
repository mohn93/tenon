//! Texture positions on faces.

use kernel::{EntityId, PlacementEdit, SceneChange};

use super::Document;
use crate::command::{CommandError, ErrorKind};

impl Document {
    /// Sets the texture placement of the faces in `ids`. Other kinds of entity are invalid
    /// input, as is an empty list or an edit that sets nothing.
    pub(super) fn position_texture_changes(
        &mut self,
        ids: &[EntityId],
        edit: PlacementEdit,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one face",
            ));
        }
        let others: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| !matches!(id, EntityId::Face(_)))
            .collect();
        if !others.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "only faces have a texture position",
            )
            .with_ids(others));
        }
        if edit == PlacementEdit::default() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "give offset, rotation_deg, scale or reset",
            ));
        }
        let faces = self.parts(ids)?.faces;
        let mut count = 0;
        let change = self
            .edit_mesh(|m| {
                let (changes, n) = m.position_texture(&faces, edit)?;
                count = n;
                Ok(changes)
            })
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
        self.positioned = Some(count);
        Ok(change)
    }
}
