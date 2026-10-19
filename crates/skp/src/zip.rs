//! The zip layer, read for random access rather than extracted.
//!
//! Both `.skp` generations carry a zip, but in opposite proportions, and that
//! is the whole reason this module parses the central directory itself instead
//! of handing the file to a general-purpose unzip:
//!
//! - **VFF (2021+)**: the zip *is* the file. `model.dat` is one big deflate
//!   entry, textures are separate entries. Measured on our corpus: a 27.7 MB
//!   file holds 917 entries, with `model.dat` 18.4 MB compressed and 92.9 MB
//!   raw. Random access means we inflate `model.dat` and leave 915 texture
//!   entries untouched until something actually asks to see a texture.
//!
//! - **Legacy (2013-2020)**: the zip is a 0.1 MB afterthought at the very end
//!   of the file. The 517 MB restaurant pack puts its end-of-central-directory
//!   record at byte 542,860,644 of 542,860,670 - the last 26 bytes - and holds
//!   nine entries: an IFC schema, a thumbnail, some xml. The 517 MB of model
//!   lives *before* the zip, outside it.
//!
//! Because a zip can sit at any offset inside a larger file, the archive's
//! base is derived from the directory record rather than assumed to be zero.
//! That one piece of arithmetic is what lets the same code read both shapes.

use crate::error::{Error, Result};
use crate::source::{ByteSource, Le};
use alloc::string::String;
use alloc::vec::Vec;

const EOCD_SIG: u32 = 0x0605_4b50;
const EOCD64_LOCATOR_SIG: u32 = 0x0706_4b50;
const EOCD64_SIG: u32 = 0x0606_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

const METHOD_STORE: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

/// One entry, located but not read.
#[derive(Debug, Clone)]
pub struct ZipEntry {
    pub name: String,
    pub method: u16,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    /// Absolute offset of the local file header in the containing file.
    pub header_offset: u64,
}

pub struct ZipDirectory {
    pub entries: Vec<ZipEntry>,
    /// Absolute offset in the containing file where the zip archive starts.
    /// Zero for a bare zip; 542,860,544-ish for a legacy `.skp`.
    pub archive_base: u64,
}

impl ZipDirectory {
    pub fn find(src: &dyn ByteSource) -> Result<Self> {
        // The comment field can push the record up to 64 KiB from the end, and
        // a zip with no comment - which is what SketchUp writes - puts it in
        // the last 22 bytes. Scanning backwards finds the real one first even
        // if entry data happens to contain the signature.
        let (tail_start, tail) = src.read_tail(66_000)?;
        let le = Le(&tail);

        let mut eocd_at = None;
        if tail.len() >= 22 {
            for i in (0..=tail.len() - 22).rev() {
                if le.u32(i)? == EOCD_SIG {
                    eocd_at = Some(i);
                    break;
                }
            }
        }
        let eocd_at = eocd_at.ok_or(Error::NoZipDirectory)?;

        let mut total_entries = le.u16(eocd_at + 10)? as u64;
        let mut cd_size = le.u32(eocd_at + 12)? as u64;
        let mut cd_offset = le.u32(eocd_at + 16)? as u64;

        // Zip64, for the day someone hands us a file over 4 GB. The locator
        // sits immediately before the classic record.
        if eocd_at >= 20 && le.u32(eocd_at - 20)? == EOCD64_LOCATOR_SIG {
            let eocd64_abs = le.u64(eocd_at - 20 + 8)?;
            let hdr = src.read_vec(eocd64_abs, 56)?;
            let h = Le(&hdr);
            if h.u32(0)? == EOCD64_SIG {
                total_entries = h.u64(32)?;
                cd_size = h.u64(40)?;
                cd_offset = h.u64(48)?;
            }
        }

        // The directory's own absolute position is known; the offset field is
        // relative to the archive start. The difference is how much of the
        // file sits in front of the zip - 26 bytes of it for a bare zip, half
        // a gigabyte of model data for a legacy .skp.
        let cd_abs = tail_start + eocd_at as u64;
        let archive_base = cd_abs
            .checked_sub(cd_size)
            .and_then(|v| v.checked_sub(cd_offset))
            .ok_or(Error::UnsupportedZip(
                "central directory runs before the start of the file",
            ))?;

        let cd_at = archive_base
            .checked_add(cd_offset)
            .ok_or(Error::Malformed("central directory offset overflows"))?;
        let cd_len = usize::try_from(cd_size).map_err(|_| Error::OutOfRange {
            offset: cd_at,
            len: usize::MAX,
            source_len: src.len(),
        })?;
        let cd = src.read_vec(cd_at, cd_len)?;
        let entries = parse_central_directory(&cd, total_entries, archive_base)?;
        Ok(Self {
            entries,
            archive_base,
        })
    }

