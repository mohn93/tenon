//! Styles, and the rendering options that are what a style actually is.
//!
//! This is the module with the largest gap between what the format carries and
//! what any existing reader exposes. openskp's five ports expose **two**
//! values per style - a "front" and a "back" face colour scraped out of
//! `styles/<name>/style.xml` with a regex over numeric item ids 4000 and 4001.
//! Both of those are wrong twice over, and the format has 77 named options:
//!
//! - id 4000 is not the front face colour, it is `BackgroundColor`, and 4001 is
//!   not the back face colour, it is `SkyColor`. The face colours are 2002 and
//!   2003. The mapping was recovered by matching every value in a model's
//!   `SketchUpModelRenderingOptions` section against every item in the same
//!   file's active `style.xml`, over 33 files, and intersecting the candidate
//!   sets: 22 ids come out uniquely determined, among them
//!   `1005 LineExtension`, `1006 DrawSilhouettes`, `1007 SilhouetteWidth`,
//!   `2001 RenderMode`, `2002 FaceFrontColor`, `2003 FaceBackColor`,
//!   `4000 BackgroundColor`, `4001 SkyColor`. The rest collapse into groups
//!   whose members hold the same value in every file, which no amount of
//!   corpus closes.
//! - the channel order is reversed. The xml stores the colour as a signed
//!   `int32` whose little-endian bytes are **R, G, B, A**, so red is the low
//!   byte; openskp reads the low byte as blue. Three independent checks say
//!   so: `LockedColor` comes out red and `HighlightColor` blue (SketchUp draws
//!   locked groups red and the selection blue, not the other way round), and
//!   in a style of this project's own authoring `SkyColor` 0xFFD0CDC6 is the
//!   pale blue (198, 205, 208) rather than the sand (208, 205, 198), while
//!   `GroundColor` 0xFFB9D0D2 is the khaki (210, 208, 185) rather than a
//!   second blue. `model.dat` stores the same four bytes in the same order, so
//!   a colour option is read straight out of the payload.
//!
//! ## Where a style's values live
//!
//! In a VFF file a `SkpStyle` record holds **only** a guid, a name and a
//! watermark list:
//!
//! ```text
//! SketchUpModelStyles
//!   SkpStyleManager
//!     SkpStyleManagerStyles
//!       SkpStyle*
//!         Entity > EntityPID
//!         SkpStyleGuid                 16 bytes
//!         SkpStyleLoadedFrom
//!         SkpStyleExtBufferPath        "[HiddenLine]" - the styles/ folder
//!         SkpStyleWatermarkPIDRefs
//!     SkpStyleManagerSelectedStylePIDRef
//!     SkpStyleManagerWorkingStyle      a second SkpStyle, the edited copy
//! ```
//!
//! The settings are not in there. They are in `SketchUpModelRenderingOptions`
//! for the document, in `ViewPageRenderingOptions` for each scene, and in the
//! external `styles/<ExtBufferPath>/style.xml` for the named style. So
//! [`parse_styles`] returns identity plus defaults, and the values a renderer
//! needs come from [`parse_rendering_options`] over the section or over a
//! scene's own copy - which is the right source anyway, because that is what
//! SketchUp restores when the scene is selected.
//!
//! [`parse_styles`] does decode a `RenderingOptions` record nested inside a
//! `SkpStyle` when it finds one, because the record shape allows it and a
//! writer that embeds it should not be silently ignored. No file on this
//! machine does - checked across the whole corpus - so that path is untested
//! against real bytes and says so here rather than pretending.
//!
//! ## Counting styles
//!
//! `SkpStyleManagerStyles` holds 1, 2 and 2 styles for the powerbank, the
//! commercial classroom and the school pack. openskp reports 3 for each,
//! because it counts `styles/*/style.xml` entries and every file ships one
//! extra - `[HiddenLine]_1`, `[Simple Style]_1` - which is the *working copy*
//! of the active style, the same style with unsaved edits, carried separately
//! as `SkpStyleManagerWorkingStyle`. The manager is the authority; a style
//! list with a duplicate in it is a reader artefact.

use crate::model::StyleDef;
use crate::tags as t;
use crate::tlv::{Record, Tlv};
use alloc::string::String;
use alloc::vec::Vec;

