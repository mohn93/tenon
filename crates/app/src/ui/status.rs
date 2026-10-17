//! The status bar: the tool chip, the prompt, the lock pill and the message on the left; the
//! agents badge, the selection count and the measurements box on the right.

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Pos2, RichText, Sense, Stroke,
    StrokeKind, TextStyle, Ui, Vec2, WidgetInfo, WidgetType,
};
use tools::ToolKind;

use super::{UiAction, View, theme};

pub(super) fn status_bar(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let t = theme::tokens(ui.ctx().theme());
    ui.horizontal_centered(|ui| {
        // The right-hand group first, right to left, so the measurements box always shows;
        // the left-hand texts take the room left over.
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
            if !view.selection.is_empty() {
                ui.label(format!("{} selected", view.selection.len()));
            }
            if view.tool != ToolKind::Select && !view.flying {
                measurements(ui, view);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let name = format!("Tool: {}", view.status.tool);
                pill(ui, view.status.tool, t.bg_hover, t.text_primary, &name);
                ui.add(
                    egui::Label::new(RichText::new(&view.status.prompt).color(t.text_secondary))
                        .truncate(),
                );
                if let Some(lock) = view.status.lock {
                    let color = lock_color(lock, t.accent);
                    let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 51);
                    pill(ui, lock, fill, color, lock);
                }
                if let Some(message) = &view.status.message {
                    ui.add(egui::Label::new(RichText::new(message).color(t.warn)).truncate());
                }
            });
        });
    });
}

/// The color of a lock: its axis's for an axis lock, otherwise `accent`.
pub(super) fn lock_color(lock: &str, accent: Color32) -> Color32 {
    if lock.contains("red axis") {
        theme::AXIS_X
    } else if lock.contains("green axis") {
        theme::AXIS_Y
    } else if lock.contains("blue axis") {
        theme::AXIS_Z
    } else {
        accent
    }
}

/// Whether the measurements box's value is being typed (it ends in "_", as `Status::value`
/// documents).
pub(super) fn typing(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.ends_with('_'))
}

/// `text` in `color` on a small rounded `fill`, named `name` (one AccessKit node, so a chip
/// reading "Line" does not share the rail button's name).
fn pill(ui: &mut Ui, text: &str, fill: Color32, color: Color32, name: &str) {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_string(), TextStyle::Body.resolve(ui.style()), color);
    let padding = Vec2::new(6.0, 2.0);
    let (rect, response) = ui.allocate_exact_size(galley.size() + 2.0 * padding, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::SWATCH_RADIUS), fill);
    ui.painter().galley(rect.min + padding, galley, color);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
}

/// The measurements box: its label on the left in small secondary text, the typed text or the
/// tool's live measurement on the right in the value font, ringed in the accent while typing.
/// Named by its label ("Measurements" when the tool gives none).
fn measurements(ui: &mut Ui, view: &View) {
    let t = theme::tokens(ui.ctx().theme());
    let label = match view.status.label {
        "" => "Measurements",
        label => label,
    };
    let value = view.status.value.as_deref();
    let size = Vec2::new(theme::MEASUREMENTS_WIDTH, theme::MEASUREMENTS_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::CONTROL_RADIUS);
    painter.rect_filled(rect, radius, t.bg_raised);
    if typing(value) {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(theme::FOCUS_RING, t.accent),
            StrokeKind::Outside,
        );
    }
    let y = rect.center().y;
    painter.text(
        Pos2::new(rect.left() + 8.0, y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(theme::SMALL_SIZE),
        t.text_secondary,
    );
    painter.text(
        Pos2::new(rect.right() - 8.0, y),
        Align2::RIGHT_CENTER,
        value.unwrap_or_default(),
        TextStyle::Monospace.resolve(ui.style()),
        t.text_primary,
    );
    response.widget_info(|| {
        // A text input's role: AccessKit gives a label's name as its value, which the value
        // would then replace.
        let mut info = WidgetInfo::labeled(WidgetType::TextEdit, true, label);
        info.current_text_value = value.map(str::to_string);
        info
    });
}
