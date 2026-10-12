//! The editable document and the command/query layer every client (UI, API, tests) goes through.

mod arc;
pub mod camera;
mod circle;
mod command;
mod document;
pub mod export;
pub mod file;
mod query;
mod rectangle;
mod rotate;

pub use arc::{DEFAULT_ARC_SEGMENTS, arc_from_bulge, arc_points};
pub use camera::{Camera, Projection, Ray, StandardView};
pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
pub use document::{Document, MAX_COORDINATE, MAX_COPIES};
pub use kernel::CurveKind;
pub use query::{
    Bounds, ChangeEvent, CurveInfo, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo,
    GuideKind, InstanceInfo, Query, QueryResult, VertexInfo,
};
pub use rectangle::{rectangle_axes, rectangle_corners};
pub use rotate::rotation;
