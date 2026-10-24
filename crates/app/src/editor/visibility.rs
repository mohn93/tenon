//! Edit > Hide and Unhide, View > X-ray and Hidden Geometry, Reverse Faces and Orient Faces:
//! the window's side of the visibility and orientation commands.

use kernel::EntityId;
use session::{Command, SelectMode};

use super::{Editor, Message};
use crate::info::count;

/// "1 entity" or "3 entities".
fn entities(n: usize) -> String {
    if n == 1 {
        "1 entity".into()
    } else {
        format!("{n} entities")
    }
}

impl Editor {
    /// Whether the selection has an edge, face, instance or guide: what can be hidden.
    fn selection_can_hide(&self) -> bool {
        let m = self.doc.model();
        self.doc
            .selection()
            .iter()
            .any(|id| m.hidden_of(*id).is_some())
    }

    /// Whether every selected edge, face, instance and guide is hidden (Entity Info's Hidden
    /// checkbox, unchecked for a mix); None when the selection has none of them.
    pub fn selection_hidden(&self) -> Option<bool> {
        let m = self.doc.model();
        let mut seen = false;
        let mut hidden = true;
        for id in self.doc.selection() {
            if let Some(h) = m.hidden_of(*id) {
                seen = true;
                hidden &= h;
            }
        }
        seen.then_some(hidden)
    }

    /// Whether Hide would do something: a selected edge, face, instance or guide is shown.
    pub fn selection_can_hide_more(&self) -> bool {
        self.selection_hidden() == Some(false)
    }

    /// Whether the open context has a hidden edge, face, instance or guide (Unhide > All).
    /// Cached by revision and context, since the menu asks every frame.
    pub fn context_has_hidden(&self) -> bool {
        let mut cache = self.hidden_cache.borrow_mut();
        if let Some((revision, context, any)) = cache.as_ref()
            && *revision == self.doc.revision()
            && context == self.doc.context()
        {
            return *any;
        }
        let any = self.doc.model().any_hidden();
        *cache = Some((self.doc.revision(), self.doc.context().to_vec(), any));
        any
    }

    /// Whether Unhide > Last would show something.
    pub fn has_last_hidden(&self) -> bool {
        self.doc.has_last_hidden()
    }

    /// Whether any selected entity is hidden (possible only while Hidden Geometry shows them).
    pub fn selection_has_hidden(&self) -> bool {
        let m = self.doc.model();
        self.doc
            .selection()
            .iter()
            .any(|id| m.hidden_of(*id) == Some(true))
    }

    /// Whether the selection has a face (Reverse Faces, Orient Faces).
    pub fn selection_has_faces(&self) -> bool {
        self.doc
            .selection()
            .iter()
            .any(|id| matches!(id, EntityId::Face(_)))
    }

    /// Runs a command of this module as a panel action, returning its outcome.
    fn run_visibility(&mut self, command: Command) -> Option<session::Outcome> {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let out = self.run(command);
        self.invalidate_hover();
        out
    }

    fn selected_ids(&self) -> Vec<EntityId> {
        self.doc.selection().iter().copied().collect()
    }

    /// Edit > Hide: hides the selected edges, faces, instances and guides as one undo step.
    pub fn hide_selection(&mut self) -> bool {
        let ids = self.selected_ids();
        if ids.is_empty() {
            self.message = Some(Message::error("Select what to hide"));
            return true;
        }
        self.set_hidden_of(Command::Hide { ids }, "Hid");
        true
    }

    /// Edit > Unhide > Selected.
    pub fn unhide_selection(&mut self) -> bool {
        let ids = self.selected_ids();
        self.set_hidden_of(Command::Unhide { ids }, "Unhid");
        true
    }

    /// Edit > Unhide > Last: what the latest Hide hid and is still hidden.
    pub fn unhide_last(&mut self) -> bool {
        self.set_hidden_of(Command::UnhideLast {}, "Unhid");
        true
    }

