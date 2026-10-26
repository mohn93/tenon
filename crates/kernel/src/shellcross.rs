//! Closed shells that cross themselves: faces of one shell whose triangles meet beyond the
//! vertices and edges they share (a corner pushed through the opposite wall, a face folded
//! back over its neighbour).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::DVec3;

use crate::geom::LENGTH_TOL;
use crate::ids::{FaceId, VertexId};
use crate::model::Model;

/// Most grid cells one triangle is entered in; a larger one is tested against every other.
const MAX_CELLS: usize = 4096;

/// How near a plane a corner counts as on it when sections of triangles are taken; far
/// finer than `LENGTH_TOL`, so the fans of a sphere's poles (nearly flat, sharing a vertex)
/// do not read as overlapping.
const ON_PLANE: f64 = 1e-11;

/// Most cells along one axis of the grid.
const MAX_AXIS: f64 = 128.0;

struct Tri {
    face: FaceId,
    shell: usize,
    ids: [VertexId; 3],
    p: [DVec3; 3],
    lo: DVec3,
    hi: DVec3,
}

/// The faces of closed shells of `m` that cross another face of their own shell, in id order.
pub fn crossing_faces(m: &Model) -> Vec<FaceId> {
    let Some(shell_of) = closed_shells(m) else {
        return Vec::new();
    };
    let mut tris: Vec<Tri> = Vec::new();
    for (&face, &shell) in &shell_of {
        for t in m.triangle_vertices(face).unwrap_or_default() {
            let p = t.map(|v| m.pos(v));
            tris.push(Tri {
                face,
                shell,
                ids: t,
                lo: p[0].min(p[1]).min(p[2]),
                hi: p[0].max(p[1]).max(p[2]),
                p,
            });
        }
    }
    let mut out: BTreeSet<FaceId> = BTreeSet::new();
    let mut test = |a: &Tri, b: &Tri| {
        if a.face == b.face
            || a.shell != b.shell
            || (out.contains(&a.face) && out.contains(&b.face))
            || a.lo.cmpgt(b.hi + DVec3::splat(LENGTH_TOL)).any()
            || b.lo.cmpgt(a.hi + DVec3::splat(LENGTH_TOL)).any()
        {
            return;
        }
        let shared: Vec<usize> = (0..3).filter(|i| b.ids.contains(&a.ids[*i])).collect();
        if cross(a, b, &shared) {
            out.insert(a.face);
            out.insert(b.face);
        }
    };
    if tris.len() < 2 {
        return Vec::new();
    }
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    let mut mean = 0.0;
    for t in &tris {
        lo = lo.min(t.lo);
        hi = hi.max(t.hi);
        mean += (t.hi - t.lo).max_element();
    }
    mean /= tris.len() as f64;
    let cell = (2.0 * mean)
        .max((hi - lo).max_element() / MAX_AXIS)
        .max(LENGTH_TOL);
    let at = |p: DVec3| ((p - lo) / cell).floor().as_ivec3();
    let mut grid: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
    let mut big: Vec<usize> = Vec::new();
    for (k, t) in tris.iter().enumerate() {
        let (c0, c1) = (at(t.lo), at(t.hi));
        let n = (c1 - c0 + 1).as_i64vec3();
        if n.x * n.y * n.z > MAX_CELLS as i64 {
            big.push(k);
            continue;
        }
        for x in c0.x..=c1.x {
            for y in c0.y..=c1.y {
                for z in c0.z..=c1.z {
                    grid.entry([x, y, z]).or_default().push(k);
                }
            }
        }
    }
    for (c, list) in &grid {
        for (i, &a) in list.iter().enumerate() {
            for &b in &list[i + 1..] {
                // A pair is met in every cell they share; test it in the one holding the low
                // corner of their boxes' overlap.
                let corner = at(tris[a].lo.max(tris[b].lo));
                if corner.to_array() == *c {
                    test(&tris[a], &tris[b]);
                }
            }
        }
    }
    for (i, &a) in big.iter().enumerate() {
        for b in (0..tris.len()).filter(|b| !big[..i].contains(b)) {
            if b != a {
                test(&tris[a], &tris[b]);
            }
        }
    }
    out.into_iter().collect()
}

