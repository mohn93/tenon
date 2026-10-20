//! The Polygon tool: click the center, then a corner (or, circumscribed, the middle of a side),
//! or type the radius; a drag from the center works as the two clicks. The cursor sets the
//! radius and the turn of the polygon. The polygon lies on the face under the center click, or
//! on a horizontal plane. Typing "8s" sets the number of sides, before or during a polygon; a
//! Ctrl tap switches inscribed and circumscribed. Both are kept for the session, also when
//! another tool is selected in between.

use glam::DVec3;
use session::{Command, DEFAULT_POLYGON_SIDES, polygon_points};

use crate::rectangle::{on_plane, plane_normal};
use crate::tool::{DragStart, Phase, Remembered, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, meant_as_count, parse_length, parse_segments};

/// Shown for a side count that is not a whole number in range.
const BAD_SIDES: &str = "Sides must be a whole number from 3 to 360, like 8s";

#[derive(Clone, Debug)]
pub struct PolygonTool {
    /// The center and the normal of the plane the polygon lies in.
    center: Option<(DVec3, DVec3)>,
    /// The point under the cursor, on that plane.
    hover: Option<DVec3>,
    sides: u32,
    /// Whether the radius reaches the middles of the sides rather than the corners.
    circumscribed: bool,
    drag_start: DragStart,
}

impl Default for PolygonTool {
    fn default() -> Self {
        PolygonTool {
            center: None,
            hover: None,
            sides: DEFAULT_POLYGON_SIDES,
            circumscribed: false,
            drag_start: DragStart::default(),
        }
    }
}

impl PolygonTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// A Polygon tool drawing polygons of `sides` sides (a count `polygon_points` accepts),
    /// circumscribed or inscribed.
    pub fn with_settings(sides: u32, circumscribed: bool) -> Self {
        PolygonTool {
            sides,
            circumscribed,
            ..Self::default()
        }
    }

    /// The radius from the center to the cursor.
    fn radius(&self) -> Option<f64> {
        let ((center, _), hover) = (self.center?, self.hover?);
        Some(center.distance(hover))
    }

    /// The way from the center to the cursor, if the cursor is off the center.
    fn direction(&self) -> Option<DVec3> {
        let ((center, _), hover) = (self.center?, self.hover?);
        let d = hover - center;
        (d.length() > 1e-9).then_some(d)
    }

    /// The command for the polygon with `radius` around the center, turned toward
    /// `direction` (the plane's u axis when None), if it can be drawn.
    fn finish(&mut self, radius: f64, direction: Option<DVec3>, out: &mut ToolOutput) {
        let Some((center, normal)) = self.center else {
            return;
        };
        match polygon_points(
            center,
            radius,
            self.sides,
            Some(normal),
            self.circumscribed,
            direction,
        ) {
            Ok(_) => {
                out.commands.push(Command::AddPolygon {
                    center: center.to_array(),
                    radius,
                    sides: Some(self.sides),
                    normal: Some(normal.to_array()),
                    circumscribed: self.circumscribed,
                    direction: direction.map(|d| d.to_array()),
                });
                self.center = None;
                self.hover = None;
            }
            Err(e) => out.message = Some(e.message),
        }
    }
}

