//! Where two faces cross: the segments along which their areas meet, for Intersect Faces.

use std::collections::HashSet;

use glam::{DAffine3, DVec2, DVec3};

use crate::geom::{
    LENGTH_TOL, PLANE_ANGLE_TOL, Plane, point_in_polygon, segment_distance_2d, signed_area2,
    vector_area,
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

    /// How many edges of the model the segment `a`-`b` crosses away from every endpoint,
    /// counting stops at `cap`.
    pub fn edges_crossed_by(&self, a: DVec3, b: DVec3, cap: usize) -> usize {
        let mut found = 0;
        self.index().near_segment(a, b, LENGTH_TOL, |s| {
            if found < cap
                && let Shape::Segment(_, p, q) = s
                && crate::geom::segment_crossing(a, b, *p, *q).is_some()
            {
                found += 1;
            }
        });
        found
    }
}

/// The face `face` of `model` with its loops moved by `to_world` (which may scale, mirror or
/// shear), and its plane recomputed from the moved points. None when the face does not exist,
/// has no area after the move, or is not finite. The plane's normal is canonical (see
/// `Plane`), not the face's front: under a mirroring `to_world` the front is not the transformed
/// normal, and callers that need the front must not read it from here.
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
/// parallel planes give nothing, and so do planes whose normals differ by less than
/// `PLANE_ANGLE_TOL` without being `same_as` (too close to parallel for a stable line). Two
/// faces meeting along an edge give that edge.
pub fn face_segments(a: &FaceShape, b: &FaceShape) -> Vec<(DVec3, DVec3)> {
    if a.plane.same_as(&b.plane) {
        return coplanar_segments(a, b);
    }
    let dir = a.plane.normal.cross(b.plane.normal);
    let l2 = dir.length_squared();
    if !l2.is_finite() || dir.length() < PLANE_ANGLE_TOL {
        return Vec::new();
    }
    // The point of the line nearest a's centroid, worked out relative to the centroid, so
    // faces far from the world origin keep their precision.
    let c = a.outer.iter().sum::<DVec3>() / a.outer.len().max(1) as f64;
    let (da, db) = (a.plane.distance(c), b.plane.distance(c));
    let origin = c - (b.plane.normal.cross(dir) * da + dir.cross(a.plane.normal) * db) / l2;
    let dir = dir / l2.sqrt();
    let on_a = line_intervals(&Flat::new(a, &a.plane), &a.plane, origin, dir);
    if on_a.is_empty() {
        return Vec::new();
    }
    let on_b = line_intervals(&Flat::new(b, &b.plane), &b.plane, origin, dir);
    // A stretch ends where the line leaves a face across one of its edges: the end is that
    // edge's point in the other face's plane, so the two stretches that end at an edge two
    // faces share (a crease) end at one point. The point of the line itself is off the edge by
    // the planes' fit error, and along a shallow crease that error is magnified. Only a
    // correction of fit-error size is taken, and only where the edge crosses the other plane
    // at a slope of at least `MIN_SLOPE`: an edge nearly along the plane (so nearly along the
    // line) meets it anywhere within the fit error, far along the line from the crossing, and
    // the point of the line stands.
    let place = |end: End, own: &FaceShape, other: &FaceShape| -> DVec3 {
        let on_line = origin + dir * end.t;
        let Some((k, i)) = end.edge else {
            return on_line;
        };
        let l = if k == 0 {
            &own.outer
        } else {
            &own.holes[k - 1]
        };
        let (p, q) = (l[i], l[(i + 1) % l.len()]);
        match edge_in_plane(p, q, &other.plane, MIN_SLOPE) {
            Some(x) if x.distance(on_line) <= LENGTH_TOL / MIN_SLOPE => x,
            _ => on_line,
        }
    };
    meet(&on_a, &on_b)
        .into_iter()
        .filter(|(lo, hi)| hi.0.t - lo.0.t >= LENGTH_TOL)
        .map(|(lo, hi)| {
            let end = |(e, from_b): (End, bool)| {
                if from_b {
                    place(e, b, a)
                } else {
                    place(e, a, b)
                }
            };
            let (s, e) = (end(lo), end(hi));
            // The ends keep their order and the stretch its length.
            if (e - s).dot(dir) >= LENGTH_TOL {
                (s, e)
            } else {
                (origin + dir * lo.0.t, origin + dir * hi.0.t)
            }
        })
        .collect()
}

/// The least sine of the angle between an edge and the other face's plane for the edge's point
/// to end a stretch. The planes' fit error, `LENGTH_TOL`, is magnified by at most its inverse
/// along the edge.
const MIN_SLOPE: f64 = 0.01;

