//! The window's panels: toolbar, status bar with the measurements box, and on the right Entity
//! Info, the Materials list and the Outliner.

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{Panel, Ui, Vec2};
use kernel::{EntityId, TexturePlacement};
use session::{Lighting, MaterialId, OutlinerTable, Projection, StandardView};
use tools::ToolKind;

use crate::editor::{ImportUnits, Status};

mod dialogs;
mod fields;
mod materials;
mod outliner;
mod right;
mod status;
#[cfg(test)]
mod tests;
mod toolbar;

/// What the user asked for through the panels this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiAction {
    Tool(ToolKind),
    Undo,
    Redo,
    EraseSelection,
    MakeGroup,
    MakeComponent,
    Explode,
    CloseInstance,
    NewModel,
    OpenFile,
    SaveFile,
    SaveFileAs,
    ImportFile,
    /// What one unit in an imported OBJ or STL file is.
    SetImportUnits(ImportUnits),
    ExportFile,
    DeleteGuides,
    ZoomExtents,
    StandardView(StandardView),
    SetProjection(Projection),
    /// Light the view by day or by night.
    SetLighting(Lighting),
    /// Paint with this material (None: the default material), switching to the Paint tool.
    ChooseMaterial(Option<MaterialId>),
    AddMaterial,
    DeleteMaterial(MaterialId),
    RenameMaterial(MaterialId, String),
    RecolorMaterial(MaterialId, [u8; 3]),
    /// Set a material's opacity, in percent (0 to 100).
    SetMaterialOpacity(MaterialId, u8),
    /// Set how strongly a material glows, in tenths (0 to 100: emission 0 to 10).
    SetMaterialEmission(MaterialId, u8),
    /// Choose an image file for a material's texture.
    LoadTexture(MaterialId),
    /// Take a material's texture off.
    RemoveTexture(MaterialId),
    /// Set the width and height a material's texture covers, in millimeters.
    SetTextureSize(MaterialId, [u32; 2]),
    /// Select the outliner node at `path` (instances from the root down to it); `toggle`
    /// (Shift, Cmd or Ctrl held) adds or removes it instead of replacing the selection.
    OutlinerSelect {
        path: Vec<EntityId>,
        toggle: bool,
    },
    /// Open the instances along `path` for editing (empty: back to the root).
    OpenPath(Vec<EntityId>),
    /// Turn fly mode on or off.
    ToggleFly,
    /// Open or close the Agent Activity window.
    ShowAgentActivity(bool),
    /// Open or close the Soften Edges window.
    ShowSoften(bool),
    /// Set the Soften Edges window's angle, in degrees (0 to 180).
    SetSoftenAngle(u8),
    /// Soften the selection (or the open context) up to the window's angle.
    Soften,
    /// Harden the selection (or the open context).
    Harden,
    /// Make the selected edges soft or hard (Entity Info's Soft checkbox).
    SetSoft(bool),
    /// Set the texture offset of the faces (those selected when the field was edited), in
    /// millimeters along u and v.
    SetTextureOffset(Vec<EntityId>, [i32; 2]),
    /// Set the texture rotation of the faces, in tenths of a degree.
    SetTextureRotation(Vec<EntityId>, i32),
    /// Set the texture scale of the faces, in thousandths.
    SetTextureScale(Vec<EntityId>, [u32; 2]),
    /// Turn the texture of each selected face a further quarter counter-clockwise.
    RotateTexture90,
    /// Clear the texture placement of the selected faces.
    ResetTexturePosition,
}

/// A material as the Materials list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialRow {
    pub id: MaterialId,
    pub name: String,
    pub color: [u8; 3],
    /// From 0 (invisible) to 1 (opaque).
    pub opacity: f32,
    /// From 0 (no glow) to 10.
    pub emission: f32,
    pub texture: Option<TextureRow>,
}

/// A material's texture as the Materials list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureRow {
    /// A small copy of the image for its swatch.
    pub swatch: egui::TextureId,
    /// The size one copy covers on the model, in meters.
    pub width: f64,
    pub height: f64,
    /// The image's size in pixels.
    pub pixels: [u32; 2],
}

/// The right panel's width, in points.
const RIGHT_PANEL_WIDTH: f32 = 180.0;

