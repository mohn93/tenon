//! The outline a section plane cuts out of triangles, and its fill.
//!
//! `section_loops` intersects triangles with a plane and chains the pieces into closed loops
//! (nested into outlines with holes) and open chains; `triangulate_section` fills the closed
//! ones. Neither panics on degenerate input: a triangle that is not finite, lies in the plane or
//! is a sliver contributes nothing or what little there is.

use std::collections::{BTreeMap, HashMap};

use glam::{DVec2, DVec3};

use crate::geom::{LENGTH_TOL, Plane, segment_distance_2d};

/// What a plane cuts out of some triangles, in the triangles' coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SectionLoops {
    /// Closed outlines with their holes: each is the outer loop first, then the holes inside
    /// it. Seen against the plane's normal an outer loop runs counter-clockwise and a hole
    /// clockwise. A loop does not repeat its first point at the end. Largest outline first.
    pub closed: Vec<Vec<Vec<DVec3>>>,
    /// Chains that do not close (a cut through a surface that is not closed), from end to end.
    pub open: Vec<Vec<DVec3>>,
}

/// How far off the plane, to the kept side, the inside test samples.
const SAMPLE_OFFSET: f64 = 1e-7;

/// What the plane through `point` with normal `normal` (any length but not zero) cuts out of
/// `triangles`, which are taken to be the surface of solids. The plane keeps what is on it and
/// on the side `normal` points to (as `SectionPlane::keeps`), so only triangles with a
/// corner strictly on the other side are cut: a triangle in the plane is not, and neither is
/// one touching it from the kept side. The pieces are noded (split where a corner lies on
/// another piece) into a planar arrangement; each bounded region of it is a closed loop with
/// its holes when it lies inside a solid (judged just on the kept side of the plane, by the
/// parity of the crossings of rays with `triangles`, most rays deciding). Pieces bounding no such region, and
/// dangling ones, are returned as open chains. Collinear points are dropped, so a loop's
/// points are its corners. Nothing is cut when a number is not finite or `normal` has no
/// direction.
pub fn section_loops(triangles: &[[DVec3; 3]], point: DVec3, normal: DVec3) -> SectionLoops {
    let Some(normal) = normal.try_normalize() else {
        return SectionLoops::default();
    };
    if !point.is_finite() {
        return SectionLoops::default();
    }
    let mut welder = Welder::default();
    // The cuts with the plane counting as the kept side (cut edges), and as the cut-away side
    // (which finds the outline of material that starts on the plane and goes to the kept
    // side; these pieces only outline fills and are not returned as chains).
    let mut ends: BTreeMap<(usize, usize), bool> = BTreeMap::new();
    for t in triangles {
        for on_plane_kept in [true, false] {
            if let Some([a, b]) = cut_triangle(t, point, normal, on_plane_kept) {
                let (a, b) = (welder.id(a), welder.id(b));
                if a != b {
                    *ends.entry((a.min(b), a.max(b))).or_default() |= on_plane_kept;
                }
            }
        }
    }
    let (u, v) = Plane { normal, d: 0.0 }.basis();
    let flat: Vec<DVec2> = welder
        .points
        .iter()
        .map(|p| DVec2::new((*p - point).dot(u), (*p - point).dot(v)))
        .collect();
    let (edges, cut) = node_edges(&ends, &flat);
    let hierarchy = Triangles::new(triangles);
    let solid = |s: DVec2| {
        let at = point + u * s.x + v * s.y + normal * SAMPLE_OFFSET;
        hierarchy.inside(at)
    };
    let found = Arrangement::new(&edges, &flat).solid_faces(&solid);

    let corners = |cycle: &[usize]| ring_corners(cycle.iter().map(|n| welder.points[*n]).collect());
    let mut closed: Vec<(f64, Vec<Vec<DVec3>>)> = Vec::new();
    for (area, rings) in &found.faces {
        let rings: Vec<Vec<DVec3>> = rings.iter().map(|c| corners(c)).collect();
        if rings[0].len() >= 3 {
            closed.push((*area, rings.into_iter().filter(|r| r.len() >= 3).collect()));
        }
    }
    closed.sort_by(|a, b| b.0.total_cmp(&a.0));

    // The pieces bounding no solid region, as chains.
    let mut adjacent: Vec<Vec<usize>> = vec![Vec::new(); welder.points.len()];
    for (k, (a, b)) in edges.iter().enumerate() {
        if cut[k] && !found.bounds_solid[k] {
            adjacent[*a].push(k);
            adjacent[*b].push(k);
        }
    }
    let mut used: Vec<bool> = (0..edges.len())
        .map(|k| !cut[k] || found.bounds_solid[k])
        .collect();
    let mut open: Vec<Vec<DVec3>> = Vec::new();
    let mut chains: Vec<Vec<usize>> = Vec::new();
    for node in 0..adjacent.len() {
        if adjacent[node].len() % 2 == 1 {
            while adjacent[node].iter().any(|e| !used[*e]) {
                chains.push(walk(node, &adjacent, &edges, &mut used));
            }
        }
    }
    for e in 0..edges.len() {
        if !used[e] {
            chains.push(walk(edges[e].0, &adjacent, &edges, &mut used));
        }
    }
    for chain in chains {
        let chain = chain_corners(chain.iter().map(|n| welder.points[*n]).collect());
        if chain.len() >= 2 {
            open.push(chain);
        }
    }
    SectionLoops {
        closed: closed.into_iter().map(|(_, rings)| rings).collect(),
        open,
    }
}

