//! Hiding and showing edges, faces, instances and guides. A hidden entity is not drawn or
//! picked; hiding a face leaves its edges, and hiding an edge leaves the faces it bounds.

use std::collections::BTreeSet;

use crate::ids::EntityId;
use crate::model::{ChangeSet, Model};

impl Model {
    /// Hides (`hidden`) or shows each of `ids` as one ChangeSet, with the ids whose flag
    /// changed in id order. Vertices, annotations, section planes and unknown ids are skipped,
    /// and so are entities already as asked.
    pub fn hide_entities(&mut self, ids: &[EntityId], hidden: bool) -> (ChangeSet, Vec<EntityId>) {
        let mut changed = Vec::new();
        for &id in ids.iter().collect::<BTreeSet<_>>() {
            if self.hidden_of(id).is_some_and(|now| now != hidden) {
                self.set_hidden(id, hidden);
                changed.push(id);
            }
        }
        (self.take_changes(), changed)
    }

    /// Sets whether section planes outside each instance of `ids` cut it, as one ChangeSet,
    /// with the ids whose flag changed in id order. Other kinds, unknown ids and instances
    /// already as asked are skipped.
    pub fn set_section_cut_entities(
        &mut self,
        ids: &[EntityId],
        cut: bool,
    ) -> (ChangeSet, Vec<EntityId>) {
        let mut changed = Vec::new();
        for &id in ids.iter().collect::<BTreeSet<_>>() {
            if let EntityId::Instance(i) = id
                && self.section_cut_of(i).is_some_and(|now| now != cut)
            {
                self.set_section_cut(i, cut);
                changed.push(id);
            }
        }
        (self.take_changes(), changed)
    }

    /// Whether this mesh has a hidden edge, face, instance or guide (stops at the first).
    pub fn any_hidden(&self) -> bool {
        self.edges().any(|(_, e)| e.hidden)
            || self.faces().any(|(_, f)| f.hidden)
            || self.instances().any(|(_, i)| i.hidden)
            || self.guides().any(|(_, g)| g.hidden())
    }

    /// The hidden edges, faces, instances and guides of this mesh (not those inside its
    /// instances), in kind then id order.
    pub fn hidden_entities(&self) -> Vec<EntityId> {
        let edges = self.edges().filter(|(_, e)| e.hidden);
        let faces = self.faces().filter(|(_, f)| f.hidden);
        let instances = self.instances().filter(|(_, i)| i.hidden);
        let guides = self.guides().filter(|(_, g)| g.hidden());
        edges
            .map(|(id, _)| EntityId::Edge(id))
            .chain(faces.map(|(id, _)| EntityId::Face(id)))
            .chain(instances.map(|(id, _)| EntityId::Instance(id)))
            .chain(guides.map(|(id, _)| EntityId::Guide(id)))
            .collect()
    }
}
