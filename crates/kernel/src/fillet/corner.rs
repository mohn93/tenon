//! Spherical corner patches for fillets.
//!
//! Where three convex fillets of one radius meet at a vertex with three faces, the crossed
//! rounds the edge cutters leave are trimmed to a patch of the sphere that touches the three
//! faces (its centre is the point `radius` inside all three). The sphere lies inside the three
//! fillet cylinders and meets each one along the arc in the plane through the centre
//! perpendicular to its edge, so the patch's boundary is the fillets' own end arcs. One cutter
//! per corner does it: the hexahedron between the three arc planes and three caps just outside
//! the faces, less the patch, subtracted from the filleted solid. A corner whose cutter fails
//! keeps the crossed rounds.

use std::collections::HashSet;

use glam::DMat3;

use super::*;

/// Caps of the cutter lie this fraction of the radius outside the faces.
const CAP: f64 = 0.25;

/// One corner's cutter and where its sphere is.
pub(super) struct Patch {
    pub vertex: VertexId,
    pub centre: DVec3,
    pub radius: f64,
    /// The cutter: closed and outward.
    pub cutter: Model,
    /// Vertices of the sphere patch, boundary included.
    pub points: usize,
    /// Each edge at the corner with the distance from the vertex to its arc plane.
    pub edges: [(EdgeId, f64); 3],
    /// For a chamfer corner, the corners of the flat triangle it makes (`centre` and `radius`
    /// are then unused).
    pub flat: Option<[DVec3; 3]>,
    /// Three concave fillets meet here: the cutter is a filler, added instead of subtracted.
    pub concave: bool,
}

/// The patches for the fillets `cuts` (already closed at their ends) of `m`.
pub(super) fn patches(m: &Model, cuts: &[(EdgeId, EdgeCut)], profile: Profile) -> Vec<Patch> {
    // Convex edges running on past the vertex; concave edges (which stop in the third face).
    let mut at: BTreeMap<(VertexId, bool), Vec<usize>> = BTreeMap::new();
    for (i, (_, cut)) in cuts.iter().enumerate() {
        for (k, v) in [cut.va, cut.vb].into_iter().enumerate() {
            if !cut.convex || matches!(cut.ends[k], End::Extend(_)) {
                at.entry((v, !cut.convex)).or_default().push(i);
            }
        }
    }
    let mut found: Vec<Patch> = at
        .into_iter()
        .filter(|(_, list)| list.len() == 3)
        .filter_map(|((v, concave), list)| {
            let three = [list[0], list[1], list[2]].map(|i| &cuts[i]);
            match profile {
                Profile::Fillet { .. } => patch(m, v, three, profile, concave),
                // Three concave chamfers meet in a point of their own: a cone of empty space.
                Profile::Chamfer { distance } if !concave => flat(m, v, three, distance),
                Profile::Chamfer { .. } => None,
            }
        })
        .collect();
    // Two patches on one edge must leave a straight run between them.
    let mut length: HashMap<EdgeId, Vec<(VertexId, f64)>> = HashMap::new();
    for p in &found {
        for (e, d) in p.edges {
            length.entry(e).or_default().push((p.vertex, d));
        }
    }
    let too_close: BTreeSet<VertexId> = cuts
        .iter()
        .filter_map(|(e, cut)| {
            let on = length.get(e)?;
            let room = cut.a.distance(cut.b) - 10.0 * LENGTH_TOL;
            let used: f64 = on.iter().map(|(_, d)| d).sum();
            (used > room).then(|| on.iter().map(|(v, _)| *v))
        })
        .flatten()
        .collect();
    found.retain(|p| !too_close.contains(&p.vertex));
    found
}

