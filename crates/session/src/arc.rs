//! Arc geometry shared by `Command::AddArc` and the Arc tool.

use glam::DVec3;
use kernel::geom::{LENGTH_TOL, PLANE_ANGLE_TOL};

use crate::circle::MAX_CIRCLE_SEGMENTS;
use crate::command::{CommandError, ErrorKind};
use crate::document::unit_vector;

/// Segments an arc gets when none are asked for, whatever its angle (as in SketchUp).
pub const DEFAULT_ARC_SEGMENTS: u32 = 12;

/// The points, in order, of the arc around `center` that starts at `start` and turns `angle`
/// degrees counter-clockwise around `normal` (Z when absent; negative angles turn clockwise),
/// as `segments` equal straight segments: `segments + 1` points, the first exactly `start`.
/// `start` must lie in the plane through `center` with that normal; the angle must be between
/// 0 and 360 degrees in size, both excluded; each segment must be at least 10 times the merge
/// tolerance long and turn clearly more than the kernel's corner threshold (or the edges
/// would not be tagged as a curve), and the end must not fall on the start.
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
        Some(n) => match unit_vector(n) {
            Some(n) => n,
            None => return Err(invalid("normal must be a finite, non-zero vector".into())),
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
    if step.abs() < 10.0 * PLANE_ANGLE_TOL {
        return Err(invalid(format!("arc is too flat for {segments} segments")));
    }
    let across = normal.cross(r);
    let end = center + r * (step * segments as f64).cos() + across * (step * segments as f64).sin();
    if end.distance(start) < 10.0 * LENGTH_TOL {
        return Err(invalid(
            "arc would close on itself (use add_circle for a whole circle)".into(),
        ));
    }
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

/// The center, angle (degrees, counter-clockwise around the normal) and unit normal of the
/// arc from `start` to `end` through `through`: the normal makes start, through, end run
/// counter-clockwise, so the angle is between 0 and 360. None when the three points are
/// (nearly) on one line or `end` is on `start`.
pub fn arc_through(start: DVec3, end: DVec3, through: DVec3) -> Option<(DVec3, f64, DVec3)> {
    let chord = end - start;
    if chord.length() < 10.0 * LENGTH_TOL {
        return None;
    }
    let off_line = chord.cross(through - start).length() / chord.length();
    if !off_line.is_finite() || off_line < LENGTH_TOL {
        return None;
    }
    // The circumcenter, with `through` as the origin.
    let (a, b) = (start - through, end - through);
    let axb = a.cross(b);
    let center = through
        + (b * a.length_squared() - a * b.length_squared()).cross(axb)
            / (2.0 * axb.length_squared());
    let normal = unit_vector((through - start).cross(end - through))?;
    let (u, v) = (start - center, end - center);
    let mut angle = normal.dot(u.cross(v)).atan2(u.dot(v));
    if angle <= 0.0 {
        angle += std::f64::consts::TAU;
    }
    Some((center, angle.to_degrees(), normal))
}

/// What `add_arc` was given, as the center, start, angle and normal `arc_points` takes: a
/// `center` and `angle`; an `end` and a point it passes `through`; or an `end` and a `bulge`
/// (the height of the arc's middle above the chord, positive to the left of start to end
/// looking down the normal, as the Arc tool pulls it).
pub fn arc_spec(
    center: Option<[f64; 3]>,
    start: [f64; 3],
    angle: Option<f64>,
    end: Option<[f64; 3]>,
    through: Option<[f64; 3]>,
    bulge: Option<f64>,
    normal: Option<[f64; 3]>,
) -> Result<(DVec3, DVec3, f64, Option<DVec3>), CommandError> {
    let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
    let start = DVec3::from(start);
    match (center, angle, end, through, bulge) {
        (Some(c), Some(a), None, None, None) => {
            Ok((DVec3::from(c), start, a, normal.map(DVec3::from)))
        }
        (None, None, Some(e), Some(t), None) => {
            if normal.is_some() {
                return Err(invalid(
                    "normal goes with center or bulge: three points set their own plane",
                ));
            }
            let (c, a, n) =
                arc_through(start, DVec3::from(e), DVec3::from(t)).ok_or_else(|| {
                    invalid("start, through and end must be apart and not on one line")
                })?;
            Ok((c, start, a, Some(n)))
        }
        (None, None, Some(e), None, Some(b)) => {
            let n = match normal {
                None => DVec3::Z,
                Some(n) => unit_vector(DVec3::from(n))
                    .ok_or_else(|| invalid("normal must be a finite, non-zero vector"))?,
            };
            let end = DVec3::from(e);
            if (end - start).dot(n).abs() > LENGTH_TOL {
                return Err(invalid(
                    "start and end must lie in one plane with the normal",
                ));
            }
            let (c, a) = arc_from_bulge(start, end, b, n)
                .ok_or_else(|| invalid("end must be away from start and bulge must not be 0"))?;
            Ok((c, start, a, Some(n)))
        }
        _ => Err(invalid(
            "give center and angle, or end with through or bulge",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_points_give_the_arc_through_them() {
        let (s, t, e) = (DVec3::X, DVec3::Y, -DVec3::X);
        let (center, angle, normal) = arc_through(s, e, t).unwrap();
        assert!(center.distance(DVec3::ZERO) < 1e-12);
        assert!((angle - 180.0).abs() < 1e-9 && normal.abs_diff_eq(DVec3::Z, 1e-12));
        // Through the other side: the same half circle, clockwise seen from above.
        let (_, angle, normal) = arc_through(s, e, -DVec3::Y).unwrap();
        assert!((angle - 180.0).abs() < 1e-9 && normal.abs_diff_eq(-DVec3::Z, 1e-12));
        // A long way round: three quarters of a circle in a vertical plane.
        let (s, e) = (DVec3::new(0.0, 0.0, 1.0), DVec3::new(0.0, 1.0, 0.0));
        let t = DVec3::new(0.0, -1.0, 0.0);
        let (center, angle, normal) = arc_through(s, e, t).unwrap();
        let points = arc_points(center, s, angle, Some(normal), 3).unwrap();
        assert!((angle - 270.0).abs() < 1e-9, "{angle}");
        assert!(points[3].distance(e) < 1e-9);
        assert!(points.iter().all(|p| (p.length() - 1.0).abs() < 1e-9));
        assert!(arc_through(s, s * 2.0, s * 3.0).is_none(), "on one line");
        assert!(arc_through(s, s, t).is_none(), "end on start");
    }

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
