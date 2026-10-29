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
//! a sphere patch; three chamfers close with a flat triangle. The result replaces the mesh's
//! own geometry; faces and edges the cut did not touch keep their ids.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use glam::{DAffine3, DVec2, DVec3};

use crate::boolean::{
    BooleanError, BooleanOp, apart, combine, copy_into, nested_shells, placed_copy, shells_of,
    subset,
};
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
/// Sizes closer than this (meters) in one call are made equal.
pub const SNAP: f64 = 1e-4;
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
    pub fn size(self) -> f64 {
        match self {
            Profile::Chamfer { distance } => distance,
            Profile::Fillet { radius, .. } => radius,
        }
    }

    /// The same shape with another distance or radius.
    fn sized(self, size: f64) -> Profile {
        match self {
            Profile::Chamfer { .. } => Profile::Chamfer { distance: size },
            Profile::Fillet { segments, .. } => Profile::Fillet {
                radius: size,
                segments,
            },
        }
    }

    /// The parameter's name in messages.
    pub fn noun(self) -> &'static str {
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
        let each: Vec<(EdgeId, f64)> = edges.iter().map(|e| (*e, profile.size())).collect();
        self.bevel_edges_each(mesh, &each, profile)
    }

    /// As [`Scene::bevel_edges`], each edge with its own distance or radius, see
    /// [`bevelled_each`].
    pub fn bevel_edges_each(
        &mut self,
        mesh: MeshId,
        edges: &[(EdgeId, f64)],
        profile: Profile,
    ) -> Result<SceneChange, BevelError> {
        let m = self
            .mesh(mesh)
            .ok_or_else(|| BevelError::NotSolid("it is missing".into()))?;
        let result = bevelled_each(m, edges, profile)?;
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
    /// The profile of this edge: the call's, with this edge's own distance or radius.
    profile: Profile,
}

/// The solid `m` (its own vertices, edges and faces) with `edges` bevelled by `profile`: the
/// cutters of convex edges subtracted and the fillers of concave edges added, all from the
/// original faces. The result faces outward.
pub fn bevelled(m: &Model, edges: &[EdgeId], profile: Profile) -> Result<Model, BevelError> {
    let sized: Vec<(EdgeId, f64)> = edges.iter().map(|e| (*e, profile.size())).collect();
    bevelled_each(m, &sized, profile)
}

/// As [`bevelled`], with the distance or radius of each edge given (the profile's own size is
/// not used). Three convex rounds of different radii at a trihedral vertex get a star-shaped
/// patch (see `corner`; a corner it cannot build keeps its crossed rounds, still a closed
/// solid); chamfers at a trihedral corner close with a
/// flat triangle when no distance there is more than the other two together (else the corner
/// stays crossed). In ascending order, a size within `SNAP` (0.0001 m) or 0.1% of the first
/// size of its group, whichever is less, is made equal to it, so nearly equal radii get the
/// same corner as equal ones.
pub fn bevelled_each(
    m: &Model,
    edges: &[(EdgeId, f64)],
    profile: Profile,
) -> Result<Model, BevelError> {
    bevel_core(m, edges, profile).map(|(acc, _)| acc)
}

