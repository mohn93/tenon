//! Interactive tools and inference. Tools turn pointer input into `session::Command`s; they never
//! edit the model directly.

pub mod inference;

pub use inference::{Axis, Inference, SnapKind, infer};
