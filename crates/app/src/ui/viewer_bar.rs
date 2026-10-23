//! The phone viewer's window: a small top bar (the model's name, Parts, Measure, Recent) over
//! the 3D view, with lists and cards laid over the view's bottom edge: the card of the selected
//! part (Isolate, Show all), the parts list, the Recent list and, while measuring, a strip with
//! the reading and Done. A line under the bar shows a message (a model that would not open).
//! No menus, tools, dock, status line, start card or orientation gizmo; the viewer cannot edit
//! (`Features::read_only`). Every row and button is at least `TOUCH_ROW` points tall.

use egui::{
    Align, Align2, Area, Button, FontId, Frame, Id, Label, Layout, Margin, Order, Panel, RichText,
    ScrollArea, Sense, Ui, WidgetInfo, WidgetType, vec2,
};
use session::Units;

use super::{UiAction, View, sheets, theme, viewport};
use crate::editor::PartCard;
use crate::platform::RecentShare;
use crate::viewer::parts::{PartRow, size_text};

/// The top bar's height in points: tall enough for a finger.
pub const BAR_HEIGHT: f32 = 44.0;

/// The least height of a row or button in a list or card, in points.
pub const TOUCH_ROW: f32 = 44.0;

/// The text size of the viewer's lists and cards, in points.
const TEXT: f32 = 15.0;

/// The part of the screen the lists may take, so the model stays in sight above them.
const LIST_SHARE: f32 = 0.55;

/// A list laid over the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerPanel {
    Parts,
    Recent,
}

/// What the viewer's bar, lists and card show.
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewerView<'a> {
    /// The list open over the view.
    pub panel: Option<ViewerPanel>,
    /// The parts list's rows (when it is open).
    pub rows: &'a [PartRow],
    /// The document's units, for the sizes.
    pub units: Units,
    /// The card of the selected part or parts.
    pub card: Option<&'a PartCard>,
    /// Whether some parts are hidden by Isolate.
    pub isolated: bool,
    /// While Measure is on, what its strip says: what to tap, or the distance.
    pub measure: Option<&'a str>,
    /// The kept models (when the Recent list is open).
    pub recent: &'a [RecentShare],
    /// Why the open model is not kept, when it is not.
    pub recent_note: Option<&'a str>,
    /// Whether the open model can be kept on this phone (offered, not kept yet).
    pub can_keep: bool,
}

/// Lays out the viewer's panels, appending the actions asked for to `actions`. Returns the
/// rectangle left for the 3D view, in points.
pub(super) fn draw(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) -> egui::Rect {
    let t = theme::tokens(ui.ctx().theme());
    let v = &view.viewer;
    Panel::top("viewer-bar")
        .resizable(false)
        .exact_size(BAR_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(t.bg_panel)
                .inner_margin(egui::Margin::symmetric(12, 0)),
        )
        .show(ui, |ui| {
            // Right to left: the buttons first, the name takes what is left.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let toggle = |ui: &mut Ui, text: &str, on: bool| {
                    ui.add(
                        Button::new(RichText::new(text).size(TEXT))
                            .selected(on)
                            .min_size(vec2(TOUCH_ROW, TOUCH_ROW)),
                    )
                    .clicked()
                };
                if toggle(ui, "Recent", v.panel == Some(ViewerPanel::Recent)) {
                    actions.push(UiAction::ViewerPanel(
                        (v.panel != Some(ViewerPanel::Recent)).then_some(ViewerPanel::Recent),
                    ));
                }
                if toggle(ui, "Measure", v.measure.is_some()) {
                    actions.push(UiAction::MeasureMode(v.measure.is_none()));
                }
                if toggle(ui, "Parts", v.panel == Some(ViewerPanel::Parts)) {
                    actions.push(UiAction::ViewerPanel(
                        (v.panel != Some(ViewerPanel::Parts)).then_some(ViewerPanel::Parts),
                    ));
                }
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

    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let list_height = screen.height() * LIST_SHARE;
    if let Some(reading) = v.measure {
        measure_strip(&ctx, central, reading, actions);
    }
    match v.panel {
        Some(ViewerPanel::Parts) => parts_list(&ctx, v, list_height, actions),
        Some(ViewerPanel::Recent) => recent_list(&ctx, v, list_height, actions),
        None => {
            if v.measure.is_none() {
                card(&ctx, v, actions);
            }
        }
    }

    match view.sheet {
        Some(sheet) => sheets::sheet(ui, sheet, view.units, actions),
        None => sheets::forget(ui),
    }
    central
}

/// A white-on-panel frame for what lies over the view.
fn overlay_frame(ctx: &egui::Context, top_round: bool) -> Frame {
    let t = theme::tokens(ctx.theme());
    let r: u8 = if top_round { 12 } else { 8 };
    Frame::new()
        .fill(t.bg_panel)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(egui::CornerRadius {
            nw: r,
            ne: r,
            sw: 0,
            se: 0,
        })
        .inner_margin(Margin::symmetric(12, 8))
}

/// An area at the bottom edge of the screen, the screen's full width.
fn bottom_area(ctx: &egui::Context, id: &str, content: impl FnOnce(&mut Ui)) {
    let width = ctx.content_rect().width();
    Area::new(Id::new(id))
        .order(Order::Foreground)
        .anchor(Align2::LEFT_BOTTOM, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            overlay_frame(ctx, true).show(ui, |ui| {
                ui.set_width(width - 24.0);
                content(ui);
            });
        });
}

