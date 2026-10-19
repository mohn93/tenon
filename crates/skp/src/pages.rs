//! Saved scenes - SketchUp calls them pages - and the camera a document opens
//! with.
//!
//! The nesting is fixed and shallow, so this module walks it by name rather
//! than searching:
//!
//! ```text
//! SketchUpModelPages
//!   PageList
//!     PageListPages
//!       ViewPage                         one per scene tab, in tab order
//!         SketchUpPage                   Entity(PID), name, description
//!         ViewPageFlags                  which properties the tab restores
//!         ViewPageCamera > Camera        absent when the tab restores no camera
//!         ViewPageStylePIDRef            bare varint
//!         ViewPageLayerPIDRefs           (u8 width, varint) runs
//!         ViewPageSectionPlanePIDRefs    same encoding
//!         ...
//!     PageListSelectedPagePIDRef
//! ```
//!
//! Three things were settled against real files rather than assumed, because
//! all three are easy to get wrong in a way that still parses:
//!
//! 1. **Names are bare UTF-8**, filling the record payload, with no length
//!    prefix and no UTF-16 anywhere. `tlv::utf16_string` is the wrong tool
//!    here even though the container header's encoding suggests it: fed
//!    `Scene 1` it reads the first four bytes as a count of 1,852,404,051 code
//!    units and fails. Over 250 pages in 130 files every `SketchUpPageName`
//!    payload decoded as valid UTF-8, including Korean tab names such as
//!    `cut01_승강장_정면`, whose 22 bytes are exactly its UTF-8 length.
//!    `layers.rs` reached the same conclusion about `LayerName`
//!    independently.
//!
//! 2. **A page with no camera is still a page.** `pair_scene-no-camera_vff2026`
//!    carries one `ViewPage` with no `ViewPageCamera` child at all, and
//!    SketchUp shows that tab. The TypeScript reader drops any page missing
//!    eye or target, so it reports zero scenes for a file whose name says one.
//!    Here the absence is reported as `eye`/`target`/`up` being `None`.
//!
//! 3. **`ortho_height` really is inches.** Across every parallel-projection
//!    page in the corpus, `CameraHeight * 25.4` lands on an exact round
//!    millimetre figure - 7000, 7120, 10800, 11300, 12000, 14400, 15000,
//!    28000, 40000, 62000 - which is what an author typing a metric plan width
//!    produces and what an accidental second conversion would destroy.
//!
//! Legacy (pre-2021) files store scenes in a different container entirely and
//! are not this module's problem.

use crate::container::{Container, Generation};
use crate::error::{Error, Result};
use crate::model::{Mm, SceneDef, INCH_TO_MM};
use crate::source::ByteSource;
use crate::stream::{InflateRead, RangeRead, RecordStream, SeqRead};
use crate::tags;
use crate::tlv::{Record, Tlv};
use crate::zip::entry_data_offset;
use alloc::string::String;
use alloc::vec::Vec;

/// Bits of `ViewPageFlags`, which say which properties a tab restores when you
/// click it. They match SketchUp's public `SUSceneFlags`, and bits 0-3 and 6
/// were each confirmed one at a time against a set of fixtures that toggle a
/// single scene checkbox: `scene_only_use_camera.skp` is `0xe21`,
/// `scene_only_use_style.skp` `0xe22`, `..._shadow_info` `0xe24`,
/// `..._axes` `0xe28`, `..._section_planes` `0xe60`, against a `0xe20`
/// baseline with every box cleared. In each case the single added bit is
/// exactly the bit below, and the corresponding `ViewPage*` child record
/// appears in the file only when its bit is set - which is why
/// `VIEW_PAGE_CAMERA` can be missing.
pub mod page_flag {
    /// The tab restores the camera. Proven: clearing it is what removes the
    /// `ViewPageCamera` record.
    pub const CAMERA_LOCATION: u32 = 0x0001;
    /// Style and fog; carried in the file as `ViewPageRenderingOptions`.
    pub const DRAWING_STYLE: u32 = 0x0002;
    pub const SHADOW_SETTINGS: u32 = 0x0004;
    /// Drawing-axes position, carried as `ViewPageSketchCS`.
    pub const AXES_LOCATION: u32 = 0x0008;
    /// Hidden geometry. `scene_only_use_hidden.skp` sets this together with
    /// bits 7 and 8, so the three move as a group in SketchUp 2026 and only
    /// this one is named by the public enum.
    pub const HIDDEN_GEOMETRY: u32 = 0x0010;
    /// Visible tags. Set in every page of every SketchUp-written file
    /// measured, which is why those files carry a `ViewPageLayerPIDRefs`
    /// record even when it is empty. This project's own writer omits both the
    /// flags and the empty list, which is what `ALL` below is for.
    pub const VISIBLE_LAYERS: u32 = 0x0020;
    pub const ACTIVE_SECTION_PLANES: u32 = 0x0040;

