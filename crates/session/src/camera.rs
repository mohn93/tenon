use glam::dcamera::rh::{proj::directx, view::look_at_mat4};
use glam::{DMat4, DQuat, DVec2, DVec3, Mat4};
use std::f64::consts::PI;

pub const UP: DVec3 = DVec3::Z;
/// Closest the view direction may get to straight up or down, in radians.
pub const MIN_POLAR: f64 = 0.01;
pub const ORBIT_RAD_PER_PX: f64 = 0.006;
/// Distance multiplier per zoom step (one scroll notch).
pub const ZOOM_STEP: f64 = 0.85;
pub const MIN_ZOOM_DISTANCE: f64 = 0.01;
/// Ground hits farther than distance() * this are ignored as anchors.
pub const MAX_ANCHOR_FACTOR: f64 = 50.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: DVec3,
    pub dir: DVec3,
}

impl Ray {
    /// Intersection with the ground plane z = 0, if it lies in front of the origin.
    pub fn hit_ground(&self) -> Option<DVec3> {
        if self.dir.z.abs() < 1e-12 {
            return None;
        }
        let t = -self.origin.z / self.dir.z;
        (t > 0.0).then(|| self.origin + self.dir * t)
    }
}

/// Perspective camera looking from `eye` at `target`, Z-up.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub eye: DVec3,
    pub target: DVec3,
    /// Vertical field of view in radians.
    pub fov_y: f64,
    /// Viewport size in physical pixels.
    pub viewport: DVec2,
}

impl Camera {
    pub fn new(viewport: DVec2) -> Self {
        Self {
            eye: DVec3::new(-8.0, -12.0, 7.0),
            target: DVec3::ZERO,
            fov_y: 35f64.to_radians(),
            viewport,
        }
    }

    pub fn forward(&self) -> DVec3 {
        (self.target - self.eye).normalize()
    }

    pub fn right(&self) -> DVec3 {
        self.forward().cross(UP).normalize()
    }

    pub fn up(&self) -> DVec3 {
        self.right().cross(self.forward())
    }

    pub fn distance(&self) -> f64 {
        (self.target - self.eye).length()
    }

    pub fn aspect(&self) -> f64 {
        self.viewport.x / self.viewport.y.max(1.0)
    }

    pub fn view(&self) -> DMat4 {
        look_at_mat4(self.eye, self.target, UP)
    }

    /// Reversed-Z perspective with an infinite far plane: depth is 1 at the near plane and
    /// approaches 0 with distance, so nothing far away is ever clipped.
    pub fn proj(&self) -> DMat4 {
        let near = (self.distance() * 1e-3).max(1e-5);
        directx::perspective_infinite_reverse(self.fov_y, self.aspect(), near)
    }

    pub fn view_proj_f32(&self) -> Mat4 {
        (self.proj() * self.view()).as_mat4()
    }

    /// World-space ray through a pixel (origin top-left, y down).
    pub fn ray(&self, px: DVec2) -> Ray {
        let ndc_x = 2.0 * px.x / self.viewport.x - 1.0;
        let ndc_y = 1.0 - 2.0 * px.y / self.viewport.y;
        let half_h = (self.fov_y * 0.5).tan();
        let dir = self.forward()
            + self.right() * (ndc_x * half_h * self.aspect())
            + self.up() * (ndc_y * half_h);
        Ray {
            origin: self.eye,
            dir: dir.normalize(),
        }
    }

    pub fn orbit(&mut self, pivot: DVec3, delta_px: DVec2) {
        let yaw = -delta_px.x * ORBIT_RAD_PER_PX;
        // Rotating forward about right() by +a tilts it toward UP, reducing its polar angle by a.
        let polar = self
            .forward()
            .angle_between(UP)
            .clamp(MIN_POLAR, PI - MIN_POLAR);
        let pitch =
            (-delta_px.y * ORBIT_RAD_PER_PX).clamp(polar - (PI - MIN_POLAR), polar - MIN_POLAR);
        let rot = DQuat::from_axis_angle(UP, yaw) * DQuat::from_axis_angle(self.right(), pitch);
        self.eye = pivot + rot * (self.eye - pivot);
        self.target = pivot + rot * (self.target - pivot);
    }

    pub fn pan(&mut self, anchor: DVec3, delta_px: DVec2) {
        let depth = (anchor - self.eye).dot(self.forward()).max(1e-6);
        let world_per_px = 2.0 * depth * (self.fov_y * 0.5).tan() / self.viewport.y;
        let offset = (self.up() * delta_px.y - self.right() * delta_px.x) * world_per_px;
        self.eye += offset;
        self.target += offset;
    }

    pub fn zoom(&mut self, anchor: DVec3, steps: f64) {
        let dist = (self.eye - anchor).length();
        if dist <= 0.0 {
            return;
        }
        let factor = (dist * ZOOM_STEP.powf(steps)).max(MIN_ZOOM_DISTANCE) / dist;
        self.eye = anchor + (self.eye - anchor) * factor;
        self.target = anchor + (self.target - anchor) * factor;
    }

    pub fn anchor_at(&self, px: DVec2) -> DVec3 {
        let ray = self.ray(px);
        let max = self.distance() * MAX_ANCHOR_FACTOR;
        match ray.hit_ground() {
            Some(p) if (p - ray.origin).length() <= max => p,
            _ => ray.origin + ray.dir * self.distance(),
        }
    }

