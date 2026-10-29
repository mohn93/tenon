//! The pre-2021 container: one uncompressed MFC `CArchive` object stream.
//!
//! A legacy `.skp` is the same UTF-16 header as a modern one followed by a raw
//! MFC archive - no zip, no deflate, no record tags. `container.rs` hands us
//! its byte range and from there the file is a sequence of *objects*, each
//! introduced by a 16-bit tag that is either a class declaration, a reference
//! to an already-declared class (meaning "a new object of that class follows"),
//! or a back-reference to an object written earlier. There is no length field
//! anywhere: every record's size is implied by its class and schema, so a
//! single wrong byte count does not fail - it silently reinterprets the rest of
//! the file. That is the whole difficulty of this format.
//!
//! Three things make it worth the trouble:
//!
//! 1. **It is the main case, not the legacy case.** Two of the three
//!    commercial packs this project measures are pre-2021, and the project's
//!    own generator writes 2017-format files.
//! 2. **It is byte-addressable.** Nothing is compressed, so a 2 MB inline
//!    JPEG costs a `pos += 2 MB`, not an inflate. On the 517 MB restaurant
//!    pack the first 383 MB is inline texture and the reader never touches a
//!    byte of it: measured, the class declaration for `CLayer` - the first
//!    record *after* the material list - sits at offset 0x16d44991.
//! 3. **Nobody reads its saved scenes.** Every other reader in this project
//!    returns zero pages for every legacy file in the corpus - the Python
//!    oracle on all 15, the C++ one too - including the ones whose filenames
//!    say 26, 15 and 5. Camera tabs are the product, so that gap is the
//!    product missing.
//!
//! # Measured
//!
//! On the 517 MB `{19.1.174}` restaurant pack (542,860,670 bytes): the six
//! scene tabs, their six different styles and the six-entry style list come
//! back in 18 ms from the tail of the file alone; the whole model - 2,865
//! definitions, 6,351 placements, 1,027,124 triangles, 1,022,438 edges,
//! 2,540 hole loops, 56,290 positioned texture mappings and 1,819,351
//! texture coordinates - in 1.7 s cold and 0.85 s warm, having read
//! 174,167,857 bytes, 32.1% of the file, and located and skipped 424,851,682
//! bytes (78.3%) of inline JPEG and PNG. Peak resident 407 MB, of which the UV
//! arrays are 14.6 MB and most of the rest is file-backed mapping.
//!
//! Against SketchUp's own hidden-line render of the same six cameras, by
//! `viewer/verify/answerkey-check.sh`: 51 of 51 facts agree and the median ink
//! ratio is 1.08x - the same band as the one VFF file in the answer key, which
//! is 1.08x. The school cafeteria is 1.09x and Food 1.06x.
//!
//! Three files in the corpus read here that no other reader in this project
//! can open at all - both `{19.1.174}` commercial packs and
//! `legacy_v15_woodworking_clamprack`. Each has exactly one cause and each is
//! written down where the fix lives: `CLayer` schema 3 below, and
//! `skip_layer_parent_ref`.
//!
//! # What is decoded, and how each thing was established
//!
//! The record layouts are ported from `openskp-fork`'s Python reader, which
//! was calibrated against real SketchUp saves, cross-checked against the same
//! models re-saved as VFF, and inverted from `openskp.create`, the writer. The
//! comments below say where a *departure* from that reader comes from. The
//! three substantive ones:
//!
//! * **Schema-driven version gates.** Every legacy file opens with a
//!   `CVersionMap` object: 59 `(class name, schema)` pairs, in the clear, at a
//!   fixed position. The Python reader keys its version-dependent byte counts
//!   off the writer version in the header (`ver >= 17`); this reader reads
//!   them out of the version map instead, because that is the number the
//!   writer actually branched on. `CEntity` schema 3 vs 5 is the
//!   persistent-id gate; `CLayer` schema 2 vs 3 is a record length.
//! * **`CLayer` schema 3.** Measured here: a schema-3 `CLayer` has a 23-byte
//!   trailing block where schema 2 has 21. This is why the Python reader
//!   cannot open either `{19.1.174}` commercial pack - it reads the
//!   definition-list anchor two bytes early, and `openskp`'s own ROADMAP
//!   records the resulting "2019" failure as an open bug. Established by
//!   aligning the definition-list anchor: with a 23-byte tail the bytes read
//!   `<u16 active-layer back-ref> <u32 0> <u32 def count> <0xFFFF class decl>`
//!   exactly as they do on every schema-2 file, and with 21 they do not.
//! * **Saved scenes.** See `scan_pages`.
//! * **Face UVs.** `CFaceTextureCoords` is read and its front matrix reaches
//!   the face that owns it, which no other reader in this project manages for
//!   a legacy file - openskp's C++ port walked the record correctly and then
//!   discarded it, so every positioned texture in a 2013-2020 file fell back
//!   to the default projection. The mapping is evaluated through
//!   `geom::UvMap`, the same recipe the modern path uses, and the face-plane
//!   convention it assumes is *verified against these files* rather than taken
//!   from the writer - see `emit_meshes` and the note on `uv_axes`.
//! * **The style, and the styles.** The line-art settings a legacy file opens
//!   with are a flat byte block in the prefix, not the `CSkpStyle` records
//!   that look like the obvious place - see the section comment above
//!   `read_rendering_options`. The *names*, the style count and which style
//!   each tab selects do come from those records; see `StyleRec`. All of it is
//!   checked against SketchUp's own report on five of these files in
//!   `viewer/oracle/su-ref/*/facts.json`.
//!
//! Two things this module shares with the modern path rather than keeping its
//! own version of, because having two was how both went wrong:
//!
//! * **Triangulation.** `geom::triangulate`, holes included. A local bridge
//!   and ear clipper used to live here and lost every hole in the corpus; see
//!   `emit_meshes`.
//! * **The edge sign convention.** `model::EdgeBatch` states it and
//!   `edges::build_adjacency` implements it for VFF; `emit_edges` had only
//!   half of it, which made a curved surface draw as hatching.
//!
//! # What is not decoded
//!
//! * A scene's *complete* captured rendering options. A tab restores a whole
//!   option set, not just a style reference, and the two can differ: the
//!   school cafeteria's style has silhouettes off at width 5 while its single
//!   tab turns them on. The values are in the page's own `CSkpStyle` record
//!   and parse here; `SceneDef` has only `style_id` to hold them, as does
//!   `pages.rs` on the modern side, which reads `ViewPageStylePIDRef` and
//!   drops `ViewPageRenderingOptions`. Closing it needs a field in `model.rs`.
//! * The rest of the pre-model prefix: watermarks, shadow info, font tables.
//!   Never visited, which is exactly why the slot base has to be recovered
//!   rather than counted. Two things are read out of it by locating a landmark
//!   - the unit, by the option's name (`read_length_unit`), and the rendering
//!   options, by the model camera (`read_rendering_options`).
//! * Thirty-five of the fifty settings a `CSkpStyle` record states. Fifteen
//!   ids are pinned uniquely against the options block over this corpus; the
//!   rest hold the same value in every file here and so cannot be told apart,
//!   exactly as `styles.rs` found on the modern side.
//! * A face's *back* texture mapping. The record carries one per side;
//!   `MeshBatch` has one UV array, so the back matrix is parsed and dropped.
//! * `CPolyline3d`, `CDimensionRadial` and anything else not in `Cls`. None
//!   occurs in this corpus. Meeting one ends the entity list it is in, with a
//!   note naming the class, and costs the rest of that list - not the file.
//!
//! # Rules this module keeps
//!
//! Nothing here panics on any input, including deliberately corrupt input:
//! every read goes through `Cur::fill`, which bounds-checks both ends, and
//! the test suite runs the whole corpus bit-flipped, truncated at seven
//! fractions, and as a valid header followed by noise - in a debug build, so
//! that an arithmetic overflow would abort rather than wrap. One malformed
//! entity costs its entity list, and one malformed definition costs that
//! definition: `resync_definition` finds the next one.

use crate::container::Container;
use crate::error::{Error, Result};
use crate::geom::{spend, DefGeometry, RawGuide, RawInstance, NO_UV};
use crate::model::{
    Counts, EdgeBatch, InstanceTable, LayerDef, LengthUnit, MaterialDef, MeshBatch, SceneDef,
    SceneIndex, StyleDef, TextureLocation, TextureRef, INCH_TO_MM, NO_MATERIAL,
};
use crate::scene::{Definition, Model, ReadOptions};
use crate::source::ByteSource;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// The id used for the root entity list, which is a definition in every way
/// except that nothing places it.
pub const ROOT_DEFINITION: u32 = u32::MAX;

/// What a definition is, apart from its geometry.
#[derive(Debug, Clone, Default)]
pub struct DefinitionInfo {
    /// The definition's archive slot, which is also what instances reference.
    pub id: u32,
    pub name: String,
    pub guid: String,
    /// Placed by a `CImage` rather than a component instance. A legacy file
    /// records "this is an image" only by which class places it.
    pub is_image: bool,
    pub always_faces_camera: bool,
}

/// Placements of one definition inside one parent.
///
/// `InstanceTable` has no parent field - the modern path keys it by the
/// definition being placed - so the parent is carried here instead. Dropping
/// it would flatten the hierarchy, which is the one thing this reader must
/// never do: a 27.7 MB classroom is 38,019 placements over 629 meshes.
#[derive(Debug)]
pub struct Placements {
    /// Definition slot of the container, or `ROOT_DEFINITION`.
    pub parent_id: u32,
    pub table: InstanceTable,
}

/// Measurements, because the design claims are all quantitative.
#[derive(Debug, Clone, Default)]
pub struct ReadStats {
    /// Bytes actually requested from the `ByteSource`. The point of the whole
    /// design is that this is far smaller than the file.
    pub bytes_read: u64,
    pub stream_len: u64,
    /// Inline image bytes that were located and deliberately not read.
    pub texture_bytes_skipped: u64,
    pub objects: u64,
    /// Where the archive's object numbering was found to start, and how.
    pub slot_base: u32,
    pub slot_base_method: &'static str,
    /// What the file says about its edges, counted over every definition
    /// before the unplaced ones are dropped: drawable, hidden, soft, smooth.
    /// An edge can be several of those at once, so these do not sum to
    /// `Counts::edges`. This is the number that decides what hidden-line
    /// export looks like - across this corpus 69% of a commercial pack's
    /// edges are softened or smoothed and so are never drawn.
    pub edge_flags: [u64; 4],
    /// `CFaceTextureCoords` sides, counted the same way: front positioned,
    /// front projected, back positioned, back projected. Bit 0 of a side's
    /// flag word is "this side has a positioned mapping" and bit 1 is "the
    /// texture is projected"; `geom.rs` establishes both against 530,000 faces
    /// of the modern container. A model with textures and zeroes here is
    /// painting everything with the default projection, which is a real and
    /// common answer - but so is a reader that lost the link from the face to
    /// its attribute container, which is the defect this number exists to
    /// make visible. The school cafeteria reads 39,580 positioned fronts, of
    /// which 36,183 are projected.
    pub uv_sides: [u64; 4],
    /// Faces that reached a non-identity front mapping - the number that
    /// actually decides a UV, since a side whose flag is set but whose matrix
    /// is the identity is the default projection either way. Counted over
    /// every definition, like `edge_flags`, and directly comparable to the
    /// Python oracle's count of faces with a non-identity `uv_transform`:
    /// see `legacy_uv_matrices_match_the_oracle`.
    pub uv_matrices: u64,
    /// Inner loops - holes - kept across every face in every definition.
    ///
    /// A face's outer loop and its holes are separate records, and a reader
    /// that keeps only the first fills a wall's own door and window openings
    /// with wall. That failure passes every count and bounds check there is,
    /// which is why the number is reported: it is directly comparable to the
    /// Python oracle's `sum(len(f['loops']) - 1)`. The school cafeteria has
    /// 720 over 393 faces, the bmw 237 over 26 - one of them a 97-hole face.
    pub inner_loops: u64,
    /// Faces whose triangulation came back short of the exact count.
    ///
    /// A polygon with `V` vertices over all its loops and `H` holes has
    /// exactly `V + 2H - 2` triangles in **any** triangulation - each hole
    /// costs two. So a shortfall is a dropped loop, and it is the one number
    /// that tells a filled-in window from a model that simply has none.
    ///
    /// It is small rather than zero, and the residue is not this module's: it
    /// is `geom::triangulate` declining to bridge a loop it cannot - a hole
    /// with fewer than three usable corners, or one whose ring collapses -
    /// which is the same decision the modern path makes on the same input. The
    /// restaurant pack reads 2,540 inner loops and comes up short on 15 faces;
    /// the school cafeteria 720 and 29.
    pub faces_short_of_exact: u64,
    /// Entity-list items abandoned after a malformed record. Non-zero means
    /// the model is incomplete but still usable.
    pub lost_entities: u64,
    /// Hidden guide lines and points, which `read_model` leaves out.
    pub hidden_guides: u64,
    /// `CDimensionLinear` and `CText` records read (whether or not they made it into the
    /// model; see `DefGeometry::annotation_skips` for those that did not).
    pub annotations: u64,
    /// Human-readable notes about anything that did not go cleanly.
    pub notes: Vec<String>,
}

/// Everything a legacy file yields, in the same shapes the modern path uses.
pub struct LegacyModel {
    pub index: SceneIndex,
    /// Every definition the file carries, placed or not - a component browser
    /// wants the unplaced ones' names even though nothing turns them into
    /// meshes.
    pub definitions: Vec<DefinitionInfo>,
    /// Triangles, one batch per (definition, material, tag). Only for
    /// definitions something places; see `reachable`.
    pub meshes: Vec<MeshBatch>,
    pub edges: Vec<EdgeBatch>,
    /// Placements as the file states them: one table per (parent, child) with
    /// the transform the file stored, which is relative to the parent. See
    /// `instance_tables` for the composed form the modern path hands out.
    pub placements: Vec<Placements>,
    pub stats: ReadStats,
}

impl LegacyModel {
    /// Which definitions something actually places, directly or by nesting.
    ///
    /// Same notion as `scene::Model::reachable` on the modern path, and it
    /// matters for the same reason: a file carries definitions nobody uses.
    /// SketchUp's blank template ships a scale figure named `Chris`, so four
    /// of the small fixtures here hold about a thousand triangles of a person
    /// who is not in the model - and the C++ oracle agrees, reporting 36 and
    /// 12 `unique_triangles` for files whose definition alone has 32.
    pub fn reachable(&self) -> alloc::collections::BTreeSet<u32> {
        let mut seen = alloc::collections::BTreeSet::new();
        let mut stack = vec![ROOT_DEFINITION];
        let mut guard = 1_000_000u32;
        while let Some(parent) = stack.pop() {
            if guard == 0 {
                break;
            }
            guard -= 1;
            for p in self.placements.iter().filter(|p| p.parent_id == parent) {
                if p.table.is_empty() {
                    continue;
                }
                if seen.insert(p.table.definition_id) {
                    stack.push(p.table.definition_id);
                }
            }
        }
        seen
    }

    /// The placement tree resolved to one table per definition, with world
    /// transforms - the shape `scene::Model::instance_tables` produces, so
    /// that a consumer cannot tell the two containers apart.
    ///
    /// The row total is the file's placed-node count less the root itself:
    /// 79 on `pair_house-plus`, where the C++ oracle reports
    /// `placed_nodes: 80` for both halves of the pair, and where the raw
    /// `CComponentInstance` count is only 66 because thirteen of those
    /// placements are reached through nesting.
    pub fn instance_tables(&self) -> Vec<InstanceTable> {
        // The root is placed once, at identity. It places nothing itself, so
        // a walk that only emits tables for things it finds placed never
        // emits one for the root - and in a room modelled in place that is
        // the floor, the walls and the ceiling. A cafeteria came out as rows
        // of tables standing in white space, with its 바닥 and 천장 tags
        // visible and nothing on them.
        let mut out: Vec<InstanceTable> = alloc::vec![InstanceTable {
            definition_id: ROOT_DEFINITION,
            matrices: IDENTITY_4X4.iter().map(|v| *v as f32).collect(),
            layer_ids: alloc::vec![u32::MAX],
            material_ids: alloc::vec![crate::model::NO_MATERIAL],
            names: Vec::new(),
        }];
        let mut stack: Vec<(u32, [f64; 16], u32)> = vec![(ROOT_DEFINITION, IDENTITY_4X4, 0)];
        let mut guard = 1_000_000u32;
        while let Some((parent, xf, depth)) = stack.pop() {
            if guard == 0 || depth > 32 {
                break;
            }
            guard -= 1;
            for p in self.placements.iter().filter(|p| p.parent_id == parent) {
                let id = p.table.definition_id;
                let at = match out.iter().position(|t| t.definition_id == id) {
                    Some(i) => i,
                    None => {
                        out.push(InstanceTable {
                            definition_id: id,
                            ..Default::default()
                        });
                        out.len() - 1
                    }
                };
                for k in 0..p.table.len() {
                    let mut local = [0.0f64; 16];
                    for a in 0..16 {
                        local[a] = p.table.matrices[k * 16 + a] as f64;
                    }
                    let world = mul_4x4(&xf, &local);
                    for a in 0..16 {
                        out[at].matrices.push(world[a] as f32);
                    }
                    out[at].layer_ids.push(p.table.layer_ids[k]);
                    out[at].material_ids.push(p.table.material_ids[k]);
                    out[at].names.push(p.table.names[k].clone());
                    stack.push((id, world, depth + 1));
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// A cursor over the stream.
//
// The archive is random-access by nature: a definition's tail is found by
// searching a short window ahead, and a texture blob is jumped over. So this
// is a sliding window rather than a sequential reader. The window keeps a
// little history behind the cursor because two records (the definition tail's
// GUID re-anchor, and `CText`'s delimiter) legitimately look backwards.
// ---------------------------------------------------------------------------

const WINDOW: usize = 1 << 16;
/// How far behind the cursor the window is guaranteed to reach after a refill.
const BACKLOG: u64 = 512;

struct Cur<'a> {
    src: &'a dyn ByteSource,
    start: u64,
    end: u64,
    pos: u64,
    win: Vec<u8>,
    win_at: u64,
    read_bytes: u64,
}

impl<'a> Cur<'a> {
    fn new(src: &'a dyn ByteSource, start: u64, end: u64) -> Self {
        Self {
            src,
            start,
            end,
            pos: start,
            win: Vec::new(),
            win_at: 0,
            read_bytes: 0,
        }
    }

    #[inline]
    fn at(&self) -> u64 {
        self.pos
    }

    fn seek(&mut self, to: u64) -> Result<()> {
        if to < self.start || to > self.end {
            return Err(Error::OutOfRange {
                offset: to,
                len: 0,
                source_len: self.end,
            });
        }
        self.pos = to;
        Ok(())
    }

    /// Guarantee `[pos, pos+n)` is in the window and return the window offset
    /// of `pos`.
    fn fill(&mut self, n: usize) -> Result<usize> {
        // Every read in this module funnels through here, which is why the
        // bounds check lives here and not at each call site: `u16_at` and
        // friends set `pos` directly, and a corrupt offset below `start`
        // would otherwise underflow the window arithmetic and ask for a
        // resize of about 2^64 bytes.
        if self.pos < self.start {
            return Err(Error::OutOfRange {
                offset: self.pos,
                len: n,
                source_len: self.end,
            });
        }
        let need_end = self
            .pos
            .checked_add(n as u64)
            .ok_or(Error::Malformed("offset overflow"))?;
        if need_end > self.end {
            return Err(Error::OutOfRange {
                offset: self.pos,
                len: n,
                source_len: self.end,
            });
        }
        let have_end = self.win_at + self.win.len() as u64;
        if self.pos >= self.win_at && need_end <= have_end {
            return Ok((self.pos - self.win_at) as usize);
        }
        // Refill, keeping a little history behind the cursor.
        let back = core::cmp::min(BACKLOG, self.pos - self.start);
        let base = self.pos - back;
        let want = core::cmp::max(n as u64 + back, WINDOW as u64);
        let take = core::cmp::min(want, self.end - base) as usize;
        self.win.resize(take, 0);
        self.src.read_exact_at(base, &mut self.win)?;
        self.read_bytes += take as u64;
        self.win_at = base;
        Ok(back as usize)
    }

    /// Bytes at the cursor without advancing. Only valid up to the next call.
    fn peek_slice(&mut self, n: usize) -> Result<&[u8]> {
        let o = self.fill(n)?;
        Ok(&self.win[o..o + n])
    }

    /// Bytes ending at the cursor. `k` must not exceed `BACKLOG`.
    ///
    /// Two records look backwards - `CText`'s variant middle is delimited by
    /// the eleven bytes *before* a string marker, and a definition's tail
    /// re-anchors on a GUID - so the window keeps a little history. Nothing
    /// asks for more than `BACKLOG`, and asking for more is an error rather
    /// than a silent short read.
    fn behind(&mut self, k: usize) -> Result<&[u8]> {
        if k as u64 > BACKLOG || (k as u64) > self.pos - self.start {
            return Err(Error::Malformed("look-behind past the window"));
        }
        // Force the window to be rebuilt around the cursor if the history is
        // not already there. `fill` always reserves `BACKLOG` bytes behind
        // `pos` when it refills, so one retry is enough.
        if self.fill(1)? < k {
            self.win.clear();
            self.win_at = 0;
            if self.fill(1)? < k {
                return Err(Error::Malformed("look-behind past the window"));
            }
        }
        let o = self.fill(1)?;
        Ok(&self.win[o - k..o])
    }

    #[inline]
    fn u8(&mut self) -> Result<u8> {
        let o = self.fill(1)?;
        self.pos += 1;
        Ok(self.win[o])
    }

    #[inline]
    fn u16(&mut self) -> Result<u16> {
        let o = self.fill(2)?;
        self.pos += 2;
        Ok(u16::from_le_bytes([self.win[o], self.win[o + 1]]))
    }

    #[inline]
    fn u32(&mut self) -> Result<u32> {
        let o = self.fill(4)?;
        self.pos += 4;
        Ok(u32::from_le_bytes([
            self.win[o],
            self.win[o + 1],
            self.win[o + 2],
            self.win[o + 3],
        ]))
    }

    #[inline]
    fn peek_u16(&mut self) -> Result<u16> {
        let o = self.fill(2)?;
        Ok(u16::from_le_bytes([self.win[o], self.win[o + 1]]))
    }

    /// One byte at an absolute offset, or `None` if it is out of range.
    ///
    /// `Option` rather than `Result` because the one caller - the rendering
    /// options block, which is read at fixed offsets from a located anchor -
    /// treats "not there" and "not plausible" the same way.
    fn byte_at(&mut self, at: u64) -> Option<u8> {
        let save = self.pos;
        self.pos = at;
        let r = self.fill(1).ok().map(|o| self.win[o]);
        self.pos = save;
        r
    }

    fn u16_at(&mut self, at: u64) -> Result<u16> {
        let save = self.pos;
        self.pos = at;
        let r = self.peek_u16();
        self.pos = save;
        r
    }

    fn u32_at(&mut self, at: u64) -> Result<u32> {
        let save = self.pos;
        self.pos = at;
        let o = self.fill(4);
        let r = o.map(|o| {
            u32::from_le_bytes([
                self.win[o],
                self.win[o + 1],
                self.win[o + 2],
                self.win[o + 3],
            ])
        });
        self.pos = save;
        r
    }

    #[inline]
    fn f64(&mut self) -> Result<f64> {
        let o = self.fill(8)?;
        self.pos += 8;
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.win[o..o + 8]);
        Ok(f64::from_le_bytes(b))
    }

    fn f64s<const N: usize>(&mut self) -> Result<[f64; N]> {
        let o = self.fill(N * 8)?;
        self.pos += (N * 8) as u64;
        let mut out = [0.0f64; N];
        for (i, slot) in out.iter_mut().enumerate() {
            let mut b = [0u8; 8];
            b.copy_from_slice(&self.win[o + i * 8..o + i * 8 + 8]);
            *slot = f64::from_le_bytes(b);
        }
        Ok(out)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let o = self.fill(N)?;
        self.pos += N as u64;
        let mut out = [0u8; N];
        out.copy_from_slice(&self.win[o..o + N]);
        Ok(out)
    }

    fn skip(&mut self, n: u64) -> Result<()> {
        let to = self
            .pos
            .checked_add(n)
            .ok_or(Error::Malformed("skip overflow"))?;
        if to > self.end {
            return Err(Error::OutOfRange {
                offset: self.pos,
                len: n as usize,
                source_len: self.end,
            });
        }
        self.pos = to;
        Ok(())
    }

    fn left(&self) -> u64 {
        self.end - self.pos
    }

    /// `ff fe ff <len> <len UTF-16LE units>`, MFC's `CString` on the wire.
    ///
    /// The length escapes to 16 and then 32 bits, which matters: a Korean
    /// pack's definition names are short but a dynamic-component attribute
    /// value can be kilobytes.
    fn utf16(&mut self) -> Result<String> {
        let m = self.fixed::<4>()?;
        if m[0] != 0xff || m[1] != 0xfe || m[2] != 0xff {
            self.pos -= 4;
            return Err(Error::Malformed("expected a string record"));
        }
        let mut units = m[3] as usize;
        if units == 0xff {
            units = self.u16()? as usize;
            if units == 0xffff {
                units = self.u32()? as usize;
            }
        }
        // A string longer than this is not a name; it is a misread length.
        if units > 1 << 22 {
            return Err(Error::Malformed("implausible string length"));
        }
        let mut s = String::with_capacity(units);
        let mut hi: Option<u16> = None;
        let mut i = 0;
        while i < units {
            let chunk = core::cmp::min(units - i, 4096);
            let o = self.fill(chunk * 2)?;
            for j in 0..chunk {
                let c = u16::from_le_bytes([self.win[o + j * 2], self.win[o + j * 2 + 1]]);
                match hi.take() {
                    Some(h) if (0xdc00..0xe000).contains(&c) => {
                        let cp = 0x10000 + (((h as u32) - 0xd800) << 10) + ((c as u32) - 0xdc00);
                        s.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                    }
                    Some(_) => {
                        s.push('\u{fffd}');
                        if (0xd800..0xdc00).contains(&c) {
                            hi = Some(c);
                        } else {
                            s.push(char::from_u32(c as u32).unwrap_or('\u{fffd}'));
                        }
                    }
                    None => {
                        if (0xd800..0xdc00).contains(&c) {
                            hi = Some(c);
                        } else {
                            s.push(char::from_u32(c as u32).unwrap_or('\u{fffd}'));
                        }
                    }
                }
            }
            self.pos += (chunk * 2) as u64;
            i += chunk;
        }
        if hi.is_some() {
            s.push('\u{fffd}');
        }
        Ok(s)
    }

    /// First occurrence of `needle` in `[pos, pos+limit)`, absolute.
    fn find(&mut self, needle: &[u8], limit: u64) -> Result<Option<u64>> {
        if needle.is_empty() {
            return Ok(None);
        }
        let stop = core::cmp::min(self.end, self.pos.saturating_add(limit));
        let save = self.pos;
        let mut at = self.pos;
        while at < stop {
            let span = core::cmp::min((stop - at) as usize, WINDOW);
            if span < needle.len() {
                break;
            }
            self.pos = at;
            let o = match self.fill(span) {
                Ok(o) => o,
                Err(_) => break,
            };
            let hay = &self.win[o..o + span];
            if let Some(i) = hay.windows(needle.len()).position(|w| w == needle) {
                self.pos = save;
                return Ok(Some(at + i as u64));
            }
            // Overlap by needle-1 so a match straddling two spans is found.
            at += (span - (needle.len() - 1)) as u64;
        }
        self.pos = save;
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Classes.
//
// 65 model classes implement a `CArchive::Serialize`; this reader needs the
// 26 that carry geometry, paint, or structure. A class is interned to this
// enum at its declaration so the per-object dispatch is an integer compare
// rather than a string compare - the C++ reader's single god-struct is
// discriminated by a `std::string` per object, which is most of why it spends
// 1.05 GB on a 46 MB file.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cls {
    Vertex,
    Edge,
    Curve,
    ArcCurve,
    EdgeUse,
    Loop,
    Face,
    Layer,
    Material,
    Dib,
    AttrContainer,
    AttrNamed,
    Camera,
    Thumbnail,
    Relationship,
    Definition,
    Instance,
    Group,
    Image,
    Ftc,
    ConstructionLine,
    ConstructionPoint,
    SectionPlane,
    SkFont,
    DimLinear,
    Text,
    ViewPage,
    /// Declared in the file but not something this reader can walk. Meeting
    /// one is fatal to the enclosing list only, never to the file.
    Unknown,
}

fn class_of(name: &[u8]) -> Cls {
    match name {
        b"CVertex" => Cls::Vertex,
        b"CEdge" => Cls::Edge,
        b"CCurve" => Cls::Curve,
        b"CArcCurve" => Cls::ArcCurve,
        b"CEdgeUse" => Cls::EdgeUse,
        b"CLoop" => Cls::Loop,
        b"CFace" => Cls::Face,
        b"CLayer" => Cls::Layer,
        b"CMaterial" => Cls::Material,
        b"CDib" => Cls::Dib,
        b"CAttributeContainer" => Cls::AttrContainer,
        b"CAttributeNamed" => Cls::AttrNamed,
        b"CCamera" => Cls::Camera,
        b"CThumbnail" => Cls::Thumbnail,
        b"CRelationship" => Cls::Relationship,
        b"CComponentDefinition" => Cls::Definition,
        b"CComponentInstance" => Cls::Instance,
        b"CGroup" => Cls::Group,
        b"CImage" => Cls::Image,
        b"CFaceTextureCoords" => Cls::Ftc,
        b"CConstructionLine" => Cls::ConstructionLine,
        b"CConstructionPoint" => Cls::ConstructionPoint,
        b"CSectionPlane" => Cls::SectionPlane,
        b"CSkFont" => Cls::SkFont,
        b"CDimensionLinear" => Cls::DimLinear,
        b"CText" => Cls::Text,
        b"CViewPage" => Cls::ViewPage,
        _ => Cls::Unknown,
    }
}

fn class_name(c: Cls) -> &'static str {
    match c {
        Cls::Vertex => "CVertex",
        Cls::Edge => "CEdge",
        Cls::Curve => "CCurve",
        Cls::ArcCurve => "CArcCurve",
        Cls::EdgeUse => "CEdgeUse",
        Cls::Loop => "CLoop",
        Cls::Face => "CFace",
        Cls::Layer => "CLayer",
        Cls::Material => "CMaterial",
        Cls::Dib => "CDib",
        Cls::AttrContainer => "CAttributeContainer",
        Cls::AttrNamed => "CAttributeNamed",
        Cls::Camera => "CCamera",
        Cls::Thumbnail => "CThumbnail",
        Cls::Relationship => "CRelationship",
        Cls::Definition => "CComponentDefinition",
        Cls::Instance => "CComponentInstance",
        Cls::Group => "CGroup",
        Cls::Image => "CImage",
        Cls::Ftc => "CFaceTextureCoords",
        Cls::ConstructionLine => "CConstructionLine",
        Cls::ConstructionPoint => "CConstructionPoint",
        Cls::SectionPlane => "CSectionPlane",
        Cls::SkFont => "CSkFont",
        Cls::DimLinear => "CDimensionLinear",
        Cls::Text => "CText",
        Cls::ViewPage => "CViewPage",
        Cls::Unknown => "<unknown>",
    }
}

// Slot-table encoding. One `u64` per archive slot; a `Vec` indexed by slot,
// not a map of heap objects.
const K_UNSET: u8 = 0;
const K_CLASS: u8 = 1;
/// A reference into the unwalked pre-model prefix: real, but not ours.
const K_PREMODEL: u8 = 2;
/// An index the writer burned without serializing anything (see `BURN`).
const K_RESERVED: u8 = 3;
/// Geometry that belonged to a definition already emitted and freed.
const K_FREED: u8 = 4;
const K_OBJ: u8 = 16;

#[derive(Clone, Copy)]
struct Slot {
    kind: u8,
    /// Index into the arena for this kind, or `u32::MAX` when the object
    /// carries nothing this reader keeps.
    idx: u32,
}

const NONE_IDX: u32 = u32::MAX;

impl Slot {
    const UNSET: Slot = Slot {
        kind: K_UNSET,
        idx: NONE_IDX,
    };
    fn obj(c: Cls, idx: u32) -> Slot {
        Slot {
            kind: K_OBJ + c as u8,
            idx,
        }
    }
    fn is(&self, c: Cls) -> bool {
        self.kind == K_OBJ + c as u8
    }
}

/// `CVersionMap`: the file's own table of class schemas, in the clear.
///
/// Every legacy file opens with it, so every version-dependent byte count in
/// this module can be keyed off the number the *writer* branched on rather
/// than off the writer's marketing version. Read straight out of the first few
/// kilobytes; no walking required.
#[derive(Default)]
struct VersionMap {
    entity: u16,
    layer: u16,
    instance: u16,
    /// `CRenderingOptions`. Not a record this reader walks - the options are a
    /// flat block in the unwalked prefix - but the schema is the only thing in
    /// the file that says which *layout* that block has. See
    /// `read_rendering_options`.
    rendering_options: u16,
    /// `CLayerManager`: 4 up to SketchUp 2019, 7 from SketchUp 2020, whose
    /// container adds a filler before the definition count (see
    /// `Archive::v20`).
    layer_manager: u16,
    found: bool,
}

fn read_version_map(c: &mut Cur) -> VersionMap {
    let mut vm = VersionMap::default();
    // The map is the first object in the archive; 8 KiB covers all 59 entries
    // in every file measured (the longest runs to 0x7bb).
    let Ok(Some(at)) = c.find(b"CVersionMap", 16 * 1024) else {
        return vm;
    };
    let save = c.at();
    if c.seek(at + 11).is_err() {
        return vm;
    }
    loop {
        let Ok(name) = c.utf16() else { break };
        let Ok(schema) = c.u32() else { break };
        let schema = schema as u16;
        match name.as_str() {
            "CEntity" => vm.entity = schema,
            "CLayer" => vm.layer = schema,
            "CComponentInstance" => vm.instance = schema,
            "CRenderingOptions" => vm.rendering_options = schema,
            "CLayerManager" => vm.layer_manager = schema,
            "End-Of-Version-Map" => {
                vm.found = true;
                break;
            }
            _ => {}
        }
        if c.left() == 0 {
            break;
        }
    }
    let _ = c.seek(save);
    vm
}

// ---------------------------------------------------------------------------
// Arena records. Flat, fixed-size, no per-entity heap allocation.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct EdgeRec {
    v1: u32,
    v2: u32,
    /// `model::edge_flag` bits: 0x01 hidden, 0x08 soft, 0x10 smooth.
    flags: u8,
    /// Archive slot of the `CLayer` this edge is on, 0 for the default.
    layer_slot: u32,
}

#[derive(Clone, Copy)]
struct FaceRec {
    normal: [f64; 3],
    /// The plane's fourth term, for the topology output.
    plane_d: f64,
    /// Archive slot of the back material, 0 for none.
    back_mat_slot: u32,
    /// `CDrawingElement` flags; bit 0 is hidden.
    flags: u8,
    /// Range into `loops`.
    loop_start: u32,
    loop_len: u32,
    mat_slot: u32,
    layer_slot: u32,
    hidden: bool,
    /// Index into `ftcs`, or `NONE_IDX` when this face's front texture is the
    /// default planar projection. Reached through the face's attribute
    /// container, which is where the `CFaceTextureCoords` record lives.
    ftc: u32,
}

/// The front side of a `CFaceTextureCoords`: its texture-space-to-face-plane
/// matrix, row-major 3x3, in inches. See `geom::UvMap` for what to do with it.
///
/// Only the front side is kept, and only when it is not the identity.
/// `MeshBatch` carries one UV array, so there is nowhere to put the back
/// side's mapping; it is parsed - it has to be, the record has no length field
/// - and dropped. The flag words are not kept either: they are counted into
/// `ReadStats::uv_sides` as the record is read, and nothing renders from them
/// (see `geom.rs`, which establishes that the matrix already encodes a
/// projected mapping exactly).
#[derive(Clone, Copy)]
struct FtcRec {
    front: [f64; 9],
    /// The front side's flag word, for the topology output's `uv_flags`.
    front_flags: u32,
}

#[derive(Clone, Copy)]
struct LoopRec {
    /// Range into `loop_uses`.
    start: u32,
    len: u32,
}

#[derive(Clone, Copy)]
struct LoopUse {
    edge: u32,
    /// True when the loop traverses the edge from v2 to v1.
    reversed: bool,
}

struct LayerRec {
    name: String,
    hidden: bool,
    rgba: [u8; 4],
}

struct MaterialRec {
    name: String,
    rgba: [u8; 4],
    /// The file stores transparency, gated by a trailing use-flag byte.
    opacity: f32,
    texture: Option<TextureRef>,
    colorized: bool,
}

/// An inline image: where its bytes are, never the bytes.
#[derive(Clone, Copy)]
struct DibRec {
    offset: u64,
    len: u64,
}

struct InstRec {
    parent: u32,
    def_slot: u32,
    /// 13 doubles exactly as stored.
    xf: [f64; 13],
    mat_slot: u32,
    layer_slot: u32,
    name: String,
    /// Placed by a `CGroup` rather than a `CComponentInstance`.
    is_group: bool,
    /// Placed by a `CImage`.
    is_image: bool,
    hidden: bool,
}

// ---------------------------------------------------------------------------
// The archive walker.
// ---------------------------------------------------------------------------

/// How deeply objects may nest. Definitions nest through instances, so this is
/// file-controlled: a component placed inside itself would recurse forever.
const MAX_DEPTH: u32 = 64;

const N_CLS: usize = 28;

/// What must follow a count `count_after_v20_filler` finds.
#[derive(Clone, Copy)]
enum Follows {
    /// An entity, a relationship: anything `plausible_list_tag` accepts.
    ListItem,
    /// The first definition of the definition list.
    Definition,
}

/// What `read_definition` turns a completed definition into.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Emit {
    /// Mesh and edge batches, for `read`.
    Batches,
    /// A `DefGeometry`, for `read_model`.
    Topology,
    /// Nothing: `read_index` wants only the materials, layers and scenes.
    Nothing,
}

/// The most bytes `resync_definition` scans over a whole file, all walks
/// together. A two-byte needle has many false positives, and every failed
/// definition starts another scan.
const RESYNC_BUDGET: u64 = 64 << 20;

/// The most slot bases `read` and `read_model` walk the file from. The probe
/// offers one candidate per model layer when no class repeats before the
/// definition list; a file of many layers could otherwise ask for as many
/// whole walks.
const MAX_BASES: usize = 4;

/// What one definition counts against `ReadOptions::max_elements`, as on the
/// modern path (`scene.rs`).
const DEFINITION_WEIGHT: u64 = 50;

/// The block SketchUp 2020 writes between the active-layer reference and the
/// definition count: an empty string record and nine zero bytes, after three
/// zero bytes. Measured on every 2020 file of the corpus.
const V20_FILLER: [u8; 16] = [
    0x00, 0x00, 0x00, 0xff, 0xfe, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Def,
    Root,
}

struct Archive<'a> {
    c: Cur<'a>,
    /// Major writer version from the header, e.g. 17. Used only where the
    /// version map has nothing to say (the pre-2014 instance GUID).
    ver: u32,
    /// `CEntity` schema >= 5: entity preambles carry a persistent-id mask,
    /// a definition's base block is 22 bytes rather than 20, and a texture
    /// block's leading pad is 2 bytes rather than 1. One gate, three uses -
    /// which is why keying it off `CEntity` and not the version string is
    /// worth doing.
    wide_entity: bool,
    /// Trailing block of a colour `CLayer`: 21 bytes at schema 2, 23 at
    /// schema 3. Measured here; see the module docs.
    layer_tail: u64,
    /// `CRenderingOptions` schema, which selects the layout of the flat
    /// options block - see `read_rendering_options`.
    ro_schema: u16,

