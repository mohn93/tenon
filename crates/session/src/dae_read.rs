//! Reading COLLADA (`.dae`) files as a scene: geometries (triangles, polylists, polygons with
//! holes, fans and strips) with their normals and texture coordinates, materials with a
//! diffuse color or texture, the visual scene's node tree, library nodes placed by
//! `instance_node` (components), the file's unit and up axis. Animations, controllers,
//! cameras and lights are left out and named in the warnings.

use std::collections::HashMap;

use glam::{DMat4, DVec2, DVec3, DVec4};
use roxmltree::{Document, Node, NodeId, ParsingOptions};

use crate::import::{MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS};
use crate::scene_read::{
    FileUp, IMAGE_MISSING, ImageBudget, MAX_SCENE_DEPTH, MAX_SCENE_ITEMS, MAX_SCENE_NODES,
    ReadScene, SceneMaterial, SceneMesh, SceneNode, ScenePolygon, SideFiles, counted,
};

/// Most XML nodes (elements and text) a COLLADA file may have. Geometry lies in the text of
/// a few elements, so real files hold far fewer; each node takes tens of bytes once parsed.
const MAX_XML_NODES: u32 = 4_000_000;

/// Most corners one polygon may have.
const MAX_CORNERS: usize = 100_000;

/// Highest input offset a primitive may have (indices per corner, less one).
const MAX_INPUT_OFFSET: usize = 63;

/// The element's name without its namespace.
fn tag<'a>(n: &Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn children<'a, 'i>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &'a str) -> Option<Node<'a, 'i>> {
    children(n, name).next()
}

/// The numbers in an element's text, or a message for one that is not a finite number.
fn numbers(n: Node) -> Result<Vec<f64>, String> {
    n.text()
        .unwrap_or("")
        .split_whitespace()
        .map(|t| {
            t.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(|| format!("<{}> holds something that is not a number", tag(&n)))
        })
        .collect()
}

/// The whole numbers in an element's text.
fn indices(n: Node) -> Result<Vec<usize>, String> {
    n.text()
        .unwrap_or("")
        .split_whitespace()
        .map(|t| {
            t.parse::<usize>()
                .map_err(|_| format!("<{}> holds something that is not an index", tag(&n)))
        })
        .collect()
}

/// A `#id` reference's id.
fn reference<'a>(n: Node<'a, '_>, attribute: &str) -> Option<&'a str> {
    n.attribute(attribute)
        .map(|u| u.strip_prefix('#').unwrap_or(u))
}

