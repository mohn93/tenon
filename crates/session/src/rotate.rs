//! Rotations shared by `Command::Rotate` and the Rotate tool.

use glam::{DAffine3, DMat3, DVec3};

/// The rigid motion that turns points by `angle` degrees about the line through `center` along
/// the unit vector `axis`, counter-clockwise when looking down the axis (from its tip toward
/// its base). Multiples of 90 degrees are exact, so quarter turns of axis-aligned geometry
/// stay on round coordinates.
pub fn rotation(center: DVec3, axis: DVec3, angle: f64) -> DAffine3 {
    let quarters = angle / 90.0;
    let (sin, cos) = if quarters == quarters.round() {
        match (quarters as i64).rem_euclid(4) {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        }
    } else {
        angle.to_radians().sin_cos()
    };
    // Rodrigues: R = cos I + sin [axis]x + (1 - cos) axis axis^T.
    let k = axis;
    let cross = DMat3::from_cols(
        DVec3::new(0.0, k.z, -k.y),
        DVec3::new(-k.z, 0.0, k.x),
        DVec3::new(k.y, -k.x, 0.0),
    );
    let outer = DMat3::from_cols(k * k.x, k * k.y, k * k.z);
    let turn = DMat3::IDENTITY * cos + cross * sin + outer * (1.0 - cos);
    DAffine3::from_mat3_translation(turn, center - turn * center)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_turns_are_exact_and_counter_clockwise() {
        let t = rotation(DVec3::new(1.0, 1.0, 0.0), DVec3::Z, 90.0);
        assert_eq!(
            t.transform_point3(DVec3::new(2.0, 1.0, 5.0)),
            DVec3::new(1.0, 2.0, 5.0)
        );
        let t = rotation(DVec3::ZERO, DVec3::X, -90.0);
        assert_eq!(t.transform_point3(DVec3::Y), -DVec3::Z);
        let t = rotation(DVec3::ZERO, DVec3::Z, 540.0);
        assert_eq!(t.transform_point3(DVec3::X), -DVec3::X);
    }

    #[test]
    fn other_angles_match_glam() {
        let axis = DVec3::new(1.0, 2.0, 3.0).normalize();
        let t = rotation(DVec3::ZERO, axis, 30.0);
        let q = glam::DQuat::from_axis_angle(axis, 30f64.to_radians());
        let p = DVec3::new(0.3, -1.0, 2.0);
        assert!(t.transform_point3(p).abs_diff_eq(q * p, 1e-12));
    }
}