    base: u32,
    slots: Vec<Slot>,
    classes: Vec<Cls>,
    class_slot: [u32; N_CLS],

    // Arenas.
    verts: Vec<[f64; 3]>,
    edges: Vec<EdgeRec>,
    faces: Vec<FaceRec>,
    loops: Vec<LoopRec>,
    loop_uses: Vec<LoopUse>,
    face_loops: Vec<u32>,
    /// Positioned texture mappings, one per `CFaceTextureCoords` that carries
    /// a non-identity front matrix. Stack-like with the rest of the geometry
    /// arenas, so the resident cost is one definition's worth and not the
    /// model's: the school cafeteria has 39,580 of these.
    ftcs: Vec<FtcRec>,
    layers: Vec<LayerRec>,
    layer_by_name: BTreeMap<String, u32>,
    materials: Vec<MaterialRec>,
    dibs: Vec<DibRec>,
    instances: Vec<InstRec>,
    /// Definition slot -> info, in declaration order.
    defs: Vec<DefinitionInfo>,
    /// Definitions placed by a `CImage`.
    image_defs: alloc::collections::BTreeSet<u32>,
    /// Emitted per definition as it completes, so the resident high-water
    /// mark is one definition and not the model.
    meshes: Vec<MeshBatch>,
    edge_batches: Vec<EdgeBatch>,

    // Walk state.
    depth: u32,
    cur_loop: Option<u32>,
    /// Which definition's geometry is currently being accumulated.
    def_stack: Vec<u32>,
    /// Guide lines and points, stack-like with the geometry arenas.
    guides: Vec<RawGuide>,
    /// The layer slot of each guide, parallel to `guides`.
    guide_layers: Vec<u32>,
    /// See `ReadStats::hidden_guides`.
    hidden_guides: u64,
    /// The hidden guides read, and each one's layer slot, kept for their definition.
    hidden_guide_items: Vec<RawGuide>,
    hidden_guide_layers: Vec<u32>,
    /// See `ReadStats::annotations`.
    annotations: u64,
    /// Dimensions and text read in the definitions still being built, stack-like with the
    /// geometry arenas; resolved against the definition's vertices and guide points when it
    /// is built (a record can be written before the geometry it refers to).
    pending_annots: Vec<PendingAnnot>,
    /// Where the last image read ended, when it was a JPEG.
    jpeg_end: Option<u64>,
    /// What a completed definition is turned into.
    emit: Emit,
    /// `Emit::Topology` output: one per definition read, in file order.
    topo_defs: Vec<Definition>,
    topo_root: Option<DefGeometry>,
    /// What `ReadOptions::max_elements` has left for the topology output.
    budget: u64,
    /// Set once `budget` ran out. Sticky: the walk catches a definition's
    /// error and moves on, so the failure has to outlive that.
    over_budget: bool,
    /// SketchUp 2020's container: `CLayerManager` schema 7 or more. Its
    /// writer adds a filler before the definition count; see `walk`.
    v20: bool,
    /// Bytes `resync_definition` may still scan, shared by every walk of the
    /// file so that the retries of a damaged file stay bounded.
    resync_left: u64,
    cline_tail: Option<u64>,
    /// Whether any material in the file carries an image. A model painted
    /// only in flat colour gets no UVs at all - there is nothing for them to
    /// address - which is the same rule `scene::textured_materials` applies to
    /// the modern path. The material manager is written before the first
    /// definition, so this is settled before any batch is built.
    any_textured: bool,

    // Burned store-map indices; see `register_burn`.
    burns: Vec<(u32, u32)>,
    cum_delta: u32,
    annot_watermark: Option<u32>,
    burn_credit: Vec<u32>,

    /// Layer slots the *model* list declared, as distinct from the copies
    /// every definition re-serializes for the same tag.
    model_layers: Vec<u32>,
    /// Where the entity walk finished, which is where the document tail - and
    /// the page list - begins.
    walk_end: u64,
    /// `CSkpStyle` records found in the tail, in file order. Filled before
    /// `scan_pages`, which needs them to say which style each tab selects.
    style_recs: Vec<StyleRec>,

    lost: u64,
    objects: u64,
    /// See `ReadStats::uv_sides`.
    uv_sides: [u64; 4],
    /// See `ReadStats::uv_matrices`.
    uv_matrices: u64,
    /// See `ReadStats::inner_loops`.
    inner_loops: u64,
    /// See `ReadStats::faces_short_of_exact`.
    faces_short: u64,
    notes: Vec<String>,
    /// Set once the first class-ref to a class we declared ourselves pins the
    /// absolute numbering; see `probe_slot_base`.
    learned_base: Option<(u32, &'static str)>,
    probing: bool,
}

/// The sentinel base used while probing. Chosen far above any real slot count
/// so that every absolute reference in the file classifies as pre-model and is
/// tolerated rather than resolved.
const PROBE_BASE: u32 = 1 << 24;

impl<'a> Archive<'a> {
    fn new(c: Cur<'a>, ver: u32, vm: &VersionMap) -> Self {
        let v20 = if vm.found {
            vm.layer_manager >= 7
        } else {
            ver >= 20
        };
        // The version map is the better source because it is the number the
        // writer branched on, but a file that has lost it is still worth
        // trying: the writer version tracks both gates across this corpus
        // (`CEntity` 5 from 2017, `CLayer` 3 from 2019).
        let wide_entity = if vm.found { vm.entity >= 5 } else { ver >= 17 };
        let layer_tail = if vm.found {
            if vm.layer >= 3 {
                23
            } else {
                21
            }
        } else if ver >= 19 {
            23
        } else {
            21
        };
        Self {
            c,
            ver,
            wide_entity,
            layer_tail,
            // Zero when the version map is missing, which the reader treats as
            // the schema-36 layout and then confirms by plausibility - the
            // right fallback, because every file old enough to have lost its
            // map is older than schema 38.
            ro_schema: vm.rendering_options,
            base: 0,
            slots: Vec::new(),
            classes: Vec::new(),
            class_slot: [NONE_IDX; N_CLS],
            verts: Vec::new(),
            edges: Vec::new(),
            faces: Vec::new(),
            loops: Vec::new(),
            loop_uses: Vec::new(),
            face_loops: Vec::new(),
            ftcs: Vec::new(),
            layers: Vec::new(),
            layer_by_name: BTreeMap::new(),
            materials: Vec::new(),
            dibs: Vec::new(),
            instances: Vec::new(),
            defs: Vec::new(),
            image_defs: alloc::collections::BTreeSet::new(),
            meshes: Vec::new(),
            edge_batches: Vec::new(),
            depth: 0,
            cur_loop: None,
            def_stack: Vec::new(),
            guides: Vec::new(),
            guide_layers: Vec::new(),
            hidden_guides: 0,
            hidden_guide_items: Vec::new(),
            hidden_guide_layers: Vec::new(),
            annotations: 0,
            pending_annots: Vec::new(),
            jpeg_end: None,
            emit: Emit::Batches,
            topo_defs: Vec::new(),
            topo_root: None,
            budget: u64::MAX,
            over_budget: false,
            v20,
            resync_left: RESYNC_BUDGET,
            cline_tail: None,
            any_textured: false,
            burns: Vec::new(),
            cum_delta: 0,
            annot_watermark: None,
            burn_credit: Vec::new(),
            model_layers: Vec::new(),
            walk_end: 0,
            style_recs: Vec::new(),
            lost: 0,
            objects: 0,
            uv_sides: [0; 4],
            uv_matrices: 0,
            inner_loops: 0,
            faces_short: 0,
            notes: Vec::new(),
            learned_base: None,
            probing: false,
        }
    }

    #[inline]
    fn next_slot(&self) -> u32 {
        self.base.wrapping_add(self.slots.len() as u32)
    }

    #[inline]
    fn index_of(&self, slot: u32) -> Option<usize> {
        slot.checked_sub(self.base)
            .map(|i| i as usize)
            .filter(|i| *i < self.slots.len())
    }

    fn entry(&self, slot: u32) -> Slot {
        match self.index_of(slot) {
            Some(i) => self.slots[i],
            None => Slot::UNSET,
        }
    }

    fn alloc(&mut self, s: Slot) -> u32 {
        let slot = self.next_slot();
        self.slots.push(s);
        slot
    }

    fn set(&mut self, slot: u32, s: Slot) {
        if let Some(i) = self.index_of(slot) {
            self.slots[i] = s;
        }
    }

    fn note(&mut self, s: String) {
        if self.notes.len() < 64 {
            self.notes.push(s);
        }
    }

    // -- the wire layer ---------------------------------------------------

    /// Read one object tag and whatever follows it.
    ///
    /// `expect` is the class the *caller's* position in the file guarantees.
    /// It is needed because a class declared in the unwalked prefix has a slot
    /// number we never saw, so a class-ref to it cannot be resolved - but the
    /// grammar says what it must be, and that is enough to register it and
    /// keep reading. This is also the hook that pins the slot base: see
    /// `note_class_ref`.
    fn read_object(&mut self, expect: Option<Cls>) -> Result<Option<(u32, Slot)>> {
        let tag = self.c.u16()?;
        if tag == 0 {
            return Ok(None);
        }
        if tag == 0x7fff {
            let big = self.c.u32()?;
            if big & 0x8000_0000 != 0 {
                return self.new_of_class(big & 0x7fff_ffff, expect).map(Some);
            }
            return self.back_ref(big).map(Some);
        }
        if tag == 0xffff {
            let schema = self.c.u16()?;
            let namelen = self.c.u16()? as usize;
            // Class names in this format are short; 40 is well past the
            // longest (`CConstructionGeometry`, 21). A bigger one means the
            // cursor is not on a class record at all.
            if namelen == 0 || namelen > 40 {
                return Err(Error::Malformed("implausible class name length"));
            }
            let mut name = [0u8; 40];
            {
                let b = self.c.peek_slice(namelen)?;
                name[..namelen].copy_from_slice(b);
            }
            self.c.skip(namelen as u64)?;
            let cls = class_of(&name[..namelen]);
            let ci = self.classes.len() as u32;
            self.classes.push(cls);
            let cslot = self.alloc(Slot {
                kind: K_CLASS,
                idx: ci,
            });
            if cls != Cls::Unknown {
                self.class_slot[cls as usize] = cslot;
            }
            let _ = schema;
            return self.new_obj(cls).map(Some);
        }
        if tag & 0x8000 != 0 {
            return self.new_of_class((tag & 0x7fff) as u32, expect).map(Some);
        }
        self.back_ref(tag as u32).map(Some)
    }

    /// A class-ref: "a new object of the class in this slot".
    fn new_of_class(&mut self, cslot: u32, expect: Option<Cls>) -> Result<(u32, Slot)> {
        let e = self.entry(cslot);
        let cls = if e.kind == K_CLASS {
            self.classes
                .get(e.idx as usize)
                .copied()
                .unwrap_or(Cls::Unknown)
        } else if e.kind == K_UNSET {
            // Declared in the prefix we never walked. The grammar names it.
            let Some(want) = expect else {
                return Err(Error::Malformed("class-ref to an unknown slot"));
            };
            self.note_class_ref(cslot, want);
            if self.class_slot[want as usize] == NONE_IDX {
                self.class_slot[want as usize] = cslot;
            }
            want
        } else {
            return Err(Error::Malformed("class-ref to a non-class slot"));
        };
        self.new_obj(cls)
    }

    /// Record that the file calls `want`'s class `cslot`.
    ///
    /// This is how the absolute slot base is recovered. While probing, our own
    /// numbering is relative to `PROBE_BASE`, so if we declared `want`'s class
    /// ourselves at relative index `r`, then `base = cslot - r`: one equation,
    /// exact, from a class the grammar names rather than from trial and error.
    fn note_class_ref(&mut self, cslot: u32, want: Cls) {
        if !self.probing || self.learned_base.is_some() {
            return;
        }
        let mine = self.class_slot[want as usize];
        if mine == NONE_IDX || mine < self.base {
            return;
        }
        let rel = mine - self.base;
        if cslot <= rel {
            return;
        }
        self.learned_base = Some((cslot - rel, "class-ref equation"));
    }

    fn new_obj(&mut self, cls: Cls) -> Result<(u32, Slot)> {
        if self.depth >= MAX_DEPTH {
            return Err(Error::Malformed("object nesting too deep"));
        }
        let slot = self.alloc(Slot::UNSET);
        self.objects += 1;
        self.depth += 1;
        let r = self.read_body(cls, slot);
        self.depth -= 1;
        let entry = match r {
            Ok(e) => e,
            Err(e) => {
                // Name the class and the offset. Without this a failure
                // surfaces as "back-ref to an unwalked slot" several records
                // later, which says nothing about which record was misread.
                self.note(format!("{} at {:#x}: {e}", class_name(cls), self.c.at()));
                return Err(e);
            }
        };
        self.set(slot, entry);
        if matches!(cls, Cls::DimLinear | Cls::Text) {
            self.annot_watermark = Some(self.next_slot());
            self.annotations += 1;
        }
        Ok((slot, entry))
    }

    fn back_ref(&mut self, mut slot: u32) -> Result<(u32, Slot)> {
        if !self.burns.is_empty() && slot >= self.burns[0].0 {
            match self.translate_ref(slot)? {
                Some(w) => slot = w,
                // A phantom index: the writer mapped an annotation's
                // connection point into the store map without serializing
                // anything for it.
                None => {
                    return Ok((
                        slot,
                        Slot {
                            kind: K_RESERVED,
                            idx: NONE_IDX,
                        },
                    ))
                }
            }
        }
        let e = self.entry(slot);
        if e.kind == K_UNSET {
            if slot < self.base {
                return Ok((
                    slot,
                    Slot {
                        kind: K_PREMODEL,
                        idx: NONE_IDX,
                    },
                ));
            }
            return Err(Error::Malformed("back-ref to an unwalked slot"));
        }
        if e.kind == K_CLASS {
            return Err(Error::Malformed("back-ref to a class slot"));
        }
        Ok((slot, e))
    }

    /// Map a file store-map index through the burn bands to our numbering:
    /// `None` for an index inside a band. The bands come from the file, so
    /// their sums are checked.
    fn translate_ref(&self, slot: u32) -> Result<Option<u32>> {
        const OVERFLOW: Error = Error::Malformed("burned slot band out of range");
        let mut offset = 0u32;
        for (start, width) in &self.burns {
            if slot < *start {
                break;
            }
            if slot < start.checked_add(*width).ok_or(OVERFLOW)? {
                return Ok(None);
            }
            offset = offset.checked_add(*width).ok_or(OVERFLOW)?;
        }
        Ok(slot.checked_sub(offset))
    }

    /// The writer maps an annotation's connection points into the MFC store
    /// map without writing any bytes for them, so the file's later
    /// back-references run ahead of a byte-exact walk. Registrations stay at
    /// walker indices; `back_ref` translates file references instead.
    fn register_burn(&mut self, delta: u32) -> Result<()> {
        const OVERFLOW: Error = Error::Malformed("burned slot band out of range");
        if let Some(w) = self.annot_watermark {
            let start = w.checked_add(self.cum_delta).ok_or(OVERFLOW)?;
            self.cum_delta = self.cum_delta.checked_add(delta).ok_or(OVERFLOW)?;
            self.burns.push((start, delta));
            self.annot_watermark = None;
            if let Some(last) = self.burn_credit.last_mut() {
                *last = last.saturating_add(1);
            }
        }
        Ok(())
    }

    // -- shared record blocks --------------------------------------------

    /// Entity preamble: an attribute-container pointer, plus a persistent-id
    /// mask and its bytes on `CEntity` schema 5 and later.
    fn preamble(&mut self) -> Result<Option<(u32, Slot)>> {
        let attrs = self.read_object(Some(Cls::AttrContainer))?;
        if self.wide_entity {
            let mask = self.c.u8()?;
            self.c.skip(mask.count_ones() as u64)?;
        }
        Ok(attrs)
    }

