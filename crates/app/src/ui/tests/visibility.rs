use super::scenes::right_click;
use super::*;
use kernel::DefinitionKind;

#[test]
fn edit_hide_needs_something_still_shown() {
    let mut h = harness(Fixture {
        selection_hidden: Some(true),
        ..Default::default()
    });
    click(&mut h, "Edit");
    assert!(h.get_by_label("Hide").accesskit_node().is_disabled());
}

#[test]
fn edit_unhide_last_and_all_need_something_hidden() {
    const UNHIDE: &str = "Unhide \u{23F5}";
    let mut h = harness(Fixture::default());
    click(&mut h, "Edit");
    click(&mut h, UNHIDE);
    assert!(h.get_by_label("Last").accesskit_node().is_disabled());
    assert!(h.get_by_label("All").accesskit_node().is_disabled());
    let mut h = harness(Fixture {
        has_hidden: true,
        has_last_hidden: true,
        ..Default::default()
    });
    click(&mut h, "Edit");
    click(&mut h, UNHIDE);
    assert!(!h.get_by_label("Last").accesskit_node().is_disabled());
    assert!(!h.get_by_label("All").accesskit_node().is_disabled());
}

#[test]
fn orient_faces_from_the_menu_uses_the_face_under_the_cursor_when_there_is_one() {
    let face: EntityId = "f4".parse().unwrap();
    let menu = ViewportMenu {
        faces: true,
        orient_from: Some(face),
        ..plain_menu()
    };
    assert_eq!(
        click(&mut harness(open_menu(menu)), "Orient Faces"),
        [
            UiAction::OrientFaces(Some(face)),
            UiAction::CloseViewportMenu
        ]
    );
    let mut h = harness(Fixture {
        has_face_selected: true,
        ..Default::default()
    });
    assert_eq!(
        click_menu(&mut h, &["Edit", "Orient Faces"]),
        [UiAction::OrientFaces(None)]
    );
}

#[test]
fn edit_hide_needs_something_that_can_be_hidden() {
    let mut h = harness(Fixture {
        selection_hidden: Some(false),
        ..Default::default()
    });
    assert_eq!(click_menu(&mut h, &["Edit", "Hide"]), [UiAction::Hide]);
    let mut h = harness(Fixture::default());
    click(&mut h, "Edit");
    assert!(h.get_by_label("Hide").accesskit_node().is_disabled());
}

#[test]
fn edit_unhide_has_selected_last_and_all() {
    const UNHIDE: &str = "Unhide \u{23F5}";
    for (item, action) in [
        ("Last", UiAction::UnhideLast),
        ("All", UiAction::UnhideAll),
        ("Selected", UiAction::UnhideSelected),
    ] {
        let mut h = harness(Fixture {
            has_hidden_selected: true,
            has_hidden: true,
            has_last_hidden: true,
            ..Default::default()
        });
        assert_eq!(click_menu(&mut h, &["Edit", UNHIDE, item]), [action]);
    }
    // Selected needs a hidden entity in the selection.
    let mut h = harness(Fixture::default());
    click(&mut h, "Edit");
    click(&mut h, UNHIDE);
    assert!(h.get_by_label("Selected").accesskit_node().is_disabled());
}

#[test]
fn edit_reverse_and_orient_faces_need_a_face() {
    for (item, action) in [
        ("Reverse Faces", UiAction::ReverseFaces),
        ("Orient Faces", UiAction::OrientFaces(None)),
    ] {
        let mut h = harness(Fixture {
            has_face_selected: true,
            ..Default::default()
        });
        assert_eq!(click_menu(&mut h, &["Edit", item]), [action], "{item}");
        let mut h = harness(Fixture::default());
        click(&mut h, "Edit");
        assert!(
            h.get_by_label(item).accesskit_node().is_disabled(),
            "{item}"
        );
    }
}

#[test]
fn view_has_x_ray_and_hidden_geometry_toggles() {
    for (item, on, wanted) in [
        ("X-ray", false, UiAction::SetXRay(true)),
        ("X-ray", true, UiAction::SetXRay(false)),
        ("Hidden Geometry", false, UiAction::SetHiddenGeometry(true)),
        ("Hidden Geometry", true, UiAction::SetHiddenGeometry(false)),
    ] {
        let fixture = Fixture {
            x_ray: on && item == "X-ray",
            hidden_geometry: on && item == "Hidden Geometry",
            ..Default::default()
        };
        let mut h = harness(fixture);
        click(&mut h, "View");
        assert_eq!(is_on(&h, item), Some(on), "{item}");
        assert_eq!(click(&mut h, item), [wanted], "{item}");
    }
}

