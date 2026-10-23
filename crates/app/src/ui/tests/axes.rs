use super::*;
use crate::menus::{Entry, items, menu_bar};
use session::{FlipAxis, FlipFrame};

// egui names a submenu's button with its arrow.
const FLIP_ALONG: &str = "Flip Along \u{23F5}";

/// The Group menu's items (submenus flattened) with their enabled flags, by label.
fn group_items(f: &Fixture) -> Vec<(String, bool)> {
    fn walk(entries: &[Entry], out: &mut Vec<(String, bool)>) {
        for e in entries {
            match e {
                Entry::Item(i) => out.push((i.label.clone(), i.enabled)),
                Entry::Submenu { entries, .. } => walk(entries, out),
                Entry::Separator => {}
            }
        }
    }
    let bar = menu_bar(&f.view(), false);
    let group = bar.iter().find(|m| m.title == "Group").unwrap();
    let mut out = Vec::new();
    walk(&group.entries, &mut out);
    out
}

fn enabled(f: &Fixture, label: &str) -> bool {
    group_items(f)
        .into_iter()
        .find(|(l, _)| l == label)
        .unwrap_or_else(|| panic!("no {label}"))
        .1
}

#[test]
fn the_axes_tool_is_in_the_tools_menu_and_on_the_rail() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click_menu(&mut h, &["Tools", "Axes"]),
        [UiAction::Tool(ToolKind::Axes)]
    );
    let h = harness(Fixture {
        tool: ToolKind::Axes,
        ..Default::default()
    });
    assert_eq!(is_on(&h, "Axes"), Some(true));
}

#[test]
fn make_unique_and_flip_follow_the_selection() {
    let none = Fixture::default();
    assert!(!enabled(&none, "Make Unique"));
    for label in ["Red", "Green", "Blue"] {
        assert!(!enabled(&none, label), "{label}");
        assert!(!enabled(&none, &format!("Component's {label}")));
    }
    let selected = Fixture {
        has_selection: true,
        ..Default::default()
    };
    assert!(!enabled(&selected, "Make Unique"));
    assert!(enabled(&selected, "Red") && enabled(&selected, "Blue"));
    assert!(!enabled(&selected, "Component's Red"));
    let one = Fixture {
        has_selection: true,
        can_make_unique: true,
        can_flip_own: true,
        ..Default::default()
    };
    assert!(enabled(&one, "Make Unique"));
    assert!(enabled(&one, "Component's Green"));
}

#[test]
fn the_group_menu_runs_make_unique_and_flip() {
    let f = || Fixture {
        has_selection: true,
        can_make_unique: true,
        can_flip_own: true,
        ..Default::default()
    };
    assert_eq!(
        click_menu(&mut harness(f()), &["Group", "Make Unique"]),
        [UiAction::MakeUnique]
    );
    assert_eq!(
        click_menu(&mut harness(f()), &["Group", FLIP_ALONG, "Green"]),
        [UiAction::Flip(FlipAxis::Green, FlipFrame::Axes)]
    );
    assert_eq!(
        click_menu(
            &mut harness(f()),
            &["Group", FLIP_ALONG, "Component's Blue"]
        ),
        [UiAction::Flip(FlipAxis::Blue, FlipFrame::Own)]
    );
}

#[test]
fn the_viewport_menu_runs_make_unique_flip_and_reset_axes() {
    let menu = ViewportMenu {
        make_unique: true,
        flip: true,
        flip_own: true,
        reset_axes: true,
        ..Default::default()
    };
    let close = UiAction::CloseViewportMenu;
    for (item, action) in [
        ("Make Unique", UiAction::MakeUnique),
        ("Red", UiAction::Flip(FlipAxis::Red, FlipFrame::Axes)),
        (
            "Component's Green",
            UiAction::Flip(FlipAxis::Green, FlipFrame::Own),
        ),
        ("Reset Axes", UiAction::ResetAxes),
    ] {
        let f = Fixture {
            viewport_menu: Some(menu),
            ..Default::default()
        };
        assert_eq!(
            click(&mut harness(f), item),
            [action, close.clone()],
            "{item}"
        );
    }
    // Each shows only where it applies.
    let h = harness(Fixture {
        viewport_menu: Some(ViewportMenu {
            flip: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    assert!(h.query_by_label("Red").is_some());
    for item in ["Make Unique", "Component's Red", "Reset Axes"] {
        assert!(h.query_by_label(item).is_none(), "{item}");
    }
}

#[test]
fn the_native_menu_ids_of_the_new_items_are_distinct_and_stable() {
    let f = Fixture {
        has_selection: true,
        ..Default::default()
    };
    let bar = menu_bar(&f.view(), true);
    let ids: Vec<String> = items(&bar)
        .iter()
        .map(|i| crate::menus::item_id(&i.action))
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "ids repeat");
    for id in [
        "MakeUnique",
        "Flip(Red, Axes)",
        "Flip(Blue, Own)",
        "Tool(Axes)",
    ] {
        assert!(ids.iter().any(|i| i == id), "{id} in {ids:?}");
    }
}
