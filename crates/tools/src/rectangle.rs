//! The Rectangle tool: click a corner, then the opposite corner (or type "width,height"). The
//! rectangle lies on the face under the first click, or on a horizontal plane.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::{Command, rectangle_axes, rectangle_corners};

use crate::inference::Inference;
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_lengths, toward_cursor};

#[derive(Clone, Debug, Default)]
pub struct RectangleTool {
    /// First corner and the normal of the plane the rectangle lies in.
    start: Option<(DVec3, DVec3)>,
    /// Opposite corner under the cursor, on that plane.
    hover: Option<DVec3>,
}

impl RectangleTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The command for the rectangle from the first corner to `opposite`, if it has an area.
    fn finish(&mut self, opposite: DVec3, out: &mut ToolOutput) {
        let Some((corner, normal)) = self.start else {
            return;
        };
        if rectangle_corners(corner, opposite, Some(normal)).is_ok() {
            out.commands.push(Command::AddRectangle {
                corner: corner.to_array(),
                opposite: opposite.to_array(),
                normal: Some(normal.to_array()),
            });
            self.start = None;
            self.hover = None;
        }
    }

    /// Width and height along the rectangle axes from the first corner to the cursor.
    fn size(&self) -> Option<(f64, f64)> {
        let ((corner, normal), hover) = (self.start?, self.hover?);
        let (u, v) = rectangle_axes(normal);
        Some(((hover - corner).dot(u), (hover - corner).dot(v)))
    }
}

/// The plane a rectangle started at `hit` lies in: the face under the cursor if the point is
/// on it, otherwise horizontal.
fn plane_normal(hit: &Inference, ctx: &ToolContext) -> DVec3 {
    hit.face
        .and_then(|id| ctx.model.face(id))
        .filter(|f| {
            let origin = ctx.model.position(f.outer[0]).unwrap();
            (hit.point - origin).dot(f.normal).abs() < LENGTH_TOL
        })
        .map_or(DVec3::Z, |f| f.normal)
}

/// The cursor's point on the plane through `origin` with `normal`: the inferred point if it is
/// on the plane, otherwise where the cursor ray crosses the plane.
fn on_plane(origin: DVec3, normal: DVec3, hit: &Inference, ctx: &ToolContext) -> Option<DVec3> {
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

    fn measurement(&self) -> (&'static str, Option<String>) {
        let size = self
            .size()
            .map(|(w, h)| format!("{}, {}", format_length(w.abs()), format_length(h.abs())));
        ("Dimensions", size)
    }

    fn anchor(&self) -> Option<DVec3> {
        self.start.map(|(corner, _)| corner)
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => {
                if let Some((corner, normal)) = self.start {
                    self.hover = on_plane(corner, normal, &hit, ctx);
                }
            }
            ToolEvent::Click(hit) => match self.start {
                None => self.start = Some((hit.point, plane_normal(&hit, ctx))),
                Some((corner, normal)) => {
                    if let Some(p) = on_plane(corner, normal, &hit, ctx) {
                        self.finish(p, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => match (self.start, parse_lengths(&text).as_deref()) {
                (None, _) => out.message = Some("Click the first corner first".into()),
                (Some((corner, normal)), Some(&[w, h])) => {
                    // Typed sizes extend toward the cursor; a negative size flips the side.
                    let (u, v) = rectangle_axes(normal);
                    let (sw, sh) = self.size().unwrap_or((0.0, 0.0));
                    let (w, h) = (toward_cursor(sw, w), toward_cursor(sh, h));
                    self.finish(corner + u * w + v * h, &mut out);
                    if self.start.is_some() {
                        out.message = Some("Width and height must not be zero".into());
                    }
                }
                (Some(_), _) => out.message = Some(format!("Not width,height: {text}")),
            },
            ToolEvent::Cancel => self.on_external_change(),
        }
        if let (Some((corner, normal)), Some(hover)) = (self.start, self.hover)
            && let Ok(c) = rectangle_corners(corner, hover, Some(normal))
        {
            out.preview = (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect();
        }
        out
    }

    fn on_external_change(&mut self) {
        self.start = None;
        self.hover = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::SnapKind;
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
    }

    impl Rig {
        fn new(model: Model) -> Self {
            Self {
                tool: RectangleTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                camera: &self.camera,
                cursor: self.cursor,
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
            r.tool.measurement(),
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
        let (w, h) = r.tool.size().unwrap();
        assert!((w - 2.0).abs() < 1e-6 && (h - 1.0).abs() < 1e-6);
    }
}
