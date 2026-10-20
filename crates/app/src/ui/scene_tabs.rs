//! The scene tabs over the 3D view, one per scene in order, the selected scene's highlighted:
//! a click goes to the scene, a double-click renames it, and the right-click menu updates,
//! renames, moves or deletes it, or adds a scene.

use egui::{Align, Layout, Ui, Vec2, WidgetInfo, WidgetType};

use super::scenes::{self, Place};
use super::{SceneRow, UiAction, View, theme, widgets};

/// The width of a tab's name field while it is renamed.
const RENAME_WIDTH: f32 = 140.0;

/// Forgets what the tabs held for scenes no longer listed; for frames they are not drawn
/// (there are no scenes).
pub(super) fn forget(ui: &Ui, view: &View) {
    scenes::forget_gone(ui, Place::Tabs, view);
}

/// The strip's tabs, scrolled sideways when they do not fit.
pub(super) fn scene_tabs(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    forget(ui, view);
    let area = egui::ScrollArea::horizontal()
        .id_salt("scene-tabs")
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_height(theme::SCENE_TABS_HEIGHT);
                for (index, row) in view.scenes.iter().enumerate() {
                    tab(ui, index, row, view.scenes.len(), actions);
                }
            });
        });
    widgets::name_scroll_bars(ui.ctx(), area.id, "scene tabs");
}

/// The tab of the scene at `index` among `count`, or its name field while it is renamed.
fn tab(ui: &mut Ui, index: usize, row: &SceneRow, count: usize, actions: &mut Vec<UiAction>) {
    if scenes::rename_field(ui, Place::Tabs, row, RENAME_WIDTH, actions) {
        return;
    }
    let response = ui.add(
        egui::Button::selectable(row.selected, &row.name)
            .min_size(Vec2::new(0.0, theme::CONTROL_HEIGHT - 4.0)),
    );
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, true, row.selected, &row.name));
    if response.double_clicked() {
        scenes::start_rename(ui, Place::Tabs, row);
    } else if response.clicked() {
        actions.push(UiAction::GoToScene(row.id));
    }
    response.context_menu(|ui| {
        // egui senses clicks on the menu's area (the layer's id with "move") without naming it.
        widgets::name_egui_widget(
            ui.ctx(),
            ui.layer_id().id.with("move"),
            egui::accesskit::Role::Menu,
            || format!("{} menu", row.name),
        );
        let item = |ui: &mut Ui, label: &str, enabled: bool| {
            widgets::menu_item(ui, label, None, enabled).clicked()
        };
        if item(ui, "Update", true) {
            actions.push(UiAction::UpdateScene(row.id));
        }
        if item(ui, "Rename", true) {
            scenes::start_rename(ui, Place::Tabs, row);
        }
        if item(ui, "Move Left", index > 0) {
            actions.push(UiAction::MoveScene(row.id, index - 1));
        }
        if item(ui, "Move Right", index + 1 < count) {
            actions.push(UiAction::MoveScene(row.id, index + 1));
        }
        if item(ui, "Delete", true) {
            actions.push(UiAction::DeleteScene(row.id));
        }
        ui.separator();
        if item(ui, "Add Scene", true) {
            actions.push(UiAction::AddScene);
        }
    });
}
