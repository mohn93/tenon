//! The widget kit: the controls the panels are built from, drawn from the theme's tokens and
//! named for AccessKit.

use std::ops::RangeInclusive;

use egui::accesskit::Role;
use egui::{
    Color32, CornerRadius, Margin, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
    WidgetInfo, WidgetType,
};
use tools::ToolKind;

use super::{fonts, icons, theme};
use crate::shortcuts::Bound;

/// A square button showing `icon`, named `label`; its tooltip is `tooltip` and the key bound
/// to `bound`, also while disabled (then the icon takes the disabled text color).
pub fn icon_button(
    ui: &mut Ui,
    icon: &str,
    label: &str,
    tooltip: &str,
    bound: Option<Bound>,
    enabled: bool,
) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    let mut glyph = RichText::new(icon)
        .family(fonts::icons())
        .size(theme::ICON_SIZE);
    if !enabled {
        glyph = glyph.color(t.text_disabled);
    }
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(glyph).min_size(Vec2::splat(theme::CONTROL_HEIGHT)),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    response
        .on_hover_ui(|ui| hint(ui, tooltip, bound))
        .on_disabled_hover_ui(|ui| hint(ui, tooltip, bound))
}

/// A tool's button on the rail: its glyph in a `RAIL_BUTTON` square, on an accent fill while
/// it is the active tool (otherwise framed only under the pointer, as a selectable label is).
/// Named for the tool; the tooltip names the tool and its key.
pub fn tool_button(ui: &mut Ui, kind: ToolKind, selected: bool) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    let mut glyph = RichText::new(icons::tool(kind))
        .family(fonts::icons())
        .size(theme::RAIL_ICON);
    if selected {
        glyph = glyph.color(t.on_accent);
    }
    // The glyph and the padding around it add up to the button's side.
    let line = ui.fonts_mut(|f| f.row_height(&egui::FontId::new(theme::RAIL_ICON, fonts::icons())));
    let padding = Vec2::splat(((theme::RAIL_BUTTON - line) / 2.0).max(0.0));
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = padding;
            // `selectable` reports the selected state to AccessKit and frames the button only
            // when selected or under the pointer.
            let mut button =
                egui::Button::selectable(selected, glyph).min_size(Vec2::splat(theme::RAIL_BUTTON));
            if selected {
                button = button.fill(t.accent);
            }
            ui.add(button)
        })
        .inner;
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, kind.label()));
    response.on_hover_ui(|ui| hint(ui, kind.label(), Some(Bound::Tool(kind))))
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

/// A section's title: small, uppercase, muted, with a line under it. Named as written.
pub fn section_header(ui: &mut Ui, title: &str) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    ui.add_space(theme::SECTION_GAP);
    let font = egui::FontId::new(theme::SECTION_SIZE, fonts::semibold());
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_uppercase(), font, t.text_secondary);
    let height = galley.size().y + theme::GRID;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    ui.painter()
        .galley(rect.left_top(), galley, t.text_secondary);
    ui.painter()
        .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, t.border));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, title));
    response
}

/// What a `numeric_row` frame gave.
pub struct NumericRow<const N: usize> {
    /// When a drag ended or a typed value was entered: the values shown when the edit began,
    /// and the values now.
    pub done: Option<([f64; N], [f64; N])>,
    /// Whether a field is being dragged or typed in.
    pub editing: bool,
}

/// A row of `label` and one field per value, changing by `speed` per point dragged; with more
/// than one field the label goes on a line of its own, so long values still fit the panel. While a
/// field is dragged or typed in, the values shown when the edit began, and those being dragged,
/// live in egui's memory under `key`, so a change of what is shown meanwhile does not leak in.
pub fn numeric_row<const N: usize>(
    ui: &mut Ui,
    key: egui::Id,
    label: &str,
    shown: [f64; N],
    range: RangeInclusive<f64>,
    speed: f64,
) -> NumericRow<N> {
    let stored = ui.data(|d| d.get_temp::<([f64; N], [f64; N])>(key));
    let (start, mut values) = stored.unwrap_or((shown, shown));
    let label_id = if N > 1 {
        Some(ui.label(label).id)
    } else {
        None
    };
    let (editing, ended) = ui
        .horizontal(|ui| {
            let label_id = label_id.unwrap_or_else(|| ui.label(label).id);
            let (mut editing, mut ended) = (false, false);
            for value in &mut values {
                let field = ui
                    .add(
                        egui::DragValue::new(value)
                            .range(range.clone())
                            .speed(speed)
                            .max_decimals(3)
                            .update_while_editing(false),
                    )
                    .labelled_by(label_id);
                // A field being typed in reports a change per keystroke; its value comes
                // when it loses focus.
                editing |= field.dragged() || field.has_focus();
                ended |= field.drag_stopped() || field.changed();
            }
            (editing, ended)
        })
        .inner;
    if editing {
        ui.data_mut(|d| d.insert_temp(key, (start, values)));
        return NumericRow {
            done: None,
            editing: true,
        };
    }
    ui.data_mut(|d| d.remove::<([f64; N], [f64; N])>(key));
    NumericRow {
        done: (ended || stored.is_some()).then_some((start, values)),
        editing: false,
    }
}

/// Takes what `numeric_row` keeps under `key` while its edit is in progress: the values shown
/// when it began and the values now.
pub fn take_row<const N: usize>(ui: &mut Ui, key: egui::Id) -> Option<([f64; N], [f64; N])> {
    let row = ui.data(|d| d.get_temp::<([f64; N], [f64; N])>(key));
    ui.data_mut(|d| d.remove::<([f64; N], [f64; N])>(key));
    row
}