    /// `SUSceneAll`. Used when a file carries no `ViewPageFlags` record at
    /// all, which this project's own writer produces: a freshly created
    /// SketchUp scene has every checkbox ticked, so all-on mirrors the engine,
    /// where zero would read as "this tab restores nothing" and silently
    /// discard a camera the file does contain.
    pub const ALL: u32 = 0x0FFF;
}

/// SketchUp's `CCamera` construction values, used when a tag is missing.
///
/// Every camera written by SketchUp itself - 368 of them across 130 files -
/// stores these three verbatim for near, far and FOV unless the user changed
/// the FOV, so defaulting to them reproduces the engine rather than inventing
/// a frustum. Inches, as everything in the file is.
const DEFAULT_NEAR_IN: f64 = 1.0;
const DEFAULT_FAR_IN: f64 = 1000.0;
const DEFAULT_FOV_DEG: f64 = 35.0;

/// A `SceneDef` whose camera half is a freshly constructed `CCamera`.
///
/// This is the floor for every scene, including one that restores no camera at
/// all. `SceneDef::default()` would make such a tab a zero-degree *parallel*
/// camera with a zero-height frustum - a thing no renderer can draw and that
/// no file ever says - so the absence of a camera is signalled by `eye`,
/// `target` and `up` being `None` and nowhere else.
fn with_default_camera() -> SceneDef {
    SceneDef {
        perspective: true,
        fov_deg: DEFAULT_FOV_DEG,
        fov_is_height: true,
        near: DEFAULT_NEAR_IN * INCH_TO_MM,
        far: DEFAULT_FAR_IN * INCH_TO_MM,
        ..SceneDef::default()
    }
}

/// Parse the `SketchUpModelPages` section payload into scenes, in tab order.
///
/// Never fails and never panics: a `ViewPage` whose bytes are wrong loses only
/// the fields that could not be read, and the surrounding pages still come
/// back. That matters because the scene list is the first thing a viewer draws
/// and one bad tab should not blank it.
pub fn parse_pages(payload: &[u8]) -> Vec<SceneDef> {
    let mut out = Vec::new();
    // Both levels are iterated rather than indexed because nothing guarantees
    // a single PageList, and a second one would otherwise be silently dropped.
    for list in Tlv::new(payload).filter(|r| r.tag == tags::PAGE_LIST) {
        for pages in list.children_with(tags::PAGE_LIST_PAGES) {
            for view_page in pages.children_with(tags::VIEW_PAGE) {
                out.push(scene_of(&view_page));
            }
        }
    }
    out
}

/// The PID of the tab that was selected when the file was saved, if recorded.
///
/// Matches `SceneDef`'s ids only once a caller has the page PIDs, which this
/// module does not expose, so it is here for a UI that wants to open on the
/// right tab and nothing else.
pub fn selected_page_id(payload: &[u8]) -> Option<u32> {
    for list in Tlv::new(payload).filter(|r| r.tag == tags::PAGE_LIST) {
        if let Some(r) = list.child(tags::PAGE_LIST_SELECTED_PAGE_PID_REF) {
            if let Some(id) = pid_ref(&r) {
                return Some(id);
            }
        }
    }
    None
}

