use kernel::{ChangeSummary, EntityId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Read-only questions about the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// Vertices, edges and faces. An entity is returned when `ids` is empty or lists it, and
    /// `kinds` is empty or lists its kind; unknown ids are simply absent from the result.
    Entities {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        kinds: Vec<EntityKind>,
    },
    /// One entity by id ("v3", "e7", "f2").
    Entity {
        id: EntityId,
    },
    Camera,
    Stats,
    History,
    /// Model changes after `since` (a revision number), oldest first.
    Events {
        since: u64,
    },
}

/// The three kinds of entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Vertex,
    Edge,
    Face,
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FaceInfo {
    pub id: EntityId,
    pub outer: Vec<EntityId>,
    pub holes: Vec<Vec<EntityId>>,
    pub normal: [f64; 3],
    pub area: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryResult {
    Entities {
        vertices: Vec<VertexInfo>,
        edges: Vec<EdgeInfo>,
        faces: Vec<FaceInfo>,
        revision: u64,
    },
    Vertex(VertexInfo),
    Edge(EdgeInfo),
    Face(FaceInfo),
    Camera {
        eye: [f64; 3],
        target: [f64; 3],
        fov_y_deg: f64,
    },
    Stats {
        vertices: usize,
        edges: usize,
        faces: usize,
        revision: u64,
    },
    History {
        undo_depth: usize,
        redo_depth: usize,
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
