//! Geometry, as flat arrays.
//!
//! One `Component` record holds a definition's whole mesh: its vertices, its
//! edges, its faces with their loops, and the instances it places. This module
//! turns that into struct-of-arrays form and nothing else - no triangulation
//! decisions, no world transforms, no material resolution. Those belong to
//! callers who know what they are drawing.
//!
//! The record layout below was not inferred. It was read off real files with
//! SketchUp's own tag names in hand, which is why this module can say things
//! like "FacePlane is four doubles" with confidence where earlier readers took
//! three and dropped the plane constant.
//!
//! ```text
//! Component
//!   DrawingElement
//!     Entity { EntityPID, EntityAttributes { AttributeContainer { ... } } }
//!     DrawingElementMaterialPIDRef, DrawingElementLayerPIDRef,
//!     DrawingElementFlags
//!   ComponentVertices -> Vertex { Entity { EntityPID }, VertexPosition[3xf64] }
//!   ComponentEdges    -> Edge { DrawingElement, EdgeStartVertexPIDRef,
//!                               EdgeEndVertexPIDRef }
//!   ComponentFaces    -> Face { DrawingElement, FaceBackMaterialPIDRef,
//!                               FacePlane[4xf64],
//!                               FaceLoops -> Loop -> LoopEdgeUses ->
//!                                 EdgeUse { EdgeUseEdgePIDRef,
//!                                           EdgeUseReversed } }
//!   ComponentInstances -> ComponentInstance
//!   ComponentGroups    -> Group -> ComponentInstance
//! ```
//!
//! ## Where a face's texture mapping lives
//!
//! Dumped out of the commercial school pack, in full, because every earlier
//! reader names these records by their hex bytes and gets one of them wrong:
//!
//! ```text
//! Face
//!   DrawingElement
//!     Entity
//!       EntityPID
//!       EntityAttributes
//!         AttributeContainer
//!           AttributeContainerAttributes
//!             FaceTextureCoords                 (0x2710)
//!               Attribute                       (0x36b0, empty base record)
//!               FaceTextureCoordsFront          (0x2711)
//!                 FaceSideTextureInfo           (0x2713)
//!                   FaceSideTextureInfoFlags      (0x2714) u32
//!                   FaceSideTextureInfoTransform  (0x2715) 9 x f64
//!                   FaceSideTextureInfoProjVec    (0x2716) 3 x f64
//!                   FaceSideTextureInfoPushPins   (0x2717) 0 or 200 bytes
//!               FaceTextureCoordsBack           (0x2712)
//!                 ... the same five records again
//! ```
//!
//! openskp reaches the same transform through a tag it calls "DC05", which is
//! the little-endian spelling of 0x05DC - the `Entity` base record, not
//! anything to do with textures. Reading it by the name SketchUp uses is why
//! the rest of this comment can be specific.
//!
//! What the records mean, measured over five files and 530,000 faces:
//!
//! - **Flags bit 0 is "this side has a positioned mapping".** It and a
//!   non-identity transform agree in 96,734 of 96,736 sides of the school
//!   pack. With the bit clear the transform is the identity, which under the
//!   recipe below is exactly SketchUp's default projection - so no branch is
//!   needed, the identity does the right thing.
//! - **Flags bit 1 is "projected".** Every side that sets it has a non-zero
//!   `FaceSideTextureInfoProjVec`; no side that clears it and has a zero
//!   `ProjVec` behaves differently. 43,713 of the school pack's sides are
//!   projected.
//! - **`ProjVec` and `PushPins` are not needed to reproduce the mapping, and
//!   that contradicts openskp's docs, which say projected textures can only
//!   be approximated because "the projection-plane basis isn't captured".**
//!   It is captured, in the transform itself: projecting one plane onto
//!   another along a fixed direction is a projective map of the plane, and a
//!   3x3 homogeneous matrix represents one exactly. SketchUp stores that
//!   matrix per face and per side, including for projected and photo-matched
//!   textures - 7,857 of the school pack's matrices have a genuinely
//!   non-trivial third column, i.e. are projective rather than affine, and
//!   they are the projected ones. `PushPins` is a fixed 200-byte block (the
//!   texture-positioning tool's four handles); nothing renders from it.
//!
//! The recipe, confirmed against openskp's own *writer* (`solveUvMatrix` in
//! `packages/typescript/src/create.ts`, which fits this matrix from three
//! known point/uv pairs) rather than against its reader:
//!
//! 1. Project the vertex into the face plane with [`uv_axes`], giving
//!    `(px, py)` in the file's own units.
//! 2. `(s, t, w) = (px, py, 1) * M^-1`, row-vector convention.
//! 3. The texture coordinate is `(s/w, t/w)`, in the same length unit as the
//!    positions. Divide by the material's tile size to get tile repeats.
//!
//! Step 3 is why the identity matrix is the default projection: it leaves
//! `(px, py)` alone, and `px / tile` is what SketchUp paints on a face nobody
//! has positioned a texture on. Measured, that is what the native files hold:
//! the 2x2 block of `M` has a median scale of 1.0000 in both SketchUp-authored
//! fixtures, and only the ones exported from other tools (Shapespark's baked
//! atlases, 3ds Max via V-Ray) carry scales from 0.06 to 1577.

use crate::error::{Error, Result};
use crate::model::NO_MATERIAL;
use crate::tags as t;
use crate::tlv::{Record, Tlv};
use alloc::string::String;
use alloc::vec::Vec;

/// A reference to a layer that has not been resolved to an index yet.
pub const NO_LAYER: u32 = u32::MAX;

/// A face with no positioned texture mapping, which means the default
/// projection applies. See [`UvMap`].
pub const NO_UV: u32 = u32::MAX;

/// `FaceSideTextureInfoFlags`, which is a `u32` and uses two bits of it.
pub mod uv_flag {
    /// This side carries a positioned mapping. With it clear the transform is
    /// the identity, which is the default projection, so nothing branches on
    /// it - it is here because it is what the file says.
    pub const POSITIONED: u32 = 0x01;
    /// The texture is projected rather than wrapped. Every side that sets this
    /// also carries a non-zero `FaceSideTextureInfoProjVec`, and the per-face
    /// transform already encodes the projection exactly, so this too is
    /// informational.
    pub const PROJECTED: u32 = 0x02;
}

/// The common header every drawable carries, flattened.
#[derive(Debug, Clone, Copy, Default)]
pub struct Common {
    pub pid: u64,
    pub material: u64,
    pub layer: u64,
    /// `DrawingElementFlags`. Base value is 0x06; bit 0 is hidden, bit 3 soft,
    /// bit 4 smooth. Bits 1 and 2 are set on effectively everything and their
    /// meaning is not established.
    pub flags: u8,
    pub has_material: bool,
    pub has_layer: bool,
}

fn varint(r: &Record) -> u64 {
    // An id's width is the record's own length - the format spends two bytes
    // on a small id and more when it needs to.
    let n = r.payload.len().min(8);
    let mut v = 0u64;
    for (i, b) in r.payload[..n].iter().enumerate() {
        v |= (*b as u64) << (8 * i);
    }
    v
}

/// An entity's layer reference as the 32-bit layer id `parse_layers` gives: [`NO_LAYER`]
/// when it has none, or when its pid does not fit 32 bits (a layer's own id saturates there,
/// so such a reference could only name the wrong layer).
pub fn layer_ref(c: &Common) -> u32 {
    if c.has_layer {
        u32::try_from(c.layer).unwrap_or(NO_LAYER)
    } else {
        NO_LAYER
    }
}

/// Read `Entity`/`DrawingElement` state out of whichever of the two wraps it.
///
/// A `Vertex` carries a bare `Entity`; everything drawable carries a
/// `DrawingElement` that contains one. Handling both here keeps the per-kind
/// parsers from each repeating the walk.
pub fn common_of(rec: &Record) -> Common {
    let mut c = Common::default();
    for child in rec.children() {
        match child.tag {
            t::ENTITY => {
                for e in child.children() {
                    if e.tag == t::ENTITY_PID {
                        c.pid = varint(&e);
                    }
                }
            }
            t::ENTITY_PID => c.pid = varint(&child),
            t::DRAWING_ELEMENT => {
                let inner = common_of(&child);
                c.pid = if inner.pid != 0 { inner.pid } else { c.pid };
                if inner.has_material {
                    c.material = inner.material;
                    c.has_material = true;
                }
                if inner.has_layer {
                    c.layer = inner.layer;
                    c.has_layer = true;
                }
                c.flags = inner.flags;
            }
            t::DRAWING_ELEMENT_MATERIAL_PID_REF => {
                c.material = varint(&child);
                c.has_material = true;
            }
            t::DRAWING_ELEMENT_LAYER_PID_REF => {
                c.layer = varint(&child);
                c.has_layer = true;
            }
            t::DRAWING_ELEMENT_FLAGS => {
                if let Some(b) = child.payload.first() {
                    c.flags = *b;
                }
            }
            _ => {}
        }
    }
    c
}

