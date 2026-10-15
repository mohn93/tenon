//! The Arc tool (SketchUp's 2-point arc): click the start, click the end, then pull the bulge
//! out and click (or type its height). The arc lies on the face under the start click, or on a
//! horizontal plane. Before the end click, a typed length sets the chord along the cursor's
//! direction. Typing "12s" sets the number of segments, before or during an arc; the count is
//! kept for the session, also when another tool is selected in between.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::{Command, DEFAULT_ARC_SEGMENTS, MAX_COORDINATE, arc_from_bulge, arc_points};

use crate::rectangle::{on_plane, plane_normal};
use crate::tool::{Phase, Remembered, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, meant_as_count, parse_length, parse_segments};

/// Shown for a segment count that is not a whole number in range.
const BAD_SEGMENTS: &str = "Segments must be a whole number from 2 to 360, like 12s";

#[derive(Clone, Debug)]
pub struct ArcTool {
    /// The start and the normal of the plane the arc lies in.
    start: Option<(DVec3, DVec3)>,
    /// The end, once clicked.
    end: Option<DVec3>,
    /// The cursor's point on the arc's plane.
    hover: Option<DVec3>,
    segments: u32,
}

impl Default for ArcTool {
    fn default() -> Self {
        ArcTool {
            start: None,
            end: None,
            hover: None,
            segments: DEFAULT_ARC_SEGMENTS,
        }
    }
}

impl ArcTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// An Arc tool drawing arcs of `segments` segments (a count `arc_points` accepts).
    pub fn with_segments(segments: u32) -> Self {
        ArcTool {
            segments,
            ..Self::default()
        }
    }

    /// The unit direction across the chord, to the left of start-to-end looking down the
    /// normal: positive bulges go this way.
    fn left(&self) -> Option<DVec3> {
        let ((start, normal), end) = (self.start?, self.end?);
        normal.cross(end - start).try_normalize()
    }

    /// The bulge the cursor pulls: its distance from the chord, signed by side.
    fn bulge(&self) -> Option<f64> {
        let ((start, _), end, hover) = (self.start?, self.end?, self.hover?);
        Some((hover - (start + end) / 2.0).dot(self.left()?))
    }

    /// The `AddArc` command for the arc with `bulge`, or why it cannot be drawn.
    fn command(&self, bulge: f64) -> Result<Command, String> {
        let ((start, normal), end) = (
            self.start.ok_or("Click the start first")?,
            self.end.ok_or("Click the end first")?,
        );
        let (center, angle) = arc_from_bulge(start, end, bulge, normal)
            .ok_or("Pull the arc away from the line between its ends")?;
        let within = |p: DVec3| p.is_finite() && p.abs().max_element() <= MAX_COORDINATE;
        let too_shallow = || "Pull the arc further from the line between its ends".to_string();
        if !within(center) {
            return Err(too_shallow());
        }
        let points =
            arc_points(center, start, angle, Some(normal), self.segments).map_err(|e| {
                if e.message.contains("far enough") {
                    format!("Too small for {} segments", self.segments)
                } else {
                    e.message
                }
            })?;
        if !points.iter().all(|&p| within(p)) {
            return Err(too_shallow());
        }
        Ok(Command::AddArc {
            center: Some(center.to_array()),
            start: start.to_array(),
            angle: Some(angle),
            end: None,
            through: None,
            bulge: None,
            normal: Some(normal.to_array()),
            segments: Some(self.segments),
        })
    }

    /// Emits the arc with `bulge` and starts over, or explains why it cannot be drawn.
    fn finish(&mut self, bulge: f64, out: &mut ToolOutput) {
        match self.command(bulge) {
            Ok(c) => {
                out.commands.push(c);
                self.on_external_change();
            }
            Err(message) => out.message = Some(message),
        }
    }

    /// Sets the end, unless it is on the start.
    fn set_end(&mut self, end: DVec3, out: &mut ToolOutput) {
        let Some((start, _)) = self.start else {
            return;
        };
        if end.distance(start) < 10.0 * LENGTH_TOL {
            out.message = Some("The end must be away from the start".into());
        } else {
            self.end = Some(end);
        }
    }
}

