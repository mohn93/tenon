//! Exporting parts as a 3MF package for 3D printers: one object per part, welded into an
//! indexed mesh in millimeters (Z up, as the model), colored through a base materials group
//! and textured through the Materials and Properties extension, each checked for being a
//! closed, outward-facing mesh. A definition placed more than once is one object that the
//! build items and component objects reference with their transforms.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;

use glam::{DAffine3, DVec2};
use kernel::{DefinitionId, MaterialId, Scene, SharedParts, Triangle};

use crate::export::{ExportObject, repeat_coords};

const MODEL_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const MATERIAL_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/material/2015/02";
const TEXTURE_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dtexture";
const RELS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Target="/3D/3dmodel.model" Id="rel0" "#,
    r#"Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>"#,
    r#"</Relationships>"#
);

/// `[Content_Types].xml`, with a default for each image extension in `images`.
fn content_types(images: &[(&str, &str)]) -> String {
    let mut s = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
        r#"<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
    ));
    for (extension, mime) in images {
        let _ = write!(
            s,
            r#"<Default Extension="{extension}" ContentType="{mime}"/>"#
        );
    }
    s.push_str("</Types>");
    s
}

/// Meters to millimeters.
const MM: f64 = 1000.0;
/// The default material's color, as in the other exports (sRGB 0 to 255).
const DEFAULT_RGB: [u8; 3] = [237, 237, 230];

/// One welded triangle: corner indices, the front material, and the texture positions
/// (meters) of the corners.
type MeshTriangle = ([u32; 3], Option<MaterialId>, [DVec2; 3]);

