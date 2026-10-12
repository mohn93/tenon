//! The native file format (`.skr`): a zip archive holding `document.json`, which records the
//! format name, a schema version, the camera and the model with every id.

use std::io::{Read, Write};
use std::path::Path;

use glam::DVec3;
use kernel::{Scene, SceneData};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::camera::Camera;

/// The `format` every file records.
pub const FORMAT: &str = "sketchup-alt";
/// The schema version this build writes and the newest it reads.
pub const SCHEMA_VERSION: u32 = 1;
/// The archive entry holding the document.
const ENTRY: &str = "document.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    format: String,
    schema_version: u32,
    camera: StoredCamera,
    model: SceneData,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCamera {
    eye: [f64; 3],
    target: [f64; 3],
    fov_y_deg: f64,
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
        },
        model: scene.to_data(),
    };
    let json = serde_json::to_vec(&stored).map_err(|e| e.to_string())?;
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    let partial = Path::new(&name);
    let result = (|| -> Result<(), String> {
        let file = std::fs::File::create(partial).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(ENTRY, options).map_err(|e| e.to_string())?;
        zip.write_all(&json).map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?;
        std::fs::rename(partial, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(partial);
    }
    result
}

/// Reads a file written by [`write`]: the model and the camera's eye, target and field of view
/// (degrees).
pub(crate) fn read(path: &Path) -> Result<(Scene, [DVec3; 2], f64), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "not a sketchup-alt file".to_string())?;
    let mut json = Vec::new();
    zip.by_name(ENTRY)
        .map_err(|_| "not a sketchup-alt file".to_string())?
        .read_to_end(&mut json)
        .map_err(|e| e.to_string())?;
    // Check the header first, so a newer file gets a clear message rather than a field error.
    let header: Value = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    if header["format"] != FORMAT {
        return Err("not a sketchup-alt file".into());
    }
    let version = header["schema_version"].as_u64().unwrap_or(0);
    if version > u64::from(SCHEMA_VERSION) {
        return Err(format!(
            "the file uses schema version {version}, newer than this app's {SCHEMA_VERSION}"
        ));
    }
    let stored: Stored = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    let scene = Scene::from_data(&stored.model)?;
    let c = &stored.camera;
    let camera = [DVec3::from(c.eye), DVec3::from(c.target)];
    if !c
        .eye
        .iter()
        .chain(&c.target)
        .chain([&c.fov_y_deg])
        .all(|x| x.is_finite())
    {
        return Err("the camera has a number that is not finite".into());
    }
    Ok((scene, camera, c.fov_y_deg))
}
