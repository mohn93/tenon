//! Section planes: a plane per entity that cuts away the part of the model on one side of it.
//! A mesh holds any number of planes and at most one is active; the cut itself is made by
//! whoever draws or picks (see `section_loops` for the cut outline). Planes bound no faces
//! and are exported only to model files.
//!
//! A plane keeps the half-space `(p - point) . normal >= 0`. It is moved by the affine
//! maps of the edits (the normal by the inverse transpose, so it stays perpendicular to the
//! plane under a stretch), flipped by negating the normal, and copied inactive.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec3};
use serde::{Deserialize, Serialize};

use crate::edit::KernelError;
use crate::ids::{SectionId, TagId};
use crate::model::{ChangeSet, Model};

/// How far from 1 a section plane's normal length may be.
const NORMAL_TOL: f64 = 1e-9;

/// A section plane: the plane through `point` with the unit vector `normal`. The side the
/// normal points to is kept.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SectionPlane {
    pub point: DVec3,
    pub normal: DVec3,
    /// The tag the plane is on; None is Untagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<TagId>,
}

impl SectionPlane {
    /// An Untagged plane through `point` with `normal` (checked when it is added).
    pub fn new(point: DVec3, normal: DVec3) -> SectionPlane {
        SectionPlane {
            point,
            normal,
            tag: None,
        }
    }

    /// The tag the plane is on; None is Untagged.
    pub fn tag(&self) -> Option<TagId> {
        self.tag
    }

    /// The same plane on `tag`.
    pub fn with_tag(mut self, tag: Option<TagId>) -> SectionPlane {
        self.tag = tag;
        self
    }

    /// The plane keeping the other side.
    pub fn flipped(&self) -> SectionPlane {
        SectionPlane {
            normal: -self.normal,
            ..*self
        }
    }

    /// The plane moved by the affine map `t`, on the same tag. The kept side stays the kept
    /// side of the same geometry, also under a stretch or a mirror.
    pub fn transformed(&self, t: DAffine3) -> SectionPlane {
        let n = t.matrix3.inverse().transpose() * self.normal;
        let normal = if n.is_finite() {
            n.try_normalize().unwrap_or(self.normal)
        } else {
            self.normal
        };
        SectionPlane {
            point: t.transform_point3(self.point),
            normal,
            tag: self.tag,
        }
    }

    /// Whether the plane keeps `p` (on it or on the side the normal points to).
    pub fn keeps(&self, p: DVec3) -> bool {
        (p - self.point).dot(self.normal) >= -1e-9
    }

    /// Fails, saying why, when a number is not finite or the normal is not a unit vector.
    pub fn check(&self) -> Result<(), String> {
        if !self.point.is_finite() || !self.normal.is_finite() {
            return Err("a section plane needs finite numbers".into());
        }
        if (self.normal.length() - 1.0).abs() > NORMAL_TOL {
            return Err("a section plane's normal must be a unit vector".into());
        }
        Ok(())
    }
}

impl Model {
    pub fn section(&self, id: SectionId) -> Option<&SectionPlane> {
        self.sections.get(&id)
    }

    pub fn sections(&self) -> impl Iterator<Item = (SectionId, &SectionPlane)> {
        self.sections.iter().map(|(k, v)| (*k, v))
    }

    /// The plane cutting this mesh, if any.
    pub fn active_section(&self) -> Option<SectionId> {
        self.active_section
    }

    /// Adds a section plane as one ChangeSet, and with `activate` makes it the active one.
    /// A normal that is not a unit vector, or a number that is not finite, is `Section`.
    pub fn add_section(
        &mut self,
        plane: SectionPlane,
        activate: bool,
    ) -> Result<ChangeSet, KernelError> {
        plane.check().map_err(KernelError::Section)?;
        let id = self.insert_section(plane);
        if activate {
            self.set_active(Some(id));
        }
        Ok(self.take_changes())
    }

