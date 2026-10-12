//! Exporting the model's faces as triangles: OBJ (with a `.mtl` beside it when faces are
//! painted), STL, and glTF (binary `.glb` or `.gltf` with its buffer embedded). Units are
//! meters. OBJ and glTF are Y-up (x, z, -y); STL keeps the model's Z-up coordinates. Colors are
//! those of the faces' front sides; STL has none.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use base64::Engine;
use glam::DVec3;
use kernel::{MaterialId, Scene, Triangle};
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

/// The default material's front color (sRGB), as the window shows it.
const DEFAULT_COLOR: [f64; 3] = [0.93, 0.93, 0.90];

/// What an export writes: the file's bytes and, for an OBJ whose faces are painted, the bytes
/// of the material library (`.mtl`) it names.
pub(crate) struct Encoded {
    pub bytes: Vec<u8>,
    pub mtl: Option<Vec<u8>>,
}

/// The bytes of `triangles` in `format`. Materials are looked up in `scene`; an OBJ refers to
/// its material library as `mtl_name`.
pub(crate) fn encode(
    scene: &Scene,
    triangles: &[Triangle],
    format: ExportFormat,
    mtl_name: &str,
) -> Encoded {
    let groups = Groups::of(scene, triangles);
    match format {
        ExportFormat::Obj => {
            let (obj, mtl) = obj(&groups, mtl_name);
            Encoded {
                bytes: obj.into_bytes(),
                mtl: mtl.map(String::into_bytes),
            }
        }
        ExportFormat::Stl => Encoded {
            bytes: stl(triangles),
            mtl: None,
        },
        ExportFormat::Glb => Encoded {
            bytes: glb(&groups),
            mtl: None,
        },
        ExportFormat::Gltf => Encoded {
            bytes: gltf(&groups).into_bytes(),
            mtl: None,
        },
    }
}

/// The triangles grouped by the material of their front side, in the order each material is
/// first used, with the name and color each group is exported with.
struct Groups<'a> {
    groups: Vec<Group<'a>>,
}

struct Group<'a> {
    /// None for the default material.
    material: Option<MaterialId>,
    name: String,
    /// sRGB, 0 to 1.
    color: [f64; 3],
    triangles: Vec<&'a Triangle>,
}

impl<'a> Groups<'a> {
    fn of(scene: &Scene, triangles: &'a [Triangle]) -> Groups<'a> {
        let mut groups: Vec<Group<'a>> = Vec::new();
        for t in triangles {
            let material = t.front.filter(|m| scene.material(*m).is_some());
            let i = match groups.iter().position(|g| g.material == material) {
                Some(i) => i,
                None => {
                    let (name, color) = match material.and_then(|m| scene.material(m)) {
                        Some(m) => (m.name.clone(), m.color.map(|c| f64::from(c) / 255.0)),
                        None => ("default".to_string(), DEFAULT_COLOR),
                    };
                    groups.push(Group {
                        material,
                        name,
                        color,
                        triangles: Vec::new(),
                    });
                    groups.len() - 1
                }
            };
            groups[i].triangles.push(t);
        }
        Groups { groups }
    }

    /// True when some triangle has a material of its own.
    fn painted(&self) -> bool {
        self.groups.iter().any(|g| g.material.is_some())
    }

    fn triangles(&self) -> impl Iterator<Item = &&'a Triangle> {
        self.groups.iter().flat_map(|g| &g.triangles)
    }
}

/// An sRGB component (0 to 1) as linear light, as glTF colors and most `.mtl` readers take it.
pub(crate) fn linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Material names for OBJ and MTL: no spaces or other characters those files split on, and no
/// two alike.
fn obj_names(groups: &Groups) -> Vec<String> {
    let mut taken: BTreeSet<String> = BTreeSet::new();
    groups
        .groups
        .iter()
        .map(|g| {
            let base: String = g
                .name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let name = std::iter::once(base.clone())
                .chain((2..).map(|n| format!("{base}_{n}")))
                .find(|n| !taken.contains(n))
                .expect("a free name");
            taken.insert(name.clone());
            name
        })
        .collect()
}

/// Z-up model coordinates to Y-up.
fn y_up(p: DVec3) -> DVec3 {
    // Adding 0.0 turns a negative zero into a positive one.
    DVec3::new(p.x + 0.0, p.z + 0.0, -p.y + 0.0)
}

