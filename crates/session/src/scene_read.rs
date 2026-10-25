//! Scenes read from glTF and COLLADA files: meshes, materials and a tree of nodes placing
//! them, in the file's own coordinates, and how they become an import tree. A node becomes a
//! group; a mesh several nodes place, or a node several nodes place (COLLADA's library nodes),
//! becomes one component. The file's axes and units are baked into points and transforms, and
//! each mesh's polygons become faces as OBJ and STL polygons do (welded, coplanar neighbours
//! merged), with the interior edges of smooth surfaces soft.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use base64::Engine;
use glam::{DAffine3, DMat3, DMat4, DVec2, DVec3};
use kernel::geom::vector_area;
use kernel::{
    DefinitionKind, ImportDefinition, ImportFace, ImportGeometry, ImportInstance, ImportTree,
    MAX_TEXTURE_OFFSET, MAX_TEXTURE_SCALE, MAX_TEXTURE_SIZE, MIN_DETERMINANT, MIN_TEXTURE_SCALE,
    MIN_TEXTURE_SIZE, MaterialId, MeshPolygon, TexturePlacement, normal_angle, polygon_geometry,
    texture_position,
};

/// Most nodes a scene file may have.
pub(crate) const MAX_SCENE_NODES: usize = 100_000;

/// Most meshes, materials, images or buffers a scene file may have, each.
pub(crate) const MAX_SCENE_ITEMS: usize = 10_000;

/// Deepest nesting of nodes a scene file may have.
pub(crate) const MAX_SCENE_DEPTH: usize = 32;

/// The axis that points up in a scene file. The model is Z-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileUp {
    X,
    Y,
    Z,
}

impl FileUp {
    /// The turn taking file coordinates to model ones: Y-up (x, y, z) becomes (x, -z, y), as
    /// OBJ import does; X-up (COLLADA's: right is -y, up x) becomes (-y, -z, x).
    pub(crate) fn turn(self) -> DMat3 {
        match self {
            FileUp::Z => DMat3::IDENTITY,
            FileUp::Y => DMat3::from_cols(DVec3::X, DVec3::Z, -DVec3::Y),
            FileUp::X => DMat3::from_cols(DVec3::Z, -DVec3::X, -DVec3::Y),
        }
    }
}

/// A scene as read, in the file's coordinates.
#[derive(Clone, Debug)]
pub(crate) struct ReadScene {
    pub meshes: Vec<SceneMesh>,
    pub materials: Vec<SceneMaterial>,
    pub nodes: Vec<SceneNode>,
    /// The nodes the scene shows, placed in the wrapper.
    pub roots: Vec<usize>,
    /// Meters per file unit.
    pub unit: f64,
    pub up: FileUp,
    /// True when texture coordinates count v down from the image's top (glTF), false when up
    /// from its bottom (COLLADA).
    pub v_down: bool,
    /// Polygons the reader dropped (a corner index out of range).
    pub skipped: usize,
    /// What the file has that is not imported, as sentences.
    pub warnings: Vec<String>,
}

/// A mesh: polygons over points, each point with the file's normal and texture coordinates
/// when the file gives them.
#[derive(Clone, Debug, Default)]
pub(crate) struct SceneMesh {
    pub name: String,
    pub points: Vec<DVec3>,
    /// Parallel to `points`, or empty.
    pub normals: Vec<DVec3>,
    /// Parallel to `points`, or empty.
    pub uvs: Vec<DVec2>,
    pub polygons: Vec<ScenePolygon>,
}

/// A polygon: its outer loop and holes (corner indices into the mesh's points) and its
/// material (an index into the scene's materials).
#[derive(Clone, Debug, Default)]
pub(crate) struct ScenePolygon {
    pub corners: Vec<usize>,
    pub holes: Vec<Vec<usize>>,
    pub material: Option<usize>,
}

/// A material: a name, a color and opacity, and a texture image.
#[derive(Clone, Debug)]
pub(crate) struct SceneMaterial {
    pub name: String,
    /// sRGB.
    pub color: [u8; 3],
    pub opacity: f32,
    /// The image file's bytes, or why it could not be read.
    pub image: Option<Result<Vec<u8>, &'static str>>,
}

