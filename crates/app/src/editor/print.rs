//! File > Export PDF... and File > Print...: the Print sheet's values, rendering the view to a
//! PDF with the window's GPU, and handing a printed temp file to the system's viewer.

#[cfg(not(target_arch = "wasm32"))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};

use render::Snapshotter;
use render::print::render_pdf;
use serde::{Deserialize, Serialize};
use session::print::{DEFAULT_DPI, MAX_DPI, MIN_DPI, Paper, PrintSpec};

use super::{Editor, Message};

/// A paper size the Print sheet offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperChoice {
    #[default]
    A4,
    A3,
    Letter,
    Legal,
    Tabloid,
}

impl PaperChoice {
    pub const ALL: [PaperChoice; 5] = [
        PaperChoice::A4,
        PaperChoice::A3,
        PaperChoice::Letter,
        PaperChoice::Legal,
        PaperChoice::Tabloid,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PaperChoice::A4 => "A4",
            PaperChoice::A3 => "A3",
            PaperChoice::Letter => "Letter",
            PaperChoice::Legal => "Legal",
            PaperChoice::Tabloid => "Tabloid",
        }
    }

    fn paper(self) -> Paper {
        match self {
            PaperChoice::A4 => Paper::A4,
            PaperChoice::A3 => Paper::A3,
            PaperChoice::Letter => Paper::Letter,
            PaperChoice::Legal => Paper::Legal,
            PaperChoice::Tabloid => Paper::Tabloid,
        }
    }
}

/// What the Print sheet asks: kept in the preferences, so missing keys are their defaults.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintOptions {
    pub paper: PaperChoice,
    pub landscape: bool,
    /// Print at 1:scale (a parallel view); None fits the view on one sheet.
    pub scale: Option<f64>,
    pub dpi: u32,
    /// Add the cut list as pages after the view.
    pub cut_list: bool,
}

impl Default for PrintOptions {
    fn default() -> Self {
        PrintOptions {
            paper: PaperChoice::A4,
            landscape: true,
            scale: None,
            dpi: DEFAULT_DPI,
            cut_list: false,
        }
    }
}

impl PrintOptions {
    /// The scale a sheet starts the 1:N field with.
    pub const DEFAULT_SCALE: f64 = 100.0;

    /// What `session::print` lays out.
    pub fn spec(&self) -> PrintSpec {
        PrintSpec {
            paper: self.paper.paper(),
            landscape: self.landscape,
            scale: self.scale,
            dpi: self.dpi,
            cut_list: self.cut_list,
        }
    }

    /// Brought into range, as a prefs file may hold anything: dpi within what printing takes,
    /// a scale that is not a positive number becomes Fit.
    pub fn cleaned(mut self) -> Self {
        self.dpi = self.dpi.clamp(MIN_DPI, MAX_DPI);
        if self.scale.is_some_and(|n| !(n.is_finite() && n > 0.0)) {
            self.scale = None;
        }
        self
    }

    /// Whether the sheet's values can be printed.
    pub fn valid(&self) -> bool {
        (MIN_DPI..=MAX_DPI).contains(&self.dpi)
            && self.scale.is_none_or(|n| n.is_finite() && n > 0.0)
    }
}

/// What the Print sheet is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintPurpose {
    /// File > Export PDF...: ask for a file.
    Export,
    /// File > Print...: print to a temp file and open it.
    Print,
}

#[cfg(not(target_arch = "wasm32"))]
/// An operating system, as far as opening a file goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerOs {
    Mac,
    Linux,
    Windows,
}

