//! The native file format (`.skr`): a zip archive holding `document.json`, which records the
//! format name, a schema version, the camera and the model with every id.

use std::io::{Read, Write};
use std::path::Path;

use kernel::{MeshData, Scene, SceneData};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::camera::{Camera, Projection};
use crate::document::MAX_COORDINATE;

/// The `format` every file records.
pub const FORMAT: &str = "sketchup-alt";
/// The schema version this build writes and the newest it reads. Version 2 added guides,
/// version 3 curves and soft edges, version 4 the camera's up direction and projection, and
/// version 5 materials; older files still open.
pub const SCHEMA_VERSION: u32 = 5;
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
/// never vertical, and they are perspective.
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
}

/// Writes the model and camera to `path`, replacing it only once the whole file is written.
pub(crate) fn write(path: &Path, scene: &Scene, camera: &Camera) -> Result<(), String> {
    let stored = Stored {
        format: FORMAT.into(),
        schema_version: SCHEMA_VERSION,
        camera: StoredCamera {
            eye: camera.eye.to_array(),
            target: camera.target.to_array(),
            fov_y_deg: camera.fov_y.to_degrees(),
            up: Some(camera.up.to_array()),
            projection: camera.projection,
        },
        model: scene.to_data(),
    };
    let json = serde_json::to_vec(&stored).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(ENTRY, options).map_err(|e| e.to_string())?;
    zip.write_all(&json).map_err(|e| e.to_string())?;
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
    let json = read_limited(entry, MAX_DOCUMENT_BYTES)?;
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
    let stored: Stored = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    check_coordinates(&stored.model)?;
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

/// Reads all of `reader`, failing once it holds more than `limit` bytes.
fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "file too large: the document is over {limit} bytes"
        ));
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
        assert_eq!(read_limited(&b"12345"[..], 5).unwrap(), b"12345");
        let err = read_limited(&b"123456"[..], 5).unwrap_err();
        assert!(err.contains("file too large"), "{err}");
    }
}
