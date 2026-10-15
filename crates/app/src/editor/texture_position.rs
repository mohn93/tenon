//! Entity Info's texture position fields for selected faces.

use std::collections::BTreeMap;

use kernel::{EntityId, TexturePlacement};
use session::Command;

use super::Editor;

impl Editor {
    /// The texture placement Entity Info shows when only faces are selected: the first
    /// selected face's (the default placement when it has none). None otherwise.
    pub fn selection_texture_position(&self) -> Option<TexturePlacement> {
        let selection = self.doc.selection();
        let m = self.doc.model();
        let mut first = None;
        for id in selection {
            let EntityId::Face(f) = id else { return None };
            let face = m.face(*f)?;
            first.get_or_insert(face.texture.unwrap_or_default());
        }
        first
    }

    /// Sets the texture offset of `faces`, in millimeters along u and v.
    pub fn set_texture_offset(&mut self, faces: Vec<EntityId>, millimeters: [i32; 2]) -> bool {
        self.position_faces(
            faces,
            Command::PositionTexture {
                ids: Vec::new(),
                offset: Some(millimeters.map(|mm| f64::from(mm) / 1000.0)),
                rotation_deg: None,
                scale: None,
                reset: false,
            },
        )
    }

    /// Sets the texture rotation of `faces`, in tenths of a degree.
    pub fn set_texture_rotation(&mut self, faces: Vec<EntityId>, tenths: i32) -> bool {
        self.position_faces(
            faces,
            Command::PositionTexture {
                ids: Vec::new(),
                offset: None,
                rotation_deg: Some(f64::from(tenths) / 10.0),
                scale: None,
                reset: false,
            },
        )
    }

    /// Sets the texture scale of `faces`, in thousandths.
    pub fn set_texture_scale(&mut self, faces: Vec<EntityId>, thousandths: [u32; 2]) -> bool {
        self.position_faces(
            faces,
            Command::PositionTexture {
                ids: Vec::new(),
                offset: None,
                rotation_deg: None,
                scale: Some(thousandths.map(|t| f64::from(t) / 1000.0)),
                reset: false,
            },
        )
    }

    /// Clears the texture placement of the selected faces.
    pub fn reset_texture_position(&mut self) -> bool {
        let faces = self.doc.selection().iter().copied().collect();
        self.position_faces(
            faces,
            Command::PositionTexture {
                ids: Vec::new(),
                offset: None,
                rotation_deg: None,
                scale: None,
                reset: true,
            },
        )
    }

    /// Turns the texture of each selected face a further quarter counter-clockwise, as one
    /// undo step (faces turned alike share a command in one atomic batch).
    pub fn rotate_texture_90(&mut self) -> bool {
        let m = self.doc.model();
        let mut by_rotation: BTreeMap<u64, (f64, Vec<EntityId>)> = BTreeMap::new();
        for id in self.doc.selection() {
            let EntityId::Face(f) = id else { continue };
            let Some(face) = m.face(*f) else { continue };
            let r = face.texture.unwrap_or_default().rotation_deg + 90.0;
            by_rotation
                .entry(r.to_bits())
                .or_insert_with(|| (r, Vec::new()))
                .1
                .push(*id);
        }
        if by_rotation.is_empty() {
            return false;
        }
        let commands = by_rotation
            .into_values()
            .map(|(r, ids)| Command::PositionTexture {
                ids,
                offset: None,
                rotation_deg: Some(r),
                scale: None,
                reset: false,
            })
            .collect();
        self.message = None;
        if let Err(e) = self.execute_batch(commands, true) {
            self.message = Some(e.error.message);
        }
        true
    }

