//! The editable document and the command/query layer every client (UI, API, tests) goes through.

mod arc;
mod batch;
pub mod camera;
mod circle;
mod command;
mod document;
pub mod export;
pub mod file;
mod import;
mod polygon;
mod query;
mod rectangle;
mod rotate;
mod schema;
mod skp;
mod skp_export;
mod sniff;
pub mod texture;

pub use arc::{DEFAULT_ARC_SEGMENTS, arc_from_bulge, arc_points, arc_through};
pub use batch::{BatchCommand, BatchRef, RefKind, STAND_IN_BASE, check_references};
pub use camera::{Camera, Lighting, Projection, Projector, Ray, StandardView};
pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
pub use command::{
    BatchError, Command, CommandError, ErrorKind, ExpectedStep, Origin, Outcome, SelectMode,
    TextureSource,
};
pub use document::{Document, MAX_COORDINATE, MAX_COPIES, check_coordinates, unit_vector};
pub use export::ExportReport;
pub use import::{
    ImportAs, ImportFormat, ImportReport, ImportUnits, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
    MAX_IMPORT_MATERIALS, MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS, UpAxis,
};
pub use kernel::{CurveKind, ImageFormat, MaterialId, Side, TexturePlacement};
pub use polygon::{DEFAULT_POLYGON_SIDES, polygon_points};
pub use query::{
    Bounds, ChangeEvent, CurveInfo, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo,
    GuideKind, InstanceInfo, MaterialInfo, OutlinerNode, OutlinerRow, OutlinerTable, Query,
    QueryResult, TextureInfo, VertexInfo,
};
pub use rectangle::{rectangle_axes, rectangle_corners};
pub use rotate::rotation;
pub use schema::ModelPoint;