/// A button as tall as a finger needs.
fn big_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(Button::new(RichText::new(text).size(TEXT)).min_size(vec2(88.0, TOUCH_ROW)))
}

/// A list's heading: its title and Close.
fn heading(ui: &mut Ui, title: &str, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        ui.set_min_height(TOUCH_ROW);
        ui.add(Label::new(RichText::new(title).size(TEXT + 2.0).strong()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if big_button(ui, "Close").clicked() {
                actions.push(UiAction::ViewerPanel(None));
            }
        });
    });
}

/// A row of a list: `title` and `detail` (right) painted in a rectangle `width` wide, at
/// least `TOUCH_ROW` tall. The row is one button for the screen reader, named by `label`.
fn row(ui: &mut Ui, width: f32, title: &str, detail: &str, label: &str) -> egui::Response {
    let t = theme::tokens(ui.ctx().theme());
    let (rect, response) = ui.allocate_exact_size(vec2(width, TOUCH_ROW + 4.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    let fill = if response.is_pointer_button_down_on() {
        t.bg_pressed
    } else if response.hovered() {
        t.bg_hover
    } else {
        t.bg_raised
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, fill);
    let inner = rect.shrink2(vec2(12.0, 0.0));
    let detail_galley = painter.layout_no_wrap(
        detail.to_string(),
        FontId::proportional(TEXT - 2.0),
        t.text_secondary,
    );
    let detail_width = detail_galley.size().x;
    painter.galley(
        egui::pos2(
            inner.right() - detail_width,
            inner.center().y - detail_galley.size().y / 2.0,
        ),
        detail_galley,
        t.text_secondary,
    );
    // The title is cut short of the detail.
    let room = (inner.width() - detail_width - 12.0).max(40.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_string(),
        FontId::proportional(TEXT),
        t.text_primary,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(inner.left(), inner.center().y - galley.size().y / 2.0),
        galley,
        t.text_primary,
    );
    response
}

/// The parts list: every part with its size, a component's instances as one row with their
/// count. A tap selects the row's parts and frames them.
fn parts_list(ctx: &egui::Context, v: &ViewerView, height: f32, actions: &mut Vec<UiAction>) {
    bottom_area(ctx, "viewer-parts", |ui| {
        heading(ui, &format!("Parts ({})", v.rows.len()), actions);
        if v.rows.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("This model has no groups or components.").size(TEXT));
            ui.add_space(8.0);
            return;
        }
        ScrollArea::vertical().max_height(height).show(ui, |ui| {
            let width = ui.available_width();
            for r in v.rows {
                let title = if r.count > 1 {
                    format!("{}  x{}", r.name, r.count)
                } else {
                    r.name.clone()
                };
                let size = r.size.map_or_else(String::new, |s| size_text(s, &v.units));
                let label = if size.is_empty() {
                    title.clone()
                } else {
                    format!("{title}, {size}")
                };
                if row(ui, width, &title, &size, &label).clicked() {
                    actions.push(UiAction::SelectParts(r.ids.clone()));
                }
                ui.add_space(4.0);
            }
        });
    });
}

