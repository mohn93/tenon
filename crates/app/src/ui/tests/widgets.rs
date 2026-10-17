use super::*;
use crate::shortcuts::{Bound, chord};
use crate::ui::widgets;

/// A themed harness around `f`, settled; frames 50 ms apart.
pub(super) fn kit<'a>(f: impl FnMut(&mut Ui) + 'a) -> Harness<'a> {
    let mut h = Harness::builder().with_step_dt(0.05).build_ui(f);
    install_theme(&h.ctx);
    h.run_ok();
    h
}

#[test]
fn a_tool_button_is_named_for_its_tool_and_shows_its_key() {
    let mut clicked = false;
    let mut h = kit(|ui| {
        clicked |= widgets::tool_button(ui, ToolKind::Rectangle, true).clicked();
    });
    assert_eq!(is_on(&h, "Rectangle"), Some(true));
    let key = chord(Bound::Tool(ToolKind::Rectangle)).unwrap().text();
    assert!(
        !drawn_texts_of(&h).contains(&key),
        "no tooltip before hover"
    );
    h.get_by_label("Rectangle").click();
    h.run_ok();
    // Resting on the button shows the tooltip (which is also labelled "Rectangle").
    h.get_by_label("Rectangle").hover();
    h.run_steps(20);
    let texts = drawn_texts_of(&h);
    assert!(texts.contains(&key), "{key} in {texts:?}");
    drop(h);
    assert!(clicked);
}

#[test]
fn a_menu_item_is_named_without_its_key_and_shows_it() {
    let h = kit(|ui| {
        widgets::menu_item(ui, "Save As...", Some(Bound::SaveFileAs), true);
        widgets::menu_item(ui, "Delete Guides", None, false);
    });
    let save_as = h.query_by_label("Save As...");
    assert!(save_as.is_some_and(|n| !n.accesskit_node().is_disabled()));
    assert!(
        h.get_by_label("Delete Guides")
            .accesskit_node()
            .is_disabled()
    );
    let key = chord(Bound::SaveFileAs).unwrap().text();
    assert!(drawn_texts_of(&h).contains(&key));
}
