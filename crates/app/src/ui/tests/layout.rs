use super::*;
use crate::ui::right::{OUTLINER_ROWS_MIN, row_pitch};
use kernel::DefinitionKind;
use session::OutlinerRow;

#[test]
fn panels_leave_a_central_view_inside_the_window() {
    let h = harness(Fixture::default());
    let rect = h.state().central;
    assert!(
        rect.min.x == theme::RAIL_WIDTH && rect.min.y > 0.0,
        "rail at the left, toolbar above: {rect:?}"
    );
    assert!(rect.max.x < 1280.0, "info panel on the right: {rect:?}");
    assert!(rect.max.y < 800.0, "status bar below: {rect:?}");
    assert!(rect.width() > 600.0 && rect.height() > 500.0, "{rect:?}");
}

/// A fixture with every toolbar button enabled.
fn toolbar_fixture() -> Fixture {
    Fixture {
        undo: Some("push_pull".into()),
        redo: Some("paint".into()),
        has_selection: true,
        has_instance_selected: true,
        inside_instance: true,
        has_guides: true,
        ..Default::default()
    }
}

/// The toolbar's buttons other than the tools, by name.
fn toolbar_buttons() -> Vec<(&'static str, UiAction)> {
    vec![
        ("Undo", UiAction::Undo),
        ("Redo", UiAction::Redo),
        ("Make Group", UiAction::MakeGroup),
        ("Make Component", UiAction::MakeComponent),
        ("Explode", UiAction::Explode),
    ]
}

/// Every toolbar button reports itself in a window of `size`.
fn every_toolbar_button_works(size: Vec2) {
    for kind in ToolKind::ALL {
        let mut h = harness_sized(toolbar_fixture(), size);
        assert_eq!(
            click(&mut h, kind.label()),
            [UiAction::Tool(kind)],
            "{kind:?} at {size:?}"
        );
    }
    for (label, action) in toolbar_buttons() {
        let mut h = harness_sized(toolbar_fixture(), size);
        assert_eq!(click(&mut h, label), [action], "{label} at {size:?}");
    }
}

/// The 3D view's top (the toolbar's height) in a window `width` points wide.
fn view_top(width: f32) -> f32 {
    harness_sized(toolbar_fixture(), Vec2::new(width, 800.0))
        .state()
        .central
        .min
        .y
}

#[test]
fn the_rail_holds_every_tool_in_one_column_by_group() {
    for size in [SCREEN, Vec2::new(960.0, 600.0)] {
        every_toolbar_button_works(size);
        let h = harness_sized(toolbar_fixture(), size);
        let order: Vec<ToolKind> = crate::ui::rail::GROUPS
            .iter()
            .flat_map(|g| g.iter().copied())
            .collect();
        assert_eq!(order.len(), ToolKind::ALL.len());
        for kind in ToolKind::ALL {
            assert!(order.contains(&kind), "{kind:?} on the rail");
        }
        let rects: Vec<Rect> = order.iter().map(|k| rect_of(&h, k.label())).collect();
        for (k, r) in order.iter().zip(&rects) {
            assert!(
                r.min.x >= 0.0 && r.max.x <= theme::RAIL_WIDTH,
                "{k:?} in the rail: {r:?}"
            );
            assert_eq!(
                (r.width(), r.height()),
                (theme::RAIL_BUTTON, theme::RAIL_BUTTON),
                "{k:?}"
            );
            assert!(
                r.max.y <= h.state().central.max.y,
                "{k:?} above the status bar at {size:?}"
            );
        }
        for pair in rects.windows(2) {
            assert_eq!(pair[0].center().x, pair[1].center().x, "one column");
            assert!(pair[1].min.y > pair[0].max.y, "top to bottom");
        }
        // 4 pt between buttons in a group, exactly `RAIL_GROUP_GAP` between groups.
        let gap =
            |a: ToolKind, b: ToolKind| rect_of(&h, b.label()).min.y - rect_of(&h, a.label()).max.y;
        assert!((gap(ToolKind::Select, ToolKind::Erase) - 4.0).abs() < 0.5);
        for (a, b) in [
            (ToolKind::Paint, ToolKind::Line),
            (ToolKind::Arc, ToolKind::PushPull),
            (ToolKind::Rotate, ToolKind::TapeMeasure),
        ] {
            let between = gap(a, b);
            assert!(
                (between - theme::RAIL_GROUP_GAP).abs() < 0.5,
                "{a:?} to {b:?}: {between}"
            );
        }
    }
}

#[test]
fn the_toolbar_without_tools_is_one_row_at_the_smallest_width() {
    assert_eq!(view_top(960.0), view_top(2400.0));
}

