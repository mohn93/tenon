//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

mod edit;
mod faces;
pub mod geom;
mod ids;
mod model;
mod regions;

pub use edit::KernelError;
pub use ids::{EdgeId, EntityId, FaceId, VertexId};
pub use model::{ChangeSet, ChangeSummary, Edge, Face, Model, Op, Vertex};
