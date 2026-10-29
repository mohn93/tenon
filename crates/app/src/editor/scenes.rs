//! The scene tabs and the Scenes tab: each change is the command an agent would send.

use kernel::{JointId, PageId};
use session::motion::{Motion, smoothstep};
use session::{Camera, Command, CommandError, ErrorKind, SceneJoints, ScenePositions};
use web_time::Instant;

use super::{Editor, Message};

/// An animation in progress, drawing only: a scene transition (the document is already at the
/// end) or a joint being played (the document stays as it is).
pub(super) struct Transition {
    kind: Kind,
    started: Instant,
    seconds: f64,
}

enum Kind {
    Motion(Box<Glide>),
    Play(Play),
}

/// A scene transition that moves parts.
struct Glide {
    motion: Motion,
    /// The frame drawn last, its joints solved at their values then: each frame solves on
    /// from it, a small step, rather than from the document's pose (None before the first).
    frame: Option<kernel::Scene>,
}

/// A joint played from one value to another (radians or meters).
struct Play {
    joint: JointId,
    from: f64,
    to: f64,
    looping: bool,
    /// A copy of the model with the joint at `from`: a looping play starts each pass again
    /// from it rather than driving back over the whole range.
    start: Box<kernel::Scene>,
    /// The frame drawn last, a copy of the model with the joint at its value then: each frame
    /// solves on from it, a small step, rather than from the document's pose.
    frame: Box<kernel::Scene>,
    /// How far along the pass (0 to 1) the last frame was.
    along: f64,
}

/// The longest a play may take, in seconds.
const MAX_PLAY_SECONDS: f64 = 600.0;

/// How a play started from the Joints tab runs: seconds for a pass.
const PLAY_SECONDS: f64 = 4.0;
/// How far (meters) a play of a prismatic joint without limits slides.
const PLAY_REACH: f64 = 0.5;

/// What the window draws while a transition runs: the camera and, when instances move, the
/// scene with them where the transition has them (None: the document's scene, unchanged).
pub struct TransitionFrame {
    pub camera: Camera,
    pub scene: Option<kernel::Scene>,
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

    /// The camera and scene to draw at `now`, or None when nothing animates (a transition or
    /// a play that has run its time ends here; a looping play never does). A transition
    /// eases by smoothstep and its camera keeps the window's viewport; a play runs the joint
    /// linearly and draws the document's camera.
    pub fn transition_frame(&mut self, now: Instant) -> Option<TransitionFrame> {
        let t = self.transition.as_mut()?;
        let x = now.saturating_duration_since(t.started).as_secs_f64() / t.seconds;
        let looping = matches!(&t.kind, Kind::Play(p) if p.looping);
        if x >= 1.0 && !looping {
            self.transition = None;
            return None;
        }
        match &mut t.kind {
            Kind::Motion(glide) => {
                let eased = smoothstep(x);
                let mut camera = glide.motion.camera_at(eased);
                camera.viewport = self.doc.camera.viewport;
                // A camera-only transition draws the document's scene: no copy, no model
                // rebuild.
                let scene = glide.motion.moves_parts().then(|| {
                    let frame =
                        glide
                            .motion
                            .scene_after(self.doc.scene(), glide.frame.as_ref(), eased);
                    glide.frame = Some(frame.clone());
                    frame
                });
                Some(TransitionFrame { camera, scene })
            }
            Kind::Play(play) => {
                let along = if looping { x.fract() } else { x };
                if along < play.along {
                    // A new pass starts from the start, not by driving back over the range.
                    play.frame.clone_from(&play.start);
                }
                play.along = along;
                let value = play.from + (play.to - play.from) * along;
                // A value that cannot be met leaves the frame as the last one drew it.
                let _ = play.frame.drive_joint(play.joint, value, None);
                Some(TransitionFrame {
                    camera: self.doc.camera.clone(),
                    scene: Some((*play.frame).clone()),
                })
            }
        }
    }

