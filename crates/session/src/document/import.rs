//! Importing OBJ and STL files as a group, a component or geometry of the open context.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

use glam::DVec3;
use kernel::{DefinitionKind, EntityId, MaterialId, MeshPolygon, SceneChange};

use super::{Document, MAX_COORDINATE};
use crate::command::{CommandError, ErrorKind};
use crate::import::{
    ImportAs, ImportFormat, ImportReport, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
    MAX_MTL_BYTES, ReadMesh, UpAxis, read_mtl, read_obj, read_stl,
};

/// The name of an imported definition whose file name gives none.
const IMPORT_NAME: &str = "Import";

/// Options of `Command::Import` besides the path.
pub(crate) struct ImportOptions {
    pub format: Option<ImportFormat>,
    pub place: ImportAs,
    pub scale: Option<f64>,
    pub up: Option<UpAxis>,
    pub merge_coplanar: Option<bool>,
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
        if file
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("skr"))
        {
            return Err(invalid("a .skr file is a model: use open, not import"));
        }
        let Some(format) = o.format.or_else(|| ImportFormat::from_path(file)) else {
            return Err(invalid("give a format, or a path ending in .obj or .stl"));
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

        let (materials, added) = self.import_materials(file, &read);
        let polygon_count = read.polygons.len();
        let polygons: Vec<MeshPolygon> = std::mem::take(&mut read.polygons)
            .into_iter()
            .map(|p| MeshPolygon {
                corners: p.corners,
                material: p.material.as_ref().and_then(|m| materials.get(m).copied()),
            })
            .collect();
        let name = file
            .file_stem()
            .map(|s| s.to_string_lossy().trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| IMPORT_NAME.to_string());
        let kind = match o.place {
            ImportAs::Component => DefinitionKind::Component,
            ImportAs::Group | ImportAs::Context => DefinitionKind::Group,
        };
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
                self.scene.apply(&added.inverse());
                return Err(file_error(e.to_string()));
            }
        };
        let definition = self.model().instance(instance).map(|i| i.definition);
        let mut report = ImportReport {
            instance: Some(EntityId::Instance(instance)),
            definition,
            polygons: polygon_count,
            faces: built.faces,
            vertices: built.vertices,
            skipped: built.skipped,
            materials: added.summary().materials.len(),
        };
        let change = SceneChange::join([added, change]);
        if o.place != ImportAs::Context {
            self.selection = BTreeSet::from([EntityId::Instance(instance)]);
            self.import_report = Some(report);
            return Ok(change);
        }
        if built.faces > MAX_CONTEXT_IMPORT_FACES {
            self.scene.apply(&change.inverse());
            return Err(invalid(&format!(
                "the file makes {} faces: at most {MAX_CONTEXT_IMPORT_FACES} can be imported \
                 into the context; import it as a group or component",
                built.faces
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

    /// The model materials the OBJ's material names stand for, and the change adding the ones
    /// the model lacks. A name the model has already is that material (whatever its color);
    /// a name only a library defines (with `Kd`) becomes a new material of that color; the
    /// name `default` (any case), and names neither knows, are the default material.
    fn import_materials(
        &mut self,
        obj: &Path,
        read: &ReadMesh,
    ) -> (BTreeMap<String, MaterialId>, SceneChange) {
        let used: BTreeSet<&String> = read
            .polygons
            .iter()
            .filter_map(|p| p.material.as_ref())
            .filter(|m| !m.eq_ignore_ascii_case("default"))
            .collect();
        let mut colors: BTreeMap<String, [u8; 3]> = BTreeMap::new();
        if !used.is_empty() {
            let dir = obj.parent().unwrap_or(Path::new(""));
            for library in read
                .libraries
                .iter()
                .filter(|l| !Path::new(l).is_absolute())
            {
                if let Ok(bytes) = read_limited(&dir.join(library), MAX_MTL_BYTES) {
                    colors.extend(read_mtl(&bytes));
                }
            }
        }
        let mut out = BTreeMap::new();
        let mut changes = Vec::new();
        for name in used {
            let existing = self
                .scene
                .materials()
                .find(|(_, m)| &m.name == name)
                .map(|(id, _)| id);
            if let Some(id) = existing {
                out.insert(name.clone(), id);
            } else if let Some(color) = colors.get(name)
                && let Ok((id, change)) = self.scene.add_material(name, *color)
            {
                out.insert(name.clone(), id);
                changes.push(change);
            }
        }
        (out, SceneChange::join(changes))
    }
}

/// The whole file, failing when it holds more than `limit` bytes.
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
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
