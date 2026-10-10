//! Window-independent editing state: routes input to navigation or the active tool, runs the
//! resulting commands on the document, and produces the overlay to draw.

use glam::{DVec2, DVec3};
use render::grid::LineVertex;
use session::{Command, Document};
use tools::overlay::overlay;
use tools::{Inference, LineTool, ToolEvent, ToolOutput, infer};

use crate::navigation::{Button, Navigation};

pub struct Editor {
    pub doc: Document,
    nav: Navigation,
    tool: LineTool,
    cursor: DVec2,
    hit: Option<Inference>,
    preview: Vec<[DVec3; 2]>,
    /// Snap radius in physical pixels.
    pick_radius_px: f64,
}

impl Editor {
    pub fn new(viewport: DVec2, pick_radius_px: f64) -> Self {
        Self {
            doc: Document::new(viewport),
            nav: Navigation::default(),
            tool: LineTool::new(),
            cursor: DVec2::ZERO,
            hit: None,
            preview: Vec::new(),
            pick_radius_px,
        }
    }

    pub fn resize(&mut self, viewport: DVec2) {
        self.doc.camera.viewport = viewport;
    }

    pub fn set_pick_radius(&mut self, px: f64) {
        self.pick_radius_px = px;
    }

    pub fn set_modifiers(&mut self, shift: bool, alt: bool) {
        self.nav.set_modifiers(shift, alt);
    }

    pub fn focus_lost(&mut self) {
        self.nav.reset();
    }

    /// Returns true if anything visible changed.
    pub fn cursor_moved(&mut self, pos: DVec2) -> bool {
        self.cursor = pos;
        let camera_moved = self.nav.cursor_moved(&mut self.doc.camera, pos);
        self.update_hover() || camera_moved
    }

    pub fn button(&mut self, button: Button, pressed: bool) -> bool {
        if self.nav.button(&self.doc.camera, button, pressed) {
            return false;
        }
        if button != Button::Left || !pressed {
            return false;
        }
        self.update_hover();
        let Some(hit) = self.hit else { return false };
        let out = self.tool.handle(ToolEvent::Click(hit));
        self.apply(out);
        self.update_hover();
        true
    }

    pub fn scroll(&mut self, steps: f64) -> bool {
        let zoomed = self.nav.scroll(&mut self.doc.camera, steps);
        if zoomed {
            self.update_hover();
        }
        zoomed
    }

    /// Escape: abandon the line being drawn.
    pub fn cancel(&mut self) -> bool {
        let out = self.tool.handle(ToolEvent::Cancel);
        self.apply(out);
        self.update_hover();
        true
    }

    pub fn undo(&mut self) -> bool {
        self.run(Command::Undo);
        self.update_hover();
        true
    }

    pub fn redo(&mut self) -> bool {
        self.run(Command::Redo);
        self.update_hover();
        true
    }

    /// Line pairs for the renderer's overlay pass.
    pub fn overlay(&self) -> Vec<LineVertex> {
        overlay(&self.preview, self.hit.as_ref(), &self.doc.camera)
            .into_iter()
            .flat_map(|s| {
                [s.a, s.b].map(|p| LineVertex {
                    pos: p.as_vec3().to_array(),
                    color: s.color,
                })
            })
            .collect()
    }

    fn apply(&mut self, out: ToolOutput) {
        for command in out.commands {
            self.run(command);
        }
        self.preview = out.preview;
    }

    fn run(&mut self, command: Command) {
        if let Err(e) = self.doc.execute(command) {
            eprintln!("command failed: {e}");
        }
    }

    /// Recomputes the inference under the cursor and the tool preview. True if either changed.
    fn update_hover(&mut self) -> bool {
        let hit = infer(
            self.doc.model(),
            &self.doc.camera,
            self.cursor,
            self.tool.anchor(),
            self.pick_radius_px,
        );
        let out = self.tool.handle(ToolEvent::Move(hit));
        let changed = self.hit != Some(hit) || self.preview != out.preview;
        self.hit = Some(hit);
        self.preview = out.preview;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::{Query, QueryResult};

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 10.0)
    }

    fn click_at(e: &mut Editor, p: DVec3) {
        let px = e.doc.camera.project(p).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }

    fn stats(e: &Editor) -> (usize, usize) {
        match e.doc.query(&Query::Stats).unwrap() {
            QueryResult::Stats { edges, faces, .. } => (edges, faces),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn square(e: &mut Editor) {
        for (x, y) in [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0), (1.0, 1.0)] {
            click_at(e, DVec3::new(x, y, 0.0));
        }
    }

    #[test]
    fn clicking_four_corners_draws_a_face() {
        let mut e = editor();
        square(&mut e);
        assert_eq!(stats(&e), (4, 1));
        let model = e.doc.model();
        let (id, _) = model.faces().next().unwrap();
        assert!((model.area(id).unwrap() - 4.0).abs() < 1e-6);
    }

    #[test]
    fn undo_and_redo_step_through_edges() {
        let mut e = editor();
        square(&mut e);
        e.undo();
        assert_eq!(stats(&e), (3, 0));
        e.redo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn preview_follows_the_cursor_until_cancelled() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let px = e.doc.camera.project(DVec3::new(-2.0, 4.0, 0.0)).unwrap();
        assert!(e.cursor_moved(px));
        assert!(!e.overlay().is_empty(), "preview line expected");
        e.cancel();
        assert!(
            e.overlay().is_empty(),
            "ground hover has no marker and no preview after cancel"
        );
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn alt_left_drag_orbits_without_drawing() {
        let mut e = editor();
        let before = e.doc.camera.clone();
        e.cursor_moved(DVec2::new(400.0, 300.0));
        e.set_modifiers(false, true);
        e.button(Button::Left, true);
        e.cursor_moved(DVec2::new(460.0, 300.0));
        e.button(Button::Left, false);
        assert_ne!(e.doc.camera, before);
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn hovering_an_endpoint_shows_a_marker() {
        let mut e = editor();
        square(&mut e);
        let px = e.doc.camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(px + DVec2::new(3.0, 0.0));
        assert_eq!(e.overlay().len(), 8, "a 4-sided marker, no preview");
    }
}
