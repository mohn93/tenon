//! Path rules shared by `--api-root` and the OBJ importer.
//!
//! The Windows rules are pure functions on path text with Windows semantics, so they are
//! tested on every OS; `canonical`, `starts_with`, `same` and `is_link` apply them on Windows
//! and are the plain `std` behaviour elsewhere.

use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};

/// `\\?\C:\x` as `C:\x`, and `\\?\UNC\server\share\x` as `\\server\share\x`: the verbatim
/// forms `canonicalize` returns on Windows, written the way people and most programs write
/// them. Any other text is returned unchanged.
pub fn strip_verbatim(path: &str) -> Cow<'_, str> {
    let Some(rest) = path.strip_prefix(r"\\?\") else {
        return Cow::Borrowed(path);
    };
    let b = rest.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        return Cow::Borrowed(rest);
    }
    if rest
        .get(..4)
        .is_some_and(|p| p.eq_ignore_ascii_case(r"UNC\"))
    {
        return Cow::Owned(format!(r"\\{}", &rest[4..]));
    }
    Cow::Borrowed(path)
}

/// Whether the Windows path `path` is `base` or lies within it, comparing names without
/// regard to case (as Windows does) and either slash as a separator. Verbatim prefixes are
/// stripped first, so `\\?\C:\a\b` lies within `c:\A`.
pub fn windows_starts_with(path: &str, base: &str) -> bool {
    let (path, base) = (strip_verbatim(path), strip_verbatim(base));
    let (path_head, path_rest) = split_head(&path);
    let (base_head, base_rest) = split_head(&base);
    if path_head != base_head {
        return false;
    }
    let mut names = names(path_rest);
    self::names(base_rest).all(|b| names.next().is_some_and(|n| same_name(n, b)))
}

/// Why the API refuses the Windows path `path`, or None when it does not: a drive-relative
/// path (`C:foo`), a path rooted without a drive (`\foo`), a device path (`\\.\`, or a
/// verbatim one other than a drive or UNC), and names Windows reads specially: one with `:`
/// (an alternate data stream), a reserved device name (`CON`, `NUL`, `COM1`, ... with any
/// extension), or one ending in a dot or a space (which Windows drops).
pub fn windows_refusal(path: &str) -> Option<&'static str> {
    let plain = strip_verbatim(path);
    let (head, rest) = split_head(&plain);
    match head {
        Head::Unc if matches!(rest.split(is_separator).next(), Some("." | "?")) => {
            return Some("a device path");
        }
        Head::Drive(_, false) => return Some("a drive-relative path (such as C:foo)"),
        Head::Rooted => return Some(r"a path rooted without a drive (such as \foo)"),
        _ => {}
    }
    for name in names(rest).filter(|n| *n != "..") {
        if name.contains(':') {
            return Some("a name with ':' (an alternate data stream)");
        }
        if name.ends_with(['.', ' ']) {
            return Some("a name ending in a dot or a space, which Windows drops");
        }
        if reserved(name) {
            return Some("a reserved device name (such as CON, NUL or COM1)");
        }
    }
    None
}

/// Whether Windows reads `path` as absolute: a drive with a separator (`C:\`) or a share
/// (`\\server\share`), not a device path, and with no name `windows_refusal` refuses.
pub fn windows_is_absolute(path: &str) -> bool {
    let (head, _) = split_head(&strip_verbatim(path));
    matches!(head, Head::Drive(_, true) | Head::Unc) && windows_refusal(path).is_none()
}

/// How a Windows path starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Head {
    /// Two separators: a UNC share (`\\server\share`) or a device path.
    Unc,
    /// A drive letter (in capitals), and whether a separator follows.
    Drive(u8, bool),
    /// One separator: rooted on the current drive.
    Rooted,
    Relative,
}

fn is_separator(c: char) -> bool {
    c == '\\' || c == '/'
}

/// `path`'s start, and the rest after it.
fn split_head(path: &str) -> (Head, &str) {
    let b = path.as_bytes();
    let sep = |i: usize| b.get(i).is_some_and(|c| *c == b'\\' || *c == b'/');
    if sep(0) && sep(1) {
        (Head::Unc, &path[2..])
    } else if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        (Head::Drive(b[0].to_ascii_uppercase(), sep(2)), &path[2..])
    } else if sep(0) {
        (Head::Rooted, &path[1..])
    } else {
        (Head::Relative, path)
    }
}

/// The names in `rest`, without empty ones (repeated separators) and `.`.
fn names(rest: &str) -> impl Iterator<Item = &str> {
    rest.split(is_separator)
        .filter(|n| !n.is_empty() && *n != ".")
}

/// Whether two names are the same to Windows: equal after simple upper-casing.
fn same_name(a: &str, b: &str) -> bool {
    fn upper(c: char) -> char {
        let mut u = c.to_uppercase();
        match (u.next(), u.next()) {
            (Some(x), None) => x,
            _ => c,
        }
    }
    a.chars().map(upper).eq(b.chars().map(upper))
}

/// A name Windows opens as a device whatever folder it is in: `CON`, `NUL`, `COM1` and the
/// like, with or without an extension (and spaces before it).
fn reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let stem = stem.to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let numbered = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"));
    let mut digit = numbered.into_iter().flat_map(str::chars);
    matches!(
        (digit.next(), digit.next()),
        (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
    )
}

/// `path` resolved after links, as `canonicalize`; on Windows without the verbatim prefix,
/// unless the plain form would name another file.
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    let real = path.canonicalize()?;
    #[cfg(windows)]
    if let Some(text) = real.to_str() {
        let plain = strip_verbatim(text);
        if windows_refusal(&plain).is_none() {
            return Ok(PathBuf::from(plain.as_ref()));
        }
    }
    Ok(real)
}