/// The patch at `v` for three fillet cuts, or None when the corner is not a plain trihedral one.
fn patch(
    m: &Model,
    v: VertexId,
    cuts: [&(EdgeId, EdgeCut); 3],
    profile: Profile,
    concave: bool,
) -> Option<Patch> {
    let Profile::Fillet { radius, .. } = profile else {
        return None;
    };
    let faces: Vec<FaceId> = m.faces_at(v).collect();
    if faces.len() != 3 || m.edges_at(v).len() != 3 {
        return None;
    }
    // The outward normal of each face, from the cuts beside it.
    let mut normal: [Option<DVec3>; 3] = [None; 3];
    for (_, cut) in cuts {
        for (f, n) in [(cut.f1, cut.n1), (cut.f2, cut.n2)] {
            normal[faces.iter().position(|g| *g == f)?] = Some(n);
        }
    }
    // Concave: the ball sits in the empty corner, on the other side of each face, so the faces'
    // normals are turned to point from the centre to the tangent points.
    let flip = if concave { -1.0 } else { 1.0 };
    let n = [normal[0]?, normal[1]?, normal[2]?].map(|x| x * flip);
    let vertex = m.pos(v);
    let rows = DMat3::from_cols(n[0], n[1], n[2]).transpose();
    if rows.determinant().abs() < 1e-6 {
        return None;
    }
    let centre = vertex + rows.inverse() * DVec3::splat(-radius);
    // Edge k lies between the faces k + 1 and k + 2: its arc runs from the first to the second.
    let mut arcs: Vec<Vec<DVec3>> = Vec::new();
    let mut away = [DVec3::ZERO; 3];
    let mut ids = [EdgeId(0); 3];
    let mut depth = [0.0; 3];
    for k in 0..3 {
        let (k1, k2) = ((k + 1) % 3, (k + 2) % 3);
        let (id, cut) = cuts
            .iter()
            .find(|(_, c)| {
                let pair = [c.f1, c.f2];
                pair.contains(&faces[k1]) && pair.contains(&faces[k2])
            })
            .map(|(e, c)| (*e, c))?;
        let along = if cut.va == v { cut.u } else { -cut.u };
        let foot = cut.a + cut.u * (centre - cut.a).dot(cut.u);
        let mut points: Vec<DVec3> = profile.profile(cut).into_iter().map(|o| foot + o).collect();
        if cut.f1 != faces[k1] {
            points.reverse();
        }
        let near = |p: DVec3, q: DVec3| p.distance(q) < 1e-7 * radius.max(1.0);
        if !near(points[0], centre + n[k1] * radius)
            || !near(points[points.len() - 1], centre + n[k2] * radius)
        {
            return None;
        }
        ids[k] = id;
        away[k] = along;
        depth[k] = (centre - vertex).dot(along);
        if depth[k] <= 10.0 * LENGTH_TOL {
            return None;
        }
        arcs.push(points);
    }
    let paint = cuts[0].1.paint;
    let (cutter, points) = cutter(centre, radius, n, away, &arcs, paint)?;
    Some(Patch {
        vertex: v,
        centre,
        radius,
        cutter,
        points,
        edges: [(ids[0], depth[0]), (ids[1], depth[1]), (ids[2], depth[2])],
        flat: None,
        concave,
    })
}

