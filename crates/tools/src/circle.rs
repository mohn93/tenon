//! The Circle tool: click the center, then a point on the rim (or type the radius); a drag from the
//! center to the rim works as the two clicks. The circle lies on the face under the center click,
//! or on a horizontal plane. Typing "24s" sets the number of sides, before or during a circle; the
//! count is kept for the session, also when another tool is selected in between.

use glam::DVec3;
use session::{Command, DEFAULT_CIRCLE_SEGMENTS, circle_points};

use crate::rectangle::{on_plane, plane_normal};
use crate::tool::{DragStart, Phase, Remembered, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, meant_as_count, parse_length, parse_segments};

/// Shown for a side count that is not a whole number in range.
const BAD_SIDES: &str = "Sides must be a whole number from 3 to 360, like 24s";

#[derive(Clone, Debug)]
pub struct CircleTool {
    /// The center and the normal of the plane the circle lies in.
    center: Option<(DVec3, DVec3)>,
    /// The point on the rim under the cursor, on that plane.
    hover: Option<DVec3>,
    segments: u32,
    drag_start: DragStart,
}

impl Default for CircleTool {
    fn default() -> Self {
        CircleTool {
            center: None,
            hover: None,
            segments: DEFAULT_CIRCLE_SEGMENTS,
            drag_start: DragStart::default(),
        }
    }
}

impl CircleTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// A Circle tool drawing circles of `sides` sides (a count `circle_points` accepts).
    pub fn with_sides(sides: u32) -> Self {
        CircleTool {
            segments: sides,
            ..Self::default()
        }
    }

    /// The radius from the center to the cursor.
    fn radius(&self) -> Option<f64> {
        let ((center, _), hover) = (self.center?, self.hover?);
        Some(center.distance(hover))
    }

    /// The command for the circle with `radius` around the center, if it can be drawn.
    fn finish(&mut self, radius: f64, out: &mut ToolOutput) {
        let Some((center, normal)) = self.center else {
            return;
        };
        match circle_points(center, radius, Some(normal), self.segments) {
            Ok(_) => {
                out.commands.push(Command::AddCircle {
                    center: center.to_array(),
                    radius,
                    normal: Some(normal.to_array()),
                    segments: Some(self.segments),
                });
                self.center = None;
                self.hover = None;
            }
            Err(e) => out.message = Some(e.message),
        }
    }
}

