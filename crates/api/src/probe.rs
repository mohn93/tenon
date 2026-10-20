//! The `pick` and `infer` queries: what lies under a pixel of a view or along a ray, and what
//! the tools' inference would snap to there. Pixels and cameras are the view's (root
//! coordinates, like `render_view`); points, rays and results are in the open context's
//! coordinates, like commands.

use glam::{DVec2, DVec3};
use kernel::EntityId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use session::{
    Camera, CommandError, Document, ErrorKind, Projection, Ray, check_coordinates, unit_vector,
};
use session::{ModelPoint, SelectMode};
use tools::{Axis, Guides, Reach, Scenery, Slab, SnapKind, infer_detail, pick, select_box};

use crate::dispatch::MAX_IMAGE_SIDE;

/// How near a pixel an edge or guide must pass for `pick` to meet it, by default, in pixels.
pub const PICK_RADIUS: f64 = 4.0;

/// How far from a pixel `infer` snaps, by default, in pixels: the window's radius on a
/// display of scale 1.
pub const INFER_RADIUS: f64 = 8.0;

/// The largest pixel radius `pick` takes.
pub const MAX_RADIUS: f64 = 1000.0;

/// The largest pixel radius `infer` takes: inference pairs every line within twice the
/// radius, so its cost grows with the square of the lines it reaches.
pub const MAX_INFER_RADIUS: f64 = 100.0;

/// A camera for one request, as `set_camera` takes it: absent fields keep the document
/// camera's values. It never moves the document camera.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct View {
    #[schemars(with = "ModelPoint")]
    pub eye: [f64; 3],
    #[schemars(with = "ModelPoint")]
    pub target: [f64; 3],
    pub fov_y_deg: Option<f64>,
    pub up: Option<[f64; 3]>,
    pub projection: Option<Projection>,
}

impl View {
    /// The document camera moved to this view.
    pub fn camera(&self, doc: &Document) -> Result<Camera, CommandError> {
        doc.camera_for(
            self.eye,
            self.target,
            self.fov_y_deg,
            self.up,
            self.projection,
        )
    }
}

/// A ray in the open context's coordinates.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RaySpec {
    #[schemars(with = "ModelPoint")]
    pub origin: [f64; 3],
    /// Any finite, non-zero length.
    pub direction: [f64; 3],
}

/// What lies under a pixel or along a ray.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PickQuery {
    /// A point of the view, in pixels from its top left corner (x right, y down). Give it or
    /// `ray`.
    pub pixel: Option<[f64; 2]>,
    /// The view's size in pixels (both or neither; default: the document camera's viewport,
    /// the window's 3D view).
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The camera seeing the view (default: the document camera).
    pub camera: Option<View>,
    pub ray: Option<RaySpec>,
    /// How near edges and guides must pass: pixels on screen for a `pixel` (default 4), meters
    /// from the ray for a `ray` (default 0: faces only).
    pub radius: Option<f64>,
}

/// What the tools' inference would snap to under a pixel, or at a model point seen in a view.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InferQuery {
    /// A point of the view, in pixels from its top left corner. Give it or `point`.
    pub pixel: Option<[f64; 2]>,
    /// A model point, looked for where the view shows it.
    #[schemars(with = "Option<ModelPoint>")]
    pub point: Option<[f64; 3]>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub camera: Option<View>,
    /// The point a tool is drawing from, as after a tool's first click: snaps along the axes
    /// from it are offered.
    #[schemars(with = "Option<ModelPoint>")]
    pub from: Option<[f64; 3]>,
    /// How far from the pixel to snap, in pixels (default 8).
    pub radius: Option<f64>,
}

/// An entity and where it is: `path` lists the instances from the root down to the one whose
/// group or component holds it (empty for the root's own entities), as `open_path` takes it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Placed {
    pub id: EntityId,
    pub path: Vec<EntityId>,
}

/// A face, edge or guide a pick met.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PickHit {
    pub id: EntityId,
    pub path: Vec<EntityId>,
    /// Where the ray meets a face, or the point of an edge or guide nearest the ray.
    pub point: [f64; 3],
    /// A face's unit normal: the direction its front side faces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal: Option<[f64; 3]>,
    /// The side of a face the ray meets: `front` or `back`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// How far along the ray `point` is, from its origin (the eye in perspective, the near
    /// end of the view in parallel).
    pub distance: f64,
}

/// What a point snapped to, from strongest to weakest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Snap {
    Endpoint,
    Midpoint,
    Origin,
    GuidePoint,
    Intersection,
    OnEdge,
    OnGuide,
    /// Along an axis through `from`.
    Axis,
    Parallel,
    Perpendicular,
    OnFace,
    /// The ground plane (z = 0 of the open context).
    Ground,
    /// Nothing: a point under the pixel at the depth of the camera's target.
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AxisName {
    X,
    Y,
    Z,
}

