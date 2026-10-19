//! Importing OBJ and STL files as a group, a component or geometry of the open context.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use glam::DVec3;
use kernel::{DefinitionKind, EntityId, MaterialId, MeshId, MeshPolygon, SceneChange};

use super::{Document, MAX_COORDINATE};
use crate::command::{CommandError, ErrorKind};
use crate::import::{
    ImportAs, ImportFormat, ImportReport, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
    MAX_IMPORT_MATERIALS, MAX_MTL_BYTES, ReadMesh, UpAxis, read_mtl, read_obj, read_stl,
};
use crate::skp::{read_skp, remove_materials};
use crate::sniff;

/// The name of an imported definition whose file name gives none.
const IMPORT_NAME: &str = "Import";

/// The materials an import added, each with the change that added it.
type AddedMaterials = Vec<(MaterialId, SceneChange)>;

/// Options of `Command::Import` besides the path.
pub(crate) struct ImportOptions {
    pub format: Option<ImportFormat>,
    pub place: ImportAs,
    pub scale: Option<f64>,
    pub up: Option<UpAxis>,
    pub merge_coplanar: Option<bool>,
    pub library_roots: Option<Vec<PathBuf>>,
}

impl Document {
    /// Reads `path` and adds its faces to the open context as one change; the report is kept
    /// for the outcome. Nothing changes on failure.
    pub(super) fn import_changes(
        &mut self,
        path: &str,
        o: &ImportOptions,
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
        let format = match o.format.or_else(|| ImportFormat::from_path(file)) {
            Some(format) => format,
            None if sniff::is_skp(file) => ImportFormat::Skp,
            None => {
                return Err(invalid(
                    "give a format, or a path ending in .obj, .stl or .skp",
                ));
            }
        };
        let scale = o.scale.unwrap_or(1.0);
        if !(scale.is_finite() && scale > 0.0) {
            return Err(invalid("scale must be a positive, finite number"));
        }
        let file_error = |m: String| CommandError::new(ErrorKind::File, format!("{path}: {m}"));
        let bytes = read_limited(file, MAX_IMPORT_BYTES).map_err(file_error)?;
        let mut read = match format {
            ImportFormat::Obj => read_obj(&bytes),
            ImportFormat::Stl => read_stl(&bytes),
            ImportFormat::Skp => return self.import_skp(path, bytes, scale, o.place),
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
            .import_materials(file, &read, o.library_roots.as_deref())
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
        let file_error = |m: String| CommandError::new(ErrorKind::File, format!("{path}: {m}"));
        let (tree, notes) = read_skp(bytes, scale, self).map_err(|e| match e.kind {
            ErrorKind::File => file_error(e.message),
            _ => e,
        })?;
        let mesh = self.active();
        let built = self.scene.import_tree(
            mesh,
            &tree,
            wrapper_kind(place),
            &import_name(Path::new(path)),
        );
        let (instance, built, change) = match built {
            Ok(done) => done,
            Err(e) => {
                remove_materials(self, &notes.added);
                return Err(file_error(e.to_string()));
            }
        };
        // Materials only skipped faces used are not kept.
        let mut in_use: HashSet<MaterialId> = HashSet::new();
        for (_, m) in self.scene.meshes() {
            in_use.extend(m.faces().flat_map(|(_, f)| f.paint()).flatten());
            in_use.extend(m.instances().filter_map(|(_, i)| i.material));
        }
        let (kept, unused): (Vec<_>, Vec<_>) = notes
            .added
            .iter()
            .cloned()
            .partition(|(id, _)| in_use.contains(id));
        remove_materials(self, &unused);
        let textures = kept
            .iter()
            .filter(|(id, _)| {
                self.scene
                    .material(*id)
                    .is_some_and(|m| m.texture.is_some())
            })
            .count();
        let definition = self.model().instance(instance).map(|i| i.definition);
        let root_faces = definition
            .and_then(|d| self.scene.mesh(MeshId::Definition(d)))
            .map_or(0, |m| m.faces().count());
        let given_guides: usize = tree
            .definitions
            .iter()
            .map(|d| &d.geometry)
            .chain([&tree.root])
            .map(|g| g.guides.len())
            .sum();
        let report = ImportReport {
            instance: Some(EntityId::Instance(instance)),
            definition,
            polygons: notes.polygons,
            faces: built.faces,
            vertices: built.vertices,
            skipped: notes.unread_faces + built.skipped,
            materials: kept.len(),
            definitions: built.definitions,
            instances: built.instances,
            guides: given_guides.saturating_sub(built.skipped_guides),
            textures,
            warnings: notes.warnings(&built),
        };
        let added = SceneChange::join(kept.into_iter().map(|(_, c)| c));
        let change = SceneChange::join([added, change]);
        self.place_import(mesh, instance, change, report, place, root_faces)
    }

    /// Finishes an import that placed `instance` in `mesh` with `change`: selects it, or for
    /// `"as": "context"` explodes it (at most `MAX_CONTEXT_IMPORT_FACES` faces, its own
    /// `faces`) and selects what came out. Keeps the report for the outcome; on failure takes
    /// `change` back.
    fn place_import(
        &mut self,
        mesh: MeshId,
        instance: kernel::InstanceId,
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
    ) -> Result<(BTreeMap<String, MaterialId>, AddedMaterials), String> {
        let used: BTreeSet<&String> = read
            .polygons
            .iter()
            .filter_map(|p| p.material.as_ref())
            .filter(|m| m.as_str() != "default")
            .collect();
        let mut colors: HashMap<String, [u8; 3]> = HashMap::new();
        if !used.is_empty() {
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
                    Some(roots) => match library.canonicalize() {
                        Ok(real) if roots.iter().any(|r| real.starts_with(r)) => real,
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

/// The kind of the definition wrapping an import placed `place`.
fn wrapper_kind(place: ImportAs) -> DefinitionKind {
    match place {
        ImportAs::Component => DefinitionKind::Component,
        ImportAs::Group | ImportAs::Context => DefinitionKind::Group,
    }
}

/// The whole file, failing when it holds more than `limit` bytes.
pub(super) fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
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
