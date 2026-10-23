//! The phone viewer's Recent list: the last five shared models it opened, kept encrypted (with
//! the key, so they open again without the computer that shared them) in the browser's
//! IndexedDB. The list logic is here, over a `Storage` trait, so the native gate tests it with
//! a fake; `idb` is the browser's storage.
//!
//! Keeping the key beside the blob on the phone is deliberate: a model that opens offline must
//! be able to decrypt itself. What the encryption protects is the model on the wire and on the
//! computer's server; on the phone it is as private as the browser's own storage.

use session::share;

/// How many models are kept.
pub const KEEP: usize = 5;

/// The largest share (the encrypted blob) that is kept, in bytes.
pub const MAX_KEPT_BYTES: usize = 64 * 1024 * 1024;

/// The IndexedDB database the viewer keeps its models in.
pub const DB_NAME: &str = "sketchup-alt-viewer";

/// The object store holding each kept model: `{ id, key, name, time, blob }`.
pub const STORE: &str = "recent";

/// The object store holding what the list needs of each one without reading the blobs:
/// `{ id, name, time, size }`.
pub const INFO_STORE: &str = "recent-info";

/// What is said while the model open now is not kept because it is too big.
pub const TOO_LARGE: &str = "This model is over 64 MiB, so it is not kept for opening offline.";

/// What is said when the browser keeps nothing (storage blocked, a private window).
pub const NO_STORAGE: &str = "This browser does not allow keeping models for opening offline.";

/// What the list knows of a kept model.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// The share's id in 32 lowercase hex digits (as in the blob's path).
    pub id: String,
    pub name: String,
    /// When it was last opened, in milliseconds since the epoch.
    pub time: f64,
    /// The encrypted blob's size in bytes.
    pub size: usize,
}

/// A kept model: its list entry, the key and the encrypted blob.
#[derive(Clone)]
pub struct Record {
    pub info: Info,
    pub key: [u8; 32],
    pub blob: Vec<u8>,
}

impl std::fmt::Debug for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Record")
            .field("info", &self.info)
            .field("key", &"<hidden>")
            .field("blob_len", &self.blob.len())
            .finish()
    }
}

/// Where kept models live. Writes may finish later (the browser's are asynchronous), so the
/// list is read back from `infos`, which always shows what was put and removed so far.
pub trait Storage {
    /// Whether anything can be kept at all.
    fn available(&self) -> bool {
        true
    }

    /// Every kept model's entry, in any order.
    fn infos(&self) -> Vec<Info>;

    /// Keeps `record`, replacing the one with the same id.
    fn put(&mut self, record: Record);

    /// Sets when `id` was last opened.
    fn touch(&mut self, id: &str, time: f64);

    /// Forgets `id`.
    fn remove(&mut self, id: &str);

    /// Reads `id` back; `done` gets None when it is not there or cannot be read. It may be
    /// called at once or later.
    fn get(&self, id: &str, done: Box<dyn FnOnce(Option<Record>)>);
}

/// What `Recent::keep` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    Yes,
    /// The share is over `MAX_KEPT_BYTES`.
    TooLarge,
    /// The browser keeps nothing.
    NoStorage,
}

/// A row of the Recent list as it is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub when: String,
    pub size: String,
}

/// The Recent list over a `Storage`.
pub struct Recent {
    storage: Box<dyn Storage>,
    /// Why the model open now is not kept, when it is not.
    note: Option<&'static str>,
}

impl Recent {
    pub fn new(storage: Box<dyn Storage>) -> Self {
        let note = (!storage.available()).then_some(NO_STORAGE);
        Self { storage, note }
    }

    /// Why the model open now is not kept, when it is not.
    pub fn note(&self) -> Option<&'static str> {
        self.note
    }

    /// Keeps `record` as the newest and drops the oldest beyond `KEEP`. A share over
    /// `MAX_KEPT_BYTES` is not kept (and `note` says why); an old copy of the same share goes
    /// either way, since it was just opened and the list should not offer a stale one.
    pub fn keep(&mut self, record: Record) -> Kept {
        if !self.storage.available() {
            self.note = Some(NO_STORAGE);
            return Kept::NoStorage;
        }
        if record.blob.len() > MAX_KEPT_BYTES {
            self.note = Some(TOO_LARGE);
            return Kept::TooLarge;
        }
        self.note = None;
        self.storage.put(record);
        for stale in self.ordered().into_iter().skip(KEEP) {
            self.storage.remove(&stale.id);
        }
        Kept::Yes
    }

    /// The kept models, newest first (ties by id).
    fn ordered(&self) -> Vec<Info> {
        let mut infos = self.storage.infos();
        infos.sort_by(|a, b| b.time.total_cmp(&a.time).then_with(|| a.id.cmp(&b.id)));
        infos
    }

    /// The list as shown at time `now` (milliseconds since the epoch).
    pub fn list(&self, now: f64) -> Vec<Entry> {
        self.ordered()
            .into_iter()
            .map(|i| Entry {
                when: age_text(now, i.time),
                size: size_text(i.size),
                id: i.id,
                name: i.name,
            })
            .collect()
    }

    /// Removes one model from the list.
    pub fn delete(&mut self, id: &str) {
        self.storage.remove(id);
    }

    /// Reads a kept model back for opening, making it the newest. `done` gets None when it is
    /// no longer there.
    pub fn open(&mut self, id: &str, now: f64, done: Box<dyn FnOnce(Option<Record>)>) {
        // What is opened was kept, whatever the model before it was.
        self.note = (!self.storage.available()).then_some(NO_STORAGE);
        if self.storage.infos().iter().any(|i| i.id == id) {
            self.storage.touch(id, now);
        }
        self.storage.get(id, done);
    }
}

/// A record's model name and bytes, decrypted with the key kept beside it. Err says what to
/// show: the stored model is damaged.
pub fn unseal(record: &Record) -> Result<(String, Vec<u8>), String> {
    let id = parse_id(&record.info.id).ok_or_else(|| app::app::DAMAGED_LINK.to_string())?;
    share::open(&record.blob, &id, &record.key).map_err(|e| e.message)
}

/// The 16 bytes of an id in 32 lowercase hex digits.
pub fn parse_id(id: &str) -> Option<[u8; 16]> {
    if id.len() != 32 || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&id[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// The id as the blob's path writes it.
pub fn id_hex(id: &[u8; 16]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(32);
    for byte in id {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// "just now", "5 min ago", "3 h ago", "2 days ago": how long before `now` (milliseconds)
/// `then` was. A time in the future (the clock moved) reads as "just now".
pub fn age_text(now: f64, then: f64) -> String {
    let seconds = ((now - then) / 1000.0).max(0.0);
    let minutes = (seconds / 60.0).floor() as u64;
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        _ => match minutes / 1440 {
            1 => "1 day ago".to_string(),
            days => format!("{days} days ago"),
        },
    }
}

/// "812 KB", "12.3 MB": a size with one decimal from a megabyte up (powers of 1024).
pub fn size_text(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{} KB", (b / KB).round() as u64)
    } else {
        format!("{:.1} MB", b / (KB * KB))
    }
}

#[cfg(test)]
mod tests;
