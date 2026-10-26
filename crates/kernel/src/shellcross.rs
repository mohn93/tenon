//! Closed shells that cross themselves: faces of one shell whose triangles meet beyond the
//! vertices and edges they share (a corner pushed through the opposite wall, a face folded
//! back over its neighbour).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::DVec3;

use crate::geom::LENGTH_TOL;
use crate::ids::{FaceId, VertexId};
use crate::model::Model;

/// How near a plane a corner counts as on it when sections of triangles are taken; far
/// finer than `LENGTH_TOL`, so the fans of a sphere's poles (nearly flat, sharing a vertex)
/// do not read as overlapping.
const ON_PLANE: f64 = 1e-11;

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
    // A grid of cells at every power of two: each triangle goes in the cells of the level
    // just larger than its box (padded by half `LENGTH_TOL` a side, so padded boxes meet
    // exactly when `test` compares them), at most eight. A triangle then looks in its own
    // level and every coarser one, so a pair is met where the larger of the two lives,
    // whatever the spread of sizes.
    let pad = DVec3::splat(LENGTH_TOL / 2.0);
    let level = |t: &Tri| ((t.hi - t.lo).max_element() + LENGTH_TOL).log2().ceil() as i32;
    let cell = |p: DVec3, k: i32| (p / 2f64.powi(k)).floor().as_i64vec3().to_array();
    let mut grid: HashMap<(i32, [i64; 3]), Vec<usize>> = HashMap::new();
    let mut levels: BTreeSet<i32> = BTreeSet::new();
    let cells_of = |t: &Tri, k: i32| {
        let (c0, c1) = (cell(t.lo - pad, k), cell(t.hi + pad, k));
        (c0[0]..=c1[0]).flat_map(move |x| {
            (c0[1]..=c1[1]).flat_map(move |y| (c0[2]..=c1[2]).map(move |z| [x, y, z]))
        })
    };
    let at: Vec<i32> = tris.iter().map(level).collect();
    for (i, t) in tris.iter().enumerate() {
        levels.insert(at[i]);
        for c in cells_of(t, at[i]) {
            grid.entry((at[i], c)).or_default().push(i);
        }
    }
    for (a, ta) in tris.iter().enumerate() {
        for &k in levels.range(at[a]..) {
            for c in cells_of(ta, k) {
                let Some(list) = grid.get(&(k, c)) else {
                    continue;
                };
                for &b in list {
                    // A pair of one level is taken once, from the lower index; a pair is met
                    // in every cell its boxes share, and tested in the one holding the low
                    // corner of their overlap.
                    if (k == at[a] && b <= a) || cell(ta.lo.max(tris[b].lo) - pad, k) != c {
                        continue;
                    }
                    test(ta, &tris[b]);
                }
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
