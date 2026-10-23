//! The Rectangle tool: click a corner, then the opposite corner (or type "width,height"); a drag
//! from the first corner to the opposite one works as the two clicks. The rectangle lies on the
//! face under the first click, or on a horizontal plane. A first corner on an edge or a point
//! rather than inside a face (a wall's foot) can start a rectangle on any face through it: the
//! rectangle lies on the face the cursor moves across, as in SketchUp.

use glam::{DMat3, DVec3};
use kernel::geom::LENGTH_TOL;
use session::{Command, Units, rectangle_axes_in, rectangle_corners};

use crate::inference::{Inference, SnapKind};
use crate::tool::{DragStart, Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, is_point_snap, parse_lengths, snap_length, toward_cursor};

#[derive(Clone, Debug, Default)]
pub struct RectangleTool {
    /// The drawing axes the last event saw (the sides follow them on axis planes).
    axes: DMat3,
    /// First corner and the normal of the plane the rectangle lies in.
    start: Option<(DVec3, DVec3)>,
    /// The first corner was not inside a face: the plane follows the face under the cursor
    /// when that face's plane holds the corner.
    floating: bool,
    /// Opposite corner under the cursor, on that plane.
    hover: Option<DVec3>,
    drag_start: DragStart,
}

impl RectangleTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The command for the rectangle from the first corner to `opposite`, if it has an area.
    fn finish(&mut self, opposite: DVec3, axes: DMat3, out: &mut ToolOutput) {
        let Some((corner, normal)) = self.start else {
            return;
        };
        if rectangle_corners(axes, corner, opposite, Some(normal)).is_ok() {
            out.commands.push(Command::AddRectangle {
                corner: corner.to_array(),
                opposite: opposite.to_array(),
                normal: Some(normal.to_array()),
            });
            self.start = None;
            self.hover = None;
        }
    }

    /// With a floating first corner, takes the plane of the face under the cursor when that
    /// plane holds the corner.
    fn follow(&mut self, hit: &Inference, ctx: &ToolContext) {
        let Some((corner, normal)) = self.start.as_mut() else {
            return;
        };
        if !self.floating {
            return;
        }
        if let Some(face) = hit.face.and_then(|id| ctx.model.face(id)) {
            let origin = ctx.model.position(face.outer[0]).unwrap();
            if (*corner - origin).dot(face.normal).abs() < LENGTH_TOL {
                *normal = face.normal;
            }
        }
    }

    /// The cursor's point `p` on the plane with its width and height rounded to the model's
    /// length snapping, unless a point of the model was snapped to.
    fn snapped(&self, p: DVec3, kind: SnapKind, units: &Units) -> DVec3 {
        let Some((corner, normal)) = self.start else {
            return p;
        };
        if units.length_snap <= 0.0 || is_point_snap(kind) {
            return p;
        }
        let (u, v) = rectangle_axes_in(self.axes, normal);
        let d = p - corner;
        corner + u * snap_length(d.dot(u), units) + v * snap_length(d.dot(v), units)
    }

    /// Width and height along the rectangle axes (on axis planes, the drawing axes `axes`) from
    /// the first corner to the cursor.
    fn size(&self, axes: DMat3) -> Option<(f64, f64)> {
        let ((corner, normal), hover) = (self.start?, self.hover?);
        let (u, v) = rectangle_axes_in(axes, normal);
        Some(((hover - corner).dot(u), (hover - corner).dot(v)))
    }
}

/// The plane a rectangle started at `hit` lies in: the face under the cursor if the point is
/// on it, otherwise horizontal (the drawing axes' red-green plane).
pub(crate) fn plane_normal(hit: &Inference, ctx: &ToolContext) -> DVec3 {
    hit.face
        .and_then(|id| ctx.model.face(id))
        .filter(|f| {
            let origin = ctx.model.position(f.outer[0]).unwrap();
            (hit.point - origin).dot(f.normal).abs() < LENGTH_TOL
        })
        .map_or(ctx.scenery.axes().z_axis, |f| f.normal)
}