/// How a rendering option's payload is encoded.
///
/// Determined by reading every `RenderingOptions` child in the corpus and
/// cross-checking the value against the same file's `style.xml`, where each
/// item declares a variant type (1 bool, 4 int, 5 colour-as-int, 6 float,
/// 7 double). That is what separates [`OptionKind::I32`] from
/// [`OptionKind::F32`], which are both four bytes: `AODistance` is
/// `00 00 20 41`, which is 1092616192 as an integer and 10.0 as a float, and
/// the style document's item 8102 declares type 6 with the value 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    /// One byte, zero or non-zero.
    Bool,
    /// Four bytes, little-endian signed. Widths, modes and percentages.
    I32,
    /// Four bytes, IEEE single. Only the ambient-occlusion scalars.
    F32,
    /// Eight bytes, IEEE double. Distances, fades and opacities.
    F64,
    /// Four bytes, in the order red, green, blue, alpha.
    Color,
    /// The tag is in SketchUp's dictionary but appears in no file here, so its
    /// payload encoding is not established. Reading one is refused rather than
    /// guessed.
    Undetermined,
}

/// Every `RenderingOptions*` member tag and its payload encoding.
///
/// 77 entries - the 78 `RENDERING_OPTIONS*` names in the tag dictionary less
/// the `RenderingOptions` container itself. 75 are observed in files on this
/// machine and their encodings are measured. The remaining two are marked
/// [`OptionKind::Undetermined`]:
///
/// - `RenderingOptionsSectionDefaultFillMaterial` (0x7388). The name says it
///   refers to a material, so it is most likely a pid reference, but a
///   reference is one to eight bytes wide with no self-description and
///   guessing would produce a material id out of thin air.
/// - `RenderingOptionsDefaultSectionFillStyleEnabled` (0x738C). Almost
///   certainly the one byte every other `*Enabled` option is. Still a guess.
///
/// Neither has a field in [`StyleDef`], so nothing a caller can see is
/// affected; what is affected is anyone who later adds section-fill support
/// and needs to know these two were never confirmed.
///
/// Three numbers in the 0x7340-0x738C range are absent from the dictionary
/// altogether - 0x734A, 0x7384 and 0x7387 - and absent from every file.
pub const OPTIONS: &[(u16, OptionKind)] = &[
    (t::RENDERING_OPTIONS_RENDER_MODE, OptionKind::I32),
    (t::RENDERING_OPTIONS_ENABLE_TRANSPARENCY, OptionKind::Bool),
    (t::RENDERING_OPTIONS_MATERIAL_TRANSPARENCY, OptionKind::Bool),
    (t::RENDERING_OPTIONS_ENABLE_TEXTURE, OptionKind::Bool),
    (t::RENDERING_OPTIONS_EDGE_DISPLAY_MODE, OptionKind::I32),
    (t::RENDERING_OPTIONS_EDGE_TYPE, OptionKind::I32),
    (t::RENDERING_OPTIONS_DISPLAY_SKETCH_AXES, OptionKind::Bool),
    (t::RENDERING_OPTIONS_DISPLAY_TEXT, OptionKind::Bool),
    (t::RENDERING_OPTIONS_DISPLAY_DIMS, OptionKind::Bool),
    (
        t::RENDERING_OPTIONS_HIDE_CONSTRUCTION_GEOMETRY,
        OptionKind::Bool,
    ),
    (t::RENDERING_OPTIONS_COLOR_BY_LAYER, OptionKind::Bool),
    (t::RENDERING_OPTIONS_EDGE_COLOR_MODE, OptionKind::I32),
    (t::RENDERING_OPTIONS_DISPLAY_INSTANCE_AXES, OptionKind::Bool),
    (t::RENDERING_OPTIONS_JITTER_EDGES, OptionKind::Bool),
    (t::RENDERING_OPTIONS_LINE_STYLE_EDGES, OptionKind::Bool),
    (t::RENDERING_OPTIONS_EXTEND_LINES, OptionKind::Bool),
    (t::RENDERING_OPTIONS_LINE_EXTENSION, OptionKind::I32),
    (t::RENDERING_OPTIONS_DRAW_SILHOUETTES, OptionKind::Bool),
    (t::RENDERING_OPTIONS_SILHOUETTE_WIDTH, OptionKind::I32),
    (t::RENDERING_OPTIONS_DRAW_DEPTH_QUE, OptionKind::Bool),
    (t::RENDERING_OPTIONS_DEPTH_QUE_WIDTH, OptionKind::I32),
    (t::RENDERING_OPTIONS_DRAW_LINE_ENDS, OptionKind::Bool),
    (t::RENDERING_OPTIONS_LINE_END_WIDTH, OptionKind::I32),
    (t::RENDERING_OPTIONS_DRAW_PROFILES_ONLY, OptionKind::Bool),
    (t::RENDERING_OPTIONS_DRAW_BACK_EDGES, OptionKind::Bool),
    (t::RENDERING_OPTIONS_BACKGROUND_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_FOREGROUND_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_HIGHLIGHT_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_LOCKED_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_CONSTRUCTION_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_FACE_FRONT_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_FACE_BACK_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_DISPLAY_WATERMARKS, OptionKind::Bool),
    (t::RENDERING_OPTIONS_DISPLAY_FOG, OptionKind::Bool),
    (t::RENDERING_OPTIONS_FOG_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_FOG_USE_BK_COLOR, OptionKind::Bool),
    (t::RENDERING_OPTIONS_FOG_START_DIST, OptionKind::F64),
    (t::RENDERING_OPTIONS_FOG_END_DIST, OptionKind::F64),
    (t::RENDERING_OPTIONS_FOG_HINT, OptionKind::I32),
    (t::RENDERING_OPTIONS_SKY_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_HORIZON_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_GROUND_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_DRAW_HORIZON, OptionKind::Bool),
    (t::RENDERING_OPTIONS_DRAW_GROUND, OptionKind::Bool),
    (t::RENDERING_OPTIONS_DRAW_UNDERGROUND, OptionKind::Bool),
    (t::RENDERING_OPTIONS_GROUND_TRANSPARENCY, OptionKind::I32),
    (t::RENDERING_OPTIONS_INACTIVE_FADE, OptionKind::F64),
    (t::RENDERING_OPTIONS_INSTANCE_FADE, OptionKind::F64),
    (t::RENDERING_OPTIONS_INACTIVE_HIDDEN, OptionKind::Bool),
    (t::RENDERING_OPTIONS_INSTANCE_HIDDEN, OptionKind::Bool),
    (t::RENDERING_OPTIONS_SECTION_ACTIVE_COLOR, OptionKind::Color),
    (
        t::RENDERING_OPTIONS_SECTION_INACTIVE_COLOR,
        OptionKind::Color,
    ),
    (
        t::RENDERING_OPTIONS_SECTION_DEFAULT_CUT_COLOR,
        OptionKind::Color,
    ),
    (
        t::RENDERING_OPTIONS_SECTION_DEFAULT_FILL_COLOR,
        OptionKind::Color,
    ),
    (t::RENDERING_OPTIONS_SECTION_CUT_WIDTH, OptionKind::I32),
    (t::RENDERING_OPTIONS_SECTION_DISPLAY_MODE, OptionKind::I32),
    (t::RENDERING_OPTIONS_SECTION_CUT_FILLED, OptionKind::Bool),
    (t::RENDERING_OPTIONS_TRANSPARENCY_SORT, OptionKind::I32),
    (t::RENDERING_OPTIONS_XRAY_OPACITY, OptionKind::F64),
    (t::RENDERING_OPTIONS_HIDE_SOFT_EDGES, OptionKind::Bool),
    (t::RENDERING_OPTIONS_SOFT_EDGE_LIMIT, OptionKind::F64),
    (t::RENDERING_OPTIONS_AVERAGE_SOFT_NORMALS, OptionKind::Bool),
    (
        t::RENDERING_OPTIONS_PHOTOMATCH_DRAW_BACKGROUND,
        OptionKind::Bool,
    ),
    (
        t::RENDERING_OPTIONS_PHOTOMATCH_BACKGROUND_OPACITY,
        OptionKind::F64,
    ),
    (
        t::RENDERING_OPTIONSB_PHOTOMATCH_DRAW_OVERLAY,
        OptionKind::Bool,
    ),
    (
        t::RENDERING_OPTIONS_PHOTOMATCH_OVERLAY_OPACITY,
        OptionKind::F64,
    ),
    (t::RENDERING_OPTIONS_DRAW_HIDDEN_GEOMETRY, OptionKind::Bool),
    (t::RENDERING_OPTIONS_DRAW_HIDDEN_OBJECTS, OptionKind::Bool),
    (
        t::RENDERING_OPTIONS_HIDE_CUSTOM_CONTROL_POINTS,
        OptionKind::Bool,
    ),
    (t::RENDERING_OPTIONS_AO_ENABLED, OptionKind::Bool),
    (t::RENDERING_OPTIONS_AO_DISTANCE, OptionKind::F32),
    (t::RENDERING_OPTIONS_AO_CONTRAST_FACTOR, OptionKind::F32),
    (
        t::RENDERING_OPTIONS_SECTION_DEFAULT_FILL_MATERIAL,
        OptionKind::Undetermined,
    ),
    (t::RENDERING_OPTIONS_AO_MULTIPLIER, OptionKind::F32),
    (t::RENDERING_OPTIONS_AO_COLOR, OptionKind::Color),
    (t::RENDERING_OPTIONS_AO_COLOR_ENABLED, OptionKind::Bool),
    (
        t::RENDERING_OPTIONS_DEFAULT_SECTION_FILL_STYLE_ENABLED,
        OptionKind::Undetermined,
    ),
];

