//! Model health: why a mesh is not a solid, entity by entity, and closing what can be closed
//! safely: welding tiny gaps, removing stray edges and internal faces, orienting faces and
//! filling holes, on a copy that is only handed back when it is a solid.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use glam::{DVec2, DVec3};

use crate::boolean::{interior_point, point_inside, subset};
use crate::geom::{LENGTH_TOL, PLANE_ANGLE_TOL, Plane, point_in_polygon, vector_area};
use crate::ids::{EdgeId, FaceId, VertexId};
use crate::model::{Face, Model};
use crate::shellcross::crossing_faces;
use crate::solid::{open_reason, signed_volume};

/// The least volume a solid encloses, as `instance_volume` asks.
pub const MIN_VOLUME: f64 = 1e-12;

/// The largest weld tolerance, in meters, however big the part.
pub const MAX_WELD: f64 = 1e-3;

/// The weld tolerance as a share of the part's bounding-box diagonal.
pub const WELD_SHARE: f64 = 1e-5;

/// What keeps a mesh from being a solid. Every list is in id order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Health {
    /// Why the mesh is not a solid (the first problem `open_reason` finds, or that it encloses
    /// no volume); None for a solid.
    pub reason: Option<String>,
    /// Edges bounding exactly one face.
    pub open_edges: Vec<EdgeId>,
    /// Edges bounding more than two faces.
    pub shared_edges: Vec<EdgeId>,
    /// Edges bounding no face.
    pub stray_edges: Vec<EdgeId>,
    /// Faces turned inside out: facing in, in a closed shell; in an open shell whose faces
    /// disagree, those on the side with less area.
    pub reversed_faces: Vec<FaceId>,
    /// Pairs of vertices on open edges nearer than the weld tolerance, the lower id first.
    pub gaps: Vec<(VertexId, VertexId)>,
    /// Faces every one of whose edges bounds three or more faces.
    pub internal_faces: Vec<FaceId>,
    /// Faces of a closed shell that cross another face of it, other than along shared edges.
    pub self_crossing: Vec<FaceId>,
    /// The mesh is closed but encloses no volume.
    pub no_volume: bool,
    /// The mesh has no faces.
    pub no_faces: bool,
}

impl Health {
    pub fn is_solid(&self) -> bool {
        self.reason.is_none()
    }
}

/// What `repair` did, or would have done.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repair {
    /// Vertices merged into a neighbour across a tiny gap.
    pub welded: usize,
    pub strays_removed: usize,
    pub internal_removed: usize,
    /// Faces made to fill holes.
    pub holes_filled: usize,
    /// Faces whose side ended up turned over.
    pub faces_flipped: usize,
    /// Why some of the mesh could not be closed (a hole that is not flat, ...).
    pub reasons: Vec<String>,
}

/// The result of `repair`.
#[derive(Clone, Debug)]
pub struct Repaired {
    /// The closed copy with its edit still pending (take its changes); None when the mesh
    /// was a solid already or could not be made one.
    pub model: Option<Model>,
    pub repair: Repair,
    /// The problems left after the attempt (none when `model` is Some).
    pub left: Health,
}

/// The weld tolerance of `m`: 1e-5 of its bounding-box diagonal, at least `LENGTH_TOL` and
/// at most `MAX_WELD`.
pub fn weld_tolerance(m: &Model) -> f64 {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    for (_, v) in m.vertices() {
        lo = lo.min(v.pos);
        hi = hi.max(v.pos);
    }
    let diagonal = if lo.x.is_finite() {
        lo.distance(hi)
    } else {
        0.0
    };
    (WELD_SHARE * diagonal).clamp(LENGTH_TOL, MAX_WELD)
}

/// Each edge's ends (lower id first) with how many face loop sides run it from the lower end
/// and from the higher.
fn side_counts(m: &Model) -> BTreeMap<(VertexId, VertexId), [u32; 2]> {
    let mut counts: BTreeMap<(VertexId, VertexId), [u32; 2]> = BTreeMap::new();
    for (_, f) in m.faces() {
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                counts.entry((a.min(b), a.max(b))).or_default()[usize::from(a > b)] += 1;
            }
        }
    }
    counts
}

fn users(counts: &BTreeMap<(VertexId, VertexId), [u32; 2]>, a: VertexId, b: VertexId) -> u32 {
    counts.get(&(a.min(b), a.max(b))).map_or(0, |[x, y]| x + y)
}

/// The sides of every loop of `f`.
fn sides(f: &Face) -> impl Iterator<Item = (VertexId, VertexId)> + '_ {
    std::iter::once(&f.outer)
        .chain(&f.holes)
        .flat_map(|l| (0..l.len()).map(move |i| (l[i], l[(i + 1) % l.len()])))
}

/// Six times the signed volume `f`'s triangles add.
fn face_volume(m: &Model, f: FaceId) -> f64 {
    m.triangulate(f)
        .unwrap_or_default()
        .iter()
        .map(|[a, b, c]| a.dot(b.cross(*c)))
        .sum()
}

