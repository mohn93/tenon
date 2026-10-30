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
//!
//! A corner of rounds and chamfers uses the star-shaped patch of rounds of different radii, a
//! chamfer standing in as the round with the same setback: its strip is that round's chord, so
//! the patch is held below the strip's plane. Concave corners of different radii or kinds get
//! the same patches for the empty corner: it is a convex corner of the empty space, so the
//! faces' normals are turned round and the cutter is added instead of subtracted.
//!
//! A corner of inside and outside edges is a saddle between the ball outside the material and
//! the ball in the empty corner, which no sphere or ring of rounds bounds. Two rounds of one
//! kind and one of the other, on three square faces, get a `Saddle`: the pair stop at the
//! planes through the unlike round's axis, and the body swept about that axis between their end
//! sections is a cutter (outside pair) or a filler (inside pair); its surface is the
//! rolling-ball blend round the unlike round.

use std::collections::HashSet;

use glam::{DMat3, DQuat};

use super::*;

/// The largest crease (radians) left soft across a face beside a patch of unequal rounds.
const SLIGHT: f64 = 0.17;

/// How near a vertex of a finished solid must be to a patch vertex to count as it, given the
/// least distance between the patch's vertices: a fraction of it, so a neighbour never does.
fn mark_tol(spacing: f64) -> f64 {
    0.25 * spacing
}

/// The least distance between two of `points`.
fn spacing_of(points: &[DVec3]) -> f64 {
    let mut least = f64::INFINITY;
    for (i, p) in points.iter().enumerate() {
        for q in &points[i + 1..] {
            least = least.min(p.distance(*q));
        }
    }
    least
}

/// Caps of the cutter lie this fraction of the radius outside the faces.
const CAP: f64 = 0.25;

#[cfg(test)]
thread_local! {
    /// Tests: no corner patches, to see the crossed cuts.
    pub(super) static OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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
    /// For a patch of rounds of different radii, the positions of its vertices.
    pub marks: Vec<DVec3>,
    /// And the least distance between two of them.
    pub spacing: f64,
    /// And the sides of its boundary that lie on the rounds' end arcs (the others lie across
    /// the faces).
    pub arc_sides: Vec<[DVec3; 2]>,
    /// And the sides that end a chamfer's strip: they keep their crease, however slight.
    pub chord_sides: Vec<[DVec3; 2]>,
}

/// The patches for the fillets `cuts` (already closed at their ends) of `m`.
pub(super) fn patches(m: &Model, cuts: &[(EdgeId, EdgeCut)]) -> Vec<Patch> {
    #[cfg(test)]
    if OFF.with(|o| o.get()) {
        return Vec::new();
    }
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
            let rounds = three
                .iter()
                .filter(|(_, c)| matches!(c.profile, Profile::Fillet { .. }))
                .count();
            match rounds {
                3 => patch(m, v, three, concave),
                // Chamfers meet in a point of their own (concave: a cone of empty space).
                0 => flat(m, v, three, concave),
                // Rounds and chamfers: the ringed patch, a chamfer standing in as the round
                // with its setback.
                _ => patch_mixed(m, v, three, concave),
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
fn patch(m: &Model, v: VertexId, cuts: [&(EdgeId, EdgeCut); 3], concave: bool) -> Option<Patch> {
    let Profile::Fillet { radius, .. } = cuts[0].1.profile else {
        return None;
    };
    // One ball touches the three faces only when the three rounds have one radius; otherwise
    // the patch is star-shaped (convex corners only).
    if cuts
        .iter()
        .any(|(_, c)| (c.profile.size() - radius).abs() > 1e-9 * radius)
    {
        return patch_mixed(m, v, cuts, concave);
    }
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
        let mut points: Vec<DVec3> = cut
            .profile
            .profile(cut)
            .into_iter()
            .map(|o| foot + o)
            .collect();
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
        marks: Vec::new(),
        spacing: f64::INFINITY,
        arc_sides: Vec::new(),
        chord_sides: Vec::new(),
    })
}

