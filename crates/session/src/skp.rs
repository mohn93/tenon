//! Reading SketchUp files (2017 and newer) into an import tree: every definition once, with
//! its faces (holes included), edges (soft and smooth ones soft, hidden ones hidden),
//! placements, guides, hidden faces, groups, components and guides (kept hidden), materials,
//! tags (the file's layers) and, from SketchUp 2021 and newer files, scenes. Images are not
//! carried; the notes say what was left out. Files from SketchUp
//! 2017 to 2020 (the legacy container) are read by `skp::legacy` into the same shape as newer
//! ones, without their scenes.

use std::collections::{HashMap, HashSet};

use glam::{DAffine3, DMat3, DVec3};
use kernel::geom::vector_area;
use kernel::{
    DefinitionKind, Guide, ImportDefinition, ImportEdge, ImportFace, ImportGeometry,
    ImportInstance, ImportTree, MAX_TEXTURE_SIZE, MIN_DETERMINANT, MIN_TEXTURE_SIZE, Material,
    MaterialId, PAGE_NAME_BASE, Page, PageView, SceneChange, Tag, TagId, Texture, TreeReport,
    UNTAGGED,
};
use skp::container::{Container, Generation};
use skp::geom::{DefGeometry, NO_LAYER, RawGuide, RawInstance};
use skp::layers::color_is_unset;
use skp::legacy;
use skp::model::{
    INCH_TO_MM, LayerDef, MaterialDef, NO_MATERIAL, SceneDef, SceneIndex, TextureLocation,
    edge_flag,
};
use skp::pages::page_flag;
use skp::scene::{Model, ReadOptions, read_model};
use skp::source::ByteSource;
use skp::zip::{ZipDirectory, read_entry_within};

use crate::camera::Projection;
use crate::command::{CommandError, ErrorKind};
use crate::import::{MAX_IMPORT_MATERIALS, MAX_IMPORT_POINTS};
use crate::texture::{MAX_TEXTURE_BYTES, check_total_pixels, dimensions, texture_from_bytes};
use crate::{Document, MAX_COORDINATE};

/// Meters per inch, SketchUp's internal unit.
const METERS_PER_INCH: f64 = 0.0254;

/// The refusal for files older than SketchUp 2017.
const OLD_MESSAGE: &str = "SketchUp files older than 2017 are not supported";

/// The oldest SketchUp version whose files are read (the major version in the file's header).
const OLDEST_VERSION: u32 = 17;

/// The name of the tag every SketchUp model has (shown as "Untagged").
const DEFAULT_TAG: &str = "Layer0";

/// Most tags one import adds; what is on the file's further layers is imported Untagged.
const MAX_IMPORT_TAGS: usize = 1024;

/// Most scenes one import adds; the file's further scenes are left off.
const MAX_IMPORT_SCENES: usize = 1024;

/// The most a file's model (`model.dat`, or a 2017-2020 file's model stream) may unpack to.
const MAX_MODEL_BYTES: u64 = 512 * 1024 * 1024;

/// The most vertices, edges, faces, loops, edge uses and instances a file may hold, over all
/// its definitions (placed or not): far past what the import limits let through, and a
/// bound on what reading costs.
const MAX_ELEMENTS: u64 = 10_000_000;