/// One placement of a definition. Never geometry.
#[derive(Debug, Clone, Default)]
pub struct RawInstance {
    pub pid: u64,
    pub name: String,
    /// PID of the definition this places. Resolved later, because a definition
    /// can be placed before it is read.
    pub def_pid: u64,
    pub guid: [u8; 16],
    /// Nine row-major rotation/scale terms, three translation terms, and a
    /// divisor. Measured across 243 real transforms the divisor is always 1,
    /// but it is applied anyway: a file where it is not would otherwise be
    /// silently misplaced rather than visibly wrong.
    pub m: [f64; 13],
    pub material: u64,
    pub has_material: bool,
    pub layer: u64,
    pub has_layer: bool,
    pub flags: u8,
    /// A `Group` is a `ComponentInstance` that happens to be unique. The
    /// distinction matters for naming and for the outliner, not for drawing.
    pub is_group: bool,
    /// An imported photograph, placed the same way a component is. Its
    /// definition is a textured rectangle.
    pub is_image: bool,
}

impl RawInstance {
    /// Row-major 4x4 with the translation converted to millimetres, which is
    /// what the contract in `model.rs` asks for. The rotation terms are
    /// unitless so they pass through.
    pub fn matrix4_mm(&self) -> [f32; 16] {
        let mut m = self.matrix4();
        m[3] *= crate::model::INCH_TO_MM as f32;
        m[7] *= crate::model::INCH_TO_MM as f32;
        m[11] *= crate::model::INCH_TO_MM as f32;
        m
    }

    /// Row-major 4x4 in the file's own units (inches).
    pub fn matrix4(&self) -> [f32; 16] {
        let d = if self.m[12] != 0.0 { self.m[12] } else { 1.0 };
        let m = &self.m;
        [
            (m[0] / d) as f32,
            (m[1] / d) as f32,
            (m[2] / d) as f32,
            (m[9] / d) as f32,
            (m[3] / d) as f32,
            (m[4] / d) as f32,
            (m[5] / d) as f32,
            (m[10] / d) as f32,
            (m[6] / d) as f32,
            (m[7] / d) as f32,
            (m[8] / d) as f32,
            (m[11] / d) as f32,
            0.0,
            0.0,
            0.0,
            1.0,
        ]
    }

    pub fn determinant(&self) -> f64 {
        let m = &self.m;
        m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6])
    }
}

/// Compose two 13-term transforms, parent first.
pub fn compose(parent: &[f64; 13], child: &[f64; 13]) -> [f64; 13] {
    let (p, c) = (parent, child);
    let mut o = [0.0f64; 13];
    for r in 0..3 {
        for q in 0..3 {
            o[r * 3 + q] = p[r * 3] * c[q] + p[r * 3 + 1] * c[3 + q] + p[r * 3 + 2] * c[6 + q];
        }
    }
    for r in 0..3 {
        o[9 + r] = p[r * 3] * c[9] + p[r * 3 + 1] * c[10] + p[r * 3 + 2] * c[11] + p[9 + r];
    }
    o[12] = p[12] * c[12];
    o
}

pub const IDENTITY13: [f64; 13] = [
    1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0,
];

/// A definition's geometry in struct-of-arrays form.
///
/// Positions stay `f64` here because the file stores `f64` and narrowing in
/// the reader would quietly change the model; the conversion to `f32` happens
/// once, on the way to the GPU, where it is a rendering decision rather than a
/// parsing one.
#[derive(Debug, Default)]
pub struct DefGeometry {
    /// Sorted vertex pids, parallel to `vertex_pos`. Sorted so that an edge's
    /// reference resolves by binary search instead of by hash lookup - ids
    /// arrive nearly in order, so this stays cache-friendly.
    pub vertex_pid: Vec<u64>,
    /// Three f64 per vertex, in the definition's own coordinates, inches.
    pub vertex_pos: Vec<f64>,

    pub edge_pid: Vec<u64>,
    /// Two vertex indices per edge. `u32::MAX` where the reference dangles.
    pub edge_v: Vec<u32>,
    pub edge_flags: Vec<u8>,
    pub edge_layer: Vec<u32>,

    /// Three f64 per face: the plane normal.
    pub face_normal: Vec<f64>,
    /// The plane's fourth term. Earlier readers drop this; it is free here and
    /// it is what lets a point be tested against a face's plane exactly.
    pub face_plane_d: Vec<f64>,
    pub face_material: Vec<u32>,
    pub face_back_material: Vec<u32>,
    pub face_layer: Vec<u32>,
    pub face_flags: Vec<u8>,
    /// Index into `uv_xf` per face, or [`NO_UV`]. Sparse on purpose: on the
    /// school pack 47,919 of 96,736 face sides carry no mapping at all, and a
    /// dense table of nine doubles per face would be 72 bytes each for a
    /// matrix that is the identity.
    ///
    /// Front side only. A face's back side has its own transform in the file
    /// and it is read to confirm the layout, but one vertex carries one UV and
    /// the renderer paints a face from its front material, so a second set
    /// would have no consumer.
    pub face_uv: Vec<u32>,
    /// Nine f64 per entry, row-major: SketchUp's texture-space-to-face-plane
    /// matrix, possibly projective. See the module comment for the recipe.
    pub uv_xf: Vec<f64>,
    /// `FaceSideTextureInfoFlags` for the front side, parallel to `face_uv`.
    pub uv_flags: Vec<u32>,
    /// CSR: face i owns loops `face_loop_ofs[i] .. face_loop_ofs[i+1]`.
    pub face_loop_ofs: Vec<u32>,
    /// CSR: loop j owns coedges `loop_ofs[j] .. loop_ofs[j+1]`.
    pub loop_ofs: Vec<u32>,
    /// Edge index per coedge.
    pub coedge_edge: Vec<u32>,
    /// Whether this use traverses its edge backwards. Needed twice over: to
    /// order a loop's vertices, and to orient the face normal that the
    /// silhouette test consumes.
    pub coedge_reversed: Vec<u8>,

    pub instances: Vec<RawInstance>,
    /// Guide lines and guide points, in the definition's own coordinates (inches).
    pub guides: Vec<RawGuide>,
    /// The layer of each guide, parallel to `guides`: a 2021+ file's layer pid (cut to 32
    /// bits), a legacy file's layer index, or [`NO_LAYER`].
    pub guide_layer: Vec<u32>,
    /// The hidden guides of this definition, which are not in `guides`, and the layer of
    /// each (as `guide_layer`).
    pub hidden_guide_items: Vec<RawGuide>,
    pub hidden_guide_layer: Vec<u32>,
    /// 2021+ files: the number of hidden guides in this definition (they are in
    /// `hidden_guide_items`, not in `guides`). Legacy files count theirs in
    /// `legacy::ReadStats::hidden_guides` and leave this 0.
    pub hidden_guides: u32,
    /// 2021+ files: records in this definition's guide lists that were not read as a guide
    /// (an unknown record, or a guide without its point or line), skipped. Always 0 for
    /// legacy files.
    pub unread_guides: u32,
}

/// A guide (construction line or point) as the file states it, in inches. A guide line is
/// infinite: a line the file bounds by start and end parameters is still a line through
/// `point` along `direction`. Values are as read; a consumer checks them (finite, a
/// direction that is not zero) before use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RawGuide {
    Line {
        point: [f64; 3],
        direction: [f64; 3],
    },
    Point {
        position: [f64; 3],
    },
}

impl DefGeometry {
    pub fn vertices(&self) -> usize {
        self.vertex_pid.len()
    }
    pub fn edges(&self) -> usize {
        self.edge_pid.len()
    }
    pub fn faces(&self) -> usize {
        self.face_flags.len()
    }
    pub fn loops(&self) -> usize {
        self.loop_ofs.len().saturating_sub(1)
    }

