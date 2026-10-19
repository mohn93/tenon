//! Reading a whole model, and turning it into what a GPU wants.
//!
//! The streaming discipline from `stream.rs` is kept here: the big sections -
//! the definition list and the root - are consumed one record at a time, and
//! only the small sections (layers, materials, styles, pages, the document
//! camera) are retained whole, because they are kilobytes and the rest of the
//! reader needs them.
//!
//! Nothing in here flattens an instance. The instance tree is resolved into a
//! placement list *per definition*, composing transforms through nesting, so a
//! chair placed a thousand times is one mesh and a thousand matrices. That is
//! the difference between a model that draws in a few hundred calls and one
//! that issues 38,019 of them and never finishes a frame.

use crate::container::{Container, Generation};
use crate::edges::{build_adjacency, build_batch, EdgeAdjacency, EdgeFilter};
use crate::error::{Error, Result};
use crate::geom::{compose, parse_component_within, DefGeometry, IDENTITY13, NO_LAYER};
use crate::model::{
    Counts, EdgeBatch, InstanceTable, MeshBatch, SceneIndex, StyleDef, INCH_TO_MM, NO_MATERIAL,
};
use crate::source::ByteSource;
use crate::stream::{InflateRead, RecordStream, SeqRead};
use crate::tags as t;
use crate::tlv::Tlv;
use crate::zip::entry_data_offset;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// A component definition: its identity and its geometry.
pub struct Definition {
    pub pid: u64,
    pub name: String,
    pub guid: [u8; 16],
    /// SketchUp's "always face camera" behaviour. Worth carrying because a
    /// definition built with it on by accident makes every instance of it turn
    /// to the camera regardless of its transform - a failure that looks
    /// plausible in perspective and obvious in plan.
    pub always_face_camera: bool,
    pub geom: DefGeometry,
}

/// Section payloads small enough to keep, for the decoders that need them.
#[derive(Default)]
pub struct Sections {
    pub layers: Option<Vec<u8>>,
    pub materials: Option<Vec<u8>>,
    pub styles: Option<Vec<u8>>,
    pub rendering_options: Option<Vec<u8>>,
    pub pages: Option<Vec<u8>>,
    pub camera: Option<Vec<u8>>,
    pub options: Option<Vec<u8>>,
    pub metadata: Option<Vec<u8>>,
}

/// The root's own geometry: everything modelled in place rather than inside a
/// component. `mesh_batches` and `edge_batches` already label it this way.
pub const ROOT_DEFINITION: u32 = u32::MAX;

/// Row-major identity, the root's placement.
pub const IDENTITY4: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0, //
];

/// Marks a zero-triangle batch that reports how many faces were skipped, so
/// a hole in the output is visible in the numbers rather than only on screen.
pub const SKIPPED_MARKER: u32 = u32::MAX - 1;

/// Anything above this and a "small" section is not small; it gets streamed
/// past instead of retained. Measured, the largest of these in the corpus is
/// under 100 KB.
const SMALL_SECTION_MAX: u64 = 64 * 1024 * 1024;

/// What one definition counts against `ReadOptions::max_elements`: a `Definition` is a few
/// hundred bytes before any geometry, about fifty elements' worth.
const DEFINITION_WEIGHT: u64 = 50;

/// The most a single `material.xml` may inflate to before `index` skips it.
const MAX_MATERIAL_XML_BYTES: u64 = 1024 * 1024;

pub struct Model {
    pub writer_version: String,
    pub is_vff: bool,
    pub root: DefGeometry,
    pub definitions: Vec<Definition>,
    pub sections: Sections,
    /// Bytes actually read from the file. The point of the whole design: on a
    /// legacy pack this should be far below the file size, because the texture
    /// blobs were seeked past rather than read.
    pub bytes_read: u64,
}

/// What to spend time on. Line art needs no textures and no UVs, and saying
/// so up front is worth hundreds of megabytes on a commercial pack.
#[derive(Debug, Clone, Copy)]
pub struct ReadOptions {
    pub want_geometry: bool,
    pub want_edges: bool,
    /// Emit `MeshBatch::uvs`. Costs 8 bytes a vertex and one matrix solve a
    /// face; a hidden-line export needs neither, and this is where it says so.
    pub want_uvs: bool,
    /// Cap on definitions read, for progressive loading. `None` for all.
    pub definition_limit: Option<usize>,
    /// Bake definitions placed this many times or fewer into shared buffers.
    /// Zero disables it. Two is the measured sweet spot: it collapses 3,774 of
    /// a commercial pack's 4,147 definitions while bounding the duplication at
    /// twice their geometry.
    pub merge_below: usize,
    /// Most vertices, edges, faces, loops, edge uses, guides and instances `read_model` takes over
    /// the root and every definition together; past it the read fails with
    /// `Malformed("too many elements")`. `None` for no limit.
    pub max_elements: Option<u64>,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            want_geometry: true,
            want_edges: true,
            want_uvs: true,
            definition_limit: None,
            merge_below: 2,
            max_elements: None,
        }
    }
}

/// A model loaded from either container, in one shape.
///
/// The two containers are different wire formats for the same objects - 65
/// model classes in SketchUp implement both a CArchive `Serialize` and a
/// tagged `Read` - so everything above this line can be container-agnostic,
/// and a viewer never has to ask which generation it is looking at.
pub struct Loaded {
    pub index: SceneIndex,
    pub meshes: Vec<MeshBatch>,
    pub edges: Vec<EdgeBatch>,
    pub instances: Vec<InstanceTable>,
    /// Bytes actually requested from the source. Far below the file size on a
    /// legacy pack, which is the whole point of the design.
    pub bytes_read: u64,
}

