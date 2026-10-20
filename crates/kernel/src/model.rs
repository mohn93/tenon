use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;
use std::sync::OnceLock;

use glam::{DAffine3, DVec3};
use serde::{Deserialize, Serialize};

use crate::geom::{LENGTH_TOL, vector_area};
use crate::ids::{
    CurveId, DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, MaterialId, PageId,
    TagId, VertexId,
};
use crate::index::Index;
use crate::texture::{TexturePlacement, is_level};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub pos: DVec3,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub v: [VertexId; 2],
    /// The curve (a circle or arc drawn as segments) this edge is part of. The edges sharing a
    /// curve id always form one chain; see `Model::curve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<CurveId>,
    /// A soft edge is hidden between two faces that form one smooth surface (the sides of a
    /// pulled circle). It always bounds at least two faces.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub soft: bool,
    /// A side of a polygon: its curve reads as a polygon rather than a circle, and its sides
    /// stay hard when pulled. Every edge of a curve has the same flag; a plain edge never has it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub polygon: bool,
    /// The tag the edge is on; None is Untagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<TagId>,
}

impl Edge {
    /// A plain edge: not part of a curve, not soft, Untagged.
    pub fn new(a: VertexId, b: VertexId) -> Edge {
        Edge {
            v: [a, b],
            curve: None,
            soft: false,
            polygon: false,
            tag: None,
        }
    }
}

/// A planar face. `outer` winds counter-clockwise around `normal`; each hole winds clockwise.
/// Consecutive loop vertices (wrapping) are always joined by an edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub outer: Vec<VertexId>,
    pub holes: Vec<Vec<VertexId>>,
    pub normal: DVec3,
    /// The material of the side `normal` points to; None is the default material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<MaterialId>,
    /// The material of the other side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<MaterialId>,
    /// Where the texture lies on top of the projection; None for the projection as it is.
    /// Never the default placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<TexturePlacement>,
    /// The tag the face is on; None is Untagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<TagId>,
}

impl Face {
    /// An unpainted, Untagged face.
    pub fn new(outer: Vec<VertexId>, holes: Vec<Vec<VertexId>>, normal: DVec3) -> Face {
        Face {
            outer,
            holes,
            normal,
            front: None,
            back: None,
            texture: None,
            tag: None,
        }
    }

    /// The same face turned over: loops reversed, normal negated, and the materials swapped and
    /// the texture placement turned over so each side looks as it did.
    pub fn reversed(&self) -> Face {
        let rev = |l: &Vec<VertexId>| l.iter().rev().copied().collect::<Vec<_>>();
        Face {
            outer: rev(&self.outer),
            holes: self.holes.iter().map(rev).collect(),
            normal: -self.normal,
            front: self.back,
            back: self.front,
            texture: self.texture.map(|t| t.turned_over(is_level(self.normal))),
            tag: self.tag,
        }
    }

    /// The front and back materials.
    pub fn paint(&self) -> [Option<MaterialId>; 2] {
        [self.front, self.back]
    }
}

/// A placed copy of a definition's geometry (a group or component).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub definition: DefinitionId,
    /// Maps the definition's coordinates into the coordinates of the mesh holding the instance.
    pub transform: DAffine3,
    /// The material shown on the sides of its faces (at any depth) that have none of their
    /// own, unless an instance nearer to them has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialId>,
    /// The tag the instance is on; None is Untagged. Hiding it hides everything inside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<TagId>,
}

impl Instance {
    /// An unpainted, Untagged instance.
    pub fn new(definition: DefinitionId, transform: DAffine3) -> Instance {
        Instance {
            definition,
            transform,
            material: None,
            tag: None,
        }
    }
}

/// Construction geometry drawn by the Tape Measure: not part of any face, exported only to
/// SketchUp files.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Guide {
    /// The infinite line through `point` along the unit vector `direction`.
    Line {
        point: DVec3,
        direction: DVec3,
        /// The tag the guide is on; None is Untagged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<TagId>,
    },
    /// A single point.
    Point {
        position: DVec3,
        /// The tag the guide is on; None is Untagged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<TagId>,
    },
}

impl Guide {
    /// An Untagged guide line through `point` along `direction`.
    pub fn line(point: DVec3, direction: DVec3) -> Guide {
        Guide::Line {
            point,
            direction,
            tag: None,
        }
    }

    /// An Untagged guide point at `position`.
    pub fn point(position: DVec3) -> Guide {
        Guide::Point {
            position,
            tag: None,
        }
    }

    /// The tag the guide is on; None is Untagged.
    pub fn tag(&self) -> Option<TagId> {
        match *self {
            Guide::Line { tag, .. } | Guide::Point { tag, .. } => tag,
        }
    }

    /// The same guide on `tag`.
    pub fn with_tag(mut self, to: Option<TagId>) -> Guide {
        match &mut self {
            Guide::Line { tag, .. } | Guide::Point { tag, .. } => *tag = to,
        }
        self
    }

    /// The guide moved by a rigid motion `t`, on the same tag.
    pub fn transformed(&self, t: DAffine3) -> Guide {
        match *self {
            Guide::Line {
                point,
                direction,
                tag,
            } => Guide::Line {
                point: t.transform_point3(point),
                direction: t.transform_vector3(direction).normalize(),
                tag,
            },
            Guide::Point { position, tag } => Guide::Point {
                position: t.transform_point3(position),
                tag,
            },
        }
    }

    /// The point a guide line passes through, or the guide point itself.
    pub fn anchor(&self) -> DVec3 {
        match *self {
            Guide::Line { point, .. } => point,
            Guide::Point { position, .. } => position,
        }
    }
}

/// One primitive, invertible change to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Op {
    AddVertex(VertexId, Vertex),
    RemoveVertex(VertexId, Vertex),
    AddEdge(EdgeId, Edge),
    RemoveEdge(EdgeId, Edge),
    /// An edge's curve, soft or polygon flag changed from the first to the second (same vertices).
    ChangeEdge(EdgeId, Edge, Edge),
    AddFace(FaceId, Face),
    RemoveFace(FaceId, Face),
    /// A vertex moved from the first position to the second.
    MoveVertex(VertexId, DVec3, DVec3),
    AddInstance(InstanceId, Instance),
    RemoveInstance(InstanceId, Instance),
    /// An instance's transform changed from the first to the second.
    TransformInstance(InstanceId, DAffine3, DAffine3),
    AddGuide(GuideId, Guide),
    RemoveGuide(GuideId, Guide),
    /// A face's front and back materials changed from the first pair to the second.
    PaintFace(FaceId, [Option<MaterialId>; 2], [Option<MaterialId>; 2]),
    /// An instance's material changed from the first to the second.
    PaintInstance(InstanceId, Option<MaterialId>, Option<MaterialId>),
    /// A face's texture placement changed from the first to the second.
    PlaceTexture(FaceId, Option<TexturePlacement>, Option<TexturePlacement>),
    /// An edge's, face's, instance's or guide's tag changed from the first to the second.
    SetTag(EntityId, Option<TagId>, Option<TagId>),
}

