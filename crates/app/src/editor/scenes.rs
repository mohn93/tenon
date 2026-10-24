//! The scene tabs and the Scenes tab: each change is the command an agent would send.

use kernel::PageId;
use session::motion::{Motion, smoothstep};
use session::{Camera, Command, ScenePositions};
use web_time::Instant;

use super::{Editor, Message};

/// A scene transition in progress: drawing only, the document is already at the end.
pub(super) struct Transition {
    motion: Motion,
    started: Instant,
    seconds: f64,
}

/// What the window draws while a transition runs: the camera and the scene with the instances
/// where the transition has them.
pub struct TransitionFrame {
    pub camera: Camera,
    pub scene: kernel::Scene,
}

impl Editor {
    /// Sets how going to a scene animates (the model setting), as one undo step. False if
    /// it already is so.
    pub fn set_scene_transition(&mut self, transition: kernel::SceneTransition) -> bool {
        if self.doc.scene().scene_transition() == transition {
            return false;
        }
        self.message = None;
        self.run(Command::SetSceneTransition {
            on: Some(transition.on),
            seconds: Some(transition.seconds),
        });
        true
    }

    /// True while a scene transition is being drawn.
    #[cfg(test)]
    pub fn transitioning(&self) -> bool {
        self.transition.is_some()
    }

    /// Ends a transition at once (any input does): the next frame shows the document.
    pub fn finish_transition(&mut self) -> bool {
        self.transition.take().is_some()
    }

    /// The camera and scene to draw at `now`, or None when no transition runs (one that has
    /// run its time ends here). The easing is smoothstep; the camera keeps the window's
    /// viewport.
    pub fn transition_frame(&mut self, now: Instant) -> Option<TransitionFrame> {
        let t = self.transition.as_ref()?;
        let x = now.saturating_duration_since(t.started).as_secs_f64() / t.seconds;
        if x >= 1.0 {
            self.transition = None;
            return None;
        }
        let eased = smoothstep(x);
        let mut camera = t.motion.camera_at(eased);
        camera.viewport = self.doc.camera.viewport;
        Some(TransitionFrame {
            camera,
            scene: t.motion.scene_at(self.doc.scene(), eased),
        })
    }

    /// Adds a scene after the last one, storing the current view and hidden tags ("Scene 1",
    /// or the lowest free number). It becomes the selected scene.
    pub fn add_scene(&mut self) -> bool {
        self.message = None;
        self.run(Command::AddScene {
            name: None,
            camera: true,
            tags: true,
            positions: None,
        });
        true
    }

    /// Stores the current view and hidden tags in a scene again, the parts it stores; with
    /// `camera` or `tags`, starts (true) or stops (false) storing that part. A scene that
    /// stores neither part has nothing to update: the status says so.
    pub fn update_scene(
        &mut self,
        scene: PageId,
        camera: Option<bool>,
        tags: Option<bool>,
    ) -> bool {
        self.message = None;
        let stores_nothing = self
            .doc
            .scene()
            .page(scene)
            .is_some_and(|p| p.view.is_none() && p.tags.is_none());
        if stores_nothing && camera.is_none() && tags.is_none() {
            self.message = Some(Message::info(
                "This scene stores neither the camera nor tag visibility: tick Camera or Tag \
                 visibility to store them",
            ));
            return true;
        }
        self.run(Command::UpdateScene {
            scene,
            camera,
            tags,
            positions: None,
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
            positions: None,
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
            positions: None,
        });
        true
    }

