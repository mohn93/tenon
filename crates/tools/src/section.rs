//! The Section Plane tool: the preview square follows the face under the cursor (its normal
//! turned to face the camera), or lies on the ground where there is none; an arrow key locks
//! the normal to the red (Right), green (Left) or blue (Up) axis; a click places the plane and
//! makes it the open context's active plane.

use glam::DVec3;
use kernel::SectionPlane;
use session::{Command, SectionShape};

use crate::inference::{Axis, Inference};
use crate::rectangle::plane_normal;
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};

/// The preview square's side as a fraction of its distance from the eye.
const PREVIEW_SIDE: f64 = 0.2;

#[derive(Clone, Debug, Default)]
pub struct SectionPlaneTool {
    /// Where the plane would go: its point and the unit normal toward the camera.
    hover: Option<(DVec3, DVec3)>,
    /// The axis the normal is locked to, until unlocked or cancelled.
    lock: Option<Axis>,
}

impl SectionPlaneTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The point and normal a click at `hit` places: the face under the cursor's normal or the
    /// ground's (or the locked axis), turned toward the camera.
    fn placement(&self, hit: &Inference, ctx: &ToolContext) -> (DVec3, DVec3) {
        let normal = self.lock.map_or_else(
            || plane_normal(hit, ctx),
            |a| a.dir_in(ctx.scenery.axes().matrix3),
        );
        let toward = ctx.camera.sight_origin(hit.point) - hit.point;
        let normal = if normal.dot(toward) < 0.0 {
            -normal
        } else {
            normal
        };
        (hit.point, normal)
    }
}

