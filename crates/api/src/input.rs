//! `tool_input`: the pointer and keyboard events a user would make in the 3D view, fed to the
//! active tool through the editor the window uses (its inference, locks and typed values).
//! The API checks their shape; the host runs them.

use glam::{DVec2, DVec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use session::{Command, CommandError, ErrorKind, Outcome, check_coordinates};

use crate::dispatch::MAX_IMAGE_SIDE;
use crate::editor::EditorState;
use crate::probe::{AxisName, Snap};

/// The `tool_input` method's params.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolInputParams {
    /// Run in order, each as the window would handle it.
    pub events: Vec<InputEvent>,
    /// The size in pixels of the view the events' pixels are in (both or neither; default
    /// the 3D view's size, `editor.viewport`). Pixels are scaled to the 3D view, which must
    /// have the same proportions.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    /// The tools' button (with `alt` held: orbit).
    #[default]
    Left,
    /// Orbits about the point under the cursor (with `shift`: pans).
    Middle,
    Right,
}

/// One thing a user does. Pointer events take a `pixel` of the view or a model `point` (open
/// context coordinates), looked for where the view shows it at the event's turn; the tools then
/// infer what is near that pixel, as for a click there.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputEvent {
    /// The cursor moves there: the tool's preview and the inference follow.
    Move {
        pixel: Option<[f64; 2]>,
        point: Option<[f64; 3]>,
    },
    /// The button goes down there.
    Down {
        pixel: Option<[f64; 2]>,
        point: Option<[f64; 3]>,
        #[serde(default)]
        button: MouseButton,
    },
    /// The button comes up there.
    Up {
        pixel: Option<[f64; 2]>,
        point: Option<[f64; 3]>,
        #[serde(default)]
        button: MouseButton,
    },
    /// Move, down and up in one place.
    Click {
        pixel: Option<[f64; 2]>,
        point: Option<[f64; 3]>,
        #[serde(default)]
        button: MouseButton,
    },
    /// Two left clicks in one place, then the double-click (the Select tool opens a group).
    DoubleClick {
        pixel: Option<[f64; 2]>,
        point: Option<[f64; 3]>,
    },
    /// Down at the first place, moving through the others, up at the last (at least two):
    /// `pixels` or `points`.
    Drag {
        #[serde(default)]
        pixels: Vec<[f64; 2]>,
        #[serde(default)]
        points: Vec<[f64; 3]>,
        #[serde(default)]
        button: MouseButton,
    },
    /// A key pressed and released with the modifiers given held: one character (an uppercase
    /// letter holds Shift: "G" makes a component), or `enter`, `backspace`, `delete`,
    /// `escape`, `space`, `right` / `left` / `up` (lock the red / green / blue axis), `down`
    /// (unlock), or `ctrl` (Ctrl tapped alone: Move and Rotate switch copying, the Tape Measure
    /// guide making). With `cmd` (or `ctrl`) only `z` (undo; with `shift` redo) is taken.
    Key {
        key: String,
        #[serde(default)]
        shift: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        cmd: bool,
    },
    /// Hold exactly these modifiers until the next `modifiers` event or the end of the call
    /// (then all are let go): Shift locks the direction the cursor snaps along, Cmd or Ctrl
    /// makes a Paint click sample, Shift toggles in the Select tool.
    Modifiers {
        #[serde(default)]
        shift: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        cmd: bool,
    },
    /// Text typed into the measurements box (replacing anything typed), then Enter: lengths,
    /// "2,3", "24s", "5x", "/5", angles.
    Type { text: String },
}

/// Where a pointer event happens: a pixel of the requested view, or a model point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot {
    Pixel(DVec2),
    Point(DVec3),
}

/// A key the editor takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A character, its case already set by Shift.
    Char(char),
    Enter,
    Backspace,
    Delete,
    Escape,
    /// The arrow keys: lock the red, green or blue axis, or unlock.
    Right,
    Left,
    Up,
    Down,
    /// Ctrl pressed and released alone.
    CtrlTap,
}

/// Modifier keys held. Ctrl and Cmd act alike (the window takes either).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub cmd: bool,
}

/// A checked event.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Move(Spot),
    Down(Spot, MouseButton),
    Up(Spot, MouseButton),
    Click(Spot, MouseButton),
    DoubleClick(Spot),
    Drag(Vec<Spot>, MouseButton),
    Key(Key, Mods),
    Modifiers(Mods),
    Type(String),
}