impl Op {
    fn inverse(&self) -> Op {
        match self.clone() {
            Op::AddVertex(id, v) => Op::RemoveVertex(id, v),
            Op::RemoveVertex(id, v) => Op::AddVertex(id, v),
            Op::AddEdge(id, e) => Op::RemoveEdge(id, e),
            Op::RemoveEdge(id, e) => Op::AddEdge(id, e),
            Op::ChangeEdge(id, from, to) => Op::ChangeEdge(id, to, from),
            Op::AddFace(id, f) => Op::RemoveFace(id, f),
            Op::RemoveFace(id, f) => Op::AddFace(id, f),
            Op::MoveVertex(id, from, to) => Op::MoveVertex(id, to, from),
            Op::AddInstance(id, i) => Op::RemoveInstance(id, i),
            Op::RemoveInstance(id, i) => Op::AddInstance(id, i),
            Op::TransformInstance(id, from, to) => Op::TransformInstance(id, to, from),
            Op::AddGuide(id, g) => Op::RemoveGuide(id, g),
            Op::RemoveGuide(id, g) => Op::AddGuide(id, g),
            Op::PaintFace(id, from, to) => Op::PaintFace(id, to, from),
            Op::PaintInstance(id, from, to) => Op::PaintInstance(id, to, from),
            Op::PlaceTexture(id, from, to) => Op::PlaceTexture(id, to, from),
            Op::SetTag(id, from, to) => Op::SetTag(id, to, from),
        }
    }

    /// The entity this op touches and whether it exists before and after the op.
    fn entity(&self) -> (EntityId, bool, bool) {
        match self {
            Op::AddVertex(id, _) => (EntityId::Vertex(*id), false, true),
            Op::RemoveVertex(id, _) => (EntityId::Vertex(*id), true, false),
            Op::MoveVertex(id, _, _) => (EntityId::Vertex(*id), true, true),
            Op::AddEdge(id, _) => (EntityId::Edge(*id), false, true),
            Op::RemoveEdge(id, _) => (EntityId::Edge(*id), true, false),
            Op::ChangeEdge(id, _, _) => (EntityId::Edge(*id), true, true),
            Op::AddFace(id, _) => (EntityId::Face(*id), false, true),
            Op::RemoveFace(id, _) => (EntityId::Face(*id), true, false),
            Op::AddInstance(id, _) => (EntityId::Instance(*id), false, true),
            Op::RemoveInstance(id, _) => (EntityId::Instance(*id), true, false),
            Op::TransformInstance(id, _, _) => (EntityId::Instance(*id), true, true),
            Op::AddGuide(id, _) => (EntityId::Guide(*id), false, true),
            Op::RemoveGuide(id, _) => (EntityId::Guide(*id), true, false),
            Op::PaintFace(id, _, _) => (EntityId::Face(*id), true, true),
            Op::PaintInstance(id, _, _) => (EntityId::Instance(*id), true, true),
            Op::PlaceTexture(id, _, _) => (EntityId::Face(*id), true, true),
            Op::SetTag(id, _, _) => (*id, true, true),
        }
    }
}

/// The ordered primitive ops of one edit. Applying `inverse()` undoes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub ops: Vec<Op>,
}

/// Net effect of a ChangeSet per entity. A moved vertex is `modified`; the edges and faces using
/// it change shape without being listed. A moved instance is `modified` too.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChangeSummary {
    pub created: Vec<EntityId>,
    pub deleted: Vec<EntityId>,
    pub modified: Vec<EntityId>,
    /// Materials added, removed, renamed or recolored (see `SceneChange::summary`). Absent
    /// when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<MaterialId>,
    /// Tags added, removed, renamed, recolored, shown or hidden (see `SceneChange::summary`).
    /// Absent when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagId>,
    /// Scenes added, removed, renamed, moved or updated (see `SceneChange::summary`). Absent
    /// when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenes: Vec<PageId>,
}

impl ChangeSummary {
    /// The net change of `summaries` applied in order: made then changed is made, made then
    /// erased is absent, changed then erased is erased. A material, tag or scene listed in any
    /// of them is listed in the result.
    pub fn join<'a>(summaries: impl IntoIterator<Item = &'a ChangeSummary>) -> ChangeSummary {
        // entity -> (existed before, exists after)
        let mut state: BTreeMap<EntityId, (bool, bool)> = BTreeMap::new();
        let mut materials = BTreeSet::new();
        let mut tags = BTreeSet::new();
        let mut scenes = BTreeSet::new();
        for s in summaries {
            let listed = s
                .created
                .iter()
                .map(|id| (id, false, true))
                .chain(s.deleted.iter().map(|id| (id, true, false)))
                .chain(s.modified.iter().map(|id| (id, true, true)));
            for (id, before, after) in listed {
                state.entry(*id).or_insert((before, after)).1 = after;
            }
            materials.extend(s.materials.iter().copied());
            tags.extend(s.tags.iter().copied());
            scenes.extend(s.scenes.iter().copied());
        }
        let mut net = ChangeSummary::default();
        for (id, (before, after)) in state {
            match (before, after) {
                (false, true) => net.created.push(id),
                (true, false) => net.deleted.push(id),
                (true, true) => net.modified.push(id),
                (false, false) => {}
            }
        }
        net.materials = materials.into_iter().collect();
        net.tags = tags.into_iter().collect();
        net.scenes = scenes.into_iter().collect();
        net
    }
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn inverse(&self) -> ChangeSet {
        ChangeSet {
            ops: self.ops.iter().rev().map(Op::inverse).collect(),
        }
    }

    pub fn summary(&self) -> ChangeSummary {
        // entity -> (existed before, exists after)
        let mut state: BTreeMap<EntityId, (bool, bool)> = BTreeMap::new();
        for op in &self.ops {
            let (id, before, after) = op.entity();
            state.entry(id).or_insert((before, after)).1 = after;
        }
        let mut s = ChangeSummary::default();
        for (id, (before, after)) in state {
            match (before, after) {
                (false, true) => s.created.push(id),
                (true, false) => s.deleted.push(id),
                (true, true) => s.modified.push(id),
                (false, false) => {}
            }
        }
        s
    }
}

