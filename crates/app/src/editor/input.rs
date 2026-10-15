//! `tool_input`: an agent's pointer and keyboard events run through the same Editor methods
//! the window calls for a user's (see `api::Input`).

use api::{Input, Key, Mods, MouseButton, Spot, ToolInput};
use glam::{DVec2, DVec3};
use session::{Command, CommandError, ErrorKind, Outcome};
use tools::{Axis, Inference};

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

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

impl Editor {
    /// Runs `input`'s events in order as the window runs a user's: each pointer event moves
    /// the cursor first, and the hover is worked out after each, as a frame drawn between them
    /// would. Modifiers held are let go at the end. An event that cannot run (a pixel outside
    /// the view, a point out of sight) stops the rest with `invalid_input`; those before it
    /// stay done.
    pub fn drive(&mut self, input: ToolInput) -> Result<Driven, CommandError> {
        let viewport = self.doc.camera.viewport;
        let view = input.view.unwrap_or(viewport);
        let (asked, ours) = (view.x / view.y, viewport.x / viewport.y);
        if ((asked - ours) / ours).abs() > ASPECT_TOLERANCE {
            return Err(invalid(format!(
                "the view must have the proportions of the 3D view, {} x {} pixels",
                viewport.x, viewport.y
            )));
        }
        self.issued = Some(Vec::new());
        let mut held = Mods::default();
        let mut result = Ok(());
        for (i, event) in input.events.into_iter().enumerate() {
            if let Err(message) = self.input(event, view, &mut held) {
                result = Err(invalid(format!("event {i}: {message}")));
                break;
            }
            self.flush_hover();
        }
        self.hold(Mods::default());
        self.flush_hover();
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
        result.map(|()| driven)
    }

    /// Runs one event; `held` are the modifiers a `modifiers` event holds.
    fn input(&mut self, event: Input, view: DVec2, held: &mut Mods) -> Result<(), String> {
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
                for _ in 0..2 {
                    self.button(Button::Left, true);
                    self.button(Button::Left, false);
                }
                self.double_click();
            }
            Input::Drag(path, button) => {
                let button = button_of(button);
                self.move_to(path[0], view)?;
                self.button(button, true);
                for at in &path[1..] {
                    self.flush_hover();
                    self.move_to(*at, view)?;
                }
                self.button(button, false);
            }
            Input::Key(key, mods) => self.press(key, mods, *held),
            Input::Modifiers(mods) => {
                *held = mods;
                self.hold(mods);
            }
            Input::Type(text) => {
                if !self.flying {
                    self.flush_hover();
                    self.typed = text;
                    self.key(EditorKey::Enter);
                }
            }
        }
        Ok(())
    }

    /// Moves the cursor to `at`: a pixel of `view` scaled to the 3D view, or where the view
    /// shows a model point.
    fn move_to(&mut self, at: Spot, view: DVec2) -> Result<(), String> {
        let viewport = self.doc.camera.viewport;
        let cursor = match at {
            Spot::Pixel(p) => {
                if p.x > view.x || p.y > view.y {
                    return Err(format!(
                        "pixel must lie in the view, 0 to {} across and 0 to {} down",
                        view.x, view.y
                    ));
                }
                p * viewport / view
            }
            Spot::Point(p) => self
                .local_camera()
                .project(p)
                .filter(|c| c.x >= 0.0 && c.y >= 0.0 && c.x <= viewport.x && c.y <= viewport.y)
                .ok_or("the point is not in view")?,
        };
        self.cursor_moved(cursor);
        Ok(())
    }

    /// Holds exactly `mods`, as the window's modifier changes do.
    fn hold(&mut self, mods: Mods) {
        self.set_modifiers(mods.shift, mods.alt);
        self.set_sample_modifier(mods.cmd);
    }

    /// Presses `key` with `mods` held on top of `held`, as the window's key handling does.
    fn press(&mut self, key: Key, mods: Mods, held: Mods) {
        let with = Mods {
            shift: held.shift || mods.shift,
            alt: held.alt || mods.alt,
            cmd: held.cmd || mods.cmd,
        };
        self.hold(with);
        match key {
            Key::CtrlTap => {
                self.toggle_copy();
            }
            Key::Char('z' | 'Z') if with.cmd => {
                if with.shift {
                    self.redo();
                } else {
                    self.undo();
                }
            }
            // Alt is the orbit modifier: Alt with a key does nothing in the window.
            _ if with.alt || with.cmd => {}
            key => {
                self.key(editor_key(key, with.shift));
            }
        }
        self.hold(held);
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
