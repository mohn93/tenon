//! Solid booleans: the union, difference and intersection of closed meshes (Solid Tools).
//!
//! Both solids are brought into one coordinate space with their faces pointing outward. Each
//! one's faces are cut along the other's surface (the crossing segments of Intersect Faces,
//! drawn into each mesh so its faces split), every resulting face is classified as inside,
//! outside or on the other solid (on: coplanar, facing the same or the opposite way) from an
//! interior point by ray parity, the faces the operation needs are kept (a subtracted solid's
//! turned over), and they are welded into one mesh, which must be closed.

use std::collections::{HashMap, HashSet};
use std::fmt;

use glam::{DAffine3, DVec3};

use crate::edit::insert_between;
use crate::geom::{LENGTH_TOL, Plane, point_in_polygon, project_on_segment};
use crate::ids::{CurveId, EdgeId, FaceId, InstanceId, MaterialId, VertexId};
use crate::index::{Index, Shape};
use crate::intersect::{FaceShape, face_segments, face_segments_cost, face_shape};
use crate::model::{Edge, Face, Instance, Model};
use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};
use crate::solid::{instance_volume, open_reason, signed_volume};
use crate::texture::{TexturePlacement, is_level};

/// What a boolean keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanOp {
    /// Everything inside either solid.
    Union,
    /// The first solid minus the others.
    Subtract,
    /// What lies inside every solid.
    Intersect,
}

/// Why a boolean was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum BooleanError {
    /// An operand names no instance of the mesh.
    Missing(InstanceId),
    /// An operand is not a solid; the text says why.
    NotSolid(InstanceId, String),
    /// Numerical trouble would leave the result open.
    NotClosed,
    /// More face pairs, segments or work than `MAX_PAIRS`, `MAX_SEGMENTS` or `MAX_WORK`.
    TooMuch,
}

/// Most face pairs whose boxes meet, per combining pass (as Intersect Faces).
pub const MAX_PAIRS: usize = 200_000;
/// Most crossing segments per combining pass (as Intersect Faces).
pub const MAX_SEGMENTS: usize = 100_000;
/// Most edge visits of the crossing tests per pass (see `face_segments_cost`).
pub const MAX_WORK: u64 = 100_000_000;
/// The text of [`BooleanError::TooMuch`].
pub const TOO_MUCH: &str = "too many faces cross to combine at once";

/// The text of [`BooleanError::NotClosed`].
pub const NOT_CLOSED: &str = "the result would not be a solid";

impl fmt::Display for BooleanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BooleanError::Missing(id) => write!(f, "no instance i{}", id.0),
            BooleanError::NotSolid(id, why) => write!(f, "i{} is not a solid: {why}", id.0),
            BooleanError::NotClosed => f.write_str(NOT_CLOSED),
            BooleanError::TooMuch => f.write_str(TOO_MUCH),
        }
    }
}

impl std::error::Error for BooleanError {}

impl Scene {
    /// Combines the instances `ids` of mesh `parent` (each a solid, see
    /// [`instance_volume`]) by `op`: for `Subtract` the first minus each of the others in
    /// order, otherwise all of them. The result is one new group placed with the first
    /// instance's transform, material, tag and hidden flag, its definition named as the first
    /// one's; every instance in `ids` is erased. An empty result (a disjoint intersection, a
    /// tool removing everything) places nothing: the instance is None and the operands are
    /// still erased. On an error nothing changes.
    pub fn solid_boolean(
        &mut self,
        parent: MeshId,
        op: BooleanOp,
        ids: &[InstanceId],
    ) -> Result<(Option<InstanceId>, SceneChange), BooleanError> {
        self.boolean_of(parent, op, ids, ids, false)
    }

    /// As `solid_boolean` with `Subtract`, but the `tools` stay: only `target` is erased.
    pub fn solid_trim(
        &mut self,
        parent: MeshId,
        target: InstanceId,
        tools: &[InstanceId],
    ) -> Result<(Option<InstanceId>, SceneChange), BooleanError> {
        let ids: Vec<InstanceId> = std::iter::once(target)
            .chain(tools.iter().copied())
            .collect();
        self.boolean_of(parent, BooleanOp::Subtract, &ids, &[target], false)
    }

    /// The union of `ids` (one or more solids) without internal faces: shells of the result
    /// that lie inside another shell (voids, and solids floating in them) are dropped. Placed
    /// and erased as `solid_boolean`.
    pub fn outer_shell(
        &mut self,
        parent: MeshId,
        ids: &[InstanceId],
    ) -> Result<(Option<InstanceId>, SceneChange), BooleanError> {
        self.boolean_of(parent, BooleanOp::Union, ids, ids, true)
    }

    /// Splits solids `a` and `b` into `a - b`, what they share and `b - a`, three new groups
    /// placed in that order (a piece with nothing in it is not made), all with the transform,
    /// material, tag and hidden flag of `a`; `a` and `b` are erased. On an error nothing
    /// changes.
    pub fn solid_split(
        &mut self,
        parent: MeshId,
        a: InstanceId,
        b: InstanceId,
    ) -> Result<(Vec<InstanceId>, SceneChange), BooleanError> {
        let ids = [a, b];
        let operands = self.solid_operands(parent, &ids)?;
        let first = &operands[0];
        let (ma, mb) = self.operand_models(&operands);
        let pieces = split(&ma[0], &mb[0])?;
        let mut parts = Vec::new();
        let mut placed = Vec::new();
        for piece in pieces.iter().flatten() {
            let (id, change) = self.place_group(parent, first, piece);
            placed.push(id);
            parts.push(change);
        }
        parts.push(self.erase_instances(parent, &ids));
        Ok((placed, SceneChange::join(parts)))
    }

    /// The instances `ids` of `parent`, each checked to be a solid.
    fn solid_operands(
        &self,
        parent: MeshId,
        ids: &[InstanceId],
    ) -> Result<Vec<Instance>, BooleanError> {
        let mesh = self
            .mesh(parent)
            .ok_or_else(|| BooleanError::Missing(ids.first().copied().unwrap_or(InstanceId(0))))?;
        let mut operands: Vec<Instance> = Vec::new();
        for id in ids {
            let instance = mesh.instance(*id).ok_or(BooleanError::Missing(*id))?;
            instance_volume(self, instance).map_err(|why| BooleanError::NotSolid(*id, why))?;
            operands.push(instance.clone());
        }
        Ok(operands)
    }

    /// The first operand's mesh as it is and the others copied into its frame, outward. Both
    /// are returned as lists (the first of one, the rest of the others).
    fn operand_models(&self, operands: &[Instance]) -> (Vec<Model>, Vec<Model>) {
        let definition = |i: &Instance| &self.definition(i.definition).expect("checked").mesh;
        let first = &operands[0];
        let back = first.transform.inverse();
        let head = vec![placed_copy(definition(first), DAffine3::IDENTITY, None)];
        let others = operands[1..]
            .iter()
            .map(|other| {
                // A face side without a material shows the instance's; keep showing it.
                let fill = other.material.filter(|m| Some(*m) != first.material);
                placed_copy(definition(other), back * other.transform, fill)
            })
            .collect();
        (head, others)
    }

    /// Combines `ids` by `op` (see `solid_boolean`), erasing `erase`; with `shell` the
    /// result keeps only its outer shells.
    fn boolean_of(
        &mut self,
        parent: MeshId,
        op: BooleanOp,
        ids: &[InstanceId],
        erase: &[InstanceId],
        shell: bool,
    ) -> Result<(Option<InstanceId>, SceneChange), BooleanError> {
        let operands = self.solid_operands(parent, ids)?;
        let Some(first) = operands.first() else {
            return Ok((None, SceneChange::default()));
        };
        let (head, others) = self.operand_models(&operands);
        let mut result = head.into_iter().next();
        // Adding or taking away solids that lie apart is adding or taking away them together,
        // in one pass (twenty bores in a plate cut at once).
        let others = if op == BooleanOp::Intersect {
            others
        } else {
            apart(others)
        };
        for b in &others {
            let Some(acc) = &result else { break };
            result = combine(acc, b, op)?;
        }
        if shell && let Some(m) = &result {
            result = Some(outer_shells(m)?);
        }

        let Some(result) = result else {
            return Ok((None, self.erase_instances(parent, erase)));
        };
        let (placed, made) = self.place_group(parent, first, &result);
        let erased = self.erase_instances(parent, erase);
        Ok((Some(placed), SceneChange::join([made, erased])))
    }

    /// A new group holding `mesh`, placed in `parent` with the transform, material, tag and
    /// hidden flag of `like`, its definition named as `like`'s.
    fn place_group(
        &mut self,
        parent: MeshId,
        like: &Instance,
        mesh: &Model,
    ) -> (InstanceId, SceneChange) {
        let name = self
            .definition(like.definition)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        let (definition, added) = self.add_definition(&name, DefinitionKind::Group);
        let filled = self
            .edit(MeshId::Definition(definition), |m| {
                copy_into(mesh, m);
                Ok(m.take_changes())
            })
            .expect("filling a new definition cannot fail");
        let mut placed = None;
        let left = self
            .edit(parent, |m| {
                let mut instance = Instance::new(definition, like.transform);
                instance.material = like.material;
                instance.tag = like.tag;
                instance.hidden = like.hidden;
                placed = Some(m.insert_instance(instance));
                Ok(m.take_changes())
            })
            .expect("placing an instance cannot fail");
        (
            placed.expect("placed"),
            SceneChange::join([SceneChange::from(added), filled, left]),
        )
    }
}

