//! The status bar: prompt, lock and message, the agents button and the measurements box.

use egui::{Align, Layout, RichText, Ui};

use super::{UiAction, View};

pub(super) fn status_bar(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        ui.label(&view.status.prompt);
        if let Some(lock) = view.status.lock {
            ui.label(RichText::new(lock).strong());
        }
        if let Some(message) = &view.status.message {
            ui.label(RichText::new(message).color(ui.visuals().warn_fg_color));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if view.agents.connected > 0 {
                let text = format!("Agents: {}", view.agents.connected);
                if ui
                    .add(egui::Button::new(RichText::new(text).strong()).small())
                    .on_hover_text("Programs connected to the agent API")
                    .clicked()
                {
                    actions.push(UiAction::ShowAgentActivity(true));
                }
            }
            // The measurements box: typed text, or the tool's live measurement.
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(120.0);
                ui.monospace(view.status.value.clone().unwrap_or_default());
            });
            ui.label(view.status.label);
        });
    });
}
