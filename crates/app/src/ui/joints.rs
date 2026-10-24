//! The Joints tab: a row per joint with its kind and value, a slider to drive it, its limits
//! and a Delete button. A click on the name selects the joint's two parts. A slider drags
//! live (`DriveJoint` with `done` false for each new value) and reports its end once.

use std::ops::RangeInclusive;

use egui::{Id, RichText, Ui, Vec2};
use kernel::{JointId, JointKind};
use session::{Units, format_length, parse_length};

use super::widgets;
use super::{JointRow, UiAction, View, icons};

/// The slider's range for an unlimited angle, in degrees.
const TURN: f64 = 360.0;
/// The slider's range for an unlimited distance, in meters (each way).
const REACH: f64 = 2.0;

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

/// What a value reads as: degrees, or a length in the model's units.
fn value_text(kind_angular: bool, value: f64, units: &Units) -> String {
    if kind_angular {
        format!("{value:.1}\u{b0}")
    } else {
        format_length(value, units)
    }
}

fn joint_row(ui: &mut Ui, row: &JointRow, units: Units, actions: &mut Vec<UiAction>) {
    let info = &row.info;
    let angular = info.kind.is_angular();
    ui.horizontal(|ui| {
        let delete = ui.available_width() - 28.0 - if info.kind.has_value() { 28.0 } else { 0.0 };
        let name = ui.add(
            egui::Button::selectable(row.selected, &info.name)
                .truncate()
                .min_size(Vec2::new((delete - 130.0).max(40.0), 0.0)),
        );
        name.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, row.selected, &info.name)
        });
        if name.clicked() {
            actions.push(UiAction::SelectJoint(info.id));
        }
        let shown = if info.kind.has_value() {
            format!(
                "{} {}",
                kind_name(info.kind),
                value_text(angular, info.value, &units)
            )
        } else {
            kind_name(info.kind).to_string()
        };
        ui.label(RichText::new(shown).weak());
        if info.kind.has_value()
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
    let response = ui
        .horizontal(|ui| {
            let label = ui.label(name);
            ui.spacing_mut().slider_width = (ui.available_width() - 90.0).max(40.0);
            widgets::slider_row(ui, label.id, name, &mut value, range, step, |b| {
                let b = b.update_while_editing(false);
                if angular {
                    b.suffix("\u{b0}")
                } else {
                    b.custom_formatter(move |v, _| format_length(v, &units))
                        .custom_parser(move |text| parse_length(text, &units).ok())
                }
            })
        })
        .inner;
    if response.dragged() {
        let reported = held.map_or(shown, |h| h.1);
        ui.data_mut(|d| d.insert_temp::<Held>(key, (value, value)));
        (value != reported).then_some((value, false))
    } else {
        ui.data_mut(|d| d.remove::<Held>(key));
        let ended = response.drag_stopped() || (response.changed() && value != shown);
        ended.then_some((value, true))
    }
}
