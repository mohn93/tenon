use kernel::TagId;

use super::*;

const WALLS: TagId = TagId(1);
const ROOF: TagId = TagId(2);

/// Walls (shown, five entities on it) and Roof (hidden, nothing on it).
pub(super) fn two_tags() -> Vec<TagRow> {
    vec![
        TagRow {
            id: WALLS,
            name: "Walls".into(),
            color: [200, 80, 60],
            visible: true,
            count: 5,
        },
        TagRow {
            id: ROOF,
            name: "Roof".into(),
            color: [60, 120, 200],
            visible: false,
            count: 0,
        },
    ]
}

/// The dock on the Tags tab, with Walls and Roof.
pub(super) fn tags_tab() -> Fixture {
    Fixture {
        tags: two_tags(),
        dock: DockLayout {
            tab: DockTab::Tags,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn the_tags_tab_lists_tags_with_their_visibility_and_counts() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click(&mut h, "Tags"),
        [UiAction::SetDock(DockLayout {
            tab: DockTab::Tags,
            ..Default::default()
        })]
    );
    let h = harness(tags_tab());
    for label in ["Walls", "Roof", "Hide Walls", "Show Roof", "5", "0"] {
        h.get_by_label(label);
    }
    for label in ["Color of Walls", "Color of Roof"] {
        h.get_by_label(label);
    }
    assert_eq!(is_on(&h, "Color by tag"), Some(false));
    // The count is on the name's row, at the right.
    let walls = rect_of(&h, "Walls");
    let count = rect_of(&h, "5");
    assert!(count.min.x >= walls.max.x - 1.0 && count.center().y - walls.center().y < 2.0);
    let h = harness(Fixture {
        tags: Vec::new(),
        ..tags_tab()
    });
    h.get_by_label("No tags");
}

#[test]
fn the_eye_shows_and_hides_and_the_toggle_colors_by_tag() {
    for (label, action) in [
        ("Hide Walls", UiAction::SetTagVisible(WALLS, false)),
        ("Show Roof", UiAction::SetTagVisible(ROOF, true)),
        ("Color by tag", UiAction::SetColorByTag(true)),
        ("Add tag", UiAction::AddTag),
    ] {
        let mut h = harness(tags_tab());
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    let mut h = harness(Fixture {
        color_by_tag: true,
        ..tags_tab()
    });
    assert_eq!(is_on(&h, "Color by tag"), Some(true));
    assert_eq!(
        click(&mut h, "Color by tag"),
        [UiAction::SetColorByTag(false)]
    );
}

#[test]
fn delete_deletes_the_chosen_tag() {
    let mut h = harness(tags_tab());
    assert_eq!(click(&mut h, "Delete tag"), [], "no tag chosen");
    assert_eq!(click(&mut h, "Roof"), []);
    assert_eq!(is_on(&h, "Roof"), Some(true), "chosen");
    assert_eq!(click(&mut h, "Delete tag"), [UiAction::DeleteTag(ROOF)]);
    // A chosen tag that goes away is forgotten.
    h.state_mut().tags.truncate(1);
    h.run_ok();
    assert_eq!(click(&mut h, "Delete tag"), []);
}

#[test]
fn double_clicking_a_name_renames_the_tag() {
    let mut h = harness(tags_tab());
    let walls = rect_of(&h, "Walls").center();
    click_at(&mut h, walls, egui::Modifiers::NONE);
    assert_eq!(click_at(&mut h, walls, egui::Modifiers::NONE), []);
    h.run_ok();
    // The field took focus with the name selected: typing replaces it.
    h.event(egui::Event::Text("Old walls".into()));
    h.run_ok();
    assert_eq!(h.state_mut().take(), [], "nothing while typing");
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(
        h.state_mut().take(),
        [UiAction::RenameTag(WALLS, "Old walls".into())]
    );
    assert!(h.query_by_label("Tag name").is_none(), "the field closed");
    h.get_by_label("Walls");
    // Escape keeps the name.
    let mut h = harness(tags_tab());
    click_at(&mut h, walls, egui::Modifiers::NONE);
    click_at(&mut h, walls, egui::Modifiers::NONE);
    h.run_ok();
    h.event(egui::Event::Text("Old".into()));
    h.run_ok();
    h.key_press(egui::Key::Escape);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
    assert!(h.query_by_label("Tag name").is_none());
}

/// The egui memory key of the Walls color picker's popup.
fn walls_picker() -> egui::Id {
    egui::Id::new(("tag-color", WALLS.0)).with("popup")
}

/// Opens Walls' color picker and drags across its color square; nothing is reported.
fn open_and_pick(h: &mut H) {
    assert_eq!(click(h, "Color of Walls"), [], "the picker opens");
    let popup = walls_picker();
    let area = h
        .ctx
        .memory(|m| m.area_rect(popup))
        .expect("the picker is open");
    // The picker's color square: the square node inside it.
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
        .expect("the color square");
    // A drag across it: nothing while dragging.
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
fn a_picked_color_is_one_change_when_the_picker_closes() {
    let mut h = harness(tags_tab());
    open_and_pick(&mut h);
    // A click outside closes it: the color picked last is set, once.
    let outside = h.state().central.center();
    let mut actions = click_at(&mut h, outside, egui::Modifiers::NONE);
    h.run_ok();
    actions.extend(h.state_mut().take());
    assert!(
        matches!(actions.as_slice(), [UiAction::RecolorTag(WALLS, c)] if *c != [200, 80, 60]),
        "{actions:?}"
    );
    // Opening and closing it without picking changes nothing.
    let mut h = harness(tags_tab());
    click(&mut h, "Color of Walls");
    assert_eq!(click_at(&mut h, outside, egui::Modifiers::NONE), []);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn deleting_a_used_tag_asks_where_its_entities_go() {
    let sheet = || Fixture {
        sheet: Some(Sheet::DeleteTag {
            tag: WALLS,
            name: "Walls".into(),
            count: 5,
            others: vec![(ROOF, "Roof".into())],
        }),
        ..Default::default()
    };
    let mut h = harness(sheet());
    assert!(
        drawn_texts(&h)
            .iter()
            .any(|t| t == "5 entities are on Walls.")
    );
    assert_eq!(click(&mut h, "Delete"), [UiAction::ConfirmDeleteTag(None)]);
    let mut h = harness(sheet());
    assert_eq!(click(&mut h, "Cancel"), [UiAction::CloseSheet]);
    let mut h = harness(sheet());
    click(&mut h, "Move to");
    click(&mut h, "Roof");
    assert_eq!(
        click(&mut h, "Delete"),
        [UiAction::ConfirmDeleteTag(Some(ROOF))]
    );
}

/// Entity Info for a selection on `tag`.
fn selection_on(tag: SelectionTag) -> Fixture {
    Fixture {
        tags: two_tags(),
        info: vec!["2 edges".into()],
        has_selection: true,
        selection_tag: Some(tag),
        ..Default::default()
    }
}

#[test]
fn entity_info_puts_the_selection_on_a_tag() {
    let mut h = harness(selection_on(SelectionTag::One(Some(WALLS))));
    assert!(drawn_texts(&h).iter().any(|t| t == "Walls"));
    click(&mut h, "Tag");
    assert_eq!(
        click(&mut h, "Roof"),
        [UiAction::SetSelectionTag(Some(ROOF))]
    );
    let mut h = harness(selection_on(SelectionTag::One(Some(WALLS))));
    click(&mut h, "Tag");
    assert_eq!(click(&mut h, "Untagged"), [UiAction::SetSelectionTag(None)]);
    let mut h = harness(selection_on(SelectionTag::One(Some(WALLS))));
    click(&mut h, "Tag");
    assert_eq!(click(&mut h, "Walls"), [], "already on it");
    // Different tags: "Mixed", and any choice applies.
    let mut h = harness(selection_on(SelectionTag::Mixed));
    assert!(drawn_texts(&h).iter().any(|t| t == "Mixed"));
    click(&mut h, "Tag");
    assert_eq!(
        click(&mut h, "Walls"),
        [UiAction::SetSelectionTag(Some(WALLS))]
    );
    let h = harness(selection_on(SelectionTag::One(None)));
    assert!(drawn_texts(&h).iter().any(|t| t == "Untagged"));
    // Nothing taggable selected (a vertex, or nothing): no field.
    let h = harness(Fixture {
        tags: two_tags(),
        ..Default::default()
    });
    assert!(h.query_by_label("Tag").is_none());
}

#[test]
fn a_color_an_agent_set_while_the_picker_is_open_stays_when_it_closes_unchanged() {
    let mut h = harness(tags_tab());
    click(&mut h, "Color of Walls");
    // An agent recolors Walls meanwhile.
    h.state_mut().tags[0].color = [1, 2, 3];
    h.run_ok();
    let outside = h.state().central.center();
    let mut actions = click_at(&mut h, outside, egui::Modifiers::NONE);
    h.run_ok();
    actions.extend(h.state_mut().take());
    assert_eq!(actions, [], "the picker's own color was not changed");
}

/// Whether anything is held for a picker or a rename.
fn held(h: &H) -> (bool, bool) {
    let renaming = h.ctx.data(|d| {
        d.get_temp::<(TagId, String, bool)>(egui::Id::new("tag-rename"))
            .is_some()
    });
    (renaming, egui::Popup::is_id_open(&h.ctx, walls_picker()))
}

#[test]
fn a_picker_whose_tab_closes_drops_its_color() {
    let mut h = harness(tags_tab());
    open_and_pick(&mut h);
    h.state_mut().dock.tab = DockTab::EntityInfo;
    h.run_ok();
    h.state_mut().dock.tab = DockTab::Tags;
    h.run_ok();
    h.run_ok();
    assert_eq!(h.state_mut().take(), [], "nothing committed later");
    assert!(!egui::Popup::is_id_open(&h.ctx, walls_picker()), "closed");
    // Opened again, it starts from the tag's color: closing it changes nothing.
    click(&mut h, "Color of Walls");
    let outside = h.state().central.center();
    assert_eq!(click_at(&mut h, outside, egui::Modifiers::NONE), []);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn a_tag_an_agent_deletes_while_renamed_or_recolored_is_forgotten() {
    let mut h = harness(tags_tab());
    let walls = rect_of(&h, "Walls").center();
    click_at(&mut h, walls, egui::Modifiers::NONE);
    click_at(&mut h, walls, egui::Modifiers::NONE);
    h.run_ok();
    h.event(egui::Event::Text("Old".into()));
    h.run_ok();
    assert!(held(&h).0, "renaming");
    // An agent deletes Walls.
    h.state_mut().tags.remove(0);
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
    assert!(!held(&h).0, "the typed name is dropped");
    assert!(h.query_by_label("Tag name").is_none());
    // The same while its color is picked.
    let mut h = harness(tags_tab());
    open_and_pick(&mut h);
    h.state_mut().tags.remove(0);
    h.run_ok();
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
    assert!(!held(&h).1, "the picker closed");
}

#[test]
fn the_list_follows_tags_an_agent_adds_and_an_undo_removes() {
    let mut h = harness(tags_tab());
    h.state_mut().tags.push(TagRow {
        id: TagId(3),
        name: "Tag".into(),
        color: [9, 9, 9],
        visible: true,
        count: 0,
    });
    h.run_ok();
    h.get_by_label("Tag");
    h.get_by_label("Color of Tag");
    h.state_mut().tags.pop();
    h.run_ok();
    assert!(h.query_by_label("Color of Tag").is_none());
}

#[test]
fn a_replaced_document_drops_the_rename_the_picker_and_the_choice() {
    // The new document has a tag under the same id: what was held must not reach it.
    let mut h = harness(tags_tab());
    let walls = rect_of(&h, "Walls").center();
    click_at(&mut h, walls, egui::Modifiers::NONE);
    click_at(&mut h, walls, egui::Modifiers::NONE);
    h.run_ok();
    h.event(egui::Event::Text("Old".into()));
    h.run_ok();
    assert!(held(&h).0, "renaming");
    h.state_mut().documents += 1;
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
    assert!(!held(&h).0, "the typed name is dropped");
    assert!(h.query_by_label("Tag name").is_none());
    // The chosen tag is forgotten too: Delete has nothing to delete.
    assert_eq!(click(&mut h, "Delete tag"), []);

    let mut h = harness(tags_tab());
    open_and_pick(&mut h);
    h.state_mut().documents += 1;
    h.run_ok();
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
    assert!(!held(&h).1, "the picker closed");
}
