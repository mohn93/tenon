//! A line drawing of the view: the edges the camera sees, hidden lines removed, on the CPU.
//!
//! The model is walked placement by placement (the root, then every shown instance with its
//! transform, as `Scene::triangles` walks it). An edge is clipped by the placement's section
//! planes as the renderer does, cut into pieces of about 2 px on screen, and each piece is
//! visible when the line of sight to its middle meets no face (and no section fill) nearer.
//! Consecutive visible pieces become one polyline. Section cuts add their fill polygons and
//! outlines; dimensions and notes add their lines and label text.

use glam::{DAffine3, DVec2, DVec3};
use kernel::{
    EntityId, MeshId, Model, Scene, SectionPlane, TagId, section_loops, triangulate_section,
};

use crate::annotation_shape::AnnotationShape;
use crate::camera::Camera;
use crate::command::{CommandError, ErrorKind};
use crate::document::{Document, note_label};

/// Most pieces a drawing tests for visibility, over all its edges.
pub const MAX_PIECES: usize = 2_000_000;
/// How long a piece is on screen, in pixels.
pub const PIECE_PX: f64 = 2.0;
/// Most pieces one edge is cut into.
pub const MAX_PIECES_PER_EDGE: usize = 256;
/// How far short of a point a face may meet its line of sight, as a share of the distance,
/// and still not hide it (the faces an edge lies on meet it there).
const SIGHT_TOL: f64 = 1e-6;

/// Where a label sits on its position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelAnchor {
    /// The text is centered on the position (dimensions and leader notes).
    Center,
    /// The position is the text's top left corner (screen notes).
    TopLeft,
}

/// The text of a dimension or a note at a place on the view.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawingLabel {
    pub text: String,
    /// In view pixels, from the top left.
    pub at: DVec2,
    pub anchor: LabelAnchor,
}

/// What the view shows, in view pixels (origin top left, y down).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawing {
    /// The view's size in pixels.
    pub size: DVec2,
    /// The fill of closed section cuts, as triangles.
    pub fills: Vec<[DVec2; 3]>,
    /// The visible stretches of the model's edges, each a polyline.
    pub lines: Vec<Vec<DVec2>>,
    /// The visible stretches of the outlines section cuts leave.
    pub cut_lines: Vec<Vec<DVec2>>,
    /// The lines of dimensions and leader notes (not hidden by anything).
    pub annotation_lines: Vec<[DVec2; 2]>,
    /// The text of dimensions and notes in view.
    pub labels: Vec<DrawingLabel>,
    /// How many pieces were tested for visibility.
    pub tested_pieces: usize,
}

impl Drawing {
    /// True when nothing is drawn.
    pub fn is_empty(&self) -> bool {
        self.fills.is_empty()
            && self.lines.is_empty()
            && self.cut_lines.is_empty()
            && self.annotation_lines.is_empty()
            && self.labels.is_empty()
    }
}

/// A mesh as placed in the root, with the section planes that cut it (its own active plane and
/// every ancestor's, in root coordinates).
struct Placement<'a> {
    mesh: MeshId,
    model: &'a Model,
    place: DAffine3,
    /// Root coordinates to the mesh's own; None for a placement that flattens space.
    back: Option<DAffine3>,
    planes: Vec<SectionPlane>,
}

impl Placement<'_> {
    fn kept(&self, p: DVec3) -> bool {
        self.planes.iter().all(|plane| plane.keeps(p))
    }
}

