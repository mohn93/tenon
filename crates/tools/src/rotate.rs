//! The Rotate tool: click the center of rotation, click a point the angle starts from, then
//! click where it ends (or type the angle in degrees). The axis is the normal of the face under
//! the center click, or vertical elsewhere. The selection turns (or else the edge, face or
//! group under the center click). Off points the angle snaps to 15 degree steps. Tapping Ctrl
//! switches to copying. Right after a rotation, typing an angle redoes it with that angle, and
//! after a copy "5x" makes 5 copies at that angle and "/5" puts 5 copies evenly up to it.

use glam::DVec3;
use kernel::EntityId;
use kernel::geom::LENGTH_TOL;
use session::{Command, MAX_COPIES, rotation};

use crate::inference::{Inference, SnapKind};
use crate::move_tool::outline;
use crate::rectangle::{on_plane, plane_normal};
use crate::select::{pick, whole_curve};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{Array, format_angle, parse_angle, parse_array};

/// Angles within this many degrees of a multiple of `STEP` snap to it.
const SNAP_DEG: f64 = 2.0;
/// The protractor's steps, in degrees.
const STEP: f64 = 15.0;
/// The protractor's radius as a fraction of its distance from the eye.
const PROTRACTOR: f64 = 0.08;

/// A rotation in progress.
#[derive(Clone, Debug)]
struct Turn {
    center: DVec3,
    /// Unit vector the rotation turns about (counter-clockwise looking down it).
    axis: DVec3,
    ids: Vec<EntityId>,
    /// What turns, drawn at its original position.
    outline: Vec<[DVec3; 2]>,
    /// Unit vector in the rotation's plane the angle is measured from, once clicked.
    reference: Option<DVec3>,
}

/// The last rotation made, which a typed angle or array replaces.
#[derive(Clone, Debug)]
struct Last {
    ids: Vec<EntityId>,
    center: DVec3,
    axis: DVec3,
    angle: f64,
    copy: bool,
    /// How many copies the last operation made (1 unless an array replaced it).
    count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct RotateTool {
    turn: Option<Turn>,
    /// The cursor on the rotation's plane.
    hover: Option<DVec3>,
    /// Whether the cursor is on a point (an endpoint, a midpoint...), which the angle follows
    /// exactly instead of snapping to steps.
    on_point: bool,
    copy: bool,
    last: Option<Last>,
}

impl RotateTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The angle from the reference to the cursor, in degrees (-180 to 180), snapped to steps
    /// unless the cursor is on a point.
    fn angle(&self) -> Option<f64> {
        let turn = self.turn.as_ref()?;
        let from = turn.reference?;
        let to = flat(self.hover? - turn.center, turn.axis).try_normalize()?;
        let angle = from
            .cross(to)
            .dot(turn.axis)
            .atan2(from.dot(to))
            .to_degrees();
        let step = (angle / STEP).round() * STEP;
        Some(if !self.on_point && (angle - step).abs() < SNAP_DEG {
            step
        } else {
            angle
        })
    }

    /// Turns what the rotation in progress holds by `angle` degrees (or copies it).
    fn finish(&mut self, angle: f64, out: &mut ToolOutput) {
        let Some(turn) = self.turn.take() else { return };
        self.hover = None;
        let last = Last {
            ids: turn.ids,
            center: turn.center,
            axis: turn.axis,
            angle,
            copy: self.copy,
            count: 1,
        };
        out.commands.push(command(&last, angle, None));
        self.last = Some(last);
    }

    /// Replaces the last rotation with the same one by `angle`, turning the way it did and
    /// making as many copies.
    fn redo_with(&mut self, angle: f64, out: &mut ToolOutput) {
        let Some(last) = self.last.as_mut() else {
            out.message = Some("Click the center of rotation first".into());
            return;
        };
        let angle = if last.angle < 0.0 { -angle } else { angle };
        last.angle = angle;
        out.commands.push(Command::Undo);
        out.commands.push(command(last, angle, Some(last.count)));
    }

