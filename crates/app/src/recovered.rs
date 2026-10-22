//! What the recovery folder's entries are and when an autosave is due: the parts of recovery
//! that do not touch files, so the window compiles everywhere (the folder itself is
//! `recovery`, native only).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use web_time::{Instant, SystemTime};

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

/// The most entries a listing returns.
pub const MAX_LISTED: usize = 200;

/// What `list_newest` found: the newest entries, and how many older candidates were left out.
#[derive(Debug, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// Older recovery notes and copies not looked at or not returned (some may turn out to
    /// belong to a running window).
    pub older: usize,
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
