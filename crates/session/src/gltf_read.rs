//! Reading glTF 2.0 files (`.gltf` JSON and `.glb` binary) as a scene: meshes of triangles
//! with their normals and texture coordinates, materials with a base color and base-color
//! texture, and the node tree. Animations, skins, cameras, lights and morph targets are left
//! out and named in the warnings.

use std::collections::BTreeMap;

use glam::{DMat4, DQuat, DVec2, DVec3};
use serde::Deserialize;
use serde_json::Value;

use crate::import::{MAX_IMPORT_BYTES, MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS};
use crate::scene_read::{
    FileUp, IMAGE_MISSING, IMAGE_REFUSED, MAX_SCENE_ITEMS, MAX_SCENE_NODES, ReadScene,
    SceneMaterial, SceneMesh, SceneNode, ScenePolygon, SideFiles, counted, srgb_byte,
};
use crate::texture::MAX_TEXTURE_BYTES;

/// Extensions a file may require that change nothing this reader reads.
const HARMLESS: [&str; 5] = [
    "KHR_materials_emissive_strength",
    "KHR_materials_unlit",
    "KHR_texture_transform",
    "KHR_mesh_quantization",
    "KHR_materials_specular",
];

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Gltf {
    asset: Option<Value>,
    scene: Option<usize>,
    scenes: Vec<GScene>,
    nodes: Vec<GNode>,
    meshes: Vec<GMesh>,
    materials: Vec<GMaterial>,
    textures: Vec<GTexture>,
    images: Vec<GImage>,
    accessors: Vec<GAccessor>,
    buffer_views: Vec<GView>,
    buffers: Vec<GBuffer>,
    animations: Vec<Value>,
    skins: Vec<Value>,
    cameras: Vec<Value>,
    extensions_required: Vec<String>,
    extensions: BTreeMap<String, Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GScene {
    nodes: Vec<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GNode {
    name: String,
    mesh: Option<usize>,
    children: Vec<usize>,
    matrix: Option<[f64; 16]>,
    translation: Option<[f64; 3]>,
    rotation: Option<[f64; 4]>,
    scale: Option<[f64; 3]>,
    camera: Option<usize>,
    extensions: BTreeMap<String, Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GMesh {
    name: String,
    primitives: Vec<GPrimitive>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GPrimitive {
    attributes: BTreeMap<String, usize>,
    indices: Option<usize>,
    material: Option<usize>,
    mode: Option<u32>,
    targets: Vec<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GMaterial {
    name: String,
    pbr_metallic_roughness: GPbr,
    alpha_mode: Option<String>,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GPbr {
    base_color_factor: [f64; 4],
    base_color_texture: Option<GTextureRef>,
}

impl Default for GPbr {
    fn default() -> Self {
        GPbr {
            base_color_factor: [1.0; 4],
            base_color_texture: None,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GTextureRef {
    index: usize,
    tex_coord: usize,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GTexture {
    source: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GImage {
    uri: Option<String>,
    buffer_view: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GAccessor {
    buffer_view: Option<usize>,
    byte_offset: u64,
    component_type: u32,
    normalized: bool,
    count: u64,
    #[serde(rename = "type")]
    kind: String,
    sparse: Option<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GView {
    buffer: usize,
    byte_offset: u64,
    byte_length: u64,
    byte_stride: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GBuffer {
    uri: Option<String>,
    byte_length: u64,
}

/// A `.glb`'s JSON chunk and BIN chunk.
type Chunks<'a> = (&'a [u8], Option<&'a [u8]>);

/// A `.glb`'s JSON chunk and BIN chunk (if any), or None when `bytes` are not a `.glb`.
fn glb_chunks(bytes: &[u8]) -> Option<Result<Chunks<'_>, String>> {
    if bytes.get(..4) != Some(b"glTF") {
        return None;
    }
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|w| u32::from_le_bytes(w.try_into().expect("4 bytes")) as usize)
    };
    Some((|| {
        if word(4) != Some(2) {
            return Err("only glTF 2.0 binary files are read".to_string());
        }
        let total = word(8).ok_or("the binary header is cut short")?;
        let end = total.min(bytes.len());
        let mut at = 12;
        let mut json = None;
        let mut bin = None;
        while at + 8 <= end {
            let len = word(at).ok_or("a chunk header is cut short")?;
            let kind = word(at + 4).ok_or("a chunk header is cut short")?;
            let start = at + 8;
            let stop = start
                .checked_add(len)
                .filter(|s| *s <= end)
                .ok_or("a chunk runs past the end of the file")?;
            match kind {
                0x4E4F_534A if json.is_none() => json = Some(&bytes[start..stop]),
                0x004E_4942 if bin.is_none() => bin = Some(&bytes[start..stop]),
                _ => {}
            }
            at = stop.div_ceil(4) * 4;
        }
        Ok((json.ok_or("the binary file has no JSON chunk")?, bin))
    })())
}

/// Reads a glTF file's `bytes` (JSON, or binary with the `glTF` header), its buffers and
/// images through `side`. Fails, saying why, for a file that is not glTF 2.0, needs an
/// extension that changes the geometry (Draco and meshopt compression among them), has
/// accessors or buffers that do not fit, or passes the limits.
pub(crate) fn read_gltf(bytes: &[u8], side: &SideFiles) -> Result<ReadScene, String> {
    let (json, bin) = match glb_chunks(bytes) {
        Some(chunks) => chunks?,
        None => (bytes, None),
    };
    let doc: Gltf = serde_json::from_slice(json).map_err(|e| format!("not a glTF file ({e})"))?;
    let version = doc
        .asset
        .as_ref()
        .and_then(|a| a.get("version"))
        .and_then(Value::as_str);
    if version.is_none_or(|v| !v.starts_with("2.")) {
        return Err("not a glTF 2.0 file".into());
    }
    let needed: Vec<&str> = doc
        .extensions_required
        .iter()
        .map(String::as_str)
        .filter(|e| !HARMLESS.contains(e))
        .collect();
    if !needed.is_empty() {
        return Err(format!(
            "the file needs glTF extensions that are not supported: {}",
            needed.join(", ")
        ));
    }
    if doc.nodes.len() > MAX_SCENE_NODES {
        return Err(format!("more than {MAX_SCENE_NODES} nodes"));
    }
    for (what, n) in [
        ("meshes", doc.meshes.len()),
        ("materials", doc.materials.len()),
        ("images", doc.images.len()),
        ("textures", doc.textures.len()),
        ("buffers", doc.buffers.len()),
    ] {
        if n > MAX_SCENE_ITEMS {
            return Err(format!("more than {MAX_SCENE_ITEMS} {what}"));
        }
    }
    let mut warnings = Vec::new();
    let buffers = read_buffers(&doc, bin, side)?;
    let data = Data {
        doc: &doc,
        buffers: &buffers,
    };

    // Which texture coordinate set each material's base-color texture uses.
    let mut uv_set: Vec<Option<usize>> = vec![None; doc.materials.len()];
    let mut materials = Vec::with_capacity(doc.materials.len());
    let mut image_cache: BTreeMap<usize, Result<Vec<u8>, &'static str>> = BTreeMap::new();
    for (i, m) in doc.materials.iter().enumerate() {
        let f = m.pbr_metallic_roughness.base_color_factor;
        let image = m
            .pbr_metallic_roughness
            .base_color_texture
            .as_ref()
            .and_then(|t| {
                uv_set[i] = Some(t.tex_coord);
                let source = doc.textures.get(t.index)?.source?;
                Some(
                    image_cache
                        .entry(source)
                        .or_insert_with(|| data.image(source, side))
                        .clone(),
                )
            });
        let blend = m.alpha_mode.as_deref() == Some("BLEND");
        materials.push(SceneMaterial {
            name: m.name.clone(),
            color: [f[0], f[1], f[2]].map(srgb_byte),
            opacity: if blend && f[3].is_finite() {
                f[3].clamp(0.0, 1.0) as f32
            } else {
                1.0
            },
            image,
        });
    }

    let mut meshes = Vec::with_capacity(doc.meshes.len());
    let (mut points, mut polygons) = (0usize, 0usize);
    let (mut lines, mut morphs, mut unread, mut skipped) = (0usize, 0usize, 0usize, 0usize);
    for m in &doc.meshes {
        let mut mesh = SceneMesh {
            name: m.name.clone(),
            ..SceneMesh::default()
        };
        // Normals and coordinates are kept only when every primitive has them.
        let all_normals = m
            .primitives
            .iter()
            .all(|p| p.attributes.contains_key("NORMAL"));
        let set_of = |p: &GPrimitive| p.material.and_then(|i| uv_set.get(i).copied().flatten());
        let all_uvs = m.primitives.iter().any(|p| set_of(p).is_some())
            && m.primitives.iter().all(|p| {
                p.attributes
                    .contains_key(&format!("TEXCOORD_{}", set_of(p).unwrap_or(0)))
            });
        for p in &m.primitives {
            morphs += usize::from(!p.targets.is_empty());
            let mode = p.mode.unwrap_or(4);
            if mode < 4 {
                lines += 1;
                continue;
            }
            if mode > 6 {
                unread += 1;
                continue;
            }
            let Some(&position) = p.attributes.get("POSITION") else {
                unread += 1;
                continue;
            };
            let positions = data.vec3(position)?;
            if points + positions.len() > MAX_IMPORT_POINTS {
                return Err(format!("more than {MAX_IMPORT_POINTS} positions"));
            }
            points += positions.len();
            let normals = match (all_normals, p.attributes.get("NORMAL")) {
                (true, Some(&n)) => Some(data.vec3(n)?),
                _ => None,
            };
            let uvs = if all_uvs {
                let set = set_of(p).unwrap_or(0);
                match p.attributes.get(&format!("TEXCOORD_{set}")) {
                    Some(&t) => Some(data.vec2(t)?),
                    None => None,
                }
            } else {
                None
            };
            if normals.as_ref().is_some_and(|n| n.len() != positions.len())
                || uvs.as_ref().is_some_and(|t| t.len() != positions.len())
            {
                return Err("a primitive's attributes have different counts".into());
            }
            let indices: Vec<usize> = match p.indices {
                Some(i) => data.indices(i)?,
                None => (0..positions.len()).collect(),
            };
            let triangles = triangles(mode, &indices);
            if polygons + triangles.len() > MAX_IMPORT_POLYGONS {
                return Err(format!("more than {MAX_IMPORT_POLYGONS} triangles"));
            }
            polygons += triangles.len();
            let start = mesh.points.len();
            mesh.points.extend(positions);
            if let Some(n) = normals {
                mesh.normals.extend(n);
            }
            if let Some(t) = uvs {
                mesh.uvs.extend(t);
            }
            let material = p.material.filter(|i| *i < materials.len());
            let count = mesh.points.len() - start;
            for t in triangles {
                // A triangle with an index past the positions is skipped.
                if t.iter().any(|&c| c >= count) {
                    skipped += 1;
                    continue;
                }
                mesh.polygons.push(ScenePolygon {
                    corners: t.iter().map(|c| start + c).collect(),
                    holes: Vec::new(),
                    material,
                });
            }
        }
        meshes.push(mesh);
    }

    let mut nodes = Vec::with_capacity(doc.nodes.len());
    let (mut cameras, mut lights) = (0usize, 0usize);
    for n in &doc.nodes {
        cameras += usize::from(n.camera.is_some());
        lights += usize::from(n.extensions.contains_key("KHR_lights_punctual"));
        nodes.push(SceneNode {
            name: n.name.clone(),
            transform: transform(n)?,
            meshes: n.mesh.filter(|m| *m < meshes.len()).into_iter().collect(),
            children: n.children.clone(),
            shared: false,
        });
    }
    let roots = match doc.scene.or((!doc.scenes.is_empty()).then_some(0)) {
        Some(s) => doc
            .scenes
            .get(s)
            .ok_or("the file's scene does not exist")?
            .nodes
            .clone(),
        None => {
            // No scene: every node no node places.
            let mut placed = vec![false; nodes.len()];
            for n in &nodes {
                for &c in &n.children {
                    if let Some(p) = placed.get_mut(c) {
                        *p = true;
                    }
                }
            }
            (0..nodes.len()).filter(|&i| !placed[i]).collect()
        }
    };

    let file_lights = doc
        .extensions
        .get("KHR_lights_punctual")
        .and_then(|l| l.get("lights"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    for (n, one, many) in [
        (doc.animations.len(), "animation was", "animations were"),
        (doc.skins.len(), "skin was", "skins were"),
        (cameras.max(doc.cameras.len()), "camera was", "cameras were"),
        (lights.max(file_lights), "light was", "lights were"),
        (
            morphs,
            "mesh's morph targets were",
            "meshes' morph targets were",
        ),
        (
            lines,
            "line or point primitive was",
            "line and point primitives were",
        ),
        (
            unread,
            "primitive could not be read and was",
            "primitives could not be read and were",
        ),
    ] {
        if n > 0 {
            warnings.push(format!("{} not imported", counted(n, one, many)));
        }
    }
    Ok(ReadScene {
        meshes,
        materials,
        nodes,
        roots,
        unit: 1.0,
        up: FileUp::Y,
        v_down: true,
        skipped,
        warnings,
    })
}

/// Each buffer's bytes: the `.glb`'s BIN chunk for a first buffer without a URI, else what
/// its URI names. Fails for a missing or short buffer, or more than `MAX_IMPORT_BYTES` in all.
fn read_buffers(doc: &Gltf, bin: Option<&[u8]>, side: &SideFiles) -> Result<Vec<Vec<u8>>, String> {
    let mut out = Vec::with_capacity(doc.buffers.len());
    let mut total = 0u64;
    for (i, b) in doc.buffers.iter().enumerate() {
        let bytes = match (&b.uri, i, bin) {
            (Some(uri), _, _) => side
                .read(uri, MAX_IMPORT_BYTES - total)
                .map_err(|e| format!("buffer {i} could not be read: {e}"))?,
            (None, 0, Some(bin)) => bin.to_vec(),
            (None, _, _) => return Err(format!("buffer {i} has no data")),
        };
        if (bytes.len() as u64) < b.byte_length {
            return Err(format!("buffer {i} is shorter than its byteLength"));
        }
        total += bytes.len() as u64;
        if total > MAX_IMPORT_BYTES {
            return Err(format!("the buffers hold over {MAX_IMPORT_BYTES} bytes"));
        }
        out.push(bytes);
    }
    Ok(out)
}

/// A node's transform: its matrix (column-major), else translation, rotation and scale.
fn transform(n: &GNode) -> Result<DMat4, String> {
    let m = match n.matrix {
        Some(m) => DMat4::from_cols_array(&m),
        None => {
            let t = n.translation.map_or(DVec3::ZERO, DVec3::from);
            let r = n.rotation.map_or(DQuat::IDENTITY, |[x, y, z, w]| {
                DQuat::from_xyzw(x, y, z, w).normalize()
            });
            let s = n.scale.map_or(DVec3::ONE, DVec3::from);
            DMat4::from_scale_rotation_translation(s, r, t)
        }
    };
    if !m.is_finite() {
        return Err(format!(
            "node \"{}\" has a transform that is not finite",
            n.name
        ));
    }
    Ok(m)
}

/// The triangles of a primitive of `mode` (4 triangles, 5 strip, 6 fan) over `indices`.
fn triangles(mode: u32, indices: &[usize]) -> Vec<[usize; 3]> {
    match mode {
        5 => (2..indices.len())
            .map(|i| {
                let (a, b, c) = (indices[i - 2], indices[i - 1], indices[i]);
                // Every other triangle of a strip turns the other way.
                if i % 2 == 0 { [a, b, c] } else { [b, a, c] }
            })
            .collect(),
        6 => (2..indices.len())
            .map(|i| [indices[0], indices[i - 1], indices[i]])
            .collect(),
        _ => indices.as_chunks::<3>().0.to_vec(),
    }
}

/// The document and its buffers, for reading accessors and images.
struct Data<'a> {
    doc: &'a Gltf,
    buffers: &'a [Vec<u8>],
}

impl Data<'_> {
    /// The bytes of buffer view `v`, checked against its buffer.
    fn view(&self, v: usize) -> Result<(&[u8], Option<u64>), String> {
        let view = self
            .doc
            .buffer_views
            .get(v)
            .ok_or(format!("buffer view {v} does not exist"))?;
        let buffer = self.buffers.get(view.buffer).ok_or(format!(
            "buffer view {v} names a buffer that does not exist"
        ))?;
        let end = view
            .byte_offset
            .checked_add(view.byte_length)
            .filter(|e| *e <= buffer.len() as u64)
            .ok_or(format!("buffer view {v} runs past the end of its buffer"))?;
        Ok((
            &buffer[view.byte_offset as usize..end as usize],
            view.byte_stride,
        ))
    }

    /// Accessor `a`'s elements of `width` components each, as numbers (normalized integers
    /// scaled to 0..1 or -1..1). An accessor without a buffer view is zeros. Fails for a
    /// missing accessor, another element type, a sparse accessor or one that does not fit.
    fn read(&self, a: usize, width: usize, max_count: u64) -> Result<Vec<f64>, String> {
        let acc = self
            .doc
            .accessors
            .get(a)
            .ok_or(format!("accessor {a} does not exist"))?;
        let kinds = ["SCALAR", "VEC2", "VEC3", "VEC4"];
        if kinds.get(width - 1) != Some(&acc.kind.as_str()) {
            return Err(format!("accessor {a} is not a {}", kinds[width - 1]));
        }
        if acc.sparse.is_some() {
            return Err(format!("accessor {a} is sparse, which is not supported"));
        }
        if acc.count > max_count {
            return Err(format!("accessor {a} has more than {max_count} elements"));
        }
        let count = acc.count as usize;
        let Some(v) = acc.buffer_view else {
            return Ok(vec![0.0; count * width]);
        };
        let size: usize = match acc.component_type {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            t => return Err(format!("accessor {a} has an unknown component type {t}")),
        };
        let (bytes, stride) = self.view(v)?;
        let element = (size * width) as u64;
        let stride = stride.unwrap_or(element).max(element);
        if count > 0 {
            let end = acc
                .byte_offset
                .checked_add(stride.checked_mul(acc.count - 1).ok_or("overflow")?)
                .and_then(|x| x.checked_add(element))
                .filter(|e| *e <= bytes.len() as u64);
            if end.is_none() {
                return Err(format!("accessor {a} runs past the end of its buffer view"));
            }
        }
        let mut out = Vec::with_capacity(count * width);
        for i in 0..count {
            let base = (acc.byte_offset + stride * i as u64) as usize;
            for c in 0..width {
                let at = base + c * size;
                let b = &bytes[at..at + size];
                let n = acc.normalized;
                let x = match acc.component_type {
                    5120 => {
                        let v = f64::from(b[0] as i8);
                        if n { (v / 127.0).max(-1.0) } else { v }
                    }
                    5121 => {
                        let v = f64::from(b[0]);
                        if n { v / 255.0 } else { v }
                    }
                    5122 => {
                        let v = f64::from(i16::from_le_bytes([b[0], b[1]]));
                        if n { (v / 32767.0).max(-1.0) } else { v }
                    }
                    5123 => {
                        let v = f64::from(u16::from_le_bytes([b[0], b[1]]));
                        if n { v / 65535.0 } else { v }
                    }
                    5125 => f64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    _ => f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                };
                if !x.is_finite() {
                    return Err(format!("accessor {a} has a number that is not finite"));
                }
                out.push(x);
            }
        }
        Ok(out)
    }

    fn vec3(&self, a: usize) -> Result<Vec<DVec3>, String> {
        let values = self.read(a, 3, MAX_IMPORT_POINTS as u64)?;
        Ok(values
            .as_chunks::<3>()
            .0
            .iter()
            .map(|v| DVec3::from_array(*v))
            .collect())
    }

    fn vec2(&self, a: usize) -> Result<Vec<DVec2>, String> {
        let values = self.read(a, 2, MAX_IMPORT_POINTS as u64)?;
        Ok(values
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| DVec2::from_array(*v))
            .collect())
    }

    fn indices(&self, a: usize) -> Result<Vec<usize>, String> {
        if let Some(acc) = self.doc.accessors.get(a)
            && !matches!(acc.component_type, 5121 | 5123 | 5125)
        {
            return Err(format!(
                "accessor {a} holds indices that are not whole numbers"
            ));
        }
        let most = 3 * MAX_IMPORT_POLYGONS as u64 + 2;
        Ok(self
            .read(a, 1, most)?
            .into_iter()
            .map(|x| x as usize)
            .collect())
    }

    /// Image `i`'s file bytes, from its buffer view or URI, or why they could not be read.
    fn image(&self, i: usize, side: &SideFiles) -> Result<Vec<u8>, &'static str> {
        let image = self.doc.images.get(i).ok_or(IMAGE_MISSING)?;
        if let Some(v) = image.buffer_view {
            let (bytes, _) = self.view(v).map_err(|_| IMAGE_MISSING)?;
            return Ok(bytes.to_vec());
        }
        let uri = image.uri.as_deref().ok_or(IMAGE_MISSING)?;
        side.read(uri, MAX_TEXTURE_BYTES).map_err(|e| {
            if e.contains("URL") || e.contains("absolute") || e.contains("outside") {
                IMAGE_REFUSED
            } else {
                IMAGE_MISSING
            }
        })
    }
}
