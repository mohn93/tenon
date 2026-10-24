//! The Settings window: a non-modal window with an Appearance tab (theme and UI scale), an
//! Input tab (what trackpad scrolling does), a Files tab (how often to autosave) and a Model
//! tab (the open model's display units, which are saved with the model). Each change is
//! reported at once through `UiAction`; the window holds only which tab is showing, in egui's
//! memory.

use egui::{Button, Id, Ui};
use session::{LengthUnit, Units, format_angle, format_length, parse_angle, parse_length};

use super::theme::{ThemeChoice, UiScale};
use super::{SettingsView, UiAction, View, widgets};
use crate::navigation::TrackpadScroll;
use crate::platform::Features;
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
    Model,
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
                let files = view.features.autosave || view.features.library;
                for (name, this) in [
                    ("Appearance", Tab::Appearance),
                    ("Input", Tab::Input),
                    ("Files", Tab::Files),
                    ("Model", Tab::Model),
                ] {
                    if this == Tab::Files && !files {
                        continue;
                    }
                    if ui.add(Button::selectable(tab == this, name)).clicked() {
                        tab = this;
                    }
                }
            });
            ui.separator();
            match tab {
                Tab::Appearance => appearance(ui, view.theme, settings.ui_scale, actions),
                Tab::Input => input(ui, settings.trackpad, actions),
                Tab::Files => files(
                    ui,
                    settings.autosave_minutes,
                    view.library.folder,
                    view.features,
                    actions,
                ),
                Tab::Model => model(ui, settings.units, settings.transition, actions),
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
fn files(
    ui: &mut Ui,
    minutes: u32,
    library: Option<&std::path::Path>,
    features: Features,
    actions: &mut Vec<UiAction>,
) {
    let mut value = minutes;
    if features.autosave {
        autosave(ui, &mut value, minutes, actions);
    }
    if features.library {
        if features.autosave {
            ui.add_space(8.0);
        }
        library_folder(ui, library, actions);
    }
}

/// The autosave slider.
fn autosave(ui: &mut Ui, value: &mut u32, minutes: u32, actions: &mut Vec<UiAction>) {
    ui.horizontal(|ui| {
        let label = ui.label("Autosave every");
        if widgets::slider_row(
            ui,
            label.id,
            "Autosave every",
            value,
            0..=prefs::MAX_AUTOSAVE_MINUTES,
            1.0,
            |b| {
                b.custom_formatter(|v, _| autosave_text(v))
                    .custom_parser(parse_autosave)
            },
        )
        .changed()
            && *value != minutes
        {
            actions.push(UiAction::SetAutosave(*value));
        }
    });
    ui.label(
        egui::RichText::new(
            "A copy of unsaved changes, offered back if the app ends without saving",
        )
        .weak(),
    );
}

/// The library folder and its buttons.
fn library_folder(ui: &mut Ui, library: Option<&std::path::Path>, actions: &mut Vec<UiAction>) {
    ui.label("Library folder");
    ui.horizontal(|ui| {
        let shown = library.map_or_else(|| "Not set".to_string(), |p| p.display().to_string());
        ui.add(egui::Label::new(egui::RichText::new(shown).weak()).truncate());
    });
    ui.horizontal(|ui| {
        if ui.button("Choose...").clicked() {
            actions.push(UiAction::ChooseLibraryFolder);
        }
        if ui
            .add_enabled(library.is_some(), Button::new("Clear"))
            .clicked()
        {
            actions.push(UiAction::ClearLibraryFolder);
        }
    });
    ui.label(
        egui::RichText::new(
            "Components, materials and models saved here appear in the Library tab",
        )
        .weak(),
    );
}

/// The height of the Model tab's precision list, in points.
const PRECISION_LIST_HEIGHT: f32 = 260.0;

/// The units a Decimal format offers on the Model tab, with their names.
const DECIMAL_UNITS: [(LengthUnit, &str); 4] = [
    (LengthUnit::Meters, "Meters"),
    (LengthUnit::Centimeters, "Centimeters"),
    (LengthUnit::Millimeters, "Millimeters"),
    (LengthUnit::Inches, "Inches"),
];

/// The formats the Model tab offers: Decimal (a unit of the list above), Architectural (feet
/// and inches) and Engineering (decimal feet).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Decimal,
    Architectural,
    Engineering,
}

