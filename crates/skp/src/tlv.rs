//! The record grammar of `model.dat`.
//!
//! Every record is `[u16 tag][u32 size][size bytes]`, little-endian, and the
//! whole file is one `SketchUpModel` record wrapping the rest. Tags are the
//! numbers in `tags.rs`, which came out of SketchUp itself, so a tag either
//! matches a named constant or it is genuinely unknown - there is no byte
//! order to get wrong and no spelling convention to remember.
//!
//! Nothing here allocates. A record's payload is a borrow of the bytes it was
//! read from, so walking a definition costs one copy of that definition, made
//! once by the caller, and no copies per record.

use crate::error::{Error, Result};
use crate::source::Le;

pub const HEADER_LEN: usize = 6;

/// How deep a record tree may nest before we call the file malformed.
///
/// Real models nest a handful of levels - model, components, a definition, its
/// faces, a face's loops. The limit exists because recursion depth here is
/// file-controlled, and this reader runs in a browser tab.
pub const MAX_DEPTH: u32 = 64;

#[derive(Clone, Copy)]
pub struct Record<'a> {
    pub tag: u16,
    pub payload: &'a [u8],
}

impl<'a> Record<'a> {
    /// Iterate this record's children, if its payload is itself records.
    pub fn children(&self) -> Tlv<'a> {
        Tlv::new(self.payload)
    }

    pub fn child(&self, tag: u16) -> Option<Record<'a>> {
        self.children().find(|r| r.tag == tag)
    }

    pub fn children_with(&self, tag: u16) -> impl Iterator<Item = Record<'a>> + 'a {
        self.children().filter(move |r| r.tag == tag)
    }

    pub fn f64_at(&self, at: usize) -> Result<f64> {
        Ok(f64::from_bits(Le(self.payload).u64(at)?))
    }

    /// Three f64 starting at `at` - a point or a vector.
    pub fn vec3_at(&self, at: usize) -> Result<[f64; 3]> {
        Ok([
            self.f64_at(at)?,
            self.f64_at(at + 8)?,
            self.f64_at(at + 16)?,
        ])
    }

    pub fn u32_at(&self, at: usize) -> Result<u32> {
        Le(self.payload).u32(at)
    }

    pub fn u16_at(&self, at: usize) -> Result<u16> {
        Le(self.payload).u16(at)
    }

    pub fn u8_at(&self, at: usize) -> Result<u8> {
        Ok(*self.payload.get(at).ok_or(Error::OutOfRange {
            offset: at as u64,
            len: 1,
            source_len: self.payload.len() as u64,
        })?)
    }

    pub fn bool_at(&self, at: usize) -> Result<bool> {
        Ok(self.u8_at(at)? != 0)
    }

    /// A length-prefixed little-endian integer of `n` bytes, as the format uses
    /// for entity ids: the width varies per record and is carried alongside.
    pub fn varint_at(&self, at: usize, n: usize) -> Result<u64> {
        if n > 8 {
            return Err(Error::Malformed("entity id wider than 64 bits"));
        }
        let b = Le(self.payload).take(at, n)?;
        let mut v = 0u64;
        for (i, byte) in b.iter().enumerate() {
            v |= (*byte as u64) << (8 * i);
        }
        Ok(v)
    }
}

/// Walks a byte range as a sequence of records.
///
/// A size that runs past the end stops the walk rather than failing it. Trailing
/// bytes that are not a record happen in real files - padding, a sentinel - and
/// a reader that threw there would refuse models SketchUp opens.
pub struct Tlv<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Tlv<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
}

impl<'a> Iterator for Tlv<'a> {
    type Item = Record<'a>;

    fn next(&mut self) -> Option<Record<'a>> {
        if self.pos + HEADER_LEN > self.bytes.len() {
            return None;
        }
        let le = Le(self.bytes);
        let tag = le.u16(self.pos).ok()?;
        let size = le.u32(self.pos + 2).ok()? as usize;
        let start = self.pos + HEADER_LEN;
        if size > self.bytes.len() - start {
            return None;
        }
        self.pos = start + size;
        Some(Record {
            tag,
            payload: &self.bytes[start..start + size],
        })
    }
}

/// Depth-first search for the first record with `tag`, at any depth.
///
/// Only follows records whose payload parses as records, so a leaf whose bytes
/// happen to look like a header is not descended into by accident - the walk
/// requires the child sizes to tile the payload exactly.
pub fn find_deep<'a>(within: &[Record<'a>], tag: u16, depth: u32) -> Option<Record<'a>> {
    if depth == 0 {
        return None;
    }
    for r in within {
        if r.tag == tag {
            return Some(*r);
        }
    }
    for r in within {
        if !payload_is_records(r.payload) {
            continue;
        }
        let kids: alloc::vec::Vec<Record<'a>> = r.children().collect();
        if let Some(found) = find_deep(&kids, tag, depth - 1) {
            return Some(found);
        }
    }
    None
}

/// Whether a payload is a clean sequence of records that tiles it exactly.
///
/// This is the test that keeps a leaf of doubles from being mistaken for a
/// container. It is not free, so it belongs on exploratory walks rather than in
/// the hot path, where the tag already says what the payload is.
pub fn payload_is_records(payload: &[u8]) -> bool {
    if payload.len() < HEADER_LEN {
        return false;
    }
    let le = Le(payload);
    let mut pos = 0usize;
    let mut seen = 0u32;
    while pos + HEADER_LEN <= payload.len() {
        let Ok(size) = le.u32(pos + 2) else {
            return false;
        };
        let next = pos + HEADER_LEN + size as usize;
        if next > payload.len() {
            return false;
        }
        pos = next;
        seen += 1;
        if seen > 1 << 20 {
            return false;
        }
    }
    pos == payload.len() && seen > 0
}

/// A UTF-16LE string with a `u32` code-unit count, as names are stored.
pub fn utf16_string(payload: &[u8], at: usize) -> Result<alloc::string::String> {
    let le = Le(payload);
    let units = le.u32(at)? as usize;
    let body = le.take(
        at + 4,
        units
            .checked_mul(2)
            .ok_or(Error::Malformed("string length overflow"))?,
    )?;
    let mut s = alloc::string::String::with_capacity(units);
    let mut i = 0;
    while i < units {
        let c = u16::from_le_bytes([body[i * 2], body[i * 2 + 1]]);
        // Surrogate pairs do occur in names - emoji in a Korean pack's tag
        // names, for instance - so decode rather than cast.
        if (0xd800..0xdc00).contains(&c) && i + 1 < units {
            let lo = u16::from_le_bytes([body[i * 2 + 2], body[i * 2 + 3]]);
            if (0xdc00..0xe000).contains(&lo) {
                let cp = 0x10000 + (((c as u32) - 0xd800) << 10) + ((lo as u32) - 0xdc00);
                s.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                i += 2;
                continue;
            }
        }
        s.push(char::from_u32(c as u32).unwrap_or('\u{fffd}'));
        i += 1;
    }
    Ok(s)
}
