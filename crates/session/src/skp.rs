//! Reading SketchUp files (2021 and newer) into an import tree: every definition once, with
//! its faces (holes included), edges (soft, smooth and hidden ones soft), placements and
//! materials. Tags, scenes, images and hidden states are not carried; the notes say what was
//! left out.

use std::collections::{HashMap, HashSet};

use glam::{DAffine3, DMat3, DVec3};
use kernel::geom::vector_area;
use kernel::{
    DefinitionKind, ImportDefinition, ImportEdge, ImportFace, ImportGeometry, ImportInstance,
    ImportTree, MAX_TEXTURE_SIZE, MIN_DETERMINANT, MIN_TEXTURE_SIZE, Material, MaterialId,
    SceneChange, Texture, TreeReport,
};
use skp::container::{Container, Generation};
use skp::geom::{DefGeometry, RawInstance};
use skp::model::{MaterialDef, NO_MATERIAL, TextureLocation, edge_flag};
use skp::scene::{ReadOptions, read_model};
use skp::source::MemorySource;
use skp::zip::{ZipDirectory, read_entry_within};

use crate::command::{CommandError, ErrorKind};
use crate::import::{MAX_IMPORT_MATERIALS, MAX_IMPORT_POINTS};
use crate::texture::{MAX_TEXTURE_BYTES, check_total_pixels, dimensions, texture_from_bytes};
use crate::{Document, MAX_COORDINATE};

/// Meters per inch, SketchUp's internal unit.
const METERS_PER_INCH: f64 = 0.0254;

/// The refusal for files older than SketchUp 2021.
pub(crate) const LEGACY_MESSAGE: &str = "SketchUp 2020 and older files cannot be imported yet; \
     open the file in SketchUp 2021 or newer and save it again";

/// The name of the tag every SketchUp model has (shown as "Untagged").
const DEFAULT_TAG: &str = "Layer0";

/// The longest material name taken from a file, leaving room for a " 2" suffix.
const MAX_NAME: usize = 240;

/// What reading a SketchUp file found besides the tree, for the import report.
#[derive(Debug, Default)]
pub(crate) struct SkpNotes {
    /// Faces in the file's root and the definitions it places.
    pub polygons: usize,
    /// Faces whose outer loop could not be read.
    pub unread_faces: usize,
    /// Holes that could not be read and were dropped.
    pub unread_holes: usize,
    /// Edges whose ends could not be read.
    pub unread_edges: usize,
    pub hidden_faces: usize,
    pub hidden_instances: usize,
    pub images: usize,
    /// Instances of definitions the file does not have.
    pub missing: usize,
    /// Instances whose transform is not finite or is flat.
    pub flat: usize,
    pub tags: usize,
    pub scenes: usize,
    /// Materials used past `MAX_IMPORT_MATERIALS`, imported as no material.
    pub dropped_materials: usize,
    /// Textures made.
    pub textures: usize,
    /// The materials added to the document, each with the change that added it.
    pub added: Vec<(MaterialId, SceneChange)>,
    /// Sentences for textures that could not be read.
    pub texture_warnings: Vec<String>,
}

