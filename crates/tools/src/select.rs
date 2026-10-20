//! The Select tool: click an edge, face, guide, dimension, note or group to select it (a
//! dimension or note also by its label), or drag a box. Clicking
//! an edge of a curve (a circle or arc) selects the whole curve. Dragging right selects what
//! lies entirely inside the box; dragging left also selects what the box touches.
//! With Shift held, clicked or boxed entities are toggled instead of replacing the selection.
//! Double-clicking a face selects it with its edges, an edge with its faces; triple-clicking
//! selects everything connected. Double-clicking a group opens it for editing; double-clicking
//! outside it, or Escape, closes it.

use glam::{DVec2, DVec3};
use kernel::geom::point_in_polygon;
use kernel::{EntityId, Face, Guide, Model};
use session::{AnnotationShape, Camera, Command, SelectMode};

use crate::inference::{
    annotation_under_where, edge_under_where, face_under_in, guide_under_where, label_under,
};
use crate::scenery::{Label, Owner, Scenery};
use crate::tool::{NoteRequest, Phase, Tool, ToolContext, ToolEvent, ToolOutput};

#[derive(Clone, Debug, Default)]
pub struct SelectTool {
    /// Cursor position where the button went down, while it is held.
    press: Option<DVec2>,
    cursor: DVec2,
    /// What a click would select, while the button is up.
    hovered: Option<EntityId>,
    /// The last double-click opened or closed a group: the triple-click that follows it does
    /// nothing more than its click.
    opened: bool,
}

impl SelectTool {
    pub fn new() -> Self {
        Self::default()
    }
}

/// What a click picks: the label under the cursor (its dimension or note, or the instance
/// placing it), else the open context's edge, else its dimension or note by their lines, else
/// its guide, else an instance's edge or annotation line (which picks the instance), else the
/// face under it (the open context's face, or the instance). Geometry outside the open
/// instance hides what is behind it but is not picked, and its edges do not outrank the open
/// context's even when they lie on top of them.
pub(crate) fn pick(ctx: &ToolContext) -> Option<EntityId> {
    let open = |o: Owner| o == Owner::Open;
    let in_instance = |o: Owner| matches!(o, Owner::Instance(_));
    match label_under(ctx.labels, ctx.cursor, |o| open(o) || in_instance(o)) {
        Some((Owner::Instance(i), _)) => return Some(EntityId::Instance(i)),
        Some((_, id)) => return Some(id),
        None => {}
    }
    let edge_of = |keep: fn(Owner) -> bool| {
        edge_under_where(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius, keep)
    };
    let annotation_of = |keep: fn(Owner) -> bool| {
        annotation_under_where(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius, keep)
    };
    if let Some((_, e, _, _)) = edge_of(open) {
        return Some(EntityId::Edge(e));
    }
    if let Some((_, id)) = annotation_of(open) {
        return Some(id);
    }
    if let Some((_, g, _)) =
        guide_under_where(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius, open)
    {
        return Some(EntityId::Guide(g));
    }
    if let Some((Owner::Instance(i), ..)) = edge_of(in_instance) {
        return Some(EntityId::Instance(i));
    }
    if let Some((Owner::Instance(i), _)) = annotation_of(in_instance) {
        return Some(EntityId::Instance(i));
    }
    match face_under_in(ctx.scenery, ctx.camera, ctx.cursor)? {
        (Owner::Open, f, _) => Some(EntityId::Face(f)),
        (Owner::Instance(i), _, _) => Some(EntityId::Instance(i)),
        (Owner::Outside, _, _) => None,
    }
}

/// The edges of the curve `id` is part of, in order, or just `id` when it is not an edge of
/// a curve.
pub(crate) fn whole_curve(model: &Model, id: EntityId) -> Vec<EntityId> {
    match id {
        EntityId::Edge(e) => model.curve_of(e).map_or(vec![id], |c| {
            c.edges.into_iter().map(EntityId::Edge).collect()
        }),
        _ => vec![id],
    }
}

/// What a double-click on `id` selects: a face with its edges, an edge with its whole curve
/// and the faces along each of its edges, a guide, dimension or note alone; sorted. Empty for
/// anything else.
fn bounded(model: &Model, id: EntityId) -> Vec<EntityId> {
    let mut ids = std::collections::BTreeSet::from([id]);
    match id {
        EntityId::Face(f) => ids.extend(model.face_edges(f).into_iter().map(EntityId::Edge)),
        EntityId::Edge(_) => {
            for e in whole_curve(model, id) {
                ids.insert(e);
                if let EntityId::Edge(e) = e {
                    ids.extend(model.faces_of_edge(e).into_iter().map(EntityId::Face));
                }
            }
        }
        EntityId::Guide(_) | EntityId::Dimension(_) | EntityId::Note(_) => {}
        _ => return Vec::new(),
    }
    ids.into_iter().collect()
}

/// A double- or triple-click's selection: replaces it, or with Shift adds to it.
fn widen(ids: Vec<EntityId>, ctx: &ToolContext, out: &mut ToolOutput) {
    if ids.is_empty() {
        return;
    }
    let mode = if ctx.shift {
        SelectMode::Add
    } else {
        SelectMode::Replace
    };
    out.commands.push(Command::Select {
        ids,
        mode,
        all: false,
    });
}

/// True when some, but not all, of `ids` are selected.
fn partly_selected(ids: &[EntityId], ctx: &ToolContext) -> bool {
    let n = ids.iter().filter(|id| ctx.selection.contains(id)).count();
    n > 0 && n < ids.len()
}

/// The corners of an instance's bounding box (bit 0 picks x, bit 1 y, bit 2 z).
fn box_corners(scenery: &Scenery, id: EntityId) -> Option<[DVec3; 8]> {
    let EntityId::Instance(i) = id else {
        return None;
    };
    let [lo, hi] = scenery.instance_box(i)?;
    Some(std::array::from_fn(|k| {
        DVec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        )
    }))
}

