//! The editable document and the command/query layer every client (UI, API, tests) goes through.

pub mod camera;
mod command;
mod document;
pub mod file;
mod query;
mod rectangle;

pub use camera::{Camera, Ray};
pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
pub use document::{Document, MAX_COORDINATE, MAX_COPIES};
pub use query::{
    Bounds, ChangeEvent, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, InstanceInfo, Query,
    QueryResult, VertexInfo,
};
pub use rectangle::{rectangle_axes, rectangle_corners};
