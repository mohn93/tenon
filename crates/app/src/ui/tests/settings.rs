use super::*;
use crate::ui::SettingsView;
use crate::ui::theme::UiScale;

/// The Settings window open at 100%.
fn settings_fixture() -> Fixture {
    Fixture {
        settings: Some(SettingsView {
            ui_scale: UiScale::P100,
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

#[test]
fn no_settings_window_while_it_is_closed() {
    let h = harness(Fixture::default());
    assert!(h.query_by_label("Appearance").is_none());
}
