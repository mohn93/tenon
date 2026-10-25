use super::*;
use crate::editor::{HealthReport, HealthRow};

fn row(label: &str, fixable: bool) -> HealthRow {
    HealthRow {
        id: "i1".parse().unwrap(),
        label: label.into(),
        fixable,
    }
}

fn fixture(rows: Vec<HealthRow>) -> Fixture {
    Fixture {
        health: Some(HealthReport {
            rows,
            checked: 3,
            asked: Vec::new(),
        }),
        ..Default::default()
    }
}

#[test]
fn clicking_a_part_asks_to_select_and_zoom_to_it() {
    let mut h = harness(fixture(vec![
        row("Box (i1): 4 open edges", true),
        row("Lid (i2): 1 stray edge", true),
    ]));
    h.get_by_label("Lid (i2): 1 stray edge").click();
    h.run_ok();
    assert_eq!(h.state_mut().take(), [UiAction::HealthPart(1)]);
}

#[test]
fn make_solid_runs_when_a_part_can_be_fixed() {
    let mut h = harness(fixture(vec![row("Box (i1): 4 open edges", true)]));
    h.get_by_label("Make Solid").click();
    h.run_ok();
    assert_eq!(h.state_mut().take(), [UiAction::MakeSolid]);
}

#[test]
fn make_solid_is_disabled_when_nothing_can_be_fixed() {
    let mut h = harness(fixture(vec![row(
        "Pair (i1): 1 edge shared by more than two faces",
        false,
    )]));
    h.get_by_label("Make Solid").click();
    h.run_ok();
    assert_eq!(h.state_mut().take(), []);
}

#[test]
fn a_report_without_rows_says_every_part_is_a_solid() {
    let h = harness(fixture(Vec::new()));
    h.get_by_label("All 3 parts are solids");
}
