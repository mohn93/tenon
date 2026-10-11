//! The Erase tool: click an edge, face or group to erase it. What is under the cursor is
//! outlined.

use glam::DVec3;
use kernel::EntityId;
use session::Command;

use crate::select::{box_outline, face_outline, pick};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};

#[derive(Clone, Debug, Default)]
pub struct EraseTool {
    /// Outline of the entity under the cursor.
    outline: Vec<[DVec3; 2]>,
}

impl EraseTool {
    pub fn new() -> Self {
        Self::default()
    }
}

fn outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
    let m = ctx.model;
    match id {
        EntityId::Edge(e) => m
            .edge(e)
            .and_then(|e| Some([m.position(e.v[0])?, m.position(e.v[1])?]))
            .into_iter()
            .collect(),
        EntityId::Face(f) => m.face(f).map(|f| face_outline(m, f)).unwrap_or_default(),
        EntityId::Instance(_) => box_outline(ctx, id),
        EntityId::Vertex(_) => Vec::new(),
    }
}

impl Tool for EraseTool {
    fn name(&self) -> &'static str {
        "Erase"
    }

    fn prompt(&self) -> &'static str {
        "Click an edge, face or group to erase it"
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(_) => {
                self.outline = pick(ctx).map(|id| outline(ctx, id)).unwrap_or_default()
            }
            ToolEvent::Click(_) => {
                if let Some(id) = pick(ctx) {
                    out.commands.push(Command::Erase { ids: vec![id] });
                }
                self.outline.clear();
            }
            ToolEvent::Value(_) => {
                out.message = Some("The Erase tool takes no typed values".into());
            }
            ToolEvent::Release(_)
            | ToolEvent::DoubleClick(_)
            | ToolEvent::Cancel
            | ToolEvent::ToggleCopy => {}
        }
        out.preview = self.outline.clone();
        out
    }

    fn on_external_change(&mut self) {
        self.outline.clear();
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
        };
        tool.handle(event, &ctx)
    }

    #[test]
    fn hovering_outlines_and_clicking_erases() {
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
        assert_eq!(
            send(&mut t, &m, &cam, edge, ToolEvent::Move(hit()))
                .preview
                .len(),
            1
        );
        let inside = DVec3::new(1.0, 1.0, 0.0);
        assert_eq!(
            send(&mut t, &m, &cam, inside, ToolEvent::Move(hit()))
                .preview
                .len(),
            4
        );
        let out = send(&mut t, &m, &cam, inside, ToolEvent::Click(hit()));
        assert_eq!(
            out.commands,
            vec![Command::Erase {
                ids: vec!["f8".parse().unwrap()]
            }]
        );
        let off = DVec3::new(-3.0, 4.0, 0.0);
        assert!(
            send(&mut t, &m, &cam, off, ToolEvent::Click(hit()))
                .commands
                .is_empty()
        );
    }
}
