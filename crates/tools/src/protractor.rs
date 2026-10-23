//! The Protractor tool: click the center, click a point on the baseline, then move to set the
//! angle (snapping to 15 degree steps as the Rotate protractor does) and click to place a guide
//! line through the center at that angle, in the protractor's plane: the face under the center,
//! or the plane an arrow key locked before the center click. A typed angle ("30", "-45") or slope
//! ("1:2", rise over run) replaces the click, turning the way the cursor does as for Rotate;
//! typed right after a guide was placed, it replaces that guide. Ctrl switches guide creation off
//! (measuring only) and on again.

use glam::{DQuat, DVec3};
use kernel::geom::LENGTH_TOL;
use session::{Command, Units};

use crate::inference::Axis;
use crate::rectangle::plane_normal;
use crate::rotate::{PROTRACTOR, circle, flat, is_point, on_rotation_plane, protractor_angle};
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_angle, parse_angle, parse_slope};

/// A measurement in progress.
#[derive(Clone, Copy, Debug)]
struct Center {
    point: DVec3,
    /// Unit vector the angle is counter-clockwise about.
    axis: DVec3,
    /// Unit vector in the plane the angle is measured from, and where its click was.
    baseline: Option<(DVec3, DVec3)>,
}

/// The guide just placed, for a typed value to replace.
#[derive(Clone, Copy, Debug)]
struct Last {
    point: DVec3,
    axis: DVec3,
    reference: DVec3,
    angle: f64,
}

impl Last {
    fn direction(&self, angle: f64) -> DVec3 {
        DQuat::from_axis_angle(self.axis, angle.to_radians()) * self.reference
    }
}

#[derive(Clone, Debug)]
pub struct ProtractorTool {
    center: Option<Center>,
    /// The cursor on the protractor's plane.
    hover: Option<DVec3>,
    /// Whether the cursor is on a point, which the angle follows exactly.
    on_point: bool,
    /// Whether placing the angle adds a guide (Ctrl toggles it).
    guides: bool,
    last: Option<Last>,
    /// The axis the protractor's plane is locked square to, set before the center click.
    plane: Option<Axis>,
}

impl Default for ProtractorTool {
    fn default() -> Self {
        ProtractorTool {
            center: None,
            hover: None,
            on_point: false,
            guides: true,
            last: None,
            plane: None,
        }
    }
}

impl ProtractorTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The angle from the baseline to the cursor, in degrees, snapped unless on a point.
    fn angle(&self, snap: f64) -> Option<f64> {
        let c = self.center.as_ref()?;
        let (from, _) = c.baseline?;
        let to = flat(self.hover? - c.point, c.axis).try_normalize()?;
        Some(protractor_angle(from, to, c.axis, self.on_point, snap))
    }

    /// Places the guide at `angle` (or only reports it), ending the measurement.
    fn finish(&mut self, angle: f64, out: &mut ToolOutput, units: &Units) {
        let Some(c) = self.center.take() else { return };
        let Some((reference, _)) = c.baseline else {
            return;
        };
        self.hover = None;
        if self.guides {
            let last = Last {
                point: c.point,
                axis: c.axis,
                reference,
                angle,
            };
            out.commands.push(Command::AddGuideLine {
                point: c.point.to_array(),
                direction: last.direction(angle).to_array(),
            });
            self.last = Some(last);
        } else {
            out.message = Some(format!("Angle: {}", format_angle(angle, units)));
            out.info = true;
        }
    }
}

