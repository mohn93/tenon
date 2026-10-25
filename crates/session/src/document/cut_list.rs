//! The `cut_list` query (see `kernel::cutlist`): the open context's parts as rows.

use kernel::cutlist::{self, Grouping};
use kernel::{EntityId, InstanceId};

use super::Document;
use crate::command::{CommandError, ErrorKind};
use crate::query::{CutListGrouping, CutRowInfo};

impl Document {
    /// The cut list of `ids` (instances of the open context; every one when empty) grouped as
    /// asked: an id that is not an instance is `invalid_input`, an unknown one `not_found`.
    pub fn cut_list(
        &self,
        ids: &[EntityId],
        group_by: CutListGrouping,
    ) -> Result<Vec<CutRowInfo>, CommandError> {
        let m = self.model();
        let mut instances: Vec<InstanceId> = Vec::new();
        for id in ids {
            let EntityId::Instance(i) = *id else {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    format!("{id} is not a group or component"),
                )
                .with_ids(vec![*id]));
            };
            if m.instance(i).is_none() {
                return Err(self.missing_error(vec![*id], format!("no entity {id}")));
            }
            instances.push(i);
        }
        let inherited = cutlist::context_material(&self.scene, &self.context);
        let grouping = match group_by {
            CutListGrouping::Definition => Grouping::Definition,
            CutListGrouping::None => Grouping::None,
        };
        let rows = cutlist::cut_list(&self.scene, m, &instances, inherited, grouping)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e))?;
        Ok(rows
            .into_iter()
            .map(|r| CutRowInfo {
                name: r.name,
                material: r
                    .material
                    .and_then(|m| self.scene.material(m))
                    .map(|m| m.name.clone()),
                length: r.size[0],
                width: r.size[1],
                thickness: r.size[2],
                quantity: r.quantity,
                solid: r.solid,
                note: r.note,
                ids: r.ids.into_iter().map(EntityId::Instance).collect(),
            })
            .collect())
    }
}
