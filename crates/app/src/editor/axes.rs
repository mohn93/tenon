//! Group > Make Unique and Flip Along, and Reset Axes (the viewport menu).

use std::collections::BTreeMap;

use kernel::{DefinitionKind, EntityId};
use session::{Command, FlipAxis, FlipFrame};

use super::{Editor, Message};
use crate::info::count;

impl Editor {
    /// How many selected component instances Make Unique would give a definition of their own:
    /// those whose definition the root reaches placed more often than the selection holds it
    /// (the kernel's rule). Kept for the revision, context and selection it was worked out for.
    fn unique_candidates(&self) -> usize {
        let key = (
            self.doc.revision(),
            self.doc.context().to_vec(),
            self.doc.selection().clone(),
        );
        if let Some((r, c, s, n)) = &*self.unique_cache.borrow()
            && (*r, c, s) == (key.0, &key.1, &key.2)
        {
            return *n;
        }
        let m = self.doc.model();
        let scene = self.doc.scene();
        let mut selected: BTreeMap<_, usize> = BTreeMap::new();
        for id in self.doc.selection() {
            if let EntityId::Instance(i) = id
                && let Some(i) = m.instance(*i)
                && scene
                    .definition(i.definition)
                    .is_some_and(|d| d.kind == DefinitionKind::Component)
            {
                *selected.entry(i.definition).or_default() += 1;
            }
        }
        let n = if selected.is_empty() {
            0
        } else {
            let placed = scene.placed_instance_counts();
            selected
                .iter()
                .filter(|(d, n)| placed.get(d).copied().unwrap_or(0) > **n)
                .map(|(_, n)| *n)
                .sum()
        };
        *self.unique_cache.borrow_mut() = Some((key.0, key.1, key.2, n));
        n
    }

    /// Whether Make Unique has something to do (`unique_candidates`).
    pub fn can_make_unique(&self) -> bool {
        self.unique_candidates() > 0
    }

    /// Gives the selected instances of shared definitions definitions of their own as one undo
    /// step, and says how many instances: "Made 2 components unique", or "Nothing to make
    /// unique".
    pub fn make_unique(&mut self) -> bool {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let candidates = self.unique_candidates();
        if self.run(Command::MakeUnique { ids: None }).is_some() {
            self.message = Some(Message::info(if candidates == 0 {
                "Nothing to make unique".to_string()
            } else {
                format!("Made {} unique", count(candidates, "component"))
            }));
        }
        self.invalidate_hover();
        true
    }

    /// Whether Flip Along the component's axes applies: exactly one instance is selected.
    pub fn can_flip_own(&self) -> bool {
        matches!(
            self.doc.selection().iter().collect::<Vec<_>>()[..],
            [EntityId::Instance(_)]
        )
    }

    /// Mirrors the selection along `axis` of the drawing axes, or (`FlipFrame::Own`) the one
    /// selected instance along its own axis, as one undo step. With nothing selected it says so.
    pub fn flip(&mut self, axis: FlipAxis, frame: FlipFrame) -> bool {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        if self.doc.selection().is_empty() {
            self.message = Some(Message::error("Select something to flip"));
            return true;
        }
        if frame == FlipFrame::Own && !self.can_flip_own() {
            self.message = Some(Message::error("Select one group or component to flip"));
            return true;
        }
        if self
            .run(Command::Flip {
                ids: None,
                axis,
                frame,
            })
            .is_some()
        {
            let colour = match axis {
                FlipAxis::Red => "red",
                FlipAxis::Green => "green",
                FlipAxis::Blue => "blue",
            };
            self.message = Some(Message::info(match frame {
                FlipFrame::Axes => format!("Flipped along {colour}"),
                FlipFrame::Own => format!("Flipped along the component's {colour}"),
            }));
        }
        self.invalidate_hover();
        true
    }

