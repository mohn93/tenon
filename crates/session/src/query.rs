use kernel::EntityId;
use serde::{Deserialize, Serialize};

/// Read-only questions about the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Query {
    /// Every vertex, edge and face.
    Entities,
    /// One entity by id ("v3", "e7", "f2").
    Entity {
        id: EntityId,
    },
    Camera,
    Stats,
    History,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VertexInfo {
    pub id: EntityId,
    pub position: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeInfo {
    pub id: EntityId,
    pub vertices: [EntityId; 2],
    pub start: [f64; 3],
    pub end: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceInfo {
    pub id: EntityId,
    pub outer: Vec<EntityId>,
    pub holes: Vec<Vec<EntityId>>,
    pub normal: [f64; 3],
    pub area: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryResult {
    Entities {
        vertices: Vec<VertexInfo>,
        edges: Vec<EdgeInfo>,
        faces: Vec<FaceInfo>,
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
    },
    History {
        undo_depth: usize,
        redo_depth: usize,
    },
}
