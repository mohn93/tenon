//! Tools > Model Health: the `model_health` query on the selected parts (or all of them),
//! listed in a small window; a row selects the part and zooms to it, and Make Solid closes
//! the parts that can be closed.

use kernel::EntityId;
use session::{Command, Query, QueryResult, SelectMode};

use super::{Editor, Message, shown_name};

/// One part that is not a solid, as the window lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct HealthRow {
    pub id: EntityId,
    /// The part's name and its problems in words.
    pub label: String,
    /// Whether Make Solid would close it.
    pub fixable: bool,
}

/// The Model Health window: what the last check found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HealthReport {
    pub rows: Vec<HealthRow>,
    /// How many parts were checked.
    pub checked: usize,
    /// The parts the check was asked for (empty: every part).
    pub asked: Vec<EntityId>,
}

impl HealthReport {
    /// Whether Make Solid has anything to close.
    pub fn fixable(&self) -> bool {
        self.rows.iter().any(|r| r.fixable)
    }
}

impl Editor {
    /// The window's report while it is open.
    pub fn health_window(&self) -> Option<&HealthReport> {
        self.health.as_ref()
    }

    /// Closes the Model Health window.
    pub fn close_health(&mut self) -> bool {
        self.health.take().is_some()
    }

    fn health_name(&self, id: EntityId) -> String {
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

    /// Checks the selected groups and components (all of the open context's when none is
    /// selected) and opens the window with the parts that are not solids.
    pub fn check_health(&mut self) -> bool {
        let selected = self.doc.selection().len();
        let ids: Vec<EntityId> = self
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
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let Some(report) = self.health_report(ids) else {
            return true;
        };
        let text = match (report.rows.len(), report.checked) {
            (_, 0) => "No parts to check".to_string(),
            (0, 1) => "The part is a solid".to_string(),
            (0, n) => format!("All {n} parts are solids"),
            (1, _) => "1 part is not a solid".to_string(),
            (n, _) => format!("{n} parts are not solids"),
        };
        self.message = Some(Message::info(text));
        self.health = Some(report);
        self.invalidate_hover();
        true
    }

    /// The `model_health` answer for `ids` as the window shows it; None (with the error in
    /// the status line) when the query failed.
    fn health_report(&mut self, ids: Vec<EntityId>) -> Option<HealthReport> {
        let result = self.doc.query(&Query::ModelHealth { ids: ids.clone() });
        let (parts, checked) = match result {
            Ok(QueryResult::ModelHealth { parts, checked, .. }) => (parts, checked),
            Ok(_) => return None,
            Err(e) => {
                self.message = Some(Message::error(e.message));
                return None;
            }
        };
        let rows = parts
            .iter()
            .map(|p| {
                let mut said: Vec<&str> = Vec::new();
                for problem in &p.problems {
                    if !said.contains(&problem.text.as_str()) {
                        said.push(&problem.text);
                    }
                }
                HealthRow {
                    id: p.id,
                    label: format!("{}: {}", self.health_name(p.id), said.join(", ")),
                    fixable: p.fixable,
                }
            })
            .collect();
        Some(HealthReport {
            rows,
            checked,
            asked: ids,
        })
    }

    /// Selects the part of row `index` of the window's list and zooms to it.
    pub fn select_health_part(&mut self, index: usize) -> bool {
        let Some(row) = self
            .health
            .as_ref()
            .and_then(|r| r.rows.get(index))
            .cloned()
        else {
            return false;
        };
        self.message = None;
        self.run(Command::Select {
            ids: vec![row.id],
            mode: SelectMode::Replace,
            all: false,
        });
        self.frame_selection();
        self.invalidate_hover();
        true
    }

    /// Runs `make_solid` on the window's fixable parts (one undo step), says what it did and
    /// checks the same parts again.
    pub fn make_solid_from_window(&mut self) -> bool {
        let Some(report) = self.health.clone() else {
            return false;
        };
        let ids: Vec<EntityId> = report
            .rows
            .iter()
            .filter(|r| r.fixable)
            .map(|r| r.id)
            .collect();
        if ids.is_empty() {
            return false;
        }
        self.reset_tool();
        self.message = None;
        let Some(outcome) = self.run(Command::MakeSolid { ids }) else {
            return true;
        };
        let entries = outcome.make_solid.map(|r| r.entries).unwrap_or_default();
        let fixed = entries.iter().filter(|e| e.fixed).count();
        let left = entries.len() - fixed;
        let parts = |n: usize| {
            if n == 1 {
                "1 part".to_string()
            } else {
                format!("{n} parts")
            }
        };
        let text = match (fixed, left) {
            (0, 0) => "Nothing to fix".to_string(),
            (f, 0) => format!("Make Solid fixed {}", parts(f)),
            (0, l) => format!("Make Solid could not fix {}", parts(l)),
            (f, l) => format!("Make Solid fixed {}, could not fix {l}", parts(f)),
        };
        if let Some(fresh) = self.health_report(report.asked) {
            self.health = Some(fresh);
        }
        self.message = Some(Message::info(text));
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

    fn add_box(e: &mut Editor, x: f64) -> EntityId {
        let out = e
            .execute(
                serde_json::from_value(serde_json::json!({
                    "type": "add_box", "corner": [x, 0.0, 0.0], "size": [1.0, 1.0, 1.0],
                    "as": "group"
                }))
                .unwrap(),
            )
            .unwrap();
        out.placed.unwrap()[0]
    }

    /// Erases the top face of part `id`, from inside it.
    fn break_box(e: &mut Editor, id: EntityId) {
        let EntityId::Instance(i) = id else { panic!() };
        let def = e.doc().model().instance(i).unwrap().definition;
        let top = e
            .doc()
            .scene()
            .definition(def)
            .unwrap()
            .mesh
            .faces()
            .find(|(_, f)| f.normal.z > 0.5)
            .map(|(f, _)| EntityId::Face(f))
            .unwrap();
        e.execute(Command::OpenPath { path: vec![id] }).unwrap();
        e.execute(Command::Erase { ids: vec![top] }).unwrap();
        e.execute(Command::OpenPath { path: vec![] }).unwrap();
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
    fn check_lists_the_broken_part_and_make_solid_closes_it() {
        let mut e = editor();
        add_box(&mut e, 0.0);
        let bad = add_box(&mut e, 3.0);
        break_box(&mut e, bad);
        select(&mut e, vec![]);
        assert!(e.check_health());
        let report = e.health_window().unwrap();
        assert_eq!(report.checked, 2);
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].id, bad);
        assert!(
            report.rows[0].label.ends_with(": 4 open edges"),
            "{}",
            report.rows[0].label
        );
        assert!(report.fixable());
        assert!(e.select_health_part(0));
        assert_eq!(
            e.doc().selection().iter().copied().collect::<Vec<_>>(),
            [bad]
        );
        assert!(e.make_solid_from_window());
        assert!(e.health_window().unwrap().rows.is_empty());
        assert_eq!(e.doc().undo_label(), Some("make_solid"));
    }

    #[test]
    fn a_selection_without_parts_asks_for_parts() {
        let mut e = editor();
        e.execute(Command::AddRectangle {
            corner: [10.0, 0.0, 0.0],
            opposite: [11.0, 1.0, 0.0],
            normal: None,
        })
        .unwrap();
        let face = e
            .doc()
            .model()
            .faces()
            .map(|(f, _)| EntityId::Face(f))
            .next()
            .unwrap();
        select(&mut e, vec![face]);
        assert!(e.check_health());
        assert!(e.health_window().is_none());
    }
}
