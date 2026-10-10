use std::fmt;

use kernel::ChangeSummary;
use serde::{Deserialize, Serialize};

/// Every state change a client can make. Values are explicit (meters, degrees, entity ids);
/// commands never refer to screen positions or cursor state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Draw a straight edge. Sticky-geometry rules apply (merging, splitting, auto faces).
    AddEdge {
        start: [f64; 3],
        end: [f64; 3],
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

/// What a successful command changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub changes: ChangeSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    NothingToUndo,
    NothingToRedo,
    NotFound,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandError {
    pub kind: ErrorKind,
    pub message: String,
}

impl CommandError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for CommandError {}
