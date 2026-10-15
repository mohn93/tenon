//! The Select tool: click an edge, face, guide or group to select it, or drag a box. Clicking
//! an edge of a curve (a circle or arc) selects the whole curve. Dragging right selects what
//! lies entirely inside the box; dragging left also selects what the box touches.
//! With Shift held, clicked or boxed entities are toggled instead of replacing the selection.
//! Double-clicking a group opens it for editing; double-clicking outside it, or Escape, closes
//! it.

use glam::{DVec2, DVec3};
use kernel::geom::point_in_polygon;
use kernel::{EntityId, Face, Guide, Model};
use session::{Command, SelectMode};

use crate::inference::{edge_under_where, face_under_in, guide_under_where};
use crate::scenery::Owner;
use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};

#[derive(Clone, Debug, Default)]
pub struct SelectTool {
    /// Cursor position where the button went down, while it is held.
    press: Option<DVec2>,
    cursor: DVec2,
}

impl SelectTool {
    pub fn new() -> Self {
        Self::default()
    }
}

/// What a click picks: the open context's edge under the cursor, else its guide, else an
/// instance's edge (which picks the instance), else the face under it (the open context's
/// face, or the instance). Geometry outside the open instance hides what is behind it but is
/// not picked, and its edges do not outrank the open context's even when they lie on top of
/// them.
pub(crate) fn pick(ctx: &ToolContext) -> Option<EntityId> {
    let edge_of = |keep: fn(Owner) -> bool| {
        edge_under_where(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius, keep)
    };
    if let Some((_, e, _, _)) = edge_of(|o| o == Owner::Open) {
        return Some(EntityId::Edge(e));
    }
    let open = |o: Owner| o == Owner::Open;
    if let Some((_, g, _)) =
        guide_under_where(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius, open)
    {
        return Some(EntityId::Guide(g));
    }
    if let Some((Owner::Instance(i), ..)) = edge_of(|o| matches!(o, Owner::Instance(_))) {
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

/// True when some, but not all, of `ids` are selected.
fn partly_selected(ids: &[EntityId], ctx: &ToolContext) -> bool {
    let n = ids.iter().filter(|id| ctx.selection.contains(id)).count();
    n > 0 && n < ids.len()
}

/// The corners of an instance's bounding box (bit 0 picks x, bit 1 y, bit 2 z).
fn box_corners(ctx: &ToolContext, id: EntityId) -> Option<[DVec3; 8]> {
    let EntityId::Instance(i) = id else {
        return None;
    };
    let [lo, hi] = ctx.scenery.instance_box(i)?;
    Some(std::array::from_fn(|k| {
        DVec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        )
    }))
}

/// The twelve edges of an instance's bounding box, for outlines.
pub(crate) fn box_outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
    let Some(c) = box_corners(ctx, id) else {
        return Vec::new();
    };
    (0..8)
        .flat_map(|k| [1, 2, 4].map(|bit| (k, k | bit)))
        .filter(|(a, b)| a != b)
        .map(|(a, b)| [c[a], c[b]])
        .collect()
}

/// Edges, faces, instances and guides selected by the screen box from `a` to `b`: inside it
/// entirely when dragging right (`b.x >= a.x`), touching it when dragging left. Entities with
/// a point behind the camera are skipped. Guide lines are never inside a box; a box dragged
/// left selects those crossing it.
fn boxed(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<EntityId> {
    let (lo, hi) = (a.min(b), a.max(b));
    let window = b.x >= a.x;
    let m = ctx.model;
    let screen = |points: &[kernel::VertexId]| -> Option<Vec<DVec2>> {
        points
            .iter()
            .map(|v| m.position(*v).and_then(|p| ctx.camera.project(p)))
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
    for (id, e) in m.edges() {
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
    for (id, _) in m.instances() {
        let id = EntityId::Instance(id);
        let Some(corners) = box_corners(ctx, id) else {
            continue;
        };
        let Some(pts) = corners
            .iter()
            .map(|p| ctx.camera.project(*p))
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
    for (id, g) in m.guides() {
        let hit = match *g {
            Guide::Point { position } => ctx.camera.project(position).is_some_and(inside),
            Guide::Line { point, direction } => {
                !window
                    && ctx
                        .camera
                        .line_span(point, direction)
                        .and_then(|[a, b]| Some((ctx.camera.project(a)?, ctx.camera.project(b)?)))
                        .is_some_and(|(p, q)| touches(p, q))
            }
        };
        if hit {
            ids.push(EntityId::Guide(id));
        }
    }
    for (id, f) in m.faces() {
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
            ToolEvent::Move(_) => {}
            ToolEvent::Click(_) => self.press = Some(ctx.cursor),
            ToolEvent::Release(_) => {
                if let Some(start) = self.press.take() {
                    let clicked = start.distance(ctx.cursor) < ctx.drag_threshold;
                    let ids = if clicked {
                        pick(ctx).map_or(Vec::new(), |id| whole_curve(ctx.model, id))
                    } else {
                        boxed(ctx, start, ctx.cursor)
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
                        out.commands.push(Command::Select { ids, mode });
                    }
                }
            }
            ToolEvent::Value(_) => {
                out.message = Some("The Select tool takes no typed values".into());
            }
            ToolEvent::DoubleClick(_) => match pick(ctx) {
                Some(id @ EntityId::Instance(_)) => {
                    out.commands.push(Command::OpenInstance { id });
                }
                None if ctx.scenery.inside_instance() => {
                    out.commands.push(Command::CloseInstance);
                }
                _ => {}
            },
            // Escape with nothing in progress closes the open instance.
            ToolEvent::Cancel => {
                if self.press.take().is_none() && ctx.scenery.inside_instance() {
                    out.commands.push(Command::CloseInstance);
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
    }
}

/// Segments drawing a guide of the open context: the part of a guide line in view, or an X
/// as wide as the pick radius on screen around a guide point.
pub(crate) fn guide_outline(ctx: &ToolContext, g: &Guide) -> Vec<[DVec3; 2]> {
    match *g {
        Guide::Line { point, direction } => {
            ctx.camera.line_span(point, direction).into_iter().collect()
        }
        Guide::Point { position } => {
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
            }
        }

        fn px(&self, p: DVec3) -> DVec2 {
            self.camera.project(p).unwrap()
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &crate::scenery::Scenery::single(&self.model),
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                drag_threshold: self.drag_threshold,
                shift: self.shift,
                selection: &self.selection,
                material: None,
                sample: false,
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
        }]
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
        let c = model.add_curve(&ring, true).unwrap();
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
        let [Command::Select { ids, mode }] = &rim.commands[..] else {
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
            })
            .unwrap();
        r.model
            .add_guide(Guide::Line {
                point: DVec3::new(0.0, 3.0, 0.0),
                direction: DVec3::X,
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
                shift: false,
                selection: &Default::default(),
                material: None,
                sample: false,
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
        model.add_curve(&ring, true).unwrap();
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
        let [Command::Select { ids, mode }] = &out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert_eq!(
            (ids.len(), *mode),
            (8, SelectMode::Add),
            "adds the missing edges"
        );

        r.selection = all.iter().copied().collect();
        let out = r.click(rim);
        let [Command::Select { ids, mode }] = &out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert_eq!(
            (ids.len(), *mode),
            (8, SelectMode::Toggle),
            "removes all of it"
        );
    }
}
