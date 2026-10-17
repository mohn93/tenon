//! `tool_input`: an agent's pointer and keyboard events run through the same Editor methods
//! the window calls for a user's (see `api::Input`).

use api::{Input, Key, Mods, MouseButton, Spot, ToolInput};
use glam::{DVec2, DVec3};
use session::{Command, CommandError, ErrorKind, Outcome};
use tools::{Axis, Inference, ToolEvent};

use super::{Editor, EditorKey};
use crate::navigation::Button;

/// How far the view `tool_input` pixels are in may differ in proportions from the 3D view.
const ASPECT_TOLERANCE: f64 = 0.01;

/// What a run of events left: the commands they ran, the inference under the cursor and the
/// tool's preview (both in the open context's coordinates).
pub struct Driven {
    pub issued: Vec<(Command, Result<Outcome, CommandError>)>,
    pub hover: Option<Inference>,
    pub preview: Vec<[DVec3; 2]>,
}

/// Why `drive` failed.
pub enum DriveError {
    /// Nothing ran.
    Refused(CommandError),
    /// An event could not run at its turn.
    Stopped(Box<Stopped>),
}

/// Event `event` could not run at its turn; the events before it stay done, and `driven` is
/// what they (and any part of the failing one) left.
pub struct Stopped {
    pub event: usize,
    pub error: CommandError,
    pub driven: Driven,
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

/// The places an event happens at.
fn spots(event: &Input) -> &[Spot] {
    match event {
        Input::Move(at)
        | Input::Down(at, _)
        | Input::Up(at, _)
        | Input::Click(at, _)
        | Input::DoubleClick(at)
        | Input::TripleClick(at) => std::slice::from_ref(at),
        Input::Drag(path, _) => path,
        Input::Key(..) | Input::Modifiers(_) | Input::Type(_) => &[],
    }
}

/// Why `p`, a pixel of `view`, cannot be used, if it lies outside it.
fn outside(p: DVec2, view: DVec2) -> Option<String> {
    (p.x > view.x || p.y > view.y).then(|| {
        format!(
            "pixel must lie in the view, 0 to {} across and 0 to {} down",
            view.x, view.y
        )
    })
}

impl Editor {
    /// Runs `input`'s events in order as the window runs a user's: each pointer event moves
    /// the cursor first, and the hover is worked out after each, as a frame drawn between them
    /// would (not while a navigation drag moves the camera, as in the window). Modifiers a
    /// `modifiers` event holds stay held after the call. Pixels outside the view are refused
    /// before anything runs. An event that cannot run at its turn (a point out of sight, a
    /// key or typing under Cmd or Alt) stops the rest; those before it stay done, and a drag
    /// it was making is let go without reaching the tool (a navigation drag stops, a tool's
    /// press is cancelled).
    pub fn drive(&mut self, input: ToolInput) -> Result<Driven, DriveError> {
        let viewport = self.doc.camera.viewport;
        let view = input.view.unwrap_or(viewport);
        let (asked, ours) = (view.x / view.y, viewport.x / viewport.y);
        if ((asked - ours) / ours).abs() > ASPECT_TOLERANCE {
            return Err(DriveError::Refused(invalid(format!(
                "the view must have the proportions of the 3D view, {} x {} pixels",
                viewport.x, viewport.y
            ))));
        }
        for (i, event) in input.events.iter().enumerate() {
            for spot in spots(event) {
                if let Spot::Pixel(p) = spot
                    && let Some(why) = outside(*p, view)
                {
                    return Err(DriveError::Refused(invalid(format!("event {i}: {why}"))));
                }
            }
        }
        // No window lies over an agent's pointer.
        self.covered = false;
        self.issued = Some(Vec::new());
        let mut stopped = None;
        for (i, event) in input.events.into_iter().enumerate() {
            if let Err(message) = self.input(event, view) {
                stopped = Some((i, invalid(format!("event {i}: {message}"))));
                break;
            }
            self.settle_hover();
        }
        self.settle_hover();
        let issued = self.issued.take().unwrap_or_default();
        let driven = Driven {
            issued,
            hover: if self.flying { None } else { self.hit },
            preview: if self.flying {
                Vec::new()
            } else {
                self.preview.clone()
            },
        };
        // The window draws the marker where the events left the cursor at its next frame.
        self.invalidate_hover();
        match stopped {
            None => Ok(driven),
            Some((event, error)) => Err(DriveError::Stopped(Box::new(Stopped {
                event,
                error,
                driven,
            }))),
        }
    }

    /// `n` (2 or 3) left clicks where the cursor is, with the double-click after the second and
    /// the triple-click after the third. As in the window, only clicks that reached the tool
    /// count.
    fn clicks(&mut self, n: u8) {
        for i in 1..=n {
            self.button(Button::Left, true);
            // The return value of `button` is a redraw flag; the press reached the tool if the
            // editor now waits for its release.
            let reached = self.tool_pressed;
            self.button(Button::Left, false);
            match i {
                2 if reached => {
                    self.double_click();
                }
                3 if reached => {
                    self.triple_click();
                }
                _ => {}
            }
        }
    }

    /// Works the hover out as a frame would: not while a navigation drag moves the camera.
    fn settle_hover(&mut self) {
        if !self.nav.dragging() {
            self.flush_hover();
        }
    }

