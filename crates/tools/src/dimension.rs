//! The Dimension tool: adds dimensions. Click a point, click a second one, move the cursor to
//! set how far the dimension line stands off, and click to place it (a typed length sets that
//! distance, on the cursor's side). Or click an edge away from its ends, then click to place a
//! dimension of its length; or click an edge of a circle or an arc, then click where its label
//! goes for a radial dimension (a diameter for a circle, a radius for an arc; Ctrl switches
//! between the two before that click). A dimension clicked on a vertex of the open context
//! follows the vertex.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use kernel::{Circle, CurveKind, EdgeId, EntityId};
use session::{AnchorInput, AnnotationShape, Command, Units};

use crate::inference::{Guides, Inference, SnapKind, infer_detail};
use crate::scenery::Owner;
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length};

/// A clicked point, and what an annotation anchored there holds on to: the open context's
/// vertex there (an endpoint snap), or the point itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct End {
    pub point: DVec3,
    pub input: AnchorInput,
}

impl End {
    /// The end a click at `hit` gives.
    pub(crate) fn at(hit: &Inference, ctx: &ToolContext) -> End {
        let vertex = (hit.kind == SnapKind::Endpoint)
            .then(|| {
                ctx.model
                    .vertices()
                    .find(|(_, v)| v.pos.distance(hit.point) <= LENGTH_TOL)
            })
            .flatten();
        match vertex {
            Some((id, v)) => End {
                point: v.pos,
                input: AnchorInput::Vertex(EntityId::Vertex(id)),
            },
            None => End {
                point: hit.point,
                input: AnchorInput::Point(hit.point.to_array()),
            },
        }
    }
}

/// Where the tool is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Idle,
    /// The first point was clicked; the second comes next.
    First(End),
    /// Both ends are known (two clicks, or an edge clicked: `edge` then is the open context's
    /// edge, measured by its vertices); the offset comes next.
    Offset {
        a: End,
        b: End,
        edge: Option<EntityId>,
    },
    /// An edge of a circle or an arc was clicked; where the label goes comes next.
    Radial {
        edge: EntityId,
        circle: Circle,
        diameter: bool,
    },
}

#[derive(Clone, Debug)]
pub struct DimensionTool {
    step: Step,
    /// The point under the cursor.
    hover: Option<DVec3>,
    /// The ends and unit offset direction of the linear dimension just added, for a typed
    /// distance to replace it.
    last: Option<(End, End, Option<EntityId>, DVec3)>,
}

impl Default for DimensionTool {
    fn default() -> Self {
        DimensionTool {
            step: Step::Idle,
            hover: None,
            last: None,
        }
    }
}

/// The part of `v` perpendicular to the line from `a` to `b`.
fn square_to(v: DVec3, a: DVec3, b: DVec3) -> DVec3 {
    match (b - a).try_normalize() {
        Some(u) => v - u * v.dot(u),
        None => v,
    }
}

/// The command adding a linear dimension between `a` and `b`, `offset` off them.
fn linear(a: End, b: End, edge: Option<EntityId>, offset: DVec3) -> Command {
    let offset = offset.to_array();
    match edge {
        Some(edge) => Command::AddEdgeDimension { edge, offset },
        None => Command::AddDimension {
            a: a.input,
            b: b.input,
            offset,
        },
    }
}

