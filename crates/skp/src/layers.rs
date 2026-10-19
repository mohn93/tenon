//! Tags (SketchUp calls them layers in the file and tags in the UI).
//!
//! The section is `SketchUpModelLayers` and its shape, read off the corpus,
//! is:
//!
//! ```text
//! SketchUpModelLayers
//!   LayerManager
//!     LayerManagerLayers
//!       Layer*                       one per tag
//!         Entity > EntityPID         the id everything else refers to
//!         LayerName                  raw UTF-8, length = the record size
//!         LayerHidden                one byte
//!         LayerMaterial > Material   the tag's colour, by reference only
//!         LayerPageBehavior          u32 flags
//!     LayerManagerCurrentLayer       a bare pid
//!     LayerManagerRootLayerGroup
//!       LayerGroup                   the unnamed root; folders hang off it
//! ```
//!
//! Three things about that are worth stating, because each one is a place a
//! reader can be confidently wrong:
//!
//! 1. **`LayerName` is not a counted UTF-16 string.** `tlv::utf16_string`
//!    reads the `u32`-counted UTF-16LE form that other parts of the format
//!    use; a layer name is *not* that. It is raw UTF-8 whose length is the
//!    record's own size - measured on the commercial classroom, where the
//!    hidden ceiling tag is the six bytes `ec b2 9c ec 9e a5`, which is
//!    `천장` in UTF-8 and nothing at all in UTF-16. The TypeScript reader
//!    decodes these as UTF-8 too, but by way of stripping NULs out of what it
//!    assumes is UTF-16, which happens to work for ASCII and mangles Korean.
//!
//! 2. **The colour is not in the record.** `LayerMaterial` holds a whole
//!    `Material` whose only content is a `MaterialXMLPath` - the name of a
//!    `materials/Layer_<name>/` folder in the zip. Every colour in a VFF file
//!    lives in that folder's `material.xml`, so [`parse_layers`] leaves
//!    `color` marked unset (alpha 0) and [`apply_layer_colors`] fills it once
//!    the material table has been merged with the xml. Trying to do it here
//!    would mean reading the zip from inside a function that was handed a
//!    single section payload.
//!
//! 3. **Folders are a separate tree, and the layers are not in it.** The
//!    layer list is flat; `LayerGroup` records form the folder hierarchy and
//!    point *back* at layers by pid. So the hierarchy has to be walked
//!    separately and joined on the id, which is what this module does, and
//!    which no existing reader does at all.
//!
//! ## What is measured and what is inferred
//!
//! Everything above is measured against the corpus. The folder *contents* are
//! not, and saying so matters more than hiding it: **no `.skp` on this machine
//! has a non-empty folder.** 555 VFF files were scanned - the whole
//! groundtruth corpus, the 2026 feature corpus, both commercial school packs,
//! every SketchUp 2026 template and shipped component - and in all of them
//! `LayerGroupChildGroups` and `LayerGroupChildLayerPIDRefs` are present and
//! zero bytes long. So the two payload encodings here are inferred:
//!
//! - `LayerGroupChildLayerPIDRefs` is read as the format's standard
//!   **PID-reference list**: a run of `[u8 width][width bytes little-endian]`
//!   entries. That encoding *is* measured, just from the other five list-type
//!   `...PIDRefs` tags, which do carry data: `ViewPageHiddenElementPIDRefs`
//!   of 6 bytes is `02 ae 12 02 b2 12`, two ids of two bytes each, and
//!   `ComponentElementPIDRefs` of 54 bytes is eighteen such entries. Singular
//!   `...PIDRef` tags have no width byte - the record size is the width.
//! - `LayerGroupChildGroups` is read as a container of nested `LayerGroup`
//!   records, which is the shape every other `<Thing>s` container in this
//!   format has (`LayerManagerLayers` holds `Layer`, `MaterialManagerMaterials`
//!   holds `Material`, `PageListPages` holds `ViewPage`).
//!
//! Both are read defensively: bytes that do not fit the shape yield no folder
//! rather than a wrong one, and a layer with no folder is the common case
//! anyway. To close this properly one fixture is needed - a model saved with
//! one nested tag folder - and the three lines of Ruby that make it are in the
//! note on [`parse_layers`].

use crate::model::{LayerDef, MaterialDef};
use crate::tags as t;
use crate::tlv::{Record, Tlv};
use alloc::string::String;
use alloc::vec::Vec;

