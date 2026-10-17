//! The Erase tool: click an edge, face, guide or group to erase it. What is under the cursor is
//! reported as hovered.

use glam::DVec3;
use kernel::EntityId;
use session::Command;

use crate::select::pick;
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};

#[derive(Clone, Debug, Default)]
pub struct EraseTool {
    /// What a click would erase.
    hovered: Option<EntityId>,
}

impl EraseTool {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Tool for EraseTool {
    fn name(&self) -> &'static str {
        "Erase"
    }

    fn prompt(&self) -> &'static str {
        "Click an edge, face, guide or group to erase it"
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    /// Each click acts at once.
    fn phase(&self) -> Phase {
        Phase::Idle
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(_) => self.hovered = pick(ctx),
            ToolEvent::Click(_) => {
                if let Some(id) = pick(ctx) {
                    out.commands.push(Command::Erase { ids: vec![id] });
                }
                self.hovered = None;
            }
            ToolEvent::Value(_) => {
                out.message = Some("The Erase tool takes no typed values".into());
            }
            ToolEvent::Cancel => self.hovered = None,
            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::ToggleCopy => {}
        }
        out
    }

    fn on_external_change(&mut self) {
        self.hovered = None;
    }

    fn hovered(&self) -> Option<EntityId> {
        self.hovered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;

    fn hit() -> Inference {
        Inference {
            point: DVec3::ZERO,
            kind: SnapKind::Free,
            face: None,
        }
    }

    fn send(
        tool: &mut EraseTool,
        model: &Model,
        camera: &Camera,
        at: DVec3,
        event: ToolEvent,
    ) -> ToolOutput {
        let ctx = ToolContext {
            model,
            scenery: &crate::scenery::Scenery::single(model),
            camera,
            cursor: camera.project(at).unwrap(),
            pick_radius: 10.0,
            drag_threshold: 4.0,
            shift: false,
            selection: &Default::default(),
            material: None,
            sample: false,
        };
        tool.handle(event, &ctx)
    }

    #[test]
    fn hovering_reports_the_target_and_clicking_erases() {
        let mut m = Model::new();
        let pts = [
            DVec3::ZERO,
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(2.0, 2.0, 0.0),
            DVec3::new(0.0, 2.0, 0.0),
        ];
        m.add_polygon(&pts).unwrap();
        let cam = Camera::new(DVec2::new(800.0, 600.0));
        let mut t = EraseTool::new();
        let edge = DVec3::new(1.0, 0.0, 0.0);
        let out = send(&mut t, &m, &cam, edge, ToolEvent::Move(hit()));
        assert_eq!(t.hovered(), Some("e2".parse().unwrap()));
        assert!(out.preview.is_empty());
        let inside = DVec3::new(1.0, 1.0, 0.0);
        let out = send(&mut t, &m, &cam, inside, ToolEvent::Move(hit()));
        assert_eq!(t.hovered(), Some("f8".parse().unwrap()));
        assert!(out.preview.is_empty());
        let off = DVec3::new(-3.0, 4.0, 0.0);
        let out = send(&mut t, &m, &cam, off, ToolEvent::Move(hit()));
        assert_eq!(t.hovered(), None);
        assert!(out.preview.is_empty());
        send(&mut t, &m, &cam, inside, ToolEvent::Move(hit()));
        let out = send(&mut t, &m, &cam, inside, ToolEvent::Click(hit()));
        assert!(out.preview.is_empty());
        assert_eq!(t.hovered(), None, "a click clears the hover");
        assert_eq!(
            out.commands,
            vec![Command::Erase {
                ids: vec!["f8".parse().unwrap()]
            }]
        );
        assert!(
            send(&mut t, &m, &cam, off, ToolEvent::Click(hit()))
                .commands
                .is_empty()
        );
    }
}
