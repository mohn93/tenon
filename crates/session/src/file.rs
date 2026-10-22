//! The native file format (`.skr`): a zip archive holding `document.json`, which records the
//! format name, a schema version, the camera and the model with every id, and the texture
//! images as the files they were read from (`textures/m<id>.png` or `.jpg`).

use std::collections::HashMap;
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use glam::DVec3;
use kernel::{
    AnchorData, DimensionData, MAX_ID, MAX_TEXTURES, MeshData, NoteData, RadialAnchorData, Scene,
    SceneData, TextureImage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::camera::{Camera, Lighting, Projection};
use crate::document::MAX_COORDINATE;
use crate::texture::{MAX_TEXTURE_BYTES, check_total_pixels, decode, dimensions};
use crate::units::{Units, check_precision};

/// The `format` every file records.
pub const FORMAT: &str = "sketchup-alt";
/// The newest schema version this build reads, and the one it writes when the model has a
/// scene.
/// Version 2 added guides, version 3 curves and soft edges, version 4 the camera's up direction
/// and projection, version 5 materials, version 6 material opacity, version 7 textures,
/// version 8 material emission and the lighting mode, version 9 texture positions on faces,
/// version 10 tags and coloring by tag, version 11 scenes, and version 12 dimensions, notes
/// and display units; older files still open (untagged before version 10, without scenes
/// before version 11, without annotations and in the default units before version 12).
pub const SCHEMA_VERSION: u32 = 12;
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
const ENTRY: &str = "document.json";
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
    units: Option<Units>,
    model: SceneData,
}

/// The saved view. Files before version 4 have no `up` or `projection`: their views are
/// never vertical, and they are perspective. `lighting` is written only at night; files before
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
}

fn is_day(lighting: &Lighting) -> bool {
    *lighting == Lighting::Day
}

/// How the view looks, saved with the camera, and the display units, saved with the model.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewLook {
    pub lighting: Lighting,
    pub color_by_tag: bool,
    pub units: Units,
}

/// Writes the model, camera, lighting and coloring to `path`, replacing it only once the whole
/// file is written. Fails when the model has given out ids past [`MAX_ID`], which no file can
/// hold.
pub(crate) fn write(
    path: &Path,
    scene: &Scene,
    camera: &Camera,
    look: ViewLook,
    access: Access,
) -> Result<(), String> {
    let bytes = write_bytes(scene, camera, look, ENTRY, &[])?;
    replace_file(path, &bytes, access)
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
        units,
    } = look;
    let mut model = scene.to_data();
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
    let needs_lights_schema =
        model.materials.iter().any(|m| m.emission > 0.0) || lighting != Lighting::Day;
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
        schema_version: if annotated {
            SCHEMA_VERSION
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
        },
        units: (units != Units::default()).then_some(units),
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
/// bytes go first to a new hidden file beside it (`.sketchup-alt-<pid>-<n>.partial`), created
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
        let partial = dir.join(format!(".sketchup-alt-{}-{n}.partial", std::process::id()));
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
    let mut zip =
        zip::ZipArchive::new(reader).map_err(|_| "not a sketchup-alt file".to_string())?;
    let entry = zip
        .by_name(document)
        .map_err(|_| "not a sketchup-alt file".to_string())?;
    let json = read_limited(entry, MAX_DOCUMENT_BYTES, "the document")?;
    // Check the header first, so a newer file gets a clear message rather than a field error.
    let header: Value = serde_json::from_slice(&json).map_err(json_error)?;
    if header["format"] != FORMAT {
        return Err("not a sketchup-alt file".into());
    }
    let Some(version) = header["schema_version"].as_u64().filter(|v| *v >= 1) else {
        return Err("the file has no valid schema version (a whole number from 1)".into());
    };
    if version > u64::from(SCHEMA_VERSION) {
        return Err(format!(
            "the file uses schema version {version}, newer than this app's {SCHEMA_VERSION}"
        ));
    }
    let mut stored: Stored = serde_json::from_slice(&json).map_err(json_error)?;
    check_coordinates(&stored.model)?;
    let units = stored.units.unwrap_or_default();
    check_precision(units.precision).map_err(|e| format!("the display units: {e}"))?;
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
            color_by_tag: false,
        };
        let page = kernel::Page {
            name: "Far".into(),
            view: Some(far),
            tags: None,
        };
        scene.add_page(page).unwrap();
        let dir = std::env::temp_dir().join(format!("session-file-far-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("far.skr");
        let camera = Camera::new(glam::DVec2::new(800.0, 600.0));
        let look = ViewLook {
            lighting: Lighting::Day,
            color_by_tag: false,
            units: Units::default(),
        };
        let err = write(&path, &scene, &camera, look, Access::Default).unwrap_err();
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