    /// Replaces the last copy with an array of copies.
    fn array(&mut self, array: Array, out: &mut ToolOutput) {
        let Some(last) = self.last.as_ref().filter(|l| l.copy) else {
            out.message = Some("Make a rotated copy first, then type 5x or /5".into());
            return;
        };
        let (step, count) = match array {
            Array::Times(n) => (last.angle, n),
            Array::Divide(n) => (last.angle / n as f64, n),
        };
        if count > MAX_COPIES {
            out.message = Some(format!("At most {MAX_COPIES} copies"));
            return;
        }
        out.commands.push(Command::Undo);
        out.commands.push(command(last, step, Some(count)));
        if let Some(last) = self.last.as_mut() {
            last.count = count;
        }
    }
}

/// The command turning (or copying, `count` times) what `last` turned, by `angle`.
fn command(last: &Last, angle: f64, count: Option<u32>) -> Command {
    Command::Rotate {
        ids: last.ids.clone(),
        center: last.center.to_array(),
        axis: Some(last.axis.to_array()),
        angle,
        copies: if last.copy { count.or(Some(1)) } else { None },
    }
}

/// `v` without its component along the unit vector `axis`.
fn flat(v: DVec3, axis: DVec3) -> DVec3 {
    v - axis * v.dot(axis)
}

/// True for snaps to a point of the model, which the angle follows exactly.
fn is_point(kind: SnapKind) -> bool {
    matches!(
        kind,
        SnapKind::Endpoint
            | SnapKind::Midpoint
            | SnapKind::Origin
            | SnapKind::Intersection
            | SnapKind::OnEdge
    )
}

/// Where the cursor is on the plane through `center` square to `axis`: a point it snapped to,
/// dropped onto the plane, else where its ray crosses the plane.
fn on_rotation_plane(
    center: DVec3,
    axis: DVec3,
    hit: &Inference,
    ctx: &ToolContext,
) -> Option<DVec3> {
    if is_point(hit.kind) {
        return Some(center + flat(hit.point - center, axis));
    }
    on_plane(center, axis, hit, ctx).or_else(|| Some(center + flat(hit.point - center, axis)))
}

/// A circle of `n` segments around `center` square to `axis`.
fn circle(center: DVec3, axis: DVec3, radius: f64, n: usize) -> Vec<[DVec3; 2]> {
    let u = axis.any_orthonormal_vector() * radius;
    let at = |k: usize| {
        let t = rotation(center, axis, 360.0 * k as f64 / n as f64);
        t.transform_point3(center + u)
    };
    (0..n).map(|k| [at(k), at(k + 1)]).collect()
}

