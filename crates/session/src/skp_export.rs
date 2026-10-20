//! The whole scene as a SketchUp 2017 (`.skp`) file, written by `skpwrite`: the root and
//! every definition the root reaches (each once, in its own coordinates), the materials,
//! every tag (as layers), guides, holes, soft edges and texture placements.
//! Meters become inches. What a SketchUp file cannot hold (glowing materials, curves as
//! curves, empty groups) and what is not written (scenes) is reported.

use std::collections::{BTreeSet, HashMap, HashSet};

use glam::{DVec2, DVec3};
use kernel::{
    DefinitionId, DefinitionKind, Face, Guide, MaterialId, Model, Scene, TagId,
    back_texture_position,
};
use skpwrite::{Definition, Doc, Edge, Geometry, Instance, Material, Tag, Texture, UvPin};

use crate::command::{CommandError, ErrorKind};

const METERS_PER_INCH: f64 = 0.0254;

/// The longest name written, in UTF-16 units: the file holds 254, and a " 1234" suffix
/// that makes it unique must still fit (and, for a tag, the "Layer_" before it).
const MAX_NAME_UNITS: usize = 240;

/// The name of the layer every SketchUp file has, which Untagged entities are on.
const DEFAULT_LAYER: &str = "Layer0";

/// What an export to `.skp` wrote, for the export report.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SkpExportStats {
    /// Definitions written (each once, however often it is placed).
    pub definitions: usize,
    /// Instances written, over the root and every definition written.
    pub instances: usize,
    pub faces: usize,
    pub edges: usize,
    pub guides: usize,
    pub materials: usize,
    /// Tags written (every tag of the model).
    pub tags: usize,
    /// What the file could not hold.
    pub warnings: Vec<String>,
    /// Faces with a textured side whose placement could not be pinned, written with the
    /// default projection.
    unplaced_textures: usize,
}

/// The scene as a SketchUp 2017 file. `invalid_input` when there is nothing to write or the
/// writer refuses the scene.
pub(crate) fn encode_skp(scene: &Scene) -> Result<(Vec<u8>, SkpExportStats), CommandError> {
    let mut stats = SkpExportStats::default();

    // Materials, in id order, each named uniquely.
    let mut material_index: HashMap<MaterialId, usize> = HashMap::new();
    let mut materials = Vec::new();
    let mut taken = HashSet::new();
    let mut glowing = 0;
    for (id, m) in scene.materials() {
        if m.glows() {
            glowing += 1;
        }
        material_index.insert(id, materials.len());
        materials.push(Material {
            name: unique_name(&mut taken, &m.name, "Material"),
            rgba: [m.color[0], m.color[1], m.color[2], 255],
            opacity: f64::from(m.opacity),
            texture: m.texture.as_ref().map(|t| Texture {
                bytes: t.image.encoded.clone(),
                width: t.width / METERS_PER_INCH,
                height: t.height / METERS_PER_INCH,
            }),
        });
    }

    // The definitions the root reaches, and which of them have anything to write.
    let reached = reached(scene);
    let mut writable: HashMap<DefinitionId, bool> = HashMap::new();
    for &d in &reached {
        has_content(scene, d, &mut writable);
    }
    let written: Vec<DefinitionId> = reached
        .iter()
        .copied()
        .filter(|d| writable.get(d).copied().unwrap_or(false))
        .collect();
    let empty = reached.len() - written.len();
    let def_index: HashMap<DefinitionId, usize> =
        written.iter().enumerate().map(|(i, d)| (*d, i)).collect();

    // Every tag, used or not (SketchUp files keep unused layers), in id order, each named
    // uniquely (and not Layer0).
    let mut taken = HashSet::from([DEFAULT_LAYER.to_string()]);
    let mut tag_index: HashMap<TagId, usize> = HashMap::new();
    let mut tags = Vec::new();
    for (id, t) in scene.tags() {
        tag_index.insert(id, tags.len());
        tags.push(Tag {
            name: unique_name(&mut taken, &t.name, "Tag"),
            rgb: t.color,
            hidden: !t.visible,
        });
    }

    let ctx = Ctx {
        scene,
        materials: &material_index,
        tags: &tag_index,
        definitions: &def_index,
    };
    let mut curves = 0;
    let mut taken = HashSet::new();
    let mut definitions = Vec::with_capacity(written.len());
    for &d in &written {
        let Some(def) = scene.definition(d) else {
            continue;
        };
        let fallback = match def.kind {
            DefinitionKind::Group => "Group",
            DefinitionKind::Component => "Component",
        };
        let (geometry, c) = ctx.geometry(&def.mesh, &mut stats);
        curves += c;
        definitions.push(Definition {
            name: unique_name(&mut taken, &def.name, fallback),
            is_group: def.kind == DefinitionKind::Group,
            geometry,
        });
    }
    let (root, c) = ctx.geometry(scene.root(), &mut stats);
    curves += c;
    if root.edges.is_empty()
        && root.faces.is_empty()
        && root.instances.is_empty()
        && root.guides.is_empty()
    {
        let message = if scene.root().instances().next().is_some() {
            "nothing to export: only empty groups or components"
        } else {
            "nothing to export: the model is empty"
        };
        return Err(CommandError::new(ErrorKind::InvalidInput, message));
    }
    stats.definitions = definitions.len();
    stats.materials = materials.len();
    stats.tags = tags.len();

    let doc = Doc {
        materials,
        tags,
        definitions,
        root,
    };
    let bytes = skpwrite::write(&doc).map_err(|e| {
        CommandError::new(
            ErrorKind::InvalidInput,
            format!("cannot write a SketchUp file: {e}"),
        )
    })?;

    let mut warn = |n: usize, one: &str, many: &str| {
        if n > 0 {
            let text = if n == 1 { one } else { many };
            stats.warnings.push(format!("{n} {text}"));
        }
    };
    warn(
        glowing,
        "material glows; SketchUp files cannot store that",
        "materials glow; SketchUp files cannot store that",
    );
    warn(
        curves,
        "curve was written as separate edges",
        "curves were written as separate edges",
    );
    warn(
        stats.unplaced_textures,
        "textured face kept the default texture placement; its corners are too close to a line",
        "textured faces kept the default texture placement; their corners are too close to a line",
    );
    warn(
        empty,
        "empty group or component was left out",
        "empty groups or components were left out",
    );
    warn(
        scene.pages().count(),
        "scene was not written",
        "scenes were not written",
    );
    stats
        .warnings
        .extend(crate::export::annotation_warnings(scene));
    Ok((bytes, stats))
}

