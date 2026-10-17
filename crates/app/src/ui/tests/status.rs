use super::*;

#[test]
fn the_status_bar_is_one_row_on_one_baseline() {
    let mut f = Fixture {
        connected: 2,
        selection: faces(&[3, 4, 5]).into_iter().collect(),
        ..Default::default()
    };
    f.status.lock = Some("Locked: red axis");
    let h = harness(f);
    let bar_top = h.state().central.max.y;
    assert_eq!(SCREEN.y - bar_top, theme::STATUS_HEIGHT);
    let center = rect_of(&h, "Tool: Line").center().y;
    for label in [
        "Click to start a line",
        "Locked: red axis",
        "Length",
        "3 selected",
        "Agents: 2",
    ] {
        let r = rect_of(&h, label);
        assert!(
            r.min.y >= bar_top && (r.center().y - center).abs() <= 1.0,
            "{label}: {r:?}"
        );
    }
    assert_eq!(rect_of(&h, "Length").width(), theme::MEASUREMENTS_WIDTH);
}

#[test]
fn the_measurements_box_is_labelled_mono_and_hidden_in_select() {
    let h = harness(Fixture::default());
    assert_eq!(
        h.get_by_label("Length").accesskit_node().value().as_deref(),
        Some("2.5_")
    );
    let mut f = Fixture::default();
    f.status.label = "";
    harness(f).get_by_label("Measurements");
    let h = harness(Fixture {
        tool: ToolKind::Select,
        ..Default::default()
    });
    assert!(h.query_by_label("Length").is_none());
    let h = harness(Fixture {
        flying: true,
        ..Default::default()
    });
    assert!(h.query_by_label("Length").is_none());
    assert!(crate::ui::status::typing(Some("2.5_")) && !crate::ui::status::typing(Some("2.5 m")));
}

#[test]
fn locks_are_colored_for_their_axis() {
    use crate::ui::status::lock_color;
    let accent = theme::DARK.accent;
    assert_eq!(lock_color("Locked: red axis", accent), theme::AXIS_X);
    assert_eq!(lock_color("Locked: green axis", accent), theme::AXIS_Y);
    assert_eq!(lock_color("Locked: blue axis", accent), theme::AXIS_Z);
    assert_eq!(lock_color("Locked: parallel", accent), accent);
}
