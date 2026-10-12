//! Curves (circles and arcs as tagged edge chains), driven through JSON commands.

use std::io::Write;
use std::path::PathBuf;

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

fn ask(d: &Document, q: Value) -> Result<Value, (ErrorKind, String)> {
    let q: Query = serde_json::from_value(q).unwrap();
    d.query(&q)
        .map(|r| serde_json::to_value(r).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn edges(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities", "kinds": ["edge"]})).unwrap()["edges"]
        .as_array()
        .unwrap()
        .clone()
}

fn curve(d: &Document, edge: &Value) -> Value {
    ask(d, json!({"type": "curve", "id": edge})).unwrap()
}

fn near(a: &Value, b: [f64; 3]) -> bool {
    (0..3).all(|i| (a[i].as_f64().unwrap() - b[i]).abs() < 1e-9)
}

/// A fresh path in the test scratch directory.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("session-curves");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn edit_file(path: &PathBuf, f: impl FnOnce(&mut Value)) {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut document: Value =
        serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    f(&mut document);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    zip.finish().unwrap();
}

#[test]
fn a_circle_is_one_curve_the_curve_query_describes() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [1, 2, 0], "radius": 2.0, "segments": 12}),
    )
    .unwrap();
    let all = edges(&d);
    assert_eq!(all.len(), 12);
    let id = all[0]["curve"].clone();
    assert!(id.as_str().unwrap().starts_with('c'), "{id}");
    assert!(
        all.iter()
            .all(|e| e["curve"] == id && e.get("soft").is_none())
    );

    let c = curve(&d, &all[5]["id"]);
    assert_eq!(c["type"], "curve");
    assert_eq!(c["id"], id);
    assert_eq!(c["kind"], "circle");
    assert_eq!(c["segments"], 12);
    assert_eq!(c["edges"].as_array().unwrap().len(), 12);
    assert_eq!(c["vertices"].as_array().unwrap().len(), 12);
    assert!(near(&c["center"], [1.0, 2.0, 0.0]), "{c}");
    assert_eq!(c["normal"], json!([0.0, 0.0, 1.0]));
    assert!((c["radius"].as_f64().unwrap() - 2.0).abs() < 1e-9);
    assert_eq!(c["angle"], json!(360.0));
    let side = 2.0 * 2.0 * (std::f64::consts::PI / 12.0).sin();
    assert!((c["length"].as_f64().unwrap() - 12.0 * side).abs() < 1e-9);
    // The first vertex is the first corner drawn, along +X from the center.
    let v0 = ask(&d, json!({"type": "entity", "id": c["vertices"][0]})).unwrap();
    assert!(near(&v0["position"], [3.0, 2.0, 0.0]));

    // Undo takes the curve away with the circle; redo brings it back under the same id.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(edges(&d).is_empty());
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(curve(&d, &all[0]["id"]), c);
}

#[test]
fn erasing_a_circle_edge_leaves_an_arc() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "segments": 24, "normal": [0, 0, -1]}),
    )
    .unwrap();
    let e = edges(&d)[0]["id"].clone();
    let c = curve(&d, &e);
    assert_eq!(c["normal"], json!([0.0, 0.0, 1.0]), "normals point up");
    run(&mut d, json!({"type": "erase", "ids": [e]})).unwrap();
    let rest = edges(&d);
    let arc = curve(&d, &rest[0]["id"]);
    assert_eq!(arc["kind"], "arc");
    assert_eq!(arc["id"], c["id"]);
    assert_eq!(arc["segments"], 23);
    assert!((arc["angle"].as_f64().unwrap() - 345.0).abs() < 1e-9);
}

#[test]
fn the_curve_query_needs_an_edge_on_a_curve() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let e = edges(&d)[0]["id"].clone();
    assert!(edges(&d)[0].get("curve").is_none());
    let (kind, message) = ask(&d, json!({"type": "curve", "id": e})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("not part of a curve"), "{message}");
    let face =
        ask(&d, json!({"type": "entities", "kinds": ["face"]})).unwrap()["faces"][0]["id"].clone();
    assert_eq!(
        ask(&d, json!({"type": "curve", "id": face})).unwrap_err().0,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask(&d, json!({"type": "curve", "id": "e999"}))
            .unwrap_err()
            .0,
        ErrorKind::NotFound
    );
}

#[test]
fn copies_and_groups_get_curves_of_their_own() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "segments": 8}),
    )
    .unwrap();
    let ids: Vec<Value> = edges(&d).iter().map(|e| e["id"].clone()).collect();
    run(
        &mut d,
        json!({"type": "copy", "ids": ids, "offset": [3, 0, 0]}),
    )
    .unwrap();
    let curves: std::collections::BTreeSet<String> = edges(&d)
        .iter()
        .map(|e| e["curve"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(curves.len(), 2);
    let copied = edges(&d)
        .into_iter()
        .find(|e| e["start"][0].as_f64().unwrap() > 1.5)
        .unwrap();
    assert!(near(&curve(&d, &copied["id"])["center"], [3.0, 0.0, 0.0]));

    run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = ask(&d, json!({"type": "selection"})).unwrap()["ids"][0].clone();
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let inside = edges(&d);
    assert_eq!(inside.len(), 8);
    assert_eq!(curve(&d, &inside[0]["id"])["kind"], "circle");
}

#[test]
fn curves_are_saved_and_older_files_still_open() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "segments": 6}),
    )
    .unwrap();
    let path = scratch("circle.skr");
    let path_text = path.to_str().unwrap();
    run(&mut d, json!({"type": "save", "path": path_text})).unwrap();
    let mut e = doc();
    run(&mut e, json!({"type": "open", "path": path_text})).unwrap();
    assert_eq!(edges(&e), edges(&d));
    let first = edges(&e)[0]["id"].clone();
    assert_eq!(curve(&e, &first), curve(&d, &first));

    // A version 2 file (before curves) opens, its edges plain.
    edit_file(&path, |doc| {
        assert_eq!(doc["schema_version"], 3);
        doc["schema_version"] = json!(2);
        for edge in doc["model"]["root"]["edges"].as_array_mut().unwrap() {
            edge.as_object_mut().unwrap().remove("curve");
        }
    });
    let mut old = doc();
    run(&mut old, json!({"type": "open", "path": path_text})).unwrap();
    assert!(edges(&old).iter().all(|e| e.get("curve").is_none()));

    // A soft edge that bounds no two faces does not.
    edit_file(&path, |doc| {
        doc["model"]["root"]["edges"][0]["soft"] = json!(true);
    });
    let mut bad = doc();
    let (kind, message) = run(&mut bad, json!({"type": "open", "path": path_text})).unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("soft edge"), "{message}");
}