    pub fn get(&self, name: &str) -> Option<&ZipEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Find an entry by exact name or by trailing path component, which is how
    /// `model.dat` appears in the wild - sometimes bare, sometimes nested.
    pub fn find_entry(&self, name: &str) -> Option<&ZipEntry> {
        self.get(name).or_else(|| {
            self.entries.iter().find(|e| {
                e.name
                    .rsplit('/')
                    .next()
                    .map(|last| last == name)
                    .unwrap_or(false)
            })
        })
    }
}

fn parse_central_directory(cd: &[u8], total: u64, archive_base: u64) -> Result<Vec<ZipEntry>> {
    let le = Le(cd);
    // The count comes from the file; no more records fit than the directory
    // has room for, so that bounds the reservation.
    let fit = (cd.len() / 46) as u64;
    let mut entries = Vec::with_capacity(core::cmp::min(total, fit) as usize);
    let mut at = 0usize;

    while at + 46 <= cd.len() {
        if le.u32(at)? != CENTRAL_SIG {
            break;
        }
        let method = le.u16(at + 10)?;
        let crc32 = le.u32(at + 16)?;
        let mut compressed_size = le.u32(at + 20)? as u64;
        let mut uncompressed_size = le.u32(at + 24)? as u64;
        let name_len = le.u16(at + 28)? as usize;
        let extra_len = le.u16(at + 30)? as usize;
        let comment_len = le.u16(at + 32)? as usize;
        let mut header_offset = le.u32(at + 42)? as u64;

        let name_bytes = le.take(at + 46, name_len)?;
        // Zip names are either CP437 or UTF-8 depending on a flag bit, but
        // every name we have ever seen in a .skp is ASCII. Non-UTF-8 bytes are
        // kept lossily rather than failing the whole directory: one odd
        // texture filename must not stop the model from opening.
        let name = String::from_utf8_lossy(name_bytes).into_owned();

        // Zip64 extended information, field id 0x0001. Present only for the
        // fields that actually overflowed, in a fixed order.
        let extra = le.take(at + 46 + name_len, extra_len)?;
        let ex = Le(extra);
        let mut ex_at = 0usize;
        while ex_at + 4 <= extra.len() {
            let id = ex.u16(ex_at)?;
            let size = ex.u16(ex_at + 2)? as usize;
            if id == 0x0001 {
                let mut p = ex_at + 4;
                if uncompressed_size == u32::MAX as u64 && p + 8 <= extra.len() {
                    uncompressed_size = ex.u64(p)?;
                    p += 8;
                }
                if compressed_size == u32::MAX as u64 && p + 8 <= extra.len() {
                    compressed_size = ex.u64(p)?;
                    p += 8;
                }
                if header_offset == u32::MAX as u64 && p + 8 <= extra.len() {
                    header_offset = ex.u64(p)?;
                }
                break;
            }
            ex_at += 4 + size;
        }

        entries.push(ZipEntry {
            name,
            method,
            compressed_size,
            uncompressed_size,
            crc32,
            header_offset: archive_base
                .checked_add(header_offset)
                .ok_or(Error::Malformed("zip entry offset overflows"))?,
        });

        at += 46 + name_len + extra_len + comment_len;
    }

    Ok(entries)
}

/// Offset of an entry's payload, skipping its local header.
///
/// The central directory records the header's position, not the data's, and
/// the local header carries its own name and extra fields whose lengths can
/// differ from the central copy. Reading the local header is not optional.
pub fn entry_data_offset(src: &dyn ByteSource, entry: &ZipEntry) -> Result<u64> {
    let hdr = src.read_vec(entry.header_offset, 30)?;
    let le = Le(&hdr);
    if le.u32(0)? != LOCAL_SIG {
        return Err(Error::UnsupportedZip("local header signature missing"));
    }
    let name_len = le.u16(26)? as u64;
    let extra_len = le.u16(28)? as u64;
    entry
        .header_offset
        .checked_add(30 + name_len + extra_len)
        .ok_or(Error::Malformed("zip entry offset overflows"))
}

/// Read one entry whole. For `model.dat` prefer the streaming path; this is
/// for the small entries - xml, thumbnails, a single texture - where holding
/// the bytes is the point.
pub fn read_entry(src: &dyn ByteSource, entry: &ZipEntry) -> Result<Vec<u8>> {
    let at = entry_data_offset(src, entry)?;
    let len = usize::try_from(entry.compressed_size).map_err(|_| Error::OutOfRange {
        offset: at,
        len: usize::MAX,
        source_len: src.len(),
    })?;
    match entry.method {
        METHOD_STORE => src.read_vec(at, len),
        METHOD_DEFLATE => {
            let raw = src.read_vec(at, len)?;
            miniz_oxide::inflate::decompress_to_vec_with_limit(
                &raw,
                usize::try_from(entry.uncompressed_size)
                    .unwrap_or(usize::MAX)
                    .saturating_add(64),
            )
            .map_err(|_| Error::Inflate("deflate stream rejected"))
        }
        _ => Err(Error::UnsupportedZip(
            "compression method other than store or deflate",
        )),
    }
}
