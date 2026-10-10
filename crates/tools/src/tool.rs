//! What every interactive tool implements, and the tools the app can switch between.

use glam::{DVec2, DVec3};
use kernel::Model;
use session::{Camera, Command};

use crate::inference::Inference;
use crate::line::LineTool;
use crate::push_pull::PushPullTool;
use crate::rectangle::RectangleTool;

/// What a tool may look at while handling an event.
pub struct ToolContext<'a> {
    pub model: &'a Model,
    pub camera: &'a Camera,
    /// Cursor position in physical pixels.
    pub cursor: DVec2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolEvent {
    /// The cursor moved; carries the current inference.
    Move(Inference),
    /// The primary button was pressed at the current inference.
    Click(Inference),
    /// Text typed into the measurements box, submitted with Enter.
    Value(String),
    /// Escape: abandon the operation in progress.
    Cancel,
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
    Line,
    Rectangle,
    PushPull,
}

impl ToolKind {
    pub fn make(self) -> Box<dyn Tool> {
        match self {
            ToolKind::Line => Box::new(LineTool::new()),
            ToolKind::Rectangle => Box::new(RectangleTool::new()),
            ToolKind::PushPull => Box::new(PushPullTool::new()),
        }
    }

    /// The keyboard shortcut that selects the tool.
    pub fn from_shortcut(c: char) -> Option<ToolKind> {
        match c.to_ascii_lowercase() {
            'l' => Some(ToolKind::Line),
            'r' => Some(ToolKind::Rectangle),
            'p' => Some(ToolKind::PushPull),
            _ => None,
        }
    }
}
