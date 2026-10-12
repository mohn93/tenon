//! New, open and save, and editing definitions' names and the definitions list.

use std::path::{Path, PathBuf};

use kernel::{DefinitionId, DefinitionKind, Scene, SceneChange};

use super::Document;
use crate::camera::Camera;
use crate::command::{CommandError, ErrorKind};
use crate::export::{self, ExportFormat};
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
    ) -> Result<(), CommandError> {
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
        if let Some(own) = &self.path {
            let same = match (own.canonicalize(), target.canonicalize()) {
                (Ok(a), Ok(b)) => a == b,
                _ => own == target,
            };
            if same {
                return Err(CommandError::new(
                    ErrorKind::InvalidInput,
                    "that is the open model's own file: export to another path",
                ));
            }
        }
        let triangles = self.scene.triangles();
        if triangles.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "nothing to export: the model has no faces",
            ));
        }
        file::replace_file(Path::new(path), &export::encode(&triangles, format))
            .map_err(|e| file_error(path, e))
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
