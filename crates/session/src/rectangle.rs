//! Rectangle geometry shared by `Command::AddRectangle` and the Rectangle tool.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;

use crate::command::{CommandError, ErrorKind};

/// In-plane side directions of rectangles drawn on a plane with unit normal `normal`: `u` is
/// horizontal when the plane is not (X on horizontal planes), and `v = normal x u`.
pub fn rectangle_axes(normal: DVec3) -> (DVec3, DVec3) {
    let u = if normal.cross(DVec3::Z).length() < 1e-9 {
        DVec3::X
    } else {
        DVec3::Z.cross(normal).normalize()
    };
    (u, normal.cross(u))
}

/// The four corners, in order, of the rectangle with opposite corners `corner` and `opposite`
/// whose sides follow `rectangle_axes(normal)`. Without a normal, the corners must differ in
/// exactly two coordinates and the rectangle lies in that axis plane.
pub fn rectangle_corners(
    corner: DVec3,
    opposite: DVec3,
    normal: Option<DVec3>,
) -> Result<[DVec3; 4], CommandError> {
    let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
    let d = opposite - corner;
    let normal = match normal {
        Some(n) if n.is_finite() && n.length() > 1e-9 => n.normalize(),
        Some(_) => return Err(invalid("normal must be a non-zero vector")),
        None => {
            let flat: Vec<DVec3> = [DVec3::X, DVec3::Y, DVec3::Z]
                .into_iter()
                .filter(|axis| d.dot(*axis).abs() < LENGTH_TOL)
                .collect();
            match flat[..] {
                [axis] => axis,
                _ => {
                    return Err(invalid(
                        "corners must differ in exactly two coordinates unless normal is given",
                    ));
                }
            }
        }
    };
    if d.dot(normal).abs() > LENGTH_TOL {
        return Err(invalid("opposite must lie in the plane through corner"));
    }
    let (u, v) = rectangle_axes(normal);
    let (du, dv) = (d.dot(u), d.dot(v));
    if du.abs() < LENGTH_TOL || dv.abs() < LENGTH_TOL {
        return Err(invalid("rectangle must have non-zero width and height"));
    }
    Ok([
        corner,
        corner + u * du,
        corner + u * du + v * dv,
        corner + v * dv,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_follow_the_world_on_axis_planes() {
        assert_eq!(rectangle_axes(DVec3::Z), (DVec3::X, DVec3::Y));
        assert_eq!(rectangle_axes(DVec3::X), (DVec3::Y, DVec3::Z));
        let (u, v) = rectangle_axes(DVec3::Y);
        assert!(u.abs_diff_eq(-DVec3::X, 1e-12) && v.abs_diff_eq(DVec3::Z, 1e-12));
    }

    #[test]
    fn corners_on_the_ground_without_a_normal() {
        let c = rectangle_corners(DVec3::ZERO, DVec3::new(2.0, -3.0, 0.0), None).unwrap();
        assert_eq!(
            c,
            [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, -3.0, 0.0),
                DVec3::new(0.0, -3.0, 0.0)
            ]
        );
    }

    #[test]
    fn ambiguous_or_flat_rectangles_are_rejected() {
        let o = DVec3::ZERO;
        assert!(rectangle_corners(o, DVec3::new(1.0, 1.0, 1.0), None).is_err());
        assert!(rectangle_corners(o, DVec3::new(1.0, 0.0, 0.0), None).is_err());
        assert!(rectangle_corners(o, DVec3::new(1.0, 1.0, 1.0), Some(DVec3::Z)).is_err());
        assert!(rectangle_corners(o, DVec3::new(1.0, 0.0, 0.0), Some(DVec3::Z)).is_err());
        assert!(rectangle_corners(o, DVec3::new(1.0, 1.0, 0.0), Some(DVec3::ZERO)).is_err());
    }
}