/// A node: placed by its parent with `transform`, holding meshes and placing other nodes.
#[derive(Clone, Debug)]
pub(crate) struct SceneNode {
    pub name: String,
    pub transform: DMat4,
    pub meshes: Vec<usize>,
    pub children: Vec<usize>,
    /// A node meant to be placed in several places (COLLADA's library nodes): a component.
    pub shared: bool,
}

/// Why a texture image was not read, as import warnings say it.
pub(crate) const IMAGE_MISSING: &str = "the image file could not be read";
pub(crate) const IMAGE_REFUSED: &str =
    "the image is not beside the file (an absolute path, a path out of its folder or a URL)";

/// Where a scene file's side files (buffers, images) are read from.
pub(crate) struct SideFiles<'a> {
    /// The folder of the file; None for a file given as bytes, which reads no side files.
    pub folder: Option<&'a Path>,
    /// Under `--api-root`, the folders a side file must lie in once resolved.
    pub roots: Option<&'a [PathBuf]>,
}

impl SideFiles<'_> {
    /// The bytes a URI names: a `data:` URI's own (base64 or percent-encoded), or a file
    /// beside the scene file named by a relative path (percent-decoded, `/` or `\` between
    /// folders) without `..` parts.
    /// Fails, saying why, for a URL or other scheme, an absolute path, a path climbing out of
    /// the folder, a file outside the roots, no folder, or a file larger than `limit`.
    pub fn read(&self, uri: &str, limit: u64) -> Result<Vec<u8>, String> {
        if let Some(data) = uri.strip_prefix("data:") {
            return data_uri(data, limit);
        }
        let scheme = uri
            .split_once(':')
            .is_some_and(|(s, _)| !s.is_empty() && !s.contains(['/', '\\']));
        if scheme {
            return Err("it names a URL, not a file beside this one".into());
        }
        let name = percent_decode(uri);
        if name.starts_with(['/', '\\']) || Path::new(&name).is_absolute() || name.contains('\0') {
            return Err("it names an absolute path, not a file beside this one".into());
        }
        if name.split(['/', '\\']).any(|part| part == "..") {
            return Err("it names a file outside this one's folder".into());
        }
        // Refused everywhere, so a file reads alike on every system: on Windows `C:x` is
        // relative to another folder and `a:b` names a stream.
        if let Some(why) = crate::fspath::windows_refusal(&name) {
            return Err(format!(
                "it names {why}, which may lie outside this file's folder"
            ));
        }
        let Some(folder) = self.folder else {
            return Err("a file read from memory has no folder to read it from".into());
        };
        // Files written on Windows may separate folders with backslashes.
        let path = folder.join(name.replace('\\', "/"));
        let path = match self.roots {
            None => path,
            Some(roots) => match crate::fspath::canonical(&path) {
                Ok(real) if roots.iter().any(|r| crate::fspath::starts_with(&real, r)) => real,
                Ok(_) => return Err("it lies outside the allowed folders".into()),
                Err(e) => return Err(e.to_string()),
            },
        };
        crate::document::read_limited(&path, limit)
    }
}

