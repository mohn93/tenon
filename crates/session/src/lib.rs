//! The editable document and the command/query layer every client (UI, API, tests) goes through.

pub mod camera;
mod command;
mod document;
mod query;

pub use camera::{Camera, Ray};
pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome};
pub use document::{Document, MAX_COORDINATE};
pub use query::{ChangeEvent, EdgeInfo, FaceInfo, Query, QueryResult, VertexInfo};