    /// Pixel position of a world point, or None if it is behind the camera.
    pub fn project(&self, p: DVec3) -> Option<DVec2> {
        let clip = self.proj() * self.view() * p.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(DVec2::new(
            (ndc.x + 1.0) * 0.5 * self.viewport.x,
            (1.0 - ndc.y) * 0.5 * self.viewport.y,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    #[test]
    fn center_ray_points_at_target() {
        let c = cam();
        let r = c.ray(DVec2::new(400.0, 300.0));
        assert!(r.origin.abs_diff_eq(c.eye, 1e-12));
        assert!(r.dir.abs_diff_eq(c.forward(), 1e-9));
    }

    #[test]
    fn project_target_lands_at_center() {
        let c = cam();
        let p = c.project(c.target).unwrap();
        assert!(p.abs_diff_eq(DVec2::new(400.0, 300.0), 1e-6));
    }

    #[test]
    fn ray_through_projected_point_hits_point() {
        let c = cam();
        let world = DVec3::new(3.0, -2.0, 1.5);
        let r = c.ray(c.project(world).unwrap());
        assert!(r.dir.abs_diff_eq((world - r.origin).normalize(), 1e-9));
    }

    #[test]
    fn point_behind_camera_does_not_project() {
        let c = cam();
        assert!(c.project(c.eye - c.forward() * 5.0).is_none());
    }

    #[test]
    fn screen_y_points_down_and_x_points_right() {
        let c = cam();
        assert!(c.project(c.target + c.up()).unwrap().y < 300.0);
        assert!(c.project(c.target + c.right()).unwrap().x > 400.0);
    }

    #[test]
    fn ray_hits_ground_plane() {
        let r = Ray {
            origin: DVec3::new(0.0, 0.0, 10.0),
            dir: DVec3::new(0.0, 0.6, -0.8),
        };
        assert!(
            r.hit_ground()
                .unwrap()
                .abs_diff_eq(DVec3::new(0.0, 7.5, 0.0), 1e-9)
        );
        let up = Ray {
            origin: DVec3::new(0.0, 0.0, 10.0),
            dir: DVec3::Z,
        };
        assert!(up.hit_ground().is_none());
    }

    #[test]
    fn orbit_preserves_distance_to_pivot() {
        let mut c = cam();
        let pivot = DVec3::new(1.0, 2.0, 0.0);
        let before = (c.eye - pivot).length();
        c.orbit(pivot, DVec2::new(120.0, -40.0));
        assert!(((c.eye - pivot).length() - before).abs() < 1e-9);
    }

    #[test]
    fn horizontal_orbit_keeps_elevation() {
        let mut c = cam();
        let polar = c.forward().angle_between(UP);
        c.orbit(c.target, DVec2::new(200.0, 0.0));
        assert!((c.forward().angle_between(UP) - polar).abs() < 1e-9);
    }

    #[test]
    fn dragging_down_raises_camera() {
        let mut c = cam();
        let z = c.eye.z;
        c.orbit(c.target, DVec2::new(0.0, 50.0));
        assert!(c.eye.z > z);
    }

    #[test]
    fn orbit_pitch_is_clamped_short_of_vertical() {
        let mut c = cam();
        for dy in [100_000.0, -100_000.0] {
            c.orbit(c.target, DVec2::new(0.0, dy));
            let polar = c.forward().angle_between(UP);
            assert!((MIN_POLAR - 1e-9..=PI - MIN_POLAR + 1e-9).contains(&polar));
        }
    }

    #[test]
    fn pan_moves_anchor_with_cursor() {
        let mut c = cam();
        let anchor = DVec3::new(2.0, 1.0, 0.0);
        let before = c.project(anchor).unwrap();
        let delta = DVec2::new(35.0, -20.0);
        c.pan(anchor, delta);
        assert!(c.project(anchor).unwrap().abs_diff_eq(before + delta, 1e-6));
    }

    #[test]
    fn zoom_keeps_anchor_on_screen_and_scales_distance() {
        let mut c = cam();
        let anchor = DVec3::new(-1.0, 3.0, 0.0);
        let px = c.project(anchor).unwrap();
        let d = (c.eye - anchor).length();
        c.zoom(anchor, 2.0);
        assert!(c.project(anchor).unwrap().abs_diff_eq(px, 1e-6));
        assert!(((c.eye - anchor).length() - d * ZOOM_STEP.powf(2.0)).abs() < 1e-9);
    }

    #[test]
    fn zoom_out_moves_away() {
        let mut c = cam();
        let d = (c.eye - c.target).length();
        c.zoom(c.target, -1.0);
        assert!((c.eye - c.target).length() > d);
    }

    #[test]
    fn zoom_never_reaches_anchor() {
        let mut c = cam();
        let anchor = c.target;
        c.zoom(anchor, 1000.0);
        assert!((c.eye - anchor).length() >= MIN_ZOOM_DISTANCE * (1.0 - 1e-9));
    }

    #[test]
    fn anchor_is_ground_point_under_cursor() {
        let c = cam();
        assert!(
            c.anchor_at(DVec2::new(400.0, 300.0))
                .abs_diff_eq(DVec3::ZERO, 1e-9)
        );
    }

    #[test]
    fn anchor_falls_back_when_cursor_is_on_sky() {
        let mut c = cam();
        c.eye = DVec3::new(0.0, -10.0, 1.0);
        c.target = DVec3::new(0.0, 0.0, 1.0);
        let px = DVec2::new(400.0, 0.0);
        let r = c.ray(px);
        assert!(
            c.anchor_at(px)
                .abs_diff_eq(r.origin + r.dir * c.distance(), 1e-9)
        );
    }
}
