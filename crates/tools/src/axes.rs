//! The Axes tool: click the new origin, click a point on the red axis, click a point fixing the
//! green axis (the plane). The three clicks become one `set_axes`; a click that gives no
//! direction (on the origin, or in line with red) is ignored.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};

/// The preview axes' length as a fraction of their distance from the eye.
const PREVIEW_LENGTH: f64 = 0.2;

#[derive(Clone, Debug, Default)]
pub struct AxesTool {
    /// The origin, then the point on the red axis, as clicked.
    points: Vec<DVec3>,
    /// The point under the cursor.
    hover: Option<DVec3>,
}

/// The unit green direction a point `p` gives for the red direction `x`: the part of `p - origin`
/// square to `x`, or None where it is in line with it.
fn green_toward(origin: DVec3, x: DVec3, p: DVec3) -> Option<DVec3> {
    let v = p - origin;
    let y = v - x * v.dot(x);
    (y.length() > LENGTH_TOL).then(|| y.normalize())
}

impl AxesTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The unit red direction of the clicked origin and red point.
    fn red(&self) -> Option<DVec3> {
        match self.points[..] {
            [o, r, ..] => (r - o).try_normalize(),
            _ => None,
        }
    }

    fn preview(&self, ctx: &ToolContext) -> Vec<[DVec3; 2]> {
        let Some(&origin) = self.points.first() else {
            return Vec::new();
        };
        let len = PREVIEW_LENGTH * ctx.camera.eye_distance(origin);
        let line = |d: DVec3| [origin, origin + d * len];
        let Some(x) = self.red() else {
            // Only the origin: the red axis follows the cursor.
            return match self.hover.and_then(|h| (h - origin).try_normalize()) {
                Some(x) => vec![line(x)],
                None => Vec::new(),
            };
        };
        let mut out = vec![line(x)];
        if let Some(y) = self
            .hover
            .and_then(|h| green_toward(origin, x, h))
            .filter(|_| self.points.len() == 2)
        {
            out.push(line(y));
            out.push(line(x.cross(y)));
        }
        out
    }
}

impl Tool for AxesTool {
    fn name(&self) -> &'static str {
        "Axes"
    }

    fn prompt(&self) -> &'static str {
        match self.points.len() {
            0 => "Click the new axes origin",
            1 => "Click a point on the red axis",
            _ => "Click a point in the red-green plane, on the green side",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        self.points.last().copied()
    }

    fn phase(&self) -> Phase {
        if self.points.is_empty() {
            Phase::Idle
        } else {
            Phase::Active {
                points: self.points.clone(),
            }
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => {
                let p = hit.point;
                self.hover = Some(p);
                match self.points.len() {
                    0 => self.points.push(p),
                    1 => {
                        if p.distance(self.points[0]) <= LENGTH_TOL {
                            out.message = Some("Click a point away from the origin".into());
                        } else {
                            self.points.push(p);
                        }
                    }
                    _ => {
                        let origin = self.points[0];
                        let x = self
                            .red()
                            .expect("two points are further apart than LENGTH_TOL");
                        match green_toward(origin, x, p) {
                            None => {
                                out.message = Some(
                                    "Click a point off the red axis for the green axis".into(),
                                );
                            }
                            Some(y) => {
                                out.commands.push(Command::SetAxes {
                                    origin: origin.to_array(),
                                    x_axis: x.to_array(),
                                    y_axis: y.to_array(),
                                });
                                self.points.clear();
                            }
                        }
                    }
                }
            }
            ToolEvent::Cancel => {
                self.points.clear();
                self.hover = None;
            }
            ToolEvent::Value(text) => {
                out.message = Some(format!("The axes have no value to type: {text}"));
            }
            ToolEvent::Release(_)
            | ToolEvent::DoubleClick(_)
            | ToolEvent::TripleClick(_)
            | ToolEvent::ToggleCopy => {}
        }
        out.preview = self.preview(ctx);
        out
    }

    fn on_external_change(&mut self) {
        self.points.clear();
        self.hover = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::{EntityId, Model};
    use session::Camera;
    use std::collections::BTreeSet;

    struct Rig {
        tool: AxesTool,
        model: Model,
        camera: Camera,
        selection: BTreeSet<EntityId>,
    }

    impl Rig {
        fn new() -> Self {
            Rig {
                tool: AxesTool::new(),
                model: Model::new(),
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                selection: BTreeSet::new(),
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &Scenery::single(&self.model),
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

        fn click(&mut self, p: [f64; 3]) -> ToolOutput {
            self.send(ToolEvent::Click(hit(p)))
        }
    }

    fn hit(p: [f64; 3]) -> Inference {
        Inference {
            point: DVec3::from_array(p),
            kind: SnapKind::Ground,
            face: None,
        }
    }

    #[test]
    fn three_clicks_emit_set_axes() {
        let mut r = Rig::new();
        assert!(r.click([1.0, 1.0, 0.0]).commands.is_empty());
        assert!(r.click([1.0, 3.0, 0.0]).commands.is_empty());
        // Red runs along +y; a point toward -x is on the green side.
        let out = r.click([-4.0, 3.0, 0.0]);
        match out.commands.as_slice() {
            [
                Command::SetAxes {
                    origin,
                    x_axis,
                    y_axis,
                },
            ] => {
                assert_eq!(*origin, [1.0, 1.0, 0.0]);
                assert!((DVec3::from_array(*x_axis) - DVec3::Y).length() < 1e-9);
                assert!((DVec3::from_array(*y_axis) + DVec3::X).length() < 1e-9);
            }
            other => panic!("one SetAxes expected, got {other:?}"),
        }
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn a_click_on_the_origin_is_ignored_with_a_message() {
        let mut r = Rig::new();
        r.click([1.0, 1.0, 0.0]);
        let out = r.click([1.0, 1.0, 0.0]);
        assert!(out.commands.is_empty() && out.message.is_some());
        assert_eq!(r.tool.prompt(), "Click a point on the red axis");
    }

    #[test]
    fn a_collinear_third_click_is_ignored_with_a_message() {
        let mut r = Rig::new();
        r.click([0.0, 0.0, 0.0]);
        r.click([2.0, 0.0, 0.0]);
        let out = r.click([-5.0, 0.0, 0.0]);
        assert!(out.commands.is_empty() && out.message.is_some());
        assert!(matches!(r.tool.phase(), Phase::Active { points } if points.len() == 2));
    }

    #[test]
    fn previews_grow_with_the_clicks_and_cancel_clears_them() {
        let mut r = Rig::new();
        assert!(
            r.send(ToolEvent::Move(hit([1.0, 0.0, 0.0])))
                .preview
                .is_empty()
        );
        r.click([0.0, 0.0, 0.0]);
        assert_eq!(
            r.send(ToolEvent::Move(hit([1.0, 0.0, 0.0]))).preview.len(),
            1
        );
        r.click([2.0, 0.0, 0.0]);
        assert_eq!(
            r.send(ToolEvent::Move(hit([0.0, 3.0, 0.0]))).preview.len(),
            3
        );
        assert_eq!(r.tool.anchor(), Some(DVec3::new(2.0, 0.0, 0.0)));
        let out = r.send(ToolEvent::Cancel);
        assert!(out.preview.is_empty());
        assert_eq!(r.tool.phase(), Phase::Idle);
    }
}