    /// Runs one event.
    fn input(&mut self, event: Input, view: DVec2) -> Result<(), String> {
        match event {
            Input::Move(at) => {
                self.move_to(at, view)?;
            }
            Input::Down(at, button) => {
                self.move_to(at, view)?;
                self.button(button_of(button), true);
            }
            Input::Up(at, button) => {
                self.move_to(at, view)?;
                self.button(button_of(button), false);
            }
            Input::Click(at, button) => {
                self.move_to(at, view)?;
                self.button(button_of(button), true);
                self.button(button_of(button), false);
            }
            Input::DoubleClick(at) => {
                self.move_to(at, view)?;
                self.clicks(2);
            }
            Input::TripleClick(at) => {
                self.move_to(at, view)?;
                self.clicks(3);
            }
            Input::Drag(path, button) => {
                let button = button_of(button);
                self.move_to(path[0], view)?;
                let (navigating, pressed) = (self.nav.dragging(), self.tool_pressed);
                self.button(button, true);
                // What this drag's own press started: a navigation drag, or a press the tool saw.
                let started = (
                    !navigating && self.nav.dragging(),
                    !pressed && self.tool_pressed,
                );
                for at in &path[1..] {
                    self.settle_hover();
                    if let Err(e) = self.move_to(*at, view) {
                        self.let_go(button, started);
                        return Err(e);
                    }
                }
                self.button(button, false);
            }
            Input::Key(key, mods) => self.press(key, mods)?,
            Input::Modifiers(mods) => {
                self.agent_mods = mods;
                self.apply_modifiers();
            }
            Input::Type(text) => {
                let held = self.modifiers();
                if held.cmd || held.alt {
                    return Err(
                        "typing is not taken while cmd, ctrl or alt is held; let go of them \
                         with a modifiers event"
                            .into(),
                    );
                }
                if !self.flying {
                    self.settle_hover();
                    self.typed = text;
                    self.key(EditorKey::Enter);
                }
            }
        }
        Ok(())
    }

    /// Lets go of `button`, pressed by a drag that cannot go on, without its release reaching
    /// the tool: a navigation drag stops where it got to, and a press the tool saw is
    /// cancelled, as losing focus does.
    /// `started` says whether its press started a navigation drag and a press the tool saw;
    /// a press made before the drag stays.
    fn let_go(&mut self, button: Button, started: (bool, bool)) {
        if started.0 && self.nav.dragging() {
            self.button(button, false);
        }
        if started.1 && std::mem::take(&mut self.tool_pressed) {
            self.send(ToolEvent::Cancel);
        }
        self.invalidate_hover();
    }

    /// Moves the cursor to `at`: a pixel of `view` scaled to the 3D view, or where the view
    /// shows a model point.
    fn move_to(&mut self, at: Spot, view: DVec2) -> Result<(), String> {
        let viewport = self.doc.camera.viewport;
        match at {
            Spot::Pixel(p) => {
                if let Some(why) = outside(p, view) {
                    return Err(why);
                }
                self.cursor_moved(p * viewport / view);
            }
            Spot::Point(p) => {
                let cursor = self
                    .local_camera()
                    .project(p)
                    .filter(|c| c.x >= 0.0 && c.y >= 0.0 && c.x <= viewport.x && c.y <= viewport.y)
                    .ok_or("the point is not in view")?;
                self.cursor_moved(cursor);
                // The tools look for the point itself there, whatever hides it.
                self.cursor_point = Some(p);
                self.invalidate_hover();
            }
        }
        Ok(())
    }

    /// Presses `key` with `mods` held on top of those held, as the window's key handling does.
    fn press(&mut self, key: Key, mods: Mods) -> Result<(), String> {
        let held = self.modifiers();
        let (cmd, alt) = (held.cmd || mods.cmd, held.alt || mods.alt);
        let is_z = matches!(key, Key::Char('z' | 'Z'));
        if key != Key::CtrlTap && !(is_z && cmd) {
            if cmd {
                return Err("with cmd or ctrl held only z (undo, with shift redo) is taken".into());
            }
            if alt {
                return Err("with alt held no key is taken".into());
            }
        }
        self.key_mods = mods;
        self.apply_modifiers();
        let shift = self.modifiers().shift;
        match key {
            Key::CtrlTap => {
                self.toggle_copy();
            }
            Key::Char('z' | 'Z') if cmd => {
                if shift {
                    self.redo();
                } else {
                    self.undo();
                }
            }
            key => {
                self.key(editor_key(key, shift));
            }
        }
        self.key_mods = Mods::default();
        self.apply_modifiers();
        Ok(())
    }
}

fn button_of(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        MouseButton::Right => Button::Other,
    }
}

/// The editor key for `key`; a letter is uppercase exactly when Shift is held.
fn editor_key(key: Key, shift: bool) -> EditorKey {
    match key {
        Key::Char(c) if c.is_ascii_alphabetic() && shift => EditorKey::Char(c.to_ascii_uppercase()),
        Key::Char(c) if c.is_ascii_alphabetic() => EditorKey::Char(c.to_ascii_lowercase()),
        Key::Char(c) => EditorKey::Char(c),
        Key::Enter => EditorKey::Enter,
        Key::Backspace => EditorKey::Backspace,
        Key::Delete => EditorKey::Delete,
        Key::Escape => EditorKey::Escape,
        Key::Right => EditorKey::LockAxis(Axis::X),
        Key::Left => EditorKey::LockAxis(Axis::Y),
        Key::Up => EditorKey::LockAxis(Axis::Z),
        Key::Down => EditorKey::Unlock,
        Key::CtrlTap => unreachable!("handled by press"),
    }
}
