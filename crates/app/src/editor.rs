//! Window-independent editing state: routes input to navigation or the active tool, runs the
//! resulting commands on the document, and produces the overlay and status text to show.

use glam::{DVec2, DVec3};
use kernel::EntityId;
use render::grid::LineVertex;
use session::{BatchError, Command, CommandError, Document, ErrorKind, Outcome, SelectMode};
use tools::overlay::overlay;
use tools::{
    Axis, Guides, Inference, Lock, SnapKind, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput,
    edge_under, infer_with,
};

use crate::navigation::{Button, Navigation};

/// Snap radius in logical pixels (scaled to physical pixels by the display scale factor).
const PICK_RADIUS_LOGICAL_PX: f64 = 8.0;
/// How far (logical px) the cursor must travel from a press for it to be a drag.
const DRAG_LOGICAL_PX: f64 = 4.0;

/// Keys the editor handles itself (navigation and undo shortcuts are handled elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKey {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Escape,
    /// Lock the direction to an axis during an operation, or unlock it if it is locked there.
    LockAxis(Axis),
    /// Remove any direction lock.
    Unlock,
}

/// What the status bar shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub tool: &'static str,
    pub prompt: &'static str,
    /// The measurements box label, e.g. "Length"; empty when the tool measures nothing.
    pub label: &'static str,
    /// Typed text (with a trailing "_") while typing, otherwise the live measurement.
    pub value: Option<String>,
    /// Feedback from the last tool action or failed command.
    pub message: Option<String>,
    /// The direction lock in force, e.g. "Locked: red axis".
    pub lock: Option<&'static str>,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}  |  {}", self.tool, self.prompt)?;
        if let Some(lock) = self.lock {
            write!(f, "  |  {lock}")?;
        }
        if let Some(value) = &self.value {
            write!(f, "  |  {}: {value}", self.label)?;
        }
        if let Some(message) = &self.message {
            write!(f, "  |  {message}")?;
        }
        Ok(())
    }
}