/// The longest material, tag or definition name taken from a file, leaving room for a " 2"
/// suffix.
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
    /// Guides that could not be read, or are not finite, have no direction or lie beyond
    /// `MAX_COORDINATE`: skipped.
    pub skipped_guides: usize,
    /// Entities of a 2017-2020 file that could not be read (after a damaged record).
    pub lost_entities: usize,
    /// Faces of a 2017-2020 file read short of their holes.
    pub faces_short: usize,
    /// Dimensions and text labels of a 2017-2020 file, which are not imported (a newer
    /// file's are not read, so not counted).
    pub annotations: usize,
    pub images: usize,
    /// Instances of definitions the file does not have.
    pub missing: usize,
    /// Instances whose transform is not finite or is flat.
    pub flat: usize,
    /// The file's layers past `MAX_IMPORT_TAGS`, not imported.
    pub dropped_tags: usize,
    /// Scenes not imported (a 2017-2020 file's, whose scenes are not read).
    pub scenes: usize,
    /// Imported scenes whose camera could not be used (an eye on its target, or none read
    /// though the scene restores one): they store no view.
    pub unusable_views: usize,
    /// The file's scenes past `MAX_IMPORT_SCENES`, not imported.
    pub dropped_scenes: usize,
    /// Materials used past `MAX_IMPORT_MATERIALS`, imported as no material (set by the
    /// caller, which counts the materials built faces keep).
    pub dropped_materials: usize,
    /// Material references of the placed geometry that name no material of the file's table
    /// (or one whose id is too large for its 32-bit numbering).
    pub unmatched_materials: usize,
    /// Used materials past the first pass's bound (`4 * MAX_IMPORT_MATERIALS`, in order of
    /// first use), imported as no material.
    pub capped_materials: usize,
    /// The file's material id of each material added, in the file's order.
    pub material_ids: Vec<(u32, MaterialId)>,
    /// Textures made.
    pub textures: usize,
    /// The materials added to the document, each with the change that added it.
    pub added: Vec<(MaterialId, SceneChange)>,
    /// The changes adding the file's layers as tags, in order.
    pub added_tags: Vec<SceneChange>,
    /// The changes adding the file's scenes, in order.
    pub added_pages: Vec<SceneChange>,
    /// Textures not imported, by reason, in the order the reasons came up.
    pub texture_failures: Vec<(&'static str, usize)>,
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
            self.dropped_tags,
            &format!(
                "tag past the {MAX_IMPORT_TAGS} an import may add was left off; what is on it \
                 is Untagged"
            ),
            &format!(
                "tags past the {MAX_IMPORT_TAGS} an import may add were left off; what is on \
                 them is Untagged"
            ),
        );
        say(
            self.scenes,
            "scene was not imported",
            "scenes were not imported",
        );
        say(
            self.annotations,
            "dimension or text label was not imported",
            "dimensions and text labels were not imported",
        );
        say(
            self.dropped_scenes,
            &format!("scene past the {MAX_IMPORT_SCENES} an import may add was left off"),
            &format!("scenes past the {MAX_IMPORT_SCENES} an import may add were left off"),
        );
        say(
            self.unusable_views,
            "scene's camera could not be used; the scene leaves the camera alone",
            "scenes' cameras could not be used; those scenes leave the camera alone",
        );
        say(
            self.lost_entities,
            "entity could not be read and was skipped",
            "entities could not be read and were skipped",
        );
        say(
            self.faces_short,
            "face may be missing a hole",
            "faces may be missing holes",
        );
        say(
            self.skipped_guides + built.skipped_guides,
            "guide could not be read and was skipped",
            "guides could not be read and were skipped",
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
            self.unread_faces + built.skipped,
            "face could not be made and was skipped",
            "faces could not be made and were skipped",
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
            "soft or smooth edge not between two faces was imported as an ordinary edge",
            "soft or smooth edges not between two faces were imported as ordinary edges",
        );
        say(
            self.dropped_materials + self.capped_materials,
            &format!("material past the {MAX_IMPORT_MATERIALS} an import may add was left off"),
            &format!("materials past the {MAX_IMPORT_MATERIALS} an import may add were left off"),
        );
        say(
            self.unmatched_materials,
            "material reference could not be matched to a material and was left off",
            "material references could not be matched to a material and were left off",
        );
        for (why, n) in &self.texture_failures {
            say(
                *n,
                &format!("texture was not imported ({why}); its material keeps its colour"),
                &format!("textures were not imported ({why}); their materials keep their colours"),
            );
        }
        out
    }
}