/// The definitions placed from the root, at any depth, in id order.
pub(crate) fn reached(scene: &Scene) -> BTreeSet<DefinitionId> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<DefinitionId> = scene
        .root()
        .instances()
        .map(|(_, i)| i.definition)
        .collect();
    while let Some(d) = stack.pop() {
        if !seen.insert(d) {
            continue;
        }
        if let Some(def) = scene.definition(d) {
            stack.extend(def.mesh.instances().map(|(_, i)| i.definition));
        }
    }
    seen
}

/// Whether a definition has anything to write: an edge, a face, a guide, or an instance of
/// a definition that has. Remembered in `memo`. The scene places no definition inside
/// itself, so the walk ends.
fn has_content(scene: &Scene, d: DefinitionId, memo: &mut HashMap<DefinitionId, bool>) -> bool {
    if let Some(&known) = memo.get(&d) {
        return known;
    }
    // Until worked out, count it empty: a loop (which a valid scene has not) then ends.
    memo.insert(d, false);
    let Some(def) = scene.definition(d) else {
        return false;
    };
    let m = &def.mesh;
    let own =
        m.edges().next().is_some() || m.faces().next().is_some() || m.guides().next().is_some();
    let placed: Vec<DefinitionId> = m.instances().map(|(_, i)| i.definition).collect();
    let content = own || placed.into_iter().any(|c| has_content(scene, c, memo));
    memo.insert(d, content);
    content
}

/// `name` trimmed (or `fallback` when blank), cut to `MAX_NAME_UNITS`, and followed by the
/// first free number from 2 when already taken.
fn unique_name(taken: &mut HashSet<String>, name: &str, fallback: &str) -> String {
    let base = match name.trim() {
        "" => fallback,
        n => n,
    };
    let mut cut = String::new();
    let mut units = 0;
    for c in base.chars() {
        units += c.len_utf16();
        if units > MAX_NAME_UNITS {
            break;
        }
        cut.push(c);
    }
    let name = if taken.contains(&cut) {
        (2..)
            .map(|n| format!("{cut} {n}"))
            .find(|n| !taken.contains(n))
            .unwrap_or(cut)
    } else {
        cut
    };
    taken.insert(name.clone());
    name
}

struct Ctx<'a> {
    scene: &'a Scene,
    materials: &'a HashMap<MaterialId, usize>,
    tags: &'a HashMap<TagId, usize>,
    definitions: &'a HashMap<DefinitionId, usize>,
}