pub struct Editor {
    doc: Document,
    nav: Navigation,
    tool: Box<dyn Tool>,
    kind: ToolKind,
    /// Whether Shift is held, as last reported by the window.
    shift: bool,
    /// Whether the left-button press reached the tool, so its release should too.
    tool_pressed: bool,
    /// Direction of the last edge the cursor rested on, for parallel/perpendicular snaps.
    reference: Option<DVec3>,
    /// The direction lock in force during the current operation.
    lock: Option<Lock>,
    /// Whether `lock` comes from holding Shift (and ends when it is released).
    shift_lock: bool,
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
            kind: ToolKind::Line,
            shift: false,
            tool_pressed: false,
            reference: None,
            lock: None,
            shift_lock: false,
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
        self.kind = kind;
        self.preview.clear();
        self.typed.clear();
        self.message = None;
        self.update_hover();
    }

    pub fn tool_kind(&self) -> ToolKind {
        self.kind
    }

    pub fn resize(&mut self, viewport: DVec2) {
        self.doc.camera.viewport = viewport;
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }

    /// Pressing Shift during an operation locks the direction the cursor follows (an axis,
    /// parallel or perpendicular); releasing it unlocks. Returns true if the lock changed.
    pub fn set_modifiers(&mut self, shift: bool, alt: bool) -> bool {
        let pressed = shift && !self.shift;
        let released = !shift && self.shift;
        self.shift = shift;
        self.nav.set_modifiers(shift, alt);
        if pressed && self.lock.is_none() {
            let (Some(anchor), Some(hit)) = (self.tool.anchor(), self.hit) else {
                return false;
            };
            let dir = match hit.kind {
                SnapKind::Axis(a) => Some(a.dir()),
                SnapKind::Parallel | SnapKind::Perpendicular => {
                    (hit.point - anchor).try_normalize()
                }
                _ => None,
            };
            if let Some(dir) = dir {
                self.lock = Some(Lock {
                    dir,
                    kind: hit.kind,
                });
                self.shift_lock = true;
                self.update_hover();
                return true;
            }
        } else if released && self.shift_lock {
            self.lock = None;
            self.shift_lock = false;
            self.update_hover();
            return true;
        }
        false
    }

    /// Returns true if a Shift lock ended, so the view needs a redraw.
    pub fn focus_lost(&mut self) -> bool {
        self.shift = false;
        let unlocked = std::mem::take(&mut self.shift_lock);
        if unlocked {
            self.lock = None;
            self.update_hover();
        }
        self.nav.reset();
        unlocked
    }

    /// A key-repeat event. Backspace only deletes typed text and Delete does nothing, so holding
    /// them never erases the selection; other keys behave as in [`Editor::key`].
    pub fn key_repeat(&mut self, key: EditorKey) -> bool {
        match key {
            EditorKey::Backspace => self.typed.pop().is_some(),
            // Holding an arrow key must not toggle the lock at the repeat rate.
            EditorKey::Delete | EditorKey::LockAxis(_) | EditorKey::Unlock => false,
            _ => self.key(key),
        }
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

    /// The copy modifier (Ctrl) was tapped.
    pub fn toggle_copy(&mut self) -> bool {
        self.send(ToolEvent::ToggleCopy);
        true
    }

    pub fn button(&mut self, button: Button, pressed: bool) -> bool {
        if self.nav.button(&self.doc.camera, button, pressed) {
            return false;
        }
        if button != Button::Left {
            return false;
        }
        // A release reaches the tool only if its press did (not one that began on a panel or
        // in another tool).
        if !pressed && !std::mem::take(&mut self.tool_pressed) {
            return false;
        }
        self.update_hover();
        let Some(hit) = self.hit else { return false };
        self.tool_pressed = pressed;
        self.send(if pressed {
            ToolEvent::Click(hit)
        } else {
            ToolEvent::Release(hit)
        });
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
    /// typed text, or else cancels the tool's operation. Delete, and Backspace with nothing
    /// typed, erase the selection. Returns true if anything changed.
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
            EditorKey::Backspace if self.typed.is_empty() => self.erase_selection(),
            EditorKey::Backspace => self.typed.pop().is_some(),
            EditorKey::Delete => self.erase_selection(),
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
            EditorKey::LockAxis(axis) => {
                if self.tool.anchor().is_none() {
                    return false;
                }
                let same = self.lock.is_some_and(|l| l.kind == SnapKind::Axis(axis));
                self.lock = (!same).then_some(Lock {
                    dir: axis.dir(),
                    kind: SnapKind::Axis(axis),
                });
                self.shift_lock = false;
                self.update_hover();
                true
            }
            EditorKey::Unlock => {
                let had = self.lock.take().is_some();
                self.shift_lock = false;
                self.update_hover();
                had
            }
        }
    }

    /// Erases the selected edges and faces as one undo step. False if nothing is selected.
    pub fn erase_selection(&mut self) -> bool {
        if self.doc.selection().is_empty() {
            return false;
        }
        let ids = self.doc.selection().iter().copied().collect();
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        self.run(Command::Erase { ids });
        self.update_hover();
        true
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

    /// The status as one line: tool, prompt, measurement and feedback separated by bars.
    #[cfg(test)]
    pub fn status(&self) -> String {
        self.status_parts().to_string()
    }

    /// The tool, what to do next, the measurement and any feedback.
    pub fn status_parts(&self) -> Status {
        let (label, value) = self.tool.measurement();
        let value = if self.typed.is_empty() {
            value
        } else {
            Some(format!("{}_", self.typed))
        };
        let lock = self.lock.map(|l| match l.kind {
            SnapKind::Axis(Axis::X) => "Locked: red axis",
            SnapKind::Axis(Axis::Y) => "Locked: green axis",
            SnapKind::Axis(Axis::Z) => "Locked: blue axis",
            SnapKind::Parallel => "Locked: parallel",
            _ => "Locked: perpendicular",
        });
        Status {
            tool: self.tool.name(),
            prompt: self.tool.prompt(),
            label,
            value,
            message: self.message.clone(),
            lock,
        }
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

    /// Sends a click, release, typed value or cancel to the tool and runs what it asks for.
    fn send(&mut self, event: ToolEvent) {
        let is_release = matches!(event, ToolEvent::Release(_));
        let out = self.handle(event);
        // A release that did nothing keeps the message from the click that preceded it.
        let keep = is_release && out.commands.is_empty() && out.message.is_none();
        if !keep {
            self.message = out.message.clone();
        }
        self.apply(out);
        self.update_hover();
    }

    fn handle(&mut self, event: ToolEvent) -> ToolOutput {
        let ctx = ToolContext {
            model: self.doc.model(),
            camera: &self.doc.camera,
            cursor: self.cursor,
            pick_radius: PICK_RADIUS_LOGICAL_PX * self.scale_factor,
            drag_threshold: DRAG_LOGICAL_PX * self.scale_factor,
            shift: self.shift,
            selection: self.doc.selection(),
        };
        self.tool.handle(event, &ctx)
    }

    /// Runs the tool's commands in order; a copy selects what it created, as in SketchUp.
    /// The first command that fails stops the rest. If it fails after an undo from the same
    /// list (an array replacing a copy), the undo is redone, and the tool drops its state.
    fn apply(&mut self, out: ToolOutput) {
        let mut undone = false;
        for command in out.commands {
            let is_undo = matches!(command, Command::Undo);
            let is_copy = matches!(command, Command::Copy { .. });
            let Some(outcome) = self.run(command) else {
                if undone {
                    let _ = self.doc.execute(Command::Redo);
                }
                self.reset_tool();
                return;
            };
            undone |= is_undo;
            if is_copy {
                let ids = outcome
                    .changes
                    .created
                    .into_iter()
                    .filter(|id| !matches!(id, EntityId::Vertex(_)))
                    .collect();
                self.run(Command::Select {
                    ids,
                    mode: SelectMode::Replace,
                });
            }
        }
        self.preview = out.preview;
    }

    /// Drops the tool's operation in progress so it can't refer to geometry that changed.
    fn reset_tool(&mut self) {
        self.tool.on_external_change();
        self.preview.clear();
    }

    /// Runs a command; a failure becomes the status message.
    fn run(&mut self, command: Command) -> Option<Outcome> {
        match self.doc.execute(command) {
            Ok(outcome) => Some(outcome),
            Err(e) if matches!(e.kind, ErrorKind::NothingToUndo | ErrorKind::NothingToRedo) => None,
            Err(e) => {
                self.message = Some(e.message);
                None
            }
        }
    }

    /// Recomputes the inference under the cursor and the tool preview. True if either changed.
    fn update_hover(&mut self) -> bool {
        let anchor = self.tool.anchor();
        if anchor.is_none() {
            // Locks last for one operation.
            self.lock = None;
            self.shift_lock = false;
        }
        let radius = PICK_RADIUS_LOGICAL_PX * self.scale_factor;
        let (model, camera) = (self.doc.model(), &self.doc.camera);
        let guides = Guides {
            anchor,
            reference: self.reference,
            lock: self.lock,
        };
        let hit = infer_with(model, camera, self.cursor, &guides, radius);
        // The edge the cursor rests on becomes the reference for later direction snaps.
        if let Some(e) = edge_under(model, camera, self.cursor, radius)
            && let Some(edge) = model.edge(e)
            && let (Some(a), Some(b)) = (model.position(edge.v[0]), model.position(edge.v[1]))
        {
            self.reference = (b - a).try_normalize();
        }
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

    fn click(e: &mut Editor) {
        e.button(Button::Left, true);
        e.button(Button::Left, false);
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

    /// Draws a 2 m square on the ground with the Rectangle tool and selects Push/Pull.
    fn square_then_push_pull(e: &mut Editor) {
        e.key(EditorKey::Char('r'));
        click_at(e, DVec3::new(0.0, 0.0, 0.0));
        click_at(e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('p'));
        assert!(e.status().starts_with("Push/Pull"), "{}", e.status());
    }

    #[test]
    fn push_pull_turns_a_rectangle_into_a_box() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(1.0, 1.0, 1.5));
        assert!(e.status().contains("Distance: 1.5 m"), "{}", e.status());
        click(&mut e);
        assert_eq!(stats(&e), (12, 6));
        e.undo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn typed_distance_finishes_a_push_pull() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        move_to(&mut e, DVec3::new(1.0, 1.0, 0.5));
        type_text(&mut e, "750mm");
        assert_eq!(stats(&e), (12, 6));
        let top = e
            .doc()
            .model()
            .vertices()
            .map(|(_, v)| v.pos.z)
            .fold(0.0, f64::max);
        assert!((top - 0.75).abs() < 1e-9);
    }

    #[test]
    fn failed_tool_commands_show_why() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        type_text(&mut e, "1");
        assert_eq!(stats(&e), (12, 6));
        // Pushing the top 3 m down would turn the 1 m sides inside out.
        click_at(&mut e, DVec3::new(1.0, 1.0, 1.0));
        type_text(&mut e, "-3");
        assert_eq!(stats(&e), (12, 6));
        assert!(e.status().contains("collapse"), "{}", e.status());
    }

    #[test]
    fn failed_clicks_keep_their_message_after_release() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char('p'));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        type_text(&mut e, "1");
        click_at(&mut e, DVec3::new(2.0, 2.0, 1.0));
        click_at(&mut e, DVec3::new(2.0, 2.0, -3.0));
        assert!(e.status().contains("collapse"), "{}", e.status());
    }

    #[test]
    fn api_edit_ends_a_push_pull_in_progress() {
        let mut e = editor();
        square_then_push_pull(&mut e);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert!(e.status().contains("Distance"), "{}", e.status());
        e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [6.0, 5.0, 0.0],
        })
        .unwrap();
        assert!(e.status().contains("Click a face"), "{}", e.status());
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

    #[test]
    fn select_tool_clicks_pick_and_delete_erases_them() {
        let mut e = editor();
        square(&mut e);
        assert!(e.key(EditorKey::Char(' ')));
        assert!(e.status().starts_with("Select"), "{}", e.status());
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert_eq!(e.doc().selection().len(), 1);
        assert!(e.key(EditorKey::Delete));
        assert_eq!(stats(&e), (4, 0));
        assert!(e.doc().selection().is_empty());
        e.undo();
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn backspace_erases_the_selection_only_when_nothing_is_typed() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('1'));
        assert!(e.key(EditorKey::Backspace));
        assert_eq!(
            stats(&e),
            (4, 1),
            "the first backspace deletes the typed digit"
        );
        assert!(e.key(EditorKey::Backspace));
        assert_eq!(stats(&e), (4, 0));
        assert!(!e.key(EditorKey::Delete), "nothing left to erase");
    }

    #[test]
    fn repeated_backspace_and_delete_never_erase_the_selection() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('1'));
        e.key(EditorKey::Char('2'));
        assert!(e.key_repeat(EditorKey::Backspace));
        assert!(e.key_repeat(EditorKey::Backspace));
        assert!(
            !e.key_repeat(EditorKey::Backspace),
            "the text is already empty"
        );
        assert_eq!(stats(&e), (4, 1));
        assert_eq!(e.doc().selection().len(), 1);
        assert!(!e.key_repeat(EditorKey::Delete));
        assert_eq!(stats(&e), (4, 1));
    }

    #[test]
    fn erasing_the_selection_clears_typed_text() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('5'));
        assert!(e.key(EditorKey::Delete));
        assert!(!e.status().contains("5_"), "{}", e.status());
    }

    #[test]
    fn losing_focus_releases_shift() {
        let mut e = editor();
        e.set_modifiers(true, false);
        e.focus_lost();
        assert!(!e.shift);
    }

    #[test]
    fn dragging_a_box_selects_what_is_inside() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        let corners: Vec<DVec2> = [(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)]
            .iter()
            .map(|&(x, y)| e.doc().camera.project(DVec3::new(x, y, 0.0)).unwrap())
            .collect();
        let lo = corners.iter().fold(DVec2::INFINITY, |a, p| a.min(*p)) - 10.0;
        let hi = corners.iter().fold(DVec2::NEG_INFINITY, |a, p| a.max(*p)) + 10.0;
        e.cursor_moved(lo);
        e.button(Button::Left, true);
        e.cursor_moved(hi);
        assert_eq!(e.overlay().len(), 8, "the box is drawn while dragging");
        e.button(Button::Left, false);
        assert_eq!(e.doc().selection().len(), 5, "four edges and the face");
    }

    #[test]
    fn erase_tool_click_erases_an_edge_and_its_face() {
        let mut e = editor();
        square(&mut e);
        assert!(e.key(EditorKey::Char('e')));
        click_at(&mut e, DVec3::new(3.0, 2.0, 0.0));
        assert_eq!(stats(&e), (3, 0));
    }

    #[test]
    fn move_tool_moves_the_selection_by_a_typed_distance() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert!(e.key(EditorKey::Char('m')));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        move_to(&mut e, DVec3::new(2.0, 4.0, 0.0));
        type_text(&mut e, "2");
        assert_eq!(stats(&e), (4, 1));
        let moved = DVec3::new(1.0, 3.0, 0.0);
        let model = e.doc().model();
        assert!(
            model
                .vertices()
                .any(|(_, v)| v.pos.abs_diff_eq(moved, 1e-6))
        );
    }

    #[test]
    fn copies_are_selected_and_arrays_replace_them() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('m'));
        assert!(e.toggle_copy());
        assert!(e.status().contains("Copy"), "{}", e.status());
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        move_to(&mut e, DVec3::new(5.0, 2.0, 0.0));
        type_text(&mut e, "3");
        assert_eq!(stats(&e), (8, 2));
        assert_eq!(
            e.doc().selection().len(),
            5,
            "the copy's four edges and face"
        );
        type_text(&mut e, "3x");
        assert_eq!(stats(&e), (16, 4));
        e.undo();
        assert_eq!(stats(&e), (4, 1), "the array is one undo step");
    }

    /// A square, selected, with the Move tool in copy mode and the base point picked.
    fn start_copy(e: &mut Editor) {
        square(e);
        e.key(EditorKey::Char(' '));
        click_at(e, DVec3::new(2.0, 2.0, 0.0));
        e.key(EditorKey::Char('m'));
        e.toggle_copy();
        click_at(e, DVec3::new(2.0, 2.0, 0.0));
        move_to(e, DVec3::new(5.0, 2.0, 0.0));
    }

    #[test]
    fn a_rejected_array_keeps_the_copy_and_disarms_the_array() {
        let mut e = editor();
        start_copy(&mut e);
        type_text(&mut e, "5000");
        assert_eq!(stats(&e), (8, 2));
        type_text(&mut e, "900x");
        assert_eq!(stats(&e), (8, 2), "the copy is restored");
        assert!(!e.status_parts().message.unwrap_or_default().is_empty());
        // Typing another array must not undo anything.
        type_text(&mut e, "3x");
        assert_eq!(stats(&e), (8, 2));
        e.undo();
        assert_eq!(stats(&e), (4, 1), "the copy is still one undo step");
    }

    #[test]
    fn a_rejected_copy_does_not_arm_an_array() {
        let mut e = editor();
        start_copy(&mut e);
        type_text(&mut e, "2000000");
        assert_eq!(stats(&e), (4, 1), "the copy was rejected");
        assert!(!e.status_parts().message.unwrap_or_default().is_empty());
        type_text(&mut e, "5x");
        assert_eq!(stats(&e), (4, 1), "nothing was undone or copied");
        assert_eq!(e.doc().model().faces().count(), 1);
    }

    #[test]
    fn a_release_without_its_press_is_not_sent_to_the_tool() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        assert!(!e.button(Button::Left, false));
        assert!(e.doc().selection().is_empty());
    }

    /// The edge from the last click's chain: both ends of every edge, as positions.
    fn edge_ends(e: &Editor) -> Vec<[DVec3; 2]> {
        let m = e.doc().model();
        m.edges()
            .map(|(_, ed)| [m.position(ed.v[0]).unwrap(), m.position(ed.v[1]).unwrap()])
            .collect()
    }

    #[test]
    fn shift_locks_the_direction_under_the_cursor() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        let up = e.doc().camera.project(DVec3::new(1.0, 1.0, 2.5)).unwrap();
        e.cursor_moved(up + DVec2::new(4.0, 0.0));
        assert!(e.set_modifiers(true, false));
        assert!(e.status().contains("Locked: blue axis"), "{}", e.status());
        click_at(&mut e, DVec3::new(4.0, 4.0, 0.0));
        let [a, b] = edge_ends(&e)[0];
        let near = |v: f64, w: f64| (v - w).abs() < 1e-9;
        let vertical = near(a.x, b.x) && near(a.y, b.y) && near(a.x, 1.0) && near(a.y, 1.0);
        assert!(vertical && (a.z - b.z).abs() > 0.5, "{a} {b}");
        assert!(e.set_modifiers(false, false), "releasing Shift unlocks");
        assert!(!e.status().contains("Locked"));
    }

    #[test]
    fn arrow_keys_lock_an_axis_during_an_operation() {
        let mut e = editor();
        assert!(!e.key(EditorKey::LockAxis(Axis::X)), "nothing to lock yet");
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert!(e.key(EditorKey::LockAxis(Axis::X)));
        assert!(e.status().contains("Locked: red axis"));
        click_at(&mut e, DVec3::new(3.0, 4.0, 0.0));
        let [a, b] = edge_ends(&e)[0];
        let near = |v: f64, w: f64| (v - w).abs() < 1e-9;
        let along_red = near(a.y, b.y) && near(a.z, 0.0) && near(b.z, 0.0);
        assert!(along_red && (a.x - b.x).abs() > 0.1, "{a} {b}");
        assert!(e.key(EditorKey::LockAxis(Axis::X)), "the same key unlocks");
        assert!(!e.status().contains("Locked"));
        e.key(EditorKey::LockAxis(Axis::Y));
        assert!(e.key(EditorKey::Unlock));
        e.key(EditorKey::Escape);
        assert!(
            !e.key(EditorKey::Unlock),
            "the chain ended, so nothing is locked"
        );
    }

    #[test]
    fn hovering_an_edge_teaches_parallel_snaps() {
        let mut e = editor();
        e.execute(Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [2.0, 1.0, 0.0],
        })
        .unwrap();
        let on_edge = e.doc().camera.project(DVec3::new(1.5, 0.75, 0.0)).unwrap();
        e.cursor_moved(on_edge + DVec2::new(0.0, 2.0));
        click_at(&mut e, DVec3::new(0.0, 3.0, 0.0));
        let along = DVec3::new(0.0, 3.0, 0.0) + DVec3::new(2.0, 1.0, 0.0).normalize() * 2.0;
        e.cursor_moved(e.doc().camera.project(along).unwrap());
        let magenta = tools::overlay::MAGENTA;
        assert!(e.overlay().iter().any(|v| v.color == magenta));
    }

    #[test]
    fn held_arrow_key_does_not_toggle_the_lock() {
        let mut e = editor();
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert!(e.key(EditorKey::LockAxis(Axis::X)));
        for _ in 0..3 {
            assert!(!e.key_repeat(EditorKey::LockAxis(Axis::X)));
        }
        assert!(e.status().contains("Locked: red axis"), "{}", e.status());
        assert!(!e.key_repeat(EditorKey::Unlock));
        assert!(e.status().contains("Locked: red axis"));
    }
}
