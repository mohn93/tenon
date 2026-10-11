//! Groups and components: definitions with a mesh of their own, placed as instances.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{DefinitionId, EntityId};
use crate::model::{ChangeSet, ChangeSummary, Model};

/// A group has one instance, and a copy of it gets a definition of its own. A component is
/// shared by all its instances: editing it changes them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionKind {
    Group,
    Component,
}

/// Geometry that instances place in other meshes.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub name: String,
    pub kind: DefinitionKind,
    pub mesh: Model,
}

/// One of a scene's meshes: the root or a definition's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MeshId {
    Root,
    Definition(DefinitionId),
}

/// One invertible change to a scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Changes inside one mesh.
    Edit(MeshId, ChangeSet),
    AddDefinition(DefinitionId, Definition),
    RemoveDefinition(DefinitionId, Definition),
}

impl Step {
    fn inverse(&self) -> Step {
        match self.clone() {
            Step::Edit(mesh, c) => Step::Edit(mesh, c.inverse()),
            Step::AddDefinition(id, d) => Step::RemoveDefinition(id, d),
            Step::RemoveDefinition(id, d) => Step::AddDefinition(id, d),
        }
    }
}

/// The ordered steps of one edit. Applying `inverse()` undoes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneChange {
    pub steps: Vec<Step>,
}

impl SceneChange {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn inverse(&self) -> SceneChange {
        SceneChange {
            steps: self.steps.iter().rev().map(Step::inverse).collect(),
        }
    }

    /// Joins changes made one after another into one.
    pub fn join(changes: impl IntoIterator<Item = SceneChange>) -> SceneChange {
        SceneChange {
            steps: changes.into_iter().flat_map(|c| c.steps).collect(),
        }
    }

    /// Net effect per entity across all meshes (ids are unique in a scene). Definitions are not
    /// listed; the instances that place them are.
    pub fn summary(&self) -> ChangeSummary {
        let ops = self
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::Edit(_, c) => Some(c.ops.iter().cloned()),
                _ => None,
            })
            .flatten()
            .collect();
        ChangeSet { ops }.summary()
    }
}

impl From<Step> for SceneChange {
    fn from(step: Step) -> Self {
        SceneChange { steps: vec![step] }
    }
}

/// The whole model: a root mesh plus the definitions its instances place.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    root: Model,
    definitions: BTreeMap<DefinitionId, Definition>,
    /// The next id to hand out, in any mesh. Never decreases, so ids are never reused.
    next_id: u64,
}