    /// `CDrawingElement`: material, visibility, the soft/smooth bits, layer.
    ///
    /// Bit positions confirmed against `CEdge::IsSoft`/`IsSmooth`, which read
    /// bytes 5 and 6 of this block - not bits of a flag word.
    fn drawbase(&mut self) -> Result<(u32, u8, u32, bool)> {
        let b = self.c.fixed::<8>()?;
        let mat = u16::from_le_bytes([b[0], b[1]]) as u32;
        let hidden = b[2] != 0;
        let mut flags = 0u8;
        if hidden {
            flags |= crate::model::edge_flag::HIDDEN;
        }
        if b[5] != 0 {
            flags |= crate::model::edge_flag::SOFT;
        }
        if b[6] != 0 {
            flags |= crate::model::edge_flag::SMOOTH;
        }
        // The layer is normally a slot id, but an entity can carry its layer
        // *by object* instead: a full inline `CLayer` on first use, an escaped
        // back-ref on later siblings. Layer ids never set 0x8000 and never
        // equal 0x7fff, so both object forms are unambiguous.
        let lay_cls = self.class_slot[Cls::Layer as usize];
        let tag = self.c.peek_u16()?;
        let layer;
        if lay_cls != NONE_IDX && lay_cls < 0x7fff && tag == (0x8000 | lay_cls as u16) {
            let r = self.read_object(Some(Cls::Layer))?;
            layer = r.map(|(s, _)| s).unwrap_or(0);
        } else if tag == 0x7fff {
            let _ = self.c.u16()?;
            let big = self.c.u32()?;
            if big & 0x8000_0000 != 0 {
                return Err(Error::Malformed("drawbase layer is a new class"));
            }
            layer = big;
        } else {
            layer = self.c.u16()? as u32;
        }
        Ok((mat, flags, layer, hidden))
    }

    /// A reference to an entity that may not have been walked yet.
    ///
    /// SketchUp serializes a dimension or a text label *before* the geometry
    /// it anchors to when both are in the same list, so these legitimately
    /// point forward - which the back-ref path rightly refuses.
    fn entity_ref(&mut self) -> Result<Option<u32>> {
        let tag = self.c.u16()?;
        if tag == 0 {
            return Ok(None);
        }
        if tag == 0x7fff {
            let big = self.c.u32()?;
            if big & 0x8000_0000 != 0 {
                return Err(Error::Malformed("entity ref is a new object"));
            }
            return Ok(Some(big));
        }
        if tag == 0xffff || tag & 0x8000 != 0 {
            return Err(Error::Malformed("entity ref is a new object"));
        }
        Ok(Some(tag as u32))
    }

    // -- per-class readers -----------------------------------------------

    fn read_body(&mut self, cls: Cls, slot: u32) -> Result<Slot> {
        match cls {
            Cls::Vertex => {
                self.preamble()?;
                let p = self.c.f64s::<3>()?;
                let i = self.verts.len() as u32;
                self.verts.push(p);
                Ok(Slot::obj(Cls::Vertex, i))
            }
            Cls::Edge => {
                self.preamble()?;
                let (_mat, flags, layer, _hidden) = self.drawbase()?;
                // The two vertex pointers come before the curve pointer -
                // the public 2017 notes have them the other way round.
                let v1 = self.vertex_ref()?;
                let v2 = self.vertex_ref()?;
                match self.read_object(None)? {
                    None => {}
                    Some((_, e)) => {
                        if !(e.is(Cls::Curve) || e.is(Cls::ArcCurve) || e.kind == K_PREMODEL) {
                            return Err(Error::Malformed("edge curve pointer is not a curve"));
                        }
                    }
                }
                let i = self.edges.len() as u32;
                self.edges.push(EdgeRec {
                    v1,
                    v2,
                    flags,
                    layer_slot: layer,
                });
                Ok(Slot::obj(Cls::Edge, i))
            }
            Cls::Curve => {
                self.preamble()?;
                self.c.skip(1)?;
                let _n = self.c.u32()?;
                Ok(Slot::obj(Cls::Curve, NONE_IDX))
            }
            Cls::ArcCurve => {
                // Schema 3 only; there is no pre-2015 arc in this corpus to
                // validate an older layout against, and guessing one would
                // silently bend geometry.
                self.preamble()?;
                self.c.skip(5)?;
                let _p = self.c.f64s::<14>()?;
                Ok(Slot::obj(Cls::ArcCurve, NONE_IDX))
            }
            Cls::EdgeUse => {
                self.preamble()?;
                let (_, edge_entry) = self
                    .read_object(Some(Cls::Edge))?
                    .ok_or(Error::Malformed("edge-use with no edge"))?;
                if !edge_entry.is(Cls::Edge) {
                    return Err(Error::Malformed("edge-use points at a non-edge"));
                }
                let sense = self.c.u8()?;
                // The parent-loop back-ref is the alignment oracle: it must
                // equal the loop we are inside. Read as a raw file index -
                // after annotations the claimed index can legitimately sit
                // ahead of our numbering (burned store-map slots), which is a
                // correction signal rather than a mis-parse.
                let p0 = self.c.at();
                let tag = self.c.u16()?;
                let ps = if tag == 0x7fff {
                    let big = self.c.u32()?;
                    if big & 0x8000_0000 != 0 {
                        return Err(Error::Malformed("edge-use parent is a new object"));
                    }
                    Some(big)
                } else if tag == 0xffff || tag & 0x8000 != 0 {
                    return Err(Error::Malformed("edge-use parent is a new object"));
                } else if tag == 0 {
                    None
                } else {
                    Some(tag as u32)
                };
                let expect = match self.cur_loop {
                    Some(l) => Some(
                        l.checked_add(self.cum_delta)
                            .ok_or(Error::Malformed("burned slot band out of range"))?,
                    ),
                    None => None,
                };
                if ps != expect {
                    let delta = match (ps, expect) {
                        (Some(p), Some(e)) if p > e => p - e,
                        _ => 0,
                    };
                    if delta > 0 && delta <= 4096 && self.annot_watermark.is_some() {
                        self.register_burn(delta)?;
                    } else {
                        self.c.seek(p0)?;
                        return Err(Error::Malformed("edge-use parent is not the current loop"));
                    }
                }
                self.loop_uses.push(LoopUse {
                    edge: edge_entry.idx,
                    reversed: sense != 0,
                });
                Ok(Slot::obj(Cls::EdgeUse, NONE_IDX))
            }
            Cls::Loop => {
                let prev = self.cur_loop;
                self.cur_loop = Some(slot);
                self.preamble()?;
                self.c.skip(2)?; // two flag bytes
                let start = self.loop_uses.len() as u32;
                let r = (|| -> Result<()> {
                    loop {
                        if self.c.peek_u16()? == 0 {
                            self.c.skip(2)?;
                            return Ok(());
                        }
                        self.read_object(Some(Cls::EdgeUse))?;
                    }
                })();
                self.cur_loop = prev;
                r?;
                let i = self.loops.len() as u32;
                let len = self.loop_uses.len() as u32 - start;
                self.loops.push(LoopRec { start, len });
                Ok(Slot::obj(Cls::Loop, i))
            }
            Cls::Face => {
                let attrs = self.preamble()?;
                let ftc = match attrs {
                    Some((_, e)) if e.is(Cls::AttrContainer) => e.idx,
                    _ => NONE_IDX,
                };
                if ftc != NONE_IDX {
                    self.uv_matrices += 1;
                }
                let (mat, flags, layer, hidden) = self.drawbase()?;
                let plane = self.c.f64s::<4>()?;
                let nloops = self.c.u32()?;
                if nloops > 10_000 {
                    return Err(Error::Malformed("implausible loop count"));
                }
                let lstart = self.face_loops.len() as u32;
                for _ in 0..nloops {
                    match self.read_object(Some(Cls::Loop))? {
                        Some((_, e)) if e.is(Cls::Loop) => self.face_loops.push(e.idx),
                        _ => {}
                    }
                }
                let llen = self.face_loops.len() as u32 - lstart;
                self.inner_loops += llen.saturating_sub(1) as u64;
                let back_mat = self.c.u16()? as u32;
                let i = self.faces.len() as u32;
                self.faces.push(FaceRec {
                    normal: [plane[0], plane[1], plane[2]],
                    plane_d: plane[3],
                    back_mat_slot: back_mat,
                    flags,
                    loop_start: lstart,
                    loop_len: llen,
                    mat_slot: mat,
                    layer_slot: layer,
                    hidden,
                    ftc,
                });
                Ok(Slot::obj(Cls::Face, i))
            }
            Cls::AttrContainer => {
                self.preamble()?;
                // A face's texture mapping is a *child of its attribute
                // container*, not a field of the face, so the container's own
                // slot has to carry it out: the face reads its container
                // through `preamble`, which is finished walking the children
                // by the time the face's own fields are read. Carrying it on
                // the slot rather than in a scratch field is what makes a
                // back-reference to an already-walked container deliver the
                // same mapping - the one thing a scratch field cannot do.
                //
                // This is the same defect openskp's C++ port had: the
                // container was read, the cursor advanced correctly, and the
                // result thrown away, so every legacy face fell back to the
                // default projection even where the author had positioned a
                // texture.
                let mut ftc = NONE_IDX;
                loop {
                    if self.c.peek_u16()? == 0 {
                        self.c.skip(2)?;
                        break;
                    }
                    if let Some((_, e)) = self.read_object(Some(Cls::AttrNamed))? {
                        if e.is(Cls::Ftc) && e.idx != NONE_IDX {
                            ftc = e.idx;
                        }
                    }
                }
                Ok(Slot::obj(Cls::AttrContainer, ftc))
            }
            Cls::AttrNamed => {
                self.preamble()?;
                self.c.skip(4)?;
                let _dict = self.c.utf16()?;
                loop {
                    let key = self.c.utf16()?;
                    if key.is_empty() {
                        break;
                    }
                    let t = self.c.u8()?;
                    self.attr_value(t, 0)?;
                }
                // A v7+ addition. Every file in this corpus is 13 or later,
                // so it is unconditional here; a real SketchUp 6 file ends at
                // the empty-key terminator and reading this would eat four
                // bytes of whatever follows.
                if self.ver >= 7 {
                    let _ = self.c.u32()?;
                }
                Ok(Slot::obj(Cls::AttrNamed, NONE_IDX))
            }
            Cls::Layer => self.read_layer(slot),
            Cls::Material => self.read_material(),
            Cls::Dib => {
                // The one record this reader refuses to read. On the 517 MB
                // restaurant pack these blobs are 383 MB; line art needs none
                // of them, and the container is uncompressed, so skipping is
                // free. We keep where they are.
                let subtype = self.c.u32()?;
                let len = self.c.u32()? as u64;
                if len > self.c.left() {
                    return Err(Error::Malformed("implausible image length"));
                }
                let at = self.c.at();
                self.c.skip(len)?;
                // Where a JPEG (subtype 1) ends, for the thumbnail's
                // quality word; see `skip_thumbnail_quality`.
                self.jpeg_end = (subtype == 1).then(|| self.c.at());
                let i = self.dibs.len() as u32;
                self.dibs.push(DibRec { offset: at, len });
                Ok(Slot::obj(Cls::Dib, i))
            }
            Cls::Camera => {
                self.c.skip(137)?;
                let _ = self.c.u16()?;
                let _name = self.c.utf16()?;
                self.c.skip(33)?;
                Ok(Slot::obj(Cls::Camera, NONE_IDX))
            }
            Cls::Thumbnail => {
                self.preamble()?;
                self.read_object(Some(Cls::Camera))?;
                self.jpeg_end = None;
                self.read_object(Some(Cls::Dib))?;
                if self.jpeg_end.is_some_and(|e| e == self.c.at()) {
                    self.skip_thumbnail_quality()?;
                }
                Ok(Slot::obj(Cls::Thumbnail, NONE_IDX))
            }
            Cls::Relationship => {
                self.preamble()?;
                self.entity_ref()?;
                self.entity_ref()?;
                Ok(Slot::obj(Cls::Relationship, NONE_IDX))
            }
            Cls::Ftc => {
                // The same information the modern container spells out as
                // `FaceTextureCoords > FaceTextureCoordsFront >
                // FaceSideTextureInfo{Transform,Flags,ProjVec,PushPins}`, with
                // the CArchive wire layer's fixed widths instead of tags: a
                // discarded `u32`, 24 doubles of which the front side is the
                // first 9 and the back side 12..21, then a pin list per side,
                // then the two flag words.
                //
                // The pins are the texture-positioning tool's handles, four
                // doubles each. Nothing renders from them: a four-pin
                // (distorted) mapping is projective, and the 3x3 *is* that
                // projective map - the third column carries it. Measured in
                // `geom.rs`'s module comment on the modern side, and the same
                // holds here.
                self.preamble()?;
                let _ = self.c.u32()?;
                let k = self.c.f64s::<24>()?;
                for _ in 0..2 {
                    let n = self.c.u32()? as u64;
                    if n > 1 << 20 {
                        return Err(Error::Malformed("implausible texture pin count"));
                    }
                    self.c.skip(n * 32)?;
                }
                let ff = self.c.u32()?;
                let bf = self.c.u32()?;
                for (side, flags) in [(0, ff), (1, bf)] {
                    if flags & crate::geom::uv_flag::POSITIONED != 0 {
                        self.uv_sides[side * 2] += 1;
                    }
                    if flags & crate::geom::uv_flag::PROJECTED != 0 {
                        self.uv_sides[side * 2 + 1] += 1;
                    }
                }
                let mut front = [0.0f64; 9];
                front.copy_from_slice(&k[0..9]);
                // The identity is the default projection under the recipe in
                // `geom::UvMap`, so storing it would cost 80 bytes a face to
                // say what `NONE_IDX` says for free. A side that was never
                // positioned is written as the identity - openskp's writer
                // does it and SketchUp's files agree - so this is the common
                // case, not an edge case.
                let idx = if front.iter().all(|v| v.is_finite()) && front != IDENTITY_3X3 {
                    let i = self.ftcs.len() as u32;
                    self.ftcs.push(FtcRec {
                        front,
                        front_flags: ff,
                    });
                    i
                } else {
                    NONE_IDX
                };
                Ok(Slot::obj(Cls::Ftc, idx))
            }
            Cls::ConstructionLine => {
                self.preamble()?;
                let (_, _, layer, hidden) = self.drawbase()?;
                let point = self.c.f64s::<3>()?;
                let direction = self.c.f64s::<3>()?;
                // The start and end parameters of a bounded guide line. Our
                // guides are infinite lines, so they are not kept.
                let _params = self.c.f64s::<2>()?;
                if hidden {
                    self.hidden_guides += 1;
                    self.hidden_guide_items
                        .push(RawGuide::Line { point, direction });
                    self.hidden_guide_layers.push(layer);
                } else {
                    self.guides.push(RawGuide::Line { point, direction });
                    self.guide_layers.push(layer);
                }
                // The trailing block varies by writing *build*, not cleanly
                // by version: 7 bytes on the 2017 calibration corpus, 4 on
                // 2016 and on a real 2018, 0 on another real 2017. Calibrate
                // on the first guide line in the file and cache it.
                let k = match self.cline_tail {
                    Some(k) => k,
                    None => {
                        // Try the writer's usual length first, then the
                        // others. The strong pass requires an unambiguous
                        // next tag; the weak pass accepts a null, which is
                        // what a guide line that ends a definition is
                        // followed by - and there 4 is the right guess,
                        // because preferring 7 would swallow three bytes of
                        // the definition tail and then cache that mistake.
                        let default = if self.ver == 17 { 7u64 } else { 4 };
                        let mut found = None;
                        for cand in [default, 0, 4, 7] {
                            if self.strict_tag_at(self.c.at() + cand, false)? {
                                found = Some(cand);
                                break;
                            }
                        }
                        if found.is_none() {
                            for cand in [4u64, 0, 7] {
                                if self.strict_tag_at(self.c.at() + cand, true)? {
                                    found = Some(cand);
                                    break;
                                }
                            }
                        }
                        let k = found.unwrap_or(4);
                        self.cline_tail = Some(k);
                        k
                    }
                };
                self.c.skip(k)?;
                Ok(Slot::obj(Cls::ConstructionLine, NONE_IDX))
            }
            Cls::ConstructionPoint => {
                self.preamble()?;
                let (_, _, layer, hidden) = self.drawbase()?;
                let position = self.c.f64s::<3>()?;
                // The slot keeps the guide's index so a text label attached to this point can
                // find it (a hidden point is not kept).
                let mut idx = NONE_IDX;
                if hidden {
                    self.hidden_guides += 1;
                    self.hidden_guide_items.push(RawGuide::Point { position });
                    self.hidden_guide_layers.push(layer);
                } else {
                    idx = self.guides.len() as u32;
                    self.guides.push(RawGuide::Point { position });
                    self.guide_layers.push(layer);
                }
                let _reserved = self.c.f64s::<3>()?;
                self.c.skip(1)?;
                Ok(Slot::obj(Cls::ConstructionPoint, idx))
            }
            Cls::SectionPlane => {
                self.preamble()?;
                self.drawbase()?;
                // An optional object pointer sits before the plane. A real
                // plane starts with a unit-normal component, so |x| <= 1
                // tells the two apart; a tag word does not decode as one.
                let first = {
                    let b = self.c.peek_slice(8)?;
                    let mut a = [0u8; 8];
                    a.copy_from_slice(b);
                    f64::from_le_bytes(a)
                };
                if !(first.abs() <= 1.0001) {
                    self.read_object(None)?;
                }
                let _plane = self.c.f64s::<4>()?;
                if self.marker_at(self.c.at())? {
                    let _name = self.c.utf16()?;
                    let _label = self.c.utf16()?;
                }
                Ok(Slot::obj(Cls::SectionPlane, NONE_IDX))
            }
            Cls::SkFont => {
                self.read_object(Some(Cls::AttrContainer))?;
                if self.wide_entity {
                    let mask = self.c.u8()?;
                    self.c.skip(mask.count_ones() as u64)?;
                }
                let _name = self.c.utf16()?;
                self.c.skip(15)?;
                Ok(Slot::obj(Cls::SkFont, NONE_IDX))
            }
            Cls::DimLinear => {
                self.preamble()?;
                let (_, _, layer, hidden) = self.drawbase()?;
                let text = self.c.utf16()?;
                self.read_object(Some(Cls::SkFont))?;
                // 161 fixed bytes plus the refs, measured over 42 dimensions
                // in 23 legacy files. The refs are what make it variable: a
                // 2-byte tag in a small archive, 6 once the store map passes
                // 0x7ffe objects - so a fixed-size skip walks off exactly on
                // the large models, and silently truncates the root list.
                //
                // Each end is `5 bytes, u32 kind, u32 4, point (3 x f64)` and a
                // connection (an entity ref, then `connection_paths`); a point
                // attached to an entity holds zero and the ref names the entity.
                let head = self.c.fixed::<37>()?;
                let a_ref = self.entity_ref()?;
                let a_paths = self.connection_paths()?;
                let mid = self.c.fixed::<32>()?;
                let b_ref = self.entity_ref()?;
                let b_paths = self.connection_paths()?;
                let tail = self.c.fixed::<72>()?;
                let end_point = |b: &[u8], r: Option<u32>| match r {
                    Some(r) => AnnotPoint::Entity(r),
                    None => AnnotPoint::Free([f64_at(b, 0), f64_at(b, 8), f64_at(b, 16)]),
                };
                let body = if a_paths != 0 || b_paths != 0 {
                    // Through an instance path: not read, as for 2021+ files.
                    PendingBody::Unread
                } else {
                    PendingBody::Linear {
                        start: end_point(&head[13..], a_ref),
                        end: end_point(&mid[8..], b_ref),
                        normal: [f64_at(&tail, 0), f64_at(&tail, 8), f64_at(&tail, 16)],
                        x_axis: [f64_at(&tail, 24), f64_at(&tail, 32), f64_at(&tail, 40)],
                        dim_type: u32::from_le_bytes([tail[48], tail[49], tail[50], tail[51]]),
                        y_pos: f64_at(&tail, 52),
                        custom_text: !text.is_empty(),
                    }
                };
                self.pending_annots.push(PendingAnnot {
                    hidden,
                    layer_slot: layer,
                    body,
                });
                Ok(Slot::obj(Cls::DimLinear, NONE_IDX))
            }
            Cls::Text => self.read_text(),
            Cls::Definition => self.read_definition(slot),
            Cls::Instance | Cls::Group => self.read_instance(cls),
            Cls::Image => {
                self.preamble()?;
                let (_m, _f, layer, hidden) = self.drawbase()?;
                let def = self.read_object(None)?.map(|(s, _)| s).unwrap_or(NONE_IDX);
                let xf12 = self.c.f64s::<12>()?;
                let _one = self.c.f64()?;
                let _path = self.c.utf16()?;
                self.c.skip(16)?;
                let mut xf = [0.0f64; 13];
                xf[..12].copy_from_slice(&xf12);
                xf[12] = 1.0;
                if def != NONE_IDX {
                    self.image_defs.insert(def);
                }
                let parent = self.def_stack.last().copied().unwrap_or(ROOT_DEFINITION);
                let i = self.instances.len() as u32;
                self.instances.push(InstRec {
                    parent,
                    def_slot: def,
                    xf,
                    mat_slot: 0,
                    layer_slot: layer,
                    name: String::new(),
                    is_group: false,
                    is_image: true,
                    hidden,
                });
                Ok(Slot::obj(Cls::Image, i))
            }
            Cls::ViewPage => {
                // Only ever met through `scan_pages`, which decodes it with
                // its own cursor; reaching one here means the walk ran into
                // the page list, which it should not.
                Err(Error::Malformed(
                    "unexpected page record in the entity walk",
                ))
            }
            Cls::Unknown => Err(Error::Malformed("no reader for this class")),
        }
    }

    /// An attribute value. Types are SketchUp's own; `0x0b` is an array, so
    /// this recurses - bounded, because a file controls the nesting.
    fn attr_value(&mut self, t: u8, depth: u32) -> Result<()> {
        if depth > 8 {
            return Err(Error::Malformed("attribute array nested too deep"));
        }
        match t {
            0x00 => {}
            0x04 | 0x09 => {
                let _ = self.c.u32()?;
            }
            0x06 | 0x0c => {
                let _ = self.c.f64()?;
            }
            0x07 => {
                let _ = self.c.u8()?;
            }
            0x0a => {
                let _ = self.c.utf16()?;
            }
            0x0b => {
                let n = self.c.u32()?;
                if n > 100_000 {
                    return Err(Error::Malformed("implausible attribute array count"));
                }
                for _ in 0..n {
                    let t2 = self.c.u8()?;
                    self.attr_value(t2, depth + 1)?;
                }
            }
            0x11 | 0x12 => {
                let _ = self.c.f64s::<3>()?;
            }
            _ => return Err(Error::Malformed("unknown attribute value type")),
        }
        Ok(())
    }

    /// A definition's JPEG thumbnail, as SketchUp 2024 writes it into a
    /// 2017-2020 file, is followed by a `u32` its length does not cover: the
    /// JPEG quality, 70 on every such file in the corpus. PNG thumbnails
    /// (subtype 4) have none, and nothing shows that a 2017-2020 SketchUp
    /// writes one. Left unread, the word is taken for the next object - a
    /// back-reference and a null - so a definition list counted them as
    /// definitions and read the root count four bytes early, which lost root
    /// entities (truck, bikes) or failed the file (barstools).
    ///
    /// Skipped only when it is a plausible quality and what can follow the
    /// end of a definition (which the thumbnail ends) reads well after the
    /// word and not without it: another definition, the root count and a
    /// first root entity, or an instance's transform and name (a definition
    /// written inline by the instance that first places it). Otherwise the
    /// cursor stays: a JPEG without the word reads as before. A texture's
    /// image is not handled here; `texture_block` has its own check.
    ///
    /// No check asks whether a definition starts at the word itself: a word
    /// of at most 100 begins with a `u16` of at most 100, which is neither
    /// a class-ref (bit 15 set, or the 0x7fff escape) nor a declaration
    /// (0xffff), the only forms `definition_at` accepts.
    fn skip_thumbnail_quality(&mut self) -> Result<()> {
        let at = self.c.at();
        let Ok(q) = self.c.u32_at(at) else {
            return Ok(());
        };
        if q > 100 {
            return Ok(());
        }
        let skip = if self.definition_at(at + 4) {
            true
        } else if self.bytes_eq_at(at + 4 + 104, &[0xff, 0xfe, 0xff]) {
            // 13 doubles, then the instance name's string record.
            true
        } else if self.bytes_eq_at(at + 104, &[0xff, 0xfe, 0xff]) {
            false
        } else {
            let with = self.c.u32_at(at + 4).is_ok_and(|n| n <= 5_000_000)
                && self.strict_tag_at(at + 8, false)?;
            let without = self.strict_tag_at(at + 4, false)?;
            with && !without
        };
        if skip {
            self.c.skip(4)?;
        }
        Ok(())
    }

    /// Reads a connection's second entity ref and its two path lists; returns how many
    /// instance-path items (and a second ref, which is one more) it held. Zero for every
    /// connection to a vertex or guide point of the dimension's own definition.
    fn connection_paths(&mut self) -> Result<u32> {
        let mut found = u32::from(self.entity_ref()?.is_some());
        for _ in 0..2 {
            let n = self.c.u32()?;
            if n > 1000 {
                return Err(Error::Malformed("implausible dimension path length"));
            }
            found += n;
            for _ in 0..n {
                // Usually a reference, but MFC writes an object in full the
                // first time anything points at it - and a dimension
                // serialized before the group it anchors in carries that whole
                // group right here.
                let tag = self.c.peek_u16()?;
                let is_new = tag == 0xffff
                    || (tag != 0x7fff && tag & 0x8000 != 0)
                    || (tag == 0x7fff && self.c.u32_at(self.c.at() + 2)? & 0x8000_0000 != 0);
                if is_new {
                    self.read_object(None)?;
                } else {
                    self.entity_ref()?;
                }
            }
        }
        Ok(found)
    }

    fn vertex_ref(&mut self) -> Result<u32> {
        match self.read_object(Some(Cls::Vertex))? {
            Some((_, e)) if e.is(Cls::Vertex) => Ok(e.idx),
            _ => Ok(NONE_IDX),
        }
    }

    fn marker_at(&mut self, at: u64) -> Result<bool> {
        let save = self.c.at();
        let r = (|| -> Result<bool> {
            self.c.seek(at)?;
            let b = self.c.peek_slice(3)?;
            Ok(b == [0xff, 0xfe, 0xff])
        })();
        self.c.seek(save)?;
        Ok(r.unwrap_or(false))
    }

    /// True when the word at `at` starts an object read in one of the
    /// unambiguous forms: a null (optional), an escape, a class declaration,
    /// or a class-ref to a class already known. Plain back-refs are excluded
    /// on purpose - any two bytes below 0x8000 would qualify, which is the
    /// ambiguity this check exists to avoid.
    fn strict_tag_at(&mut self, at: u64, allow_null: bool) -> Result<bool> {
        let Ok(t) = self.c.u16_at(at) else {
            return Ok(false);
        };
        if t == 0 {
            return Ok(allow_null);
        }
        if t == 0x7fff || t == 0xffff {
            return Ok(true);
        }
        if t & 0x8000 != 0 {
            return Ok(self.entry((t & 0x7fff) as u32).kind == K_CLASS);
        }
        Ok(false)
    }

    // -- small absolute-offset helpers ------------------------------------

    fn byte_at(&mut self, at: u64) -> Option<u8> {
        let save = self.c.at();
        let r = self
            .c
            .seek(at)
            .ok()
            .and_then(|_| self.c.peek_slice(1).ok().map(|b| b[0]));
        let _ = self.c.seek(save);
        r
    }

    fn bytes_eq_at(&mut self, at: u64, needle: &[u8]) -> bool {
        let save = self.c.at();
        let r = (|| -> Option<bool> {
            self.c.seek(at).ok()?;
            let b = self.c.peek_slice(needle.len()).ok()?;
            Some(b == needle)
        })()
        .unwrap_or(false);
        let _ = self.c.seek(save);
        r
    }

    fn at_marker(&mut self) -> bool {
        self.c
            .peek_slice(3)
            .map(|b| b == [0xff, 0xfe, 0xff])
            .unwrap_or(false)
    }

