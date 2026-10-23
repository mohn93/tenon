//! Where two faces cross: the segments along which their areas meet, for Intersect Faces.

use glam::{DAffine3, DVec2, DVec3};

use crate::geom::{
    LENGTH_TOL, PLANE_ANGLE_TOL, Plane, point_in_polygon, segment_distance_2d, vector_area,
};
use crate::ids::FaceId;
use crate::index::Shape;
use crate::model::Model;

/// A face as a flat polygon with holes in some common (world) space.
#[derive(Clone, Debug)]
pub struct FaceShape {
    pub outer: Vec<DVec3>,
    pub holes: Vec<Vec<DVec3>>,
    pub plane: Plane,
}

/// How far off a boundary piece the side test looks, in meters.
const SIDE_PROBE: f64 = 10.0 * LENGTH_TOL;

impl Model {
    /// The faces with a triangle whose bounding box meets the box `min`..`max`, in id order.
    pub fn faces_near_box(&self, min: DVec3, max: DVec3) -> Vec<FaceId> {
        let mut ids = Vec::new();
        self.index().near_box(min, max, |s| {
            if let Shape::Triangle(id, [a, b, c]) = s {
                let (lo, hi) = (a.min(*b).min(*c), a.max(*b).max(*c));
                if !(lo.cmpgt(max).any() || hi.cmplt(min).any()) {
                    ids.push(*id);
                }
            }
        });
        ids.sort();
        ids.dedup();
        ids
    }
}

/// The face `face` of `model` with its loops moved by `to_world` (which may scale, mirror or
/// shear), and its plane recomputed from the moved points. None when the face does not exist,
/// has no area after the move, or is not finite.
pub fn face_shape(model: &Model, face: FaceId, to_world: DAffine3) -> Option<FaceShape> {
    let f = model.face(face)?;
    let map = |l: &[crate::ids::VertexId]| -> Option<Vec<DVec3>> {
        l.iter()
            .map(|v| model.position(*v).map(|p| to_world.transform_point3(p)))
            .collect()
    };
    let outer = map(&f.outer)?;
    let holes = f.holes.iter().map(|h| map(h)).collect::<Option<Vec<_>>>()?;
    if outer.len() < 3
        || outer
            .iter()
            .chain(holes.iter().flatten())
            .any(|p| !p.is_finite())
    {
        return None;
    }
    let area = vector_area(&outer);
    let len = area.length();
    if !len.is_finite() || len < 1e-12 {
        return None;
    }
    let centroid = outer.iter().sum::<DVec3>() / outer.len() as f64;
    let plane = Plane::through(centroid, area / len);
    Some(FaceShape {
        outer,
        holes,
        plane,
    })
}

/// The segments along which the areas of `a` and `b` meet (boundaries included), each longer
/// than `LENGTH_TOL`, in a fixed order. Crossing planes give the stretches of their common
/// line inside both faces. Coplanar faces give the boundary pieces of each lying within the
/// other's area, except pieces where the two only touch (the areas on opposite sides). Distinct
/// parallel planes give nothing. Two faces meeting along an edge give that edge.
pub fn face_segments(a: &FaceShape, b: &FaceShape) -> Vec<(DVec3, DVec3)> {
    if a.plane.same_as(&b.plane) {
        return coplanar_segments(a, b);
    }
    let dir = a.plane.normal.cross(b.plane.normal);
    let l2 = dir.length_squared();
    if !l2.is_finite() || dir.length() < PLANE_ANGLE_TOL {
        return Vec::new();
    }
    let origin =
        (b.plane.normal.cross(dir) * a.plane.d + dir.cross(a.plane.normal) * b.plane.d) / l2;
    let dir = dir / l2.sqrt();
    let on_a = line_intervals(&Flat::new(a, &a.plane), &a.plane, origin, dir);
    let on_b = line_intervals(&Flat::new(b, &b.plane), &b.plane, origin, dir);
    meet(&on_a, &on_b)
        .into_iter()
        .filter(|(t0, t1)| t1 - t0 >= LENGTH_TOL)
        .map(|(t0, t1)| (origin + dir * t0, origin + dir * t1))
        .collect()
}

/// A face's loops in the 2D basis of a plane.
struct Flat {
    outer: Vec<DVec2>,
    holes: Vec<Vec<DVec2>>,
}

impl Flat {
    fn new(s: &FaceShape, plane: &Plane) -> Flat {
        let flat = |l: &[DVec3]| l.iter().map(|p| plane.project(*p)).collect::<Vec<_>>();
        Flat {
            outer: flat(&s.outer),
            holes: s.holes.iter().map(|h| flat(h)).collect(),
        }
    }

    fn loops(&self) -> impl Iterator<Item = &Vec<DVec2>> {
        std::iter::once(&self.outer).chain(self.holes.iter())
    }

    fn on_boundary(&self, p: DVec2) -> bool {
        self.loops().any(|l| {
            (0..l.len()).any(|i| segment_distance_2d(p, l[i], l[(i + 1) % l.len()]) < LENGTH_TOL)
        })
    }

