//! The Settings window: a non-modal window with an Appearance tab (theme and UI scale), an
//! Input tab (what trackpad scrolling does) and a Files tab (how often to autosave). Each change is reported at once through
//! `UiAction`; the window holds only which tab is showing, in egui's memory.

use egui::{Button, Id, Ui};

use super::theme::{ThemeChoice, UiScale};
use super::{SettingsView, UiAction, View, widgets};
use crate::navigation::TrackpadScroll;
use crate::prefs;

/// The window's width, in points.
const WIDTH: f32 = 380.0;

/// The tab showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Appearance,
    Input,
    Files,
}

/// The egui memory key of the tab showing.
fn tab_id() -> Id {
    Id::new("settings-tab")
}

/// The Settings window, centred over the window when it first opens.
pub(super) fn settings_window(
    ui: &mut Ui,
    view: &View,
    settings: &SettingsView,
    actions: &mut Vec<UiAction>,
) {
    let mut open = true;
    let ctx = ui.ctx().clone();
    let mut tab = ctx
        .data(|d| d.get_temp::<Tab>(tab_id()))
        .unwrap_or_default();
    let shown = egui::Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .min_width(WIDTH)
        .max_width(WIDTH)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .constrain(true)
        .show(&ctx, |ui| {
            ui.horizontal(|ui| {
                for (name, this) in [
                    ("Appearance", Tab::Appearance),
                    ("Input", Tab::Input),
                    ("Files", Tab::Files),
                ] {
                    if ui.add(Button::selectable(tab == this, name)).clicked() {
                        tab = this;
                    }
                }
            });
            ui.separator();
            match tab {
                Tab::Appearance => appearance(ui, view.theme, settings.ui_scale, actions),
                Tab::Input => input(ui, settings.trackpad, actions),
                Tab::Files => files(ui, settings.autosave_minutes, actions),
            }
        });
    widgets::name_title_bar(&ctx, &shown, "Settings");
    ctx.data_mut(|d| d.insert_temp(tab_id(), tab));
    if !open {
        actions.push(UiAction::ShowSettings(false));
    }
}

/// The theme and the UI scale, each a row of selectable buttons.
fn appearance(ui: &mut Ui, theme: ThemeChoice, scale: UiScale, actions: &mut Vec<UiAction>) {
    egui::Grid::new("settings-appearance")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Theme");
            ui.horizontal(|ui| {
                for choice in ThemeChoice::ALL {
                    if ui
                        .add(Button::selectable(theme == choice, choice.label()))
                        .clicked()
                    {
                        actions.push(UiAction::SetTheme(choice));
                    }
                }
            });
            ui.end_row();
            ui.label("UI scale");
            ui.horizontal(|ui| {
                for s in UiScale::ALL {
                    if ui.add(Button::selectable(scale == s, s.label())).clicked() {
                        actions.push(UiAction::SetUiScale(s));
                    }
                }
            });
            ui.end_row();
        });
}

/// What a plain trackpad scroll does: two radio buttons, naming Ctrl for Cmd off macOS.
fn input(ui: &mut Ui, trackpad: TrackpadScroll, actions: &mut Vec<UiAction>) {
    let cmd = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    ui.label("Trackpad scroll");
    for (mode, label) in [
        (
            TrackpadScroll::Orbit,
            format!("Orbits (Shift pans, {cmd} zooms)"),
        ),
        (
            TrackpadScroll::Zoom,
            format!("Zooms (Shift pans, {cmd} orbits)"),
        ),
    ] {
        if ui.radio(trackpad == mode, label).clicked() && trackpad != mode {
            actions.push(UiAction::SetTrackpad(mode));
        }
    }
}

/// How often the open model is copied to the recovery folder while it has unsaved changes: a
/// slider of minutes, 0 shown as "Off".
fn files(ui: &mut Ui, minutes: u32, actions: &mut Vec<UiAction>) {
    let mut value = minutes;
    ui.horizontal(|ui| {
        let label = ui.label("Autosave every");
        if widgets::slider_row(
            ui,
            label.id,
            "Autosave every",
            &mut value,
            0..=prefs::MAX_AUTOSAVE_MINUTES,
            1.0,
            |b| {
                b.custom_formatter(|v, _| autosave_text(v))
                    .custom_parser(parse_autosave)
            },
        )
        .changed()
            && value != minutes
        {
            actions.push(UiAction::SetAutosave(value));
        }
    });
    ui.label(
        egui::RichText::new(
            "A copy of unsaved changes, offered back if the app ends without saving",
        )
        .weak(),
    );
}

/// The autosave interval as the box shows it: "Off" or "5 min".
fn autosave_text(minutes: f64) -> String {
    if minutes < 0.5 {
        "Off".into()
    } else {
        format!("{minutes:.0} min")
    }
}

/// An interval typed in the box: "off", or minutes with or without "min".
fn parse_autosave(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("off") {
        return Some(0.0);
    }
    let number = text.strip_suffix("min").unwrap_or(text).trim();
    number.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_autosave_box_reads_off_or_minutes() {
        assert_eq!(autosave_text(0.0), "Off");
        assert_eq!(autosave_text(5.0), "5 min");
        assert_eq!(parse_autosave("Off"), Some(0.0));
        assert_eq!(parse_autosave(" 10 min"), Some(10.0));
        assert_eq!(parse_autosave("7"), Some(7.0));
        assert_eq!(parse_autosave("soon"), None);
    }
}