/// Reads a COLLADA file's `bytes`, its images through `side`. Fails, saying why, for a file
/// that is not COLLADA XML, has data that does not fit, or passes the limits.
pub(crate) fn read_dae(bytes: &[u8], side: &SideFiles) -> Result<ReadScene, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not a COLLADA file (not UTF-8)")?;
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_XML_NODES,
        ..ParsingOptions::default()
    };
    let xml = Document::parse_with_options(text, options)
        .map_err(|e| format!("not a COLLADA file ({e})"))?;
    let root = xml.root_element();
    if tag(&root) != "COLLADA" {
        return Err("not a COLLADA file".into());
    }
    let mut by_id: HashMap<&str, Node> = HashMap::new();
    for n in xml.descendants().filter(Node::is_element) {
        if let Some(id) = n.attribute("id") {
            by_id.entry(id).or_insert(n);
        }
    }
    let asset = child(root, "asset");
    let unit = asset
        .and_then(|a| child(a, "unit"))
        .and_then(|u| u.attribute("meter"))
        .map_or(Ok(1.0), |m| {
            m.trim()
                .parse::<f64>()
                .ok()
                .filter(|m| m.is_finite() && *m > 0.0)
                .ok_or("the file's unit is not a positive length")
        })?;
    let up = match asset
        .and_then(|a| child(a, "up_axis"))
        .and_then(|u| u.text())
        .map(str::trim)
    {
        Some("Z_UP") => FileUp::Z,
        Some("X_UP") => FileUp::X,
        _ => FileUp::Y,
    };

    let mut r = Reader {
        by_id,
        side,
        scene: ReadScene {
            meshes: Vec::new(),
            materials: Vec::new(),
            nodes: Vec::new(),
            roots: Vec::new(),
            unit,
            up,
            v_down: false,
            skipped: 0,
            warnings: Vec::new(),
        },
        material_index: HashMap::new(),
        mesh_index: HashMap::new(),
        node_index: HashMap::new(),
        images: HashMap::new(),
        sources: HashMap::new(),
        budget: ImageBudget::default(),
        points: 0,
        polygons: 0,
        lines: 0,
        unread: 0,
        cameras: 0,
        lights: 0,
        controllers: 0,
    };
    let visual = child(root, "scene")
        .and_then(|s| child(s, "instance_visual_scene"))
        .and_then(|i| reference(i, "url"))
        .and_then(|id| r.by_id.get(id).copied())
        .or_else(|| child(root, "library_visual_scenes").and_then(|l| child(l, "visual_scene")));
    if let Some(visual) = visual {
        for n in children(visual, "node") {
            let i = r.node(n, false, 1)?;
            r.scene.roots.push(i);
        }
    }
    let animations = root
        .children()
        .filter(|c| tag(c) == "library_animations")
        .flat_map(|l| children(l, "animation"))
        .count();
    for (n, one, many) in [
        (animations, "animation was", "animations were"),
        (
            r.controllers,
            "skin or morph controller was",
            "skin and morph controllers were",
        ),
        (r.cameras, "camera was", "cameras were"),
        (r.lights, "light was", "lights were"),
        (r.lines, "set of lines was", "sets of lines were"),
        (
            r.unread,
            "part could not be read and was",
            "parts could not be read and were",
        ),
    ] {
        if n > 0 {
            r.scene
                .warnings
                .push(format!("{} not imported", counted(n, one, many)));
        }
    }
    Ok(r.scene)
}

/// A geometry instance's material binding: each symbol with its scene material, sorted.
type Binding = Vec<(String, Option<usize>)>;

struct Reader<'a, 'i, 's> {
    by_id: HashMap<&'a str, Node<'a, 'i>>,
    side: &'s SideFiles<'s>,
    scene: ReadScene,
    /// The scene material made for each `<material>` id.
    material_index: HashMap<&'a str, Option<usize>>,
    /// The scene mesh made for each geometry with its material binding.
    mesh_index: HashMap<(NodeId, Binding), usize>,
    /// The scene node made for each `<node>`.
    node_index: HashMap<NodeId, usize>,
    /// Each image id's file bytes, or why they could not be read.
    images: HashMap<&'a str, Result<Vec<u8>, &'static str>>,
    /// Each `<source>`'s numbers and stride, read once however many geometries use it.
    sources: HashMap<NodeId, (Vec<f64>, usize)>,
    /// What the images may still hold.
    budget: ImageBudget,
    points: usize,
    polygons: usize,
    lines: usize,
    unread: usize,
    cameras: usize,
    lights: usize,
    controllers: usize,
}

