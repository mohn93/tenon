//! A reader for SketchUp `.skp` files, built so that file size is not a limit.
//!
//! The design follows from measuring real files rather than from the format
//! documentation. Two measurements shaped everything here:
//!
//! 1. A 517 MB commercial pack is 380 MB of inline JPEG and 160 MB of
//!    geometry, with no outer compression anywhere. Line art needs none of
//!    the JPEG, so the reader seeks past it. The large file is a 160 MB parse.
//!
//! 2. Component instances are the difference between a model that renders and
//!    one that does not. A 27.7 MB commercial classroom holds 38,019 placed
//!    nodes drawn from 629 unique meshes. Flattened, that is 54,540 primitives
//!    and a tab that never paints a frame; kept as instances it is 77.5 MB of
//!    GPU data. Nothing in this crate flattens an instance.
//!
//! Offsets are `u64` throughout and nothing is read that is not needed.

// The code is upstream's (see VENDORED.md); keep its style rather than
// rewriting it for clippy. Rustc warnings are fixed, not allowed.
#![allow(clippy::all)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod container;
pub mod edges;
pub mod error;
pub mod geom;
pub mod layers;
pub mod legacy;
pub mod materials;
pub mod model;
pub mod pages;
pub mod scene;
pub mod source;
pub mod stream;
pub mod styles;
pub mod tags;
pub mod tlv;
pub mod walk;
pub mod zip;

pub use container::{Container, Generation};
pub use error::{Error, Result};
pub use model::{
    Counts, EdgeBatch, InstanceTable, LayerDef, LengthUnit, MaterialDef, MeshBatch, SceneDef,
    SceneIndex, StyleDef,
};
pub use source::{ByteSource, MemorySource};
