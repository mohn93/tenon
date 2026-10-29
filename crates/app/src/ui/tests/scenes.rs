use kernel::PageId;

use super::*;

const FRONT: PageId = PageId(40);
const TOP: PageId = PageId(41);
const BACK: PageId = PageId(42);

/// Front (selected, stores both), Top (the camera only) and Back (tag visibility only).
pub(super) fn three_scenes() -> Vec<SceneRow> {
    let row = |id, name: &str, camera, tags, selected| SceneRow {
        id,
        name: name.into(),
        camera,
        tags,
        positions: false,
        joints: false,
        sections: false,
        selected,
    };
    vec![
        row(FRONT, "Front", true, true, true),
        row(TOP, "Top", true, false, false),
        row(BACK, "Back", false, true, false),
    ]
}

/// Three scenes, the dock on Entity Info: each name is only the tab's.
pub(super) fn with_scenes() -> Fixture {
    Fixture {
        scenes: three_scenes(),
        ..Default::default()
    }
}

/// The dock on the Scenes tab, with the three scenes (their names are also the tabs').
pub(super) fn scenes_tab() -> Fixture {
    Fixture {
        dock: DockLayout {
            tab: DockTab::Scenes,
            ..Default::default()
        },
        ..with_scenes()
    }
}

/// The node named `label` in the Scenes tab, not the scene tab of that name (the lower one).
fn panel_row<'a>(h: &'a H, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(label)
        .max_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y))
        .unwrap()
}

/// The checkbox named `label` (the Camera menu shares its name with the Camera checkbox).
fn checkbox<'a>(h: &'a H, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(label)
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::CheckBox)
        .unwrap_or_else(|| panic!("the {label} checkbox"))
}

