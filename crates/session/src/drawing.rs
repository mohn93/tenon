//! A line drawing of the view: the edges the camera sees, hidden lines removed, on the CPU.
//!
//! The model is walked placement by placement (the root, then every shown instance with its
//! transform, as `Scene::triangles` walks it). An edge is clipped by the placement's section
//! planes as the renderer does, cut into pieces of about 2 px on screen, and each piece is
//! visible when the line of sight to its middle meets no face (and no section fill) nearer.
//! Consecutive visible pieces become one polyline. Section cuts add their fill polygons and
//! outlines; dimensions and notes add their lines and label text.

use glam::{DAffine3, DVec2, DVec3};
use kernel::{EntityId, MeshId, Model, Scene, SectionPlane, TagId, ray_hits_triangle};

use crate::annotation_shape::AnnotationShape;
use crate::camera::Camera;
use crate::command::{CommandError, ErrorKind};
use crate::document::{Document, note_label};
use crate::section_clip::{clip_segment, cut_surface};

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
    /// The box of the mesh's vertices as placed, when it holds faces.
    bounds: Option<[DVec3; 2]>,
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
        let bounds = model.faces().next().and_then(|_| {
            let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
            for (_, v) in model.vertices() {
                let p = place.transform_point3(v.pos);
                lo = lo.min(p);
                hi = hi.max(p);
            }
            let pad = DVec3::splat(1e-6);
            (lo.is_finite() && hi.is_finite()).then_some([lo - pad, hi + pad])
        });
        out.push(Placement {
            mesh,
            model,
            place,
            back,
            planes: planes.clone(),
            bounds,
        });
        for (_, instance) in model
            .instances()
            .filter(|(_, i)| scene.placed_instance_shown(i))
        {
            // An instance left whole is cut by none of the planes outside it.
            let outside = if instance.section_cut {
                planes.clone()
            } else {
                Vec::new()
            };
            walk(
                scene,
                MeshId::Definition(instance.definition),
                place * instance.transform,
                outside,
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

/// A fill triangle: its corners in the root.
struct FillTriangle {
    world: [DVec3; 3],
}

/// Cells across and down the view the fill triangles are bucketed in.
const GRID: usize = 32;

/// The fill triangles, bucketed by the screen cells they cover so a sight test visits only
/// those near its point. A triangle that could not be projected (a corner outside the depths
/// the camera sees) is in `always`: it can hide anything.
struct FillGrid {
    triangles: Vec<FillTriangle>,
    cells: Vec<Vec<u32>>,
    always: Vec<u32>,
    size: DVec2,
}

impl FillGrid {
    fn new(size: DVec2) -> FillGrid {
        FillGrid {
            triangles: Vec::new(),
            cells: vec![Vec::new(); GRID * GRID],
            always: Vec::new(),
            size,
        }
    }

    /// The cell column and row of a screen point, clamped to the grid (a point beyond the
    /// view shares the border cells with the triangles beyond it).
    fn cell(&self, p: DVec2) -> (usize, usize) {
        let at = |v: f64, size: f64| {
            ((v / size.max(1e-9) * GRID as f64).floor().max(0.0) as usize).min(GRID - 1)
        };
        (at(p.x, self.size.x), at(p.y, self.size.y))
    }

    /// Adds a triangle with its footprint on screen, or without one.
    fn add(&mut self, world: [DVec3; 3], screen: Option<[DVec2; 3]>) {
        let k = self.triangles.len() as u32;
        self.triangles.push(FillTriangle { world });
        let Some([a, b, c]) = screen else {
            self.always.push(k);
            return;
        };
        let ((x0, y0), (x1, y1)) = (
            self.cell(a.min(b).min(c) - 1.0),
            self.cell(a.max(b).max(c) + 1.0),
        );
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.cells[y * GRID + x].push(k);
            }
        }
    }

    /// True when a fill triangle meets `from + t * dir` for some `0 < t <= reach`; `screen`
    /// is where the far end is.
    fn hides(&self, from: DVec3, dir: DVec3, reach: f64, screen: DVec2) -> bool {
        let (x, y) = self.cell(screen);
        self.always
            .iter()
            .chain(&self.cells[y * GRID + x])
            .any(|&k| {
                ray_hits_triangle(from, dir, self.triangles[k as usize].world)
                    .is_some_and(|t| t > 0.0 && t <= reach)
            })
    }
}

/// Whether the segment `from + t * dir`, `0 <= t <= reach`, meets the box.
fn segment_meets_box(from: DVec3, dir: DVec3, reach: f64, lo: DVec3, hi: DVec3) -> bool {
    let (mut t0, mut t1) = (0.0f64, reach);
    for k in 0..3 {
        if dir[k].abs() < 1e-300 {
            if from[k] < lo[k] || from[k] > hi[k] {
                return false;
            }
            continue;
        }
        let inv = 1.0 / dir[k];
        let (mut a, mut b) = ((lo[k] - from[k]) * inv, (hi[k] - from[k]) * inv);
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return false;
        }
    }
    true
}

