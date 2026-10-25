use super::*;
use crate::shortcuts::{Bound, chord};

#[test]
fn the_file_menu_reports_its_items() {
    for (item, action) in [
        ("New", UiAction::NewModel),
        ("Open...", UiAction::OpenFile),
        ("Recover Models...", UiAction::ShowRecover(true)),
        ("Save", UiAction::SaveFile),
        ("Save As...", UiAction::SaveFileAs),
        ("Import...", UiAction::ImportFile),
        ("Export...", UiAction::ExportFile),
        ("Export PDF...", UiAction::ExportPdf),
        ("Open on Phone...", UiAction::OpenOnPhone),
        ("Print...", UiAction::Print),
    ] {
        let mut h = harness(Fixture::default());
        assert_eq!(click_menu(&mut h, &["File", item]), [action], "{item}");
    }
    // Open Recent and Recover Models... sit between Open... and Save.
    let bar = crate::menus::menu_bar(&Fixture::default().view(), false);
    let file = &bar.iter().find(|m| m.title == "File").unwrap().entries;
    let labels: Vec<&str> = file
        .iter()
        .take(5)
        .map(|entry| match entry {
            crate::menus::Entry::Item(i) => i.label.as_str(),
            crate::menus::Entry::Submenu { label, .. } => label,
            crate::menus::Entry::Separator => "-",
        })
        .collect();
    assert_eq!(
        labels,
        ["New", "Open...", "Open Recent", "Recover Models...", "Save"]
    );
}

#[test]
fn the_file_menu_leaves_out_what_the_system_does_not_offer() {
    let labels = |features| {
        let f = Fixture {
            features,
            ..Fixture::default()
        };
        let bar = crate::menus::menu_bar(&f.view(), false);
        let file = &bar.iter().find(|m| m.title == "File").unwrap().entries;
        file.iter()
            .filter_map(|entry| match entry {
                crate::menus::Entry::Item(i) => Some(i.label.clone()),
                crate::menus::Entry::Submenu { label, .. } => Some((*label).to_string()),
                crate::menus::Entry::Separator => None,
            })
            .collect::<Vec<_>>()
    };
    let all = labels(crate::platform::Features::ALL);
    for gone in [
        "Open Recent",
        "Recover Models...",
        "Export PDF...",
        "Save Model to Library...",
        "Print...",
        "Open on Phone...",
        "Quit",
    ] {
        assert!(all.iter().any(|l| l == gone), "{gone} is in the desktop's");
    }
    let web = labels(crate::platform::Features::WEB);
    assert_eq!(
        web,
        [
            "New",
            "Open...",
            "Save",
            "Save As...",
            "Import...",
            "Export..."
        ]
    );
}

// egui names a submenu's button with its arrow.
const OPEN_RECENT: &str = "Open Recent \u{23F5}";
const INTERSECT_FACES: &str = "Intersect Faces \u{23F5}";

fn with_recent(paths: &[&str]) -> Fixture {
    Fixture {
        recent: paths.iter().map(PathBuf::from).collect(),
        ..Default::default()
    }
}

#[test]
fn open_recent_lists_the_recent_files_and_clears() {
    let f = || with_recent(&["/a/house.skr", "/b/shed.skr"]);
    let mut h = harness(f());
    click(&mut h, "File");
    click(&mut h, OPEN_RECENT);
    assert!(
        rect_of(&h, "house.skr").min.y < rect_of(&h, "shed.skr").min.y,
        "newest first"
    );
    assert_eq!(
        click_menu(&mut harness(f()), &["File", OPEN_RECENT, "shed.skr"]),
        [UiAction::OpenRecent("/b/shed.skr".into())]
    );
    assert_eq!(
        click_menu(&mut harness(f()), &["File", OPEN_RECENT, "Clear Menu"]),
        [UiAction::ClearRecent]
    );

    // Nothing to list: no file rows, and Clear Menu does nothing.
    let mut h = harness(Fixture::default());
    let there = rects_named(&h, "Clear Menu");
    click(&mut h, "File");
    click(&mut h, OPEN_RECENT);
    assert!(
        new_node(&h, "Clear Menu", &there)
            .accesskit_node()
            .is_disabled()
    );
    new_node(&h, "Clear Menu", &there).click();
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
    let bar = crate::menus::menu_bar(&Fixture::default().view(), true);
    let entries = crate::menus::recent_entries_in(&bar).unwrap();
    assert!(matches!(entries, [crate::menus::Entry::Item(i)] if i.action == UiAction::ClearRecent));
}

