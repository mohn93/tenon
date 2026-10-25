//! A bounding-volume hierarchy over the triangles of one mesh, in the mesh's own coordinates,
//! and the minimum distance between two of them placed anywhere.
//!
//! A tree is built once per definition and placed by a transform at query time, so the same
//! tree serves every instance of the definition in every pose (a crank turning through a
//! sweep moves its instances, not its geometry).

use glam::{DAffine3, DMat3, DVec3};

/// Triangles per leaf at most.
const LEAF: usize = 4;

/// The triangles of a mesh in a binary tree of axis-aligned boxes.
#[derive(Clone, Debug, Default)]
pub struct Bvh {
    tris: Vec<[DVec3; 3]>,
    nodes: Vec<Node>,
}

/// A box with either two children (`count == 0`: the left one follows it, the right one is at
/// `first`) or the triangles `first..first + count`.
#[derive(Clone, Copy, Debug)]
struct Node {
    lo: DVec3,
    hi: DVec3,
    first: u32,
    count: u32,
}

/// The two nearest points of two surfaces and how far apart they are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Closest {
    pub distance: f64,
    /// On the first surface.
    pub a: DVec3,
    /// On the second surface.
    pub b: DVec3,
}

impl Closest {
    pub fn new(a: DVec3, b: DVec3) -> Closest {
        Closest {
            distance: a.distance(b),
            a,
            b,
        }
    }

    /// The same pair with the surfaces exchanged.
    pub fn swapped(self) -> Closest {
        Closest {
            distance: self.distance,
            a: self.b,
            b: self.a,
        }
    }

    /// The nearer of the two (this one on a tie).
    pub fn min(self, other: Closest) -> Closest {
        if other.distance < self.distance {
            other
        } else {
            self
        }
    }
}

/// Nothing found yet: infinitely far.
pub const NONE: Closest = Closest {
    distance: f64::INFINITY,
    a: DVec3::ZERO,
    b: DVec3::ZERO,
};