/// `m` (closed) with only its outer shells: a shell (a set of faces joined across edges) that
/// lies inside another shell is dropped, so voids and what floats in them go.
fn outer_shells(m: &Model) -> Result<Model, BooleanError> {
    let shells = shells_of(m);
    if shells.len() < 2 {
        return Ok(m.clone());
    }
    let parts: Vec<Model> = shells.iter().map(|faces| subset(m, faces)).collect();
    let mut keep: Vec<usize> = Vec::new();
    for (i, faces) in shells.iter().enumerate() {
        let p = faces
            .iter()
            .find_map(|f| interior_point(m, *f).map(|p| (p, m.face(*f).expect("face").normal)))
            .ok_or(BooleanError::NotClosed)?;
        let mut enclosed = false;
        for (j, other) in parts.iter().enumerate() {
            if i == j {
                continue;
            }
            match classify(p.0, p.1, other, true).ok_or(BooleanError::NotClosed)? {
                Class::Inside => enclosed = true,
                Class::Outside => {}
                // Shells never share a face.
                Class::Same | Class::Opposite => return Err(BooleanError::NotClosed),
            }
        }
        if !enclosed {
            keep.push(i);
        }
    }
    if keep.is_empty() {
        return Err(BooleanError::NotClosed);
    }
    if keep.len() == shells.len() {
        return Ok(m.clone());
    }
    let faces: Vec<FaceId> = keep.iter().flat_map(|i| shells[*i].clone()).collect();
    Ok(subset(m, &faces))
}

/// Pairs of `shells` of `m` where one lies inside the other (a void in its solid, a part
/// floating in a void), either way round.
pub(crate) fn nested_shells(m: &Model, shells: &[Vec<FaceId>]) -> Vec<[usize; 2]> {
    let parts: Vec<Model> = shells.iter().map(|faces| subset(m, faces)).collect();
    let mut pairs = Vec::new();
    for (i, faces) in shells.iter().enumerate() {
        let Some(p) = faces
            .iter()
            .find_map(|f| interior_point(m, *f).map(|p| (p, m.face(*f).expect("face").normal)))
        else {
            continue;
        };
        for (j, other) in parts.iter().enumerate() {
            if i != j && classify(p.0, p.1, other, true) == Some(Class::Inside) {
                pairs.push([i, j]);
            }
        }
    }
    pairs
}

/// The faces of `m` grouped into shells: faces joined across the edges of their loops. An edge
/// that more than two faces meet at (two solids sharing it) joins each face only to the one
/// that closes its solid there, so solids touching along an edge or at a vertex stay apart.
pub(crate) fn shells_of(m: &Model) -> Vec<Vec<FaceId>> {
    let ids: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
    let at: HashMap<FaceId, usize> = ids.iter().enumerate().map(|(i, f)| (*f, i)).collect();
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut seen: HashSet<(VertexId, VertexId)> = HashSet::new();
    for id in &ids {
        let f = m.face(*id).expect("face");
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            for k in 0..l.len() {
                let (a, b) = (l[k], l[(k + 1) % l.len()]);
                let on = m.faces_on_side(a, b);
                let pairs: Vec<(FaceId, FaceId)> = if on.len() <= 2 {
                    on.iter().map(|g| (*id, *g)).collect()
                } else if seen.insert((a.min(b), a.max(b))) {
                    wedges(m, a, b, &on)
                } else {
                    Vec::new()
                };
                for (f, g) in pairs {
                    let (x, y) = (root(&mut parent, at[&f]), root(&mut parent, at[&g]));
                    parent[x] = y;
                }
            }
        }
    }
    let mut groups: Vec<Vec<FaceId>> = Vec::new();
    let mut index: HashMap<usize, usize> = HashMap::new();
    for (i, id) in ids.iter().enumerate() {
        let r = root(&mut parent, i);
        let g = *index.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[g].push(*id);
    }
    groups
}

/// The pairs of faces at the edge a-b that bound one wedge of material each: `forward` run a to
/// b, `back` run b to a. Going round the edge, the material of a face running a to b lies
/// towards decreasing angle, and the next face met there (running b to a) closes it.
fn wedges(m: &Model, a: VertexId, b: VertexId, on: &[FaceId]) -> Vec<(FaceId, FaceId)> {
    let runs_forward = |f: FaceId| {
        let face = m.face(f).expect("face");
        std::iter::once(&face.outer)
            .chain(&face.holes)
            .any(|l| (0..l.len()).any(|i| (l[i], l[(i + 1) % l.len()]) == (a, b)))
    };
    let (forward, back): (Vec<FaceId>, Vec<FaceId>) = on.iter().partition(|f| runs_forward(**f));
    let d = (m.pos(b) - m.pos(a)).normalize_or_zero();
    let basis = d.any_orthonormal_vector();
    let basis2 = d.cross(basis);
    // The angle about d of the direction a face runs from the edge into itself.
    let angle = |f: FaceId, along: f64| -> f64 {
        let u = m.face(f).expect("face").normal.cross(d * along);
        u.dot(basis2).atan2(u.dot(basis))
    };
    let back_angles: Vec<(FaceId, f64)> = back.iter().map(|g| (*g, angle(*g, -1.0))).collect();
    let fwd_angles: Vec<(FaceId, f64)> = forward.iter().map(|f| (*f, angle(*f, 1.0))).collect();
    // The turn from `from` down to `to`; a face at the same angle is a whole turn away.
    let gap = |from: f64, to: f64| match (from - to).rem_euclid(std::f64::consts::TAU) {
        g if g < 1e-9 => std::f64::consts::TAU,
        g => g,
    };
    let nearest = |of: &[(FaceId, f64)], cost: &dyn Fn(f64) -> f64| {
        of.iter()
            .min_by(|x, y| cost(x.1).total_cmp(&cost(y.1)))
            .map(|(g, _)| *g)
    };
    // A pair counts when each face is the other's neighbour round the edge; a sheet or fin
    // with no wedge of its own joins nothing.
    let mut pairs = Vec::new();
    for (f, alpha) in &fwd_angles {
        let Some(g) = nearest(&back_angles, &|t| gap(*alpha, t)) else {
            continue;
        };
        let t = back_angles.iter().find(|x| x.0 == g).expect("back").1;
        if nearest(&fwd_angles, &|a| gap(a, t)) == Some(*f) {
            pairs.push((*f, g));
        }
    }
    // Faces back to back (the same plane, facing apart) are one solid, as they are where
    // the edge has no other faces: two solids stacked face to face.
    for (f, alpha) in &fwd_angles {
        for (g, t) in &back_angles {
            if (alpha - t)
                .rem_euclid(std::f64::consts::TAU)
                .min((t - alpha).rem_euclid(std::f64::consts::TAU))
                < 1e-9
            {
                pairs.push((*f, *g));
            }
        }
    }
    pairs
}

/// A copy of `m` holding only `faces` and their edges and vertices.
pub(crate) fn subset(m: &Model, faces: &[FaceId]) -> Model {
    let mut out = Model::new();
    let mut vertices: HashMap<VertexId, VertexId> = HashMap::new();
    let mut curves: HashMap<CurveId, CurveId> = HashMap::new();
    for id in faces {
        let f = m.face(*id).expect("face");
        for v in f.outer.iter().chain(f.holes.iter().flatten()) {
            vertices
                .entry(*v)
                .or_insert_with(|| out.insert_vertex(m.pos(*v)));
        }
    }
    let mut done: HashSet<(VertexId, VertexId)> = HashSet::new();
    let mut made = Vec::new();
    for id in faces {
        let f = m.face(*id).expect("face");
        let loops: Vec<Vec<VertexId>> = std::iter::once(&f.outer)
            .chain(&f.holes)
            .map(|l| l.iter().map(|v| vertices[v]).collect())
            .collect();
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            for k in 0..l.len() {
                let (a, b) = (l[k], l[(k + 1) % l.len()]);
                if done.insert((a.min(b), a.max(b)))
                    && let Some(e) = m.edge_between(a, b).and_then(|e| m.edge(e))
                {
                    let mut like = e.clone();
                    like.curve = e
                        .curve
                        .map(|c| *curves.entry(c).or_insert_with(|| out.new_curve()));
                    out.insert_edge_like(vertices[&a], vertices[&b], &like);
                }
            }
        }
        made.push((loops, f));
    }
    for (mut loops, f) in made {
        let outer = loops.remove(0);
        out.insert_face(Face {
            outer,
            holes: loops,
            ..f.clone()
        });
    }
    out.take_changes();
    out
}

