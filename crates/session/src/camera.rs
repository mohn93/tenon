use glam::dcamera::rh::{proj::directx, view::look_at_mat4};
use glam::{DAffine3, DMat4, DQuat, DVec2, DVec3, Mat4};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

pub const UP: DVec3 = DVec3::Z;
/// Closest the view direction may get to straight up or down when orbiting, in radians.
pub const MIN_POLAR: f64 = 0.01;
pub const ORBIT_RAD_PER_PX: f64 = 0.006;
/// Distance multiplier per zoom step (one scroll notch).
pub const ZOOM_STEP: f64 = 0.85;
pub const MIN_ZOOM_DISTANCE: f64 = 0.01;
/// Ground hits farther than distance() * this are ignored as anchors.
pub const MAX_ANCHOR_FACTOR: f64 = 50.0;
/// How far (in eye-to-target distances) `Camera::line_span` reaches along an infinite line.
pub const LINE_REACH: f64 = 1000.0;
/// How far in front of and behind the target (in eye-to-target distances) a parallel view
/// sees. Geometry behind the eye is seen too: in a parallel view the eye only sets the zoom.
pub const PARALLEL_DEPTH: f64 = 100.0;
/// A view whose direction is closer than this (sine of the angle) to straight up or down is
/// vertical: its screen's up is a horizontal direction instead of `UP`.
pub const VERTICAL_TOLERANCE: f64 = 1e-9;
/// Spare room zoom extents leaves around the model in a parallel view.
pub const FIT_MARGIN: f64 = 1.05;

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
        (t > 0.0).then(|| {
            let p = self.origin + self.dir * t;
            DVec3::new(p.x, p.y, 0.0)
        })
    }
}

/// How the camera projects the model onto the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    /// Farther things look smaller; the view spans `fov_y` vertically.
    #[default]
    Perspective,
    /// Parallel (orthographic): sizes do not change with distance. The view is
    /// `Camera::view_height` tall, as tall as the perspective view is at the target, so
    /// switching keeps what is at the target the same size.
    Parallel,
}

/// The six views along the axes, and an isometric one from the front left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StandardView {
    /// From above (+Z), +Y up the screen.
    Top,
    /// From below (-Z), +Y up the screen.
    Bottom,
    /// From the front (-Y), looking along +Y.
    Front,
    /// From the back (+Y).
    Back,
    /// From the left (-X), looking along +X.
    Left,
    /// From the right (+X).
    Right,
    /// From the front left above, along (1, 1, -1).
    Iso,
}

impl StandardView {
    pub const ALL: [StandardView; 7] = [
        StandardView::Top,
        StandardView::Bottom,
        StandardView::Front,
        StandardView::Back,
        StandardView::Left,
        StandardView::Right,
        StandardView::Iso,
    ];

    /// The unit direction from the target to the eye, and the screen's up direction.
    pub fn direction(self) -> (DVec3, DVec3) {
        match self {
            StandardView::Top => (DVec3::Z, DVec3::Y),
            StandardView::Bottom => (-DVec3::Z, DVec3::Y),
            StandardView::Front => (-DVec3::Y, UP),
            StandardView::Back => (DVec3::Y, UP),
            StandardView::Left => (-DVec3::X, UP),
            StandardView::Right => (DVec3::X, UP),
            StandardView::Iso => (DVec3::new(-1.0, -1.0, 1.0).normalize(), UP),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StandardView::Top => "Top",
            StandardView::Bottom => "Bottom",
            StandardView::Front => "Front",
            StandardView::Back => "Back",
            StandardView::Left => "Left",
            StandardView::Right => "Right",
            StandardView::Iso => "Iso",
        }
    }
}

/// Camera looking from `eye` at `target`, with `up` pointing up the screen, in perspective or
/// parallel projection.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub eye: DVec3,
    pub target: DVec3,
    /// Which way is up: `UP` for the document's camera, except in a vertical view (straight
    /// down or up), where it is the horizontal direction pointing up the screen; and in a
    /// camera seen from another frame (see `placed`). Orbiting and the ground plane always
    /// use `UP`.
    pub up: DVec3,
    /// Vertical field of view in radians. A parallel view keeps it, to size the view (see
    /// `view_height`) and for switching back.
    pub fov_y: f64,
    /// Viewport size in physical pixels.
    pub viewport: DVec2,
    pub projection: Projection,
}

