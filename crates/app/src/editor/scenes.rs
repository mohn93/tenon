//! The scene tabs and the Scenes tab: each change is the command an agent would send.

use kernel::PageId;
use session::Command;

use super::Editor;

impl Editor {
    /// Adds a scene after the last one, storing the current view and hidden tags ("Scene 1",
    /// or the lowest free number). It becomes the selected scene.
    pub fn add_scene(&mut self) -> bool {
        self.message = None;
        self.run(Command::AddScene {
            name: None,
            camera: true,
            tags: true,
        });
        true
    }

    /// Stores the current view and hidden tags in a scene again, the parts it stores; with
    /// `camera` or `tags`, starts (true) or stops (false) storing that part.
    pub fn update_scene(
        &mut self,
        scene: PageId,
        camera: Option<bool>,
        tags: Option<bool>,
    ) -> bool {
        self.message = None;
        self.run(Command::UpdateScene {
            scene,
            camera,
            tags,
        });
        true
    }

    /// Renames a scene. A refused name leaves it and says why in the status.
    pub fn rename_scene(&mut self, scene: PageId, name: String) -> bool {
        self.message = None;
        self.run(Command::ChangeScene {
            scene,
            name: Some(name),
            camera: None,
            tags: None,
        });
        true
    }

    /// Starts (true) or stops storing the view (`camera`) or tag visibility (`tags`) in a
    /// scene. A part started is stored from now; the other part keeps what it stores.
    pub fn set_scene_stores(
        &mut self,
        scene: PageId,
        camera: Option<bool>,
        tags: Option<bool>,
    ) -> bool {
        self.message = None;
        self.run(Command::ChangeScene {
            scene,
            name: None,
            camera,
            tags,
        });
        true
    }

    /// Moves a scene to `index` in the order (0: first).
    pub fn move_scene(&mut self, scene: PageId, index: usize) -> bool {
        self.message = None;
        self.run(Command::MoveScene { scene, index });
        true
    }

    pub fn delete_scene(&mut self, scene: PageId) -> bool {
        self.message = None;
        self.run(Command::DeleteScene { scene });
        true
    }

    /// Goes to a scene: its view (the tool's operation goes on, as for any camera change)
    /// and its tag visibility. It becomes the selected scene.
    pub fn go_to_scene(&mut self, scene: PageId) -> bool {
        self.run_camera_command(Command::GoToScene { scene });
        // Tags shown or hidden change what the cursor is over even when the camera stays.
        self.invalidate_hover();
        true
    }

