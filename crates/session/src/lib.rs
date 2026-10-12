//! The editable document and the command/query layer every client (UI, API, tests) goes through.

pub mod camera;
mod circle;
mod command;
mod document;
pub mod export;
pub mod file;
mod query;
mod rectangle;
mod rotate;

pub use camera::{Camera, Ray};
pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
pub use document::{Document, MAX_COORDINATE, MAX_COPIES};
pub use query::{
    Bounds, ChangeEvent, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo, GuideKind,
    InstanceInfo, Query, QueryResult, VertexInfo,
};
pub use rectangle::{rectangle_axes, rectangle_corners};
pub use rotate::rotation;
