//! Window preferences kept between sessions in `~/.tenon/prefs.json`. They are window
//! chrome: never part of a model, never read or written by the API or in headless mode.

#[cfg(not(target_arch = "wasm32"))]
use std::io;
#[cfg(not(target_arch = "wasm32"))]
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::editor::{ImportOptions, PrintOptions};
use crate::navigation::TrackpadScroll;
use crate::ui::DockLayout;
use crate::ui::theme::{ThemeChoice, UiScale};

/// What the prefs file keeps. Keys it does not know are ignored, and dropped at the next save;
/// a key missing (a file from before it) is its default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeChoice,
    /// How large the panels are drawn (Settings > Appearance). Any value that is not one of
    /// the four percents loads as 100.
    pub ui_scale: UiScale,
    /// What a plain trackpad scroll does (Settings > Input).
    pub trackpad: TrackpadScroll,
    /// The dock's width, split and tab.
    pub dock: DockLayout,
    /// Files opened or saved in the window, newest first, absolute, at most `RECENT_LEN`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<PathBuf>,
    /// The Import Options sheet's values when Import was last pressed.
    pub import: ImportOptions,
    /// The Print sheet's values when Export PDF or Print was last pressed.
    pub print: PrintOptions,
    /// How often the open model is copied to the recovery folder, in minutes (0: never).
    /// Loaded at most `MAX_AUTOSAVE_MINUTES`.
    pub autosave_minutes: u32,
    /// The component library's folder (Settings > Files); None: not chosen. Loaded only when
    /// absolute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<PathBuf>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: ThemeChoice::default(),
            ui_scale: UiScale::default(),
            trackpad: TrackpadScroll::default(),
            dock: DockLayout::default(),
            recent: Vec::new(),
            import: ImportOptions::default(),
            print: PrintOptions::default(),
            autosave_minutes: DEFAULT_AUTOSAVE_MINUTES,
            library: None,
        }
    }
}

/// The autosave interval of a new prefs file, in minutes.
pub const DEFAULT_AUTOSAVE_MINUTES: u32 = 5;

/// The longest autosave interval, in minutes.
pub const MAX_AUTOSAVE_MINUTES: u32 = 60;

/// How many recent files the prefs keep.
pub const RECENT_LEN: usize = 10;

/// How many recent files the empty-model card lists.
pub const CARD_RECENT: usize = 5;

/// The first `n` entries of `recent` that are regular files now.
pub fn existing(recent: &[PathBuf], n: usize) -> Vec<PathBuf> {
    recent
        .iter()
        .filter(|p| p.is_file())
        .take(n)
        .cloned()
        .collect()
}

/// Puts `path` (made absolute) first in `recent`, dropping an equal entry and any past
/// `RECENT_LEN`. A path that cannot be made absolute is left out.
pub fn remember_recent(recent: &mut Vec<PathBuf>, path: &Path) {
    let Ok(path) = std::path::absolute(path) else {
        return;
    };
    recent.retain(|p| *p != path);
    recent.insert(0, path);
    recent.truncate(RECENT_LEN);
}

/// `recent` as `load` keeps it: absolute paths only, each once (the first kept), at most
/// `RECENT_LEN`.
fn cleaned(recent: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for path in recent {
        if path.is_absolute() && !kept.contains(&path) && kept.len() < RECENT_LEN {
            kept.push(path);
        }
    }
    kept
}

#[cfg(not(target_arch = "wasm32"))]
/// The largest prefs file read; anything larger is not one this app wrote.
const MAX_BYTES: u64 = 64 * 1024;

/// The prefs file for the home folder `home` (the macOS layout).
#[cfg(test)]
pub fn path_in(home: &Path) -> PathBuf {
    home.join(crate::cli::FALLBACK_DIR).join("prefs.json")
}

/// Where the window's preferences live: `path`, and an earlier location read once when `path`
/// does not exist yet.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq)]
pub struct PrefsFile {
    pub path: PathBuf,
    pub legacy: Option<PathBuf>,
}

/// The prefs of `file`: those at `file.path`, or, when nothing is there yet, those at the
/// legacy path. The legacy file is only read; the next save writes `file.path`, after which
/// it is no longer looked at.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_file(file: &PrefsFile) -> Prefs {
    let source = source_of(file);
    match read(source) {
        Ok(Some(bytes)) => parse(&bytes, &source.display().to_string()),
        Ok(None) => Prefs::default(),
        Err(e) => unreadable(source, &e),
    }
}

