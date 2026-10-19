use core::fmt;

#[derive(Debug)]
pub enum Error {
    /// A read ran past the end of the source, or a length field claimed more
    /// bytes than exist. Always a malformed file, never a bug in the caller.
    OutOfRange {
        offset: u64,
        len: usize,
        source_len: u64,
    },
    /// The file does not begin with the UTF-16 "SketchUp Model" header.
    NotSkp,
    /// No end-of-central-directory record in the last 64 KiB.
    NoZipDirectory,
    /// A zip field we do not support, named so the message is actionable.
    UnsupportedZip(&'static str),
    Inflate(&'static str),
    /// The bytes parse, but say something impossible. Distinct from
    /// OutOfRange so that "this file is truncated" and "this file is lying"
    /// read differently in a bug report.
    Malformed(&'static str),
    /// Work was cancelled from outside. Not a failure.
    Cancelled,
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OutOfRange {
                offset,
                len,
                source_len,
            } => write!(
                f,
                "read of {len} bytes at {offset} runs past the end of a {source_len} byte file"
            ),
            Error::NotSkp => write!(f, "not a SketchUp file: missing the UTF-16 model header"),
            Error::NoZipDirectory => write!(f, "no zip central directory in the last 64 KiB"),
            Error::UnsupportedZip(what) => write!(f, "unsupported zip feature: {what}"),
            Error::Inflate(what) => write!(f, "inflate failed: {what}"),
            Error::Malformed(what) => write!(f, "malformed file: {what}"),
            Error::Cancelled => write!(f, "cancelled"),
            Error::Io(what) => write!(f, "io error: {what}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

pub type Result<T> = core::result::Result<T, Error>;
