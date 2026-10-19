//! A whole document into the blank SketchUp 2017 scaffold. Ported from openskp's
//! `SkpBuilder` (`create.py`): the same splice points and patches, written in one pass
//! because the whole document is known up front. See `PORTED.md`.
//!
//! The file is the scaffold cut at its empty sections, with ours spliced in:
//!
//! ```text
//! scaffold[..3396]          prefix; persistent-id counter at 1987, Iso camera at 2993
//! u32 material count        overwrites the scaffold's 0 at 3396
//! materials                 slots from 9
//! scaffold[3400..3505]      layer marker, layer count, CLayer and Layer0 (2 slots)
//! scaffold[3505..3507]      active-layer anchor: Layer0's slot, moved by the materials
//! u32 definition count, definitions
//! u32 root count, root entities
//! scaffold[3515..]          the tail; six slot references move by every slot added
//! ```

use crate::SCAFFOLD;
use crate::archive::{
    Archive, DrawFlags, ImageKind, MAX_STRING_UNITS, TextureRecord, Use, UvSides,
};
use crate::doc::{Doc, Edge, Face, Geometry, Guide, Instance, UvPin};
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Where the material count sits, right before the material list.
const MATERIAL_COUNT_POS: usize = 3396;
/// Where the material list goes, and where the layer section starts.
const MATERIAL_INSERT_POS: usize = 3400;
/// The first slot after the prefix: where the materials start numbering.
const BASE: u32 = 9;
/// The layer count, which stays 1 (Layer0).
const LAYER_COUNT_POS: usize = 3405;
/// The active-layer anchor, a back-reference to Layer0.
const ACTIVE_LAYER_ANCHOR_POS: usize = 3505;
const DEF_COUNT_POS: usize = 3507;
const ROOT_COUNT_POS: usize = 3511;
const TAIL_POS: usize = 3515;
/// The slots the scaffold's layer section takes: `CLayer` and Layer0.
const LAYER_SECTION_SLOTS: u32 = 2;
/// The scaffold's next slot after its (empty) definition and root lists.
const SCAFFOLD_NEXT_SLOT: u32 = BASE + LAYER_SECTION_SLOTS;
/// The `u32` holding the last persistent id handed out, for the whole file.
const PID_COUNTER_POS: usize = 1987;
/// Tail offsets of slot references that move by every slot inserted before them.
const TAIL_REF_POSITIONS: [usize; 6] = [409, 468, 477, 479, 1383, 1385];

/// The standard Iso camera, over the scaffold's default one.
const ISO_CAMERA_PREFIX_POS: usize = 2993;
const ISO_CAMERA_PREFIX: [u8; 98] = [
    89, 64, 0, 0, 0, 0, 0, 0, 89, 192, 0, 0, 0, 0, 0, 0, 89, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 63, 44, 12, 112, 189, 32, 218, 191, 63, 44, 12, 112,
    189, 32, 218, 63, 63, 44, 12, 112, 189, 32, 234, 63, 0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0,
    64, 143, 64, 0, 0, 0, 0, 0, 0, 0, 62, 64, 42, 223, 39, 44, 128, 52, 87,
];
const ISO_CAMERA_TAIL_1: (usize, [u8; 16]) = (
    509,
    [
        208, 168, 105, 97, 60, 68, 45, 71, 153, 164, 102, 125, 26, 223, 168, 54,
    ],
);
const ISO_CAMERA_TAIL_2: (usize, [u8; 15]) = (
    1390,
    [
        78, 83, 200, 68, 119, 2, 146, 70, 187, 169, 88, 39, 187, 167, 226,
    ],
);

/// Every definition's timestamp: a fixed 2026-01-01T00:00:00Z, so the same document
/// gives the same bytes.
const TIMESTAMP: u32 = 1_767_225_600;

/// Faces whose normal is within this of vertical take the world axes for their texture
/// basis, as SketchUp does (openskp's `VERTICAL_TOLERANCE`, measured).
const VERTICAL_TOLERANCE: f64 = 1e-3;

/// An instance whose 3x3 has a determinant smaller than this is flat: refused. The
/// project's kernel and reader use the same bound.
const MIN_DETERMINANT: f64 = 1e-12;

/// Which entity list an error is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Root,
    /// Index into `Doc::definitions`.
    Definition(usize),
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Owner::Root => write!(f, "the root"),
            Owner::Definition(d) => write!(f, "definition {d}"),
        }
    }
}