#[test]
fn recent_labels_tell_same_names_apart() {
    use crate::menus::recent_label;
    let twins = [
        PathBuf::from("/a/Old/house.skr"),
        PathBuf::from("/a/New/house.skr"),
    ];
    assert_eq!(recent_label(&twins[0], &twins), "house.skr \u{2014} Old");
    assert_eq!(recent_label(&twins[1], &twins), "house.skr \u{2014} New");
    let alone = [PathBuf::from("/a/shed.skr")];
    assert_eq!(recent_label(&alone[0], &alone), "shed.skr");
}

#[test]
fn recent_paths_reads_the_submenu() {
    use crate::menus::{menu_bar, recent_paths};
    let f = with_recent(&["/a/house.skr", "/b/shed.skr"]);
    assert_eq!(recent_paths(&menu_bar(&f.view(), true)), f.recent);
    let empty = Fixture::default();
    assert_eq!(
        recent_paths(&menu_bar(&empty.view(), true)),
        Vec::<PathBuf>::new()
    );
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

    // The context bar's indicator, queried before a hover repeats its name in a tooltip.
    let mut h = harness(Fixture::default());
    let at = rect_of(&h, "Agent Activity");
    assert!(at.max.y <= theme::CONTEXT_BAR_HEIGHT, "in the bar: {at:?}");
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
    for kind in [ToolKind::LookAround, ToolKind::PositionCamera] {
        assert_eq!(
            camera_menu(plain, kind.label()),
            [UiAction::Tool(kind)],
            "{kind:?}"
        );
    }
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
    // Studio Lighting sits next to it; at night it still lights by studio, and unchecking it goes
    // back to day.
    assert_eq!(
        camera_menu(parallel_night, "Studio Lighting"),
        [UiAction::SetLighting(Lighting::Studio)]
    );
    let studio = || Fixture {
        lighting: Lighting::Studio,
        ..Default::default()
    };
    assert_eq!(
        camera_menu(studio, "Studio Lighting"),
        [UiAction::SetLighting(Lighting::Day)]
    );
    assert_eq!(
        camera_menu(studio, "Night Lighting"),
        [UiAction::SetLighting(Lighting::Night)]
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
fn edit_intersect_faces_runs_each_scope_and_needs_a_selection() {
    for (item, scope) in [
        ("With Model", IntersectScope::Model),
        ("With Selection", IntersectScope::Selection),
        ("With Context", IntersectScope::Context),
    ] {
        let mut h = harness(edit_fixture(true, false));
        assert_eq!(
            click_menu(&mut h, &["Edit", INTERSECT_FACES, item]),
            [UiAction::IntersectFaces(scope)],
            "{item}"
        );
        // Without a selection the item is disabled and does nothing.
        let mut h = harness(edit_fixture(false, false));
        click(&mut h, "Edit");
        click(&mut h, INTERSECT_FACES);
        assert!(
            h.get_by_label(item).accesskit_node().is_disabled(),
            "{item}"
        );
        h.get_by_label(item).click();
        h.run_ok();
        assert_eq!(h.state_mut().take(), [], "{item}");
    }
}

#[test]
fn intersect_faces_has_native_ids_that_leave_the_others_alone() {
    use crate::menus::item_id;
    assert_eq!(
        item_id(&UiAction::IntersectFaces(IntersectScope::Selection)),
        "IntersectFaces(Selection)"
    );
    assert_eq!(item_id(&UiAction::ShowSoften(true)), "ShowSoften(true)");
    assert_eq!(item_id(&UiAction::Soften), "Soften");
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
    // The new tools show their keys in the menus that hold them.
    for kind in [ToolKind::LookAround, ToolKind::PositionCamera] {
        let key = chord(Bound::Tool(kind)).unwrap().text();
        assert!(drawn_texts(&h).contains(&key), "{kind:?}: {key}");
        h.get_by_label(kind.label());
    }
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
            ("Studio Lighting", false),
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
            ("Studio Lighting", false),
            ("Perspective", true),
            ("Parallel Projection", false),
        ],
    );
    // Units are chosen in the Import Options sheet: File has no Import Units submenu.
    let mut h = harness(Fixture::default());
    click(&mut h, "File");
    assert!(h.query_by_label("Import...").is_some());
    assert!(h.query_by_label("Import Units \u{23F5}").is_none());
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
    h.get_by_label("Make Component").hover();
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
    use crate::ui::context_bar::step_name;
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
        ToolKind::Polygon,
        ToolKind::Arc,
        ToolKind::Freehand,
        ToolKind::RotatedRectangle,
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
        ToolKind::Scale,
        ToolKind::Offset,
        ToolKind::FollowMe,
        ToolKind::TapeMeasure,
        ToolKind::Dimension,
        ToolKind::Text,
        ToolKind::Erase,
        ToolKind::Paint,
        ToolKind::Protractor,
    ] {
        assert_eq!(
            click_menu(&mut harness(f()), &["Tools", kind.label()]),
            [UiAction::Tool(kind)]
        );
    }
    assert_eq!(
        click_menu(&mut harness(f()), &["Tools", "3D Text..."]),
        [UiAction::Show3dText]
    );
    for (item, action) in [
        ("Make Group", UiAction::MakeGroup),
        ("Make Component...", UiAction::MakeComponent),
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
    // Down by less than the room left under the window, which stays on screen.
    let to = from + Vec2::new(-120.0, 40.0);
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
        (moved - Vec2::new(-120.0, 40.0)).length() < 2.0,
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
    assert!(matches!(file.last(), Some(Entry::Item(i))
        if i.action == UiAction::Quit && i.label == "Quit"));
    assert!(egui.iter().all(|m| m.title != APP_MENU));
    let native = menu_bar(&f.view(), true);
    assert_eq!(native[0].title, APP_MENU);
    let quits = items(&native)
        .iter()
        .filter(|i| i.action == UiAction::Quit)
        .count();
    assert_eq!(quits, 1);
    assert!(
        matches!(native[0].entries.as_slice(), [Entry::Item(s), Entry::Item(q)]
        if s.label == "Settings..." && q.action == UiAction::Quit && q.label == "Quit sketchup-alt")
    );
    assert_eq!(
        click_menu(&mut harness(Fixture::default()), &["File", "Quit"]),
        [UiAction::Quit]
    );
}

