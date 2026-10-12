//! New, open and save, and editing definitions' names and the definitions list.

use std::path::{Path, PathBuf};

use kernel::{DefinitionId, DefinitionKind, Scene, SceneChange};

use super::Document;
use crate::camera::Camera;
use crate::command::{CommandError, ErrorKind};
use crate::file;

impl Document {
    /// An empty model with the default view.
    pub(super) fn new_model(&mut self) {
        let camera = Camera::new(self.camera.viewport);
        self.replace(Scene::new(), camera);
        self.path = None;
    }

    /// Replaces the model and view with a file's. A file whose view cannot be used (an eye
    /// straight above its target, say) opens with the default view.
    pub(super) fn open_file(&mut self, path: &str) -> Result<(), CommandError> {
        let (scene, [eye, target], fov) =
            file::read(Path::new(path)).map_err(|e| file_error(path, e))?;
        let camera = self
            .camera_for(eye.to_array(), target.to_array(), Some(fov))
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
