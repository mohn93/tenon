//! The editor around the document, as agents see and set it: the tool and what it is doing,
//! the material the Paint tool paints with, the units imports are read in, fly mode, the
//! status bar and the measurements box.

use kernel::MaterialId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use session::ImportUnits;
use tools::ToolKind;

/// A tool, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
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

impl From<ToolKind> for ToolName {
    fn from(kind: ToolKind) -> Self {
        match kind {
            ToolKind::Select => ToolName::Select,
            ToolKind::Line => ToolName::Line,
            ToolKind::Rectangle => ToolName::Rectangle,
            ToolKind::Circle => ToolName::Circle,
            ToolKind::Arc => ToolName::Arc,
            ToolKind::PushPull => ToolName::PushPull,
            ToolKind::Move => ToolName::Move,
            ToolKind::Rotate => ToolName::Rotate,
            ToolKind::TapeMeasure => ToolName::TapeMeasure,
            ToolKind::Erase => ToolName::Erase,
            ToolKind::Paint => ToolName::Paint,
        }
    }
}

impl From<ToolName> for ToolKind {
    fn from(name: ToolName) -> Self {
        match name {
            ToolName::Select => ToolKind::Select,
            ToolName::Line => ToolKind::Line,
            ToolName::Rectangle => ToolKind::Rectangle,
            ToolName::Circle => ToolKind::Circle,
            ToolName::Arc => ToolKind::Arc,
            ToolName::PushPull => ToolKind::PushPull,
            ToolName::Move => ToolKind::Move,
            ToolName::Rotate => ToolKind::Rotate,
            ToolName::TapeMeasure => ToolKind::TapeMeasure,
            ToolName::Erase => ToolKind::Erase,
            ToolName::Paint => ToolKind::Paint,
        }
    }
}

/// Whether the tool is in the middle of an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhaseName {
    /// The next click starts an operation.
    Idle,
    /// An operation is in progress.
    Active,
}

/// A window the editor shows over the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Dialog {
    /// Edit > Soften Edges...
    SoftenEdges,
}

/// What the editor is doing: the `editor` query's result and `editor` notification.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct EditorState {
    /// The active tool.
    pub tool: ToolName,
    pub phase: PhaseName,
    /// The points the operation in progress was given so far (clicks), in the open
    /// context's coordinates; empty when idle (and for the Select tool, whose clicks and
    /// boxes are on screen).
    pub points: Vec<[f64; 3]>,
    /// The material the Paint tool paints with; null for the default material.
    pub paint_material: Option<MaterialId>,
    /// What one unit of an imported OBJ or STL file is (File > Import Units).
    pub import_units: ImportUnits,
    /// Whether fly mode is on (the window only).
    pub flying: bool,
    /// Fly mode's speed, meters a second.
    pub fly_speed: f64,
    /// The 3D view's size in pixels: the view `pick` and `infer` use by default.
    pub viewport: [f64; 2],
    /// The status bar as the window shows it: tool, prompt, lock, measurement and message.
    pub status: String,
    /// What to do next.
    pub prompt: String,
    /// Feedback from the last tool action or failed command.
    pub message: Option<String>,
    /// The direction lock in force, e.g. "Locked: red axis".
    pub lock: Option<String>,
    /// The measurements box's label, e.g. "Length"; empty when the tool measures nothing.
    pub measurement_label: String,
    /// The measurements box's text: what is typed while typing, else the live measurement.
    pub measurement: Option<String>,
    /// Whether text is being typed into the measurements box.
    pub typing: bool,
    /// The Soften Edges window's angle, in degrees.
    pub soften_angle: u32,
    /// The window open over the view, if any.
    pub dialog: Option<Dialog>,
}

/// Commands that set the editor rather than the model: no undo step, no revision. `execute`
/// takes them (not `batch`).
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EditorCommand {
    /// Switch to a tool (a fresh one, even if it is the active tool), dropping the operation in
    /// progress and anything typed.
    SetTool { tool: ToolName },
    /// The material the Paint tool paints with (null or absent: the default material). The
    /// tool does not change.
    SetPaintMaterial {
        #[serde(default)]
        material: Option<MaterialId>,
    },
    /// What one unit of an imported file is, for the window's File > Import.
    SetImportUnits { units: ImportUnits },
    /// Abort the tool's operation in progress, like Escape: drops typed text, the direction
    /// lock and the operation (and a copy array or a retyped angle it waits for). It never
    /// closes the open group (that is `close_instance`).
    Cancel,
}

/// The `type` of each editor command.
pub const EDITOR_COMMANDS: [&str; 4] = [
    "set_tool",
    "set_paint_material",
    "set_import_units",
    "cancel",
];
