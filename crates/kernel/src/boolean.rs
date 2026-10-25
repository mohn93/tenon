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

use crate::geom::{LENGTH_TOL, Plane, point_in_polygon};
use crate::ids::{CurveId, FaceId, InstanceId, MaterialId, VertexId};
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
            match classify(p.0, p.1, other).ok_or(BooleanError::NotClosed)? {
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

/// The faces of `m` grouped into shells: faces joined across the edges of their loops.
fn shells_of(m: &Model) -> Vec<Vec<FaceId>> {
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
    for (i, id) in ids.iter().enumerate() {
        let f = m.face(*id).expect("face");
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            for k in 0..l.len() {
                for g in m.faces_on_side(l[k], l[(k + 1) % l.len()]) {
                    let (a, b) = (root(&mut parent, i), root(&mut parent, at[&g]));
                    parent[a] = b;
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
            classify(p, f.normal, other).ok_or(BooleanError::NotClosed)?
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

/// Hits nearer than this to a face's boundary make a ray ambiguous.
const GRAZE: f64 = 4.0 * LENGTH_TOL;

/// Where `p` (on a face of normal `n`) lies against the closed mesh `s`: on its surface when a
/// face of it holds `p`, else inside or outside by the parity of the faces rays from `p`
/// cross, the majority of the first agreeing rays that graze no edge. None when no ray is
/// clear.
fn classify(p: DVec3, n: DVec3, s: &Model) -> Option<Class> {
    if let Some(normal) = surface_normal(p, s) {
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
    match classify(p, DVec3::Z, s)? {
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