impl Camera {
    pub fn new(viewport: DVec2) -> Self {
        Self {
            eye: DVec3::new(-8.0, -12.0, 7.0),
            target: DVec3::ZERO,
            up: UP,
            fov_y: 35f64.to_radians(),
            viewport,
            projection: Projection::Perspective,
        }
    }

    pub fn forward(&self) -> DVec3 {
        (self.target - self.eye).normalize()
    }

    pub fn right(&self) -> DVec3 {
        self.forward().cross(self.up).normalize()
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

    pub fn is_parallel(&self) -> bool {
        self.projection == Projection::Parallel
    }

    /// True when the view looks straight down or up (see `VERTICAL_TOLERANCE`).
    pub fn is_vertical(&self) -> bool {
        self.forward().cross(UP).length() < VERTICAL_TOLERANCE
    }

    /// Height of the view at the target, in meters: in a parallel view, of the whole view.
    pub fn view_height(&self) -> f64 {
        2.0 * self.distance() * (self.fov_y * 0.5).tan()
    }

    /// How far `p` lies in front of the eye along the view direction.
    pub fn depth(&self, p: DVec3) -> f64 {
        (p - self.eye).dot(self.forward())
    }

    /// The depths (see `depth`) between which the camera sees: from the near plane on in
    /// perspective; `PARALLEL_DEPTH` distances either side of the target in a parallel view,
    /// so the near end can lie behind the eye.
    pub fn depth_range(&self) -> (f64, f64) {
        match self.projection {
            Projection::Perspective => (self.near(), f64::INFINITY),
            Projection::Parallel => {
                let d = self.distance();
                (d - d * PARALLEL_DEPTH, d + d * PARALLEL_DEPTH)
            }
        }
    }

    /// True when `p` lies between the depths the camera sees.
    pub fn sees_depth(&self, p: DVec3) -> bool {
        let (near, far) = self.depth_range();
        let depth = self.depth(p);
        depth > near && depth < far
    }

    /// World length of one pixel at `p`, across the view.
    pub fn pixel_size(&self, p: DVec3) -> f64 {
        let depth = match self.projection {
            Projection::Perspective => self.depth(p).max(1e-6),
            Projection::Parallel => self.distance(),
        };
        2.0 * depth * (self.fov_y * 0.5).tan() / self.viewport.y
    }

    /// How far `p` is from the eye, for sizing things drawn around it on screen: in a parallel
    /// view every point is as far as the target.
    pub fn eye_distance(&self, p: DVec3) -> f64 {
        match self.projection {
            Projection::Perspective => self.eye.distance(p),
            Projection::Parallel => self.distance(),
        }
    }

    /// Where the line of sight to `p` starts: the eye, or in a parallel view the point behind
    /// `p` where the view begins (see `depth_range`).
    pub fn sight_origin(&self, p: DVec3) -> DVec3 {
        match self.projection {
            Projection::Perspective => self.eye,
            Projection::Parallel => p - self.forward() * (self.depth(p) - self.depth_range().0),
        }
    }

    pub fn view(&self) -> DMat4 {
        look_at_mat4(self.eye, self.target, self.up)
    }

    /// Reversed-Z projection: depth is 1 at the near plane and decreases with distance. In
    /// perspective the far plane is infinite, so nothing far away is ever clipped; a parallel
    /// view sees the depths of `depth_range`, linearly.
    pub fn proj(&self) -> DMat4 {
        match self.projection {
            Projection::Perspective => {
                directx::perspective_infinite_reverse(self.fov_y, self.aspect(), self.near())
            }
            Projection::Parallel => {
                let h = self.view_height() * 0.5;
                let w = h * self.aspect();
                let (near, far) = self.depth_range();
                // Near and far swapped: reversed depth.
                directx::orthographic(-w, w, -h, h, far, near)
            }
        }
    }

    /// Distance from the eye to the perspective near plane.
    pub fn near(&self) -> f64 {
        (self.distance() * 1e-3).max(1e-5)
    }

    /// The part of the infinite line through `point` along `dir` worth drawing or snapping
    /// to: within the depths the camera sees (in perspective from just past the near plane
    /// to `LINE_REACH` eye-to-target distances), and within `LINE_REACH` distances along the
    /// line of the point nearest the eye (in a parallel view, nearest the line of sight
    /// through the target). Points beyond look the same as the end. None when no part of the
    /// line is in view in depth (or `dir` is zero).
    pub fn line_span(&self, point: DVec3, dir: DVec3) -> Option<[DVec3; 2]> {
        let dir = dir.try_normalize()?;
        let reach = self.distance() * LINE_REACH;
        let (near, far, from) = match self.projection {
            Projection::Perspective => (self.near() * 2.0, reach, self.eye),
            Projection::Parallel => {
                let (near, far) = self.depth_range();
                let margin = (far - near) * 1e-3;
                (near + margin, far - margin, self.target)
            }
        };
        // Along the line, around the point nearest `from`.
        let foot = (from - point).dot(dir);
        let (mut lo, mut hi) = (foot - reach, foot + reach);
        // In depth: near <= depth0 + t * slope <= far.
        let depth0 = self.depth(point);
        let slope = dir.dot(self.forward());
        if slope.abs() < 1e-12 {
            if !(near..=far).contains(&depth0) {
                return None;
            }
        } else {
            let (a, b) = ((near - depth0) / slope, (far - depth0) / slope);
            lo = lo.max(a.min(b));
            hi = hi.min(a.max(b));
        }
        (lo < hi).then(|| [point + dir * lo, point + dir * hi])
    }

    pub fn view_proj_f32(&self) -> Mat4 {
        (self.proj() * self.view()).as_mat4()
    }

    /// World-space ray through a pixel (origin top-left, y down). In perspective it starts at
    /// the eye; in a parallel view every ray runs along the view direction from where the
    /// view begins (see `depth_range`), so it meets geometry behind the eye too.
    pub fn ray(&self, px: DVec2) -> Ray {
        let ndc_x = 2.0 * px.x / self.viewport.x - 1.0;
        let ndc_y = 1.0 - 2.0 * px.y / self.viewport.y;
        let half_h = (self.fov_y * 0.5).tan();
        match self.projection {
            Projection::Perspective => {
                let dir = self.forward()
                    + self.right() * (ndc_x * half_h * self.aspect())
                    + self.up() * (ndc_y * half_h);
                Ray {
                    origin: self.eye,
                    dir: dir.normalize(),
                }
            }
            Projection::Parallel => {
                let half = self.view_height() * 0.5;
                let forward = self.forward();
                let origin = self.eye
                    + forward * self.depth_range().0
                    + self.right() * (ndc_x * half * self.aspect())
                    + self.up() * (ndc_y * half);
                Ray {
                    origin,
                    dir: forward,
                }
            }
        }
    }

    /// The same view in other coordinates: `t` (rigid) maps this camera's coordinates to the
    /// new ones. Points `t` maps project where they did.
    pub fn placed(&self, t: DAffine3) -> Camera {
        Camera {
            eye: t.transform_point3(self.eye),
            target: t.transform_point3(self.target),
            up: t.transform_vector3(self.up).normalize(),
            ..self.clone()
        }
    }

    /// Orbits by a mouse drag of `delta_px` (see `turn`): dragging right turns the camera
    /// clockwise seen from above, dragging down raises the eye.
    pub fn orbit(&mut self, pivot: DVec3, delta_px: DVec2) {
        self.turn(
            pivot,
            -delta_px.x * ORBIT_RAD_PER_PX,
            -delta_px.y * ORBIT_RAD_PER_PX,
        );
    }

    /// Turns the camera about `pivot`: by `yaw` radians about `UP` (counter-clockwise seen
    /// from above), then by `pitch` radians tilting the view toward `UP` (negative raises the
    /// eye). The pitch stops `MIN_POLAR` short of straight up or down, so from a vertical
    /// view only a pitch toward the horizon moves it (from Top a negative pitch, from Bottom
    /// a positive one). A view still vertical turns its
    /// screen's up with the yaw; any other view has `UP` up.
    pub fn turn(&mut self, pivot: DVec3, yaw: f64, pitch: f64) {
        // Rotating forward about right() by +a tilts it toward UP, reducing its polar angle by a.
        let polar = self
            .forward()
            .angle_between(UP)
            .clamp(MIN_POLAR, PI - MIN_POLAR);
        let pitch = pitch.clamp(polar - (PI - MIN_POLAR), polar - MIN_POLAR);
        let spin = DQuat::from_axis_angle(UP, yaw);
        let rot = spin * DQuat::from_axis_angle(self.right(), pitch);
        self.eye = pivot + rot * (self.eye - pivot);
        self.target = pivot + rot * (self.target - pivot);
        self.up = if self.is_vertical() {
            spin * self.up
        } else {
            UP
        };
    }

    pub fn pan(&mut self, anchor: DVec3, delta_px: DVec2) {
        let world_per_px = self.pixel_size(anchor);
        let offset = (self.up() * delta_px.y - self.right() * delta_px.x) * world_per_px;
        self.eye += offset;
        self.target += offset;
    }

    /// Zooms `steps` scroll notches toward `anchor`, which stays where it is on screen. The
    /// eye never gets nearer than `MIN_ZOOM_DISTANCE` to the anchor (in a parallel view, to
    /// the target: the view height shrinks with the distance).
    pub fn zoom(&mut self, anchor: DVec3, steps: f64) {
        let dist = match self.projection {
            Projection::Perspective => (self.eye - anchor).length(),
            Projection::Parallel => self.distance(),
        };
        if dist <= 0.0 {
            return;
        }
        let factor = (dist * ZOOM_STEP.powf(steps)).max(MIN_ZOOM_DISTANCE) / dist;
        self.eye = anchor + (self.eye - anchor) * factor;
        self.target = anchor + (self.target - anchor) * factor;
    }

    /// Moves the eye along the view toward the target to `1 / factor` of its distance (no
    /// nearer than `MIN_ZOOM_DISTANCE`): things at the target look `factor` times bigger.
    pub fn zoom_by(&mut self, factor: f64) {
        let d = (self.distance() / factor).max(MIN_ZOOM_DISTANCE);
        self.eye = self.target - self.forward() * d;
    }

    /// Looks at the target from `view`'s direction, from the same distance.
    pub fn look_from(&mut self, view: StandardView) {
        let (from, up) = view.direction();
        self.eye = self.target + from * self.distance();
        self.up = up;
    }

    /// Keeps the view direction and moves the camera so the box `lo`..`hi` fits the view
    /// around its center: in perspective, the sphere around the box fits; in a parallel view
    /// the box itself does, with `FIT_MARGIN` to spare.
    pub fn fit(&mut self, lo: DVec3, hi: DVec3) {
        let center = (lo + hi) * 0.5;
        let half_y = self.fov_y * 0.5;
        let d = match self.projection {
            Projection::Perspective => {
                let half_x = (half_y.tan() * self.aspect()).atan();
                let radius = (hi - lo).length() * 0.5;
                radius / half_y.min(half_x).sin()
            }
            Projection::Parallel => {
                let (right, up) = (self.right(), self.up());
                let forward = self.forward();
                let (mut across, mut along, mut depth) = (0.0f64, 0.0f64, 0.0f64);
                for k in 0..8 {
                    let corner = DVec3::new(
                        if k & 1 == 0 { lo.x } else { hi.x },
                        if k & 2 == 0 { lo.y } else { hi.y },
                        if k & 4 == 0 { lo.z } else { hi.z },
                    ) - center;
                    across = across.max(corner.dot(right).abs());
                    along = along.max(corner.dot(up).abs());
                    depth = depth.max(corner.dot(forward).abs());
                }
                let height = (2.0 * along).max(2.0 * across / self.aspect()) * FIT_MARGIN;
                let d = height / (2.0 * half_y.tan());
                // The whole box must also lie inside the depth range.
                d.max(depth * FIT_MARGIN / PARALLEL_DEPTH)
            }
        };
        let forward = self.forward();
        self.target = center;
        self.eye = center - forward * d.max(MIN_ZOOM_DISTANCE);
    }

    /// The ground point under a pixel, if the ground is there within `MAX_ANCHOR_FACTOR`
    /// eye-to-target distances (of the eye; in a parallel view, of the target in depth).
    pub fn ground_at(&self, px: DVec2) -> Option<DVec3> {
        let ray = self.ray(px);
        let p = ray.hit_ground()?;
        let far = match self.projection {
            Projection::Perspective => (p - ray.origin).length(),
            Projection::Parallel => (self.depth(p) - self.distance()).abs(),
        };
        (far <= self.distance() * MAX_ANCHOR_FACTOR).then_some(p)
    }

    /// The point to orbit about or zoom toward under a pixel: the ground there, else the
    /// point as deep as the target.
    pub fn anchor_at(&self, px: DVec2) -> DVec3 {
        self.ground_at(px).unwrap_or_else(|| {
            let ray = self.ray(px);
            let along = match self.projection {
                Projection::Perspective => self.distance(),
                Projection::Parallel => self.distance() - self.depth_range().0,
            };
            ray.origin + ray.dir * along
        })
    }

    /// Pixel position of a world point, or None if the camera does not see its depth (behind
    /// the eye in perspective; outside `depth_range` in a parallel view).
    pub fn project(&self, p: DVec3) -> Option<DVec2> {
        if self.is_parallel() && !self.sees_depth(p) {
            return None;
        }
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
    fn line_spans_stay_in_front_of_the_camera() {
        let c = cam();
        // A line through the target across the view: both ends project, far to either side.
        let [a, b] = c.line_span(c.target, c.right()).unwrap();
        assert!(c.project(a).is_some() && c.project(b).is_some());
        assert!(a.distance(b) > c.distance() * 100.0);
        // A line through the eye along the view (given pointing backwards) starts just past
        // the near plane.
        let [far, near] = c.line_span(c.eye, -c.forward()).unwrap();
        let depth = |p: DVec3| (p - c.eye).dot(c.forward());
        assert!(depth(near) > c.near() && depth(near) < c.near() * 3.0);
        assert!((depth(far) - c.distance() * LINE_REACH).abs() < 1e-6);
        // A line wholly behind the camera, across the view, has no span.
        let behind = c.eye - c.forward();
        assert!(c.line_span(behind, c.right()).is_none());
        assert!(c.line_span(c.target, DVec3::ZERO).is_none());
    }

    #[test]
    fn ray_through_projected_point_hits_point() {
        let c = cam();
        let world = DVec3::new(3.0, -2.0, 1.5);
        let r = c.ray(c.project(world).unwrap());
        assert!(r.dir.abs_diff_eq((world - r.origin).normalize(), 1e-9));
    }

    #[test]
    fn a_placed_camera_sees_placed_points_where_they_were() {
        let c = cam();
        let t = DAffine3::from_translation(DVec3::new(1.0, 2.0, 3.0))
            * DAffine3::from_axis_angle(DVec3::new(1.0, -1.0, 0.5).normalize(), 1.2);
        let placed = c.placed(t);
        for p in [
            DVec3::new(3.0, -2.0, 1.5),
            DVec3::ZERO,
            DVec3::new(-1.0, 4.0, 0.0),
        ] {
            let (a, b) = (
                c.project(p).unwrap(),
                placed.project(t.transform_point3(p)).unwrap(),
            );
            assert!(a.abs_diff_eq(b, 1e-6), "{a} vs {b}");
        }
        let ray = placed.ray(DVec2::new(100.0, 50.0));
        let original = c.ray(DVec2::new(100.0, 50.0));
        assert!(ray.dir.abs_diff_eq(t.transform_vector3(original.dir), 1e-9));
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
    fn ground_hit_has_exactly_zero_z() {
        for i in 1..200 {
            let oz = 0.37 * i as f64;
            let r = Ray {
                origin: DVec3::new(0.3, 0.7, oz),
                dir: DVec3::new(0.3, 0.2, -0.1 - 0.013 * i as f64).normalize(),
            };
            assert_eq!(r.hit_ground().unwrap().z, 0.0, "origin z {oz}");
        }
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

    fn parallel() -> Camera {
        Camera {
            projection: Projection::Parallel,
            ..cam()
        }
    }

    fn top() -> Camera {
        let mut c = cam();
        c.look_from(StandardView::Top);
        c
    }

    #[test]
    fn parallel_rays_run_along_the_view_and_hit_what_they_project_from() {
        let c = parallel();
        let world = DVec3::new(3.0, -2.0, 1.5);
        let px = c.project(world).unwrap();
        let r = c.ray(px);
        assert!(r.dir.abs_diff_eq(c.forward(), 1e-12));
        assert!(
            c.ray(DVec2::new(10.0, 590.0))
                .dir
                .abs_diff_eq(c.forward(), 1e-12)
        );
        let t = (world - r.origin).dot(r.dir);
        assert!((r.origin + r.dir * t).abs_diff_eq(world, 1e-9));
        assert!(
            c.project(r.origin + r.dir * 1e-3)
                .unwrap()
                .abs_diff_eq(px, 1e-6)
        );
        let center = c.ray(DVec2::new(400.0, 300.0));
        assert!(((c.target - center.origin).length() - c.distance() * PARALLEL_DEPTH).abs() < 1e-9);
    }

    #[test]
    fn parallel_depth_is_reversed_and_linear_over_the_range() {
        let c = parallel();
        let (near, far) = c.depth_range();
        let z = |depth: f64| {
            let clip = c.proj() * c.view() * (c.eye + c.forward() * depth).extend(1.0);
            clip.z / clip.w
        };
        assert!((z(near) - 1.0).abs() < 1e-9 && z(far).abs() < 1e-9);
        assert!((z(c.distance()) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_parallel_view_sees_behind_the_eye_but_not_past_its_depth_range() {
        let c = parallel();
        let behind = c.eye - c.forward() * 5.0;
        assert!(c.project(behind).is_some());
        let (_, far) = c.depth_range();
        assert!(c.project(c.eye + c.forward() * (far + 1.0)).is_none());
        assert!(c.project(c.eye - c.forward() * (far + 1.0)).is_none());
    }

    #[test]
    fn a_parallel_view_keeps_sizes_and_the_view_height() {
        let c = parallel();
        let size = c.pixel_size(c.target);
        assert!((size - c.view_height() / 600.0).abs() < 1e-12);
        assert_eq!(c.pixel_size(c.eye + c.forward() * 1000.0), size);
        assert_eq!(c.eye_distance(DVec3::new(100.0, 0.0, 0.0)), c.distance());
        // One meter across the view is as many pixels near as far.
        let span = |p: DVec3| c.project(p + c.right()).unwrap().x - c.project(p).unwrap().x;
        assert!((span(c.target) - span(c.target + c.forward() * 30.0)).abs() < 1e-6);
        assert!((span(c.target) - 1.0 / size).abs() < 1e-6);
    }

    #[test]
    fn parallel_zoom_and_pan_keep_the_anchor_under_the_cursor() {
        let mut c = parallel();
        let anchor = DVec3::new(-1.0, 3.0, 0.0);
        let px = c.project(anchor).unwrap();
        let height = c.view_height();
        c.zoom(anchor, 2.0);
        assert!(c.project(anchor).unwrap().abs_diff_eq(px, 1e-6));
        assert!((c.view_height() - height * ZOOM_STEP.powi(2)).abs() < 1e-9);
        let delta = DVec2::new(35.0, -20.0);
        c.pan(anchor, delta);
        assert!(c.project(anchor).unwrap().abs_diff_eq(px + delta, 1e-6));
        c.zoom(anchor, 1000.0);
        assert!((c.distance() - MIN_ZOOM_DISTANCE).abs() < 1e-9);
    }

    #[test]
    fn parallel_anchors_are_the_ground_or_the_target_depth() {
        let c = parallel();
        assert!(
            c.anchor_at(DVec2::new(400.0, 300.0))
                .abs_diff_eq(DVec3::ZERO, 1e-9)
        );
        let mut level = parallel();
        level.eye = DVec3::new(0.0, -10.0, 1.0);
        level.target = DVec3::new(0.0, 0.0, 1.0);
        let sky = level.anchor_at(DVec2::new(400.0, 0.0));
        assert!((level.depth(sky) - level.distance()).abs() < 1e-9);
    }

    #[test]
    fn parallel_line_spans_stay_in_the_depth_range() {
        let c = parallel();
        let [a, b] = c.line_span(c.target, c.right()).unwrap();
        assert!(c.project(a).is_some() && c.project(b).is_some());
        let [a, b] = c.line_span(c.eye, c.forward()).unwrap();
        assert!(c.sees_depth(a) && c.sees_depth(b));
        let (near, far) = c.depth_range();
        assert!(c.depth(a).min(c.depth(b)) < near + (far - near) * 0.01);
        let beyond = c.eye + c.forward() * (far * 2.0);
        assert!(c.line_span(beyond, c.right()).is_none());
    }

    #[test]
    fn a_top_view_has_north_up_and_east_right() {
        let c = top();
        assert!(c.is_vertical());
        assert!(c.right().abs_diff_eq(DVec3::X, 1e-12) && c.up().abs_diff_eq(DVec3::Y, 1e-12));
        let center = DVec2::new(400.0, 300.0);
        assert!(c.project(c.target + DVec3::X).unwrap().x > center.x);
        assert!(c.project(c.target + DVec3::Y).unwrap().y < center.y);
        let mut bottom = cam();
        bottom.look_from(StandardView::Bottom);
        assert!(
            bottom.right().abs_diff_eq(-DVec3::X, 1e-12),
            "seen from below, mirrored"
        );
    }

    #[test]
    fn orbiting_a_top_view_spins_it_then_tilts_it_toward_the_top_of_the_screen() {
        let mut c = top();
        let d = c.distance();
        c.orbit(c.target, DVec2::new(0.0, 100.0));
        assert!(c.is_vertical(), "already as high as it goes");
        c.orbit(c.target, DVec2::new(-100.0, 0.0));
        assert!(c.is_vertical());
        let spun = DQuat::from_axis_angle(UP, 100.0 * ORBIT_RAD_PER_PX) * DVec3::Y;
        assert!(c.up.abs_diff_eq(spun, 1e-12));
        let right = c.right();
        c.orbit(c.target, DVec2::new(0.0, -100.0));
        assert!(!c.is_vertical() && c.up == UP);
        assert!(
            c.right().abs_diff_eq(right, 1e-9),
            "no flip on leaving the vertical"
        );
        assert!((c.distance() - d).abs() < 1e-9);
    }

    #[test]
    fn fit_frames_a_box_from_the_current_direction() {
        let mut c = cam();
        let forward = c.forward();
        let (lo, hi) = (DVec3::new(-5.0, 0.0, 0.0), DVec3::new(5.0, 2.0, 3.0));
        c.fit(lo, hi);
        assert!(c.forward().abs_diff_eq(forward, 1e-12));
        assert!(c.target.abs_diff_eq((lo + hi) * 0.5, 1e-12));
        let mut p = parallel();
        p.look_from(StandardView::Front);
        p.fit(lo, hi);
        // 10 m wide at aspect 4:3 needs 7.5 m of height; 3 m tall needs less.
        assert!((p.view_height() - 7.5 * FIT_MARGIN).abs() < 1e-9);
        let mut point = cam();
        point.fit(DVec3::ONE, DVec3::ONE);
        assert!((point.distance() - MIN_ZOOM_DISTANCE).abs() < 1e-12);
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
