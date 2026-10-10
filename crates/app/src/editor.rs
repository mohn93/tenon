//! Window-independent editing state: routes input to navigation or the active tool, runs the
//! resulting commands on the document, and produces the overlay and status text to show.

use glam::{DVec2, DVec3};
use render::grid::LineVertex;
use session::{BatchError, Command, CommandError, Document, ErrorKind, Outcome};
use tools::overlay::overlay;
use tools::{Inference, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput, infer};

use crate::navigation::{Button, Navigation};

/// Snap radius in logical pixels (scaled to physical pixels by the display scale factor).
const PICK_RADIUS_LOGICAL_PX: f64 = 8.0;

/// Keys the editor handles itself (navigation and undo shortcuts are handled elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKey {
    Char(char),
    Enter,
    Backspace,
    Escape,
}

pub struct Editor {
    doc: Document,
    nav: Navigation,
    tool: Box<dyn Tool>,
    cursor: DVec2,
    hit: Option<Inference>,
    preview: Vec<[DVec3; 2]>,
    /// Text typed into the measurements box, not yet submitted.
    typed: String,
    /// Feedback from the last tool action or failed command.
    message: Option<String>,
    /// Physical pixels per logical pixel.
    scale_factor: f64,
}

impl Editor {
    pub fn new(viewport: DVec2, scale_factor: f64) -> Self {
        Self {
            doc: Document::new(viewport),
            nav: Navigation::default(),
            tool: ToolKind::Line.make(),
            cursor: DVec2::ZERO,
            hit: None,
            preview: Vec::new(),
            typed: String::new(),
            message: None,
            scale_factor,
        }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// Runs a command from outside the UI (the agent API). A command that can change the model
    /// ends the tool's operation in progress, which may refer to geometry that changed.
    pub fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        if command.changes_model() {
            self.reset_tool();
        }
        let result = self.doc.execute(command);
        self.update_hover();
        result
    }