    pub fn vertex(&self, i: u32) -> [f64; 3] {
        let k = i as usize * 3;
        [
            self.vertex_pos[k],
            self.vertex_pos[k + 1],
            self.vertex_pos[k + 2],
        ]
    }

    pub fn normal(&self, f: usize) -> [f64; 3] {
        [
            self.face_normal[f * 3],
            self.face_normal[f * 3 + 1],
            self.face_normal[f * 3 + 2],
        ]
    }

    /// This face's own texture-space-to-face-plane matrix, if it has one.
    pub fn uv_matrix(&self, f: usize) -> Option<[f64; 9]> {
        let i = *self.face_uv.get(f)? as usize;
        if i == NO_UV as usize {
            return None;
        }
        let s = self.uv_xf.get(i * 9..i * 9 + 9)?;
        let mut m = [0.0f64; 9];
        m.copy_from_slice(s);
        Some(m)
    }

    /// How this face's texture is mapped: positioned, projected, or neither.
    pub fn uv_flags_of(&self, f: usize) -> u32 {
        let Some(&i) = self.face_uv.get(f) else {
            return 0;
        };
        if i == NO_UV {
            0
        } else {
            self.uv_flags.get(i as usize).copied().unwrap_or(0)
        }
    }

    /// The mapping for one face, ready to evaluate per vertex.
    pub fn uv_map(&self, f: usize) -> UvMap {
        UvMap::new(self.normal(f), self.uv_matrix(f).as_ref())
    }

    fn vertex_index(&self, pid: u64) -> u32 {
        match self.vertex_pid.binary_search(&pid) {
            Ok(i) => i as u32,
            Err(_) => u32::MAX,
        }
    }

    fn edge_index(&self, pid: u64) -> u32 {
        match self.edge_pid.binary_search(&pid) {
            Ok(i) => i as u32,
            Err(_) => u32::MAX,
        }
    }

    /// The ordered ring of vertex indices for one loop.
    ///
    /// A loop is stored as edge *uses*, not as vertices, so the ring has to be
    /// walked: each use contributes its edge's start or end depending on
    /// whether it is reversed. A dangling edge reference aborts the loop
    /// rather than producing a ring with a hole in it.
    pub fn loop_ring(&self, loop_idx: usize, out: &mut Vec<u32>) -> bool {
        out.clear();
        let (a, b) = (
            self.loop_ofs[loop_idx] as usize,
            self.loop_ofs[loop_idx + 1] as usize,
        );
        for c in a..b {
            let e = self.coedge_edge[c];
            if e == u32::MAX {
                return false;
            }
            let (v0, v1) = (self.edge_v[e as usize * 2], self.edge_v[e as usize * 2 + 1]);
            if v0 == u32::MAX || v1 == u32::MAX {
                return false;
            }
            let first = if self.coedge_reversed[c] != 0 { v1 } else { v0 };
            if out.last() != Some(&first) {
                out.push(first);
            }
        }
        // A closing vertex equal to the first is redundant for a polygon.
        if out.len() > 1 && out.first() == out.last() {
            out.pop();
        }
        out.len() >= 3
    }
}

/// Parse one `Component` record payload.
pub fn parse_component(payload: &[u8]) -> Result<DefGeometry> {
    let mut unlimited = u64::MAX;
    parse_component_within(payload, &mut unlimited)
}

/// `n` elements more against `budget`: `Malformed("too many elements")` once it is spent.
pub(crate) fn spend(budget: &mut u64, n: u64) -> Result<()> {
    *budget = budget
        .checked_sub(n)
        .ok_or(Error::Malformed("too many elements"))?;
    Ok(())
}

