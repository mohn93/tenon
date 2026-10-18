use super::*;
use crate::editor::{ImportAs, ImportOptions, ImportUnits, UpAxis};

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