/// The cut of one triangle: the two ends of the piece of the plane inside it. None unless a
/// corner is strictly on the cut-away side and another on the kept side or on the plane.
fn cut_triangle(
    t: &[DVec3; 3],
    point: DVec3,
    normal: DVec3,
    on_plane_kept: bool,
) -> Option<[DVec3; 2]> {
    if !t.iter().all(|p| p.is_finite()) {
        return None;
    }
    let d = t.map(|p| (p - point).dot(normal));
    // As `SectionPlane::keeps`: on the plane counts as the kept side (or not).
    let kept = d.map(|x| {
        if on_plane_kept {
            x >= -KEEP_TOL
        } else {
            x > KEEP_TOL
        }
    });
    if kept == [true; 3] || kept == [false; 3] {
        return None;
    }
    let mut cuts: Vec<DVec3> = Vec::with_capacity(2);
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        if kept[i] != kept[j] {
            let k = d[i] / (d[i] - d[j]);
            cuts.push(t[i] + (t[j] - t[i]) * k);
        }
    }
    match cuts[..] {
        [a, b] if a.distance(b) >= LENGTH_TOL && a.is_finite() && b.is_finite() => Some([a, b]),
        _ => None,
    }
}

/// The tolerance `SectionPlane::keeps` has.
const KEEP_TOL: f64 = 1e-9;

/// The directions `Triangles::inside` casts rays in: the six axes, each tilted a little so
/// that no ray runs along the edges of axis-aligned solids. A face shared by two solids
/// misleads the rays that meet it first; being near an axis each, only one of the six does.
const RAYS: [[f64; 3]; 6] = [
    [1.0, 0.123, 0.071],
    [-1.0, 0.087, -0.131],
    [0.097, 1.0, -0.113],
    [-0.059, -1.0, 0.127],
    [0.083, 0.141, 1.0],
    [-0.109, 0.067, -1.0],
];

/// Rays that must cross an odd number of triangles for a point to be inside.
const INSIDE_VOTES: usize = 4;

/// Triangles with a bounding volume hierarchy, for the nearest triangle along a ray.
struct Triangles<'a> {
    triangles: &'a [[DVec3; 3]],
    /// Triangle numbers, grouped by leaf.
    order: Vec<usize>,
    nodes: Vec<BvhNode>,
}

struct BvhNode {
    lo: DVec3,
    hi: DVec3,
    /// A leaf: the range of `order`; else the first child is the next node and this is the
    /// second child's number.
    kind: BvhKind,
}

enum BvhKind {
    Leaf(usize, usize),
    Inner(usize),
}

const LEAF_SIZE: usize = 4;