impl<'a, 'i> Reader<'a, 'i, '_> {
    /// The scene node of `<node>` `n` (made once per element), `shared` when placed by an
    /// `instance_node`.
    fn node(&mut self, n: Node<'a, 'i>, shared: bool, depth: usize) -> Result<usize, String> {
        if let Some(&i) = self.node_index.get(&n.id()) {
            self.scene.nodes[i].shared |= shared;
            return Ok(i);
        }
        if depth > MAX_SCENE_DEPTH {
            return Err(format!("nodes are nested more than {MAX_SCENE_DEPTH} deep"));
        }
        if self.scene.nodes.len() >= MAX_SCENE_NODES {
            return Err(format!("more than {MAX_SCENE_NODES} nodes"));
        }
        let index = self.scene.nodes.len();
        self.node_index.insert(n.id(), index);
        self.scene.nodes.push(SceneNode {
            name: n
                .attribute("name")
                .or(n.attribute("id"))
                .unwrap_or("")
                .to_string(),
            transform: transform(n)?,
            meshes: Vec::new(),
            children: Vec::new(),
            shared,
        });
        for c in n.children().filter(Node::is_element) {
            match tag(&c) {
                "node" => {
                    let i = self.node(c, false, depth + 1)?;
                    self.scene.nodes[index].children.push(i);
                }
                "instance_node" => {
                    let target = reference(c, "url").and_then(|id| self.by_id.get(id).copied());
                    match target.filter(|t| tag(t) == "node") {
                        Some(t) => {
                            let i = self.node(t, true, depth + 1)?;
                            self.scene.nodes[index].children.push(i);
                        }
                        None => self.unread += 1,
                    }
                }
                "instance_geometry" => {
                    if let Some(m) = self.instance_geometry(c)? {
                        self.scene.nodes[index].meshes.push(m);
                    }
                }
                "instance_camera" => self.cameras += 1,
                "instance_light" => self.lights += 1,
                "instance_controller" => self.controllers += 1,
                _ => {}
            }
        }
        Ok(index)
    }

    /// The scene mesh an `<instance_geometry>` places: its geometry with its material
    /// binding, made once per geometry and binding. None for a geometry that is missing or
    /// not a mesh.
    fn instance_geometry(&mut self, g: Node<'a, 'i>) -> Result<Option<usize>, String> {
        let Some(geometry) = reference(g, "url").and_then(|id| self.by_id.get(id).copied()) else {
            self.unread += 1;
            return Ok(None);
        };
        let Some(mesh) = child(geometry, "mesh") else {
            self.unread += 1;
            return Ok(None);
        };
        let mut binding: Binding = Vec::new();
        if let Some(tc) = child(g, "bind_material").and_then(|b| child(b, "technique_common")) {
            for im in children(tc, "instance_material") {
                let (Some(symbol), Some(target)) =
                    (im.attribute("symbol"), reference(im, "target"))
                else {
                    continue;
                };
                let m = self.material(target)?;
                binding.push((symbol.to_string(), m));
            }
        }
        binding.sort();
        let key = (geometry.id(), binding);
        if let Some(&m) = self.mesh_index.get(&key) {
            return Ok(Some(m));
        }
        if self.scene.meshes.len() >= MAX_SCENE_ITEMS {
            return Err(format!("more than {MAX_SCENE_ITEMS} meshes"));
        }
        let name = geometry
            .attribute("name")
            .or(geometry.attribute("id"))
            .unwrap_or("");
        let built = self.mesh(mesh, name, &key.1)?;
        self.scene.meshes.push(built);
        let m = self.scene.meshes.len() - 1;
        self.mesh_index.insert(key, m);
        Ok(Some(m))
    }

    /// The scene material of `<material>` `id` (made once), None when missing.
    fn material(&mut self, id: &str) -> Result<Option<usize>, String> {
        if let Some(&m) = self.material_index.get(id) {
            return Ok(m);
        }
        let Some((&key, &node)) = self.by_id.get_key_value(id) else {
            return Ok(None);
        };
        if tag(&node) != "material" {
            self.material_index.insert(key, None);
            return Ok(None);
        }
        if self.scene.materials.len() >= MAX_SCENE_ITEMS {
            return Err(format!("more than {MAX_SCENE_ITEMS} materials"));
        }
        let effect = child(node, "instance_effect")
            .and_then(|e| reference(e, "url"))
            .and_then(|id| self.by_id.get(id).copied());
        let (color, opacity, image) = match effect {
            Some(e) => self.effect(e),
            None => ([204, 204, 204], 1.0, None),
        };
        self.scene.materials.push(SceneMaterial {
            name: node
                .attribute("name")
                .or(node.attribute("id"))
                .unwrap_or("")
                .to_string(),
            color,
            opacity,
            image,
        });
        let m = Some(self.scene.materials.len() - 1);
        self.material_index.insert(key, m);
        Ok(m)
    }

    /// An effect's diffuse color (sRGB, as COLLADA writers give it), opacity and diffuse
    /// texture image.
    #[allow(clippy::type_complexity)]
    fn effect(
        &mut self,
        effect: Node<'a, 'i>,
    ) -> ([u8; 3], f32, Option<Result<Vec<u8>, &'static str>>) {
        let shading = child(effect, "profile_COMMON")
            .and_then(|p| child(p, "technique"))
            .and_then(|t| {
                t.children()
                    .find(|c| matches!(tag(c), "lambert" | "phong" | "blinn" | "constant"))
            });
        let Some(shading) = shading else {
            return ([204, 204, 204], 1.0, None);
        };
        let byte = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut color = [204, 204, 204];
        let mut image = None;
        if let Some(diffuse) = child(shading, "diffuse") {
            if let Some(c) = child(diffuse, "color").and_then(|c| numbers(c).ok())
                && c.len() >= 3
            {
                color = [byte(c[0]), byte(c[1]), byte(c[2])];
            }
            if let Some(t) = child(diffuse, "texture").and_then(|t| t.attribute("texture")) {
                image = Some(self.texture_image(effect, t));
            }
        }
        // A_ONE (the default): opacity is the transparent color's alpha times transparency.
        let mut opacity = 1.0;
        if let Some(transparent) = child(shading, "transparent")
            && let Some(c) = child(transparent, "color").and_then(|c| numbers(c).ok())
        {
            let factor = child(shading, "transparency")
                .and_then(|t| child(t, "float"))
                .and_then(|f| numbers(f).ok())
                .and_then(|f| f.first().copied())
                .unwrap_or(1.0);
            let alpha = if transparent.attribute("opaque") == Some("RGB_ZERO") {
                1.0 - factor * (c.iter().take(3).sum::<f64>() / 3.0)
            } else {
                c.get(3).copied().unwrap_or(1.0) * factor
            };
            opacity = alpha.clamp(0.0, 1.0) as f32;
        }
        (color, opacity, image)
    }

    /// The image a diffuse `<texture texture="...">` shows: through the effect's sampler and
    /// surface parameters (COLLADA 1.4), or an image id named straight away.
    fn texture_image(
        &mut self,
        effect: Node<'a, 'i>,
        sampler: &str,
    ) -> Result<Vec<u8>, &'static str> {
        let param = |sid: &str| {
            effect
                .descendants()
                .find(|n| tag(n) == "newparam" && n.attribute("sid") == Some(sid))
        };
        let mut image_id: Option<&'a str> = None;
        if let Some(s) = param(sampler) {
            let surface = s
                .descendants()
                .find(|n| tag(n) == "source")
                .and_then(|n| n.text())
                .map(str::trim);
            let from_surface = surface
                .and_then(param)
                .and_then(|p| p.descendants().find(|n| tag(n) == "init_from"))
                .and_then(|n| n.text())
                .map(str::trim);
            // COLLADA 1.5: <sampler2D><instance_image url="#id"/>.
            let from_instance = s
                .descendants()
                .find(|n| tag(n) == "instance_image")
                .and_then(|n| reference(n, "url"));
            image_id = from_surface.or(from_instance);
        }
        let id = image_id.unwrap_or(sampler);
        let Some((&key, &image)) = self.by_id.get_key_value(id) else {
            return Err(IMAGE_MISSING);
        };
        if let Some(read) = self.images.get(key) {
            return read.clone();
        }
        let uri = image
            .descendants()
            .find(|n| tag(n) == "init_from")
            .and_then(|n| {
                // COLLADA 1.5 puts the URI in <init_from><ref>.
                child(n, "ref").and_then(|r| r.text()).or(n.text())
            })
            .map(str::trim);
        let read = match uri {
            None => Err(IMAGE_MISSING),
            Some(uri) => self.budget.read(self.side, uri),
        };
        self.images.insert(key, read.clone());
        read
    }

    /// A `<mesh>`'s polygons as a scene mesh: a point per distinct (position, normal,
    /// texture coordinate) its corners use, the materials as `binding` maps their symbols.
    fn mesh(
        &mut self,
        mesh: Node<'a, 'i>,
        name: &str,
        binding: &[(String, Option<usize>)],
    ) -> Result<SceneMesh, String> {
        let mut out = SceneMesh {
            name: name.to_string(),
            ..SceneMesh::default()
        };
        let parts: Vec<Node> = mesh.children().filter(Node::is_element).collect();
        let polygonal = |n: &Node| {
            matches!(
                tag(n),
                "triangles" | "polylist" | "polygons" | "trifans" | "tristrips"
            )
        };
        let all_normals = parts
            .iter()
            .filter(|p| polygonal(p))
            .all(|p| self.input(*p, "NORMAL").is_some());
        let any_uvs = parts
            .iter()
            .filter(|p| polygonal(p))
            .any(|p| self.input(*p, "TEXCOORD").is_some());
        let mut point_of: HashMap<(usize, Option<usize>, Option<usize>), usize> = HashMap::new();
        // Taken out while the corners borrow it, and put back for the next mesh.
        let mut sources = std::mem::take(&mut self.sources);
        for part in parts {
            if matches!(tag(&part), "lines" | "linestrips") {
                self.lines += 1;
                continue;
            }
            if !polygonal(&part) {
                continue;
            }
            let Some((position, p_offset)) = self.input(part, "VERTEX") else {
                self.unread += 1;
                continue;
            };
            let normal = self.input(part, "NORMAL").filter(|_| all_normals);
            let uv = self.input(part, "TEXCOORD");
            let most = part
                .descendants()
                .filter(|n| tag(n) == "input")
                .filter_map(|n| n.attribute("offset")?.parse::<usize>().ok())
                .max()
                .unwrap_or(0);
            if most > MAX_INPUT_OFFSET {
                return Err(format!("an input's offset is over {MAX_INPUT_OFFSET}"));
            }
            let stride = most + 1;
            let material = part.attribute("material").and_then(|symbol| {
                binding
                    .iter()
                    .find(|(s, _)| s == symbol)
                    .map(|(_, m)| *m)
                    .unwrap_or_else(|| self.material(symbol).ok().flatten())
            });
            let source = |n: Node<'a, 'i>, sources: &mut HashMap<NodeId, (Vec<f64>, usize)>| {
                if let std::collections::hash_map::Entry::Vacant(e) = sources.entry(n.id()) {
                    e.insert(read_source(n)?);
                }
                Ok::<(), String>(())
            };
            source(position, &mut sources)?;
            if let Some((n, _)) = normal {
                source(n, &mut sources)?;
            }
            if let Some((n, _)) = uv {
                source(n, &mut sources)?;
            }
            // Each polygon's loops as corner tuples.
            let loops = loops(part, stride)?;
            for polygon in loops {
                if self.polygons >= MAX_IMPORT_POLYGONS {
                    return Err(format!("more than {MAX_IMPORT_POLYGONS} polygons"));
                }
                self.polygons += 1;
                let mut rings: Vec<Vec<usize>> = Vec::new();
                let mut bad = false;
                for ring in polygon {
                    let mut ids = Vec::with_capacity(ring.len());
                    for corner in ring.chunks_exact(stride) {
                        let pick = |(src, off): (Node, usize), width: usize| {
                            let (values, s) = &sources[&src.id()];
                            let i = corner[off];
                            let at = i.checked_mul(*s)?;
                            let v = values.get(at..at.checked_add(width)?)?;
                            Some((i, v.to_vec()))
                        };
                        let Some((pi, p)) = pick((position, p_offset), 3) else {
                            bad = true;
                            break;
                        };
                        let n = normal.and_then(|n| pick(n, 3));
                        let t = uv.and_then(|t| pick(t, 2));
                        if normal.is_some() && n.is_none() {
                            bad = true;
                            break;
                        }
                        let key = (pi, n.as_ref().map(|n| n.0), t.as_ref().map(|t| t.0));
                        let id = match point_of.get(&key) {
                            Some(&id) => id,
                            None => {
                                if self.points >= MAX_IMPORT_POINTS {
                                    return Err(format!("more than {MAX_IMPORT_POINTS} positions"));
                                }
                                self.points += 1;
                                out.points.push(DVec3::new(p[0], p[1], p[2]));
                                if all_normals && let Some((_, n)) = &n {
                                    out.normals.push(DVec3::new(n[0], n[1], n[2]));
                                }
                                if any_uvs {
                                    out.uvs.push(
                                        t.map_or(DVec2::ZERO, |(_, t)| DVec2::new(t[0], t[1])),
                                    );
                                }
                                point_of.insert(key, out.points.len() - 1);
                                out.points.len() - 1
                            }
                        };
                        ids.push(id);
                    }
                    if bad {
                        break;
                    }
                    rings.push(ids);
                }
                if bad || rings.is_empty() {
                    self.scene.skipped += 1;
                    continue;
                }
                let corners = rings.remove(0);
                out.polygons.push(ScenePolygon {
                    corners,
                    holes: rings,
                    material,
                });
            }
        }
        if out.normals.len() != out.points.len() {
            out.normals.clear();
        }
        self.sources = sources;
        Ok(out)
    }

    /// A primitive's input of `semantic` (VERTEX: the position source the `<vertices>` name),
    /// as its source element and offset; for TEXCOORD the lowest set. A NORMAL or TEXCOORD
    /// given in `<vertices>` counts with the VERTEX offset.
    fn input(&self, part: Node<'a, 'i>, semantic: &str) -> Option<(Node<'a, 'i>, usize)> {
        let inputs: Vec<Node> = children(part, "input").collect();
        let offset_of = |n: &Node| {
            n.attribute("offset")
                .and_then(|o| o.parse().ok())
                .unwrap_or(0)
        };
        let source = |n: &Node| reference(*n, "source").and_then(|id| self.by_id.get(id).copied());
        let vertex = inputs
            .iter()
            .find(|n| n.attribute("semantic") == Some("VERTEX"));
        let via_vertices = |want: &str| {
            let v = vertex?;
            let vertices = source(v)?;
            let i = children(vertices, "input").find(|n| n.attribute("semantic") == Some(want))?;
            Some((source(&i)?, offset_of(v)))
        };
        match semantic {
            "VERTEX" => via_vertices("POSITION"),
            _ => {
                let direct = inputs
                    .iter()
                    .filter(|n| n.attribute("semantic") == Some(semantic))
                    .min_by_key(|n| {
                        n.attribute("set")
                            .and_then(|s| s.parse::<u32>().ok())
                            .unwrap_or(0)
                    })
                    .and_then(|n| Some((source(n)?, offset_of(n))));
                direct.or_else(|| via_vertices(semantic))
            }
        }
    }
}