    /// Goes to the scene after the selected one (`forward`) or before it, wrapping around;
    /// with none selected, to the first (forward) or the last. False when there are no scenes.
    pub fn cycle_scene(&mut self, forward: bool) -> bool {
        let scenes: Vec<PageId> = self.doc.scene().pages().map(|(id, _)| id).collect();
        let n = scenes.len();
        if n == 0 {
            return false;
        }
        let at = self
            .doc
            .selected_scene()
            .and_then(|s| scenes.iter().position(|p| *p == s));
        let next = match (at, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        self.go_to_scene(scenes[next])
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use kernel::PageId;
    use session::{Command, StandardView};

    use crate::editor::Editor;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    fn names(e: &Editor) -> Vec<String> {
        e.doc()
            .scene()
            .pages()
            .map(|(_, p)| p.name.clone())
            .collect()
    }

    fn ids(e: &Editor) -> Vec<PageId> {
        e.doc().scene().pages().map(|(id, _)| id).collect()
    }

    #[test]
    fn the_scene_controls_run_the_scene_commands_one_undo_step_each() {
        let mut e = editor();
        assert!(e.add_scene());
        e.standard_view(StandardView::Top);
        assert!(e.add_scene());
        assert_eq!(names(&e), ["Scene 1", "Scene 2"]);
        let [one, two] = ids(&e)[..] else { panic!() };
        assert_eq!(e.doc().selected_scene(), Some(two));
        e.rename_scene(one, "Front".into());
        e.move_scene(two, 0);
        assert_eq!(names(&e), ["Scene 2", "Front"]);
        e.update_scene(one, None, Some(false));
        assert!(e.doc().scene().page(one).unwrap().tags.is_none());
        e.delete_scene(two);
        assert_eq!(names(&e), ["Front"]);
        // Undo takes back the delete, the update, the move and the rename, one at a time.
        e.undo();
        assert_eq!(names(&e), ["Scene 2", "Front"]);
        e.undo();
        assert!(e.doc().scene().page(one).unwrap().tags.is_some());
        e.undo();
        assert_eq!(names(&e), ["Front", "Scene 2"]);
        e.undo();
        assert_eq!(names(&e), ["Scene 1", "Scene 2"]);
    }

    #[test]
    fn the_checkboxes_keep_the_other_part_as_stored() {
        let mut e = editor();
        e.execute(Command::AddTag {
            name: Some("Walls".into()),
            color: None,
        })
        .unwrap();
        let walls = e.doc().scene().tags().next().unwrap().0;
        e.add_scene();
        let one = ids(&e)[0];
        let stored = e.doc().scene().page(one).unwrap().clone();
        // Tag visibility changes, then Camera is unticked: the stored tags stay as they were.
        e.set_tag_visible(walls, false);
        e.standard_view(StandardView::Top);
        e.set_scene_stores(one, Some(false), None);
        let page = e.doc().scene().page(one).unwrap().clone();
        assert_eq!(page.view, None);
        assert_eq!(page.tags, stored.tags, "nothing hidden, as stored");
        assert_eq!(e.doc().undo_label(), Some("change_scene"));
        // Ticked again, the camera is stored from now; the tags still stay.
        e.set_scene_stores(one, Some(true), None);
        let page = e.doc().scene().page(one).unwrap().clone();
        assert_eq!(
            page.view.unwrap().eye,
            e.doc().camera.eye.to_array(),
            "the top view"
        );
        assert_eq!(page.tags, stored.tags);
    }

    #[test]
    fn a_refused_name_says_why() {
        let mut e = editor();
        e.add_scene();
        e.add_scene();
        let [one, _] = ids(&e)[..] else { panic!() };
        e.rename_scene(one, "Scene 2".into());
        assert!(e.status_parts().message.is_some());
        assert_eq!(names(&e), ["Scene 1", "Scene 2"]);
    }

    #[test]
    fn going_to_a_scene_moves_the_camera_and_selects_it() {
        let mut e = editor();
        e.add_scene();
        let front = e.doc().camera.clone();
        e.standard_view(StandardView::Top);
        let one = ids(&e)[0];
        assert!(e.go_to_scene(one));
        assert_eq!(e.doc().camera, front);
        assert_eq!(e.doc().selected_scene(), Some(one));
        assert_eq!(e.doc().undo_label(), Some("add_scene"), "no undo step");
    }

    #[test]
    fn page_keys_cycle_through_the_scenes_wrapping() {
        let mut e = editor();
        assert!(!e.cycle_scene(true), "no scenes");
        for _ in 0..3 {
            e.add_scene();
        }
        let [a, b, c] = ids(&e)[..] else { panic!() };
        assert_eq!(e.doc().selected_scene(), Some(c));
        let mut seen = Vec::new();
        for _ in 0..4 {
            e.cycle_scene(true);
            seen.push(e.doc().selected_scene().unwrap());
        }
        assert_eq!(seen, [a, b, c, a]);
        e.cycle_scene(false);
        assert_eq!(e.doc().selected_scene(), Some(c));
        // None selected (the selected one deleted): forward goes to the first, back to the last.
        e.execute(Command::DeleteScene { scene: c }).unwrap();
        assert_eq!(e.doc().selected_scene(), None);
        e.cycle_scene(false);
        assert_eq!(e.doc().selected_scene(), Some(b));
        e.execute(Command::DeleteScene { scene: b }).unwrap();
        e.cycle_scene(true);
        assert_eq!(e.doc().selected_scene(), Some(a));
    }
}
