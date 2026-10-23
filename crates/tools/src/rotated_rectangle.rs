//! The Rotated Rectangle tool: click the first corner, click the second (the first side, at any
//! angle), then move to set the width and the plane and click. A drag from the first corner to
//! the second works as the two clicks. Typed after the first click: `length` (toward the cursor)
//! or `length,angle`, the angle in degrees in the plane of the first corner (the face under it,
//! or the horizontal plane of the drawing axes) from its red direction, counter-clockwise seen
//! from the plane's normal as the Protractor measures. Typed after the second click: `width`
//! (toward the cursor) or `width,angle`, the angle that tilts the rectangle about the first side
//! up from that plane. Escape steps back.

use glam::{DMat3, DQuat, DVec3};
use kernel::geom::LENGTH_TOL;
use session::{Command, Units, rectangle_axes_in};

use crate::inference::{Inference, SnapKind};
use crate::rectangle::{on_plane, plane_normal};
use crate::tool::{DragStart, Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{
    format_length, is_point_snap, parse_angle, parse_length, snap_distance, snap_length,
};

#[derive(Clone, Debug, Default)]
pub struct RotatedRectangleTool {
    /// The drawing axes the last event saw.
    axes: DMat3,
    /// The first corner and the normal of the plane its side and width start in.
    corner: Option<(DVec3, DVec3)>,
    /// The end of the first side, once clicked.
    edge_end: Option<DVec3>,
    /// The point under the cursor: the side's end, then the width point.
    hover: Option<DVec3>,
    drag_start: DragStart,
}

/// `v` square to the unit vector `dir`.
fn square_to(v: DVec3, dir: DVec3) -> DVec3 {
    v - dir * v.dot(dir)
}

/// The corners of the rectangle with the side `a`-`b` and the width to `w`, and its width.
fn corners(a: DVec3, b: DVec3, w: DVec3) -> Option<([DVec3; 4], f64)> {
    let side = (b - a).try_normalize()?;
    let across = square_to(w - a, side);
    let width = across.length();
    (width >= LENGTH_TOL).then(|| ([a, b, b + across, a + across], width))
}

impl RotatedRectangleTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The point a cursor at `hit` gives: where the cursor ray crosses the corner's plane
    /// when the cursor is on nothing in particular, else the inferred point (which may leave
    /// the plane and so tilt the rectangle).
    /// Unless a point of the model was snapped to, the side's length and then the width are
    /// rounded to the model's length snapping.
    fn point_at(&self, hit: &Inference, ctx: &ToolContext) -> DVec3 {
        let p = match (self.corner, hit.kind) {
            (Some((a, normal)), SnapKind::Ground | SnapKind::Free) => {
                on_plane(a, normal, hit, ctx).unwrap_or(hit.point)
            }
            _ => hit.point,
        };
        let Some((a, _)) = self.corner else { return p };
        if ctx.units.length_snap <= 0.0 || is_point_snap(hit.kind) {
            return p;
        }
        match self.edge_end {
            None => snap_distance(a, p, hit.kind, &ctx.units),
            Some(b) => match (b - a).try_normalize() {
                Some(side) => {
                    let across = square_to(p - a, side);
                    let width = across.length();
                    if width < LENGTH_TOL {
                        return p;
                    }
                    p - across + across * (snap_length(width, &ctx.units) / width)
                }
                None => p,
            },
        }
    }

    /// Ends the first side at `b`, if it has a length.
    fn set_side(&mut self, b: DVec3, out: &mut ToolOutput) {
        let Some((a, _)) = self.corner else { return };
        if a.distance(b) < LENGTH_TOL {
            out.message = Some("Click away from the first corner".into());
            return;
        }
        self.edge_end = Some(b);
        self.hover = None;
    }

    /// Finishes the rectangle with the width point `w`, if it has a width.
    fn finish(&mut self, w: DVec3, out: &mut ToolOutput) {
        let (Some((a, _)), Some(b)) = (self.corner, self.edge_end) else {
            return;
        };
        if corners(a, b, w).is_none() {
            out.message = Some("Move off the first side to set the width".into());
            return;
        }
        out.commands.push(Command::AddRotatedRectangle {
            corner: a.to_array(),
            edge_end: b.to_array(),
            width_point: w.to_array(),
        });
        self.on_external_change();
    }

    /// Handles typed text: the side after the first click, the width after the second.
    fn typed(&mut self, text: &str, units: &Units, out: &mut ToolOutput) {
        let Some((a, normal)) = self.corner else {
            out.message = Some("Click the first corner first".into());
            return;
        };
        let (length, angle) = match text.split_once(',') {
            Some((l, d)) => (parse_length(l, units), parse_angle(d).map(Some)),
            None => (parse_length(text, units), Some(None)),
        };
        let (Some(length), Some(angle)) = (length, angle) else {
            let what = if self.edge_end.is_none() {
                "length,angle"
            } else {
                "width,angle"
            };
            out.message = Some(format!("Not {what}: {text}"));
            return;
        };
        if length.abs() < LENGTH_TOL {
            out.message = Some("The length must not be zero".into());
            return;
        }
        let Some(b) = self.edge_end else {
            let dir = match angle {
                Some(deg) => {
                    let (u, _) = rectangle_axes_in(self.axes, normal);
                    Some(DQuat::from_axis_angle(normal, deg.to_radians()) * u)
                }
                None => self.hover.and_then(|h| (h - a).try_normalize()),
            };
            match dir {
                Some(dir) => self.set_side(a + dir * length, out),
                None => out.message = Some("Move the cursor to show the direction".into()),
            }
            return;
        };
        let Some(side) = (b - a).try_normalize() else {
            return;
        };
        // In the plane of the corner square to the side, toward the cursor; the tilt goes up
        // toward the plane's normal.
        let (Some(across), Some(up)) = (
            normal.cross(side).try_normalize(),
            square_to(normal, side).try_normalize(),
        ) else {
            out.message = Some("The first side runs along the plane's normal".into());
            return;
        };
        let across = match self.hover {
            Some(h) if (h - a).dot(across) <= -LENGTH_TOL => -across,
            _ => across,
        };
        let rad = angle.unwrap_or(0.0).to_radians();
        let dir = across * rad.cos() + up * rad.sin();
        self.finish(a + dir * length, out);
    }
}

