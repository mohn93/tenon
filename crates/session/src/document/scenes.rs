//! Scenes: saved views and tag visibility, gone back to in one step.

use kernel::{KernelError, Page, PageId, PageView, SceneChange, TagEdit};

use glam::DVec3;

use super::{Document, unit_vector};
use crate::camera::{Lighting, Projection};
use crate::command::{CommandError, ErrorKind};
use crate::query::{QueryResult, SceneInfo, SceneView};

impl Document {
    /// The scene last added or gone to, while it exists.
    pub fn selected_scene(&self) -> Option<PageId> {
        self.selected_scene
            .filter(|id| self.scene.page(*id).is_some())
    }

    /// The view a scene stores now: the camera, lighting and color by tag.
    fn current_view(&self) -> PageView {
        self.camera.page_view(self.lighting, self.color_by_tag)
    }

    /// Adds a scene after the last one, storing the current view (`camera`) and hidden tags
    /// (`tags`); without `name` it is "Scene 1" (or the lowest free number). It becomes the
    /// selected scene.
    pub(super) fn add_scene_changes(
        &mut self,
        name: Option<&str>,
        camera: bool,
        tags: bool,
    ) -> Result<SceneChange, CommandError> {
        let page = Page {
            name: name.map_or_else(|| self.scene.free_page_name(), str::to_string),
            view: camera.then(|| self.current_view()),
            tags: tags.then(|| self.scene.hidden_tags()),
        };
        let (id, change) = self
            .scene
            .add_page(page)
            .map_err(|e| scene_error(e, None))?;
        self.selected_scene = Some(id);
        Ok(change)
    }

    /// Stores the current view and hidden tags in a scene again; a part absent is stored again
    /// when the scene stores it, false stops storing it.
    pub(super) fn update_scene_changes(
        &mut self,
        id: PageId,
        camera: Option<bool>,
        tags: Option<bool>,
    ) -> Result<SceneChange, CommandError> {
        let page = self.page(id)?;
        let camera = camera.unwrap_or(page.view.is_some());
        let tags = tags.unwrap_or(page.tags.is_some());
        let page = Page {
            name: page.name.clone(),
            view: camera.then(|| self.current_view()),
            tags: tags.then(|| self.scene.hidden_tags()),
        };
        self.scene
            .change_page(id, page)
            .map_err(|e| scene_error(e, Some(id)))
    }

