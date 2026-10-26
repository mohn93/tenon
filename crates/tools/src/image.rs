//! The Image tool: a picture follows the cursor on the face or ground plane under it; a click
//! sets its first corner, moving sizes it keeping the image's proportions (the corner stays
//! put, the picture grows toward the cursor), and a second click or a typed width finishes. The
//! plane is the Rectangle tool's: the face under the first click, or a horizontal plane, and a
//! first corner on an edge or a point lets the plane follow the face the cursor moves across.
//! Escape ends it (the editor then goes back to the tool used before).

use glam::{DMat3, DVec3};
use kernel::geom::LENGTH_TOL;
use session::{Command, rectangle_axes_in};

use crate::inference::{Inference, SnapKind};
use crate::rectangle::{on_plane, plane_normal};
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_lengths, snap_length, snaps};

/// The width of the picture drawn at the cursor before the first click, in meters.
const FLOATING_WIDTH: f64 = 1.0;
/// The least width a click or a typed length may give, in meters.
const MIN_WIDTH: f64 = 0.001;

/// An image ready to place: what the tool needs to know of it.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageSpec {
    /// The picture's name (the file's name without its extension).
    pub name: String,
    /// The file's bytes in base64, as `import_image`'s `data` takes them.
    pub data: String,
    /// Height over width.
    pub aspect: f64,
}

#[derive(Clone, Debug)]
pub struct ImageTool {
    image: ImageSpec,
    axes: DMat3,
    /// First corner and the normal of the plane the picture lies in.
    start: Option<(DVec3, DVec3)>,
    /// The first corner was not inside a face: the plane follows the face under the cursor
    /// when that face's plane holds the corner.
    floating: bool,
    /// The picture's size corner under the cursor, on the plane (after the first click).
    hover: Option<DVec3>,
    /// Before the first click: the point and plane the picture would start at.
    ahead: Option<(DVec3, DVec3)>,
}

/// Where a picture of `aspect` (height over width) lies when one corner is at `corner` and the
/// cursor is at `cursor` on the plane with `normal`: its lower left corner, the direction of its
/// width and its width. It grows from the corner toward the cursor by whichever of the two
/// distances gives the larger picture. None below the least width.
fn placed(
    axes: DMat3,
    normal: DVec3,
    corner: DVec3,
    cursor: DVec3,
    aspect: f64,
    width: Option<f64>,
) -> Option<(DVec3, DVec3, f64)> {
    let (u, v) = rectangle_axes_in(axes, normal);
    let d = cursor - corner;
    let (dx, dy) = (d.dot(u), d.dot(v));
    let width = width.unwrap_or_else(|| dx.abs().max(dy.abs() / aspect));
    if !(width.is_finite() && width >= MIN_WIDTH) {
        return None;
    }
    let left = if dx < 0.0 { -width } else { 0.0 };
    let down = if dy < 0.0 { -width * aspect } else { 0.0 };
    Some((corner + u * left + v * down, u, width))
}

impl ImageTool {
    pub fn new(image: ImageSpec) -> Self {
        Self {
            image,
            axes: DMat3::IDENTITY,
            start: None,
            floating: false,
            hover: None,
            ahead: None,
        }
    }

    /// The command placing the picture grown from the first corner toward `cursor` (or, with
    /// `width`, that wide), if it has a size.
    fn finish(&mut self, cursor: DVec3, width: Option<f64>, out: &mut ToolOutput) {
        let Some((corner, normal)) = self.start else {
            return;
        };
        let Some((origin, x_axis, width)) =
            placed(self.axes, normal, corner, cursor, self.image.aspect, width)
        else {
            return;
        };
        out.commands.push(Command::ImportImage {
            path: None,
            data: Some(self.image.data.clone()),
            origin: origin.to_array(),
            normal: Some(normal.to_array()),
            x_axis: Some(x_axis.to_array()),
            width,
            name: Some(self.image.name.clone()),
            lift: None,
        });
        self.start = None;
        self.hover = None;
        self.floating = false;
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

    /// The cursor's point `p` with the picture's width rounded to the model's length snapping,
    /// unless a point of the model was snapped to.
    fn snapped(&self, p: DVec3, hit: &Inference, ctx: &ToolContext) -> DVec3 {
        let Some((corner, normal)) = self.start else {
            return p;
        };
        if !snaps(hit, &ctx.units) {
            return p;
        }
        match placed(self.axes, normal, corner, p, self.image.aspect, None) {
            Some((_, _, width)) => {
                let (u, v) = rectangle_axes_in(self.axes, normal);
                let d = p - corner;
                let w = snap_length(width, &ctx.units);
                corner + u * w.copysign(d.dot(u)) + v * (w * self.image.aspect).copysign(d.dot(v))
            }
            None => p,
        }
    }

    /// The picture's width from the first corner to the cursor.
    fn width(&self) -> Option<f64> {
        let ((corner, normal), hover) = (self.start?, self.hover?);
        placed(self.axes, normal, corner, hover, self.image.aspect, None).map(|(_, _, w)| w)
    }

    /// The four sides of the picture of `width` at `origin`, as a preview.
    fn outline(&self, normal: DVec3, origin: DVec3, x_axis: DVec3, width: f64) -> Vec<[DVec3; 2]> {
        let y = normal.cross(x_axis);
        let h = width * self.image.aspect;
        let c = [
            origin,
            origin + x_axis * width,
            origin + x_axis * width + y * h,
            origin + y * h,
        ];
        (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect()
    }
}

impl Tool for ImageTool {
    fn name(&self) -> &'static str {
        "Image"
    }

    fn prompt(&self) -> &'static str {
        match self.start {
            None => "Click the picture's first corner (Esc cancels)",
            Some(_) => "Click the opposite corner, or type the width",
        }
    }

