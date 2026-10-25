//! The MFC `CArchive` object protocol, written: slot numbering, class declarations and
//! references, back-references, persistent ids, and one method per record this writer
//! emits. Ported from openskp's `_ArchiveWriter` (`create.py`); see `PORTED.md`.
//!
//! Every new class declaration and every new object takes the next slot. A tag is a `u16`:
//! `0xffff` declares a class, `0x8000 | slot` makes a new object of the class in `slot`, a
//! plain value refers back to an object, and `0x7fff` escapes to a `u32` for slots at or
//! past 0x7fff (with bit 31 set for a class reference).

use std::collections::BTreeMap;

/// `CAttributeContainer`, declared in the scaffold's prefix.
pub(crate) const ATTR_CONTAINER_SLOT: u32 = 3;
/// `CCamera`, declared in the scaffold's prefix.
pub(crate) const CAMERA_SLOT: u32 = 7;

const MATERIAL_SCHEMA: u16 = 12;
const DIB_SCHEMA: u16 = 3;
const LAYER_SCHEMA: u16 = 3;
const DEFINITION_SCHEMA: u16 = 11;
const INSTANCE_SCHEMA: u16 = 6;
const GROUP_SCHEMA: u16 = 1;
const THUMBNAIL_SCHEMA: u16 = 1;
const FTC_SCHEMA: u16 = 4;
const VERTEX_SCHEMA: u16 = 0;
const EDGE_SCHEMA: u16 = 2;
const FACE_SCHEMA: u16 = 3;
const LOOP_SCHEMA: u16 = 1;
const EDGE_USE_SCHEMA: u16 = 1;
const CONSTRUCTION_LINE_SCHEMA: u16 = 1;
const CONSTRUCTION_POINT_SCHEMA: u16 = 0;

/// What a guide line's start and end parameters hold when it is unbounded that way.
const CLINE_INFINITE: f64 = 1e30;

/// A definition's 22-byte base block: zero but for offsets 3 and 4, like the drawbase.
const DEFINITION_BASE_BLOCK: [u8; 22] = [
    0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// The 176 bytes after `CCamera`'s class reference that SketchUp writes for a definition's
/// default thumbnail camera, copied as openskp copies them.
const CAMERA_TEMPLATE: [u8; 176] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,
    0, 0, 62, 64, 0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 255, 254, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// The side of a face's texture mapping that is not positioned.
const IDENTITY_UV: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// The longest string record: a `u8` count of UTF-16 units, 0xff excluded.
pub(crate) const MAX_STRING_UNITS: usize = 254;

/// CDib's image format tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageKind {
    Png,
    Jpeg,
}

impl ImageKind {
    pub(crate) fn of(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Some(Self::Jpeg)
        } else {
            None
        }
    }

    fn subtype(self) -> u32 {
        match self {
            Self::Png => 4,
            Self::Jpeg => 1,
        }
    }

    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Png => ".png",
            Self::Jpeg => ".jpg",
        }
    }
}

/// A textured material's image and tile size.
pub(crate) struct TextureRecord<'a> {
    pub kind: ImageKind,
    pub bytes: &'a [u8],
    pub width: f64,
    pub height: f64,
    pub path: &'a str,
}

/// Edge-record flags.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DrawFlags {
    pub hidden: bool,
    pub soft: bool,
    pub smooth: bool,
}

/// A face's front and back texture matrices; `None` for a side not positioned.
pub(crate) type UvSides = (Option<[f64; 9]>, Option<[f64; 9]>);

/// One edge use: the edge's slot and whether the loop runs against it.
pub(crate) type Use = (u32, bool);

pub(crate) struct Archive {
    pub next_slot: u32,
    pub next_pid: u64,
    pub class_slot: BTreeMap<&'static str, u32>,
    pub buf: Vec<u8>,
}

impl Archive {
    pub(crate) fn new(next_slot: u32, next_pid: u64) -> Self {
        Self {
            next_slot,
            next_pid,
            class_slot: BTreeMap::new(),
            buf: Vec::new(),
        }
    }

    // -- primitives -------------------------------------------------------

    pub(crate) fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub(crate) fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn f64s(&mut self, vs: &[f64]) {
        for v in vs {
            self.f64(*v);
        }
    }

    pub(crate) fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    /// `ff fe ff`, a `u8` count of UTF-16 units, the units. Callers check the length
    /// first; anything longer is cut at a character boundary rather than written wrong.
    pub(crate) fn string(&mut self, s: &str) {
        let mut units: Vec<u16> = Vec::new();
        for c in s.chars() {
            let mut b = [0u16; 2];
            let enc = c.encode_utf16(&mut b);
            if units.len() + enc.len() > MAX_STRING_UNITS {
                break;
            }
            units.extend_from_slice(enc);
        }
        self.bytes(&[0xff, 0xfe, 0xff]);
        self.u8(units.len() as u8);
        for u in units {
            self.u16(u);
        }
    }