/// A `<source>`'s numbers and the stride between its elements.
fn read_source(source: Node) -> Result<(Vec<f64>, usize), String> {
    let array = child(source, "float_array").ok_or("a source has no float_array")?;
    let values = numbers(array)?;
    let accessor = child(source, "technique_common").and_then(|t| child(t, "accessor"));
    let stride = accessor
        .and_then(|a| a.attribute("stride"))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let offset = accessor
        .and_then(|a| a.attribute("offset"))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    Ok((values.get(offset..).unwrap_or(&[]).to_vec(), stride))
}

/// A primitive's polygons, each its loops (the outer one first, then holes) of corner index
/// tuples (`stride` indices per corner): triangles in threes, polylists by their `vcount`,
/// each `<p>` of `<polygons>` one polygon and each `<ph>` one with its `<h>` holes, fans and
/// strips cut into triangles.
fn loops(part: Node, stride: usize) -> Result<Vec<Vec<Vec<usize>>>, String> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<Vec<Vec<usize>>>, polygon: Vec<Vec<usize>>| {
        out.push(polygon);
        if out.len() > MAX_IMPORT_POLYGONS {
            return Err(format!("more than {MAX_IMPORT_POLYGONS} polygons"));
        }
        Ok(())
    };
    let corners = |p: &[usize]| p.len() / stride;
    let corner = |p: &[usize], i: usize| p[i * stride..(i + 1) * stride].to_vec();
    match tag(&part) {
        "triangles" => {
            for p in children(part, "p") {
                let p = indices(p)?;
                for t in p.chunks_exact(3 * stride) {
                    push(&mut out, vec![t.to_vec()])?;
                }
            }
        }
        "polylist" => {
            let p = child(part, "p")
                .map(indices)
                .transpose()?
                .unwrap_or_default();
            let counts = child(part, "vcount")
                .map(indices)
                .transpose()?
                .unwrap_or_default();
            let mut at = 0usize;
            for n in counts {
                if n > MAX_CORNERS {
                    return Err(format!("a polygon has more than {MAX_CORNERS} corners"));
                }
                let end = at + n * stride;
                let Some(ring) = p.get(at..end) else {
                    return Err("a polylist's vcount asks for more indices than it has".into());
                };
                push(&mut out, vec![ring.to_vec()])?;
                at = end;
            }
        }
        "polygons" => {
            for c in part.children().filter(Node::is_element) {
                match tag(&c) {
                    "p" => push(&mut out, vec![indices(c)?])?,
                    "ph" => {
                        let mut rings = Vec::new();
                        if let Some(p) = child(c, "p") {
                            rings.push(indices(p)?);
                        }
                        for h in children(c, "h") {
                            rings.push(indices(h)?);
                        }
                        push(&mut out, rings)?;
                    }
                    _ => {}
                }
            }
        }
        "trifans" | "tristrips" => {
            let fan = tag(&part) == "trifans";
            for p in children(part, "p") {
                let p = indices(p)?;
                for i in 2..corners(&p) {
                    let (a, b, c) = if fan {
                        (corner(&p, 0), corner(&p, i - 1), corner(&p, i))
                    } else if i % 2 == 0 {
                        (corner(&p, i - 2), corner(&p, i - 1), corner(&p, i))
                    } else {
                        (corner(&p, i - 1), corner(&p, i - 2), corner(&p, i))
                    };
                    push(&mut out, vec![[a, b, c].concat()])?;
                }
            }
        }
        _ => {}
    }
    for polygon in &mut out {
        for ring in polygon.iter_mut() {
            if ring.len() / stride > MAX_CORNERS {
                return Err(format!("a polygon has more than {MAX_CORNERS} corners"));
            }
            ring.truncate(ring.len() / stride * stride);
        }
    }
    Ok(out)
}

