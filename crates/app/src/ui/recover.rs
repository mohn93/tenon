//! The Recover Models window: the models a window that ended without saving (or an agent's
//! discard) left in the recovery folder, each with Open and Discard, and Discard All. What it
//! reports goes through `UiAction`; Discard and Discard All ask first, in a sheet.

use egui::{RichText, Ui};

use super::{RecoverRow, UiAction, widgets};

/// The window's title.
pub const TITLE: &str = "Recover Models";

/// The Recover Models window, centred over the window when it first opens.
pub(super) fn recover_window(ui: &mut Ui, rows: &[RecoverRow], actions: &mut Vec<UiAction>) {
    let mut open = true;
    let ctx = ui.ctx().clone();
    let room = (ctx.content_rect().height() - 160.0).max(120.0);
    let shown = egui::Window::new(TITLE)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .constrain(true)
        .show(&ctx, |ui| {
            if rows.is_empty() {
                ui.label(RichText::new("No models to recover").weak());
                return;
            }
            ui.label("These models were not saved. Open one to keep working on it.");
            ui.separator();
            let list = egui::ScrollArea::vertical()
                .max_height(room)
                .show(ui, |ui| {
                    for (k, row) in rows.iter().enumerate() {
                        if k > 0 {
                            ui.separator();
                        }
                        recover_row(ui, row, actions);
                    }
                });
            widgets::name_scroll_bars(ui.ctx(), list.id, "Recovered models");
            ui.separator();
            if ui.button("Discard All").clicked() {
                actions.push(UiAction::DiscardRecovered(None));
            }
        });
    widgets::name_title_bar(&ctx, &shown, TITLE);
    if !open {
        actions.push(UiAction::ShowRecover(false));
    }
}

/// One model: its name, where it came from, when it was saved, and its buttons (Discard only
/// when it cannot be opened, with why).
fn recover_row(ui: &mut Ui, row: &RecoverRow, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(&row.name).strong());
            ui.label(
                row.original
                    .as_ref()
                    .map_or_else(|| "Untitled".into(), |p| p.display().to_string()),
            );
            let what = if row.discarded {
                "Discarded by an agent"
            } else {
                "Autosaved"
            };
            ui.label(RichText::new(format!("{what} {}", row.saved)).weak());
            if let Some(error) = &row.error {
                ui.label(RichText::new(error).color(ui.visuals().warn_fg_color));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Discard").clicked() {
                actions.push(UiAction::DiscardRecovered(Some(row.model.clone())));
            }
            if row.error.is_none() && ui.button("Open").clicked() {
                actions.push(UiAction::OpenRecovered(row.model.clone()));
            }
        });
    });
}
