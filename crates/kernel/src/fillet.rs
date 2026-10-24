//! Chamfers of straight edges of a solid.
//!
//! Each chosen edge lies between two flat faces. Along it a prism is built whose cross-section
//! is the region between the two faces and the chamfer line, reaching a little past both faces
//! so the boolean cuts them cleanly: a convex edge's prism (a cutter) is subtracted, a concave
//! edge's (a filler) is added, with the boolean core of `boolean.rs`. At an end shared with
//! another chosen convex edge, a cutter runs on past the end (meeting cutters overlap and leave
//! a closed corner); elsewhere the prism ends in the plane of the face at that end, so no
//! neighbouring face is notched. The result replaces the mesh's own geometry; faces and edges
//! the cut did not touch keep their ids.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use glam::{DAffine3, DVec2, DVec3};

use crate::boolean::{BooleanError, BooleanOp, apart, combine, placed_copy};
use crate::geom::{LENGTH_TOL, vector_area};
use crate::ids::{CurveId, EdgeId, FaceId, VertexId};
use crate::model::{Face, Model};
use crate::scene::{MeshId, Scene, SceneChange};
use crate::solid::{open_reason, signed_volume};

/// The smallest chamfer distance, in meters.
pub const MIN_SIZE: f64 = 1e-6;
/// The most edges one command bevels.
pub const MAX_EDGES: usize = 1000;
/// Faces whose normals are closer than this (degrees) are tangent: no edge to bevel.
pub const MIN_ANGLE_DEG: f64 = 1.0;

/// The shape left where an edge was.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Profile {
    /// A flat face set back `distance` on both faces.
    Chamfer { distance: f64 },
}

impl Profile {
    /// How far the cut reaches into each face.
    fn size(self) -> f64 {
        match self {
            Profile::Chamfer { distance } => distance,
        }
    }

    /// The parameter's name in messages.
    fn noun(self) -> &'static str {
        match self {
            Profile::Chamfer { .. } => "distance",
        }
    }

    /// The prism's cross-section around `cut`, as offsets from a point on the edge (all
    /// perpendicular to it): the region between the faces and the profile, widened past both
    /// faces away from the material it does not change (outward for a cutter, inward for a
    /// filler) so none of its faces lies in a face of the solid.
    fn section(self, cut: &EdgeCut) -> Vec<DVec3> {
        let s = self.size();
        let reach = if cut.convex { s } else { -s };
        let (p1, p2) = (cut.d1 * s, cut.d2 * s);
        match self {
            Profile::Chamfer { .. } => vec![
                p1,
                p2,
                p2 + cut.n2 * reach,
                (cut.n1 + cut.n2) * reach,
                p1 + cut.n1 * reach,
            ],
        }
    }
}

/// Why edges could not be bevelled.
#[derive(Clone, Debug, PartialEq)]
pub enum BevelError {
    /// The mesh's own geometry is not a solid; the text says why.
    NotSolid(String),
    /// The distance or radius is out of range, or too many edges; the text says why.
    Invalid(String),
    /// This edge cannot be bevelled; the text says why and names it.
    Edge(EdgeId, String),
    /// The boolean failed.
    Boolean(BooleanError),
}

