//! Chamfers and fillets of straight edges of a solid.
//!
//! Each chosen edge lies between two flat faces. Along it a prism is built whose cross-section
//! is the region between the two faces and the chamfer line or fillet arc, reaching a little
//! past both faces so the boolean cuts them cleanly: a convex edge's prism (a cutter) is
//! subtracted, a concave edge's (a filler) is added, with the boolean core of `boolean.rs`. At
//! an end shared with another chosen convex edge where running on leaves the solid, a cutter
//! runs on past the end (meeting cutters overlap and leave a closed corner); elsewhere the
//! prism ends in the plane of the face at that end, so no neighbouring face is notched. The
//! fillers' own regions are taken out of the cutters, so a chamfer never removes what a cove
//! beside it adds; a cove whose two faces are both cut away at an end stops short there. The
//! new faces take the first face's materials; the edges along a fillet (between its faces and
//! where it meets the flat faces) are soft, so it is shaded as one smooth round. Where three
//! fillets of convex edges meet at a vertex of three faces, `corner` trims the crossed rounds to
//! a sphere patch. The result replaces the mesh's own geometry; faces and edges the cut did not touch keep their ids.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use glam::{DAffine3, DVec2, DVec3};

use crate::boolean::{BooleanError, BooleanOp, apart, combine, placed_copy};
use crate::geom::{LENGTH_TOL, vector_area};
use crate::ids::{CurveId, EdgeId, FaceId, MaterialId, VertexId};
use crate::model::{Face, Model};
use crate::scene::{MeshId, Scene, SceneChange};
use crate::solid::{open_reason, signed_volume};

mod corner;

/// The smallest chamfer distance or fillet radius, in meters.
pub const MIN_SIZE: f64 = 1e-6;
/// The most edges one command bevels.
pub const MAX_EDGES: usize = 1000;
/// Faces whose normals are closer than this (degrees) are tangent: no edge to bevel.
pub const MIN_ANGLE_DEG: f64 = 1.0;
/// A fillet's faces per quarter turn by default, and the most allowed.
pub const DEFAULT_SEGMENTS: u32 = 8;
pub const MAX_SEGMENTS: u32 = 90;

/// The shape left where an edge was.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Profile {
    /// A flat face set back `distance` on both faces.
    Chamfer { distance: f64 },
    /// A round of `radius` tangent to both faces, made of `segments` faces per quarter turn
    /// (in proportion for other angles, at least 1).
    Fillet { radius: f64, segments: u32 },
}

impl Profile {
    /// The distance or radius.
    fn size(self) -> f64 {
        match self {
            Profile::Chamfer { distance } => distance,
            Profile::Fillet { radius, .. } => radius,
        }
    }

    /// The parameter's name in messages.
    fn noun(self) -> &'static str {
        match self {
            Profile::Chamfer { .. } => "distance",
            Profile::Fillet { .. } => "radius",
        }
    }

    /// How far the cut reaches into each face from an edge whose faces' inward directions are
    /// `angle` (radians) apart.
    fn setback(self, angle: f64) -> f64 {
        match self {
            Profile::Chamfer { distance } => distance,
            Profile::Fillet { radius, .. } => radius / (angle / 2.0).tan(),
        }
    }

    /// The new surface's cross-section, from the point on the first face to the point on the
    /// second, as offsets from a point on the edge: the chamfer line, or the fillet's arc
    /// (tangent to both faces) as a polyline with its corners on the arc.
    fn profile(self, cut: &EdgeCut) -> Vec<DVec3> {
        let s = cut.setback;
        let (p1, p2) = (cut.d1 * s, cut.d2 * s);
        match self {
            Profile::Chamfer { .. } => vec![p1, p2],
            Profile::Fillet { radius, segments } => {
                let angle = cut.d1.dot(cut.d2).clamp(-1.0, 1.0).acos();
                let centre = (cut.d1 + cut.d2).normalize() * (radius / (angle / 2.0).sin());
                let (w1, w2) = ((p1 - centre) / radius, (p2 - centre) / radius);
                let turn = std::f64::consts::PI - angle;
                let steps = arc_steps(segments, turn);
                let across = (w2 - w1 * turn.cos()).normalize();
                let mut points = vec![p1];
                for k in 1..steps {
                    let t = turn * k as f64 / steps as f64;
                    points.push(centre + (w1 * t.cos() + across * t.sin()) * radius);
                }
                points.push(p2);
                points
            }
        }
    }

    /// The region between the two faces and the profile alone, as offsets from a point on the
    /// edge, in order around it: what a filler adds or a cutter removes.
    fn core(self, cut: &EdgeCut) -> Vec<DVec3> {
        std::iter::once(DVec3::ZERO)
            .chain(self.profile(cut))
            .collect()
    }

    /// The prism's cross-section around `cut`, as offsets from a point on the edge (all
    /// perpendicular to it): the region between the faces and the profile, widened past both
    /// faces away from the material it does not change (outward for a cutter, inward for a
    /// filler) so none of its faces lies in a face of the solid. Its first sides (side i joins
    /// points i and i + 1), as many as the profile has, make the new surface.
    fn section(self, cut: &EdgeCut) -> Vec<DVec3> {
        let s = cut.setback;
        let reach = if cut.convex { s } else { -s };
        let mut points = self.profile(cut);
        let (p1, p2) = (points[0], points[points.len() - 1]);
        points.extend([
            p2 + cut.n2 * reach,
            (cut.n1 + cut.n2) * reach,
            p1 + cut.n1 * reach,
        ]);
        points
    }
}

/// The faces of a fillet of `segments` per quarter turn around `turn` radians: in
/// proportion, at least 1.
fn arc_steps(segments: u32, turn: f64) -> usize {
    ((segments as f64 * turn / std::f64::consts::FRAC_PI_2).round() as usize).max(1)
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
    /// These two edges' cuts would overlap on a face between them; the text names both.
    Edges([EdgeId; 2], String),
    /// The boolean failed.
    Boolean(BooleanError),
}