/// Whether `path` is `base` or lies within it: `Path::starts_with`, and on Windows
/// `windows_starts_with` (a path that is not Unicode is compared as `Path::starts_with` does).
pub fn starts_with(path: &Path, base: &Path) -> bool {
    #[cfg(windows)]
    if let (Some(p), Some(b)) = (path.to_str(), base.to_str()) {
        return windows_starts_with(p, b);
    }
    path.starts_with(base)
}

/// Whether `a` and `b` name the same place (by `starts_with`'s rules).
pub fn same(a: &Path, b: &Path) -> bool {
    starts_with(a, b) && starts_with(b, a)
}

/// Whether `meta` (from `symlink_metadata`) is a link: a symbolic link, and on Windows any
/// reparse point (a junction, say).
pub fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_and_unc_prefixes_normalise() {
        assert_eq!(strip_verbatim(r"\\?\C:\work\a.skr"), r"C:\work\a.skr");
        assert_eq!(strip_verbatim(r"\\?\d:\"), r"d:\");
        assert_eq!(
            strip_verbatim(r"\\?\UNC\server\share\a"),
            r"\\server\share\a"
        );
        assert_eq!(strip_verbatim(r"\\?\unc\server\share"), r"\\server\share");
        // Already plain, or a verbatim form with no plain one: unchanged.
        for same in [
            r"C:\work",
            r"\\server\share\a",
            r"\\?\Volume{0b1c}\a",
            r"\\?\GLOBALROOT\Device",
            r"\\?\C:",
            "/home/me",
            r"\\?\ü",
        ] {
            assert_eq!(strip_verbatim(same), same);
        }
        // Verbatim and plain forms are judged alike.
        assert!(windows_starts_with(r"\\?\C:\work\a", r"C:\work"));
        assert!(windows_starts_with(r"C:\work\a", r"\\?\C:\work"));
        assert!(windows_starts_with(
            r"\\?\UNC\server\share\dir\a",
            r"\\server\share\dir"
        ));
        assert!(windows_starts_with(
            r"\\server\share\dir",
            r"\\server\share\"
        ));
    }

    #[test]
    fn windows_paths_compare_case_insensitively() {
        assert!(windows_starts_with(r"c:\Work\Sub\A.SKR", r"C:\work"));
        assert!(windows_starts_with(r"C:\ÄRGER\x", r"c:\ärger"));
        assert!(windows_starts_with(r"C:/work//sub/", r"C:\WORK\"));
        assert!(windows_starts_with(r"C:\work", r"C:\work"));
        assert!(windows_starts_with(r"C:\work", r"C:\"));
        // Whole names only, and the same start.
        assert!(!windows_starts_with(r"C:\workshop\a", r"C:\work"));
        assert!(!windows_starts_with(r"C:\work", r"C:\work\sub"));
        assert!(!windows_starts_with(r"D:\work\a", r"C:\work"));
        assert!(!windows_starts_with(r"\\work\a\b", r"C:\work"));
        assert!(!windows_starts_with(r"\\server\share\a", r"\server\share"));
        assert!(!windows_starts_with(r"work\a", r"\work"));
        assert!(!windows_starts_with(r"\\server\share\a", r"\\other\share"));
    }

    #[test]
    fn windows_absolute_paths() {
        for path in [
            r"C:\",
            r"c:/x",
            r"\\server\share\x",
            r"\\?\C:\x",
            r"\\?\UNC\s\h",
        ] {
            assert!(windows_is_absolute(path), "{path}");
        }
        for path in ["", "x", r"C:x", r"C:", r"\x", "/x", r"\\.\C:\x", r"C:\nul"] {
            assert!(!windows_is_absolute(path), "{path}");
        }
    }

    #[test]
    fn refused_windows_forms() {
        for (path, why) in [
            (r"C:foo", "drive-relative"),
            (r"C:", "drive-relative"),
            (r"\foo", "without a drive"),
            ("/foo", "without a drive"),
            (r"\\.\pipe\x", "device"),
            (r"\\.\C:\x", "device"),
            (r"\\?\Volume{0b1c}\a", "device"),
            (r"C:\work\a:stream", ":"),
            (r"ab:stream", ":"),
            // One letter before the colon reads as a drive.
            (r"a:stream", "drive-relative"),
            (r"C:\work\a.skr:x:$DATA", ":"),
            ("CON", "reserved"),
            ("nul.txt", "reserved"),
            (r"C:\work\Com1.tar.gz", "reserved"),
            (r"C:\work\lpt9", "reserved"),
            (r"C:\work\COM¹", "reserved"),
            (r"C:\work\aux .txt", "reserved"),
            (r"C:\work\a.", "dot or a space"),
            (r"C:\work\a \b", "dot or a space"),
            (r"\\?\C:\work\nul", "reserved"),
        ] {
            let reason = windows_refusal(path).unwrap_or_else(|| panic!("{path} accepted"));
            assert!(reason.contains(why), "{path}: {reason}");
        }
        for path in [
            r"C:\work\a.skr",
            r"c:/work/a.skr",
            r"\\server\share\a",
            r"\\?\C:\work\a",
            r"\\?\UNC\server\share\a",
            r"sub\a.skr",
            "a.skr",
            r"..\a",
            r".\a",
            r"C:\work\console.skr",
            r"C:\work\com10",
            r"C:\work\nullity",
            r"C:\work\.hidden",
            "",
        ] {
            assert_eq!(windows_refusal(path), None, "{path}");
        }
    }
}
