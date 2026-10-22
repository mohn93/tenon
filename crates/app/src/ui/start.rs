//! The card over an empty, unedited model: New, Open, the recent files and a hint for drawing
//! a first box. It has no animation and asks for no repaint of its own.

use std::path::Path;

use egui::{Align, Layout, Rect, RichText, Sense, Ui, Vec2, WidgetInfo, WidgetType};

use super::{StartCard, UiAction, fonts, icons, theme, widgets};

/// The hint at the card's foot.
pub const HINT: &str = "R: drag a rectangle.  P: pull it up into a box.";

/// The card in the bottom left of the view (`view_rect`), in its own `Area`, so presses on it
/// never reach the tools.
pub fn start_card(
    ui: &Ui,
    view_rect: Rect,
    card: &StartCard,
    show_recent: bool,
    actions: &mut Vec<UiAction>,
) {
    let t = theme::tokens(ui.ctx().theme());
    // The frame's padding and its 1 pt border lie inside the card's size.
    let inner = theme::START_CARD - Vec2::splat(2.0 * (theme::START_CARD_PADDING + 1.0));
    let min = view_rect.left_bottom()
        + Vec2::new(
            theme::START_CARD_INSET,
            -theme::START_CARD_INSET - theme::START_CARD.y,
        );
    egui::Area::new(egui::Id::new("start-card"))
        .order(egui::Order::Middle)
        .movable(false)
        .constrain(false)
        .fixed_pos(min)
        .show(ui.ctx(), |ui| {
            // Named for tests, before the contents: a later widget would win their clicks.
            let rect = Rect::from_min_size(min, theme::START_CARD);
            let named = ui.interact(rect, egui::Id::new("start-card-frame"), Sense::hover());
            named.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Start card"));
            let frame = egui::Frame::new()
                .fill(t.bg_raised)
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(egui::CornerRadius::same(theme::WINDOW_RADIUS))
                .inner_margin(egui::Margin::same(theme::START_CARD_PADDING as i8));
            frame.show(ui, |ui| {
                ui.set_min_size(inner);
                ui.set_max_size(inner);
                ui.spacing_mut().item_spacing.y = 2.0;
                contents(ui, card, show_recent, actions);
            });
        });
}

fn contents(ui: &mut Ui, card: &StartCard, show_recent: bool, actions: &mut Vec<UiAction>) {
    let t = theme::tokens(ui.ctx().theme());
    ui.horizontal(|ui| {
        if ui.button("New").clicked() {
            actions.push(UiAction::NewModel);
        }
        if ui.button("Open...").clicked() {
            actions.push(UiAction::OpenFile);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let dismiss = widgets::icon_button(
                ui,
                icons::X,
                "Dismiss",
                "Hide this card until the app is opened again",
                None,
                true,
            );
            if dismiss.clicked() {
                actions.push(UiAction::DismissStart);
            }
        });
    });
    ui.add_space(theme::GRID);
    if show_recent {
        ui.label(
            RichText::new("RECENT")
                .size(theme::SECTION_SIZE)
                .family(fonts::semibold())
                .color(t.text_secondary),
        );
        if card.recent.is_empty() {
            ui.label(RichText::new("No recent files").color(t.text_secondary));
        }
        ui.scope(|ui| {
            ui.spacing_mut().interact_size.y = theme::START_CARD_ROW;
            ui.spacing_mut().button_padding.y = 0.0;
            for path in card.recent {
                if recent_row(ui, path).clicked() {
                    actions.push(UiAction::OpenRecent(path.clone()));
                }
            }
        });
    }
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.label(
            RichText::new(HINT)
                .size(theme::SMALL_SIZE)
                .color(t.text_secondary),
        );
    });
}

/// A recent file's row: its name (cut short to fit one row), its full path in the tooltip.
fn recent_row(ui: &mut Ui, path: &Path) -> egui::Response {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    );
    ui.add(egui::Button::new(name).frame(false).truncate())
        .on_hover_text(path.display().to_string())
}
