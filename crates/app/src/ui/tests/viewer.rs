use super::*;
use crate::platform::Features;

fn viewer() -> Fixture {
    let mut f = Fixture {
        features: Features::VIEWER,
        // Everything an editor shows is on, so what the viewer hides is seen to be hidden.
        undo: Some("push_pull".into()),
        has_selection: true,
        has_instance_selected: true,
        start: Some(Vec::new()),
        ..Default::default()
    };
    f.status.message = None;
    f
}

/// A phone: 375 x 812 points (a 3x display of 1125 x 2436 pixels).
const PHONE: Vec2 = Vec2::new(375.0, 812.0);

#[test]
fn the_viewer_is_a_top_bar_over_a_full_view() {
    let h = harness_built(viewer(), PHONE, 3.0);
    let rect = h.state().central;
    assert_eq!(rect.min.x, 0.0, "no rail: {rect:?}");
    assert_eq!(rect.min.y, crate::ui::viewer_bar::BAR_HEIGHT, "{rect:?}");
    assert_eq!(rect.max, Pos2::new(PHONE.x, PHONE.y), "no dock or status");
    assert!(
        h.query_by_label("Shared model").is_some(),
        "the bar names the model"
    );
}

#[test]
fn the_viewer_has_no_menus_tools_or_editing_buttons() {
    let h = harness_sized(viewer(), PHONE);
    for label in [
        "File",
        "Edit",
        "View",
        "Camera",
        "Undo",
        "Redo",
        "Make Group",
        "Make Component",
        "Explode",
        "Select",
        "Line",
        "Rectangle",
        "Push/Pull",
        "Move",
        "Paint Bucket",
        "Entity Info",
        "Materials",
        "Tags",
        "Scenes",
    ] {
        assert!(h.query_by_label(label).is_none(), "{label} is shown");
    }
    for tool in ToolKind::ALL {
        assert!(
            h.query_by_label(tool.label()).is_none(),
            "{} is shown",
            tool.label()
        );
    }
}

#[test]
fn a_message_shows_under_the_bar() {
    let mut f = viewer();
    f.status.message = Some("This link is damaged or expired".into());
    let h = harness_sized(f, PHONE);
    assert!(
        h.query_by_label("This link is damaged or expired")
            .is_some()
    );
    assert!(h.state().central.min.y > 44.0);
}

// The bar, lists and card, at a phone's size.

use crate::editor::PartCard;
use crate::platform::RecentShare;
use crate::ui::ViewerPanel;
use crate::viewer::parts::PartRow;
use kernel::{DefinitionKind, InstanceId};

fn row(name: &str, count: usize, size: [f64; 3], ids: &[u64]) -> PartRow {
    PartRow {
        name: name.into(),
        kind: if count > 1 {
            DefinitionKind::Component
        } else {
            DefinitionKind::Group
        },
        count,
        size: Some(size),
        ids: ids.iter().map(|i| InstanceId(*i)).collect(),
    }
}

