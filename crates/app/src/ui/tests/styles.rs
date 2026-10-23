use kernel::{EdgeColorMode, Fog, FogColor, Ground, Sketchy, StyleId, StyleSettings};
use session::{EdgesPatch, StylePatch};

use super::*;
use crate::ui::{EdgeEffect, StyleRow};

const PLAIN: StyleId = StyleId(1);
const INKED: StyleId = StyleId(2);

/// The View menu's submenu button, with its arrow.
const EDGE_STYLE: &str = "Edge Style \u{23F5}";

/// A style with every optional part on.
fn inked_settings() -> StyleSettings {
    let mut s = StyleSettings::default();
    s.edges.profiles = Some(4.0);
    s.edges.sketchy = Some(Sketchy {
        jitter: 2.0,
        extension: 6.0,
    });
    s.edges.endpoints = Some(8.0);
    s.sky = Some([100, 150, 230]);
    s.ground = Some(Ground {
        color: [90, 120, 80],
        opacity: 0.5,
        from_below: false,
    });
    s.fog = Some(Fog {
        start: 10.0,
        end: 100.0,
        color: FogColor::Background,
    });
    s
}

/// Plain (every part off) and Inked (every part on).
fn two_styles() -> Vec<StyleRow> {
    vec![
        StyleRow {
            id: PLAIN,
            name: "Plain".into(),
            settings: StyleSettings::default(),
        },
        StyleRow {
            id: INKED,
            name: "Inked".into(),
            settings: inked_settings(),
        },
    ]
}

