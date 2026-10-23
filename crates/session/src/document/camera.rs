//! Camera commands: placing, orbiting, panning, zooming, zoom extents, standard views,
//! projection, looking around and positioning the camera. None is an undo step or changes the
//! revision.

use glam::{DQuat, DVec3};
use kernel::MeshId;

use super::{Document, check_coordinates, unit_vector};
use crate::camera::{Camera, Projection, UP, VERTICAL_TOLERANCE, ZOOM_STEP};
use crate::command::{Command, CommandError, ErrorKind};

/// The most the view tilts up or down in `look_around`, in degrees.
const LOOK_PITCH_LIMIT: f64 = 89.0;
/// The eye height `position_camera` stands at by default: 5'6", in meters.
pub const DEFAULT_EYE_HEIGHT: f64 = 1.676;
pub const MIN_EYE_HEIGHT: f64 = 0.01;
pub const MAX_EYE_HEIGHT: f64 = 10_000.0;

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
            Command::LookAround { yaw, pitch } => {
                finite(&[yaw, pitch], "yaw and pitch")?;
                let limit = LOOK_PITCH_LIMIT.to_radians();
                let pitch = (camera.forward().z.clamp(-1.0, 1.0).asin() + pitch.to_radians())
                    .clamp(-limit, limit);
                let heading = DQuat::from_axis_angle(UP, yaw.to_radians()) * heading(&camera);
                let (eye, distance) = (camera.eye, camera.distance());
                set_view(&mut camera, eye, heading, pitch, distance);
            }
            Command::PositionCamera {
                point,
                eye_height,
                look_at,
            } => {
                check_coordinates(&[point])?;
                let height = eye_height.unwrap_or(DEFAULT_EYE_HEIGHT);
                if !(MIN_EYE_HEIGHT..=MAX_EYE_HEIGHT).contains(&height) {
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "eye_height must be between {MIN_EYE_HEIGHT} and {MAX_EYE_HEIGHT} m"
                        ),
                    ));
                }
                let eye = DVec3::from(point) + UP * height;
                let (heading, distance) = match look_at {
                    Some(p) => {
                        check_coordinates(&[p])?;
                        let flat = DVec3::new(p[0] - eye.x, p[1] - eye.y, 0.0);
                        let Some(h) = flat.try_normalize().filter(|_| flat.length() >= 1e-6) else {
                            return Err(CommandError::new(
                                ErrorKind::InvalidInput,
                                "look_at must lie away from the point, not straight above or below it",
                            ));
                        };
                        (h, flat.length().max(0.01))
                    }
                    None => (heading(&camera), camera.distance()),
                };
                camera.projection = Projection::Perspective;
                set_view(&mut camera, eye, heading, 0.0, distance);
            }
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

/// The horizontal direction the view faces: its forward direction without the height, or in a
/// vertical view the screen's up (what a look toward the horizon from there would face).
fn heading(camera: &Camera) -> DVec3 {
    let forward = camera.forward();
    let flat = DVec3::new(forward.x, forward.y, 0.0);
    if let Some(h) = flat.try_normalize().filter(|_| flat.length() > 1e-6) {
        return h;
    }
    let up = DVec3::new(camera.up.x, camera.up.y, 0.0);
    let up = up.try_normalize().unwrap_or(DVec3::Y);
    if forward.z < 0.0 { up } else { -up }
}

/// Points `camera` from `eye` along `heading` (horizontal, unit) tilted `pitch` radians up,
/// with its target `distance` ahead and `UP` up.
fn set_view(camera: &mut Camera, eye: DVec3, heading: DVec3, pitch: f64, distance: f64) {
    let forward = heading * pitch.cos() + UP * pitch.sin();
    camera.eye = eye;
    camera.target = eye + forward * distance;
    camera.up = UP;
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