impl SkpNotes {
    /// The report's warnings: what the file has that the import left out or changed.
    pub fn warnings(&self, built: &TreeReport) -> Vec<String> {
        let mut out = Vec::new();
        let mut say = |n: usize, one: &str, many: &str| {
            if n == 1 {
                out.push(format!("1 {one}"));
            } else if n > 1 {
                out.push(format!("{n} {many}"));
            }
        };
        say(
            self.tags,
            "tag was not imported (tags are not supported yet)",
            "tags were not imported (tags are not supported yet)",
        );
        say(
            self.scenes,
            "scene was not imported",
            "scenes were not imported",
        );
        say(
            self.hidden_faces,
            "hidden face was imported visible",
            "hidden faces were imported visible",
        );
        say(
            self.hidden_instances,
            "hidden group or component was imported visible",
            "hidden groups or components were imported visible",
        );
        say(self.images, "image was skipped", "images were skipped");
        say(
            self.missing,
            "instance of a definition the file lacks was skipped",
            "instances of definitions the file lacks were skipped",
        );
        say(
            self.flat,
            "instance with a flat or broken placement was skipped",
            "instances with a flat or broken placement were skipped",
        );
        say(
            built.unused,
            "definition placed nowhere was not imported",
            "definitions placed nowhere were not imported",
        );
        say(
            self.unread_holes + built.holes_dropped,
            "face hole could not be read and was dropped",
            "face holes could not be read and were dropped",
        );
        say(
            built.split,
            "face was not flat or valid and was made from its outer loop (cut into triangles \
             where needed)",
            "faces were not flat or valid and were made from their outer loops (cut into \
             triangles where needed)",
        );
        say(
            self.unread_edges + built.skipped_edges,
            "edge could not be read and was skipped",
            "edges could not be read and were skipped",
        );
        say(
            built.hard,
            "soft, smooth or hidden edge not between two faces was imported as an ordinary edge",
            "soft, smooth or hidden edges not between two faces were imported as ordinary edges",
        );
        say(
            self.dropped_materials,
            &format!("material past the {MAX_IMPORT_MATERIALS} an import may add was left off"),
            &format!("materials past the {MAX_IMPORT_MATERIALS} an import may add were left off"),
        );
        out.extend(self.texture_warnings.iter().cloned());
        out
    }
}

/// Reads a SketchUp file's bytes into an import tree whose positions are meters times
/// `scale`, adding the materials its faces and instances use to `document`'s scene (listed
/// in the notes, so a caller can take them out again). A file from SketchUp 2020 or older is
/// `invalid_input`; a file that cannot be read is a `file` error (the message does not name
/// the file). Nothing is added on an error.
pub(crate) fn read_skp(
    bytes: Vec<u8>,
    scale: f64,
    document: &mut Document,
) -> Result<(ImportTree, SkpNotes), CommandError> {
    let file_error = |m: String| CommandError::new(ErrorKind::File, m);
    let src = MemorySource::new(bytes);
    let container = Container::open(&src).map_err(|e| file_error(e.to_string()))?;
    if container.generation == Generation::Legacy {
        return Err(CommandError::new(ErrorKind::InvalidInput, LEGACY_MESSAGE));
    }
    let model = read_model(&src, &container, &ReadOptions::default())
        .map_err(|e| file_error(e.to_string()))?;
    let index = model.index(&src, &container);
    let mut notes = SkpNotes {
        tags: index
            .layers
            .iter()
            .filter(|l| l.name != DEFAULT_TAG)
            .count(),
        scenes: index.scenes.len(),
        ..SkpNotes::default()
    };

    // Definitions by pid (the first of a repeated pid wins), and those the root reaches
    // through instances that are imported.
    let unit = METERS_PER_INCH * scale;
    let mut by_pid: HashMap<u64, usize> = HashMap::new();
    for (i, d) in model.definitions.iter().enumerate() {
        by_pid.entry(d.pid).or_insert(i);
    }
    let placed = |g: &DefGeometry| -> Vec<usize> {
        g.instances
            .iter()
            .filter(|i| !i.is_image && transform(i, unit).is_some())
            .filter_map(|i| by_pid.get(&i.def_pid).copied())
            .collect()
    };
    let mut reached = vec![false; model.definitions.len()];
    let mut stack = placed(&model.root);
    while let Some(d) = stack.pop() {
        if !std::mem::replace(&mut reached[d], true) {
            stack.extend(placed(&model.definitions[d].geom));
        }
    }
    let geometries = || {
        std::iter::once(&model.root).chain(
            model
                .definitions
                .iter()
                .zip(&reached)
                .filter(|(_, r)| **r)
                .map(|(d, _)| &d.geom),
        )
    };
    let points: usize = geometries().map(DefGeometry::vertices).sum();
    if points > MAX_IMPORT_POINTS {
        return Err(file_error(format!(
            "more than {MAX_IMPORT_POINTS} vertices in the placed definitions"
        )));
    }

    // Groups: definitions every placement of which is a group.
    let mut component = vec![false; model.definitions.len()];
    for i in geometries().flat_map(|g| &g.instances) {
        if let Some(&d) = by_pid.get(&i.def_pid)
            && !i.is_group
        {
            component[d] = true;
        }
    }

    let materials = add_materials(document, &src, &container, &index.materials, scale, {
        let mut used: HashSet<u32> = HashSet::new();
        for g in geometries() {
            used.extend(g.face_material.iter().chain(&g.face_back_material));
            used.extend(g.instances.iter().filter_map(instance_material));
        }
        used.remove(&NO_MATERIAL);
        used
    })?;
    notes.textures = materials.textures;
    notes.dropped_materials = materials.dropped;
    notes.texture_warnings = materials.warnings;
    notes.added = materials.added;

    let result = (|| {
        let root = geometry(&model.root, unit, &by_pid, &materials.ids, &mut notes)?;
        let mut definitions = Vec::with_capacity(model.definitions.len());
        for (i, d) in model.definitions.iter().enumerate() {
            let kind = if component[i] {
                DefinitionKind::Component
            } else {
                DefinitionKind::Group
            };
            let name = match d.name.trim() {
                "" if kind == DefinitionKind::Group => "Group".to_string(),
                "" => "Component".to_string(),
                n => n.to_string(),
            };
            // A definition the root does not reach is not built: it needs no geometry.
            let geometry = if reached[i] {
                geometry(&d.geom, unit, &by_pid, &materials.ids, &mut notes)?
            } else {
                ImportGeometry::default()
            };
            definitions.push(ImportDefinition {
                name,
                kind,
                geometry,
            });
        }
        Ok(ImportTree { definitions, root })
    })();
    match result {
        Ok(tree) => Ok((tree, notes)),
        Err(e) => {
            remove_materials(document, &notes.added);
            Err(e)
        }
    }
}

