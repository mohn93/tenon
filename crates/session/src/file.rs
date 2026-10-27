//! The native file format (`.tnn`): a zip archive holding `document.json`, which records the
//! format name, a schema version, the camera and the model with every id, and the texture
//! images as the files they were read from (`textures/m<id>.png` or `.jpg`).

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use glam::DVec3;
use kernel::{
    AnchorData, DimensionData, MAX_ID, MAX_TEXTURES, MeshData, NoteData, RadialAnchorData, Scene,
    SceneData, Shadows, StyleId, TextureImage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::camera::{Camera, Lighting, Projection};
use crate::document::{MAX_COORDINATE, SectionDisplay, SectionFillColor};
use crate::texture::{MAX_TEXTURE_BYTES, check_total_pixels, decode, dimensions};
use crate::units::{StoredUnits, Units};

/// The `format` every file records. It keeps the name the project had before it became Tenon,
/// so files stay readable by every build.
pub const FORMAT: &str = "sketchup-alt";
/// The extension of a model file.
pub const EXTENSION: &str = "tnn";
/// The extension model files had before the project was renamed Tenon; such files still open.
pub const LEGACY_EXTENSION: &str = "skr";

/// Whether `ext` (no dot, any case) is the extension of a model file, `tnn` or the legacy `skr`.
pub fn is_model_extension(ext: &str) -> bool {
    ext.eq_ignore_ascii_case(EXTENSION) || ext.eq_ignore_ascii_case(LEGACY_EXTENSION)
}

/// The newest schema version this build reads.
/// Version 2 added guides, version 3 curves and soft edges, version 4 the camera's up direction
/// and projection, version 5 materials, version 6 material opacity, version 7 textures,
/// version 8 material emission and the lighting mode, version 9 texture positions on faces,
/// version 10 tags and coloring by tag, version 11 scenes, version 12 dimensions, notes
/// and display units, version 13 section planes and the section display settings, and
/// version 14 hidden entities and the x-ray and hidden geometry view flags, version 15 the
/// model axes, version 16 the location and sun shadow settings, version 17 decimal feet
/// and the length and angle snapping settings of the display units, and version 18 styles
/// and the style a view chooses, and version 19 material metalness and roughness, and version
/// 20 studio lighting, and version 21 ambient occlusion in a style, and version 22 the material
/// section fill and instances left whole by section planes, version 23 the instance positions a
/// scene stores, and version 24 the scene transition setting, and version 25 joints, and version
/// 26 material density, and version 27 couplings between joints, and version 28 cam
/// followers, and version 29 the joint values a scene stores; older files still open
/// (untagged before version 10, without scenes before version 11, without annotations and in
/// the default units before version 12, without section planes and with the default section
/// display before version 13).
pub const SCHEMA_VERSION: u32 = 29;
/// The version written when a scene stores joint values.
pub const SCENE_JOINTS_SCHEMA_VERSION: u32 = 29;
/// The version written when the model has a cam follower.
pub const CAM_FOLLOWERS_SCHEMA_VERSION: u32 = 28;
/// The version written when the model has a coupling between joints.
pub const COUPLINGS_SCHEMA_VERSION: u32 = 27;
/// The version written when a material has a density.
pub const DENSITY_SCHEMA_VERSION: u32 = 26;
/// The version written when the model has a joint.
pub const JOINTS_SCHEMA_VERSION: u32 = 25;
/// The version written when the scene transition is not the default.
pub const TRANSITION_SCHEMA_VERSION: u32 = 24;
/// The version written when a scene stores instance positions.
pub const SCENE_POSITIONS_SCHEMA_VERSION: u32 = 23;
/// The version written when the section fill takes the material's color or an instance is not
/// cut by section planes.
pub const SECTION_CUT_SCHEMA_VERSION: u32 = 22;
/// The version written when a style has ambient occlusion on or with other than the default
/// radius or strength.
pub const AMBIENT_OCCLUSION_SCHEMA_VERSION: u32 = 21;
/// The version written when the view or a scene's view is lit by the studio lights.
pub const STUDIO_SCHEMA_VERSION: u32 = 20;
/// The version written when a material has a metalness above 0 or a roughness below 1.
pub const FINISH_SCHEMA_VERSION: u32 = 19;
/// The version written when the model has a style or the camera or a scene's view chooses
/// one.
pub const STYLES_SCHEMA_VERSION: u32 = 18;
/// The version written when the display units have the decimal feet or snapping settings but
/// the model has no style.
pub const SNAP_SCHEMA_VERSION: u32 = 17;
/// The version written when the location or the shadow settings are not the defaults but the
/// display units are readable by version 16.
pub const SHADOWS_SCHEMA_VERSION: u32 = 16;
/// The version written when the model axes are not the world axes but the location and the
/// shadow settings are the defaults.
pub const AXES_SCHEMA_VERSION: u32 = 15;
/// The version written when an edge, face, instance or guide is hidden, or the camera or a
/// scene's view has `x_ray` or `hidden_geometry` on, and the model axes are the world axes.
pub const HIDDEN_SCHEMA_VERSION: u32 = 14;
/// The version written when the model has a section plane or non-default section display but
/// nothing is hidden.
pub const SECTIONS_SCHEMA_VERSION: u32 = 13;
/// The version written when the model has a dimension or a note, or the non-default display
/// units, but no section plane and the default section display.
pub const ANNOTATIONS_SCHEMA_VERSION: u32 = 12;
/// The version written when the model has a scene but no dimension or note and the default
/// display units.
pub const SCENES_SCHEMA_VERSION: u32 = 11;
/// The version written when the model has a tag or the view colors faces by tag, but no
/// scene.
pub const TAGS_SCHEMA_VERSION: u32 = 10;
/// The version written when a face has a texture position but the model has no tag.
pub const TEXTURE_POSITION_SCHEMA_VERSION: u32 = 9;
/// The version written when a material glows or the view is lit by night but no face has a
/// texture position and the model has no tag.
pub const LIGHTS_SCHEMA_VERSION: u32 = 8;
/// The version written when a material has a texture but none glows and the view is lit by
/// day.
pub const TEXTURED_SCHEMA_VERSION: u32 = 7;
/// The version written when a material is see-through but none has a texture or glows.
pub const TRANSLUCENT_SCHEMA_VERSION: u32 = 6;
/// The version written when every material is opaque and none has a texture or glows, no face
/// has a texture position and the model has no tag: such a model needs nothing versions 6 to
/// 10 added, so its file is the same as before and older builds open it.
pub const OPAQUE_SCHEMA_VERSION: u32 = 5;
/// The archive entry holding the document.
pub(crate) const ENTRY: &str = "document.json";
/// The largest `document.json` this build reads, uncompressed.
const MAX_DOCUMENT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    format: String,
    schema_version: u32,
    camera: StoredCamera,
    /// The display units; absent for the default (meters, 3 decimals) and before version 12.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    units: Option<StoredUnits>,
    model: SceneData,
}

