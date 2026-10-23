//! The Tape Measure tool: measures distances and draws guides. Click where to measure from,
//! then where to measure to; the measurements box shows the distance. Measured from a point,
//! the second click adds a guide point there; measured from an edge or a guide line (clicked
//! anywhere along it), the distance is taken square to that line and the second click adds a
//! guide line parallel to it through the end. A typed length sets the distance, in the
//! direction of the cursor; typed right after a guide was added, it moves that guide to the
//! new distance. Ctrl switches guide creation off (measuring only) and on again.

use glam::DVec3;
use kernel::Guide;
use kernel::geom::LENGTH_TOL;
use session::{Command, Units};

use crate::inference::{SnapKind, edge_under_in, guide_under_where};
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, is_point_snap, parse_length, snap_length};

/// Where a measurement starts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Start {
    /// At a point: distances are straight to the cursor.
    Point(DVec3),
    /// On the line through `point` (where it was clicked) along the unit `dir`: distances are
    /// square to it.
    Line { point: DVec3, dir: DVec3 },
}

impl Start {
    fn point(self) -> DVec3 {
        match self {
            Start::Point(p) | Start::Line { point: p, .. } => p,
        }
    }

    /// The offset from the start to `p`: to the point, or square to the line.
    fn offset(self, p: DVec3) -> DVec3 {
        match self {
            Start::Point(s) => p - s,
            Start::Line { point, dir } => {
                let d = p - point;
                d - dir * d.dot(dir)
            }
        }
    }

