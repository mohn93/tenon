//! The window's panels: toolbar, status bar with the measurements box, and on the right Entity
//! Info, the Materials list and the Outliner.

use std::collections::BTreeSet;

use egui::collapsing_header::CollapsingState;
use egui::{Align, Color32, Layout, Panel, RichText, Sense, Ui, Vec2};
use kernel::{DefinitionKind, EntityId};
use session::{MaterialId, OutlinerRow, OutlinerTable, Projection, StandardView};
use tools::ToolKind;

use crate::editor::{ImportUnits, MATERIAL_PALETTE, Status};

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
    /// Paint with this material (None: the default material), switching to the Paint tool.
    ChooseMaterial(Option<MaterialId>),
    AddMaterial,
    DeleteMaterial(MaterialId),
    RenameMaterial(MaterialId, String),
    RecolorMaterial(MaterialId, [u8; 3]),
    /// Set a material's opacity, in percent (0 to 100).
    SetMaterialOpacity(MaterialId, u8),
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
}

/// A material as the Materials list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialRow {
    pub id: MaterialId,
    pub name: String,
    pub color: [u8; 3],
    /// From 0 (invisible) to 1 (opaque).
    pub opacity: f32,
}

/// The default material's swatch: its front color.
const DEFAULT_SWATCH: Color32 = Color32::from_rgb(237, 237, 230);

/// What the panels show.
pub struct View<'a> {
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
}

/// The fly mode controls, shown over the Camera menu's Fly mode item.
pub const FLY_HELP: &str = "Walk or fly through the model.\n\
    Mouse: look around (trackpad: two-finger scroll)\n\
    W A S D: forward, left, back, right\n\
    E or Space: up; Q or Ctrl: down\n\
    Wheel, [ ] or - =: slower, faster (trackpad: Cmd+scroll)\n\
    Shift: 3x faster; Option/Alt: slower\n\
    Esc or F: leave fly mode";

/// The least height the Outliner's list keeps when the panel is short, in points.
const OUTLINER_MIN_HEIGHT: f32 = 120.0;

/// The Materials list's greatest height, in points.
const MATERIALS_MAX_HEIGHT: f32 = 220.0;

/// Room kept below the Materials list for its buttons, name field and colors, and for the
/// Outliner's heading and minimum height, in points.
const BELOW_MATERIALS_LIST: f32 = 166.0 + OUTLINER_MIN_HEIGHT;

/// The least height the right panel lays out its contents in; a shorter panel scrolls them.
const PANEL_MIN_HEIGHT: f32 = 480.0;

/// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
/// the 3D view, in points.
pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
    let mut actions = Vec::new();
    Panel::top("toolbar").show(ui, |ui| {
        // Wraps onto more rows when the window is narrower than the buttons (see the ui tests).
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("File", |ui| {
                let items = [
                    ("New", "Cmd+N", UiAction::NewModel),
                    ("Open...", "Cmd+O", UiAction::OpenFile),
                    ("Save", "Cmd+S", UiAction::SaveFile),
                    ("Save As...", "Shift+Cmd+S", UiAction::SaveFileAs),
                    ("Import...", "Cmd+I", UiAction::ImportFile),
                    ("Export...", "Shift+Cmd+E", UiAction::ExportFile),
                ];
                for (label, key, action) in items {
                    if ui
                        .add(egui::Button::new(label).shortcut_text(key))
                        .clicked()
                    {
                        actions.push(action);
                    }
                }
                ui.menu_button("Import Units", |ui| {
                    for units in ImportUnits::ALL {
                        if ui
                            .selectable_label(view.import_units == units, units.label())
                            .clicked()
                        {
                            actions.push(UiAction::SetImportUnits(units));
                        }
                    }
                });
            });
            ui.menu_button("Edit", |ui| {
                let erase = egui::Button::new("Erase Selection").shortcut_text("Delete");
                if ui.add_enabled(view.has_selection, erase).clicked() {
                    actions.push(UiAction::EraseSelection);
                }
                if ui
                    .add_enabled(view.has_guides, egui::Button::new("Delete Guides"))
                    .clicked()
                {
                    actions.push(UiAction::DeleteGuides);
                }
            });
            ui.menu_button("Camera", |ui| {
                if ui
                    .add(egui::Button::new("Zoom Extents").shortcut_text("Shift+Z"))
                    .clicked()
                {
                    actions.push(UiAction::ZoomExtents);
                }
                ui.separator();
                for v in StandardView::ALL {
                    if ui.button(v.label()).clicked() {
                        actions.push(UiAction::StandardView(v));
                    }
                }
                ui.separator();
                for (projection, label) in [
                    (Projection::Perspective, "Perspective"),
                    (Projection::Parallel, "Parallel Projection"),
                ] {
                    if ui
                        .selectable_label(view.projection == projection, label)
                        .clicked()
                    {
                        actions.push(UiAction::SetProjection(projection));
                    }
                }
                ui.separator();
                let mut flying = view.flying;
                if ui
                    .checkbox(&mut flying, "Fly mode (F)")
                    .on_hover_text(FLY_HELP)
                    .clicked()
                {
                    actions.push(UiAction::ToggleFly);
                }
            });
            ui.separator();
            for kind in ToolKind::ALL {
                let key = match kind.shortcut() {
                    ' ' => "Space".to_string(),
                    c => c.to_ascii_uppercase().to_string(),
                };
                let button = ui
                    .selectable_label(view.tool == kind, kind.label())
                    .on_hover_text(format!("{} ({key})", kind.label()));
                if button.clicked() {
                    actions.push(UiAction::Tool(kind));
                }
            }
            ui.separator();
            if ui.button("Undo").on_hover_text("Cmd+Z").clicked() {
                actions.push(UiAction::Undo);
            }
            if ui.button("Redo").on_hover_text("Shift+Cmd+Z").clicked() {
                actions.push(UiAction::Redo);
            }
            ui.separator();
            let buttons = [
                (view.has_selection, "Make group", "G", UiAction::MakeGroup),
                (
                    view.has_selection,
                    "Make component",
                    "Shift+G",
                    UiAction::MakeComponent,
                ),
                (view.has_instance_selected, "Explode", "", UiAction::Explode),
                (
                    view.inside_instance,
                    "Close group",
                    "Escape",
                    UiAction::CloseInstance,
                ),
            ];
            for (enabled, label, key, action) in buttons {
                let mut button = ui.add_enabled(enabled, egui::Button::new(label));
                if !key.is_empty() {
                    button = button.on_hover_text(key);
                }
                if button.clicked() {
                    actions.push(action);
                }
            }
        });
    });
    Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(&view.status.prompt);
            if let Some(lock) = view.status.lock {
                ui.label(RichText::new(lock).strong());
            }
            if let Some(message) = &view.status.message {
                ui.label(RichText::new(message).color(ui.visuals().warn_fg_color));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // The measurements box: typed text, or the tool's live measurement.
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(120.0);
                    ui.monospace(view.status.value.clone().unwrap_or_default());
                });
                ui.label(view.status.label);
            });
        });
    });
    Panel::right("entity_info")
        .resizable(false)
        .default_size(180.0)
        .show(ui, |ui| {
            if ui.available_height() >= PANEL_MIN_HEIGHT {
                right_panel(ui, view, &mut actions);
            } else {
                // A short window: lay the panel out at its least height and let it scroll.
                egui::ScrollArea::vertical()
                    .id_salt("right-panel")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let size = Vec2::new(ui.available_width(), PANEL_MIN_HEIGHT);
                        ui.allocate_ui(size, |ui| right_panel(ui, view, &mut actions));
                    });
            }
        });
    (actions, ui.available_rect_before_wrap())
}

