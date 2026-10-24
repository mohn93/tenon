//! Section planes in the window: the View menu's display toggles and the context menu on a
//! selected plane. Each change is the command an agent would send.

use kernel::EntityId;
use session::Command;

use super::Editor;

/// The one section plane that is selected: its id and whether it is the open context's active
/// plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedSection {
    pub id: EntityId,
    pub active: bool,
}

impl Editor {
    /// The selected section plane, when it is the only thing selected.
    pub fn selected_section(&self) -> Option<SelectedSection> {
        let mut selected = self.doc.selection().iter();
        let (Some(&id @ EntityId::Section(s)), None) = (selected.next(), selected.next()) else {
            return None;
        };
        let m = self.doc.model();
        m.section(s)?;
        Some(SelectedSection {
            id,
            active: m.active_section() == Some(s),
        })
    }

    /// Whether every selected instance is cut by section planes outside it (Entity Info's
    /// "Cut by section planes", unchecked for a mix); None when no instance is selected.
    pub fn selection_section_cut(&self) -> Option<bool> {
        let m = self.doc.model();
        let mut seen = false;
        let mut cut = true;
        for id in self.doc.selection() {
            if let EntityId::Instance(i) = id
                && let Some(c) = m.section_cut_of(*i)
            {
                seen = true;
                cut &= c;
            }
        }
        seen.then_some(cut)
    }