/// Takes out materials an import added.
pub(crate) fn remove_materials(document: &mut Document, added: &[(MaterialId, SceneChange)]) {
    document
        .scene_mut()
        .apply(&SceneChange::join(added.iter().map(|(_, c)| c.clone())).inverse());
}

/// The material an instance carries, as the file's material table numbers it.
fn instance_material(i: &RawInstance) -> Option<u32> {
    // The table's ids are the pids cut to 32 bits (larger ones saturate).
    (i.has_material && i.material < u64::from(NO_MATERIAL)).then_some(i.material as u32)
}

/// One mesh of the file in meters (`unit` per inch), counting what is left out in `notes`.
fn geometry(
    g: &DefGeometry,
    unit: f64,
    by_pid: &HashMap<u64, usize>,
    materials: &HashMap<u32, MaterialId>,
    notes: &mut SkpNotes,
) -> Result<ImportGeometry, CommandError> {
    let points: Vec<DVec3> = g
        .vertex_pos
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| DVec3::from_array(*p) * unit)
        .collect();
    check_positions(&points, unit)?;
    let material = |id: Option<&u32>| id.and_then(|id| materials.get(id).copied());

    let mut faces = Vec::with_capacity(g.faces());
    let mut ring: Vec<u32> = Vec::new();
    for f in 0..g.faces() {
        notes.polygons += 1;
        if g.face_flags.get(f).is_some_and(|flags| flags & 1 != 0) {
            notes.hidden_faces += 1;
        }
        let (Some(&first), Some(&end)) = (g.face_loop_ofs.get(f), g.face_loop_ofs.get(f + 1))
        else {
            notes.unread_faces += 1;
            continue;
        };
        let mut loops: Vec<Vec<usize>> = Vec::new();
        for l in first as usize..(end as usize).min(g.loops()) {
            if g.loop_ring(l, &mut ring) {
                loops.push(ring.iter().map(|&v| v as usize).collect());
            } else if loops.is_empty() {
                break;
            } else {
                notes.unread_holes += 1;
            }
        }
        if loops.is_empty() {
            notes.unread_faces += 1;
            continue;
        }
        // The front is the side the file's plane normal points to: wind the outer loop
        // counter-clockwise around it.
        if let Some(n) = g.face_normal.get(3 * f..3 * f + 3) {
            let outer: Vec<DVec3> = loops[0]
                .iter()
                .filter_map(|&v| points.get(v))
                .copied()
                .collect();
            if vector_area(&outer).dot(DVec3::new(n[0], n[1], n[2])) < 0.0 {
                loops[0].reverse();
            }
        }
        faces.push(ImportFace {
            loops,
            front: material(g.face_material.get(f)),
            back: material(g.face_back_material.get(f)),
        });
    }

    let mut edges = Vec::with_capacity(g.edges());
    for (e, ends) in g.edge_v.as_chunks::<2>().0.iter().enumerate() {
        if ends.contains(&u32::MAX) {
            notes.unread_edges += 1;
            continue;
        }
        let flags = g.edge_flags.get(e).copied().unwrap_or(0);
        edges.push(ImportEdge {
            a: ends[0] as usize,
            b: ends[1] as usize,
            soft: flags & (edge_flag::SOFT | edge_flag::SMOOTH | edge_flag::HIDDEN) != 0,
        });
    }

    let mut instances = Vec::with_capacity(g.instances.len());
    for i in &g.instances {
        if i.is_image {
            notes.images += 1;
            continue;
        }
        let Some(&definition) = by_pid.get(&i.def_pid) else {
            notes.missing += 1;
            continue;
        };
        let Some(transform) = transform(i, unit) else {
            notes.flat += 1;
            continue;
        };
        if i.flags & 1 != 0 {
            notes.hidden_instances += 1;
        }
        instances.push(ImportInstance {
            definition,
            transform,
            material: instance_material(i).and_then(|m| materials.get(&m).copied()),
        });
    }
    Ok(ImportGeometry {
        points,
        faces,
        edges,
        guides: Vec::new(),
        instances,
    })
}

