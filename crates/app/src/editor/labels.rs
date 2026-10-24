//! The labels of the model's dimensions and notes: what the window writes over the view for
//! them, and the rectangles it last wrote them in, which the tools pick by.

use kernel::EntityId;
use session::note_label;
use tools::{Label, Owner, ToolKind};

use super::{Editor, scenery_in};
use crate::feedback::{HudLabel, LabelAnchor, LabelLook};
use session::Document;

/// The labels the window last drew, with the document, revision and open context they were
/// drawn for.
pub(super) type DrawnLabels = Option<(u64, u64, Vec<kernel::InstanceId>, Vec<Label>)>;

impl Editor {
    /// Where the window last drew the labels, in view pixels (see `viewport::hud`): the tools
    /// pick dimensions and notes by them while the model and the open context stay as they
    /// are now.
    pub fn set_labels(&mut self, labels: Vec<Label>) {
        self.labels = Some((
            self.documents,
            self.doc.revision(),
            self.doc.context().to_vec(),
            labels,
        ));
    }

    /// The label of every dimension and note in view, in view pixels: a dimension's at the
    /// middle of its line (a radial one's at its label point), a leader note's at its text's
    /// position, a screen note's at its place on the view. A dimension measures what is drawn
    /// (its shape placed in root coordinates), so one in a scaled group shows the scaled
    /// length. Labels whose point is out of view (or behind the eye) are left out, and so are
    /// those on hidden tags. Those of the open context, or of an instance of it, are lit as it
    /// is selected or hovered (in the danger color when Erase hovers); those outside the open
    /// instance are faded.
    pub(super) fn hud_labels(&self) -> Vec<HudLabel> {
        self.labels_seen_by(&self.doc.camera, self.hovered())
    }

    /// The labels of an image of the model rendered from `camera` (its viewport the image size,
    /// in pixels), as the window would draw them for that view: those of `hud_labels`, with
    /// nothing hovered.
    pub fn image_labels(&self, camera: &session::Camera) -> Vec<HudLabel> {
        self.labels_seen_by(camera, None)
    }

    /// `hud_labels` for `camera`, with `hovered` lit.
    fn labels_seen_by(&self, camera: &session::Camera, hovered: Option<EntityId>) -> Vec<HudLabel> {
        let layout = self.layout();
        let scenery = scenery_in(&layout, &self.doc);
        let to_root = self.doc.context_transform();
        let units = self.doc.units();
        let selection = self.doc.selection();
        let mut out = Vec::new();
        for &(k, id, shape) in scenery.annotations() {
            let part = scenery.part(k);
            let text = match id {
                // Measured as drawn: a scaled group's dimension shows the scaled length.
                EntityId::Dimension(_) => shape.transformed(to_root).dimension_label(&units),
                EntityId::Note(n) => note_label(part.model, n, &units, to_root * part.place),
                EntityId::Vertex(_)
                | EntityId::Edge(_)
                | EntityId::Face(_)
                | EntityId::Instance(_)
                | EntityId::Guide(_)
                | EntityId::Section(_) => None,
            };
            let Some(text) = text else { continue };
            let (at, anchor) = match (shape.label_point(), shape) {
                (Some(p), _) => {
                    let Some(at) = camera.project(to_root.transform_point3(p)) else {
                        continue;
                    };
                    // Out of view: not drawn, so not picked either.
                    if !(at.cmpge(glam::DVec2::ZERO).all() && at.cmple(camera.viewport).all()) {
                        continue;
                    }
                    (at, LabelAnchor::Center)
                }
                (None, session::AnnotationShape::Screen { at }) => (
                    glam::DVec2::new(at[0], at[1]) * camera.viewport,
                    LabelAnchor::TopLeft,
                ),
                (None, _) => continue,
            };
            // What selecting or hovering lights the label: the annotation itself, or the
            // instance placing it.
            let lit = match part.owner {
                Owner::Open => Some(id),
                Owner::Instance(i) => Some(EntityId::Instance(i)),
                Owner::Outside => None,
            };
            let look = match lit {
                None => LabelLook::Faded,
                Some(lit) if selection.contains(&lit) => LabelLook::Selected,
                Some(lit) if hovered == Some(lit) && self.kind == ToolKind::Erase => {
                    LabelLook::Erasing
                }
                Some(lit) if hovered == Some(lit) => LabelLook::Hovered,
                Some(_) => LabelLook::Normal,
            };
            out.push(HudLabel {
                text,
                at,
                anchor,
                look,
                owner: part.owner,
                id,
            });
        }
        out
    }
}