/// Why a document cannot be written.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteError {
    /// A face with no loop, or a loop with fewer than three vertices.
    ShortLoop { owner: Owner, face: usize },
    /// An edge or a face loop names a vertex the geometry does not have.
    VertexOutOfRange { owner: Owner, index: u32 },
    /// Two consecutive vertices of a face loop with no edge between them in `edges`.
    MissingEdge {
        owner: Owner,
        face: usize,
        a: u32,
        b: u32,
    },
    /// A definition places itself, directly or through others.
    RecursiveDefinition { definition: usize },
    /// An instance of a definition the document does not have.
    DefinitionOutOfRange {
        owner: Owner,
        instance: usize,
        definition: usize,
    },
    /// An instance of a definition with nothing in it, which is not written.
    EmptyDefinition {
        owner: Owner,
        instance: usize,
        definition: usize,
    },
    /// A face or an instance names a material the document does not have.
    MaterialOutOfRange { index: usize },
    /// A texture that is neither PNG nor JPEG.
    UnknownTextureFormat { material: usize },
    /// A name longer than a string record holds.
    StringTooLong { what: &'static str, units: usize },
    /// A NaN or an infinity.
    NonFinite { what: &'static str },
    /// Geometry with no extent: a face of no area, an edge from a vertex to itself, a
    /// guide line with no direction, texture pins on one line.
    Degenerate {
        owner: Owner,
        what: &'static str,
        index: usize,
    },
    /// Nothing in the root.
    EmptyDocument,
    /// More materials than an entity's material reference can name.
    TooManyMaterials,
    /// A texture whose width or height is not more than zero.
    TextureSize { material: usize },
    /// Two materials, or two definitions, with the same name.
    DuplicateName { what: &'static str, name: String },
    /// A count or a length past what its `u32` field holds.
    TooLarge { what: &'static str },
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShortLoop { owner, face } => {
                write!(
                    f,
                    "face {face} of {owner} has a loop of fewer than 3 vertices"
                )
            }
            Self::VertexOutOfRange { owner, index } => {
                write!(f, "{owner} has no vertex {index}")
            }
            Self::MissingEdge { owner, face, a, b } => write!(
                f,
                "face {face} of {owner} runs from vertex {a} to {b} with no edge between them"
            ),
            Self::RecursiveDefinition { definition } => {
                write!(f, "definition {definition} places itself")
            }
            Self::DefinitionOutOfRange {
                owner,
                instance,
                definition,
            } => write!(
                f,
                "instance {instance} of {owner} places definition {definition}, which does not exist"
            ),
            Self::EmptyDefinition {
                owner,
                instance,
                definition,
            } => write!(
                f,
                "instance {instance} of {owner} places definition {definition}, which is empty"
            ),
            Self::MaterialOutOfRange { index } => write!(f, "there is no material {index}"),
            Self::UnknownTextureFormat { material } => {
                write!(f, "material {material}'s texture is neither PNG nor JPEG")
            }
            Self::StringTooLong { what, units } => write!(
                f,
                "a {what} is {units} UTF-16 units long; at most {MAX_STRING_UNITS} fit"
            ),
            Self::NonFinite { what } => write!(f, "a {what} is not a finite number"),
            Self::Degenerate { owner, what, index } => {
                write!(f, "{what} {index} of {owner} is degenerate")
            }
            Self::EmptyDocument => write!(f, "the document has nothing in its root"),
            Self::TooManyMaterials => write!(f, "too many materials for one file"),
            Self::TextureSize { material } => {
                write!(f, "material {material}'s texture has no width or height")
            }
            Self::DuplicateName { what, name } => {
                write!(f, "two {what}s are named \"{name}\"")
            }
            Self::TooLarge { what } => write!(f, "a {what} is too large for the file"),
        }
    }
}

impl std::error::Error for WriteError {}

/// Write `doc` as a SketchUp 2017 file.
pub fn write(doc: &Doc) -> Result<Vec<u8>, WriteError> {
    let plan = Plan::new(doc)?;
    let pid_start = u64::from(read_u32(SCAFFOLD, PID_COUNTER_POS)) + 1;
    let mut ar = Archive::new(BASE, pid_start);

    // Materials, numbered from the scaffold's base.
    let mut material_slots = Vec::with_capacity(doc.materials.len());
    for (i, m) in doc.materials.iter().enumerate() {
        let path = plan.texture_paths[i].as_deref().unwrap_or("");
        let texture = m.texture.as_ref().and_then(|t| {
            Some(TextureRecord {
                kind: ImageKind::of(&t.bytes)?,
                bytes: &t.bytes,
                width: t.width,
                height: t.height,
                path,
            })
        });
        let opacity = (m.opacity < 1.0).then(|| m.opacity.max(0.0));
        material_slots.push(ar.material(&m.name, m.rgba, opacity, texture));
    }
    if material_slots.iter().any(|s| *s >= 0x7fff) {
        return Err(WriteError::TooManyMaterials);
    }
    let materials_buf = std::mem::take(&mut ar.buf);
    let material_shift = ar.next_slot - BASE;

    // The scaffold's layer section follows: its `CLayer` declaration and Layer0, moved by
    // the materials. Nothing declared among the materials is needed again.
    ar.class_slot.clear();
    ar.class_slot.insert("CLayer", BASE + material_shift);
    ar.next_slot += LAYER_SECTION_SLOTS;

    let mut def_slots: Vec<Option<u32>> = vec![None; doc.definitions.len()];
    let ctx = Ctx {
        doc,
        plan: &plan,
        material_slots: &material_slots,
    };
    for &d in &plan.order {
        let (slot, count_at) = ar.definition_start();
        let def = &doc.definitions[d];
        let count = ctx.entities(&mut ar, Owner::Definition(d), &def.geometry, &def_slots);
        ar.patch_u32(count_at, count);
        ar.definition_end(plan.def_guids[d], &def.name, TIMESTAMP);
        def_slots[d] = Some(slot);
    }
    let defs_buf = std::mem::take(&mut ar.buf);
    let root_count = ctx.entities(&mut ar, Owner::Root, &doc.root, &def_slots);
    let root_buf = std::mem::take(&mut ar.buf);

    let total_shift = ar.next_slot - SCAFFOLD_NEXT_SLOT;
    let last_pid = u32::try_from(ar.next_pid - 1).unwrap_or(u32::MAX);

    let mut out = Vec::with_capacity(
        SCAFFOLD.len() + materials_buf.len() + defs_buf.len() + root_buf.len() + 64,
    );
    out.extend_from_slice(&SCAFFOLD[..MATERIAL_COUNT_POS]);
    put_u32(&mut out, PID_COUNTER_POS, last_pid);
    put(&mut out, ISO_CAMERA_PREFIX_POS, &ISO_CAMERA_PREFIX);
    out.extend_from_slice(&plan.material_count.to_le_bytes());
    out.extend_from_slice(&materials_buf);

    let layers_at = out.len();
    out.extend_from_slice(&SCAFFOLD[MATERIAL_INSERT_POS..ACTIVE_LAYER_ANCHOR_POS]);
    put_u32(
        &mut out,
        layers_at + (LAYER_COUNT_POS - MATERIAL_INSERT_POS),
        1,
    );

    let mut anchor = SCAFFOLD[ACTIVE_LAYER_ANCHOR_POS..DEF_COUNT_POS].to_vec();
    shift_ref(&mut anchor, 0, material_shift);
    out.extend_from_slice(&anchor);

    out.extend_from_slice(&plan.def_count.to_le_bytes());
    out.extend_from_slice(&defs_buf);
    out.extend_from_slice(&SCAFFOLD[DEF_COUNT_POS + 4..ROOT_COUNT_POS]);
    out.extend_from_slice(&root_count.to_le_bytes());
    out.extend_from_slice(&SCAFFOLD[ROOT_COUNT_POS + 4..TAIL_POS]);
    out.extend_from_slice(&root_buf);

    // The tail, patched in offset order: a reference that widens to the escape grows the
    // buffer, moving everything after it.
    let mut tail = SCAFFOLD[TAIL_POS..].to_vec();
    let mut growth = 0;
    for pos in TAIL_REF_POSITIONS.iter().take(4) {
        growth += shift_ref(&mut tail, pos + growth, total_shift);
    }
    put(
        &mut tail,
        ISO_CAMERA_TAIL_1.0 + growth,
        &ISO_CAMERA_TAIL_1.1,
    );
    for pos in TAIL_REF_POSITIONS.iter().skip(4) {
        growth += shift_ref(&mut tail, pos + growth, total_shift);
    }
    put(
        &mut tail,
        ISO_CAMERA_TAIL_2.0 + growth,
        &ISO_CAMERA_TAIL_2.1,
    );
    out.extend_from_slice(&tail);
    Ok(out)
}

fn read_u32(b: &[u8], at: usize) -> u32 {
    let mut w = [0u8; 4];
    if let Some(s) = b.get(at..at + 4) {
        w.copy_from_slice(s);
    }
    u32::from_le_bytes(w)
}

fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
    if let Some(s) = buf.get_mut(at..at + bytes.len()) {
        s.copy_from_slice(bytes);
    }
}