/// The geometry of one mesh, plus the instances and guides placed in it. Every mutation is
/// recorded as an `Op` until `take_changes`.
#[derive(Clone, Debug, Default)]
pub struct Model {
    vertices: BTreeMap<VertexId, Vertex>,
    edges: BTreeMap<EdgeId, Edge>,
    faces: BTreeMap<FaceId, Face>,
    instances: BTreeMap<InstanceId, Instance>,
    guides: BTreeMap<GuideId, Guide>,
    next_id: u64,
    pending: Vec<Op>,
    /// Built on first use after a change to vertices, edges or faces; see `Model::index`.
    index: OnceLock<Index>,
    /// The edges at each vertex, kept in step by `apply_op`. Vertices without edges have no
    /// entry.
    vertex_edges: HashMap<VertexId, BTreeSet<EdgeId>>,
    /// The faces whose loops use each vertex, kept in step by `apply_op`. Vertices on no face
    /// have no entry.
    vertex_faces: HashMap<VertexId, BTreeSet<FaceId>>,
    /// The vertices in each cell of a `VERTEX_CELL` grid, kept in step by `apply_op`, so
    /// lookups by position visit only the vertices nearby.
    vertex_cells: HashMap<[i64; 3], Vec<VertexId>>,
}

/// Side of the cells of the vertex grid, in meters.
const VERTEX_CELL: f64 = 1e-3;

/// The vertex-grid cell holding `p`.
fn vertex_cell(p: DVec3) -> [i64; 3] {
    (p / VERTEX_CELL).floor().to_array().map(|c| c as i64)
}

