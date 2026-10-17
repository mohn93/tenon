use super::*;

#[test]
fn the_breadcrumb_opens_each_level_and_the_last_is_not_a_button() {
    let f = || house_fixture(&["i1", "i3"]); // Model > House > Kitchen
    let h = harness(f());
    h.get_by_label("Editing Kitchen");
    assert!(h.query_by_label("Go to Kitchen").is_none());
    assert_eq!(
        click(&mut harness(f()), "Go to Model"),
        [UiAction::OpenPath(vec![])]
    );
    assert_eq!(
        click(&mut harness(f()), "Go to House"),
        [UiAction::OpenPath(ids(&["i1"]))]
    );
    harness(Fixture::default()).get_by_label("Editing Model");
}

#[test]
fn a_deep_path_folds_the_oldest_crumbs() {
    use crate::ui::context_bar::fold_crumbs;
    assert_eq!(
        fold_crumbs(&[50.0, 60.0, 70.0], 500.0, 20.0),
        [Some(0), Some(1), Some(2)]
    );
    // Model and the last always show; the oldest after Model fold first.
    assert_eq!(
        fold_crumbs(&[50.0, 60.0, 70.0, 80.0], 220.0, 20.0),
        [Some(0), None, Some(2), Some(3)]
    );
    assert_eq!(
        fold_crumbs(&[50.0, 60.0, 70.0, 80.0], 10.0, 20.0),
        [Some(0), None, Some(3)]
    );
    // Two crumbs or fewer never fold.
    assert_eq!(fold_crumbs(&[50.0, 60.0], 10.0, 20.0), [Some(0), Some(1)]);
    assert_eq!(fold_crumbs(&[50.0], 10.0, 20.0), [Some(0)]);
}

#[test]
fn long_crumb_names_are_cut() {
    let mut f = house_fixture(&["i1"]);
    f.outliner.roots[0].name = "A house whose name runs on and on".into();
    let h = harness(f);
    h.get_by_label("Editing A house whose name runs…");
}

#[test]
fn group_buttons_show_only_when_they_apply() {
    let h = harness(Fixture::default());
    for label in ["Make Group", "Make Component", "Explode"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
    let mut h = harness(Fixture {
        has_selection: true,
        ..Default::default()
    });
    assert!(h.query_by_label("Explode").is_none());
    assert_eq!(click(&mut h, "Make Group"), [UiAction::MakeGroup]);
    let mut h = harness(Fixture {
        has_selection: true,
        has_instance_selected: true,
        ..Default::default()
    });
    assert_eq!(click(&mut h, "Explode"), [UiAction::Explode]);
}

#[test]
fn the_day_night_toggle_switches_the_lighting() {
    let mut h = harness(Fixture::default());
    assert_eq!(is_on(&h, "Night Lighting"), Some(false));
    assert_eq!(
        click(&mut h, "Night Lighting"),
        [UiAction::SetLighting(Lighting::Night)]
    );
    let mut h = harness(Fixture {
        lighting: Lighting::Night,
        ..Default::default()
    });
    assert_eq!(is_on(&h, "Night Lighting"), Some(true));
    assert_eq!(
        click(&mut h, "Night Lighting"),
        [UiAction::SetLighting(Lighting::Day)]
    );
}

#[test]
fn the_context_bar_is_one_row_at_960() {
    let deep = || {
        let mut f = house_fixture(&["i1", "i3"]);
        f.has_selection = true;
        f.has_instance_selected = true;
        f.connected = 3;
        f
    };
    for width in [960.0, 1280.0] {
        let h = harness_sized(deep(), Vec2::new(width, 600.0));
        assert_eq!(
            h.state().central.min.y,
            theme::CONTEXT_BAR_HEIGHT,
            "{width}"
        );
        for label in [
            "Undo",
            "Make Group",
            "Explode",
            "Night Lighting",
            "Agent Activity",
            "Editing Kitchen",
        ] {
            let r = rect_of(&h, label);
            assert!(
                r.max.y <= theme::CONTEXT_BAR_HEIGHT && r.max.x <= width,
                "{label} at {width}: {r:?}"
            );
        }
    }
}
