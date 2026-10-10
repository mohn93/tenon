//! Geometry kernel: vertices, edges and faces with SketchUp-style sticky geometry.
//! No GPU or UI dependencies.

pub mod geom;
mod ids;
mod model;

pub use ids::{EdgeId, EntityId, FaceId, VertexId};
pub use model::{ChangeSet, ChangeSummary, Edge, Face, Model, Op, Vertex};
