//! Selection and erase, driven through JSON commands.

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

fn selection(d: &Document) -> Value {
    ask(d, json!({"type": "selection"}))["ids"].clone()
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
fn select_modes_combine_with_the_current_selection() {
    let mut d = doc();
    square(&mut d);
    let revision = d.revision();
    run(&mut d, json!({"type": "select", "ids": ["f8"]})).unwrap();
    assert_eq!(selection(&d), json!(["f8"]));
    run(
        &mut d,
        json!({"type": "select", "ids": ["e2", "e4"], "mode": "add"}),
    )
    .unwrap();
    assert_eq!(selection(&d), json!(["e2", "e4", "f8"]));
    run(
        &mut d,
        json!({"type": "select", "ids": ["f8", "e6"], "mode": "toggle"}),
    )
    .unwrap();
    assert_eq!(selection(&d), json!(["e2", "e4", "e6"]));
    run(
        &mut d,
        json!({"type": "select", "ids": ["e2"], "mode": "remove"}),
    )
    .unwrap();
    assert_eq!(selection(&d), json!(["e4", "e6"]));
    run(&mut d, json!({"type": "select", "ids": []})).unwrap();
    assert_eq!(selection(&d), json!([]));
    assert_eq!(
        d.revision(),
        revision,
        "selection does not change the model"
    );
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
}

#[test]
fn only_existing_edges_and_faces_can_be_selected() {
    let mut d = doc();
    square(&mut d);
    run(&mut d, json!({"type": "select", "ids": ["f8"]})).unwrap();
    assert_eq!(
        run(&mut d, json!({"type": "select", "ids": ["v0", "e2"]})).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["v0".to_string()])
    );
    assert_eq!(
        run(&mut d, json!({"type": "select", "ids": ["e2", "f99"]})).unwrap_err(),
        (ErrorKind::NotFound, vec!["f99".to_string()])
    );
    assert_eq!(selection(&d), json!(["f8"]), "unchanged after errors");
}

#[test]
fn erase_is_one_undo_step() {
    let mut d = doc();
    square(&mut d);
    let out = run(&mut d, json!({"type": "erase", "ids": ["e2"]})).unwrap();
    assert_eq!(
        out["changes"]["deleted"].as_array().unwrap().len(),
        2,
        "{out}"
    );
    assert_eq!(counts(&d), (4, 3, 0));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), (4, 4, 1));
    run(&mut d, json!({"type": "erase", "ids": ["f8"]})).unwrap();
    assert_eq!(counts(&d), (4, 4, 0));
}

#[test]
fn bad_erases_name_the_entities_and_change_nothing() {
    let mut d = doc();
    square(&mut d);
    assert_eq!(
        run(&mut d, json!({"type": "erase", "ids": ["v0"]})).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["v0".to_string()])
    );
    assert_eq!(
        run(&mut d, json!({"type": "erase", "ids": ["e2", "e99"]})).unwrap_err(),
        (ErrorKind::NotFound, vec!["e99".to_string()])
    );
    assert_eq!(
        run(&mut d, json!({"type": "erase", "ids": []}))
            .unwrap_err()
            .0,
        ErrorKind::InvalidInput
    );
    assert_eq!(counts(&d), (4, 4, 1));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
}

#[test]
fn entities_that_stop_existing_leave_the_selection() {
    let mut d = doc();
    square(&mut d);
    run(&mut d, json!({"type": "select", "ids": ["f8", "e2", "e4"]})).unwrap();
    run(&mut d, json!({"type": "erase", "ids": ["e2"]})).unwrap();
    assert_eq!(selection(&d), json!(["e4"]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(
        selection(&d),
        json!(["e4"]),
        "undo does not restore the selection"
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(selection(&d), json!([]));
}

#[test]
fn failed_atomic_batch_restores_the_selection() {
    let mut d = doc();
    square(&mut d);
    run(&mut d, json!({"type": "select", "ids": ["f8"]})).unwrap();
    let cmds = vec![
        cmd(json!({"type": "select", "ids": ["e2"]})),
        cmd(json!({"type": "erase", "ids": ["e4"]})),
        cmd(json!({"type": "erase", "ids": ["e99"]})),
    ];
    let err = d.execute_batch(cmds, true).unwrap_err();
    assert!(err.rolled_back);
    assert_eq!(selection(&d), json!(["f8"]));
    assert_eq!(counts(&d), (4, 4, 1));
}

#[test]
fn select_and_erase_are_classified_for_tools() {
    assert!(!cmd(json!({"type": "select", "ids": []})).changes_model());
    assert!(cmd(json!({"type": "erase", "ids": ["e1"]})).changes_model());
}
