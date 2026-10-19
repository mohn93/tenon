//! Materials, which live in two places at once.
//!
//! A VFF `.skp` splits every material across `model.dat` and the zip, and a
//! reader that looks in only one of them gets half an answer:
//!
//! ```text
//! model.dat                                     the zip
//!   SketchUpModelMaterials                        materials/<folder>/material.xml
//!     MaterialManager                               name, colour, opacity,
//!       MaterialManagerMaterials                    colourised flag
//!         Material*                                 <mat:texture> filename
//!           Entity > EntityPID    the id            and tile size
//!           MaterialXMLPath       the folder       materials/<folder>/<image>
//!           MaterialHidden                          the pixels - never read here
//!           MaterialTextureJpgQuality
//! ```
//!
//! So `model.dat` has the **ids** and nothing else a renderer wants, and the
//! xml has everything a renderer wants and no id. `MaterialXMLPath` is the
//! join key: it is the zip folder name, byte for byte, in all 281 materials
//! of the commercial school pack and in every file measured.
//!
//! Two counts follow from this and they are the ones to check a port against:
//!
//! - **The section does not hold every material.** Each tag owns a
//!   `Layer_<tag name>` material that is stored *inside* the layer record, not
//!   in the material manager. The school pack is 281 + 4 = 285; the commercial
//!   classroom 148 + 14 = 162; the small powerbank 13 + 1 = 14. Those are
//!   exactly the figures openskp reports, because openskp counts
//!   `material.xml` entries. Checked over 553 VFF files on this machine:
//!   `{section folders} ∪ {layer folders}` equals `{material.xml folders}`
//!   with zero exceptions either way, so [`parse_materials`] plus
//!   [`parse_layer_materials`] is a complete enumeration and the merge never
//!   has to invent an entry.
//!
//!   That is the enumeration. It is **not** the material list: SketchUp's own
//!   `model.materials` is 281, not 285, because a `Layer_<tag>` carrier is a
//!   tag's colour and not a material anybody paints with. `scene.rs`'s
//!   `index()` drops them again once `layers.rs` has taken the colours, which
//!   is why this module still has to produce them. See [`is_layer_material`].
//! - **No material in `model.dat` carries a colour.** Not one tag for it
//!   exists in the record; scanned across the corpus a `Material` has only
//!   `Entity`, `MaterialXMLPath`, `MaterialHidden`, `MaterialLoadedFrom`,
//!   `MaterialTextureJpgQuality` and sometimes `MaterialThumbnailImage`. So
//!   the colour in [`parse_materials`]'s output is a placeholder until
//!   [`merge_material_xml`] has run.
//!
//! ## Texture bytes are never read
//!
//! [`TextureRef`] records a zip entry index, not pixels. On the 517 MB
//! restaurant pack the images are 380 MB of the file and line art needs none
//! of them; on a 46 MB VFF file they are 10.4 MB. Both existing ports load
//! them eagerly - the C++ one calls `zip.get(candidate)` while parsing the
//! xml - which is most of why they cannot open a large pack.

use crate::model::{MaterialDef, TextureLocation, TextureRef, INCH_TO_MM};
use crate::tags as t;
use crate::tlv::{Record, Tlv};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// What `model.dat` knows about a material before the xml is merged in.
///
/// `name` is the zip folder, which is a sanitised form of the real name
/// (`*9` is stored in `materials/_9/`), so it is a join key rather than
/// something to show a user. `rgba` is SketchUp's own fallback grey, the same
/// 128/128/128 the C++ port falls back to when an attribute is missing, so a
/// caller that skips the merge still paints something rather than nothing.
pub fn parse_materials(payload: &[u8]) -> Vec<MaterialDef> {
    let mut out = Vec::new();
    let manager = unwrap_to(payload, t::MATERIAL_MANAGER);
    for list in Tlv::new(manager).filter(|r| r.tag == t::MATERIAL_MANAGER_MATERIALS) {
        for m in list.children().filter(|r| r.tag == t::MATERIAL) {
            if let Some(def) = material_record(&m) {
                out.push(def);
            }
        }
    }
    out
}