    /// `0x8000|slot`, or the big-tag escape for slots at or past 0x7fff.
    ///
    /// 0x7fff is deliberately excluded from the short form: `0x8000|0x7fff`
    /// is 0xffff, which the decoder reads as a class *declaration* before it
    /// ever tests the class-ref bit, so no encoder can emit the short form
    /// for exactly that slot.
    fn is_class_ref_at(&mut self, at: u64, slot: u32) -> bool {
        if slot < 0x7fff {
            self.c
                .u16_at(at)
                .map(|t| t == 0x8000 | slot as u16)
                .unwrap_or(false)
        } else {
            self.c.u16_at(at).map(|t| t == 0x7fff).unwrap_or(false)
                && self
                    .c
                    .u32_at(at + 2)
                    .map(|v| v == 0x8000_0000 | slot)
                    .unwrap_or(false)
        }
    }

    // -- CLayer -----------------------------------------------------------

    fn read_layer(&mut self, slot: u32) -> Result<Slot> {
        self.preamble()?;
        let name = self.c.utf16()?;
        // Flag bytes: three on the schema-2 era, four later. Their length is
        // not stated anywhere, so read up to eight and stop at the next
        // string record, which is the layer's internal name.
        let mut first = 0u8;
        let mut n = 0;
        while n < 8 && !self.at_marker() {
            let b = self.c.u8()?;
            if n == 0 {
                first = b;
            }
            n += 1;
        }
        let _internal = self.c.utf16()?;
        let flags = self.c.u16()?;
        let rgba;
        if flags & 0x00ff != 0 {
            // Colour-by-layer with a *textured* material: the layer embeds
            // the same block a CMaterial carries. The low byte of the flag
            // word is the tell; a plain colour layer has zero there, while
            // its high byte carries something unrelated, so the word as a
            // whole is non-zero either way.
            let t = self.texture_block()?;
            self.c.skip(4)?;
            rgba = t.rgba;
        } else {
            rgba = self.c.fixed::<4>()?;
            let _ = self.c.utf16()?;
            self.c.skip(self.layer_tail)?;
            self.skip_layer_colour_ext()?;
            self.skip_layer_parent_ref(slot)?;
        }
        let hidden = first != 0;
        // One record per name. Every definition re-serializes its own
        // `CLayer` for the same tag, so without this a file with 600
        // definitions would report 600 layers.
        let idx = match self.layer_by_name.get(&name) {
            Some(i) => *i,
            None => {
                let i = self.layers.len() as u32;
                self.layer_by_name.insert(name.clone(), i);
                self.layers.push(LayerRec { name, hidden, rgba });
                i
            }
        };
        Ok(Slot::obj(Cls::Layer, idx))
    }

    /// Some zero-material files write twelve zero bytes and a `u32` after a
    /// colour layer's tail, then another `CLayer`. Skip those sixteen bytes
    /// only when a layer really does follow; a definition-list back-ref must
    /// be left alone.
    fn skip_layer_colour_ext(&mut self) -> Result<()> {
        const PAD: usize = 12;
        const EXT: u64 = 16;
        if self.c.left() < EXT + 2 {
            return Ok(());
        }
        let zeros = {
            let b = self.c.peek_slice(PAD)?;
            b.iter().all(|x| *x == 0)
        };
        if !zeros {
            return Ok(());
        }
        let at = self.c.at() + EXT;
        if self.layer_record_at(at, true) {
            self.c.skip(EXT)?;
        }
        Ok(())
    }

    fn layer_record_at(&mut self, at: u64, unmatched_class_ref: bool) -> bool {
        let Ok(tag) = self.c.u16_at(at) else {
            return false;
        };
        if tag == 0xffff
            && self.c.u16_at(at + 4).map(|n| n == 6).unwrap_or(false)
            && self.bytes_eq_at(at + 6, b"CLayer")
        {
            return true;
        }
        let ls = self.class_slot[Cls::Layer as usize];
        if ls != NONE_IDX && self.is_class_ref_at(at, ls) {
            return true;
        }
        unmatched_class_ref && tag & 0x8000 != 0 && tag != 0xffff
    }

    /// A template `Layer0` written after a custom tag carries an extra
    /// `<parent u16>` before the layer list's own tail. Consume it only when
    /// the file cannot be read without doing so.
    ///
    /// This is the guard that costs the Python and C++ readers a file. Their
    /// rule is "consume the parent when both of the next two words name
    /// `CLayer` objects", and it is not sufficient: what normally follows the
    /// model's last layer is `<active-layer back-ref u16><u32 definition
    /// count>`, so in a file with seventeen tags at slots 32..48, a
    /// definition count of 39 *is* a valid layer slot. Both readers then eat
    /// two bytes, read the count two bytes late as 0xFFFF0000 and give up -
    /// "implausible definition count" / "invalid definition count" on
    /// `legacy_v15_woodworking_clamprack_15scenes_inlinestyle.skp`.
    ///
    /// The count is what tells the two layouts apart, so read it both ways
    /// and keep the parent only when the ordinary reading is impossible.
    fn skip_layer_parent_ref(&mut self, self_slot: u32) -> Result<()> {
        if self.c.left() < 8 {
            return Ok(());
        }
        let here = self.c.at();
        let a = self.c.u16_at(here)?;
        let b = self.c.u16_at(here + 2)?;
        if a == 0 || a & 0x8000 != 0 || a == 0x7fff || a as u32 == self_slot {
            return Ok(());
        }
        if b == 0 || b & 0x8000 != 0 || b == 0x7fff {
            return Ok(());
        }
        if !(self.entry(a as u32).is(Cls::Layer) && self.entry(b as u32).is(Cls::Layer)) {
            return Ok(());
        }
        // `<anchor u16><u32 count>` with no parent, versus
        // `<parent u16><anchor u16><u32 count>` with one.
        const CEILING: u32 = 5_000_000;
        let without = self.c.u32_at(here + 2)?;
        let with = self.c.u32_at(here + 4)?;
        if without <= CEILING || with > CEILING {
            return Ok(());
        }
        self.c.skip(2)?;
        Ok(())
    }

    // -- CMaterial / textures --------------------------------------------

    fn read_material(&mut self) -> Result<Slot> {
        self.preamble()?;
        let name = self.c.utf16()?;
        let texflag = self.c.u16()?;
        let rec = if texflag == 0 {
            let rgba = self.c.fixed::<4>()?;
            let _path = self.c.utf16()?;
            self.c.skip(8)?;
            let stored = self.c.f64()?;
            let use_op = self.c.u8()?;
            MaterialRec {
                name,
                rgba,
                opacity: solid_opacity(stored, use_op),
                texture: None,
                colorized: false,
            }
        } else {
            let t = self.texture_block()?;
            let texture = self.dibs.get(t.dib as usize).map(|d| TextureRef {
                name: if t.file.is_empty() {
                    name.clone()
                } else {
                    t.file.clone()
                },
                location: TextureLocation::Inline {
                    offset: d.offset,
                    len: d.len,
                },
                tile_mm: [t.w * INCH_TO_MM, t.h * INCH_TO_MM],
            });
            MaterialRec {
                name,
                rgba: t.rgba,
                opacity: solid_opacity(t.opacity, t.use_opacity),
                texture,
                colorized: t.colorized,
            }
        };
        let i = self.materials.len() as u32;
        self.any_textured |= rec.texture.is_some();
        self.materials.push(rec);
        Ok(Slot::obj(Cls::Material, i))
    }

    /// The textured payload: an embedded `CDib`, the applied tile size, the
    /// source file name, the average colour, and the opacity. Shared verbatim
    /// between a textured `CMaterial` and a colour-by-layer `CLayer`.
    fn texture_block(&mut self) -> Result<TexBlock> {
        self.c.skip(if self.wide_entity { 2 } else { 1 })?;
        let dib = match self.read_object(Some(Cls::Dib))? {
            Some((_, e)) if e.is(Cls::Dib) => e.idx,
            _ => return Err(Error::Malformed("texture object is not an image")),
        };
        // An optional `u32` sits between the image and the two applied-size
        // doubles. The distance to the next string record says which layout
        // this is: 16 bytes means no filler, 20 means one.
        let here = self.c.at();
        let marker = self
            .c
            .find(&[0xff, 0xfe, 0xff], 28)?
            .ok_or(Error::Malformed("texture size block misaligned"))?;
        match marker - here {
            20 => {
                let _ = self.c.u32()?;
            }
            16 => {}
            _ => return Err(Error::Malformed("texture size block misaligned")),
        }
        let w = self.c.f64()?;
        let h = self.c.f64()?;
        let file = self.c.utf16()?;
        let avg = self.c.fixed::<9>()?; // RGBA + 00 + RGBA, the colour twice
        let _ = self.c.utf16()?;
        let blob = self.c.fixed::<8>()?;
        let opacity = self.c.f64()?;
        let use_opacity = self.c.u8()?;
        Ok(TexBlock {
            // The fourth byte is not an alpha: it is 0 on a plain textured
            // material and 0xff on a colorized one (see `colorized`). The
            // modern path gives every material an opaque colour, with the
            // opacity separate.
            rgba: [avg[0], avg[1], avg[2], 0xff],
            opacity,
            use_opacity,
            dib,
            w,
            h,
            file,
            colorized: blob[4] != 0 || avg[3] == 0xff,
        })
    }

    // -- CText ------------------------------------------------------------

    /// The fields of a `CText` middle that runs from `start` to the string marker at `marker`
    /// (123 bytes in every sample, more when a connection holds paths):
    ///
    /// ```text
    /// 16 bytes, u32 kind, u32 4, anchor 3 x f64, connection (entity ref, second ref,
    /// u32 n1, u32 n2), leader vector 3 x f64, text direction 3 x f64, u32 leader type,
    /// the 11-byte block
    /// ```
    ///
    /// The anchor is zero when the label is attached to an entity (the ref names it). `None`
    /// when the middle is not this layout (a shorter record, a connection with paths): the
    /// label is then counted as unread. The cursor is left where it was.
    fn text_fields(&mut self, start: u64, marker: u64) -> Option<(AnnotPoint, [f64; 3], u32)> {
        const TAIL: u64 = 24 + 24 + 4 + 11;
        let save = self.c.at();
        let r = (|| -> Result<Option<(AnnotPoint, [f64; 3], u32)>> {
            if marker < start + 24 + 24 + 12 + TAIL {
                return Ok(None);
            }
            self.c.seek(start + 24)?;
            let a = self.c.f64s::<3>()?;
            let r1 = self.entity_ref()?;
            let r2 = self.entity_ref()?;
            let (n1, n2) = (self.c.u32()?, self.c.u32()?);
            if r2.is_some() || n1 != 0 || n2 != 0 || self.c.at() + TAIL != marker {
                return Ok(None);
            }
            let vector = self.c.f64s::<3>()?;
            let _direction = self.c.f64s::<3>()?;
            let kind = self.c.u32()?;
            let anchor = match r1 {
                Some(r) => AnnotPoint::Entity(r),
                None => AnnotPoint::Free(a),
            };
            Ok(Some((anchor, vector, kind)))
        })();
        let _ = self.c.seek(save);
        r.ok().flatten()
    }

    fn read_text(&mut self) -> Result<Slot> {
        self.preamble()?;
        let (_, _, layer, hidden) = self.drawbase()?;
        self.read_object(Some(Cls::SkFont))?;
        // The middle of this record is variant-length, delimited by an
        // eleven-byte block `01 00 00 00 ?? 00 <u32 arrow> 01` immediately
        // before the text string. Search for the string marker and test the
        // bytes behind it.
        let start = self.c.at();
        let mut from = start;
        let idx = loop {
            let save = self.c.at();
            self.c.seek(from)?;
            let found = self.c.find(&[0xff, 0xfe, 0xff], start + 512 - from)?;
            self.c.seek(save)?;
            let Some(idx) = found else {
                return Err(Error::Malformed("text delimiter not found"));
            };
            if idx >= start + 11 {
                let save = self.c.at();
                self.c.seek(idx)?;
                let blk = self.c.behind(11).map(|b| {
                    let mut a = [0u8; 11];
                    a.copy_from_slice(b);
                    a
                });
                self.c.seek(save)?;
                if let Ok(blk) = blk {
                    let arrow = u32::from_le_bytes([blk[6], blk[7], blk[8], blk[9]]);
                    if blk[0..4] == [1, 0, 0, 0] && arrow <= 8 && blk[10] == 1 {
                        break idx;
                    }
                }
            }
            from = idx + 3;
        };
        let fields = self.text_fields(start, idx);
        self.c.seek(idx)?;
        let text = self.c.utf16()?;
        self.pending_annots.push(PendingAnnot {
            hidden,
            layer_slot: layer,
            body: match fields {
                Some((anchor, vector, kind)) => PendingBody::Leader {
                    anchor,
                    vector,
                    kind,
                    text,
                },
                None => PendingBody::Unread,
            },
        });
        self.c.skip(5)?;
        // Optional leader-attachment refs. Only the escaped six-byte form is
        // recognisable without risk: a two-byte back-ref here would be
        // indistinguishable from the next list item's own tag.
        while self.c.left() >= 6 {
            let t = self.c.peek_u16()?;
            if t != 0x7fff {
                break;
            }
            let v = self.c.u32_at(self.c.at() + 2)?;
            if v & 0x8000_0000 != 0 {
                break;
            }
            self.c.skip(6)?;
        }
        Ok(Slot::obj(Cls::Text, NONE_IDX))
    }

    // -- instances --------------------------------------------------------

    fn read_instance(&mut self, cls: Cls) -> Result<Slot> {
        self.preamble()?;
        let (mat, _flags, layer, hidden) = self.drawbase()?;
        let (def_slot, def_entry) = self
            .read_object(Some(Cls::Definition))?
            .ok_or(Error::Malformed("instance with no definition"))?;
        if !def_entry.is(Cls::Definition) {
            return Err(Error::Malformed(
                "instance definition ref is not a definition",
            ));
        }
        let xf = self.c.f64s::<13>()?;
        let name = self.c.utf16()?;
        // The trailing instance GUID is a pre-2014 thing, full stop, for both
        // `CComponentInstance` and `CGroup`. The class's own schema number
        // looked like the gate and is not: a 2013 file reports schema 6 for
        // an instance that carries no GUID, and `CGroup` reports schema 1 on
        // every version from 3 to 2025. Reading sixteen bytes that belong to
        // the next sibling's tag does not raise - it silently truncates the
        // rest of the list.
        if self.ver >= 14 {
            self.c.skip(16)?;
        }
        let parent = self.def_stack.last().copied().unwrap_or(ROOT_DEFINITION);
        let i = self.instances.len() as u32;
        self.instances.push(InstRec {
            parent,
            def_slot,
            xf,
            mat_slot: mat,
            layer_slot: layer,
            name,
            is_group: cls == Cls::Group,
            is_image: false,
            hidden,
        });
        Ok(Slot::obj(cls, i))
    }

    // -- CComponentDefinition ---------------------------------------------

    fn read_definition(&mut self, slot: u32) -> Result<Slot> {
        self.preamble()?;
        // Undecoded base block. 22 bytes once `CEntity` reaches schema 5,
        // 20 before that - the same gate as the persistent-id mask, because
        // both are `CEntity`'s own serialization changing.
        self.c.skip(if self.wide_entity { 22 } else { 20 })?;
        let nlayers = self.c.u32()?;
        if nlayers > 10_000 {
            return Err(Error::Malformed("implausible definition layer count"));
        }
        // The count is real layers; some writers interleave null separators.
        let mut got = 0;
        while got < nlayers {
            if self.c.peek_u16()? == 0 {
                self.c.skip(2)?;
                continue;
            }
            self.read_object(Some(Cls::Layer))?;
            got += 1;
        }
        let decl = self.c.u16()?;
        if decl == 0x7fff {
            let _ = self.c.u32()?;
        }
        // SketchUp 2020's filler again (see `walk`), here before the
        // undecoded `u32` that precedes the count. When it is not exactly
        // there, openskp's search runs before the two words are read: a
        // filler of another length makes them read as a plausible count
        // (0xfffe on this corpus), so waiting for an implausible one is not
        // enough.
        let mut found = None;
        if self.v20 {
            // The filler, then the `u32` (0 on every file measured). Checking
            // the word too keeps a shorter filler, whose zeros run into that
            // word, from passing for this one.
            let here = self.c.at();
            if self.bytes_eq_at(here, &V20_FILLER) && self.bytes_eq_at(here + 16, &[0; 4]) {
                self.c.skip(V20_FILLER.len() as u64)?;
            } else {
                found = self.count_after_v20_filler(self.c.at(), 5_000_000, Follows::ListItem)?;
            }
        }
        let count = match found {
            Some(n) => n,
            None => {
                let _ = self.c.u32()?;
                let count_pos = self.c.at();
                let mut count = self.c.u32()?;
                if self.v20 && (count > 5_000_000 || count == 0) {
                    if let Some(n) =
                        self.count_after_v20_filler(count_pos, 5_000_000, Follows::ListItem)?
                    {
                        count = n;
                    }
                }
                count
            }
        };
        if count > 5_000_000 {
            return Err(Error::Malformed("implausible definition entity count"));
        }

        let mark = self.mark();
        self.def_stack.push(slot);
        let res = self.definition_body(slot, count);
        self.def_stack.pop();
        match res {
            Ok(()) => {
                match self.emit {
                    Emit::Batches => self.emit_definition(slot, &mark),
                    Emit::Topology => {
                        let geom = self.emit_topology(slot, &mark);
                        if let (Some(geom), Some(info)) = (geom, self.defs.last()) {
                            self.topo_defs.push(Definition {
                                pid: slot as u64,
                                name: info.name.clone(),
                                guid: unhex16(&info.guid),
                                always_face_camera: info.always_faces_camera,
                                geom,
                            });
                        }
                    }
                    Emit::Nothing => {}
                }
                self.release(&mark);
                Ok(Slot::obj(Cls::Definition, (self.defs.len() - 1) as u32))
            }
            Err(e) => {
                self.release(&mark);
                Err(e)
            }
        }
    }

    fn definition_body(&mut self, slot: u32, count: u32) -> Result<()> {
        self.read_entity_list(count, Owner::Def)?;
        let nrel_pos = self.c.at();
        let mut nrel = self.c.u32()?;
        if self.v20 && nrel > 100_000 {
            if let Some(n) = self.count_after_v20_filler(nrel_pos, 100_000, Follows::ListItem)? {
                nrel = n;
            }
        }
        if nrel > 100_000 {
            return Err(Error::Malformed("definition list misaligned"));
        }
        for _ in 0..nrel {
            self.read_object(Some(Cls::Relationship))?;
        }
        let _ = self.c.u16()?;
        self.reanchor_on_marker();
        let guid = self.c.fixed::<16>()?;
        let name = self.c.utf16()?;
        let _desc = self.c.utf16()?;
        let _path = self.c.utf16()?;
        let _stamp = self.c.u32()?;
        // Then an undecoded block of roughly 39-47 bytes and the thumbnail
        // object. Anchor on the thumbnail rather than guessing the length.
        let here = self.c.at();
        let thumb_cls = self.class_slot[Cls::Thumbnail as usize];
        let mut tpos = None;
        for off in 0..96u64 {
            let p = here + off;
            if self.bytes_eq_at(p, &[0xff, 0xff])
                && self.bytes_eq_at(p + 4, &[0x0a, 0x00])
                && self.bytes_eq_at(p + 6, b"CThumbnail")
            {
                tpos = Some(p);
                break;
            }
            if thumb_cls != NONE_IDX && self.is_class_ref_at(p, thumb_cls) {
                tpos = Some(p);
                break;
            }
        }
        let Some(tpos) = tpos else {
            return Err(Error::Malformed("definition tail: thumbnail not found"));
        };
        // Component-behaviour flags sit nine bytes before the thumbnail:
        // bit 0 always-faces-camera, bit 1 shadows-face-sun.
        let behavior = if tpos >= here + 9 {
            self.byte_at(tpos - 9).unwrap_or(0)
        } else {
            0
        };
        self.c.seek(tpos)?;
        self.read_object(Some(Cls::Thumbnail))?;
        self.defs.push(DefinitionInfo {
            id: slot,
            name,
            guid: hex16(&guid),
            is_image: false,
            always_faces_camera: behavior & 1 != 0,
        });
        Ok(())
    }

    /// The GUID-plus-name read assumes a fixed prefix width, and some writers
    /// skew it by a byte or two in either direction. Search both ways within
    /// four bytes, nearest first, and leave the cursor alone if nothing
    /// matches - the strict read that follows should raise, not guess.
    fn reanchor_on_marker(&mut self) {
        let here = self.c.at();
        if self.bytes_eq_at(here + 16, &[0xff, 0xfe, 0xff]) {
            return;
        }
        for d in [-1i64, 1, -2, 2, -3, 3, -4, 4] {
            let at = here as i64 + d;
            if at < 0 {
                continue;
            }
            if self.bytes_eq_at(at as u64 + 16, &[0xff, 0xfe, 0xff]) {
                let _ = self.c.seek(at as u64);
                return;
            }
        }
    }

    // -- entity lists -----------------------------------------------------

    fn read_entity_list(&mut self, count: u32, owner: Owner) -> Result<u32> {
        self.burn_credit.push(0);
        let mut n = 0u32;
        let mut stop = None;
        while n < count {
            let p = self.c.at();
            if owner == Owner::Def && self.burn_credit.last().copied().unwrap_or(0) > 0 {
                // Burned store-map indices mean the declared count includes
                // phantom entities the stream never carries. The definition
                // tail's own signature - nrel = 0, then padding, then a
                // 16-byte GUID with a string record at +22 - marks the real
                // end of the list.
                if self.c.u32_at(p).map(|v| v == 0).unwrap_or(false)
                    && self.bytes_eq_at(p + 22, &[0xff, 0xfe, 0xff])
                {
                    break;
                }
            }
            let r = self.read_object(None);
            match r {
                Ok(_) => n += 1,
                Err(e) => {
                    // An over-declared count runs into the document tail; a
                    // burned list overshoots by its phantoms; and a record
                    // this reader cannot decode ends the list rather than the
                    // file. In every case stop here and leave the cursor on
                    // the item that failed, so the tail read either resyncs
                    // or fails loudly.
                    let _ = self.c.seek(p);
                    stop = Some(e);
                    break;
                }
            }
        }
        self.burn_credit.pop();
        if let Some(e) = stop {
            if n < count {
                self.lost += (count - n) as u64;
                let where_ = if owner == Owner::Root {
                    "root"
                } else {
                    "definition"
                };
                self.note(format!(
                    "{where_} entity list stopped at item {n} of {count} at {:#x}: {e}",
                    self.c.at()
                ));
            }
        }
        Ok(n)
    }

    // -- arena marks ------------------------------------------------------

    fn mark(&self) -> Mark {
        Mark {
            verts: self.verts.len() as u32,
            edges: self.edges.len() as u32,
            faces: self.faces.len() as u32,
            loops: self.loops.len() as u32,
            loop_uses: self.loop_uses.len() as u32,
            face_loops: self.face_loops.len() as u32,
            ftcs: self.ftcs.len() as u32,
            guides: self.guides.len() as u32,
            hidden_guide_items: self.hidden_guide_items.len() as u32,
            annots: self.pending_annots.len() as u32,
            instances: self.instances.len() as u32,
            slot: self.next_slot(),
        }
    }

    /// Drop a definition's geometry once its batches are built.
    ///
    /// This is what keeps the resident high-water mark at one definition
    /// rather than at the model: a definition's geometry is referenced only
    /// from inside that definition, and definitions nest strictly, so the
    /// arenas behave as stacks. Any slot that pointed into the freed region is
    /// marked so a stale back-reference reads as unknown instead of as
    /// another definition's vertex.
    fn release(&mut self, m: &Mark) {
        self.verts.truncate(m.verts as usize);
        self.edges.truncate(m.edges as usize);
        self.faces.truncate(m.faces as usize);
        self.loops.truncate(m.loops as usize);
        self.loop_uses.truncate(m.loop_uses as usize);
        self.face_loops.truncate(m.face_loops as usize);
        self.ftcs.truncate(m.ftcs as usize);
        self.guides.truncate(m.guides as usize);
        self.guide_layers.truncate(m.guides as usize);
        self.hidden_guide_items
            .truncate(m.hidden_guide_items as usize);
        self.hidden_guide_layers
            .truncate(m.hidden_guide_items as usize);
        self.pending_annots.truncate(m.annots as usize);
        {
            let from = core::cmp::min(m.slot.saturating_sub(self.base) as usize, self.slots.len());
            for i in from..self.slots.len() {
                let k = self.slots[i].kind;
                if k == K_OBJ + Cls::Vertex as u8
                    || k == K_OBJ + Cls::Edge as u8
                    || k == K_OBJ + Cls::Face as u8
                    || k == K_OBJ + Cls::Loop as u8
                    // Both of these now carry an index into `ftcs`, which was
                    // just truncated with the rest; a back-reference from a
                    // later definition has to read as unknown rather than as
                    // somebody else's mapping.
                    || k == K_OBJ + Cls::Ftc as u8
                    || k == K_OBJ + Cls::AttrContainer as u8
                {
                    self.slots[i] = Slot {
                        kind: K_FREED,
                        idx: NONE_IDX,
                    };
                }
            }
        }
    }

    // -- emitting one definition's batches --------------------------------

    fn layer_index(&self, slot: u32) -> u32 {
        if slot == 0 {
            return 0;
        }
        let e = self.entry(slot);
        if e.is(Cls::Layer) && e.idx != NONE_IDX {
            e.idx
        } else {
            0
        }
    }

    fn material_index(&self, slot: u32) -> u32 {
        if slot == 0 {
            return NO_MATERIAL;
        }
        let e = self.entry(slot);
        if e.is(Cls::Material) && e.idx != NONE_IDX {
            e.idx
        } else {
            NO_MATERIAL
        }
    }

    fn emit_definition(&mut self, id: u32, m: &Mark) {
        self.emit_edges(id, m);
        self.emit_meshes(id, m);
    }

    /// One definition's (or the root's) topology, in the shape the modern
    /// path's `geom::parse_component` produces: vertices, edges, faces with
    /// their loops in CSR form, placements and guides. Hidden faces are kept
    /// (bit 0 of `face_flags`), unlike the batches, which skip them.
    ///
    /// Indices are rebased onto this definition's own arrays; a reference
    /// that leaves them (into an enclosing definition's arenas, or past the
    /// end) becomes `u32::MAX`, which `DefGeometry` already means as a
    /// dangling reference. Counts every element against `budget`; `None`
    /// once it is spent, with `over_budget` set.
    fn emit_topology(&mut self, id: u32, m: &Mark) -> Option<DefGeometry> {
        if self.over_budget {
            return None;
        }
        let g = self.build_topology(id, m);
        if g.is_none() {
            self.over_budget = true;
        }
        g
    }

