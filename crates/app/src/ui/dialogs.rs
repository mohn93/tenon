//! The Agent Activity and Soften Edges windows.

use egui::{RichText, Ui};

use super::{Agents, UiAction};

/// The Agent Activity window: how many agents are connected, the recovery files their
/// discards left, and what they asked for, newest first.
pub(super) fn agent_activity(
    ui: &mut Ui,
    agents: &Agents,
    log: &[String],
    actions: &mut Vec<UiAction>,
) {
    let mut open = true;
    egui::Window::new("Agent Activity")
        .open(&mut open)
        .collapsible(false)
        .default_width(420.0)
        .show(ui.ctx(), |ui| {
            ui.label(match agents.connected {
                0 => "No agents connected".to_string(),
                1 => "1 agent connected".to_string(),
                n => format!("{n} agents connected"),
            });
            if !agents.recovered.is_empty() {
                ui.label(
                    RichText::new("Unsaved changes agents discarded were kept in:")
                        .color(ui.visuals().warn_fg_color),
                );
                for path in agents.recovered {
                    ui.monospace(path.display().to_string());
                }
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    if log.is_empty() {
                        ui.label(RichText::new("Nothing yet").weak());
                    }
                    for line in log {
                        ui.monospace(line);
                    }
                });
        });
    if !open {
        actions.push(UiAction::ShowAgentActivity(false));
    }
}

/// The Soften Edges window: an angle and the Soften and Harden buttons, which act on the
/// selection, or on the open context when nothing is selected.
pub(super) fn soften_window(ui: &mut Ui, angle: u8, actions: &mut Vec<UiAction>) {
    let mut open = true;
    egui::Window::new("Soften Edges")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ui.ctx().content_rect().center())
        .show(ui.ctx(), |ui| {
            let mut value = angle;
            ui.horizontal(|ui| {
                ui.label("Angle");
                if ui
                    .add(egui::Slider::new(&mut value, 0..=180).suffix("°"))
                    .changed()
                {
                    actions.push(UiAction::SetSoftenAngle(value));
                }
            });
            ui.label("Edges between faces meeting at up to this angle");
            ui.horizontal(|ui| {
                if ui.button("Soften").clicked() {
                    actions.push(UiAction::Soften);
                }
                if ui.button("Harden").clicked() {
                    actions.push(UiAction::Harden);
                }
            });
            ui.label(RichText::new("Selection, or everything open when nothing is").weak());
        });
    if !open {
        actions.push(UiAction::ShowSoften(false));
    }
}
