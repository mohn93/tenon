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
}

/// The patches for the fillets `cuts` (already closed at their ends) of `m`.
pub(super) fn patches(m: &Model, cuts: &[(EdgeId, EdgeCut)], profile: Profile) -> Vec<Patch> {
    let Profile::Fillet { .. } = profile else {
        return Vec::new();
    };
    let mut at: BTreeMap<VertexId, Vec<usize>> = BTreeMap::new();
    for (i, (_, cut)) in cuts.iter().enumerate() {
        if !cut.convex {
            continue;
        }
        for (k, v) in [cut.va, cut.vb].into_iter().enumerate() {
            if matches!(cut.ends[k], End::Extend(_)) {
                at.entry(v).or_default().push(i);
            }
        }
    }
    let mut found: Vec<Patch> = at
        .into_iter()
        .filter(|(_, list)| list.len() == 3)
        .filter_map(|(v, list)| {
            let three = [list[0], list[1], list[2]].map(|i| &cuts[i]);
            patch(m, v, three, profile)
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
fn patch(m: &Model, v: VertexId, cuts: [&(EdgeId, EdgeCut); 3], profile: Profile) -> Option<Patch> {
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
    let n = [normal[0]?, normal[1]?, normal[2]?];
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
    // The sphere patch, facing the centre.
    let mut tris: Vec<[VertexId; 3]> = Vec::new();
    let points;
    if steps[0] == steps[1] && steps[1] == steps[2] {
        let g = steps[0];
        let mut grid: HashMap<(usize, usize), VertexId> = HashMap::new();
        // Corner 0 at i = g, corner 1 at j = g, corner 2 at k = g; the arcs run 0-1 (edge 2),
        // 1-2 (edge 0), 2-0 (edge 1).
        let omega = |a: usize, b: usize| n[a].dot(n[b]).clamp(-1.0, 1.0).acos();
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
                    m.insert_vertex(centre + dir.normalize() * radius)
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
    } else {
        // Unequal arcs: concentric rings from the middle point out to the boundary, each ring
        // slerped from the middle to every boundary point; neighbouring rings are joined by
        // quads cut in two, the innermost ring by a fan.
        let mut ring: Vec<VertexId> = Vec::new();
        for k in [2, 0, 1] {
            ring.extend(&arc_ids[k][..arc_ids[k].len() - 1]);
        }
        let count = ring.len();
        let middle = (n[0] + n[1] + n[2]).try_normalize()?;
        // Rings about one and a quarter arc steps apart: a ring closer to the boundary would
        // poke through the faceted fillets there.
        let step = arcs
            .iter()
            .map(|a| (a[0] - centre).angle_between(a[a.len() - 1] - centre) / (a.len() - 1) as f64)
            .fold(f64::INFINITY, f64::min);
        let reach = ring
            .iter()
            .map(|id| middle.angle_between(m.pos(*id) - centre))
            .fold(0.0, f64::max);
        let rings = ((reach / (1.25 * step)).ceil() as usize).max(1);
        let q = m.insert_vertex(centre + middle * radius);
        let mut layers: Vec<Vec<VertexId>> = Vec::new();
        for j in 1..rings {
            let t = j as f64 / rings as f64;
            let mut layer = Vec::with_capacity(count);
            for id in &ring {
                let to = (m.pos(*id) - centre) / radius;
                layer.push(m.insert_vertex(centre + slerp(middle, to, t) * radius));
            }
            layers.push(layer);
        }
        layers.push(ring);
        points = 1 + layers.iter().map(Vec::len).sum::<usize>();
        for i in 0..count {
            let j = (i + 1) % count;
            tris.push([q, layers[0][i], layers[0][j]]);
            for w in layers.windows(2) {
                // Cut the quad along its shorter diagonal.
                let d = |a: VertexId, b: VertexId| m.pos(a).distance(m.pos(b));
                if d(w[0][i], w[1][j]) <= d(w[0][j], w[1][i]) {
                    tris.push([w[0][i], w[1][i], w[1][j]]);
                    tris.push([w[0][i], w[1][j], w[0][j]]);
                } else {
                    tris.push([w[0][i], w[1][i], w[0][j]]);
                    tris.push([w[1][i], w[1][j], w[0][j]]);
                }
            }
        }
    }
    let mut painted = Vec::new();
    for t in tris {
        let ps = t.map(|v| m.pos(v));
        let normal = (ps[1] - ps[0]).cross(ps[2] - ps[0]).try_normalize()?;
        let toward = centre - (ps[0] + ps[1] + ps[2]) / 3.0;
        let normal = if normal.dot(toward) < 0.0 {
            -normal
        } else {
            normal
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

/// Subtracts the patches' cutters from `m`: all at once where they allow, else one at a time,
/// leaving a corner crossed when its cutter fails. Gives the result and the patches that took.
pub(super) fn apply(m: Model, patches: &[Patch]) -> (Model, Vec<usize>) {
    if patches.is_empty() {
        return (m, Vec::new());
    }
    let all: Vec<usize> = (0..patches.len()).collect();
    let tools: Vec<Model> = patches.iter().map(|p| p.cutter.clone()).collect();
    let mut acc = m.clone();
    let mut ok = true;
    for tool in apart(tools) {
        match combine(&acc, &tool, BooleanOp::Subtract) {
            Ok(Some(next)) => acc = next,
            _ => {
                ok = false;
                break;
            }
        }
    }
    if ok && all.iter().all(|i| took(&acc, &patches[*i])) {
        return (acc, all);
    }
    let mut acc = m;
    let mut done = Vec::new();
    for (i, p) in patches.iter().enumerate() {
        if let Ok(Some(next)) = combine(&acc, &p.cutter, BooleanOp::Subtract)
            && took(&next, p)
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