impl Bvh {
    /// A tree over `tris` (degenerate ones included; they still bound the surface).
    pub fn new(mut tris: Vec<[DVec3; 3]>) -> Bvh {
        let mut nodes = Vec::with_capacity(2 * tris.len() / LEAF + 1);
        if !tris.is_empty() {
            build(&mut tris, 0, &mut nodes);
        }
        Bvh { tris, nodes }
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    pub fn triangles(&self) -> &[[DVec3; 3]] {
        &self.tris
    }

    /// The box around every triangle, in the mesh's coordinates; None when empty.
    pub fn bounds(&self) -> Option<(DVec3, DVec3)> {
        self.nodes.first().map(|n| (n.lo, n.hi))
    }

    /// The box around every triangle once placed by `place`; None when empty.
    pub fn placed_bounds(&self, place: &DAffine3) -> Option<(DVec3, DVec3)> {
        self.nodes.first().map(|n| placed_box(n, place))
    }

    /// The nearest points of this tree placed by `pa` and `other` placed by `pb`, when they
    /// come nearer than `bound`. The search stops as soon as a pair within `stop` is found
    /// (0 for the exact minimum; a tolerance to ask only whether they meet). Surfaces that
    /// cross are 0 apart.
    pub fn distance(
        &self,
        pa: &DAffine3,
        other: &Bvh,
        pb: &DAffine3,
        bound: f64,
        stop: f64,
    ) -> Option<Closest> {
        if self.is_empty() || other.is_empty() {
            return None;
        }
        if let Some(rel) = relative(pa, pb) {
            let local = self.distance_placed(&DAffine3::IDENTITY, other, &rel, bound, stop)?;
            return Some(Closest {
                distance: local.distance,
                a: pa.transform_point3(local.a),
                b: pa.transform_point3(local.b),
            });
        }
        self.distance_placed(pa, other, pb, bound, stop)
    }

    fn distance_placed(
        &self,
        pa: &DAffine3,
        other: &Bvh,
        pb: &DAffine3,
        bound: f64,
        stop: f64,
    ) -> Option<Closest> {
        let mut search = Search {
            a: self,
            b: other,
            pa,
            pb,
            best: None,
            bound,
            stop,
        };
        search.pair(0, 0);
        search.best
    }

    /// The point of this tree placed by `place` nearest `q`, as `Closest` with `a` = `q` and
    /// `b` on the surface; None when empty.
    pub fn nearest_point(&self, place: &DAffine3, q: DVec3) -> Option<Closest> {
        if self.is_empty() {
            return None;
        }
        let mut best = NONE;
        self.point_rec(0, place, q, &mut best);
        Some(best)
    }

    fn point_rec(&self, i: usize, place: &DAffine3, q: DVec3, best: &mut Closest) {
        let n = self.nodes[i];
        if point_gap(placed_box(&n, place), q) >= best.distance {
            return;
        }
        if n.count > 0 {
            for t in &self.tris[n.first as usize..(n.first + n.count) as usize] {
                let t = t.map(|p| place.transform_point3(p));
                *best = best.min(Closest::new(q, closest_on_triangle(q, t)));
            }
            return;
        }
        let kids = [i + 1, n.first as usize];
        let gap = kids.map(|k| point_gap(placed_box(&self.nodes[k], place), q));
        let order = if gap[0] <= gap[1] { [0, 1] } else { [1, 0] };
        for k in order {
            self.point_rec(kids[k], place, q, best);
        }
    }

    /// The least and greatest `t` at which the line `p + d t` (world coordinates, `d` not
    /// zero) meets this tree placed by `place`: a triangle's boundary counts, a triangle
    /// lying along the line does not (its neighbours give the points). None when the line
    /// misses every triangle or `place` cannot be inverted.
    pub fn line_hits(&self, place: &DAffine3, p: DVec3, d: DVec3) -> Option<(f64, f64)> {
        if self.is_empty()
            || !place.is_finite()
            || place.matrix3.determinant().abs() < crate::MIN_DETERMINANT
        {
            return None;
        }
        // The line in the mesh's coordinates keeps its parameter: an affine map takes
        // p + d t to p' + d' t.
        let back = place.inverse();
        let (p, d) = (back.transform_point3(p), back.transform_vector3(d));
        let mut hits: Option<(f64, f64)> = None;
        self.line_rec(0, p, d, &mut hits);
        hits
    }

    fn line_rec(&self, i: usize, p: DVec3, d: DVec3, hits: &mut Option<(f64, f64)>) {
        let n = self.nodes[i];
        if !line_meets_box(n.lo, n.hi, p, d) {
            return;
        }
        if n.count > 0 {
            for t in &self.tris[n.first as usize..(n.first + n.count) as usize] {
                if let Some(s) = line_triangle(p, d, *t) {
                    *hits = Some(hits.map_or((s, s), |(lo, hi)| (lo.min(s), hi.max(s))));
                }
            }
            return;
        }
        self.line_rec(i + 1, p, d, hits);
        self.line_rec(n.first as usize, p, d, hits);
    }

    /// Whether this tree placed by `pa` and `other` placed by `pb` cross: an edge of a
    /// triangle of one passes through a triangle of the other (its boundary included) with
    /// its ends more than `tol` to either side. Surfaces that only touch (faces lying on each
    /// other, edges along faces) do not cross; a crossing shallower than `tol` may be missed.
    pub fn crosses(&self, pa: &DAffine3, other: &Bvh, pb: &DAffine3, tol: f64) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        match relative(pa, pb) {
            Some(rel) => crossing_pair(self, &DAffine3::IDENTITY, other, &rel, 0, 0, tol),
            None => crossing_pair(self, pa, other, pb, 0, 0, tol),
        }
    }
}

/// `pb` as seen from `pa` (`pa` inverted, then `pb`) when `pa` only turns, mirrors and moves
/// (no scale or shear), so two trees can be compared in the first one's own coordinates,
/// where distances are the world's. Boxes stay tight there for two parts that stand square
/// to each other however they are turned in the world (a piston in a tilted bore), where
/// boxes placed in the world would grow with the tilt and meet far more often.
fn relative(pa: &DAffine3, pb: &DAffine3) -> Option<DAffine3> {
    let m = pa.matrix3;
    let off = (m.transpose() * m - DMat3::IDENTITY).to_cols_array();
    (pa.is_finite() && off.iter().all(|x| x.abs() < 1e-12)).then(|| pa.inverse() * *pb)
}

fn build(tris: &mut [[DVec3; 3]], offset: usize, nodes: &mut Vec<Node>) -> usize {
    let (lo, hi) = tris.iter().flatten().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    let at = nodes.len();
    nodes.push(Node {
        lo,
        hi,
        first: offset as u32,
        count: tris.len() as u32,
    });
    if tris.len() <= LEAF {
        return at;
    }
    let centre = |t: &[DVec3; 3]| t[0] + t[1] + t[2];
    let (clo, chi) = tris.iter().map(centre).fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    );
    let extent = chi - clo;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let mid = tris.len() / 2;
    tris.select_nth_unstable_by(mid, |a, b| centre(a)[axis].total_cmp(&centre(b)[axis]));
    let (left, right) = tris.split_at_mut(mid);
    build(left, offset, nodes);
    let r = build(right, offset + mid, nodes);
    nodes[at].first = r as u32;
    nodes[at].count = 0;
    at
}

