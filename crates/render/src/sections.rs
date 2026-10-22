//! Section planes in the picture: clipping triangles and edges at the active planes, the fill
//! and the cut edges a cut leaves, and the plane entities' outlines.
//!
//! The clipping works on world coordinates, one placement at a time (see `scene::Walk`). A
//! plane keeps what `SectionPlane::keeps` keeps, so faces lying in the plane stay.

use glam::DVec3;
use kernel::{SectionPlane, section_loops, triangulate_section};
use session::{Camera, SectionShape};

use crate::annotations::{ARROW_PT, arrow};
use crate::grid::LineVertex;

/// The fill of closed cuts: #3a3a3a, whatever the theme.
pub const CUT_FILL_COLOR: [f32; 4] = [0.227, 0.227, 0.227, 1.0];

/// How far a plane's arrows reach from its sides, in points.
pub const SECTION_ARROW_PT: f64 = 3.0 * ARROW_PT;

/// How far from the plane a point may be on the cut-away side and still be kept (as
/// `SectionPlane::keeps`).
const KEEP_TOL: f64 = 1e-9;

/// A section plane placed in root coordinates for drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionLook {
    pub shape: SectionShape,
    pub color: [f32; 4],
    /// The plane is its context's active one, drawn in the style's accent.
    pub active: bool,
}

fn distance(plane: &SectionPlane, p: DVec3) -> f64 {
    (p - plane.point).dot(plane.normal)
}

/// The part of the convex polygon `poly` that `plane` keeps; `mix(a, b, t)` gives the
/// attributes `t` of the way from corner a to corner b. Fewer than three corners left means
/// nothing is kept.
pub fn clip_polygon<A: Copy>(
    poly: &[(DVec3, A)],
    plane: &SectionPlane,
    mix: &impl Fn(A, A, f64) -> A,
) -> Vec<(DVec3, A)> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (k, &(p, a)) in poly.iter().enumerate() {
        let (q, b) = poly[(k + 1) % poly.len()];
        let (dp, dq) = (distance(plane, p), distance(plane, q));
        let (kept_p, kept_q) = (dp >= -KEEP_TOL, dq >= -KEEP_TOL);
        if kept_p {
            out.push((p, a));
        }
        if kept_p != kept_q {
            let t = dp / (dp - dq);
            out.push((p.lerp(q, t), mix(a, b, t)));
        }
    }
    out
}

/// The triangles that remain of `tri` after every plane of `planes` cut it: none, the
/// triangle itself, or the pieces of the kept polygon fanned out.
pub fn clip_triangle<A: Copy>(
    tri: [(DVec3, A); 3],
    planes: &[SectionPlane],
    mix: &impl Fn(A, A, f64) -> A,
) -> Vec<[(DVec3, A); 3]> {
    let mut poly = tri.to_vec();
    for plane in planes {
        if poly.iter().all(|(p, _)| distance(plane, *p) >= -KEEP_TOL) {
            continue;
        }
        poly = clip_polygon(&poly, plane, mix);
        if poly.len() < 3 {
            return Vec::new();
        }
    }
    (1..poly.len() - 1)
        .map(|k| [poly[0], poly[k], poly[k + 1]])
        .collect()
}

/// The part of the segment a-b that every plane keeps.
pub fn clip_segment(a: DVec3, b: DVec3, planes: &[SectionPlane]) -> Option<[DVec3; 2]> {
    let (mut a, mut b) = (a, b);
    for plane in planes {
        let (da, db) = (distance(plane, a), distance(plane, b));
        let (kept_a, kept_b) = (da >= -KEEP_TOL, db >= -KEEP_TOL);
        match (kept_a, kept_b) {
            (true, true) => {}
            (false, false) => return None,
            (true, false) => b = a.lerp(b, da / (da - db)),
            (false, true) => a = a.lerp(b, da / (da - db)),
        }
    }
    Some([a, b])
}

/// What the planes of a placement cut out of its surface, in world coordinates.
#[derive(Debug, Default, PartialEq)]
pub struct Cut {
    /// The triangles filling the closed cuts, each inside every other plane's kept side.
    pub fill: Vec<[DVec3; 3]>,
    /// The segments of the cut outlines, closed loops and open chains alike.
    pub edges: Vec<[DVec3; 2]>,
}

