use super::*;
use crate::shortcuts::{Bound, chord};

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
        assert_eq!(click_menu(&mut h, &["File", item]), [action], "{item}");
    }
}

#[test]
fn the_view_menu_and_the_indicator_open_agent_activity() {
    let mut h = harness(Fixture::default());
    assert!(
        rect_of(&h, "View").min.x > rect_of(&h, "Camera").min.x,
        "View right of Camera"
    );
    assert_eq!(
        click_menu(&mut h, &["View", "Agent Activity"]),
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
    click_menu(&mut harness(f()), &["Camera", item])
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
    assert_eq!(camera_menu(plain, "Fly Mode"), [UiAction::ToggleFly]);
    assert_eq!(
        camera_menu(plain, "Night Lighting"),
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
    // Unchecking Night Lighting goes back to day.
    assert_eq!(
        camera_menu(parallel_night, "Night Lighting"),
        [UiAction::SetLighting(Lighting::Day)]
    );
}

/// A fixture for the Edit menu tests.
fn edit_fixture(has_selection: bool, has_guides: bool) -> Fixture {
    Fixture {
        has_selection,
        has_guides,
        ..Default::default()
    }
}

#[test]
fn the_edit_menu_erases_the_selection_and_deletes_guides() {
    for (item, action) in [
        ("Erase", UiAction::EraseSelection),
        ("Delete Guides", UiAction::DeleteGuides),
        ("Soften Edges...", UiAction::ShowSoften(true)),
    ] {
        let mut h = harness(edit_fixture(true, true));
        assert_eq!(click_menu(&mut h, &["Edit", item]), [action], "{item}");
    }
}

#[test]
fn the_edit_menu_items_need_a_selection_and_guides() {
    for item in ["Erase", "Delete Guides"] {
        let mut h = harness(edit_fixture(false, false));
        let there = rects_named(&h, item);
        click(&mut h, "Edit");
        assert!(
            new_node(&h, item, &there).accesskit_node().is_disabled(),
            "{item}"
        );
        // A disabled item does nothing.
        new_node(&h, item, &there).click();
        h.run_ok();
        assert_eq!(h.state_mut().take(), [], "{item}");
    }
    // Soften Edges... works on the open context.
    let mut h = harness(edit_fixture(false, false));
    assert_eq!(
        click_menu(&mut h, &["Edit", "Soften Edges..."]),
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

#[test]
fn menu_items_show_the_tables_keys() {
    let mut h = harness(Fixture::default());
    click(&mut h, "File");
    let texts = drawn_texts(&h);
    for bound in [
        Bound::NewModel,
        Bound::OpenFile,
        Bound::SaveFile,
        Bound::SaveFileAs,
        Bound::ImportFile,
        Bound::ExportFile,
    ] {
        let key = chord(bound).unwrap().text();
        assert!(texts.contains(&key), "{key} in {texts:?}");
    }
    let mut h = harness(Fixture::default());
    click(&mut h, "Camera");
    let fly = chord(Bound::ToggleFly).unwrap().text();
    assert!(drawn_texts(&h).contains(&fly), "{fly}");
    h.get_by_label("Fly Mode");
}

#[test]
fn checked_menu_items_show_the_state() {
    let checked = |f: Fixture, menu: &str, items: &[(&str, bool)]| {
        let mut h = harness(f);
        let there: Vec<_> = items.iter().map(|(i, _)| rects_named(&h, i)).collect();
        click(&mut h, menu);
        for ((item, want), there) in items.iter().zip(&there) {
            let on = new_node(&h, item, there)
                .accesskit_node()
                .toggled()
                .map(|t| t == egui::accesskit::Toggled::True);
            assert_eq!(on, Some(*want), "{item}");
        }
    };
    checked(
        Fixture {
            flying: true,
            lighting: Lighting::Night,
            projection: Projection::Parallel,
            ..Default::default()
        },
        "Camera",
        &[
            ("Fly Mode", true),
            ("Night Lighting", true),
            ("Perspective", false),
            ("Parallel Projection", true),
        ],
    );
    checked(
        Fixture::default(),
        "Camera",
        &[
            ("Fly Mode", false),
            ("Night Lighting", false),
            ("Perspective", true),
            ("Parallel Projection", false),
        ],
    );
    // Import Units is a submenu of File.
    let mut h = harness(Fixture {
        import_units: ImportUnits::Millimeters,
        ..Default::default()
    });
    click(&mut h, "File");
    click(&mut h, "Import Units \u{23F5}");
    for units in ImportUnits::ALL {
        assert_eq!(
            is_on(&h, units.label()),
            Some(units == ImportUnits::Millimeters),
            "{units:?}"
        );
    }
}

#[test]
fn tool_buttons_mark_the_active_tool_and_name_their_keys() {
    let mut h = harness(Fixture::default());
    assert_eq!(is_on(&h, "Line"), Some(true));
    assert_eq!(is_on(&h, "Move"), Some(false));
    h.get_by_label("Rectangle").hover();
    h.run_steps(20);
    let key = chord(Bound::Tool(ToolKind::Rectangle)).unwrap().text();
    assert!(drawn_texts(&h).contains(&key), "the tooltip's key chip");
}

#[test]
fn group_buttons_name_their_keys() {
    let mut h = harness(Fixture {
        has_selection: true,
        ..Default::default()
    });
    h.get_by_label("Make component").hover();
    h.run_steps(20);
    let key = chord(Bound::MakeComponent).unwrap().text();
    assert!(drawn_texts(&h).contains(&key), "{key}");
}

#[test]
fn undo_and_redo_are_disabled_when_empty_and_name_their_step() {
    let mut h = harness(Fixture::default());
    for label in ["Undo", "Redo"] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label}"
        );
        assert_eq!(click(&mut h, label), [], "{label}");
    }
    let mut h = harness(Fixture {
        undo: Some("push_pull".into()),
        ..Default::default()
    });
    assert!(!h.get_by_label("Undo").accesskit_node().is_disabled());
    assert!(h.get_by_label("Redo").accesskit_node().is_disabled());
    h.get_by_label("Undo").hover();
    h.run_steps(20);
    let texts = drawn_texts(&h);
    assert!(texts.iter().any(|t| t == "Undo Push pull"), "{texts:?}");
    assert!(
        texts.contains(&chord(Bound::Undo).unwrap().text()),
        "{texts:?}"
    );
    assert_eq!(click(&mut h, "Undo"), [UiAction::Undo]);
}

#[test]
fn history_labels_read_as_words() {
    use crate::ui::toolbar::step_name;
    assert_eq!(step_name("push_pull"), "Push pull");
    assert_eq!(step_name("batch"), "Batch");
    assert_eq!(step_name(""), "");
}

#[test]
fn the_view_menu_sets_the_theme() {
    // egui names a submenu's button with its arrow.
    const THEME: &str = "Theme \u{23F5}";
    for (item, choice) in [
        ("System", ThemeChoice::System),
        ("Dark", ThemeChoice::Dark),
        ("Light", ThemeChoice::Light),
    ] {
        let mut h = harness(Fixture::default());
        assert_eq!(
            click_menu(&mut h, &["View", THEME, item]),
            [UiAction::SetTheme(choice)],
            "{item}"
        );
    }
    let mut h = harness(Fixture {
        theme: ThemeChoice::Dark,
        ..Default::default()
    });
    click(&mut h, "View");
    click(&mut h, THEME);
    assert_eq!(is_on(&h, "Dark"), Some(true));
    assert_eq!(is_on(&h, "Light"), Some(false));
    assert_eq!(is_on(&h, "System"), Some(false));
}

#[test]
fn the_draw_tools_group_and_help_menus_report_their_items() {
    let f = || Fixture {
        has_selection: true,
        has_instance_selected: true,
        inside_instance: true,
        ..Default::default()
    };
    for kind in [
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Arc,
    ] {
        assert_eq!(
            click_menu(&mut harness(f()), &["Draw", kind.label()]),
            [UiAction::Tool(kind)]
        );
    }
    for kind in [
        ToolKind::Select,
        ToolKind::PushPull,
        ToolKind::Move,
        ToolKind::Rotate,
        ToolKind::TapeMeasure,
        ToolKind::Erase,
        ToolKind::Paint,
    ] {
        assert_eq!(
            click_menu(&mut harness(f()), &["Tools", kind.label()]),
            [UiAction::Tool(kind)]
        );
    }
    for (item, action) in [
        ("Make Group", UiAction::MakeGroup),
        ("Make Component", UiAction::MakeComponent),
        ("Explode", UiAction::Explode),
        ("Close Group", UiAction::CloseInstance),
    ] {
        assert_eq!(
            click_menu(&mut harness(f()), &["Group", item]),
            [action],
            "{item}"
        );
    }
}

#[test]
fn help_opens_a_window_listing_every_binding() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click_menu(&mut h, &["Help", "Keyboard Shortcuts"]),
        [UiAction::ShowShortcuts(true)]
    );
    // A window tall enough to draw every row.
    let tall = Vec2::new(1280.0, 1600.0);
    let mut h = harness_sized(
        Fixture {
            shortcuts: true,
            ..Default::default()
        },
        tall,
    );
    let texts = drawn_texts(&h);
    for s in crate::shortcuts::TABLE {
        assert!(texts.iter().any(|t| t == s.bound.label()), "{:?}", s.bound);
        assert!(texts.contains(&s.chord.text()), "{:?} {texts:?}", s.bound);
    }
    for group in crate::shortcuts::GROUPS {
        assert!(texts.iter().any(|t| *t == group.to_uppercase()), "{group}");
    }
    assert_eq!(
        click(&mut h, "Close window"),
        [UiAction::ShowShortcuts(false)]
    );
}