/// Load a file whichever container it uses.
pub fn load(src: &dyn ByteSource, c: &Container, opt: &ReadOptions) -> Result<Loaded> {
    match c.generation {
        Generation::Vff => {
            let m = read_model(src, c, opt)?;
            let index = m.index(src, c);
            let textured = textured_materials(&index);
            let want = if opt.want_uvs && !textured.is_empty() {
                Some(&textured)
            } else {
                None
            };
            let (meshes, edges, instances) = if opt.want_geometry {
                (
                    m.mesh_batches_with(true, want),
                    m.edge_batches(crate::edges::EdgeFilter::All),
                    m.instance_tables(),
                )
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };
            let mut l = Loaded {
                index,
                meshes,
                edges,
                instances,
                bytes_read: m.bytes_read,
            };
            if opt.merge_below > 0 {
                merge_small(&mut l, opt.merge_below);
            }
            enforce_uv_invariant(&mut l);
            Ok(l)
        }
        Generation::Legacy => {
            let m = crate::legacy::read(src, c)?;
            // `placements` is the file's own statement: one table per
            // (parent, child) pair, with transforms relative to the parent.
            // A renderer needs them composed through the nesting, which is
            // what `instance_tables` does.
            //
            // Taking the raw tables instead looks almost right and is badly
            // wrong: a school cafeteria stated 1,786 placement records that
            // resolve to 6,186 placed nodes, so three quarters of the model
            // was never drawn. It rendered as a speck in the middle of a
            // correctly-sized bounding box, which is exactly the shape of
            // failure that survives a count check.
            let instances = m.instance_tables();
            // Whoever built these batches, the renderer draws the drawable
            // prefix - so make sure there is one.
            let mut edges = m.edges;
            for b in &mut edges {
                crate::edges::sort_drawable_first(b);
            }
            let mut index = m.index;
            // The same two corrections the VFF index gets, applied here rather
            // than inside the legacy reader so there is one statement of what
            // a material list is. `*60` is how the file spells SketchUp's
            // `Material60` - 177 of the food pack's 179 names are starred -
            // and a `Layer_<tag>` material is a tag's colour, not a material.
            for mat in &mut index.materials {
                mat.name = crate::materials::display_name(&mat.name);
            }
            index
                .materials
                .retain(|mat| !crate::materials::is_layer_material(&mat.name));
            index.counts.textures = index
                .materials
                .iter()
                .filter(|mat| mat.texture.is_some())
                .count() as u32;
            let mut l = Loaded {
                index,
                meshes: m.meshes,
                edges,
                instances,
                bytes_read: m.stats.bytes_read,
            };
            if opt.merge_below > 0 {
                merge_small(&mut l, opt.merge_below);
            }
            if !opt.want_uvs {
                for b in &mut l.meshes {
                    b.uvs = Vec::new();
                }
            }
            enforce_uv_invariant(&mut l);
            Ok(l)
        }
    }
}

/// `MeshBatch::uvs` is parallel to `positions` or it is empty.
///
/// Checked here rather than trusted, because both containers feed this and
/// they are written by different hands. A half-filled UV array is not a
/// cosmetic defect: it shifts every vertex after the gap onto the wrong part
/// of the image, and the symptom looks like a bad texture rather than like a
/// bug in the reader.
fn enforce_uv_invariant(l: &mut Loaded) {
    for b in &mut l.meshes {
        if b.uvs.len() * 3 != b.positions.len() * 2 {
            b.uvs.clear();
        }
    }
}

/// The ids of materials that carry an image.
///
/// A model whose materials are all flat colour gets no UVs at all, which is
/// not an optimisation so much as the honest answer: there is nothing for them
/// to address.
pub fn textured_materials(index: &SceneIndex) -> alloc::collections::BTreeSet<u32> {
    index
        .materials
        .iter()
        .filter(|m| m.texture.is_some())
        .map(|m| m.id)
        .collect()
}

pub fn read_model(src: &dyn ByteSource, c: &Container, opt: &ReadOptions) -> Result<Model> {
    match c.generation {
        Generation::Vff => {
            let e = c
                .model_entry()
                .ok_or(Error::Malformed("VFF file with no model.dat"))?;
            let at = entry_data_offset(src, e)?;
            let r = InflateRead::new(src, at, e.compressed_size, e.uncompressed_size)?;
            let total = e.uncompressed_size;
            let mut rs = RecordStream::new(r);
            let mut m = read_stream(&mut rs, total, c, opt)?;
            // Report file bytes, not stream bytes - see InflateRead::source_bytes.
            // The inflated figure would make a 7.7 MB file look like it read
            // 315% of itself, which is true of the stream and nonsense about
            // the file.
            m.bytes_read = rs.r.source_bytes();
            Ok(m)
        }
        Generation::Legacy => {
            // The legacy container is a different wire format entirely and is
            // handled by its own module; this path exists so the error says
            // that rather than failing to find a record.
            let _ = c.legacy_stream;
            Err(Error::Malformed("legacy container: use the legacy reader"))
        }
    }
}

