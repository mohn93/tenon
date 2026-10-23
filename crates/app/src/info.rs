//! Text for the Entity Info panel: what is selected and its measurements.

use glam::{DAffine3, DMat3, DQuat};
use kernel::{CurveKind, DefinitionKind, Dimension, EdgeId, EntityId, Guide, Model, Note};
use session::{AnnotationShape, Document, Units, note_label};

/// Entity Info still shows meters (it follows the model units from the next step on).
fn format_length(meters: f64) -> String {
    session::format_length(meters, &Units::default())
}

/// Lines describing the selection: counts, plus length, area or normal for a single entity,
/// the radius, segments, angle and length of a whole curve (a circle or arc), the direction
/// or position of a single guide, the kind and text of a single dimension or note, or the
/// kind, name and rotation of a single instance; a single face's front and back materials and
/// a painted instance's material. While a group or component is open, a first line says
/// which.
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
    let mut sections = Vec::new();
    let (mut dimensions, mut notes) = (0, 0);
    let mut annotation = None;
    for id in doc.selection() {
        match id {
            EntityId::Edge(e) => edges.push(*e),
            EntityId::Face(f) => faces.push(*f),
            EntityId::Instance(i) => instances.push(*i),
            EntityId::Guide(g) => guides.push(*g),
            EntityId::Dimension(_) => {
                dimensions += 1;
                annotation = Some(*id);
            }
            EntityId::Note(_) => {
                notes += 1;
                annotation = Some(*id);
            }
            EntityId::Section(s) => sections.push(*s),
            EntityId::Vertex(_) => {}
        }
    }
    let annotations = dimensions + notes;
    if let ([], [], [], [], [s], 0) = (
        &edges[..],
        &faces[..],
        &instances[..],
        &guides[..],
        &sections[..],
        annotations,
    ) {
        return section_info(doc, *s);
    }
    if annotations == 1
        && edges.is_empty()
        && faces.is_empty()
        && instances.is_empty()
        && guides.is_empty()
        && sections.is_empty()
        && let Some(id) = annotation
    {
        return annotation_info(doc, id);
    }
    if let ([], [], [], [g], [], 0) = (
        &edges[..],
        &faces[..],
        &instances[..],
        &guides[..],
        &sections[..],
        annotations,
    ) {
        let xyz = |v: glam::DVec3| v.to_array().map(number).join(", ");
        let place = |v: glam::DVec3| v.to_array().map(format_length).join(", ");
        return match m.guide(*g) {
            Some(Guide::Line {
                point, direction, ..
            }) => {
                vec![
                    "1 guide line".into(),
                    format!("Direction: {}", xyz(*direction)),
                    format!("Through: {}", place(*point)),
                ]
            }
            Some(Guide::Point { position, .. }) => {
                vec![
                    "1 guide point".into(),
                    format!("Position: {}", place(*position)),
                ]
            }
            None => Vec::new(),
        };
    }
    // Guides, section planes and annotations: none is described with other entities.
    let guides = guides.len();
    let planes = sections.len();
    let others = guides + planes + annotations;
    if instances.is_empty()
        && faces.is_empty()
        && others == 0
        && let Some(lines) = curve_info(m, &edges)
    {
        return lines;
    }
    if let ([], [], [i], 0) = (&edges[..], &faces[..], &instances[..], others) {
        let instance = m.instance(*i);
        let d = instance.and_then(|i| doc.scene().definition(i.definition));
        let mut lines = match (d, instance) {
            (Some(d), Some(i)) if d.kind == DefinitionKind::Component => {
                // The name is Entity Info's Name field.
                vec![
                    "1 component".into(),
                    count(doc.scene().instance_count(i.definition), "instance"),
                ]
            }
            _ => vec!["1 group".into()],
        };
        lines.extend(instance.map_or_else(Vec::new, |i| placement(i.transform)));
        if let Some(material) = instance.and_then(|i| i.material) {
            lines.push(format!("Material: {}", material_name(doc, Some(material))));
        }
        return lines;
    }
    match (&edges[..], &faces[..]) {
        ([], []) if instances.is_empty() && others == 0 => vec!["Nothing selected".into()],
        ([e], []) if instances.is_empty() && others == 0 => vec![
            "1 edge".into(),
            format!("Length: {}", format_length(edge_length(m, *e))),
        ],
        ([], [f]) if instances.is_empty() && others == 0 => {
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
            if planes > 0 {
                parts.push(count(planes, "section plane"));
            }
            if dimensions > 0 {
                parts.push(count(dimensions, "dimension"));
            }
            if notes > 0 {
                parts.push(count(notes, "note"));
            }
            vec![parts.join(", ")]
        }
    }
}

