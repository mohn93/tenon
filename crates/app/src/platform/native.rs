//! The desktop's `Platform`: native dialogs (rfd), preferences in a file, the system clipboard
//! (arboard) and links opened by the system's own program.

use std::path::{Path, PathBuf};

use super::{
    Answer, Download, Features, Filter, OpenRequest, Picked, Platform, Reply, SaveRequest, Saved,
    Source, UnsavedChoice, UnsavedPrompt,
};
use crate::clipboard::{Clipboard, SystemClipboard};
use crate::prefs::{self, PrefsFile};

/// Dialogs and files, as the window always used them.
pub struct NativePlatform {
    /// Where the preferences are kept (None: they last the session).
    prefs: Option<PrefsFile>,
}

impl NativePlatform {
    pub fn new(prefs: Option<PrefsFile>) -> Self {
        Self { prefs }
    }
}

fn dialog(filters: &[Filter]) -> rfd::FileDialog {
    filters.iter().fold(rfd::FileDialog::new(), |d, f| {
        d.add_filter(f.name, &f.extensions)
    })
}

fn picked(path: PathBuf) -> Picked {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Picked {
        name,
        source: Source::Path(path),
    }
}

/// What the unsaved-changes alert's answer asks for. Yes and No come from a backend that
/// ignores custom labels; anything else (Escape, a dismissed alert) cancels.
fn unsaved_choice(answer: &rfd::MessageDialogResult) -> UnsavedChoice {
    use rfd::MessageDialogResult as R;
    match answer {
        R::Yes => UnsavedChoice::Save,
        R::No => UnsavedChoice::DontSave,
        R::Custom(label) if label == "Save" => UnsavedChoice::Save,
        R::Custom(label) if label == "Don't Save" => UnsavedChoice::DontSave,
        _ => UnsavedChoice::Cancel,
    }
}

impl Platform for NativePlatform {
    fn features(&self) -> Features {
        Features::ALL
    }

    fn pick_open(
        &self,
        request: &OpenRequest,
        _reply: Reply<Option<Picked>>,
    ) -> Answer<Option<Picked>> {
        Answer::Ready(dialog(&request.filters).pick_file().map(picked))
    }

    fn pick_save_path(
        &self,
        request: &SaveRequest,
        _reply: Reply<Option<PathBuf>>,
    ) -> Answer<Option<PathBuf>> {
        Answer::Ready(
            dialog(&request.filters)
                .set_file_name(request.name.clone())
                .save_file(),
        )
    }

    fn pick_folder(&self, _reply: Reply<Option<PathBuf>>) -> Answer<Option<PathBuf>> {
        Answer::Ready(rfd::FileDialog::new().pick_folder())
    }

    fn save_files(
        &self,
        request: &SaveRequest,
        files: Vec<Download>,
        _reply: Reply<Saved>,
    ) -> Answer<Saved> {
        let Some(path) = dialog(&request.filters)
            .set_file_name(request.name.clone())
            .save_file()
        else {
            return Answer::Ready(Ok(None));
        };
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let saved_as = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let mut chosen = Some(path);
        for file in files {
            // The first file is the one named; the others go beside it.
            let target = chosen.take().unwrap_or_else(|| dir.join(&file.name));
            if let Err(e) = std::fs::write(&target, &file.bytes) {
                return Answer::Ready(Err(format!("{}: {e}", target.display())));
            }
        }
        Answer::Ready(Ok(saved_as))
    }

    fn read_prefs(&self) -> Option<String> {
        prefs::read_text(self.prefs.as_ref()?)
    }

    fn write_prefs(&self, text: &str) -> Result<(), String> {
        let Some(file) = &self.prefs else {
            return Ok(());
        };
        prefs::save_text(&file.path, text).map_err(|e| format!("{}: {e}", file.path.display()))
    }

    fn confirm_unsaved(
        &self,
        parent: Option<&winit::window::Window>,
        prompt: &UnsavedPrompt,
        _reply: Reply<UnsavedChoice>,
    ) -> Answer<UnsavedChoice> {
        // With a parent, macOS shows an NSAlert sheet: Escape cancels, Cmd+D is Don't Save.
        let mut dialog = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(prompt.question.clone())
            .set_description(prompt.detail)
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                "Save".into(),
                "Don't Save".into(),
                "Cancel".into(),
            ));
        if let Some(window) = parent {
            dialog = dialog.set_parent(window);
        }
        let answer = dialog.show();
        Answer::Ready(unsaved_choice(&answer))
    }

    fn clipboard(&self) -> Box<dyn Clipboard> {
        Box::new(SystemClipboard::default())
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        crate::editor::open_with_viewer(Path::new(url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsaved_answers_map_to_save_dont_save_and_cancel() {
        use rfd::MessageDialogResult as R;
        let custom = |s: &str| R::Custom(s.into());
        for answer in [custom("Save"), R::Yes] {
            assert_eq!(unsaved_choice(&answer), UnsavedChoice::Save, "{answer}");
        }
        for answer in [custom("Don't Save"), R::No] {
            assert_eq!(unsaved_choice(&answer), UnsavedChoice::DontSave, "{answer}");
        }
        for answer in [custom("Cancel"), R::Cancel, R::Ok, custom("other")] {
            assert_eq!(unsaved_choice(&answer), UnsavedChoice::Cancel, "{answer}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn preferences_are_kept_in_the_file_and_without_one_not_at_all() {
        let dir =
            std::env::temp_dir().join(format!("sketchup-alt-platform-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".config")).unwrap();
        let file = PrefsFile {
            path: dir.join(".config/sketchup-alt/prefs.json"),
            legacy: None,
        };
        let platform = NativePlatform::new(Some(file));
        assert_eq!(platform.read_prefs(), None, "no file yet");
        platform.write_prefs("{\"theme\": \"dark\"}").unwrap();
        assert_eq!(
            platform.read_prefs().as_deref(),
            Some("{\"theme\": \"dark\"}")
        );
        let none = NativePlatform::new(None);
        assert_eq!(none.read_prefs(), None);
        assert_eq!(none.write_prefs("{}"), Ok(()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
