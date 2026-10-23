//! Hiding and showing edges, faces, instances and guides.

use kernel::{EntityId, SceneChange};

use super::Document;
use crate::command::CommandError;

impl Document {
    /// Hides the edges, faces, instances and guides in `ids`. Dimensions, notes and section
    /// planes in `ids` are left alone; a vertex or an unknown id is refused. Hidden ones leave
    /// the selection (see `changed`).
    pub(super) fn hide_changes(&mut self, ids: &[EntityId]) -> Result<SceneChange, CommandError> {
        self.parts(ids)?;
        let (change, changed) = self.set_hidden(ids, true);
        if !changed.is_empty() {
            self.last_hidden = changed.clone();
        }
        self.hidden_changed = Some(changed);
        Ok(change)
    }

    /// Shows the entities in `ids` (same rules as `hide_changes`).
    pub(super) fn unhide_changes(&mut self, ids: &[EntityId]) -> Result<SceneChange, CommandError> {
        self.parts(ids)?;
        let (change, changed) = self.set_hidden(ids, false);
        self.hidden_changed = Some(changed);
        Ok(change)
    }

    /// Shows what the most recent `hide` hid that is in the open context and still hidden.
    pub(super) fn unhide_last_changes(&mut self) -> Result<SceneChange, CommandError> {
        let m = self.model();
        let ids: Vec<EntityId> = self
            .last_hidden
            .iter()
            .copied()
            .filter(|id| m.hidden_of(*id) == Some(true))
            .collect();
        let (change, changed) = self.set_hidden(&ids, false);
        self.hidden_changed = Some(changed);
        Ok(change)
    }

    /// Shows every hidden entity of the open context (not those inside its instances).
    pub(super) fn unhide_all_changes(&mut self) -> Result<SceneChange, CommandError> {
        let ids = self.model().hidden_entities();
        let (change, changed) = self.set_hidden(&ids, false);
        self.hidden_changed = Some(changed);
        Ok(change)
    }

    fn set_hidden(&mut self, ids: &[EntityId], hidden: bool) -> (SceneChange, Vec<EntityId>) {
        let mut changed = Vec::new();
        let change = self
            .edit_mesh(|m| {
                let (changes, c) = m.hide_entities(ids, hidden);
                changed = c;
                Ok(changes)
            })
            .expect("hiding cannot fail");
        (change, changed)
    }

    /// Whether the selection may not hold `id`: it is hidden (and Hidden Geometry, which lets
    /// hidden entities be picked, is off).
    pub(super) fn is_unselectable(&self, id: EntityId) -> bool {
        self.model().hidden_of(id) == Some(true)
    }
}
