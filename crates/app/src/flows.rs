//! The window's file flows: new, open, save, import, export, textures and the unsaved-changes
//! question, as steps over the editor and a `Platform`. A request the platform answers at once
//! runs on; one it answers later is held (`Waiting`) until `Flows::reply` is given the answer.
//! What the window itself must do afterwards (clear its autosave, remember a file, quit) comes
//! back as `Effect`s. No window is needed, so the flows run in tests against a fake platform.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kernel::MaterialId;
use winit::window::Window;

use crate::editor::{Editor, PrintOptions};
use crate::platform::{
    Answer, Download, Filter, OpenKind, OpenRequest, Picked, Platform, Reply, SaveRequest, Saved,
    Source, UnsavedChoice, UnsavedPrompt,
};

/// What happens once unsaved changes are settled (saved, or given up).
#[derive(Clone, Debug, PartialEq)]
pub enum Then {
    NewModel,
    /// Ask for a model to open.
    Open,
    OpenFile(PathBuf),
    /// The window opens the recovered model whose copy is at the path.
    OpenRecovered(PathBuf),
    Quit,
}

/// What a request waiting for the platform will do with its answer.
#[derive(Debug)]
enum Waiting {
    Unsaved(Then),
    SavePath(Option<Then>),
    SaveFiles(Option<Then>),
    PickOpen,
    PickImport,
    PickTexture(MaterialId),
    PickFolder,
    ExportPath,
    ExportFiles,
    PdfPath(PrintOptions),
}

/// A late answer from the platform, sent to the window's event loop.
#[derive(Debug)]
pub enum PlatformReply {
    Unsaved(UnsavedChoice),
    Picked(Option<Picked>),
    Path(Option<PathBuf>),
    Saved(Saved),
}

/// What the window does after a step.
#[derive(Debug, PartialEq)]
pub enum Effect {
    /// The model the autosave copied was saved or replaced.
    ClearAutosave,
    /// The model's file (as of the step: later steps may replace the model) goes first in the
    /// recent files.
    RememberFile(Option<PathBuf>),
    Quit,
    OpenRecovered(PathBuf),
    /// The Import Options sheet opens for this file (a name only when the platform keeps
    /// the file's bytes: see `Flows::import_confirmed`).
    ImportSheet(PathBuf),
    LibraryFolder(PathBuf),
    /// Print the view to this PDF.
    Pdf(PrintOptions, PathBuf),
    /// The window asks which format to export (where files have no path): each format's name
    /// and extension, the one exported last first. It answers with `Flows::export_as`.
    ExportSheet(Vec<(&'static str, &'static str)>),
    /// The window asks the unsaved-changes question itself (`Features::in_app_dialogs`) and
    /// answers with `PlatformReply::Unsaved`.
    AskUnsaved(UnsavedPrompt),
}

/// What a step works on.
pub struct Env<'a> {
    pub editor: &'a mut Editor,
    pub platform: &'a dyn Platform,
    /// The window a dialog belongs to.
    pub parent: Option<&'a Window>,
    /// The name of the model opened from the Recover Models window, while it is the open one.
    pub recovered: Option<&'a str>,
}

/// The flows' state: the request waiting for the platform, and the file a platform without
/// paths last kept.
pub struct Flows {
    waiting: Option<Waiting>,
    /// How a late answer reaches the window.
    notify: Arc<dyn Fn(PlatformReply) + Send + Sync>,
    /// The name of the file the model was opened from or saved to, where files have no path.
    kept_name: Option<String>,
    /// The file picked for the open Import Options sheet, where files have no path.
    import: Option<(String, Vec<u8>)>,
    /// The model was opened from, or last saved to, a file the platform may have kept (where
    /// files have no path): Save may write back to it. Cleared by New and by opening a path.
    bound: bool,
}

/// The effect that remembers the open model's file.
fn remember(env: &Env) -> Effect {
    Effect::RememberFile(env.editor.doc().path().map(Path::to_path_buf))
}

fn skr_filters() -> Vec<Filter> {
    vec![Filter::new("SketchUp Alt model", &["skr"])]
}

/// The File > Open filters: `.skr` models, and SketchUp files (opened as new models).
fn open_filters() -> Vec<Filter> {
    vec![
        Filter::new("SketchUp Alt or SketchUp model", &["skr", "skp"]),
        Filter::new("SketchUp Alt model", &["skr"]),
        Filter::new("SketchUp model", &["skp"]),
    ]
}

fn import_filters() -> Vec<Filter> {
    vec![
        Filter::new("OBJ, STL or SketchUp", &["obj", "stl", "skp"]),
        Filter::new("OBJ", &["obj"]),
        Filter::new("STL", &["stl"]),
        Filter::new("SketchUp", &["skp"]),
    ]
}

/// The name the save dialog suggests: the file's, else the SketchUp file's stem with `.skr`
/// (never the SketchUp file itself), else "Untitled.skr".
/// A recovered model never saved (`recovered`: its name) suggests "<name>.skr".
pub fn save_name(
    path: Option<&Path>,
    imported_from: Option<&Path>,
    recovered: Option<&str>,
) -> String {
    match (path, imported_from.and_then(|p| p.file_stem()), recovered) {
        (None, Some(stem), _) => format!("{}.skr", stem.to_string_lossy()),
        (None, None, Some(name)) => format!("{name}.skr"),
        _ => file_name(path),
    }
}