    /// Batch version of `execute`.
    pub fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        if commands.iter().any(Command::changes_model) {
            self.reset_tool();
        }
        let result = self.doc.execute_batch(commands, atomic);
        self.update_hover();
        result
    }

    /// Switches to another tool, dropping the current tool's operation and any typed text.
    pub fn select_tool(&mut self, kind: ToolKind) {
        self.tool = kind.make();
        self.preview.clear();
        self.typed.clear();
        self.message = None;
        self.update_hover();
    }

    pub fn resize(&mut self, viewport: DVec2) {
        self.doc.camera.viewport = viewport;
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
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

    /// The cursor left the window: hide the inference marker.
    pub fn cursor_left(&mut self) -> bool {
        self.hit.take().is_some()
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
        self.send(ToolEvent::Click(hit));
        true
    }

    pub fn scroll(&mut self, steps: f64) -> bool {
        let zoomed = self.nav.scroll(&mut self.doc.camera, steps);
        if zoomed {
            self.update_hover();
        }
        zoomed
    }

    /// Typing goes to the measurements box once it starts with a digit, "." or "-"; other
    /// letters are tool shortcuts. Enter submits the typed text to the tool. Escape clears the
    /// typed text, or else cancels the tool's operation. Returns true if anything changed.
    pub fn key(&mut self, key: EditorKey) -> bool {
        match key {
            EditorKey::Char(c) if self.typed.is_empty() => {
                if c.is_ascii_digit() || c == '.' || c == '-' {
                    self.typed.push(c);
                    true
                } else if let Some(kind) = ToolKind::from_shortcut(c) {
                    self.select_tool(kind);
                    true
                } else {
                    false
                }
            }
            EditorKey::Char(c) => {
                if !c.is_control() {
                    self.typed.push(c);
                }
                true
            }
            EditorKey::Backspace => self.typed.pop().is_some(),
            EditorKey::Enter if self.typed.is_empty() => false,
            EditorKey::Enter => {
                let text = std::mem::take(&mut self.typed);
                self.send(ToolEvent::Value(text));
                true
            }
            EditorKey::Escape if !self.typed.is_empty() => {
                self.typed.clear();
                true
            }
            EditorKey::Escape => {
                self.send(ToolEvent::Cancel);
                true
            }
        }
    }

    pub fn undo(&mut self) -> bool {
        self.reset_tool();
        self.run(Command::Undo);
        self.update_hover();
        true
    }

    pub fn redo(&mut self) -> bool {
        self.reset_tool();
        self.run(Command::Redo);
        self.update_hover();
        true
    }

    /// One line describing the tool, what to do next, the measurement and any feedback.
    pub fn status(&self) -> String {
        let (label, value) = self.tool.measurement();
        let mut parts = vec![self.tool.name().to_string(), self.tool.prompt().to_string()];
        if !self.typed.is_empty() {
            parts.push(format!("{label}: {}_", self.typed));
        } else if let Some(value) = value {
            parts.push(format!("{label}: {value}"));
        }
        parts.extend(self.message.clone());
        parts.join("  |  ")
    }

    /// Line pairs for the renderer's overlay pass.
    pub fn overlay(&self) -> Vec<LineVertex> {
        overlay(
            &self.preview,
            self.hit.as_ref(),
            &self.doc.camera,
            self.scale_factor,
        )
        .into_iter()
        .flat_map(|s| {
            [s.a, s.b].map(|p| LineVertex {
                pos: p.as_vec3().to_array(),
                color: s.color,
            })
        })
        .collect()
    }

    /// Sends a click, typed value or cancel to the tool and runs what it asks for.
    fn send(&mut self, event: ToolEvent) {
        let out = self.handle(event);
        self.message = out.message.clone();
        self.apply(out);
        self.update_hover();
    }

    fn handle(&mut self, event: ToolEvent) -> ToolOutput {
        let ctx = ToolContext {
            model: self.doc.model(),
            camera: &self.doc.camera,
            cursor: self.cursor,
        };
        self.tool.handle(event, &ctx)
    }

    fn apply(&mut self, out: ToolOutput) {
        for command in out.commands {
            self.run(command);
        }
        self.preview = out.preview;
    }

    /// Drops the tool's operation in progress so it can't refer to geometry that changed.
    fn reset_tool(&mut self) {
        self.tool.on_external_change();
        self.preview.clear();
    }

    fn run(&mut self, command: Command) {
        match self.doc.execute(command) {
            Ok(_) => {}
            Err(e) if matches!(e.kind, ErrorKind::NothingToUndo | ErrorKind::NothingToRedo) => {}
            Err(e) => self.message = Some(e.message),
        }
    }

    /// Recomputes the inference under the cursor and the tool preview. True if either changed.
    fn update_hover(&mut self) -> bool {
        let hit = infer(
            self.doc.model(),
            &self.doc.camera,
            self.cursor,
            self.tool.anchor(),
            PICK_RADIUS_LOGICAL_PX * self.scale_factor,
        );
        let out = self.handle(ToolEvent::Move(hit));
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
        Editor::new(DVec2::new(800.0, 600.0), 1.25)
    }

    fn click_at(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }

    fn stats(e: &Editor) -> (usize, usize) {
        match e.doc().query(&Query::Stats).unwrap() {
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
        let model = e.doc().model();
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
        let px = e.doc().camera.project(DVec3::new(-2.0, 4.0, 0.0)).unwrap();
        assert!(e.cursor_moved(px));
        assert!(!e.overlay().is_empty(), "preview line expected");
        e.key(EditorKey::Escape);
        assert!(
            e.overlay().is_empty(),
            "ground hover has no marker and no preview after cancel"
        );
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn undo_mid_chain_cancels_the_chain() {
        let mut e = editor();
        for (x, y) in [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0)] {
            click_at(&mut e, DVec3::new(x, y, 0.0));
        }
        e.undo();
        click_at(&mut e, DVec3::new(5.0, 5.0, 0.0));
        assert_eq!(stats(&e).0, 1, "the click only starts a new chain");
    }

    #[test]
    fn alt_left_drag_orbits_without_drawing() {
        let mut e = editor();
        let before = e.doc().camera.clone();
        e.cursor_moved(DVec2::new(400.0, 300.0));
        e.set_modifiers(false, true);
        e.button(Button::Left, true);
        e.cursor_moved(DVec2::new(460.0, 300.0));
        e.button(Button::Left, false);
        assert_ne!(e.doc().camera, before);
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn hovering_an_endpoint_shows_a_marker() {
        let mut e = editor();
        square(&mut e);
        let px = e.doc().camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(px + DVec2::new(3.0, 0.0));
        assert_eq!(e.overlay().len(), 8, "a 4-sided marker, no preview");
    }

    #[test]
    fn api_edit_mid_chain_cancels_the_chain() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let out = e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [6.0, 5.0, 0.0],
        });
        assert!(out.is_ok());
        click_at(&mut e, DVec3::new(-2.0, -2.0, 0.0));
        assert_eq!(
            stats(&e),
            (1, 0),
            "the click starts a new chain instead of adding an edge"
        );
    }

    #[test]
    fn api_camera_move_keeps_the_chain() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        e.execute(Command::SetCamera {
            eye: [-6.0, -10.0, 6.0],
            target: [0.0, 0.0, 0.0],
            fov_y_deg: None,
        })
        .unwrap();
        click_at(&mut e, DVec3::new(3.0, 1.0, 0.0));
        assert_eq!(stats(&e), (1, 0), "the chain continued");
    }

    fn move_to(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
    }

    fn type_text(e: &mut Editor, text: &str) {
        for c in text.chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
    }

    #[test]
    fn typing_shows_in_the_status_and_escape_clears_it_first() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        e.key(EditorKey::Char('2'));
        e.key(EditorKey::Char('.'));
        e.key(EditorKey::Char('5'));
        e.key(EditorKey::Backspace);
        assert!(e.status().contains("Length: 2._"), "{}", e.status());
        e.key(EditorKey::Escape);
        assert!(!e.status().contains('_'));
        assert!(e.status().contains("Click the next point"), "chain kept");
        e.key(EditorKey::Escape);
        assert!(e.status().contains("Click to start a line"));
    }

    #[test]
    fn typed_length_draws_the_next_edge() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(4.0, 1.0, 0.0));
        type_text(&mut e, "2");
        assert_eq!(stats(&e), (1, 0));
        let end = DVec3::new(3.0, 1.0, 0.0);
        let model = e.doc().model();
        assert!(model.vertices().any(|(_, v)| v.pos.abs_diff_eq(end, 1e-9)));
    }

    #[test]
    fn letters_are_shortcuts_only_when_nothing_is_typed() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        assert!(
            e.key(EditorKey::Char('l')),
            "selecting Line again restarts it"
        );
        assert!(e.status().contains("Click to start a line"));
        assert!(!e.key(EditorKey::Char('z')));
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        e.key(EditorKey::Char('1'));
        e.key(EditorKey::Char('m'));
        assert!(e.status().contains("Length: 1m_"), "{}", e.status());
    }

    #[test]
    fn rectangle_tool_draws_one_undo_step_per_rectangle() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('r')));
        assert!(e.status().starts_with("Rectangle"), "{}", e.status());
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert_eq!(stats(&e), (4, 1));
        e.undo();
        assert_eq!(stats(&e), (0, 0));
    }

    #[test]
    fn typed_size_finishes_a_rectangle() {
        let mut e = editor();
        e.key(EditorKey::Char('r'));
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        move_to(&mut e, DVec3::new(1.0, 1.0, 0.0));
        type_text(&mut e, "3,2");
        let model = e.doc().model();
        let (face, _) = model.faces().next().unwrap();
        assert!((model.area(face).unwrap() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn leaving_the_window_hides_the_marker() {
        let mut e = editor();
        square(&mut e);
        let px = e.doc().camera.project(DVec3::new(3.0, 1.0, 0.0)).unwrap();
        e.cursor_moved(px);
        assert!(!e.overlay().is_empty());
        assert!(e.cursor_left());
        assert!(e.overlay().is_empty());
    }
}