#[test]
fn the_shortcuts_window_fits_a_short_window() {
    for height in [600.0, 400.0] {
        let size = Vec2::new(960.0, height);
        let mut h = harness_sized(
            Fixture {
                shortcuts: true,
                ..Default::default()
            },
            size,
        );
        let screen = Rect::from_min_size(Pos2::ZERO, size);
        let close = rect_of(&h, "Close window");
        assert!(screen.contains_rect(close), "the close button: {close:?}");
        if height < 500.0 {
            assert!(
                !drawn_texts(&h).iter().any(|t| t == "Fly Mode"),
                "Fly Mode is below the fold before scrolling"
            );
        }
        // Short of room for every row: the list scrolls to the last one.
        h.event(egui::Event::PointerMoved(
            close.center() + Vec2::new(0.0, 100.0),
        ));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, -2000.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_ok();
        assert!(
            drawn_texts(&h).iter().any(|t| t == "Fly Mode"),
            "Fly Mode at {height}"
        );
    }
}

#[test]
fn the_shortcuts_window_can_be_dragged_by_its_title() {
    let mut h = harness(Fixture {
        shortcuts: true,
        ..Default::default()
    });
    let before = rect_of(&h, "Close window");
    // The node named for the window covers it all; its title bar is the top strip.
    let from = h.get_by_label("Keyboard Shortcuts").rect().center_top() + Vec2::new(0.0, 12.0);
    let to = from + Vec2::new(-120.0, 60.0);
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_ok();
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(
            from + (to - from) * (k as f32 / 4.0),
        ));
        h.run_ok();
    }
    h.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_ok();
    let moved = rect_of(&h, "Close window").min - before.min;
    assert!(
        (moved - Vec2::new(-120.0, 60.0)).length() < 2.0,
        "{moved:?}"
    );
}

