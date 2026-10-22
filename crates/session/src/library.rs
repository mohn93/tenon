//! Library items: `.skr` files that carry a `library.json` entry saying what they hold (one
//! component, one material or a whole model) and its name. Any `.skr` file is an item: one
//! without the entry is a model named by its file name.
//!
//! Library files are untrusted. Listing reads the archive's directory and the small
//! `library.json` entry only, never follows links and stays within [`MAX_LISTED`] items;
//! reading an item for insertion goes through the `.skr` reader's checks.

use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::time::UNIX_EPOCH;

use kernel::{EntityId, Material, MaterialId, Scene};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::command::{CommandError, ErrorKind};
use crate::file;
use crate::fspath::is_link;

/// The most items [`list`] returns.
pub const MAX_LISTED: usize = 1000;
/// The longest item name, in characters.
pub const MAX_NAME_CHARS: usize = 256;
/// The largest item file listed or inserted, in bytes.
pub const MAX_ITEM_BYTES: u64 = 256 * 1024 * 1024;
/// The most folder entries [`list`] looks at.
const MAX_SCANNED: usize = 100_000;
/// The archive entry saying what an item holds.
pub(crate) const INFO_ENTRY: &str = "library.json";
/// The largest `library.json` read.
const MAX_INFO_BYTES: u64 = 16 * 1024;

/// What a library item holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryKind {
    /// One component instance, at the origin.
    Component,
    /// One material.
    Material,
    /// A whole model.
    Model,
}

/// The `library.json` entry.
#[derive(Serialize, Deserialize)]
struct Info {
    kind: LibraryKind,
    #[serde(default)]
    name: Option<String>,
}

/// What `save_to_library` saves: an instance id ("i4") or a material id ("m2").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryRef {
    Entity(EntityId),
    Material(MaterialId),
}

impl FromStr for LibraryRef {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        if s.starts_with('m') {
            s.parse().map(LibraryRef::Material)
        } else {
            s.parse().map(LibraryRef::Entity)
        }
        .map_err(|_| format!("invalid item id {s:?}: an instance (i<n>) or a material (m<n>)"))
    }
}

impl fmt::Display for LibraryRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryRef::Entity(id) => write!(f, "{id}"),
            LibraryRef::Material(id) => write!(f, "{id}"),
        }
    }
}

impl TryFrom<String> for LibraryRef {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        s.parse()
    }
}

impl From<LibraryRef> for String {
    fn from(r: LibraryRef) -> String {
        r.to_string()
    }
}

impl Serialize for LibraryRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LibraryRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for LibraryRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LibraryRef".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[im](0|[1-9][0-9]*)$",
            "description": "An instance id (i<n>) or a material id (m<n>)"
        })
    }
}

/// One item of a library folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LibraryItem {
    /// The file, as the folder and the file name make it.
    pub path: String,
    pub name: String,
    pub kind: LibraryKind,
    /// When the file was last written, in seconds since 1970 (0 when unknown).
    pub modified: u64,
    /// Why the file cannot be used as an item; absent for a good one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What [`list`] found.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LibraryListing {
    /// At most [`MAX_LISTED`], sorted by name ignoring case.
    pub items: Vec<LibraryItem>,
    /// True when the folder holds more `.skr` files than were listed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more: bool,
}

/// An item read for use: what it holds, its name and its scene (a component's root holds one
/// instance at the origin, a material's scene holds only the material).
pub struct LibraryContent {
    pub kind: LibraryKind,
    pub name: String,
    pub scene: Scene,
}

/// Whether `name` can name an item: not blank, at most [`MAX_NAME_CHARS`] characters, no
/// control characters.
pub(crate) fn check_name(name: &str) -> Result<(), CommandError> {
    let bad = name.trim().is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control);
    if bad {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!(
                "an item's name must be from 1 to {MAX_NAME_CHARS} characters, with no control \
                 characters"
            ),
        ));
    }
    Ok(())
}

pub(crate) fn encode_info(kind: LibraryKind, name: &str) -> Result<Vec<u8>, CommandError> {
    serde_json::to_vec(&Info {
        kind,
        name: Some(name.to_string()),
    })
    .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
}

pub(crate) fn is_skr(name: &str) -> bool {
    name.len() > ".skr".len()
        && name
            .get(name.len() - ".skr".len()..)
            .is_some_and(|e| e.eq_ignore_ascii_case(".skr"))
}

fn stem(name: &str) -> String {
    name[..name.len() - ".skr".len()]
        .chars()
        .take(MAX_NAME_CHARS)
        .collect()
}