/// A box on screen, as the Select tool drags it.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenBox {
    /// One corner, in pixels from the view's top left (x right, y down).
    pub from: [f64; 2],
    /// The opposite corner.
    pub to: [f64; 2],
    /// Select what the box touches (the window's box dragged left), not only what lies
    /// entirely inside it (dragged right). Default false.
    #[serde(default)]
    pub crossing: bool,
    /// The view's size in pixels (both or neither; default: the document camera's viewport).
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Commands the API resolves from the view before running them as session commands.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScreenCommand {
    /// `select` by a box on screen: what the Select tool's box selects of the open context,
    /// seen by the document camera, combined with the selection by `mode`.
    Select {
        #[serde(rename = "box")]
        screen_box: ScreenBox,
        #[serde(default)]
        mode: SelectMode,
    },
}

/// The ids a screen box selects (see `ScreenCommand::Select`).
pub fn box_ids(doc: &Document, b: &ScreenBox) -> Result<Vec<EntityId>, CommandError> {
    let camera = view_camera(doc, None, b.width, b.height)?;
    let from = pixel_in(&camera, b.from)?;
    let to = pixel_in(&camera, b.to)?;
    let (scenery, local) = (scenery(doc), local(doc, &camera));
    Ok(select_box(&scenery, &local, from, to, b.crossing, &[]))
}

/// What the tools' inference snapped to.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Inferred {
    pub kind: Snap,
    /// For `axis`: which axis through `from` the point is held to (what Shift would lock).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<AxisName>,
    pub point: [f64; 3],
    /// What the point snapped to: the vertex of an endpoint; the edge of a midpoint or of a
    /// point on an edge; the guide of a guide point or of a point on a guide line; the two
    /// edges or guide lines crossing at an intersection; the face of a point on a face.
    pub entities: Vec<Placed>,
}

