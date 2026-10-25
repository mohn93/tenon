//! Importing OBJ, STL, SketchUp, glTF and COLLADA files as a group, a component or geometry of the
//! open context.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use glam::{DVec2, DVec3};
use kernel::{
    DefinitionKind, EntityId, ImportTree, InstanceId, KernelError, MAX_TEXTURE_SIZE,
    MIN_TEXTURE_SIZE, Material, MaterialId, MeshId, MeshPolygon, Scene, SceneChange, Texture,
    TreeReport,
};

use super::{Document, MAX_COORDINATE};
use crate::command::{CommandError, ErrorKind};
use crate::dae_read::read_dae;
use crate::gltf_read::read_gltf;
use crate::import::{
    ImportAs, ImportFormat, ImportReport, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
    MAX_IMPORT_MATERIALS, MAX_MTL_BYTES, ReadMesh, UpAxis, read_mtl, read_obj, read_stl,
};
use crate::scene_read::{
    FileUp, Mapping, ReadScene, SideFiles, build_tree, counted, name_or, reached, texture_size,
};
use crate::skp::{SkpNotes, display_length, read_skp, remove_added, remove_materials};
use crate::sniff;
use crate::texture::{check_total_pixels, dimensions, texture_from_bytes};
use crate::units::Units;

/// The name of an imported definition whose file name gives none.
const IMPORT_NAME: &str = "Import";

/// The materials an import added, each with the change that added it.
type AddedMaterials = Vec<(MaterialId, SceneChange)>;

/// What building an import tree gives: the wrapper placed (if any), the counts and the change.
pub(super) type Built = (Option<InstanceId>, TreeReport, SceneChange);

/// A SketchUp file read and built into the scene.
pub(super) struct BuiltSkp {
    tree: ImportTree,
    pub(super) notes: SkpNotes,
    /// The wrapper placed, when the tree was built into one.
    instance: Option<InstanceId>,
    built: TreeReport,
    /// The change building made (without the materials).
    change: SceneChange,
    /// The materials added and kept, each with the change that added it.
    kept: AddedMaterials,
    /// The change adding the file's layers as tags, then its scenes.
    tags_and_scenes: SceneChange,
}

/// Options of `Command::Import` besides the path; the same defaults as the command.
#[derive(Clone, Debug, Default)]
pub struct ImportOptions {
    /// The file's format; by default the one the name's extension says, else SketchUp for
    /// bytes with SketchUp's header.
    pub format: Option<ImportFormat>,
    /// What the import makes: a group (default), a component or loose geometry.
    pub place: ImportAs,
    /// Meters per file unit (default 1).
    pub scale: Option<f64>,
    /// The file's up axis (OBJ and STL).
    pub up: Option<UpAxis>,
    /// Merge neighbouring coplanar faces with one material (default true).
    pub merge_coplanar: Option<bool>,
    /// Under `--api-root`, the folders an OBJ's material libraries may be read from. Not used
    /// by [`Document::import_bytes`], which reads no libraries.
    pub library_roots: Option<Vec<PathBuf>>,
}

/// Where an import's bytes come from.
pub(super) enum Source {
    /// Read from the path (the command's own).
    Path,
    /// Given in memory under the path's name: no file is read, and no material library beside it.
    Bytes(Vec<u8>),
}