    /// Inside the outer loop and outside every hole; the boundary may go either way.
    fn inside_open(&self, p: DVec2) -> bool {
        point_in_polygon(p, &self.outer) && !self.holes.iter().any(|h| point_in_polygon(p, h))
    }

    fn inside_closed(&self, p: DVec2) -> bool {
        self.on_boundary(p) || self.inside_open(p)
    }
}

/// The parameter ranges of the 3D line `origin + t * dir` (unit `dir`, in the plane) inside the
/// face, boundary included, sorted and merged.
fn line_intervals(flat: &Flat, plane: &Plane, origin: DVec3, dir: DVec3) -> Vec<(f64, f64)> {
    let q0 = plane.project(origin);
    let r = plane.project(origin + dir) - q0;
    flat_intervals(flat, q0, r)
}

/// The same in 2D: the ranges of `q0 + t * r` (r of unit length) inside the closed area.
fn flat_intervals(flat: &Flat, q0: DVec2, r: DVec2) -> Vec<(f64, f64)> {
    let param = |x: DVec2| (x - q0).dot(r);
    let mut ts: Vec<f64> = Vec::new();
    for l in flat.loops() {
        for i in 0..l.len() {
            let (p, q) = (l[i], l[(i + 1) % l.len()]);
            let (sp, sq) = (r.perp_dot(p - q0), r.perp_dot(q - q0));
            if sp.abs() < LENGTH_TOL {
                ts.push(param(p));
            }
            if sq.abs() < LENGTH_TOL {
                ts.push(param(q));
            }
            if (sp > LENGTH_TOL && sq < -LENGTH_TOL) || (sp < -LENGTH_TOL && sq > LENGTH_TOL) {
                ts.push(param(p + (q - p) * (sp / (sp - sq))));
            }
        }
    }
    ts.retain(|t| t.is_finite());
    ts.sort_by(f64::total_cmp);
    let mut out: Vec<(f64, f64)> = Vec::new();
    for w in ts.windows(2) {
        let (t0, t1) = (w[0], w[1]);
        if t1 > t0 && flat.inside_closed(q0 + r * ((t0 + t1) * 0.5)) {
            match out.last_mut() {
                Some(last) if last.1 >= t0 => last.1 = t1,
                _ => out.push((t0, t1)),
            }
        }
    }
    out
}

/// The overlaps of two sorted interval lists.
fn meet(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if hi > lo {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn coplanar_segments(a: &FaceShape, b: &FaceShape) -> Vec<(DVec3, DVec3)> {
    let (fa, fb) = (Flat::new(a, &a.plane), Flat::new(b, &a.plane));
    let mut out = Vec::new();
    boundary_inside(a, &fa, &fb, &mut out);
    boundary_inside(b, &fb, &fa, &mut out);
    out
}

/// Adds the pieces of `own`'s boundary that lie in `other`'s closed area, leaving out pieces
/// along `other`'s boundary where the two areas lie on opposite sides, and pieces already in
/// `out`.
fn boundary_inside(own: &FaceShape, own2: &Flat, other2: &Flat, out: &mut Vec<(DVec3, DVec3)>) {
    let loops3 = std::iter::once(&own.outer).chain(own.holes.iter());
    for (l3, l2) in loops3.zip(own2.loops()) {
        for i in 0..l3.len() {
            let j = (i + 1) % l3.len();
            let (p, q) = (l3[i], l3[j]);
            let (p2, q2) = (l2[i], l2[j]);
            let len = p2.distance(q2);
            if len < LENGTH_TOL {
                continue;
            }
            let r = (q2 - p2) / len;
            for (t0, t1) in meet(&flat_intervals(other2, p2, r), &[(0.0, len)]) {
                if t1 - t0 < LENGTH_TOL {
                    continue;
                }
                let mid = p2 + r * ((t0 + t1) * 0.5);
                if other2.on_boundary(mid) && opposite_sides(own2, other2, mid, r) {
                    continue;
                }
                let at = |t: f64| p + (q - p) * (t / len);
                let piece = (at(t0), at(t1));
                if !out.iter().any(|s| same_segment(*s, piece)) {
                    out.push(piece);
                }
            }
        }
    }
}

/// Whether, beside the boundary point `m` running along `r`, the two areas lie on opposite
/// sides of it.
fn opposite_sides(a: &Flat, b: &Flat, m: DVec2, r: DVec2) -> bool {
    let n = r.perp() * SIDE_PROBE;
    let side = |f: &Flat| {
        if f.inside_open(m + n) {
            1
        } else if f.inside_open(m - n) {
            -1
        } else {
            0
        }
    };
    side(a) * side(b) == -1
}

fn same_segment(s: (DVec3, DVec3), t: (DVec3, DVec3)) -> bool {
    let close = |a: DVec3, b: DVec3| a.distance(b) < LENGTH_TOL;
    (close(s.0, t.0) && close(s.1, t.1)) || (close(s.0, t.1) && close(s.1, t.0))
}