/// The axis-aligned box around node box `n` moved by `place`.
fn placed_box(n: &Node, place: &DAffine3) -> (DVec3, DVec3) {
    let centre = place.transform_point3((n.lo + n.hi) * 0.5);
    let half = (n.hi - n.lo) * 0.5;
    let m = place.matrix3;
    let extent = m.x_axis.abs() * half.x + m.y_axis.abs() * half.y + m.z_axis.abs() * half.z;
    (centre - extent, centre + extent)
}

/// How far apart two axis-aligned boxes are (0 when they meet).
fn box_distance((alo, ahi): (DVec3, DVec3), (blo, bhi): (DVec3, DVec3)) -> f64 {
    let gap = (blo - ahi).max(alo - bhi).max(DVec3::ZERO);
    gap.length()
}

/// How far point `p` is from a box (0 inside it).
fn point_gap((lo, hi): (DVec3, DVec3), p: DVec3) -> f64 {
    (lo - p).max(p - hi).max(DVec3::ZERO).length()
}

struct Search<'a> {
    a: &'a Bvh,
    b: &'a Bvh,
    pa: &'a DAffine3,
    pb: &'a DAffine3,
    best: Option<Closest>,
    bound: f64,
    stop: f64,
}

impl Search<'_> {
    fn limit(&self) -> f64 {
        self.best.map_or(self.bound, |c| c.distance)
    }

    fn done(&self) -> bool {
        self.best.is_some_and(|c| c.distance <= self.stop)
    }

    fn pair(&mut self, i: usize, j: usize) {
        if self.done() {
            return;
        }
        let (na, nb) = (self.a.nodes[i], self.b.nodes[j]);
        let (ba, bb) = (placed_box(&na, self.pa), placed_box(&nb, self.pb));
        if box_distance(ba, bb) >= self.limit() {
            return;
        }
        match (na.count > 0, nb.count > 0) {
            (true, true) => self.leaves(&na, &nb),
            (a_leaf, b_leaf) => {
                // Open the larger box (or the one that is not a leaf), nearer child first.
                let open_a =
                    b_leaf || (!a_leaf && (ba.1 - ba.0).length() >= (bb.1 - bb.0).length());
                let kids = |n: usize, node: &Node| [n + 1, node.first as usize];
                let pairs: [(usize, usize); 2] = if open_a {
                    kids(i, &na).map(|k| (k, j))
                } else {
                    kids(j, &nb).map(|k| (i, k))
                };
                let d = pairs.map(|(x, y)| {
                    box_distance(
                        placed_box(&self.a.nodes[x], self.pa),
                        placed_box(&self.b.nodes[y], self.pb),
                    )
                });
                let order = if d[0] <= d[1] { [0, 1] } else { [1, 0] };
                for k in order {
                    if d[k] < self.limit() {
                        self.pair(pairs[k].0, pairs[k].1);
                    }
                }
            }
        }
    }

    fn leaves(&mut self, na: &Node, nb: &Node) {
        let placed =
            |t: &[DVec3; 3], place: &DAffine3| Near::of(t.map(|p| place.transform_point3(p)));
        let mut tb = [None; LEAF];
        let range = |n: &Node| n.first as usize..(n.first + n.count) as usize;
        for (slot, t) in tb.iter_mut().zip(&self.b.tris[range(nb)]) {
            *slot = Some(placed(t, self.pb));
        }
        for t in &self.a.tris[range(na)] {
            let ta = placed(t, self.pa);
            for u in tb.iter().flatten() {
                if ta.apart(u, self.limit()) {
                    continue;
                }
                let c = triangle_distance(ta.t, u.t);
                if c.distance < self.limit() {
                    self.best = Some(c);
                    if self.done() {
                        return;
                    }
                }
            }
        }
    }
}

/// A placed triangle with its box and unit normal, for telling cheaply that two triangles
/// are no nearer than a distance.
#[derive(Clone, Copy)]
struct Near {
    t: [DVec3; 3],
    lo: DVec3,
    hi: DVec3,
    normal: DVec3,
}

/// How much farther than a limit two triangles' lower bound must be before they are passed
/// over: a margin far above rounding, so no pair that could be nearer is ever skipped.
const APART_MARGIN: f64 = 1e-12;

