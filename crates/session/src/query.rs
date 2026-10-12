use kernel::{ChangeSummary, CurveKind, DefinitionId, DefinitionKind, EntityId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::camera::Projection;

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
    /// The open context: the path of instances being edited, from the root.
    Context,
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
    /// whole view's height).
    Camera {
        eye: [f64; 3],
        target: [f64; 3],
        up: [f64; 3],
        fov_y_deg: f64,
        projection: Projection,
        view_height: f64,
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
    History {
        undo_depth: usize,
        redo_depth: usize,
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
    /// `path` lists the open instances from the root; empty when the root is open.
    /// `definition` is the definition being edited.
    Context {
        path: Vec<EntityId>,
        definition: Option<DefinitionId>,
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