/// Parse the `SketchUpModelCamera` section payload: the camera the document
/// opens with, which is not one of the saved scenes.
///
/// Returned as a `SceneDef` so a viewer has one camera type, with `name` left
/// empty - SketchUp has no name for this camera - and `flags` set to
/// `CAMERA_LOCATION` alone, which is the truth about it: it restores a camera
/// and nothing else.
pub fn parse_camera(payload: &[u8]) -> Option<SceneDef> {
    let camera = Tlv::new(payload).find(|r| r.tag == tags::CAMERA)?;
    let mut scene = SceneDef {
        flags: page_flag::CAMERA_LOCATION,
        ..with_default_camera()
    };
    read_camera(&camera, &mut scene);
    // The page description belongs to the page; this camera has only its own.
    scene.description = text_of(&camera, tags::CAMERA_DESCRIPTION);
    Some(scene)
}

/// The four camera tags that `SceneDef` has nowhere to put: SketchUp's 2D
/// camera, used by Match Photo and by LayOut viewports.
///
/// All 371 cameras in the corpus store this at identity - `is_2d` false,
/// `scale` 1.0, both centres 0.0 - so nothing here is exercised by a real
/// file and nothing is lost by `parse_pages` not carrying it. It exists so
/// that "every camera field is read" is true rather than nearly true.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2d {
    pub is_2d: bool,
    pub scale: f64,
    /// Centre offset in the camera's own 2D space; not a length in the model,
    /// so not converted.
    pub center: [f64; 2],
}

/// Parse the 2D camera out of the same payload `parse_camera` takes, or out of
/// a `ViewPageCamera` payload.
pub fn parse_camera_2d(payload: &[u8]) -> Option<Camera2d> {
    let camera = Tlv::new(payload).find(|r| r.tag == tags::CAMERA)?;
    Some(Camera2d {
        is_2d: bool_of(&camera, tags::CAMERA_IS2_D, false),
        scale: f64_of(&camera, tags::CAMERA2_D_SCALE, 1.0),
        center: [
            f64_of(&camera, tags::CAMERA2_DX_CENTER, 0.0),
            f64_of(&camera, tags::CAMERA2_DY_CENTER, 0.0),
        ],
    })
}

fn scene_of(view_page: &Record) -> SceneDef {
    let mut scene = with_default_camera();

    if let Some(head) = view_page.child(tags::SKETCH_UP_PAGE) {
        scene.name = text_of(&head, tags::SKETCH_UP_PAGE_NAME);
        scene.description = text_of(&head, tags::SKETCH_UP_PAGE_DESCRIPTION);
    }
    if scene.name.is_empty() {
        // `ViewPageTabText` duplicates the name in 245 of 250 pages measured
        // and is *empty* in the other five, so the tab text is the fallback
        // and never the source. No page had an empty name, so this never
        // fired on the corpus; it is here so a file that does lose the name
        // still shows a tab label.
        scene.name = text_of(view_page, tags::VIEW_PAGE_TAB_TEXT);
    }

    scene.flags = match view_page.child(tags::VIEW_PAGE_FLAGS).map(|r| r.u32_at(0)) {
        Some(Ok(f)) => f,
        _ => page_flag::ALL,
    };

    // ViewPageCamera wraps exactly one Camera. Its absence is meaningful, not
    // an error: this tab does not restore a camera.
    if let Some(wrap) = view_page.child(tags::VIEW_PAGE_CAMERA) {
        if let Some(camera) = wrap.child(tags::CAMERA) {
            read_camera(&camera, &mut scene);
        }
    }

    scene.style_id = view_page
        .child(tags::VIEW_PAGE_STYLE_PID_REF)
        .and_then(|r| pid_ref(&r));
    scene.hidden_layer_ids = view_page
        .child(tags::VIEW_PAGE_LAYER_PID_REFS)
        .map(pid_ref_list)
        .unwrap_or_default();
    scene.section_plane_ids = view_page
        .child(tags::VIEW_PAGE_SECTION_PLANE_PID_REFS)
        .map(pid_ref_list)
        .unwrap_or_default();

    scene
}