impl Near {
    fn of(t: [DVec3; 3]) -> Near {
        Near {
            t,
            lo: t[0].min(t[1]).min(t[2]),
            hi: t[0].max(t[1]).max(t[2]),
            normal: (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero(),
        }
    }

    /// Whether `other` is surely farther than `limit` from this triangle: their boxes are, or
    /// the corners of one lie all on one side of the other's plane, farther than `limit`.
    fn apart(&self, other: &Near, limit: f64) -> bool {
        let limit = limit + APART_MARGIN;
        box_distance((self.lo, self.hi), (other.lo, other.hi)) > limit
            || plane_gap(self, other) > limit
            || plane_gap(other, self) > limit
    }
}

/// How far the corners of `q` lie from the plane of `p` when all lie on one side of it (0
/// when they straddle it or `p` is degenerate): no point of `q` is nearer than that to `p`.
fn plane_gap(p: &Near, q: &Near) -> f64 {
    if p.normal == DVec3::ZERO {
        return 0.0;
    }
    let d = q.t.map(|x| (x - p.t[0]).dot(p.normal));
    let (lo, hi) = (d[0].min(d[1]).min(d[2]), d[0].max(d[1]).max(d[2]));
    lo.max(-hi).max(0.0)
}

fn crossing_pair(
    a: &Bvh,
    pa: &DAffine3,
    b: &Bvh,
    pb: &DAffine3,
    i: usize,
    j: usize,
    tol: f64,
) -> bool {
    let (na, nb) = (a.nodes[i], b.nodes[j]);
    let (ba, bb) = (placed_box(&na, pa), placed_box(&nb, pb));
    if box_distance(ba, bb) > tol {
        return false;
    }
    match (na.count > 0, nb.count > 0) {
        (true, true) => {
            let ta: Vec<[DVec3; 3]> = a.tris[na.first as usize..(na.first + na.count) as usize]
                .iter()
                .map(|t| t.map(|p| pa.transform_point3(p)))
                .collect();
            let tb: Vec<[DVec3; 3]> = b.tris[nb.first as usize..(nb.first + nb.count) as usize]
                .iter()
                .map(|t| t.map(|p| pb.transform_point3(p)))
                .collect();
            ta.iter().any(|p| {
                tb.iter()
                    .any(|q| edges_pierce(p, q, tol) || edges_pierce(q, p, tol))
            })
        }
        (a_leaf, b_leaf) => {
            let open_a = b_leaf || (!a_leaf && (ba.1 - ba.0).length() >= (bb.1 - bb.0).length());
            if open_a {
                crossing_pair(a, pa, b, pb, i + 1, j, tol)
                    || crossing_pair(a, pa, b, pb, na.first as usize, j, tol)
            } else {
                crossing_pair(a, pa, b, pb, i, j + 1, tol)
                    || crossing_pair(a, pa, b, pb, i, nb.first as usize, tol)
            }
        }
    }
}

/// Whether an edge of triangle `p` passes through triangle `q` (its boundary within `tol`
/// included) with its ends more than `tol` to either side of `q`'s plane.
fn edges_pierce(p: &[DVec3; 3], q: &[DVec3; 3], tol: f64) -> bool {
    let n = (q[1] - q[0]).cross(q[2] - q[0]);
    let len = n.length();
    if len == 0.0 || !len.is_finite() {
        return false;
    }
    let n = n / len;
    let side: [f64; 3] = p.map(|v| (v - q[0]).dot(n));
    (0..3).any(|k| {
        let (d0, d1) = (side[k], side[(k + 1) % 3]);
        if !((d0 > tol && d1 < -tol) || (d0 < -tol && d1 > tol)) {
            return false;
        }
        let (e0, e1) = (p[k], p[(k + 1) % 3]);
        let x = e0 + (e1 - e0) * (d0 / (d0 - d1));
        (0..3).all(|m| {
            let (u, v) = (q[m], q[(m + 1) % 3]);
            let edge = v - u;
            let el = edge.length();
            el == 0.0 || edge.cross(x - u).dot(n) >= -tol * el
        })
    })
}

/// The nearest points of two triangles (0 apart where they cross or touch).
pub fn triangle_distance(p: [DVec3; 3], q: [DVec3; 3]) -> Closest {
    for k in 0..3 {
        if let Some(x) = segment_crosses(p[k], p[(k + 1) % 3], q) {
            return Closest {
                distance: 0.0,
                a: x,
                b: x,
            };
        }
        if let Some(x) = segment_crosses(q[k], q[(k + 1) % 3], p) {
            return Closest {
                distance: 0.0,
                a: x,
                b: x,
            };
        }
    }
    let mut best = Closest {
        distance: f64::INFINITY,
        a: p[0],
        b: q[0],
    };
    let mut take = |a: DVec3, b: DVec3| {
        let d = a.distance(b);
        if d < best.distance {
            best = Closest { distance: d, a, b };
        }
    };
    for i in 0..3 {
        for j in 0..3 {
            let (a, b) = segment_segment(p[i], p[(i + 1) % 3], q[j], q[(j + 1) % 3]);
            take(a, b);
        }
        take(p[i], closest_on_triangle(p[i], q));
        let on_p = closest_on_triangle(q[i], p);
        take(on_p, q[i]);
    }
    best
}

/// Where the segment `a`-`b` passes through triangle `t` (not lying in its plane), if it does.
fn segment_crosses(a: DVec3, b: DVec3, [p, q, r]: [DVec3; 3]) -> Option<DVec3> {
    let d = b - a;
    let (e1, e2) = (q - p, r - p);
    let h = d.cross(e2);
    let det = e1.dot(h);
    let scale = d.length() * e1.length() * e2.length();
    if det.abs() <= 1e-12 * scale || scale == 0.0 {
        return None;
    }
    let s = a - p;
    let u = s.dot(h) / det;
    let qv = s.cross(e1);
    let v = d.dot(qv) / det;
    let t = e2.dot(qv) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && (0.0..=1.0).contains(&t)).then(|| a + d * t)
}