/// The saved view. Files before version 4 have no `up` or `projection`: their views are
/// never vertical, and they are perspective. `lighting` is written only at night or in the studio; files before
/// version 8 are lit by day. `color_by_tag` is written only when on; files before version 10
/// color by material.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredCamera {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub fov_y_deg: f64,
    #[serde(default)]
    pub up: Option<[f64; 3]>,
    #[serde(default)]
    pub projection: Projection,
    #[serde(default, skip_serializing_if = "is_day")]
    pub lighting: Lighting,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub color_by_tag: bool,
    /// Faces drawn as glass and hidden entities drawn ghosted, written only when on (they
    /// need version 14).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub x_ray: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden_geometry: bool,
    /// The sun shadow settings, written only when not the defaults (they need version 16).
    #[serde(default, skip_serializing_if = "Shadows::is_default")]
    pub shadows: Shadows,
    /// The style the view draws with, written only when one is chosen (it needs version 18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleId>,
    /// The section display settings, written only when off; files before version 13 show
    /// planes, apply cuts and fill them.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub section_planes: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub section_cuts: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub section_fill: bool,
    /// The fill color, written only when not the dark default (it needs version 22).
    #[serde(default, skip_serializing_if = "is_dark")]
    pub section_fill_color: SectionFillColor,
}