/// A part's mesh: welded positions (mm) and its triangles.
struct Mesh {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<MeshTriangle>,
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
            mesh.triangles.push((ids, t.front, t.uvs));
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
    for (t, ..) in &mesh.triangles {
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
        .map(|(t, ..)| {
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

/// A 3MF transform: twelve numbers, the rows of the matrix acting on row vectors, with the
/// translation (meters) in mm.
fn transform(t: &DAffine3) -> String {
    let m = t.matrix3;
    let v = [
        m.x_axis.x,
        m.x_axis.y,
        m.x_axis.z,
        m.y_axis.x,
        m.y_axis.y,
        m.y_axis.z,
        m.z_axis.x,
        m.z_axis.y,
        m.z_axis.z,
        t.translation.x * MM,
        t.translation.y * MM,
        t.translation.z * MM,
    ];
    v.map(|x| ((x * 1e9).round() / 1e9 + 0.0).to_string())
        .join(" ")
}

/// A textured material's resources: its `texture2d` and the coordinates its group holds.
struct TextureGroup {
    /// The group's resource id.
    id: usize,
    /// The `texture2d` resource id.
    texture: usize,
    coords: Vec<[f64; 2]>,
    index: HashMap<[u64; 2], usize>,
    width: f64,
    height: f64,
}

/// The package holding `parts`, with a report per object that has faces of its own. An object
/// whose triangles all collapse is reported but not written (an object needs triangles); when
/// no object is left that is an error. `name_of` names an object's definition. Materials are
/// looked up in `scene`; a textured material's triangles carry its image's coordinates.
pub(crate) fn encode(
    scene: &Scene,
    parts: &SharedParts,
    name_of: &dyn Fn(Option<DefinitionId>) -> String,
) -> Result<(Vec<u8>, Vec<ExportObject>), String> {
    let meshes: Vec<Mesh> = parts.objects.iter().map(|o| weld(&o.triangles)).collect();
    if meshes.iter().all(|m| m.triangles.is_empty()) {
        return Err("nothing to export: every triangle collapses to a line".to_string());
    }
    // The base materials: each one used, in first-use order; None is the default.
    let mut used: Vec<Option<MaterialId>> = Vec::new();
    let mut slots: HashMap<Option<MaterialId>, usize> = HashMap::new();
    for mesh in &meshes {
        for (_, m, _) in &mesh.triangles {
            let m = m.filter(|m| scene.material(*m).is_some());
            slots.entry(m).or_insert_with(|| {
                used.push(m);
                used.len() - 1
            });
        }
    }
    let slot = |m: Option<MaterialId>| slots[&m.filter(|m| scene.material(*m).is_some())];
    // Resource ids: the base materials are 1, then the textures and their groups, then the
    // objects.
    let mut next_id = 2;
    let mut images: Vec<(std::sync::Arc<kernel::TextureImage>, usize)> = Vec::new();
    let mut groups: Vec<TextureGroup> = Vec::new();
    let mut group_of: HashMap<MaterialId, usize> = HashMap::new();
    for m in used.iter().flatten() {
        let Some(texture) = scene.material(*m).and_then(|m| m.texture.as_ref()) else {
            continue;
        };
        let texture_id = match images
            .iter()
            .find(|(i, _)| std::sync::Arc::ptr_eq(i, &texture.image))
        {
            Some((_, id)) => *id,
            None => {
                images.push((texture.image.clone(), next_id));
                next_id += 1;
                next_id - 1
            }
        };
        group_of.insert(*m, groups.len());
        groups.push(TextureGroup {
            id: next_id,
            texture: texture_id,
            coords: Vec::new(),
            index: HashMap::new(),
            width: texture.width,
            height: texture.height,
        });
        next_id += 1;
    }
    let mut body = String::new();
    let mut objects = Vec::new();
    // The id each object is referenced by: its component object when it places others, else
    // its mesh.
    let mut ref_id: Vec<Option<usize>> = Vec::new();
    for (object, mesh) in parts.objects.iter().zip(&meshes) {
        let name = name_of(object.definition);
        let has_refs = object.refs.iter().any(|(i, _)| ref_id[*i].is_some());
        let mesh_name = if has_refs {
            format!("{name} (body)")
        } else {
            name.clone()
        };
        if !object.triangles.is_empty() {
            let why = open_reason(mesh);
            objects.push(ExportObject {
                name: mesh_name.clone(),
                vertices: mesh.vertices.len(),
                triangles: mesh.triangles.len(),
                closed: why.is_none(),
                why,
            });
        }
        let mut mesh_id = None;
        if !mesh.triangles.is_empty() {
            let id = next_id;
            next_id += 1;
            mesh_id = Some(id);
            // The object's own color is its most used one.
            let mut counts = vec![0usize; used.len()];
            for (_, m, _) in &mesh.triangles {
                counts[slot(*m)] += 1;
            }
            let main = (0..counts.len()).max_by_key(|&k| (counts[k], std::cmp::Reverse(k)));
            let pid = main.map_or(String::new(), |k| format!(" pid=\"1\" pindex=\"{k}\""));
            let _ = writeln!(
                body,
                "<object id=\"{id}\" type=\"model\" name=\"{}\"{pid}>\n<mesh>\n<vertices>",
                escape(&mesh_name)
            );
            for v in &mesh.vertices {
                let _ = writeln!(
                    body,
                    "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
                    v[0], v[1], v[2]
                );
            }
            body.push_str("</vertices>\n<triangles>\n");
            for (t, m, uvs) in &mesh.triangles {
                let m = m.filter(|m| scene.material(*m).is_some());
                let group = m.and_then(|m| group_of.get(&m).copied());
                match group {
                    Some(k) => {
                        let g = &mut groups[k];
                        // The image's v counts up from its bottom (3MF as OBJ); `texcoords`
                        // counts down, so the sign flips, and the whole-repeat shift stays.
                        let st = repeat_coords(*uvs, g.width, g.height);
                        let p = st.map(|q| {
                            let c = [q.x + 0.0, -q.y + 0.0];
                            let key = c.map(|x| x.to_bits());
                            *g.index.entry(key).or_insert_with(|| {
                                g.coords.push(c);
                                g.coords.len() - 1
                            })
                        });
                        let _ = writeln!(
                            body,
                            "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"{}\" p1=\"{}\" p2=\"{}\" p3=\"{}\"/>",
                            t[0], t[1], t[2], g.id, p[0], p[1], p[2]
                        );
                    }
                    None => {
                        let _ = writeln!(
                            body,
                            "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" pid=\"1\" p1=\"{}\"/>",
                            t[0],
                            t[1],
                            t[2],
                            slot(m)
                        );
                    }
                }
            }
            body.push_str("</triangles>\n</mesh>\n</object>\n");
        }
        if has_refs {
            let id = next_id;
            next_id += 1;
            let _ = writeln!(
                body,
                "<object id=\"{id}\" type=\"model\" name=\"{}\">\n<components>",
                escape(&name)
            );
            if let Some(m) = mesh_id {
                let _ = writeln!(body, "<component objectid=\"{m}\"/>");
            }
            for (i, place) in &object.refs {
                if let Some(r) = ref_id[*i] {
                    let _ = writeln!(
                        body,
                        "<component objectid=\"{r}\" transform=\"{}\"/>",
                        transform(place)
                    );
                }
            }
            body.push_str("</components>\n</object>\n");
            ref_id.push(Some(id));
        } else {
            ref_id.push(mesh_id);
        }
    }
    let mut xml = String::new();
    let extension = if groups.is_empty() {
        String::new()
    } else {
        format!(" xmlns:m=\"{MATERIAL_NS}\"")
    };
    let _ = write!(
        xml,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" \
         xml:lang=\"en-US\" xmlns=\"{MODEL_NS}\"{extension}>\n\
         <metadata name=\"Application\">tenon</metadata>\n<resources>\n"
    );
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
    let mut files: Vec<(String, std::sync::Arc<kernel::TextureImage>)> = Vec::new();
    for (k, (image, id)) in images.iter().enumerate() {
        let path = format!("3D/Textures/texture{k}.{}", image.format.extension());
        let _ = writeln!(
            xml,
            "<m:texture2d id=\"{id}\" path=\"/{path}\" contenttype=\"{}\" \
             tilestyleu=\"wrap\" tilestylev=\"wrap\"/>",
            image.format.mime()
        );
        files.push((path, image.clone()));
    }
    for g in &groups {
        let _ = writeln!(
            xml,
            "<m:texture2dgroup id=\"{}\" texid=\"{}\">",
            g.id, g.texture
        );
        for [u, v] in &g.coords {
            let _ = writeln!(xml, "<m:tex2coord u=\"{u}\" v=\"{v}\"/>");
        }
        xml.push_str("</m:texture2dgroup>\n");
    }
    xml.push_str(&body);
    xml.push_str("</resources>\n<build>\n");
    for (i, place) in &parts.items {
        let Some(id) = ref_id[*i] else { continue };
        if *place == DAffine3::IDENTITY {
            let _ = writeln!(xml, "<item objectid=\"{id}\"/>");
        } else {
            let _ = writeln!(
                xml,
                "<item objectid=\"{id}\" transform=\"{}\"/>",
                transform(place)
            );
        }
    }
    xml.push_str("</build>\n</model>\n");

    let mut extensions: Vec<(&str, &str)> = Vec::new();
    for (_, image) in &files {
        let e = (image.format.extension(), image.format.mime());
        if !extensions.contains(&e) {
            extensions.push(e);
        }
    }
    let types = content_types(&extensions);
    let mut texture_rels = String::new();
    for (k, (path, _)) in files.iter().enumerate() {
        let _ = write!(
            texture_rels,
            "<Relationship Target=\"/{path}\" Id=\"rel{k}\" Type=\"{TEXTURE_REL}\"/>"
        );
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), types.into_bytes()),
        ("_rels/.rels".into(), RELS.as_bytes().to_vec()),
    ];
    if !files.is_empty() {
        let rels = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             {texture_rels}</Relationships>"
        );
        entries.push(("3D/_rels/3dmodel.model.rels".into(), rels.into_bytes()));
    }
    entries.push(("3D/3dmodel.model".into(), xml.into_bytes()));
    for (path, image) in &files {
        entries.push((path.clone(), image.encoded.clone()));
    }
    for (name, bytes) in entries {
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    Ok((
        zip.finish().map_err(|e| e.to_string())?.into_inner(),
        objects,
    ))
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};

    use super::*;

    fn triangle(corners: [DVec3; 3]) -> Triangle {
        Triangle {
            corners,
            normal: DVec3::Z,
            normals: [DVec3::Z; 3],
            uvs: [DVec2::ZERO; 3],
            front: None,
            back: None,
        }
    }

    #[test]
    fn a_part_whose_triangles_all_collapse_is_not_written() {
        // Two corners within a nanometer weld into one.
        let line = triangle([DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y * 1e-13]);
        let flat = triangle([DVec3::ZERO, DVec3::X, DVec3::Y]);
        let parts = SharedParts {
            objects: vec![
                kernel::PartObject {
                    triangles: vec![line],
                    ..Default::default()
                },
                kernel::PartObject {
                    triangles: vec![flat],
                    ..Default::default()
                },
            ],
            items: vec![(0, DAffine3::IDENTITY), (1, DAffine3::IDENTITY)],
            skipped: 0,
        };
        let names = |_: Option<DefinitionId>| "Part".to_string();
        let (bytes, objects) = encode(&Scene::new(), &parts, &names).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("3D/3dmodel.model").unwrap(), &mut xml)
            .unwrap();
        // Every object written holds triangles, and every build item names one.
        assert_eq!(xml.matches("<object ").count(), 1, "{xml}");
        assert!(!xml.contains("<triangles>\n</triangles>"), "{xml}");
        assert!(xml.contains("<item objectid=\"2\"/>") && !xml.contains("objectid=\"3\""));
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].why.as_deref(), Some("it has no triangles"));
        // Nothing but collapsed triangles is nothing to write.
        let only_line = SharedParts {
            objects: parts.objects[..1].to_vec(),
            items: vec![(0, DAffine3::IDENTITY)],
            skipped: 0,
        };
        assert!(encode(&Scene::new(), &only_line, &names).is_err());
    }
}
