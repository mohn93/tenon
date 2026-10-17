//! The toolbar: menus, tool buttons, Undo and Redo, and the group buttons.

use egui::Ui;
use session::{Lighting, Projection, StandardView};
use tools::ToolKind;

use super::{FLY_HELP, UiAction, View, icons, widgets};
use crate::editor::ImportUnits;
use crate::shortcuts::Bound;

pub(super) fn toolbar(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    // Wraps onto more rows when the window is narrower than the buttons (see the ui tests).
    ui.horizontal_wrapped(|ui| {
        // Tighter than the theme so the default window's toolbar stays on one row (M8b), until
        // M11b moves the tools to a rail.
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().button_padding.x = 4.0;
        ui.menu_button("File", |ui| {
            let items = [
                ("New", Bound::NewModel, UiAction::NewModel),
                ("Open...", Bound::OpenFile, UiAction::OpenFile),
                ("Save", Bound::SaveFile, UiAction::SaveFile),
                ("Save As...", Bound::SaveFileAs, UiAction::SaveFileAs),
                ("Import...", Bound::ImportFile, UiAction::ImportFile),
                ("Export...", Bound::ExportFile, UiAction::ExportFile),
            ];
            for (label, bound, action) in items {
                if widgets::menu_item(ui, label, Some(bound), true).clicked() {
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
            let erase = Some(Bound::EraseSelection);
            if widgets::menu_item(ui, "Erase Selection", erase, view.has_selection).clicked() {
                actions.push(UiAction::EraseSelection);
            }
            if widgets::menu_item(ui, "Delete Guides", None, view.has_guides).clicked() {
                actions.push(UiAction::DeleteGuides);
            }
            ui.separator();
            if widgets::menu_item(ui, "Soften Edges...", None, true).clicked() {
                actions.push(UiAction::ShowSoften(true));
            }
        });
        ui.menu_button("Camera", |ui| {
            if widgets::menu_item(ui, "Zoom Extents", Some(Bound::ZoomExtents), true).clicked() {
                actions.push(UiAction::ZoomExtents);
            }
            ui.separator();
            for v in StandardView::ALL {
                if widgets::menu_item(ui, v.label(), None, true).clicked() {
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
            let fly = format!(
                "Fly mode ({})",
                Bound::ToggleFly.key_text().unwrap_or_default()
            );
            if ui
                .checkbox(&mut flying, fly)
                .on_hover_text(FLY_HELP)
                .clicked()
            {
                actions.push(UiAction::ToggleFly);
            }
        });
        ui.menu_button("View", |ui| {
            if widgets::menu_item(ui, "Agent Activity", None, true).clicked() {
                actions.push(UiAction::ShowAgentActivity(true));
            }
        });
        ui.separator();
        for kind in ToolKind::ALL {
            if widgets::tool_button(ui, kind, view.tool == kind).clicked() {
                actions.push(UiAction::Tool(kind));
            }
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
pub(super) fn step_name(label: &str) -> String {
    let words = label.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