    fn build_topology(&mut self, id: u32, m: &Mark) -> Option<DefGeometry> {
        let weight = if id == ROOT_DEFINITION {
            1
        } else {
            DEFINITION_WEIGHT + 1
        };
        spend(&mut self.budget, weight).ok()?;
        let mut g = DefGeometry::default();

        let vbase = m.verts as usize;
        let verts = self.verts.get(vbase..).unwrap_or(&[]);
        spend(&mut self.budget, verts.len() as u64).ok()?;
        for (i, p) in verts.iter().enumerate() {
            g.vertex_pid.push(i as u64);
            g.vertex_pos.extend_from_slice(p);
        }
        let local_vertex = |v: u32| -> u32 {
            match (v as usize).checked_sub(vbase) {
                Some(k) if k < verts.len() => k as u32,
                _ => u32::MAX,
            }
        };

        let ebase = m.edges as usize;
        let edges = self.edges.get(ebase..).unwrap_or(&[]);
        spend(&mut self.budget, edges.len() as u64).ok()?;
        for (i, e) in edges.iter().enumerate() {
            g.edge_pid.push(i as u64);
            g.edge_v.push(local_vertex(e.v1));
            g.edge_v.push(local_vertex(e.v2));
            g.edge_flags.push(e.flags);
            g.edge_layer.push(self.layer_index(e.layer_slot));
        }

        let faces = self.faces.get(m.faces as usize..).unwrap_or(&[]);
        spend(&mut self.budget, faces.len() as u64).ok()?;
        g.face_loop_ofs.push(0);
        g.loop_ofs.push(0);
        for f in faces {
            g.face_normal.extend_from_slice(&f.normal);
            g.face_plane_d.push(f.plane_d);
            g.face_material.push(self.material_index(f.mat_slot));
            g.face_back_material
                .push(self.material_index(f.back_mat_slot));
            g.face_layer.push(self.layer_index(f.layer_slot));
            g.face_flags.push(f.flags);
            match self.front_matrix(f.ftc) {
                Some(xf) => {
                    g.face_uv.push((g.uv_xf.len() / 9) as u32);
                    g.uv_xf.extend_from_slice(&xf);
                    g.uv_flags.push(
                        self.ftcs
                            .get(f.ftc as usize)
                            .map(|r| r.front_flags)
                            .unwrap_or(0),
                    );
                }
                None => g.face_uv.push(NO_UV),
            }
            let lend = f.loop_start.saturating_add(f.loop_len);
            for li in f.loop_start..lend {
                // A loop belongs to this definition only if it was read
                // after the mark; anything else is a stale reference.
                let Some(&lidx) = self.face_loops.get(li as usize) else {
                    continue;
                };
                if lidx < m.loops {
                    continue;
                }
                let Some(lr) = self.loops.get(lidx as usize).copied() else {
                    continue;
                };
                let from = lr.start as usize;
                let Some(to) = from.checked_add(lr.len as usize) else {
                    continue;
                };
                if from < m.loop_uses as usize || to > self.loop_uses.len() || lr.len == 0 {
                    continue;
                }
                spend(&mut self.budget, lr.len as u64 + 1).ok()?;
                for u in &self.loop_uses[from..to] {
                    let e = match (u.edge as usize).checked_sub(ebase) {
                        Some(k) if k < edges.len() => k as u32,
                        _ => u32::MAX,
                    };
                    g.coedge_edge.push(e);
                    g.coedge_reversed.push(u.reversed as u8);
                }
                g.loop_ofs.push(g.coedge_edge.len() as u32);
            }
            g.face_loop_ofs.push(g.loop_ofs.len() as u32 - 1);
        }

        let instances = self.instances.get(m.instances as usize..).unwrap_or(&[]);
        for inst in instances.iter().filter(|i| i.parent == id) {
            spend(&mut self.budget, 1).ok()?;
            let material = self.material_index(inst.mat_slot);
            g.instances.push(RawInstance {
                pid: 0,
                name: inst.name.clone(),
                def_pid: inst.def_slot as u64,
                guid: [0; 16],
                m: inst.xf,
                material: material as u64,
                has_material: material != NO_MATERIAL,
                layer: self.layer_index(inst.layer_slot) as u64,
                has_layer: true,
                flags: if inst.hidden {
                    crate::model::edge_flag::HIDDEN
                } else {
                    0
                },
                is_group: inst.is_group,
                is_image: inst.is_image,
            });
        }

        let guides = self.guides.get(m.guides as usize..).unwrap_or(&[]);
        spend(&mut self.budget, guides.len() as u64).ok()?;
        g.guides.extend_from_slice(guides);
        let layers = self.guide_layers.get(m.guides as usize..).unwrap_or(&[]);
        g.guide_layer = layers.iter().map(|&s| self.layer_index(s)).collect();
        let at = m.hidden_guide_items as usize;
        let hidden = self.hidden_guide_items.get(at..).unwrap_or(&[]);
        spend(&mut self.budget, hidden.len() as u64).ok()?;
        g.hidden_guide_items.extend_from_slice(hidden);
        let layers = self.hidden_guide_layers.get(at..).unwrap_or(&[]);
        g.hidden_guide_layer = layers.iter().map(|&s| self.layer_index(s)).collect();
        self.resolve_annotations(&mut g, m)?;
        Some(g)
    }

    /// Turns the dimensions and labels read since `m` into `g.annotations` and
    /// `g.annotation_skips`, the shapes the modern path produces, counting each against the
    /// element budget. A point attached to an entity becomes that vertex's or guide point's
    /// position; one attached to anything else (or to something outside this definition) is
    /// unread. `None` once the budget is spent.
    fn resolve_annotations(&mut self, g: &mut DefGeometry, m: &Mark) -> Option<()> {
        use crate::annotations::RawAnnotation;
        let at = m.annots as usize;
        let count = self.pending_annots.len().saturating_sub(at);
        spend(&mut self.budget, count as u64).ok()?;
        let pending = self.pending_annots.get(at..).unwrap_or(&[]);
        let finite = |p: [f64; 3]| p.iter().all(|x| x.is_finite());
        let point = |p: AnnotPoint| -> Option<[f64; 3]> {
            let p = match p {
                AnnotPoint::Free(p) => p,
                AnnotPoint::Entity(r) => {
                    let e = self.entry(self.translate_ref(r).ok().flatten()?);
                    if e.is(Cls::Vertex) {
                        let k = (e.idx as usize).checked_sub(m.verts as usize)?;
                        let v = g.vertex_pos.get(3 * k..3 * k + 3)?;
                        [v[0], v[1], v[2]]
                    } else if e.is(Cls::ConstructionPoint) && e.idx >= m.guides {
                        match self.guides.get(e.idx as usize)? {
                            RawGuide::Point { position } => *position,
                            RawGuide::Line { .. } => return None,
                        }
                    } else {
                        return None;
                    }
                }
            };
            finite(p).then_some(p)
        };
        let mut found = Vec::new();
        let mut skips = crate::annotations::AnnotationSkips::default();
        for a in pending {
            if a.hidden {
                skips.hidden = skips.hidden.saturating_add(1);
                continue;
            }
            let layer = self.layer_index(a.layer_slot);
            let made = match &a.body {
                PendingBody::Unread => None,
                PendingBody::Linear {
                    start,
                    end,
                    normal,
                    x_axis,
                    dim_type,
                    y_pos,
                    custom_text,
                } => (|| {
                    let (start, end) = (point(*start)?, point(*end)?);
                    let offset = crate::annotations::linear_offset(
                        *dim_type, *normal, *x_axis, *y_pos, start, end,
                    )?;
                    Some(RawAnnotation::Linear {
                        start,
                        end,
                        offset,
                        layer,
                        custom_text: *custom_text,
                    })
                })(),
                PendingBody::Leader {
                    anchor,
                    vector,
                    kind,
                    text,
                } => {
                    if *kind != 2 {
                        skips.screen_text = skips.screen_text.saturating_add(1);
                        continue;
                    }
                    point(*anchor)
                        .filter(|_| finite(*vector))
                        .map(|anchor| RawAnnotation::Leader {
                            anchor,
                            vector: *vector,
                            text: text.clone(),
                            layer,
                        })
                }
            };
            match made {
                Some(a) => found.push(a),
                None => skips.unread = skips.unread.saturating_add(1),
            }
        }
        g.annotations = found;
        g.annotation_skips = skips;
        Some(())
    }

    /// Edges, with their flags and both adjacent face normals.
    ///
    /// `face_counts` under two means a boundary edge, which is always a
    /// profile; the renderer's silhouette test is `dot(n0,v)*dot(n1,v) < 0`
    /// evaluated per edge in this definition's own space, so the normals have
    /// to travel with the edge rather than be looked up through a face.
    ///
    /// The sign convention is stated on `model::EdgeBatch` and implemented for
    /// the modern container in `edges::build_adjacency`; both halves of it are
    /// below, and `legacy_coplanar_faces_store_equal_edge_normals` is the
    /// check that says so.
    fn emit_edges(&mut self, id: u32, m: &Mark) {
        let ebase = m.edges as usize;
        let ne = self.edges.len() - ebase;
        if ne == 0 {
            return;
        }
        let mut fcount = vec![0u8; ne];
        let mut fnorm = vec![0f32; ne * 6];
        for f in &self.faces[m.faces as usize..] {
            for li in f.loop_start..f.loop_start.saturating_add(f.loop_len) {
                let Some(&lidx) = self.face_loops.get(li as usize) else {
                    continue;
                };
                let Some(lr) = self.loops.get(lidx as usize).copied() else {
                    continue;
                };
                let from = lr.start as usize;
                let to = from.saturating_add(lr.len as usize);
                if to > self.loop_uses.len() {
                    continue;
                }
                for u in &self.loop_uses[from..to] {
                    let eu = u.edge as usize;
                    if eu < ebase || eu - ebase >= ne {
                        continue;
                    }
                    let k = eu - ebase;
                    let slot = fcount[k] as usize;
                    if slot < 2 {
                        // Negate for a reversed use, and then negate the
                        // *second* one again on storage. Both halves, the way
                        // `edges::build_adjacency` does it for the modern
                        // container - this did only the first, and the
                        // invariant on `model::EdgeBatch` says what that
                        // costs: two coplanar faces then store opposite
                        // normals instead of equal ones, `d0 * d1 < 0` fires
                        // on every triangulation diagonal in the model, and
                        // once the softened range is drawn at all a curved
                        // surface comes out as dense hatching instead of an
                        // outline. Measured on the school cafeteria: 77.9% of
                        // edges had nearly antiparallel pairs against 0.1% on
                        // the VFF classroom, and 94.7% of its softened edges
                        // passed the profile test against 5.5%.
                        let s = if u.reversed { -1.0 } else { 1.0 };
                        let store = if slot == 1 { -s } else { s };
                        for a in 0..3 {
                            fnorm[k * 6 + slot * 3 + a] = (f.normal[a] * store) as f32;
                        }
                    }
                    fcount[k] = fcount[k].saturating_add(1);
                }
            }
        }

        let mut b = EdgeBatch {
            definition_id: id,
            ..Default::default()
        };
        b.positions.reserve(ne * 6);
        b.flags.reserve(ne);
        for k in 0..ne {
            let e = self.edges[ebase + k];
            let (Some(a), Some(c)) = (self.verts.get(e.v1 as usize), self.verts.get(e.v2 as usize))
            else {
                continue;
            };
            for p in [a, c] {
                for a in 0..3 {
                    b.positions.push((p[a] * INCH_TO_MM) as f32);
                }
            }
            b.face_normals.extend_from_slice(&fnorm[k * 6..k * 6 + 6]);
            b.flags.push(e.flags);
            b.face_counts.push(fcount[k]);
            b.layer_ids.push(self.layer_index(e.layer_slot));
        }
        if !b.is_empty() {
            self.edge_batches.push(b);
        }
    }

    /// Triangles, one batch per (material, layer) pair the definition uses,
    /// with texture coordinates where the paint could be an image.
    ///
    /// # The face-plane convention, verified rather than assumed
    ///
    /// A `CFaceTextureCoords` matrix maps texture space to *face-plane
    /// coordinates*, and a matrix is meaningless without knowing which in-plane
    /// axes those coordinates are measured along. openskp's two ports disagree:
    /// its reader derives the axes from the face normal alone, its writer
    /// (`solveUvMatrix` in `create.ts`) derives them from the face's own first
    /// edge direction. Both cannot be the file's convention, and picking the
    /// wrong one rotates a texture by a per-face angle - which looks like a
    /// texture problem, not a reader bug.
    ///
    /// Settled on these files, with no ground truth needed, by the one thing
    /// the two conventions cannot both satisfy: **UV continuity across a shared
    /// vertex.** Group faces by (normal, stored matrix) - so every face in a
    /// group has the identical mapping - and compare the UV each face computes
    /// at a vertex position they share. The normal-only axes are the same for
    /// every face in the group, so they must agree; per-face first-edge axes
    /// differ wherever two faces start their loops along different edges, so
    /// they cannot. Measured:
    ///
    /// ```text
    ///                            normal-only axes      first-edge axes
    /// school cafeteria   20,341 samples   max 1e-11 in   2.3% wrong, max 532 in
    /// bear home           2,838 samples   max 0          65% wrong,  max 22 in
    /// bmw m3                196 samples   max 9e-11 in   1.0% wrong, max 334 in
    /// ```
    ///
    /// The normal-only axes are exact to floating-point noise; the first-edge
    /// axes tear the texture at shared edges by up to 13 metres. So this uses
    /// `geom::uv_axes`, which is the normal-only frame, and which is also what
    /// the modern path uses - so the two containers agree by construction.
    ///
    /// `tests/legacy.rs::legacy_uvs_are_continuous_across_shared_vertices`
    /// keeps that measurement from rotting.
    fn emit_meshes(&mut self, id: u32, m: &Mark) {
        let fbase = m.faces as usize;
        if self.faces.len() == fbase {
            return;
        }
        // Group keys in first-seen order; a definition uses a handful of
        // materials, so a linear scan beats a map.
        let mut keys: Vec<(u32, u32)> = Vec::new();
        for f in &self.faces[fbase..] {
            let k = (
                self.material_index(f.mat_slot),
                self.layer_index(f.layer_slot),
            );
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        let mut rings: Vec<Vec<[f64; 3]>> = Vec::new();
        // One flat point array per face plus one index ring per loop, which is
        // the shape `geom::triangulate` takes.
        let mut pts: Vec<[f64; 3]> = Vec::new();
        let mut loops: Vec<Vec<u32>> = Vec::new();
        for (mat, layer) in keys {
            let mut b = MeshBatch {
                definition_id: id,
                material_id: mat,
                layer_id: layer,
                ..Default::default()
            };
            // Per batch, not per face, so that `uvs` is either parallel to
            // `positions` for the whole batch or absent from it - the
            // invariant `model.rs` states and `scene::enforce_uv_invariant`
            // checks. A batch with no paint of its own is included because
            // the instance that places it can carry an image.
            let want_uv = self.wants_uv(mat);
            for fi in fbase..self.faces.len() {
                let f = self.faces[fi];
                if f.hidden {
                    continue;
                }
                if (
                    self.material_index(f.mat_slot),
                    self.layer_index(f.layer_slot),
                ) != (mat, layer)
                {
                    continue;
                }
                rings.clear();
                self.face_rings(&f, &mut rings);
                if rings.is_empty() || rings[0].len() < 3 {
                    continue;
                }
                // Loop 0 is the outer ring and the rest are holes - a door
                // and a window in a wall. This hands all of them to
                // `geom::triangulate`, the earcut port the modern path uses
                // and the one that has been run over a 489-loop perforated
                // panel.
                //
                // What was here before was a nearest-vertex bridge feeding a
                // local ear clipper. The clipper refused degenerate ears, and
                // a bridge duplicates two vertices by construction, so a
                // bridged ring always presented it with one; after a full pass
                // with no ear it force-clipped, which is a triangle drawn
                // across the opening. Every hole in the corpus hit that path.
                // Measured by the exact-count test either side of the change
                // (`V + 2H - 2` triangles for `V` loop vertices and `H`
                // holes), with the school cafeteria's 393 hole-bearing faces
                // as the scale:
                //
                //     file          faces short of exact   triangles covering
                //                                          a vertex of their
                //                                          own face
                //     restaurant        1,268 -> 15           15,877 -> 295
                //     food                138 ->  3            1,354 ->  43
                //     cafeteria           393 -> 29            2,346 ->  62
                //     bear home            21 ->  1              184 ->   2
                //     eurovision           23 ->  0              120 ->   4
                //
                // 393 of 393 - every hole-bearing face in the cafeteria was
                // short. The symptom was a wall face filling its own
                // openings: on the restaurant pack's Afternoon camera the
                // right-hand wall, the window glass, the door leaves and the
                // inside of the picture frame all shaded to one flat
                // RGB(102,102,102) over 38.7% of the frame, a value
                // SketchUp's own render of the same camera does not contain
                // once.
                pts.clear();
                loops.clear();
                for ring in rings.iter() {
                    let start = pts.len() as u32;
                    pts.extend_from_slice(ring);
                    loops.push((start..pts.len() as u32).collect());
                }
                let refs: Vec<&[u32]> = loops.iter().map(|l| l.as_slice()).collect();
                let tris = crate::geom::triangulate(&pts, &refs, f.normal);
                if tris.is_empty() {
                    continue;
                }
                // Any triangulation of a polygon with `V` vertices over all
                // its loops and `H` holes has exactly `V + 2H - 2` triangles.
                // Counting the shortfall is what distinguishes a hole that was
                // dropped from a model that has none; see
                // `ReadStats::faces_short_of_exact`.
                let exact = pts.len() + 2 * refs.len().saturating_sub(1) - 2;
                if tris.len() < exact {
                    self.faces_short += 1;
                }
                let uv = if want_uv {
                    Some(crate::geom::UvMap::new(
                        f.normal,
                        self.front_matrix(f.ftc).as_ref(),
                    ))
                } else {
                    None
                };
                let vbase = (b.positions.len() / 3) as u32;
                // Every loop's points, holes included: a hole's vertices are
                // vertices of the face and the triangles the bridge produces
                // reference them.
                for p in &pts {
                    for a in 0..3 {
                        b.positions.push((p[a] * INCH_TO_MM) as f32);
                    }
                    for a in 0..3 {
                        b.normals.push(f.normal[a] as f32);
                    }
                    if let Some(map) = &uv {
                        // Inches in, inches out, then scaled once. `UvMap` is
                        // linear in the point only through the 2x2 block: the
                        // matrix's translation column and its projective row
                        // are in the file's own unit, so converting the point
                        // first leaves the offset un-scaled and slides the
                        // texture. Measured on `pair_feature-pack`, whose
                        // positioned faces come out 1,070 mm of texture space
                        // away from where they belong if the millimetre point
                        // is handed in instead; on the bear home, 1,729 mm.
                        let t = map.at(*p);
                        b.uvs.push((t[0] * INCH_TO_MM) as f32);
                        b.uvs.push((t[1] * INCH_TO_MM) as f32);
                    }
                }
                for t in &tris {
                    b.indices.push(vbase + t[0]);
                    b.indices.push(vbase + t[1]);
                    b.indices.push(vbase + t[2]);
                }
            }
            if !b.indices.is_empty() {
                self.meshes.push(b);
            }
        }
    }

    /// Whether a batch painted with `mat` should carry UVs.
    ///
    /// Mirrors `scene::Model::wants_uv`: a model with no image anywhere gets
    /// none at all, and within a model that has one, a batch qualifies when
    /// its own paint is an image or when it has no paint of its own - because
    /// then the instance placing it decides, and this reader does not know
    /// which instance that will be.
    fn wants_uv(&self, mat: u32) -> bool {
        if !self.any_textured {
            return false;
        }
        if mat == NO_MATERIAL {
            return true;
        }
        self.materials
            .get(mat as usize)
            .is_some_and(|m| m.texture.is_some())
    }

    /// One face's stored front mapping, or `None` for the default projection.
    fn front_matrix(&self, ftc: u32) -> Option<[f64; 9]> {
        if ftc == NONE_IDX {
            return None;
        }
        self.ftcs.get(ftc as usize).map(|r| r.front)
    }

    /// A face's loops as point rings, outer first, in inches.
    fn face_rings(&self, f: &FaceRec, out: &mut Vec<Vec<[f64; 3]>>) {
        for li in f.loop_start..f.loop_start.saturating_add(f.loop_len) {
            let Some(&lidx) = self.face_loops.get(li as usize) else {
                continue;
            };
            let Some(lr) = self.loops.get(lidx as usize).copied() else {
                continue;
            };
            let from = lr.start as usize;
            let to = from.saturating_add(lr.len as usize);
            if to > self.loop_uses.len() || lr.len < 3 {
                continue;
            }
            let mut ring: Vec<[f64; 3]> = Vec::with_capacity(lr.len as usize);
            let mut ok = true;
            for u in &self.loop_uses[from..to] {
                let Some(e) = self.edges.get(u.edge as usize) else {
                    ok = false;
                    break;
                };
                // The loop walks the edge in its own direction, so the point
                // it contributes is the edge's start under that direction.
                let v = if u.reversed { e.v2 } else { e.v1 };
                let Some(p) = self.verts.get(v as usize) else {
                    ok = false;
                    break;
                };
                ring.push(*p);
            }
            if ok && ring.len() >= 3 {
                out.push(ring);
            }
        }
    }
}

struct Mark {
    verts: u32,
    edges: u32,
    faces: u32,
    loops: u32,
    loop_uses: u32,
    face_loops: u32,
    ftcs: u32,
    guides: u32,
    hidden_guide_items: u32,
    annots: u32,
    /// Not released: `finish` groups every placement at the end.
    instances: u32,
    slot: u32,
}

/// A point of a dimension or label: stated in the record, or the entity (a store-map index
/// of the file, as `entity_ref` reads it) the record is attached to.
#[derive(Clone, Copy)]
enum AnnotPoint {
    Free([f64; 3]),
    Entity(u32),
}

enum PendingBody {
    /// Not read: a connection through an instance path, or a layout not recognised.
    Unread,
    Linear {
        start: AnnotPoint,
        end: AnnotPoint,
        normal: [f64; 3],
        x_axis: [f64; 3],
        /// 0 aligned, 1 horizontal, 2 vertical (see `annotations::linear_offset`).
        dim_type: u32,
        y_pos: f64,
        custom_text: bool,
    },
    Leader {
        anchor: AnnotPoint,
        vector: [f64; 3],
        /// 2 is a pushpin leader; other values are screen or view-based text.
        kind: u32,
        text: String,
    },
}

/// A dimension or label read from the file, waiting for its definition to be built.
struct PendingAnnot {
    hidden: bool,
    layer_slot: u32,
    body: PendingBody,
}

/// The `f64` at byte `at` of `b` (the caller has checked the length).
fn f64_at(b: &[u8], at: usize) -> f64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    f64::from_le_bytes(a)
}

struct TexBlock {
    rgba: [u8; 4],
    opacity: f64,
    use_opacity: u8,
    dib: u32,
    w: f64,
    h: f64,
    file: String,
    colorized: bool,
}

/// The stored double is a transparency with zero meaning opaque, gated by a
/// trailing use-flag byte exactly the way VFF's `useTrans` gates its own.
fn solid_opacity(stored: f64, use_flag: u8) -> f32 {
    if use_flag == 0 {
        return 1.0;
    }
    let v = 1.0 - stored;
    if v.is_nan() {
        1.0
    } else {
        v.clamp(0.0, 1.0) as f32
    }
}

/// `hex16` in reverse; anything that is not 32 hex digits reads as zero.
fn unhex16(s: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    let b = s.as_bytes();
    if b.len() != 32 {
        return out;
    }
    for (i, o) in out.iter_mut().enumerate() {
        let hi = (b[2 * i] as char).to_digit(16);
        let lo = (b[2 * i + 1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) => *o = (h * 16 + l) as u8,
            _ => return [0; 16],
        }
    }
    out
}

fn hex16(b: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for x in b {
        s.push(
            char::from_digit((x >> 4) as u32, 16)
                .unwrap_or('0')
                .to_ascii_uppercase(),
        );
        s.push(
            char::from_digit((x & 0xf) as u32, 16)
                .unwrap_or('0')
                .to_ascii_uppercase(),
        );
    }
    s
}

/// Splice `hole` into `outer` through the closest pair of points.
///
/// This is the standard bridge: cut the outer ring at its nearest vertex,
/// insert the hole traversed from its own nearest vertex and closed back on
/// itself, then resume the outer ring. It can produce a degenerate bridge when
/// two holes are nearly coincident; ear clipping then drops the affected
/// triangles rather than producing crossed ones.
// ---------------------------------------------------------------------------
// Finding the start of the walk, and the slot base.
// ---------------------------------------------------------------------------

/// How far into the stream to look for the material manager.
///
/// Measured across the corpus the first `CMaterial` class declaration sits
/// between 0x11c2 and 0x29331, and the 517 MB pack's is at 0x22772 - the
/// manager is always in the first quarter-megabyte because the *prefix* is
/// small; it is the material *list* that is huge, since that is where the
/// inline textures live. Bounding this is not a nicety: searching 517 MB for
/// a byte pattern would read the whole file and defeat the point.
const ANCHOR_WINDOW: u64 = 4 << 20;

/// How far back from the end of the stream to look for the page list when the
/// entity walk did not get there. Measured, the page list ends between 1.4 KB
/// and 615 KB before the end of the archive on every file in the corpus.
const PAGE_WINDOW: u64 = 8 << 20;

struct Anchor {
    start: u64,
    materials: u32,
}

/// Locate the material manager: a `u32` count immediately followed by the
/// first `CMaterial` class declaration.
///
/// A file with no materials at all has no `CMaterial` record anywhere, so fall
/// back to the layer manager, whose own marker sits a fixed distance before
/// the first `CLayer` declaration.
fn find_anchor(c: &mut Cur, wide_entity: bool) -> Result<Anchor> {
    if let Some(at) = class_decl(c, b"CMaterial", ANCHOR_WINDOW)? {
        let count = c.u32_at(at - 4)?;
        if count <= 100_000 {
            return Ok(Anchor {
                start: at,
                materials: count,
            });
        }
    }
    if let Some(at) = class_decl(c, b"CLayer", ANCHOR_WINDOW)? {
        let back = if wide_entity { 9 } else { 8 };
        if at >= back {
            return Ok(Anchor {
                start: at - back,
                materials: 0,
            });
        }
    }
    Err(Error::Malformed(
        "no material or layer class record near the start of the archive",
    ))
}

/// The offset of a `0xFFFF <u16 schema> <u16 len> <name>` class declaration.
fn class_decl(c: &mut Cur, name: &[u8], window: u64) -> Result<Option<u64>> {
    let mut needle = Vec::with_capacity(name.len() + 2);
    needle.push(name.len() as u8);
    needle.push(0);
    needle.extend_from_slice(name);
    let save = c.at();
    let mut from = save;
    let stop = save.saturating_add(window);
    let out = loop {
        if from >= stop {
            break None;
        }
        c.seek(from)?;
        let Some(at) = c.find(&needle, stop - from)? else {
            break None;
        };
        from = at + 1;
        // The needle starts at the *name length* word, which is four bytes
        // into the declaration: `0xFFFF <u16 schema> <u16 len> <name>`.
        if at >= c.start + 4 {
            if let Ok(t) = c.u16_at(at - 4) {
                if t == 0xffff {
                    break Some(at - 4);
                }
            }
        }
    };
    c.seek(save)?;
    Ok(out)
}

impl<'a> Archive<'a> {
    /// Walk the material and layer managers with relative numbering until the
    /// file names the absolute slot of a class we declared ourselves.
    ///
    /// This is deterministic, not a search: a class-ref carries the class's
    /// absolute slot, and we know its index relative to our own base, so one
    /// subtraction gives the answer. Any class used twice in the walked region
    /// supplies the equation, which in practice means the second material or
    /// the second layer - available in all but the very smallest files.
    ///
    /// When a file has exactly one material and one layer, no class repeats
    /// before the definition list, so the fallback is the definition-list
    /// anchor: an absolute back-reference to the active layer, an object we
    /// just allocated relatively. With one layer that is exact too; with
    /// several it yields one candidate per layer and the caller tries them.
    fn probe(&mut self, a: &Anchor) -> (Option<(u32, &'static str)>, Vec<u32>) {
        self.base = PROBE_BASE;
        self.probing = true;
        let mut candidates = Vec::new();
        let r = (|| -> Result<()> {
            self.c.seek(a.start)?;
            for _ in 0..a.materials {
                self.read_object(Some(Cls::Material))?;
                if self.learned_base.is_some() {
                    return Ok(());
                }
            }
            let _ = self.c.u32()?;
            if self.wide_entity {
                let _ = self.c.u8()?;
            }
            let layer_count = self.c.u32()?;
            if layer_count == 0 || layer_count > 100_000 {
                return Err(Error::Malformed(
                    "implausible layer count in the base probe",
                ));
            }
            let mut layer_slots: Vec<u32> = Vec::new();
            while (layer_slots.len() as u32) < layer_count {
                if let Some((s, e)) = self.read_object(Some(Cls::Layer))? {
                    if e.is(Cls::Layer) {
                        layer_slots.push(s);
                    }
                }
                if self.learned_base.is_some() {
                    return Ok(());
                }
            }
            self.collect_trailing_layers(&mut layer_slots, true)?;
            if self.learned_base.is_some() {
                return Ok(());
            }
            // The definition-list anchor. Under the probe base every absolute
            // back-ref classifies as pre-model, which is the signal that the
            // prefix parsed: anything else means it did not.
            let Some((s, e)) = self.read_object(None)? else {
                return Err(Error::Malformed("base probe: no definition-list anchor"));
            };
            if e.kind != K_PREMODEL {
                return Err(Error::Malformed("base probe: anchor did not resolve"));
            }
            for rel in layer_slots {
                let off = rel - PROBE_BASE;
                if s > off {
                    let cand = s - off;
                    if cand > 0 && cand < PROBE_BASE && !candidates.contains(&cand) {
                        candidates.push(cand);
                    }
                }
            }
            Ok(())
        })();
        // A probe failure is not reported: either it learned enough, or the
        // caller has no base to try and says so itself.
        let _ = r;
        self.probing = false;
        (self.learned_base, candidates)
    }

    fn collect_trailing_layers(
        &mut self,
        out: &mut Vec<u32>,
        unmatched_class_ref: bool,
    ) -> Result<()> {
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 4096 || self.c.left() < 2 {
                return Ok(());
            }
            let tag = self.c.peek_u16()?;
            if tag == 0 {
                self.c.skip(2)?;
                continue;
            }
            let here = self.c.at();
            if !self.layer_record_at(here, unmatched_class_ref) {
                return Ok(());
            }
            match self.read_object(Some(Cls::Layer))? {
                Some((s, e)) if e.is(Cls::Layer) => out.push(s),
                _ => return Ok(()),
            }
        }
    }