fn read_stream<R: SeqRead>(
    rs: &mut RecordStream<R>,
    total: u64,
    c: &Container,
    opt: &ReadOptions,
) -> Result<Model> {
    let mut m = Model {
        writer_version: c.header.version.clone(),
        is_vff: c.generation == Generation::Vff,
        root: DefGeometry::default(),
        definitions: Vec::new(),
        sections: Sections::default(),
        bytes_read: 0,
    };
    let mut budget = opt.max_elements.unwrap_or(u64::MAX);

    let Some(root) = rs.header(total)? else {
        return Err(Error::Malformed("model stream is empty"));
    };
    if root.tag != t::SKETCH_UP_MODEL {
        return Err(Error::Malformed(
            "model stream does not begin with SketchUpModel",
        ));
    }
    // The model record cannot run past the stream's claimed size.
    let root_end = core::cmp::min(rs.enter(&root)?, total);
    let mut buf: Vec<u8> = Vec::new();

    while let Some(sec) = rs.header(root_end)? {
        match sec.tag {
            t::SKETCH_UP_MODEL_COMPONENTS => {
                let sec_end = rs.enter(&sec)?;
                read_definitions(rs, sec_end, &mut m, opt, &mut buf, &mut budget)?;
            }
            t::SKETCH_UP_MODEL_ROOT => {
                rs.payload(sec.size, &mut buf)?;
                if opt.want_geometry || opt.want_edges {
                    // The root's own Component holds whatever geometry is not
                    // inside a definition - in practice walls and slabs drawn
                    // in place, plus the top-level placements.
                    for inner in Tlv::new(&buf) {
                        if inner.tag == t::COMPONENT {
                            m.root = parse_component_within(inner.payload, &mut budget)?;
                        }
                    }
                }
            }
            t::SKETCH_UP_MODEL_LAYERS => keep(rs, &sec, &mut m.sections.layers)?,
            t::SKETCH_UP_MODEL_MATERIALS => keep(rs, &sec, &mut m.sections.materials)?,
            t::SKETCH_UP_MODEL_STYLES => keep(rs, &sec, &mut m.sections.styles)?,
            t::SKETCH_UP_MODEL_RENDERING_OPTIONS => {
                keep(rs, &sec, &mut m.sections.rendering_options)?
            }
            t::SKETCH_UP_MODEL_PAGES => keep(rs, &sec, &mut m.sections.pages)?,
            t::SKETCH_UP_MODEL_CAMERA => keep(rs, &sec, &mut m.sections.camera)?,
            t::SKETCH_UP_MODEL_OPTIONS => keep(rs, &sec, &mut m.sections.options)?,
            t::MODEL_METADATA => keep(rs, &sec, &mut m.sections.metadata)?,
            _ => rs.skip(sec.size)?,
        }
    }

    Ok(m)
}

fn keep<R: SeqRead>(
    rs: &mut RecordStream<R>,
    sec: &crate::stream::Header,
    slot: &mut Option<Vec<u8>>,
) -> Result<()> {
    if sec.size > SMALL_SECTION_MAX {
        return rs.skip(sec.size);
    }
    let mut v = Vec::new();
    rs.payload(sec.size, &mut v)?;
    *slot = Some(v);
    Ok(())
}

fn read_definitions<R: SeqRead>(
    rs: &mut RecordStream<R>,
    sec_end: u64,
    m: &mut Model,
    opt: &ReadOptions,
    buf: &mut Vec<u8>,
    budget: &mut u64,
) -> Result<()> {
    // SketchUpModelComponents > DefinitionList > DefinitionListDefs, each of
    // which may carry an OverflowSizeTag that `enter` deals with.
    let mut inner_end = sec_end;
    for expect in [t::DEFINITION_LIST, t::DEFINITION_LIST_DEFS] {
        match rs.header(inner_end)? {
            Some(h) if h.tag == expect => inner_end = rs.enter(&h)?,
            Some(h) => {
                rs.skip(h.size)?;
                return Ok(());
            }
            None => return Ok(()),
        }
    }

    while let Some(d) = rs.header(inner_end)? {
        if d.tag != t::COMPONENT_DEFINITION {
            rs.skip(d.size)?;
            continue;
        }
        if let Some(limit) = opt.definition_limit {
            if m.definitions.len() >= limit {
                rs.skip(d.size)?;
                continue;
            }
        }
        rs.payload(d.size, buf)?;
        // A definition costs far more than one element; count it so, before it is made.
        crate::geom::spend(budget, DEFINITION_WEIGHT)?;
        m.definitions.push(parse_definition(buf, budget)?);
    }
    Ok(())
}

