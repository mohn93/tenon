//! The recovery folder: the window's autosaves and the work agents discarded, each model file
//! with a sidecar note (`<model>.json`) saying what it is, which session wrote it, the file it
//! was a copy of and when. Each window process holds an OS lock on its own
//! `session-<id>.lock` while it runs; an entry whose session lock is gone or can be taken
//! belongs to a process that ended without cleaning up (a crash), and is offered back.

// Until the window's autosave and Recover Models window use them (M16b task 2).
#![cfg_attr(not(test), allow(dead_code))]

use std::cell::RefCell;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use session::Document;

/// What a recovery entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The window's periodic copy of its model.
    Autosave,
    /// Unsaved work an agent's `new`, `open` or `quit` discarded.
    Discarded,
}

/// A model in the recovery folder another session left behind.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: Kind,
    /// The name the model went by: its file's stem, or "Untitled".
    pub name: String,
    /// The file the model was opened from or saved to.
    pub original: Option<PathBuf>,
    /// When the copy was written.
    pub saved: SystemTime,
    /// The copy.
    pub model: PathBuf,
    /// Why the entry cannot be opened (it can only be discarded).
    pub error: Option<String>,
}

/// This process's hold on the recovery folder: its lock, and the autosave it last wrote.
pub struct Session {
    dir: PathBuf,
    id: String,
    lock_path: PathBuf,
    /// Locked for the session's life: other processes see the session live.
    lock: File,
    /// This session's autosave model, once written or adopted.
    autosave: RefCell<Option<PathBuf>>,
}

/// The largest sidecar read; anything larger is not one this app wrote.
const MAX_SIDECAR_BYTES: u64 = 16 * 1024;

/// The end of an autosave model's file name.
const AUTOSAVE_SUFFIX: &str = ".autosave.skr";

/// What a sidecar holds.
#[derive(Debug, Serialize, Deserialize)]
struct Sidecar {
    kind: Kind,
    /// The id of the session that wrote the entry.
    session: String,
    /// The file the model was a copy of (left out when it had none, or its path is not
    /// Unicode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original: Option<String>,
    /// When the model was written, in seconds since the Unix epoch.
    saved: u64,
}

/// Makes sure the recovery folder `dir` exists and is private, as the app's other private
/// folders (0700, not a link; the `~/.sketchup-alt` fallback folder too, and on Linux and
/// Windows missing parents are created).
pub fn ensure_folder(dir: &Path) -> io::Result<()> {
    if let Some(parent) = crate::cli::private_dir_for(dir) {
        crate::cli::ensure_private_dir(parent)?;
    }
    crate::cli::ensure_private_xdg_dir(crate::paths::Os::current(), dir)
}

/// The stem recovery files of the model at `open_file` (or an unsaved one) are named with:
/// the file's stem with characters other than letters, digits, `-`, `_` and spaces made `_`,
/// or "untitled".
pub fn file_stem(open_file: Option<&Path>) -> String {
    open_file
        .and_then(Path::file_stem)
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "untitled".into())
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn lock_name(id: &str) -> String {
    format!("session-{id}.lock")
}

/// The sidecar of the model at `model`.
fn sidecar_path(model: &Path) -> PathBuf {
    let mut path = model.as_os_str().to_owned();
    path.push(".json");
    PathBuf::from(path)
}

/// Whether `id` is one a session could have: digits in groups joined by `-`.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// Writes the sidecar of `model`.
fn write_sidecar(
    model: &Path,
    kind: Kind,
    session: &str,
    original: Option<&Path>,
    saved: SystemTime,
) -> Result<(), String> {
    let sidecar = Sidecar {
        kind,
        session: session.to_string(),
        original: original.and_then(Path::to_str).map(str::to_string),
        saved: unix_secs(saved),
    };
    let json = serde_json::to_vec_pretty(&sidecar).map_err(|e| e.to_string())?;
    session::file::replace_private(&sidecar_path(model), &json)
}

/// Removes the entry whose model is `model`: its sidecar first, so no entry is left pointing
/// at a missing model.
fn remove_entry(model: &Path) -> io::Result<()> {
    let gone = |r: io::Result<()>| match r {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    };
    gone(std::fs::remove_file(sidecar_path(model)))?;
    gone(std::fs::remove_file(model))
}

