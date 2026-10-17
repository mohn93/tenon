use super::*;
use crate::shortcuts::{Bound, chord};
use crate::ui::widgets::{self, Fill};

/// A themed harness around `f`, settled; frames 50 ms apart.
pub(super) fn kit<'a>(f: impl FnMut(&mut Ui) + 'a) -> Harness<'a> {
    let mut f = f;
    let mut h = Harness::builder()
        .with_step_dt(0.05)
        .build_ui(move |ui: &mut Ui| {
            if !themed_first_pass(ui.ctx()) {
                f(ui);
            }
        });
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
    let side = Vec2::splat(theme::RAIL_BUTTON);
    assert_eq!(h.get_by_label("Rectangle").rect().size(), side);
    assert!(drawn_texts_of(&h).contains(&crate::ui::icons::tool(ToolKind::Rectangle).to_string()));
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

#[test]
fn a_section_header_is_named_as_written_and_drawn_uppercase() {
    let h = kit(|ui| {
        widgets::section_header(ui, "Entity Info");
    });
    h.get_by_label("Entity Info");
    assert!(drawn_texts_of(&h).iter().any(|t| t == "ENTITY INFO"));
}

#[test]
fn a_swatch_is_named_and_marked_when_chosen() {
    let h = kit(|ui| {
        widgets::swatch(ui, Fill::Color(egui::Color32::RED), "Brick color", true);
        widgets::swatch(ui, Fill::Color(egui::Color32::BLUE), "Glass color", false);
    });
    assert_eq!(is_on(&h, "Brick color"), Some(true));
    assert_eq!(is_on(&h, "Glass color"), Some(false));
}

#[test]
fn a_numeric_row_reports_a_typed_value_once_entered() {
    let key = egui::Id::new("test-row");
    let mut done = Vec::new();
    let mut h = kit(|ui| {
        if let Some(d) =
            widgets::numeric_row(ui, key, "Size (m)", [1.0, 0.5], 0.0..=10.0, 0.01).done
        {
            done.push(d);
        }
    });
    h.get_all_by_label("Size (m)").nth(1).unwrap().click();
    h.run_ok();
    h.event(egui::Event::Text("2".into()));
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    drop(h);
    assert_eq!(done, [([1.0, 0.5], [1.0, 2.0])]);
}

#[test]
fn a_numeric_rows_values_are_mono_and_its_label_is_not() {
    let h = kit(|ui| {
        widgets::numeric_row(
            ui,
            egui::Id::new("font-row"),
            "Rotation",
            [12.5],
            0.0..=90.0,
            0.1,
        );
    });
    let mut fonts = Vec::new();
    fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::FontFamily)>) {
        match shape {
            egui::Shape::Text(t) => {
                let family = t.galley.job.sections[0].format.font_id.family.clone();
                out.push((t.galley.text().to_string(), family));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    for clipped in &h.output().shapes {
        collect(&clipped.shape, &mut fonts);
    }
    let family = |text: &str| fonts.iter().find(|(t, _)| t.contains(text)).map(|f| &f.1);
    assert_eq!(
        family("12.5"),
        Some(&egui::FontFamily::Monospace),
        "{fonts:?}"
    );
    assert_eq!(
        family("Rotation"),
        Some(&egui::FontFamily::Proportional),
        "{fonts:?}"
    );
}

#[test]
fn an_icon_button_is_named_and_can_be_disabled() {
    let mut h = kit(|ui| {
        widgets::icon_button(
            ui,
            crate::ui::icons::UNDO,
            "Undo",
            "Nothing to undo",
            Some(Bound::Undo),
            false,
        );
    });
    assert!(h.get_by_label("Undo").accesskit_node().is_disabled());
    h.get_by_label("Undo").hover();
    h.run_steps(20);
    assert!(drawn_texts_of(&h).iter().any(|t| t == "Nothing to undo"));
}