    /// Edit > Unhide > All: everything hidden in the open context.
    pub fn unhide_all(&mut self) -> bool {
        self.set_hidden_of(Command::UnhideAll {}, "Unhid");
        true
    }

    /// Entity Info's Hidden checkbox: hides or shows everything selected.
    pub fn set_selection_hidden(&mut self, hidden: bool) -> bool {
        if !self.selection_can_hide() {
            return false;
        }
        if hidden {
            self.hide_selection()
        } else {
            self.unhide_selection()
        }
    }

    /// Hides or shows the instance an Outliner row stands for, opening the context that holds
    /// it first, as selecting the row does.
    pub fn outliner_set_hidden(&mut self, path: &[EntityId], hidden: bool) -> bool {
        let Some((id, parent)) = path.split_last() else {
            return false;
        };
        let context: Vec<EntityId> = self
            .doc
            .context()
            .iter()
            .map(|i| EntityId::Instance(*i))
            .collect();
        let moved = parent != context.as_slice();
        let selection = self.selected_ids();
        if moved
            && self
                .run_visibility(Command::OpenPath {
                    path: parent.to_vec(),
                })
                .is_none()
        {
            return true;
        }
        let ids = vec![*id];
        if hidden {
            self.set_hidden_of(Command::Hide { ids }, "Hid");
        } else {
            self.set_hidden_of(Command::Unhide { ids }, "Unhid");
        }
        // Hiding the open group or one around it closes it, as SketchUp does: the user stays
        // at its parent rather than inside what is no longer drawn.
        let closes = hidden && context.starts_with(path);
        if moved && !closes {
            // Back where the user was, with what was selected there (what is hidden leaves it).
            let message = self.message.take();
            let _ = self.execute(Command::OpenPath { path: context });
            let _ = self.execute(Command::Select {
                ids: selection,
                mode: SelectMode::Replace,
                all: false,
            });
            self.message = message;
        }
        true
    }

    /// Runs a hide or unhide command and says how many entities it changed.
    fn set_hidden_of(&mut self, command: Command, done: &str) {
        let Some(out) = self.run_visibility(command) else {
            return;
        };
        let n = out.changed.map_or(0, |c| c.len());
        self.message = Some(Message::info(if n == 0 {
            format!(
                "Nothing to {}",
                if done == "Hid" { "hide" } else { "unhide" }
            )
        } else {
            format!("{done} {}", entities(n))
        }));
    }

    /// Edit > Reverse Faces: turns the selected faces over as one undo step.
    pub fn reverse_faces(&mut self) -> bool {
        let ids: Vec<EntityId> = self
            .doc
            .selection()
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Face(_)))
            .collect();
        if ids.is_empty() {
            self.message = Some(Message::error("Select the faces to reverse"));
            return true;
        }
        if let Some(out) = self.run_visibility(Command::ReverseFaces { ids }) {
            let n = out.changed.map_or(0, |c| c.len());
            self.message = Some(Message::info(flipped(n)));
        }
        true
    }

    /// Orient Faces: from `from` (the face right-clicked) when it is a selected face, else
    /// from the selected face with the lowest id, turns the faces joined to it so they agree
    /// with it (among the selected faces when several are selected, else the whole connected
    /// surface), as one undo step.
    pub fn orient_faces(&mut self, from: Option<EntityId>) -> bool {
        let faces: Vec<EntityId> = self
            .doc
            .selection()
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Face(_)))
            .collect();
        let start = from
            .filter(|f| faces.contains(f))
            .or(faces.first().copied());
        let Some(id) = start else {
            self.message = Some(Message::error("Select a face to orient from"));
            return true;
        };
        let ids = if faces.len() > 1 { faces } else { Vec::new() };
        if let Some(report) = self
            .run_visibility(Command::OrientFaces { id, ids })
            .and_then(|o| o.orient)
        {
            let n = report.flipped.len();
            self.message = Some(Message::info(match (n, report.conflicts) {
                (0, 0) => "Faces already agree".to_string(),
                (_, 0) => flipped(n),
                (0, c) => format!("{}, not all could agree", count(c, "conflict")),
                (_, c) => format!("{}, {}", flipped(n), count(c, "conflict")),
            }));
        }
        true
    }

    /// View > X-ray: draws every face as glass. False if it already is as asked.
    pub fn set_x_ray(&mut self, on: bool) -> bool {
        if self.doc.x_ray == on {
            return false;
        }
        self.run_camera_command(Command::SetXRay { on });
        true
    }

    /// View > Hidden Geometry: draws hidden entities ghosted and lets them be picked. False if
    /// it already is as asked.
    pub fn set_hidden_geometry(&mut self, on: bool) -> bool {
        if self.doc.hidden_geometry == on {
            return false;
        }
        self.run_camera_command(Command::SetHiddenGeometry { on });
        true
    }
}

