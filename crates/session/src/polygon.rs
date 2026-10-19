//! Regular polygon geometry shared by `Command::AddPolygon` and the Polygon tool.

use std::f64::consts::{PI, TAU};

use glam::DVec3;
use kernel::geom::LENGTH_TOL;

use crate::circle::MAX_CIRCLE_SEGMENTS;
use crate::command::{CommandError, ErrorKind};
use crate::document::unit_vector;
use crate::rectangle::rectangle_axes;

/// Sides a polygon gets when none are asked for.
pub const DEFAULT_POLYGON_SIDES: u32 = 6;

/// The corners, in order, of the regular polygon with `sides` sides around `center`, in the
/// plane through `center` with `normal` (Z when absent), counter-clockwise around the normal.
/// Inscribed, `radius` is the distance to the corners and the first corner lies along
/// `direction`; circumscribed, `radius` is the distance to the middles of the sides and the
/// middle of the side ending at the first corner lies along `direction`. `direction` is
/// projected into the plane (the plane's u axis from `rectangle_axes` when absent).
pub fn polygon_points(
    center: DVec3,
    radius: f64,
    sides: u32,
    normal: Option<DVec3>,
    circumscribed: bool,
    direction: Option<DVec3>,
) -> Result<Vec<DVec3>, CommandError> {
    let invalid = |m: String| CommandError::new(ErrorKind::InvalidInput, m);
    let normal = match normal {
        None => DVec3::Z,
        Some(n) => match unit_vector(n) {
            Some(n) => n,
            None => return Err(invalid("normal must be a finite, non-zero vector".into())),
        },
    };
    if !(3..=MAX_CIRCLE_SEGMENTS).contains(&sides) {
        return Err(invalid(format!(
            "sides must be between 3 and {MAX_CIRCLE_SEGMENTS}"
        )));
    }
    let half_turn = PI / sides as f64;
    let corner_radius = if circumscribed {
        radius / half_turn.cos()
    } else {
        radius
    };
    // Neighbouring corners must be farther apart than the merge tolerance.
    let side = 2.0 * corner_radius * half_turn.sin();
    if !radius.is_finite() || radius <= 0.0 || side < 10.0 * LENGTH_TOL {
        return Err(invalid(
            "radius must be a positive number large enough for the sides".into(),
        ));
    }
    let d = match direction {
        None => rectangle_axes(normal).0,
        Some(v) => {
            let in_plane = v - normal * v.dot(normal);
            if !in_plane.is_finite() || in_plane.length() < 1e-9 {
                return Err(invalid(
                    "direction must be a finite vector that is not along the normal".into(),
                ));
            }
            unit_vector(in_plane).ok_or_else(|| invalid("direction is not usable".into()))?
        }
    };
    let w = normal.cross(d);
    let first = if circumscribed { half_turn } else { 0.0 };
    Ok((0..sides)
        .map(|k| {
            let a = TAU * k as f64 / sides as f64 + first;
            center + (d * a.cos() + w * a.sin()) * corner_radius
        })
        .collect())
}