impl Tool for ArcTool {
    fn name(&self) -> &'static str {
        "Arc"
    }

    fn prompt(&self) -> &'static str {
        match (self.start, self.end) {
            (None, _) => "Click the start of the arc; type 12s for 12 segments",
            (Some(_), None) => "Click the end of the arc, or type the distance to it",
            (Some(_), Some(_)) => "Pull out the bulge and click, or type its height",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        match (self.start, self.end, self.hover) {
            (Some(_), Some(_), _) => ("Bulge", self.bulge().map(|b| format_length(b.abs()))),
            (Some((s, _)), None, Some(h)) => ("Length", Some(format_length(s.distance(h)))),
            _ => ("Length", None),
        }
    }

    fn anchor(&self) -> Option<DVec3> {
        match (self.start, self.end) {
            (Some((s, _)), Some(e)) => Some((s + e) / 2.0),
            (Some((s, _)), None) => Some(s),
            _ => None,
        }
    }

    /// The start, then the end once clicked.
    fn phase(&self) -> Phase {
        match self.start {
            Some((start, _)) => Phase::Active {
                points: [Some(start), self.end].into_iter().flatten().collect(),
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => {
                if let Some((start, normal)) = self.start {
                    self.hover = on_plane(start, normal, &hit, ctx);
                }
            }
            ToolEvent::Click(hit) => match (self.start, self.end) {
                (None, _) => self.start = Some((hit.point, plane_normal(&hit, ctx))),
                (Some((start, normal)), None) => {
                    if let Some(p) = on_plane(start, normal, &hit, ctx) {
                        self.set_end(p, &mut out);
                    }
                }
                (Some((start, normal)), Some(_)) => {
                    self.hover = on_plane(start, normal, &hit, ctx);
                    if let Some(b) = self.bulge() {
                        self.finish(b, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => {
                if let Some(n) = parse_segments(&text) {
                    match arc_points(DVec3::ZERO, DVec3::X, 90.0, None, n) {
                        Ok(_) => {
                            self.segments = n;
                            out.message = Some(format!("Arcs get {n} segments"));
                        }
                        Err(_) => out.message = Some(BAD_SEGMENTS.into()),
                    }
                } else if meant_as_count(&text) {
                    out.message = Some(BAD_SEGMENTS.into());
                } else {
                    match (self.start, self.end, parse_length(&text)) {
                        (None, _, _) => {
                            out.message = Some("Click the start first, or type 12s".into());
                        }
                        (_, _, None) => {
                            out.message = Some(format!("Not a length or a segment count: {text}"));
                        }
                        (Some((start, _)), None, Some(d)) => {
                            match self.hover.and_then(|h| (h - start).try_normalize()) {
                                Some(dir) => self.set_end(start + dir * d, &mut out),
                                None => {
                                    out.message =
                                        Some("Point toward the end, then type the distance".into())
                                }
                            }
                        }
                        (Some(_), Some(_), Some(h)) => {
                            // The bulge keeps the side the cursor is on (left when on the chord).
                            let side = self.bulge().filter(|b| *b < 0.0).map_or(1.0, |_| -1.0);
                            self.finish(h.abs() * side, &mut out);
                        }
                    }
                }
            }
            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::ToggleCopy => {}
            ToolEvent::Cancel => self.on_external_change(),
        }
        match (self.start, self.end, self.hover) {
            (Some(_), Some(_), _) => {
                if let Some(Command::AddArc {
                    center: Some(center),
                    start,
                    angle: Some(angle),
                    normal,
                    ..
                }) = self.bulge().and_then(|b| self.command(b).ok())
                    && let Ok(points) = arc_points(
                        DVec3::from(center),
                        DVec3::from(start),
                        angle,
                        normal.map(DVec3::from),
                        self.segments,
                    )
                {
                    out.preview = points.windows(2).map(|w| [w[0], w[1]]).collect();
                } else if let (Some((s, _)), Some(e)) = (self.start, self.end) {
                    out.preview = vec![[s, e]];
                }
            }
            (Some((s, _)), None, Some(h)) => out.preview = vec![[s, h]],
            _ => {}
        }
        out
    }

    fn on_external_change(&mut self) {
        self.start = None;
        self.end = None;
        self.hover = None;
    }

    fn remember(&self, r: &mut Remembered) {
        r.arc_segments = self.segments;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::Model;
    use session::{Camera, Document, Query, QueryResult};

    fn ground(x: f64, y: f64) -> Inference {
        Inference {
            point: DVec3::new(x, y, 0.0),
            kind: SnapKind::Ground,
            face: None,
        }
    }

    fn send(tool: &mut ArcTool, event: ToolEvent) -> ToolOutput {
        let model = Model::new();
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let ctx = ToolContext {
            model: &model,
            scenery: &Scenery::single(&model),
            camera: &camera,
            cursor: DVec2::ZERO,
            pick_radius: 10.0,
            drag_threshold: 4.0,
            shift: false,
            selection: &Default::default(),
            material: None,
            sample: false,
        };
        tool.handle(event, &ctx)
    }

    /// The arc's center, start, angle and segments.
    fn parts(c: &Command) -> ([f64; 3], [f64; 3], f64, u32) {
        let Command::AddArc {
            center: Some(center),
            start,
            angle: Some(angle),
            normal,
            segments,
            ..
        } = c
        else {
            panic!("{c:?}");
        };
        assert_eq!(*normal, Some([0.0, 0.0, 1.0]));
        (*center, *start, *angle, segments.unwrap())
    }

    fn near(a: [f64; 3], b: [f64; 3]) -> bool {
        DVec3::from(a).distance(DVec3::from(b)) < 1e-9
    }

    #[test]
    fn three_clicks_draw_an_arc_through_the_bulge() {
        let mut tool = ArcTool::new();
        send(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        send(&mut tool, ToolEvent::Move(ground(2.0, 0.0)));
        assert_eq!(tool.measurement(), ("Length", Some("2 m".into())));
        send(&mut tool, ToolEvent::Click(ground(2.0, 0.0)));
        let preview = send(&mut tool, ToolEvent::Move(ground(1.0, 1.0)));
        assert_eq!(preview.preview.len(), 12, "the arc's segments");
        assert_eq!(tool.measurement(), ("Bulge", Some("1 m".into())));
        let out = send(&mut tool, ToolEvent::Click(ground(1.0, 1.0)));
        let [c] = &out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        let (center, start, angle, segments) = parts(c);
        assert!(near(center, [1.0, 0.0, 0.0]) && near(start, [0.0; 3]));
        assert!(
            (angle + 180.0).abs() < 1e-9,
            "over the top, clockwise: {angle}"
        );
        assert_eq!(segments, 12);
        assert_eq!(tool.anchor(), None, "ready for the next arc");

        // The command draws one arc curve ending at the second click.
        let mut d = Document::new(DVec2::new(800.0, 600.0));
        let made = d.execute(c.clone()).unwrap();
        let edge = made
            .changes
            .created
            .iter()
            .find(|id| matches!(id, kernel::EntityId::Edge(_)))
            .copied()
            .unwrap();
        let Ok(QueryResult::Curve(info)) = d.query(&Query::Curve { id: edge }) else {
            panic!("no curve");
        };
        assert_eq!(info.segments, 12);
        let last = d
            .query(&Query::Entity {
                id: *info.vertices.last().unwrap(),
            })
            .unwrap();
        let QueryResult::Vertex(v) = last else {
            panic!()
        };
        assert!(near(v.position, [0.0; 3]) || near(v.position, [2.0, 0.0, 0.0]));
    }

    #[test]
    fn typed_lengths_set_the_chord_then_the_bulge_on_the_cursor_side() {
        let mut tool = ArcTool::new();
        send(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        send(&mut tool, ToolEvent::Move(ground(5.0, 0.0)));
        send(&mut tool, ToolEvent::Value("4".into()));
        assert_eq!(tool.end, Some(DVec3::new(4.0, 0.0, 0.0)));
        // The cursor is below the chord (to the right of start-to-end): the bulge goes there.
        send(&mut tool, ToolEvent::Move(ground(2.0, -0.3)));
        let out = send(&mut tool, ToolEvent::Value("2".into()));
        let (center, _, angle, _) = parts(&out.commands[0]);
        assert!(near(center, [2.0, 0.0, 0.0]), "{center:?}");
        assert!((angle - 180.0).abs() < 1e-9, "under the chord: {angle}");
    }

    #[test]
    fn a_typed_segment_count_applies_to_later_arcs() {
        let mut tool = ArcTool::new();
        let out = send(&mut tool, ToolEvent::Value("6s".into()));
        assert_eq!(out.message.as_deref(), Some("Arcs get 6 segments"));
        let refused = send(&mut tool, ToolEvent::Value("1s".into()));
        assert_eq!(
            refused.message.as_deref(),
            Some("Segments must be a whole number from 2 to 360, like 12s")
        );
        let refused = send(&mut tool, ToolEvent::Value("400s".into()));
        assert_eq!(
            refused.message.as_deref(),
            Some("Segments must be a whole number from 2 to 360, like 12s")
        );
        let refused = send(&mut tool, ToolEvent::Value("0s".into()));
        assert_eq!(
            refused.message.as_deref(),
            Some("Segments must be a whole number from 2 to 360, like 12s")
        );
        send(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        send(&mut tool, ToolEvent::Click(ground(1.0, 0.0)));
        let out = send(&mut tool, ToolEvent::Click(ground(0.5, 0.2)));
        assert_eq!(parts(&out.commands[0]).3, 6);
    }

    #[test]
    fn a_flat_bulge_or_an_end_on_the_start_is_refused() {
        let mut tool = ArcTool::new();
        send(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        let out = send(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        assert!(out.message.unwrap().contains("away from the start"));
        assert_eq!(tool.end, None);
        send(&mut tool, ToolEvent::Click(ground(2.0, 0.0)));
        let out = send(&mut tool, ToolEvent::Click(ground(1.0, 0.0)));
        assert!(out.commands.is_empty());
        assert!(out.message.unwrap().contains("Pull the arc"));
        // Escape starts over.
        send(&mut tool, ToolEvent::Cancel);
        assert_eq!((tool.start, tool.end), (None, None));
    }

    #[test]
    fn a_too_shallow_or_too_small_arc_is_refused_and_the_tool_keeps_its_ends() {
        let mut tool = ArcTool::new();
        send(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        send(&mut tool, ToolEvent::Click(ground(10_000.0, 0.0)));
        // A 1 mm bulge on a 10 km chord has a center far outside the model limits.
        let out = send(&mut tool, ToolEvent::Click(ground(5_000.0, 0.001)));
        assert!(out.commands.is_empty());
        assert!(out.message.unwrap().contains("Pull the arc further"));
        assert!(tool.start.is_some() && tool.end.is_some(), "ends kept");

        // A tiny arc cannot be split into so many segments.
        let mut tool = ArcTool::new();
        send(&mut tool, ToolEvent::Value("360s".into()));
        send(&mut tool, ToolEvent::Click(ground(0.0, 0.0)));
        send(&mut tool, ToolEvent::Click(ground(0.00002, 0.0)));
        let out = send(&mut tool, ToolEvent::Click(ground(0.00001, 0.00001)));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Too small for 360 segments"));
        assert!(tool.start.is_some() && tool.end.is_some(), "ends kept");
    }
}
