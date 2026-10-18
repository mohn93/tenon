use super::*;
use crate::editor::{ImportAs, ImportOptions, ImportUnits, RenameTarget, UpAxis};
use kernel::DefinitionKind;

/// The Import Options sheet for /m/chair.obj with the default options.
fn import_fixture() -> Fixture {
    Fixture {
        sheet: Some(Sheet::Import {
            path: "/m/chair.obj".into(),
            options: ImportOptions::default(),
        }),
        ..Default::default()
    }
}

#[test]
fn the_import_sheet_reports_its_options() {
    let mut h = harness(import_fixture());
    assert!(drawn_texts(&h).iter().any(|t| t == "chair.obj"));
    assert_eq!(click(&mut h, "Units"), []);
    assert_eq!(click(&mut h, "Millimeters"), []);
    assert_eq!(click(&mut h, "Z up"), []);
    assert_eq!(click(&mut h, "Loose geometry"), []);
    assert_eq!(click(&mut h, "Merge coplanar faces"), []);
    assert_eq!(
        click(&mut h, "Import"),
        [UiAction::ConfirmImport(ImportOptions {
            units: ImportUnits::Millimeters,
            up: Some(UpAxis::Z),
            place: ImportAs::Context,
            merge_coplanar: false,
        })]
    );
}

#[test]
fn the_import_sheet_cancels_on_escape_and_cancel() {
    let mut h = harness(import_fixture());
    h.key_press(egui::Key::Escape);
    h.run_ok();
    assert_eq!(h.state_mut().take(), [UiAction::CloseSheet]);
    let mut h = harness(import_fixture());
    assert_eq!(click(&mut h, "Cancel"), [UiAction::CloseSheet]);
}

#[test]
fn a_sheet_settles_in_one_frame() {
    let mut h = harness(import_fixture());
    assert_eq!(h.try_run().unwrap(), 1);
}

#[test]
fn a_sheet_keeps_presses_off_the_view() {
    let h = harness(import_fixture());
    let at = h.state().central.center();
    let layer = h.ctx.layer_id_at(at).expect("the sheet's backdrop");
    assert_ne!(
        layer.order,
        egui::Order::Background,
        "Gui::wants_pointer_at is true there"
    );
}

/// The text the field named `label` holds.
fn field_text(h: &H, label: &str) -> String {
    h.get_by_label(label)
        .accesskit_node()
        .value()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn make_component_asks_for_a_name() {
    let fixture = || Fixture {
        sheet: Some(Sheet::NameComponent {
            suggested: "Component 3".into(),
        }),
        ..Default::default()
    };
    let mut h = harness(fixture());
    assert_eq!(field_text(&h, "Name"), "Component 3");
    // The field has focus with all of it selected: typing replaces it.
    h.event(egui::Event::Text("Chair".into()));
    h.run_ok();
    assert_eq!(field_text(&h, "Name"), "Chair");
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(
        h.state_mut().take(),
        [UiAction::ConfirmName("Chair".into())]
    );

    let mut h = harness(fixture());
    h.event(egui::Event::Text("   ".into()));
    h.run_ok();
    assert!(h.get_by_label("Create").accesskit_node().is_disabled());
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(h.state_mut().take(), [], "a blank component name");
    assert_eq!(click(&mut h, "Create"), []);
    // Escape with the field focused closes the sheet.
    let mut h = harness(fixture());
    h.key_press(egui::Key::Escape);
    h.run_ok();
    assert_eq!(h.state_mut().take(), [UiAction::CloseSheet]);
}

#[test]
fn the_rename_sheet_names_the_kind() {
    let rename = |kind, name: &str| Fixture {
        sheet: Some(Sheet::Rename {
            target: RenameTarget {
                definition: "d4".parse().unwrap(),
                kind,
                name: name.into(),
            },
        }),
        ..Default::default()
    };
    let h = harness(rename(DefinitionKind::Component, "Chair"));
    assert!(h.query_by_label("Rename Component").is_some());
    assert_eq!(field_text(&h, "Name"), "Chair");
    let mut h = harness(rename(DefinitionKind::Group, "Shed"));
    assert!(h.query_by_label("Rename Group").is_some());
    // A group may go without a name.
    h.key_press(egui::Key::Backspace);
    h.run_ok();
    assert_eq!(field_text(&h, "Name"), "");
    assert!(!h.get_by_label("Rename").accesskit_node().is_disabled());
    assert_eq!(
        click(&mut h, "Rename"),
        [UiAction::ConfirmName(String::new())]
    );
}
