use glam::dcamera::rh::{proj::directx, view::look_at_mat4};
use glam::{DMat4, DVec2, DVec3, Mat4};

pub const UP: DVec3 = DVec3::Z;

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

    pub fn proj(&self) -> DMat4 {
        let d = self.distance();
        directx::perspective(self.fov_y, self.aspect(), (d * 1e-3).max(1e-4), d * 1e3)
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
        Ray { origin: self.eye, dir: dir.normalize() }
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
        let r = Ray { origin: DVec3::new(0.0, 0.0, 10.0), dir: DVec3::new(0.0, 0.6, -0.8) };
        assert!(r.hit_ground().unwrap().abs_diff_eq(DVec3::new(0.0, 7.5, 0.0), 1e-9));
        let up = Ray { origin: DVec3::new(0.0, 0.0, 10.0), dir: DVec3::Z };
        assert!(up.hit_ground().is_none());
    }
}