/// Reads a SketchUp file's bytes into an import tree whose positions are meters times
/// `scale`, adding the materials its faces and instances use and its layers (as tags) to
/// `document`'s scene (listed in the notes, so a caller can take them out again: see
/// `remove_added`). With `only`, just the materials of those ids (the file's numbering) are
/// added; the others import as no material. A file from
/// before SketchUp 2017, or one whose placed model reaches beyond `MAX_COORDINATE`, is
/// `invalid_input`; a file that cannot be read is a `file` error (the message does not name
/// the file). Nothing is added on an error.
///
/// Nothing here walks the placements through the nesting: every definition is visited once,
/// so the work is linear in the file however often its definitions place each other.
pub(crate) fn read_skp(
    bytes: &[u8],
    scale: f64,
    document: &mut Document,
    only: Option<&HashSet<u32>>,
) -> Result<(ImportTree, SkpNotes), CommandError> {
    let file_error = |m: String| CommandError::new(ErrorKind::File, m);
    let src = Bytes(bytes);
    let container = Container::open(&src).map_err(|e| file_error(e.to_string()))?;
    let (model, index, stats) = read_file(&src, &container)?;
    let mut notes = SkpNotes {
        lost_entities: stats.as_ref().map_or(0, |s| saturate(s.lost_entities)),
        faces_short: stats
            .as_ref()
            .map_or(0, |s| saturate(s.faces_short_of_exact)),
        annotations: stats.as_ref().map_or(0, |s| saturate(s.annotations)),
        // A 2017-2020 file's scenes are not read (counted); a newer file's are added below.
        scenes: match container.generation {
            Generation::Legacy => index.scenes.len(),
            Generation::Vff => 0,
        },
        ..SkpNotes::default()
    };

    // Definitions by pid (the first of a repeated pid wins), and those the root reaches
    // through the instances that are imported.
    let unit = METERS_PER_INCH * scale;
    let mut by_pid: HashMap<u64, usize> = HashMap::new();
    for (i, d) in model.definitions.iter().enumerate() {
        by_pid.entry(d.pid).or_insert(i);
    }
    let placements = |g| placements(g, unit, &by_pid);
    let mut reached = vec![false; model.definitions.len()];
    let mut stack: Vec<usize> = placements(&model.root).iter().map(|p| p.1).collect();
    while let Some(d) = stack.pop() {
        if !std::mem::replace(&mut reached[d], true) {
            stack.extend(placements(&model.definitions[d].geom).iter().map(|p| p.1));
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
    check_world_size(&model.root, &model.definitions, unit, &by_pid)?;
    // Guide records not read, in the reached geometry.
    notes.skipped_guides = geometries()
        .map(|g| g.unread_guides as usize)
        .fold(0, usize::saturating_add);

    // Groups: definitions every imported placement of which is a group.
    let mut component = vec![false; model.definitions.len()];
    for (i, d, _) in geometries().flat_map(placements) {
        if !i.is_group {
            component[d] = true;
        }
    }

    // The table's materials the reached geometry uses, in order of first use: its faces' and
    // its imported instances' (a skipped instance's material is not counted). A reference
    // to an id the table lacks (or too large for its 32-bit numbering) is unmatched.
    let table: HashSet<u32> = index
        .materials
        .iter()
        .map(|m| m.id)
        .filter(|id| *id != NO_MATERIAL)
        .collect();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut used: Vec<u32> = Vec::new();
    for g in geometries() {
        let faces = g.face_material.iter().chain(&g.face_back_material).copied();
        let instances = placements(g)
            .into_iter()
            .filter(|(i, _, _)| i.has_material)
            .map(|(i, _, _)| instance_material(i).unwrap_or(NO_MATERIAL));
        for (id, from_instance) in faces
            .map(|f| (f, false))
            .chain(instances.map(|i| (i, true)))
        {
            if table.contains(&id) {
                if seen.insert(id) {
                    used.push(id);
                }
            } else if id != NO_MATERIAL || from_instance {
                notes.unmatched_materials += 1;
            }
        }
    }
    if let Some(only) = only {
        used.retain(|m| only.contains(m));
    }
    // The first pass keeps to a bound, so a file naming very many materials costs little.
    let first_pass = 4 * MAX_IMPORT_MATERIALS;
    notes.capped_materials = used.len().saturating_sub(first_pass);
    used.truncate(first_pass);
    let used: HashSet<u32> = used.into_iter().collect();
    let materials = add_materials(document, &src, &container, &index.materials, scale, used)?;
    notes.textures = materials.textures;
    notes.texture_failures = materials.failures;
    notes.added = materials.added;
    notes.material_ids = materials.order;
    let tags = match add_tags(document, &index.layers) {
        Ok(tags) => tags,
        Err(e) => {
            remove_materials(document, &notes.added);
            return Err(e);
        }
    };
    notes.added_tags = tags.added;
    notes.dropped_tags = tags.dropped;
    if container.generation == Generation::Vff {
        match add_pages(document, &index.scenes, &tags.ids, unit) {
            Ok(pages) => {
                notes.added_pages = pages.added;
                notes.unusable_views = pages.unusable_views;
                notes.dropped_scenes = pages.dropped;
            }
            Err(e) => {
                remove_added(document, &notes);
                return Err(e);
            }
        }
    }
    let ids = Ids {
        materials: &materials.ids,
        tags: &tags.ids,
    };

    let result = (|| {
        let root = geometry(&model.root, unit, &by_pid, &ids, &mut notes)?;
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
                n => n.chars().take(MAX_NAME).collect(),
            };
            // A definition the root does not reach is not built: it needs no geometry.
            let geometry = if reached[i] {
                geometry(&d.geom, unit, &by_pid, &ids, &mut notes)?
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
            remove_added(document, &notes);
            Err(e)
        }
    }
}

/// Reads a file's model and its placement-free index (with a 2017-2020 file's reading
/// statistics), under the size limits: a model over `MAX_MODEL_BYTES` or `MAX_ELEMENTS` and a
/// file from before SketchUp 2017 are `invalid_input`; one that cannot be read is a `file`
/// error.
fn read_file(
    src: &Bytes,
    container: &Container,
) -> Result<(Model, SceneIndex, Option<legacy::ReadStats>), CommandError> {
    let invalid = |m: String| CommandError::new(ErrorKind::InvalidInput, m);
    let read_error = |e: skp::Error| match e {
        skp::Error::Malformed("too many elements") => invalid(format!(
            "the file has more than {MAX_ELEMENTS} vertices, edges, faces, loops and \
             instances: too large to import"
        )),
        e => CommandError::new(ErrorKind::File, e.to_string()),
    };
    let too_large = || {
        invalid(format!(
            "the file's model unpacks to more than {} MiB: too large to import",
            MAX_MODEL_BYTES >> 20
        ))
    };
    let options = ReadOptions {
        max_elements: Some(MAX_ELEMENTS),
        ..ReadOptions::default()
    };
    match container.generation {
        Generation::Legacy => {
            if major_version(&container.header.version) < OLDEST_VERSION {
                return Err(invalid(OLD_MESSAGE.to_string()));
            }
            // The model is the file's own (uncompressed) stream.
            if container
                .legacy_stream
                .is_some_and(|(start, end)| end.saturating_sub(start) > MAX_MODEL_BYTES)
            {
                return Err(too_large());
            }
            let (model, index, stats) =
                legacy::read_model_and_index(src, container, &options).map_err(read_error)?;
            Ok((model, index, Some(stats)))
        }
        Generation::Vff => {
            if container
                .model_entry()
                .is_some_and(|e| e.uncompressed_size > MAX_MODEL_BYTES)
            {
                return Err(too_large());
            }
            let model = read_model(src, container, &options).map_err(read_error)?;
            let index = model.index_without_placements(src, container);
            Ok((model, index, None))
        }
    }
}

/// The major version of a header's version text (`{20.0.1}` is 20); 0 when it has none.
fn major_version(version: &str) -> u32 {
    let digits: String = version
        .trim_start_matches('{')
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        0
    } else {
        digits.parse().unwrap_or(u32::MAX)
    }
}

/// A count as a `usize`, saturating.
fn saturate(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// An instance that is imported: the index of the definition it places and its transform
/// (see `transform`). None for an image, a definition the file lacks or a broken transform.
fn placement(
    i: &RawInstance,
    unit: f64,
    by_pid: &HashMap<u64, usize>,
) -> Option<(usize, DAffine3)> {
    if i.is_image {
        return None;
    }
    Some((*by_pid.get(&i.def_pid)?, transform(i, unit)?))
}

/// A geometry's imported instances, each with its definition's index and its transform.
fn placements<'g>(
    g: &'g DefGeometry,
    unit: f64,
    by_pid: &HashMap<u64, usize>,
) -> Vec<(&'g RawInstance, usize, DAffine3)> {
    g.instances
        .iter()
        .filter_map(|i| placement(i, unit, by_pid).map(|(d, t)| (i, d, t)))
        .collect()
}

/// An axis-aligned box: its lowest and highest corners.
type Box3 = (DVec3, DVec3);

/// Fails when the placed model (`root` and the definitions it places, their points and guide
/// points, each instance through its transform, `unit` meters per inch) reaches beyond
/// `MAX_COORDINATE` (`invalid_input`), or when a definition places itself (`file`). Each
/// definition's box is computed once, from its points and its instances' boxes, so this is
/// linear in the file.
fn check_world_size(
    root: &DefGeometry,
    definitions: &[skp::scene::Definition],
    unit: f64,
    by_pid: &HashMap<u64, usize>,
) -> Result<(), CommandError> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unseen,
        Searching,
        Done,
    }
    // A geometry's points and the points of its guides that are imported.
    let own = |g: &DefGeometry| -> Option<Box3> {
        let anchors = g.guides.iter().filter_map(|r| match guide(r, unit)? {
            Guide::Line { point, .. } => Some(point),
            Guide::Point { position, .. } => Some(position),
        });
        g.vertex_pos
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| DVec3::from_array(*p) * unit)
            .chain(anchors)
            .fold(None, |b, p| Some(grow(b, (p, p))))
    };
    let placed_box = |b: Box3, t: &DAffine3| -> Box3 {
        let mut out = None;
        for k in 0..8 {
            let corner = DVec3::new(
                if k & 1 == 0 { b.0.x } else { b.1.x },
                if k & 2 == 0 { b.0.y } else { b.1.y },
                if k & 4 == 0 { b.0.z } else { b.1.z },
            );
            let p = t.transform_point3(corner);
            out = Some(grow(out, (p, p)));
        }
        out.expect("eight corners")
    };
    let too_large = || {
        CommandError::new(
            ErrorKind::InvalidInput,
            format!(
                "the model reaches beyond +/-{MAX_COORDINATE} m at scale {}",
                unit / METERS_PER_INCH
            ),
        )
    };
    let mut state = vec![State::Unseen; definitions.len()];
    let mut boxes: Vec<Option<Box3>> = vec![None; definitions.len()];
    // The box of a geometry whose placed definitions are all done.
    let box_of = |g: &DefGeometry, boxes: &[Option<Box3>]| -> Option<Box3> {
        placements(g, unit, by_pid)
            .iter()
            .filter_map(|(_, d, t)| boxes[*d].map(|b| placed_box(b, t)))
            .fold(own(g), |acc, b| Some(grow(acc, b)))
    };
    for (_, start, _) in placements(root, unit, by_pid) {
        if state[start] == State::Done {
            continue;
        }
        state[start] = State::Searching;
        let mut stack: Vec<(usize, Vec<usize>)> = vec![(
            start,
            placements(&definitions[start].geom, unit, by_pid)
                .iter()
                .map(|p| p.1)
                .collect(),
        )];
        while let Some((d, children)) = stack.last_mut() {
            let d = *d;
            if let Some(child) = children.pop() {
                match state[child] {
                    State::Searching => {
                        return Err(CommandError::new(
                            ErrorKind::File,
                            "a definition places itself",
                        ));
                    }
                    State::Done => {}
                    State::Unseen => {
                        state[child] = State::Searching;
                        let next = placements(&definitions[child].geom, unit, by_pid)
                            .iter()
                            .map(|p| p.1)
                            .collect();
                        stack.push((child, next));
                    }
                }
            } else {
                boxes[d] = box_of(&definitions[d].geom, &boxes);
                // Past f64's range: no use going on.
                if boxes[d].is_some_and(|(lo, hi)| !(lo.is_finite() && hi.is_finite())) {
                    return Err(too_large());
                }
                state[d] = State::Done;
                stack.pop();
            }
        }
    }
    if let Some((lo, hi)) = box_of(root, &boxes)
        && (!(lo.is_finite() && hi.is_finite())
            || lo.abs().max(hi.abs()).max_element() > MAX_COORDINATE)
    {
        return Err(too_large());
    }
    Ok(())
}

