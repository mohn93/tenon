//! Groups, components and the open context.

use std::collections::BTreeSet;

use kernel::{DefinitionKind, EntityId, InstanceId, KernelError, MeshId, SceneChange};

use super::{Document, MAX_COORDINATE, check_coordinates};
use crate::command::{CommandError, ErrorKind};
use crate::query::{Bounds, InstanceInfo, OutlinerNode, OutlinerRow, OutlinerTable};

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
        if parts.edges.is_empty() && parts.faces.is_empty() && parts.instances.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "guides alone cannot be grouped: include an edge, face or instance",
            )
            .with_ids(ids.to_vec()));
        }
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
        let definition = self.model().instance(instance).expect("placed").definition;
        let guides = self
            .scene
            .move_guides(mesh, MeshId::Definition(definition), &parts.guides);
        self.selection = BTreeSet::from([EntityId::Instance(instance)]);
        Ok(SceneChange::join([change, guides]))
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
            let mut points: Vec<[f64; 3]> = placed.map(|p| p.to_array()).collect();
            // Guides come out too, and the instance box does not cover them.
            if let Some(inst) = self.model().instance(*i)
                && let Some(def) = self.scene.definition(inst.definition)
            {
                points.extend(
                    def.mesh
                        .guides()
                        .map(|(_, g)| inst.transform.transform_point3(g.anchor()).to_array()),
                );
            }
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

    /// Makes `path` the open context; the selection is cleared unless it already was.
    pub(super) fn open_path(&mut self, path: &[EntityId]) -> Result<(), CommandError> {
        let mut context = Vec::with_capacity(path.len());
        let mut mesh = MeshId::Root;
        for id in path {
            let EntityId::Instance(i) = *id else {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    format!("{id} is not an instance: only instances can be opened"),
                )
                .with_ids(vec![*id]));
            };
            let Some(instance) = self.scene.mesh(mesh).and_then(|m| m.instance(i)) else {
                let place = match context.last() {
                    None => "the root".to_string(),
                    Some(parent) => EntityId::Instance(*parent).to_string(),
                };
                return Err(CommandError::new(
                    ErrorKind::NotFound,
                    format!("no instance {id} in {place}"),
                )
                .with_ids(vec![*id]));
            };
            mesh = MeshId::Definition(instance.definition);
            context.push(i);
        }
        if context != self.context {
            self.context = context;
            self.selection.clear();
        }
        Ok(())
    }

    /// The tree of placed groups and components, from the root's instances down, with the
    /// open path and the selection marked. Built from `outliner_rows`; its size grows with
    /// nesting, since each placement repeats its definition's subtree; `depth` limits how
    /// many levels below the root's instances it goes (None: all).
    pub fn outliner(&self, depth: Option<u32>) -> Vec<OutlinerNode> {
        let table = self.outliner_rows();
        outliner_nodes(&table, &table.roots, &mut Vec::new(), depth, self)
    }

    /// The outliner tree stored flat: the root's instances and the instances each definition
    /// places. Linear in the model; no open or selected marks (they follow the context and
    /// selection, which change no revision).
    pub fn outliner_rows(&self) -> OutlinerTable {
        let rows = |mesh: &kernel::Model| -> Vec<OutlinerRow> {
            mesh.instances()
                .map(|(id, i)| {
                    let d = self.scene.definition(i.definition).expect("placed");
                    OutlinerRow {
                        id: EntityId::Instance(id),
                        definition: i.definition,
                        kind: d.kind,
                        name: d.name.clone(),
                        material: i.material,
                    }
                })
                .collect()
        };
        OutlinerTable {
            roots: rows(self.scene.root()),
            by_definition: self
                .scene
                .definitions()
                .map(|(id, d)| (id, rows(&d.mesh)))
                .collect(),
        }
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
            material: i.material,
            tag: i.tag,
        }
    }
}

fn invalid(e: KernelError, ids: &[EntityId]) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, e.to_string()).with_ids(ids.to_vec())
}

/// The nodes for `rows`, placed along `path` (instance ids from the root), with their marks.
/// `depth` is how many levels below these nodes to include (None: all).
fn outliner_nodes(
    table: &OutlinerTable,
    rows: &[OutlinerRow],
    path: &mut Vec<InstanceId>,
    depth: Option<u32>,
    doc: &Document,
) -> Vec<OutlinerNode> {
    let in_context = *path == doc.context;
    rows.iter()
        .map(|row| {
            let EntityId::Instance(id) = row.id else {
                unreachable!("outliner rows are instances")
            };
            path.push(id);
            let open = doc.context.starts_with(path);
            let below = table.children(row.definition);
            let (children, truncated) = match depth {
                Some(0) => (Vec::new(), !below.is_empty()),
                _ => {
                    let depth = depth.map(|d| d - 1);
                    (outliner_nodes(table, below, path, depth, doc), false)
                }
            };
            path.pop();
            OutlinerNode {
                id: row.id,
                definition: row.definition,
                kind: row.kind,
                name: row.name.clone(),
                material: row.material,
                open,
                selected: in_context && doc.selection.contains(&row.id),
                children,
                truncated,
            }
        })
        .collect()
}
