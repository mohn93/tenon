//! explode_view, driven through JSON commands.

use glam::DVec2;
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind, Query};

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

/// A unit-ish group (2 x 2 x 2 box) whose lower corner is at `at`; returns its id.
fn group_at(d: &mut Document, at: [f64; 3]) -> Value {
    let out = run(
        d,
        json!({"type": "add_box", "corner": at, "size": [2, 2, 2], "as": "group"}),
    )
    .unwrap();
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('i'))
        .unwrap()
        .clone()
}

fn bounds(d: &Document, id: &Value) -> [Value; 2] {
    let e = ask(d, json!({"type": "entity", "id": id}));
    [e["bounds"]["min"].clone(), e["bounds"]["max"].clone()]
}

fn min_of(d: &Document, id: &Value) -> [f64; 3] {
    let m = bounds(d, id)[0].clone();
    [0, 1, 2].map(|k| m[k].as_f64().unwrap())
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9)
}

fn depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

/// Three boxes with centers (1,1,1), (5,1,1), (9,1,1); the middle one is the default center.
fn row(d: &mut Document) -> [Value; 3] {
    [
        group_at(d, [0.0, 0.0, 0.0]),
        group_at(d, [4.0, 0.0, 0.0]),
        group_at(d, [8.0, 0.0, 0.0]),
    ]
}

#[test]
fn instances_move_away_from_the_center_of_their_boxes() {
    let mut d = doc();
    let [a, b, c] = row(&mut d);
    let before = depth(&d);
    let out = run(
        &mut d,
        json!({"type": "explode_view", "ids": [a, b, c], "factor": 1.0}),
    )
    .unwrap();
    // The middle box sits at the center and does not move.
    assert_eq!(out["changes"]["modified"].as_array().unwrap().len(), 2);
    // The default center is 5 (the middle of 0..10), so a and c move by -4 and +4.
    assert!(near(min_of(&d, &a), [-4.0, 0.0, 0.0]));
    assert!(near(min_of(&d, &b), [4.0, 0.0, 0.0]));
    assert!(near(min_of(&d, &c), [12.0, 0.0, 0.0]));
    assert_eq!(depth(&d), before + 1);
    assert_eq!(
        ask(&d, json!({"type": "history"}))["undo"][0],
        "explode_view"
    );
}

#[test]
fn an_explicit_center_replaces_the_default() {
    let mut d = doc();
    let [a, b, _] = row(&mut d);
    run(
        &mut d,
        json!({"type": "explode_view", "ids": [a, b], "factor": 2.0, "center": [1, 1, 1]}),
    )
    .unwrap();
    // a's center is the center: it stays; b's center is 4 away, moves by 8.
    assert!(near(min_of(&d, &a), [0.0, 0.0, 0.0]));
    assert!(near(min_of(&d, &b), [12.0, 0.0, 0.0]));
}

#[test]
fn factor_zero_changes_nothing_and_makes_no_undo_step() {
    let mut d = doc();
    let [a, b, c] = row(&mut d);
    let before = depth(&d);
    run(
        &mut d,
        json!({"type": "explode_view", "ids": [a, b, c], "factor": 0}),
    )
    .unwrap();
    assert_eq!(depth(&d), before);
    assert!(near(min_of(&d, &a), [0.0, 0.0, 0.0]));
}

#[test]
fn a_negative_factor_pulls_instances_in() {
    let mut d = doc();
    let [a, b, c] = row(&mut d);
    run(
        &mut d,
        json!({"type": "explode_view", "ids": [a, b, c], "factor": -0.5}),
    )
    .unwrap();
    assert!(near(min_of(&d, &a), [2.0, 0.0, 0.0]));
    assert!(near(min_of(&d, &b), [4.0, 0.0, 0.0]));
    assert!(near(min_of(&d, &c), [6.0, 0.0, 0.0]));
}

#[test]
fn bad_arguments_are_invalid_input() {
    let mut d = doc();
    let [a, b, _] = row(&mut d);
    let bad = |d: &mut Document, v: Value| run(d, v).unwrap_err().0;
    let made = run(
        &mut d,
        json!({"type": "add_edge", "start": [20, 0, 0], "end": [21, 0, 0]}),
    )
    .unwrap();
    let edge = made["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('e'))
        .unwrap()
        .clone();
    for factor in [json!(-1.0), json!(-0.99001), json!(100.5), json!(1e300)] {
        assert_eq!(
            bad(
                &mut d,
                json!({"type": "explode_view", "ids": [a, b], "factor": factor})
            ),
            ErrorKind::InvalidInput,
            "{factor}"
        );
    }
    assert_eq!(
        bad(
            &mut d,
            json!({"type": "explode_view", "ids": [], "factor": 1})
        ),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        bad(
            &mut d,
            json!({"type": "explode_view", "ids": [a, edge], "factor": 1})
        ),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        bad(
            &mut d,
            json!({"type": "explode_view", "ids": [a], "factor": 1, "center": [1e9, 0, 0]})
        ),
        ErrorKind::InvalidInput
    );
    // The edges of the bad calls changed nothing.
    assert!(near(min_of(&d, &a), [0.0, 0.0, 0.0]));
    // The ends of the range are fine.
    for factor in [-0.99, 100.0] {
        run(
            &mut d,
            json!({"type": "explode_view", "ids": [a, b], "factor": factor}),
        )
        .unwrap();
    }
}

#[test]
fn one_undo_puts_everything_back() {
    let mut d = doc();
    let [a, b, c] = row(&mut d);
    run(
        &mut d,
        json!({"type": "explode_view", "ids": [a, b, c], "factor": 3}),
    )
    .unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(near(min_of(&d, &a), [0.0, 0.0, 0.0]));
    assert!(near(min_of(&d, &b), [4.0, 0.0, 0.0]));
    assert!(near(min_of(&d, &c), [8.0, 0.0, 0.0]));
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert!(near(min_of(&d, &c), [20.0, 0.0, 0.0]));
}

#[test]
fn a_batch_can_explode_instances_it_just_made() {
    let mut d = doc();
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [2, 2, 2], "as": "group"}),
        json!({"type": "add_box", "corner": [4, 0, 0], "size": [2, 2, 2], "as": "group"}),
        json!({"type": "explode_view", "ids": ["$0:i0", "$1:i0"], "factor": 1}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
    assert_eq!(depth(&d), 1);
    let all = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    let mut xs: Vec<f64> = all["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["bounds"]["min"][0].as_f64().unwrap())
        .collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs, vec![-2.0, 6.0]);
}
