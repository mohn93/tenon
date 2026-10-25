//! The Joints tab: a row per joint with its kind and value, a slider to drive it (none for a
//! joint that follows a cam), its limits and a Delete button. A click on the name selects the joint's two parts. A slider drags
//! live (`DriveJoint` with `done` false for each new value) and reports its end once.

use std::ops::RangeInclusive;

use egui::{Id, RichText, Ui, Vec2};
use kernel::{JointId, JointKind};
use session::{Units, format_length, parse_angle, parse_length};

use super::widgets;
use super::{JointRow, UiAction, View, icons};

/// The slider's range for an unlimited angle, in degrees.
const TURN: f64 = 360.0;
/// The slider's range for an unlimited distance, in meters (each way).
const REACH: f64 = 2.0;
/// The least width of a joint's name before its kind and value are cut short too.
const NAME_MIN: f32 = 64.0;
/// The least width of a slider.
const SLIDER_MIN: f32 = 40.0;
/// The least width of a value box: "-179.5°" and "-0.025 m" fit.
const VALUE_BOX: f32 = 80.0;

/// The kind as the panel names it.
pub(super) fn kind_name(kind: JointKind) -> &'static str {
    match kind {
        JointKind::Fixed => "Fixed",
        JointKind::Revolute => "Revolute",
        JointKind::Prismatic => "Prismatic",
        JointKind::Cylindrical => "Cylindrical",
        JointKind::Ball => "Ball",
    }
}

/// The Joints tab (the tab is its title); the dock's top area scrolls it.
pub(super) fn joints(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    if view.joints.is_empty() {
        ui.label(RichText::new("No joints").weak());
    }
    for row in view.joints {
        joint_row(ui, row, view.units, actions);
        ui.separator();
    }
}

/// What a value reads as: degrees to a tenth, or a length in the model's units. Zero, and a
/// value that rounds to it, has no sign.
fn value_text(kind_angular: bool, value: f64, units: &Units) -> String {
    if kind_angular {
        // Adding zero turns a negative zero positive.
        let tenths = (value * 10.0).round() / 10.0 + 0.0;
        format!("{tenths:.1}\u{b0}")
    } else {
        format_length(value, units)
    }
}

/// The width of `text` laid out on one line in `style` (unless the text sets its own).
fn text_width(ui: &Ui, text: RichText, style: egui::TextStyle) -> f32 {
    egui::WidgetText::from(text)
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, style)
        .size()
        .x
}

/// `add` in a part of the row exactly `width` wide, so a widget that truncates stops there.
fn fixed_width<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let size = Vec2::new(width, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            add(ui)
        },
    )
    .inner
}

fn joint_row(ui: &mut Ui, row: &JointRow, units: Units, actions: &mut Vec<UiAction>) {
    let info = &row.info;
    let angular = info.kind.is_angular();
    ui.horizontal(|ui| {
        let play = info.kind.has_value() && !row.follows;
        let shown = if info.kind.has_value() {
            format!(
                "{} {}",
                kind_name(info.kind),
                value_text(angular, info.value, &units)
            )
        } else {
            kind_name(info.kind).to_string()
        };
        // The buttons keep their size and the kind its text; the name takes what is left, cut
        // short, and only past NAME_MIN does the kind give way too.
        let gap = ui.spacing().item_spacing.x;
        let mut buttons = widgets::icon_button_width(ui, icons::MINUS) + gap;
        if play {
            buttons += widgets::icon_button_width(ui, icons::PLAY) + gap;
        }
        let kind = text_width(ui, RichText::new(&shown).weak(), egui::TextStyle::Body);
        let width = (ui.available_width() - buttons - gap).max(0.0);
        let name_width = (width - kind).max(NAME_MIN.min(width));
        let name = fixed_width(ui, name_width, |ui| {
            ui.add(
                egui::Button::selectable(row.selected, &info.name)
                    .truncate()
                    .min_size(Vec2::new(name_width, 0.0)),
            )
        });
        name.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, row.selected, &info.name)
        });
        if name.clicked() {
            actions.push(UiAction::SelectJoint(info.id));
        }
        fixed_width(ui, (width - name_width).max(0.0), |ui| {
            ui.add(egui::Label::new(RichText::new(shown).weak()).truncate())
        });
        if play
            && widgets::icon_button(
                ui,
                icons::PLAY,
                &format!("Play {}", info.name),
                "Play the joint over its limits (a whole turn when it has none) without changing the model; any click stops it",
                None,
                true,
            )
            .clicked()
        {
            actions.push(UiAction::PlayJoint(info.id));
        }
        if widgets::icon_button(
            ui,
            icons::MINUS,
            &format!("Delete {}", info.name),
            "Delete the joint (the parts stay where they are)",
            None,
            true,
        )
        .clicked()
        {
            actions.push(UiAction::DeleteJoint(info.id));
        }
    });
    if !info.kind.has_value() {
        return;
    }
    if row.follows {
        // A drive of a follower is refused: its value comes from the cam.
        ui.label(RichText::new("Follows its cam; drive the cam's joints").weak());
        return;
    }
    let joint = info.id;
    let slide = info.slide;
    let (lo, hi) = match info.limits {
        Some([lo, hi]) => (lo, hi),
        None if angular => (-TURN, TURN),
        None => (-REACH, REACH),
    };
    let what = if angular { "angle" } else { "distance" };
    let range = lo.min(info.value)..=hi.max(info.value);
    if let Some((value, done)) = drive(
        ui,
        joint,
        &format!("{} {what}", info.name),
        info.value,
        range,
        angular,
        units,
    ) {
        actions.push(UiAction::DriveJoint {
            joint,
            value,
            slide: None,
            done,
        });
    }
    if let Some(slide) = slide
        && let Some((value, done)) = drive(
            ui,
            joint,
            &format!("{} slide", info.name),
            slide,
            -REACH.max(slide.abs())..=REACH.max(slide.abs()),
            false,
            units,
        )
    {
        actions.push(UiAction::DriveJoint {
            joint,
            value: info.value,
            slide: Some(value),
            done,
        });
    }
    if let Some([lo, hi]) = info.limits {
        ui.label(
            RichText::new(format!(
                "Limits {} to {}",
                value_text(angular, lo, &units),
                value_text(angular, hi, &units)
            ))
            .weak(),
        );
    }
}