impl Document {
    /// Reads `path` and adds its faces to the open context as one change; the report is kept
    /// for the outcome. Nothing changes on failure.
    pub(super) fn import_changes(
        &mut self,
        path: &str,
        o: &ImportOptions,
        source: Source,
    ) -> Result<SceneChange, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        let file = Path::new(path);
        super::files::check_nul(file)?;
        if file
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("skr"))
        {
            return Err(invalid("a .skr file is a model: use open, not import"));
        }
        let from_bytes = matches!(source, Source::Bytes(_));
        let sniffed = match &source {
            Source::Path => sniff::is_skp(file),
            Source::Bytes(b) => sniff::is_skp_bytes(b),
        };
        let format = match o.format.or_else(|| ImportFormat::from_path(file)) {
            Some(format) => format,
            None if sniffed => ImportFormat::Skp,
            None => {
                return Err(invalid(
                    "give a format, or a path ending in .obj, .stl, .skp, .gltf, .glb or .dae",
                ));
            }
        };
        let scale = o.scale.unwrap_or(1.0);
        if !(scale.is_finite() && scale > 0.0) {
            return Err(invalid("scale must be a positive, finite number"));
        }
        let file_error = |m: String| CommandError::new(ErrorKind::File, format!("{path}: {m}"));
        let bytes = match source {
            Source::Path => read_limited(file, MAX_IMPORT_BYTES).map_err(file_error)?,
            Source::Bytes(b) if b.len() as u64 > MAX_IMPORT_BYTES => {
                return Err(file_error(format!(
                    "the file is over {MAX_IMPORT_BYTES} bytes"
                )));
            }
            Source::Bytes(b) => b,
        };
        let mut read = match format {
            ImportFormat::Obj => read_obj(&bytes),
            ImportFormat::Stl => read_stl(&bytes),
            ImportFormat::Skp => return self.import_skp(path, bytes, scale, o.place),
            ImportFormat::Gltf | ImportFormat::Dae => {
                return self.import_scene(path, format, bytes, scale, o, from_bytes);
            }
        }
        .map_err(file_error)?;
        let up = o.up.unwrap_or(format.default_up());
        let points: Vec<DVec3> = read
            .points
            .iter()
            .map(|p| up.to_model(*p) * scale)
            .collect();
        if points
            .iter()
            .any(|p| !p.is_finite() || p.abs().max_element() > MAX_COORDINATE)
        {
            return Err(file_error(format!(
                "a position is beyond +/-{MAX_COORDINATE} m at scale {scale}"
            )));
        }

        let (materials, added) = self
            .import_materials(file, &read, o.library_roots.as_deref(), from_bytes)
            .map_err(file_error)?;
        let polygon_count = read.polygons.len();
        let polygons: Vec<MeshPolygon> = std::mem::take(&mut read.polygons)
            .into_iter()
            .map(|p| MeshPolygon {
                corners: p.corners,
                material: p.material.as_ref().and_then(|m| materials.get(m).copied()),
            })
            .collect();
        let name = import_name(file);
        let kind = wrapper_kind(o.place);
        let mesh = self.active();
        let imported = self.scene.import_mesh(
            mesh,
            &points,
            &polygons,
            o.merge_coplanar.unwrap_or(true),
            kind,
            &name,
        );
        let (instance, built, change) = match imported {
            Ok(done) => done,
            Err(e) => {
                self.scene
                    .apply(&SceneChange::join(added.iter().map(|(_, c)| c.clone())).inverse());
                return Err(file_error(e.to_string()));
            }
        };
        let definition = self.model().instance(instance).map(|i| i.definition);
        // Materials only skipped polygons named are not kept.
        let in_use: HashSet<MaterialId> = definition
            .and_then(|d| self.scene.mesh(MeshId::Definition(d)))
            .map(|m| m.faces().filter_map(|(_, f)| f.front).collect())
            .unwrap_or_default();
        let (kept, unused): (Vec<_>, Vec<_>) =
            added.into_iter().partition(|(id, _)| in_use.contains(id));
        for (_, change) in unused {
            self.scene.apply(&change.inverse());
        }
        let added = SceneChange::join(kept.into_iter().map(|(_, c)| c));
        let report = ImportReport {
            instance: Some(EntityId::Instance(instance)),
            definition,
            polygons: polygon_count,
            faces: built.faces,
            vertices: built.vertices,
            skipped: built.skipped,
            materials: added.summary().materials.len(),
            ..ImportReport::default()
        };
        let change = SceneChange::join([added, change]);
        self.place_import(mesh, instance, change, report, o.place, built.faces)
    }

    /// Reads a SketchUp file's `bytes` and adds it to the open context as `import_changes`
    /// does: each definition once, the root geometry in a new group or component named after
    /// the file (exploded for `"as": "context"`). Nothing changes on failure.
    fn import_skp(
        &mut self,
        path: &str,
        bytes: Vec<u8>,
        scale: f64,
        place: ImportAs,
    ) -> Result<SceneChange, CommandError> {
        let mesh = self.active();
        let (kind, name) = (wrapper_kind(place), import_name(Path::new(path)));
        // A file imported into an empty model in the default units sets the display length
        // unit to the file's; anywhere else the model's units stay.
        let fresh = self.units == Units::default()
            && self
                .scene
                .meshes()
                .all(|(_, m)| m.edges().next().is_none() && m.instances().next().is_none());
        let b = self.build_skp(path, bytes, scale, |scene, tree| {
            scene
                .import_tree(mesh, tree, kind, &name)
                .map(|(i, r, c)| (Some(i), r, c))
        })?;
        let instance = b.instance.expect("import_tree places the wrapper");
        let definition = self.model().instance(instance).map(|i| i.definition);
        let root_faces = definition
            .and_then(|d| self.scene.mesh(MeshId::Definition(d)))
            .map_or(0, |m| m.faces().count());
        let mut report = self.skp_report(&b);
        report.instance = Some(EntityId::Instance(instance));
        report.definition = definition;
        let added = SceneChange::join(b.kept.into_iter().map(|(_, c)| c));
        let change = SceneChange::join([added, b.tags_and_scenes, b.change]);
        let unit = b.notes.unit;
        let changed = self.place_import(mesh, instance, change, report, place, root_faces)?;
        if let Some(unit) = unit.filter(|_| fresh) {
            // `execute` records the units change with the step, so undo restores them.
            self.units.length = display_length(unit);
        }
        Ok(changed)
    }

    /// Reads a glTF or COLLADA file's `bytes` and adds it to the open context as `import_changes` does:
    /// its nodes as groups and shared meshes as components (see `scene_read`), inside a new
    /// group or component named after the file (exploded for `"as": "context"`), at the
    /// file's own units times `scale`; `o.up` overrides the file's up axis. Side files are
    /// read beside `path` (none `from_bytes`). Nothing changes on failure.
    fn import_scene(
        &mut self,
        path: &str,
        format: ImportFormat,
        bytes: Vec<u8>,
        scale: f64,
        o: &ImportOptions,
        from_bytes: bool,
    ) -> Result<SceneChange, CommandError> {
        let file_error = |m: String| CommandError::new(ErrorKind::File, format!("{path}: {m}"));
        let file = Path::new(path);
        let side = SideFiles {
            folder: (!from_bytes).then(|| file.parent().unwrap_or(Path::new(""))),
            roots: o.library_roots.as_deref(),
        };
        let read = match format {
            ImportFormat::Gltf => read_gltf(&bytes, &side),
            ImportFormat::Dae => read_dae(&bytes, &side),
            _ => unreachable!("not a scene format"),
        }
        .map_err(file_error)?;
        drop(bytes);
        let scale = read.unit * scale;
        if !(scale.is_finite() && scale > 0.0) {
            return Err(file_error(
                "the file's unit is not a positive length".into(),
            ));
        }
        let up = match o.up {
            Some(UpAxis::Y) => FileUp::Y,
            Some(UpAxis::Z) => FileUp::Z,
            None => read.up,
        };
        let turn = up.turn();
        let (_, meshes) = reached(&read).map_err(file_error)?;
        let mut warnings = read.warnings.clone();
        let (materials, added, textures) =
            self.scene_materials(&read, &meshes, turn * scale, &mut warnings)?;
        let mapping = Mapping {
            turn,
            scale,
            materials: &materials,
            v_down: read.v_down,
            merge: o.merge_coplanar.unwrap_or(true),
        };
        let undo_materials = |doc: &mut Document, added: &AddedMaterials| {
            doc.scene
                .apply(&SceneChange::join(added.iter().map(|(_, c)| c.clone())).inverse());
        };
        let (tree, notes) = match build_tree(&read, &mapping) {
            Ok(built) => built,
            Err(e) => {
                undo_materials(self, &added);
                return Err(file_error(e));
            }
        };
        let beyond = std::iter::once(&tree.root)
            .chain(tree.definitions.iter().map(|d| &d.geometry))
            .any(|g| {
                g.points
                    .iter()
                    .any(|p| p.abs().max_element() > MAX_COORDINATE)
                    || g.instances
                        .iter()
                        .any(|i| i.transform.translation.abs().max_element() > MAX_COORDINATE)
            });
        if beyond {
            undo_materials(self, &added);
            return Err(file_error(format!(
                "a position is beyond +/-{MAX_COORDINATE} m at scale {scale}"
            )));
        }
        let mesh = self.active();
        let (kind, name) = (wrapper_kind(o.place), import_name(file));
        let (instance, built, change) = match self.scene.import_tree(mesh, &tree, kind, &name) {
            Ok(done) => done,
            Err(e) => {
                undo_materials(self, &added);
                return Err(file_error(e.to_string()));
            }
        };
        // Materials only skipped polygons used are not kept.
        let in_use: HashSet<MaterialId> = self
            .scene
            .meshes()
            .flat_map(|(_, m)| m.faces().filter_map(|(_, f)| f.front).collect::<Vec<_>>())
            .collect();
        let (kept, unused): (Vec<_>, Vec<_>) =
            added.into_iter().partition(|(id, _)| in_use.contains(id));
        undo_materials(self, &unused);
        let definition = self.model().instance(instance).map(|i| i.definition);
        let root_faces = definition
            .and_then(|d| self.scene.mesh(MeshId::Definition(d)))
            .map_or(0, |m| m.faces().count());
        let textures = kept.iter().filter(|(id, _)| textures.contains(id)).count();
        if built.split > 0 {
            warnings.push(format!(
                "{} could not be made as given and {} made from {} outer loop alone",
                counted(built.split, "face with holes", "faces with holes"),
                if built.split == 1 { "was" } else { "were" },
                if built.split == 1 { "its" } else { "their" },
            ));
        }
        let report = ImportReport {
            instance: Some(EntityId::Instance(instance)),
            definition,
            polygons: notes.polygons + read.skipped,
            faces: built.faces,
            vertices: built.vertices,
            skipped: read.skipped + notes.skipped + built.skipped,
            materials: kept.len(),
            definitions: built.definitions,
            instances: built.instances,
            textures,
            warnings,
            ..ImportReport::default()
        };
        let change = SceneChange::join(kept.into_iter().map(|(_, c)| c).chain([change]));
        self.place_import(mesh, instance, change, report, o.place, root_faces)
    }

    /// Adds a material for each scene material the reached `meshes` use (at most
    /// `MAX_IMPORT_MATERIALS`, in the file's order; the rest leave their faces unpainted),
    /// named as the file names it, numbered against the model's names (`Brick 2`), with its
    /// texture when the image can be used (else its color, with a warning per reason). A
    /// texture's size is the one most polygons painted with it give by their texture
    /// coordinates, their points taken through `affine` (file to model, without node
    /// transforms); 1 m by 1 m when none gives one. Returns each scene material's model material and texture size, the
    /// materials added with their changes, and those of them with a texture.
    #[allow(clippy::type_complexity)]
    fn scene_materials(
        &mut self,
        read: &ReadScene,
        meshes: &[usize],
        affine: glam::DMat3,
        warnings: &mut Vec<String>,
    ) -> Result<
        (
            Vec<Option<(MaterialId, Option<DVec2>)>>,
            AddedMaterials,
            HashSet<MaterialId>,
        ),
        CommandError,
    > {
        // For each material used, how often each texture size its polygons' coordinates give
        // comes up (sizes rounded to a millionth of a meter).
        let mut sizes: BTreeMap<usize, HashMap<[i64; 2], usize>> = BTreeMap::new();
        let mut fitted = 0usize;
        for &m in meshes {
            let mesh = &read.meshes[m];
            let mapped = !mesh.uvs.is_empty() && mesh.uvs.len() == mesh.points.len();
            for p in &mesh.polygons {
                let Some(i) = p.material.filter(|i| *i < read.materials.len()) else {
                    continue;
                };
                let counts = sizes.entry(i).or_default();
                let textured = matches!(read.materials[i].image, Some(Ok(_)));
                if !(mapped && textured) || fitted >= MAX_SIZE_FITS {
                    continue;
                }
                let corners: Option<Vec<DVec3>> = p
                    .corners
                    .iter()
                    .map(|&c| mesh.points.get(c).map(|q| affine * *q))
                    .collect();
                let uvs: Option<Vec<DVec2>> = p
                    .corners
                    .iter()
                    .map(|&c| mesh.uvs.get(c).copied())
                    .collect();
                if let (Some(corners), Some(uvs)) = (corners, uvs)
                    && let Some(size) = texture_size(&corners, &uvs, read.v_down)
                {
                    fitted += 1;
                    let key = (size * 1e6).round().as_i64vec2().to_array();
                    *counts.entry(key).or_default() += 1;
                }
            }
        }
        // The size most polygons give; the smallest of those that tie.
        let first_use: BTreeMap<usize, Option<DVec2>> = sizes
            .into_iter()
            .map(|(i, counts)| {
                let best = counts
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                    .map(|(k, _)| DVec2::new(k[0] as f64, k[1] as f64) / 1e6);
                (i, best)
            })
            .collect();
        let mut out = vec![None; read.materials.len()];
        let mut added: AddedMaterials = Vec::new();
        let mut with_texture = HashSet::new();
        let mut failures: Vec<(&'static str, usize)> = Vec::new();
        let mut fail = |why: &'static str| match failures.iter_mut().find(|(w, _)| *w == why) {
            Some((_, n)) => *n += 1,
            None => failures.push((why, 1)),
        };
        let mut taken: HashSet<String> = self
            .scene
            .materials()
            .map(|(_, m)| m.name.clone())
            .collect();
        let mut pixels = self.texture_pixels_besides(None);
        let dropped = first_use.len().saturating_sub(MAX_IMPORT_MATERIALS);
        for (&i, size) in first_use.iter().take(MAX_IMPORT_MATERIALS) {
            let m = &read.materials[i];
            let base = name_or(&m.name, "Material");
            let name = std::iter::once(base.clone())
                .chain((2..).map(|n| format!("{base} {n}")))
                .find(|n| !taken.contains(n))
                .expect("a free name");
            taken.insert(name.clone());
            let texture = match &m.image {
                None => None,
                Some(Err(why)) => {
                    fail(why);
                    None
                }
                Some(Ok(bytes)) => {
                    let size = size.unwrap_or(DVec2::ONE);
                    match scene_texture(bytes, size, pixels) {
                        Ok(t) => Some(t),
                        Err(why) => {
                            fail(why);
                            None
                        }
                    }
                }
            };
            let color = texture
                .as_ref()
                .map_or(m.color, |t| crate::texture::average_color(&t.image));
            let material = |texture: Option<Texture>| Material {
                name: name.clone(),
                color,
                opacity: m.opacity,
                texture,
                emission: 0.0,
                metalness: kernel::DEFAULT_METALNESS,
                roughness: kernel::DEFAULT_ROUGHNESS,
                density: None,
            };
            let had_texture = texture.is_some();
            let result = match self.scene.add_material_from(material(texture)) {
                Err(_) if had_texture => {
                    fail(MODEL_LIMIT);
                    self.scene.add_material_from(material(None))
                }
                r => r,
            };
            match result {
                Ok((id, change)) => {
                    if let Some(t) = self.scene.material(id).and_then(|m| m.texture.as_ref()) {
                        pixels += u64::from(t.image.width) * u64::from(t.image.height);
                        with_texture.insert(id);
                    }
                    let size = self
                        .scene
                        .material(id)
                        .and_then(|m| m.texture.as_ref())
                        .map(|t| DVec2::new(t.width, t.height));
                    out[i] = Some((id, size));
                    added.push((id, change));
                }
                Err(e) => {
                    self.scene
                        .apply(&SceneChange::join(added.iter().map(|(_, c)| c.clone())).inverse());
                    return Err(CommandError::new(
                        ErrorKind::File,
                        format!("material \"{name}\": {e}"),
                    ));
                }
            }
        }
        for (why, n) in failures {
            warnings.push(match n {
                1 => format!("1 texture was not imported ({why}); its material keeps its colour"),
                n => format!(
                    "{n} textures were not imported ({why}); their materials keep their colours"
                ),
            });
        }
        if dropped > 0 {
            warnings.push(format!(
                "{} not imported (at most {MAX_IMPORT_MATERIALS} are); faces painted with \
                 them are left unpainted",
                counted(dropped, "material was", "materials were")
            ));
        }
        Ok((out, added, with_texture))
    }

    /// Reads a SketchUp file's `bytes` (from `path`, named in messages) and builds its tree
    /// into the scene with `build`, keeping only the materials the built faces and instances
    /// use (at most `MAX_IMPORT_MATERIALS`; the bytes are read twice when more are used) and
    /// every tag the file's layers made and every scene. Nothing changes on failure.
    pub(super) fn build_skp(
        &mut self,
        path: &str,
        bytes: Vec<u8>,
        scale: f64,
        mut build: impl FnMut(&mut Scene, &ImportTree) -> Result<Built, KernelError>,
    ) -> Result<BuiltSkp, CommandError> {
        let file_error = |m: String| CommandError::new(ErrorKind::File, format!("{path}: {m}"));
        let mut only: Option<HashSet<u32>> = None;
        let (mut dropped, mut capped) = (0, 0);
        // At most twice: when the faces made keep more than `MAX_IMPORT_MATERIALS` materials,
        // everything is taken back and the file read again keeping only the first of them.
        let (tree, mut notes, instance, built, change, kept) = loop {
            let (tree, notes) =
                read_skp(&bytes, scale, self, only.as_ref()).map_err(|e| match e.kind {
                    ErrorKind::File => file_error(e.message),
                    _ => e,
                })?;
            let (instance, built, change) = match build(&mut self.scene, &tree) {
                Ok(done) => done,
                Err(e) => {
                    remove_added(self, &notes);
                    return Err(file_error(e.to_string()));
                }
            };
            // Materials only skipped faces used are not kept.
            let mut in_use: HashSet<MaterialId> = HashSet::new();
            for (_, m) in self.scene.meshes() {
                in_use.extend(m.faces().flat_map(|(_, f)| f.paint()).flatten());
                in_use.extend(m.instances().filter_map(|(_, i)| i.material));
            }
            if only.is_none() {
                let kept: Vec<u32> = notes
                    .material_ids
                    .iter()
                    .filter(|(_, id)| in_use.contains(id))
                    .map(|(file, _)| *file)
                    .collect();
                if kept.len() > MAX_IMPORT_MATERIALS {
                    self.scene.apply(&change.inverse());
                    remove_added(self, &notes);
                    dropped = kept.len() - MAX_IMPORT_MATERIALS;
                    capped = notes.capped_materials;
                    only = Some(kept.into_iter().take(MAX_IMPORT_MATERIALS).collect());
                    continue;
                }
            }
            let (kept, unused): (Vec<_>, Vec<_>) = notes
                .added
                .iter()
                .cloned()
                .partition(|(id, _)| in_use.contains(id));
            remove_materials(self, &unused);
            break (tree, notes, instance, built, change, kept);
        };
        notes.dropped_materials = dropped;
        notes.capped_materials += capped;
        let tags_and_scenes =
            SceneChange::join(notes.added_tags.iter().chain(&notes.added_pages).cloned());
        Ok(BuiltSkp {
            tree,
            notes,
            instance,
            built,
            change,
            kept,
            tags_and_scenes,
        })
    }

    /// The import report of a built SketchUp file, without the instance or definition made.
    pub(super) fn skp_report(&self, b: &BuiltSkp) -> ImportReport {
        let textures = b
            .kept
            .iter()
            .filter(|(id, _)| {
                self.scene
                    .material(*id)
                    .is_some_and(|m| m.texture.is_some())
            })
            .count();
        let given_guides: usize = b
            .tree
            .definitions
            .iter()
            .map(|d| &d.geometry)
            .chain([&b.tree.root])
            .map(|g| g.guides.len())
            .sum();
        ImportReport {
            polygons: b.notes.polygons,
            faces: b.built.faces,
            vertices: b.built.vertices,
            skipped: b.notes.unread_faces + b.built.skipped,
            materials: b.kept.len(),
            tags: b.notes.added_tags.len(),
            scenes: b.notes.added_pages.len(),
            definitions: b.built.definitions,
            instances: b.built.instances,
            guides: given_guides.saturating_sub(b.built.skipped_guides),
            textures,
            warnings: b.notes.warnings(&b.built),
            ..ImportReport::default()
        }
    }

    /// Finishes an import that placed `instance` in `mesh` with `change`: selects it, or for
    /// `"as": "context"` explodes it (at most `MAX_CONTEXT_IMPORT_FACES` faces, its own
    /// `faces`) and selects what came out. Keeps the report for the outcome; on failure takes
    /// `change` back.
    fn place_import(
        &mut self,
        mesh: MeshId,
        instance: InstanceId,
        change: SceneChange,
        mut report: ImportReport,
        place: ImportAs,
        faces: usize,
    ) -> Result<SceneChange, CommandError> {
        let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
        if place != ImportAs::Context {
            self.selection = BTreeSet::from([EntityId::Instance(instance)]);
            self.import_report = Some(report);
            return Ok(change);
        }
        if faces > MAX_CONTEXT_IMPORT_FACES {
            self.scene.apply(&change.inverse());
            return Err(invalid(&format!(
                "the file makes {faces} faces: at most {MAX_CONTEXT_IMPORT_FACES} can be \
                 imported into the context; import it as a group or component"
            )));
        }
        match self.scene.explode(mesh, instance) {
            Ok(exploded) => {
                let change = SceneChange::join([change, exploded]);
                self.selection = change
                    .summary()
                    .created
                    .into_iter()
                    .filter(|id| !matches!(id, EntityId::Vertex(_)) && self.exists(*id))
                    .collect();
                report.instance = None;
                report.definition = None;
                self.import_report = Some(report);
                Ok(change)
            }
            Err(e) => {
                self.scene.apply(&change.inverse());
                Err(CommandError::new(ErrorKind::InvalidInput, e.to_string()))
            }
        }
    }

    /// The model materials the OBJ's material names stand for, and the changes adding the
    /// ones the model lacks (one per material; the caller drops those no built face uses). A
    /// name the model has already is that material (whatever its color); a name only a library
    /// defines (with `Kd`) becomes a new material of that color; the name `default` exactly,
    /// and names neither knows, are the default material. Fails when more than
    /// `MAX_IMPORT_MATERIALS` new materials would be added.
    fn import_materials(
        &mut self,
        obj: &Path,
        read: &ReadMesh,
        roots: Option<&[PathBuf]>,
        from_bytes: bool,
    ) -> Result<(BTreeMap<String, MaterialId>, AddedMaterials), String> {
        let used: BTreeSet<&String> = read
            .polygons
            .iter()
            .filter_map(|p| p.material.as_ref())
            .filter(|m| m.as_str() != "default")
            .collect();
        let mut colors: HashMap<String, [u8; 3]> = HashMap::new();
        // Bytes have no folder to find a library in.
        if !used.is_empty() && !from_bytes {
            let dir = obj.parent().unwrap_or(Path::new(""));
            let mut tried: HashSet<&String> = HashSet::new();
            for library in read
                .libraries
                .iter()
                .filter(|l| !Path::new(l).is_absolute())
                .filter(|l| tried.insert(l))
            {
                let library = dir.join(library);
                // Under roots, the resolved file is what is read.
                let library = match roots {
                    None => library,
                    Some(roots) => match crate::fspath::canonical(&library) {
                        Ok(real) if roots.iter().any(|r| crate::fspath::starts_with(&real, r)) => {
                            real
                        }
                        _ => continue,
                    },
                };
                if let Ok(bytes) = read_limited(&library, MAX_MTL_BYTES) {
                    colors.extend(read_mtl(&bytes));
                }
            }
        }
        let existing: HashMap<&str, MaterialId> = self
            .scene
            .materials()
            .map(|(id, m)| (m.name.as_str(), id))
            .collect();
        let mut out = BTreeMap::new();
        let mut wanted: Vec<(&String, [u8; 3])> = Vec::new();
        for name in used {
            if let Some(id) = existing.get(name.as_str()) {
                out.insert(name.clone(), *id);
            } else if let Some(color) = colors.get(name) {
                wanted.push((name, *color));
            }
        }
        if wanted.len() > MAX_IMPORT_MATERIALS {
            return Err(format!(
                "the file uses more than {MAX_IMPORT_MATERIALS} materials the model lacks"
            ));
        }
        let mut added = Vec::new();
        for (name, color) in wanted {
            if let Ok((id, change)) = self.scene.add_material(name, color) {
                out.insert(name.clone(), id);
                added.push((id, change));
            }
        }
        Ok((out, added))
    }
}