fn placements(scene: &Scene, cuts: bool) -> Vec<Placement<'_>> {
    fn walk<'a>(
        scene: &'a Scene,
        mesh: MeshId,
        place: DAffine3,
        mut planes: Vec<SectionPlane>,
        cuts: bool,
        out: &mut Vec<Placement<'a>>,
    ) {
        let Some(model) = scene.mesh(mesh) else {
            return;
        };
        if cuts && let Some(plane) = model.active_section().and_then(|s| model.section(s)) {
            planes.push(plane.transformed(place));
        }
        let back = (place.matrix3.determinant().abs() > 1e-18).then(|| place.inverse());
        out.push(Placement {
            mesh,
            model,
            place,
            back,
            planes: planes.clone(),
        });
        for (_, instance) in model.instances().filter(|(_, i)| scene.tag_shown(i.tag)) {
            walk(
                scene,
                MeshId::Definition(instance.definition),
                place * instance.transform,
                planes.clone(),
                cuts,
                out,
            );
        }
    }
    let mut out = Vec::new();
    walk(
        scene,
        MeshId::Root,
        DAffine3::IDENTITY,
        Vec::new(),
        cuts,
        &mut out,
    );
    out
}

/// The part of the segment a-b that every plane keeps (the renderer's rule: a piece that only
/// touches a plane from the cut-away side is dropped).
fn clip_segment(a: DVec3, b: DVec3, planes: &[SectionPlane]) -> Option<[DVec3; 2]> {
    const TOL: f64 = 1e-9;
    let dist = |plane: &SectionPlane, p: DVec3| (p - plane.point).dot(plane.normal);
    let (mut p, mut q) = (a, b);
    let mut clipped = false;
    for plane in planes {
        let (dp, dq) = (dist(plane, p), dist(plane, q));
        match (dp >= -TOL, dq >= -TOL) {
            (true, true) => {}
            (false, false) => return None,
            (true, false) => {
                if dp <= TOL {
                    return None;
                }
                q = p.lerp(q, dp / (dp - dq));
                clipped = true;
            }
            (false, true) => {
                if dq <= TOL {
                    return None;
                }
                p = p.lerp(q, dp / (dp - dq));
                clipped = true;
            }
        }
    }
    (!clipped || p.distance(q) > 1e-9).then_some([p, q])
}

/// The triangles that remain of `tri` after every plane cut it.
fn clip_triangle(tri: [DVec3; 3], planes: &[SectionPlane]) -> Vec<[DVec3; 3]> {
    const TOL: f64 = 1e-9;
    let dist = |plane: &SectionPlane, p: DVec3| (p - plane.point).dot(plane.normal);
    let mut poly = tri.to_vec();
    for plane in planes {
        if poly.iter().all(|p| dist(plane, *p) >= -TOL) {
            continue;
        }
        let mut out = Vec::with_capacity(poly.len() + 1);
        for (k, &p) in poly.iter().enumerate() {
            let q = poly[(k + 1) % poly.len()];
            let (dp, dq) = (dist(plane, p), dist(plane, q));
            if dp >= -TOL {
                out.push(p);
            }
            if (dp > TOL && dq < -TOL) || (dp < -TOL && dq > TOL) {
                out.push(p.lerp(q, dp / (dp - dq)));
            }
        }
        poly = out;
        if poly.len() < 3 {
            return Vec::new();
        }
    }
    let clipped = poly.len() != 3 || poly.iter().zip(&tri).any(|(a, b)| a != b);
    (1..poly.len() - 1)
        .map(|k| [poly[0], poly[k], poly[k + 1]])
        .filter(|t| !clipped || (t[1] - t[0]).cross(t[2] - t[0]).length() > 1e-12)
        .collect()
}

/// What the planes of a placement cut out of its surface (world coordinates).
#[derive(Default)]
struct Cut {
    fill: Vec<[DVec3; 3]>,
    edges: Vec<[DVec3; 2]>,
}