/// The meshes gathered, in order, into meshes of several whose bounding boxes do not meet.
pub(crate) fn apart(meshes: Vec<Model>) -> Vec<Model> {
    let pad = DVec3::splat(10.0 * LENGTH_TOL);
    let mut groups: Vec<Vec<(Model, (DVec3, DVec3))>> = Vec::new();
    for m in meshes {
        let (lo, hi) = bounds(m.vertices().map(|(_, v)| v.pos));
        let (lo, hi) = (lo - pad, hi + pad);
        let meets = |g: &Vec<(Model, (DVec3, DVec3))>| {
            g.iter()
                .any(|(_, (l, h))| lo.cmple(*h).all() && hi.cmpge(*l).all())
        };
        match groups.last_mut() {
            Some(g) if !meets(g) => g.push((m, (lo, hi))),
            _ => groups.push(vec![(m, (lo, hi))]),
        }
    }
    groups
        .into_iter()
        .map(|g| {
            if g.len() == 1 {
                return g.into_iter().next().expect("one").0;
            }
            let mut all = Model::new();
            for (m, _) in &g {
                copy_into(m, &mut all);
            }
            all.take_changes();
            all
        })
        .collect()
}

/// A copy of `mesh` moved by `place` (which may mirror), with its faces turned outward and
/// face sides without a material given `fill`. Curves get ids of the copy's own.
pub(crate) fn placed_copy(mesh: &Model, place: DAffine3, fill: Option<MaterialId>) -> Model {
    let mut m = Model::new();
    let mirror = place.matrix3.determinant() < 0.0;
    let normals = place.matrix3.inverse().transpose();
    let mut vertices: HashMap<VertexId, VertexId> = HashMap::new();
    for (id, v) in mesh.vertices() {
        vertices.insert(id, m.insert_vertex(place.transform_point3(v.pos)));
    }
    let mut curves: HashMap<CurveId, CurveId> = HashMap::new();
    for (_, e) in mesh.edges() {
        let mut like = e.clone();
        like.curve = e
            .curve
            .map(|c| *curves.entry(c).or_insert_with(|| m.new_curve()));
        m.insert_edge_like(vertices[&e.v[0]], vertices[&e.v[1]], &like);
    }
    for (_, f) in mesh.faces() {
        let map = |l: &Vec<VertexId>| {
            let mut l: Vec<VertexId> = l.iter().map(|v| vertices[v]).collect();
            // A mirror turns the winding around the moved normal.
            if mirror {
                l.reverse();
            }
            l
        };
        let (front, back) = (f.front.or(fill), f.back.or(fill));
        // Only a painted face shows a texture; its placement takes up the move.
        let texture = if front.is_some() || back.is_some() {
            TexturePlacement::carried(f.texture, f.normal, &place)
        } else {
            None
        };
        m.insert_face(Face {
            outer: map(&f.outer),
            holes: f.holes.iter().map(map).collect(),
            normal: (normals * f.normal).normalize_or_zero(),
            front,
            back,
            texture,
            tag: f.tag,
            hidden: f.hidden,
        });
    }
    if signed_volume(&m) < 0.0 {
        let all: Vec<FaceId> = m.faces().map(|(id, _)| id).collect();
        m.reverse_faces(&all);
    }
    m.take_changes();
    m
}

/// Copies every vertex, edge and face of `src` into `m` as they are, curves under new ids.
pub(crate) fn copy_into(src: &Model, m: &mut Model) {
    let mut vertices: HashMap<VertexId, VertexId> = HashMap::new();
    for (id, v) in src.vertices() {
        vertices.insert(id, m.insert_vertex(v.pos));
    }
    let mut curves: HashMap<CurveId, CurveId> = HashMap::new();
    for (_, e) in src.edges() {
        let mut like = e.clone();
        like.curve = e
            .curve
            .map(|c| *curves.entry(c).or_insert_with(|| m.new_curve()));
        m.insert_edge_like(vertices[&e.v[0]], vertices[&e.v[1]], &like);
    }
    for (_, f) in src.faces() {
        let map = |l: &Vec<VertexId>| l.iter().map(|v| vertices[v]).collect::<Vec<_>>();
        m.insert_face(Face {
            outer: map(&f.outer),
            holes: f.holes.iter().map(map).collect(),
            ..f.clone()
        });
    }
}

/// Where a part carried over was joined to the rest in the model it came from: the positions of
/// the vertices they used in common and the segments of the edges they used in common.
#[derive(Default)]
pub(crate) struct Shared {
    pub points: Vec<DVec3>,
    pub lines: Vec<(DVec3, DVec3)>,
}

impl Shared {
    /// What the faces `a` and `b` of `m` use in common (vertices, and edges both have a face on).
    pub(crate) fn of(m: &Model, a: &[FaceId], b: &[FaceId]) -> Shared {
        let used = |faces: &[FaceId]| -> HashSet<VertexId> {
            faces
                .iter()
                .flat_map(|f| {
                    let f = m.face(*f).expect("face");
                    f.outer.iter().chain(f.holes.iter().flatten()).copied()
                })
                .collect()
        };
        let (ua, ub) = (used(a), used(b));
        let mut shared = Shared::default();
        let common: HashSet<VertexId> = ua.intersection(&ub).copied().collect();
        shared.points = common.iter().map(|v| m.pos(*v)).collect();
        let (fa, fb): (HashSet<FaceId>, HashSet<FaceId>) =
            (a.iter().copied().collect(), b.iter().copied().collect());
        for (_, e) in m.edges() {
            let [x, y] = e.v;
            if common.contains(&x) && common.contains(&y) {
                let on = m.faces_on_side(x, y);
                if on.iter().any(|f| fa.contains(f)) && on.iter().any(|f| fb.contains(f)) {
                    shared.lines.push((m.pos(x), m.pos(y)));
                }
            }
        }
        shared
    }

    fn has_point(&self, p: DVec3) -> bool {
        self.points.iter().any(|q| q.distance(p) < LENGTH_TOL)
    }

    fn on_line(&self, p: DVec3) -> bool {
        self.lines
            .iter()
            .any(|(a, b)| project_on_segment(p, *a, *b).1 < LENGTH_TOL)
    }
}

/// Like `copy_into`, but a vertex of `src` where `m` already has one joins it, and an edge `m`
/// already has stays as it is (its attributes win): a part touching what is in `m` along an edge
/// or at a vertex keeps sharing it. Where the two then overlap along a line, edges are split at
/// the other's vertices lying inside them so they conform (no T-junctions).
#[cfg(test)]
pub(crate) fn copy_merged(src: &Model, m: &mut Model) {
    copy_sharing(src, m, None);
}

/// `copy_merged` for a part that was joined to `m`'s content only as `shared` says: other
/// vertices and edges that merely coincide stay apart, as `copy_into` leaves them.
pub(crate) fn copy_sharing(src: &Model, m: &mut Model, shared: Option<&Shared>) {
    let before: Vec<VertexId> = m.vertices().map(|(id, _)| id).collect();
    let before_edges: Vec<EdgeId> = m.edges().map(|(id, _)| id).collect();
    let mut vertices: HashMap<VertexId, VertexId> = HashMap::new();
    for (id, v) in src.vertices() {
        let found = if shared.is_none_or(|s| s.has_point(v.pos)) {
            m.find_vertex(v.pos)
        } else {
            None
        };
        let at = found.unwrap_or_else(|| m.insert_vertex(v.pos));
        vertices.insert(id, at);
    }
    let mut curves: HashMap<CurveId, CurveId> = HashMap::new();
    let mut added: Vec<EdgeId> = Vec::new();
    for (_, e) in src.edges() {
        let (a, b) = (vertices[&e.v[0]], vertices[&e.v[1]]);
        if let Some(have) = m.edge_between(a, b) {
            added.push(have);
            continue;
        }
        let mut like = e.clone();
        like.curve = e
            .curve
            .map(|c| *curves.entry(c).or_insert_with(|| m.new_curve()));
        added.push(m.insert_edge_like(a, b, &like));
    }
    for (_, f) in src.faces() {
        let map = |l: &Vec<VertexId>| l.iter().map(|v| vertices[v]).collect::<Vec<_>>();
        m.insert_face(Face {
            outer: map(&f.outer),
            holes: f.holes.iter().map(map).collect(),
            ..f.clone()
        });
    }
    if before.is_empty() || shared.is_some_and(|s| s.lines.is_empty()) {
        return;
    }
    let new: Vec<VertexId> = vertices.values().copied().collect();
    let mut edges = before_edges;
    edges.extend(added);
    edges.sort();
    edges.dedup();
    conform(m, &before, &new, &edges, shared);
}

