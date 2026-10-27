//! Reading glTF 2.0 files (`.gltf` JSON and `.glb` binary) as a scene: meshes of triangles
//! with their normals and texture coordinates, materials with a base color and base-color
//! texture, and the node tree. Animations, skins, cameras, lights and morph targets are left
//! out and named in the warnings.

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::{DMat4, DQuat, DVec2, DVec3};
use serde::Deserialize;
use serde_json::Value;

use crate::draco;
use crate::import::{MAX_IMPORT_BYTES, MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS};
use crate::meshopt;
use crate::scene_read::{
    FileUp, IMAGE_MISSING, ImageBudget, MAX_SCENE_ITEMS, MAX_SCENE_NODES, ReadScene, SceneMaterial,
    SceneMesh, SceneNode, ScenePolygon, SideFiles, counted, paint_vertex_colors, srgb_byte,
};

/// Extensions a file may require that change nothing this reader reads, or that it applies
/// (`KHR_texture_transform`, and the two compressions, decoded here).
const HARMLESS: [&str; 7] = [
    "EXT_meshopt_compression",
    "KHR_draco_mesh_compression",
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
    extensions: BTreeMap<String, Value>,
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
    extensions: GTextureExtensions,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GTextureExtensions {
    #[serde(rename = "KHR_texture_transform")]
    transform: Option<GTextureTransform>,
}

/// `KHR_texture_transform`: texture coordinates scaled, then turned, then offset.
#[derive(Deserialize, Clone, Copy)]
#[serde(default, rename_all = "camelCase")]
struct GTextureTransform {
    offset: [f64; 2],
    rotation: f64,
    scale: [f64; 2],
    tex_coord: Option<usize>,
}

impl Default for GTextureTransform {
    fn default() -> Self {
        GTextureTransform {
            offset: [0.0; 2],
            rotation: 0.0,
            scale: [1.0; 2],
            tex_coord: None,
        }
    }
}

impl GTextureTransform {
    /// Texture coordinates `uv` as the transform moves them.
    fn apply(&self, uv: DVec2) -> DVec2 {
        let (sin, cos) = self.rotation.sin_cos();
        let s = uv * DVec2::from(self.scale);
        DVec2::new(cos * s.x + sin * s.y, cos * s.y - sin * s.x) + DVec2::from(self.offset)
    }

    /// True when a texture placement (a turn and a scale along each side) lays the moved
    /// coordinates exactly: no mirror, and no shear (equal scales, or a turn by quarters).
    fn is_simple(&self) -> bool {
        let [x, y] = self.scale;
        let quarter = (self.rotation / std::f64::consts::FRAC_PI_2).fract().abs();
        x > 0.0
            && y > 0.0
            && ((x - y).abs() <= 1e-9 * x.max(y) || quarter < 1e-9 || quarter > 1.0 - 1e-9)
    }
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
    extensions: BTreeMap<String, Value>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct GBuffer {
    uri: Option<String>,
    byte_length: u64,
    extensions: BTreeMap<String, Value>,
}

/// Most objects a glTF's JSON may hold, and most values its arrays may hold in all: each
/// becomes a few hundred bytes at most once read, so a small file cannot ask for gigabytes.
const MAX_JSON_OBJECTS: usize = 1_000_000;
const MAX_JSON_VALUES: usize = 8_000_000;

/// Fails when `json` holds more objects or array values than the limits (strings are skipped).
fn check_json_size(json: &[u8]) -> Result<(), String> {
    let (mut objects, mut values) = (0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for &b in json {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => objects += 1,
            b'[' | b',' => values += 1,
            _ => {}
        }
    }
    if objects > MAX_JSON_OBJECTS || values > MAX_JSON_VALUES {
        return Err(format!(
            "the file's JSON holds too many items (at most {MAX_JSON_OBJECTS} objects and \
             {MAX_JSON_VALUES} values)"
        ));
    }
    Ok(())
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
    check_json_size(json)?;
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
    let compressed = meshopt_views(&doc)?;
    let buffers = read_buffers(&doc, bin, side, &compressed)?;
    let decoded = decode_views(&buffers, &compressed)?;
    let data = Data {
        doc: &doc,
        buffers: &buffers,
        decoded: &decoded,
    };

    // Which texture coordinate set each material's base-color texture uses.
    let mut uv_set: Vec<Option<usize>> = vec![None; doc.materials.len()];
    // And how it moves those coordinates.
    let mut uv_transform: Vec<Option<GTextureTransform>> = vec![None; doc.materials.len()];
    let mut sheared = 0usize;
    let mut materials = Vec::with_capacity(doc.materials.len());
    let mut image_cache: BTreeMap<usize, Result<Vec<u8>, &'static str>> = BTreeMap::new();
    let mut budget = ImageBudget::default();
    for (i, m) in doc.materials.iter().enumerate() {
        let f = m.pbr_metallic_roughness.base_color_factor;
        let image = m
            .pbr_metallic_roughness
            .base_color_texture
            .as_ref()
            .and_then(|t| {
                uv_set[i] = Some(t.tex_coord);
                if let Some(x) = t.extensions.transform {
                    uv_set[i] = Some(x.tex_coord.unwrap_or(t.tex_coord));
                    uv_transform[i] = Some(x);
                    sheared += usize::from(!x.is_simple());
                }
                let source = doc.textures.get(t.index)?.source?;
                Some(
                    image_cache
                        .entry(source)
                        .or_insert_with(|| data.image(source, side, &mut budget))
                        .clone(),
                )
            });
        let blend = m.alpha_mode.as_deref() == Some("BLEND");
        materials.push(SceneMaterial {
            name: m.name.clone(),
            color: [f[0], f[1], f[2]].map(srgb_byte),
            tint: [f[0], f[1], f[2]],
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
        // Normals are kept only when every primitive has them; coordinates and colors are
        // filled in (zeros, white) for the primitives that lack them.
        let all_normals = m
            .primitives
            .iter()
            .all(|p| p.attributes.contains_key("NORMAL"));
        let set_of = |p: &GPrimitive| p.material.and_then(|i| uv_set.get(i).copied().flatten());
        let (mut any_uvs, mut any_colors) = (false, false);
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
            if !p.attributes.contains_key("POSITION") {
                unread += 1;
                continue;
            }
            let got = data.primitive(p, mode, all_normals, set_of(p))?;
            let positions = got.positions;
            if points + positions.len() > MAX_IMPORT_POINTS {
                return Err(format!("more than {MAX_IMPORT_POINTS} positions"));
            }
            points += positions.len();
            let uvs = got.uvs.map(|uvs| {
                match p
                    .material
                    .and_then(|i| uv_transform.get(i).copied().flatten())
                {
                    Some(x) => uvs.into_iter().map(|uv| x.apply(uv)).collect(),
                    None => uvs,
                }
            });
            let (normals, colors) = (got.normals, got.colors);
            if normals.as_ref().is_some_and(|n| n.len() != positions.len())
                || uvs.as_ref().is_some_and(|t| t.len() != positions.len())
                || colors.as_ref().is_some_and(|c| c.len() != positions.len())
            {
                return Err("a primitive's attributes have different counts".into());
            }
            let indices: Vec<usize> = match got.indices {
                Some(i) => i,
                None => (0..positions.len()).collect(),
            };
            let triangles = triangles(mode, &indices);
            if polygons + triangles.len() > MAX_IMPORT_POLYGONS {
                return Err(format!("more than {MAX_IMPORT_POLYGONS} triangles"));
            }
            polygons += triangles.len();
            let start = mesh.points.len();
            let n = positions.len();
            mesh.points.extend(positions);
            if let Some(n) = normals {
                mesh.normals.extend(n);
            }
            any_uvs |= uvs.is_some();
            mesh.uvs.extend(uvs.unwrap_or_else(|| vec![DVec2::ZERO; n]));
            any_colors |= colors.is_some();
            mesh.colors
                .extend(colors.unwrap_or_else(|| vec![[1.0; 3]; n]));
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
        if !any_uvs {
            mesh.uvs.clear();
        }
        if !any_colors {
            mesh.colors.clear();
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
    if sheared > 0 {
        warnings.push(format!(
            "{} the image (KHR_texture_transform) in a way a texture position cannot, so the \
             nearest turn and scale {} laid",
            counted(
                sheared,
                "material's texture transform shears or mirrors",
                "materials' texture transforms shear or mirror"
            ),
            if sheared == 1 { "was" } else { "were" },
        ));
    }
    let mut scene = ReadScene {
        meshes,
        materials,
        nodes,
        roots,
        unit: 1.0,
        up: FileUp::Y,
        v_down: true,
        skipped,
        warnings,
    };
    paint_vertex_colors(&mut scene);
    Ok(scene)
}

/// `EXT_meshopt_compression` on a buffer view.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeshoptView {
    buffer: usize,
    #[serde(default)]
    byte_offset: u64,
    byte_length: u64,
    byte_stride: u64,
    count: u64,
    mode: String,
    #[serde(default)]
    filter: Option<String>,
}

const MESHOPT: &str = "EXT_meshopt_compression";

/// The buffer views that are meshopt compressed, by view index.
fn meshopt_views(doc: &Gltf) -> Result<Vec<(usize, MeshoptView)>, String> {
    let mut out = Vec::new();
    for (i, v) in doc.buffer_views.iter().enumerate() {
        if let Some(ext) = v.extensions.get(MESHOPT) {
            let m: MeshoptView = serde_json::from_value(ext.clone())
                .map_err(|e| format!("buffer view {i} has a broken {MESHOPT} ({e})"))?;
            out.push((i, m));
        }
    }
    Ok(out)
}

/// Each compressed view's bytes, decoded, with its stride. Fails, saying why, for a view
/// outside its buffer, an unknown mode or filter, a broken stream, or more than
/// `MAX_IMPORT_BYTES` of output in all.
fn decode_views(
    buffers: &[Vec<u8>],
    views: &[(usize, MeshoptView)],
) -> Result<HashMap<usize, (Vec<u8>, u64)>, String> {
    let mut out = HashMap::new();
    let mut total = 0u64;
    for (i, m) in views {
        let buffer = buffers.get(m.buffer).ok_or(format!(
            "buffer view {i} names a compressed buffer that does not exist"
        ))?;
        let end = m
            .byte_offset
            .checked_add(m.byte_length)
            .filter(|e| *e <= buffer.len() as u64)
            .ok_or(format!(
                "buffer view {i}'s compressed data runs past its buffer"
            ))?;
        let mode = match m.mode.as_str() {
            "ATTRIBUTES" => meshopt::Mode::Attributes,
            "TRIANGLES" => meshopt::Mode::Triangles,
            "INDICES" => meshopt::Mode::Indices,
            other => {
                return Err(format!(
                    "buffer view {i} has an unknown {MESHOPT} mode {other}"
                ));
            }
        };
        let filter = match m.filter.as_deref() {
            None | Some("NONE") => meshopt::Filter::None,
            Some("OCTAHEDRAL") => meshopt::Filter::Octahedral,
            Some("QUATERNION") => meshopt::Filter::Quaternion,
            Some("EXPONENTIAL") => meshopt::Filter::Exponential,
            Some(other) => {
                return Err(format!(
                    "buffer view {i} has an unknown {MESHOPT} filter {other}"
                ));
            }
        };
        if m.byte_stride == 0 || m.byte_stride > 256 {
            return Err(format!(
                "buffer view {i} has a compressed stride outside 1-256"
            ));
        }
        let size = m
            .count
            .checked_mul(m.byte_stride)
            .filter(|s| total.saturating_add(*s) <= MAX_IMPORT_BYTES)
            .ok_or(format!(
                "the compressed views decode to over {MAX_IMPORT_BYTES} bytes"
            ))?;
        total += size;
        let bytes = meshopt::decode(
            mode,
            filter,
            &buffer[m.byte_offset as usize..end as usize],
            m.count as usize,
            m.byte_stride as usize,
        )
        .map_err(|e| format!("buffer view {i}: {e}"))?;
        out.insert(*i, (bytes, m.byte_stride));
    }
    Ok(out)
}

/// Each buffer's bytes: the `.glb`'s BIN chunk for a first buffer without a URI, else what
/// its URI names. A fallback buffer (what only uncompressed copies of compressed views use)
/// is left empty and unread. Fails for a missing or short buffer, or more than
/// `MAX_IMPORT_BYTES` in all.
fn read_buffers(
    doc: &Gltf,
    bin: Option<&[u8]>,
    side: &SideFiles,
    compressed: &[(usize, MeshoptView)],
) -> Result<Vec<Vec<u8>>, String> {
    let storage: HashSet<usize> = compressed.iter().map(|(_, m)| m.buffer).collect();
    let parents: HashSet<usize> = compressed
        .iter()
        .map(|(i, _)| doc.buffer_views[*i].buffer)
        .collect();
    let plain_users: HashSet<usize> = doc
        .buffer_views
        .iter()
        .filter(|v| !v.extensions.contains_key(MESHOPT))
        .map(|v| v.buffer)
        .collect();
    let mut out = Vec::with_capacity(doc.buffers.len());
    let mut total = 0u64;
    for (i, b) in doc.buffers.iter().enumerate() {
        let flagged = b
            .extensions
            .get(MESHOPT)
            .and_then(|e| e.get("fallback"))
            .and_then(Value::as_bool)
            == Some(true);
        if flagged || (parents.contains(&i) && !storage.contains(&i) && !plain_users.contains(&i)) {
            out.push(Vec::new());
            continue;
        }
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
    /// Decoded compressed views and their strides, by view index.
    decoded: &'a HashMap<usize, (Vec<u8>, u64)>,
}

/// What a primitive gives: positions, and the normals, texture coordinates, colors (linear)
/// and corner indices it has.
struct PrimitiveData {
    positions: Vec<DVec3>,
    normals: Option<Vec<DVec3>>,
    uvs: Option<Vec<DVec2>>,
    colors: Option<Vec<[f64; 3]>>,
    indices: Option<Vec<usize>>,
}

impl Data<'_> {
    /// The bytes of buffer view `v`, checked against its buffer.
    fn view(&self, v: usize) -> Result<(&[u8], Option<u64>), String> {
        if let Some((bytes, stride)) = self.decoded.get(&v) {
            return Ok((bytes, Some(*stride)));
        }
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

    /// Accessor `a`'s colors (RGB, linear): floats or normalized unsigned integers as a
    /// VEC3 or VEC4. None for a color accessor of another kind.
    fn colors(&self, a: usize) -> Result<Option<Vec<[f64; 3]>>, String> {
        let Some(acc) = self.doc.accessors.get(a) else {
            return Err(format!("accessor {a} does not exist"));
        };
        let width = match acc.kind.as_str() {
            "VEC3" => 3,
            "VEC4" => 4,
            _ => return Ok(None),
        };
        if !(acc.component_type == 5126
            || acc.normalized && matches!(acc.component_type, 5121 | 5123))
        {
            return Ok(None);
        }
        let values = self.read(a, width, MAX_IMPORT_POINTS as u64)?;
        Ok(Some(
            values
                .chunks_exact(width)
                .map(|c| [c[0], c[1], c[2]])
                .collect(),
        ))
    }

    /// A primitive's data: through its accessors, or decoded from its Draco stream. Normals
    /// are read when `normals`, texture coordinates of `set` when it is given, `COLOR_0`
    /// when it is there.
    fn primitive(
        &self,
        p: &GPrimitive,
        mode: u32,
        normals: bool,
        set: Option<usize>,
    ) -> Result<PrimitiveData, String> {
        let uv_name = set.map(|s| format!("TEXCOORD_{s}"));
        if let Some(ext) = p.extensions.get("KHR_draco_mesh_compression") {
            if mode != 4 {
                return Err(
                    "a Draco primitive that is not a triangle list is not supported".into(),
                );
            }
            return self.draco(p, ext, normals, uv_name.as_deref());
        }
        let positions = self.vec3(p.attributes["POSITION"])?;
        let normals = match (normals, p.attributes.get("NORMAL")) {
            (true, Some(&n)) => Some(self.vec3(n)?),
            _ => None,
        };
        let uvs = match uv_name.as_ref().and_then(|n| p.attributes.get(n)) {
            Some(&t) => Some(self.vec2(t)?),
            None => None,
        };
        let colors = match p.attributes.get("COLOR_0") {
            Some(&c) => self.colors(c)?,
            None => None,
        };
        let indices = match p.indices {
            Some(i) => Some(self.indices(i)?),
            None => None,
        };
        Ok(PrimitiveData {
            positions,
            normals,
            uvs,
            colors,
            indices,
        })
    }

    /// A primitive decoded from its `KHR_draco_mesh_compression` stream: the stream's
    /// triangles, and the attributes the extension names, in the accessors' units.
    fn draco(
        &self,
        p: &GPrimitive,
        ext: &Value,
        normals: bool,
        uv_name: Option<&str>,
    ) -> Result<PrimitiveData, String> {
        let view = ext
            .get("bufferView")
            .and_then(Value::as_u64)
            .ok_or("a Draco primitive names no buffer view")? as usize;
        let ids = ext
            .get("attributes")
            .and_then(Value::as_object)
            .ok_or("a Draco primitive names no attributes")?;
        let (bytes, _) = self.view(view)?;
        // The attributes to take, and the accessor each is read in the units of.
        let mut wanted = Vec::new();
        let mut names = Vec::new();
        for name in ["POSITION", "NORMAL", "COLOR_0"].into_iter().chain(uv_name) {
            if name == "NORMAL" && !normals {
                continue;
            }
            let (Some(id), Some(&acc)) = (
                ids.get(name).and_then(Value::as_u64),
                p.attributes.get(name),
            ) else {
                continue;
            };
            let acc = self
                .doc
                .accessors
                .get(acc)
                .ok_or(format!("accessor {acc} does not exist"))?;
            // Each attribute is read in the element type glTF gives it; an accessor of
            // another type is left out, as `read` would refuse it.
            let width = match (name, acc.kind.as_str()) {
                ("POSITION" | "NORMAL" | "COLOR_0", "VEC3") => 3,
                ("COLOR_0", "VEC4") => 4,
                (_, "VEC2") if name.starts_with("TEXCOORD_") => 2,
                _ => continue,
            };
            if name == "COLOR_0"
                && !(acc.component_type == 5126
                    || acc.normalized && matches!(acc.component_type, 5121 | 5123))
            {
                continue;
            }
            wanted.push(draco::Wanted {
                id: u32::try_from(id).map_err(|_| "a Draco attribute id is too large")?,
                width,
                normalized: acc.normalized,
            });
            names.push(name);
        }
        if !names.contains(&"POSITION") {
            return Err("a Draco primitive has no position attribute".into());
        }
        let decoded = draco::decode(bytes, &wanted).map_err(|e| format!("a Draco stream: {e}"))?;
        let mut values: HashMap<&str, (Vec<f64>, usize)> = names
            .into_iter()
            .zip(
                decoded
                    .values
                    .into_iter()
                    .zip(wanted.iter().map(|w| w.width)),
            )
            .collect();
        let vec3 = |v: Vec<f64>| -> Vec<DVec3> {
            v.as_chunks::<3>()
                .0
                .iter()
                .map(|c| DVec3::new(c[0], c[1], c[2]))
                .collect()
        };
        let positions = vec3(values.remove("POSITION").map(|v| v.0).unwrap_or_default());
        let normals = values.remove("NORMAL").map(|v| vec3(v.0));
        let uvs = uv_name.and_then(|n| values.remove(n)).map(|(v, _)| {
            v.as_chunks::<2>()
                .0
                .iter()
                .map(|c| DVec2::new(c[0], c[1]))
                .collect()
        });
        let colors = values
            .remove("COLOR_0")
            .map(|(v, width)| v.chunks_exact(width).map(|c| [c[0], c[1], c[2]]).collect());
        Ok(PrimitiveData {
            positions,
            normals,
            uvs,
            colors,
            indices: Some(decoded.indices),
        })
    }

    /// Image `i`'s file bytes, from its buffer view or URI, or why they could not be read.
    fn image(
        &self,
        i: usize,
        side: &SideFiles,
        budget: &mut ImageBudget,
    ) -> Result<Vec<u8>, &'static str> {
        let image = self.doc.images.get(i).ok_or(IMAGE_MISSING)?;
        if let Some(v) = image.buffer_view {
            let (bytes, _) = self.view(v).map_err(|_| IMAGE_MISSING)?;
            return budget.keep(bytes);
        }
        let uri = image.uri.as_deref().ok_or(IMAGE_MISSING)?;
        budget.read(side, uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_read::{IMAGE_OVERSIZE, IMAGES_TOO_LARGE, MAX_SCENE_IMAGE_BYTES};

    #[test]
    fn many_images_naming_one_large_file_do_not_each_hold_a_copy() {
        let dir = std::env::temp_dir().join(format!("gltf-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = 10 * 1024 * 1024;
        std::fs::write(dir.join("big.png"), vec![0u8; big]).unwrap();
        let n = 20;
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "images": vec![serde_json::json!({"uri": "big.png"}); n],
            "textures": (0..n).map(|i| serde_json::json!({"source": i})).collect::<Vec<_>>(),
            "materials": (0..n)
                .map(|i| serde_json::json!({"pbrMetallicRoughness": {"baseColorTexture": {"index": i}}}))
                .collect::<Vec<_>>(),
        });
        let side = SideFiles {
            folder: Some(&dir),
            roots: None,
        };
        let scene = read_gltf(json.to_string().as_bytes(), &side).unwrap();
        let kept: u64 = scene
            .materials
            .iter()
            .filter_map(|m| m.image.as_ref()?.as_ref().ok())
            .map(|b| b.len() as u64)
            .sum();
        assert!(kept <= MAX_SCENE_IMAGE_BYTES, "{kept}");
        assert_eq!(kept, 12 * big as u64);
        assert_eq!(
            scene.materials[n - 1].image,
            Some(Err(IMAGES_TOO_LARGE)),
            "past the budget"
        );
        std::fs::remove_dir_all(&dir).unwrap();

        // An image in the file's own buffer over the texture limit is not copied.
        let bytes = vec![0u8; 17 * 1024 * 1024];
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": bytes.len()}],
            "bufferViews": [{"buffer": 0, "byteLength": bytes.len()}],
            "images": [{"bufferView": 0}],
            "textures": [{"source": 0}],
            "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
        })
        .to_string();
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut glb = Vec::new();
        let total = 12 + 8 + json.len() + 8 + bytes.len();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        glb.extend_from_slice(&bytes);
        let memory = SideFiles {
            folder: None,
            roots: None,
        };
        let scene = read_gltf(&glb, &memory).unwrap();
        assert_eq!(scene.materials[0].image, Some(Err(IMAGE_OVERSIZE)));
    }
}
