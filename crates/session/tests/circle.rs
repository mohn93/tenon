//! Circles, driven through JSON commands.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn counts(d: &Document) -> [u64; 3] {
    let s = ask(d, json!({"type": "stats"}));
    ["vertices", "edges", "faces"].map(|k| s[k].as_u64().unwrap())
}

#[test]
fn a_circle_is_a_24_sided_face_in_one_undo_step() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [1, 2, 0], "radius": 2.0}),
    )
    .unwrap();
    assert_eq!(counts(&d), [24, 24, 1]);
    let face = &ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0];
    assert_eq!(face["normal"], json!([0.0, 0.0, 1.0]));
    let area = face["area"].as_f64().unwrap();
    let n = 24.0_f64;
    let expected = 0.5 * n * 4.0 * (std::f64::consts::TAU / n).sin();
    assert!((area - expected).abs() < 1e-9, "{area} vs {expected}");
    let vertices = ask(&d, json!({"type": "entities", "kinds": ["vertex"]}));
    for v in vertices["vertices"].as_array().unwrap() {
        let p = &v["position"];
        let (x, y) = (p[0].as_f64().unwrap() - 1.0, p[1].as_f64().unwrap() - 2.0);
        assert!(((x * x + y * y).sqrt() - 2.0).abs() < 1e-9);
    }
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), [0, 0, 0]);
}

#[test]
fn segments_and_plane_can_be_chosen() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 3, 1], "radius": 1.0, "normal": [0, -1, 0], "segments": 6}),
    )
    .unwrap();
    assert_eq!(counts(&d), [6, 6, 1]);
    let vertices = ask(&d, json!({"type": "entities", "kinds": ["vertex"]}));
    for v in vertices["vertices"].as_array().unwrap() {
        assert!((v["position"][1].as_f64().unwrap() - 3.0).abs() < 1e-12);
    }
}

#[test]
fn a_pulled_circle_makes_a_cylinder() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "segments": 12}),
    )
    .unwrap();
    let face = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 2.0}),
    )
    .unwrap();
    assert_eq!(counts(&d), [24, 36, 14]);
}

#[test]
fn bad_circles_are_refused() {
    let mut d = doc();
    for bad in [
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "segments": 2}),
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "segments": 361}),
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 0.0}),
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": -1.0}),
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1e-6, "segments": 360}),
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "normal": [0, 0, 0]}),
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "normal": [1e155, 1e155, 0]}),
        json!({"type": "add_circle", "center": [2e6, 0, 0], "radius": 1.0}),
        json!({"type": "add_circle", "center": [999999, 0, 0], "radius": 10.0}),
    ] {
        let (kind, _) = run(&mut d, bad.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{bad}");
    }
    assert_eq!(counts(&d), [0, 0, 0]);
}
