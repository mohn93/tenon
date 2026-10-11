//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

mod edit;
mod erase;
mod faces;
pub mod geom;
mod ids;
mod mesh;
mod model;
mod push_pull;
mod regions;
mod transform;
mod weld;

pub use edit::KernelError;
pub use ids::{EdgeId, EntityId, FaceId, VertexId};
pub use model::{ChangeSet, ChangeSummary, Edge, Face, Model, Op, Vertex};
