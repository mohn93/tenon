//! Circle geometry shared by `Command::AddCircle` and the Circle tool.

use std::f64::consts::TAU;

use glam::DVec3;
use kernel::geom::LENGTH_TOL;

use crate::command::{CommandError, ErrorKind};
use crate::document::unit_vector;
use crate::rectangle::rectangle_axes;

/// Sides a circle gets when none are asked for.
pub const DEFAULT_CIRCLE_SEGMENTS: u32 = 24;
/// Most sides a circle may have.
pub const MAX_CIRCLE_SEGMENTS: u32 = 360;

/// The corners, in order, of the regular polygon with `segments` sides inscribed in the circle
/// around `center` with `radius`, in the plane through `center` with `normal` (Z when absent).
/// The first corner lies along `rectangle_axes(normal).0` from the center, and the corners run
/// counter-clockwise around the normal.
pub fn circle_points(
    center: DVec3,
    radius: f64,
    normal: Option<DVec3>,
    segments: u32,
) -> Result<Vec<DVec3>, CommandError> {
    let invalid = |m: String| CommandError::new(ErrorKind::InvalidInput, m);
    let normal = match normal {
        None => DVec3::Z,
        Some(n) => match unit_vector(n) {
            Some(n) => n,
            None => return Err(invalid("normal must be a finite, non-zero vector".into())),
        },
    };
    if !(3..=MAX_CIRCLE_SEGMENTS).contains(&segments) {
        return Err(invalid(format!(
            "segments must be between 3 and {MAX_CIRCLE_SEGMENTS}"
        )));
    }
    // Neighbouring corners must be farther apart than the merge tolerance.
    let side = 2.0 * radius * (std::f64::consts::PI / segments as f64).sin();
    if !radius.is_finite() || radius <= 0.0 || side < 10.0 * LENGTH_TOL {
        return Err(invalid(
            "radius must be a positive number large enough for the segments".into(),
        ));
    }
    let (u, v) = rectangle_axes(normal);
    Ok((0..segments)
        .map(|k| {
            let a = TAU * k as f64 / segments as f64;
            center + (u * a.cos() + v * a.sin()) * radius
        })
        .collect())
}