fn click(h: &mut H, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

#[test]
fn the_bar_has_parts_measure_and_recent_buttons_a_finger_can_hit() {
    let mut h = harness_built(viewer(), PHONE, 3.0);
    for label in ["Parts", "Measure", "Recent"] {
        let rect = h.get_by_label(label).rect();
        assert!(
            rect.height() >= 36.0,
            "{label} is {} pt tall",
            rect.height()
        );
        assert!(rect.width() >= 44.0, "{label} is {} pt wide", rect.width());
        assert!(rect.max.x <= PHONE.x, "{label} is on screen");
    }
    assert!(
        h.query_by_label("Shared model").is_some(),
        "the name still shows beside them"
    );
    click(&mut h, "Parts");
    assert_eq!(
        h.state_mut().take(),
        vec![UiAction::ViewerPanel(Some(ViewerPanel::Parts))]
    );
    click(&mut h, "Recent");
    assert_eq!(
        h.state_mut().take(),
        vec![UiAction::ViewerPanel(Some(ViewerPanel::Recent))]
    );
    click(&mut h, "Measure");
    assert_eq!(h.state_mut().take(), vec![UiAction::MeasureMode(true)]);
}

#[test]
fn an_open_list_closes_from_its_own_button() {
    let mut f = viewer();
    f.viewer_panel = Some(ViewerPanel::Parts);
    let mut h = harness_built(f, PHONE, 3.0);
    click(&mut h, "Parts");
    assert_eq!(h.state_mut().take(), vec![UiAction::ViewerPanel(None)]);
    click(&mut h, "Close");
    assert_eq!(h.state_mut().take(), vec![UiAction::ViewerPanel(None)]);
}

#[test]
fn the_parts_list_rows_are_tall_enough_and_select_their_parts() {
    let mut f = viewer();
    f.viewer_panel = Some(ViewerPanel::Parts);
    f.part_rows = vec![
        row("Shed", 1, [2.0, 1.0, 3.0], &[1]),
        row("Crate", 3, [0.5, 0.5, 0.5], &[2, 3, 4]),
    ];
    let mut h = harness_built(f, PHONE, 3.0);
    assert!(h.query_by_label("Parts (2)").is_some());
    let shed = h.get_by_label("Shed, 2 m x 1 m x 3 m").rect();
    let crates = h.get_by_label("Crate  x3, 0.5 m x 0.5 m x 0.5 m").rect();
    assert!(
        shed.height() >= 44.0 && crates.height() >= 44.0,
        "{shed:?} {crates:?}"
    );
    assert!(shed.max.x <= PHONE.x && crates.max.x <= PHONE.x);
    assert!(shed.min.y > crate::ui::viewer_bar::BAR_HEIGHT);
    click(&mut h, "Crate  x3, 0.5 m x 0.5 m x 0.5 m");
    assert_eq!(
        h.state_mut().take(),
        vec![UiAction::SelectParts(vec![
            InstanceId(2),
            InstanceId(3),
            InstanceId(4)
        ])]
    );
}

#[test]
fn a_long_parts_list_stays_within_the_lower_part_of_the_screen() {
    let mut f = viewer();
    f.viewer_panel = Some(ViewerPanel::Parts);
    f.part_rows = (0..40)
        .map(|i| row(&format!("Part {i}"), 1, [1.0, 1.0, 1.0], &[i]))
        .collect();
    let h = harness_built(f, PHONE, 3.0);
    let first = h.get_by_label("Part 0, 1 m x 1 m x 1 m").rect();
    assert!(
        first.min.y > PHONE.y * 0.40,
        "the list leaves the model's upper part in view: {first:?}"
    );
}

#[test]
fn an_empty_parts_list_says_so() {
    let mut f = viewer();
    f.viewer_panel = Some(ViewerPanel::Parts);
    let h = harness_built(f, PHONE, 3.0);
    assert!(
        h.query_by_label_contains("no groups or components")
            .is_some()
    );
}

fn card(isolated: bool) -> PartCard {
    PartCard {
        title: "Crate".into(),
        size: Some("1.2 m x 0.6 m x 2 m".into()),
        ordinal: Some("2 of 3".into()),
        isolated,
    }
}

#[test]
fn the_card_shows_name_size_and_position_with_isolate() {
    let mut f = viewer();
    f.part_card = Some(card(false));
    let mut h = harness_built(f, PHONE, 3.0);
    assert!(h.query_by_label("Crate").is_some());
    assert!(h.query_by_label("1.2 m x 0.6 m x 2 m").is_some());
    assert!(h.query_by_label("2 of 3").is_some());
    assert!(h.query_by_label("Show all").is_none());
    let isolate = h.get_by_label("Isolate").rect();
    assert!(isolate.height() >= 44.0, "{isolate:?}");
    click(&mut h, "Isolate");
    assert_eq!(h.state_mut().take(), vec![UiAction::Isolate]);
}

#[test]
fn an_isolated_card_offers_show_all_and_so_does_the_bar_without_a_card() {
    let mut f = viewer();
    f.part_card = Some(card(true));
    f.isolated = true;
    let mut h = harness_built(f, PHONE, 3.0);
    assert!(h.query_by_label("Isolate").is_none());
    click(&mut h, "Show all");
    assert_eq!(h.state_mut().take(), vec![UiAction::ShowAll]);

    let mut f = viewer();
    f.isolated = true;
    let mut h = harness_built(f, PHONE, 3.0);
    assert!(h.query_by_label("Other parts are hidden").is_some());
    click(&mut h, "Show all");
    assert_eq!(h.state_mut().take(), vec![UiAction::ShowAll]);
}

#[test]
fn nothing_selected_and_nothing_hidden_shows_no_card() {
    let h = harness_built(viewer(), PHONE, 3.0);
    assert!(h.query_by_label("Isolate").is_none());
    assert!(h.query_by_label("Show all").is_none());
}

#[test]
fn measure_shows_its_reading_with_done_and_hides_the_card() {
    let mut f = viewer();
    f.measure = Some("Tap the first point".into());
    f.part_card = Some(card(false));
    let mut h = harness_built(f, PHONE, 3.0);
    assert!(h.query_by_label("Tap the first point").is_some());
    assert!(h.query_by_label("Isolate").is_none(), "taps measure now");
    let done = h.get_by_label("Done").rect();
    assert!(done.height() >= 44.0 && done.max.x <= PHONE.x, "{done:?}");
    click(&mut h, "Done");
    assert_eq!(h.state_mut().take(), vec![UiAction::MeasureMode(false)]);
}

#[test]
fn the_measure_button_leaves_measure_too() {
    let mut f = viewer();
    f.measure = Some("1.5 m".into());
    let mut h = harness_built(f, PHONE, 3.0);
    assert!(h.query_by_label("1.5 m").is_some());
    click(&mut h, "Measure");
    assert_eq!(h.state_mut().take(), vec![UiAction::MeasureMode(false)]);
}

fn kept(id: &str, name: &str) -> RecentShare {
    RecentShare {
        id: id.into(),
        name: name.into(),
        when: "5 min ago".into(),
        size: "1.2 MB".into(),
    }
}

#[test]
fn the_recent_list_opens_and_deletes() {
    let mut f = viewer();
    f.viewer_panel = Some(ViewerPanel::Recent);
    f.kept = vec![kept("a", "House"), kept("b", "Garden")];
    f.kept_note = Some("This model is over 64 MiB, so it was not kept.".into());
    let mut h = harness_built(f, PHONE, 3.0);
    assert!(
        h.query_by_label_contains("over 64 MiB").is_some(),
        "the list says why"
    );
    let house = h.get_by_label("House, 5 min ago  1.2 MB").rect();
    assert!(
        house.height() >= 44.0 && house.max.x <= PHONE.x,
        "{house:?}"
    );
    for delete in h.query_all_by_label("Delete") {
        let r = delete.rect();
        assert!(
            r.height() >= 44.0 && r.width() >= 44.0 && r.max.x <= PHONE.x,
            "{r:?}"
        );
    }
    click(&mut h, "Garden, 5 min ago  1.2 MB");
    assert_eq!(h.state_mut().take(), vec![UiAction::OpenKept("b".into())]);
    h.query_all_by_label("Delete").next().unwrap().click();
    h.run();
    assert_eq!(h.state_mut().take(), vec![UiAction::DeleteKept("a".into())]);
}

#[test]
fn an_empty_recent_list_says_so() {
    let mut f = viewer();
    f.viewer_panel = Some(ViewerPanel::Recent);
    let h = harness_built(f, PHONE, 3.0);
    assert!(h.query_by_label_contains("Nothing kept").is_some());
}