/// The flat closure of three chamfers of `distance` at `v`, or None when the corner is not a
/// plain trihedral one.
fn flat(m: &Model, v: VertexId, cuts: [&(EdgeId, EdgeCut); 3], distance: f64) -> Option<Patch> {
    let faces: Vec<FaceId> = m.faces_at(v).collect();
    if faces.len() != 3 || m.edges_at(v).len() != 3 {
        return None;
    }
    let vertex = m.pos(v);
    // Per edge: its direction from the vertex and, for each face, its direction into it.
    let into = |cut: &EdgeCut, f: FaceId| if cut.f1 == f { cut.d1 } else { cut.d2 };
    let away = |cut: &EdgeCut| if cut.va == v { cut.u } else { -cut.u };
    let mut normal = [DVec3::ZERO; 3];
    let mut meet = [DVec3::ZERO; 3];
    for (k, f) in faces.iter().enumerate() {
        let beside: Vec<&EdgeCut> = cuts
            .iter()
            .map(|(_, c)| c)
            .filter(|c| c.f1 == *f || c.f2 == *f)
            .collect();
        let [a, b] = beside[..] else {
            return None;
        };
        normal[k] = if a.f1 == *f { a.n1 } else { a.n2 };
        let (ea, eb) = (away(a), away(b));
        let (da, db) = (into(a, *f), into(b, *f));
        let (ca, cb) = (db.dot(ea), da.dot(eb));
        if ca.abs() < 1e-3 || cb.abs() < 1e-3 {
            return None;
        }
        let (s, t) = (distance / ca, distance / cb);
        if s <= 0.0 || t <= 0.0 {
            return None;
        }
        meet[k] = vertex + ea * s + eb * t;
    }
    let mut across = (meet[1] - meet[0]).cross(meet[2] - meet[0]).try_normalize()?;
    if across.dot(vertex - meet[0]) > 0.0 {
        across = -across;
    }
    let reach = CAP * distance;
    let planes: [(DVec3, DVec3); 4] = [
        (across, meet[0]),
        (normal[0], vertex + normal[0] * reach),
        (normal[1], vertex + normal[1] * reach),
        (normal[2], vertex + normal[2] * reach),
    ];
    let at = |k: [usize; 3]| -> Option<DVec3> {
        let rows = DMat3::from_cols(planes[k[0]].0, planes[k[1]].0, planes[k[2]].0).transpose();
        if rows.determinant().abs() < 1e-6 {
            return None;
        }
        Some(
            rows.inverse()
                * DVec3::new(
                    planes[k[0]].0.dot(planes[k[0]].1),
                    planes[k[1]].0.dot(planes[k[1]].1),
                    planes[k[2]].0.dot(planes[k[2]].1),
                ),
        )
    };
    // The tetrahedron below the plane and inside the caps: the tip is in it.
    let top = at([1, 2, 3])?;
    let base = [at([0, 2, 3])?, at([0, 1, 3])?, at([0, 1, 2])?];
    if (top - meet[0]).dot(across) >= 0.0 {
        return None;
    }
    let mut cutter = Model::new();
    let ids: Vec<VertexId> = std::iter::once(top)
        .chain(base)
        .map(|p| cutter.insert_vertex(p))
        .collect();
    let centroid = (top + base[0] + base[1] + base[2]) / 4.0;
    for (i, j) in (0..4).flat_map(|i| (i + 1..4).map(move |j| (i, j))) {
        cutter.insert_edge(ids[i], ids[j]);
    }
    let [outer, inner] = cuts[0].1.paint;
    for skip in 0..4 {
        let mut l: Vec<VertexId> = (0..4).filter(|i| *i != skip).map(|i| ids[i]).collect();
        let pts: Vec<DVec3> = l.iter().map(|w| cutter.pos(*w)).collect();
        let mut normal = vector_area(&pts).try_normalize()?;
        if normal.dot((pts[0] + pts[1] + pts[2]) / 3.0 - centroid) < 0.0 {
            normal = -normal;
            l.reverse();
        }
        let mut face = Face::new(l, Vec::new(), normal);
        face.front = outer;
        face.back = inner;
        cutter.insert_face(face);
    }
    cutter.take_changes();
    if open_reason(&cutter).is_some() || signed_volume(&cutter) <= 0.0 {
        return None;
    }
    let reach_on = |k: usize| -> f64 {
        let along = away(&cuts[k].1);
        meet.iter()
            .map(|p| (*p - vertex).dot(along))
            .fold(0.0, f64::max)
    };
    Some(Patch {
        vertex: v,
        centre: vertex,
        radius: distance,
        cutter,
        points: 0,
        edges: [
            (cuts[0].0, reach_on(0)),
            (cuts[1].0, reach_on(1)),
            (cuts[2].0, reach_on(2)),
        ],
        flat: Some(meet),
        concave: false,
    })
}