/// The encoding of one option tag, or `None` if the tag is not one.
pub fn option_kind(tag: u16) -> Option<OptionKind> {
    OPTIONS.iter().find(|(t, _)| *t == tag).map(|(_, k)| *k)
}

/// How many bytes a payload of this kind must be. `None` for undetermined.
pub fn option_width(kind: OptionKind) -> Option<usize> {
    match kind {
        OptionKind::Bool => Some(1),
        OptionKind::I32 | OptionKind::F32 | OptionKind::Color => Some(4),
        OptionKind::F64 => Some(8),
        OptionKind::Undetermined => None,
    }
}

/// Decode a `RenderingOptions` payload into the line-art settings.
///
/// Takes the `SketchUpModelRenderingOptions` section payload, a scene's
/// `ViewPageRenderingOptions` payload, or a bare `RenderingOptions` record's
/// payload - all three occur and all three are the same record underneath.
///
/// Every field of [`StyleDef`] has a tag and every one of them is read here.
/// An option the file omits keeps SketchUp's factory value from
/// `StyleDef::default`, which is the right behaviour and not a shortcut: a
/// file written by an older SketchUp simply has fewer options, and the missing
/// ones are the ones the writer had no concept of.
pub fn parse_rendering_options(payload: &[u8]) -> StyleDef {
    let mut s = StyleDef::default();
    apply_rendering_options(&mut s, payload);
    s
}

