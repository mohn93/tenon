//! The wire contract agents rely on: input limits, canonical ids, the change log and batches.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, MAX_COORDINATE, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn cmd(v: Value) -> Command {
    serde_json::from_value(v).expect("valid command json")
}

fn edge(a: [f64; 3], b: [f64; 3]) -> Command {
    Command::AddEdge { start: a, end: b }
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn square() -> Vec<Command> {
    let c = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 2.0, 0.0],
    ];
    (0..4).map(|i| edge(c[i], c[(i + 1) % 4])).collect()
}

#[test]
fn coordinates_beyond_the_limit_are_rejected() {
    let mut d = doc();
    let far = MAX_COORDINATE * 2.0;
    let e = d.execute(edge([0.0; 3], [far, 0.0, 0.0])).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    let e = d
        .execute(cmd(json!({"type": "set_camera", "eye": [1e308, 0, 0], "target": [-1e308, 0, 0.5], "fov_y_deg": null})))
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert!(
        d.execute(edge([0.0; 3], [MAX_COORDINATE, 0.0, 0.0]))
            .is_ok()
    );
}

#[test]
fn only_canonical_ids_parse() {
    for bad in ["v+5", "v007", "f 3"] {
        assert!(
            serde_json::from_value::<Query>(json!({"type": "entity", "id": bad})).is_err(),
            "{bad}"
        );
    }
    assert!(serde_json::from_value::<Query>(json!({"type": "entity", "id": "v0"})).is_ok());
}

#[test]
fn stats_and_history_report_the_revision() {
    let mut d = doc();
    d.execute(edge([0.0; 3], [1.0, 0.0, 0.0])).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["revision"], 1);
    assert_eq!(ask(&d, json!({"type": "history"}))["revision"], 1);
}

#[test]
fn events_list_changes_after_a_revision() {
    let mut d = doc();
    for c in square() {
        d.execute(c).unwrap();
    }
    d.execute(Command::Undo).unwrap();
    let r = ask(&d, json!({"type": "events", "since": 3}));
    assert_eq!(r["revision"], 5);
    assert_eq!(r["truncated"], false);
    let events = r["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["revision"], 4);
    assert!(
        events[0]["changes"]["created"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id.as_str().unwrap().starts_with('f'))
    );
    assert!(
        events[1]["changes"]["deleted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id.as_str().unwrap().starts_with('f'))
    );
    let none = ask(&d, json!({"type": "events", "since": 5}));
    assert_eq!(none["events"], json!([]));
}

#[test]
fn old_events_are_dropped_and_reported_as_truncated() {
    let mut d = doc();
    for i in 0..1100 {
        let x = i as f64;
        d.execute(edge([x, 0.0, 0.0], [x, 1.0, 0.0])).unwrap();
    }
    let r = ask(&d, json!({"type": "events", "since": 0}));
    assert_eq!(r["truncated"], true);
    assert_eq!(r["events"].as_array().unwrap().len(), 1024);
    assert_eq!(
        ask(&d, json!({"type": "events", "since": 1090}))["truncated"],
        false
    );
}

#[test]
fn plain_batch_runs_in_order_and_stops_at_the_first_error() {
    let mut d = doc();
    let mut cmds = square();
    cmds.insert(2, edge([0.0; 3], [0.0; 3]));
    let err = d.execute_batch(cmds, false).unwrap_err();
    assert_eq!(err.index, 2);
    assert_eq!(err.completed.len(), 2);
    assert!(!err.rolled_back);
    assert_eq!(ask(&d, json!({"type": "stats"}))["edges"], 2);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 2);
}

#[test]
fn atomic_batch_is_one_undo_step() {
    let mut d = doc();
    let outcomes = d.execute_batch(square(), true).unwrap();
    assert_eq!(outcomes.len(), 4);
    assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 1);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
    assert_eq!(d.revision(), 1);
    d.execute(Command::Undo).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["edges"], 0);
}

#[test]
fn failed_atomic_batch_changes_nothing() {
    let mut d = doc();
    d.execute(edge([5.0, 5.0, 0.0], [6.0, 5.0, 0.0])).unwrap();
    let camera = d.camera.clone();
    let mut cmds = square();
    cmds.insert(
        1,
        cmd(json!({"type": "set_camera", "eye": [10.0, 0.0, 5.0], "target": [0.0, 0.0, 0.0], "fov_y_deg": null})),
    );
    cmds.push(edge([0.0; 3], [f64::NAN, 0.0, 0.0]));
    let err = d.execute_batch(cmds, true).unwrap_err();
    assert_eq!(err.index, 5);
    assert!(err.rolled_back);
    assert_eq!(ask(&d, json!({"type": "stats"}))["edges"], 1);
    assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 0);
    assert_eq!(d.camera, camera);
    assert_eq!(d.revision(), 1);
}

#[test]
fn atomic_batch_rejects_undo() {
    let mut d = doc();
    let err = d.execute_batch(vec![Command::Undo], true).unwrap_err();
    assert_eq!(err.error.kind, ErrorKind::InvalidInput);
}

#[test]
fn batch_error_serializes_for_clients() {
    let mut d = doc();
    let err = d
        .execute_batch(vec![edge([0.0; 3], [0.0; 3])], true)
        .unwrap_err();
    let v = serde_json::to_value(&err).unwrap();
    assert_eq!(v["index"], 0);
    assert_eq!(v["error"]["kind"], "invalid_input");
    assert_eq!(v["rolled_back"], true);
}

#[test]
fn events_since_far_future_revision_is_empty_and_not_truncated() {
    let mut d = doc();
    d.execute(edge([0.0; 3], [1.0, 0.0, 0.0])).unwrap();
    for since in [u64::MAX, 5] {
        let r = ask(&d, json!({"type": "events", "since": since}));
        assert_eq!(r["events"], json!([]), "since {since}");
        assert_eq!(r["truncated"], false, "since {since}");
    }
}

#[test]
fn truncation_boundary_is_exact() {
    let mut d = doc();
    for i in 0..1024 {
        let x = i as f64;
        d.execute(edge([x, 0.0, 0.0], [x, 1.0, 0.0])).unwrap();
    }
    let r = ask(&d, json!({"type": "events", "since": 0}));
    assert_eq!(r["events"].as_array().unwrap().len(), 1024);
    assert_eq!(r["truncated"], false);

    d.execute(edge([5000.0, 0.0, 0.0], [5000.0, 1.0, 0.0]))
        .unwrap();
    let r = ask(&d, json!({"type": "events", "since": 0}));
    assert_eq!(r["truncated"], true);
    let r = ask(&d, json!({"type": "events", "since": 1}));
    assert_eq!(r["truncated"], false);
    assert_eq!(r["events"].as_array().unwrap().len(), 1024);
}