/// The nearest points of segments `p1`-`q1` and `p2`-`q2` (Ericson, Real-Time Collision
/// Detection 5.1.9), degenerate segments included.
pub fn segment_segment(p1: DVec3, q1: DVec3, p2: DVec3, q2: DVec3) -> (DVec3, DVec3) {
    let c = segment_segment_ranges((p1, q1 - p1, 0.0, 1.0), (p2, q2 - p2, 0.0, 1.0));
    (c.a, c.b)
}

/// The nearest points of two segments or lines, each `(p, d, lo, hi)`: the points `p + d t`
/// for `t` in `lo..=hi` (infinite bounds give a ray or a line).
pub fn segment_segment_ranges(
    s1: (DVec3, DVec3, f64, f64),
    s2: (DVec3, DVec3, f64, f64),
) -> Closest {
    let (p1, d1, lo1, hi1) = s1;
    let (p2, d2, lo2, hi2) = s2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    const EPS: f64 = 1e-300;
    let (s, t);
    if a <= EPS && e <= EPS {
        s = 0.0;
        t = 0.0;
    } else if a <= EPS {
        s = 0.0;
        t = (f / e).clamp(lo2, hi2);
    } else {
        let c = d1.dot(r);
        if e <= EPS {
            t = 0.0;
            s = (-c / a).clamp(lo1, hi1);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > 1e-12 * a * e {
                ((b * f - c * e) / denom).clamp(lo1, hi1)
            } else {
                0.0f64.clamp(lo1, hi1)
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < lo2 || t0 > hi2 {
                t0 = t0.clamp(lo2, hi2);
                s0 = ((b * t0 - c) / a).clamp(lo1, hi1);
            }
            s = s0;
            t = t0;
        }
    }
    Closest::new(p1 + d1 * s, p2 + d2 * t)
}

/// Whether the line `p + d t` passes through the box `lo..hi` (grown a little, so a line
/// along a face of it counts).
fn line_meets_box(lo: DVec3, hi: DVec3, p: DVec3, d: DVec3) -> bool {
    let pad = 1e-9 * (hi - lo).max_element().max(1e-9);
    let (lo, hi) = (lo - DVec3::splat(pad), hi + DVec3::splat(pad));
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for k in 0..3 {
        if d[k] == 0.0 {
            if p[k] < lo[k] || p[k] > hi[k] {
                return false;
            }
            continue;
        }
        let (a, b) = ((lo[k] - p[k]) / d[k], (hi[k] - p[k]) / d[k]);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
        if t0 > t1 {
            return false;
        }
    }
    true
}

/// Where the line `p + d t` passes through triangle `t` (its boundary included): the `t`.
/// None when it misses, or the triangle is degenerate or lies along the line.
fn line_triangle(p: DVec3, d: DVec3, t: [DVec3; 3]) -> Option<f64> {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let dn = n.dot(d);
    if dn.abs() <= 1e-14 * n.length() * d.length() {
        return None;
    }
    let s = n.dot(t[0] - p) / dn;
    in_triangle(t, p + d * s).then_some(s)
}

/// Whether `x`, in the plane of the triangle, lies inside it (within rounding).
fn in_triangle([a, b, c]: [DVec3; 3], x: DVec3) -> bool {
    let n = (b - a).cross(c - a);
    let tol = -1e-12 * n.length_squared();
    n.dot((b - a).cross(x - a)) >= tol
        && n.dot((c - b).cross(x - b)) >= tol
        && n.dot((a - c).cross(x - c)) >= tol
}

/// A segment or line (`p + d t`, `t` in `lo..=hi`, bounds possibly infinite) against a
/// triangle: the nearest points, 0 apart where it passes through.
pub fn segment_triangle(p: DVec3, d: DVec3, lo: f64, hi: f64, t: [DVec3; 3]) -> Closest {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let dn = n.dot(d);
    if dn.abs() > 1e-14 * n.length() * d.length() {
        let s = n.dot(t[0] - p) / dn;
        if s >= lo && s <= hi {
            let x = p + d * s;
            if in_triangle(t, x) {
                return Closest::new(x, x);
            }
        }
    }
    let mut best = NONE;
    for s in [lo, hi] {
        if s.is_finite() {
            let q = p + d * s;
            best = best.min(Closest::new(q, closest_on_triangle(q, t)));
        }
    }
    for i in 0..3 {
        let (a, b) = (t[i], t[(i + 1) % 3]);
        best = best.min(segment_segment_ranges((p, d, lo, hi), (a, b - a, 0.0, 1.0)));
    }
    best
}

/// The point of triangle `t` nearest `p` (Ericson 5.1.5); a degenerate triangle gives the
/// nearest point of its sides.
pub fn closest_on_triangle(p: DVec3, [a, b, c]: [DVec3; 3]) -> DVec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    if ab.cross(ac).length_squared() <= 1e-30 {
        let candidates = [
            segment_segment(p, p, a, b).1,
            segment_segment(p, p, b, c).1,
            segment_segment(p, p, c, a).1,
        ];
        return candidates
            .into_iter()
            .min_by(|x, y| x.distance_squared(p).total_cmp(&y.distance_squared(p)))
            .expect("three");
    }
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [DVec3; 3] {
        [a.into(), b.into(), c.into()]
    }

    #[test]
    fn parallel_triangles_are_their_gap_apart() {
        let p = tri([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let q = tri([0.0, 0.0, 0.5], [1.0, 0.0, 0.5], [0.0, 1.0, 0.5]);
        assert!((triangle_distance(p, q).distance - 0.5).abs() < 1e-12);
    }

    #[test]
    fn crossing_triangles_are_zero_apart() {
        let p = tri([0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]);
        let q = tri([0.5, 0.5, -1.0], [0.5, 0.5, 1.0], [0.6, -1.0, 0.0]);
        assert_eq!(triangle_distance(p, q).distance, 0.0);
    }

    #[test]
    fn skew_edges_give_the_edge_distance() {
        let p = tri([-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]);
        let q = tri([0.0, -1.0, 0.3], [0.0, 1.0, 0.3], [0.0, 0.0, 1.0]);
        let c = triangle_distance(p, q);
        assert!((c.distance - 0.3).abs() < 1e-12);
        assert!(c.a.distance(DVec3::ZERO) < 1e-12);
    }

    #[test]
    fn coplanar_overlapping_triangles_are_zero_apart() {
        let p = tri([0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]);
        let q = tri([0.2, 0.2, 0.0], [0.5, 0.2, 0.0], [0.2, 0.5, 0.0]);
        assert!(triangle_distance(p, q).distance < 1e-12);
    }

    #[test]
    fn crossing_needs_an_edge_through_a_triangle_beyond_the_tolerance() {
        let p = tri([0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]);
        let through = tri([0.5, 0.5, -1.0], [0.5, 0.5, 1.0], [0.6, -1.0, 0.0]);
        assert!(edges_pierce(&through, &p, 1e-6) || edges_pierce(&p, &through, 1e-6));
        // Lying on it, or standing on it with one end in its plane, is touching.
        let flat = tri([0.2, 0.2, 0.0], [0.5, 0.2, 0.0], [0.2, 0.5, 0.0]);
        assert!(!edges_pierce(&flat, &p, 1e-6) && !edges_pierce(&p, &flat, 1e-6));
        let standing = tri([0.5, 0.5, 0.0], [0.6, 0.5, 1.0], [0.5, 0.6, 1.0]);
        assert!(!edges_pierce(&standing, &p, 1e-6) && !edges_pierce(&p, &standing, 1e-6));
        // Through it by less than the tolerance does not count.
        let shallow = tri([0.5, 0.5, -1e-7], [0.6, 0.5, 1.0], [0.5, 0.6, 1.0]);
        assert!(!edges_pierce(&shallow, &p, 1e-6));
        // Through its boundary does.
        let edge = tri([1.0, 0.0, -1.0], [1.0, 0.0, 1.0], [1.0, -1.0, 0.0]);
        assert!(edges_pierce(&edge, &p, 1e-6));
    }

    fn grid(n: usize, z: f64) -> Vec<[DVec3; 3]> {
        let mut out = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let (x, y) = (i as f64, j as f64);
                out.push(tri([x, y, z], [x + 1.0, y, z], [x, y + 1.0, z]));
                out.push(tri([x + 1.0, y, z], [x + 1.0, y + 1.0, z], [x, y + 1.0, z]));
            }
        }
        out
    }

    #[test]
    fn the_tree_finds_the_brute_force_minimum() {
        let a = Bvh::new(grid(8, 0.0));
        let b = Bvh::new(grid(8, 0.0));
        let pa = DAffine3::IDENTITY;
        let pb =
            DAffine3::from_rotation_x(0.3) * DAffine3::from_translation(DVec3::new(3.3, 2.1, 1.7));
        let tree = a.distance(&pa, &b, &pb, f64::INFINITY, 0.0).unwrap();
        let mut brute = f64::INFINITY;
        for t in a.triangles() {
            for u in b.triangles() {
                let ta = t.map(|p| pa.transform_point3(p));
                let ub = u.map(|p| pb.transform_point3(p));
                brute = brute.min(triangle_distance(ta, ub).distance);
            }
        }
        assert!((tree.distance - brute).abs() < 1e-12, "{tree:?} vs {brute}");
        assert!((tree.a.distance(tree.b) - tree.distance).abs() < 1e-12);
    }

    #[test]
    fn turned_and_scaled_placements_find_the_brute_force_minimum() {
        // Both placed turned (compared in the first one's coordinates), and the first one
        // scaled too (compared in the world's): the same minimum as triangle by triangle.
        let a = Bvh::new(grid(6, 0.0));
        let b = Bvh::new(grid(6, 0.0));
        let pb =
            DAffine3::from_rotation_x(0.6) * DAffine3::from_translation(DVec3::new(2.3, 1.1, 1.2));
        for pa in [
            DAffine3::from_rotation_y(0.4) * DAffine3::from_translation(DVec3::new(1.0, -2.0, 0.3)),
            DAffine3::from_rotation_y(0.4) * DAffine3::from_scale(DVec3::new(1.0, -1.0, 1.0)),
            DAffine3::from_rotation_y(0.4) * DAffine3::from_scale(DVec3::new(1.0, 2.0, 0.5)),
        ] {
            let tree = a.distance(&pa, &b, &pb, f64::INFINITY, 0.0).unwrap();
            let mut brute = f64::INFINITY;
            for t in a.triangles() {
                for u in b.triangles() {
                    let ta = t.map(|p| pa.transform_point3(p));
                    let ub = u.map(|p| pb.transform_point3(p));
                    brute = brute.min(triangle_distance(ta, ub).distance);
                }
            }
            assert!((tree.distance - brute).abs() < 1e-12, "{tree:?} vs {brute}");
            assert!(
                (tree.a.distance(tree.b) - tree.distance).abs() < 1e-9,
                "{tree:?}"
            );
            // b stood up as a wall across the middle of a
            let crossing = pa
                * DAffine3::from_translation(DVec3::new(0.25, 3.3, -2.7))
                * DAffine3::from_rotation_x(std::f64::consts::FRAC_PI_2);
            assert!(a.crosses(&pa, &b, &crossing, 1e-9));
            // clearly apart: the surfaces cannot cross; a stood-up wall that crosses is at distance 0
            if brute > 1e-6 {
                assert!(!a.crosses(&pa, &b, &pb, 1e-9), "apart by {brute}");
            }
            let mut wall = f64::INFINITY;
            for t in a.triangles() {
                for u in b.triangles() {
                    let ta = t.map(|p| pa.transform_point3(p));
                    let ub = u.map(|p| (crossing).transform_point3(p));
                    wall = wall.min(triangle_distance(ta, ub).distance);
                }
            }
            assert!(wall < 1e-9, "crossing meshes are {wall} apart");
        }
    }

    #[test]
    fn nearest_point_finds_the_brute_force_minimum_under_a_placement() {
        let a = Bvh::new(grid(8, 0.0));
        let place = DAffine3::from_rotation_y(0.4)
            * DAffine3::from_scale(DVec3::new(1.0, 2.0, 0.5))
            * DAffine3::from_translation(DVec3::new(1.0, -2.0, 0.3));
        for q in [
            DVec3::new(3.3, 2.1, 1.7),
            DVec3::new(-5.0, 4.0, -2.0),
            DVec3::new(2.0, 2.0, 0.2),
        ] {
            let c = a.nearest_point(&place, q).unwrap();
            let brute = a
                .triangles()
                .iter()
                .map(|t| {
                    let t = t.map(|p| place.transform_point3(p));
                    closest_on_triangle(q, t).distance(q)
                })
                .fold(f64::INFINITY, f64::min);
            assert!((c.distance - brute).abs() < 1e-12, "{c:?} vs {brute}");
            assert_eq!(c.a, q);
        }
        assert!(Bvh::default().nearest_point(&place, DVec3::ZERO).is_none());
    }

    #[test]
    fn lines_use_parameter_ranges() {
        let line = (DVec3::ZERO, DVec3::X, f64::NEG_INFINITY, f64::INFINITY);
        let seg = (DVec3::new(5.0, 1.0, 2.0), DVec3::X, 0.0, 1.0);
        let c = segment_segment_ranges(line, seg);
        assert!((c.distance - 5.0f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn a_bound_below_the_distance_finds_nothing() {
        let a = Bvh::new(grid(2, 0.0));
        let b = Bvh::new(grid(2, 1.0));
        let p = DAffine3::IDENTITY;
        assert!(a.distance(&p, &b, &p, 0.5, 0.0).is_none());
        assert!((a.distance(&p, &b, &p, 2.0, 0.0).unwrap().distance - 1.0).abs() < 1e-12);
    }

    /// The side walls of a 64-sided cylinder of radius `r` along Z, two triangles a side.
    fn tube_walls(r: f64) -> Vec<[DVec3; 3]> {
        let at = |k: usize, z: f64| {
            let a = k as f64 * std::f64::consts::TAU / 64.0;
            [r * a.cos(), r * a.sin(), z]
        };
        (0..64)
            .flat_map(|k| {
                let (p, q) = (at(k, 0.0), at(k + 1, 0.0));
                let (u, v) = (at(k, 0.05), at(k + 1, 0.05));
                [tri(p, q, v), tri(p, v, u)]
            })
            .collect()
    }

    #[test]
    fn a_pin_in_its_bore_is_the_brute_force_minimum_apart() {
        // Nearly every pair of facing triangles is about the gap apart, so most are passed over
        // by their boxes or planes: the nearest pair must never be among them.
        let pin = Bvh::new(tube_walls(0.0095));
        let bore = Bvh::new(tube_walls(0.01));
        let pa = DAffine3::IDENTITY;
        for pb in [
            DAffine3::from_translation(DVec3::new(0.0002, 0.0, 0.01)),
            DAffine3::from_rotation_z(0.03)
                * DAffine3::from_translation(DVec3::new(0.0, -0.0003, 0.0)),
        ] {
            let tree = pin.distance(&pa, &bore, &pb, 0.005, 0.0).unwrap();
            let mut brute = f64::INFINITY;
            for t in pin.triangles() {
                for u in bore.triangles() {
                    let ub = u.map(|p| pb.transform_point3(p));
                    brute = brute.min(triangle_distance(*t, ub).distance);
                }
            }
            assert_eq!(tree.distance, brute);
        }
    }

    #[test]
    fn triangles_told_apart_are_farther_than_the_limit() {
        // A spread of triangles near one another, turned every way.
        let mut tris = Vec::new();
        for k in 0..60 {
            let f = k as f64;
            let c = DVec3::new((f * 0.7).sin(), (f * 1.3).cos(), (f * 0.31).sin()) * 0.02;
            let d = DVec3::new((f * 2.1).cos(), (f * 0.5).sin(), (f * 1.7).cos()) * 0.01;
            let e = DVec3::new((f * 0.9).sin(), (f * 2.9).cos(), (f * 0.2).cos()) * 0.01;
            tris.push(Near::of([c, c + d, c + e]));
        }
        let mut apart = 0;
        for p in &tris {
            for q in &tris {
                let d = triangle_distance(p.t, q.t).distance;
                for limit in [0.0, 0.001, 0.005, 0.01] {
                    if p.apart(q, limit) {
                        apart += 1;
                        assert!(d > limit, "{d} within {limit}");
                    }
                }
            }
        }
        assert!(apart > 1000, "{apart}");
    }
}
