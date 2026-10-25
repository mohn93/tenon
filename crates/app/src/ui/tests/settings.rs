use super::*;
use crate::navigation::TrackpadScroll;
use crate::ui::SettingsView;
use crate::ui::theme::UiScale;

/// The Settings window open at 100%.
pub(super) fn settings_fixture() -> Fixture {
    Fixture {
        settings: Some(SettingsView {
            ui_scale: UiScale::P100,
            trackpad: TrackpadScroll::Orbit,
            autosave_minutes: 5,
            transition: kernel::SceneTransition::DEFAULT,
            units: session::Units::default(),
        }),
        ..Default::default()
    }
}

#[test]
fn settings_sets_the_theme_and_the_ui_scale() {
    let mut h = harness(settings_fixture());
    assert!(h.query_by_label("Settings").is_some(), "the window");
    assert_eq!(is_on(&h, "Appearance"), Some(true));
    assert_eq!(is_on(&h, "Input"), Some(false));
    assert_eq!(is_on(&h, "100%"), Some(true));
    assert_eq!(is_on(&h, "125%"), Some(false));
    assert_eq!(is_on(&h, "System"), Some(true));
    assert_eq!(click(&mut h, "125%"), [UiAction::SetUiScale(UiScale::P125)]);
    assert_eq!(click(&mut h, "90%"), [UiAction::SetUiScale(UiScale::P90)]);
    assert_eq!(
        click(&mut h, "Dark"),
        [UiAction::SetTheme(ThemeChoice::Dark)]
    );
    assert_eq!(
        click(&mut h, "Close window"),
        [UiAction::ShowSettings(false)]
    );
}

#[test]
fn the_settings_window_settles_in_one_frame() {
    let mut h = harness(settings_fixture());
    assert_eq!(h.try_run().unwrap(), 1);
}

/// The Input tab's choice labels, with the window's word for Cmd.
fn trackpad_labels() -> (String, String) {
    let cmd = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    (
        format!("Orbits (Shift pans, {cmd} zooms)"),
        format!("Zooms (Shift pans, {cmd} orbits)"),
    )
}

#[test]
fn the_input_tab_sets_the_trackpad() {
    let (orbits, zooms) = trackpad_labels();
    let mut h = harness(settings_fixture());
    assert!(h.query_by_label(&orbits).is_none(), "on the Input tab");
    assert_eq!(click(&mut h, "Input"), []);
    assert_eq!(is_on(&h, "Input"), Some(true));
    assert!(
        h.query_by_label("125%").is_none(),
        "not on the Appearance tab"
    );
    assert!(drawn_texts(&h).iter().any(|t| t == "Trackpad scroll"));
    assert_eq!(is_on(&h, &orbits), Some(true));
    assert_eq!(is_on(&h, &zooms), Some(false));
    assert_eq!(
        click(&mut h, &zooms),
        [UiAction::SetTrackpad(TrackpadScroll::Zoom)]
    );
    // The tab stays chosen while the window is open.
    h.run_ok();
    assert_eq!(is_on(&h, "Input"), Some(true));
    let mut h = harness(Fixture {
        settings: Some(SettingsView {
            ui_scale: UiScale::P100,
            trackpad: TrackpadScroll::Zoom,
            autosave_minutes: 5,
            transition: kernel::SceneTransition::DEFAULT,
            units: session::Units::default(),
        }),
        ..Default::default()
    });
    click(&mut h, "Input");
    assert_eq!(is_on(&h, &zooms), Some(true));
    assert_eq!(
        click(&mut h, &orbits),
        [UiAction::SetTrackpad(TrackpadScroll::Orbit)]
    );
}

#[test]
fn no_settings_window_while_it_is_closed() {
    let h = harness(Fixture::default());
    assert!(h.query_by_label("Appearance").is_none());
}

#[test]
fn the_model_tab_sets_the_models_units() {
    use session::{LengthUnit, Units};
    let mut h = harness(settings_fixture());
    model_tab(&mut h);
    let texts = drawn_texts(&h);
    assert!(texts.iter().any(|t| t == "Meters"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "0.000"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.contains("saved in the model")),
        "the tab says where the units are kept: {texts:?}"
    );
    assert_eq!(click(&mut h, "Format"), []);
    assert_eq!(
        click(&mut h, "Architectural"),
        [UiAction::SetUnits(Units {
            length: LengthUnit::FeetInches,
            precision: 3,
            ..Units::default()
        })]
    );
    // Feet and inches name their precision as a fraction of an inch.
    let mut h = harness(Fixture {
        settings: Some(SettingsView {
            transition: kernel::SceneTransition::DEFAULT,
            units: Units {
                length: LengthUnit::FeetInches,
                precision: 3,
                ..Units::default()
            },
            ..settings_fixture().settings.unwrap()
        }),
        ..Default::default()
    });
    model_tab(&mut h);
    assert!(drawn_texts(&h).iter().any(|t| t == "1/8\""));
    assert_eq!(click(&mut h, "Precision"), []);
    assert_eq!(
        click(&mut h, "1/64\""),
        [UiAction::SetUnits(Units {
            length: LengthUnit::FeetInches,
            precision: 6,
            ..Units::default()
        })]
    );
}