fn is_dark(c: &SectionFillColor) -> bool {
    *c == SectionFillColor::Dark
}

fn yes() -> bool {
    true
}

fn is_true(on: &bool) -> bool {
    *on
}

fn is_day(lighting: &Lighting) -> bool {
    *lighting == Lighting::Day
}

/// How the view looks, saved with the camera, and the display units, saved with the model.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewLook {
    pub lighting: Lighting,
    pub color_by_tag: bool,
    pub x_ray: bool,
    pub hidden_geometry: bool,
    pub shadows: Shadows,
    pub style: Option<StyleId>,
    pub section: SectionDisplay,
    pub units: Units,
}

/// Writes the model, camera, lighting and coloring to `path` (with the `extra` archive entries), replacing it only once the whole
/// file is written. Fails when the model has given out ids past [`MAX_ID`], which no file can
/// hold.
pub(crate) fn write(
    path: &Path,
    scene: &Scene,
    camera: &Camera,
    look: ViewLook,
    access: Access,
    extra: &[(&str, &[u8])],
) -> Result<(), String> {
    let bytes = encode(scene, camera, look, extra)?;
    replace_file(path, &bytes, access)
}

/// The bytes [`write`] puts in a file: the `.tnn` archive of the model, camera, lighting and
/// coloring with the `extra` entries. What the web's save hands out as a download.
pub(crate) fn encode(
    scene: &Scene,
    camera: &Camera,
    look: ViewLook,
    extra: &[(&str, &[u8])],
) -> Result<Vec<u8>, String> {
    write_bytes(scene, camera, look, ENTRY, extra)
}

/// The most entries a `.tnn` file's archive may hold for [`read_entry`] to look into it.
const MAX_LISTED_ENTRIES: usize = 10_000;

/// The entry `name` of the `.tnn` file at `path`, or None when the archive has no such entry
/// or it is over `limit` bytes. Reads the archive's directory and that entry only; fails when
/// the file is not an archive holding a document, or holds more than 10,000 entries.
pub(crate) fn read_entry(path: &Path, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "not a Tenon file".to_string())?;
    if zip.len() > MAX_LISTED_ENTRIES {
        return Err(format!(
            "the file holds more than {MAX_LISTED_ENTRIES} entries"
        ));
    }
    if zip.by_name(ENTRY).is_err() {
        return Err("not a Tenon file".into());
    }
    match zip.by_name(name) {
        Ok(entry) if entry.size() <= limit => Ok(read_limited(entry, limit, name).ok()),
        _ => Ok(None),
    }
}