/// Read a `Camera` record into the camera half of a `SceneDef`.
///
/// What each field means is in `reference/RENDER.md` section 3 and is the
/// reason all of them are read rather than the seven the other readers take:
///
/// - `fov_deg` is degrees, and `fov_is_height` says whether it is already the
///   *vertical* field of view. When false it is horizontal and
///   `GetHeightFieldOfView` divides `tan(fov/2)` by the effective aspect, so a
///   reader that ignores the flag frames the shot wrong. The default here is
///   `true` because that is the no-op branch when `aspect_ratio` is zero, and
///   because all 368 SketchUp-written cameras measured store `true`.
/// - `ortho_height` is the full vertical extent of the parallel frustum and is
///   the only thing that sets framing when `perspective` is false. In a
///   perspective camera it is stale - SketchUp leaves its 133 inch default
///   sitting there - so it must not be used to second-guess the FOV.
/// - `near`/`far` as stored are the constructor's 1 inch and 1000 inch in every
///   file measured. SketchUp throws them away at draw time and recomputes from
///   the model bounds, then caps `far/near` at 3000:1; `derive_near_far` does
///   that, and the stored pair is carried through only so nothing is silently
///   dropped.
/// - `image_width_mm` is **not** converted. RENDER.md records the camera's stored image width as
///   millimetres, and SketchUp's angle-to-focal-length treats anything below 0.1 as the
///   35 mm frame by using a half-aperture of 18.0 mm - a number that only makes
///   sense if the field it replaces is also millimetres. Every camera in the
///   corpus stores 0.0, so there is no fixture that could have told the
///   difference; the disassembly is the evidence.
fn read_camera(camera: &Record, scene: &mut SceneDef) {
    scene.eye = vec3_of(camera, tags::CAMERA_EYE).map(to_mm);
    scene.target = vec3_of(camera, tags::CAMERA_TARGET).map(to_mm);
    // `up` is a direction. Scaling it would not change the view but would make
    // every consumer renormalise, and a reader that scaled it once would scale
    // it twice somewhere.
    scene.up = vec3_of(camera, tags::CAMERA_UP);

    scene.perspective = bool_of(camera, tags::CAMERA_PERSPECTIVE, true);
    scene.fov_deg = f64_of(camera, tags::CAMERA_FOV, DEFAULT_FOV_DEG);
    scene.fov_is_height = bool_of(camera, tags::CAMERA_FOV_IS_HEIGHT, true);
    scene.fixed_fov = bool_of(camera, tags::CAMERA_FIXED_FOV, false);
    scene.ortho_height = f64_of(camera, tags::CAMERA_HEIGHT, 0.0) * INCH_TO_MM;
    scene.aspect_ratio = f64_of(camera, tags::CAMERA_ASPECT_RATIO, 0.0);
    scene.image_width_mm = f64_of(camera, tags::CAMERA_IMAGE_WIDTH, 0.0);
    scene.near = f64_of(camera, tags::CAMERA_NEAR, DEFAULT_NEAR_IN) * INCH_TO_MM;
    scene.far = f64_of(camera, tags::CAMERA_FAR, DEFAULT_FAR_IN) * INCH_TO_MM;
    // Absent in every 2021-era file, where the engine behaved as if false, so
    // false is what keeps those files framing the way they were saved.
    scene.allow_clipping_in_parallel = bool_of(
        camera,
        tags::CAMERA_ALLOW_CLIPPING_IN_PARALLEL_PROJECTION,
        false,
    );
}

/// What SketchUp's model-extents step and SketchUp's near/far derivation produce for a camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearFar {
    pub near: Mm,
    pub far: Mm,
    /// The eye SketchUp actually draws from. A parallel camera that would clip
    /// is moved backwards instead of clipping, so this is not always the eye
    /// the scene stored.
    pub eye: [Mm; 3],
    /// The engine's own "this view clips" signal: a perspective camera whose
    /// near plane had to be pushed out to keep `far/near` at 3000:1.
    pub clipped: bool,
}

