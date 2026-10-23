//! Reversing faces and orienting a surface.

use kernel::{EntityId, FaceId, SceneChange};

use super::Document;
use crate::command::{CommandError, ErrorKind, OrientOutcome};

impl Document {
    /// Turns the faces in `ids` over. Ids of other kinds are ignored; a face the open context
    /// does not hold is `not_found`.
    pub(super) fn reverse_faces_changes(
        &mut self,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        let faces: Vec<FaceId> = ids
            .iter()
            .filter_map(|id| match id {
                EntityId::Face(f) => Some(*f),
                _ => None,
            })
            .collect();
        self.check_faces(&faces)?;
        let mut changed = Vec::new();
        let change = self
            .edit_mesh(|m| {
                let (changes, turned) = m.reverse_faces(&faces);
                changed = turned;
                Ok(changes)
            })
            .expect("reversing cannot fail");
        self.hidden_changed = Some(changed.into_iter().map(EntityId::Face).collect());
        Ok(change)
    }

    /// Orients the faces connected to face `id` (within `ids` when it is not empty). Anything
    /// but a face is invalid input; a face the open context does not hold is `not_found`.
    pub(super) fn orient_faces_changes(
        &mut self,
        id: EntityId,
        ids: &[EntityId],
    ) -> Result<SceneChange, CommandError> {
        let mut faces = Vec::new();
        let mut others = Vec::new();
        for &e in std::iter::once(&id).chain(ids) {
            match e {
                EntityId::Face(f) => faces.push(f),
                other => others.push(other),
            }
        }
        if !others.is_empty() {
            return Err(
                CommandError::new(ErrorKind::InvalidInput, "only faces can be oriented")
                    .with_ids(others),
            );
        }
        self.check_faces(&faces)?;
        let start = faces[0];
        let within = &faces[1..];
        let mut report = None;
        let change = self
            .edit_mesh(|m| {
                let (changes, r) = m.orient_faces(start, within);
                report = Some(r);
                Ok(changes)
            })
            .expect("orienting cannot fail");
        let r = report.expect("the edit ran");
        self.orient_report = Some(OrientOutcome {
            flipped: r.flipped.into_iter().map(EntityId::Face).collect(),
            conflicts: r.conflicts,
        });
        Ok(change)
    }

    /// `not_found` naming the faces the open context does not hold.
    fn check_faces(&self, faces: &[FaceId]) -> Result<(), CommandError> {
        let m = self.model();
        let missing: Vec<EntityId> = faces
            .iter()
            .filter(|f| m.face(**f).is_none())
            .map(|f| EntityId::Face(*f))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(CommandError::new(ErrorKind::NotFound, "no such faces").with_ids(missing))
        }
    }
}