/// The archive [`write`] writes, with the document under the name `entry` and the `extra`
/// entries (name, bytes) after the textures.
pub(crate) fn write_bytes(
    scene: &Scene,
    camera: &Camera,
    look: ViewLook,
    entry: &str,
    extra: &[(&str, &[u8])],
) -> Result<Vec<u8>, String> {
    let ViewLook {
        lighting,
        color_by_tag,
        x_ray,
        hidden_geometry,
        shadows,
        style,
        section,
        units,
    } = look;
    let mut model = scene.to_data();
    // A view can only choose a style the written model has (a clip or library item of part of
    // a model has none), so the others are dropped and such files are written as before.
    let known: BTreeSet<u64> = model.styles.iter().map(|s| s.id).collect();
    let style = style.filter(|s| known.contains(&s.0));
    for v in model.pages.iter_mut().filter_map(|p| p.view.as_mut()) {
        if v.style.is_some_and(|s| !known.contains(&s.0)) {
            v.style = None;
        }
    }
    // Materials sharing an image share its entry, so the file reads back within the texture
    // pixel budget, which counts each entry once.
    let mut entries: HashMap<*const TextureImage, String> = HashMap::new();
    for t in model
        .materials
        .iter_mut()
        .filter_map(|m| m.texture.as_mut())
    {
        if let Some(image) = &t.image {
            t.file = entries
                .entry(Arc::as_ptr(image))
                .or_insert_with(|| t.file.clone())
                .clone();
        }
    }
    if model.next_id > MAX_ID {
        return Err(format!(
            "the model has used ids past {MAX_ID}, more than a file can hold"
        ));
    }
    // What opening checks, so no file is written that will not open.
    check_coordinates(&model)?;
    let positioned = std::iter::once(&model.root)
        .chain(model.definitions.iter().map(|d| &d.mesh))
        .any(|m| m.faces.iter().any(|f| f.texture_position.is_some()));
    // An entity on a tag needs the tag, so tags alone decide.
    let tagged = !model.tags.is_empty() || color_by_tag;
    let has_scenes = !model.pages.is_empty();
    let annotated = std::iter::once(&model.root)
        .chain(model.definitions.iter().map(|d| &d.mesh))
        .any(|m| !m.dimensions.is_empty() || !m.notes.is_empty())
        || units != Units::default();
    let sectioned = std::iter::once(&model.root)
        .chain(model.definitions.iter().map(|d| &d.mesh))
        .any(|m| !m.sections.is_empty())
        || section != SectionDisplay::default();
    // Every hidden flag lives in the mesh data, and the view flags in the camera and scenes,
    // so a model with none of them writes what it always did.
    let hidden = x_ray
        || hidden_geometry
        || model
            .pages
            .iter()
            .any(|p| p.view.is_some_and(|v| v.x_ray || v.hidden_geometry))
        || std::iter::once(&model.root)
            .chain(model.definitions.iter().map(|d| &d.mesh))
            .any(|m| {
                m.edges.iter().any(|e| e.hidden)
                    || m.faces.iter().any(|f| f.hidden)
                    || m.instances.iter().any(|i| i.hidden)
                    || m.guides.iter().any(|g| g.hidden)
            });
    // The location lives in the model data, the settings in the camera and scenes.
    let shaded = model.location.is_some()
        || !shadows.is_default()
        || model
            .pages
            .iter()
            .any(|p| p.view.is_some_and(|v| !v.shadows.is_default()));
    let moving_scenes = model.pages.iter().any(|p| p.positions.is_some());
    let jointed_scenes = model.pages.iter().any(|p| p.joints.is_some());
    let styled = !model.styles.is_empty()
        || style.is_some()
        || model
            .pages
            .iter()
            .any(|p| p.view.is_some_and(|v| v.style.is_some()));
    let needs_lights_schema =
        model.materials.iter().any(|m| m.emission > 0.0) || lighting == Lighting::Night;
    let studio = lighting == Lighting::Studio
        || model.pages.iter().any(|p| p.view.is_some_and(|v| v.studio));
    let occluded = model
        .styles
        .iter()
        .any(|s| !s.settings.ambient_occlusion.is_default());
    let section_cut = section.fill_color != SectionFillColor::Dark
        || std::iter::once(&model.root)
            .chain(model.definitions.iter().map(|d| &d.mesh))
            .any(|m| m.instances.iter().any(|i| !i.section_cut));
    let finished = model
        .materials
        .iter()
        .any(|m| m.metalness > 0.0 || m.roughness < 1.0);
    let weighed = model.materials.iter().any(|m| m.density.is_some());
    let textured = model.materials.iter().any(|m| m.texture.is_some());
    let translucent = model.materials.iter().any(|m| m.opacity < 1.0);
    let mut images: Vec<(String, Arc<TextureImage>)> = model
        .materials
        .iter()
        .filter_map(|m| {
            let t = m.texture.as_ref()?;
            Some((t.file.clone(), t.image.clone()?))
        })
        .collect();
    images.sort_by(|a, b| a.0.cmp(&b.0));
    images.dedup_by(|a, b| a.0 == b.0);
    let stored = Stored {
        format: FORMAT.into(),
        schema_version: if jointed_scenes {
            SCENE_JOINTS_SCHEMA_VERSION
        } else if !model.cam_followers.is_empty() {
            CAM_FOLLOWERS_SCHEMA_VERSION
        } else if !model.couplings.is_empty() {
            COUPLINGS_SCHEMA_VERSION
        } else if weighed {
            DENSITY_SCHEMA_VERSION
        } else if !model.joints.is_empty() {
            JOINTS_SCHEMA_VERSION
        } else if model.transition.is_some() {
            TRANSITION_SCHEMA_VERSION
        } else if moving_scenes {
            SCENE_POSITIONS_SCHEMA_VERSION
        } else if section_cut {
            SECTION_CUT_SCHEMA_VERSION
        } else if occluded {
            AMBIENT_OCCLUSION_SCHEMA_VERSION
        } else if studio {
            STUDIO_SCHEMA_VERSION
        } else if finished {
            FINISH_SCHEMA_VERSION
        } else if styled {
            STYLES_SCHEMA_VERSION
        } else if units.needs_snap_schema() {
            SNAP_SCHEMA_VERSION
        } else if shaded {
            SHADOWS_SCHEMA_VERSION
        } else if model.axes.is_some() {
            AXES_SCHEMA_VERSION
        } else if hidden {
            HIDDEN_SCHEMA_VERSION
        } else if sectioned {
            SECTIONS_SCHEMA_VERSION
        } else if annotated {
            ANNOTATIONS_SCHEMA_VERSION
        } else if has_scenes {
            SCENES_SCHEMA_VERSION
        } else if tagged {
            TAGS_SCHEMA_VERSION
        } else if positioned {
            TEXTURE_POSITION_SCHEMA_VERSION
        } else if needs_lights_schema {
            LIGHTS_SCHEMA_VERSION
        } else if textured {
            TEXTURED_SCHEMA_VERSION
        } else if translucent {
            TRANSLUCENT_SCHEMA_VERSION
        } else {
            OPAQUE_SCHEMA_VERSION
        },
        camera: StoredCamera {
            eye: camera.eye.to_array(),
            target: camera.target.to_array(),
            fov_y_deg: camera.fov_y.to_degrees(),
            up: Some(camera.up.to_array()),
            projection: camera.projection,
            lighting,
            color_by_tag,
            x_ray,
            hidden_geometry,
            shadows,
            style,
            section_planes: section.planes,
            section_cuts: section.cuts,
            section_fill: section.fill,
            section_fill_color: section.fill_color,
        },
        units: (units != Units::default()).then(|| StoredUnits::from(units)),
        model,
    };
    let json = serde_json::to_vec(&stored).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(entry, options).map_err(|e| e.to_string())?;
    zip.write_all(&json).map_err(|e| e.to_string())?;
    // PNG and JPEG files are compressed already.
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, image) in &images {
        zip.start_file(name.as_str(), stored)
            .map_err(|e| e.to_string())?;
        zip.write_all(&image.encoded).map_err(|e| e.to_string())?;
    }
    for (name, bytes) in extra {
        zip.start_file(*name, options).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Who may read a file written: as the user's other files (the umask or the folder's
/// inherited permissions decide), or the user only (0600; on Windows an owner-only DACL).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Default,
    Private,
}