fn put_u32(buf: &mut [u8], at: usize, v: u32) {
    put(buf, at, &v.to_le_bytes());
}

/// Renumber the `u16` slot reference at `at` by `shift`, keeping its class-ref bit, and
/// widen it to the escape when the new slot no longer fits. Returns how many bytes it
/// grew (0 or 4).
fn shift_ref(buf: &mut Vec<u8>, at: usize, shift: u32) -> usize {
    let Some(s) = buf.get(at..at + 2) else {
        return 0;
    };
    let tag = u16::from_le_bytes([s[0], s[1]]);
    let class_bit = tag & 0x8000;
    let slot = u32::from(tag & 0x7fff) + shift;
    if slot < 0x7fff {
        put(buf, at, &(class_bit | slot as u16).to_le_bytes());
        return 0;
    }
    let big = if class_bit != 0 {
        0x8000_0000 | slot
    } else {
        slot
    };
    let mut wide = 0x7fffu16.to_le_bytes().to_vec();
    wide.extend_from_slice(&big.to_le_bytes());
    buf.splice(at..at + 2, wide);
    4
}

// -- validation -----------------------------------------------------------

/// What the writing pass needs, worked out and checked before a byte is written.
struct Plan {
    /// Definitions to write, each after every definition it places.
    order: Vec<usize>,
    /// How many there are, as the file's count.
    def_count: u32,
    /// How many materials there are, as the file's count.
    material_count: u32,
    def_guids: Vec<[u8; 16]>,
    root: GeomPlan,
    defs: Vec<GeomPlan>,
    /// A textured material's stored file name.
    texture_paths: Vec<Option<String>>,
}