    /// Makes `id` the active plane (None: no plane cuts), as one ChangeSet. An unknown plane
    /// is `Section`. Already so changes nothing.
    pub fn set_active_section(&mut self, id: Option<SectionId>) -> Result<ChangeSet, KernelError> {
        if id.is_some_and(|id| !self.sections.contains_key(&id)) {
            return Err(unknown());
        }
        self.set_active(id);
        Ok(self.take_changes())
    }

    /// Negates a plane's normal, keeping its point and whether it is active, as one ChangeSet.
    /// An unknown plane is `Section`.
    pub fn flip_section(&mut self, id: SectionId) -> Result<ChangeSet, KernelError> {
        let plane = *self.sections.get(&id).ok_or_else(unknown)?;
        self.replace_section(id, plane.flipped());
        Ok(self.take_changes())
    }

    /// Removes planes as one ChangeSet; erasing the active plane leaves none active in the
    /// same change. Unknown ids are skipped.
    pub fn erase_sections(&mut self, ids: &[SectionId]) -> ChangeSet {
        for id in ids.iter().collect::<BTreeSet<_>>() {
            if self.section(*id).is_some() {
                self.remove_section(*id);
            }
        }
        self.take_changes()
    }

    /// Moves planes by the affine map `t` as one ChangeSet (see `SectionPlane::transformed`),
    /// keeping their ids and whether they are active (each is reported as modified). Unknown
    /// ids are skipped, and so is a plane the map would take to a number that is not finite.
    pub fn transform_sections(&mut self, ids: &[SectionId], t: DAffine3) -> ChangeSet {
        for id in ids.iter().collect::<BTreeSet<_>>() {
            if let Some(p) = self.section(*id).copied() {
                let moved = p.transformed(t);
                if moved.check().is_ok() {
                    self.replace_section(*id, moved);
                }
            }
        }
        self.take_changes()
    }

    /// Adds an inactive copy of each plane per affine map in `places`, with new ids, as one
    /// ChangeSet. Unknown ids are skipped.
    pub fn copy_sections(&mut self, ids: &[SectionId], places: &[DAffine3]) -> ChangeSet {
        let originals: Vec<SectionPlane> = ids
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| self.section(*id).copied())
            .collect();
        for t in places {
            for p in &originals {
                let copy = p.transformed(*t);
                if copy.check().is_ok() {
                    self.insert_section(copy);
                }
            }
        }
        self.take_changes()
    }

    /// Every plane of the mesh.
    pub(crate) fn all_sections(&self) -> Vec<SectionId> {
        self.sections.keys().copied().collect()
    }

    /// The planes `ids` names (unknown ids skipped), placed by `t`, to be put into a mesh by
    /// `land_sections`. A plane the map takes to a number that is not finite is left out.
    pub(crate) fn lift_sections(
        &self,
        ids: &[SectionId],
        t: DAffine3,
    ) -> Vec<(SectionId, SectionPlane)> {
        ids.iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| Some((*id, self.section(*id)?.transformed(t))))
            .filter(|(_, p)| p.check().is_ok())
            .collect()
    }

    /// Puts lifted planes into this mesh, inactive, under their old ids with `keep_ids`, else
    /// under new ones. Returns the ids they got.
    pub(crate) fn land_sections(
        &mut self,
        planes: Vec<(SectionId, SectionPlane)>,
        keep_ids: bool,
    ) -> Vec<SectionId> {
        planes
            .into_iter()
            .map(|(id, p)| {
                if keep_ids {
                    self.insert_section_as(id, p);
                    id
                } else {
                    self.insert_section(p)
                }
            })
            .collect()
    }

    /// Checks that every plane is valid and the active plane is one of them.
    pub(crate) fn validate_sections(&self) -> Result<(), String> {
        for (id, p) in &self.sections {
            p.check().map_err(|e| format!("{id:?}: {e}"))?;
        }
        if let Some(id) = self.active_section
            && !self.sections.contains_key(&id)
        {
            return Err(format!("the active section plane {id:?} does not exist"));
        }
        Ok(())
    }
}

fn unknown() -> KernelError {
    KernelError::Section("no such section plane".into())
}
