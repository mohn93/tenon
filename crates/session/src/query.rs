use std::collections::BTreeMap;

use kernel::{
    ChangeSummary, CurveKind, DefinitionId, DefinitionKind, EntityId, ImageFormat, MaterialId,
    TexturePlacement,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::camera::{Lighting, Projection};

/// Read-only questions about the document. Entities and coordinates are those of the open
/// context (see `Command::OpenInstance`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// Vertices, edges, faces, instances and guides. An entity is returned when `ids` is empty
    /// or lists it, `kinds` is empty or lists its kind, and `within` is absent or contains all
    /// its vertices (an instance: its bounding box; a guide point: its position; a guide line
    /// is in no box); unknown ids are simply absent from the result.
    Entities {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        kinds: Vec<EntityKind>,
        #[serde(default)]
        within: Option<Bounds>,
    },
    /// One entity by id ("v3", "e7", "f2", "i4", "g9").
    Entity {
        id: EntityId,
    },
    /// The curve (circle or arc) an edge is part of, given any of its edges.
    Curve {
        id: EntityId,
    },
    Camera,
    Stats,
    History,
    /// The selected edges, faces, instances and guides.
    Selection,
    /// Every group and component definition.
    Definitions,
    /// Every material.
    Materials,
    /// The open context: the path of instances being edited, from the root.
    Context,
    /// The tree of placed groups and components: the root's instances, each with the
    /// instances its definition places, down to `depth` levels below the root's instances
    /// (0: the root's instances only; absent: every level).
    Outliner {
        #[serde(default)]
        depth: Option<u32>,
    },
    /// The file the model was last opened from or saved to, and whether it changed since.
    File,
    /// Model changes after `since` (a revision number), oldest first.
    Events {
        since: u64,
    },
}

/// An axis-aligned box, bounds included.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    pub fn contains(&self, p: [f64; 3]) -> bool {
        (0..3).all(|i| self.min[i] <= p[i] && p[i] <= self.max[i])
    }
}

/// The kinds of entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Vertex,
    Edge,
    Face,
    Instance,
    Guide,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VertexInfo {
    pub id: EntityId,
    pub position: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EdgeInfo {
    pub id: EntityId,
    pub vertices: [EntityId; 2],
    pub start: [f64; 3],
    pub end: [f64; 3],
    /// The curve the edge is part of ("c12"), absent for a plain edge. Edges with the same
    /// curve form one circle or arc; the `curve` query describes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<String>,
    /// True for a soft edge: hidden, between faces that form one smooth surface. Absent when
    /// false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub soft: bool,
}

/// A curve: a chain of edges drawn as one circle or arc. Its shape is worked out from where
/// its edges are now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CurveInfo {
    /// "c12": the curve id its edges report.
    pub id: String,
    pub kind: CurveKind,
    /// In order along the curve, counter-clockwise around `normal` when there is one.
    pub edges: Vec<EntityId>,
    /// In order; a circle's first vertex is not repeated at the end.
    pub vertices: Vec<EntityId>,
    /// Straight segments; an edge other geometry split in two still counts once.
    pub segments: usize,
    /// Total length of the edges, in meters.
    pub length: f64,
    /// Center, unit normal (pointing up: +z, or on a vertical plane +y, else +x), radius and
    /// swept angle in degrees (360 for a circle); absent for kind `curve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FaceInfo {
    pub id: EntityId,
    pub outer: Vec<EntityId>,
    pub holes: Vec<Vec<EntityId>>,
    pub normal: [f64; 3],
    pub area: f64,
    /// The material of the side the normal points to; absent for the default material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front_material: Option<MaterialId>,
    /// The material of the other side; absent for the default material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_material: Option<MaterialId>,
    /// Where the texture lies on top of its projection (see `position_texture`); absent when
    /// it lies as projected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_position: Option<TexturePlacement>,
}

/// A placed group or component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstanceInfo {
    pub id: EntityId,
    pub definition: DefinitionId,
    pub name: String,
    pub kind: DefinitionKind,
    /// Column-major 3x4 matrix mapping the definition's coordinates into the open context's:
    /// x axis, y axis, z axis, translation.
    pub transform: [f64; 12],
    /// The box around the instance's geometry, in the open context's coordinates. Absent for
    /// an empty definition.
    pub bounds: Option<Bounds>,
    /// The material shown on face sides inside it without one of their own; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialId>,
}

/// Whether a guide is a line or a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GuideKind {
    Line,
    Point,
}

/// A guide line (through `point` along the unit vector `direction`) or a guide point (at
/// `point`, no `direction`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GuideInfo {
    pub id: EntityId,
    pub kind: GuideKind,
    pub point: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f64; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DefinitionInfo {
    pub id: DefinitionId,
    pub name: String,
    pub kind: DefinitionKind,
    /// How many instances place it, in all contexts.
    pub instances: usize,
    pub edges: usize,
    pub faces: usize,
}

