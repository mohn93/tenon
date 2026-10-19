//! Telling the two `.skp` generations apart, and framing what is inside each.
//!
//! Both start with the same header: a `ff fe ff <len>` tag followed by `len`
//! UTF-16LE code units, twice - once for the literal text `SketchUp Model`,
//! once for a brace-wrapped writer version such as `{19.1.174}` or `{25.x}`.
//! What follows the header is what differs, and the version string alone is
//! not a reliable discriminator, so the container is identified by structure:
//!
//! - a zip local header within the first 256 bytes means the model lives
//!   inside the zip as `model.dat` - the VFF container, 2021 and later;
//! - otherwise the model is a raw MFC `CArchive` stream starting right after
//!   the header, optionally followed by a small zip of side assets.
//!
//! That zip is *optional*, and finding out cost a fixture. Every legacy file
//! in our corpus from `{13.0.1}` through `{18.0.1}` has no zip anywhere in it,
//! while `{19.1.174}` and a 2017-format file written by a modern SketchUp both
//! append one. A reader that requires the zip rejects a 43.6 MB school
//! cafeteria that is otherwise perfectly readable.

use crate::error::{Error, Result};
use crate::source::{ByteSource, Le};
use crate::zip::{read_entry, ZipDirectory, ZipEntry};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generation {
    /// 2021+. The model is a deflate entry named `model.dat` inside the zip.
    Vff,
    /// 2013-2020. The model is an uncompressed CArchive stream in the file
    /// itself; the zip at the end holds only side assets.
    Legacy,
}

pub struct Header {
    pub magic: String,
    pub version: String,
    /// First byte after the header - where a legacy CArchive stream begins.
    pub end: u64,
}

/// A `.skp` opened far enough to say what it is and where its parts are.
///
/// Nothing large has been read at this point. On the 517 MB restaurant pack
/// this costs two small reads: the header, and the 26-byte directory record
/// at the very end of the file.
pub struct Container {
    pub generation: Generation,
    pub header: Header,
    /// Absent in every legacy file older than roughly 2019, and required for
    /// VFF, where the model itself is an entry.
    pub zip: Option<ZipDirectory>,
    /// For `Legacy`, the byte range of the CArchive stream: everything between
    /// the header and the start of the appended zip. For `Vff`, `None` - the
    /// model is a zip entry instead.
    pub legacy_stream: Option<(u64, u64)>,
}

impl Container {
    pub fn open(src: &dyn ByteSource) -> Result<Self> {
        let header = read_header(src)?;

        // A zip local header this early can only be the VFF container. In a
        // legacy file the first quarter-kilobyte is CArchive class records.
        let probe_len = core::cmp::min(256, src.len()) as usize;
        let probe = src.read_vec(0, probe_len)?;
        let generation = if find(&probe, b"PK\x03\x04").is_some() {
            Generation::Vff
        } else {
            Generation::Legacy
        };

        let zip = match ZipDirectory::find(src) {
            Ok(z) => Some(z),
            // VFF keeps the model in the zip, so a missing directory there is
            // fatal. Legacy keeps the model outside it, so a missing directory
            // just means this file has no side assets.
            Err(e) => match generation {
                Generation::Vff => return Err(e),
                Generation::Legacy => None,
            },
        };

        let legacy_stream = match generation {
            Generation::Vff => None,
            Generation::Legacy => {
                // The CArchive runs from the end of the header to wherever the
                // appended zip begins, or to the end of the file when there is
                // no zip. `archive_base` is derived from the directory record,
                // so this needs no guessing.
                let end = zip
                    .as_ref()
                    .map(|z| z.archive_base)
                    .unwrap_or_else(|| src.len());
                Some((header.end, end))
            }
        };

        Ok(Self {
            generation,
            header,
            zip,
            legacy_stream,
        })
    }

    /// The model stream's byte range for a legacy file, or the `model.dat`
    /// entry for a VFF one. This is the single thing a geometry pass needs.
    pub fn model_entry(&self) -> Option<&ZipEntry> {
        match self.generation {
            Generation::Vff => self.zip.as_ref()?.find_entry("model.dat"),
            Generation::Legacy => None,
        }
    }

    /// Entries that are texture images, left unread.
    ///
    /// Line art never asks for these. On a 46 MB VFF file they are 10.4 MB; on
    /// the 517 MB legacy pack the equivalent inline blobs are 380 MB, which is
    /// the entire reason this reader is built around seeking rather than
    /// loading.
    pub fn texture_entries(&self) -> impl Iterator<Item = &ZipEntry> {
        self.zip.iter().flat_map(|z| z.entries.iter()).filter(|e| {
            let n = e.name.to_ascii_lowercase();
            n.ends_with(".png") || n.ends_with(".jpg") || n.ends_with(".jpeg")
        })
    }

    pub fn read(&self, src: &dyn ByteSource, entry: &ZipEntry) -> Result<Vec<u8>> {
        read_entry(src, entry)
    }
}

fn read_header(src: &dyn ByteSource) -> Result<Header> {
    let head = src.read_vec(0, core::cmp::min(512, src.len()) as usize)?;
    let (magic, after_magic) = read_utf16_tag(&head, 0)?;
    if magic != "SketchUp Model" {
        return Err(Error::NotSkp);
    }
    // The version tag is optional in principle; a file without one is still
    // readable, so a failure here is not fatal.
    let (version, end) = match read_utf16_tag(&head, after_magic) {
        Ok((v, e)) => (v, e),
        Err(_) => (String::new(), after_magic),
    };
    Ok(Header {
        magic,
        version,
        end: end as u64,
    })
}

/// Read one `ff fe ff <u8 len>` + UTF-16LE run, returning it and the offset
/// just past it.
fn read_utf16_tag(bytes: &[u8], at: usize) -> Result<(String, usize)> {
    let le = Le(bytes);
    let tag = le.take(at, 4)?;
    if tag[0] != 0xff || tag[1] != 0xfe || tag[2] != 0xff {
        return Err(Error::NotSkp);
    }
    let units = tag[3] as usize;
    let body = le.take(at + 4, units * 2)?;
    let mut s = String::with_capacity(units);
    for i in 0..units {
        let c = u16::from_le_bytes([body[i * 2], body[i * 2 + 1]]);
        s.push(char::from_u32(c as u32).unwrap_or('\u{fffd}'));
    }
    Ok((s, at + 4 + units * 2))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