/// [`parse_component`], counting every vertex, edge, face, loop, edge use, guide and instance
/// against `budget` and failing with `Malformed("too many elements")` once it is spent, so a
/// small deflated record of millions of empty elements cannot take all memory.
pub fn parse_component_within(payload: &[u8], budget: &mut u64) -> Result<DefGeometry> {
    // The geometry's own tables, before any element.
    spend(budget, 1)?;
    let mut g = DefGeometry::default();

    // Vertices first: edges reference them, and within a component the
    // vertices record always precedes the edges record in every file measured.
    // The parse does not rely on that - references resolve in a second pass -
    // but the common case costs nothing.
    for sec in Tlv::new(payload) {
        if sec.tag == t::COMPONENT_VERTICES {
            for v in sec.children_with(t::VERTEX) {
                let c = common_of(&v);
                let mut pos = [0.0f64; 3];
                for p in v.children_with(t::VERTEX_POSITION) {
                    if let Ok(xyz) = p.vec3_at(0) {
                        pos = xyz;
                    }
                }
                spend(budget, 1)?;
                g.vertex_pid.push(c.pid);
                g.vertex_pos.extend_from_slice(&pos);
            }
        }
    }
    // Sort by pid, carrying positions along, so lookups can binary-search.
    if !is_sorted(&g.vertex_pid) {
        let mut order: Vec<u32> = (0..g.vertex_pid.len() as u32).collect();
        order.sort_unstable_by_key(|&i| g.vertex_pid[i as usize]);
        let pid: Vec<u64> = order.iter().map(|&i| g.vertex_pid[i as usize]).collect();
        let mut pos = Vec::with_capacity(g.vertex_pos.len());
        for &i in &order {
            let k = i as usize * 3;
            pos.extend_from_slice(&g.vertex_pos[k..k + 3]);
        }
        g.vertex_pid = pid;
        g.vertex_pos = pos;
    }

    // Edges.
    let mut edge_start: Vec<u64> = Vec::new();
    let mut edge_end: Vec<u64> = Vec::new();
    for sec in Tlv::new(payload) {
        if sec.tag == t::COMPONENT_EDGES {
            for e in sec.children_with(t::EDGE) {
                let c = common_of(&e);
                let mut a = 0u64;
                let mut b = 0u64;
                for ch in e.children() {
                    match ch.tag {
                        t::EDGE_START_VERTEX_PID_REF => a = varint(&ch),
                        t::EDGE_END_VERTEX_PID_REF => b = varint(&ch),
                        _ => {}
                    }
                }
                spend(budget, 1)?;
                g.edge_pid.push(c.pid);
                edge_start.push(a);
                edge_end.push(b);
                g.edge_flags.push(c.flags);
                g.edge_layer.push(layer_ref(&c));
            }
        }
    }
    if !is_sorted(&g.edge_pid) {
        let mut order: Vec<u32> = (0..g.edge_pid.len() as u32).collect();
        order.sort_unstable_by_key(|&i| g.edge_pid[i as usize]);
        let take =
            |src: &Vec<u64>| -> Vec<u64> { order.iter().map(|&i| src[i as usize]).collect() };
        let pid = take(&g.edge_pid);
        let st = take(&edge_start);
        let en = take(&edge_end);
        let fl: Vec<u8> = order.iter().map(|&i| g.edge_flags[i as usize]).collect();
        let ly: Vec<u32> = order.iter().map(|&i| g.edge_layer[i as usize]).collect();
        g.edge_pid = pid;
        edge_start = st;
        edge_end = en;
        g.edge_flags = fl;
        g.edge_layer = ly;
    }
    g.edge_v.reserve(g.edge_pid.len() * 2);
    for i in 0..g.edge_pid.len() {
        g.edge_v.push(g.vertex_index(edge_start[i]));
        g.edge_v.push(g.vertex_index(edge_end[i]));
    }

    // Faces, with their loops in CSR form.
    g.face_loop_ofs.push(0);
    g.loop_ofs.push(0);
    for sec in Tlv::new(payload) {
        if sec.tag != t::COMPONENT_FACES {
            continue;
        }
        for f in sec.children_with(t::FACE) {
            spend(budget, 1)?;
            let c = common_of(&f);
            let mut n = [0.0f64; 3];
            let mut d = 0.0f64;
            let mut back = NO_MATERIAL as u64;
            let mut has_back = false;
            for ch in f.children() {
                match ch.tag {
                    t::FACE_PLANE => {
                        // Four doubles: the unit normal and the plane's
                        // constant term. Readers that take 24 bytes here get
                        // the normal and throw the plane away.
                        if let Ok(xyz) = ch.vec3_at(0) {
                            n = xyz;
                        }
                        if ch.payload.len() >= 32 {
                            d = ch.f64_at(24).unwrap_or(0.0);
                        }
                    }
                    t::FACE_BACK_MATERIAL_PID_REF => {
                        back = varint(&ch);
                        has_back = true;
                    }
                    _ => {}
                }
            }
            g.face_normal.extend_from_slice(&n);
            g.face_plane_d.push(d);
            g.face_material.push(if c.has_material {
                c.material as u32
            } else {
                NO_MATERIAL
            });
            g.face_back_material
                .push(if has_back { back as u32 } else { NO_MATERIAL });
            g.face_layer.push(layer_ref(&c));
            g.face_flags.push(c.flags);
            match front_texture_info(&f) {
                // The identity carries no information: under the recipe in
                // the module comment it *is* the default projection, and
                // storing it would cost 72 bytes a face to say "nothing".
                Some((m, _)) if is_identity3(&m) => g.face_uv.push(NO_UV),
                Some((m, fl)) => {
                    g.face_uv.push((g.uv_xf.len() / 9) as u32);
                    g.uv_xf.extend_from_slice(&m);
                    g.uv_flags.push(fl);
                }
                None => g.face_uv.push(NO_UV),
            }

            let mut loops_here = 0u32;
            for fl in f.children_with(t::FACE_LOOPS) {
                for lp in fl.children_with(t::LOOP) {
                    let mut coedges = 0u32;
                    for lu in lp.children_with(t::LOOP_EDGE_USES) {
                        for eu in lu.children_with(t::EDGE_USE) {
                            let mut eref = 0u64;
                            let mut rev = 0u8;
                            for ch in eu.children() {
                                match ch.tag {
                                    t::EDGE_USE_EDGE_PID_REF => eref = varint(&ch),
                                    t::EDGE_USE_REVERSED => {
                                        rev = ch.payload.first().copied().unwrap_or(0)
                                    }
                                    _ => {}
                                }
                            }
                            spend(budget, 1)?;
                            g.coedge_edge.push(g.edge_index(eref));
                            g.coedge_reversed.push(rev);
                            coedges += 1;
                        }
                    }
                    if coedges > 0 {
                        spend(budget, 1)?;
                        g.loop_ofs.push(g.coedge_edge.len() as u32);
                        loops_here += 1;
                    }
                }
            }
            let _ = loops_here;
            g.face_loop_ofs.push(g.loop_ofs.len() as u32 - 1);
        }
    }

    // Guides. Layout from openskp's VFF reader (see VENDORED.md), with
    // SketchUp's own tag names:
    //
    //   ComponentConstructionLines  -> ConstructionLine  { ConstructionGeometry,
    //       ConstructionLineGeomLine[8 x f64: point, direction, start, end], ... }
    //   ComponentConstructionPoints -> ConstructionPoint { ConstructionGeometry,
    //       ConstructionPointPosition[3 x f64], ... }
    //
    // `ConstructionGeometry` wraps the usual `DrawingElement`, whose flags
    // carry the hidden bit. A line's start and end parameters are dropped: a
    // guide line is infinite. Anything else in either list is skipped and
    // counted, never read as a guide.
    for sec in Tlv::new(payload) {
        let points = match sec.tag {
            t::COMPONENT_CONSTRUCTION_LINES => false,
            t::COMPONENT_CONSTRUCTION_POINTS => true,
            _ => continue,
        };
        for r in sec.children() {
            let (guide, common) = guide_of(&r, points);
            let flags = common.flags;
            let Some(guide) = guide else {
                g.unread_guides = g.unread_guides.saturating_add(1);
                continue;
            };
            if flags & 1 != 0 {
                g.hidden_guides = g.hidden_guides.saturating_add(1);
                spend(budget, 1)?;
                g.hidden_guide_items.push(guide);
                g.hidden_guide_layer.push(layer_ref(&common));
                continue;
            }
            spend(budget, 1)?;
            g.guides.push(guide);
            g.guide_layer.push(layer_ref(&common));
        }
    }

    // Placements. Three sections carry them and two of the three wrap each
    // one in a single-record envelope:
    //
    //   ComponentInstances -> ComponentInstance
    //   ComponentGroups    -> Group  -> ComponentInstance
    //   ComponentImages    -> Image  -> ComponentInstance
    //
    // A Group and an Image are both just a ComponentInstance in a envelope
    // that says what kind of thing it is. Missing the Images section cost one
    // placement and one corner of the bounding box on a fixture, which is
    // exactly the kind of omission that looks like nothing until a model is
    // missing a photograph.
    for sec in Tlv::new(payload) {
        let wrapper = match sec.tag {
            t::COMPONENT_INSTANCES => None,
            t::COMPONENT_GROUPS => Some(t::GROUP),
            t::COMPONENT_IMAGES => Some(t::IMAGE),
            _ => continue,
        };
        let is_group = sec.tag != t::COMPONENT_INSTANCES;
        match wrapper {
            None => {
                for i in sec.children_with(t::COMPONENT_INSTANCE) {
                    spend(budget, 1)?;
                    g.instances.push(parse_instance(&i, false));
                }
            }
            Some(w) => {
                for env in sec.children_with(w) {
                    for i in env.children_with(t::COMPONENT_INSTANCE) {
                        let mut inst = parse_instance(&i, is_group);
                        inst.is_image = sec.tag == t::COMPONENT_IMAGES;
                        spend(budget, 1)?;
                        g.instances.push(inst);
                    }
                }
            }
        }
    }

    Ok(g)
}

fn parse_instance(rec: &Record, is_group: bool) -> RawInstance {
    let c = common_of(rec);
    let mut i = RawInstance {
        pid: c.pid,
        material: c.material,
        has_material: c.has_material,
        layer: c.layer,
        has_layer: c.has_layer,
        flags: c.flags,
        is_group,
        m: IDENTITY13,
        ..Default::default()
    };
    for ch in rec.children() {
        match ch.tag {
            t::COMPONENT_INSTANCE_NAME => {
                // Raw UTF-8, length is the record's own. Not the
                // length-prefixed UTF-16 that names use elsewhere in the
                // format - a detail worth stating because getting it wrong
                // yields plausible-looking mojibake rather than an error.
                i.name = String::from_utf8_lossy(ch.payload).into_owned();
            }
            t::COMPONENT_INSTANCE_TRANSFORM => {
                if ch.payload.len() >= 104 {
                    for q in 0..13 {
                        i.m[q] = ch.f64_at(q * 8).unwrap_or(i.m[q]);
                    }
                }
            }
            t::COMPONENT_INSTANCE_DEF_PID_REF => i.def_pid = varint(&ch),
            t::COMPONENT_INSTANCE_GUID => {
                if ch.payload.len() >= 16 {
                    i.guid.copy_from_slice(&ch.payload[..16]);
                }
            }
            _ => {}
        }
    }
    i
}

/// One record of a guide list read as a guide, with its drawing element (flags and layer).
/// `None` when the record is not the guide its list holds (`points` says which), or lacks
/// its point or line: a point needs three doubles, a line its eight (as openskp requires).
fn guide_of(rec: &Record, points: bool) -> (Option<RawGuide>, Common) {
    let (tag, body, need) = if points {
        (t::CONSTRUCTION_POINT, t::CONSTRUCTION_POINT_POSITION, 24)
    } else {
        (t::CONSTRUCTION_LINE, t::CONSTRUCTION_LINE_GEOM_LINE, 64)
    };
    if rec.tag != tag {
        return (None, Common::default());
    }
    let common = rec
        .child(t::CONSTRUCTION_GEOMETRY)
        .map_or(Common::default(), |cg| common_of(&cg));
    let guide = rec
        .child(body)
        .filter(|b| b.payload.len() >= need)
        .and_then(|b| {
            let first = b.vec3_at(0).ok()?;
            Some(if points {
                RawGuide::Point { position: first }
            } else {
                RawGuide::Line {
                    point: first,
                    direction: b.vec3_at(24).ok()?,
                }
            })
        });
    (guide, common)
}

fn is_sorted(v: &[u64]) -> bool {
    v.windows(2).all(|w| w[0] <= w[1])
}

// ---------------------------------------------------------------------------
// Texture mapping
// ---------------------------------------------------------------------------