/// The faces to turn over so that every shell agrees and faces the right way: a closed shell
/// (every side of its faces used by exactly two faces) outward, or inward when it lies inside
/// an odd number of other closed shells (a void, or a void in a solid floating in a void); an
/// open one to the side with more area. A shell is the faces reached across sides used by
/// exactly two faces. Also the number of sides whose faces could not be made to agree (a
/// Moebius strip).
fn turns(m: &Model) -> (BTreeSet<FaceId>, usize) {
    let mut by_side: BTreeMap<(VertexId, VertexId), Vec<(FaceId, bool)>> = BTreeMap::new();
    for (id, f) in m.faces() {
        for (a, b) in sides(f) {
            by_side
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push((id, a < b));
        }
    }
    let mut turn: BTreeMap<FaceId, bool> = BTreeMap::new();
    let mut shells: Vec<(Vec<FaceId>, bool)> = Vec::new();
    let mut conflicts = 0;
    for (start, _) in m.faces() {
        if turn.contains_key(&start) {
            continue;
        }
        turn.insert(start, false);
        let mut shell = vec![start];
        let mut closed = true;
        let mut queue = VecDeque::from([start]);
        while let Some(f) = queue.pop_front() {
            let tf = turn[&f];
            for (a, b) in sides(m.face(f).expect("listed")) {
                let list = &by_side[&(a.min(b), a.max(b))];
                let [(f1, d1), (f2, d2)] = list[..] else {
                    closed = false;
                    continue;
                };
                if f1 == f2 {
                    continue;
                }
                let ((_, df), (g, dg)) = if f1 == f {
                    ((f1, d1), (f2, d2))
                } else {
                    ((f2, d2), (f1, d1))
                };
                // The neighbour must run the side the other way round once both are turned.
                let tg = dg == (df != tf);
                match turn.get(&g) {
                    Some(&t) => {
                        if t != tg {
                            conflicts += 1;
                        }
                    }
                    None => {
                        turn.insert(g, tg);
                        shell.push(g);
                        queue.push_back(g);
                    }
                }
            }
        }
        shells.push((shell, closed));
    }
    let depth = closed_depths(m, &shells);
    let mut out = BTreeSet::new();
    for (k, (shell, closed)) in shells.into_iter().enumerate() {
        let invert = if closed {
            let six: f64 = shell
                .iter()
                .map(|f| face_volume(m, *f) * if turn[f] { -1.0 } else { 1.0 })
                .sum();
            (six < 0.0) != (depth[k] % 2 == 1)
        } else {
            let (mut turned, mut kept) = (0.0, 0.0);
            for f in &shell {
                let a = m.face_area(*f).abs();
                if turn[f] { turned += a } else { kept += a }
            }
            turned > kept
        };
        out.extend(shell.into_iter().filter(|f| turn[f] != invert));
    }
    // Each conflict is seen from both faces.
    (out, conflicts / 2)
}

/// For each of `shells` (faces, and whether closed) how many other closed shells hold it; 0
/// for an open one. Shells are tested only against those whose bounding box holds theirs.
fn closed_depths(m: &Model, shells: &[(Vec<FaceId>, bool)]) -> Vec<usize> {
    let bounds = |faces: &[FaceId]| {
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for f in faces {
            for v in &m.face(*f).expect("listed").outer {
                lo = lo.min(m.pos(*v));
                hi = hi.max(m.pos(*v));
            }
        }
        (lo, hi)
    };
    let boxes: Vec<_> = shells.iter().map(|(faces, _)| bounds(faces)).collect();
    let mut parts: BTreeMap<usize, Model> = BTreeMap::new();
    let mut depth = vec![0; shells.len()];
    for (i, (faces, closed)) in shells.iter().enumerate() {
        if !closed {
            continue;
        }
        let (lo, hi) = boxes[i];
        let holders: Vec<usize> = (0..shells.len())
            .filter(|&j| {
                j != i && shells[j].1 && boxes[j].0.cmple(lo).all() && boxes[j].1.cmpge(hi).all()
            })
            .collect();
        if holders.is_empty() {
            continue;
        }
        let Some(p) = faces.iter().find_map(|f| interior_point(m, *f)) else {
            continue;
        };
        for j in holders {
            let part = parts.entry(j).or_insert_with(|| subset(m, &shells[j].0));
            if point_inside(p, part) == Some(true) {
                depth[i] += 1;
            }
        }
    }
    depth
}

/// The vertex pairs across tiny gaps: both on open edges, nearer than the weld tolerance and
/// than a quarter of every edge at either, not joined by an edge and not on one face.
fn gaps(m: &Model, counts: &BTreeMap<(VertexId, VertexId), [u32; 2]>) -> Vec<(VertexId, VertexId)> {
    let tol = weld_tolerance(m);
    let mut open: BTreeSet<VertexId> = BTreeSet::new();
    for (_, e) in m.edges() {
        let [a, b] = e.v;
        if users(counts, a, b) == 1 {
            open.extend([a, b]);
        }
    }
    let shortest = |v: VertexId| {
        m.edges_at(v)
            .iter()
            .map(|e| {
                let [a, b] = m.edge(*e).expect("listed").v;
                m.pos(a).distance(m.pos(b))
            })
            .fold(f64::INFINITY, f64::min)
    };
    let mut sorted: Vec<(VertexId, DVec3)> = open.iter().map(|v| (*v, m.pos(*v))).collect();
    sorted.sort_by(|x, y| x.1.x.total_cmp(&y.1.x));
    let mut out = Vec::new();
    for i in 0..sorted.len() {
        let (a, pa) = sorted[i];
        for &(b, pb) in &sorted[i + 1..] {
            if pb.x - pa.x >= tol {
                break;
            }
            let d = pa.distance(pb);
            if d >= tol || d >= 0.25 * shortest(a) || d >= 0.25 * shortest(b) {
                continue;
            }
            if m.edge_between(a, b).is_some()
                || m.faces_at(a).any(|f| m.faces_at(b).any(|g| g == f))
            {
                continue;
            }
            out.push((a.min(b), a.max(b)));
        }
    }
    out.sort();
    out
}