#[test]
fn settings_is_in_the_app_menu_or_ends_edit() {
    use crate::menus::{APP_MENU, Entry, menu_bar};
    let f = Fixture::default();
    let native = menu_bar(&f.view(), true);
    let app = &native[0];
    assert_eq!(app.title, APP_MENU);
    let labels: Vec<&str> = app
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Item(i) => Some(i.label.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["Settings...", "Quit sketchup-alt"]);
    let settings_items = |bar: &[crate::menus::Menu]| {
        crate::menus::items(bar)
            .iter()
            .filter(|i| i.action == UiAction::ShowSettings(true))
            .count()
    };
    assert_eq!(settings_items(&native), 1, "once, in the app menu");
    let egui = menu_bar(&f.view(), false);
    let edit = &egui.iter().find(|m| m.title == "Edit").unwrap().entries;
    let [.., Entry::Separator, Entry::Item(settings)] = edit.as_slice() else {
        panic!("Edit ends with a separator and Settings...");
    };
    assert_eq!(settings.label, "Settings...");
    assert_eq!(settings.action, UiAction::ShowSettings(true));
    assert_eq!(settings.bound, Some(Bound::Settings));
    assert_eq!(settings_items(&egui), 1);
    let key = chord(Bound::Settings).unwrap();
    assert_eq!(key.text_for(false), "Ctrl+,");
    let mut h = harness(Fixture::default());
    click(&mut h, "Edit");
    assert!(drawn_texts(&h).contains(&key.text()), "the key beside it");
    assert_eq!(
        click_menu(&mut harness(Fixture::default()), &["Edit", "Settings..."]),
        [UiAction::ShowSettings(true)]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_app_menu_reads_about_settings_then_quit() {
    use crate::menus::{APP_MENU, Entry, menu_bar};
    use crate::native_menu::{AppSlot, app_menu_order};
    let f = Fixture::default();
    let bar = menu_bar(&f.view(), true);
    let app = &bar[0];
    assert_eq!(app.title, APP_MENU);
    let read: Vec<String> = app_menu_order(&app.entries)
        .into_iter()
        .flat_map(|slot| match slot {
            AppSlot::Entries(entries) => entries
                .iter()
                .map(|e| match e {
                    Entry::Item(i) => i.label.clone(),
                    _ => "?".into(),
                })
                .collect(),
            AppSlot::Separator => vec!["-".into()],
            AppSlot::About => vec!["About".into()],
            AppSlot::Hide => vec!["Hide".into()],
            AppSlot::HideOthers => vec!["HideOthers".into()],
            AppSlot::ShowAll => vec!["ShowAll".into()],
        })
        .collect();
    assert_eq!(
        read,
        [
            "About",
            "-",
            "Settings...",
            "-",
            "Hide",
            "HideOthers",
            "ShowAll",
            "-",
            "Quit sketchup-alt"
        ]
    );
}

#[test]
fn the_menu_bar_has_the_same_items_in_every_state() {
    use crate::menus::{item_key, items, menu_bar};
    // The Open Recent rows are the stated exception: that submenu is rebuilt when they change.
    let ids = |f: &Fixture| {
        items(&menu_bar(&f.view(), true))
            .iter()
            .filter(|i| !matches!(i.action, UiAction::OpenRecent(_)))
            .map(|i| item_key(i))
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
        section_display: SectionDisplay {
            planes: false,
            cuts: false,
            fill: false,
            ..Default::default()
        },
        recent: vec!["/a/house.skr".into(), "/b/shed.skr".into()],
        ..super::styles::styles_tab(Some(kernel::StyleId(2)))
    };
    assert_eq!(
        empty,
        ids(&busy(Lighting::Day)),
        "native items are built once and only synced"
    );
    assert_eq!(empty, ids(&busy(Lighting::Night)), "also at night");
    assert_eq!(empty, ids(&busy(Lighting::Studio)), "and in the studio");
    let unique: std::collections::BTreeSet<_> = empty.iter().collect();
    assert_eq!(unique.len(), empty.len(), "ids are unique");
    for id in [
        "ToggleEdgeEffect(Profiles)",
        "ToggleEdgeEffect(Sketchy)",
        "ToggleEdgeEffect(Endpoints)",
    ] {
        assert!(empty.iter().any(|e| e == id), "{id}");
    }
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

#[test]
fn every_cmd_chord_is_a_native_menu_accelerator() {
    let f = Fixture::default();
    let bar = crate::menus::menu_bar(&f.view(), true);
    let bound: Vec<Bound> = crate::menus::items(&bar)
        .iter()
        .filter_map(|i| i.bound)
        .collect();
    for s in crate::shortcuts::TABLE.iter().filter(|s| s.chord.command) {
        assert!(bound.contains(&s.bound), "{:?} has no menu item", s.bound);
    }
}

#[test]
fn text_actions_are_off_while_typing() {
    use crate::menus::{items, menu_bar, native_enabled};
    let f = Fixture {
        undo: Some("paint".into()),
        redo: Some("erase".into()),
        has_selection: true,
        has_clip: true,
        ..Default::default()
    };
    let bar = menu_bar(&f.view(), true);
    for i in items(&bar) {
        let text = matches!(
            i.action,
            UiAction::Undo
                | UiAction::Redo
                | UiAction::SelectAll
                | UiAction::SelectNone
                | UiAction::Cut
                | UiAction::Copy
                | UiAction::Paste
                | UiAction::PasteInPlace
        );
        assert_eq!(
            native_enabled(i, true, false),
            i.enabled && !text,
            "{}",
            i.label
        );
        assert_eq!(native_enabled(i, false, false), i.enabled, "{}", i.label);
    }
}

#[test]
fn select_all_and_none_yield_to_a_focused_text_field() {
    use crate::menus::{items, menu_bar, native_enabled};
    let f = edit_fixture(true, false);
    let bar = menu_bar(&f.view(), true);
    let item = |action: UiAction| {
        *items(&bar)
            .iter()
            .find(|i| i.action == action)
            .expect("an item")
    };
    for action in [UiAction::SelectAll, UiAction::SelectNone] {
        assert!(
            native_enabled(item(action.clone()), false, false),
            "{action:?}"
        );
        assert!(
            !native_enabled(item(action.clone()), true, false),
            "{action:?}"
        );
    }
    // A text field has no Cmd+D: Duplicate stays on.
    assert!(native_enabled(item(UiAction::Duplicate), true, false));
}

#[test]
fn the_edit_menu_selects_all_none_and_duplicates() {
    for (item, action) in [
        ("Select All", UiAction::SelectAll),
        ("Select None", UiAction::SelectNone),
        ("Duplicate", UiAction::Duplicate),
    ] {
        let mut h = harness(edit_fixture(true, false));
        assert_eq!(click_menu(&mut h, &["Edit", item]), [action], "{item}");
    }
    // Without a selection only Select All is on.
    for (item, off) in [
        ("Select All", false),
        ("Select None", true),
        ("Duplicate", true),
    ] {
        let mut h = harness(edit_fixture(false, false));
        let there = rects_named(&h, item);
        click(&mut h, "Edit");
        let node = new_node(&h, item, &there);
        assert_eq!(node.accesskit_node().is_disabled(), off, "{item}");
    }
    // Their keys are the table's.
    let mut h = harness(edit_fixture(true, false));
    click(&mut h, "Edit");
    let texts = drawn_texts(&h);
    for bound in [Bound::SelectAll, Bound::SelectNone, Bound::Duplicate] {
        let key = chord(bound).unwrap().text();
        assert!(texts.contains(&key), "{key} in {texts:?}");
    }
}

#[test]
fn the_edit_menu_cuts_copies_and_pastes() {
    let fixture = |has_selection, has_clip| Fixture {
        has_selection,
        has_clip,
        ..Default::default()
    };
    for (item, action) in [
        ("Cut", UiAction::Cut),
        ("Copy", UiAction::Copy),
        ("Paste", UiAction::Paste),
        ("Paste In Place", UiAction::PasteInPlace),
    ] {
        let mut h = harness(fixture(true, true));
        assert_eq!(click_menu(&mut h, &["Edit", item]), [action], "{item}");
    }
    // Cut and Copy need a selection, Paste and Paste In Place a clip on the clipboard.
    for (has_selection, has_clip) in [(false, false), (true, false), (false, true)] {
        for (item, needs_selection) in [
            ("Cut", true),
            ("Copy", true),
            ("Paste", false),
            ("Paste In Place", false),
        ] {
            let mut h = harness(fixture(has_selection, has_clip));
            let there = rects_named(&h, item);
            click(&mut h, "Edit");
            let on = if needs_selection {
                has_selection
            } else {
                has_clip
            };
            let node = new_node(&h, item, &there);
            assert_eq!(
                node.accesskit_node().is_disabled(),
                !on,
                "{item} {has_selection} {has_clip}"
            );
        }
    }
    // After Redo, before Select All; their keys are the table's and Paste In Place has none.
    let f = fixture(true, true);
    let bar = crate::menus::menu_bar(&f.view(), false);
    let edit = &bar.iter().find(|m| m.title == "Edit").unwrap().entries;
    let labels: Vec<Option<&str>> = edit
        .iter()
        .map(|e| match e {
            crate::menus::Entry::Item(i) => Some(i.label.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        labels[..8],
        [
            Some("Undo"),
            Some("Redo"),
            None,
            Some("Cut"),
            Some("Copy"),
            Some("Paste"),
            Some("Paste In Place"),
            None
        ]
    );
    let mut h = harness(fixture(true, true));
    click(&mut h, "Edit");
    let texts = drawn_texts(&h);
    for bound in [Bound::Cut, Bound::Copy, Bound::Paste] {
        let key = chord(bound).unwrap().text();
        assert!(texts.contains(&key), "{key} in {texts:?}");
    }
}

#[test]
fn cut_copy_and_paste_yield_to_a_focused_text_field() {
    use crate::menus::{items, menu_bar, native_enabled, native_event_acts};
    let f = Fixture {
        has_selection: true,
        has_clip: true,
        ..Default::default()
    };
    let bar = menu_bar(&f.view(), true);
    for action in [
        UiAction::Cut,
        UiAction::Copy,
        UiAction::Paste,
        UiAction::PasteInPlace,
    ] {
        let item = *items(&bar)
            .iter()
            .find(|i| i.action == action)
            .expect("an item");
        assert!(native_enabled(item, false, false), "{action:?}");
        // Disabled, so the key goes on to the field, and an event that raced it does nothing.
        assert!(!native_enabled(item, true, false), "{action:?}");
        assert!(
            native_event_acts(&action, false, false, false),
            "{action:?}"
        );
        assert!(
            !native_event_acts(&action, false, true, false),
            "{action:?}"
        );
        // A held key acts once.
        assert!(
            !native_event_acts(&action, true, false, false),
            "{action:?}"
        );
    }
}

#[test]
fn held_keys_repeat_only_undo_and_redo_through_the_menu() {
    use crate::menus::accepts_repeat;
    assert!(accepts_repeat(&UiAction::Undo) && accepts_repeat(&UiAction::Redo));
    for a in [
        UiAction::SaveFile,
        UiAction::NewModel,
        UiAction::Quit,
        UiAction::Tool(ToolKind::Line),
    ] {
        assert!(!accepts_repeat(&a), "{a:?}");
    }
}

#[test]
fn native_menu_events_act_once_and_leave_typing_alone() {
    use crate::menus::native_event_acts;
    // A click or a first key press acts.
    assert!(native_event_acts(&UiAction::SaveFile, false, false, false));
    assert!(native_event_acts(&UiAction::Undo, false, false, false));
    // Held Cmd+S saves once; held Cmd+Z keeps undoing.
    assert!(!native_event_acts(&UiAction::SaveFile, true, false, false));
    assert!(!native_event_acts(&UiAction::NewModel, true, false, false));
    assert!(native_event_acts(&UiAction::Undo, true, false, false));
    assert!(native_event_acts(&UiAction::Redo, true, false, false));
    // While a text field has focus Cmd+Z is the field's, even if the menu raced it.
    assert!(!native_event_acts(&UiAction::Undo, false, true, false));
    assert!(!native_event_acts(&UiAction::Redo, true, true, false));
    assert!(native_event_acts(&UiAction::SaveFile, false, true, false));
    // Cmd+A is the field's too; Cmd+D is not a field's, and a held one duplicates once.
    assert!(!native_event_acts(&UiAction::SelectAll, false, true, false));
    assert!(!native_event_acts(
        &UiAction::SelectNone,
        false,
        true,
        false
    ));
    assert!(native_event_acts(&UiAction::Duplicate, false, true, false));
    assert!(!native_event_acts(&UiAction::Duplicate, true, false, false));
    assert!(!native_event_acts(&UiAction::SelectAll, true, false, false));
}

#[test]
fn only_quit_works_while_a_sheet_is_open() {
    use crate::menus::{items, menu_bar, native_enabled, native_event_acts};
    let f = Fixture {
        undo: Some("paint".into()),
        has_selection: true,
        recent: vec!["/a/house.skr".into()],
        ..Default::default()
    };
    let bar = menu_bar(&f.view(), true);
    for i in items(&bar) {
        assert_eq!(
            native_enabled(i, false, true),
            i.action == UiAction::Quit,
            "{}",
            i.label
        );
    }
    assert!(!native_event_acts(&UiAction::OpenFile, false, false, true));
    assert!(native_event_acts(&UiAction::Quit, false, false, true));
}

#[test]
fn the_group_menu_renames_and_purges() {
    use crate::editor::RenameTarget;
    use crate::menus::{Entry, menu_bar};
    let target = RenameTarget {
        definition: "d2".parse().unwrap(),
        kind: kernel::DefinitionKind::Component,
        name: "Chair".into(),
    };
    let with_target = || Fixture {
        rename: Some(target.clone()),
        ..Default::default()
    };
    let bar = menu_bar(&with_target().view(), false);
    let group = &bar.iter().find(|m| m.title == "Group").unwrap().entries;
    let labels: Vec<&str> = group
        .iter()
        .map(|entry| match entry {
            Entry::Item(i) => i.label.as_str(),
            Entry::Submenu { label, .. } => label,
            Entry::Separator => "-",
        })
        .collect();
    assert_eq!(
        labels,
        [
            "Make Group",
            "Make Component...",
            "Explode",
            "Close Group",
            "-",
            "Make Unique",
            "Flip Along",
            "-",
            "Rename Component...",
            "Purge Unused",
        ]
    );
    let mut h = harness(with_target());
    assert_eq!(
        click_menu(&mut h, &["Group", "Rename Component..."]),
        [UiAction::RenameComponent]
    );
    let mut h = harness(with_target());
    click(&mut h, "Group");
    assert!(
        drawn_texts(&h).iter().any(|t| t == "F2"),
        "the key it shows"
    );
    let mut h = harness(Fixture::default());
    click(&mut h, "Group");
    assert!(
        h.get_by_label("Rename Component...")
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(click(&mut h, "Rename Component..."), []);
    // Purge Unused is always enabled: nothing has to be selected.
    let mut h = harness(Fixture::default());
    assert_eq!(
        click_menu(&mut h, &["Group", "Purge Unused"]),
        [UiAction::PurgeUnused]
    );
}

#[test]
fn field_of_view_needs_perspective() {
    assert_eq!(
        camera_menu(Fixture::default, "Field of View..."),
        [UiAction::ShowFieldOfView]
    );
    let mut h = harness(Fixture {
        projection: Projection::Parallel,
        ..Default::default()
    });
    click(&mut h, "Camera");
    assert!(
        h.get_by_label("Field of View...")
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(click(&mut h, "Field of View..."), []);
}

fn solid_tools_fixture(enabled: bool) -> Fixture {
    use crate::editor::SolidTools;
    Fixture {
        // As `Editor::solid_tools` reports two solid groups selected.
        solid_tools: if enabled {
            SolidTools::all_enabled()
        } else {
            SolidTools::default()
        },
        ..Fixture::default()
    }
}

const SOLID_TOOLS: &str = "Solid Tools \u{23F5}";

#[test]
fn tools_solid_tools_lists_its_items_and_runs_them_when_enabled() {
    use crate::editor::SolidOp;
    for op in SolidOp::ALL {
        let mut h = harness(solid_tools_fixture(true));
        assert_eq!(
            click_menu(&mut h, &["Tools", SOLID_TOOLS, op.label()]),
            [UiAction::SolidTool(op)],
            "{op:?}"
        );
        // Disabled without suitable solids: nothing runs.
        let mut h = harness(solid_tools_fixture(false));
        click(&mut h, "Tools");
        click(&mut h, SOLID_TOOLS);
        assert!(
            h.get_by_label(op.label()).accesskit_node().is_disabled(),
            "{op:?}"
        );
        h.get_by_label(op.label()).click();
        h.run_ok();
        assert_eq!(h.state_mut().take(), [], "{op:?}");
    }
}

#[test]
fn solid_tools_items_carry_the_tooltip_reason_and_stable_native_ids() {
    use crate::editor::SolidOp;
    use crate::menus::{Entry, item_id, menu_bar};
    let f = Fixture {
        solid_tools: crate::editor::SolidTools::refused("Select two or more solids"),
        ..Fixture::default()
    };
    let bar = menu_bar(&f.view(), true);
    let tools = &bar.iter().find(|m| m.title == "Tools").unwrap().entries;
    let Some(Entry::Submenu { entries, .. }) = tools
        .iter()
        .find(|e| matches!(e, Entry::Submenu { label, .. } if *label == "Solid Tools"))
    else {
        panic!("no Solid Tools submenu");
    };
    let labels: Vec<_> = entries
        .iter()
        .map(|e| match e {
            Entry::Item(i) => {
                assert!(!i.enabled);
                assert_eq!(i.tooltip.as_deref(), Some("Select two or more solids"));
                i.label.as_str()
            }
            _ => panic!("only items"),
        })
        .collect();
    assert_eq!(
        labels,
        [
            "Union",
            "Subtract",
            "Intersect",
            "Trim",
            "Split",
            "Outer Shell"
        ]
    );
    assert_eq!(
        item_id(&UiAction::SolidTool(SolidOp::OuterShell)),
        "SolidTool(OuterShell)"
    );
    // The ids of items that were already there did not change.
    assert_eq!(item_id(&UiAction::Show3dText), "Show3dText");
    assert_eq!(item_id(&UiAction::MakeGroup), "MakeGroup");
    assert_eq!(item_id(&UiAction::Tool(ToolKind::Select)), "Tool(Select)");
}

#[test]
fn tools_check_interference_is_enabled_runs_and_keeps_a_stable_native_id() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click_menu(&mut h, &["Tools", "Check Interference"]),
        [UiAction::CheckInterference]
    );
    let bar = crate::menus::menu_bar(&Fixture::default().view(), true);
    let tools = &bar.iter().find(|m| m.title == "Tools").unwrap().entries;
    let item = tools
        .iter()
        .find_map(|e| match e {
            crate::menus::Entry::Item(i) if i.label == "Check Interference" => Some(i),
            _ => None,
        })
        .expect("Check Interference item");
    assert!(item.enabled);
    assert_eq!(crate::menus::item_key(item), "CheckInterference");
    // The ids of items that were already there did not change.
    assert_eq!(
        crate::menus::item_id(&UiAction::SolidTool(crate::editor::SolidOp::Union)),
        "SolidTool(Union)"
    );
}

#[test]
fn tools_model_health_is_enabled_runs_and_keeps_a_stable_native_id() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click_menu(&mut h, &["Tools", "Model Health"]),
        [UiAction::CheckHealth]
    );
    let bar = crate::menus::menu_bar(&Fixture::default().view(), true);
    let tools = &bar.iter().find(|m| m.title == "Tools").unwrap().entries;
    let item = tools
        .iter()
        .find_map(|e| match e {
            crate::menus::Entry::Item(i) if i.label == "Model Health" => Some(i),
            _ => None,
        })
        .expect("Model Health item");
    assert!(item.enabled);
    assert_eq!(crate::menus::item_key(item), "CheckHealth");
    assert_eq!(
        crate::menus::item_id(&UiAction::CheckInterference),
        "CheckInterference"
    );
}