/// Splits the `edges` of `m` at any vertex of `old` or `new` lying inside them (within
/// `LENGTH_TOL`), until none does. A piece that duplicates an existing edge is not added. Only
/// what lies in the region both vertex sets reach, and on the `shared` lines if given, is
/// looked at.
fn conform(
    m: &mut Model,
    old: &[VertexId],
    new: &[VertexId],
    edges: &[EdgeId],
    shared: Option<&Shared>,
) {
    let t = DVec3::splat(LENGTH_TOL);
    let bounds_of = |vs: &[VertexId]| {
        vs.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), v| (lo.min(m.pos(*v)), hi.max(m.pos(*v))),
        )
    };
    let ((olo, ohi), (nlo, nhi)) = (bounds_of(old), bounds_of(new));
    let (lo, hi) = (olo.max(nlo) - t, ohi.min(nhi) + t);
    if lo.cmpgt(hi).any() {
        return;
    }
    let overlaps = |a: DVec3, b: DVec3| a.min(b).cmple(hi).all() && a.max(b).cmpge(lo).all();
    let cand: Vec<VertexId> = old
        .iter()
        .chain(new)
        .copied()
        .filter(|v| overlaps(m.pos(*v), m.pos(*v)) && shared.is_none_or(|s| s.on_line(m.pos(*v))))
        .collect();
    let mut work: Vec<EdgeId> = edges
        .iter()
        .copied()
        .filter(|e| {
            m.edge(*e).is_some_and(|e| {
                let (p, q) = (m.pos(e.v[0]), m.pos(e.v[1]));
                overlaps(p, q) && shared.is_none_or(|s| s.on_line(p) && s.on_line(q))
            })
        })
        .collect();
    while let Some(e) = work.pop() {
        let Some(edge) = m.edge(e) else { continue };
        let [a, b] = edge.v;
        let (pa, pb) = (m.pos(a), m.pos(b));
        let len = pa.distance(pb);
        let (elo, ehi) = (pa.min(pb), pa.max(pb));
        let inside = cand.iter().copied().find(|v| {
            if *v == a || *v == b {
                return false;
            }
            let p = m.pos(*v);
            if p.cmpgt(ehi + t).any() || p.cmplt(elo - t).any() {
                return false;
            }
            let (t, dist) = project_on_segment(p, pa, pb);
            dist < LENGTH_TOL && t * len > LENGTH_TOL && (1.0 - t) * len > LENGTH_TOL
        });
        let Some(v) = inside else { continue };
        let old = m.remove_edge(e);
        for (x, y) in [(a, v), (v, b)] {
            let id = match m.edge_between(x, y) {
                Some(id) => id,
                None => m.insert_edge_like(x, y, &old),
            };
            work.push(id);
        }
        for id in m.faces_on_side(a, b) {
            let mut f = m.face(id).expect("face").clone();
            for l in std::iter::once(&mut f.outer).chain(f.holes.iter_mut()) {
                insert_between(l, a, b, v);
            }
            m.replace_face(id, f);
        }
    }
}

/// Where a face lies against the other solid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Inside,
    Outside,
    /// On its surface, facing the same way.
    Same,
    /// On its surface, facing the other way.
    Opposite,
}

/// How the kept faces of the two cut solids are chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Union,
    /// `a - b`.
    Subtract,
    Intersect,
    /// `b - a`.
    Reverse,
}

impl From<BooleanOp> for Mode {
    fn from(op: BooleanOp) -> Mode {
        match op {
            BooleanOp::Union => Mode::Union,
            BooleanOp::Subtract => Mode::Subtract,
            BooleanOp::Intersect => Mode::Intersect,
        }
    }
}

impl Mode {
    fn keep_a(self, c: Class) -> bool {
        match self {
            Mode::Union => matches!(c, Class::Outside | Class::Same),
            Mode::Intersect => matches!(c, Class::Inside | Class::Same),
            Mode::Subtract => matches!(c, Class::Outside | Class::Opposite),
            Mode::Reverse => c == Class::Inside,
        }
    }

    fn keep_b(self, c: Class) -> bool {
        match self {
            Mode::Union => c == Class::Outside,
            Mode::Intersect | Mode::Subtract => c == Class::Inside,
            Mode::Reverse => matches!(c, Class::Outside | Class::Opposite),
        }
    }
}

/// `a` combined with `b` by `op` (`a - b` for `Subtract`); both closed, outward and in one
/// space. None when nothing is left.
pub(crate) fn combine(a: &Model, b: &Model, op: BooleanOp) -> Result<Option<Model>, BooleanError> {
    Cut::new(a, b)?.make(op.into())
}

/// `a - b`, `a` and `b` common, and `b - a` (in that order; None where nothing is left),
/// from one cutting and classifying pass.
pub(crate) fn split(a: &Model, b: &Model) -> Result<[Option<Model>; 3], BooleanError> {
    let cut = Cut::new(a, b)?;
    Ok([
        cut.make(Mode::Subtract)?,
        cut.make(Mode::Intersect)?,
        cut.make(Mode::Reverse)?,
    ])
}

/// Two solids with each one's faces cut along the other's surface and every face classified
/// against the other solid.
struct Cut<'a> {
    a: &'a Model,
    b: &'a Model,
    wa: Model,
    wb: Model,
    class_a: Vec<(FaceId, Class)>,
    class_b: Vec<(FaceId, Class)>,
}

impl<'a> Cut<'a> {
    fn new(a: &'a Model, b: &'a Model) -> Result<Cut<'a>, BooleanError> {
        let segments = crossings(a, b)?;
        let (wa, wb) = (a.clone(), b.clone());
        // Each solid is cut and classified on its own, the two side by side.
        let cut = |mut w: Model, own: &Model, other: &Model| {
            if !segments.is_empty() {
                w.draw_segments(&segments);
                w.take_changes();
            }
            let classes = classes(&w, own, other);
            (w, classes)
        };
        let ((wa, class_a), (wb, class_b)) = both(|| cut(wa, a, b), || cut(wb, b, a));
        let (class_a, class_b) = (class_a?, class_b?);
        Ok(Cut {
            a,
            b,
            wa,
            wb,
            class_a,
            class_b,
        })
    }

    /// The solid `mode` keeps, welded and verified closed; None when nothing is left.
    fn make(&self, mode: Mode) -> Result<Option<Model>, BooleanError> {
        let pick = |classes: &[(FaceId, Class)], keep: &dyn Fn(Class) -> bool| -> Vec<FaceId> {
            classes
                .iter()
                .filter(|(_, c)| keep(*c))
                .map(|(f, _)| *f)
                .collect()
        };
        let kept_a = pick(&self.class_a, &|c| mode.keep_a(c));
        let kept_b = pick(&self.class_b, &|c| mode.keep_b(c));
        if kept_a.is_empty() && kept_b.is_empty() {
            return Ok(None);
        }
        let mut merge = Merge::default();
        merge.add(&self.wa, &kept_a, mode == Mode::Reverse, 0);
        merge.add(&self.wb, &kept_b, mode == Mode::Subtract, 1);
        let welded = merge.finish();
        if open_reason(&welded.model).is_some() {
            return Err(BooleanError::NotClosed);
        }
        // Seams between coplanar pieces that need not be there are cleaned away; should that
        // ever open the mesh, the seams stay.
        let mut clean = welded.model.clone();
        clean_seams(&mut clean, &welded, [self.a, self.b]);
        let result = if open_reason(&clean).is_none() {
            clean
        } else {
            welded.model
        };
        if signed_volume(&result) <= 0.0 {
            return Err(BooleanError::NotClosed);
        }
        Ok(Some(result))
    }
}

/// `f()` and `g()`, run at the same time on a second thread where one can be started (not in
/// the browser); each gives what it would alone.
fn both<A: Send, B: Send>(f: impl FnOnce() -> A + Send, g: impl FnOnce() -> B + Send) -> (A, B) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // `g` waits here for whichever thread runs it: the second one, or this one when no
        // thread can be started.
        let g = std::sync::Mutex::new(Some(g));
        // What a test forced is forced on the second thread too.
        #[cfg(test)]
        let forced = crate::edit::FORCE_INDEXES.with(|f| f.get());
        let take = || {
            #[cfg(test)]
            crate::edit::FORCE_INDEXES.with(|f| f.set(forced));
            let g = g.lock().unwrap_or_else(|e| e.into_inner()).take();
            g.expect("run once")()
        };
        std::thread::scope(|s| {
            // As much stack as the main thread has, for the deepest meshes.
            let other = std::thread::Builder::new()
                .stack_size(8 << 20)
                .spawn_scoped(s, take);
            let a = f();
            let b = match other {
                Ok(handle) => handle
                    .join()
                    .unwrap_or_else(|e| std::panic::resume_unwind(e)),
                Err(_) => take(),
            };
            (a, b)
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        (f(), g())
    }
}

/// The segments along which the faces of `a` meet the faces of `b`, each once, each at least
/// `LENGTH_TOL` long. Face pairs come from bounding-box overlap.
fn crossings(a: &Model, b: &Model) -> Result<Vec<(DVec3, DVec3)>, BooleanError> {
    crossings_within(a, b, (MAX_PAIRS, MAX_SEGMENTS, MAX_WORK))
}

