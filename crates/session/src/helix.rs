//! Helix geometry for `Command::AddHelix`.

use std::f64::consts::TAU;

use glam::DVec3;
use kernel::geom::LENGTH_TOL;

use crate::command::{CommandError, ErrorKind};
use crate::document::unit_vector;
use crate::rectangle::rectangle_axes;

/// Edges per turn when none are asked for.
pub const DEFAULT_HELIX_SEGMENTS: u32 = 24;
/// Most edges per turn.
const MAX_SEGMENTS: u32 = 360;
/// Most turns a helix may have.
const MAX_TURNS: f64 = 1000.0;
/// Most points a helix may have (a curve takes no more).
const MAX_POINTS: f64 = 10_000.0;

/// What `helix_points` needs.
pub struct Helix {
    pub center: DVec3,
    /// Default Z; any length.
    pub axis: Option<DVec3>,
    pub radius: f64,
    pub pitch: f64,
    pub turns: f64,
    pub segments: u32,
    /// Projected into the plane square to the axis; default the plane's u axis.
    pub start_direction: Option<DVec3>,
    pub left_handed: bool,
}

/// The points, in order, of the helix: the first at `center + radius * start`, the last at the
/// exact end angle, each step turning by `turns / steps` of a turn and rising by its share of
/// the pitch.
pub fn helix_points(h: Helix) -> Result<Vec<DVec3>, CommandError> {
    let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
    let Some(axis) = unit_vector(h.axis.unwrap_or(DVec3::Z)) else {
        return Err(invalid("axis must be a finite, non-zero vector"));
    };
    if !h.center.is_finite() {
        return Err(invalid("center must be finite"));
    }
    if !(3..=MAX_SEGMENTS).contains(&h.segments) {
        return Err(invalid(&format!(
            "segments must be between 3 and {MAX_SEGMENTS}"
        )));
    }
    if !h.turns.is_finite() || h.turns <= 0.0 || h.turns > MAX_TURNS {
        return Err(invalid(&format!(
            "turns must be above 0 and at most {MAX_TURNS}"
        )));
    }
    if !h.radius.is_finite() || h.radius <= 0.0 {
        return Err(invalid("radius must be a positive number"));
    }
    if !h.pitch.is_finite() || h.pitch <= 0.0 {
        return Err(invalid(
            "pitch must be a positive number (left_handed turns the other way)",
        ));
    }
    if h.pitch < 10.0 * LENGTH_TOL {
        return Err(invalid(
            "pitch must be at least ten times the merge tolerance",
        ));
    }
    let steps_f = (h.turns * h.segments as f64 - 1e-9).ceil().max(1.0);
    if steps_f + 1.0 > MAX_POINTS {
        return Err(invalid(&format!(
            "a helix takes at most {MAX_POINTS} points: fewer turns or segments"
        )));
    }
    let steps = steps_f as usize;
    let (u, _) = rectangle_axes(axis);
    let u = match h.start_direction {
        None => u,
        Some(d) => {
            if !d.is_finite() {
                return Err(invalid("start_direction must be finite"));
            }
            let across = d.reject_from_normalized(axis);
            if across.length() <= 1e-9 * d.length() {
                return Err(invalid(
                    "start_direction must not be zero or along the axis",
                ));
            }
            across.normalize()
        }
    };
    let v = axis.cross(u);
    let sign = if h.left_handed { -1.0 } else { 1.0 };
    let total = h.turns * TAU;
    let rise = h.pitch * h.turns;
    let points: Vec<DVec3> = (0..=steps)
        .map(|k| {
            let f = k as f64 / steps as f64;
            let a = sign * total * f;
            h.center + (u * a.cos() + v * a.sin()) * h.radius + axis * (rise * f)
        })
        .collect();
    if !points.iter().all(|p| p.is_finite()) {
        return Err(invalid("the helix is too large"));
    }
    // Neighbouring points must be farther apart than the merge tolerance.
    let least = points
        .windows(2)
        .map(|w| w[0].distance(w[1]))
        .fold(f64::INFINITY, f64::min);
    if least < 10.0 * LENGTH_TOL {
        return Err(invalid(
            "radius and pitch must be large enough for the segments",
        ));
    }
    Ok(points)
}
