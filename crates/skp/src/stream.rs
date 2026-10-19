//! Reading `model.dat` without holding `model.dat`.
//!
//! A 27.7 MB commercial file inflates its model stream to 92.8 MB, and a 46 MB
//! one to 145 MB. Those are the files we have; a 500 MB VFF file would be
//! multiple gigabytes, which is past what a single allocation can even
//! represent in a 32-bit wasm address space. So the stream is consumed
//! sequentially and records are materialised one at a time.
//!
//! The structure cooperates. `model.dat` is one `SketchUpModel` record whose
//! children are a handful of big sections, and the biggest of those,
//! `SketchUpModelComponents`, holds each component definition as a separate
//! child. So the resident high-water mark is the largest single definition,
//! not the model.
//!
//! Two sources implement the same sequential contract: a raw byte range, which
//! is what a legacy file's uncompressed CArchive is, and a deflate stream,
//! which is what a VFF `model.dat` is.

use crate::error::{Error, Result};
use crate::source::ByteSource;
use alloc::vec;
use alloc::vec::Vec;

/// Bytes in order, with the right to throw them away.
pub trait SeqRead {
    /// Fill `buf` exactly, or fail.
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()>;
    /// Advance `n` bytes without keeping them.
    fn skip(&mut self, n: u64) -> Result<()>;
    /// How many bytes have been consumed so far.
    fn position(&self) -> u64;
    /// Total bytes available, if known. Used only for progress reporting.
    fn total_hint(&self) -> Option<u64>;
}

/// A plain byte range of a source. Legacy files need nothing more than this,
/// because their container is not compressed at all.
pub struct RangeRead<'s> {
    src: &'s dyn ByteSource,
    pos: u64,
    end: u64,
    start: u64,
}

impl<'s> RangeRead<'s> {
    pub fn new(src: &'s dyn ByteSource, start: u64, end: u64) -> Self {
        Self {
            src,
            pos: start,
            end,
            start,
        }
    }
}

impl<'s> SeqRead for RangeRead<'s> {
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        let n = buf.len() as u64;
        if self.pos.checked_add(n).map_or(true, |e| e > self.end) {
            return Err(Error::OutOfRange {
                offset: self.pos,
                len: buf.len(),
                source_len: self.end,
            });
        }
        self.src.read_exact_at(self.pos, buf)?;
        self.pos += n;
        Ok(())
    }

    fn skip(&mut self, n: u64) -> Result<()> {
        if self.pos.checked_add(n).map_or(true, |e| e > self.end) {
            return Err(Error::OutOfRange {
                offset: self.pos,
                len: n as usize,
                source_len: self.end,
            });
        }
        self.pos += n;
        Ok(())
    }

    fn position(&self) -> u64 {
        self.pos - self.start
    }

    fn total_hint(&self) -> Option<u64> {
        Some(self.end.saturating_sub(self.start))
    }
}

const IN_CHUNK: usize = 256 * 1024;
const OUT_CHUNK: usize = 256 * 1024;

/// A raw deflate stream, inflated on demand.
///
/// Skipping still costs the inflate - a deflate stream cannot be seeked - but
/// it costs no memory, which is the part that matters when the thing being
/// skipped is a 2 MB texture blob we are never going to decode.
pub struct InflateRead<'s> {
    src: &'s dyn ByteSource,
    /// Where the entry's compressed data begins, so consumed bytes can be
    /// reported against the file rather than against the inflated stream.
    data_at: u64,
    in_at: u64,
    in_end: u64,
    in_buf: Vec<u8>,
    in_pos: usize,
    in_len: usize,
    state: alloc::boxed::Box<miniz_oxide::inflate::stream::InflateState>,
    out: Vec<u8>,
    out_pos: usize,
    consumed: u64,
    expect: u64,
    /// Bytes inflated so far; more than `expect` is refused.
    produced: u64,
    finished: bool,
}

impl<'s> InflateRead<'s> {
    /// Fails when the compressed range does not lie inside the source, so a
    /// damaged size is refused here rather than partway through the stream.
    pub fn new(
        src: &'s dyn ByteSource,
        data_at: u64,
        compressed: u64,
        uncompressed: u64,
    ) -> Result<Self> {
        let in_end = data_at.checked_add(compressed).ok_or(Error::Malformed(
            "compressed entry runs past the largest offset",
        ))?;
        if in_end > src.len() {
            return Err(Error::OutOfRange {
                offset: data_at,
                len: usize::try_from(compressed).unwrap_or(usize::MAX),
                source_len: src.len(),
            });
        }
        Ok(Self {
            src,
            data_at,
            in_at: data_at,
            in_end,
            in_buf: vec![0u8; IN_CHUNK],
            in_pos: 0,
            in_len: 0,
            state: miniz_oxide::inflate::stream::InflateState::new_boxed(
                miniz_oxide::DataFormat::Raw,
            ),
            out: Vec::with_capacity(OUT_CHUNK * 2),
            out_pos: 0,
            consumed: 0,
            expect: uncompressed,
            produced: 0,
            finished: false,
        })
    }