impl fmt::Display for BevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BevelError::NotSolid(why) => write!(f, "not a solid: {why}"),
            BevelError::Invalid(why) | BevelError::Edge(_, why) | BevelError::Edges(_, why) => {
                f.write_str(why)
            }
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
    va: VertexId,
    vb: VertexId,
    f1: FaceId,
    f2: FaceId,
    /// The materials of the first face's outer and inner sides, for the new surface.
    paint: [Option<MaterialId>; 2],
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
    /// How far the cut reaches into each face.
    setback: f64,
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
    if let Profile::Fillet { segments, .. } = profile
        && !(1..=MAX_SEGMENTS).contains(&segments)
    {
        return Err(BevelError::Invalid(format!(
            "segments must be 1 to {MAX_SEGMENTS}"
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
    let mut cuts = Vec::new();
    // How many chosen convex edges end at each vertex, and the faces beside them.
    let mut convex_ends: HashMap<VertexId, usize> = HashMap::new();
    let mut cut_faces: HashMap<VertexId, Vec<FaceId>> = HashMap::new();
    for e in &edges {
        if m.edge(*e).is_none() {
            return Err(BevelError::Edge(
                *e,
                format!("e{} is not an edge of the solid", e.0),
            ));
        }
        let cut = analyze(m, *e, outward, profile).map_err(|why| BevelError::Edge(*e, why))?;
        if cut.convex {
            for v in [cut.va, cut.vb] {
                *convex_ends.entry(v).or_default() += 1;
                cut_faces.entry(v).or_default().extend([cut.f1, cut.f2]);
            }
        }
        cuts.push((*e, cut));
    }
    if let Some([x, y]) = crossing_strips(&cuts) {
        return Err(BevelError::Edges(
            [x, y],
            format!("{noun} too large for edges e{} and e{}", x.0, y.0),
        ));
    }
    let mut cutters = Vec::new();
    let mut fillers = Vec::new();
    let mut cores = Vec::new();
    let mut rounds = Vec::new();
    let mut closed_cuts = Vec::new();
    for (e, mut cut) in cuts {
        let at = Corner {
            run_on: &|v| convex_ends.get(&v).copied().unwrap_or(0) > 1,
            cut: &|v, f| cut_faces.get(&v).is_some_and(|fs| fs.contains(&f)),
        };
        close_ends(m, &mut cut, e, outward, &at).map_err(|why| BevelError::Edge(e, why))?;
        let too_large = || BevelError::Edge(e, format!("{noun} too large for edge e{}", e.0));
        closed_cuts.push((e, cut.clone()));
        let sides = profile.profile(&cut).len() - 1;
        let tool = prism(&cut, &profile.section(&cut), 0..sides).ok_or_else(too_large)?;
        if let Profile::Fillet { .. } = profile {
            rounds.push(Round::of(&cut, &tool, &profile.profile(&cut)));
        }
        if cut.convex {
            cutters.push(tool);
        } else {
            cores.push(prism(&cut, &profile.core(&cut), 0..0).ok_or_else(too_large)?);
            fillers.push(tool);
        }
    }
    // A cutter reaching past its faces must not take away what a filler beside it adds (a
    // chamfered edge meeting a chamfered inside corner): the fillers' own regions are taken out
    // of the cutters first.
    let mut trimmed = Vec::new();
    for mut cutter in cutters {
        let (lo, hi) = bounds(&cutter);
        for core in &cores {
            let (clo, chi) = bounds(core);
            if clo.cmple(hi).all() && chi.cmpge(lo).all() {
                match combine(&cutter, core, BooleanOp::Subtract)? {
                    Some(rest) => cutter = rest,
                    None => break,
                }
            }
        }
        trimmed.push(cutter);
    }

    let mut acc = placed_copy(m, DAffine3::IDENTITY, None);
    for (op, tools) in [(BooleanOp::Union, fillers), (BooleanOp::Subtract, trimmed)] {
        for tool in apart(tools) {
            acc = combine(&acc, &tool, op)?.ok_or(BooleanError::NotClosed)?;
        }
    }
    let corners = corner::patches(m, &closed_cuts, profile);
    let (mut acc, done) = corner::apply(acc, &corners);
    soften_rounds(&mut acc, &rounds);
    if let Profile::Fillet { segments, .. } = profile {
        let step = std::f64::consts::FRAC_PI_2 / segments as f64;
        corner::soften(&mut acc, &corners, &done, step);
    }
    Ok(acc)
}

/// Where a fillet's surface lies, to soften the edges along it.
struct Round {
    /// A point on the edge and the edge's direction.
    a: DVec3,
    u: DVec3,
    /// The profile's corners, as offsets from `a`: the lines through them along `u` hold
    /// the fillet's edges.
    corners: Vec<DVec3>,
    /// The prism's extent along `u` from `a`.
    span: (f64, f64),
    /// The largest angle (radians) between two faces meeting along the fillet: one step of
    /// the arc.
    step: f64,
    /// The prism's bounds.
    bounds: (DVec3, DVec3),
}

impl Round {
    fn of(cut: &EdgeCut, tool: &Model, profile: &[DVec3]) -> Round {
        let (lo, hi) =
            tool.vertices()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, v)| {
                    let t = (v.pos - cut.a).dot(cut.u);
                    (lo.min(t), hi.max(t))
                });
        let turn = cut.n1.angle_between(cut.n2);
        Round {
            a: cut.a,
            u: cut.u,
            corners: profile.to_vec(),
            span: (lo, hi),
            step: turn / (profile.len() - 1) as f64,
            bounds: bounds(tool),
        }
    }

    /// Whether the segment p-q lies along the fillet, on one of its lines.
    fn holds(&self, p: DVec3, q: DVec3) -> bool {
        let (lo, hi) = self.bounds;
        let inside = |x: DVec3| x.cmpge(lo).all() && x.cmple(hi).all();
        if !(inside(p) && inside(q)) {
            return false;
        }
        let on = |x: DVec3, c: DVec3| {
            let r = x - self.a;
            let t = r.dot(self.u);
            t >= self.span.0 - LENGTH_TOL
                && t <= self.span.1 + LENGTH_TOL
                && (r - self.u * t - c).length() < LENGTH_TOL
        };
        self.corners.iter().any(|c| on(p, *c) && on(q, *c))
    }
}

/// Softens the edges of `m` along the fillets `rounds` (between two of a fillet's faces, or
/// where a fillet meets a flat face tangent to it), so each reads as one smooth round.
fn soften_rounds(m: &mut Model, rounds: &[Round]) {
    if rounds.is_empty() {
        return;
    }
    let mut soft = Vec::new();
    for (id, e) in m.edges() {
        if e.soft {
            continue;
        }
        let [a, b] = e.v;
        let (p, q) = (m.pos(a), m.pos(b));
        let [f, g] = m.faces_on_side(a, b)[..] else {
            continue;
        };
        let (nf, ng) = (
            m.face(f).expect("face").normal,
            m.face(g).expect("face").normal,
        );
        let angle = nf.angle_between(ng);
        if angle > 1e-9
            && rounds
                .iter()
                .any(|r| angle <= r.step + 1e-6 && r.holds(p, q))
        {
            soft.push((id, e.curve, e.polygon));
        }
    }
    for (id, curve, polygon) in soft {
        m.set_edge(id, curve, true, polygon);
    }
    m.take_changes();
}

/// The bounding box of `m`'s vertices, grown by `LENGTH_TOL`.
fn bounds(m: &Model) -> (DVec3, DVec3) {
    let pad = DVec3::splat(LENGTH_TOL);
    m.vertices().fold(
        (DVec3::INFINITY, DVec3::NEG_INFINITY),
        |(lo, hi), (_, v)| (lo.min(v.pos - pad), hi.max(v.pos + pad)),
    )
}

/// The geometry of edge `id` for a cut by `profile`, or why it cannot be
/// bevelled; its ends are left to `close_ends`. `outward` is 1 when the faces of `m` point
/// out, -1 when in.
fn analyze(m: &Model, id: EdgeId, outward: f64, profile: Profile) -> Result<EdgeCut, String> {
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
    // The seams of a curved surface are soft and its rims are curves: neither is bevelled.
    // A flat face beside a soft edge (one next to an earlier fillet) is fine.
    if edge.soft {
        return Err(format!("{name} is soft: it lies within a curved surface"));
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
    let size = profile.setback(d1.angle_between(d2));
    let noun = profile.noun();
    let too_large = || format!("{noun} too large for {name}");
    if !size.is_finite() {
        return Err(too_large());
    }
    let length = a.distance(b);
    for (face, d) in [(face1, d1), (face2, d2)] {
        if !strip_clear(m, face, (va, vb), a, u, d, length, size) {
            return Err(too_large());
        }
    }
    let reach = 2.0 * size * (2.0 + (n1 + n2).length());
    let sides = |f: &Face| {
        if outward > 0.0 {
            [f.front, f.back]
        } else {
            [f.back, f.front]
        }
    };
    Ok(EdgeCut {
        va,
        vb,
        f1,
        f2,
        paint: sides(face1),
        a,
        b,
        u,
        n1,
        n2,
        d1,
        d2,
        convex,
        setback: size,
        ends: [End::Extend(reach); 2],
    })
}

/// A chosen edge's strip on one face: the edge, its ends, its direction into the face, and
/// the strip's corners.
type Strip = (EdgeId, [VertexId; 2], DVec3, [DVec3; 4]);

/// The first two chosen edges (by id) that share no vertex but whose strips on a face they
/// both bound overlap, facing each other (their directions into the face more than 90 degrees
/// apart): the two cuts would meet within the face and leave no tangent strip of it between
/// them. Strips of edges turning less than that (the sides of an earlier round, the edges of a
/// box face) overlap only where the cuts' corner closes.
fn crossing_strips(cuts: &[(EdgeId, EdgeCut)]) -> Option<[EdgeId; 2]> {
    let mut strips: BTreeMap<FaceId, Vec<Strip>> = BTreeMap::new();
    for (e, cut) in cuts {
        for (f, d) in [(cut.f1, cut.d1), (cut.f2, cut.d2)] {
            let deep = d * cut.setback;
            let corners = [cut.a, cut.b, cut.b + deep, cut.a + deep];
            strips
                .entry(f)
                .or_default()
                .push((*e, [cut.va, cut.vb], d, corners));
        }
    }
    let mut found: Option<[EdgeId; 2]> = None;
    for list in strips.values() {
        for (i, (e, ends, d, corners)) in list.iter().enumerate() {
            for (g, other_ends, other_d, other) in &list[i + 1..] {
                let touching = ends.iter().any(|v| other_ends.contains(v));
                if touching || d.dot(*other_d) >= -1e-9 {
                    continue;
                }
                if overlap(corners, other) {
                    let pair = [(*e).min(*g), (*e).max(*g)];
                    if found.is_none_or(|f| pair < f) {
                        found = Some(pair);
                    }
                }
            }
        }
    }
    found
}

/// Whether two rectangles in one plane overlap by more than `LENGTH_TOL` (separating axes:
/// their sides' directions).
fn overlap(p: &[DVec3; 4], q: &[DVec3; 4]) -> bool {
    let axes = [p[1] - p[0], p[3] - p[0], q[1] - q[0], q[3] - q[0]];
    axes.iter().all(|axis| {
        let axis = axis.normalize_or_zero();
        if axis == DVec3::ZERO {
            return true;
        }
        let span = |r: &[DVec3; 4]| {
            r.iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), c| {
                    let t = c.dot(axis);
                    (lo.min(t), hi.max(t))
                })
        };
        let ((plo, phi), (qlo, qhi)) = (span(p), span(q));
        phi.min(qhi) - plo.max(qlo) > LENGTH_TOL
    })
}