/// What keeps `m` from being a solid.
pub fn check(m: &Model) -> Health {
    let crossing = crossing_faces(m);
    let reason = open_reason(m)
        .or_else(|| {
            (signed_volume(m).abs() <= MIN_VOLUME).then(|| "it encloses no volume".to_string())
        })
        .or_else(|| (!crossing.is_empty()).then(|| "its faces cross each other".to_string()));
    let mut h = Health {
        no_faces: m.faces().next().is_none(),
        no_volume: reason.as_deref() == Some("it encloses no volume"),
        reason,
        self_crossing: crossing,
        ..Health::default()
    };
    if h.is_solid() {
        return h;
    }
    let counts = side_counts(m);
    for (id, e) in m.edges() {
        let [a, b] = e.v;
        match users(&counts, a, b) {
            0 => h.stray_edges.push(id),
            1 => h.open_edges.push(id),
            2 => {}
            _ => h.shared_edges.push(id),
        }
    }
    if !h.shared_edges.is_empty() {
        h.internal_faces = internal_faces(m, &counts);
    }
    h.reversed_faces = turns(m).0.into_iter().collect();
    h.gaps = gaps(m, &counts);
    h
}

fn internal_faces(m: &Model, counts: &BTreeMap<(VertexId, VertexId), [u32; 2]>) -> Vec<FaceId> {
    m.faces()
        .filter(|(_, f)| sides(f).all(|(a, b)| users(counts, a, b) >= 3))
        .map(|(id, _)| id)
        .collect()
}

/// Whether face `id` of `m` keeps the face invariants (loops of three or more distinct
/// vertices joined by edges, flat within `LENGTH_TOL`, wound the right way) and triangulates.
fn face_ok(m: &Model, id: FaceId) -> bool {
    let Some(f) = m.face(id) else {
        return true;
    };
    let origin = m.pos(f.outer[0]);
    for (k, l) in std::iter::once(&f.outer).chain(&f.holes).enumerate() {
        if l.len() < 3 || l.iter().collect::<BTreeSet<_>>().len() != l.len() {
            return false;
        }
        for i in 0..l.len() {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            if m.edge_between(a, b).is_none()
                || (m.pos(a) - origin).dot(f.normal).abs() > LENGTH_TOL
            {
                return false;
            }
        }
        let pts: Vec<DVec3> = l.iter().map(|v| m.pos(*v)).collect();
        let signed = vector_area(&pts).dot(f.normal);
        if (k == 0) != (signed > 0.0) {
            return false;
        }
    }
    m.triangulate(id).is_some_and(|t| !t.is_empty())
}

/// Whether an edge of `m` passes through the inside of face `f`: crossing its plane there, or
/// ending there on a vertex that is not the face's own (a cap on a hole that would cut through
/// the part).
fn pierced(m: &Model, f: FaceId) -> bool {
    let Some(face) = m.face(f) else {
        return false;
    };
    let plane = Plane::through(m.pos(face.outer[0]), face.normal);
    let ring = |l: &[VertexId]| {
        l.iter()
            .map(|v| plane.project(m.pos(*v)))
            .collect::<Vec<_>>()
    };
    let outer = ring(&face.outer);
    let holes: Vec<Vec<DVec2>> = face.holes.iter().map(|h| ring(h)).collect();
    let own: BTreeSet<VertexId> = face
        .outer
        .iter()
        .chain(face.holes.iter().flatten())
        .copied()
        .collect();
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for v in &own {
        lo = lo.min(m.pos(*v));
        hi = hi.max(m.pos(*v));
    }
    let pad = DVec3::splat(LENGTH_TOL);
    let (lo, hi) = (lo - pad, hi + pad);
    let within = |p: DVec3| {
        let q = plane.project(p);
        point_in_polygon(q, &outer) && !holes.iter().any(|h| point_in_polygon(q, h))
    };
    for (_, e) in m.edges() {
        let [a, b] = e.v;
        let (pa, pb) = (m.pos(a), m.pos(b));
        if pa.min(pb).cmpgt(hi).any() || pa.max(pb).cmplt(lo).any() {
            continue;
        }
        let (da, db) = (plane.distance(pa), plane.distance(pb));
        for (v, p, d) in [(a, pa, da), (b, pb, db)] {
            if d.abs() <= LENGTH_TOL && !own.contains(&v) && within(p) {
                return true;
            }
        }
        let crosses =
            (da > LENGTH_TOL && db < -LENGTH_TOL) || (da < -LENGTH_TOL && db > LENGTH_TOL);
        if crosses && within(pa + (pb - pa) * (da / (da - db))) {
            return true;
        }
    }
    false
}

/// Whether any two sides of `rings` (closed 2D loops) that do not follow each other in one
/// loop touch or cross.
fn crosses(rings: &[Vec<DVec2>]) -> bool {
    let orient = |a: DVec2, b: DVec2, c: DVec2| (b - a).perp_dot(c - a);
    let touch = |p: DVec2, q: DVec2, r: DVec2, s: DVec2| {
        let (d1, d2) = (orient(r, s, p), orient(r, s, q));
        let (d3, d4) = (orient(p, q, r), orient(p, q, s));
        let on = |d: f64, a: DVec2, b: DVec2, c: DVec2| {
            d == 0.0 && c.cmpge(a.min(b)).all() && c.cmple(a.max(b)).all()
        };
        (d1 * d2 < 0.0 && d3 * d4 < 0.0)
            || on(d1, r, s, p)
            || on(d2, r, s, q)
            || on(d3, p, q, r)
            || on(d4, p, q, s)
    };
    let mut segments: Vec<(usize, usize, DVec2, DVec2)> = rings
        .iter()
        .enumerate()
        .flat_map(|(k, l)| (0..l.len()).map(move |i| (k, i, l[i], l[(i + 1) % l.len()])))
        .collect();
    // Swept by x: only sides whose x ranges overlap can touch.
    segments.sort_by(|a, b| a.2.x.min(a.3.x).total_cmp(&b.2.x.min(b.3.x)));
    for (x, &(k, i, p, q)) in segments.iter().enumerate() {
        let right = p.x.max(q.x);
        for &(k2, j, r, s) in &segments[x + 1..] {
            if r.x.min(s.x) > right {
                break;
            }
            let n = rings[k].len();
            if k == k2 && (j == (i + 1) % n || i == (j + 1) % n) {
                continue;
            }
            if touch(p, q, r, s) {
                return true;
            }
        }
    }
    false
}

