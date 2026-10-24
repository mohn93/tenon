//! Section planes and how they are shown.

use glam::DVec3;
use kernel::{EntityId, SceneChange, SectionId, SectionPlane};

use super::{Document, check_coordinates, unit_vector};
use crate::command::{CommandError, ErrorKind};
use crate::query::SectionPlaneInfo;

/// How section planes are shown. A view setting like the lighting: saved with the camera, not
/// an undo step, not part of the model's revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionDisplay {
    /// The plane entities are drawn.
    pub planes: bool,
    /// The active planes' cuts are applied.
    pub cuts: bool,
    /// Closed cuts are filled.
    pub fill: bool,
}

impl Default for SectionDisplay {
    fn default() -> Self {
        SectionDisplay {
            planes: true,
            cuts: true,
            fill: true,
        }
    }
}

fn invalid(message: &str) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

impl Document {
    pub(super) fn set_section_display_changes(
        &mut self,
        planes: Option<bool>,
        cuts: Option<bool>,
        fill: Option<bool>,
    ) {
        let s = &mut self.section_display;
        s.planes = planes.unwrap_or(s.planes);
        s.cuts = cuts.unwrap_or(s.cuts);
        s.fill = fill.unwrap_or(s.fill);
    }

    pub(super) fn add_section_plane_changes(
        &mut self,
        point: [f64; 3],
        normal: [f64; 3],
        activate: bool,
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[point])?;
        let Some(normal) = unit_vector(DVec3::from(normal)) else {
            return Err(invalid("normal must be a finite, non-zero vector"));
        };
        self.edit_mesh(|m| m.add_section(SectionPlane::new(DVec3::from(point), normal), activate))
            .map_err(|e| invalid(&e.to_string()))
    }

    /// The plane `id` names, which must be in the open context.
    fn section_of(&self, id: EntityId) -> Result<SectionId, CommandError> {
        match id {
            EntityId::Section(s) if self.model().section(s).is_some() => Ok(s),
            EntityId::Section(_) => Err(if self.elsewhere_note(id).is_some() {
                self.no_such_entities(vec![id])
            } else {
                invalid(&format!("{id} is not a section plane of the open context"))
                    .with_ids(vec![id])
            }),
            _ => Err(invalid(&format!("{id} is not a section plane")).with_ids(vec![id])),
        }
    }

    pub(super) fn flip_section_plane_changes(
        &mut self,
        plane: EntityId,
    ) -> Result<SceneChange, CommandError> {
        let s = self.section_of(plane)?;
        self.edit_mesh(|m| m.flip_section(s))
            .map_err(|e| invalid(&e.to_string()).with_ids(vec![plane]))
    }

    pub(super) fn activate_section_plane_changes(
        &mut self,
        plane: EntityId,
    ) -> Result<SceneChange, CommandError> {
        let s = self.section_of(plane)?;
        self.edit_mesh(|m| m.set_active_section(Some(s)))
            .map_err(|e| invalid(&e.to_string()).with_ids(vec![plane]))
    }

    pub(super) fn deactivate_section_changes(&mut self) -> SceneChange {
        self.edit_mesh(|m| m.set_active_section(None))
            .expect("clearing the active plane cannot fail")
    }

    pub(super) fn section_plane_info(&self, id: SectionId) -> SectionPlaneInfo {
        let m = self.model();
        let p = m.section(id).expect("checked");
        SectionPlaneInfo {
            id: EntityId::Section(id),
            point: p.point.to_array(),
            normal: p.normal.to_array(),
            active: m.active_section() == Some(id),
            tag: p.tag(),
        }
    }
}