#[test]
fn the_right_panel_keeps_its_width_with_a_texture_shown() {
    let mut h = harness(Fixture {
        materials: vec![brick()],
        material: Some(MaterialId(31)),
        ..Default::default()
    });
    let width = |h: &mut H| {
        for _ in 0..3 {
            h.run_ok();
        }
        h.state().central.width()
    };
    let before = width(&mut h);
    h.state_mut().materials[0].texture = Some(TextureRow {
        swatch: egui::TextureId::default(),
        width: 123.456,
        height: 789.012,
        pixels: [4096, 4096],
    });
    assert_eq!(width(&mut h), before, "a texture shown");
    let shown = h.state_mut().materials[0].texture.take();
    assert_eq!(width(&mut h), before, "after the texture");
    h.state_mut().materials[0].texture = shown;
    h.state_mut().texture_position = Some(TexturePosition {
        placement: TexturePlacement {
            offset: [-123456.789, -987654.321],
            rotation_deg: 359.9,
            scale: [999.999, 999.999],
        },
        faces: faces(&[3]),
    });
    assert_eq!(width(&mut h), before, "texture position");
}

/// `n` materials named "Material 0", "Material 1", ...
pub(super) fn many_materials(n: u64) -> Vec<MaterialRow> {
    (0..n)
        .map(|k| MaterialRow {
            id: MaterialId(100 + k),
            name: format!("Material {k}"),
            color: [10, 20, 30],
            opacity: 1.0,
            emission: 0.0,
            texture: None,
        })
        .collect()
}

/// `n` unnamed groups at the root.
pub(super) fn many_groups(n: u64) -> OutlinerTable {
    OutlinerTable {
        roots: (0..n)
            .map(|k| OutlinerRow {
                id: format!("i{}", 200 + k).parse().unwrap(),
                definition: "d1".parse().unwrap(),
                kind: DefinitionKind::Group,
                name: String::new(),
                material: None,
            })
            .collect(),
        by_definition: BTreeMap::new(),
    }
}

/// A fixture with 40 materials (the first chosen) and `groups` groups.
pub(super) fn crowded(groups: u64) -> Fixture {
    Fixture {
        info: vec!["1 group".into()],
        materials: many_materials(40),
        material: Some(MaterialId(100)),
        outliner: many_groups(groups),
        ..Default::default()
    }
}

#[test]
fn a_short_window_leaves_the_outliner_its_minimum_height() {
    // Short enough that the Materials list is down to its least rows, tall enough that the
    // panel is not yet scrolled (with the 36 pt context bar, 28 pt controls and section titles it
    // needs about 655 points).
    let height = 660.0;
    let h = harness_sized(crowded(40), Vec2::new(1280.0, height));
    let model = rect_of(&h, "Model");
    let least = OUTLINER_ROWS_MIN as f32 * row_pitch(&h.ctx.global_style());
    // The panel ends where the status bar begins, as the 3D view does.
    let bottom = h.state().central.max.y;
    assert!(
        model.min.y <= bottom - least,
        "the Outliner starts at {} over a panel ending at {bottom} in a {height} point window",
        model.min.y
    );
}

#[test]
fn a_very_short_window_scrolls_the_right_panel() {
    let height = 300.0;
    let mut h = harness_sized(crowded(1), Vec2::new(1280.0, height));
    let before = rect_of(&h, "Model").min.y;
    assert!(
        before > height,
        "the Outliner is below the window at {before}"
    );
    // Scroll with the pointer over Entity Info.
    h.event(egui::Event::PointerMoved(Pos2::new(1200.0, 60.0)));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0.0, -400.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_ok();
    let after = rect_of(&h, "Model").min.y;
    assert!(
        after < before - 100.0,
        "the panel scrolled from {before} to {after}"
    );
}

#[test]
fn no_material_row_is_cut_by_the_list_bottom() {
    for height in [800.0, 600.0, 520.0] {
        let h = harness_sized(crowded(40), Vec2::new(1280.0, height));
        let bottom = rect_of(&h, "Add").min.y - h.ctx.global_style().spacing.item_spacing.y;
        for k in 0..40 {
            let row = rect_of(&h, &format!("Material {k}"));
            assert!(
                !(row.min.y < bottom - 0.5 && row.max.y > bottom + 0.5),
                "Material {k} straddles the list's bottom {bottom} at {height} points: {row:?}"
            );
        }
    }
}

#[test]
fn long_names_keep_the_panel_width() {
    let long = "A material whose name is far longer than the panel is wide";
    let mut f = crowded(3);
    f.materials[0].name = long.into();
    f.outliner.roots[0].name = long.into();
    let width = harness(f).state().central.width();
    assert_eq!(width, harness(crowded(3)).state().central.width());
}

#[test]
fn the_panels_settle_in_one_frame() {
    // Nothing changed: one frame and no request for another (the window can go idle).
    for f in [Fixture::default(), crowded(40)] {
        let mut h = harness(f);
        assert_eq!(h.try_run().unwrap(), 1);
    }
}

#[test]
fn the_panels_fit_the_smallest_window() {
    let size = Vec2::new(960.0, 600.0);
    every_toolbar_button_works(size);
    assert_eq!(view_top(size.x), view_top(2400.0), "one toolbar row");
    let h = harness_sized(crowded(40), size);
    let view = h.state().central;
    assert!(
        view.width() >= 600.0 && view.height() >= 350.0,
        "a usable 3D view: {view:?}"
    );
    h.get_by_label("Model");
}
