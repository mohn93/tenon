//! Entity ids that live outside the open context: where they are, for error messages.

use std::collections::{BTreeSet, VecDeque};

use kernel::{EntityId, InstanceId, MeshId, Model};

use super::Document;
use crate::batch::OpenContext;
use crate::command::{CommandError, ErrorKind};

/// Whether `m` holds the entity.
pub(super) fn holds(m: &Model, id: EntityId) -> bool {
    match id {
        EntityId::Vertex(v) => m.vertex(v).is_some(),
        EntityId::Edge(e) => m.edge(e).is_some(),
        EntityId::Face(f) => m.face(f).is_some(),
        EntityId::Instance(i) => m.instance(i).is_some(),
        EntityId::Guide(g) => m.guide(g).is_some(),
        EntityId::Dimension(d) => m.dimension(d).is_some(),
        EntityId::Note(n) => m.note(n).is_some(),
        EntityId::Section(p) => m.section(p).is_some(),
    }
}

impl Document {
    /// The instances to open, from the root, to reach a mesh that holds `id` (any one path
    /// when its definition has several instances). None when no mesh reachable from the root
    /// holds it. Searches only when called, so keep it to error paths.
    pub(super) fn path_to_entity(&self, id: EntityId) -> Option<Vec<InstanceId>> {
        let target = self
            .scene
            .meshes()
            .find(|(_, m)| holds(m, id))
            .map(|(mesh, _)| mesh)?;
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([(MeshId::Root, Vec::new())]);
        while let Some((mesh, path)) = queue.pop_front() {
            if mesh == target {
                return Some(path);
            }
            let Some(model) = self.scene.mesh(mesh) else {
                continue;
            };
            for (instance, data) in model.instances() {
                let next = MeshId::Definition(data.definition);
                if seen.insert(data.definition) {
                    let mut p = path.clone();
                    p.push(instance);
                    queue.push_back((next, p));
                }
            }
        }
        None
    }

    /// "<id> is inside i1 > i2; open that path first (open_path)" when `id` is in another
    /// mesh reachable through instances, otherwise None.
    pub(super) fn elsewhere_note(&self, id: EntityId) -> Option<String> {
        let path = self.path_to_entity(id)?;
        if path.is_empty() {
            return None;
        }
        let path: Vec<String> = path
            .into_iter()
            .map(|i| EntityId::Instance(i).to_string())
            .collect();
        Some(format!(
            "{id} is inside {}; open that path first (open_path)",
            path.join(" > ")
        ))
    }

    /// The `not_found` error for ids the open context lacks. `plain` is the message when none
    /// of them exists elsewhere in the model; otherwise the message names, for each that does,
    /// the path to open.
    pub(super) fn missing_error(&self, missing: Vec<EntityId>, plain: String) -> CommandError {
        let notes: Vec<String> = missing
            .iter()
            .filter_map(|id| self.elsewhere_note(*id))
            .collect();
        let message = if notes.is_empty() {
            plain
        } else {
            notes.join(". ")
        };
        CommandError::new(ErrorKind::NotFound, message).with_ids(missing)
    }

    /// `missing_error` with the usual message.
    pub(super) fn no_such_entities(&self, missing: Vec<EntityId>) -> CommandError {
        self.missing_error(missing, "no such entities".to_string())
    }
}

impl OpenContext for Document {
    fn holds(&self, id: EntityId) -> bool {
        self.exists(id)
    }

    fn missing_text(&self, id: EntityId) -> String {
        self.elsewhere_note(id)
            .unwrap_or_else(|| format!("{id} is not in the open context"))
    }
}
