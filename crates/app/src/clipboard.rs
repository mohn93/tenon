//! The clipboard Cut, Copy and Paste use: the system's in the window (so clips pass between
//! app processes), one in memory for tests and headless runs.

/// Text on a clipboard.
pub trait Clipboard {
    /// The text on the clipboard (None: it holds no text); Err says why it could not be read.
    fn get_text(&mut self) -> Result<Option<String>, String>;
    /// Puts `text` on the clipboard; Err says why it could not.
    fn set_text(&mut self, text: String) -> Result<(), String>;
}

/// A clipboard of this process only.
#[derive(Debug, Default)]
pub struct MemoryClipboard {
    text: Option<String>,
}

impl Clipboard for MemoryClipboard {
    fn get_text(&mut self) -> Result<Option<String>, String> {
        Ok(self.text.clone())
    }

    fn set_text(&mut self, text: String) -> Result<(), String> {
        self.text = Some(text);
        Ok(())
    }
}

/// The system clipboard, through arboard. It is opened at first use and kept open (on X11 the
/// text copied is served while it is); a failure to open it (no display) is retried next time.
#[derive(Default)]
pub struct SystemClipboard {
    board: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    fn board(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.board.is_none() {
            self.board = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        Ok(self.board.as_mut().expect("opened above"))
    }
}

impl Clipboard for SystemClipboard {
    fn get_text(&mut self) -> Result<Option<String>, String> {
        match self.board()?.get_text() {
            Ok(text) => Ok(Some(text)),
            // Empty, or holding something other than text (an image): no text.
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set_text(&mut self, text: String) -> Result<(), String> {
        self.board()?.set_text(text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_clipboard_keeps_the_last_text() {
        let mut c = MemoryClipboard::default();
        assert_eq!(c.get_text(), Ok(None));
        c.set_text("a".into()).unwrap();
        c.set_text("b".into()).unwrap();
        assert_eq!(c.get_text(), Ok(Some("b".into())));
    }
}
