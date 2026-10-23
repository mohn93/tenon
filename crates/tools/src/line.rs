//! The Line tool: click to start, click again to place each edge; edges chain from the last point.
//! A drag from the start works as two clicks: it draws one edge, and the chain goes on from its
//! end. Typing a length places the next point that far along the current direction.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::inference::Inference;
use crate::tool::{DragStart, Phase, Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length, snap_distance_on_surface};

#[derive(Clone, Debug, Default)]
pub struct LineTool {
    /// First point of the current chain; clicking it again closes the chain.
    first: Option<DVec3>,
    /// Point the next edge starts from.
    last: Option<DVec3>,
    /// Latest inferred cursor point.
    hover: Option<DVec3>,
    drag_start: DragStart,
}

impl LineTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the edge from the chain's last point to `p` and continues (or closes) the chain.
    fn place(&mut self, last: DVec3, p: DVec3, out: &mut ToolOutput) {
        out.commands.push(Command::AddEdge {
            start: last.to_array(),
            end: p.to_array(),
        });
        if self.first.is_some_and(|f| f.distance(p) < LENGTH_TOL) {
            self.reset();
        } else {
            self.last = Some(p);
        }
    }

    /// Where the cursor is: its inference's point, at a multiple of the model's length snapping
    /// from the chain's last point unless a point of the model was snapped to.
    fn snapped(&self, hit: &Inference, ctx: &ToolContext) -> DVec3 {
        match self.last {
            Some(last) => snap_distance_on_surface(last, hit.point, hit, ctx),
            None => hit.point,
        }
    }

    fn reset(&mut self) {
        self.first = None;
        self.last = None;
        self.drag_start = DragStart::default();
    }
}