/// Writes `bytes` to `path`, replacing the file there only once they are all on disk. The
/// bytes go first to a new hidden file beside it (`.tenon-<pid>-<n>.partial`), created
/// only where nothing is, so they never go through a link and no file of the user's is
/// touched; it is renamed over `path` once written, or removed if the write fails.
pub(crate) fn replace_file(path: &Path, bytes: &[u8], access: Access) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let (partial, mut file) = loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let partial = dir.join(format!(".tenon-{}-{n}.partial", std::process::id()));
        match create_new(&partial, access) {
            Ok(file) => break (partial, file),
            // Another file (a process's that reused this id, say) has the name.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    };
    let result = (|| -> Result<(), String> {
        file.write_all(bytes).map_err(|e| e.to_string())?;
        // The data must be on disk before the rename makes it the file.
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&partial, path).map_err(|e| e.to_string())?;
        sync_parent(path);
        Ok(())
    })();
    if result.is_err() {
        // The file this call created, and nothing else.
        let _ = std::fs::remove_file(&partial);
    }
    result
}

/// [`replace_file`] for a file only the user may read (0600; on Windows an owner-only DACL):
/// the notes the window keeps beside its recovery copies.
pub fn replace_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    replace_file(path, bytes, Access::Private)
}

/// Creates the file `path` for writing, failing when anything is there. `Access::Private`:
/// mode 0600 on Unix, and on Windows a DACL for the current user only.
fn create_new(path: &Path, access: Access) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    if access == Access::Private {
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        #[cfg(windows)]
        return crate::windows::create_private_file(path);
    }
    options.open(path)
}