    /// Materials, layers, definitions, root. The whole model, in file order.
    fn walk(&mut self, a: &Anchor) -> Result<()> {
        self.c.seek(a.start)?;
        for _ in 0..a.materials {
            self.read_object(Some(Cls::Material))?;
        }
        // Layer-list marker: a `u32`, then a byte on `CEntity` schema 5+,
        // then the count.
        let _ = self.c.u32()?;
        if self.wide_entity {
            let _ = self.c.u8()?;
        }
        let layer_count = self.c.u32()?;
        if layer_count > 100_000 {
            return Err(Error::Malformed("implausible layer count"));
        }
        let mut layer_slots: Vec<u32> = Vec::new();
        let mut guard = 0;
        while (layer_slots.len() as u32) < layer_count {
            guard += 1;
            if guard > layer_count as u64 + 4096 {
                break;
            }
            let tag = self.c.peek_u16()?;
            if tag == 0 {
                self.c.skip(2)?;
                continue;
            }
            // A plain back-ref here is the definition-list anchor, not a
            // layer: some writers over-count the list by its separators.
            if tag != 0xffff && tag & 0x8000 == 0 {
                break;
            }
            match self.read_object(Some(Cls::Layer))? {
                Some((s, e)) if e.is(Cls::Layer) => layer_slots.push(s),
                _ => {}
            }
        }
        self.collect_trailing_layers(&mut layer_slots, false)?;
        self.model_layers = layer_slots;

        // Definition list: a pointer to the *active* layer, then the count.
        let anchor = self.read_object(None)?;
        match anchor {
            Some((_, e)) if e.is(Cls::Layer) || e.kind == K_PREMODEL => {}
            _ => return Err(Error::Malformed("definition-list anchor is not a layer")),
        }
        // SketchUp 2020 writes a filler here (`V20_FILLER`). Skipped when it
        // is exactly there; otherwise a count that cannot be right is looked
        // for again past a filler of another length, as openskp does.
        if self.v20 && self.bytes_eq_at(self.c.at(), &V20_FILLER) {
            self.c.skip(V20_FILLER.len() as u64)?;
        }
        let count_pos = self.c.at();
        let mut def_count = self.c.u32()?;
        if self.v20 && (def_count > 1_000_000 || def_count == 0) {
            if let Some(n) =
                self.count_after_v20_filler(count_pos, 1_000_000, Follows::Definition)?
            {
                def_count = n;
            }
        }
        if def_count > 1_000_000 {
            return Err(Error::Malformed("implausible definition count"));
        }
        let mut done = 0u32;
        while done < def_count {
            let p = self.c.at();
            let r = self.read_object(Some(Cls::Definition));
            if self.over_budget {
                return Err(Error::Malformed("too many elements"));
            }
            match r {
                Ok(_) => done += 1,
                Err(e) => {
                    self.note(format!(
                        "definition {done} of {def_count} failed at {p:#x}: {e}"
                    ));
                    self.lost += 1;
                    done += 1;
                    if !self.resync_definition(p)? {
                        // The rest of the definitions and the whole root list
                        // are unread: an error, not an empty model.
                        return Err(Error::Malformed(
                            "a definition failed and the next one could not be found",
                        ));
                    }
                }
            }
        }

        // Definitions past the declared count, written back to back.
        let def_cls = self.class_slot[Cls::Definition as usize];
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 1_000_000 || self.c.left() < 2 {
                break;
            }
            let tag = self.c.peek_u16()?;
            let mut is_def = def_cls != NONE_IDX && self.is_class_ref_at(self.c.at(), def_cls);
            if !is_def && tag == 0xffff {
                is_def = self.bytes_eq_at(self.c.at() + 6, b"CComponentDefinition");
            }
            if !is_def {
                break;
            }
            if self.read_object(None).is_err() {
                break;
            }
        }

        if self.over_budget {
            return Err(Error::Malformed("too many elements"));
        }

        // Root entity list.
        let count_pos = self.c.at();
        let mut root_count = self.c.u32()?;
        if self.v20 && root_count > 5_000_000 {
            if let Some(n) = self.count_after_v20_filler(count_pos, 5_000_000, Follows::ListItem)? {
                root_count = n;
            }
        }
        if root_count > 5_000_000 {
            return Err(Error::Malformed("implausible root entity count"));
        }
        let mark = self.mark();
        self.read_entity_list(root_count, Owner::Root)?;
        match self.emit {
            Emit::Batches => self.emit_definition(ROOT_DEFINITION, &mark),
            Emit::Topology => self.topo_root = self.emit_topology(ROOT_DEFINITION, &mark),
            Emit::Nothing => {}
        }
        self.release(&mark);
        if self.over_budget {
            return Err(Error::Malformed("too many elements"));
        }
        self.walk_end = self.c.at();
        Ok(())
    }

    /// openskp's `findCountAfterV20Filler` (MIT; see `VENDORED.md`): when a
    /// SketchUp 2020 count read at `count_pos` cannot be right, look for an
    /// empty string record within the next 12 bytes, then for the first
    /// count `0 < n <= limit` past its zero padding - at the offsets that
    /// keep the count 4-byte aligned with the padding lengths seen, 9 and 13
    /// - that what `follows` says comes next. On a match the cursor is left
    /// after the count; otherwise it is not moved.
    ///
    /// A count of zero is never found this way, so a list that is really
    /// empty stays unread (an error or a counted loss) rather than taking a
    /// later word for its count.
    fn count_after_v20_filler(
        &mut self,
        count_pos: u64,
        limit: u32,
        follows: Follows,
    ) -> Result<Option<u32>> {
        const MAX_PAD: u64 = 29;
        let Some(marker) =
            (count_pos..count_pos + 12).find(|&i| self.bytes_eq_at(i, &[0xff, 0xfe, 0xff]))
        else {
            return Ok(None);
        };
        if self.byte_at(marker + 3) != Some(0) {
            // A string that is not empty is data, not the filler.
            return Ok(None);
        }
        let mut pad = 1;
        while pad <= MAX_PAD {
            let at = marker + 4 + pad;
            let Ok(n) = self.c.u32_at(at) else {
                break;
            };
            let next = match follows {
                Follows::ListItem => self.plausible_list_tag(at + 4),
                Follows::Definition => self.definition_at(at + 4),
            };
            if n > 0 && n <= limit && next {
                self.c.seek(at + 4)?;
                return Ok(Some(n));
            }
            pad += 4;
        }
        Ok(None)
    }

    /// A `CComponentDefinition` starts at `at`: its class declaration, or a
    /// class-ref to the class once it is known.
    fn definition_at(&mut self, at: u64) -> bool {
        let def_cls = self.class_slot[Cls::Definition as usize];
        if def_cls != NONE_IDX && self.is_class_ref_at(at, def_cls) {
            return true;
        }
        self.bytes_eq_at(at, &[0xff, 0xff])
            && self.bytes_eq_at(at + 4, &[20, 0])
            && self.bytes_eq_at(at + 6, b"CComponentDefinition")
    }

    /// The word at `at` can start an object read: a null, an escape, a class
    /// declaration, a class-ref to a class already known, or a back-ref
    /// below the next slot.
    fn plausible_list_tag(&mut self, at: u64) -> bool {
        let Ok(t) = self.c.u16_at(at) else {
            return false;
        };
        if t == 0 || t == 0x7fff || t == 0xffff {
            return true;
        }
        if t & 0x8000 != 0 {
            return self.entry((t & 0x7fff) as u32).kind == K_CLASS;
        }
        (t as u32) < self.next_slot()
    }

    /// After a definition fails, find the next one.
    ///
    /// The signature is strong enough to use: a class-ref (or declaration) for
    /// `CComponentDefinition`, then the entity preamble, then the base block,
    /// then a small layer count, then a layer record. A definition that cannot
    /// be read costs one definition instead of the rest of the file.
    fn resync_definition(&mut self, from: u64) -> Result<bool> {
        let def_cls = self.class_slot[Cls::Definition as usize];
        if def_cls == NONE_IDX || def_cls >= 0x7fff {
            return Ok(false);
        }
        let needle = (0x8000u16 | def_cls as u16).to_le_bytes();
        let mut at = from + 1;
        // Bound the work, not just the attempts: a two-byte needle has many
        // false positives, so without a byte budget a corrupt file could ask
        // for terabytes of scanning. The budget is the file's, not the
        // call's (`RESYNC_BUDGET`): every failed definition starts a scan.
        let budget_end = core::cmp::min(self.c.end, at.saturating_add(self.resync_left));
        let r = self.resync_scan(&needle, &mut at, budget_end);
        self.resync_left = self
            .resync_left
            .saturating_sub(core::cmp::min(at, budget_end).saturating_sub(from));
        r
    }

