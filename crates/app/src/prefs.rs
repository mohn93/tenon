//! Window preferences kept between sessions in `~/.sketchup-alt/prefs.json`. They are window
//! chrome: never part of a model, never read or written by the API or in headless mode.

use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::ui::theme::ThemeChoice;

/// What the prefs file keeps. Keys it does not know are ignored, and dropped at the next save.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeChoice,
}

/// The largest prefs file read; anything larger is not one this app wrote.
const MAX_BYTES: u64 = 64 * 1024;

/// The prefs file for the home folder `home`.
pub fn path_in(home: &Path) -> PathBuf {
    home.join(crate::cli::FALLBACK_DIR).join("prefs.json")
}

/// The prefs at `path`: the defaults when there is no file, and the defaults with a warning
/// on stderr when it cannot be read or is not valid (it is replaced at the next change). The
/// warning never quotes the file.
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
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        eprintln!(
            "warning: {} is not valid preferences (line {}, column {}); using the defaults",
            path.display(),
            e.line(),
            e.column()
        );
        Prefs::default()
    })
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
            },
        )
        .unwrap();
        assert_eq!(load(&path).theme, ThemeChoice::Light);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"theme\": \"light\"\n}"
        );
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        save(
            &path,
            &Prefs {
                theme: ThemeChoice::Dark,
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
                    theme: ThemeChoice::Dark
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
                    theme: ThemeChoice::Dark
                }
            )
            .is_err()
        );
        assert!(!path.exists());
        std::fs::remove_dir_all(home).unwrap();
    }
}
