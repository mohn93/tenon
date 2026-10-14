//! Text for the Entity Info panel: what is selected and its measurements.

use glam::{DAffine3, DQuat};
use kernel::{CurveKind, DefinitionKind, EdgeId, EntityId, Guide, Model};
use session::Document;
use tools::vcb::format_length;

/// Lines describing the selection: counts, plus length, area or normal for a single entity,
/// the radius, segments, angle and length of a whole curve (a circle or arc), the direction
/// or position of a single guide, or the kind, name and rotation of a single instance; a single
/// face's front and back materials and a painted instance's material. While a group or
/// component is open, a first line says which.
pub fn entity_info(doc: &Document) -> Vec<String> {
    let mut lines = editing(doc).into_iter().collect::<Vec<_>>();
    lines.extend(selection_info(doc));
    lines
}

/// "Editing group" or "Editing component <name>" while one is open.
fn editing(doc: &Document) -> Option<String> {
    let (scene, path) = (doc.scene(), doc.context());
    let (last, outer) = path.split_last()?;
    let (parent, _) = scene.resolve(outer)?;
    let instance = scene.mesh(parent)?.instance(*last)?;
    let d = scene.definition(instance.definition)?;
    Some(match d.kind {
        DefinitionKind::Group => "Editing group".into(),
        DefinitionKind::Component => format!("Editing component {}", d.name),
    })
}

fn selection_info(doc: &Document) -> Vec<String> {
    let m = doc.model();
    let (mut edges, mut faces, mut instances) = (Vec::new(), Vec::new(), Vec::new());
    let mut guides = Vec::new();
    for id in doc.selection() {
        match id {
            EntityId::Edge(e) => edges.push(*e),
            EntityId::Face(f) => faces.push(*f),
            EntityId::Instance(i) => instances.push(*i),
            EntityId::Guide(g) => guides.push(*g),
            EntityId::Vertex(_) => {}
        }
    }
    if let ([], [], [], [g]) = (&edges[..], &faces[..], &instances[..], &guides[..]) {
        let xyz = |v: glam::DVec3| v.to_array().map(number).join(", ");
        let place = |v: glam::DVec3| v.to_array().map(format_length).join(", ");
        return match m.guide(*g) {
            Some(Guide::Line { point, direction }) => {
                vec![
                    "1 guide line".into(),
                    format!("Direction: {}", xyz(*direction)),
                    format!("Through: {}", place(*point)),
                ]
            }
            Some(Guide::Point { position }) => {
                vec![
                    "1 guide point".into(),
                    format!("Position: {}", place(*position)),
                ]
            }
            None => Vec::new(),
        };
    }
    let guides = guides.len();
    if instances.is_empty()
        && faces.is_empty()
        && guides == 0
        && let Some(lines) = curve_info(m, &edges)
    {
        return lines;
    }
    if let ([], [], [i], 0) = (&edges[..], &faces[..], &instances[..], guides) {
        let instance = m.instance(*i);
        let d = instance.and_then(|i| doc.scene().definition(i.definition));
        let mut lines = match d {
            Some(d) if d.kind == DefinitionKind::Component => {
                vec!["1 component".into(), format!("Name: {}", d.name)]
            }
            _ => vec!["1 group".into()],
        };
        lines.extend(instance.and_then(|i| rotation(i.transform)));
        if let Some(material) = instance.and_then(|i| i.material) {
            lines.push(format!("Material: {}", material_name(doc, Some(material))));
        }
        return lines;
    }
    match (&edges[..], &faces[..]) {
        ([], []) if instances.is_empty() && guides == 0 => vec!["Nothing selected".into()],
        ([e], []) if instances.is_empty() && guides == 0 => vec![
            "1 edge".into(),
            format!("Length: {}", format_length(edge_length(m, *e))),
        ],
        ([], [f]) if instances.is_empty() && guides == 0 => {
            let face = m.face(*f).unwrap();
            let n = face.normal.to_array().map(number);
            vec![
                "1 face".into(),
                format!("Area: {} m²", number(m.area(*f).unwrap_or(0.0))),
                format!("Normal: {}, {}, {}", n[0], n[1], n[2]),
                format!("Front: {}", material_name(doc, face.front)),
                format!("Back: {}", material_name(doc, face.back)),
            ]
        }
        _ => {
            let mut parts = Vec::new();
            if !edges.is_empty() {
                parts.push(count(edges.len(), "edge"));
            }
            if !faces.is_empty() {
                parts.push(count(faces.len(), "face"));
            }
            if !instances.is_empty() {
                parts.push(count(instances.len(), "instance"));
            }
            if guides > 0 {
                parts.push(count(guides, "guide"));
            }
            vec![parts.join(", ")]
        }
    }
}

/// A material's name, or "Default" for none.
fn material_name(doc: &Document, material: Option<kernel::MaterialId>) -> String {
    material
        .and_then(|m| doc.scene().material(m))
        .map_or_else(|| "Default".to_string(), |m| m.name.clone())
}