/// `FaceSideTextureInfo` for the front side: its transform and its flags.
///
/// Walks the named path rather than searching for a tag at any depth. The
/// search version worked and was 4% of the parse on a commercial pack, because
/// `payload_is_records` has to be run on every candidate payload to avoid
/// descending into a leaf of doubles. The path is fixed and it is spelled out
/// in the module comment.
fn front_texture_info(face: &Record) -> Option<([f64; 9], u32)> {
    // A Face wraps its Entity in a DrawingElement; a bare Entity is accepted
    // because `common_of` accepts both and a reader that insisted would
    // silently drop mappings on whatever writer does it the other way.
    let entity = face
        .child(t::DRAWING_ELEMENT)
        .and_then(|de| de.child(t::ENTITY))
        .or_else(|| face.child(t::ENTITY))?;
    let info = entity
        .child(t::ENTITY_ATTRIBUTES)?
        .child(t::ATTRIBUTE_CONTAINER)?
        .child(t::ATTRIBUTE_CONTAINER_ATTRIBUTES)?
        // The attribute container holds whatever attributes the face has, and
        // a dynamic component puts `AttributeNamed` siblings in here too.
        .children_with(t::FACE_TEXTURE_COORDS)
        .next()?
        .child(t::FACE_TEXTURE_COORDS_FRONT)?
        .child(t::FACE_SIDE_TEXTURE_INFO)?;
    let xf = info.child(t::FACE_SIDE_TEXTURE_INFO_TRANSFORM)?;
    if xf.payload.len() < 72 {
        return None;
    }
    let mut m = [0.0f64; 9];
    for (i, slot) in m.iter_mut().enumerate() {
        *slot = xf.f64_at(i * 8).ok()?;
        if !slot.is_finite() {
            return None;
        }
    }
    let flags = info
        .child(t::FACE_SIDE_TEXTURE_INFO_FLAGS)
        .and_then(|r| r.u32_at(0).ok())
        .unwrap_or(0);
    Some((m, flags))
}

fn is_identity3(m: &[f64; 9]) -> bool {
    *m == [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
}

/// SketchUp's texture axes for a face plane.
///
/// The first axis is horizontal and perpendicular to the normal's horizontal
/// projection, which is what makes a wall texture stand upright, and the
/// second is the normal crossed into it. A horizontal face degenerates to
/// world X and ±Y.
///
/// The basis depends only on the normal, so two coplanar faces agree on it and
/// an unpositioned texture runs continuously across them. Note that openskp's
/// *writer* uses a different basis for the same matrix - the face's first edge
/// direction - which cannot be right for a reader: it would give every face
/// its own axes and no two faces would line up.
pub fn uv_axes(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let (nx, ny, nz) = (n[0], n[1], n[2]);
    let (cx, cy) = (-ny, nx);
    let clen = (cx * cx + cy * cy).sqrt();
    if clen < 1e-9 {
        // Horizontal. The Y flip keeps a floor and a ceiling from mirroring
        // each other's texture.
        (
            [1.0, 0.0, 0.0],
            [0.0, if nz >= 0.0 { 1.0 } else { -1.0 }, 0.0],
        )
    } else {
        let ex = [cx / clen, cy / clen, 0.0];
        let ey = [
            ny * ex[2] - nz * ex[1],
            nz * ex[0] - nx * ex[2],
            nx * ex[1] - ny * ex[0],
        ];
        (ex, ey)
    }
}

/// One face's texture mapping, prepared once and evaluated per vertex.
///
/// # Hand it the point in the file's own unit - inches - and scale after
///
/// `at` is **not** linear in the point. The stored matrix's translation column
/// and its projective row are in the unit the file was written in, and they
/// multiply the `1` of the homogeneous `(px, py, 1)`, which no unit conversion
/// touches. So converting the point to millimetres first leaves the offset
/// un-scaled and slides the texture across the face. Measured on
/// `pair_feature-pack` that is 1,070 mm of texture space; on the legacy bear
/// home, 1,729 mm.
///
/// The one correct order is: inches in, inches out, multiply the result by
/// [`crate::model::INCH_TO_MM`]. `MeshBatch::uvs` in `model.rs` is the
/// contract that says the result is millimetres.
#[derive(Debug, Clone, Copy)]
pub struct UvMap {
    ex: [f64; 3],
    ey: [f64; 3],
    /// The inverse of the stored matrix, or `None` for the default projection.
    inv: Option<[f64; 9]>,
}

impl UvMap {
    pub fn new(normal: [f64; 3], xf: Option<&[f64; 9]>) -> Self {
        let (ex, ey) = uv_axes(normal);
        let inv = match xf {
            Some(m) if !is_identity3(m) => invert3(m),
            _ => None,
        };
        Self { ex, ey, inv }
    }

    /// Whether this mapping is the plain face-plane projection.
    pub fn is_default(&self) -> bool {
        self.inv.is_none()
    }

    pub fn at(&self, p: [f64; 3]) -> [f64; 2] {
        let px = p[0] * self.ex[0] + p[1] * self.ex[1] + p[2] * self.ex[2];
        let py = p[0] * self.ey[0] + p[1] * self.ey[1] + p[2] * self.ey[2];
        let Some(m) = &self.inv else { return [px, py] };
        let u = px * m[0] + py * m[3] + m[6];
        let v = px * m[1] + py * m[4] + m[7];
        let mut w = px * m[2] + py * m[5] + m[8];
        // A face whose projection plane passes through the eye point has a
        // vanishing w somewhere on it. That is a real configuration in a
        // photo-matched texture, and dividing by it would hand the GPU an
        // infinity that poisons the whole triangle; the nearest finite answer
        // is the unprojected one.
        if w.abs() < 1e-12 {
            w = 1.0;
        }
        [u / w, v / w]
    }
}

/// Inverse of a row-major 3x3, or `None` when it is singular.
fn invert3(m: &[f64; 9]) -> Option<[f64; 9]> {
    let (a, b, c, d, e, f, g, h, i) = (m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]);
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    // Scale-relative, not absolute: these matrices legitimately carry terms in
    // the thousands and in the 1e-13, so a fixed epsilon rejects good matrices
    // and accepts bad ones.
    let mag = m.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
    if !det.is_finite() || det.abs() <= 1e-18 * mag * mag * mag {
        return None;
    }
    let k = 1.0 / det;
    Some([
        (e * i - f * h) * k,
        (c * h - b * i) * k,
        (b * f - c * e) * k,
        (f * g - d * i) * k,
        (a * i - c * g) * k,
        (c * d - a * f) * k,
        (d * h - e * g) * k,
        (b * g - a * h) * k,
        (a * e - b * d) * k,
    ])
}

// ---------------------------------------------------------------------------
// Triangulation
// ---------------------------------------------------------------------------

/// Ear clipping with hole elimination and a z-order curve, after Mapbox's
/// earcut.
///
/// A simpler scheme - bridge each hole into the outer ring, then clip ears by
/// rescanning - is enough for a wall with a window, and it is what this file
/// had first. Then a real file produced a single face with a 508-vertex outer
/// ring and **489 loops**: a perforated panel in a definition named "Sumele".
/// The naive version spent over a second on that one face. Hole elimination in
/// sorted order plus a hashed containment test handles it in milliseconds,
/// which is why the full algorithm is here rather than a sketch of it.
///
/// Triangulation is view-independent in SketchUp - `CFaceTesselator` works in
/// the face's plane from its loops - so this runs once per definition and the
/// result serves every placement.
const NIL: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Node {
    /// Index of the source vertex this node stands for.
    i: u32,
    x: f64,
    y: f64,
    prev: u32,
    next: u32,
    z: i64,
    prev_z: u32,
    next_z: u32,
    steiner: bool,
}

struct Ring {
    n: Vec<Node>,
}

impl Ring {
    fn new(cap: usize) -> Self {
        Self {
            n: Vec::with_capacity(cap),
        }
    }

    fn insert(&mut self, i: u32, x: f64, y: f64, last: u32) -> u32 {
        let id = self.n.len() as u32;
        self.n.push(Node {
            i,
            x,
            y,
            prev: NIL,
            next: NIL,
            z: 0,
            prev_z: NIL,
            next_z: NIL,
            steiner: false,
        });
        if last == NIL {
            self.n[id as usize].prev = id;
            self.n[id as usize].next = id;
        } else {
            let next = self.n[last as usize].next;
            self.n[id as usize].next = next;
            self.n[id as usize].prev = last;
            self.n[next as usize].prev = id;
            self.n[last as usize].next = id;
        }
        id
    }