/// A document that does not parse, said without quoting it: serde's own messages quote the
/// file's text (a string of the wrong kind, an unknown field's name).
fn json_error(e: serde_json::Error) -> String {
    use serde_json::error::Category;
    let (line, column) = (e.line(), e.column());
    match e.classify() {
        Category::Eof | Category::Io => "the document ends early".into(),
        Category::Syntax => {
            format!("the document is not valid JSON (line {line}, column {column})")
        }
        Category::Data => format!(
            "the document has a value of the wrong kind or an unknown name (line {line}, column {column})"
        ),
    }
}

/// What [`read`] reads: the model, the saved view and the display units.
pub(crate) struct Opened {
    pub scene: Scene,
    pub camera: StoredCamera,
    pub units: Units,
}

/// Reads a file written by [`write`]: the model, the saved view and the display units.
pub(crate) fn read(path: &Path) -> Result<Opened, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let (opened, _) = read_archive(file, ENTRY, &[])?;
    Ok(opened)
}

/// Reads the bytes of a file written by [`write`], with the same checks as [`read`].
pub(crate) fn read_slice(bytes: &[u8]) -> Result<Opened, String> {
    let (opened, _) = read_archive(std::io::Cursor::new(bytes), ENTRY, &[])?;
    Ok(opened)
}

/// Reads an archive written by [`write_bytes`] with the document under `entry`, with the same
/// checks as [`read`]; also returns the bytes of the `extra` entries (name, most bytes), in
/// order.
pub(crate) fn read_bytes(
    bytes: &[u8],
    entry: &str,
    extra: &[(&str, u64)],
) -> Result<(Scene, StoredCamera, Vec<Vec<u8>>), String> {
    let (opened, extras) = read_archive(std::io::Cursor::new(bytes), entry, extra)?;
    Ok((opened.scene, opened.camera, extras))
}

