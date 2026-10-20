//! The Offset tool: press a face and move to set how far a copy of its loops goes sideways in
//! its plane (inward, or outward for the outer loop), then click again (or type the distance).
//! With edges forming a chain selected, press near them: the copy goes to the side of the
//! cursor. Right after an offset, a typed distance redoes it; a double-click on a face repeats
//! the last distance (kept for the session).

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::geom::{LENGTH_TOL, point_in_polygon};
use kernel::offset::{offset_arc, offset_chain, offset_face, offset_loop, plane_normal, side_of};
use kernel::{EdgeId, EntityId, FaceId, Model};
use session::{Camera, Command};

use crate::inference::face_under_in;
use crate::scenery::Owner;
use crate::tool::{DragStart, Phase, Remembered, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length, toward_cursor};

/// The smallest distance an offset has (the command's).
const LEAST: f64 = 10.0 * LENGTH_TOL;

const TOO_FAR: &str = "The offset distance is too large for this shape";

/// What is offset.
#[derive(Clone, Debug)]
enum Source {
    Face {
        id: FaceId,
        normal: DVec3,
        /// Outer loop first, counter-clockwise around `normal`, then the holes.
        rings: Vec<Vec<DVec3>>,
    },
    Chain {
        edges: Vec<EdgeId>,
        points: Vec<DVec3>,
        closed: bool,
        /// The unit normal of the plane of the chain.
        normal: DVec3,
        /// The center of the circle an open chain on one arc lies on.
        center: Option<DVec3>,
    },
}

impl Source {
    fn face(model: &Model, id: FaceId) -> Option<Source> {
        let f = model.face(id)?;
        let rings = std::iter::once(&f.outer)
            .chain(&f.holes)
            .map(|l| l.iter().filter_map(|v| model.position(*v)).collect())
            .collect();
        Some(Source::Face {
            id,
            normal: f.normal,
            rings,
        })
    }

    /// The chain the selected edges form, if they form one in a plane. A straight chain has no
    /// plane of its own: it takes its face's, else the ground's (or, upright, the view's).
    fn chain(
        model: &Model,
        selection: &BTreeSet<EntityId>,
        camera: &Camera,
        cursor: DVec2,
    ) -> Option<Source> {
        let edges: Vec<EdgeId> = selection
            .iter()
            .filter_map(|id| match id {
                EntityId::Edge(e) => Some(*e),
                _ => None,
            })
            .collect();
        let (vertices, closed) = model.edge_chain(&edges)?;
        let points: Vec<DVec3> = vertices.iter().filter_map(|v| model.position(*v)).collect();
        let normal = match plane_normal(&points) {
            Some(n) => {
                if points
                    .iter()
                    .any(|p| (*p - points[0]).dot(n).abs() >= LENGTH_TOL)
                {
                    return None;
                }
                n
            }
            None => {
                let dir = (*points.last()? - points[0]).try_normalize()?;
                let face = model
                    .faces_of_edge(edges[0])
                    .first()
                    .and_then(|f| model.face(*f))
                    .map(|f| f.normal);
                let base = face.unwrap_or(DVec3::Z);
                let view = camera.ray(cursor).dir;
                let n = if base.cross(dir).length() < 1e-6 {
                    view
                } else {
                    base
                };
                (n - dir * n.dot(dir)).try_normalize()?
            }
        };
        let curve = model.edge(edges[0]).and_then(|e| e.curve);
        let one = curve.is_some()
            && edges
                .iter()
                .all(|e| model.edge(*e).and_then(|e| e.curve) == curve);
        let center = one
            .then(|| model.curve_of(edges[0]).and_then(|c| c.circle))
            .flatten()
            .filter(|_| !closed)
            .map(|c| c.center);
        Some(Source::Chain {
            edges,
            points,
            closed,
            normal,
            center,
        })
    }

    fn normal(&self) -> DVec3 {
        match self {
            Source::Face { normal, .. } | Source::Chain { normal, .. } => normal.normalize(),
        }
    }

    fn origin(&self) -> DVec3 {
        match self {
            Source::Face { rings, .. } => rings[0][0],
            Source::Chain { points, .. } => points[0],
        }
    }

