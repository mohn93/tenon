//! The model axes and the axes of a group or component.

use glam::DVec3;
use kernel::{Axes, MeshId, SceneChange};

use super::{Document, check_coordinates};
use crate::command::{CommandError, ErrorKind};

impl Document {
    /// Moves the model axes at the root, or the open definition's axes inside a context.
    pub(super) fn set_axes_changes(
        &mut self,
        origin: [f64; 3],
        x_axis: [f64; 3],
        y_axis: [f64; 3],
    ) -> Result<SceneChange, CommandError> {
        check_coordinates(&[origin])?;
        let axes = Axes::new(
            DVec3::from(origin),
            DVec3::from(x_axis),
            DVec3::from(y_axis),
        )
        .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
        match self.active() {
            MeshId::Root => Ok(self.scene.set_axes(axes)),
            MeshId::Definition(definition) => {
                // What opening checks: the geometry must stay within the limit in the new frame.
                let inverse = axes.frame().inverse();
                let m = self.model();
                let mut points: Vec<[f64; 3]> = m
                    .vertices()
                    .map(|(_, v)| inverse.transform_point3(v.pos).to_array())
                    .collect();
                points.extend(
                    m.guides()
                        .map(|(_, g)| inverse.transform_point3(g.anchor()).to_array()),
                );
                for (_, i) in m.instances() {
                    if let Some(b) = self.scene.instance_bounds(i) {
                        points.extend(b.map(|p| inverse.transform_point3(p).to_array()));
                    }
                }
                check_coordinates(&points)?;
                self.scene
                    .reorigin_definition(definition, axes.frame())
                    .map_err(|e| self.kernel_error(e, &[]))
            }
        }
    }

    /// Puts the model axes back to the world axes; inside a context there is nothing to reset.
    pub(super) fn reset_axes_changes(&mut self) -> SceneChange {
        match self.active() {
            MeshId::Root => self.scene.set_axes(Axes::WORLD),
            MeshId::Definition(_) => SceneChange::default(),
        }
    }
}