/// A tree over the boxes of the placements that hold faces, so a sight test walks only the
/// placements its line passes through.
struct PlacementTree {
    nodes: Vec<TreeNode>,
}

struct TreeNode {
    lo: DVec3,
    hi: DVec3,
    kids: TreeKids,
}

enum TreeKids {
    Leaf(Vec<usize>),
    Inner(usize, usize),
}

/// Most placements in a leaf.
const TREE_LEAF: usize = 4;

impl PlacementTree {
    fn new(parts: &[Placement<'_>]) -> PlacementTree {
        let mut tree = PlacementTree { nodes: Vec::new() };
        let mut items: Vec<usize> = (0..parts.len())
            .filter(|&i| parts[i].bounds.is_some() && parts[i].back.is_some())
            .collect();
        if !items.is_empty() {
            tree.build(parts, &mut items);
        }
        tree
    }

    fn build(&mut self, parts: &[Placement<'_>], items: &mut [usize]) -> usize {
        let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        for &i in items.iter() {
            let [l, h] = parts[i].bounds.expect("only placements with bounds");
            lo = lo.min(l);
            hi = hi.max(h);
        }
        let at = self.nodes.len();
        self.nodes.push(TreeNode {
            lo,
            hi,
            kids: TreeKids::Leaf(Vec::new()),
        });
        if items.len() <= TREE_LEAF {
            self.nodes[at].kids = TreeKids::Leaf(items.to_vec());
            return at;
        }
        let extent = hi - lo;
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let centre = |i: &usize| {
            let [l, h] = parts[*i].bounds.expect("only placements with bounds");
            (l[axis] + h[axis]) * 0.5
        };
        items.sort_by(|a, b| centre(a).total_cmp(&centre(b)).then(a.cmp(b)));
        let (left, right) = items.split_at_mut(items.len() / 2);
        let l = self.build(parts, left);
        let r = self.build(parts, right);
        self.nodes[at].kids = TreeKids::Inner(l, r);
        at
    }

    /// True when `hit` holds for some placement whose box the segment meets.
    fn any(
        &self,
        from: DVec3,
        dir: DVec3,
        reach: f64,
        hit: &mut impl FnMut(usize) -> bool,
    ) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n];
            if !segment_meets_box(from, dir, reach, node.lo, node.hi) {
                continue;
            }
            match &node.kids {
                TreeKids::Leaf(items) => {
                    if items.iter().any(|&i| hit(i)) {
                        return true;
                    }
                }
                TreeKids::Inner(l, r) => stack.extend([*l, *r]),
            }
        }
        false
    }
}

/// What can hide a point: the faces of every placement and the section fill.
///
/// The cost of one test is a walk of the placement tree (about log P boxes) plus, for each
/// placement whose box the line of sight meets, a walk of that mesh's own index, plus the fill
/// triangles in the point's screen cell. A drawing tests at most `MAX_PIECES` pieces.
struct Occluders<'a> {
    scene: &'a Scene,
    placements: &'a [Placement<'a>],
    tree: PlacementTree,
    fills: FillGrid,
}