impl fmt::Display for BevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BevelError::NotSolid(why) => write!(f, "not a solid: {why}"),
            BevelError::Invalid(why) | BevelError::Edge(_, why) => f.write_str(why),
            BevelError::Boolean(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for BevelError {}

impl From<BooleanError> for BevelError {
    fn from(e: BooleanError) -> Self {
        BevelError::Boolean(e)
    }
}

impl Scene {
    /// Bevels `edges` of mesh `mesh` (whose own geometry must be a solid; nested instances are
    /// left alone) with `profile`, see [`bevelled`]. On an error nothing changes.
    pub fn bevel_edges(
        &mut self,
        mesh: MeshId,
        edges: &[EdgeId],
        profile: Profile,
    ) -> Result<SceneChange, BevelError> {
        let m = self
            .mesh(mesh)
            .ok_or_else(|| BevelError::NotSolid("it is missing".into()))?;
        let result = bevelled(m, edges, profile)?;
        Ok(self
            .edit(mesh, |m| {
                replace_geometry(m, &result);
                Ok(m.take_changes())
            })
            .expect("replacing geometry cannot fail"))
    }
}

/// How one end of a prism is closed.
#[derive(Clone, Copy, Debug)]
enum End {
    /// Runs on this far past the end vertex.
    Extend(f64),
    /// Ends in the plane through the point with this normal.
    Plane(DVec3, DVec3),
}

/// One chosen edge and the faces beside it.
#[derive(Clone, Debug)]
struct EdgeCut {
    a: DVec3,
    b: DVec3,
    /// Unit direction a to b.
    u: DVec3,
    /// Outward normals of the two faces.
    n1: DVec3,
    n2: DVec3,
    /// Unit directions into each face from the edge, perpendicular to it.
    d1: DVec3,
    d2: DVec3,
    /// The solid's angle at the edge is under 180 degrees: material is removed.
    convex: bool,
    ends: [End; 2],
}

/// The solid `m` (its own vertices, edges and faces) with `edges` bevelled by `profile`: the
/// cutters of convex edges subtracted and the fillers of concave edges added, all from the
/// original faces. The result faces outward.
pub fn bevelled(m: &Model, edges: &[EdgeId], profile: Profile) -> Result<Model, BevelError> {
    let size = profile.size();
    let noun = profile.noun();
    if !(size.is_finite() && size > MIN_SIZE) {
        return Err(BevelError::Invalid(format!(
            "{noun} must be more than {MIN_SIZE} m"
        )));
    }
    if edges.len() > MAX_EDGES {
        return Err(BevelError::Invalid(format!(
            "at most {MAX_EDGES} edges can be bevelled at once"
        )));
    }
    if let Some(why) = open_reason(m) {
        return Err(BevelError::NotSolid(why));
    }
    let volume = signed_volume(m);
    if volume.abs() <= 1e-12 {
        return Err(BevelError::NotSolid("it encloses no volume".into()));
    }
    let outward = volume.signum();
    let edges: Vec<EdgeId> = edges
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut ends: HashMap<VertexId, usize> = HashMap::new();
    for e in &edges {
        let edge = m
            .edge(*e)
            .ok_or_else(|| BevelError::Edge(*e, format!("e{} is not an edge of the solid", e.0)))?;
        for v in edge.v {
            *ends.entry(v).or_default() += 1;
        }
    }
    let mut cuts = Vec::new();
    for e in &edges {
        let cut = analyze(m, *e, outward, size, noun, &|v| ends[&v] > 1)
            .map_err(|why| BevelError::Edge(*e, why))?;
        let prism = prism(&cut, &profile.section(&cut))
            .ok_or_else(|| BevelError::Edge(*e, format!("{noun} too large for edge e{}", e.0)))?;
        cuts.push((cut.convex, prism));
    }

    let mut acc = placed_copy(m, DAffine3::IDENTITY, None);
    let (cutters, fillers): (Vec<_>, Vec<_>) = cuts.into_iter().partition(|(c, _)| *c);
    for (op, tools) in [(BooleanOp::Union, fillers), (BooleanOp::Subtract, cutters)] {
        for tool in apart(tools.into_iter().map(|(_, p)| p).collect()) {
            acc = combine(&acc, &tool, op)?.ok_or(BooleanError::NotClosed)?;
        }
    }
    Ok(acc)
}

/// The geometry of edge `id` for a cut reaching `size` into its faces, or why it cannot be
/// bevelled. `outward` is 1 when the faces of `m` point out, -1 when in; `shared` says
/// whether a vertex ends another chosen edge too.
fn analyze(
    m: &Model,
    id: EdgeId,
    outward: f64,
    size: f64,
    noun: &str,
    shared: &dyn Fn(VertexId) -> bool,
) -> Result<EdgeCut, String> {
    let name = format!("edge e{}", id.0);
    let edge = m.edge(id).expect("checked");
    if edge.curve.is_some() && !edge.polygon {
        return Err(format!("{name} is not straight: it is part of a curve"));
    }
    let [va, vb] = edge.v;
    let (a, b) = (m.pos(va), m.pos(vb));
    let u = (b - a).normalize();
    let faces = m.faces_on_side(va, vb);
    let [f1, f2] = faces[..] else {
        return Err(format!("{name} does not bound exactly two faces"));
    };
    for f in [f1, f2] {
        if m.face_edges(f)
            .iter()
            .any(|e| m.edge(*e).is_some_and(|e| e.soft))
        {
            return Err(format!(
                "{name} is not between two flat faces: f{} is part of a curved surface",
                f.0
            ));
        }
    }
    let (face1, face2) = (m.face(f1).expect("face"), m.face(f2).expect("face"));
    let (n1, n2) = (face1.normal * outward, face2.normal * outward);
    if n1.dot(n2) > MIN_ANGLE_DEG.to_radians().cos() {
        return Err(format!(
            "{name} lies between tangent faces (less than {MIN_ANGLE_DEG} degree apart)"
        ));
    }
    // Into a face from its loop side: to the left of the side's direction, seen from the
    // face's own normal (for an outer loop and for a hole alike).
    let into = |face: &Face| -> DVec3 {
        let forward = std::iter::once(&face.outer)
            .chain(&face.holes)
            .any(|l| (0..l.len()).any(|i| l[i] == va && l[(i + 1) % l.len()] == vb));
        let dir = if forward { u } else { -u };
        face.normal.cross(dir).normalize()
    };
    let (d1, d2) = (into(face1), into(face2));
    let convex = d2.dot(n1) < 0.0;
    let too_large = || format!("{noun} too large for {name}");
    let length = a.distance(b);
    for (face, d) in [(face1, d1), (face2, d2)] {
        if !strip_clear(m, face, (va, vb), a, u, d, length, size) {
            return Err(too_large());
        }
    }
    let reach = 2.0 * size * (2.0 + (n1 + n2).length());
    let mut ends = [End::Extend(reach); 2];
    for (k, (v, p)) in [(va, a), (vb, b)].into_iter().enumerate() {
        if convex && shared(v) {
            continue;
        }
        let others: Vec<FaceId> = m.faces_at(v).filter(|f| *f != f1 && *f != f2).collect();
        let Some(first) = others.first() else {
            return Err(format!("{name} has no face at its end v{}", v.0));
        };
        let n3 = m.face(*first).expect("face").normal;
        let p3 = m.pos(m.face(*first).expect("face").outer[0]);
        let flat = others.iter().all(|f| {
            let g = m.face(*f).expect("face");
            g.normal.dot(n3).abs() > 1.0 - 1e-9
                && g.outer
                    .iter()
                    .all(|w| (m.pos(*w) - p3).dot(n3).abs() < LENGTH_TOL)
        });
        if !flat {
            return Err(format!(
                "{name} ends at v{}, where the other faces do not lie in one plane",
                v.0
            ));
        }
        if n3.dot(u).abs() < 1e-3 {
            return Err(format!(
                "{name} meets f{} at too shallow an angle at its end",
                first.0
            ));
        }
        ends[k] = End::Plane(p, n3);
    }
    Ok(EdgeCut {
        a,
        b,
        u,
        n1,
        n2,
        d1,
        d2,
        convex,
        ends,
    })
}

/// Whether the strip of `face` beside its edge `ends` (from `a` along `u` for `length`, `size`
/// deep along `d`) holds no vertex and no loop side of the face other than the sides at the
/// edge's ends (which the prism's end planes follow).
#[allow(clippy::too_many_arguments)]
fn strip_clear(
    m: &Model,
    face: &Face,
    ends: (VertexId, VertexId),
    a: DVec3,
    u: DVec3,
    d: DVec3,
    length: f64,
    size: f64,
) -> bool {
    let tol = LENGTH_TOL;
    let lo = DVec2::new(tol, tol);
    let hi = DVec2::new(length - tol, size + tol);
    let at = |v: VertexId| {
        let p = m.pos(v) - a;
        DVec2::new(p.dot(u), p.dot(d))
    };
    let inside = |p: DVec2| p.cmpge(lo).all() && p.cmple(hi).all();
    let touches = |v: VertexId| v == ends.0 || v == ends.1;
    for l in std::iter::once(&face.outer).chain(&face.holes) {
        for i in 0..l.len() {
            let (v, w) = (l[i], l[(i + 1) % l.len()]);
            if !touches(v) && inside(at(v)) {
                return false;
            }
            if !touches(v) && !touches(w) && crosses_box(at(v), at(w), lo, hi) {
                return false;
            }
        }
    }
    true
}

/// Whether the segment p-q meets the box `lo`..`hi` (Liang-Barsky clipping).
fn crosses_box(p: DVec2, q: DVec2, lo: DVec2, hi: DVec2) -> bool {
    if lo.cmpgt(hi).any() {
        return false;
    }
    let d = q - p;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (dk, room) in [
        (-d.x, p.x - lo.x),
        (d.x, hi.x - p.x),
        (-d.y, p.y - lo.y),
        (d.y, hi.y - p.y),
    ] {
        if dk == 0.0 {
            if room < 0.0 {
                return false;
            }
        } else {
            let r = room / dk;
            if dk < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    t0 <= t1
}

/// The closed, outward prism with cross-section `section` (offsets from the edge, in order
/// around it) running along `cut` between its two ends. None when the ends cross (the edge
/// is too short for the section).
fn prism(cut: &EdgeCut, section: &[DVec3]) -> Option<Model> {
    let cap = |end: End, p: DVec3, sign: f64| -> Vec<DVec3> {
        section
            .iter()
            .map(|q| match end {
                End::Extend(by) => p + *q + cut.u * by * sign,
                End::Plane(o, n) => {
                    let start = p + *q;
                    start - cut.u * ((start - o).dot(n) / cut.u.dot(n))
                }
            })
            .collect()
    };
    let start = cap(cut.ends[0], cut.a, -1.0);
    let end = cap(cut.ends[1], cut.b, 1.0);
    if start
        .iter()
        .zip(&end)
        .any(|(s, e)| (*e - *s).dot(cut.u) < 10.0 * LENGTH_TOL)
    {
        return None;
    }
    let mut m = Model::new();
    let s: Vec<VertexId> = start.iter().map(|p| m.insert_vertex(*p)).collect();
    let e: Vec<VertexId> = end.iter().map(|p| m.insert_vertex(*p)).collect();
    let n = section.len();
    let mut loops: Vec<Vec<VertexId>> = vec![s.iter().rev().copied().collect(), e.clone()];
    for i in 0..n {
        let j = (i + 1) % n;
        loops.push(vec![s[i], s[j], e[j], e[i]]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        m.insert_edge(s[i], s[j]);
        m.insert_edge(e[i], e[j]);
        m.insert_edge(s[i], e[i]);
    }
    for l in loops {
        let points: Vec<DVec3> = l.iter().map(|v| m.pos(*v)).collect();
        let normal = vector_area(&points).normalize_or_zero();
        m.insert_face(Face::new(l, Vec::new(), normal));
    }
    if signed_volume(&m) < 0.0 {
        let all: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
        m.reverse_faces(&all);
    }
    m.take_changes();
    Some(m)
}

/// Replaces the vertices, edges and faces of `m` by those of `r`, keeping the ids of faces
/// and edges `r` has unchanged (the same loops of points, the same ends) and the curves of
/// kept edges. Instances, guides and annotations stay.
fn replace_geometry(m: &mut Model, r: &Model) {
    let vertices: BTreeMap<VertexId, VertexId> = r
        .vertices()
        .map(|(id, v)| (id, v.pos))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(id, p)| (id, m.find_vertex(p).unwrap_or_else(|| m.insert_vertex(p))))
        .collect();
    let loops_of = |f: &Face| -> Vec<Vec<VertexId>> {
        std::iter::once(&f.outer).chain(&f.holes).cloned().collect()
    };
    let mut have: HashMap<Vec<Vec<VertexId>>, FaceId> = m
        .faces()
        .map(|(id, f)| (face_key(&loops_of(f)), id))
        .collect();
    let mut added: Vec<Face> = Vec::new();
    for (_, f) in r.faces() {
        let loops: Vec<Vec<VertexId>> = loops_of(f)
            .iter()
            .map(|l| l.iter().map(|v| vertices[v]).collect())
            .collect();
        if have.remove(&face_key(&loops)).is_none() {
            let mut loops = loops.into_iter();
            added.push(Face {
                outer: loops.next().expect("outer loop"),
                holes: loops.collect(),
                ..f.clone()
            });
        }
    }
    let wanted: BTreeMap<(VertexId, VertexId), _> = r
        .edges()
        .map(|(_, e)| {
            let (a, b) = (vertices[&e.v[0]], vertices[&e.v[1]]);
            ((a.min(b), a.max(b)), e)
        })
        .collect();
    let mut curves: HashMap<CurveId, CurveId> = HashMap::new();
    let mut dropped = Vec::new();
    for (id, e) in m.edges() {
        let [a, b] = e.v;
        match wanted.get(&(a.min(b), a.max(b))) {
            Some(like) => {
                if let (Some(c), Some(own)) = (like.curve, e.curve) {
                    curves.entry(c).or_insert(own);
                }
            }
            None => dropped.push(id),
        }
    }
    let mut gone: Vec<FaceId> = have.into_values().collect();
    gone.sort();
    for f in gone {
        m.remove_face(f);
    }
    let mut touched: BTreeSet<VertexId> = BTreeSet::new();
    for e in dropped {
        touched.extend(m.remove_edge(e).v);
    }
    for (&(a, b), like) in &wanted {
        if m.edge_between(a, b).is_none() {
            let mut like = (*like).clone();
            like.curve = like
                .curve
                .map(|c| *curves.entry(c).or_insert_with(|| m.new_curve()));
            m.insert_edge_like(a, b, &like);
        }
    }
    for f in added {
        m.insert_face(f);
    }
    for v in touched {
        if m.edges_at(v).is_empty() && m.faces_at(v).next().is_none() {
            m.remove_vertex(v);
        }
    }
}

/// A face's loops by vertex, each started at its smallest id, holes sorted: equal for the
/// same face whichever vertex its loops were listed from.
fn face_key(loops: &[Vec<VertexId>]) -> Vec<Vec<VertexId>> {
    let turn = |l: &Vec<VertexId>| {
        let at = (0..l.len()).min_by_key(|i| l[*i]).unwrap_or(0);
        l[at..].iter().chain(&l[..at]).copied().collect::<Vec<_>>()
    };
    let mut holes: Vec<Vec<VertexId>> = loops[1..].iter().map(turn).collect();
    holes.sort();
    std::iter::once(turn(&loops[0])).chain(holes).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Solid;
    use crate::scene::DefinitionKind;

    fn cuboid(size: [f64; 3]) -> Model {
        Solid::cuboid(DVec3::from(size)).unwrap().to_model()
    }

    fn edge_at(m: &Model, p: [f64; 3], q: [f64; 3]) -> EdgeId {
        let (p, q) = (DVec3::from(p), DVec3::from(q));
        m.edges()
            .find(|(_, e)| {
                let (a, b) = (m.pos(e.v[0]), m.pos(e.v[1]));
                (a.distance(p) < 1e-9 && b.distance(q) < 1e-9)
                    || (a.distance(q) < 1e-9 && b.distance(p) < 1e-9)
            })
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no edge {p} {q}"))
    }

    fn has_vertex(m: &Model, p: [f64; 3]) -> bool {
        m.vertices()
            .any(|(_, v)| v.pos.distance(DVec3::from(p)) < 1e-9)
    }

    fn chamfer(distance: f64) -> Profile {
        Profile::Chamfer { distance }
    }

    fn closed(m: &Model) {
        assert_eq!(open_reason(m), None);
        m.validate().unwrap();
    }

    #[test]
    fn one_box_edge_loses_a_triangular_prism() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        let r = bevelled(&m, &[e], chamfer(0.5)).unwrap();
        closed(&r);
        assert!((signed_volume(&r) - (24.0 - 0.125 * 2.0)).abs() < 1e-9);
        assert_eq!(r.faces().count(), 7);
        for p in [
            [0.0, 0.5, 4.0],
            [0.0, 0.0, 3.5],
            [2.0, 0.5, 4.0],
            [2.0, 0.0, 3.5],
        ] {
            assert!(has_vertex(&r, p), "{p:?}");
        }
        assert!(!has_vertex(&r, [0.0, 0.0, 4.0]));
    }

    #[test]
    fn three_edges_at_a_corner_close_the_corner() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let edges = [
            edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 4.0], [0.0, 3.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 0.0], [0.0, 0.0, 4.0]),
        ];
        let r = bevelled(&m, &edges, chamfer(0.5)).unwrap();
        closed(&r);
        let v = signed_volume(&r);
        // Each prism alone removes d^2/2 per meter; they overlap only within the corner cube.
        let all = 0.125 * (2.0 + 3.0 + 4.0);
        let apart = 0.125 * (1.5 + 2.5 + 3.5);
        assert!(v > 24.0 - all && v < 24.0 - apart, "{v}");
        assert!(!has_vertex(&r, [0.0, 0.0, 4.0]));
    }

    #[test]
    fn all_twelve_edges_of_a_box_stay_a_solid() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
        let r = bevelled(&m, &edges, chamfer(0.25)).unwrap();
        closed(&r);
        let v = signed_volume(&r);
        // Four edges of each length; prisms overlap only within the corner cubes.
        let per = 0.25 * 0.25 / 2.0;
        let all = per * 4.0 * (2.0 + 3.0 + 4.0);
        let apart = per * 4.0 * (1.5 + 2.5 + 3.5);
        assert!(v > 24.0 - all && v < 24.0 - apart, "{v}");
        // 6 shrunken faces and 12 chamfers; at each corner three chamfers meet in a point.
        assert_eq!(r.faces().count(), 18);
    }

    fn l_shape() -> Model {
        let a = cuboid([4.0, 2.0, 1.0]);
        let b = cuboid([1.0, 2.0, 3.0]);
        combine(&a, &b, BooleanOp::Union).unwrap().unwrap()
    }

    #[test]
    fn a_concave_edge_gains_a_triangular_prism() {
        let m = l_shape();
        let before = signed_volume(&m);
        let e = edge_at(&m, [1.0, 0.0, 1.0], [1.0, 2.0, 1.0]);
        let r = bevelled(&m, &[e], chamfer(0.25)).unwrap();
        closed(&r);
        assert!((signed_volume(&r) - (before + 0.03125 * 2.0)).abs() < 1e-9);
        assert!(has_vertex(&r, [1.25, 0.0, 1.0]));
        assert!(has_vertex(&r, [1.0, 2.0, 1.25]));
        assert!(!has_vertex(&r, [1.0, 0.0, 1.0]));
    }

    #[test]
    fn slanted_end_faces_are_not_notched() {
        // A triangular prism: each top edge ends at side faces 60 degrees off square.
        let m = Solid::prism(1.0, 2.0, 3, 0.0).unwrap().to_model();
        let top: Vec<EdgeId> = m
            .edges()
            .filter(|(_, e)| m.pos(e.v[0]).z > 1.0 && m.pos(e.v[1]).z > 1.0)
            .map(|(id, _)| id)
            .collect();
        let r = bevelled(&m, &top[..1], chamfer(0.1)).unwrap();
        closed(&r);
        assert!(signed_volume(&r) < signed_volume(&m));
        // The two corners become four; nothing else is cut.
        assert_eq!(r.vertices().count(), 8);
        assert_eq!(r.faces().count(), 6);
    }

    #[test]
    fn an_inward_solid_is_bevelled_outward() {
        let mut m = cuboid([2.0, 3.0, 4.0]);
        let all: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
        m.reverse_faces(&all);
        m.take_changes();
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        let r = bevelled(&m, &[e], chamfer(0.5)).unwrap();
        closed(&r);
        assert!((signed_volume(&r) - 23.75).abs() < 1e-9);
    }

    fn refused(m: &Model, e: EdgeId, profile: Profile, text: &str) {
        match bevelled(m, &[e], profile) {
            Err(BevelError::Edge(id, why)) => {
                assert_eq!(id, e);
                assert!(why.contains(text), "{why}");
                assert!(why.contains(&format!("e{}", e.0)), "{why}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_curve_edge_is_not_straight() {
        let m = Solid::cylinder(1.0, 2.0, 12).unwrap().to_model();
        let e = m
            .edges()
            .find(|(_, e)| e.curve.is_some())
            .map(|(id, _)| id)
            .unwrap();
        refused(&m, e, chamfer(0.1), "not straight");
    }

    #[test]
    fn a_face_on_a_curved_surface_is_refused() {
        let m = Solid::cylinder(1.0, 2.0, 12).unwrap().to_model();
        let e = m.edges().find(|(_, e)| e.soft).map(|(id, _)| id).unwrap();
        refused(&m, e, chamfer(0.1), "curved surface");
    }

    #[test]
    fn a_seam_between_coplanar_faces_is_tangent() {
        let mut m = cuboid([2.0, 3.0, 4.0]);
        m.add_edge(DVec3::new(1.0, 0.0, 4.0), DVec3::new(1.0, 3.0, 4.0))
            .unwrap();
        closed(&m);
        let e = edge_at(&m, [1.0, 0.0, 4.0], [1.0, 3.0, 4.0]);
        refused(&m, e, chamfer(0.1), "tangent");
    }

    #[test]
    fn a_distance_past_a_face_is_too_large() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        // The top face is 3 deep, the front 4 high.
        refused(&m, e, chamfer(3.0), "distance too large for edge");
        assert!(bevelled(&m, &[e], chamfer(2.9)).is_ok());
    }

    #[test]
    fn a_non_positive_distance_and_an_open_mesh_are_refused() {
        let mut m = cuboid([2.0, 3.0, 4.0]);
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        assert!(matches!(
            bevelled(&m, &[e], chamfer(0.0)),
            Err(BevelError::Invalid(_))
        ));
        let f = m.faces().next().unwrap().0;
        m.erase(&[], &[f]);
        assert!(matches!(
            bevelled(&m, &[e], chamfer(0.1)),
            Err(BevelError::NotSolid(_))
        ));
    }

    #[test]
    fn untouched_faces_and_edges_keep_their_ids_in_the_scene() {
        let mut scene = Scene::new();
        let (i, _) = scene
            .add_solid(
                MeshId::Root,
                &Solid::cuboid(DVec3::new(2.0, 3.0, 4.0)).unwrap(),
                DefinitionKind::Group,
                "B",
                DAffine3::IDENTITY,
                None,
            )
            .unwrap();
        let def = scene
            .mesh(MeshId::Root)
            .unwrap()
            .instance(i)
            .unwrap()
            .definition;
        let mesh = MeshId::Definition(def);
        let m = scene.mesh(mesh).unwrap().clone();
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        let far = edge_at(&m, [0.0, 3.0, 0.0], [2.0, 3.0, 0.0]);
        let change = scene.bevel_edges(mesh, &[e], chamfer(0.5)).unwrap();
        let after = scene.mesh(mesh).unwrap();
        closed(after);
        assert!(after.edge(far).is_some());
        assert!(after.edge(e).is_none());
        // The back, bottom faces keep their ids; top, front and the two ends change.
        let kept = m.faces().filter(|(f, _)| after.face(*f).is_some()).count();
        assert_eq!(kept, 2);
        scene.apply(&change.inverse());
        assert_eq!(*scene.mesh(mesh).unwrap(), m);
    }
}