/// Entity Info, the Materials list and the Outliner.
fn right_panel(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    ui.heading("Entity Info");
    for line in view.info {
        ui.label(line);
    }
    ui.separator();
    materials(ui, view, actions);
    ui.separator();
    outliner(ui, view, actions);
}

/// A small square of `color` that can be clicked.
fn swatch(ui: &mut Ui, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
    ui.painter().rect_filled(rect, 2.0, color);
    response
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The Materials list: the default material and every material, the one painted with
/// highlighted (clicking one chooses it and the Paint tool); Add and Delete; and for the
/// chosen material its name (renamed on Enter or leaving the field), colors to pick from and
/// its opacity (set when a drag of the slider ends).
fn materials(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    ui.heading("Materials");
    // Shorter when the panel is short, so the Outliner below keeps its minimum height.
    let max_height =
        (ui.available_height() - BELOW_MATERIALS_LIST).clamp(48.0, MATERIALS_MAX_HEIGHT);
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            let rows = std::iter::once((None, "Default", DEFAULT_SWATCH)).chain(
                view.materials
                    .iter()
                    .map(|m| (Some(m.id), m.name.as_str(), rgb(m.color))),
            );
            for (id, name, color) in rows {
                ui.horizontal(|ui| {
                    let picked = swatch(ui, color).clicked();
                    let label = ui.selectable_label(view.material == id, name);
                    if picked || label.clicked() {
                        actions.push(UiAction::ChooseMaterial(id));
                    }
                });
            }
        });
    ui.horizontal(|ui| {
        if ui.button("Add").clicked() {
            actions.push(UiAction::AddMaterial);
        }
        let current = view.material;
        if ui
            .add_enabled(current.is_some(), egui::Button::new("Delete"))
            .clicked()
            && let Some(id) = current
        {
            actions.push(UiAction::DeleteMaterial(id));
        }
    });
    let Some(m) = view
        .material
        .and_then(|id| view.materials.iter().find(|m| m.id == id))
    else {
        return;
    };
    // The name being typed lives in egui's memory until the field loses focus.
    let key = egui::Id::new(("material-name", m.id.0));
    let mut text = ui
        .data(|d| d.get_temp::<String>(key))
        .unwrap_or_else(|| m.name.clone());
    let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
    if edit.lost_focus() {
        // Escape discards the edit; so does deleting the material in the same frame.
        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let deleted = actions.contains(&UiAction::DeleteMaterial(m.id));
        if text != m.name && !escaped && !deleted {
            actions.push(UiAction::RenameMaterial(m.id, text));
        }
        ui.data_mut(|d| d.remove::<String>(key));
    } else if edit.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
    ui.horizontal_wrapped(|ui| {
        for color in MATERIAL_PALETTE {
            let response = swatch(ui, rgb(color));
            if response.clicked() && color != m.color {
                actions.push(UiAction::RecolorMaterial(m.id, color));
            }
        }
    });
    // The value being dragged lives in egui's memory until the drag ends, so a drag is one
    // change (one undo step).
    let key = egui::Id::new(("material-opacity", m.id.0));
    let shown = percent(m.opacity);
    let mut value = ui.data(|d| d.get_temp::<u8>(key)).unwrap_or(shown);
    let slider = ui
        .horizontal(|ui| {
            ui.label("Opacity");
            // Fit the panel: the slider takes what the label and the number leave.
            ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(40.0);
            ui.add(egui::Slider::new(&mut value, 0..=100).suffix("%"))
        })
        .inner;
    if slider.dragged() {
        ui.data_mut(|d| d.insert_temp(key, value));
    } else {
        if (slider.drag_stopped() || slider.changed()) && value != shown {
            actions.push(UiAction::SetMaterialOpacity(m.id, value));
        }
        ui.data_mut(|d| d.remove::<u8>(key));
    }
}