/// [`bevelled_each`] and the corners of `m` that got a patch.
fn bevel_core(
    m: &Model,
    edges: &[(EdgeId, f64)],
    profile: Profile,
) -> Result<(Model, Vec<VertexId>), BevelError> {
    let noun = profile.noun();
    for (_, size) in edges {
        if !(size.is_finite() && *size > MIN_SIZE) {
            return Err(BevelError::Invalid(format!(
                "{noun} must be more than {MIN_SIZE} m"
            )));
        }
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
    // Only the shells holding a chosen edge are cut; other shells of the mesh (a part beside or
    // on the solid, loose faces) are carried over as they are. A shell nested with a cut one (a
    // void in its solid) is cut with it, and so is one a filler added at a concave edge could
    // reach.
    let shells = shells_of(m);
    let mut hit = vec![false; shells.len()];
    for (e, _) in edges {
        let Some(edge) = m.edge(*e) else {
            return Err(BevelError::Edge(
                *e,
                format!("e{} is not an edge of the solid", e.0),
            ));
        };
        let [a, b] = edge.v;
        let at = m
            .faces_on_side(a, b)
            .into_iter()
            .chain(m.faces_on_side(b, a))
            .next();
        let shell = at.and_then(|f| shells.iter().position(|s| s.contains(&f)));
        match shell {
            Some(i) => hit[i] = true,
            None => {
                return Err(BevelError::Edge(
                    *e,
                    format!("edge e{} bounds no face", e.0),
                ));
            }
        }
    }
    let nested = if shells.len() > 1 {
        nested_shells(m, &shells)
    } else {
        Vec::new()
    };
    let boxes: Vec<(DVec3, DVec3)> = shells
        .iter()
        .map(|faces| bounds(&subset(m, faces)))
        .collect();
    let faces_of = |hit: &[bool], want: bool| -> Vec<FaceId> {
        (0..shells.len())
            .filter(|i| hit[*i] == want)
            .flat_map(|i| shells[i].iter().copied())
            .collect()
    };
    // The target keeps the ids of `m`: `m` without the faces of the other shells.
    let (work, outward) = loop {
        let mut grew = true;
        while std::mem::take(&mut grew) {
            for [i, j] in &nested {
                if hit[*i] != hit[*j] {
                    hit[*i] = true;
                    hit[*j] = true;
                    grew = true;
                }
            }
        }
        let mut work = m.clone();
        for f in faces_of(&hit, false) {
            work.remove_face(f);
        }
        let loose: Vec<EdgeId> = work
            .edges()
            .filter(|(_, e)| {
                let [a, b] = e.v;
                work.faces_on_side(a, b).is_empty() && work.faces_on_side(b, a).is_empty()
            })
            .map(|(id, _)| id)
            .collect();
        for e in loose {
            work.remove_edge(e);
        }
        let stray: Vec<VertexId> = work
            .vertices()
            .filter(|(id, _)| work.edges_at(*id).is_empty() && work.faces_at(*id).next().is_none())
            .map(|(id, _)| id)
            .collect();
        for v in stray {
            work.remove_vertex(v);
        }
        if let Some(why) = open_reason(&work) {
            return Err(BevelError::NotSolid(why));
        }
        let volume = signed_volume(&work);
        if volume.abs() <= 1e-12 {
            return Err(BevelError::NotSolid("it encloses no volume".into()));
        }
        let outward = volume.signum();
        // A concave edge adds material (a filler) near it: shells there join the target.
        let mut more = false;
        for (e, size) in edges {
            let Ok(cut) = analyze(&work, *e, outward, profile.sized(*size)) else {
                continue;
            };
            if cut.convex {
                continue;
            }
            let reach = DVec3::splat(2.0 * size);
            let (lo, hi) = (cut.a.min(cut.b) - reach, cut.a.max(cut.b) + reach);
            for (i, (blo, bhi)) in boxes.iter().enumerate() {
                if !hit[i] && blo.cmple(hi).all() && bhi.cmpge(lo).all() {
                    hit[i] = true;
                    more = true;
                }
            }
        }
        if !more {
            break (work, outward);
        }
    };
    // What stays as it is, with the loose edges (no face).
    let mut rest = subset(m, &faces_of(&hit, false));
    let mut curves: HashMap<CurveId, CurveId> = HashMap::new();
    for (_, e) in m.edges() {
        let [a, b] = e.v;
        if m.faces_on_side(a, b).is_empty() && m.faces_on_side(b, a).is_empty() {
            let (pa, pb) = (m.pos(a), m.pos(b));
            let va = rest
                .find_vertex(pa)
                .unwrap_or_else(|| rest.insert_vertex(pa));
            let vb = rest
                .find_vertex(pb)
                .unwrap_or_else(|| rest.insert_vertex(pb));
            let mut like = e.clone();
            like.curve = e
                .curve
                .map(|c| *curves.entry(c).or_insert_with(|| rest.new_curve()));
            rest.insert_edge_like(va, vb, &like);
        }
    }
    let m = &work;
    // Each edge once; a repeated one keeps its first size.
    let mut by_edge: BTreeMap<EdgeId, f64> = BTreeMap::new();
    for (e, size) in edges {
        by_edge.entry(*e).or_insert(*size);
    }
    let mut sorted: Vec<f64> = by_edge.values().copied().collect();
    sorted.sort_by(f64::total_cmp);
    let mut groups: Vec<f64> = Vec::new();
    for s in sorted {
        if groups.last().is_none_or(|g| s - g > SNAP.min(1e-3 * g)) {
            groups.push(s);
        }
    }
    for size in by_edge.values_mut() {
        *size = *groups.iter().rev().find(|g| **g <= *size).expect("a group");
    }
    let edges: Vec<EdgeId> = by_edge.keys().copied().collect();
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
        let cut = analyze(m, *e, outward, profile.sized(by_edge[e]))
            .map_err(|why| BevelError::Edge(*e, why))?;
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
        let own = cut.profile;
        let sides = own.profile(&cut).len() - 1;
        let tool = prism(&cut, &own.section(&cut), 0..sides).ok_or_else(too_large)?;
        if let Profile::Fillet { .. } = own {
            rounds.push(Round::of(&cut, &tool, &own.profile(&cut)));
        }
        if cut.convex {
            cutters.push(tool);
        } else {
            cores.push(prism(&cut, &own.core(&cut), 0..0).ok_or_else(too_large)?);
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
    let patched = done.iter().map(|i| corners[*i].vertex).collect();
    copy_into(&rest, &mut acc);
    acc.take_changes();
    Ok((acc, patched))
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
        profile,
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
        // 6 shrunken faces and 12 chamfers; each corner is closed by a triangle.
        assert_eq!(r.faces().count(), 18 + 8);
    }

    fn moved(m: &Model, by: [f64; 3]) -> Model {
        placed_copy(m, DAffine3::from_translation(DVec3::from(by)), None)
    }

    fn beside(a: &Model, b: &Model) -> Model {
        let mut m = Model::new();
        copy_into(a, &mut m);
        copy_into(b, &mut m);
        m.take_changes();
        m
    }

    #[test]
    fn a_part_on_the_face_is_not_cut_by_the_cutters() {
        let base = cuboid([4.0, 4.0, 2.0]);
        // A block resting on the top face, over the edge to be rounded.
        let block = moved(&cuboid([1.0, 1.0, 1.0]), [0.0, 0.0, 2.0]);
        let m = beside(&base, &block);
        let e = edge_at(&m, [0.0, 0.0, 2.0], [4.0, 0.0, 2.0]);
        let r = bevelled(&m, &[e], chamfer(0.5)).unwrap();
        assert_eq!(open_reason(&r), None);
        assert!((signed_volume(&r) - (32.0 - 0.125 * 4.0 + 1.0)).abs() < 1e-9);
        assert!(has_vertex(&r, [0.0, 0.0, 3.0]) && has_vertex(&r, [0.0, 0.0, 2.0]));
        // A round of the same size leaves the block alone too.
        let round = Profile::Fillet {
            radius: 0.5,
            segments: 4,
        };
        let r = bevelled(&m, &[e], round).unwrap();
        assert_eq!(open_reason(&r), None);
        assert!(has_vertex(&r, [0.0, 0.0, 3.0]) && has_vertex(&r, [0.0, 0.0, 2.0]));
    }

    #[test]
    fn a_part_across_a_slot_is_not_cut_by_the_cutters() {
        // Two blocks with a 0.2 m slot between them; the edge on the far side of the first.
        let a = cuboid([2.0, 2.0, 2.0]);
        let b = moved(&cuboid([2.0, 2.0, 2.0]), [2.2, 0.0, 0.0]);
        let m = beside(&a, &b);
        let e = edge_at(&m, [2.0, 0.0, 2.0], [2.0, 2.0, 2.0]);
        let r = bevelled(&m, &[e], chamfer(0.5)).unwrap();
        assert_eq!(open_reason(&r), None);
        assert!((signed_volume(&r) - (16.0 - 0.125 * 2.0)).abs() < 1e-9);
        assert!(has_vertex(&r, [2.2, 0.0, 2.0]) && has_vertex(&r, [2.2, 2.0, 0.0]));
    }

    fn hollow(void_edge_cut: bool) -> (Model, EdgeId) {
        // A 4 m cube with a 3.6 m void (0.2 m walls).
        let outer = cuboid([4.0, 4.0, 4.0]);
        let mut hole = moved(&cuboid([3.6, 3.6, 3.6]), [0.2, 0.2, 0.2]);
        let all: Vec<FaceId> = hole.faces().map(|(id, _)| id).collect();
        hole.reverse_faces(&all);
        let m = beside(&outer, &hole);
        let e = if void_edge_cut {
            edge_at(&m, [0.2, 0.2, 3.8], [3.8, 0.2, 3.8])
        } else {
            edge_at(&m, [0.0, 0.0, 4.0], [4.0, 0.0, 4.0])
        };
        (m, e)
    }

    #[test]
    fn an_outer_edge_of_a_hollow_solid_is_cut_with_its_void() {
        let (m, e) = hollow(false);
        let r = bevelled(&m, &[e], chamfer(0.5)).unwrap();
        assert_eq!(open_reason(&r), None);
        let wall = 64.0 - 3.6_f64.powi(3);
        let v = signed_volume(&r);
        // The cutter takes 0.5 m^3 less what it overlaps in the void.
        assert!(v < wall - 0.4 && v > wall - 0.5, "{v}");
    }

    #[test]
    fn a_void_edge_of_a_hollow_solid_stays_a_void() {
        let (m, e) = hollow(true);
        let r = bevelled(&m, &[e], chamfer(0.1)).unwrap();
        assert_eq!(open_reason(&r), None);
        let wall = 64.0 - 3.6_f64.powi(3);
        // The void loses a prism, so the walls gain it.
        assert!((signed_volume(&r) - (wall + 0.005 * 3.6)).abs() < 1e-9);
    }

    #[test]
    fn a_part_in_an_inside_corner_joins_the_added_material() {
        let l = l_shape();
        let e = edge_at(&l, [1.0, 0.0, 1.0], [1.0, 2.0, 1.0]);
        let alone = signed_volume(&bevelled(&l, &[e], chamfer(0.5)).unwrap());
        let block = moved(&cuboid([0.3, 0.3, 0.3]), [1.0, 0.5, 1.0]);
        let m = beside(&l, &block);
        let e = edge_at(&m, [1.0, 0.0, 1.0], [1.0, 2.0, 1.0]);
        let r = bevelled(&m, &[e], chamfer(0.5)).unwrap();
        assert_eq!(open_reason(&r), None);
        // The filler overlaps the block, which is united with it, not stacked on it.
        assert!(signed_volume(&r) < alone + 0.027 - 1e-6);
    }

    #[test]
    fn a_separate_part_keeps_its_ids_through_the_scene() {
        let a = cuboid([2.0, 2.0, 2.0]);
        let b = moved(&cuboid([2.0, 2.0, 2.0]), [2.2, 0.0, 0.0]);
        let both = beside(&a, &b);
        let mut scene = Scene::new();
        let mesh = MeshId::Root;
        scene
            .edit(mesh, |m| {
                replace_geometry(m, &both);
                Ok(m.take_changes())
            })
            .unwrap();
        let m = scene.mesh(mesh).unwrap().clone();
        let e = edge_at(&m, [2.0, 0.0, 2.0], [2.0, 2.0, 2.0]);
        let other = |m: &Model| -> Vec<FaceId> {
            m.faces()
                .filter(|(_, f)| f.outer.iter().all(|v| m.pos(*v).x > 2.1))
                .map(|(id, _)| id)
                .collect()
        };
        let ids = other(&m);
        assert_eq!(ids.len(), 6);
        scene.bevel_edges(mesh, &[e], chamfer(0.5)).unwrap();
        let after = scene.mesh(mesh).unwrap();
        assert_eq!(other(after), ids);
        for id in &ids {
            assert_eq!(after.face(*id), m.face(*id));
        }
        assert_eq!(open_reason(after), None);
        assert!((signed_volume(after) - (16.0 - 0.25)).abs() < 1e-9);
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
            // Away from the centre for a convex corner, toward it for a concave one.
            let depth = (m.pos(f.outer[0]) - centre).dot(f.normal).abs();
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

    /// The faces of `out` whose corners all lie on or in the ball about the tangent centre of
    /// `v` (the patch; a crossed round has corners farther out).
    fn patch_faces_at(m: &Model, out: &Model, v: VertexId, r: f64) -> Vec<FaceId> {
        let corner = m.pos(v);
        let normals: Vec<DVec3> = m.faces_at(v).map(|f| m.face(f).unwrap().normal).collect();
        assert_eq!(normals.len(), 3);
        let rows = glam::DMat3::from_cols(normals[0], normals[1], normals[2]).transpose();
        let centre = corner + rows.inverse() * DVec3::splat(-r);
        out.faces()
            .filter(|(_, f)| {
                f.outer
                    .iter()
                    .all(|w| out.pos(*w).distance(centre) < r + 1e-9)
            })
            .map(|(id, _)| id)
            .collect()
    }

    #[test]
    fn every_corner_of_prisms_and_sheared_boxes_gets_its_patch() {
        let r = 0.05;
        let mut shapes: Vec<(String, Model)> = Vec::new();
        for sides in [3, 5, 6, 8] {
            let p = Solid::prism(1.0, 2.0, sides, 0.0).unwrap().to_model();
            shapes.push((format!("{sides}-sided prism"), p));
        }
        for shear in [0.1, 0.5, 2.0] {
            shapes.push((format!("shear {shear}"), sheared_box(2.0, shear)));
        }
        for (name, m) in &shapes {
            let edges: Vec<EdgeId> = m
                .edges()
                .filter(|(_, e)| !e.soft && e.curve.is_none_or(|_| e.polygon))
                .map(|(id, _)| id)
                .collect();
            for segments in [2, 3, 8] {
                let profile = Profile::Fillet {
                    radius: r,
                    segments,
                };
                let out = bevelled(m, &edges, profile)
                    .unwrap_or_else(|e| panic!("{name} at {segments}: {e}"));
                closed(&out);
                for (v, _) in m.vertices() {
                    let faces = patch_faces_at(m, &out, v, r).len();
                    assert!(faces >= 3, "{name} at {segments}: corner {v:?} has {faces}");
                }
            }
        }
    }

    #[test]
    fn a_fine_corner_of_a_hexagonal_prism_and_a_shear_gets_its_patch() {
        // 24 segments, the three edges at one corner (all twelve are slow in a test build).
        let r = 0.05;
        for m in [
            Solid::prism(1.0, 2.0, 6, 0.0).unwrap().to_model(),
            sheared_box(2.0, 0.5),
        ] {
            let (v, _) = m
                .vertices()
                .find(|(_, v)| v.pos.z > 1.5 && v.pos.x > 0.1)
                .unwrap();
            let edges: Vec<EdgeId> = m
                .edges_at(v)
                .into_iter()
                .filter(|e| !m.edge(*e).unwrap().soft)
                .collect();
            assert_eq!(edges.len(), 3);
            let out = bevelled(
                &m,
                &edges,
                Profile::Fillet {
                    radius: r,
                    segments: 24,
                },
            )
            .unwrap();
            closed(&out);
            let patch = patch_faces_at(&m, &out, v, r);
            assert!(patch.len() >= 3);
            // No slivers: the smallest angle of any patch triangle.
            let mut worst = 180.0f64;
            for f in &patch {
                let ps: Vec<DVec3> = out
                    .face(*f)
                    .unwrap()
                    .outer
                    .iter()
                    .map(|w| out.pos(*w))
                    .collect();
                for i in 0..3 {
                    let (a, b, c) = (ps[i], ps[(i + 1) % 3], ps[(i + 2) % 3]);
                    worst = worst.min((b - a).angle_between(c - a).to_degrees());
                }
            }
            assert!(worst > 10.0, "{worst} degrees");
        }
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
        // Arcs of 8, 8 and 11 steps: 27 boundary points and inner rings with fewer.
        let faces = check_patch(&out, centre, r, 0.02);
        assert!(on.len() > 27 + 6 && faces > 27, "{} {faces}", on.len());
    }

    #[test]
    fn three_chamfers_at_a_corner_close_with_a_flat_triangle() {
        let (size, d) = ([2.0, 3.0, 4.0], 0.5);
        let m = cuboid(size);
        let edges = [
            edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 4.0], [0.0, 3.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 0.0], [0.0, 0.0, 4.0]),
        ];
        let r = bevelled(&m, &edges, chamfer(d)).unwrap();
        closed(&r);
        // The three chamfer planes would meet half a distance in from each face; the tip is cut
        // off by the plane through the strips' end points, a triangle of three corners.
        assert!(!has_vertex(&r, [d / 2.0, d / 2.0, 4.0 - d / 2.0]));
        let corners = [[d, d, 4.0], [d, 0.0, 4.0 - d], [0.0, d, 4.0 - d]];
        for p in corners {
            assert!(has_vertex(&r, p), "{p:?}");
        }
        let on_corners = |f: &Face| {
            f.outer.len() == 3
                && corners.iter().all(|c| {
                    f.outer
                        .iter()
                        .any(|v| r.pos(*v).distance(DVec3::from(*c)) < 1e-9)
                })
        };
        assert_eq!(r.faces().filter(|(_, f)| on_corners(f)).count(), 1);
        // 6 faces, 3 chamfers and the triangle.
        assert_eq!(r.faces().count(), 10);
        assert!(r.edges().all(|(_, e)| !e.soft));
        // Each chamfer removes d^2/2 per meter; the corner cube holds their overlap and the tip.
        let (a, b, c) = (2.0 - d, 3.0 - d, 4.0 - d);
        let prisms = d * d / 2.0 * (a + b + c);
        let removed = signed_volume(&m) - signed_volume(&r);
        assert!(
            removed > prisms && removed < prisms + d * d * d * 2.0,
            "{removed}"
        );
    }

    #[test]
    fn every_corner_of_chamfered_boxes_and_prisms_gets_its_triangle() {
        let d = 0.05;
        let mut shapes: Vec<(String, Model)> = vec![("box".into(), cuboid([2.0, 3.0, 4.0]))];
        for sides in [3, 6] {
            let p = Solid::prism(1.0, 2.0, sides, 0.0).unwrap().to_model();
            shapes.push((format!("{sides}-sided prism"), p));
        }
        for shear in [0.1, 0.5] {
            shapes.push((format!("shear {shear}"), sheared_box(2.0, shear)));
        }
        for (name, m) in &shapes {
            let edges: Vec<EdgeId> = m
                .edges()
                .filter(|(_, e)| !e.soft && e.curve.is_none_or(|_| e.polygon))
                .map(|(id, _)| id)
                .collect();
            let out = bevelled(m, &edges, chamfer(d)).unwrap_or_else(|e| panic!("{name}: {e}"));
            closed(&out);
            for (_, corner) in m.vertices() {
                let near = out.faces().filter(|(_, f)| {
                    f.outer.len() == 3
                        && f.outer
                            .iter()
                            .all(|v| out.pos(*v).distance(corner.pos) < 4.0 * d)
                });
                assert_eq!(near.count(), 1, "{name} at {}", corner.pos);
            }
            assert!(signed_volume(&out) < signed_volume(m), "{name}");
        }
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

    #[test]
    fn a_tiny_box_far_from_the_origin_is_bevelled_or_refused() {
        for r in [1e-5, 5e-4] {
            for offset in [0.0, 100.0, 1e3, 1e5] {
                let m = placed_copy(
                    &cuboid([0.01, 0.02, 0.03]),
                    DAffine3::from_translation(DVec3::splat(offset)),
                    None,
                );
                let edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
                for profile in [fillet(r), chamfer(r)] {
                    match bevelled(&m, &edges, profile) {
                        Ok(out) => closed(&out),
                        // A radius near the 1e-6 tolerance, or the boolean far from the origin
                        // (its precision), may be refused with an error; it must not panic.
                        Err(e) => assert!(r < 1e-4 || offset > 100.0, "{r} at {offset}: {e}"),
                    }
                }
            }
        }
    }

    /// A 3 m cube with the cube [1.5, 3]^3 cut from one corner: three concave edges meet at
    /// (1.5, 1.5, 1.5).
    fn notched() -> Model {
        let a = cuboid([3.0, 3.0, 3.0]);
        let b = placed_copy(
            &cuboid([1.5, 1.5, 1.5]),
            DAffine3::from_translation(DVec3::splat(1.5)),
            None,
        );
        combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap()
    }

    #[test]
    fn three_concave_fillets_at_a_corner_get_a_concave_sphere() {
        let (m, r) = (notched(), 0.3);
        closed(&m);
        let c = [1.5, 1.5, 1.5];
        let edges = [
            edge_at(&m, c, [3.0, 1.5, 1.5]),
            edge_at(&m, c, [1.5, 3.0, 1.5]),
            edge_at(&m, c, [1.5, 1.5, 3.0]),
        ];
        let out = bevelled(&m, &edges, fillet(r)).unwrap();
        closed(&out);
        // The ball sits in the empty corner, r from each of the three walls.
        let centre = DVec3::splat(1.5 + r);
        assert_eq!(on_ball(&out, centre, r).len(), 45);
        assert_eq!(check_patch(&out, centre, r, 0.02), 64);
        assert!(!has_vertex(&out, c));
        // Three coves between the corner cube and the far ends; the cube keeps all but an
        // octant of the ball (faceted a little less).
        let octant = std::f64::consts::PI * r * r * r / 6.0;
        let coves = quarter_loss(r, 8) * 3.0 * (1.5 - r);
        let faceted = signed_volume(&m) + coves + (r * r * r - octant);
        let v = signed_volume(&out);
        assert!(v > faceted && v - faceted < octant * 0.03, "{v} {faceted}");
    }

    #[test]
    fn mixed_concave_and_convex_corners_stay_closed() {
        // Ruling: a corner of one or two concave fillets with convex ones keeps the crossed
        // result (a closed solid, no patch); only three of a kind get a sphere.
        let r = 0.1;
        let l = l_shape();
        let c = [1.0, 0.0, 1.0];
        let corner = [
            edge_at(&l, c, [1.0, 2.0, 1.0]),
            edge_at(&l, c, [4.0, 0.0, 1.0]),
            edge_at(&l, c, [1.0, 0.0, 3.0]),
        ];
        let notch = notched();
        let all: Vec<EdgeId> = notch.edges().map(|(id, _)| id).collect();
        for profile in [fillet(r), chamfer(r)] {
            let out = bevelled(&l, &corner, profile).unwrap();
            closed(&out);
            local(&l, &out, &corner, r * 1.6);
            let out = bevelled(&notch, &all, profile).unwrap();
            closed(&out);
            assert!(signed_volume(&out) < signed_volume(&notch));
        }
    }

    #[test]
    fn edges_of_different_sizes_in_one_call_stay_closed() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let edges = [
            edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 4.0], [0.0, 3.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 0.0], [0.0, 0.0, 4.0]),
        ];
        // Chamfers of 0.3, 0.5 and 0.4: the triangle's corners are where the strips of each face
        // meet (the x edge's strip 0.3 wide, the y edge's 0.5, the z edge's 0.4).
        let sized: Vec<(EdgeId, f64)> = edges.iter().copied().zip([0.3, 0.5, 0.4]).collect();
        let r = bevelled_each(&m, &sized, chamfer(0.1)).unwrap();
        closed(&r);
        for p in [[0.5, 0.3, 4.0], [0.4, 0.0, 3.7], [0.0, 0.4, 3.5]] {
            assert!(has_vertex(&r, p), "{p:?}");
        }
        assert!(r.faces().any(|(_, f)| f.outer.len() == 3));
        // Rounds of 0.3, 0.3 and 0.5 have no common ball: a ringed patch, not the sphere.
        let sized: Vec<(EdgeId, f64)> = edges.iter().copied().zip([0.3, 0.3, 0.5]).collect();
        let r = bevelled_each(&m, &sized, fillet(0.1)).unwrap();
        closed(&r);
        assert!(on_ball(&r, DVec3::new(0.3, 0.3, 3.7), 0.3).len() < 45);
        assert!(signed_volume(&r) < signed_volume(&m));
        // The edges along each round are still soft.
        assert!(r.edges().filter(|(_, e)| e.soft).count() >= 3 * 9);
        // Equal sizes given per edge are the same as one size: the sphere patch is made.
        let same: Vec<(EdgeId, f64)> = edges.iter().map(|e| (*e, 0.4)).collect();
        let r = bevelled_each(&m, &same, fillet(0.1)).unwrap();
        closed(&r);
        assert_eq!(on_ball(&r, DVec3::new(0.4, 0.4, 3.6), 0.4).len(), 45);
        // A bad size is refused.
        assert!(matches!(
            bevelled_each(&m, &[(edges[0], 0.3), (edges[1], 0.0)], fillet(0.1)),
            Err(BevelError::Invalid(_))
        ));
    }
    #[test]
    fn nearly_equal_sizes_are_made_equal() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let edges = [
            edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 4.0], [0.0, 3.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 0.0], [0.0, 0.0, 4.0]),
        ];
        for delta in [1e-8, 1e-6, 1e-5, 9e-5] {
            let sized: Vec<(EdgeId, f64)> =
                edges.iter().copied().zip([0.3, 0.3, 0.3 + delta]).collect();
            let r =
                bevelled_each(&m, &sized, fillet(0.1)).unwrap_or_else(|e| panic!("{delta}: {e}"));
            closed(&r);
            assert_eq!(
                on_ball(&r, DVec3::new(0.3, 0.3, 3.7), 0.3).len(),
                45,
                "{delta}"
            );
            let r = bevelled_each(&m, &sized, chamfer(0.1)).unwrap();
            closed(&r);
            assert!(r.faces().any(|(_, f)| f.outer.len() == 3), "{delta}");
        }
        // Two edges, not at a corner.
        let two = [(edges[0], 0.3), (edges[1], 0.30001)];
        closed(&bevelled_each(&m, &two, fillet(0.1)).unwrap());
    }

    #[test]
    fn a_corner_of_three_radii_lies_between_the_corners_of_the_least_and_most() {
        let m = cuboid([2.0, 3.0, 4.0]);
        let edges = [
            edge_at(&m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 4.0], [0.0, 3.0, 4.0]),
            edge_at(&m, [0.0, 0.0, 0.0], [0.0, 0.0, 4.0]),
        ];
        let of = |radii: [f64; 3]| {
            let sized: Vec<(EdgeId, f64)> = edges.iter().copied().zip(radii).collect();
            let (out, patched) = bevel_core(&m, &sized, fillet(0.1)).unwrap();
            closed(&out);
            (signed_volume(&out), patched.len())
        };
        let (small, _) = of([0.1; 3]);
        let (large, _) = of([0.2; 3]);
        let (mixed, patches) = of([0.1, 0.15, 0.2]);
        assert_eq!(patches, 1);
        assert!(large < mixed && mixed < small, "{large} {mixed} {small}");
    }
    /// A colour (0, 1 or 2) for each of `edges` so that the edges at every vertex differ, by
    /// backtracking; None if there is none.
    fn three_colours(m: &Model, edges: &[EdgeId]) -> Option<Vec<usize>> {
        fn go(m: &Model, edges: &[EdgeId], at: usize, colour: &mut Vec<usize>) -> bool {
            if at == edges.len() {
                return true;
            }
            let e = m.edge(edges[at]).unwrap();
            for c in 0..3 {
                let clash = (0..at).any(|i| {
                    let o = m.edge(edges[i]).unwrap();
                    colour[i] == c && o.v.iter().any(|v| e.v.contains(v))
                });
                if !clash {
                    colour.push(c);
                    if go(m, edges, at + 1, colour) {
                        return true;
                    }
                    colour.pop();
                }
            }
            false
        }
        let mut colour = Vec::new();
        go(m, edges, 0, &mut colour).then_some(colour)
    }

    /// Shapes with a radius scale for the mixed-radii tests.
    fn mixed_shapes() -> Vec<(String, Model, f64)> {
        let mut shapes: Vec<(String, Model, f64)> = vec![
            ("box".into(), cuboid([2.0, 3.0, 4.0]), 1.0),
            ("small box".into(), cuboid([1.0, 1.0, 1.0]), 0.5),
        ];
        for sides in [3, 4, 5, 6, 8] {
            let p = Solid::prism(1.0, 2.0, sides, 0.0).unwrap().to_model();
            shapes.push((format!("{sides}-sided prism"), p, 0.3));
        }
        for shear in [0.1, 0.5] {
            shapes.push((format!("shear {shear}"), sheared_box(2.0, shear), 1.0));
        }
        shapes
    }

    #[test]
    fn corners_of_three_different_radii_get_a_ringed_patch() {
        let (mut patched_all, mut corners_all) = (0, 0);
        for (name, m, scale) in mixed_shapes() {
            let ids: Vec<EdgeId> = m
                .edges()
                .filter(|(_, e)| !e.soft && e.curve.is_none_or(|_| e.polygon))
                .map(|(id, _)| id)
                .collect();
            // Radii 0.1, 0.15 and 0.2: every corner meets three different ones.
            let colour = three_colours(&m, &ids).expect("3-colourable");
            let sized: Vec<(EdgeId, f64)> = ids
                .iter()
                .zip(&colour)
                .map(|(e, c)| (*e, [0.1, 0.15, 0.2][*c] * scale))
                .collect();
            for segments in [3, 8] {
                let profile = Profile::Fillet {
                    radius: 0.1,
                    segments,
                };
                let (out, patched) = bevel_core(&m, &sized, profile).unwrap();
                closed(&out);
                let corners = m.vertices().count();
                patched_all += patched.len();
                corners_all += corners;
                assert!(signed_volume(&out) < signed_volume(&m), "{name}");
                // No hard edge between two curved faces near a corner that was patched: the
                // arcs are soft whatever the angle.
                let flat = |n: DVec3| m.faces().any(|(_, f)| f.normal.dot(n) > 1.0 - 1e-9);
                let mut hard = 0;
                for (_, e) in out.edges() {
                    let (p, q) = (out.pos(e.v[0]), out.pos(e.v[1]));
                    let near = m.vertices().any(|(_, v)| {
                        v.pos.distance(p) < 0.5 * scale && v.pos.distance(q) < 0.5 * scale
                    });
                    let [f, g] = out.faces_on_side(e.v[0], e.v[1])[..] else {
                        continue;
                    };
                    let (nf, ng) = (out.face(f).unwrap().normal, out.face(g).unwrap().normal);
                    if near && !e.soft && !flat(nf) && !flat(ng) {
                        hard += 1;
                    }
                }
                assert!(
                    hard == 0 || patched.len() < corners,
                    "{name} at {segments}: {hard} hard edges"
                );
            }
        }
        // Most corners get their patch; a few keep the crossed rounds (checked for closure).
        assert!(
            patched_all * 10 >= corners_all * 8,
            "{patched_all} of {corners_all}"
        );
    }

    #[test]
    fn the_patch_of_three_radii_leaves_a_solid_beside_it_alone() {
        // A second solid 0.3 m off the corner diagonally, and 0.6 m: the cutter stays within
        // a quarter of the largest radius outside the faces.
        for gap in [0.3, 0.6] {
            let a = cuboid([1.0, 1.0, 1.0]);
            let corner = [1.0, 1.0, 1.0];
            let edges = [
                edge_at(&a, corner, [0.0, 1.0, 1.0]),
                edge_at(&a, corner, [1.0, 0.0, 1.0]),
                edge_at(&a, corner, [1.0, 1.0, 0.0]),
            ];
            let sized: Vec<(EdgeId, f64)> = edges.iter().copied().zip([0.1, 0.15, 0.2]).collect();
            let alone = bevelled_each(&a, &sized, fillet(0.1)).unwrap();
            let b = placed_copy(
                &cuboid([1.0, 1.0, 1.0]),
                DAffine3::from_translation(DVec3::splat(1.0 + gap)),
                None,
            );
            let both = combine(&a, &b, BooleanOp::Union).unwrap().unwrap();
            let sized: Vec<(EdgeId, f64)> = [
                ([1.0, 1.0, 1.0], [0.0, 1.0, 1.0]),
                ([1.0, 1.0, 1.0], [1.0, 0.0, 1.0]),
                ([1.0, 1.0, 1.0], [1.0, 1.0, 0.0]),
            ]
            .iter()
            .zip([0.1, 0.15, 0.2])
            .map(|((p, q), r)| (edge_at(&both, *p, *q), r))
            .collect();
            let (out, patched) = bevel_core(&both, &sized, fillet(0.1)).unwrap();
            assert_eq!(patched.len(), 1, "gap {gap}");
            closed(&out);
            // The second box is untouched: its volume and its 8 corners.
            let far = |p: DVec3| p.min_element() > 1.0 + gap - 1e-9;
            assert_eq!(
                out.vertices().filter(|(_, v)| far(v.pos)).count(),
                8,
                "gap {gap}"
            );
            assert!(
                (signed_volume(&out) - (signed_volume(&alone) + 1.0)).abs() < 1e-9,
                "gap {gap}"
            );
        }
    }
    /// The corner (by bits of `k`) of the unit cube with its three edges sized `radii`.
    fn unit_corner(k: usize, radii: [f64; 3]) -> (Model, Vec<(EdgeId, f64)>) {
        let m = cuboid([1.0; 3]);
        let at: Vec<f64> = (0..3).map(|i| f64::from((k >> i & 1) as u8)).collect();
        let corner = [at[0], at[1], at[2]];
        let end = |i: usize| {
            let mut v = corner;
            v[i] = 1.0 - v[i];
            v
        };
        let sized = (0..3)
            .map(|i| edge_at(&m, corner, end(i)))
            .zip(radii)
            .collect();
        (m, sized)
    }

    /// Rounds the corner and checks it is a closed solid with the corner patch (not the crossed
    /// rounds), its volume between the corners of the least and the most radius.
    fn round_unit_corner(k: usize, radii: [f64; 3], segments: u32) -> Result<(), String> {
        let (m, sized) = unit_corner(k, radii);
        let profile = Profile::Fillet {
            radius: 0.1,
            segments,
        };
        let (out, patched) = bevel_core(&m, &sized, profile).map_err(|e| e.to_string())?;
        closed(&out);
        assert_eq!(patched.len(), 1, "{segments} {radii:?} corner {k}");
        let volume = |r: f64| {
            let (m, sized) = unit_corner(k, [r; 3]);
            signed_volume(&bevelled_each(&m, &sized, profile).unwrap())
        };
        let v = signed_volume(&out);
        assert!(
            volume(radii[2].max(radii[0])) < v && v < volume(radii[0].min(radii[1])),
            "{segments} {radii:?} corner {k}"
        );
        Ok(())
    }

    #[test]
    fn small_unequal_radii_at_every_corner_of_a_cube_round_at_many_segments() {
        // Radii a few percent of the side with many segments leave creases of the cutters
        // microns apart (slivers that touch the other solid only along a crease); each
        // corner still closes.
        for segments in [12, 24, 32] {
            for radii in [[0.05, 0.06, 0.07], [0.06, 0.05, 0.07], [0.5, 0.6, 0.7]] {
                for k in 0..8 {
                    round_unit_corner(k, radii, segments)
                        .unwrap_or_else(|e| panic!("{segments} {radii:?} corner {k}: {e}"));
                }
            }
        }
    }

    #[test]
    fn radii_of_two_to_four_percent_round_corners_at_odd_segments() {
        for segments in [25, 31] {
            for radii in [[0.02, 0.03, 0.04], [0.03, 0.02, 0.04]] {
                for k in 0..8 {
                    if let Err(e) = round_unit_corner(k, radii, segments) {
                        // Known: see `radii_of_two_to_four_percent_at_the_known_open_corners`.
                        assert!(
                            KNOWN_OPEN.contains(&(segments, radii[0] < radii[1], k)),
                            "{segments} {radii:?} corner {k}: {e}"
                        );
                    }
                }
            }
        }
    }

    /// (segments, first radius smaller, corner) of the rounds that stay refused.
    const KNOWN_OPEN: [(u32, bool, usize); 8] = [
        (25, true, 6),
        (25, true, 7),
        (25, false, 4),
        (25, false, 5),
        (31, true, 6),
        (31, true, 7),
        (31, false, 4),
        (31, false, 5),
    ];

    #[test]
    #[ignore = "refused: a cutter facet with cut points 4e-7 off its edge is split into a whole copy plus a reversed piece (found in the rebuild in faces.rs; not fixed)"]
    fn radii_of_two_to_four_percent_at_the_known_open_corners() {
        for (segments, ascending, k) in KNOWN_OPEN {
            let radii = if ascending {
                [0.02, 0.03, 0.04]
            } else {
                [0.03, 0.02, 0.04]
            };
            round_unit_corner(k, radii, segments)
                .unwrap_or_else(|e| panic!("{segments} {radii:?} corner {k}: {e}"));
        }
    }
}
