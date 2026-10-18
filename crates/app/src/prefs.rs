//! Window preferences kept between sessions in `~/.sketchup-alt/prefs.json`. They are window
//! chrome: never part of a model, never read or written by the API or in headless mode.

use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::editor::ImportOptions;
use crate::ui::DockLayout;
use crate::ui::theme::{ThemeChoice, UiScale};

/// What the prefs file keeps. Keys it does not know are ignored, and dropped at the next save;
/// a key missing (a file from before it) is its default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeChoice,
    /// How large the panels are drawn (Settings > Appearance). Any value that is not one of
    /// the four percents loads as 100.
    pub ui_scale: UiScale,
    /// The dock's width, split and tab.
    pub dock: DockLayout,
    /// Files opened or saved in the window, newest first, absolute, at most `RECENT_LEN`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<PathBuf>,
    /// The Import Options sheet's values when Import was last pressed.
    pub import: ImportOptions,
}

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

/// The largest prefs file read; anything larger is not one this app wrote.
const MAX_BYTES: u64 = 64 * 1024;

/// The prefs file for the home folder `home`.
pub fn path_in(home: &Path) -> PathBuf {
    home.join(crate::cli::FALLBACK_DIR).join("prefs.json")
}

/// The prefs at `path`: the defaults when there is no file, and the defaults with a warning
/// on stderr when it cannot be read or is not valid (it is replaced at the next change). The
/// warning never quotes the file. Dock values out of range are brought into it, and the
/// recent files are cleaned (see `cleaned`).
pub fn load(path: &Path) -> Prefs {
    let bytes = match read(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Prefs::default(),
        Err(e) => {
            eprintln!(
                "warning: cannot read {}: {e}; using the default preferences",
                path.display()
            );
            return Prefs::default();
        }
    };
    let prefs: Prefs = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        eprintln!(
            "warning: {} is not valid preferences (line {}, column {}); using the defaults",
            path.display(),
            e.line(),
            e.column()
        );
        Prefs::default()
    });
    Prefs {
        dock: prefs.dock.clamped(),
        recent: cleaned(prefs.recent),
        ..prefs
    }
}

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

/// Writes `prefs` to `path` (mode 0600), in a folder made or checked private like the rest of
/// `~/.sketchup-alt` (0700, not a link). The bytes go first to a new hidden file beside it
/// (`.prefs-<pid>-<n>.partial`), created only where nothing is, so they never go through a
/// link; it is renamed over `path` once written, or removed if the write fails.
pub fn save(path: &Path, prefs: &Prefs) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(|| io::Error::other("the preferences path has no folder"))?;
    crate::cli::ensure_private_dir(dir)?;
    let json = serde_json::to_vec_pretty(prefs).map_err(io::Error::other)?;
    let (partial, mut file) = loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let partial = dir.join(format!(".prefs-{}-{n}.partial", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&partial)
        {
            Ok(file) => break (partial, file),
            // Left by an earlier run with this process id.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let written = file
        .write_all(&json)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{DockLayout, DockTab};
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;

    /// A fresh folder standing in for HOME.
    fn home(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sketchup-alt-prefs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(load(&path).theme, ThemeChoice::Light);
        assert_eq!(load(&path).ui_scale, UiScale::P110);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"theme\": \"light\",\n  \"ui_scale\": 110,\n  \"dock\": {\n    \"width\": 280.0,\n    \"split\": 0.55,\n    \"tab\": \"entity_info\"\n  },\n  \"import\": {\n    \"units\": \"meters\",\n    \"as\": \"group\",\n    \"merge_coplanar\": true\n  }\n}"
        );
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
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
            import: ImportOptions::default(),
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
                import: ImportOptions::default(),
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
        assert_eq!(
            recent,
            [PathBuf::from("/a/1.skr"), PathBuf::from("/a/2.skr")]
        );
        let mut recent = Vec::new();
        for i in 1..=12 {
            remember_recent(&mut recent, Path::new(&format!("/a/{i}.skr")));
        }
        let want: Vec<PathBuf> = (3..=12)
            .rev()
            .map(|i| PathBuf::from(format!("/a/{i}.skr")))
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
        let mut list = vec!["rel.skr".to_string(), "/x/a.skr".into(), "/x/a.skr".into()];
        list.extend((0..12).map(|i| format!("/y/{i}.skr")));
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({ "recent": list })).unwrap(),
        )
        .unwrap();
        let mut want = vec![PathBuf::from("/x/a.skr")];
        want.extend((0..9).map(|i| PathBuf::from(format!("/y/{i}.skr"))));
        assert_eq!(load(&path).recent, want);
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
    fn m11d_prefs_load_with_defaults_for_the_new_keys() {
        let home = home("m11d");
        let path = path_in(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            b"{\"theme\": \"dark\", \"dock\": {\"width\": 300.0, \"split\": 0.5, \"tab\": \"materials\"}, \"recent\": [\"/a/house.skr\"]}",
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
        assert_eq!(prefs.recent, [PathBuf::from("/a/house.skr")]);
        assert_eq!(prefs.import, ImportOptions::default());
        assert_eq!(prefs.ui_scale, UiScale::P100);
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
