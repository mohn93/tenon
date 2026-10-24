//! Tools > Solid Tools: Union, Subtract, Intersect, Trim, Split and Outer Shell on the
//! selected groups and components.

use kernel::EntityId;
use session::{Command, Query, QueryResult};

use super::{Editor, Message, shown_name};

/// One of the Solid Tools menu's operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolidOp {
    Union,
    Subtract,
    Intersect,
    Trim,
    Split,
    OuterShell,
}

impl SolidOp {
    pub const ALL: [SolidOp; 6] = [
        SolidOp::Union,
        SolidOp::Subtract,
        SolidOp::Intersect,
        SolidOp::Trim,
        SolidOp::Split,
        SolidOp::OuterShell,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SolidOp::Union => "Union",
            SolidOp::Subtract => "Subtract",
            SolidOp::Intersect => "Intersect",
            SolidOp::Trim => "Trim",
            SolidOp::Split => "Split",
            SolidOp::OuterShell => "Outer Shell",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|o| *o == self).unwrap_or(0)
    }
}

/// Whether a Solid Tools item can act on the selection, and what its tooltip says: what it
/// would do (naming the target) when enabled, why not when disabled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SolidItem {
    pub enabled: bool,
    pub tooltip: Option<String>,
}

/// The six items' state, in `SolidOp::ALL` order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SolidTools {
    items: [SolidItem; 6],
}

impl SolidTools {
    /// Every item enabled, for tests of the menu's wiring.
    #[cfg(test)]
    pub fn all_enabled() -> Self {
        let on = SolidItem {
            enabled: true,
            tooltip: None,
        };
        Self {
            items: std::array::from_fn(|_| on.clone()),
        }
    }

    /// Every item disabled with `reason`, for tests of the menu's wiring.
    #[cfg(test)]
    pub fn refused(reason: &str) -> Self {
        let off = SolidItem {
            enabled: false,
            tooltip: Some(reason.to_string()),
        };
        Self {
            items: std::array::from_fn(|_| off.clone()),
        }
    }

    pub fn get(&self, op: SolidOp) -> &SolidItem {
        &self.items[op.index()]
    }
}

/// What `Editor::solid_tools` was worked out for, and its answer.
pub(super) type SolidCache = (
    u64,
    Vec<kernel::InstanceId>,
    std::collections::BTreeSet<EntityId>,
    SolidTools,
);

const PLURAL: &str = "Select two or more solid groups or components";

impl Editor {
    /// The selection's order is by id (it keeps no pick order), so the target of Subtract and
    /// Trim is the selected instance with the lowest id.
    fn solid_target(&self) -> Option<EntityId> {
        self.doc
            .selection()
            .iter()
            .copied()
            .find(|id| matches!(id, EntityId::Instance(_)))
    }

    /// The name the target goes by in a tooltip: "Box (i3)".
    fn solid_target_name(&self, id: EntityId) -> String {
        let EntityId::Instance(i) = id else {
            return id.to_string();
        };
        let named = self
            .doc
            .model()
            .instance(i)
            .and_then(|inst| self.doc.scene().definition(inst.definition))
            .map(|d| shown_name(d.kind, &d.name).to_string());
        match named {
            Some(name) if !name.is_empty() => format!("{name} ({id})"),
            _ => id.to_string(),
        }
    }

    /// Why the selection is not a set of solids, if it is not one.
    fn not_solids(&self) -> Option<String> {
        for id in self.doc.selection() {
            if !matches!(id, EntityId::Instance(_)) {
                return Some("Select only groups or components".to_string());
            }
        }
        for id in self.doc.selection() {
            if let Ok(QueryResult::Solid {
                solid: false,
                reason,
                ..
            }) = self.doc.query(&Query::Solid { id: *id })
            {
                let why = reason.unwrap_or_default();
                return Some(format!(
                    "{} is not a solid: {why}",
                    self.solid_target_name(*id)
                ));
            }
        }
        None
    }

    /// The state of the Solid Tools items for the selection, kept for the revision, context
    /// and selection it was worked out for (it asks the kernel about every selected solid).
    pub fn solid_tools(&self) -> SolidTools {
        let key = (
            self.doc.revision(),
            self.doc.context().to_vec(),
            self.doc.selection().clone(),
        );
        if let Some((r, c, s, tools)) = &*self.solid_cache.borrow()
            && (*r, c, s) == (key.0, &key.1, &key.2)
        {
            return tools.clone();
        }
        let tools = self.work_out_solid_tools();
        *self.solid_cache.borrow_mut() = Some((key.0, key.1, key.2, tools.clone()));
        tools
    }

