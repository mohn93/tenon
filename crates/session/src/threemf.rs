//! Exporting parts as a 3MF package for 3D printers: one object per part, welded into an
//! indexed mesh in millimeters (Z up, as the model), colored through a base materials group,
//! each checked for being a closed, outward-facing mesh.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;

use kernel::{MaterialId, Scene, Triangle};

use crate::export::ExportObject;

const MODEL_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const CONTENT_TYPES: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    r#"<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
    r#"</Types>"#
);
const RELS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Target="/3D/3dmodel.model" Id="rel0" "#,
    r#"Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>"#,
    r#"</Relationships>"#
);

/// Meters to millimeters.
const MM: f64 = 1000.0;
/// The default material's color, as in the other exports (sRGB 0 to 255).
const DEFAULT_RGB: [u8; 3] = [237, 237, 230];

/// A part's mesh: welded positions (mm) and triangles as indices with the material of each.
struct Mesh {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<([u32; 3], Option<MaterialId>)>,
}

/// Welds `triangles` (meters): corners equal to a nanometer are one vertex; a triangle that
/// collapses to a line is dropped.
fn weld(triangles: &[Triangle]) -> Mesh {
    let mut index: HashMap<[i64; 3], u32> = HashMap::new();
    let mut mesh = Mesh {
        vertices: Vec::new(),
        triangles: Vec::new(),
    };
    for t in triangles {
        let ids = t.corners.map(|p| {
            let mm = [p.x * MM, p.y * MM, p.z * MM];
            let key = mm.map(|c| (c * 1e6).round() as i64);
            *index.entry(key).or_insert_with(|| {
                // Adding 0.0 turns a negative zero into a positive one.
                mesh.vertices.push(key.map(|k| k as f64 / 1e6 + 0.0));
                (mesh.vertices.len() - 1) as u32
            })
        });
        if ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2] {
            mesh.triangles.push((ids, t.front));
        }
    }
    mesh
}

/// Why `mesh` is not a closed solid with outward faces, or None when it is.
fn open_reason(mesh: &Mesh) -> Option<String> {
    if mesh.triangles.is_empty() {
        return Some("it has no triangles".into());
    }
    let mut sides: HashMap<(u32, u32), u32> = HashMap::new();
    for (t, _) in &mesh.triangles {
        for i in 0..3 {
            *sides.entry((t[i], t[(i + 1) % 3])).or_default() += 1;
        }
    }
    let (mut unpaired, mut shared) = (0, 0);
    let ends: std::collections::BTreeSet<(u32, u32)> =
        sides.keys().map(|&(a, b)| (a.min(b), a.max(b))).collect();
    for (a, b) in ends {
        let forward = sides.get(&(a, b)).copied().unwrap_or(0);
        let back = sides.get(&(b, a)).copied().unwrap_or(0);
        match (forward, back) {
            (1, 1) => {}
            (1, 0) | (0, 1) => unpaired += 1,
            _ => shared += 1,
        }
    }
    if unpaired + shared > 0 {
        let mut why = Vec::new();
        if unpaired > 0 {
            let s = if unpaired == 1 { "" } else { "s" };
            why.push(format!("{unpaired} edge{s} bounded by one triangle"));
        }
        if shared > 0 {
            let s = if shared == 1 { "" } else { "s" };
            why.push(format!(
                "{shared} edge{s} shared by more than two triangles or traversed twice the same way"
            ));
        }
        return Some(why.join(", "));
    }
    let volume: f64 = mesh
        .triangles
        .iter()
        .map(|(t, _)| {
            let [a, b, c] = t.map(|i| glam::DVec3::from(mesh.vertices[i as usize]));
            a.dot(b.cross(c))
        })
        .sum::<f64>()
        / 6.0;
    if volume <= 0.0 {
        return Some("its faces point inward or it encloses no volume".into());
    }
    None
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The package holding `parts` (name and triangles in meters), with a report per object.
/// Materials are looked up in `scene`.
pub(crate) fn encode(
    scene: &Scene,
    parts: &[(String, Vec<Triangle>)],
) -> Result<(Vec<u8>, Vec<ExportObject>), String> {
    let meshes: Vec<Mesh> = parts.iter().map(|(_, t)| weld(t)).collect();
    // The base materials: each one used, in first-use order; None is the default.
    let mut used: Vec<Option<MaterialId>> = Vec::new();
    for mesh in &meshes {
        for (_, m) in &mesh.triangles {
            let m = m.filter(|m| scene.material(*m).is_some());
            if !used.contains(&m) {
                used.push(m);
            }
        }
    }
    let slot = |m: Option<MaterialId>| {
        let m = m.filter(|m| scene.material(*m).is_some());
        used.iter().position(|u| *u == m).expect("listed")
    };
    let mut xml = String::new();
    let _ = write!(
        xml,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" \
         xml:lang=\"en-US\" xmlns=\"{MODEL_NS}\">\n\
         <metadata name=\"Application\">sketchup-alt</metadata>\n<resources>\n"
    );
    if !used.is_empty() {
        xml.push_str("<basematerials id=\"1\">\n");
        for m in &used {
            let (name, rgb, opacity) = match m.and_then(|m| scene.material(m)) {
                Some(m) => (m.name.as_str(), m.color, m.opacity),
                None => ("default", DEFAULT_RGB, 1.0),
            };
            let alpha = (f64::from(opacity).clamp(0.0, 1.0) * 255.0).round() as u8;
            let _ = writeln!(
                xml,
                "<base name=\"{}\" displaycolor=\"#{:02X}{:02X}{:02X}{alpha:02X}\"/>",
                escape(name),
                rgb[0],
                rgb[1],
                rgb[2]
            );
        }
        xml.push_str("</basematerials>\n");
    }
    let mut objects = Vec::new();
    for (i, ((name, _), mesh)) in parts.iter().zip(&meshes).enumerate() {
        let id = i + 2;
        // The object's own color is its most used one.
        let mut counts = vec![0usize; used.len()];
        for (_, m) in &mesh.triangles {
            counts[slot(*m)] += 1;
        }
        let main = (0..counts.len()).max_by_key(|&k| (counts[k], std::cmp::Reverse(k)));
        let pid = main.map_or(String::new(), |k| format!(" pid=\"1\" pindex=\"{k}\""));
        let _ = writeln!(
            xml,
            "<object id=\"{id}\" type=\"model\" name=\"{}\"{pid}>\n<mesh>\n<vertices>",
            escape(name)
        );
        for v in &mesh.vertices {
            let _ = writeln!(
                xml,
                "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
                v[0], v[1], v[2]
            );
        }
        xml.push_str("</vertices>\n<triangles>\n");
        for (t, m) in &mesh.triangles {
            let _ = writeln!(
                xml,
                "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"1\" p1=\"{}\"/>",
                t[0],
                t[1],
                t[2],
                slot(*m)
            );
        }
        xml.push_str("</triangles>\n</mesh>\n</object>\n");
        let why = open_reason(mesh);
        objects.push(ExportObject {
            name: name.clone(),
            vertices: mesh.vertices.len(),
            triangles: mesh.triangles.len(),
            closed: why.is_none(),
            why,
        });
    }
    xml.push_str("</resources>\n<build>\n");
    for i in 0..parts.len() {
        let _ = writeln!(xml, "<item objectid=\"{}\"/>", i + 2);
    }
    xml.push_str("</build>\n</model>\n");

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in [
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("3D/3dmodel.model", xml.as_bytes()),
    ] {
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
    }
    Ok((
        zip.finish().map_err(|e| e.to_string())?.into_inner(),
        objects,
    ))
}
