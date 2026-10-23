//! The Shadows tab: the On toggle, the time and date of the sun, the Light and Dark levels, On
//! ground, and the model's Location. The shadow settings are a view setting and change as the
//! controls move (no undo step); the location is the model's and is set when a field is
//! entered or left, one undo step per edit.

use egui::text::{CCursor, CCursorRange};
use egui::{Color32, Id, RichText, Ui};
use kernel::{Location, Shadows};

use super::widgets;
use super::{ShadowChange, UiAction, View};

/// The slider's steps for the time of day, in minutes.
const TIME_STEP: u16 = 15;

/// The latest time of day the slider reaches: 23:45.
const LAST_TIME: u16 = 24 * 60 - TIME_STEP;

/// The month names the date label uses.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The day of the year (1 to 366) of a date.
pub(super) fn day_of_year(year: u16, month: u8, day: u8) -> u16 {
    let before: u16 = (1..month)
        .map(|m| u16::from(Shadows::days_in_month(year, m)))
        .sum();
    before + u16::from(day)
}

/// The month and day of day number `n` (1 to 366, held to the year's length) of `year`.
pub(super) fn month_and_day(year: u16, n: u16) -> (u8, u8) {
    let mut left = n.max(1);
    for month in 1..=12u8 {
        let days = u16::from(Shadows::days_in_month(year, month));
        if left <= days {
            return (month, left as u8);
        }
        left -= days;
    }
    (12, 31)
}

/// "Nov 8".
pub(super) fn date_label(month: u8, day: u8) -> String {
    let name = MONTHS
        .get(usize::from(month).wrapping_sub(1))
        .unwrap_or(&"?");
    format!("{name} {day}")
}

/// "13:30".
pub(super) fn time_text(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// "2026-11-08".
fn date_text(year: u16, month: u8, day: u8) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}

/// A text field named `name` for the value `shown`: what is typed lives in egui's memory while
/// the field has focus, and is returned once, when Enter or leaving the field ends the edit
/// with a changed text (Escape keeps the value). The text is selected as the field takes focus.
fn text_field(ui: &mut Ui, name: &str, shown: &str, width: f32) -> Option<String> {
    let key = Id::new(("shadows-text", name));
    let mut text = ui
        .data(|d| d.get_temp::<String>(key))
        .unwrap_or_else(|| shown.to_owned());
    let output = egui::TextEdit::singleline(&mut text)
        .id(Id::new(("shadows-field", name)))
        .desired_width(width)
        .show(ui);
    let edit = &output.response.response;
    edit.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name);
        info.current_text_value = Some(text.clone());
        info
    });
    if edit.gained_focus() {
        let mut state = output.state.clone();
        let all = CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count()));
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), edit.id);
    }
    if edit.lost_focus() {
        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
        ui.data_mut(|d| d.remove::<String>(key));
        return (!escaped && text != shown).then_some(text);
    }
    if edit.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
    None
}

/// The Shadows tab (the tab is its title); the dock's top area scrolls it.
pub(super) fn shadows(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let s = view.shadows;
    let mut on = s.on;
    if ui
        .checkbox(&mut on, "On")
        .on_hover_text("Draw the sun's light and shadows (not while Night Lighting is on)")
        .changed()
    {
        actions.push(UiAction::SetShadowsOn(on));
    }
    widgets::section_header(ui, "Time and date");
    time_row(ui, &s, actions);
    date_row(ui, &s, actions);
    widgets::section_header(ui, "Light and dark");
    level_row(ui, "Light", s.light, actions, |light| ShadowChange {
        light: Some(light),
        ..Default::default()
    });
    level_row(ui, "Dark", s.dark, actions, |dark| ShadowChange {
        dark: Some(dark),
        ..Default::default()
    });
    let mut ground = s.on_ground;
    if ui
        .checkbox(&mut ground, "On ground")
        .on_hover_text("Draw shadows on the ground plane")
        .changed()
    {
        actions.push(UiAction::SetShadows(ShadowChange {
            on_ground: Some(ground),
            ..Default::default()
        }));
    }
    widgets::section_header(ui, "Location");
    location(ui, view.location, actions);
}

