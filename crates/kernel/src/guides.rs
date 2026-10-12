//! Guide lines and guide points: construction geometry that tools snap to. Guides never merge
//! with edges or each other and bound no faces.

use std::collections::BTreeSet;

use glam::DAffine3;

use crate::edit::KernelError;
use crate::ids::GuideId;
use crate::model::{ChangeSet, Guide, Model};
use crate::scene::{MeshId, Scene, SceneChange};

impl Model {
    /// Adds a guide as one ChangeSet; a guide line's direction is normalized. Non-finite
    /// numbers and a zero direction are `NotFinite`.
    pub fn add_guide(&mut self, guide: Guide) -> Result<ChangeSet, KernelError> {
        let guide = match guide {
            Guide::Line { point, direction } => Guide::Line {
                point,
                direction: direction.try_normalize().ok_or(KernelError::NotFinite)?,
            },
            g => g,
        };
        if !guide.anchor().is_finite() {
            return Err(KernelError::NotFinite);
        }
        self.insert_guide(guide);
        Ok(self.take_changes())
    }

    /// Removes guides as one ChangeSet; unknown ids are skipped.
    pub fn erase_guides(&mut self, ids: &[GuideId]) -> ChangeSet {
        for id in ids.iter().collect::<BTreeSet<_>>() {
            if self.guide(*id).is_some() {
                self.remove_guide(*id);
            }
        }
        self.take_changes()
    }

    /// Moves guides by the rigid motion `t` as one ChangeSet, keeping their ids (each is
    /// reported as modified); unknown ids are skipped.
    pub fn transform_guides(&mut self, ids: &[GuideId], t: DAffine3) -> ChangeSet {
        for id in ids.iter().collect::<BTreeSet<_>>() {
            if self.guide(*id).is_some() {
                let g = self.remove_guide(*id);
                self.insert_guide_as(*id, g.transformed(t));
            }
        }
        self.take_changes()
    }

    /// Adds a copy of each guide per rigid motion in `places`, with new ids, as one ChangeSet;
    /// unknown ids are skipped.
    pub fn copy_guides(&mut self, ids: &[GuideId], places: &[DAffine3]) -> ChangeSet {
        let originals: Vec<Guide> = ids
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| self.guide(*id).copied())
            .collect();
        for t in places {
            for g in &originals {
                self.insert_guide(g.transformed(*t));
            }
        }
        self.take_changes()
    }
}

impl Scene {
    /// Moves guides from mesh `from` to mesh `to`, keeping their ids and positions (for a
    /// group made where they were drawn). Unknown ids are skipped.
    pub fn move_guides(&mut self, from: MeshId, to: MeshId, ids: &[GuideId]) -> SceneChange {
        let mut moved = Vec::new();
        let left = self
            .edit(from, |m| {
                for id in ids.iter().collect::<BTreeSet<_>>() {
                    if m.guide(*id).is_some() {
                        moved.push((*id, m.remove_guide(*id)));
                    }
                }
                Ok(m.take_changes())
            })
            .expect("removing guides cannot fail");
        let placed = self
            .edit(to, |m| {
                for (id, g) in moved {
                    m.insert_guide_as(id, g);
                }
                Ok(m.take_changes())
            })
            .expect("adding guides cannot fail");
        SceneChange::join([left, placed])
    }
}