/// The file `file`'s prefs are read from: the legacy one while there is none at `path`.
#[cfg(not(target_arch = "wasm32"))]
fn source_of(file: &PrefsFile) -> &Path {
    let missing = matches!(
        std::fs::symlink_metadata(&file.path),
        Err(e) if e.kind() == io::ErrorKind::NotFound
    );
    match &file.legacy {
        Some(legacy) if missing => legacy,
        _ => &file.path,
    }
}

/// The text of the prefs file `file` names (see `load_file`), for a platform to hand to
/// `from_text`. None when there is none or it cannot be read (a warning goes to stderr then).
#[cfg(not(target_arch = "wasm32"))]
pub fn read_text(file: &PrefsFile) -> Option<String> {
    let source = source_of(file);
    match read(source) {
        Ok(Some(bytes)) => {
            // Not valid: the warning names the file, as `load`'s does, and the defaults apply.
            if serde_json::from_slice::<Prefs>(&bytes).is_err() {
                parse(&bytes, &source.display().to_string());
                return None;
            }
            String::from_utf8(bytes).ok()
        }
        Ok(None) => None,
        Err(e) => {
            unreadable(source, &e);
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn unreadable(path: &Path, e: &io::Error) -> Prefs {
    eprintln!(
        "warning: cannot read {}: {e}; using the default preferences",
        path.display()
    );
    Prefs::default()
}

/// The prefs at `path`: the defaults when there is no file, and the defaults with a warning
/// on stderr when it cannot be read or is not valid (it is replaced at the next change). The
/// warning never quotes the file. Dock values and the autosave interval out of range are
/// brought into it, and the recent files are cleaned (see `cleaned`).
#[cfg(not(target_arch = "wasm32"))]
pub fn load(path: &Path) -> Prefs {
    match read(path) {
        Ok(Some(bytes)) => parse(&bytes, &path.display().to_string()),
        Ok(None) => Prefs::default(),
        Err(e) => unreadable(path, &e),
    }
}

/// The prefs in `text` (what `to_text` wrote), checked as `load` checks a file's.
pub fn from_text(text: &str) -> Prefs {
    parse(text.as_bytes(), "the stored preferences")
}

/// The text the prefs are kept as.
pub fn to_text(prefs: &Prefs) -> String {
    serde_json::to_string_pretty(prefs).expect("preferences are serializable")
}

/// The prefs in `bytes`, which came from `what`: the defaults with a warning when they are
/// not valid, else normalized.
fn parse(bytes: &[u8], what: &str) -> Prefs {
    let prefs: Prefs = serde_json::from_slice(bytes).unwrap_or_else(|e| {
        eprintln!(
            "warning: {what} is not valid preferences (line {}, column {}); using the defaults",
            e.line(),
            e.column()
        );
        Prefs::default()
    });
    Prefs {
        dock: prefs.dock.clamped(),
        recent: cleaned(prefs.recent),
        print: prefs.print.cleaned(),
        autosave_minutes: prefs.autosave_minutes.min(MAX_AUTOSAVE_MINUTES),
        library: prefs
            .library
            .filter(|p| p.is_absolute() && !p.to_string_lossy().contains('\0')),
        ..prefs
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The bytes of the regular file at `path`, or None when there is nothing there. A link, a
/// folder, a pipe or a file over `MAX_BYTES` is an error, so reading never blocks or runs long.
fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let not_a_file = || io::Error::other("it is not a regular file");
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(meta) if !meta.file_type().is_file() => return Err(not_a_file()),
        Ok(_) => {}
    }
    let file = std::fs::File::open(path)?;
    // Checked again on what was opened, in case the name changed in between.
    if !file.metadata()?.is_file() {
        return Err(not_a_file());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::other(format!(
            "it is larger than {} KiB",
            MAX_BYTES / 1024
        )));
    }
    Ok(Some(bytes))
}

/// Writes `prefs` to `path` (mode 0600; on Windows owner-only), in a folder made or checked
/// private like the app's other private folders (0700, not a link; on Linux and Windows missing
/// parents are created). The bytes go
/// first to a new hidden file beside it (`.prefs-<pid>-<n>.partial`), created only where nothing
/// is, so they never go through a link; it is renamed over `path` once written, or removed if
/// the write fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(path: &Path, prefs: &Prefs) -> io::Result<()> {
    save_text(path, &to_text(prefs))
}

/// `save` for prefs already written as text (`to_text`).
#[cfg(not(target_arch = "wasm32"))]
pub fn save_text(path: &Path, json: &str) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(|| io::Error::other("the preferences path has no folder"))?;
    crate::cli::ensure_private_xdg_dir(crate::paths::Os::current(), dir)?;
    let (partial, mut file) = loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let partial = dir.join(format!(".prefs-{}-{n}.partial", std::process::id()));
        match create_private_file(&partial) {
            Ok(file) => break (partial, file),
            // Left by an earlier run with this process id.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let written = file
        .write_all(json.as_bytes())
        // On disk before the rename makes it the file.
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file);
            std::fs::rename(&partial, path)
        });
    match written {
        Ok(()) => {
            // The rename itself, best effort.
            if let Ok(dir) = std::fs::File::open(dir) {
                let _ = dir.sync_all();
            }
            Ok(())
        }
        Err(e) => {
            // The file this call created, and nothing else.
            let _ = std::fs::remove_file(&partial);
            Err(e)
        }
    }
}

