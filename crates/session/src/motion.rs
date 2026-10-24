//! Motion between two states of a view: a camera and the transforms of some instances, as a
//! scene transition draws it and `render_view` renders a frame of it. Drawing only: nothing
//! here changes a document.

use glam::{DAffine3, DMat3, DQuat, DVec3};
use kernel::{InstanceId, MeshId, Scene};

use crate::camera::Camera;

/// The ease of a transition: 0 to 1 over `t` from 0 to 1, flat at both ends. `t` is clamped.
pub fn smoothstep(t: f64) -> f64 {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

/// The camera `t` of the way from `a` to `b`: the target, up and field of view interpolated
/// (`up` by direction), and the eye swung around the target: the direction from the target to
/// the eye turned by spherical interpolation (exactly opposite directions turn about `up`, so
/// going from the front to the back circles the model rather than passing through it) and the
/// distance interpolated geometrically, so it never reaches 0. The viewport is `a`'s; the
/// projection switches at the end. `t` 0 is `a` and 1 is `b`, exactly.
pub fn lerp_camera(a: &Camera, b: &Camera, t: f64) -> Camera {
    if t <= 0.0 || t.is_nan() {
        return a.clone();
    }
    if t >= 1.0 {
        return b.clone();
    }
    let up = a.up.lerp(b.up, t);
    let target = a.target.lerp(b.target, t);
    let (da, db) = (a.eye - a.target, b.eye - b.target);
    let (la, lb) = (da.length(), db.length());
    let eye = if la > 1e-12 && lb > 1e-12 && la.is_finite() && lb.is_finite() {
        let (ua, ub) = (da / la, db / lb);
        let cos = ua.dot(ub).clamp(-1.0, 1.0);
        let axis = ua.cross(ub);
        let axis = if axis.length() > 1e-9 {
            axis.normalize()
        } else if cos > 0.0 {
            // The same direction: no turn.
            DVec3::ZERO
        } else {
            // Exactly opposite: turn about up (its part across the direction), or any axis
            // across it when up lies along it.
            (a.up - ua * a.up.dot(ua))
                .try_normalize()
                .unwrap_or_else(|| ua.any_orthonormal_vector())
        };
        let direction = if axis == DVec3::ZERO {
            ua
        } else {
            DQuat::from_axis_angle(axis, cos.acos() * t) * ua
        };
        target + direction * (la * (lb / la).powf(t))
    } else {
        // An eye at its target has no direction: move it straight.
        a.eye.lerp(b.eye, t)
    };
    Camera {
        eye,
        target,
        // Opposite ups have no between: the later one.
        up: up.try_normalize().unwrap_or(b.up),
        fov_y: a.fov_y + (b.fov_y - a.fov_y) * t,
        viewport: a.viewport,
        projection: a.projection,
    }
}

/// A transform split into a rotation (determinant +1), a symmetric stretch (negative when the
/// transform mirrors) and a translation, so that the linear part is `rotation * stretch`.
/// None when the linear part is singular.
fn polar(t: &DAffine3) -> Option<(DQuat, DMat3)> {
    let m = t.matrix3;
    let det = m.determinant();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    // Newton iteration to the orthogonal factor, which keeps the sign of the determinant,
    // scaled by the Frobenius norms so that badly conditioned transforms converge quickly too.
    let frobenius = |a: &DMat3| a.to_cols_array().iter().map(|v| v * v).sum::<f64>().sqrt();
    let mut q = m;
    for _ in 0..100 {
        let inverse = q.inverse().transpose();
        let gamma = (frobenius(&inverse) / frobenius(&q)).sqrt();
        if !gamma.is_finite() || gamma <= 0.0 {
            return None;
        }
        let next = (q * gamma + inverse * (1.0 / gamma)) * 0.5;
        let done = (next - q).abs_diff_eq(DMat3::ZERO, 1e-14);
        q = next;
        if done {
            break;
        }
    }
    // Not converged to a rotation (a near-singular transform): no decomposition.
    if !(q.transpose() * q).abs_diff_eq(DMat3::IDENTITY, 1e-9) {
        return None;
    }
    let rotation = if det < 0.0 { -q } else { q };
    let stretch = rotation.transpose() * m;
    Some((DQuat::from_mat3(&rotation).normalize(), stretch))
}

/// The transform `t` of the way from `a` to `b`: the translation interpolated, the rotation
/// (of the polar decomposition) by spherical interpolation and the scale linearly. `t` 0 is
/// `a` and 1 is `b`, exactly. A pair that does not have a sane between (one mirrors and the
/// other does not, or one is singular) keeps the linear part of `a` until halfway and then
/// takes `b`'s, while the translation still moves.
pub fn lerp_transform(a: &DAffine3, b: &DAffine3, t: f64) -> DAffine3 {
    if t <= 0.0 || t.is_nan() {
        return *a;
    }
    if t >= 1.0 {
        return *b;
    }
    let translation = a.translation.lerp(b.translation, t);
    let matrix3 = match (polar(a), polar(b)) {
        (Some((ra, sa)), Some((rb, sb)))
            if (a.matrix3.determinant() < 0.0) == (b.matrix3.determinant() < 0.0) =>
        {
            let r = DMat3::from_quat(ra.slerp(rb, t));
            let s = sa * (1.0 - t) + sb * t;
            r * s
        }
        _ if t < 0.5 => a.matrix3,
        _ => b.matrix3,
    };
    DAffine3 {
        matrix3,
        translation,
    }
}

/// A move from one view to another: the cameras and the transforms of the instances of the
/// root that change.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    pub from_camera: Camera,
    pub to_camera: Camera,
    /// Each instance of the root with its transform at the start and at the end.
    pub moves: Vec<(InstanceId, DAffine3, DAffine3)>,
}

