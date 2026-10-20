//! Dimensions and leader notes, drawn anew each frame for the camera like guides: their lines
//! are fixed in the model, but their arrows and the extension lines' overshoot are sized in
//! pixels. Labels are the window's (egui); screen notes have no lines.

use glam::{DVec2, DVec3};
use session::{AnnotationShape, Camera};

use crate::grid::LineVertex;

pub const ANNOTATION_COLOR: [f32; 4] = [0.12, 0.12, 0.16, 1.0];
/// Annotations outside the open instance, while one is open.
pub const FADED_ANNOTATION_COLOR: [f32; 4] = [0.66, 0.66, 0.70, 1.0];

/// How long an arrowhead's sides are, in physical pixels.
pub const ARROW_PX: f64 = 10.0;
/// How far an extension line goes past the dimension line, in physical pixels.
pub const OVERSHOOT_PX: f64 = 4.0;
/// Half the angle at an arrow's tip.
const ARROW_HALF_ANGLE: f64 = 0.35;

/// One annotation placed in root coordinates, in its color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnnotationLook {
    pub shape: AnnotationShape,
    pub color: [f32; 4],
}

/// Line pairs drawing `annotations` as `camera` sees them.
pub fn annotation_lines(annotations: &[AnnotationLook], camera: &Camera) -> Vec<LineVertex> {
    let mut out = Vec::new();
    for look in annotations {
        for [a, b] in annotation_segments(&look.shape, camera) {
            for p in [a, b] {
                out.push(LineVertex {
                    pos: p.as_vec3().to_array(),
                    color: look.color,
                });
            }
        }
    }
    out
}

/// The segments drawing `shape` for `camera`: its lines (see `AnnotationShape::segments`),
/// the extension lines' overshoot past the dimension line, and arrowheads `ARROW_PX` long at
/// the dimension line's ends, where a radial line meets the circle, and at a leader's anchor.
pub fn annotation_segments(shape: &AnnotationShape, camera: &Camera) -> Vec<[DVec3; 2]> {
    let mut out = shape.segments();
    match *shape {
        AnnotationShape::Linear { a, b, line } => {
            for (from, to) in [(a, line[0]), (b, line[1])] {
                if let Some(dir) = (to - from).try_normalize()
                    && from.distance(to) > kernel::geom::LENGTH_TOL
                    && camera.sees_depth(to)
                {
                    // How much of a unit along `dir` the screen shows, so the overshoot is
                    // OVERSHOOT_PX long however the line is foreshortened.
                    let across = DVec2::new(dir.dot(camera.right()), dir.dot(camera.up())).length();
                    if across > 0.05 {
                        let len = OVERSHOOT_PX * camera.pixel_size(to) / across;
                        out.push([to, to + dir * len]);
                    }
                }
            }
            out.extend(arrow(camera, line[0], line[0] - line[1]));
            out.extend(arrow(camera, line[1], line[1] - line[0]));
        }
        AnnotationShape::Radial {
            circle,
            label,
            diameter,
        } => {
            let near = AnnotationShape::radial_point(&circle, label);
            let out_dir = near - circle.center;
            out.extend(arrow(camera, near, out_dir));
            if diameter {
                out.extend(arrow(camera, circle.center - out_dir, -out_dir));
            }
        }
        AnnotationShape::Leader { anchor, position } => {
            out.extend(arrow(camera, anchor, anchor - position));
        }
        AnnotationShape::Screen { .. } => {}
    }
    out
}

