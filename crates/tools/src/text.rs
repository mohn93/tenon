//! The Text tool: adds notes. Click a point on the model (a vertex, which the note follows, or
//! any point on an edge, a face or a guide), then click where the text goes: a leader note.
//! Or click empty space: a note fixed to the view there (only the root holds those). Then the
//! text is asked for (the window's note sheet; a typed value); for a leader note no text
//! (an empty value) gives its automatic text, the point's coordinates.

use glam::{DVec2, DVec3};
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::dimension::End;
use crate::inference::{Inference, SnapKind};
use crate::tool::{NoteRequest, Phase, Tool, ToolContext, ToolEvent, ToolOutput};

/// Where the tool is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Idle,
    /// The point a leader note points at was clicked; where its text goes comes next.
    Leader(End),
    /// Where the note goes is known; its text comes next.
    Text(NoteRequest),
}

#[derive(Clone, Debug)]
pub struct TextTool {
    step: Step,
    /// The point under the cursor.
    hover: Option<DVec3>,
}

impl Default for TextTool {
    fn default() -> Self {
        TextTool {
            step: Step::Idle,
            hover: None,
        }
    }
}

/// Whether a click at `hit` lands on the model, rather than on the ground or in empty space.
fn on_model(hit: &Inference) -> bool {
    !matches!(
        hit.kind,
        SnapKind::Ground
            | SnapKind::Free
            | SnapKind::Axis(_)
            | SnapKind::Parallel
            | SnapKind::Perpendicular
    )
}

impl TextTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// What a click at `hit` does while nothing is in progress.
    fn start(&mut self, hit: &Inference, ctx: &ToolContext, out: &mut ToolOutput) {
        if on_model(hit) {
            self.step = Step::Leader(End::at(hit, ctx));
            return;
        }
        if ctx.scenery.inside_instance() {
            out.message =
                Some("Notes fixed to the view go in the model's root: close the group".into());
            return;
        }
        let at = ctx.cursor / ctx.camera.viewport.max(DVec2::ONE);
        self.ask(
            NoteRequest::Screen {
                at: at.clamp(DVec2::ZERO, DVec2::ONE).to_array(),
            },
            out,
        );
    }

    /// Asks for the text of the note `request` places.
    fn ask(&mut self, request: NoteRequest, out: &mut ToolOutput) {
        self.step = Step::Text(request);
        out.note = Some(request);
    }

    /// Adds the note `request` places with `text` (blank: a leader note's automatic text).
    fn finish(&mut self, request: NoteRequest, text: String, out: &mut ToolOutput) {
        let written = (!text.trim().is_empty()).then_some(text);
        match (request, written) {
            (
                NoteRequest::Leader {
                    anchor, position, ..
                },
                text,
            ) => out.commands.push(Command::AddNote {
                anchor,
                position: position.to_array(),
                text,
            }),
            (NoteRequest::Screen { at }, Some(text)) => {
                out.commands.push(Command::AddScreenNote { at, text })
            }
            (NoteRequest::Screen { .. }, None) => {
                out.message = Some("A note fixed to the view needs text".into());
                return;
            }
            // The Select tool asks for these.
            (NoteRequest::Edit(_), _) => return,
        }
        self.step = Step::Idle;
    }
}

