//! Telling whether a file already at a path is of a given format, so `save` and `export`
//! do not replace other kinds of files by mistake.

use std::io::Read;
use std::path::Path;

use crate::command::{CommandError, ErrorKind};

/// How much of a file is read to recognise it.
const HEAD: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Skr,
    Obj,
    Mtl,
    Stl,
    Glb,
    Gltf,
    /// A PNG or JPEG image.
    Image,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Skr => "sketchup-alt model",
            Kind::Obj => "OBJ",
            Kind::Mtl => "MTL material library",
            Kind::Stl => "STL",
            Kind::Glb => "binary glTF",
            Kind::Gltf => "glTF",
            Kind::Image => "PNG or JPEG",
        }
    }
}

const OBJ_KEYWORDS: &[&str] = &[
    "v",
    "vt",
    "vn",
    "vp",
    "f",
    "l",
    "p",
    "o",
    "g",
    "s",
    "mtllib",
    "usemtl",
    "cstype",
    "deg",
    "curv",
    "curv2",
    "surf",
    "parm",
    "end",
    "bmat",
    "step",
    "trim",
    "hole",
    "scrv",
    "sp",
    "con",
    "mg",
    "lod",
    "shadow_obj",
    "trace_obj",
];
const MTL_KEYWORDS: &[&str] = &[
    "newmtl",
    "Ka",
    "Kd",
    "Ks",
    "Ke",
    "Ns",
    "Ni",
    "d",
    "Tr",
    "Tf",
    "illum",
    "bump",
    "disp",
    "decal",
    "refl",
    "sharpness",
    "Pr",
    "Pm",
    "Ps",
    "Pc",
    "Pcr",
    "aniso",
    "anisor",
    "norm",
];

/// True when `path` may be replaced by a file of `kind`: nothing is there, or an empty
/// regular file, or a regular file of that kind. A link is judged by what it points to.
pub(crate) fn replaceable(path: &Path, kind: Kind) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return e.kind() == std::io::ErrorKind::NotFound,
    };
    if !meta.is_file() {
        return false;
    }
    if meta.len() == 0 {
        return true;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    if kind == Kind::Skr {
        return zip::ZipArchive::new(file).is_ok_and(|mut z| z.by_name("document.json").is_ok());
    }
    let mut head = Vec::new();
    if file.take(HEAD).read_to_end(&mut head).is_err() {
        return false;
    }
    is_kind(&head, meta.len(), kind)
}

/// Whether a file of `len` bytes starting with `head` is of `kind` (not `Skr`).
fn is_kind(head: &[u8], len: u64, kind: Kind) -> bool {
    match kind {
        Kind::Skr => false,
        Kind::Glb => head.starts_with(b"glTF"),
        Kind::Image => {
            head.starts_with(b"\x89PNG\r\n\x1a\n") || head.starts_with(&[0xFF, 0xD8, 0xFF])
        }
        Kind::Stl => {
            let text = head.trim_ascii_start();
            text.get(..5)
                .is_some_and(|w| w.eq_ignore_ascii_case(b"solid"))
                || (len >= 84
                    && head.len() >= 84
                    && u64::from(u32::from_le_bytes(
                        head[80..84].try_into().expect("4 bytes"),
                    )) * 50
                        + 84
                        == len)
        }
        Kind::Gltf => {
            head.trim_ascii_start().starts_with(b"{") && head.windows(7).any(|w| w == b"\"asset\"")
        }
        Kind::Obj => keyword_text(head, len, OBJ_KEYWORDS, None),
        Kind::Mtl => keyword_text(head, len, MTL_KEYWORDS, Some("newmtl")),
    }
}

/// UTF-8 text whose complete lines are blank, `#` comments or start with one of `keywords`
/// (or `map_...`, for MTL), with at least one keyword line and, if `needed`, that keyword.
fn keyword_text(head: &[u8], len: u64, keywords: &[&str], needed: Option<&str>) -> bool {
    let text = match std::str::from_utf8(head) {
        Ok(t) => t,
        // A head cut inside a character.
        Err(e) if e.error_len().is_none() => {
            std::str::from_utf8(&head[..e.valid_up_to()]).expect("valid up to here")
        }
        Err(_) => return false,
    };
    let mut lines: Vec<&str> = text.lines().collect();
    // The last line may be cut short when the file goes on past the head.
    if (head.len() as u64) < len && !text.ends_with('\n') {
        lines.pop();
    }
    let (mut any, mut has_needed) = (false, needed.is_none());
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let word = line.split_whitespace().next().expect("not blank");
        let known = keywords.contains(&word) || (needed.is_some() && word.starts_with("map_"));
        if !known {
            return false;
        }
        any = true;
        has_needed |= needed == Some(word);
    }
    any && has_needed
}

/// The error for a target that is not of `kind`.
pub(crate) fn refusal(path: &Path, kind: Kind) -> CommandError {
    CommandError::new(
        ErrorKind::InvalidInput,
        format!(
            "{} exists and is not a {} file: pass \"overwrite\": true to replace it",
            path.display(),
            kind.name()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_of(text: &str, kind: Kind) -> bool {
        is_kind(text.as_bytes(), text.len() as u64, kind)
    }

    #[test]
    fn an_exported_obj_head_is_obj() {
        let obj = "# x\nmtllib a.mtl\nv 0 0 0\nvn 0 0 1\nusemtl m\nf 1//1 1//1 1//1\n";
        assert!(kind_of(obj, Kind::Obj));
        assert!(!kind_of(obj, Kind::Mtl));
    }

    #[test]
    fn a_library_is_mtl() {
        let mtl = "newmtl a\nKd 1 1 1\nmap_Kd a.png\n";
        assert!(kind_of(mtl, Kind::Mtl));
        assert!(!kind_of(mtl, Kind::Obj));
    }

    #[test]
    fn other_text_is_neither() {
        for kind in [Kind::Obj, Kind::Mtl] {
            assert!(!kind_of("hello world\n", kind));
            assert!(!kind_of("# only a comment\n", kind));
        }
    }

    #[test]
    fn image_signatures_are_images() {
        assert!(is_kind(b"\x89PNG\r\n\x1a\nrest", 12, Kind::Image));
        assert!(is_kind(&[0xFF, 0xD8, 0xFF, 0xE0, 0], 5, Kind::Image));
        assert!(!is_kind(b"GIF89a", 6, Kind::Image));
    }

    #[test]
    fn a_head_cutting_a_character_in_two_still_reads_as_obj() {
        let mut text = String::from("v 0 0 0\n");
        while text.len() < HEAD as usize + 100 {
            text.push_str("# \u{e9}\u{e9}\u{e9}\u{e9}\n");
        }
        let bytes = text.as_bytes();
        // Cut so the last character is split.
        let mut cut = HEAD as usize;
        while std::str::from_utf8(&bytes[..cut]).is_ok() {
            cut += 1;
        }
        let head = &bytes[..cut];
        assert!(std::str::from_utf8(head).is_err());
        assert!(is_kind(head, bytes.len() as u64, Kind::Obj));
    }
}
