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

/// Most located ids a message spells out.
const MAX_NOTES: usize = 3;

impl Document {
    /// For each id, the note saying where it is when it is in a mesh other than the open
    /// one's: "<id> is inside i1 > i2; open that path first (open_path)", or for the root
    /// "<id> is in the root; go back to it first (open_path with [])". None when it exists
    /// nowhere reachable. Finds each id's mesh once and runs one search from the root for all,
    /// so keep it to error paths.
    pub(super) fn elsewhere_notes(&self, ids: &[EntityId]) -> Vec<Option<String>> {
        let targets: Vec<Option<MeshId>> = ids
            .iter()
            .map(|id| {
                self.scene
                    .meshes()
                    .find(|(_, m)| holds(m, *id))
                    .map(|(mesh, _)| mesh)
            })
            .collect();
        let mut wanted: Vec<MeshId> = targets.iter().flatten().copied().collect();
        wanted.dedup();
        // Any one path to each wanted mesh: breadth first from the root.
        let mut paths: Vec<(MeshId, Vec<InstanceId>)> = Vec::new();
        if !wanted.is_empty() {
            let mut seen = BTreeSet::new();
            let mut queue = VecDeque::from([(MeshId::Root, Vec::new())]);
            while let Some((mesh, path)) = queue.pop_front() {
                if wanted.contains(&mesh) {
                    paths.push((mesh, path.clone()));
                    if paths.len() == wanted.len() {
                        break;
                    }
                }
                let Some(model) = self.scene.mesh(mesh) else {
                    continue;
                };
                for (instance, data) in model.instances() {
                    if seen.insert(data.definition) {
                        let mut p = path.clone();
                        p.push(instance);
                        queue.push_back((MeshId::Definition(data.definition), p));
                    }
                }
            }
        }
        ids.iter()
            .zip(targets)
            .map(|(id, target)| {
                let path = &paths.iter().find(|(m, _)| Some(*m) == target)?.1;
                if path.is_empty() {
                    return Some(format!(
                        "{id} is in the root; go back to it first (open_path with [])"
                    ));
                }
                let path: Vec<String> = path
                    .iter()
                    .map(|i| EntityId::Instance(*i).to_string())
                    .collect();
                Some(format!(
                    "{id} is inside {}; open that path first (open_path)",
                    path.join(" > ")
                ))
            })
            .collect()
    }

    /// `elsewhere_notes` for one id.
    pub(super) fn elsewhere_note(&self, id: EntityId) -> Option<String> {
        self.elsewhere_notes(&[id]).pop().flatten()
    }

    /// The `not_found` error for ids the open context lacks. `plain` is the message when none
    /// of them exists elsewhere in the model; otherwise the message names, for each that does
    /// (the first few), where to go, and lists the ones that exist nowhere.
    pub(super) fn missing_error(&self, missing: Vec<EntityId>, plain: String) -> CommandError {
        let found = self.elsewhere_notes(&missing);
        let notes: Vec<&String> = found.iter().flatten().collect();
        let message = if notes.is_empty() {
            plain
        } else {
            let mut text = notes
                .iter()
                .take(MAX_NOTES)
                .map(|n| n.as_str())
                .collect::<Vec<_>>()
                .join(". ");
            if notes.len() > MAX_NOTES {
                text.push_str(&format!(". and {} more", notes.len() - MAX_NOTES));
            }
            let nowhere: Vec<String> = missing
                .iter()
                .zip(&found)
                .filter(|(_, n)| n.is_none())
                .map(|(id, _)| id.to_string())
                .collect();
            if !nowhere.is_empty() {
                text.push_str(&format!("; no such entities: {}", nowhere.join(", ")));
            }
            text
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