impl Session {
    /// Starts this process's session in the recovery folder `dir` (made or checked private):
    /// creates its lock file `session-<pid>-<unix secs>.lock` (`-2`, `-3` added for the
    /// process's later sessions, or when taken) and holds an exclusive lock on it until
    /// dropped.
    pub fn start(dir: &Path) -> io::Result<Session> {
        ensure_folder(dir)?;
        // Sessions this process started before (tests start several) keep their ids even
        // once their lock files are gone.
        static STARTED: AtomicU32 = AtomicU32::new(1);
        let base = format!("{}-{}", std::process::id(), unix_secs(SystemTime::now()));
        let mut n = STARTED.fetch_add(1, Ordering::Relaxed);
        loop {
            let id = if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            let lock_path = dir.join(lock_name(&id));
            match crate::prefs::create_private_file(&lock_path) {
                Ok(lock) => {
                    if let Err(e) = lock.try_lock() {
                        let _ = std::fs::remove_file(&lock_path);
                        return Err(match e {
                            std::fs::TryLockError::Error(e) => e,
                            std::fs::TryLockError::WouldBlock => {
                                io::Error::other("the session lock is held elsewhere")
                            }
                        });
                    }
                    return Ok(Session {
                        dir: dir.to_path_buf(),
                        id,
                        lock_path,
                        lock,
                        autosave: RefCell::new(None),
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    n = STARTED.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// This session's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The autosave model's path for a model at `original`.
    fn autosave_path(&self, original: Option<&Path>) -> PathBuf {
        self.dir.join(format!(
            "{}-{}{AUTOSAVE_SUFFIX}",
            file_stem(original),
            self.id
        ))
    }

    /// Copies `doc` (whose file is `original`) to this session's autosave, replacing the one
    /// before in place (private, written whole before it replaces anything), then its sidecar.
    /// The folder is checked first.
    pub fn write_autosave(&self, doc: &Document, original: Option<&Path>) -> Result<(), String> {
        ensure_folder(&self.dir).map_err(|e| e.to_string())?;
        let path = self.autosave_path(original);
        doc.write_copy(&path).map_err(|e| e.message)?;
        let previous = self.autosave.replace(Some(path.clone()));
        let written = write_sidecar(&path, Kind::Autosave, &self.id, original, SystemTime::now());
        match previous.filter(|p| *p != path) {
            // The model was renamed: the old entry goes once the new one is whole.
            Some(old) if written.is_ok() => {
                let _ = remove_entry(&old);
            }
            // Keep the old entry; the new model without its sidecar is no entry.
            Some(old) => {
                let _ = std::fs::remove_file(&path);
                self.autosave.replace(Some(old));
            }
            None => {}
        }
        written
    }

    /// Removes this session's autosave, if there is one.
    pub fn clear_autosave(&self) {
        if let Some(path) = self.autosave.take() {
            let _ = remove_entry(&path);
        }
    }

    /// Writes the sidecar of a recovery file an agent's discard was kept in (`path`, a copy
    /// of the model at `original`). Best effort: without it the file is still there, but not
    /// listed.
    pub fn note_discarded(&self, path: &Path, original: Option<&Path>) {
        if let Err(e) = write_sidecar(path, Kind::Discarded, &self.id, original, SystemTime::now())
        {
            eprintln!(
                "warning: cannot note the recovery file {}: {e}",
                path.display()
            );
        }
    }

    /// Makes `entry` (another session's, from `list`) this session's autosave: its model is
    /// renamed under this session's id, with a new sidecar, and the old sidecar removed.
    /// Returns the model's new path. This session's previous autosave is removed.
    pub fn adopt(&self, entry: &Entry) -> io::Result<PathBuf> {
        if entry.model.parent() != Some(self.dir.as_path()) {
            return Err(io::Error::other("the entry is not in the recovery folder"));
        }
        let target = self.autosave_path(entry.original.as_deref());
        std::fs::rename(&entry.model, &target)?;
        if let Err(e) = write_sidecar(
            &target,
            Kind::Autosave,
            &self.id,
            entry.original.as_deref(),
            entry.saved,
        ) {
            let _ = std::fs::rename(&target, &entry.model);
            return Err(io::Error::other(e));
        }
        let _ = std::fs::remove_file(sidecar_path(&entry.model));
        if let Some(old) = self.autosave.replace(Some(target.clone()))
            && old != target
        {
            let _ = remove_entry(&old);
        }
        Ok(target)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
        let _ = self.lock.unlock();
    }
}

/// The metadata of `path` when it is a regular file (not a link) only the user may read.
fn private_file(path: &Path) -> Result<std::fs::Metadata, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            "the file is missing".to_string()
        } else {
            e.to_string()
        }
    })?;
    if !meta.file_type().is_file() || session::fspath::is_link(&meta) {
        return Err("it is not a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("it is accessible to other users".into());
        }
    }
    Ok(meta)
}

/// Opens the regular file `path` checked by `private_file`, checking again on what was opened
/// in case the name changed in between.
fn open_checked(path: &Path, checked: &std::fs::Metadata) -> io::Result<File> {
    let file = File::open(path)?;
    let opened = file.metadata()?;
    let same = opened.is_file() && {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            (opened.dev(), opened.ino()) == (checked.dev(), checked.ino())
        }
        #[cfg(not(unix))]
        {
            let _ = checked;
            true
        }
    };
    if !same {
        return Err(io::Error::other("the file changed while it was opened"));
    }
    Ok(file)
}

/// Reads the sidecar at `path` (checked by `private_file`).
fn read_sidecar(path: &Path, meta: &std::fs::Metadata) -> Result<Sidecar, String> {
    use std::io::Read;
    let file = open_checked(path, meta).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_SIDECAR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        return Err("it is too large".into());
    }
    let sidecar: Sidecar =
        serde_json::from_slice(&bytes).map_err(|_| "it is not valid".to_string())?;
    if !valid_id(&sidecar.session) {
        return Err("it names no valid session".into());
    }
    Ok(sidecar)
}