impl Tool for TextTool {
    fn name(&self) -> &'static str {
        "Text"
    }

    fn prompt(&self) -> &'static str {
        match self.step {
            Step::Idle => "Click a point for a note pointing at it, or empty space",
            Step::Leader(_) => "Click where the text goes",
            Step::Text(NoteRequest::Screen { .. }) => "Type the note's text",
            Step::Text(_) => "Type the note's text (none: the point's coordinates)",
        }
    }

    fn measurement(&self, _units: &session::Units) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    /// The point the note points at, then where its text goes.
    fn phase(&self) -> Phase {
        let points = match self.step {
            Step::Idle => return Phase::Idle,
            Step::Leader(a) => vec![a.point],
            Step::Text(NoteRequest::Leader {
                point, position, ..
            }) => vec![point, position],
            Step::Text(_) => Vec::new(),
        };
        Phase::Active { points }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => {
                self.hover = Some(hit.point);
                match self.step {
                    // A click while the text is asked for drops that note.
                    Step::Idle | Step::Text(_) => self.start(&hit, ctx, &mut out),
                    Step::Leader(a) if a.point.distance(hit.point) <= LENGTH_TOL => {
                        out.message = Some("Click away from the point".into());
                    }
                    Step::Leader(a) => self.ask(
                        NoteRequest::Leader {
                            anchor: a.input,
                            point: a.point,
                            position: hit.point,
                        },
                        &mut out,
                    ),
                }
            }
            ToolEvent::Value(text) => match self.step {
                Step::Text(request) => self.finish(request, text, &mut out),
                Step::Idle => out.message = Some("Click where the note goes first".into()),
                Step::Leader(_) => out.message = Some("Click where the text goes first".into()),
            },
            ToolEvent::Cancel => self.on_external_change(),
            ToolEvent::Release(_)
            | ToolEvent::DoubleClick(_)
            | ToolEvent::TripleClick(_)
            | ToolEvent::ToggleCopy => {}
        }
        match (self.step, self.hover) {
            (Step::Leader(a), Some(hover)) => out.preview.push([a.point, hover]),
            (
                Step::Text(NoteRequest::Leader {
                    point, position, ..
                }),
                _,
            ) => out.preview.push([point, position]),
            _ => {}
        }
        out
    }

    fn on_external_change(&mut self) {
        self.step = Step::Idle;
    }

    fn awaits_text(&self) -> bool {
        matches!(self.step, Step::Text(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimension::tests::Rig;
    use session::AnchorInput;

    fn rig() -> Rig<TextTool> {
        Rig::new(TextTool::new())
    }

    fn v(x: f64, y: f64) -> DVec3 {
        DVec3::new(x, y, 0.0)
    }

    #[test]
    fn a_point_then_a_place_asks_for_the_text_of_a_leader_note() {
        let mut r = rig();
        r.click(v(0.0, 0.0));
        let out = r.hover(v(1.0, 1.0));
        let [[from, to]] = out.preview[..] else {
            panic!("{:?}", out.preview);
        };
        assert!(from == v(0.0, 0.0) && to.abs_diff_eq(v(1.0, 1.0), 1e-9));
        let out = r.click(v(1.0, 1.0));
        let anchor = r.vertex(v(0.0, 0.0));
        let Some(NoteRequest::Leader {
            anchor: a,
            point,
            position,
        }) = out.note
        else {
            panic!("{:?}", out.note);
        };
        assert_eq!((a, point), (anchor, v(0.0, 0.0)));
        assert!(position.abs_diff_eq(v(1.0, 1.0), 1e-9));
        assert!(r.tool.awaits_text());
        // No text: the automatic text.
        let out = r.send(ToolEvent::Value(" ".into()));
        assert_eq!(
            out.commands,
            vec![Command::AddNote {
                anchor,
                position: position.to_array(),
                text: None
            }]
        );
        assert!(!r.tool.awaits_text());
        assert_eq!(r.tool.phase(), Phase::Idle);
        // A point on an edge is fixed; written text is kept.
        r.click(v(2.5, 0.0));
        r.click(v(2.5, 1.0));
        let out = r.send(ToolEvent::Value("Wall".into()));
        let [
            Command::AddNote {
                anchor: AnchorInput::Point(p),
                ref text,
                ..
            },
        ] = out.commands[..]
        else {
            panic!("{:?}", out.commands);
        };
        assert!(DVec3::from(p).abs_diff_eq(v(2.5, 0.0), 1e-9));
        assert_eq!(text.as_deref(), Some("Wall"));
    }

    #[test]
    fn empty_space_asks_for_the_text_of_a_note_fixed_to_the_view() {
        let mut r = rig();
        r.cursor = DVec2::new(200.0, 30.0);
        let hit = Inference {
            point: DVec3::new(0.0, 50.0, 30.0),
            kind: SnapKind::Free,
            face: None,
        };
        let out = r.send(ToolEvent::Click(hit));
        assert_eq!(out.note, Some(NoteRequest::Screen { at: [0.25, 0.05] }));
        assert_eq!(r.tool.phase(), Phase::Active { points: vec![] });
        let out = r.send(ToolEvent::Value(String::new()));
        assert!(out.commands.is_empty() && out.message.unwrap().contains("needs text"));
        assert!(r.tool.awaits_text(), "still asking");
        let out = r.send(ToolEvent::Value("North\nside".into()));
        assert_eq!(
            out.commands,
            vec![Command::AddScreenNote {
                at: [0.25, 0.05],
                text: "North\nside".into()
            }]
        );
    }

    #[test]
    fn escape_drops_the_note_at_any_step() {
        let mut r = rig();
        r.click(v(0.0, 0.0));
        r.send(ToolEvent::Cancel);
        assert_eq!(r.tool.phase(), Phase::Idle);
        r.click(v(0.0, 0.0));
        r.click(v(1.0, 1.0));
        let out = r.send(ToolEvent::Cancel);
        assert!(out.commands.is_empty() && out.preview.is_empty());
        assert!(!r.tool.awaits_text());
        let out = r.send(ToolEvent::Value("late".into()));
        assert!(out.commands.is_empty());
        // The text goes away from the point.
        r.click(v(0.0, 0.0));
        let out = r.click(v(0.0, 0.0));
        assert!(out.note.is_none() && out.message.unwrap().contains("away"));
    }
}
