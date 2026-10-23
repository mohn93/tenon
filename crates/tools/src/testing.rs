//! A rig for the unit tests of the tools that read the camera, the drawing axes and Shift.

use std::collections::BTreeSet;

use glam::{DAffine3, DVec2, DVec3};
use kernel::{EntityId, FaceId, Model};
use session::{Camera, Command};

use crate::inference::{Inference, SnapKind};
use crate::scenery::Scenery;
use crate::tool::{Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};

pub(crate) struct Rig {
    pub tool: Box<dyn Tool>,
    pub model: Model,
    pub camera: Camera,
    pub axes: DAffine3,
    pub shift: bool,
    pub cursor: DVec2,
    selection: BTreeSet<EntityId>,
}

impl Rig {
    pub fn new(kind: ToolKind, model: Model) -> Self {
        Rig {
            tool: kind.make(),
            model,
            camera: Camera::new(DVec2::new(800.0, 600.0)),
            axes: DAffine3::IDENTITY,
            shift: false,
            cursor: DVec2::ZERO,
            selection: BTreeSet::new(),
        }
    }

    pub fn send(&mut self, event: ToolEvent) -> ToolOutput {
        let scenery = Scenery::single(&self.model).with_axes(self.axes);
        let ctx = ToolContext {
            model: &self.model,
            scenery: &scenery,
            camera: &self.camera,
            cursor: self.cursor,
            pick_radius: 10.0,
            drag_threshold: 4.0,
            quick_press: false,
            shift: self.shift,
            selection: &self.selection,
            material: None,
            sample: false,
            labels: &[],
        };
        self.tool.handle(event, &ctx)
    }

    /// Puts the cursor over `p` and returns the inference there, as `kind` on `face`.
    pub fn at(&mut self, p: DVec3, kind: SnapKind, face: Option<FaceId>) -> Inference {
        self.cursor = self.camera.project(p).unwrap();
        Inference {
            point: p,
            kind,
            face,
        }
    }

    /// The inference at `p` for a point on the ground or in free space.
    pub fn free(&mut self, p: DVec3) -> Inference {
        let kind = if p.z.abs() < 1e-9 {
            SnapKind::Ground
        } else {
            SnapKind::Free
        };
        self.at(p, kind, None)
    }

    pub fn click(&mut self, p: DVec3) -> ToolOutput {
        let hit = self.free(p);
        self.send(ToolEvent::Click(hit))
    }

    pub fn mv(&mut self, p: DVec3) -> ToolOutput {
        let hit = self.free(p);
        self.send(ToolEvent::Move(hit))
    }

    pub fn release(&mut self, p: DVec3) -> ToolOutput {
        let hit = self.free(p);
        self.send(ToolEvent::Release(hit))
    }

    pub fn value(&mut self, text: &str) -> ToolOutput {
        self.send(ToolEvent::Value(text.into()))
    }
}

/// The only command of `out`.
pub(crate) fn only(out: &ToolOutput) -> &Command {
    assert_eq!(out.commands.len(), 1, "{:?}", out.commands);
    &out.commands[0]
}