/// `b` grown to hold `more`; not finite (NaN) when either is not.
fn grow(b: Option<Box3>, more: Box3) -> Box3 {
    let finite = |b: &Box3| b.0.is_finite() && b.1.is_finite();
    match b {
        None => more,
        Some(b) if !(finite(&b) && finite(&more)) => (DVec3::NAN, DVec3::NAN),
        Some((lo, hi)) => (lo.min(more.0), hi.max(more.1)),
    }
}

/// Takes out the scenes, tags and materials an import added.
pub(crate) fn remove_added(document: &mut Document, notes: &SkpNotes) {
    let added = notes.added_tags.iter().chain(&notes.added_pages).cloned();
    document
        .scene_mut()
        .apply(&SceneChange::join(added).inverse());
    remove_materials(document, &notes.added);
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

/// The materials and tags an import added, by the file's ids.
struct Ids<'a> {
    materials: &'a HashMap<u32, MaterialId>,
    /// By the file's layer id (a 2021+ file's layer pid, a 2017-2020 file's layer index).
    tags: &'a HashMap<u32, TagId>,
}

impl Ids<'_> {
    /// The tag of a layer reference; None for Layer0, no layer or a layer not imported.
    fn tag(&self, layer: u32) -> Option<TagId> {
        (layer != NO_LAYER)
            .then(|| self.tags.get(&layer).copied())
            .flatten()
    }
}

