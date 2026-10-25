use super::*;
use crate::editor::{
    ImportAs, ImportOptions, ImportUnits, PaperChoice, PrintOptions, PrintPurpose, RenameTarget,
    UpAxis,
};
use crate::ui::{PhoneShare, PhoneView};
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

/// The 3D Text sheet starting with `text`, 0.3 m tall and 0.05 m deep, filled.
fn text_3d_fixture(text: &str) -> Fixture {
    Fixture {
        sheet: Some(Sheet::Text3d {
            spec: tools::TextSpec {
                text: text.into(),
                height: 0.3,
                depth: 0.05,
                filled: true,
            },
        }),
        ..Default::default()
    }
}

#[test]
fn the_3d_text_sheet_shows_and_reads_sizes_in_the_model_unit() {
    use session::{LengthUnit, Units};
    let mut fixture = text_3d_fixture("Plan");
    fixture.units = Units {
        length: LengthUnit::Centimeters,
        ..Units::default()
    };
    let mut h = harness(fixture);
    assert!(drawn_texts(&h).iter().any(|t| t == "30 cm"));
    assert!(drawn_texts(&h).iter().any(|t| t == "5 cm"));
    // A bare number is centimeters; the sheet still reports meters.
    h.get_by_label("Height").click();
    h.run_ok();
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.run_ok();
    h.event(egui::Event::Text("50".into()));
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    assert_eq!(
        click(&mut h, "Place"),
        [UiAction::Confirm3dText(tools::TextSpec {
            text: "Plan".into(),
            height: 0.5,
            depth: 0.05,
            filled: true,
        })]
    );
}

#[test]
fn the_3d_text_sheet_reports_the_text_and_its_sizes() {
    let mut h = harness(text_3d_fixture(""));
    assert!(h.query_by_label("3D Text").is_some(), "the title");
    assert!(h.get_by_label("Place").accesskit_node().is_disabled());
    assert!(drawn_texts(&h).iter().any(|t| t == "Type the text to draw"));
    // The field has focus: typed text, over two lines, is the text.
    h.event(egui::Event::Text("Hello".into()));
    h.key_press(egui::Key::Enter);
    h.event(egui::Event::Text("o8%".into()));
    h.run_ok();
    assert_eq!(field_text(&h, "Text to draw"), "Hello\no8%");
    assert_eq!(h.state_mut().take(), [], "Enter makes a new line");
    assert_eq!(
        click(&mut h, "Place"),
        [UiAction::Confirm3dText(tools::TextSpec {
            text: "Hello\no8%".into(),
            height: 0.3,
            depth: 0.05,
            filled: true,
        })]
    );
    // Not filled: the depth is disabled and placed as 0.
    let mut h = harness(text_3d_fixture("Plan"));
    assert!(!h.get_by_label("Depth").accesskit_node().is_disabled());
    assert_eq!(click(&mut h, "Filled"), []);
    assert!(h.get_by_label("Depth").accesskit_node().is_disabled());
    assert_eq!(
        click(&mut h, "Place"),
        [UiAction::Confirm3dText(tools::TextSpec {
            text: "Plan".into(),
            height: 0.3,
            depth: 0.0,
            filled: false,
        })]
    );
    let long = vec!["line"; 65].join("\n");
    let h = harness(text_3d_fixture(&long));
    assert!(h.get_by_label("Place").accesskit_node().is_disabled());
    assert!(drawn_texts(&h).iter().any(|t| t == "At most 64 lines"));
    let mut h = harness(text_3d_fixture("Plan"));
    assert_eq!(click(&mut h, "Cancel"), [UiAction::CloseSheet]);
}

fn print_fixture(purpose: PrintPurpose) -> Fixture {
    Fixture {
        sheet: Some(Sheet::Print {
            purpose,
            spec: PrintOptions::default(),
        }),
        ..Default::default()
    }
}

#[test]
fn the_print_sheet_reports_its_options() {
    let mut h = harness(print_fixture(PrintPurpose::Export));
    assert!(h.query_by_label("Export PDF").is_some(), "the title");
    assert_eq!(click(&mut h, "Paper"), []);
    assert_eq!(click(&mut h, "Letter"), []);
    assert_eq!(click(&mut h, "Portrait"), []);
    assert_eq!(click(&mut h, "1 :"), []);
    assert_eq!(click(&mut h, "Add cut list pages"), []);
    assert_eq!(
        click(&mut h, "Export..."),
        [UiAction::ConfirmPrint(PrintOptions {
            paper: PaperChoice::Letter,
            landscape: false,
            scale: Some(PrintOptions::DEFAULT_SCALE),
            dpi: 150,
            cut_list: true,
        })]
    );
}

#[test]
fn the_print_sheet_starts_from_the_remembered_values_and_fits_by_default() {
    let mut h = harness(print_fixture(PrintPurpose::Print));
    assert!(h.query_by_label("Print Setup").is_some(), "the title");
    assert_eq!(
        click(&mut h, "Print"),
        [UiAction::ConfirmPrint(PrintOptions::default())]
    );
    let remembered = PrintOptions {
        paper: PaperChoice::A3,
        landscape: false,
        scale: Some(50.0),
        dpi: 200,
        cut_list: false,
    };
    let mut h = harness(Fixture {
        sheet: Some(Sheet::Print {
            purpose: PrintPurpose::Print,
            spec: remembered,
        }),
        ..Default::default()
    });
    assert_eq!(click(&mut h, "Fit to page"), []);
    assert_eq!(
        click(&mut h, "Print"),
        [UiAction::ConfirmPrint(PrintOptions {
            scale: None,
            ..remembered
        })]
    );
}

