//! The outline a section plane cuts out of triangles, and its fill.
//!
//! `section_loops` intersects triangles with a plane and chains the pieces into closed loops
//! (nested into outlines with holes) and open chains; `triangulate_section` fills the closed
//! ones. Neither panics on degenerate input: a triangle that is not finite, lies in the plane or
//! is a sliver contributes nothing or what little there is.

use std::collections::{BTreeSet, HashMap};

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

/// What the plane through `point` with normal `normal` (any length but not zero) cuts out of
/// `triangles`. A triangle lying in the plane is skipped: its neighbours' cuts outline it.
/// Collinear points are dropped, so a loop's points are its corners. Nothing is cut when a
/// number is not finite or `normal` has no direction.
pub fn section_loops(triangles: &[[DVec3; 3]], point: DVec3, normal: DVec3) -> SectionLoops {
    let Some(normal) = normal.try_normalize() else {
        return SectionLoops::default();
    };
    if !point.is_finite() {
        return SectionLoops::default();
    }
    let mut welder = Welder::default();
    let mut ends: BTreeSet<(usize, usize)> = BTreeSet::new();
    for t in triangles {
        if let Some([a, b]) = cut_triangle(t, point, normal) {
            let (a, b) = (welder.id(a), welder.id(b));
            if a != b {
                ends.insert((a.min(b), a.max(b)));
            }
        }
    }
    let edges: Vec<(usize, usize)> = ends.into_iter().collect();
    let mut adjacent: Vec<Vec<usize>> = vec![Vec::new(); welder.points.len()];
    for (k, (a, b)) in edges.iter().enumerate() {
        adjacent[*a].push(k);
        adjacent[*b].push(k);
    }

    let mut used = vec![false; edges.len()];
    let mut chains: Vec<Vec<usize>> = Vec::new();
    // Chains from the nodes where an odd number of pieces meet: ends of open chains.
    for node in 0..adjacent.len() {
        if adjacent[node].len() % 2 == 1 {
            while adjacent[node].iter().any(|e| !used[*e]) {
                chains.push(walk(node, &adjacent, &edges, &mut used));
            }
        }
    }
    // What is left closes up.
    for e in 0..edges.len() {
        if !used[e] {
            chains.push(walk(edges[e].0, &adjacent, &edges, &mut used));
        }
    }

    let mut rings: Vec<Vec<DVec3>> = Vec::new();
    let mut open: Vec<Vec<DVec3>> = Vec::new();
    for chain in chains {
        let mut points: Vec<DVec3> = chain.iter().map(|n| welder.points[*n]).collect();
        if chain.len() >= 4 && chain.first() == chain.last() {
            points.pop();
            let ring = ring_corners(points);
            if ring.len() >= 3 {
                rings.push(ring);
            }
        } else {
            let chain = chain_corners(points);
            if chain.len() >= 2 {
                open.push(chain);
            }
        }
    }
    SectionLoops {
        closed: nest(rings, point, normal),
        open,
    }
}

/// The cut of one triangle: the two ends of the piece of the plane inside it. None when the
/// plane misses it, touches it at one point, or contains it.
fn cut_triangle(t: &[DVec3; 3], point: DVec3, normal: DVec3) -> Option<[DVec3; 2]> {
    if !t.iter().all(|p| p.is_finite()) {
        return None;
    }
    let d = t.map(|p| (p - point).dot(normal));
    let side = d.map(|x| {
        if x > LENGTH_TOL {
            1
        } else if x < -LENGTH_TOL {
            -1
        } else {
            0
        }
    });
    if side == [0; 3] {
        return None;
    }
    // The corners on the plane, and where the sides from one side to the other cross it. A
    // corner on the plane and the side opposite it can only give two points in all.
    let mut cuts: Vec<DVec3> = Vec::with_capacity(3);
    for i in 0..3 {
        if side[i] == 0 {
            cuts.push(t[i] - normal * d[i]);
        }
    }
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        if side[i] * side[j] < 0 {
            let k = d[i] / (d[i] - d[j]);
            cuts.push(t[i] + (t[j] - t[i]) * k);
        }
    }
    match cuts[..] {
        [a, b] if a.distance(b) >= LENGTH_TOL && a.is_finite() && b.is_finite() => Some([a, b]),
        _ => None,
    }
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
    fn new(ring: &[DVec3], origin: DVec3, u: DVec3, v: DVec3) -> Ring2 {
        let points: Vec<DVec2> = ring
            .iter()
            .map(|p| DVec2::new((*p - origin).dot(u), (*p - origin).dot(v)))
            .collect();
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

/// Rings nested even-odd: a ring inside an even number of others is an outline, inside an odd
/// number a hole of the smallest ring around it.
fn nest(rings: Vec<Vec<DVec3>>, point: DVec3, normal: DVec3) -> Vec<Vec<Vec<DVec3>>> {
    if rings.is_empty() {
        return Vec::new();
    }
    let (u, v) = Plane { normal, d: 0.0 }.basis();
    let flat: Vec<Ring2> = rings.iter().map(|r| Ring2::new(r, point, u, v)).collect();
    // Largest first, so a ring can only be inside the ones before it.
    let mut order: Vec<usize> = (0..rings.len())
        .filter(|k| flat[*k].area.abs() > LENGTH_TOL * LENGTH_TOL)
        .collect();
    order.sort_by(|a, b| flat[*b].area.abs().total_cmp(&flat[*a].area.abs()));

    // For each ring (by position in `order`): the rings around it, the smallest last.
    let around: Vec<Vec<usize>> = (0..order.len())
        .map(|k| {
            (0..k)
                .filter(|j| flat[order[*j]].contains(&flat[order[k]]))
                .collect()
        })
        .collect();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group_of: HashMap<usize, usize> = HashMap::new();
    for k in 0..order.len() {
        let hole_of = around[k]
            .last()
            .filter(|j| around[k].len() % 2 == 1 && around[**j].len().is_multiple_of(2));
        match hole_of {
            Some(j) => groups[group_of[j]].push(order[k]),
            None => {
                group_of.insert(k, groups.len());
                groups.push(vec![order[k]]);
            }
        }
    }
    groups
        .into_iter()
        .map(|g| {
            g.into_iter()
                .enumerate()
                .map(|(i, r)| {
                    let mut ring = rings[r].clone();
                    // Outline counter-clockwise, holes clockwise.
                    if (flat[r].area > 0.0) != (i == 0) {
                        ring.reverse();
                    }
                    ring
                })
                .collect()
        })
        .collect()
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