impl Tool for ProtractorTool {
    fn name(&self) -> &'static str {
        "Protractor"
    }

    fn prompt(&self) -> &'static str {
        match (&self.center, self.guides) {
            (None, true) if self.last.is_some() => {
                "Type an angle to change the guide, or click a new center; arrows lock the plane"
            }
            (None, true) => "Click the center; arrows lock the plane (Ctrl measures only)",
            (None, false) => "Measuring only: click the center; Ctrl switches guides on",
            (Some(Center { baseline: None, .. }), _) => "Click a point on the baseline",
            (Some(_), true) => {
                "Click to set the angle and place a guide, or type an angle or rise:run"
            }
            (Some(_), false) => "Click to measure the angle, or type an angle or rise:run",
        }
    }

    fn measurement(&self, units: &session::Units) -> (&'static str, Option<String>) {
        (
            "Angle",
            self.angle(units.angle_snap).map(|a| format_angle(a, units)),
        )
    }

    fn anchor(&self) -> Option<DVec3> {
        self.center.map(|c| c.point)
    }

    /// The center, then the click on the baseline.
    fn phase(&self) -> Phase {
        match &self.center {
            Some(c) => Phase::Active {
                points: std::iter::once(c.point)
                    .chain(c.baseline.map(|(_, at)| at))
                    .collect(),
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => {
                self.on_point = is_point(hit.kind);
                self.hover = match &self.center {
                    Some(c) => on_rotation_plane(c.point, c.axis, &hit, ctx),
                    None => self.plane.map(|_| hit.point),
                };
            }
            ToolEvent::Click(hit) => match &mut self.center {
                None => {
                    self.center = Some(Center {
                        point: hit.point,
                        axis: self.plane.map_or_else(
                            || plane_normal(&hit, ctx),
                            |a| a.dir_in(ctx.scenery.axes().matrix3),
                        ),
                        baseline: None,
                    });
                    self.last = None;
                    self.hover = Some(hit.point);
                }
                Some(c) if c.baseline.is_none() => {
                    let from = on_rotation_plane(c.point, c.axis, &hit, ctx)
                        .map(|p| flat(p - c.point, c.axis))
                        .filter(|v| v.length() >= LENGTH_TOL);
                    match from {
                        Some(v) => c.baseline = Some((v.normalize(), c.point + v)),
                        None => out.message = Some("Click away from the center".into()),
                    }
                }
                Some(_) => {
                    self.on_point = is_point(hit.kind);
                    let c = self.center.unwrap();
                    self.hover = on_rotation_plane(c.point, c.axis, &hit, ctx);
                    match self.angle(ctx.units.angle_snap) {
                        Some(a) => self.finish(a, &mut out, &ctx.units),
                        None => out.message = Some("Move away from the center".into()),
                    }
                }
            },
            ToolEvent::Value(text) => {
                let typed = parse_angle(&text).or_else(|| parse_slope(&text, &ctx.units));
                match (typed, self.center, self.last) {
                    (None, ..) => {
                        out.message = Some(format!("Not an angle or rise:run: {text}"));
                    }
                    (Some(typed), Some(c), _) if c.baseline.is_some() => {
                        // A typed angle turns the way the cursor does.
                        let a = if self.angle(ctx.units.angle_snap).is_some_and(|a| a < 0.0) {
                            -typed
                        } else {
                            typed
                        };
                        self.finish(a, &mut out, &ctx.units);
                    }
                    (Some(_), Some(_), _) => out.message = Some("Click the baseline first".into()),
                    // Right after a guide was placed: replace it, turning the way it did.
                    (Some(typed), None, Some(last)) if self.guides => {
                        let a = if last.angle < 0.0 { -typed } else { typed };
                        out.commands.push(Command::Undo {
                            count: None,
                            expect: None,
                        });
                        out.commands.push(Command::AddGuideLine {
                            point: last.point.to_array(),
                            direction: last.direction(a).to_array(),
                        });
                        self.last = Some(Last { angle: a, ..last });
                    }
                    (Some(_), None, _) => out.message = Some("Click the center first".into()),
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
            ToolEvent::Cancel => {
                self.on_external_change();
                self.plane = None;
            }
            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) => {}
        }
        if let Some(c) = &self.center {
            let radius = PROTRACTOR * ctx.camera.eye_distance(c.point);
            out.preview = circle(c.point, c.axis, radius, 24);
            if let Some((from, _)) = c.baseline {
                out.preview.push([c.point, c.point + from * radius]);
            }
            if let Some(hover) = self.hover {
                out.preview.push([c.point, hover]);
            }
            if let (Some((reference, _)), Some(angle)) =
                (c.baseline, self.angle(ctx.units.angle_snap))
            {
                let dir = DQuat::from_axis_angle(c.axis, angle.to_radians()) * reference;
                out.preview.push([c.point, c.point + dir * radius]);
                if self.guides {
                    out.preview.extend(ctx.camera.line_span(c.point, dir));
                }
            }
        } else if let (Some(axis), Some(at)) = (self.plane, self.hover) {
            let radius = PROTRACTOR * ctx.camera.eye_distance(at);
            out.preview = circle(at, axis.dir_in(ctx.scenery.axes().matrix3), radius, 24);
        }
        out
    }

    fn on_external_change(&mut self) {
        self.center = None;
        self.hover = None;
        self.last = None;
    }

    /// Taken only while idle.
    fn lock_plane(&mut self, axis: Option<Axis>) -> bool {
        if self.center.is_some() {
            return false;
        }
        self.plane = axis;
        true
    }

    fn plane_lock(&self) -> Option<Axis> {
        self.plane
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::SnapKind;
    use crate::testing::Rig;
    use crate::tool::ToolKind;
    use glam::{DAffine3, DMat3};
    use kernel::Model;

    fn v(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    fn rig() -> Rig {
        Rig::new(ToolKind::Protractor, Model::new())
    }

    fn polar(deg: f64, r: f64) -> DVec3 {
        v(r * deg.to_radians().cos(), r * deg.to_radians().sin(), 0.0)
    }

    fn guide(out: &ToolOutput) -> (DVec3, DVec3) {
        match out.commands.last() {
            Some(Command::AddGuideLine { point, direction }) => {
                (DVec3::from_array(*point), DVec3::from_array(*direction))
            }
            other => panic!("not a guide: {other:?}"),
        }
    }

    fn close(a: DVec3, b: DVec3) -> bool {
        a.abs_diff_eq(b, 1e-9)
    }

    /// Center at (1, 1, 0) and baseline along +X, the cursor not yet moved.
    fn started() -> Rig {
        let mut r = rig();
        r.click(v(1.0, 1.0, 0.0));
        r.click(v(3.0, 1.0, 0.0));
        r
    }

    #[test]
    fn center_baseline_and_angle_place_a_guide_through_the_center() {
        let mut r = started();
        let c = v(1.0, 1.0, 0.0);
        let out = r.mv(c + polar(30.0, 1.0));
        assert!(!out.preview.is_empty());
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Angle", Some("30°".to_string()))
        );
        let out = r.click(c + polar(30.0, 1.0));
        let (point, dir) = guide(&out);
        assert_eq!(point, c);
        assert!(close(dir, polar(30.0, 1.0)), "{dir:?}");
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn the_angle_snaps_to_steps_off_points_and_not_on_them() {
        let c = v(1.0, 1.0, 0.0);
        let mut r = started();
        r.mv(c + polar(31.0, 1.0));
        assert_eq!(
            r.tool.measurement(&Default::default()).1,
            Some("30°".to_string())
        );
        r.mv(c + polar(38.0, 1.0));
        assert_eq!(
            r.tool.measurement(&Default::default()).1,
            Some("38°".to_string())
        );
        // On an endpoint the angle is exact.
        let hit = r.at(c + polar(31.0, 1.0), SnapKind::Endpoint, None);
        r.send(ToolEvent::Move(hit));
        assert_eq!(
            r.tool.measurement(&Default::default()).1,
            Some("31°".to_string())
        );
        // Clockwise is negative.
        r.mv(c + polar(-44.0, 1.0));
        assert_eq!(
            r.tool.measurement(&Default::default()).1,
            Some("-45°".to_string())
        );
    }

    #[test]
    fn the_plane_is_the_face_under_the_center() {
        // A wall in the plane x = 0.
        let mut m = Model::new();
        m.add_polygon(&[
            v(0.0, 0.0, 0.0),
            v(0.0, 4.0, 0.0),
            v(0.0, 4.0, 3.0),
            v(0.0, 0.0, 3.0),
        ])
        .unwrap();
        let wall = m.faces().next().unwrap().0;
        let mut r = Rig::new(ToolKind::Protractor, m);
        let on = |r: &mut Rig, y, z| r.at(v(0.0, y, z), SnapKind::OnFace, Some(wall));
        let hit = on(&mut r, 1.0, 1.0);
        r.send(ToolEvent::Click(hit));
        let hit = on(&mut r, 3.0, 1.0);
        r.send(ToolEvent::Click(hit));
        let hit = on(&mut r, 3.0, 3.0);
        r.send(ToolEvent::Move(hit));
        let out = r.send(ToolEvent::Click(hit));
        let (point, dir) = guide(&out);
        assert_eq!(point, v(0.0, 1.0, 1.0));
        assert!(
            dir.x.abs() < 1e-9 && dir.y.abs() > 0.6 && dir.z.abs() > 0.6,
            "{dir:?}"
        );
        assert!(
            (dir.y.abs() - dir.z.abs()).abs() < 1e-9,
            "45 degrees: {dir:?}"
        );
    }

    #[test]
    fn an_arrow_lock_sets_the_plane_from_the_drawing_axes() {
        let mut r = rig();
        // Red axis along +Y: locking red puts the protractor in the plane normal to +Y.
        r.axes = DAffine3::from_mat3_translation(
            DMat3::from_cols(DVec3::Y, -DVec3::X, DVec3::Z),
            DVec3::ZERO,
        );
        assert!(r.tool.lock_plane(Some(crate::Axis::X)));
        assert_eq!(r.tool.plane_lock(), Some(crate::Axis::X));
        r.click(v(0.0, 0.0, 0.0));
        r.click(v(0.0, 0.0, 1.0));
        let hit = r.at(v(1.0, 0.0, 0.0), SnapKind::Endpoint, None);
        r.send(ToolEvent::Move(hit));
        let out = r.send(ToolEvent::Click(hit));
        let (_, dir) = guide(&out);
        // From +Z toward +X is a quarter turn about +Y.
        assert!(close(dir, v(1.0, 0.0, 0.0)), "{dir:?}");
    }

    #[test]
    fn a_typed_angle_places_the_guide_turned_the_way_the_cursor_is() {
        let mut r = started();
        let out = r.value("30");
        assert!(close(guide(&out).1, polar(30.0, 1.0)));
        let mut r = started();
        let out = r.value("-45");
        assert!(close(guide(&out).1, polar(-45.0, 1.0)));
        // With the cursor on the clockwise side a plain angle turns clockwise.
        let mut r = started();
        r.mv(v(1.0, 1.0, 0.0) + polar(-60.0, 1.0));
        let out = r.value("30");
        assert!(close(guide(&out).1, polar(-30.0, 1.0)));
        let mut r = started();
        let out = r.value("22.5deg");
        assert!(close(guide(&out).1, polar(22.5, 1.0)));
    }

    #[test]
    fn a_typed_slope_is_rise_over_run() {
        let mut r = started();
        let out = r.value("1:2");
        let a = 0.5f64.atan().to_degrees();
        assert!(close(guide(&out).1, polar(a, 1.0)), "{:?}", guide(&out));
        let mut r = started();
        let out = r.value("3:12");
        assert!(close(
            guide(&out).1,
            polar(0.25f64.atan().to_degrees(), 1.0)
        ));
    }

    #[test]
    fn a_typed_value_right_after_a_click_replaces_the_guide() {
        let mut r = started();
        let c = v(1.0, 1.0, 0.0);
        r.click(c + polar(30.0, 1.0));
        let out = r.value("45");
        assert_eq!(out.commands.len(), 2);
        assert!(matches!(out.commands[0], Command::Undo { .. }));
        assert!(close(guide(&out).1, polar(45.0, 1.0)));
        // And again, with a slope.
        let out = r.value("1:1");
        assert_eq!(out.commands.len(), 2);
        assert!(close(guide(&out).1, polar(45.0, 1.0)));
        // A new center forgets the guide.
        r.click(v(5.0, 5.0, 0.0));
        let out = r.value("10");
        assert!(out.commands.is_empty());
    }

    #[test]
    fn replacing_keeps_the_direction_the_guide_turned() {
        let mut r = started();
        let c = v(1.0, 1.0, 0.0);
        r.click(c + polar(-30.0, 1.0));
        let out = r.value("45");
        assert!(close(guide(&out).1, polar(-45.0, 1.0)));
    }

    #[test]
    fn ctrl_measures_without_a_guide() {
        let mut r = started();
        r.send(ToolEvent::ToggleCopy);
        let out = r.click(v(1.0, 1.0, 0.0) + polar(60.0, 1.0));
        assert!(out.commands.is_empty());
        assert!(out.info && out.message.as_deref().unwrap().contains("60"));
        assert_eq!(r.tool.phase(), Phase::Idle);
        // A typed angle only measures too.
        r.click(v(1.0, 1.0, 0.0));
        r.click(v(3.0, 1.0, 0.0));
        let out = r.value("1:1");
        assert!(out.commands.is_empty() && out.message.as_deref().unwrap().contains("45"));
        // Ctrl again brings the guides back.
        r.send(ToolEvent::ToggleCopy);
        r.click(v(1.0, 1.0, 0.0));
        r.click(v(3.0, 1.0, 0.0));
        assert_eq!(r.value("20").commands.len(), 1);
    }

    #[test]
    fn bad_input_explains_why() {
        let mut r = rig();
        assert!(r.value("30").message.is_some());
        r.click(v(1.0, 1.0, 0.0));
        let out = r.click(v(1.0, 1.0, 0.0));
        assert!(out.message.is_some(), "baseline on the center");
        assert!(matches!(r.tool.phase(), Phase::Active { ref points } if points.len() == 1));
        assert!(r.value("30").message.is_some(), "no baseline yet");
        r.click(v(3.0, 1.0, 0.0));
        for bad in ["x", "1:0", "1:2:3", ""] {
            let out = r.value(bad);
            assert!(out.commands.is_empty() && out.message.is_some(), "{bad:?}");
        }
        assert!(matches!(r.tool.phase(), Phase::Active { ref points } if points.len() == 2));
    }

    #[test]
    fn a_bad_typed_value_keeps_the_preview() {
        let mut r = started();
        r.mv(v(1.0, 1.0, 0.0) + polar(30.0, 1.0));
        let out = r.value("x");
        assert!(out.message.is_some() && !out.preview.is_empty());
    }

    #[test]
    fn a_click_without_moving_places_a_guide_along_the_baseline() {
        let mut r = started();
        let out = r.click(v(1.0, 1.0, 0.0) + polar(0.0, 1.0));
        assert!(close(guide(&out).1, v(1.0, 0.0, 0.0)));
    }

    #[test]
    fn escape_cancels_and_the_tool_has_no_shortcut() {
        let mut r = started();
        r.send(ToolEvent::Cancel);
        assert_eq!(r.tool.phase(), Phase::Idle);
        assert_eq!(ToolKind::Protractor.shortcut(), None);
        assert_eq!(ToolKind::Freehand.shortcut(), None);
        assert_eq!(ToolKind::RotatedRectangle.shortcut(), None);
    }
}
