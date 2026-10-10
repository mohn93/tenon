//! Rectangles, push/pull and the entities filter, driven through JSON commands.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn cmd(v: Value) -> Command {
    serde_json::from_value(v).expect("valid command json")
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, Vec<String>)> {
    d.execute(cmd(v))
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.ids.iter().map(|i| i.to_string()).collect()))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn counts(d: &Document) -> (u64, u64, u64) {
    let s = ask(d, json!({"type": "stats"}));
    let n = |k: &str| s[k].as_u64().unwrap();
    (n("vertices"), n("edges"), n("faces"))
}

/// The id of the one face an outcome created.
fn created_face(outcome: &Value) -> String {
    let faces: Vec<&str> = outcome["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .filter(|id| id.starts_with('f'))
        .collect();
    assert_eq!(faces.len(), 1, "{outcome}");
    faces[0].to_string()
}

fn rectangle(d: &mut Document, corner: [f64; 3], opposite: [f64; 3]) -> String {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": corner, "opposite": opposite}),
    )
    .unwrap();
    created_face(&out)
}

#[test]
fn rectangle_on_the_ground_is_one_undo_step() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 3.0, 0.0]);
    assert_eq!(counts(&d), (4, 4, 1));
    let info = ask(&d, json!({"type": "entity", "id": face}));
    assert_eq!(info["area"], json!(6.0));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (0, 0, 0));
}

#[test]
fn rectangle_on_a_tilted_plane_follows_its_normal() {
    let mut d = doc();
    let a = 0.5f64.sqrt();
    // In the plane with normal (0, a, a): u = -X, v = (0, -a, a).
    let out = run(
        &mut d,
        json!({
            "type": "add_rectangle",
            "corner": [1.0, 0.0, 0.0],
            "opposite": [-1.0, -1.5 * a, 1.5 * a],
            "normal": [0.0, 1.0, 1.0]
        }),
    )
    .unwrap();
    let info = ask(&d, json!({"type": "entity", "id": created_face(&out)}));
    assert!((info["area"].as_f64().unwrap() - 3.0).abs() < 1e-9);
    let n: Vec<f64> = serde_json::from_value(info["normal"].clone()).unwrap();
    assert!((n[0]).abs() < 1e-9 && (n[1] - a).abs() < 1e-9 && (n[2] - a).abs() < 1e-9);
}

#[test]
fn bad_rectangles_change_nothing() {
    let mut d = doc();
    for bad in [
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 1]}),
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 0, 0]}),
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 1], "normal": [0, 0, 1]}),
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2e6, 1, 0]}),
    ] {
        assert_eq!(
            run(&mut d, bad.clone()).unwrap_err().0,
            ErrorKind::InvalidInput,
            "{bad}"
        );
    }
    assert_eq!(counts(&d), (0, 0, 0));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 0);
}

#[test]
fn push_pull_makes_a_box_in_one_undo_step() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 1.5}),
    )
    .unwrap();
    assert_eq!(counts(&d), (8, 12, 6));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (4, 4, 1));
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(counts(&d), (8, 12, 6));
}

#[test]
fn pulling_a_box_top_reports_moved_vertices() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    let out = run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 1.0}),
    )
    .unwrap();
    let top = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .find(|id| {
            id.starts_with('f')
                && ask(&d, json!({"type": "entity", "id": id}))["normal"] == json!([0.0, 0.0, 1.0])
        })
        .unwrap();
    let out = run(
        &mut d,
        json!({"type": "push_pull", "face": top, "distance": 0.5}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"].as_array().unwrap().len(), 4);
    assert_eq!(counts(&d), (8, 12, 6));
}

#[test]
fn bad_push_pulls_name_the_entity() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    let pp =
        |id: &str, distance: f64| json!({"type": "push_pull", "face": id, "distance": distance});
    assert_eq!(
        run(&mut d, pp("e2", 1.0)).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["e2".to_string()])
    );
    assert_eq!(
        run(&mut d, pp("f999", 1.0)).unwrap_err(),
        (ErrorKind::NotFound, vec!["f999".to_string()])
    );
    assert_eq!(
        run(&mut d, pp(&face, 0.0)).unwrap_err().0,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        run(&mut d, pp(&face, 2e6)).unwrap_err().0,
        ErrorKind::InvalidInput
    );
    assert!(
        serde_json::from_value::<Command>(json!({"type": "push_pull", "face": face, "dist": 1}))
            .is_err()
    );
    assert_eq!(counts(&d), (4, 4, 1));
}

#[test]
fn atomic_batch_of_rectangle_and_push_pull_is_one_undo_step() {
    let mut d = doc();
    // Ids are allocated in order, so on an empty model the rectangle's face is f8.
    let cmds = vec![
        cmd(json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]})),
        cmd(json!({"type": "push_pull", "face": "f8", "distance": 1})),
    ];
    d.execute_batch(cmds, true).unwrap();
    assert_eq!(counts(&d), (8, 12, 6));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (0, 0, 0));
}

#[test]
fn atomic_batches_without_model_changes_add_no_undo_step() {
    let mut d = doc();
    let camera_only = vec![cmd(
        json!({"type": "set_camera", "eye": [5, 5, 5], "target": [0, 0, 0]}),
    )];
    d.execute_batch(camera_only, true).unwrap();
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 0);
    assert_eq!(d.revision(), 0);

    rectangle(&mut d, [0.0, 0.0, 0.0], [1.0, 1.0, 0.0]);
    let existing = vec![cmd(
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    )];
    let out = d.execute_batch(existing, true).unwrap();
    assert!(out[0].changes.created.is_empty());
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
    assert_eq!(d.revision(), 1);
}

#[test]
fn mixed_atomic_batch_rolls_back_the_camera_too() {
    let mut d = doc();
    let camera = d.camera.clone();
    let cmds = vec![
        cmd(json!({"type": "set_camera", "eye": [5, 5, 5], "target": [0, 0, 0]})),
        cmd(json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]})),
        cmd(json!({"type": "push_pull", "face": "f999", "distance": 1})),
    ];
    let err = d.execute_batch(cmds, true).unwrap_err();
    assert_eq!(err.index, 2);
    assert!(err.rolled_back);
    assert_eq!(d.camera, camera);
    assert_eq!(counts(&d), (0, 0, 0));
}

#[test]
fn entities_within_bounds() {
    let mut d = doc();
    rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    rectangle(&mut d, [5.0, 5.0, 0.0], [6.0, 6.0, 0.0]);
    let r = ask(
        &d,
        json!({"type": "entities", "within": {"min": [-1, -1, -1], "max": [3, 3, 1]}}),
    );
    assert_eq!(r["vertices"].as_array().unwrap().len(), 4);
    assert_eq!(r["edges"].as_array().unwrap().len(), 4);
    assert_eq!(r["faces"].as_array().unwrap().len(), 1);
    let edge_on_boundary = ask(
        &d,
        json!({"type": "entities", "kinds": ["edge"], "within": {"min": [0, 0, 0], "max": [2, 0, 0]}}),
    );
    assert_eq!(edge_on_boundary["edges"].as_array().unwrap().len(), 1);
}

#[test]
fn rectangle_after_a_box_faces_up() {
    let mut d = doc();
    let base = rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    run(
        &mut d,
        json!({"type": "push_pull", "face": base, "distance": 1.0}),
    )
    .unwrap();
    let face = rectangle(&mut d, [5.0, 0.0, 0.0], [6.0, 1.0, 0.0]);
    let info = ask(&d, json!({"type": "entity", "id": face}));
    assert_eq!(info["normal"], json!([0.0, 0.0, 1.0]));
}