    /// Starts (true) or stops storing instance positions in a scene. Started, it stores the
    /// selected instances of the root, or every instance of the root when none is selected.
    pub fn set_scene_positions(&mut self, scene: PageId, on: bool) -> bool {
        self.message = None;
        let positions = if on {
            let chosen: Vec<_> = self
                .doc
                .selection()
                .iter()
                .copied()
                .filter(|e| matches!(e, kernel::EntityId::Instance(_)))
                .collect();
            if chosen.is_empty() || !self.doc.context().is_empty() {
                ScenePositions::Flag(true)
            } else {
                ScenePositions::Ids(chosen)
            }
        } else {
            ScenePositions::Flag(false)
        };
        self.run(Command::ChangeScene {
            scene,
            name: None,
            camera: None,
            tags: None,
            positions: Some(positions),
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
        self.go_to_scene_at(scene, Instant::now())
    }

    /// `go_to_scene` with the time the transition (if the model setting has one) starts.
    pub(super) fn go_to_scene_at(&mut self, scene: PageId, now: Instant) -> bool {
        let setting = self.doc.scene().scene_transition();
        // Worked out first, from where things are now to where going there puts them.
        let motion = (setting.on && setting.seconds > 0.0)
            .then(|| self.doc.motion_between(None, scene).ok())
            .flatten();
        self.run_camera_command(Command::GoToScene { scene });
        if let Some(motion) = motion
            && self.doc.selected_scene() == Some(scene)
            && !motion.is_still()
        {
            self.transition = Some(Transition {
                motion,
                started: now,
                seconds: setting.seconds,
            });
        }
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
    fn the_positions_checkbox_stores_the_selected_instances_or_all_of_them() {
        let mut e = editor();
        let cmd = |v: serde_json::Value| serde_json::from_value::<Command>(v).unwrap();
        for x in [0.0, 5.0] {
            e.execute(cmd(serde_json::json!({
                "type": "add_rectangle", "corner": [x, 0, 0], "opposite": [x + 1.0, 1, 0]
            })))
            .unwrap();
            let face = e
                .doc()
                .scene()
                .root()
                .faces()
                .map(|(id, _)| kernel::EntityId::Face(id))
                .last()
                .expect("the rectangle's face");
            e.execute(cmd(
                serde_json::json!({"type": "make_group", "ids": [face.to_string()]}),
            ))
            .unwrap();
        }
        let instances: Vec<kernel::EntityId> = e
            .doc()
            .scene()
            .root()
            .instances()
            .map(|(id, _)| kernel::EntityId::Instance(id))
            .collect();
        assert_eq!(instances.len(), 2);
        assert!(e.add_scene());
        let scene = ids(&e)[0];
        // The group just made is selected: only it is stored.
        assert!(e.set_scene_positions(scene, true));
        let stored = |e: &Editor| e.doc().scene().page(scene).unwrap().positions.clone();
        assert_eq!(stored(&e).unwrap().len(), 1);
        assert!(e.set_scene_positions(scene, false));
        assert!(stored(&e).is_none());
        // With nothing selected every instance of the root is stored.
        e.execute(cmd(serde_json::json!({"type": "select", "ids": []})))
            .unwrap();
        assert!(e.set_scene_positions(scene, true));
        assert_eq!(stored(&e).unwrap().len(), 2);
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
    fn updating_a_scene_that_stores_nothing_says_so() {
        let mut e = editor();
        e.add_scene();
        let one = ids(&e)[0];
        e.set_scene_stores(one, Some(false), Some(false));
        let revision = e.doc().revision();
        e.update_scene(one, None, None);
        let message = e.status_parts().message.expect("a status note");
        assert!(message.contains("neither"), "{message}");
        assert_eq!(e.doc().revision(), revision);
        // Ticking a part stores it as before.
        e.update_scene(one, Some(true), None);
        assert!(e.doc().scene().page(one).unwrap().view.is_some());
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

    /// Two groups of one square each (x = 0 and 5); a scene storing their positions and the
    /// front view; then both groups move 4 m and the view turns to the top. Returns the scene
    /// and the first group.
    fn moved_since_a_scene(e: &mut Editor) -> (PageId, kernel::InstanceId) {
        let cmd = |v: serde_json::Value| serde_json::from_value::<Command>(v).unwrap();
        for x in [0.0, 5.0] {
            e.execute(cmd(serde_json::json!({
                "type": "add_rectangle", "corner": [x, 0, 0], "opposite": [x + 1.0, 1, 0]
            })))
            .unwrap();
            let face = e
                .doc()
                .scene()
                .root()
                .faces()
                .map(|(id, _)| kernel::EntityId::Face(id))
                .last()
                .expect("the rectangle's face");
            e.execute(cmd(
                serde_json::json!({"type": "make_group", "ids": [face.to_string()]}),
            ))
            .unwrap();
        }
        let first = e.doc().scene().root().instances().next().unwrap().0;
        e.standard_view(StandardView::Front);
        e.execute(cmd(serde_json::json!({"type": "select", "ids": []})))
            .unwrap();
        assert!(e.add_scene());
        let scene = ids(e)[0];
        assert!(e.set_scene_positions(scene, true));
        e.execute(cmd(serde_json::json!({
            "type": "move",
            "ids": [kernel::EntityId::Instance(first).to_string()],
            "offset": [4, 0, 0]
        })))
        .unwrap();
        e.standard_view(StandardView::Top);
        (scene, first)
    }

    fn x_of(scene: &kernel::Scene, id: kernel::InstanceId) -> f64 {
        scene.root().instance(id).unwrap().transform.translation.x
    }

    #[test]
    fn going_to_a_scene_animates_the_drawing_only_with_smoothstep() {
        use std::time::Duration;
        let mut e = editor();
        let (scene, first) = moved_since_a_scene(&mut e);
        let start_camera = e.doc().camera.clone();
        let t0 = web_time::Instant::now();
        assert!(!e.transitioning());
        assert!(e.go_to_scene_at(scene, t0));
        assert!(e.transitioning());
        // The document is at the end at once, in one undo step.
        assert_eq!(x_of(e.doc().scene(), first), 0.0);
        assert_eq!(e.doc().undo_label(), Some("Go to scene Scene 1"));
        // Drawn: the start at t = 0, halfway at half the time (smoothstep(0.5) = 0.5), done at 1.
        let at = |ms| t0 + Duration::from_millis(ms);
        let f0 = e.transition_frame(at(0)).expect("a frame");
        assert_eq!(f0.camera, start_camera);
        assert_eq!(x_of(&f0.scene, first), 4.0);
        let f1 = e.transition_frame(at(500)).expect("a frame");
        assert_eq!(x_of(&f1.scene, first), 2.0);
        let end = e.doc().camera.clone();
        let want = start_camera.eye.lerp(end.eye, 0.5);
        assert!((f1.camera.eye - want).length() < 1e-9);
        // Smoothstep: a quarter of the time is less than a quarter of the way.
        let f2 = e.transition_frame(at(250)).expect("a frame");
        assert!(x_of(&f2.scene, first) > 4.0 * (1.0 - 0.25), "slow start");
        // The model itself never moved in the frames.
        assert_eq!(x_of(e.doc().scene(), first), 0.0);
        assert!(e.transition_frame(at(1000)).is_none(), "the time is up");
        assert!(!e.transitioning());
    }

    #[test]
    fn input_and_commands_finish_a_transition_at_once() {
        let mut e = editor();
        let (scene, _) = moved_since_a_scene(&mut e);
        let t0 = web_time::Instant::now();
        e.go_to_scene_at(scene, t0);
        assert!(e.finish_transition());
        assert!(!e.finish_transition(), "nothing left to finish");
        assert!(e.transition_frame(t0).is_none());
        // Any command (an agent's, a menu item) ends it too.
        e.standard_view(StandardView::Top);
        e.go_to_scene_at(scene, t0);
        assert!(e.transitioning());
        e.execute(Command::SetLighting {
            mode: session::Lighting::Night,
        })
        .unwrap();
        assert!(!e.transitioning());
    }

    #[test]
    fn nothing_animates_when_off_zero_long_or_with_nothing_to_move() {
        let mut e = editor();
        let (scene, _) = moved_since_a_scene(&mut e);
        let t0 = web_time::Instant::now();
        assert!(e.set_scene_transition(kernel::SceneTransition {
            on: false,
            seconds: 1.0,
        }));
        assert_eq!(e.doc().undo_label(), Some("set_scene_transition"));
        e.go_to_scene_at(scene, t0);
        assert!(!e.transitioning(), "off");
        e.undo();
        e.undo();
        assert_eq!(
            e.doc().scene().scene_transition(),
            kernel::SceneTransition::DEFAULT
        );
        e.standard_view(StandardView::Top);
        assert!(e.set_scene_transition(kernel::SceneTransition {
            on: true,
            seconds: 0.0,
        }));
        e.go_to_scene_at(scene, t0);
        assert!(!e.transitioning(), "zero seconds");
        assert!(
            !e.set_scene_transition(kernel::SceneTransition {
                on: true,
                seconds: 0.0,
            }),
            "unchanged"
        );
        // Already at the scene: nothing differs, nothing to draw.
        e.set_scene_transition(kernel::SceneTransition::DEFAULT);
        e.go_to_scene_at(scene, t0);
        e.finish_transition();
        e.go_to_scene_at(scene, t0);
        assert!(!e.transitioning(), "already there");
    }
}