/// Whether the session `id` is running: its lock file in `dir` is there and locked. A lock
/// file that is not a regular file is no session's; one that cannot be opened or tested is
/// taken as live, so nothing in use is offered.
fn is_live(dir: &Path, id: &str) -> bool {
    let path = dir.join(lock_name(id));
    let meta = match std::fs::symlink_metadata(&path) {
        Err(e) => return e.kind() != io::ErrorKind::NotFound,
        Ok(meta) => meta,
    };
    if !meta.file_type().is_file() || session::fspath::is_link(&meta) {
        return false;
    }
    let Ok(file) = open_checked(&path, &meta) else {
        return true;
    };
    match file.try_lock() {
        // Unlocked when `file` closes.
        Ok(()) => false,
        Err(_) => true,
    }
}

/// The entries in the recovery folder `dir` whose session has ended, newest first:
/// autosaves, and with `include_discarded` the work agents discarded. `own`'s entries and
/// those of sessions still running are left out. Sidecars that are links or open to other
/// users are skipped; an entry whose sidecar cannot be read, or whose model is missing, a
/// link or open to other users, is listed with an `error` (it can only be discarded). Lock
/// files of ended sessions with no entries left are removed.
pub fn list(dir: &Path, own: &Session, include_discarded: bool) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut locks = Vec::new();
    let mut sidecars = Vec::new();
    for item in read.flatten() {
        let name = item.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(id) = name
            .strip_prefix("session-")
            .and_then(|n| n.strip_suffix(".lock"))
        {
            if valid_id(id) {
                locks.push(id.to_string());
            }
        } else if name.ends_with(".skr.json") {
            sidecars.push(item.path());
        }
    }
    let mut live = std::collections::HashMap::new();
    let mut is_live = |id: &str| {
        *live
            .entry(id.to_string())
            .or_insert_with(|| is_live(dir, id))
    };
    let mut with_entries = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for side in sidecars {
        let Ok(meta) = private_file(&side) else {
            continue;
        };
        let model = side.with_extension("");
        let entry = match read_sidecar(&side, &meta) {
            Ok(sidecar) => {
                with_entries.insert(sidecar.session.clone());
                if sidecar.session == own.id || is_live(&sidecar.session) {
                    continue;
                }
                let original = sidecar.original.map(PathBuf::from);
                Entry {
                    kind: sidecar.kind,
                    name: original
                        .as_deref()
                        .and_then(Path::file_stem)
                        .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned()),
                    original,
                    saved: UNIX_EPOCH
                        .checked_add(Duration::from_secs(sidecar.saved))
                        .unwrap_or(UNIX_EPOCH),
                    error: private_file(&model)
                        .err()
                        .map(|e| format!("the model file cannot be used: {e}")),
                    model,
                }
            }
            Err(e) => {
                let file_name = model.file_name().map(|n| n.to_string_lossy().into_owned());
                let kind = if file_name
                    .as_deref()
                    .is_some_and(|n| n.ends_with(AUTOSAVE_SUFFIX))
                {
                    Kind::Autosave
                } else {
                    Kind::Discarded
                };
                Entry {
                    kind,
                    name: file_name.unwrap_or_default(),
                    original: None,
                    saved: meta.modified().unwrap_or(UNIX_EPOCH),
                    error: Some(format!("the recovery note cannot be read: {e}")),
                    model,
                }
            }
        };
        if entry.kind == Kind::Autosave || include_discarded {
            entries.push(entry);
        }
    }
    for id in locks {
        if id != own.id && !with_entries.contains(&id) && !is_live(&id) {
            let _ = std::fs::remove_file(dir.join(lock_name(&id)));
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.saved));
    entries
}