    /// Plays a joint: for `seconds` its value runs from `from` to `to` (degrees or meters),
    /// the window drawing the model solved at each value, the document unchanged. Any input
    /// or command stops it. `invalid_input` for a joint that cannot be driven, values outside
    /// its limits or that cannot be met (both ends are solved first), or a time that is not
    /// above 0 and at most 600 seconds.
    pub fn play_joint(
        &mut self,
        joint: JointId,
        from: f64,
        to: f64,
        seconds: f64,
        looping: bool,
    ) -> Result<(), CommandError> {
        self.play_joint_at(joint, from, to, seconds, looping, Instant::now())
    }

    pub(super) fn play_joint_at(
        &mut self,
        joint: JointId,
        from: f64,
        to: f64,
        seconds: f64,
        looping: bool,
        now: Instant,
    ) -> Result<(), CommandError> {
        let invalid = |why: &str| CommandError::new(ErrorKind::InvalidInput, why);
        if !from.is_finite() || !to.is_finite() {
            return Err(invalid("from and to must be finite numbers"));
        }
        if !seconds.is_finite() || seconds <= 0.0 || seconds > MAX_PLAY_SECONDS {
            return Err(invalid("seconds must be above 0 and at most 600"));
        }
        // Both ends are solved on a copy first: the play then only meets what it was told.
        let start = self
            .doc
            .scene_with_joints(self.doc.scene().clone(), &[(joint, from)])?;
        self.doc.scene_with_joints(start.clone(), &[(joint, to)])?;
        let angular = self
            .doc
            .scene()
            .joint(joint)
            .is_some_and(|j| j.kind.is_angular());
        let unit = if angular { 1f64.to_radians() } else { 1.0 };
        self.message = None;
        self.transition = Some(Transition {
            kind: Kind::Play(Play {
                joint,
                from: from * unit,
                to: to * unit,
                looping,
                frame: Box::new(start.clone()),
                start: Box::new(start),
                along: 0.0,
            }),
            started: now,
            seconds,
        });
        Ok(())
    }

