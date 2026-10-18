//! The context bar: the menus, Undo and Redo, the breadcrumb of the open path, the group
//! buttons that apply, the day/night toggle and the agents indicator.

use egui::{Align, Button, Layout, RichText, Sense, TextStyle, Ui, Vec2, WidgetInfo, WidgetType};
use kernel::EntityId;
use session::Lighting;

use super::{UiAction, View, fonts, icons, menu_bar, outliner, theme, widgets};
use crate::shortcuts::Bound;

/// The longest crumb name drawn whole, in characters.
const CRUMB_CHARS: usize = 24;

pub(super) fn context_bar(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    ui.horizontal_centered(|ui| {
        if view.egui_menus {
            menu_bar::show(ui, &crate::menus::menu_bar(view, false), actions);
            ui.separator();
        }
        history(ui, view, actions);
        // The right-hand group first, right to left; the breadcrumb takes the room left over.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            agents_indicator(ui, view, actions);
            night_toggle(ui, view, actions);
            ui.separator();
            let buttons = [
                (
                    view.has_instance_selected,
                    icons::ARROWS_OUT,
                    "Explode",
                    None,
                    UiAction::Explode,
                ),
                (
                    view.has_selection,
                    icons::CUBE,
                    "Make Component",
                    Some(Bound::MakeComponent),
                    UiAction::MakeComponent,
                ),
                (
                    view.has_selection,
                    icons::BOUNDING_BOX,
                    "Make Group",
                    Some(Bound::MakeGroup),
                    UiAction::MakeGroup,
                ),
            ];
            for (applies, icon, label, bound, action) in buttons {
                if applies && widgets::icon_button(ui, icon, label, label, bound, true).clicked() {
                    actions.push(action);
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                breadcrumb(ui, view, actions);
            });
        });
    });
}

/// Undo and Redo, each naming the step it would take back or do again.
fn history(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let undo = view.undo.map_or("Nothing to undo".into(), |l| {
        format!("Undo {}", step_name(l))
    });
    if widgets::icon_button(
        ui,
        icons::UNDO,
        "Undo",
        &undo,
        Some(Bound::Undo),
        view.undo.is_some(),
    )
    .clicked()
    {
        actions.push(UiAction::Undo);
    }
    let redo = view.redo.map_or("Nothing to redo".into(), |l| {
        format!("Redo {}", step_name(l))
    });
    if widgets::icon_button(
        ui,
        icons::REDO,
        "Redo",
        &redo,
        Some(Bound::Redo),
        view.redo.is_some(),
    )
    .clicked()
    {
        actions.push(UiAction::Redo);
    }
}

/// A selectable icon button, on at night: the moon at night, the sun by day. A click asks for
/// the other lighting.
fn night_toggle(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let night = view.lighting == Lighting::Night;
    let glyph = if night { icons::MOON } else { icons::SUN };
    let response = ui.add(
        Button::selectable(
            night,
            RichText::new(glyph)
                .family(fonts::icons())
                .size(theme::ICON_SIZE),
        )
        .min_size(Vec2::splat(theme::CONTROL_HEIGHT)),
    );
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, true, night, "Night Lighting"));
    if response.on_hover_text("Night Lighting").clicked() {
        let other = if night {
            Lighting::Day
        } else {
            Lighting::Night
        };
        actions.push(UiAction::SetLighting(other));
    }
}

/// The robot glyph, and the number of agents connected in accent text when there are any. A
/// click opens Agent Activity.
fn agents_indicator(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let t = theme::tokens(ui.ctx().theme());
    let connected = view.agents.connected;
    let glyph = RichText::new(icons::ROBOT)
        .family(fonts::icons())
        .size(theme::ICON_SIZE);
    let button = if connected > 0 {
        let count = RichText::new(connected.to_string()).color(t.accent);
        Button::new((glyph, count))
    } else {
        Button::new(glyph)
    };
    let response = ui.add(button.min_size(Vec2::splat(theme::CONTROL_HEIGHT)));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Agent Activity"));
    let tip = match connected {
        0 => "No agents connected".to_string(),
        1 => "1 agent connected".to_string(),
        n => format!("{n} agents connected"),
    };
    if response.on_hover_text(tip).clicked() {
        actions.push(UiAction::ShowAgentActivity(true));
    }
}

/// A level of the open path: "Model", then each open instance.
pub(super) struct Crumb {
    pub name: String,
    /// The instances from the root down to this level (empty for Model).
    pub path: Vec<EntityId>,
}

