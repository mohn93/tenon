//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

mod edit;
mod erase;
mod faces;
pub mod geom;
mod group;
mod ids;
mod index;
mod mesh;
mod model;
mod push_pull;
mod regions;
mod scene;
mod transform;
mod weld;

pub use edit::KernelError;
pub use ids::{DefinitionId, EdgeId, EntityId, FaceId, InstanceId, VertexId};
pub use index::{Index, Shape};
pub use model::{ChangeSet, ChangeSummary, Edge, Face, Instance, Model, Op, Vertex};
pub use scene::{Definition, DefinitionKind, MeshId, Scene, SceneChange, Step};