#[derive(Default)]
struct GeomPlan {
    /// Per face: its plane, its loops as (edge index, reversed), its texture matrices.
    faces: Vec<FacePlan>,
    /// Per guide line: the unit direction.
    directions: Vec<[f64; 3]>,
}

struct FacePlan {
    plane: [f64; 4],
    loops: Vec<Vec<(usize, bool)>>,
    uv: Option<UvSides>,
}

impl Plan {
    fn new(doc: &Doc) -> Result<Self, WriteError> {
        let mut texture_paths = Vec::with_capacity(doc.materials.len());
        let mut names = HashSet::with_capacity(doc.materials.len());
        for (i, m) in doc.materials.iter().enumerate() {
            check_string("material name", &m.name)?;
            unique(&mut names, "material", &m.name)?;
            finite("material opacity", &[m.opacity])?;
            match &m.texture {
                None => texture_paths.push(None),
                Some(t) => {
                    let kind = ImageKind::of(&t.bytes)
                        .ok_or(WriteError::UnknownTextureFormat { material: i })?;
                    finite("texture size", &[t.width, t.height])?;
                    if t.width <= 0.0 || t.height <= 0.0 {
                        return Err(WriteError::TextureSize { material: i });
                    }
                    fits_u32("texture image", t.bytes.len())?;
                    texture_paths.push(Some(texture_path(&m.name, kind)));
                }
            }
        }
        let material_count = fits_u32("material list", doc.materials.len())?;
        let empty: Vec<bool> = doc
            .definitions
            .iter()
            .map(|d| entity_count(&d.geometry) == 0)
            .collect();
        let mut defs = Vec::with_capacity(doc.definitions.len());
        let mut names = HashSet::with_capacity(doc.definitions.len());
        for (i, d) in doc.definitions.iter().enumerate() {
            check_string("definition name", &d.name)?;
            unique(&mut names, "definition", &d.name)?;
            defs.push(plan_geometry(
                doc,
                Owner::Definition(i),
                &d.geometry,
                &empty,
            )?);
        }
        let root = plan_geometry(doc, Owner::Root, &doc.root, &empty)?;
        if entity_count(&doc.root) == 0 {
            return Err(WriteError::EmptyDocument);
        }
        let order = order_definitions(doc, &empty)?;
        let def_count = fits_u32("definition list", order.len())?;
        let def_guids = doc
            .definitions
            .iter()
            .enumerate()
            .map(|(i, d)| definition_guid(i, d))
            .collect();
        Ok(Self {
            order,
            def_count,
            material_count,
            def_guids,
            root,
            defs,
            texture_paths,
        })
    }

    fn geometry(&self, owner: Owner) -> &GeomPlan {
        match owner {
            Owner::Root => &self.root,
            Owner::Definition(d) => &self.defs[d],
        }
    }
}

fn entity_count(g: &Geometry) -> usize {
    g.edges.len() + g.faces.len() + g.instances.len() + g.guides.len()
}

fn check_string(what: &'static str, s: &str) -> Result<(), WriteError> {
    let units = s.encode_utf16().count();
    if units > MAX_STRING_UNITS {
        return Err(WriteError::StringTooLong { what, units });
    }
    Ok(())
}

