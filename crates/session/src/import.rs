//! Reading OBJ and STL files for import: positions, polygons and (OBJ) material names, in the
//! file's own coordinates. `Document` turns them into a group, a component or geometry of the
//! open context (see `Command::Import`).

use std::collections::HashMap;
use std::path::Path;

use glam::DVec3;
use kernel::{DefinitionId, EntityId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Largest file `import` reads, in bytes.
pub const MAX_IMPORT_BYTES: u64 = 128 * 1024 * 1024;

/// Most polygons (STL: triangles) one imported file may have.
pub const MAX_IMPORT_POLYGONS: usize = 200_000;

/// Most positions (OBJ `v` lines) one imported file may have.
pub const MAX_IMPORT_POINTS: usize = 1_000_000;

/// Most new materials one import may add from the OBJ's libraries.
pub const MAX_IMPORT_MATERIALS: usize = 1024;

/// Most faces an import into the open context (`"as": "context"`) may make: those are drawn
/// with sticky geometry, which takes seconds for this many in a release build.
pub const MAX_CONTEXT_IMPORT_FACES: usize = 10_000;

/// Largest material library (`.mtl`) read beside an OBJ, in bytes; a larger one is ignored.
pub(crate) const MAX_MTL_BYTES: u64 = 16 * 1024 * 1024;

/// An import file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    /// Wavefront OBJ (text), with materials from its `.mtl` libraries.
    Obj,
    /// STL, binary or ASCII.
    Stl,
    /// SketchUp, from SketchUp 2017 or newer.
    Skp,
}

impl ImportFormat {
    /// The format a file name's extension asks for (`.obj`, `.stl`, `.skp`, any case).
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "obj" => ImportFormat::Obj,
            "stl" => ImportFormat::Stl,
            "skp" => ImportFormat::Skp,
            _ => return None,
        })
    }

    /// The axis that points up in files of this format unless told otherwise: Y for OBJ, Z
    /// for STL (as `export` writes them) and SketchUp (which is Z-up; `up` is ignored).
    pub fn default_up(self) -> UpAxis {
        match self {
            ImportFormat::Obj => UpAxis::Y,
            ImportFormat::Stl | ImportFormat::Skp => UpAxis::Z,
        }
    }
}

/// The file axis that points up. The model is Z-up; a Y-up file's (x, y, z) becomes
/// (x, -z, y), undoing what `export` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpAxis {
    Y,
    Z,
}

impl UpAxis {
    /// A file position in model coordinates (before scaling).
    pub fn to_model(self, p: DVec3) -> DVec3 {
        match self {
            UpAxis::Y => DVec3::new(p.x, -p.z, p.y),
            UpAxis::Z => p,
        }
    }
}

/// What an import makes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportAs {
    /// A new group, named after the file, placed in the open context.
    #[default]
    Group,
    /// A new component, named after the file, placed in the open context.
    Component,
    /// Loose geometry in the open context, merging with what it touches (as if the group
    /// were made and exploded).
    Context,
}

/// The length one unit of an imported OBJ or STL file stands for (the files carry no units):
/// the window's File > Import Units, which sets the `import` command's `scale`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportUnits {
    #[default]
    Meters,
    Centimeters,
    Millimeters,
    Inches,
    Feet,
}

impl ImportUnits {
    pub const ALL: [ImportUnits; 5] = [
        ImportUnits::Meters,
        ImportUnits::Centimeters,
        ImportUnits::Millimeters,
        ImportUnits::Inches,
        ImportUnits::Feet,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ImportUnits::Meters => "Meters",
            ImportUnits::Centimeters => "Centimeters",
            ImportUnits::Millimeters => "Millimeters",
            ImportUnits::Inches => "Inches",
            ImportUnits::Feet => "Feet",
        }
    }

    /// Meters per file unit: the import command's `scale`.
    pub fn meters(self) -> f64 {
        match self {
            ImportUnits::Meters => 1.0,
            ImportUnits::Centimeters => 0.01,
            ImportUnits::Millimeters => 0.001,
            ImportUnits::Inches => 0.0254,
            ImportUnits::Feet => 0.3048,
        }
    }
}

