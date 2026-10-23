//! Edit > Intersect Faces and the viewport's right-click menu for faces and groups.

use std::collections::HashMap;

use kernel::{EntityId, FaceId};
use session::{Command, IntersectScope};

use super::{Editor, Message};
use crate::info::count;
use crate::ui::ViewportMenu;

impl Editor {
    /// Whether the selection has a face, group or component to intersect.
    pub fn can_intersect(&self) -> bool {
        self.doc
            .selection()
            .iter()
            .any(|id| matches!(id, EntityId::Face(_) | EntityId::Instance(_)))
    }

    /// The viewport's right-click menu at `at` for the selection: the section plane's entries
    /// for a lone plane; otherwise Hide for what can be hidden, Unhide for a hidden selection,
    /// the face commands for faces and Intersect Faces for faces or groups. None for a
    /// selection with none of them.
    pub fn viewport_menu(&self, at: egui::Pos2, face: Option<EntityId>) -> Option<ViewportMenu> {
        let section = self.selected_section().map(|s| s.active);
        let plain = section.is_none();
        let menu = ViewportMenu {
            at,
            section,
            intersect: plain && self.can_intersect(),
            hide: plain && self.selection_can_hide_more(),
            unhide: plain && self.selection_has_hidden(),
            faces: plain && self.selection_has_faces(),
            orient_from: face.filter(|f| self.doc.selection().contains(f)),
            make_unique: plain && self.can_make_unique(),
            flip: plain && !self.doc.selection().is_empty(),
            flip_own: plain && self.can_flip_own(),
            reset_axes: self.axes_moved(),
        };
        (menu.section.is_some()
            || menu.intersect
            || menu.hide
            || menu.unhide
            || menu.faces
            || menu.make_unique
            || menu.flip
            || menu.reset_axes)
            .then_some(menu)
    }

    /// Whether a right-click in the view opens the viewport menu: a selection or moved root axes give
    /// it entries, and the window is not a viewer (which has no way to close it, and nothing in it to run).
    pub fn opens_viewport_menu(&self, read_only: bool) -> bool {
        !read_only && self.viewport_menu(egui::Pos2::ZERO, None).is_some()
    }

    /// The menu to draw for the point it was opened at, clearing the point when the selection
    /// no longer has a menu, so it does not come back at the old place with a later selection.
    pub fn open_viewport_menu(
        &self,
        opened: &mut Option<egui::Pos2>,
        face: Option<EntityId>,
    ) -> Option<ViewportMenu> {
        let menu = opened.and_then(|at| self.viewport_menu(at, face));
        if menu.is_none() {
            *opened = None;
        }
        menu
    }