/// Refuses a second use of a name.
fn unique<'a>(
    names: &mut HashSet<&'a str>,
    what: &'static str,
    name: &'a str,
) -> Result<(), WriteError> {
    if names.insert(name) {
        Ok(())
    } else {
        Err(WriteError::DuplicateName {
            what,
            name: name.to_string(),
        })
    }
}

/// `n` as the `u32` a count or length is stored in.
fn fits_u32(what: &'static str, n: usize) -> Result<u32, WriteError> {
    u32::try_from(n).map_err(|_| WriteError::TooLarge { what })
}

fn finite(what: &'static str, vs: &[f64]) -> Result<(), WriteError> {
    if vs.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(WriteError::NonFinite { what })
    }
}

/// The file name a textured material stores: its own name with the image's extension,
/// cut to fit a string record.
fn texture_path(name: &str, kind: ImageKind) -> String {
    let ext = kind.extension();
    let room = MAX_STRING_UNITS - ext.len();
    let mut out = String::new();
    let mut units = 0;
    for c in name.chars() {
        if units + c.len_utf16() > room {
            break;
        }
        units += c.len_utf16();
        out.push(c);
    }
    out.push_str(ext);
    out
}

fn material_in_range(doc: &Doc, m: Option<usize>) -> Result<(), WriteError> {
    match m {
        Some(index) if index >= doc.materials.len() => {
            Err(WriteError::MaterialOutOfRange { index })
        }
        _ => Ok(()),
    }
}

fn plan_geometry(
    doc: &Doc,
    owner: Owner,
    g: &Geometry,
    empty: &[bool],
) -> Result<GeomPlan, WriteError> {
    for v in &g.vertices {
        finite("vertex position", v)?;
    }
    let nv = g.vertices.len();
    let mut by_pair: HashMap<(u32, u32), usize> = HashMap::with_capacity(g.edges.len());
    for (i, e) in g.edges.iter().enumerate() {
        for index in [e.a, e.b] {
            if index as usize >= nv {
                return Err(WriteError::VertexOutOfRange { owner, index });
            }
        }
        if e.a == e.b {
            return Err(WriteError::Degenerate {
                owner,
                what: "edge",
                index: i,
            });
        }
        if by_pair.insert((e.a.min(e.b), e.a.max(e.b)), i).is_some() {
            return Err(WriteError::Degenerate {
                owner,
                what: "duplicate edge",
                index: i,
            });
        }
    }

    let mut faces = Vec::with_capacity(g.faces.len());
    for (fi, f) in g.faces.iter().enumerate() {
        faces.push(plan_face(doc, owner, g, fi, f, &by_pair)?);
    }

    for (i, inst) in g.instances.iter().enumerate() {
        check_instance(doc, owner, i, inst, empty)?;
    }

    let mut directions = Vec::new();
    for (i, guide) in g.guides.iter().enumerate() {
        match guide {
            Guide::Line { point, direction } => {
                finite("guide point", point)?;
                finite("guide direction", direction)?;
                let len = dot(*direction, *direction).sqrt();
                if !len.is_finite() || len <= 0.0 {
                    return Err(WriteError::Degenerate {
                        owner,
                        what: "guide",
                        index: i,
                    });
                }
                directions.push(direction.map(|c| c / len));
            }
            Guide::Point { position } => finite("guide point", position)?,
        }
    }
    Ok(GeomPlan { faces, directions })
}

fn check_instance(
    doc: &Doc,
    owner: Owner,
    i: usize,
    inst: &Instance,
    empty: &[bool],
) -> Result<(), WriteError> {
    let definition = inst.definition;
    match empty.get(definition) {
        None => {
            return Err(WriteError::DefinitionOutOfRange {
                owner,
                instance: i,
                definition,
            });
        }
        Some(true) => {
            return Err(WriteError::EmptyDefinition {
                owner,
                instance: i,
                definition,
            });
        }
        Some(false) => {}
    }
    for row in &inst.rotation {
        finite("instance rotation", row)?;
    }
    finite("instance translation", &inst.translation)?;
    let det = det3(&inst.rotation);
    if !det.is_finite() || det.abs() < MIN_DETERMINANT {
        return Err(WriteError::Degenerate {
            owner,
            what: "instance",
            index: i,
        });
    }
    material_in_range(doc, inst.material)
}

