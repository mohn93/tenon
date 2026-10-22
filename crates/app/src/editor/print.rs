//! File > Export PDF... and File > Print...: the Print sheet's values, rendering the view to a
//! PDF with the window's GPU, and handing a printed temp file to the system's viewer.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
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
}

impl Default for PrintOptions {
    fn default() -> Self {
        PrintOptions {
            paper: PaperChoice::A4,
            landscape: true,
            scale: None,
            dpi: DEFAULT_DPI,
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

/// An operating system, as far as opening a file goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Linux,
    Windows,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// The program and arguments that open `path` in the default viewer. No shell string is
/// built: the path is an argument of its own.
pub fn open_command(platform: Platform, path: &Path) -> (OsString, Vec<OsString>) {
    let path = path.as_os_str().to_owned();
    match platform {
        Platform::Mac => ("open".into(), vec![path]),
        Platform::Linux => ("xdg-open".into(), vec![path]),
        Platform::Windows => (
            "cmd".into(),
            vec!["/C".into(), "start".into(), "".into(), path],
        ),
    }
}

/// The temp file number `n` of process `pid` is printed to.
pub fn print_file_name(pid: u32, n: u64) -> String {
    format!("sketchup-alt-print-{pid}-{n}.pdf")
}

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

impl Editor {
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

    /// File > Print...: prints to a private temp file and opens it with the system's viewer.
    /// The status reports the failure of either step.
    pub fn print_and_open(
        &mut self,
        snapshotter: Option<&mut Snapshotter>,
        options: &PrintOptions,
    ) -> bool {
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

/// Opens `path` with the system's default viewer. The program is reaped on a thread, so it
/// does not linger as a zombie.
fn open_file(path: &Path) -> Result<(), String> {
    let (program, args) = open_command(Platform::current(), path);
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
        let (program, args) = open_command(Platform::Mac, path);
        assert_eq!(program, "open");
        assert_eq!(args, [path.as_os_str()]);
        let (program, args) = open_command(Platform::Linux, path);
        assert_eq!(program, "xdg-open");
        assert_eq!(args, [path.as_os_str()]);
        let (program, args) = open_command(Platform::Windows, path);
        assert_eq!(program, "cmd");
        assert_eq!(
            args,
            [
                OsString::from("/C"),
                "start".into(),
                "".into(),
                path.as_os_str().to_owned()
            ]
        );
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
    fn options_map_to_a_layout_spec_and_are_cleaned() {
        let options = PrintOptions {
            paper: PaperChoice::Tabloid,
            landscape: false,
            scale: Some(50.0),
            dpi: 200,
        };
        let spec = options.spec();
        assert_eq!(spec.paper, Paper::Tabloid);
        assert!(!spec.landscape);
        assert_eq!((spec.scale, spec.dpi), (Some(50.0), 200));
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