impl Tool for SectionPlaneTool {
    fn name(&self) -> &'static str {
        "Section Plane"
    }

    fn prompt(&self) -> &'static str {
        "Click a face or the ground to place a section plane; arrows lock the normal to an axis"
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("", None)
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
            ToolEvent::Move(hit) => self.hover = Some(self.placement(&hit, ctx)),
            ToolEvent::Click(hit) => {
                let (point, normal) = self.placement(&hit, ctx);
                self.hover = Some((point, normal));
                out.commands.push(Command::AddSectionPlane {
                    point: point.to_array(),
                    normal: normal.to_array(),
                    activate: Some(true),
                });
            }
            ToolEvent::Cancel => {
                self.hover = None;
                self.lock = None;
            }
            ToolEvent::Value(text) => {
                out.message = Some(format!("A section plane has no value to type: {text}"));
            }
            ToolEvent::Release(_)
            | ToolEvent::DoubleClick(_)
            | ToolEvent::TripleClick(_)
            | ToolEvent::ToggleCopy => {}
        }
        if let Some((point, normal)) = self.hover {
            let side = PREVIEW_SIDE * ctx.camera.eye_distance(point);
            let shape = SectionShape::new(&SectionPlane::new(point, normal), side);
            out.preview.extend(shape.segments());
            out.preview.push([point, point + normal * side * 0.5]);
        }
        out
    }

    fn on_external_change(&mut self) {
        self.hover = None;
    }

    fn lock_plane(&mut self, axis: Option<Axis>) -> bool {
        self.lock = axis;
        true
    }

    fn plane_lock(&self) -> Option<Axis> {
        self.lock
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;
    use kernel::{EntityId, Model};
    use session::Camera;
    use std::collections::BTreeSet;

    use crate::inference::SnapKind;
    use crate::scenery::Scenery;

    /// A 2 m square on the ground and a camera looking at it from above.
    struct Rig {
        tool: SectionPlaneTool,
        model: Model,
        camera: Camera,
        selection: BTreeSet<EntityId>,
        axes: glam::DAffine3,
    }

    impl Rig {
        fn new() -> Self {
            let mut model = Model::new();
            model
                .add_polygon(&[
                    DVec3::ZERO,
                    DVec3::new(2.0, 0.0, 0.0),
                    DVec3::new(2.0, 2.0, 0.0),
                    DVec3::new(0.0, 2.0, 0.0),
                ])
                .unwrap();
            Rig {
                tool: SectionPlaneTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                selection: BTreeSet::new(),
                axes: glam::DAffine3::IDENTITY,
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &Scenery::single(&self.model).with_axes(self.axes),
                camera: &self.camera,
                cursor: DVec2::ZERO,
                pick_radius: 10.0,
                drag_threshold: 4.0,
                scale_factor: 1.0,
                to_world: glam::DAffine3::IDENTITY,
                quick_press: false,
                shift: false,
                selection: &self.selection,
                material: None,
                sample: false,
                labels: &[],
            };
            self.tool.handle(event, &ctx)
        }

        fn at(&self, p: DVec3) -> Inference {
            let cursor = self.camera.project(p).unwrap();
            let face =
                crate::inference::face_under(&self.model, &self.camera, cursor).map(|(f, _)| f);
            Inference {
                point: p,
                kind: SnapKind::Ground,
                face,
            }
        }
    }

    fn placed(out: &ToolOutput) -> ([f64; 3], [f64; 3], Option<bool>) {
        match out.commands.as_slice() {
            [
                Command::AddSectionPlane {
                    point,
                    normal,
                    activate,
                },
            ] => (*point, *normal, *activate),
            other => panic!("one AddSectionPlane expected, got {other:?}"),
        }
    }

    #[test]
    fn a_click_on_a_face_places_an_active_plane_facing_the_camera() {
        let mut r = Rig::new();
        let p = DVec3::new(1.0, 1.0, 0.0);
        let hit = r.at(p);
        let (point, normal, activate) = placed(&r.send(ToolEvent::Click(hit)));
        assert_eq!(point, p.to_array());
        assert_eq!(activate, Some(true));
        // The default camera looks down on the ground from above.
        let toward = r.camera.eye - p;
        assert!(DVec3::from_array(normal).dot(toward) > 0.0);
        assert!((DVec3::from_array(normal).abs() - DVec3::Z).length() < 1e-9);
    }

    #[test]
    fn a_normal_pointing_away_from_the_eye_is_turned_around() {
        let mut r = Rig::new();
        // Look from below: the ground face's +Z normal points away from the eye.
        r.camera.eye = DVec3::new(1.0, 1.0, -8.0);
        r.camera.target = DVec3::new(1.0, 1.0, 0.0);
        r.camera.up = DVec3::Y;
        let hit = Inference {
            point: DVec3::new(1.0, 1.0, 0.0),
            kind: SnapKind::Ground,
            face: None,
        };
        let (_, normal, _) = placed(&r.send(ToolEvent::Click(hit)));
        assert!(normal[2] < -0.99);
    }

    #[test]
    fn an_axis_lock_sets_the_normal_and_cancel_clears_it() {
        let mut r = Rig::new();
        let hit = r.at(DVec3::new(1.0, 1.0, 0.0));
        assert!(r.tool.lock_plane(Some(Axis::X)));
        assert_eq!(r.tool.plane_lock(), Some(Axis::X));
        let (_, normal, _) = placed(&r.send(ToolEvent::Click(hit)));
        assert!((normal[0].abs() - 1.0).abs() < 1e-9 && normal[1].abs() < 1e-9);
        r.send(ToolEvent::Cancel);
        assert_eq!(r.tool.plane_lock(), None);
    }

    #[test]
    fn an_axis_lock_follows_the_drawing_axes() {
        let mut r = Rig::new();
        // Red along world y.
        r.axes = glam::DAffine3::from_mat3_translation(
            glam::DMat3::from_cols(DVec3::Y, -DVec3::X, DVec3::Z),
            DVec3::ZERO,
        );
        let hit = r.at(DVec3::new(1.0, 1.0, 0.0));
        r.tool.lock_plane(Some(Axis::X));
        let (_, normal, _) = placed(&r.send(ToolEvent::Click(hit)));
        assert!((normal[1].abs() - 1.0).abs() < 1e-9 && normal[0].abs() < 1e-9);
    }

    #[test]
    fn moving_previews_the_square_and_its_normal() {
        let mut r = Rig::new();
        let hit = r.at(DVec3::new(1.0, 1.0, 0.0));
        let out = r.send(ToolEvent::Move(hit));
        assert!(out.commands.is_empty());
        assert_eq!(out.preview.len(), 5);
        let out = r.send(ToolEvent::Cancel);
        assert!(out.preview.is_empty());
    }
}