/// A `file` error when a position is not finite or lies beyond `MAX_COORDINATE`.
fn check_positions(points: &[DVec3], unit: f64) -> Result<(), CommandError> {
    if points
        .iter()
        .any(|p| !p.is_finite() || p.abs().max_element() > MAX_COORDINATE)
    {
        return Err(CommandError::new(
            ErrorKind::File,
            format!(
                "a position is not finite or is beyond +/-{MAX_COORDINATE} m at scale {}",
                unit / METERS_PER_INCH
            ),
        ));
    }
    Ok(())
}

/// An instance's placement with its translation in meters (`unit` per inch), or None when
/// it is not finite, is flat (`|det| < MIN_DETERMINANT`) or moves beyond `MAX_COORDINATE`.
/// Read as `RawInstance::matrix4` does, in full precision: row-major rotation and scale terms
/// `m[0..9]`, translation `m[9..12]`, all divided by `m[12]` (when not zero).
fn transform(i: &RawInstance, unit: f64) -> Option<DAffine3> {
    let m = &i.m;
    let d = if m[12] != 0.0 { m[12] } else { 1.0 };
    let matrix3 = DMat3::from_cols(
        DVec3::new(m[0], m[3], m[6]) / d,
        DVec3::new(m[1], m[4], m[7]) / d,
        DVec3::new(m[2], m[5], m[8]) / d,
    );
    let translation = DVec3::new(m[9], m[10], m[11]) / d * unit;
    let t = DAffine3 {
        matrix3,
        translation,
    };
    (t.is_finite()
        && matrix3.determinant().abs() >= MIN_DETERMINANT
        && translation.abs().max_element() <= MAX_COORDINATE)
        .then_some(t)
}

/// The materials an import added, and how the file's material ids map to them.
struct Added {
    ids: HashMap<u32, MaterialId>,
    added: Vec<(MaterialId, SceneChange)>,
    textures: usize,
    dropped: usize,
    warnings: Vec<String>,
}

