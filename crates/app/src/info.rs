//! Text for the Entity Info panel: what is selected and its measurements.

use kernel::{EntityId, Model};
use session::Document;
use tools::vcb::format_length;

/// Lines describing the selection: counts, plus length, area or normal for a single entity.
pub fn entity_info(doc: &Document) -> Vec<String> {
    let m = doc.model();
    let (mut edges, mut faces) = (Vec::new(), Vec::new());
    for id in doc.selection() {
        match id {
            EntityId::Edge(e) => edges.push(*e),
            EntityId::Face(f) => faces.push(*f),
            EntityId::Vertex(_) | EntityId::Instance(_) => {}
        }
    }
    match (&edges[..], &faces[..]) {
        ([], []) => vec!["Nothing selected".into()],
        ([e], []) => vec![
            "1 edge".into(),
            format!("Length: {}", format_length(edge_length(m, *e))),
        ],
        ([], [f]) => {
            let face = m.face(*f).unwrap();
            let n = face.normal.to_array().map(number);
            vec![
                "1 face".into(),
                format!("Area: {} m²", number(m.area(*f).unwrap_or(0.0))),
                format!("Normal: {}, {}, {}", n[0], n[1], n[2]),
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
            vec![parts.join(", ")]
        }
    }
}

fn edge_length(m: &Model, e: kernel::EdgeId) -> f64 {
    let v = m.edge(e).unwrap().v;
    match (m.position(v[0]), m.position(v[1])) {
        (Some(a), Some(b)) => a.distance(b),
        _ => 0.0,
    }
}

fn count(n: usize, word: &str) -> String {
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
        assert_eq!(entity_info(&d), ["1 face", "Area: 6 m²", "Normal: 0, 0, 1"]);
        select(&mut d, &["e2"]);
        assert_eq!(entity_info(&d), ["1 edge", "Length: 2 m"]);
        select(&mut d, &["e2", "e4", "f8"]);
        assert_eq!(entity_info(&d), ["2 edges, 1 face"]);
    }
}
