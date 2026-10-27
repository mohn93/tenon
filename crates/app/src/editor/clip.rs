//! Cut, Copy, Paste and Paste In Place: clips of the selection on the clipboard, and the Paste
//! tool that places one under the cursor.

use kernel::EntityId;
use session::{Command, CommandError, ErrorKind, clip};
use tools::{PasteTool, TextSpec};

use super::{Editor, Message};
use crate::clipboard::Clipboard;

/// What Paste says of a clipboard that holds no clip.
pub const NOT_A_CLIP: &str = "The clipboard does not hold tenon geometry";

impl Editor {
    /// Makes Cut, Copy and Paste use `clipboard` (the window gives the system's).
    pub fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        self.clipboard = clipboard;
        self.refresh_clipboard();
    }

    /// Looks whether the clipboard holds a clip now (the window asks when it gains focus,
    /// since another app may have changed it). Reads the clipboard, so not every frame.
    pub fn refresh_clipboard(&mut self) {
        self.clip_ready = self
            .clipboard
            .get_text()
            .is_ok_and(|text| text.is_some_and(|t| t.starts_with(clip::HEADER)));
    }

    /// Whether the clipboard held a clip when last looked at: Paste and Paste In Place are
    /// enabled then.
    pub fn clip_ready(&self) -> bool {
        self.clip_ready
    }

    /// Whether the Paste tool is out.
    pub fn pasting(&self) -> bool {
        self.paste_return.is_some()
    }

    /// Copies the selection to the clipboard as a clip (Cmd+C), putting the Paste tool away
    /// (it holds the clip before). With nothing selected, or when the clipboard cannot be
    /// written, the status says why and nothing changes.
    pub fn copy(&mut self) -> Result<(), CommandError> {
        let ids = self.selected_for("copy")?;
        self.put_clip(&ids)
    }

    /// Copies the selection to the clipboard, then erases it as one undo step labelled `cut`
    /// (Cmd+X), putting the Paste tool away. Nothing is erased when the copy fails.
    pub fn cut(&mut self) -> Result<(), CommandError> {
        let ids = self.selected_for("cut")?;
        self.put_clip(&ids)?;
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let result = self.execute_recorded_as(Command::Erase { ids }, "cut");
        self.invalidate_hover();
        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                self.message = Some(Message::error(e.message.clone()));
                Err(e)
            }
        }
    }

    /// Pastes the clip on the clipboard (Cmd+V). With `at` (a point of the open context) its
    /// base point lands there at once; without it the Paste tool comes out and the clip follows
    /// the cursor until a click places it (Escape goes back to the tool used before). Text that
    /// is not a clip is refused with `NOT_A_CLIP` in the status; a damaged clip with why.
    pub fn paste(&mut self, at: Option<[f64; 3]>) -> Result<(), CommandError> {
        let text = self.clip_text()?;
        if at.is_some() {
            return self.paste_now(text, at);
        }
        let outline = match clip::outline(&text) {
            Ok(outline) => outline,
            Err(e) => return Err(self.refuse(e)),
        };
        self.start_paste_tool(PasteTool::new(text, &outline));
        Ok(())
    }

    /// Brings out the Paste tool placing 3D text (Tools > 3D Text..., after its sheet): the
    /// letters' outline follows the cursor, the first line's baseline start on the point under
    /// it, until a click draws the text there as one `add_3d_text` step (Escape goes back to
    /// the tool used before). Text with no letters the font has is refused in the status.
    pub fn place_3d_text(&mut self, spec: TextSpec) -> Result<(), CommandError> {
        let shape = session::text3d::shape(&spec.text, spec.height);
        if shape.faces.is_empty() {
            return Err(self.refuse(CommandError::new(
                ErrorKind::InvalidInput,
                "The text has no letters to draw",
            )));
        }
        self.start_paste_tool(PasteTool::text(spec, shape.segments()));
        Ok(())
    }

    /// Pastes the clip on the clipboard where it was copied from, in world coordinates (Edit >
    /// Paste In Place).
    pub fn paste_in_place(&mut self) -> Result<(), CommandError> {
        let text = self.clip_text()?;
        self.paste_now(text, None)
    }

    /// Puts the Paste tool away, back to the tool used before it.
    pub(super) fn end_paste(&mut self) {
        if let Some(back) = self.paste_return.take() {
            // Keep what the paste said (a refusal) and let the button's release go nowhere.
            let message = self.message.take();
            self.select_tool(back);
            self.message = message;
            self.tool_pressed = false;
        }
    }

    /// The selection's ids for `verb`; with nothing selected the status says so.
    fn selected_for(&mut self, verb: &str) -> Result<Vec<EntityId>, CommandError> {
        if self.doc.selection().is_empty() {
            self.message = Some(Message::error(format!("Select something to {verb}")));
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                format!("select something to {verb} first"),
            ));
        }
        Ok(self.doc.selection().iter().copied().collect())
    }

    /// Puts the clip of `ids` on the clipboard. The Paste tool, if out, goes: its clip is no
    /// longer the clipboard's.
    fn put_clip(&mut self, ids: &[EntityId]) -> Result<(), CommandError> {
        let text = match self.doc.clip(ids) {
            Ok(text) => text,
            Err(e) => return Err(self.refuse(e)),
        };
        if let Err(why) = self.clipboard.set_text(text) {
            return Err(self.refuse(CommandError::new(
                ErrorKind::File,
                format!("Could not copy to the clipboard: {why}"),
            )));
        }
        self.clip_ready = true;
        self.end_paste();
        Ok(())
    }

    /// The clipboard's text when it is a clip (by its header; pasting checks the rest).
    fn clip_text(&mut self) -> Result<String, CommandError> {
        match self.clipboard.get_text() {
            Ok(Some(text)) if text.starts_with(clip::HEADER) => {
                self.clip_ready = true;
                Ok(text)
            }
            Err(why) => {
                self.clip_ready = false;
                Err(self.refuse(CommandError::new(
                    ErrorKind::File,
                    format!("Could not read the clipboard: {why}"),
                )))
            }
            Ok(_) => {
                self.clip_ready = false;
                Err(self.refuse(CommandError::new(ErrorKind::InvalidInput, NOT_A_CLIP)))
            }
        }
    }

    /// Brings out `tool` (a Paste tool) over the current one, which Escape or the placing
    /// click goes back to.
    pub(super) fn start_paste_tool(&mut self, tool: impl tools::Tool + 'static) {
        // Read before `hover_now`, which shows the hover again.
        let in_view = !self.flying && !self.hover_hidden && !self.covered;
        let back = self.paste_return.unwrap_or(self.kind);
        self.select_tool(back);
        self.paste_return = Some(back);
        self.tool = Box::new(tool);
        self.sync_operation();
        if in_view {
            self.hover_now();
        }
    }

    /// Pastes `text` as one `paste` step and ends the Paste tool if it was out.
    fn paste_now(&mut self, text: String, at: Option<[f64; 3]>) -> Result<(), CommandError> {
        self.end_paste();
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let result = self.execute_recorded_as(Command::Paste { clip: text, at }, "paste");
        self.invalidate_hover();
        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(self.refuse(e)),
        }
    }

    /// Shows `e` in the status and gives it back.
    pub(super) fn refuse(&mut self, e: CommandError) -> CommandError {
        self.message = Some(Message::error(e.message.clone()));
        e
    }
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};
    use session::{Command, SelectMode};
    use tools::{Phase, ToolKind};

    use super::*;
    use crate::editor::EditorKey;
    use crate::navigation::Button;

    /// An editor with a 2 m square at the origin, all of it selected.
    fn square() -> Editor {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        e.execute(Command::AddRectangle {
            corner: [0.0; 3],
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
        e
    }

    fn faces(e: &Editor) -> usize {
        e.doc().model().faces().count()
    }

    fn point_at(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.refresh_hover();
    }

    #[test]
    fn the_paste_tool_follows_the_cursor_and_a_click_pastes_once() {
        let mut e = square();
        e.copy().unwrap();
        e.select_tool(ToolKind::Rectangle);
        point_at(&mut e, DVec3::new(5.0, 0.0, 0.0));
        e.paste(None).unwrap();
        assert!(e.pasting());
        assert_eq!(e.status_parts().tool, "Paste");
        assert_eq!(e.tool_kind(), ToolKind::Rectangle, "the tool to go back to");
        assert_eq!(e.phase(), Phase::Active { points: vec![] });
        e.refresh_hover();
        assert!(!e.hud(false).lines.is_empty(), "the clip's outline shows");
        // A press pastes; its release goes to no tool.
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        assert_eq!(faces(&e), 2);
        assert_eq!(e.doc().undo_label(), Some("paste"));
        assert!(!e.pasting());
        assert_eq!(e.status_parts().tool, "Rectangle");
        assert_eq!(e.phase(), Phase::Idle, "the release started no rectangle");
    }

    fn spec(text: &str) -> TextSpec {
        TextSpec {
            text: text.into(),
            height: 0.5,
            depth: 0.1,
            filled: true,
        }
    }

    #[test]
    fn text_3d_follows_the_cursor_and_a_click_draws_it_as_one_selected_group() {
        let mut e = square();
        e.select_tool(ToolKind::Line);
        point_at(&mut e, DVec3::new(5.0, 0.0, 0.0));
        e.place_3d_text(spec("Hi o")).unwrap();
        assert!(e.pasting());
        assert_eq!(e.status_parts().tool, "3D Text");
        assert_eq!(e.tool_kind(), ToolKind::Line, "the tool to go back to");
        e.refresh_hover();
        assert!(!e.hud(false).lines.is_empty(), "the letters' outline shows");
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        assert_eq!(e.doc().undo_label(), Some("add_3d_text"));
        assert!(!e.pasting());
        assert_eq!(e.status_parts().tool, "Line");
        let selected: Vec<_> = e.doc().selection().iter().copied().collect();
        let [kernel::EntityId::Instance(group)] = selected[..] else {
            panic!("the new group is selected: {selected:?}")
        };
        let placed = e.doc().model().instance(group).unwrap();
        assert!(
            placed
                .transform
                .translation
                .distance(DVec3::new(5.0, 0.0, 0.0))
                < 1e-6
        );
        e.undo();
        assert!(e.doc().model().instances().next().is_none(), "one step");
        assert_eq!(faces(&e), 1);
    }

    #[test]
    fn text_3d_without_letters_is_refused_and_escape_cancels_it() {
        let mut e = square();
        assert!(e.place_3d_text(spec("\u{4E2D}")).is_err());
        assert!(!e.pasting());
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("The text has no letters to draw")
        );
        e.place_3d_text(spec("A")).unwrap();
        e.key(EditorKey::Escape);
        assert!(!e.pasting());
        assert_eq!(e.status_parts().tool, "Select");
        assert!(e.doc().model().instances().next().is_none());
    }

    #[test]
    fn pasting_from_select_shows_the_snap_marker_and_label() {
        let mut e = square();
        e.copy().unwrap();
        assert_eq!(e.tool_kind(), ToolKind::Select);
        point_at(&mut e, DVec3::new(2.0, 2.0, 0.0));
        assert!(e.hud(false).marker.is_none(), "Select shows no marker");
        assert_eq!(e.snap_label(), None);
        e.paste(None).unwrap();
        e.refresh_hover();
        assert!(
            e.hud(false).marker.is_some(),
            "the Paste tool shows the snap"
        );
        assert_eq!(e.snap_label().as_deref(), Some("Endpoint"));
    }

    #[test]
    fn a_paste_that_changes_nothing_still_puts_the_paste_tool_away() {
        let mut e = square();
        e.copy().unwrap();
        point_at(&mut e, DVec3::ZERO);
        e.paste(None).unwrap();
        let revision = e.doc().revision();
        // The clip lands exactly on the square it was copied from.
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        assert_eq!(e.doc().revision(), revision, "nothing changed");
        assert!(!e.pasting());
        assert_eq!(e.status_parts().tool, "Select");
    }

    #[test]
    fn escape_puts_the_paste_tool_away() {
        let mut e = square();
        e.copy().unwrap();
        point_at(&mut e, DVec3::new(5.0, 0.0, 0.0));
        e.paste(None).unwrap();
        // Undo and redo while it is out keep it: the clip names nothing in the model.
        e.undo();
        e.redo();
        assert!(e.pasting());
        e.key(EditorKey::Escape);
        assert!(!e.pasting());
        assert_eq!(e.status_parts().tool, "Select");
        assert_eq!(faces(&e), 1);
        // A tool chosen while pasting ends it too.
        e.paste(None).unwrap();
        e.select_tool(ToolKind::Line);
        assert!(!e.pasting());
    }

    #[test]
    fn copy_and_cut_need_a_selection_and_cut_is_one_step() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        assert!(e.copy().is_err());
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Select something to copy")
        );
        assert!(e.cut().is_err());
        assert!(!e.clip_ready());
        let mut e = square();
        e.cut().unwrap();
        assert_eq!(faces(&e), 0);
        assert_eq!(e.doc().undo_label(), Some("cut"));
        assert!(e.clip_ready());
        e.undo();
        assert_eq!(faces(&e), 1);
        assert_eq!(e.doc().model().edges().count(), 4);
    }

    /// A clipboard that cannot be written.
    struct Broken;

    impl Clipboard for Broken {
        fn get_text(&mut self) -> Result<Option<String>, String> {
            Err("timed out".into())
        }

        fn set_text(&mut self, _text: String) -> Result<(), String> {
            Err("no display".into())
        }
    }

    #[test]
    fn a_clipboard_that_fails_is_reported_and_cut_erases_nothing() {
        let mut e = square();
        e.set_clipboard(Box::new(Broken));
        let revision = e.doc().revision();
        assert!(e.cut().is_err());
        assert_eq!(
            e.status_parts().message.as_deref(),
            Some("Could not copy to the clipboard: no display")
        );
        assert_eq!(e.doc().revision(), revision);
        // Reading fails too: said as such, not as text that is no clip.
        for in_place in [true, false] {
            let err = if in_place {
                e.paste_in_place()
            } else {
                e.paste(None)
            }
            .unwrap_err();
            assert_eq!(err.kind, ErrorKind::File);
            assert_eq!(
                e.status_parts().message.as_deref(),
                Some("Could not read the clipboard: timed out")
            );
        }
        assert!(!e.pasting());
        assert_eq!(e.doc().revision(), revision);
    }

    #[test]
    fn copying_or_cutting_puts_the_paste_tool_away() {
        let mut e = square();
        e.copy().unwrap();
        e.select_tool(ToolKind::Rectangle);
        point_at(&mut e, DVec3::new(5.0, 0.0, 0.0));
        e.paste(None).unwrap();
        e.copy().unwrap();
        assert!(!e.pasting(), "copy");
        assert_eq!(e.status_parts().tool, "Rectangle");
        e.paste(None).unwrap();
        e.cut().unwrap();
        assert!(!e.pasting(), "cut");
        assert_eq!(e.status_parts().tool, "Rectangle");
        assert_eq!(faces(&e), 0);
    }
}