/// An opacity (0 to 1) in whole percent.
fn percent(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// The Outliner: a Model row (back to the root) and the tree of groups and components, the
/// selected ones highlighted and the open ones bold. A click selects a node, a double-click
/// opens it. Only expanded nodes' children are laid out.
fn outliner(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    ui.heading("Outliner");
    expand_open_path(ui.ctx(), view.context);
    #[cfg(test)]
    ui.data_mut(|d| d.remove::<Vec<tests::Recorded>>(tests::recorded_id()));
    egui::ScrollArea::vertical()
        .id_salt("outliner")
        .auto_shrink([false, true])
        .min_scrolled_height(OUTLINER_MIN_HEIGHT)
        .show(ui, |ui| {
            let at_root = view.context.is_empty();
            let mut model = RichText::new("Model");
            if at_root {
                model = model.strong();
            }
            let row = row_button(ui, false, model);
            #[cfg(test)]
            tests::record(ui, &[], row.rect, false, false, at_root);
            // Already at the root: nothing to do (and the tool keeps its operation).
            if row.clicked() && !at_root {
                actions.push(UiAction::OpenPath(Vec::new()));
            }
            let mut path = Vec::new();
            for row in &view.outliner.roots {
                outliner_node(ui, view, row, &mut path, actions);
            }
        });
}

/// The collapsing state's id of the node at `path`.
fn outliner_id(path: &[EntityId]) -> egui::Id {
    egui::Id::new(("outliner", path))
}

/// Expands the nodes along the open path when it changed since the last frame, so the open
/// instance and its contents show.
fn expand_open_path(ctx: &egui::Context, context: &[EntityId]) {
    let key = egui::Id::new("outliner-context");
    if ctx.data(|d| d.get_temp::<Vec<EntityId>>(key)).as_deref() == Some(context) {
        return;
    }
    for end in 1..=context.len() {
        let mut state =
            CollapsingState::load_with_default_open(ctx, outliner_id(&context[..end]), true);
        state.set_open(true);
        state.store(ctx);
    }
    ctx.data_mut(|d| d.insert_temp(key, context.to_vec()));
}

/// One node and, when expanded, its children. `path` leads to the node's parent.
fn outliner_node(
    ui: &mut Ui,
    view: &View,
    row: &OutlinerRow,
    path: &mut Vec<EntityId>,
    actions: &mut Vec<UiAction>,
) {
    let selected = path.as_slice() == view.context && view.selection.contains(&row.id);
    path.push(row.id);
    let open = view.context.starts_with(path);
    let children = view.outliner.children(row.definition);
    if children.is_empty() {
        let clicked = ui
            .horizontal(|ui| {
                // Lined up with the names of nodes that have an expand button.
                ui.add_space(ui.spacing().indent);
                outliner_row(ui, row, path, selected, open)
            })
            .inner;
        actions.extend(clicked);
    } else {
        // Nodes on the open path start expanded.
        let state = CollapsingState::load_with_default_open(ui.ctx(), outliner_id(path), open);
        let (_toggle, header, _) = state
            .show_header(ui, |ui| outliner_row(ui, row, path, selected, open))
            .body(|ui| {
                for child in children {
                    outliner_node(ui, view, child, path, actions);
                }
            });
        #[cfg(test)]
        tests::record(ui, path, _toggle.rect, true, false, false);
        actions.extend(header.inner);
    }
    path.pop();
}

/// A node's name and what clicking it asks for. `path` leads to the node.
fn outliner_row(
    ui: &mut Ui,
    row: &OutlinerRow,
    path: &[EntityId],
    selected: bool,
    open: bool,
) -> Option<UiAction> {
    let mut text = RichText::new(outliner_label(row));
    if open {
        text = text.strong();
    }
    let response = row_button(ui, selected, text);
    #[cfg(test)]
    tests::record(ui, path, response.rect, false, selected, open);
    if response.double_clicked() {
        Some(UiAction::OpenPath(path.to_vec()))
    } else if response.clicked() {
        // Shift, or Cmd on macOS and Ctrl elsewhere. Ctrl is accepted on macOS too, though
        // the system may turn a Ctrl-click into a secondary click there.
        let toggle = ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);
        Some(UiAction::OutlinerSelect {
            path: path.to_vec(),
            toggle,
        })
    } else {
        None
    }
}

/// A selectable row as wide as the space left, its text on the left (cut short if too long).
fn row_button(ui: &mut Ui, selected: bool, text: RichText) -> egui::Response {
    let width = ui.available_width();
    ui.add(
        egui::Button::selectable(selected, text)
            .right_text("")
            .truncate()
            .min_size(Vec2::new(width, 0.0)),
    )
}