    fn resync_scan(&mut self, needle: &[u8; 2], at: &mut u64, budget_end: u64) -> Result<bool> {
        let mut tries = 0;
        while tries < 1 << 16 && *at < budget_end {
            tries += 1;
            self.c.seek(*at)?;
            let Some(hit) = self.c.find(needle, budget_end - *at)? else {
                *at = budget_end;
                return Ok(false);
            };
            *at = hit + 1;
            // Validate: preamble, base block, plausible layer count, layer.
            let mut p = hit + 2;
            let attr = self.c.u16_at(p)?;
            p += 2;
            if attr == 0x7fff {
                p += 4;
            }
            if self.wide_entity {
                let Some(mask) = self.byte_at(p) else {
                    continue;
                };
                p += 1 + mask.count_ones() as u64;
                p += 22;
            } else {
                p += 20;
            }
            let Ok(nlayers) = self.c.u32_at(p) else {
                continue;
            };
            if nlayers == 0 || nlayers > 16 {
                continue;
            }
            if !self.layer_record_at(p + 4, false) {
                continue;
            }
            self.c.seek(hit)?;
            return Ok(true);
        }
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Saved scenes.
//
// This is the part no reader does. Every existing implementation returns zero
// pages for every legacy file in this corpus, and the reason is a single wrong
// assumption: that a scene record's capture flags are one of two values. The
// Python reader only accepts 0xc00 and 0xc20; the files here carry 0x7f, 0x7e,
// 0xfef and 0xffff. Measured across the corpus, legacy pages use exactly the
// same bits as the VFF `ViewPageFlags` in `pages::page_flag` - `SUSceneFlags`
// - and the proof is bit 0: `pair_scene-no-camera_legacy2017_1scene.skp`
// carries 0x7e, and its matched VFF twin reads back with no camera at all.
// Older writers store 0xffff, which is "everything", undefined bits included.
//
// The record is:
//
//     entity preamble            (attribute pointer, + pid bytes at CEntity 5)
//     utf16 name
//     utf16 description
//     u32   capture flags
//     CCamera object             (present even when bit 0 is clear in 0x7e)
//     ... one block per remaining capture bit ...
//
// The blocks after the camera are not decoded, and decoding them is not
// necessary: a captured style is a serialized `CSkpStyle` carrying its own
// PNG, so record sizes vary from 657 bytes to 17 KB in the same file and no
// fixed stride exists. Instead the *next* record is located by its own
// signature - the same two-byte class-ref for `CViewPage` each time, followed
// by a preamble, a plausible name, a second string and a sane flag word. The
// class-ref byte pair is learned from the first record that follows the
// declaration, so this needs no knowledge of the archive's slot numbering and
// works whether or not the entity walk succeeded.
//
// Checked against the corpus: on all 15 legacy files the declared count and
// the number of records found agree, and on the 13 whose filename encodes a
// scene count it is that count - 26, 15, 5, 2, 2, 2, 1, 1, 1, 1, 0, 0, 0. The
// two commercial packs read 6 ('White', 'Daytime', 'Afternoon',
// 'Late afternoon', 'Sunset', 'Night') and 0. And on the seven matched
// legacy/VFF pairs, the names, eyes, targets, fields of view, projections,
// parallel-projection heights and documented capture bits all agree exactly
// with what `pages.rs` reads off the VFF twin - see
// `legacy_and_vff_pairs_agree_on_scenes`.
// ---------------------------------------------------------------------------

/// The bit that says a page restores its camera. Same bit as VFF.
const PAGE_USE_CAMERA: u32 = crate::pages::page_flag::CAMERA_LOCATION;
const PAGE_USE_LAYERS: u32 = crate::pages::page_flag::VISIBLE_LAYERS;
/// The bit that says a page restores a style. Same bit as VFF, where it is
/// what puts a `ViewPageRenderingOptions` child in the record.
const PAGE_USE_STYLE: u32 = crate::pages::page_flag::DRAWING_STYLE;

struct PageHead {
    tag: u16,
    name: String,
    description: String,
    flags: u32,
    /// Offset just past the flag word.
    body: u64,
}

/// Where the page list starts, and how many pages it declares.
fn find_page_list(c: &mut Cur, from: Option<u64>) -> Result<Option<(u64, u32)>> {
    let lo = match from {
        Some(f) if f > c.start && f < c.end => f,
        _ => core::cmp::max(c.start, c.end.saturating_sub(PAGE_WINDOW)),
    };
    let save = c.at();
    c.seek(lo)?;
    let found = class_decl(c, b"CViewPage", c.end - lo)?;
    c.seek(save)?;
    let Some(decl) = found else { return Ok(None) };
    if decl < c.start + 4 {
        return Ok(None);
    }
    // The page manager is `<u32 thumb w><u32 thumb h><u32 reserved><u32
    // count>` and then the pages, so the count is the word immediately before
    // the first page - which is the class declaration, because the page list
    // is the only place `CViewPage` is ever used.
    let count = c.u32_at(decl - 4)?;
    if count == 0 || count > 10_000 {
        return Ok(None);
    }
    Ok(Some((decl, count)))
}

fn read_page_head(
    c: &mut Cur,
    wide_entity: bool,
    at: u64,
    want_tag: Option<u16>,
) -> Option<PageHead> {
    let save = c.at();
    let r = (|| -> Option<PageHead> {
        c.seek(at).ok()?;
        let tag = c.u16().ok()?;
        if let Some(w) = want_tag {
            if tag != w {
                return None;
            }
        }
        if tag == 0xffff {
            let _schema = c.u16().ok()?;
            if c.u16().ok()? != 9 {
                return None;
            }
            let n = c.peek_slice(9).ok()?;
            if n != b"CViewPage" {
                return None;
            }
            c.skip(9).ok()?;
        } else if tag == 0x7fff {
            let big = c.u32().ok()?;
            if big & 0x8000_0000 == 0 {
                return None;
            }
        } else if tag & 0x8000 == 0 {
            return None;
        }
        // Entity preamble. A page's attribute pointer is null in every file
        // measured; a non-null one would be a class-ref or a back-ref, which
        // this cannot follow without the slot table, so require the null and
        // let a page with attributes be skipped rather than mis-read.
        if c.u16().ok()? != 0 {
            return None;
        }
        if wide_entity {
            let mask = c.u8().ok()?;
            c.skip(mask.count_ones() as u64).ok()?;
        }
        let name = c.utf16().ok()?;
        if name.is_empty() || name.chars().count() > 120 {
            return None;
        }
        let description = c.utf16().ok()?;
        if description.chars().count() > 1000 {
            return None;
        }
        let flags = c.u32().ok()?;
        // A capture mask. Zero would mean a tab that restores nothing, which
        // SketchUp does not write, and anything above 16 bits is not a mask.
        if flags == 0 || flags > 0xffff {
            return None;
        }
        Some(PageHead {
            tag,
            name,
            description,
            flags,
            body: c.at(),
        })
    })();
    let _ = c.seek(save);
    r
}

/// A scene whose camera half is a freshly constructed `CCamera`.
///
/// This mirrors `pages::with_default_camera`, which is private to that module.
/// The values are SketchUp's own constructor defaults - 1 inch near, 1000
/// inches far, 35 degrees - and they matter because `SceneDef::default()`
/// would describe a zero-degree parallel camera with a zero-height frustum,
/// which no renderer can draw and no file ever says. Keeping the two paths on
/// the same floor is what makes `pair_scene-no-camera`'s two halves agree.
fn default_camera_scene() -> SceneDef {
    SceneDef {
        perspective: true,
        fov_deg: 35.0,
        fov_is_height: true,
        near: 1.0 * INCH_TO_MM,
        far: 1000.0 * INCH_TO_MM,
        ..SceneDef::default()
    }
}

/// The camera a page restores, and the offset just past it.
///
/// `CCamera` has no entity preamble. 176 bytes for an empty camera name:
/// three points, a near and a far that are the constructor's own 1 and 1000
/// inches and describe nothing (`pages::derive_near_far` is what a viewer
/// should use), a perspective byte, the field of view, the parallel-projection
/// height, 32 reserved bytes, a word, a name, and 33 bytes more.
fn read_page_camera(c: &mut Cur, at: u64) -> Option<(SceneDef, u64)> {
    let default = default_camera_scene;
    let save = c.at();
    let r = (|| -> Option<(SceneDef, u64)> {
        c.seek(at).ok()?;
        let tag = c.peek_u16().ok()?;
        if tag == 0 {
            c.skip(2).ok()?;
            return Some((default(), c.at()));
        }
        if tag == 0xffff {
            c.skip(4).ok()?;
            let n = c.u16().ok()? as u64;
            if n > 40 {
                return None;
            }
            c.skip(n).ok()?;
        } else if tag == 0x7fff {
            c.skip(2).ok()?;
            let big = c.u32().ok()?;
            if big & 0x8000_0000 == 0 {
                // A back-ref: the camera lives somewhere we did not walk.
                return Some((default(), c.at()));
            }
        } else if tag & 0x8000 != 0 {
            c.skip(2).ok()?;
        } else {
            c.skip(2).ok()?;
            return Some((default(), c.at()));
        }
        let eye = c.f64s::<3>().ok()?;
        let target = c.f64s::<3>().ok()?;
        let up = c.f64s::<3>().ok()?;
        let near = c.f64().ok()?;
        let far = c.f64().ok()?;
        let perspective = c.u8().ok()? != 0;
        let fov = c.f64().ok()?;
        let ortho = c.f64().ok()?;
        c.skip(32).ok()?;
        let _ = c.u16().ok()?;
        let _camera_name = c.utf16().ok()?;
        c.skip(33).ok()?;
        if !(eye.iter().all(|v| v.is_finite()) && target.iter().all(|v| v.is_finite())) {
            return None;
        }
        let scene = SceneDef {
            eye: Some([
                eye[0] * INCH_TO_MM,
                eye[1] * INCH_TO_MM,
                eye[2] * INCH_TO_MM,
            ]),
            target: Some([
                target[0] * INCH_TO_MM,
                target[1] * INCH_TO_MM,
                target[2] * INCH_TO_MM,
            ]),
            up: Some(up),
            fov_deg: fov,
            // Legacy `CCamera` stores no equivalent of VFF's
            // `fov_is_height` tag; SketchUp's own default is the vertical
            // field of view, and every file measured here round-trips that
            // way through the modern path.
            fov_is_height: true,
            perspective,
            ortho_height: ortho * INCH_TO_MM,
            near: near * INCH_TO_MM,
            far: far * INCH_TO_MM,
            ..SceneDef::default()
        };
        Some((scene, c.at()))
    })();
    let _ = c.seek(save);
    r
}

impl<'a> Archive<'a> {
    /// Every `CSkpStyle` record in the document tail, in file order.
    ///
    /// Found by signature, like the page list, because the entity walk has
    /// ended by here and the slot numbering is gone. The signature is weak on
    /// purpose - an empty string, a word, two strings - and the settings run
    /// is what confirms it: a count between 20 and 200, then that many
    /// entries whose ids strictly increase, whose `present` word is 0 or 1 and
    /// whose type is one this reader knows. Measured, that accepts exactly the
    /// records SketchUp wrote and nothing else: 0 hits in all five VFF files,
    /// which carry no such record at all, and 2, 3 or 7 in each legacy file.
    fn style_records(&mut self, from: Option<u64>) -> Vec<StyleRec> {
        let lo = match from {
            Some(f) if f > self.c.start && f < self.c.end => f,
            _ => core::cmp::max(self.c.start, self.c.end.saturating_sub(PAGE_WINDOW)),
        };
        let save = self.c.at();
        let mut out: Vec<StyleRec> = Vec::new();
        let mut p = lo;
        // Chunked rather than one `find` per candidate: the leading empty
        // string is a four-byte pattern that occurs 5,566 times in the last
        // 8 MB of the restaurant pack, and seeking to each one in turn costs a
        // window refill apiece. Reading the region once in 32 KB pieces and
        // collecting the candidates first keeps the whole scan at one pass
        // over the tail, and keeps a corrupt file from turning it into
        // thousands.
        const CHUNK: usize = 1 << 15;
        while p + 16 < self.c.end && out.len() < 256 {
            // Three bytes of overlap so a pattern straddling two pieces is
            // seen by the first of them; candidates at or past `CHUNK` belong
            // to the next piece and are left to it.
            let span = core::cmp::min((self.c.end - p) as usize, CHUNK + 3);
            let hits: Vec<u64> = {
                if self.c.seek(p).is_err() {
                    break;
                }
                match self.c.peek_slice(span) {
                    Ok(buf) => buf
                        .windows(4)
                        .enumerate()
                        .take(CHUNK)
                        .filter(|(_, w)| *w == [0xff, 0xfe, 0xff, 0x00])
                        .map(|(i, _)| p + i as u64)
                        .collect(),
                    Err(_) => break,
                }
            };
            for at in hits {
                if let Some(r) = self.read_style_record(at) {
                    out.push(r);
                }
            }
            p += CHUNK as u64;
        }
        let _ = self.c.seek(save);
        out
    }

    /// One record at `at`, which points at its leading empty string.
    fn read_style_record(&mut self, at: u64) -> Option<StyleRec> {
        let save = self.c.at();
        let r = (|| -> Option<StyleRec> {
            self.c.seek(at + 4).ok()?;
            // `SkpStyleLoadedFrom`, 3 in every record measured. Bounded rather
            // than required, so that a writer that uses another value is read
            // instead of being silently skipped.
            if self.c.u32().ok()? > 16 {
                return None;
            }
            let name_at = self.c.at();
            let name = self.c.utf16().ok()?;
            if name.is_empty() || name.chars().count() > 120 {
                return None;
            }
            let description = self.c.utf16().ok()?;
            if description.chars().count() > 1000 {
                return None;
            }
            let count = self.c.u32().ok()?;
            if !(20..=200).contains(&count) {
                return None;
            }
            let mut opts: Vec<(u32, u32)> = Vec::new();
            let mut prev = -1i64;
            let mut parsed = 0u32;
            for _ in 0..count {
                let id = self.c.u32().ok()?;
                if id as i64 <= prev {
                    break;
                }
                let present = self.c.u32().ok()?;
                if present > 1 {
                    break;
                }
                let kind = self.c.u32().ok()?;
                let Some(width) = style_value_width(kind) else {
                    break;
                };
                if STYLE_IDS.contains(&id) && present == 1 && (width == 1 || width == 4) {
                    let v = if width == 1 {
                        self.c.u8().ok()? as u32
                    } else {
                        self.c.u32().ok()?
                    };
                    opts.push((id, v));
                } else {
                    self.c.skip(width).ok()?;
                }
                prev = id as i64;
                parsed += 1;
            }
            // The run stops before its declared count on every real record -
            // one id past the sky block changes shape - so the test is how
            // much of it parsed, not whether all of it did.
            if parsed < 20 {
                return None;
            }
            Some(StyleRec {
                at: name_at,
                name,
                opts,
            })
        })();
        let _ = self.c.seek(save);
        r
    }

    /// The scene tabs. See the block comment above for the method.
    fn scan_pages(&mut self, from: Option<u64>) -> Vec<SceneDef> {
        let Ok(Some((decl, count))) = find_page_list(&mut self.c, from) else {
            return Vec::new();
        };
        // A tab's captured style, and the manager's own record, are both past
        // the page list - so when the caller has not already scanned for them
        // (the scene-only read has not), start here rather than at the floor
        // `PAGE_WINDOW` sets. On the 518 MB pack that is the difference
        // between scanning half a megabyte and scanning eight.
        if self.style_recs.is_empty() {
            self.style_recs = self.style_records(Some(decl));
        }
        let mut out: Vec<SceneDef> = Vec::new();
        // Per record: where its camera ended, and where the next record began.
        // The span between them holds the blocks this reader does not decode,
        // and the hidden-tag list if the record has one.
        let mut spans: Vec<(u64, u64)> = Vec::new();
        let mut tag: Option<u16> = None;
        let mut at = decl;
        let mut after_camera = decl;
        while out.len() < count as usize {
            let head = if out.is_empty() {
                read_page_head(&mut self.c, self.wide_entity, at, None)
            } else {
                // Scan forward for the next record. Byte-granular because the
                // blocks in between carry a serialized style with an embedded
                // image and so have no stride: records in one real file run
                // from 657 bytes to 17 KB.
                let mut found = None;
                let stop = core::cmp::min(self.c.end, after_camera.saturating_add(PAGE_SPAN));
                let mut p = after_camera;
                while p + 8 < stop {
                    // The class-ref is a known two-byte pattern once the
                    // first repeat has been seen, so most positions are
                    // rejected by one 16-bit compare rather than by a full
                    // header parse.
                    if let Some(want) = tag {
                        if self.c.u16_at(p).map(|t| t != want).unwrap_or(true) {
                            p += 1;
                            continue;
                        }
                    }
                    if let Some(h) = read_page_head(&mut self.c, self.wide_entity, p, tag) {
                        found = Some((p, h));
                        break;
                    }
                    p += 1;
                }
                match found {
                    Some((p, h)) => {
                        if let Some(last) = spans.last_mut() {
                            last.1 = p;
                        }
                        Some(h)
                    }
                    None => None,
                }
            };
            let Some(head) = head else { break };
            // Record 0 brings the class declaration with it; records 1..n
            // reuse the class-ref, which is learned from the first of them and
            // then required of all the rest.
            tag = if out.is_empty() { None } else { Some(head.tag) };
            let mut scene = match read_page_camera(&mut self.c, head.body) {
                Some((s, end)) => {
                    after_camera = end;
                    s
                }
                None => {
                    after_camera = head.body;
                    default_camera_scene()
                }
            };
            if head.flags & PAGE_USE_CAMERA == 0 {
                // The tab does not restore a camera, so the one stored in the
                // record is not what clicking the tab does. A legacy record
                // carries the camera anyway, where a VFF page simply omits
                // the `ViewPageCamera` child - so drop it here and fall back
                // to the same floor `pages::with_default_camera` uses, which
                // is what makes the two containers agree on
                // `pair_scene-no-camera`.
                scene = default_camera_scene();
            }
            scene.name = head.name;
            scene.description = head.description;
            scene.flags = head.flags;
            out.push(scene);
            spans.push((
                after_camera,
                core::cmp::min(self.c.end, after_camera + PAGE_SPAN),
            ));
            at = after_camera;
        }
        for i in 0..out.len() {
            if out[i].flags & PAGE_USE_LAYERS != 0 {
                let (a, b) = spans[i];
                out[i].hidden_layer_ids = self.find_hidden_layers(a, b);
            }
            // Which style the tab selects. The record it captures sits in the
            // same span as the hidden-tag list, between the camera and the
            // next page; its *name* is what identifies the style, because the
            // style list is reconstructed from these names (see
            // `style_records`).
            //
            // The *first* record in the span, not the last: the final page's
            // span runs to the end of the file and so takes in the manager's
            // own record as well, and on the restaurant pack that would label
            // the Night tab with the document's selected style instead of its
            // own.
            //
            // A page whose span holds no record at all is one that selects a
            // style already written - CArchive gives it a two-byte
            // back-reference rather than a second copy of the object, which is
            // why a fifteen-scene rack on one style has one record and a
            // six-scene restaurant on six styles has six. Inheriting the
            // previous tab's style is what that back-reference means; it is
            // not a guess at a value the file withholds.
            if out[i].flags & PAGE_USE_STYLE != 0 {
                let (a, b) = spans[i];
                out[i].style_id = match self.style_recs.iter().find(|r| r.at >= a && r.at < b) {
                    Some(r) => self.style_id_of(&r.name),
                    None if i > 0 => out[i - 1].style_id,
                    None => None,
                };
            }
        }
        out
    }

    /// The index in `index.styles` of the style with this name.
    fn style_id_of(&self, name: &str) -> Option<u32> {
        let mut id = 0u32;
        let mut seen: Vec<&str> = Vec::new();
        for r in &self.style_recs {
            if seen.contains(&r.name.as_str()) {
                continue;
            }
            if r.name == name {
                return Some(id);
            }
            seen.push(&r.name);
            id += 1;
        }
        None
    }

    /// The tags a page turns off.
    ///
    /// The list is `<u32 count><u16 layer slot>...`, as this project's own
    /// writer emits it, but in a SketchUp-written record it sits behind the
    /// captured style and shadow blocks, which have no fixed size - so it is
    /// found rather than seeked to. Every candidate must declare a count no
    /// larger than the model's tag list and then that many *distinct* slots,
    /// each one an actual `CLayer` object from the model-level list, and the
    /// match must be the only one in the record. Ambiguity yields nothing: a
    /// wrong tag list hides the wrong half of the model, which is worse than
    /// an empty one.
    ///
    /// Verified on `pair_two-scenes_legacy2017_2scenes.skp`, whose first page
    /// has exactly one candidate - `<1><slot 21>`, and slot 21 is
    /// `MediumLayer` - matching its VFF twin's `hidden_layers:
    /// ['MediumLayer']`, while the second page has no candidate at all,
    /// matching its twin's empty list.
    fn find_hidden_layers(&mut self, from: u64, to: u64) -> Vec<u32> {
        if self.model_layers.is_empty() {
            return Vec::new();
        }
        let max = self.model_layers.len() as u32;
        let stop = core::cmp::min(to, self.c.end);
        let mut found: Option<Vec<u32>> = None;
        let mut q = from;
        while q + 6 <= stop {
            let Ok(n) = self.c.u32_at(q) else { break };
            if n >= 1 && n <= max && q + 4 + 2 * n as u64 <= stop {
                let mut ids: Vec<u32> = Vec::with_capacity(n as usize);
                let mut ok = true;
                for j in 0..n as u64 {
                    let Ok(sl) = self.c.u16_at(q + 4 + 2 * j) else {
                        ok = false;
                        break;
                    };
                    if !self.model_layers.contains(&(sl as u32)) {
                        ok = false;
                        break;
                    }
                    let id = self.layer_index(sl as u32);
                    if ids.contains(&id) {
                        ok = false;
                        break;
                    }
                    ids.push(id);
                }
                if ok {
                    if found.is_some() {
                        return Vec::new();
                    }
                    found = Some(ids);
                }
            }
            q += 1;
        }
        found.unwrap_or_default()
    }
}

/// How far past a page record to look for the next one, and how far past the
/// last one to look for its tag list. A captured style with a watermark image
/// is the largest thing that can sit in between; the widest measured is 17 KB.
const PAGE_SPAN: u64 = 1 << 20;

/// `LengthUnit` out of the model's own options block.
///
/// The options are a run of `<utf16 key><u8 type><value>` dictionaries in the
/// archive prefix - the part this reader never walks - so the key is found by
/// its UTF-16 bytes and the value read straight after. Values are SketchUp's
/// own `LengthUnit` enum. Confirmed on all 14 legacy files in the corpus: the
/// 517 MB restaurant pack and the school cafeteria read millimetres, the
/// bear home reads feet, the rest inches. The Python reader reports no unit at
/// all for a legacy file ("there is no known source for the model's unit
/// system here"), so this is strictly more than the oracle has.
fn read_length_unit(c: &mut Cur) -> Option<LengthUnit> {
    const KEY: &[u8] = &[
        b'L', 0, b'e', 0, b'n', 0, b'g', 0, b't', 0, b'h', 0, b'U', 0, b'n', 0, b'i', 0, b't', 0,
    ];
    let save = c.at();
    let r = (|| -> Option<LengthUnit> {
        c.seek(c.start).ok()?;
        let at = c.find(KEY, 1 << 18).ok()??;
        c.seek(at + KEY.len() as u64).ok()?;
        if c.u8().ok()? != 0x04 {
            return None;
        }
        match c.u32().ok()? {
            0 => Some(LengthUnit::Inches),
            1 => Some(LengthUnit::Feet),
            2 => Some(LengthUnit::Millimetres),
            3 => Some(LengthUnit::Centimetres),
            4 => Some(LengthUnit::Metres),
            _ => None,
        }
    })();
    let _ = c.seek(save);
    r
}

// ---------------------------------------------------------------------------
// The style a legacy file carries.
//
// A legacy file holds its line-art settings in two places and only one of them
// is the one the viewport reads.
//
// The obvious place is the `CSkpStyle` records in the document tail: each one
// carries a name, a description and a tidy `<u32 id> <u32 present> <u32 type>
// <value>` run with the same numeric ids as a modern file's
// `styles/<name>/style.xml` - 1005 LineExtension, 1006 DrawSilhouettes,
// 1007 SilhouetteWidth, 2001 RenderMode, 2002/2003 the face colours, 4000
// BackgroundColor, 4001 SkyColor. GOALS.md records what happened when this
// project patched those: the name read back in the Styles browser and the
// settings did not take effect, on files this project wrote and on files
// SketchUp itself had written.
//
// What the viewport reads is a flat block of bytes that begins where the model
// `CCamera`'s body ends - one byte per flag, four per integer and four per
// colour, in a fixed order with no tags and no lengths. The offsets were
// measured from the writer's side, by saving the same model twice with one
// option changed and diffing, with a control save first to tell the two GUIDs
// that change on every save apart from a real difference. This inverts that
// table and reads.
//
// # How the table was checked from the reader's side
//
// Measured against the `CSkpStyle` run, which is self-describing - it declares
// a count, an id and a type per entry - and therefore an independent witness
// for the fourteen options whose ids are uniquely known. On every file in the
// corpus that carries a style record, the working copy's run agrees with this
// block on all fourteen: `pair_two-scenes` 14/14, the school cafeteria 14/14
// (`DrawSilhouettes` 0 and `SilhouetteWidth` 5 from both sources), the
// `{19.1.174}` restaurant pack 14/14 at its own schema. Where a *saved* copy
// disagrees it disagrees on one field and in a way that means something: the
// restaurant's first record holds a black `ForegroundColor` where the block
// holds (29, 23, 21), which is an edited style the author never saved.
//
// Two more cross-checks: the colour channel order comes out matching the three
// independent checks in `styles.rs` (a pale blue `SkyColor` of (198, 205, 208)
// and a khaki `GroundColor` of (210, 208, 185), not the two swapped), and all
// seven matched legacy/VFF pairs agree with what `styles::parse_rendering_options`
// reads off the VFF twin - see `legacy_and_vff_pairs_agree_on_rendering_options`.
//
// # Where the block starts, and why that is not one number
//
// Two things move it, and both are read out of the version map rather than
// guessed from the writer version, for the same reason the rest of this module
// does: the version map is the number the writer branched on.
//
// * **`CEntity` schema.** The block starts 176 bytes past the camera's body at
//   schema 5 and 175 at schema 3. One byte, and this reader does not know what
//   it is: a page's `CCamera` is 176 bytes in both (see `read_page_camera`), so
//   the extra byte is between the camera and the block rather than inside the
//   camera, and the bytes either side of the boundary are zero in every file so
//   content cannot place it. What is measured is the correlation, on twelve
//   files: 175 on `{13.0.1}`, `{15.3.x}` and `{16.0.1}`, all of which are
//   `CEntity` 3; 176 on `{17.3.116}`, `{18.0.1}` and `{19.1.174}`, all
//   `CEntity` 5.
// * **`CRenderingOptions` schema.** 36 through SketchUp 2017, 37 in 2018, 38
//   in 2019. Schema 38 inserts one byte before `EdgeDisplayMode` and five more
//   (a colour and a flag) at offset 163, which is how a fixed table read off a
//   2017 file lands `DisplayWatermarks` and `DrawBackEdges` on the wrong bytes
//   of the 518 MB pack. Established by byte-diffing the same model saved as
//   2013 and as 2019 - the two restaurant files - whose option blocks differ
//   only by those insertions and by the one setting the author changed.
//   Schema 37 needs no shift: every field this reads is where schema 36 puts
//   it, which the plausibility check below confirms on the `{18.0.1}` file.
//
// Both gates are then *checked* rather than trusted. `plausible` requires
// twenty flag bytes to be 0 or 1, nine integers to be in range, six colours to
// be opaque and the ground transparency to be a percentage - about a hundred
// bits of constraint, which no neighbouring offset satisfies. If the gated
// position fails, a +-8 byte window is searched and an unambiguous single
// candidate accepted; if that fails too, nothing is returned and the style
// comes back at `StyleDef::default` rather than wrong.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The named styles, and which one each tab selects.
//
// A `CSkpStyle` record is the one *tagged* thing in the whole legacy container:
// after its guid, an empty string and a small word, it holds a name, a
// description, and then `<u32 count>` settings of the form `<u32 id>
// <u32 present> <u32 type> <value>`, where the type gives the value's width.
// The ids are the same numeric ids a modern file's `styles/<name>/style.xml`
// uses, which `styles.rs` recovered over 33 files.
//
// This is *not* where the viewport's settings come from - see
// `read_rendering_options` for the block that is - but it is where the names
// are, and SketchUp's own style list is exactly the set of names these records
// carry. Measured against `viewer/oracle/su-ref/*/facts.json`, which is
// SketchUp itself reporting on five of these files:
//
//     file                     records   distinct names   SketchUp says
//     03 급식실 Cafeteria            3              2             2 styles
//     2. Food 음식 (2013)            2              1             1 style
//     2. Food 음식 (2019)            2              1             1 style
//     1. Cozy Western (2013)        7              6             6 styles
//     1. Cozy Western (2019)        7              6             6 styles
//
// and the **last** record names the selected style in all five - the
// cafeteria's `[Architectural Design Style]1`, the restaurant's
// `[Construction Documentation Style]2`. The restaurant's first six records
// sit one inside each page record, in page order, and their names are
// `[Construction Documentation Style]` 3, 2, 5, 4, 1, 6 - which is exactly
// what `facts.json` reports for its six tabs White, Daytime, Afternoon, Late
// afternoon, Sunset and Night. That is what `PAGE_USE_STYLE` reads.
//
// # What a record's settings are used for, and what they are not
//
// Each style's [`StyleDef`] starts from the options block - the document's own
// complete state - and is then overlaid with the eleven settings this record
// states that have both a known id and a field in the contract. Of the fifty
// ids a record carries, fifteen are pinned uniquely and the rest collapse into
// two groups whose members hold the same value in every file here, exactly as
// `styles.rs` found on the modern side. So a non-active style's eleven overlaid
// fields are the file's own statement and the other eleven are the active
// style's; that is better than factory defaults and it is said out loud here
// rather than hidden.
//
// The ids were pinned by intersecting each id's value against each offset in
// the options block over the 17 files in this corpus - the block being already
// confirmed against SketchUp itself. Three of them correct what this project
// had written down from diffing bundled styles: `1005` is `LineExtension` and
// `1011` is `LineEndWidth`, not `1010`/`1011` as `RENDER.md` section 7 has
// them, and `1000` is `EdgeDisplayMode`.
//
// # What is still missing
//
// A page restores a *complete* set of rendering options, not just a style
// reference, and they can differ from the style's saved ones: the cafeteria's
// style has `DrawSilhouettes` off at width 5 while its single tab turns them
// on. Those values are in the page's captured record and parse here, but
// `SceneDef` has only `style_id` to put them in - as does `pages.rs` on the
// modern side, which reads `ViewPageStylePIDRef` and drops
// `ViewPageRenderingOptions`. Closing that needs a field in `model.rs`, which
// this module does not own.
// ---------------------------------------------------------------------------

/// One `CSkpStyle` record found in the document tail.
struct StyleRec {
    /// Offset of the record's name string, which is what places the record
    /// inside a page's span.
    at: u64,
    name: String,
    /// `(id, little-endian value)` for the ids this reader knows, in file
    /// order. A colour's four bytes are the four bytes of the value, red
    /// first, so they need no reordering.
    opts: Vec<(u32, u32)>,
}

/// Payload width by the record's own type word. Types 1, 4 and 7 are the ones
/// that occur; the rest are from `styles.rs`'s variant table, where 1 is bool,
/// 4 int, 5 colour-as-int, 6 float and 7 double. An unrecognised type ends the
/// run rather than being stepped over by a guess.
fn style_value_width(t: u32) -> Option<u64> {
    match t {
        0 => Some(0),
        1 => Some(1),
        2 => Some(2),
        3 | 4 | 5 | 6 => Some(4),
        7 | 8 => Some(8),
        _ => None,
    }
}

/// The ids whose meaning is established and which have a field in [`StyleDef`].
///
/// Fifteen ids are pinned uniquely over this corpus; these eleven are the ones
/// the contract has somewhere to put. The four left out are `4001 SkyColor`,
/// `4002 DrawHorizon`, `4003 GroundColor` and `4005 GroundTransparency`, which
/// are sky-and-ground settings with no field here - the options block reads
/// them too and they go the same nowhere.
const STYLE_IDS: &[u32] = &[
    1000, 1005, 1006, 1007, 1009, 1011, 1014, 2001, 2002, 2003, 4000,
];

/// Overlay one record's settings onto a style.
fn apply_style_opts(s: &mut StyleDef, opts: &[(u32, u32)]) {
    let rgba = |v: u32| [v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8];
    for (id, v) in opts {
        match id {
            1000 => s.edge_display_mode = *v as i32,
            1005 => s.line_extension = *v as i32,
            1006 => s.draw_silhouettes = *v != 0,
            1007 => s.silhouette_width = *v as i32,
            1009 => s.depth_cue_width = *v as i32,
            1011 => s.line_end_width = *v as i32,
            1014 => s.foreground = rgba(*v),
            2001 => s.render_mode = *v as i32,
            2002 => s.face_front = rgba(*v),
            2003 => s.face_back = rgba(*v),
            4000 => s.background = rgba(*v),
            _ => {}
        }
    }
}

/// How far into the archive the model camera can sit.
///
/// It is preceded by the version map and the thumbnail PNGs and nothing else,
/// so this is small: 3,327 bytes on the `{16.0.1}` file and 140,148 on the
/// 518 MB restaurant pack, the largest in the corpus. The window is bounded
/// because a file without a camera must not turn into a whole-file scan.
const CAMERA_WINDOW: u64 = 1 << 21;

/// Byte offsets in the `CRenderingOptions` block at schema 36.
///
/// One per [`StyleDef`] field that has one. A name here with no field is read
/// anyway, by [`OptBlock::plausible`], because a byte that must be 0 or 1 is
/// evidence about the block's position whether or not anyone wants its value.
mod ro {
    pub const RENDER_MODE: u64 = 3;
    pub const MODEL_TRANSPARENCY: u64 = 7;
    pub const JITTER_EDGES: u64 = 9;
    pub const EDGE_DISPLAY_MODE: u64 = 10;
    pub const BACKGROUND_COLOR: u64 = 14;
    pub const FOREGROUND_COLOR: u64 = 18;
    pub const DISPLAY_COLOR_BY_LAYER: u64 = 32;
    pub const TEXTURE: u64 = 33;
    pub const EDGE_COLOR_MODE: u64 = 34;
    pub const EXTEND_LINES: u64 = 38;
    pub const LINE_EXTENSION: u64 = 39;
    pub const DRAW_SILHOUETTES: u64 = 43;
    pub const SILHOUETTE_WIDTH: u64 = 44;
    pub const DRAW_DEPTH_QUE: u64 = 48;
    pub const DEPTH_QUE_WIDTH: u64 = 49;
    pub const DRAW_LINE_ENDS: u64 = 53;
    pub const LINE_END_WIDTH: u64 = 54;
    pub const DRAW_PROFILES_ONLY: u64 = 58;
    pub const DRAW_HIDDEN: u64 = 59;
    pub const FACE_FRONT_COLOR: u64 = 64;
    pub const FACE_BACK_COLOR: u64 = 68;
    pub const DISPLAY_FOG: u64 = 90;
    pub const EDGE_TYPE: u64 = 116;
    pub const DISPLAY_SKETCH_AXES: u64 = 120;
    pub const DISPLAY_TEXT: u64 = 121;
    pub const DISPLAY_DIMS: u64 = 122;
    pub const HIDE_CONSTRUCTION_GEOMETRY: u64 = 123;
    pub const SKY_COLOR: u64 = 124;
    pub const GROUND_COLOR: u64 = 132;
    pub const DRAW_HORIZON: u64 = 136;
    pub const DRAW_GROUND: u64 = 137;
    pub const DRAW_UNDERGROUND: u64 = 138;
    pub const GROUND_TRANSPARENCY: u64 = 139;
    pub const DISPLAY_WATERMARKS: u64 = 181;
    pub const DRAW_BACK_EDGES: u64 = 190;

    /// Every flag byte in the block, for the plausibility check.
    pub const FLAGS: [u64; 20] = [
        MODEL_TRANSPARENCY,
        JITTER_EDGES,
        DISPLAY_COLOR_BY_LAYER,
        TEXTURE,
        EXTEND_LINES,
        DRAW_SILHOUETTES,
        DRAW_DEPTH_QUE,
        DRAW_LINE_ENDS,
        DRAW_PROFILES_ONLY,
        DRAW_HIDDEN,
        DISPLAY_FOG,
        DISPLAY_SKETCH_AXES,
        DISPLAY_TEXT,
        DISPLAY_DIMS,
        HIDE_CONSTRUCTION_GEOMETRY,
        DRAW_HORIZON,
        DRAW_GROUND,
        DRAW_UNDERGROUND,
        DISPLAY_WATERMARKS,
        DRAW_BACK_EDGES,
    ];

    /// Every colour, all of which are opaque in every file measured.
    pub const COLORS: [u64; 6] = [
        BACKGROUND_COLOR,
        FOREGROUND_COLOR,
        FACE_FRONT_COLOR,
        FACE_BACK_COLOR,
        SKY_COLOR,
        GROUND_COLOR,
    ];

    /// Integers with a closed set of legal values. `RenderMode` is
    /// 0 Wireframe, 1 HiddenLine, 2 Shaded, 3 a colour-set mode and
    /// 5 Monochrome, recovered in `RENDER.md` by diffing 111 bundled styles;
    /// 4 does not occur.
    pub const ENUMS: [(u64, &[u32]); 4] = [
        (RENDER_MODE, &[0, 1, 2, 3, 5]),
        (EDGE_DISPLAY_MODE, &[0, 1, 2]),
        (EDGE_COLOR_MODE, &[0, 1, 2]),
        (EDGE_TYPE, &[0, 1, 2]),
    ];

    /// Integers that are pixel widths, so one byte of a four-byte field.
    pub const WIDTHS: [u64; 5] = [
        LINE_EXTENSION,
        SILHOUETTE_WIDTH,
        DEPTH_QUE_WIDTH,
        LINE_END_WIDTH,
        GROUND_TRANSPARENCY,
    ];
}

/// The block's position and the schema's insertions, resolved once.
#[derive(Clone, Copy)]
struct OptBlock {
    base: u64,
    /// `(from, extra)` pairs, ascending: an offset at or past `from` sits
    /// `extra` bytes later than the schema-36 table says. Empty at schema 36
    /// and 37.
    bands: &'static [(u64, u64)],
}

/// Schema 38's two insertions: one byte ahead of `EdgeDisplayMode` and five
/// more - a colour and a flag - ahead of offset 163.
const RO38_BANDS: &[(u64, u64)] = &[(10, 1), (163, 6)];

impl OptBlock {
    fn at(&self, o: u64) -> u64 {
        let mut extra = 0;
        for (from, add) in self.bands {
            if o >= *from {
                extra = *add;
            }
        }
        self.base + o + extra
    }

    fn flag(&self, c: &mut Cur, o: u64) -> Option<u8> {
        c.byte_at(self.at(o))
    }

    /// A four-byte little-endian integer. The writer's table calls these one
    /// byte each, which is true of the only byte that ever varies; they are
    /// read full width here so that a value above 255 shows up as the misread
    /// it would be rather than being silently truncated.
    fn int(&self, c: &mut Cur, o: u64) -> Option<u32> {
        c.u32_at(self.at(o)).ok()
    }

    fn color(&self, c: &mut Cur, o: u64) -> Option<[u8; 4]> {
        let at = self.at(o);
        // Red first, not an ARGB integer - the order `styles.rs` settles with
        // three independent checks, and `model.dat` stores the same four bytes
        // in the same order, so the two containers need no conversion between
        // them.
        Some([
            c.byte_at(at)?,
            c.byte_at(at + 1)?,
            c.byte_at(at + 2)?,
            c.byte_at(at + 3)?,
        ])
    }

    /// Whether the block really starts here.
    fn plausible(&self, c: &mut Cur) -> bool {
        for o in ro::FLAGS {
            match self.flag(c, o) {
                Some(0) | Some(1) => {}
                _ => return false,
            }
        }
        for (o, allowed) in ro::ENUMS {
            match self.int(c, o) {
                Some(v) if allowed.contains(&v) => {}
                _ => return false,
            }
        }
        for o in ro::WIDTHS {
            match self.int(c, o) {
                Some(v) if v <= 255 => {}
                _ => return false,
            }
        }
        if !matches!(self.int(c, ro::GROUND_TRANSPARENCY), Some(v) if v <= 100) {
            return false;
        }
        for o in ro::COLORS {
            match self.color(c, o) {
                Some(rgba) if rgba[3] == 0xff => {}
                _ => return false,
            }
        }
        true
    }

    fn read(&self, c: &mut Cur) -> StyleDef {
        let mut s = StyleDef::default();
        let flag = |c: &mut Cur, o: u64| self.flag(c, o) == Some(1);
        let int = |c: &mut Cur, o: u64| self.int(c, o).unwrap_or(0) as i32;
        s.render_mode = int(c, ro::RENDER_MODE);
        s.draw_silhouettes = flag(c, ro::DRAW_SILHOUETTES);
        s.silhouette_width = int(c, ro::SILHOUETTE_WIDTH);
        s.extend_lines = flag(c, ro::EXTEND_LINES);
        s.line_extension = int(c, ro::LINE_EXTENSION);
        s.jitter_edges = flag(c, ro::JITTER_EDGES);
        s.draw_depth_cue = flag(c, ro::DRAW_DEPTH_QUE);
        s.depth_cue_width = int(c, ro::DEPTH_QUE_WIDTH);
        s.draw_line_ends = flag(c, ro::DRAW_LINE_ENDS);
        s.line_end_width = int(c, ro::LINE_END_WIDTH);
        s.draw_profiles_only = flag(c, ro::DRAW_PROFILES_ONLY);
        s.draw_back_edges = flag(c, ro::DRAW_BACK_EDGES);
        s.edge_display_mode = int(c, ro::EDGE_DISPLAY_MODE);
        s.edge_color_mode = int(c, ro::EDGE_COLOR_MODE);
        s.display_color_by_layer = flag(c, ro::DISPLAY_COLOR_BY_LAYER);
        s.texture_enabled = flag(c, ro::TEXTURE);
        s.display_watermarks = flag(c, ro::DISPLAY_WATERMARKS);
        s.display_sketch_axes = flag(c, ro::DISPLAY_SKETCH_AXES);
        if let Some(v) = self.color(c, ro::BACKGROUND_COLOR) {
            s.background = v;
        }
        if let Some(v) = self.color(c, ro::FOREGROUND_COLOR) {
            s.foreground = v;
        }
        if let Some(v) = self.color(c, ro::FACE_FRONT_COLOR) {
            s.face_front = v;
        }
        if let Some(v) = self.color(c, ro::FACE_BACK_COLOR) {
            s.face_back = v;
        }
        // `id` and `name` have no byte in this block - it is the document's
        // option state and says nothing about which style that state belongs
        // to. `finish` fills both from the `CSkpStyle` records in the tail.
        s
    }
}

/// The document's rendering options: the style a legacy file actually has.
///
/// `None` when the block cannot be located and confirmed, which leaves the
/// caller at `StyleDef::default` - SketchUp's factory values. That matters
/// here rather than being a formality: the factory default is silhouettes on
/// at width 3 while the shipped Hidden Line style has them off at width 1, and
/// the files in this corpus hold both (the restaurant pack: on at width 1; the
/// school cafeteria: off at width 5). Substituting either for the other
/// changes every line in the export, so a style that cannot be read has to
/// stay visibly absent.
fn read_rendering_options(c: &mut Cur, ro_schema: u16, wide_entity: bool) -> Option<StyleDef> {
    let save = c.at();
    let r = (|| -> Option<StyleDef> {
        c.seek(c.start).ok()?;
        let decl = class_decl(c, b"CCamera", CAMERA_WINDOW).ok()??;
        // `0xFFFF <u16 schema> <u16 len> "CCamera"` - the object's own body
        // follows the declaration immediately, as it does for every
        // first-use-declares-the-class record in a CArchive.
        let body = decl.checked_add(4 + 2 + 7)?;
        let nominal = body.checked_add(if wide_entity { 176 } else { 175 })?;
        let bands: &'static [(u64, u64)] = if ro_schema >= 38 { RO38_BANDS } else { &[] };

        let gated = OptBlock {
            base: nominal,
            bands,
        };
        if gated.plausible(c) {
            return Some(gated.read(c));
        }
        // The gate missed - an unmeasured schema, or a camera record that grew
        // again. Search, and insist on exactly one answer: a wrong style is
        // worse than none, and the plausibility check is strong enough that
        // two candidates mean the signal is not there.
        let mut found: Option<OptBlock> = None;
        for bands in [bands, if bands.is_empty() { RO38_BANDS } else { &[] }] {
            for d in -8i64..=8 {
                let base = nominal.checked_add_signed(d)?;
                let cand = OptBlock { base, bands };
                if cand.plausible(c) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(cand);
                }
            }
        }
        let cand = found?;
        Some(cand.read(c))
    })();
    let _ = c.seek(save);
    r
}

// ---------------------------------------------------------------------------
// Public entry points.
// ---------------------------------------------------------------------------

fn major_version(v: &str) -> u32 {
    let digits: String = v
        .trim_start_matches('{')
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().unwrap_or(0)
}

fn stream_of(c: &Container) -> Result<(u64, u64)> {
    c.legacy_stream
        .ok_or(Error::Malformed("not a legacy container"))
}

/// The scene tabs alone, without walking a byte of geometry.
///
/// This is what a viewer wants first: on the 517 MB restaurant pack it reads
/// the version map out of the head of the file and the page list out of the
/// last few hundred kilobytes, and nothing else at all - 18 ms. Each tab comes
/// back with the id of the style it selects, because the `CSkpStyle` records
/// that give those ids are in the same tail. Hidden-tag lists are left empty
/// because resolving a tag reference needs the slot table, which only the full
/// walk builds.
pub fn read_scenes(src: &dyn ByteSource, container: &Container) -> Result<Vec<SceneDef>> {
    let (start, end) = stream_of(container)?;
    let mut c = Cur::new(src, start, end);
    let vm = read_version_map(&mut c);
    let ver = major_version(&container.header.version);
    let mut ar = Archive::new(c, ver, &vm);
    // `scan_pages` scans for the style records itself here: the tabs carry the
    // id of the style they select, and the records are in the same tail the
    // page list is in.
    Ok(ar.scan_pages(None))
}

/// The `CLayerManager` schema the file's version map states, or `None` when
/// the map is not found. 7 or more is SketchUp 2020's container, whose filler
/// before the definition count the walk skips; 2017 to 2019 write 4.
pub fn layer_manager_schema(src: &dyn ByteSource, container: &Container) -> Result<Option<u16>> {
    let (start, end) = stream_of(container)?;
    let vm = read_version_map(&mut Cur::new(src, start, end));
    Ok(vm.found.then_some(vm.layer_manager))
}

/// What every walk of a file shares: the stream, the version map, the anchor
/// and the slot bases to try, in order.
struct Prepared {
    start: u64,
    end: u64,
    ver: u32,
    vm: VersionMap,
    anchor: Anchor,
    bases: Vec<(u32, &'static str)>,
    /// Bytes the version map, anchor and probe read.
    bytes: u64,
}

fn prepare(src: &dyn ByteSource, container: &Container) -> Result<Prepared> {
    let (start, end) = stream_of(container)?;
    let ver = major_version(&container.header.version);

    // The version map and the anchor are cheap and shared by every attempt.
    let (vm, anchor, head_bytes) = {
        let mut c = Cur::new(src, start, end);
        let vm = read_version_map(&mut c);
        let wide = if vm.found { vm.entity >= 5 } else { ver >= 17 };
        let a = find_anchor(&mut c, wide)?;
        (vm, a, c.read_bytes)
    };

    let (learned, candidates, probe_bytes) = {
        let mut ar = Archive::new(Cur::new(src, start, end), ver, &vm);
        let (l, cands) = ar.probe(&anchor);
        (l, cands, ar.c.read_bytes)
    };

    let mut bases: Vec<(u32, &'static str)> = Vec::new();
    if let Some((b, how)) = learned {
        bases.push((b, how));
    }
    for cand in candidates {
        if !bases.iter().any(|(b, _)| *b == cand) {
            bases.push((cand, "definition-list anchor"));
        }
    }
    if bases.is_empty() {
        return Err(Error::Malformed(
            "could not recover the archive's slot base",
        ));
    }
    // Each base is a whole walk; see `MAX_BASES`.
    bases.truncate(MAX_BASES);
    Ok(Prepared {
        start,
        end,
        ver,
        vm,
        anchor,
        bases,
        bytes: head_bytes + probe_bytes,
    })
}

/// Walk the file from each candidate slot base in turn until one walks,
/// emitting definitions as `emit` says. A budget failure ("too many
/// elements") ends the search: another base would read the same elements.
fn walk_file<'a>(
    src: &'a dyn ByteSource,
    p: &Prepared,
    emit: Emit,
    budget: u64,
) -> Result<(Archive<'a>, u32, &'static str)> {
    let mut last: Option<Error> = None;
    let mut resync_left = RESYNC_BUDGET;
    for &(base, how) in &p.bases {
        let mut ar = Archive::new(Cur::new(src, p.start, p.end), p.ver, &p.vm);
        ar.base = base;
        ar.emit = emit;
        ar.budget = budget;
        ar.resync_left = resync_left;
        let r = ar.walk(&p.anchor);
        resync_left = ar.resync_left;
        match r {
            Ok(()) => return Ok((ar, base, how)),
            Err(e @ Error::Malformed("too many elements")) => return Err(e),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or(Error::Malformed("the archive did not walk")))
}

/// Read the whole model.
pub fn read(src: &dyn ByteSource, container: &Container) -> Result<LegacyModel> {
    let p = prepare(src, container)?;
    let (ar, base, how) = walk_file(src, &p, Emit::Batches, u64::MAX)?;
    let mut model = ar.finish(container, base, how);
    model.stats.bytes_read += p.bytes;
    Ok(model)
}

/// The model as topology, in the shape the modern path's `scene::read_model`
/// gives: one `DefGeometry` per definition (`Definition::pid` is its archive
/// slot, which is what an instance's `def_pid` names) and one for the root.
///
/// Unlike `read`, nothing is computed from the placements - no bounds, no
/// reachability, no instance tables - so the work is linear in the file
/// however its definitions place each other. Hidden faces are kept, flagged.
/// Material and layer references are indices into `read_index`'s lists.
/// `ReadOptions::max_elements` applies as on the modern path; the other
/// options do not.
pub fn read_model(
    src: &dyn ByteSource,
    container: &Container,
    options: &ReadOptions,
) -> Result<(Model, ReadStats)> {
    let (m, _, stats) = read_model_and_index(src, container, options)?;
    Ok((m, stats))
}

/// The materials (textures as inline byte ranges), layers, styles and scenes,
/// without the placements: `bbox` is `None` and scenes keep the near and far
/// planes the file stores. Walks the file once, building no geometry.
pub fn read_index(src: &dyn ByteSource, container: &Container) -> Result<SceneIndex> {
    let p = prepare(src, container)?;
    let (mut ar, base, how) = walk_file(src, &p, Emit::Nothing, u64::MAX)?;
    let (index, _) = ar.index_and_stats(container, base, how);
    Ok(index)
}

/// `read_model` and `read_index` from one walk.
pub fn read_model_and_index(
    src: &dyn ByteSource,
    container: &Container,
    options: &ReadOptions,
) -> Result<(Model, SceneIndex, ReadStats)> {
    let p = prepare(src, container)?;
    let budget = options.max_elements.unwrap_or(u64::MAX);
    let (mut ar, base, how) = walk_file(src, &p, Emit::Topology, budget)?;
    let (index, mut stats) = ar.index_and_stats(container, base, how);
    stats.bytes_read += p.bytes;
    let root = ar
        .topo_root
        .take()
        .ok_or(Error::Malformed("the root entity list was not read"))?;
    let definitions = core::mem::take(&mut ar.topo_defs);
    let mut flags = [0u64; 4];
    for g in core::iter::once(&root).chain(definitions.iter().map(|d| &d.geom)) {
        for f in &g.edge_flags {
            count_edge_flags(&mut flags, *f);
        }
    }
    stats.edge_flags = flags;
    let model = Model {
        writer_version: container.header.version.clone(),
        is_vff: false,
        root,
        definitions,
        sections: Default::default(),
        bytes_read: stats.bytes_read,
    };
    Ok((model, index, stats))
}

/// One edge's flags into `ReadStats::edge_flags`: drawable, hidden, soft,
/// smooth.
fn count_edge_flags(hist: &mut [u64; 4], f: u8) {
    if crate::model::edge_flag::is_drawable(f) {
        hist[0] += 1;
    }
    if f & crate::model::edge_flag::HIDDEN != 0 {
        hist[1] += 1;
    }
    if f & crate::model::edge_flag::SOFT != 0 {
        hist[2] += 1;
    }
    if f & crate::model::edge_flag::SMOOTH != 0 {
        hist[3] += 1;
    }
}

impl<'a> Archive<'a> {
    /// Everything the index says that the placements do not decide: the unit,
    /// styles, scenes, layers and materials, and the reading's statistics.
    /// `bbox` is `None`, `counts.edges` and `counts.triangles` 0, and
    /// `ReadStats::edge_flags` zero; `finish` fills those in.
    fn index_and_stats(
        &mut self,
        container: &Container,
        base: u32,
        how: &'static str,
    ) -> (SceneIndex, ReadStats) {
        let unit = read_length_unit(&mut self.c);
        let (ro_schema, wide_entity) = (self.ro_schema, self.wide_entity);
        let block = read_rendering_options(&mut self.c, ro_schema, wide_entity);
        let walk_end = self.walk_end;
        // Before the pages, which need the records to say which style each tab
        // selects.
        self.style_recs = self.style_records(Some(walk_end));
        let scenes = self.scan_pages(Some(walk_end));

        // One `StyleDef` per distinct name the tail's `CSkpStyle` records
        // carry, in first-appearance order, each starting from the options
        // block - the document's own complete state - and overlaid with what
        // its own record states. See the section comment above `StyleRec`.
        //
        // A file whose block cannot be confirmed still gets its names, with
        // SketchUp's factory values underneath and `ReadStats::notes` saying
        // so; a file with neither comes back with an empty list rather than
        // with defaults dressed up as the file's own settings.
        let block_style = block.clone().unwrap_or_default();
        if block.is_none() {
            self.notes.push(String::from(
                "the rendering options block was not found; styles carry factory defaults",
            ));
        }
        let mut styles: Vec<StyleDef> = Vec::new();
        for r in &self.style_recs {
            if styles.iter().any(|s| s.name == r.name) {
                continue;
            }
            styles.push(StyleDef {
                id: styles.len() as u32,
                name: r.name.clone(),
                ..block_style.clone()
            });
        }
        // The *last* record with a given name wins: a style that appears both
        // as a page's capture and in the manager is stated twice, and the
        // manager's copy is the style's own saved settings.
        for r in &self.style_recs {
            if let Some(s) = styles.iter_mut().find(|s| s.name == r.name) {
                apply_style_opts(s, &r.opts);
            }
        }
        // The style the document opens with. Measured on five files against
        // SketchUp's own `selected_style`: it is the last record in the tail.
        let active = self
            .style_recs
            .last()
            .map(|r| r.name.clone())
            .and_then(|n| styles.iter().find(|s| s.name == n).map(|s| s.id));
        if styles.is_empty() {
            styles.extend(block.clone());
        }
        let active = active.or_else(|| styles.first().map(|s| s.id));

        // Layers, in the order the model list declared them, with any layer
        // that only a definition mentioned appended.
        let mut layers = Vec::with_capacity(self.layers.len());
        for (i, l) in self.layers.iter().enumerate() {
            layers.push(LayerDef {
                id: i as u32,
                name: l.name.clone(),
                hidden: l.hidden,
                color: l.rgba,
                folder: Vec::new(),
            });
        }

        let mut materials = Vec::with_capacity(self.materials.len());
        let mut textures = 0u32;
        let mut texture_bytes = 0u64;
        for (i, m) in self.materials.iter().enumerate() {
            if let Some(t) = &m.texture {
                textures += 1;
                if let TextureLocation::Inline { len, .. } = t.location {
                    texture_bytes += len;
                }
            }
            materials.push(MaterialDef {
                id: i as u32,
                // `*18` is what SketchUp shows as `Material18`, as on the
                // modern path.
                name: crate::materials::display_name(&m.name),
                rgba: m.rgba,
                opacity: m.opacity,
                texture: m.texture.clone(),
                colorized: m.colorized,
            });
        }

        let index = SceneIndex {
            writer_version: container.header.version.clone(),
            generation_is_vff: false,
            unit,
            bbox: None,
            scenes,
            layers,
            materials,
            styles,
            active_style: active,
            counts: Counts {
                definitions: self.defs.len() as u32,
                // Placements inside a definition that failed are not part of
                // the model.
                instances: {
                    let read: alloc::collections::BTreeSet<u32> =
                        self.defs.iter().map(|d| d.id).collect();
                    self.instances
                        .iter()
                        .filter(|i| i.parent == ROOT_DEFINITION || read.contains(&i.parent))
                        .count() as u32
                },
                triangles: 0,
                edges: 0,
                textures,
            },
            thumbnail_png: None,
        };
        let stats = ReadStats {
            bytes_read: self.c.read_bytes,
            stream_len: self.c.end - self.c.start,
            texture_bytes_skipped: texture_bytes,
            objects: self.objects,
            slot_base: base,
            slot_base_method: how,
            edge_flags: [0; 4],
            uv_sides: self.uv_sides,
            uv_matrices: self.uv_matrices,
            inner_loops: self.inner_loops,
            faces_short_of_exact: self.faces_short,
            lost_entities: self.lost,
            hidden_guides: self.hidden_guides,
            annotations: self.annotations,
            notes: core::mem::take(&mut self.notes),
        };
        (index, stats)
    }

    fn finish(mut self, container: &Container, base: u32, how: &'static str) -> LegacyModel {
        let (mut index, mut stats) = self.index_and_stats(container, base, how);

        // Placements, grouped by (parent, definition) so that nothing is
        // flattened and the hierarchy survives.
        let mut placements: Vec<Placements> = Vec::new();
        for inst in &self.instances {
            let key = (inst.parent, inst.def_slot);
            let pos = placements
                .iter()
                .position(|p| (p.parent_id, p.table.definition_id) == key)
                .unwrap_or_else(|| {
                    placements.push(Placements {
                        parent_id: inst.parent,
                        table: InstanceTable {
                            definition_id: inst.def_slot,
                            ..Default::default()
                        },
                    });
                    placements.len() - 1
                });
            let t = &mut placements[pos].table;
            for v in row_major_4x4(&inst.xf) {
                t.matrices.push(v);
            }
            t.layer_ids.push(self.layer_index(inst.layer_slot));
            t.material_ids.push(self.material_index(inst.mat_slot));
            t.names.push(inst.name.clone());
        }

        let mut definitions = core::mem::take(&mut self.defs);
        for d in definitions.iter_mut() {
            if self.image_defs.contains(&d.id) {
                d.is_image = true;
            }
        }

        // Every edge the file carries, placed or not: this is the number the
        // C++ oracle reports as `edge_flags.total`, and matching it exactly is
        // how the geometry layer is checked.
        index.counts.edges = self.edge_batches.iter().map(|e| e.len() as u64).sum();

        // The bounding box is taken before the unreachable definitions are
        // dropped, because it is computed by walking the placement tree and
        // so only ever sees what is placed anyway.
        let bbox = self.model_bbox(&placements);
        if let Some(bb) = bbox {
            for s in index.scenes.iter_mut() {
                let (Some(eye), Some(target)) = (s.eye, s.target) else {
                    continue;
                };
                if let Some(nf) = crate::pages::derive_near_far(
                    eye,
                    target,
                    bb,
                    s.perspective,
                    s.allow_clipping_in_parallel,
                ) {
                    s.near = nf.near;
                    s.far = nf.far;
                }
            }
        }
        index.bbox = bbox;

        // Counted before the filter below, because the oracle's own
        // histogram covers every edge the file carries.
        for b in &self.edge_batches {
            for f in &b.flags {
                count_edge_flags(&mut stats.edge_flags, *f);
            }
        }

        let mut model = LegacyModel {
            index,
            definitions,
            meshes: core::mem::take(&mut self.meshes),
            edges: core::mem::take(&mut self.edge_batches),
            placements,
            stats,
        };

        // Batches for definitions nothing places are work nobody asked for
        // and GPU memory nobody will look at. The modern path never builds
        // them; this one has to build and then drop, because reachability is
        // not known until the root entity list has been read - and the root
        // list is the last thing in the archive.
        let live = model.reachable();
        model
            .meshes
            .retain(|m| m.definition_id == ROOT_DEFINITION || live.contains(&m.definition_id));
        model
            .edges
            .retain(|e| e.definition_id == ROOT_DEFINITION || live.contains(&e.definition_id));
        model.index.counts.triangles = model
            .meshes
            .iter()
            .map(|m| (m.indices.len() / 3) as u64)
            .sum();
        model
    }

    /// The model's own bounds, by walking the placement tree.
    ///
    /// Needed because `pages::derive_near_far` wants them: the near and far a
    /// legacy camera stores are the constructor's 1 and 1000 inches and
    /// describe nothing, so a viewer that used them would clip an interior to
    /// pieces.
    fn model_bbox(&self, placements: &[Placements]) -> Option<[f64; 6]> {
        // Each definition's local bounds, from the batches already built.
        let mut local: BTreeMap<u32, [f32; 6]> = BTreeMap::new();
        let mut grow = |id: u32, p: &[f32]| {
            let e = local.entry(id).or_insert([
                f32::INFINITY,
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ]);
            for c in p.chunks_exact(3) {
                for a in 0..3 {
                    if c[a] < e[a] {
                        e[a] = c[a];
                    }
                    if c[a] > e[3 + a] {
                        e[3 + a] = c[a];
                    }
                }
            }
        };
        for m in &self.meshes {
            grow(m.definition_id, &m.positions);
        }
        for b in &self.edge_batches {
            grow(b.definition_id, &b.positions);
        }
        let mut out = [
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut any = false;
        let mut stack: Vec<(u32, [f64; 16], u32)> = vec![(ROOT_DEFINITION, IDENTITY_4X4, 0)];
        let mut visited = 0u32;
        while let Some((id, xf, depth)) = stack.pop() {
            visited += 1;
            if visited > 1 << 20 || depth > 32 {
                continue;
            }
            if let Some(b) = local.get(&id) {
                if b[0] <= b[3] {
                    for i in 0..8 {
                        let p = [
                            b[if i & 1 == 0 { 0 } else { 3 }] as f64,
                            b[if i & 2 == 0 { 1 } else { 4 }] as f64,
                            b[if i & 4 == 0 { 2 } else { 5 }] as f64,
                        ];
                        let q = apply_4x4(&xf, p);
                        for a in 0..3 {
                            if q[a] < out[a] {
                                out[a] = q[a];
                            }
                            if q[a] > out[3 + a] {
                                out[3 + a] = q[a];
                            }
                        }
                        any = true;
                    }
                }
            }
            for p in placements.iter().filter(|p| p.parent_id == id) {
                for k in 0..p.table.len() {
                    let mut child = [0.0f64; 16];
                    for a in 0..16 {
                        child[a] = p.table.matrices[k * 16 + a] as f64;
                    }
                    stack.push((p.table.definition_id, mul_4x4(&xf, &child), depth + 1));
                }
            }
        }
        if any {
            Some(out)
        } else {
            None
        }
    }
}

const IDENTITY_4X4: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// What a `CFaceTextureCoords` side holds when nothing positioned it, which
/// under `geom::UvMap` is exactly the default planar projection.
const IDENTITY_3X3: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// The 13 stored doubles as the row-major 4x4 `model.rs` asks for.
///
/// Ground truth is the writer, which is validated against real SketchUp:
/// `openskp.create`'s `write_instance` takes a **row-major 3x3** plus a
/// translation and stores "exactly this 3x3 matrix (9 f64s) + translation (3
/// f64s) + a trailing 1.0 - the 4th row of a standard 4x4 affine matrix,
/// always [0,0,0,1], is omitted entirely rather than stored". The Python
/// reader's own point transform agrees:
/// `tx = m0*x + m1*y + m2*z + m9`, and its instanced bounding boxes match
/// SketchUp's. So:
///
/// ```text
///   [ m0 m1 m2 m9  ]
///   [ m3 m4 m5 m10 ]
///   [ m6 m7 m8 m11 ]
///   [ 0  0  0  1   ]
/// ```
///
/// Note this is *not* SketchUp's own `Transformation#to_a`, which is
/// column-major with the axes first - the archive stores the transpose of the
/// 3x3 part.
///
/// The thirteenth double is the uniform divisor SketchUp keeps in element 15
/// of its 4x4. It is 1.0 in every instance measured; when it is not, the
/// translation is divided by it, which is what the engine does.
fn row_major_4x4(m: &[f64; 13]) -> [f32; 16] {
    let w = if m[12].is_finite() && m[12] != 0.0 {
        m[12]
    } else {
        1.0
    };
    let t = |v: f64| (v * INCH_TO_MM / w) as f32;
    [
        m[0] as f32,
        m[1] as f32,
        m[2] as f32,
        t(m[9]),
        m[3] as f32,
        m[4] as f32,
        m[5] as f32,
        t(m[10]),
        m[6] as f32,
        m[7] as f32,
        m[8] as f32,
        t(m[11]),
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn mul_4x4(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut o = [0.0f64; 16];
    for r in 0..4 {
        for c in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[r * 4 + k] * b[k * 4 + c];
            }
            o[r * 4 + c] = s;
        }
    }
    o
}

fn apply_4x4(m: &[f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemorySource;
    use alloc::vec::Vec;

    /// A `CText` record as a 2016 file (no wide entity preamble) writes it: the class, no
    /// attributes, a drawing element on layer 0, no font, the eleven-byte block before the
    /// text, the text "Hi" and its five trailing bytes.
    fn text_record() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&[0xff, 0xff, 0, 0, 5, 0]);
        b.extend_from_slice(b"CText");
        b.extend_from_slice(&[0, 0]); // no attribute container
        b.extend_from_slice(&[0; 8]); // drawing element
        b.extend_from_slice(&[0, 0]); // layer slot
        b.extend_from_slice(&[0, 0]); // no font
        b.extend_from_slice(&[1, 0, 0, 0, 0, 0, 2, 0, 0, 0, 1]);
        b.extend_from_slice(&[0xff, 0xfe, 0xff, 2, b'H', 0, b'i', 0]);
        b.extend_from_slice(&[0; 5]);
        b.extend_from_slice(&[0; 8]); // what follows the record
        b
    }

    #[test]
    fn text_records_are_counted_as_annotations() {
        use crate::source::ByteSource;
        let src = MemorySource::new(text_record());
        let end = src.len();
        let c = Cur::new(&src, 0, end);
        let mut archive = Archive::new(c, 16, &VersionMap::default());
        let (_, slot) = archive.read_object(None).unwrap().unwrap();
        assert!(slot.is(Cls::Text));
        assert_eq!(archive.annotations, 1);
        assert_eq!(archive.c.left(), 8);
        // The middle is too short to hold the fields: the label is unread, not misread.
        assert!(matches!(
            archive.pending_annots[0].body,
            PendingBody::Unread
        ));
    }

    fn f64s(v: &[f64]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    fn empty_mark() -> Mark {
        Mark {
            verts: 0,
            edges: 0,
            faces: 0,
            loops: 0,
            loop_uses: 0,
            face_loops: 0,
            ftcs: 0,
            guides: 0,
            hidden_guide_items: 0,
            annots: 0,
            instances: 0,
            slot: 0,
        }
    }

    /// A `CText` with the 123-byte middle every sample has: free anchor, leader vector.
    fn full_text_record(kind: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&[0xff, 0xff, 0, 0, 5, 0]);
        b.extend_from_slice(b"CText");
        b.extend_from_slice(&[0, 0]); // no attribute container
        b.extend_from_slice(&[0; 8]); // drawing element
        b.extend_from_slice(&[0, 0]); // layer slot
        b.extend_from_slice(&[0, 0]); // no font
        b.extend_from_slice(&[0; 16]);
        b.extend_from_slice(&[1, 0, 0, 0, 4, 0, 0, 0]);
        b.extend_from_slice(&f64s(&[1.0, 2.0, 3.0])); // anchor
        b.extend_from_slice(&[0; 12]); // no connection
        b.extend_from_slice(&f64s(&[4.0, 5.0, 6.0])); // leader vector
        b.extend_from_slice(&f64s(&[0.0, 0.0, 1.0])); // text direction
        b.extend_from_slice(&kind.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0, 1, 0, 3, 0, 0, 0, 1]);
        b.extend_from_slice(&[0xff, 0xfe, 0xff, 2, b'H', 0, b'i', 0]);
        b.extend_from_slice(&[0; 5]);
        b
    }

    /// A `CDimensionLinear` between two free points, `dim_type` and `y_pos` as given.
    fn dimension_record(dim_type: u32, y_pos: f64, via_path: bool) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&[0xff, 0xff, 0, 0, 16, 0]);
        b.extend_from_slice(b"CDimensionLinear");
        b.extend_from_slice(&[0, 0]); // no attribute container
        b.extend_from_slice(&[0; 8]); // drawing element
        b.extend_from_slice(&[0, 0]); // layer slot
        b.extend_from_slice(&[0xff, 0xfe, 0xff, 0]); // no text of its own
        b.extend_from_slice(&[0, 0]); // no font
                                      // One end: the block with its point, then the connection (an entity ref, the second
                                      // ref, the two path counts and any path items).
        let end = |head: &[u8], p: [f64; 3], paths: u32| {
            let mut e = Vec::new();
            e.extend_from_slice(head);
            e.extend_from_slice(&f64s(&p));
            e.extend_from_slice(&[0, 0]); // no entity
            e.extend_from_slice(&[0, 0]); // no second ref
            e.extend_from_slice(&paths.to_le_bytes());
            e.extend(std::iter::repeat_n([1u8, 0], paths as usize).flatten());
            e.extend_from_slice(&0u32.to_le_bytes());
            e
        };
        // 5 bytes, kind 1, 4; and 8 bytes of the same kind for the second end.
        b.extend_from_slice(&end(&[0, 0, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0], [0.0; 3], 0));
        b.extend_from_slice(&end(
            &[1, 0, 0, 0, 4, 0, 0, 0],
            [100.0, 0.0, 0.0],
            u32::from(via_path),
        ));
        b.extend_from_slice(&f64s(&[0.0, 0.0, 1.0])); // normal
        b.extend_from_slice(&f64s(&[1.0, 0.0, 0.0])); // x axis
        b.extend_from_slice(&dim_type.to_le_bytes());
        b.extend_from_slice(&y_pos.to_le_bytes());
        b.extend_from_slice(&0f64.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b
    }