/// The breadcrumb's crumbs for the open path, Model first. A level the Outliner does not list
/// is named "Group".
pub(super) fn crumbs(view: &View) -> Vec<Crumb> {
    let mut out = vec![Crumb {
        name: "Model".into(),
        path: Vec::new(),
    }];
    let mut rows = Some(view.outliner.roots.as_slice());
    for (k, id) in view.context.iter().enumerate() {
        let row = rows.and_then(|rows| rows.iter().find(|r| r.id == *id));
        out.push(Crumb {
            name: row.map_or("Group", outliner::display_name).to_string(),
            path: view.context[..=k].to_vec(),
        });
        rows = row.map(|r| view.outliner.children(r.definition));
    }
    out
}

/// The crumbs to draw, in order, given each crumb's width with its separator, the room and the
/// ellipsis's width: indices, with `None` for the one "…". Model and the last crumb always
/// show; when more than two do not fit, the oldest after Model fold first.
pub(super) fn fold_crumbs(widths: &[f32], room: f32, ellipsis: f32) -> Vec<Option<usize>> {
    let n = widths.len();
    if n <= 2 || widths.iter().sum::<f32>() <= room {
        return (0..n).map(Some).collect();
    }
    let last = n - 1;
    let fits = |from: usize| widths[0] + ellipsis + widths[from..].iter().sum::<f32>() <= room;
    // Index 1 folds first; the last crumb stays even when nothing else fits.
    let mut first = 2;
    while first < last && !fits(first) {
        first += 1;
    }
    let mut out = vec![Some(0), None];
    out.extend((first..n).map(Some));
    out
}

/// A name cut to `CRUMB_CHARS` characters, the last of them "…", when it is longer.
fn cut(name: &str) -> String {
    if name.chars().count() <= CRUMB_CHARS {
        name.to_string()
    } else {
        let mut cut: String = name.chars().take(CRUMB_CHARS - 1).collect();
        cut.push('…');
        cut
    }
}

/// The crumbs, separated by carets: a button per level that opens it, the open level as strong
/// text, and the oldest levels folded into "…" when they do not fit.
fn breadcrumb(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let t = theme::tokens(ui.ctx().theme());
    let crumbs = crumbs(view);
    let names: Vec<String> = crumbs.iter().map(|c| cut(&c.name)).collect();
    let font = TextStyle::Button.resolve(ui.style());
    let width = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_string(), font.clone(), t.text_primary)
            .size()
            .x
    };
    let gap = ui.spacing().item_spacing.x;
    let padding = 2.0 * ui.spacing().button_padding.x;
    let caret = egui::FontId::new(font.size, fonts::icons());
    let separator = ui
        .painter()
        .layout_no_wrap(icons::CARET_RIGHT.into(), caret.clone(), t.text_secondary)
        .size()
        .x
        + gap;
    let widths: Vec<f32> = names
        .iter()
        .enumerate()
        .map(|(k, name)| width(name) + padding + gap + if k > 0 { separator } else { 0.0 })
        .collect();
    let ellipsis = width("…") + gap + separator;
    let shown = fold_crumbs(&widths, ui.available_width(), ellipsis);
    let last = crumbs.len() - 1;
    for (k, entry) in shown.iter().enumerate() {
        if k > 0 {
            let separator = ui.label(
                RichText::new(icons::CARET_RIGHT)
                    .font(caret.clone())
                    .color(t.text_secondary),
            );
            // Read as text, not as the glyph's code point; as the hidden levels' tooltip
            // separates them.
            separator.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, ">"));
        }
        match *entry {
            Some(i) if i == last => {
                let response = ui.label(RichText::new(&names[i]).strong());
                let name = format!("Editing {}", names[i]);
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &name));
            }
            Some(i) => {
                let response = ui.add(Button::new(&names[i]).frame(false));
                let name = format!("Go to {}", names[i]);
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
                if response.clicked() {
                    actions.push(UiAction::OpenPath(crumbs[i].path.clone()));
                }
            }
            None => {
                let hidden: Vec<&str> = (1..last)
                    .filter(|i| !shown.contains(&Some(*i)))
                    .map(|i| names[i].as_str())
                    .collect();
                let response =
                    ui.add(egui::Label::new(RichText::new("…").weak()).sense(Sense::hover()));
                response
                    .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, "Hidden levels"));
                response.on_hover_text(hidden.join(" > "));
            }
        }
    }
}

/// A history label as words: `push_pull` reads "Push pull".
pub(crate) fn step_name(label: &str) -> String {
    let words = label.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