/// "1 circle" (or arc, or curve) with its radius, angle, segments and length when `edges` are
/// exactly the edges of one curve.
fn curve_info(m: &Model, edges: &[EdgeId]) -> Option<Vec<String>> {
    let curve = m.curve_of(*edges.first()?)?;
    let mut mine = curve.edges.clone();
    mine.sort();
    if mine != edges {
        return None;
    }
    let mut lines = vec![match curve.kind() {
        CurveKind::Circle => "1 circle".to_string(),
        CurveKind::Arc => "1 arc".to_string(),
        CurveKind::Curve => "1 curve".to_string(),
    }];
    if let Some(c) = curve.circle {
        lines.push(format!("Radius: {}", format_length(c.radius)));
        if !curve.closed {
            lines.push(format!("Angle: {}°", number(c.angle.to_degrees())));
        }
    }
    lines.push(format!("Segments: {}", curve.segments));
    lines.push(format!("Length: {}", format_length(curve.length)));
    Some(lines)
}

/// "Rotation: 30° about 0, 0, 1" for a turned placement (the turn of at most 180 degrees,
/// counter-clockwise looking down the axis); None when it is not turned.
fn rotation(t: DAffine3) -> Option<String> {
    let (mut axis, angle) = DQuat::from_mat3(&t.matrix3).to_axis_angle();
    let mut degrees = angle.to_degrees();
    if degrees > 180.0 {
        degrees = 360.0 - degrees;
        axis = -axis;
    }
    if number(degrees) == "0" {
        return None;
    }
    let a = axis.to_array().map(number);
    Some(format!(
        "Rotation: {}° about {}, {}, {}",
        number(degrees),
        a[0],
        a[1],
        a[2]
    ))
}

fn edge_length(m: &Model, e: kernel::EdgeId) -> f64 {
    let v = m.edge(e).unwrap().v;
    match (m.position(v[0]), m.position(v[1])) {
        (Some(a), Some(b)) => a.distance(b),
        _ => 0.0,
    }
}