    /// Whether the model axes are moved and no group or component is open, so Reset Axes
    /// has something to put back.
    pub fn axes_moved(&self) -> bool {
        self.doc.context().is_empty() && !self.doc.scene().axes().is_world()
    }

    /// Puts the model axes back to the world axes as one undo step.
    pub fn reset_axes(&mut self) -> bool {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        if self.run(Command::ResetAxes {}).is_some() {
            self.message = Some(Message::info("Axes reset"));
        }
        self.invalidate_hover();
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};
    use kernel::EntityId;
    use session::{Command, FlipAxis, FlipFrame, SelectMode};
    use tools::ToolKind;

    use crate::editor::{Button, Editor};

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

    /// A 2 m box component "Box" and a copy of it 5 m along X, the copy selected; returns
    /// (first, copy).
    fn two_boxes(e: &mut Editor) -> (EntityId, EntityId) {
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        e.execute(Command::PushPull {
            face: "f8".parse().unwrap(),
            distance: 1.0,
        })
        .unwrap();
        let m = e.doc().model();
        let ids = m
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
            .collect();
        e.execute(Command::MakeComponent {
            ids,
            name: "Box".into(),
        })
        .unwrap();
        let first = *e.doc().selection().first().unwrap();
        let out = e
            .execute(Command::Copy {
                ids: vec![first],
                offset: [5.0, 0.0, 0.0],
                count: None,
            })
            .unwrap();
        let copy = *out.changes.created.first().unwrap();
        select(e, vec![copy]);
        (first, copy)
    }

    #[test]
    fn make_unique_needs_a_selected_instance_with_company() {
        let mut e = editor();
        let (first, copy) = two_boxes(&mut e);
        assert!(e.can_make_unique());
        select(&mut e, vec![first, copy]);
        assert!(!e.can_make_unique(), "every instance is selected");
        select(&mut e, vec![]);
        assert!(!e.can_make_unique());
        select(&mut e, vec![copy]);
        assert!(e.make_unique());
        assert!(
            e.status().ends_with("Made 1 component unique"),
            "{}",
            e.status()
        );
        assert!(
            !e.can_make_unique(),
            "the copy is the only one of its own now"
        );
        e.execute(Command::Undo {
            count: None,
            expect: None,
        })
        .unwrap();
        assert!(e.can_make_unique());
    }

    #[test]
    fn make_unique_counts_the_instances_it_made_unique() {
        let mut e = editor();
        let (first, copy) = two_boxes(&mut e);
        let out = e
            .execute(Command::Copy {
                ids: vec![first],
                offset: [0.0, 5.0, 0.0],
                count: None,
            })
            .unwrap();
        let third = *out.changes.created.first().unwrap();
        select(&mut e, vec![copy, third]);
        e.make_unique();
        assert!(
            e.status().ends_with("Made 2 components unique"),
            "{}",
            e.status()
        );
    }

    #[test]
    fn make_unique_follows_the_kernels_rule_for_unplaced_and_group_definitions() {
        let mut e = editor();
        let (first, copy) = two_boxes(&mut e);
        // Box A is made of the copy and erased: its Box instance is no longer placed.
        select(&mut e, vec![copy]);
        e.execute(Command::MakeComponent {
            ids: vec![copy],
            name: "A".into(),
        })
        .unwrap();
        let a = *e.doc().selection().first().unwrap();
        e.execute(Command::Erase { ids: vec![a] }).unwrap();
        select(&mut e, vec![first]);
        assert!(!e.can_make_unique(), "the other Box instance is not placed");
        e.make_unique();
        assert!(
            e.status().ends_with("Nothing to make unique"),
            "{}",
            e.status()
        );
    }

    #[test]
    fn making_nothing_unique_says_so() {
        let mut e = editor();
        let (first, copy) = two_boxes(&mut e);
        select(&mut e, vec![first, copy]);
        e.make_unique();
        assert!(
            e.status().ends_with("Nothing to make unique"),
            "{}",
            e.status()
        );
    }