/// What the window shows of the agents using the API.
pub struct Agents<'a> {
    /// Connections open now.
    pub connected: usize,
    /// The activity log's lines, newest first, while the Agent Activity window is open.
    pub log: Option<&'a [String]>,
    /// Recovery files written this session.
    pub recovered: &'a [PathBuf],
}

/// What the panels show.
pub struct View<'a> {
    pub agents: Agents<'a>,
    pub tool: ToolKind,
    pub status: &'a Status,
    pub info: &'a [String],
    pub has_selection: bool,
    /// Whether a group or component is selected.
    pub has_instance_selected: bool,
    /// Whether a group or component is open for editing.
    pub inside_instance: bool,
    /// Whether the open context has guides to delete.
    pub has_guides: bool,
    /// The camera's projection, checked in the Camera menu.
    pub projection: Projection,
    /// How the view is lit; Night lighting is checked in the Camera menu at night.
    pub lighting: Lighting,
    /// Whether fly mode is on, checked in the Camera menu.
    pub flying: bool,
    /// The unit imported files are read in, checked in File > Import Units.
    pub import_units: ImportUnits,
    /// Every material, in id order.
    pub materials: &'a [MaterialRow],
    /// The material the Paint tool paints with (None: the default material).
    pub material: Option<MaterialId>,
    /// The placed groups and components (`Document::outliner_rows`).
    pub outliner: &'a OutlinerTable,
    /// The open context's instances, from the root down.
    pub context: &'a [EntityId],
    /// The selection, in the open context.
    pub selection: &'a BTreeSet<EntityId>,
    /// The Soften Edges window's angle in degrees while it is open.
    pub soften: Option<u8>,
    /// Whether the selected edges are all soft, when only edges are selected.
    pub edges_soft: Option<bool>,
    /// The first selected face's texture placement, when only faces are selected.
    pub texture_position: Option<TexturePosition>,
}

/// What Entity Info's texture position fields show and apply to.
#[derive(Clone, Debug, PartialEq)]
pub struct TexturePosition {
    /// The first selected face's placement.
    pub placement: TexturePlacement,
    /// The selected faces.
    pub faces: Vec<EntityId>,
}

/// The fly mode controls, shown over the Camera menu's Fly mode item.
pub const FLY_HELP: &str = "Walk or fly through the model.\n\
    Mouse: look around (trackpad: two-finger scroll)\n\
    W A S D: forward, left, back, right\n\
    E or Space: up; Q or Ctrl: down\n\
    Wheel, [ ] or - =: slower, faster (trackpad: Cmd+scroll)\n\
    Shift: 3x faster; Option/Alt: slower\n\
    Esc or F: leave fly mode";

/// The least height the right panel lays out its contents in; a shorter panel scrolls them.
const PANEL_MIN_HEIGHT: f32 = 480.0;

/// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
/// the 3D view, in points.
pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
    let mut actions = Vec::new();
    Panel::top("toolbar").show(ui, |ui| toolbar::toolbar(ui, view, &mut actions));
    Panel::bottom("status").show(ui, |ui| status::status_bar(ui, view, &mut actions));
    // A fixed width: egui grows a panel to fit wide content and keeps the width after, which
    // would narrow the 3D view for good once a wide row was shown.
    Panel::right("entity_info")
        .resizable(false)
        .exact_size(RIGHT_PANEL_WIDTH)
        .show(ui, |ui| {
            if ui.available_height() >= PANEL_MIN_HEIGHT {
                right::right_panel(ui, view, &mut actions);
            } else {
                // A short window: lay the panel out at its least height and let it scroll.
                egui::ScrollArea::vertical()
                    .id_salt("right-panel")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let size = Vec2::new(ui.available_width(), PANEL_MIN_HEIGHT);
                        ui.allocate_ui(size, |ui| right::right_panel(ui, view, &mut actions));
                    });
            }
        });
    if let Some(angle) = view.soften {
        dialogs::soften_window(ui, angle, &mut actions);
    }
    if let Some(log) = view.agents.log {
        dialogs::agent_activity(ui, &view.agents, log, &mut actions);
    }
    (actions, ui.available_rect_before_wrap())
}