    // -- slots ------------------------------------------------------------

    fn alloc(&mut self) -> u32 {
        let s = self.next_slot;
        self.next_slot += 1;
        s
    }

    fn alloc_pid(&mut self) -> u64 {
        let p = self.next_pid;
        self.next_pid += 1;
        p
    }

    /// Start a new object of `class`: declare the class on first use, refer to it after.
    /// Returns the object's slot.
    pub(crate) fn new_of_class(&mut self, class: &'static str, schema: u16) -> u32 {
        match self.class_slot.get(class) {
            None => {
                self.u16(0xffff);
                self.u16(schema);
                self.u16(class.len() as u16);
                self.bytes(class.as_bytes());
                let c = self.alloc();
                self.class_slot.insert(class, c);
            }
            Some(&c) => self.class_ref(c),
        }
        self.alloc()
    }

    /// A reference to a class already declared, by its slot. 0x7fff itself takes the
    /// escape: `0x8000 | 0x7fff` is `0xffff`, which reads as a declaration.
    fn class_ref(&mut self, slot: u32) {
        if slot < 0x7fff {
            self.u16(0x8000 | slot as u16);
        } else {
            self.u16(0x7fff);
            self.u32(0x8000_0000 | slot);
        }
    }

    /// A new object of a class declared in the scaffold's prefix.
    fn new_of_prefix_class(&mut self, slot: u32) -> u32 {
        self.class_ref(slot);
        self.alloc()
    }

    pub(crate) fn null(&mut self) {
        self.u16(0);
    }

    /// A reference back to an object already written. A bare 0x7fff is the escape marker,
    /// so that slot takes the escape too.
    pub(crate) fn back_ref(&mut self, slot: u32) {
        if slot < 0x7fff {
            self.u16(slot as u16);
        } else {
            self.u16(0x7fff);
            self.u32(slot);
        }
    }

    /// A persistent id: a mask of its non-zero bytes, then those bytes.
    fn pid(&mut self, pid: u64) {
        let mut mask = 0u8;
        let mut out = [0u8; 8];
        let mut n = 0;
        for (bit, byte) in pid.to_le_bytes().into_iter().enumerate() {
            if byte != 0 {
                mask |= 1 << bit;
                out[n] = byte;
                n += 1;
            }
        }
        self.u8(mask);
        self.bytes(&out[..n]);
    }

    // -- shared blocks ----------------------------------------------------

    /// No attribute container, then a fresh persistent id.
    fn preamble(&mut self) {
        self.null();
        let p = self.alloc_pid();
        self.pid(p);
    }

    /// No attribute container and persistent id 0: the structural records.
    fn preamble_no_pid(&mut self) {
        self.null();
        self.pid(0);
    }

    /// A real attribute container holding `ftc` (a face's texture mapping) or nothing,
    /// then a fresh persistent id.
    fn preamble_with_container(&mut self, ftc: Option<UvSides>) {
        self.new_of_prefix_class(ATTR_CONTAINER_SLOT);
        // The container's own preamble: no attributes, persistent id 0.
        self.bytes(&[0, 0, 0]);
        if let Some((front, back)) = ftc {
            self.face_texture_coords(front, back);
        }
        self.null();
        let p = self.alloc_pid();
        self.pid(p);
    }

    /// `CDrawingElement`: material, hidden, two bytes SketchUp drops the entity without
    /// (1, 1), soft, smooth, a pad byte and the layer: 0 for Layer0, else a reference to
    /// the layer's slot.
    fn drawbase(&mut self, material: u16, f: DrawFlags, layer: u32) {
        self.u16(material);
        self.bytes(&[f.hidden as u8, 1, 1, f.soft as u8, f.smooth as u8, 0]);
        self.back_ref(layer);
    }

    // -- records ----------------------------------------------------------

    pub(crate) fn vertex(&mut self, p: [f64; 3]) -> u32 {
        let slot = self.new_of_class("CVertex", VERTEX_SCHEMA);
        self.preamble();
        self.f64s(&p);
        slot
    }

    /// A `CEdge` up to its vertices: the caller writes both (inline or as references),
    /// then calls `edge_end`.
    pub(crate) fn edge_start(&mut self, flags: DrawFlags, layer: u32) -> u32 {
        let slot = self.new_of_class("CEdge", EDGE_SCHEMA);
        self.preamble();
        self.drawbase(0, flags, layer);
        slot
    }