/// An arrowhead with its tip at `tip`, pointing along `dir` as the screen shows it: two
/// strokes `ARROW_PX` long back from the tip. None where the camera does not see the tip, or
/// `dir` points along the view.
fn arrow(camera: &Camera, tip: DVec3, dir: DVec3) -> Vec<[DVec3; 2]> {
    if !camera.sees_depth(tip) {
        return Vec::new();
    }
    let (right, up) = (camera.right(), camera.up());
    let Some(d) = DVec2::new(dir.dot(right), dir.dot(up)).try_normalize() else {
        return Vec::new();
    };
    let size = ARROW_PX * camera.pixel_size(tip);
    [ARROW_HALF_ANGLE, -ARROW_HALF_ANGLE]
        .map(|angle| {
            let back = DVec2::from_angle(angle).rotate(-d);
            [tip, tip + (right * back.x + up * back.y) * size]
        })
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::Circle;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    fn screen(c: &Camera, p: DVec3) -> DVec2 {
        c.project(p).unwrap()
    }

    #[test]
    fn a_linear_dimension_has_extension_lines_with_overshoot_and_arrows_in_pixels() {
        let c = cam();
        let (a, b) = (DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0));
        let shift = DVec3::new(0.0, -0.5, 0.0);
        let shape = AnnotationShape::Linear {
            a,
            b,
            line: [a + shift, b + shift],
        };
        let s = annotation_segments(&shape, &c);
        // Two extension lines, the dimension line, two overshoots and two arrows of two.
        assert_eq!(s.len(), 3 + 2 + 4);
        assert_eq!(s[0], [a, a + shift]);
        assert_eq!(s[2], [a + shift, b + shift]);
        let over = s[3];
        assert_eq!(over[0], a + shift);
        let len = screen(&c, over[0]).distance(screen(&c, over[1]));
        assert!((len - OVERSHOOT_PX).abs() < 1.0, "overshoot {len} px");
        for arrow in &s[5..] {
            let len = screen(&c, arrow[0]).distance(screen(&c, arrow[1]));
            assert!((len - ARROW_PX).abs() < 0.5, "arrow {len} px");
        }
        // The arrows at a's end point back along the line toward b.
        let tip = screen(&c, a + shift);
        let wing = screen(&c, s[5][1]);
        let toward_b = screen(&c, b + shift) - tip;
        assert!((wing - tip).dot(toward_b) > 0.0);
        let v = annotation_lines(
            &[AnnotationLook {
                shape,
                color: ANNOTATION_COLOR,
            }],
            &c,
        );
        assert_eq!(v.len(), 2 * s.len());
    }

    #[test]
    fn a_radial_dimension_has_an_arrow_at_the_circle_and_a_diameter_two() {
        let c = cam();
        let circle = Circle {
            center: DVec3::ZERO,
            normal: DVec3::Z,
            radius: 1.0,
        };
        let label = DVec3::new(2.0, 0.0, 0.0);
        let radius = AnnotationShape::Radial {
            circle,
            label,
            diameter: false,
        };
        let s = annotation_segments(&radius, &c);
        assert_eq!(s.len(), 2 + 2);
        assert!(s[2..].iter().all(|w| w[0] == DVec3::X));
        let diameter = AnnotationShape::Radial {
            circle,
            label,
            diameter: true,
        };
        let s = annotation_segments(&diameter, &c);
        assert_eq!(s.len(), 2 + 4);
        assert_eq!(s[0], [-DVec3::X, DVec3::X]);
        assert!(s[4..].iter().all(|w| w[0] == -DVec3::X));
    }

    #[test]
    fn a_leader_points_at_its_anchor_and_screen_notes_draw_nothing() {
        let c = cam();
        let leader = AnnotationShape::Leader {
            anchor: DVec3::ZERO,
            position: DVec3::new(1.0, 1.0, 1.0),
        };
        let s = annotation_segments(&leader, &c);
        assert_eq!(s.len(), 3);
        assert!(s[1..].iter().all(|w| w[0] == DVec3::ZERO));
        assert!(annotation_segments(&AnnotationShape::Screen { at: [0.1, 0.1] }, &c).is_empty());
        // Behind the eye: the leader's line is kept (the GPU clips it), the arrow is not.
        let behind = AnnotationShape::Leader {
            anchor: c.eye - c.forward() * 5.0,
            position: c.target,
        };
        assert_eq!(annotation_segments(&behind, &c).len(), 1);
    }
}