/// Recompute near and far the way SketchUp does, from the model bounds.
///
/// This lives here because it is camera semantics, and because the stored
/// `near`/`far` are a constant 1 and 1000 inches that describe no file in
/// particular - a viewer that used them directly would clip an interior to
/// pieces. SketchUp's model-extents step ` and SketchUp's near/far derivation `,
/// with every constant converted from the engine's inches.
///
/// The one step not taken straight from the disassembly: after a parallel
/// camera's eye is pushed back, both distances are re-measured from the new
/// eye, which shifts them by the same amount because the projection direction
/// has not changed. That follows from the geometry rather than from code I
/// read.
///
/// `bbox` is min xyz then max xyz, as `SceneIndex::bbox` is. Returns `None`
/// only when eye and target coincide, which has no view direction at all.
pub fn derive_near_far(
    eye: [Mm; 3],
    target: [Mm; 3],
    bbox: [Mm; 6],
    perspective: bool,
    allow_clipping_in_parallel: bool,
) -> Option<NearFar> {
    const MARGIN: Mm = 0.5 * INCH_TO_MM;
    const MIN_PARALLEL_NEAR: Mm = 0.1 * INCH_TO_MM;
    const DEGENERATE_FAR: Mm = 0.01 * INCH_TO_MM;
    const FALLBACK_FAR: Mm = 10_000.0 * INCH_TO_MM;
    /// `far/near` is capped at this in perspective. 254 m of far gives 85 mm of
    /// near, which is exactly the near clipping an interior scene suffers.
    /// A ratio, not a length, so it is not an `Mm`.
    const MAX_DEPTH_RATIO: f64 = 3000.0;

    let mut dir = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if !(len > 0.0) || !len.is_finite() {
        return None;
    }
    for c in dir.iter_mut() {
        *c /= len;
    }

    // Project all eight corners of the box onto the view direction.
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for i in 0..8 {
        let corner = [
            bbox[if i & 1 == 0 { 0 } else { 3 }],
            bbox[if i & 2 == 0 { 1 } else { 4 }],
            bbox[if i & 4 == 0 { 2 } else { 5 }],
        ];
        let d = (corner[0] - eye[0]) * dir[0]
            + (corner[1] - eye[1]) * dir[1]
            + (corner[2] - eye[2]) * dir[2];
        lo = lo.min(d);
        hi = hi.max(d);
    }
    if !lo.is_finite() || !hi.is_finite() {
        return None;
    }

    let mut d_near = lo - MARGIN;
    let mut d_far = hi + MARGIN;
    let mut eye = eye;

    // A parallel camera does not clip by default; it steps backwards instead.
    // The cost is that the eye is no longer where the scene said it was, which
    // is why this returns the moved eye rather than hiding it.
    if !perspective && !allow_clipping_in_parallel && d_near < MIN_PARALLEL_NEAR {
        let back = MIN_PARALLEL_NEAR - d_near;
        for k in 0..3 {
            eye[k] -= dir[k] * back;
        }
        d_near += back;
        d_far += back;
    }

    let far_raw = if d_far > DEGENERATE_FAR {
        d_far
    } else {
        FALLBACK_FAR
    };
    let mut near = d_near * 0.9;
    let far = far_raw * 1.1;
    let mut clipped = false;
    if perspective {
        let limit = far / MAX_DEPTH_RATIO;
        if near < limit {
            near = limit;
            clipped = true;
        }
    }
    Some(NearFar {
        near,
        far,
        eye,
        clipped,
    })
}

/// One top-level section's payload, read without holding the rest of the
/// stream.
///
/// The sections this module wants are kilobytes inside a stream that inflates
/// to 145 MB on the largest file here, so the walk skips rather than buffers,
/// exactly as `walk::walk_model` does.
pub fn read_section(
    src: &dyn ByteSource,
    container: &Container,
    tag: u16,
) -> Result<Option<Vec<u8>>> {
    match container.generation {
        Generation::Vff => {
            let entry = container
                .model_entry()
                .ok_or(Error::Malformed("VFF file with no model.dat"))?;
            let at = entry_data_offset(src, entry)?;
            let r = InflateRead::new(src, at, entry.compressed_size, entry.uncompressed_size)?;
            section(RecordStream::new(r), entry.uncompressed_size, tag)
        }
        Generation::Legacy => {
            let (start, end) = container
                .legacy_stream
                .ok_or(Error::Malformed("legacy file with no model stream"))?;
            let r = RangeRead::new(src, start, end);
            section(RecordStream::new(r), end - start, tag)
        }
    }
}

