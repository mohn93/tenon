//! What every interactive tool implements, and the tools the app can switch between.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{EntityId, Model};
use session::{Camera, Command};

use crate::erase::EraseTool;
use crate::inference::Inference;
use crate::line::LineTool;
use crate::move_tool::MoveTool;
use crate::push_pull::PushPullTool;
use crate::rectangle::RectangleTool;
use crate::scenery::Scenery;
use crate::select::SelectTool;

/// What a tool may look at while handling an event. Points, the camera and entity ids are in
/// the open context's coordinates (see `session::Document::context`).
pub struct ToolContext<'a> {
    /// The open context's mesh: what tools edit.
    pub model: &'a Model,
    /// Everything visible, for inference and picking; `scenery.open()` is `model`.
    pub scenery: &'a Scenery<'a>,
    pub camera: &'a Camera,
    /// Cursor position in physical pixels.
    pub cursor: DVec2,
    /// How close (physical pixels) the cursor must be to an edge to pick it.
    pub pick_radius: f64,
    /// How far (physical pixels) the cursor must travel from a press for it to be a drag.
    pub drag_threshold: f64,
    /// Whether Shift is held.
    pub shift: bool,
    /// The selected edges and faces.
    pub selection: &'a BTreeSet<EntityId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolEvent {
    /// The cursor moved; carries the current inference.
    Move(Inference),
    /// The primary button was pressed at the current inference.
    Click(Inference),
    /// The primary button was released at the current inference.
    Release(Inference),
    /// Text typed into the measurements box, submitted with Enter.
    Value(String),
    /// Escape: abandon the operation in progress.
    Cancel,
    /// The copy modifier was tapped (Ctrl): tools that can copy switch between moving and
    /// copying.
    ToggleCopy,
}

/// What a tool wants done after an event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolOutput {
    /// Commands to execute, in order (each is one undo step).
    pub commands: Vec<Command>,
    /// Segments to draw as a live preview.
    pub preview: Vec<[DVec3; 2]>,
    /// Feedback for the user, e.g. why typed text was ignored.
    pub message: Option<String>,
}

/// An interactive way to build commands. Tools never edit the model themselves.
pub trait Tool {
    fn name(&self) -> &'static str;
    /// What the user should do next.
    fn prompt(&self) -> &'static str;
    /// The measurements box label and the live value, e.g. ("Length", Some("2.5 m")).
    fn measurement(&self) -> (&'static str, Option<String>);
    /// The point axis inference starts from, if any.
    fn anchor(&self) -> Option<DVec3>;
    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput;
    /// The model changed outside the tool (agent edit, undo, redo): drop any operation in
    /// progress, since it may refer to geometry that no longer exists.
    fn on_external_change(&mut self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Select,
    Line,
    Rectangle,
    PushPull,
    Move,
    Erase,
}

impl ToolKind {
    /// Every tool, in toolbar order.
    pub const ALL: [ToolKind; 6] = [
        ToolKind::Select,
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::PushPull,
        ToolKind::Move,
        ToolKind::Erase,
    ];

    pub fn make(self) -> Box<dyn Tool> {
        match self {
            ToolKind::Select => Box::new(SelectTool::new()),
            ToolKind::Line => Box::new(LineTool::new()),
            ToolKind::Rectangle => Box::new(RectangleTool::new()),
            ToolKind::PushPull => Box::new(PushPullTool::new()),
            ToolKind::Move => Box::new(MoveTool::new()),
            ToolKind::Erase => Box::new(EraseTool::new()),
        }
    }

    /// The tool's name, as its `Tool::name` reports it.
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Select => "Select",
            ToolKind::Line => "Line",
            ToolKind::Rectangle => "Rectangle",
            ToolKind::PushPull => "Push/Pull",
            ToolKind::Move => "Move",
            ToolKind::Erase => "Erase",
        }
    }

    /// The key that selects the tool.
    pub fn shortcut(self) -> char {
        match self {
            ToolKind::Select => ' ',
            ToolKind::Line => 'l',
            ToolKind::Rectangle => 'r',
            ToolKind::PushPull => 'p',
            ToolKind::Move => 'm',
            ToolKind::Erase => 'e',
        }
    }

    pub fn from_shortcut(c: char) -> Option<ToolKind> {
        let c = c.to_ascii_lowercase();
        Self::ALL.into_iter().find(|k| k.shortcut() == c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_tool_names_and_shortcuts_round_trip() {
        for kind in ToolKind::ALL {
            assert_eq!(kind.make().name(), kind.label());
            assert_eq!(ToolKind::from_shortcut(kind.shortcut()), Some(kind));
        }
        assert_eq!(ToolKind::from_shortcut('E'), Some(ToolKind::Erase));
        assert_eq!(ToolKind::from_shortcut('z'), None);
    }
}