/// The `Layer_<name>` materials, which the layers section owns.
///
/// Pass the `SketchUpModelLayers` payload. These are real materials with real
/// ids - they are how a tag's colour is stored - and leaving them out makes
/// every material count disagree with every other reader.
pub fn parse_layer_materials(layers_payload: &[u8]) -> Vec<MaterialDef> {
    let mut out = Vec::new();
    let manager = unwrap_to(layers_payload, t::LAYER_MANAGER);
    for list in Tlv::new(manager).filter(|r| r.tag == t::LAYER_MANAGER_LAYERS) {
        for layer in list.children().filter(|r| r.tag == t::LAYER) {
            for holder in layer.children().filter(|r| r.tag == t::LAYER_MATERIAL) {
                for m in holder.children().filter(|r| r.tag == t::MATERIAL) {
                    if let Some(def) = material_record(&m) {
                        out.push(def);
                    }
                }
            }
        }
    }
    out
}

fn material_record(m: &Record<'_>) -> Option<MaterialDef> {
    // Without an id nothing can refer to this material, so it is the one field
    // whose absence drops the entry. Everything else degrades to a default and
    // the remaining materials survive.
    let id = entity_pid(m)?;
    Some(MaterialDef {
        id,
        name: m
            .child(t::MATERIAL_XML_PATH)
            .map(|r| utf8_body(r.payload))
            .unwrap_or_default(),
        rgba: [128, 128, 128, 255],
        opacity: 1.0,
        texture: None,
        colorized: false,
    })
}

/// Fold `material.xml` into the table parsed from `model.dat`.
///
/// * `xmls` - `(folder, bytes)` for each `materials/<folder>/material.xml`.
///   The folder is the middle path component, which is the join key.
/// * `images` - `(zip entry name, zip entry index)` for every image entry in
///   the archive. Only names and indices: the bytes stay in the file.
///
/// A material whose folder has no xml keeps its placeholder values, and an xml
/// no material claims is ignored - that combination does not occur in any file
/// measured, but a malformed one must not drop the materials that are fine.
pub fn merge_material_xml(
    materials: &mut [MaterialDef],
    xmls: &[(&str, &[u8])],
    images: &[(&str, u32)],
) {
    // The school pack is 285 materials against 285 xml documents; a linear
    // scan per material would be 81k comparisons of owned strings. One map.
    let mut by_folder: BTreeMap<&str, &[u8]> = BTreeMap::new();
    for (folder, bytes) in xmls {
        by_folder.insert(folder, bytes);
    }

    for m in materials.iter_mut() {
        let Some(xml) = by_folder.get(m.name.as_str()).copied() else {
            continue;
        };
        apply_xml(m, xml, images);
    }
}