impl Ctx<'_> {
    /// One mesh in inches, and how many curves it has. Adds what it writes to `stats`.
    fn geometry(&self, m: &Model, stats: &mut SkpExportStats) -> (Geometry, usize) {
        let mut index = HashMap::new();
        let mut vertices = Vec::new();
        for (id, v) in m.vertices() {
            // Past u32, an index the writer refuses as out of range.
            index.insert(id, u32::try_from(vertices.len()).unwrap_or(u32::MAX));
            vertices.push(inches(v.pos));
        }
        let at = |v| index.get(&v).copied().unwrap_or(u32::MAX);

        let mut curves = BTreeSet::new();
        let edges: Vec<Edge> = m
            .edges()
            .map(|(_, e)| {
                if let Some(c) = e.curve {
                    curves.insert(c);
                }
                Edge {
                    a: at(e.v[0]),
                    b: at(e.v[1]),
                    soft: e.soft,
                    smooth: e.soft,
                    hidden: false,
                    tag: self.tag(e.tag),
                }
            })
            .collect();

        let mut faces = Vec::new();
        for (_, f) in m.faces() {
            let loops = std::iter::once(&f.outer)
                .chain(&f.holes)
                .map(|l| l.iter().map(|v| at(*v)).collect())
                .collect();
            let points: Vec<DVec3> = f.outer.iter().filter_map(|v| m.position(*v)).collect();
            let front_uv = self.pins(f, &points, f.front, false);
            let back_uv = self.pins(f, &points, f.back, true);
            if matches!(front_uv, Pins::Unplaceable) || matches!(back_uv, Pins::Unplaceable) {
                stats.unplaced_textures += 1;
            }
            faces.push(skpwrite::Face {
                loops,
                front: self.material(f.front),
                back: self.material(f.back),
                front_uv: front_uv.into_option(),
                back_uv: back_uv.into_option(),
                tag: self.tag(f.tag),
            });
        }

        let instances: Vec<Instance> = m
            .instances()
            .filter_map(|(_, i)| {
                let definition = *self.definitions.get(&i.definition)?;
                let c = |j: usize| i.transform.matrix3.col(j);
                Some(Instance {
                    definition,
                    rotation: [0, 1, 2].map(|r| [c(0)[r], c(1)[r], c(2)[r]]),
                    translation: inches(i.transform.translation),
                    material: self.material(i.material),
                    tag: self.tag(i.tag),
                })
            })
            .collect();

        let guides: Vec<skpwrite::Guide> = m
            .guides()
            .map(|(_, g)| match *g {
                Guide::Line {
                    point,
                    direction,
                    tag,
                } => skpwrite::Guide::Line {
                    point: inches(point),
                    direction: direction.to_array(),
                    tag: self.tag(tag),
                },
                Guide::Point { position, tag } => skpwrite::Guide::Point {
                    position: inches(position),
                    tag: self.tag(tag),
                },
            })
            .collect();

        stats.faces += faces.len();
        stats.edges += edges.len();
        stats.instances += instances.len();
        stats.guides += guides.len();
        let g = Geometry {
            vertices,
            edges,
            faces,
            instances,
            guides,
        };
        (g, curves.len())
    }

    fn material(&self, m: Option<MaterialId>) -> Option<usize> {
        m.and_then(|m| self.materials.get(&m).copied())
    }

    fn tag(&self, t: Option<TagId>) -> Option<usize> {
        t.and_then(|t| self.tags.get(&t).copied())
    }

    /// Three pins fixing a side's texture where the face shows it: three corners of the
    /// outer loop that are not on one line, each with where it lies on the texture, in
    /// tiles. `Unplaceable` when no three corners are spread enough, on the face or on the
    /// texture (judged by shape, not size, as the writer does), so that one face keeps the
    /// default projection rather than failing the export.
    fn pins(
        &self,
        face: &Face,
        points: &[DVec3],
        material: Option<MaterialId>,
        back: bool,
    ) -> Pins {
        let Some(texture) = material
            .and_then(|m| self.scene.material(m))
            .and_then(|m| m.texture.as_ref())
        else {
            return Pins::Untextured;
        };
        let size = DVec2::new(texture.width, texture.height);
        let Some(corners) = spread_corners(points) else {
            return Pins::Unplaceable;
        };
        let pins = corners.map(|p| {
            let front = face.texture_position(p);
            let uv = if back {
                back_texture_position(front)
            } else {
                front
            } / size;
            (inches(p), uv.to_array())
        });
        let on_texture = pins.map(|(_, uv)| DVec2::from_array(uv) * size);
        if spread(on_texture) {
            Pins::Placed(pins)
        } else {
            Pins::Unplaceable
        }
    }
}

/// A face side's texture pins.
enum Pins {
    Untextured,
    Placed([UvPin; 3]),
    Unplaceable,
}

impl Pins {
    fn into_option(self) -> Option<[UvPin; 3]> {
        match self {
            Pins::Placed(p) => Some(p),
            _ => None,
        }
    }
}

/// True when three points are clearly not on one line: twice their triangle's area is more
/// than 1e-6 times the square of its longest side (the writer refuses below 1e-9).
fn spread([a, b, c]: [DVec2; 3]) -> bool {
    let twice_area = (b - a).perp_dot(c - a).abs();
    let longest = [b - a, c - a, c - b]
        .iter()
        .map(|s| s.length_squared())
        .fold(0.0, f64::max);
    twice_area.is_finite() && longest.is_finite() && twice_area > 1e-6 * longest
}

fn inches(p: DVec3) -> [f64; 3] {
    (p / METERS_PER_INCH).to_array()
}

/// Three of `points` far from lying on one line: the first, the farthest from it, and the
/// farthest from the line through those two. None when all lie on one line.
fn spread_corners(points: &[DVec3]) -> Option<[DVec3; 3]> {
    let a = *points.first()?;
    let b = points
        .iter()
        .copied()
        .max_by(|p, q| a.distance_squared(*p).total_cmp(&a.distance_squared(*q)))?;
    let along = b - a;
    let c = points.iter().copied().max_by(|p, q| {
        along
            .cross(*p - a)
            .length_squared()
            .total_cmp(&along.cross(*q - a).length_squared())
    })?;
    (along.cross(c - a).length() > 1e-12).then_some([a, b, c])
}
