use super::*;
use crate::editor::{CutListReport, CutListRow};

fn row(name: &str, solid: bool) -> CutListRow {
    let length = if solid { "1.2 m" } else { "0.5 m" };
    CutListRow {
        cells: [name, "Oak", length, "0.1 m", "0.02 m", "3"].map(String::from),
        solid,
        note: (!solid).then(|| "it has an open edge".to_string()),
        ids: vec!["i1".parse().unwrap()],
    }
}

fn fixture(rows: Vec<CutListRow>) -> Fixture {
    Fixture {
        cut_list: Some(CutListReport {
            headers: [
                "Name",
                "Material",
                "Length (m)",
                "Width (m)",
                "Thickness (m)",
                "Quantity",
            ]
            .map(String::from),
            parts: rows.len(),
            rows,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn the_window_shows_the_table_and_a_click_selects_the_row() {
    let mut h = harness(fixture(vec![row("Board", true), row("Sheet", false)]));
    h.get_by_label("Length (m)");
    h.get_by_label("1.2 m");
    h.get_by_label("Sheet (not solid)");
    h.get_by_label("Board").click();
    h.run_ok();
    assert_eq!(h.state_mut().take(), [UiAction::CutListRow(0)]);
}

#[test]
fn export_csv_asks_for_the_file_and_an_empty_list_says_so() {
    let mut h = harness(fixture(vec![row("Board", true)]));
    h.get_by_label("Export CSV...").click();
    h.run_ok();
    assert_eq!(h.state_mut().take(), [UiAction::ExportCutList]);
    let h = harness(fixture(Vec::new()));
    h.get_by_label("No parts");
}

#[test]
fn tools_cut_list_opens_the_window() {
    let mut h = harness(Fixture::default());
    assert_eq!(
        click_menu(&mut h, &["Tools", "Cut List"]),
        [UiAction::ShowCutList]
    );
}
