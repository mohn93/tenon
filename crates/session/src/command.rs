use std::fmt;

use kernel::{ChangeSummary, EntityId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Every state change a client can make. Values are explicit (meters, degrees, entity ids);
/// commands never refer to screen positions or cursor state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Draw a straight edge. Sticky-geometry rules apply (merging, splitting, auto faces).
    AddEdge {
        start: [f64; 3],
        end: [f64; 3],
    },
    /// Draw a rectangle from one corner to the opposite one, as one undo step. Its sides follow
    /// the world axes on axis planes. `normal` picks the plane; without it the corners must
    /// differ in exactly two coordinates.
    AddRectangle {
        corner: [f64; 3],
        opposite: [f64; 3],
        #[serde(default)]
        normal: Option<[f64; 3]>,
    },
    /// Move a face along its normal by `distance` meters (negative: against the normal),
    /// extruding new sides or stretching the faces around it.
    PushPull {
        face: EntityId,
        distance: f64,
    },
    Undo,
    Redo,
    /// Place the camera. Not recorded in undo history.
    SetCamera {
        eye: [f64; 3],
        target: [f64; 3],
        fov_y_deg: Option<f64>,
    },
}

impl Command {
    /// True for commands that can change the model (everything except camera moves).
    pub fn changes_model(&self) -> bool {
        !matches!(self, Command::SetCamera { .. })
    }
}

/// What a successful command changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Outcome {
    pub changes: ChangeSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    NothingToUndo,
    NothingToRedo,
    NotFound,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandError {
    pub kind: ErrorKind,
    pub message: String,
    /// Entities the error refers to, when there are any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<EntityId>,
}

impl CommandError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            ids: Vec::new(),
        }
    }

    pub fn with_ids(mut self, ids: Vec<EntityId>) -> Self {
        self.ids = ids;
        self
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for CommandError {}

/// A batch stopped at command `index`. `completed` holds the outcomes of the commands before it;
/// for an atomic batch they were rolled back and nothing changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BatchError {
    pub index: usize,
    pub error: CommandError,
    pub completed: Vec<Outcome>,
    pub rolled_back: bool,
}
