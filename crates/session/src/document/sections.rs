//! Section planes and how they are shown.

use glam::DVec3;
use kernel::{EntityId, SceneChange, SectionId, SectionPlane};
use serde::{Deserialize, Serialize};

use super::{Document, check_coordinates, unit_vector};
use crate::command::{CommandError, ErrorKind};
use crate::query::SectionPlaneInfo;

/// The color closed cuts are filled with.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SectionFillColor {
    /// The fixed dark gray (#3a3a3a).
    #[default]
    Dark,
    /// The color of the material most of the cut faces' area shows on their front, darkened by 20%.
    Material,
}

impl SectionFillColor {
    /// How much of the material's color the fill keeps (the rest is dark), so a cut reads as
    /// a cut.
    pub const MATERIAL_SHADE: f32 = 0.8;
}

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
    /// The color of the fill.
    pub fill_color: SectionFillColor,
}

impl Default for SectionDisplay {
    fn default() -> Self {
        SectionDisplay {
            planes: true,
            cuts: true,
            fill: true,
            fill_color: SectionFillColor::Dark,
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
        fill_color: Option<SectionFillColor>,
    ) {
        let s = &mut self.section_display;
        s.planes = planes.unwrap_or(s.planes);
        s.cuts = cuts.unwrap_or(s.cuts);
        s.fill = fill.unwrap_or(s.fill);
        s.fill_color = fill_color.unwrap_or(s.fill_color);
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
            EntityId::Section(_) => Err(match self.elsewhere_note(id) {
                Some(note) => CommandError::new(ErrorKind::NotFound, note).with_ids(vec![id]),
                None => invalid(&format!("{id} is not a section plane of the open context"))
                    .with_ids(vec![id]),
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

    pub(super) fn set_section_cut_changes(
        &mut self,
        ids: &[EntityId],
        cut: bool,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(invalid("ids must list at least one instance"));
        }
        self.parts(ids)?;
        if let Some(bad) = ids.iter().find(|id| !matches!(id, EntityId::Instance(_))) {
            return Err(invalid(&format!("{bad} is not an instance")).with_ids(vec![*bad]));
        }
        let mut changed = Vec::new();
        let change = self
            .edit_mesh(|m| {
                let (changes, c) = m.set_section_cut_entities(ids, cut);
                changed = c;
                Ok(changes)
            })
            .expect("setting a flag cannot fail");
        self.changed_entities = Some(changed);
        Ok(change)
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