fn parse_definition(payload: &[u8], budget: &mut u64) -> Result<Definition> {
    let mut def = Definition {
        pid: 0,
        name: String::new(),
        guid: [0u8; 16],
        always_face_camera: false,
        geom: DefGeometry::default(),
    };
    for r in Tlv::new(payload) {
        match r.tag {
            t::COMPONENT => {
                def.geom = parse_component_within(r.payload, budget)?;
                let c = crate::geom::common_of(&r);
                if c.pid != 0 {
                    def.pid = c.pid;
                }
            }
            t::COMPONENT_DEFINITION_NAME => {
                def.name = String::from_utf8_lossy(r.payload).into_owned();
            }
            t::COMPONENT_DEFINITION_GUID => {
                if r.payload.len() >= 16 {
                    def.guid.copy_from_slice(&r.payload[..16]);
                }
            }
            t::COMPONENT_BEHAVIOR => {
                for b in r.children() {
                    if b.tag == t::COMPONENT_BEHAVIOR_ALWAYS_FACE_CAMERA {
                        def.always_face_camera = b.payload.first().copied().unwrap_or(0) != 0;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(def)
}

// ---------------------------------------------------------------------------
// GPU-facing output
// ---------------------------------------------------------------------------

impl Model {
    /// Hidden guides skipped over the root and every definition (2021+ files; a legacy
    /// read counts its own in `legacy::ReadStats::hidden_guides`).
    pub fn hidden_guides(&self) -> u64 {
        self.geometries().map(|g| u64::from(g.hidden_guides)).sum()
    }

    /// Guide-list records that were not read as a guide, over the root and every
    /// definition (2021+ files; see `DefGeometry::unread_guides`).
    pub fn unread_guides(&self) -> u64 {
        self.geometries().map(|g| u64::from(g.unread_guides)).sum()
    }

    fn geometries(&self) -> impl Iterator<Item = &DefGeometry> {
        core::iter::once(&self.root).chain(self.definitions.iter().map(|d| &d.geom))
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        c.definitions = self.definitions.len() as u32;
        let mut tri = 0u64;
        let mut edg = 0u64;
        let live = self.reachable();
        let placed = self
            .definitions
            .iter()
            .filter(|d| live.contains(&d.pid))
            .map(|d| &d.geom);
        for d in core::iter::once(&self.root).chain(placed) {
            edg += d.edges() as u64;
            // One triangle per loop-ring vertex beyond the second is the exact
            // count for a simple polygon and a close bound with holes, which
            // is all a progress indicator needs.
            for f in 0..d.faces() {
                let (l0, l1) = (d.face_loop_ofs[f] as usize, d.face_loop_ofs[f + 1] as usize);
                let mut n = 0usize;
                for lp in l0..l1 {
                    n += (d.loop_ofs[lp + 1] - d.loop_ofs[lp]) as usize;
                }
                tri += n.saturating_sub(2) as u64;
            }
        }
        c.triangles = tri;
        c.edges = edg;
        c
    }

    /// A definition index by pid, for resolving instance references.
    fn by_pid(&self) -> BTreeMap<u64, usize> {
        self.definitions
            .iter()
            .enumerate()
            .map(|(i, d)| (d.pid, i))
            .collect()
    }

    /// Which definitions are actually placed, directly or through nesting.
    ///
    /// A file carries definitions nobody uses. SketchUp's own blank template
    /// ships a scale figure, so a 0.2 MB file saved from it holds about a
    /// thousand triangles of a person who is not in the model - and a
    /// commercial pack measured here has 442 unused definitions out of 1,339,
    /// a third of the file. Triangulating those is work nobody asked for and
    /// GPU memory nobody will look at.
    ///
    /// They are still parsed, because a component browser will want their
    /// names and a thumbnail; they are simply not turned into meshes.
    pub fn reachable(&self) -> alloc::collections::BTreeSet<u64> {
        let index = self.by_pid();
        let mut seen = alloc::collections::BTreeSet::new();
        let mut stack: Vec<&DefGeometry> = vec![&self.root];
        let mut guard = 1_000_000u32;
        while let Some(g) = stack.pop() {
            if guard == 0 {
                break;
            }
            guard -= 1;
            for inst in &g.instances {
                if let Some(&di) = index.get(&inst.def_pid) {
                    if seen.insert(inst.def_pid) {
                        stack.push(&self.definitions[di].geom);
                    }
                }
            }
        }
        seen
    }

    /// Resolve the instance tree into one placement list per definition.
    ///
    /// Transforms compose through nesting, so a definition placed inside
    /// another accumulates both. A definition that contains itself - which a
    /// corrupt file can claim - is stopped by the depth guard rather than by
    /// running out of memory.
    pub fn instance_tables(&self) -> Vec<InstanceTable> {
        let index = self.by_pid();
        let mut out: Vec<InstanceTable> = self
            .definitions
            .iter()
            .map(|d| InstanceTable {
                definition_id: d.pid as u32,
                ..Default::default()
            })
            .collect();

        // The root is placed once, at identity. It is not a component and
        // nothing references it, so without this row a renderer that draws
        // "every definition that has placements" draws no root geometry at
        // all - and in a room modelled in place that is the floor, the walls
        // and the ceiling. The symptom is a render of the furniture floating
        // in white space, which is exactly what it looked like.
        let root_table = InstanceTable {
            definition_id: ROOT_DEFINITION,
            matrices: IDENTITY4.to_vec(),
            layer_ids: alloc::vec![u32::MAX],
            material_ids: alloc::vec![NO_MATERIAL],
            names: Vec::new(),
        };
        let mut slot: BTreeMap<u64, usize> = self
            .definitions
            .iter()
            .enumerate()
            .map(|(i, d)| (d.pid, i))
            .collect();
        // Placements are discovered depth-first from the root, carrying the
        // transform AND the paint down the tree.
        //
        // The paint has to be inherited because that is SketchUp's own rule:
        // painting a component paints every face inside it that has no
        // material of its own, however deep. A chair group painted tan from
        // the outliner has a seat two levels down with no paint, and reading
        // only the immediate instance leaves that seat default-white. On the
        // school cafeteria 1,055,678 of 3.7 million frame triangles are
        // painted this way and nothing else says what colour they are.
        let mut stack: Vec<(&DefGeometry, [f64; 13], u32, u32)> =
            vec![(&self.root, IDENTITY13, 0, NO_MATERIAL)];
        const MAX_DEPTH: u32 = 32;
        let mut guard: u64 = 4_000_000;

        while let Some((geom, xform, depth, inherited)) = stack.pop() {
            if depth > MAX_DEPTH {
                continue;
            }
            for inst in &geom.instances {
                if guard == 0 {
                    return out;
                }
                guard -= 1;
                let world = compose(&xform, &inst.m);
                let Some(&di) = index.get(&inst.def_pid) else {
                    continue;
                };
                let d = &self.definitions[di];
                // Nearest painted ancestor wins, and this instance is nearer
                // than its parent.
                let paint = if inst.has_material {
                    inst.material as u32
                } else {
                    inherited
                };
                let table = &mut out[*slot.entry(d.pid).or_insert(di)];
                let mut tmp = crate::geom::RawInstance {
                    m: world,
                    ..Default::default()
                };
                tmp.m = world;
                table.matrices.extend_from_slice(&tmp.matrix4_mm());
                table.layer_ids.push(if inst.has_layer {
                    inst.layer as u32
                } else {
                    u32::MAX
                });
                table.material_ids.push(paint);
                stack.push((&d.geom, world, depth + 1, paint));
            }
        }
        out.retain(|t| !t.is_empty());
        // Always, even when the root holds no geometry of its own. It is the
        // scene's root node either way, and making its presence conditional
        // puts an off-by-one into every count that depends on it - which is
        // how this was noticed.
        out.insert(0, root_table);
        out
    }

    /// Triangles per definition, grouped by front material.
    ///
    /// Faces are emitted flat - one vertex per face corner, carrying the
    /// face's plane normal. Smooth shading needs averaged vertex normals per
    /// edge use, which SketchUp computes with a separate calculator; for
    /// hidden-line output, which is what this viewer exists for, flat normals
    /// are not an approximation of anything that shows.
    pub fn mesh_batches(&self) -> Vec<MeshBatch> {
        self.mesh_batches_with(true, None)
    }

    /// `placed_only` skips definitions nothing places - see `reachable`.
    ///
    /// `textured` is the set of material ids that carry an image; pass `None`
    /// for no UVs at all. Only batches whose paint could be an image get
    /// them - see [`Model::wants_uv`] for why that includes the unpainted
    /// ones - which is what keeps `MeshBatch::uvs` from being 8 bytes a vertex
    /// on geometry nothing will ever sample.
    pub fn mesh_batches_with(
        &self,
        placed_only: bool,
        textured: Option<&alloc::collections::BTreeSet<u32>>,
    ) -> Vec<MeshBatch> {
        let mut out = Vec::new();
        self.emit_meshes(u32::MAX, &self.root, &mut out, textured);
        let live = if placed_only {
            Some(self.reachable())
        } else {
            None
        };
        for d in &self.definitions {
            if let Some(l) = &live {
                if !l.contains(&d.pid) {
                    continue;
                }
            }
            self.emit_meshes(d.pid as u32, &d.geom, &mut out, textured);
        }
        out
    }

    /// Whether a batch painted with `material` needs UVs.
    ///
    /// `NO_MATERIAL` does: a face with no paint of its own is painted by the
    /// instance that places it, and that paint can be a texture. Dropping the
    /// UVs there is what leaves a component whose material was applied from
    /// the outliner rather than to its faces rendering flat - which is most of
    /// a commercial pack's furniture.
    fn wants_uv(material: u32, textured: Option<&alloc::collections::BTreeSet<u32>>) -> bool {
        match textured {
            None => false,
            Some(set) => material == NO_MATERIAL || set.contains(&material),
        }
    }

    fn emit_meshes(
        &self,
        def_id: u32,
        g: &DefGeometry,
        out: &mut Vec<MeshBatch>,
        textured: Option<&alloc::collections::BTreeSet<u32>>,
    ) {
        if g.faces() == 0 {
            return;
        }
        let mut by_mat: BTreeMap<u32, MeshBatch> = BTreeMap::new();
        let mut ring: Vec<u32> = Vec::new();
        let mut rings: Vec<Vec<u32>> = Vec::new();

        // The vertex table is built once per definition. Rebuilding it per
        // face is how the C++ reader came to spend fifty seconds on a 7.7 MB
        // room, and writing this module I made the identical mistake and
        // watched the small file take ten times as long as the large one.
        // Clearing and resizing a vector per face is the same O(faces x
        // vertices) in different clothing.
        let mut pts: Vec<[f64; 3]> = Vec::with_capacity(g.vertices());
        for i in 0..g.vertices() {
            pts.push(g.vertex(i as u32));
        }

        let mut skipped_no_ring = 0usize;
        let mut skipped_no_tris = 0usize;

        for f in 0..g.faces() {
            let (l0, l1) = (g.face_loop_ofs[f] as usize, g.face_loop_ofs[f + 1] as usize);
            if l1 <= l0 {
                continue;
            }
            rings.clear();
            for lp in l0..l1 {
                if g.loop_ring(lp, &mut ring) {
                    rings.push(ring.clone());
                }
            }
            if rings.is_empty() {
                skipped_no_ring += 1;
                continue;
            }
            let refs: Vec<&[u32]> = rings.iter().map(|r| r.as_slice()).collect();
            let tris = crate::geom::triangulate(&pts, &refs, g.normal(f));
            if tris.is_empty() {
                skipped_no_tris += 1;
                continue;
            }

            // A face painted on one side only shows that paint from both.
            //
            // SketchUp paints front and back independently and draws whichever
            // side you are looking at; one `MeshBatch` carries one material,
            // so a face with a back material and no front one would otherwise
            // come out in the style's default colour. On the commercial
            // classroom that is 35,604 of 189,178 faces - the roof slab among
            // them, which is why that building had no roof. A face painted on
            // BOTH sides keeps its front material and the back one is lost;
            // that is 3,580 faces in the same file and it needs a second batch
            // per face to fix, not a fallback.
            let mat = if g.face_material[f] != NO_MATERIAL {
                g.face_material[f]
            } else {
                g.face_back_material[f]
            };
            let batch = by_mat.entry(mat).or_insert_with(|| MeshBatch {
                definition_id: def_id,
                material_id: mat,
                layer_id: g.face_layer[f],
                ..Default::default()
            });
            let n = g.normal(f);
            // Decided per material rather than per face, which is also per
            // batch - so `uvs` is either parallel to `positions` for the whole
            // batch or absent from it, with no third state to handle.
            let uv = if Self::wants_uv(mat, textured) {
                Some(g.uv_map(f))
            } else {
                None
            };
            for tri in tris {
                for &v in &tri {
                    let p = g.vertex(v);
                    batch.positions.push((p[0] * INCH_TO_MM) as f32);
                    batch.positions.push((p[1] * INCH_TO_MM) as f32);
                    batch.positions.push((p[2] * INCH_TO_MM) as f32);
                    batch.normals.push(n[0] as f32);
                    batch.normals.push(n[1] as f32);
                    batch.normals.push(n[2] as f32);
                    if let Some(map) = &uv {
                        // Inches in, inches out, scaled once at the end.
                        // `UvMap` is not linear in the point - its matrix
                        // carries a translation in the file's own unit - so
                        // handing it the millimetre point slides the texture
                        // across the face. See `UvMap`'s own doc comment,
                        // which records how far.
                        let t = map.at(p);
                        batch.uvs.push((t[0] * INCH_TO_MM) as f32);
                        batch.uvs.push((t[1] * INCH_TO_MM) as f32);
                    }
                }
                let base = (batch.positions.len() / 3 - 3) as u32;
                batch.indices.extend_from_slice(&[base, base + 1, base + 2]);
            }
        }
        // A face that produced nothing is a real defect somewhere - a
        // dangling edge reference, a self-intersecting ring - and it shows up
        // as a hole in the model rather than as an error, so it is counted
        // and reportable instead of silently dropped.
        if skipped_no_ring + skipped_no_tris > 0 {
            out.push(MeshBatch {
                definition_id: def_id,
                material_id: SKIPPED_MARKER,
                layer_id: (skipped_no_ring as u32) << 16 | skipped_no_tris as u32,
                ..Default::default()
            });
        }
        out.extend(by_mat.into_values());
    }

    pub fn edge_batches(&self, filter: EdgeFilter) -> Vec<EdgeBatch> {
        self.edge_batches_with(filter, true)
    }

    pub fn edge_batches_with(&self, filter: EdgeFilter, placed_only: bool) -> Vec<EdgeBatch> {
        let mut out = Vec::new();
        let live = if placed_only {
            Some(self.reachable())
        } else {
            None
        };
        let mut push = |id: u32, g: &DefGeometry| {
            if g.edges() == 0 {
                return;
            }
            let adj: EdgeAdjacency = build_adjacency(g);
            let b = build_batch(id, g, &adj, filter);
            if !b.is_empty() {
                out.push(b);
            }
        };
        push(u32::MAX, &self.root);
        for d in &self.definitions {
            if let Some(l) = &live {
                if !l.contains(&d.pid) {
                    continue;
                }
            }
            push(d.pid as u32, &d.geom);
        }
        out
    }

    /// Model bounds in millimetres, computed over resolved placements.
    pub fn bounds(&self) -> Option<[f64; 6]> {
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        let mut any = false;
        let index = self.by_pid();
        let mut stack: Vec<(&DefGeometry, [f64; 13], u32)> = vec![(&self.root, IDENTITY13, 0)];
        let mut guard: u64 = 4_000_000;
        while let Some((g, x, depth)) = stack.pop() {
            if depth > 32 || guard == 0 {
                continue;
            }
            guard -= 1;
            for i in 0..g.vertices() {
                let p = g.vertex(i as u32);
                let w = [
                    x[0] * p[0] + x[1] * p[1] + x[2] * p[2] + x[9],
                    x[3] * p[0] + x[4] * p[1] + x[5] * p[2] + x[10],
                    x[6] * p[0] + x[7] * p[1] + x[8] * p[2] + x[11],
                ];
                for k in 0..3 {
                    lo[k] = lo[k].min(w[k] * INCH_TO_MM);
                    hi[k] = hi[k].max(w[k] * INCH_TO_MM);
                }
                any = true;
            }
            for inst in &g.instances {
                if let Some(&di) = index.get(&inst.def_pid) {
                    stack.push((&self.definitions[di].geom, compose(&x, &inst.m), depth + 1));
                }
            }
        }
        if any {
            Some([lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]])
        } else {
            None
        }
    }

    /// Everything a viewer needs before geometry: scenes, tags, materials,
    /// bounds and counts.
    ///
    /// `src` and `c` are needed because a material's texture reference and its
    /// tile size live in a `material.xml` zip entry rather than in
    /// `model.dat`. Those entries are a few hundred bytes each; the images
    /// they name are not read.
    pub fn index(&self, src: &dyn ByteSource, c: &Container) -> SceneIndex {
        let mut idx = self.index_without_placements(src, c);
        idx.bbox = self.bounds();
        idx.counts.instances = self
            .instance_tables()
            .iter()
            .map(|t| t.len())
            .sum::<usize>() as u32;
        idx
    }

    /// [`Model::index`] without what needs every placement resolved through
    /// the nesting: `bbox` is `None` and `counts.instances` is 0. Those walks
    /// grow with the number of resolved placements, which a small file can
    /// make enormous (a definition placing another many times, many levels
    /// deep); this costs time linear in the file.
    pub fn index_without_placements(&self, src: &dyn ByteSource, c: &Container) -> SceneIndex {
        let mut idx = SceneIndex {
            writer_version: self.writer_version.clone(),
            generation_is_vff: self.is_vff,
            bbox: None,
            counts: self.counts(),
            ..Default::default()
        };

        if let Some(p) = &self.sections.pages {
            idx.scenes = crate::pages::parse_pages(p);
        }
        if let Some(p) = &self.sections.layers {
            idx.layers = crate::layers::parse_layers(p);
        }

        // Materials come from two places: the materials section, plus the
        // Layer_<name> materials that live inside the layers section. Leaving
        // the second set out makes every count disagree with every other
        // reader.
        if let Some(p) = &self.sections.materials {
            idx.materials = crate::materials::parse_materials(p);
        }
        if let Some(p) = &self.sections.layers {
            idx.materials
                .extend(crate::materials::parse_layer_materials(p));
        }

        if let Some(zip) = &c.zip {
            let mut xml_bytes: Vec<(String, Vec<u8>)> = Vec::new();
            let mut images: Vec<(String, u32)> = Vec::new();
            for (i, e) in zip.entries.iter().enumerate() {
                if crate::materials::is_texture_entry(&e.name) {
                    images.push((e.name.clone(), i as u32));
                } else if e.name.ends_with("material.xml") {
                    if let Some(folder) = crate::materials::material_xml_folder(&e.name) {
                        // A material description is a few kilobytes; a
                        // larger one is skipped like an unreadable one.
                        if let Ok(b) = crate::zip::read_entry_within(src, e, MAX_MATERIAL_XML_BYTES)
                        {
                            xml_bytes.push((folder.to_string(), b));
                        }
                    }
                }
            }
            let xmls: Vec<(&str, &[u8])> = xml_bytes
                .iter()
                .map(|(f, b)| (f.as_str(), b.as_slice()))
                .collect();
            let imgs: Vec<(&str, u32)> = images.iter().map(|(n, i)| (n.as_str(), *i)).collect();
            crate::materials::merge_material_xml(&mut idx.materials, &xmls, &imgs);
        }

        // Styles, then the document's own rendering options on top. The
        // options section is what the viewport actually reads, so it wins
        // where the two disagree - which they do, because a file can carry a
        // style whose settings were never applied.
        if let Some(p) = &self.sections.styles {
            idx.styles = crate::styles::parse_styles(p);
            idx.active_style = crate::styles::selected_style_id(p);
        }
        if let Some(p) = &self.sections.rendering_options {
            let opts = crate::styles::parse_rendering_options(p);
            match idx
                .active_style
                .and_then(|id| idx.styles.iter().position(|s| s.id == id))
            {
                Some(k) => {
                    let name = idx.styles[k].name.clone();
                    let id = idx.styles[k].id;
                    idx.styles[k] = StyleDef { id, name, ..opts };
                }
                None => {
                    idx.styles.push(opts);
                    idx.active_style = idx.styles.last().map(|s| s.id);
                }
            }
        }

        // Tag colours first, because that is what the `Layer_<name>`
        // materials are for - and then drop them, because they are not
        // materials anybody paints with and SketchUp does not list them.
        // Order matters: filtering before this would leave every tag grey.
        crate::layers::apply_layer_colors(&mut idx.layers, &idx.materials);
        idx.materials
            .retain(|m| !crate::materials::is_layer_material(&m.name));
        idx.counts.textures = idx.materials.iter().filter(|m| m.texture.is_some()).count() as u32;
        idx
    }

    /// The document camera: where the file opens, which is not one of the
    /// saved scenes.
    pub fn document_camera(&self) -> Option<crate::model::SceneDef> {
        self.sections
            .camera
            .as_deref()
            .and_then(crate::pages::parse_camera)
    }
}

const _: () = {
    // NO_LAYER and NO_MATERIAL must stay distinguishable from real ids.
    assert!(NO_LAYER == u32::MAX);
    assert!(NO_MATERIAL == u32::MAX);
};

// ---------------------------------------------------------------------------
// Collapsing the long tail
// ---------------------------------------------------------------------------

/// Bake definitions that are barely placed into shared world-space buffers.
///
/// Instancing is the right answer when a chair is placed four thousand times.
/// It is the wrong answer when a definition is placed once, which on a real
/// commercial pack is most of them: of 4,147 definitions in a 517 MB
/// restaurant, 3,435 are placed exactly once and 3,774 at most twice. Keeping
/// those as separate instanced batches costs a draw call and a worker message
/// each - 4,764 draw calls and 8,911 messages for a model whose per-frame
/// triangle count is only 2.3 million. The browser does not fall over on the
/// geometry; it falls over on the bookkeeping.
///
/// So anything placed `max_placements` times or fewer gets its placements
/// baked into the root's own buffers, grouped by material and tag. The
/// duplication is bounded by that threshold and the draw calls collapse.
///
/// Edges carry two adjacent-face normals, and those have to be transformed
/// too: the silhouette test consumes them. For a rigid placement this is the
/// same answer in world space as in local space; for a scaled one the
/// inverse-transpose is what keeps it the same.
pub fn merge_small(l: &mut Loaded, max_placements: usize) -> MergeReport {
    let mut rep = MergeReport::default();

    // Which definitions are we collapsing, and what paint was each placement
    // carrying? The paint has to come along: a baked placement loses its
    // instance row, so a component painted from the outliner would arrive in
    // the root's buffers with no material at all and draw default-white. On
    // the school cafeteria that is a million triangles of tan chairs.
    let mut victims: BTreeMap<u32, Vec<([f32; 16], u32)>> = BTreeMap::new();
    l.instances.retain(|t| {
        if t.definition_id == ROOT_DEFINITION || t.len() > max_placements || t.is_empty() {
            return true;
        }
        let mut mats = Vec::with_capacity(t.len());
        for i in 0..t.len() {
            let mut m = [0.0f32; 16];
            m.copy_from_slice(&t.matrices[i * 16..i * 16 + 16]);
            mats.push((m, t.material_ids.get(i).copied().unwrap_or(NO_MATERIAL)));
        }
        victims.insert(t.definition_id, mats);
        false
    });
    if victims.is_empty() {
        return rep;
    }
    rep.definitions = victims.len() as u32;

    // Faces, grouped by (material, tag) so tag visibility and paint survive.
    let mut merged_meshes: BTreeMap<(u32, u32), MeshBatch> = BTreeMap::new();
    let mut kept_meshes: Vec<MeshBatch> = Vec::new();
    for b in core::mem::take(&mut l.meshes) {
        let Some(mats) = victims.get(&b.definition_id) else {
            kept_meshes.push(b);
            continue;
        };
        for (m, paint) in mats {
            // SketchUp's rule: a face with no material of its own is painted
            // by the instance that places it. That is resolved here rather
            // than in the renderer because here is where the instance row
            // stops existing.
            let mat = if b.material_id == NO_MATERIAL && *paint != NO_MATERIAL {
                *paint
            } else {
                b.material_id
            };
            let out = merged_meshes
                .entry((mat, b.layer_id))
                .or_insert_with(|| MeshBatch {
                    definition_id: ROOT_DEFINITION,
                    material_id: mat,
                    layer_id: b.layer_id,
                    ..Default::default()
                });
            let base = (out.positions.len() / 3) as u32;
            let n = b.positions.len() / 3;
            // UVs pass through untouched. They are a property of the face and
            // its material, not of where the definition was put: scaling a
            // component scales its geometry and its texture together, which is
            // what SketchUp does and what leaving them alone reproduces.
            //
            // Both sides of this merge agree because `wants_uv` is a function
            // of the material id and the key here starts with the material id,
            // so a batch with UVs never lands in the same bucket as one
            // without. The guard is still here because a malformed pairing
            // would otherwise shift every UV after it by one vertex.
            let uvs_agree = out.uvs.len() * 3 == out.positions.len() * 2;
            for v in 0..n {
                let p = [
                    b.positions[v * 3],
                    b.positions[v * 3 + 1],
                    b.positions[v * 3 + 2],
                ];
                let w = xform_point(m, p);
                out.positions.extend_from_slice(&w);
                let nn = [b.normals[v * 3], b.normals[v * 3 + 1], b.normals[v * 3 + 2]];
                out.normals.extend_from_slice(&xform_normal(m, nn));
                if uvs_agree && b.uvs.len() >= (v + 1) * 2 {
                    out.uvs.push(b.uvs[v * 2]);
                    out.uvs.push(b.uvs[v * 2 + 1]);
                }
            }
            out.indices.extend(b.indices.iter().map(|i| i + base));
            rep.triangles += (b.indices.len() / 3) as u64;
        }
    }
    l.meshes = kept_meshes;
    l.meshes.extend(merged_meshes.into_values());

    // Edges, grouped by tag. Drawable ones stay in front, as they are in a
    // per-definition batch, so the renderer's prefix draw still works.
    // Three buckets per tag, not two: drawable, softened-and-therefore-only-a-
    // silhouette, and hidden. They have to stay separable because the renderer
    // draws the first always, the second only when silhouettes are on, and the
    // third never. Merging the last two was what made a softened chair back
    // undrawable.
    let mut merged_draw: BTreeMap<u32, EdgeBatch> = BTreeMap::new();
    let mut merged_soft: BTreeMap<u32, EdgeBatch> = BTreeMap::new();
    let mut merged_rest: BTreeMap<u32, EdgeBatch> = BTreeMap::new();
    let mut kept_edges: Vec<EdgeBatch> = Vec::new();
    for b in core::mem::take(&mut l.edges) {
        let Some(mats) = victims.get(&b.definition_id) else {
            kept_edges.push(b);
            continue;
        };
        for (m, _) in mats {
            for e in 0..b.len() {
                let tag = b.layer_ids.get(e).copied().unwrap_or(u32::MAX);
                let dst = if (e as u32) < b.drawable {
                    &mut merged_draw
                } else if (e as u32) < b.silhouette {
                    &mut merged_soft
                } else {
                    &mut merged_rest
                };
                let out = dst.entry(tag).or_insert_with(|| EdgeBatch {
                    definition_id: ROOT_DEFINITION,
                    ..Default::default()
                });
                let p0 = [
                    b.positions[e * 6],
                    b.positions[e * 6 + 1],
                    b.positions[e * 6 + 2],
                ];
                let p1 = [
                    b.positions[e * 6 + 3],
                    b.positions[e * 6 + 4],
                    b.positions[e * 6 + 5],
                ];
                out.positions.extend_from_slice(&xform_point(m, p0));
                out.positions.extend_from_slice(&xform_point(m, p1));
                let n0 = [
                    b.face_normals[e * 6],
                    b.face_normals[e * 6 + 1],
                    b.face_normals[e * 6 + 2],
                ];
                let n1 = [
                    b.face_normals[e * 6 + 3],
                    b.face_normals[e * 6 + 4],
                    b.face_normals[e * 6 + 5],
                ];
                out.face_normals.extend_from_slice(&xform_normal(m, n0));
                out.face_normals.extend_from_slice(&xform_normal(m, n1));
                out.flags.push(b.flags[e]);
                out.face_counts.push(b.face_counts[e]);
                out.layer_ids.push(tag);
                rep.edges += 1;
            }
        }
    }
    l.edges = kept_edges;
    let append = |d: &mut EdgeBatch, r: &EdgeBatch| {
        d.positions.extend_from_slice(&r.positions);
        d.face_normals.extend_from_slice(&r.face_normals);
        d.flags.extend_from_slice(&r.flags);
        d.face_counts.extend_from_slice(&r.face_counts);
        d.layer_ids.extend_from_slice(&r.layer_ids);
    };
    let tags: alloc::collections::BTreeSet<u32> = merged_draw
        .keys()
        .chain(merged_soft.keys())
        .chain(merged_rest.keys())
        .copied()
        .collect();
    for tag in tags {
        let mut d = merged_draw.remove(&tag).unwrap_or_else(|| EdgeBatch {
            definition_id: ROOT_DEFINITION,
            ..Default::default()
        });
        d.drawable = d.flags.len() as u32;
        if let Some(r) = merged_soft.remove(&tag) {
            append(&mut d, &r);
        }
        d.silhouette = d.flags.len() as u32;
        if let Some(r) = merged_rest.remove(&tag) {
            append(&mut d, &r);
        }
        l.edges.push(d);
    }

    for b in &mut l.edges {
        crate::edges::sort_drawable_first(b);
    }
    rep.batches_after = (l.meshes.len() + l.edges.len()) as u32;
    rep
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MergeReport {
    pub definitions: u32,
    pub triangles: u64,
    pub edges: u64,
    pub batches_after: u32,
}

#[inline]
fn xform_point(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
}

/// Inverse-transpose of the upper-left 3x3, applied and renormalised.
///
/// The cheap thing - multiplying by the matrix itself - is right only when the
/// placement is a rotation. A mirrored or non-uniformly scaled component is
/// common in a real model and would come out with its normals pointing the
/// wrong way, which for the silhouette test means an outline that is
/// inside-out on exactly those parts.
#[inline]
fn xform_normal(m: &[f32; 16], n: [f32; 3]) -> [f32; 3] {
    let a = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]];
    let det = a[0] * (a[4] * a[8] - a[5] * a[7]) - a[1] * (a[3] * a[8] - a[5] * a[6])
        + a[2] * (a[3] * a[7] - a[4] * a[6]);
    if det.abs() < 1e-20 {
        return n;
    }
    let inv = 1.0 / det;
    // cofactor transpose, i.e. the inverse-transpose up to the determinant's
    // sign, which the normalisation below removes anyway except for mirroring.
    let c = [
        (a[4] * a[8] - a[5] * a[7]) * inv,
        (a[5] * a[6] - a[3] * a[8]) * inv,
        (a[3] * a[7] - a[4] * a[6]) * inv,
        (a[2] * a[7] - a[1] * a[8]) * inv,
        (a[0] * a[8] - a[2] * a[6]) * inv,
        (a[1] * a[6] - a[0] * a[7]) * inv,
        (a[1] * a[5] - a[2] * a[4]) * inv,
        (a[2] * a[3] - a[0] * a[5]) * inv,
        (a[0] * a[4] - a[1] * a[3]) * inv,
    ];
    let mut o = [
        c[0] * n[0] + c[1] * n[1] + c[2] * n[2],
        c[3] * n[0] + c[4] * n[1] + c[5] * n[2],
        c[6] * n[0] + c[7] * n[1] + c[8] * n[2],
    ];
    let len = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
    if len > 0.0 {
        o[0] /= len;
        o[1] /= len;
        o[2] /= len;
    }
    o
}