    fn measurement(&self, units: &session::Units) -> (&'static str, Option<String>) {
        ("Width", self.width().map(|w| format_length(w, units)))
    }

    fn anchor(&self) -> Option<DVec3> {
        self.start.map(|(corner, _)| corner)
    }

    /// Placing the picture is an operation in progress from the start; its points are the
    /// first corner once clicked.
    fn phase(&self) -> Phase {
        Phase::Active {
            points: self.start.map(|(corner, _)| corner).into_iter().collect(),
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        self.axes = ctx.scenery.axes().matrix3;
        match event {
            ToolEvent::Move(hit) => {
                self.follow(&hit, ctx);
                if let Some((corner, normal)) = self.start {
                    self.hover =
                        on_plane(corner, normal, &hit, ctx).map(|p| self.snapped(p, &hit, ctx));
                } else {
                    self.ahead = Some((hit.point, plane_normal(&hit, ctx)));
                }
            }
            ToolEvent::Click(hit) => match self.start {
                None => {
                    self.start = Some((hit.point, plane_normal(&hit, ctx)));
                    self.floating = hit.kind != SnapKind::OnFace;
                    self.ahead = None;
                }
                Some(_) => {
                    self.follow(&hit, ctx);
                    let (corner, normal) = self.start.unwrap();
                    if let Some(p) = on_plane(corner, normal, &hit, ctx) {
                        let p = self.snapped(p, &hit, ctx);
                        self.finish(p, None, &mut out);
                    }
                }
            },
            ToolEvent::Value(text) => {
                match (self.start, parse_lengths(&text, &ctx.units).as_deref()) {
                    (None, _) => out.message = Some("Click the first corner first".into()),
                    (Some((corner, _)), Some(&[width])) if width.abs() >= MIN_WIDTH => {
                        // The picture grows toward the cursor (up and right when it is not on the
                        // plane yet).
                        let toward = self.hover.unwrap_or(corner);
                        self.finish(toward, Some(width.abs()), &mut out);
                    }
                    (Some(_), Some(&[_])) => {
                        out.message = Some("The width must not be zero".into())
                    }
                    (Some(_), _) => out.message = Some(format!("Not a width: {text}")),
                }
            }
            ToolEvent::Release(_)
            | ToolEvent::DoubleClick(_)
            | ToolEvent::TripleClick(_)
            | ToolEvent::ToggleCopy
            | ToolEvent::Cancel => {}
        }
        if out.commands.is_empty() {
            out.preview = match (self.start, self.hover, self.ahead) {
                (Some((corner, normal)), Some(hover), _) => {
                    placed(self.axes, normal, corner, hover, self.image.aspect, None)
                        .map(|(origin, x, w)| self.outline(normal, origin, x, w))
                        .unwrap_or_default()
                }
                (None, _, Some((point, normal))) => {
                    let (u, _) = rectangle_axes_in(self.axes, normal);
                    self.outline(normal, point, u, FLOATING_WIDTH)
                }
                _ => Vec::new(),
            };
        }
        out
    }

    fn on_external_change(&mut self) {
        self.start = None;
        self.hover = None;
        self.floating = false;
        self.ahead = None;
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
            locked: None,
        }
    }

    /// A 4 x 2 picture.
    fn tool() -> ImageTool {
        ImageTool::new(ImageSpec {
            name: "pic".into(),
            data: "AAAA".into(),
            aspect: 0.5,
        })
    }