/// The labels the tools pick by: the last drawn (`drawn`), unless the document (the
/// `documents`th), its model or its open context changed since.
pub(super) fn current<'a>(drawn: &'a DrawnLabels, documents: u64, doc: &Document) -> &'a [Label] {
    match drawn {
        Some((d, revision, context, labels))
            if *d == documents && *revision == doc.revision() && context == doc.context() =>
        {
            labels
        }
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};
    use kernel::EntityId;
    use session::{AnchorInput, Command, SelectMode};
    use tools::{Label, Owner};

    use super::super::Editor;
    use crate::feedback::{HudLabel, LabelAnchor, LabelLook};
    use crate::navigation::Button;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    fn create(e: &mut Editor, command: Command) -> EntityId {
        let out = e.execute(command).unwrap();
        *out.changes
            .created
            .iter()
            .find(|id| matches!(id, EntityId::Dimension(_) | EntityId::Note(_)))
            .unwrap()
    }

    fn dimension(e: &mut Editor) -> EntityId {
        let point = |p: [f64; 3]| AnchorInput::Point(p);
        create(
            e,
            Command::AddDimension {
                a: point([0.0, 0.0, 0.0]),
                b: point([2.0, 0.0, 0.0]),
                offset: [0.0, -1.0, 0.0],
            },
        )
    }

    fn labels(e: &mut Editor) -> Vec<HudLabel> {
        e.refresh_hover();
        e.hud(false).labels
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
    fn a_dimension_is_labelled_at_the_middle_of_its_line_and_lit_when_selected() {
        let mut e = editor();
        let d = dimension(&mut e);
        let [label] = &labels(&mut e)[..] else {
            panic!("{:?}", labels(&mut e));
        };
        assert_eq!(label.text, "2 m");
        assert_eq!((label.owner, label.id), (Owner::Open, d));
        assert_eq!(label.anchor, LabelAnchor::Center);
        assert_eq!(label.look, LabelLook::Normal);
        let mid = e.doc().camera.project(DVec3::new(1.0, -1.0, 0.0)).unwrap();
        assert!(label.at.distance(mid) < 1e-9);
        select(&mut e, vec![d]);
        assert_eq!(labels(&mut e)[0].look, LabelLook::Selected);
        // Display units change the text.
        e.execute(Command::SetUnits {
            length: Some(session::LengthUnit::Millimeters),
            precision: None,
            length_snap: None,
            angle_precision: None,
            angle_snap: None,
        })
        .unwrap();
        assert_eq!(labels(&mut e)[0].text, "2000 mm");
    }

    #[test]
    fn labels_behind_the_eye_are_hidden_and_screen_notes_sit_at_their_fractions() {
        let mut e = editor();
        let camera = e.doc().camera.clone();
        let behind = camera.eye - camera.forward() * 3.0;
        create(
            &mut e,
            Command::AddNote {
                anchor: AnchorInput::Point(camera.target.to_array()),
                position: behind.to_array(),
                text: Some("Behind".into()),
            },
        );
        let screen = create(
            &mut e,
            Command::AddScreenNote {
                at: [0.25, 0.5],
                text: "Plan".into(),
            },
        );
        let [label] = &labels(&mut e)[..] else {
            panic!("{:?}", labels(&mut e));
        };
        assert_eq!((label.id, label.text.as_str()), (screen, "Plan"));
        assert_eq!(label.anchor, LabelAnchor::TopLeft);
        assert_eq!(label.at, DVec2::new(200.0, 300.0));
    }

    #[test]
    fn a_groups_labels_light_with_it_and_fade_outside_the_open_group() {
        let mut e = editor();
        let d = dimension(&mut e);
        let out = e
            .execute(Command::MakeGroup {
                ids: vec![d],
                name: None,
            })
            .err();
        assert!(out.is_some(), "annotations alone are not grouped");
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        e.execute(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
            all: true,
        })
        .unwrap();
        let ids: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        let group = e
            .execute(Command::MakeGroup { ids, name: None })
            .unwrap()
            .changes
            .created
            .into_iter()
            .find(|id| matches!(id, EntityId::Instance(_)))
            .unwrap();
        let EntityId::Instance(i) = group else {
            unreachable!()
        };
        let label = labels(&mut e).remove(0);
        assert_eq!(
            (label.owner, label.text.as_str()),
            (Owner::Instance(i), "2 m")
        );
        select(&mut e, vec![group]);
        assert_eq!(labels(&mut e)[0].look, LabelLook::Selected);
        // A screen note in the root, then the group opened: the note is outside it.
        create(
            &mut e,
            Command::AddScreenNote {
                at: [0.1, 0.1],
                text: "Title".into(),
            },
        );
        e.execute(Command::OpenInstance { id: group }).unwrap();
        let looks: Vec<(Owner, LabelLook)> =
            labels(&mut e).iter().map(|l| (l.owner, l.look)).collect();
        assert_eq!(
            looks,
            vec![
                (Owner::Open, LabelLook::Normal),
                (Owner::Outside, LabelLook::Faded)
            ]
        );
    }

    #[test]
    fn select_picks_a_dimension_by_its_drawn_label_until_the_model_changes() {
        let mut e = editor();
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        let d = dimension(&mut e);
        e.execute(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        // A label drawn over the rectangle's face: clicking there selects the dimension.
        let over = e.doc().camera.project(DVec3::new(1.0, 1.0, 0.0)).unwrap();
        e.set_labels(vec![Label {
            owner: Owner::Open,
            id: d,
            min: over - DVec2::splat(15.0),
            max: over + DVec2::splat(15.0),
        }]);
        let click = |e: &mut Editor| {
            e.cursor_moved(over);
            e.button(Button::Left, true);
            e.button(Button::Left, false);
            e.doc().selection().iter().copied().collect::<Vec<_>>()
        };
        assert_eq!(click(&mut e), vec![d]);
        e.cursor_moved(over + DVec2::ONE);
        e.refresh_hover();
        assert_eq!(e.hovered(), Some(d));
        // After an edit the old rectangles are stale: the face is picked again.
        e.execute(Command::SetUnits {
            length: None,
            precision: Some(2),
            length_snap: None,
            angle_precision: None,
            angle_snap: None,
        })
        .unwrap();
        assert!(matches!(click(&mut e)[..], [EntityId::Face(_)]));
    }

    /// A 2 x 2 rectangle and the dimension of its 2 m side (line at y = -1), grouped.
    fn grouped(e: &mut Editor) -> (EntityId, EntityId) {
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        let d = dimension(e);
        e.execute(Command::Select {
            ids: Vec::new(),
            mode: SelectMode::Replace,
            all: true,
        })
        .unwrap();
        let ids: Vec<EntityId> = e.doc().selection().iter().copied().collect();
        let group = e
            .execute(Command::MakeGroup { ids, name: None })
            .unwrap()
            .changes
            .created
            .into_iter()
            .find(|id| matches!(id, EntityId::Instance(_)))
            .unwrap();
        (group, d)
    }

    /// The label text and the Entity Info lines of dimension `d` selected inside `group`.
    fn label_and_info(e: &mut Editor, group: EntityId, d: EntityId) -> (String, Vec<String>) {
        let label = labels(e).remove(0).text;
        e.execute(Command::OpenInstance { id: group }).unwrap();
        select(e, vec![d]);
        let info = crate::info::entity_info(e.doc());
        e.execute(Command::CloseInstance {}).unwrap();
        (label, info)
    }

    #[test]
    fn a_dimension_in_a_scaled_or_turned_group_is_labelled_as_drawn() {
        let mut e = editor();
        let (group, d) = grouped(&mut e);
        e.execute(Command::Scale {
            ids: vec![group],
            origin: [0.0, 0.0, 0.0],
            factors: [2.0, 2.0, 2.0],
        })
        .unwrap();
        let (label, info) = label_and_info(&mut e, group, d);
        assert_eq!(label, "4 m");
        assert_eq!(info, ["Editing group", "1 linear dimension", "Text: 4 m"]);
        // Turned a quarter about Z (and still scaled): the same length, at the turned line.
        e.execute(Command::Rotate {
            ids: vec![group],
            center: [0.0, 0.0, 0.0],
            axis: None,
            angle: 90.0,
            copies: None,
        })
        .unwrap();
        let mid = e.doc().camera.project(DVec3::new(2.0, 2.0, 0.0)).unwrap();
        assert!(labels(&mut e)[0].at.distance(mid) < 1e-6);
        let (label, info) = label_and_info(&mut e, group, d);
        assert_eq!(label, "4 m");
        assert_eq!(info[2], "Text: 4 m");
    }

    #[test]
    fn labels_out_of_view_are_left_out_and_erase_hovers_in_danger() {
        let mut e = editor();
        let d = dimension(&mut e);
        // Far to the side, out of view.
        let point = |p: [f64; 3]| AnchorInput::Point(p);
        create(
            &mut e,
            Command::AddDimension {
                a: point([5000.0, 0.0, 0.0]),
                b: point([5002.0, 0.0, 0.0]),
                offset: [0.0, -1.0, 0.0],
            },
        );
        let shown: Vec<EntityId> = labels(&mut e).iter().map(|l| l.id).collect();
        assert_eq!(shown, vec![d]);
        e.select_tool(tools::ToolKind::Erase);
        let on_line = e.doc().camera.project(DVec3::new(1.5, -1.0, 0.0)).unwrap();
        e.cursor_moved(on_line);
        e.refresh_hover();
        assert_eq!(e.hovered(), Some(d));
        assert_eq!(labels(&mut e)[0].look, LabelLook::Erasing);
    }
}
