//! Where bytes come from.
//!
//! A `.skp` is read through this trait and never held whole. The 517 MB
//! restaurant pack in our corpus is 380 MB of inline JPEG that line art never
//! needs, so the reader seeks past it instead of loading it. Offsets are `u64`
//! so the size of the file never bounds how much memory we keep: on `wasm32`
//! the 4 GB address space limits what is *resident*, not what can be opened.
//!
//! Implementations only have to answer "give me these bytes at this offset".
//! That is deliberately the smallest possible contract, because one of the
//! implementations is a JavaScript `Blob` on the other side of a worker
//! boundary, and anything richer would not survive the trip.

use crate::error::{Error, Result};
use alloc::vec;
use alloc::vec::Vec;

pub trait ByteSource {
    /// Total size in bytes.
    fn len(&self) -> u64;

    /// Fill `buf` from `offset`, or fail. Partial reads are never success.
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read `len` bytes into a fresh `Vec`. Convenience for the small reads -
    /// headers, directory records - where a borrow is not worth the plumbing.
    ///
    /// The range is checked against the source before anything is allocated,
    /// so a length field read from a damaged file cannot ask for gigabytes.
    fn read_vec(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.bounds_check(offset, len)?;
        let mut v = vec![0u8; len];
        self.read_exact_at(offset, &mut v)?;
        Ok(v)
    }

    /// Read the last `len` bytes, or the whole file if it is shorter.
    fn read_tail(&self, len: usize) -> Result<(u64, Vec<u8>)> {
        let total = self.len();
        let take = core::cmp::min(len as u64, total);
        let start = total - take;
        Ok((start, self.read_vec(start, take as usize)?))
    }

    fn bounds_check(&self, offset: u64, len: usize) -> Result<()> {
        let end = offset.checked_add(len as u64);
        match end {
            Some(e) if e <= self.len() => Ok(()),
            _ => Err(Error::OutOfRange {
                offset,
                len,
                source_len: self.len(),
            }),
        }
    }
}

/// A source backed by bytes already in memory.
///
/// Used for test fixtures and for the small decompressed streams we do hold
/// whole, such as a single `material.xml`. Not how a large file is opened.
pub struct MemorySource {
    bytes: Vec<u8>,
}

impl MemorySource {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl ByteSource for MemorySource {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.bounds_check(offset, buf.len())?;
        let start = offset as usize;
        buf.copy_from_slice(&self.bytes[start..start + buf.len()]);
        Ok(())
    }
}

/// Little-endian scalar reads over a byte slice, bounds-checked.
///
/// Every length and offset in both container generations is little-endian, and
/// every one of them is attacker-controlled in the sense that matters: these
/// are files from the internet, opened in a browser tab belonging to someone
/// who just wanted to look at a room.
pub(crate) struct Le<'a>(pub &'a [u8]);

impl<'a> Le<'a> {
    #[inline]
    pub fn u16(&self, at: usize) -> Result<u16> {
        let b = self.take(at, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    #[inline]
    pub fn u32(&self, at: usize) -> Result<u32> {
        let b = self.take(at, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    #[inline]
    pub fn u64(&self, at: usize) -> Result<u64> {
        let b = self.take(at, 8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    #[inline]
    pub fn take(&self, at: usize, len: usize) -> Result<&'a [u8]> {
        at.checked_add(len)
            .and_then(|end| self.0.get(at..end))
            .ok_or(Error::OutOfRange {
                offset: at as u64,
                len,
                source_len: self.0.len() as u64,
            })
    }
}
