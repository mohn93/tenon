//! The window's panels: the context bar, the tool rail, the status bar with the measurements
//! box, and on the right the dock: Entity Info and Materials as tabs over the Outliner. Over
//! the 3D view, the tools' snap marker, preview lines and chip, the tool's cursor and the
//! orientation gizmo.

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{Panel, Ui};
use kernel::{EntityId, TexturePlacement};
use session::{Camera, Lighting, MaterialId, OutlinerTable, Projection, StandardView};
use tools::ToolKind;

use crate::editor::{ImportUnits, Status};
use crate::feedback::Hud;
use theme::ThemeChoice;

pub(crate) mod context_bar;
mod dialogs;
mod dock;
mod fonts;
mod icons;
mod materials;
mod menu_bar;
mod outliner;
mod rail;
mod right;
mod start;
mod status;
#[cfg(test)]
mod tests;
pub mod theme;
pub(crate) mod viewport;
pub(crate) use context_bar::step_name;
pub use dock::DockLayout;
// Only the tests name a tab outside the dock.
#[cfg(test)]
pub use dock::DockTab;
pub(crate) mod widgets;

/// What the user asked for through the panels this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    Tool(ToolKind),
    Undo,
    Redo,
    /// Select everything in the open context, keeping the tool.
    SelectAll,
    /// Select nothing, keeping the tool.
    SelectNone,
    /// Copy the selection with Move, the base point under the cursor.
    Duplicate,
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
    /// Choose the material to paint with (None: the default material), keeping the tool.
    ChooseMaterial(Option<MaterialId>),
    /// Choose the material and the Paint tool (a material tile double-clicked).
    PaintWithMaterial(Option<MaterialId>),
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
    /// Ask about unsaved changes, then quit.
    Quit,
    /// Open or close the Agent Activity window.
    ShowAgentActivity(bool),
    /// Draw the window in this theme, and keep it in the preferences.
    SetTheme(ThemeChoice),
    /// Lay the dock out so (a resize, a divider drag or a tab chosen), and keep it in the
    /// preferences.
    SetDock(DockLayout),
    /// Open or close the Keyboard Shortcuts window.
    ShowShortcuts(bool),
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
    /// Open this recent file (after asking about unsaved changes).
    OpenRecent(PathBuf),
    /// Empty the recent files, and keep that in the preferences.
    ClearRecent,
    /// Hide the empty-model card for the rest of the session.
    DismissStart,
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
    /// The label of the step Undo would undo (`Document::undo_label`); None: nothing to undo.
    pub undo: Option<&'a str>,
    /// The label of the step Redo would redo.
    pub redo: Option<&'a str>,
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
    /// The theme chosen in View > Theme.
    pub theme: ThemeChoice,
    /// Whether the Keyboard Shortcuts window is open.
    pub shortcuts: bool,
    /// Whether the menus are drawn by egui (otherwise the window has native ones).
    pub egui_menus: bool,
    /// The dock's width, split and tab, as kept in the preferences.
    pub dock: DockLayout,
    /// What is drawn over the 3D view for the tools.
    pub hud: &'a Hud,
    /// The pointer over the 3D view (`viewport::cursor_icon`).
    pub cursor: egui::CursorIcon,
    /// The camera the orientation gizmo follows (its projection is `projection`).
    pub camera: &'a Camera,
    /// The card over an empty, unedited model; None: hidden.
    pub start: Option<StartCard<'a>>,
    /// The recent files as kept in the preferences, newest first (File > Open Recent).
    pub recent: &'a [PathBuf],
}

/// What the empty-model card lists; `View::start` is None when the card is hidden.
pub struct StartCard<'a> {
    /// The recent files that exist, newest first (at most `prefs::CARD_RECENT`).
    pub recent: &'a [PathBuf],
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

/// The dock panel's id.
const DOCK_ID: &str = "dock";

/// The id of the dock panel's resize edge, as egui 0.36 registers it (`Panel::resize_id`, not
/// public): the panel's id with "__resize".
fn dock_resize_id() -> egui::Id {
    egui::Id::new(DOCK_ID).with("__resize")
}

/// Lays out the panels for one pass, appending the actions asked for to `actions`. Returns the
/// rectangle left for the 3D view, in points.
///
/// egui can lay a frame out more than once, and only the first pass sees the frame's input; so
/// a caller keeps every pass's actions and the last pass's rectangle.
pub fn draw(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) -> egui::Rect {
    let t = theme::tokens(ui.ctx().theme());
    Panel::top("context")
        .resizable(false)
        .exact_size(theme::CONTEXT_BAR_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::symmetric(8, 0)),
        )
        .show(ui, |ui| context_bar::context_bar(ui, view, actions));
    Panel::bottom("status")
        .resizable(false)
        .exact_size(theme::STATUS_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::symmetric(8, 0)),
        )
        .show(ui, |ui| status::status_bar(ui, view, actions));
    Panel::left("rail")
        .resizable(false)
        .exact_size(theme::RAIL_WIDTH)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::symmetric(6, 8)),
        )
        .show(ui, |ui| rail::rail(ui, view, actions));
    let panel = Panel::right(DOCK_ID)
        .resizable(true)
        .default_size(view.dock.width)
        .size_range(dock::DOCK_WIDTH)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::same(dock::DOCK_MARGIN)),
        )
        .show(ui, |ui| dock::dock(ui, view, actions));
    let mut layout = panel.inner;
    // The width the panel's edge was dragged to, once the drag ends: not while dragging, not a
    // width that only rounding moved, and not a width the panel was held to without a drag
    // (the window too narrow for the saved width).
    let width = panel.response.rect.width();
    let edge_dragged = ui
        .ctx()
        .read_response(dock_resize_id())
        .is_some_and(|r| r.drag_stopped());
    if edge_dragged && (width - view.dock.width).abs() > 0.5 {
        layout.width = width.round();
    }
    if layout != view.dock {
        actions.push(UiAction::SetDock(layout));
    }
    if let Some(angle) = view.soften {
        dialogs::soften_window(ui, angle, actions);
    }
    if let Some(log) = view.agents.log {
        dialogs::agent_activity(ui, &view.agents, log, actions);
    }
    if view.shortcuts {
        dialogs::shortcuts_window(ui, actions);
    }
    let central = ui.available_rect_before_wrap();
    // In the root's own layer, under every window and menu.
    viewport::hud(ui, central, view.hud);
    if let Some(card) = &view.start {
        start::start_card(ui, central, card, actions);
    }
    if !view.flying {
        viewport::gizmo(ui, central, view.camera, view.projection, actions);
    }
    viewport::set_cursor(ui.ctx(), central, view.cursor);
    central
}
