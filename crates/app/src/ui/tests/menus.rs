use super::*;

#[test]
fn the_file_menu_reports_its_items() {
    for (item, action) in [
        ("New", UiAction::NewModel),
        ("Open...", UiAction::OpenFile),
        ("Save", UiAction::SaveFile),
        ("Save As...", UiAction::SaveFileAs),
        ("Import...", UiAction::ImportFile),
        ("Export...", UiAction::ExportFile),
    ] {
        let mut h = harness(Fixture::default());
        click(&mut h, "File");
        assert_eq!(click(&mut h, item), [action], "{item}");
    }
}

#[test]
fn the_view_menu_and_the_indicator_open_agent_activity() {
    let mut h = harness(Fixture::default());
    assert!(
        rect_of(&h, "View").min.x > rect_of(&h, "Camera").min.x,
        "View right of Camera"
    );
    click(&mut h, "View");
    assert_eq!(
        click(&mut h, "Agent Activity"),
        [UiAction::ShowAgentActivity(true)]
    );

    // Status bar: nothing at 0 agents, "Agents: 2" at 2, and a click on it opens the log.
    let h = harness(Fixture::default());
    assert!(h.query_by_label_contains("Agents:").is_none());
    let mut h = harness(Fixture {
        connected: 2,
        ..Default::default()
    });
    let at = rect_of(&h, "Agents: 2");
    assert!(
        at.min.y >= h.state().central.max.y,
        "in the status bar: {at:?}"
    );
    assert_eq!(
        click(&mut h, "Agents: 2"),
        [UiAction::ShowAgentActivity(true)]
    );

    // The window lists the log and the recovery files, and closes.
    let mut h = harness(Fixture {
        connected: 2,
        log: Some(vec!["5 s ago  #1  execute save /a.skr  ok".into()]),
        recovered: vec![PathBuf::from("/r/untitled-1.skr")],
        ..Default::default()
    });
    for want in [
        "2 agents connected",
        "5 s ago  #1  execute save /a.skr  ok",
        "/r/untitled-1.skr",
    ] {
        h.get_by_label(want);
    }
    assert_eq!(
        click(&mut h, "Close window"),
        [UiAction::ShowAgentActivity(false)]
    );
}

/// Opens the Camera menu over a fixture from `f` and clicks `item`.
fn camera_menu(f: impl Fn() -> Fixture, item: &str) -> Vec<UiAction> {
    let mut h = harness(f());
    click(&mut h, "Camera");
    click(&mut h, item)
}

#[test]
fn the_camera_menu_has_views_zoom_extents_and_projections() {
    use session::StandardView;
    let plain = Fixture::default;
    assert_eq!(camera_menu(plain, "Zoom Extents"), [UiAction::ZoomExtents]);
    for v in StandardView::ALL {
        assert_eq!(
            camera_menu(plain, v.label()),
            [UiAction::StandardView(v)],
            "{v:?}"
        );
    }
    assert_eq!(
        camera_menu(plain, "Parallel Projection"),
        [UiAction::SetProjection(Projection::Parallel)]
    );
    // Picking the projection already in use still reports it.
    assert_eq!(
        camera_menu(plain, "Perspective"),
        [UiAction::SetProjection(Projection::Perspective)]
    );
    assert_eq!(camera_menu(plain, "Fly mode (F)"), [UiAction::ToggleFly]);
    assert_eq!(
        camera_menu(plain, "Night lighting"),
        [UiAction::SetLighting(Lighting::Night)]
    );
    let parallel_night = || Fixture {
        projection: Projection::Parallel,
        lighting: Lighting::Night,
        ..Default::default()
    };
    assert_eq!(
        camera_menu(parallel_night, "Perspective"),
        [UiAction::SetProjection(Projection::Perspective)]
    );
    // Unchecking Night lighting goes back to day.
    assert_eq!(
        camera_menu(parallel_night, "Night lighting"),
        [UiAction::SetLighting(Lighting::Day)]
    );
}

/// A harness with the Edit menu open.
fn edit_menu(has_selection: bool, has_guides: bool) -> H {
    let mut h = harness(Fixture {
        has_selection,
        has_guides,
        ..Default::default()
    });
    click(&mut h, "Edit");
    h
}

#[test]
fn the_edit_menu_erases_the_selection_and_deletes_guides() {
    for (item, action) in [
        ("Erase Selection", UiAction::EraseSelection),
        ("Delete Guides", UiAction::DeleteGuides),
        ("Soften Edges...", UiAction::ShowSoften(true)),
    ] {
        let mut h = edit_menu(true, true);
        assert_eq!(click(&mut h, item), [action], "{item}");
    }
}

#[test]
fn the_edit_menu_items_need_a_selection_and_guides() {
    for item in ["Erase Selection", "Delete Guides"] {
        let mut h = edit_menu(false, false);
        assert!(
            h.get_by_label(item).accesskit_node().is_disabled(),
            "{item}"
        );
        assert_eq!(click(&mut h, item), [], "{item}");
    }
    // Soften Edges... works on the open context.
    let mut h = edit_menu(false, false);
    assert_eq!(
        click(&mut h, "Soften Edges..."),
        [UiAction::ShowSoften(true)]
    );
}

#[test]
fn the_soften_window_sets_its_angle_softens_hardens_and_closes() {
    let open = || Fixture {
        soften: Some(20),
        ..Default::default()
    };
    for (label, action) in [
        ("Soften", UiAction::Soften),
        ("Harden", UiAction::Harden),
        ("Close window", UiAction::ShowSoften(false)),
    ] {
        let mut h = harness(open());
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    // A click near the slider's left end sets a small angle, three quarters along a large one.
    let mut angles = Vec::new();
    for along in [0.04, 0.75] {
        let mut h = harness(open());
        let slider = rect_of(&h, "Angle");
        let width = h.ctx.global_style().spacing.slider_width;
        let at = Pos2::new(slider.min.x + along * width, slider.center().y);
        for a in click_at(&mut h, at, egui::Modifiers::NONE) {
            if let UiAction::SetSoftenAngle(angle) = a {
                angles.push(angle);
            }
        }
    }
    assert!(angles.iter().any(|a| *a < 20), "{angles:?}");
    assert!(angles.iter().any(|a| *a > 90), "{angles:?}");
    assert!(angles.iter().all(|a| *a <= 180), "{angles:?}");
    // Closed: no window to click.
    let h = harness(Fixture::default());
    for label in ["Soften", "Harden", "Angle"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
}