/// One mesh of the file in meters (`unit` per inch), counting what is left out in `notes`.
fn geometry(
    g: &DefGeometry,
    unit: f64,
    by_pid: &HashMap<u64, usize>,
    ids: &Ids,
    notes: &mut SkpNotes,
) -> Result<ImportGeometry, CommandError> {
    let materials = ids.materials;
    let layer = |at: &Vec<u32>, i: usize| at.get(i).and_then(|l| ids.tag(*l));
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
        let hidden = g.face_flags.get(f).is_some_and(|flags| flags & 1 != 0);
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
            tag: layer(&g.face_layer, f),
            hidden,
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
            soft: flags & (edge_flag::SOFT | edge_flag::SMOOTH) != 0,
            tag: layer(&g.edge_layer, e),
            hidden: flags & edge_flag::HIDDEN != 0,
        });
    }

    let mut instances = Vec::with_capacity(g.instances.len());
    for i in &g.instances {
        if i.is_image {
            notes.images += 1;
            continue;
        }
        let Some((definition, transform)) = placement(i, unit, by_pid) else {
            if by_pid.contains_key(&i.def_pid) {
                notes.flat += 1;
            } else {
                notes.missing += 1;
            }
            continue;
        };
        instances.push(ImportInstance {
            definition,
            transform,
            material: instance_material(i).and_then(|m| materials.get(&m).copied()),
            tag: i
                .has_layer
                .then(|| u32::try_from(i.layer).ok().and_then(|l| ids.tag(l)))
                .flatten(),
            hidden: i.flags & 1 != 0,
        });
    }
    let mut guides = Vec::with_capacity(g.guides.len());
    for (k, r) in g.guides.iter().enumerate() {
        match guide(r, unit) {
            Some(guide) => guides.push(guide.with_tag(layer(&g.guide_layer, k))),
            None => notes.skipped_guides += 1,
        }
    }
    for (k, r) in g.hidden_guide_items.iter().enumerate() {
        match guide(r, unit) {
            Some(guide) => guides.push(
                guide
                    .with_tag(layer(&g.hidden_guide_layer, k))
                    .with_hidden(true),
            ),
            None => notes.skipped_guides += 1,
        }
    }
    Ok(ImportGeometry {
        points,
        faces,
        edges,
        guides,
        instances,
    })
}

