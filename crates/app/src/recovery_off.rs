//! Recovery where there is no folder to keep it in (the browser): the entries and the autosave
//! rule are shared with the desktop (`recovered`); a `Session` cannot exist, so nothing is
//! ever autosaved or listed.

use std::io;
use std::path::{Path, PathBuf};

use session::Document;

pub use crate::recovered::{Entry, Kind, Listing, MAX_LISTED, autosave_due, file_stem};

/// A hold on the recovery folder. Nothing makes one here.
pub enum Session {}

impl Session {
    pub fn write_autosave(&self, _doc: &Document, _original: Option<&Path>) -> Result<(), String> {
        match *self {}
    }

    pub fn clear_autosave(&self) {
        match *self {}
    }

    pub fn note_discarded(&self, _path: &Path, _original: Option<&Path>) {
        match *self {}
    }

    pub fn adopt(&self, _entry: &Entry) -> io::Result<PathBuf> {
        match *self {}
    }
}

/// There is no folder: the same error a folder that cannot be used gives.
pub fn ensure_folder(_dir: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "there is no recovery folder here",
    ))
}

pub fn list_newest(_dir: &Path, _own: &Session, _discarded: bool, _max: usize) -> Listing {
    Listing::default()
}

pub fn discard(_entry: &Entry) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "there is no recovery folder here",
    ))
}