/// How deep a folder tree may nest before we stop following it.
///
/// SketchUp's own UI stops offering "new folder inside" well before this. The
/// limit is here because the depth is file-controlled and this code runs in a
/// browser tab.
const MAX_FOLDER_DEPTH: u32 = 32;

/// Alpha 0 marks "the file carried no colour for this tag". A real tag colour
/// always has alpha 255, so this cannot collide with one.
const COLOR_UNSET: [u8; 4] = [0, 0, 0, 0];

/// Every tag in the model, in file order, with its folder path.
///
/// `color` comes back unset - see [`apply_layer_colors`]. `hidden` is the
/// layer's own flag **or-ed with every folder above it**, because SketchUp
/// hides a tag whose folder is hidden and the per-tag flag is not updated when
/// that happens: a reader that honoured only `LayerHidden` would draw a
/// ceiling the artist had switched off by its folder.
///
/// To produce the fixture that would confirm the folder encoding, with the
/// verification bridge up (`verify/su_up.sh`), `verify/ask.sh folders f.rb`:
///
/// ```text
/// m = Sketchup.active_model
/// outer = m.layers.add_folder("Outer"); inner = outer.add_folder("Inner")
/// inner.add_layer(m.layers.add("Deep")); m.save("/tmp/folders.skp"); "ok"
/// ```
pub fn parse_layers(payload: &[u8]) -> Vec<LayerDef> {
    let mut out = Vec::new();

    // The caller may hand us the section payload or the manager's own payload;
    // both happen depending on whether the section was entered or retained.
    let manager = unwrap_to(payload, t::LAYER_MANAGER);

    for list in Tlv::new(manager).filter(|r| r.tag == t::LAYER_MANAGER_LAYERS) {
        for layer in list.children().filter(|r| r.tag == t::LAYER) {
            // One malformed layer must not cost us the others, so every field
            // is optional and a layer with no id is the only thing dropped -
            // without an id nothing can refer to it and no folder can claim
            // it, so it would be invisible in the UI anyway.
            let Some(id) = entity_pid(&layer) else {
                continue;
            };
            out.push(LayerDef {
                id,
                name: layer
                    .child(t::LAYER_NAME)
                    .map(|r| utf8_body(r.payload))
                    .unwrap_or_default(),
                hidden: layer
                    .child(t::LAYER_HIDDEN)
                    .and_then(|r| r.bool_at(0).ok())
                    .unwrap_or(false),
                color: COLOR_UNSET,
                folder: Vec::new(),
            });
        }
    }

    apply_folders(manager, &mut out);
    out
}

/// Fill in `color` from the merged material table.
///
/// SketchUp stores a tag's colour as a material named `Layer_<tag name>`, and
/// that material's colour only exists in `materials/Layer_<name>/material.xml`.
/// So this is a second pass by design: call it after
/// `materials::merge_material_xml`. Both existing readers do the same join and
/// both do it by this name prefix; the convention is SketchUp's, not theirs.
///
/// Tags whose material is missing keep their unset colour rather than being
/// given a made-up one, so a caller can tell "white" from "not recorded".
pub fn apply_layer_colors(layers: &mut [LayerDef], materials: &[MaterialDef]) {
    for layer in layers.iter_mut() {
        // Build the key once per layer rather than per material: the school
        // pack has 285 materials and four tags, and the commercial classroom
        // 162 and fourteen.
        let mut key = String::with_capacity(6 + layer.name.len());
        key.push_str("Layer_");
        key.push_str(&layer.name);
        if let Some(m) = materials.iter().find(|m| m.name == key) {
            layer.color = m.rgba;
        }
    }
}

/// Whether `parse_layers` found no colour for this tag.
pub fn color_is_unset(color: [u8; 4]) -> bool {
    color[3] == 0
}

/// Walk the folder tree and stamp each layer it claims.
fn apply_folders(manager: &[u8], layers: &mut [LayerDef]) {
    for root in Tlv::new(manager).filter(|r| r.tag == t::LAYER_MANAGER_ROOT_LAYER_GROUP) {
        for group in root.children().filter(|r| r.tag == t::LAYER_GROUP) {
            // The outermost `LayerGroup` is an unnamed container that always
            // exists, even in a model with no folders at all - it is present
            // with an empty name in every file measured. Its name is not part
            // of any path, so recursion starts below it.
            descend(&group, &mut Vec::new(), false, layers, 0);
        }
    }
}