/// Checked `tool_input` params, for the host to run.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolInput {
    pub events: Vec<Input>,
    /// The view the pixels are in; None: the 3D view itself.
    pub view: Option<DVec2>,
}

/// What the cursor snaps to, as the window's marker shows it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Hover {
    pub kind: Snap,
    /// For `axis`: the axis the point is held to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<AxisName>,
    /// In the open context's coordinates.
    pub point: [f64; 3],
}

/// A command the events made the editor run, with its outcome or why it failed.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Issued {
    pub command: Command,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CommandError>,
}

/// The `tool_input` result: the editor after the events.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ToolInputResult {
    /// The tool, its phase and points, status bar and measurements box.
    pub editor: EditorState,
    /// What the cursor snaps to now (null while flying).
    pub inference: Option<Hover>,
    /// The tool's rubber-band geometry: segments in the open context's coordinates.
    pub preview: Vec<[[f64; 3]; 2]>,
    /// Every command the events ran, in order (tools, shortcuts, undo).
    pub commands: Vec<Issued>,
    pub revision: u64,
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

/// `params` checked: each event's shape, points within the coordinate limit, key names.
/// Pixels are checked against the view by the host.
pub fn check(params: ToolInputParams) -> Result<ToolInput, CommandError> {
    let view = match (params.width, params.height) {
        (None, None) => None,
        (Some(w), Some(h)) => {
            let sides = 1..=MAX_IMAGE_SIDE;
            if !sides.contains(&w) || !sides.contains(&h) {
                return Err(invalid(format!(
                    "width and height must be between 1 and {MAX_IMAGE_SIDE}"
                )));
            }
            Some(DVec2::new(w as f64, h as f64))
        }
        _ => return Err(invalid("give both width and height, or neither")),
    };
    let events = params
        .events
        .into_iter()
        .enumerate()
        .map(|(i, e)| event(e).map_err(|e| invalid(format!("event {i}: {}", e.message))))
        .collect::<Result<_, _>>()?;
    Ok(ToolInput { events, view })
}

fn event(e: InputEvent) -> Result<Input, CommandError> {
    Ok(match e {
        InputEvent::Move { pixel, point } => Input::Move(spot(pixel, point)?),
        InputEvent::Down {
            pixel,
            point,
            button,
        } => Input::Down(spot(pixel, point)?, button),
        InputEvent::Up {
            pixel,
            point,
            button,
        } => Input::Up(spot(pixel, point)?, button),
        InputEvent::Click {
            pixel,
            point,
            button,
        } => Input::Click(spot(pixel, point)?, button),
        InputEvent::DoubleClick { pixel, point } => Input::DoubleClick(spot(pixel, point)?),
        InputEvent::Drag {
            pixels,
            points,
            button,
        } => {
            let path: Vec<Spot> = match (pixels.is_empty(), points.is_empty()) {
                (false, true) => pixels
                    .into_iter()
                    .map(|p| spot(Some(p), None))
                    .collect::<Result<_, _>>()?,
                (true, false) => points
                    .into_iter()
                    .map(|p| spot(None, Some(p)))
                    .collect::<Result<_, _>>()?,
                _ => return Err(invalid("give pixels or points")),
            };
            if path.len() < 2 {
                return Err(invalid("a drag needs at least two places"));
            }
            Input::Drag(path, button)
        }
        InputEvent::Key {
            key,
            shift,
            alt,
            ctrl,
            cmd,
        } => {
            let mods = Mods {
                shift,
                alt,
                cmd: cmd || ctrl,
            };
            let (key, mods) = key_named(&key, mods)?;
            if mods.cmd && !matches!(key, Key::Char('z' | 'Z')) {
                return Err(invalid(
                    "with cmd or ctrl only z (undo, with shift redo) is taken; file actions \
                     are commands",
                ));
            }
            Input::Key(key, mods)
        }
        InputEvent::Modifiers {
            shift,
            alt,
            ctrl,
            cmd,
        } => Input::Modifiers(Mods {
            shift,
            alt,
            cmd: cmd || ctrl,
        }),
        InputEvent::Type { text } => {
            if text.is_empty() || text.chars().any(char::is_control) {
                return Err(invalid(
                    "text must be non-empty, without control characters",
                ));
            }
            Input::Type(text)
        }
    })
}