#[test]
fn the_model_tab_offers_formats_units_and_snapping() {
    use session::{LengthUnit, Units};
    let with = |units: Units| {
        let mut h = harness(Fixture {
            settings: Some(SettingsView {
                transition: kernel::SceneTransition::DEFAULT,
                units,
                ..settings_fixture().settings.unwrap()
            }),
            ..Default::default()
        });
        model_tab(&mut h);
        h
    };
    // Decimal shows the unit; Engineering and Architectural do not.
    let mut h = with(Units::default());
    assert!(h.query_by_label("Unit").is_some());
    assert_eq!(click(&mut h, "Unit"), []);
    assert_eq!(
        click(&mut h, "Centimeters"),
        [UiAction::SetUnits(Units {
            length: LengthUnit::Centimeters,
            ..Units::default()
        })]
    );
    let mut h = with(Units {
        length: LengthUnit::Feet,
        ..Units::default()
    });
    assert!(h.query_by_label("Unit").is_none());
    assert!(drawn_texts(&h).iter().any(|t| t == "Engineering"));
    assert_eq!(click(&mut h, "Format"), []);
    assert_eq!(
        click(&mut h, "Decimal"),
        [UiAction::SetUnits(Units::default())]
    );
    // Snapping: the boxes turn it on with a round step, and off with 0.
    let mut h = with(Units::default());
    assert_eq!(
        click(&mut h, "Length snapping"),
        [UiAction::SetUnits(Units {
            length_snap: 0.1,
            ..Units::default()
        })]
    );
    let mut h = with(Units {
        angle_snap: 0.0,
        ..Units::default()
    });
    assert_eq!(
        click(&mut h, "Angle snapping"),
        [UiAction::SetUnits(Units::default())]
    );
    let mut h = with(Units::default());
    assert_eq!(
        click(&mut h, "Angle snapping"),
        [UiAction::SetUnits(Units {
            angle_snap: 0.0,
            ..Units::default()
        })]
    );
    // Angle precision.
    let mut h = with(Units::default());
    assert_eq!(click(&mut h, "Angle precision"), []);
    assert_eq!(
        click(&mut h, "0.00"),
        [UiAction::SetUnits(Units {
            angle_precision: 2,
            ..Units::default()
        })]
    );
}

#[test]
fn the_snapping_values_are_typed_in_the_model_unit_and_degrees() {
    use session::{LengthUnit, Units};
    let mut h = harness(Fixture {
        settings: Some(SettingsView {
            transition: kernel::SceneTransition::DEFAULT,
            units: Units {
                length: LengthUnit::FeetInches,
                length_snap: 0.0254,
                ..Units::default()
            },
            ..settings_fixture().settings.unwrap()
        }),
        ..Default::default()
    });
    model_tab(&mut h);
    assert!(drawn_texts(&h).iter().any(|t| t == "15°"));
    // A bare number is inches in feet and inches.
    h.get_by_label("Length snapping value").click();
    h.run_ok();
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.run_ok();
    h.event(egui::Event::Text("6".into()));
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    let got = h.state_mut().take();
    assert_eq!(
        got,
        [UiAction::SetUnits(Units {
            length: LengthUnit::FeetInches,
            length_snap: 0.0254 * 6.0,
            ..Units::default()
        })]
    );
}

#[test]
fn the_snapping_boxes_show_full_precision_and_escape_discards() {
    use session::Units;
    let units = Units {
        precision: 0,
        angle_precision: 0,
        length_snap: 0.1,
        angle_snap: 7.5,
        ..Units::default()
    };
    let mut h = harness(Fixture {
        settings: Some(SettingsView {
            transition: kernel::SceneTransition::DEFAULT,
            units,
            ..settings_fixture().settings.unwrap()
        }),
        ..Default::default()
    });
    model_tab(&mut h);
    let texts = drawn_texts(&h);
    assert!(texts.iter().any(|t| t == "0.1 m"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "7.5°"), "{texts:?}");
    // Typing a value and pressing Escape changes nothing.
    h.get_by_label("Length snapping value").click();
    h.run_ok();
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.run_ok();
    h.event(egui::Event::Text("2".into()));
    h.run_ok();
    h.key_press(egui::Key::Escape);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

/// Clicks the Settings window's Model tab (the breadcrumb and the outliner's root share its
/// name): the one in the row of tabs.
fn model_tab(h: &mut H) {
    let row = h.get_by_label("Appearance").rect().center().y;
    h.get_all_by_label("Model")
        .find(|n| (n.rect().center().y - row).abs() < 1.0)
        .expect("the Model tab")
        .click();
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn the_model_tab_sets_the_scene_transition() {
    use kernel::SceneTransition;
    let mut h = harness(settings_fixture());
    model_tab(&mut h);
    assert_eq!(is_on(&h, "Animate scene changes"), Some(true));
    assert_eq!(
        click(&mut h, "Animate scene changes"),
        [UiAction::SetSceneTransition(SceneTransition {
            on: false,
            seconds: 1.0
        })]
    );
    // Typed seconds apply when the box is left; a value out of range changes nothing.
    h.get_by_label("Transition seconds").click();
    h.run_ok();
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.run_ok();
    h.event(egui::Event::Text("2.5".into()));
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(
        h.state_mut().take(),
        [UiAction::SetSceneTransition(SceneTransition {
            on: true,
            seconds: 2.5
        })]
    );
    h.get_by_label("Transition seconds").click();
    h.run_ok();
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.run_ok();
    h.event(egui::Event::Text("11".into()));
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn the_autosave_slider_shows_its_rail_on_the_window() {
    let mut h = harness(settings_fixture());
    click(&mut h, "Files");
    let slider = h
        .get_all_by_label("Autosave every")
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Slider)
        .expect("the slider")
        .rect();
    // The rail: a bar across the slider in a fill other than the window's.
    let window = theme::DARK.bg_raised;
    let rail = drawn_shapes(&h).into_iter().any(|s| match s {
        egui::Shape::Rect(r) => {
            r.fill != window
                && r.fill.a() > 0
                && slider.contains_rect(r.rect)
                && r.rect.width() > 0.8 * slider.width()
        }
        _ => false,
    });
    assert!(rail, "a rail across {slider:?}");
}
