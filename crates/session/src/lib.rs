//! The editable document and the command/query layer every client (UI, API, tests) goes through.

pub mod camera;
mod command;
mod document;
mod query;

pub use camera::{Camera, Ray};
pub use command::{Command, CommandError, ErrorKind, Outcome};
pub use document::Document;
pub use query::{EdgeInfo, FaceInfo, Query, QueryResult, VertexInfo};