/// The Time slider (15-minute steps) and its HH:MM field.
fn time_row(ui: &mut Ui, s: &Shadows, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        let label = ui.label("Time");
        let mut minutes = s.minutes.min(LAST_TIME);
        ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(40.0);
        let slider = ui
            .add(
                egui::Slider::new(&mut minutes, 0..=LAST_TIME)
                    .step_by(f64::from(TIME_STEP))
                    .show_value(false),
            )
            .labelled_by(label.id);
        if slider.changed() && minutes != s.minutes {
            actions.push(UiAction::SetShadows(ShadowChange {
                time: Some(time_text(minutes)),
                ..Default::default()
            }));
        }
        if let Some(text) = text_field(ui, "Time of day", &time_text(s.minutes), 50.0) {
            actions.push(UiAction::SetShadows(ShadowChange {
                time: Some(text),
                ..Default::default()
            }));
        }
    });
}

/// The Date slider (day of the year, with its "Nov 8" label) and the year field.
fn date_row(ui: &mut Ui, s: &Shadows, actions: &mut Vec<UiAction>) {
    let days = day_of_year(s.year, 12, 31);
    ui.horizontal(|ui| {
        let label = ui.label("Date");
        let mut n = day_of_year(s.year, s.month, s.day);
        ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(40.0);
        let slider = ui
            .add(egui::Slider::new(&mut n, 1..=days).show_value(false))
            .labelled_by(label.id);
        if slider.changed() {
            let (month, day) = month_and_day(s.year, n);
            if (month, day) != (s.month, s.day) {
                actions.push(UiAction::SetShadows(ShadowChange {
                    date: Some(date_text(s.year, month, day)),
                    ..Default::default()
                }));
            }
        }
        ui.label(date_label(s.month, s.day));
    });
    ui.horizontal(|ui| {
        ui.label("Year");
        if let Some(text) = text_field(ui, "Year value", &s.year.to_string(), 50.0) {
            // The day stays, held to the new year's month (Feb 29). Text that is no year goes
            // to the command as typed, which refuses it.
            let date = match text.trim().parse::<u16>() {
                Ok(year) => {
                    let day = s.day.min(Shadows::days_in_month(year, s.month));
                    date_text(year, s.month, day.max(1))
                }
                Err(_) => format!("{}-{:02}-{:02}", text.trim(), s.month, s.day),
            };
            actions.push(UiAction::SetShadows(ShadowChange {
                date: Some(date),
                ..Default::default()
            }));
        }
    });
}

/// A level slider from 0 to 100 percent with its value box.
fn level_row(
    ui: &mut Ui,
    name: &str,
    shown: u8,
    actions: &mut Vec<UiAction>,
    change: impl FnOnce(u8) -> ShadowChange,
) {
    ui.horizontal(|ui| {
        let label = ui.label(name);
        let mut value = shown;
        ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(40.0);
        let slider = widgets::slider_row(ui, label.id, name, &mut value, 0..=100, 1.0, |b| {
            b.suffix("%")
        });
        if slider.changed() && value != shown {
            actions.push(UiAction::SetShadows(change(value)));
        }
    });
}

/// The egui memory key of the message about a location field that is no number.
fn error_key() -> Id {
    Id::new("shadows-location-error")
}

/// The four location fields. Entering a number sets the whole location (one undo step); text
/// that is no number says so under the fields; a number out of range goes to the command,
/// which refuses it in the status line.
fn location(ui: &mut Ui, now: Location, actions: &mut Vec<UiAction>) {
    type Set = fn(&mut Location, f64);
    let fields: [(&str, f64, Set); 4] = [
        ("Latitude", now.latitude, |l, v| l.latitude = v),
        ("Longitude", now.longitude, |l, v| l.longitude = v),
        ("North angle", now.north_angle, |l, v| l.north_angle = v),
        ("UTC offset", now.utc_offset, |l, v| l.utc_offset = v),
    ];
    for (name, value, set) in fields {
        ui.horizontal(|ui| {
            ui.label(name);
            let field = format!("{name} value");
            let Some(text) = text_field(ui, &field, &value.to_string(), 80.0) else {
                return;
            };
            match text.trim().parse::<f64>() {
                Ok(number) => {
                    let mut next = now;
                    set(&mut next, number);
                    ui.data_mut(|d| d.remove::<String>(error_key()));
                    actions.push(UiAction::SetLocation(next));
                }
                Err(_) => {
                    let why = format!("{name} must be a number");
                    ui.data_mut(|d| d.insert_temp(error_key(), why));
                }
            }
        });
    }
    if let Some(why) = ui.data(|d| d.get_temp::<String>(error_key())) {
        ui.label(RichText::new(why).color(Color32::from_rgb(200, 60, 50)));
    }
    ui.label(
        RichText::new("North angle is clockwise from the green axis; UTC offset is in hours.")
            .weak(),
    );
}
