use super::*;

#[test]
fn the_view_menu_toggles_the_section_display() {
    for (item, action) in [
        ("Section Planes", UiAction::SetSectionPlanes(false)),
        ("Section Cuts", UiAction::SetSectionCuts(false)),
        ("Section Fill", UiAction::SetSectionFill(false)),
    ] {
        let mut h = harness(Fixture::default());
        assert_eq!(click_menu(&mut h, &["View", item]), [action], "{item}");
    }
    // Each is checked while on; turning one off offers turning it on.
    let off = SectionDisplay {
        planes: true,
        cuts: false,
        fill: false,
    };
    let mut h = harness(Fixture {
        section_display: off,
        ..Default::default()
    });
    assert_eq!(
        click_menu(&mut h, &["View", "Section Cuts"]),
        [UiAction::SetSectionCuts(true)]
    );
    let bar = crate::menus::menu_bar(
        &Fixture {
            section_display: off,
            ..Default::default()
        }
        .view(),
        false,
    );
    let view = &bar.iter().find(|m| m.title == "View").unwrap().entries;
    let checked: Vec<(String, Option<bool>)> = view
        .iter()
        .filter_map(|e| match e {
            crate::menus::Entry::Item(i) if i.label.starts_with("Section ") => {
                Some((i.label.clone(), i.checked))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        checked,
        [
            ("Section Planes".to_string(), Some(true)),
            ("Section Cuts".to_string(), Some(false)),
            ("Section Fill".to_string(), Some(false)),
        ]
    );
}

#[test]
fn the_section_plane_tool_is_in_the_tools_menu_and_on_the_rail() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click_menu(&mut h, &["Tools", "Section Plane"]),
        [UiAction::Tool(ToolKind::SectionPlane)]
    );
    let h = harness(Fixture {
        tool: ToolKind::SectionPlane,
        ..Default::default()
    });
    assert_eq!(is_on(&h, "Section Plane"), Some(true));
}

#[test]
fn the_section_plane_menu_flips_and_toggles_the_active_cut() {
    let open = |active: bool| Fixture {
        viewport_menu: Some(ViewportMenu {
            at: Pos2::new(300.0, 200.0),
            section: Some(active),
            intersect: false,
            hide: false,
            unhide: false,
            faces: false,
            orient_from: None,
        }),
        ..Default::default()
    };
    assert_eq!(
        click(&mut harness(open(true)), "Flip"),
        [UiAction::FlipSection, UiAction::CloseViewportMenu]
    );
    // Checked while the plane is the active cut; clicking clears it.
    let mut h = harness(open(true));
    assert_eq!(is_on(&h, "Active Cut"), Some(true));
    assert_eq!(
        click(&mut h, "Active Cut"),
        [
            UiAction::SetSectionActive(false),
            UiAction::CloseViewportMenu
        ]
    );
    let mut h = harness(open(false));
    assert_eq!(is_on(&h, "Active Cut"), Some(false));
    assert_eq!(
        click(&mut h, "Active Cut"),
        [
            UiAction::SetSectionActive(true),
            UiAction::CloseViewportMenu
        ]
    );
    // No menu open: no such items.
    assert!(
        harness(Fixture::default())
            .query_by_label("Active Cut")
            .is_none()
    );
}

#[test]
fn the_faces_and_groups_menu_intersects_with_each_scope() {
    let open = || Fixture {
        viewport_menu: Some(ViewportMenu {
            at: Pos2::new(300.0, 200.0),
            section: None,
            intersect: true,
            hide: false,
            unhide: false,
            faces: false,
            orient_from: None,
        }),
        ..Default::default()
    };
    for (item, scope) in [
        ("With Model", IntersectScope::Model),
        ("With Selection", IntersectScope::Selection),
        ("With Context", IntersectScope::Context),
    ] {
        assert_eq!(
            click(&mut harness(open()), item),
            [UiAction::IntersectFaces(scope), UiAction::CloseViewportMenu],
            "{item}"
        );
    }
    // It is not the section plane's menu, and that one has no intersect items.
    let h = harness(open());
    for item in ["Flip", "Active Cut"] {
        assert!(h.query_by_label(item).is_none(), "{item}");
    }
    let h = harness(Fixture {
        viewport_menu: Some(ViewportMenu {
            at: Pos2::new(300.0, 200.0),
            section: Some(false),
            intersect: false,
            hide: false,
            unhide: false,
            faces: false,
            orient_from: None,
        }),
        ..Default::default()
    });
    assert!(h.query_by_label("With Model").is_none());
}
