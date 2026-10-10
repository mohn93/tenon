//! What tools draw on top of the scene: the preview line and the inference marker.

use glam::DVec3;
use session::Camera;

use crate::inference::{Axis, Inference, SnapKind};

pub type Color = [f32; 4];

pub const PREVIEW: Color = [0.1, 0.1, 0.1, 1.0];
pub const RED: Color = [0.85, 0.15, 0.15, 1.0];
pub const GREEN: Color = [0.15, 0.65, 0.20, 1.0];
pub const BLUE: Color = [0.15, 0.30, 0.85, 1.0];
pub const ENDPOINT: Color = [0.10, 0.60, 0.10, 1.0];
pub const MIDPOINT: Color = [0.10, 0.70, 0.80, 1.0];
pub const ON_EDGE: Color = [0.85, 0.10, 0.10, 1.0];
pub const ON_FACE: Color = [0.20, 0.25, 0.85, 1.0];

/// Marker half-size in logical pixels.
const MARKER_PX: f64 = 5.0;

/// A colored world-space segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub a: DVec3,
    pub b: DVec3,
    pub color: Color,
}

/// Preview segments (axis-colored when following an axis) plus a marker at the inference point
/// sized to stay about the same on screen (`scale_factor` converts logical to physical pixels).
pub fn overlay(
    preview: &[[DVec3; 2]],
    hit: Option<&Inference>,
    camera: &Camera,
    scale_factor: f64,
) -> Vec<Segment> {
    let line_color = match hit.map(|h| h.kind) {
        Some(SnapKind::Axis(Axis::X)) => RED,
        Some(SnapKind::Axis(Axis::Y)) => GREEN,
        Some(SnapKind::Axis(Axis::Z)) => BLUE,
        _ => PREVIEW,
    };
    let mut out: Vec<Segment> = preview
        .iter()
        .map(|[a, b]| Segment {
            a: *a,
            b: *b,
            color: line_color,
        })
        .collect();

    let Some(hit) = hit else { return out };
    let color = match hit.kind {
        SnapKind::Endpoint | SnapKind::Origin => ENDPOINT,
        SnapKind::Midpoint => MIDPOINT,
        SnapKind::OnEdge => ON_EDGE,
        SnapKind::OnFace => ON_FACE,
        _ => return out,
    };
    let depth = (hit.point - camera.eye).dot(camera.forward()).max(1e-6);
    let size =
        MARKER_PX * scale_factor * 2.0 * depth * (camera.fov_y * 0.5).tan() / camera.viewport.y;
    let (r, u) = (camera.right() * size, camera.up() * size);
    let p = hit.point;
    let corners = [p - r - u, p + r - u, p + r + u, p - r + u];
    for i in 0..4 {
        out.push(Segment {
            a: corners[i],
            b: corners[(i + 1) % 4],
            color,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    #[test]
    fn preview_takes_the_axis_color() {
        let hit = Inference {
            point: DVec3::new(0.0, 3.0, 0.0),
            kind: SnapKind::Axis(Axis::Y),
            face: None,
        };
        let segs = overlay(&[[DVec3::ZERO, hit.point]], Some(&hit), &cam(), 1.0);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].color, GREEN);
    }

    #[test]
    fn endpoint_marker_is_a_square_of_constant_screen_size() {
        let c = cam();
        for p in [DVec3::new(1.0, 0.0, 0.0), DVec3::new(30.0, 40.0, 0.0)] {
            let hit = Inference {
                point: p,
                kind: SnapKind::Endpoint,
                face: None,
            };
            let segs = overlay(&[], Some(&hit), &c, 1.0);
            assert_eq!(segs.len(), 4);
            assert!(segs.iter().all(|s| s.color == ENDPOINT));
            let side_px = c
                .project(segs[0].a)
                .unwrap()
                .distance(c.project(segs[0].b).unwrap());
            assert!(
                (side_px - 2.0 * MARKER_PX).abs() < 0.5,
                "marker side {side_px}px"
            );
        }
    }

    #[test]
    fn marker_scales_with_the_display() {
        let c = cam();
        let hit = Inference {
            point: DVec3::new(1.0, 0.0, 0.0),
            kind: SnapKind::Endpoint,
            face: None,
        };
        let side = |scale: f64| {
            let segs = overlay(&[], Some(&hit), &c, scale);
            c.project(segs[0].a)
                .unwrap()
                .distance(c.project(segs[0].b).unwrap())
        };
        assert!((side(2.0) - 2.0 * side(1.0)).abs() < 0.5);
        assert!((side(2.0) - 4.0 * MARKER_PX).abs() < 0.5);
    }

    #[test]
    fn ground_hits_have_no_marker() {
        let hit = Inference {
            point: DVec3::ZERO,
            kind: SnapKind::Ground,
            face: None,
        };
        assert!(overlay(&[], Some(&hit), &cam(), 1.0).is_empty());
    }
}