    fn remove(&mut self, p: u32) {
        let (prev, next) = (self.n[p as usize].prev, self.n[p as usize].next);
        self.n[next as usize].prev = prev;
        self.n[prev as usize].next = next;
        let pz = self.n[p as usize].prev_z;
        let nz = self.n[p as usize].next_z;
        if pz != NIL {
            self.n[pz as usize].next_z = nz;
        }
        if nz != NIL {
            self.n[nz as usize].prev_z = pz;
        }
    }

    #[inline]
    fn next(&self, p: u32) -> u32 {
        self.n[p as usize].next
    }
    #[inline]
    fn prev(&self, p: u32) -> u32 {
        self.n[p as usize].prev
    }
    #[inline]
    fn x(&self, p: u32) -> f64 {
        self.n[p as usize].x
    }
    #[inline]
    fn y(&self, p: u32) -> f64 {
        self.n[p as usize].y
    }

    fn build_ring(&mut self, ring: &[u32], pts: &dyn Fn(u32) -> [f64; 2], ccw: bool) -> u32 {
        let mut last = NIL;
        if ccw == (signed_area_ring(ring, pts) > 0.0) {
            for &v in ring {
                let p = pts(v);
                last = self.insert(v, p[0], p[1], last);
            }
        } else {
            for &v in ring.iter().rev() {
                let p = pts(v);
                last = self.insert(v, p[0], p[1], last);
            }
        }
        if last != NIL {
            let nx = self.next(last);
            if self.equals(last, nx) {
                self.remove(last);
                last = nx;
            }
        }
        last
    }

    fn equals(&self, a: u32, b: u32) -> bool {
        self.x(a) == self.x(b) && self.y(a) == self.y(b)
    }

    /// Drop collinear and duplicate points.
    ///
    /// `end` moves with `p` when a node is removed. Leaving it fixed - which
    /// is the obvious reading - makes the loop's termination condition
    /// unreachable on some rings and it spins forever. That cost a hang on a
    /// real file, so it is worth stating.
    fn filter(&mut self, start: u32, end: u32) -> u32 {
        if start == NIL {
            return start;
        }
        let mut end = if end == NIL { start } else { end };
        let mut p = start;
        loop {
            let mut again = false;
            let nx = self.next(p);
            let pr = self.prev(p);
            if !self.n[p as usize].steiner && (self.equals(p, nx) || area(self, pr, p, nx) == 0.0) {
                self.remove(p);
                p = pr;
                end = pr;
                if p == self.next(p) {
                    return NIL;
                }
                again = true;
            } else {
                p = nx;
            }
            if !again && p == end {
                break;
            }
        }
        end
    }
}

fn signed_area_ring(ring: &[u32], pts: &dyn Fn(u32) -> [f64; 2]) -> f64 {
    let mut s = 0.0;
    let n = ring.len();
    for i in 0..n {
        let a = pts(ring[i]);
        let b = pts(ring[(i + 1) % n]);
        s += (a[0] - b[0]) * (a[1] + b[1]);
    }
    s
}

#[inline]
fn area(r: &Ring, p: u32, q: u32, s: u32) -> f64 {
    (r.y(q) - r.y(p)) * (r.x(s) - r.x(q)) - (r.x(q) - r.x(p)) * (r.y(s) - r.y(q))
}

#[inline]
fn point_in_triangle(
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
    px: f64,
    py: f64,
) -> bool {
    (cx - px) * (ay - py) >= (ax - px) * (cy - py)
        && (ax - px) * (by - py) >= (bx - px) * (ay - py)
        && (bx - px) * (cy - py) >= (cx - px) * (by - py)
}

fn locally_inside(r: &Ring, a: u32, b: u32) -> bool {
    let (pa, na) = (r.prev(a), r.next(a));
    if area(r, pa, a, na) < 0.0 {
        area(r, a, b, na) >= 0.0 && area(r, a, pa, b) >= 0.0
    } else {
        area(r, a, b, pa) < 0.0 || area(r, a, na, b) < 0.0
    }
}

fn middle_inside(r: &Ring, a: u32, b: u32) -> bool {
    let mut p = a;
    let mut inside = false;
    let (px, py) = ((r.x(a) + r.x(b)) / 2.0, (r.y(a) + r.y(b)) / 2.0);
    loop {
        let nx = r.next(p);
        if (r.y(p) > py) != (r.y(nx) > py)
            && r.y(nx) != r.y(p)
            && px < (r.x(nx) - r.x(p)) * (py - r.y(p)) / (r.y(nx) - r.y(p)) + r.x(p)
        {
            inside = !inside;
        }
        p = nx;
        if p == a {
            break;
        }
    }
    inside
}

fn intersects(r: &Ring, p1: u32, q1: u32, p2: u32, q2: u32) -> bool {
    let o1 = sign(area(r, p1, q1, p2));
    let o2 = sign(area(r, p1, q1, q2));
    let o3 = sign(area(r, p2, q2, p1));
    let o4 = sign(area(r, p2, q2, q1));
    if o1 != o2 && o3 != o4 {
        return true;
    }
    if o1 == 0 && on_segment(r, p1, p2, q1) {
        return true;
    }
    if o2 == 0 && on_segment(r, p1, q2, q1) {
        return true;
    }
    if o3 == 0 && on_segment(r, p2, p1, q2) {
        return true;
    }
    if o4 == 0 && on_segment(r, p2, q1, q2) {
        return true;
    }
    false
}

fn on_segment(r: &Ring, p: u32, q: u32, s: u32) -> bool {
    r.x(q) <= r.x(p).max(r.x(s))
        && r.x(q) >= r.x(p).min(r.x(s))
        && r.y(q) <= r.y(p).max(r.y(s))
        && r.y(q) >= r.y(p).min(r.y(s))
}

fn sign(v: f64) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

fn intersects_polygon(r: &Ring, a: u32, b: u32) -> bool {
    let mut p = a;
    loop {
        let nx = r.next(p);
        if r.n[p as usize].i != r.n[a as usize].i
            && r.n[nx as usize].i != r.n[a as usize].i
            && r.n[p as usize].i != r.n[b as usize].i
            && r.n[nx as usize].i != r.n[b as usize].i
            && intersects(r, p, nx, a, b)
        {
            return true;
        }
        p = nx;
        if p == a {
            break;
        }
    }
    false
}

fn is_valid_diagonal(r: &Ring, a: u32, b: u32) -> bool {
    r.n[r.next(a) as usize].i != r.n[b as usize].i
        && r.n[r.prev(a) as usize].i != r.n[b as usize].i
        && !intersects_polygon(r, a, b)
        && ((locally_inside(r, a, b)
            && locally_inside(r, b, a)
            && middle_inside(r, a, b)
            && (area(r, r.prev(a), a, r.prev(b)) != 0.0 || area(r, a, r.prev(b), b) != 0.0))
            || (r.equals(a, b)
                && area(r, r.prev(a), a, r.next(a)) > 0.0
                && area(r, r.prev(b), b, r.next(b)) > 0.0))
}

fn z_order(x: f64, y: f64, min_x: f64, min_y: f64, inv_size: f64) -> i64 {
    let mut x = (((x - min_x) * inv_size) as i64).clamp(0, 32767);
    let mut y = (((y - min_y) * inv_size) as i64).clamp(0, 32767);
    x = (x | (x << 8)) & 0x00FF00FF;
    x = (x | (x << 4)) & 0x0F0F0F0F;
    x = (x | (x << 2)) & 0x33333333;
    x = (x | (x << 1)) & 0x55555555;
    y = (y | (y << 8)) & 0x00FF00FF;
    y = (y | (y << 4)) & 0x0F0F0F0F;
    y = (y | (y << 2)) & 0x33333333;
    y = (y | (y << 1)) & 0x55555555;
    x | (y << 1)
}