impl Occluders<'_> {
    /// True when something lies between `from` and `to`, nearer to `to` than `tol` (meters)
    /// excepted; `screen` is where `to` is on screen.
    fn hides(&self, from: DVec3, to: DVec3, screen: DVec2, tol: f64) -> bool {
        let dir = to - from;
        let reach = (1.0 - tol / dir.length().max(1e-300)).max(0.0);
        let mut hit = |i: usize| {
            let part = &self.placements[i];
            let Some(back) = part.back else { return false };
            part.model
                .index()
                .ray_hit_at(
                    back.transform_point3(from),
                    back.transform_vector3(dir),
                    0.0,
                    reach,
                    |f, t| {
                        part.model.face(f).is_some_and(|f| self.scene.face_shown(f))
                            && part.kept(from + dir * t)
                    },
                )
                .is_some()
        };
        self.tree.any(from, dir, reach, &mut hit) || self.fills.hides(from, dir, reach, screen)
    }
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

    // Count the pieces before the costly work (cutting surfaces, testing sight lines), so a
    // model too large is refused at once.
    let too_large = || {
        CommandError::new(
            ErrorKind::InvalidInput,
            "the model is too large for a line drawing at this size",
        )
    };
    let mut jobs: Vec<(Segment, bool)> = Vec::new();
    let mut tested = 0usize;
    let mut add_job = |camera: &Camera, a: DVec3, b: DVec3, cut: bool| {
        if let Some(job) = Segment::of(camera, a, b) {
            tested += job.pieces;
            if tested > MAX_PIECES {
                return Err(too_large());
            }
            jobs.push((job, cut));
        }
        Ok(())
    };
    for part in &parts {
        for (_, edge) in part.model.edges() {
            if edge.soft || !scene.edge_shown(edge) {
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
            if let Some([a, b]) = clip_segment(a, b, &part.planes) {
                add_job(&camera, a, b, false)?;
            }
        }
    }
    let mut fills = FillGrid::new(size_px);
    for part in parts.iter().filter(|p| !p.planes.is_empty()) {
        // A mirroring placement turns triangles inside out; swapping two corners restores
        // the winding.
        let mirrored = part.place.matrix3.determinant() < 0.0;
        let surface: Vec<[DVec3; 3]> = part
            .model
            .faces()
            .filter(|(_, f)| scene.face_shown(f))
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
        for [a, b] in cut.edges {
            add_job(&camera, a, b, true)?;
        }
        for tri in cut.fill {
            let screen = tri.map(|p| projector.project(p));
            if let [Some(a), Some(b), Some(c)] = screen {
                drawing.fills.push([a, b, c]);
                fills.add(tri, Some([a, b, c]));
            } else {
                fills.add(tri, None);
            }
        }
    }
    let occluders = Occluders {
        scene,
        placements: &parts,
        tree: PlacementTree::new(&parts),
        fills,
    };
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

/// The dimensions and notes of `doc` as `camera` sees them, `size_px` pixels across: their
/// lines and label text only. Nothing is tested for visibility (annotations are never hidden),
/// so this costs no hidden-line pass and cannot be refused for size.
pub fn annotation_labels(doc: &Document, camera: &Camera, size_px: DVec2) -> Drawing {
    let mut camera = camera.clone();
    camera.viewport = size_px;
    let parts = placements(doc.scene(), doc.section_display.cuts);
    let mut drawing = Drawing {
        size: size_px,
        ..Drawing::default()
    };
    annotations(doc, &camera, &parts, &mut drawing);
    drawing
}

/// The points a print of `doc` must cover, in root coordinates: the shown geometry that the
/// section cuts keep (kept vertices, the kept parts of edges, the outlines of the cuts) and
/// the points of the shown dimensions and notes. A print at scale sizes its sheets from these.
pub fn extent_points(doc: &Document) -> Vec<DVec3> {
    let scene = doc.scene();
    let display = doc.section_display;
    let parts = placements(scene, display.cuts);
    let mut out = Vec::new();
    for part in &parts {
        let model = part.model;
        for (_, edge) in model.edges().filter(|(_, e)| scene.edge_shown(e)) {
            let (Some(a), Some(b)) = (model.position(edge.v[0]), model.position(edge.v[1])) else {
                continue;
            };
            let (a, b) = (
                part.place.transform_point3(a),
                part.place.transform_point3(b),
            );
            if let Some([a, b]) = clip_segment(a, b, &part.planes) {
                out.extend([a, b]);
            }
        }
        for (_, face) in model.faces().filter(|(_, f)| scene.face_shown(f)) {
            out.extend(
                face.outer
                    .iter()
                    .filter_map(|v| model.position(*v))
                    .map(|p| part.place.transform_point3(p))
                    .filter(|p| part.kept(*p)),
            );
        }
        if !part.planes.is_empty() {
            let mirrored = part.place.matrix3.determinant() < 0.0;
            let surface: Vec<[DVec3; 3]> = model
                .faces()
                .filter(|(_, f)| scene.face_shown(f))
                .flat_map(|(id, _)| model.triangulate(id).unwrap_or_default())
                .map(|t| {
                    let [a, b, c] = t.map(|p| part.place.transform_point3(p));
                    if mirrored { [a, c, b] } else { [a, b, c] }
                })
                .collect();
            if !surface.is_empty() {
                let cut = cut_surface(&surface, &part.planes, false);
                out.extend(cut.edges.into_iter().flatten());
            }
        }
        let shown = |tag: Option<TagId>| scene.tag_shown(tag);
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
            if let Some(shape) = AnnotationShape::of(model, id) {
                out.extend(shape.transformed(part.place).points());
            }
        }
    }
    out
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
            let visible = projector.project(mid).is_some_and(|screen| {
                let from = camera.sight_origin(mid);
                // Meters a face may lie in front of the point and not hide it: a share of
                // the distance to the eye, in a parallel view to the target (not to the
                // far-back point the sight line starts from).
                let tol = SIGHT_TOL
                    * if camera.is_parallel() {
                        camera.distance()
                    } else {
                        from.distance(mid)
                    };
                !occluders.hides(from, mid, screen, tol)
            });
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