/// The cutter and the number of vertices of its sphere patch. `n` are the faces' outward
/// normals, `away[k]` the direction of edge `k` from the vertex, `arcs[k]` its arc from face
/// `k + 1` to face `k + 2`.
fn cutter(
    centre: DVec3,
    radius: f64,
    n: [DVec3; 3],
    away: [DVec3; 3],
    arcs: &[Vec<DVec3>],
    paint: [Option<MaterialId>; 2],
) -> Option<(Model, usize)> {
    // Planes: P_k through the centre with normal away[k]; cap k outside face k.
    let plane = |k: usize, cap: bool| -> (DVec3, DVec3) {
        if cap {
            (n[k], centre + n[k] * radius * (1.0 + CAP))
        } else {
            (away[k], centre)
        }
    };
    let corner = |bits: [bool; 3]| -> Option<DVec3> {
        let ps = [0, 1, 2].map(|k| plane(k, bits[k]));
        let rows = DMat3::from_cols(ps[0].0, ps[1].0, ps[2].0).transpose();
        if rows.determinant().abs() < 1e-6 {
            return None;
        }
        Some(
            rows.inverse()
                * DVec3::new(
                    ps[0].0.dot(ps[0].1),
                    ps[1].0.dot(ps[1].1),
                    ps[2].0.dot(ps[2].1),
                ),
        )
    };
    let mut hex: HashMap<[bool; 3], DVec3> = HashMap::new();
    for b in 0..8 {
        let bits = [b & 1 != 0, b & 2 != 0, b & 4 != 0];
        let p = corner(bits)?;
        // Inside every plane's half space.
        for k in 0..3 {
            for cap in [false, true] {
                let (nk, pk) = plane(k, cap);
                if (p - pk).dot(nk) > 1e-9 * radius.max(1.0) {
                    return None;
                }
            }
        }
        hex.insert(bits, p);
    }
    let mut m = Model::new();
    let mut sides: HashSet<(VertexId, VertexId)> = HashSet::new();
    let mut vertex_of: HashMap<[bool; 3], VertexId> = HashMap::new();
    for (bits, p) in &hex {
        if *bits != [false; 3] {
            vertex_of.insert(*bits, m.insert_vertex(*p));
        }
    }
    // Arc vertices: the end of arc k is the start of arc k + 1.
    let steps: Vec<usize> = arcs.iter().map(|a| a.len() - 1).collect();
    let mut arc_ids: Vec<Vec<VertexId>> = Vec::new();
    let mut tangent: [Option<VertexId>; 3] = [None; 3];
    for (k, arc) in arcs.iter().enumerate() {
        let (k1, k2) = ((k + 1) % 3, (k + 2) % 3);
        let mut ids = Vec::new();
        for (i, p) in arc.iter().enumerate() {
            let end = if i == 0 {
                Some(k1)
            } else if i == arc.len() - 1 {
                Some(k2)
            } else {
                None
            };
            let id = match end {
                Some(f) => *tangent[f].get_or_insert_with(|| m.insert_vertex(*p)),
                None => m.insert_vertex(*p),
            };
            ids.push(id);
        }
        arc_ids.push(ids);
    }
    let mut add = |m: &mut Model, mut l: Vec<VertexId>, normal: DVec3| -> FaceId {
        let pts: Vec<DVec3> = l.iter().map(|v| m.pos(*v)).collect();
        if vector_area(&pts).dot(normal) < 0.0 {
            l.reverse();
        }
        for i in 0..l.len() {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            if sides.insert((a.min(b), a.max(b))) {
                m.insert_edge(a, b);
            }
        }
        m.insert_face(Face::new(l, Vec::new(), normal))
    };
    // The six hexahedron faces; the three on the arc planes lose the centre to the arc.
    for k in 0..3 {
        let (k1, k2) = ((k + 1) % 3, (k + 2) % 3);
        for cap in [false, true] {
            let at = |b1: bool, b2: bool| {
                let mut bits = [false; 3];
                bits[k] = cap;
                bits[k1] = b1;
                bits[k2] = b2;
                bits
            };
            let mut l: Vec<VertexId> = Vec::new();
            if !cap {
                // Cycle: centre, (k1 cap), (both caps), (k2 cap); the arc runs k1 to k2.
                l.extend(arc_ids[k].iter().rev().copied());
                l.push(vertex_of[&at(true, false)]);
                l.push(vertex_of[&at(true, true)]);
                l.push(vertex_of[&at(false, true)]);
                add(&mut m, l, away[k]);
            } else {
                for (b1, b2) in [(false, false), (true, false), (true, true), (false, true)] {
                    l.push(vertex_of[&at(b1, b2)]);
                }
                add(&mut m, l, n[k]);
            }
        }
    }
    // The sphere patch, facing the centre. Every inner vertex must lie inside the faceted
    // fillets it borders (a vertex outside would cut through their facets), so inner points are
    // checked and pulled toward the middle until they are.
    let inside = |p: DVec3| -> bool {
        let margin = 1e-3 * radius;
        (0..3).all(|k| {
            let w = (p - centre) - away[k] * (p - centre).dot(away[k]);
            arcs[k].windows(2).all(|pair| {
                let (a, b) = (pair[0] - centre, pair[1] - centre);
                let turn = a.cross(b);
                let in_wedge = a.cross(w).dot(turn) >= 0.0 && w.cross(b).dot(turn) >= 0.0;
                let Some(out) = (a + b).try_normalize() else {
                    return true;
                };
                !in_wedge || w.dot(out) < a.dot(out) - margin
            })
        })
    };
    let mut tris: Vec<[VertexId; 3]> = Vec::new();
    let mut inward = false;
    let mut points = 0;
    let mut grid: HashMap<(usize, usize), VertexId> = HashMap::new();
    if steps[0] == steps[1] && steps[1] == steps[2] {
        let g = steps[0];
        let mut inner: HashMap<(usize, usize), DVec3> = HashMap::new();
        // Corner 0 at i = g, corner 1 at j = g, corner 2 at k = g; the arcs run 0-1 (edge 2),
        // 1-2 (edge 0), 2-0 (edge 1).
        let omega = |a: usize, b: usize| n[a].dot(n[b]).clamp(-1.0, 1.0).acos();
        for i in 1..g {
            for j in 1..(g - i) {
                let k = g - i - j;
                let lam = [
                    i as f64 / g as f64,
                    j as f64 / g as f64,
                    k as f64 / g as f64,
                ];
                let mut dir = DVec3::ZERO;
                for a in 0..3 {
                    let (b, c) = ((a + 1) % 3, (a + 2) % 3);
                    let span = lam[b] + lam[c];
                    let w = if span < 1e-12 {
                        omega(a, b)
                    } else {
                        (lam[b] * omega(a, b) + lam[c] * omega(a, c)) / span
                    };
                    dir += n[a] * (lam[a] * w).sin();
                }
                inner.insert((i, j), centre + dir.normalize() * radius);
            }
        }
        if inner.values().all(|p| inside(*p)) {
            for i in 0..=g {
                for j in 0..=(g - i) {
                    let k = g - i - j;
                    let id = if k == 0 {
                        arc_ids[2][j]
                    } else if i == 0 {
                        arc_ids[0][k]
                    } else if j == 0 {
                        arc_ids[1][i]
                    } else {
                        m.insert_vertex(inner[&(i, j)])
                    };
                    grid.insert((i, j), id);
                }
            }
            points = grid.len();
            for i in 0..g {
                for j in 0..(g - i) {
                    tris.push([grid[&(i, j)], grid[&(i + 1, j)], grid[&(i, j + 1)]]);
                    if i + j + 2 <= g {
                        tris.push([grid[&(i + 1, j)], grid[&(i + 1, j + 1)], grid[&(i, j + 1)]]);
                    }
                }
            }
        }
    }
    if grid.is_empty() {
        // Concentric rings from the middle point out to the boundary, each ring slerped from
        // the middle to every boundary point; neighbouring rings are joined by quads cut on
        // their shorter diagonal, the innermost ring by a fan.
        let mut ring: Vec<VertexId> = Vec::new();
        for k in [2, 0, 1] {
            ring.extend(&arc_ids[k][..arc_ids[k].len() - 1]);
        }
        let count = ring.len();
        let middle = (n[0] + n[1] + n[2]).try_normalize()?;
        let to: Vec<DVec3> = ring
            .iter()
            .map(|id| (m.pos(*id) - centre) / radius)
            .collect();
        let step = arcs
            .iter()
            .map(|a| (a[0] - centre).angle_between(a[a.len() - 1] - centre) / (a.len() - 1) as f64)
            .fold(f64::INFINITY, f64::min);
        let reach = to
            .iter()
            .map(|d| middle.angle_between(*d))
            .fold(0.0, f64::max);
        let at = |i: usize, t: f64| centre + slerp(middle, to[i], t) * radius;
        // The finest ring count whose inner points all lie inside the fillets: along each spoke
        // the first ring sits as far out as it may (at most R - 1 of R), the rest evenly spaced.
        let mut layers_at: Vec<Vec<DVec3>> = Vec::new();
        for rings in (1..=((reach / step).ceil() as usize).max(1)).rev() {
            if rings == 1 {
                layers_at.clear();
                break;
            }
            let nominal = (rings - 1) as f64 / rings as f64;
            let mut first: Vec<f64> = (0..count)
                .map(|i| {
                    let mut t = nominal;
                    while t > 0.05 && !inside(at(i, t)) {
                        t -= 0.5 / rings as f64 / 4.0;
                    }
                    t
                })
                .collect();
            for _ in 0..3 {
                let before = first.clone();
                for i in 0..count {
                    let near = before[(i + 1) % count].min(before[(i + count - 1) % count]);
                    first[i] = first[i].min(near + 0.15);
                }
            }
            // Inner rings have fewer points (about in proportion to their distance from the
            // middle, at least 6), each on a spoke between two boundary points.
            let layers: Vec<Vec<DVec3>> = (1..rings)
                .map(|j| {
                    let along = j as f64 / (rings - 1) as f64;
                    let points = ((count as f64 * j as f64 / rings as f64).round() as usize)
                        .clamp(6.min(count), count);
                    (0..points)
                        .map(|k| {
                            let spoke = k as f64 * count as f64 / points as f64;
                            let (i0, f) = (spoke.floor() as usize % count, spoke.fract());
                            let i1 = (i0 + 1) % count;
                            let dir = slerp(to[i0], to[i1], f);
                            let t = (first[i0] + (first[i1] - first[i0]) * f) * along;
                            centre + slerp(middle, dir, t) * radius
                        })
                        .collect()
                })
                .collect();
            if first.iter().all(|t| *t > 0.05) && layers.iter().flatten().all(|p| inside(*p)) {
                layers_at = layers;
                break;
            }
        }
        // The middle point, sunk into the ball where a coarse fillet's facets would cut it.
        let mut depth = 1.0;
        while depth > 0.5 && !inside(centre + middle * radius * depth) {
            depth -= 0.02;
        }
        let q = m.insert_vertex(centre + middle * radius * depth);
        let mut layers: Vec<Vec<VertexId>> = layers_at
            .iter()
            .map(|l| l.iter().map(|p| m.insert_vertex(*p)).collect())
            .collect();
        layers.push(ring);
        points = usize::from(depth == 1.0) + layers.iter().map(Vec::len).sum::<usize>();
        let first = &layers[0];
        for i in 0..first.len() {
            tris.push([q, first[i], first[(i + 1) % first.len()]]);
        }
        for w in layers.windows(2) {
            // Walk both rings round together, each time advancing the side that makes the
            // shorter new edge.
            let (inner, outer) = (&w[0], &w[1]);
            let d = |a: VertexId, b: VertexId| m.pos(a).distance(m.pos(b));
            let (mut i, mut j) = (0, 0);
            while i < inner.len() || j < outer.len() {
                let (a, b) = (inner[i % inner.len()], outer[j % outer.len()]);
                let next_inner = inner[(i + 1) % inner.len()];
                let next_outer = outer[(j + 1) % outer.len()];
                let advance_inner = if i >= inner.len() {
                    false
                } else if j >= outer.len() {
                    true
                } else {
                    d(next_inner, b) <= d(a, next_outer)
                };
                if advance_inner {
                    tris.push([a, b, next_inner]);
                    i += 1;
                } else {
                    tris.push([a, b, next_outer]);
                    j += 1;
                }
            }
        }
        // The triangles run counter-clockwise round the ring: turned over if the ring seen from
        // outside is counter-clockwise, they face the centre.
        let turn: f64 = (0..count)
            .map(|i| {
                let (a, b) = (to[i], to[(i + 1) % count]);
                a.cross(b).dot(n[0] + n[1] + n[2])
            })
            .sum();
        for t in &mut tris {
            if turn > 0.0 {
                t.swap(1, 2);
            }
        }
        inward = true;
    }
    let mut painted = Vec::new();
    for t in tris {
        let ps = t.map(|v| m.pos(v));
        let normal = (ps[1] - ps[0]).cross(ps[2] - ps[0]).try_normalize()?;
        // Ring triangles already face the centre; a grid triangle is turned toward it.
        let toward = centre - (ps[0] + ps[1] + ps[2]) / 3.0;
        let normal = if inward || normal.dot(toward) >= 0.0 {
            normal
        } else {
            -normal
        };
        painted.push(add(&mut m, t.to_vec(), normal));
    }
    let [outer, inner] = paint;
    for f in painted {
        let mut face = m.face(f).expect("face").clone();
        face.front = outer;
        face.back = inner;
        m.replace_face(f, face);
    }
    m.take_changes();
    if open_reason(&m).is_some() || signed_volume(&m) <= 0.0 {
        return None;
    }
    Some((m, points))
}