#[test]
fn the_edit_menu_names_the_undo_and_redo_steps() {
    let mut h = harness(Fixture {
        undo: Some("push_pull".into()),
        ..Default::default()
    });
    assert_eq!(
        click_menu(&mut h, &["Edit", "Undo Push pull"]),
        [UiAction::Undo]
    );
    let mut h = harness(Fixture::default());
    // Nothing to redo: the item reads "Redo" and does nothing.
    assert_eq!(click_menu(&mut h, &["Edit", "Redo"]), []);
}

#[test]
fn quit_ends_the_file_menu_off_macos_and_leads_the_app_menu_on_it() {
    use crate::menus::{APP_MENU, Entry, items, menu_bar};
    let f = Fixture::default();
    let egui = menu_bar(&f.view(), false);
    let file = &egui.iter().find(|m| m.title == "File").unwrap().entries;
    assert!(matches!(file.last(), Some(Entry::Item(i)) if i.action == UiAction::Quit));
    assert!(egui.iter().all(|m| m.title != APP_MENU));
    let native = menu_bar(&f.view(), true);
    assert_eq!(native[0].title, APP_MENU);
    let quits = items(&native)
        .iter()
        .filter(|i| i.action == UiAction::Quit)
        .count();
    assert_eq!(quits, 1);
    assert_eq!(
        click_menu(&mut harness(Fixture::default()), &["File", "Quit"]),
        [UiAction::Quit]
    );
}

#[test]
fn the_menu_bar_has_the_same_items_in_every_state() {
    use crate::menus::{item_id, items, menu_bar};
    let ids = |f: &Fixture| {
        items(&menu_bar(&f.view(), true))
            .iter()
            .map(|i| item_id(&i.action))
            .collect::<Vec<_>>()
    };
    let empty = ids(&Fixture::default());
    let busy = |lighting| Fixture {
        undo: Some("paint".into()),
        redo: Some("erase".into()),
        has_selection: true,
        has_instance_selected: true,
        inside_instance: true,
        has_guides: true,
        flying: true,
        lighting,
        ..Default::default()
    };
    assert_eq!(
        empty,
        ids(&busy(Lighting::Day)),
        "native items are built once and only synced"
    );
    assert_eq!(empty, ids(&busy(Lighting::Night)), "also at night");
    let unique: std::collections::BTreeSet<_> = empty.iter().collect();
    assert_eq!(unique.len(), empty.len(), "ids are unique");
}

#[test]
fn no_egui_menus_when_the_window_has_native_ones() {
    let h = harness(Fixture {
        egui_menus: false,
        ..Default::default()
    });
    for title in [
        "File", "Edit", "Draw", "Tools", "Group", "Camera", "View", "Help",
    ] {
        assert!(h.query_by_label(title).is_none(), "{title}");
    }
}
