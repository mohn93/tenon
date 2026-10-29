//! A SketchUp 2017 (`.skp`) writer, ported from openskp's Python writer (MIT; see
//! `PORTED.md`).
//!
//! A [`Doc`] - materials, tags, component and group definitions, and the root, all in
//! inches - is spliced into a blank document SketchUp itself saved ([`SCAFFOLD`]), at the
//! points openskp found by diffing SDK-authored files. Tags are written as layers after the
//! scaffold's Layer0. The same document always gives the same bytes.

mod archive;
mod doc;
mod png;
mod write;

pub use doc::{
    Definition, Doc, Edge, Face, Geometry, Guide, Instance, Material, Page, PageView, Tag, Texture,
    UvPin,
};
pub use write::{Owner, WriteError, write};

/// The blank SketchUp 2017 document every file is built from: openskp's
/// `_scaffold/blank_v17.skp`, written by the SketchUp SDK. Its splice offsets in
/// `write.rs` hold for these exact bytes only.
pub const SCAFFOLD: &[u8] = include_bytes!("blank_v17.skp");