/// Adds one material per entry of `table` whose id is in `used` (in the table's order, the
/// first of a repeated id winning), named uniquely against the model's names; at most
/// `MAX_IMPORT_MATERIALS`, the rest counted in `dropped`. A texture that cannot be read
/// leaves the material its colour and a warning.
fn add_materials(
    document: &mut Document,
    src: &MemorySource,
    container: &Container,
    table: &[MaterialDef],
    scale: f64,
    used: HashSet<u32>,
) -> Result<Added, CommandError> {
    let mut out = Added {
        ids: HashMap::new(),
        added: Vec::new(),
        textures: 0,
        dropped: 0,
        warnings: Vec::new(),
    };
    let mut seen: HashSet<u32> = HashSet::new();
    let mut pixels = document.texture_pixels_besides(None);
    for m in table {
        if !used.contains(&m.id) || !seen.insert(m.id) {
            continue;
        }
        if out.added.len() == MAX_IMPORT_MATERIALS {
            out.dropped += 1;
            continue;
        }
        let base: String = match m.name.trim() {
            "" => "Material".to_string(),
            n => n.chars().take(MAX_NAME).collect(),
        };
        let name = document.scene().free_material_name(&base);
        let opacity = if m.opacity.is_finite() {
            m.opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let color = [m.rgba[0], m.rgba[1], m.rgba[2]];
        let texture = match &m.texture {
            None => None,
            Some(t) => match read_texture(src, container.zip.as_ref(), t, scale, pixels) {
                Ok(texture) => Some(texture),
                Err(why) => {
                    out.warnings.push(format!(
                        "the texture of material \"{name}\" was not imported: {why}"
                    ));
                    None
                }
            },
        };
        let material = |texture: Option<Texture>| Material {
            name: name.clone(),
            color,
            opacity,
            texture,
            emission: 0.0,
        };
        let had_texture = texture.is_some();
        let result = match document.scene_mut().add_material_from(material(texture)) {
            // Past the model's texture limit, say: keep the colour.
            Err(e) if had_texture => {
                out.warnings.push(format!(
                    "the texture of material \"{name}\" was not imported: {e}"
                ));
                document.scene_mut().add_material_from(material(None))
            }
            r => r,
        };
        match result {
            Ok((id, change)) => {
                if had_texture
                    && document
                        .scene()
                        .material(id)
                        .is_some_and(|m| m.texture.is_some())
                {
                    out.textures += 1;
                    let t = document
                        .scene()
                        .material(id)
                        .and_then(|m| m.texture.as_ref());
                    pixels += t.map_or(0, |t| u64::from(t.image.width) * u64::from(t.image.height));
                }
                out.ids.insert(m.id, id);
                out.added.push((id, change));
            }
            Err(e) => {
                remove_materials(document, &out.added);
                return Err(CommandError::new(
                    ErrorKind::File,
                    format!("material \"{name}\": {e}"),
                ));
            }
        }
    }
    Ok(out)
}

/// A material's texture from the file's zip, its tile size in meters times `scale` (each side
/// clamped to the texture size limits). Fails, saying why, for an image that is missing,
/// larger than `MAX_TEXTURE_BYTES`, not PNG or JPEG, too many pixels (`others` are held
/// already) or broken.
fn read_texture(
    src: &MemorySource,
    zip: Option<&ZipDirectory>,
    t: &skp::model::TextureRef,
    scale: f64,
    others: u64,
) -> Result<Texture, String> {
    let TextureLocation::ZipEntry(at) = t.location else {
        return Err("the image is not in the file's archive".into());
    };
    let entry = zip
        .and_then(|z| z.entries.get(at as usize))
        .ok_or("the image is missing from the file")?;
    if entry.uncompressed_size > MAX_TEXTURE_BYTES {
        return Err(format!(
            "the image is over {MAX_TEXTURE_BYTES} bytes, the most a texture may have"
        ));
    }
    let bytes = read_entry_within(src, entry, MAX_TEXTURE_BYTES)
        .map_err(|e| format!("the image cannot be read ({e})"))?;
    let (_, width, height) = dimensions(&bytes)?;
    check_total_pixels(others, u64::from(width) * u64::from(height))?;
    let [w, h] = t
        .tile_mm_drawable()
        .map(|mm| (mm / 1000.0 * scale).clamp(MIN_TEXTURE_SIZE, MAX_TEXTURE_SIZE));
    texture_from_bytes(bytes, w, h)
}