fn apply_xml(m: &mut MaterialDef, xml: &[u8], images: &[(&str, u32)]) {
    let folder = m.name.clone();
    let Some(attrs) = element(xml, b"material") else {
        return;
    };

    if let Some(v) = attr(attrs, b"name") {
        let name = display_name(&decode_entities(v));
        if !name.is_empty() {
            m.name = name;
        }
    }
    m.rgba = [
        attr(attrs, b"colorRed").and_then(parse_u8).unwrap_or(128),
        attr(attrs, b"colorGreen").and_then(parse_u8).unwrap_or(128),
        attr(attrs, b"colorBlue").and_then(parse_u8).unwrap_or(128),
        255,
    ];

    // `trans` is a TRANSPARENCY and only counts when `useTrans="1"`; with the
    // flag off the attribute is a leftover that takes the values 0, 0.5 and 1
    // indifferently across the corpus, so reading it unconditionally would
    // make a third of all materials half-invisible.
    //
    // That `trans` is transparency rather than alpha is worth the evidence,
    // because this project's *writer* assumes the opposite (openskp
    // `vff_write.material_xml` emits `trans = alpha/255`) and one of the two
    // is wrong. The reading here is settled by what the values mean on real
    // models: in the legacy Eurovision stage `<Blue Glass>` stores 0.85 and
    // `<Red Glass>` 0.88, a BMW's `light-1` lens stores 0.72, and
    // `[Water Light Blue]` stores 0.65. Read as transparency those are glass
    // at 15% opacity, a lens at 28% and water at 35%, which is what those
    // materials are; read as alpha they would be 85-88% opaque sheets of
    // "glass". SketchUp's own shipped `Glass_Basic_01.skm` stores 0.75, and
    // stock SketchUp glass is clearly see-through, not three-quarters solid.
    // Three of openskp's five ports agree; its writer does not, and that is a
    // bug on the writer's side rather than here.
    let use_trans = attr(attrs, b"useTrans").map(|v| v == b"1").unwrap_or(false);
    m.opacity = if use_trans {
        let trans = attr(attrs, b"trans").and_then(parse_f64).unwrap_or(0.0);
        clamp01(1.0 - trans) as f32
    } else {
        1.0
    };

    // type="2" is a colourised copy - the "[Name]1" material SketchUp makes
    // when a textured material is re-tinted. colorizeType 0 is a hue shift and
    // 1 a tint; the contract carries only the flag, so the type is not kept.
    m.colorized = attr(attrs, b"type").map(|v| v == b"2").unwrap_or(false);

    let Some(tex) = element(xml, b"texture") else {
        return;
    };
    // `textureFilename` is whatever path the authoring machine had, and on
    // commercial files that is an absolute Windows path - the school pack is
    // full of `C:/Users/jo/AppData/Local/Temp/ChaosGroupTextureCache/...`.
    // Only the last component can possibly match a zip entry.
    let declared = attr(tex, b"textureFilename")
        .map(decode_entities)
        .unwrap_or_default();
    let base = basename(&declared);

    // Tile size. The xml stores INCHES and the contract is millimetres.
    //
    // Measured, not assumed: the Korean school pack was modelled in
    // millimetres, and its textures come out as round numbers only after
    // multiplying by 25.4 - `xScale="23.622047244094489"` is 600.0 mm exactly,
    // `118.11023622047244` is 3000.0 mm, and `82.677165354330711` is 2100.0 mm.
    // Left in inches a 600 mm tile would repeat 24 times where it should
    // repeat once.
    //
    // The numbers are not always sane. The commercial classroom contains
    // `xScale="3543307" yScale="35"` on `Wood_Grain_Stain` - a lost decimal
    // point, 3.543307 in (90 mm) written as 3543307 in (90 km). It is passed
    // through rather than guessed at: inventing a value would hide a defect
    // the audit should see, and a renderer can clamp what it cannot use.
    let tile_mm = [
        attr(tex, b"xScale").and_then(parse_f64).unwrap_or(0.0) * INCH_TO_MM,
        attr(tex, b"yScale").and_then(parse_f64).unwrap_or(0.0) * INCH_TO_MM,
    ];

    let (name, index) = match resolve_image(xml, &folder, &base, images) {
        Some(v) => v,
        // `hasTexture="1"` with no image in the archive happens; the material
        // keeps its flat colour rather than a dangling reference.
        None => return,
    };
    m.texture = Some(TextureRef {
        name,
        location: TextureLocation::ZipEntry(index),
        tile_mm,
    });
}

