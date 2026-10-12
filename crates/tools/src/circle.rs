//! The Circle tool: click the center, then a point on the rim (or type the radius). The
//! circle lies on the face under the center click, or on a horizontal plane. Typing "24s" sets
//! the number of sides, before or during a circle; the count is kept for later circles.

use glam::DVec3;
use session::{Command, DEFAULT_CIRCLE_SEGMENTS, circle_points};

use crate::rectangle::{on_plane, plane_normal};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length, parse_segments};

#[derive(Clone, Debug)]
pub struct CircleTool {
    /// The center and the normal of the plane the circle lies in.
    center: Option<(DVec3, DVec3)>,
    /// The point on the rim under the cursor, on that plane.
    hover: Option<DVec3>,
    segments: u32,
}

impl Default for CircleTool {
    fn default() -> Self {
        CircleTool {
            center: None,
            hover: None,
            segments: DEFAULT_CIRCLE_SEGMENTS,
        }
    }
}

impl CircleTool {
    pub fn new() -> Self {
        Self::default()
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

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => {
                if let Some((center, normal)) = self.center {
                    self.hover = on_plane(center, normal, &hit, ctx);
                }
            }
            ToolEvent::Click(hit) => match self.center {
                None => self.center = Some((hit.point, plane_normal(&hit, ctx))),
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
                        }
                        Err(e) => out.message = Some(e.message),
                    }
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
            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::ToggleCopy => {}
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
    }
}