/// `crossings` refused past `(pairs, segments, work)`.
fn crossings_within(
    a: &Model,
    b: &Model,
    (max_pairs, max_segments, max_work): (usize, usize, u64),
) -> Result<Vec<(DVec3, DVec3)>, BooleanError> {
    let pad = DVec3::splat(LENGTH_TOL);
    let (mut pairs, mut work) = (0usize, 0u64);
    let mut shapes: HashMap<FaceId, Option<FaceShape>> = HashMap::new();
    let mut out = Vec::new();
    let mut seen: HashSet<[i64; 6]> = HashSet::new();
    for (fa, _) in a.faces() {
        let Some(sa) = face_shape(a, fa, DAffine3::IDENTITY) else {
            continue;
        };
        let (lo, hi) = bounds(sa.outer.iter().copied());
        for fb in b.faces_near_box(lo - pad, hi + pad) {
            let sb = shapes
                .entry(fb)
                .or_insert_with(|| face_shape(b, fb, DAffine3::IDENTITY));
            let Some(sb) = sb else { continue };
            pairs += 1;
            work = work.saturating_add(face_segments_cost(&sa, sb));
            if pairs > max_pairs || work > max_work {
                return Err(BooleanError::TooMuch);
            }
            for s in face_segments(&sa, sb) {
                if s.0.distance(s.1) >= LENGTH_TOL && seen.insert(segment_key(s)) {
                    out.push(s);
                    if out.len() > max_segments {
                        return Err(BooleanError::TooMuch);
                    }
                }
            }
        }
    }
    Ok(out)
}

/// The class against the solid `other` of each face of `m` (the solid `own` with its faces
/// cut), in id order. Faces the cutting made off the surface of `own` (a cut loop that closed
/// a region of its own) are left out.
fn classes(m: &Model, own: &Model, other: &Model) -> Result<Vec<(FaceId, Class)>, BooleanError> {
    let (lo, hi) = bounds(other.vertices().map(|(_, v)| v.pos));
    let pad = DVec3::splat(10.0 * LENGTH_TOL);
    let mut out = Vec::new();
    for (id, f) in m.faces() {
        let (flo, fhi) = bounds(f.outer.iter().map(|v| m.pos(*v)));
        let class = if flo.cmpgt(hi + pad).any() || fhi.cmplt(lo - pad).any() {
            Class::Outside
        } else {
            let p = interior_point(m, id).ok_or(BooleanError::NotClosed)?;
            if surface_normal(p, own).is_none() {
                continue;
            }
            classify(p, f.normal, other, true).ok_or(BooleanError::NotClosed)?
        };
        out.push((id, class));
    }
    Ok(out)
}

/// A point well inside face `f`: the centroid of its triangle with the largest inscribed
/// circle.
pub(crate) fn interior_point(m: &Model, f: FaceId) -> Option<DVec3> {
    m.triangulate(f)?
        .iter()
        .filter_map(|[a, b, c]| {
            let area2 = (*b - *a).cross(*c - *a).length();
            let perimeter = a.distance(*b) + b.distance(*c) + c.distance(*a);
            let r = area2 / perimeter;
            (r.is_finite() && r > 0.0).then(|| (r, (*a + *b + *c) / 3.0))
        })
        .max_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, p)| p)
}

/// Ray directions for the parity test: unrelated to the axes and to each other, so a ray rarely
/// runs along a face or through an edge of a modelled solid.
const DIRECTIONS: [[f64; 3]; 9] = [
    [0.5773, 0.3511, 0.7369],
    [-0.4317, 0.8123, 0.3921],
    [0.2213, -0.6347, 0.7403],
    [-0.7121, -0.2531, -0.6547],
    [0.8317, 0.4483, -0.3271],
    [-0.1829, 0.9137, -0.3627],
    [0.6911, -0.7013, 0.1747],
    [-0.3343, -0.5171, 0.7879],
    [0.1123, 0.2381, -0.9647],
];

/// The cosine above which two unit normals count as parallel for `classify`.
const COPLANAR_DOT: f64 = 0.999;

/// Hits nearer than this to a face's boundary make a ray ambiguous.
const GRAZE: f64 = 4.0 * LENGTH_TOL;

/// Where `p` (on a face of normal `n`) lies against the closed mesh `s`: on its surface when a
/// face of it holds `p`, else inside or outside by the parity of the faces rays from `p`
/// cross, the majority of the first agreeing rays that graze no edge. None when no ray is
/// clear. With `coplanar` set, `p` counts as on the surface only where the face holding it is
/// parallel to `n` (a sliver touching the surface along a crease is inside or outside, and
/// its sign of `n . normal` is noise); else any face holding `p` decides.
fn classify(p: DVec3, n: DVec3, s: &Model, coplanar: bool) -> Option<Class> {
    if let Some(normal) = surface_normal(p, s)
        && (!coplanar || n.dot(normal).abs() > COPLANAR_DOT)
    {
        return Some(if n.dot(normal) > 0.0 {
            Class::Same
        } else {
            Class::Opposite
        });
    }
    let (mut inside, mut outside) = (0, 0);
    for d in DIRECTIONS {
        match parity(p, DVec3::from(d).normalize(), s) {
            Some(true) => inside += 1,
            Some(false) => outside += 1,
            None => {}
        }
        if inside >= 2 || outside >= 2 {
            break;
        }
    }
    match inside.cmp(&outside) {
        std::cmp::Ordering::Greater => Some(Class::Inside),
        std::cmp::Ordering::Less => Some(Class::Outside),
        std::cmp::Ordering::Equal => None,
    }
}

/// Whether `p` lies inside the closed mesh `s` (by the parity of rays, as `classify`); None
/// when it lies on its surface or no ray is clear.
pub(crate) fn point_inside(p: DVec3, s: &Model) -> Option<bool> {
    match classify(p, DVec3::Z, s, false)? {
        Class::Inside => Some(true),
        Class::Outside => Some(false),
        Class::Same | Class::Opposite => None,
    }
}

/// The normal of a face of `s` holding the point `p` (within `LENGTH_TOL` of its plane and
/// inside its loops), if one does.
fn surface_normal(p: DVec3, s: &Model) -> Option<DVec3> {
    let pad = DVec3::splat(2.0 * LENGTH_TOL);
    for f in s.faces_near_box(p - pad, p + pad) {
        let Some(face) = s.face(f) else { continue };
        let v0 = s.pos(face.outer[0]);
        if (p - v0).dot(face.normal).abs() >= LENGTH_TOL {
            continue;
        }
        let plane = Plane::through(v0, face.normal);
        let flat = |l: &[VertexId]| {
            l.iter()
                .map(|v| plane.project(s.pos(*v)))
                .collect::<Vec<_>>()
        };
        let q = plane.project(p);
        if point_in_polygon(q, &flat(&face.outer))
            && !face.holes.iter().any(|h| point_in_polygon(q, &flat(h)))
        {
            return Some(face.normal);
        }
    }
    None
}

/// Whether the ray from `p` along `d` crosses an odd number of the faces of `s`; None when it
/// grazes a face (runs nearly along it, starts on it or passes near its boundary).
fn parity(p: DVec3, d: DVec3, s: &Model) -> Option<bool> {
    let mut count = 0;
    for (f, t) in s.index().ray_hits(p, d, 0.0, f64::INFINITY) {
        let face = s.face(f)?;
        if t < GRAZE || d.dot(face.normal).abs() < 1e-6 {
            return None;
        }
        let h = p + d * t;
        let near = std::iter::once(&face.outer).chain(&face.holes).any(|l| {
            (0..l.len())
                .any(|i| segment_distance(h, s.pos(l[i]), s.pos(l[(i + 1) % l.len()])) < GRAZE)
        });
        if near {
            return None;
        }
        count += 1;
    }
    Some(count % 2 == 1)
}

/// The faces kept from both solids, welded into one mesh.
#[derive(Default)]
struct Merge {
    /// Per kept face: its loops as points, its attributes (turned over as needed), which
    /// operand it came from, and per loop side the source edge's attributes.
    faces: Vec<Piece>,
}

struct Piece {
    loops: Vec<Vec<DVec3>>,
    /// The source edge of each loop side, per loop: (attributes, curve keyed by operand).
    sides: Vec<Vec<Option<Edge>>>,
    face: Face,
    operand: usize,
}

/// The welded mesh and, per face, the operand it came from.
struct Welded {
    model: Model,
    operand: HashMap<FaceId, usize>,
}

impl Merge {
    fn add(&mut self, m: &Model, faces: &[FaceId], flip: bool, operand: usize) {
        for &id in faces {
            let f = m.face(id).expect("kept faces exist");
            let mut loops: Vec<Vec<VertexId>> = std::iter::once(f.outer.clone())
                .chain(f.holes.clone())
                .collect();
            let mut face = f.clone();
            if flip {
                // Turned over, keeping each material on the side it is named for: the tool's
                // front material shows on the cut.
                loops.iter_mut().for_each(|l| l.reverse());
                face.normal = -face.normal;
                // The texture shows on the front as it did, as `Face::reversed` keeps it.
                face.texture = face.texture.map(|t| t.turned_over(is_level(f.normal)));
            }
            let sides = loops
                .iter()
                .map(|l| {
                    (0..l.len())
                        .map(|i| {
                            let (a, b) = (l[i], l[(i + 1) % l.len()]);
                            m.edge_between(a, b).and_then(|e| m.edge(e)).map(|e| {
                                let mut e = e.clone();
                                // Curves of the two operands never share an id.
                                e.curve = e.curve.map(|c| CurveId(c.0 * 2 + operand as u64));
                                e
                            })
                        })
                        .collect()
                })
                .collect();
            let loops = loops
                .iter()
                .map(|l| l.iter().map(|v| m.pos(*v)).collect())
                .collect();
            self.faces.push(Piece {
                loops,
                sides,
                face,
                operand,
            });
        }
    }

