//! The editor around the document, as agents see and set it: the tool and what it is doing,
//! the material the Paint tool paints with, the units imports are read in, fly mode, the
//! status bar and the measurements box.

use kernel::{EntityId, MaterialId};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// Whether fly mode is on (only the window moves the eye while flying).
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
    /// The modifier keys held: the user's in the window and those `tool_input` holds.
    pub modifiers: crate::input::Mods,
    /// What a click would act on now (Select, Push/Pull, Paint, Erase): an id of the open
    /// context; null otherwise.
    pub hovered: Option<EntityId>,
    /// The snap the window names by the cursor, e.g. "Endpoint" or "Midpoint in Group"; null
    /// when there is none (always in Select).
    pub snap_label: Option<String>,
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
    /// Abort the tool's operation in progress, like Escape pressed until the tool is idle:
    /// leaves fly mode, drops typed text, the direction lock and the operation (and a copy
    /// array or a retyped angle it waits for), and lets go of the modifiers `tool_input`
    /// holds. It never closes the open group (that is `close_instance`).
    Cancel {},
    /// Turn fly mode on or off (F in the window), and set its speed in meters a second (0.25
    /// to 60; absent: unchanged).
    SetFlyMode {
        on: bool,
        #[serde(default)]
        speed: Option<f64>,
    },
    /// Open (`soften_edges`) or close (null or absent) the window shown over the view.
    SetDialog { dialog: Option<Dialog> },
    /// The Soften Edges window's angle, in whole degrees (0 to 180).
    SetSoftenAngle { angle: u32 },
}

/// The `type` of each editor command.
pub const EDITOR_COMMANDS: [&str; 7] = [
    "set_tool",
    "set_paint_material",
    "set_import_units",
    "cancel",
    "set_fly_mode",
    "set_dialog",
    "set_soften_angle",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_editor_command_is_listed() {
        let schema = serde_json::to_value(schemars::schema_for!(EditorCommand)).unwrap();
        let text = schema.to_string();
        let mut listed = 0;
        for variant in schema["oneOf"].as_array().expect("variants") {
            let name = &variant["properties"]["type"];
            let name = name["const"]
                .as_str()
                .or_else(|| name["enum"][0].as_str())
                .unwrap_or_else(|| panic!("no type in {variant}"));
            assert!(EDITOR_COMMANDS.contains(&name), "{name} not listed");
            listed += 1;
        }
        assert_eq!(listed, EDITOR_COMMANDS.len(), "{text}");
    }
}