/// Two scenes are equal when they hold the same entities and definitions under the same ids.
impl PartialEq for Scene {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.definitions == other.definitions
    }
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    /// A scene whose root mesh is `model`.
    pub fn from_root(model: Model) -> Self {
        Scene {
            next_id: model.next_id(),
            root: model,
            definitions: BTreeMap::new(),
        }
    }

    pub fn root(&self) -> &Model {
        &self.root
    }

    pub fn mesh(&self, id: MeshId) -> Option<&Model> {
        match id {
            MeshId::Root => Some(&self.root),
            MeshId::Definition(d) => self.definitions.get(&d).map(|d| &d.mesh),
        }
    }

    fn mesh_mut(&mut self, id: MeshId) -> Option<&mut Model> {
        match id {
            MeshId::Root => Some(&mut self.root),
            MeshId::Definition(d) => self.definitions.get_mut(&d).map(|d| &mut d.mesh),
        }
    }

    pub fn definition(&self, id: DefinitionId) -> Option<&Definition> {
        self.definitions.get(&id)
    }

    pub fn definitions(&self) -> impl Iterator<Item = (DefinitionId, &Definition)> {
        self.definitions.iter().map(|(k, v)| (*k, v))
    }

    /// Every mesh: the root first, then the definitions'.
    pub fn meshes(&self) -> impl Iterator<Item = (MeshId, &Model)> {
        std::iter::once((MeshId::Root, &self.root)).chain(
            self.definitions
                .iter()
                .map(|(id, d)| (MeshId::Definition(*id), &d.mesh)),
        )
    }

    /// How many instances of a definition the scene holds, in all meshes.
    pub fn instance_count(&self, id: DefinitionId) -> usize {
        self.meshes()
            .flat_map(|(_, m)| m.instances())
            .filter(|(_, i)| i.definition == id)
            .count()
    }

    /// Runs one kernel edit on a mesh and returns it as a scene change. New entities get ids
    /// unique across the scene. Panics if the mesh does not exist.
    pub fn edit(
        &mut self,
        mesh: MeshId,
        f: impl FnOnce(&mut Model) -> Result<ChangeSet, KernelError>,
    ) -> Result<SceneChange, KernelError> {
        let first = self.next_id;
        let m = self.mesh_mut(mesh).expect("edit of a missing mesh");
        m.reserve_ids(first);
        let result = f(m);
        let next = m.next_id();
        self.next_id = self.next_id.max(next);
        let changes = result?;
        Ok(if changes.is_empty() {
            SceneChange::default()
        } else {
            Step::Edit(mesh, changes).into()
        })
    }

    /// Adds an empty definition and returns its id and the step that added it.
    pub(crate) fn add_definition(
        &mut self,
        name: &str,
        kind: DefinitionKind,
    ) -> (DefinitionId, Step) {
        let id = DefinitionId(self.next_id);
        let step = Step::AddDefinition(
            id,
            Definition {
                name: name.to_string(),
                kind,
                mesh: Model::new(),
            },
        );
        self.apply_step(&step);
        (id, step)
    }

    /// Applies a change (e.g. an inverse for undo).
    pub fn apply(&mut self, change: &SceneChange) {
        for step in &change.steps {
            self.apply_step(step);
        }
    }

    fn apply_step(&mut self, step: &Step) {
        match step {
            Step::Edit(mesh, c) => {
                let m = self.mesh_mut(*mesh).expect("change to a missing mesh");
                m.apply(c);
                let next = m.next_id();
                self.next_id = self.next_id.max(next);
            }
            Step::AddDefinition(id, d) => {
                self.next_id = self.next_id.max(id.0 + 1).max(d.mesh.next_id());
                self.definitions.insert(*id, d.clone());
            }
            Step::RemoveDefinition(id, _) => {
                self.definitions.remove(id);
            }
        }
    }

    /// Checks every mesh, that instances place existing definitions, that no definition places
    /// itself (directly or deeper), and that no id is used in two meshes.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for (mesh, m) in self.meshes() {
            m.validate().map_err(|e| format!("{mesh:?}: {e}"))?;
            for (id, i) in m.instances() {
                if !self.definitions.contains_key(&i.definition) {
                    return Err(format!("instance {id:?} places a missing definition"));
                }
            }
            for id in entity_ids(m) {
                if !seen.insert(id) {
                    return Err(format!("{id} is used in two meshes"));
                }
            }
        }
        for id in self.definitions.keys() {
            if self.places(*id, *id) {
                return Err(format!("definition {id} places itself"));
            }
        }
        Ok(())
    }

    /// True when `inside`'s mesh places `target`, directly or through other definitions.
    fn places(&self, inside: DefinitionId, target: DefinitionId) -> bool {
        let mut stack = vec![inside];
        let mut visited = BTreeSet::new();
        while let Some(d) = stack.pop() {
            if !visited.insert(d) {
                continue;
            }
            for (_, i) in self.definitions[&d].mesh.instances() {
                if i.definition == target {
                    return true;
                }
                stack.push(i.definition);
            }
        }
        false
    }
}

fn entity_ids(m: &Model) -> impl Iterator<Item = EntityId> + '_ {
    m.vertices()
        .map(|(id, _)| EntityId::Vertex(id))
        .chain(m.edges().map(|(id, _)| EntityId::Edge(id)))
        .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
        .chain(m.instances().map(|(id, _)| EntityId::Instance(id)))
}
