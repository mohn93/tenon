//! The Joints tab: selecting a joint's parts, deleting it, and driving it with a slider. Each
//! change is the command an agent would send; a drag is the exception: its frames drive the
//! joint without leaving undo steps (they are undone when it ends) and the end makes one.

use kernel::{EntityId, JointId};
use session::{Command, Query, QueryResult, SelectMode};

use super::Editor;

/// A slider drag in progress: each frame's drive is an undo step until the drag ends.
pub(super) struct JointDrag {
    joint: JointId,
    /// The frames that made an undo step so far.
    steps: usize,
    /// The document's revision after the last of them: anything else since means the history
    /// is no longer only these steps.
    revision: u64,
    /// The last value and slide the joint was driven to.
    good: (f64, Option<f64>),
}

impl Editor {
    /// Every joint as the session lists it, in the order they were made.
    pub fn joint_infos(&self) -> Vec<session::JointInfo> {
        match self.doc.query(&Query::Joints {}) {
            Ok(QueryResult::Joints { joints, .. }) => joints,
            _ => Vec::new(),
        }
    }

    /// Selects the two parts of a joint (just its part B for a joint to the ground).
    pub fn select_joint(&mut self, joint: JointId) -> bool {
        self.message = None;
        let Some(info) = self.joint_infos().into_iter().find(|j| j.id == joint) else {
            return false;
        };
        let ids: Vec<EntityId> = info.a.into_iter().chain([info.b]).collect();
        self.run(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        });
        true
    }

    /// Whether every part of the joint is selected.
    pub fn joint_selected(&self, info: &session::JointInfo) -> bool {
        let selection = self.doc.selection();
        selection.contains(&info.b) && info.a.is_none_or(|a| selection.contains(&a))
    }

    /// Deletes a joint, as one undo step.
    pub fn delete_joint(&mut self, joint: JointId) -> bool {
        self.message = None;
        self.joint_drag = None;
        self.run(Command::DeleteJoint { joint }).is_some()
    }

    /// Drives a joint to `value` (degrees or meters) and `slide`. With `done` false this is a
    /// frame of a drag: the pose follows with no undo step kept (a refusal leaves the last
    /// pose and says why in the status). With `done` true the drag ends (or a value was
    /// entered): the frames are undone and the joint is driven to the last pose that was met
    /// in one undo step.
    pub fn drive_joint(&mut self, joint: JointId, value: f64, slide: Option<f64>, done: bool) {
        if self.joint_drag.as_ref().is_some_and(|d| d.joint != joint) {
            self.end_joint_drag();
        }
        self.message = None;
        let current = self
            .joint_infos()
            .into_iter()
            .find(|j| j.id == joint)
            .map(|j| (j.value, j.slide));
        if self.joint_drag.is_none() {
            self.joint_drag = Some(JointDrag {
                joint,
                steps: 0,
                revision: self.doc.revision(),
                good: current.unwrap_or((value, slide)),
            });
        }
        let before = self.doc.revision();
        let driven = self.run(Command::Drive {
            joint,
            value,
            slide,
        });
        let stepped = driven.is_some() && self.doc.revision() != before;
        let revision = self.doc.revision();
        if let Some(drag) = self.joint_drag.as_mut() {
            if driven.is_some() {
                drag.good = (value, slide.or(drag.good.1));
            }
            if stepped {
                drag.steps += 1;
                drag.revision = revision;
            }
        }
        if done {
            self.end_joint_drag();
        }
    }

    /// Ends the drag: its frames become one undo step.
    fn end_joint_drag(&mut self) {
        let Some(drag) = self.joint_drag.take() else {
            return;
        };
        if drag.steps == 0 || self.doc.revision() != drag.revision {
            return;
        }
        let shown = self.message.take();
        let undone = self.run(Command::Undo {
            count: u32::try_from(drag.steps).ok(),
            expect: None,
        });
        if undone.is_some() {
            let (value, slide) = drag.good;
            self.run(Command::Drive {
                joint: drag.joint,
                value,
                slide,
            });
        }
        // The refusal that was shown stays until something else is said.
        if self.message.is_none() {
            self.message = shown;
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use session::Command;

    use super::super::MessageKind;
    use super::*;

    fn cmd(v: serde_json::Value) -> Command {
        serde_json::from_value(v).unwrap()
    }

    /// Two boxes (x = 0 and 5) and a hinge between them at x = 2 about z, with `limits`.
    fn hinged(limits: Option<[f64; 2]>) -> (Editor, JointId, EntityId, EntityId) {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let mut part = |x: f64| {
            let out = e
                .execute(cmd(serde_json::json!({
                    "type": "add_box", "corner": [x, 0.0, 0.0], "size": [1.0, 1.0, 1.0]
                })))
                .unwrap();
            out.placed.unwrap()[0]
        };
        let (a, b) = (part(0.0), part(5.0));
        let out = e
            .execute(cmd(serde_json::json!({
                "type": "add_joint", "kind": "revolute", "a": a.to_string(),
                "b": b.to_string(), "point": [2.0, 0.0, 0.0], "axis": [0, 0, 1],
                "limits": limits
            })))
            .unwrap();
        (e, out.changes.joints[0], a, b)
    }

    fn value(e: &Editor) -> f64 {
        e.joint_infos()[0].value
    }

    fn moved(e: &Editor, part: EntityId) -> glam::DVec3 {
        let EntityId::Instance(i) = part else {
            panic!()
        };
        e.doc()
            .scene()
            .root()
            .instance(i)
            .unwrap()
            .transform
            .translation
    }

    #[test]
    fn the_rows_list_joints_with_kind_and_value() {
        let (mut e, j, ..) = hinged(None);
        e.drive_joint(j, 30.0, None, true);
        let [info] = &e.joint_infos()[..] else {
            panic!()
        };
        assert_eq!(info.kind, kernel::JointKind::Revolute);
        assert!((info.value - 30.0).abs() < 1e-6);
        assert_eq!(info.name, "Joint 1");
    }

    #[test]
    fn a_drag_follows_live_and_makes_one_undo_step_when_it_ends() {
        let (mut e, j, _, b) = hinged(None);
        let start = moved(&e, b);
        assert_eq!(e.doc().undo_label(), Some("add_joint"));
        for v in [10.0, 20.0, 35.0, 50.0] {
            e.drive_joint(j, v, None, false);
            assert!((value(&e) - v).abs() < 1e-6, "the pose follows each frame");
        }
        assert_ne!(moved(&e, b), start, "the part moved");
        let live = moved(&e, b);
        e.drive_joint(j, 50.0, None, true);
        assert!((value(&e) - 50.0).abs() < 1e-6);
        assert!(
            (moved(&e, b) - live).length() < 1e-6,
            "the end changes no pose"
        );
        assert_eq!(e.doc().undo_label(), Some("drive"));
        assert!(e.undo());
        assert_eq!(e.doc().undo_label(), Some("add_joint"), "one step");
        assert!(value(&e).abs() < 1e-6);
        assert!((moved(&e, b) - start).length() < 1e-6);
        assert!(e.redo());
        assert!((value(&e) - 50.0).abs() < 1e-6);
    }

    #[test]
    fn a_drag_that_returns_to_its_start_leaves_no_step() {
        let (mut e, j, ..) = hinged(None);
        e.drive_joint(j, 40.0, None, false);
        e.drive_joint(j, 0.0, None, true);
        assert_eq!(e.doc().undo_label(), Some("add_joint"));
        assert!(value(&e).abs() < 1e-6);
    }

    #[test]
    fn a_refused_drive_keeps_the_last_pose_and_says_why() {
        let (mut e, j, _, b) = hinged(Some([-45.0, 45.0]));
        e.drive_joint(j, 30.0, None, false);
        let good = moved(&e, b);
        e.drive_joint(j, 60.0, None, false);
        let message = e.message.clone().expect("the reason");
        assert_eq!(message.kind, MessageKind::Error);
        assert!(message.text.contains("limit"), "{}", message.text);
        assert!((value(&e) - 30.0).abs() < 1e-6);
        assert!((moved(&e, b) - good).length() < 1e-6);
        e.drive_joint(j, 60.0, None, true);
        assert!((value(&e) - 30.0).abs() < 1e-6, "the last good pose stays");
        assert!(e.message.is_some(), "the reason stays in the status");
        assert_eq!(e.doc().undo_label(), Some("drive"));
        assert!(e.undo());
        assert_eq!(e.doc().undo_label(), Some("add_joint"));
    }

    #[test]
    fn an_entered_value_is_one_undo_step() {
        let (mut e, j, ..) = hinged(None);
        e.drive_joint(j, 25.0, None, true);
        assert_eq!(e.doc().undo_label(), Some("drive"));
        assert!(e.undo());
        assert_eq!(e.doc().undo_label(), Some("add_joint"));
    }

    #[test]
    fn selecting_a_joint_selects_its_two_parts() {
        let (mut e, j, a, b) = hinged(None);
        let info = e.joint_infos().remove(0);
        assert!(!e.joint_selected(&info));
        assert!(e.select_joint(j));
        assert_eq!(e.doc().selection(), &[a, b].into_iter().collect());
        assert!(e.joint_selected(&info));
    }

    #[test]
    fn deleting_a_joint_leaves_the_parts_where_they_are() {
        let (mut e, j, _, b) = hinged(None);
        e.drive_joint(j, 40.0, None, true);
        let pose = moved(&e, b);
        assert!(e.delete_joint(j));
        assert!(e.joint_infos().is_empty());
        assert!((moved(&e, b) - pose).length() < 1e-9);
        assert!(e.undo());
        assert_eq!(e.joint_infos().len(), 1);
    }
}