/// The file's name, or "Untitled.skr" for a model never saved.
pub fn file_name(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name()).map_or_else(
        || "Untitled.skr".into(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// `Do you want to save changes to "house.skr"?`
pub fn unsaved_question(path: Option<&Path>) -> String {
    question_for(&file_name(path))
}

fn question_for(name: &str) -> String {
    format!("Do you want to save changes to \"{name}\"?")
}

/// The `.skr` name of a file kept under `name`: a SketchUp file's stem gets the extension.
fn skr_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("skp") && !stem.is_empty() => {
            format!("{stem}.skr")
        }
        _ => name.to_string(),
    }
}

impl Flows {
    pub fn new(notify: Arc<dyn Fn(PlatformReply) + Send + Sync>) -> Self {
        Self {
            waiting: None,
            notify,
            kept_name: None,
            import: None,
            bound: false,
        }
    }

    /// The name of the file the model was opened from or saved to, where files have no path.
    pub fn kept_name(&self) -> Option<&str> {
        self.kept_name.as_deref()
    }

    fn reply<T: Send + 'static>(&self, wrap: fn(T) -> PlatformReply) -> Reply<T> {
        let notify = self.notify.clone();
        Box::new(move |answer| notify(wrap(answer)))
    }

    /// Before the model is replaced or the window closes: if it has unsaved changes, asks
    /// whether to save them, then does `then` (unless the answer cancels, or a save fails).
    pub fn settle_and(&mut self, env: &mut Env, then: Then) -> Vec<Effect> {
        if self.waiting.is_some() {
            return Vec::new();
        }
        let doc = env.editor.doc();
        if !doc.is_modified() {
            return self.run_then(env, then);
        }
        let question = match (doc.path(), &self.kept_name) {
            (None, Some(name)) => question_for(name),
            (path, _) => unsaved_question(path),
        };
        let prompt = UnsavedPrompt {
            question,
            detail: "Your changes will be lost if you don't save them.",
        };
        if env.platform.features().in_app_dialogs {
            self.waiting = Some(Waiting::Unsaved(then));
            return vec![Effect::AskUnsaved(prompt)];
        }
        let answer =
            env.platform
                .confirm_unsaved(env.parent, &prompt, self.reply(PlatformReply::Unsaved));
        match answer {
            Answer::Ready(choice) => self.after_unsaved(env, choice, then),
            Answer::Pending => {
                self.waiting = Some(Waiting::Unsaved(then));
                Vec::new()
            }
        }
    }

    fn after_unsaved(&mut self, env: &mut Env, choice: UnsavedChoice, then: Then) -> Vec<Effect> {
        match choice {
            UnsavedChoice::Save => self.save(env, false, Some(then)),
            UnsavedChoice::DontSave => self.run_then(env, then),
            UnsavedChoice::Cancel => Vec::new(),
        }
    }

    fn run_then(&mut self, env: &mut Env, then: Then) -> Vec<Effect> {
        match then {
            Then::NewModel => {
                env.editor.new_model();
                self.kept_name = None;
                self.bound = false;
                vec![Effect::ClearAutosave]
            }
            Then::Open => self.pick_open(env),
            Then::OpenFile(path) => self.open_path(env, &path),
            Then::OpenRecovered(model) => vec![Effect::OpenRecovered(model)],
            Then::Quit => vec![Effect::Quit],
        }
    }

    /// Opens the file at `path` (unsaved changes settled); a file that fails to open says why
    /// in the status.
    fn open_path(&mut self, env: &mut Env, path: &Path) -> Vec<Effect> {
        if env.editor.open_file(path) {
            self.kept_name = None;
            self.bound = false;
            vec![Effect::ClearAutosave, remember(env)]
        } else {
            Vec::new()
        }
    }

    fn pick_open(&mut self, env: &mut Env) -> Vec<Effect> {
        let request = OpenRequest {
            filters: open_filters(),
            kind: OpenKind::Model,
        };
        match env
            .platform
            .pick_open(&request, self.reply(PlatformReply::Picked))
        {
            Answer::Ready(picked) => self.opened(env, picked),
            Answer::Pending => {
                self.waiting = Some(Waiting::PickOpen);
                Vec::new()
            }
        }
    }

    fn opened(&mut self, env: &mut Env, picked: Option<Picked>) -> Vec<Effect> {
        match picked {
            None => Vec::new(),
            Some(Picked {
                source: Source::Path(path),
                ..
            }) => self.open_path(env, &path),
            Some(Picked {
                name,
                source: Source::Bytes(bytes),
            }) => {
                if env.editor.open_bytes(&name, &bytes) {
                    env.platform.file_opened(&name);
                    self.bound = true;
                    self.kept_name = Some(skr_name(&name));
                    vec![Effect::ClearAutosave, remember(env)]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// File > Open: settles unsaved changes, asks for a file and opens it.
    pub fn open(&mut self, env: &mut Env) -> Vec<Effect> {
        self.settle_and(env, Then::Open)
    }

    /// The name the save dialog suggests for the open model.
    fn suggested_name(&self, env: &Env) -> String {
        let doc = env.editor.doc();
        match (doc.path(), &self.kept_name) {
            (None, Some(name)) if doc.imported_from().is_none() => name.clone(),
            _ => save_name(doc.path(), doc.imported_from(), env.recovered),
        }
    }

    /// Saves to the current file, or to one the user picks when there is none or `choose` is
    /// set (where files have no path: downloads the bytes, or writes them where the platform
    /// kept the file). Then does `then`, once saved.
    pub fn save(&mut self, env: &mut Env, choose: bool, then: Option<Then>) -> Vec<Effect> {
        if self.waiting.is_some() {
            return Vec::new();
        }
        let name = self.suggested_name(env);
        if env.platform.features().file_paths {
            if !(choose || env.editor.doc().path().is_none()) {
                return self.saved_to(env, None, then);
            }
            let request = SaveRequest {
                filters: skr_filters(),
                name,
                ask: true,
                write_back: false,
            };
            return match env
                .platform
                .pick_save_path(&request, self.reply(PlatformReply::Path))
            {
                Answer::Ready(Some(path)) => self.saved_to(env, Some(path), then),
                Answer::Ready(None) => Vec::new(),
                Answer::Pending => {
                    self.waiting = Some(Waiting::SavePath(then));
                    Vec::new()
                }
            };
        }
        let Some(bytes) = env.editor.encode_model() else {
            return Vec::new();
        };
        let request = SaveRequest {
            filters: skr_filters(),
            name: name.clone(),
            ask: choose,
            write_back: !choose && self.bound,
        };
        let files = vec![Download { name, bytes }];
        match env
            .platform
            .save_files(&request, files, self.reply(PlatformReply::Saved))
        {
            Answer::Ready(result) => self.saved_bytes(env, result, then),
            Answer::Pending => {
                self.waiting = Some(Waiting::SaveFiles(then));
                Vec::new()
            }
        }
    }

    fn saved_to(
        &mut self,
        env: &mut Env,
        path: Option<PathBuf>,
        then: Option<Then>,
    ) -> Vec<Effect> {
        if !env.editor.save_file(path.as_deref()) {
            return Vec::new();
        }
        self.kept_name = None;
        let mut effects = vec![Effect::ClearAutosave, remember(env)];
        if let Some(then) = then {
            effects.extend(self.run_then(env, then));
        }
        effects
    }

    fn saved_bytes(&mut self, env: &mut Env, result: Saved, then: Option<Then>) -> Vec<Effect> {
        match result {
            Ok(Some(name)) => {
                env.editor.saved_as(&name);
                self.kept_name = Some(name);
                self.bound = true;
                let mut effects = vec![Effect::ClearAutosave];
                if let Some(then) = then {
                    effects.extend(self.run_then(env, then));
                }
                effects
            }
            // The user cancelled: the model still has its unsaved changes.
            Ok(None) => Vec::new(),
            Err(why) => {
                env.editor.show_error(&format!("Cannot save: {why}"));
                Vec::new()
            }
        }
    }

    /// File > Import: asks for an OBJ, STL or SketchUp file, then the window opens the Import
    /// Options sheet for it.
    pub fn import(&mut self, env: &mut Env) -> Vec<Effect> {
        if self.waiting.is_some() {
            return Vec::new();
        }
        let request = OpenRequest {
            filters: import_filters(),
            kind: OpenKind::Import,
        };
        match env
            .platform
            .pick_open(&request, self.reply(PlatformReply::Picked))
        {
            Answer::Ready(picked) => self.imported(picked),
            Answer::Pending => {
                self.waiting = Some(Waiting::PickImport);
                Vec::new()
            }
        }
    }

    fn imported(&mut self, picked: Option<Picked>) -> Vec<Effect> {
        self.import = None;
        match picked {
            None => Vec::new(),
            Some(Picked {
                source: Source::Path(path),
                ..
            }) => vec![Effect::ImportSheet(path)],
            Some(Picked {
                name,
                source: Source::Bytes(bytes),
            }) => {
                let shown = PathBuf::from(&name);
                self.import = Some((name, bytes));
                vec![Effect::ImportSheet(shown)]
            }
        }
    }

    /// The Import Options sheet's Import was pressed for the file `path` (what `ImportSheet`
    /// named). True if it was imported.
    pub fn import_confirmed(&mut self, env: &mut Env, path: &Path) -> bool {
        match self.import.take() {
            Some((name, bytes)) => env.editor.import_bytes(&name, bytes),
            None => env.editor.import_file(path),
        }
    }

    /// The name the exported files start with: the model's file name or kept name, without
    /// its extension.
    fn export_stem(&self, env: &Env) -> String {
        let stem = file_name(env.editor.doc().path());
        let stem = stem.strip_suffix(".skr").unwrap_or(&stem).to_string();
        match (&self.kept_name, env.editor.doc().path()) {
            (Some(kept), None) => kept.strip_suffix(".skr").unwrap_or(kept).to_string(),
            _ => stem,
        }
    }

    /// File > Export: asks where to export, the format by the name's extension. Where files
    /// have no path the window asks for the format first (`Effect::ExportSheet`, answered by
    /// `export_as`).
    pub fn export(&mut self, env: &mut Env) -> Vec<Effect> {
        if self.waiting.is_some() {
            return Vec::new();
        }
        let stem = self.export_stem(env);
        let formats = env.editor.export_formats();
        if !env.platform.features().file_paths {
            return vec![Effect::ExportSheet(formats)];
        }
        let request = SaveRequest {
            filters: formats
                .iter()
                .map(|(name, ext)| Filter::new(name, &[*ext]))
                .collect(),
            name: format!("{stem}.{}", formats[0].1),
            ask: true,
            write_back: false,
        };
        match env
            .platform
            .pick_save_path(&request, self.reply(PlatformReply::Path))
        {
            Answer::Ready(path) => {
                if let Some(path) = path {
                    env.editor.export_file(&path);
                }
                Vec::new()
            }
            Answer::Pending => {
                self.waiting = Some(Waiting::ExportPath);
                Vec::new()
            }
        }
    }

    /// The export format `ext` was chosen (where files have no path): exports the model as
    /// bytes and has the platform save them, offering only that format.
    pub fn export_as(&mut self, env: &mut Env, ext: &str) -> Vec<Effect> {
        if self.waiting.is_some() {
            return Vec::new();
        }
        let Some((name, ext)) = env
            .editor
            .export_formats()
            .into_iter()
            .find(|(_, x)| *x == ext)
        else {
            return Vec::new();
        };
        let stem = self.export_stem(env);
        let request = SaveRequest {
            filters: vec![Filter::new(name, &[ext])],
            name: format!("{stem}.{ext}"),
            ask: true,
            write_back: false,
        };
        let Some(files) = env.editor.export_model(ext, &stem) else {
            return Vec::new();
        };
        let files = files
            .into_iter()
            .map(|(name, bytes)| Download { name, bytes })
            .collect();
        // The export's own status says what was made; only a failure to save is added to it.
        match env
            .platform
            .save_files(&request, files, self.reply(PlatformReply::Saved))
        {
            Answer::Ready(Err(why)) => env.editor.show_error(&format!("Cannot export: {why}")),
            Answer::Ready(Ok(_)) => {}
            Answer::Pending => self.waiting = Some(Waiting::ExportFiles),
        }
        Vec::new()
    }

    /// File > Export PDF: asks for the PDF's file, which the window then prints to.
    pub fn export_pdf(&mut self, env: &mut Env, options: PrintOptions) -> Vec<Effect> {
        if self.waiting.is_some() {
            return Vec::new();
        }
        let stem = file_name(env.editor.doc().path());
        let stem = stem.strip_suffix(".skr").unwrap_or(&stem);
        let request = SaveRequest {
            filters: vec![Filter::new("PDF", &["pdf"])],
            name: format!("{stem}.pdf"),
            ask: true,
            write_back: false,
        };
        match env
            .platform
            .pick_save_path(&request, self.reply(PlatformReply::Path))
        {
            Answer::Ready(Some(path)) => vec![Effect::Pdf(options, path)],
            Answer::Ready(None) => Vec::new(),
            Answer::Pending => {
                self.waiting = Some(Waiting::PdfPath(options));
                Vec::new()
            }
        }
    }

    /// Asks for a PNG or JPEG image to give `material` as its texture.
    pub fn load_texture(&mut self, env: &mut Env, material: MaterialId) {
        if self.waiting.is_some() {
            return;
        }
        let request = OpenRequest {
            filters: vec![Filter::new("PNG or JPEG image", &["png", "jpg", "jpeg"])],
            kind: OpenKind::Texture,
        };
        match env
            .platform
            .pick_open(&request, self.reply(PlatformReply::Picked))
        {
            Answer::Ready(picked) => self.textured(env, material, picked),
            Answer::Pending => self.waiting = Some(Waiting::PickTexture(material)),
        }
    }

    fn textured(&mut self, env: &mut Env, material: MaterialId, picked: Option<Picked>) {
        match picked {
            Some(Picked {
                source: Source::Path(path),
                ..
            }) => {
                env.editor.load_texture(material, &path);
            }
            Some(Picked {
                source: Source::Bytes(_),
                ..
            }) => env
                .editor
                .show_error("Texture images cannot be loaded from files here yet"),
            None => {}
        }
    }

    /// Asks for the library folder.
    pub fn choose_folder(&mut self, env: &mut Env) -> Vec<Effect> {
        if self.waiting.is_some() {
            return Vec::new();
        }
        match env.platform.pick_folder(self.reply(PlatformReply::Path)) {
            Answer::Ready(Some(folder)) => vec![Effect::LibraryFolder(folder)],
            Answer::Ready(None) => Vec::new(),
            Answer::Pending => {
                self.waiting = Some(Waiting::PickFolder);
                Vec::new()
            }
        }
    }

    /// The platform answered a pending request.
    pub fn answered(&mut self, env: &mut Env, answer: PlatformReply) -> Vec<Effect> {
        let Some(waiting) = self.waiting.take() else {
            return Vec::new();
        };
        match (waiting, answer) {
            (Waiting::Unsaved(then), PlatformReply::Unsaved(choice)) => {
                self.after_unsaved(env, choice, then)
            }
            (Waiting::SavePath(then), PlatformReply::Path(Some(path))) => {
                self.saved_to(env, Some(path), then)
            }
            (Waiting::SaveFiles(then), PlatformReply::Saved(result)) => {
                self.saved_bytes(env, result, then)
            }
            (Waiting::PickOpen, PlatformReply::Picked(picked)) => self.opened(env, picked),
            (Waiting::PickImport, PlatformReply::Picked(picked)) => self.imported(picked),
            (Waiting::PickTexture(material), PlatformReply::Picked(picked)) => {
                self.textured(env, material, picked);
                Vec::new()
            }
            (Waiting::PickFolder, PlatformReply::Path(path)) => {
                path.map(Effect::LibraryFolder).into_iter().collect()
            }
            (Waiting::ExportPath, PlatformReply::Path(path)) => {
                if let Some(path) = path {
                    env.editor.export_file(&path);
                }
                Vec::new()
            }
            (Waiting::ExportFiles, PlatformReply::Saved(result)) => {
                if let Err(why) = result {
                    env.editor.show_error(&format!("Cannot export: {why}"));
                }
                Vec::new()
            }
            (Waiting::PdfPath(options), PlatformReply::Path(path)) => path
                .map(|path| Effect::Pdf(options, path))
                .into_iter()
                .collect(),
            // An answer to something no longer waited for.
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    use glam::DVec2;
    use session::{Command, Document};

    use super::*;
    use crate::clipboard::{Clipboard, MemoryClipboard};
    use crate::platform::Features;

    /// A platform that answers from queues, and remembers what it was asked.
    struct Fake {
        features: Features,
        unsaved: RefCell<VecDeque<Answer<UnsavedChoice>>>,
        opens: RefCell<VecDeque<Answer<Option<Picked>>>>,
        paths: RefCell<VecDeque<Answer<Option<PathBuf>>>>,
        saves: RefCell<VecDeque<Answer<Saved>>>,
        asked: RefCell<Vec<UnsavedPrompt>>,
        saved: RefCell<Vec<(SaveRequest, Vec<Download>)>>,
        path_requests: RefCell<Vec<SaveRequest>>,
        prefs: RefCell<Option<String>>,
        asks: Cell<usize>,
        kinds: RefCell<Vec<OpenKind>>,
        opened: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(features: Features) -> Self {
            Self {
                features,
                unsaved: RefCell::default(),
                opens: RefCell::default(),
                paths: RefCell::default(),
                saves: RefCell::default(),
                asked: RefCell::default(),
                saved: RefCell::default(),
                path_requests: RefCell::default(),
                prefs: RefCell::default(),
                asks: Cell::new(0),
                kinds: RefCell::default(),
                opened: RefCell::default(),
            }
        }
    }

    impl Platform for Fake {
        fn features(&self) -> Features {
            self.features
        }

        fn pick_open(
            &self,
            request: &OpenRequest,
            _reply: Reply<Option<Picked>>,
        ) -> Answer<Option<Picked>> {
            self.kinds.borrow_mut().push(request.kind);
            self.opens.borrow_mut().pop_front().expect("an open answer")
        }

        fn file_opened(&self, name: &str) {
            self.opened.borrow_mut().push(name.to_string());
        }

        fn pick_save_path(
            &self,
            request: &SaveRequest,
            _reply: Reply<Option<PathBuf>>,
        ) -> Answer<Option<PathBuf>> {
            self.path_requests.borrow_mut().push(request.clone());
            self.paths.borrow_mut().pop_front().expect("a path answer")
        }

        fn pick_folder(&self, _reply: Reply<Option<PathBuf>>) -> Answer<Option<PathBuf>> {
            Answer::Ready(None)
        }

        fn save_files(
            &self,
            request: &SaveRequest,
            files: Vec<Download>,
            _reply: Reply<Saved>,
        ) -> Answer<Saved> {
            self.saved.borrow_mut().push((request.clone(), files));
            self.saves.borrow_mut().pop_front().expect("a save answer")
        }

        fn read_prefs(&self) -> Option<String> {
            self.prefs.borrow().clone()
        }

        fn write_prefs(&self, text: &str) -> Result<(), String> {
            *self.prefs.borrow_mut() = Some(text.to_string());
            Ok(())
        }

        fn confirm_unsaved(
            &self,
            _parent: Option<&Window>,
            prompt: &UnsavedPrompt,
            _reply: Reply<UnsavedChoice>,
        ) -> Answer<UnsavedChoice> {
            self.asks.set(self.asks.get() + 1);
            self.asked.borrow_mut().push(prompt.clone());
            self.unsaved.borrow_mut().pop_front().expect("an answer")
        }

        fn clipboard(&self) -> Box<dyn Clipboard> {
            Box::new(MemoryClipboard::default())
        }

        fn open_url(&self, _url: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn flows() -> Flows {
        Flows::new(Arc::new(|_| {}))
    }

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    fn edited() -> Editor {
        let mut e = editor();
        e.execute(Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        assert!(e.doc().is_modified());
        e
    }

    /// Runs `step` over `editor` and `platform`.
    fn run(
        flows: &mut Flows,
        editor: &mut Editor,
        platform: &Fake,
        step: impl FnOnce(&mut Flows, &mut Env) -> Vec<Effect>,
    ) -> Vec<Effect> {
        let mut env = Env {
            editor,
            platform,
            parent: None,
            recovered: None,
        };
        step(flows, &mut env)
    }

    /// The bytes of a `.skr` file of a model with one edge.
    fn skr_bytes() -> Vec<u8> {
        let mut doc = Document::new(DVec2::new(800.0, 600.0));
        doc.execute(Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        doc.save_bytes().unwrap()
    }

    fn web() -> Fake {
        Fake::new(Features::WEB)
    }

    fn desktop() -> Fake {
        Fake::new(Features::ALL)
    }

    #[test]
    fn a_model_without_changes_is_replaced_without_asking() {
        let platform = desktop();
        let mut editor = editor();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::NewModel)
        });
        assert_eq!(fx, [Effect::ClearAutosave]);
        assert_eq!(platform.asks.get(), 0);
    }

    #[test]
    fn a_window_with_its_own_dialogs_asks_the_unsaved_question_itself() {
        let platform = web();
        let mut editor = edited();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::NewModel)
        });
        let [Effect::AskUnsaved(prompt)] = &fx[..] else {
            panic!("expected the question, got {fx:?}");
        };
        assert!(prompt.question.contains("Untitled.skr"));
        assert_eq!(platform.asks.get(), 0);
        // The model is kept until the answer comes; a second request is ignored meanwhile.
        assert!(editor.doc().is_modified());
        let again = run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::Quit)
        });
        assert!(again.is_empty());
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.answered(env, PlatformReply::Unsaved(UnsavedChoice::DontSave))
        });
        assert_eq!(fx, [Effect::ClearAutosave]);
        assert!(!editor.doc().is_modified());
    }

    #[test]
    fn cancelling_the_unsaved_question_leaves_the_model_and_does_nothing() {
        let platform = desktop();
        platform
            .unsaved
            .borrow_mut()
            .push_back(Answer::Ready(UnsavedChoice::Cancel));
        let mut editor = edited();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::Quit)
        });
        assert!(fx.is_empty());
        assert!(editor.doc().is_modified());
        assert_eq!(
            platform.asked.borrow()[0].question,
            "Do you want to save changes to \"Untitled.skr\"?"
        );
    }

    #[test]
    fn dont_save_goes_on_and_save_goes_on_only_once_saved() {
        let platform = desktop();
        platform
            .unsaved
            .borrow_mut()
            .push_back(Answer::Ready(UnsavedChoice::DontSave));
        let mut editor = edited();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::Quit)
        });
        assert_eq!(fx, [Effect::Quit]);

        // Save with no file asks for a path; cancelling that stops the quit.
        platform
            .unsaved
            .borrow_mut()
            .push_back(Answer::Ready(UnsavedChoice::Save));
        platform.paths.borrow_mut().push_back(Answer::Ready(None));
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::Quit)
        });
        assert!(fx.is_empty());
        assert!(editor.doc().is_modified());
        assert_eq!(platform.path_requests.borrow()[0].name, "Untitled.skr");

        // Saved to the path chosen: the quit goes on.
        let dir = std::env::temp_dir().join(format!("sketchup-alt-flows-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("house.skr");
        platform
            .unsaved
            .borrow_mut()
            .push_back(Answer::Ready(UnsavedChoice::Save));
        platform
            .paths
            .borrow_mut()
            .push_back(Answer::Ready(Some(path.clone())));
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::Quit)
        });
        assert_eq!(
            fx,
            [
                Effect::ClearAutosave,
                Effect::RememberFile(Some(path.clone())),
                Effect::Quit
            ]
        );
        assert!(!editor.doc().is_modified());
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_model_saved_from_the_prompt_is_remembered_before_the_new_one_replaces_it() {
        let platform = desktop();
        let dir =
            std::env::temp_dir().join(format!("sketchup-alt-flows-rem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("house.skr");
        platform
            .unsaved
            .borrow_mut()
            .push_back(Answer::Ready(UnsavedChoice::Save));
        platform
            .paths
            .borrow_mut()
            .push_back(Answer::Ready(Some(path.clone())));
        let mut editor = edited();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::NewModel)
        });
        assert!(fx.contains(&Effect::RememberFile(Some(path))), "{fx:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_question_answered_later_holds_the_step_until_the_answer_comes() {
        let platform = desktop();
        platform.unsaved.borrow_mut().push_back(Answer::Pending);
        let mut editor = edited();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::NewModel)
        });
        assert!(fx.is_empty());
        assert!(editor.doc().is_modified(), "nothing happens meanwhile");
        // Another request while one waits is ignored.
        let fx = run(&mut f, &mut editor, &platform, |f, env| f.open(env));
        assert!(fx.is_empty());
        assert_eq!(platform.asks.get(), 1);

        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.answered(env, PlatformReply::Unsaved(UnsavedChoice::DontSave))
        });
        assert_eq!(fx, [Effect::ClearAutosave]);
        assert!(!editor.doc().is_modified());
        // An answer nobody waits for does nothing.
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.answered(env, PlatformReply::Unsaved(UnsavedChoice::DontSave))
        });
        assert!(fx.is_empty());
    }

    #[test]
    fn a_picked_file_of_bytes_opens_and_is_kept_by_name() {
        let platform = web();
        platform.opens.borrow_mut().push_back(Answer::Pending);
        let mut editor = editor();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| f.open(env));
        assert!(fx.is_empty(), "the picker has not answered");
        let picked = Picked {
            name: "house.skr".into(),
            source: Source::Bytes(skr_bytes()),
        };
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.answered(env, PlatformReply::Picked(Some(picked)))
        });
        assert_eq!(fx, [Effect::ClearAutosave, Effect::RememberFile(None)]);
        assert!(!editor.doc().is_modified());
        assert_eq!(editor.doc().scene().root().edges().count(), 1);
        assert_eq!(f.kept_name(), Some("house.skr"));
    }

    #[test]
    fn a_file_that_cannot_be_opened_says_why_and_changes_nothing() {
        let platform = web();
        platform
            .opens
            .borrow_mut()
            .push_back(Answer::Ready(Some(Picked {
                name: "bad.skr".into(),
                source: Source::Bytes(b"not a model".to_vec()),
            })));
        let mut editor = editor();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| f.open(env));
        assert!(fx.is_empty());
        assert_eq!(f.kept_name(), None);
        assert_eq!(editor.message_kind(), Some(api::MessageKind::Error));
    }

    #[test]
    fn saving_where_files_have_no_path_hands_over_the_bytes_and_marks_the_model_saved() {
        let platform = web();
        platform
            .saves
            .borrow_mut()
            .push_back(Answer::Ready(Ok(Some("mine.skr".into()))));
        let mut editor = edited();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.save(env, false, None)
        });
        assert_eq!(fx, [Effect::ClearAutosave]);
        assert!(!editor.doc().is_modified());
        assert_eq!(f.kept_name(), Some("mine.skr"));
        let saved = platform.saved.borrow();
        let (request, files) = &saved[0];
        assert_eq!(request.name, "Untitled.skr");
        assert!(!request.ask, "Save does not insist on asking");
        assert_eq!(files.len(), 1);
        // The bytes reopen as the model that was saved.
        let mut other = Document::new(DVec2::new(800.0, 600.0));
        other.open_bytes("x.skr", &files[0].bytes, true).unwrap();
        assert_eq!(other.scene().root().edges().count(), 1);
        drop(saved);

        // Save As asks again, suggesting the name kept.
        platform
            .saves
            .borrow_mut()
            .push_back(Answer::Ready(Ok(None)));
        run(&mut f, &mut editor, &platform, |f, env| {
            f.save(env, true, None)
        });
        let saved = platform.saved.borrow();
        assert_eq!(saved[1].0.name, "mine.skr");
        assert!(saved[1].0.ask);
    }

    #[test]
    fn a_cancelled_download_leaves_the_unsaved_changes() {
        let platform = web();
        platform
            .saves
            .borrow_mut()
            .push_back(Answer::Ready(Ok(None)));
        let mut editor = edited();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.save(env, false, Some(Then::Quit))
        });
        assert!(fx.is_empty(), "no quit");
        assert!(editor.doc().is_modified());
        assert_eq!(f.kept_name(), None);
    }

    #[test]
    fn a_failed_save_says_why_and_does_not_go_on() {
        let platform = web();
        platform.saves.borrow_mut().push_back(Answer::Pending);
        let mut editor = edited();
        let mut f = flows();
        run(&mut f, &mut editor, &platform, |f, env| {
            f.save(env, false, Some(Then::Quit))
        });
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.answered(env, PlatformReply::Saved(Err("disk full".into())))
        });
        assert!(fx.is_empty());
        assert!(editor.doc().is_modified());
        assert_eq!(editor.message_kind(), Some(api::MessageKind::Error));
    }

    #[test]
    fn an_imported_file_of_bytes_waits_for_the_options_sheet() {
        let platform = web();
        let obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec();
        platform
            .opens
            .borrow_mut()
            .push_back(Answer::Ready(Some(Picked {
                name: "tri.obj".into(),
                source: Source::Bytes(obj),
            })));
        let mut editor = editor();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| f.import(env));
        assert_eq!(fx, [Effect::ImportSheet(PathBuf::from("tri.obj"))]);
        let mut env = Env {
            editor: &mut editor,
            platform: &platform,
            parent: None,
            recovered: None,
        };
        assert!(f.import_confirmed(&mut env, Path::new("tri.obj")));
        assert!(editor.doc().is_modified());
    }

    fn with_rectangle() -> Editor {
        let mut editor = editor();
        editor
            .execute(Command::AddRectangle {
                corner: [0.0, 0.0, 0.0],
                opposite: [1.0, 1.0, 0.0],
                normal: None,
            })
            .unwrap();
        editor
    }

    #[test]
    fn exporting_where_files_have_no_path_asks_for_the_format_then_offers_only_it() {
        let platform = web();
        let mut editor = with_rectangle();
        let mut f = flows();
        let fx = run(&mut f, &mut editor, &platform, |f, env| f.export(env));
        let [Effect::ExportSheet(formats)] = &fx[..] else {
            panic!("expected the format sheet, got {fx:?}");
        };
        assert_eq!(formats[0].1, "obj", "OBJ before any export");
        assert!(formats.iter().any(|(_, x)| *x == "stl"));
        assert!(platform.saved.borrow().is_empty(), "nothing saved yet");

        platform
            .saves
            .borrow_mut()
            .push_back(Answer::Ready(Ok(Some("Untitled.stl".into()))));
        run(&mut f, &mut editor, &platform, |f, env| {
            f.export_as(env, "stl")
        });
        {
            let saved = platform.saved.borrow();
            let (request, files) = &saved[0];
            assert_eq!(request.name, "Untitled.stl");
            assert_eq!(request.filters.len(), 1, "only the chosen format");
            assert_eq!(request.filters[0].extensions, ["stl"]);
            assert!(request.ask && !request.write_back);
            assert_eq!(files[0].name, "Untitled.stl");
            // The bytes are an STL (ASCII "solid" or binary 80-byte header and a count).
            assert!(files[0].bytes.len() > 84);
        }
        // The format used last comes first next time.
        let fx = run(&mut f, &mut editor, &platform, |f, env| f.export(env));
        let [Effect::ExportSheet(formats)] = &fx[..] else {
            panic!("expected the format sheet");
        };
        assert_eq!(formats[0].1, "stl");
        // An unknown format does nothing.
        let fx = run(&mut f, &mut editor, &platform, |f, env| {
            f.export_as(env, "xyz")
        });
        assert!(fx.is_empty());
        assert_eq!(platform.saved.borrow().len(), 1);
    }

    fn opened_skr(name: &str) -> Answer<Option<Picked>> {
        Answer::Ready(Some(Picked {
            name: name.into(),
            source: Source::Bytes(skr_bytes()),
        }))
    }

    #[test]
    fn a_save_may_write_back_only_while_the_model_is_the_kept_file() {
        let platform = web();
        platform.opens.borrow_mut().push_back(opened_skr("a.skr"));
        let mut editor = editor();
        let mut f = flows();
        run(&mut f, &mut editor, &platform, |f, env| f.open(env));
        assert_eq!(*platform.opened.borrow(), ["a.skr"]);
        assert_eq!(*platform.kinds.borrow(), [OpenKind::Model]);

        let save = |f: &mut Flows, editor: &mut Editor, choose: bool| {
            platform
                .saves
                .borrow_mut()
                .push_back(Answer::Ready(Ok(Some("x.skr".into()))));
            run(f, editor, &platform, |f, env| f.save(env, choose, None));
            platform.saved.borrow().last().unwrap().0.clone()
        };
        // Opened from the kept file: Save writes back; Save As never.
        assert!(save(&mut f, &mut editor, false).write_back);
        assert!(!save(&mut f, &mut editor, true).write_back);

        // New: the kept file is not this model's.
        run(&mut f, &mut editor, &platform, |f, env| {
            f.settle_and(env, Then::NewModel)
        });
        editor
            .execute(Command::AddEdge {
                start: [0.0, 0.0, 0.0],
                end: [2.0, 0.0, 0.0],
            })
            .unwrap();
        assert!(!save(&mut f, &mut editor, false).write_back);
        // Saved to a file: the next Save is to that one.
        assert!(save(&mut f, &mut editor, false).write_back);
    }

    #[test]
    fn a_file_that_fails_to_open_is_not_kept() {
        let platform = web();
        platform
            .opens
            .borrow_mut()
            .push_back(Answer::Ready(Some(Picked {
                name: "bad.skr".into(),
                source: Source::Bytes(b"not a model".to_vec()),
            })));
        let mut editor = editor();
        let mut f = flows();
        run(&mut f, &mut editor, &platform, |f, env| f.open(env));
        assert!(platform.opened.borrow().is_empty());
        assert_eq!(f.kept_name(), None);
    }

    #[test]
    fn import_texture_and_export_leave_the_kept_file_alone() {
        let platform = web();
        platform.opens.borrow_mut().push_back(opened_skr("a.skr"));
        let mut editor = editor();
        let mut f = flows();
        run(&mut f, &mut editor, &platform, |f, env| f.open(env));
        assert_eq!(platform.opened.borrow().len(), 1);

        platform
            .opens
            .borrow_mut()
            .push_back(Answer::Ready(Some(Picked {
                name: "tri.obj".into(),
                source: Source::Bytes(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()),
            })));
        run(&mut f, &mut editor, &platform, |f, env| f.import(env));
        platform.opens.borrow_mut().push_back(Answer::Ready(None));
        let material = MaterialId(1);
        run(&mut f, &mut editor, &platform, |f, env| {
            f.load_texture(env, material);
            Vec::new()
        });
        assert_eq!(
            *platform.kinds.borrow(),
            [OpenKind::Model, OpenKind::Import, OpenKind::Texture]
        );
        assert_eq!(platform.opened.borrow().len(), 1, "only the open was kept");

        editor
            .execute(Command::AddRectangle {
                corner: [0.0, 0.0, 0.0],
                opposite: [1.0, 1.0, 0.0],
                normal: None,
            })
            .unwrap();
        platform
            .saves
            .borrow_mut()
            .push_back(Answer::Ready(Ok(Some("a.obj".into()))));
        run(&mut f, &mut editor, &platform, |f, env| {
            f.export_as(env, "obj")
        });
        assert!(!platform.saved.borrow()[0].0.write_back);
        // The model is still the kept file's: Save writes back.
        platform
            .saves
            .borrow_mut()
            .push_back(Answer::Ready(Ok(Some("a.skr".into()))));
        run(&mut f, &mut editor, &platform, |f, env| {
            f.save(env, false, None)
        });
        assert!(platform.saved.borrow()[1].0.write_back);
    }

    #[test]
    fn preferences_are_text_the_platform_keeps() {
        let platform = web();
        let prefs = crate::prefs::Prefs {
            autosave_minutes: 7,
            ..crate::prefs::Prefs::default()
        };
        platform
            .write_prefs(&crate::prefs::to_text(&prefs))
            .unwrap();
        let read = platform.read_prefs().map(|t| crate::prefs::from_text(&t));
        assert_eq!(read, Some(prefs));
    }

    #[test]
    fn the_unsaved_question_names_the_file() {
        assert_eq!(
            unsaved_question(Some(Path::new("/m/house.skr"))),
            "Do you want to save changes to \"house.skr\"?"
        );
        assert_eq!(
            unsaved_question(None),
            "Do you want to save changes to \"Untitled.skr\"?"
        );
    }

    #[test]
    fn a_sketchup_file_kept_by_name_is_saved_as_skr() {
        assert_eq!(skr_name("desk.skp"), "desk.skr");
        assert_eq!(skr_name("house.skr"), "house.skr");
    }
}
