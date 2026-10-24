use kernel::{Location, Shadows};

use super::*;
use crate::ui::ShadowChange;
use crate::ui::shadows::{date_label, day_of_year, month_and_day, time_text};

/// The dock on the Shadows tab.
fn shadows_tab() -> Fixture {
    Fixture {
        dock: DockLayout {
            tab: DockTab::Shadows,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Clicks `along` (0 to 1) of the way across the slider named `label`.
/// A window tall enough for the whole tab (the dock's top area is a share of the height and
/// scrolls).
fn tall(f: Fixture) -> H {
    harness_sized(f, Vec2::new(1280.0, 1800.0))
}

fn click_slider(h: &mut H, label: &str, along: f32) -> Vec<UiAction> {
    let slider = rect_of(h, label);
    let at = Pos2::new(slider.min.x + along * slider.width(), slider.center().y);
    let mut actions = click_at(h, at, egui::Modifiers::NONE);
    // The fixture does not follow the change, so a drag's frames repeat it.
    actions.dedup();
    actions
}

fn change(f: impl FnOnce(&mut ShadowChange)) -> UiAction {
    let mut c = ShadowChange::default();
    f(&mut c);
    UiAction::SetShadows(c)
}

#[test]
fn the_tab_opens_the_shadows_panel() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click(&mut h, "Shadows"),
        [UiAction::SetDock(DockLayout {
            tab: DockTab::Shadows,
            ..Default::default()
        })]
    );
    let h = tall(shadows_tab());
    assert_eq!(is_on(&h, "Shadows"), Some(true));
    for label in [
        "On",
        "Time",
        "Time of day",
        "Date",
        "Year value",
        "Light",
        "Light value",
        "Dark",
        "Dark value",
        "On ground",
        "Latitude value",
        "Longitude value",
        "North angle value",
        "UTC offset value",
        "Nov 8",
    ] {
        h.get_by_label(label);
    }
    assert_eq!(is_on(&h, "On"), Some(false));
    assert_eq!(is_on(&h, "On ground"), Some(true));
}

#[test]
fn the_toggles_report_their_new_value() {
    let mut h = tall(shadows_tab());
    assert_eq!(click(&mut h, "On"), [UiAction::SetShadowsOn(true)]);
    assert_eq!(
        click(&mut h, "On ground"),
        [change(|c| c.on_ground = Some(false))]
    );
    let mut h = harness(Fixture {
        shadows: Shadows {
            on: true,
            ..Default::default()
        },
        ..shadows_tab()
    });
    assert_eq!(click(&mut h, "On"), [UiAction::SetShadowsOn(false)]);
}

#[test]
fn the_time_field_takes_a_clock_time() {
    let mut h = tall(shadows_tab());
    assert_eq!(
        type_into(&mut h, "Time of day", 0, "09:15", egui::Key::Enter),
        [change(|c| c.time = Some("09:15".into()))]
    );
    // Text that is no time goes to the command, which refuses it.
    let mut h = tall(shadows_tab());
    assert_eq!(
        type_into(&mut h, "Time of day", 0, "25:00", egui::Key::Enter),
        [change(|c| c.time = Some("25:00".into()))]
    );
    // Entering the shown text again changes nothing.
    let mut h = tall(shadows_tab());
    assert_eq!(
        type_into(&mut h, "Time of day", 0, "13:30", egui::Key::Enter),
        []
    );
}

#[test]
fn the_time_slider_moves_in_quarter_hours() {
    let mut h = tall(shadows_tab());
    let actions = click_slider(&mut h, "Time", 0.5);
    let [UiAction::SetShadows(ShadowChange { time: Some(t), .. })] = actions.as_slice() else {
        panic!("{actions:?}")
    };
    let (hh, mm) = t.split_once(':').unwrap();
    let minutes = hh.parse::<u16>().unwrap() * 60 + mm.parse::<u16>().unwrap();
    assert_eq!(minutes % 15, 0, "{t}");
    assert!((11 * 60..=13 * 60).contains(&minutes), "{t}");
}

#[test]
fn the_date_slider_and_year_field_change_the_date() {
    let mut h = tall(shadows_tab());
    let actions = click_slider(&mut h, "Date", 0.0);
    assert_eq!(actions.len(), 1, "one change for one click: {actions:?}");
    let [UiAction::SetShadows(ShadowChange { date: Some(d), .. })] = actions.as_slice() else {
        panic!("{actions:?}")
    };
    assert!(d.starts_with("2026-01-0"), "{d}");
    let mut h = tall(shadows_tab());
    assert_eq!(
        type_into(&mut h, "Year value", 0, "2030", egui::Key::Enter),
        [change(|c| c.date = Some("2030-11-08".into()))]
    );
    // Feb 29 moves to Feb 28 in a year without it.
    let mut h = tall(Fixture {
        shadows: Shadows {
            year: 2024,
            month: 2,
            day: 29,
            ..Default::default()
        },
        ..shadows_tab()
    });
    h.get_by_label("Feb 29");
    assert_eq!(
        type_into(&mut h, "Year value", 0, "2025", egui::Key::Enter),
        [change(|c| c.date = Some("2025-02-28".into()))]
    );
}

#[test]
fn the_levels_report_percentages() {
    let mut h = tall(shadows_tab());
    assert_eq!(
        type_into(&mut h, "Light value", 0, "60", egui::Key::Enter),
        [change(|c| c.light = Some(60))]
    );
    let mut h = tall(shadows_tab());
    let actions = click_slider(&mut h, "Dark", 0.999);
    assert_eq!(actions, [change(|c| c.dark = Some(100))]);
}

#[test]
fn the_location_fields_set_the_whole_location() {
    for (field, text, wanted) in [
        (
            "Latitude",
            "51.5",
            Location {
                latitude: 51.5,
                ..Location::DEFAULT
            },
        ),
        (
            "Longitude",
            "-0.12",
            Location {
                longitude: -0.12,
                ..Location::DEFAULT
            },
        ),
        (
            "North angle",
            "90",
            Location {
                north_angle: 90.0,
                ..Location::DEFAULT
            },
        ),
        (
            "UTC offset",
            "5.5",
            Location {
                utc_offset: 5.5,
                ..Location::DEFAULT
            },
        ),
    ] {
        let mut h = tall(shadows_tab());
        assert_eq!(
            type_into(&mut h, &format!("{field} value"), 0, text, egui::Key::Enter),
            [UiAction::SetLocation(wanted)],
            "{field}"
        );
    }
}

#[test]
fn a_location_field_that_is_no_number_says_so_and_changes_nothing() {
    let mut h = tall(shadows_tab());
    assert_eq!(
        type_into(&mut h, "Latitude value", 0, "north", egui::Key::Enter),
        []
    );
    h.get_by_label("Latitude must be a number");
    // A number out of range goes to the command.
    let mut h = tall(shadows_tab());
    assert_eq!(
        type_into(&mut h, "Latitude value", 0, "95", egui::Key::Enter),
        [UiAction::SetLocation(Location {
            latitude: 95.0,
            ..Location::DEFAULT
        })]
    );
}

#[test]
fn the_view_menu_toggles_shadows() {
    for on in [false, true] {
        let mut h = harness(Fixture {
            shadows: Shadows {
                on,
                ..Default::default()
            },
            ..Default::default()
        });
        // The tab has the item's name: the item is the node that appears with the menu.
        let there = rects_named(&h, "Shadows");
        click(&mut h, "View");
        let item = new_node(&h, "Shadows", &there);
        assert_eq!(
            item.accesskit_node().toggled(),
            Some(if on {
                egui::accesskit::Toggled::True
            } else {
                egui::accesskit::Toggled::False
            })
        );
        item.click();
        h.run_ok();
        assert_eq!(h.state_mut().take(), [UiAction::SetShadowsOn(!on)]);
    }
    use crate::menus::item_id;
    assert_eq!(
        item_id(&UiAction::SetShadowsOn(true)),
        item_id(&UiAction::SetShadowsOn(false))
    );
}

#[test]
fn dates_and_times_read_as_the_panel_shows_them() {
    assert_eq!(day_of_year(2026, 1, 1), 1);
    assert_eq!(day_of_year(2026, 12, 31), 365);
    assert_eq!(day_of_year(2024, 12, 31), 366);
    assert_eq!(day_of_year(2026, 11, 8), 312);
    for year in [2026, 2024, 1900, 2000] {
        for n in 1..=day_of_year(year, 12, 31) {
            let (m, d) = month_and_day(year, n);
            assert_eq!(day_of_year(year, m, d), n, "{year} day {n}");
        }
    }
    assert_eq!(month_and_day(2026, 400), (12, 31));
    assert_eq!(date_label(11, 8), "Nov 8");
    assert_eq!(date_label(2, 29), "Feb 29");
    assert_eq!(time_text(13 * 60 + 30), "13:30");
    assert_eq!(time_text(5), "00:05");
}

#[test]
fn a_time_off_the_quarter_hours_is_shown_and_not_rewritten() {
    let odd = Fixture {
        shadows: Shadows {
            minutes: 13 * 60 + 37,
            ..Default::default()
        },
        ..shadows_tab()
    };
    let mut h = tall(odd);
    h.run_ok();
    assert_eq!(h.state_mut().take(), [], "no input, no change");
    assert_eq!(
        h.get_by_label("Time of day")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("13:37")
    );
    // Typing a time off the steps sets exactly that time.
    let mut h = tall(shadows_tab());
    assert_eq!(
        type_into(&mut h, "Time of day", 0, "09:10", egui::Key::Enter),
        [change(|c| c.time = Some("09:10".into()))]
    );
}

#[test]
fn no_slider_changes_what_it_shows_without_input() {
    let odd = Fixture {
        shadows: Shadows {
            minutes: 5,
            month: 2,
            day: 29,
            year: 2024,
            light: 3,
            dark: 99,
            ..Default::default()
        },
        ..shadows_tab()
    };
    let mut h = tall(odd);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn a_year_that_is_no_number_says_so() {
    let mut h = tall(shadows_tab());
    assert_eq!(
        type_into(&mut h, "Year value", 0, "abc", egui::Key::Enter),
        []
    );
    h.get_by_label("Year must be a number");
}

#[test]
fn the_number_message_goes_on_escape_and_with_the_document() {
    let mut h = tall(shadows_tab());
    type_into(&mut h, "Latitude value", 0, "north", egui::Key::Enter);
    h.get_by_label("Latitude must be a number");
    h.state_mut().documents += 1;
    h.run_ok();
    assert!(h.query_by_label("Latitude must be a number").is_none());
    type_into(&mut h, "Latitude value", 0, "north", egui::Key::Enter);
    h.get_by_label("Latitude must be a number");
    // Entering the shown text again ends the edit and clears the message.
    type_into(&mut h, "Latitude value", 0, "40.0167", egui::Key::Enter);
    assert!(h.query_by_label("Latitude must be a number").is_none());
    type_into(&mut h, "Longitude value", 0, "west", egui::Key::Enter);
    h.get_by_label("Longitude must be a number");
    type_into(&mut h, "Latitude value", 0, "5", egui::Key::Escape);
    assert!(h.query_by_label("Longitude must be a number").is_none());
}

#[test]
fn text_left_in_a_field_when_it_goes_is_forgotten() {
    let mut h = tall(shadows_tab());
    h.get_by_label("Latitude value").click();
    h.run_ok();
    h.event(egui::Event::Text("12".into()));
    h.run_ok();
    // The tab changes while the field has text typed in it.
    h.state_mut().dock.tab = DockTab::Materials;
    h.run_ok();
    h.state_mut().dock.tab = DockTab::Shadows;
    h.run_ok();
    assert_eq!(
        h.get_by_label("Latitude value")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("40.0167")
    );
}

/// The distinct rows of the tab buttons named `names`: each row's (top, left, right).
fn tab_rows(h: &H, names: &[&str]) -> Vec<(f32, f32, f32)> {
    let mut rows: Vec<(f32, f32, f32)> = Vec::new();
    for name in names {
        let r = rect_of(h, name);
        match rows
            .iter_mut()
            .find(|(top, ..)| (*top - r.min.y).abs() < 2.0)
        {
            Some(row) => {
                row.1 = row.1.min(r.min.x);
                row.2 = row.2.max(r.max.x);
            }
            None => rows.push((r.min.y, r.min.x, r.max.x)),
        }
    }
    rows
}

#[test]
fn tabs_that_do_not_fit_wrap_into_balanced_justified_rows() {
    let all = [
        "Entity Info",
        "Materials",
        "Tags",
        "Scenes",
        "Joints",
        "Shadows",
        "Styles",
        "Library",
    ];
    let h = harness(Fixture::default());
    let rows = tab_rows(&h, &all);
    assert_eq!(rows.len(), 2, "{rows:?}");
    let count = |row: &(f32, f32, f32)| {
        all.iter()
            .filter(|n| (rect_of(&h, n).min.y - row.0).abs() < 2.0)
            .count()
    };
    assert_eq!([count(&rows[0]), count(&rows[1])], [4, 4]);
    assert!((rows[0].2 - rows[1].2).abs() <= 1.0, "justified: {rows:?}");
    assert!((rows[0].1 - rows[1].1).abs() <= 1.0, "{rows:?}");
    // Without the Library tab: four and three, both rows as wide as the dock.
    let h = harness(Fixture {
        features: crate::platform::Features::WEB,
        ..Default::default()
    });
    let five = &all[..7];
    let rows = tab_rows(&h, five);
    let count = |row: &(f32, f32, f32)| {
        five.iter()
            .filter(|n| (rect_of(&h, n).min.y - row.0).abs() < 2.0)
            .count()
    };
    assert_eq!([count(&rows[0]), count(&rows[1])], [4, 3], "{rows:?}");
    assert!((rows[0].2 - rows[1].2).abs() <= 1.0, "justified: {rows:?}");
}