impl Motion {
    /// True when nothing differs between the ends.
    pub fn is_still(&self) -> bool {
        self.from_camera == self.to_camera && self.moves.iter().all(|(_, a, b)| a == b)
    }

    /// The camera at `t` (0 to 1, not eased).
    pub fn camera_at(&self, t: f64) -> Camera {
        lerp_camera(&self.from_camera, &self.to_camera, t)
    }

    /// A copy of `scene` with the instances moved to where they are at `t` (0 to 1, not
    /// eased); an instance `scene` lacks is skipped.
    pub fn scene_at(&self, scene: &Scene, t: f64) -> Scene {
        let mut out = scene.clone();
        for (i, a, b) in &self.moves {
            // A missing instance or a non-finite result leaves that instance where it is.
            let _ = out.set_instance_transform(MeshId::Root, *i, lerp_transform(a, b, t));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn rotation_angle(t: &DAffine3) -> Option<f64> {
        polar(t).map(|(r, _)| r.to_axis_angle().1)
    }

    fn rot_z(angle: f64, at: DVec3) -> DAffine3 {
        DAffine3::from_rotation_translation(DQuat::from_rotation_z(angle), at)
    }

    #[test]
    fn smoothstep_is_flat_at_the_ends_and_clamped() {
        assert_eq!(smoothstep(0.0), 0.0);
        assert_eq!(smoothstep(1.0), 1.0);
        assert_eq!(smoothstep(0.5), 0.5);
        assert_eq!(smoothstep(-3.0), 0.0);
        assert_eq!(smoothstep(7.0), 1.0);
        assert_eq!(smoothstep(f64::NAN), 0.0);
        assert!(smoothstep(0.1) < 0.1, "slow at first");
        assert!(smoothstep(0.9) > 0.9, "slow at the end");
    }

    #[test]
    fn the_ends_are_exact() {
        let a = rot_z(0.3, DVec3::new(1.0, 2.0, 3.0));
        let b = DAffine3::from_scale_rotation_translation(
            DVec3::new(2.0, 3.0, 4.0),
            DQuat::from_rotation_x(1.1),
            DVec3::new(-5.0, 0.5, 9.0),
        );
        assert_eq!(lerp_transform(&a, &b, 0.0), a);
        assert_eq!(lerp_transform(&a, &b, 1.0), b);
        let ca = Camera::new(glam::DVec2::new(100.0, 50.0));
        let mut cb = ca.clone();
        cb.eye = DVec3::new(9.0, 9.0, 9.0);
        cb.fov_y = 1.0;
        cb.projection = crate::camera::Projection::Parallel;
        assert_eq!(lerp_camera(&ca, &cb, 0.0), ca);
        assert_eq!(lerp_camera(&ca, &cb, 1.0), cb);
    }

    #[test]
    fn a_turn_is_a_slerp_not_a_shear() {
        let a = DAffine3::IDENTITY;
        let b = rot_z(FRAC_PI_2, DVec3::new(10.0, 0.0, 0.0));
        let mid = lerp_transform(&a, &b, 0.5);
        let x = mid.matrix3 * DVec3::X;
        let expected = DVec3::new((FRAC_PI_2 / 2.0).cos(), (FRAC_PI_2 / 2.0).sin(), 0.0);
        assert!((x - expected).length() < 1e-9, "{x:?} vs {expected:?}");
        assert!((x.length() - 1.0).abs() < 1e-9, "no shrinking mid-turn");
        assert!((mid.translation - DVec3::new(5.0, 0.0, 0.0)).length() < 1e-12);
    }

    #[test]
    fn scale_is_lerped_and_a_scaled_turn_keeps_its_rotation() {
        let a = DAffine3::from_scale_rotation_translation(
            DVec3::splat(1.0),
            DQuat::IDENTITY,
            DVec3::ZERO,
        );
        let b = DAffine3::from_scale_rotation_translation(
            DVec3::new(3.0, 3.0, 3.0),
            DQuat::from_rotation_z(FRAC_PI_2),
            DVec3::ZERO,
        );
        let mid = lerp_transform(&a, &b, 0.5);
        let x = mid.matrix3 * DVec3::X;
        assert!((x.length() - 2.0).abs() < 1e-9, "scale 2 halfway: {x:?}");
        let angle = rotation_angle(&mid).unwrap();
        assert!((angle - FRAC_PI_2 / 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_mirror_is_not_flattened_through_zero() {
        let a = DAffine3::IDENTITY;
        let b = DAffine3::from_scale(DVec3::new(-1.0, 1.0, 1.0));
        for k in 1..10 {
            let t = f64::from(k) / 10.0;
            let m = lerp_transform(&a, &b, t);
            assert!(
                m.matrix3.determinant().abs() > 0.5,
                "t {t}: {}",
                m.matrix3.determinant()
            );
        }
        // Two mirrors interpolate as usual.
        let c = DAffine3::from_scale(DVec3::new(-1.0, 1.0, 1.0));
        let d =
            DAffine3::from_rotation_translation(DQuat::from_rotation_z(FRAC_PI_2), DVec3::ZERO) * c;
        let m = lerp_transform(&c, &d, 0.5);
        assert!(m.matrix3.determinant() < 0.0);
        assert!((m.matrix3.determinant() + 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_near_singular_transform_gives_a_true_rotation_or_none() {
        for squash in [1e-3, 1e-6, 1e-9, 1e-11] {
            let m = DAffine3::from_scale_rotation_translation(
                DVec3::new(1e4, 1.0, squash),
                DQuat::from_rotation_x(0.7) * DQuat::from_rotation_z(0.4),
                DVec3::ZERO,
            ) * DAffine3::from_rotation_y(1.3);
            if let Some((r, _)) = polar(&m) {
                let r = DMat3::from_quat(r);
                assert!(
                    (r.transpose() * r).abs_diff_eq(DMat3::IDENTITY, 1e-9),
                    "squash {squash}"
                );
                assert!((r.determinant() - 1.0).abs() < 1e-9);
            }
            let mid = lerp_transform(&m, &DAffine3::IDENTITY, 0.5);
            assert!(mid.is_finite());
        }
    }

    #[test]
    fn a_singular_transform_does_not_break_it() {
        let a = DAffine3::from_scale(DVec3::new(1.0, 1.0, 0.0));
        let b = DAffine3::IDENTITY;
        let m = lerp_transform(&a, &b, 0.25);
        assert!(m.is_finite());
    }

    #[test]
    fn camera_lerp_moves_everything_and_switches_projection_at_the_end() {
        use crate::camera::Projection;
        let a = Camera::new(glam::DVec2::new(100.0, 50.0));
        let mut b = a.clone();
        b.eye = DVec3::new(10.0, 0.0, 0.0);
        b.target = DVec3::new(0.0, 4.0, 0.0);
        b.fov_y = a.fov_y + 0.2;
        b.projection = Projection::Parallel;
        let m = lerp_camera(&a, &b, 0.5);
        assert!((m.target - a.target.lerp(b.target, 0.5)).length() < 1e-12);
        let (la, lb) = ((a.eye - a.target).length(), (b.eye - b.target).length());
        let lm = (m.eye - m.target).length();
        assert!((lm - (la * lb).sqrt()).abs() < 1e-9, "{lm} vs {la} {lb}");
        assert!((m.fov_y - (a.fov_y + 0.1)).abs() < 1e-12);
        assert_eq!(m.projection, Projection::Perspective);
        assert!((m.up.length() - 1.0).abs() < 1e-12);
        assert_eq!(
            lerp_camera(&a, &b, 0.999).projection,
            Projection::Perspective
        );
        assert_eq!(lerp_camera(&a, &b, 1.0).projection, Projection::Parallel);
    }

    /// A camera at `eye` looking at the origin, Z up.
    fn looking_from(eye: DVec3) -> Camera {
        let mut c = Camera::new(glam::DVec2::new(100.0, 50.0));
        c.eye = eye;
        c.target = DVec3::ZERO;
        c.up = DVec3::Z;
        c
    }

    #[test]
    fn front_to_back_circles_the_target_instead_of_passing_through_it() {
        let front = looking_from(DVec3::new(0.0, -10.0, 2.0));
        let back = looking_from(DVec3::new(0.0, 10.0, -2.0));
        let distance = front.eye.length();
        for k in 1..10 {
            let m = lerp_camera(&front, &back, f64::from(k) / 10.0);
            assert!(m.eye.is_finite() && m.up.is_finite());
            assert!(
                ((m.eye - m.target).length() - distance).abs() < 1e-9,
                "distance kept at {k}"
            );
        }
        // Exactly opposite: halfway is off to the side (about the up axis), not at the target.
        let mid = lerp_camera(&front, &back, 0.5);
        assert!(mid.eye.x.abs() > 9.0, "{:?}", mid.eye);
        // The parallel view height follows the distance, so it stays as it was.
        let mut pa = front.clone();
        pa.projection = crate::camera::Projection::Parallel;
        let mut pb = back.clone();
        pb.projection = crate::camera::Projection::Parallel;
        let pm = lerp_camera(&pa, &pb, 0.5);
        assert!(((pm.eye - pm.target).length() - distance).abs() < 1e-9);
    }

    #[test]
    fn a_quarter_turn_keeps_the_distance() {
        let a = looking_from(DVec3::new(0.0, -10.0, 0.0));
        let b = looking_from(DVec3::new(10.0, 0.0, 0.0));
        let m = lerp_camera(&a, &b, 0.5);
        let s = std::f64::consts::FRAC_1_SQRT_2 * 10.0;
        assert!(
            (m.eye - DVec3::new(s, -s, 0.0)).length() < 1e-9,
            "{:?}",
            m.eye
        );
        // Different distances meet geometrically.
        let far = looking_from(DVec3::new(40.0, 0.0, 0.0));
        let m = lerp_camera(&a, &far, 0.5);
        assert!(((m.eye - m.target).length() - 20.0).abs() < 1e-9);
    }
}