/// The unit vector a fraction `t` of the way from `a` to `b` along the great circle.
fn slerp(a: DVec3, b: DVec3, t: f64) -> DVec3 {
    let omega = a.dot(b).clamp(-1.0, 1.0).acos();
    if omega < 1e-9 {
        return a;
    }
    ((a * ((1.0 - t) * omega).sin() + b * (t * omega).sin()) / omega.sin()).normalize()
}

/// Subtracts the convex patches' cutters from `m` and adds the concave ones' fillers: all at
/// once where they allow, else one at a time, leaving a corner crossed when its cutter fails.
/// Gives the result and the patches that took.
pub(super) fn apply(m: Model, patches: &[Patch]) -> (Model, Vec<usize>) {
    let mut acc = m;
    let mut done = Vec::new();
    for (op, concave) in [(BooleanOp::Union, true), (BooleanOp::Subtract, false)] {
        let some: Vec<usize> = (0..patches.len())
            .filter(|i| patches[*i].concave == concave)
            .collect();
        if some.is_empty() {
            continue;
        }
        let (next, took) = apply_all(acc, patches, &some, op);
        acc = next;
        done.extend(took);
    }
    done.sort();
    (acc, done)
}

fn apply_all(
    m: Model,
    patches: &[Patch],
    some: &[usize],
    op: BooleanOp,
) -> (Model, Vec<usize>) {
    let tools: Vec<Model> = some.iter().map(|i| patches[*i].cutter.clone()).collect();
    let mut acc = m.clone();
    let mut ok = true;
    for tool in apart(tools) {
        match combine(&acc, &tool, op) {
            Ok(Some(next)) => acc = next,
            _ => {
                ok = false;
                break;
            }
        }
    }
    if ok && some.iter().all(|i| took(&acc, &patches[*i])) {
        return (acc, some.to_vec());
    }
    let mut acc = m;
    let mut done = Vec::new();
    for &i in some {
        let r = combine(&acc, &patches[i].cutter, op);
        if let Ok(Some(next)) = r
            && took(&next, &patches[i])
        {
            acc = next;
            done.push(i);
        }
    }
    (acc, done)
}