impl<'a> Triangles<'a> {
    fn new(triangles: &'a [[DVec3; 3]]) -> Self {
        let mut me = Triangles {
            triangles,
            order: (0..triangles.len()).collect(),
            nodes: Vec::new(),
        };
        if !triangles.is_empty() {
            me.build(0, triangles.len());
        }
        me
    }

    fn build(&mut self, start: usize, end: usize) -> usize {
        let at = self.nodes.len();
        let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        let (mut clo, mut chi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        for &i in &self.order[start..end] {
            let t = &self.triangles[i];
            let c = (t[0] + t[1] + t[2]) / 3.0;
            clo = clo.min(c);
            chi = chi.max(c);
            for p in t {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
        }
        self.nodes.push(BvhNode {
            lo,
            hi,
            kind: BvhKind::Leaf(start, end),
        });
        if end - start <= LEAF_SIZE {
            return at;
        }
        let size = chi - clo;
        let axis = if size.x >= size.y && size.x >= size.z {
            0
        } else if size.y >= size.z {
            1
        } else {
            2
        };
        let triangles = self.triangles;
        let centre = |i: &usize| {
            let t = &triangles[*i];
            ((t[0] + t[1] + t[2]) / 3.0)[axis]
        };
        let mid = (start + end) / 2;
        self.order[start..end]
            .select_nth_unstable_by(mid - start, |a, b| centre(a).total_cmp(&centre(b)));
        self.build(start, mid);
        let second = self.build(mid, end);
        self.nodes[at].kind = BvhKind::Inner(second);
        at
    }

    /// How many triangles the ray from `origin` along `dir` meets.
    fn crossings(&self, origin: DVec3, dir: DVec3) -> usize {
        if self.nodes.is_empty() {
            return 0;
        }
        let inv = DVec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut count = 0;
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n];
            if !slab(node.lo, node.hi, origin, inv) {
                continue;
            }
            match node.kind {
                BvhKind::Leaf(a, b) => {
                    count += self.order[a..b]
                        .iter()
                        .filter(|i| ray_hit(&self.triangles[**i], origin, dir))
                        .count();
                }
                BvhKind::Inner(second) => {
                    stack.push(second);
                    stack.push(n + 1);
                }
            }
        }
        count
    }

    /// Whether `p` is inside the solids bounded by the triangles: an odd number of triangles
    /// lie in front of it along at least `INSIDE_VOTES` of the `RAYS`. A face two solids share
    /// exists once, which turns the count even for the rays that meet it; the others still
    /// say inside.
    fn inside(&self, p: DVec3) -> bool {
        let votes = RAYS
            .iter()
            .filter(|d| self.crossings(p, DVec3::from_array(**d).normalize()) % 2 == 1)
            .count();
        votes >= INSIDE_VOTES
    }
}

