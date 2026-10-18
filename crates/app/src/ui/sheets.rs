//! The window's sheets: one modal dialog at a time over the whole window, drawn with
//! `egui::Modal`. Sheets are window chrome: what they hold while open lives in egui's memory,
//! and what they report goes through `UiAction`.

use std::path::{Path, PathBuf};

use egui::{Id, Modal, RichText, Ui};

use super::{UiAction, theme, widgets};
use crate::editor::{ImportAs, ImportOptions, ImportUnits, UpAxis};

/// The sheet open over the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Sheet {
    /// Import Options, after a file was chosen and before it is read.
    Import {
        path: PathBuf,
        options: ImportOptions,
    },
}

/// A sheet's width, in points.
const WIDTH: f32 = 340.0;

/// The id of the sheet's modal area, and the egui memory key of what it holds while open.
fn sheet_id() -> Id {
    Id::new("sheet")
}

/// What an open sheet holds between frames: the sheet it was opened as, and the values being
/// edited. A different sheet starts over from its own values.
#[derive(Clone)]
struct Editing {
    origin: Sheet,
    values: Values,
}

#[derive(Clone)]
enum Values {
    Import(ImportOptions),
}

impl Values {
    /// What `sheet` starts with.
    fn of(sheet: &Sheet) -> Self {
        match sheet {
            Sheet::Import { options, .. } => Values::Import(*options),
        }
    }
}

/// Draws `sheet` over the window. Escape, a click on the backdrop and Cancel report
/// `CloseSheet`; the confirm button reports the sheet's confirm action.
pub(super) fn sheet(ui: &mut Ui, sheet: &Sheet, actions: &mut Vec<UiAction>) {
    let id = sheet_id();
    let mut values = ui
        .data(|d| d.get_temp::<Editing>(id))
        .filter(|e| e.origin == *sheet)
        .map_or_else(|| Values::of(sheet), |e| e.values);
    let modal = Modal::new(id)
        .area(Modal::default_area(id).fade_in(false))
        .show(ui.ctx(), |ui| {
            ui.set_width(WIDTH);
            match (sheet, &mut values) {
                (Sheet::Import { path, .. }, Values::Import(options)) => {
                    import_sheet(ui, path, options, actions);
                }
            }
        });
    ui.data_mut(|d| {
        d.insert_temp(
            id,
            Editing {
                origin: sheet.clone(),
                values,
            },
        )
    });
    if modal.should_close() {
        actions.push(UiAction::CloseSheet);
    }
}

/// Forgets what a sheet held, once none is open, so the next one opens fresh.
pub(super) fn forget(ui: &Ui) {
    ui.data_mut(|d| d.remove::<Editing>(sheet_id()));
}

/// The Import Options sheet: units, up axis, what the import makes and whether coplanar faces
/// merge, for the file at `path`.
fn import_sheet(
    ui: &mut Ui,
    path: &Path,
    options: &mut ImportOptions,
    actions: &mut Vec<UiAction>,
) {
    let t = theme::tokens(ui.ctx().theme());
    widgets::section_header(ui, "Import Options");
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    ui.label(RichText::new(name).color(t.text_secondary));
    ui.add_space(theme::GRID);
    egui::Grid::new("import-options")
        .num_columns(2)
        .spacing([theme::GRID * 2.0, theme::GRID])
        .show(ui, |ui| {
            let label = ui.label("Units");
            egui::ComboBox::from_id_salt("import-units")
                .selected_text(options.units.label())
                .show_ui(ui, |ui| {
                    for units in ImportUnits::ALL {
                        ui.selectable_value(&mut options.units, units, units.label());
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();

            ui.label("Up axis");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut options.up, None, "Automatic");
                    ui.radio_value(&mut options.up, Some(UpAxis::Y), "Y up");
                    ui.radio_value(&mut options.up, Some(UpAxis::Z), "Z up");
                });
                ui.label(
                    RichText::new("Automatic: Y for OBJ, Z for STL")
                        .small()
                        .color(t.text_secondary),
                );
            });
            ui.end_row();

            ui.label("Import as");
            ui.vertical(|ui| {
                ui.radio_value(&mut options.place, ImportAs::Group, "Group");
                ui.radio_value(&mut options.place, ImportAs::Component, "Component");
                ui.radio_value(&mut options.place, ImportAs::Context, "Loose geometry");
            });
            ui.end_row();
        });
    ui.checkbox(&mut options.merge_coplanar, "Merge coplanar faces");
    if buttons(ui, "Import", true, actions) {
        actions.push(UiAction::ConfirmImport(*options));
    }
}

/// The sheet's buttons at the right: `confirm` (enabled when `can_confirm`) rightmost, Cancel
/// beside it. Cancel reports `CloseSheet`. True when `confirm` was clicked.
fn buttons(ui: &mut Ui, confirm: &str, can_confirm: bool, actions: &mut Vec<UiAction>) -> bool {
    ui.add_space(theme::GRID * 2.0);
    let mut confirmed = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            confirmed = ui
                .add_enabled(can_confirm, egui::Button::new(confirm))
                .clicked();
            if ui.button("Cancel").clicked() {
                actions.push(UiAction::CloseSheet);
            }
        });
    });
    confirmed
}
