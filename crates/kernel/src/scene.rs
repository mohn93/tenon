//! Groups and components: definitions with a mesh of their own, placed as instances.

use std::collections::{BTreeMap, BTreeSet};

use glam::{DAffine3, DVec3};
use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{CurveId, DefinitionId, EntityId, InstanceId, MaterialId};
use crate::material::Material;
use crate::model::{ChangeSet, ChangeSummary, Instance, Model};

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
    /// A definition's name changed from the first to the second.
    RenameDefinition(DefinitionId, String, String),
    AddMaterial(MaterialId, Material),
    RemoveMaterial(MaterialId, Material),
    /// A material's name or color changed from the first to the second.
    ChangeMaterial(MaterialId, Material, Material),
}

impl Step {
    fn inverse(&self) -> Step {
        match self.clone() {
            Step::Edit(mesh, c) => Step::Edit(mesh, c.inverse()),
            Step::AddDefinition(id, d) => Step::RemoveDefinition(id, d),
            Step::RemoveDefinition(id, d) => Step::AddDefinition(id, d),
            Step::RenameDefinition(id, from, to) => Step::RenameDefinition(id, to, from),
            Step::AddMaterial(id, m) => Step::RemoveMaterial(id, m),
            Step::RemoveMaterial(id, m) => Step::AddMaterial(id, m),
            Step::ChangeMaterial(id, from, to) => Step::ChangeMaterial(id, to, from),
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
    /// listed; the instances that place them are. Materials added, removed or changed are
    /// listed in `materials` (one added and removed again is not).
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
        let mut summary = ChangeSet { ops }.summary();
        // material -> (existed before, exists after)
        let mut state: BTreeMap<MaterialId, (bool, bool)> = BTreeMap::new();
        for step in &self.steps {
            let (id, before, after) = match step {
                Step::AddMaterial(id, _) => (*id, false, true),
                Step::RemoveMaterial(id, _) => (*id, true, false),
                Step::ChangeMaterial(id, _, _) => (*id, true, true),
                _ => continue,
            };
            state.entry(id).or_insert((before, after)).1 = after;
        }
        summary.materials = state
            .into_iter()
            .filter(|(_, (before, after))| *before || *after)
            .map(|(id, _)| id)
            .collect();
        summary
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
    /// The materials faces and instances may be painted with, shared by every mesh.
    materials: BTreeMap<MaterialId, Material>,
    /// The next id to hand out, in any mesh. Never decreases, so ids are never reused.
    next_id: u64,
}

/// Two scenes are equal when they hold the same entities, definitions and materials under the
/// same ids.
impl PartialEq for Scene {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && self.definitions == other.definitions
            && self.materials == other.materials
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
            materials: BTreeMap::new(),
        }
    }

    /// A scene of these meshes; new ids start at `next_id` or after the largest id used,
    /// whichever is later. The caller validates the result.
    pub(crate) fn from_meshes(
        root: Model,
        definitions: BTreeMap<DefinitionId, Definition>,
        materials: BTreeMap<MaterialId, Material>,
        next_id: u64,
    ) -> Self {
        let used = definitions
            .iter()
            .map(|(id, d)| id.0.saturating_add(1).max(d.mesh.next_id()))
            .chain(materials.keys().map(|id| id.0.saturating_add(1)))
            .chain([root.next_id(), next_id])
            .max()
            .unwrap_or(0);
        Scene {
            root,
            definitions,
            materials,
            next_id: used,
        }
    }

    /// The id the next new entity or definition gets.
    pub(crate) fn next_id(&self) -> u64 {
        self.next_id
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

    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.materials.get(&id)
    }

    /// Every material, by id.
    pub fn materials(&self) -> impl Iterator<Item = (MaterialId, &Material)> {
        self.materials.iter().map(|(k, v)| (*k, v))
    }