/// The closed shells of `m` (faces reached across sides used by exactly two faces, every side
/// of them so used) as a face to shell number map; None when there is none.
fn closed_shells(m: &Model) -> Option<BTreeMap<FaceId, usize>> {
    let mut by_side: BTreeMap<(VertexId, VertexId), Vec<FaceId>> = BTreeMap::new();
    for (id, f) in m.faces() {
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                by_side.entry((a.min(b), a.max(b))).or_default().push(id);
            }
        }
    }
    let mut shell: BTreeMap<FaceId, usize> = BTreeMap::new();
    let mut closed: Vec<bool> = Vec::new();
    for (start, _) in m.faces() {
        if shell.contains_key(&start) {
            continue;
        }
        let k = closed.len();
        closed.push(true);
        shell.insert(start, k);
        let mut stack = vec![start];
        while let Some(f) = stack.pop() {
            let face = m.face(f).expect("listed");
            for l in std::iter::once(&face.outer).chain(&face.holes) {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    let list = &by_side[&(a.min(b), a.max(b))];
                    if list.len() != 2 {
                        closed[k] = false;
                        continue;
                    }
                    for &g in list {
                        if shell.insert(g, k).is_none() {
                            stack.push(g);
                        }
                    }
                }
            }
        }
    }
    shell.retain(|_, k| closed[*k]);
    (!shell.is_empty()).then_some(shell)
}

/// Whether triangles `a` and `b`, sharing the corners `shared` of `a`, meet beyond what they
/// share.
fn cross(a: &Tri, b: &Tri, shared: &[usize]) -> bool {
    let normal = |t: &[DVec3; 3]| (t[1] - t[0]).cross(t[2] - t[0]).try_normalize();
    let (Some(na), Some(nb)) = (normal(&a.p), normal(&b.p)) else {
        return false;
    };
    let dist_b: [f64; 3] = std::array::from_fn(|i| (b.p[i] - a.p[0]).dot(na));
    let dist_a: [f64; 3] = std::array::from_fn(|i| (a.p[i] - b.p[0]).dot(nb));
    let flat = |d: &[f64; 3]| d.iter().all(|x| x.abs() <= LENGTH_TOL);
    let apart = |d: &[f64; 3]| d.iter().all(|x| *x > ON_PLANE) || d.iter().all(|x| *x < -ON_PLANE);
    if flat(&dist_a) && flat(&dist_b) {
        return overlap_in_plane(&a.p, &b.p, na);
    }
    if apart(&dist_a) || apart(&dist_b) {
        return false;
    }
    let Some(line) = na.cross(nb).try_normalize() else {
        return false;
    };
    let (Some(ia), Some(ib)) = (interval(&a.p, &dist_a, line), interval(&b.p, &dist_b, line))
    else {
        return false;
    };
    let common = ia.1.min(ib.1) - ia.0.max(ib.0);
    let legit = if shared.len() >= 2 {
        a.p[shared[0]].distance(a.p[shared[1]])
    } else {
        0.0
    };
    common > legit + LENGTH_TOL
}

/// The extent along `line` of the part of triangle `p` on plane 0 (`dist` are its corners'
/// distances to it), None when it does not reach it.
fn interval(p: &[DVec3; 3], dist: &[f64; 3], line: DVec3) -> Option<(f64, f64)> {
    let mut ts: Vec<f64> = Vec::new();
    for i in 0..3 {
        let j = (i + 1) % 3;
        if dist[i].abs() <= ON_PLANE {
            ts.push(line.dot(p[i]));
        }
        if (dist[i] > ON_PLANE && dist[j] < -ON_PLANE)
            || (dist[i] < -ON_PLANE && dist[j] > ON_PLANE)
        {
            let s = dist[i] / (dist[i] - dist[j]);
            ts.push(line.dot(p[i] + (p[j] - p[i]) * s));
        }
    }
    let lo = ts.iter().copied().min_by(f64::total_cmp)?;
    let hi = ts.iter().copied().max_by(f64::total_cmp)?;
    Some((lo, hi))
}

/// Whether two triangles in one plane (normal `n`) overlap in more than their boundaries:
/// no side of either has the other wholly on its far side.
fn overlap_in_plane(a: &[DVec3; 3], b: &[DVec3; 3], n: DVec3) -> bool {
    for t in [a, b] {
        for i in 0..3 {
            let Some(axis) = n.cross(t[(i + 1) % 3] - t[i]).try_normalize() else {
                continue;
            };
            let range = |q: &[DVec3; 3]| {
                let d: [f64; 3] = std::array::from_fn(|k| axis.dot(q[k]));
                (
                    d.iter().copied().fold(f64::INFINITY, f64::min),
                    d.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                )
            };
            let (ra, rb) = (range(a), range(b));
            if ra.1 <= rb.0 + LENGTH_TOL || rb.1 <= ra.0 + LENGTH_TOL {
                return false;
            }
        }
    }
    true
}