/// The `library.json` entry of the `.skr` file at `path` when it has a valid one (kind known, a
/// usable name when it has one): what a save of a model opened from an item writes back.
pub(crate) fn info_of(path: &Path) -> Option<Vec<u8>> {
    let bytes = file::read_entry(path, INFO_ENTRY, MAX_INFO_BYTES).ok()??;
    let info: Info = serde_json::from_slice(&bytes).ok()?;
    if info.name.as_deref().is_some_and(|n| check_name(n).is_err()) {
        return None;
    }
    Some(bytes)
}

/// The kind and name a file's `library.json` gives, or a model named by the file's stem when
/// it has none or it is not valid.
fn describe(path: &Path, file_name: &str) -> Result<(LibraryKind, String), String> {
    let info = file::read_entry(path, INFO_ENTRY, MAX_INFO_BYTES)?
        .and_then(|bytes| serde_json::from_slice::<Info>(&bytes).ok())
        .and_then(|info| {
            let name = info.name?;
            check_name(&name).ok()?;
            Some((info.kind, name))
        });
    Ok(info.unwrap_or((LibraryKind::Model, stem(file_name))))
}

/// The `.skr` files directly in `folder`, sorted by name ignoring case, at most
/// [`MAX_LISTED`]. Links, folders and files over [`MAX_ITEM_BYTES`] are left out; a file that
/// is not a model archive is listed with its `error`. With more than [`MAX_LISTED`] files the
/// first by file name are listed (and `more` is set), so the work stays bounded.
pub fn list(folder: &Path) -> Result<LibraryListing, CommandError> {
    let file_error =
        |e: String| CommandError::new(ErrorKind::File, format!("{}: {e}", folder.display()));
    if !std::fs::metadata(folder)
        .map_err(|e| file_error(e.to_string()))?
        .is_dir()
    {
        return Err(file_error("not a folder".into()));
    }
    let read = std::fs::read_dir(folder).map_err(|e| file_error(e.to_string()))?;
    let mut found: Vec<(String, std::path::PathBuf, u64)> = Vec::new();
    for entry in read.take(MAX_SCANNED).flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_skr(&name) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if is_link(&meta) || !meta.is_file() || meta.len() > MAX_ITEM_BYTES {
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        found.push((name, entry.path(), modified));
    }
    found.sort_by_cached_key(|(name, ..)| (name.to_lowercase(), name.clone()));
    let more = found.len() > MAX_LISTED;
    found.truncate(MAX_LISTED);
    let mut items: Vec<LibraryItem> = found
        .into_iter()
        .map(|(file_name, path, modified)| {
            let (kind, name, error) = match describe(&path, &file_name) {
                Ok((kind, name)) => (kind, name, None),
                Err(e) => (LibraryKind::Model, stem(&file_name), Some(e)),
            };
            LibraryItem {
                path: path.display().to_string(),
                name,
                kind,
                modified,
                error,
            }
        })
        .collect();
    items.sort_by_cached_key(|i| (i.name.to_lowercase(), i.name.clone(), i.path.clone()));
    Ok(LibraryListing { items, more })
}

/// Reads the item at `path` with every check opening a `.skr` file makes: a regular file of
/// at most [`MAX_ITEM_BYTES`].
pub fn read_item(path: &Path) -> Result<LibraryContent, CommandError> {
    let named = path.display().to_string();
    let file_error = |e: String| CommandError::new(ErrorKind::File, format!("{named}: {e}"));
    let meta = std::fs::metadata(path).map_err(|e| file_error(e.to_string()))?;
    if !meta.is_file() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!("{named} is not a file"),
        ));
    }
    if meta.len() > MAX_ITEM_BYTES {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            format!("{named} is too large for a library item: over {MAX_ITEM_BYTES} bytes"),
        ));
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("item.skr");
    let (kind, name) = describe(path, file_name).map_err(file_error)?;
    let opened = file::read(path).map_err(file_error)?;
    Ok(LibraryContent {
        kind,
        name,
        scene: opened.scene,
    })
}

/// What the Paste tool draws for the component or model item at `path`: its edges and base
/// point (the corner a click puts at the cursor). A material item has none: `invalid_input`.
pub fn outline(path: &Path) -> Result<crate::ClipOutline, CommandError> {
    let item = read_item(path)?;
    if item.kind == LibraryKind::Material {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "a material item is not placed with the cursor",
        ));
    }
    let clip = crate::clip::from_scene(item.scene, String::new())?;
    Ok(crate::clip::outline_of(&clip))
}

/// Whether two materials are the same but for their ids and names: color, opacity, emission
/// and texture (its image's file bytes and its size).
pub(crate) fn same_material(a: &Material, b: &Material) -> bool {
    a.color == b.color
        && a.opacity == b.opacity
        && a.emission == b.emission
        && match (&a.texture, &b.texture) {
            (None, None) => true,
            (Some(x), Some(y)) => {
                x.width == y.width && x.height == y.height && x.image.encoded == y.image.encoded
            }
            _ => false,
        }
}