/// What the other chosen edges do at a vertex.
struct Corner<'a> {
    /// Another chosen convex edge ends there.
    run_on: &'a dyn Fn(VertexId) -> bool,
    /// A chosen convex edge ending there lies beside this face, cutting it there.
    cut: &'a dyn Fn(VertexId, FaceId) -> bool,
}

/// Decides how each end of `cut` (edge `id`) is closed: a convex edge runs on past an end
/// where another chosen convex edge ends too and running on leaves the solid (every other
/// face there faces along the run-on direction); any other end stops in the plane of the
/// other faces at its vertex, which must be one plane. A concave edge whose two faces are
/// both cut at an end by chosen convex edges stops its setback short of that plane: what it would
/// add there would rest on nothing but the corner.
fn close_ends(
    m: &Model,
    cut: &mut EdgeCut,
    id: EdgeId,
    outward: f64,
    at: &Corner<'_>,
) -> Result<(), String> {
    let name = format!("edge e{}", id.0);
    let size = cut.setback;
    let u = cut.u;
    for (k, (v, p, along)) in [(cut.va, cut.a, -u), (cut.vb, cut.b, u)]
        .into_iter()
        .enumerate()
    {
        let others: Vec<FaceId> = m
            .faces_at(v)
            .filter(|f| *f != cut.f1 && *f != cut.f2)
            .collect();
        let leaves = others.iter().all(|f| {
            let n = m.face(*f).expect("face").normal * outward;
            n.dot(along) > 1e-9
        });
        if cut.convex && (at.run_on)(v) && leaves {
            continue;
        }
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
        let unsupported = !cut.convex && (at.cut)(v, cut.f1) && (at.cut)(v, cut.f2);
        let p = if unsupported { p - along * size } else { p };
        cut.ends[k] = End::Plane(p, n3);
    }
    Ok(())
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
///
/// The sides in `surface` (side i joins section points i and i + 1) are painted with the
/// first face's outer material on the front and its inner one on the back: the boolean shows
/// a cutter's front material on the cut, and a filler's faces point out as they are.
fn prism(cut: &EdgeCut, section: &[DVec3], surface: std::ops::Range<usize>) -> Option<Model> {
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
    let mut loops: Vec<Vec<VertexId>> = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        loops.push(vec![s[i], s[j], e[j], e[i]]);
    }
    loops.push(s.iter().rev().copied().collect());
    loops.push(e.clone());
    for i in 0..n {
        let j = (i + 1) % n;
        m.insert_edge(s[i], s[j]);
        m.insert_edge(e[i], e[j]);
        m.insert_edge(s[i], e[i]);
    }
    let mut faces = Vec::new();
    for l in loops {
        let points: Vec<DVec3> = l.iter().map(|v| m.pos(*v)).collect();
        let normal = vector_area(&points).normalize_or_zero();
        faces.push(m.insert_face(Face::new(l, Vec::new(), normal)));
    }
    if signed_volume(&m) < 0.0 {
        m.reverse_faces(&faces);
    }
    let [outer, inner] = cut.paint;
    for i in surface {
        let mut f = m.face(faces[i]).expect("face").clone();
        f.front = outer;
        f.back = inner;
        m.replace_face(faces[i], f);
    }
    m.take_changes();
    Some(m)
}