/// The fill (when `fill`) and the cut edges where each of `planes` cuts `triangles`, the
/// surface of one placement, before any plane clipped it; each plane's cut is clipped by the
/// others.
pub fn cut_surface(triangles: &[[DVec3; 3]], planes: &[SectionPlane], fill: bool) -> Cut {
    let mut cut = Cut::default();
    for (k, plane) in planes.iter().enumerate() {
        let others: Vec<SectionPlane> = planes
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != k)
            .map(|(_, p)| *p)
            .collect();
        // A plane that cuts nothing away leaves nothing to outline.
        if triangles
            .iter()
            .flatten()
            .all(|p| distance(plane, *p) >= -KEEP_TOL)
        {
            continue;
        }
        let loops = section_loops(triangles, plane.point, plane.normal);
        if fill {
            for t in triangulate_section(&loops.closed, plane.normal) {
                let tri = t.map(|p| (p, ()));
                for piece in clip_triangle(tri, &others, &|_, _, _| ()) {
                    cut.fill.push(piece.map(|(p, ())| p));
                }
            }
        }
        let mut outline = |points: &[DVec3], closed: bool| {
            let n = if closed {
                points.len()
            } else {
                points.len() - 1
            };
            for j in 0..n {
                let (a, b) = (points[j], points[(j + 1) % points.len()]);
                if let Some(s) = clip_segment(a, b, &others) {
                    cut.edges.push(s);
                }
            }
        };
        for region in &loops.closed {
            for ring in region {
                outline(ring, true);
            }
        }
        for chain in &loops.open {
            outline(chain, false);
        }
    }
    cut
}