/// The fill (when `fill`) and the outlines where each of `planes` cuts `triangles`, each
/// plane's cut clipped by the others (as the renderer's `cut_surface`).
fn cut_surface(triangles: &[[DVec3; 3]], planes: &[SectionPlane], fill: bool) -> Cut {
    let mut cut = Cut::default();
    for (k, plane) in planes.iter().enumerate() {
        let others: Vec<SectionPlane> = planes
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != k)
            .map(|(_, p)| *p)
            .collect();
        // A plane that cuts nothing away leaves nothing to outline.
        if triangles
            .iter()
            .flatten()
            .all(|p| (*p - plane.point).dot(plane.normal) >= -1e-9)
        {
            continue;
        }
        let loops = section_loops(triangles, plane.point, plane.normal);
        if fill {
            for t in triangulate_section(&loops.closed, plane.normal) {
                cut.fill.extend(clip_triangle(t, &others));
            }
        }
        let mut outline = |points: &[DVec3], closed: bool| {
            let n = if closed {
                points.len()
            } else {
                points.len() - 1
            };
            for j in 0..n {
                let (a, b) = (points[j], points[(j + 1) % points.len()]);
                if let Some(s) = clip_segment(a, b, &others) {
                    cut.edges.push(s);
                }
            }
        };
        for region in &loops.closed {
            for ring in region {
                outline(ring, true);
            }
        }
        for chain in &loops.open {
            outline(chain, false);
        }
    }
    cut
}

/// A fill triangle with its footprint on screen, for the sight tests.
struct FillTriangle {
    world: [DVec3; 3],
    lo: DVec2,
    hi: DVec2,
}

/// What can hide a point: the faces of every placement and the section fill.
struct Occluders<'a> {
    scene: &'a Scene,
    placements: &'a [Placement<'a>],
    fills: Vec<FillTriangle>,
}

impl Occluders<'_> {
    /// True when something lies between `from` and `to` (nearer than `to` by `SIGHT_TOL` of
    /// the distance); `screen` is where `to` is on screen.
    fn hides(&self, from: DVec3, to: DVec3, screen: DVec2) -> bool {
        let dir = to - from;
        let reach = 1.0 - SIGHT_TOL;
        for part in self.placements {
            let Some(back) = part.back else { continue };
            let hit = part.model.index().ray_hit_at(
                back.transform_point3(from),
                back.transform_vector3(dir),
                0.0,
                reach,
                |f, t| {
                    part.model
                        .face(f)
                        .is_some_and(|f| self.scene.tag_shown(f.tag))
                        && part.kept(from + dir * t)
                },
            );
            if hit.is_some() {
                return true;
            }
        }
        self.fills.iter().any(|f| {
            screen.cmpge(f.lo - 1.0).all()
                && screen.cmple(f.hi + 1.0).all()
                && ray_triangle(from, dir, f.world).is_some_and(|t| t > 0.0 && t <= reach)
        })
    }
}

/// The parameter at which `from + t * dir` meets the triangle (Moller-Trumbore).
fn ray_triangle(from: DVec3, dir: DVec3, [a, b, c]: [DVec3; 3]) -> Option<f64> {
    let (e1, e2) = (b - a, c - a);
    let h = dir.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = from - a;
    let u = s.dot(h) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    let eps = 1e-12;
    (u >= -eps && v >= -eps && u + v <= 1.0 + eps).then(|| e2.dot(q) / det)
}

/// The part of the segment in front of the camera's near depth (or inside a parallel view's
/// depths).
fn clip_depth(camera: &Camera, a: DVec3, b: DVec3) -> Option<[DVec3; 2]> {
    let (near, far) = camera.depth_range();
    let near = near + near.abs() * 1e-6 + 1e-9;
    let far = if far.is_finite() {
        far - far.abs() * 1e-6
    } else {
        far
    };
    let (mut p, mut q) = (a, b);
    for (bound, sign) in [(near, 1.0), (far, -1.0)] {
        if !bound.is_finite() {
            continue;
        }
        let dp = sign * (camera.depth(p) - bound);
        let dq = sign * (camera.depth(q) - bound);
        match (dp >= 0.0, dq >= 0.0) {
            (true, true) => {}
            (false, false) => return None,
            (true, false) => q = p.lerp(q, dp / (dp - dq)),
            (false, true) => p = p.lerp(q, dp / (dp - dq)),
        }
    }
    Some([p, q])
}

