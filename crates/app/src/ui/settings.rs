//! The Settings window: a non-modal window with an Appearance tab (theme and UI scale) and an
//! Input tab (what trackpad scrolling does). Each change is reported at once through
//! `UiAction`; the window holds only which tab is showing, in egui's memory.

use egui::{Button, Id, Ui};

use super::theme::{ThemeChoice, UiScale};
use super::{SettingsView, UiAction, View};
use crate::navigation::TrackpadScroll;

/// The window's width, in points.
const WIDTH: f32 = 380.0;

/// The tab showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Appearance,
    Input,
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
    egui::Window::new("Settings")
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
                for (name, this) in [("Appearance", Tab::Appearance), ("Input", Tab::Input)] {
                    if ui.add(Button::selectable(tab == this, name)).clicked() {
                        tab = this;
                    }
                }
            });
            ui.separator();
            match tab {
                Tab::Appearance => appearance(ui, view.theme, settings.ui_scale, actions),
                Tab::Input => input(ui, settings.trackpad, actions),
            }
        });
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