#[test]
fn the_new_items_have_stable_native_ids() {
    use crate::menus::item_id;
    assert_eq!(item_id(&UiAction::Hide), "Hide");
    assert_eq!(item_id(&UiAction::UnhideAll), "UnhideAll");
    assert_eq!(item_id(&UiAction::ReverseFaces), "ReverseFaces");
    assert_eq!(
        item_id(&UiAction::SetXRay(true)),
        item_id(&UiAction::SetXRay(false))
    );
    assert_eq!(
        item_id(&UiAction::SetHiddenGeometry(true)),
        item_id(&UiAction::SetHiddenGeometry(false))
    );
    // Every native id is unique across the whole bar.
    let bar = crate::menus::menu_bar(&Fixture::default().view(), true);
    let mut seen = std::collections::BTreeSet::new();
    for item in crate::menus::items(&bar) {
        let id = item_id(&item.action);
        assert!(seen.insert(id.clone()), "{id} twice");
    }
}

#[test]
fn entity_info_shows_a_hidden_checkbox_for_the_selection() {
    for (hidden, wanted) in [
        (false, UiAction::SetHidden(true)),
        (true, UiAction::SetHidden(false)),
    ] {
        let mut h = harness(Fixture {
            info: vec!["1 face".into()],
            selection_hidden: Some(hidden),
            ..Default::default()
        });
        assert_eq!(click(&mut h, "Hidden"), [wanted]);
    }
    assert!(
        harness(Fixture::default())
            .query_by_label("Hidden")
            .is_none()
    );
}

fn open_menu(menu: ViewportMenu) -> Fixture {
    Fixture {
        viewport_menu: Some(menu),
        ..Default::default()
    }
}

fn plain_menu() -> ViewportMenu {
    ViewportMenu {
        at: Pos2::new(300.0, 200.0),
        section: None,
        intersect: false,
        hide: false,
        unhide: false,
        faces: false,
        orient_from: None,
    }
}

#[test]
fn the_viewport_menu_hides_unhides_reverses_and_orients() {
    let all = ViewportMenu {
        hide: true,
        unhide: true,
        faces: true,
        ..plain_menu()
    };
    for (item, action) in [
        ("Hide", UiAction::Hide),
        ("Unhide", UiAction::UnhideSelected),
        ("Reverse Faces", UiAction::ReverseFaces),
        ("Orient Faces", UiAction::OrientFaces(None)),
    ] {
        assert_eq!(
            click(&mut harness(open_menu(all)), item),
            [action, UiAction::CloseViewportMenu],
            "{item}"
        );
    }
    // Each entry shows only where it applies.
    let h = harness(open_menu(ViewportMenu {
        hide: true,
        ..plain_menu()
    }));
    assert!(h.query_by_label("Hide").is_some());
    for item in ["Unhide", "Reverse Faces", "Orient Faces"] {
        assert!(h.query_by_label(item).is_none(), "{item}");
    }
}

#[test]
fn an_outliner_row_hides_and_unhides_from_its_context_menu() {
    let rows = |own_hidden: bool| {
        let mut row = table_row("i1", "d1", DefinitionKind::Group, "House");
        row.own_hidden = own_hidden;
        row.hidden = own_hidden;
        Fixture {
            outliner: OutlinerTable {
                roots: vec![row],
                by_definition: [("d1".parse().unwrap(), Vec::new())].into(),
            },
            ..Default::default()
        }
    };
    let mut h = harness(rows(false));
    right_click(&mut h, "House");
    assert_eq!(h.state_mut().take(), []);
    assert_eq!(
        click(&mut h, "Hide"),
        [UiAction::OutlinerSetHidden {
            path: ids(&["i1"]),
            hidden: true
        }]
    );
    let mut h = harness(rows(true));
    right_click(&mut h, "House");
    assert_eq!(
        click(&mut h, "Unhide"),
        [UiAction::OutlinerSetHidden {
            path: ids(&["i1"]),
            hidden: false
        }]
    );
}