#[test]
fn the_print_sheet_cancels_and_clamps_a_resolution_out_of_range() {
    let mut h = harness(print_fixture(PrintPurpose::Print));
    assert_eq!(click(&mut h, "Cancel"), [UiAction::CloseSheet]);
    let mut h = harness(Fixture {
        sheet: Some(Sheet::Print {
            purpose: PrintPurpose::Print,
            spec: PrintOptions {
                dpi: 9000,
                ..PrintOptions::default()
            },
        }),
        ..Default::default()
    });
    assert_eq!(
        click(&mut h, "Print"),
        [UiAction::ConfirmPrint(PrintOptions {
            dpi: 300,
            ..PrintOptions::default()
        })]
    );
}

fn phone_share() -> PhoneShare {
    PhoneShare {
        url: "http://192.168.1.20:50123/#v1.abc.def".into(),
        qr_width: 21,
        qr: vec![true; 21 * 21],
        sent: 0,
        seconds_left: 1781,
        stale: false,
    }
}

fn phone_fixture(share: PhoneShare) -> Fixture {
    Fixture {
        sheet: Some(Sheet::Phone {
            view: PhoneView::Sharing(share),
        }),
        ..Default::default()
    }
}

#[test]
fn the_phone_sheet_shows_the_link_status_and_time_left() {
    let h = harness(phone_fixture(phone_share()));
    let texts = drawn_texts(&h);
    for want in [
        "OPEN ON PHONE",
        "http://192.168.1.20:50123/#v1.abc.def",
        "Waiting for the phone",
        "Stops in 29:41",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    assert!(h.query_by_label("Share the new version").is_none());
}

#[test]
fn the_phone_sheet_counts_phones() {
    for (sent, want) in [(1, "Sent to 1 phone"), (2, "Sent to 2 phones")] {
        let h = harness(phone_fixture(PhoneShare {
            sent,
            ..phone_share()
        }));
        assert!(drawn_texts(&h).iter().any(|t| t == want), "{want}");
    }
}

#[test]
fn the_phone_sheet_stops_and_closes() {
    let mut h = harness(phone_fixture(phone_share()));
    assert_eq!(click(&mut h, "Stop"), [UiAction::StopSharing]);
    let mut h = harness(phone_fixture(phone_share()));
    h.key_press(egui::Key::Escape);
    h.run_ok();
    assert_eq!(h.state_mut().take(), [UiAction::CloseSheet]);
}

#[test]
fn the_phone_sheet_offers_the_new_version_when_stale() {
    let mut h = harness(phone_fixture(PhoneShare {
        stale: true,
        ..phone_share()
    }));
    assert_eq!(
        click(&mut h, "Share the new version"),
        [UiAction::ShareNewVersion]
    );
}

#[test]
fn the_phone_sheet_has_a_copy_button() {
    let mut h = harness(phone_fixture(phone_share()));
    assert_eq!(click(&mut h, "Copy"), []);
}

#[test]
fn the_phone_sheet_says_why_there_is_no_share() {
    let mut h = harness(Fixture {
        sheet: Some(Sheet::Phone {
            view: PhoneView::Unavailable("Connect this computer to Wi-Fi to share".into()),
        }),
        ..Default::default()
    });
    assert!(
        drawn_texts(&h)
            .iter()
            .any(|t| t == "Connect this computer to Wi-Fi to share")
    );
    assert!(h.query_by_label("Stop").is_none());
    assert_eq!(click(&mut h, "Close"), [UiAction::CloseSheet]);
}

#[test]
fn the_qr_is_drawn_with_a_quiet_zone_and_big_enough_modules() {
    use crate::ui::sheets::{qr_module_size, qr_rects};
    for ppp in [1.0f32, 1.5, 2.0, 3.0] {
        for width in [21usize, 33, 41, 45, 57] {
            let px = qr_module_size(width, ppp) * ppp;
            assert!(px >= 4.0 - 1e-3, "{width} at {ppp}: {px}");
            assert!(
                (px - px.round()).abs() < 1e-3,
                "whole pixels: {width} at {ppp}: {px}"
            );
        }
    }
    // One dark module at the top-left corner sits 4 modules in from the edge.
    let mut modules = vec![false; 25];
    modules[0] = true;
    modules[24] = true;
    let origin = egui::pos2(10.0, 20.0);
    let rects = qr_rects(origin, 5, &modules, 4.0);
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].min, egui::pos2(10.0 + 16.0, 20.0 + 16.0));
    assert_eq!(rects[0].size(), egui::vec2(4.0, 4.0));
    assert_eq!(rects[1].min, egui::pos2(10.0 + 32.0, 20.0 + 32.0));
    // The code with its quiet zone is (5 + 8) modules wide.
    assert!(rects.iter().all(|r| r.max.x <= 10.0 + 13.0 * 4.0 - 16.0));
}

#[test]
fn the_clock_reads_minutes_and_seconds() {
    use crate::ui::sheets::clock;
    assert_eq!(clock(1800), "30:00");
    assert_eq!(clock(61), "1:01");
    assert_eq!(clock(0), "0:00");
}
