//! The whole scene as a SketchUp 2017 (`.skp`) file, written by `skpwrite`: the root and
//! every definition the root reaches (each once, in its own coordinates), the materials,
//! guides, holes, soft edges and texture placements. Meters become inches. What a SketchUp
//! file cannot hold (glowing materials, curves as curves, empty groups) is reported.

use std::collections::{BTreeSet, HashMap, HashSet};

use glam::{DVec2, DVec3};
use kernel::{
    DefinitionId, DefinitionKind, Face, Guide, MaterialId, Model, Scene, back_texture_position,
};
use skpwrite::{Definition, Doc, Edge, Geometry, Instance, Material, Texture, UvPin};

use crate::command::{CommandError, ErrorKind};

const METERS_PER_INCH: f64 = 0.0254;

/// The longest name written, in UTF-16 units: the file holds 254, and a " 1234" suffix
/// that makes it unique must still fit.
const MAX_NAME_UNITS: usize = 240;

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
    /// What the file could not hold.
    pub warnings: Vec<String>,
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

    let ctx = Ctx {
        scene,
        materials: &material_index,
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
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "nothing to export: the model is empty",
        ));
    }
    stats.definitions = definitions.len();
    stats.materials = materials.len();

    let doc = Doc {
        materials,
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
        empty,
        "empty group or component was left out",
        "empty groups or components were left out",
    );
    Ok((bytes, stats))
}

/// The definitions placed from the root, at any depth, in id order.
fn reached(scene: &Scene) -> BTreeSet<DefinitionId> {
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
                }
            })
            .collect();

        let faces: Vec<skpwrite::Face> = m
            .faces()
            .map(|(_, f)| {
                let loops = std::iter::once(&f.outer)
                    .chain(&f.holes)
                    .map(|l| l.iter().map(|v| at(*v)).collect())
                    .collect();
                let points: Vec<DVec3> = f.outer.iter().filter_map(|v| m.position(*v)).collect();
                skpwrite::Face {
                    loops,
                    front: self.material(f.front),
                    back: self.material(f.back),
                    front_uv: self.pins(f, &points, f.front, false),
                    back_uv: self.pins(f, &points, f.back, true),
                }
            })
            .collect();

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
                })
            })
            .collect();

        let guides: Vec<skpwrite::Guide> = m
            .guides()
            .map(|(_, g)| match *g {
                Guide::Line { point, direction } => skpwrite::Guide::Line {
                    point: inches(point),
                    direction: direction.to_array(),
                },
                Guide::Point { position } => skpwrite::Guide::Point {
                    position: inches(position),
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

    /// Three pins fixing a side's texture where the face shows it: three corners of the
    /// outer loop that are not on one line, each with where it lies on the texture, in
    /// tiles. None for a side whose material has no texture.
    fn pins(
        &self,
        face: &Face,
        points: &[DVec3],
        material: Option<MaterialId>,
        back: bool,
    ) -> Option<[UvPin; 3]> {
        let texture = self.scene.material(material?)?.texture.as_ref()?;
        let size = DVec2::new(texture.width, texture.height);
        let [a, b, c] = spread_corners(points)?;
        Some([a, b, c].map(|p| {
            let front = face.texture_position(p);
            let uv = if back {
                back_texture_position(front)
            } else {
                front
            } / size;
            (inches(p), uv.to_array())
        }))
    }
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