    fn work_out_solid_tools(&self) -> SolidTools {
        let n = self.doc.selection().len();
        let refusal = if n == 0 {
            Some(PLURAL.to_string())
        } else {
            self.not_solids()
        };
        let mut tools = SolidTools::default();
        for op in SolidOp::ALL {
            let needs = match op {
                SolidOp::OuterShell => 1,
                _ => 2,
            };
            let reason = refusal.clone().or_else(|| {
                if n < needs {
                    Some(PLURAL.to_string())
                } else if op == SolidOp::Split && n != 2 {
                    Some("Split needs exactly two solids".to_string())
                } else {
                    None
                }
            });
            let item = match reason {
                Some(reason) => SolidItem {
                    enabled: false,
                    tooltip: Some(reason),
                },
                None => SolidItem {
                    enabled: true,
                    tooltip: self.solid_tip(op),
                },
            };
            tools.items[op.index()] = item;
        }
        tools
    }

    fn solid_tip(&self, op: SolidOp) -> Option<String> {
        let target = self.solid_target().map(|t| self.solid_target_name(t));
        Some(match (op, target) {
            (SolidOp::Subtract, Some(t)) => {
                format!("Cut the other selected solids out of {t} (the lowest-numbered)")
            }
            (SolidOp::Trim, Some(t)) => {
                format!(
                    "Cut the other selected solids out of {t} (the lowest-numbered), keeping them"
                )
            }
            (SolidOp::Union, _) => "Join the selected solids into one".to_string(),
            (SolidOp::Intersect, _) => "Keep what the selected solids share".to_string(),
            (SolidOp::Split, _) => {
                "Split the two solids into their parts and what they share".to_string()
            }
            (SolidOp::OuterShell, _) => {
                "Join the selected solids and drop what is inside".to_string()
            }
            _ => return None,
        })
    }