/// Find the zip entry holding a material's image, in the order that works.
///
/// 1. `materials/<folder>/<basename of textureFilename>` - the normal case.
/// 2. The folder's only non-xml sibling. The stored name and the declared one
///    do differ in the wild: openskp recorded `..._Safety.jpg` in the xml
///    against `..._Saftey.jpg` on disk.
/// 3. `<mat:image path>`, which is how a colourised copy that kept no image of
///    its own points at the source material's folder - in the commercial
///    classroom `materials/[Aluminum]1/` has no image and its xml says
///    `path="materials/[Aluminum]/Aluminum.jpg"`. Tried both as a full archive
///    path and as one relative to this folder, with any `./` stripped.
///
/// The name returned is the **resolved entry's** basename, not the declared
/// one, so it always names the bytes the index points at. That differs from
/// openskp, which reports the declared string: on the school pack three
/// materials declare `Metal_Corrogated_Shiny.jpg` or a cache hash that is not
/// in the archive while the folder holds a differently named file, and openskp
/// reports a filename its own `data` did not come from.
fn resolve_image(
    xml: &[u8],
    folder: &str,
    base: &str,
    images: &[(&str, u32)],
) -> Option<(String, u32)> {
    let prefix = {
        let mut p = String::with_capacity(10 + folder.len());
        p.push_str("materials/");
        p.push_str(folder);
        p.push('/');
        p
    };

    if !base.is_empty() {
        let mut want = prefix.clone();
        want.push_str(base);
        if let Some((n, i)) = images.iter().find(|(n, _)| *n == want.as_str()) {
            return Some((String::from(basename(n)), *i));
        }
    }

    if let Some((n, i)) = images.iter().find(|(n, _)| n.starts_with(prefix.as_str())) {
        return Some((String::from(basename(n)), *i));
    }

    if let Some(p) = element(xml, b"image").and_then(|a| attr(a, b"path")) {
        let p = decode_entities(p);
        let rel = p.trim_start_matches(['.', '/']);
        if let Some((n, i)) = images.iter().find(|(n, _)| *n == rel) {
            return Some((String::from(basename(n)), *i));
        }
        let mut want = prefix;
        want.push_str(rel);
        if let Some((n, i)) = images.iter().find(|(n, _)| *n == want.as_str()) {
            return Some((String::from(basename(n)), *i));
        }
    }

    None
}

/// What SketchUp shows for a material whose name the file abbreviates.
///
/// A material nobody renamed is stored as `*`, `*1`, `*60`; SketchUp's own
/// material browser shows those as `Material`, `Material1`, `Material60`. The
/// mapping was checked exhaustively against SketchUp's reported material list
/// on four commercial packs - all 177 starred names in the food pack and all
/// 154 in the classroom map this way, with no exceptions and no collisions.
///
/// Only a star followed by digits, or a bare star. `*Brick` is a name
/// somebody typed and is left alone; so is anything with a star in the middle.
/// Getting that boundary wrong would rename real materials, and the artist
/// reads this string.
pub fn display_name(raw: &str) -> String {
    let Some(rest) = raw.strip_prefix('*') else {
        return String::from(raw);
    };
    if !rest.bytes().all(|b| b.is_ascii_digit()) {
        return String::from(raw);
    }
    let mut s = String::with_capacity(8 + rest.len());
    s.push_str("Material");
    s.push_str(rest);
    s
}

/// Whether a material is a tag's colour carrier rather than a material.
///
/// SketchUp stores a tag's colour as a material named `Layer_<tag name>`
/// inside the layers section. `layers.rs` needs them - that is where a tag's
/// colour comes from - but SketchUp does not list them in `model.materials`
/// and nor should the artist's material list. The classroom pack reports 285
/// against SketchUp's 281, and the four are exactly `Layer_Layer0`,
/// `Layer_천장`, `Layer_교실` and `Layer_학교`.
///
/// Nothing is painted with one: across the classroom, the cafeteria and the
/// VFF classroom fixture, zero mesh batches and zero placements reference a
/// `Layer_` material id, so dropping them from the index costs no colour.
pub fn is_layer_material(name: &str) -> bool {
    name.starts_with("Layer_")
}

/// Whether a zip entry name is a material's image file.
///
/// Anything under `materials/<folder>/` that is not xml, rather than a list of
/// extensions. An allowlist of `jpg`/`png` looked safe and was wrong: across
/// the corpus the images are 4,295 `.jpg`, 6 `.png`, 1 `.jpeg` **and 3
/// `.tif`** - a Russian-authored fan model embeds `Plastic.tif`,
/// `metal_fine.tif` and `set_round.tif`, and an allowlist silently dropped all
/// three of its textures while openskp found them. SketchUp will accept
/// whatever the authoring machine had, so the rule is "not the metadata".
///
/// The thumbnails under `meta/`, `thumbnails/` and `scene_thumbnails/` are
/// excluded by the prefix, because a material must never resolve to the
/// model's own preview image.
pub fn is_texture_entry(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("materials/") else {
        return false;
    };
    // Exactly `<folder>/<file>`: a deeper path is not something SketchUp
    // writes and is not a sibling of any `material.xml`.
    let Some((folder, file)) = rest.split_once('/') else {
        return false;
    };
    // Compared as bytes, not by slicing the `str`. `file[file.len() - 4..]`
    // panics when the name ends in a multi-byte character - a texture called
    // `창문.jpg` has its last four bytes straddling a character boundary - and
    // these names come out of commercial Korean packs.
    let b = file.as_bytes();
    !folder.is_empty()
        && !b.is_empty()
        && !file.contains('/')
        && !(b.len() >= 4 && b[b.len() - 4..].eq_ignore_ascii_case(b".xml"))
}