/// The dock on the Styles tab with Plain and Inked, `style` chosen.
pub(super) fn styles_tab(style: Option<StyleId>) -> Fixture {
    Fixture {
        styles: two_styles(),
        style,
        dock: DockLayout {
            tab: DockTab::Styles,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn edges(f: impl FnOnce(&mut EdgesPatch)) -> StylePatch {
    let mut e = EdgesPatch::default();
    f(&mut e);
    StylePatch {
        edges: Some(e),
        ..Default::default()
    }
}

fn change(style: StyleId, patch: StylePatch) -> UiAction {
    UiAction::ChangeStyle(style, patch)
}

/// A window tall enough to show the whole editor (the dock's top area scrolls).
fn tall(f: Fixture) -> H {
    harness_sized(f, Vec2::new(1280.0, 3200.0))
}

/// Clicks `along` (0 to 1) of the way across the slider named `label`.
fn click_slider(h: &mut H, label: &str, along: f32) -> Vec<UiAction> {
    let slider = rect_of(h, label);
    let at = Pos2::new(slider.min.x + along * slider.width(), slider.center().y);
    let mut actions = click_at(h, at, egui::Modifiers::NONE);
    actions.dedup();
    actions
}

#[test]
fn the_styles_tab_lists_default_and_every_style() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click(&mut h, "Styles"),
        [UiAction::SetDock(DockLayout {
            tab: DockTab::Styles,
            ..Default::default()
        })]
    );
    let h = tall(styles_tab(None));
    for label in ["Default", "Plain", "Inked", "Add style", "Delete style"] {
        h.get_by_label(label);
    }
    assert_eq!(is_on(&h, "Default"), Some(true));
    assert_eq!(is_on(&h, "Plain"), Some(false));
    // Default has no settings to edit.
    assert!(h.query_by_label("Show edges").is_none());
    let h = tall(styles_tab(Some(INKED)));
    assert_eq!(is_on(&h, "Default"), Some(false));
    assert_eq!(is_on(&h, "Inked"), Some(true));
    h.get_by_label("Show edges");
}

#[test]
fn clicking_a_row_chooses_it() {
    let mut h = tall(styles_tab(None));
    assert_eq!(click(&mut h, "Inked"), [UiAction::ChooseStyle(Some(INKED))]);
    let mut h = tall(styles_tab(Some(INKED)));
    assert_eq!(click(&mut h, "Default"), [UiAction::ChooseStyle(None)]);
}

#[test]
fn add_copies_the_chosen_style_and_delete_deletes_it() {
    let mut h = tall(styles_tab(Some(INKED)));
    assert_eq!(
        click(&mut h, "Add style"),
        [UiAction::AddStyle(Some(INKED))]
    );
    assert_eq!(
        click(&mut h, "Delete style"),
        [UiAction::DeleteStyle(INKED)]
    );
    let mut h = tall(styles_tab(None));
    assert_eq!(click(&mut h, "Add style"), [UiAction::AddStyle(None)]);
    assert_eq!(
        click(&mut h, "Delete style"),
        [],
        "Default cannot be deleted"
    );
}

#[test]
fn double_clicking_a_name_renames_the_style() {
    let mut h = tall(styles_tab(None));
    let at = rect_of(&h, "Plain").center();
    click_at(&mut h, at, egui::Modifiers::NONE);
    assert_eq!(click_at(&mut h, at, egui::Modifiers::NONE), []);
    h.run_ok();
    h.event(egui::Event::Text("Pencil".into()));
    h.run_ok();
    assert_eq!(h.state_mut().take(), [], "nothing while typing");
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(
        h.state_mut().take(),
        [UiAction::RenameStyle(PLAIN, "Pencil".into())]
    );
    // Escape keeps the name.
    let mut h = tall(styles_tab(None));
    click_at(&mut h, at, egui::Modifiers::NONE);
    click_at(&mut h, at, egui::Modifiers::NONE);
    h.run_ok();
    h.event(egui::Event::Text("Old".into()));
    h.run_ok();
    h.key_press(egui::Key::Escape);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn the_edge_controls_show_the_style_and_change_it() {
    let mut h = tall(styles_tab(Some(PLAIN)));
    assert_eq!(is_on(&h, "Show edges"), Some(true));
    assert_eq!(is_on(&h, "Profiles"), Some(false));
    assert_eq!(
        click(&mut h, "Show edges"),
        [change(PLAIN, edges(|e| e.show_edges = Some(false)))]
    );
    assert_eq!(
        click(&mut h, "Profiles"),
        [change(PLAIN, edges(|e| e.profiles = Some(Some(3.0))))]
    );
    assert_eq!(
        click(&mut h, "Sketchy edges"),
        [change(
            PLAIN,
            edges(|e| e.sketchy = Some(Some(Sketchy {
                jitter: 2.0,
                extension: 8.0
            })))
        )]
    );
    assert_eq!(
        click(&mut h, "Endpoints"),
        [change(PLAIN, edges(|e| e.endpoints = Some(Some(8.0))))]
    );
    assert_eq!(
        click(&mut h, "By axis"),
        [change(
            PLAIN,
            edges(|e| e.color_mode = Some(EdgeColorMode::ByAxis))
        )]
    );
    assert_eq!(
        click(&mut h, "By material"),
        [change(
            PLAIN,
            edges(|e| e.color_mode = Some(EdgeColorMode::ByMaterial))
        )]
    );
    // Turned off from Inked: null parts.
    let mut h = tall(styles_tab(Some(INKED)));
    assert_eq!(is_on(&h, "Profiles"), Some(true));
    assert_eq!(
        click(&mut h, "Profiles"),
        [change(INKED, edges(|e| e.profiles = Some(None)))]
    );
    assert_eq!(
        click(&mut h, "Sketchy edges"),
        [change(INKED, edges(|e| e.sketchy = Some(None)))]
    );
    assert_eq!(
        click(&mut h, "Endpoints"),
        [change(INKED, edges(|e| e.endpoints = Some(None)))]
    );
}

#[test]
fn the_scene_controls_turn_sky_ground_and_fog_on_and_off() {
    let mut h = tall(styles_tab(Some(PLAIN)));
    let patch = |f: fn(&mut StylePatch)| {
        let mut p = StylePatch::default();
        f(&mut p);
        change(PLAIN, p)
    };
    assert_eq!(
        click(&mut h, "Sky"),
        [patch(|p| p.sky = Some(Some([135, 180, 235])))]
    );
    assert_eq!(
        click(&mut h, "Ground"),
        [patch(|p| {
            p.ground = Some(Some(Ground {
                color: [150, 140, 120],
                opacity: 0.5,
                from_below: false,
            }))
        })]
    );
    assert_eq!(
        click(&mut h, "Fog"),
        [patch(|p| {
            p.fog = Some(Some(Fog {
                start: 10.0,
                end: 100.0,
                color: FogColor::Background,
            }))
        })]
    );
    let mut h = tall(styles_tab(Some(INKED)));
    for (label, off) in [
        (
            "Sky",
            (|p: &mut StylePatch| p.sky = Some(None)) as fn(&mut StylePatch),
        ),
        ("Ground", |p| p.ground = Some(None)),
        ("Fog", |p| p.fog = Some(None)),
    ] {
        let mut p = StylePatch::default();
        off(&mut p);
        assert_eq!(click(&mut h, label), [change(INKED, p)], "{label}");
    }
    // From below changes the ground.
    assert_eq!(
        click(&mut h, "Ground from below"),
        [change(
            INKED,
            StylePatch {
                ground: Some(Some(Ground {
                    color: [90, 120, 80],
                    opacity: 0.5,
                    from_below: true,
                })),
                ..Default::default()
            }
        )]
    );
}

#[test]
fn a_slider_click_sets_its_value_once() {
    let mut h = tall(styles_tab(Some(INKED)));
    let actions = click_slider(&mut h, "Profile width", 0.999);
    assert!(
        matches!(actions.as_slice(),
            [UiAction::ChangeStyle(INKED, StylePatch { edges: Some(EdgesPatch { profiles: Some(Some(w)), .. }), .. })]
            if *w > 9.0),
        "{actions:?}"
    );
    let actions = click_slider(&mut h, "Ground opacity", 0.999);
    assert!(
        matches!(actions.as_slice(),
            [UiAction::ChangeStyle(INKED, StylePatch { ground: Some(Some(g)), .. })]
            if g.opacity > 0.95 && g.color == [90, 120, 80]),
        "{actions:?}"
    );
    let actions = click_slider(&mut h, "Jitter", 0.999);
    assert!(
        matches!(actions.as_slice(),
            [UiAction::ChangeStyle(INKED, StylePatch { edges: Some(EdgesPatch { sketchy: Some(Some(s)), .. }), .. })]
            if s.jitter > 9.0 && s.extension == 6.0),
        "{actions:?}"
    );
    let actions = click_slider(&mut h, "Endpoint length", 0.999);
    assert!(
        matches!(actions.as_slice(),
            [UiAction::ChangeStyle(INKED, StylePatch { edges: Some(EdgesPatch { endpoints: Some(Some(l)), .. }), .. })]
            if *l > 28.0),
        "{actions:?}"
    );
}

#[test]
fn dragging_a_slider_is_one_change_when_it_ends() {
    let mut h = tall(styles_tab(Some(INKED)));
    let slider = rect_of(&h, "Profile width");
    let width = h
        .ctx
        .global_style()
        .spacing
        .slider_width
        .min(slider.width());
    let start = Pos2::new(slider.min.x + 0.2 * width, slider.center().y);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(egui::Event::PointerMoved(start));
    h.event(button(start, true));
    h.run_ok();
    for k in 1..=10 {
        h.event(egui::Event::PointerMoved(
            start + Vec2::new(3.0 * k as f32, 0.0),
        ));
        h.run_ok();
    }
    assert_eq!(h.state_mut().take(), [], "nothing while dragging");
    h.event(button(start + Vec2::new(30.0, 0.0), false));
    h.run_ok();
    let actions = h.state_mut().take();
    assert!(
        matches!(actions.as_slice(),
            [UiAction::ChangeStyle(INKED, StylePatch { edges: Some(EdgesPatch { profiles: Some(Some(w)), .. }), .. })]
            if *w > 4.0),
        "{actions:?}"
    );
}

#[test]
fn the_fog_distances_are_in_the_models_units() {
    let mut h = tall(styles_tab(Some(INKED)));
    // 10 m and 100 m, in the default units.
    h.get_by_label("Fog start value");
    h.get_by_label("Fog end value");
    let actions = type_into(&mut h, "Fog start value", 0, "20", egui::Key::Enter);
    let want = change(
        INKED,
        StylePatch {
            fog: Some(Some(Fog {
                start: 20.0,
                end: 100.0,
                color: FogColor::Background,
            })),
            ..Default::default()
        },
    );
    assert_eq!(actions, [want]);
    // A model in feet: 50 ft is 15.24 m.
    let units = session::Units {
        length: session::LengthUnit::Feet,
        ..Default::default()
    };
    let mut h = tall(Fixture {
        units,
        ..styles_tab(Some(INKED))
    });
    let actions = type_into(&mut h, "Fog end value", 0, "50'", egui::Key::Enter);
    assert!(
        matches!(actions.as_slice(),
            [UiAction::ChangeStyle(INKED, StylePatch { fog: Some(Some(f)), .. })]
            if (f.end - 15.24).abs() < 0.01 && f.start == 10.0),
        "{actions:?}"
    );
}

#[test]
fn the_fog_colour_is_the_background_or_custom() {
    let mut h = tall(styles_tab(Some(INKED)));
    let patch = |color| {
        change(
            INKED,
            StylePatch {
                fog: Some(Some(Fog {
                    start: 10.0,
                    end: 100.0,
                    color,
                })),
                ..Default::default()
            },
        )
    };
    assert_eq!(
        click(&mut h, "Custom"),
        [patch(FogColor::Rgb([245, 245, 245]))]
    );
    h.state_mut().styles[1].settings.fog = Some(Fog {
        start: 10.0,
        end: 100.0,
        color: FogColor::Rgb([1, 2, 3]),
    });
    h.run_ok();
    assert_eq!(is_on(&h, "Custom"), Some(true));
    assert_eq!(
        click(&mut h, "Same as background"),
        [patch(FogColor::Background)]
    );
}

/// The popup id of the colour picker named `name`.
fn picker(name: &str) -> egui::Id {
    egui::Id::new(("style-color", name)).with("popup")
}

/// Opens the picker of the swatch `name` and drags across its colour square.
fn open_and_pick(h: &mut H, name: &str) {
    assert_eq!(click(h, name), [], "the picker opens");
    let area = h
        .ctx
        .memory(|m| m.area_rect(picker(name)))
        .expect("the picker is open");
    let square = h
        .root()
        .children_recursive()
        .filter_map(|n| {
            let b = n.accesskit_node().bounding_box()?;
            let r = Rect::from_min_max(
                Pos2::new(b.x0 as f32, b.y0 as f32),
                Pos2::new(b.x1 as f32, b.y1 as f32),
            );
            (area.contains_rect(r) && r.width() > 40.0 && (r.width() - r.height()).abs() < 1.0)
                .then_some(r)
        })
        .next()
        .expect("the colour square");
    let (from, to) = (
        square.min + square.size() * 0.3,
        square.min + square.size() * 0.7,
    );
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(
            from + (to - from) * k as f32 / 4.0,
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
    assert_eq!(h.state_mut().take(), [], "nothing while the picker is open");
}

#[test]
fn a_picked_colour_is_one_change_when_the_picker_closes() {
    let outside = |h: &H| h.state().central.center();
    for name in [
        "Edge colour",
        "Background colour",
        "Sky colour",
        "Ground colour",
    ] {
        let mut h = tall(styles_tab(Some(INKED)));
        open_and_pick(&mut h, name);
        let at = outside(&h);
        let mut actions = click_at(&mut h, at, egui::Modifiers::NONE);
        h.run_ok();
        actions.extend(h.state_mut().take());
        let [UiAction::ChangeStyle(INKED, p)] = actions.as_slice() else {
            panic!("{name}: {actions:?}");
        };
        match name {
            "Edge colour" => assert!(p.edges.as_ref().is_some_and(|e| e.color.is_some())),
            "Background colour" => assert!(p.background.is_some()),
            "Sky colour" => assert!(matches!(p.sky, Some(Some(_)))),
            _ => assert!(
                matches!(p.ground, Some(Some(g)) if g.opacity == 0.5 && !g.from_below && g.color != [90, 120, 80])
            ),
        }
        // Opened and closed without picking: nothing.
        let mut h = tall(styles_tab(Some(INKED)));
        click(&mut h, name);
        let at = outside(&h);
        assert_eq!(click_at(&mut h, at, egui::Modifiers::NONE), [], "{name}");
        h.run_ok();
        assert_eq!(h.state_mut().take(), [], "{name}");
    }
}

#[test]
fn the_fog_colour_swatch_sets_a_custom_colour() {
    let mut f = styles_tab(Some(INKED));
    f.styles[1].settings.fog = Some(Fog {
        start: 10.0,
        end: 100.0,
        color: FogColor::Rgb([1, 2, 3]),
    });
    let mut h = tall(f);
    open_and_pick(&mut h, "Fog colour");
    let at = h.state().central.center();
    let mut actions = click_at(&mut h, at, egui::Modifiers::NONE);
    h.run_ok();
    actions.extend(h.state_mut().take());
    assert!(
        matches!(actions.as_slice(),
            [UiAction::ChangeStyle(INKED, StylePatch { fog: Some(Some(Fog { color: FogColor::Rgb(c), start, end })), .. })]
            if *c != [1, 2, 3] && *start == 10.0 && *end == 100.0),
        "{actions:?}"
    );
}

#[test]
fn a_picker_left_open_when_the_tab_closes_drops_its_colour() {
    let mut h = tall(styles_tab(Some(INKED)));
    open_and_pick(&mut h, "Background colour");
    h.state_mut().dock.tab = DockTab::Materials;
    h.run_ok();
    h.run_ok();
    h.state_mut().dock.tab = DockTab::Styles;
    h.run_ok();
    h.run_ok();
    let at = h.state().central.center();
    click_at(&mut h, at, egui::Modifiers::NONE);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn nothing_is_reported_for_values_shown_unchanged() {
    // Every value box and slider shows what is stored; a few frames report nothing.
    let mut h = tall(styles_tab(Some(INKED)));
    for _ in 0..5 {
        h.run_ok();
    }
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn a_chosen_style_that_goes_away_leaves_no_editor() {
    let mut h = tall(styles_tab(Some(INKED)));
    h.get_by_label("Show edges");
    h.state_mut().styles.truncate(1);
    h.state_mut().style = None;
    h.run_ok();
    assert!(h.query_by_label("Show edges").is_none());
    h.get_by_label("Plain");
}

#[test]
fn the_edge_style_menu_toggles_the_chosen_styles_effects() {
    for (item, effect) in [
        ("Profiles", EdgeEffect::Profiles),
        ("Sketchy", EdgeEffect::Sketchy),
        ("Endpoints", EdgeEffect::Endpoints),
    ] {
        let mut h = tall(styles_tab(Some(INKED)));
        assert_eq!(
            click_menu(&mut h, &["View", EDGE_STYLE, item]),
            [UiAction::ToggleEdgeEffect(effect)],
            "{item}"
        );
    }
    // The items show the chosen style's effects, and are off for Default.
    let f = styles_tab(Some(PLAIN));
    let menus = crate::menus::menu_bar(&f.view(), false);
    let flags = |f: &Fixture| {
        let menus = crate::menus::menu_bar(&f.view(), false);
        crate::menus::items(&menus)
            .into_iter()
            .filter(|i| matches!(i.action, UiAction::ToggleEdgeEffect(_)))
            .map(|i| (i.label.clone(), i.enabled, i.checked))
            .collect::<Vec<_>>()
    };
    drop(menus);
    assert_eq!(
        flags(&styles_tab(Some(INKED))),
        [
            ("Profiles".into(), true, Some(true)),
            ("Sketchy".into(), true, Some(true)),
            ("Endpoints".into(), true, Some(true)),
        ]
    );
    assert_eq!(
        flags(&styles_tab(Some(PLAIN))),
        [
            ("Profiles".into(), true, Some(false)),
            ("Sketchy".into(), true, Some(false)),
            ("Endpoints".into(), true, Some(false)),
        ]
    );
    assert_eq!(
        flags(&styles_tab(None)),
        [
            ("Profiles".into(), false, Some(false)),
            ("Sketchy".into(), false, Some(false)),
            ("Endpoints".into(), false, Some(false)),
        ]
    );
}

#[test]
fn the_edge_effect_patch_turns_one_effect_over() {
    use crate::ui::edge_effect_patch;
    let plain = StyleSettings::default();
    assert_eq!(
        edge_effect_patch(&plain, EdgeEffect::Profiles),
        edges(|e| e.profiles = Some(Some(3.0)))
    );
    let inked = inked_settings();
    assert_eq!(
        edge_effect_patch(&inked, EdgeEffect::Profiles),
        edges(|e| e.profiles = Some(None))
    );
    assert_eq!(
        edge_effect_patch(&inked, EdgeEffect::Sketchy),
        edges(|e| e.sketchy = Some(None))
    );
    assert_eq!(
        edge_effect_patch(&inked, EdgeEffect::Endpoints),
        edges(|e| e.endpoints = Some(None))
    );
}
