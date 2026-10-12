//! Arc geometry shared by `Command::AddArc` and the Arc tool.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;

use crate::circle::MAX_CIRCLE_SEGMENTS;
use crate::command::{CommandError, ErrorKind};

/// Segments an arc gets when none are asked for, whatever its angle (as in SketchUp).
pub const DEFAULT_ARC_SEGMENTS: u32 = 12;

/// The points, in order, of the arc around `center` that starts at `start` and turns `angle`
/// degrees counter-clockwise around `normal` (Z when absent; negative angles turn clockwise),
/// as `segments` equal straight segments: `segments + 1` points, the first exactly `start`.
/// `start` must lie in the plane through `center` with that normal; the angle must be between
/// 0 and 360 degrees in size, both excluded, and each segment at least 10 times the merge
/// tolerance long.
pub fn arc_points(
    center: DVec3,
    start: DVec3,
    angle: f64,
    normal: Option<DVec3>,
    segments: u32,
) -> Result<Vec<DVec3>, CommandError> {
    let invalid = |m: String| CommandError::new(ErrorKind::InvalidInput, m);
    let normal = match normal {
        None => DVec3::Z,
        Some(n) => match n.try_normalize() {
            Some(n) => n,
            None => return Err(invalid("normal must be a non-zero vector".into())),
        },
    };
    if !(2..=MAX_CIRCLE_SEGMENTS).contains(&segments) {
        return Err(invalid(format!(
            "segments must be between 2 and {MAX_CIRCLE_SEGMENTS}"
        )));
    }
    if !angle.is_finite() || angle.abs() < 1e-9 || angle.abs() > 360.0 - 1e-9 {
        return Err(invalid(
            "angle must be more than 0 and less than 360 degrees in size (use add_circle for \
             a whole circle)"
                .into(),
        ));
    }
    let r = start - center;
    if r.dot(normal).abs() > LENGTH_TOL {
        return Err(invalid(
            "start must lie in the plane through center with the normal".into(),
        ));
    }
    let r = r - normal * r.dot(normal);
    let radius = r.length();
    let step = angle.to_radians() / segments as f64;
    let side = 2.0 * radius * (step.abs() / 2.0).sin();
    if side < 10.0 * LENGTH_TOL {
        return Err(invalid(
            "start must be far enough from center for the segments".into(),
        ));
    }
    let across = normal.cross(r);
    Ok((0..=segments)
        .map(|k| {
            if k == 0 {
                return start;
            }
            let a = step * k as f64;
            center + r * a.cos() + across * a.sin()
        })
        .collect())
}

/// The center and angle (degrees, counter-clockwise around `normal`) of the arc from `start`
/// to `end` in the plane with unit `normal` whose middle lies `bulge` meters from the middle
/// of the chord: to the left of start-to-end looking down the normal when positive, to the
/// right when negative. None when the chord or the bulge is too short.
pub fn arc_from_bulge(start: DVec3, end: DVec3, bulge: f64, normal: DVec3) -> Option<(DVec3, f64)> {
    let chord = end - start;
    let half = chord.length() / 2.0;
    if half < LENGTH_TOL || !bulge.is_finite() || bulge.abs() < LENGTH_TOL {
        return None;
    }
    let left = normal.cross(chord / (2.0 * half));
    let radius = (half * half + bulge * bulge) / (2.0 * bulge.abs());
    let middle = (start + end) / 2.0 + left * bulge;
    let center = middle - left * bulge.signum() * radius;
    let angle = -bulge.signum() * 4.0 * (bulge.abs() / half).atan();
    Some((center, angle.to_degrees()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bulge_as_high_as_half_the_chord_is_a_half_circle() {
        let (s, e) = (DVec3::new(-1.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
        let (center, angle) = arc_from_bulge(s, e, 1.0, DVec3::Z).unwrap();
        assert!(center.distance(DVec3::ZERO) < 1e-12);
        assert!(
            (angle + 180.0).abs() < 1e-12,
            "clockwise over the top: {angle}"
        );
        let points = arc_points(center, s, angle, None, 4).unwrap();
        assert!(points[2].distance(DVec3::Y) < 1e-12);
        assert!(points[4].distance(e) < 1e-12);

        // A shallow bulge below the chord: a short arc turning counter-clockwise.
        let (center, angle) = arc_from_bulge(s, e, -0.25, DVec3::Z).unwrap();
        let points = arc_points(center, s, angle, None, 2).unwrap();
        assert!(angle > 0.0 && angle < 180.0);
        assert!(points[1].distance(DVec3::new(0.0, -0.25, 0.0)) < 1e-12);
        assert!(points[2].distance(e) < 1e-12);
    }
}
