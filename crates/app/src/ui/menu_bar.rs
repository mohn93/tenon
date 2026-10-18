//! The menu bar drawn as egui menus, from `menus::menu_bar`.

use egui::{Button, Ui, WidgetInfo, WidgetType};

use super::{UiAction, widgets};
use crate::menus::{Entry, Menu};
use crate::shortcuts::Bound;

/// Draws `menus` as menu buttons, appending the action of each item clicked to `actions`.
pub(super) fn show(ui: &mut Ui, menus: &[Menu], actions: &mut Vec<UiAction>) {
    for menu in menus {
        ui.menu_button(menu.title, |ui| entries(ui, &menu.entries, actions));
    }
}

fn entries(ui: &mut Ui, list: &[Entry], actions: &mut Vec<UiAction>) {
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
                ui.menu_button(*label, |ui| entries(ui, sub, actions));
            }
        }
    }
}
