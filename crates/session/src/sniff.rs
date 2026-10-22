//! Telling whether a file already at a path is of a given format, so `save` and `export`
//! do not replace other kinds of files by mistake.

use std::io::Read;
use std::path::Path;

use crate::command::{CommandError, ErrorKind};

/// How much of a file is read to recognise it.
const HEAD: u64 = 64 * 1024;
/// How much of a `.gltf` is read looking for its `asset`.
const GLTF_HEAD: u64 = 16 * 1024 * 1024;

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
    /// A SketchUp model.
    Skp,
    /// An SVG drawing.
    Svg,
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
            Kind::Skp => "SketchUp model",
            Kind::Svg => "SVG",
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
    if kind == Kind::Gltf {
        // The `asset` may come after a long `accessors` list: read on, as far as it.
        return gltf_json(std::io::BufReader::new(file.take(GLTF_HEAD)));
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
        Kind::Skp => head.starts_with(&skp_magic()),
        Kind::Image => {
            head.starts_with(b"\x89PNG\r\n\x1a\n") || head.starts_with(&[0xFF, 0xD8, 0xFF])
        }
        Kind::Stl => {
            ascii_stl(head)
                || (len >= 84
                    && head.len() >= 84
                    && u64::from(u32::from_le_bytes(
                        head[80..84].try_into().expect("4 bytes"),
                    )) * 50
                        + 84
                        == len)
        }
        Kind::Gltf => gltf_json(head),
        Kind::Svg => {
            let text = String::from_utf8_lossy(head);
            let text = text.trim_start_matches('\u{feff}').trim_start();
            (text.starts_with("<?xml") || text.starts_with("<svg")) && text.contains("<svg")
        }
        Kind::Obj => keyword_text(head, len, OBJ_KEYWORDS, None),
        Kind::Mtl => keyword_text(head, len, MTL_KEYWORDS, Some("newmtl")),
    }
}

/// The start of every SketchUp file: `ff fe ff`, a length of 14 and "SketchUp Model" in
/// UTF-16LE.
fn skp_magic() -> Vec<u8> {
    let mut magic = vec![0xff, 0xfe, 0xff, 14];
    magic.extend("SketchUp Model".encode_utf16().flat_map(u16::to_le_bytes));
    magic
}

/// True when the regular file at `path` starts as a SketchUp file does.
pub(crate) fn is_skp(path: &Path) -> bool {
    let magic = skp_magic();
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::new();
    meta.is_file()
        && file.take(magic.len() as u64).read_to_end(&mut head).is_ok()
        && is_kind(&head, meta.len(), Kind::Skp)
}

/// ASCII STL: a first line `solid` (alone or followed by a name), then a line starting with
/// `facet` or `endsolid`.
fn ascii_stl(head: &[u8]) -> bool {
    let mut lines = head
        .split(|b| *b == b'\n')
        .map(<[u8]>::trim_ascii)
        .filter(|l| !l.is_empty());
    let first_word = |line: &[u8]| -> Vec<u8> {
        let end = line
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(line.len());
        line[..end].to_ascii_lowercase()
    };
    let Some(first) = lines.next() else {
        return false;
    };
    if first_word(first) != b"solid" {
        return false;
    }
    lines
        .next()
        .is_some_and(|l| matches!(first_word(l).as_slice(), b"facet" | b"endsolid"))
}

/// glTF JSON: an object whose `asset` holds a `version` string. Reading stops at the
/// `asset`.
fn gltf_json(reader: impl std::io::Read) -> bool {
    use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};
    use std::cell::Cell;

    #[derive(serde::Deserialize)]
    struct Asset {
        version: Option<String>,
    }
    struct Find<'a>(&'a Cell<bool>);
    impl<'de> Visitor<'de> for Find<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a glTF object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            while let Some(key) = map.next_key::<String>()? {
                if key == "asset" {
                    let asset: Asset = map.next_value()?;
                    self.0.set(asset.version.is_some());
                    // Found: stop reading.
                    return Err(de::Error::custom("found"));
                }
                map.next_value::<IgnoredAny>()?;
            }
            Ok(())
        }
    }
    let found = Cell::new(false);
    let _ = serde_json::Deserializer::from_reader(reader).deserialize_map(Find(&found));
    found.get()
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
    fn only_stl_text_is_stl() {
        assert!(kind_of("solid box\nfacet normal 0 0 1\n", Kind::Stl));
        assert!(kind_of("  SOLID\r\n  endsolid\r\n", Kind::Stl));
        assert!(kind_of("solid\n\nendsolid x\n", Kind::Stl));
        assert!(!kind_of("Solidarity forever\nfacet\n", Kind::Stl));
        assert!(!kind_of("solid ground\nis what we stand on\n", Kind::Stl));
        assert!(!kind_of("solid\n", Kind::Stl));
    }

    #[test]
    fn only_json_with_an_asset_version_is_gltf() {
        assert!(kind_of(r#"{"asset": {"version": "2.0"}}"#, Kind::Gltf));
        let long = format!(
            r#"{{"accessors": [{}], "asset": {{"generator": "x", "version": "2.0"}}}}"#,
            vec!["{\"count\": 3}"; 10_000].join(",")
        );
        assert!(gltf_json(long.as_bytes()));
        assert!(!kind_of(r#"{"asset": 1, "user": "data"}"#, Kind::Gltf));
        assert!(!kind_of(r#"{"asset": {"name": "x"}}"#, Kind::Gltf));
        assert!(!kind_of(r#"{"user": "\"asset\""}"#, Kind::Gltf));
        assert!(!kind_of("[1, 2]", Kind::Gltf));
        assert!(!kind_of("not json", Kind::Gltf));
    }

    #[test]
    fn the_sketchup_header_is_skp() {
        let mut head = vec![0xff, 0xfe, 0xff, 14];
        for c in "SketchUp Model{23.1.340}".encode_utf16() {
            head.extend(c.to_le_bytes());
        }
        assert!(is_kind(&head, 100, Kind::Skp));
        head[4] = b's';
        assert!(!is_kind(&head, 100, Kind::Skp));
        assert!(!is_kind(&[0xff, 0xfe], 2, Kind::Skp));
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
