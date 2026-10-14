//! Soft and hard edges.

use std::collections::BTreeSet;

use kernel::{EdgeId, EntityId, SceneChange};

use super::Document;
use crate::command::{CommandError, ErrorKind};

impl Document {
    /// Softens or hardens the edges in `ids` and around the faces in `ids`. Other kinds of
    /// entity are invalid input, as is an empty list.
    pub(super) fn soften_changes(
        &mut self,
        ids: &[EntityId],
        soft: bool,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge or face",
            ));
        }
        let edges = self.edges_of(ids)?;
        let mut report = None;
        let change = self
            .edit_mesh(|m| {
                let (changes, r) = m.soften_edges(&edges, soft);
                report = Some(r);
                Ok(changes)
            })
            .expect("softening cannot fail");
        self.soften_report = report;
        Ok(change)
    }

    /// Softens the edges in `ids` and around the faces in `ids` (every edge of the open
    /// context when `ids` is empty) whose faces meet at no more than `max_angle_deg`.
    pub(super) fn soften_by_angle_changes(
        &mut self,
        ids: &[EntityId],
        max_angle_deg: f64,
    ) -> Result<SceneChange, CommandError> {
        if !(0.0..=180.0).contains(&max_angle_deg) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("max_angle_deg must be a number from 0 to 180, not {max_angle_deg}"),
            ));
        }
        let edges = if ids.is_empty() {
            self.model().edges().map(|(id, _)| id).collect()
        } else {
            self.edges_of(ids)?
        };
        let mut report = None;
        let change = self
            .edit_mesh(|m| {
                let (changes, r) = m.soften_by_angle(&edges, max_angle_deg.to_radians());
                report = Some(r);
                Ok(changes)
            })
            .expect("softening cannot fail");
        self.soften_report = report;
        Ok(change)
    }

    /// The edges in `ids` and around the faces in `ids`, in id order. Vertices, instances and
    /// guides are invalid input (naming them); unknown ids are `not_found`.
    fn edges_of(&self, ids: &[EntityId]) -> Result<Vec<EdgeId>, CommandError> {
        let others: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| !matches!(id, EntityId::Edge(_) | EntityId::Face(_)))
            .collect();
        if !others.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "only edges and faces can be softened or hardened",
            )
            .with_ids(others));
        }
        let parts = self.parts(ids)?;
        let m = self.model();
        let mut edges: BTreeSet<EdgeId> = parts.edges.into_iter().collect();
        for f in parts.faces {
            edges.extend(m.face_edges(f));
        }
        Ok(edges.into_iter().collect())
    }
}