impl Model {
    /// Merges the vertices across `pairs` (each group into its lowest id) and refits the
    /// normals of the faces at the kept vertices. Returns how many vertices went and the
    /// faces refitted.
    fn weld_gaps(&mut self, pairs: &[(VertexId, VertexId)]) -> (usize, BTreeSet<FaceId>) {
        let mut root: BTreeMap<VertexId, VertexId> = BTreeMap::new();
        fn find(root: &BTreeMap<VertexId, VertexId>, mut v: VertexId) -> VertexId {
            while let Some(&r) = root.get(&v) {
                if r == v {
                    break;
                }
                v = r;
            }
            v
        }
        for &(a, b) in pairs {
            let (ra, rb) = (find(&root, a), find(&root, b));
            if ra != rb {
                root.insert(ra.max(rb), ra.min(rb));
            }
        }
        let target: BTreeMap<VertexId, VertexId> = root
            .keys()
            .map(|v| (*v, find(&root, *v)))
            .filter(|(v, r)| v != r)
            .collect();
        if target.is_empty() {
            return (0, BTreeSet::new());
        }
        self.weld_vertices(&target);
        let kept: BTreeSet<VertexId> = target.values().copied().collect();
        let faces: BTreeSet<FaceId> = kept.iter().flat_map(|v| self.faces_at(*v)).collect();
        for &id in &faces {
            let f = self.face(id).expect("listed").clone();
            let pts: Vec<DVec3> = f.outer.iter().map(|v| self.pos(*v)).collect();
            let n = vector_area(&pts).normalize_or_zero();
            if n != DVec3::ZERO && n.dot(f.normal) > 0.0 && n != f.normal {
                self.replace_face(id, Face { normal: n, ..f });
            }
        }
        (target.len(), faces)
    }