/// The drawing of `doc`'s model seen by `camera`, `size_px` pixels across (the camera's
/// viewport is taken to be that size). Fails when it would test more than `MAX_PIECES`
/// pieces for visibility.
pub fn line_drawing(
    doc: &Document,
    camera: &Camera,
    size_px: DVec2,
) -> Result<Drawing, CommandError> {
    let mut camera = camera.clone();
    camera.viewport = size_px;
    let scene = doc.scene();
    let display = doc.section_display;
    let parts = placements(scene, display.cuts);
    let mut drawing = Drawing {
        size: size_px,
        ..Drawing::default()
    };
    let projector = camera.projector();

    // The edges and the cuts, in root coordinates.
    let mut edges: Vec<[DVec3; 2]> = Vec::new();
    let mut cut_edges: Vec<[DVec3; 2]> = Vec::new();
    let mut fills: Vec<FillTriangle> = Vec::new();
    for part in &parts {
        for (_, edge) in part.model.edges() {
            if edge.soft || !scene.tag_shown(edge.tag) {
                continue;
            }
            let (Some(a), Some(b)) = (part.model.vertex(edge.v[0]), part.model.vertex(edge.v[1]))
            else {
                continue;
            };
            let (a, b) = (
                part.place.transform_point3(a.pos),
                part.place.transform_point3(b.pos),
            );
            if let Some(kept) = clip_segment(a, b, &part.planes) {
                edges.push(kept);
            }
        }
        if part.planes.is_empty() {
            continue;
        }
        // A mirroring placement turns triangles inside out; swapping two corners restores
        // the winding.
        let mirrored = part.place.matrix3.determinant() < 0.0;
        let surface: Vec<[DVec3; 3]> = part
            .model
            .faces()
            .filter(|(_, f)| scene.tag_shown(f.tag))
            .flat_map(|(id, _)| part.model.triangulate(id).unwrap_or_default())
            .map(|t| {
                let [a, b, c] = t.map(|p| part.place.transform_point3(p));
                if mirrored { [a, c, b] } else { [a, b, c] }
            })
            .collect();
        if surface.is_empty() {
            continue;
        }
        let cut = cut_surface(&surface, &part.planes, display.fill);
        cut_edges.extend(cut.edges);
        for tri in cut.fill {
            let screen = tri.map(|p| projector.project(p));
            if let [Some(a), Some(b), Some(c)] = screen {
                drawing.fills.push([a, b, c]);
                fills.push(FillTriangle {
                    world: tri,
                    lo: a.min(b).min(c),
                    hi: a.max(b).max(c),
                });
            }
        }
    }

    let occluders = Occluders {
        scene,
        placements: &parts,
        fills,
    };
    // Count the pieces before testing any, so a model too large is refused at once.
    let mut jobs: Vec<(Segment, bool)> = Vec::new();
    let mut tested = 0usize;
    for (segments, cut) in [(&edges, false), (&cut_edges, true)] {
        for &[a, b] in segments {
            if let Some(job) = Segment::of(&camera, a, b) {
                tested += job.pieces;
                if tested > MAX_PIECES {
                    return Err(CommandError::new(
                        ErrorKind::InvalidInput,
                        "the model is too large for a line drawing at this size",
                    ));
                }
                jobs.push((job, cut));
            }
        }
    }
    for (job, cut) in jobs {
        let runs = job.visible_runs(&camera, &occluders);
        if cut {
            drawing.cut_lines.extend(runs);
        } else {
            drawing.lines.extend(runs);
        }
    }
    drawing.tested_pieces = tested;

    annotations(doc, &camera, &parts, &mut drawing);
    Ok(drawing)
}

/// A segment in front of the camera and in view, cut into the pieces tested for visibility.
struct Segment {
    a: DVec3,
    b: DVec3,
    pieces: usize,
}

