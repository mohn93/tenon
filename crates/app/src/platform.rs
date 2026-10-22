//! What the window asks of the system around it: files to open and save, preference text, the
//! unsaved-changes question, the clipboard and links. The native implementation (`native`)
//! uses dialogs, files and the system clipboard as the window always did; the web build
//! brings its own.
//!
//! A request that the system can answer at once returns [`Answer::Ready`] and never calls its
//! `reply`. One that waits for the user returns [`Answer::Pending`], and `reply` is called
//! once with the answer, from any thread, when it comes. The window holds the work that
//! depends on the answer until then.

use std::path::PathBuf;

use crate::clipboard::Clipboard;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::NativePlatform;

/// How a request is answered.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer<T> {
    /// Here is the answer; `reply` will not be called.
    Ready(T),
    /// The answer comes later, through `reply`.
    Pending,
}

/// What a save came to: the name the first file was kept under (None: the user cancelled), or
/// why it could not be kept.
pub type Saved = Result<Option<String>, String>;

/// Where a pending answer goes. It may be called from another thread.
pub type Reply<T> = Box<dyn FnOnce(T) + Send>;

/// The parts of the app a system may not offer. The window hides what is off (menu items, the
/// Library tab, the autosave setting) and never starts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    /// Models and files have paths the app opens, saves and remembers; off: files come and go
    /// as bytes (`Source::Bytes`, `Platform::save_files`).
    pub file_paths: bool,
    /// The component library folder (the Library tab, Save to Library).
    pub library: bool,
    /// Autosave and recovered models.
    pub autosave: bool,
    /// Print and Export PDF.
    pub print: bool,
    /// Open Recent and the start card's list.
    pub recent: bool,
    /// The window draws its own dialogs (the unsaved-changes question, About) where the system
    /// has no native ones to show; `Platform::confirm_unsaved` is not called then.
    pub in_app_dialogs: bool,
    /// The app can be quit (File > Quit, Ctrl+Q); a browser tab is closed by the page's user.
    pub quit: bool,
    /// Paste arrives as an event from the system (`AppEvent::Paste`) after the key press,
    /// rather than the key press reading the clipboard.
    pub paste_by_event: bool,
}

impl Features {
    /// Everything on: a desktop.
    pub const ALL: Features = Features {
        file_paths: true,
        library: true,
        autosave: true,
        print: true,
        recent: true,
        in_app_dialogs: false,
        quit: true,
        paste_by_event: false,
    };

    /// Only what a browser tab offers: files as bytes, nothing that needs folders or processes.
    pub const WEB: Features = Features {
        file_paths: false,
        library: false,
        autosave: false,
        print: false,
        recent: false,
        in_app_dialogs: true,
        quit: false,
        paste_by_event: true,
    };
}

/// A kind of file a dialog offers: its name and extensions, lowercase, without the dot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: &'static str,
    pub extensions: Vec<&'static str>,
}

impl Filter {
    pub fn new(name: &'static str, extensions: &[&'static str]) -> Self {
        Self {
            name,
            extensions: extensions.to_vec(),
        }
    }
}

/// Asks for a file to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    /// The kinds of file offered, the first one chosen at first.
    pub filters: Vec<Filter>,
}

/// Asks where to save a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveRequest {
    pub filters: Vec<Filter>,
    /// The file name suggested.
    pub name: String,
    /// Ask where to save even if the system could reuse the file's last place (Save As, Export).
    pub ask: bool,
}

/// What a picked file is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A file on disk, read by the app.
    Path(PathBuf),
    /// The file's bytes, read by the system.
    Bytes(Vec<u8>),
}

/// A file the user picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picked {
    /// The file's name, with its extension.
    pub name: String,
    pub source: Source,
}

/// One file of a save: the first of `Platform::save_files` is the one the user named; the rest
/// (an OBJ's `.mtl` and textures) go beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The unsaved-changes question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsavedPrompt {
    /// `Do you want to save changes to "house.skr"?`
    pub question: String,
    /// What happens if the answer is not Save.
    pub detail: &'static str,
}

/// The answer to the unsaved-changes question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsavedChoice {
    Save,
    DontSave,
    Cancel,
}

/// The system the window runs on.
pub trait Platform {
    /// What this system offers.
    fn features(&self) -> Features;

    /// Asks for a file to open; None: the user cancelled.
    fn pick_open(
        &self,
        request: &OpenRequest,
        reply: Reply<Option<Picked>>,
    ) -> Answer<Option<Picked>>;

    /// Asks for a file path to save to (only where `features().file_paths`); None: cancelled.
    fn pick_save_path(
        &self,
        request: &SaveRequest,
        reply: Reply<Option<PathBuf>>,
    ) -> Answer<Option<PathBuf>>;

    /// Asks for a folder (only where `features().library`); None: cancelled.
    fn pick_folder(&self, reply: Reply<Option<PathBuf>>) -> Answer<Option<PathBuf>>;

    /// Saves `files` under the name the user picks (a download in a browser); the answer is
    /// the name of the first file as saved, None: cancelled.
    fn save_files(
        &self,
        request: &SaveRequest,
        files: Vec<Download>,
        reply: Reply<Saved>,
    ) -> Answer<Saved>;

    /// The stored preferences as text; None: nothing is stored (or it cannot be read).
    fn read_prefs(&self) -> Option<String>;

    /// Stores the preferences. Err says why they could not be kept; the change lasts the
    /// session.
    fn write_prefs(&self, text: &str) -> Result<(), String>;

    /// Asks whether to save unsaved changes. `parent` is the window the question is about.
    fn confirm_unsaved(
        &self,
        parent: Option<&winit::window::Window>,
        prompt: &UnsavedPrompt,
        reply: Reply<UnsavedChoice>,
    ) -> Answer<UnsavedChoice>;

    /// The clipboard Cut, Copy and Paste use.
    fn clipboard(&self) -> Box<dyn Clipboard>;

    /// Whether the open model has unsaved changes, told whenever a frame is drawn; a browser
    /// asks before the tab closes while it does.
    fn set_modified(&self, _modified: bool) {}

    /// Opens a link or a file in the system's own program or tab.
    fn open_url(&self, url: &str) -> Result<(), String>;
}
