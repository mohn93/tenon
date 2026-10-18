//! The menu bar drawn as egui menus, from `menus::menu_bar`.

use egui::{Button, Ui, WidgetInfo, WidgetType};

use super::{UiAction, widgets};
use crate::menus::{Entry, Menu};
use crate::shortcuts::Bound;

/// Draws `menus` as menu buttons, appending the action of each item clicked to `actions`.
pub(super) fn show(ui: &mut Ui, menus: &[Menu], actions: &mut Vec<UiAction>) {
    for menu in menus {
        ui.menu_button(menu.title, |ui| {
            entries(ui, menu.title, &menu.entries, actions)
        });
    }
}

/// Draws a menu's entries inside its open area, which is named "`title` menu" (its button is
/// named `title`).
fn entries(ui: &mut Ui, title: &str, list: &[Entry], actions: &mut Vec<UiAction>) {
    // egui senses clicks on the menu's area (egui 0.36's id for it, `Area::begin`: the layer's
    // id with "move") without naming it.
    widgets::name_egui_widget(
        ui.ctx(),
        ui.layer_id().id.with("move"),
        egui::accesskit::Role::Menu,
        || format!("{title} menu"),
    );
    for entry in list {
        match entry {
            Entry::Item(item) => {
                let response = match item.checked {
                    None => widgets::menu_item(ui, &item.label, item.bound, item.enabled),
                    Some(checked) => {
                        let mut button = Button::selectable(checked, &item.label);
                        if let Some(key) = item.bound.and_then(Bound::key_text) {
                            button = button.shortcut_text(key);
                        }
                        let response = ui.add_enabled(item.enabled, button);
                        // Named by the label alone, without the key.
                        response.widget_info(|| {
                            WidgetInfo::selected(
                                WidgetType::SelectableLabel,
                                item.enabled,
                                checked,
                                &item.label,
                            )
                        });
                        response
                    }
                };
                let response = match &item.tooltip {
                    Some(tip) => response.on_hover_text(tip.as_str()),
                    None => response,
                };
                if response.clicked() {
                    actions.push(item.action.clone());
                }
            }
            Entry::Separator => {
                ui.separator();
            }
            Entry::Submenu {
                label,
                entries: sub,
            } => {
                ui.menu_button(*label, |ui| entries(ui, label, sub, actions));
            }
        }
    }
}