    /// No curve.
    pub(crate) fn edge_end(&mut self) {
        self.null();
    }

    /// A `CFace`: the plane, the loops as edge uses (outer first), the back material.
    pub(crate) fn face(
        &mut self,
        (material, back_material): (u16, u16),
        layer: u32,
        hidden: bool,
        plane: [f64; 4],
        loops: &[Vec<Use>],
        uv: Option<UvSides>,
    ) {
        self.new_of_class("CFace", FACE_SCHEMA);
        match uv {
            Some(ftc) => self.preamble_with_container(Some(ftc)),
            None => self.preamble(),
        }
        let flags = DrawFlags {
            hidden,
            ..DrawFlags::default()
        };
        self.drawbase(material, flags, layer);
        self.f64s(&plane);
        // Validation refuses a count past u32 (`Plan::new`).
        self.u32(u32::try_from(loops.len()).unwrap_or(u32::MAX));
        for (i, uses) in loops.iter().enumerate() {
            let loop_slot = self.new_of_class("CLoop", LOOP_SCHEMA);
            self.preamble_no_pid();
            // 1 marks the outer loop, 0 a hole; the second byte is always 1.
            self.bytes(&[(i == 0) as u8, 1]);
            for &(edge, reversed) in uses {
                self.new_of_class("CEdgeUse", EDGE_USE_SCHEMA);
                self.preamble_no_pid();
                self.back_ref(edge);
                self.u8(reversed as u8);
                self.back_ref(loop_slot);
            }
            self.null();
        }
        self.u16(back_material);
    }

    /// `CFaceTextureCoords`: both sides' texture-to-plane matrices, no pins, the
    /// positioned flags.
    fn face_texture_coords(&mut self, front: Option<[f64; 9]>, back: Option<[f64; 9]>) {
        self.new_of_class("CFaceTextureCoords", FTC_SCHEMA);
        self.preamble_no_pid();
        self.u32(0);
        let mut ks = [0.0f64; 24];
        ks[0..9].copy_from_slice(&front.unwrap_or(IDENTITY_UV));
        ks[12..21].copy_from_slice(&back.unwrap_or(IDENTITY_UV));
        self.f64s(&ks);
        self.u32(0);
        self.u32(0);
        self.u32(front.is_some() as u32);
        self.u32(back.is_some() as u32);
    }

    /// A solid or textured `CMaterial`. `opacity` is `None` for an opaque material.
    pub(crate) fn material(
        &mut self,
        name: &str,
        rgba: [u8; 4],
        opacity: Option<f64>,
        texture: Option<TextureRecord<'_>>,
    ) -> u32 {
        let slot = self.new_of_class("CMaterial", MATERIAL_SCHEMA);
        self.preamble();
        self.string(name);
        match texture {
            None => {
                self.u16(0);
                self.bytes(&rgba);
                self.string("");
                self.bytes(&[0; 8]);
            }
            Some(t) => {
                self.u16(1);
                self.bytes(&[0, 0]);
                self.new_of_class("CDib", DIB_SCHEMA);
                self.u32(t.kind.subtype());
                // Validation refuses a length past u32 (`Plan::new`).
                self.u32(u32::try_from(t.bytes.len()).unwrap_or(u32::MAX));
                self.bytes(t.bytes);
                if t.kind == ImageKind::Jpeg {
                    self.u32(90);
                }
                self.f64(t.width);
                self.f64(t.height);
                self.string(t.path);
                // The average colour, twice. Alpha 254: 255 here reads as colorized.
                let [r, g, b, _] = rgba;
                self.bytes(&[r, g, b, 254, 0, r, g, b, 254]);
                self.string("");
                self.u32(1);
                self.u32(0);
            }
        }
        // The stored value is a transparency, gated by the byte after it.
        match opacity {
            None => {
                self.f64(1.0);
                self.u8(0);
            }
            Some(o) => {
                self.f64(1.0 - o);
                self.u8(1);
            }
        }
        slot
    }

    /// A definition's own copy of Layer0: no persistent ids.
    fn embedded_layer0(&mut self) -> u32 {
        self.layer_record("Layer0", [0; 4], false, false)
    }

    /// A model layer (a tag): its name, colour and hidden flag, with persistent ids.
    pub(crate) fn layer(&mut self, name: &str, rgb: [u8; 3], hidden: bool) -> u32 {
        let [r, g, b] = rgb;
        self.layer_record(name, [r, g, b, 255], hidden, true)
    }