    /// One mesh of the pieces: points within `LENGTH_TOL` become one vertex, a vertex lying on
    /// a loop side is put into it, and edges keep the attributes of their source edges (new
    /// cut edges are hard; an edge is soft only if every source of it is).
    fn finish(self) -> Welded {
        let mut m = Model::new();
        let ids: Vec<Vec<Vec<VertexId>>> = self
            .faces
            .iter()
            .map(|piece| {
                piece
                    .loops
                    .iter()
                    .map(|l| {
                        l.iter()
                            .map(|p| m.find_vertex(*p).unwrap_or_else(|| m.insert_vertex(*p)))
                            .collect()
                    })
                    .collect()
            })
            .collect();
        let points = Index::of_points(m.vertices().map(|(id, v)| (id.0, v.pos)));
        let mut curves: HashMap<CurveId, CurveId> = HashMap::new();
        let mut operand = HashMap::new();
        for (piece, loops) in self.faces.iter().zip(ids) {
            let mut out: Vec<Vec<VertexId>> = Vec::new();
            for (k, l) in loops.iter().enumerate() {
                let mut ring: Vec<VertexId> = Vec::new();
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    let mut chain = vec![a];
                    if a != b {
                        chain.extend(on_side(&m, &points, a, b));
                    }
                    for w in chain
                        .iter()
                        .copied()
                        .chain([b])
                        .collect::<Vec<_>>()
                        .windows(2)
                    {
                        let (u, v) = (w[0], w[1]);
                        if u == v {
                            continue;
                        }
                        let like = piece.sides[k][i].clone();
                        match m.edge_between(u, v) {
                            Some(e) => {
                                let have = m.edge(e).expect("found").clone();
                                let soft = have.soft && like.as_ref().is_some_and(|l| l.soft);
                                if soft != have.soft {
                                    m.set_edge(e, have.curve, soft, have.polygon);
                                }
                            }
                            None => {
                                let mut like = like.unwrap_or_else(|| Edge::new(u, v));
                                like.curve = like
                                    .curve
                                    .map(|c| *curves.entry(c).or_insert_with(|| m.new_curve()));
                                m.insert_edge_like(u, v, &like);
                            }
                        }
                    }
                    ring.extend(chain);
                }
                ring.dedup();
                while ring.len() > 1 && ring.first() == ring.last() {
                    ring.pop();
                }
                if ring.len() >= 3 {
                    out.push(ring);
                } else if k == 0 {
                    break;
                }
            }
            if out.is_empty() {
                continue;
            }
            let outer = out.remove(0);
            let id = m.insert_face(Face {
                outer,
                holes: out,
                ..piece.face.clone()
            });
            operand.insert(id, piece.operand);
        }
        m.take_changes();
        Welded { model: m, operand }
    }
}

/// The vertices lying on the open segment a-b (within `LENGTH_TOL`), ordered from a to b.
fn on_side(m: &Model, points: &Index, a: VertexId, b: VertexId) -> Vec<VertexId> {
    let (pa, pb) = (m.pos(a), m.pos(b));
    let ab = pb - pa;
    let l2 = ab.length_squared();
    let mut found: Vec<(f64, VertexId)> = Vec::new();
    points.near_segment(pa, pb, LENGTH_TOL, |s| {
        if let Shape::Point(id, p) = *s
            && id != a
            && id != b
        {
            let t = (p - pa).dot(ab) / l2;
            if t > 0.0 && t < 1.0 && (pa + ab * t).distance(p) < LENGTH_TOL {
                found.push((t, id));
            }
        }
    });
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    found.into_iter().map(|(_, id)| id).collect()
}

/// Erases the hard edges between two coplanar faces facing the same way and painted, textured,
/// tagged and hidden alike that a boolean made: where the faces came from different operands,
/// or where the edge lies along no edge of its operand (a cut that split one face into pieces
/// that both stayed). The two faces merge into one.
fn clean_seams(m: &mut Model, welded: &Welded, operands: [&Model; 2]) {
    let mut seams: Vec<(VertexId, VertexId)> = Vec::new();
    for (_, e) in m.edges() {
        let [a, b] = e.v;
        if e.soft || e.hidden {
            continue;
        }
        let [f, g] = m.faces_on_side(a, b)[..] else {
            continue;
        };
        if !mergeable(m, f, g) {
            continue;
        }
        let (of, og) = (welded.operand.get(&f), welded.operand.get(&g));
        let made = match (of, og) {
            (Some(x), Some(y)) if x != y => true,
            (Some(x), Some(_)) => !along_edge(operands[*x], m.pos(a), m.pos(b)),
            _ => false,
        };
        if made {
            seams.push((a, b));
        }
    }
    for (a, b) in seams {
        let Some(e) = m.edge_between(a, b) else {
            continue;
        };
        match m.faces_on_side(a, b)[..] {
            [f, g] if mergeable(m, f, g) => m.erase_pending(&[e], &[]),
            // An earlier merge (of a closed or U-shaped seam) took both faces: the rest of
            // the seam is left bounding nothing.
            [] => m.erase_pending(&[e], &[]),
            _ => {}
        }
    }
    m.take_changes();
}

/// Whether faces `f` and `g` lie in one plane, face the same way and look alike.
fn mergeable(m: &Model, f: FaceId, g: FaceId) -> bool {
    let (Some(ff), Some(gg)) = (m.face(f), m.face(g)) else {
        return false;
    };
    f != g
        && ff.normal.dot(gg.normal) > 0.0
        && m.shared_plane(f, g).is_some()
        && ff.paint() == gg.paint()
        && ff.texture == gg.texture
        && ff.tag == gg.tag
        && ff.hidden == gg.hidden
}

/// Whether the segment p-q lies along an edge of `m`.
fn along_edge(m: &Model, p: DVec3, q: DVec3) -> bool {
    let mid = (p + q) * 0.5;
    let dir = (q - p).normalize_or_zero();
    let mut found = false;
    m.index().near_segment(mid, mid, LENGTH_TOL, |s| {
        if let Shape::Segment(_, a, b) = *s
            && segment_distance(mid, a, b) < LENGTH_TOL
            && (b - a).normalize_or_zero().cross(dir).length() < 1e-6
        {
            found = true;
        }
    });
    found
}

/// Distance from `p` to the closed segment a-b.
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

fn bounds(points: impl Iterator<Item = DVec3>) -> (DVec3, DVec3) {
    points.fold((DVec3::INFINITY, DVec3::NEG_INFINITY), |(lo, hi), p| {
        (lo.min(p), hi.max(p))
    })
}