impl Tool for RotateTool {
    fn name(&self) -> &'static str {
        "Rotate"
    }

    fn prompt(&self) -> &'static str {
        match (&self.turn, self.copy) {
            (
                Some(Turn {
                    reference: None, ..
                }),
                _,
            ) => "Click where the angle starts, or type an angle",
            (Some(_), false) => "Click to set the angle, or type it in degrees",
            (Some(_), true) => "Copy: click to set the angle, or type it in degrees",
            (None, _) if self.last.as_ref().is_some_and(|l| l.copy) => {
                "Type an angle, 5x or /5, or click a new center"
            }
            (None, _) if self.last.is_some() => "Type an angle to change it, or click a new center",
            (None, false) => "Select geometry or a group, then click the center (Ctrl copies)",
            (None, true) => "Copy: select geometry or a group, then click the center",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("Angle", self.angle().map(format_angle))
    }

    fn anchor(&self) -> Option<DVec3> {
        self.turn.as_ref().map(|t| t.center)
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => {
                self.on_point = is_point(hit.kind);
                self.hover = self
                    .turn
                    .as_ref()
                    .and_then(|t| on_rotation_plane(t.center, t.axis, &hit, ctx));
            }
            ToolEvent::Click(hit) => match &mut self.turn {
                None => {
                    let ids: Vec<EntityId> = if ctx.selection.is_empty() {
                        pick(ctx).map_or(Vec::new(), |id| whole_curve(ctx.model, id))
                    } else {
                        ctx.selection.iter().copied().collect()
                    };
                    if ids.is_empty() {
                        out.message =
                            Some("Select geometry, or click an edge, face or group".into());
                    } else {
                        self.turn = Some(Turn {
                            center: hit.point,
                            axis: plane_normal(&hit, ctx),
                            outline: outline(ctx, &ids),
                            ids,
                            reference: None,
                        });
                        self.hover = Some(hit.point);
                    }
                }
                Some(turn) if turn.reference.is_none() => {
                    let from = on_rotation_plane(turn.center, turn.axis, &hit, ctx)
                        .map(|p| flat(p - turn.center, turn.axis))
                        .filter(|v| v.length() >= LENGTH_TOL);
                    match from {
                        Some(v) => turn.reference = Some(v.normalize()),
                        None => {
                            out.message = Some("Click away from the center".into());
                        }
                    }
                }
                Some(_) => {
                    self.on_point = is_point(hit.kind);
                    let t = self.turn.as_ref().unwrap();
                    self.hover = on_rotation_plane(t.center, t.axis, &hit, ctx);
                    match self.angle() {
                        Some(a) if a.abs() > 1e-9 => self.finish(a, &mut out),
                        _ => out.message = Some("Move away from the start to turn".into()),
                    }
                }
            },
            ToolEvent::Value(text) => {
                match (parse_angle(&text), parse_array(&text)) {
                    (Some(0.0), _) => {
                        out.message = Some("Angle must not be zero".into());
                    }
                    (Some(a), _) if self.turn.is_some() => {
                        // A typed angle turns the way the cursor does.
                        let a = if self.angle().is_some_and(|c| c < 0.0) {
                            -a
                        } else {
                            a
                        };
                        self.finish(a, &mut out);
                    }
                    (Some(a), _) => self.redo_with(a, &mut out),
                    (None, Some(array)) if self.turn.is_none() => self.array(array, &mut out),
                    _ => out.message = Some(format!("Not an angle: {text}")),
                }
            }
            ToolEvent::ToggleCopy => self.copy = !self.copy,
            ToolEvent::Cancel => {
                self.turn = None;
                self.hover = None;
            }
            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) => {}
        }
        if let Some(t) = &self.turn {
            let radius = PROTRACTOR * ctx.camera.eye_distance(t.center);
            out.preview = t.outline.clone();
            out.preview.extend(circle(t.center, t.axis, radius, 24));
            if let Some(from) = t.reference {
                out.preview.push([t.center, t.center + from * radius]);
            }
            if let Some(hover) = self.hover {
                out.preview.push([t.center, hover]);
            }
            if let Some(angle) = self.angle() {
                let turned = rotation(t.center, t.axis, angle);
                out.preview.extend(
                    t.outline
                        .iter()
                        .map(|[a, b]| [turned.transform_point3(*a), turned.transform_point3(*b)]),
                );
            }
        }
        out
    }

    fn on_external_change(&mut self) {
        self.turn = None;
        self.hover = None;
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;
    use std::collections::BTreeSet;

    /// A 2 m square (face f8) on the ground and a camera looking at it.
    struct Rig {
        tool: RotateTool,
        model: Model,
        camera: Camera,
        selection: BTreeSet<EntityId>,
        cursor: DVec2,
    }

    impl Rig {
        fn new(selected: &[&str]) -> Self {
            let mut model = Model::new();
            let pts = [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ];
            model.add_polygon(&pts).unwrap();
            Self {
                tool: RotateTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                selection: selected.iter().map(|s| s.parse().unwrap()).collect(),
                cursor: DVec2::ZERO,
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &crate::scenery::Scenery::single(&self.model),
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                drag_threshold: 4.0,
                shift: false,
                selection: &self.selection,
            };
            self.tool.handle(event, &ctx)
        }

        /// Puts the cursor over `p` and clicks there, snapped as `kind`.
        fn click_as(&mut self, p: DVec3, kind: SnapKind) -> ToolOutput {
            self.cursor = self.camera.project(p).unwrap();
            let face = crate::inference::face_under(&self.model, &self.camera, self.cursor)
                .map(|(f, _)| f);
            self.send(ToolEvent::Click(Inference {
                point: p,
                kind,
                face,
            }))
        }

        fn click(&mut self, p: DVec3) -> ToolOutput {
            self.click_as(p, SnapKind::Ground)
        }

        fn hover(&mut self, p: DVec3) -> ToolOutput {
            self.cursor = self.camera.project(p).unwrap();
            self.send(ToolEvent::Move(Inference {
                point: p,
                kind: SnapKind::Ground,
                face: None,
            }))
        }
    }

    fn ids(s: &[&str]) -> Vec<EntityId> {
        s.iter().map(|s| s.parse().unwrap()).collect()
    }

    fn rotate(ids_: &[&str], center: [f64; 3], angle: f64, copies: Option<u32>) -> Command {
        Command::Rotate {
            ids: ids(ids_),
            center,
            axis: Some([0.0, 0.0, 1.0]),
            angle,
            copies,
        }
    }

    fn turned(out: &ToolOutput) -> f64 {
        match out.commands.last() {
            Some(Command::Rotate { angle, .. }) => *angle,
            other => panic!("not a rotation: {other:?}"),
        }
    }

    #[test]
    fn three_clicks_turn_the_selection_about_the_faces_normal() {
        let mut r = Rig::new(&["f8"]);
        r.click(DVec3::new(1.0, 1.0, 0.0));
        r.click(DVec3::new(2.0, 1.0, 0.0));
        let live = r.hover(DVec3::new(1.0, 2.0, 0.0));
        assert_eq!(r.tool.measurement(), ("Angle", Some("90°".into())));
        assert_eq!(
            live.preview.len(),
            4 + 24 + 2 + 4,
            "outline, protractor, arms, turned"
        );
        let out = r.click(DVec3::new(1.0, 2.0, 0.0));
        assert_eq!(
            out.commands,
            vec![rotate(&["f8"], [1.0, 1.0, 0.0], 90.0, None)]
        );
        assert!(r.tool.anchor().is_none());
    }

    #[test]
    fn the_protractor_keeps_its_screen_size_in_a_parallel_view() {
        let mut r = Rig::new(&["f8"]);
        r.camera.projection = session::Projection::Parallel;
        r.camera.look_from(session::StandardView::Top);
        // Zoomed in to half a meter from the target (the origin): the center is 1.5 m from the eye.
        r.camera.zoom_by(r.camera.distance() / 0.5);
        let out = r.click(DVec3::new(1.0, 1.0, 0.0));
        let ring = &out.preview[4..4 + 24];
        let radius = ring[0][0].distance(DVec3::new(1.0, 1.0, 0.0));
        assert!((radius - PROTRACTOR * 0.5).abs() < 1e-9, "{radius}");
    }

    #[test]
    fn angles_snap_to_steps_off_points_and_follow_points_exactly() {
        let mut r = Rig::new(&["f8"]);
        r.click(DVec3::ZERO);
        r.click(DVec3::new(1.0, 0.0, 0.0));
        let near_45 = DVec3::new(1.0, 1.0 + 0.02, 0.0);
        let out = r.click(near_45);
        assert_eq!(turned(&out), 45.0);
        r.click(DVec3::ZERO);
        r.click(DVec3::new(1.0, 0.0, 0.0));
        let out = r.click_as(near_45, SnapKind::Endpoint);
        assert!((turned(&out) - 45.0).abs() > 0.1, "{}", turned(&out));
    }

    #[test]
    fn clockwise_turns_are_negative_and_typed_angles_follow_the_cursor() {
        let mut r = Rig::new(&["f8"]);
        r.click(DVec3::ZERO);
        r.click(DVec3::new(1.0, 0.0, 0.0));
        r.hover(DVec3::new(1.0, -0.3, 0.0));
        let out = r.send(ToolEvent::Value("30".into()));
        assert_eq!(out.commands, vec![rotate(&["f8"], [0.0; 3], -30.0, None)]);
    }

    #[test]
    fn an_angle_typed_after_the_center_or_after_a_rotation_turns_by_it() {
        let mut r = Rig::new(&["f8"]);
        r.click(DVec3::new(1.0, 1.0, 0.0));
        let out = r.send(ToolEvent::Value("45".into()));
        assert_eq!(
            out.commands,
            vec![rotate(&["f8"], [1.0, 1.0, 0.0], 45.0, None)]
        );
        let again = r.send(ToolEvent::Value("-60".into()));
        assert_eq!(
            again.commands,
            vec![Command::Undo, rotate(&["f8"], [1.0, 1.0, 0.0], -60.0, None)]
        );
        r.tool.on_external_change();
        let refused = r.send(ToolEvent::Value("10".into()));
        assert!(refused.commands.is_empty() && refused.message.is_some());
    }

    #[test]
    fn without_a_selection_the_center_click_picks_what_turns() {
        let mut r = Rig::new(&[]);
        assert!(r.click(DVec3::new(1.0, 1.0, 0.0)).message.is_none());
        let out = r.send(ToolEvent::Value("90".into()));
        assert_eq!(
            out.commands,
            vec![rotate(&["f8"], [1.0, 1.0, 0.0], 90.0, None)]
        );
        let empty = r.click(DVec3::new(-3.0, 4.0, 0.0));
        assert!(empty.message.is_some() && r.tool.anchor().is_none());
    }

    #[test]
    fn ctrl_copies_and_arrays_replace_the_copy() {
        let mut r = Rig::new(&["f8"]);
        r.send(ToolEvent::ToggleCopy);
        r.click(DVec3::ZERO);
        let out = r.send(ToolEvent::Value("90".into()));
        assert_eq!(out.commands, vec![rotate(&["f8"], [0.0; 3], 90.0, Some(1))]);
        let times = r.send(ToolEvent::Value("3x".into()));
        assert_eq!(
            times.commands,
            vec![Command::Undo, rotate(&["f8"], [0.0; 3], 90.0, Some(3))]
        );
        let divide = r.send(ToolEvent::Value("/3".into()));
        assert_eq!(
            divide.commands,
            vec![Command::Undo, rotate(&["f8"], [0.0; 3], 30.0, Some(3))]
        );
        let big = r.send(ToolEvent::Value("101x".into()));
        assert_eq!(big.message.as_deref(), Some("At most 100 copies"));
    }

    #[test]
    fn a_retyped_angle_keeps_the_direction_of_the_last_rotation() {
        let mut r = Rig::new(&["f8"]);
        r.click(DVec3::ZERO);
        r.send(ToolEvent::Value("-30".into()));
        let again = r.send(ToolEvent::Value("45".into()));
        assert_eq!(
            again.commands,
            vec![Command::Undo, rotate(&["f8"], [0.0; 3], -45.0, None)]
        );
        let flipped = r.send(ToolEvent::Value("-20".into()));
        assert_eq!(turned(&flipped), 20.0, "a typed minus flips it");
    }

    #[test]
    fn a_retyped_angle_after_an_array_keeps_the_count() {
        let mut r = Rig::new(&["f8"]);
        r.send(ToolEvent::ToggleCopy);
        r.click(DVec3::ZERO);
        r.send(ToolEvent::Value("90".into()));
        r.send(ToolEvent::Value("3x".into()));
        let out = r.send(ToolEvent::Value("45".into()));
        assert_eq!(
            out.commands,
            vec![Command::Undo, rotate(&["f8"], [0.0; 3], 45.0, Some(3))]
        );
        let five = r.send(ToolEvent::Value("5x".into()));
        assert_eq!(turned(&five), 45.0);
        assert!(matches!(
            five.commands.last(),
            Some(Command::Rotate {
                copies: Some(5),
                ..
            })
        ));
        let divide = r.send(ToolEvent::Value("/5".into()));
        assert_eq!(
            divide.commands.last(),
            Some(&rotate(&["f8"], [0.0; 3], 9.0, Some(5)))
        );
    }

    #[test]
    fn arrays_need_a_rotated_copy() {
        let mut r = Rig::new(&["f8"]);
        r.click(DVec3::ZERO);
        r.send(ToolEvent::Value("90".into()));
        let out = r.send(ToolEvent::Value("3x".into()));
        assert!(out.commands.is_empty() && out.message.is_some());
    }

    #[test]
    fn a_reference_on_the_center_or_a_zero_angle_is_refused() {
        let mut r = Rig::new(&["f8"]);
        r.click(DVec3::new(1.0, 1.0, 0.0));
        let out = r.click(DVec3::new(1.0, 1.0, 0.0));
        assert!(out.message.is_some() && out.commands.is_empty());
        r.click(DVec3::new(2.0, 1.0, 0.0));
        let out = r.click(DVec3::new(2.0, 1.0, 0.0));
        assert!(out.message.is_some() && out.commands.is_empty());
        assert!(r.send(ToolEvent::Value("0".into())).message.is_some());
        assert!(r.tool.anchor().is_some(), "still turning");
        r.send(ToolEvent::Cancel);
        assert!(r.tool.anchor().is_none());
    }

    /// An 8-sided circle of radius 1 on the ground, as the model and as a document's.
    fn circle_rig(r: &mut Rig) -> session::Document {
        let ring: Vec<DVec3> = (0..8)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 8.0;
                DVec3::new(a.cos(), a.sin(), 0.0)
            })
            .collect();
        let mut model = Model::new();
        model.add_curve(&ring, true).unwrap();
        r.model = model;
        let mut d = session::Document::new(DVec2::new(800.0, 600.0));
        d.execute(Command::AddCircle {
            center: [0.0; 3],
            radius: 1.0,
            normal: None,
            segments: Some(8),
        })
        .unwrap();
        d
    }

    /// The middle of one rim edge.
    fn rim() -> DVec3 {
        let at = |deg: f64| {
            let a = deg.to_radians();
            DVec3::new(a.cos(), a.sin(), 0.0)
        };
        (at(225.0) + at(270.0)) / 2.0
    }

    fn circle_of(d: &session::Document, ids: &[EntityId]) -> kernel::CurveCircle {
        let EntityId::Edge(e) = ids[0] else {
            panic!("{ids:?}")
        };
        let c = d.model().curve_of(e).unwrap();
        assert_eq!(c.kind(), kernel::CurveKind::Circle);
        c.circle.unwrap()
    }

    #[test]
    fn without_a_selection_turning_a_rim_edge_turns_the_whole_circle() {
        let mut r = Rig::new(&[]);
        let mut d = circle_rig(&mut r);
        r.click(rim());
        r.click(DVec3::new(0.0, 0.0, 0.0) + DVec3::X * 3.0);
        let out = r.click(DVec3::new(0.0, 3.0, 0.0));
        let [Command::Rotate { ids, .. }] = &out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert_eq!(ids.len(), 8, "every edge of the circle");
        d.execute(out.commands[0].clone()).unwrap();
        let circle = circle_of(&d, ids);
        assert!((circle.radius - 1.0).abs() < 1e-9);
    }
}
