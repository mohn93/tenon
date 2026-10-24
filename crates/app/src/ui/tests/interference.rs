use super::*;
use crate::editor::{InterferenceReport, InterferenceRow};

fn row(label: &str) -> InterferenceRow {
    InterferenceRow {
        a: "i1".parse().unwrap(),
        b: "i2".parse().unwrap(),
        label: label.into(),
    }
}

fn fixture(rows: Vec<InterferenceRow>) -> Fixture {
    Fixture {
        interference: Some(InterferenceReport { rows, skipped: 0 }),
        ..Default::default()
    }
}

#[test]
fn clicking_a_pair_asks_to_select_and_zoom_to_it() {
    let mut h = harness(fixture(vec![
        row("Pin and Bore: 4 m³"),
        row("A and B: 1 m³"),
    ]));
    h.get_by_label("A and B: 1 m³").click();
    h.run_ok();
    assert_eq!(h.state_mut().take(), [UiAction::InterferencePair(1)]);
}

#[test]
fn an_empty_report_says_no_interference() {
    let h = harness(fixture(Vec::new()));
    h.get_by_label("No interference");
}