/// The model-axis bounds (lowest and highest corner) of `ids`: the vertices of its edges, faces
/// (outer and hole loops) and vertices, its instances' boxes, its guides' anchors and the
/// points of its dimensions' and leader notes' lines and labels (see
/// `AnnotationShape::points`). None when none of them exists.
pub(crate) fn selection_bounds(
    ctx: &ToolContext,
    ids: &std::collections::BTreeSet<EntityId>,
) -> Option<[DVec3; 2]> {
    let m = ctx.model;
    let mut points = Vec::new();
    for id in ids {
        match *id {
            EntityId::Edge(e) => {
                if let Some(e) = m.edge(e) {
                    points.extend(e.v.iter().filter_map(|v| m.position(*v)));
                }
            }
            EntityId::Face(f) => {
                if let Some(f) = m.face(f) {
                    points.extend(face_loops(f).flatten().filter_map(|v| m.position(*v)));
                }
            }
            EntityId::Vertex(v) => points.extend(m.position(v)),
            EntityId::Instance(i) => {
                points.extend(ctx.scenery.instance_box(i).into_iter().flatten())
            }
            EntityId::Guide(g) => points.extend(m.guide(g).map(Guide::anchor)),
            EntityId::Dimension(_) | EntityId::Note(_) => {
                points.extend(AnnotationShape::of(m, *id).map_or(Vec::new(), |s| s.points()))
            }
        }
    }
    let first = *points.first()?;
    Some(
        points
            .iter()
            .fold([first, first], |[lo, hi], p| [lo.min(*p), hi.max(*p)]),
    )
}

/// The twelve edges of an instance's bounding box, for outlines.
pub(crate) fn box_outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
    let Some(c) = box_corners(ctx.scenery, id) else {
        return Vec::new();
    };
    (0..8)
        .flat_map(|k| [1, 2, 4].map(|bit| (k, k | bit)))
        .filter(|(a, b)| a != b)
        .map(|(a, b)| [c[a], c[b]])
        .collect()
}

/// The open context's edges, faces, instances, guides, dimensions and notes a screen box with
/// corners `a` and `b` selects, as `camera` (in the open context's coordinates) sees
/// `scenery`: those inside it entirely, or with `crossing` those touching it (the Select
/// tool's box dragged right, or left). Entities with a point behind the camera are skipped,
/// and so are those on hidden tags. Guide lines are never inside a box; a crossing box selects
/// those crossing it. Screen notes go by their rectangles among `labels` (the window's last
/// layout): inside the box, or with `crossing` overlapping it.
pub fn select_box(
    scenery: &Scenery,
    camera: &Camera,
    a: DVec2,
    b: DVec2,
    crossing: bool,
    labels: &[Label],
) -> Vec<EntityId> {
    let (lo, hi) = (a.min(b), a.max(b));
    let window = !crossing;
    let m = scenery.open();
    let screen = |points: &[kernel::VertexId]| -> Option<Vec<DVec2>> {
        points
            .iter()
            .map(|v| m.position(*v).and_then(|p| camera.project(p)))
            .collect()
    };
    let inside = |s: DVec2| s.cmpge(lo).all() && s.cmple(hi).all();
    let corners = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
    let touches = |p: DVec2, q: DVec2| {
        inside(p)
            || inside(q)
            || (0..4).any(|i| segments_cross(p, q, corners[i], corners[(i + 1) % 4]))
    };
    let chosen = |closed: bool, pts: &[DVec2]| {
        if window {
            pts.iter().all(|p| inside(*p))
        } else {
            let n = pts.len();
            let sides = if closed { n } else { n - 1 };
            (0..sides).any(|i| touches(pts[i], pts[(i + 1) % n]))
        }
    };
    let mut ids = Vec::new();
    for (id, e) in m.edges().filter(|(_, e)| scenery.shows(e.tag)) {
        if screen(&e.v).is_some_and(|pts| chosen(false, &pts)) {
            ids.push(EntityId::Edge(id));
        }
    }
    // A crossing box also selects a face that it lies entirely inside: some corner of the box
    // is within the face's projected outer loop and outside all of its holes.
    let within = |f: &Face| {
        let Some(outer) = screen(&f.outer) else {
            return false;
        };
        let holes: Vec<Vec<DVec2>> = f.holes.iter().filter_map(|h| screen(h)).collect();
        corners
            .iter()
            .any(|c| point_in_polygon(*c, &outer) && !holes.iter().any(|h| point_in_polygon(*c, h)))
    };
    // Instances by their bounding boxes: inside when every corner is, touching when the box's
    // screen rectangle overlaps the selection box.
    for (id, _) in m.instances().filter(|(_, i)| scenery.shows(i.tag)) {
        let id = EntityId::Instance(id);
        let Some(corners) = box_corners(scenery, id) else {
            continue;
        };
        let Some(pts) = corners
            .iter()
            .map(|p| camera.project(*p))
            .collect::<Option<Vec<DVec2>>>()
        else {
            continue;
        };
        let hit = if window {
            pts.iter().all(|p| inside(*p))
        } else {
            let (bl, bh) = pts
                .iter()
                .fold((pts[0], pts[0]), |(l, h), p| (l.min(*p), h.max(*p)));
            bl.cmple(hi).all() && bh.cmpge(lo).all()
        };
        if hit {
            ids.push(id);
        }
    }
    for (id, g) in m.guides().filter(|(_, g)| scenery.shows(g.tag())) {
        let hit = match *g {
            Guide::Point { position, .. } => camera.project(position).is_some_and(inside),
            Guide::Line {
                point, direction, ..
            } => {
                !window
                    && camera
                        .line_span(point, direction)
                        .and_then(|[a, b]| Some((camera.project(a)?, camera.project(b)?)))
                        .is_some_and(|(p, q)| touches(p, q))
            }
        };
        if hit {
            ids.push(EntityId::Guide(id));
        }
    }
    for (part, id, shape) in scenery.annotations() {
        if scenery.part(*part).owner != Owner::Open {
            continue;
        }
        let hit = match shape {
            AnnotationShape::Screen { .. } => labels
                .iter()
                .filter(|l| l.owner == Owner::Open && l.id == *id)
                .any(|l| {
                    if window {
                        inside(l.min) && inside(l.max)
                    } else {
                        l.min.cmple(hi).all() && l.max.cmpge(lo).all()
                    }
                }),
            _ if window => shape
                .points()
                .iter()
                .all(|p| camera.project(*p).is_some_and(inside)),
            _ => shape.segments().iter().any(|[p, q]| {
                camera
                    .project(*p)
                    .zip(camera.project(*q))
                    .is_some_and(|(p, q)| touches(p, q))
            }),
        };
        if hit {
            ids.push(*id);
        }
    }
    for (id, f) in m.faces().filter(|(_, f)| scenery.shows(f.tag)) {
        let all = face_loops(f).all(|l| screen(l).is_some_and(|pts| !pts.is_empty()));
        let hit = if window {
            all && face_loops(f).all(|l| screen(l).is_some_and(|pts| chosen(true, &pts)))
        } else {
            face_loops(f).any(|l| screen(l).is_some_and(|pts| chosen(true, &pts)))
                || (all && within(f))
        };
        if hit {
            ids.push(EntityId::Face(id));
        }
    }
    ids
}