impl DimensionTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The offset the cursor gives a dimension between `a` and `b`.
    fn offset(&self, a: End, b: End) -> Option<DVec3> {
        Some(square_to(self.hover? - a.point, a.point, b.point))
    }

    /// The edge `hit` snapped onto (`SnapKind::OnEdge`): whose it is, the edge (an id in its
    /// own mesh) and its ends in the open context's coordinates. The inference's own detail:
    /// before the first click a tool has no anchor, so the editor's inference (its reference
    /// edge and locks need an anchor) is `infer_detail`'s without guides.
    fn snapped_edge(hit: &Inference, ctx: &ToolContext) -> Option<(Owner, EdgeId, DVec3, DVec3)> {
        if hit.kind != SnapKind::OnEdge {
            return None;
        }
        let guides = Guides::default();
        let (again, involved) = infer_detail(
            ctx.scenery,
            ctx.camera,
            ctx.cursor,
            &guides,
            ctx.pick_radius,
        );
        if again.kind != SnapKind::OnEdge || again.point.distance(hit.point) > LENGTH_TOL {
            return None;
        }
        let [(k, EntityId::Edge(e))] = involved[..] else {
            return None;
        };
        let part = ctx.scenery.part(k);
        let [a, b] = part.model.edge(e)?.v;
        let end = |v| {
            part.model
                .vertex(v)
                .map(|at| part.place.transform_point3(at.pos))
        };
        Some((part.owner, e, end(a)?, end(b)?))
    }

    /// What a first click at `hit` starts: on an edge away from its ends, a dimension of the
    /// edge (radial on a circle or an arc of the open context); elsewhere at a point.
    fn start(hit: &Inference, ctx: &ToolContext) -> Step {
        if let Some((owner, e, a, b)) = Self::snapped_edge(hit, ctx) {
            let point = |p: DVec3| End {
                point: p,
                input: AnchorInput::Point(p.to_array()),
            };
            if owner != Owner::Open {
                return Step::Offset {
                    a: point(a),
                    b: point(b),
                    edge: None,
                };
            }
            if let Some(curve) = ctx.model.curve_of(e)
                && let Some(c) = curve.circle
                && matches!(curve.kind(), CurveKind::Circle | CurveKind::Arc)
            {
                return Step::Radial {
                    edge: EntityId::Edge(e),
                    circle: Circle {
                        center: c.center,
                        normal: c.normal,
                        radius: c.radius,
                    },
                    diameter: curve.kind() == CurveKind::Circle,
                };
            }
            let ends = ctx.model.edge(e).map(|edge| edge.v);
            let vertex = |v| {
                ctx.model.vertex(v).map(|at| End {
                    point: at.pos,
                    input: AnchorInput::Vertex(EntityId::Vertex(v)),
                })
            };
            if let Some([va, vb]) = ends
                && let (Some(a), Some(b)) = (vertex(va), vertex(vb))
            {
                return Step::Offset {
                    a,
                    b,
                    edge: Some(EntityId::Edge(e)),
                };
            }
        }
        Step::First(End::at(hit, ctx))
    }

    /// Adds the linear dimension between `a` and `b` at `offset`.
    fn finish(
        &mut self,
        a: End,
        b: End,
        edge: Option<EntityId>,
        offset: DVec3,
        out: &mut ToolOutput,
    ) {
        out.commands.push(linear(a, b, edge, offset));
        self.last = offset.try_normalize().map(|dir| (a, b, edge, dir));
        self.step = Step::Idle;
    }

    /// What a typed value does: in the offset step it sets the offset's length on the cursor's
    /// side; right after a dimension was added it moves that dimension's line.
    fn typed(&mut self, text: &str, units: &Units, out: &mut ToolOutput) {
        let Some(length) = parse_length(text, units) else {
            out.message = Some(format!("Not a length: {text}"));
            return;
        };
        if length.abs() <= LENGTH_TOL {
            out.message = Some("The dimension line must stand off the measured line".into());
            return;
        }
        match (self.step, self.last) {
            (Step::Offset { a, b, edge }, _) => {
                match self.offset(a, b).and_then(DVec3::try_normalize) {
                    Some(dir) => self.finish(a, b, edge, dir * length, out),
                    None => {
                        out.message = Some("Move the cursor to the side the line goes".into());
                    }
                }
            }
            (Step::Idle, Some((a, b, edge, dir))) => {
                out.commands.push(Command::Undo {
                    count: None,
                    expect: None,
                });
                self.finish(a, b, edge, dir * length, out);
            }
            (Step::Idle, None) => {
                out.message = Some("Click where the dimension starts first".into());
            }
            (Step::First(_), _) => {
                out.message = Some("Click the second point first".into());
            }
            (Step::Radial { .. }, _) => {
                out.message = Some("Click where the label goes".into());
            }
        }
    }

    /// The dimension to be, as the cursor would place it now.
    fn preview(&self) -> Option<AnnotationShape> {
        let hover = self.hover?;
        match self.step {
            Step::Idle => None,
            Step::First(a) => Some(AnnotationShape::Leader {
                anchor: a.point,
                position: hover,
            }),
            Step::Offset { a, b, .. } => {
                let off = self.offset(a, b)?;
                Some(AnnotationShape::Linear {
                    a: a.point,
                    b: b.point,
                    line: [a.point + off, b.point + off],
                })
            }
            Step::Radial {
                circle, diameter, ..
            } => Some(AnnotationShape::Radial {
                circle,
                label: hover,
                diameter,
            }),
        }
    }
}