/// "1 edge", "2 edges".
pub(crate) fn count(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// Rounded to 3 decimals, without trailing zeros or a negative zero.
fn number(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;
    use session::{Command, SelectMode};

    fn doc_with_square() -> Document {
        let mut d = Document::new(DVec2::new(800.0, 600.0));
        d.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 3.0, 0.0],
            normal: None,
        })
        .unwrap();
        d
    }

    fn select(d: &mut Document, ids: &[&str]) {
        d.execute(Command::Select {
            ids: ids.iter().map(|s| s.parse().unwrap()).collect(),
            mode: SelectMode::Replace,
        })
        .unwrap();
    }

    #[test]
    fn describes_the_selection() {
        let mut d = doc_with_square();
        assert_eq!(entity_info(&d), ["Nothing selected"]);
        select(&mut d, &["f8"]);
        assert_eq!(
            entity_info(&d),
            [
                "1 face",
                "Area: 6 m²",
                "Normal: 0, 0, 1",
                "Front: Default",
                "Back: Default"
            ]
        );
        select(&mut d, &["e2"]);
        assert_eq!(entity_info(&d), ["1 edge", "Length: 2 m"]);
        select(&mut d, &["e2", "e4", "f8"]);
        assert_eq!(entity_info(&d), ["2 edges, 1 face"]);
    }

    #[test]
    fn names_the_materials_of_a_face_and_a_painted_group() {
        let mut d = doc_with_square();
        let added = d
            .execute(Command::AddMaterial {
                name: Some("Brick".into()),
                color: Some([180, 60, 40]),
                opacity: None,
                texture: None,
                emission: None,
            })
            .unwrap();
        let brick = added.changes.materials[0];
        d.execute(Command::Paint {
            ids: vec!["f8".parse().unwrap()],
            material: Some(brick),
            side: kernel::Side::Back,
        })
        .unwrap();
        select(&mut d, &["f8"]);
        let lines = entity_info(&d);
        assert_eq!(lines[3..], ["Front: Default", "Back: Brick"]);
        d.execute(Command::MakeGroup {
            ids: vec!["f8".parse().unwrap()],
        })
        .unwrap();
        let group = *d.selection().iter().next().unwrap();
        d.execute(Command::Paint {
            ids: vec![group],
            material: Some(brick),
            side: kernel::Side::Front,
        })
        .unwrap();
        assert_eq!(entity_info(&d), ["1 group", "Material: Brick"]);
    }

    #[test]
    fn describes_a_whole_curve() {
        let mut d = Document::new(DVec2::new(800.0, 600.0));
        d.execute(Command::AddCircle {
            center: [0.0, 0.0, 0.0],
            radius: 2.0,
            normal: None,
            segments: Some(6),
        })
        .unwrap();
        let edges: Vec<EntityId> = d.model().edges().map(|(e, _)| EntityId::Edge(e)).collect();
        let pick = |d: &mut Document, ids: Vec<EntityId>| {
            d.execute(Command::Select {
                ids,
                mode: SelectMode::Replace,
            })
            .unwrap();
        };
        pick(&mut d, edges.clone());
        // A hexagon in a circle of radius 2 has sides of 2.
        assert_eq!(
            entity_info(&d),
            ["1 circle", "Radius: 2 m", "Segments: 6", "Length: 12 m"]
        );
        pick(&mut d, edges[..2].to_vec());
        assert_eq!(entity_info(&d), ["2 edges"]);

        d.execute(Command::AddArc {
            center: [10.0, 0.0, 0.0],
            start: [11.0, 0.0, 0.0],
            angle: 90.0,
            normal: None,
            segments: Some(3),
        })
        .unwrap();
        let arc: Vec<EntityId> = d
            .model()
            .edges()
            .filter(|(_, e)| e.v.iter().any(|v| d.model().position(*v).unwrap().x > 5.0))
            .map(|(e, _)| EntityId::Edge(e))
            .collect();
        pick(&mut d, arc);
        let lines = entity_info(&d);
        assert_eq!(
            lines[..4],
            ["1 arc", "Radius: 1 m", "Angle: 90°", "Segments: 3"]
        );
        assert!(lines[4].starts_with("Length: 1.55"), "{lines:?}");
    }

    #[test]
    fn describes_guides() {
        let mut d = doc_with_square();
        d.execute(Command::AddGuideLine {
            point: [0.0, -1.0, 0.0],
            direction: [0.0, 2.0, 0.0],
        })
        .unwrap();
        d.execute(Command::AddGuidePoint {
            position: [1.5, 2.0, 0.25],
        })
        .unwrap();
        select(&mut d, &["g9"]);
        assert_eq!(
            entity_info(&d),
            [
                "1 guide line",
                "Direction: 0, 1, 0",
                "Through: 0 m, -1 m, 0 m"
            ]
        );
        select(&mut d, &["g10"]);
        assert_eq!(
            entity_info(&d),
            ["1 guide point", "Position: 1.5 m, 2 m, 0.25 m"]
        );
        select(&mut d, &["g9", "g10", "e2"]);
        assert_eq!(entity_info(&d), ["1 edge, 2 guides"]);
    }

    #[test]
    fn describes_groups_and_components() {
        let mut d = doc_with_square();
        let ids = ["e2", "e4", "e6", "e7", "f8"]
            .map(|s| s.parse().unwrap())
            .to_vec();
        d.execute(Command::MakeGroup { ids }).unwrap();
        assert_eq!(entity_info(&d), ["1 group"]);
        let group = *d.selection().first().unwrap();
        d.execute(Command::MakeComponent {
            ids: vec![group],
            name: "Table".into(),
        })
        .unwrap();
        assert_eq!(entity_info(&d), ["1 component", "Name: Table"]);
        d.execute(Command::AddEdge {
            start: [5.0, 0.0, 0.0],
            end: [6.0, 0.0, 0.0],
        })
        .unwrap();
        let edge = d
            .model()
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .next()
            .unwrap();
        let component = *d.selection().first().unwrap();
        d.execute(Command::Select {
            ids: vec![edge, component],
            mode: SelectMode::Replace,
        })
        .unwrap();
        assert_eq!(entity_info(&d), ["1 edge, 1 instance"]);
    }

    #[test]
    fn a_turned_instance_shows_its_rotation() {
        let mut d = doc_with_square();
        let ids = ["e2", "e4", "e6", "e7", "f8"]
            .map(|s| s.parse().unwrap())
            .to_vec();
        d.execute(Command::MakeGroup { ids }).unwrap();
        let group = *d.selection().first().unwrap();
        let turn = |d: &mut Document, angle: f64| {
            d.execute(Command::Rotate {
                ids: vec![group],
                center: [1.0, 1.0, 0.0],
                axis: None,
                angle,
                copies: None,
            })
            .unwrap();
        };
        turn(&mut d, 90.0);
        assert_eq!(entity_info(&d), ["1 group", "Rotation: 90° about 0, 0, 1"]);
        turn(&mut d, 150.0);
        assert_eq!(
            entity_info(&d),
            ["1 group", "Rotation: 120° about 0, 0, -1"]
        );
        turn(&mut d, 120.0);
        assert_eq!(entity_info(&d), ["1 group"]);
    }

    #[test]
    fn says_what_is_being_edited() {
        let mut d = doc_with_square();
        let ids = ["e2", "e4", "e6", "e7", "f8"]
            .map(|s| s.parse().unwrap())
            .to_vec();
        d.execute(Command::MakeComponent {
            ids,
            name: "Table".into(),
        })
        .unwrap();
        let table = *d.selection().first().unwrap();
        d.execute(Command::OpenInstance { id: table }).unwrap();
        assert_eq!(
            entity_info(&d),
            ["Editing component Table", "Nothing selected"]
        );
        let ids = d.model().faces().map(|(f, _)| EntityId::Face(f)).collect();
        d.execute(Command::MakeGroup { ids }).unwrap();
        let group = *d.selection().first().unwrap();
        d.execute(Command::OpenInstance { id: group }).unwrap();
        assert_eq!(entity_info(&d), ["Editing group", "Nothing selected"]);
    }
}
