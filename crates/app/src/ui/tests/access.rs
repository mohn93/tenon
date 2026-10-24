//! The AccessKit tree: none until a screen reader asks for it, and a name for every control
//! in it.

use egui::accesskit::{Action, Role};

use super::*;
use crate::editor::{ImportOptions, RenameTarget};
use crate::navigation::TrackpadScroll;
use crate::ui::icons;
use crate::ui::theme::UiScale;

/// Runs one frame of the panels for `f` on a plain context (not a harness, which turns AccessKit
/// on) and returns what it output.
fn plain_frame(ctx: &egui::Context, f: &Fixture) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        let mut actions = Vec::new();
        draw(ui, &f.view(), &mut actions);
    });
    output.textures_delta.clear();
    output
}

#[test]
fn no_accesskit_tree_until_a_screen_reader_asks() {
    let ctx = egui::Context::default();
    install_theme(&ctx);
    let f = Fixture::default();
    for _ in 0..2 {
        let output = plain_frame(&ctx, &f);
        assert!(output.platform_output.accesskit_update.is_none());
    }
    ctx.enable_accesskit();
    let output = plain_frame(&ctx, &f);
    assert!(output.platform_output.accesskit_update.is_some());
    ctx.disable_accesskit();
    let output = plain_frame(&ctx, &f);
    assert!(output.platform_output.accesskit_update.is_none());
}

/// The roles a screen reader treats as controls, which must say what they are.
const CONTROL_ROLES: [Role; 8] = [
    Role::Button,
    Role::CheckBox,
    Role::RadioButton,
    Role::ComboBox,
    Role::TextInput,
    Role::SpinButton,
    Role::Slider,
    Role::Tab,
];

/// Whether `node` is a control: one of the control roles, or anything that can be clicked.
fn is_control(node: &egui_kittest::kittest::AccessKitNode<'_>) -> bool {
    CONTROL_ROLES.contains(&node.role()) || node.data().supports_action(Action::Click)
}

/// What a screen reader says for `node`: its name (its own label or the nodes that label it),
/// or a text label's text. None when empty or only icon glyphs, which read as nothing or as a
/// code point.
fn name_of(node: &egui_kittest::kittest::AccessKitNode<'_>) -> Option<String> {
    let text = node
        .label()
        .or_else(|| (node.role() == Role::Label).then(|| node.value())?)?;
    let mut rest = text.clone();
    for g in icons::all() {
        rest = rest.replace(g, "");
    }
    (!rest.trim().is_empty()).then_some(text)
}

/// The node's rect, when it has one.
fn bounds(node: &egui_kittest::kittest::AccessKitNode<'_>) -> Option<Rect> {
    node.bounding_box().map(|b| {
        Rect::from_min_max(
            Pos2::new(b.x0 as f32, b.y0 as f32),
            Pos2::new(b.x1 as f32, b.y1 as f32),
        )
    })
}

/// Every control in the last frame's tree without a name, with its role and rect.
fn unnamed_controls(h: &H) -> Vec<String> {
    h.root()
        .children_recursive()
        .filter(|n| {
            let ak = n.accesskit_node();
            is_control(&ak) && name_of(&ak).is_none()
        })
        .map(|n| {
            let ak = n.accesskit_node();
            format!("{:?} at {:?}", ak.role(), bounds(&ak))
        })
        .collect()
}

