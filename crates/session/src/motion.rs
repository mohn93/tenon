//! Motion between two states of a view: a camera and the transforms of some instances, as a
//! scene transition draws it and `render_view` renders a frame of it. Drawing only: nothing
//! here changes a document.

use glam::{DAffine3, DMat3, DQuat};
use kernel::{InstanceId, MeshId, Scene};

use crate::camera::Camera;

/// The ease of a transition: 0 to 1 over `t` from 0 to 1, flat at both ends. `t` is clamped.
pub fn smoothstep(t: f64) -> f64 {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

/// The camera `t` of the way from `a` to `b`: eye, target, up and field of view interpolated
/// (`up` by direction), the viewport of `a`; the projection switches at the end. `t` 0 is `a`
/// and 1 is `b`, exactly.
pub fn lerp_camera(a: &Camera, b: &Camera, t: f64) -> Camera {
    if t <= 0.0 || t.is_nan() {
        return a.clone();
    }
    if t >= 1.0 {
        return b.clone();
    }
    let up = a.up.lerp(b.up, t);
    Camera {
        eye: a.eye.lerp(b.eye, t),
        target: a.target.lerp(b.target, t),
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
    // Newton iteration to the orthogonal factor, which keeps the sign of the determinant.
    let mut q = m;
    for _ in 0..50 {
        let next = (q + q.inverse().transpose()) * 0.5;
        let done = (next - q).abs_diff_eq(DMat3::ZERO, 1e-14);
        q = next;
        if done {
            break;
        }
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
    use glam::DVec3;
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
        assert!((m.eye - a.eye.lerp(b.eye, 0.5)).length() < 1e-12);
        assert!((m.target - a.target.lerp(b.target, 0.5)).length() < 1e-12);
        assert!((m.fov_y - (a.fov_y + 0.1)).abs() < 1e-12);
        assert_eq!(m.projection, Projection::Perspective);
        assert!((m.up.length() - 1.0).abs() < 1e-12);
        assert_eq!(
            lerp_camera(&a, &b, 0.999).projection,
            Projection::Perspective
        );
        assert_eq!(lerp_camera(&a, &b, 1.0).projection, Projection::Parallel);
    }
}
