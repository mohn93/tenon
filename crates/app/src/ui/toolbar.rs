//! The toolbar: menus, Undo and Redo, and the group buttons.

use egui::Ui;

use super::{UiAction, View, icons, menu_bar, widgets};
use crate::shortcuts::Bound;

pub(super) fn toolbar(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    // Wraps only if the window is narrower than the menus and buttons together; the tests keep
    // it on one row down to the smallest window.
    ui.horizontal_wrapped(|ui| {
        // Tighter than the theme so the toolbar, with all of the menus, stays on one row.
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().button_padding.x = 2.0;
        if view.egui_menus {
            menu_bar::show(ui, &crate::menus::menu_bar(view, false), actions);
        }
        ui.separator();
        let undo = view.undo.map_or("Nothing to undo".into(), |l| {
            format!("Undo {}", step_name(l))
        });
        if widgets::icon_button(
            ui,
            icons::UNDO,
            "Undo",
            &undo,
            Some(Bound::Undo),
            view.undo.is_some(),
        )
        .clicked()
        {
            actions.push(UiAction::Undo);
        }
        let redo = view.redo.map_or("Nothing to redo".into(), |l| {
            format!("Redo {}", step_name(l))
        });
        if widgets::icon_button(
            ui,
            icons::REDO,
            "Redo",
            &redo,
            Some(Bound::Redo),
            view.redo.is_some(),
        )
        .clicked()
        {
            actions.push(UiAction::Redo);
        }
        ui.separator();
        let buttons = [
            (
                view.has_selection,
                "Make group",
                Some(Bound::MakeGroup),
                UiAction::MakeGroup,
            ),
            (
                view.has_selection,
                "Make component",
                Some(Bound::MakeComponent),
                UiAction::MakeComponent,
            ),
            (
                view.has_instance_selected,
                "Explode",
                None,
                UiAction::Explode,
            ),
            (
                view.inside_instance,
                "Close group",
                Some(Bound::CloseInstance),
                UiAction::CloseInstance,
            ),
        ];
        for (enabled, label, bound, action) in buttons {
            let button = ui
                .add_enabled(enabled, egui::Button::new(label))
                .on_hover_ui(|ui| widgets::hint(ui, label, bound))
                .on_disabled_hover_ui(|ui| widgets::hint(ui, label, bound));
            if button.clicked() {
                actions.push(action);
            }
        }
    });
}

/// A history label as words: `push_pull` reads "Push pull".
pub(crate) fn step_name(label: &str) -> String {
    let words = label.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
