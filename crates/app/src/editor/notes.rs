//! Notes' text: the Text tool asks for a new note's text, and a double-click with Select for
//! a note's new text; the window's note sheet answers. Also the model's display units, which
//! labels are written in (Settings' Model tab).

use glam::DVec3;
use kernel::{EntityId, Note};
use session::{Command, Units, format_length};
use tools::{NoteRequest, ToolEvent};

use super::Editor;

/// What the note sheet starts with for a request.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteText {
    /// The text in the field: a note's own text, or none yet.
    pub text: String,
    /// For a leader note, the automatic text it gets when the field is left blank (the
    /// point's coordinates); None when the note needs text (a note fixed to the view).
    pub automatic: Option<String>,
}

impl Editor {
    /// Takes the note whose text the window should ask for, if a tool asked since.
    pub fn take_note_request(&mut self) -> Option<NoteRequest> {
        self.note.take()
    }

    /// Whether the tool waits for the text of a note it is placing.
    pub fn awaits_note_text(&self) -> bool {
        self.tool.awaits_text()
    }

    /// What the note sheet for `request` starts with; None when the note to change is gone.
    pub fn note_text(&self, request: NoteRequest) -> Option<NoteText> {
        match request {
            NoteRequest::Leader { point, .. } => Some(NoteText {
                text: String::new(),
                automatic: Some(self.coordinates(point)),
            }),
            NoteRequest::Screen { .. } => Some(NoteText {
                text: String::new(),
                automatic: None,
            }),
            NoteRequest::Edit(EntityId::Note(n)) => {
                let model = self.doc.model();
                let note = model.note(n)?;
                let automatic = match note {
                    Note::Leader { .. } => Some(self.coordinates(model.note_point(n)?)),
                    Note::Screen { .. } => None,
                };
                Some(NoteText {
                    text: note.text().unwrap_or_default().to_string(),
                    automatic,
                })
            }
            NoteRequest::Edit(_) => None,
        }
    }

    /// `point` of the open context written as a leader note's automatic text: in root
    /// coordinates and the display units.
    fn coordinates(&self, point: DVec3) -> String {
        let p = self.doc.context_transform().transform_point3(point);
        let units = self.doc.units();
        let [x, y, z] = p.to_array().map(|c| format_length(c, &units));
        format!("({x}, {y}, {z})")
    }

    /// The note sheet's answer to `request`: the Text tool adds its note with `text`, or the
    /// note's text changes to it. Blank text gives a leader note its automatic text. True if
    /// the model changed.
    pub fn finish_note(&mut self, request: NoteRequest, text: String) -> bool {
        let before = self.doc.revision();
        match request {
            NoteRequest::Edit(note) => {
                let text = (!text.trim().is_empty()).then_some(text);
                self.run(Command::ChangeNote { note, text });
            }
            NoteRequest::Leader { .. } | NoteRequest::Screen { .. } => {
                if self.awaits_note_text() {
                    self.send(ToolEvent::Value(text));
                }
            }
        }
        self.doc.revision() != before
    }

    /// The note sheet was closed without an answer: the Text tool drops the note it was
    /// placing.
    pub fn cancel_note(&mut self) {
        if self.awaits_note_text() {
            self.send(ToolEvent::Cancel);
        }
    }

    /// Sets the model's display units, as one undo step. False if they already are these.
    pub fn set_units(&mut self, units: Units) -> bool {
        if self.doc.units() == units {
            return false;
        }
        self.run(Command::SetUnits {
            length: Some(units.length),
            precision: Some(units.precision),
            length_snap: Some(units.length_snap),
            angle_precision: Some(units.angle_precision),
            angle_snap: Some(units.angle_snap),
        });
        self.doc.units() == units
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;
    use session::LengthUnit;

    #[test]
    fn units_are_set_as_an_undo_step() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let inches = Units {
            length: LengthUnit::Inches,
            precision: 2,
            ..Units::default()
        };
        assert!(e.set_units(inches));
        assert!(!e.set_units(inches), "already so");
        assert_eq!(e.doc().units(), inches);
        assert_eq!(e.doc().undo_label(), Some("set_units"));
        e.undo();
        assert_eq!(e.doc().units(), Units::default());
    }

    #[test]
    fn snaps_below_their_minimums_are_refused_with_the_commands_message() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        for (units, text) in [
            (
                Units {
                    length_snap: 1e-7,
                    ..Units::default()
                },
                "length_snap must be 0",
            ),
            (
                Units {
                    angle_snap: 0.001,
                    ..Units::default()
                },
                "angle_snap must be 0",
            ),
        ] {
            assert!(!e.set_units(units));
            assert_eq!(e.doc().units(), Units::default());
            let status = e.status();
            assert!(status.contains(text), "{status}");
        }
    }
}