impl Format {
    const ALL: [(Format, &'static str); 3] = [
        (Format::Decimal, "Decimal"),
        (Format::Architectural, "Architectural"),
        (Format::Engineering, "Engineering"),
    ];

    fn of(length: LengthUnit) -> Format {
        match length {
            LengthUnit::FeetInches => Format::Architectural,
            LengthUnit::Feet => Format::Engineering,
            _ => Format::Decimal,
        }
    }
}

/// The length snapping a freshly ticked box starts with, in meters: a round step of the unit.
fn default_length_snap(length: LengthUnit) -> f64 {
    match length {
        LengthUnit::Meters => 0.1,
        LengthUnit::Centimeters => 0.01,
        LengthUnit::Millimeters => 0.001,
        LengthUnit::Inches | LengthUnit::FeetInches => 0.0254,
        LengthUnit::Feet => 0.3048,
    }
}

/// The angle snapping a freshly ticked box starts with, in degrees.
const DEFAULT_ANGLE_SNAP: f64 = 15.0;

/// How the Model tab names `precision` for `length`: the decimals shown ("0.00"), or for feet
/// and inches the finest fraction of an inch ("1/16\"").
fn precision_text(length: LengthUnit, precision: u8) -> String {
    match (length, precision) {
        (LengthUnit::FeetInches, 0) => "1\"".into(),
        (LengthUnit::FeetInches, p) => format!("1/{}\"", 1u32 << p),
        (_, 0) => "0".into(),
        (_, p) => format!("0.{}", "0".repeat(p.into())),
    }
}

/// A single-line box whose text is kept while it has focus and reported (once) when Enter or
/// a click elsewhere ends the edit (Escape discards it); unfocused it shows `current`. None until then.
fn committed_text(
    ui: &mut Ui,
    id: Id,
    label: &str,
    current: &str,
    enabled: bool,
) -> Option<String> {
    let mut text = if ui.memory(|m| m.has_focus(id)) {
        ui.data(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| current.to_string())
    } else {
        current.to_string()
    };
    let response = ui.add_enabled(
        enabled,
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(90.0),
    );
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, label);
        info.current_text_value = Some(text.clone());
        info
    });
    if response.lost_focus() {
        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
        ui.data_mut(|d| d.remove_temp::<String>(id));
        return (text != current && !escaped).then_some(text);
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    }
    None
}