impl Segment {
    /// The part of a-b the camera sees; None when nothing of it is in view or it is seen end on.
    fn of(camera: &Camera, a: DVec3, b: DVec3) -> Option<Segment> {
        let [a, b] = clip_depth(camera, a, b)?;
        let projector = camera.projector();
        let (pa, pb) = (projector.project(a)?, projector.project(b)?);
        let length = pa.distance(pb);
        let (lo, hi) = (pa.min(pb), pa.max(pb));
        if length < 1e-9 || !(lo.cmple(camera.viewport).all() && hi.cmpge(DVec2::ZERO).all()) {
            return None;
        }
        let pieces = ((length / PIECE_PX).ceil() as usize).clamp(1, MAX_PIECES_PER_EDGE);
        Some(Segment { a, b, pieces })
    }

    /// The visible stretches as polylines on screen.
    fn visible_runs(&self, camera: &Camera, occluders: &Occluders<'_>) -> Vec<Vec<DVec2>> {
        let projector = camera.projector();
        let (a, b, pieces) = (self.a, self.b, self.pieces);
        let at = |i: usize| a.lerp(b, i as f64 / pieces as f64);
        let mut runs = Vec::new();
        let mut start: Option<usize> = None;
        let finish = |from: usize, to: usize, runs: &mut Vec<Vec<DVec2>>| {
            if let (Some(p), Some(q)) = (projector.project(at(from)), projector.project(at(to))) {
                runs.push(vec![p, q]);
            }
        };
        for i in 0..pieces {
            let mid = a.lerp(b, (i as f64 + 0.5) / pieces as f64);
            let visible = projector
                .project(mid)
                .is_some_and(|screen| !occluders.hides(camera.sight_origin(mid), mid, screen));
            match (visible, start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    finish(s, i, &mut runs);
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            finish(s, pieces, &mut runs);
        }
        runs
    }
}

/// Adds the lines and labels of the dimensions and notes of every shown placement. Labels are
/// measured on the placed shapes; a leader note without text shows its anchor's coordinates
/// in root coordinates; screen notes (the root's only) sit at their fractions of the view.
fn annotations(doc: &Document, camera: &Camera, parts: &[Placement<'_>], drawing: &mut Drawing) {
    let scene = doc.scene();
    let units = doc.units();
    let projector = camera.projector();
    let size = camera.viewport;
    let in_view = |p: DVec2| p.cmpge(DVec2::ZERO).all() && p.cmple(size).all();
    let shown = |tag: Option<TagId>| scene.tag_shown(tag);
    for part in parts {
        let model = part.model;
        let shapes = model
            .dimensions()
            .filter(|(_, d)| shown(d.tag()))
            .map(|(id, _)| EntityId::Dimension(id))
            .chain(
                model
                    .notes()
                    .filter(|(_, n)| shown(n.tag()))
                    .map(|(id, _)| EntityId::Note(id)),
            );
        for id in shapes {
            let Some(shape) = AnnotationShape::of(model, id) else {
                continue;
            };
            let shape = shape.transformed(part.place);
            shape.for_each_segment(|[a, b]| {
                if let Some([a, b]) = clip_depth(camera, a, b)
                    && let (Some(a), Some(b)) = (projector.project(a), projector.project(b))
                {
                    drawing.annotation_lines.push([a, b]);
                }
            });
            let text = match id {
                EntityId::Dimension(_) => shape.dimension_label(&units),
                EntityId::Note(n) => note_label(model, n, &units, part.place),
                _ => None,
            };
            let Some(text) = text else { continue };
            let (at, anchor) = match (shape.label_point(), shape) {
                (Some(p), _) => {
                    let Some(at) = projector.project(p).filter(|at| in_view(*at)) else {
                        continue;
                    };
                    (at, LabelAnchor::Center)
                }
                (None, AnnotationShape::Screen { at }) => {
                    // Only the root holds screen notes.
                    if part.mesh != MeshId::Root {
                        continue;
                    }
                    (DVec2::new(at[0], at[1]) * size, LabelAnchor::TopLeft)
                }
                (None, _) => continue,
            };
            drawing.labels.push(DrawingLabel { text, at, anchor });
        }
    }
}