/// "Flipped 1 face" or "Flipped 3 faces", "No faces changed" for none.
fn flipped(n: usize) -> String {
    if n == 0 {
        "No faces changed".into()
    } else {
        format!("Flipped {}", count(n, "face"))
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use kernel::EntityId;
    use session::{Command, SelectMode};

    use crate::editor::Editor;
    use crate::ui::ViewportMenu;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// A 2 x 2 x 1 m box with its corner at the origin.
    fn add_box(e: &mut Editor) {
        add_box_at(e, 0.0);
    }

    /// The same box with its corner at (`at`, `at`).
    fn add_box_at(e: &mut Editor, at: f64) {
        let out = e
            .execute(Command::AddRectangle {
                corner: [at, at, 0.0],
                opposite: [at + 2.0, at + 2.0, 0.0],
                normal: None,
            })
            .unwrap();
        let face = *out
            .changes
            .created
            .iter()
            .find(|id| matches!(id, EntityId::Face(_)))
            .unwrap();
        let EntityId::Face(f) = face else {
            unreachable!()
        };
        let up = e.doc().model().face(f).unwrap().normal.z.signum();
        e.execute(Command::PushPull { face, distance: up }).unwrap();
    }

    fn select(e: &mut Editor, ids: Vec<EntityId>) {
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
    }

    fn selected(e: &Editor) -> Vec<EntityId> {
        e.doc().selection().iter().copied().collect()
    }

    fn select_all(e: &mut Editor) {
        e.execute(Command::Select {
            ids: vec![],
            mode: SelectMode::Replace,
            all: true,
        })
        .unwrap();
    }

    fn faces(e: &Editor) -> Vec<EntityId> {
        e.doc()
            .model()
            .faces()
            .map(|(id, _)| EntityId::Face(id))
            .collect()
    }

    fn message(e: &Editor) -> Option<String> {
        e.status_parts().message
    }

    #[test]
    fn hide_says_how_many_and_the_selection_goes() {
        let mut e = editor();
        add_box(&mut e);
        let face = faces(&e)[0];
        select(&mut e, vec![face]);
        assert_eq!(e.selection_hidden(), Some(false));
        assert!(e.hide_selection());
        assert_eq!(message(&e).as_deref(), Some("Hid 1 entity"));
        assert!(e.doc().selection().is_empty());
        assert_eq!(e.doc().undo_label(), Some("hide"));
        select_all(&mut e);
        e.hide_selection();
        assert_eq!(message(&e).as_deref(), Some("Hid 17 entities"));
    }

    #[test]
    fn hide_with_nothing_selected_says_so() {
        let mut e = editor();
        add_box(&mut e);
        select(&mut e, vec![]);
        e.hide_selection();
        assert_eq!(message(&e).as_deref(), Some("Select what to hide"));
        assert_eq!(e.doc().undo_label(), Some("push_pull"));
    }

    #[test]
    fn unhide_in_its_three_forms() {
        let mut e = editor();
        add_box(&mut e);
        let [a, b, ..] = faces(&e)[..] else { panic!() };
        e.execute(Command::SetHiddenGeometry { on: true }).unwrap();
        select(&mut e, vec![a]);
        e.hide_selection();
        assert_eq!(e.doc().selection().len(), 1, "kept while shown as ghosts");
        assert_eq!(e.selection_hidden(), Some(true));
        assert!(e.selection_has_hidden());
        e.unhide_selection();
        assert_eq!(message(&e).as_deref(), Some("Unhid 1 entity"));
        assert_eq!(e.selection_hidden(), Some(false));
        e.unhide_selection();
        assert_eq!(message(&e).as_deref(), Some("Nothing to unhide"));

        e.execute(Command::SetHiddenGeometry { on: false }).unwrap();
        select(&mut e, vec![a]);
        e.hide_selection();
        select(&mut e, vec![b]);
        e.hide_selection();
        e.unhide_last();
        assert_eq!(message(&e).as_deref(), Some("Unhid 1 entity"));
        e.unhide_all();
        assert_eq!(message(&e).as_deref(), Some("Unhid 1 entity"));
        e.unhide_all();
        assert_eq!(message(&e).as_deref(), Some("Nothing to unhide"));
        e.unhide_last();
        assert_eq!(message(&e).as_deref(), Some("Nothing to unhide"));
    }

    #[test]
    fn the_hidden_checkbox_is_mixed_until_everything_selected_is_hidden() {
        let mut e = editor();
        add_box(&mut e);
        assert_eq!(e.selection_hidden(), None, "nothing selected");
        let [a, b, ..] = faces(&e)[..] else { panic!() };
        e.execute(Command::SetHiddenGeometry { on: true }).unwrap();
        select(&mut e, vec![a]);
        e.hide_selection();
        select(&mut e, vec![a, b]);
        assert_eq!(e.selection_hidden(), Some(false), "one of two");
        assert!(e.set_selection_hidden(true));
        assert_eq!(e.selection_hidden(), Some(true));
        assert!(e.set_selection_hidden(false));
        assert_eq!(e.selection_hidden(), Some(false));
    }

    #[test]
    fn reverse_and_orient_say_what_they_turned() {
        let mut e = editor();
        add_box(&mut e);
        let all = faces(&e);
        select(&mut e, vec![all[0], all[1]]);
        assert!(e.reverse_faces());
        assert_eq!(message(&e).as_deref(), Some("Flipped 2 faces"));
        assert_eq!(e.doc().undo_label(), Some("reverse_faces"));
        select(&mut e, vec![all[0]]);
        e.reverse_faces();
        assert_eq!(message(&e).as_deref(), Some("Flipped 1 face"));

        // One face selected: the whole connected surface is oriented from it.
        select(&mut e, vec![all[2]]);
        assert!(e.orient_faces(None));
        assert_eq!(message(&e).as_deref(), Some("Flipped 1 face"));
        assert_eq!(e.doc().undo_label(), Some("orient_faces"));
        e.orient_faces(None);
        assert_eq!(message(&e).as_deref(), Some("Faces already agree"));
    }

    #[test]
    fn orient_with_several_faces_selected_stays_among_them() {
        let mut e = editor();
        add_box(&mut e);
        let all = faces(&e);
        select(&mut e, vec![all[3]]);
        e.reverse_faces();
        // The odd face is out of scope, so it stays.
        select(&mut e, vec![all[0], all[1], all[2]]);
        e.orient_faces(None);
        assert_eq!(message(&e).as_deref(), Some("Faces already agree"));
        select(&mut e, all.clone());
        e.orient_faces(None);
        assert_eq!(message(&e).as_deref(), Some("Flipped 1 face"));
    }

    #[test]
    fn reverse_and_orient_need_a_face() {
        let mut e = editor();
        add_box(&mut e);
        select(&mut e, vec![]);
        e.reverse_faces();
        assert_eq!(message(&e).as_deref(), Some("Select the faces to reverse"));
        e.orient_faces(None);
        assert_eq!(message(&e).as_deref(), Some("Select a face to orient from"));
        assert!(!e.selection_has_faces());
        select_all(&mut e);
        assert!(e.selection_has_faces());
    }

    #[test]
    fn the_flags_toggle_through_the_camera_commands() {
        let mut e = editor();
        assert!(e.set_x_ray(true));
        assert!(e.doc().x_ray);
        assert!(!e.set_x_ray(true), "already on");
        assert!(e.set_hidden_geometry(true));
        assert!(e.doc().hidden_geometry);
        assert!(e.set_hidden_geometry(false) && e.set_x_ray(false));
        assert!(!e.doc().x_ray && !e.doc().hidden_geometry);
        assert_eq!(e.doc().undo_label(), None, "no undo step");
    }

    #[test]
    fn an_outliner_row_hides_and_unhides_its_instance() {
        let mut e = editor();
        add_box(&mut e);
        select_all(&mut e);
        let ids = e.doc().selection().iter().copied().collect();
        let out = e.execute(Command::MakeGroup { ids, name: None }).unwrap();
        let group = out
            .changes
            .created
            .iter()
            .copied()
            .find(|id| matches!(id, EntityId::Instance(_)))
            .unwrap();
        assert!(e.outliner_set_hidden(&[group], true));
        let rows = e.doc().outliner_rows();
        assert!(rows.roots[0].own_hidden);
        assert_eq!(message(&e).as_deref(), Some("Hid 1 entity"));
        assert!(e.outliner_set_hidden(&[group], false));
        assert!(!e.doc().outliner_rows().roots[0].own_hidden);
    }

    #[test]
    fn orient_works_from_the_face_that_was_right_clicked() {
        let mut e = editor();
        add_box(&mut e);
        let all = faces(&e);
        select(&mut e, vec![all[0]]);
        e.reverse_faces();
        select_all(&mut e);
        // Without a face to go by, the lowest id (the odd one) is the reference.
        assert!(e.orient_faces(None));
        assert_eq!(message(&e).as_deref(), Some("Flipped 5 faces"));
        e.undo();
        // From another face, the odd one is turned to agree with it.
        e.orient_faces(Some(all[1]));
        assert_eq!(message(&e).as_deref(), Some("Flipped 1 face"));
        e.orient_faces(None);
        assert_eq!(message(&e).as_deref(), Some("Faces already agree"));
        // A face that is not selected is no reference.
        select(&mut e, vec![all[0], all[1]]);
        e.orient_faces(Some(all[2]));
        assert_eq!(message(&e).as_deref(), Some("Faces already agree"));
    }

    #[test]
    fn the_menu_remembers_the_selected_face_under_the_cursor() {
        let at = egui::pos2(10.0, 20.0);
        let mut e = editor();
        add_box(&mut e);
        let all = faces(&e);
        select(&mut e, vec![all[0], all[1]]);
        assert_eq!(
            e.viewport_menu(at, Some(all[1])).unwrap().orient_from,
            Some(all[1])
        );
        assert_eq!(e.viewport_menu(at, Some(all[2])).unwrap().orient_from, None);
    }

    #[test]
    fn hide_is_offered_only_for_something_still_shown() {
        let at = egui::pos2(10.0, 20.0);
        let mut e = editor();
        add_box(&mut e);
        let all = faces(&e);
        e.execute(Command::SetHiddenGeometry { on: true }).unwrap();
        select(&mut e, vec![all[0]]);
        e.hide_selection();
        assert!(!e.selection_can_hide_more());
        assert!(!e.viewport_menu(at, None).unwrap().hide);
        select(&mut e, vec![all[0], all[1]]);
        assert!(e.selection_can_hide_more(), "a mix");
        select(&mut e, vec![all[1]]);
        assert!(e.selection_can_hide_more());
    }

    #[test]
    fn unhide_all_and_last_are_offered_only_when_they_would_do_something() {
        let mut e = editor();
        add_box(&mut e);
        let all = faces(&e);
        assert!(!e.context_has_hidden() && !e.has_last_hidden());
        select(&mut e, vec![all[0]]);
        e.hide_selection();
        assert!(e.context_has_hidden() && e.has_last_hidden());
        e.unhide_all();
        assert!(!e.context_has_hidden() && !e.has_last_hidden());
        select(&mut e, vec![all[0]]);
        e.hide_selection();
        e.undo();
        assert!(!e.context_has_hidden() && !e.has_last_hidden(), "undone");
        e.redo();
        assert!(e.context_has_hidden());
    }

    #[test]
    fn hiding_from_a_nested_outliner_row_keeps_the_context_and_selection() {
        let mut e = editor();
        add_box(&mut e);
        select_all(&mut e);
        let ids = e.doc().selection().iter().copied().collect();
        let out = e.execute(Command::MakeGroup { ids, name: None }).unwrap();
        let outer = out
            .changes
            .created
            .iter()
            .copied()
            .find(|id| matches!(id, EntityId::Instance(_)))
            .unwrap();
        e.execute(Command::OpenPath { path: vec![outer] }).unwrap();
        add_box_at(&mut e, 5.0);
        select_all(&mut e);
        let ids = e.doc().selection().iter().copied().collect();
        let out = e.execute(Command::MakeGroup { ids, name: None }).unwrap();
        let inner = out
            .changes
            .created
            .iter()
            .copied()
            .find(|id| matches!(id, EntityId::Instance(_)))
            .unwrap();
        e.execute(Command::OpenPath { path: vec![] }).unwrap();
        select(&mut e, vec![outer]);
        assert!(e.outliner_set_hidden(&[outer, inner], true));
        assert!(e.doc().context().is_empty(), "back at the root");
        assert_eq!(selected(&e), vec![outer]);
        assert_eq!(message(&e).as_deref(), Some("Hid 1 entity"));
        // Opening inside to see it: the inner group is hidden.
        e.execute(Command::OpenPath { path: vec![outer] }).unwrap();
        assert!(e.context_has_hidden());
    }

    #[test]
    fn hiding_the_open_group_from_the_outliner_leaves_it() {
        let mut e = editor();
        add_box(&mut e);
        select_all(&mut e);
        let ids = e.doc().selection().iter().copied().collect();
        let out = e.execute(Command::MakeGroup { ids, name: None }).unwrap();
        let group = out
            .changes
            .created
            .iter()
            .copied()
            .find(|id| matches!(id, EntityId::Instance(_)))
            .unwrap();
        e.execute(Command::OpenPath { path: vec![group] }).unwrap();
        assert!(e.outliner_set_hidden(&[group], true));
        assert!(e.doc().context().is_empty(), "closed with the group hidden");
        assert!(e.doc().outliner_rows().roots[0].own_hidden);
    }

    #[test]
    fn the_right_click_menu_offers_hide_unhide_and_face_commands() {
        let at = egui::pos2(10.0, 20.0);
        let mut e = editor();
        add_box(&mut e);
        let all = faces(&e);
        select(&mut e, vec![all[0]]);
        assert_eq!(
            e.viewport_menu(at, None),
            Some(ViewportMenu {
                at,
                section: None,
                intersect: true,
                hide: true,
                unhide: false,
                faces: true,
                orient_from: None,
                flip: true,
                ..Default::default()
            })
        );
        // An edge alone: Hide only.
        let edge = e
            .doc()
            .model()
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .next()
            .unwrap();
        select(&mut e, vec![edge]);
        let menu = e.viewport_menu(at, None).unwrap();
        assert!(menu.hide && !menu.unhide && !menu.faces && !menu.intersect);
        // A hidden selection (only possible with Hidden Geometry) can be unhidden.
        e.execute(Command::SetHiddenGeometry { on: true }).unwrap();
        e.hide_selection();
        assert!(e.viewport_menu(at, None).unwrap().unhide);
        assert!(e.opens_viewport_menu(false));
        assert!(!e.opens_viewport_menu(true), "a viewer never opens it");
    }
}
