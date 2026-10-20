//! The Paste tool: the clip's edges follow the cursor, its base point on the inferred point
//! under it, and a click pastes there. Escape ends it (the editor then goes back to the tool
//! used before). Typed distances are not taken.

use glam::DVec3;
use session::{ClipOutline, Command};

use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};

#[derive(Clone, Debug)]
pub struct PasteTool {
    /// The clip text pasted on a click.
    clip: String,
    /// The clip's edges, relative to its base point.
    outline: Vec<[DVec3; 2]>,
    hover: Option<DVec3>,
}

impl PasteTool {
    /// A Paste tool for `clip` (clip text) whose outline is `outline` (`session::clip::outline`
    /// of the same text).
    pub fn new(clip: String, outline: &ClipOutline) -> Self {
        let base = outline.base;
        Self {
            clip,
            outline: outline
                .segments
                .iter()
                .map(|[a, b]| [*a - base, *b - base])
                .collect(),
            hover: None,
        }
    }

    /// The outline with its base point at `at`.
    fn placed(&self, at: DVec3) -> Vec<[DVec3; 2]> {
        self.outline
            .iter()
            .map(|[a, b]| [*a + at, *b + at])
            .collect()
    }
}

impl Tool for PasteTool {
    fn name(&self) -> &'static str {
        "Paste"
    }

    fn prompt(&self) -> &'static str {
        "Click where to paste (Esc cancels)"
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    /// Placing the clip is an operation in progress from the start, with no points given.
    fn phase(&self) -> Phase {
        Phase::Active { points: Vec::new() }
    }

    fn handle(&mut self, event: ToolEvent, _ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => {
                self.hover = Some(hit.point);
                out.commands.push(Command::Paste {
                    clip: self.clip.clone(),
                    at: Some(hit.point.to_array()),
                });
            }
            ToolEvent::Value(_) => {
                out.message = Some("Click where to paste: typed distances are not taken".into());
            }
            ToolEvent::Cancel
            | ToolEvent::ToggleCopy
            | ToolEvent::Release(_)
            | ToolEvent::DoubleClick(_)
            | ToolEvent::TripleClick(_) => {}
        }
        if out.commands.is_empty()
            && let Some(at) = self.hover
        {
            out.preview = self.placed(at);
        }
        out
    }

    /// The clip names no entity of the model: it stays ready to paste.
    fn on_external_change(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;
    use std::collections::BTreeSet;

    fn at(p: DVec3) -> Inference {
        Inference {
            point: p,
            kind: SnapKind::Ground,
            face: None,
        }
    }

    fn send(tool: &mut PasteTool, event: ToolEvent) -> ToolOutput {
        let model = Model::new();
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let selection = BTreeSet::new();
        let ctx = ToolContext {
            model: &model,
            scenery: &Scenery::single(&model),
            camera: &camera,
            cursor: DVec2::ZERO,
            pick_radius: 10.0,
            drag_threshold: 4.0,
            quick_press: false,
            shift: false,
            selection: &selection,
            material: None,
            sample: false,
            labels: &[],
        };
        tool.handle(event, &ctx)
    }

    fn tool() -> PasteTool {
        let outline = ClipOutline {
            base: DVec3::new(1.0, 1.0, 0.0),
            segments: vec![[DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 1.0, 0.0)]],
        };
        PasteTool::new("clip".into(), &outline)
    }

    #[test]
    fn the_outline_follows_the_cursor_by_its_base_point() {
        let mut t = tool();
        assert_eq!(t.phase(), Phase::Active { points: vec![] });
        let out = send(&mut t, ToolEvent::Move(at(DVec3::new(5.0, 0.0, 2.0))));
        assert_eq!(
            out.preview,
            vec![[DVec3::new(5.0, 0.0, 2.0), DVec3::new(7.0, 0.0, 2.0)]]
        );
        assert!(out.commands.is_empty());
    }

    #[test]
    fn a_click_pastes_there_and_values_are_refused() {
        let mut t = tool();
        let typed = send(&mut t, ToolEvent::Value("3".into()));
        assert!(typed.commands.is_empty() && typed.message.is_some());
        let out = send(&mut t, ToolEvent::Click(at(DVec3::new(2.0, 3.0, 0.0))));
        assert_eq!(
            out.commands,
            vec![Command::Paste {
                clip: "clip".into(),
                at: Some([2.0, 3.0, 0.0]),
            }]
        );
        assert!(out.preview.is_empty());
    }
}
