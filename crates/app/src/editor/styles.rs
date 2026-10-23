//! The Styles tab and View > Edge Style: each change is the command an agent would send.

use kernel::StyleId;
use session::{Command, StylePatch};

use super::Editor;

impl Editor {
    /// Adds a style (a copy of `from`, or the default settings) and chooses it. A refused add
    /// says why in the status and chooses nothing.
    pub fn add_style(&mut self, from: Option<StyleId>) -> bool {
        self.message = None;
        let newest = |e: &Editor| e.doc.scene().styles().map(|(id, _)| id).max();
        let before = newest(self);
        let chosen = self.doc.style();
        self.run(Command::AddStyle { name: None, from });
        let after = newest(self);
        if after != before {
            if let Some(new) = after {
                self.style_before_add.insert(new, chosen);
            }
            self.run_camera_command(Command::SetStyle { style: after });
        }
        true
    }

    /// Renames a style. A refused name leaves it and says why in the status.
    pub fn rename_style(&mut self, style: StyleId, name: String) -> bool {
        self.change_style(style, Some(name), None)
    }

    /// Changes the parts of a style's settings `patch` gives: one undo step.
    pub fn change_style_settings(&mut self, style: StyleId, patch: StylePatch) -> bool {
        self.change_style(style, None, Some(patch))
    }

    fn change_style(
        &mut self,
        style: StyleId,
        name: Option<String>,
        settings: Option<StylePatch>,
    ) -> bool {
        self.message = None;
        self.run(Command::ChangeStyle {
            style,
            name,
            settings,
        });
        true
    }

    /// Deletes a style (undoable); the view and scenes that chose it go back to the default.
    pub fn delete_style(&mut self, style: StyleId) -> bool {
        self.message = None;
        self.run(Command::DeleteStyle { style });
        true
    }

    /// Chooses the style the view draws with (None: the default look): no undo step. False if
    /// it already is.
    pub fn choose_style(&mut self, style: Option<StyleId>) -> bool {
        if self.doc.style() == style {
            return false;
        }
        self.run_camera_command(Command::SetStyle { style });
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use kernel::{StyleId, StyleSettings};
    use session::{Command, EdgesPatch, StylePatch};

    use crate::editor::Editor;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    fn names(e: &Editor) -> Vec<(StyleId, String)> {
        e.doc()
            .scene()
            .styles()
            .map(|(id, s)| (id, s.name.clone()))
            .collect()
    }

    fn profiles(width: f32) -> StylePatch {
        StylePatch {
            edges: Some(EdgesPatch {
                profiles: Some(Some(width)),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_new_model_forgets_what_was_chosen_before_a_style_was_added() {
        let mut e = editor();
        e.add_style(None);
        assert!(!e.style_before_add.is_empty());
        e.execute(Command::New { discard: true }).unwrap();
        assert!(e.style_before_add.is_empty());
    }

    #[test]
    fn adding_a_style_chooses_it_in_one_undo_step() {
        let mut e = editor();
        assert!(e.add_style(None));
        let [(first, name)] = names(&e).try_into().unwrap();
        assert_eq!(name, "Style 1");
        assert_eq!(e.doc().style(), Some(first));
        assert_eq!(e.doc().chosen_style(), Some(&StyleSettings::default()));
        // The choice is a view setting: undo takes the style, one step, and the view is default.
        e.undo();
        assert!(names(&e).is_empty());
        assert_eq!(e.doc().style(), None);
    }

    #[test]
    fn adding_copies_the_style_it_is_given() {
        let mut e = editor();
        e.add_style(None);
        let first = e.doc().style().unwrap();
        e.change_style_settings(first, profiles(5.0));
        e.add_style(Some(first));
        let second = e.doc().style().unwrap();
        assert_ne!(first, second);
        assert_eq!(e.doc().chosen_style().unwrap().edges.profiles, Some(5.0));
        e.add_style(None);
        assert_eq!(e.doc().chosen_style().unwrap().edges.profiles, None);
    }

    #[test]
    fn a_change_is_one_undo_step_and_a_rename_too() {
        let mut e = editor();
        e.add_style(None);
        let id = e.doc().style().unwrap();
        e.change_style_settings(id, profiles(4.0));
        assert_eq!(e.doc().chosen_style().unwrap().edges.profiles, Some(4.0));
        e.rename_style(id, "Ink".into());
        assert_eq!(names(&e), [(id, "Ink".into())]);
        e.undo();
        assert_eq!(names(&e), [(id, "Style 1".into())]);
        assert_eq!(e.doc().chosen_style().unwrap().edges.profiles, Some(4.0));
        e.undo();
        assert_eq!(e.doc().chosen_style().unwrap().edges.profiles, None);
    }

    #[test]
    fn a_refused_name_says_why_and_changes_nothing() {
        let mut e = editor();
        e.add_style(None);
        e.add_style(None);
        let [(a, _), (b, _)] = names(&e).try_into().unwrap();
        e.rename_style(b, "Style 1".into());
        assert_eq!(names(&e)[1], (b, "Style 2".into()));
        assert!(e.message_kind().is_some());
        e.rename_style(a, "  ".into());
        assert_eq!(names(&e)[0], (a, "Style 1".into()));
    }

    #[test]
    fn choosing_is_no_undo_step_and_deleting_falls_back_to_default() {
        let mut e = editor();
        e.add_style(None);
        let id = e.doc().style().unwrap();
        assert!(e.choose_style(None));
        assert_eq!(e.doc().style(), None);
        assert!(!e.choose_style(None), "already default");
        assert!(e.choose_style(Some(id)));
        assert!(!e.choose_style(Some(id)));
        let revision = e.doc().revision();
        e.choose_style(None);
        assert_eq!(e.doc().revision(), revision, "a view change");
        e.choose_style(Some(id));
        e.delete_style(id);
        assert!(names(&e).is_empty());
        assert_eq!(e.doc().style(), None);
        e.undo();
        assert_eq!(names(&e).len(), 1);
        assert_eq!(e.doc().style(), Some(id), "undo brings the choice back");
    }

    #[test]
    fn a_scene_keeps_the_style_it_was_saved_with() {
        let mut e = editor();
        e.add_style(None);
        let id = e.doc().style().unwrap();
        e.add_scene();
        e.choose_style(None);
        let scene = e.doc().scene().pages().next().unwrap().0;
        e.go_to_scene(scene);
        assert_eq!(e.doc().style(), Some(id));
        e.delete_style(id);
        e.go_to_scene(scene);
        assert_eq!(e.doc().style(), None);
        // The agent's commands see the same.
        e.execute(Command::SetStyle { style: None }).unwrap();
    }

    #[test]
    fn undoing_an_add_goes_back_to_the_style_chosen_before() {
        let mut e = editor();
        e.add_style(None);
        let first = e.doc().style().unwrap();
        e.add_style(Some(first));
        let second = e.doc().style().unwrap();
        assert_ne!(first, second);
        e.undo();
        assert_eq!(e.doc().style(), Some(first));
        e.undo();
        assert_eq!(e.doc().style(), None);
        // From the default look it stays the default.
        e.add_style(None);
        e.choose_style(None);
        e.add_style(None);
        e.undo();
        assert_eq!(e.doc().style(), None);
    }
}
