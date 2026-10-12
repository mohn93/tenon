//! The window's panels: toolbar, status bar with the measurements box, and on the right Entity
//! Info, the Materials list and the Outliner.

use egui::collapsing_header::CollapsingState;
use egui::{Align, Color32, Layout, Panel, RichText, Sense, Ui, Vec2};
use kernel::{DefinitionKind, EntityId};
use session::{MaterialId, OutlinerNode, Projection, StandardView};
use tools::ToolKind;

use crate::editor::{MATERIAL_PALETTE, Status};

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
    /// Select the outliner node at `path` (instances from the root down to it); `toggle`
    /// (Shift or Cmd/Ctrl held) adds or removes it instead of replacing the selection.
    OutlinerSelect {
        path: Vec<EntityId>,
        toggle: bool,
    },
    /// Open the instances along `path` for editing (empty: back to the root).
    OpenPath(Vec<EntityId>),
}

/// A material as the Materials list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialRow {
    pub id: MaterialId,
    pub name: String,
    pub color: [u8; 3],
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
    /// Every material, in id order.
    pub materials: &'a [MaterialRow],
    /// The material the Paint tool paints with (None: the default material).
    pub material: Option<MaterialId>,
    /// The tree of placed groups and components (`Document::outliner`).
    pub outliner: &'a [OutlinerNode],
}

/// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
/// the 3D view, in points.
pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
    let mut actions = Vec::new();
    Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                let items = [
                    ("New", "Cmd+N", UiAction::NewModel),
                    ("Open...", "Cmd+O", UiAction::OpenFile),
                    ("Save", "Cmd+S", UiAction::SaveFile),
                    ("Save As...", "Shift+Cmd+S", UiAction::SaveFileAs),
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
            ui.label(view.status.prompt);
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
            ui.heading("Entity Info");
            for line in view.info {
                ui.label(line);
            }
            ui.separator();
            materials(ui, view, &mut actions);
            ui.separator();
            outliner(ui, view, &mut actions);
        });
    (actions, ui.available_rect_before_wrap())
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
/// chosen material its name (renamed on Enter or leaving the field) and colors to pick from.
fn materials(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    ui.heading("Materials");
    egui::ScrollArea::vertical()
        .max_height(220.0)
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
}

/// The Outliner: a Model row (back to the root) and the tree of groups and components, the
/// selected ones highlighted and the open ones bold. A click selects a node, a double-click
/// opens it.
fn outliner(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    ui.heading("Outliner");
    egui::ScrollArea::vertical()
        .id_salt("outliner")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let mut model = RichText::new("Model");
            if !view.inside_instance {
                model = model.strong();
            }
            if row_button(ui, false, model).clicked() {
                actions.push(UiAction::OpenPath(Vec::new()));
            }
            let mut path = Vec::new();
            for node in view.outliner {
                outliner_node(ui, node, &mut path, actions);
            }
        });
}

/// One node and, when expanded, its children. `path` leads to the node's parent.
fn outliner_node(
    ui: &mut Ui,
    node: &OutlinerNode,
    path: &mut Vec<EntityId>,
    actions: &mut Vec<UiAction>,
) {
    path.push(node.id);
    if node.children.is_empty() {
        let clicked = ui
            .horizontal(|ui| {
                // Lined up with the names of nodes that have an expand button.
                ui.add_space(ui.spacing().indent);
                outliner_row(ui, node, path)
            })
            .inner;
        actions.extend(clicked);
    } else {
        let id = ui.make_persistent_id(("outliner", path.as_slice()));
        // Nodes on the open path start expanded.
        let (_, header, _) = CollapsingState::load_with_default_open(ui.ctx(), id, node.open)
            .show_header(ui, |ui| outliner_row(ui, node, path))
            .body(|ui| {
                for child in &node.children {
                    outliner_node(ui, child, path, actions);
                }
            });
        actions.extend(header.inner);
    }
    path.pop();
}