    fn available(&self) -> usize {
        self.out.len() - self.out_pos
    }

    /// Inflate until at least `want` bytes are buffered, or the stream ends.
    fn pump(&mut self, want: usize) -> Result<()> {
        use miniz_oxide::inflate::stream::inflate;
        use miniz_oxide::{MZError, MZFlush, MZStatus};

        while self.available() < want && !self.finished {
            // Drop what the caller has already taken, so the buffer tracks the
            // window we still owe rather than everything inflated so far.
            if self.out_pos > OUT_CHUNK {
                self.out.drain(..self.out_pos);
                self.out_pos = 0;
            }

            if self.in_pos == self.in_len && self.in_at < self.in_end {
                let left = self.in_end - self.in_at;
                let take = core::cmp::min(left, IN_CHUNK as u64) as usize;
                self.src
                    .read_exact_at(self.in_at, &mut self.in_buf[..take])?;
                self.in_at += take as u64;
                self.in_pos = 0;
                self.in_len = take;
            }
            // With the input used up, the inflater can still hold output: a
            // window it could not hand over because the buffer was full, or
            // the last codes, which it decodes only once told no more input
            // is coming. So keep calling with empty input, and say so with
            // `Finish`, until the stream ends or nothing more comes out.
            let input_done = self.in_pos == self.in_len && self.in_at == self.in_end;
            let flush = if input_done {
                MZFlush::Finish
            } else {
                MZFlush::None
            };

            let before = self.out.len();
            self.out.resize(before + OUT_CHUNK, 0);
            let r = inflate(
                &mut self.state,
                &self.in_buf[self.in_pos..self.in_len],
                &mut self.out[before..],
                flush,
            );
            self.out.truncate(before + r.bytes_written);
            self.in_pos += r.bytes_consumed;
            // The entry's claimed size is what callers bound their work by: a stream that
            // runs past it is not what its header says.
            self.produced = self.produced.saturating_add(r.bytes_written as u64);
            if self.produced > self.expect {
                return Err(Error::Malformed(
                    "deflate stream is longer than its entry claims",
                ));
            }
            let progressed = r.bytes_written != 0 || r.bytes_consumed != 0;

            match r.status {
                Ok(MZStatus::StreamEnd) => self.finished = true,
                Ok(_) if progressed => {}
                // `Finish` reports a full output buffer as `Buf`.
                Err(MZError::Buf) if input_done && progressed => {}
                // Input exhausted with output still owed: a truncated entry.
                // The read that wanted the bytes reports it.
                Ok(_) | Err(MZError::Buf) if input_done => self.finished = true,
                // No progress and no end: the stream is stuck, which means
                // the data is not what its header claimed.
                Ok(_) => return Err(Error::Inflate("deflate stream made no progress")),
                Err(_) => return Err(Error::Inflate("deflate stream rejected")),
            }
        }
        Ok(())
    }

    pub fn uncompressed_hint(&self) -> u64 {
        self.expect
    }

    /// Compressed bytes pulled from the file so far.
    ///
    /// Distinct from `position()`, which is how far into the *inflated* stream
    /// the parser has got. Reporting the inflated figure as "bytes read" makes
    /// a 7.7 MB file look like it read 315% of itself, which is true of the
    /// stream and nonsense about the file.
    pub fn source_bytes(&self) -> u64 {
        self.in_at - self.data_at
    }
}

impl<'s> SeqRead for InflateRead<'s> {
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.pump(buf.len())?;
        if self.available() < buf.len() {
            return Err(Error::OutOfRange {
                offset: self.consumed,
                len: buf.len(),
                source_len: self.consumed + self.available() as u64,
            });
        }
        let from = self.out_pos;
        buf.copy_from_slice(&self.out[from..from + buf.len()]);
        self.out_pos += buf.len();
        self.consumed += buf.len() as u64;
        Ok(())
    }

    fn skip(&mut self, mut n: u64) -> Result<()> {
        while n > 0 {
            let want = core::cmp::min(n, OUT_CHUNK as u64) as usize;
            self.pump(want)?;
            let got = core::cmp::min(want, self.available());
            if got == 0 {
                return Err(Error::OutOfRange {
                    offset: self.consumed,
                    len: n as usize,
                    source_len: self.consumed,
                });
            }
            self.out_pos += got;
            self.consumed += got as u64;
            n -= got as u64;
        }
        Ok(())
    }

    fn position(&self) -> u64 {
        self.consumed
    }

    fn total_hint(&self) -> Option<u64> {
        Some(self.expect)
    }
}