/// A guide of the file in meters (`unit` per inch), or None when its point is not finite or
/// lies beyond `MAX_COORDINATE`, or a line's direction is not finite or has no length.
fn guide(r: &RawGuide, unit: f64) -> Option<Guide> {
    let at = |p: [f64; 3]| {
        let p = DVec3::from_array(p) * unit;
        (p.is_finite() && p.abs().max_element() <= MAX_COORDINATE).then_some(p)
    };
    match *r {
        RawGuide::Line { point, direction } => Some(Guide::Line {
            point: at(point)?,
            direction: DVec3::from_array(direction).try_normalize()?,
            tag: None,
            hidden: false,
        }),
        RawGuide::Point { position } => Some(Guide::Point {
            position: at(position)?,
            tag: None,
            hidden: false,
        }),
    }
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
    /// The file's id of each material added, in order.
    order: Vec<(u32, MaterialId)>,
    textures: usize,
    /// Textures not imported, by reason.
    failures: Vec<(&'static str, usize)>,
}

/// Adds one material per entry of `table` whose id is in `used` (in the table's order, the
/// first of a repeated id winning), named uniquely against the model's names. The caller
/// keeps to `MAX_IMPORT_MATERIALS` (see `read_skp`'s `only`). A texture that cannot be read
/// leaves the material its colour and a warning.
fn add_materials(
    document: &mut Document,
    src: &dyn ByteSource,
    container: &Container,
    table: &[MaterialDef],
    scale: f64,
    used: HashSet<u32>,
) -> Result<Added, CommandError> {
    let mut out = Added {
        ids: HashMap::new(),
        added: Vec::new(),
        order: Vec::new(),
        textures: 0,
        failures: Vec::new(),
    };
    let failed = |why: &'static str, failures: &mut Vec<(&'static str, usize)>| match failures
        .iter_mut()
        .find(|(w, _)| *w == why)
    {
        Some((_, n)) => *n += 1,
        None => failures.push((why, 1)),
    };
    let mut seen: HashSet<u32> = HashSet::new();
    let mut pixels = document.texture_pixels_besides(None);
    // Names are made unique against this set, built once: "Brick", then "Brick 2", ...
    let mut taken: HashSet<String> = document
        .scene()
        .materials()
        .map(|(_, m)| m.name.clone())
        .collect();
    let mut next: HashMap<String, usize> = HashMap::new();
    for m in table {
        if !used.contains(&m.id) || !seen.insert(m.id) {
            continue;
        }
        let base: String = match m.name.trim() {
            "" => "Material".to_string(),
            n => n.chars().take(MAX_NAME).collect(),
        };
        let name = if taken.contains(&base) {
            let n = next.entry(base.clone()).or_insert(2);
            while taken.contains(&format!("{base} {n}")) {
                *n += 1;
            }
            format!("{base} {n}")
        } else {
            base
        };
        taken.insert(name.clone());
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
                    failed(why, &mut out.failures);
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
            Err(_) if had_texture => {
                failed(MODEL_LIMIT, &mut out.failures);
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
                out.order.push((m.id, id));
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

/// The tags an import added, and how the file's layer ids map to them.
struct AddedTags {
    ids: HashMap<u32, TagId>,
    added: Vec<SceneChange>,
    /// Layers past `MAX_IMPORT_TAGS`.
    dropped: usize,
}

/// Adds one tag per layer of the file but Layer0 (in the file's order, the first of a
/// repeated id winning, at most `MAX_IMPORT_TAGS`; further ids are counted once each): its name (made unique against the
/// model's tags: "Walls 2"), colour (the next free tag colour when the file has none) and
/// visibility.
fn add_tags(document: &mut Document, layers: &[LayerDef]) -> Result<AddedTags, CommandError> {
    let mut out = AddedTags {
        ids: HashMap::new(),
        added: Vec::new(),
        dropped: 0,
    };
    let mut taken: HashSet<String> = document
        .scene()
        .tags()
        .map(|(_, t)| t.name.clone())
        .chain([UNTAGGED.to_string()])
        .collect();
    let mut next: HashMap<String, usize> = HashMap::new();
    // Layer ids met, so that a repeated id (added or dropped) counts once.
    let mut seen: HashSet<u32> = HashSet::new();
    for l in layers {
        if l.name == DEFAULT_TAG || !seen.insert(l.id) {
            continue;
        }
        if out.added.len() == MAX_IMPORT_TAGS {
            out.dropped += 1;
            continue;
        }
        let base: String = match l.name.trim() {
            "" => "Tag".to_string(),
            n => n.chars().take(MAX_NAME).collect(),
        };
        let name = if taken.contains(&base) {
            let n = next.entry(base.clone()).or_insert(2);
            while taken.contains(&format!("{base} {n}")) {
                *n += 1;
            }
            format!("{base} {n}")
        } else {
            base
        };
        taken.insert(name.clone());
        let color = if color_is_unset(l.color) {
            document.scene().next_tag_color()
        } else {
            [l.color[0], l.color[1], l.color[2]]
        };
        let tag = Tag {
            name,
            color,
            visible: !l.hidden,
        };
        match document.scene_mut().add_tag(tag) {
            Ok((id, change)) => {
                out.ids.insert(l.id, id);
                out.added.push(change);
            }
            Err(e) => {
                let added = SceneChange::join(out.added.iter().cloned());
                document.scene_mut().apply(&added.inverse());
                return Err(CommandError::new(
                    ErrorKind::File,
                    format!("tag \"{}\": {e}", l.name),
                ));
            }
        }
    }
    Ok(out)
}

/// The scenes an import added.
struct AddedPages {
    added: Vec<SceneChange>,
    /// Scenes whose camera could not be used.
    unusable_views: usize,
    /// Scenes past `MAX_IMPORT_SCENES`.
    dropped: usize,
}

/// Adds one scene per page of a 2021+ file, in the file's order and after the model's scenes,
/// none selected, at most `MAX_IMPORT_SCENES` (further ones are counted). Its name is trimmed
/// and cut to `MAX_NAME` characters (a blank one is "Scene N", the lowest number free) and
/// made unique against the model's scenes ("Front 2"). It stores the page's camera when the
/// page restores one (else no view; a camera that cannot be used, or none read, is counted and
/// left out too), and its hidden layers as hidden tags when the page restores tag
/// visibility (else no tag visibility); Layer0 and layers not imported are left out of them.
///
/// The camera is in meters (`unit` per inch) and in the root's coordinates: it goes through
/// the open context's transform, as the imported geometry does, so the scene frames what it
/// framed in the file. A parallel camera keeps its view's height at the target (scaled with
/// the geometry) through the field of view, which is what sets that height here.
fn add_pages(
    document: &mut Document,
    scenes: &[SceneDef],
    tags: &HashMap<u32, TagId>,
    unit: f64,
) -> Result<AddedPages, CommandError> {
    let mut out = AddedPages {
        added: Vec::new(),
        unusable_views: 0,
        dropped: scenes.len().saturating_sub(MAX_IMPORT_SCENES),
    };
    let mut taken: HashSet<String> = document
        .scene()
        .pages()
        .map(|(_, p)| p.name.clone())
        .collect();
    let mut next: HashMap<String, usize> = HashMap::new();
    let mut blank = 1;
    let context = document.context_transform();
    for def in scenes.iter().take(MAX_IMPORT_SCENES) {
        let base: String = def.name.trim().chars().take(MAX_NAME).collect();
        let name = if base.is_empty() {
            while taken.contains(&format!("{PAGE_NAME_BASE} {blank}")) {
                blank += 1;
            }
            format!("{PAGE_NAME_BASE} {blank}")
        } else if taken.contains(&base) {
            let n = next.entry(base.clone()).or_insert(2);
            while taken.contains(&format!("{base} {n}")) {
                *n += 1;
            }
            format!("{base} {n}")
        } else {
            base
        };
        taken.insert(name.clone());
        let view = if def.flags & page_flag::CAMERA_LOCATION != 0 {
            let view = page_view(document, def, unit / INCH_TO_MM, context);
            out.unusable_views += usize::from(view.is_none());
            view
        } else {
            None
        };
        let tags = (def.flags & page_flag::VISIBLE_LAYERS != 0).then(|| {
            def.hidden_layer_ids
                .iter()
                .filter_map(|l| tags.get(l).copied())
                .collect()
        });
        let page = Page { name, view, tags };
        match document.scene_mut().add_page(page) {
            Ok((_, change)) => out.added.push(change),
            Err(e) => {
                let added = SceneChange::join(out.added.iter().cloned());
                document.scene_mut().apply(&added.inverse());
                return Err(CommandError::new(
                    ErrorKind::File,
                    format!("scene \"{}\": {e}", def.name),
                ));
            }
        }
    }
    Ok(out)
}

/// A page's camera as a stored view: `per_mm` meters per millimetre (the reader's unit),
/// through `context` into the root's coordinates, checked as `set_camera` checks a camera.
/// None when the page has no camera or it cannot be used.
fn page_view(
    document: &Document,
    def: &SceneDef,
    per_mm: f64,
    context: DAffine3,
) -> Option<PageView> {
    let (eye, target) = (def.eye?, def.target?);
    let eye = context.transform_point3(DVec3::from(eye) * per_mm);
    let target = context.transform_point3(DVec3::from(target) * per_mm);
    let up = context.transform_vector3(DVec3::from(def.up.unwrap_or([0.0, 0.0, 1.0])));
    // The context's scale, for lengths that are not positions.
    let scale = context.matrix3.determinant().abs().cbrt();
    let degrees = if def.perspective {
        // A horizontal field of view is made vertical by the camera's own aspect, when it
        // has one (SketchUp's files store vertical ones).
        let fov = def.fov_deg.to_radians();
        if def.fov_is_height || def.aspect_ratio <= 0.0 {
            def.fov_deg
        } else {
            (2.0 * ((fov / 2.0).tan() / def.aspect_ratio).atan()).to_degrees()
        }
    } else {
        // The parallel view's height at the target is 2 d tan(fov / 2).
        let height = def.ortho_height * per_mm * scale;
        (2.0 * (height / (2.0 * eye.distance(target))).atan()).to_degrees()
    };
    // Inside the range `set_camera` takes, also once converted to radians and back.
    let degrees = degrees.clamp(1.0 + 1e-9, 170.0 - 1e-9);
    let projection = if def.perspective {
        Projection::Perspective
    } else {
        Projection::Parallel
    };
    let camera = document
        .camera_for(
            eye.to_array(),
            target.to_array(),
            Some(degrees),
            Some(up.to_array()),
            Some(projection),
        )
        .ok()?;
    Some(PageView {
        eye: camera.eye.to_array(),
        target: camera.target.to_array(),
        up: camera.up.to_array(),
        fov_y: camera.fov_y,
        parallel: !def.perspective,
        night: false,
        color_by_tag: false,
        x_ray: false,
        hidden_geometry: false,
    })
}

/// Why a texture was not imported, as the warnings say it.
const MISSING: &str = "the image is missing from the file";
const OVERSIZE: &str = "the image is over 16 MiB or 4096 pixels a side";
const UNREADABLE: &str = "the image is not a PNG or JPEG that can be read";
const TOO_MANY_PIXELS: &str = "all textures together would hold too many pixels";
const MODEL_LIMIT: &str = "the model holds at most 256 textures";

/// A material's texture from the file's zip (or, in a 2017-2020 file, from its place in the
/// file's stream), its tile size in meters times `scale` (each side clamped to the texture
/// size limits). Fails, saying why, for an image that is missing, larger than
/// `MAX_TEXTURE_BYTES`, not PNG or JPEG, too many pixels (`others` are held already) or
/// broken.
fn read_texture(
    src: &dyn ByteSource,
    zip: Option<&ZipDirectory>,
    t: &skp::model::TextureRef,
    scale: f64,
    others: u64,
) -> Result<Texture, &'static str> {
    let bytes = match t.location {
        TextureLocation::ZipEntry(at) => {
            let entry = zip
                .and_then(|z| z.entries.get(at as usize))
                .ok_or(MISSING)?;
            if entry.uncompressed_size > MAX_TEXTURE_BYTES {
                return Err(OVERSIZE);
            }
            read_entry_within(src, entry, MAX_TEXTURE_BYTES).map_err(|_| UNREADABLE)?
        }
        TextureLocation::Inline { offset, len } => {
            if len > MAX_TEXTURE_BYTES {
                return Err(OVERSIZE);
            }
            if offset.checked_add(len).is_none_or(|end| end > src.len()) {
                return Err(MISSING);
            }
            src.read_vec(offset, len as usize).map_err(|_| MISSING)?
        }
    };
    let (_, width, height) = dimensions(&bytes).map_err(|e| {
        if e.contains("pixels") || e.contains("too large") {
            OVERSIZE
        } else {
            UNREADABLE
        }
    })?;
    check_total_pixels(others, u64::from(width) * u64::from(height))
        .map_err(|_| TOO_MANY_PIXELS)?;
    let [w, h] = t
        .tile_mm_drawable()
        .map(|mm| (mm / 1000.0 * scale).clamp(MIN_TEXTURE_SIZE, MAX_TEXTURE_SIZE));
    texture_from_bytes(bytes, w, h).map_err(|_| UNREADABLE)
}

/// File bytes held in memory, read in place.
struct Bytes<'a>(&'a [u8]);

impl ByteSource for Bytes<'_> {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> skp::Result<()> {
        self.bounds_check(offset, buf.len())?;
        let start = offset as usize;
        buf.copy_from_slice(&self.0[start..start + buf.len()]);
        Ok(())
    }
}