    /// A `CLayer`: name, hidden flag, a second persistent id (two zero bytes before it),
    /// its colour material's name and colour, an empty string and the 21-byte tail.
    /// `with_pids` false writes both persistent ids as 0, as a definition's own copy of
    /// Layer0 has them.
    fn layer_record(&mut self, name: &str, rgba: [u8; 4], hidden: bool, with_pids: bool) -> u32 {
        let slot = self.new_of_class("CLayer", LAYER_SCHEMA);
        if with_pids {
            self.preamble();
        } else {
            self.preamble_no_pid();
        }
        self.string(name);
        self.bytes(&[hidden as u8, 0, 0]);
        let second = if with_pids { self.alloc_pid() } else { 0 };
        self.pid(second);
        self.string(&format!("Layer_{name}"));
        self.u16(256);
        self.bytes(&rgba);
        self.string("");
        self.bytes(&[0; 8]);
        self.f64(0.5);
        self.bytes(&[0; 5]);
        slot
    }

    /// A `CComponentDefinition` up to its entity list. Returns the definition's slot and
    /// where its entity count goes.
    pub(crate) fn definition_start(&mut self) -> (u32, usize) {
        let slot = self.new_of_class("CComponentDefinition", DEFINITION_SCHEMA);
        self.preamble_with_container(None);
        self.bytes(&DEFINITION_BASE_BLOCK);
        self.u32(1);
        let layer = self.embedded_layer0();
        self.back_ref(layer);
        self.u32(0);
        let count_at = self.buf.len();
        self.u32(0);
        (slot, count_at)
    }

    pub(crate) fn patch_u32(&mut self, at: usize, v: u32) {
        if let Some(b) = self.buf.get_mut(at..at + 4) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }

    /// Close a definition: no relationships, its GUID and name, the timestamp, the
    /// behaviour block and a thumbnail with no image. The block's last `u32` is 1 for a
    /// group's definition and 0 for a component's: it is what SketchUp's `group?` reads.
    pub(crate) fn definition_end(
        &mut self,
        guid: [u8; 16],
        name: &str,
        timestamp: u32,
        group: bool,
    ) {
        self.u32(0);
        self.u16(0);
        self.bytes(&guid);
        self.string(name);
        self.string("");
        self.string("");
        self.u32(timestamp);
        self.bytes(&[0; 39]);
        self.u32(group as u32);
        self.new_of_class("CThumbnail", THUMBNAIL_SCHEMA);
        self.preamble_no_pid();
        self.new_of_prefix_class(CAMERA_SLOT);
        self.bytes(&CAMERA_TEMPLATE);
        self.null();
    }

    /// A `CComponentInstance` or a `CGroup`: the 3x3 row-major, the translation, the 1.0
    /// divisor, the name, the GUID. An instance carries an empty attribute container, a
    /// group none.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn instance(
        &mut self,
        group: bool,
        definition: u32,
        material: u16,
        layer: u32,
        hidden: bool,
        rotation: &[[f64; 3]; 3],
        translation: &[f64; 3],
        name: &str,
        guid: [u8; 16],
    ) {
        if group {
            self.new_of_class("CGroup", GROUP_SCHEMA);
            self.preamble();
        } else {
            self.new_of_class("CComponentInstance", INSTANCE_SCHEMA);
            self.preamble_with_container(None);
        }
        let flags = DrawFlags {
            hidden,
            ..DrawFlags::default()
        };
        self.drawbase(material, flags, layer);
        self.back_ref(definition);
        for row in rotation {
            self.f64s(row);
        }
        self.f64s(translation);
        self.f64(1.0);
        self.string(name);
        self.bytes(&guid);
    }

    /// An infinite `CConstructionLine` through `point` along the unit `direction`.
    pub(crate) fn construction_line(
        &mut self,
        point: [f64; 3],
        direction: [f64; 3],
        layer: u32,
        hidden: bool,
    ) {
        self.new_of_class("CConstructionLine", CONSTRUCTION_LINE_SCHEMA);
        self.preamble();
        let flags = DrawFlags {
            hidden,
            ..DrawFlags::default()
        };
        self.drawbase(0, flags, layer);
        self.f64s(&point);
        self.f64s(&direction);
        self.f64(-CLINE_INFINITE);
        self.f64(CLINE_INFINITE);
        self.bytes(&[0; 4]);
    }

    pub(crate) fn construction_point(&mut self, position: [f64; 3], layer: u32, hidden: bool) {
        self.new_of_class("CConstructionPoint", CONSTRUCTION_POINT_SCHEMA);
        self.preamble();
        let flags = DrawFlags {
            hidden,
            ..DrawFlags::default()
        };
        self.drawbase(0, flags, layer);
        self.f64s(&position);
        self.f64s(&[0.0; 3]);
        self.u8(0);
    }
}
