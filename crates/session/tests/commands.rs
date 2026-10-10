//! Drives the document only through serialized commands and queries, the way the API will.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query, QueryResult};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

/// Parses a JSON command, runs it, and returns the outcome as JSON.
fn run(d: &mut Document, cmd: Value) -> Result<Value, ErrorKind> {
    let cmd: Command = serde_json::from_value(cmd).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| e.kind)
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).expect("valid query json");
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn edge(d: &mut Document, a: [f64; 3], b: [f64; 3]) -> Value {
    run(d, json!({"type": "add_edge", "start": a, "end": b})).unwrap()
}

fn draw_square(d: &mut Document) {
    edge(d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
    edge(d, [2.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    edge(d, [2.0, 2.0, 0.0], [0.0, 2.0, 0.0]);
    edge(d, [0.0, 2.0, 0.0], [0.0, 0.0, 0.0]);
}

#[test]
fn add_edge_reports_created_ids_as_strings() {
    let mut d = doc();
    let out = edge(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    assert_eq!(out["changes"]["created"], json!(["v0", "v1", "e2"]));
}

#[test]
fn square_via_commands_makes_a_queryable_face() {
    let mut d = doc();
    draw_square(&mut d);
    let stats = ask(&d, json!({"type": "stats"}));
    assert_eq!(
        stats,
        json!({"type": "stats", "vertices": 4, "edges": 4, "faces": 1})
    );
    let all = ask(&d, json!({"type": "entities"}));
    let face = &all["faces"][0];
    assert_eq!(face["area"], json!(4.0));
    assert_eq!(face["normal"], json!([0.0, 0.0, 1.0]));
    let one = ask(&d, json!({"type": "entity", "id": face["id"]}));
    assert_eq!(one["type"], "face");
}

#[test]
fn undo_and_redo_walk_history() {
    let mut d = doc();
    draw_square(&mut d);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 0);
    assert_eq!(
        ask(&d, json!({"type": "history"})),
        json!({"type": "history", "undo_depth": 3, "redo_depth": 1})
    );
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 1);
    for _ in 0..4 {
        run(&mut d, json!({"type": "undo"})).unwrap();
    }
    assert_eq!(
        run(&mut d, json!({"type": "undo"})),
        Err(ErrorKind::NothingToUndo)
    );
    assert_eq!(ask(&d, json!({"type": "stats"}))["edges"], 0);
}

#[test]
fn new_edit_clears_redo() {
    let mut d = doc();
    edge(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    run(&mut d, json!({"type": "undo"})).unwrap();
    edge(&mut d, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    assert_eq!(
        run(&mut d, json!({"type": "redo"})),
        Err(ErrorKind::NothingToRedo)
    );
}

#[test]
fn duplicate_edge_adds_no_undo_step() {
    let mut d = doc();
    edge(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    edge(&mut d, [1.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
}

#[test]
fn invalid_input_is_a_structured_error() {
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [0, 0, 0]}),
    );
    assert_eq!(r, Err(ErrorKind::InvalidInput));
    let q: Query = serde_json::from_value(json!({"type": "entity", "id": "f99"})).unwrap();
    assert_eq!(d.query(&q).unwrap_err().kind, ErrorKind::NotFound);
    assert!(serde_json::from_value::<Query>(json!({"type": "entity", "id": "zz"})).is_err());
}

#[test]
fn set_camera_moves_camera_without_history() {
    let mut d = doc();
    run(&mut d, json!({"type": "set_camera", "eye": [10.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0], "fov_y_deg": 50.0})).unwrap();
    let cam = ask(&d, json!({"type": "camera"}));
    assert_eq!(cam["eye"], json!([10.0, 0.0, 5.0]));
    assert!((cam["fov_y_deg"].as_f64().unwrap() - 50.0).abs() < 1e-9);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 0);
    let vertical = run(
        &mut d,
        json!({"type": "set_camera", "eye": [0.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0], "fov_y_deg": null}),
    );
    assert_eq!(vertical, Err(ErrorKind::InvalidInput));
}

#[test]
fn query_results_round_trip_through_json() {
    let mut d = doc();
    draw_square(&mut d);
    let r = d.query(&Query::Entities).unwrap();
    let back: QueryResult = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
    assert_eq!(back, r);
}
