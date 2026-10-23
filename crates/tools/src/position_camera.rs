//! The Position Camera tool: click a point to stand there with the eye at the eye height
//! (remembered for the session), or press at A and drag to B to stand at A looking toward B.
//! A typed length before the click sets the eye height. Once the camera is placed the editor
//! switches to the Look Around tool. The tool only sends camera commands.

use glam::DVec3;
use session::{Command, DEFAULT_EYE_HEIGHT};

use crate::look_around::parse_eye_height;
use crate::tool::{
    DragStart, Phase, Remembered, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput,
};
use crate::vcb::format_length;

#[derive(Clone, Debug)]
pub struct PositionCameraTool {
    eye_height: f64,
    /// Where the button went down, until its release.
    standing: Option<DVec3>,
    drag_start: DragStart,
}

impl Default for PositionCameraTool {
    fn default() -> Self {
        Self::with_eye_height(None)
    }
}

impl PositionCameraTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The tool with the eye height of an earlier use (None: the default, 5'6").
    pub fn with_eye_height(height: Option<f64>) -> Self {
        PositionCameraTool {
            eye_height: height.unwrap_or(DEFAULT_EYE_HEIGHT),
            standing: None,
            drag_start: DragStart::default(),
        }
    }

    fn place(&mut self, point: DVec3, look_at: Option<DVec3>, out: &mut ToolOutput) {
        out.commands.push(Command::PositionCamera {
            point: point.to_array(),
            eye_height: Some(self.eye_height),
            look_at: look_at.map(|p| p.to_array()),
        });
        out.next_tool = Some(ToolKind::LookAround);
    }
}

impl Tool for PositionCameraTool {
    fn name(&self) -> &'static str {
        "Position Camera"
    }

    fn prompt(&self) -> &'static str {
        match self.standing {
            None => "Click to stand there, or drag to look toward a point; type an eye height",
            Some(_) => "Release where the view should look",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("Eye height", Some(format_length(self.eye_height)))
    }

    fn anchor(&self) -> Option<DVec3> {
        self.standing
    }

    fn phase(&self) -> Phase {
        match self.standing {
            Some(p) => Phase::Active { points: vec![p] },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Click(hit) => {
                self.standing = Some(hit.point);
                self.drag_start.arm(ctx.cursor);
            }
            ToolEvent::Move(hit) => {
                if let Some(a) = self.standing {
                    out.preview = vec![[a, hit.point]];
                }
            }
            ToolEvent::Release(hit) => {
                let dragged = self.drag_start.released(ctx);
                if let Some(a) = self.standing.take() {
                    self.place(a, dragged.then_some(hit.point), &mut out);
                }
            }
            ToolEvent::Value(text) => match parse_eye_height(&text) {
                Ok(h) => {
                    self.eye_height = h;
                    out.message = Some(format!("Eye height {}", format_length(h)));
                    out.info = true;
                }
                Err(e) => out.message = Some(e),
            },
            ToolEvent::Cancel => self.on_external_change(),
            ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) | ToolEvent::ToggleCopy => {}
        }
        out
    }

    fn on_external_change(&mut self) {
        self.standing = None;
        self.drag_start = DragStart::default();
    }

    fn remember(&self, r: &mut Remembered) {
        r.eye_height = Some(self.eye_height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Rig, only};
    use kernel::Model;

    fn rig() -> Rig {
        Rig::new(ToolKind::PositionCamera, Model::new())
    }

    #[test]
    fn a_click_stands_at_the_point_at_the_default_height() {
        let mut r = rig();
        let p = DVec3::new(2.0, 3.0, 0.0);
        assert!(r.click(p).commands.is_empty());
        let out = r.release(p);
        assert_eq!(
            only(&out),
            &Command::PositionCamera {
                point: [2.0, 3.0, 0.0],
                eye_height: Some(DEFAULT_EYE_HEIGHT),
                look_at: None
            }
        );
        assert_eq!(out.next_tool, Some(ToolKind::LookAround));
    }

    #[test]
    fn a_drag_stands_at_the_press_looking_at_the_release() {
        let mut r = rig();
        let (a, b) = (DVec3::new(0.0, 0.0, 0.0), DVec3::new(4.0, 5.0, 0.0));
        r.click(a);
        let moved = r.mv(b);
        assert_eq!(moved.preview, vec![[a, b]]);
        let out = r.release(b);
        assert_eq!(
            only(&out),
            &Command::PositionCamera {
                point: a.to_array(),
                eye_height: Some(DEFAULT_EYE_HEIGHT),
                look_at: Some(b.to_array())
            }
        );
        assert_eq!(out.next_tool, Some(ToolKind::LookAround));
    }

    #[test]
    fn a_typed_height_is_used_and_remembered() {
        let mut r = rig();
        let out = r.value("1.2");
        assert!(out.info && out.commands.is_empty());
        let p = DVec3::new(1.0, 1.0, 0.0);
        r.click(p);
        match only(&r.release(p)) {
            Command::PositionCamera { eye_height, .. } => assert_eq!(*eye_height, Some(1.2)),
            c => panic!("{c:?}"),
        }
        let mut kept = Remembered::default();
        r.tool.remember(&mut kept);
        assert_eq!(kept.eye_height, Some(1.2));
        let mut next = ToolKind::PositionCamera.make_with(&kept);
        assert_eq!(next.measurement().1.as_deref(), Some("1.2 m"));
        next.on_external_change();
    }

    #[test]
    fn a_bad_height_is_refused_and_escape_drops_the_press() {
        let mut r = rig();
        let out = r.value("0");
        assert!(out.message.is_some() && !out.info);
        r.click(DVec3::ZERO);
        assert!(matches!(r.tool.phase(), Phase::Active { .. }));
        r.send(ToolEvent::Cancel);
        assert_eq!(r.tool.phase(), Phase::Idle);
        assert!(r.release(DVec3::ZERO).commands.is_empty());
    }
}