/// Apply a `RenderingOptions` payload onto an existing style.
pub fn apply_rendering_options(style: &mut StyleDef, payload: &[u8]) {
    // The section wraps exactly one RenderingOptions record; a scene's copy
    // does too. Unwrapping is a slice, so accept either depth.
    let body = unwrap_to(payload, t::RENDERING_OPTIONS);

    for r in Tlv::new(body) {
        // Width is checked against the table before reading, so a record whose
        // size does not match its tag is skipped instead of being
        // reinterpreted - which is how a four-byte float would otherwise be
        // read as a plausible-looking integer.
        let Some(kind) = option_kind(r.tag) else {
            continue;
        };
        if option_width(kind) != Some(r.payload.len()) {
            continue;
        }
        match r.tag {
            t::RENDERING_OPTIONS_RENDER_MODE => style.render_mode = i32_of(&r),
            t::RENDERING_OPTIONS_DRAW_SILHOUETTES => style.draw_silhouettes = bool_of(&r),
            t::RENDERING_OPTIONS_SILHOUETTE_WIDTH => style.silhouette_width = i32_of(&r),
            t::RENDERING_OPTIONS_EXTEND_LINES => style.extend_lines = bool_of(&r),
            t::RENDERING_OPTIONS_LINE_EXTENSION => style.line_extension = i32_of(&r),
            t::RENDERING_OPTIONS_JITTER_EDGES => style.jitter_edges = bool_of(&r),
            t::RENDERING_OPTIONS_DRAW_DEPTH_QUE => style.draw_depth_cue = bool_of(&r),
            t::RENDERING_OPTIONS_DEPTH_QUE_WIDTH => style.depth_cue_width = i32_of(&r),
            t::RENDERING_OPTIONS_DRAW_LINE_ENDS => style.draw_line_ends = bool_of(&r),
            t::RENDERING_OPTIONS_LINE_END_WIDTH => style.line_end_width = i32_of(&r),
            t::RENDERING_OPTIONS_DRAW_PROFILES_ONLY => style.draw_profiles_only = bool_of(&r),
            t::RENDERING_OPTIONS_DRAW_BACK_EDGES => style.draw_back_edges = bool_of(&r),
            t::RENDERING_OPTIONS_EDGE_DISPLAY_MODE => style.edge_display_mode = i32_of(&r),
            t::RENDERING_OPTIONS_EDGE_COLOR_MODE => style.edge_color_mode = i32_of(&r),
            t::RENDERING_OPTIONS_COLOR_BY_LAYER => style.display_color_by_layer = bool_of(&r),
            t::RENDERING_OPTIONS_ENABLE_TEXTURE => style.texture_enabled = bool_of(&r),
            t::RENDERING_OPTIONS_BACKGROUND_COLOR => style.background = color_of(&r),
            t::RENDERING_OPTIONS_FOREGROUND_COLOR => style.foreground = color_of(&r),
            t::RENDERING_OPTIONS_FACE_FRONT_COLOR => style.face_front = color_of(&r),
            t::RENDERING_OPTIONS_FACE_BACK_COLOR => style.face_back = color_of(&r),
            t::RENDERING_OPTIONS_DISPLAY_WATERMARKS => style.display_watermarks = bool_of(&r),
            t::RENDERING_OPTIONS_DISPLAY_SKETCH_AXES => style.display_sketch_axes = bool_of(&r),
            // Decoded but with nowhere in the contract to put it: fog, the
            // section-plane colours, the photo-match overlay, the ambient
            // occlusion scalars, the soft-edge limit. Their encodings are in
            // OPTIONS so a later field costs one line here, not another day
            // of measurement.
            _ => {}
        }
    }
}

