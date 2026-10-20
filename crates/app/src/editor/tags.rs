//! The Tags tab and Entity Info's Tag field: each change is the command an agent would send.

use kernel::{EntityId, TagId};
use session::Command;

use super::Editor;

/// The tag Entity Info shows for the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionTag {
    /// Every selected entity is on this tag (None: Untagged).
    One(Option<TagId>),
    /// The selected entities are on different tags.
    Mixed,
}

impl Editor {
    /// Adds a tag named "Tag" (numbered when taken) in the next palette color.
    pub fn add_tag(&mut self) -> bool {
        self.message = None;
        self.run(Command::AddTag {
            name: None,
            color: None,
        });
        true
    }

    /// Renames a tag. A refused name leaves it and says why in the status.
    pub fn rename_tag(&mut self, tag: TagId, name: String) -> bool {
        self.change_tag(tag, Some(name), None, None)
    }

    /// Recolors a tag.
    pub fn recolor_tag(&mut self, tag: TagId, color: [u8; 3]) -> bool {
        self.change_tag(tag, None, Some(color), None)
    }

    /// Shows or hides what is on a tag.
    pub fn set_tag_visible(&mut self, tag: TagId, visible: bool) -> bool {
        self.change_tag(tag, None, None, Some(visible))
    }

    fn change_tag(
        &mut self,
        tag: TagId,
        name: Option<String>,
        color: Option<[u8; 3]>,
        visible: Option<bool>,
    ) -> bool {
        self.message = None;
        self.run(Command::ChangeTag {
            tag,
            name,
            color,
            visible,
        });
        self.invalidate_hover();
        true
    }

    /// Deletes a tag; what was on it moves to `move_to` (None: Untagged).
    pub fn delete_tag(&mut self, tag: TagId, move_to: Option<TagId>) -> bool {
        self.message = None;
        self.run(Command::DeleteTag { tag, move_to });
        self.invalidate_hover();
        true
    }

    /// Draws faces in their tags' colors, or by their materials. False if it already does.
    pub fn set_color_by_tag(&mut self, on: bool) -> bool {
        if self.doc.color_by_tag == on {
            return false;
        }
        self.run_camera_command(Command::SetColorByTag { on });
        true
    }

    /// The tag Entity Info shows for the selection: None when nothing is selected or a
    /// selected entity cannot be tagged (a vertex).
    pub fn selection_tag(&self) -> Option<SelectionTag> {
        let m = self.doc.model();
        let mut shown = None;
        for id in self.doc.selection() {
            let tag = m.tag_of(*id)?;
            shown = match shown {
                None => Some(SelectionTag::One(tag)),
                Some(SelectionTag::One(t)) if t == tag => shown,
                Some(_) => return Some(SelectionTag::Mixed),
            };
        }
        shown
    }

    /// Puts the selected entities on `tag` (None: Untagged), as one undo step. False when
    /// nothing that can be tagged is selected.
    pub fn set_selection_tag(&mut self, tag: Option<TagId>) -> bool {
        let ids: Vec<EntityId> = self
            .doc
            .selection()
            .iter()
            .copied()
            .filter(|id| !matches!(id, EntityId::Vertex(_)))
            .collect();
        if ids.is_empty() {
            return false;
        }
        self.message = None;
        self.run(Command::SetTag { ids, tag });
        self.invalidate_hover();
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use kernel::{EntityId, TagId};
    use session::Command;

    use super::SelectionTag;
    use crate::editor::Editor;

    /// A 2 x 2 rectangle, all of it selected.
    fn rectangle() -> Editor {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        e.select_all();
        e
    }

    fn tags(e: &Editor) -> Vec<(TagId, String, [u8; 3], bool)> {
        e.doc()
            .scene()
            .tags()
            .map(|(id, t)| (id, t.name.clone(), t.color, t.visible))
            .collect()
    }

    #[test]
    fn the_tag_controls_run_the_tag_commands() {
        let mut e = rectangle();
        assert_eq!(e.selection_tag(), Some(SelectionTag::One(None)));
        assert!(e.add_tag());
        let (t, name, ..) = tags(&e)[0].clone();
        assert_eq!(name, "Tag");
        e.rename_tag(t, "Walls".into());
        e.recolor_tag(t, [1, 2, 3]);
        e.set_tag_visible(t, false);
        assert_eq!(tags(&e), [(t, "Walls".into(), [1, 2, 3], false)]);
        assert!(e.set_selection_tag(Some(t)));
        assert_eq!(e.selection_tag(), Some(SelectionTag::One(Some(t))));
        assert_eq!(e.doc().scene().tag_count(t), 5, "four edges and the face");
        // One edge back to Untagged: the selection is mixed.
        let edge = *e
            .doc()
            .selection()
            .iter()
            .find(|id| matches!(id, EntityId::Edge(_)))
            .unwrap();
        e.execute(Command::SetTag {
            ids: vec![edge],
            tag: None,
        })
        .unwrap();
        assert_eq!(e.selection_tag(), Some(SelectionTag::Mixed));
        // Each change was one undo step: undo the edge, the selection's tag, then visibility.
        e.undo();
        e.undo();
        assert_eq!(e.selection_tag(), Some(SelectionTag::One(None)));
        e.undo();
        assert!(tags(&e)[0].3, "shown again");
        e.redo();
        e.redo();
        e.delete_tag(t, None);
        assert!(tags(&e).is_empty());
        assert_eq!(e.selection_tag(), Some(SelectionTag::One(None)));
        e.undo();
        assert_eq!(e.selection_tag(), Some(SelectionTag::One(Some(t))));
    }

    #[test]
    fn a_refused_name_says_why_and_color_by_tag_is_a_view_setting() {
        let mut e = rectangle();
        e.add_tag();
        let t = tags(&e)[0].0;
        e.rename_tag(t, "Untagged".into());
        assert!(e.status_parts().message.is_some());
        assert_eq!(tags(&e)[0].1, "Tag");
        let revision = e.doc().revision();
        assert!(e.set_color_by_tag(true));
        assert!(!e.set_color_by_tag(true), "already on");
        assert!(e.doc().color_by_tag);
        assert_eq!(e.doc().revision(), revision, "not a model change");
    }

    #[test]
    fn nothing_selected_has_no_tag() {
        let mut e = rectangle();
        e.select_none();
        assert_eq!(e.selection_tag(), None);
        assert!(!e.set_selection_tag(None));
    }
}