    /// The guide `offset` away from the start: a guide point, or a parallel guide line.
    fn guide(self, offset: DVec3) -> Command {
        match self {
            Start::Point(s) => Command::AddGuidePoint {
                position: (s + offset).to_array(),
            },
            Start::Line { point, dir } => Command::AddGuideLine {
                point: (point + offset).to_array(),
                direction: dir.to_array(),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct TapeMeasureTool {
    start: Option<Start>,
    /// The point under the cursor.
    hover: Option<DVec3>,
    /// Whether the second click adds a guide (Ctrl toggles it).
    guides: bool,
    /// The start and unit offset direction of the guide just added, for a typed distance to
    /// replace it.
    last: Option<(Start, DVec3)>,
}

impl Default for TapeMeasureTool {
    fn default() -> Self {
        TapeMeasureTool {
            start: None,
            hover: None,
            guides: true,
            last: None,
        }
    }
}

impl TapeMeasureTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The live distance from the start to the cursor.
    fn distance(&self) -> Option<f64> {
        Some(self.start?.offset(self.hover?).length())
    }

    /// Ends the measurement at `offset` from the start: adds the guide, or reports the
    /// distance when measuring only.
    fn finish(&mut self, start: Start, offset: DVec3, units: &Units, out: &mut ToolOutput) {
        let length = offset.length();
        if length < LENGTH_TOL {
            out.message = Some(match start {
                Start::Point(_) => "Click away from the start point".into(),
                Start::Line { .. } => "Click away from the line to place a guide".into(),
            });
            return;
        }
        if self.guides {
            out.commands.push(start.guide(offset));
            self.last = Some((start, offset / length));
        } else {
            out.message = Some(format!("Distance: {}", format_length(length, units)));
            out.info = true;
        }
        self.start = None;
    }

    /// Where the cursor is: its inference's point, moved so its distance from the start is a
    /// multiple of the model's length snapping unless a point of the model was snapped to.
    fn snapped(&self, hit: &crate::inference::Inference, units: &Units) -> DVec3 {
        let Some(start) = self.start else {
            return hit.point;
        };
        if units.length_snap <= 0.0 || is_point_snap(hit.kind) {
            return hit.point;
        }
        let offset = start.offset(hit.point);
        let length = offset.length();
        if length < LENGTH_TOL {
            return hit.point;
        }
        hit.point - offset + offset * (snap_length(length, units) / length)
    }

    /// Where a click at `hit` starts measuring: along the edge or guide line under the
    /// cursor when it snapped onto one, else at the point.
    fn start_at(hit: &crate::inference::Inference, ctx: &ToolContext) -> Start {
        let line = match hit.kind {
            SnapKind::OnEdge => edge_under_in(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius)
                .and_then(|(_, _, a, b)| (b - a).try_normalize()),
            SnapKind::OnGuide => {
                guide_under_where(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius, |_| {
                    true
                })
                .and_then(|(_, _, g)| match g {
                    Guide::Line { direction, .. } => Some(direction),
                    Guide::Point { .. } => None,
                })
            }
            _ => None,
        };
        match line {
            Some(dir) => Start::Line {
                point: hit.point,
                dir,
            },
            None => Start::Point(hit.point),
        }
    }
}

impl Tool for TapeMeasureTool {
    fn name(&self) -> &'static str {
        "Tape Measure"
    }

    fn prompt(&self) -> &'static str {
        match (self.start, self.guides) {
            (None, true) => {
                "Click a point, or an edge for a parallel guide; Ctrl switches guides off"
            }
            (None, false) => "Click where to measure from; Ctrl switches guides on",
            (Some(_), true) => "Click where to place the guide, or type the distance",
            (Some(_), false) => "Click where to measure to",
        }
    }

    fn measurement(&self, units: &session::Units) -> (&'static str, Option<String>) {
        ("Length", self.distance().map(|d| format_length(d, units)))
    }

    fn anchor(&self) -> Option<DVec3> {
        self.start.map(Start::point)
    }

    /// Where the measurement starts.
    fn phase(&self) -> Phase {
        match self.start {
            Some(start) => Phase::Active {
                points: vec![start.point()],
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(self.snapped(&hit, &ctx.units)),
            ToolEvent::Click(hit) => {
                let p = self.snapped(&hit, &ctx.units);
                self.hover = Some(p);
                match self.start {
                    None => {
                        self.start = Some(Self::start_at(&hit, ctx));
                        self.last = None;
                    }
                    Some(start) => self.finish(start, start.offset(p), &ctx.units, &mut out),
                }
            }
            ToolEvent::Value(text) => {
                let Some(length) = parse_length(&text, &ctx.units) else {
                    out.message = Some(format!("Not a length: {text}"));
                    return out;
                };
                if length.abs() < LENGTH_TOL {
                    out.message = Some("The distance must not be zero".into());
                    return out;
                }
                match (self.start, self.last) {
                    (Some(start), _) => {
                        if !self.guides {
                            out.message =
                                Some("Measuring only: Ctrl switches guides on to place one".into());
                        } else if let Some(dir) =
                            self.hover.and_then(|h| start.offset(h).try_normalize())
                        {
                            self.finish(start, dir * length, &ctx.units, &mut out);
                        } else {
                            out.message = Some("Move the cursor to show the direction".into());
                        }
                    }
                    // Right after a guide was added: move it to the typed distance.
                    (None, Some((start, dir))) => {
                        out.commands.push(Command::Undo {
                            count: None,
                            expect: None,
                        });
                        self.finish(start, dir * length, &ctx.units, &mut out);
                    }
                    (None, None) => {
                        out.message = Some("Click where to measure from first".into());
                    }
                }
            }
            ToolEvent::ToggleCopy => {
                self.guides = !self.guides;
                self.last = None;
                out.message = Some(if self.guides {
                    "Guides on".into()
                } else {
                    "Measuring only".into()
                });
                out.info = true;
            }
            ToolEvent::Cancel => self.on_external_change(),
            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) => {}
        }
        if let (Some(start), Some(hover)) = (self.start, self.hover) {
            let offset = start.offset(hover);
            out.preview.push([hover - offset, hover]);
            if let Start::Line { dir, .. } = start
                && self.guides
                && offset.length() >= LENGTH_TOL
            {
                out.preview.extend(ctx.camera.line_span(hover, dir));
            }
        }
        out
    }

    fn on_external_change(&mut self) {
        self.start = None;
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, infer};
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;

    /// An edge from (0, 0, 0) to (4, 0, 0) and a guide line along Y through (6, 0, 0), seen by
    /// the default camera.
    struct Rig {
        tool: TapeMeasureTool,
        model: Model,
        camera: Camera,
        cursor: DVec2,
    }

    impl Rig {
        fn new() -> Self {
            let mut model = Model::new();
            model
                .add_edge(DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0))
                .unwrap();
            model
                .add_guide(Guide::Line {
                    point: DVec3::new(6.0, 0.0, 0.0),
                    direction: DVec3::Y,
                    tag: None,
                    hidden: false,
                })
                .unwrap();
            Rig {
                tool: TapeMeasureTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let scenery = Scenery::single(&self.model);
            let ctx = ToolContext {
                model: &self.model,
                scenery: &scenery,
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                drag_threshold: 4.0,
                scale_factor: 1.0,
                to_world: glam::DAffine3::IDENTITY,
                quick_press: false,
                units: session::Units::default(),
                shift: false,
                selection: &Default::default(),
                material: None,
                sample: false,
                labels: &[],
            };
            self.tool.handle(event, &ctx)
        }

        /// The inference with the cursor over `p`.
        fn at(&mut self, p: DVec3) -> Inference {
            self.cursor = self.camera.project(p).unwrap();
            infer(
                &self.model,
                &self.camera,
                self.cursor,
                self.tool.anchor(),
                10.0,
            )
        }

        fn click(&mut self, p: DVec3) -> ToolOutput {
            let hit = self.at(p);
            self.send(ToolEvent::Click(hit))
        }

        fn hover(&mut self, p: DVec3) -> ToolOutput {
            let hit = self.at(p);
            self.send(ToolEvent::Move(hit))
        }
    }

    /// Asserts that `commands` end with a guide line through about `point` along `direction`.
    fn assert_line(commands: &[Command], point: [f64; 3], direction: [f64; 3]) {
        let Some(Command::AddGuideLine {
            point: p,
            direction: d,
        }) = commands.last()
        else {
            panic!("no guide line in {commands:?}");
        };
        assert!(
            DVec3::from(*p).abs_diff_eq(DVec3::from(point), 1e-9) && *d == direction,
            "{commands:?}"
        );
    }

    #[test]
    fn from_an_edge_a_parallel_guide_line_goes_through_the_second_click() {
        let mut r = Rig::new();
        r.click(DVec3::new(1.5, 0.0, 0.0));
        assert_eq!(
            r.tool.start.map(|s| matches!(s, Start::Line { .. })),
            Some(true)
        );
        r.hover(DVec3::new(2.0, 2.5, 0.0));
        let (label, value) = r.tool.measurement(&Default::default());
        assert_eq!((label, value.as_deref()), ("Length", Some("2.5 m")));
        let out = r.click(DVec3::new(2.0, 2.5, 0.0));
        let [Command::AddGuideLine { point, direction }] = out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert!((point[1] - 2.5).abs() < 1e-9 && point[2].abs() < 1e-9);
        assert!((point[0] - 1.5).abs() < 1e-6, "square to the edge");
        assert_eq!(direction, [1.0, 0.0, 0.0]);
        assert_eq!(r.tool.anchor(), None);
    }

    #[test]
    fn from_a_point_the_second_click_adds_a_guide_point() {
        let mut r = Rig::new();
        r.click(DVec3::new(4.0, 0.0, 0.0));
        assert_eq!(r.tool.start, Some(Start::Point(DVec3::new(4.0, 0.0, 0.0))));
        let out = r.click(DVec3::ZERO);
        assert_eq!(
            out.commands,
            vec![Command::AddGuidePoint { position: [0.0; 3] }]
        );
        // Clicking the start point again adds nothing.
        r.click(DVec3::new(4.0, 0.0, 0.0));
        let out = r.click(DVec3::new(4.0, 0.0, 0.0));
        assert!(out.commands.is_empty());
        assert!(out.message.unwrap().contains("away"));
    }

    #[test]
    fn from_a_guide_line_a_parallel_guide_line_too() {
        let mut r = Rig::new();
        r.click(DVec3::new(6.0, 3.0, 0.0));
        let out = r.send(ToolEvent::Value("1.5".into()));
        assert!(out.message.unwrap().contains("Move the cursor"));
        r.hover(DVec3::new(8.0, 3.0, 0.0));
        let out = r.send(ToolEvent::Value("1.5".into()));
        assert_eq!(out.commands.len(), 1);
        assert_line(&out.commands, [7.5, 3.0, 0.0], [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_typed_length_follows_the_cursor_and_retyping_replaces_the_guide() {
        let mut r = Rig::new();
        r.click(DVec3::new(2.5, 0.0, 0.0));
        r.hover(DVec3::new(2.5, -1.0, 0.0));
        let out = r.send(ToolEvent::Value("2".into()));
        assert_eq!(out.commands.len(), 1);
        assert_line(&out.commands, [2.5, -2.0, 0.0], [1.0, 0.0, 0.0]);
        let out = r.send(ToolEvent::Value("3".into()));
        assert_eq!(
            (out.commands.len(), &out.commands[0]),
            (
                2,
                &Command::Undo {
                    count: None,
                    expect: None
                }
            )
        );
        assert_line(&out.commands, [2.5, -3.0, 0.0], [1.0, 0.0, 0.0]);
        // A new measurement ends the chance to retype.
        r.click(DVec3::new(4.0, 0.0, 0.0));
        r.send(ToolEvent::Cancel);
        let out = r.send(ToolEvent::Value("3".into()));
        assert!(out.commands.is_empty());
    }

    #[test]
    fn ctrl_switches_to_measuring_only() {
        let mut r = Rig::new();
        let out = r.send(ToolEvent::ToggleCopy);
        assert_eq!(out.message.as_deref(), Some("Measuring only"));
        r.click(DVec3::ZERO);
        let out = r.click(DVec3::new(4.0, 0.0, 0.0));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Distance: 4 m"));
        r.click(DVec3::ZERO);
        r.hover(DVec3::new(4.0, 0.0, 0.0));
        let out = r.send(ToolEvent::Value("2".into()));
        assert!(out.commands.is_empty() && out.message.is_some());
        r.send(ToolEvent::ToggleCopy);
        let out = r.send(ToolEvent::Value("2".into()));
        assert_eq!(
            out.commands,
            vec![Command::AddGuidePoint {
                position: [2.0, 0.0, 0.0]
            }]
        );
    }

    #[test]
    fn the_preview_shows_the_measure_and_the_guide_to_be() {
        let mut r = Rig::new();
        r.click(DVec3::new(1.0, 0.0, 0.0));
        let out = r.hover(DVec3::new(1.0, 2.0, 0.0));
        assert_eq!(out.preview.len(), 2, "the measure and the guide line");
        assert!(out.preview[0][0].abs_diff_eq(DVec3::new(1.0, 0.0, 0.0), 1e-6));
        let out = r.send(ToolEvent::Cancel);
        assert!(out.preview.is_empty());
        assert_eq!(r.tool.anchor(), None);
    }
}