/// The folder component of `materials/<folder>/material.xml`, if that is what
/// `name` is. The join key for [`merge_material_xml`].
pub fn material_xml_folder(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("materials/")?;
    let (folder, file) = rest.split_once('/')?;
    if file.eq_ignore_ascii_case("material.xml") && !folder.is_empty() {
        Some(folder)
    } else {
        None
    }
}

// ── xml by byte scan ─────────────────────────────────────────────────────
//
// These documents are small - a few kilobytes - but there are up to 285 of
// them per file and some carry an embedded V-Ray CDATA blob tens of kilobytes
// long, so the scan takes the first match of each element and stops. An xml
// dependency would be a third-party parser in a wasm bundle for four
// attributes and a filename, and the format is SketchUp's own serialiser
// writing one fixed shape, not arbitrary xml.

/// The attribute span of the first `<name ...>` or `<ns:name ...>` element.
fn element<'a>(xml: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let mut at = 0usize;
    while at < xml.len() {
        let rel = xml[at..].iter().position(|b| *b == b'<')?;
        let mut p = at + rel + 1;
        // An optional namespace prefix. Scanning it rather than hard-coding
        // `mat:` because a file written by a different serialiser uses a
        // different prefix, or none, for the same element.
        let start = p;
        while p < xml.len() && is_name_byte(xml[p]) {
            p += 1;
        }
        if p < xml.len() && xml[p] == b':' {
            p += 1;
        } else {
            p = start;
        }
        if xml.len() >= p + name.len()
            && xml[p..p + name.len()].eq_ignore_ascii_case(name)
            && xml
                .get(p + name.len())
                .is_some_and(|b| b.is_ascii_whitespace() || *b == b'/' || *b == b'>')
        {
            let attrs_start = p + name.len();
            let end = attrs_start + xml[attrs_start..].iter().position(|b| *b == b'>')?;
            return Some(&xml[attrs_start..end]);
        }
        at = at + rel + 1;
    }
    None
}

/// `key="value"` or `key='value'` within an attribute span.
///
/// The key must start the span or follow whitespace, which is what keeps
/// `trans` from matching inside `useTrans` - the two differ only by a prefix
/// and both appear on every material element.
fn attr<'a>(attrs: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut at = 0usize;
    while at + key.len() <= attrs.len() {
        let boundary = at == 0 || attrs[at - 1].is_ascii_whitespace();
        if boundary && attrs[at..at + key.len()].eq_ignore_ascii_case(key) {
            let mut p = at + key.len();
            while p < attrs.len() && attrs[p].is_ascii_whitespace() {
                p += 1;
            }
            if attrs.get(p) == Some(&b'=') {
                p += 1;
                while p < attrs.len() && attrs[p].is_ascii_whitespace() {
                    p += 1;
                }
                if let Some(q @ (b'"' | b'\'')) = attrs.get(p).copied() {
                    let body = &attrs[p + 1..];
                    let end = body.iter().position(|b| *b == q)?;
                    return Some(&body[..end]);
                }
            }
        }
        at += 1;
    }
    None
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'
}