    /// Renames a scene and chooses what it stores: a part turned off is dropped, a part
    /// turned on is stored from the current state only when the scene lacks it, and a part that
    /// stays keeps what was stored.
    pub(super) fn change_scene_changes(
        &mut self,
        id: PageId,
        name: Option<&str>,
        camera: Option<bool>,
        tags: Option<bool>,
    ) -> Result<SceneChange, CommandError> {
        if name.is_none() && camera.is_none() && tags.is_none() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "change_scene needs a name, camera or tags",
            ));
        }
        let page = self.page(id)?;
        let view = match camera {
            Some(false) => None,
            Some(true) => Some(page.view.unwrap_or_else(|| self.current_view())),
            None => page.view,
        };
        let hidden = match tags {
            Some(false) => None,
            Some(true) => Some(
                page.tags
                    .clone()
                    .unwrap_or_else(|| self.scene.hidden_tags()),
            ),
            None => page.tags.clone(),
        };
        let page = Page {
            name: name.map_or_else(|| page.name.clone(), str::to_string),
            view,
            tags: hidden,
        };
        self.scene
            .change_page(id, page)
            .map_err(|e| scene_error(e, Some(id)))
    }

    pub(super) fn move_scene_changes(
        &mut self,
        id: PageId,
        index: usize,
    ) -> Result<SceneChange, CommandError> {
        self.scene
            .move_page(id, index)
            .map_err(|e| scene_error(e, Some(id)))
    }

    pub(super) fn delete_scene_changes(&mut self, id: PageId) -> Result<SceneChange, CommandError> {
        self.scene
            .delete_page(id)
            .map_err(|e| scene_error(e, Some(id)))
    }

    /// Goes to a scene: sets the camera, projection, lighting and color by tag it stores (not
    /// part of the returned change), and returns the tag visibility changes it makes. It
    /// becomes the selected scene. Nothing changes on failure.
    pub(super) fn go_to_scene_changes(&mut self, id: PageId) -> Result<SceneChange, CommandError> {
        let page = self.page(id)?.clone();
        let view = match &page.view {
            Some(v) => {
                let projection = if v.parallel {
                    Projection::Parallel
                } else {
                    Projection::Perspective
                };
                let degrees = v.fov_y.to_degrees();
                let mut camera = self
                    .camera_for(v.eye, v.target, Some(degrees), Some(v.up), Some(projection))
                    .map_err(|e| {
                        CommandError::new(
                            ErrorKind::InvalidInput,
                            format!("scene {id}'s view cannot be used: {}", e.message),
                        )
                    })?;
                // The stored values as they are: through degrees the field of view could drift
                // by a bit, and an update would then store a change. A vertical view's up is
                // the stored one (horizontal, checked by `camera_for`); any other view's is
                // straight up.
                camera.fov_y = v.fov_y;
                let up = DVec3::from(v.up);
                if camera.is_vertical() && up.z == 0.0 {
                    // A unit up (as stored from a camera) is kept bit for bit.
                    camera.up = if (up.length() - 1.0).abs() < 1e-12 {
                        up
                    } else {
                        unit_vector(up).unwrap_or(camera.up)
                    };
                }
                Some((camera, v.night, v.color_by_tag))
            }
            None => None,
        };
        let mut changes = Vec::new();
        if let Some(hidden) = &page.tags {
            let flips: Vec<_> = self
                .scene
                .tags()
                .filter(|(t, tag)| tag.visible == hidden.contains(t))
                .map(|(t, tag)| (t, !tag.visible))
                .collect();
            for (t, visible) in flips {
                let edit = TagEdit {
                    visible: Some(visible),
                    ..TagEdit::default()
                };
                changes.push(
                    self.scene
                        .change_tag(t, edit)
                        .expect("showing or hiding a tag cannot fail"),
                );
            }
        }
        if let Some((camera, night, color_by_tag)) = view {
            self.camera = camera;
            self.lighting = if night {
                Lighting::Night
            } else {
                Lighting::Day
            };
            self.color_by_tag = color_by_tag;
        }
        self.selected_scene = Some(id);
        Ok(SceneChange::join(changes))
    }

    /// Every scene in order, with what it stores and whether it is selected.
    pub(super) fn scene_infos(&self) -> Vec<SceneInfo> {
        let selected = self.selected_scene();
        self.scene
            .pages()
            .map(|(id, p)| SceneInfo {
                id,
                name: p.name.clone(),
                camera: p.view.is_some(),
                tags: p.tags.is_some(),
                selected: selected == Some(id),
            })
            .collect()
    }

    /// The `scene` query: the view and hidden tags a scene stores.
    pub(super) fn scene_detail(&self, id: PageId) -> Result<QueryResult, CommandError> {
        let page = self.page(id)?;
        Ok(QueryResult::Scene {
            id,
            name: page.name.clone(),
            view: page.view.map(|v| SceneView {
                eye: v.eye,
                target: v.target,
                up: v.up,
                fov_y_deg: v.fov_y.to_degrees(),
                projection: if v.parallel {
                    Projection::Parallel
                } else {
                    Projection::Perspective
                },
                lighting: if v.night {
                    Lighting::Night
                } else {
                    Lighting::Day
                },
                color_by_tag: v.color_by_tag,
            }),
            hidden_tags: page.tags.as_ref().map(|t| t.iter().copied().collect()),
            revision: self.revision,
        })
    }

    fn page(&self, id: PageId) -> Result<&Page, CommandError> {
        self.scene
            .page(id)
            .ok_or_else(|| scene_error(KernelError::PageNotFound, Some(id)))
    }
}

/// A kernel refusal as a command error; a missing scene or tag is `not_found`.
fn scene_error(e: KernelError, id: Option<PageId>) -> CommandError {
    match (e, id) {
        (KernelError::PageNotFound, Some(id)) => {
            CommandError::new(ErrorKind::NotFound, format!("no scene {id}"))
        }
        (e @ (KernelError::PageNotFound | KernelError::TagNotFound), _) => {
            CommandError::new(ErrorKind::NotFound, e.to_string())
        }
        (e, _) => CommandError::new(ErrorKind::InvalidInput, e.to_string()),
    }
}
