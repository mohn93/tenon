//! Move and copy, driven through JSON commands.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, Vec<String>)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
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

/// A 2x2 square on an empty model: vertices v0 v1 v3 v5, edges e2 e4 e6 e7, face f8.
fn square(d: &mut Document) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
}

#[test]
fn moving_a_face_moves_its_vertices_in_one_undo_step() {
    let mut d = doc();
    square(&mut d);
    let out = run(
        &mut d,
        json!({"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"].as_array().unwrap().len(), 4);
    let v = ask(&d, json!({"type": "entity", "id": "v0"}));
    assert_eq!(v["position"], json!([0.0, 0.0, 1.0]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    let v = ask(&d, json!({"type": "entity", "id": "v0"}));
    assert_eq!(v["position"], json!([0.0, 0.0, 0.0]));
}

#[test]
fn vertices_can_be_moved_within_their_face() {
    let mut d = doc();
    square(&mut d);
    run(
        &mut d,
        json!({"type": "move", "ids": ["v0"], "offset": [-1, -1, 0]}),
    )
    .unwrap();
    let area = ask(&d, json!({"type": "entity", "id": "f8"}))["area"]
        .as_f64()
        .unwrap();
    assert!((area - 6.0).abs() < 1e-9, "{area}");
}

#[test]
fn bad_moves_name_the_entities() {
    let mut d = doc();
    square(&mut d);
    let mv = |ids: Value, offset: Value| json!({"type": "move", "ids": ids, "offset": offset});
    assert_eq!(
        run(&mut d, mv(json!(["v0"]), json!([0, 0, 1]))).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["f8".to_string()]),
        "lifting one corner bends the face"
    );
    assert_eq!(
        run(&mut d, mv(json!(["v0", "e99"]), json!([1, 0, 0]))).unwrap_err(),
        (ErrorKind::NotFound, vec!["e99".to_string()])
    );
    assert_eq!(
        run(&mut d, mv(json!([]), json!([1, 0, 0]))).unwrap_err().0,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        run(&mut d, mv(json!(["f8"]), json!([2e6, 0, 0])))
            .unwrap_err()
            .0,
        ErrorKind::InvalidInput
    );
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
}

#[test]
fn copy_with_a_count_is_one_undo_step() {
    let mut d = doc();
    square(&mut d);
    run(
        &mut d,
        json!({"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 2}),
    )
    .unwrap();
    assert_eq!(counts(&d), (12, 12, 3));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (4, 4, 1));
    run(
        &mut d,
        json!({"type": "copy", "ids": ["e2"], "offset": [0, 3, 0]}),
    )
    .unwrap();
    assert_eq!(counts(&d), (6, 5, 1));
}

#[test]
fn bad_copies_change_nothing() {
    let mut d = doc();
    square(&mut d);
    let cp = |ids: Value, count: u32| json!({"type": "copy", "ids": ids, "offset": [3, 0, 0], "count": count});
    assert_eq!(
        run(&mut d, cp(json!(["v0"]), 1)).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["v0".to_string()])
    );
    for count in [0, 101] {
        assert_eq!(
            run(&mut d, cp(json!(["f8"]), count)).unwrap_err().0,
            ErrorKind::InvalidInput,
            "{count}"
        );
    }
    assert_eq!(
        run(
            &mut d,
            json!({"type": "copy", "ids": ["f8"], "offset": [6e5, 0, 0], "count": 2})
        )
        .unwrap_err()
        .0,
        ErrorKind::InvalidInput
    );
    assert_eq!(counts(&d), (4, 4, 1));
}

#[test]
fn a_tiny_move_far_from_the_origin_still_moves() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_edge", "start": [1e6, 1e6, 1e6], "end": [1e6, 1e6, 999999]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "move", "ids": ["e2"], "offset": [-1.5e-6, 0, 0]}),
    )
    .unwrap();
    let all = ask(&d, json!({"type": "entities", "kinds": ["vertex"]}));
    let xs: Vec<f64> = all["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["position"][0].as_f64().unwrap())
        .collect();
    assert!(xs.iter().all(|x| *x == 1e6 - 1.5e-6), "{xs:?}");
}