    /// Fills the holes left by open edges with faces where that is safe (see the module
    /// notes in `repair`). Returns the faces made and why the other holes were left.
    fn fill_holes(&mut self) -> (Vec<FaceId>, Vec<String>) {
        let counts = side_counts(self);
        // The new faces run each open side the other way round.
        let mut next: BTreeMap<VertexId, Vec<VertexId>> = BTreeMap::new();
        for (&(lo, hi), &[up, down]) in &counts {
            match (up, down) {
                (1, 0) => next.entry(hi).or_default().push(lo),
                (0, 1) => next.entry(lo).or_default().push(hi),
                _ => {}
            }
        }
        let mut reasons = Vec::new();
        let name = |m: &Model, a: VertexId, b: VertexId| match m.edge_between(a, b) {
            Some(e) => format!("e{}", e.0),
            None => format!("v{}-v{}", a.0, b.0),
        };
        let mut used: BTreeSet<VertexId> = BTreeSet::new();
        let mut loops: Vec<Vec<VertexId>> = Vec::new();
        let starts: Vec<VertexId> = next.keys().copied().collect();
        for start in starts {
            if used.contains(&start) {
                continue;
            }
            let mut l = vec![start];
            let mut v = start;
            let ok = loop {
                used.insert(v);
                let Some(to) = next.get(&v) else {
                    break Err(format!(
                        "the open edges at {} do not close into a loop",
                        name(self, start, l.get(1).copied().unwrap_or(start))
                    ));
                };
                if to.len() > 1 {
                    break Err(format!("holes meet at vertex v{}", v.0));
                }
                v = to[0];
                if v == start {
                    break Ok(());
                }
                if used.contains(&v) {
                    break Err(format!("holes meet at vertex v{}", v.0));
                }
                l.push(v);
            };
            match ok {
                Ok(()) => loops.push(l),
                Err(why) => {
                    if !reasons.contains(&why) {
                        reasons.push(why);
                    }
                }
            }
        }

        // Flat loops with their normal and plane.
        struct Flat {
            l: Vec<VertexId>,
            normal: DVec3,
            plane: Plane,
        }
        let mut flat: Vec<Flat> = Vec::new();
        for l in loops {
            let pts: Vec<DVec3> = l.iter().map(|v| self.pos(*v)).collect();
            let edge = name(self, l[0], l[1]);
            let area = vector_area(&pts);
            if l.len() < 3 || area.length() <= LENGTH_TOL * LENGTH_TOL {
                reasons.push(format!("the hole at {edge} has no area"));
                continue;
            }
            let normal = area.normalize();
            if pts
                .iter()
                .any(|p| (*p - pts[0]).dot(normal).abs() > LENGTH_TOL)
            {
                reasons.push(format!("the hole at {edge} is not flat"));
                continue;
            }
            flat.push(Flat {
                plane: Plane::through(pts[0], normal),
                l,
                normal,
            });
        }
        // Which loops lie inside which, in one plane; a loop's box must lie in the other's.
        let boxes: Vec<(DVec3, DVec3)> = flat
            .iter()
            .map(|f| {
                f.l.iter().fold(
                    (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
                    |(lo, hi), v| (lo.min(self.pos(*v)), hi.max(self.pos(*v))),
                )
            })
            .collect();
        let inside = |i: usize, j: usize| -> bool {
            let (a, b) = (&flat[i], &flat[j]);
            let pad = DVec3::splat(LENGTH_TOL);
            if (boxes[i].0 + pad).cmplt(boxes[j].0).any()
                || (boxes[i].1 - pad).cmpgt(boxes[j].1).any()
                || a.normal.dot(b.normal).abs() < 1.0 - PLANE_ANGLE_TOL
                || b.plane.distance(self.pos(a.l[0])).abs() > LENGTH_TOL
            {
                return false;
            }
            let poly: Vec<_> = b.l.iter().map(|v| b.plane.project(self.pos(*v))).collect();
            a.l.iter()
                .all(|v| point_in_polygon(b.plane.project(self.pos(*v)), &poly))
        };
        let containers: Vec<Vec<usize>> = (0..flat.len())
            .map(|i| {
                (0..flat.len())
                    .filter(|&j| j != i && inside(i, j))
                    .collect()
            })
            .collect();
        let mut holes_of: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut outers = Vec::new();
        for i in 0..flat.len() {
            match containers[i][..] {
                [] => outers.push(i),
                [j] if containers[j].is_empty() && flat[i].normal.dot(flat[j].normal) < 0.0 => {
                    holes_of.entry(j).or_default().push(i)
                }
                _ => reasons.push(format!(
                    "the hole at {} lies inside another hole",
                    name(self, flat[i].l[0], flat[i].l[1])
                )),
            }
        }
        let mut made = Vec::new();
        for i in outers {
            let holes: Vec<Vec<VertexId>> = holes_of
                .get(&i)
                .into_iter()
                .flatten()
                .map(|h| flat[*h].l.clone())
                .collect();
            let plane = flat[i].plane;
            let rings: Vec<Vec<_>> = std::iter::once(&flat[i].l)
                .chain(&holes)
                .map(|l| l.iter().map(|v| plane.project(self.pos(*v))).collect())
                .collect();
            if crosses(&rings) {
                reasons.push(format!(
                    "the hole at {} crosses itself",
                    name(self, flat[i].l[0], flat[i].l[1])
                ));
                continue;
            }
            made.push(self.insert_face(Face::new(flat[i].l.clone(), holes, flat[i].normal)));
        }
        (made, reasons)
    }
}

/// Closes `m` where that is safe, on a copy: welds vertices across tiny gaps, removes stray
/// edges and internal faces, makes each shell agree, fills flat holes and turns each closed
/// shell outward. The copy is handed back only when it is a solid and every face touched or
/// made is valid; a solid is never touched.
pub fn repair(m: &Model) -> Repaired {
    let before = check(m);
    if before.is_solid() {
        return Repaired {
            model: None,
            repair: Repair::default(),
            left: before,
        };
    }
    let mut c = m.clone();
    let mut r = Repair::default();
    let mut touched: BTreeSet<FaceId> = BTreeSet::new();

    let (welded, refitted) = c.weld_gaps(&before.gaps);
    r.welded = welded;
    touched.extend(refitted);

    let counts = side_counts(&c);
    let strays: Vec<EdgeId> = c
        .edges()
        .filter(|(_, e)| users(&counts, e.v[0], e.v[1]) == 0)
        .map(|(id, _)| id)
        .collect();
    for e in &strays {
        let edge = c.remove_edge(*e);
        for v in edge.v {
            if c.edges_at(v).is_empty() && c.faces_at(v).next().is_none() {
                c.remove_vertex(v);
            }
        }
    }
    r.strays_removed = strays.len();

    let internal = internal_faces(&c, &counts);
    for f in &internal {
        c.remove_face(*f);
    }
    r.internal_removed = internal.len();

    // Turned here rather than by `reverse_faces`, which would end the edit.
    let turn_over = |c: &mut Model, faces: BTreeSet<FaceId>| {
        for f in faces {
            let turned = c.face(f).expect("listed").reversed();
            c.replace_face(f, turned);
        }
    };
    let (agree, _) = turns(&c);
    turn_over(&mut c, agree);
    let (made, mut reasons) = c.fill_holes();
    let mut cut = false;
    for &f in &made {
        if pierced(&c, f) {
            cut = true;
            let l = &c.face(f).expect("made").outer;
            let e = c.edge_between(l[0], l[1]).expect("a side");
            reasons.push(format!(
                "a face filling the hole at e{} would cut through the part",
                e.0
            ));
        }
    }
    r.holes_filled = made.len();
    r.reasons = reasons;
    touched.extend(made);
    let (outward, conflicts) = turns(&c);
    turn_over(&mut c, outward);
    if conflicts > 0 {
        r.reasons
            .push("some faces cannot be made to agree (the surface has one side)".into());
    }

    r.faces_flipped = m
        .faces()
        .filter(|(id, f)| c.face(*id).is_some_and(|g| g.normal.dot(f.normal) < 0.0))
        .count();
    let bad: Vec<FaceId> = touched
        .iter()
        .copied()
        .filter(|f| !face_ok(&c, *f))
        .collect();
    for f in &bad {
        r.reasons
            .push(format!("face f{} would not be flat or valid", f.0));
    }
    let left = check(&c);
    if !left.self_crossing.is_empty() {
        r.reasons
            .push("faces of the part would cross each other".into());
    }
    let model = (left.is_solid() && bad.is_empty() && !cut).then_some(c);
    Repaired {
        model,
        repair: r,
        left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Solid;

    fn cube() -> Model {
        Solid::cuboid(DVec3::new(1.0, 2.0, 3.0)).unwrap().to_model()
    }

    /// `m` with `pos` given its own vertex in face `f`'s loops: a copy of that corner
    /// `offset` away, joined by new edges to its neighbours in those loops.
    fn split_corner(m: &mut Model, f: FaceId, at: VertexId, offset: DVec3) -> VertexId {
        let copy = m.insert_vertex(m.pos(at) + offset);
        let mut face = m.face(f).unwrap().clone();
        let n = face.outer.len();
        let i = face.outer.iter().position(|v| *v == at).unwrap();
        let (prev, next) = (face.outer[(i + n - 1) % n], face.outer[(i + 1) % n]);
        face.outer[i] = copy;
        m.replace_face(f, face);
        for other in [prev, next] {
            if m.edge_between(copy, other).is_none() {
                m.insert_edge(copy, other);
            }
        }
        // The old sides stay as edges of the neighbouring faces.
        copy
    }

    /// A model of `faces` (loops of indices into `points`, wound around their outside), one
    /// vertex per distinct point.
    fn build(points: &[DVec3], faces: &[Vec<usize>]) -> Model {
        let mut m = Model::new();
        let mut at: Vec<(DVec3, VertexId)> = Vec::new();
        let mut vertex = |m: &mut Model, p: DVec3| match at.iter().find(|(q, _)| *q == p) {
            Some((_, v)) => *v,
            None => {
                let v = m.insert_vertex(p);
                at.push((p, v));
                v
            }
        };
        for f in faces {
            let l: Vec<VertexId> = f.iter().map(|i| vertex(&mut m, points[*i])).collect();
            for i in 0..l.len() {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                if m.edge_between(a, b).is_none() {
                    m.insert_edge(a, b);
                }
            }
            let pts: Vec<DVec3> = f.iter().map(|i| points[*i]).collect();
            m.insert_face(Face::new(l, Vec::new(), vector_area(&pts).normalize()));
        }
        m.take_changes();
        m
    }

    /// The corners of the box from `lo` to `hi`: index bit 0 is x, bit 1 y, bit 2 z.
    fn corners(lo: DVec3, hi: DVec3) -> Vec<DVec3> {
        (0..8)
            .map(|i| {
                let pick = |bit: usize, a: f64, b: f64| if i & bit == 0 { a } else { b };
                DVec3::new(
                    pick(1, lo.x, hi.x),
                    pick(2, lo.y, hi.y),
                    pick(4, lo.z, hi.z),
                )
            })
            .collect()
    }

    /// The box's sides, `+ offset` into the points: bottom, top, front, back, left, right.
    fn sides_of_box(offset: usize) -> Vec<Vec<usize>> {
        [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ]
        .iter()
        .map(|q| q.iter().map(|i| i + offset).collect())
        .collect()
    }

    fn volume(m: &Model) -> f64 {
        signed_volume(m)
    }

    /// The 2 m box from the origin with a void (the box from 0.5 to 1.5 m, its faces facing
    /// into the void): a solid of 7 m³.
    fn hollow() -> Model {
        let mut points = corners(DVec3::ZERO, DVec3::splat(2.0));
        points.extend(corners(DVec3::splat(0.5), DVec3::splat(1.5)));
        let mut faces = sides_of_box(0);
        faces.extend(
            sides_of_box(8)
                .into_iter()
                .map(|q| q.into_iter().rev().collect()),
        );
        build(&points, &faces)
    }

    #[test]
    fn a_void_keeps_facing_in_when_the_solid_around_it_is_repaired() {
        let m = hollow();
        assert!(check(&m).is_solid());
        assert!((volume(&m) - 7.0).abs() < 1e-9);

        let mut stray = m.clone();
        stray
            .add_edge(DVec3::splat(3.0), DVec3::splat(4.0))
            .unwrap();
        stray.take_changes();
        let h = check(&stray);
        assert!(h.reversed_faces.is_empty(), "{h:?}");
        let r = repair(&stray);
        assert_eq!(r.repair.faces_flipped, 0);
        assert!((volume(&r.model.expect("closed")) - 7.0).abs() < 1e-9);

        let mut open = m.clone();
        let top = open
            .faces()
            .find(|(_, f)| f.normal.z > 0.5 && open.pos(f.outer[0]).z > 1.9)
            .map(|(id, _)| id)
            .unwrap();
        open.remove_face(top);
        open.take_changes();
        let r = repair(&open);
        assert_eq!(r.repair.faces_flipped, 0);
        let fixed = r.model.expect("closed");
        assert!((volume(&fixed) - 7.0).abs() < 1e-9);
        fixed.validate().unwrap();
    }

    #[test]
    fn a_ring_with_sixteen_hundred_holes_is_checked_for_crossings_quickly() {
        let mut rings = vec![vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(40.0, 0.0),
            DVec2::new(40.0, 40.0),
            DVec2::new(0.0, 40.0),
        ]];
        for k in 0..1600 {
            let c = DVec2::new((k % 40) as f64 + 0.5, (k / 40) as f64 + 0.5);
            rings.push(
                (0..16)
                    .rev()
                    .map(|i| {
                        let a = std::f64::consts::TAU * i as f64 / 16.0;
                        c + DVec2::new(a.cos(), a.sin()) * 0.3
                    })
                    .collect(),
            );
        }
        let t = std::time::Instant::now();
        assert!(!crosses(&rings));
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        // A hole moved onto its neighbour crosses it.
        for p in rings[2].iter_mut() {
            p.x -= 0.5;
        }
        assert!(crosses(&rings));
    }

    #[test]
    fn a_hole_whose_cap_would_cut_through_the_part_is_not_filled() {
        // A 2 x 2 x 1 m box missing its top, with a pocket from its floor rising to 1.5 m:
        // a cap on the rim would cut the pocket's walls.
        let mut points = corners(DVec3::ZERO, DVec3::new(2.0, 2.0, 1.0));
        points.extend(corners(
            DVec3::new(0.5, 0.5, 0.0),
            DVec3::new(1.5, 1.5, 1.5),
        ));
        let mut faces: Vec<Vec<usize>> = sides_of_box(0)[2..].to_vec();
        faces.extend(
            sides_of_box(8)[1..]
                .iter()
                .map(|q| q.iter().rev().copied().collect()),
        );
        let mut m = build(&points, &faces);
        let at = |m: &Model, i: usize| {
            m.vertices()
                .find(|(_, v)| v.pos == points[i])
                .map(|(id, _)| id)
                .unwrap()
        };
        let outer = [0, 2, 3, 1].map(|i| at(&m, i)).to_vec();
        let hole = [8, 9, 11, 10].map(|i| at(&m, i)).to_vec();
        m.insert_face(Face::new(outer, vec![hole], DVec3::NEG_Z));
        m.take_changes();
        let h = check(&m);
        assert_eq!(h.open_edges.len(), 4, "{h:?}");
        let r = repair(&m);
        assert!(r.model.is_none(), "{:?}", r.repair);
        assert!(
            r.repair.reasons.iter().any(|s| s.contains("cut through")),
            "{:?}",
            r.repair.reasons
        );
    }

    #[test]
    fn a_box_with_one_face_missing_is_filled() {
        let mut m = cube();
        let f = m.faces().next().unwrap().0;
        m.remove_face(f);
        m.take_changes();
        let h = check(&m);
        assert_eq!(h.open_edges.len(), 4);
        assert!(h.reversed_faces.is_empty() && h.shared_edges.is_empty());
        let r = repair(&m);
        assert_eq!(r.repair.holes_filled, 1);
        let fixed = r.model.unwrap();
        fixed.validate().unwrap();
        assert!((volume(&fixed) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn a_box_with_a_face_turned_over_is_oriented() {
        let mut m = cube();
        let f = m.faces().nth(2).unwrap().0;
        m.reverse_faces(&[f]);
        let h = check(&m);
        assert_eq!(h.reversed_faces, vec![f]);
        let r = repair(&m);
        assert_eq!(r.repair.faces_flipped, 1);
        let fixed = r.model.unwrap();
        assert!((volume(&fixed) - 6.0).abs() < 1e-9);
        fixed.validate().unwrap();
    }

    #[test]
    fn two_boxes_sharing_one_edge_are_reported_not_fixed() {
        let mut points = corners(DVec3::ZERO, DVec3::ONE);
        points.extend(corners(
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(2.0, 2.0, 1.0),
        ));
        let mut faces = sides_of_box(0);
        faces.extend(sides_of_box(8));
        let m = build(&points, &faces);
        let h = check(&m);
        assert_eq!(h.shared_edges.len(), 1, "{h:?}");
        assert!(h.internal_faces.is_empty());
        let r = repair(&m);
        assert!(r.model.is_none());
        assert_eq!(r.left.shared_edges, h.shared_edges);
    }

    #[test]
    fn a_stray_edge_on_a_box_is_removed() {
        let mut m = cube();
        m.add_edge(DVec3::ZERO, DVec3::splat(-1.0)).unwrap();
        let h = check(&m);
        assert_eq!(h.stray_edges.len(), 1);
        let r = repair(&m);
        assert_eq!(r.repair.strays_removed, 1);
        let fixed = r.model.unwrap();
        assert_eq!(fixed.vertices().count(), 8);
        assert_eq!(fixed.edges().count(), 12);
        fixed.validate().unwrap();
    }

    #[test]
    fn a_solid_sphere_is_left_alone() {
        let m = Solid::sphere(1.0, 16).unwrap().to_model();
        assert!(check(&m).is_solid());
        let r = repair(&m);
        assert!(r.model.is_none());
        assert_eq!(r.repair, Repair::default());
        assert!(r.left.is_solid());
    }

    #[test]
    fn a_tube_missing_its_top_ring_gets_a_ring_with_a_hole() {
        let mut m = Solid::tube(2.0, 1.0, 3.0, 16).unwrap().to_model();
        let whole = volume(&m);
        let top = m
            .faces()
            .find(|(_, f)| f.normal.z > 0.5)
            .map(|(id, _)| id)
            .unwrap();
        m.remove_face(top);
        m.take_changes();
        let r = repair(&m);
        assert_eq!(r.repair.holes_filled, 1, "{:?}", r.repair);
        let fixed = r.model.unwrap();
        assert!((volume(&fixed) - whole).abs() < 1e-9);
        let made = fixed.faces().find(|(_, f)| f.normal.z > 0.5).unwrap().1;
        assert_eq!(made.holes.len(), 1);
        fixed.validate().unwrap();
    }

    #[test]
    fn a_hole_that_is_not_flat_is_reported() {
        // A cube of triangles missing two that meet across the top front edge.
        let points = corners(DVec3::ZERO, DVec3::ONE);
        let faces: Vec<Vec<usize>> = sides_of_box(0)
            .iter()
            .flat_map(|q| [vec![q[0], q[1], q[2]], vec![q[0], q[2], q[3]]])
            .filter(|t| *t != vec![4, 5, 7] && *t != vec![0, 5, 4])
            .collect();
        assert_eq!(faces.len(), 10);
        let m = build(&points, &faces);
        let r = repair(&m);
        assert!(r.model.is_none());
        assert!(
            r.repair.reasons.iter().any(|s| s.contains("not flat")),
            "{:?}",
            r.repair.reasons
        );
        assert!(!r.left.open_edges.is_empty());
    }

    #[test]
    fn a_wall_inside_a_box_is_removed() {
        let mut points = corners(DVec3::ZERO, DVec3::new(0.5, 1.0, 1.0));
        points.extend(corners(DVec3::new(0.5, 0.0, 0.0), DVec3::ONE));
        let left = sides_of_box(0);
        let right = sides_of_box(8);
        // Both halves but their inner sides, and the wall between them once.
        let mut faces: Vec<Vec<usize>> = left[..5].to_vec();
        faces.extend(right[..4].iter().cloned());
        faces.push(right[5].clone());
        faces.push(left[5].clone());
        let m = build(&points, &faces);
        let h = check(&m);
        assert_eq!(h.shared_edges.len(), 4);
        assert_eq!(h.internal_faces.len(), 1);
        let r = repair(&m);
        assert_eq!(r.repair.internal_removed, 1);
        let fixed = r.model.unwrap();
        assert!((volume(&fixed) - 1.0).abs() < 1e-9);
        fixed.validate().unwrap();
    }

    #[test]
    fn a_lone_face_cannot_be_made_a_solid() {
        let mut m = Model::new();
        m.add_polygon(&[DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y])
            .unwrap();
        let r = repair(&m);
        assert!(r.model.is_none());
        assert!(r.left.no_volume, "{:?}", r.left);
    }

    #[test]
    fn a_box_missing_a_face_with_another_turned_over_is_closed_outward() {
        let mut m = cube();
        let ids: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
        m.remove_face(ids[0]);
        m.take_changes();
        m.reverse_faces(&[ids[3]]);
        let r = repair(&m);
        let fixed = r.model.expect("closed");
        assert!((volume(&fixed) - 6.0).abs() < 1e-9);
        assert_eq!(r.repair.faces_flipped, 1);
    }

    #[test]
    fn vertices_a_tenth_of_a_micron_apart_are_welded() {
        let mut m = cube();
        let f = m.faces().next().unwrap().0;
        let at = m.face(f).unwrap().outer[0];
        split_corner(&mut m, f, at, DVec3::new(1e-7, 0.0, 0.0));
        m.take_changes();
        let h = check(&m);
        assert!(!h.is_solid());
        assert_eq!(h.gaps.len(), 1, "{h:?}");
        let r = repair(&m);
        let fixed = r.model.expect("welded");
        assert_eq!(r.repair.welded, 1);
        assert!(check(&fixed).is_solid());
        fixed.validate().unwrap();
        assert_eq!(fixed.vertices().count(), 8);
    }

    #[test]
    fn a_gap_wider_than_the_tolerance_is_not_welded() {
        let mut m = cube();
        let f = m.faces().next().unwrap().0;
        let at = m.face(f).unwrap().outer[0];
        // 1e-5 of the 3.7 m diagonal is 37 µm.
        split_corner(&mut m, f, at, DVec3::new(1e-4, 0.0, 0.0));
        m.take_changes();
        assert!(check(&m).gaps.is_empty());
        assert_eq!(repair(&m).repair.welded, 0);
    }

    #[test]
    fn the_weld_tolerance_scales_with_the_part_between_floor_and_ceiling() {
        let tol = |s: f64| weld_tolerance(&Solid::cuboid(DVec3::splat(s)).unwrap().to_model());
        assert!((tol(1.0) - 1e-5 * 3f64.sqrt()).abs() < 1e-12);
        assert_eq!(tol(0.01), LENGTH_TOL);
        assert_eq!(tol(1000.0), MAX_WELD);
    }

    /// The box of triangles from the origin to `hi` with its top corner (index 7) moved to `top`.
    fn box_with_corner(hi: DVec3, top: DVec3) -> Model {
        let mut points = corners(DVec3::ZERO, hi);
        points[7] = top;
        let triangles: Vec<Vec<usize>> = sides_of_box(0)
            .iter()
            .flat_map(|q| [vec![q[0], q[1], q[2]], vec![q[0], q[2], q[3]]])
            .collect();
        build(&points, &triangles)
    }

    #[test]
    fn valid_shells_are_not_reported_as_crossing() {
        for m in [
            cube(),
            hollow(),
            Solid::sphere(1.0, 16).unwrap().to_model(),
            Solid::tube(2.0, 1.0, 3.0, 16).unwrap().to_model(),
        ] {
            let h = check(&m);
            assert!(h.self_crossing.is_empty(), "{h:?}");
            assert!(h.is_solid());
        }
    }

    #[test]
    fn a_corner_pushed_through_the_opposite_wall_is_reported() {
        // The corner at (1, 1, 1) pulled down through the bottom face: the shell is closed
        // with volume, yet it crosses itself.
        let m = box_with_corner(DVec3::ONE, DVec3::new(0.7, 0.6, -0.5));
        let h = check(&m);
        assert!(
            h.open_edges.is_empty() && h.shared_edges.is_empty(),
            "{h:?}"
        );
        assert!(!h.self_crossing.is_empty(), "{h:?}");
        assert!(!h.is_solid());
        assert_eq!(h.reason.as_deref(), Some("its faces cross each other"));
        let r = repair(&m);
        assert!(r.model.is_none());
        assert!(
            r.repair
                .reasons
                .iter()
                .any(|s| s.contains("cross each other")),
            "{:?}",
            r.repair.reasons
        );
    }

    #[test]
    fn two_solids_overlapping_in_one_mesh_are_not_a_crossing_shell() {
        let mut points = corners(DVec3::ZERO, DVec3::ONE);
        points.extend(corners(DVec3::splat(0.5), DVec3::splat(1.5)));
        let mut faces = sides_of_box(0);
        faces.extend(sides_of_box(8));
        let m = build(&points, &faces);
        assert!(check(&m).self_crossing.is_empty());
    }

    #[test]
    fn a_face_folded_back_over_its_neighbour_is_reported() {
        // A box whose top is two triangles; one is turned about its diagonal to lie over the
        // other half of the top (coplanar, overlapping).
        let points = corners(DVec3::ZERO, DVec3::ONE);
        let mut faces: Vec<Vec<usize>> = sides_of_box(0)[..1].to_vec();
        faces.extend(sides_of_box(0)[2..].iter().cloned());
        faces.push(vec![4, 5, 7]);
        faces.push(vec![4, 7, 6]);
        let ok = build(&points, &faces);
        assert!(check(&ok).self_crossing.is_empty());
        // Move vertex 6 onto the far side of the diagonal 5-4... over the first triangle.
        let mut points = points;
        points[6] = DVec3::new(0.9, 0.2, 1.0);
        let folded = build(&points, &faces);
        assert!(!check(&folded).self_crossing.is_empty());
    }

    #[test]
    fn a_big_sphere_is_checked_for_crossings_quickly() {
        let m = Solid::sphere(1.0, 200).unwrap().to_model();
        let t = std::time::Instant::now();
        assert!(check(&m).self_crossing.is_empty());
        assert!(t.elapsed() < std::time::Duration::from_secs(10));
    }
}
