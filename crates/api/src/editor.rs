//! The editor around the document, as agents see and set it: the tool and what it is doing,
//! the material the Paint tool paints with, the units imports are read in, fly mode, the
//! status bar and the measurements box.

use kernel::{EntityId, JointId, MaterialId};
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
    RotatedRectangle,
    Circle,
    Polygon,
    Arc,
    Freehand,
    PushPull,
    Move,
    Rotate,
    Scale,
    Offset,
    FollowMe,
    TapeMeasure,
    Protractor,
    Dimension,
    Text,
    Erase,
    Paint,
    SectionPlane,
    Axes,
    LookAround,
    PositionCamera,
}

impl From<ToolKind> for ToolName {
    fn from(kind: ToolKind) -> Self {
        match kind {
            ToolKind::Select => ToolName::Select,
            ToolKind::Line => ToolName::Line,
            ToolKind::Rectangle => ToolName::Rectangle,
            ToolKind::RotatedRectangle => ToolName::RotatedRectangle,
            ToolKind::Freehand => ToolName::Freehand,
            ToolKind::Protractor => ToolName::Protractor,
            ToolKind::Circle => ToolName::Circle,
            ToolKind::Polygon => ToolName::Polygon,
            ToolKind::Arc => ToolName::Arc,
            ToolKind::PushPull => ToolName::PushPull,
            ToolKind::Move => ToolName::Move,
            ToolKind::Rotate => ToolName::Rotate,
            ToolKind::Scale => ToolName::Scale,
            ToolKind::Offset => ToolName::Offset,
            ToolKind::FollowMe => ToolName::FollowMe,
            ToolKind::TapeMeasure => ToolName::TapeMeasure,
            ToolKind::Dimension => ToolName::Dimension,
            ToolKind::Text => ToolName::Text,
            ToolKind::Erase => ToolName::Erase,
            ToolKind::Paint => ToolName::Paint,
            ToolKind::SectionPlane => ToolName::SectionPlane,
            ToolKind::Axes => ToolName::Axes,
            ToolKind::LookAround => ToolName::LookAround,
            ToolKind::PositionCamera => ToolName::PositionCamera,
        }
    }
}