/// What an `import` command read and made.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImportReport {
    /// The group or component instance placed (absent for `"as": "context"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<EntityId>,
    /// Its definition (absent for `"as": "context"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<DefinitionId>,
    /// Polygons in the file (STL: triangles; SketchUp: faces of the root and of the
    /// definitions it places, each definition once).
    pub polygons: usize,
    /// Faces made from them, after coplanar neighbours merged (SketchUp: no merging).
    pub faces: usize,
    /// Vertices made: coincident corners are welded into one.
    pub vertices: usize,
    /// Polygons that made no face: fewer than three distinct corners, no area, the same
    /// corners as an earlier polygon, or (SketchUp) an outer loop that could not be read.
    pub skipped: usize,
    /// Materials added: from the OBJ's material library, or the SketchUp file's materials
    /// its faces and instances use.
    pub materials: usize,
    /// SketchUp: tags added, one per layer of the file but Layer0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tags: usize,
    /// SketchUp: scenes added, one per scene of a SketchUp 2021 or newer file.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub scenes: usize,
    /// SketchUp: definitions made from the file's (each once, however often it is placed).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub definitions: usize,
    /// SketchUp: instances made from the file's.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub instances: usize,
    /// Guides made.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub guides: usize,
    /// SketchUp: materials added with a texture.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub textures: usize,
    /// SketchUp: what the file has that the import left out or changed, as sentences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// A polygon as read: corner indices into the points, and its OBJ material name.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReadPolygon {
    pub corners: Vec<usize>,
    pub material: Option<String>,
}

/// A file's mesh as read, in the file's coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ReadMesh {
    pub points: Vec<DVec3>,
    pub polygons: Vec<ReadPolygon>,
    /// Material libraries the OBJ names (`mtllib`), as written.
    pub libraries: Vec<String>,
}

/// Reads an OBJ: `v` positions (extra numbers ignored), `f` polygons (corners `v`, `v/vt`,
/// `v/vt/vn` or `v//vn`; negative indices count back from the last position), `usemtl` and
/// `mtllib`; lines may continue with a trailing backslash. Everything else (normals, texture
/// coordinates, lines, groups, smoothing) is ignored.
pub(crate) fn read_obj(bytes: &[u8]) -> Result<ReadMesh, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut mesh = ReadMesh::default();
    let mut material: Option<String> = None;
    let mut pending = String::new();
    let mut start = 0;
    let mut corner_total = 0usize;
    for (n, raw) in text.lines().enumerate() {
        let raw = raw.split('#').next().unwrap_or("");
        if pending.is_empty() {
            start = n + 1;
        }
        if let Some(cont) = raw.trim_end().strip_suffix('\\') {
            pending.push_str(cont);
            pending.push(' ');
            continue;
        }
        pending.push_str(raw);
        let line = std::mem::take(&mut pending);
        let at = |what: String| format!("line {start}: {what}");
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("v") => {
                let mut xyz = [0.0; 3];
                for c in &mut xyz {
                    let t = tokens
                        .next()
                        .ok_or_else(|| at("a position needs x, y and z".into()))?;
                    *c = number(t).map_err(at)?;
                }
                if mesh.points.len() == MAX_IMPORT_POINTS {
                    return Err(format!("more than {MAX_IMPORT_POINTS} positions"));
                }
                mesh.points.push(DVec3::from(xyz));
            }
            Some("f") => {
                let mut corners = Vec::new();
                for t in tokens {
                    let index = t.split('/').next().unwrap_or("");
                    let i: i64 = index
                        .parse()
                        .map_err(|_| at("a corner index is not a whole number".into()))?;
                    let count = mesh.points.len() as i64;
                    let resolved = if i > 0 { i - 1 } else { count + i };
                    if i == 0 || !(0..count).contains(&resolved) {
                        return Err(at(format!("corner {i} names no position")));
                    }
                    if corner_total == MAX_IMPORT_POINTS {
                        return Err(at(format!("more than {MAX_IMPORT_POINTS} corners in all")));
                    }
                    corner_total += 1;
                    corners.push(resolved as usize);
                }
                if mesh.polygons.len() == MAX_IMPORT_POLYGONS {
                    return Err(format!("more than {MAX_IMPORT_POLYGONS} polygons"));
                }
                mesh.polygons.push(ReadPolygon {
                    corners,
                    material: material.clone(),
                });
            }
            Some("usemtl") => {
                let name = line.trim_start()["usemtl".len()..].trim();
                material = (!name.is_empty()).then(|| name.to_string());
            }
            Some("mtllib") => {
                // A name with spaces (export writes the file's name as it is) is the whole
                // rest of the line; the words alone are the fallback for several libraries.
                let whole = line.trim_start()["mtllib".len()..].trim();
                let words: Vec<&str> = tokens.collect();
                if words.len() > 1 {
                    mesh.libraries.push(whole.to_string());
                }
                mesh.libraries.extend(words.into_iter().map(str::to_string));
            }
            _ => {}
        }
    }
    Ok(mesh)
}