/// Replaces the vertices, edges and faces of `m` by those of `r`, keeping the ids of faces
/// and edges `r` has unchanged (the same loops of points, the same ends) and the curves of
/// kept edges, whose softness follows `r`. Instances, guides and annotations stay.
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
    let mut softened = Vec::new();
    for (id, e) in m.edges() {
        let [a, b] = e.v;
        match wanted.get(&(a.min(b), a.max(b))) {
            Some(like) => {
                if let (Some(c), Some(own)) = (like.curve, e.curve) {
                    curves.entry(c).or_insert(own);
                }
                if like.soft != e.soft {
                    softened.push((id, e.curve, like.soft, e.polygon));
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
    for (e, curve, soft, polygon) in softened {
        m.set_edge(e, curve, soft, polygon);
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

    /// Every vertex of `r` is a corner of `m` or lies within `reach` of one of `edges`.
    fn local(m: &Model, r: &Model, edges: &[EdgeId], reach: f64) {
        let near = |p: DVec3| {
            edges.iter().any(|e| {
                let e = m.edge(*e).unwrap();
                let (a, b) = (m.pos(e.v[0]), m.pos(e.v[1]));
                let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                (a + (b - a) * t).distance(p) < reach
            })
        };
        for (_, v) in r.vertices() {
            let corner = m.vertices().any(|(_, w)| w.pos.distance(v.pos) < 1e-9);
            assert!(corner || near(v.pos), "stray vertex {}", v.pos);
        }
    }

    #[test]
    fn a_chamfer_meeting_a_concave_chamfer_cuts_no_groove() {
        let m = l_shape();
        let before = signed_volume(&m);
        let edges = [
            edge_at(&m, [1.0, 0.0, 1.0], [1.0, 2.0, 1.0]),
            edge_at(&m, [1.0, 0.0, 1.0], [4.0, 0.0, 1.0]),
        ];
        let r = bevelled(&m, &edges, chamfer(0.25)).unwrap();
        closed(&r);
        // The cove adds 0.03125 per meter over 2 m, the chamfer removes as much over 3 m.
        let v = signed_volume(&r) - before;
        assert!((v + 0.03125).abs() < 1e-9, "{v}");
        local(&m, &r, &edges, 0.3);
    }

    #[test]
    fn convex_chamfers_at_a_reflex_corner_stop_at_the_faces_there() {
        let m = l_shape();
        let before = signed_volume(&m);
        let edges = [
            edge_at(&m, [1.0, 0.0, 1.0], [4.0, 0.0, 1.0]),
            edge_at(&m, [1.0, 0.0, 1.0], [1.0, 0.0, 3.0]),
        ];
        let r = bevelled(&m, &edges, chamfer(0.25)).unwrap();
        closed(&r);
        let v = signed_volume(&r) - before;
        assert!((v + 0.03125 * 5.0).abs() < 1e-9, "{v}");
        local(&m, &r, &edges, 0.3);
    }

    #[test]
    fn a_cove_between_two_cut_faces_stops_short_of_the_corner() {
        let m = l_shape();
        let before = signed_volume(&m);
        let edges = [
            edge_at(&m, [1.0, 0.0, 1.0], [1.0, 2.0, 1.0]),
            edge_at(&m, [1.0, 0.0, 1.0], [4.0, 0.0, 1.0]),
            edge_at(&m, [1.0, 0.0, 1.0], [1.0, 0.0, 3.0]),
        ];
        let r = bevelled(&m, &edges, chamfer(0.25)).unwrap();
        closed(&r);
        // Chamfers of 3 m and 2 m; the cove starts 0.25 in from the front, where both of its
        // faces are cut away.
        let v = signed_volume(&r) - before;
        assert!((v - 0.03125 * (-5.0 + 1.75)).abs() < 1e-9, "{v}");
        local(&m, &r, &edges, 0.3);
    }

    #[test]
    fn all_edges_of_an_l_stay_a_solid_without_grooves() {
        let m = l_shape();
        let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
        let r = bevelled(&m, &edges, chamfer(0.1)).unwrap();
        closed(&r);
        local(&m, &r, &edges, 0.12);
        assert!(signed_volume(&r) < signed_volume(&m));
    }

    #[test]
    fn the_chamfer_face_takes_the_first_faces_paint() {
        let mut m = cuboid([2.0, 3.0, 4.0]);
        let all: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
        for f in all {
            let mut face = m.face(f).unwrap().clone();
            face.front = Some(MaterialId(7));
            face.back = Some(MaterialId(8));
            m.replace_face(f, face);
        }
        m.take_changes();
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        let r = bevelled(&m, &[e], chamfer(0.5)).unwrap();
        closed(&r);
        let slope = DVec3::new(0.0, -1.0, 1.0).normalize();
        let new = r
            .faces()
            .find(|(_, f)| f.normal.distance(slope) < 1e-9)
            .unwrap()
            .1;
        assert_eq!(new.paint(), [Some(MaterialId(7)), Some(MaterialId(8))]);
        assert!(r.faces().all(|(_, f)| f.front == Some(MaterialId(7))));
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

    fn fillet(radius: f64) -> Profile {
        Profile::Fillet {
            radius,
            segments: DEFAULT_SEGMENTS,
        }
    }

    /// What a quarter-turn fillet of `r` in `n` faces removes per meter of edge: the square of
    /// side `r` less the polygon inscribed in its quarter circle.
    fn quarter_loss(r: f64, n: usize) -> f64 {
        let step = std::f64::consts::FRAC_PI_2 / n as f64;
        r * r - n as f64 * r * r * step.sin() / 2.0
    }

    #[test]
    fn one_box_edge_loses_a_quarter_round() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        let r = bevelled(&m, &[e], fillet(0.5)).unwrap();
        closed(&r);
        let v = signed_volume(&r);
        assert!(
            (v - (24.0 - quarter_loss(0.5, 8) * 2.0)).abs() < 1e-9,
            "{v}"
        );
        // Within the faceting error of the true round.
        let round = 24.0 - (0.25 - std::f64::consts::PI * 0.25 / 4.0) * 2.0;
        assert!((v - round).abs() < 0.01, "{v}");
        // 6 faces (top and front narrowed) and 8 round ones; tangent at 0.5 on both faces.
        assert_eq!(r.faces().count(), 14);
        for p in [
            [0.0, 0.5, 4.0],
            [0.0, 0.0, 3.5],
            [2.0, 0.5, 4.0],
            [2.0, 0.0, 3.5],
        ] {
            assert!(has_vertex(&r, p), "{p:?}");
        }
        assert!(!has_vertex(&r, [0.0, 0.0, 4.0]));
        // The arc's corners lie on the circle about (y, z) = (0.5, 3.5).
        for (_, v) in r.vertices() {
            let (y, z) = (v.pos.y, v.pos.z);
            if y < 0.5 - 1e-9 && z > 3.5 + 1e-9 {
                let d = DVec2::new(y - 0.5, z - 3.5).length();
                assert!((d - 0.5).abs() < 1e-9, "{}", v.pos);
            }
        }
    }

    #[test]
    fn the_edges_along_a_fillet_are_soft() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        let r = bevelled(&m, &[e], fillet(0.5)).unwrap();
        // 7 edges between the 8 round faces and the 2 where it meets top and front.
        let soft: Vec<_> = r.edges().filter(|(_, e)| e.soft).collect();
        assert_eq!(soft.len(), 9);
        for (_, e) in &soft {
            let (a, b) = (r.pos(e.v[0]), r.pos(e.v[1]));
            assert!((b - a).normalize().dot(DVec3::X).abs() > 1.0 - 1e-12);
        }
        let tangent = r
            .edge(edge_at(&r, [0.0, 0.5, 4.0], [2.0, 0.5, 4.0]))
            .unwrap();
        assert!(tangent.soft);
        let tangent = r
            .edge(edge_at(&r, [0.0, 0.0, 3.5], [2.0, 0.0, 3.5]))
            .unwrap();
        assert!(tangent.soft);
        // The smooth normals turn across the round.
        assert!(!r.smooth_normals().is_empty());
        // The round's faces all face between front and top.
        let round = r
            .faces()
            .filter(|(_, f)| f.normal.x.abs() < 1e-9 && f.normal.y < -1e-9 && f.normal.z > 1e-9)
            .count();
        assert_eq!(round, 8);
    }

    #[test]
    fn segments_set_the_faces_of_a_quarter_turn_in_proportion() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        for (segments, faces) in [(1, 7), (3, 9), (12, 18)] {
            let r = bevelled(
                &m,
                &[e],
                Profile::Fillet {
                    radius: 0.5,
                    segments,
                },
            )
            .unwrap();
            closed(&r);
            assert_eq!(r.faces().count(), faces, "{segments}");
            let v = signed_volume(&r);
            assert!((v - (24.0 - quarter_loss(0.5, segments as usize) * 2.0)).abs() < 1e-9);
        }
        // A triangular prism's upright edges turn 120 degrees: 8 * 120 / 90, rounded, faces.
        assert_eq!(arc_steps(8, 120f64.to_radians()), 11);
        assert_eq!(arc_steps(1, 10f64.to_radians()), 1);
        let p = Solid::prism(1.0, 2.0, 3, 0.0).unwrap().to_model();
        let top = upright(&p);
        let r = bevelled(&p, &[top], fillet(0.05)).unwrap();
        closed(&r);
        assert_eq!(r.faces().count(), 5 + 11);
        assert_eq!(r.edges().filter(|(_, e)| e.soft).count(), 12);
    }

    /// An upright edge of a triangular prism, whose faces meet at 60 degrees inside.
    fn upright(p: &Model) -> EdgeId {
        p.edges()
            .find(|(_, e)| {
                let (a, b) = (p.pos(e.v[0]), p.pos(e.v[1]));
                (a - b).truncate().length() < 1e-9
            })
            .map(|(id, _)| id)
            .unwrap()
    }

    #[test]
    fn a_slanted_edge_is_tangent_at_its_setback() {
        // The arc touches each face r / tan(30 degrees) from the edge.
        let p = Solid::prism(1.0, 2.0, 3, 0.0).unwrap().to_model();
        let top = upright(&p);
        let e = p.edge(top).unwrap();
        let (a, b) = (p.pos(e.v[0]), p.pos(e.v[1]));
        let r = bevelled(&p, &[top], fillet(0.05)).unwrap();
        let setback = 0.05 / 30f64.to_radians().tan();
        // The two new corners at each end lie in the end face, setback from the edge.
        for end in [a, b] {
            let found = r
                .vertices()
                .filter(|(_, v)| {
                    (v.pos.z - end.z).abs() < 1e-9 && (v.pos.distance(end) - setback).abs() < 1e-9
                })
                .count();
            assert_eq!(found, 2, "{end}");
        }
    }

    #[test]
    fn a_radius_past_a_face_is_too_large() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        refused(&m, e, fillet(3.0), "radius too large for edge");
        // A 60 degree edge sets back further than the radius.
        let p = Solid::prism(1.0, 2.0, 3, 0.0).unwrap().to_model();
        let top = upright(&p);
        // The side is sqrt(3) wide: a setback of r * sqrt(3) fits for r just under 1.
        refused(&p, top, fillet(1.01), "radius too large");
        assert!(bevelled(&p, &[top], fillet(0.9)).is_ok());
        for segments in [0, 91] {
            assert!(matches!(
                bevelled(&m, &[e], Profile::Fillet { radius: 0.1, segments }),
                Err(BevelError::Invalid(why)) if why.contains("segments")
            ));
        }
        assert!(matches!(
            bevelled(&m, &[e], fillet(0.0)),
            Err(BevelError::Invalid(why)) if why.contains("radius")
        ));
    }

    #[test]
    fn all_twelve_edges_of_a_box_round_off_closed() {
        let (x, y, z, r) = (2.0, 3.0, 4.0, 0.25);
        let m = cuboid([x, y, z]);
        let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
        let out = bevelled(&m, &edges, fillet(r)).unwrap();
        closed(&out);
        let v = signed_volume(&out);
        // Each edge loses its faceted quarter round between the corner cubes; each corner
        // keeps an octant of the ball.
        let octant = std::f64::consts::PI * r * r * r / 6.0;
        let smooth = rounded_box_volume([x, y, z], r, 8, octant);
        assert!(
            v < smooth && smooth - v < 8.0 * octant * 0.03,
            "{smooth} {v}"
        );
        local(&m, &out, &edges, r * 1.01);
        // Each round's 9 lines are soft along its straight run between the corners, and each
        // patch's 84 inner sides and 24 boundary arcs.
        let soft = out.edges().filter(|(_, e)| e.soft).count();
        assert_eq!(soft, 12 * 9 + 8 * 108);
    }

    /// Vertices of `m` at distance `r` from `c` (within 1e-9).
    fn on_ball(m: &Model, c: DVec3, r: f64) -> Vec<DVec3> {
        m.vertices()
            .map(|(_, v)| v.pos)
            .filter(|p| (p.distance(c) - r).abs() < 1e-9)
            .collect()
    }

    /// The volume of a box of `size` with the 12 edges rounded (`n` faces to a quarter turn,
    /// the straight runs between the corner cubes) and a corner octant of `corner` in each cube.
    fn rounded_box_volume(size: [f64; 3], r: f64, n: usize, corner: f64) -> f64 {
        let [x, y, z] = size;
        x * y * z - quarter_loss(r, n) * 4.0 * (x + y + z - 6.0 * r) - 8.0 * (r * r * r - corner)
    }

    #[test]
    fn twelve_filleted_edges_of_a_box_end_in_sphere_octants() {
        let (size, r) = ([2.0, 3.0, 4.0], 0.25);
        let m = cuboid(size);
        let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
        let out = bevelled(&m, &edges, fillet(r)).unwrap();
        closed(&out);
        assert!(open_reason(&out).is_none());
        // Every corner: an octant of the ball about the point r in from the three faces, with
        // 8 faces to a quarter turn: 1 + 2 + .. + 8 vertices on the grid, 64 triangles.
        let corners = [0.0, 1.0].map(|a| [0.0, 1.0].map(|b| [0.0, 1.0].map(|c| (a, b, c))));
        let mut all = 0;
        for (a, b, c) in corners.into_iter().flatten().flatten() {
            let corner = DVec3::new(a * size[0], b * size[1], c * size[2]);
            let centre = DVec3::new(
                if a == 0.0 { r } else { size[0] - r },
                if b == 0.0 { r } else { size[1] - r },
                if c == 0.0 { r } else { size[2] - r },
            );
            let on = on_ball(&out, centre, r);
            // 9 * 10 / 2 grid points, every one at distance r.
            assert_eq!(on.len(), 45, "{corner}");
            all += on.len();
            let faces = out
                .faces()
                .filter(|(_, f)| {
                    f.outer
                        .iter()
                        .all(|v| (out.pos(*v).distance(centre) - r).abs() < 1e-9)
                })
                .count();
            assert_eq!(faces, 64, "{corner}");
        }
        assert_eq!(all, 8 * 45);
        // 6 flat faces, 12 * 8 round faces, 8 * 64 patch triangles.
        assert_eq!(out.faces().count(), 6 + 12 * 8 + 8 * 64);
        // Within the faceting of the smooth value.
        let v = signed_volume(&out);
        let octant = std::f64::consts::PI * r * r * r / 6.0;
        let smooth = rounded_box_volume(size, r, 8, octant);
        // The faceted octant holds a little less than the ball's, about 2 percent.
        assert!(
            v < smooth && smooth - v < 8.0 * octant * 0.03,
            "{v} {smooth}"
        );
        // Smooth shading: the patch and the fillets share soft edges, none are creases.
        let soft = out.edges().filter(|(_, e)| e.soft).count();
        // 12 rounds with 9 lines each, 8 patches with 64 triangles: 84 inner sides and 24 on
        // the boundary arcs.
        assert_eq!(soft, 12 * 9 + 8 * (84 + 24));
    }

    #[test]
    fn three_filleted_edges_at_one_corner_get_one_sphere() {
        let (size, r) = ([2.0, 3.0, 4.0], 0.5);
        let m = cuboid(size);
        let edges = [
            edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 4.0], [0.0, 3.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 0.0], [0.0, 0.0, 4.0]),
        ];
        let out = bevelled(&m, &edges, fillet(r)).unwrap();
        closed(&out);
        let centre = DVec3::new(r, r, 4.0 - r);
        let on = on_ball(&out, centre, r);
        assert_eq!(on.len(), 45);
        // The patch's corners are the fillets' tangent points on the faces.
        for p in [[0.0, r, 3.5], [r, 0.0, 3.5], [r, r, 4.0]] {
            assert!(has_vertex(&out, p), "{p:?}");
            assert!(on.iter().any(|q| q.distance(DVec3::from(p)) < 1e-9));
        }
        assert!(!has_vertex(&out, [0.0, 0.0, 4.0]));
        let v = signed_volume(&out);
        // Three rounds run between the corner cube and the far ends; the cube keeps an octant.
        let octant = std::f64::consts::PI * r * r * r / 6.0;
        let edges = quarter_loss(r, 8) * ((2.0 - r) + (3.0 - r) + (4.0 - r));
        let faceted = 24.0 - edges - (r * r * r - octant);
        assert!(v < faceted && faceted - v < octant * 0.03, "{v} {faceted}");
    }

    /// A box sheared along x by `shear` per unit height: faces are parallelograms.
    fn sheared_box(size: f64, shear: f64) -> Model {
        let mut m = Model::new();
        let mut v = Vec::new();
        for z in [0.0, 1.0] {
            for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                v.push(m.insert_vertex(DVec3::new(
                    x * size + z * size * shear,
                    y * size,
                    z * size,
                )));
            }
        }
        let loops = [
            vec![v[0], v[3], v[2], v[1]],
            vec![v[4], v[5], v[6], v[7]],
            vec![v[0], v[1], v[5], v[4]],
            vec![v[1], v[2], v[6], v[5]],
            vec![v[2], v[3], v[7], v[6]],
            vec![v[3], v[0], v[4], v[7]],
        ];
        let mut sides = BTreeSet::new();
        for l in &loops {
            for i in 0..4 {
                let (a, b) = (l[i], l[(i + 1) % 4]);
                if sides.insert((a.min(b), a.max(b))) {
                    m.insert_edge(a, b);
                }
            }
        }
        for l in loops {
            let pts: Vec<DVec3> = l.iter().map(|w| m.pos(*w)).collect();
            m.insert_face(Face::new(l, Vec::new(), vector_area(&pts).normalize()));
        }
        m.take_changes();
        assert!(signed_volume(&m) > 0.0);
        m
    }

    /// Checks the sphere patch about `centre`: its faces' planes lie within `sag` of the ball,
    /// and every edge between a patch face and any other face is soft. Gives the patch's
    /// face count.
    fn check_patch(m: &Model, centre: DVec3, r: f64, sag: f64) -> usize {
        let on = |f: &Face| {
            f.holes.is_empty()
                && f.outer
                    .iter()
                    .all(|w| (m.pos(*w).distance(centre) - r).abs() < 1e-9)
        };
        let patch: Vec<FaceId> = m.faces().filter(|(_, f)| on(f)).map(|(id, _)| id).collect();
        for id in &patch {
            let f = m.face(*id).unwrap();
            let depth = (m.pos(f.outer[0]) - centre).dot(f.normal);
            assert!(r - depth < sag * r, "sag {} of r", (r - depth) / r);
            for i in 0..f.outer.len() {
                let (a, b) = (f.outer[i], f.outer[(i + 1) % f.outer.len()]);
                let e = m.edge(m.edge_between(a, b).unwrap()).unwrap();
                assert!(e.soft, "hard edge {} {}", m.pos(a), m.pos(b));
            }
        }
        patch.len()
    }

    #[test]
    fn oblique_corners_get_sphere_patches_tangent_to_their_faces() {
        let (size, r) = (2.0, 0.15);
        let m = sheared_box(size, 0.1);
        closed(&m);
        let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
        let out = bevelled(&m, &edges, fillet(r)).unwrap();
        closed(&out);
        let mut patches = 0;
        for (_, corner) in m.vertices() {
            // The ball tangent to the three faces: centre r inside each.
            let faces: Vec<&Face> = m
                .faces()
                .filter(|(_, f)| f.outer.iter().any(|w| m.pos(*w) == corner.pos))
                .map(|(_, f)| f)
                .collect();
            assert_eq!(faces.len(), 3);
            let rows = glam::DMat3::from_cols(faces[0].normal, faces[1].normal, faces[2].normal)
                .transpose();
            let centre = corner.pos + rows.inverse() * DVec3::splat(-r);
            let on = on_ball(&out, centre, r);
            // Arcs of 7 to 9 steps, rings out to them, and a soft join to every fillet.
            assert!(on.len() > 3 * 7, "{} at {}", on.len(), corner.pos);
            check_patch(&out, centre, r, 0.02);
            patches += 1;
        }
        assert_eq!(patches, 8);
        assert!(signed_volume(&out) < signed_volume(&m));
    }

    #[test]
    fn a_prism_corner_with_unequal_arcs_gets_a_ringed_patch() {
        // A triangular prism's top corner: two 90 degree edges and an upright of 120.
        let p = Solid::prism(1.0, 2.0, 3, 0.0).unwrap().to_model();
        let r = 0.1;
        let top: Vec<EdgeId> = p
            .edges()
            .filter(|(_, e)| {
                let (a, b) = (p.pos(e.v[0]), p.pos(e.v[1]));
                a.z > 1.0 && b.z > 1.0
            })
            .map(|(id, _)| id)
            .collect();
        let corner = p
            .vertices()
            .map(|(_, v)| v.pos)
            .find(|q| q.z > 1.0 && q.x > 0.99)
            .unwrap();
        let mut edges: Vec<EdgeId> = top
            .iter()
            .copied()
            .filter(|e| {
                let e = p.edge(*e).unwrap();
                p.pos(e.v[0]) == corner || p.pos(e.v[1]) == corner
            })
            .collect();
        edges.push(
            p.edges()
                .find(|(_, e)| {
                    let (a, b) = (p.pos(e.v[0]), p.pos(e.v[1]));
                    (a == corner || b == corner) && (a - b).truncate().length() < 1e-9
                })
                .map(|(id, _)| id)
                .unwrap(),
        );
        assert_eq!(edges.len(), 3);
        let out = bevelled(&p, &edges, fillet(r)).unwrap();
        closed(&out);
        assert!(signed_volume(&out) < signed_volume(&p));
        // The ball sits r inside the top and both sides, here on the vertical bisector.
        let centre = DVec3::new(1.0 - r / 30f64.to_radians().sin(), 0.0, 2.0 - r);
        let on = on_ball(&out, centre, r);
        // Arcs of 8, 8 and 11 steps: 27 boundary points, some inner rings of 27, the middle.
        let faces = check_patch(&out, centre, r, 0.02);
        assert!(
            on.len() > 27 + 27 && (on.len() - 1).is_multiple_of(27),
            "{}",
            on.len()
        );
        let rings = (on.len() - 1) / 27;
        assert_eq!(faces, 27 * (1 + 2 * (rings - 1)));
    }

    #[test]
    fn three_chamfers_at_a_corner_meet_in_a_point() {
        let (size, d) = ([2.0, 3.0, 4.0], 0.5);
        let m = cuboid(size);
        let edges = [
            edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 4.0], [0.0, 3.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 0.0], [0.0, 0.0, 4.0]),
        ];
        let r = bevelled(&m, &edges, chamfer(d)).unwrap();
        closed(&r);
        // The three chamfer planes meet half a distance in from each face.
        assert!(has_vertex(&r, [d / 2.0, d / 2.0, 4.0 - d / 2.0]));
        // 6 faces and 3 chamfers; the faces at the corner each lose a corner cut.
        assert_eq!(r.faces().count(), 9);
        assert!(r.edges().all(|(_, e)| !e.soft));
    }

    #[test]
    fn mixed_rounds_and_chamfers_and_two_edge_corners_stay_solid() {
        let (size, r) = ([2.0, 3.0, 4.0], 0.3);
        let m = cuboid(size);
        // Two of the three edges at the corner: no sphere.
        let two = [
            edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 4.0], [0.0, 3.0, 4.0]),
        ];
        let out = bevelled(&m, &two, fillet(r)).unwrap();
        closed(&out);
        assert!(on_ball(&out, DVec3::new(r, r, 4.0 - r), r).len() < 45);
        // A corner rounded, then the rest of the edges chamfered and rounded larger.
        let three = [
            two[0],
            two[1],
            edge_at(&m, [0.0, 0.0, 0.0], [0.0, 0.0, 4.0]),
        ];
        let once = bevelled(&m, &three, fillet(r)).unwrap();
        closed(&once);
        let far = edge_at(&once, [2.0, 3.0, 0.0], [2.0, 3.0, 4.0]);
        let chamfered = bevelled(&once, &[far], chamfer(0.2)).unwrap();
        closed(&chamfered);
        let bottom = edge_at(&once, [0.0, 3.0, 0.0], [2.0, 3.0, 0.0]);
        let again = bevelled(&once, &[bottom], fillet(0.5)).unwrap();
        closed(&again);
        // Unequal radii at one corner: round two edges big, then the third small.
        let big = bevelled(&m, &two, fillet(0.6)).unwrap();
        closed(&big);
        let third = big
            .edges()
            .find(|(_, e)| {
                let (a, b) = (big.pos(e.v[0]), big.pos(e.v[1]));
                a.truncate() == glam::DVec2::ZERO && b.truncate() == glam::DVec2::ZERO
            })
            .map(|(id, _)| id)
            .unwrap();
        // The third round (0.2) is smaller than the two at the corner (0.6): refused cleanly,
        // where the other faces at its end do not lie in one plane.
        match bevelled(&big, &[third], fillet(0.2)) {
            Err(BevelError::Edge(id, why)) => {
                assert_eq!(id, third);
                assert!(why.contains("do not lie in one plane"), "{why}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_patch_is_shaded_smoothly_with_the_fillets() {
        let (size, r) = ([2.0, 3.0, 4.0], 0.5);
        let m = cuboid(size);
        let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
        let out = bevelled(&m, &edges, fillet(r)).unwrap();
        let smooth = out.smooth_normals();
        let centre = DVec3::new(r, r, r);
        let mut checked = 0;
        for (id, v) in out.vertices() {
            let radial = (v.pos - centre) / r;
            if (v.pos.distance(centre) - r).abs() > 1e-9 {
                continue;
            }
            // An inner point of the patch: every face around it is a patch face.
            let inner = out.faces_at(id).all(|f| {
                out.face(f)
                    .unwrap()
                    .outer
                    .iter()
                    .all(|w| (out.pos(*w).distance(centre) - r).abs() < 1e-9)
            });
            if !inner || v.pos.x > r - 1e-9 || v.pos.y > r - 1e-9 || v.pos.z > r - 1e-9 {
                continue;
            }
            for f in out.faces_at(id) {
                let n = smooth.get(f, id).expect("shaded as a fan");
                assert!(n.dot(radial) > 0.995, "{n} {radial}");
                checked += 1;
            }
        }
        assert!(checked > 20);
    }

    #[test]
    fn a_concave_edge_gains_a_cove() {
        let m = l_shape();
        let before = signed_volume(&m);
        let e = edge_at(&m, [1.0, 0.0, 1.0], [1.0, 2.0, 1.0]);
        let r = bevelled(&m, &[e], fillet(0.25)).unwrap();
        closed(&r);
        let v = signed_volume(&r);
        assert!(
            (v - (before + quarter_loss(0.25, 8) * 2.0)).abs() < 1e-9,
            "{v}"
        );
        assert!(has_vertex(&r, [1.25, 0.0, 1.0]));
        assert!(has_vertex(&r, [1.0, 2.0, 1.25]));
        assert!(!has_vertex(&r, [1.0, 0.0, 1.0]));
        // The cove's corners lie on the circle about (x, z) = (1.25, 1.25).
        for (_, w) in r.vertices() {
            let (x, z) = (w.pos.x, w.pos.z);
            if x > 1.0 + 1e-9 && x < 1.25 - 1e-9 && z > 1.0 + 1e-9 && z < 1.25 - 1e-9 {
                let d = DVec2::new(x - 1.25, z - 1.25).length();
                assert!((d - 0.25).abs() < 1e-9, "{}", w.pos);
            }
        }
        assert_eq!(r.edges().filter(|(_, e)| e.soft).count(), 9);
    }

    #[test]
    fn all_edges_of_an_l_round_off_without_grooves() {
        let m = l_shape();
        let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
        let r = bevelled(&m, &edges, fillet(0.1)).unwrap();
        closed(&r);
        local(&m, &r, &edges, 0.101);
        assert!(signed_volume(&r) < signed_volume(&m));
    }

    fn crossing(m: &Model, edges: &[EdgeId], profile: Profile, text: &str) {
        match bevelled(m, edges, profile) {
            Err(BevelError::Edges([x, y], why)) => {
                assert!(edges.contains(&x) && edges.contains(&y) && x < y);
                assert_eq!(
                    why,
                    format!("{text} too large for edges e{} and e{}", x.0, y.0)
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cuts_meeting_across_a_face_are_too_large() {
        let m = cuboid([1.0, 1.0, 1.0]);
        let all: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
        crossing(&m, &all, fillet(0.6), "radius");
        closed(&bevelled(&m, &all, fillet(0.45)).unwrap());
        let parallel = [
            edge_at(&m, [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]),
            edge_at(&m, [0.0, 1.0, 1.0], [1.0, 1.0, 1.0]),
        ];
        crossing(&m, &parallel, chamfer(0.7), "distance");
        crossing(&m, &parallel, fillet(0.7), "radius");
        // Each alone fits.
        assert!(bevelled(&m, &parallel[..1], chamfer(0.7)).is_ok());
        assert!(bevelled(&m, &parallel, chamfer(0.45)).is_ok());
    }

    /// The edges of `m` around its top face (at height `z`).
    fn top_loop(m: &Model, z: f64) -> Vec<EdgeId> {
        let top = m
            .faces()
            .find(|(_, f)| f.normal.z > 1.0 - 1e-9 && m.pos(f.outer[0]).z > z - 1e-9)
            .map(|(id, _)| id)
            .unwrap();
        m.face_edges(top)
    }

    #[test]
    fn a_rounded_box_rounds_again_around_its_top() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let upright: Vec<EdgeId> = m
            .edges()
            .filter(|(_, e)| (m.pos(e.v[0]) - m.pos(e.v[1])).truncate().length() < 1e-9)
            .map(|(id, _)| id)
            .collect();
        assert_eq!(upright.len(), 4);
        let first = Profile::Fillet {
            radius: 0.25,
            segments: 4,
        };
        let once = bevelled(&m, &upright, first).unwrap();
        closed(&once);
        let edges = top_loop(&once, 4.0);
        // 4 sides and 4 quarter rounds of 4 faces.
        assert_eq!(edges.len(), 4 + 16);
        let twice = bevelled(&once, &edges, fillet(0.2)).unwrap();
        closed(&twice);
        let removed = signed_volume(&once) - signed_volume(&twice);
        // The straight runs lose a quarter round each; along the corners less than their
        // outer length's worth.
        let loss = quarter_loss(0.2, 8);
        let straight = 2.0 * (1.5 + 2.5);
        let corners = 16.0 * 2.0 * 0.25 * (std::f64::consts::PI / 16.0).sin();
        assert!(
            removed > loss * straight && removed < loss * (straight + corners),
            "{removed}"
        );
        assert!(!has_vertex(&twice, [0.25, 0.0, 4.0]));
    }

    #[test]
    fn a_chamfer_beside_a_fillet_is_allowed() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let e = edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);
        let rounded = bevelled(&m, &[e], fillet(0.5)).unwrap();
        // The top face and the front face now have a soft edge; their other edges still bevel.
        let edges = [
            edge_at(&rounded, [0.0, 3.0, 4.0], [2.0, 3.0, 4.0]),
            edge_at(&rounded, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
        ];
        let r = bevelled(&rounded, &edges, chamfer(0.5)).unwrap();
        closed(&r);
        let v = signed_volume(&rounded) - 0.125 * 2.0 * 2.0;
        assert!((signed_volume(&r) - v).abs() < 1e-9);
        // The round stays soft.
        assert_eq!(r.edges().filter(|(_, e)| e.soft).count(), 9);
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
    fn a_soft_edge_is_refused() {
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