/// Whether the ray (`inv` the reciprocals of its direction) meets the box.
fn slab(lo: DVec3, hi: DVec3, origin: DVec3, inv: DVec3) -> bool {
    let (mut near, mut far) = (0.0_f64, f64::INFINITY);
    for k in 0..3 {
        let (a, b) = ((lo[k] - origin[k]) * inv[k], (hi[k] - origin[k]) * inv[k]);
        if a.is_nan() || b.is_nan() {
            // On the slab's plane along a ray parallel to it: inside when between.
            if origin[k] < lo[k] || origin[k] > hi[k] {
                return false;
            }
            continue;
        }
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    near <= far
}

/// Whether the ray from `origin` along `dir` meets the triangle (Moeller-Trumbore), its edges
/// half-open so a ray through a seam meets one side only.
fn ray_hit(t: &[DVec3; 3], origin: DVec3, dir: DVec3) -> bool {
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let h = dir.cross(e2);
    let a = e1.dot(h);
    if a.abs() <= 1e-14 || a.is_nan() {
        return false;
    }
    let f = 1.0 / a;
    let s = origin - t[0];
    let u = f * s.dot(h);
    if !(0.0..1.0).contains(&u) {
        return false;
    }
    let q = s.cross(e1);
    let v = f * dir.dot(q);
    if !(v >= 0.0 && u + v < 1.0) {
        return false;
    }
    f * e2.dot(q) > 1e-12
}

/// `edges` split at the nodes lying on them (a corner on another piece), without duplicates:
/// pieces that overlap end up as the same pieces. With each piece, whether any edge it comes
/// from was flagged.
fn node_edges(
    edges: &BTreeMap<(usize, usize), bool>,
    flat: &[DVec2],
) -> (Vec<(usize, usize)>, Vec<bool>) {
    let mut order: Vec<usize> = (0..flat.len()).collect();
    order.sort_by(|a, b| flat[*a].x.total_cmp(&flat[*b].x));
    let mut out: BTreeMap<(usize, usize), bool> = BTreeMap::new();
    for (&(a, b), &flag) in edges {
        let (pa, pb) = (flat[a], flat[b]);
        let (lo, hi) = (pa.x.min(pb.x) - LENGTH_TOL, pa.x.max(pb.x) + LENGTH_TOL);
        let first = order.partition_point(|n| flat[*n].x < lo);
        let along = (pb - pa).length_squared();
        let mut on: Vec<(f64, usize)> = vec![(0.0, a), (1.0, b)];
        for n in order[first..].iter().take_while(|n| flat[**n].x <= hi) {
            if *n != a && *n != b && segment_distance_2d(flat[*n], pa, pb) < LENGTH_TOL {
                on.push(((flat[*n] - pa).dot(pb - pa) / along, *n));
            }
        }
        on.sort_by(|x, y| x.0.total_cmp(&y.0));
        for w in on.windows(2) {
            let (x, y) = (w[0].1, w[1].1);
            if x != y {
                *out.entry((x.min(y), x.max(y))).or_default() |= flag;
            }
        }
    }
    out.into_iter().unzip()
}

/// The regions a set of pieces (edges between nodes) cuts the plane into.
struct Arrangement<'a> {
    edges: &'a [(usize, usize)],
    flat: &'a [DVec2],
}

/// The solid regions of an `Arrangement`.
struct Solids {
    /// Each solid region: its area, then its outline (counter-clockwise) and holes (clockwise)
    /// as cycles of nodes.
    faces: Vec<(f64, Vec<Vec<usize>>)>,
    /// Per edge: whether it bounds a solid region.
    bounds_solid: Vec<bool>,
}

impl<'a> Arrangement<'a> {
    fn new(edges: &'a [(usize, usize)], flat: &'a [DVec2]) -> Self {
        Arrangement { edges, flat }
    }

    /// The node half-edge `h` (edge `h / 2`, backwards when odd) starts from.
    fn from(&self, h: usize) -> usize {
        let e = self.edges[h / 2];
        if h.is_multiple_of(2) { e.0 } else { e.1 }
    }

    fn to(&self, h: usize) -> usize {
        self.from(h ^ 1)
    }

