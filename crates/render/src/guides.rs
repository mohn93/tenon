//! Guide lines and guide points, drawn anew each frame for the camera: a guide line is
//! infinite, so only the part in view is drawn, dashed; a guide point is a small cross of
//! constant size on screen.

use glam::{DVec2, DVec3};
use session::{Camera, Ray};

use crate::grid::LineVertex;

pub const GUIDE_COLOR: [f32; 4] = [0.25, 0.25, 0.32, 1.0];
/// Guides outside the open instance, while one is open.
pub const FADED_GUIDE_COLOR: [f32; 4] = [0.68, 0.68, 0.72, 1.0];

/// Length of a dash and of the gap after it, in physical pixels.
const DASH_PX: f64 = 8.0;
/// Half the size of a guide point's cross, in physical pixels.
const CROSS_PX: f64 = 5.0;
/// How far beyond the viewport (physical pixels) dashes are still drawn.
const MARGIN_PX: f64 = 16.0;

/// One guide placed in root coordinates: a line through `point` along the unit `direction`,
/// or, without a direction, a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideShape {
    pub point: DVec3,
    pub direction: Option<DVec3>,
    pub color: [f32; 4],
}

/// Line pairs drawing `guides` as `camera` sees them.
pub fn guide_lines(guides: &[GuideShape], camera: &Camera) -> Vec<LineVertex> {
    let mut out = Vec::new();
    let mut push = |a: DVec3, b: DVec3, color: [f32; 4]| {
        for p in [a, b] {
            out.push(LineVertex {
                pos: p.as_vec3().to_array(),
                color,
            });
        }
    };
    for g in guides {
        match g.direction {
            Some(dir) => {
                for [a, b] in dashes(camera, g.point, dir) {
                    push(a, b, g.color);
                }
            }
            None => {
                if !camera.sees_depth(g.point) {
                    continue;
                }
                let size = CROSS_PX * camera.pixel_size(g.point);
                let (r, u) = (camera.right() * size, camera.up() * size);
                push(g.point - r - u, g.point + r + u, g.color);
                push(g.point - r + u, g.point + r - u, g.color);
            }
        }
    }
    out
}

/// Dashes along the part of the line through `point` along `dir` that is on screen, each
/// `DASH_PX` long on screen, as world segments on the line.
fn dashes(camera: &Camera, point: DVec3, dir: DVec3) -> Vec<[DVec3; 2]> {
    let Some([a, b]) = camera.line_span(point, dir) else {
        return Vec::new();
    };
    let (Some(sa), Some(sb)) = (camera.project(a), camera.project(b)) else {
        return Vec::new();
    };
    let lo = DVec2::splat(-MARGIN_PX);
    let hi = camera.viewport + MARGIN_PX;
    let Some((sa, sb)) = clip(sa, sb, lo, hi) else {
        return Vec::new();
    };
    let n = (sa.distance(sb) / DASH_PX).ceil().max(1.0) as usize;
    let on_line = |s: DVec2| closest_on_line(point, dir, &camera.ray(s));
    (0..n)
        .step_by(2)
        .map(|k| {
            let at = |k: usize| sa.lerp(sb, k.min(n) as f64 / n as f64);
            [on_line(at(k)), on_line(at(k + 1))]
        })
        .collect()
}

/// The part of the 2D segment p-q inside the box `lo`..`hi` (Liang-Barsky), if any.
fn clip(p: DVec2, q: DVec2, lo: DVec2, hi: DVec2) -> Option<(DVec2, DVec2)> {
    let d = q - p;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (dp, from_lo, to_hi) in [(d.x, p.x - lo.x, hi.x - p.x), (d.y, p.y - lo.y, hi.y - p.y)] {
        for (num, den) in [(from_lo, -dp), (to_hi, dp)] {
            if den == 0.0 {
                if num < 0.0 {
                    return None;
                }
            } else {
                let t = num / den;
                if den < 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
            }
        }
    }
    (t0 < t1).then(|| (p + d * t0, p + d * t1))
}

/// The point on the line `origin + t * dir` closest to the ray.
fn closest_on_line(origin: DVec3, dir: DVec3, ray: &Ray) -> DVec3 {
    let w0 = origin - ray.origin;
    let (a, b, c) = (dir.dot(dir), dir.dot(ray.dir), ray.dir.dot(ray.dir));
    let (d, e) = (dir.dot(w0), ray.dir.dot(w0));
    let denom = a * c - b * b;
    let t = if denom.abs() < 1e-12 * a * c {
        0.0
    } else {
        (b * e - c * d) / denom
    };
    origin + dir * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    #[test]
    fn a_guide_line_is_dashed_across_the_view_and_stays_on_its_line() {
        let c = cam();
        let line = GuideShape {
            point: DVec3::new(0.0, 1.0, 0.0),
            direction: Some(DVec3::X),
            color: GUIDE_COLOR,
        };
        let v = guide_lines(&[line], &c);
        assert!(v.len() > 40, "{} vertices", v.len());
        for p in &v {
            assert!((p.pos[1] - 1.0).abs() < 1e-5 && p.pos[2].abs() < 1e-5);
        }
        // Each dash is about DASH_PX long on screen.
        let s = |k: usize| c.project(DVec3::from(v[k].pos.map(f64::from))).unwrap();
        let len = s(0).distance(s(1));
        assert!((len - DASH_PX).abs() < 0.5, "{len}");
    }

    #[test]
    fn guide_points_are_crosses_and_guides_out_of_view_are_skipped() {
        let c = cam();
        let point = GuideShape {
            point: DVec3::new(1.0, 1.0, 0.0),
            direction: None,
            color: GUIDE_COLOR,
        };
        let v = guide_lines(&[point], &c);
        assert_eq!(v.len(), 4);
        let s = |k: usize| c.project(DVec3::from(v[k].pos.map(f64::from))).unwrap();
        let diagonal = 2.0 * CROSS_PX * 2f64.sqrt();
        assert!((s(0).distance(s(1)) - diagonal).abs() < 0.5);
        let behind = GuideShape {
            point: c.eye - c.forward() * 5.0,
            ..point
        };
        assert!(guide_lines(&[behind], &c).is_empty());
        let off_screen = GuideShape {
            point: c.target + c.up() * c.distance() * 10.0,
            direction: Some(c.right()),
            color: GUIDE_COLOR,
        };
        assert!(guide_lines(&[off_screen], &c).is_empty());
    }

    #[test]
    fn parallel_views_draw_guide_points_behind_the_eye_at_full_size() {
        let mut c = cam();
        c.projection = session::Projection::Parallel;
        let behind = GuideShape {
            point: c.eye - c.forward() * 5.0,
            direction: None,
            color: GUIDE_COLOR,
        };
        let v = guide_lines(&[behind], &c);
        assert_eq!(v.len(), 4);
        let s = |k: usize| c.project(DVec3::from(v[k].pos.map(f64::from))).unwrap();
        assert!((s(0).distance(s(1)) - 2.0 * CROSS_PX * 2f64.sqrt()).abs() < 0.5);
    }
}
