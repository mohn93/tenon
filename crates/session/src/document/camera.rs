//! Camera commands: placing, orbiting, panning, zooming, zoom extents, standard views and
//! projection. None is an undo step or changes the revision.

use glam::DVec3;
use kernel::MeshId;

use super::{Document, check_coordinates, unit_vector};
use crate::camera::{Camera, Projection, UP, VERTICAL_TOLERANCE, ZOOM_STEP};
use crate::command::{Command, CommandError, ErrorKind};

impl Document {
    /// The camera a camera command would leave, after checking it: the eye and target stay
    /// within the coordinate limit. Does not change the document.
    pub(super) fn moved_camera(&self, command: &Command) -> Result<Camera, CommandError> {
        let mut camera = self.camera.clone();
        match *command {
            Command::SetCamera {
                eye,
                target,
                fov_y_deg,
                up,
                projection,
            } => return self.camera_for(eye, target, fov_y_deg, up, projection),
            Command::Orbit { yaw, pitch, about } => {
                finite(&[yaw, pitch], "yaw and pitch")?;
                let pivot = match about {
                    Some(p) => {
                        check_coordinates(&[p])?;
                        DVec3::from(p)
                    }
                    None => camera.target,
                };
                camera.turn(pivot, yaw.to_radians(), -pitch.to_radians());
            }
            Command::Pan { right, up } => {
                finite(&[right, up], "right and up")?;
                let offset = camera.right() * right + camera.up() * up;
                camera.eye += offset;
                camera.target += offset;
            }
            Command::Zoom { factor, about } => {
                if !(factor.is_finite() && factor > 0.0) {
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "factor must be a positive number",
                    ));
                }
                match about {
                    Some(p) => {
                        check_coordinates(&[p])?;
                        // `zoom` takes scroll steps: `ZOOM_STEP^steps` of the distance.
                        camera.zoom(DVec3::from(p), -factor.ln() / ZOOM_STEP.ln());
                    }
                    None => camera.zoom_by(factor),
                }
            }
            Command::ZoomExtents {} => {
                if let Some([lo, hi]) = self.scene.shown_bounds(MeshId::Root, &self.context) {
                    camera.fit(lo, hi);
                }
            }
            Command::StandardView { view } => {
                camera.look_from_in(view, self.drawing_axes_in_world().matrix3)
            }
            Command::SetProjection { projection } => camera.projection = projection,
            _ => unreachable!("not a camera command"),
        }
        check_coordinates(&[camera.eye.to_array(), camera.target.to_array()]).map_err(|_| {
            CommandError::new(
                ErrorKind::InvalidInput,
                "the camera would move beyond the coordinate limit",
            )
        })?;
        Ok(camera)
    }

    /// The current camera moved to `eye`/`target` (and `fov_y_deg`, `projection` if given),
    /// after the checks `SetCamera` applies. A vertical view takes its screen's up from `up`
    /// (default +Y). Does not change the document.
    pub fn camera_for(
        &self,
        eye: [f64; 3],
        target: [f64; 3],
        fov_y_deg: Option<f64>,
        up: Option<[f64; 3]>,
        projection: Option<Projection>,
    ) -> Result<Camera, CommandError> {
        check_coordinates(&[eye, target])?;
        let (eye, target) = (DVec3::from(eye), DVec3::from(target));
        let fov = fov_y_deg.unwrap_or(self.camera.fov_y.to_degrees());
        if eye.distance(target) < 1e-6 {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "eye and target must be distinct points",
            ));
        }
        if !(1.0..=170.0).contains(&fov) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "fov_y_deg must be between 1 and 170",
            ));
        }
        let vertical = (target - eye).normalize().cross(UP).length() < VERTICAL_TOLERANCE;
        let up = if vertical {
            let u = DVec3::from(up.unwrap_or([0.0, 1.0, 0.0]));
            unit_vector(DVec3::new(u.x, u.y, 0.0)).ok_or_else(|| {
                CommandError::new(
                    ErrorKind::InvalidInput,
                    "a view straight down or up needs an up direction that is not vertical",
                )
            })?
        } else {
            UP
        };
        let mut camera = self.camera.clone();
        camera.eye = eye;
        camera.target = target;
        camera.up = up;
        camera.fov_y = fov.to_radians();
        if let Some(projection) = projection {
            camera.projection = projection;
        }
        Ok(camera)
    }
}

fn finite(values: &[f64], what: &str) -> Result<(), CommandError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!("{what} must be finite numbers"),
        ))
    }
}