/// A finite number. The error does not quote the text (it is the file's).
fn number(t: &str) -> Result<f64, String> {
    t.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| "a coordinate is not a finite number".to_string())
}

/// The diffuse colors (`Kd`, linear) of an MTL's materials, as sRGB bytes, in the order the
/// names first got a color; a later `Kd` of the same name replaces the color. Keywords are
/// whole words (`newmtlX` is nothing), separated by any whitespace. Materials without `Kd`
/// are left out.
pub(crate) fn read_mtl(bytes: &[u8]) -> Vec<(String, [u8; 3])> {
    let text = String::from_utf8_lossy(bytes);
    let mut out: Vec<(String, [u8; 3])> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("newmtl") => {
                let name = line["newmtl".len()..].trim();
                current = (!name.is_empty()).then(|| name.to_string());
            }
            Some("Kd") => {
                let Some(name) = &current else { continue };
                let values: Vec<f64> = tokens.take(3).filter_map(|t| number(t).ok()).collect();
                if let [r, g, b] = values[..] {
                    let color = [r, g, b].map(srgb_byte);
                    match at.get(name) {
                        Some(&i) => out[i].1 = color,
                        None => {
                            at.insert(name.clone(), out.len());
                            out.push((name.clone(), color));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A linear color component (0 to 1, clamped) as an sRGB byte.
fn srgb_byte(linear: f64) -> u8 {
    let l = linear.clamp(0.0, 1.0);
    let s = if l <= 0.003_130_8 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Reads an STL: binary when its size is 84 bytes plus 50 per triangle its header counts,
/// else ASCII (`solid ... facet ... outer loop vertex x y z ... endloop ... endsolid`). Each
/// triangle has corners of its own (import welds them); stored normals are ignored, the
/// corner order gives the front.
pub(crate) fn read_stl(bytes: &[u8]) -> Result<ReadMesh, String> {
    let binary_count = (bytes.len() >= 84)
        .then(|| u32::from_le_bytes(bytes[80..84].try_into().expect("4 bytes")) as u64)
        .filter(|n| 84 + 50 * n == bytes.len() as u64);
    let triangles = match binary_count {
        Some(n) => {
            if n as usize > MAX_IMPORT_POLYGONS {
                return Err(format!("more than {MAX_IMPORT_POLYGONS} triangles"));
            }
            let mut out = Vec::with_capacity(n as usize);
            for k in 0..n as usize {
                let record = &bytes[84 + 50 * k..84 + 50 * (k + 1)];
                let value = |i: usize| {
                    let at = 12 + 4 * i;
                    f64::from(f32::from_le_bytes(
                        record[at..at + 4].try_into().expect("4 bytes"),
                    ))
                };
                let corner =
                    |c: usize| DVec3::new(value(3 * c), value(3 * c + 1), value(3 * c + 2));
                let t = vec![corner(0), corner(1), corner(2)];
                if !t.iter().all(|p| p.is_finite()) {
                    return Err(format!(
                        "triangle {} has a number that is not finite",
                        k + 1
                    ));
                }
                out.push(t);
            }
            out
        }
        None => read_ascii_stl(bytes)?,
    };
    let mut mesh = ReadMesh::default();
    for t in triangles {
        let first = mesh.points.len();
        mesh.polygons.push(ReadPolygon {
            corners: (first..first + t.len()).collect(),
            material: None,
        });
        mesh.points.extend(t);
    }
    Ok(mesh)
}

fn read_ascii_stl(bytes: &[u8]) -> Result<Vec<Vec<DVec3>>, String> {
    let text = String::from_utf8_lossy(bytes);
    if !text
        .trim_start()
        .get(..5)
        .is_some_and(|w| w.eq_ignore_ascii_case("solid"))
    {
        return Err(if bytes.len() >= 84 {
            "not an STL file: binary size does not match its triangle count, and it does not \
             start with \"solid\""
                .into()
        } else {
            "not an STL file".into()
        });
    }
    let mut out = Vec::new();
    let mut corners: Vec<DVec3> = Vec::new();
    let mut tokens = text.split_whitespace();
    let mut total = 0usize;
    while let Some(t) = tokens.next() {
        let word = t.to_ascii_lowercase();
        match word.as_str() {
            "vertex" => {
                if total == MAX_IMPORT_POINTS {
                    return Err(format!("more than {MAX_IMPORT_POINTS} vertices"));
                }
                total += 1;
                let mut xyz = [0.0; 3];
                for c in &mut xyz {
                    let t = tokens.next().ok_or_else(|| {
                        format!("facet {}: a vertex needs x, y and z", out.len() + 1)
                    })?;
                    *c = number(t).map_err(|e| format!("facet {}: {e}", out.len() + 1))?;
                }
                corners.push(DVec3::from(xyz));
            }
            "endloop" => {
                if out.len() == MAX_IMPORT_POLYGONS {
                    return Err(format!("more than {MAX_IMPORT_POLYGONS} triangles"));
                }
                out.push(std::mem::take(&mut corners));
            }
            _ => {}
        }
    }
    if !corners.is_empty() {
        return Err(format!(
            "facet {}: vertices without an endloop",
            out.len() + 1
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::linear;

    #[test]
    fn obj_reads_positions_polygons_negative_indices_and_materials() {
        let text = "# a comment\nmtllib a.mtl b.mtl\nv 0 0 0\nv 1 0 0 1.0\nv 1 1 0\nv 0 1 \\\n 0\n\
                    vt 0 0\nvn 0 0 1\ng side\nusemtl Red\nf 1/1/1 2/1/1 3//1\nusemtl\nf -4 -2 -1 # tail\n";
        let mesh = read_obj(text.as_bytes()).unwrap();
        assert_eq!(mesh.points.len(), 4);
        assert_eq!(mesh.points[3], DVec3::new(0.0, 1.0, 0.0));
        assert_eq!(mesh.libraries, ["a.mtl b.mtl", "a.mtl", "b.mtl"]);
        assert_eq!(
            mesh.polygons,
            vec![
                ReadPolygon {
                    corners: vec![0, 1, 2],
                    material: Some("Red".into())
                },
                ReadPolygon {
                    corners: vec![0, 2, 3],
                    material: None
                },
            ]
        );
    }

    #[test]
    fn obj_errors_name_the_line() {
        for (text, wanted) in [
            ("v 0 0\n", "line 1"),
            ("v 0 0 0\nv 1 0 nan\n", "line 2"),
            ("v 0 0 0\n\nf 1 2 3\n", "line 3"),
            ("v 0 0 0\nf 0 1 1\n", "names no position"),
            ("v 0 0 0\nf 1 -2 1\n", "names no position"),
            ("v 0 0 0\nf a b c\n", "corner index is not a whole number"),
        ] {
            let e = read_obj(text.as_bytes()).unwrap_err();
            assert!(e.contains(wanted), "{text:?}: {e}");
        }
    }

    #[test]
    fn mtl_colors_come_back_as_the_bytes_export_wrote() {
        for c in 0..=255u8 {
            let l = linear(f64::from(c) / 255.0);
            let text = format!("newmtl m\nKd {l:.6} {l:.6} {l:.6}\n");
            assert_eq!(read_mtl(text.as_bytes()), vec![("m".into(), [c; 3])]);
        }
        let text = "newmtl a\nKa 1 1 1\nnewmtl b\nKd 1 0 0\nnewmtl b\nKd 0 0 1\n";
        assert_eq!(read_mtl(text.as_bytes()), vec![("b".into(), [0, 0, 255])]);
    }

    #[test]
    fn mtl_keywords_are_whole_words_and_any_whitespace_separates() {
        let text = "newmtlX\nKd 1 1 1\nnewmtl\tTab Name\nKd\t1\t0\t0\nKdd 0 1 0\nKd1 0 1 0\n\
                    newmtl Two\nKd 0 1 0\nnewmtl Tab Name\nKd 0 0 1\n";
        assert_eq!(
            read_mtl(text.as_bytes()),
            vec![
                ("Tab Name".into(), [0, 0, 255]),
                ("Two".into(), [0, 255, 0])
            ]
        );
    }

    #[test]
    fn mtl_with_many_materials_reads_quickly() {
        let mut text = String::new();
        for i in 0..50_000 {
            text += &format!("newmtl m{i}\nKd 0.5 0.5 0.5\n");
        }
        let start = web_time::Instant::now();
        assert_eq!(read_mtl(text.as_bytes()).len(), 50_000);
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn stl_reads_ascii_and_binary_and_refuses_neither() {
        let ascii = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\n\
                     vertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let mesh = read_stl(ascii.as_bytes()).unwrap();
        assert_eq!(mesh.points.len(), 3);
        assert_eq!(mesh.polygons[0].corners, vec![0, 1, 2]);

        // A binary file whose header starts with "solid", as some exporters write.
        let mut bin = b"solid binary".to_vec();
        bin.resize(80, 0);
        bin.extend_from_slice(&1u32.to_le_bytes());
        for v in [
            [0.0f32, 0.0, 1.0],
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
        ] {
            for x in v {
                bin.extend_from_slice(&x.to_le_bytes());
            }
        }
        bin.extend_from_slice(&[0, 0]);
        let mesh = read_stl(&bin).unwrap();
        assert_eq!(mesh.points[1], DVec3::new(2.0, 0.0, 0.0));

        bin.push(0);
        assert!(
            read_stl(&bin).is_ok(),
            "a size mismatch falls back to ASCII"
        );
        let e = read_stl(&[7u8; 100]).unwrap_err();
        assert!(e.contains("not an STL file"), "{e}");
    }

    #[test]
    fn y_up_files_turn_back_into_z_up() {
        assert_eq!(
            UpAxis::Y.to_model(DVec3::new(1.0, 2.0, 3.0)),
            DVec3::new(1.0, -3.0, 2.0)
        );
        assert_eq!(UpAxis::Z.to_model(DVec3::ONE), DVec3::ONE);
    }

    #[test]
    fn corner_totals_are_bounded_and_ascii_stl_is_case_blind() {
        let mut text = String::from("v 0 0 0\n");
        let face = format!("f{}\n", " 1".repeat(MAX_IMPORT_POINTS + 1));
        text.push_str(&face);
        let e = read_obj(text.as_bytes()).unwrap_err();
        assert!(e.contains("corners"), "{e}");
        let mut stl = String::from("SOLID t\nFACET NORMAL 0 0 1\nOUTER LOOP\n");
        stl.push_str(&"VERTEX 0 0 0\n".repeat(MAX_IMPORT_POINTS + 1));
        let e = read_stl(stl.as_bytes()).unwrap_err();
        assert!(e.contains("more than"), "{e}");
        let ok = "SOLID t\nOUTER LOOP\nVERTEX 0 0 0\nVERTEX 1 0 0\nVERTEX 0 1 0\nENDLOOP\n";
        assert_eq!(read_stl(ok.as_bytes()).unwrap().polygons.len(), 1);
        let e = read_stl(b"solid t\nvertex 0 0 0\nvertex 1 0").unwrap_err();
        assert!(e.contains("facet 1"), "{e}");
        let e = read_stl(b"solid t\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\n").unwrap_err();
        assert!(e.contains("endloop"), "{e}");
    }
}