impl Tool for DimensionTool {
    fn name(&self) -> &'static str {
        "Dimension"
    }

    fn prompt(&self) -> &'static str {
        match self.step {
            Step::Idle => "Click a point, or an edge, an arc or a circle",
            Step::First(_) => "Click the second point",
            Step::Offset { .. } => "Click where the dimension line goes, or type its distance",
            Step::Radial { diameter: true, .. } => {
                "Click where the label goes; Ctrl switches to the radius"
            }
            Step::Radial {
                diameter: false, ..
            } => "Click where the label goes; Ctrl switches to the diameter",
        }
    }

    fn measurement(&self, units: &session::Units) -> (&'static str, Option<String>) {
        match self.step {
            Step::Idle => ("Length", None),
            Step::First(a) => (
                "Length",
                self.hover
                    .map(|h| format_length(a.point.distance(h), units)),
            ),
            Step::Offset { a, b, .. } => (
                "Offset",
                self.offset(a, b).map(|o| format_length(o.length(), units)),
            ),
            Step::Radial {
                circle,
                diameter: true,
                ..
            } => ("Diameter", Some(format_length(circle.radius * 2.0, units))),
            Step::Radial {
                circle,
                diameter: false,
                ..
            } => ("Radius", Some(format_length(circle.radius, units))),
        }
    }

    fn anchor(&self) -> Option<DVec3> {
        match self.step {
            Step::First(a) => Some(a.point),
            _ => None,
        }
    }

    /// The points clicked so far: the ends, or a circle's center.
    fn phase(&self) -> Phase {
        let points = match self.step {
            Step::Idle => return Phase::Idle,
            Step::First(a) => vec![a.point],
            Step::Offset { a, b, .. } => vec![a.point, b.point],
            Step::Radial { circle, .. } => vec![circle.center],
        };
        Phase::Active { points }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => {
                self.hover = Some(hit.point);
                match self.step {
                    Step::Idle => {
                        self.step = Self::start(&hit, ctx);
                        self.last = None;
                    }
                    Step::First(a) => {
                        let b = End::at(&hit, ctx);
                        if a.point.distance(b.point) <= LENGTH_TOL {
                            out.message = Some("Click away from the first point".into());
                        } else {
                            self.step = Step::Offset { a, b, edge: None };
                        }
                    }
                    Step::Offset { a, b, edge } => {
                        let offset = square_to(hit.point - a.point, a.point, b.point);
                        // On the measured line there is no side for the line to stand off
                        // to (nor for a distance typed next).
                        if offset.length() <= LENGTH_TOL {
                            out.message = Some(
                                "Click off the measured line, where the dimension goes".into(),
                            );
                        } else {
                            self.finish(a, b, edge, offset, &mut out);
                        }
                    }
                    Step::Radial {
                        edge,
                        circle,
                        diameter,
                    } => {
                        out.commands.push(Command::AddRadialDimension {
                            edge,
                            diameter,
                            leader: (hit.point - circle.center).to_array(),
                        });
                        self.step = Step::Idle;
                    }
                }
            }
            ToolEvent::Value(text) => self.typed(&text, &ctx.units, &mut out),
            ToolEvent::ToggleCopy => {
                if let Step::Radial { diameter, .. } = &mut self.step {
                    *diameter = !*diameter;
                    out.message = Some(if *diameter { "Diameter" } else { "Radius" }.into());
                    out.info = true;
                }
            }
            ToolEvent::Cancel => self.on_external_change(),
            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) => {}
        }
        if let Some(shape) = self.preview() {
            out.preview = shape.segments();
        }
        out
    }

    fn on_external_change(&mut self) {
        self.step = Step::Idle;
        self.last = None;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::inference::infer;
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::{Model, VertexId};
    use session::{Camera, circle_points};

    /// An edge from (0, 0, 0) to (4, 0, 0) and an 8-sided circle of radius 2 around
    /// (0, 6, 0), seen by the default camera.
    pub(crate) struct Rig<T> {
        pub tool: T,
        pub model: Model,
        pub camera: Camera,
        pub cursor: DVec2,
    }

    impl<T: Tool> Rig<T> {
        pub fn new(tool: T) -> Self {
            let mut model = Model::new();
            model
                .add_edge(DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0))
                .unwrap();
            model.add_curve(&circle(), true, false).unwrap();
            Rig {
                tool,
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
            }
        }

        pub fn send(&mut self, event: ToolEvent) -> ToolOutput {
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
        pub fn at(&mut self, p: DVec3) -> Inference {
            self.cursor = self.camera.project(p).unwrap();
            infer(
                &self.model,
                &self.camera,
                self.cursor,
                self.tool.anchor(),
                10.0,
            )
        }

        pub fn click(&mut self, p: DVec3) -> ToolOutput {
            let hit = self.at(p);
            self.send(ToolEvent::Click(hit))
        }

        pub fn hover(&mut self, p: DVec3) -> ToolOutput {
            let hit = self.at(p);
            self.send(ToolEvent::Move(hit))
        }

        /// The vertex at `p`.
        pub fn vertex(&self, p: DVec3) -> AnchorInput {
            let (id, _): (VertexId, _) = self
                .model
                .vertices()
                .find(|(_, v)| v.pos.distance(p) < 1e-9)
                .unwrap();
            AnchorInput::Vertex(EntityId::Vertex(id))
        }
    }

    /// The corners of the rig's circle.
    pub(crate) fn circle() -> Vec<DVec3> {
        circle_points(DVec3::new(0.0, 6.0, 0.0), 2.0, None, 8).unwrap()
    }

    fn rig() -> Rig<DimensionTool> {
        Rig::new(DimensionTool::new())
    }

    fn v(x: f64, y: f64) -> DVec3 {
        DVec3::new(x, y, 0.0)
    }

    /// The offset of the one linear dimension `commands` add.
    fn offset_of(commands: &[Command]) -> DVec3 {
        match commands.last() {
            Some(
                Command::AddDimension { offset, .. } | Command::AddEdgeDimension { offset, .. },
            ) => DVec3::from(*offset),
            other => panic!("no linear dimension: {other:?}"),
        }
    }

    #[test]
    fn two_points_and_an_offset_click_add_a_dimension_that_follows_the_vertices() {
        let mut r = rig();
        r.click(v(0.0, 0.0));
        assert_eq!(
            r.tool.phase(),
            Phase::Active {
                points: vec![v(0.0, 0.0)]
            }
        );
        r.hover(v(4.0, 0.0));
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Length", Some("4 m".into()))
        );
        r.click(v(4.0, 0.0));
        let out = r.hover(v(2.0, 1.0));
        assert_eq!(
            out.preview.len(),
            3,
            "two extension lines and the dimension line"
        );
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Offset", Some("1 m".into()))
        );
        let out = r.click(v(2.5, 1.5));
        let [Command::AddDimension { a, b, offset }] = out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert_eq!((a, b), (r.vertex(v(0.0, 0.0)), r.vertex(v(4.0, 0.0))));
        assert!(
            DVec3::from(offset).abs_diff_eq(v(0.0, 1.5), 1e-9),
            "{offset:?}"
        );
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn points_off_the_vertices_are_fixed_and_the_same_point_twice_is_refused() {
        let mut r = rig();
        r.click(v(1.0, -2.0));
        let out = r.click(v(1.0, -2.0));
        assert!(out.message.unwrap().contains("away"));
        r.click(v(3.0, -2.0));
        let out = r.click(v(2.0, -3.0));
        let [Command::AddDimension { a, b, .. }] = out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        let near = |end: AnchorInput, q: DVec3| matches!(end, AnchorInput::Point(p) if DVec3::from(p).abs_diff_eq(q, 1e-9));
        assert!(
            near(a, v(1.0, -2.0)) && near(b, v(3.0, -2.0)),
            "{a:?} {b:?}"
        );
    }

    #[test]
    fn a_typed_distance_sets_the_offset_on_the_cursors_side_and_retyping_replaces_it() {
        let mut r = rig();
        r.click(v(0.0, 0.0));
        r.click(v(4.0, 0.0));
        let out = r.send(ToolEvent::Value("0.5".into()));
        assert!(
            out.commands.is_empty() && out.message.is_some(),
            "no side yet"
        );
        r.hover(v(1.0, -2.0));
        let out = r.send(ToolEvent::Value("0.5".into()));
        assert!(offset_of(&out.commands).abs_diff_eq(v(0.0, -0.5), 1e-9));
        let out = r.send(ToolEvent::Value("0.75".into()));
        assert!(
            matches!(out.commands[0], Command::Undo { .. }),
            "{:?}",
            out.commands
        );
        assert!(offset_of(&out.commands).abs_diff_eq(v(0.0, -0.75), 1e-9));
        let out = r.send(ToolEvent::Value("0".into()));
        assert!(out.commands.is_empty() && out.message.unwrap().contains("stand off"));
        let out = r.send(ToolEvent::Value("one".into()));
        assert!(out.commands.is_empty() && out.message.unwrap().contains("Not a length"));
    }

    #[test]
    fn a_click_on_the_measured_line_is_refused_and_typing_still_places_the_line() {
        let mut r = rig();
        r.click(v(0.0, -2.0));
        r.click(v(3.0, -2.0));
        let out = r.click(v(1.5, -2.0));
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        assert!(out.message.unwrap().contains("off the measured line"));
        assert!(matches!(r.tool.phase(), Phase::Active { ref points } if points.len() == 2));
        // The offset step goes on: a typed distance on the cursor's side, then retyped.
        r.hover(v(1.5, -1.0));
        let out = r.send(ToolEvent::Value("0.5".into()));
        assert!(offset_of(&out.commands).abs_diff_eq(v(0.0, 0.5), 1e-9));
        let out = r.send(ToolEvent::Value("0.25".into()));
        assert!(matches!(out.commands[0], Command::Undo { .. }));
        assert!(offset_of(&out.commands).abs_diff_eq(v(0.0, 0.25), 1e-9));
    }

    #[test]
    fn an_edge_clicked_away_from_its_ends_is_measured() {
        let mut r = rig();
        r.click(v(1.3, 0.0));
        assert_eq!(
            r.tool.phase(),
            Phase::Active {
                points: vec![v(0.0, 0.0), v(4.0, 0.0)]
            }
        );
        let out = r.click(v(2.0, 1.0));
        let [Command::AddEdgeDimension { edge, offset }] = out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert!(matches!(edge, EntityId::Edge(_)));
        assert!(DVec3::from(offset).abs_diff_eq(v(0.0, 1.0), 1e-9));
    }

    #[test]
    fn a_circle_edge_gives_a_diameter_and_ctrl_a_radius() {
        let mut r = rig();
        // Along the circle's first side, away from its ends.
        let points = circle();
        r.click(points[0].lerp(points[1], 0.3));
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Diameter", Some("4 m".into()))
        );
        let out = r.hover(v(3.0, 7.0));
        assert!(!out.preview.is_empty());
        let out = r.send(ToolEvent::ToggleCopy);
        assert_eq!(out.message.as_deref(), Some("Radius"));
        let out = r.send(ToolEvent::Value("2".into()));
        assert!(out.commands.is_empty() && out.message.is_some());
        let out = r.click(v(3.0, 7.0));
        let [
            Command::AddRadialDimension {
                edge,
                diameter,
                leader,
            },
        ] = out.commands[..]
        else {
            panic!("{:?}", out.commands);
        };
        assert!(matches!(edge, EntityId::Edge(_)) && !diameter);
        assert!(
            DVec3::from(leader).abs_diff_eq(v(3.0, 1.0), 1e-6),
            "{leader:?}"
        );
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn escape_drops_the_dimension_in_progress() {
        let mut r = rig();
        r.click(v(0.0, 0.0));
        r.click(v(4.0, 0.0));
        let out = r.send(ToolEvent::Cancel);
        assert!(out.commands.is_empty() && out.preview.is_empty());
        assert_eq!(r.tool.phase(), Phase::Idle);
        let out = r.send(ToolEvent::Value("1".into()));
        assert!(out.commands.is_empty(), "nothing to retype");
    }
}