    /// A material id for a new material, taken from the entity id counter.
    pub(crate) fn alloc_material(&mut self) -> MaterialId {
        let id = MaterialId(self.next_id);
        self.next_id += 1;
        id
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

    /// Removes a definition and returns the step that removed it.
    pub(crate) fn remove_definition(&mut self, id: DefinitionId) -> Step {
        let step = Step::RemoveDefinition(id, self.definitions[&id].clone());
        self.apply_step(&step);
        step
    }

    /// Follows a path of instances from the root, each one in the mesh of the one before.
    /// Returns the mesh the last one places and the transform from that mesh's coordinates to
    /// the root's (the root itself for an empty path). None if a step is not where it should
    /// be.
    pub fn resolve(&self, path: &[InstanceId]) -> Option<(MeshId, DAffine3)> {
        let mut mesh = MeshId::Root;
        let mut transform = DAffine3::IDENTITY;
        for id in path {
            let i = self.mesh(mesh)?.instance(*id)?;
            transform *= i.transform;
            mesh = MeshId::Definition(i.definition);
        }
        Some((mesh, transform))
    }

    /// The box around a mesh's vertices and instances, in the mesh's coordinates. None when
    /// the mesh is empty or missing.
    pub fn bounds(&self, mesh: MeshId) -> Option<[DVec3; 2]> {
        let m = self.mesh(mesh)?;
        let mut points: Vec<DVec3> = m.vertices().map(|(_, v)| v.pos).collect();
        for (_, i) in m.instances() {
            points.extend(self.instance_bounds(i).into_iter().flatten());
        }
        enclose(&points)
    }

    /// The box around an instance's geometry, in the coordinates of the mesh holding it. None
    /// when its definition is empty or missing.
    pub fn instance_bounds(&self, instance: &Instance) -> Option<[DVec3; 2]> {
        let [lo, hi] = self.bounds(MeshId::Definition(instance.definition))?;
        let corners: Vec<DVec3> = (0..8)
            .map(|k| {
                let corner = DVec3::new(
                    if k & 1 == 0 { lo.x } else { hi.x },
                    if k & 2 == 0 { lo.y } else { hi.y },
                    if k & 4 == 0 { lo.z } else { hi.z },
                );
                instance.transform.transform_point3(corner)
            })
            .collect();
        enclose(&corners)
    }

    /// The anchors of the guides inside an instance's definition, nested definitions
    /// included, in the coordinates of the mesh holding the instance. Bounds leave guides
    /// out; this lets callers check them.
    pub fn instance_guide_anchors(&self, instance: &Instance) -> Vec<DVec3> {
        let mut out = Vec::new();
        if let Some(m) = self.mesh(MeshId::Definition(instance.definition)) {
            out.extend(
                m.guides()
                    .map(|(_, g)| instance.transform.transform_point3(g.anchor())),
            );
            for (_, i) in m.instances() {
                let placed = Instance::new(i.definition, instance.transform * i.transform);
                out.extend(self.instance_guide_anchors(&placed));
            }
        }
        out
    }

    /// Renames a definition. An unchanged name changes nothing.
    pub fn rename_definition(
        &mut self,
        id: DefinitionId,
        name: &str,
    ) -> Result<SceneChange, KernelError> {
        let d = self
            .definitions
            .get(&id)
            .ok_or(KernelError::DefinitionNotFound)?;
        if d.name == name {
            return Ok(SceneChange::default());
        }
        let step = Step::RenameDefinition(id, d.name.clone(), name.to_string());
        self.apply_step(&step);
        Ok(step.into())
    }

    /// Removes every definition that no instance places, including ones only unused
    /// definitions placed.
    pub fn purge(&mut self) -> SceneChange {
        let mut steps = Vec::new();
        loop {
            let unused = self
                .definitions()
                .find(|(id, _)| self.instance_count(*id) == 0)
                .map(|(id, _)| id);
            let Some(id) = unused else { break };
            steps.push(self.remove_definition(id));
        }
        SceneChange { steps }
    }

    /// Applies a change (e.g. an inverse for undo).
    pub fn apply(&mut self, change: &SceneChange) {
        for step in &change.steps {
            self.apply_step(step);
        }
    }

    pub(crate) fn apply_step(&mut self, step: &Step) {
        match step {
            Step::Edit(mesh, c) => {
                let m = self.mesh_mut(*mesh).expect("change to a missing mesh");
                m.apply(c);
                let next = m.next_id();
                self.next_id = self.next_id.max(next);
            }
            Step::AddDefinition(id, d) => {
                self.next_id = self
                    .next_id
                    .max(id.0.saturating_add(1))
                    .max(d.mesh.next_id());
                self.definitions.insert(*id, d.clone());
            }
            Step::RemoveDefinition(id, _) => {
                self.definitions.remove(id);
            }
            Step::RenameDefinition(id, _, to) => {
                if let Some(d) = self.definitions.get_mut(id) {
                    d.name = to.clone();
                }
            }
            Step::AddMaterial(id, m) => {
                self.next_id = self.next_id.max(id.0.saturating_add(1));
                self.materials.insert(*id, m.clone());
            }
            Step::RemoveMaterial(id, _) => {
                self.materials.remove(id);
            }
            Step::ChangeMaterial(id, _, to) => {
                if let Some(m) = self.materials.get_mut(id) {
                    *m = to.clone();
                }
            }
        }
    }

    /// Checks every mesh, that instances place existing definitions, that no definition places
    /// itself (directly or deeper), that no number is the id of two things (entities in any
    /// mesh, curves, definitions and materials all take ids from one counter), that every
    /// material painted exists, and that material names are valid and distinct.
    pub fn validate(&self) -> Result<(), String> {
        let mut names = BTreeSet::new();
        for (id, m) in &self.materials {
            crate::material::check_name(&m.name).map_err(|e| format!("material {id}: {e}"))?;
            if !names.insert(m.name.as_str()) {
                return Err(format!(
                    "material {id}: two materials are named {:?}",
                    m.name
                ));
            }
        }
        let missing = |m: &Option<MaterialId>| m.is_some_and(|m| !self.materials.contains_key(&m));
        // Each id with the prefix of what holds it ("v", "e", "f", "i", "g", "c", "d", "m").
        let mut owner: BTreeMap<u64, char> = BTreeMap::new();
        let mut claim = |id: u64, prefix: char| match owner.insert(id, prefix) {
            None => Ok(()),
            Some(p) if p == prefix => Err(format!("{prefix}{id} is used in two meshes")),
            Some(p) => Err(format!("{p}{id} and {prefix}{id} share an id")),
        };
        for id in self.definitions.keys() {
            claim(id.0, 'd')?;
        }
        for id in self.materials.keys() {
            claim(id.0, 'm')?;
        }
        for (mesh, m) in self.meshes() {
            m.validate().map_err(|e| format!("{mesh:?}: {e}"))?;
            for (id, i) in m.instances() {
                if !self.definitions.contains_key(&i.definition) {
                    return Err(format!("instance {id:?} places a missing definition"));
                }
                if missing(&i.material) {
                    return Err(format!(
                        "instance {id:?} is painted with a missing material"
                    ));
                }
            }
            for (id, f) in m.faces() {
                if missing(&f.front) || missing(&f.back) {
                    return Err(format!("face {id:?} is painted with a missing material"));
                }
            }
            for id in entity_ids(m) {
                let (prefix, n) = match id {
                    EntityId::Vertex(v) => ('v', v.0),
                    EntityId::Edge(e) => ('e', e.0),
                    EntityId::Face(f) => ('f', f.0),
                    EntityId::Instance(i) => ('i', i.0),
                    EntityId::Guide(g) => ('g', g.0),
                };
                claim(n, prefix)?;
            }
            let curves: BTreeSet<CurveId> = m.edges().filter_map(|(_, e)| e.curve).collect();
            for c in curves {
                claim(c.0, 'c')?;
            }
        }
        if let Some(id) = self.cycle() {
            return Err(format!("definition {id} places itself"));
        }
        Ok(())
    }

    /// A definition that places itself (directly or deeper), if any: one depth-first search
    /// over the whole definition graph, finding an edge back to a definition still being
    /// searched.
    fn cycle(&self) -> Option<DefinitionId> {
        #[derive(Clone, Copy, PartialEq)]
        enum State {
            Searching,
            Done,
        }
        let mut state: BTreeMap<DefinitionId, State> = BTreeMap::new();
        for start in self.definitions.keys() {
            if state.contains_key(start) {
                continue;
            }
            state.insert(*start, State::Searching);
            let children = |d: &DefinitionId| {
                self.definitions[d]
                    .mesh
                    .instances()
                    .map(|(_, i)| i.definition)
                    .collect::<Vec<_>>()
                    .into_iter()
            };
            let mut stack = vec![(*start, children(start))];
            while let Some((d, next)) = stack.last_mut() {
                match next.next() {
                    Some(child) => match state.get(&child) {
                        Some(State::Searching) => return Some(child),
                        Some(State::Done) => {}
                        None => {
                            state.insert(child, State::Searching);
                            let grand = children(&child);
                            stack.push((child, grand));
                        }
                    },
                    None => {
                        state.insert(*d, State::Done);
                        stack.pop();
                    }
                }
            }
        }
        None
    }
}

/// The smallest box holding the points; None when there are none.
fn enclose(points: &[DVec3]) -> Option<[DVec3; 2]> {
    let first = *points.first()?;
    Some(
        points
            .iter()
            .fold([first, first], |[lo, hi], p| [lo.min(*p), hi.max(*p)]),
    )
}

fn entity_ids(m: &Model) -> impl Iterator<Item = EntityId> + '_ {
    m.vertices()
        .map(|(id, _)| EntityId::Vertex(id))
        .chain(m.edges().map(|(id, _)| EntityId::Edge(id)))
        .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
        .chain(m.instances().map(|(id, _)| EntityId::Instance(id)))
        .chain(m.guides().map(|(id, _)| EntityId::Guide(id)))
}