    /// The regions whose sample point `solid` accepts.
    fn solid_faces(&self, solid: &dyn Fn(DVec2) -> bool) -> Solids {
        let n = self.flat.len();
        let mut adjacent: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (k, (a, b)) in self.edges.iter().enumerate() {
            adjacent[*a].push(k);
            adjacent[*b].push(k);
        }
        let mut alive = vec![true; self.edges.len()];
        // Take away dangling pieces and bridges (a piece with the same region on both sides)
        // until none is left: what remains outlines regions.
        let cycles = loop {
            let mut degree: Vec<usize> = adjacent
                .iter()
                .map(|l| l.iter().filter(|e| alive[**e]).count())
                .collect();
            let mut stack: Vec<usize> = (0..n).filter(|v| degree[*v] == 1).collect();
            while let Some(v) = stack.pop() {
                let Some(e) = adjacent[v].iter().copied().find(|e| alive[*e]) else {
                    continue;
                };
                if degree[v] != 1 {
                    continue;
                }
                alive[e] = false;
                let (a, b) = self.edges[e];
                let w = if a == v { b } else { a };
                degree[v] -= 1;
                degree[w] -= 1;
                if degree[w] == 1 {
                    stack.push(w);
                }
            }
            let (cycles, cycle_of) = self.cycles(&alive);
            let bridges: Vec<usize> = (0..self.edges.len())
                .filter(|e| alive[*e] && cycle_of[2 * e] == cycle_of[2 * e + 1])
                .collect();
            if bridges.is_empty() {
                break cycles;
            }
            for e in bridges {
                alive[e] = false;
            }
        };

        // The connected pieces.
        let mut parent: Vec<usize> = (0..n).collect();
        fn root(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        for (k, (a, b)) in self.edges.iter().enumerate() {
            if alive[k] {
                let (ra, rb) = (root(&mut parent, *a), root(&mut parent, *b));
                parent[ra] = rb;
            }
        }

        struct Cycle {
            half_edges: Vec<usize>,
            nodes: Vec<usize>,
            ring: Ring2,
            component: usize,
        }
        let mut all: Vec<Cycle> = Vec::new();
        for c in cycles {
            let nodes: Vec<usize> = c.iter().map(|h| self.from(*h)).collect();
            let ring = Ring2::new(nodes.iter().map(|n| self.flat[*n]).collect());
            let component = root(&mut parent, nodes[0]);
            all.push(Cycle {
                half_edges: c,
                nodes,
                ring,
                component,
            });
        }
        let tiny = LENGTH_TOL * LENGTH_TOL;
        let faces: Vec<usize> = (0..all.len())
            .filter(|k| all[*k].ring.area > tiny)
            .collect();
        // A piece's outer boundary (clockwise) is a hole of the smallest region of another
        // piece around it.
        let mut holes: HashMap<usize, Vec<usize>> = HashMap::new();
        for k in (0..all.len()).filter(|k| all[*k].ring.area < -tiny) {
            let around = faces
                .iter()
                .copied()
                .filter(|f| {
                    all[*f].component != all[k].component
                        && all[*f].ring.area > -all[k].ring.area
                        && all[*f].ring.contains(&all[k].ring)
                })
                .min_by(|a, b| all[*a].ring.area.total_cmp(&all[*b].ring.area));
            if let Some(f) = around {
                holes.entry(f).or_default().push(k);
            }
        }

        let mut out = Solids {
            faces: Vec::new(),
            bounds_solid: vec![false; self.edges.len()],
        };
        for f in faces {
            let cycles: Vec<usize> = std::iter::once(f)
                .chain(holes.get(&f).into_iter().flatten().copied())
                .collect();
            let rings: Vec<&Ring2> = cycles.iter().map(|c| &all[*c].ring).collect();
            let Some(sample) = interior_point(&rings) else {
                continue;
            };
            if solid(sample) {
                for c in &cycles {
                    for h in &all[*c].half_edges {
                        out.bounds_solid[h / 2] = true;
                    }
                }
                out.faces.push((
                    all[f].ring.area,
                    cycles.iter().map(|c| all[*c].nodes.clone()).collect(),
                ));
            }
        }
        out
    }

    /// The boundary cycles of the regions of the live pieces, as half-edges with the region on
    /// their left, and the cycle each half-edge is in.
    fn cycles(&self, alive: &[bool]) -> (Vec<Vec<usize>>, Vec<usize>) {
        let n = self.flat.len();
        let mut around: Vec<Vec<usize>> = vec![Vec::new(); n];
        for e in (0..self.edges.len()).filter(|e| alive[*e]) {
            around[self.from(2 * e)].push(2 * e);
            around[self.from(2 * e + 1)].push(2 * e + 1);
        }
        let angle = |h: usize| {
            let d = self.flat[self.to(h)] - self.flat[self.from(h)];
            d.y.atan2(d.x)
        };
        let mut slot = vec![0; 2 * self.edges.len()];
        for list in &mut around {
            list.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
            for (i, h) in list.iter().enumerate() {
                slot[*h] = i;
            }
        }
        // At the end of a half-edge, the next one is the first clockwise from the way back.
        let next = |h: usize| {
            let list = &around[self.to(h)];
            list[(slot[h ^ 1] + list.len() - 1) % list.len()]
        };
        let mut cycle_of = vec![usize::MAX; 2 * self.edges.len()];
        let mut cycles = Vec::new();
        for e in (0..self.edges.len()).filter(|e| alive[*e]) {
            for start in [2 * e, 2 * e + 1] {
                if cycle_of[start] != usize::MAX {
                    continue;
                }
                let mut cycle = Vec::new();
                let mut h = start;
                while cycle_of[h] == usize::MAX {
                    cycle_of[h] = cycles.len();
                    cycle.push(h);
                    h = next(h);
                }
                cycles.push(cycle);
            }
        }
        (cycles, cycle_of)
    }
}

/// A point inside the region bounded by `rings` (the outline first, then the holes): the
/// middle of the largest triangle of its triangulation.
fn interior_point(rings: &[&Ring2]) -> Option<DVec2> {
    let mut points: Vec<DVec2> = Vec::new();
    let mut coords: Vec<f64> = Vec::new();
    let mut hole_starts: Vec<usize> = Vec::new();
    for (k, r) in rings.iter().enumerate() {
        if k > 0 {
            hole_starts.push(points.len());
        }
        for p in &r.points {
            points.push(*p);
            coords.extend([p.x, p.y]);
        }
    }
    let indices = earcutr::earcut(&coords, &hole_starts, 2).ok()?;
    indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| [points[c[0]], points[c[1]], points[c[2]]])
        .max_by(|a, b| {
            let area = |t: &[DVec2; 3]| (t[1] - t[0]).perp_dot(t[2] - t[0]).abs();
            area(a).total_cmp(&area(b))
        })
        .filter(|t| (t[1] - t[0]).perp_dot(t[2] - t[0]).abs() > 0.0)
        .map(|t| (t[0] + t[1] + t[2]) / 3.0)
}

