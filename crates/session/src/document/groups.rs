//! Groups, components and the open context.

use std::collections::BTreeSet;

use kernel::{DefinitionKind, EntityId, KernelError, SceneChange};

use super::{Document, MAX_COORDINATE, check_coordinates};
use crate::command::{CommandError, ErrorKind};
use crate::query::{Bounds, InstanceInfo};

impl Document {
    /// Makes a group (`name` None) or a named component from `ids` and selects its instance.
    pub(super) fn group_changes(
        &mut self,
        ids: &[EntityId],
        name: Option<&String>,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge, face or instance",
            ));
        }
        let (kind, name) = match name {
            None => (DefinitionKind::Group, ""),
            Some(n) if n.trim().is_empty() => {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    "a component needs a name",
                ));
            }
            Some(n) => (DefinitionKind::Component, n.as_str()),
        };
        let parts = self.parts(ids)?;
        let mesh = self.active();
        let (instance, change) = self
            .scene
            .make_group(
                mesh,
                &parts.edges,
                &parts.faces,
                &parts.instances,
                kind,
                name,
            )
            .map_err(|e| invalid(e, ids))?;
        self.selection = BTreeSet::from([EntityId::Instance(instance)]);
        Ok(change)
    }

    /// Explodes instances and selects what comes out of them.
    pub(super) fn explode_changes(
        &mut self,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one instance",
            ));
        }
        let others: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| !matches!(id, EntityId::Instance(_)))
            .collect();
        if !others.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "only instances can be exploded",
            )
            .with_ids(others));
        }
        let parts = self.parts(ids)?;
        for i in &parts.instances {
            let placed = self.instance_box(*i).into_iter().flatten();
            let points: Vec<[f64; 3]> = placed.map(|p| p.to_array()).collect();
            if check_coordinates(&points).is_err() {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "{} would be placed beyond +/-{MAX_COORDINATE} m",
                        EntityId::Instance(*i)
                    ),
                )
                .with_ids(vec![EntityId::Instance(*i)]));
            }
        }
        let mesh = self.active();
        let mut done = Vec::new();
        for i in &parts.instances {
            match self.scene.explode(mesh, *i) {
                Ok(c) => done.push(c),
                Err(e) => {
                    for c in done.iter().rev() {
                        self.scene.apply(&c.inverse());
                    }
                    return Err(invalid(e, ids));
                }
            }
        }
        let change = SceneChange::join(done);
        self.selection = change
            .summary()
            .created
            .into_iter()
            .filter(|id| !matches!(id, EntityId::Vertex(_)) && self.exists(*id))
            .collect();
        Ok(change)
    }

    /// Opens an instance of the open context for editing.
    pub(super) fn open_instance(&mut self, id: EntityId) -> Result<(), CommandError> {
        let EntityId::Instance(i) = id else {
            return Err(
                CommandError::new(ErrorKind::InvalidInput, "only instances can be opened")
                    .with_ids(vec![id]),
            );
        };
        if self.model().instance(i).is_none() {
            return Err(
                CommandError::new(ErrorKind::NotFound, format!("no entity {id}"))
                    .with_ids(vec![id]),
            );
        }
        self.context.push(i);
        self.selection.clear();
        Ok(())
    }

    /// Closes the open instance.
    pub(super) fn close_instance(&mut self) -> Result<(), CommandError> {
        if self.context.pop().is_none() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "no instance is open",
            ));
        }
        self.selection.clear();
        Ok(())
    }

    pub(super) fn instance_info(&self, id: kernel::InstanceId) -> InstanceInfo {
        let i = self.model().instance(id).unwrap();
        let d = self.scene.definition(i.definition).unwrap();
        InstanceInfo {
            id: EntityId::Instance(id),
            definition: i.definition,
            name: d.name.clone(),
            kind: d.kind,
            transform: i.transform.to_cols_array(),
            bounds: self.instance_box(id).map(|[lo, hi]| Bounds {
                min: lo.to_array(),
                max: hi.to_array(),
            }),
        }
    }
}

fn invalid(e: KernelError, ids: &[EntityId]) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, e.to_string()).with_ids(ids.to_vec())
}
