//! Clipping at section planes: triangles, polygons and segments cut by the planes a placement
//! sits behind, and the fill and outline a plane cuts out of a surface. Shared by the renderer
//! and the line drawing (`drawing`). A plane keeps what `SectionPlane::keeps` keeps, so faces
//! lying in the plane stay.

use glam::DVec3;
use kernel::{SectionPlane, section_loops, triangulate_section};

/// How far from the plane a point may be on the cut-away side and still be kept (as
/// `SectionPlane::keeps`).
const KEEP_TOL: f64 = 1e-9;

/// Twice the area below which a piece of a clipped triangle is dropped, in square meters.
const MIN_AREA2: f64 = 1e-12;

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
        if dp >= -KEEP_TOL {
            out.push((p, a));
        }
        // A corner on the plane is kept as it is; only a side from clearly beyond it to
        // clearly short of it is cut.
        if (dp > KEEP_TOL && dq < -KEEP_TOL) || (dp < -KEEP_TOL && dq > KEEP_TOL) {
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
    // Only a clipped polygon can have collapsed to a sliver.
    let clipped = poly.len() != 3 || poly.iter().zip(&tri).any(|(a, b)| a.0 != b.0);
    (1..poly.len() - 1)
        .map(|k| [poly[0], poly[k], poly[k + 1]])
        .filter(|t| !clipped || (t[1].0 - t[0].0).cross(t[2].0 - t[0].0).length() > MIN_AREA2)
        .collect()
}

/// The part of the segment a-b that every plane keeps.
pub fn clip_segment(a: DVec3, b: DVec3, planes: &[SectionPlane]) -> Option<[DVec3; 2]> {
    let (mut p, mut q) = (a, b);
    let mut clipped = false;
    for plane in planes {
        let (dp, dq) = (distance(plane, p), distance(plane, q));
        let (kept_p, kept_q) = (dp >= -KEEP_TOL, dq >= -KEEP_TOL);
        match (kept_p, kept_q) {
            (true, true) => {}
            (false, false) => return None,
            (true, false) => {
                // Ending on the plane from the cut-away side leaves nothing but a point.
                if dp <= KEEP_TOL {
                    return None;
                }
                q = p.lerp(q, dp / (dp - dq));
                clipped = true;
            }
            (false, true) => {
                if dq <= KEEP_TOL {
                    return None;
                }
                p = p.lerp(q, dp / (dp - dq));
                clipped = true;
            }
        }
    }
    (!clipped || p.distance(q) > 1e-9).then_some([p, q])
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