    /// The Joints tab's Play button: plays the joint over its limits (looping a joint that
    /// turns without limits through a whole turn from where it is). A refusal says why in the
    /// status.
    pub fn play_joint_range(&mut self, joint: JointId) -> bool {
        let Some(info) = self.joint_infos().into_iter().find(|j| j.id == joint) else {
            return false;
        };
        let (from, to, looping) = match info.limits {
            Some([lo, hi]) => (lo, hi, false),
            None if info.kind.is_angular() => (info.value, info.value + 360.0, true),
            None => (info.value, info.value + PLAY_REACH, false),
        };
        if let Err(e) = self.play_joint(joint, from, to, PLAY_SECONDS, looping) {
            self.message = Some(Message::error(e.message));
        }
        true
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
            joints: None,
            sections: None,
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
            .is_some_and(|p| p.view.is_none() && p.tags.is_none() && p.sections.is_none());
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
            joints: None,
            sections: None,
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
            joints: None,
            sections: None,
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
            joints: None,
            sections: None,
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
            joints: None,
            sections: None,
        });
        true
    }

    /// Starts (true) or stops storing joint values in a scene. Started, it stores the current
    /// value of every joint that has one.
    pub fn set_scene_joints(&mut self, scene: PageId, on: bool) -> bool {
        self.message = None;
        self.run(Command::ChangeScene {
            scene,
            name: None,
            camera: None,
            tags: None,
            positions: None,
            joints: Some(SceneJoints::Flag(on)),
            sections: None,
        });
        true
    }

    /// Starts (true) or stops storing the active section planes and the section display in a
    /// scene. Started, it stores them as they are now.
    pub fn set_scene_sections(&mut self, scene: PageId, on: bool) -> bool {
        self.message = None;
        self.run(Command::ChangeScene {
            scene,
            name: None,
            camera: None,
            tags: None,
            positions: None,
            joints: None,
            sections: Some(on),
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
                kind: Kind::Motion(Box::new(Glide {
                    motion,
                    frame: None,
                })),
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
    use session::{Command, ErrorKind, StandardView};

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
        assert_eq!(x_of(f0.scene.as_ref().unwrap(), first), 4.0);
        let f1 = e.transition_frame(at(500)).expect("a frame");
        assert_eq!(x_of(f1.scene.as_ref().unwrap(), first), 2.0);
        let end = e.doc().camera.clone();
        let want = session::motion::lerp_camera(&start_camera, &end, 0.5).eye;
        assert!((f1.camera.eye - want).length() < 1e-9);
        // Smoothstep: a quarter of the time is less than a quarter of the way.
        let f2 = e.transition_frame(at(250)).expect("a frame");
        assert!(
            x_of(f2.scene.as_ref().unwrap(), first) > 4.0 * (1.0 - 0.25),
            "slow start"
        );
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

    #[test]
    fn a_camera_only_transition_draws_the_documents_scene() {
        use std::time::Duration;
        let mut e = editor();
        e.standard_view(StandardView::Front);
        assert!(e.add_scene());
        let scene = ids(&e)[0];
        e.standard_view(StandardView::Top);
        let t0 = web_time::Instant::now();
        assert!(e.go_to_scene_at(scene, t0));
        let frame = e
            .transition_frame(t0 + Duration::from_millis(300))
            .expect("a frame");
        assert!(frame.scene.is_none(), "no instance moves: no scene copy");
        assert_ne!(frame.camera, e.doc().camera);
    }

    #[test]
    fn labels_during_a_transition_follow_the_drawn_camera_and_instances() {
        use std::time::Duration;
        let mut e = editor();
        let (scene, first) = moved_since_a_scene(&mut e);
        // A note inside the first group, which the transition moves.
        let group = kernel::EntityId::Instance(first);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.execute(Command::AddNote {
            anchor: session::AnchorInput::Point([0.5, 0.5, 0.0]),
            position: [0.5, 0.5, 1.0],
            text: Some("A".into()),
        })
        .unwrap();
        e.execute(Command::CloseInstance {}).unwrap();
        let t0 = web_time::Instant::now();
        assert!(e.go_to_scene_at(scene, t0));
        let frame = e
            .transition_frame(t0 + Duration::from_millis(500))
            .expect("a frame");
        let moved = frame.scene.as_ref().expect("an instance moves");
        let place = moved.root().instance(first).unwrap().transform;
        let want = frame
            .camera
            .project(place.transform_point3(glam::DVec3::new(0.5, 0.5, 1.0)))
            .unwrap();
        let drawn = e.transition_hud(false, &frame).labels;
        assert_eq!(drawn.len(), 1);
        assert!(
            (drawn[0].at - want).length() < 1e-6,
            "{:?} vs {want}",
            drawn[0].at
        );
        // The document's view puts it elsewhere.
        let end = e.hud(false).labels;
        assert!((end[0].at - want).length() > 1.0);
    }

    /// A unit box hinged to the ground about the vertical line through (3, 0): its instance
    /// and joint.
    fn hinged_box(e: &mut Editor) -> (kernel::InstanceId, kernel::JointId) {
        let cmd = |v: serde_json::Value| serde_json::from_value::<Command>(v).unwrap();
        e.execute(cmd(serde_json::json!({
            "type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1]
        })))
        .unwrap();
        let part = e.doc().scene().root().instances().next().unwrap().0;
        let out = e
            .execute(cmd(serde_json::json!({
                "type": "add_joint", "kind": "revolute", "b": kernel::EntityId::Instance(part).to_string(),
                "point": [3, 0, 0], "axis": [0, 0, 1]
            })))
            .unwrap();
        (part, out.changes.joints[0])
    }

    /// The turn (degrees about Z) of an instance of `scene`.
    fn turn_of(scene: &kernel::Scene, id: kernel::InstanceId) -> f64 {
        let x = scene.root().instance(id).unwrap().transform.matrix3 * glam::DVec3::X;
        x.y.atan2(x.x).to_degrees()
    }

    #[test]
    fn play_draws_the_joint_at_interpolated_values_and_changes_nothing() {
        use std::time::Duration;
        let mut e = editor();
        let (part, joint) = hinged_box(&mut e);
        let revision = e.doc().revision();
        let undo = e.doc().undo_label().map(str::to_string);
        let t0 = web_time::Instant::now();
        e.play_joint_at(joint, 0.0, 90.0, 2.0, false, t0).unwrap();
        assert!(e.transitioning());
        let at = |ms| t0 + Duration::from_millis(ms);
        // Linear: a quarter of the time is a quarter of the turn, and the camera is the
        // document's.
        let frame = e.transition_frame(at(500)).expect("a frame");
        assert!((turn_of(frame.scene.as_ref().unwrap(), part) - 22.5).abs() < 1e-6);
        assert_eq!(frame.camera, e.doc().camera);
        let frame = e.transition_frame(at(1500)).expect("a frame");
        assert!((turn_of(frame.scene.as_ref().unwrap(), part) - 67.5).abs() < 1e-6);
        // The document never moved and made no step.
        assert!(turn_of(e.doc().scene(), part).abs() < 1e-9);
        assert_eq!(e.doc().revision(), revision);
        assert_eq!(e.doc().undo_label().map(str::to_string), undo);
        // The time is up: the window draws the document again.
        assert!(e.transition_frame(at(2000)).is_none());
        assert!(!e.transitioning());
    }

    #[test]
    fn a_looping_play_starts_again_and_input_stops_it_leaving_the_document() {
        use std::time::Duration;
        let mut e = editor();
        let (part, joint) = hinged_box(&mut e);
        let revision = e.doc().revision();
        let t0 = web_time::Instant::now();
        e.play_joint_at(joint, 0.0, 90.0, 2.0, true, t0).unwrap();
        let at = |ms| t0 + Duration::from_millis(ms);
        // Past the time it is on its second pass: 3 s in is 1 s into the pass.
        let frame = e.transition_frame(at(3000)).expect("still playing");
        assert!((turn_of(frame.scene.as_ref().unwrap(), part) - 45.0).abs() < 1e-6);
        assert!(e.transitioning());
        // Input stops it (the window calls this on a press): the document is as it was.
        assert!(e.finish_transition());
        assert!(e.transition_frame(at(3100)).is_none());
        assert!(turn_of(e.doc().scene(), part).abs() < 1e-9);
        assert_eq!(e.doc().revision(), revision);
        // A command stops it too.
        e.play_joint_at(joint, 0.0, 90.0, 2.0, true, t0).unwrap();
        e.execute(Command::SetLighting {
            mode: session::Lighting::Night,
        })
        .unwrap();
        assert!(!e.transitioning());
    }

    #[test]
    fn a_looping_play_starts_each_pass_from_the_start_without_driving_back() {
        use std::time::Duration;
        let mut e = editor();
        let (part, joint) = hinged_box(&mut e);
        let t0 = web_time::Instant::now();
        e.play_joint_at(joint, 0.0, 90.0, 2.0, true, t0).unwrap();
        let at = |ms| t0 + Duration::from_millis(ms);
        let frame = e.transition_frame(at(1900)).expect("playing");
        assert!((turn_of(frame.scene.as_ref().unwrap(), part) - 85.5).abs() < 1e-6);
        // Into the next pass: one small step from the start, not 9 back over the range.
        let steps = kernel::solve_steps_taken();
        let frame = e.transition_frame(at(2100)).expect("playing");
        assert!((turn_of(frame.scene.as_ref().unwrap(), part) - 4.5).abs() < 1e-6);
        assert_eq!(kernel::solve_steps_taken() - steps, 1);
    }

    #[test]
    fn a_play_that_cannot_run_is_refused_before_it_starts() {
        let mut e = editor();
        let (_, joint) = hinged_box(&mut e);
        let cmd = |v: serde_json::Value| serde_json::from_value::<Command>(v).unwrap();
        e.execute(cmd(serde_json::json!({
            "type": "change_joint", "joint": joint.to_string(), "limits": [-10, 50]
        })))
        .unwrap();
        let t0 = web_time::Instant::now();
        let err = e
            .play_joint_at(joint, 0.0, 80.0, 2.0, false, t0)
            .unwrap_err();
        assert!(err.message.contains("limits"), "{}", err.message);
        for (from, to, seconds) in [
            (0.0, 10.0, 0.0),
            (0.0, 10.0, 601.0),
            (0.0, 10.0, f64::NAN),
            (f64::NAN, 10.0, 1.0),
        ] {
            assert!(
                e.play_joint_at(joint, from, to, seconds, false, t0)
                    .is_err()
            );
        }
        assert!(!e.transitioning());
        let missing = kernel::JointId(9999);
        let err = e
            .play_joint_at(missing, 0.0, 1.0, 1.0, false, t0)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        // Within the limits it plays.
        e.play_joint_at(joint, -10.0, 50.0, 1.0, false, t0).unwrap();
        assert!(e.transitioning());
    }

    #[test]
    fn the_play_button_plays_over_the_limits_or_a_turn_and_says_why_when_it_cannot() {
        let mut e = editor();
        let (_, joint) = hinged_box(&mut e);
        assert!(e.play_joint_range(joint));
        assert!(e.transitioning(), "a whole turn");
        e.finish_transition();
        let cmd = |v: serde_json::Value| serde_json::from_value::<Command>(v).unwrap();
        e.execute(cmd(serde_json::json!({
            "type": "change_joint", "joint": joint.to_string(), "limits": [-10, 50]
        })))
        .unwrap();
        assert!(e.play_joint_range(joint));
        assert!(e.transitioning(), "over the limits");
        e.finish_transition();
        // A joint that cannot be played says why in the status.
        e.execute(cmd(serde_json::json!({
            "type": "add_joint", "kind": "fixed",
            "b": kernel::EntityId::Instance(e.doc().scene().root().instances().next().unwrap().0).to_string(),
            "point": [0, 0, 0], "axis": [0, 0, 1]
        })))
        .unwrap();
        let fixed = e.joint_infos().last().unwrap().id;
        assert!(e.play_joint_range(fixed));
        assert!(!e.transitioning());
        assert!(e.status_parts().message.is_some());
    }

    #[test]
    fn a_transition_interpolates_joint_values_and_solves_each_frame() {
        use std::time::Duration;
        let mut e = editor();
        let (part, joint) = hinged_box(&mut e);
        let cmd = |v: serde_json::Value| serde_json::from_value::<Command>(v).unwrap();
        e.execute(cmd(serde_json::json!({
            "type": "drive", "joint": joint.to_string(), "value": 60
        })))
        .unwrap();
        e.execute(cmd(serde_json::json!({
            "type": "add_scene", "joints": true
        })))
        .unwrap();
        let scene = ids(&e)[0];
        e.execute(cmd(serde_json::json!({
            "type": "drive", "joint": joint.to_string(), "value": 0
        })))
        .unwrap();
        let t0 = web_time::Instant::now();
        assert!(e.go_to_scene_at(scene, t0));
        assert!(e.transitioning());
        // The document is at the scene at once, in one undo step.
        assert!((turn_of(e.doc().scene(), part) - 60.0).abs() < 1e-6);
        assert_eq!(e.doc().undo_label(), Some("Go to scene Scene 1"));
        let at = |ms| t0 + Duration::from_millis(ms);
        let f0 = e.transition_frame(at(0)).expect("a frame");
        assert!(turn_of(f0.scene.as_ref().unwrap(), part).abs() < 1e-6);
        let f1 = e.transition_frame(at(500)).expect("a frame");
        assert!((turn_of(f1.scene.as_ref().unwrap(), part) - 30.0).abs() < 1e-6);
        assert!((turn_of(e.doc().scene(), part) - 60.0).abs() < 1e-6);
        assert!(e.transition_frame(at(1000)).is_none());
    }

    #[test]
    fn the_joint_values_checkbox_stores_every_joint_value_or_none() {
        let mut e = editor();
        let (_, joint) = hinged_box(&mut e);
        assert!(e.add_scene());
        let scene = ids(&e)[0];
        assert!(e.set_scene_joints(scene, true));
        let stored = |e: &Editor| e.doc().scene().page(scene).unwrap().joints.clone();
        assert_eq!(
            stored(&e).unwrap().keys().copied().collect::<Vec<_>>(),
            [joint]
        );
        assert_eq!(e.doc().undo_label(), Some("change_scene"));
        assert!(e.set_scene_joints(scene, false));
        assert!(stored(&e).is_none());
    }
}
