//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

mod connected;
mod curves;
mod data;
mod edit;
mod erase;
mod faces;
pub mod geom;
mod group;
mod guides;
mod ids;
mod import;
mod index;
mod material;
mod mesh;
mod model;
mod push_pull;
mod regions;
mod scene;
mod smooth;
mod soften;
mod texture;
mod transform;
mod triangles;
mod weld;

pub use curves::{Curve, CurveCircle, CurveKind};
pub use data::{
    DefinitionData, EdgeData, FaceData, GuideData, InstanceData, MAX_ID, MIN_DETERMINANT,
    MaterialData, MeshData, SceneData, TextureData, VertexData,
};
pub use edit::KernelError;
pub use ids::{
    CurveId, DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, MaterialId, VertexId,
};
pub use import::{MAX_IMPORT_FACES, MeshPolygon, MeshReport};
pub use index::{Index, Shape};
pub use material::{MAX_EMISSION, MAX_MATERIAL_NAME, Material, MaterialEdit, Side};
pub use model::{ChangeSet, ChangeSummary, Edge, Face, Guide, Instance, Model, Op, Vertex};
pub use scene::{Definition, DefinitionKind, MeshId, Scene, SceneChange, Step};
pub use smooth::{ShadedTriangle, SmoothNormals};
pub use soften::SoftenReport;
pub use texture::{
    ImageFormat, MAX_TEXTURE_OFFSET, MAX_TEXTURE_SCALE, MAX_TEXTURE_SIZE, MAX_TEXTURES,
    MIN_TEXTURE_SCALE, MIN_TEXTURE_SIZE, PlacementEdit, Texture, TextureImage, TexturePlacement,
    back_texture_position, normal_angle, texture_axes, texture_position,
};
pub use triangles::Triangle;