/// The segment's endpoints on a grid of `LENGTH_TOL`, smaller first, so equal segments either
/// way round share a key.
fn segment_key((a, b): (DVec3, DVec3)) -> [i64; 6] {
    let q = |p: DVec3| (p / LENGTH_TOL).round().to_array().map(|c| c as i64);
    let (a, b) = (q(a), q(b));
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::*;
    use crate::primitives::Solid;

    fn face_ids(m: &Model) -> Vec<FaceId> {
        m.faces().map(|(id, _)| id).collect()
    }

    fn solid(s: Solid, at: DVec3) -> Model {
        placed_copy(&s.to_model(), DAffine3::from_translation(at), None)
    }

    fn cuboid(size: [f64; 3], at: [f64; 3]) -> Model {
        solid(Solid::cuboid(DVec3::from(size)).unwrap(), DVec3::from(at))
    }

    fn closed(m: &Model) {
        assert_eq!(open_reason(m), None);
        m.validate().unwrap();
    }

    /// The area of a regular `n`-gon of circumradius `r`.
    fn ngon(r: f64, n: u32) -> f64 {
        0.5 * n as f64 * r * r * (TAU / n as f64).sin()
    }

    #[test]
    fn a_pocket_is_cut_from_a_face() {
        let a = cuboid([4.0, 4.0, 2.0], [0.0; 3]);
        let b = cuboid([2.0, 2.0, 2.0], [1.0, 1.0, 1.0]);
        let r = combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap();
        closed(&r);
        assert!((signed_volume(&r) - 28.0).abs() < 1e-9);
        // The top with a square hole, the four pocket walls and its floor.
        assert_eq!(r.faces().count(), 6 + 5);
    }

    #[test]
    fn a_flush_pocket_has_no_lid() {
        let a = cuboid([4.0, 4.0, 2.0], [0.0; 3]);
        let b = cuboid([2.0, 2.0, 1.0], [1.0, 1.0, 1.0]);
        let r = combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap();
        closed(&r);
        assert!((signed_volume(&r) - 28.0).abs() < 1e-9);
        assert_eq!(r.faces().count(), 11);
    }

    #[test]
    fn a_union_of_boxes_sharing_a_face_drops_it() {
        let a = cuboid([1.0, 1.0, 1.0], [0.0; 3]);
        let b = cuboid([1.0, 1.0, 1.0], [1.0, 0.0, 0.0]);
        let r = combine(&a, &b, BooleanOp::Union).unwrap().unwrap();
        closed(&r);
        assert!((signed_volume(&r) - 2.0).abs() < 1e-9);
        // The seams between the two boxes' coplanar sides are cleaned away.
        assert_eq!(r.faces().count(), 6);
    }

    #[test]
    fn a_bore_goes_through_a_box() {
        let a = cuboid([2.0, 2.0, 1.0], [0.0; 3]);
        let b = solid(
            Solid::cylinder(0.5, 3.0, 24).unwrap(),
            DVec3::new(1.0, 1.0, -1.0),
        );
        let r = combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap();
        closed(&r);
        assert!((signed_volume(&r) - (4.0 - ngon(0.5, 24))).abs() < 1e-9);
        // The bore's walls stay soft between each other and hard at the rims.
        let soft = r.edges().filter(|(_, e)| e.soft).count();
        assert_eq!(soft, 24);
    }

    #[test]
    fn crossing_cylinders_intersect() {
        let a = solid(
            Solid::cylinder(1.0, 4.0, 24).unwrap(),
            DVec3::new(0.0, 0.0, -2.0),
        );
        let turn = DAffine3::from_translation(DVec3::new(-2.0, 0.0, 0.0))
            * DAffine3::from_rotation_y(TAU / 4.0);
        let b = placed_copy(
            &Solid::cylinder(0.8, 4.0, 24).unwrap().to_model(),
            turn,
            None,
        );
        let mut volumes = Vec::new();
        for op in [BooleanOp::Intersect, BooleanOp::Union, BooleanOp::Subtract] {
            let r = combine(&a, &b, op).unwrap().unwrap();
            closed(&r);
            volumes.push(signed_volume(&r));
        }
        let [common, union, rest] = volumes[..] else {
            unreachable!()
        };
        let (va, vb) = (signed_volume(&a), signed_volume(&b));
        assert!(common > 0.5 && common < vb, "{common}");
        assert!((va + vb - union - common).abs() < 1e-9);
        assert!((va - rest - common).abs() < 1e-9);
    }

    #[test]
    fn equal_crossing_cylinders_make_a_steinmetz_solid() {
        let a = solid(
            Solid::cylinder(1.0, 4.0, 24).unwrap(),
            DVec3::new(0.0, 0.0, -2.0),
        );
        let turn = DAffine3::from_translation(DVec3::new(-2.0, 0.0, 0.0))
            * DAffine3::from_rotation_y(TAU / 4.0);
        let b = placed_copy(
            &Solid::cylinder(1.0, 4.0, 24).unwrap().to_model(),
            turn,
            None,
        );
        let r = combine(&a, &b, BooleanOp::Intersect).unwrap().unwrap();
        closed(&r);
        // Near the Steinmetz solid's 16/3 r^3, a little less for the facets.
        let v = signed_volume(&r);
        assert!(v < 16.0 / 3.0 && v > 0.95 * 16.0 / 3.0, "{v}");
    }

    #[test]
    fn tools_apart_are_cut_in_one_pass() {
        let tools: Vec<Model> = (0..3)
            .map(|i| cuboid([0.5, 0.5, 3.0], [0.25 + i as f64, 0.25, -1.0]))
            .chain([cuboid([0.5, 0.5, 3.0], [0.5, 0.5, -1.0])])
            .collect();
        let groups = apart(tools);
        // The fourth meets the first and starts a group of its own.
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].faces().count(), 18);
        let mut r = cuboid([3.0, 1.0, 1.0], [0.0; 3]);
        for g in &groups {
            r = combine(&r, g, BooleanOp::Subtract).unwrap().unwrap();
        }
        closed(&r);
        // Three 0.25 m² holes, the fourth overlapping the first by a quarter of its area.
        let removed = 3.0 * 0.25 + 0.25 - 0.0625;
        assert!((signed_volume(&r) - (3.0 - removed)).abs() < 1e-9);
    }

    #[test]
    fn a_box_cuts_a_sphere_in_half() {
        let s = Solid::sphere(1.0, 24).unwrap().to_model();
        let whole = signed_volume(&s);
        let a = placed_copy(&s, DAffine3::IDENTITY, None);
        let b = cuboid([4.0, 4.0, 2.0], [-2.0, -2.0, 0.0]);
        let r = combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap();
        closed(&r);
        assert!((signed_volume(&r) - whole / 2.0).abs() < 1e-9);
        // Through the faces, not along the equator.
        let c = cuboid([4.0, 4.0, 2.0], [-2.0, -2.0, 0.3]);
        let r = combine(&a, &c, BooleanOp::Subtract).unwrap().unwrap();
        closed(&r);
        let v = signed_volume(&r);
        assert!(v > whole / 2.0 && v < whole, "{v}");
    }

    #[test]
    fn disjoint_and_swallowed_results_are_empty() {
        let a = cuboid([1.0, 1.0, 1.0], [0.0; 3]);
        let far = cuboid([1.0, 1.0, 1.0], [5.0, 0.0, 0.0]);
        assert_eq!(combine(&a, &far, BooleanOp::Intersect).unwrap(), None);
        let big = cuboid([3.0, 3.0, 3.0], [-1.0; 3]);
        assert_eq!(combine(&a, &big, BooleanOp::Subtract).unwrap(), None);
        // Touching along a face only: no common volume.
        let next = cuboid([1.0, 1.0, 1.0], [1.0, 0.0, 0.0]);
        assert_eq!(combine(&a, &next, BooleanOp::Intersect).unwrap(), None);
        // A disjoint union keeps both.
        let r = combine(&a, &far, BooleanOp::Union).unwrap().unwrap();
        closed(&r);
        assert!((signed_volume(&r) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn boxes_meeting_along_an_edge_are_refused() {
        let a = cuboid([1.0, 1.0, 1.0], [0.0; 3]);
        let b = cuboid([1.0, 1.0, 1.0], [1.0, 1.0, 0.0]);
        // The union would have an edge bounding four faces: not a solid.
        assert_eq!(
            combine(&a, &b, BooleanOp::Union),
            Err(BooleanError::NotClosed)
        );
        // Meeting at a corner only is fine.
        let c = cuboid([1.0, 1.0, 1.0], [1.0, 1.0, 1.0]);
        let r = combine(&a, &c, BooleanOp::Union).unwrap().unwrap();
        closed(&r);
        assert!((signed_volume(&r) - 2.0).abs() < 1e-9);
        assert_eq!(combine(&a, &b, BooleanOp::Intersect).unwrap(), None);
        let r = combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap();
        assert!((signed_volume(&r) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn seams_from_a_touching_tool_are_cleaned_away() {
        let unit = || cuboid([1.0, 1.0, 1.0], [0.0; 3]);
        // A tool outside touching the middle of a face: a closed seam loop.
        let touch = cuboid([1.0, 0.5, 0.5], [1.0, 0.25, 0.25]);
        let r = combine(&unit(), &touch, BooleanOp::Subtract)
            .unwrap()
            .unwrap();
        closed(&r);
        assert_eq!((r.faces().count(), r.edges().count()), (6, 12));
        assert!((signed_volume(&r) - 1.0).abs() < 1e-9);
        // Reaching the face's edge: a U-shaped seam.
        let edge = cuboid([1.0, 0.5, 0.5], [1.0, 0.25, 0.5]);
        let r = combine(&unit(), &edge, BooleanOp::Subtract)
            .unwrap()
            .unwrap();
        closed(&r);
        assert_eq!((r.faces().count(), r.edges().count()), (6, 12));
        // A box flush inside, top faces coplanar: the union is the outer box.
        let inner = cuboid([0.5, 0.5, 0.5], [0.25, 0.25, 0.5]);
        let r = combine(&unit(), &inner, BooleanOp::Union).unwrap().unwrap();
        closed(&r);
        assert_eq!((r.faces().count(), r.edges().count()), (6, 12));
        assert!((signed_volume(&r) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn too_many_crossings_are_refused() {
        let a = cuboid([2.0, 2.0, 1.0], [0.0; 3]);
        let b = solid(
            Solid::cylinder(0.5, 3.0, 24).unwrap(),
            DVec3::new(1.0, 1.0, -1.0),
        );
        for limits in [
            (3, MAX_SEGMENTS, MAX_WORK),
            (MAX_PAIRS, 3, MAX_WORK),
            (MAX_PAIRS, MAX_SEGMENTS, 3),
        ] {
            assert_eq!(crossings_within(&a, &b, limits), Err(BooleanError::TooMuch));
        }
        assert!(crossings_within(&a, &b, (MAX_PAIRS, MAX_SEGMENTS, MAX_WORK)).is_ok());
    }

    #[test]
    fn a_flipped_tool_face_keeps_its_texture_look() {
        let a = cuboid([4.0, 4.0, 2.0], [0.0; 3]);
        let mut tool = Solid::cuboid(DVec3::splat(2.0)).unwrap().to_model();
        let placement = TexturePlacement {
            offset: [0.25, 0.5],
            rotation_deg: 30.0,
            scale: [2.0, 1.0],
        };
        for f in face_ids(&tool) {
            tool.set_paint(f, [Some(MaterialId(3)), None]);
            tool.set_texture(f, Some(placement));
        }
        tool.take_changes();
        let b = placed_copy(
            &tool,
            DAffine3::from_translation(DVec3::new(1.0, 1.0, 1.0)),
            None,
        );
        let moved: HashMap<[i64; 3], Face> = b
            .faces()
            .map(|(_, f)| {
                (
                    (f.normal * 10.0).round().to_array().map(|c| c as i64),
                    f.clone(),
                )
            })
            .collect();
        let r = combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap();
        let cut: Vec<&Face> = r
            .faces()
            .map(|(_, f)| f)
            .filter(|f| f.front == Some(MaterialId(3)))
            .collect();
        assert_eq!(cut.len(), 5);
        for f in cut {
            let key = (-f.normal * 10.0).round().to_array().map(|c| c as i64);
            let tool_face = &moved[&key];
            assert!(tool_face.texture.is_some());
            assert_eq!(
                f.texture,
                tool_face
                    .texture
                    .map(|t| t.turned_over(is_level(tool_face.normal)))
            );
        }
    }

    #[test]
    fn a_tool_shows_its_front_material_on_the_cut() {
        let mut a = cuboid([4.0, 4.0, 2.0], [0.0; 3]);
        let all: Vec<FaceId> = face_ids(&a);
        for f in &all {
            a.set_paint(*f, [Some(MaterialId(1)), Some(MaterialId(2))]);
        }
        a.take_changes();
        let b = placed_copy(
            &Solid::cuboid(DVec3::splat(2.0)).unwrap().to_model(),
            DAffine3::from_translation(DVec3::new(1.0, 1.0, 1.0)),
            Some(MaterialId(3)),
        );
        let r = combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap();
        closed(&r);
        let pocket: Vec<&Face> = r
            .faces()
            .map(|(_, f)| f)
            .filter(|f| f.front == Some(MaterialId(3)))
            .collect();
        assert_eq!(pocket.len(), 5);
        // The pocket's floor faces up, out of the remaining solid.
        assert!(pocket.iter().any(|f| f.normal.z > 0.99));
        assert!(
            r.faces()
                .map(|(_, f)| f)
                .filter(|f| f.front != Some(MaterialId(3)))
                .all(|f| f.paint() == [Some(MaterialId(1)), Some(MaterialId(2))])
        );
    }

    #[test]
    fn a_split_makes_three_pieces_that_add_up() {
        let a = cuboid([2.0, 2.0, 2.0], [0.0; 3]);
        let b = cuboid([2.0, 2.0, 2.0], [1.0, 1.0, 1.0]);
        let [only_a, both, only_b] = split(&a, &b).unwrap();
        let volumes: Vec<f64> = [only_a, both, only_b]
            .iter()
            .map(|p| {
                let p = p.as_ref().unwrap();
                closed(p);
                signed_volume(p)
            })
            .collect();
        assert!((volumes[0] - 7.0).abs() < 1e-9, "{volumes:?}");
        assert!((volumes[1] - 1.0).abs() < 1e-9, "{volumes:?}");
        assert!((volumes[2] - 7.0).abs() < 1e-9, "{volumes:?}");
        let union = combine(&a, &b, BooleanOp::Union).unwrap().unwrap();
        assert!((volumes.iter().sum::<f64>() - signed_volume(&union)).abs() < 1e-9);
    }

    #[test]
    fn a_split_without_overlap_keeps_both_whole() {
        let a = cuboid([1.0, 1.0, 1.0], [0.0; 3]);
        let b = cuboid([1.0, 1.0, 1.0], [5.0, 0.0, 0.0]);
        let [only_a, both, only_b] = split(&a, &b).unwrap();
        assert!(both.is_none());
        assert!((signed_volume(&only_a.unwrap()) - 1.0).abs() < 1e-9);
        assert!((signed_volume(&only_b.unwrap()) - 1.0).abs() < 1e-9);
        // Equal solids: only the common part.
        let [x, y, z] = split(&a, &a.clone()).unwrap();
        assert!(x.is_none() && z.is_none());
        assert!((signed_volume(&y.unwrap()) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn solids_stacked_face_to_face_stay_one_shell() {
        // Two cubes with the same vertices on the common face, which both keep.
        let a = cuboid([2.0, 2.0, 2.0], [0.0; 3]);
        let b = cuboid([2.0, 2.0, 2.0], [0.0, 0.0, 2.0]);
        let mut m = Model::new();
        copy_merged(&a, &mut m);
        copy_merged(&b, &mut m);
        assert_eq!(m.faces().count(), 12);
        assert_eq!(shells_of(&m).len(), 1);
        // Their outer shell is as it was.
        assert_eq!(outer_shells(&m).unwrap().faces().count(), 12);
    }

    #[test]
    fn the_outer_shell_drops_a_void() {
        let a = cuboid([4.0, 4.0, 4.0], [0.0; 3]);
        let b = cuboid([1.0, 1.0, 1.0], [1.5, 1.5, 1.5]);
        let hollow = combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap();
        closed(&hollow);
        assert!((signed_volume(&hollow) - 63.0).abs() < 1e-9);
        assert_eq!(hollow.faces().count(), 12);
        let shell = outer_shells(&hollow).unwrap();
        closed(&shell);
        assert_eq!(shell.faces().count(), 6);
        assert!((signed_volume(&shell) - 64.0).abs() < 1e-9);
        // A solid with no void is as it was; so are solids apart.
        let far = cuboid([1.0, 1.0, 1.0], [9.0, 0.0, 0.0]);
        let both = combine(&shell, &far, BooleanOp::Union).unwrap().unwrap();
        assert_eq!(outer_shells(&both).unwrap().faces().count(), 12);
    }

    #[test]
    fn the_outer_shell_drops_a_floating_solid_and_nested_voids() {
        let hollow = |size: f64, at: f64, inner: f64| {
            let a = cuboid([size; 3], [at; 3]);
            let b = cuboid([inner; 3], [at + (size - inner) / 2.0; 3]);
            combine(&a, &b, BooleanOp::Subtract).unwrap().unwrap()
        };
        // A cube floating in the void of a hollow box.
        let cube = cuboid([0.5, 0.5, 0.5], [1.75; 3]);
        let both = combine(&hollow(4.0, 0.0, 2.0), &cube, BooleanOp::Union)
            .unwrap()
            .unwrap();
        closed(&both);
        assert_eq!(both.faces().count(), 12 + 6);
        let shell = outer_shells(&both).unwrap();
        closed(&shell);
        assert_eq!(shell.faces().count(), 6);
        assert!((signed_volume(&shell) - 64.0).abs() < 1e-9);
        // A void in a solid floating in a void: the outer box only.
        let nested = combine(
            &hollow(4.0, 0.0, 3.0),
            &hollow(1.5, 1.25, 0.5),
            BooleanOp::Union,
        )
        .unwrap()
        .unwrap();
        closed(&nested);
        assert_eq!(nested.faces().count(), 6 + 6 + 6 + 6);
        let shell = outer_shells(&nested).unwrap();
        assert_eq!(shell.faces().count(), 6);
        assert!((signed_volume(&shell) - 64.0).abs() < 1e-9);
    }

    #[test]
    fn a_split_piece_of_b_shows_b_outside_and_a_on_the_cut() {
        let mut a = cuboid([2.0, 2.0, 2.0], [0.0; 3]);
        for f in face_ids(&a) {
            a.set_paint(f, [Some(MaterialId(1)), Some(MaterialId(2))]);
        }
        a.take_changes();
        let b = placed_copy(
            &Solid::cuboid(DVec3::splat(2.0)).unwrap().to_model(),
            DAffine3::from_translation(DVec3::splat(1.0)),
            Some(MaterialId(3)),
        );
        let [_, _, only_b] = split(&a, &b).unwrap();
        let only_b = only_b.unwrap();
        closed(&only_b);
        let (mut own, mut cut) = (0, 0);
        for (_, f) in only_b.faces() {
            match f.paint() {
                [Some(MaterialId(3)), Some(MaterialId(3))] => own += 1,
                [Some(MaterialId(1)), Some(MaterialId(2))] => cut += 1,
                other => panic!("{other:?}"),
            }
        }
        // b's three outer sides (with a notch) and a's three faces turned over.
        assert_eq!((own, cut), (own, 3));
        assert!(own >= 3);
    }

    #[test]
    fn an_inward_or_mirrored_operand_is_turned_outward() {
        let mut inward = Solid::cuboid(DVec3::ONE).unwrap().to_model();
        let all: Vec<FaceId> = face_ids(&inward);
        inward.reverse_faces(&all);
        let a = placed_copy(&inward, DAffine3::IDENTITY, None);
        assert!(signed_volume(&a) > 0.0);
        let mirrored = placed_copy(
            &Solid::cuboid(DVec3::ONE).unwrap().to_model(),
            DAffine3::from_scale(DVec3::new(-1.0, 1.0, 1.0)),
            None,
        );
        closed(&mirrored);
        assert!((signed_volume(&mirrored) - 1.0).abs() < 1e-9);
        let r = combine(&a, &mirrored, BooleanOp::Union).unwrap().unwrap();
        closed(&r);
        assert!((signed_volume(&r) - 2.0).abs() < 1e-9);
    }
}