/// Walks records off a sequential stream.
///
/// `enter` descends into a container without buffering it, which is how the
/// walk stays bounded: the model record and the components record are entered,
/// and only a single definition is ever materialised.
pub struct RecordStream<R: SeqRead> {
    pub r: R,
    /// One header of pushback. Needed because entering a container means
    /// reading its first child to see whether it is an OverflowSizeTag, and a
    /// sequential stream cannot be rewound if it is not.
    pending: Option<Header>,
}

#[derive(Clone, Copy)]
pub struct Header {
    pub tag: u16,
    pub size: u64,
}

impl<R: SeqRead> RecordStream<R> {
    pub fn new(r: R) -> Self {
        Self { r, pending: None }
    }

    /// Read the next 6-byte header, or None at the end of the current range.
    pub fn header(&mut self, limit: u64) -> Result<Option<Header>> {
        if let Some(h) = self.pending.take() {
            return Ok(Some(h));
        }
        if self.r.position() + crate::tlv::HEADER_LEN as u64 > limit {
            return Ok(None);
        }
        let mut h = [0u8; crate::tlv::HEADER_LEN];
        match self.r.read_exact(&mut h) {
            Ok(()) => {}
            Err(Error::OutOfRange { .. }) => return Ok(None),
            Err(e) => return Err(e),
        }
        let tag = u16::from_le_bytes([h[0], h[1]]);
        let size = u32::from_le_bytes([h[2], h[3], h[4], h[5]]) as u64;
        Ok(Some(Header { tag, size }))
    }

    /// Materialise a payload into a caller-owned buffer, reused across calls.
    pub fn payload(&mut self, size: u64, into: &mut Vec<u8>) -> Result<()> {
        // A single record larger than this is not something we have ever seen
        // and is more likely a misread size than a real definition. Refusing
        // it keeps a corrupt file from asking for a gigabyte.
        const SANE_RECORD: u64 = 768 * 1024 * 1024;
        if size > SANE_RECORD {
            return Err(Error::Malformed("record claims an implausible size"));
        }
        // Below the cap the size is still only a claim. Grow the buffer as the
        // bytes arrive, at most doubling it, so a damaged size on a short
        // stream fails after allocating about what the stream holds rather
        // than what it claimed.
        const FIRST_STEP: usize = 64 * 1024;
        let size = size as usize;
        into.clear();
        while into.len() < size {
            let at = into.len();
            let n = core::cmp::min(size - at, core::cmp::max(at, FIRST_STEP));
            into.resize(at + n, 0);
            self.r.read_exact(&mut into[at..])?;
        }
        Ok(())
    }

    pub fn skip(&mut self, size: u64) -> Result<()> {
        self.r.skip(size)
    }

    pub fn position(&self) -> u64 {
        self.r.position()
    }

    /// Descend into a container and return the offset at which it ends.
    ///
    /// A container's declared size is only its low 32 bits. When the content
    /// needs more, the record's *first child* is an `OverflowSizeTag` whose
    /// four bytes are the high word - so the format carries records larger
    /// than 4 GB, and a reader that ignores the tag silently stops early on
    /// exactly the files that most need reading. In every file measured so far
    /// the high word is zero and the tag is pure overhead, which is precisely
    /// why it is easy to get wrong and never notice.
    pub fn enter(&mut self, h: &Header) -> Result<u64> {
        let payload_start = self.r.position();
        let mut size = h.size;
        // Only a record big enough to hold one can carry an OverflowSizeTag.
        // Reading a header out of a one-byte leaf would consume the records
        // that follow it, which is a corruption that looks like a malformed
        // file rather than like the bug it is - so the guard is not optional.
        if size >= crate::tlv::HEADER_LEN as u64 + 4 {
            if let Some(first) = self.header(payload_start + crate::tlv::HEADER_LEN as u64)? {
                if first.tag == crate::tags::OVERFLOW_SIZE_TAG && first.size == 4 {
                    let mut b = [0u8; 4];
                    self.r.read_exact(&mut b)?;
                    let high = u32::from_le_bytes(b) as u64;
                    size |= high << 32;
                } else {
                    self.pending = Some(first);
                }
            }
        }
        payload_start
            .checked_add(size)
            .ok_or(Error::Malformed("record end overflows"))
    }
}