fn index_curve(r: &mut Ring, start: u32, min_x: f64, min_y: f64, inv_size: f64) {
    let mut p = start;
    loop {
        if r.n[p as usize].z == 0 {
            r.n[p as usize].z = z_order(r.x(p), r.y(p), min_x, min_y, inv_size);
        }
        r.n[p as usize].prev_z = r.prev(p);
        r.n[p as usize].next_z = r.next(p);
        p = r.next(p);
        if p == start {
            break;
        }
    }
    let pz = r.n[p as usize].prev_z;
    if pz == NIL || pz == p {
        return;
    }
    r.n[pz as usize].next_z = NIL;
    r.n[p as usize].prev_z = NIL;
    sort_linked(r, p);
}

/// Merge sort over the z-order list. Iterative, because the lists get long.
fn sort_linked(r: &mut Ring, mut list: u32) {
    let mut in_size = 1usize;
    loop {
        let mut p = list;
        list = NIL;
        let mut tail = NIL;
        let mut num_merges = 0;
        while p != NIL {
            num_merges += 1;
            let mut q = p;
            let mut p_size = 0usize;
            for _ in 0..in_size {
                p_size += 1;
                q = r.n[q as usize].next_z;
                if q == NIL {
                    break;
                }
            }
            let mut q_size = in_size;
            while p_size > 0 || (q_size > 0 && q != NIL) {
                let e;
                if p_size == 0 {
                    e = q;
                    q = r.n[q as usize].next_z;
                    q_size -= 1;
                } else if q_size == 0 || q == NIL {
                    e = p;
                    p = r.n[p as usize].next_z;
                    p_size -= 1;
                } else if r.n[p as usize].z <= r.n[q as usize].z {
                    e = p;
                    p = r.n[p as usize].next_z;
                    p_size -= 1;
                } else {
                    e = q;
                    q = r.n[q as usize].next_z;
                    q_size -= 1;
                }
                if tail != NIL {
                    r.n[tail as usize].next_z = e;
                } else {
                    list = e;
                }
                r.n[e as usize].prev_z = tail;
                tail = e;
            }
            p = q;
        }
        if tail == NIL {
            return;
        }
        r.n[tail as usize].next_z = NIL;
        if num_merges <= 1 {
            return;
        }
        in_size *= 2;
    }
}

fn is_ear(r: &Ring, ear: u32) -> bool {
    let (a, b, c) = (r.prev(ear), ear, r.next(ear));
    if area(r, a, b, c) >= 0.0 {
        return false;
    }
    let (ax, ay) = (r.x(a), r.y(a));
    let (bx, by) = (r.x(b), r.y(b));
    let (cx, cy) = (r.x(c), r.y(c));
    let x0 = ax.min(bx).min(cx);
    let y0 = ay.min(by).min(cy);
    let x1 = ax.max(bx).max(cx);
    let y1 = ay.max(by).max(cy);
    let mut p = r.next(c);
    while p != a {
        if r.x(p) >= x0
            && r.x(p) <= x1
            && r.y(p) >= y0
            && r.y(p) <= y1
            && point_in_triangle(ax, ay, bx, by, cx, cy, r.x(p), r.y(p))
            && area(r, r.prev(p), p, r.next(p)) >= 0.0
        {
            return false;
        }
        p = r.next(p);
    }
    true
}

fn is_ear_hashed(r: &Ring, ear: u32, min_x: f64, min_y: f64, inv_size: f64) -> bool {
    let (a, b, c) = (r.prev(ear), ear, r.next(ear));
    if area(r, a, b, c) >= 0.0 {
        return false;
    }
    let (ax, ay) = (r.x(a), r.y(a));
    let (bx, by) = (r.x(b), r.y(b));
    let (cx, cy) = (r.x(c), r.y(c));
    let x0 = ax.min(bx).min(cx);
    let y0 = ay.min(by).min(cy);
    let x1 = ax.max(bx).max(cx);
    let y1 = ay.max(by).max(cy);
    let min_z = z_order(x0, y0, min_x, min_y, inv_size);
    let max_z = z_order(x1, y1, min_x, min_y, inv_size);

    let mut p = r.n[ear as usize].prev_z;
    let mut n = r.n[ear as usize].next_z;
    while p != NIL && r.n[p as usize].z >= min_z && n != NIL && r.n[n as usize].z <= max_z {
        if r.x(p) >= x0
            && r.x(p) <= x1
            && r.y(p) >= y0
            && r.y(p) <= y1
            && p != a
            && p != c
            && point_in_triangle(ax, ay, bx, by, cx, cy, r.x(p), r.y(p))
            && area(r, r.prev(p), p, r.next(p)) >= 0.0
        {
            return false;
        }
        p = r.n[p as usize].prev_z;
        if r.x(n) >= x0
            && r.x(n) <= x1
            && r.y(n) >= y0
            && r.y(n) <= y1
            && n != a
            && n != c
            && point_in_triangle(ax, ay, bx, by, cx, cy, r.x(n), r.y(n))
            && area(r, r.prev(n), n, r.next(n)) >= 0.0
        {
            return false;
        }
        n = r.n[n as usize].next_z;
    }
    while p != NIL && r.n[p as usize].z >= min_z {
        if r.x(p) >= x0
            && r.x(p) <= x1
            && r.y(p) >= y0
            && r.y(p) <= y1
            && p != a
            && p != c
            && point_in_triangle(ax, ay, bx, by, cx, cy, r.x(p), r.y(p))
            && area(r, r.prev(p), p, r.next(p)) >= 0.0
        {
            return false;
        }
        p = r.n[p as usize].prev_z;
    }
    while n != NIL && r.n[n as usize].z <= max_z {
        if r.x(n) >= x0
            && r.x(n) <= x1
            && r.y(n) >= y0
            && r.y(n) <= y1
            && n != a
            && n != c
            && point_in_triangle(ax, ay, bx, by, cx, cy, r.x(n), r.y(n))
            && area(r, r.prev(n), n, r.next(n)) >= 0.0
        {
            return false;
        }
        n = r.n[n as usize].next_z;
    }
    true
}

fn split_polygon(r: &mut Ring, a: u32, b: u32) -> u32 {
    let (ai, ax, ay) = (r.n[a as usize].i, r.x(a), r.y(a));
    let (bi, bx, by) = (r.n[b as usize].i, r.x(b), r.y(b));
    let a2 = r.n.len() as u32;
    r.n.push(Node {
        i: ai,
        x: ax,
        y: ay,
        prev: NIL,
        next: NIL,
        z: 0,
        prev_z: NIL,
        next_z: NIL,
        steiner: false,
    });
    let b2 = r.n.len() as u32;
    r.n.push(Node {
        i: bi,
        x: bx,
        y: by,
        prev: NIL,
        next: NIL,
        z: 0,
        prev_z: NIL,
        next_z: NIL,
        steiner: false,
    });

    let an = r.next(a);
    let bp = r.prev(b);
    r.n[a as usize].next = b;
    r.n[b as usize].prev = a;
    r.n[a2 as usize].next = an;
    r.n[an as usize].prev = a2;
    r.n[b2 as usize].next = a2;
    r.n[a2 as usize].prev = b2;
    r.n[bp as usize].next = b2;
    r.n[b2 as usize].prev = bp;
    b2
}

fn cure_local_intersections(r: &mut Ring, mut start: u32, out: &mut Vec<[u32; 3]>) -> u32 {
    let mut p = start;
    loop {
        let a = r.prev(p);
        let b = r.next(r.next(p));
        if !r.equals(a, b)
            && intersects(r, a, p, r.next(p), b)
            && locally_inside(r, a, b)
            && locally_inside(r, b, a)
        {
            out.push([r.n[a as usize].i, r.n[p as usize].i, r.n[b as usize].i]);
            r.remove(p);
            let np = r.next(p);
            r.remove(np);
            p = b;
            start = b;
        }
        p = r.next(p);
        if p == start {
            break;
        }
    }
    r.filter(p, NIL)
}