/// A node's name and what clicking it asks for. `path` leads to the node.
fn outliner_row(ui: &mut Ui, node: &OutlinerNode, path: &[EntityId]) -> Option<UiAction> {
    let mut text = RichText::new(outliner_label(node));
    if node.open {
        text = text.strong();
    }
    let row = row_button(ui, node.selected, text);
    if row.double_clicked() {
        Some(UiAction::OpenPath(path.to_vec()))
    } else if row.clicked() {
        let toggle = ui.input(|i| i.modifiers.shift || i.modifiers.command);
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
fn outliner_label(node: &OutlinerNode) -> String {
    match node.kind {
        DefinitionKind::Component => format!("<{}>", node.name),
        DefinitionKind::Group if node.name.is_empty() => "Group".to_string(),
        DefinitionKind::Group => node.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2};

    fn run(ctx: &egui::Context, input: egui::RawInput, view: &View) -> (Vec<UiAction>, Rect) {
        let mut out = None;
        ctx.run_ui(input, |ui| out = Some(draw(ui, view)))
            .drop_without_applying_deltas();
        out.unwrap()
    }

    /// The window's default size (see `app.rs`); the toolbar needs about 1200 points.
    fn screen() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
            ..Default::default()
        }
    }

    fn status() -> Status {
        Status {
            tool: "Line",
            prompt: "Click to start a line",
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
            materials: &[],
            material: None,
            outliner: &[],
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
            materials: &[],
            material: None,
            outliner: &[],
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
            materials: &[],
            material: None,
            outliner: &[],
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
            materials: &[],
            material: None,
            outliner: &[],
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

    #[test]
    fn clicking_a_tool_button_reports_it() {
        let ctx = egui::Context::default();
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
            materials: &[],
            material: None,
            outliner: &[],
        };
        let (_, central) = run(&ctx, screen(), &view);
        let row = central.min.y / 2.0;
        // Click along the toolbar row until a press lands on the Rectangle button.
        let mut clicked = Vec::new();
        for x in (2..1280).step_by(4) {
            let pos = Pos2::new(x as f32, row);
            let pointer = |pressed: bool| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            let mut press = screen();
            press.events = vec![egui::Event::PointerMoved(pos), pointer(true)];
            let _ = run(&ctx, press, &view);
            let mut release = screen();
            release.events = vec![pointer(false)];
            clicked.extend(run(&ctx, release, &view).0);
        }
        // Paint is the last tool: the toolbar still fits a 1280-point window.
        for kind in [ToolKind::Rectangle, ToolKind::Arc, ToolKind::Paint] {
            assert!(clicked.contains(&UiAction::Tool(kind)), "{clicked:?}");
        }
        assert!(clicked.contains(&UiAction::Undo));
        for action in [
            UiAction::MakeGroup,
            UiAction::MakeComponent,
            UiAction::Explode,
            UiAction::CloseInstance,
        ] {
            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
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
            },
            MaterialRow {
                id: MaterialId(32),
                name: "Glass".into(),
                color: [90, 170, 170],
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
            materials: &rows,
            material: Some(MaterialId(31)),
            outliner: &[],
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
        // With the default material chosen there is nothing to delete, rename or recolor.
        view.material = None;
        let clicked = panel_actions(&view, 1280.0 - 120.0, Some("Old"));
        assert!(
            !clicked.iter().any(|a| matches!(
                a,
                UiAction::DeleteMaterial(_)
                    | UiAction::RenameMaterial(..)
                    | UiAction::RecolorMaterial(..)
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
            materials: rows,
            material: Some(MaterialId(31)),
            outliner: &[],
        }
    }

    fn brick() -> [MaterialRow; 1] {
        [MaterialRow {
            id: MaterialId(31),
            name: "Brick".into(),
            color: [180, 60, 40],
        }]
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

    fn node(
        id: &str,
        kind: DefinitionKind,
        name: &str,
        children: Vec<OutlinerNode>,
    ) -> OutlinerNode {
        OutlinerNode {
            id: id.parse().unwrap(),
            definition: "d1".parse().unwrap(),
            kind,
            name: name.into(),
            material: None,
            open: false,
            selected: false,
            children,
        }
    }

    /// Two "Box" components each placing the group i27; the first open, the group in it
    /// selected.
    fn boxes() -> Vec<OutlinerNode> {
        let group = node("i27", DefinitionKind::Group, "", Vec::new());
        let mut first = node("i55", DefinitionKind::Component, "Box", vec![group.clone()]);
        first.open = true;
        first.children[0].selected = true;
        let second = node("i56", DefinitionKind::Component, "Box", vec![group]);
        vec![first, second]
    }

    fn ids(ids: &[&str]) -> Vec<EntityId> {
        ids.iter().map(|id| id.parse().unwrap()).collect()
    }

    fn outliner_view<'a>(
        status: &'a Status,
        info: &'a [String],
        tree: &'a [OutlinerNode],
    ) -> View<'a> {
        View {
            tool: ToolKind::Line,
            status,
            info,
            has_selection: true,
            has_instance_selected: true,
            inside_instance: true,
            has_guides: false,
            projection: Projection::Perspective,
            materials: &[],
            material: None,
            outliner: tree,
        }
    }

    /// For each y down the right panel at `x` (every 4 points), the actions of one click
    /// there with `modifiers` held, each in a fresh context (so no two make a double-click).
    fn outliner_clicks(view: &View, x: f32, modifiers: egui::Modifiers) -> Vec<(f32, UiAction)> {
        let mut found = Vec::new();
        for y in (0..800).step_by(4) {
            let ctx = egui::Context::default();
            run(&ctx, screen(), view);
            let pos = Pos2::new(x, y as f32);
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
            let mut actions = run(&ctx, press, view).0;
            let mut release = screen();
            release.events = vec![pointer(false)];
            actions.extend(run(&ctx, release, view).0);
            found.extend(actions.into_iter().map(|a| (y as f32, a)));
        }
        found
    }

    #[test]
    fn outliner_labels_name_groups_and_components() {
        let [first, _] = <[OutlinerNode; 2]>::try_from(boxes()).unwrap();
        assert_eq!(outliner_label(&first), "<Box>");
        assert_eq!(outliner_label(&first.children[0]), "Group");
        let named = node("i3", DefinitionKind::Group, "Wall", Vec::new());
        assert_eq!(outliner_label(&named), "Wall");
    }

    #[test]
    fn outliner_rows_select_their_paths_and_the_model_row_goes_back_to_the_root() {
        let status = status();
        let info = vec!["1 group".to_string()];
        let tree = boxes();
        let view = outliner_view(&status, &info, &tree);
        let clicked: Vec<UiAction> = outliner_clicks(&view, 1280.0 - 100.0, Default::default())
            .into_iter()
            .map(|(_, a)| a)
            .collect();
        for action in [
            UiAction::OpenPath(Vec::new()),
            UiAction::OutlinerSelect {
                path: ids(&["i55"]),
                toggle: false,
            },
            UiAction::OutlinerSelect {
                path: ids(&["i55", "i27"]),
                toggle: false,
            },
            UiAction::OutlinerSelect {
                path: ids(&["i56"]),
                toggle: false,
            },
        ] {
            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
        }
        // The second box is not on the open path, so it starts collapsed.
        assert!(
            !clicked.iter().any(|a| matches!(
                a,
                UiAction::OutlinerSelect { path, .. } if *path == ids(&["i56", "i27"])
            )),
            "{clicked:?}"
        );
        let shifted = outliner_clicks(&view, 1280.0 - 100.0, egui::Modifiers::SHIFT);
        assert!(
            shifted.iter().any(|(_, a)| *a
                == UiAction::OutlinerSelect {
                    path: ids(&["i56"]),
                    toggle: true,
                }),
            "{shifted:?}"
        );
    }

    #[test]
    fn double_clicking_an_outliner_row_opens_it_and_expanding_shows_children() {
        let status = status();
        let info = vec!["1 group".to_string()];
        let tree = boxes();
        let view = outliner_view(&status, &info, &tree);
        let x = 1280.0 - 100.0;
        let rows = outliner_clicks(&view, x, Default::default());
        let y_of = |path: &[&str]| {
            rows.iter()
                .find(|(_, a)| {
                    matches!(a, UiAction::OutlinerSelect { path: p, .. } if *p == ids(path))
                })
                .map(|(y, _)| *y)
                .expect("found the row")
        };
        let ctx = egui::Context::default();
        run(&ctx, screen(), &view);
        let inner = Pos2::new(x, y_of(&["i55", "i27"]));
        let mut actions = click(&ctx, &view, inner);
        actions.extend(click(&ctx, &view, inner));
        assert!(
            actions.contains(&UiAction::OpenPath(ids(&["i55", "i27"]))),
            "{actions:?}"
        );
        // The expand button sits left of the name: clicking it shows the second box's group.
        let second = y_of(&["i56"]);
        let (_, central) = run(&egui::Context::default(), screen(), &view);
        let wanted = UiAction::OutlinerSelect {
            path: ids(&["i56", "i27"]),
            toggle: false,
        };
        let expanded = (0..40).step_by(2).any(|dx| {
            let ctx = egui::Context::default();
            run(&ctx, screen(), &view);
            let button = click(&ctx, &view, Pos2::new(central.max.x + dx as f32, second));
            // Wait out the opening animation, then click down the rows below.
            for _ in 0..30 {
                run(&ctx, screen(), &view);
            }
            button.is_empty()
                && (1..16).any(|k| {
                    click(&ctx, &view, Pos2::new(x, second + 4.0 * k as f32)).contains(&wanted)
                })
        });
        assert!(expanded, "an expand button left of the second box");
    }
}
