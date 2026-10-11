//! The Select tool: click an edge or face to select it, or drag a box. Dragging right selects
//! what lies entirely inside the box; dragging left also selects what the box touches. With
//! Shift held, clicked or boxed entities are toggled instead of replacing the selection.

use glam::{DVec2, DVec3};
use kernel::{EntityId, Face, Model};
use session::{Command, SelectMode};

use crate::inference::{edge_under, face_under};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};

/// How far (physical pixels) the cursor must move between press and release to make a box.
const DRAG_PX: f64 = 4.0;

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

/// The edge under the cursor, or else the face under it.
pub(crate) fn pick(ctx: &ToolContext) -> Option<EntityId> {
    edge_under(ctx.model, ctx.camera, ctx.cursor, ctx.pick_radius)
        .map(EntityId::Edge)
        .or_else(|| face_under(ctx.model, ctx.camera, ctx.cursor).map(|(f, _)| EntityId::Face(f)))
}

/// Edges and faces selected by the screen box from `a` to `b`: inside it entirely when
/// dragging right (`b.x >= a.x`), touching it when dragging left. Entities with a point behind
/// the camera are skipped.
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
    let touches = |p: DVec2, q: DVec2| {
        let corners = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
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
    for (id, f) in m.faces() {
        let all = face_loops(f).all(|l| screen(l).is_some_and(|pts| !pts.is_empty()));
        let hit = if window {
            all && face_loops(f).all(|l| screen(l).is_some_and(|pts| chosen(true, &pts)))
        } else {
            face_loops(f).any(|l| screen(l).is_some_and(|pts| chosen(true, &pts)))
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
            None => "Click an edge or face, or drag a box; Shift adds or removes",
            Some(_) => "Release to select: drag right for inside the box, left for touching",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("", None)
    }

    fn anchor(&self) -> Option<DVec3> {
        None
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        self.cursor = ctx.cursor;
        match event {
            ToolEvent::Move(_) => {}
            ToolEvent::Click(_) => self.press = Some(ctx.cursor),
            ToolEvent::Release(_) => {
                if let Some(start) = self.press.take() {
                    let ids = if start.distance(ctx.cursor) < DRAG_PX {
                        pick(ctx).into_iter().collect()
                    } else {
                        boxed(ctx, start, ctx.cursor)
                    };
                    let mode = if ctx.shift {
                        SelectMode::Toggle
                    } else {
                        SelectMode::Replace
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
            ToolEvent::Cancel => self.press = None,
        }
        if let Some(start) = self.press
            && start.distance(self.cursor) >= DRAG_PX
        {
            out.preview = rubber_band(ctx, start, self.cursor);
        }
        out
    }

    fn on_external_change(&mut self) {
        self.press = None;
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
        shift: bool,
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
            }
        }

        fn px(&self, p: DVec3) -> DVec2 {
            self.camera.project(p).unwrap()
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                shift: self.shift,
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
}
