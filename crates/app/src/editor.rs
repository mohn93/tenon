//! Window-independent editing state: routes input to navigation or the active tool, runs the
//! resulting commands on the document, and produces the overlay and status text to show.

use std::path::Path;

use glam::{DVec2, DVec3};
use kernel::EntityId;
use render::grid::LineVertex;
use session::{
    BatchError, Camera, Command, CommandError, Document, ErrorKind, Outcome, SelectMode,
};
use tools::overlay::overlay;
use tools::{
    Axis, Guides, Inference, Lock, Scenery, SnapKind, Tool, ToolContext, ToolEvent, ToolKind,
    ToolOutput, edge_under_in, infer_in,
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

    /// The copy modifier (Ctrl) was tapped. The Move and Rotate tools switch copying, the Tape
    /// Measure switches guide creation; elsewhere it does nothing and returns false.
    pub fn toggle_copy(&mut self) -> bool {
        if !copies(self.kind) && self.kind != ToolKind::TapeMeasure {
            return false;
        }
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

    /// The primary button was double-clicked (call it after the second release). The Select
    /// tool opens the group under the cursor, or closes the open one when there is nothing of
    /// it there.
    pub fn double_click(&mut self) -> bool {
        self.update_hover();
        let Some(hit) = self.hit else { return false };
        self.send(ToolEvent::DoubleClick(hit));
        true
    }

    pub fn scroll(&mut self, steps: f64) -> bool {
        let zoomed = self.nav.scroll(&mut self.doc.camera, steps);
        if zoomed {
            self.update_hover();
        }
        zoomed
    }

    /// Typing goes to the measurements box once it starts with a digit, ".", "-", "/" or "*"
    /// (or "x" in the Move and Rotate tools, for arrays like "x5"); "g" makes a group and
    /// "G" (Shift+G) a component of the selection; other letters are tool shortcuts. Enter
    /// submits the typed text to the tool. Escape clears the typed text, or else cancels the
    /// tool's operation (the Select tool then closes the open group). Delete, and Backspace
    /// with nothing typed, erase the selection. Returns true if anything changed.
    pub fn key(&mut self, key: EditorKey) -> bool {
        match key {
            EditorKey::Char(c) if self.typed.is_empty() => {
                let starts_value = c.is_ascii_digit() || matches!(c, '.' | '-' | '/' | '*');
                let starts_array = matches!(c, 'x' | 'X') && copies(self.kind);
                if starts_value || starts_array {
                    self.typed.push(c);
                    true
                } else if c == 'g' {
                    self.make_group()
                } else if c == 'G' {
                    self.make_component()
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

    /// Makes a group of the selection. With nothing selected it says so in the status line
    /// (and returns true, so the window redraws).
    pub fn make_group(&mut self) -> bool {
        let ids: Vec<EntityId> = self.doc.selection().iter().copied().collect();
        if ids.is_empty() {
            self.message = Some("Select geometry to make a group".into());
            return true;
        }
        self.run_selection_command(Command::MakeGroup { ids })
    }

    /// Makes a component of the selection, named "Component N" with the first free N. With
    /// nothing selected it says so in the status line (and returns true, so the window redraws).
    pub fn make_component(&mut self) -> bool {
        let ids: Vec<EntityId> = self.doc.selection().iter().copied().collect();
        if ids.is_empty() {
            self.message = Some("Select geometry to make a component".into());
            return true;
        }
        let names: Vec<&str> = self
            .doc
            .scene()
            .definitions()
            .map(|(_, d)| d.name.as_str())
            .collect();
        let name = (1..)
            .map(|n| format!("Component {n}"))
            .find(|n| !names.contains(&n.as_str()))
            .expect("a free name");
        self.run_selection_command(Command::MakeComponent { ids, name })
    }

    /// Explodes the selected groups and components. With none selected it says so in the
    /// status line (and returns true, so the window redraws).
    pub fn explode_selection(&mut self) -> bool {
        let ids: Vec<EntityId> = self
            .doc
            .selection()
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Instance(_)))
            .collect();
        if ids.is_empty() {
            self.message = Some("Select a group or component to explode".into());
            return true;
        }
        self.run_selection_command(Command::Explode { ids })
    }

    /// Erases every guide of the open context as one undo step. False if there are none.
    pub fn delete_guides(&mut self) -> bool {
        if self.doc.model().guides().next().is_none() {
            return false;
        }
        self.run_selection_command(Command::DeleteGuides)
    }

    /// Closes the open group or component. False if none is open.
    pub fn close_instance(&mut self) -> bool {
        if self.doc.context().is_empty() {
            return false;
        }
        self.run_selection_command(Command::CloseInstance)
    }

    /// Runs a command that works on the selection or the open context, dropping the tool's
    /// operation and any typed text first.
    fn run_selection_command(&mut self, command: Command) -> bool {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        self.run(command);
        self.update_hover();
        true
    }

    /// Starts an empty model, dropping unsaved changes (the window asks about them first).
    /// Always true (the view changes).
    pub fn new_model(&mut self) -> bool {
        self.run_selection_command(Command::New { discard: true })
    }

    /// Opens a `.skr` file, dropping unsaved changes (the window asks about them first). False
    /// if it cannot be opened; the status then says why.
    pub fn open_file(&mut self, path: &Path) -> bool {
        let revision = self.doc.revision();
        self.run_selection_command(Command::Open {
            path: path.display().to_string(),
            discard: true,
        });
        self.doc.revision() != revision
    }

    /// Saves to `path`, or to the file last opened or saved. False if it cannot be saved; the
    /// status then says why.
    pub fn save_file(&mut self, path: Option<&Path>) -> bool {
        self.message = None;
        let saved = self
            .run(Command::Save {
                path: path.map(|p| p.display().to_string()),
            })
            .is_some();
        if saved {
            self.message = Some(format!(
                "Saved {}",
                self.doc
                    .path()
                    .map_or_else(String::new, |p| p.display().to_string())
            ));
        }
        saved
    }

    /// Exports every face of the model to `path`, in the format its extension names. False if
    /// it cannot be exported; the status then says why.
    pub fn export_file(&mut self, path: &Path) -> bool {
        self.message = None;
        let exported = self
            .run(Command::Export {
                path: path.display().to_string(),
                format: None,
            })
            .is_some();
        if exported {
            self.message = Some(format!("Exported {}", path.display()));
        }
        exported
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

    /// Line pairs for the renderer's overlay pass. Tools work in the open context's
    /// coordinates; the overlay is drawn in the root's.
    pub fn overlay(&self) -> Vec<LineVertex> {
        let t = self.doc.context_transform();
        let preview: Vec<[DVec3; 2]> = self
            .preview
            .iter()
            .map(|[a, b]| [t.transform_point3(*a), t.transform_point3(*b)])
            .collect();
        let hit = self.hit.map(|h| Inference {
            point: t.transform_point3(h.point),
            ..h
        });
        overlay(&preview, hit.as_ref(), &self.doc.camera, self.scale_factor)
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

    /// The camera in the open context's coordinates. Placements turn and move, so inside a
    /// turned instance its up direction is no longer the context's Z.
    fn local_camera(&self) -> Camera {
        self.doc
            .camera
            .placed(self.doc.context_transform().inverse())
    }

    /// Everything tools can see, in the open context's coordinates.
    fn scenery(&self) -> Scenery<'_> {
        Scenery::of(self.doc.scene(), self.doc.context()).expect("the open context exists")
    }

    fn handle(&mut self, event: ToolEvent) -> ToolOutput {
        let camera = self.local_camera();
        let scenery =
            Scenery::of(self.doc.scene(), self.doc.context()).expect("the open context exists");
        let ctx = ToolContext {
            model: self.doc.model(),
            scenery: &scenery,
            camera: &camera,
            cursor: self.cursor,
            pick_radius: PICK_RADIUS_LOGICAL_PX * self.scale_factor,
            drag_threshold: DRAG_LOGICAL_PX * self.scale_factor,
            shift: self.shift,
            selection: self.doc.selection(),
        };
        self.tool.handle(event, &ctx)
    }

    /// Runs the tool's commands in order; a copy selects what it created, as in SketchUp.
    /// The first command that fails stops the rest, and so does a copy or rotation that changes
    /// nothing (a copy landing on existing geometry, a rotation of what lies on its axis): it
    /// leaves no undo step for an array or a retyped angle to replace. If it
    /// fails after an undo from the same list (an array replacing a copy), the undo is redone
    /// and the copy reselected; either way the tool drops its state.
    fn apply(&mut self, out: ToolOutput) {
        let mut undone = false;
        let selected: Vec<EntityId> = self.doc.selection().iter().copied().collect();
        for command in out.commands {
            let is_undo = matches!(command, Command::Undo);
            let is_copy = matches!(
                command,
                Command::Copy { .. }
                    | Command::Rotate {
                        copies: Some(_),
                        ..
                    }
            );
            let is_turn = matches!(command, Command::Rotate { copies: None, .. });
            let revision = self.doc.revision();
            let outcome = self.run(command);
            let unchanged =
                (is_copy || is_turn) && outcome.is_some() && self.doc.revision() == revision;
            if unchanged {
                self.message = Some(if is_copy {
                    "Nothing new to copy".into()
                } else {
                    "Nothing turned: it lies on the axis".into()
                });
            }
            let Some(outcome) = outcome.filter(|_| !unchanged) else {
                if undone && let Ok(redone) = self.doc.execute(Command::Redo) {
                    if is_copy && !redone.changes.created.is_empty() {
                        self.select_created(redone);
                    } else {
                        // Only a refused copy reselects its copies; anything else (a plain
                        // move or turn, a guide) keeps what was selected.
                        self.run(Command::Select {
                            ids: selected,
                            mode: SelectMode::Replace,
                        });
                    }
                }
                self.reset_tool();
                return;
            };
            undone |= is_undo;
            if is_copy {
                self.select_created(outcome);
            }
        }
        self.preview = out.preview;
    }

    /// Selects the edges, faces, instances and guides a command created in the open context (a
    /// copied group's new contents live in its new definition and are left out).
    fn select_created(&mut self, outcome: Outcome) {
        let m = self.doc.model();
        let ids = outcome
            .changes
            .created
            .into_iter()
            .filter(|id| match *id {
                EntityId::Edge(e) => m.edge(e).is_some(),
                EntityId::Face(f) => m.face(f).is_some(),
                EntityId::Instance(i) => m.instance(i).is_some(),
                EntityId::Guide(g) => m.guide(g).is_some(),
                EntityId::Vertex(_) => false,
            })
            .collect();
        self.run(Command::Select {
            ids,
            mode: SelectMode::Replace,
        });
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
        let camera = self.local_camera();
        let scenery = self.scenery();
        let guides = Guides {
            anchor,
            reference: self.reference,
            lock: self.lock,
        };
        let hit = infer_in(&scenery, &camera, self.cursor, &guides, radius);
        // The edge the cursor rests on (anywhere) becomes the reference for direction snaps.
        let reference = edge_under_in(&scenery, &camera, self.cursor, radius)
            .and_then(|(_, _, a, b)| (b - a).try_normalize());
        if reference.is_some() {
            self.reference = reference;
        }
        let out = self.handle(ToolEvent::Move(hit));
        let changed = self.hit != Some(hit) || self.preview != out.preview;
        self.hit = Some(hit);
        self.preview = out.preview;
        changed
    }
}

/// True for the tools Ctrl switches to copying.
fn copies(kind: ToolKind) -> bool {
    matches!(kind, ToolKind::Move | ToolKind::Rotate)
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
            up: None,
            projection: None,
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
    fn lines_snap_to_guides_and_select_picks_and_erases_them() {
        let mut e = editor();
        e.execute(Command::AddGuidePoint {
            position: [1.0, 1.0, 0.0],
        })
        .unwrap();
        e.execute(Command::AddGuideLine {
            point: [0.0, 3.0, 0.0],
            direction: [1.0, 0.0, 0.0],
        })
        .unwrap();
        let point = DVec3::new(1.0, 1.0, 0.0);
        e.cursor_moved(e.doc().camera.project(point).unwrap() + DVec2::new(2.0, 1.0));
        assert_eq!(
            e.hit.map(|h| (h.point, h.kind)),
            Some((point, SnapKind::GuidePoint))
        );
        // A line drawn from the guide point to the guide line ends on it.
        click(&mut e);
        click_at(&mut e, DVec3::new(1.0, 3.0, 0.0));
        e.key(EditorKey::Escape);
        let ends = edge_ends(&e);
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0][0], point);
        assert!(
            ends[0][1].abs_diff_eq(DVec3::new(1.0, 3.0, 0.0), 1e-9),
            "the line ends on the guide line: {ends:?}"
        );

        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(-2.0, 3.0, 0.0));
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            vec!["g1".parse().unwrap()]
        );
        assert!(e.key(EditorKey::Delete));
        assert_eq!(e.doc().model().guides().count(), 1);
    }

    #[test]
    fn t_measures_from_an_edge_to_a_parallel_guide_and_ctrl_measures_only() {
        let mut e = editor();
        square(&mut e);
        assert!(e.key(EditorKey::Char('t')));
        assert_eq!(e.tool_kind(), ToolKind::TapeMeasure);
        // From the bottom side of the square, 1.5 m out (the guide follows the cursor's side).
        click_at(&mut e, DVec3::new(2.5, 1.0, 0.0));
        move_to(&mut e, DVec3::new(2.5, -0.5, 0.0));
        assert!(e.status().contains("Length: 1.5 m"), "{}", e.status());
        type_text(&mut e, "2");
        let guides: Vec<kernel::Guide> = e.doc().model().guides().map(|(_, g)| *g).collect();
        let [kernel::Guide::Line { point, direction }] = guides[..] else {
            panic!("{guides:?}");
        };
        assert!((point.y + 1.0).abs() < 1e-9, "{point:?}");
        assert_eq!(direction, DVec3::X);
        // Retyping moves that guide: one undo step replaced by another.
        type_text(&mut e, "3");
        let lines: Vec<f64> = e
            .doc()
            .model()
            .guides()
            .map(|(_, g)| g.anchor().y)
            .collect();
        assert_eq!(lines.len(), 1);
        assert!((lines[0] + 2.0).abs() < 1e-9, "{lines:?}");

        assert!(e.toggle_copy());
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        click_at(&mut e, DVec3::new(3.0, 3.0, 0.0));
        assert!(e.status().contains("Distance: 2.828 m"), "{}", e.status());
        assert_eq!(
            e.doc().model().guides().count(),
            1,
            "no guide when measuring"
        );

        assert!(e.delete_guides());
        assert_eq!(e.doc().model().guides().count(), 0);
        assert!(!e.delete_guides());
        e.undo();
        assert_eq!(e.doc().model().guides().count(), 1);
    }

    #[test]
    fn a_refused_retyped_guide_distance_keeps_the_guide_and_the_selection() {
        let mut e = editor();
        square(&mut e);
        let face = e
            .doc()
            .model()
            .faces()
            .next()
            .map(|(id, _)| EntityId::Face(id))
            .unwrap();
        e.run(Command::Select {
            ids: vec![face],
            mode: SelectMode::Replace,
        });
        assert!(e.key(EditorKey::Char('t')));
        click_at(&mut e, DVec3::new(2.5, 1.0, 0.0));
        move_to(&mut e, DVec3::new(2.5, -0.5, 0.0));
        type_text(&mut e, "2");
        let guide = |e: &Editor| {
            e.doc()
                .model()
                .guides()
                .map(|(_, g)| *g)
                .collect::<Vec<_>>()
        };
        let before = guide(&e);
        assert_eq!(before.len(), 1);
        let selected: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        type_text(&mut e, "2000000");
        assert!(e.status().contains("within +/-1000000 m"), "{}", e.status());
        assert_eq!(guide(&e), before);
        let after: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        assert_eq!(after, selected);
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
        type_text(&mut e, "50000");
        assert_eq!(stats(&e), (8, 2));
        // 30 copies 50 km apart pass the tool's count limit but leave the coordinate limit.
        type_text(&mut e, "30x");
        assert_eq!(stats(&e), (8, 2), "the copy is restored");
        assert!(!e.status_parts().message.unwrap_or_default().is_empty());
        assert_eq!(
            e.doc().selection().len(),
            5,
            "the restored copy is selected"
        );
        let model = e.doc().model();
        assert!(e.doc().selection().iter().all(|id| {
            match *id {
                EntityId::Edge(edge) => model
                    .edge(edge)
                    .unwrap()
                    .v
                    .iter()
                    .all(|v| model.position(*v).unwrap().x > 3.5),
                EntityId::Face(f) => model.face(f).unwrap().outer.len() == 4,
                EntityId::Vertex(_) | EntityId::Instance(_) | EntityId::Guide(_) => false,
            }
        }));
        // Typing another array must not undo anything.
        type_text(&mut e, "3x");
        assert_eq!(stats(&e), (8, 2));
        e.undo();
        assert_eq!(stats(&e), (4, 1), "the copy is still one undo step");
    }

    #[test]
    fn a_copy_that_changes_nothing_does_not_arm_an_array() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Char(' '));
        click_at(&mut e, DVec3::new(2.0, 1.0, 0.0));
        let selected = e.doc().selection().clone();
        assert_eq!(selected.len(), 1, "the bottom edge");
        e.key(EditorKey::Char('m'));
        e.toggle_copy();
        // Copy the bottom edge onto the top edge, which is already there.
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        click_at(&mut e, DVec3::new(1.0, 3.0, 0.0));
        assert_eq!(stats(&e), (4, 1));
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Nothing new to copy")
        );
        type_text(&mut e, "3x");
        assert_eq!(stats(&e), (4, 1), "the last edge drawn was not undone");
        assert_eq!(e.doc().selection(), &selected);
        e.undo();
        assert_eq!(stats(&e), (3, 0), "the square's last edge is the last step");
    }

    #[test]
    fn typed_arrays_can_start_with_a_symbol_or_x() {
        for (distance, array) in [("6", "/3"), ("2", "*3"), ("2", "x3")] {
            let mut e = editor();
            start_copy(&mut e);
            type_text(&mut e, distance);
            assert_eq!(stats(&e).1, 2, "{array}");
            type_text(&mut e, array);
            assert_eq!(stats(&e), (13, 4), "{array}: four squares in a row");
        }
    }

    #[test]
    fn x_starts_typing_only_in_the_move_tool() {
        let mut e = editor();
        assert!(!e.key(EditorKey::Char('x')));
        assert!(!e.status().contains("x_"), "{}", e.status());
        e.key(EditorKey::Char('m'));
        assert!(e.key(EditorKey::Char('x')));
        assert!(e.status().contains("x_"), "{}", e.status());
    }

    #[test]
    fn ctrl_outside_the_move_tool_keeps_the_message() {
        let mut e = editor();
        type_text(&mut e, "2");
        let message = e.status_parts().message;
        assert!(message.is_some(), "the line tool needs a start point");
        assert!(!e.toggle_copy());
        assert_eq!(e.status_parts().message, message);
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

    /// A 2 x 2 x 1 box at the origin, grouped and moved by (10, 0, 0). Returns the instance.
    fn moved_group(e: &mut Editor) -> EntityId {
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        e.execute(Command::PushPull {
            face: "f8".parse().unwrap(),
            distance: 1.0,
        })
        .unwrap();
        let m = e.doc().model();
        let ids = m
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
            .collect();
        e.execute(Command::MakeGroup { ids }).unwrap();
        let group = *e.doc().selection().first().unwrap();
        e.execute(Command::Move {
            ids: vec![group],
            offset: [10.0, 0.0, 0.0],
        })
        .unwrap();
        group
    }

    /// The box of `moved_group`, turned a quarter turn about X (its top, z = 1, now faces -Y
    /// at y = -1) and then moved by (10, 0, 0). Returns the instance.
    fn turned_group(e: &mut Editor) -> EntityId {
        let group = moved_group(e);
        e.execute(Command::Rotate {
            ids: vec![group],
            center: [10.0, 0.0, 0.0],
            axis: Some([1.0, 0.0, 0.0]),
            angle: 90.0,
            copies: None,
        })
        .unwrap();
        group
    }

    /// True if the open context has an edge between these two points.
    fn has_edge(e: &Editor, a: DVec3, b: DVec3) -> bool {
        let m = e.doc().model();
        m.edges().any(|(_, edge)| {
            let (p, q) = (
                m.position(edge.v[0]).unwrap(),
                m.position(edge.v[1]).unwrap(),
            );
            (p.abs_diff_eq(a, 1e-9) && q.abs_diff_eq(b, 1e-9))
                || (p.abs_diff_eq(b, 1e-9) && q.abs_diff_eq(a, 1e-9))
        })
    }

    #[test]
    fn tools_inside_a_moved_group_work_in_its_coordinates() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(10.0, 0.0, 1.0));
        click_at(&mut e, DVec3::new(12.0, 2.0, 1.0));
        e.key(EditorKey::Escape);
        assert!(
            has_edge(&e, DVec3::new(0.0, 0.0, 1.0), DVec3::new(2.0, 2.0, 1.0)),
            "the diagonal is drawn in the group's own coordinates"
        );
        assert_eq!(stats(&e), (13, 7), "it splits the group's top");
    }

    #[test]
    fn tools_inside_a_turned_group_work_in_its_coordinates() {
        let mut e = editor();
        let group = turned_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Line);
        // The group's top corners (0, 0, 1) and (2, 2, 1), where they are shown.
        click_at(&mut e, DVec3::new(10.0, -1.0, 0.0));
        click_at(&mut e, DVec3::new(12.0, -1.0, 2.0));
        e.key(EditorKey::Escape);
        assert!(
            has_edge(&e, DVec3::new(0.0, 0.0, 1.0), DVec3::new(2.0, 2.0, 1.0)),
            "the diagonal is drawn in the group's own coordinates"
        );
        assert_eq!(stats(&e), (13, 7), "it splits the group's top");
    }

    #[test]
    fn a_turned_group_is_picked_and_boxed_where_it_is_shown() {
        let mut e = editor();
        let group = turned_group(&mut e);
        clear_selection(&mut e);
        e.select_tool(ToolKind::Select);
        click_at(&mut e, DVec3::new(11.0, -1.0, 1.0));
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            [group]
        );
        assert_eq!(box_min(&e, group), DVec3::new(10.0, -1.0, 0.0));
        assert_eq!(box_max(&e, group), DVec3::new(12.0, 0.0, 2.0));
    }

    #[test]
    fn the_marker_follows_the_cursor_inside_a_moved_group() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Line);
        let corner = DVec3::new(10.0, 0.0, 1.0);
        let px = e.doc().camera.project(corner).unwrap();
        e.cursor_moved(px + DVec2::new(3.0, 2.0));
        let overlay = e.overlay();
        assert!(!overlay.is_empty());
        assert!(
            overlay
                .iter()
                .all(|v| DVec3::from(v.pos.map(f64::from)).distance(corner) < 0.5),
            "the endpoint marker is drawn at the corner in root coordinates"
        );
    }

    #[test]
    fn lines_from_the_root_snap_to_a_groups_corner() {
        let mut e = editor();
        moved_group(&mut e);
        e.select_tool(ToolKind::Line);
        let corner = DVec3::new(10.0, 0.0, 0.0);
        let px = e.doc().camera.project(corner).unwrap();
        e.cursor_moved(px + DVec2::new(3.0, -2.0));
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        click_at(&mut e, DVec3::new(10.0, -3.0, 0.0));
        e.key(EditorKey::Escape);
        assert!(has_edge(&e, corner, DVec3::new(10.0, -3.0, 0.0)));
    }

    fn clear_selection(e: &mut Editor) {
        e.execute(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
        })
        .unwrap();
    }

    fn double_click_at(e: &mut Editor, p: DVec3) {
        click_at(e, p);
        click_at(e, p);
        e.double_click();
    }

    /// The minimum corner of an instance's bounding box in the open context.
    fn box_min(e: &Editor, id: EntityId) -> DVec3 {
        match e.doc().query(&Query::Entity { id }).unwrap() {
            QueryResult::Instance(info) => DVec3::from(info.bounds.unwrap().min),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The maximum corner of an instance's bounding box in the open context.
    fn box_max(e: &Editor, id: EntityId) -> DVec3 {
        match e.doc().query(&Query::Entity { id }).unwrap() {
            QueryResult::Instance(info) => DVec3::from(info.bounds.unwrap().max),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn clicking_a_group_selects_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        clear_selection(&mut e);
        e.select_tool(ToolKind::Select);
        click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            [group]
        );
    }

    #[test]
    fn double_clicking_a_group_opens_it_and_escape_closes_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.select_tool(ToolKind::Select);
        double_click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
        let open: Vec<EntityId> = e
            .doc()
            .context()
            .iter()
            .map(|i| EntityId::Instance(*i))
            .collect();
        assert_eq!(open, [group]);
        e.key(EditorKey::Escape);
        assert!(e.doc().context().is_empty());
    }

    #[test]
    fn double_clicking_outside_the_open_group_closes_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Select);
        double_click_at(&mut e, DVec3::new(-3.0, -3.0, 0.0));
        assert!(e.doc().context().is_empty());
    }

    #[test]
    fn a_window_around_a_group_selects_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        clear_selection(&mut e);
        e.select_tool(ToolKind::Select);
        let corners: Vec<DVec2> = [0.0, 2.0]
            .into_iter()
            .flat_map(|x| [0.0, 2.0].map(move |y| (x, y)))
            .flat_map(|(x, y)| [0.0, 1.0].map(move |z| DVec3::new(10.0 + x, y, z)))
            .map(|p| e.doc().camera.project(p).unwrap())
            .collect();
        let lo = corners.iter().fold(corners[0], |a, b| a.min(*b)) - DVec2::splat(20.0);
        let hi = corners.iter().fold(corners[0], |a, b| a.max(*b)) + DVec2::splat(20.0);
        e.cursor_moved(lo);
        e.button(Button::Left, true);
        e.cursor_moved(hi);
        e.button(Button::Left, false);
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            [group]
        );
    }

    #[test]
    fn the_move_tool_moves_a_selected_group() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.select_tool(ToolKind::Move);
        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
        let px = e.doc().camera.project(DVec3::new(10.0, 3.0, 0.0)).unwrap();
        e.cursor_moved(px);
        for c in "5".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert!(
            box_min(&e, group).abs_diff_eq(DVec3::new(10.0, 5.0, 0.0), 1e-6),
            "{:?}",
            box_min(&e, group)
        );
    }

    #[test]
    fn q_turns_a_selected_group_by_a_typed_angle() {
        let mut e = editor();
        let group = moved_group(&mut e);
        assert!(e.key(EditorKey::Char('q')));
        assert!(e.status().starts_with("Rotate"), "{}", e.status());
        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
        type_text(&mut e, "90");
        assert_eq!(box_min(&e, group), DVec3::new(8.0, 0.0, 0.0));
        assert_eq!(box_max(&e, group), DVec3::new(10.0, 2.0, 1.0));
        type_text(&mut e, "-90");
        assert_eq!(box_min(&e, group), DVec3::new(10.0, -2.0, 0.0));
        e.execute(Command::Undo).unwrap();
        assert_eq!(
            box_min(&e, group),
            DVec3::new(10.0, 0.0, 0.0),
            "one undo step"
        );
    }

    #[test]
    fn a_rotation_that_turns_nothing_is_not_replaced_by_a_retyped_angle() {
        let mut e = editor();
        e.execute(Command::AddEdge {
            start: [5.0, 5.0, 0.0],
            end: [5.0, 5.0, 1.0],
        })
        .unwrap();
        let edge = e.doc().model().edges().next().unwrap().0;
        e.execute(Command::Select {
            ids: vec![EntityId::Edge(edge)],
            mode: SelectMode::Replace,
        })
        .unwrap();
        // An unrelated later step, which a retyped angle's undo would take back.
        e.execute(Command::AddEdge {
            start: [-5.0, 0.0, 0.0],
            end: [-4.0, 0.0, 0.0],
        })
        .unwrap();
        e.select_tool(ToolKind::Rotate);
        click_at(&mut e, DVec3::new(5.0, 5.0, 0.0));
        type_text(&mut e, "90");
        assert!(e.status().contains("Nothing turned"), "{}", e.status());
        type_text(&mut e, "45");
        assert_eq!(stats(&e), (2, 0), "the later edge was not undone");
    }

    #[test]
    fn a_refused_retyped_angle_keeps_the_selection() {
        let mut e = editor();
        square(&mut e);
        let face = EntityId::Face(e.doc().model().faces().next().unwrap().0);
        e.execute(Command::Select {
            ids: vec![face],
            mode: SelectMode::Replace,
        })
        .unwrap();
        e.select_tool(ToolKind::Rotate);
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        type_text(&mut e, "30");
        type_text(&mut e, "360");
        assert!(
            e.status().contains("whole number of turns"),
            "{}",
            e.status()
        );
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            vec![face],
            "the selection survives the refused angle"
        );
    }

    /// Every edge and face in the open context.
    fn context_contents(e: &Editor) -> Vec<EntityId> {
        let m = e.doc().model();
        m.edges()
            .map(|(id, _)| EntityId::Edge(id))
            .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
            .collect()
    }

    #[test]
    fn tools_inside_a_nested_turned_group_work_in_the_inner_groups_coordinates() {
        let mut e = editor();
        let outer = turned_group(&mut e);
        e.execute(Command::OpenInstance { id: outer }).unwrap();
        let ids = context_contents(&e);
        e.execute(Command::MakeGroup { ids }).unwrap();
        let inner = *e.doc().selection().first().unwrap();
        // The inner group's (x, y) go to (2 - y, x) in the outer one, and so to (12 - y, -z, x)
        // in the world.
        e.execute(Command::Rotate {
            ids: vec![inner],
            center: [1.0, 1.0, 0.0],
            axis: Some([0.0, 0.0, 1.0]),
            angle: 90.0,
            copies: None,
        })
        .unwrap();
        e.execute(Command::OpenInstance { id: inner }).unwrap();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(12.0, -1.0, 0.0));
        click_at(&mut e, DVec3::new(10.0, -1.0, 2.0));
        e.key(EditorKey::Escape);
        assert!(
            has_edge(&e, DVec3::new(0.0, 0.0, 1.0), DVec3::new(2.0, 2.0, 1.0)),
            "the diagonal is drawn in the inner group's own coordinates"
        );
        assert_eq!(stats(&e), (13, 7), "it splits the inner group's top");
    }

    #[test]
    fn rotating_inside_a_turned_group_uses_the_context_coordinates_and_the_faces_normal() {
        let mut e = editor();
        let group = turned_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        let ids = context_contents(&e);
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
        })
        .unwrap();
        e.select_tool(ToolKind::Rotate);
        // The middle of the group's top (1, 1, 1), where it is shown; its normal is the
        // group's own Z, which points along -Y in the world.
        click_at(&mut e, DVec3::new(11.0, -1.0, 1.0));
        type_text(&mut e, "30");
        let (s, c) = 30f64.to_radians().sin_cos();
        let turn = |x: f64, y: f64| {
            DVec3::new(
                1.0 + (x - 1.0) * c - (y - 1.0) * s,
                1.0 + (x - 1.0) * s + (y - 1.0) * c,
                1.0,
            )
        };
        assert!(
            has_edge(&e, turn(0.0, 0.0), turn(2.0, 0.0)),
            "the box turned about the vertical line through (1, 1) in its own coordinates"
        );
        assert_eq!(stats(&e), (12, 6));
    }

    #[test]
    fn rotated_copies_select_the_copies_and_arrays_replace_them() {
        let mut e = editor();
        moved_group(&mut e);
        e.select_tool(ToolKind::Rotate);
        assert!(e.toggle_copy());
        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
        type_text(&mut e, "90");
        let instances = |e: &Editor| e.doc().model().instances().count();
        assert_eq!(instances(&e), 2);
        let copy = *e.doc().selection().first().unwrap();
        assert_eq!(box_min(&e, copy), DVec3::new(8.0, 0.0, 0.0));
        type_text(&mut e, "x3");
        assert_eq!(instances(&e), 4, "a full turn of copies");
        assert_eq!(e.doc().selection().len(), 3);
    }

    #[test]
    fn copying_a_group_selects_the_copy() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.select_tool(ToolKind::Move);
        e.toggle_copy();
        click_at(&mut e, DVec3::new(10.0, 0.0, 0.0));
        let px = e.doc().camera.project(DVec3::new(10.0, 3.0, 0.0)).unwrap();
        e.cursor_moved(px);
        for c in "4".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        let selected: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        assert_eq!(selected.len(), 1);
        assert_ne!(selected[0], group);
        assert!(matches!(selected[0], EntityId::Instance(_)));
        assert_eq!(e.status_parts().message, None);
        assert!(box_min(&e, selected[0]).abs_diff_eq(DVec3::new(10.0, 4.0, 0.0), 1e-6));
    }

    /// Names of the definitions in the model, in id order.
    fn definition_names(e: &Editor) -> Vec<String> {
        e.doc()
            .scene()
            .definitions()
            .map(|(_, d)| d.name.clone())
            .collect()
    }

    #[test]
    fn g_and_shift_g_group_the_selection_and_explode_takes_it_apart() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('g')), "the message needs a redraw");
        assert!(e.status().contains("Select geometry to make a group"));
        square(&mut e);
        e.select_tool(ToolKind::Select);
        click_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert!(e.key(EditorKey::Char('g')));
        let group = *e.doc().selection().first().unwrap();
        assert!(matches!(group, EntityId::Instance(_)));
        assert_eq!(definition_names(&e), [""]);
        assert!(e.key(EditorKey::Char('G')));
        assert!(e.key(EditorKey::Char('G')));
        assert_eq!(
            definition_names(&e),
            ["", "Component 1", "Component 2"],
            "each component gets the next free name"
        );
        assert!(e.explode_selection());
        assert!(e.explode_selection());
        assert!(e.explode_selection());
        assert_eq!(
            definition_names(&e).len(),
            3,
            "the components stay, and so does the group inside Component 1"
        );
        assert_eq!(stats(&e), (4, 1), "the square is back in the root");
        assert!(e.explode_selection(), "the message needs a redraw");
        assert!(
            e.status()
                .contains("Select a group or component to explode")
        );
        assert_eq!(
            stats(&e),
            (4, 1),
            "the square's edges and face are no group"
        );
    }

    #[test]
    fn close_instance_goes_back_out() {
        let mut e = editor();
        assert!(!e.close_instance());
        let group = moved_group(&mut e);
        e.execute(Command::OpenInstance { id: group }).unwrap();
        assert!(e.close_instance());
        assert!(e.doc().context().is_empty());
    }

    /// Clicks along the 2-long edge at y = 0, z = 1 of the moved group's box, from x = 10 to 12.
    fn click_along_top_front_edge(e: &mut Editor) -> Vec<Vec<EntityId>> {
        (1..20)
            .map(|k| {
                clear_selection(e);
                click_at(e, DVec3::new(10.0 + 2.0 * k as f64 / 20.0, 0.0, 1.0));
                e.doc().selection().iter().copied().collect()
            })
            .collect()
    }

    #[test]
    fn an_outside_edge_on_the_open_edge_never_steals_the_pick() {
        let mut e = editor();
        let group = moved_group(&mut e);
        e.execute(Command::AddEdge {
            start: [9.0, 0.0, 1.0],
            end: [13.0, 0.0, 1.0],
        })
        .unwrap();
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Select);
        let picks = click_along_top_front_edge(&mut e);
        assert!(
            picks
                .iter()
                .all(|p| matches!(p.as_slice(), [EntityId::Edge(_)])),
            "{picks:?}"
        );
    }

    #[test]
    fn a_root_edge_on_a_groups_edge_wins_the_pick_at_the_root() {
        let mut e = editor();
        moved_group(&mut e);
        e.execute(Command::AddEdge {
            start: [9.0, 0.0, 1.0],
            end: [13.0, 0.0, 1.0],
        })
        .unwrap();
        e.select_tool(ToolKind::Select);
        let picks = click_along_top_front_edge(&mut e);
        assert!(
            picks
                .iter()
                .all(|p| matches!(p.as_slice(), [EntityId::Edge(_)])),
            "{picks:?}"
        );
    }

    #[test]
    fn push_pull_from_the_root_ignores_a_groups_face() {
        let mut e = editor();
        let group = moved_group(&mut e);
        let before = box_min(&e, group);
        let top = box_max(&e, group);
        e.select_tool(ToolKind::PushPull);
        click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
        let px = e.doc().camera.project(DVec3::new(11.0, 1.0, 2.0)).unwrap();
        e.cursor_moved(px);
        for c in "2".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert_eq!(box_min(&e, group), before);
        assert_eq!(box_max(&e, group), top);
        assert_eq!(stats(&e), (0, 0), "the root gained nothing");
    }

    #[test]
    fn crossing_a_group_with_a_box_dragged_leftwards_selects_it() {
        let mut e = editor();
        let group = moved_group(&mut e);
        clear_selection(&mut e);
        e.select_tool(ToolKind::Select);
        let centre = e.doc().camera.project(DVec3::new(11.0, 1.0, 0.5)).unwrap();
        e.cursor_moved(centre + DVec2::new(30.0, 30.0));
        e.button(Button::Left, true);
        e.cursor_moved(centre);
        e.button(Button::Left, false);
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            [group]
        );
    }

    #[test]
    fn the_erase_tool_erases_a_group_and_undo_restores_it() {
        let mut e = editor();
        moved_group(&mut e);
        assert_eq!(definition_names(&e).len(), 1);
        e.select_tool(ToolKind::Erase);
        click_at(&mut e, DVec3::new(11.0, 1.0, 1.0));
        assert!(e.doc().model().instances().next().is_none());
        assert_eq!(definition_names(&e).len(), 0, "its definition went too");
        assert!(e.undo());
        assert!(e.doc().model().instances().next().is_some());
        assert_eq!(definition_names(&e).len(), 1);
    }

    /// A path for a test file, unique to this process.
    fn temp_file(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("sketchup-alt-editor-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn saving_opening_and_starting_over() {
        let mut e = editor();
        square(&mut e);
        e.key(EditorKey::Escape);
        assert!(!e.save_file(None), "a model never saved needs a path");
        assert!(e.status().contains("give a path"), "{}", e.status());
        let path = temp_file("square.skr");
        assert!(e.save_file(Some(&path)));
        assert!(e.status().contains("Saved"), "{}", e.status());
        assert!(!e.doc().is_modified());

        assert!(e.new_model());
        assert_eq!(stats(&e), (0, 0));
        assert!(e.open_file(&path));
        assert_eq!(stats(&e), (4, 1));
        assert_eq!(e.doc().path(), Some(path.as_path()));

        let missing = temp_file("missing.skr");
        assert!(!e.open_file(&missing));
        assert!(e.status().contains("missing.skr"), "{}", e.status());
        assert_eq!(stats(&e), (4, 1), "a failed open leaves the model");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn exporting_writes_the_file_its_extension_names() {
        let mut e = editor();
        let path = temp_file("square.stl");
        assert!(
            !e.export_file(&path),
            "an empty model has nothing to export"
        );
        assert!(e.status().contains("no faces"), "{}", e.status());
        square(&mut e);
        e.key(EditorKey::Escape);
        assert!(e.export_file(&path));
        assert!(e.status().contains("Exported"), "{}", e.status());
        assert_eq!(std::fs::read(&path).unwrap().len(), 84 + 50 * 2);
        let unknown = temp_file("square.dwg");
        assert!(!e.export_file(&unknown));
        assert!(e.status().contains(".obj"), "{}", e.status());
        let _ = std::fs::remove_file(&path);
    }

    /// Vertices of the open context, as positions.
    fn positions(e: &Editor) -> Vec<DVec3> {
        e.doc().model().vertices().map(|(_, v)| v.pos).collect()
    }

    #[test]
    fn the_circle_tool_draws_a_circle_around_the_first_click() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('c')));
        assert_eq!(e.tool_kind(), ToolKind::Circle);
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        click_at(&mut e, DVec3::new(3.0, 1.0, 0.0));
        assert_eq!(stats(&e), (24, 1));
        let center = DVec3::new(1.0, 1.0, 0.0);
        for p in positions(&e) {
            assert!((p.distance(center) - 2.0).abs() < 1e-6, "{p}");
        }
    }

    #[test]
    fn a_typed_side_count_and_radius_shape_the_circle() {
        let mut e = editor();
        e.select_tool(ToolKind::Circle);
        for c in "8s".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert!(e.status().contains("8 sides"), "{}", e.status());
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        let px = e.doc().camera.project(DVec3::new(1.0, 0.0, 0.0)).unwrap();
        e.cursor_moved(px);
        for c in "1.5".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert_eq!(stats(&e), (8, 1));
        for p in positions(&e) {
            assert!((p.length() - 1.5).abs() < 1e-9, "{p}");
        }
        // The count stays for the next circle.
        click_at(&mut e, DVec3::new(5.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(6.0, 0.0, 0.0));
        assert_eq!(stats(&e), (16, 2));
    }

    #[test]
    fn the_arc_tool_draws_an_arc_from_three_clicks() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('a')));
        assert_eq!(e.tool_kind(), ToolKind::Arc);
        for c in "4s".chars() {
            e.key(EditorKey::Char(c));
        }
        e.key(EditorKey::Enter);
        assert!(e.status().contains("4 segments"), "{}", e.status());
        click_at(&mut e, DVec3::new(0.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(2.0, 0.0, 0.0));
        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
        assert_eq!(stats(&e), (4, 0));
        for p in positions(&e) {
            assert!(
                (p.distance(DVec3::new(1.0, 0.0, 0.0)) - 1.0).abs() < 1e-6,
                "{p}"
            );
        }
        let edges: Vec<_> = e.doc().model().edges().map(|(id, _)| id).collect();
        let curve = e.doc().model().curve_of(edges[0]).unwrap();
        assert_eq!(curve.edges.len(), 4);
    }

    #[test]
    fn a_circle_started_on_a_wall_lies_in_the_wall() {
        let mut e = editor();
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [4.0, 0.0, 3.0],
            normal: None,
        })
        .unwrap();
        e.select_tool(ToolKind::Circle);
        click_at(&mut e, DVec3::new(2.0, 0.0, 1.5));
        click_at(&mut e, DVec3::new(2.5, 0.0, 1.5));
        let circle: Vec<DVec3> = positions(&e)
            .into_iter()
            .filter(|p| p.distance(DVec3::new(2.0, 0.0, 1.5)) < 0.6)
            .collect();
        assert_eq!(circle.len(), 24);
        assert!(
            circle.iter().all(|p| p.y.abs() < 1e-9),
            "in the wall's plane"
        );
    }
}