impl Tool for RotatedRectangleTool {
    fn name(&self) -> &'static str {
        "Rotated Rectangle"
    }

    fn prompt(&self) -> &'static str {
        match (self.corner, self.edge_end) {
            (None, _) => "Click the first corner",
            (Some(_), None) => "Click the end of the first side, or type length,angle",
            (Some(_), Some(_)) => "Move to set the width and plane and click, or type width,angle",
        }
    }

    fn measurement(&self, units: &session::Units) -> (&'static str, Option<String>) {
        let (Some((a, _)), hover) = (self.corner, self.hover) else {
            return ("Length", None);
        };
        match self.edge_end {
            None => ("Length", hover.map(|h| format_length(a.distance(h), units))),
            Some(b) => (
                "Width",
                hover
                    .and_then(|h| corners(a, b, h))
                    .map(|(_, w)| format_length(w, units)),
            ),
        }
    }

    fn anchor(&self) -> Option<DVec3> {
        self.edge_end.or(self.corner.map(|(a, _)| a))
    }

    fn phase(&self) -> Phase {
        match self.corner {
            Some((a, _)) => Phase::Active {
                points: std::iter::once(a).chain(self.edge_end).collect(),
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        self.axes = ctx.scenery.axes().matrix3;
        match event {
            ToolEvent::Move(hit) => {
                if self.corner.is_some() {
                    self.hover = Some(self.point_at(&hit, ctx));
                }
            }
            ToolEvent::Click(hit) => match (self.corner, self.edge_end) {
                (None, _) => {
                    self.corner = Some((hit.point, plane_normal(&hit, ctx)));
                    self.drag_start.arm(ctx.cursor);
                }
                (Some(_), None) => {
                    let b = self.point_at(&hit, ctx);
                    self.set_side(b, &mut out);
                }
                (Some(_), Some(_)) => {
                    let w = self.point_at(&hit, ctx);
                    self.finish(w, &mut out);
                }
            },
            ToolEvent::Release(hit) => {
                if self.drag_start.released(ctx) {
                    return self.handle(ToolEvent::Click(hit), ctx);
                }
            }
            ToolEvent::Value(text) => self.typed(&text, &ctx.units, &mut out),
            ToolEvent::Cancel => {
                if self.edge_end.take().is_none() {
                    self.on_external_change();
                }
                self.hover = None;
            }
            ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) | ToolEvent::ToggleCopy => {}
        }
        if let (Some((a, _)), Some(hover)) = (self.corner, self.hover) {
            match self.edge_end {
                None => out.preview.push([a, hover]),
                Some(b) => {
                    if let Some((c, _)) = corners(a, b, hover) {
                        out.preview = (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect();
                    } else {
                        out.preview.push([a, b]);
                    }
                }
            }
        } else if let (Some((a, _)), Some(b)) = (self.corner, self.edge_end) {
            out.preview.push([a, b]);
        }
        out
    }

    fn on_external_change(&mut self) {
        self.corner = None;
        self.edge_end = None;
        self.hover = None;
        self.drag_start = DragStart::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::SnapKind;
    use crate::testing::{Rig, only};
    use crate::tool::ToolKind;
    use glam::DAffine3;
    use kernel::Model;

    fn v(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    fn rig() -> Rig {
        Rig::new(ToolKind::RotatedRectangle, Model::new())
    }

    fn rotated(out: &ToolOutput) -> [DVec3; 3] {
        match only(out) {
            Command::AddRotatedRectangle {
                corner,
                edge_end,
                width_point,
            } => [corner, edge_end, width_point].map(|q| DVec3::from_array(*q)),
            other => panic!("not a rotated rectangle: {other:?}"),
        }
    }

    fn close(a: DVec3, b: DVec3) -> bool {
        a.abs_diff_eq(b, 1e-9)
    }

    #[test]
    fn three_clicks_draw_a_rectangle_at_any_angle() {
        let mut r = rig();
        assert!(r.click(v(0.0, 0.0, 0.0)).commands.is_empty());
        assert!(r.click(v(2.0, 1.0, 0.0)).commands.is_empty());
        let moved = r.mv(v(0.0, 1.5, 0.0));
        assert_eq!(moved.preview.len(), 4);
        let out = r.click(v(0.0, 1.5, 0.0));
        assert_eq!(
            rotated(&out),
            [v(0.0, 0.0, 0.0), v(2.0, 1.0, 0.0), v(0.0, 1.5, 0.0)]
        );
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn the_preview_is_the_rectangle_toward_the_cursor() {
        let mut r = rig();
        r.click(v(0.0, 0.0, 0.0));
        r.click(v(2.0, 0.0, 0.0));
        let out = r.mv(v(1.0, -0.75, 0.0));
        let corners: Vec<DVec3> = out.preview.iter().map(|[a, _]| *a).collect();
        let expect = [
            v(0.0, 0.0, 0.0),
            v(2.0, 0.0, 0.0),
            v(2.0, -0.75, 0.0),
            v(0.0, -0.75, 0.0),
        ];
        assert!(
            expect.iter().all(|e| corners.iter().any(|c| close(*c, *e))),
            "{corners:?}"
        );
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Width", Some("0.75 m".to_string()))
        );
    }

    #[test]
    fn a_width_point_on_the_side_is_refused() {
        let mut r = rig();
        r.click(v(0.0, 0.0, 0.0));
        r.click(v(2.0, 0.0, 0.0));
        let out = r.click(v(1.0, 0.0, 0.0));
        assert!(out.commands.is_empty() && out.message.is_some());
        assert!(matches!(r.tool.phase(), Phase::Active { ref points } if points.len() == 2));
    }

    #[test]
    fn the_first_side_can_be_typed_with_an_angle_from_the_red_axis() {
        let mut r = rig();
        r.click(v(1.0, 1.0, 0.0));
        r.mv(v(2.0, 1.0, 0.0));
        let out = r.value("2,90");
        assert!(out.commands.is_empty());
        assert!(matches!(r.tool.phase(), Phase::Active { ref points }
            if points.len() == 2 && close(points[1], v(1.0, 3.0, 0.0))));
        let mut r = rig();
        r.click(v(0.0, 0.0, 0.0));
        r.value("3,30");
        let h = 3.0 * 30f64.to_radians().cos();
        let s = 3.0 * 30f64.to_radians().sin();
        assert!(
            matches!(r.tool.phase(), Phase::Active { ref points }
            if close(points[1], v(h, s, 0.0))),
            "{:?}",
            r.tool.phase()
        );
    }

    #[test]
    fn a_typed_length_alone_goes_toward_the_cursor() {
        let mut r = rig();
        r.click(v(0.0, 0.0, 0.0));
        r.mv(v(-1.0, 0.0, 0.0));
        r.value("2");
        assert!(
            matches!(r.tool.phase(), Phase::Active { ref points }
            if close(points[1], v(-2.0, 0.0, 0.0))),
            "{:?}",
            r.tool.phase()
        );
    }

    #[test]
    fn the_typed_angle_follows_the_moved_drawing_axes() {
        let mut r = rig();
        // Red axis along +Y, green along -X.
        r.axes = DAffine3::from_mat3_translation(
            glam::DMat3::from_cols(DVec3::Y, -DVec3::X, DVec3::Z),
            DVec3::ZERO,
        );
        r.click(v(0.0, 0.0, 0.0));
        r.value("2,0");
        assert!(
            matches!(r.tool.phase(), Phase::Active { ref points }
            if close(points[1], v(0.0, 2.0, 0.0))),
            "{:?}",
            r.tool.phase()
        );
        let mut r2 = rig();
        r2.axes = r.axes;
        r2.click(v(0.0, 0.0, 0.0));
        r2.value("2,90");
        assert!(
            matches!(r2.tool.phase(), Phase::Active { ref points }
            if close(points[1], v(-2.0, 0.0, 0.0))),
            "{:?}",
            r2.tool.phase()
        );
    }

    /// A rig with the side (0,0,0)-(2,0,0) drawn and the cursor over `hover`.
    fn with_side(hover: DVec3) -> Rig {
        let mut r = rig();
        r.click(v(0.0, 0.0, 0.0));
        r.value("2,0");
        r.mv(hover);
        r
    }

    #[test]
    fn a_typed_width_goes_toward_the_cursor() {
        let mut r = with_side(v(1.0, -0.7, 0.0));
        let out = r.value("1");
        assert_eq!(
            rotated(&out),
            [v(0.0, 0.0, 0.0), v(2.0, 0.0, 0.0), v(0.0, -1.0, 0.0)]
        );
        let mut r = with_side(v(1.0, 0.7, 0.0));
        let out = r.value("1.5");
        assert_eq!(rotated(&out)[2], v(0.0, 1.5, 0.0));
    }

    #[test]
    fn a_typed_width_angle_tilts_the_plane_about_the_side() {
        let mut r = with_side(v(1.0, 0.7, 0.0));
        let out = r.value("1,90");
        assert!(close(rotated(&out)[2], v(0.0, 0.0, 1.0)));
        let mut r = with_side(v(1.0, 0.7, 0.0));
        let out = r.value("2,30");
        let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
        assert!(close(rotated(&out)[2], v(0.0, 2.0 * c, 2.0 * s)));
        // From the other side the tilt is still upward.
        let mut r = with_side(v(1.0, -0.7, 0.0));
        let out = r.value("2,30");
        assert!(close(rotated(&out)[2], v(0.0, -2.0 * c, 2.0 * s)));
    }

    #[test]
    fn the_default_plane_is_the_face_under_the_first_corner() {
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
        let mut r = Rig::new(ToolKind::RotatedRectangle, m);
        let hit = r.at(v(0.0, 1.0, 1.0), SnapKind::OnFace, Some(wall));
        r.send(ToolEvent::Click(hit));
        r.value("1,0");
        match r.tool.phase() {
            Phase::Active { points } => {
                // Horizontal in the wall.
                assert!(points[1].x.abs() < 1e-9 && (points[1].z - 1.0).abs() < 1e-9);
                assert!((points[1].distance(points[0]) - 1.0).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bad_typed_values_explain_why() {
        let mut r = rig();
        assert!(r.value("2").message.is_some());
        r.click(v(0.0, 0.0, 0.0));
        for bad in ["x", "0", "2,x", "2,3,4", ""] {
            let out = r.value(bad);
            assert!(out.commands.is_empty() && out.message.is_some(), "{bad:?}");
        }
        r.value("2,0");
        for bad in ["x", "0", "1,x", "1,2,3"] {
            let out = r.value(bad);
            assert!(out.commands.is_empty() && out.message.is_some(), "{bad:?}");
        }
        assert!(matches!(r.tool.phase(), Phase::Active { ref points } if points.len() == 2));
    }

    #[test]
    fn escape_steps_back() {
        let mut r = rig();
        r.click(v(0.0, 0.0, 0.0));
        r.click(v(2.0, 0.0, 0.0));
        r.send(ToolEvent::Cancel);
        assert!(matches!(r.tool.phase(), Phase::Active { ref points } if points.len() == 1));
        r.send(ToolEvent::Cancel);
        assert_eq!(r.tool.phase(), Phase::Idle);
    }

    #[test]
    fn a_drag_is_the_first_side() {
        let mut r = rig();
        r.click(v(0.0, 0.0, 0.0));
        r.mv(v(2.0, 0.0, 0.0));
        r.release(v(2.0, 0.0, 0.0));
        assert!(matches!(r.tool.phase(), Phase::Active { ref points } if points.len() == 2));
        // A release without a drag leaves the corner.
        let mut r = rig();
        r.click(v(0.0, 0.0, 0.0));
        r.release(v(0.0, 0.0, 0.0));
        assert!(matches!(r.tool.phase(), Phase::Active { ref points } if points.len() == 1));
    }
}
