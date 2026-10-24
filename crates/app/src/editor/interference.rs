//! Tools > Check Interference: the `interference` query on the selected parts (or all of
//! them), the colliding parts selected and listed in a small window.

use kernel::EntityId;
use session::{Command, LengthUnit, Query, QueryResult, SelectMode};

use super::{Editor, Message, shown_name};

/// One interfering pair as the window lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct InterferenceRow {
    pub a: EntityId,
    pub b: EntityId,
    /// What the row says: both parts' names and the overlap volume in the model's units.
    pub label: String,
}

/// The Check Interference window: what the last check found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InterferenceReport {
    pub rows: Vec<InterferenceRow>,
    /// How many parts were left out for not being solids.
    pub skipped: usize,
}

/// `cubic_meters` in the cube of the model's length unit, with about three significant digits.
fn volume_text(cubic_meters: f64, unit: LengthUnit) -> String {
    let (scale, suffix): (f64, &str) = match unit {
        LengthUnit::Meters => (1.0, "m³"),
        LengthUnit::Centimeters => (100.0, "cm³"),
        LengthUnit::Millimeters => (1000.0, "mm³"),
        LengthUnit::Inches | LengthUnit::FeetInches => (1.0 / 0.0254, "in³"),
        LengthUnit::Feet => (1.0 / (12.0 * 0.0254), "ft³"),
    };
    let v = cubic_meters * scale.powi(3);
    let decimals = if v > 0.0 {
        (2.0 - v.log10().floor()).clamp(0.0, 9.0) as usize
    } else {
        3
    };
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        &s
    };
    format!("{s} {suffix}")
}

impl Editor {
    /// The window's report while it is open.
    pub fn interference_window(&self) -> Option<&InterferenceReport> {
        self.interference.as_ref()
    }

    /// Closes the Check Interference window.
    pub fn close_interference(&mut self) -> bool {
        self.interference.take().is_some()
    }

    fn part_name(&self, id: EntityId) -> String {
        let EntityId::Instance(i) = id else {
            return id.to_string();
        };
        self.doc
            .model()
            .instance(i)
            .and_then(|inst| self.doc.scene().definition(inst.definition))
            .map(|d| shown_name(d.kind, &d.name).to_string())
            .filter(|n| !n.is_empty())
            .map_or_else(|| id.to_string(), |n| format!("{n} ({id})"))
    }