    /// Where the cursor's ray meets the plane.
    fn plane_point(&self, camera: &Camera, cursor: DVec2) -> Option<DVec3> {
        let ray = camera.ray(cursor);
        let n = self.normal();
        let denom = ray.dir.dot(n);
        if denom.abs() < 1e-9 {
            return None;
        }
        let t = (self.origin() - ray.origin).dot(n) / denom;
        (t > 0.0).then(|| ray.origin + ray.dir * t)
    }

    /// The loops or chains as sides.
    fn sides(&self) -> Vec<(DVec3, DVec3)> {
        let mut out = Vec::new();
        let mut line = |pts: &[DVec3], closed: bool| {
            let n = pts.len();
            let count = if closed { n } else { n.saturating_sub(1) };
            out.extend((0..count).map(|i| (pts[i], pts[(i + 1) % n])));
        };
        match self {
            Source::Face { rings, .. } => rings.iter().for_each(|r| line(r, true)),
            Source::Chain { points, closed, .. } => line(points, *closed),
        }
        out
    }

    /// The distance of the point `p` of the plane from the nearest side: for a face negative
    /// outside its outer loop; and, for a chain, the point itself when it lies to one side.
    fn measure(&self, p: DVec3) -> (f64, Option<DVec3>) {
        let near = self
            .sides()
            .iter()
            .map(|(a, b)| {
                let ab = *b - *a;
                let t = ((p - *a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                (*a + ab * t).distance(p)
            })
            .fold(f64::INFINITY, f64::min);
        match self {
            Source::Face { rings, normal, .. } => {
                let o = rings[0][0];
                let u = (rings[0][1] - o).normalize();
                let v = normal.cross(u);
                let flat = |q: DVec3| DVec2::new((q - o).dot(u), (q - o).dot(v));
                let outer: Vec<DVec2> = rings[0].iter().map(|q| flat(*q)).collect();
                let inside = point_in_polygon(flat(p), &outer);
                (if inside { near } else { -near }, None)
            }
            Source::Chain {
                points,
                closed,
                normal,
                ..
            } => {
                let side = side_of(points, *closed, normal.normalize(), p);
                (near, side.map(|_| p))
            }
        }
    }

    /// The new lines (points, closed) of an offset by `distance` (a face: positive inward; a
    /// chain: positive to the side of `toward`), or why there are none.
    fn lines(
        &self,
        distance: f64,
        toward: Option<DVec3>,
    ) -> Result<Vec<(Vec<DVec3>, bool)>, String> {
        match self {
            Source::Face { rings, normal, .. } => offset_face(rings, *normal, distance)
                .map(|loops| loops.into_iter().map(|l| (l, true)).collect())
                .map_err(|_| TOO_FAR.to_string()),
            Source::Chain {
                points,
                closed,
                normal,
                center,
                ..
            } => {
                let n = normal.normalize();
                let side = toward
                    .and_then(|t| side_of(points, *closed, n, t))
                    .ok_or("Move the cursor to the side to offset")?;
                let d = side * distance;
                match (closed, center) {
                    (true, _) => offset_loop(points, n, d),
                    (false, Some(c)) => offset_arc(points, n, d, *c),
                    (false, None) => offset_chain(points, n, d),
                }
                .map(|l| vec![(l, *closed)])
                .map_err(|_| TOO_FAR.to_string())
            }
        }
    }

    fn command(&self, distance: f64, toward: Option<DVec3>) -> Command {
        match self {
            Source::Face { id, .. } => Command::Offset {
                ids: vec![EntityId::Face(*id)],
                distance,
                toward: None,
            },
            Source::Chain { edges, .. } => Command::Offset {
                ids: edges.iter().map(|e| EntityId::Edge(*e)).collect(),
                distance,
                toward: toward.map(|t| t.to_array()),
            },
        }
    }

    /// Whether the cursor is within `radius` pixels of the chain.
    fn near(&self, camera: &Camera, cursor: DVec2, radius: f64) -> bool {
        self.sides().iter().any(|(a, b)| {
            let (Some(a), Some(b)) = (camera.project(*a), camera.project(*b)) else {
                return false;
            };
            kernel::geom::segment_distance_2d(cursor, a, b) <= radius
        })
    }
}

/// An offset in progress.
#[derive(Clone, Debug)]
struct Drag {
    source: Source,
    /// Where it was pressed.
    start: DVec3,
    /// Where the drag's first click was pressed and released, on screen.
    press: DVec2,
    release: Option<DVec2>,
    /// Signed for a face (negative: outward), positive for a chain.
    distance: f64,
    /// The cursor's point, for a chain, while it lies to one side.
    toward: Option<DVec3>,
}

/// The offset a typed value redoes.
#[derive(Clone, Debug)]
struct Last {
    source: Source,
    distance: f64,
    toward: Option<DVec3>,
}

#[derive(Clone, Debug, Default)]
pub struct OffsetTool {
    drag: Option<Drag>,
    /// The face a click would offset, while no offset is in progress.
    hovered: Option<FaceId>,
    drag_start: DragStart,
    last: Option<Last>,
    /// The distance a double-click repeats.
    repeat: Option<f64>,
    /// The distance a double-click repeated before a retype, until the next event or external
    /// change: the editor refusing the retype restores it.
    before_retype: Option<Option<f64>>,
}

impl OffsetTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// An Offset tool whose double-click repeats `distance`.
    pub fn with_distance(distance: Option<f64>) -> Self {
        OffsetTool {
            repeat: distance,
            ..Self::default()
        }
    }

    /// Offsets by `distance`, if it makes a valid shape; else says why not.
    fn run(
        &mut self,
        source: &Source,
        distance: f64,
        toward: Option<DVec3>,
        out: &mut ToolOutput,
    ) -> bool {
        if distance.abs() < LEAST {
            out.message = Some("The offset distance is too small".into());
            return false;
        }
        if let Err(message) = source.lines(distance, toward) {
            out.message = Some(message);
            return false;
        }
        self.last = Some(Last {
            source: source.clone(),
            distance,
            toward,
        });
        self.repeat = Some(distance);
        out.commands.push(source.command(distance, toward));
        true
    }

    fn finish(&mut self, distance: f64, toward: Option<DVec3>, out: &mut ToolOutput) {
        let Some(d) = self.drag.clone() else { return };
        if self.run(&d.source, distance, toward, out) {
            self.drag = None;
        }
    }

    /// Redoes the last offset at the typed `text`, keeping its direction.
    fn retype(&mut self, text: &str, out: &mut ToolOutput) {
        let Some(last) = self.last.clone() else {
            out.message = Some("Click a face first".into());
            return;
        };
        match parse_length(text) {
            None => out.message = Some(format!("Not a length: {text}")),
            Some(v) if v.abs() < LEAST => {
                out.message = Some("The offset distance is too small".into());
            }
            Some(v) => {
                let distance = match last.source {
                    Source::Face { .. } if last.distance < 0.0 => -v,
                    Source::Face { .. } => v,
                    Source::Chain { .. } => v.abs(),
                };
                let mark = out.commands.len();
                out.commands.push(Command::Undo {
                    count: None,
                    expect: None,
                });
                let before = self.repeat;
                if self.run(&last.source, distance, last.toward, out) {
                    self.before_retype = Some(before);
                } else {
                    out.commands.truncate(mark);
                }
            }
        }
    }

    /// Repeats the last distance on the face of the offset in progress, else on the face under
    /// the cursor.
    fn repeat(&mut self, ctx: &ToolContext, out: &mut ToolOutput) {
        let source = match &self.drag {
            Some(Drag {
                source: source @ Source::Face { .. },
                ..
            }) => Some(source.clone()),
            _ => open_face_under(ctx).and_then(|(f, _)| Source::face(ctx.model, f)),
        };
        out.info = true;
        match (self.repeat, source) {
            (None, _) => {
                out.message =
                    Some("Offset a face first; a double-click then repeats that distance".into());
            }
            (Some(_), None) => {
                out.message = Some(
                    "Offset works on faces: double-click a face to repeat the last offset".into(),
                );
            }
            (Some(distance), Some(source)) => {
                out.info = false;
                self.drag = None;
                self.drag_start = DragStart::default();
                self.hovered = None;
                self.run(&source, distance, None, out);
                self.last = None;
            }
        }
    }

    /// What a press at the cursor would offset: the selected chain when the cursor is near it,
    /// else the face under the cursor. The point pressed is the second.
    fn pressed(ctx: &ToolContext) -> Option<(Source, DVec3)> {
        let chain = Source::chain(ctx.model, ctx.selection, ctx.camera, ctx.cursor)
            .filter(|c| c.near(ctx.camera, ctx.cursor, ctx.pick_radius));
        if let Some(chain) = chain {
            let p = chain.plane_point(ctx.camera, ctx.cursor)?;
            return Some((chain, p));
        }
        let (face, p) = open_face_under(ctx)?;
        Some((Source::face(ctx.model, face)?, p))
    }
}

/// Whether the cursor lies where a double-click's second press would: within the drag
/// threshold of the drag's first press, or twice that of its release.
fn second_press(d: &Drag, ctx: &ToolContext) -> bool {
    d.press.distance(ctx.cursor) < ctx.drag_threshold
        || d.release
            .is_some_and(|r| r.distance(ctx.cursor) < 2.0 * ctx.drag_threshold)
}

/// The open context's face under the cursor, and the point on it there.
fn open_face_under(ctx: &ToolContext) -> Option<(FaceId, DVec3)> {
    face_under_in(ctx.scenery, ctx.camera, ctx.cursor)
        .filter(|(owner, ..)| *owner == Owner::Open)
        .map(|(_, f, p)| (f, p))
}

impl Tool for OffsetTool {
    fn name(&self) -> &'static str {
        "Offset"
    }

