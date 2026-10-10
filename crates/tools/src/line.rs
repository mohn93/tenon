//! The Line tool: click to start, click again to place each edge; edges chain from the last point.

use glam::DVec3;
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::inference::Inference;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolEvent {
    /// The cursor moved; carries the current inference.
    Move(Inference),
    /// The primary button was pressed at the current inference.
    Click(Inference),
    /// Escape: abandon the current chain.
    Cancel,
}

/// What a tool wants done after an event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolOutput {
    /// Commands to execute, in order (each is one undo step).
    pub commands: Vec<Command>,
    /// Segments to draw as a live preview.
    pub preview: Vec<[DVec3; 2]>,
}

#[derive(Clone, Debug, Default)]
pub struct LineTool {
    /// First point of the current chain; clicking it again closes the chain.
    first: Option<DVec3>,
    /// Point the next edge starts from.
    last: Option<DVec3>,
    /// Latest inferred cursor point.
    hover: Option<DVec3>,
}

impl LineTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The point the next edge would start from, if a chain is in progress.
    pub fn anchor(&self) -> Option<DVec3> {
        self.last
    }

    pub fn handle(&mut self, event: ToolEvent) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => {
                let p = hit.point;
                self.hover = Some(p);
                match self.last {
                    None => {
                        self.first = Some(p);
                        self.last = Some(p);
                    }
                    Some(last) if last.distance(p) < LENGTH_TOL => {}
                    Some(last) => {
                        out.commands.push(Command::AddEdge {
                            start: last.to_array(),
                            end: p.to_array(),
                        });
                        if self.first.is_some_and(|f| f.distance(p) < LENGTH_TOL) {
                            *self = Self {
                                hover: self.hover,
                                ..Self::default()
                            };
                        } else {
                            self.last = Some(p);
                        }
                    }
                }
            }
            ToolEvent::Cancel => {
                *self = Self {
                    hover: self.hover,
                    ..Self::default()
                }
            }
        }
        if let (Some(a), Some(b)) = (self.last, self.hover)
            && a.distance(b) >= LENGTH_TOL
        {
            out.preview.push([a, b]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::SnapKind;

    fn at(x: f64, y: f64) -> Inference {
        Inference {
            point: DVec3::new(x, y, 0.0),
            kind: SnapKind::Ground,
        }
    }

    fn edge(a: (f64, f64), b: (f64, f64)) -> Command {
        Command::AddEdge {
            start: [a.0, a.1, 0.0],
            end: [b.0, b.1, 0.0],
        }
    }

    #[test]
    fn first_click_starts_and_moving_previews() {
        let mut t = LineTool::new();
        assert!(t.handle(ToolEvent::Click(at(0.0, 0.0))).commands.is_empty());
        let out = t.handle(ToolEvent::Move(at(2.0, 0.0)));
        assert_eq!(out.preview, vec![[DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0)]]);
        assert_eq!(t.anchor(), Some(DVec3::ZERO));
    }

    #[test]
    fn clicks_chain_edges() {
        let mut t = LineTool::new();
        t.handle(ToolEvent::Click(at(0.0, 0.0)));
        let a = t.handle(ToolEvent::Click(at(2.0, 0.0)));
        let b = t.handle(ToolEvent::Click(at(2.0, 1.0)));
        assert_eq!(a.commands, vec![edge((0.0, 0.0), (2.0, 0.0))]);
        assert_eq!(b.commands, vec![edge((2.0, 0.0), (2.0, 1.0))]);
        assert_eq!(t.anchor(), Some(DVec3::new(2.0, 1.0, 0.0)));
    }

    #[test]
    fn clicking_the_first_point_closes_the_chain() {
        let mut t = LineTool::new();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)] {
            t.handle(ToolEvent::Click(at(x, y)));
        }
        let out = t.handle(ToolEvent::Click(at(0.0, 0.0)));
        assert_eq!(out.commands, vec![edge((1.0, 1.0), (0.0, 0.0))]);
        assert_eq!(t.anchor(), None);
        assert!(out.preview.is_empty());
    }

    #[test]
    fn clicking_the_same_point_twice_does_nothing() {
        let mut t = LineTool::new();
        t.handle(ToolEvent::Click(at(1.0, 1.0)));
        assert!(t.handle(ToolEvent::Click(at(1.0, 1.0))).commands.is_empty());
        assert_eq!(t.anchor(), Some(DVec3::new(1.0, 1.0, 0.0)));
    }

    #[test]
    fn cancel_drops_the_chain() {
        let mut t = LineTool::new();
        t.handle(ToolEvent::Click(at(0.0, 0.0)));
        let out = t.handle(ToolEvent::Cancel);
        assert!(out.preview.is_empty() && out.commands.is_empty());
        assert_eq!(t.anchor(), None);
    }
}