    /// Checks the selected groups and components against each other (all of them when none is
    /// selected), selects the parts that collide and opens the window with the pairs. The
    /// query runs here, on the calling thread; the app has no worker for queries.
    pub fn check_interference(&mut self) -> bool {
        if !self.doc.context().is_empty() {
            self.message = Some(Message::error(
                "Close the open group or component to check interference",
            ));
            return true;
        }
        let selected = self.doc.selection().len();
        let mut ids: Vec<EntityId> = self
            .doc
            .selection()
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Instance(_)))
            .collect();
        if selected > 0 && ids.is_empty() {
            self.message = Some(Message::error(
                "Select parts (groups or components) to check",
            ));
            return true;
        }
        // One selected part is checked against all the others.
        let only = (ids.len() == 1).then(|| ids[0]);
        if only.is_some() {
            ids.clear();
        }
        let result = self.doc.query(&Query::Interference {
            ids,
            touching: false,
        });
        let (mut pairs, skipped) = match result {
            Ok(QueryResult::Interference { pairs, skipped, .. }) => (pairs, skipped.len()),
            Ok(_) => return true,
            Err(e) => {
                self.message = Some(Message::error(e.message));
                return true;
            }
        };
        if let Some(id) = only {
            pairs.retain(|p| p.a == id || p.b == id);
        }
        let unit = self.doc.units().length;
        let rows: Vec<InterferenceRow> = pairs
            .iter()
            .map(|p| InterferenceRow {
                a: p.a,
                b: p.b,
                label: format!(
                    "{} and {}: {}",
                    self.part_name(p.a),
                    self.part_name(p.b),
                    volume_text(p.volume, unit)
                ),
            })
            .collect();
        let mut colliding: Vec<EntityId> = Vec::new();
        for r in &rows {
            for id in [r.a, r.b] {
                if !colliding.contains(&id) {
                    colliding.push(id);
                }
            }
        }
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        if !colliding.is_empty() {
            self.run(Command::Select {
                ids: colliding,
                mode: SelectMode::Replace,
                all: false,
            });
        }
        let text = match rows.len() {
            0 => "No interference".to_string(),
            1 => "Interference: 1 pair".to_string(),
            n => format!("Interference: {n} pairs"),
        };
        self.message = Some(Message::info(text));
        self.interference = Some(InterferenceReport { rows, skipped });
        self.invalidate_hover();
        true
    }

    /// Selects both parts of row `index` of the window's list and zooms to them.
    pub fn select_interference_pair(&mut self, index: usize) -> bool {
        let Some(row) = self
            .interference
            .as_ref()
            .and_then(|r| r.rows.get(index))
            .cloned()
        else {
            return false;
        };
        self.message = None;
        self.run(Command::Select {
            ids: vec![row.a, row.b],
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
    use session::{Command, SelectMode};

    use crate::editor::Editor;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// A 2 m box group and a copy of it `dx` meters along X; returns (first, copy).
    fn two_boxes(e: &mut Editor, dx: f64) -> (EntityId, EntityId) {
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        e.execute(Command::PushPull {
            face: "f8".parse().unwrap(),
            distance: 2.0,
        })
        .unwrap();
        let m = e.doc().model();
        let ids = m
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
            .collect();
        e.execute(Command::MakeGroup { ids, name: None }).unwrap();
        let first = *e.doc().selection().first().unwrap();
        let out = e
            .execute(Command::Copy {
                ids: vec![first],
                offset: [dx, 0.0, 0.0],
                count: None,
            })
            .unwrap();
        (first, out.placed.unwrap()[0])
    }

    fn select(e: &mut Editor, ids: Vec<EntityId>) {
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
    }

    #[test]
    fn check_selects_the_colliding_parts_and_lists_the_pair() {
        let mut e = editor();
        let (a, b) = two_boxes(&mut e, 1.0);
        select(&mut e, vec![]);
        assert!(e.check_interference());
        let report = e.interference_window().unwrap();
        assert_eq!(report.rows.len(), 1);
        assert!(
            report.rows[0].label.ends_with(": 4 m³"),
            "{}",
            report.rows[0].label
        );
        let sel: Vec<_> = e.doc().selection().iter().copied().collect();
        assert!(sel.contains(&a) && sel.contains(&b) && sel.len() == 2);
    }

    #[test]
    fn one_selected_part_is_checked_against_all_the_others() {
        let mut e = editor();
        let (a, b) = two_boxes(&mut e, 1.0);
        select(&mut e, vec![a]);
        assert!(e.check_interference());
        let report = e.interference_window().unwrap();
        assert_eq!(report.rows.len(), 1);
        let sel: Vec<_> = e.doc().selection().iter().copied().collect();
        assert!(sel.contains(&a) && sel.contains(&b));
    }

    #[test]
    fn a_selection_without_parts_asks_for_parts() {
        let mut e = editor();
        two_boxes(&mut e, 1.0);
        e.execute(Command::AddRectangle {
            corner: [10.0, 0.0, 0.0],
            opposite: [11.0, 1.0, 0.0],
            normal: None,
        })
        .unwrap();
        let loose = e
            .doc()
            .model()
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .next()
            .unwrap();
        select(&mut e, vec![loose]);
        assert!(e.check_interference());
        assert!(e.interference_window().is_none());
        assert!(e.status().contains("Select parts"), "{}", e.status());
    }

    #[test]
    fn check_is_refused_inside_an_open_group() {
        let mut e = editor();
        let (a, _) = two_boxes(&mut e, 1.0);
        e.execute(Command::OpenInstance { id: a }).unwrap();
        assert!(e.check_interference());
        assert!(e.interference_window().is_none());
        assert!(e.status().contains("Close the open"), "{}", e.status());
    }

    #[test]
    fn a_new_document_drops_the_window() {
        let mut e = editor();
        two_boxes(&mut e, 1.0);
        e.check_interference();
        assert!(e.interference_window().is_some());
        e.document_replaced();
        assert!(e.interference_window().is_none());
    }

    #[test]
    fn check_with_nothing_colliding_says_so() {
        let mut e = editor();
        two_boxes(&mut e, 5.0);
        assert!(e.check_interference());
        assert!(e.interference_window().unwrap().rows.is_empty());
        assert!(e.status().contains("No interference"), "{}", e.status());
    }

    #[test]
    fn clicking_a_pair_selects_both_and_zooms_to_them() {
        let mut e = editor();
        let (a, b) = two_boxes(&mut e, 1.0);
        select(&mut e, vec![]);
        e.check_interference();
        select(&mut e, vec![a]);
        let before = e.doc().camera.clone();
        assert!(e.select_interference_pair(0));
        let sel: Vec<_> = e.doc().selection().iter().copied().collect();
        assert!(sel.contains(&a) && sel.contains(&b) && sel.len() == 2);
        assert_ne!(e.doc().camera, before);
        assert!(!e.select_interference_pair(9));
        assert!(e.close_interference());
        assert!(e.interference_window().is_none());
    }

    #[test]
    fn volumes_are_written_in_the_models_units() {
        use super::volume_text;
        use session::LengthUnit;
        assert_eq!(volume_text(4.0, LengthUnit::Meters), "4 m³");
        assert_eq!(volume_text(0.000_125, LengthUnit::Meters), "0.000125 m³");
        assert_eq!(volume_text(0.001, LengthUnit::Millimeters), "1000000 mm³");
    }
}
