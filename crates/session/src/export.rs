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

/// What an `export` wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExportReport {
    /// Triangles written.
    pub triangles: usize,
    /// Faces left out because they could not be triangulated (counted once per placement).
    pub skipped_faces: usize,
}

/// What an export writes: the file's bytes and, for an OBJ whose faces are painted, the bytes
/// of the material library (`.mtl`) it names.
pub(crate) struct Encoded {
    pub bytes: Vec<u8>,
    pub mtl: Option<Vec<u8>>,
}

/// The bytes of `triangles` in `format`. Materials are looked up in `scene`; an OBJ refers to
/// its material library as `mtl_name`. Fails when the triangles do not fit the format's 32-bit
/// counts: a binary STL holds at most 2^32 - 1 triangles and a `.glb` at most 4 GiB (about
/// 59 million triangles).
pub(crate) fn encode(
    scene: &Scene,
    triangles: &[Triangle],
    format: ExportFormat,
    mtl_name: &str,
) -> Result<Encoded, String> {
    let groups = Groups::of(scene, triangles);
    Ok(match format {
        ExportFormat::Obj => {
            let (obj, mtl) = obj(&groups, mtl_name);
            Encoded {
                bytes: obj.into_bytes(),
                mtl: mtl.map(String::into_bytes),
            }
        }
        ExportFormat::Stl => Encoded {
            bytes: stl(triangles)?,
            mtl: None,
        },
        ExportFormat::Glb => Encoded {
            bytes: glb(&groups)?,
            mtl: None,
        },
        ExportFormat::Gltf => Encoded {
            bytes: gltf(&groups).into_bytes(),
            mtl: None,
        },
    })
}

/// A binary STL's triangle count, if it fits its 32 bits.
fn stl_count(triangles: usize) -> Result<u32, String> {
    u32::try_from(triangles).map_err(|_| {
        format!(
            "too many triangles for a binary STL: {triangles} (at most {})",
            u32::MAX
        )
    })
}

/// A `.glb`'s total, JSON chunk and BIN chunk lengths (chunks already padded), if the whole
/// file fits the 32-bit length in its header.
fn glb_lengths(json: usize, bin: usize) -> Result<[u32; 3], String> {
    // Header 12 bytes, then each chunk's 8-byte header and data.
    let total = json
        .checked_add(bin)
        .and_then(|n| n.checked_add(12 + 8 + 8));
    match total.map(u32::try_from) {
        Some(Ok(total)) => Ok([total, json as u32, bin as u32]),
        _ => Err(
            "the model is too large for a .glb (over 4 GiB): export to .gltf or .obj".to_string(),
        ),
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
    /// 1 is opaque.
    opacity: f32,
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
                    let (name, color, opacity) = match material.and_then(|m| scene.material(m)) {
                        Some(m) => (
                            m.name.clone(),
                            m.color.map(|c| f64::from(c) / 255.0),
                            m.opacity,
                        ),
                        None => ("default".to_string(), DEFAULT_COLOR, 1.0),
                    };
                    groups.push(Group {
                        material,
                        name,
                        color,
                        opacity,
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

/// `x` as the f64 of its shortest decimal form, so 0.4 is written 0.4 and not
/// 0.4000000059604645.
fn decimal(x: f32) -> f64 {
    x.to_string().parse().unwrap_or(f64::from(x))
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
            if g.opacity < 1.0 {
                // `d` is the opacity; some readers take `Tr`, its complement, instead.
                let d = f64::from(g.opacity);
                let _ = writeln!(mtl, "d {d:.6}\nTr {:.6}", 1.0 - d);
            }
        }
        mtl
    });
    (out, mtl)
}

/// Binary STL: an 80-byte header, the triangle count, then per triangle its normal, its three
/// corners (f32, little-endian) and a zero attribute word.
fn stl(triangles: &[Triangle]) -> Result<Vec<u8>, String> {
    let count = stl_count(triangles.len())?;
    let mut out = Vec::with_capacity(84 + 50 * triangles.len());
    let mut header = [0u8; 80];
    let title = b"sketchup-alt export, meters";
    header[..title.len()].copy_from_slice(title);
    out.extend_from_slice(&header);
    out.extend_from_slice(&count.to_le_bytes());
    for t in triangles {
        for v in std::iter::once(t.normal).chain(t.corners) {
            for c in v.to_array() {
                out.extend_from_slice(&(c as f32).to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    Ok(out)
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
            pbr["baseColorFactor"] = json!([r, gr, b, decimal(g.opacity)]);
        }
        let mut material = json!({"name": g.name, "doubleSided": true,
            "pbrMetallicRoughness": pbr});
        if g.opacity < 1.0 {
            material["alphaMode"] = json!("BLEND");
        }
        materials.push(material);
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
fn glb(groups: &Groups) -> Result<Vec<u8>, String> {
    let (mut bin, doc) = gltf_parts(groups);
    let mut json = serde_json::to_vec(&doc).expect("a JSON value serializes");
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let [total, json_len, bin_len] = glb_lengths(json.len(), bin.len())?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&json_len.to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&bin_len.to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_past_32_bits_are_refused() {
        assert_eq!(stl_count(5), Ok(5));
        assert_eq!(stl_count(u32::MAX as usize), Ok(u32::MAX));
        let err = stl_count(u32::MAX as usize + 1).unwrap_err();
        assert!(err.contains("too many triangles for a binary STL"), "{err}");

        assert_eq!(glb_lengths(100, 72), Ok([200, 100, 72]));
        let most = u32::MAX as usize - 28;
        assert_eq!(
            glb_lengths(4, most - 4),
            Ok([u32::MAX, 4, (most - 4) as u32])
        );
        // 60 million triangles take 72 bytes each in the BIN chunk.
        let err = glb_lengths(1000, 60_000_000 * 72).unwrap_err();
        assert!(err.contains("too large for a .glb"), "{err}");
        assert!(glb_lengths(4, most - 3).is_err());
        assert!(glb_lengths(usize::MAX, 8).is_err());
    }
}
