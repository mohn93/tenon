//! The editable document and the command/query layer every client (UI, API, tests) goes through.

mod annotation_shape;
mod arc;
mod batch;
pub mod camera;
mod circle;
pub mod clip;
mod command;
mod document;
pub mod drawing;
pub mod export;
pub mod file;
pub mod fspath;
mod import;
mod polygon;
mod query;
mod rectangle;
mod rotate;
mod schema;
mod section_shape;
mod skp;
mod skp_export;
mod sniff;
mod svg;
pub mod text3d;
pub mod texture;
pub mod units;
#[cfg(windows)]
pub mod windows;

pub use annotation_shape::AnnotationShape;
pub use arc::{DEFAULT_ARC_SEGMENTS, arc_from_bulge, arc_points, arc_through};
pub use batch::{BatchCommand, BatchRef, RefKind, STAND_IN_BASE, check_references};
pub use camera::{Camera, Lighting, Projection, Projector, Ray, StandardView};
pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
pub use clip::ClipOutline;
pub use command::{
    AnchorInput, BatchError, Command, CommandError, ErrorKind, ExpectedStep, Origin, Outcome,
    SelectMode, TextureSource,
};
pub use document::{
    Document, MAX_COORDINATE, MAX_COPIES, SectionDisplay, check_coordinates, dimension_label,
    dimension_value, note_label, unit_vector,
};
pub use export::ExportReport;
pub use import::{
    ImportAs, ImportFormat, ImportReport, ImportUnits, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
    MAX_IMPORT_MATERIALS, MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS, UpAxis,
};
pub use kernel::{CurveKind, ImageFormat, MaterialId, PageId, Side, TagId, TexturePlacement};
pub use polygon::{DEFAULT_POLYGON_SIDES, polygon_points};
pub use query::{
    AnchorInfo, Bounds, ChangeEvent, CurveInfo, DefinitionInfo, DimensionInfo, DimensionKind,
    EdgeInfo, EntityKind, FaceInfo, GuideInfo, GuideKind, InstanceInfo, MaterialInfo, NoteInfo,
    NoteKind, OutlinerNode, OutlinerRow, OutlinerTable, Query, QueryResult, SceneInfo, SceneView,
    SectionPlaneInfo, TagInfo, TextureInfo, VertexInfo,
};
pub use rectangle::{rectangle_axes, rectangle_corners};
pub use rotate::rotation;
pub use schema::ModelPoint;
pub use section_shape::{MAX_SECTION_OUTLINES, MIN_SECTION_SIDE, SECTION_SIDE_SCALE, SectionShape};
pub use text3d::{SkippedCharacter, Text3dReport};
pub use units::{LengthUnit, MAX_PRECISION, Units, format_length_in};