/// Where the segment `p`-`q` meets `plane`; None when the sine of its angle to the plane is
/// under `min_slope`.
fn edge_in_plane(p: DVec3, q: DVec3, plane: &Plane, min_slope: f64) -> Option<DVec3> {
    let d = q - p;
    let slope = plane.normal.dot(d);
    if slope.abs() < min_slope * d.length() {
        return None;
    }
    let s = (-plane.distance(p) / slope).clamp(0.0, 1.0);
    Some(p + d * s)
}

/// An upper bound on the work `face_segments(a, b)` does, in edge visits: the product of the two
/// faces' edge counts for coplanar faces (every boundary piece is tested against the other's
/// loops), and for crossing planes the edges of both times the windows the common line can have
/// (at most one more than the smaller edge count). Callers sum it to bound a whole command.
pub fn face_segments_cost(a: &FaceShape, b: &FaceShape) -> u64 {
    let edges =
        |s: &FaceShape| (s.outer.len() + s.holes.iter().map(Vec::len).sum::<usize>()) as u64;
    let (ea, eb) = (edges(a), edges(b));
    if a.plane.same_as(&b.plane) {
        ea.saturating_mul(eb)
    } else {
        (ea + eb).saturating_mul(ea.min(eb) + 1)
    }
}

/// A face's loops in the 2D basis of a plane.
struct Flat {
    outer: Vec<DVec2>,
    holes: Vec<Vec<DVec2>>,
    /// Per loop (outer first): +1 when the face's area lies to the left of the loop's
    /// direction, -1 when to the right, 0 for a loop without area.
    sides: Vec<f64>,
}

