use kernel::{ChangeSummary, DefinitionId, DefinitionKind, EntityId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Read-only questions about the document. Entities and coordinates are those of the open
/// context (see `Command::OpenInstance`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// Vertices, edges, faces and instances. An entity is returned when `ids` is empty or lists
    /// it, `kinds` is empty or lists its kind, and `within` is absent or contains all its
    /// vertices (an instance: its bounding box); unknown ids are simply absent from the result.
    Entities {
        #[serde(default)]
        ids: Vec<EntityId>,
        #[serde(default)]
        kinds: Vec<EntityKind>,
        #[serde(default)]
        within: Option<Bounds>,
    },
    /// One entity by id ("v3", "e7", "f2", "i4").
    Entity {
        id: EntityId,
    },
    Camera,
    Stats,
    History,
    /// The selected edges, faces and instances.
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
        revision: u64,
    },
    Vertex(VertexInfo),
    Edge(EdgeInfo),
    Face(FaceInfo),
    Instance(InstanceInfo),
    Camera {
        eye: [f64; 3],
        target: [f64; 3],
        fov_y_deg: f64,
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