/// Two models are equal when they hold the same entities under the same ids.
impl PartialEq for Model {
    fn eq(&self, other: &Self) -> bool {
        self.vertices == other.vertices
            && self.edges == other.edges
            && self.faces == other.faces
            && self.instances == other.instances
            && self.guides == other.guides
    }
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }

    /// A model holding exactly these entities, with nothing recorded. New ids continue after
    /// the largest one given. The caller validates the result.
    pub(crate) fn from_parts(
        vertices: BTreeMap<VertexId, Vertex>,
        edges: BTreeMap<EdgeId, Edge>,
        faces: BTreeMap<FaceId, Face>,
        instances: BTreeMap<InstanceId, Instance>,
        guides: BTreeMap<GuideId, Guide>,
    ) -> Model {
        let next_id = vertices
            .keys()
            .map(|k| k.0)
            .chain(edges.keys().map(|k| k.0))
            .chain(edges.values().filter_map(|e| e.curve.map(|c| c.0)))
            .chain(faces.keys().map(|k| k.0))
            .chain(instances.keys().map(|k| k.0))
            .chain(guides.keys().map(|k| k.0))
            .max()
            .map_or(0, |m| m.saturating_add(1));
        let mut m = Model {
            vertices: BTreeMap::new(),
            edges: BTreeMap::new(),
            faces: BTreeMap::new(),
            instances,
            guides,
            next_id,
            ..Model::default()
        };
        for (id, v) in &vertices {
            m.link_vertex(*id, v.pos);
        }
        m.vertices = vertices;
        for (id, e) in edges {
            m.link_edge(id, &e);
            m.edges.insert(id, e);
        }
        for (id, f) in faces {
            m.link_face(id, &f);
            m.faces.insert(id, f);
        }
        m
    }

    // ---- read access ----

    /// The spatial index over the current geometry, built on first use after a change.
    pub fn index(&self) -> &Index {
        self.index.get_or_init(|| Index::build(self))
    }

    pub fn vertex(&self, id: VertexId) -> Option<&Vertex> {
        self.vertices.get(&id)
    }

    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(&id)
    }

    pub fn face(&self, id: FaceId) -> Option<&Face> {
        self.faces.get(&id)
    }

    pub fn vertices(&self) -> impl Iterator<Item = (VertexId, &Vertex)> {
        self.vertices.iter().map(|(k, v)| (*k, v))
    }

    pub fn edges(&self) -> impl Iterator<Item = (EdgeId, &Edge)> {
        self.edges.iter().map(|(k, v)| (*k, v))
    }

    pub fn faces(&self) -> impl Iterator<Item = (FaceId, &Face)> {
        self.faces.iter().map(|(k, v)| (*k, v))
    }

    pub fn instance(&self, id: InstanceId) -> Option<&Instance> {
        self.instances.get(&id)
    }

    pub fn instances(&self) -> impl Iterator<Item = (InstanceId, &Instance)> {
        self.instances.iter().map(|(k, v)| (*k, v))
    }

    pub fn guide(&self, id: GuideId) -> Option<&Guide> {
        self.guides.get(&id)
    }

    pub fn guides(&self) -> impl Iterator<Item = (GuideId, &Guide)> {
        self.guides.iter().map(|(k, v)| (*k, v))
    }

    /// True when the mesh holds no vertices and no instances (guides do not count).
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() && self.instances.is_empty()
    }

    pub(crate) fn pos(&self, id: VertexId) -> DVec3 {
        self.vertices[&id].pos
    }

    /// Position of a vertex, or None if it does not exist.
    pub fn position(&self, id: VertexId) -> Option<DVec3> {
        self.vertices.get(&id).map(|v| v.pos)
    }

    pub fn edge_between(&self, a: VertexId, b: VertexId) -> Option<EdgeId> {
        if a == b {
            return None;
        }
        self.vertex_edges
            .get(&a)?
            .iter()
            .copied()
            .find(|e| self.edges[e].v.contains(&b))
    }

    /// The vertices within `r` of `p` (`r` small next to a meter), in id order.
    pub(crate) fn vertices_near(&self, p: DVec3, r: f64) -> Vec<VertexId> {
        let (lo, hi) = (
            vertex_cell(p - DVec3::splat(r)),
            vertex_cell(p + DVec3::splat(r)),
        );
        let mut found = Vec::new();
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    for v in self.vertex_cells.get(&[x, y, z]).into_iter().flatten() {
                        if self.vertices[v].pos.distance(p) < r {
                            found.push(*v);
                        }
                    }
                }
            }
        }
        found.sort();
        found
    }

    /// The edges at vertex `v`, in id order.
    pub fn edges_at(&self, v: VertexId) -> Vec<EdgeId> {
        self.vertex_edges
            .get(&v)
            .map_or_else(Vec::new, |s| s.iter().copied().collect())
    }

    /// The faces whose outer loop or a hole uses vertex `v`, in id order.
    pub fn faces_at(&self, v: VertexId) -> impl Iterator<Item = FaceId> + '_ {
        self.vertex_faces.get(&v).into_iter().flatten().copied()
    }

    /// The faces with a loop side a-b (either way round), in id order.
    pub(crate) fn faces_on_side(&self, a: VertexId, b: VertexId) -> Vec<FaceId> {
        self.faces_at(a)
            .filter(|f| {
                let f = &self.faces[f];
                std::iter::once(&f.outer)
                    .chain(&f.holes)
                    .any(|l| crate::edit::loop_has_side(l, a, b))
            })
            .collect()
    }

    /// Area of a face (outer area minus holes), or None if it does not exist.
    pub fn area(&self, id: FaceId) -> Option<f64> {
        self.faces.contains_key(&id).then(|| self.face_area(id))
    }

    pub(crate) fn face_area(&self, id: FaceId) -> f64 {
        let f = &self.faces[&id];
        let area = |l: &Vec<VertexId>| {
            vector_area(&l.iter().map(|v| self.pos(*v)).collect::<Vec<_>>()).dot(f.normal)
        };
        area(&f.outer) + f.holes.iter().map(area).sum::<f64>()
    }

    // ---- change tracking ----

    /// Ends the current edit and returns everything recorded since the last call. Curves and
    /// soft edges the edit touched are brought back in line first (see `upkeep`).
    pub fn take_changes(&mut self) -> ChangeSet {
        self.upkeep();
        ChangeSet {
            ops: std::mem::take(&mut self.pending),
        }
    }

    /// Applies a ChangeSet (e.g. an inverse for undo) without recording it.
    pub fn apply(&mut self, changes: &ChangeSet) {
        for op in &changes.ops {
            self.apply_op(op.clone());
        }
    }

    fn apply_op(&mut self, op: Op) {
        // The index holds vertex, edge and face positions only: tags, paint, instances and
        // guides leave it valid.
        if matches!(
            op,
            Op::AddVertex(..)
                | Op::RemoveVertex(..)
                | Op::MoveVertex(..)
                | Op::AddEdge(..)
                | Op::RemoveEdge(..)
                | Op::AddFace(..)
                | Op::RemoveFace(..)
        ) {
            self.index.take();
        }
        match op {
            Op::AddVertex(id, v) => {
                self.bump(id.0);
                if let Some(old) = self.vertices.remove(&id) {
                    self.unlink_vertex(id, old.pos);
                }
                self.link_vertex(id, v.pos);
                self.vertices.insert(id, v);
            }
            Op::RemoveVertex(id, _) => {
                if let Some(v) = self.vertices.remove(&id) {
                    self.unlink_vertex(id, v.pos);
                }
            }
            Op::AddEdge(id, e) => {
                self.bump(id.0);
                if let Some(c) = e.curve {
                    self.bump(c.0);
                }
                if let Some(old) = self.edges.remove(&id) {
                    self.unlink_edge(id, &old);
                }
                self.link_edge(id, &e);
                self.edges.insert(id, e);
            }
            Op::RemoveEdge(id, _) => {
                if let Some(e) = self.edges.remove(&id) {
                    self.unlink_edge(id, &e);
                }
            }
            Op::ChangeEdge(id, _, to) => {
                if let Some(c) = to.curve {
                    self.bump(c.0);
                }
                if let Some(e) = self.edges.get_mut(&id) {
                    // The adjacency maps are keyed by the end vertices: a change of ends
                    // must go through RemoveEdge/AddEdge.
                    debug_assert_eq!(e.v, to.v, "ChangeEdge must not change an edge's vertices");
                    *e = to;
                }
            }
            Op::AddFace(id, f) => {
                self.bump(id.0);
                if let Some(old) = self.faces.remove(&id) {
                    self.unlink_face(id, &old);
                }
                self.link_face(id, &f);
                self.faces.insert(id, f);
            }
            Op::RemoveFace(id, _) => {
                if let Some(f) = self.faces.remove(&id) {
                    self.unlink_face(id, &f);
                }
            }
            Op::MoveVertex(id, _, to) => {
                if let Some(v) = self.vertices.get_mut(&id) {
                    let from = std::mem::replace(&mut v.pos, to);
                    self.unlink_vertex(id, from);
                    self.link_vertex(id, to);
                }
            }
            Op::AddInstance(id, i) => {
                self.bump(id.0);
                self.instances.insert(id, i);
            }
            Op::RemoveInstance(id, _) => {
                self.instances.remove(&id);
            }
            Op::TransformInstance(id, _, to) => {
                if let Some(i) = self.instances.get_mut(&id) {
                    i.transform = to;
                }
            }
            Op::AddGuide(id, g) => {
                self.bump(id.0);
                self.guides.insert(id, g);
            }
            Op::RemoveGuide(id, _) => {
                self.guides.remove(&id);
            }
            Op::PaintFace(id, _, [front, back]) => {
                if let Some(f) = self.faces.get_mut(&id) {
                    f.front = front;
                    f.back = back;
                }
            }
            Op::PaintInstance(id, _, to) => {
                if let Some(i) = self.instances.get_mut(&id) {
                    i.material = to;
                }
            }
            Op::PlaceTexture(id, _, to) => {
                if let Some(f) = self.faces.get_mut(&id) {
                    f.texture = to;
                }
            }
            Op::SetTag(id, _, to) => match id {
                EntityId::Edge(e) => {
                    if let Some(e) = self.edges.get_mut(&e) {
                        e.tag = to;
                    }
                }
                EntityId::Face(f) => {
                    if let Some(f) = self.faces.get_mut(&f) {
                        f.tag = to;
                    }
                }
                EntityId::Instance(i) => {
                    if let Some(i) = self.instances.get_mut(&i) {
                        i.tag = to;
                    }
                }
                EntityId::Guide(g) => {
                    if let Some(g) = self.guides.get_mut(&g) {
                        *g = g.with_tag(to);
                    }
                }
                EntityId::Vertex(_) => {}
            },
        }
    }

    fn link_vertex(&mut self, id: VertexId, pos: DVec3) {
        self.vertex_cells
            .entry(vertex_cell(pos))
            .or_default()
            .push(id);
    }

    fn unlink_vertex(&mut self, id: VertexId, pos: DVec3) {
        let cell = vertex_cell(pos);
        if let Some(list) = self.vertex_cells.get_mut(&cell) {
            list.retain(|v| *v != id);
            if list.is_empty() {
                self.vertex_cells.remove(&cell);
            }
        }
    }

    fn link_edge(&mut self, id: EdgeId, e: &Edge) {
        for v in e.v {
            self.vertex_edges.entry(v).or_default().insert(id);
        }
    }

    fn unlink_edge(&mut self, id: EdgeId, e: &Edge) {
        for v in e.v {
            if let Some(set) = self.vertex_edges.get_mut(&v) {
                set.remove(&id);
                if set.is_empty() {
                    self.vertex_edges.remove(&v);
                }
            }
        }
    }

    fn link_face(&mut self, id: FaceId, f: &Face) {
        for v in std::iter::once(&f.outer).chain(&f.holes).flatten() {
            self.vertex_faces.entry(*v).or_default().insert(id);
        }
    }

    fn unlink_face(&mut self, id: FaceId, f: &Face) {
        for v in std::iter::once(&f.outer).chain(&f.holes).flatten() {
            if let Some(set) = self.vertex_faces.get_mut(v) {
                set.remove(&id);
                if set.is_empty() {
                    self.vertex_faces.remove(v);
                }
            }
        }
    }

    fn bump(&mut self, used: u64) {
        self.next_id = self.next_id.max(used + 1);
    }

    /// The id the next new entity gets.
    pub(crate) fn next_id(&self) -> u64 {
        self.next_id
    }

    /// The vertices with ids in `ids`, in id order.
    pub(crate) fn vertices_in(&self, ids: Range<u64>) -> impl Iterator<Item = VertexId> + '_ {
        self.vertices
            .range(VertexId(ids.start)..VertexId(ids.end))
            .map(|(k, _)| *k)
    }

    /// The edges with ids in `ids`, in id order.
    pub(crate) fn edges_in(&self, ids: Range<u64>) -> impl Iterator<Item = EdgeId> + '_ {
        self.edges
            .range(EdgeId(ids.start)..EdgeId(ids.end))
            .map(|(k, _)| *k)
    }

    /// Makes new entities get ids from `first` on (or later), so that ids stay unique across
    /// the meshes of a scene.
    pub(crate) fn reserve_ids(&mut self, first: u64) {
        self.next_id = self.next_id.max(first);
    }

    /// The ops recorded since the last `take_changes`.
    pub(crate) fn pending(&self) -> &[Op] {
        &self.pending
    }

    fn record(&mut self, op: Op) {
        self.apply_op(op.clone());
        self.pending.push(op);
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ---- recorded primitive mutations (used by edit and face code) ----

    pub(crate) fn insert_vertex(&mut self, pos: DVec3) -> VertexId {
        let id = VertexId(self.alloc());
        self.record(Op::AddVertex(id, Vertex { pos }));
        id
    }

    pub(crate) fn remove_vertex(&mut self, id: VertexId) -> Vertex {
        let v = self.vertices[&id].clone();
        self.record(Op::RemoveVertex(id, v.clone()));
        v
    }

    pub(crate) fn move_vertex(&mut self, id: VertexId, to: DVec3) {
        let from = self.pos(id);
        self.record(Op::MoveVertex(id, from, to));
    }

    pub(crate) fn insert_edge(&mut self, a: VertexId, b: VertexId) -> EdgeId {
        self.insert_edge_like(a, b, None, false, false, None)
    }

    /// Adds the edge a-b on `curve`, with `soft` and `polygon`, and on `tag` (the attributes
    /// of an edge it replaces). An edge on no curve is never a polygon side.
    pub(crate) fn insert_edge_like(
        &mut self,
        a: VertexId,
        b: VertexId,
        curve: Option<CurveId>,
        soft: bool,
        polygon: bool,
        tag: Option<TagId>,
    ) -> EdgeId {
        let id = EdgeId(self.alloc());
        let edge = Edge {
            v: [a, b],
            curve,
            soft,
            polygon: polygon && curve.is_some(),
            tag,
        };
        self.record(Op::AddEdge(id, edge));
        id
    }

    /// Puts an edge on `curve` (or none) and sets its soft and polygon flags, recording a change
    /// only when something differs. The edge keeps its tag. An edge on no curve is never a
    /// polygon side.
    pub(crate) fn set_edge(
        &mut self,
        id: EdgeId,
        curve: Option<CurveId>,
        soft: bool,
        polygon: bool,
    ) {
        let from = self.edges[&id].clone();
        let polygon = polygon && curve.is_some();
        if from.curve == curve && from.soft == soft && from.polygon == polygon {
            return;
        }
        let to = Edge {
            v: from.v,
            curve,
            soft,
            polygon,
            tag: from.tag,
        };
        self.record(Op::ChangeEdge(id, from, to));
    }

    /// A new curve id, unique like entity ids.
    pub(crate) fn new_curve(&mut self) -> CurveId {
        CurveId(self.alloc())
    }

    pub(crate) fn remove_edge(&mut self, id: EdgeId) -> Edge {
        let e = self.edges[&id].clone();
        self.record(Op::RemoveEdge(id, e.clone()));
        e
    }

    pub(crate) fn insert_face(&mut self, face: Face) -> FaceId {
        let id = FaceId(self.alloc());
        self.record(Op::AddFace(id, face));
        id
    }

    pub(crate) fn remove_face(&mut self, id: FaceId) -> Face {
        let f = self.faces[&id].clone();
        self.record(Op::RemoveFace(id, f.clone()));
        f
    }

    pub(crate) fn insert_instance(&mut self, instance: Instance) -> InstanceId {
        let id = InstanceId(self.alloc());
        self.record(Op::AddInstance(id, instance));
        id
    }

    /// Adds an instance under an id it had elsewhere (an instance moved between meshes).
    pub(crate) fn insert_instance_as(&mut self, id: InstanceId, instance: Instance) {
        self.record(Op::AddInstance(id, instance));
    }

    pub(crate) fn remove_instance(&mut self, id: InstanceId) -> Instance {
        let i = self.instances[&id].clone();
        self.record(Op::RemoveInstance(id, i.clone()));
        i
    }

    pub(crate) fn insert_guide(&mut self, guide: Guide) -> GuideId {
        let id = GuideId(self.alloc());
        self.record(Op::AddGuide(id, guide));
        id
    }

    /// Adds a guide under an id it had elsewhere (a guide moved between meshes) or before
    /// (a guide moved in place).
    pub(crate) fn insert_guide_as(&mut self, id: GuideId, guide: Guide) {
        self.record(Op::AddGuide(id, guide));
    }

    pub(crate) fn remove_guide(&mut self, id: GuideId) -> Guide {
        let g = self.guides[&id];
        self.record(Op::RemoveGuide(id, g));
        g
    }

    /// Moves instances by `offset` as one ChangeSet. Each listed instance moves once; unknown
    /// ids are skipped.
    pub fn move_instances(&mut self, ids: &[InstanceId], offset: DVec3) -> ChangeSet {
        self.transform_instances(ids, DAffine3::from_translation(offset))
    }

    /// Applies `t` after the transforms of these instances (unknown ids are skipped), as one
    /// ChangeSet: the instances move and turn with `t` in this mesh's coordinates.
    pub fn transform_instances(&mut self, ids: &[InstanceId], t: DAffine3) -> ChangeSet {
        let ids: BTreeSet<InstanceId> = ids.iter().copied().collect();
        for id in &ids {
            if let Some(i) = self.instances.get(id) {
                let from = i.transform;
                let to = t * from;
                self.record(Op::TransformInstance(*id, from, to));
            }
        }
        self.take_changes()
    }

    /// Sets a face's front and back materials, recording a change only when they differ.
    pub(crate) fn set_paint(&mut self, id: FaceId, paint: [Option<MaterialId>; 2]) {
        let from = self.faces[&id].paint();
        if from != paint {
            self.record(Op::PaintFace(id, from, paint));
        }
    }

    /// Sets a face's texture placement (a default placement is stored as none), recording a
    /// change only when it differs.
    pub(crate) fn set_texture(&mut self, id: FaceId, texture: Option<TexturePlacement>) {
        let texture = texture.filter(|t| !t.is_default());
        let from = self.faces[&id].texture;
        if from != texture {
            self.record(Op::PlaceTexture(id, from, texture));
        }
    }

    /// Sets an instance's material, recording a change only when it differs.
    pub(crate) fn set_instance_material(&mut self, id: InstanceId, material: Option<MaterialId>) {
        let from = self.instances[&id].material;
        if from != material {
            self.record(Op::PaintInstance(id, from, material));
        }
    }

    /// The tag of an edge, face, instance or guide (None: Untagged); None for a vertex or an
    /// unknown id.
    pub fn tag_of(&self, id: EntityId) -> Option<Option<TagId>> {
        match id {
            EntityId::Edge(e) => self.edges.get(&e).map(|e| e.tag),
            EntityId::Face(f) => self.faces.get(&f).map(|f| f.tag),
            EntityId::Instance(i) => self.instances.get(&i).map(|i| i.tag),
            EntityId::Guide(g) => self.guides.get(&g).map(Guide::tag),
            EntityId::Vertex(_) => None,
        }
    }

    /// Puts an edge, face, instance or guide on `tag` (None: Untagged), recording a change
    /// only when it differs. Vertices and unknown ids are skipped.
    pub(crate) fn set_tag(&mut self, id: EntityId, tag: Option<TagId>) {
        if let Some(from) = self.tag_of(id)
            && from != tag
        {
            self.record(Op::SetTag(id, from, tag));
        }
    }

    /// Replaces a face's data, keeping its id.
    pub(crate) fn replace_face(&mut self, id: FaceId, face: Face) {
        self.remove_face(id);
        self.record(Op::AddFace(id, face));
    }

    // ---- invariants ----

    /// Checks structural invariants. Used by tests after every edit.
    pub fn validate(&self) -> Result<(), String> {
        // The lookups first: the checks below use them.
        self.validate_adjacency()?;
        let mut pairs = BTreeSet::new();
        for (id, e) in &self.edges {
            let [a, b] = e.v;
            if !self.vertices.contains_key(&a) || !self.vertices.contains_key(&b) {
                return Err(format!("edge {id:?} references a missing vertex"));
            }
            if a == b || self.pos(a).distance(self.pos(b)) < LENGTH_TOL {
                return Err(format!("edge {id:?} has zero length"));
            }
            if !pairs.insert((a.min(b), a.max(b))) {
                return Err(format!("edge {id:?} duplicates another edge"));
            }
        }
        // Vertices closer than LENGTH_TOL fall in the same or neighbouring cells of a
        // LENGTH_TOL grid, so only those are compared.
        let cell = |p: DVec3| (p / LENGTH_TOL).floor().to_array().map(|c| c as i64);
        let mut grid: HashMap<[i64; 3], Vec<VertexId>> = HashMap::new();
        for (ib, b) in &self.vertices {
            let [x, y, z] = cell(b.pos);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let key = [
                            x.saturating_add(dx),
                            y.saturating_add(dy),
                            z.saturating_add(dz),
                        ];
                        for ia in grid.get(&key).into_iter().flatten() {
                            if self.pos(*ia).distance(b.pos) < LENGTH_TOL {
                                return Err(format!("vertices {ia:?} and {ib:?} coincide"));
                            }
                        }
                    }
                }
            }
            grid.entry([x, y, z]).or_default().push(*ib);
        }
        for (id, i) in &self.instances {
            if !i.transform.is_finite() {
                return Err(format!("instance {id:?} has a non-finite transform"));
            }
        }
        for (id, g) in &self.guides {
            let ok = match g {
                Guide::Line {
                    point, direction, ..
                } => {
                    point.is_finite()
                        && direction.is_finite()
                        && (direction.length() - 1.0).abs() <= 1e-9
                }
                Guide::Point { position, .. } => position.is_finite(),
            };
            if !ok {
                return Err(format!(
                    "guide {id:?} is not finite or its direction is not a unit vector"
                ));
            }
        }
        for (id, f) in &self.faces {
            if (f.normal.length() - 1.0).abs() > 1e-9 {
                return Err(format!("face {id:?} normal is not unit length"));
            }
            if f.outer.is_empty() {
                return Err(format!("face {id:?} has an empty outer loop"));
            }
            if f.outer
                .iter()
                .chain(f.holes.iter().flatten())
                .any(|v| !self.vertices.contains_key(v))
            {
                return Err(format!("face {id:?} references a missing vertex"));
            }
            let origin = self.pos(f.outer[0]);
            for (k, lp) in std::iter::once(&f.outer).chain(&f.holes).enumerate() {
                if lp.len() < 3 {
                    return Err(format!("face {id:?} has a loop with fewer than 3 vertices"));
                }
                if lp.iter().collect::<BTreeSet<_>>().len() != lp.len() {
                    return Err(format!("face {id:?} has a loop that repeats a vertex"));
                }
                for i in 0..lp.len() {
                    let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
                    if self.edge_between(a, b).is_none() {
                        return Err(format!("face {id:?} loop side {a:?}-{b:?} has no edge"));
                    }
                    if (self.pos(a) - origin).dot(f.normal).abs() > LENGTH_TOL {
                        return Err(format!("face {id:?} vertex {a:?} is off the face plane"));
                    }
                }
                let pts: Vec<_> = lp.iter().map(|v| self.pos(*v)).collect();
                let signed = vector_area(&pts).dot(f.normal);
                let ok = if k == 0 { signed > 0.0 } else { signed < 0.0 };
                if !ok {
                    return Err(format!("face {id:?} loop {k} winds the wrong way"));
                }
            }
        }
        self.validate_curves()
    }

    /// Checks that the vertex grid and the vertex-to-edge and vertex-to-face maps match the
    /// vertices, edges and faces.
    fn validate_adjacency(&self) -> Result<(), String> {
        let gridded: usize = self.vertex_cells.values().map(Vec::len).sum();
        let placed = self.vertices.iter().all(|(id, v)| {
            self.vertex_cells
                .get(&vertex_cell(v.pos))
                .is_some_and(|l| l.contains(id))
        });
        if gridded != self.vertices.len() || !placed {
            return Err("the vertex grid does not match the vertices".into());
        }
        let mut edges: HashMap<VertexId, BTreeSet<EdgeId>> = HashMap::new();
        for (id, e) in &self.edges {
            for v in e.v {
                edges.entry(v).or_default().insert(*id);
            }
        }
        if edges != self.vertex_edges {
            return Err("the vertex-to-edge map does not match the edges".into());
        }
        let mut faces: HashMap<VertexId, BTreeSet<FaceId>> = HashMap::new();
        for (id, f) in &self.faces {
            for v in std::iter::once(&f.outer).chain(&f.holes).flatten() {
                faces.entry(*v).or_default().insert(*id);
            }
        }
        if faces != self.vertex_faces {
            return Err("the vertex-to-face map does not match the faces".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_changes_undo_and_redo_exactly() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        let b = m.insert_vertex(DVec3::X);
        m.insert_edge(a, b);
        let first = m.take_changes();
        let after_first = m.clone();

        let e = m.edge_between(a, b).unwrap();
        m.remove_edge(e);
        let second = m.take_changes();

        m.apply(&second.inverse());
        assert_eq!(m, after_first);
        m.apply(&first.inverse());
        assert_eq!(m, Model::new());
        m.apply(&first);
        m.apply(&second);
        assert_eq!(m.edges().count(), 0);
        assert_eq!(m.vertices().count(), 2);
    }

    #[test]
    fn summaries_join_to_the_net_change() {
        let v = |n| EntityId::Vertex(VertexId(n));
        let a = ChangeSummary {
            created: vec![v(1), v(2)],
            deleted: vec![v(9)],
            modified: vec![v(5)],
            materials: vec![],
            tags: vec![],
            scenes: vec![],
        };
        let b = ChangeSummary {
            created: vec![v(3)],
            deleted: vec![v(2), v(5)],
            modified: vec![v(1)],
            materials: vec![],
            tags: vec![],
            scenes: vec![],
        };
        let net = ChangeSummary::join([&a, &b]);
        // made then changed is made; made then erased is gone
        assert_eq!(net.created, vec![v(1), v(3)]);
        assert_eq!(net.deleted, vec![v(5), v(9)]);
        assert!(net.modified.is_empty());
    }

    #[test]
    fn summary_reports_net_effect() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        let b = m.insert_vertex(DVec3::X);
        let c = m.insert_vertex(DVec3::Y);
        let e = m.insert_edge(a, b);
        m.take_changes();

        m.remove_edge(e);
        let e2 = m.insert_edge(a, c);
        let s = m.take_changes().summary();
        assert_eq!(s.created, vec![EntityId::Edge(e2)]);
        assert_eq!(s.deleted, vec![EntityId::Edge(e)]);
        assert!(s.modified.is_empty());
    }

    #[test]
    fn moved_vertices_undo_exactly_and_count_as_modified() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        m.take_changes();
        let before = m.clone();
        m.move_vertex(a, DVec3::X);
        m.move_vertex(a, DVec3::Y);
        let c = m.take_changes();
        assert_eq!(m.position(a), Some(DVec3::Y));
        assert_eq!(c.summary().modified, vec![EntityId::Vertex(a)]);
        assert!(c.summary().created.is_empty() && c.summary().deleted.is_empty());
        m.apply(&c.inverse());
        assert_eq!(m, before);
    }

    #[test]
    fn ids_are_never_reused_after_undo() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        let c = m.take_changes();
        m.apply(&c.inverse());
        let b = m.insert_vertex(DVec3::X);
        assert_ne!(a, b);
    }

    #[test]
    fn validate_checks_face_invariants() {
        // Unit right triangle on the ground with its three sides as edges.
        let base = || {
            let mut m = Model::new();
            let a = m.insert_vertex(DVec3::ZERO);
            let b = m.insert_vertex(DVec3::X);
            let c = m.insert_vertex(DVec3::Y);
            m.insert_edge(a, b);
            m.insert_edge(b, c);
            m.insert_edge(c, a);
            (m, a, b, c)
        };

        let (mut m, a, b, c) = base();
        m.insert_face(Face::new(vec![a, b, c], vec![], DVec3::Z));
        assert!(m.validate().is_ok());

        let bad_faces: Vec<(&str, Face)> = vec![
            ("empty outer loop", Face::new(vec![], vec![], DVec3::Z)),
            (
                "missing vertex",
                Face::new(vec![a, b, VertexId(999)], vec![], DVec3::Z),
            ),
            ("wrong winding", Face::new(vec![a, c, b], vec![], DVec3::Z)),
            (
                "non-unit normal",
                Face::new(vec![a, b, c], vec![], DVec3::new(0.0, 0.0, 2.0)),
            ),
        ];
        for (what, face) in bad_faces {
            let (mut m, _, _, _) = base();
            m.insert_face(face);
            assert!(m.validate().is_err(), "{what} was accepted");
        }
    }

    #[test]
    fn lookups_follow_edits_undo_and_redo() {
        let mut m = Model::new();
        let square = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        m.add_polygon(&square).unwrap();
        let before = m.clone();
        let a = m.find_vertex(DVec3::ZERO).unwrap();
        let c = m.find_vertex(DVec3::new(1.0, 1.0, 0.0)).unwrap();
        let face = m.faces().next().unwrap().0;
        assert_eq!(m.edges_at(a).len(), 2);
        assert_eq!(m.faces_at(a).collect::<Vec<_>>(), vec![face]);

        // A diagonal splits the face; then the corner moves.
        let split = m.add_edge(DVec3::ZERO, DVec3::new(1.0, 1.0, 0.0)).unwrap();
        m.validate().unwrap();
        assert_eq!(m.edges_at(a).len(), 3);
        assert_eq!(m.faces_at(a).count(), 2);
        let diagonal = m.edge_between(a, c).unwrap();
        assert_eq!(m.edge_between(c, a), Some(diagonal));
        assert_eq!(m.edge_between(a, a), None);
        let moved = m
            .move_vertices(&BTreeSet::from([c]), DVec3::new(0.5, 0.5, 0.0))
            .unwrap();
        m.validate().unwrap();
        assert_eq!(m.find_vertex(DVec3::new(1.0, 1.0, 0.0)), None);
        assert_eq!(m.find_vertex(DVec3::new(1.5, 1.5, 0.0)), Some(c));

        m.apply(&moved.inverse());
        m.apply(&split.inverse());
        m.validate().unwrap();
        assert_eq!(m, before);
        assert_eq!(m.edges_at(a).len(), 2);
        assert_eq!(m.edge_between(a, c), None);
        assert_eq!(m.faces_at(c).collect::<Vec<_>>(), vec![face]);
        m.apply(&split);
        m.apply(&moved);
        m.validate().unwrap();
        assert_eq!(m.find_vertex(DVec3::new(1.5, 1.5, 0.0)), Some(c));
    }

    #[test]
    fn find_vertex_takes_the_oldest_within_tolerance_across_cells() {
        let mut m = Model::new();
        // Both sides of a grid cell border, closer than LENGTH_TOL to the probe.
        let border = VERTEX_CELL * 7.0;
        let late = m.insert_vertex(DVec3::new(border + 4e-7, 0.0, 0.0));
        let early = m.insert_vertex(DVec3::new(border - 4e-7, 0.0, 0.0));
        let probe = DVec3::new(border, 0.0, 0.0);
        assert!(vertex_cell(m.pos(late)) != vertex_cell(m.pos(early)));
        assert_eq!(m.find_vertex(probe), Some(late.min(early)));
        assert_eq!(m.vertices_near(probe, LENGTH_TOL), vec![late, early]);
        assert_eq!(m.find_vertex(DVec3::new(border + 2e-6, 0.0, 0.0)), None);
    }

    #[test]
    fn add_vertex_over_an_existing_id_in_the_same_cell_keeps_it_in_the_grid() {
        let mut m = Model::new();
        let old = DVec3::new(VERTEX_CELL * 7.2, 0.0, 0.0);
        let new = DVec3::new(VERTEX_CELL * 7.6, 0.0, 0.0);
        let id = m.insert_vertex(old);
        m.take_changes();
        let mut to = m.vertices[&id].clone();
        to.pos = new;
        assert_eq!(vertex_cell(old), vertex_cell(new));
        m.apply(&ChangeSet {
            ops: vec![Op::AddVertex(id, to)],
        });
        m.validate().unwrap();
        assert_eq!(m.find_vertex(new), Some(id));
        assert_eq!(m.find_vertex(old), None);
    }

    #[test]
    fn tag_along_finds_pieces_cut_off_from_both_ends() {
        let mut m = Model::new();
        // Two pieces of the segment 0..4 on x, with no vertex at either end.
        let v: Vec<_> = [1.0, 2.0, 3.0]
            .iter()
            .map(|x| m.insert_vertex(DVec3::new(*x, 0.0, 0.0)))
            .collect();
        let e0 = m.insert_edge(v[0], v[1]);
        let e1 = m.insert_edge(v[1], v[2]);
        let off = m.insert_vertex(DVec3::new(2.0, 1.0, 0.0));
        let e2 = m.insert_edge(v[1], off);
        let curve = m.new_curve();
        m.tag_along(
            DVec3::ZERO,
            DVec3::new(4.0, 0.0, 0.0),
            Some(curve),
            false,
            false,
        );
        assert_eq!(m.edge(e0).unwrap().curve, Some(curve));
        assert_eq!(m.edge(e1).unwrap().curve, Some(curve));
        assert_eq!(m.edge(e2).unwrap().curve, None);
    }

    #[test]
    fn validate_catches_stale_lookups() {
        let mut m = Model::new();
        m.add_polygon(&[DVec3::ZERO, DVec3::X, DVec3::Y]).unwrap();
        m.validate().unwrap();
        let mut broken = m.clone();
        broken.vertex_edges.clear();
        assert!(broken.validate().unwrap_err().contains("vertex-to-edge"));
        let mut broken = m.clone();
        broken.vertex_faces.clear();
        assert!(broken.validate().unwrap_err().contains("vertex-to-face"));
        let mut broken = m.clone();
        broken.vertex_cells.clear();
        assert!(broken.validate().unwrap_err().contains("vertex grid"));
    }

    #[test]
    fn only_geometry_changes_drop_the_index() {
        let mut m = Model::new();
        m.add_polygon(&[DVec3::ZERO, DVec3::X, DVec3::Y]).unwrap();
        let face = m.faces().next().unwrap().0;
        let edge = m.edges().next().unwrap().0;
        let built = |m: &Model| m.index.get().is_some();
        m.index();
        assert!(built(&m));
        m.set_paint(face, [Some(MaterialId(100)), None]);
        m.set_edge(edge, Some(CurveId(101)), false, false);
        m.insert_guide(Guide::point(DVec3::Z));
        let i = m.insert_instance(Instance::new(DefinitionId(102), DAffine3::IDENTITY));
        m.transform_instances(&[i], DAffine3::from_translation(DVec3::X));
        m.set_instance_material(i, Some(MaterialId(100)));
        m.take_changes();
        assert!(
            built(&m),
            "paint, tags, guides and instances keep the index"
        );
        let v = m.find_vertex(DVec3::X).unwrap();
        m.move_vertex(v, DVec3::new(2.0, 0.0, 0.0));
        assert!(!built(&m), "a moved vertex drops it");
        assert_eq!(
            m.index()
                .ray_hit(DVec3::new(0.2, 0.2, 1.0), -DVec3::Z, 0.0, 9.0)
                .map(|h| h.0),
            Some(face)
        );
    }

    #[test]
    fn validate_catches_duplicate_edges() {
        let mut m = Model::new();
        let a = m.insert_vertex(DVec3::ZERO);
        let b = m.insert_vertex(DVec3::X);
        m.insert_edge(a, b);
        assert!(m.validate().is_ok());
        m.insert_edge(b, a);
        assert!(m.validate().unwrap_err().contains("duplicates"));
    }
}