    /// Runs a `position_texture` command on the faces of `ids` (its `ids` filled in). False
    /// when there are none.
    fn position_faces(&mut self, ids: Vec<EntityId>, mut command: Command) -> bool {
        let faces: Vec<EntityId> = ids
            .into_iter()
            .filter(|id| matches!(id, EntityId::Face(_)))
            .collect();
        if faces.is_empty() {
            return false;
        }
        if let Command::PositionTexture { ids, .. } = &mut command {
            *ids = faces;
        }
        self.message = None;
        self.reset_tool();
        self.run(command);
        self.invalidate_hover();
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use session::SelectMode;

    use super::*;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// Two 1 m squares on the ground; returns their faces.
    fn two_squares(e: &mut Editor) -> Vec<EntityId> {
        for x in [0.0, 2.0] {
            e.execute(Command::AddRectangle {
                corner: [x, 0.0, 0.0],
                opposite: [x + 1.0, 1.0, 0.0],
                normal: None,
            })
            .unwrap();
        }
        e.doc()
            .model()
            .faces()
            .map(|(id, _)| EntityId::Face(id))
            .collect()
    }

    fn select(e: &mut Editor, ids: Vec<EntityId>) {
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
        })
        .unwrap();
    }

    fn placement(e: &Editor, face: EntityId) -> Option<TexturePlacement> {
        let EntityId::Face(f) = face else {
            unreachable!()
        };
        e.doc().model().face(f).unwrap().texture
    }

    #[test]
    fn the_fields_show_the_first_face_and_set_every_selected_face() {
        let mut e = editor();
        let faces = two_squares(&mut e);
        assert_eq!(e.selection_texture_position(), None, "nothing selected");
        select(&mut e, faces.clone());
        assert_eq!(
            e.selection_texture_position(),
            Some(TexturePlacement::default())
        );
        assert!(e.set_texture_offset(faces.clone(), [250, -500]));
        assert!(e.set_texture_rotation(faces.clone(), 455));
        assert!(e.set_texture_scale(faces.clone(), [2000, 500]));
        let expected = TexturePlacement {
            offset: [0.25, -0.5],
            rotation_deg: 45.5,
            scale: [2.0, 0.5],
        };
        for f in &faces {
            assert_eq!(placement(&e, *f), Some(expected));
        }
        assert_eq!(e.selection_texture_position(), Some(expected));
        // A bad value says why in the status and changes nothing.
        assert!(e.set_texture_scale(faces.clone(), [0, 1000]));
        assert!(e.status_parts().message.unwrap().contains("scale"));
        assert_eq!(placement(&e, faces[0]), Some(expected));
        assert!(e.reset_texture_position());
        assert!(faces.iter().all(|f| placement(&e, *f).is_none()));
        e.undo();
        assert_eq!(placement(&e, faces[1]), Some(expected));

        // Not faces only: no fields, and the setters do nothing.
        let edge = e
            .doc()
            .model()
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .next()
            .unwrap();
        select(&mut e, vec![faces[0], edge]);
        assert_eq!(e.selection_texture_position(), None);
        select(&mut e, vec![edge]);
        assert!(!e.reset_texture_position());
        assert!(!e.set_texture_rotation(vec![edge], 10));

        // The faces given are set, whatever is selected now.
        select(&mut e, vec![faces[1]]);
        assert!(e.set_texture_rotation(vec![faces[0]], 100));
        assert_eq!(placement(&e, faces[0]).unwrap().rotation_deg, 10.0);
        assert_eq!(placement(&e, faces[1]), Some(expected));
    }

    fn undo_depth(e: &Editor) -> usize {
        match e.doc().query(&session::Query::History {}).unwrap() {
            session::QueryResult::History { undo_depth, .. } => undo_depth,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rotate_90_turns_each_face_a_further_quarter_in_one_undo_step() {
        let mut e = editor();
        let faces = two_squares(&mut e);
        select(&mut e, vec![faces[0]]);
        e.set_texture_rotation(vec![faces[0]], 300);
        select(&mut e, faces.clone());
        let depth = undo_depth(&e);
        assert!(e.rotate_texture_90());
        assert_eq!(undo_depth(&e), depth + 1);
        assert_eq!(placement(&e, faces[0]).unwrap().rotation_deg, 120.0);
        assert_eq!(placement(&e, faces[1]).unwrap().rotation_deg, 90.0);
        e.undo();
        assert_eq!(placement(&e, faces[0]).unwrap().rotation_deg, 30.0);
        assert_eq!(placement(&e, faces[1]), None);
        // Past a full turn it comes back round.
        for _ in 0..4 {
            e.rotate_texture_90();
        }
        assert_eq!(placement(&e, faces[0]).unwrap().rotation_deg, 30.0);
        assert_eq!(placement(&e, faces[1]), None);
    }
}
