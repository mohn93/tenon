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