/// "Keep on this phone" for the open model, with what it means.
fn keep_row(ui: &mut Ui, actions: &mut Vec<UiAction>) {
    ui.add(
        Label::new(
            RichText::new(
                "A kept model, with its key, can be read by pages served at this address.",
            )
            .size(TEXT - 2.0),
        )
        .wrap(),
    );
    if big_button(ui, "Keep on this phone").clicked() {
        actions.push(UiAction::KeepModel);
    }
}

/// The Recent list: the models kept on this phone, each opened with a tap and removed with
/// Delete.
fn recent_list(ctx: &egui::Context, v: &ViewerView, height: f32, actions: &mut Vec<UiAction>) {
    bottom_area(ctx, "viewer-recent", |ui| {
        heading(ui, "Recent", actions);
        if v.can_keep {
            keep_row(ui, actions);
            ui.add_space(4.0);
        }
        if let Some(note) = v.recent_note {
            ui.add(Label::new(RichText::new(note).size(TEXT)).wrap());
            ui.add_space(4.0);
        }
        if v.recent.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("Nothing kept on this phone yet.").size(TEXT));
            ui.add_space(8.0);
            return;
        }
        ScrollArea::vertical().max_height(height).show(ui, |ui| {
            let delete_width = 88.0;
            let gap = ui.spacing().item_spacing.x;
            let width = ui.available_width() - delete_width - gap;
            for r in v.recent {
                ui.horizontal(|ui| {
                    let detail = format!("{}  {}", r.when, r.size);
                    let label = format!("{}, {}", r.name, detail);
                    if row(ui, width, &r.name, &detail, &label).clicked() {
                        actions.push(UiAction::OpenKept(r.id.clone()));
                    }
                    if ui
                        .add(
                            Button::new(RichText::new("Delete").size(TEXT))
                                .min_size(vec2(delete_width, TOUCH_ROW + 4.0)),
                        )
                        .on_hover_text("Remove from this phone")
                        .clicked()
                    {
                        actions.push(UiAction::DeleteKept(r.id.clone()));
                    }
                });
                ui.add_space(4.0);
            }
        });
    });
}

/// The selected part's card (name, size, "1 of N", Isolate or Show all), or when nothing is
/// selected but parts are hidden, a bar with Show all.
fn card(ctx: &egui::Context, v: &ViewerView, actions: &mut Vec<UiAction>) {
    if v.card.is_none() && !v.isolated && !v.can_keep {
        return;
    }
    bottom_area(ctx, "viewer-card", |ui| {
        if let Some(card) = v.card {
            ui.add(Label::new(RichText::new(&card.title).size(TEXT + 3.0).strong()).truncate());
            if let Some(size) = &card.size {
                ui.label(RichText::new(size).size(TEXT));
            }
            if let Some(ordinal) = &card.ordinal {
                ui.label(RichText::new(ordinal).size(TEXT));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if card.isolated {
                    if big_button(ui, "Show all").clicked() {
                        actions.push(UiAction::ShowAll);
                    }
                } else if big_button(ui, "Isolate").clicked() {
                    actions.push(UiAction::Isolate);
                }
            });
        } else if v.isolated {
            ui.horizontal(|ui| {
                ui.set_min_height(TOUCH_ROW);
                ui.label(RichText::new("Other parts are hidden").size(TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if big_button(ui, "Show all").clicked() {
                        actions.push(UiAction::ShowAll);
                    }
                });
            });
        }
        if v.can_keep {
            ui.add_space(6.0);
            keep_row(ui, actions);
        }
    });
}

/// Measure's strip under the bar: what to tap, or the distance, and Done.
fn measure_strip(
    ctx: &egui::Context,
    central: egui::Rect,
    reading: &str,
    actions: &mut Vec<UiAction>,
) {
    let width = ctx.content_rect().width();
    Area::new(Id::new("viewer-measure-strip"))
        .order(Order::Foreground)
        .fixed_pos(central.left_top() + vec2(8.0, 8.0))
        .show(ctx, |ui| {
            overlay_frame(ctx, false).show(ui, |ui| {
                ui.set_width(width - 40.0);
                ui.horizontal(|ui| {
                    ui.set_min_height(TOUCH_ROW);
                    ui.add(Label::new(RichText::new(reading).size(TEXT + 3.0).strong()).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if big_button(ui, "Done").clicked() {
                            actions.push(UiAction::MeasureMode(false));
                        }
                    });
                });
            });
        });
}