/// What a drag of a slider holds in egui's memory: the value and the last one reported.
type Held = (f64, f64);

/// A row of `name`, a slider over `range` and its value box (degrees, or the model's units
/// for `angular` false). While a slider is dragged each new value is returned with `done`
/// false; when the drag ends, or a value is entered, it is returned once with `done` true.
fn drive(
    ui: &mut Ui,
    joint: JointId,
    name: &str,
    shown: f64,
    range: RangeInclusive<f64>,
    angular: bool,
    units: Units,
) -> Option<(f64, bool)> {
    let key = Id::new(("joint-slider", joint.0, name));
    let held = ui.data(|d| d.get_temp::<Held>(key));
    let mut value = held.map_or(shown, |h| h.0);
    let step = if angular { 0.5 } else { 0.005 };
    let (track, shown_box) = ui
        .horizontal(|ui| {
            // The box is as wide as its value (VALUE_BOX at least) and the label as its name,
            // cut short to leave the slider SLIDER_MIN; the slider takes the rest.
            let gap = ui.spacing().item_spacing.x;
            let font = ui.style().drag_value_text_style.clone();
            let text = RichText::new(value_text(angular, value, &units)).text_style(font.clone());
            let box_width =
                (text_width(ui, text, font) + 2.0 * ui.spacing().button_padding.x).max(VALUE_BOX);
            let width = (ui.available_width() - box_width - 2.0 * gap).max(0.0);
            let label_width = text_width(ui, RichText::new(name), egui::TextStyle::Body)
                .min((width - SLIDER_MIN).max(0.0));
            let label = fixed_width(ui, label_width, |ui| {
                ui.add(egui::Label::new(name).truncate())
            });
            ui.spacing_mut().slider_width = (width - label_width).max(SLIDER_MIN);
            ui.spacing_mut().interact_size.x = box_width;
            widgets::slider_and_box(ui, label.id, name, &mut value, range, step, |b| {
                b.update_while_editing(false)
                    .custom_formatter(move |v, _| value_text(angular, v, &units))
                    .custom_parser(move |text| {
                        if angular {
                            parse_angle(text).ok()
                        } else {
                            parse_length(text, &units).ok()
                        }
                    })
            })
        })
        .inner;
    // The slider rounds a value off its step (a solved joint a hair off zero) without any
    // input: only a change the user made (a click, keys or a typed value) is an edit.
    let touched = [&track, &shown_box]
        .iter()
        .any(|r| r.clicked() || r.has_focus() || r.lost_focus());
    let response = track.union(shown_box);
    if response.dragged() {
        let reported = held.map_or(shown, |h| h.1);
        ui.data_mut(|d| d.insert_temp::<Held>(key, (value, value)));
        (value != reported).then_some((value, false))
    } else {
        ui.data_mut(|d| d.remove::<Held>(key));
        let ended = response.drag_stopped() || (response.changed() && touched && value != shown);
        ended.then_some((value, true))
    }
}