/// A placed group or component in the `outliner` tree. A component's contents appear under
/// each of its instances, with the same ids; a node's path is the ids from the root down to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OutlinerNode {
    pub id: EntityId,
    pub definition: DefinitionId,
    pub kind: DefinitionKind,
    /// The definition's name; empty for an unnamed group.
    pub name: String,
    /// The instance's material; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialId>,
    /// True when the node is on the open context's path (the open instance or one it is in).
    /// Absent when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
    /// True when the node is an instance of the open context and is selected. Absent when
    /// false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub selected: bool,
    /// The instances its definition places, in id order.
    pub children: Vec<OutlinerNode>,
    /// True when the query's `depth` left out this node's children (it has some). Absent
    /// when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// One placed group or component in an `OutlinerTable`: what `OutlinerNode` says about it
/// apart from the marks and children, which depend on where it is placed.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlinerRow {
    pub id: EntityId,
    pub definition: DefinitionId,
    pub kind: DefinitionKind,
    /// The definition's name; empty for an unnamed group.
    pub name: String,
    pub material: Option<MaterialId>,
}

/// The outliner tree stored flat (`Document::outliner_rows`): the root's instances and, for
/// each definition, the instances it places. Its size is linear in the model however deep the
/// nesting; a node's children are `children(node.definition)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutlinerTable {
    pub roots: Vec<OutlinerRow>,
    pub by_definition: BTreeMap<DefinitionId, Vec<OutlinerRow>>,
}

impl OutlinerTable {
    /// The instances `definition` places, in id order (none for an unknown definition).
    pub fn children(&self, definition: DefinitionId) -> &[OutlinerRow] {
        self.by_definition
            .get(&definition)
            .map_or(&[], Vec::as_slice)
    }
}

/// A material and how much of the model uses it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaterialInfo {
    pub id: MaterialId,
    pub name: String,
    /// sRGB red, green and blue, 0 to 255.
    pub color: [u8; 3],
    /// From 0 (invisible) to 1 (opaque). Always present.
    pub opacity: f32,
    /// How strongly its faces glow, from 0 (not at all) to 10. Always present.
    pub emission: f32,
    /// The texture faces painted with it show; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<TextureInfo>,
    /// Faces painted with it on either side, in the whole model (each definition counted once).
    pub faces: usize,
    /// Instances painted with it, in the whole model.
    pub instances: usize,
}

/// A material's texture: the size one copy covers on the model, in meters, and the image's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextureInfo {
    pub width: f64,
    pub height: f64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub format: ImageFormat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryResult {
    Entities {
        vertices: Vec<VertexInfo>,
        edges: Vec<EdgeInfo>,
        faces: Vec<FaceInfo>,
        instances: Vec<InstanceInfo>,
        guides: Vec<GuideInfo>,
        revision: u64,
    },
    Vertex(VertexInfo),
    Edge(EdgeInfo),
    Face(FaceInfo),
    Instance(InstanceInfo),
    Guide(GuideInfo),
    Curve(CurveInfo),
    /// The view: `up` is the screen's up direction (+Z, except in a view straight down or
    /// up); `view_height` is the height the view spans at the target (in a parallel view, the
    /// whole view's height); `lighting` is `day` or `night`.
    Camera {
        eye: [f64; 3],
        target: [f64; 3],
        up: [f64; 3],
        fov_y_deg: f64,
        projection: Projection,
        view_height: f64,
        lighting: Lighting,
    },
    /// Counts for the open context, plus the number of definitions.
    Stats {
        vertices: usize,
        edges: usize,
        faces: usize,
        instances: usize,
        definitions: usize,
        revision: u64,
    },
    /// Undo history: how many steps can be undone and redone, and their labels (the `type` of
    /// the command that made each, or `batch` for an atomic batch of differing commands), the
    /// next to undo or redo first.
    History {
        undo_depth: usize,
        redo_depth: usize,
        undo: Vec<String>,
        redo: Vec<String>,
        revision: u64,
    },
    Selection {
        ids: Vec<EntityId>,
        revision: u64,
    },
    Definitions {
        definitions: Vec<DefinitionInfo>,
        revision: u64,
    },
    Materials {
        materials: Vec<MaterialInfo>,
        revision: u64,
    },
    /// `path` lists the open instances from the root; empty when the root is open.
    /// `definition` is the definition being edited.
    Context {
        path: Vec<EntityId>,
        definition: Option<DefinitionId>,
    },
    /// The root's instances, in id order, each with what it places.
    Outliner {
        instances: Vec<OutlinerNode>,
        revision: u64,
    },
    /// `path` is null for a model never saved. `modified` is true when the model changed
    /// since it was opened, saved or started.
    File {
        path: Option<String>,
        modified: bool,
        revision: u64,
    },
    /// `truncated` is true when changes after `since` were dropped from the log; the client
    /// should re-read the model (e.g. with `entities`) instead of applying `events`.
    Events {
        revision: u64,
        events: Vec<ChangeEvent>,
        truncated: bool,
    },
}

/// One model change: the revision it produced and what it changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChangeEvent {
    pub revision: u64,
    pub changes: ChangeSummary,
}
