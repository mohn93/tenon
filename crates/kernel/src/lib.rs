//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

mod annotation;
mod axes;
mod clip;
mod connected;
mod curves;
mod data;
mod edit;
mod erase;
mod faces;
pub mod geom;
mod group;
mod guides;
mod hide;
mod ids;
mod import;
mod import_tree;
mod index;
pub mod intersect;
mod location;
mod material;
mod mesh;
mod model;
pub mod offset;
mod orient;
mod page;
mod push_pull;
mod regions;
mod scene;
mod section;
mod section_loops;
mod smooth;
mod soften;
pub mod sweep;
mod tag;
mod texture;
mod transform;
mod triangles;
mod weld;

pub use annotation::{
    Anchor, Circle, Dimension, MAX_NOTE_CHARS, MAX_NOTE_LINES, Note, RadialAnchor, check_note_text,
};
pub use axes::Axes;
pub use clip::Inserted;
pub use curves::{Curve, CurveCircle, CurveKind};
pub use data::{
    AnchorData, AxesData, CircleData, DefinitionData, DimensionData, EdgeData, FaceData, GuideData,
    InstanceData, LocationData, MAX_ID, MIN_DETERMINANT, MaterialData, MeshData, NoteData,
    PageData, RadialAnchorData, SceneData, SectionData, TagData, TextureData, VertexData,
};
pub use edit::KernelError;
pub use ids::{
    CurveId, DefinitionId, DimensionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, MaterialId,
    NoteId, PageId, SectionId, TagId, VertexId,
};
pub use import::{MAX_IMPORT_FACES, MeshPolygon, MeshReport};
pub use import_tree::{
    ImportDefinition, ImportEdge, ImportFace, ImportGeometry, ImportInstance, ImportTree,
    MAX_IMPORT_DEFINITIONS, MAX_IMPORT_DEPTH, MAX_IMPORT_INSTANCES, MAX_IMPORT_PLACED_FACES,
    MAX_IMPORT_PLACEMENTS, TreeReport,
};
pub use index::{Index, Shape, ray_hits_triangle};
pub use location::Location;
pub use material::{MAX_EMISSION, MAX_MATERIAL_NAME, Material, MaterialEdit, Side};
pub use model::{ChangeSet, ChangeSummary, Edge, Face, Guide, Instance, Model, Op, Vertex};
pub use orient::OrientReport;
pub use page::{MAX_PAGE_NAME, PAGE_NAME_BASE, Page, PageView, SHADOW_YEARS, Shadows};
pub use scene::{Definition, DefinitionKind, MeshId, Parts, Scene, SceneChange, Step};
pub use section::SectionPlane;
pub use section_loops::{SectionLoops, section_loops, triangulate_section};
pub use smooth::{ShadedTriangle, SmoothNormals};
pub use soften::SoftenReport;
pub use tag::{MAX_TAG_NAME, TAG_COLORS, Tag, TagEdit, UNTAGGED, tag_shown_among};
pub use texture::{
    ImageFormat, MAX_TEXTURE_OFFSET, MAX_TEXTURE_SCALE, MAX_TEXTURE_SIZE, MAX_TEXTURES,
    MIN_TEXTURE_SCALE, MIN_TEXTURE_SIZE, PlacementEdit, Texture, TextureImage, TexturePlacement,
    back_texture_position, normal_angle, texture_axes, texture_position,
};
pub use triangles::Triangle;
