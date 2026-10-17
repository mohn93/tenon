//! The widget kit: the controls the panels are built from, drawn from the theme's tokens and
//! named for AccessKit.

use egui::{CornerRadius, Margin, Response, RichText, Ui, Vec2, WidgetInfo, WidgetType};
use tools::ToolKind;

use super::theme;
use crate::shortcuts::Bound;

/// A tool's button: its name, on an accent fill while it is the active tool (otherwise framed
/// only under the pointer, as a selectable label is). The tooltip names the tool and its key.
pub fn tool_button(ui: &mut Ui, kind: ToolKind, selected: bool) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    let mut text = RichText::new(kind.label());
    if selected {
        text = text.color(t.on_accent);
    }
    // `selectable` reports the selected state to AccessKit and frames the button only when
    // selected or under the pointer.
    let mut button =
        egui::Button::selectable(selected, text).min_size(Vec2::new(0.0, theme::CONTROL_HEIGHT));
    if selected {
        button = button.fill(t.accent);
    }
    ui.add(button)
        .on_hover_ui(|ui| hint(ui, kind.label(), Some(Bound::Tool(kind))))
}

/// A menu item showing the key bound to `bound` on its right; named `label` alone.
pub fn menu_item(ui: &mut Ui, label: &str, bound: Option<Bound>, enabled: bool) -> Response {
    let mut button = egui::Button::new(label);
    if let Some(key) = bound.and_then(Bound::key_text) {
        button = button.shortcut_text(key);
    }
    let response = ui.add_enabled(enabled, button);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    response
}

/// A tooltip's body: `label`, then the key bound to `bound` in a chip.
pub fn hint(ui: &mut Ui, label: &str, bound: Option<Bound>) {
    ui.horizontal(|ui| {
        ui.label(label);
        if let Some(key) = bound.and_then(Bound::key_text) {
            chip(ui, &key);
        }
    });
}

/// Small muted text on a raised, rounded background: a key in a tooltip.
pub fn chip(ui: &mut Ui, text: &str) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    egui::Frame::new()
        .fill(t.bg_hover)
        .corner_radius(CornerRadius::same(theme::SWATCH_RADIUS))
        .inner_margin(Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(theme::SMALL_SIZE)
                    .color(t.text_secondary),
            )
        })
        .response
}
