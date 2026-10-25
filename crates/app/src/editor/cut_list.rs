//! Tools > Cut List: the `cut_list` query on the open context shown in a window, in the
//! model's display units, with the rows selecting their parts and the list exportable as CSV.

use kernel::{EntityId, InstanceId};
use session::cutlist::{header, table_cells};
use session::{Command, CutListGrouping, Query, QueryResult, SelectMode};

use super::{Editor, Message};

/// One row of the window.
#[derive(Clone, Debug, PartialEq)]
pub struct CutListRow {
    /// Name, material, length, width, thickness and quantity as the table shows them.
    pub cells: [String; 6],
    /// Whether the part is a solid; a part that is not is marked, with `note` as its tooltip.
    pub solid: bool,
    pub note: Option<String>,
    /// What clicking the row selects.
    pub ids: Vec<EntityId>,
}

/// The Cut List window: the rows as of the revision it was last read at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CutListReport {
    /// The column titles, the length unit named on the sizes.
    pub headers: [String; 6],
    pub rows: Vec<CutListRow>,
    /// Parts in all (the sum of the quantities).
    pub parts: usize,
    /// The document revision and the open context the rows were read at.
    pub revision: u64,
    pub context: Vec<InstanceId>,
}

impl Editor {
    /// The window's report while it is open.
    pub fn cut_list_window(&self) -> Option<&CutListReport> {
        self.cut_list.as_ref()
    }

    /// Closes the Cut List window.
    pub fn close_cut_list(&mut self) -> bool {
        self.cut_list.take().is_some()
    }

    /// Tools > Cut List: opens the window on the open context's parts.
    pub fn open_cut_list(&mut self) -> bool {
        self.message = None;
        match self.read_cut_list() {
            Some(report) => {
                self.cut_list = Some(report);
                true
            }
            None => true,
        }
    }

    /// Reads the list again when the model changed since the window read it.
    pub fn refresh_cut_list(&mut self) {
        let stale = self.cut_list.as_ref().is_some_and(|r| {
            r.revision != self.doc.revision() || r.context != self.doc.context().to_vec()
        });
        if stale && let Some(report) = self.read_cut_list() {
            self.cut_list = Some(report);
        }
    }

    fn read_cut_list(&mut self) -> Option<CutListReport> {
        let result = self.doc.query(&Query::CutList {
            ids: Vec::new(),
            group_by: CutListGrouping::Definition,
        });
        let rows = match result {
            Ok(QueryResult::CutList { rows }) => rows,
            Ok(_) => return None,
            Err(e) => {
                self.message = Some(Message::error(e.message));
                return None;
            }
        };
        let units = self.doc.units();
        let heads = header(&units);
        Some(CutListReport {
            headers: std::array::from_fn(|i| heads[i].clone()),
            parts: rows.iter().map(|r| r.quantity).sum(),
            rows: rows
                .iter()
                .map(|r| CutListRow {
                    cells: table_cells(r, &units),
                    solid: r.solid,
                    note: r.note.clone(),
                    ids: r.ids.clone(),
                })
                .collect(),
            revision: self.doc.revision(),
            context: self.doc.context().to_vec(),
        })
    }

    /// Selects the parts of row `index` of the window and zooms to them.
    pub fn select_cut_list_row(&mut self, index: usize) -> bool {
        let Some(ids) = self
            .cut_list
            .as_ref()
            .and_then(|r| r.rows.get(index))
            .map(|r| r.ids.clone())
        else {
            return false;
        };
        self.message = None;
        self.run(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        });
        self.frame_selection();
        self.invalidate_hover();
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use kernel::EntityId;
    use session::{Command, LengthUnit};

    use crate::editor::Editor;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// The first board is a new component; the next ones are copies of it.
    fn add_board(e: &mut Editor, x: f64) {
        if x == 0.0 {
            e.execute(Command::AddBox {
                corner: [0.0, 0.0, 0.0],
                size: [1.0, 0.1, 0.02],
                placement: session::PrimitiveAs::Component,
                name: Some("Board".into()),
                material: None,
            })
            .unwrap();
        } else {
            let first = e.doc().model().instances().next().unwrap().0;
            e.execute(Command::Copy {
                ids: vec![EntityId::Instance(first)],
                offset: [x, 0.0, 0.0],
                count: None,
            })
            .unwrap();
        }
    }

    #[test]
    fn the_window_lists_the_parts_in_the_display_units_and_selects_a_row() {
        let mut e = editor();
        add_board(&mut e, 0.0);
        add_board(&mut e, 2.0);
        e.execute(Command::SetUnits {
            length: Some(LengthUnit::Millimeters),
            precision: Some(0),
            length_snap: None,
            angle_precision: None,
            angle_snap: None,
        })
        .unwrap();
        assert!(e.cut_list_window().is_none());
        e.open_cut_list();
        let report = e.cut_list_window().expect("open");
        assert_eq!(report.headers[2], "Length (mm)");
        assert_eq!(report.parts, 2);
        assert_eq!(report.rows.len(), 1);
        assert_eq!(
            report.rows[0].cells,
            ["Board", "", "1000 mm", "100 mm", "20 mm", "2"].map(String::from)
        );
        assert!(e.select_cut_list_row(0));
        assert_eq!(e.doc().selection().len(), 2);
        assert!(!e.select_cut_list_row(5));
        assert!(e.close_cut_list());
        assert!(e.cut_list_window().is_none());
    }

    #[test]
    fn the_window_follows_the_model_and_the_units() {
        let mut e = editor();
        add_board(&mut e, 0.0);
        e.open_cut_list();
        assert_eq!(e.cut_list_window().unwrap().parts, 1);
        add_board(&mut e, 2.0);
        e.refresh_cut_list();
        assert_eq!(e.cut_list_window().unwrap().parts, 2);
        e.execute(Command::SetUnits {
            length: Some(LengthUnit::Inches),
            precision: Some(1),
            length_snap: None,
            angle_precision: None,
            angle_snap: None,
        })
        .unwrap();
        e.refresh_cut_list();
        let report = e.cut_list_window().unwrap();
        assert_eq!(report.headers[2], "Length (in)");
        assert_eq!(report.rows[0].cells[2], "39.4\"");
    }
}