    /// Entity Info's "Cut by section planes" checkbox: sets it for every selected instance, as
    /// one undo step.
    pub fn set_selection_section_cut(&mut self, cut: bool) -> bool {
        let ids: Vec<EntityId> = self
            .doc
            .selection()
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Instance(_)))
            .collect();
        if ids.is_empty() {
            return false;
        }
        self.message = None;
        self.run(Command::SetSectionCut { ids, cut });
        true
    }

    /// Shows or hides the section planes, the cuts they make, or the fill of the cuts (None:
    /// as it is). False if nothing changed.
    pub fn set_section_display(
        &mut self,
        planes: Option<bool>,
        cuts: Option<bool>,
        fill: Option<bool>,
    ) -> bool {
        let now = self.doc.section_display;
        let same = planes.is_none_or(|v| v == now.planes)
            && cuts.is_none_or(|v| v == now.cuts)
            && fill.is_none_or(|v| v == now.fill);
        if same {
            return false;
        }
        self.run_camera_command(Command::SetSectionDisplay {
            planes,
            cuts,
            fill,
            fill_color: None,
        });
        true
    }

    /// Negates the selected section plane's normal (the other side is kept). False if no
    /// single plane is selected.
    pub fn flip_selected_section(&mut self) -> bool {
        let Some(selected) = self.selected_section() else {
            return false;
        };
        self.message = None;
        self.run(Command::FlipSectionPlane { plane: selected.id });
        self.invalidate_hover();
        true
    }

    /// Makes the selected section plane the active one (`on`) or clears the open context's
    /// active plane (when it is this one). False if no single plane is selected or nothing
    /// changes.
    pub fn set_selected_section_active(&mut self, on: bool) -> bool {
        let Some(selected) = self.selected_section() else {
            return false;
        };
        if selected.active == on {
            return false;
        }
        self.message = None;
        self.run(if on {
            Command::ActivateSectionPlane { plane: selected.id }
        } else {
            Command::DeactivateSection {}
        });
        self.invalidate_hover();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::Command;

    fn editor_with_planes() -> (Editor, EntityId, EntityId) {
        let mut e = Editor::new(glam::DVec2::new(800.0, 600.0), 1.0);
        let add = |e: &mut Editor, x: f64, activate: bool| {
            e.execute_recorded(Command::AddSectionPlane {
                point: [x, 0.0, 0.0],
                normal: [1.0, 0.0, 0.0],
                activate: Some(activate),
            })
            .unwrap()
            .changes
            .created[0]
        };
        let (a, b) = (add(&mut e, 1.0, true), add(&mut e, 2.0, false));
        (e, a, b)
    }

    fn select(e: &mut Editor, ids: &[EntityId]) {
        e.execute_recorded(Command::Select {
            ids: ids.to_vec(),
            mode: session::SelectMode::Replace,
            all: false,
        })
        .unwrap();
    }

    #[test]
    fn the_cut_checkbox_sets_every_selected_instance_in_one_undo_step() {
        let mut e = Editor::new(glam::DVec2::new(800.0, 600.0), 1.0);
        let mut groups = Vec::new();
        for x in [0.0, 5.0] {
            e.execute_recorded(Command::AddRectangle {
                corner: [x, 0.0, 0.0],
                opposite: [x + 1.0, 1.0, 0.0],
                normal: None,
            })
            .unwrap();
            let m = e.doc().model();
            let ids: Vec<EntityId> = m
                .faces()
                .map(|(id, _)| EntityId::Face(id))
                .chain(m.edges().map(|(id, _)| EntityId::Edge(id)))
                .collect();
            let out = e
                .execute_recorded(Command::MakeGroup { ids, name: None })
                .unwrap();
            groups.push(
                *out.changes
                    .created
                    .iter()
                    .find(|id| matches!(id, EntityId::Instance(_)))
                    .unwrap(),
            );
        }
        select(&mut e, &[]);
        assert_eq!(e.selection_section_cut(), None, "nothing selected");
        select(&mut e, &groups);
        assert_eq!(e.selection_section_cut(), Some(true));
        assert!(e.set_selection_section_cut(false));
        assert_eq!(e.selection_section_cut(), Some(false));
        select(&mut e, &groups[..1]);
        assert!(e.set_selection_section_cut(true));
        select(&mut e, &groups);
        assert_eq!(e.selection_section_cut(), Some(false), "a mix is unchecked");
        // The last step set only the first group back; undoing it leaves both off, and one
        // more undo (the step for both) puts both back.
        for expected in [Some(false), Some(true)] {
            e.execute_recorded(Command::Undo {
                count: None,
                expect: None,
            })
            .unwrap();
            assert_eq!(e.selection_section_cut(), expected);
        }
    }

    #[test]
    fn only_a_single_selected_plane_has_a_context_menu() {
        let (mut e, a, b) = editor_with_planes();
        assert_eq!(e.selected_section(), None);
        select(&mut e, &[a]);
        assert_eq!(
            e.selected_section(),
            Some(SelectedSection {
                id: a,
                active: true
            })
        );
        select(&mut e, &[b]);
        assert_eq!(
            e.selected_section(),
            Some(SelectedSection {
                id: b,
                active: false
            })
        );
        select(&mut e, &[a, b]);
        assert_eq!(e.selected_section(), None);
    }

    #[test]
    fn flip_and_active_cut_send_the_agent_commands() {
        let (mut e, a, b) = editor_with_planes();
        select(&mut e, &[a]);
        assert!(e.flip_selected_section());
        let EntityId::Section(id) = a else { panic!() };
        assert_eq!(e.doc().model().section(id).unwrap().normal.x, -1.0);
        // Unchecking Active Cut clears the active plane; checking it on another makes that
        // one active.
        assert!(e.set_selected_section_active(false));
        assert_eq!(e.doc().model().active_section(), None);
        assert!(!e.set_selected_section_active(false), "already so");
        select(&mut e, &[b]);
        assert!(e.set_selected_section_active(true));
        let EntityId::Section(other) = b else {
            panic!()
        };
        assert_eq!(e.doc().model().active_section(), Some(other));
        // Nothing selected: nothing to do.
        select(&mut e, &[]);
        assert!(!e.flip_selected_section());
    }

    #[test]
    fn the_display_toggles_are_view_settings() {
        let (mut e, ..) = editor_with_planes();
        assert!(e.set_section_display(None, Some(false), None));
        assert!(!e.doc().section_display.cuts);
        assert!(e.doc().section_display.planes);
        assert!(
            !e.set_section_display(None, Some(false), None),
            "already off"
        );
        assert!(e.set_section_display(Some(false), None, Some(false)));
        assert!(!e.doc().section_display.planes && !e.doc().section_display.fill);
    }
}