impl From<ToolName> for ToolKind {
    fn from(name: ToolName) -> Self {
        match name {
            ToolName::Select => ToolKind::Select,
            ToolName::Line => ToolKind::Line,
            ToolName::Rectangle => ToolKind::Rectangle,
            ToolName::RotatedRectangle => ToolKind::RotatedRectangle,
            ToolName::Freehand => ToolKind::Freehand,
            ToolName::Protractor => ToolKind::Protractor,
            ToolName::Circle => ToolKind::Circle,
            ToolName::Polygon => ToolKind::Polygon,
            ToolName::Arc => ToolKind::Arc,
            ToolName::PushPull => ToolKind::PushPull,
            ToolName::Move => ToolKind::Move,
            ToolName::Rotate => ToolKind::Rotate,
            ToolName::Scale => ToolKind::Scale,
            ToolName::Offset => ToolKind::Offset,
            ToolName::FollowMe => ToolKind::FollowMe,
            ToolName::TapeMeasure => ToolKind::TapeMeasure,
            ToolName::Dimension => ToolKind::Dimension,
            ToolName::Text => ToolKind::Text,
            ToolName::Erase => ToolKind::Erase,
            ToolName::Paint => ToolKind::Paint,
            ToolName::SectionPlane => ToolKind::SectionPlane,
            ToolName::Axes => ToolKind::Axes,
            ToolName::LookAround => ToolKind::LookAround,
            ToolName::PositionCamera => ToolKind::PositionCamera,
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

/// What the status bar's message is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// Something was refused or failed: a typed value the tool would not take, a command
    /// that failed. One from a typed value flashes by the cursor.
    Error,
    /// Information: a side count, a measured distance, a file saved.
    Info,
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
    /// Whether `message` reports an error or informs; null when there is no message.
    pub message_kind: Option<MessageKind>,
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
    /// Edit > Duplicate (Cmd+D): the Move tool, copying the selection. With the cursor in the
    /// view the base point is clicked there, so the copy follows the cursor until a click or
    /// a typed distance places it (one undo step); otherwise Move waits for a base point.
    /// `invalid_input` with nothing selected.
    Duplicate {},
    /// Edit > Copy (Cmd+C): puts the selection on the app's clipboard as clip text (see
    /// Clipboard), in world coordinates. `invalid_input` with nothing selected. (Named apart
    /// from the `copy` command, which copies by an offset.)
    CopySelection {},
    /// Edit > Cut (Cmd+X): `copy_selection`, then erases the selection as one undo step labelled `cut`.
    /// Nothing is erased when the copy fails. `invalid_input` with nothing selected.
    CutSelection {},
    /// Edit > Paste (Cmd+V): with `at` (a point of the open context), pastes the clip on the
    /// clipboard with its base point there (one `paste` step); without it the Paste tool comes
    /// out, the clip following the cursor until a click places it (Escape or `cancel` goes
    /// back to the tool used before). `invalid_input` when the clipboard holds no clip.
    /// (Named apart from the `paste` command, which takes the clip text.)
    PasteClipboard {
        #[serde(default)]
        at: Option<[f64; 3]>,
    },
    /// Edit > Paste In Place: pastes the clip on the clipboard where it was copied from, in
    /// world coordinates (one `paste` step). `invalid_input` when the clipboard holds no clip.
    PasteInPlace {},
    /// Brings out the Paste tool with the library item at `path` (a `.skr` file; see
    /// `save_to_library`), like `paste_clipboard` without `at`: the item's edges follow the
    /// cursor, its box's minimum corner on the snapped point under it, and the next click
    /// (`tool_input`) inserts it there as one `insert` step, with new definitions each time
    /// (`insert_library_item`). Escape or `cancel` goes back to the tool used before. `path`
    /// follows the read rules (`--api-root`). `invalid_input` for a material item (use
    /// `insert_library_item`), and for a file that is not a valid item, before the tool comes
    /// out.
    PlaceLibraryItem { path: String },
    /// Brings out the Image tool with a PNG or JPEG, as File > Import does for an image file:
    /// give `path` (follows the read rules, `--api-root`) or `data` (the file's bytes in
    /// base64), exactly one. A picture follows the cursor on the face or ground plane under it;
    /// the first click (`tool_input`) sets a corner, moving sizes it keeping the image's
    /// proportions, a typed length sets the width, and the second click (or the typed width)
    /// places it as one `import_image` step, facing the camera (`tool_input` reports it with
    /// `data` left out). Escape or `cancel` goes back to the tool used
    /// before. `invalid_input` or `file` for an image over the limits (see `import_image`),
    /// before the tool comes out.
    PlaceImage {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        data: Option<String>,
    },
    /// File > Export PDF...: renders the current view (camera, display settings and section
    /// cuts; nothing highlighted) with the GPU and writes it as a PDF at `path`, one page per
    /// sheet. Without `scale` the view is fitted on one sheet; with `scale` (1:N, a parallel
    /// view only) the model is printed at that scale, tiled across sheets (at most 100) with
    /// trim marks, a scale bar and a "Sheet r,c of R x C" footer. Paths follow the export
    /// rules (`--api-root`, `overwrite`, never the open model's own file). The reply holds
    /// `pdf: {sheets, warnings}`. `invalid_input` "printing needs a GPU adapter" when there
    /// is none.
    ExportPdf {
        path: String,
        /// "a4" (default), "a3", "letter", "legal" or "tabloid".
        #[serde(default)]
        paper: Option<String>,
        /// Default true.
        #[serde(default)]
        landscape: Option<bool>,
        /// Print at 1:scale; absent fits the view on one sheet.
        #[serde(default)]
        scale: Option<f64>,
        /// Pixels per inch of the images, 72 to 300 (default 150).
        #[serde(default)]
        dpi: Option<u32>,
        /// Add the cut list (see the `cut_list` query; the whole model grouped by definition,
        /// sizes in the display units) as pages after the view's sheets, counted in
        /// `pdf.sheets`. Default false.
        #[serde(default)]
        cut_list: Option<bool>,
        /// Replace an existing file that is not a PDF (default false).
        #[serde(default)]
        overwrite: Option<bool>,
    },
    /// Plays a joint in the window: for `seconds` the joint's value runs linearly from `from`
    /// to `to` (degrees for an angle, meters for a distance), the model drawn at each frame's
    /// value with everything connected to it solved (couplings and cam followers too). Nothing
    /// in the document changes (no undo step, no revision): any input in the window, another
    /// command or the end of the time (without `loop`) stops it, and the window shows the
    /// document as it was. With `loop` it starts again from `from` each time it reaches `to`.
    /// Window only: a program without a window answers `invalid_input`. `from` and `to` must be
    /// within the joint's limits, and both must be reachable (checked before it starts;
    /// `invalid_input` otherwise), as must be `seconds` (above 0, at most 600). A fixed or
    /// ball joint, and one that follows a cam, cannot be played.
    Play {
        joint: JointId,
        from: f64,
        to: f64,
        seconds: f64,
        #[serde(default, rename = "loop")]
        looping: Option<bool>,
    },
}

/// What `export_pdf` reports.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PdfReport {
    /// Pages written.
    pub sheets: usize,
    /// Things the print could not do exactly (a capped image size, labels left out).
    pub warnings: Vec<String>,
}

/// The `type` of each editor command.
pub const EDITOR_COMMANDS: [&str; 16] = [
    "set_tool",
    "set_paint_material",
    "set_import_units",
    "cancel",
    "set_fly_mode",
    "set_dialog",
    "set_soften_angle",
    "duplicate",
    "copy_selection",
    "cut_selection",
    "paste_clipboard",
    "paste_in_place",
    "place_library_item",
    "place_image",
    "export_pdf",
    "play",
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

    /// `execute` sends a `type` listed here to the editor, so none may be a model command's.
    #[test]
    fn no_editor_command_shares_a_model_commands_type() {
        let schema = serde_json::to_value(schemars::schema_for!(session::Command)).unwrap();
        let text = schema.to_string();
        assert!(
            text.contains("\"copy\""),
            "the model's copy is in the schema"
        );
        for name in EDITOR_COMMANDS {
            assert!(!text.contains(&format!("\"{name}\"")), "{name}");
        }
    }
}