/// A `data:` URI's bytes (what follows `data:`): base64 after `;base64,`, else percent-decoded.
fn data_uri(data: &str, limit: u64) -> Result<Vec<u8>, String> {
    let Some((head, body)) = data.split_once(',') else {
        return Err("a data URI without a comma".into());
    };
    let too_large = || format!("a data URI holds over {limit} bytes");
    if head.ends_with(";base64") {
        if body.len() as u64 / 4 * 3 > limit + 3 {
            return Err(too_large());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(body.trim())
            .map_err(|_| "a data URI's base64 is broken".to_string())?;
        if bytes.len() as u64 > limit {
            return Err(too_large());
        }
        Ok(bytes)
    } else {
        let bytes = percent_decode_bytes(body);
        if bytes.len() as u64 > limit {
            return Err(too_large());
        }
        Ok(bytes)
    }
}

/// `%XX` escapes decoded, as text (a file name).
fn percent_decode(s: &str) -> String {
    String::from_utf8_lossy(&percent_decode_bytes(s)).into_owned()
}

/// `%XX` escapes decoded; anything else as it is.
fn percent_decode_bytes(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// A linear color component (0 to 1, clamped) as an sRGB byte.
pub(crate) fn srgb_byte(linear: f64) -> u8 {
    let l = if linear.is_finite() {
        linear.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let s = if l <= 0.003_130_8 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// A count and a noun as a phrase: "1 animation", "3 animations".
pub(crate) fn counted(n: usize, one: &str, many: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

/// What turning a scene into an import tree gives besides the tree.
#[derive(Clone, Debug, Default)]
pub(crate) struct TreeNotes {
    /// Polygons of the meshes the tree holds, each mesh once.
    pub polygons: usize,
    /// Polygons that made no face.
    pub skipped: usize,
}

/// How a scene becomes model geometry: the turn and scale taking file coordinates to model
/// ones, and the material each scene material became (None: unpainted) with its texture size
/// (meters along the image's width and height) when it has a texture.
pub(crate) struct Mapping<'a> {
    pub turn: DMat3,
    pub scale: f64,
    pub materials: &'a [Option<(MaterialId, Option<DVec2>)>],
    pub v_down: bool,
    pub merge: bool,
}

impl Mapping<'_> {
    /// The map from file coordinates to model ones.
    fn affine(&self) -> DAffine3 {
        DAffine3::from_mat3(self.turn * self.scale)
    }

    /// A node transform in file coordinates as one in model coordinates.
    fn conjugate(&self, t: &DMat4) -> DAffine3 {
        let m = self.affine();
        m * DAffine3::from_mat4(*t) * m.inverse()
    }
}

/// The meshes and nodes the roots reach (each once), in the order first reached. Fails for a
/// node that places itself or nesting deeper than `MAX_SCENE_DEPTH`.
pub(crate) fn reached(scene: &ReadScene) -> Result<(Vec<usize>, Vec<usize>), String> {
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut seen_nodes = HashSet::new();
    let mut seen_meshes = HashSet::new();
    // Depth-first with the path held, so a cycle is found.
    let mut on_path = vec![false; scene.nodes.len()];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for &root in &scene.roots {
        if on_path.get(root).is_none() {
            continue;
        }
        if !seen_nodes.insert(root) {
            continue;
        }
        nodes.push(root);
        stack.push((root, 0));
        on_path[root] = true;
        while let Some(top) = stack.last_mut() {
            let node = &scene.nodes[top.0];
            if top.1 == 0 {
                for &m in &node.meshes {
                    if m < scene.meshes.len() && seen_meshes.insert(m) {
                        meshes.push(m);
                    }
                }
            }
            match node.children.get(top.1).copied() {
                Some(child) => {
                    top.1 += 1;
                    if child >= scene.nodes.len() {
                        continue;
                    }
                    if on_path[child] {
                        return Err("a node places itself".into());
                    }
                    // A node reached before (off the path) has been walked already.
                    if !seen_nodes.insert(child) {
                        continue;
                    }
                    if stack.len() >= MAX_SCENE_DEPTH {
                        return Err(format!("nodes are nested more than {MAX_SCENE_DEPTH} deep"));
                    }
                    nodes.push(child);
                    on_path[child] = true;
                    stack.push((child, 0));
                }
                None => {
                    on_path[top.0] = false;
                    stack.pop();
                }
            }
        }
    }
    Ok((nodes, meshes))
}

/// The scene as an import tree (see the module's description). `mapping` gives each
/// material's model material. Fails with `TooManyFaces` (as a message) past the import face
/// limit, or for a node tree `reached` refuses.
pub(crate) fn build_tree(
    scene: &ReadScene,
    mapping: &Mapping,
) -> Result<(ImportTree, TreeNotes), String> {
    let (nodes, meshes) = reached(scene)?;
    // How many times each node is placed and each mesh held.
    let mut parents: HashMap<usize, usize> = HashMap::new();
    let mut holders: HashMap<usize, usize> = HashMap::new();
    for &n in &nodes {
        let node = &scene.nodes[n];
        for &c in &node.children {
            *parents.entry(c).or_default() += 1;
        }
        for &m in &node.meshes {
            *holders.entry(m).or_default() += 1;
        }
    }
    for &r in &scene.roots {
        *parents.entry(r).or_default() += 1;
    }
    let mut b = Builder {
        scene,
        mapping,
        tree: ImportTree::default(),
        notes: TreeNotes::default(),
        mesh_definition: HashMap::new(),
        node_definition: HashMap::new(),
        shared_mesh: meshes
            .iter()
            .copied()
            .filter(|m| holders.get(m).copied().unwrap_or(0) > 1)
            .collect(),
        shared_node: nodes
            .iter()
            .copied()
            .filter(|n| scene.nodes[*n].shared || parents.get(n).copied().unwrap_or(0) > 1)
            .collect(),
        polygons_seen: HashSet::new(),
    };
    // Roots holding nothing (a camera, a light) are left out.
    let roots: Vec<usize> = scene
        .roots
        .iter()
        .copied()
        .filter(|&r| {
            scene
                .nodes
                .get(r)
                .is_some_and(|n| !n.meshes.is_empty() || !n.children.is_empty())
        })
        .collect();
    // A scene of one node holding meshes only it holds, placing nothing and not mirrored goes
    // straight into the wrapper, its transform taken into the points.
    let flat = match roots[..] {
        [r] => {
            let node = &scene.nodes[r];
            node.children.is_empty()
                && !b.shared_node.contains(&r)
                && node.meshes.iter().all(|m| !b.shared_mesh.contains(m))
                && mapping.conjugate(&node.transform).matrix3.determinant() > MIN_DETERMINANT
        }
        _ => false,
    };
    let mut root = if flat {
        let node = &scene.nodes[roots[0]];
        b.geometry(&node.meshes, mapping.conjugate(&node.transform))?
    } else {
        let mut g = ImportGeometry::default();
        for &r in &roots {
            b.place(r, &mut g, 1)?;
        }
        g
    };
    if root.points.iter().any(|p| !p.is_finite()) {
        return Err("a position is not finite".into());
    }
    root.guides.clear();
    b.tree.root = root;
    Ok((b.tree, b.notes))
}

struct Builder<'a> {
    scene: &'a ReadScene,
    mapping: &'a Mapping<'a>,
    tree: ImportTree,
    notes: TreeNotes,
    /// The component made for each shared mesh.
    mesh_definition: HashMap<usize, usize>,
    /// The component made for each shared node.
    node_definition: HashMap<usize, usize>,
    shared_mesh: HashSet<usize>,
    shared_node: HashSet<usize>,
    /// Meshes whose polygons are counted.
    polygons_seen: HashSet<usize>,
}

impl Builder<'_> {
    /// Adds node `n`'s placement to `into`: an instance of its group, or of its component.
    /// A node holding nothing and placing nothing is left out.
    fn place(&mut self, n: usize, into: &mut ImportGeometry, depth: usize) -> Result<(), String> {
        let node = &self.scene.nodes[n];
        if node.meshes.is_empty() && node.children.is_empty() {
            return Ok(());
        }
        if depth > MAX_SCENE_DEPTH {
            return Err(format!("nodes are nested more than {MAX_SCENE_DEPTH} deep"));
        }
        // A node holding nothing but one placement of a shared node (how SketchUp writes a
        // component instance) is that placement, moved by both transforms.
        if let ([], [c]) = (&node.meshes[..], &node.children[..])
            && self.shared_node.contains(c)
            && !self.shared_node.contains(&n)
        {
            let combined = node.transform * self.scene.nodes[*c].transform;
            return self.place_as(*c, combined, into, depth + 1);
        }
        self.place_as(n, node.transform, into, depth)
    }

    /// Adds node `n`'s placement with `transform` (file coordinates) to `into`.
    fn place_as(
        &mut self,
        n: usize,
        transform: DMat4,
        into: &mut ImportGeometry,
        depth: usize,
    ) -> Result<(), String> {
        let node = &self.scene.nodes[n];
        if node.meshes.is_empty() && node.children.is_empty() {
            return Ok(());
        }
        if depth > MAX_SCENE_DEPTH {
            return Err(format!("nodes are nested more than {MAX_SCENE_DEPTH} deep"));
        }
        let transform = self.mapping.conjugate(&transform);
        // A node holding one shared mesh and nothing else is placed as the mesh's component.
        if let [m] = node.meshes[..]
            && node.children.is_empty()
            && self.shared_mesh.contains(&m)
            && !self.shared_node.contains(&n)
        {
            let definition = self.mesh_component(m)?;
            into.instances.push(instance(definition, transform));
            return Ok(());
        }
        let definition = match self.node_definition.get(&n) {
            Some(&d) => d,
            None => {
                let mut g = self.geometry_of(&node.meshes)?;
                for &c in &node.children {
                    if c < self.scene.nodes.len() {
                        self.place(c, &mut g, depth + 1)?;
                    }
                }
                let shared = self.shared_node.contains(&n);
                let kind = if shared {
                    DefinitionKind::Component
                } else {
                    DefinitionKind::Group
                };
                let fallback = if shared { "Component" } else { "Group" };
                self.tree.definitions.push(ImportDefinition {
                    name: name_or(&node.name, fallback),
                    kind,
                    geometry: g,
                });
                let d = self.tree.definitions.len() - 1;
                if shared {
                    self.node_definition.insert(n, d);
                }
                d
            }
        };
        into.instances.push(instance(definition, transform));
        Ok(())
    }

    /// The geometry of a node's meshes: those only it holds as faces, shared ones as
    /// instances of their components.
    fn geometry_of(&mut self, meshes: &[usize]) -> Result<ImportGeometry, String> {
        let (shared, own): (Vec<usize>, Vec<usize>) = meshes
            .iter()
            .copied()
            .filter(|&m| m < self.scene.meshes.len())
            .partition(|m| self.shared_mesh.contains(m));
        let mut g = self.geometry(&own, DAffine3::IDENTITY)?;
        for m in shared {
            let d = self.mesh_component(m)?;
            g.instances.push(instance(d, DAffine3::IDENTITY));
        }
        Ok(g)
    }

    /// The component made for shared mesh `m` (made on first use).
    fn mesh_component(&mut self, m: usize) -> Result<usize, String> {
        if let Some(&d) = self.mesh_definition.get(&m) {
            return Ok(d);
        }
        let g = self.geometry(&[m], DAffine3::IDENTITY)?;
        self.tree.definitions.push(ImportDefinition {
            name: name_or(&self.scene.meshes[m].name, "Component"),
            kind: DefinitionKind::Component,
            geometry: g,
        });
        let d = self.tree.definitions.len() - 1;
        self.mesh_definition.insert(m, d);
        Ok(d)
    }

    /// Meshes' polygons in model coordinates, moved by `place` (not a mirror), made into
    /// faces together.
    fn geometry(&mut self, meshes: &[usize], place: DAffine3) -> Result<ImportGeometry, String> {
        let affine = place * self.mapping.affine();
        let turn_normal = affine.matrix3.inverse().transpose();
        let mut points = Vec::new();
        let mut normals = Vec::new();
        let mut polygons = Vec::new();
        let mut textures = Vec::new();
        let mut holed: Vec<ImportFace> = Vec::new();
        let mut holed_points: Vec<DVec3> = Vec::new();
        let mut smooth = true;
        for &m in meshes {
            let mesh = &self.scene.meshes[m];
            if self.polygons_seen.insert(m) {
                self.notes.polygons += mesh.polygons.len();
            }
            smooth &= mesh.normals.len() == mesh.points.len();
        }
        for &m in meshes {
            let mesh = &self.scene.meshes[m];
            let start = points.len();
            points.extend(mesh.points.iter().map(|p| affine.transform_point3(*p)));
            if smooth {
                normals.extend(
                    mesh.normals
                        .iter()
                        .map(|n| turn_normal.mul_vec3(*n).normalize_or_zero()),
                );
            }
            for p in &mesh.polygons {
                let material = p
                    .material
                    .and_then(|i| self.mapping.materials.get(i).copied().flatten());
                let corners: Vec<DVec3> = p
                    .corners
                    .iter()
                    .filter_map(|&c| points.get(start + c).copied())
                    .collect();
                if corners.len() != p.corners.len() {
                    self.notes.skipped += 1;
                    continue;
                }
                let texture = material
                    .and_then(|(_, size)| size)
                    .filter(|_| mesh.uvs.len() == mesh.points.len())
                    .and_then(|size| {
                        let uvs: Vec<DVec2> = p.corners.iter().map(|&c| mesh.uvs[c]).collect();
                        placement(&corners, &uvs, size, self.mapping.v_down)
                    });
                if p.holes.is_empty() {
                    polygons.push(MeshPolygon {
                        corners: p.corners.iter().map(|c| start + c).collect(),
                        material: material.map(|(id, _)| id),
                    });
                    textures.push(texture);
                } else {
                    // A face with holes is made as given; it does not merge.
                    let first = holed_points.len();
                    let mut loops = Vec::new();
                    let mut ok = true;
                    for l in std::iter::once(&p.corners).chain(&p.holes) {
                        let mut ids = Vec::new();
                        for &c in l {
                            match points.get(start + c) {
                                Some(q) => {
                                    ids.push(holed_points.len());
                                    holed_points.push(*q);
                                }
                                None => ok = false,
                            }
                        }
                        loops.push(ids);
                    }
                    if !ok {
                        holed_points.truncate(first);
                        self.notes.skipped += 1;
                        continue;
                    }
                    holed.push(ImportFace {
                        loops,
                        front: material.map(|(id, _)| id),
                        back: None,
                        texture: texture.filter(|t| !t.is_default()),
                        tag: None,
                        hidden: false,
                    });
                }
            }
        }
        let (mut g, skipped) = polygon_geometry(
            &points,
            &polygons,
            &textures,
            if smooth { &normals } else { &[] },
            self.mapping.merge,
        )
        .map_err(|e| e.to_string())?;
        self.notes.skipped += skipped;
        let offset = g.points.len();
        g.points.extend(holed_points);
        for mut f in holed {
            for l in &mut f.loops {
                for c in l.iter_mut() {
                    *c += offset;
                }
            }
            g.faces.push(f);
        }
        Ok(g)
    }
}

fn instance(definition: usize, transform: DAffine3) -> ImportInstance {
    ImportInstance {
        definition,
        transform,
        material: None,
        tag: None,
        hidden: false,
    }
}

/// `name` trimmed and cut to 240 characters, or `fallback` when blank.
pub(crate) fn name_or(name: &str, fallback: &str) -> String {
    match name.trim() {
        "" => fallback.to_string(),
        n => n.chars().take(240).collect(),
    }
}

/// A polygon's unit normal by its vector area, or None when it has none.
fn polygon_normal(corners: &[DVec3]) -> Option<DVec3> {
    let n = vector_area(corners);
    (n.length() > 1e-12).then(|| n.normalize())
}

/// The map `q = o + B st` from a polygon's texture coordinates to its texture projection
/// (meters along the projection's u and v, see `texture_position`), fitted on its largest
/// corner triangle: `o` and B's columns (meters per repeat along the image's width and
/// height, v counted up the image). None when no triangle has area in both.
fn uv_map(corners: &[DVec3], uvs: &[DVec2], v_down: bool) -> Option<(DVec2, DVec2, DVec2)> {
    let normal = polygon_normal(corners)?;
    let q: Vec<DVec2> = corners
        .iter()
        .map(|p| texture_position(normal, *p))
        .collect();
    let st: Vec<DVec2> = uvs
        .iter()
        .map(|t| DVec2::new(t.x, if v_down { -t.y } else { t.y }))
        .collect();
    let mut best: Option<(f64, usize)> = None;
    for i in 1..corners.len().saturating_sub(1) {
        let area = (st[i] - st[0]).perp_dot(st[i + 1] - st[0]).abs();
        if area.is_finite() && best.is_none_or(|(a, _)| area > a) {
            best = Some((area, i));
        }
    }
    let (area, i) = best?;
    if area < 1e-12 {
        return None;
    }
    let (ds1, ds2) = (st[i] - st[0], st[i + 1] - st[0]);
    let (dq1, dq2) = (q[i] - q[0], q[i + 1] - q[0]);
    // B [ds1 ds2] = [dq1 dq2], so B = Q S^-1.
    let det = ds1.perp_dot(ds2);
    let inv = |a: DVec2, b: DVec2| (a * ds2.y - b * ds1.y) / det;
    let b1 = inv(dq1, dq2);
    let b2 = (dq2 * ds1.x - dq1 * ds2.x) / det;
    let o = q[0] - b1 * st[0].x - b2 * st[0].y;
    (o.is_finite() && b1.is_finite() && b2.is_finite()).then_some((o, b1, b2))
}

/// A texture's size fitted from a polygon's texture coordinates: the meters one repeat of
/// the image covers along its width and height, each clamped to the texture size limits.
pub(crate) fn texture_size(corners: &[DVec3], uvs: &[DVec2], v_down: bool) -> Option<DVec2> {
    let (_, b1, b2) = uv_map(corners, uvs, v_down)?;
    let (w, h) = (b1.length(), b2.length());
    (w > 0.0 && h > 0.0).then(|| {
        DVec2::new(
            w.clamp(MIN_TEXTURE_SIZE, MAX_TEXTURE_SIZE),
            h.clamp(MIN_TEXTURE_SIZE, MAX_TEXTURE_SIZE),
        )
    })
}

/// The texture placement that lays a texture of `size` on a polygon as its texture
/// coordinates say: the turn of the image's width direction, and the scale along width and
/// height (a sheared mapping takes the nearest; a mirrored one the mirror's size). None when
/// the coordinates have no area, or the placement is beyond the placement limits.
pub(crate) fn placement(
    corners: &[DVec3],
    uvs: &[DVec2],
    size: DVec2,
    v_down: bool,
) -> Option<TexturePlacement> {
    let (o, b1, b2) = uv_map(corners, uvs, v_down)?;
    // q = o + A t with t = st * size: A's columns are B's over the size.
    let (a1, a2) = (b1 / size.x, b2 / size.y);
    let s1 = a1.length();
    let turn = a1.y.atan2(a1.x);
    let r = DVec2::from_angle(turn);
    let s2 = a2.dot(r.perp()).abs();
    if !(s1.is_finite() && s2.is_finite() && s1 > 0.0 && s2 > 0.0) {
        return None;
    }
    let scale = [s1, s2];
    if scale
        .iter()
        .any(|s| !(MIN_TEXTURE_SCALE..=MAX_TEXTURE_SCALE).contains(s))
    {
        return None;
    }
    // The texture position at the projection's origin, brought within one repeat so the
    // offset stays small and neighbours mapped alike get the same one.
    let t0 = {
        let q = -o;
        let turned = DVec2::new(r.x * q.x + r.y * q.y, r.x * q.y - r.y * q.x);
        DVec2::new(turned.x / s1, turned.y / s2)
    };
    // Just short of a whole repeat is a whole repeat, so rounding does not split neighbours.
    let within = |x: f64, period: f64| {
        let r = x.rem_euclid(period);
        if period - r < 1e-6 * period { 0.0 } else { r }
    };
    let t0 = DVec2::new(within(t0.x, size.x), within(t0.y, size.y));
    let offset = -r.rotate(DVec2::new(t0.x * s1, t0.y * s2));
    if !offset.is_finite() || offset.abs().max_element() > MAX_TEXTURE_OFFSET {
        return None;
    }
    // Rounding off the default placement is snapped back to it, so it is left off.
    let snap = |x: f64, to: f64, tol: f64| if (x - to).abs() < tol { to } else { x };
    let rotation = normal_angle(turn.to_degrees());
    Some(TexturePlacement {
        offset: [
            snap(offset.x, 0.0, 1e-9) + 0.0,
            snap(offset.y, 0.0, 1e-9) + 0.0,
        ],
        rotation_deg: normal_angle(snap(snap(rotation, 0.0, 1e-6), 360.0, 1e-6)),
        scale: scale.map(|s| snap(s, 1.0, 1e-6)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_files_stay_beside_the_file() {
        let dir = std::env::temp_dir().join(format!("side-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a b.bin"), b"abc").unwrap();
        // Names Windows would read as another place, present so only the refusal stops them.
        for name in ["C:x.bin", "sub/a:b.bin", "sub/NUL.bin"] {
            std::fs::write(dir.join(name), b"abc").unwrap();
        }
        let side = SideFiles {
            folder: Some(&dir),
            roots: None,
        };
        assert_eq!(side.read("sub/a%20b.bin", 10).unwrap(), b"abc");
        assert_eq!(side.read("sub\\a b.bin", 10).unwrap(), b"abc");
        assert!(
            side.read("sub/a b.bin", 2)
                .unwrap_err()
                .contains("too large")
        );
        for refused in [
            "../x.bin",
            "sub/../../x.bin",
            "%2e%2e/x.bin",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "http://example.com/x.bin",
            "file:///etc/passwd",
            "C:/x.bin",
            // Drive-relative and alternate data streams once decoded, as Windows reads them.
            "C%3Ax.bin",
            "sub/a%3Ab.bin",
            "sub/NUL.bin",
        ] {
            assert!(side.read(refused, 10).is_err(), "{refused}");
        }
        assert_eq!(
            side.read("data:application/octet-stream;base64,YWJj", 10)
                .unwrap(),
            b"abc"
        );
        assert_eq!(side.read("data:,a%20b", 10).unwrap(), b"a b");
        assert_eq!(side.read("data:,%FF%00", 10).unwrap(), [0xFF, 0x00]);
        assert!(side.read("data:;base64,YWJjZGVm", 3).is_err());
        let memory = SideFiles {
            folder: None,
            roots: None,
        };
        assert!(
            memory
                .read("sub/a b.bin", 10)
                .unwrap_err()
                .contains("memory")
        );
        let roots = [dir.join("elsewhere")];
        let rooted = SideFiles {
            folder: Some(&dir),
            roots: Some(&roots),
        };
        assert!(rooted.read("sub/a b.bin", 10).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn y_and_x_up_turn_into_z_up() {
        let p = DVec3::new(1.0, 2.0, 3.0);
        assert_eq!(FileUp::Y.turn() * p, DVec3::new(1.0, -3.0, 2.0));
        assert_eq!(FileUp::X.turn() * p, DVec3::new(-2.0, -3.0, 1.0));
        assert_eq!(FileUp::Z.turn() * p, p);
        for up in [FileUp::X, FileUp::Y] {
            assert!(
                (up.turn().determinant() - 1.0).abs() < 1e-12,
                "a turn, not a mirror"
            );
        }
    }

    #[test]
    fn texture_placements_reproduce_the_coordinates() {
        // A sloping quad with coordinates turned 30 degrees and stretched along v.
        let corners = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(2.0, 1.0, 1.0),
            DVec3::new(0.0, 1.0, 1.0),
        ];
        let normal = polygon_normal(&corners).unwrap();
        let (sin, cos) = 30f64.to_radians().sin_cos();
        let uvs: Vec<DVec2> = corners
            .iter()
            .map(|p| {
                let q = texture_position(normal, *p);
                let turned = DVec2::new(cos * q.x + sin * q.y, cos * q.y - sin * q.x);
                DVec2::new(turned.x / 0.5 + 3.0, -(turned.y / 0.25) + 7.0)
            })
            .collect();
        let size = texture_size(&corners, &uvs, true).unwrap();
        assert!((size - DVec2::new(0.5, 0.25)).length() < 1e-9, "{size}");
        let p = placement(&corners, &uvs, size, true).unwrap();
        assert!((p.rotation_deg - 30.0).abs() < 1e-9, "{p:?}");
        assert!((p.scale[0] - 1.0).abs() < 1e-9 && (p.scale[1] - 1.0).abs() < 1e-9);
        // Each corner lands where its coordinates say, whole repeats aside.
        for (c, uv) in corners.iter().zip(&uvs) {
            let t = p.place(texture_position(normal, *c));
            let want = DVec2::new(uv.x * size.x, -uv.y * size.y);
            let d = t - want;
            let off = DVec2::new(d.x / size.x, d.y / size.y);
            assert!((off - off.round()).length() < 1e-9, "{t} {want}");
        }
        // Half the size: twice the scale.
        let q = placement(&corners, &uvs, size * 2.0, true).unwrap();
        assert!((q.scale[0] - 0.5).abs() < 1e-9, "{q:?}");
        // Coordinates without area give none.
        assert!(placement(&corners, &[DVec2::ZERO; 4], size, true).is_none());
    }
}