fn read_archive<R: Read + Seek>(
    reader: R,
    document: &str,
    extra: &[(&str, u64)],
) -> Result<(Opened, Vec<Vec<u8>>), String> {
    let mut zip = zip::ZipArchive::new(reader).map_err(|_| "not a Tenon file".to_string())?;
    let entry = zip
        .by_name(document)
        .map_err(|_| "not a Tenon file".to_string())?;
    let json = read_limited(entry, MAX_DOCUMENT_BYTES, "the document")?;
    // Check the header first, so a newer file gets a clear message rather than a field error.
    // Only its two fields are kept (the rest is skipped, not built); a document that is not
    // an object they can be read from that way is read whole, which finds them as before or
    // says why it cannot.
    #[derive(Deserialize)]
    struct Header {
        #[serde(default)]
        format: Value,
        #[serde(default)]
        schema_version: Value,
    }
    let object = json.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{');
    let header = match serde_json::from_slice::<Header>(&json)
        .ok()
        .filter(|_| object)
    {
        Some(h) => h,
        None => {
            let whole: Value = serde_json::from_slice(&json).map_err(json_error)?;
            Header {
                format: whole["format"].clone(),
                schema_version: whole["schema_version"].clone(),
            }
        }
    };
    if header.format != FORMAT {
        return Err("not a Tenon file".into());
    }
    let Some(version) = header.schema_version.as_u64().filter(|v| *v >= 1) else {
        return Err("the file has no valid schema version (a whole number from 1)".into());
    };
    if version > u64::from(SCHEMA_VERSION) {
        return Err(format!(
            "the file uses schema version {version}, newer than this app's {SCHEMA_VERSION}"
        ));
    }
    let mut stored: Stored = serde_json::from_slice(&json).map_err(json_error)?;
    check_coordinates(&stored.model)?;
    stored
        .camera
        .shadows
        .check()
        .map_err(|e| format!("the camera view: {e}"))?;
    let units = stored.units.map(Units::from).unwrap_or_default();
    units
        .check()
        .map_err(|e| format!("the display units: {e}"))?;
    // Refuse before decoding anything: each decode can take far more memory than its entry.
    if stored
        .model
        .materials
        .iter()
        .filter(|m| m.texture.is_some())
        .count()
        > MAX_TEXTURES
    {
        return Err(format!("more than {MAX_TEXTURES} materials have a texture"));
    }
    // Sum the images' sizes from their headers before decoding any, against the total budget.
    let mut sizes: HashMap<String, u64> = HashMap::new();
    for m in &stored.model.materials {
        let Some(t) = &m.texture else { continue };
        if t.file == document || sizes.contains_key(&t.file) {
            continue;
        }
        let bytes = texture_entry(&mut zip, m.id, &t.file)?;
        let (_, width, height) =
            dimensions(&bytes).map_err(|e| format!("material m{}'s texture: {e}", m.id))?;
        sizes.insert(t.file.clone(), u64::from(width) * u64::from(height));
    }
    check_total_pixels(0, sizes.values().sum())?;
    // A shared entry is decoded once.
    let mut decoded: HashMap<String, Arc<TextureImage>> = HashMap::new();
    for m in &mut stored.model.materials {
        let Some(t) = &mut m.texture else { continue };
        if t.file == document {
            return Err(format!("material m{}'s texture is not an image", m.id));
        }
        let image = match decoded.get(&t.file) {
            Some(image) => image.clone(),
            None => {
                let bytes = texture_entry(&mut zip, m.id, &t.file)?;
                let image = Arc::new(
                    decode(bytes).map_err(|e| format!("material m{}'s texture: {e}", m.id))?,
                );
                decoded.insert(t.file.clone(), image.clone());
                image
            }
        };
        t.image = Some(image);
    }
    let scene = Scene::from_data(&stored.model)?;
    let c = &stored.camera;
    if let Some(s) = c.style
        && scene.style(s).is_none()
    {
        return Err(format!("the camera view chooses a missing style {s}"));
    }
    if !c
        .eye
        .iter()
        .chain(&c.target)
        .chain([&c.fov_y_deg])
        .chain(c.up.iter().flatten())
        .all(|x| x.is_finite())
    {
        return Err("the camera has a number that is not finite".into());
    }
    let mut extras = Vec::new();
    for (name, limit) in extra {
        let entry = zip
            .by_name(name)
            .map_err(|_| format!("the archive has no {name}"))?;
        extras.push(read_limited(entry, *limit, name)?);
    }
    let opened = Opened {
        scene,
        camera: stored.camera,
        units,
    };
    Ok((opened, extras))
}

/// The bytes of texture entry `file` of material `id`, within [`MAX_TEXTURE_BYTES`].
fn texture_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    id: u64,
    file: &str,
) -> Result<Vec<u8>, String> {
    let entry = zip
        .by_name(file)
        .map_err(|_| format!("material m{id}'s texture is missing"))?;
    read_limited(entry, MAX_TEXTURE_BYTES, "the texture")
        .map_err(|e| format!("material m{id}'s texture: {e}"))
}

/// Reads all of `reader` (`what` names it in the error), failing once it holds more than
/// `limit` bytes.
fn read_limited(reader: impl Read, limit: u64, what: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("file too large: {what} is over {limit} bytes"));
    }
    Ok(bytes)
}