/// A single section plane: "Section Plane", whether it is the open context's active plane,
/// its point and its unit normal (the side it points to is kept).
fn section_info(doc: &Document, id: kernel::SectionId) -> Vec<String> {
    let m = doc.model();
    let Some(plane) = m.section(id) else {
        return Vec::new();
    };
    let place = plane.point.to_array().map(format_length).join(", ");
    let n = plane.normal.to_array().map(number);
    vec![
        "Section Plane".into(),
        if m.active_section() == Some(id) {
            "Active".into()
        } else {
            "Not active".into()
        },
        format!("Point: {place}"),
        format!("Normal: {}, {}, {}", n[0], n[1], n[2]),
    ]
}

/// The kind and label text of the dimension or note `id` of the open context: "1 linear
/// dimension", "1 radial dimension" or "1 diameter dimension", or "1 leader note" or "1
/// screen note", then its text as the view writes it (in the display units; a dimension
/// measured as placed, so in a scaled open group it shows the scaled length). A note's text is
/// given by its first line, with an ellipsis when it has more.
fn annotation_info(doc: &Document, id: EntityId) -> Vec<String> {
    let m = doc.model();
    let units = doc.units();
    let (kind, text) = match id {
        EntityId::Dimension(d) => {
            let kind = match m.dimension(d) {
                Some(Dimension::Linear { .. }) => "1 linear dimension",
                Some(Dimension::Radial {
                    diameter: false, ..
                }) => "1 radial dimension",
                Some(Dimension::Radial { diameter: true, .. }) => "1 diameter dimension",
                None => return Vec::new(),
            };
            // As its label is drawn: measured placed in root coordinates (scaled with the
            // open group).
            let placed = AnnotationShape::of(m, id).map(|s| s.transformed(doc.context_transform()));
            (kind, placed.and_then(|s| s.dimension_label(&units)))
        }
        EntityId::Note(n) => {
            let kind = match m.note(n) {
                Some(Note::Leader { .. }) => "1 leader note",
                Some(Note::Screen { .. }) => "1 screen note",
                None => return Vec::new(),
            };
            (kind, note_label(m, n, &units, doc.context_transform()))
        }
        EntityId::Vertex(_)
        | EntityId::Edge(_)
        | EntityId::Face(_)
        | EntityId::Instance(_)
        | EntityId::Guide(_)
        | EntityId::Section(_) => return Vec::new(),
    };
    let mut lines = vec![kind.to_string()];
    if let Some(text) = text {
        let mut rows = text.lines();
        let first = rows.next().unwrap_or_default();
        let more = if rows.next().is_some() { "…" } else { "" };
        lines.push(format!("Text: {first}{more}"));
    }
    lines
}

/// A material's name, or "Default" for none.
fn material_name(doc: &Document, material: Option<kernel::MaterialId>) -> String {
    material
        .and_then(|m| doc.scene().material(m))
        .map_or_else(|| "Default".to_string(), |m| m.name.clone())
}

/// "1 circle" (or arc, curve or polygon) with its radius, angle, segments (a polygon's sides)
/// and length when `edges` are exactly the edges of one curve.
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
        CurveKind::Polygon => "1 polygon".to_string(),
    }];
    if let Some(c) = curve.circle {
        lines.push(format!("Radius: {}", format_length(c.radius)));
        if !curve.closed {
            lines.push(format!("Angle: {}°", number(c.angle.to_degrees())));
        }
    }
    if curve.kind() == CurveKind::Polygon {
        lines.push(format!("Sides: {}", curve.segments));
    } else {
        lines.push(format!("Segments: {}", curve.segments));
    }
    lines.push(format!("Length: {}", format_length(curve.length)));
    Some(lines)
}