/// Whether the checkbox named `label` is checked.
fn checked(h: &H, label: &str) -> bool {
    checkbox(h, label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// Clicks the checkbox named `label` and runs. Returns the actions.
fn click_checkbox(h: &mut H, label: &str) -> Vec<UiAction> {
    checkbox(h, label).click();
    h.run_ok();
    h.state_mut().take()
}

/// Presses and releases the secondary button on the widget named `label`, then runs.
pub(super) fn right_click(h: &mut H, label: &str) {
    let pos = rect_of(h, label).center();
    h.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.run_ok();
}

#[test]
fn the_tabs_show_only_with_scenes_right_over_the_view() {
    let h = harness(Fixture::default());
    assert_eq!(h.state().central.min.y, theme::CONTEXT_BAR_HEIGHT);
    assert!(h.query_by_label("Front").is_none());
    let h = harness(with_scenes());
    let central = h.state().central;
    assert_eq!(
        central.min.y,
        theme::CONTEXT_BAR_HEIGHT + theme::SCENE_TABS_HEIGHT
    );
    let front = rect_of(&h, "Front");
    let top = rect_of(&h, "Top");
    let back = rect_of(&h, "Back");
    // In order, left to right, between the rail and the dock, under the context bar.
    assert!(front.max.x <= top.min.x && top.max.x <= back.min.x);
    assert!(front.min.x >= central.min.x && front.min.y >= theme::CONTEXT_BAR_HEIGHT);
    assert!(front.max.y <= central.min.y);
    assert_eq!(is_on(&h, "Front"), Some(true), "the selected scene's tab");
    assert_eq!(is_on(&h, "Top"), Some(false));
}

#[test]
fn a_click_goes_to_the_scene() {
    let mut h = harness(with_scenes());
    assert_eq!(click(&mut h, "Top"), [UiAction::GoToScene(TOP)]);
    // The selected scene's own tab goes back to its view.
    assert_eq!(click(&mut h, "Front"), [UiAction::GoToScene(FRONT)]);
}

#[test]
fn a_double_click_renames_the_tab_once() {
    let mut h = harness(with_scenes());
    let top = rect_of(&h, "Top").center();
    assert_eq!(
        click_at(&mut h, top, egui::Modifiers::NONE),
        [UiAction::GoToScene(TOP)]
    );
    assert_eq!(click_at(&mut h, top, egui::Modifiers::NONE), []);
    h.run_ok();
    // The field took focus with the name selected: typing replaces it.
    h.event(egui::Event::Text("Plan".into()));
    h.run_ok();
    assert_eq!(h.state_mut().take(), [], "nothing while typing");
    assert!(h.query_by_label("Scene name").is_some());
    h.key_press(egui::Key::Enter);
    h.run_ok();
    h.run_ok();
    assert_eq!(
        h.state_mut().take(),
        [UiAction::RenameScene(TOP, "Plan".into())]
    );
    assert!(h.query_by_label("Scene name").is_none(), "the tab is back");
    // Escape keeps the name.
    let front = rect_of(&h, "Front").center();
    click_at(&mut h, front, egui::Modifiers::NONE);
    click_at(&mut h, front, egui::Modifiers::NONE);
    h.state_mut().take();
    h.event(egui::Event::Text("Gone".into()));
    h.run_ok();
    h.key_press(egui::Key::Escape);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
    assert!(h.query_by_label("Scene name").is_none());
}

#[test]
fn a_rename_is_forgotten_when_its_scene_goes() {
    let mut h = harness(with_scenes());
    let top = rect_of(&h, "Top").center();
    click_at(&mut h, top, egui::Modifiers::NONE);
    click_at(&mut h, top, egui::Modifiers::NONE);
    h.run_ok();
    assert!(h.query_by_label("Scene name").is_some());
    h.state_mut().scenes.remove(1);
    h.run_ok();
    assert!(h.query_by_label("Scene name").is_none());
    h.state_mut().take();
    // Nor does it come back when a scene with that id is listed again.
    h.state_mut().scenes = three_scenes();
    h.run_ok();
    assert!(h.query_by_label("Scene name").is_none());
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn a_rename_is_forgotten_when_the_document_is_replaced() {
    for f in [with_scenes(), scenes_tab()] {
        let mut h = harness(f);
        // The tab's rename, or the Scenes tab's row's.
        let at = if h.state().dock.tab == DockTab::Scenes {
            panel_row(&h, "Top").rect().center()
        } else {
            rect_of(&h, "Top").center()
        };
        click_at(&mut h, at, egui::Modifiers::NONE);
        click_at(&mut h, at, egui::Modifiers::NONE);
        h.run_ok();
        h.event(egui::Event::Text("Plan".into()));
        h.run_ok();
        assert!(h.query_by_label("Scene name").is_some());
        h.state_mut().take();
        // New or open: the same id is listed, but it names the new document's scene.
        h.state_mut().documents += 1;
        h.run_ok();
        h.run_ok();
        assert!(h.query_by_label("Scene name").is_none());
        assert_eq!(h.state_mut().take(), [], "the typed name is dropped");
    }
}

#[test]
fn the_tab_menu_updates_renames_moves_deletes_and_adds() {
    for (item, action) in [
        ("Update", Some(UiAction::UpdateScene(TOP))),
        ("Move Left", Some(UiAction::MoveScene(TOP, 0))),
        ("Move Right", Some(UiAction::MoveScene(TOP, 2))),
        ("Delete", Some(UiAction::DeleteScene(TOP))),
        ("Add Scene", Some(UiAction::AddScene)),
        ("Rename", None),
    ] {
        let mut h = harness(with_scenes());
        right_click(&mut h, "Top");
        assert_eq!(h.state_mut().take(), [], "opening the menu does nothing");
        assert!(h.query_by_label("Top menu").is_some(), "{item}");
        assert_eq!(click(&mut h, item), action.into_iter().collect::<Vec<_>>());
        assert!(h.query_by_label("Top menu").is_none(), "{item} closes it");
        if item == "Rename" {
            h.run_ok();
            h.event(egui::Event::Text("Plan".into()));
            h.run_ok();
            h.key_press(egui::Key::Enter);
            h.run_ok();
            assert_eq!(
                h.state_mut().take(),
                [UiAction::RenameScene(TOP, "Plan".into())]
            );
        }
    }
}

#[test]
fn the_first_and_last_tabs_do_not_move_past_the_ends() {
    let mut h = harness(with_scenes());
    right_click(&mut h, "Front");
    assert_eq!(click(&mut h, "Move Left"), [], "the first tab");
    let mut h = harness(with_scenes());
    right_click(&mut h, "Back");
    assert_eq!(click(&mut h, "Move Right"), [], "the last tab");
    right_click(&mut h, "Back");
    assert_eq!(click(&mut h, "Move Left"), [UiAction::MoveScene(BACK, 1)]);
}

#[test]
fn the_scenes_tab_lists_scenes_and_goes_to_them() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click(&mut h, "Scenes"),
        [UiAction::SetDock(DockLayout {
            tab: DockTab::Scenes,
            ..Default::default()
        })]
    );
    let h = harness(Fixture {
        scenes: Vec::new(),
        ..scenes_tab()
    });
    h.get_by_label("No scenes");
    let mut h = harness(scenes_tab());
    for name in ["Front", "Top", "Back"] {
        assert_eq!(h.get_all_by_label(name).count(), 2, "a tab and a row");
    }
    let row = panel_row(&h, "Back").rect().center();
    assert!(row.x > h.state().central.max.x, "in the dock");
    assert_eq!(
        click_at(&mut h, row, egui::Modifiers::NONE),
        [UiAction::GoToScene(BACK)]
    );
}

#[test]
fn the_scenes_tab_buttons_act_on_the_selected_scene() {
    for (label, action) in [
        ("Add scene", UiAction::AddScene),
        ("Delete scene", UiAction::DeleteScene(TOP)),
        ("Update scene", UiAction::UpdateScene(TOP)),
        ("Move scene up", UiAction::MoveScene(TOP, 0)),
        ("Move scene down", UiAction::MoveScene(TOP, 2)),
    ] {
        let mut f = scenes_tab();
        f.scenes[0].selected = false;
        f.scenes[1].selected = true;
        let mut h = harness(f);
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    // The first scene does not move up, the last not down; with none selected only Add acts.
    let mut h = harness(scenes_tab());
    assert_eq!(click(&mut h, "Move scene up"), []);
    let mut f = scenes_tab();
    f.scenes[0].selected = false;
    f.scenes[2].selected = true;
    let mut h = harness(f);
    assert_eq!(click(&mut h, "Move scene down"), []);
    let mut f = scenes_tab();
    f.scenes[0].selected = false;
    let mut h = harness(f);
    for label in [
        "Delete scene",
        "Update scene",
        "Move scene up",
        "Move scene down",
    ] {
        assert_eq!(click(&mut h, label), [], "{label}");
    }
}

#[test]
fn the_checkboxes_say_and_change_what_the_selected_scene_stores() {
    let mut f = scenes_tab();
    f.scenes[0].selected = false;
    f.scenes[1].selected = true;
    let mut h = harness(f);
    assert!(checked(&h, "Camera"));
    assert!(!checked(&h, "Tag visibility"));
    assert_eq!(
        click_checkbox(&mut h, "Camera"),
        [UiAction::StoreSceneCamera(TOP, false)]
    );
    assert_eq!(
        click_checkbox(&mut h, "Tag visibility"),
        [UiAction::StoreSceneTags(TOP, true)]
    );
    assert!(!checked(&h, "Positions"));
    assert_eq!(
        click_checkbox(&mut h, "Positions"),
        [UiAction::StoreScenePositions(TOP, true)]
    );
    assert!(!checked(&h, "Joint values"));
    assert_eq!(
        click_checkbox(&mut h, "Joint values"),
        [UiAction::StoreSceneJoints(TOP, true)]
    );
    assert!(!checked(&h, "Section planes"));
    assert_eq!(
        click_checkbox(&mut h, "Section planes"),
        [UiAction::StoreSceneSections(TOP, true)]
    );
    // With none selected they are off and do nothing.
    let mut f = scenes_tab();
    f.scenes[0].selected = false;
    let mut h = harness(f);
    assert!(!checked(&h, "Camera"));
    assert_eq!(click_checkbox(&mut h, "Camera"), []);
}

#[test]
fn a_double_clicked_row_renames_the_scene() {
    let mut h = harness(scenes_tab());
    let row = panel_row(&h, "Back").rect().center();
    assert_eq!(
        click_at(&mut h, row, egui::Modifiers::NONE),
        [UiAction::GoToScene(BACK)]
    );
    assert_eq!(click_at(&mut h, row, egui::Modifiers::NONE), []);
    h.run_ok();
    h.event(egui::Event::Text("Rear".into()));
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(
        h.state_mut().take(),
        [UiAction::RenameScene(BACK, "Rear".into())]
    );
    // Only the row was renamed, not the tab: one field at a time.
    assert_eq!(h.get_all_by_label("Back").count(), 2);
}

#[test]
fn the_view_menu_adds_scenes_and_steps_through_them() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click_menu(&mut h, &["View", "Add Scene"]),
        [UiAction::AddScene]
    );
    // With no scenes there is nothing to step through.
    assert_eq!(click_menu(&mut h, &["View", "Next Scene"]), []);
    let mut h = harness(with_scenes());
    assert_eq!(
        click_menu(&mut h, &["View", "Previous Scene"]),
        [UiAction::PreviousScene]
    );
    assert_eq!(
        click_menu(&mut h, &["View", "Next Scene"]),
        [UiAction::NextScene]
    );
    click(&mut h, "View");
    let texts = drawn_texts(&h);
    for key in ["Page Up", "Page Down"] {
        assert!(texts.iter().any(|t| t == key), "{key} in {texts:?}");
    }
}