/// A `<node>`'s transform: its `matrix` (row-major), `translate`, `rotate` (axis and degrees)
/// and `scale` elements composed in order.
fn transform(n: Node) -> Result<DMat4, String> {
    let mut m = DMat4::IDENTITY;
    for t in n.children().filter(Node::is_element) {
        let step = match tag(&t) {
            "matrix" => {
                let v = numbers(t)?;
                if v.len() != 16 {
                    return Err("a matrix does not have 16 numbers".into());
                }
                DMat4::from_cols_array(&v.try_into().expect("16 numbers")).transpose()
            }
            "translate" => match numbers(t)?[..] {
                [x, y, z] => DMat4::from_translation(DVec3::new(x, y, z)),
                _ => return Err("a translate does not have 3 numbers".into()),
            },
            "rotate" => match numbers(t)?[..] {
                [x, y, z, angle] => {
                    let axis = DVec3::new(x, y, z);
                    if axis.length() < 1e-12 {
                        DMat4::IDENTITY
                    } else {
                        DMat4::from_axis_angle(axis.normalize(), angle.to_radians())
                    }
                }
                _ => return Err("a rotate does not have 4 numbers".into()),
            },
            "scale" => match numbers(t)?[..] {
                [x, y, z] => DMat4::from_scale(DVec3::new(x, y, z)),
                _ => return Err("a scale does not have 3 numbers".into()),
            },
            _ => continue,
        };
        m *= step;
    }
    if !m.is_finite() || m.row(3) != DVec4::W {
        return Err("a node's transform is not finite or not affine".into());
    }
    Ok(m)
}
