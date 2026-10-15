//! The editable document and the command/query layer every client (UI, API, tests) goes through.

mod arc;
pub mod camera;
mod circle;
mod command;
mod document;
pub mod export;
pub mod file;
mod import;
mod query;
mod rectangle;
mod rotate;
pub mod texture;

pub use arc::{DEFAULT_ARC_SEGMENTS, arc_from_bulge, arc_points};
pub use camera::{Camera, Lighting, Projection, Projector, Ray, StandardView};
pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
pub use command::{
    BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode, TextureSource,
};
pub use document::{Document, MAX_COORDINATE, MAX_COPIES, check_coordinates, unit_vector};
pub use export::ExportReport;
pub use import::{
    ImportAs, ImportFormat, ImportReport, ImportUnits, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
    MAX_IMPORT_MATERIALS, MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS, UpAxis,
};
pub use kernel::{CurveKind, ImageFormat, MaterialId, Side, TexturePlacement};
pub use query::{
    Bounds, ChangeEvent, CurveInfo, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo,
    GuideKind, InstanceInfo, MaterialInfo, OutlinerNode, OutlinerRow, OutlinerTable, Query,
    QueryResult, TextureInfo, VertexInfo,
};
pub use rectangle::{rectangle_axes, rectangle_corners};
pub use rotate::rotation;