    /// Crosses the selected faces and instances with `scope` as one undo step, adding the
    /// edges where they meet. The status says how many edges and faces that made.
    pub fn intersect_faces(&mut self, scope: IntersectScope) -> bool {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let holes_before: HashMap<FaceId, usize> = self
            .doc()
            .model()
            .faces()
            .filter(|(_, f)| !f.holes.is_empty())
            .map(|(id, f)| (id, f.holes.len()))
            .collect();
        let out = self.run(Command::IntersectFaces {
            ids: Vec::new(),
            with: scope,
        });
        self.invalidate_hover();
        if let Some((report, changes)) = out.and_then(|o| o.intersect.map(|r| (r, o.changes))) {
            // A face the crossing only cut a hole in keeps its id, so it is not among the split
            // faces: count the changed faces that have more holes than before as well.
            let holed = changes
                .modified
                .iter()
                .filter(|id| match id {
                    EntityId::Face(f) => self.doc().model().face(*f).is_some_and(|face| {
                        face.holes.len() > holes_before.get(f).copied().unwrap_or(0)
                    }),
                    _ => false,
                })
                .count();
            self.message = Some(Message::info(if report.added_edges.is_empty() {
                "No faces crossed".to_string()
            } else {
                format!(
                    "Added {}, split {}",
                    count(report.added_edges.len(), "edge"),
                    count(report.split_faces.len() + holed, "face")
                )
            }));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use kernel::EntityId;
    use session::{Command, IntersectScope, SelectMode};

    use crate::editor::Editor;
    use crate::ui::ViewportMenu;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// A 2 x 2 x 1 m box with its corner at `at` (x and y).
    fn add_box(e: &mut Editor, at: f64) {
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

    fn select_all(e: &mut Editor) {
        e.execute(Command::Select {
            ids: vec![],
            mode: SelectMode::Replace,
            all: true,
        })
        .unwrap();
    }

    fn selected(e: &Editor) -> Vec<EntityId> {
        e.doc().selection().iter().copied().collect()
    }

    fn message(e: &Editor) -> Option<String> {
        e.status_parts().message
    }

    #[test]
    fn crossing_boxes_report_the_edges_and_split_faces() {
        let mut e = editor();
        add_box(&mut e, 0.0);
        select_all(&mut e);
        let out = e.execute(Command::MakeGroup { ids: selected(&e) }).unwrap();
        let group = out
            .changes
            .created
            .iter()
            .copied()
            .find(|id| matches!(id, EntityId::Instance(_)))
            .unwrap();
        e.execute(Command::Move {
            ids: vec![group],
            offset: [1.0, 1.0, 0.5],
        })
        .unwrap();
        add_box(&mut e, 0.0);
        select_all(&mut e);
        e.intersect_faces(IntersectScope::Model);
        let text = message(&e).unwrap();
        assert!(text.starts_with("Added "), "{text}");
        assert!(text.contains(", split "), "{text}");
        assert_eq!(e.doc().undo_label(), Some("intersect_faces"));
    }

    #[test]
    fn a_face_that_only_gained_a_hole_counts_as_split() {
        let mut e = editor();
        // A box from z -1 to 1 in a group, then a floor through its middle.
        let out = e
            .execute(Command::AddRectangle {
                corner: [2.0, 2.0, -1.0],
                opposite: [3.0, 3.0, -1.0],
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
        e.execute(Command::PushPull {
            face,
            distance: 2.0 * up,
        })
        .unwrap();
        select_all(&mut e);
        e.execute(Command::MakeGroup { ids: selected(&e) }).unwrap();
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [6.0, 6.0, 0.0],
            normal: None,
        })
        .unwrap();
        select_all(&mut e);
        e.intersect_faces(IntersectScope::Model);
        let text = message(&e).unwrap();
        // The box is in a group, so only the floor changes: it keeps its id and gains a hole.
        assert!(text.ends_with("split 1 face"), "{text}");
    }

    #[test]
    fn a_face_that_already_had_a_hole_is_not_counted_for_a_change_to_its_loop() {
        let mut e = editor();
        // A box in a group whose side runs along the side of a hole in the floor.
        let out = e
            .execute(Command::AddRectangle {
                corner: [2.0, 1.5, -1.0],
                opposite: [3.0, 2.5, -1.0],
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
        e.execute(Command::PushPull {
            face,
            distance: 2.0 * up,
        })
        .unwrap();
        select_all(&mut e);
        e.execute(Command::MakeGroup { ids: selected(&e) }).unwrap();
        for (c, o) in [([0.0, 0.0], [6.0, 6.0]), ([1.0, 1.0], [2.0, 2.0])] {
            e.execute(Command::AddRectangle {
                corner: [c[0], c[1], 0.0],
                opposite: [o[0], o[1], 0.0],
                normal: None,
            })
            .unwrap();
        }
        let inner = e
            .doc()
            .model()
            .faces()
            .find(|(_, f)| f.holes.is_empty())
            .unwrap()
            .0;
        e.execute(Command::Erase {
            ids: vec![EntityId::Face(inner)],
        })
        .unwrap();
        select_all(&mut e);
        e.intersect_faces(IntersectScope::Model);
        let text = message(&e).unwrap();
        assert!(text.ends_with("split 0 faces"), "{text}");
    }

    #[test]
    fn faces_that_cross_nothing_say_so() {
        let mut e = editor();
        add_box(&mut e, 0.0);
        add_box(&mut e, 5.0);
        select_all(&mut e);
        let revision = e.doc().revision();
        e.intersect_faces(IntersectScope::Selection);
        assert_eq!(message(&e).as_deref(), Some("No faces crossed"));
        assert_eq!(e.doc().revision(), revision);
    }

    #[test]
    fn nothing_selected_is_an_error_in_the_status() {
        let mut e = editor();
        add_box(&mut e, 0.0);
        e.execute(Command::Select {
            ids: vec![],
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        e.intersect_faces(IntersectScope::Model);
        assert_eq!(
            message(&e).as_deref(),
            Some("Select the faces or groups to intersect")
        );
    }

    #[test]
    fn only_faces_and_instances_make_a_selection_to_intersect() {
        let mut e = editor();
        add_box(&mut e, 0.0);
        assert!(!e.can_intersect(), "nothing selected");
        select_all(&mut e);
        assert!(e.can_intersect(), "faces");
        let out = e
            .execute(Command::AddGuidePoint {
                position: [9.0, 9.0, 0.0],
            })
            .unwrap();
        let guide = out
            .changes
            .created
            .iter()
            .copied()
            .find(|id| matches!(id, EntityId::Guide(_)))
            .unwrap();
        e.execute(Command::Select {
            ids: vec![guide],
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        assert!(!e.can_intersect(), "a guide");
    }

    #[test]
    fn a_menu_that_stops_showing_does_not_come_back_with_a_later_selection() {
        let mut e = editor();
        add_box(&mut e, 0.0);
        select_all(&mut e);
        let mut opened = Some(egui::pos2(10.0, 20.0));
        assert!(e.open_viewport_menu(&mut opened, None).is_some());
        // The selection goes without a press (Select None, Delete, undo, an agent).
        e.execute(Command::Select {
            ids: vec![],
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        assert_eq!(e.open_viewport_menu(&mut opened, None), None);
        assert_eq!(opened, None);
        select_all(&mut e);
        assert_eq!(e.open_viewport_menu(&mut opened, None), None);
    }

    #[test]
    fn the_right_click_menu_follows_the_selection() {
        let at = egui::pos2(10.0, 20.0);
        let mut e = editor();
        assert_eq!(e.viewport_menu(at, None), None, "nothing selected");
        add_box(&mut e, 0.0);
        select_all(&mut e);
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
        let out = e
            .execute(Command::AddSectionPlane {
                point: [0.0, 0.0, 0.5],
                normal: [0.0, 0.0, 1.0],
                activate: None,
            })
            .unwrap();
        let plane = out
            .changes
            .created
            .iter()
            .copied()
            .find(|id| matches!(id, EntityId::Section(_)))
            .unwrap();
        e.execute(Command::Select {
            ids: vec![plane],
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        assert_eq!(
            e.viewport_menu(at, None),
            Some(ViewportMenu {
                at,
                section: Some(true),
                intersect: false,
                hide: false,
                unhide: false,
                faces: false,
                orient_from: None,
                ..Default::default()
            }),
            "a lone plane keeps its own menu"
        );
        assert!(e.opens_viewport_menu(false));
        assert!(!e.opens_viewport_menu(true), "a viewer never opens it");
    }
}