/// The five predefined entities and numeric character references, in one pass.
///
/// One pass and not five sequential replacements: replacing `&amp;` first
/// turns `&amp;lt;` into `<` instead of the literal `&lt;` it means. Material
/// names really do contain these - SketchUp's own default material is
/// `&lt;auto&gt;`.
fn decode_entities(v: &[u8]) -> String {
    let mut out = Vec::with_capacity(v.len());
    let mut i = 0usize;
    while i < v.len() {
        if v[i] != b'&' {
            out.push(v[i]);
            i += 1;
            continue;
        }
        let rest = &v[i..];
        let Some(semi) = rest.iter().take(12).position(|b| *b == b';') else {
            out.push(v[i]);
            i += 1;
            continue;
        };
        let body = &rest[1..semi];
        let replaced = match body {
            b"amp" => Some(Replacement::Byte(b'&')),
            b"lt" => Some(Replacement::Byte(b'<')),
            b"gt" => Some(Replacement::Byte(b'>')),
            b"quot" => Some(Replacement::Byte(b'"')),
            b"apos" => Some(Replacement::Byte(b'\'')),
            _ if body.first() == Some(&b'#') => numeric_ref(&body[1..]).map(Replacement::Char),
            _ => None,
        };
        match replaced {
            Some(Replacement::Byte(b)) => out.push(b),
            Some(Replacement::Char(c)) => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            None => {
                out.push(v[i]);
                i += 1;
                continue;
            }
        }
        i += semi + 1;
    }
    utf8_body(&out)
}

enum Replacement {
    Byte(u8),
    Char(char),
}

fn numeric_ref(digits: &[u8]) -> Option<char> {
    let (radix, digits) = match digits.first() {
        Some(b'x' | b'X') => (16u32, &digits[1..]),
        _ => (10u32, digits),
    };
    if digits.is_empty() {
        return None;
    }
    let mut v = 0u32;
    for d in digits {
        let n = (*d as char).to_digit(radix)?;
        v = v.checked_mul(radix)?.checked_add(n)?;
    }
    char::from_u32(v)
}

fn basename(s: &str) -> &str {
    // Both separators: a Windows authoring machine writes backslashes into an
    // attribute a Mac reader has to split.
    let cut = s.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0);
    &s[cut..]
}

fn parse_u8(v: &[u8]) -> Option<u8> {
    let s = core::str::from_utf8(v).ok()?.trim();
    // A channel written as a float ("255.0") is not something we have seen,
    // but parsing through f64 costs nothing and cannot be wrong.
    match s.parse::<u8>() {
        Ok(n) => Some(n),
        Err(_) => {
            let f = s.parse::<f64>().ok()?;
            Some(clamp01(f / 255.0).mul_add(255.0, 0.5) as u8)
        }
    }
}

fn parse_f64(v: &[u8]) -> Option<f64> {
    let f = core::str::from_utf8(v).ok()?.trim().parse::<f64>().ok()?;
    if f.is_finite() {
        Some(f)
    } else {
        None
    }
}

fn clamp01(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

// ── record plumbing, shared shape with layers.rs ─────────────────────────

fn entity_pid(rec: &Record<'_>) -> Option<u32> {
    let e = rec.child(t::ENTITY)?;
    let pid = e.child(t::ENTITY_PID)?;
    let n = pid.payload.len();
    if n == 0 || n > 8 {
        return None;
    }
    let v = pid.varint_at(0, n).ok()?;
    Some(if v > u32::MAX as u64 {
        u32::MAX
    } else {
        v as u32
    })
}

/// A string record: the whole payload, UTF-8, no count and no terminator.
/// Material folder names carry Korean in commercial packs, so this has to
/// decode rather than cast.
fn utf8_body(payload: &[u8]) -> String {
    match core::str::from_utf8(payload) {
        Ok(s) => String::from(s.trim_end_matches('\0')),
        Err(_) => {
            let mut s = String::with_capacity(payload.len());
            let mut rest = payload;
            while !rest.is_empty() {
                match core::str::from_utf8(rest) {
                    Ok(good) => {
                        s.push_str(good);
                        break;
                    }
                    Err(e) => {
                        let ok = e.valid_up_to();
                        s.push_str(core::str::from_utf8(&rest[..ok]).unwrap_or(""));
                        s.push('\u{fffd}');
                        let skip = e.error_len().unwrap_or(rest.len() - ok);
                        rest = &rest[(ok + skip).min(rest.len())..];
                    }
                }
            }
            s
        }
    }
}

/// See the note in `layers.rs`: searches rather than taking the first record,
/// so a retained section that still carries its `OverflowSizeTag` reads.
fn unwrap_to(payload: &[u8], inner: u16) -> &[u8] {
    match Tlv::new(payload).find(|r| r.tag == inner) {
        Some(r) => r.payload,
        None => payload,
    }
}