fn section<R: SeqRead>(mut rs: RecordStream<R>, total: u64, want: u16) -> Result<Option<Vec<u8>>> {
    let Some(root) = rs.header(total)? else {
        return Ok(None);
    };
    // A legacy file's first bytes are MFC class records, not this wrapper, so
    // this is also how "wrong generation" comes back as nothing found.
    if root.tag != tags::SKETCH_UP_MODEL {
        return Ok(None);
    }
    let end = rs.enter(&root)?;
    let mut buf = Vec::new();
    while let Some(sec) = rs.header(end)? {
        if sec.tag == want {
            rs.payload(sec.size, &mut buf)?;
            return Ok(Some(buf));
        }
        rs.skip(sec.size)?;
    }
    Ok(None)
}

/// A `(u8 byte-count, little-endian varint)` run, repeated to the end of the
/// payload.
///
/// Proven rather than taken from the TypeScript comment: decoding
/// `ViewPageLayerPIDRefs` this way in `platform.skp` gives 4906, 22945 and
/// 22067, and those are the `Entity` PIDs of the layers named
/// `03_천장 ceiling`, `08_조명 lighting` and `07_안내 signage` in the same
/// file - the three a top-down plan view would hide, on the one page of seven
/// that is a plan view. `stairwell.skp` and `cmp_mine.skp` agree, and the
/// Python reader resolves the same page to the same three names. A byte count
/// of 1 occurs too (`scened.skp`, id 1), so the width really does vary per
/// entry and is not a fixed two bytes.
fn pid_ref_list(record: Record<'_>) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Ok(width) = record.u8_at(at) {
        // Zero would not advance; past eight is not an id the format can hold.
        if width == 0 || width > 8 {
            break;
        }
        let Ok(id) = record.varint_at(at + 1, width as usize) else {
            break;
        };
        // `hidden_layer_ids` is u32. An id that does not fit is skipped rather
        // than truncated, because a truncated id names a different layer.
        if let Ok(id) = u32::try_from(id) {
            out.push(id);
        }
        at += 1 + width as usize;
    }
    out
}

/// A single PID as a bare little-endian varint filling the payload - no length
/// prefix, unlike the lists above.
///
/// Proven by resolution: `ViewPageStylePIDRef` reads `9712` -> 4759 in
/// `platform.skp` and `12f506` -> 455954 in the classroom, and both are the
/// `Entity` PID of a real `SkpStyle` in their own file. Both also equal that
/// file's `SkpStyleManagerSelectedStylePIDRef` byte for byte, which is the
/// same encoding reached by a different path.
fn pid_ref(record: &Record<'_>) -> Option<u32> {
    let width = record.payload.len();
    if width == 0 || width > 8 {
        return None;
    }
    u32::try_from(record.varint_at(0, width).ok()?).ok()
}

/// A child record's payload as text.
///
/// UTF-8 and lossy: all 250 page names, descriptions and tab texts in the
/// corpus are valid UTF-8, and a name that is not is still worth showing as a
/// tab label.
fn text_of(parent: &Record<'_>, tag: u16) -> String {
    match parent.child(tag) {
        Some(r) => String::from_utf8_lossy(r.payload).into_owned(),
        None => String::new(),
    }
}

/// A non-finite scalar is treated as missing. A NaN FOV costs a renderer its
/// whole frame, and no file is improved by passing one along.
fn f64_of(parent: &Record<'_>, tag: u16, fallback: f64) -> f64 {
    match parent.child(tag).map(|r| r.f64_at(0)) {
        Some(Ok(v)) if v.is_finite() => v,
        _ => fallback,
    }
}

fn bool_of(parent: &Record<'_>, tag: u16, fallback: bool) -> bool {
    match parent.child(tag).map(|r| r.bool_at(0)) {
        Some(Ok(v)) => v,
        _ => fallback,
    }
}

fn vec3_of(parent: &Record<'_>, tag: u16) -> Option<[f64; 3]> {
    let v = parent.child(tag)?.vec3_at(0).ok()?;
    if v.iter().all(|c| c.is_finite()) {
        Some(v)
    } else {
        None
    }
}

fn to_mm(p: [f64; 3]) -> [Mm; 3] {
    [p[0] * INCH_TO_MM, p[1] * INCH_TO_MM, p[2] * INCH_TO_MM]
}
