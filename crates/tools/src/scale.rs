//! The Scale tool: grips sit on the corners, edge middles and face middles of the selection's
//! box along the model axes. Click a grip, move, and click again (or type a factor or length)
//! to scale the selection about the opposite grip. Corner grips scale uniformly, edge and face
//! grips along their axes; holding Shift switches that. Tapping Ctrl switches scaling about the
//! box's center. With nothing selected, a click selects what the Select tool's click would.
//! Right after a scale, typing a value redoes it with that value.

use glam::DVec3;
use kernel::EntityId;
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::inference::Inference;
use crate::move_tool::outline;
use crate::rectangle::on_plane;
use crate::rotate::is_point;
use crate::select::{pick, selection_bounds, whole_curve};
use crate::tool::{DragStart, Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{ScaleValue, parse_scale};

/// How close (physical pixels) the cursor must be to a grip to take it.
const GRIP_REACH: f64 = 10.0;
/// A grip's side as a fraction of its distance from the eye.
const GRIP_SIZE: f64 = 0.012;
/// Factors smaller than this in size count as zero.
const MIN_FACTOR: f64 = 1e-4;
/// The largest factor in size (as the `scale` command allows).
const MAX_FACTOR: f64 = 1e4;

/// A grip, as the side of the box it sits on along each model axis (-1, 0 or 1).
type Sign = [i8; 3];

/// A selection's box along the model axes. Flat axes (shorter than `LENGTH_TOL`) have no
/// extent: they never scale and no grip sits on their ends.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    center: DVec3,
    half: DVec3,
    flat: [bool; 3],
}

impl Bounds {
    fn new([lo, hi]: [DVec3; 2]) -> Self {
        let flat = [0, 1, 2].map(|i| hi[i] - lo[i] < LENGTH_TOL);
        let mut half = (hi - lo) / 2.0;
        for (i, &f) in flat.iter().enumerate() {
            if f {
                half[i] = 0.0;
            }
        }
        Bounds {
            center: (lo + hi) / 2.0,
            half,
            flat,
        }
    }

    fn of(ctx: &ToolContext) -> Option<Bounds> {
        selection_bounds(ctx, ctx.selection).map(Bounds::new)
    }

    fn all_flat(&self) -> bool {
        self.flat.iter().all(|f| *f)
    }

    /// How many axes have extent.
    fn sized(&self) -> usize {
        self.flat.iter().filter(|f| !**f).count()
    }

    fn at(&self, s: Sign) -> DVec3 {
        self.center + DVec3::new(s[0].into(), s[1].into(), s[2].into()) * self.half
    }