/// The rotation line of a placement and, when it is not a pure rotation, a line with its
/// scale along its own axes ("Scale: 2, 1, 1"; a mirror shows a negative x).
fn placement(t: DAffine3) -> Vec<String> {
    let m = t.matrix3;
    let lengths = [m.x_axis.length(), m.y_axis.length(), m.z_axis.length()];
    let mirror = m.determinant() < 0.0;
    if !mirror && lengths.iter().all(|l| (l - 1.0).abs() < 1e-9) {
        return rotation(m).into_iter().collect();
    }
    let sign = if mirror { -1.0 } else { 1.0 };
    let scale = [sign * lengths[0], lengths[1], lengths[2]];
    let turn = DMat3::from_cols(
        m.x_axis / scale[0],
        m.y_axis / scale[1],
        m.z_axis / scale[2],
    );
    let mut lines: Vec<String> = rotation(turn).into_iter().collect();
    let s = scale.map(number);
    lines.push(format!("Scale: {}, {}, {}", s[0], s[1], s[2]));
    lines
}

/// "Rotation: 30° about 0, 0, 1" for a turned placement (the turn of at most 180 degrees,
/// counter-clockwise looking down the axis); None when it is not turned.
fn rotation(m: DMat3) -> Option<String> {
    let (mut axis, angle) = DQuat::from_mat3(&m).to_axis_angle();
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
            all: false,
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
                all: false,
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
            center: Some([10.0, 0.0, 0.0]),
            start: [11.0, 0.0, 0.0],
            angle: Some(90.0),
            end: None,
            through: None,
            bulge: None,
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
    fn describes_a_polygon_by_its_sides() {
        let mut m = Model::new();
        let corners: Vec<glam::DVec3> = (0..6)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / 6.0;
                glam::DVec3::new(2.0 * a.cos(), 2.0 * a.sin(), 0.0)
            })
            .collect();
        m.add_curve(&corners, true, true).unwrap();
        let mut edges: Vec<EdgeId> = m.edges().map(|(e, _)| e).collect();
        edges.sort();
        assert_eq!(
            curve_info(&m, &edges).unwrap(),
            ["1 polygon", "Radius: 2 m", "Sides: 6", "Length: 12 m"]
        );
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
        // The name is Entity Info's Name field, not a line.
        assert_eq!(entity_info(&d), ["1 component", "1 instance"]);
        let out = d
            .execute(Command::Copy {
                ids: vec![*d.selection().first().unwrap()],
                offset: [0.0, 5.0, 0.0],
                count: None,
            })
            .unwrap();
        let copy = *out.changes.created.first().unwrap();
        d.execute(Command::Select {
            ids: vec![copy],
            mode: SelectMode::Replace,
            all: false,
        })
        .unwrap();
        assert_eq!(
            entity_info(&d),
            ["1 component", "2 instances"],
            "the copies share the definition"
        );
        d.execute(Command::MakeUnique { ids: None }).unwrap();
        assert_eq!(entity_info(&d)[1], "1 instance", "made unique");
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
            all: false,
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
    fn entity_info_shows_scale_and_true_rotation() {
        let group = |d: &mut Document| {
            let ids = d
                .model()
                .faces()
                .map(|(f, _)| EntityId::Face(f))
                .chain(d.model().edges().map(|(e, _)| EntityId::Edge(e)))
                .collect();
            d.execute(Command::MakeGroup { ids }).unwrap();
            *d.selection().first().unwrap()
        };
        let scale = |d: &mut Document, id: EntityId, factors: [f64; 3]| {
            d.execute(Command::Scale {
                ids: vec![id],
                origin: [0.0; 3],
                factors,
            })
            .unwrap();
        };
        // Stretched along its own x, then turned a quarter about Z.
        let mut d = doc_with_square();
        let g = group(&mut d);
        scale(&mut d, g, [2.0, 1.0, 1.0]);
        assert_eq!(entity_info(&d), ["1 group", "Scale: 2, 1, 1"]);
        d.execute(Command::Rotate {
            ids: vec![g],
            center: [0.0; 3],
            axis: None,
            angle: 90.0,
            copies: None,
        })
        .unwrap();
        assert_eq!(
            entity_info(&d),
            ["1 group", "Rotation: 90° about 0, 0, 1", "Scale: 2, 1, 1"]
        );
        // Mirrored.
        let mut d = doc_with_square();
        let g = group(&mut d);
        scale(&mut d, g, [-1.0, 1.0, 1.0]);
        assert_eq!(entity_info(&d), ["1 group", "Scale: -1, 1, 1"]);
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

    /// Runs `command` and returns the one id it created.
    fn create(d: &mut Document, command: Command) -> EntityId {
        let out = d.execute(command).unwrap();
        let [id] = out.changes.created[..] else {
            panic!("{:?}", out.changes.created);
        };
        id
    }

    #[test]
    fn describes_a_dimension_and_a_note_with_their_text() {
        let mut d = doc_with_square();
        let point = |p: [f64; 3]| session::AnchorInput::Point(p);
        let dimension = create(
            &mut d,
            Command::AddDimension {
                a: point([0.0, 0.0, 0.0]),
                b: point([2.0, 0.0, 0.0]),
                offset: [0.0, -1.0, 0.0],
            },
        );
        let leader = create(
            &mut d,
            Command::AddNote {
                anchor: point([2.0, 3.0, 0.0]),
                position: [3.0, 4.0, 1.0],
                text: None,
            },
        );
        let screen = create(
            &mut d,
            Command::AddScreenNote {
                at: [0.1, 0.1],
                text: "Ground floor\nScale 1:50".into(),
            },
        );
        let show = |d: &mut Document, id: EntityId| {
            select(d, &[&id.to_string()]);
            entity_info(d)
        };
        assert_eq!(show(&mut d, dimension), ["1 linear dimension", "Text: 2 m"]);
        assert_eq!(
            show(&mut d, leader),
            ["1 leader note", "Text: (2 m, 3 m, 0 m)"]
        );
        assert_eq!(
            show(&mut d, screen),
            ["1 screen note", "Text: Ground floor…"]
        );
        // The text follows the display units.
        d.execute(Command::SetUnits {
            length: Some(session::LengthUnit::Centimeters),
            precision: None,
            length_snap: None,
            angle_precision: None,
            angle_snap: None,
        })
        .unwrap();
        assert_eq!(
            show(&mut d, dimension),
            ["1 linear dimension", "Text: 200 cm"]
        );
        // With other entities, counted.
        select(
            &mut d,
            &[
                &dimension.to_string(),
                &leader.to_string(),
                &screen.to_string(),
                "f8",
            ],
        );
        assert_eq!(entity_info(&d), ["1 face, 1 dimension, 2 notes"]);
    }

    #[test]
    fn describes_a_section_plane_active_or_not() {
        let mut d = doc_with_square();
        let first = create(
            &mut d,
            Command::AddSectionPlane {
                point: [1.0, 0.0, 0.5],
                normal: [0.0, 0.0, 2.0],
                activate: Some(true),
            },
        );
        let second = create(
            &mut d,
            Command::AddSectionPlane {
                point: [0.0, 0.0, 0.0],
                normal: [1.0, 0.0, 0.0],
                activate: Some(false),
            },
        );
        let show = |d: &mut Document, ids: &[EntityId]| {
            let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
            select(d, &ids.iter().map(String::as_str).collect::<Vec<_>>());
            entity_info(d)
        };
        assert_eq!(
            show(&mut d, &[first]),
            [
                "Section Plane",
                "Active",
                "Point: 1 m, 0 m, 0.5 m",
                "Normal: 0, 0, 1"
            ]
        );
        assert_eq!(
            show(&mut d, &[second]),
            [
                "Section Plane",
                "Not active",
                "Point: 0 m, 0 m, 0 m",
                "Normal: 1, 0, 0"
            ]
        );
        assert_eq!(show(&mut d, &[first, second]), ["2 section planes"]);
    }
}