/// The name of a definition imported from `file`: its stem, or `IMPORT_NAME`.
fn import_name(file: &Path) -> String {
    file.file_stem()
        .map(|s| s.to_string_lossy().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| IMPORT_NAME.to_string())
}

/// Most polygons whose texture coordinates are fitted to choose the scene's texture sizes.
const MAX_SIZE_FITS: usize = 100_000;

/// Why a texture was not imported, as the warnings say it (as SketchUp import words them).
const OVERSIZE: &str = crate::scene_read::IMAGE_OVERSIZE;
const UNREADABLE: &str = "the image is not a PNG or JPEG that can be read";
const TOO_MANY_PIXELS: &str = "all textures together would hold too many pixels";
const MODEL_LIMIT: &str = "the model holds at most 256 textures";

/// A texture of a scene file's image `bytes`, `size` meters (clamped to the texture size
/// limits), or why it cannot be used (`others`: the pixels textures hold already).
fn scene_texture(bytes: &[u8], size: DVec2, others: u64) -> Result<Texture, &'static str> {
    let (_, width, height) = dimensions(bytes).map_err(|e| {
        if e.contains("pixels") || e.contains("too large") {
            OVERSIZE
        } else {
            UNREADABLE
        }
    })?;
    check_total_pixels(others, u64::from(width) * u64::from(height))
        .map_err(|_| TOO_MANY_PIXELS)?;
    let [w, h] = size
        .to_array()
        .map(|x| x.clamp(MIN_TEXTURE_SIZE, MAX_TEXTURE_SIZE));
    texture_from_bytes(bytes.to_vec(), w, h).map_err(|_| UNREADABLE)
}

/// The kind of the definition wrapping an import placed `place`.
fn wrapper_kind(place: ImportAs) -> DefinitionKind {
    match place {
        ImportAs::Component => DefinitionKind::Component,
        ImportAs::Group | ImportAs::Context => DefinitionKind::Group,
    }
}

/// The whole file, failing when it holds more than `limit` bytes.
pub(crate) fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    // A pipe or device could block forever (or eat the host's stdin): regular files only.
    if !std::fs::metadata(path)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("not a regular file".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("file too large: over {limit} bytes"));
    }
    Ok(bytes)
}