impl Tool for PolygonTool {
    fn name(&self) -> &'static str {
        "Polygon"
    }

    fn prompt(&self) -> &'static str {
        match (self.center, self.circumscribed) {
            (None, _) => "Click the center; 8s sets the sides; Ctrl: inscribed or circumscribed",
            (Some(_), false) => "Click a corner, or type the radius",
            (Some(_), true) => "Click the middle of a side, or type the radius",
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
                        let d = p - center;
                        let direction = (d.length() > 1e-9).then_some(d);
                        self.finish(center.distance(p), direction, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => {
                if let Some(n) = parse_segments(&text) {
                    match polygon_points(DVec3::ZERO, 1.0, n, None, false, None) {
                        Ok(_) => {
                            self.sides = n;
                            out.message = Some(format!("Polygons get {n} sides"));
                            out.info = true;
                        }
                        Err(_) => out.message = Some(BAD_SIDES.into()),
                    }
                } else if meant_as_count(&text) {
                    out.message = Some(BAD_SIDES.into());
                } else {
                    match (self.center, parse_length(&text)) {
                        (None, _) => {
                            out.message = Some("Click the center first, or type 8s".into());
                        }
                        (Some(_), Some(r)) => self.finish(r, self.direction(), &mut out),
                        (Some(_), None) => {
                            out.message = Some(format!("Not a radius or a side count: {text}"));
                        }
                    }
                }
            }
            ToolEvent::ToggleCopy => {
                self.circumscribed = !self.circumscribed;
                out.message = Some(
                    if self.circumscribed {
                        "Polygons are drawn circumscribed"
                    } else {
                        "Polygons are drawn inscribed"
                    }
                    .into(),
                );
                out.info = true;
            }
            ToolEvent::Release(hit) => {
                if self.drag_start.released(ctx) {
                    return self.handle(ToolEvent::Click(hit), ctx);
                }
            }
            ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) => {}
            ToolEvent::Cancel => self.on_external_change(),
        }
        if let (Some((center, normal)), Some(r)) = (self.center, self.radius())
            && let Ok(points) = polygon_points(
                center,
                r,
                self.sides,
                Some(normal),
                self.circumscribed,
                self.direction(),
            )
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
        r.polygon_sides = self.sides;
        r.polygon_circumscribed = self.circumscribed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use crate::scenery::Scenery;
    use crate::tool::ToolKind;
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;

    fn ground(x: f64, y: f64) -> Inference {
        Inference {
            point: DVec3::new(x, y, 0.0),
            kind: SnapKind::Ground,
            face: None,
        }
    }

    fn on_ground(tool: &mut dyn Tool, event: ToolEvent) -> ToolOutput {
        let model = Model::new();
        let scenery = Scenery::single(&model);
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let ctx = ToolContext {
            model: &model,
            scenery: &scenery,
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

    fn polygon(
        center: [f64; 3],
        radius: f64,
        sides: u32,
        circumscribed: bool,
        direction: Option<[f64; 3]>,
    ) -> Command {
        Command::AddPolygon {
            center,
            radius,
            sides: Some(sides),
            normal: Some([0.0, 0.0, 1.0]),
            circumscribed,
            direction,
        }
    }

    #[test]
    fn two_clicks_draw_a_hexagon() {
        let mut tool = PolygonTool::new();
        on_ground(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        let out = on_ground(&mut tool, ToolEvent::Move(ground(2.0, 0.0)));
        assert_eq!(out.preview.len(), 7, "six sides and the radius");
        assert_eq!(tool.measurement(), ("Radius", Some("2 m".into())));
        let out = on_ground(&mut tool, ToolEvent::Click(ground(2.0, 0.0)));
        assert_eq!(
            out.commands,
            vec![polygon([0.0; 3], 2.0, 6, false, Some([2.0, 0.0, 0.0]))]
        );
        assert_eq!(tool.phase(), Phase::Idle);
    }

    #[test]
    fn a_click_on_the_center_is_refused_and_keeps_the_center() {
        let mut tool = PolygonTool::new();
        on_ground(&mut tool, ToolEvent::Click(ground(1.0, 1.0)));
        let out = on_ground(&mut tool, ToolEvent::Click(ground(1.0, 1.0)));
        assert!(out.commands.is_empty());
        let message = out.message.expect("the refusal is explained");
        assert!(message.contains("radius"), "{message}");
        assert_eq!(tool.anchor(), Some(DVec3::new(1.0, 1.0, 0.0)));
    }

    #[test]
    fn typed_sides_and_radius() {
        let mut tool = PolygonTool::new();
        let out = on_ground(&mut tool, ToolEvent::Value("8s".into()));
        assert_eq!(out.message.as_deref(), Some("Polygons get 8 sides"));
        assert!(out.info);
        on_ground(&mut tool, ToolEvent::Click(ground(1.0, 1.0)));
        for text in ["2s", "361s", "2.5s", "s"] {
            let out = on_ground(&mut tool, ToolEvent::Value(text.into()));
            assert_eq!(out.message.as_deref(), Some(BAD_SIDES), "{text}");
            assert!(!out.info && out.commands.is_empty(), "{text}");
        }
        // No hover yet: the plane's u axis.
        let out = on_ground(&mut tool, ToolEvent::Value("1.5".into()));
        assert_eq!(
            out.commands,
            vec![polygon([1.0, 1.0, 0.0], 1.5, 8, false, None)]
        );
        // With a hover, the typed radius goes toward it.
        on_ground(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        on_ground(&mut tool, ToolEvent::Move(ground(0.0, 3.0)));
        let out = on_ground(&mut tool, ToolEvent::Value("1".into()));
        assert_eq!(
            out.commands,
            vec![polygon([0.0; 3], 1.0, 8, false, Some([0.0, 3.0, 0.0]))]
        );
    }

    #[test]
    fn a_typed_radius_needs_a_center() {
        let mut tool = PolygonTool::new();
        let out = on_ground(&mut tool, ToolEvent::Value("2".into()));
        assert!(out.commands.is_empty());
        assert_eq!(
            out.message.as_deref(),
            Some("Click the center first, or type 8s")
        );
    }

    #[test]
    fn ctrl_switches_circumscribed() {
        let mut tool = PolygonTool::new();
        let out = on_ground(&mut tool, ToolEvent::ToggleCopy);
        assert_eq!(
            out.message.as_deref(),
            Some("Polygons are drawn circumscribed")
        );
        assert!(out.info);
        on_ground(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        let out = on_ground(&mut tool, ToolEvent::Click(ground(1.0, 0.0)));
        assert_eq!(
            out.commands,
            vec![polygon([0.0; 3], 1.0, 6, true, Some([1.0, 0.0, 0.0]))]
        );
        let out = on_ground(&mut tool, ToolEvent::ToggleCopy);
        assert_eq!(out.message.as_deref(), Some("Polygons are drawn inscribed"));
    }

    #[test]
    fn escape_cancels_the_polygon_but_keeps_the_settings() {
        let mut tool = PolygonTool::new();
        on_ground(&mut tool, ToolEvent::Value("5s".into()));
        on_ground(&mut tool, ToolEvent::ToggleCopy);
        on_ground(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        on_ground(&mut tool, ToolEvent::Move(ground(1.0, 0.0)));
        let out = on_ground(&mut tool, ToolEvent::Cancel);
        assert!(out.commands.is_empty() && out.preview.is_empty());
        assert_eq!(tool.anchor(), None);
        on_ground(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        let out = on_ground(&mut tool, ToolEvent::Click(ground(2.0, 0.0)));
        assert_eq!(
            out.commands,
            vec![polygon([0.0; 3], 2.0, 5, true, Some([2.0, 0.0, 0.0]))]
        );
    }

    #[test]
    fn remember_keeps_sides_and_mode() {
        let mut tool = PolygonTool::new();
        on_ground(&mut tool, ToolEvent::Value("7s".into()));
        on_ground(&mut tool, ToolEvent::ToggleCopy);
        let mut r = Remembered::default();
        assert_eq!((r.polygon_sides, r.polygon_circumscribed), (6, false));
        tool.remember(&mut r);
        assert_eq!((r.polygon_sides, r.polygon_circumscribed), (7, true));

        let mut fresh = ToolKind::Polygon.make_with(&r);
        on_ground(&mut *fresh, ToolEvent::Click(ground(0.0, 0.0)));
        let out = on_ground(&mut *fresh, ToolEvent::Click(ground(1.0, 0.0)));
        assert_eq!(
            out.commands,
            vec![polygon([0.0; 3], 1.0, 7, true, Some([1.0, 0.0, 0.0]))]
        );
    }
}