/// The fixtures the name check walks, each with what it shows. Some need clicks after the
/// harness is built (a Settings tab, an open menu); those are in `every_control_has_a_name`.
fn fixtures() -> Vec<(&'static str, Fixture)> {
    let d2: kernel::DefinitionId = "d2".parse().unwrap();
    let kitchen = RenameTarget {
        definition: d2,
        kind: DefinitionKind::Component,
        name: "Kitchen".into(),
    };
    let mut textured = brick();
    textured.texture = Some(small_texture());
    let mut glass = brick();
    glass.id = MaterialId(32);
    glass.name = "Glass".into();
    let sheets = [
        (
            "the Import Options sheet",
            Sheet::Import {
                path: "/m/chair.obj".into(),
                options: ImportOptions::default(),
            },
        ),
        (
            "the Make Component sheet",
            Sheet::NameComponent {
                suggested: "Component#1".into(),
            },
        ),
        (
            "the Rename sheet",
            Sheet::Rename {
                target: kitchen.clone(),
            },
        ),
        (
            "the Print sheet",
            Sheet::Print {
                purpose: crate::editor::PrintPurpose::Export,
                spec: crate::editor::PrintOptions::default(),
            },
        ),
        (
            "the Field of View sheet",
            Sheet::FieldOfView { degrees: 35.0 },
        ),
        (
            "the note sheet",
            Sheet::NoteText {
                request: tools::NoteRequest::Screen { at: [0.5, 0.5] },
                text: "Plan".into(),
                automatic: None,
            },
        ),
        (
            "the 3D Text sheet",
            Sheet::Text3d {
                spec: tools::TextSpec {
                    text: "Hello".into(),
                    height: 0.3,
                    depth: 0.05,
                    filled: true,
                },
            },
        ),
        (
            "the Delete Tag sheet",
            Sheet::DeleteTag {
                tag: kernel::TagId(1),
                name: "Walls".into(),
                count: 5,
                others: vec![(kernel::TagId(2), "Roof".into())],
            },
        ),
    ];
    let mut all = vec![
        ("the default", Fixture::default()),
        ("the tags tab", super::tags::tags_tab()),
        (
            "the shadows tab",
            Fixture {
                dock: DockLayout {
                    tab: DockTab::Shadows,
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
        (
            "the styles tab, Default chosen",
            super::styles::styles_tab(None),
        ),
        (
            "the styles tab, a style chosen",
            super::styles::styles_tab(Some(kernel::StyleId(2))),
        ),
        ("the scenes tab and scene tabs", super::scenes::scenes_tab()),
        (
            "entity info's tag field",
            Fixture {
                tags: super::tags::two_tags(),
                has_selection: true,
                selection_tag: Some(crate::editor::SelectionTag::Mixed),
                ..Default::default()
            },
        ),
        (
            "the house with a selected component",
            Fixture {
                selection: ids(&["i3"]).into_iter().collect(),
                has_selection: true,
                has_instance_selected: true,
                info: vec!["1 component".into()],
                rename: Some(kitchen),
                ..house_fixture(&["i1"])
            },
        ),
        (
            "materials with a textured row, one chosen",
            Fixture {
                materials: vec![textured, glass],
                material: Some(MaterialId(31)),
                dock: DockLayout {
                    split: *crate::ui::dock::SPLIT.end(),
                    ..super::layout::materials_tab()
                },
                ..Default::default()
            },
        ),
        (
            "texture position fields",
            with_texture_position(faces(&[3, 4]), kernel::TexturePlacement::default()),
        ),
        (
            "the soften window",
            Fixture {
                soften: Some(20),
                ..Default::default()
            },
        ),
        (
            "the agent activity window",
            Fixture {
                connected: 1,
                log: Some(vec![
                    "5 s ago  #1  execute save /a.skr  ok".into(),
                    "3 s ago  #2  query model  ok".into(),
                ]),
                ..Default::default()
            },
        ),
        (
            "the shortcuts window",
            Fixture {
                shortcuts: true,
                ..Default::default()
            },
        ),
        (
            "the start card with two recent files",
            Fixture {
                start: Some(vec!["/m/house.skr".into(), "/m/shed.skr".into()]),
                ..Default::default()
            },
        ),
    ];
    for (what, sheet) in sheets {
        all.push((
            what,
            Fixture {
                sheet: Some(sheet),
                ..Default::default()
            },
        ));
    }
    all
}

/// The Settings window open on its first tab.
fn settings_open() -> Fixture {
    Fixture {
        settings: Some(SettingsView {
            ui_scale: UiScale::P100,
            trackpad: TrackpadScroll::Orbit,
            autosave_minutes: 5,
            transition: kernel::SceneTransition::DEFAULT,
            units: session::Units::default(),
        }),
        ..Default::default()
    }
}

#[test]
fn every_control_has_a_name() {
    let mut failures = Vec::new();
    let mut check = |what: &str, h: &H| {
        for control in unnamed_controls(h) {
            failures.push(format!("{what}: {control}"));
        }
    };
    for (what, f) in fixtures() {
        check(what, &harness(f));
    }
    // The Settings window on each tab.
    let mut h = harness(settings_open());
    check("Settings, Appearance", &h);
    click(&mut h, "Input");
    check("Settings, Input", &h);
    // Inside a group with the egui menus, each menu open in turn.
    let inside = || Fixture {
        recent: vec!["/m/house.skr".into(), "/m/shed.skr".into()],
        ..house_fixture(&["i1"])
    };
    // A dock whose top area and Outliner both overflow, so both show their scroll bars.
    let mut crowded = super::layout::crowded(40);
    crowded.dock.split = *crate::ui::dock::SPLIT.end();
    let h = harness_sized(crowded, Vec2::new(960.0, 600.0));
    check("a crowded dock", &h);
    for bar in ["Scroll Materials", "Scroll Outliner"] {
        assert!(h.query_by_label(bar).is_some(), "{bar}");
    }
    let h = harness(inside());
    check("inside a group", &h);
    // The open group's expand button says what it does and to which node.
    assert!(h.query_by_label("Collapse House").is_some());
    let titles: Vec<&str> = crate::menus::menu_bar(&inside().view(), false)
        .iter()
        .map(|m| m.title)
        .collect();
    assert!(!titles.is_empty());
    for title in titles {
        let mut h = harness(inside());
        click(&mut h, title);
        check(&format!("inside a group, the {title} menu open"), &h);
    }
    // A scene tab's menu, and a tab and a row being renamed.
    let mut h = harness(super::scenes::with_scenes());
    super::scenes::right_click(&mut h, "Top");
    check("a scene tab's menu", &h);
    click(&mut h, "Rename");
    h.run_ok();
    check("a scene tab renamed", &h);
    let mut h = harness(inside());
    click(&mut h, "File");
    // egui names a submenu's button with its arrow.
    click(&mut h, &format!("{} \u{23F5}", crate::menus::OPEN_RECENT));
    check("inside a group, File > Open Recent open", &h);
    assert!(
        failures.is_empty(),
        "unnamed controls:\n{}",
        failures.join("\n")
    );
}

#[test]
fn icon_buttons_are_named_by_their_label() {
    let glyphs = icons::all();
    for (what, f) in [
        ("the default", Fixture::default()),
        ("the house", house_fixture(&["i1"])),
    ] {
        let h = harness(f);
        let drawn: Vec<(String, Rect)> = drawn_text_rects(&h)
            .into_iter()
            .filter(|(text, _)| glyphs.iter().any(|g| text.contains(g)))
            .collect();
        assert!(!drawn.is_empty(), "{what}: no icons drawn");
        for (text, rect) in drawn {
            // The innermost node the glyph is drawn in: its button, or its label.
            let (node, at) = h
                .root()
                .children_recursive()
                .filter_map(|n| Some((n.accesskit_node(), bounds(&n.accesskit_node())?)))
                .filter(|(_, at)| at.contains(rect.center()))
                .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
                .unwrap_or_else(|| panic!("{what}: the icon {text:?} at {rect:?} is in no node"));
            assert!(
                name_of(&node).is_some(),
                "{what}: the icon {text:?} at {rect:?} is in {:?} at {at:?}, which has no name",
                node.role(),
            );
        }
    }
}

/// Sends AccessKit's `action` to the node named `label`, runs, and returns the actions.
pub(super) fn accesskit_action(h: &mut H, label: &str, action: Action) -> Vec<UiAction> {
    let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
    h.event(egui::Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action,
            target_tree,
            target_node,
            data: None,
        },
    ));
    h.run_ok();
    h.state_mut().take()
}

#[test]
fn a_slider_value_box_steps_on_increment_and_decrement() {
    // At the dock's widest the track is longest, where a rate of range / width falls below a
    // step and would round back to the start.
    let fixture = || {
        let mut row = brick();
        row.opacity = 0.4;
        row.emission = 2.0;
        Fixture {
            materials: vec![row],
            material: Some(MaterialId(31)),
            dock: DockLayout {
                width: *crate::ui::dock::DOCK_WIDTH.end(),
                split: *crate::ui::dock::SPLIT.end(),
                ..super::layout::materials_tab()
            },
            ..Default::default()
        }
    };
    let id = MaterialId(31);
    for (label, action, want) in [
        (
            "Opacity value",
            Action::Increment,
            UiAction::SetMaterialOpacity(id, 41),
        ),
        (
            "Opacity value",
            Action::Decrement,
            UiAction::SetMaterialOpacity(id, 39),
        ),
        (
            "Glow value",
            Action::Increment,
            UiAction::SetMaterialEmission(id, 21),
        ),
        (
            "Glow value",
            Action::Decrement,
            UiAction::SetMaterialEmission(id, 19),
        ),
    ] {
        let mut h = harness(fixture());
        assert_eq!(
            accesskit_action(&mut h, label, action),
            [want],
            "{label} {action:?}"
        );
    }
}
