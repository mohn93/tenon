//! Exporting the model's faces as triangles: OBJ, STL, and glTF (binary `.glb` or `.gltf`
//! with its buffer embedded). Units are meters. OBJ and glTF are Y-up (x, z, -y); STL keeps
//! the model's Z-up coordinates.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use base64::Engine;
use glam::DVec3;
use kernel::Triangle;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// An export file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Wavefront OBJ (text).
    Obj,
    /// Binary STL.
    Stl,
    /// Binary glTF 2.0.
    Glb,
    /// glTF 2.0 JSON with the buffer embedded as base64.
    Gltf,
}

impl ExportFormat {
    /// The format a file name's extension asks for (`.obj`, `.stl`, `.glb`, `.gltf`, any case).
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "obj" => ExportFormat::Obj,
            "stl" => ExportFormat::Stl,
            "glb" => ExportFormat::Glb,
            "gltf" => ExportFormat::Gltf,
            _ => return None,
        })
    }
}

/// The bytes of `triangles` in `format`.
pub(crate) fn encode(triangles: &[Triangle], format: ExportFormat) -> Vec<u8> {
    match format {
        ExportFormat::Obj => obj(triangles).into_bytes(),
        ExportFormat::Stl => stl(triangles),
        ExportFormat::Glb => glb(triangles),
        ExportFormat::Gltf => gltf(triangles).into_bytes(),
    }
}

/// Z-up model coordinates to Y-up.
fn y_up(p: DVec3) -> DVec3 {
    // Adding 0.0 turns a negative zero into a positive one.
    DVec3::new(p.x + 0.0, p.z + 0.0, -p.y + 0.0)
}

/// OBJ with shared positions and normals; faces are triangles.
fn obj(triangles: &[Triangle]) -> String {
    let mut out = String::from("# sketchup-alt export: meters, Y up\no model\n");
    let mut positions: HashMap<[u64; 3], usize> = HashMap::new();
    let mut normals: HashMap<[u64; 3], usize> = HashMap::new();
    let mut faces = String::new();
    let key = |p: DVec3| p.to_array().map(|c| (c + 0.0).to_bits());
    for t in triangles {
        let n = y_up(t.normal);
        let next = normals.len() + 1;
        let ni = *normals.entry(key(n)).or_insert_with(|| {
            let _ = writeln!(out, "vn {} {} {}", n.x, n.y, n.z);
            next
        });
        let mut ids = [0; 3];
        for (k, c) in t.corners.iter().enumerate() {
            let p = y_up(*c);
            let next = positions.len() + 1;
            ids[k] = *positions.entry(key(p)).or_insert_with(|| {
                let _ = writeln!(out, "v {} {} {}", p.x, p.y, p.z);
                next
            });
        }
        let _ = writeln!(
            faces,
            "f {}//{ni} {}//{ni} {}//{ni}",
            ids[0], ids[1], ids[2]
        );
    }
    out.push_str(&faces);
    out
}

/// Binary STL: an 80-byte header, the triangle count, then per triangle its normal, its three
/// corners (f32, little-endian) and a zero attribute word.
fn stl(triangles: &[Triangle]) -> Vec<u8> {
    let mut out = Vec::with_capacity(84 + 50 * triangles.len());
    let mut header = [0u8; 80];
    let title = b"sketchup-alt export, meters";
    header[..title.len()].copy_from_slice(title);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
    for t in triangles {
        for v in std::iter::once(t.normal).chain(t.corners) {
            for c in v.to_array() {
                out.extend_from_slice(&(c as f32).to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

/// The vertex buffer (positions then normals, f32 little-endian, Y-up) and the glTF document
/// describing it, its buffer without a `uri` (a `.glb`'s own chunk).
fn gltf_parts(triangles: &[Triangle]) -> (Vec<u8>, serde_json::Value) {
    let count = triangles.len() * 3;
    let mut buffer = Vec::with_capacity(count * 24);
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for t in triangles {
        for c in t.corners {
            // The bounds must hold the stored f32 values, so round before comparing.
            let p = y_up(c).as_vec3();
            lo = lo.min(p.as_dvec3());
            hi = hi.max(p.as_dvec3());
            for x in p.to_array() {
                buffer.extend_from_slice(&x.to_le_bytes());
            }
        }
    }
    for t in triangles {
        let n = y_up(t.normal);
        for _ in 0..3 {
            for x in n.to_array() {
                buffer.extend_from_slice(&(x as f32).to_le_bytes());
            }
        }
    }
    let half = count * 12;
    let doc = json!({
        "asset": {"version": "2.0", "generator": "sketchup-alt"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "name": "model"}],
        "materials": [{"name": "default", "doubleSided": true,
                       "pbrMetallicRoughness": {"metallicFactor": 0}}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "mode": 4,
                                    "material": 0}]}],
        "buffers": [{"byteLength": buffer.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": half, "target": 34962},
            {"buffer": 0, "byteOffset": half, "byteLength": half, "target": 34962}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
             "min": lo.to_array().map(|x| x as f32), "max": hi.to_array().map(|x| x as f32)},
            {"bufferView": 1, "componentType": 5126, "count": count, "type": "VEC3"}
        ]
    });
    (buffer, doc)
}

/// Binary glTF: a 12-byte header, a JSON chunk padded with spaces and a BIN chunk padded with
/// zeros, each to a multiple of 4 bytes.
fn glb(triangles: &[Triangle]) -> Vec<u8> {
    let (mut bin, doc) = gltf_parts(triangles);
    let mut json = serde_json::to_vec(&doc).expect("a JSON value serializes");
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// glTF JSON with the buffer embedded as a base64 data URI.
fn gltf(triangles: &[Triangle]) -> String {
    let (bytes, mut doc) = gltf_parts(triangles);
    doc["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ));
    serde_json::to_string_pretty(&doc).expect("a JSON value serializes")
}