/// The open model's display units: the format (decimal, architectural or engineering), the
/// unit of a decimal format, the precision, the length and angle snapping and the angle
/// precision. They are the model's, saved with it, and each change is an undo step; the tab
/// says so.
fn model(
    ui: &mut Ui,
    units: Units,
    transition: kernel::SceneTransition,
    actions: &mut Vec<UiAction>,
) {
    let mut chosen = units;
    let format = Format::of(units.length);
    // The snapping values show every digit they have, not the display precision.
    let full = Units {
        precision: session::MAX_PRECISION,
        angle_precision: session::MAX_ANGLE_PRECISION,
        ..units
    };
    egui::Grid::new("settings-model")
        .num_columns(2)
        .show(ui, |ui| {
            let label = ui.label("Format");
            let mut picked = format;
            egui::ComboBox::from_id_salt("settings-format")
                .selected_text(
                    Format::ALL
                        .iter()
                        .find(|(f, _)| *f == format)
                        .map_or("", |(_, n)| n),
                )
                .show_ui(ui, |ui| {
                    for (f, text) in Format::ALL {
                        ui.selectable_value(&mut picked, f, text);
                    }
                })
                .response
                .labelled_by(label.id);
            if picked != format {
                chosen.length = match picked {
                    Format::Decimal => LengthUnit::Meters,
                    Format::Architectural => LengthUnit::FeetInches,
                    Format::Engineering => LengthUnit::Feet,
                };
            }
            ui.end_row();
            if format == Format::Decimal {
                let label = ui.label("Unit");
                let name = DECIMAL_UNITS
                    .iter()
                    .find(|(u, _)| *u == units.length)
                    .map_or("", |(_, n)| n);
                egui::ComboBox::from_id_salt("settings-units")
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for (unit, text) in DECIMAL_UNITS {
                            ui.selectable_value(&mut chosen.length, unit, text);
                        }
                    })
                    .response
                    .labelled_by(label.id);
                ui.end_row();
            }
            let label = ui.label("Precision");
            // Tall enough that all seven precisions show without scrolling.
            egui::ComboBox::from_id_salt("settings-precision")
                .height(PRECISION_LIST_HEIGHT)
                .selected_text(precision_text(units.length, units.precision))
                .show_ui(ui, |ui| {
                    for p in 0..=session::MAX_PRECISION {
                        let text = precision_text(units.length, p);
                        ui.selectable_value(&mut chosen.precision, p, text);
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();
            // Length snapping: a box to turn it on and a length in the model's unit.
            let mut on = units.length_snap > 0.0;
            if ui.checkbox(&mut on, "Length snapping").changed() {
                chosen.length_snap = if on {
                    default_length_snap(units.length)
                } else {
                    0.0
                };
            }
            let shown = if on {
                format_length(units.length_snap, &full)
            } else {
                String::new()
            };
            if let Some(text) = committed_text(
                ui,
                Id::new("settings-length-snap"),
                "Length snapping value",
                &shown,
                on,
            ) {
                match parse_length(&text, &units) {
                    Ok(m) if m > 0.0 && m <= session::MAX_LENGTH_SNAP => chosen.length_snap = m,
                    _ => {}
                }
            }
            ui.end_row();
            let label = ui.label("Angle precision");
            egui::ComboBox::from_id_salt("settings-angle-precision")
                .selected_text(precision_text(LengthUnit::Meters, units.angle_precision))
                .show_ui(ui, |ui| {
                    for p in 0..=session::MAX_ANGLE_PRECISION {
                        let text = precision_text(LengthUnit::Meters, p);
                        ui.selectable_value(&mut chosen.angle_precision, p, text);
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();
            let mut on = units.angle_snap > 0.0;
            if ui.checkbox(&mut on, "Angle snapping").changed() {
                chosen.angle_snap = if on { DEFAULT_ANGLE_SNAP } else { 0.0 };
            }
            let shown = if on {
                format_angle(units.angle_snap, &full)
            } else {
                String::new()
            };
            if let Some(text) = committed_text(
                ui,
                Id::new("settings-angle-snap"),
                "Angle snapping value",
                &shown,
                on,
            ) {
                match parse_angle(&text) {
                    Ok(d) if d > 0.0 && d <= session::MAX_ANGLE_SNAP => chosen.angle_snap = d,
                    _ => {}
                }
            }
            ui.end_row();
        });
    ui.label(
        egui::RichText::new(
            "Dimensions, notes and tools use these units, and typed lengths are read in them. \
             Snapping rounds what the cursor measures; typed values are never rounded. They are \
             saved in the model, not in the preferences, and changing them can be undone.",
        )
        .weak(),
    );
    if chosen != units {
        actions.push(UiAction::SetUnits(chosen));
    }
    ui.separator();
    // Going to a scene in the window: animated or at once, and how long it takes.
    let mut next = transition;
    egui::Grid::new("settings-transition")
        .num_columns(2)
        .show(ui, |ui| {
            ui.checkbox(&mut next.on, "Animate scene changes");
            ui.end_row();
            ui.label("Duration (s)");
            if let Some(text) = committed_text(
                ui,
                Id::new("settings-transition-seconds"),
                "Transition seconds",
                &format!("{}", transition.seconds),
                transition.on,
            ) {
                match text.trim().parse::<f64>() {
                    Ok(s)
                        if kernel::SceneTransition {
                            on: true,
                            seconds: s,
                        }
                        .check()
                        .is_ok() =>
                    {
                        next.seconds = s;
                    }
                    _ => {}
                }
            }
            ui.end_row();
        });
    ui.label(
        egui::RichText::new(
            "Going to a scene in the window moves the view and the instances the scene stores \
             over this time (0 to 10 seconds); any input finishes it at once. Saved in the model.",
        )
        .weak(),
    );
    if next != transition {
        actions.push(UiAction::SetSceneTransition(next));
    }
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

    #[test]
    fn precisions_read_as_decimals_or_fractions_of_an_inch() {
        assert_eq!(precision_text(LengthUnit::Meters, 0), "0");
        assert_eq!(precision_text(LengthUnit::Millimeters, 3), "0.000");
        assert_eq!(precision_text(LengthUnit::FeetInches, 0), "1\"");
        assert_eq!(precision_text(LengthUnit::FeetInches, 4), "1/16\"");
        assert_eq!(precision_text(LengthUnit::FeetInches, 6), "1/64\"");
    }
}
