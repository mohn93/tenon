//! Tags and putting entities on them.

use kernel::{EntityId, KernelError, SceneChange, Tag, TagEdit, TagId};

use super::Document;
use crate::command::{CommandError, ErrorKind};
use crate::query::TagInfo;

/// The name `add_tag` gives when none is passed (made unique with a number).
const DEFAULT_TAG_NAME: &str = "Tag";

impl Document {
    /// Adds a shown tag; without `name` it is "Tag" (or "Tag 2", ...), without `color` the next
    /// color of the palette.
    pub(super) fn add_tag_changes(
        &mut self,
        name: Option<&str>,
        color: Option<[u8; 3]>,
    ) -> Result<SceneChange, CommandError> {
        let name = match name {
            Some(n) => n.to_string(),
            None => self.scene.free_tag_name(DEFAULT_TAG_NAME),
        };
        let color = color.unwrap_or_else(|| self.scene.next_tag_color());
        self.scene
            .add_tag(Tag {
                name,
                color,
                visible: true,
            })
            .map(|(_, change)| change)
            .map_err(|e| tag_error(e, None))
    }

    pub(super) fn change_tag_changes(
        &mut self,
        id: TagId,
        edit: TagEdit,
    ) -> Result<SceneChange, CommandError> {
        self.scene
            .change_tag(id, edit)
            .map_err(|e| tag_error(e, Some(id)))
    }

    /// Removes a tag, moving what is on it to `move_to` (None: Untagged).
    pub(super) fn delete_tag_changes(
        &mut self,
        id: TagId,
        move_to: Option<TagId>,
    ) -> Result<SceneChange, CommandError> {
        if self.scene.tag(id).is_none() {
            return Err(tag_error(KernelError::TagNotFound, Some(id)));
        }
        self.scene
            .delete_tag(id, move_to)
            .map_err(|e| tag_error(e, move_to))
    }

    /// Puts edges, faces, instances and guides of the open context on `tag` (None: Untagged).
    /// Vertices are invalid input, as is an empty list.
    pub(super) fn set_tag_changes(
        &mut self,
        ids: &[EntityId],
        tag: Option<TagId>,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one edge, face, instance or guide",
            ));
        }
        self.parts(ids)?;
        let mesh = self.active();
        self.scene
            .set_tags(mesh, ids, tag)
            .map_err(|e| tag_error(e, tag))
    }

    /// Every tag with how many entities in the whole model are on it.
    pub(super) fn tag_infos(&self) -> Vec<TagInfo> {
        self.scene
            .tags()
            .map(|(id, t)| TagInfo {
                id,
                name: t.name.clone(),
                color: t.color,
                visible: t.visible,
                count: self.scene.tag_count(id),
            })
            .collect()
    }
}

/// A kernel refusal as a command error; a missing tag is `not_found`, naming `id`.
fn tag_error(e: KernelError, id: Option<TagId>) -> CommandError {
    match (e, id) {
        (KernelError::TagNotFound, Some(id)) => {
            CommandError::new(ErrorKind::NotFound, format!("no tag {id}"))
        }
        (KernelError::TagNotFound, None) => CommandError::new(ErrorKind::NotFound, "no such tag"),
        (e, _) => CommandError::new(ErrorKind::InvalidInput, e.to_string()),
    }
}
