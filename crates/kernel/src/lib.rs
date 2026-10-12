//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

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
mod transform;
mod triangles;
mod weld;

pub use curves::{Curve, CurveCircle, CurveKind};
pub use data::{
    DefinitionData, EdgeData, FaceData, GuideData, InstanceData, MAX_ID, MaterialData, MeshData,
    SceneData, VertexData,
};
pub use edit::KernelError;
pub use ids::{
    CurveId, DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, MaterialId, VertexId,
};
pub use import::{MeshPolygon, MeshReport};
pub use index::{Index, Shape};
pub use material::{MAX_MATERIAL_NAME, Material, Side};
pub use model::{ChangeSet, ChangeSummary, Edge, Face, Guide, Instance, Model, Op, Vertex};
pub use scene::{Definition, DefinitionKind, MeshId, Scene, SceneChange, Step};
pub use triangles::Triangle;
