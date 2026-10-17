//! Edit > Soften Edges and the Soft checkbox in Entity Info.

use kernel::{EntityId, SoftenReport};
use session::Command;

use super::{Editor, Message};
use crate::info::count;

/// The angle the Soften Edges window starts with, in degrees.
pub const DEFAULT_SOFTEN_ANGLE: u8 = 20;

/// The Soften Edges window: whether it is open, and its angle (0 to 180 degrees), kept for
/// the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoftenWindow {
    pub open: bool,
    pub angle: u8,
}

impl Default for SoftenWindow {
    fn default() -> Self {
        SoftenWindow {
            open: false,
            angle: DEFAULT_SOFTEN_ANGLE,
        }
    }
}

impl Editor {
    /// The Soften Edges window's angle while it is open.
    pub fn soften_window(&self) -> Option<u8> {
        self.soften.open.then_some(self.soften.angle)
    }

    /// Opens or closes the Soften Edges window.
    pub fn show_soften(&mut self, open: bool) -> bool {
        self.soften.open = open;
        true
    }

    /// Sets the Soften Edges window's angle, in degrees (at most 180).
    pub fn set_soften_angle(&mut self, angle: u8) -> bool {
        self.soften.angle = angle.min(180);
        true
    }

    /// Softens the selected edges and the edges of selected faces whose faces meet at no more
    /// than the window's angle, or every such edge of the open context when nothing is
    /// selected, as one undo step. The status says how many changed.
    pub fn soften_selection(&mut self) -> bool {
        let Some(ids) = self.soften_targets() else {
            return true;
        };
        let max_angle_deg = f64::from(self.soften.angle);
        if let Some(r) = self.run_soften(Command::SoftenByAngle { ids, max_angle_deg }) {
            self.message = Some(Message::info(changed("Softened", r)));
        }
        true
    }

    /// Hardens the selected edges and the edges of selected faces, or every edge of the open
    /// context when nothing is selected, as one undo step.
    pub fn harden_selection(&mut self) -> bool {
        let Some(mut ids) = self.soften_targets() else {
            return true;
        };
        if ids.is_empty() {
            ids = self
                .doc
                .model()
                .edges()
                .map(|(id, _)| EntityId::Edge(id))
                .collect();
        }
        if ids.is_empty() {
            self.message = Some(Message::info("No edges to harden"));
            return true;
        }
        if let Some(r) = self.run_soften(Command::SoftenEdges { ids, soft: false }) {
            self.message = Some(Message::info(changed("Hardened", r)));
        }
        true
    }

    /// Whether every selected edge is soft, when the selection is edges only (Entity Info's
    /// Soft checkbox); None otherwise.
    pub fn selection_soft(&self) -> Option<bool> {
        let selection = self.doc.selection();
        let m = self.doc.model();
        let mut soft = true;
        for id in selection {
            let EntityId::Edge(e) = id else { return None };
            soft &= m.edge(*e)?.soft;
        }
        (!selection.is_empty()).then_some(soft)
    }

    /// Makes the selected edges soft or hard, as one undo step (Entity Info's Soft checkbox).
    /// False when no edge is selected.
    pub fn set_selection_soft(&mut self, soft: bool) -> bool {
        let ids: Vec<EntityId> = self
            .doc
            .selection()
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Edge(_)))
            .collect();
        if ids.is_empty() {
            return false;
        }
        if let Some(r) = self.run_soften(Command::SoftenEdges { ids, soft }) {
            self.message = (r.changed == 0 && r.skipped > 0)
                .then(|| Message::error("Only an edge between two faces can be soft"));
        }
        true
    }

    /// The selected edges and faces, empty when nothing is selected; None (saying so in the
    /// status) when only other things are selected.
    fn soften_targets(&mut self) -> Option<Vec<EntityId>> {
        let selection = self.doc.selection();
        let ids: Vec<EntityId> = selection
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Edge(_) | EntityId::Face(_)))
            .collect();
        if !selection.is_empty() && ids.is_empty() {
            self.message = Some(Message::error("Select edges or faces to soften or harden"));
            return None;
        }
        Some(ids)
    }

    /// Runs a soften command as a panel action; its report, or None if it failed (the status
    /// then says why).
    fn run_soften(&mut self, command: Command) -> Option<SoftenReport> {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let out = self.run(command);
        self.invalidate_hover();
        out?.soften
    }
}

