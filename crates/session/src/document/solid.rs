//! Solid Tools: `solid_union`, `solid_subtract` and `solid_intersect`.

use std::collections::BTreeSet;

use kernel::boolean::{BooleanError, BooleanOp, NOT_CLOSED, TOO_MUCH};
use kernel::{EntityId, InstanceId, SceneChange};

use super::Document;
use crate::command::{CommandError, ErrorKind};

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

impl Document {
    /// `solid_union` or `solid_intersect` of `ids` (two or more).
    pub(super) fn solid_all_changes(
        &mut self,
        op: BooleanOp,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        if ids.len() < 2 {
            return Err(invalid("ids must name at least 2 groups or components"));
        }
        self.solid_changes(op, ids)
    }

    /// `solid_subtract`: `target` minus each of `tools` in order.
    pub(super) fn solid_subtract_changes(
        &mut self,
        target: EntityId,
        tools: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        if tools.is_empty() {
            return Err(invalid("tools must name at least 1 group or component"));
        }
        if tools.contains(&target) {
            return Err(
                invalid(format!("{target} is the target and cannot also be a tool"))
                    .with_ids(vec![target]),
            );
        }
        let ids: Vec<EntityId> = std::iter::once(target)
            .chain(tools.iter().copied())
            .collect();
        self.solid_changes(BooleanOp::Subtract, &ids)
    }

    /// Combines the instances `ids` of the open context (the first one first), as
    /// `Scene::solid_boolean` does. The new group becomes the selection.
    fn solid_changes(
        &mut self,
        op: BooleanOp,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
        if !missing.is_empty() {
            return Err(self.no_such_entities(missing));
        }
        let mut seen = BTreeSet::new();
        let mut instances: Vec<InstanceId> = Vec::new();
        for id in ids {
            let EntityId::Instance(i) = *id else {
                return Err(
                    invalid(format!("{id} is not a group or component")).with_ids(vec![*id])
                );
            };
            if !seen.insert(i) {
                return Err(invalid(format!("{id} is named twice")).with_ids(vec![*id]));
            }
            instances.push(i);
        }
        let mesh = self.active();
        let (placed, change) =
            self.scene
                .solid_boolean(mesh, op, &instances)
                .map_err(|e| match e {
                    BooleanError::Missing(i) => self.no_such_entities(vec![EntityId::Instance(i)]),
                    BooleanError::NotSolid(i, why) => {
                        invalid(format!("{} is not a solid: {why}", EntityId::Instance(i)))
                            .with_ids(vec![EntityId::Instance(i)])
                    }
                    BooleanError::NotClosed => invalid(NOT_CLOSED).with_ids(ids.to_vec()),
                    BooleanError::TooMuch => invalid(TOO_MUCH),
                })?;
        match placed {
            Some(p) => self.selection = BTreeSet::from([EntityId::Instance(p)]),
            None => self.solid_empty = Some(true),
        }
        Ok(change)
    }
}