/// How the Outliner names a node: a component as `<Name>`, a group by its name or as `Group`.
fn outliner_label(row: &OutlinerRow) -> String {
    match row.kind {
        DefinitionKind::Component => format!("<{}>", row.name),
        DefinitionKind::Group if row.name.is_empty() => "Group".to_string(),
        DefinitionKind::Group => row.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2};

    static NO_ROWS: OutlinerTable = OutlinerTable {
        roots: Vec::new(),
        by_definition: std::collections::BTreeMap::new(),
    };
    static NO_SELECTION: BTreeSet<EntityId> = BTreeSet::new();

    fn run(ctx: &egui::Context, input: egui::RawInput, view: &View) -> (Vec<UiAction>, Rect) {
        let mut out = None;
        ctx.run_ui(input, |ui| out = Some(draw(ui, view)))
            .drop_without_applying_deltas();
        out.unwrap()
    }

    /// The window's default size (see `app.rs`); the toolbar fits one row of it.
    fn screen() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
            ..Default::default()
        }
    }

    fn status() -> Status {
        Status {
            tool: "Line",
            prompt: "Click to start a line".into(),
            label: "Length",
            value: Some("2.5_".into()),
            message: Some("Not a length: x".into()),
            lock: None,
        }
    }

    /// One frame with a primary click (press, then release in the next frame) at `pos`.
    fn click(ctx: &egui::Context, view: &View, pos: Pos2) -> Vec<UiAction> {
        let pointer = |pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut press = screen();
        press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
        let mut actions = run(ctx, press, view).0;
        let mut release = screen();
        release.events = vec![pointer(false)];
        actions.extend(run(ctx, release, view).0);
        actions
    }

    #[test]
    fn the_file_menu_reports_its_items() {
        let ctx = egui::Context::default();
        let status = status();
        let info = Vec::new();
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: false,
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: &[],
            material: None,
            outliner: &NO_ROWS,
            context: &[],
            selection: &NO_SELECTION,
        };
        let (_, central) = run(&ctx, screen(), &view);
        let file = Pos2::new(14.0, central.min.y / 2.0);
        let mut clicked = Vec::new();
        // Open the menu, then click one spot below it; repeat down the menu.
        for y in (0..160).step_by(6) {
            click(&ctx, &view, file);
            let _ = run(&ctx, screen(), &view);
            clicked.extend(click(
                &ctx,
                &view,
                Pos2::new(40.0, central.min.y + y as f32),
            ));
            // Close the menu if the click missed every item.
            let mut escape = screen();
            escape.events = vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }];
            let _ = run(&ctx, escape, &view);
        }
        for action in [
            UiAction::NewModel,
            UiAction::OpenFile,
            UiAction::SaveFile,
            UiAction::SaveFileAs,
            UiAction::ImportFile,
            UiAction::ExportFile,
        ] {
            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
        }
    }

    /// The actions produced by opening the menu under each x in `xs` on the toolbar row and
    /// clicking one spot below it, down the menu.
    fn menu_actions(view: &View, xs: std::ops::Range<i32>) -> Vec<UiAction> {
        let ctx = egui::Context::default();
        let (_, central) = run(&ctx, screen(), view);
        let row = central.min.y / 2.0;
        let mut clicked = Vec::new();
        for x in xs.step_by(12) {
            for y in (0..300).step_by(8) {
                click(&ctx, view, Pos2::new(x as f32, row));
                let _ = run(&ctx, screen(), view);
                clicked.extend(click(
                    &ctx,
                    view,
                    Pos2::new(x as f32 + 10.0, central.min.y + y as f32),
                ));
                let mut escape = screen();
                escape.events = vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }];
                let _ = run(&ctx, escape, view);
            }
        }
        clicked
    }

    #[test]
    fn the_camera_menu_has_views_zoom_extents_and_projections() {
        let status = status();
        let info = Vec::new();
        let mut view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: false,
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: &[],
            material: None,
            outliner: &NO_ROWS,
            context: &[],
            selection: &NO_SELECTION,
        };
        // The Camera menu sits right after File and Edit.
        let clicked = menu_actions(&view, 80..140);
        assert!(clicked.contains(&UiAction::ZoomExtents), "{clicked:?}");
        for v in StandardView::ALL {
            assert!(
                clicked.contains(&UiAction::StandardView(v)),
                "{v:?} in {clicked:?}"
            );
        }
        for p in [Projection::Perspective, Projection::Parallel] {
            assert!(
                clicked.contains(&UiAction::SetProjection(p)),
                "{p:?} in {clicked:?}"
            );
        }
        assert!(clicked.contains(&UiAction::ToggleFly), "{clicked:?}");
        view.projection = Projection::Parallel;
        let clicked = menu_actions(&view, 80..140);
        assert!(clicked.contains(&UiAction::SetProjection(Projection::Perspective)));
    }

    fn edit_menu_actions(has_selection: bool, has_guides: bool) -> Vec<UiAction> {
        let status = status();
        let info = Vec::new();
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection,
            has_instance_selected: false,
            inside_instance: false,
            has_guides,
            projection: Projection::Perspective,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: &[],
            material: None,
            outliner: &NO_ROWS,
            context: &[],
            selection: &NO_SELECTION,
        };
        // The Edit button sits right after File (which ends before x = 40).
        menu_actions(&view, 40..80)
    }

    #[test]
    fn the_edit_menu_erases_the_selection_and_deletes_guides() {
        let clicked = edit_menu_actions(true, true);
        assert!(clicked.contains(&UiAction::EraseSelection), "{clicked:?}");
        assert!(clicked.contains(&UiAction::DeleteGuides), "{clicked:?}");
    }

    #[test]
    fn the_edit_menu_items_need_a_selection_and_guides() {
        let clicked = edit_menu_actions(false, false);
        assert!(!clicked.contains(&UiAction::EraseSelection), "{clicked:?}");
        assert!(!clicked.contains(&UiAction::DeleteGuides), "{clicked:?}");
    }

    #[test]
    fn panels_leave_a_central_view_inside_the_window() {
        let ctx = egui::Context::default();
        let status = status();
        let info = vec!["Nothing selected".to_string()];
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: false,
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: &[],
            material: None,
            outliner: &NO_ROWS,
            context: &[],
            selection: &NO_SELECTION,
        };
        let (actions, rect) = run(&ctx, screen(), &view);
        assert!(actions.is_empty());
        assert!(
            rect.min.x == 0.0 && rect.min.y > 0.0,
            "toolbar above: {rect:?}"
        );
        assert!(rect.max.x < 1280.0, "info panel on the right: {rect:?}");
        assert!(rect.max.y < 800.0, "status bar below: {rect:?}");
        assert!(rect.width() > 600.0 && rect.height() > 500.0, "{rect:?}");
    }

    /// A `width` x 800 point window.
    fn screen_of(width: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 800.0))),
            ..Default::default()
        }
    }

    /// Runs `f` with a view that has every toolbar button enabled.
    fn with_toolbar_view<R>(f: impl FnOnce(&View) -> R) -> R {
        let status = status();
        let info = Vec::new();
        let view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: true,
            has_instance_selected: true,
            inside_instance: true,
            has_guides: true,
            projection: Projection::Perspective,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: &[],
            material: None,
            outliner: &NO_ROWS,
            context: &[],
            selection: &NO_SELECTION,
        };
        f(&view)
    }

    /// The 3D view's rectangle after one layout pass of a `width`-point window; its top is
    /// the toolbar's height.
    fn central_of(width: f32) -> Rect {
        with_toolbar_view(|view| run(&egui::Context::default(), screen_of(width), view).1)
    }

    /// Clicks over the whole toolbar of a `width`-point window, every 4 points across and 6
    /// down. Returns the actions and the toolbar's height (the 3D view's top).
    fn toolbar_clicks(width: f32) -> (Vec<UiAction>, f32) {
        with_toolbar_view(|view| toolbar_clicks_in(view, width))
    }

    fn toolbar_clicks_in(view: &View, width: f32) -> (Vec<UiAction>, f32) {
        let ctx = egui::Context::default();
        let (_, central) = run(&ctx, screen_of(width), view);
        let mut clicked = Vec::new();
        for y in (3..central.min.y as i32).step_by(6) {
            for x in (2..width as i32).step_by(4) {
                let pos = Pos2::new(x as f32, y as f32);
                let pointer = |pressed: bool| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                let mut press = screen_of(width);
                press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
                let _ = run(&ctx, press, view);
                let mut release = screen_of(width);
                release.events = vec![pointer(false)];
                clicked.extend(run(&ctx, release, view).0);
            }
        }
        (clicked, central.min.y)
    }

    /// Every toolbar button (not the menus) is in `clicked`.
    fn assert_every_toolbar_button(clicked: &[UiAction], width: f32) {
        for kind in ToolKind::ALL {
            assert!(
                clicked.contains(&UiAction::Tool(kind)),
                "{kind:?} at {width} points: {clicked:?}"
            );
        }
        for action in [
            UiAction::Undo,
            UiAction::Redo,
            UiAction::MakeGroup,
            UiAction::MakeComponent,
            UiAction::Explode,
            UiAction::CloseInstance,
        ] {
            assert!(
                clicked.contains(&action),
                "{action:?} at {width} points: {clicked:?}"
            );
        }
    }

    #[test]
    fn clicking_a_tool_button_reports_it() {
        let (clicked, height) = toolbar_clicks(1280.0);
        assert_every_toolbar_button(&clicked, 1280.0);
        // The default window keeps the toolbar on one row, as a very wide one does.
        let wide = central_of(2400.0).min.y;
        assert_eq!(height, wide, "one toolbar row at 1280 points");
    }

    #[test]
    fn a_narrow_window_wraps_the_toolbar_and_keeps_every_button() {
        let one_row = central_of(2400.0).min.y;
        for width in [900.0, 640.0] {
            let (clicked, height) = toolbar_clicks(width);
            assert_every_toolbar_button(&clicked, width);
            assert!(
                height > one_row + 10.0,
                "the toolbar wraps at {width} points: {height} vs {one_row}"
            );
            assert!(height < 200.0, "{height} points of toolbar at {width}");
            let central = central_of(width);
            assert!(
                central.width() > 0.0 && central.height() > 0.0,
                "a 3D view is left at {width} points: {central:?}"
            );
        }
    }

    /// Clicks down the right panel at `x`, every 4 points; after each click, types `typed`
    /// and presses Enter. Returns every action.
    fn panel_actions(view: &View, x: f32, typed: Option<&str>) -> Vec<UiAction> {
        panel_actions_ending(view, x, typed, egui::Key::Enter)
    }

    /// `panel_actions`, ending what was typed with `end` instead of Enter.
    fn panel_actions_ending(
        view: &View,
        x: f32,
        typed: Option<&str>,
        end: egui::Key,
    ) -> Vec<UiAction> {
        let ctx = egui::Context::default();
        let mut clicked = Vec::new();
        for y in (0..800).step_by(4) {
            clicked.extend(click(&ctx, view, Pos2::new(x, y as f32)));
            if let Some(text) = typed {
                let mut typing = screen();
                typing.events = vec![egui::Event::Text(text.into())];
                clicked.extend(run(&ctx, typing, view).0);
                let mut ending = screen();
                ending.events = vec![egui::Event::Key {
                    key: end,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }];
                clicked.extend(run(&ctx, ending, view).0);
                clicked.extend(run(&ctx, screen(), view).0);
            }
        }
        clicked
    }

    #[test]
    fn the_materials_list_chooses_adds_renames_recolors_and_deletes() {
        let status = status();
        let info = vec!["Nothing selected".to_string()];
        let rows = [
            MaterialRow {
                id: MaterialId(31),
                name: "Brick".into(),
                color: [180, 60, 40],
                opacity: 1.0,
            },
            MaterialRow {
                id: MaterialId(32),
                name: "Glass".into(),
                color: [90, 170, 170],
                opacity: 0.4,
            },
        ];
        let mut view = View {
            tool: ToolKind::Line,
            status: &status,
            info: &info,
            has_selection: false,
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: &rows,
            material: Some(MaterialId(31)),
            outliner: &NO_ROWS,
            context: &[],
            selection: &NO_SELECTION,
        };
        // Down the panel's left part: swatches and names, Add, the name field, the palette.
        let clicked = panel_actions(&view, 1280.0 - 160.0, None);
        for action in [
            UiAction::ChooseMaterial(None),
            UiAction::ChooseMaterial(Some(MaterialId(31))),
            UiAction::ChooseMaterial(Some(MaterialId(32))),
            UiAction::AddMaterial,
            UiAction::RecolorMaterial(MaterialId(31), MATERIAL_PALETTE[0]),
        ] {
            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
        }
        // Further right: Delete, and typing into the name field renames on Enter.
        let clicked = panel_actions(&view, 1280.0 - 120.0, Some("Old"));
        assert!(
            clicked.contains(&UiAction::DeleteMaterial(MaterialId(31))),
            "{clicked:?}"
        );
        assert!(
            clicked.iter().any(|a| matches!(
                a,
                UiAction::RenameMaterial(MaterialId(31), name) if name.contains("Old")
            )),
            "{clicked:?}"
        );
        // Clicking on the opacity slider sets the opacity.
        assert!(
            clicked
                .iter()
                .any(|a| matches!(a, UiAction::SetMaterialOpacity(MaterialId(31), p) if *p < 100)),
            "{clicked:?}"
        );
        // With the default material chosen there is nothing to delete, rename or recolor.
        view.material = None;
        let clicked = panel_actions(&view, 1280.0 - 120.0, Some("Old"));
        assert!(
            !clicked.iter().any(|a| matches!(
                a,
                UiAction::DeleteMaterial(_)
                    | UiAction::RenameMaterial(..)
                    | UiAction::RecolorMaterial(..)
                    | UiAction::SetMaterialOpacity(..)
            )),
            "{clicked:?}"
        );
    }

    /// Two materials, the first chosen, for the name-field tests.
    fn name_field_view<'a>(
        status: &'a Status,
        info: &'a [String],
        rows: &'a [MaterialRow],
    ) -> View<'a> {
        View {
            tool: ToolKind::Line,
            status,
            info,
            has_selection: false,
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: rows,
            material: Some(MaterialId(31)),
            outliner: &NO_ROWS,
            context: &[],
            selection: &NO_SELECTION,
        }
    }

    fn brick() -> [MaterialRow; 1] {
        [MaterialRow {
            id: MaterialId(31),
            name: "Brick".into(),
            color: [180, 60, 40],
            opacity: 1.0,
        }]
    }

    #[test]
    fn dragging_the_opacity_slider_sets_the_opacity_once_when_it_ends() {
        let status = status();
        let info = vec!["Nothing selected".to_string()];
        let rows = brick();
        let view = name_field_view(&status, &info, &rows);
        // The slider's row: where a click near the panel's right end sets an opacity.
        let y = (0..800)
            .step_by(2)
            .map(|y| y as f32)
            .find(|y| {
                let ctx = egui::Context::default();
                run(&ctx, screen(), &view);
                click(&ctx, &view, Pos2::new(1280.0 - 80.0, *y))
                    .iter()
                    .any(|a| matches!(a, UiAction::SetMaterialOpacity(..)))
            })
            .expect("found the slider")
            + 6.0;
        let ctx = egui::Context::default();
        run(&ctx, screen(), &view);
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let start = Pos2::new(1280.0 - 82.0, y);
        let mut actions = Vec::new();
        let mut input = screen();
        input.events = vec![egui::Event::PointerMoved(start), button(start, true)];
        actions.extend(run(&ctx, input, &view).0);
        for k in 1..=10 {
            let mut input = screen();
            let at = Pos2::new(start.x - 3.0 * k as f32, y);
            input.events = vec![egui::Event::PointerMoved(at)];
            actions.extend(run(&ctx, input, &view).0);
        }
        assert!(actions.is_empty(), "nothing while dragging: {actions:?}");
        let end = Pos2::new(start.x - 30.0, y);
        let mut input = screen();
        input.events = vec![button(end, false)];
        actions.extend(run(&ctx, input, &view).0);
        actions.extend(run(&ctx, screen(), &view).0);
        assert!(
            matches!(actions.as_slice(), [UiAction::SetMaterialOpacity(MaterialId(31), p)] if *p < 100),
            "{actions:?}"
        );
    }

    #[test]
    fn escape_in_the_material_name_field_discards_the_edit() {
        let status = status();
        let info = vec!["Nothing selected".to_string()];
        let rows = brick();
        let view = name_field_view(&status, &info, &rows);
        let entered = panel_actions(&view, 1280.0 - 120.0, Some("Old"));
        assert!(
            entered
                .iter()
                .any(|a| matches!(a, UiAction::RenameMaterial(..))),
            "Enter renames: {entered:?}"
        );
        let escaped = panel_actions_ending(&view, 1280.0 - 120.0, Some("Old"), egui::Key::Escape);
        assert!(
            !escaped
                .iter()
                .any(|a| matches!(a, UiAction::RenameMaterial(..))),
            "{escaped:?}"
        );
    }

    #[test]
    fn deleting_a_material_does_not_also_rename_it() {
        let status = status();
        let info = vec!["Nothing selected".to_string()];
        let rows = brick();
        let view = name_field_view(&status, &info, &rows);
        let x = 1280.0 - 120.0;
        let row_of = |wanted: fn(&UiAction) -> bool, typed: Option<&str>| {
            (0..800)
                .step_by(4)
                .find(|y| {
                    let ctx = egui::Context::default();
                    run(&ctx, screen(), &view);
                    let mut actions = click(&ctx, &view, Pos2::new(x, *y as f32));
                    if let Some(text) = typed {
                        let mut typing = screen();
                        typing.events = vec![
                            egui::Event::Text(text.into()),
                            egui::Event::Key {
                                key: egui::Key::Enter,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers: Default::default(),
                            },
                        ];
                        actions.extend(run(&ctx, typing, &view).0);
                        actions.extend(run(&ctx, screen(), &view).0);
                    }
                    actions.iter().any(wanted)
                })
                .expect("found the row") as f32
        };
        let delete_y = row_of(|a| matches!(a, UiAction::DeleteMaterial(_)), None);
        let field_y = row_of(|a| matches!(a, UiAction::RenameMaterial(..)), Some("Old"));
        // Type into the field, then press and release on Delete in one frame.
        let ctx = egui::Context::default();
        run(&ctx, screen(), &view);
        let mut actions = click(&ctx, &view, Pos2::new(x, field_y));
        let mut typing = screen();
        typing.events = vec![egui::Event::Text("Old".into())];
        actions.extend(run(&ctx, typing, &view).0);
        let pos = Pos2::new(x, delete_y);
        let pointer = |pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut both = screen();
        both.events = vec![
            egui::Event::PointerMoved(pos),
            pointer(true),
            pointer(false),
        ];
        actions.extend(run(&ctx, both, &view).0);
        actions.extend(run(&ctx, screen(), &view).0);
        assert!(
            actions.contains(&UiAction::DeleteMaterial(MaterialId(31))),
            "{actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, UiAction::RenameMaterial(..))),
            "{actions:?}"
        );
    }

    /// A row the Outliner laid out in the last frame: a node's name (or the Model row, with
    /// an empty path) or a node's expand button, with the marks it was drawn with.
    #[derive(Clone, Debug)]
    pub(super) struct Recorded {
        pub path: Vec<EntityId>,
        pub rect: Rect,
        pub toggle: bool,
        pub selected: bool,
        pub open: bool,
    }

    pub(super) fn recorded_id() -> egui::Id {
        egui::Id::new("outliner-test-rows")
    }

    pub(super) fn record(
        ui: &Ui,
        path: &[EntityId],
        rect: Rect,
        toggle: bool,
        selected: bool,
        open: bool,
    ) {
        let row = Recorded {
            path: path.to_vec(),
            rect,
            toggle,
            selected,
            open,
        };
        ui.data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<Recorded>>(recorded_id())
                .push(row)
        });
    }

    /// Lays out one frame and returns the Outliner's rows.
    fn rows(ctx: &egui::Context, view: &View) -> Vec<Recorded> {
        run(ctx, screen(), view);
        ctx.data(|d| d.get_temp::<Vec<Recorded>>(recorded_id()))
            .unwrap_or_default()
    }

    /// The name row (not the expand button) at `path`.
    fn row<'a>(rows: &'a [Recorded], path: &[EntityId]) -> Option<&'a Recorded> {
        rows.iter().find(|r| !r.toggle && r.path == path)
    }

    fn row_at(rows: &[Recorded], path: &[EntityId]) -> Pos2 {
        row(rows, path)
            .unwrap_or_else(|| panic!("a row for {path:?} in {rows:?}"))
            .rect
            .center()
    }

    /// One click at `pos` with `modifiers` held: press, then release in the next frame.
    fn click_with(
        ctx: &egui::Context,
        view: &View,
        pos: Pos2,
        modifiers: egui::Modifiers,
    ) -> Vec<UiAction> {
        let pointer = |pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        let mut press = screen();
        press.events = vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(pos),
            pointer(true),
        ];
        let mut actions = run(ctx, press, view).0;
        let mut release = screen();
        release.events = vec![egui::Event::ModifiersChanged(modifiers), pointer(false)];
        actions.extend(run(ctx, release, view).0);
        actions
    }

    fn table_row(id: &str, definition: &str, kind: DefinitionKind, name: &str) -> OutlinerRow {
        OutlinerRow {
            id: id.parse().unwrap(),
            definition: definition.parse().unwrap(),
            kind,
            name: name.into(),
            material: None,
        }
    }

    /// Two "Box" components (definition d1) each placing the group i27 (definition d2).
    fn boxes() -> OutlinerTable {
        let component = |id| table_row(id, "d1", DefinitionKind::Component, "Box");
        let group = table_row("i27", "d2", DefinitionKind::Group, "");
        OutlinerTable {
            roots: vec![component("i55"), component("i56")],
            by_definition: [
                ("d1".parse().unwrap(), vec![group]),
                ("d2".parse().unwrap(), Vec::new()),
            ]
            .into(),
        }
    }

    fn ids(ids: &[&str]) -> Vec<EntityId> {
        ids.iter().map(|id| id.parse().unwrap()).collect()
    }

    fn outliner_view<'a>(
        status: &'a Status,
        info: &'a [String],
        table: &'a OutlinerTable,
        context: &'a [EntityId],
        selection: &'a BTreeSet<EntityId>,
    ) -> View<'a> {
        View {
            tool: ToolKind::Line,
            status,
            info,
            has_selection: !selection.is_empty(),
            has_instance_selected: !selection.is_empty(),
            inside_instance: !context.is_empty(),
            has_guides: false,
            projection: Projection::Perspective,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: &[],
            material: None,
            outliner: table,
            context,
            selection,
        }
    }

    #[test]
    fn outliner_labels_name_groups_and_components() {
        let table = boxes();
        assert_eq!(outliner_label(&table.roots[0]), "<Box>");
        assert_eq!(
            outliner_label(&table.children(table.roots[0].definition)[0]),
            "Group"
        );
        let named = table_row("i3", "d4", DefinitionKind::Group, "Wall");
        assert_eq!(outliner_label(&named), "Wall");
    }

    #[test]
    fn outliner_rows_show_the_open_path_and_the_selection_and_select_their_paths() {
        let status = status();
        let info = vec!["1 group".to_string()];
        let table = boxes();
        let (context, selection) = (ids(&["i55"]), ids(&["i27"]).into_iter().collect());
        let view = outliner_view(&status, &info, &table, &context, &selection);
        let laid = rows(&egui::Context::default(), &view);
        // The first box is open, so expanded; the second starts collapsed.
        let names: Vec<&[EntityId]> = laid
            .iter()
            .filter(|r| !r.toggle)
            .map(|r| r.path.as_slice())
            .collect();
        assert_eq!(
            names,
            [ids(&[]), ids(&["i55"]), ids(&["i55", "i27"]), ids(&["i56"])]
        );
        let marks = |path: &[&str]| {
            let r = row(&laid, &ids(path)).unwrap();
            (r.open, r.selected)
        };
        assert_eq!(marks(&["i55"]), (true, false));
        assert_eq!(marks(&["i55", "i27"]), (false, true));
        assert_eq!(marks(&["i56"]), (false, false));

        for (path, modifiers, toggle) in [
            (&["i55"][..], egui::Modifiers::NONE, false),
            (&["i55", "i27"], egui::Modifiers::NONE, false),
            (&["i56"], egui::Modifiers::NONE, false),
            (&["i56"], egui::Modifiers::SHIFT, true),
            (&["i56"], egui::Modifiers::COMMAND, true),
            (&["i56"], egui::Modifiers::CTRL, true),
        ] {
            // A fresh context for each, so no two clicks make a double-click.
            let ctx = egui::Context::default();
            run(&ctx, screen(), &view);
            let clicked = click_with(&ctx, &view, row_at(&laid, &ids(path)), modifiers);
            let wanted = UiAction::OutlinerSelect {
                path: ids(path),
                toggle,
            };
            assert_eq!(clicked, [wanted], "{path:?} with {modifiers:?}");
        }
    }

    #[test]
    fn the_model_row_goes_back_to_the_root_only_from_inside() {
        let status = status();
        let info = Vec::new();
        let table = boxes();
        let selection = BTreeSet::new();
        let inside = ids(&["i55"]);
        for (context, wanted) in [
            (&inside[..], vec![UiAction::OpenPath(Vec::new())]),
            (&[][..], Vec::new()),
        ] {
            let view = outliner_view(&status, &info, &table, context, &selection);
            let ctx = egui::Context::default();
            let laid = rows(&ctx, &view);
            let model = row(&laid, &[]).unwrap();
            assert_eq!(model.open, context.is_empty(), "bold at the root");
            assert_eq!(click(&ctx, &view, model.rect.center()), wanted);
        }
    }

    #[test]
    fn double_clicking_an_outliner_row_opens_it_and_expanding_shows_children() {
        let status = status();
        let info = Vec::new();
        let table = boxes();
        let (context, selection) = (ids(&["i55"]), BTreeSet::new());
        let view = outliner_view(&status, &info, &table, &context, &selection);
        let ctx = egui::Context::default();
        let laid = rows(&ctx, &view);
        let inner = row_at(&laid, &ids(&["i55", "i27"]));
        let mut actions = click(&ctx, &view, inner);
        actions.extend(click(&ctx, &view, inner));
        assert!(
            actions.contains(&UiAction::OpenPath(ids(&["i55", "i27"]))),
            "{actions:?}"
        );

        // The expand button left of the second box shows its group.
        let ctx = egui::Context::default();
        let laid = rows(&ctx, &view);
        let button = laid
            .iter()
            .find(|r| r.toggle && r.path == ids(&["i56"]))
            .expect("the second box has an expand button");
        assert_eq!(click(&ctx, &view, button.rect.center()), []);
        let laid = rows(&ctx, &view);
        let wanted = UiAction::OutlinerSelect {
            path: ids(&["i56", "i27"]),
            toggle: false,
        };
        // Let the opening animation finish before clicking the new row.
        let laid = (0..30).fold(laid, |_, _| rows(&ctx, &view));
        assert_eq!(
            click(&ctx, &view, row_at(&laid, &ids(&["i56", "i27"]))),
            [wanted]
        );
    }

    #[test]
    fn a_newly_opened_path_expands_its_ancestors() {
        let status = status();
        let info = Vec::new();
        let table = boxes();
        let selection = BTreeSet::new();
        let ctx = egui::Context::default();
        let root = outliner_view(&status, &info, &table, &[], &selection);
        let laid = rows(&ctx, &root);
        assert!(row(&laid, &ids(&["i56", "i27"])).is_none(), "collapsed");
        let context = ids(&["i56", "i27"]);
        let inside = outliner_view(&status, &info, &table, &context, &selection);
        let laid = (0..30).fold(Vec::new(), |_, _| rows(&ctx, &inside));
        let group = row(&laid, &context).expect("the open group shows");
        assert!(group.open);
        // Collapsing it by hand sticks while the context stays the same.
        let button = laid
            .iter()
            .find(|r| r.toggle && r.path == ids(&["i56"]))
            .unwrap();
        click(&ctx, &inside, button.rect.center());
        let laid = (0..30).fold(Vec::new(), |_, _| rows(&ctx, &inside));
        assert!(row(&laid, &context).is_none(), "{laid:?}");
    }

    #[test]
    fn a_short_window_leaves_the_outliner_its_minimum_height() {
        let status = status();
        let info = vec!["1 group".to_string()];
        let materials: Vec<MaterialRow> = (0..40)
            .map(|k| MaterialRow {
                id: MaterialId(100 + k),
                name: format!("Material {k}"),
                color: [10, 20, 30],
                opacity: 1.0,
            })
            .collect();
        let table = OutlinerTable {
            roots: (0..40)
                .map(|k| table_row(&format!("i{}", 200 + k), "d1", DefinitionKind::Group, ""))
                .collect(),
            by_definition: Default::default(),
        };
        let selection = BTreeSet::new();
        let mut view = outliner_view(&status, &info, &table, &[], &selection);
        view.materials = &materials;
        view.material = Some(MaterialId(100));
        let height = 520.0;
        let ctx = egui::Context::default();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, height))),
            ..Default::default()
        };
        run(&ctx, input(), &view);
        run(&ctx, input(), &view);
        let laid = ctx
            .data(|d| d.get_temp::<Vec<Recorded>>(recorded_id()))
            .unwrap();
        let model = row(&laid, &[]).unwrap();
        assert!(
            model.rect.min.y <= height - OUTLINER_MIN_HEIGHT,
            "the Outliner starts at {} in a {height} point window",
            model.rect.min.y
        );
    }

    #[test]
    fn a_very_short_window_scrolls_the_right_panel() {
        let status = status();
        let info = vec!["1 group".to_string()];
        let table = OutlinerTable {
            roots: vec![table_row("i200", "d1", DefinitionKind::Group, "")],
            by_definition: Default::default(),
        };
        let selection = BTreeSet::new();
        let materials: Vec<MaterialRow> = (0..40)
            .map(|k| MaterialRow {
                id: MaterialId(100 + k),
                name: format!("Material {k}"),
                color: [10, 20, 30],
                opacity: 1.0,
            })
            .collect();
        let mut view = outliner_view(&status, &info, &table, &[], &selection);
        view.materials = &materials;
        view.material = Some(MaterialId(100));
        let height = 300.0;
        let ctx = egui::Context::default();
        let frame = |k: f64, events: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, height))),
            time: Some(k * 0.1),
            events,
            ..Default::default()
        };
        let model_y = |ctx: &egui::Context| {
            let laid = ctx
                .data(|d| d.get_temp::<Vec<Recorded>>(recorded_id()))
                .unwrap();
            row(&laid, &[]).unwrap().rect.min.y
        };
        run(&ctx, frame(0.0, vec![]), &view);
        run(&ctx, frame(1.0, vec![]), &view);
        let before = model_y(&ctx);
        assert!(
            before > height,
            "the Outliner is below the window at {before}"
        );
        // Scroll with the pointer over Entity Info.
        let over = Pos2::new(1200.0, 60.0);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, -400.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        run(
            &ctx,
            frame(2.0, vec![egui::Event::PointerMoved(over), wheel]),
            &view,
        );
        for k in 3..30 {
            run(&ctx, frame(k as f64, vec![]), &view);
        }
        let after = model_y(&ctx);
        assert!(
            after < before - 100.0,
            "the panel scrolled from {before} to {after}"
        );
    }
}