    #[test]
    fn flip_says_the_axis_and_needs_one_instance_for_the_components_own() {
        let mut e = editor();
        let (first, copy) = two_boxes(&mut e);
        assert!(e.can_flip_own());
        let revision = e.doc().revision();
        e.flip(FlipAxis::Red, FlipFrame::Axes);
        assert!(e.status().ends_with("Flipped along red"), "{}", e.status());
        assert!(e.doc().revision() > revision);
        e.flip(FlipAxis::Blue, FlipFrame::Own);
        assert!(
            e.status().ends_with("Flipped along the component's blue"),
            "{}",
            e.status()
        );
        select(&mut e, vec![first, copy]);
        assert!(!e.can_flip_own());
        let revision = e.doc().revision();
        e.flip(FlipAxis::Green, FlipFrame::Own);
        assert!(
            e.status()
                .ends_with("Select one group or component to flip"),
            "{}",
            e.status()
        );
        assert_eq!(e.doc().revision(), revision);
        select(&mut e, vec![]);
        e.flip(FlipAxis::Green, FlipFrame::Axes);
        assert!(
            e.status().ends_with("Select something to flip"),
            "{}",
            e.status()
        );
    }

    #[test]
    fn reset_axes_is_offered_only_at_the_root_with_moved_axes() {
        let mut e = editor();
        assert!(!e.axes_moved());
        e.execute(Command::SetAxes {
            origin: [1.0, 0.0, 0.0],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [-1.0, 0.0, 0.0],
        })
        .unwrap();
        assert!(e.axes_moved());
        let menu = e.viewport_menu(egui::Pos2::ZERO, None).expect("a menu");
        assert!(menu.reset_axes, "even with nothing selected");
        assert!(e.reset_axes());
        assert!(e.status().ends_with("Axes reset"), "{}", e.status());
        assert!(!e.axes_moved());
        assert_eq!(e.viewport_menu(egui::Pos2::ZERO, None), None);
        e.execute(Command::Undo {
            count: None,
            expect: None,
        })
        .unwrap();
        assert!(e.axes_moved());
        let (first, _) = two_boxes(&mut e);
        e.execute(Command::OpenInstance { id: first }).unwrap();
        assert!(
            !e.axes_moved(),
            "inside a context there is nothing to reset to"
        );
    }

    #[test]
    fn the_viewport_menu_offers_make_unique_and_flip() {
        let mut e = editor();
        assert_eq!(e.viewport_menu(egui::Pos2::ZERO, None), None);
        let (first, copy) = two_boxes(&mut e);
        let menu = e.viewport_menu(egui::Pos2::ZERO, None).expect("a menu");
        assert!(menu.make_unique && menu.flip && menu.flip_own && !menu.reset_axes);
        select(&mut e, vec![first, copy]);
        let menu = e.viewport_menu(egui::Pos2::ZERO, None).expect("a menu");
        assert!(!menu.make_unique && menu.flip && !menu.flip_own);
    }

    #[test]
    fn the_axes_tool_moves_the_axes_in_three_clicks() {
        let mut e = editor();
        e.select_tool(ToolKind::Axes);
        for p in [
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(1.0, 3.0, 0.0),
            DVec3::new(-2.0, 3.0, 0.0),
        ] {
            let px = e.doc().camera.project(p).unwrap();
            e.cursor_moved(px);
            e.button(Button::Left, true);
            e.button(Button::Left, false);
        }
        assert!(e.status().ends_with("Axes moved"), "{}", e.status());
        let axes = e.doc().scene().axes();
        assert!(
            axes.origin.distance(DVec3::new(1.0, 1.0, 0.0)) < 1e-6,
            "{axes:?}"
        );
        assert!((axes.x - DVec3::Y).length() < 1e-6, "{axes:?}");
        assert!((axes.y + DVec3::X).length() < 1e-6, "{axes:?}");
    }
}
