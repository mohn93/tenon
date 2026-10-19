//! Regular polygons: the corner maths and the `add_polygon` command.

use glam::{DVec2, DVec3};
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query, polygon_points};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command =
        serde_json::from_value(v).map_err(|e| (ErrorKind::InvalidInput, e.to_string()))?;
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn close(a: DVec3, b: [f64; 3]) -> bool {
    (a - DVec3::from(b)).length() < 1e-12
}

#[test]
fn polygon_points_inscribed_and_circumscribed() {
    let p = polygon_points(DVec3::ZERO, 1.0, 4, None, false, None).unwrap();
    let want = [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
    ];
    assert_eq!(p.len(), 4);
    for (a, b) in p.iter().zip(want) {
        assert!(close(*a, b), "{a:?} vs {b:?}");
    }

    let c = polygon_points(DVec3::ZERO, 1.0, 4, None, true, None).unwrap();
    assert!(close(c[0], [1.0, 1.0, 0.0]), "{:?}", c[0]);

    let d = polygon_points(
        DVec3::ZERO,
        1.0,
        4,
        None,
        false,
        Some(DVec3::new(0.0, 2.0, 0.0)),
    )
    .unwrap();
    assert!(close(d[0], [0.0, 1.0, 0.0]), "{:?}", d[0]);
}

#[test]
fn polygon_points_refuses_bad_input() {
    for sides in [2, 361] {
        let e = polygon_points(DVec3::ZERO, 1.0, sides, None, false, None).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }
    let e = polygon_points(
        DVec3::ZERO,
        1.0,
        6,
        None,
        false,
        Some(DVec3::new(0.0, 0.0, 5.0)),
    )
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    let e = polygon_points(DVec3::ZERO, 1e-7, 6, None, false, None).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
}

#[test]
fn add_polygon_makes_one_face_and_one_polygon_curve() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_polygon", "center": [0, 0, 0], "radius": 1.0}),
    )
    .unwrap();
    let created = out["changes"]["created"].as_array().unwrap();
    let with = |p: char| {
        created
            .iter()
            .filter(|id| id.as_str().unwrap().starts_with(p))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(with('f').len(), 1);
    let edges = with('e');
    assert_eq!(edges.len(), 6);
    let c = ask(&d, json!({"type": "curve", "id": edges[0]}));
    assert_eq!(c["kind"], "polygon");
    assert_eq!(c["segments"], 6);
    let h = ask(&d, json!({"type": "history"}));
    assert_eq!(h["undo_depth"], 1);
    assert!(h.to_string().contains("add_polygon"), "{h}");
}

#[test]
fn add_polygon_over_json() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_polygon", "center": [0, 0, 0], "radius": 2, "sides": 8, "circumscribed": true}),
    )
    .unwrap();
    let edges = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with('e'))
        .count();
    assert_eq!(edges, 8);
    let (kind, _) = run(
        &mut d,
        json!({"type": "add_polygon", "center": [0, 0, 0], "radius": 2, "bogus": 1}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}