/// Every named style in the model, in file order.
///
/// `name` is `SkpStyleExtBufferPath`, which is the `styles/<name>/` folder and
/// matches the `name` attribute of that folder's `style.xml` in every file
/// checked - `[HiddenLine]`, `[Architectural Design Style]`, `[Simple Style]`.
/// Reading the name from `model.dat` rather than the xml means the style list
/// is available without touching the zip, which is what the one-second index
/// budget needs.
///
/// The settings come back at `StyleDef::default` unless the record carries a
/// nested `RenderingOptions`; see the module note on why.
pub fn parse_styles(payload: &[u8]) -> Vec<StyleDef> {
    let mut out = Vec::new();
    let manager = unwrap_to(payload, t::SKP_STYLE_MANAGER);
    for list in Tlv::new(manager).filter(|r| r.tag == t::SKP_STYLE_MANAGER_STYLES) {
        for st in list.children().filter(|r| r.tag == t::SKP_STYLE) {
            let Some(id) = entity_pid(&st) else { continue };
            let mut def = StyleDef {
                id,
                name: st
                    .child(t::SKP_STYLE_EXT_BUFFER_PATH)
                    .map(|r| utf8_body(r.payload))
                    .unwrap_or_default(),
                ..Default::default()
            };
            if let Some(ro) = st.child(t::RENDERING_OPTIONS) {
                apply_rendering_options(&mut def, ro.payload);
            }
            out.push(def);
        }
    }
    out
}

/// The style the document opens with, as a pid.
///
/// `SceneIndex::active_style` has nowhere else to come from: a scene's
/// `ViewPageStylePIDRef` says which style *that scene* restores, and the
/// document-level answer is only here.
pub fn selected_style_id(payload: &[u8]) -> Option<u32> {
    let manager = unwrap_to(payload, t::SKP_STYLE_MANAGER);
    let r = Tlv::new(manager).find(|r| r.tag == t::SKP_STYLE_MANAGER_SELECTED_STYLE_PID_REF)?;
    let n = r.payload.len();
    if n == 0 || n > 8 {
        return None;
    }
    let v = r.varint_at(0, n).ok()?;
    Some(if v > u32::MAX as u64 {
        u32::MAX
    } else {
        v as u32
    })
}

fn bool_of(r: &Record<'_>) -> bool {
    r.bool_at(0).unwrap_or(false)
}

fn i32_of(r: &Record<'_>) -> i32 {
    r.u32_at(0).map(|v| v as i32).unwrap_or(0)
}

/// Four bytes, red first.
///
/// Not an ARGB integer. Reading it as one swaps red and blue, which is exactly
/// the bug in the two existing readers and the reason the module note spells
/// out the three checks that settle the order.
fn color_of(r: &Record<'_>) -> [u8; 4] {
    match (r.u8_at(0), r.u8_at(1), r.u8_at(2), r.u8_at(3)) {
        (Ok(a), Ok(b), Ok(c), Ok(d)) => [a, b, c, d],
        _ => [0, 0, 0, 255],
    }
}

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

/// The table must stay the same size as the dictionary's own option list, or a
/// tag has been added without an encoding and `apply_rendering_options` would
/// skip it silently.
const _: () = {
    assert!(OPTIONS.len() == 77);
};
