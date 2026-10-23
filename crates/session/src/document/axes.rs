//! The model axes and the axes of a group or component.

use glam::DVec3;
use kernel::{Axes, EntityId, Instance, MeshId, SceneChange};

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
                self.check_reorigin(definition, axes.frame())?;
                self.scene
                    .reorigin_definition(definition, axes.frame())
                    .map_err(|e| self.kernel_error(e, &[]))
            }
        }
    }

    /// Refuses a new frame for `definition` that would put something beyond the coordinate
    /// limit, as opening a saved file would: the definition's own content in the new frame (all
    /// eight corners of nested instances, their guides, empty instances, section planes and
    /// annotations) and where the frame lands every instance that places the definition.
    fn check_reorigin(
        &self,
        definition: kernel::DefinitionId,
        frame: glam::DAffine3,
    ) -> Result<(), CommandError> {
        let inverse = frame.inverse();
        let m = self.model();
        let at = |p: DVec3| p.to_array();
        let mut points: Vec<[f64; 3]> = m
            .vertices()
            .map(|(_, v)| at(inverse.transform_point3(v.pos)))
            .collect();
        points.extend(
            m.guides()
                .map(|(_, g)| at(inverse.transform_point3(g.anchor()))),
        );
        points.extend(
            m.sections()
                .map(|(_, p)| at(inverse.transform_point3(p.point))),
        );
        points.extend(m.annotation_points(inverse).into_iter().map(at));
        for (_, i) in m.instances() {
            let placed = Instance::new(i.definition, inverse * i.transform);
            match self.scene.instance_bounds(&placed) {
                // The box around all eight corners, so its corners cover them.
                Some(b) => points.extend(b.map(at)),
                None => points.push(at(placed.transform.translation)),
            }
            points.extend(
                self.scene
                    .instance_guide_anchors(&placed)
                    .into_iter()
                    .map(at),
            );
        }
        check_coordinates(&points)?;
        for (_, mesh) in self.scene.meshes() {
            for (id, i) in mesh.instances().filter(|(_, i)| i.definition == definition) {
                let to = (i.transform * frame).translation;
                if check_coordinates(&[at(to)]).is_err() {
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "the new axes would place {} beyond +/-{} m",
                            EntityId::Instance(id),
                            super::MAX_COORDINATE
                        ),
                    )
                    .with_ids(vec![EntityId::Instance(id)]));
                }
            }
        }
        Ok(())
    }

    /// Puts the model axes back to the world axes; inside a context there is nothing to reset.
    pub(super) fn reset_axes_changes(&mut self) -> SceneChange {
        match self.active() {
            MeshId::Root => self.scene.set_axes(Axes::WORLD),
            MeshId::Definition(_) => SceneChange::default(),
        }
    }
}
