//! The toolbar: menus, tool buttons, Undo and Redo, and the group buttons.

use egui::Ui;
use session::{Lighting, Projection, StandardView};
use tools::ToolKind;

use super::{FLY_HELP, UiAction, View};
use crate::editor::ImportUnits;

pub(super) fn toolbar(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    // Wraps onto more rows when the window is narrower than the buttons (see the ui tests).
    ui.horizontal_wrapped(|ui| {
        // Tighter than the default so the default window's toolbar stays on one row.
        ui.spacing_mut().item_spacing.x = 7.0;
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
                if item(ui, label, key, true).clicked() {
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
            if item(ui, "Erase Selection", "Delete", view.has_selection).clicked() {
                actions.push(UiAction::EraseSelection);
            }
            if ui
                .add_enabled(view.has_guides, egui::Button::new("Delete Guides"))
                .clicked()
            {
                actions.push(UiAction::DeleteGuides);
            }
            ui.separator();
            if ui.button("Soften Edges...").clicked() {
                actions.push(UiAction::ShowSoften(true));
            }
        });
        ui.menu_button("Camera", |ui| {
            if item(ui, "Zoom Extents", "Shift+Z", true).clicked() {
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
            let mut night = view.lighting == Lighting::Night;
            if ui.checkbox(&mut night, "Night lighting").clicked() {
                let mode = if night {
                    Lighting::Night
                } else {
                    Lighting::Day
                };
                actions.push(UiAction::SetLighting(mode));
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
        ui.menu_button("View", |ui| {
            if ui.button("Agent Activity").clicked() {
                actions.push(UiAction::ShowAgentActivity(true));
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
}

/// A menu item showing `key` on its right, named `label` alone for AccessKit.
fn item(ui: &mut Ui, label: &str, key: &str, enabled: bool) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(key));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label)
    });
    response
}