/// Numbers the points, giving points within `LENGTH_TOL` of each other one number.
#[derive(Default)]
struct Welder {
    points: Vec<DVec3>,
    cells: HashMap<[i64; 3], Vec<usize>>,
}

impl Welder {
    fn id(&mut self, p: DVec3) -> usize {
        let cell = (p / LENGTH_TOL).floor().to_array().map(|c| c as i64);
        for dx in -1..=1_i64 {
            for dy in -1..=1_i64 {
                for dz in -1..=1_i64 {
                    let key = [
                        cell[0].saturating_add(dx),
                        cell[1].saturating_add(dy),
                        cell[2].saturating_add(dz),
                    ];
                    for n in self.cells.get(&key).into_iter().flatten() {
                        if self.points[*n].distance(p) < LENGTH_TOL {
                            return *n;
                        }
                    }
                }
            }
        }
        let n = self.points.len();
        self.points.push(p);
        self.cells.entry(cell).or_default().push(n);
        n
    }
}

/// The nodes along the unused pieces from `start`, taking a piece at each node while one is
/// left; the pieces taken are marked used.
fn walk(
    start: usize,
    adjacent: &[Vec<usize>],
    edges: &[(usize, usize)],
    used: &mut [bool],
) -> Vec<usize> {
    let mut path = vec![start];
    let mut at = start;
    while let Some(e) = adjacent[at].iter().copied().find(|e| !used[*e]) {
        used[e] = true;
        at = if edges[e].0 == at {
            edges[e].1
        } else {
            edges[e].0
        };
        path.push(at);
    }
    path
}

/// Distance from `p` to the segment a-b.
fn segment_distance(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let l2 = ab.length_squared();
    let t = if l2 > 0.0 {
        ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t).distance(p)
}

/// `points` without the ones on the segment between their neighbours.
fn chain_corners(points: Vec<DVec3>) -> Vec<DVec3> {
    let mut out: Vec<DVec3> = Vec::with_capacity(points.len());
    for q in points {
        while out.len() >= 2
            && segment_distance(out[out.len() - 1], out[out.len() - 2], q) < LENGTH_TOL
        {
            out.pop();
        }
        out.push(q);
    }
    out
}

/// A closed ring without the points on the segment between their neighbours.
fn ring_corners(points: Vec<DVec3>) -> Vec<DVec3> {
    let mut out = chain_corners(points);
    loop {
        let n = out.len();
        if n < 3 {
            return out;
        }
        if segment_distance(out[n - 1], out[n - 2], out[0]) < LENGTH_TOL {
            out.pop();
        } else if segment_distance(out[0], out[n - 1], out[1]) < LENGTH_TOL {
            out.remove(0);
        } else {
            return out;
        }
    }
}