impl Tool for LineTool {
    fn name(&self) -> &'static str {
        "Line"
    }

    fn prompt(&self) -> &'static str {
        match self.last {
            None => "Click to start a line",
            Some(_) => "Click the next point, or type a length",
        }
    }

    fn measurement(&self, units: &session::Units) -> (&'static str, Option<String>) {
        let length = self
            .last
            .zip(self.hover)
            .map(|(a, b)| format_length(a.distance(b), units));
        ("Length", length)
    }

    fn anchor(&self) -> Option<DVec3> {
        self.last
    }

    /// The chain's first point and the point the next edge starts from.
    fn phase(&self) -> Phase {
        match (self.first, self.last) {
            (Some(first), Some(last)) if first != last => Phase::Active {
                points: vec![first, last],
            },
            (_, Some(last)) => Phase::Active { points: vec![last] },
            _ => Phase::Idle,
        }
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(self.snapped(&hit, ctx)),
            ToolEvent::Click(hit) => {
                let p = self.snapped(&hit, ctx);
                self.hover = Some(p);
                match self.last {
                    None => {
                        self.first = Some(p);
                        self.last = Some(p);
                        self.drag_start.arm(ctx.cursor);
                    }
                    Some(last) if last.distance(p) < LENGTH_TOL => {}
                    Some(last) => self.place(last, p, &mut out),
                }
            }
            ToolEvent::Release(hit) => {
                if self.drag_start.released(ctx) {
                    return self.handle(ToolEvent::Click(hit), ctx);
                }
            }
            ToolEvent::Value(text) => {
                let dir = self
                    .last
                    .zip(self.hover)
                    .map(|(a, b)| b - a)
                    .filter(|d| d.length() >= LENGTH_TOL);
                match (self.last, dir, parse_length(&text, &ctx.units)) {
                    (None, _, _) | (_, None, _) => {
                        out.message = Some("Move the cursor to set a direction first".into());
                    }
                    (_, _, None) => out.message = Some(format!("Not a length: {text}")),
                    (Some(_), Some(_), Some(len)) if len.abs() < LENGTH_TOL => {
                        out.message = Some("Length must not be zero".into());
                    }
                    (Some(last), Some(dir), Some(len)) => {
                        let p = last + dir.normalize() * len;
                        self.place(last, p, &mut out);
                    }
                }
            }
            ToolEvent::DoubleClick(_) | ToolEvent::TripleClick(_) | ToolEvent::ToggleCopy => {}
            ToolEvent::Cancel => self.reset(),
        }
        if let (Some(a), Some(b)) = (self.last, self.hover)
            && a.distance(b) >= LENGTH_TOL
        {
            out.preview.push([a, b]);
        }
        out
    }

    fn on_external_change(&mut self) {
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, SnapKind};
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;

    fn at(x: f64, y: f64) -> Inference {
        Inference {
            point: DVec3::new(x, y, 0.0),
            kind: SnapKind::Ground,
            face: None,
            locked: None,
        }
    }

    fn edge(a: (f64, f64), b: (f64, f64)) -> Command {
        Command::AddEdge {
            start: [a.0, a.1, 0.0],
            end: [b.0, b.1, 0.0],
        }
    }

    /// Runs events through a Line tool with an empty model.
    struct Rig {
        tool: LineTool,
        model: Model,
        camera: Camera,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                tool: LineTool::new(),
                model: Model::new(),
                camera: Camera::new(DVec2::new(800.0, 600.0)),
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let ctx = ToolContext {
                model: &self.model,
                scenery: &crate::scenery::Scenery::single(&self.model),
                camera: &self.camera,
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
            self.tool.handle(event, &ctx)
        }

        fn click(&mut self, x: f64, y: f64) -> ToolOutput {
            self.send(ToolEvent::Click(at(x, y)))
        }
    }

    #[test]
    fn first_click_starts_and_moving_previews() {
        let mut r = Rig::new();
        assert!(r.click(0.0, 0.0).commands.is_empty());
        let out = r.send(ToolEvent::Move(at(2.0, 0.0)));
        assert_eq!(out.preview, vec![[DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0)]]);
        assert_eq!(r.tool.anchor(), Some(DVec3::ZERO));
        assert_eq!(
            r.tool.measurement(&Default::default()),
            ("Length", Some("2 m".to_string()))
        );
    }

    #[test]
    fn clicks_chain_edges() {
        let mut r = Rig::new();
        r.click(0.0, 0.0);
        let a = r.click(2.0, 0.0);
        let b = r.click(2.0, 1.0);
        assert_eq!(a.commands, vec![edge((0.0, 0.0), (2.0, 0.0))]);
        assert_eq!(b.commands, vec![edge((2.0, 0.0), (2.0, 1.0))]);
        assert_eq!(r.tool.anchor(), Some(DVec3::new(2.0, 1.0, 0.0)));
    }

    #[test]
    fn clicking_the_first_point_closes_the_chain() {
        let mut r = Rig::new();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)] {
            r.click(x, y);
        }
        let out = r.click(0.0, 0.0);
        assert_eq!(out.commands, vec![edge((1.0, 1.0), (0.0, 0.0))]);
        assert_eq!(r.tool.anchor(), None);
        assert!(out.preview.is_empty());
    }

    #[test]
    fn clicking_the_same_point_twice_does_nothing() {
        let mut r = Rig::new();
        r.click(1.0, 1.0);
        assert!(r.click(1.0, 1.0).commands.is_empty());
        assert_eq!(r.tool.anchor(), Some(DVec3::new(1.0, 1.0, 0.0)));
    }

    #[test]
    fn cancel_drops_the_chain() {
        let mut r = Rig::new();
        r.click(0.0, 0.0);
        let out = r.send(ToolEvent::Cancel);
        assert!(out.preview.is_empty() && out.commands.is_empty());
        assert_eq!(r.tool.anchor(), None);
    }

    #[test]
    fn typed_length_places_the_point_along_the_cursor_direction() {
        let mut r = Rig::new();
        r.click(1.0, 1.0);
        r.send(ToolEvent::Move(at(1.0, 5.0)));
        let out = r.send(ToolEvent::Value("2.5".into()));
        assert_eq!(out.commands, vec![edge((1.0, 1.0), (1.0, 3.5))]);
        assert_eq!(r.tool.anchor(), Some(DVec3::new(1.0, 3.5, 0.0)));
        let back = r.send(ToolEvent::Value("-500mm".into()));
        assert_eq!(back.commands, vec![edge((1.0, 3.5), (1.0, 3.0))]);
    }

    #[test]
    fn typed_text_that_cannot_be_used_explains_why() {
        let mut r = Rig::new();
        assert!(r.send(ToolEvent::Value("2".into())).message.is_some());
        r.click(0.0, 0.0);
        r.send(ToolEvent::Move(at(3.0, 0.0)));
        let out = r.send(ToolEvent::Value("two".into()));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Not a length: two"));
    }

    #[test]
    fn external_change_drops_the_chain() {
        let mut r = Rig::new();
        r.click(0.0, 0.0);
        r.tool.on_external_change();
        assert_eq!(r.tool.anchor(), None);
    }
}