/// Removes `entry`'s files (its sidecar, then its model; a link is removed, not followed).
pub fn discard(entry: &Entry) -> io::Result<()> {
    remove_entry(&entry.model)
}

/// Whether an autosave should run now: autosave is on (`interval` not zero), the document is
/// `modified`, its `revision` is not the one last autosaved, no tool operation is in progress
/// (`busy`), and `interval` has passed since `since` (the later of the last autosave and the
/// first unsaved change).
pub fn autosave_due(
    now: Instant,
    since: Instant,
    interval: Duration,
    revision: u64,
    saved_revision: Option<u64>,
    modified: bool,
    busy: bool,
) -> bool {
    !interval.is_zero()
        && modified
        && !busy
        && saved_revision != Some(revision)
        && now.saturating_duration_since(since) >= interval
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use glam::DVec2;
    use serde_json::Value;
    use session::Command;

    use super::*;

    /// A fresh private folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sketchup-alt-recovery-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A document with one edge, modified.
    fn doc() -> Document {
        let mut doc = Document::new(DVec2::new(800.0, 600.0));
        doc.execute(Command::AddEdge {
            start: [0.0; 3],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        doc
    }

    /// Ends `session` as a crash would: the lock released, its file and entries left.
    fn crash(session: Session) {
        session.lock.unlock().unwrap();
        std::mem::forget(session);
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn sidecar(model: &Path) -> Value {
        let mut path = model.as_os_str().to_owned();
        path.push(".json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn opens(path: &Path) -> bool {
        let mut d = Document::new(DVec2::new(800.0, 600.0));
        d.execute(Command::Open {
            path: path.display().to_string(),
            discard: true,
        })
        .is_ok()
    }

    #[test]
    fn a_session_holds_its_lock_until_dropped() {
        let dir = folder("lock");
        let a = Session::start(&dir).unwrap();
        let lock = dir.join(format!("session-{}.lock", a.id()));
        assert!(a.id().starts_with(&format!("{}-", std::process::id())));
        let other = File::open(&lock).unwrap();
        assert!(matches!(
            other.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        // A second session in the same folder gets its own id and lock.
        let b = Session::start(&dir).unwrap();
        assert_ne!(a.id(), b.id());
        drop(a);
        assert!(!lock.exists());
        other.try_lock().unwrap();
        drop(b);
        assert!(names(&dir).is_empty(), "{:?}", names(&dir));
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosaves_are_overwritten_in_place_with_a_sidecar() {
        let dir = folder("overwrite");
        let s = Session::start(&dir).unwrap();
        let mut d = doc();
        let original = Path::new("/x/My house?.skr");
        s.write_autosave(&d, Some(original)).unwrap();
        d.execute(Command::AddEdge {
            start: [0.0; 3],
            end: [0.0, 1.0, 0.0],
        })
        .unwrap();
        s.write_autosave(&d, Some(original)).unwrap();
        let model = dir.join(format!("My house_-{}.autosave.skr", s.id()));
        let lock = format!("session-{}.lock", s.id());
        let model_name = model.file_name().unwrap().to_string_lossy().into_owned();
        let mut expected = vec![model_name.clone(), format!("{model_name}.json"), lock];
        expected.sort();
        assert_eq!(names(&dir), expected);
        let note = sidecar(&model);
        assert_eq!(note["kind"], "autosave");
        assert_eq!(note["session"], s.id());
        assert_eq!(note["original"], "/x/My house?.skr");
        assert!(note["saved"].as_u64().unwrap() > 1_700_000_000);
        let mut back = Document::new(DVec2::new(800.0, 600.0));
        back.execute(Command::Open {
            path: model.display().to_string(),
            discard: false,
        })
        .unwrap();
        assert_eq!(back.model().edges().count(), 2);
        #[cfg(unix)]
        for p in [model.clone(), dir.join(format!("{model_name}.json"))] {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{}", p.display());
        }
        // An untitled model, after a clean-up, is "untitled".
        s.clear_autosave();
        assert_eq!(names(&dir), [format!("session-{}.lock", s.id())]);
        s.write_autosave(&d, None).unwrap();
        assert!(
            dir.join(format!("untitled-{}.autosave.skr", s.id()))
                .is_file()
        );
        // A new name replaces the old pair: one autosave per session.
        s.write_autosave(&d, Some(Path::new("/y/barn.skr")))
            .unwrap();
        assert_eq!(names(&dir).len(), 3, "{:?}", names(&dir));
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_sessions_never_list_each_other_until_one_crashes() {
        let dir = folder("two");
        let a = Session::start(&dir).unwrap();
        let b = Session::start(&dir).unwrap();
        a.write_autosave(&doc(), Some(Path::new("/x/house.skr")))
            .unwrap();
        b.write_autosave(&doc(), None).unwrap();
        assert!(list(&dir, &a, true).is_empty(), "own and live");
        assert!(list(&dir, &b, true).is_empty(), "own and live");
        crash(a);
        let found = list(&dir, &b, false);
        assert_eq!(found.len(), 1, "{found:?}");
        let e = &found[0];
        assert_eq!(e.kind, Kind::Autosave);
        assert_eq!(e.name, "house");
        assert_eq!(e.original.as_deref(), Some(Path::new("/x/house.skr")));
        assert_eq!(e.error, None);
        assert!(e.saved > UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        assert!(opens(&e.model));
        // The crashed session's lock file stays while it has entries, then goes.
        let locks = |dir: &Path| {
            names(dir)
                .into_iter()
                .filter(|n| n.starts_with("session-"))
                .count()
        };
        assert_eq!(locks(&dir), 2);
        discard(e).unwrap();
        assert!(list(&dir, &b, true).is_empty());
        assert_eq!(locks(&dir), 1);
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_dropped_without_cleaning_up_lists_as_crashed() {
        let dir = folder("dropped");
        let a = Session::start(&dir).unwrap();
        a.write_autosave(&doc(), None).unwrap();
        drop(a);
        let b = Session::start(&dir).unwrap();
        let found = list(&dir, &b, false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Untitled");
        assert_eq!(found[0].original, None);
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discarded_files_are_listed_only_when_asked() {
        let dir = folder("discarded");
        let a = Session::start(&dir).unwrap();
        let kept = dir.join("house-1700000000.skr");
        doc().write_copy(&kept).unwrap();
        a.note_discarded(&kept, Some(Path::new("/x/house.skr")));
        assert_eq!(sidecar(&kept)["kind"], "discarded");
        assert_eq!(sidecar(&kept)["session"], a.id());
        // Clearing the autosave never touches discarded files.
        a.clear_autosave();
        drop(a);
        let b = Session::start(&dir).unwrap();
        assert!(list(&dir, &b, false).is_empty());
        let found = list(&dir, &b, true);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].kind, found[0].model.clone()),
            (Kind::Discarded, kept)
        );
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_adopted_entry_becomes_this_sessions_autosave() {
        let dir = folder("adopt");
        let a = Session::start(&dir).unwrap();
        a.write_autosave(&doc(), Some(Path::new("/x/house.skr")))
            .unwrap();
        crash(a);
        let b = Session::start(&dir).unwrap();
        let entry = list(&dir, &b, false).remove(0);
        let model = b.adopt(&entry).unwrap();
        assert_eq!(model, dir.join(format!("house-{}.autosave.skr", b.id())));
        assert!(!entry.model.exists());
        assert_eq!(sidecar(&model)["session"], b.id());
        assert_eq!(sidecar(&model)["original"], "/x/house.skr");
        assert!(opens(&model));
        assert!(list(&dir, &b, true).is_empty(), "now our own");
        // Crashing again offers it again.
        crash(b);
        let c = Session::start(&dir).unwrap();
        let again = list(&dir, &c, false);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].model, model);
        // Adopted then saved: the clean-up removes the recovered files.
        let adopted = c.adopt(&again[0]).unwrap();
        c.clear_autosave();
        assert!(!adopted.exists());
        assert!(list(&dir, &c, true).is_empty());
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_entries_are_listed_with_an_error_and_can_be_discarded() {
        let dir = folder("broken");
        let a = Session::start(&dir).unwrap();
        a.write_autosave(&doc(), None).unwrap();
        let model = dir.join(format!("untitled-{}.autosave.skr", a.id()));
        crash(a);
        let b = Session::start(&dir).unwrap();
        // The model is gone.
        std::fs::remove_file(&model).unwrap();
        let found = list(&dir, &b, false);
        assert_eq!(found.len(), 1);
        assert!(found[0].error.as_deref().unwrap().contains("missing"));
        discard(&found[0]).unwrap();
        // A sidecar that is not valid.
        let bad = dir.join("odd.autosave.skr.json");
        session::file::replace_private(&bad, b"{not json").unwrap();
        let found = list(&dir, &b, false);
        assert_eq!(found.len(), 1);
        assert!(found[0].error.is_some());
        assert_eq!(found[0].kind, Kind::Autosave);
        discard(&found[0]).unwrap();
        assert!(!bad.exists());
        // A time no clock reaches.
        let late = dir.join("late.autosave.skr.json");
        let note = format!(
            "{{\"kind\": \"autosave\", \"session\": \"1-2\", \"saved\": {}}}",
            u64::MAX
        );
        session::file::replace_private(&late, note.as_bytes()).unwrap();
        let found = list(&dir, &b, false);
        assert_eq!(found.len(), 1);
        discard(&found[0]).unwrap();
        // A partial file is never an entry.
        std::fs::write(dir.join(".sketchup-alt-1-0.partial"), b"x").unwrap();
        assert!(list(&dir, &b, true).is_empty());
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn links_and_files_open_to_others_are_not_listed() {
        let dir = folder("links");
        let a = Session::start(&dir).unwrap();
        a.write_autosave(&doc(), None).unwrap();
        let model = dir.join(format!("untitled-{}.autosave.skr", a.id()));
        let note = dir.join(format!("untitled-{}.autosave.skr.json", a.id()));
        crash(a);
        let b = Session::start(&dir).unwrap();
        // The sidecar open to others: not listed.
        std::fs::set_permissions(&note, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(list(&dir, &b, true).is_empty());
        std::fs::set_permissions(&note, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(list(&dir, &b, true).len(), 1);
        // A sidecar that is a link: not listed, not followed.
        let outside = folder("links-outside");
        std::fs::create_dir_all(&outside).unwrap();
        let target = outside.join("note.json");
        std::fs::rename(&note, &target).unwrap();
        std::os::unix::fs::symlink(&target, &note).unwrap();
        assert!(list(&dir, &b, true).is_empty());
        std::fs::remove_file(&note).unwrap();
        std::fs::rename(&target, &note).unwrap();
        // A model that is a link, or open to others: listed with an error, never opened.
        std::fs::set_permissions(&model, std::fs::Permissions::from_mode(0o644)).unwrap();
        let found = list(&dir, &b, true);
        assert!(found[0].error.as_deref().unwrap().contains("other users"));
        let real = outside.join("model.skr");
        std::fs::rename(&model, &real).unwrap();
        std::os::unix::fs::symlink(&real, &model).unwrap();
        let found = list(&dir, &b, true);
        assert!(
            found[0]
                .error
                .as_deref()
                .unwrap()
                .contains("not a regular file")
        );
        // Discarding removes the link, not what it points to.
        discard(&found[0]).unwrap();
        assert!(real.exists());
        assert!(std::fs::symlink_metadata(&model).is_err());
        drop(b);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_open_to_others_is_refused() {
        let dir = folder("open");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Session::start(&dir).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let s = Session::start(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = s.write_autosave(&doc(), None).unwrap_err();
        assert!(e.contains("other users"), "{e}");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosave_is_due_after_the_interval_with_new_unsaved_changes() {
        let since = Instant::now();
        let min = Duration::from_secs(60);
        let due = |after: u64, revision, saved, modified, busy, interval| {
            autosave_due(
                since + Duration::from_secs(after),
                since,
                interval,
                revision,
                saved,
                modified,
                busy,
            )
        };
        assert!(due(60, 3, None, true, false, min));
        assert!(due(61, 3, Some(2), true, false, min));
        assert!(!due(59, 3, None, true, false, min), "too soon");
        assert!(!due(60, 3, Some(3), true, false, min), "already saved");
        assert!(!due(60, 3, None, false, false, min), "not modified");
        assert!(!due(60, 3, None, true, true, min), "mid-operation");
        assert!(!due(600, 3, None, true, false, Duration::ZERO), "off");
    }
}