/// A ring in the plane's 2D frame.
struct Ring2 {
    points: Vec<DVec2>,
    /// Signed area: positive counter-clockwise.
    area: f64,
    lo: DVec2,
    hi: DVec2,
}

impl Ring2 {
    fn new(points: Vec<DVec2>) -> Ring2 {
        let area = 0.5
            * (0..points.len())
                .map(|i| points[i].perp_dot(points[(i + 1) % points.len()]))
                .sum::<f64>();
        let lo = points.iter().fold(DVec2::INFINITY, |a, p| a.min(*p));
        let hi = points.iter().fold(DVec2::NEG_INFINITY, |a, p| a.max(*p));
        Ring2 {
            points,
            area,
            lo,
            hi,
        }
    }

    /// Whether the ring contains `inner` (a ring with less area): by where most of `inner`'s
    /// points are, those on this ring's outline not counting.
    fn contains(&self, inner: &Ring2) -> bool {
        let tol = DVec2::splat(LENGTH_TOL);
        if inner.lo.cmplt(self.lo - tol).any() || inner.hi.cmpgt(self.hi + tol).any() {
            return false;
        }
        let step = inner.points.len().div_ceil(8).max(1);
        let (mut inside, mut outside) = (0, 0);
        for q in inner.points.iter().step_by(step) {
            let n = self.points.len();
            let on_outline = (0..n).any(|i| {
                segment_distance_2d(*q, self.points[i], self.points[(i + 1) % n]) < LENGTH_TOL
            });
            if on_outline {
                continue;
            }
            if self.encloses(*q) {
                inside += 1;
            } else {
                outside += 1;
            }
        }
        inside > outside
    }

    /// Even-odd test of a point not on the outline.
    fn encloses(&self, q: DVec2) -> bool {
        let n = self.points.len();
        let mut inside = false;
        for i in 0..n {
            let (a, b) = (self.points[i], self.points[(i + 1) % n]);
            if (a.y > q.y) != (b.y > q.y) && q.x < a.x + (q.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                inside = !inside;
            }
        }
        inside
    }
}

/// Triangles filling the closed loops `section_loops` returns (each an outline and its
/// holes), wound counter-clockwise about `normal`. A group whose outline has fewer than three
/// points or a point that is not finite is left out, as a hole with fewer than three; nothing
/// is filled when `normal` has no direction.
pub fn triangulate_section(loops: &[Vec<Vec<DVec3>>], normal: DVec3) -> Vec<[DVec3; 3]> {
    let Some(normal) = normal.try_normalize() else {
        return Vec::new();
    };
    let (u, v) = Plane { normal, d: 0.0 }.basis();
    let mut out = Vec::new();
    for group in loops {
        let usable = |r: &Vec<DVec3>| r.len() >= 3 && r.iter().all(|p| p.is_finite());
        // An outline that is unusable takes its holes with it.
        let Some(outline) = group.first().filter(|r| usable(r)) else {
            continue;
        };
        let origin = outline[0];
        let rings: Vec<&Vec<DVec3>> = std::iter::once(outline)
            .chain(group[1..].iter().filter(|r| usable(r)))
            .collect();
        let mut points: Vec<DVec3> = Vec::new();
        let mut coords: Vec<f64> = Vec::new();
        let mut hole_starts: Vec<usize> = Vec::new();
        for (k, ring) in rings.iter().enumerate() {
            if k > 0 {
                hole_starts.push(points.len());
            }
            for p in ring.iter() {
                points.push(*p);
                coords.extend([(*p - origin).dot(u), (*p - origin).dot(v)]);
            }
        }
        let Ok(indices) = earcutr::earcut(&coords, &hole_starts, 2) else {
            continue;
        };
        for c in indices.as_chunks::<3>().0 {
            let t = [points[c[0]], points[c[1]], points[c[2]]];
            let facing = (t[1] - t[0]).cross(t[2] - t[0]).dot(normal);
            if facing > 0.0 {
                out.push(t);
            } else if facing < 0.0 {
                out.push([t[0], t[2], t[1]]);
            }
        }
    }
    out
}
