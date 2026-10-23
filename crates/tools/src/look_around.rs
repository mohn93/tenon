//! The Look Around tool: drag to turn the view about the eye (0.0025 radian per pixel, right
//! turns right and down looks down, as Fly Mode's look). A typed length sets the eye height
//! above the face directly under the eye, else above z = 0, keeping the heading. The tool only
//! sends camera commands.

use glam::{DVec2, DVec3};
use kernel::EntityId;
use session::{Command, MAX_EYE_HEIGHT, MIN_EYE_HEIGHT, Ray};

use crate::probe::{Reach, pick};
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length};

/// Radians the view turns per pixel of drag: Fly Mode's look rate.
pub const LOOK_RAD_PER_PX: f64 = 0.0025;

#[derive(Clone, Debug, Default)]
pub struct LookAroundTool {
    /// Where the cursor was at the press or the last move of a drag.
    last: Option<DVec2>,
}

impl LookAroundTool {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The point on the ground or the shown face straight below `eye` (the highest face below
/// it), in the open context's coordinates.
fn under_eye(eye: DVec3, ctx: &ToolContext) -> DVec3 {
    let ray = Ray {
        origin: eye,
        dir: DVec3::NEG_Z,
    };
    let floor = pick(ctx.scenery, &ray, Reach::Around { radius: 0.0 })
        .into_iter()
        .find(|h| matches!(h.id, EntityId::Face(_)))
        .map_or(0.0, |h| h.point.z);
    DVec3::new(eye.x, eye.y, floor)
}

/// The eye height typed, if it is a length within the limits `position_camera` accepts.
pub(crate) fn parse_eye_height(text: &str) -> Result<f64, String> {
    match parse_length(text) {
        Some(h) if (MIN_EYE_HEIGHT..=MAX_EYE_HEIGHT).contains(&h) => Ok(h),
        Some(_) => Err(format!(
            "The eye height must be between {} and {}",
            format_length(MIN_EYE_HEIGHT),
            format_length(MAX_EYE_HEIGHT)
        )),
        None => Err(format!("Not an eye height: {text}")),
    }
}

impl Tool for LookAroundTool {
    fn name(&self) -> &'static str {
        "Look Around"
    }

    fn prompt(&self) -> &'static str {
        "Drag to turn the view, or type an eye height"
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("Eye height", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn phase(&self) -> Phase {
        Phase::Idle
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Click(_) => self.last = Some(ctx.cursor),
            ToolEvent::Move(_) => {
                if let Some(last) = self.last.as_mut() {
                    let d = ctx.cursor - std::mem::replace(last, ctx.cursor);
                    if d.is_finite() && d != DVec2::ZERO {
                        out.commands.push(Command::LookAround {
                            yaw: -(d.x * LOOK_RAD_PER_PX).to_degrees(),
                            pitch: -(d.y * LOOK_RAD_PER_PX).to_degrees(),
                        });
                    }
                }
            }
            ToolEvent::Release(_) | ToolEvent::Cancel => self.last = None,
            ToolEvent::Value(text) => match parse_eye_height(&text) {
                Ok(h) => {
                    let point = under_eye(ctx.camera.eye, ctx);
                    out.commands.push(Command::PositionCamera {
                        point: point.to_array(),
                        eye_height: Some(h),
                        look_at: None,
                    });
                }
                Err(e) => out.message = Some(e),
            },
            ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) | ToolEvent::ToggleCopy => {}
        }
        out
    }

    fn on_external_change(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Rig, only};
    use crate::tool::ToolKind;
    use kernel::Model;

    fn rig(model: Model) -> Rig {
        Rig::new(ToolKind::LookAround, model)
    }

    fn look(out: &ToolOutput) -> (f64, f64) {
        match only(out) {
            Command::LookAround { yaw, pitch } => (*yaw, *pitch),
            c => panic!("{c:?}"),
        }
    }

    fn drag_to(r: &mut Rig, x: f64, y: f64) -> ToolOutput {
        r.cursor = DVec2::new(x, y);
        r.send(ToolEvent::Move(crate::inference::Inference {
            point: DVec3::ZERO,
            kind: crate::inference::SnapKind::Ground,
            face: None,
        }))
    }

    fn press(r: &mut Rig, x: f64, y: f64) {
        r.cursor = DVec2::new(x, y);
        r.send(ToolEvent::Click(crate::inference::Inference {
            point: DVec3::ZERO,
            kind: crate::inference::SnapKind::Ground,
            face: None,
        }));
    }

    #[test]
    fn dragging_right_turns_right_at_flys_rate() {
        let mut r = rig(Model::new());
        press(&mut r, 100.0, 100.0);
        let (yaw, pitch) = look(&drag_to(&mut r, 140.0, 100.0));
        assert!(
            (yaw + (40.0 * LOOK_RAD_PER_PX).to_degrees()).abs() < 1e-9,
            "{yaw}"
        );
        assert_eq!(pitch, 0.0);
    }

    #[test]
    fn dragging_down_looks_down_and_moves_are_incremental() {
        let mut r = rig(Model::new());
        press(&mut r, 100.0, 100.0);
        let (_, p1) = look(&drag_to(&mut r, 100.0, 120.0));
        let (_, p2) = look(&drag_to(&mut r, 100.0, 150.0));
        assert!((p1 + (20.0 * LOOK_RAD_PER_PX).to_degrees()).abs() < 1e-9);
        assert!((p2 + (30.0 * LOOK_RAD_PER_PX).to_degrees()).abs() < 1e-9);
    }

    #[test]
    fn moving_without_a_press_or_after_release_does_nothing() {
        let mut r = rig(Model::new());
        assert!(drag_to(&mut r, 10.0, 10.0).commands.is_empty());
        assert!(drag_to(&mut r, 20.0, 10.0).commands.is_empty());
        press(&mut r, 100.0, 100.0);
        r.release(DVec3::ZERO);
        assert!(drag_to(&mut r, 150.0, 100.0).commands.is_empty());
    }

    #[test]
    fn a_typed_height_stands_on_the_ground_under_the_eye() {
        let mut r = rig(Model::new());
        r.camera.eye = DVec3::new(2.0, 3.0, 9.0);
        let out = r.value("1.8");
        assert_eq!(
            only(&out),
            &Command::PositionCamera {
                point: [2.0, 3.0, 0.0],
                eye_height: Some(1.8),
                look_at: None
            }
        );
    }

    #[test]
    fn a_typed_height_stands_on_the_face_under_the_eye() {
        let mut model = Model::new();
        let p = |x, y| DVec3::new(x, y, 2.0);
        model
            .add_polygon(&[p(0.0, 0.0), p(5.0, 0.0), p(5.0, 5.0), p(0.0, 5.0)])
            .unwrap();
        let mut r = rig(model);
        r.camera.eye = DVec3::new(2.0, 3.0, 9.0);
        let out = r.value("5'6");
        match only(&out) {
            Command::PositionCamera {
                point, eye_height, ..
            } => {
                assert_eq!(*point, [2.0, 3.0, 2.0]);
                assert!((eye_height.unwrap() - 1.6764).abs() < 1e-9);
            }
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn a_bad_height_says_so_and_sends_nothing() {
        let mut r = rig(Model::new());
        for text in ["abc", "0", "-1", "20000"] {
            let out = r.value(text);
            assert!(out.commands.is_empty(), "{text}");
            assert!(out.message.is_some(), "{text}");
        }
    }
}