impl Tool for CircleTool {
    fn name(&self) -> &'static str {
        "Circle"
    }

    fn prompt(&self) -> &'static str {
        match self.center {
            None => "Click the center; type 24s for 24 sides",
            Some(_) => "Click a point on the circle, or type the radius",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("Radius", self.radius().map(format_length))
    }

    fn anchor(&self) -> Option<DVec3> {
        self.center.map(|(c, _)| c)
    }

    fn phase(&self) -> Phase {
        match self.center {
            Some((center, _)) => Phase::Active {
                points: vec![center],
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => {
                if let Some((center, normal)) = self.center {
                    self.hover = on_plane(center, normal, &hit, ctx);
                }
            }
            ToolEvent::Click(hit) => match self.center {
                None => {
                    self.center = Some((hit.point, plane_normal(&hit, ctx)));
                    self.drag_start.arm(ctx.cursor);
                }
                Some((center, normal)) => {
                    if let Some(p) = on_plane(center, normal, &hit, ctx) {
                        self.finish(center.distance(p), &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => {
                if let Some(n) = parse_segments(&text) {
                    match circle_points(DVec3::ZERO, 1.0, None, n) {
                        Ok(_) => {
                            self.segments = n;
                            out.message = Some(format!("Circles get {n} sides"));
                            out.info = true;
                        }
                        Err(_) => out.message = Some(BAD_SIDES.into()),
                    }
                } else if meant_as_count(&text) {
                    out.message = Some(BAD_SIDES.into());
                } else {
                    match (self.center, parse_length(&text)) {
                        (None, _) => {
                            out.message = Some("Click the center first, or type 24s".into());
                        }
                        (Some(_), Some(r)) => self.finish(r, &mut out),
                        (Some(_), None) => {
                            out.message = Some(format!("Not a radius or a side count: {text}"));
                        }
                    }
                }
            }
            ToolEvent::Release(hit) => {
                if self.drag_start.released(ctx) {
                    return self.handle(ToolEvent::Click(hit), ctx);
                }
            }
            ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) | ToolEvent::ToggleCopy => {}
            ToolEvent::Cancel => self.on_external_change(),
        }
        if let (Some((center, normal)), Some(r)) = (self.center, self.radius())
            && let Ok(points) = circle_points(center, r, Some(normal), self.segments)
        {
            let n = points.len();
            out.preview = (0..n).map(|i| [points[i], points[(i + 1) % n]]).collect();
            out.preview.push([center, self.hover.unwrap_or(center)]);
        }
        out
    }

    fn on_external_change(&mut self) {
        self.center = None;
        self.hover = None;
        self.drag_start = DragStart::default();
    }

    fn remember(&self, r: &mut Remembered) {
        r.circle_sides = self.segments;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::Model;
    use session::{Camera, Document};

    fn ground(x: f64, y: f64) -> Inference {
        Inference {
            point: DVec3::new(x, y, 0.0),
            kind: SnapKind::Ground,
            face: None,
        }
    }

    fn send(
        tool: &mut CircleTool,
        scenery: &Scenery,
        model: &Model,
        event: ToolEvent,
    ) -> ToolOutput {
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let ctx = ToolContext {
            model,
            scenery,
            camera: &camera,
            cursor: DVec2::ZERO,
            pick_radius: 10.0,
            drag_threshold: 4.0,
            quick_press: false,
            shift: false,
            selection: &Default::default(),
            material: None,
            sample: false,
            labels: &[],
        };
        tool.handle(event, &ctx)
    }

    fn on_ground(tool: &mut CircleTool, event: ToolEvent) -> ToolOutput {
        let model = Model::new();
        send(tool, &Scenery::single(&model), &model, event)
    }

    fn circle(center: [f64; 3], radius: f64, segments: u32) -> Command {
        Command::AddCircle {
            center,
            radius,
            normal: Some([0.0, 0.0, 1.0]),
            segments: Some(segments),
        }
    }

    #[test]
    fn a_circle_in_an_open_group_is_in_the_groups_coordinates() {
        let mut d = Document::new(DVec2::new(800.0, 600.0));
        let made = d
            .execute(Command::AddRectangle {
                corner: [0.0, 0.0, 0.0],
                opposite: [1.0, 1.0, 0.0],
                normal: None,
            })
            .unwrap();
        d.execute(Command::MakeGroup {
            ids: made
                .changes
                .created
                .into_iter()
                .filter(|id| !matches!(id, kernel::EntityId::Vertex(_)))
                .collect(),
        })
        .unwrap();
        let group = *d.selection().iter().next().unwrap();
        d.execute(Command::Move {
            ids: vec![group],
            offset: [10.0, 0.0, 0.0],
        })
        .unwrap();
        d.execute(Command::OpenInstance { id: group }).unwrap();
        assert!(!d.context().is_empty());

        let mut tool = CircleTool::new();
        let scenery = Scenery::of(d.scene(), d.context()).unwrap();
        // Hits arrive in the open group's own coordinates, not the root's.
        send(
            &mut tool,
            &scenery,
            d.model(),
            ToolEvent::Click(ground(2.0, 3.0)),
        );
        let out = send(
            &mut tool,
            &scenery,
            d.model(),
            ToolEvent::Click(ground(4.0, 3.0)),
        );
        assert_eq!(out.commands, vec![circle([2.0, 3.0, 0.0], 2.0, 24)]);
        d.execute(out.commands[0].clone()).unwrap();
    }

    #[test]
    fn a_typed_radius_right_after_the_center_click_draws_the_circle() {
        let mut tool = CircleTool::new();
        on_ground(&mut tool, ToolEvent::Click(ground(1.0, 1.0)));
        assert_eq!(tool.hover, None);
        let out = on_ground(&mut tool, ToolEvent::Value("2".into()));
        assert_eq!(out.commands, vec![circle([1.0, 1.0, 0.0], 2.0, 24)]);
        assert_eq!(tool.anchor(), None);
    }

    #[test]
    fn a_click_on_the_center_is_refused_and_keeps_the_center() {
        let mut tool = CircleTool::new();
        on_ground(&mut tool, ToolEvent::Click(ground(1.0, 1.0)));
        let out = on_ground(&mut tool, ToolEvent::Click(ground(1.0, 1.0)));
        assert!(out.commands.is_empty());
        let message = out.message.expect("the refusal is explained");
        assert!(message.contains("radius"), "{message}");
        assert_eq!(tool.anchor(), Some(DVec3::new(1.0, 1.0, 0.0)));
    }

    #[test]
    fn bad_side_counts_say_what_a_count_is() {
        let mut tool = CircleTool::new();
        let out = on_ground(&mut tool, ToolEvent::Value("400s".into()));
        assert_eq!(
            out.message.as_deref(),
            Some("Sides must be a whole number from 3 to 360, like 24s")
        );
        on_ground(&mut tool, ToolEvent::Click(ground(1.0, 1.0)));
        for text in ["0s", "2.5s", "-3s", "s"] {
            let out = on_ground(&mut tool, ToolEvent::Value(text.into()));
            assert_eq!(
                out.message.as_deref(),
                Some("Sides must be a whole number from 3 to 360, like 24s"),
                "{text}"
            );
            assert!(out.commands.is_empty(), "{text}");
        }
        // The count did not change and the circle goes on.
        let out = on_ground(&mut tool, ToolEvent::Value("2".into()));
        assert_eq!(out.commands, vec![circle([1.0, 1.0, 0.0], 2.0, 24)]);
    }

    #[test]
    fn a_typed_side_count_applies_to_the_next_circle() {
        let mut tool = CircleTool::new();
        let out = on_ground(&mut tool, ToolEvent::Value("8s".into()));
        assert_eq!(out.message.as_deref(), Some("Circles get 8 sides"));
        on_ground(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        let out = on_ground(&mut tool, ToolEvent::Click(ground(1.0, 0.0)));
        assert_eq!(out.commands, vec![circle([0.0; 3], 1.0, 8)]);
    }

    #[test]
    fn escape_cancels_the_circle_but_keeps_the_side_count() {
        let mut tool = CircleTool::new();
        on_ground(&mut tool, ToolEvent::Value("8s".into()));
        on_ground(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        on_ground(&mut tool, ToolEvent::Move(ground(1.0, 0.0)));
        let out = on_ground(&mut tool, ToolEvent::Cancel);
        assert!(out.commands.is_empty() && out.preview.is_empty());
        assert_eq!(tool.anchor(), None);
        on_ground(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        let out = on_ground(&mut tool, ToolEvent::Click(ground(2.0, 0.0)));
        assert_eq!(out.commands, vec![circle([0.0; 3], 2.0, 8)]);
    }
}