/// A pixel (finite, not negative) or a point within the coordinate limit; exactly one.
fn spot(pixel: Option<[f64; 2]>, point: Option<[f64; 3]>) -> Result<Spot, CommandError> {
    match (pixel, point) {
        (Some(p), None) => {
            let p = DVec2::from(p);
            if p.is_finite() && p.x >= 0.0 && p.y >= 0.0 {
                Ok(Spot::Pixel(p))
            } else {
                Err(invalid("a pixel must be finite and not negative"))
            }
        }
        (None, Some(p)) => {
            check_coordinates(&[p])?;
            Ok(Spot::Point(DVec3::from(p)))
        }
        _ => Err(invalid("give a pixel or a point")),
    }
}

/// The key `name` names, with Shift added for an uppercase letter and the letter's case set
/// by Shift (as the window does, so Caps Lock never matters).
fn key_named(name: &str, mut mods: Mods) -> Result<(Key, Mods), CommandError> {
    let key = match name {
        "enter" => Key::Enter,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "escape" => Key::Escape,
        "space" => Key::Char(' '),
        "right" => Key::Right,
        "left" => Key::Left,
        "up" => Key::Up,
        "down" => Key::Down,
        "ctrl" => Key::CtrlTap,
        _ => {
            let mut chars = name.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(invalid(format!("unknown key {name:?}")));
            };
            if c.is_control() {
                return Err(invalid(format!("unknown key {name:?}")));
            }
            if c.is_ascii_uppercase() {
                mods.shift = true;
            }
            Key::Char(if !c.is_ascii_alphabetic() {
                c
            } else if mods.shift {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            })
        }
    };
    Ok((key, mods))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn checked(v: serde_json::Value) -> Result<ToolInput, CommandError> {
        check(serde_json::from_value(v).unwrap())
    }

    #[test]
    fn events_are_checked_and_keys_named() {
        let t = checked(json!({"events": [
            {"type": "click", "pixel": [10, 20]},
            {"type": "move", "point": [1, 2, 3]},
            {"type": "key", "key": "G"},
            {"type": "key", "key": "z", "shift": true, "ctrl": true},
            {"type": "key", "key": "right"},
            {"type": "drag", "pixels": [[0, 0], [5, 5]], "button": "middle"},
            {"type": "type", "text": "2,3"}
        ], "width": 640, "height": 400}))
        .unwrap();
        assert_eq!(t.view, Some(DVec2::new(640.0, 400.0)));
        assert_eq!(
            t.events[0],
            Input::Click(Spot::Pixel(DVec2::new(10.0, 20.0)), MouseButton::Left)
        );
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert_eq!(t.events[2], Input::Key(Key::Char('G'), shift));
        let redo = Mods {
            shift: true,
            alt: false,
            cmd: true,
        };
        assert_eq!(t.events[3], Input::Key(Key::Char('Z'), redo));
        assert_eq!(t.events[4], Input::Key(Key::Right, Mods::default()));
        assert!(matches!(t.events[5], Input::Drag(ref p, MouseButton::Middle) if p.len() == 2));
    }

    #[test]
    fn bad_events_name_their_place() {
        for (bad, message) in [
            (json!({"type": "click"}), "event 0: give a pixel or a point"),
            (
                json!({"type": "click", "pixel": [1, 1], "point": [0, 0, 0]}),
                "event 0: give a pixel or a point",
            ),
            (
                json!({"type": "move", "pixel": [-1, 0]}),
                "event 0: a pixel must be finite and not negative",
            ),
            (
                json!({"type": "drag", "pixels": [[0, 0]]}),
                "event 0: a drag needs at least two places",
            ),
            (
                json!({"type": "key", "key": "f1"}),
                "event 0: unknown key \"f1\"",
            ),
            (
                json!({"type": "key", "key": "s", "cmd": true}),
                "event 0: with cmd or ctrl only z (undo, with shift redo) is taken; file \
                 actions are commands",
            ),
            (
                json!({"type": "type", "text": ""}),
                "event 0: text must be non-empty, without control characters",
            ),
        ] {
            let e = checked(json!({"events": [bad]})).unwrap_err();
            assert_eq!(
                (e.kind, e.message.as_str()),
                (ErrorKind::InvalidInput, message)
            );
        }
        let e = checked(json!({"events": [], "width": 10})).unwrap_err();
        assert_eq!(e.message, "give both width and height, or neither");
        let e = checked(json!({"events": [{"type": "move", "point": [2e6, 0, 0]}]})).unwrap_err();
        assert!(
            e.message.starts_with("event 0: coordinates"),
            "{}",
            e.message
        );
    }
}