/// What a swatch shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Color(Color32),
    Image(egui::TextureId),
}

/// A material's tile in the Materials grid: a `TILE` square of its color or image, ringed in
/// the accent when `chosen`, with its name under it in small secondary text, cut short with
/// "…" to the tile's width. Clicked or double-clicked; named `name`.
pub fn material_tile(ui: &mut Ui, fill: Fill, name: &str, chosen: bool) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    let mut job = egui::text::LayoutJob::single_section(
        name.to_owned(),
        egui::TextFormat::simple(
            egui::FontId::proportional(theme::SMALL_SIZE),
            t.text_secondary,
        ),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: theme::TILE,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let caption = ui.fonts_mut(|f| f.layout_job(job));
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(theme::TILE, theme::TILE + caption.size().y),
        Sense::click(),
    );
    let square = egui::Rect::from_min_size(rect.min, Vec2::splat(theme::TILE));
    let radius = CornerRadius::same(theme::SWATCH_RADIUS);
    match fill {
        Fill::Color(color) => {
            ui.painter().rect_filled(square, radius, color);
        }
        Fill::Image(image) => egui::Image::new((image, square.size()))
            .corner_radius(radius)
            .paint_at(ui, square),
    }
    ui.painter().rect_stroke(
        square,
        radius,
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    if chosen {
        ui.painter().rect_stroke(
            square,
            radius,
            Stroke::new(theme::FOCUS_RING, t.accent),
            StrokeKind::Outside,
        );
    }
    ui.painter()
        .galley(square.left_bottom(), caption, t.text_secondary);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, chosen, name));
    response
}

/// A small square of a color or an image that can be clicked, ringed in the accent when
/// `selected`. Named `label`.
pub fn swatch(ui: &mut Ui, fill: Fill, label: &str, selected: bool) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    let radius = CornerRadius::same(theme::SWATCH_RADIUS);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(theme::SWATCH_SIZE), Sense::click());
    match fill {
        Fill::Color(color) => {
            ui.painter().rect_filled(rect, radius, color);
        }
        Fill::Image(image) => egui::Image::new((image, rect.size()))
            .corner_radius(radius)
            .paint_at(ui, rect),
    }
    ui.painter()
        .rect_stroke(rect, radius, Stroke::new(1.0, t.border), StrokeKind::Inside);
    if selected {
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(theme::FOCUS_RING, t.accent),
            StrokeKind::Outside,
        );
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, label));
    response
}

/// Names a widget egui draws without an AccessKit name: a window's title bar, a panel's resize
/// edge, a menu's area, a scroll bar. Does nothing unless the widget was laid out this pass or
/// the last (`accesskit_node_builder` would otherwise add an empty node), and makes no name
/// while AccessKit is off.
pub fn name_egui_widget(
    ctx: &egui::Context,
    id: egui::Id,
    role: Role,
    name: impl FnOnce() -> String,
) {
    if ctx.read_response(id).is_none() {
        return;
    }
    ctx.accesskit_node_builder(id, |node| {
        node.set_role(role);
        node.set_label(name());
    });
}

/// Names the title bar of the window `shown` (what `Window::show` returned) "`title` title
/// bar" (the window itself is named `title`). egui senses clicks and drags on it without naming
/// it.
pub fn name_title_bar<R>(ctx: &egui::Context, shown: &Option<egui::InnerResponse<R>>, title: &str) {
    if let Some(shown) = shown {
        // egui 0.36's id for it (`Window::show_dyn`): the window's area id with "__title_click".
        let id = shown.response.layer_id.id.with("__title_click");
        name_egui_widget(ctx, id, Role::TitleBar, || format!("{title} title bar"));
    }
}

/// Names the scroll bars of the scroll area `id` (`ScrollAreaOutput::id`) "Scroll `what`".
/// egui gives them a role but no name.
pub fn name_scroll_bars(ctx: &egui::Context, id: egui::Id, what: &str) {
    // egui 0.36's ids for them (`ScrollArea::show_viewport_dyn`): the area's id with the axis.
    for axis in 0..2usize {
        name_egui_widget(ctx, id.with(axis), Role::ScrollBar, || {
            format!("Scroll {what}")
        });
    }
}

/// A slider for `value` in `range` with its value box after it, the slider labelled by the
/// label widget `label` and the box named "`name` value". egui's slider draws its own box
/// without a name, so the slider is drawn without one and the box here; `slider` and
/// `value_box` set their formats. Returns the two joined, as egui's slider does.
pub fn slider_row<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: egui::Id,
    name: &str,
    value: &mut T,
    range: RangeInclusive<T>,
    slider: impl for<'v> FnOnce(egui::Slider<'v>) -> egui::Slider<'v>,
    value_box: impl for<'v> FnOnce(egui::DragValue<'v>) -> egui::DragValue<'v>,
) -> Response {
    let track = ui
        .add(slider(
            egui::Slider::new(value, range.clone()).show_value(false),
        ))
        .labelled_by(label);
    // Dragging the box moves the value at the slider's rate: its range across its width.
    let span = (range.end().to_f64() - range.start().to_f64()).abs();
    let speed = span / f64::from(track.rect.width().max(1.0));
    let shown = ui.add(value_box(
        egui::DragValue::new(value)
            .range(range)
            .clamp_existing_to_range(true)
            .speed(speed),
    ));
    ui.ctx()
        .accesskit_node_builder(shown.id, |node| node.set_label(format!("{name} value")));
    track.union(shown)
}