fn face_loops(f: &Face) -> impl Iterator<Item = &Vec<kernel::VertexId>> {
    std::iter::once(&f.outer).chain(&f.holes)
}

/// True if the 2D segments p-q and r-s cross or touch.
fn segments_cross(p: DVec2, q: DVec2, r: DVec2, s: DVec2) -> bool {
    let orient = |a: DVec2, b: DVec2, c: DVec2| (b - a).perp_dot(c - a);
    let (d1, d2) = (orient(r, s, p), orient(r, s, q));
    let (d3, d4) = (orient(p, q, r), orient(p, q, s));
    d1 * d2 <= 0.0 && d3 * d4 <= 0.0 && (d1 != 0.0 || d2 != 0.0 || d3 != 0.0 || d4 != 0.0)
}

/// World-space segments that draw the screen box from `a` to `b`, just in front of the camera.
fn rubber_band(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<[DVec3; 2]> {
    let depth = ctx.camera.distance() * 0.5;
    let at = |px: DVec2| {
        let ray = ctx.camera.ray(px);
        ray.origin + ray.dir * depth
    };
    let c = [a, DVec2::new(b.x, a.y), b, DVec2::new(a.x, b.y)].map(at);
    (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect()
}

impl Tool for SelectTool {
    fn name(&self) -> &'static str {
        "Select"
    }

    fn prompt(&self) -> &'static str {
        match self.press {
            None => {
                "Click an edge, face or group, or drag a box; Shift adds or removes; \
                 double-click a group to edit it"
            }
            Some(_) => "Release to select: drag right for inside the box, left for touching",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    /// Active while the button is held: a click or a box being dragged, both on screen.
    fn phase(&self) -> Phase {
        match self.press {
            Some(_) => Phase::Active { points: Vec::new() },
            None => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        self.cursor = ctx.cursor;
        match event {
            ToolEvent::Move(_) => {
                self.hovered = if self.press.is_none() {
                    pick(ctx)
                } else {
                    None
                };
            }
            ToolEvent::Click(_) => {
                self.press = Some(ctx.cursor);
                self.hovered = None;
            }
            ToolEvent::Release(_) => {
                if let Some(start) = self.press.take() {
                    let clicked = start.distance(ctx.cursor) < ctx.drag_threshold;
                    let ids = if clicked {
                        pick(ctx).map_or(Vec::new(), |id| whole_curve(ctx.model, id))
                    } else {
                        // Dragged left: what the box touches.
                        let crossing = ctx.cursor.x < start.x;
                        select_box(
                            ctx.scenery,
                            ctx.camera,
                            start,
                            ctx.cursor,
                            crossing,
                            ctx.labels,
                        )
                    };
                    let mode = if !ctx.shift {
                        SelectMode::Replace
                    } else if clicked && partly_selected(&ids, ctx) {
                        // Shift-clicking a half-selected curve completes it.
                        SelectMode::Add
                    } else {
                        SelectMode::Toggle
                    };
                    // Shift with nothing picked leaves the selection alone.
                    if !(ctx.shift && ids.is_empty()) {
                        out.commands.push(Command::Select {
                            ids,
                            mode,
                            all: false,
                        });
                    }
                }
            }
            ToolEvent::Value(_) => {
                out.message = Some("The Select tool takes no typed values".into());
            }
            // The click's own release selected what is under the cursor; a double- or
            // triple-click widens that.
            ToolEvent::DoubleClick(_) => {
                self.opened = false;
                match pick(ctx) {
                    Some(id @ EntityId::Instance(_)) => {
                        out.commands.push(Command::OpenInstance { id });
                        self.opened = true;
                    }
                    None if ctx.scenery.inside_instance() => {
                        out.commands.push(Command::CloseInstance {});
                        self.opened = true;
                    }
                    // A note: selected alone, and its text asked for.
                    Some(id @ EntityId::Note(_)) => {
                        widen(bounded(ctx.model, id), ctx, &mut out);
                        out.note = Some(NoteRequest::Edit(id));
                    }
                    Some(id) => widen(bounded(ctx.model, id), ctx, &mut out),
                    None => {}
                }
            }
            ToolEvent::TripleClick(_) => {
                if !std::mem::take(&mut self.opened)
                    && let Some(id @ (EntityId::Edge(_) | EntityId::Face(_))) = pick(ctx)
                {
                    widen(ctx.model.connected(id), ctx, &mut out);
                }
            }
            // Escape with nothing in progress closes the open instance.
            ToolEvent::Cancel => {
                self.hovered = None;
                if self.press.take().is_none() && ctx.scenery.inside_instance() {
                    out.commands.push(Command::CloseInstance {});
                }
            }
            ToolEvent::ToggleCopy => {}
        }
        if let Some(start) = self.press
            && start.distance(self.cursor) >= ctx.drag_threshold
        {
            out.preview = rubber_band(ctx, start, self.cursor);
        }
        out
    }

    fn on_external_change(&mut self) {
        self.press = None;
        self.hovered = None;
    }

    fn hovered(&self) -> Option<EntityId> {
        self.hovered
    }
}

/// Segments drawing a guide of the open context: the part of a guide line in view, or an X
/// as wide as the pick radius on screen around a guide point.
pub(crate) fn guide_outline(ctx: &ToolContext, g: &Guide) -> Vec<[DVec3; 2]> {
    match *g {
        Guide::Line {
            point, direction, ..
        } => ctx.camera.line_span(point, direction).into_iter().collect(),
        Guide::Point { position, .. } => {
            let c = ctx.camera;
            if !c.sees_depth(position) {
                return Vec::new();
            }
            let size = ctx.pick_radius * c.pixel_size(position);
            let (r, u) = (c.right() * size, c.up() * size);
            vec![
                [position - r - u, position + r + u],
                [position - r + u, position + r - u],
            ]
        }
    }
}

/// Positions of the vertices of a face's loops, for outlines.
pub(crate) fn face_outline(model: &Model, f: &Face) -> Vec<[DVec3; 2]> {
    let mut out = Vec::new();
    for l in face_loops(f) {
        let pts: Vec<DVec3> = l.iter().filter_map(|v| model.position(*v)).collect();
        for i in 0..pts.len() {
            out.push([pts[i], pts[(i + 1) % pts.len()]]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use session::Camera;

    /// A 2 m square on the ground (face f8) seen by the default camera.
    struct Rig {
        tool: SelectTool,
        model: Model,
        camera: Camera,
        cursor: DVec2,
        drag_threshold: f64,
        shift: bool,
        selection: std::collections::BTreeSet<EntityId>,
        labels: Vec<Label>,
    }

    impl Rig {
        fn new() -> Self {
            let mut model = Model::new();
            let pts = [
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(2.0, 2.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ];
            model.add_polygon(&pts).unwrap();
            Self {
                tool: SelectTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
                shift: false,
                drag_threshold: 4.0,
                selection: Default::default(),
                labels: Vec::new(),
            }
        }

        /// The square, a 1 m box beside it (x 3 to 4, y -2 to -1) and an 8-sided circle of
        /// radius 0.5 m at (-1.5, -2.5) on the ground.
        fn boxed() -> Self {
            let mut r = Self::new();
            let p = |x, y| DVec3::new(x, y, 0.0);
            let c = r
                .model
                .add_polygon(&[p(3.0, -2.0), p(4.0, -2.0), p(4.0, -1.0), p(3.0, -1.0)])
                .unwrap();
            let base = c
                .summary()
                .created
                .iter()
                .find_map(|id| match id {
                    EntityId::Face(f) => Some(*f),
                    _ => None,
                })
                .unwrap();
            r.model.push_pull(base, 1.0).unwrap();
            let ring: Vec<DVec3> = (0..8)
                .map(|k| {
                    let a = std::f64::consts::TAU * k as f64 / 8.0;
                    p(-1.5 + 0.5 * a.cos(), -2.5 + 0.5 * a.sin())
                })
                .collect();
            r.model.add_curve(&ring, true, false).unwrap();
            r
        }

        fn px(&self, p: DVec3) -> DVec2 {
            self.camera.project(p).unwrap()
        }

        /// Clicks twice at `p`, then double-clicks there.
        fn double_click(&mut self, p: DVec3) -> ToolOutput {
            self.click(p);
            self.click(p);
            self.send(ToolEvent::DoubleClick(Self::hit()))
        }

        /// Clicks three times at `p`, double-clicking after the second, then triple-clicks.
        fn triple_click(&mut self, p: DVec3) -> ToolOutput {
            self.double_click(p);
            self.click(p);
            self.send(ToolEvent::TripleClick(Self::hit()))
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &crate::scenery::Scenery::single(&self.model),
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                drag_threshold: self.drag_threshold,
                quick_press: false,
                shift: self.shift,
                selection: &self.selection,
                material: None,
                sample: false,
                labels: &self.labels,
            };
            self.tool.handle(event, &ctx)
        }

        fn hit() -> Inference {
            Inference {
                point: DVec3::ZERO,
                kind: SnapKind::Free,
                face: None,
            }
        }

        /// Presses at `a`, moves to `b` and releases there.
        fn drag(&mut self, a: DVec2, b: DVec2) -> ToolOutput {
            self.cursor = a;
            self.send(ToolEvent::Click(Self::hit()));
            self.cursor = b;
            self.send(ToolEvent::Move(Self::hit()));
            self.send(ToolEvent::Release(Self::hit()))
        }

        fn click(&mut self, p: DVec3) -> ToolOutput {
            let at = self.px(p);
            self.drag(at, at)
        }
    }

    fn select(ids: &[&str], mode: SelectMode) -> Vec<Command> {
        vec![Command::Select {
            ids: ids.iter().map(|s| s.parse().unwrap()).collect(),
            mode,
            all: false,
        }]
    }

    /// The ids of a single `Select` command, and its mode.
    fn selected(out: &ToolOutput) -> (Vec<EntityId>, SelectMode) {
        match &out.commands[..] {
            [Command::Select { ids, mode, .. }] => (ids.clone(), *mode),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_double_click_on_a_face_selects_it_with_its_edges() {
        let mut r = Rig::new();
        let out = r.double_click(DVec3::new(1.0, 1.0, 0.0));
        assert_eq!(
            out.commands,
            select(&["e2", "e4", "e6", "e7", "f8"], SelectMode::Replace)
        );
    }

    #[test]
    fn a_double_click_on_an_edge_selects_it_with_its_faces() {
        let mut r = Rig::boxed();
        // The box's vertical edge nearest the camera, between its x = 3 and y = -2 sides.
        let (ids, mode) = selected(&r.double_click(DVec3::new(3.0, -2.0, 0.5)));
        assert_eq!(mode, SelectMode::Replace);
        let [EntityId::Edge(e), EntityId::Face(a), EntityId::Face(b)] = ids[..] else {
            panic!("{ids:?}");
        };
        let edge = r.model.edge(e).unwrap();
        let ends = edge.v.map(|v| r.model.position(v).unwrap());
        assert!(ends.iter().all(|p| p.x == 3.0 && p.y == -2.0), "{ends:?}");
        let normals = [a, b].map(|f| r.model.face(f).unwrap().normal);
        assert!(normals.contains(&DVec3::NEG_X) && normals.contains(&DVec3::NEG_Y));
        // A circle's edge: the whole curve and its face.
        let at = |deg: f64| {
            let a = deg.to_radians();
            DVec3::new(-1.5 + 0.5 * a.cos(), -2.5 + 0.5 * a.sin(), 0.0)
        };
        let rim = (at(225.0) + at(270.0)) / 2.0;
        let (ids, _) = selected(&r.double_click(rim));
        let edges = ids
            .iter()
            .filter(|id| matches!(id, EntityId::Edge(_)))
            .count();
        let faces: Vec<_> = ids
            .iter()
            .filter(|id| matches!(id, EntityId::Face(_)))
            .collect();
        assert_eq!((edges, faces.len()), (8, 1), "{ids:?}");
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn a_triple_click_selects_everything_connected() {
        let mut r = Rig::boxed();
        let top = DVec3::new(3.5, -1.5, 1.0);
        let (ids, mode) = selected(&r.triple_click(top));
        assert_eq!(mode, SelectMode::Replace);
        let edges = ids
            .iter()
            .filter(|id| matches!(id, EntityId::Edge(_)))
            .count();
        let faces = ids
            .iter()
            .filter(|id| matches!(id, EntityId::Face(_)))
            .count();
        assert_eq!((edges, faces), (12, 6), "{ids:?}");
        assert!(!ids.contains(&"f8".parse().unwrap()), "not the square");
        r.shift = true;
        let (again, mode) = selected(&r.triple_click(top));
        assert_eq!((again, mode), (ids, SelectMode::Add));
        // A triple-click on nothing does nothing more than its click.
        r.shift = false;
        assert!(
            r.triple_click(DVec3::new(-3.0, 4.0, 0.0))
                .commands
                .is_empty()
        );
    }

    #[test]
    fn a_double_click_on_a_group_still_opens_it() {
        let scene = scene_with_a_guide_in_a_group();
        let scenery = crate::scenery::Scenery::of(&scene, &[]).unwrap();
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let mut tool = SelectTool::new();
        let send = |tool: &mut SelectTool, event: ToolEvent| {
            let ctx = ToolContext {
                model: scene.root(),
                scenery: &scenery,
                camera: &camera,
                cursor: camera.project(DVec3::new(0.5, 0.5, 0.0)).unwrap(),
                pick_radius: 10.0,
                drag_threshold: 4.0,
                quick_press: false,
                shift: false,
                selection: &Default::default(),
                material: None,
                sample: false,
                labels: &[],
            };
            tool.handle(event, &ctx)
        };
        let out = send(&mut tool, ToolEvent::DoubleClick(Rig::hit()));
        let id = scene.root().instances().next().unwrap().0;
        assert_eq!(
            out.commands,
            vec![Command::OpenInstance {
                id: EntityId::Instance(id)
            }]
        );
        // The third click of a triple-click lands in what just opened: it does nothing more.
        send(&mut tool, ToolEvent::Click(Rig::hit()));
        send(&mut tool, ToolEvent::Release(Rig::hit()));
        assert!(
            send(&mut tool, ToolEvent::TripleClick(Rig::hit()))
                .commands
                .is_empty()
        );
        assert!(!tool.opened, "the triple-click clears it");
    }

    #[test]
    fn hovering_reports_what_a_click_would_select() {
        let mut r = Rig::new();
        let hover = |r: &mut Rig, p: DVec3| {
            r.cursor = r.px(p);
            r.send(ToolEvent::Move(Rig::hit()));
            r.tool.hovered()
        };
        let edge = hover(&mut r, DVec3::new(1.0, 0.0, 0.0));
        assert_eq!(edge, Some("e2".parse().unwrap()));
        let face = hover(&mut r, DVec3::new(1.0, 1.0, 0.0));
        assert_eq!(face, Some("f8".parse().unwrap()));
        assert_eq!(hover(&mut r, DVec3::new(-3.0, 4.0, 0.0)), None);
        // While the button is held (a box being dragged) nothing is hovered.
        hover(&mut r, DVec3::new(1.0, 1.0, 0.0));
        r.send(ToolEvent::Click(Rig::hit()));
        assert_eq!(r.tool.hovered(), None);
        assert_eq!(hover(&mut r, DVec3::new(1.5, 1.5, 0.0)), None);
    }

    #[test]
    fn clicking_picks_edges_before_faces() {
        let mut r = Rig::new();
        let edge = r.click(DVec3::new(1.0, 0.0, 0.0));
        assert_eq!(edge.commands, select(&["e2"], SelectMode::Replace));
        let face = r.click(DVec3::new(1.0, 1.0, 0.0));
        assert_eq!(face.commands, select(&["f8"], SelectMode::Replace));
    }

    #[test]
    fn clicking_a_curve_selects_all_of_it_and_soft_edges_are_not_picked() {
        let mut r = Rig::new();
        // A cylinder: an 8-sided circle of radius 1 pulled up 1 m, in front of the square.
        let mut model = Model::new();
        let ring: Vec<DVec3> = (0..8)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 8.0;
                DVec3::new(a.cos(), a.sin(), 0.0)
            })
            .collect();
        let c = model.add_curve(&ring, true, false).unwrap();
        let face = c
            .summary()
            .created
            .iter()
            .find_map(|id| match id {
                EntityId::Face(f) => Some(*f),
                _ => None,
            })
            .unwrap();
        model.push_pull(face, 1.0).unwrap();
        r.model = model;
        let at = |deg: f64, z: f64| {
            let a = deg.to_radians();
            DVec3::new(a.cos(), a.sin(), z)
        };

        // The middle of a top rim edge facing the camera: the whole top circle.
        let rim = r.click((at(225.0, 1.0) + at(270.0, 1.0)) / 2.0);
        let [Command::Select { ids, mode, .. }] = &rim.commands[..] else {
            panic!("{:?}", rim.commands);
        };
        assert_eq!(*mode, SelectMode::Replace);
        assert_eq!(ids.len(), 8);
        let EntityId::Edge(first) = ids[0] else {
            panic!("{ids:?}")
        };
        let top = r.model.edge(first).unwrap().curve;
        assert!(top.is_some());
        assert!(ids.iter().all(|id| matches!(id, EntityId::Edge(e)
            if r.model.edge(*e).unwrap().curve == top)));

        // The soft edge up the side facing the camera is not picked: the face behind it is.
        let side = r.click(at(225.0, 0.5));
        let [Command::Select { ids, .. }] = &side.commands[..] else {
            panic!("{:?}", side.commands);
        };
        assert!(matches!(ids[..], [EntityId::Face(_)]), "{ids:?}");
    }

    #[test]
    fn drag_threshold_decides_between_click_and_box() {
        let mut r = Rig::new();
        r.drag_threshold = 8.0;
        let a = r.px(DVec3::new(1.0, 1.0, 0.0));
        let out = r.drag(a, a + DVec2::new(6.0, 0.0));
        assert_eq!(out.commands, select(&["f8"], SelectMode::Replace));
        r.drag_threshold = 4.0;
        let out = r.drag(a, a + DVec2::new(6.0, 0.0));
        assert_ne!(out.commands, select(&["f8"], SelectMode::Replace));
    }

    #[test]
    fn clicking_nothing_clears_unless_shift_is_held() {
        let mut r = Rig::new();
        let off = DVec3::new(-3.0, 4.0, 0.0);
        assert_eq!(r.click(off).commands, select(&[], SelectMode::Replace));
        r.shift = true;
        assert!(r.click(off).commands.is_empty());
        let face = r.click(DVec3::new(1.0, 1.0, 0.0));
        assert_eq!(face.commands, select(&["f8"], SelectMode::Toggle));
    }

    #[test]
    fn dragging_right_selects_what_is_inside_the_box() {
        let mut r = Rig::new();
        let pts: Vec<DVec2> = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]
            .iter()
            .map(|&(x, y)| r.px(DVec3::new(x, y, 0.0)))
            .collect();
        let lo = pts.iter().fold(DVec2::INFINITY, |a, p| a.min(*p)) - 5.0;
        let hi = pts.iter().fold(DVec2::NEG_INFINITY, |a, p| a.max(*p)) + 5.0;
        let all = r.drag(lo, hi);
        assert_eq!(
            all.commands,
            select(&["e2", "e4", "e6", "e7", "f8"], SelectMode::Replace)
        );
        // A box around only the edge from (0,0) to (2,0) holds just that edge.
        let (a, b) = (pts[0].min(pts[1]) - 5.0, pts[0].max(pts[1]) + 5.0);
        let one = r.drag(a, b);
        assert_eq!(one.commands, select(&["e2"], SelectMode::Replace));
    }

    #[test]
    fn dragging_left_selects_what_the_box_touches() {
        let mut r = Rig::new();
        // A small box around the middle of the edge from (0,0) to (2,0), dragged right to left.
        let mid = r.px(DVec3::new(1.0, 0.0, 0.0));
        let (a, b) = (mid + DVec2::new(6.0, -6.0), mid + DVec2::new(-6.0, 6.0));
        let out = r.drag(a, b);
        assert_eq!(out.commands, select(&["e2", "f8"], SelectMode::Replace));
        let window = r.drag(b, a);
        assert_eq!(window.commands, select(&[], SelectMode::Replace));
    }

    #[test]
    fn dragging_left_inside_a_face_selects_it() {
        let mut r = Rig::new();
        // A small box around the middle of the square's interior, away from every edge.
        let mid = r.px(DVec3::new(1.0, 1.0, 0.0));
        let (a, b) = (mid + DVec2::new(6.0, -6.0), mid + DVec2::new(-6.0, 6.0));
        let out = r.drag(a, b);
        assert_eq!(out.commands, select(&["f8"], SelectMode::Replace));
    }

    #[test]
    fn guides_are_clicked_after_edges_and_boxed() {
        let mut r = Rig::new();
        r.model
            .add_guide(Guide::Point {
                position: DVec3::new(3.0, 1.0, 0.0),
                tag: None,
            })
            .unwrap();
        r.model
            .add_guide(Guide::Line {
                point: DVec3::new(0.0, 3.0, 0.0),
                direction: DVec3::X,
                tag: None,
            })
            .unwrap();
        let point = r.click(DVec3::new(3.0, 1.0, 0.0));
        assert_eq!(point.commands, select(&["g9"], SelectMode::Replace));
        let line = r.click(DVec3::new(-1.0, 3.0, 0.0));
        assert_eq!(line.commands, select(&["g10"], SelectMode::Replace));
        // Where the guide line runs over the square's edge, the edge wins.
        r.model
            .add_guide(Guide::Line {
                point: DVec3::ZERO,
                direction: DVec3::X,
                tag: None,
            })
            .unwrap();
        let edge = r.click(DVec3::new(1.0, 0.0, 0.0));
        assert_eq!(edge.commands, select(&["e2"], SelectMode::Replace));

        // A window around the guide point holds it; one across the guide line does not hold
        // the line, but the same box dragged left selects it.
        let p = r.px(DVec3::new(3.0, 1.0, 0.0));
        let window = r.drag(p - 5.0, p + 5.0);
        assert_eq!(window.commands, select(&["g9"], SelectMode::Replace));
        let q = r.px(DVec3::new(-1.0, 3.0, 0.0));
        let (a, b) = (q - 6.0, q + 6.0);
        assert_eq!(r.drag(a, b).commands, select(&[], SelectMode::Replace));
        let crossing = r.drag(DVec2::new(b.x, a.y), DVec2::new(a.x, b.y));
        assert_eq!(crossing.commands, select(&["g10"], SelectMode::Replace));
    }

    #[test]
    fn dragging_draws_a_box_until_release() {
        let mut r = Rig::new();
        r.cursor = DVec2::new(100.0, 100.0);
        r.send(ToolEvent::Click(Rig::hit()));
        r.cursor = DVec2::new(200.0, 160.0);
        let out = r.send(ToolEvent::Move(Rig::hit()));
        assert_eq!(out.preview.len(), 4);
        let corner = r.camera.project(out.preview[0][0]).unwrap();
        assert!(corner.distance(DVec2::new(100.0, 100.0)) < 1e-6);
        let cancelled = r.send(ToolEvent::Cancel);
        assert!(cancelled.preview.is_empty());
        assert!(r.send(ToolEvent::Release(Rig::hit())).commands.is_empty());
    }

    /// A scene whose only instance, a group holding a unit square at the origin, also holds a
    /// guide point at (3, 1, 0).
    fn scene_with_a_guide_in_a_group() -> kernel::Scene {
        let mut m = Model::new();
        let p = |x, y| DVec3::new(x, y, 0.0);
        m.add_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)])
            .unwrap();
        let mut s = kernel::Scene::from_root(m);
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (i, _) = s
            .make_group(
                kernel::MeshId::Root,
                &edges,
                &faces,
                &[],
                kernel::DefinitionKind::Group,
                "",
            )
            .unwrap();
        let def = s.root().instance(i).unwrap().definition;
        s.edit(kernel::MeshId::Definition(def), |m| {
            m.add_guide(Guide::Point {
                position: p(3.0, 1.0),
                tag: None,
            })
        })
        .unwrap();
        s
    }

    #[test]
    fn a_guide_inside_a_closed_group_is_not_picked() {
        let scene = scene_with_a_guide_in_a_group();
        let scenery = crate::scenery::Scenery::of(&scene, &[]).unwrap();
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let click = |p: DVec3| {
            let ctx = ToolContext {
                model: scene.root(),
                scenery: &scenery,
                camera: &camera,
                cursor: camera.project(p).unwrap(),
                pick_radius: 10.0,
                drag_threshold: 4.0,
                quick_press: false,
                shift: false,
                selection: &Default::default(),
                material: None,
                sample: false,
                labels: &[],
            };
            pick(&ctx)
        };
        let picked = click(DVec3::new(3.0, 1.0, 0.0));
        assert!(!matches!(picked, Some(EntityId::Guide(_))), "{picked:?}");
        // The group's own geometry is still picked as the group.
        assert!(matches!(
            click(DVec3::new(0.5, 0.0, 0.0)),
            Some(EntityId::Instance(_))
        ));
    }

    #[test]
    fn shift_click_completes_a_half_selected_curve_and_removes_a_whole_one() {
        let mut r = Rig::new();
        let ring: Vec<DVec3> = (0..8)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 8.0;
                DVec3::new(a.cos(), a.sin(), 0.0)
            })
            .collect();
        let mut model = Model::new();
        model.add_curve(&ring, true, false).unwrap();
        r.model = model;
        let at = |deg: f64| {
            let a = deg.to_radians();
            DVec3::new(a.cos(), a.sin(), 0.0)
        };
        let rim = (at(225.0) + at(270.0)) / 2.0;
        let all: Vec<EntityId> =
            whole_curve(&r.model, EntityId::Edge(r.model.edges().next().unwrap().0));
        assert_eq!(all.len(), 8);
        r.shift = true;

        r.selection = all[..4].iter().copied().collect();
        let out = r.click(rim);
        let [Command::Select { ids, mode, .. }] = &out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert_eq!(
            (ids.len(), *mode),
            (8, SelectMode::Add),
            "adds the missing edges"
        );

        r.selection = all.iter().copied().collect();
        let out = r.click(rim);
        let [Command::Select { ids, mode, .. }] = &out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert_eq!(
            (ids.len(), *mode),
            (8, SelectMode::Toggle),
            "removes all of it"
        );
    }

    /// The Rig's square with a dimension below it (from (0, 0) to (2, 0), its line at y = -1)
    /// and a screen note; returns the rig and the two ids.
    fn annotated() -> (Rig, EntityId, EntityId) {
        let mut r = Rig::new();
        let mut scene = kernel::Scene::from_root(r.model.clone());
        let dimension = scene
            .edit(kernel::MeshId::Root, |m| {
                m.add_dimension(kernel::Dimension::Linear {
                    a: kernel::Anchor::Point(DVec3::ZERO),
                    b: kernel::Anchor::Point(DVec3::new(2.0, 0.0, 0.0)),
                    offset: DVec3::new(0.0, -1.0, 0.0),
                    tag: None,
                })
            })
            .unwrap()
            .summary()
            .created[0];
        let note = kernel::Note::Screen {
            at: [0.05, 0.05],
            text: "Plan".into(),
            tag: None,
        };
        let note = scene
            .add_note(kernel::MeshId::Root, note)
            .unwrap()
            .summary()
            .created[0];
        r.model = scene.root().clone();
        (r, dimension, note)
    }

    #[test]
    fn dimensions_are_picked_and_hovered_by_their_lines_and_labels() {
        let (mut r, dimension, note) = annotated();
        let on_line = DVec3::new(1.5, -1.0, 0.0);
        let (ids, _) = selected(&r.click(on_line));
        assert_eq!(ids, vec![dimension]);
        r.cursor = r.px(on_line);
        r.send(ToolEvent::Move(Rig::hit()));
        assert_eq!(r.tool.hovered(), Some(dimension));
        // The label lies over the square's face: the label wins.
        let over = r.px(DVec3::new(1.0, 1.0, 0.0));
        let (ids, _) = selected(&r.click(DVec3::new(1.0, 1.0, 0.0)));
        assert_eq!(ids.len(), 1);
        assert!(matches!(ids[0], EntityId::Face(_)));
        r.labels = vec![Label {
            owner: Owner::Open,
            id: dimension,
            min: over - DVec2::splat(20.0),
            max: over + DVec2::splat(20.0),
        }];
        let (ids, _) = selected(&r.click(DVec3::new(1.0, 1.0, 0.0)));
        assert_eq!(ids, vec![dimension]);
        // A screen note is picked by its label alone.
        r.labels.push(Label {
            owner: Owner::Open,
            id: note,
            min: DVec2::new(40.0, 30.0),
            max: DVec2::new(90.0, 50.0),
        });
        let out = r.drag(DVec2::new(60.0, 40.0), DVec2::new(60.0, 40.0));
        assert_eq!(selected(&out).0, vec![note]);
        // Labels outside the open context are not picked.
        r.labels = vec![Label {
            owner: Owner::Outside,
            ..r.labels[1]
        }];
        let out = r.drag(DVec2::new(60.0, 40.0), DVec2::new(60.0, 40.0));
        assert_eq!(selected(&out).0, vec![]);
    }

    #[test]
    fn double_clicking_a_note_selects_it_and_asks_for_its_text() {
        let (mut r, dimension, note) = annotated();
        let at = DVec3::new(-3.0, 3.0, 0.0);
        let px = r.px(at);
        r.labels = vec![Label {
            owner: Owner::Open,
            id: note,
            min: px - DVec2::splat(10.0),
            max: px + DVec2::splat(10.0),
        }];
        let out = r.double_click(at);
        assert_eq!(selected(&out).0, vec![note]);
        assert_eq!(out.note, Some(crate::tool::NoteRequest::Edit(note)));
        // A dimension's double-click asks for nothing.
        let out = r.double_click(DVec3::new(1.5, -1.0, 0.0));
        assert_eq!(selected(&out).0, vec![dimension]);
        assert_eq!(out.note, None);
    }

    #[test]
    fn a_box_selects_dimensions_by_their_lines_and_screen_notes_by_their_labels() {
        let (mut r, dimension, note) = annotated();
        let scenery = crate::scenery::Scenery::single(&r.model);
        let corners = [
            DVec3::new(-0.2, -1.2, 0.0),
            DVec3::new(2.2, 0.2, 0.0),
            DVec3::new(-0.2, 0.2, 0.0),
            DVec3::new(2.2, -1.2, 0.0),
        ]
        .map(|p| r.px(p));
        let lo = corners.iter().fold(corners[0], |a, b| a.min(*b));
        let hi = corners.iter().fold(corners[0], |a, b| a.max(*b));
        let label = Label {
            owner: Owner::Open,
            id: note,
            min: DVec2::new(40.0, 30.0),
            max: DVec2::new(90.0, 50.0),
        };
        let window = select_box(&scenery, &r.camera, lo, hi, false, &[label]);
        assert!(window.contains(&dimension), "{window:?}");
        assert!(!window.contains(&note));
        // Half the dimension: not inside, but touching.
        let mid = r.px(DVec3::new(1.0, -1.0, 0.0));
        let half = (DVec2::new(mid.x, lo.y), hi);
        assert!(!select_box(&scenery, &r.camera, half.0, half.1, false, &[]).contains(&dimension));
        assert!(select_box(&scenery, &r.camera, half.0, half.1, true, &[]).contains(&dimension));
        // The screen note's rectangle inside the box, or touched by a crossing box.
        let around = select_box(
            &scenery,
            &r.camera,
            DVec2::new(30.0, 20.0),
            DVec2::new(100.0, 60.0),
            false,
            &[label],
        );
        assert_eq!(around, vec![note]);
        let touching = (DVec2::new(80.0, 45.0), DVec2::new(120.0, 80.0));
        assert!(
            select_box(&scenery, &r.camera, touching.0, touching.1, false, &[label]).is_empty()
        );
        assert_eq!(
            select_box(&scenery, &r.camera, touching.0, touching.1, true, &[label]),
            vec![note]
        );
        // Without its label (no window drew it), a screen note is never boxed.
        r.labels.clear();
        assert!(
            !select_box(
                &scenery,
                &r.camera,
                DVec2::ZERO,
                DVec2::new(800.0, 600.0),
                true,
                &[]
            )
            .contains(&note)
        );
    }
}
