//! Revolve: a face turned about an axis into a solid.

use glam::DVec3;
use kernel::{EntityId, SceneChange};

use super::{Document, check_coordinates};
use crate::command::{CommandError, ErrorKind};

impl Document {
    /// Revolves face `face` about the line through `center` along `axis` (`angle` degrees,
    /// default 360, in `segments` per full turn, default 24). Refusals change nothing.
    pub(super) fn revolve_changes(
        &mut self,
        face: EntityId,
        (center, axis): ([f64; 3], [f64; 3]),
        (angle, segments): (Option<f64>, Option<u32>),
    ) -> Result<SceneChange, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        if !self.exists(face) {
            return Err(self.no_such_entities(vec![face]));
        }
        let EntityId::Face(profile) = face else {
            return Err(invalid("face must be a face").with_ids(vec![face]));
        };
        check_coordinates(&[center])?;
        if axis.iter().any(|c| !c.is_finite()) || DVec3::from(axis).length_squared() == 0.0 {
            return Err(invalid("axis must be a finite, non-zero vector"));
        }
        let plan = self
            .model()
            .plan_revolve(
                profile,
                DVec3::from(center),
                DVec3::from(axis),
                angle.unwrap_or(360.0),
                segments.unwrap_or(kernel::DEFAULT_SEGMENTS),
            )
            .map_err(|e| invalid(&e.to_string()).with_ids(vec![face]))?;
        let points: Vec<[f64; 3]> = plan.points().iter().map(|p| p.to_array()).collect();
        check_coordinates(&points)?;
        Ok(self
            .edit_mesh(|m| Ok(m.revolve(&plan)))
            .expect("drawing a planned revolve cannot fail"))
    }
}