/// "Softened 3 edges", or "No edges changed".
fn changed(done: &str, r: SoftenReport) -> String {
    if r.changed == 0 {
        "No edges changed".into()
    } else {
        format!("{done} {}", count(r.changed, "edge"))
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use session::SelectMode;

    use super::*;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// A 2 x 2 x 1 m box.
    fn add_box(e: &mut Editor) {
        let out = e
            .execute(Command::AddRectangle {
                corner: [0.0, 0.0, 0.0],
                opposite: [2.0, 2.0, 0.0],
                normal: None,
            })
            .unwrap();
        let face = *out
            .changes
            .created
            .iter()
            .find(|id| matches!(id, EntityId::Face(_)))
            .unwrap();
        let EntityId::Face(f) = face else {
            unreachable!()
        };
        let up = e.doc().model().face(f).unwrap().normal.z.signum();
        e.execute(Command::PushPull { face, distance: up }).unwrap();
    }

    fn select(e: &mut Editor, ids: Vec<EntityId>) {
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
    }

    fn soft(e: &Editor) -> usize {
        e.doc().model().edges().filter(|(_, x)| x.soft).count()
    }

    fn message(e: &Editor) -> Option<String> {
        e.status_parts().message
    }

    #[test]
    fn the_window_keeps_its_angle_and_opens_and_closes() {
        let mut e = editor();
        assert_eq!(e.soften_window(), None);
        e.show_soften(true);
        assert_eq!(e.soften_window(), Some(DEFAULT_SOFTEN_ANGLE));
        e.set_soften_angle(200);
        assert_eq!(e.soften_window(), Some(180));
        e.show_soften(false);
        assert_eq!(e.soften_window(), None);
        e.show_soften(true);
        assert_eq!(e.soften_window(), Some(180));
    }

    #[test]
    fn soften_and_harden_take_the_open_context_when_nothing_is_selected() {
        let mut e = editor();
        add_box(&mut e);
        e.soften_selection();
        assert_eq!(
            soft(&e),
            0,
            "the box's faces meet at 90 degrees, more than 20"
        );
        assert_eq!(message(&e).as_deref(), Some("No edges changed"));
        e.set_soften_angle(90);
        e.soften_selection();
        assert_eq!(soft(&e), 12);
        assert_eq!(message(&e).as_deref(), Some("Softened 12 edges"));
        e.harden_selection();
        assert_eq!(soft(&e), 0);
        assert_eq!(message(&e).as_deref(), Some("Hardened 12 edges"));
        e.undo();
        assert_eq!(soft(&e), 12);
    }

    #[test]
    fn soften_takes_the_selected_faces_edges_and_refuses_other_selections() {
        let mut e = editor();
        add_box(&mut e);
        let m = e.doc().model();
        let top = m
            .faces()
            .find(|(_, f)| f.normal.z.abs() > 0.5 && m.position(f.outer[0]).unwrap().z > 0.5)
            .map(|(id, _)| EntityId::Face(id))
            .unwrap();
        select(&mut e, vec![top]);
        e.set_soften_angle(90);
        e.soften_selection();
        assert_eq!(soft(&e), 4);
        assert_eq!(message(&e).as_deref(), Some("Softened 4 edges"));

        e.execute(Command::AddGuidePoint {
            position: [5.0, 5.0, 0.0],
        })
        .unwrap();
        let guide = e
            .doc()
            .model()
            .guides()
            .map(|(id, _)| EntityId::Guide(id))
            .next()
            .unwrap();
        select(&mut e, vec![guide]);
        e.harden_selection();
        assert_eq!(soft(&e), 4);
        assert_eq!(
            message(&e).as_deref(),
            Some("Select edges or faces to soften or harden")
        );
    }

    #[test]
    fn the_soft_checkbox_shows_and_sets_the_selected_edges() {
        let mut e = editor();
        add_box(&mut e);
        e.execute(Command::AddEdge {
            start: [5.0, 0.0, 0.0],
            end: [6.0, 0.0, 0.0],
        })
        .unwrap();
        let m = e.doc().model();
        let lone = m
            .edges()
            .find(|(_, x)| m.position(x.v[0]).unwrap().x > 4.0)
            .map(|(id, _)| EntityId::Edge(id))
            .unwrap();
        let other = m
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .find(|id| *id != lone)
            .unwrap();
        assert_eq!(e.selection_soft(), None, "nothing selected");
        select(&mut e, vec![other]);
        assert_eq!(e.selection_soft(), Some(false));
        assert!(e.set_selection_soft(true));
        assert_eq!(e.selection_soft(), Some(true));
        assert_eq!(soft(&e), 1);

        select(&mut e, vec![lone]);
        assert!(e.set_selection_soft(true));
        assert_eq!(e.selection_soft(), Some(false));
        assert_eq!(
            message(&e).as_deref(),
            Some("Only an edge between two faces can be soft")
        );

        let face = e
            .doc()
            .model()
            .faces()
            .map(|(id, _)| EntityId::Face(id))
            .next()
            .unwrap();
        select(&mut e, vec![other, face]);
        assert_eq!(e.selection_soft(), None, "not edges only");
    }
}
