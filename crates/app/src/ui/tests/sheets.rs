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
fn the_import_sheet_for_a_sketchup_file_hides_units_up_and_merging() {
    let shown = |path: &str| {
        let h = harness(Fixture {
            sheet: Some(Sheet::Import {
                path: path.into(),
                options: ImportOptions::default(),
            }),
            ..Default::default()
        });
        let texts = drawn_texts(&h);
        ["Import as", "Units", "Up axis", "Merge coplanar faces"]
            .map(|label| texts.iter().any(|t| t == label))
    };
    assert_eq!(shown("/m/desk.skp"), [true, false, false, false]);
    assert_eq!(shown("/m/DESK.SKP"), [true, false, false, false]);
    assert_eq!(shown("/m/chair.obj"), [true, true, true, true]);
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

#[test]
fn the_field_of_view_sheet_applies_whole_degrees() {
    let mut h = harness(Fixture {
        sheet: Some(Sheet::FieldOfView { degrees: 35.4 }),
        ..Default::default()
    });
    assert!(h.query_by_label("Field of View").is_some(), "the title");
    // The number and the suffix are painted apart; the field's value reads them together.
    assert_eq!(field_text(&h, "Field of view"), "35°");
    assert_eq!(
        type_into(&mut h, "Field of view", 0, "50", egui::Key::Enter),
        []
    );
    assert_eq!(click(&mut h, "Apply"), [UiAction::ConfirmFieldOfView(50.0)]);
}

/// Review Focus 5: with a text field focused, Copy and Paste are the field's: text goes to and
/// from it and the panels ask for no geometry to be copied or pasted.
#[test]
fn a_focused_field_copies_and_pastes_text() {
    let mut h = harness(Fixture {
        sheet: Some(Sheet::NameComponent {
            suggested: "Component 3".into(),
        }),
        ..Default::default()
    });
    // The field has focus with all of its text selected.
    h.event(egui::Event::Copy);
    // One frame: its output carries the copied text.
    h.step();
    let copied = h
        .output()
        .platform_output
        .commands
        .iter()
        .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "Component 3"));
    assert!(copied, "{:?}", h.output().platform_output.commands);
    h.event(egui::Event::Paste("Shed".into()));
    h.run_ok();
    assert_eq!(field_text(&h, "Name"), "Shed");
    assert_eq!(h.state_mut().take(), []);
}

/// The note sheet for `request`, starting with `text`, blank allowed with `automatic`.
fn note_fixture(request: tools::NoteRequest, text: &str, automatic: Option<&str>) -> Fixture {
    Fixture {
        sheet: Some(Sheet::NoteText {
            request,
            text: text.into(),
            automatic: automatic.map(Into::into),
        }),
        ..Default::default()
    }
}

fn leader() -> tools::NoteRequest {
    tools::NoteRequest::Leader {
        anchor: session::AnchorInput::Point([2.0, 0.0, 0.0]),
        point: glam::DVec3::new(2.0, 0.0, 0.0),
        position: glam::DVec3::new(3.0, -1.0, 0.0),
    }
}

#[test]
fn the_note_sheet_takes_text_or_none_for_a_leader_note() {
    let automatic = "(2 m, 0 m, 0 m)";
    let mut h = harness(note_fixture(leader(), "", Some(automatic)));
    assert!(h.query_by_label("Add Text").is_some(), "the title");
    assert_eq!(field_text(&h, "Note text"), "");
    let texts = drawn_texts(&h);
    assert!(texts.iter().any(|t| t == automatic), "the hint: {texts:?}");
    assert!(
        texts
            .iter()
            .any(|t| t == "Leave it empty to show the point's coordinates")
    );
    assert_eq!(click(&mut h, "Add"), [UiAction::ConfirmNote(String::new())]);
    // The field has focus: typed text, over two lines, is the note's.
    let mut h = harness(note_fixture(leader(), "", Some(automatic)));
    h.event(egui::Event::Text("North".into()));
    h.key_press(egui::Key::Enter);
    h.event(egui::Event::Text("wall".into()));
    h.run_ok();
    assert_eq!(field_text(&h, "Note text"), "North\nwall");
    assert_eq!(h.state_mut().take(), [], "Enter makes a new line");
    assert_eq!(
        click(&mut h, "Add"),
        [UiAction::ConfirmNote("North\nwall".into())]
    );
    let mut h = harness(note_fixture(leader(), "", Some(automatic)));
    assert_eq!(click(&mut h, "Cancel"), [UiAction::CloseSheet]);
}

#[test]
fn a_note_fixed_to_the_view_needs_text_within_the_limits() {
    let screen = tools::NoteRequest::Screen { at: [0.1, 0.05] };
    let mut h = harness(note_fixture(screen, "", None));
    assert!(h.get_by_label("Add").accesskit_node().is_disabled());
    assert!(
        drawn_texts(&h)
            .iter()
            .any(|t| t == "Type the text the note shows")
    );
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
    h.event(egui::Event::Text("Plan".into()));
    h.run_ok();
    assert_eq!(
        click(&mut h, "Add"),
        [UiAction::ConfirmNote("\nPlan".into())],
        "Enter made a new line"
    );
    // Changing a note: its text in the field; too many lines are not taken.
    let edit = tools::NoteRequest::Edit("n4".parse().unwrap());
    let h = harness(note_fixture(edit, "Old", None));
    assert!(h.query_by_label("Edit Text").is_some());
    assert_eq!(field_text(&h, "Note text"), "Old");
    assert!(!h.get_by_label("Change").accesskit_node().is_disabled());
    let long = vec!["line"; 65].join("\n");
    let h = harness(note_fixture(edit, &long, Some("(0 m, 0 m, 0 m)")));
    assert!(h.get_by_label("Change").accesskit_node().is_disabled());
    assert!(drawn_texts(&h).iter().any(|t| t == "At most 64 lines"));
}
