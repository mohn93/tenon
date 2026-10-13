//! What every interactive tool implements, and the tools the app can switch between.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use kernel::{EntityId, MaterialId, Model};
use session::{Camera, Command, DEFAULT_ARC_SEGMENTS, DEFAULT_CIRCLE_SEGMENTS};

use crate::arc::ArcTool;
use crate::circle::CircleTool;
use crate::erase::EraseTool;
use crate::inference::Inference;
use crate::line::LineTool;
use crate::move_tool::MoveTool;
use crate::paint::PaintTool;
use crate::push_pull::PushPullTool;
use crate::rectangle::RectangleTool;
use crate::rotate::RotateTool;
use crate::scenery::Scenery;
use crate::select::SelectTool;
use crate::tape::TapeMeasureTool;

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
    /// The material the Paint tool paints with (None: the default material).
    pub material: Option<MaterialId>,
    /// Whether the sampling modifier is held (Cmd on macOS, Ctrl elsewhere): the Paint tool
    /// then picks up the material clicked instead of painting.
    pub sample: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolEvent {
    /// The cursor moved; carries the current inference.
    Move(Inference),
    /// The primary button was pressed at the current inference.
    Click(Inference),
    /// The primary button was released at the current inference.
    Release(Inference),
    /// The primary button was double-clicked at the current inference (sent after the second
    /// release).
    DoubleClick(Inference),
    /// Text typed into the measurements box, submitted with Enter.
    Value(String),
    /// Escape: abandon the operation in progress.
    Cancel,
    /// The copy modifier was tapped (Ctrl): tools that can copy switch between moving and
    /// copying; the Tape Measure switches guide creation on and off.
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
    /// A material the tool picked up to paint with next (Some(None): the default material).
    pub sampled: Option<Option<MaterialId>>,
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
    /// Writes the settings this tool keeps for the session (such as a circle's side count)
    /// into `r`, so the tool made next time starts with them.
    fn remember(&self, _r: &mut Remembered) {}
}

/// Tool settings kept for the session, across switching tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remembered {
    /// Sides of the circles the Circle tool draws.
    pub circle_sides: u32,
    /// Segments of the arcs the Arc tool draws.
    pub arc_segments: u32,
}

impl Default for Remembered {
    fn default() -> Self {
        Remembered {
            circle_sides: DEFAULT_CIRCLE_SEGMENTS,
            arc_segments: DEFAULT_ARC_SEGMENTS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    PushPull,
    Move,
    Rotate,
    TapeMeasure,
    Erase,
    Paint,
}

impl ToolKind {
    /// Every tool, in toolbar order.
    pub const ALL: [ToolKind; 11] = [
        ToolKind::Select,
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Arc,
        ToolKind::PushPull,
        ToolKind::Move,
        ToolKind::Rotate,
        ToolKind::TapeMeasure,
        ToolKind::Erase,
        ToolKind::Paint,
    ];

    /// The tool with default settings.
    pub fn make(self) -> Box<dyn Tool> {
        self.make_with(&Remembered::default())
    }

    /// The tool, starting with the settings in `r`.
    pub fn make_with(self, r: &Remembered) -> Box<dyn Tool> {
        match self {
            ToolKind::Select => Box::new(SelectTool::new()),
            ToolKind::Line => Box::new(LineTool::new()),
            ToolKind::Rectangle => Box::new(RectangleTool::new()),
            ToolKind::Circle => Box::new(CircleTool::with_sides(r.circle_sides)),
            ToolKind::Arc => Box::new(ArcTool::with_segments(r.arc_segments)),
            ToolKind::PushPull => Box::new(PushPullTool::new()),
            ToolKind::Move => Box::new(MoveTool::new()),
            ToolKind::Rotate => Box::new(RotateTool::new()),
            ToolKind::TapeMeasure => Box::new(TapeMeasureTool::new()),
            ToolKind::Erase => Box::new(EraseTool::new()),
            ToolKind::Paint => Box::new(PaintTool::new()),
        }
    }

    /// The tool's name, as its `Tool::name` reports it.
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Select => "Select",
            ToolKind::Line => "Line",
            ToolKind::Rectangle => "Rectangle",
            ToolKind::Circle => "Circle",
            ToolKind::Arc => "Arc",
            ToolKind::PushPull => "Push/Pull",
            ToolKind::Move => "Move",
            ToolKind::Rotate => "Rotate",
            ToolKind::TapeMeasure => "Tape Measure",
            ToolKind::Erase => "Erase",
            ToolKind::Paint => "Paint",
        }
    }

    /// The key that selects the tool.
    pub fn shortcut(self) -> char {
        match self {
            ToolKind::Select => ' ',
            ToolKind::Line => 'l',
            ToolKind::Rectangle => 'r',
            ToolKind::Circle => 'c',
            ToolKind::Arc => 'a',
            ToolKind::PushPull => 'p',
            ToolKind::Move => 'm',
            ToolKind::Rotate => 'q',
            ToolKind::TapeMeasure => 't',
            ToolKind::Erase => 'e',
            ToolKind::Paint => 'b',
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