/// Fails, naming the entity, when a vertex, an instance's position, a guide's point, a
/// section plane's point or an annotation's numbers are beyond `MAX_COORDINATE`, the limit commands keep to (a dimension's
/// offset and a radial dimension's leader reach from one point within it to another, so up
/// to twice the limit; a fixed circle's center plus or minus its radius along each axis is
/// within it).
fn check_coordinates(model: &SceneData) -> Result<(), String> {
    let meshes = std::iter::once(&model.root).chain(model.definitions.iter().map(|d| &d.mesh));
    let within = |xs: &[f64]| xs.iter().all(|x| x.abs() <= MAX_COORDINATE);
    let beyond = |what: String| format!("{what} is beyond +/-{MAX_COORDINATE} m");
    if let Some(a) = &model.axes
        && !within(&a.origin)
    {
        return Err(beyond("the axes' origin".to_string()));
    }
    for mesh in meshes {
        let MeshData {
            vertices,
            instances,
            guides,
            dimensions,
            notes,
            sections,
            ..
        } = mesh;
        if let Some(v) = vertices.iter().find(|v| !within(&v.position)) {
            return Err(beyond(format!("vertex v{}", v.id)));
        }
        if let Some(i) = instances.iter().find(|i| !within(&i.transform[9..])) {
            return Err(beyond(format!("instance i{}'s position", i.id)));
        }
        if let Some(g) = guides.iter().find(|g| !within(&g.point)) {
            return Err(beyond(format!("guide g{}", g.id)));
        }
        if let Some(p) = sections.iter().find(|p| !within(&p.point)) {
            return Err(beyond(format!("section plane s{}", p.id)));
        }
        let anchor = |a: &AnchorData| match a {
            AnchorData::Vertex(_) => true,
            AnchorData::Point(p) => within(p),
        };
        let reach = |xs: &[f64]| xs.iter().all(|x| x.abs() <= 2.0 * MAX_COORDINATE);
        for d in dimensions {
            let (id, ok) = match d {
                DimensionData::Linear {
                    id, a, b, offset, ..
                } => (id, anchor(a) && anchor(b) && reach(offset)),
                DimensionData::Radial {
                    id, curve, leader, ..
                } => {
                    let center = match curve {
                        RadialAnchorData::Curve(_) => true,
                        // The circle reaches `radius * sqrt(1 - n^2)` from the center along
                        // each axis (see `Circle::extent`).
                        RadialAnchorData::Fixed(c) => {
                            let n = DVec3::from(c.normal).try_normalize().unwrap_or(DVec3::Z);
                            c.center.iter().zip(n.to_array()).all(|(x, n)| {
                                x.abs() + c.radius.abs() * (1.0 - n * n).max(0.0).sqrt()
                                    <= MAX_COORDINATE
                            })
                        }
                    };
                    (id, center && reach(leader))
                }
            };
            if !ok {
                return Err(beyond(format!("dimension r{id}")));
            }
        }
        for n in notes {
            if let NoteData::Leader {
                id,
                anchor: a,
                position,
                ..
            } = n
                && !(anchor(a) && within(position))
            {
                return Err(beyond(format!("note n{id}")));
            }
        }
    }
    for p in &model.pages {
        if let Some(v) = &p.view
            && !(within(&v.eye) && within(&v.target))
        {
            return Err(beyond(format!("scene p{}'s view", p.id)));
        }
    }
    Ok(())
}

/// Makes a rename in `path`'s directory durable, where the system allows it.
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if let Ok(dir) = std::fs::File::open(dir) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_view_beyond_the_limit_is_not_written() {
        let mut scene = Scene::new();
        let far = kernel::PageView {
            eye: [2.0 * MAX_COORDINATE, 0.0, 0.0],
            target: [0.0; 3],
            up: [0.0, 0.0, 1.0],
            fov_y: 35f64.to_radians(),
            parallel: false,
            night: false,
            studio: false,
            color_by_tag: false,
            x_ray: false,
            hidden_geometry: false,
            shadows: kernel::Shadows::default(),
            style: None,
        };
        let page = kernel::Page {
            name: "Far".into(),
            view: Some(far),
            tags: None,
            positions: None,
            joints: None,
        };
        scene.add_page(page).unwrap();
        let dir = std::env::temp_dir().join(format!("session-file-far-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("far.tnn");
        let camera = Camera::new(glam::DVec2::new(800.0, 600.0));
        let look = ViewLook {
            lighting: Lighting::Day,
            color_by_tag: false,
            x_ray: false,
            hidden_geometry: false,
            shadows: kernel::Shadows::default(),
            style: None,
            section: SectionDisplay::default(),
            units: Units::default(),
        };
        let err = write(&path, &scene, &camera, look, Access::Default, &[]).unwrap_err();
        assert!(err.contains("scene p") && err.contains("beyond"), "{err}");
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reading_stops_past_the_limit() {
        assert_eq!(
            read_limited(&b"12345"[..], 5, "the document").unwrap(),
            b"12345"
        );
        let err = read_limited(&b"123456"[..], 5, "the document").unwrap_err();
        assert!(err.contains("file too large"), "{err}");
    }
}
