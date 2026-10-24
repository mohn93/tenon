//! Solid Tools: `solid_union`, `solid_subtract`, `solid_intersect`, `solid_trim`, `solid_split`
//! and `outer_shell`.

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

    /// `solid_trim`: as `solid_subtract`, but the tools stay.
    pub(super) fn solid_trim_changes(
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
        let instances = self.solid_instances(&ids)?;
        let mesh = self.active();
        let result = self.scene.solid_trim(mesh, instances[0], &instances[1..]);
        self.solid_finish(result.map(|(p, c)| (p.into_iter().collect(), c)), &ids)
    }

    /// `solid_split` of `a` and `b`.
    pub(super) fn solid_split_changes(
        &mut self,
        a: EntityId,
        b: EntityId,
    ) -> Result<SceneChange, CommandError> {
        let ids = [a, b];
        let instances = self.solid_instances(&ids)?;
        let mesh = self.active();
        let result = self.scene.solid_split(mesh, instances[0], instances[1]);
        self.solid_finish(result, &ids)
    }

    /// `outer_shell` of `ids` (one or more).
    pub(super) fn outer_shell_changes(
        &mut self,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(invalid("ids must name at least 1 group or component"));
        }
        let instances = self.solid_instances(ids)?;
        let mesh = self.active();
        let result = self.scene.outer_shell(mesh, &instances);
        self.solid_finish(result.map(|(p, c)| (p.into_iter().collect(), c)), ids)
    }

    /// Combines the instances `ids` of the open context (the first one first), as
    /// `Scene::solid_boolean` does. The new group becomes the selection.
    fn solid_changes(
        &mut self,
        op: BooleanOp,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        let instances = self.solid_instances(ids)?;
        let mesh = self.active();
        let result = self.scene.solid_boolean(mesh, op, &instances);
        self.solid_finish(result.map(|(p, c)| (p.into_iter().collect(), c)), ids)
    }

    /// The instances named by `ids`, each once; anything else is refused.
    fn solid_instances(&self, ids: &[EntityId]) -> Result<Vec<InstanceId>, CommandError> {
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
        Ok(instances)
    }

    /// The outcome of a solid operation: errors as commands report them; the groups made
    /// become the selection, none made sets the outcome's `empty`.
    fn solid_finish(
        &mut self,
        result: Result<(Vec<InstanceId>, SceneChange), BooleanError>,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        let (placed, change) = result.map_err(|e| match e {
            BooleanError::Missing(i) => self.no_such_entities(vec![EntityId::Instance(i)]),
            BooleanError::NotSolid(i, why) => {
                invalid(format!("{} is not a solid: {why}", EntityId::Instance(i)))
                    .with_ids(vec![EntityId::Instance(i)])
            }
            BooleanError::NotClosed => invalid(NOT_CLOSED).with_ids(ids.to_vec()),
            BooleanError::TooMuch => invalid(TOO_MUCH),
        })?;
        if placed.is_empty() {
            self.solid_empty = Some(true);
        } else {
            self.selection = placed.into_iter().map(EntityId::Instance).collect();
        }
        Ok(change)
    }
}
