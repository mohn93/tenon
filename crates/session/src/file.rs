//! The native file format (`.skr`): a zip archive holding `document.json`, which records the
//! format name, a schema version, the camera and the model with every id, and the texture
//! images as the files they were read from (`textures/m<id>.png` or `.jpg`).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use kernel::{MAX_ID, MAX_TEXTURES, MeshData, Scene, SceneData, TextureImage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::camera::{Camera, Lighting, Projection};
use crate::document::MAX_COORDINATE;
use crate::texture::{MAX_TEXTURE_BYTES, check_total_pixels, decode, dimensions};

/// The `format` every file records.
pub const FORMAT: &str = "sketchup-alt";
/// The newest schema version this build reads, and the one it writes when a face has a texture
/// position. Version 2 added guides, version 3 curves and soft edges, version 4 the camera's up
/// direction and projection, version 5 materials, version 6 material opacity, version 7
/// textures, version 8 material emission and the lighting mode, and version 9 texture
/// positions on faces; older files still open.
pub const SCHEMA_VERSION: u32 = 9;
/// The version written when a material glows or the view is lit by night but no face has a
/// texture position.
pub const LIGHTS_SCHEMA_VERSION: u32 = 8;
/// The version written when a material has a texture but none glows and the view is lit by
/// day.
pub const TEXTURED_SCHEMA_VERSION: u32 = 7;
/// The version written when a material is see-through but none has a texture or glows.
pub const TRANSLUCENT_SCHEMA_VERSION: u32 = 6;
/// The version written when every material is opaque and none has a texture or glows, and no
/// face has a texture position: such a model needs nothing versions 6 to 9 added, so its file
/// is the same as before and older builds open it.
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
    model: SceneData,
}

/// The saved view. Files before version 4 have no `up` or `projection`: their views are
/// never vertical, and they are perspective. `lighting` is written only at night; files before
/// version 8 are lit by day.
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
}

fn is_day(lighting: &Lighting) -> bool {
    *lighting == Lighting::Day
}

/// Writes the model, camera and lighting to `path`, replacing it only once the whole file is
/// written. Fails when the model has given out ids past [`MAX_ID`], which no file can hold.
pub(crate) fn write(
    path: &Path,
    scene: &Scene,
    camera: &Camera,
    lighting: Lighting,
) -> Result<(), String> {
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
    let positioned = std::iter::once(&model.root)
        .chain(model.definitions.iter().map(|d| &d.mesh))
        .any(|m| m.faces.iter().any(|f| f.texture_position.is_some()));
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
        schema_version: if positioned {
            SCHEMA_VERSION
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
        },
        model,
    };
    let json = serde_json::to_vec(&stored).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(ENTRY, options).map_err(|e| e.to_string())?;
    zip.write_all(&json).map_err(|e| e.to_string())?;
    // PNG and JPEG files are compressed already.
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, image) in &images {
        zip.start_file(name.as_str(), stored)
            .map_err(|e| e.to_string())?;
        zip.write_all(&image.encoded).map_err(|e| e.to_string())?;
    }
    let bytes = zip.finish().map_err(|e| e.to_string())?.into_inner();
    replace_file(path, &bytes)
}

/// Writes `bytes` to `path`, replacing the file there only once they are all on disk.
pub(crate) fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    let partial = Path::new(&name);
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::File::create(partial).map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        // The data must be on disk before the rename makes it the file.
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(partial, path).map_err(|e| e.to_string())?;
        sync_parent(path);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(partial);
    }
    result
}

/// Reads a file written by [`write`]: the model and the saved view.
pub(crate) fn read(path: &Path) -> Result<(Scene, StoredCamera), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "not a sketchup-alt file".to_string())?;
    let entry = zip
        .by_name(ENTRY)
        .map_err(|_| "not a sketchup-alt file".to_string())?;
    let json = read_limited(entry, MAX_DOCUMENT_BYTES, "the document")?;
    // Check the header first, so a newer file gets a clear message rather than a field error.
    let header: Value = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
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
    let mut stored: Stored = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    check_coordinates(&stored.model)?;
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
        if t.file == ENTRY || sizes.contains_key(&t.file) {
            continue;
        }
        let bytes = texture_entry(&mut zip, m.id, &t.file)?;
        let (_, width, height) = dimensions(&bytes)
            .map_err(|e| format!("material m{}'s texture {}: {e}", m.id, t.file))?;
        sizes.insert(t.file.clone(), u64::from(width) * u64::from(height));
    }
    check_total_pixels(0, sizes.values().sum())?;
    // A shared entry is decoded once.
    let mut decoded: HashMap<String, Arc<TextureImage>> = HashMap::new();
    for m in &mut stored.model.materials {
        let Some(t) = &mut m.texture else { continue };
        if t.file == ENTRY {
            return Err(format!("material m{}'s texture is not an image", m.id));
        }
        let image = match decoded.get(&t.file) {
            Some(image) => image.clone(),
            None => {
                let bytes = texture_entry(&mut zip, m.id, &t.file)?;
                let image = Arc::new(
                    decode(bytes)
                        .map_err(|e| format!("material m{}'s texture {}: {e}", m.id, t.file))?,
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
    Ok((scene, stored.camera))
}

/// The bytes of texture entry `file` of material `id`, within [`MAX_TEXTURE_BYTES`].
fn texture_entry(
    zip: &mut zip::ZipArchive<std::fs::File>,
    id: u64,
    file: &str,
) -> Result<Vec<u8>, String> {
    let entry = zip
        .by_name(file)
        .map_err(|_| format!("material m{id}'s texture {file} is missing"))?;
    read_limited(entry, MAX_TEXTURE_BYTES, &format!("texture {file}"))
        .map_err(|e| format!("material m{id}'s texture {file}: {e}"))
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

/// Fails, naming the entity, when a vertex, an instance's position or a guide's point is beyond
/// `MAX_COORDINATE`, the limit commands keep to.
fn check_coordinates(model: &SceneData) -> Result<(), String> {
    let meshes = std::iter::once(&model.root).chain(model.definitions.iter().map(|d| &d.mesh));
    let within = |xs: &[f64]| xs.iter().all(|x| x.abs() <= MAX_COORDINATE);
    let beyond = |what: String| format!("{what} is beyond +/-{MAX_COORDINATE} m");
    for mesh in meshes {
        let MeshData {
            vertices,
            instances,
            guides,
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
    fn reading_stops_past_the_limit() {
        assert_eq!(
            read_limited(&b"12345"[..], 5, "the document").unwrap(),
            b"12345"
        );
        let err = read_limited(&b"123456"[..], 5, "the document").unwrap_err();
        assert!(err.contains("file too large"), "{err}");
    }
}