fn earcut_linked(
    r: &mut Ring,
    mut ear: u32,
    out: &mut Vec<[u32; 3]>,
    min_x: f64,
    min_y: f64,
    inv_size: f64,
    pass: u8,
) {
    if ear == NIL {
        return;
    }
    if pass == 0 && inv_size > 0.0 {
        index_curve(r, ear, min_x, min_y, inv_size);
    }
    let mut stop = ear;
    while r.prev(ear) != r.next(ear) {
        let prev = r.prev(ear);
        let next = r.next(ear);
        let hit = if inv_size > 0.0 {
            is_ear_hashed(r, ear, min_x, min_y, inv_size)
        } else {
            is_ear(r, ear)
        };
        if hit {
            out.push([
                r.n[prev as usize].i,
                r.n[ear as usize].i,
                r.n[next as usize].i,
            ]);
            r.remove(ear);
            ear = r.next(next);
            stop = ear;
            continue;
        }
        ear = next;
        if ear == stop {
            // No ear found on a full lap: repair, then retry.
            if pass == 0 {
                let e = r.filter(ear, NIL);
                earcut_linked(r, e, out, min_x, min_y, inv_size, 1);
            } else if pass == 1 {
                let e0 = r.filter(ear, NIL);
                let e = cure_local_intersections(r, e0, out);
                earcut_linked(r, e, out, min_x, min_y, inv_size, 2);
            } else if pass == 2 {
                split_earcut(r, ear, out, min_x, min_y, inv_size);
            }
            return;
        }
    }
}

fn split_earcut(
    r: &mut Ring,
    start: u32,
    out: &mut Vec<[u32; 3]>,
    min_x: f64,
    min_y: f64,
    inv_size: f64,
) {
    let mut a = start;
    loop {
        let mut b = r.next(r.next(a));
        while b != r.prev(a) {
            if r.n[a as usize].i != r.n[b as usize].i && is_valid_diagonal(r, a, b) {
                let mut c = split_polygon(r, a, b);
                let a2 = r.filter(a, r.next(a));
                c = r.filter(c, r.next(c));
                earcut_linked(r, a2, out, min_x, min_y, inv_size, 0);
                earcut_linked(r, c, out, min_x, min_y, inv_size, 0);
                return;
            }
            b = r.next(b);
        }
        a = r.next(a);
        if a == start {
            break;
        }
    }
}

fn get_leftmost(r: &Ring, start: u32) -> u32 {
    let mut p = start;
    let mut leftmost = start;
    loop {
        if r.x(p) < r.x(leftmost) || (r.x(p) == r.x(leftmost) && r.y(p) < r.y(leftmost)) {
            leftmost = p;
        }
        p = r.next(p);
        if p == start {
            break;
        }
    }
    leftmost
}

fn sector_contains_sector(r: &Ring, m: u32, p: u32) -> bool {
    area(r, r.prev(m), m, r.prev(p)) < 0.0 && area(r, r.next(p), m, r.next(m)) < 0.0
}

fn find_hole_bridge(r: &Ring, hole: u32, outer: u32) -> u32 {
    let mut p = outer;
    let (hx, hy) = (r.x(hole), r.y(hole));
    let mut qx = f64::NEG_INFINITY;
    let mut m = NIL;
    loop {
        let nx = r.next(p);
        if hy <= r.y(p) && hy >= r.y(nx) && r.y(nx) != r.y(p) {
            let x = r.x(p) + (hy - r.y(p)) * (r.x(nx) - r.x(p)) / (r.y(nx) - r.y(p));
            if x <= hx && x > qx {
                qx = x;
                m = if r.x(p) < r.x(nx) { p } else { nx };
                if x == hx {
                    return m;
                }
            }
        }
        p = nx;
        if p == outer {
            break;
        }
    }
    if m == NIL {
        return NIL;
    }
    let stop = m;
    let (mx, my) = (r.x(m), r.y(m));
    let mut tan_min = f64::INFINITY;
    let mut m = m;
    p = m;
    loop {
        if hx >= r.x(p)
            && r.x(p) >= mx
            && hx != r.x(p)
            && point_in_triangle(
                if hy < my { hx } else { qx },
                hy,
                mx,
                my,
                if hy < my { qx } else { hx },
                hy,
                r.x(p),
                r.y(p),
            )
        {
            let tan = (hy - r.y(p)).abs() / (hx - r.x(p));
            if locally_inside(r, p, hole)
                && (tan < tan_min
                    || (tan == tan_min
                        && (r.x(p) > r.x(m)
                            || (r.x(p) == r.x(m) && sector_contains_sector(r, m, p)))))
            {
                m = p;
                tan_min = tan;
            }
        }
        p = r.next(p);
        if p == stop {
            break;
        }
    }
    m
}

fn eliminate_hole(r: &mut Ring, hole: u32, outer: u32) -> u32 {
    let bridge = find_hole_bridge(r, hole, outer);
    if bridge == NIL {
        return outer;
    }
    let bridge_reverse = split_polygon(r, bridge, hole);
    let b2 = r.filter(bridge_reverse, r.next(bridge_reverse));
    let _ = b2;
    r.filter(bridge, r.next(bridge))
}

/// Triangulate a face from its loops. Loop 0 is the outer ring; the rest are
/// holes, which is what a window opening in a wall is.
pub fn triangulate(points: &[[f64; 3]], rings: &[&[u32]], normal: [f64; 3]) -> Vec<[u32; 3]> {
    if rings.is_empty() || rings[0].len() < 3 {
        return Vec::new();
    }
    // A triangle is already one, and a quad with no holes splits along
    // whichever diagonal stays inside it. Between them these cover most faces
    // in an architectural model, so the general path rarely runs.
    if rings.len() == 1 {
        let outer = rings[0];
        if outer.len() == 3 {
            return alloc::vec![[outer[0], outer[1], outer[2]]];
        }
        if outer.len() == 4 {
            let p = |i: usize| points[outer[i] as usize];
            let d02 = dist2(p(0), p(2));
            let d13 = dist2(p(1), p(3));
            return if d02 <= d13 {
                alloc::vec![
                    [outer[0], outer[1], outer[2]],
                    [outer[0], outer[2], outer[3]]
                ]
            } else {
                alloc::vec![
                    [outer[0], outer[1], outer[3]],
                    [outer[1], outer[2], outer[3]]
                ]
            };
        }
    }

    let (ex, ey) = plane_basis(normal);
    let to2 = move |v: u32| -> [f64; 2] {
        let p = points[v as usize];
        [dot(p, ex), dot(p, ey)]
    };

    let total: usize = rings.iter().map(|r| r.len()).sum();
    let mut r = Ring::new(total + rings.len() * 2 + 8);
    let mut outer = r.build_ring(rings[0], &to2, true);
    if outer == NIL || r.next(outer) == r.prev(outer) {
        return Vec::new();
    }

    // Holes, eliminated in order of their leftmost point, as earcut requires.
    let mut hole_nodes: Vec<u32> = Vec::with_capacity(rings.len());
    for hole in &rings[1..] {
        if hole.len() < 3 {
            continue;
        }
        let last = r.build_ring(hole, &to2, false);
        if last == NIL {
            continue;
        }
        if last == r.next(last) {
            r.n[last as usize].steiner = true;
        }
        hole_nodes.push(get_leftmost(&r, last));
    }
    hole_nodes.sort_by(|&a, &b| {
        r.x(a)
            .partial_cmp(&r.x(b))
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    for h in hole_nodes {
        outer = eliminate_hole(&mut r, h, outer);
    }

    // A z-order index pays for itself once the ring is big; below that the
    // plain scan is cheaper than building the index.
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut inv_size = 0.0;
    if total > 80 {
        for ring in rings {
            for &v in *ring {
                let p = to2(v);
                min_x = min_x.min(p[0]);
                min_y = min_y.min(p[1]);
                max_x = max_x.max(p[0]);
                max_y = max_y.max(p[1]);
            }
        }
        inv_size = (max_x - min_x).max(max_y - min_y);
        inv_size = if inv_size != 0.0 {
            32767.0 / inv_size
        } else {
            0.0
        };
    }

    let mut out: Vec<[u32; 3]> = Vec::with_capacity(total);
    earcut_linked(&mut r, outer, &mut out, min_x, min_y, inv_size, 0);
    out
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Two in-plane axes for a normal. Seeds from the axis least aligned with it,
/// so the basis never degenerates.
pub fn plane_basis(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let a = if n[0].abs() <= n[1].abs() && n[0].abs() <= n[2].abs() {
        [1.0, 0.0, 0.0]
    } else if n[1].abs() <= n[2].abs() {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let mut ex = cross(a, n);
    normalize(&mut ex);
    let mut ey = cross(n, ex);
    normalize(&mut ey);
    (ex, ey)
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: &mut [f64; 3]) {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 {
        v[0] /= l;
        v[1] /= l;
        v[2] /= l;
    }
}