/// The camera seeing a view of `size` pixels (default: the document camera's viewport) from
/// `view` (default: the document camera).
fn view_camera(
    doc: &Document,
    view: Option<&View>,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<Camera, CommandError> {
    let mut camera = match view {
        Some(v) => v.camera(doc)?,
        None => doc.camera.clone(),
    };
    match (width, height) {
        (None, None) => {}
        (Some(w), Some(h)) => {
            let sides = 1..=MAX_IMAGE_SIDE;
            if !sides.contains(&w) || !sides.contains(&h) {
                return Err(invalid(format!(
                    "width and height must be between 1 and {MAX_IMAGE_SIDE}"
                )));
            }
            camera.viewport = DVec2::new(w as f64, h as f64);
        }
        _ => return Err(invalid("give both width and height, or neither")),
    }
    Ok(camera)
}

/// `pixel` checked to lie in `camera`'s view.
fn pixel_in(camera: &Camera, pixel: [f64; 2]) -> Result<DVec2, CommandError> {
    let p = DVec2::from(pixel);
    let size = camera.viewport;
    if p.is_finite() && p.x >= 0.0 && p.y >= 0.0 && p.x <= size.x && p.y <= size.y {
        Ok(p)
    } else {
        Err(invalid(format!(
            "pixel must lie in the view, 0 to {} across and 0 to {} down",
            size.x, size.y
        )))
    }
}

/// A pixel radius: `radius`, or `default` when absent, at most `max`.
fn pixel_radius(radius: Option<f64>, default: f64, max: f64) -> Result<f64, CommandError> {
    let r = radius.unwrap_or(default);
    if r.is_finite() && (0.0..=max).contains(&r) {
        Ok(r)
    } else {
        Err(invalid(format!(
            "radius must be between 0 and {max} pixels"
        )))
    }
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

/// Everything placed, seen from the open context.
fn scenery(doc: &Document) -> Scenery<'_> {
    Scenery::of(doc.scene(), doc.context()).expect("the open context exists")
}

/// `camera` (in root coordinates) carried into the open context's.
fn local(doc: &Document, camera: &Camera) -> Camera {
    camera.placed(doc.context_transform().inverse())
}

fn path_of(scenery: &Scenery, part: usize) -> Vec<EntityId> {
    scenery
        .path(part)
        .iter()
        .map(|i| EntityId::Instance(*i))
        .collect()
}

/// Answers `pick`: every face the ray meets, front to back, and the edges and guides passing
/// within the radius.
pub fn pick_query(doc: &Document, q: &PickQuery) -> Result<Vec<PickHit>, CommandError> {
    let hits = match (&q.pixel, &q.ray) {
        (Some(pixel), None) => {
            let camera = view_camera(doc, q.camera.as_ref(), q.width, q.height)?;
            let cursor = pixel_in(&camera, *pixel)?;
            let radius_px = pixel_radius(q.radius, PICK_RADIUS, MAX_RADIUS)?;
            let (scenery, camera) = (scenery(doc), local(doc, &camera));
            let ray = camera.ray(cursor);
            let reach = Reach::Screen {
                camera: &camera,
                cursor,
                radius_px,
            };
            (pick(&scenery, &ray, reach), scenery, ray)
        }
        (None, Some(spec)) => {
            if q.camera.is_some() || q.width.is_some() || q.height.is_some() {
                return Err(invalid(
                    "camera, width and height go with a pixel, not a ray",
                ));
            }
            check_coordinates(&[spec.origin])?;
            let dir = unit_vector(DVec3::from(spec.direction))
                .ok_or_else(|| invalid("direction must be a finite, non-zero vector"))?;
            let radius = q.radius.unwrap_or(0.0);
            if !(radius.is_finite() && radius >= 0.0) {
                return Err(invalid(
                    "radius must be a finite number of meters, at least 0",
                ));
            }
            let scenery = scenery(doc);
            let ray = Ray {
                origin: DVec3::from(spec.origin),
                dir,
            };
            (pick(&scenery, &ray, Reach::Around { radius }), scenery, ray)
        }
        _ => return Err(invalid("give a pixel or a ray")),
    };
    let (hits, scenery, ray) = hits;
    Ok(hits
        .into_iter()
        .map(|h| PickHit {
            id: h.id,
            path: path_of(&scenery, h.part),
            point: h.point.to_array(),
            normal: h.normal.map(|n| n.to_array()),
            side: h.normal.map(|n| {
                if n.dot(ray.dir) <= 0.0 {
                    "front"
                } else {
                    "back"
                }
                .to_string()
            }),
            distance: h.distance,
        })
        .collect())
}

/// The name agents see for a snap, and the axis of an axis snap.
pub fn snap_name(kind: SnapKind) -> (Snap, Option<AxisName>) {
    match kind {
        SnapKind::Endpoint => (Snap::Endpoint, None),
        SnapKind::Midpoint => (Snap::Midpoint, None),
        SnapKind::Origin => (Snap::Origin, None),
        SnapKind::GuidePoint => (Snap::GuidePoint, None),
        SnapKind::Intersection => (Snap::Intersection, None),
        SnapKind::OnEdge => (Snap::OnEdge, None),
        SnapKind::OnGuide => (Snap::OnGuide, None),
        SnapKind::Axis(a) => (
            Snap::Axis,
            Some(match a {
                Axis::X => AxisName::X,
                Axis::Y => AxisName::Y,
                Axis::Z => AxisName::Z,
            }),
        ),
        SnapKind::Parallel => (Snap::Parallel, None),
        SnapKind::Perpendicular => (Snap::Perpendicular, None),
        SnapKind::OnFace => (Snap::OnFace, None),
        SnapKind::Ground => (Snap::Ground, None),
        SnapKind::Free => (Snap::Nothing, None),
    }
}

/// Answers `infer`: what a tool's click at the pixel (or where the view shows the point)
/// would snap to.
pub fn infer_query(doc: &Document, q: &InferQuery) -> Result<Inferred, CommandError> {
    let camera = view_camera(doc, q.camera.as_ref(), q.width, q.height)?;
    let (scenery, local) = (scenery(doc), local(doc, &camera));
    let cursor = match (&q.pixel, &q.point) {
        (Some(pixel), None) => pixel_in(&camera, *pixel)?,
        (None, Some(point)) => {
            check_coordinates(&[*point])?;
            local
                .project(DVec3::from(*point))
                .filter(|p| pixel_in(&camera, p.to_array()).is_ok())
                .ok_or_else(|| invalid("the point is not in view"))?
        }
        _ => return Err(invalid("give a pixel or a point")),
    };
    if let Some(from) = q.from {
        check_coordinates(&[from])?;
    }
    let radius_px = pixel_radius(q.radius, INFER_RADIUS, MAX_INFER_RADIUS)?;
    let guides = Guides {
        anchor: q.from.map(DVec3::from),
        ..Guides::default()
    };
    // A point is looked for where it is, whatever hides it.
    let scenery = match q.point {
        Some(p) => scenery.within(Slab::around(&local, DVec3::from(p), radius_px)),
        None => scenery,
    };
    let (hit, involved) = infer_detail(&scenery, &local, cursor, &guides, radius_px);
    let (kind, axis) = snap_name(hit.kind);
    Ok(Inferred {
        kind,
        axis,
        point: hit.point.to_array(),
        entities: involved
            .into_iter()
            .map(|(k, id)| Placed {
                id,
                path: path_of(&scenery, k),
            })
            .collect(),
    })
}