    /// Runs the Solid Tools operation on the selection as one undo step; the new group becomes
    /// the selection. A refusal (or a selection that does not suit) shows in the status line.
    pub fn solid_tool(&mut self, op: SolidOp) -> bool {
        let item = self.solid_tools().get(op).clone();
        if !item.enabled {
            self.message = Some(Message::error(item.tooltip.unwrap_or_default()));
            return true;
        }
        let ids: Vec<EntityId> = self.doc.selection().iter().copied().collect();
        let target = ids[0];
        let rest: Vec<EntityId> = ids[1..].to_vec();
        let command = match op {
            SolidOp::Union => Command::SolidUnion { ids },
            SolidOp::Intersect => Command::SolidIntersect { ids },
            SolidOp::OuterShell => Command::OuterShell { ids },
            SolidOp::Subtract => Command::SolidSubtract {
                target,
                tools: rest,
            },
            SolidOp::Trim => Command::SolidTrim {
                target,
                tools: rest,
            },
            SolidOp::Split => Command::SolidSplit {
                a: target,
                b: rest[0],
            },
        };
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        if let Some(outcome) = self.run(command) {
            self.message = Some(Message::info(if outcome.empty == Some(true) {
                format!("{}: nothing was left", op.label())
            } else {
                op.label().to_string()
            }));
        }
        self.invalidate_hover();
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use kernel::EntityId;
    use session::{Command, SelectMode};

    use super::SolidOp;
    use crate::editor::Editor;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    fn select(e: &mut Editor, ids: Vec<EntityId>) {
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
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
        let copy = out.placed.unwrap()[0];
        (first, copy)
    }

    fn instances(e: &Editor) -> usize {
        e.doc().model().instances().count()
    }

    #[test]
    fn the_items_need_solids_and_say_why_not() {
        let mut e = editor();
        let (a, b) = two_boxes(&mut e, 1.0);
        select(&mut e, vec![]);
        let t = e.solid_tools();
        for op in SolidOp::ALL {
            assert!(!t.get(op).enabled, "{op:?}");
            assert!(t.get(op).tooltip.as_deref().unwrap().contains("Select"));
        }
        select(&mut e, vec![a]);
        let t = e.solid_tools();
        assert!(!t.get(SolidOp::Union).enabled);
        assert!(
            t.get(SolidOp::Union)
                .tooltip
                .as_deref()
                .unwrap()
                .contains("two or more")
        );
        assert!(t.get(SolidOp::OuterShell).enabled);
        select(&mut e, vec![a, b]);
        let t = e.solid_tools();
        for op in SolidOp::ALL {
            assert!(t.get(op).enabled, "{op:?}");
        }
    }

    #[test]
    fn split_needs_exactly_two() {
        let mut e = editor();
        let (a, b) = two_boxes(&mut e, 1.0);
        let out = e
            .execute(Command::Copy {
                ids: vec![a],
                offset: [0.0, 1.0, 0.0],
                count: None,
            })
            .unwrap();
        let c = out.placed.unwrap()[0];
        select(&mut e, vec![a, b, c]);
        let t = e.solid_tools();
        assert!(t.get(SolidOp::Union).enabled);
        assert!(!t.get(SolidOp::Split).enabled);
        assert!(
            t.get(SolidOp::Split)
                .tooltip
                .as_deref()
                .unwrap()
                .contains("exactly two")
        );
    }

    #[test]
    fn a_selected_non_solid_names_the_reason() {
        let mut e = editor();
        let (a, _) = two_boxes(&mut e, 1.0);
        e.execute(Command::AddRectangle {
            corner: [10.0, 0.0, 0.0],
            opposite: [11.0, 1.0, 0.0],
            normal: None,
        })
        .unwrap();
        // A loose face next to the group: not only groups.
        let face = e
            .doc()
            .model()
            .faces()
            .map(|(id, _)| EntityId::Face(id))
            .last()
            .unwrap();
        select(&mut e, vec![a, face]);
        let t = e.solid_tools();
        assert!(!t.get(SolidOp::Union).enabled);
        assert!(
            t.get(SolidOp::Union)
                .tooltip
                .as_deref()
                .unwrap()
                .contains("only groups")
        );
        // A group of just that face is a group but not a solid.
        e.execute(Command::MakeGroup {
            ids: vec![face],
            name: None,
        })
        .unwrap();
        let flat = *e.doc().selection().first().unwrap();
        select(&mut e, vec![a, flat]);
        let t = e.solid_tools();
        assert!(!t.get(SolidOp::Union).enabled);
        assert!(
            t.get(SolidOp::Union)
                .tooltip
                .as_deref()
                .unwrap()
                .contains("is not a solid")
        );
    }

    #[test]
    fn union_through_the_menu_action_makes_one_group() {
        let mut e = editor();
        let (a, b) = two_boxes(&mut e, 1.0);
        select(&mut e, vec![a, b]);
        assert_eq!(instances(&e), 2);
        assert!(e.solid_tool(SolidOp::Union));
        assert_eq!(instances(&e), 1);
        assert_eq!(e.doc().selection().len(), 1);
        assert!(e.status().ends_with("Union"), "{}", e.status());
    }

    #[test]
    fn subtract_takes_the_lowest_id_as_target_and_names_it() {
        let mut e = editor();
        let (a, b) = two_boxes(&mut e, 1.0);
        select(&mut e, vec![b, a]);
        let tip = e
            .solid_tools()
            .get(SolidOp::Subtract)
            .tooltip
            .clone()
            .unwrap();
        assert!(tip.contains(&format!("({a})")), "{tip}");
        let before = e
            .doc()
            .model()
            .instances()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert!(e.solid_tool(SolidOp::Subtract));
        assert_eq!(instances(&e), 1);
        let after = e
            .doc()
            .model()
            .instances()
            .map(|(id, _)| id)
            .next()
            .unwrap();
        assert!(!before.contains(&after));
        // The cut box is 1 m wide: 2 x 2 x 2 minus the 1 m overlap.
        match e
            .doc()
            .query(&session::Query::Solid {
                id: EntityId::Instance(after),
            })
            .unwrap()
        {
            session::QueryResult::Solid {
                volume: Some(v), ..
            } => {
                assert!((v - 4.0).abs() < 1e-6, "{v}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn trim_keeps_the_tools() {
        let mut e = editor();
        let (a, b) = two_boxes(&mut e, 1.0);
        select(&mut e, vec![a, b]);
        assert!(e.solid_tool(SolidOp::Trim));
        assert_eq!(instances(&e), 2);
        assert!(
            e.doc()
                .model()
                .instance(match b {
                    EntityId::Instance(i) => i,
                    _ => unreachable!(),
                })
                .is_some()
        );
    }

    #[test]
    fn a_refused_operation_shows_in_the_status() {
        let mut e = editor();
        let (a, _) = two_boxes(&mut e, 1.0);
        select(&mut e, vec![a]);
        assert!(e.solid_tool(SolidOp::Union));
        assert!(e.status().contains("two or more"), "{}", e.status());
        assert_eq!(instances(&e), 2);
    }
}