    fn send(tool: &mut ImageTool, model: &Model, event: ToolEvent) -> ToolOutput {
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let ctx = ToolContext {
            model,
            scenery: &crate::scenery::Scenery::single(model),
            camera: &camera,
            cursor: DVec2::ZERO,
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
        tool.handle(event, &ctx)
    }

    fn image(out: &ToolOutput) -> (DVec3, DVec3, DVec3, f64) {
        match &out.commands[..] {
            [
                Command::ImportImage {
                    origin,
                    normal,
                    x_axis,
                    width,
                    ..
                },
            ] => (
                DVec3::from(*origin),
                DVec3::from(normal.unwrap()),
                DVec3::from(x_axis.unwrap()),
                *width,
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_picture_grows_from_the_first_corner_keeping_its_proportions() {
        let (mut t, m) = (tool(), Model::new());
        let out = send(&mut t, &m, ToolEvent::Move(ground(1.0, 1.0)));
        assert_eq!(out.preview.len(), 4, "a picture follows the cursor");
        send(&mut t, &m, ToolEvent::Click(ground(1.0, 1.0)));
        let out = send(&mut t, &m, ToolEvent::Move(ground(4.0, 2.0)));
        assert_eq!(out.preview.len(), 4);
        assert_eq!(
            t.measurement(&Default::default()),
            ("Width", Some("3 m".to_string()))
        );
        let (origin, normal, x, width) =
            image(&send(&mut t, &m, ToolEvent::Click(ground(4.0, 2.0))));
        assert_eq!(
            (origin, normal, x, width),
            (DVec3::new(1.0, 1.0, 0.0), DVec3::Z, DVec3::X, 3.0)
        );
        assert_eq!(t.anchor(), None);
    }

    #[test]
    fn moving_left_and_down_puts_the_origin_at_the_lower_left() {
        let (mut t, m) = (tool(), Model::new());
        send(&mut t, &m, ToolEvent::Click(ground(5.0, 5.0)));
        send(&mut t, &m, ToolEvent::Move(ground(3.0, 4.5)));
        let (origin, _, _, width) = image(&send(&mut t, &m, ToolEvent::Click(ground(3.0, 4.5))));
        // Two meters across wins over the half meter down: 2 x 1.
        assert_eq!((origin, width), (DVec3::new(3.0, 4.0, 0.0), 2.0));
    }

    #[test]
    fn a_typed_width_finishes_toward_the_cursor() {
        let (mut t, m) = (tool(), Model::new());
        assert!(
            send(&mut t, &m, ToolEvent::Value("2".into()))
                .message
                .is_some()
        );
        send(&mut t, &m, ToolEvent::Click(ground(0.0, 0.0)));
        send(&mut t, &m, ToolEvent::Move(ground(-1.0, 1.0)));
        let (origin, _, _, width) = image(&send(&mut t, &m, ToolEvent::Value("500mm".into())));
        assert_eq!((origin, width), (DVec3::new(-0.5, 0.0, 0.0), 0.5));
    }

    #[test]
    fn bad_typed_widths_explain_why() {
        let (mut t, m) = (tool(), Model::new());
        send(&mut t, &m, ToolEvent::Click(ground(0.0, 0.0)));
        for bad in ["0", "x", "2,3"] {
            let out = send(&mut t, &m, ToolEvent::Value(bad.into()));
            assert!(out.commands.is_empty() && out.message.is_some(), "{bad}");
        }
    }

    #[test]
    fn a_picture_started_on_a_wall_lies_in_the_walls_plane() {
        let mut m = Model::new();
        m.add_polygon(&[
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(0.0, 4.0, 0.0),
            DVec3::new(0.0, 4.0, 3.0),
            DVec3::new(0.0, 0.0, 3.0),
        ])
        .unwrap();
        let wall: FaceId = m.faces().next().unwrap().0;
        let wall_normal = m.face(wall).unwrap().normal;
        let on_wall = |y: f64, z: f64| Inference {
            point: DVec3::new(0.0, y, z),
            kind: SnapKind::OnFace,
            face: Some(wall),
            locked: None,
        };
        let mut t = tool();
        send(&mut t, &m, ToolEvent::Click(on_wall(1.0, 1.0)));
        let (origin, normal, x, width) =
            image(&send(&mut t, &m, ToolEvent::Click(on_wall(3.0, 2.0))));
        assert_eq!(
            (origin, normal, width),
            (DVec3::new(0.0, 1.0, 1.0), wall_normal, 2.0)
        );
        // The width runs along the wall's horizontal direction and the picture stands upright.
        assert!(x.dot(DVec3::Z).abs() < 1e-9 && x.dot(wall_normal).abs() < 1e-9);
    }
}