fn plan_face(
    doc: &Doc,
    owner: Owner,
    g: &Geometry,
    fi: usize,
    f: &Face,
    by_pair: &HashMap<(u32, u32), usize>,
) -> Result<FacePlan, WriteError> {
    material_in_range(doc, f.front)?;
    material_in_range(doc, f.back)?;
    if f.loops.is_empty() {
        return Err(WriteError::ShortLoop { owner, face: fi });
    }
    fits_u32("face's loop list", f.loops.len())?;
    let mut loops = Vec::with_capacity(f.loops.len());
    let mut seen = HashSet::new();
    for ring in &f.loops {
        if ring.len() < 3 {
            return Err(WriteError::ShortLoop { owner, face: fi });
        }
        seen.clear();
        if !ring.iter().all(|v| seen.insert(*v)) {
            return Err(WriteError::Degenerate {
                owner,
                what: "face loop",
                index: fi,
            });
        }
        let mut uses = Vec::with_capacity(ring.len());
        for k in 0..ring.len() {
            let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
            for index in [a, b] {
                if index as usize >= g.vertices.len() {
                    return Err(WriteError::VertexOutOfRange { owner, index });
                }
            }
            let e = *by_pair
                .get(&(a.min(b), a.max(b)))
                .ok_or(WriteError::MissingEdge {
                    owner,
                    face: fi,
                    a,
                    b,
                })?;
            uses.push((e, g.edges[e].a != a));
        }
        loops.push(uses);
    }
    let outer: Vec<[f64; 3]> = f.loops[0].iter().map(|v| g.vertices[*v as usize]).collect();
    let degenerate = |what| WriteError::Degenerate {
        owner,
        what,
        index: fi,
    };
    let plane = newell_plane(&outer).ok_or(degenerate("face"))?;
    let normal = [plane[0], plane[1], plane[2]];
    let side = |pins: &Option<[UvPin; 3]>, material: Option<usize>| -> Result<_, WriteError> {
        let Some(pins) = pins else {
            return Ok(None);
        };
        for (p, uv) in pins {
            finite("texture pin", p)?;
            finite("texture pin", uv)?;
        }
        let size = material
            .and_then(|m| doc.materials.get(m))
            .and_then(|m| m.texture.as_ref())
            .map(|t| [t.width, t.height]);
        uv_matrix(pins, size, normal)
            .map(Some)
            .ok_or(degenerate("texture pins of face"))
    };
    let front = side(&f.front_uv, f.front)?;
    let back = side(&f.back_uv, f.back)?;
    let uv = (front.is_some() || back.is_some()).then_some((front, back));
    Ok(FacePlan { plane, loops, uv })
}