fn descend(
    group: &Record<'_>,
    path: &mut Vec<String>,
    hidden_above: bool,
    layers: &mut [LayerDef],
    depth: u32,
) {
    if depth > MAX_FOLDER_DEPTH {
        return;
    }

    // `LayerGroupHidden` is the folder's own switch. `HiddenOnNewPages` is a
    // different thing - it decides the default for scenes created later - and
    // deliberately does not affect visibility now.
    let hidden_here = hidden_above
        || group
            .child(t::LAYER_GROUP_HIDDEN)
            .and_then(|r| r.bool_at(0).ok())
            .unwrap_or(false);

    for refs in group
        .children()
        .filter(|r| r.tag == t::LAYER_GROUP_CHILD_LAYER_PID_REFS)
    {
        for id in pid_refs(refs.payload) {
            if let Some(l) = layers.iter_mut().find(|l| l.id == id) {
                // Outermost first, which is what a tree view wants and what
                // the contract in DONE.md section 0 asks for.
                l.folder = path.clone();
                l.hidden |= hidden_here;
            }
        }
    }

    for kids in group
        .children()
        .filter(|r| r.tag == t::LAYER_GROUP_CHILD_GROUPS)
    {
        for child in kids.children().filter(|r| r.tag == t::LAYER_GROUP) {
            let name = child
                .child(t::LAYER_GROUP_NAME)
                .map(|r| utf8_body(r.payload))
                .unwrap_or_default();
            path.push(name);
            descend(&child, path, hidden_here, layers, depth + 1);
            path.pop();
        }
    }
}

/// A list-type `...PIDRefs` payload: `[u8 width][width bytes LE]` repeated.
///
/// Stops at the first entry that does not fit rather than failing, so a
/// truncated list still yields the ids that were whole.
fn pid_refs(payload: &[u8]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < payload.len() {
        let width = payload[at] as usize;
        // Width 0 would make no progress and loop forever; width over 8 is not
        // an id in any file. Either means we are not looking at a ref list.
        if width == 0 || width > 8 || at + 1 + width > payload.len() {
            break;
        }
        let mut v = 0u64;
        for (i, b) in payload[at + 1..at + 1 + width].iter().enumerate() {
            v |= (*b as u64) << (8 * i);
        }
        out.push(clamp_id(v));
        at += 1 + width;
    }
    out
}

/// `Entity > EntityPID`, the id of anything in this format.
///
/// The pid is a little-endian integer whose *width is the record's size* - two
/// bytes in a small model, three in the commercial classroom, and the writer
/// grows it with the model. That is why `Entity` records come out 8 or 9 bytes
/// long across the corpus.
fn entity_pid(rec: &Record<'_>) -> Option<u32> {
    let e = rec.child(t::ENTITY)?;
    let pid = e.child(t::ENTITY_PID)?;
    let n = pid.payload.len();
    if n == 0 || n > 8 {
        return None;
    }
    Some(clamp_id(pid.varint_at(0, n).ok()?))
}

/// The contract carries ids as `u32`. Nothing observed comes close to the top
/// of that, and saturating is better than silently truncating a huge pid into
/// a collision with a real one.
fn clamp_id(v: u64) -> u32 {
    if v > u32::MAX as u64 {
        u32::MAX
    } else {
        v as u32
    }
}

/// A name record: the whole payload is the string, UTF-8, no count, no
/// terminator. Invalid bytes become replacement characters rather than
/// dropping the record - one odd byte in one tag name must not cost the model.
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
                        // Safety of the index: `valid_up_to` is by definition
                        // a char boundary within `rest`.
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

/// If `payload` holds a record tagged `inner`, return that record's payload;
/// otherwise return `payload` unchanged.
///
/// This is the one piece of plumbing every section decoder needs. `walk.rs`
/// solves the same problem for the streaming walk by *entering* a record; here
/// the section has already been retained whole, so unwrapping is a slice.
///
/// It searches rather than taking the first record, for two reasons. A caller
/// that retained the section with `RecordStream::payload` keeps the raw bytes,
/// which begin with an `OverflowSizeTag` whenever the section needed one -
/// `enter` consumes that, `payload` does not. And accepting a manager record
/// that is not alone lets a section with trailing padding still read. The
/// wrapper tags are container tags that never appear inside their own bodies,
/// so the search cannot match the wrong thing.
fn unwrap_to(payload: &[u8], inner: u16) -> &[u8] {
    match Tlv::new(payload).find(|r| r.tag == inner) {
        Some(r) => r.payload,
        None => payload,
    }
}