/// Whether `m` has the sphere patch of `p`.
fn took(m: &Model, p: &Patch) -> bool {
    if open_reason(m).is_some() || signed_volume(m) <= 0.0 {
        return false;
    }
    if let Some(corners) = p.flat {
        return corners.iter().all(|c| {
            m.vertices()
                .any(|(_, v)| v.pos.distance(*c) < 5.0 * LENGTH_TOL.min(1e-3 * p.radius))
        });
    }
    on_sphere(m, p).count() >= p.points
}

fn on_sphere<'a>(m: &'a Model, p: &'a Patch) -> impl Iterator<Item = VertexId> + 'a {
    m.vertices()
        .filter(|(_, v)| {
            (v.pos.distance(p.centre) - p.radius).abs() < (5.0 * LENGTH_TOL).min(1e-3 * p.radius)
        })
        .map(|(id, _)| id)
}

/// Softens the edges inside the patches `done` and between each patch and the fillet facets
/// beside it (not the flat faces it touches at a point).
pub(super) fn soften(m: &mut Model, patches: &[Patch], done: &[usize], step: f64) {
    let mut soft = Vec::new();
    for &i in done {
        let p = &patches[i];
        let on: BTreeSet<VertexId> = on_sphere(m, p).collect();
        let is_patch = |f: FaceId| {
            let face = m.face(f).expect("face");
            face.holes.is_empty() && face.outer.iter().all(|v| on.contains(v))
        };
        let faces: BTreeSet<FaceId> = on
            .iter()
            .flat_map(|v| m.faces_at(*v))
            .filter(|f| is_patch(*f))
            .collect();
        for f in faces {
            let face = m.face(f).expect("face");
            for j in 0..face.outer.len() {
                let (a, b) = (face.outer[j], face.outer[(j + 1) % face.outer.len()]);
                let [g, h] = m.faces_on_side(a, b)[..] else {
                    continue;
                };
                let other = if g == f { h } else { g };
                let (nf, no) = (face.normal, m.face(other).expect("face").normal);
                if (is_patch(other) || nf.angle_between(no) <= 2.0 * step + 1e-6)
                    && let Some(e) = m.edge_between(a, b)
                {
                    let edge = m.edge(e).expect("edge");
                    soft.push((e, edge.curve, edge.polygon));
                }
            }
        }
    }
    for (e, curve, polygon) in soft {
        m.set_edge(e, curve, true, polygon);
    }
    m.take_changes();
}
