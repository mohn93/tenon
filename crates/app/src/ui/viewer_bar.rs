//! The phone viewer's window: a small top bar with the model's name over the 3D view, and a
//! line for a message (a model that would not open) under it. No menus, tools, dock, status
//! line, start card or orientation gizmo; the viewer cannot edit (`Features::read_only`).

use egui::{Align, Label, Layout, Panel, RichText, Ui};

use super::{UiAction, View, sheets, theme, viewport};

/// The top bar's height in points: tall enough for a finger.
pub const BAR_HEIGHT: f32 = 44.0;

/// Lays out the viewer's panels, appending the actions asked for to `actions`. Returns the
/// rectangle left for the 3D view, in points.
pub(super) fn draw(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) -> egui::Rect {
    let t = theme::tokens(ui.ctx().theme());
    Panel::top("viewer-bar")
        .resizable(false)
        .exact_size(BAR_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::symmetric(12, 0)),
        )
        .show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(Label::new(RichText::new(view.model_name).strong()).truncate());
            });
        });
    if let Some(message) = &view.status.message {
        Panel::top("viewer-message")
            .resizable(false)
            .exact_size(theme::STATUS_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(t.bg_panel)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ui, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(Label::new(RichText::new(message).color(t.warn)).truncate());
                });
            });
    }
    let central = ui.available_rect_before_wrap();
    viewport::hud(ui, central, view.hud);
    viewport::set_cursor(ui.ctx(), central, view.cursor);
    match view.sheet {
        Some(sheet) => sheets::sheet(ui, sheet, actions),
        None => sheets::forget(ui),
    }
    central
}
