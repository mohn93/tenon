use super::*;
use crate::ui::outliner::outliner_label;
use kernel::DefinitionKind;

/// Two "Box" components (definition d1) each placing the group i27 (definition d2).
fn boxes() -> OutlinerTable {
    let component = |id| table_row(id, "d1", DefinitionKind::Component, "Box");
    let group = table_row("i27", "d2", DefinitionKind::Group, "");
    OutlinerTable {
        roots: vec![component("i55"), component("i56")],
        by_definition: [
            ("d1".parse().unwrap(), vec![group]),
            ("d2".parse().unwrap(), Vec::new()),
        ]
        .into(),
    }
}

/// The boxes with `context` open and `selection` selected.
fn boxes_fixture(context: &[&str], selection: &[&str]) -> Fixture {
    let selection: BTreeSet<EntityId> = ids(selection).into_iter().collect();
    Fixture {
        info: vec!["1 group".into()],
        has_selection: !selection.is_empty(),
        has_instance_selected: !selection.is_empty(),
        inside_instance: !context.is_empty(),
        outliner: boxes(),
        context: ids(context),
        selection,
        ..Default::default()
    }
}

/// The name row (not the expand button) at `path`.
fn row<'a>(rows: &'a [Recorded], path: &[EntityId]) -> Option<&'a Recorded> {
    rows.iter().find(|r| !r.toggle && r.path == path)
}

fn row_at(rows: &[Recorded], path: &[EntityId]) -> Pos2 {
    row(rows, path)
        .unwrap_or_else(|| panic!("a row for {path:?} in {rows:?}"))
        .rect
        .center()
}

fn toggle_at(rows: &[Recorded], path: &[EntityId]) -> Pos2 {
    rows.iter()
        .find(|r| r.toggle && r.path == path)
        .unwrap_or_else(|| panic!("an expand button for {path:?}"))
        .rect
        .center()
}

#[test]
fn outliner_labels_name_groups_and_components() {
    let table = boxes();
    assert_eq!(outliner_label(&table.roots[0]), "<Box>");
    assert_eq!(
        outliner_label(&table.children(table.roots[0].definition)[0]),
        "Group"
    );
    let named = table_row("i3", "d4", DefinitionKind::Group, "Wall");
    assert_eq!(outliner_label(&named), "Wall");
}

#[test]
fn outliner_rows_show_the_open_path_and_the_selection_and_select_their_paths() {
    let fixture = || boxes_fixture(&["i55"], &["i27"]);
    let laid = rows(&harness(fixture()));
    // The first box is open, so expanded; the second starts collapsed.
    let names: Vec<&[EntityId]> = laid
        .iter()
        .filter(|r| !r.toggle)
        .map(|r| r.path.as_slice())
        .collect();
    assert_eq!(
        names,
        [ids(&[]), ids(&["i55"]), ids(&["i55", "i27"]), ids(&["i56"])]
    );
    let marks = |path: &[&str]| {
        let r = row(&laid, &ids(path)).unwrap();
        (r.open, r.selected)
    };
    assert_eq!(marks(&["i55"]), (true, false));
    assert_eq!(marks(&["i55", "i27"]), (false, true));
    assert_eq!(marks(&["i56"]), (false, false));
    for (path, modifiers, toggle) in [
        (&["i55"][..], egui::Modifiers::NONE, false),
        (&["i55", "i27"], egui::Modifiers::NONE, false),
        (&["i56"], egui::Modifiers::NONE, false),
        (&["i56"], egui::Modifiers::SHIFT, true),
        (&["i56"], egui::Modifiers::COMMAND, true),
        (&["i56"], egui::Modifiers::CTRL, true),
    ] {
        // A fresh harness for each, so no two clicks make a double-click.
        let mut h = harness(fixture());
        let clicked = click_at(&mut h, row_at(&laid, &ids(path)), modifiers);
        let wanted = UiAction::OutlinerSelect {
            path: ids(path),
            toggle,
        };
        assert_eq!(clicked, [wanted], "{path:?} with {modifiers:?}");
    }
}

#[test]
fn the_model_row_goes_back_to_the_root_only_from_inside() {
    for (context, wanted) in [
        (&["i55"][..], vec![UiAction::OpenPath(Vec::new())]),
        (&[][..], Vec::new()),
    ] {
        let mut h = harness(boxes_fixture(context, &[]));
        let laid = rows(&h);
        let model = row(&laid, &[]).unwrap();
        assert_eq!(model.open, context.is_empty(), "bold at the root");
        assert_eq!(
            click_at(&mut h, model.rect.center(), egui::Modifiers::NONE),
            wanted
        );
    }
}

#[test]
fn double_clicking_an_outliner_row_opens_it_and_expanding_shows_children() {
    let mut h = harness(boxes_fixture(&["i55"], &[]));
    let inner = row_at(&rows(&h), &ids(&["i55", "i27"]));
    let mut actions = click_at(&mut h, inner, egui::Modifiers::NONE);
    actions.extend(click_at(&mut h, inner, egui::Modifiers::NONE));
    assert!(
        actions.contains(&UiAction::OpenPath(ids(&["i55", "i27"]))),
        "{actions:?}"
    );
    // The expand button left of the second box shows its group.
    let mut h = harness(boxes_fixture(&["i55"], &[]));
    let button = toggle_at(&rows(&h), &ids(&["i56"]));
    assert_eq!(click_at(&mut h, button, egui::Modifiers::NONE), []);
    let wanted = UiAction::OutlinerSelect {
        path: ids(&["i56", "i27"]),
        toggle: false,
    };
    let at = row_at(&rows(&h), &ids(&["i56", "i27"]));
    // Far enough from the last click in time not to make a double-click.
    h.run_steps(10);
    assert_eq!(click_at(&mut h, at, egui::Modifiers::NONE), [wanted]);
}

#[test]
fn a_newly_opened_path_expands_its_ancestors() {
    let mut h = harness(boxes_fixture(&[], &[]));
    assert!(row(&rows(&h), &ids(&["i56", "i27"])).is_none(), "collapsed");
    let context = ids(&["i56", "i27"]);
    h.state_mut().context = context.clone();
    h.state_mut().inside_instance = true;
    h.run_ok();
    let laid = rows(&h);
    assert!(row(&laid, &context).expect("the open group shows").open);
    // Collapsing it by hand sticks while the context stays the same.
    click_at(
        &mut h,
        toggle_at(&laid, &ids(&["i56"])),
        egui::Modifiers::NONE,
    );
    h.run_ok();
    assert!(row(&rows(&h), &context).is_none(), "{:?}", rows(&h));
}
