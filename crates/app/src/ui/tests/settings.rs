use super::*;
use crate::navigation::TrackpadScroll;
use crate::ui::SettingsView;
use crate::ui::theme::UiScale;

/// The Settings window open at 100%.
fn settings_fixture() -> Fixture {
    Fixture {
        settings: Some(SettingsView {
            ui_scale: UiScale::P100,
            trackpad: TrackpadScroll::Orbit,
            autosave_minutes: 5,
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