#[cfg(not(target_arch = "wasm32"))]
impl ViewerOs {
    pub fn current() -> ViewerOs {
        if cfg!(target_os = "macos") {
            ViewerOs::Mac
        } else if cfg!(windows) {
            ViewerOs::Windows
        } else {
            ViewerOs::Linux
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The program and arguments that open `path` in the default viewer. No shell string is
/// built: the path is an argument of its own.
pub fn open_command(platform: ViewerOs, path: &Path) -> (OsString, Vec<OsString>) {
    let path = path.as_os_str().to_owned();
    match platform {
        ViewerOs::Mac => ("open".into(), vec![path]),
        ViewerOs::Linux => ("xdg-open".into(), vec![path]),
        ViewerOs::Windows => ("explorer.exe".into(), vec![path]),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The temp file number `n` of process `pid` is printed to.
pub fn print_file_name(pid: u32, n: u64) -> String {
    format!("sketchup-alt-print-{pid}-{n}.pdf")
}

#[cfg(not(target_arch = "wasm32"))]
/// A free path for a printed file in the system's temp folder.
fn temp_print_path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(print_file_name(std::process::id(), n));
        if std::fs::symlink_metadata(&path).is_err() {
            return path;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// How long a printed temp file is kept.
const STALE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[cfg(not(target_arch = "wasm32"))]
/// Removes the files of `dir` named like a printed temp file that are regular files (not
/// links), older than 24 hours at `now`, and, where `owner` is given (Unix), owned by that user.
/// Returns how many were removed. Failures are ignored: it is housekeeping.
pub fn remove_stale_prints(dir: &Path, now: std::time::SystemTime, owner: Option<u32>) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("sketchup-alt-print-") && name.ends_with(".pdf")) {
            continue;
        }
        // The entry itself, never what a link points to.
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_file() || !owned_by(&meta, owner) {
            continue;
        }
        let old = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > STALE);
        if old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(unix)]
fn owned_by(meta: &std::fs::Metadata, owner: Option<u32>) -> bool {
    use std::os::unix::fs::MetadataExt;
    owner.is_none_or(|uid| meta.uid() == uid)
}

#[cfg(not(any(unix, target_arch = "wasm32")))]
fn owned_by(_: &std::fs::Metadata, _: Option<u32>) -> bool {
    true
}

#[cfg(not(target_arch = "wasm32"))]
/// The user id of the file at `path` (the current user's, for a file just created), where the
/// system has user ids.
fn file_owner(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(path).ok().map(|m| m.uid())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// A print waiting to run: the window shows its status message first, so the render (which
/// blocks the window) does not start before "Printing..." is on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintJob {
    pub options: PrintOptions,
    /// The file to write (Export PDF); None prints to a temp file and opens it.
    pub path: Option<PathBuf>,
}

/// Holds one [`PrintJob`] for two frames: the frame whose actions queued it presented before
/// the message was set, the next one presents the message, and the job runs after that.
#[derive(Debug, Default)]
pub struct PrintQueue {
    job: Option<PrintJob>,
    frames: u8,
}

impl PrintQueue {
    /// Queues `job`, replacing one still waiting.
    pub fn queue(&mut self, job: PrintJob) {
        self.job = Some(job);
        self.frames = 0;
    }

    /// Whether a job waits (so another frame is needed).
    pub fn waiting(&self) -> bool {
        self.job.is_some()
    }

    /// Called at the end of every frame, after its actions: the job once the message has been
    /// presented.
    pub fn frame_done(&mut self) -> Option<PrintJob> {
        self.job.as_ref()?;
        self.frames += 1;
        if self.frames < 2 {
            return None;
        }
        self.job.take()
    }
}

impl Editor {
    /// Shows `text` in the status bar as information.
    pub fn show_info(&mut self, text: &str) {
        self.message = Some(Message::info(text));
    }

    /// Prints the view as `options` say to the PDF at `path` (replacing a file there: the save
    /// dialog asked). False if it cannot be printed; the status then says why.
    pub fn export_pdf_file(
        &mut self,
        snapshotter: Option<&mut Snapshotter>,
        options: &PrintOptions,
        path: &Path,
    ) -> bool {
        self.message = None;
        let Some(snapshotter) = snapshotter else {
            self.show_error("Printing needs a GPU adapter");
            return false;
        };
        let printed = match render_pdf(snapshotter, self.doc(), &options.spec()) {
            Ok(printed) => printed,
            Err(e) => {
                self.show_error(&e.message);
                return false;
            }
        };
        let written = self
            .doc()
            .write_pdf(&path.display().to_string(), &printed.pdf, true, false);
        if let Err(e) = written {
            self.show_error(&e.message);
            return false;
        }
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let sheets = printed.sheets;
        let mut message = format!(
            "Exported {name} ({sheets} sheet{})",
            if sheets == 1 { "" } else { "s" }
        );
        if !printed.warnings.is_empty() {
            message = format!("{message} ({})", printed.warnings.join("; "));
        }
        self.message = Some(Message::info(message));
        true
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// File > Print...: prints to a private temp file and opens it with the system's viewer.
    /// The status reports the failure of either step.
    pub fn print_and_open(
        &mut self,
        snapshotter: Option<&mut Snapshotter>,
        options: &PrintOptions,
    ) -> bool {
        self.message = None;
        let Some(snapshotter) = snapshotter else {
            self.show_error("Printing needs a GPU adapter");
            return false;
        };
        let printed = match render_pdf(snapshotter, self.doc(), &options.spec()) {
            Ok(printed) => printed,
            Err(e) => {
                self.show_error(&e.message);
                return false;
            }
        };
        let path = temp_print_path();
        if let Err(e) = session::file::replace_private(&path, &printed.pdf) {
            self.show_error(&format!("Could not write {}: {e}", path.display()));
            return false;
        }
        // Housekeeping: earlier prints no one will look at now.
        remove_stale_prints(
            &std::env::temp_dir(),
            std::time::SystemTime::now(),
            file_owner(&path),
        );
        match open_file(&path) {
            Ok(()) => {
                self.message = Some(Message::info(format!("Opened {}", path.display())));
                true
            }
            Err(e) => {
                self.show_error(&format!(
                    "Printed {} but could not open it: {e}",
                    path.display()
                ));
                false
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Opens `path` with the system's default viewer. The program is reaped on a thread, so it
/// does not linger as a zombie.
pub(crate) fn open_file(path: &Path) -> Result<(), String> {
    let (program, args) = open_command(ViewerOs::current(), path);
    let mut child = std::process::Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", program.to_string_lossy()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_viewer_is_opened_per_os_without_a_shell_string() {
        let path = Path::new("/tmp/a b/sketchup-alt-print-1-0.pdf");
        let (program, args) = open_command(ViewerOs::Mac, path);
        assert_eq!(program, "open");
        assert_eq!(args, [path.as_os_str()]);
        let (program, args) = open_command(ViewerOs::Linux, path);
        assert_eq!(program, "xdg-open");
        assert_eq!(args, [path.as_os_str()]);
        let (program, args) = open_command(ViewerOs::Windows, path);
        assert_eq!(program, "explorer.exe");
        assert_eq!(args, [path.as_os_str()]);
    }

    #[test]
    fn printed_files_are_named_by_process_and_number() {
        assert_eq!(print_file_name(42, 3), "sketchup-alt-print-42-3.pdf");
        let a = temp_print_path();
        assert!(
            a.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!("sketchup-alt-print-{}-", std::process::id()))
        );
    }

    #[test]
    fn a_print_waits_two_frames_for_its_message_to_be_shown() {
        let job = PrintJob {
            options: PrintOptions::default(),
            path: None,
        };
        let mut queue = PrintQueue::default();
        assert_eq!(queue.frame_done(), None);
        assert!(!queue.waiting());
        queue.queue(job.clone());
        assert!(queue.waiting());
        // The frame that queued it, then the frame that presented the message.
        assert_eq!(queue.frame_done(), None);
        assert!(queue.waiting());
        assert_eq!(queue.frame_done(), Some(job.clone()));
        assert!(!queue.waiting());
        assert_eq!(queue.frame_done(), None);
        // Queuing again starts the wait over.
        queue.queue(job.clone());
        assert_eq!(queue.frame_done(), None);
        queue.queue(job.clone());
        assert_eq!(queue.frame_done(), None);
        assert_eq!(queue.frame_done(), Some(job));
    }

    #[test]
    fn only_old_regular_printed_files_of_the_user_are_removed() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join(format!("stale-prints-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        let make = |name: &str, age_hours: u64| {
            let path = dir.join(name);
            let file = std::fs::File::create(&path).unwrap();
            file.set_modified(now - Duration::from_secs(age_hours * 3600))
                .unwrap();
            path
        };
        let old = make("sketchup-alt-print-1-0.pdf", 25);
        let fresh = make("sketchup-alt-print-1-1.pdf", 23);
        let other_name = make("notes.pdf", 100);
        let other_ext = make("sketchup-alt-print-1-2.txt", 100);
        let folder = dir.join("sketchup-alt-print-1-3.pdf");
        std::fs::create_dir(&folder).unwrap();
        #[cfg(unix)]
        let link = {
            let target = make("target.dat", 100);
            let link = dir.join("sketchup-alt-print-1-4.pdf");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            link
        };
        assert_eq!(remove_stale_prints(&dir, now, file_owner(&fresh)), 1);
        assert!(!old.exists());
        for kept in [&fresh, &other_name, &other_ext, &folder] {
            assert!(kept.exists(), "{kept:?}");
        }
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(&link).is_ok(), "a link is left");
        // Another user's file is left alone.
        let theirs = make("sketchup-alt-print-2-0.pdf", 48);
        let me = file_owner(&theirs).unwrap_or(0);
        if cfg!(unix) {
            assert_eq!(remove_stale_prints(&dir, now, Some(me.wrapping_add(1))), 0);
            assert!(theirs.exists());
        }
        assert_eq!(remove_stale_prints(&dir, now, Some(me)), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_map_to_a_layout_spec_and_are_cleaned() {
        let options = PrintOptions {
            paper: PaperChoice::Tabloid,
            landscape: false,
            scale: Some(50.0),
            dpi: 200,
            cut_list: true,
        };
        let spec = options.spec();
        assert_eq!(spec.paper, Paper::Tabloid);
        assert!(!spec.landscape);
        assert_eq!((spec.scale, spec.dpi), (Some(50.0), 200));
        assert!(spec.cut_list);
        let wild = PrintOptions {
            scale: Some(-3.0),
            dpi: 5000,
            ..options
        };
        assert!(!wild.valid());
        let cleaned = wild.cleaned();
        assert_eq!((cleaned.scale, cleaned.dpi), (None, MAX_DPI));
        assert!(cleaned.valid());
    }
}