/// Creates the file `path` for writing, failing when anything is there: mode 0600 on Unix, and
/// on Windows a DACL for the current user only.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_private_file(path: &Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(windows)]
    {
        session::windows::create_private_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{DockLayout, DockTab};
    use serde_json::json;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// A fresh folder standing in for HOME.
    fn home(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tenon-prefs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn old_prefs_are_read_once() {
        let home = home("legacy");
        let old = path_in(&home);
        let new = home.join(".config/tenon/prefs.json");
        std::fs::create_dir_all(home.join(".config")).unwrap();
        let file = PrefsFile {
            path: new.clone(),
            legacy: Some(old.clone()),
        };
        let kept = Prefs {
            recent: vec![PathBuf::from("/a/b.skr")],
            ..Prefs::default()
        };
        save(&old, &kept).unwrap();
        let old_bytes = std::fs::read(&old).unwrap();
        // The new file is missing: the old one is read.
        let mut loaded = load_file(&file);
        assert_eq!(loaded, kept);
        assert!(!new.exists(), "reading does not write");
        // The next save goes to the new place (made, with its missing parents, private).
        loaded.recent.push(PathBuf::from("/c.skr"));
        save(&new, &loaded).unwrap();
        assert_eq!(
            std::fs::read(&old).unwrap(),
            old_bytes,
            "old file untouched"
        );
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&new), 0o600);
        assert_eq!(mode(new.parent().unwrap()), 0o700);
        // Now the new file wins, even though the old one is still there.
        assert_eq!(load_file(&file), loaded);
        // An unreadable new file is not replaced by the old one.
        std::fs::write(&new, b"not json").unwrap();
        assert_eq!(load_file(&file), Prefs::default());
        // No legacy path: just the new file.
        std::fs::remove_file(&new).unwrap();
        let only_new = PrefsFile {
            path: new,
            legacy: None,
        };
        assert_eq!(load_file(&only_new), Prefs::default());
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn prefs_round_trip_in_a_private_folder() {
        let home = home("round-trip");
        let path = path_in(&home);
        assert_eq!(load(&path), Prefs::default(), "no file yet");
        save(
            &path,
            &Prefs {
                theme: ThemeChoice::Light,
                ui_scale: UiScale::P110,
                trackpad: TrackpadScroll::Zoom,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(load(&path).theme, ThemeChoice::Light);
        assert_eq!(load(&path).ui_scale, UiScale::P110);
        assert_eq!(load(&path).trackpad, TrackpadScroll::Zoom);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"theme\": \"light\",\n  \"ui_scale\": 110,\n  \"trackpad\": \"zoom\",\n  \"dock\": {\n    \"width\": 280.0,\n    \"split\": 0.55,\n    \"tab\": \"entity_info\"\n  },\n  \"import\": {\n    \"units\": \"meters\",\n    \"as\": \"group\",\n    \"merge_coplanar\": true\n  },\n  \"print\": {\n    \"paper\": \"a4\",\n    \"landscape\": true,\n    \"scale\": null,\n    \"dpi\": 150,\n    \"cut_list\": false\n  },\n  \"autosave_minutes\": 5\n}"
        );
        #[cfg(unix)]
        {
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().unwrap()), 0o700);
        }
        save(
            &path,
            &Prefs {
                theme: ThemeChoice::Dark,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(load(&path).theme, ThemeChoice::Dark, "replaced");
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["prefs.json"], "no temporary file is left");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_dock_and_a_theme_round_trip_together() {
        let home = home("dock-and-theme");
        let path = path_in(&home);
        let prefs = Prefs {
            theme: ThemeChoice::Dark,
            dock: DockLayout {
                width: 333.0,
                split: 0.3,
                tab: DockTab::Materials,
            },
            recent: Vec::new(),
            ui_scale: UiScale::P125,
            trackpad: TrackpadScroll::Zoom,
            import: ImportOptions::default(),
            print: PrintOptions::default(),
            autosave_minutes: 30,
            library: None,
        };
        assert_ne!(prefs.theme, ThemeChoice::default());
        assert_ne!(prefs.dock, DockLayout::default());
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path), prefs);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn dock_prefs_load_old_files_and_clamp_bad_values() {
        let home = home("dock");
        let path = path_in(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{\"theme\": \"dark\"}").unwrap();
        assert_eq!(
            load(&path),
            Prefs {
                theme: ThemeChoice::Dark,
                dock: DockLayout::default(),
                recent: Vec::new(),
                ui_scale: UiScale::P100,
                trackpad: TrackpadScroll::Orbit,
                import: ImportOptions::default(),
                print: PrintOptions::default(),
                autosave_minutes: 5,
                library: None,
            }
        );
        std::fs::write(
            &path,
            b"{\"dock\": {\"width\": 9000, \"split\": -3, \"tab\": \"materials\"}}",
        )
        .unwrap();
        assert_eq!(
            load(&path).dock,
            DockLayout {
                width: 420.0,
                split: 0.2,
                tab: DockTab::Materials
            }
        );
        std::fs::write(
            &path,
            b"{\"theme\": \"light\", \"dock\": {\"width\": \"wide\"}}",
        )
        .unwrap();
        assert_eq!(
            load(&path),
            Prefs::default(),
            "a wrong type is an invalid file"
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn bad_prefs_files_give_the_defaults() {
        let home = home("bad");
        let path = path_in(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bytes in [
            &b""[..],
            b"not json",
            b"{\"theme\": \"purple\"}",
            b"[1, 2]",
            b"{\"theme\": 3}",
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(
                load(&path),
                Prefs::default(),
                "{}",
                String::from_utf8_lossy(bytes)
            );
        }
        // Keys it does not know are ignored.
        std::fs::write(&path, b"{\"theme\": \"dark\", \"later\": 3}").unwrap();
        assert_eq!(load(&path).theme, ThemeChoice::Dark);
        // Something that is not a file.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(load(&path), Prefs::default());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn prefs_are_not_read_through_a_link_or_from_a_huge_file() {
        let home = home("odd-files");
        let path = path_in(&home);
        crate::cli::ensure_private_dir(path.parent().unwrap()).unwrap();
        let elsewhere = home.join("elsewhere.json");
        std::fs::write(&elsewhere, b"{\"theme\": \"light\"}").unwrap();
        std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
        assert_eq!(load(&path), Prefs::default(), "a link");
        std::fs::remove_file(&path).unwrap();
        let mut big = b"{\"theme\": \"light\", \"pad\": \"".to_vec();
        big.resize(MAX_BYTES as usize + 10, b'x');
        big.extend_from_slice(b"\"}");
        std::fs::write(&path, &big).unwrap();
        assert_eq!(load(&path), Prefs::default(), "too large");
        // Saving replaces the odd file with a good one.
        save(
            &path,
            &Prefs {
                theme: ThemeChoice::Light,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(load(&path).theme, ThemeChoice::Light);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn prefs_are_not_written_through_a_linked_folder() {
        let home = home("linked");
        let elsewhere = home.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.join(crate::cli::FALLBACK_DIR)).unwrap();
        let path = path_in(&home);
        assert!(
            save(
                &path,
                &Prefs {
                    theme: ThemeChoice::Dark,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_dir(&elsewhere).unwrap().count(),
            0,
            "target untouched"
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn prefs_are_not_written_into_an_open_folder() {
        let home = home("open");
        let path = path_in(&home);
        std::fs::create_dir(path.parent().unwrap()).unwrap();
        std::fs::set_permissions(
            path.parent().unwrap(),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert!(
            save(
                &path,
                &Prefs {
                    theme: ThemeChoice::Dark,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert!(!path.exists());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn recent_files_round_trip_newest_first() {
        let mut recent = Vec::new();
        for p in ["/a/1.skr", "/a/2.skr", "/a/1.skr"] {
            remember_recent(&mut recent, Path::new(p));
        }
        // `absolute` also gives a rooted path its drive on Windows.
        let abs = |p: &str| std::path::absolute(p).unwrap();
        assert_eq!(recent, [abs("/a/1.skr"), abs("/a/2.skr")]);
        let mut recent = Vec::new();
        for i in 1..=12 {
            remember_recent(&mut recent, Path::new(&format!("/a/{i}.skr")));
        }
        let want: Vec<PathBuf> = (3..=12)
            .rev()
            .map(|i| abs(&format!("/a/{i}.skr")))
            .collect();
        assert_eq!(recent, want, "the last ten, newest first");
        let home = home("recent");
        let path = path_in(&home);
        let prefs = Prefs {
            recent,
            ..Default::default()
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path), prefs);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn recent_files_load_clean() {
        let home = home("recent-clean");
        let path = path_in(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A rooted path with no drive is not absolute on Windows.
        let drive = if cfg!(windows) { "C:" } else { "" };
        let mut list = vec![
            "rel.skr".to_string(),
            format!("{drive}/x/a.skr"),
            format!("{drive}/x/a.skr"),
        ];
        list.extend((0..12).map(|i| format!("{drive}/y/{i}.skr")));
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({ "recent": list })).unwrap(),
        )
        .unwrap();
        let mut want = vec![PathBuf::from(format!("{drive}/x/a.skr"))];
        want.extend((0..9).map(|i| PathBuf::from(format!("{drive}/y/{i}.skr"))));
        assert_eq!(load(&path).recent, want);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn the_library_folder_round_trips_and_loads_only_when_absolute() {
        let home = home("library");
        let path = path_in(&home);
        let drive = if cfg!(windows) { "C:" } else { "" };
        let folder = PathBuf::from(format!("{drive}/lib/parts"));
        let prefs = Prefs {
            library: Some(folder.clone()),
            ..Default::default()
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path).library, Some(folder));
        // Not written when unset, and a relative folder is dropped on load.
        save(&path, &Prefs::default()).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("library"));
        std::fs::write(&path, br#"{"library": "parts"}"#).unwrap();
        assert_eq!(load(&path).library, None);
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(load(&path).library, None);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn old_prefs_files_have_no_recent_files() {
        let home = home("recent-old");
        let path = path_in(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{\"theme\": \"dark\"}").unwrap();
        assert!(load(&path).recent.is_empty());
        // A wrong type is an invalid file, as for any key.
        std::fs::write(&path, b"{\"theme\": \"dark\", \"recent\": \"/a.skr\"}").unwrap();
        assert_eq!(load(&path), Prefs::default());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn import_options_round_trip() {
        use crate::editor::{ImportAs, ImportUnits, UpAxis};
        let home = home("import");
        let path = path_in(&home);
        let options = ImportOptions {
            units: ImportUnits::Inches,
            up: None,
            place: ImportAs::Component,
            merge_coplanar: false,
        };
        let prefs = Prefs {
            import: options,
            ..Default::default()
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path).import, options);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["import"]["as"], "component");
        assert!(json["import"].get("up").is_none(), "{json}");
        let options = ImportOptions {
            up: Some(UpAxis::Z),
            ..options
        };
        save(
            &path,
            &Prefs {
                import: options,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(load(&path).import, options);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn print_options_round_trip_and_load_into_range() {
        use crate::editor::PaperChoice;
        let home = home("print");
        let path = path_in(&home);
        let options = PrintOptions {
            paper: PaperChoice::Tabloid,
            landscape: false,
            scale: Some(200.0),
            dpi: 300,
            cut_list: true,
        };
        save(
            &path,
            &Prefs {
                print: options,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(load(&path).print, options);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["print"]["paper"], "tabloid");
        // A file from before printing has the defaults; wild values are brought into range.
        std::fs::write(&path, b"{\"theme\": \"dark\"}").unwrap();
        assert_eq!(load(&path).print, PrintOptions::default());
        std::fs::write(
            &path,
            b"{\"print\": {\"dpi\": 9000, \"scale\": -2.0, \"paper\": \"legal\"}}",
        )
        .unwrap();
        let loaded = load(&path).print;
        assert_eq!((loaded.dpi, loaded.scale), (300, None));
        assert_eq!(loaded.paper, PaperChoice::Legal);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn m11d_prefs_load_with_defaults_for_the_new_keys() {
        let home = home("m11d");
        // A rooted path with no drive is not absolute on Windows.
        let house = format!("{}/a/house.skr", if cfg!(windows) { "C:" } else { "" });
        let path = path_in(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "theme": "dark",
                "dock": {"width": 300.0, "split": 0.5, "tab": "materials"},
                "recent": [house],
            }))
            .unwrap(),
        )
        .unwrap();
        let prefs = load(&path);
        assert_eq!(prefs.theme, ThemeChoice::Dark);
        assert_eq!(
            prefs.dock,
            DockLayout {
                width: 300.0,
                split: 0.5,
                tab: DockTab::Materials
            }
        );
        assert_eq!(prefs.recent, [PathBuf::from(&house)]);
        assert_eq!(prefs.import, ImportOptions::default());
        assert_eq!(prefs.ui_scale, UiScale::P100);
        assert_eq!(prefs.trackpad, TrackpadScroll::Orbit);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn an_unknown_ui_scale_loads_as_100() {
        let home = home("ui-scale");
        let path = path_in(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let load_json = |json: &str| {
            std::fs::write(&path, json).unwrap();
            load(&path)
        };
        assert_eq!(load_json("{\"ui_scale\": 125}").ui_scale, UiScale::P125);
        assert_eq!(load_json("{}").ui_scale, UiScale::P100, "an older file");
        // Any other value, of any type, is 100 and leaves the rest of the file alone.
        for odd in [
            "300",
            "0",
            "-90",
            "110.0",
            "\"big\"",
            "null",
            "[110]",
            "{\"p\": 110}",
        ] {
            let prefs = load_json(&format!("{{\"theme\": \"dark\", \"ui_scale\": {odd}}}"));
            assert_eq!(prefs.ui_scale, UiScale::P100, "{odd}");
            assert_eq!(prefs.theme, ThemeChoice::Dark, "{odd}");
        }
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_bad_trackpad_value_is_an_invalid_file() {
        let home = home("trackpad");
        let path = path_in(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let load_json = |json: &[u8]| {
            std::fs::write(&path, json).unwrap();
            load(&path)
        };
        assert_eq!(
            load_json(b"{\"trackpad\": \"zoom\"}").trackpad,
            TrackpadScroll::Zoom
        );
        assert_eq!(
            load_json(b"{\"trackpad\": \"orbit\"}").trackpad,
            TrackpadScroll::Orbit
        );
        assert_eq!(
            load_json(b"{\"theme\": \"dark\"}").trackpad,
            TrackpadScroll::Orbit,
            "an older file"
        );
        for bad in [
            &b"{\"theme\": \"dark\", \"trackpad\": \"sideways\"}"[..],
            b"{\"theme\": \"dark\", \"trackpad\": 1}",
        ] {
            assert_eq!(
                load_json(bad),
                Prefs::default(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn autosave_minutes_default_to_5_and_load_clamped() {
        assert_eq!(Prefs::default().autosave_minutes, 5);
        let home = home("autosave");
        let path = path_in(&home);
        // Makes the folder, private.
        save(&path, &Prefs::default()).unwrap();
        let load_json = |json: &str| {
            std::fs::write(&path, json).unwrap();
            load(&path)
        };
        assert_eq!(
            load_json("{\"theme\": \"dark\"}").autosave_minutes,
            5,
            "an older file"
        );
        assert_eq!(
            load_json("{\"autosave_minutes\": 0}").autosave_minutes,
            0,
            "off"
        );
        assert_eq!(load_json("{\"autosave_minutes\": 60}").autosave_minutes, 60);
        assert_eq!(
            load_json("{\"autosave_minutes\": 500}").autosave_minutes,
            60
        );
        // Not a count of minutes: the file is not valid, so all defaults.
        assert_eq!(load_json("{\"autosave_minutes\": -1}"), Prefs::default());
        let prefs = Prefs {
            autosave_minutes: 12,
            ..Prefs::default()
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path), prefs);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_relative_path_is_remembered_absolute() {
        let mut recent = Vec::new();
        remember_recent(&mut recent, Path::new("x.skr"));
        assert_eq!(recent, [std::path::absolute("x.skr").unwrap()]);
        assert!(recent[0].is_absolute());
    }

    #[test]
    fn the_card_lists_only_files_that_exist() {
        let home = home("recent-existing");
        let (a, b) = (home.join("a.skr"), home.join("b.skr"));
        std::fs::write(&a, b"").unwrap();
        std::fs::write(&b, b"").unwrap();
        // A folder is not a file either.
        let list = vec![home.join("gone.skr"), b.clone(), home.clone(), a.clone()];
        assert_eq!(existing(&list, 5), [b.clone(), a]);
        assert_eq!(existing(&list, 1), [b]);
        std::fs::remove_dir_all(home).unwrap();
    }
}