/// The cursor's point on the plane through `origin` with `normal`: the inferred point if it is
/// on the plane, otherwise where the cursor ray crosses the plane.
pub(crate) fn on_plane(
    origin: DVec3,
    normal: DVec3,
    hit: &Inference,
    ctx: &ToolContext,
) -> Option<DVec3> {
    if (hit.point - origin).dot(normal).abs() < LENGTH_TOL {
        return Some(hit.point);
    }
    let ray = ctx.camera.ray(ctx.cursor);
    let denom = ray.dir.dot(normal);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = (origin - ray.origin).dot(normal) / denom;
    (t > 0.0).then(|| ray.origin + ray.dir * t)
}

impl Tool for RectangleTool {
    fn name(&self) -> &'static str {
        "Rectangle"
    }

    fn prompt(&self) -> &'static str {
        match self.start {
            None => "Click the first corner",
            Some(_) => "Click the opposite corner, or type width,height",
        }
    }

    fn measurement(&self, units: &session::Units) -> (&'static str, Option<String>) {
        let size = self.size(self.axes).map(|(w, h)| {
            format!(
                "{}, {}",
                format_length(w.abs(), units),
                format_length(h.abs(), units)
            )
        });
        ("Dimensions", size)
    }

    fn anchor(&self) -> Option<DVec3> {
        self.start.map(|(corner, _)| corner)
    }

    fn phase(&self) -> Phase {
        match self.start {
            Some((corner, _)) => Phase::Active {
                points: vec![corner],
            },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        self.axes = ctx.scenery.axes().matrix3;
        match event {
            ToolEvent::Move(hit) => {
                self.follow(&hit, ctx);
                if let Some((corner, normal)) = self.start {
                    self.hover = on_plane(corner, normal, &hit, ctx)
                        .map(|p| self.snapped(p, hit.kind, &ctx.units));
                }
            }
            ToolEvent::Click(hit) => match self.start {
                None => {
                    self.start = Some((hit.point, plane_normal(&hit, ctx)));
                    self.floating = hit.kind != SnapKind::OnFace;
                    self.drag_start.arm(ctx.cursor);
                }
                Some(_) => {
                    self.follow(&hit, ctx);
                    let (corner, normal) = self.start.unwrap();
                    if let Some(p) = on_plane(corner, normal, &hit, ctx) {
                        let p = self.snapped(p, hit.kind, &ctx.units);
                        self.finish(p, self.axes, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => {
                match (self.start, parse_lengths(&text, &ctx.units).as_deref()) {
                    (None, _) => out.message = Some("Click the first corner first".into()),
                    (Some((corner, normal)), Some(&[w, h])) => {
                        // Typed sizes extend toward the cursor; a negative size flips the side.
                        let (u, v) = rectangle_axes_in(self.axes, normal);
                        let (sw, sh) = self.size(self.axes).unwrap_or((0.0, 0.0));
                        let (w, h) = (toward_cursor(sw, w), toward_cursor(sh, h));
                        self.finish(corner + u * w + v * h, self.axes, &mut out);
                        if self.start.is_some() {
                            out.message = Some("Width and height must not be zero".into());
                        }
                    }
                    (Some(_), _) => out.message = Some(format!("Not width,height: {text}")),
                }
            }
            ToolEvent::Release(hit) => {
                if self.drag_start.released(ctx) {
                    return self.handle(ToolEvent::Click(hit), ctx);
                }
            }
            ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) | ToolEvent::ToggleCopy => {}
            ToolEvent::Cancel => self.on_external_change(),
        }
        if let (Some((corner, normal)), Some(hover)) = (self.start, self.hover)
            && let Ok(c) = rectangle_corners(self.axes, corner, hover, Some(normal))
        {
            out.preview = (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect();
        }
        out
    }

    fn on_external_change(&mut self) {
        self.start = None;
        self.hover = None;
        self.floating = false;
        self.drag_start = DragStart::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;
    use kernel::{FaceId, Model};
    use session::Camera;

    fn ground(x: f64, y: f64) -> Inference {
        Inference {
            point: DVec3::new(x, y, 0.0),
            kind: SnapKind::Ground,
            face: None,
        }
    }

    struct Rig {
        tool: RectangleTool,
        model: Model,
        camera: Camera,
        cursor: DVec2,
        axes: glam::DAffine3,
    }

    impl Rig {
        fn new(model: Model) -> Self {
            Self {
                tool: RectangleTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
                axes: glam::DAffine3::IDENTITY,
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &crate::scenery::Scenery::single(&self.model).with_axes(self.axes),
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
    }

    fn rect(corner: [f64; 3], opposite: [f64; 3], normal: DVec3) -> Command {
        Command::AddRectangle {
            corner,
            opposite,
            normal: Some(normal.to_array()),
        }
    }

    #[test]
    fn two_clicks_on_the_ground_draw_one_rectangle() {
        let mut r = Rig::new(Model::new());
        assert!(
            r.send(ToolEvent::Click(ground(1.0, 1.0)))
                .commands
                .is_empty()
        );
        let moved = r.send(ToolEvent::Move(ground(3.0, 4.0)));
        assert_eq!(moved.preview.len(), 4);
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Dimensions", Some("2 m, 3 m".to_string()))
        );
        let out = r.send(ToolEvent::Click(ground(3.0, 4.0)));
        assert_eq!(
            out.commands,
            vec![rect([1.0, 1.0, 0.0], [3.0, 4.0, 0.0], DVec3::Z)]
        );
        assert!(out.preview.is_empty());
        assert_eq!(r.tool.anchor(), None);
    }

    #[test]
    fn the_sides_follow_the_drawing_axes_on_the_ground() {
        let mut r = Rig::new(Model::new());
        // Axes turned 45 degrees about z.
        let s = std::f64::consts::FRAC_1_SQRT_2;
        r.axes = glam::DAffine3::from_mat3_translation(
            glam::DMat3::from_cols(DVec3::new(s, s, 0.0), DVec3::new(-s, s, 0.0), DVec3::Z),
            DVec3::ZERO,
        );
        r.send(ToolEvent::Click(ground(0.0, 0.0)));
        let moved = r.send(ToolEvent::Move(ground(2.0, 0.0)));
        // A square of side sqrt 2 turned with the axes.
        let corners: Vec<DVec3> = moved.preview.iter().map(|[a, _]| *a).collect();
        let expect = [
            DVec3::ZERO,
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::new(1.0, -1.0, 0.0),
        ];
        assert_eq!(corners.len(), 4);
        assert!(
            expect
                .iter()
                .all(|e| corners.iter().any(|c| c.abs_diff_eq(*e, 1e-9))),
            "{corners:?}"
        );
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Dimensions", Some("1.414 m, 1.414 m".to_string()))
        );
    }

    #[test]
    fn a_click_without_area_is_ignored() {
        let mut r = Rig::new(Model::new());
        r.send(ToolEvent::Click(ground(1.0, 1.0)));
        assert!(
            r.send(ToolEvent::Click(ground(3.0, 1.0)))
                .commands
                .is_empty()
        );
        assert!(
            r.tool.anchor().is_some(),
            "still waiting for the opposite corner"
        );
    }

    #[test]
    fn typed_size_extends_toward_the_cursor() {
        let mut r = Rig::new(Model::new());
        r.send(ToolEvent::Click(ground(1.0, 1.0)));
        r.send(ToolEvent::Move(ground(0.0, 3.0)));
        let out = r.send(ToolEvent::Value("2,500mm".into()));
        assert_eq!(
            out.commands,
            vec![rect([1.0, 1.0, 0.0], [-1.0, 1.5, 0.0], DVec3::Z)]
        );
    }

    #[test]
    fn bad_typed_sizes_explain_why() {
        let mut r = Rig::new(Model::new());
        assert!(r.send(ToolEvent::Value("2,3".into())).message.is_some());
        r.send(ToolEvent::Click(ground(1.0, 1.0)));
        for bad in ["2", "2,x", "2,0"] {
            let out = r.send(ToolEvent::Value(bad.into()));
            assert!(out.commands.is_empty() && out.message.is_some(), "{bad}");
        }
    }

    #[test]
    fn starts_on_the_plane_of_the_face_under_the_cursor() {
        // A wall in the plane x = 0.
        let mut m = Model::new();
        let pts = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(0.0, 4.0, 0.0),
            DVec3::new(0.0, 4.0, 3.0),
            DVec3::new(0.0, 0.0, 3.0),
        ];
        m.add_polygon(&pts).unwrap();
        let wall: FaceId = m.faces().next().unwrap().0;
        let normal = m.face(wall).unwrap().normal;
        let mut r = Rig::new(m);
        let on_wall = |y: f64, z: f64| Inference {
            point: DVec3::new(0.0, y, z),
            kind: SnapKind::OnFace,
            face: Some(wall),
        };
        r.send(ToolEvent::Click(on_wall(1.0, 1.0)));
        let out = r.send(ToolEvent::Click(on_wall(2.0, 2.5)));
        assert_eq!(
            out.commands,
            vec![rect([0.0, 1.0, 1.0], [0.0, 2.0, 2.5], normal)]
        );
    }

    #[test]
    fn a_corner_on_a_walls_foot_takes_the_plane_of_the_face_the_cursor_crosses() {
        // A floor on z = 0 and a wall in the plane y = 0 standing on its front edge.
        let mut m = Model::new();
        m.add_polygon(&[
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(4.0, 0.0, 0.0),
            DVec3::new(4.0, 3.0, 0.0),
            DVec3::new(0.0, 3.0, 0.0),
        ])
        .unwrap();
        m.add_polygon(&[
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(4.0, 0.0, 0.0),
            DVec3::new(4.0, 0.0, 3.0),
            DVec3::new(0.0, 0.0, 3.0),
        ])
        .unwrap();
        let face_with = |m: &Model, n: DVec3| {
            m.faces()
                .find(|(_, f)| f.normal.cross(n).length() < 1e-9)
                .unwrap()
                .0
        };
        let (floor, wall) = (face_with(&m, DVec3::Z), face_with(&m, DVec3::Y));
        let wall_normal = m.face(wall).unwrap().normal;
        let mut r = Rig::new(m);
        let at = |p: DVec3, kind: SnapKind, face: FaceId| Inference {
            point: p,
            kind,
            face: Some(face),
        };
        // The first corner on the shared edge, seen over the floor.
        r.send(ToolEvent::Click(at(
            DVec3::new(1.0, 0.0, 0.0),
            SnapKind::OnEdge,
            floor,
        )));
        // Across the wall the rectangle stands in the wall.
        r.send(ToolEvent::Move(at(
            DVec3::new(2.0, 0.0, 2.0),
            SnapKind::OnFace,
            wall,
        )));
        let out = r.send(ToolEvent::Value("1,2".into()));
        assert_eq!(out.commands.len(), 1);
        let Command::AddRectangle {
            corner,
            opposite,
            normal,
        } = &out.commands[0]
        else {
            unreachable!()
        };
        assert_eq!(*normal, Some(wall_normal.to_array()));
        assert_eq!(*corner, [1.0, 0.0, 0.0]);
        assert!((opposite[1]).abs() < 1e-12 && (opposite[2] - 2.0).abs() < 1e-12);
        // Back over the floor, the next one lies on the floor.
        r.send(ToolEvent::Click(at(
            DVec3::new(1.0, 0.0, 0.0),
            SnapKind::OnEdge,
            wall,
        )));
        let out = r.send(ToolEvent::Click(at(
            DVec3::new(2.0, 1.0, 0.0),
            SnapKind::OnFace,
            floor,
        )));
        let floor_normal = r.model.face(floor).unwrap().normal;
        assert_eq!(
            out.commands,
            vec![rect([1.0, 0.0, 0.0], [2.0, 1.0, 0.0], floor_normal)]
        );
        // A corner inside a face keeps that face's plane.
        r.send(ToolEvent::Click(at(
            DVec3::new(1.0, 1.0, 0.0),
            SnapKind::OnFace,
            floor,
        )));
        r.send(ToolEvent::Move(at(
            DVec3::new(2.0, 0.0, 2.0),
            SnapKind::OnFace,
            wall,
        )));
        assert_eq!(r.tool.start.unwrap().1, floor_normal);
    }

    #[test]
    fn off_plane_cursor_is_projected_through_the_view_ray() {
        let mut r = Rig::new(Model::new());
        r.send(ToolEvent::Click(ground(0.0, 0.0)));
        let target = DVec3::new(2.0, 1.0, 0.0);
        r.cursor = r.camera.project(target).unwrap();
        // An axis inference above the plane: the corner comes from the cursor ray instead.
        let above = Inference {
            point: DVec3::new(0.0, 0.0, 2.0),
            kind: SnapKind::Axis(crate::Axis::Z),
            face: None,
        };
        r.send(ToolEvent::Move(above));
        let (w, h) = r.tool.size(DMat3::IDENTITY).unwrap();
        assert!((w - 2.0).abs() < 1e-6 && (h - 1.0).abs() < 1e-6);
    }
}