    /// Every grip: each side vector other than zero that only leaves the middle on axes with
    /// extent.
    fn grips(&self) -> Vec<Sign> {
        let mut out = Vec::new();
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    let s: Sign = [x, y, z];
                    let on_flat = (0..3).any(|i| s[i] != 0 && self.flat[i]);
                    if s != [0; 3] && !on_flat {
                        out.push(s);
                    }
                }
            }
        }
        out
    }

    /// The box's edges (twelve for a solid box, four for a flat one, one for a line).
    fn edges(&self) -> Vec<[DVec3; 2]> {
        let flat_bits = (0..3).filter(|&i| self.flat[i]).fold(0, |m, i| m | 1 << i);
        let corner = |k: usize| {
            let side = |bit: usize| if k & 1 << bit == 0 { -1 } else { 1 };
            self.at([side(0), side(1), side(2)])
        };
        let mut out = Vec::new();
        for k in (0..8).filter(|k| k & flat_bits == 0) {
            for axis in (0..3).filter(|&i| !self.flat[i] && k & 1 << i == 0) {
                out.push([corner(k), corner(k | 1 << axis)]);
            }
        }
        out
    }

    /// The grip under the cursor: the one whose projection is nearest it, within reach.
    fn grip_under(&self, ctx: &ToolContext) -> Option<Sign> {
        self.grips()
            .into_iter()
            .filter_map(|s| {
                let d = ctx.camera.project(self.at(s))?.distance(ctx.cursor);
                (d <= GRIP_REACH).then_some((s, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(s, _)| s)
    }
}

fn active(s: Sign) -> [bool; 3] {
    s.map(|v| v != 0)
}

fn count(axes: [bool; 3]) -> usize {
    axes.iter().filter(|a| **a).count()
}

fn opposite(s: Sign) -> Sign {
    s.map(|v| -v)
}

/// A scale in progress.
#[derive(Clone, Debug)]
struct Stretch {
    ids: Vec<EntityId>,
    bounds: Bounds,
    grip: Sign,
    /// What scales, drawn at its original position.
    outline: Vec<[DVec3; 2]>,
    /// The cursor point the grip follows.
    point: DVec3,
}

impl Stretch {
    fn grip_point(&self) -> DVec3 {
        self.bounds.at(self.grip)
    }

    /// From the opposite grip to the grip: the size a typed length replaces.
    fn span(&self) -> DVec3 {
        self.grip_point() - self.bounds.at(opposite(self.grip))
    }

    /// The point that stays put: the box center, or the grip opposite the one dragged.
    fn fixed(&self, about_center: bool) -> DVec3 {
        if about_center {
            self.bounds.center
        } else {
            self.bounds.at(opposite(self.grip))
        }
    }

    /// Whether every sized axis scales by one factor: a corner grip's default, which Shift
    /// switches.
    fn uniform(&self, shift: bool) -> bool {
        (count(active(self.grip)) == self.bounds.sized()) != shift
    }

    /// The axes that scale.
    fn moving(&self, shift: bool) -> [bool; 3] {
        if self.uniform(shift) {
            self.bounds.flat.map(|f| !f)
        } else {
            active(self.grip)
        }
    }

    /// Where the cursor puts the grip: a point it snapped to, else the point nearest the
    /// cursor ray on the line from the fixed point through the grip, or for a two-axis grip
    /// scaling non-uniformly, where the ray crosses the plane of those axes through the grip.
    fn place(&mut self, hit: &Inference, ctx: &ToolContext, about_center: bool, shift: bool) {
        if is_point(hit.kind) {
            self.point = hit.point;
            return;
        }
        let fixed = self.fixed(about_center);
        let grip = self.grip_point();
        let axes = active(self.grip);
        let plane = (!self.uniform(shift) && count(axes) == 2)
            .then(|| axes.iter().position(|a| !a))
            .flatten()
            .and_then(|i| on_plane(grip, DVec3::AXES[i], hit, ctx));
        self.point = plane.unwrap_or_else(|| nearest_on_line(fixed, grip - fixed, hit, ctx));
    }

    /// The factors along x, y and z the cursor point gives.
    fn factors(&self, about_center: bool, shift: bool) -> [f64; 3] {
        let fixed = self.fixed(about_center);
        let d = self.grip_point() - fixed;
        let p = self.point - fixed;
        let moving = self.moving(shift);
        if self.uniform(shift) {
            let f = p.dot(d) / d.dot(d);
            moving.map(|m| if m { f } else { 1.0 })
        } else {
            [0, 1, 2].map(|i| if moving[i] { p[i] / d[i] } else { 1.0 })
        }
    }
}

/// The point on the line through `origin` along `dir` nearest the cursor ray (the inferred
/// point dropped onto the line when the ray runs along it, or when the nearest point of the
/// ray's line lies behind where the ray starts).
fn nearest_on_line(origin: DVec3, dir: DVec3, hit: &Inference, ctx: &ToolContext) -> DVec3 {
    let u = dir.normalize();
    let ray = ctx.camera.ray(ctx.cursor);
    let r = ray.dir.normalize();
    let b = u.dot(r);
    let denom = 1.0 - b * b;
    if denom < 1e-9 {
        return origin + u * (hit.point - origin).dot(u);
    }
    let w = origin - ray.origin;
    let (d, e) = (u.dot(w), r.dot(w));
    if (e - b * d) / denom <= 0.0 {
        return origin + u * (hit.point - origin).dot(u);
    }
    origin + u * ((b * e - d) / denom)
}

/// The last scale made, which a typed value replaces.
#[derive(Clone, Debug)]
struct Last {
    ids: Vec<EntityId>,
    origin: DVec3,
    moving: [bool; 3],
    uniform: bool,
    /// From the opposite grip to the grip, for typed lengths.
    span: DVec3,
    /// The box's size along each axis, for typed lengths.
    size: DVec3,
    factors: [f64; 3],
}

#[derive(Clone, Debug, Default)]
pub struct ScaleTool {
    stretch: Option<Stretch>,
    drag: DragStart,
    /// Scale about the box's center instead of the opposite grip.
    about_center: bool,
    /// Whether Shift was held at the last event.
    shift: bool,
    /// The grip under the cursor while idle.
    hovered: Option<Sign>,
    /// Whether something was selected at the last event, for the idle prompt.
    selected: bool,
    last: Option<Last>,
}

/// Factors from typed `values` for the `moving` axes: one value for all of them, or one each
/// in x, y, z order. A length is a new size, whatever the fixed point: one value scaling
/// `uniform`ly is the size along the grip (divided by the length of `span`, from the opposite
/// grip to the grip: the diagonal for a corner grip); otherwise each is the size along its
/// axis (divided by the box's `size` there). It takes the sign of the `current` factor.
fn typed_factors(
    values: &[ScaleValue],
    moving: [bool; 3],
    uniform: bool,
    span: DVec3,
    size: DVec3,
    current: [f64; 3],
) -> Result<[f64; 3], String> {
    let axes: Vec<usize> = (0..3).filter(|&i| moving[i]).collect();
    let signed = |i: usize, f: f64| if current[i] < 0.0 { -f } else { f };
    let mut factors = [1.0; 3];
    match values {
        [one] => {
            for &i in &axes {
                factors[i] = match *one {
                    ScaleValue::Factor(f) => f,
                    ScaleValue::Length(l) if uniform => signed(i, l / span.length()),
                    ScaleValue::Length(l) => signed(i, l / size[i]),
                };
            }
        }
        _ if values.len() == axes.len() => {
            for (&i, v) in axes.iter().zip(values) {
                factors[i] = match *v {
                    ScaleValue::Factor(f) => f,
                    ScaleValue::Length(l) => signed(i, l / size[i]),
                };
            }
        }
        _ if axes.len() == 1 => return Err("Type 1 value".into()),
        _ => return Err(format!("Type 1 or {} values", axes.len())),
    }
    Ok(factors)
}

/// Why `factors` cannot scale, if they cannot.
fn refusal(factors: [f64; 3], moving: [bool; 3]) -> Option<&'static str> {
    if (0..3).any(|i| moving[i] && (factors[i].is_nan() || factors[i].abs() < MIN_FACTOR)) {
        Some("Scale must not be zero")
    } else if factors
        .iter()
        .any(|f| !f.is_finite() || f.abs() > MAX_FACTOR)
    {
        Some("Scale must be at most 10000 in size")
    } else if factors == [1.0; 3] {
        Some("A scale of 1 changes nothing")
    } else {
        None
    }
}

/// A factor for display, to a thousandth: "1.5".
fn format_factor(f: f64) -> String {
    let s = format!("{f:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// `p` scaled by `factors` about `origin`.
fn scaled(p: DVec3, origin: DVec3, factors: [f64; 3]) -> DVec3 {
    origin + (p - origin) * DVec3::from(factors)
}

/// A camera-facing square of side `side` around `at`.
fn square(ctx: &ToolContext, at: DVec3, side: f64) -> [[DVec3; 2]; 4] {
    let (r, u) = (
        ctx.camera.right() * side / 2.0,
        ctx.camera.up() * side / 2.0,
    );
    let c = [at - r - u, at + r - u, at + r + u, at - r + u];
    [[c[0], c[1]], [c[1], c[2]], [c[2], c[3]], [c[3], c[0]]]
}

impl ScaleTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scales what the stretch holds by `factors`, unless they cannot scale.
    fn finish(&mut self, factors: [f64; 3], out: &mut ToolOutput) {
        let Some(s) = &self.stretch else { return };
        let moving = s.moving(self.shift);
        let uniform = s.uniform(self.shift);
        if let Some(why) = refusal(factors, moving) {
            out.message = Some(why.into());
            return;
        }
        let Some(s) = self.stretch.take() else { return };
        let origin = s.fixed(self.about_center);
        out.commands.push(Command::Scale {
            ids: s.ids.clone(),
            origin: origin.to_array(),
            factors,
        });
        self.last = Some(Last {
            span: s.span(),
            size: s.bounds.half * 2.0,
            uniform,
            ids: s.ids,
            origin,
            moving,
            factors,
        });
    }

    /// Replaces the last scale with one by the typed `values`, along the same axes.
    fn redo_with(&mut self, values: &[ScaleValue], out: &mut ToolOutput) {
        let Some(last) = self.last.as_mut() else {
            out.message = Some("Click a grip first".into());
            return;
        };
        let factors = match typed_factors(
            values,
            last.moving,
            last.uniform,
            last.span,
            last.size,
            last.factors,
        ) {
            Ok(f) => f,
            Err(why) => {
                out.message = Some(why);
                return;
            }
        };
        if let Some(why) = refusal(factors, last.moving) {
            out.message = Some(why.into());
            return;
        }
        last.factors = factors;
        out.commands.push(Command::Undo {
            count: None,
            expect: None,
        });
        out.commands.push(Command::Scale {
            ids: last.ids.clone(),
            origin: last.origin.to_array(),
            factors,
        });
    }

    /// The first click: on a grip it starts scaling; with nothing selected it selects.
    fn start(&mut self, ctx: &ToolContext, out: &mut ToolOutput) {
        if ctx.selection.is_empty() {
            match pick(ctx) {
                Some(id) => out.commands.push(Command::Select {
                    ids: whole_curve(ctx.model, id),
                    mode: session::SelectMode::Replace,
                    all: false,
                }),
                None => {
                    out.message = Some("Select geometry, or click an edge, face or group".into())
                }
            }
            return;
        }
        let Some(bounds) = Bounds::of(ctx).filter(|b| !b.all_flat()) else {
            out.message = Some("Nothing to scale: select something with size".into());
            return;
        };
        let Some(grip) = bounds.grip_under(ctx) else {
            out.message = Some("Click a grip to scale the selection".into());
            return;
        };
        let ids: Vec<EntityId> = ctx.selection.iter().copied().collect();
        self.stretch = Some(Stretch {
            outline: outline(ctx, &ids),
            ids,
            bounds,
            grip,
            point: bounds.at(grip),
        });
        self.hovered = None;
        self.drag.arm(ctx.cursor);
    }

    /// Moves the grip to the cursor and finishes there.
    fn finish_at(&mut self, hit: &Inference, ctx: &ToolContext, out: &mut ToolOutput) {
        let (about_center, shift) = (self.about_center, self.shift);
        let Some(s) = self.stretch.as_mut() else {
            return;
        };
        s.place(hit, ctx, about_center, shift);
        let factors = s.factors(about_center, shift);
        if factors == [1.0; 3] {
            out.message = Some("Move the grip to scale".into());
        } else {
            self.finish(factors, out);
        }
    }

    fn preview(&self, ctx: &ToolContext) -> Vec<[DVec3; 2]> {
        if let Some(s) = &self.stretch {
            let origin = s.fixed(self.about_center);
            let factors = s.factors(self.about_center, self.shift);
            return s
                .bounds
                .edges()
                .iter()
                .chain(&s.outline)
                .map(|[a, b]| [scaled(*a, origin, factors), scaled(*b, origin, factors)])
                .collect();
        }
        if ctx.selection.is_empty() {
            return Vec::new();
        }
        let Some(bounds) = Bounds::of(ctx).filter(|b| !b.all_flat()) else {
            return Vec::new();
        };
        let mut out = bounds.edges();
        for s in bounds.grips() {
            let at = bounds.at(s);
            let grow = if self.hovered == Some(s) { 1.5 } else { 1.0 };
            let side = GRIP_SIZE * ctx.camera.eye_distance(at) * grow;
            out.extend(square(ctx, at, side));
        }
        out
    }
}

impl Tool for ScaleTool {
    fn name(&self) -> &'static str {
        "Scale"
    }

    fn prompt(&self) -> &'static str {
        match (&self.stretch, self.about_center) {
            (Some(_), false) => "Click to scale, or type a factor or length; Shift: uniform or not",
            (Some(_), true) => {
                "About the center: click to scale, or type a factor or length; Shift: uniform or not"
            }
            (None, _) if self.last.is_some() => "Type a scale to change it, or click a grip",
            (None, false) if self.selected => {
                "Click a grip to scale the selection; Ctrl scales about the center"
            }
            (None, true) if self.selected => {
                "About the center: click a grip to scale the selection"
            }
            (None, _) => "Click an edge, face or group to select what to scale",
        }
    }

    /// The factors of the scale in progress: one when uniform, else those of the moving axes.
    fn measurement(&self, _units: &session::Units) -> (&'static str, Option<String>) {
        let value = self.stretch.as_ref().map(|s| {
            let factors = s.factors(self.about_center, self.shift);
            let moving = s.moving(self.shift);
            let shown: Vec<String> = (0..3)
                .filter(|&i| moving[i])
                .map(|i| format_factor(factors[i]))
                .collect();
            if s.uniform(self.shift) {
                shown[0].clone()
            } else {
                shown.join(", ")
            }
        });
        ("Scale", value)
    }

    /// Shift switches uniform scaling; it locks no direction.
    fn anchor(&self) -> Option<DVec3> {
        None
    }

    /// The fixed point, then the grip.
    fn phase(&self) -> Phase {
        match &self.stretch {
            Some(s) => Phase::Active {
                points: vec![s.fixed(self.about_center), s.grip_point()],
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        self.selected = !ctx.selection.is_empty();
        self.shift = ctx.shift;
        match event {
            ToolEvent::Move(hit) => {
                let (about_center, shift) = (self.about_center, self.shift);
                match self.stretch.as_mut() {
                    Some(s) => s.place(&hit, ctx, about_center, shift),
                    None => {
                        self.hovered = Bounds::of(ctx)
                            .filter(|b| self.selected && !b.all_flat())
                            .and_then(|b| b.grip_under(ctx));
                    }
                }
            }
            ToolEvent::Click(hit) => {
                if self.stretch.is_some() {
                    self.finish_at(&hit, ctx, &mut out);
                } else {
                    self.start(ctx, &mut out);
                }
            }
            ToolEvent::Release(hit) => {
                if self.drag.released(ctx) && self.stretch.is_some() {
                    self.finish_at(&hit, ctx, &mut out);
                }
            }
            ToolEvent::Value(text) => match parse_scale(&text, &ctx.units) {
                None => out.message = Some(format!("Not a scale factor or length: {text}")),
                Some(values) => {
                    let typed = self.stretch.as_ref().map(|s| {
                        let current = s.factors(self.about_center, self.shift);
                        let moving = s.moving(self.shift);
                        let uniform = s.uniform(self.shift);
                        typed_factors(
                            &values,
                            moving,
                            uniform,
                            s.span(),
                            s.bounds.half * 2.0,
                            current,
                        )
                    });
                    match typed {
                        Some(Ok(factors)) => self.finish(factors, &mut out),
                        Some(Err(why)) => out.message = Some(why),
                        None => self.redo_with(&values, &mut out),
                    }
                }
            },
            ToolEvent::ToggleCopy => {
                self.about_center = !self.about_center;
                out.message = Some(
                    if self.about_center {
                        "Scaling about the center"
                    } else {
                        "Scaling about the opposite grip"
                    }
                    .into(),
                );
                out.info = true;
            }
            ToolEvent::Cancel => {
                self.stretch = None;
                self.drag = DragStart::default();
            }
            ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) => {}
        }
        out.preview = self.preview(ctx);
        out
    }

    fn on_external_change(&mut self) {
        self.stretch = None;
        self.drag = DragStart::default();
        self.hovered = None;
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::SnapKind;
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::Model;
    use session::{Camera, SelectMode};
    use std::collections::BTreeSet;

    /// A model, its selection and a camera looking at it.
    struct Rig {
        tool: ScaleTool,
        model: Model,
        camera: Camera,
        selection: BTreeSet<EntityId>,
        cursor: DVec2,
        shift: bool,
    }

    /// Every edge and face of `model`.
    fn everything(model: &Model) -> BTreeSet<EntityId> {
        let edges = model.edges().map(|(e, _)| EntityId::Edge(e));
        edges
            .chain(model.faces().map(|(f, _)| EntityId::Face(f)))
            .collect()
    }

    impl Rig {
        /// A selected 2 m box at the origin.
        fn cube() -> Self {
            let mut model = Model::new();
            let pts = [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ];
            model.add_polygon(&pts).unwrap();
            let base = model.faces().next().unwrap().0;
            let up = model.face(base).unwrap().normal.z.signum();
            model.push_pull(base, 2.0 * up).unwrap();
            Self::of(model, true)
        }

        /// A 2 m square on the ground at the origin.
        fn square(selected: bool) -> Self {
            let mut model = Model::new();
            let pts = [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ];
            model.add_polygon(&pts).unwrap();
            Self::of(model, selected)
        }

        fn of(model: Model, selected: bool) -> Self {
            let selection = if selected {
                everything(&model)
            } else {
                BTreeSet::new()
            };
            Self {
                tool: ScaleTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                selection,
                cursor: DVec2::ZERO,
                shift: false,
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
                shift: self.shift,
                selection: &self.selection,
                material: None,
                sample: false,
                labels: &[],
            };
            self.tool.handle(event, &ctx)
        }

        fn hit(&mut self, p: DVec3) -> Inference {
            self.cursor = self.camera.project(p).unwrap();
            Inference {
                point: p,
                kind: SnapKind::Free,
                face: None,
                locked: None,
            }
        }

        fn click(&mut self, p: DVec3) -> ToolOutput {
            let hit = self.hit(p);
            let out = self.send(ToolEvent::Click(hit));
            self.send(ToolEvent::Release(hit));
            out
        }

        fn hover(&mut self, p: DVec3) -> ToolOutput {
            let hit = self.hit(p);
            self.send(ToolEvent::Move(hit))
        }

        fn type_(&mut self, text: &str) -> ToolOutput {
            self.send(ToolEvent::Value(text.into()))
        }

        /// Clicks the grip at `grip`, moves to `to` and clicks there.
        fn drag(&mut self, grip: DVec3, to: DVec3) -> ToolOutput {
            self.click(grip);
            self.hover(to);
            self.click(to)
        }
    }

    fn v(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    /// The origin and factors of the only command in `out`, a scale.
    fn scale_of(out: &ToolOutput) -> (DVec3, DVec3) {
        match &out.commands[..] {
            [
                Command::Scale {
                    origin, factors, ..
                },
            ] => (DVec3::from(*origin), DVec3::from(*factors)),
            other => panic!("not one scale: {other:?} ({:?})", out.message),
        }
    }

    fn close(a: DVec3, b: DVec3) -> bool {
        a.distance(b) < 1e-6
    }

    #[test]
    fn grips_cover_a_box() {
        let mut r = Rig::cube();
        let scenery = Scenery::single(&r.model);
        let ctx = ToolContext {
            model: &r.model,
            scenery: &scenery,
            camera: &r.camera,
            cursor: DVec2::ZERO,
            pick_radius: 10.0,
            drag_threshold: 4.0,
            scale_factor: 1.0,
            to_world: glam::DAffine3::IDENTITY,
            quick_press: false,
            units: session::Units::default(),
            shift: false,
            selection: &r.selection,
            material: None,
            sample: false,
            labels: &[],
        };
        let b = Bounds::of(&ctx).unwrap();
        assert!(close(b.center, v(1.0, 1.0, 1.0)) && close(b.half, v(1.0, 1.0, 1.0)));
        assert_eq!(b.grips().len(), 26);
        assert_eq!(b.edges().len(), 12);
        // Idle, the box and a square per grip.
        let out = r.hover(v(5.0, 5.0, 0.0));
        assert_eq!(out.preview.len(), 12 + 26 * 4);

        let mut r = Rig::square(true);
        let scenery = Scenery::single(&r.model);
        let ctx = ToolContext {
            model: &r.model,
            scenery: &scenery,
            camera: &r.camera,
            cursor: DVec2::ZERO,
            pick_radius: 10.0,
            drag_threshold: 4.0,
            scale_factor: 1.0,
            to_world: glam::DAffine3::IDENTITY,
            quick_press: false,
            units: session::Units::default(),
            shift: false,
            selection: &r.selection,
            material: None,
            sample: false,
            labels: &[],
        };
        let b = Bounds::of(&ctx).unwrap();
        let grips = b.grips();
        assert_eq!(grips.len(), 8, "4 corners and 4 edge middles");
        assert!(grips.iter().all(|s| s[2] == 0));
        assert_eq!(b.edges().len(), 4);
        // Hovering a grip draws it larger.
        let out = r.hover(v(2.0, 2.0, 0.0));
        assert_eq!(out.preview.len(), 4 + 8 * 4);
        assert_eq!(r.tool.hovered, Some([1, 1, 0]));
    }

    #[test]
    fn corner_grip_scales_uniformly_about_the_opposite_corner() {
        let mut r = Rig::cube();
        r.click(v(2.0, 2.0, 2.0));
        assert_eq!(
            r.tool.phase(),
            Phase::Active {
                points: vec![DVec3::ZERO, v(2.0, 2.0, 2.0)]
            }
        );
        r.hover(v(3.0, 3.0, 3.0));
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Scale", Some("1.5".into()))
        );
        let out = r.click(v(3.0, 3.0, 3.0));
        let (origin, factors) = scale_of(&out);
        assert!(close(origin, DVec3::ZERO), "{origin}");
        assert!(close(factors, DVec3::splat(1.5)), "{factors}");
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn face_grip_scales_one_axis_and_shift_makes_it_uniform() {
        let mut r = Rig::cube();
        let (origin, factors) = scale_of(&r.drag(v(2.0, 1.0, 1.0), v(4.0, 1.0, 1.0)));
        assert!(close(origin, v(0.0, 1.0, 1.0)), "{origin}");
        assert!(close(factors, v(2.0, 1.0, 1.0)), "{factors}");

        let mut r = Rig::cube();
        r.shift = true;
        let (_, factors) = scale_of(&r.drag(v(2.0, 1.0, 1.0), v(4.0, 1.0, 1.0)));
        assert!(close(factors, v(2.0, 2.0, 2.0)), "{factors}");
    }

    #[test]
    fn an_edge_grip_follows_the_plane_of_its_axes() {
        let mut r = Rig::cube();
        r.click(v(2.0, 2.0, 1.0));
        r.hover(v(3.0, 4.0, 1.0));
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Scale", Some("1.5, 2".into()))
        );
        let (origin, factors) = scale_of(&r.click(v(3.0, 4.0, 1.0)));
        assert!(close(origin, v(0.0, 0.0, 1.0)), "{origin}");
        assert!(close(factors, v(1.5, 2.0, 1.0)), "{factors}");
    }

    #[test]
    fn ctrl_scales_about_the_center() {
        let mut r = Rig::cube();
        let out = r.send(ToolEvent::ToggleCopy);
        assert_eq!(out.message.as_deref(), Some("Scaling about the center"));
        let (origin, factors) = scale_of(&r.drag(v(2.0, 2.0, 2.0), v(3.0, 3.0, 3.0)));
        assert!(close(origin, v(1.0, 1.0, 1.0)), "{origin}");
        assert!(close(factors, DVec3::splat(2.0)), "{factors}");
        // Also while scaling.
        r.click(v(2.0, 2.0, 2.0));
        let out = r.send(ToolEvent::ToggleCopy);
        assert_eq!(
            out.message.as_deref(),
            Some("Scaling about the opposite grip")
        );
        let (origin, _) = scale_of(&r.type_("2"));
        assert!(close(origin, DVec3::ZERO), "{origin}");
    }

    #[test]
    fn typed_values() {
        // A corner grip: one factor for every axis.
        let mut r = Rig::cube();
        r.click(v(2.0, 2.0, 2.0));
        let (_, factors) = scale_of(&r.type_("2"));
        assert!(close(factors, DVec3::splat(2.0)), "{factors}");
        // A face grip: a length along its axis.
        let mut r = Rig::cube();
        r.click(v(2.0, 1.0, 1.0));
        let (_, factors) = scale_of(&r.type_("4m"));
        assert!(close(factors, v(2.0, 1.0, 1.0)), "{factors}");
        // An edge grip takes one or two values.
        let mut r = Rig::cube();
        r.click(v(2.0, 2.0, 1.0));
        let out = r.type_("1,2,3");
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Type 1 or 2 values"));
        let (_, factors) = scale_of(&r.type_("1m, 3m"));
        assert!(close(factors, v(0.5, 1.5, 1.0)), "{factors}");
        // A length follows the cursor's side of the fixed point.
        let mut r = Rig::cube();
        r.click(v(2.0, 1.0, 1.0));
        r.hover(v(-2.0, 1.0, 1.0));
        let (_, factors) = scale_of(&r.type_("4m"));
        assert!(close(factors, v(-2.0, 1.0, 1.0)), "{factors}");
        assert!(r.type_("nope").message.is_some());
    }

    #[test]
    fn a_typed_length_is_the_new_size_along_the_grip() {
        // A face grip: 4 m wide, whether the opposite face or the center stays put.
        for about_center in [false, true] {
            let mut r = Rig::cube();
            if about_center {
                r.send(ToolEvent::ToggleCopy);
            }
            r.click(v(2.0, 1.0, 1.0));
            let (origin, factors) = scale_of(&r.type_("4m"));
            let fixed = if about_center { 1.0 } else { 0.0 };
            assert!(close(origin, v(fixed, 1.0, 1.0)), "{origin}");
            assert!(
                close(factors, v(2.0, 1.0, 1.0)),
                "{about_center}: {factors}"
            );
        }
        // A corner grip: the new diagonal, here twice the old one.
        let mut r = Rig::cube();
        r.send(ToolEvent::ToggleCopy);
        r.click(v(2.0, 2.0, 2.0));
        let diagonal = 2.0 * 2.0 * 3f64.sqrt();
        let (_, factors) = scale_of(&r.type_(&format!("{diagonal}m")));
        assert!(close(factors, DVec3::splat(2.0)), "{factors}");
        // A retyped length after a scale about the center is a size too.
        let (_, factors) = {
            let out = r.type_("4m");
            match &out.commands[..] {
                [Command::Undo { .. }, Command::Scale { factors, .. }] => {
                    ((), DVec3::from(*factors))
                }
                other => panic!("{other:?}"),
            }
        };
        let f = 4.0 / (2.0 * 3f64.sqrt());
        assert!(close(factors, DVec3::splat(f)), "{factors}");
    }

    #[test]
    fn shift_takes_one_length_per_axis_on_a_face_grip() {
        let mut r = Rig::cube();
        r.shift = true;
        r.click(v(2.0, 1.0, 1.0));
        let (_, factors) = scale_of(&r.type_("1m,1m,1m"));
        assert!(close(factors, DVec3::splat(0.5)), "{factors}");
    }

    #[test]
    fn factors_beyond_the_limits_are_refused() {
        let mut r = Rig::cube();
        r.click(v(2.0, 2.0, 2.0));
        let out = r.type_("20000");
        assert!(out.commands.is_empty());
        assert_eq!(
            out.message.as_deref(),
            Some("Scale must be at most 10000 in size")
        );
        assert!(
            matches!(r.tool.phase(), Phase::Active { .. }),
            "still scaling"
        );
        let out = r.type_("0.00001");
        assert_eq!(out.message.as_deref(), Some("Scale must not be zero"));
        let (_, factors) = scale_of(&r.type_("10000"));
        assert!(close(factors, DVec3::splat(1e4)), "{factors}");
    }

    #[test]
    fn a_typed_value_after_a_scale_redoes_it() {
        let mut r = Rig::cube();
        r.drag(v(2.0, 1.0, 1.0), v(3.0, 1.0, 1.0));
        let out = r.type_("3");
        assert_eq!(
            out.commands,
            vec![
                Command::Undo {
                    count: None,
                    expect: None
                },
                Command::Scale {
                    ids: r.selection.iter().copied().collect(),
                    origin: [0.0, 1.0, 1.0],
                    factors: [3.0, 1.0, 1.0],
                }
            ]
        );
        assert_eq!(r.type_("1,2").message.as_deref(), Some("Type 1 value"));
        r.tool.on_external_change();
        let out = r.type_("2");
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Click a grip first"));
    }

    #[test]
    fn zero_is_refused() {
        let mut r = Rig::cube();
        let out = r.drag(v(2.0, 1.0, 1.0), v(0.0, 1.0, 1.0));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Scale must not be zero"));
        assert!(
            matches!(r.tool.phase(), Phase::Active { .. }),
            "still scaling"
        );
        assert_eq!(
            r.type_("0").message.as_deref(),
            Some("Scale must not be zero")
        );
        r.send(ToolEvent::Cancel);
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn nothing_selected_click_selects() {
        let mut r = Rig::square(false);
        let out = r.click(v(1.0, 1.0, 0.0));
        let [Command::Select { ids, mode, all }] = &out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert_eq!((ids.len(), *mode, *all), (1, SelectMode::Replace, false));
        assert!(matches!(ids[0], EntityId::Face(_)));
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn flat_selections_and_clicks_off_the_grips_say_why() {
        let mut r = Rig::square(true);
        let out = r.click(v(1.0, 1.0, 0.0));
        assert_eq!(
            out.message.as_deref(),
            Some("Click a grip to scale the selection")
        );
        // One vertex has no size.
        let corner = r.model.vertices().next().unwrap().0;
        r.selection = BTreeSet::from([EntityId::Vertex(corner)]);
        let out = r.click(v(0.0, 0.0, 0.0));
        assert_eq!(
            out.message.as_deref(),
            Some("Nothing to scale: select something with size")
        );
        assert!(out.preview.is_empty());
        // One edge along an axis: grips on its ends only.
        let edge = r.model.edges().next().unwrap().0;
        r.selection = BTreeSet::from([EntityId::Edge(edge)]);
        let out = r.hover(v(5.0, 5.0, 0.0));
        assert_eq!(out.preview.len(), 1 + 2 * 4);
    }
}