/// The patch at `v` for three convex rounds of different radii: no ball touches the three faces
/// and all three arcs, so the patch is star-shaped about the mean of the arcs' centres, bounded by
/// the three end arcs (each at its own depth along its edge) and a straight segment across each
/// face from one arc's end to the next arc's start. None when the corner is not a plain
/// trihedral one or the pieces do not close.
fn patch_mixed(
    m: &Model,
    v: VertexId,
    cuts: [&(EdgeId, EdgeCut); 3],
    concave: bool,
) -> Option<Patch> {
    let faces: Vec<FaceId> = m.faces_at(v).collect();
    if faces.len() != 3 || m.edges_at(v).len() != 3 {
        return None;
    }
    let vertex = m.pos(v);
    let mut normal: [Option<DVec3>; 3] = [None; 3];
    for (_, cut) in cuts {
        for (f, n) in [(cut.f1, cut.n1), (cut.f2, cut.n2)] {
            normal[faces.iter().position(|g| *g == f)?] = Some(n);
        }
    }
    // Concave: the empty corner is the convex one, with the faces' normals turned round, and
    // the patch is a filler for it.
    let flip = if concave { -1.0 } else { 1.0 };
    let n = [normal[0]?, normal[1]?, normal[2]?].map(|x| x * flip);
    // Edge k lies between the faces k + 1 and k + 2.
    let mut edge: Vec<&(EdgeId, EdgeCut)> = Vec::new();
    for k in 0..3 {
        let (k1, k2) = ((k + 1) % 3, (k + 2) % 3);
        edge.push(cuts.iter().find(|(_, c)| {
            let pair = [c.f1, c.f2];
            pair.contains(&faces[k1]) && pair.contains(&faces[k2])
        })?);
    }
    let away: Vec<DVec3> = edge
        .iter()
        .map(|(_, c)| if c.va == v { c.u } else { -c.u })
        .collect();
    // Where the tangent lines of the two rounds beside face j cross.
    let mut meet = [DVec3::ZERO; 3];
    for j in 0..3 {
        let (a, b) = ((j + 1) % 3, (j + 2) % 3);
        let into = |c: &EdgeCut| if c.f1 == faces[j] { c.d1 } else { c.d2 };
        let (ca, cb) = (into(&edge[b].1).dot(away[a]), into(&edge[a].1).dot(away[b]));
        if ca.abs() < 1e-3 || cb.abs() < 1e-3 {
            return None;
        }
        let (s, t) = (edge[b].1.setback / ca, edge[a].1.setback / cb);
        if s <= 0.0 || t <= 0.0 {
            return None;
        }
        meet[j] = vertex + away[a] * s + away[b] * t;
    }
    // A chamfer stands in as the round with the same setback: its strip is that round's chord.
    let radii: Vec<f64> = edge.iter().map(|(_, c)| c.profile.round_size(c)).collect();
    // Each arc ends at the furthest of the two crossings along its edge.
    let mut arcs: Vec<Vec<DVec3>> = Vec::new();
    let mut centres = [DVec3::ZERO; 3];
    let mut depth = [0.0; 3];
    for k in 0..3 {
        let (k1, k2) = ((k + 1) % 3, (k + 2) % 3);
        let cut = &edge[k].1;
        depth[k] = ((meet[k1] - vertex).dot(away[k])).max((meet[k2] - vertex).dot(away[k]));
        if depth[k] <= 10.0 * LENGTH_TOL {
            return None;
        }
        let foot = vertex + away[k] * depth[k];
        let mut points: Vec<DVec3> = cut
            .profile
            .profile(cut)
            .into_iter()
            .map(|o| foot + o)
            .collect();
        if cut.f1 != faces[k1] {
            points.reverse();
        }
        centres[k] = points[0] - n[k1] * radii[k];
        arcs.push(points);
    }
    let rmin = radii.iter().copied().fold(f64::INFINITY, f64::min);
    let rmax = radii.iter().copied().fold(0.0, f64::max);
    // The loop: arcs 2, 0, 1 (they run face 0 to 1, 1 to 2, 2 to 0), a segment across the face
    // between one arc's end and the next one's start. Each point remembers its arc.
    let same = |p: DVec3, q: DVec3| p.distance(q) < 1e-9 * rmax.max(1.0);
    // Each entry: the point, its arc (for the skirt's direction) and the arc its side to the
    // next point lies on (None for a segment across a face).
    let mut ring: Vec<(DVec3, usize, Option<usize>)> = Vec::new();
    for k in [2, 0, 1] {
        for (i, p) in arcs[k].iter().enumerate() {
            if i == 0
                && let Some(last) = ring.last_mut()
                && same(last.0, *p)
            {
                last.2 = Some(k);
                continue;
            }
            ring.push((*p, k, (i + 1 < arcs[k].len()).then_some(k)));
        }
    }
    if same(ring[0].0, ring[ring.len() - 1].0) {
        ring.pop();
    }
    let count = ring.len();
    if count < 4 {
        return None;
    }
    let c = (centres[0] + centres[1] + centres[2]) / 3.0;
    let middle = (n[0] + n[1] + n[2]).try_normalize()?;
    let dist: Vec<f64> = ring.iter().map(|(p, _, _)| p.distance(c)).collect();
    let to: Vec<DVec3> = ring
        .iter()
        .map(|(p, _, _)| (*p - c).try_normalize())
        .collect::<Option<_>>()?;
    let rho = (radii[0] + radii[1] + radii[2]) / 3.0;
    // Ring sides along a chamfer's strip.
    let chord: Vec<bool> = ring
        .iter()
        .map(|(_, _, side)| {
            side.is_some_and(|k| matches!(edge[k].1.profile, Profile::Chamfer { .. }))
        })
        .collect();
    // Inside the faceted rounds, and below the faces.
    let inside = |p: DVec3| -> bool {
        let margin = 1e-3 * rmin;
        if (0..3).any(|j| n[j].dot(p - vertex) > -1e-6 * rmin) {
            return false;
        }
        (0..3).all(|k| {
            let q = p - centres[k];
            let w = q - away[k] * q.dot(away[k]);
            arcs[k].windows(2).all(|pair| {
                let (a, b) = (pair[0] - centres[k], pair[1] - centres[k]);
                let turn = a.cross(b);
                let in_wedge = a.cross(w).dot(turn) >= 0.0 && w.cross(b).dot(turn) >= 0.0;
                let Some(out) = (a + b).try_normalize() else {
                    return true;
                };
                !in_wedge || w.dot(out) < a.dot(out) - margin
            })
        })
    };
    // The apex, lowered while it pokes out of a chamfer's plane or a round.
    let mut depth_mid = 1.0;
    // The first steps toward the ring must be inside too, or no layer has room to start.
    let lowered = |top: f64| {
        inside(c + middle * top)
            && (0..count)
                .all(|i| inside(c + slerp(middle, to[i], 0.15) * (top + (dist[i] - top) * 0.15)))
    };
    while depth_mid > 0.5 && !lowered(rho * depth_mid) {
        depth_mid -= 0.02;
    }
    let top = rho * depth_mid;
    // A point of the star: fraction t of the way from the apex to ring point i.
    let spot = |dir: DVec3, d: f64, t: f64| c + slerp(middle, dir, t) * (top + (d - top) * t);
    let step = arcs
        .iter()
        .map(|a| (a[0] - c).angle_between(a[a.len() - 1] - c) / (a.len() - 1) as f64)
        .fold(f64::INFINITY, f64::min);
    let reach = to
        .iter()
        .map(|d| middle.angle_between(*d))
        .fold(0.0, f64::max);
    let mut layers_at: Vec<Vec<DVec3>> = Vec::new();
    let mut found = false;
    for rings in (2..=((reach / step).ceil() as usize).max(2)).rev() {
        let nominal = (rings - 1) as f64 / rings as f64;
        let mut first: Vec<f64> = (0..count)
            .map(|i| {
                let mut t = nominal;
                while t > 0.05 && !inside(spot(to[i], dist[i], t)) {
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
        let build = |first: &[f64]| -> Vec<Vec<DVec3>> {
            (1..rings)
                .map(|j| {
                    let along = j as f64 / (rings - 1) as f64;
                    let points = ((count as f64 * j as f64 / rings as f64).round() as usize)
                        .clamp(6.min(count), count);
                    (0..points)
                        .map(|k| {
                            let s = k as f64 * count as f64 / points as f64;
                            let (i0, f) = (s.floor() as usize % count, s.fract());
                            let i1 = (i0 + 1) % count;
                            let (dir, d) = if chord[i0] {
                                // Along a chamfer's straight strip: on the chord itself.
                                let w = ring[i0].0.lerp(ring[i1].0, f) - c;
                                (w.try_normalize().unwrap_or(to[i0]), w.length())
                            } else {
                                (
                                    slerp(to[i0], to[i1], f),
                                    dist[i0] + (dist[i1] - dist[i0]) * f,
                                )
                            };
                            let t = (first[i0] + (first[i1] - first[i0]) * f) * along;
                            spot(dir, d, t)
                        })
                        .collect()
                })
                .collect()
        };
        // Points between ring samples can fall outside the faceted rounds: shrink and rebuild.
        for _ in 0..8 {
            if first.iter().any(|t| *t <= 0.05) {
                break;
            }
            let layers = build(&first);
            if layers.iter().flatten().all(|p| inside(*p)) {
                layers_at = layers;
                found = true;
                break;
            }
            for t in first.iter_mut() {
                *t *= 0.85;
            }
        }
        if found {
            break;
        }
    }
    if !found {
        return None;
    }
    let apex = c + middle * top;
    if !inside(apex) {
        return None;
    }

    // The cutter: the patch, a skirt standing out from each arc in its own plane and from each
    // segment square to its face, and a cone over the outer ends of the skirts.
    let mut cutter = Model::new();
    let mut marks: Vec<DVec3> = Vec::new();
    let vid = |cutter: &mut Model, marks: &mut Vec<DVec3>, p: DVec3| {
        marks.push(p);
        cutter.insert_vertex(p)
    };
    let q = vid(&mut cutter, &mut marks, apex);
    let mut layers: Vec<Vec<VertexId>> = Vec::new();
    for l in &layers_at {
        let ids = l.iter().map(|p| vid(&mut cutter, &mut marks, *p)).collect();
        layers.push(ids);
    }
    let boundary: Vec<VertexId> = ring
        .iter()
        .map(|(p, _, _)| vid(&mut cutter, &mut marks, *p))
        .collect();
    layers.push(boundary.clone());
    let mut sides: HashSet<(VertexId, VertexId)> = HashSet::new();
    let mut add = |cutter: &mut Model, mut l: Vec<VertexId>, normal: DVec3| {
        let pts: Vec<DVec3> = l.iter().map(|w| cutter.pos(*w)).collect();
        if vector_area(&pts).dot(normal) < 0.0 {
            l.reverse();
        }
        for i in 0..l.len() {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            if sides.insert((a.min(b), a.max(b))) {
                cutter.insert_edge(a, b);
            }
        }
        cutter.insert_face(Face::new(l, Vec::new(), normal))
    };
    let mut painted: Vec<FaceId> = Vec::new();
    let tri = |cutter: &mut Model,
               add: &mut dyn FnMut(&mut Model, Vec<VertexId>, DVec3) -> FaceId,
               t: [VertexId; 3],
               flip: bool|
     -> Option<FaceId> {
        let ps = t.map(|w| cutter.pos(w));
        let normal = (ps[1] - ps[0]).cross(ps[2] - ps[0]).try_normalize()?;
        Some(add(cutter, t.to_vec(), if flip { -normal } else { normal }))
    };
    let mut add_dyn = |cutter: &mut Model, l: Vec<VertexId>, normal: DVec3| add(cutter, l, normal);
    // The patch's triangles, all wound the same way round the ring.
    let mut patch_tris: Vec<[VertexId; 3]> = Vec::new();
    let first = &layers[0];
    for i in 0..first.len() {
        patch_tris.push([q, first[i], first[(i + 1) % first.len()]]);
    }
    for w in layers.windows(2) {
        let (inner, outer) = (&w[0], &w[1]);
        let d = |cutter: &Model, a: VertexId, b: VertexId| cutter.pos(a).distance(cutter.pos(b));
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
                d(&cutter, next_inner, b) <= d(&cutter, a, next_outer)
            };
            if advance_inner {
                patch_tris.push([a, b, next_inner]);
                i += 1;
            } else {
                patch_tris.push([a, b, next_outer]);
                j += 1;
            }
        }
    }
    // One orientation for the whole patch: its winding is consistent, so it either faces
    // away from the centre as wound (and is turned over) or already toward it; the cone is
    // wound the same way round and takes the opposite sign.
    let away_from_c: f64 = patch_tris
        .iter()
        .map(|t| {
            let ps = t.map(|w| cutter.pos(w));
            (ps[1] - ps[0])
                .cross(ps[2] - ps[0])
                .dot((ps[0] + ps[1] + ps[2]) / 3.0 - c)
        })
        .sum();
    let turn = away_from_c > 0.0;
    for t in &patch_tris {
        painted.push(tri(&mut cutter, &mut add_dyn, *t, turn)?);
    }
    // Skirts and cone.
    let high = 2.0 * rmax;
    let mut outer_ids: Vec<VertexId> = Vec::new();
    for (p, k, _) in ring.iter() {
        // Radially from the arc's centre, which at an arc's ends is along the face's normal.
        let radial = (*p - centres[*k]).try_normalize()?;
        outer_ids.push(cutter.insert_vertex(*p + radial * high));
    }
    for i in 0..count {
        let j = (i + 1) % count;
        let (ob, oa) = (outer_ids[j], outer_ids[i]);
        let quad = vec![boundary[i], boundary[j], ob, oa];
        if let Some(k) = ring[i].2 {
            // On an arc: the skirt lies in the arc's plane, the cutter on the vertex side.
            add_dyn(&mut cutter, quad, away[k]);
        } else {
            // Across a face: square to it, the cutter on the vertex side.
            let (a, b) = (ring[i].0, ring[j].0);
            let along = (b - a).try_normalize()?;
            let face = (0..3).find(|f| {
                n[*f].dot(a - vertex).abs() < 1e-7 * rmax
                    && n[*f].dot(b - vertex).abs() < 1e-7 * rmax
            })?;
            let mut wall = along.cross(n[face]).try_normalize()?;
            if wall.dot((a + b) / 2.0 - vertex) < 0.0 {
                wall = -wall;
            }
            add_dyn(&mut cutter, quad, wall);
        }
    }
    let top = c + middle * (3.0 * high + rmax);
    let top_id = cutter.insert_vertex(top);
    for i in 0..count {
        painted.push(tri(
            &mut cutter,
            &mut add_dyn,
            [top_id, outer_ids[i], outer_ids[(i + 1) % count]],
            !turn,
        )?);
    }
    cutter.take_changes();
    if open_reason(&cutter).is_some() || signed_volume(&cutter) <= 0.0 {
        return None;
    }
    let _ = painted;
    // The skirts and the cone reach far; cut them off just outside the faces, so nothing
    // beside the corner is touched.
    let planes = [
        (away[0], vertex + away[0] * (depth[0] + rmax)),
        (away[1], vertex + away[1] * (depth[1] + rmax)),
        (away[2], vertex + away[2] * (depth[2] + rmax)),
        (n[0], vertex + n[0] * CAP * rmax),
        (n[1], vertex + n[1] * CAP * rmax),
        (n[2], vertex + n[2] * CAP * rmax),
    ];
    let clip = hexahedron(&planes)?;
    let mut cutter = combine(&cutter, &clip, BooleanOp::Intersect).ok()??;
    let [outer_paint, inner_paint] = cuts[0].1.paint;
    let ids: Vec<FaceId> = cutter.faces().map(|(id, _)| id).collect();
    for f in ids {
        let mut face = cutter.face(f).expect("face").clone();
        face.front = outer_paint;
        face.back = inner_paint;
        cutter.replace_face(f, face);
    }
    cutter.take_changes();
    if open_reason(&cutter).is_some() || signed_volume(&cutter) <= 0.0 {
        return None;
    }
    Some(Patch {
        vertex: v,
        centre: c,
        radius: rho,
        cutter,
        points: marks.len(),
        edges: [
            (edge[0].0, depth[0]),
            (edge[1].0, depth[1]),
            (edge[2].0, depth[2]),
        ],
        flat: None,
        concave,
        spacing: spacing_of(&marks),
        marks,
        // A chamfer's strip keeps its crease, so its end is not softened.
        arc_sides: (0..count)
            .filter(|i| {
                ring[*i]
                    .2
                    .is_some_and(|k| matches!(edge[k].1.profile, Profile::Fillet { .. }))
            })
            .map(|i| [ring[i].0, ring[(i + 1) % count].0])
            .collect(),
        chord_sides: (0..count)
            .filter(|i| chord[*i])
            .map(|i| [ring[i].0, ring[(i + 1) % count].0])
            .collect(),
    })
}

/// The hexahedron inside `planes` (`[arc 0, arc 1, arc 2, cap 0, cap 1, cap 2]`, each a normal
/// pointing out and a point on it; planes `k` and `k + 3` face each other along different
/// directions, `k` with `k` of the other triple), or None when they do not bound one.
fn hexahedron(planes: &[(DVec3, DVec3); 6]) -> Option<Model> {
    let corner = |bits: [bool; 3]| -> Option<DVec3> {
        let ps = [0, 1, 2].map(|k| planes[k + if bits[k] { 3 } else { 0 }]);
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
    let mut m = Model::new();
    let mut at: HashMap<[bool; 3], VertexId> = HashMap::new();
    for b in 0..8 {
        let bits = [b & 1 != 0, b & 2 != 0, b & 4 != 0];
        at.insert(bits, m.insert_vertex(corner(bits)?));
    }
    let mut sides: HashSet<(VertexId, VertexId)> = HashSet::new();
    for k in 0..3 {
        let (k1, k2) = ((k + 1) % 3, (k + 2) % 3);
        for cap in [false, true] {
            let l: Vec<VertexId> = [(false, false), (true, false), (true, true), (false, true)]
                .iter()
                .map(|(b1, b2)| {
                    let mut bits = [false; 3];
                    bits[k] = cap;
                    bits[k1] = *b1;
                    bits[k2] = *b2;
                    at[&bits]
                })
                .collect();
            let normal = planes[k + if cap { 3 } else { 0 }].0;
            let mut l = l;
            let pts: Vec<DVec3> = l.iter().map(|w| m.pos(*w)).collect();
            if vector_area(&pts).dot(normal) < 0.0 {
                l.reverse();
            }
            for i in 0..4 {
                let (a, b) = (l[i], l[(i + 1) % 4]);
                if sides.insert((a.min(b), a.max(b))) {
                    m.insert_edge(a, b);
                }
            }
            m.insert_face(Face::new(l, Vec::new(), normal));
        }
    }
    m.take_changes();
    (open_reason(&m).is_none() && signed_volume(&m) > 0.0).then_some(m)
}

/// The flat closure of three chamfers of `distance` at `v`, or None when the corner is not a
/// plain trihedral one.
fn flat(m: &Model, v: VertexId, cuts: [&(EdgeId, EdgeCut); 3], concave: bool) -> Option<Patch> {
    let distance = cuts.iter().map(|(_, c)| c.setback).fold(0.0, f64::max);
    // A distance more than the other two together leaves the corner crossed.
    let total: f64 = cuts.iter().map(|(_, c)| c.setback).sum();
    if 2.0 * distance > total {
        return None;
    }
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
        // Concave: the empty corner is the convex one, its normals turned round.
        normal[k] = if a.f1 == *f { a.n1 } else { a.n2 } * if concave { -1.0 } else { 1.0 };
        let (ea, eb) = (away(a), away(b));
        let (da, db) = (into(a, *f), into(b, *f));
        let (ca, cb) = (db.dot(ea), da.dot(eb));
        if ca.abs() < 1e-3 || cb.abs() < 1e-3 {
            return None;
        }
        // Along edge a the strip of b reaches b's distance, and along b that of a.
        let (s, t) = (b.setback / ca, a.setback / cb);
        if s <= 0.0 || t <= 0.0 {
            return None;
        }
        meet[k] = vertex + ea * s + eb * t;
    }
    let mut across = (meet[1] - meet[0])
        .cross(meet[2] - meet[0])
        .try_normalize()?;
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
        concave,
        marks: Vec::new(),
        spacing: f64::INFINITY,
        arc_sides: Vec::new(),
        chord_sides: Vec::new(),
    })
}

