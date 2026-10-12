//! New, open and save, and editing definitions' names and the definitions list.

use std::path::{Path, PathBuf};

use kernel::{DefinitionId, DefinitionKind, Scene, SceneChange};

use super::Document;
use crate::camera::Camera;
use crate::command::{CommandError, ErrorKind};
use crate::export::{self, ExportFormat, ExportReport};
use crate::file;

impl Document {
    /// Fails when the model has changes not saved, unless the command said to discard them.
    pub(super) fn check_unsaved(&self, discard: bool) -> Result<(), CommandError> {
        if self.is_modified() && !discard {
            return Err(CommandError::new(
                ErrorKind::UnsavedChanges,
                "the model has unsaved changes: save it first, or pass \"discard\": true",
            ));
        }
        Ok(())
    }

    /// An empty model with the default view.
    pub(super) fn new_model(&mut self) {
        let camera = Camera::new(self.camera.viewport);
        self.replace(Scene::new(), camera);
        self.path = None;
    }

    /// Replaces the model and view with a file's. A file whose view cannot be used (an eye
    /// on its target, say) opens with the default view.
    pub(super) fn open_file(&mut self, path: &str) -> Result<(), CommandError> {
        let (scene, c) = file::read(Path::new(path)).map_err(|e| file_error(path, e))?;
        let camera = self
            .camera_for(c.eye, c.target, Some(c.fov_y_deg), c.up, Some(c.projection))
            .unwrap_or_else(|_| Camera::new(self.camera.viewport));
        self.replace(scene, camera);
        self.path = Some(PathBuf::from(path));
        Ok(())
    }

    /// Saves to `path`, or to the file last opened or saved.
    pub(super) fn save_file(&mut self, path: Option<&str>) -> Result<(), CommandError> {
        let target = path
            .map(PathBuf::from)
            .or_else(|| self.path.clone())
            .ok_or_else(|| {
                CommandError::new(
                    ErrorKind::InvalidInput,
                    "give a path: the model was never opened or saved",
                )
            })?;
        file::write(&target, &self.scene, &self.camera)
            .map_err(|e| file_error(&target.display().to_string(), e))?;
        self.path = Some(target);
        self.clean_revision = self.revision;
        Ok(())
    }

    /// Writes every face of the whole model as triangles in `format`, or the format the
    /// path's extension names.
    pub(super) fn export_file(
        &self,
        path: &str,
        format: Option<ExportFormat>,
    ) -> Result<ExportReport, CommandError> {
        let Some(format) = format.or_else(|| ExportFormat::from_path(Path::new(path))) else {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "give a format, or a path ending in .obj, .stl, .glb or .gltf",
            ));
        };
        let target = Path::new(path);
        if target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("skr"))
        {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a .skr path is a model file: use save, not export",
            ));
        }
        let own = |p: &Path| {
            self.path
                .as_ref()
                .is_some_and(|own| match (own.canonicalize(), p.canonicalize()) {
                    (Ok(a), Ok(b)) => a == b,
                    _ => own == p,
                })
        };
        if own(target) {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "that is the open model's own file: export to another path",
            ));
        }
        let (triangles, skipped_faces) = self.scene.triangles_and_skipped();
        if triangles.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "nothing to export: the model has no faces",
            ));
        }
        // An OBJ with painted faces names a material library beside it: same name, `.mtl`.
        let mtl_path = target.with_extension("mtl");
        let mtl_name = mtl_path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let encoded = export::encode(&self.scene, &triangles, format, &mtl_name)
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e))?;
        if let Some(mtl) = &encoded.mtl {
            // On a case-blind file system "x.MTL" is the library's own path.
            let same = |a: &Path, b: &Path| {
                a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
            };
            if same(&mtl_path, target) || own(&mtl_path) {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "an OBJ of painted faces writes its materials to {}, which is not \
                         free: export to another path",
                        mtl_path.display()
                    ),
                ));
            }
            file::replace_file(&mtl_path, mtl)
                .map_err(|e| file_error(&mtl_path.display().to_string(), e))?;
        }
        file::replace_file(target, &encoded.bytes).map_err(|e| file_error(path, e))?;
        Ok(ExportReport {
            triangles: triangles.len(),
            skipped_faces,
        })
    }

    /// Swaps in another model: history, selection and the open context start over, and event
    /// subscribers are told to read the model again.
    fn replace(&mut self, scene: Scene, camera: Camera) {
        self.scene = scene;
        self.camera = camera;
        self.context.clear();
        self.selection.clear();
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
        self.events.clear();
        self.log_start = self.revision;
        self.clean_revision = self.revision;
    }

    pub(super) fn rename_changes(
        &mut self,
        id: DefinitionId,
        name: &str,
    ) -> Result<SceneChange, CommandError> {
        let Some(d) = self.scene.definition(id) else {
            return Err(CommandError::new(
                ErrorKind::NotFound,
                format!("no definition {id}"),
            ));
        };
        if d.kind == DefinitionKind::Component && name.trim().is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "a component needs a name",
            ));
        }
        self.scene
            .rename_definition(id, name)
            .map_err(|e| CommandError::new(ErrorKind::NotFound, e.to_string()))
    }
}

fn file_error(path: &str, e: String) -> CommandError {
    CommandError::new(ErrorKind::File, format!("{path}: {e}"))
}