/// The plane of a polygon by Newell's method, which holds for concave polygons: the unit
/// normal and `d` with `n . p + d = 0`, through the centroid. `None` when it has no area.
fn newell_plane(points: &[[f64; 3]]) -> Option<[f64; 4]> {
    let n = points.len();
    let mut nrm = [0.0f64; 3];
    let mut c = [0.0f64; 3];
    for i in 0..n {
        let (p, q) = (points[i], points[(i + 1) % n]);
        nrm[0] += (p[1] - q[1]) * (p[2] + q[2]);
        nrm[1] += (p[2] - q[2]) * (p[0] + q[0]);
        nrm[2] += (p[0] - q[0]) * (p[1] + q[1]);
        for k in 0..3 {
            c[k] += p[k];
        }
    }
    let len = dot(nrm, nrm).sqrt();
    if !len.is_finite() || len < 1e-9 {
        return None;
    }
    let nrm = nrm.map(|v| v / len);
    let c = c.map(|v| v / n as f64);
    Some([nrm[0], nrm[1], nrm[2], -dot(nrm, c)])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// SketchUp's texture basis for a face normal: `Z x n` and `n x` that, or the world axes
/// for a face within `VERTICAL_TOLERANCE` of horizontal - turned half round for a face
/// looking down (openskp's `face_uv_basis`).
fn uv_basis(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let (cx, cy) = (-n[1], n[0]);
    let clen = (cx * cx + cy * cy).sqrt();
    if clen < VERTICAL_TOLERANCE {
        let x = if n[2] >= 0.0 { 1.0 } else { -1.0 };
        ([x, 0.0, 0.0], [0.0, 1.0, 0.0])
    } else {
        let xr = [cx / clen, cy / clen, 0.0];
        let yr = [
            n[1] * xr[2] - n[2] * xr[1],
            n[2] * xr[0] - n[0] * xr[2],
            n[0] * xr[1] - n[1] * xr[0],
        ];
        (xr, yr)
    }
}

/// The matrix SketchUp stores for a positioned texture, from three pins: it takes
/// `(u, v, 1)`, in inches of texture space, to the pin's position in the face's texture
/// basis, row-vector convention. Pins are in tiles; `size` is the tile in inches.
/// `None` when the pins' coordinates are on one line, judged against their own spread so
/// that a tiny face keeps its placement.
fn uv_matrix(pins: &[UvPin; 3], size: Option<[f64; 2]>, normal: [f64; 3]) -> Option<[f64; 9]> {
    let (ux, wx) = uv_basis(normal);
    let [sw, sh] = size.unwrap_or([1.0, 1.0]);
    let a = pins.map(|(_, uv)| [uv[0] * sw, uv[1] * sh, 1.0]);
    if !spread_in_2d(&a) {
        return None;
    }
    let bx = pins.map(|(p, _)| dot(p, ux));
    let by = pins.map(|(p, _)| dot(p, wx));
    let [a0, c0, e0] = solve3(&a, bx)?;
    let [b0, d0, f0] = solve3(&a, by)?;
    let m = [a0, b0, 0.0, c0, d0, 0.0, e0, f0, 1.0];
    m.iter().all(|v| v.is_finite()).then_some(m)
}

fn det3(m: &[[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// True when the points `(x, y)` of three rows `[x, y, 1]` are not on one line: twice
/// their triangle's area is more than `PIN_SPREAD` times the square of its longest side.
fn spread_in_2d(a: &[[f64; 3]; 3]) -> bool {
    let side = |i: usize, j: usize| [a[j][0] - a[i][0], a[j][1] - a[i][1]];
    let (p, q, r) = (side(0, 1), side(0, 2), side(1, 2));
    let twice_area = (p[0] * q[1] - p[1] * q[0]).abs();
    let longest = [p, q, r]
        .iter()
        .map(|s| s[0] * s[0] + s[1] * s[1])
        .fold(0.0, f64::max);
    twice_area.is_finite() && longest.is_finite() && twice_area > PIN_SPREAD * longest
}

/// Texture pins closer than this to one line (the sine of their smallest angle, about) are
/// refused: independent of the face's size, unlike an absolute bound on the determinant,
/// which refused the pins of a small face.
const PIN_SPREAD: f64 = 1e-9;

/// `a x = b` by Cramer's rule; `None` when `a` is singular.
fn solve3(a: &[[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let d = det3(a);
    if !d.is_finite() || d == 0.0 {
        return None;
    }
    let mut x = [0.0; 3];
    for (col, out) in x.iter_mut().enumerate() {
        let mut m = *a;
        for r in 0..3 {
            m[r][col] = b[r];
        }
        *out = det3(&m) / d;
    }
    Some(x)
}

/// Definitions in an order where each comes after every definition it places, found by a
/// depth-first walk from each in index order. Empty ones are left out.
fn order_definitions(doc: &Doc, empty: &[bool]) -> Result<Vec<usize>, WriteError> {
    const NEW: u8 = 0;
    const OPEN: u8 = 1;
    const DONE: u8 = 2;
    let n = doc.definitions.len();
    let mut state = vec![NEW; n];
    let mut order = Vec::with_capacity(n);
    for start in 0..n {
        if state[start] != NEW || empty[start] {
            continue;
        }
        // (definition, next instance to look at)
        let mut stack = vec![(start, 0usize)];
        state[start] = OPEN;
        while let Some(top) = stack.last_mut() {
            let (d, next) = *top;
            let instances = &doc.definitions[d].geometry.instances;
            if let Some(inst) = instances.get(next) {
                top.1 += 1;
                let c = inst.definition;
                match state[c] {
                    NEW => {
                        state[c] = OPEN;
                        stack.push((c, 0));
                    }
                    OPEN => return Err(WriteError::RecursiveDefinition { definition: c }),
                    _ => {}
                }
            } else {
                state[d] = DONE;
                order.push(d);
                stack.pop();
            }
        }
    }
    Ok(order)
}

// -- deterministic GUIDs --------------------------------------------------

/// Two FNV-1a streams with different offsets: 128 bits that depend only on what was fed.
struct Hasher([u64; 2]);

impl Hasher {
    fn new() -> Self {
        Self([0xcbf2_9ce4_8422_2325, 0x6c62_272e_07bb_0142])
    }

    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            for h in &mut self.0 {
                *h ^= u64::from(x);
                *h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }

    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }

    /// The hash as a version-4-shaped GUID.
    fn guid(&self) -> [u8; 16] {
        let mut g = [0u8; 16];
        g[..8].copy_from_slice(&self.0[0].to_le_bytes());
        g[8..].copy_from_slice(&self.0[1].to_le_bytes());
        g[6] = (g[6] & 0x0f) | 0x40;
        g[8] = (g[8] & 0x3f) | 0x80;
        g
    }
}

/// A definition's GUID, from its position and content, so that two documents' different
/// definitions do not share one.
fn definition_guid(index: usize, d: &crate::doc::Definition) -> [u8; 16] {
    let mut h = Hasher::new();
    h.bytes(b"definition");
    h.u64(index as u64);
    h.bytes(d.name.as_bytes());
    h.u64(d.is_group as u64);
    let g = &d.geometry;
    for v in &g.vertices {
        v.iter().for_each(|c| h.f64(*c));
    }
    for e in &g.edges {
        h.u64(u64::from(e.a) << 32 | u64::from(e.b));
    }
    for f in &g.faces {
        h.u64(f.loops.len() as u64);
        for ring in &f.loops {
            h.u64(ring.len() as u64);
            ring.iter().for_each(|v| h.u64(u64::from(*v)));
        }
    }
    for i in &g.instances {
        h.u64(i.definition as u64);
        i.rotation.iter().flatten().for_each(|c| h.f64(*c));
        i.translation.iter().for_each(|c| h.f64(*c));
    }
    h.u64(g.guides.len() as u64);
    h.guid()
}

fn instance_guid(owner: Owner, index: usize, definition: [u8; 16]) -> [u8; 16] {
    let mut h = Hasher::new();
    h.bytes(b"instance");
    h.u64(match owner {
        Owner::Root => u64::MAX,
        Owner::Definition(d) => d as u64,
    });
    h.u64(index as u64);
    h.bytes(&definition);
    h.guid()
}

// -- writing --------------------------------------------------------------

struct Ctx<'a> {
    doc: &'a Doc,
    plan: &'a Plan,
    material_slots: &'a [u32],
}

impl Ctx<'_> {
    fn material(&self, m: Option<usize>) -> u16 {
        m.and_then(|i| self.material_slots.get(i))
            .map_or(0, |s| *s as u16)
    }

    /// One entity list: each face after the edges it is the first to use (vertices inline
    /// on first use), then the loose edges, the instances and the guides. Returns how many
    /// entities it wrote.
    fn entities(
        &self,
        ar: &mut Archive,
        owner: Owner,
        g: &Geometry,
        def_slots: &[Option<u32>],
    ) -> u32 {
        let plan = self.plan.geometry(owner);
        let mut vslot: Vec<Option<u32>> = vec![None; g.vertices.len()];
        let mut eslot: Vec<Option<u32>> = vec![None; g.edges.len()];
        let mut count = 0u32;

        for (f, fp) in g.faces.iter().zip(&plan.faces) {
            let mut loops: Vec<Vec<Use>> = Vec::with_capacity(fp.loops.len());
            for uses in &fp.loops {
                let mut out = Vec::with_capacity(uses.len());
                for &(e, reversed) in uses {
                    let slot = match eslot[e] {
                        Some(s) => s,
                        None => {
                            count += 1;
                            write_edge(ar, g, &g.edges[e], &mut vslot)
                        }
                    };
                    eslot[e] = Some(slot);
                    out.push((slot, reversed));
                }
                loops.push(out);
            }
            ar.face(
                self.material(f.front),
                self.material(f.back),
                fp.plane,
                &loops,
                fp.uv,
            );
            count += 1;
        }

        for (e, edge) in g.edges.iter().enumerate() {
            if eslot[e].is_none() {
                eslot[e] = Some(write_edge(ar, g, edge, &mut vslot));
                count += 1;
            }
        }

        for (i, inst) in g.instances.iter().enumerate() {
            let def = &self.doc.definitions[inst.definition];
            let Some(slot) = def_slots[inst.definition] else {
                continue;
            };
            let guid = instance_guid(owner, i, self.plan.def_guids[inst.definition]);
            // A group's name is its instance's; a component instance keeps the empty
            // name SketchUp gives a placement nobody renamed.
            let name = if def.is_group { def.name.as_str() } else { "" };
            ar.instance(
                def.is_group,
                slot,
                self.material(inst.material),
                &inst.rotation,
                &inst.translation,
                name,
                guid,
            );
            count += 1;
        }

        let mut directions = plan.directions.iter();
        for guide in &g.guides {
            match guide {
                Guide::Line { point, .. } => {
                    let d = directions.next().copied().unwrap_or([1.0, 0.0, 0.0]);
                    ar.construction_line(*point, d);
                }
                Guide::Point { position } => ar.construction_point(*position),
            }
            count += 1;
        }
        count
    }
}

/// A top-level `CEdge`, its vertices inline the first time they are used.
fn write_edge(ar: &mut Archive, g: &Geometry, e: &Edge, vslot: &mut [Option<u32>]) -> u32 {
    let slot = ar.edge_start(DrawFlags {
        hidden: e.hidden,
        soft: e.soft,
        smooth: e.smooth,
    });
    for v in [e.a as usize, e.b as usize] {
        match vslot[v] {
            Some(s) => ar.back_ref(s),
            None => vslot[v] = Some(ar.vertex(g.vertices[v])),
        }
    }
    ar.edge_end();
    slot
}