impl Flat {
    fn new(s: &FaceShape, plane: &Plane) -> Flat {
        let flat = |l: &[DVec3]| l.iter().map(|p| plane.project(*p)).collect::<Vec<_>>();
        let outer = flat(&s.outer);
        let holes: Vec<Vec<DVec2>> = s.holes.iter().map(|h| flat(h)).collect();
        let sides = std::iter::once((&outer, 1.0))
            .chain(holes.iter().map(|h| (h, -1.0)))
            .map(|(l, inside)| {
                let a = signed_area2(l);
                if a > 0.0 {
                    inside
                } else if a < 0.0 {
                    -inside
                } else {
                    0.0
                }
            })
            .collect();
        Flat {
            outer,
            holes,
            sides,
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

/// Where a stretch of a line inside a face ends: the parameter along the line, and the face
/// edge (loop, side) the line crosses there, if it crosses one.
#[derive(Clone, Copy)]
struct End {
    t: f64,
    edge: Option<(usize, usize)>,
}

impl End {
    fn at(t: f64) -> End {
        End { t, edge: None }
    }
}

/// The ranges of the 3D line `origin + t * dir` (unit `dir`, in the plane) inside the face,
/// boundary included, sorted and merged.
fn line_intervals(flat: &Flat, plane: &Plane, origin: DVec3, dir: DVec3) -> Vec<(End, End)> {
    let q0 = plane.project(origin);
    let r = plane.project(origin + dir) - q0;
    flat_stretches(flat, q0, r, f64::NEG_INFINITY, f64::INFINITY)
}

/// The parameter ranges of `q0 + t * r` (r of unit length) inside the closed area, within
/// `lo..=hi` (either may be infinite).
fn flat_intervals(flat: &Flat, q0: DVec2, r: DVec2, lo: f64, hi: f64) -> Vec<(f64, f64)> {
    flat_stretches(flat, q0, r, lo, hi)
        .into_iter()
        .map(|(a, b)| (a.t, b.t))
        .collect()
}

/// The same with each end's edge. Only the windows inside the range are classified.
fn flat_stretches(flat: &Flat, q0: DVec2, r: DVec2, lo: f64, hi: f64) -> Vec<(End, End)> {
    let param = |x: DVec2| (x - q0).dot(r);
    let mut ts: Vec<End> = Vec::new();
    for (k, l) in flat.loops().enumerate() {
        for i in 0..l.len() {
            let (p, q) = (l[i], l[(i + 1) % l.len()]);
            let (sp, sq) = (r.perp_dot(p - q0), r.perp_dot(q - q0));
            if sp.abs() < LENGTH_TOL {
                ts.push(End::at(param(p)));
            }
            if sq.abs() < LENGTH_TOL {
                ts.push(End::at(param(q)));
            }
            // An edge that passes from one side of the line to the other crosses it, even
            // when one end is within `LENGTH_TOL` of it: a shallow edge ending that near the
            // line crosses it far from that end, and the line leaves the area there.
            if sp * sq < 0.0 && sp.abs().max(sq.abs()) > LENGTH_TOL {
                ts.push(End {
                    t: param(p + (q - p) * (sp / (sp - sq))),
                    edge: Some((k, i)),
                });
            }
        }
    }
    ts.retain(|e| e.t.is_finite() && e.t > lo && e.t < hi);
    for end in [lo, hi] {
        if end.is_finite() {
            ts.push(End::at(end));
        }
    }
    ts.sort_by(|x, y| x.t.total_cmp(&y.t));
    let mut out: Vec<(End, End)> = Vec::new();
    for w in ts.windows(2) {
        let (e0, e1) = (w[0], w[1]);
        if e1.t > e0.t && flat.inside_closed(q0 + r * ((e0.t + e1.t) * 0.5)) {
            match out.last_mut() {
                Some(last) if last.1.t >= e0.t => last.1 = e1,
                _ => out.push((e0, e1)),
            }
        }
    }
    out
}

/// The overlaps of two sorted interval lists, each end with whether it comes from `b`.
fn meet(a: &[(End, End)], b: &[(End, End)]) -> Vec<((End, bool), (End, bool))> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() && j < b.len() {
        let lo = if a[i].0.t >= b[j].0.t {
            (a[i].0, false)
        } else {
            (b[j].0, true)
        };
        let hi = if a[i].1.t <= b[j].1.t {
            (a[i].1, false)
        } else {
            (b[j].1, true)
        };
        if hi.0.t > lo.0.t {
            out.push((lo, hi));
        }
        if a[i].1.t < b[j].1.t {
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
    let mut seen = HashSet::new();
    boundary_inside(a, &fa, &fb, &mut out, &mut seen);
    boundary_inside(b, &fb, &fa, &mut out, &mut seen);
    out
}

/// Adds the pieces of `own`'s boundary that lie in `other`'s closed area, leaving out pieces
/// along `other`'s boundary where the two areas lie on opposite sides, and pieces already in
/// `out`.
fn boundary_inside(
    own: &FaceShape,
    own2: &Flat,
    other2: &Flat,
    out: &mut Vec<(DVec3, DVec3)>,
    seen: &mut HashSet<[i64; 6]>,
) {
    let loops3 = std::iter::once(&own.outer).chain(own.holes.iter());
    for (k, (l3, l2)) in loops3.zip(own2.loops()).enumerate() {
        for i in 0..l3.len() {
            let j = (i + 1) % l3.len();
            let (p, q) = (l3[i], l3[j]);
            let (p2, q2) = (l2[i], l2[j]);
            let len = p2.distance(q2);
            if len < LENGTH_TOL {
                continue;
            }
            let r = (q2 - p2) / len;
            for (t0, t1) in flat_intervals(other2, p2, r, 0.0, len) {
                if t1 - t0 < LENGTH_TOL {
                    continue;
                }
                let mid = p2 + r * ((t0 + t1) * 0.5);
                if opposite_sides(own2.sides[k], other2, mid, r) {
                    continue;
                }
                let at = |t: f64| p + (q - p) * (t / len);
                let piece = (at(t0), at(t1));
                if seen.insert(segment_key(piece)) {
                    out.push(piece);
                }
            }
        }
    }
}

/// Whether the point `m`, on a boundary piece of the other face running along `r` with the
/// own face's area on side `own_side` of it (see `Flat::sides`), lies along an edge of `other`
/// whose area is on the opposite side. False when `m` is not along an edge of `other`.
fn opposite_sides(own_side: f64, other: &Flat, m: DVec2, r: DVec2) -> bool {
    for (k, l) in other.loops().enumerate() {
        for i in 0..l.len() {
            let (p, q) = (l[i], l[(i + 1) % l.len()]);
            let d = q - p;
            let len = d.length();
            if len < LENGTH_TOL
                || segment_distance_2d(m, p, q) >= LENGTH_TOL
                || r.perp_dot(d).abs() > 1e-6 * len
            {
                continue;
            }
            return own_side * other.sides[k] * d.dot(r).signum() < 0.0;
        }
    }
    false
}

/// The segment's endpoints on a grid of `LENGTH_TOL`, smaller endpoint first, so equal
/// segments (either way round) share a key.
fn segment_key((a, b): (DVec3, DVec3)) -> [i64; 6] {
    let q = |p: DVec3| {
        [
            (p.x / LENGTH_TOL).round() as i64,
            (p.y / LENGTH_TOL).round() as i64,
            (p.z / LENGTH_TOL).round() as i64,
        ]
    };
    let (a, b) = (q(a), q(b));
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
}