    fn prompt(&self) -> &'static str {
        match (&self.drag, &self.last) {
            (None, None) => "Click a face to offset it, or select edges and click one",
            (None, Some(_)) => "Type a distance to change the offset, or click a face",
            (Some(_), _) => "Move to set the distance, then click or type it",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        (
            "Distance",
            self.drag.as_ref().map(|d| format_length(d.distance)),
        )
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    /// Where the offset was pressed.
    fn phase(&self) -> Phase {
        match &self.drag {
            Some(d) => Phase::Active {
                points: vec![d.start],
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        // Any event after a retype comes after the editor ran it.
        self.before_retype = None;
        match event {
            ToolEvent::Move(_) => match self.drag.as_mut() {
                Some(d) => {
                    if let Some(p) = d.source.plane_point(ctx.camera, ctx.cursor) {
                        let (distance, toward) = d.source.measure(p);
                        d.distance = distance;
                        d.toward = toward;
                    }
                }
                None => {
                    let over_chain =
                        Source::chain(ctx.model, ctx.selection, ctx.camera, ctx.cursor)
                            .is_some_and(|c| c.near(ctx.camera, ctx.cursor, ctx.pick_radius));
                    self.hovered = open_face_under(ctx).map(|(f, _)| f).filter(|_| !over_chain);
                }
            },
            ToolEvent::Click(_) => match self.drag.as_ref() {
                None => {
                    if let Some((source, start)) = Self::pressed(ctx) {
                        let (distance, toward) = source.measure(start);
                        self.drag = Some(Drag {
                            source,
                            start,
                            press: ctx.cursor,
                            release: None,
                            distance,
                            toward,
                        });
                        self.drag_start.arm(ctx.cursor);
                        self.last = None;
                    } else {
                        // Say what Offset works on (informing, not refusing).
                        out.info = true;
                        let under = face_under_in(ctx.scenery, ctx.camera, ctx.cursor);
                        out.message = Some(
                            match under {
                                Some((Owner::Instance(_), ..)) => {
                                    "Open the group to offset its faces"
                                }
                                Some((Owner::Outside, ..)) => {
                                    "Close the group to offset faces outside it"
                                }
                                _ => "Offset works on faces: click a face to offset it",
                            }
                            .into(),
                        );
                    }
                    self.hovered = None;
                }
                // A quick press a few pixels from the first's press, or within twice that of its
                // release, is a double-click's second press: it does not finish the offset.
                Some(d) if ctx.quick_press && second_press(d, ctx) => {}
                Some(d) => {
                    let (distance, toward) = (d.distance, d.toward);
                    if distance.abs() >= LEAST {
                        self.finish(distance, toward, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => match (self.drag.as_ref(), parse_length(&text)) {
                (None, _) => self.retype(&text, &mut out),
                (Some(_), None) => out.message = Some(format!("Not a length: {text}")),
                (Some(_), Some(v)) if v.abs() < LEAST => {
                    out.message = Some("The offset distance is too small".into());
                }
                (Some(d), Some(v)) => {
                    let toward = d.toward;
                    let v = match d.source {
                        // A typed distance goes the way the cursor is: negative reverses it.
                        Source::Face { .. } => Some(toward_cursor(d.distance, v)),
                        Source::Chain { .. } if v < 0.0 => {
                            out.message =
                                Some("Type a positive distance: the cursor picks the side".into());
                            None
                        }
                        Source::Chain { .. } => Some(v),
                    };
                    if let Some(v) = v {
                        self.finish(v, toward, &mut out);
                    }
                }
            },
            ToolEvent::Release(hit) => {
                if self.drag_start.released(ctx) {
                    return self.handle(ToolEvent::Click(hit), ctx);
                }
                if let Some(d) = self.drag.as_mut() {
                    d.release.get_or_insert(ctx.cursor);
                }
            }
            ToolEvent::DoubleClick(_) => self.repeat(ctx, &mut out),
            ToolEvent::TripleClick(_) | ToolEvent::ToggleCopy => {}
            ToolEvent::Cancel => self.on_external_change(),
        }
        if let Some(d) = &self.drag
            && d.distance.abs() >= LEAST
        {
            match d.source.lines(d.distance, d.toward) {
                Ok(lines) => {
                    for (points, closed) in lines {
                        let n = points.len();
                        let count = if closed { n } else { n - 1 };
                        out.preview
                            .extend((0..count).map(|i| [points[i], points[(i + 1) % n]]));
                    }
                }
                Err(message) => out.message = Some(message),
            }
        }
        out
    }

    fn on_external_change(&mut self) {
        self.before_retype = None;
        self.drag = None;
        self.hovered = None;
        self.drag_start = DragStart::default();
        self.last = None;
    }

    fn on_refused(&mut self) {
        if let Some(repeat) = self.before_retype.take() {
            self.repeat = repeat;
        }
        self.on_external_change();
    }

    fn hovered(&self) -> Option<EntityId> {
        self.hovered.map(EntityId::Face)
    }

    fn remember(&self, r: &mut Remembered) {
        r.offset_distance = self.repeat;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind, face_under};
    use crate::scenery::Scenery;

    /// A 2x2 rectangle on the ground, a camera looking at it and a selection.
    struct Rig {
        tool: OffsetTool,
        model: Model,
        face: FaceId,
        camera: Camera,
        cursor: DVec2,
        selection: BTreeSet<EntityId>,
    }

    impl Rig {
        fn new() -> Self {
            Self::with_distance(None)
        }

        fn with_distance(distance: Option<f64>) -> Self {
            let mut model = Model::new();
            let pts = [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ];
            model.add_polygon(&pts).unwrap();
            let face = model.faces().next().unwrap().0;
            Self {
                tool: OffsetTool::with_distance(distance),
                model,
                face,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
                selection: BTreeSet::new(),
            }
        }

        /// Selects the edge along y = 0.
        fn select_bottom_edge(&mut self) -> EdgeId {
            let id = self
                .model
                .edges()
                .find(|(_, e)| {
                    e.v.iter()
                        .all(|v| self.model.position(*v).unwrap().y.abs() < 1e-9)
                })
                .unwrap()
                .0;
            self.selection.insert(EntityId::Edge(id));
            id
        }

        fn at(&mut self, p: DVec3, make: fn(Inference) -> ToolEvent) -> ToolOutput {
            self.cursor = self.camera.project(p).unwrap();
            let hit = Inference {
                point: p,
                kind: SnapKind::OnFace,
                face: face_under(&self.model, &self.camera, self.cursor).map(|(f, _)| f),
            };
            self.send(make(hit))
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &Scenery::single(&self.model),
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                drag_threshold: 4.0,
                quick_press: false,
                shift: false,
                selection: &self.selection,
                material: None,
                sample: false,
            };
            self.tool.handle(event, &ctx)
        }
    }

    fn offset(face: FaceId, distance: f64) -> Command {
        Command::Offset {
            ids: vec![EntityId::Face(face)],
            distance,
            toward: None,
        }
    }

    fn undo() -> Command {
        Command::Undo {
            count: None,
            expect: None,
        }
    }

    /// The distance of the one offset command in `out`.
    fn distance_of(out: &ToolOutput) -> f64 {
        let [Command::Offset { distance, .. }] = out.commands.as_slice() else {
            panic!("expected one offset, got {:?}", out.commands);
        };
        *distance
    }

    #[test]
    fn hovering_a_face_reports_it() {
        let mut r = Rig::new();
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Move);
        assert_eq!(r.tool.hovered(), Some(EntityId::Face(r.face)));
        r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::Move);
        assert_eq!(r.tool.hovered(), None);
    }

    #[test]
    fn a_drag_inward_offsets_by_the_distance_from_the_nearest_side() {
        let mut r = Rig::new();
        r.at(DVec3::new(0.5, 1.0, 0.0), ToolEvent::Click);
        let out = r.at(DVec3::new(0.3, 1.0, 0.0), ToolEvent::Move);
        assert_eq!(out.preview.len(), 4, "the new loop");
        assert_eq!(r.tool.measurement().1.as_deref(), Some("0.3 m"));
        let out = r.at(DVec3::new(0.3, 1.0, 0.0), ToolEvent::Click);
        assert!((distance_of(&out) - 0.3).abs() < 1e-6);
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn a_drag_outward_gives_a_negative_distance() {
        let mut r = Rig::new();
        r.at(DVec3::new(0.5, 1.0, 0.0), ToolEvent::Click);
        r.at(DVec3::new(-0.3, 1.0, 0.0), ToolEvent::Move);
        let out = r.at(DVec3::new(-0.3, 1.0, 0.0), ToolEvent::Click);
        assert!((distance_of(&out) + 0.3).abs() < 1e-6);
    }

    #[test]
    fn a_press_drag_release_offsets() {
        let mut r = Rig::new();
        r.at(DVec3::new(0.5, 1.0, 0.0), ToolEvent::Click);
        r.at(DVec3::new(0.3, 1.0, 0.0), ToolEvent::Move);
        let out = r.at(DVec3::new(0.3, 1.0, 0.0), ToolEvent::Release);
        assert!((distance_of(&out) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn a_typed_distance_finishes_and_follows_the_cursor_side() {
        let mut r = Rig::new();
        r.at(DVec3::new(0.5, 1.0, 0.0), ToolEvent::Click);
        let out = r.send(ToolEvent::Value("0.25".into()));
        assert_eq!(out.commands, vec![offset(r.face, 0.25)]);
        let mut r = Rig::new();
        r.at(DVec3::new(0.5, 1.0, 0.0), ToolEvent::Click);
        r.at(DVec3::new(-0.5, 1.0, 0.0), ToolEvent::Move);
        let out = r.send(ToolEvent::Value("0.25".into()));
        assert_eq!(out.commands, vec![offset(r.face, -0.25)]);
    }

    #[test]
    fn a_typed_distance_after_an_offset_redoes_it() {
        let mut r = Rig::new();
        r.at(DVec3::new(0.5, 1.0, 0.0), ToolEvent::Click);
        r.send(ToolEvent::Value("0.25".into()));
        assert_eq!(
            r.tool.prompt(),
            "Type a distance to change the offset, or click a face"
        );
        let out = r.send(ToolEvent::Value("0.5".into()));
        assert_eq!(out.commands, vec![undo(), offset(r.face, 0.5)]);
        // Bad values change nothing; one the shape cannot take keeps the undo out too.
        let out = r.send(ToolEvent::Value("abc".into()));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Not a length: abc"));
        let out = r.send(ToolEvent::Value("5".into()));
        assert!(out.commands.is_empty(), "{:?}", out.commands);
        assert_eq!(out.message.as_deref(), Some(TOO_FAR));
        // A negative value reverses the direction.
        let out = r.send(ToolEvent::Value("-0.5".into()));
        assert_eq!(out.commands, vec![undo(), offset(r.face, -0.5)]);
        // A new offset's first click forgets the last one.
        r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        r.send(ToolEvent::Cancel);
        let out = r.send(ToolEvent::Value("2".into()));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Click a face first"));
    }

    #[test]
    fn a_double_click_repeats_the_last_distance() {
        let mut r = Rig::with_distance(Some(0.4));
        let p = DVec3::new(1.0, 1.0, 0.0);
        r.at(p, ToolEvent::Click);
        r.at(p, ToolEvent::Release);
        r.at(p, ToolEvent::Click);
        r.at(p, ToolEvent::Release);
        let out = r.at(p, ToolEvent::DoubleClick);
        assert_eq!(out.commands, vec![offset(r.face, 0.4)]);
        assert_eq!(r.tool.phase(), Phase::Idle);
        // Without a distance to repeat it explains and offsets nothing.
        let mut r = Rig::new();
        let out = r.at(p, ToolEvent::DoubleClick);
        assert!(out.commands.is_empty() && out.info);
        // Off any face it says what the tool works on.
        let mut r = Rig::with_distance(Some(0.4));
        let out = r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::DoubleClick);
        assert!(out.commands.is_empty() && out.info);
    }

    #[test]
    fn an_offset_made_by_typing_is_what_a_double_click_repeats_and_is_remembered() {
        let mut r = Rig::new();
        r.at(DVec3::new(0.5, 1.0, 0.0), ToolEvent::Click);
        r.send(ToolEvent::Value("0.25".into()));
        let mut remembered = Remembered::default();
        r.tool.remember(&mut remembered);
        assert_eq!(remembered.offset_distance, Some(0.25));
        let mut r = Rig::with_distance(remembered.offset_distance);
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::DoubleClick);
        assert_eq!(out.commands, vec![offset(r.face, 0.25)]);
    }

    #[test]
    fn an_invalid_distance_shows_the_error_and_emits_nothing() {
        let mut r = Rig::new();
        r.at(DVec3::new(0.5, 1.0, 0.0), ToolEvent::Click);
        // 1 m inward from every side of a 2 m square leaves nothing.
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Move);
        assert_eq!(out.message.as_deref(), Some(TOO_FAR));
        assert!(out.preview.is_empty());
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::Click);
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some(TOO_FAR));
        let out = r.send(ToolEvent::Value("3".into()));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some(TOO_FAR));
        // The offset is still in progress: a valid distance goes through.
        let out = r.send(ToolEvent::Value("0.5".into()));
        assert_eq!(out.commands.len(), 1);
    }

    #[test]
    fn selected_edges_are_offset_toward_the_cursor_side() {
        let mut r = Rig::new();
        let edge = r.select_bottom_edge();
        // The press on the edge, then the cursor on either side of it.
        for (y, side) in [(0.5, 1.0), (-0.5, -1.0)] {
            r.at(DVec3::new(1.0, 0.0, 0.0), ToolEvent::Click);
            let out = r.at(DVec3::new(1.0, y, 0.0), ToolEvent::Move);
            assert_eq!(out.preview.len(), 1);
            let out = r.at(DVec3::new(1.0, y, 0.0), ToolEvent::Click);
            let [
                Command::Offset {
                    ids,
                    distance,
                    toward,
                },
            ] = out.commands.as_slice()
            else {
                panic!("{:?}", out.commands);
            };
            assert_eq!(ids, &[EntityId::Edge(edge)]);
            assert!((distance - 0.5).abs() < 1e-6, "{distance}");
            let toward = DVec3::from(toward.unwrap());
            assert!(
                (toward.y - y).abs() < 1e-6 && side * toward.y > 0.0,
                "{toward}"
            );
            r.tool.on_external_change();
        }
    }

    #[test]
    fn selected_edges_are_not_used_away_from_them() {
        let mut r = Rig::new();
        r.select_bottom_edge();
        // The face under the cursor is offset instead.
        r.at(DVec3::new(1.0, 1.5, 0.0), ToolEvent::Click);
        let out = r.send(ToolEvent::Value("0.25".into()));
        assert_eq!(out.commands, vec![offset(r.face, 0.25)]);
        // Over the selected edge, no face is hovered.
        r.at(DVec3::new(1.0, 0.0, 0.0), ToolEvent::Move);
        assert_eq!(r.tool.hovered(), None);
    }

    #[test]
    fn clicking_off_any_face_says_what_offset_works_on() {
        let mut r = Rig::new();
        let out = r.at(DVec3::new(-3.0, 4.0, 0.0), ToolEvent::Click);
        assert!(out.commands.is_empty() && out.info);
        assert_eq!(r.tool.phase(), Phase::Idle);
        let out = r.send(ToolEvent::Value("1".into()));
        assert_eq!(out.message.as_deref(), Some("Click a face first"));
    }

    #[test]
    fn a_refused_retype_keeps_the_double_click_distance() {
        let mut r = Rig::new();
        r.at(DVec3::new(0.5, 1.0, 0.0), ToolEvent::Click);
        r.send(ToolEvent::Value("0.2".into()));
        r.send(ToolEvent::Value("0.3".into()));
        r.tool.on_refused();
        let out = r.at(DVec3::new(1.0, 1.0, 0.0), ToolEvent::DoubleClick);
        assert!(out.message.is_none() || !out.info);
        assert_eq!(out.commands, vec![offset(r.face, 0.2)]);
    }
}