/// Line pairs drawing `looks` as `camera` sees them, on a display of `pixels_per_point`:
/// each outline, and an arrow `SECTION_ARROW_PT` long from the middle of each side along the
/// normal, toward the kept side.
pub fn section_lines(
    looks: &[SectionLook],
    camera: &Camera,
    pixels_per_point: f64,
    out: &mut Vec<LineVertex>,
) {
    let mut push = |[a, b]: [DVec3; 2], color: [f32; 4]| {
        for p in [a, b] {
            out.push(LineVertex {
                pos: p.as_vec3().to_array(),
                color,
            });
        }
    };
    for look in looks {
        let s = &look.shape;
        for seg in s.segments() {
            push(seg, look.color);
        }
        let head = ARROW_PT * pixels_per_point;
        for base in s.arrow_bases() {
            if !camera.sees_depth(base) {
                continue;
            }
            let len = SECTION_ARROW_PT * pixels_per_point * camera.pixel_size(base);
            let tip = base + s.normal * len;
            push([base, tip], look.color);
            arrow(camera, tip, s.normal, head, &mut |seg| {
                push(seg, look.color)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;

    fn plane_x(at: f64) -> SectionPlane {
        SectionPlane::new(DVec3::new(at, 0.0, 0.0), DVec3::X)
    }

    fn mix(a: f64, b: f64, t: f64) -> f64 {
        a + (b - a) * t
    }

    fn tri(xs: [f64; 3]) -> [(DVec3, f64); 3] {
        [
            (DVec3::new(xs[0], 0.0, 0.0), 0.0),
            (DVec3::new(xs[1], 1.0, 0.0), 1.0),
            (DVec3::new(xs[2], 0.0, 1.0), 2.0),
        ]
    }

    fn area(t: &[(DVec3, f64); 3]) -> f64 {
        (t[1].0 - t[0].0).cross(t[2].0 - t[0].0).length() / 2.0
    }

    #[test]
    fn a_triangle_on_the_kept_side_is_unchanged_and_one_on_the_other_is_gone() {
        let kept = tri([1.0, 2.0, 3.0]);
        assert_eq!(clip_triangle(kept, &[plane_x(0.0)], &mix), vec![kept]);
        let cut = tri([-1.0, -2.0, -3.0]);
        assert!(clip_triangle(cut, &[plane_x(0.0)], &mix).is_empty());
        // Touching the plane along an edge or at a corner keeps (or drops) it whole.
        let touching = tri([0.0, 0.0, 1.0]);
        assert_eq!(
            clip_triangle(touching, &[plane_x(0.0)], &mix),
            vec![touching]
        );
        let corner = tri([0.0, -1.0, -1.0]);
        let pieces = clip_triangle(corner, &[plane_x(0.0)], &mix);
        assert!(pieces.iter().all(|t| area(t) < 1e-12), "{pieces:?}");
    }

    #[test]
    fn a_straddling_triangle_is_split_and_attributes_follow() {
        // Corners at x = -1, 1, 1: the plane x = 0 cuts it half way along two sides.
        let t = tri([-1.0, 1.0, 1.0]);
        let pieces = clip_triangle(t, &[plane_x(0.0)], &mix);
        assert_eq!(pieces.len(), 2, "a quadrilateral fans into two triangles");
        let total: f64 = pieces.iter().map(area).sum();
        assert!((total - 0.75 * area(&t)).abs() < 1e-12, "{total}");
        for piece in &pieces {
            for (p, a) in piece {
                assert!(p.x >= -1e-12);
                // The attribute is the corner index interpolated along the sides.
                if (p.x - 0.0).abs() < 1e-12 {
                    assert!(*a > 0.0 && *a < 2.0, "{a}");
                }
            }
        }
        // One corner kept: a smaller triangle.
        let one = tri([1.0, -1.0, -1.0]);
        let pieces = clip_triangle(one, &[plane_x(0.0)], &mix);
        assert_eq!(pieces.len(), 1);
        assert!((area(&pieces[0]) - area(&one) / 4.0).abs() < 1e-12);
    }

    #[test]
    fn two_planes_cut_a_triangle_by_both() {
        let t = [
            (DVec3::new(-2.0, -2.0, 0.0), ()),
            (DVec3::new(2.0, -2.0, 0.0), ()),
            (DVec3::new(0.0, 2.0, 0.0), ()),
        ];
        let planes = [
            plane_x(0.0),
            SectionPlane::new(DVec3::new(0.0, -1.0, 0.0), DVec3::Y),
        ];
        let pieces = clip_triangle(t, &planes, &|_, _, _| ());
        assert!(!pieces.is_empty());
        for piece in &pieces {
            for (p, ()) in piece {
                assert!(p.x >= -1e-12 && p.y >= -1.0 - 1e-12, "{p}");
            }
        }
    }

    #[test]
    fn segments_are_cut_at_the_plane() {
        let a = DVec3::new(-1.0, 0.0, 0.0);
        let b = DVec3::new(3.0, 0.0, 0.0);
        let p = [plane_x(0.0)];
        assert_eq!(clip_segment(a, b, &p), Some([DVec3::ZERO, b]));
        assert_eq!(clip_segment(b, a, &p), Some([b, DVec3::ZERO]));
        assert_eq!(clip_segment(a, DVec3::new(-0.5, 0.0, 0.0), &p), None);
        // Lying in the plane keeps it.
        let in_plane = [DVec3::new(0.0, -1.0, 0.0), DVec3::new(0.0, 1.0, 0.0)];
        assert_eq!(clip_segment(in_plane[0], in_plane[1], &p), Some(in_plane));
        // Two planes: the part between x = 0 and x = 2 (the second plane keeps x <= 2).
        let both = [
            plane_x(0.0),
            SectionPlane::new(DVec3::new(2.0, 0.0, 0.0), -DVec3::X),
        ];
        assert_eq!(
            clip_segment(a, b, &both),
            Some([DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0)])
        );
    }

    fn box_triangles() -> Vec<[DVec3; 3]> {
        // A cube [-1, 1]^3 as twelve triangles, wound outward.
        let c = |x: f64, y: f64, z: f64| DVec3::new(x, y, z);
        let quads = [
            [
                c(-1., -1., -1.),
                c(-1., 1., -1.),
                c(1., 1., -1.),
                c(1., -1., -1.),
            ],
            [
                c(-1., -1., 1.),
                c(1., -1., 1.),
                c(1., 1., 1.),
                c(-1., 1., 1.),
            ],
            [
                c(-1., -1., -1.),
                c(1., -1., -1.),
                c(1., -1., 1.),
                c(-1., -1., 1.),
            ],
            [
                c(-1., 1., -1.),
                c(-1., 1., 1.),
                c(1., 1., 1.),
                c(1., 1., -1.),
            ],
            [
                c(-1., -1., -1.),
                c(-1., -1., 1.),
                c(-1., 1., 1.),
                c(-1., 1., -1.),
            ],
            [
                c(1., -1., -1.),
                c(1., 1., -1.),
                c(1., 1., 1.),
                c(1., -1., 1.),
            ],
        ];
        quads
            .iter()
            .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
            .collect()
    }

    #[test]
    fn a_plane_through_a_box_fills_the_square_and_outlines_it() {
        let cut = cut_surface(&box_triangles(), &[plane_x(0.0)], true);
        let fill: f64 = cut
            .fill
            .iter()
            .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.0)
            .sum();
        assert!((fill - 4.0).abs() < 1e-9, "{fill}");
        assert!(cut.fill.iter().flatten().all(|p| p.x.abs() < 1e-9));
        let length: f64 = cut.edges.iter().map(|[a, b]| a.distance(*b)).sum();
        assert!((length - 8.0).abs() < 1e-9, "{length}");
        // Without the fill only the outline remains.
        let outline = cut_surface(&box_triangles(), &[plane_x(0.0)], false);
        assert!(outline.fill.is_empty());
        assert_eq!(outline.edges, cut.edges);
    }

    #[test]
    fn a_second_plane_clips_the_first_planes_cut() {
        let planes = [
            plane_x(0.0),
            SectionPlane::new(DVec3::new(0.0, 0.5, 0.0), -DVec3::Y),
        ];
        let cut = cut_surface(&box_triangles(), &planes, true);
        // The x = 0 square cut to y <= 0.5 is 1.5 x 2; the y = 0.5 square cut to x >= 0 as well.
        let on_x: f64 = cut
            .fill
            .iter()
            .filter(|t| t.iter().all(|p| p.x.abs() < 1e-9))
            .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() / 2.0)
            .sum();
        assert!((on_x - 3.0).abs() < 1e-9, "{on_x}");
        assert!(
            cut.fill
                .iter()
                .flatten()
                .all(|p| p.x >= -1e-9 && p.y <= 0.5 + 1e-9)
        );
    }

    #[test]
    fn an_open_surface_gives_edges_and_no_fill() {
        let one_face = &box_triangles()[..2];
        let cut = cut_surface(one_face, &[plane_x(0.0)], true);
        assert!(cut.fill.is_empty());
        let length: f64 = cut.edges.iter().map(|[a, b]| a.distance(*b)).sum();
        assert!((length - 2.0).abs() < 1e-9, "{length}");
    }

    #[test]
    fn a_plane_draws_its_outline_and_four_arrows_in_points() {
        let cam = Camera::new(DVec2::new(800.0, 600.0));
        let shape = SectionShape::new(&SectionPlane::new(DVec3::ZERO, DVec3::Z), 2.0);
        let look = SectionLook {
            shape,
            color: [1.0; 4],
            active: false,
        };
        let mut v = Vec::new();
        section_lines(&[look], &cam, 1.0, &mut v);
        // Four sides, then four shafts with two strokes each.
        assert_eq!(v.len(), 2 * (4 + 4 * 3));
        let s = |k: usize| cam.project(DVec3::from(v[k].pos.map(f64::from))).unwrap();
        let shaft = s(8).distance(s(9));
        assert!(shaft > 0.5 * SECTION_ARROW_PT, "{shaft}");
        let (mut v2, look2) = (Vec::new(), look);
        section_lines(&[look2], &cam, 2.0, &mut v2);
        let s2 = |k: usize| cam.project(DVec3::from(v2[k].pos.map(f64::from))).unwrap();
        let ratio = s2(8).distance(s2(9)) / shaft;
        assert!((ratio - 2.0).abs() < 0.1, "{ratio}");
    }
}