/// OBJ with shared positions and normals; faces are triangles. When some faces are painted,
/// each material's faces follow a `usemtl` line and the material library is returned too.
fn obj(groups: &Groups, mtl_name: &str) -> (String, Option<String>) {
    let mut out = String::from("# sketchup-alt export: meters, Y up\n");
    let painted = groups.painted();
    if painted {
        let _ = writeln!(out, "mtllib {mtl_name}");
    }
    out.push_str("o model\n");
    let names = obj_names(groups);
    let mut positions: HashMap<[u64; 3], usize> = HashMap::new();
    let mut normals: HashMap<[u64; 3], usize> = HashMap::new();
    let mut faces = String::new();
    let key = |p: DVec3| p.to_array().map(|c| (c + 0.0).to_bits());
    for (g, name) in groups.groups.iter().zip(&names) {
        if painted {
            let _ = writeln!(faces, "usemtl {name}");
        }
        for t in &g.triangles {
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
    }
    out.push_str(&faces);
    let mtl = painted.then(|| {
        let mut mtl = String::from("# sketchup-alt materials: diffuse colors, linear\n");
        for (g, name) in groups.groups.iter().zip(&names) {
            let [r, gr, b] = g.color.map(linear);
            let _ = writeln!(mtl, "newmtl {name}\nKd {r:.6} {gr:.6} {b:.6}");
        }
        mtl
    });
    (out, mtl)
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

/// The vertex buffer (positions then normals, f32 little-endian, Y-up, each material's
/// triangles together) and the glTF document describing it: one primitive per material, its
/// buffer without a `uri` (a `.glb`'s own chunk).
fn gltf_parts(groups: &Groups) -> (Vec<u8>, serde_json::Value) {
    let count: usize = groups.groups.iter().map(|g| g.triangles.len() * 3).sum();
    let mut buffer = Vec::with_capacity(count * 24);
    let mut bounds = Vec::new();
    for g in &groups.groups {
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for t in &g.triangles {
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
        bounds.push((lo, hi));
    }
    for t in groups.triangles() {
        let n = y_up(t.normal);
        for _ in 0..3 {
            for x in n.to_array() {
                buffer.extend_from_slice(&(x as f32).to_le_bytes());
            }
        }
    }
    let half = count * 12;
    let mut accessors = Vec::new();
    let mut primitives = Vec::new();
    let mut materials = Vec::new();
    let mut start = 0;
    for (k, (g, (lo, hi))) in groups.groups.iter().zip(&bounds).enumerate() {
        let n = g.triangles.len() * 3;
        accessors.push(
            json!({"bufferView": 0, "byteOffset": start * 12, "componentType": 5126,
            "count": n, "type": "VEC3",
            "min": lo.to_array().map(|x| x as f32), "max": hi.to_array().map(|x| x as f32)}),
        );
        accessors.push(
            json!({"bufferView": 1, "byteOffset": start * 12, "componentType": 5126,
            "count": n, "type": "VEC3"}),
        );
        primitives.push(
            json!({"attributes": {"POSITION": 2 * k, "NORMAL": 2 * k + 1},
            "mode": 4, "material": k}),
        );
        let mut pbr = json!({"metallicFactor": 0});
        if g.material.is_some() {
            let [r, gr, b] = g.color.map(linear);
            pbr["baseColorFactor"] = json!([r, gr, b, 1.0]);
        }
        materials.push(json!({"name": g.name, "doubleSided": true,
            "pbrMetallicRoughness": pbr}));
        start += n;
    }
    let doc = json!({
        "asset": {"version": "2.0", "generator": "sketchup-alt"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "name": "model"}],
        "materials": materials,
        "meshes": [{"primitives": primitives}],
        "buffers": [{"byteLength": buffer.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": half, "target": 34962},
            {"buffer": 0, "byteOffset": half, "byteLength": half, "target": 34962}
        ],
        "accessors": accessors
    });
    (buffer, doc)
}

/// Binary glTF: a 12-byte header, a JSON chunk padded with spaces and a BIN chunk padded with
/// zeros, each to a multiple of 4 bytes.
fn glb(groups: &Groups) -> Vec<u8> {
    let (mut bin, doc) = gltf_parts(groups);
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
fn gltf(groups: &Groups) -> String {
    let (bytes, mut doc) = gltf_parts(groups);
    doc["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ));
    serde_json::to_string_pretty(&doc).expect("a JSON value serializes")
}