///The cutter and the number of vertices of its sphere patch. `n` are the faces' outward
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

fn apply_all(m: Model, patches: &[Patch], some: &[usize], op: BooleanOp) -> (Model, Vec<usize>) {
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
    if !p.marks.is_empty() {
        // A mark standing in the air needs no cut: the patch lies above the material there
        // (a vertex near a chamfer's plane), so nothing of it shows.
        let strict = (5.0 * LENGTH_TOL).min(1e-3 * p.radius);
        return p.marks.iter().all(|c| {
            let near = m
                .vertices()
                .map(|(_, v)| v.pos.distance(*c))
                .fold(f64::INFINITY, f64::min);
            near < strict
                || (near < mark_tol(p.spacing)
                    && crate::boolean::point_inside(*c, m) == Some(false))
        });
    }
    if let Some(corners) = p.flat {
        return corners.iter().all(|c| {
            m.vertices()
                .any(|(_, v)| v.pos.distance(*c) < (5.0 * LENGTH_TOL).min(1e-3 * p.radius))
        });
    }
    on_sphere(m, p).count() >= p.points
}

fn on_sphere<'a>(m: &'a Model, p: &'a Patch) -> impl Iterator<Item = VertexId> + 'a {
    let tol = if p.marks.is_empty() {
        (5.0 * LENGTH_TOL).min(1e-3 * p.radius)
    } else {
        mark_tol(p.spacing)
    };
    m.vertices()
        .filter(move |(_, v)| {
            if p.marks.is_empty() {
                (v.pos.distance(p.centre) - p.radius).abs() < tol
            } else {
                p.marks.iter().any(|c| c.distance(v.pos) < tol)
            }
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
                // A patch of unequal rounds softens along its arcs whatever the angle, and
                // across a face only where the crease is slight.
                let (pa, pb) = (m.pos(a), m.pos(b));
                let tol = (1e-7 * p.radius.max(1e-3)).max(if p.marks.is_empty() {
                    0.0
                } else {
                    mark_tol(p.spacing)
                });
                let on = |sides: &[[DVec3; 2]]| {
                    sides.iter().any(|[u, w]| {
                        (u.distance(pa) < tol && w.distance(pb) < tol)
                            || (u.distance(pb) < tol && w.distance(pa) < tol)
                    })
                };
                if on(&p.chord_sides) {
                    continue;
                }
                let on_arc = on(&p.arc_sides);
                let slight = if p.marks.is_empty() {
                    nf.angle_between(no) <= 2.0 * step + 1e-6
                } else {
                    nf.angle_between(no) <= SLIGHT
                };
                if (is_patch(other) || on_arc || slight)
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

/// A corner where two rounds of one kind (both convex or both concave) and a round of the
/// other kind meet: the surface there is a saddle, which no sphere or ring of rounds bounds.
pub(super) struct Saddle {
    pub vertex: VertexId,
    /// How each edge at the corner ends: the like pair stop at the planes through the unlike
    /// round's axis, the unlike round runs through to the third face.
    pub ends: [(EdgeId, End); 3],
    /// The swept body between the like pair's end sections: a cutter when the pair is convex
    /// (the rounds of two outside edges turning round an inside one), a filler when it is
    /// concave (insides turning round an outside).
    pub sweep: Model,
    pub add: bool,
    /// Vertices of the swept surface between its ends, to check that it took.
    pub marks: Vec<DVec3>,
    /// And those at its ends too, to soften its edges.
    pub seams: Vec<DVec3>,
    /// The least distance between neighbouring ones.
    pub spacing: f64,
}

/// The saddle at `v`, or None when the corner is not one: three rounds (two of one kind, one
/// of the other) at a vertex of three mutually square faces, the like pair with as many
/// segments each. The like pair's sections, swept about the unlike round's axis from one's end
/// plane to the other's, give the body; its surface is the rolling-ball blend about the unlike
/// round, a torus piece when the like radii are equal and a loft between them when not.
pub(super) fn saddle(m: &Model, v: VertexId, cuts: [&(EdgeId, EdgeCut); 3]) -> Option<Saddle> {
    let faces: Vec<FaceId> = m.faces_at(v).collect();
    if faces.len() != 3 || m.edges_at(v).len() != 3 {
        return None;
    }
    if cuts
        .iter()
        .any(|(_, c)| !matches!(c.profile, Profile::Fillet { .. }))
    {
        return None;
    }
    let convex = cuts.iter().filter(|(_, c)| c.convex).count();
    if convex == 0 || convex == 3 {
        return None;
    }
    // The odd one out, and the pair of the other kind.
    let third = (0..3).find(|i| cuts[*i].1.convex == (convex == 1))?;
    let like: Vec<usize> = (0..3).filter(|i| *i != third).collect();
    let (t, add) = (&cuts[third].1, !cuts[like[0]].1.convex);
    let normal = |c: &EdgeCut, f: FaceId| if c.f1 == f { c.n1 } else { c.n2 };
    // The third round's faces x and y; z is the face both of the pair lie on.
    let (x, y) = (t.f1, t.f2);
    let z = *faces.iter().find(|f| **f != x && **f != y)?;
    let (nx, ny, nz) = (normal(t, x), normal(t, y), normal(&cuts[like[0]].1, z));
    if nx.dot(ny).abs() > 1e-6 || nx.dot(nz).abs() > 1e-6 || ny.dot(nz).abs() > 1e-6 {
        return None;
    }
    // The pair: `a` beside x, `b` beside y.
    let beside = |f: FaceId| {
        like.iter()
            .copied()
            .find(|i| cuts[*i].1.f1 == f || cuts[*i].1.f2 == f)
    };
    let (a, b) = (beside(x)?, beside(y)?);
    if a == b {
        return None;
    }
    let (ca, cb) = (&cuts[a].1, &cuts[b].1);
    let away = |c: &EdgeCut| if c.va == v { c.u } else { -c.u };
    let (ua, ub, uc) = (away(ca), away(cb), away(t));
    let s = t.setback;
    // The axis: parallel to the third edge, `s` from both of its faces (in the empty corner of
    // an inside edge, in the material of an outside one).
    let sign = if t.convex { -1.0 } else { 1.0 };
    let vertex = m.pos(v);
    let axis_at = vertex + sign * s * (nx + ny);
    let depth = |u: DVec3| (axis_at - vertex).dot(u);
    let (da, db) = (depth(ua), depth(ub));
    if da <= 10.0 * LENGTH_TOL || db <= 10.0 * LENGTH_TOL {
        return None;
    }
    // The pair must be long enough to hold the saddle's depth and the round at their far
    // ends, and the third round too.
    let length = |c: &EdgeCut| c.a.distance(c.b);
    if length(ca) < da + 2.0 * ca.setback
        || length(cb) < db + 2.0 * cb.setback
        || length(t) < 3.0 * s
    {
        return None;
    }
    // Each pair member's section starting on its face beside the third round, with the sides
    // widened past the faces by `reach` (less beside the third round, which the axis is near).
    let section = |c: &EdgeCut, own: FaceId, foot: DVec3| -> (Vec<DVec3>, usize) {
        let mut points = c.profile.profile(c);
        if c.f1 != own {
            points.reverse();
        }
        let sides = points.len() - 1;
        let (n_own, n_z) = (normal(c, own), normal(c, z));
        let widen = if c.convex { 1.0 } else { -1.0 };
        let reach = c.setback;
        let (r_own, r_z) = ((0.5 * s).min(reach), reach);
        let (first, last) = (points[0], points[sides]);
        points.extend([
            last + n_z * widen * r_z,
            n_own * widen * r_own + n_z * widen * r_z,
            first + n_own * widen * r_own,
        ]);
        (points.into_iter().map(|o| foot + o).collect(), sides)
    };
    let (pa, sides) = section(ca, x, vertex + ua * da);
    let (pb, sides_b) = section(cb, y, vertex + ub * db);
    if sides != sides_b {
        return None;
    }
    let start = vertex + ua * da - axis_at;
    let finish = vertex + ub * db - axis_at;
    let axis = uc;
    let angle = f64::atan2(axis.dot(start.cross(finish)), start.dot(finish));
    if (angle.abs() - std::f64::consts::FRAC_PI_2).abs() > 1e-6 {
        return None;
    }
    let turn = |p: DVec3, by: f64| axis_at + DQuat::from_axis_angle(axis, by) * (p - axis_at);
    let steps = sides.max(2);
    let n = pa.len();
    let rings: Vec<Vec<DVec3>> = (0..=steps)
        .map(|k| {
            let t = k as f64 / steps as f64;
            (0..n)
                .map(|i| {
                    let back = turn(pb[i], -angle);
                    turn(pa[i].lerp(back, t), angle * t)
                })
                .collect()
        })
        .collect();
    let mut sweep = Model::new();
    let ids: Vec<Vec<VertexId>> = rings
        .iter()
        .map(|r| r.iter().map(|p| sweep.insert_vertex(*p)).collect())
        .collect();
    let mut loops: Vec<Vec<VertexId>> = Vec::new();
    let mut surface: Vec<bool> = Vec::new();
    for k in 0..steps {
        for i in 0..n {
            let j = (i + 1) % n;
            loops.push(vec![ids[k][i], ids[k][j], ids[k + 1][j]]);
            loops.push(vec![ids[k][i], ids[k + 1][j], ids[k + 1][i]]);
            surface.extend([i < sides; 2]);
        }
    }
    loops.push(ids[0].iter().rev().copied().collect());
    loops.push(ids[steps].clone());
    surface.extend([false; 2]);
    let mut seen: HashSet<(VertexId, VertexId)> = HashSet::new();
    for l in &loops {
        for i in 0..l.len() {
            let (p, q) = (l[i], l[(i + 1) % l.len()]);
            if seen.insert((p.min(q), p.max(q))) {
                sweep.insert_edge(p, q);
            }
        }
    }
    let mut made: Vec<FaceId> = Vec::new();
    for l in loops {
        let pts: Vec<DVec3> = l.iter().map(|w| sweep.pos(*w)).collect();
        let normal = vector_area(&pts).try_normalize()?;
        made.push(sweep.insert_face(Face::new(l, Vec::new(), normal)));
    }
    if signed_volume(&sweep) < 0.0 {
        sweep.reverse_faces(&made);
    }
    let [outer, inner] = ca.paint;
    for (f, on) in made.iter().zip(&surface) {
        if *on {
            let mut face = sweep.face(*f).expect("face").clone();
            face.front = outer;
            face.back = inner;
            sweep.replace_face(*f, face);
        }
    }
    sweep.take_changes();
    if open_reason(&sweep).is_some() || signed_volume(&sweep) <= 0.0 {
        return None;
    }
    let marks: Vec<DVec3> = (1..steps)
        .flat_map(|k| (0..=sides).map(move |i| (k, i)))
        .map(|(k, i)| rings[k][i])
        .collect();
    let mut spacing = f64::INFINITY;
    for k in 0..=steps {
        for i in 0..=sides {
            if i < sides {
                spacing = spacing.min(rings[k][i].distance(rings[k][i + 1]));
            }
            if k < steps {
                spacing = spacing.min(rings[k][i].distance(rings[k + 1][i]));
            }
        }
    }
    let seams: Vec<DVec3> = rings
        .iter()
        .flat_map(|r| r[..=sides].iter().copied())
        .collect();
    let plane_of = |u: DVec3, d: f64| End::Plane(vertex + u * d, u);
    Some(Saddle {
        vertex: v,
        ends: [
            (cuts[a].0, plane_of(ua, da)),
            (cuts[b].0, plane_of(ub, db)),
            // The third round runs to the plane of the face the pair lie on.
            (cuts[third].0, End::Plane(vertex, nz)),
        ],
        sweep,
        add,
        marks,
        seams,
        spacing,
    })
}

/// Softens the edges of `m` between vertices of the swept surfaces `saddles` (the sweeps'
/// facets, their seams with the straight rounds, the third round and the flat faces).
pub(super) fn soften_saddles(m: &mut Model, saddles: &[&Saddle]) {
    let mut soft = Vec::new();
    for s in saddles {
        let tol = mark_tol(s.spacing);
        let on = |p: DVec3| s.seams.iter().any(|c| c.distance(p) < tol);
        for (id, e) in m.edges() {
            if !e.soft && on(m.pos(e.v[0])) && on(m.pos(e.v[1])) {
                soft.push((id, e.curve, e.polygon));
            }
        }
    }
    for (id, curve, polygon) in soft {
        m.set_edge(id, curve, true, polygon);
    }
    m.take_changes();
}

impl Saddle {
    /// Whether `m` is a closed solid with the swept surface in it.
    pub fn took(&self, m: &Model) -> bool {
        open_reason(m).is_none()
            && signed_volume(m) > 0.0
            && self.marks.iter().all(|c| {
                m.vertices()
                    .any(|(_, v)| v.pos.distance(*c) < mark_tol(self.spacing))
            })
    }
}

/// The saddles at the vertices where exactly three of `cuts` end.
pub(super) fn saddles(m: &Model, cuts: &[(EdgeId, EdgeCut)]) -> Vec<Saddle> {
    #[cfg(test)]
    if OFF.with(|o| o.get()) {
        return Vec::new();
    }
    let mut at: BTreeMap<VertexId, Vec<usize>> = BTreeMap::new();
    for (i, (_, cut)) in cuts.iter().enumerate() {
        for v in [cut.va, cut.vb] {
            at.entry(v).or_default().push(i);
        }
    }
    at.into_iter()
        .filter(|(_, list)| list.len() == 3)
        .filter_map(|(v, list)| saddle(m, v, [&cuts[list[0]], &cuts[list[1]], &cuts[list[2]]]))
        .collect()
}