    fn walk(bytes: Vec<u8>) -> (Vec<crate::annotations::RawAnnotation>, u32, u32) {
        use crate::source::ByteSource;
        let src = MemorySource::new(bytes);
        let end = src.len();
        let c = Cur::new(&src, 0, end);
        let mut archive = Archive::new(c, 16, &VersionMap::default());
        archive.read_object(None).unwrap().unwrap();
        let mut g = DefGeometry::default();
        archive.resolve_annotations(&mut g, &empty_mark()).unwrap();
        (
            g.annotations,
            g.annotation_skips.unread,
            g.annotation_skips.screen_text,
        )
    }

    #[test]
    fn a_text_label_is_read_with_its_anchor_vector_and_string() {
        use crate::annotations::RawAnnotation;
        let (found, unread, screen) = walk(full_text_record(2));
        assert_eq!((unread, screen), (0, 0));
        assert_eq!(
            found,
            [RawAnnotation::Leader {
                anchor: [1.0, 2.0, 3.0],
                vector: [4.0, 5.0, 6.0],
                text: "Hi".into(),
                layer: 0,
            }]
        );
        // Screen text (leader type 0) is counted, not imported.
        let (found, unread, screen) = walk(full_text_record(0));
        assert_eq!((found.len(), unread, screen), (0, 0, 1));
    }

    #[test]
    fn a_linear_dimension_is_read_with_its_offset() {
        use crate::annotations::RawAnnotation;
        // Aligned: the segment runs along x, so the line is 10 along n x x = +y.
        let (found, unread, _) = walk(dimension_record(0, 10.0, false));
        assert_eq!(unread, 0);
        assert_eq!(
            found,
            [RawAnnotation::Linear {
                start: [0.0, 0.0, 0.0],
                end: [100.0, 0.0, 0.0],
                offset: [0.0, 10.0, 0.0],
                layer: 0,
                custom_text: false,
            }]
        );
        // Vertical, on a segment that is not vertical: unread.
        let (found, unread, _) = walk(dimension_record(2, 10.0, false));
        assert_eq!((found.len(), unread), (0, 1));
    }

    #[test]
    fn a_dimension_through_an_instance_path_is_unread() {
        let (found, unread, _) = walk(dimension_record(0, 10.0, true));
        assert_eq!((found.len(), unread), (0, 1));
    }
}
