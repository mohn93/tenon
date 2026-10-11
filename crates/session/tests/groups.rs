//! Groups, components and the open context, driven through JSON commands.

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

/// Vertices, edges, faces and instances in the open context, and definitions in the model.
fn counts(d: &Document) -> [u64; 5] {
    let s = ask(d, json!({"type": "stats"}));
    ["vertices", "edges", "faces", "instances", "definitions"].map(|k| s[k].as_u64().unwrap())
}

/// A 2 x 2 x 1 box on an empty model.
fn cube(d: &mut Document) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "push_pull", "face": "f8", "distance": 1.0}),
    )
    .unwrap();
}

/// Every edge and face id in the open context.
fn everything(d: &Document) -> Vec<Value> {
    let all = ask(d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect()
}

/// The instance id among a command's created ids.
fn created_instance(out: &Value) -> String {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .find(|id| id.starts_with('i'))
        .expect("an instance was created")
}

/// Groups the whole box and returns the instance id.
fn grouped_cube(d: &mut Document, command: Value) -> String {
    cube(d);
    let mut command = command;
    command["ids"] = Value::Array(everything(d));
    let out = run(d, command).unwrap();
    created_instance(&out)
}

/// The id of the face whose normal is +Z.
fn top_face(d: &Document) -> String {
    let faces = ask(d, json!({"type": "entities", "kinds": ["face"]}));
    faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"] == json!([0.0, 0.0, 1.0]))
        .map(|f| f["id"].as_str().unwrap().to_string())
        .unwrap()
}

#[test]
fn making_a_group_is_one_undo_step_and_selects_it() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    assert_eq!(counts(&d), [0, 0, 0, 1, 1]);
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([i]));
    let info = ask(&d, json!({"type": "entity", "id": i}));
    assert_eq!(info["kind"], "group");
    assert_eq!(
        info["bounds"],
        json!({"min": [0.0, 0.0, 0.0], "max": [2.0, 2.0, 1.0]})
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), [8, 12, 6, 0, 0]);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 1, 1]);
}

#[test]
fn an_open_instance_is_edited_in_its_own_coordinates() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(
        &mut d,
        json!({"type": "move", "ids": [i], "offset": [10, 0, 0]}),
    )
    .unwrap();
    let moved = ask(&d, json!({"type": "entity", "id": i}));
    assert_eq!(moved["bounds"]["min"], json!([10.0, 0.0, 0.0]));

    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    let context = ask(&d, json!({"type": "context"}));
    assert_eq!(context["path"], json!([i]));
    assert_eq!(context["definition"], moved["definition"]);
    assert_eq!(counts(&d), [8, 12, 6, 0, 1]);
    let inside = ask(&d, json!({"type": "entities", "kinds": ["vertex"]}));
    let xs: Vec<f64> = inside["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["position"][0].as_f64().unwrap())
        .collect();
    assert!(xs.iter().all(|x| (0.0..=2.0).contains(x)), "{xs:?}");

    let top = top_face(&d);
    run(
        &mut d,
        json!({"type": "push_pull", "face": top, "distance": 1.0}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "context"}))["path"], json!([]));
    let grown = ask(&d, json!({"type": "entity", "id": i}));
    assert_eq!(
        grown["bounds"],
        json!({"min": [10.0, 0.0, 0.0], "max": [12.0, 2.0, 2.0]})
    );
}

#[test]
fn editing_a_component_changes_every_instance() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_component", "name": "Crate"}));
    run(
        &mut d,
        json!({"type": "copy", "ids": [i], "offset": [3, 0, 0], "count": 2}),
    )
    .unwrap();
    let defs = ask(&d, json!({"type": "definitions"}));
    assert_eq!(defs["definitions"].as_array().unwrap().len(), 1);
    assert_eq!(defs["definitions"][0]["name"], "Crate");
    assert_eq!(defs["definitions"][0]["instances"], 3);

    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    let top = top_face(&d);
    run(
        &mut d,
        json!({"type": "push_pull", "face": top, "distance": 1.0}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let all = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    let tops: Vec<f64> = all["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["bounds"]["max"][2].as_f64().unwrap())
        .collect();
    assert_eq!(tops, [2.0, 2.0, 2.0]);
}

#[test]
fn a_copied_group_is_independent() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(
        &mut d,
        json!({"type": "copy", "ids": [i], "offset": [3, 0, 0]}),
    )
    .unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 2, 2]);
}

#[test]
fn erasing_a_group_removes_its_definition_until_undone() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(&mut d, json!({"type": "erase", "ids": [i]})).unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 0, 0]);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 1, 1]);
}

#[test]
fn exploding_selects_what_comes_out() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(&mut d, json!({"type": "explode", "ids": [i]})).unwrap();
    assert_eq!(counts(&d), [8, 12, 6, 0, 0]);
    let selected = ask(&d, json!({"type": "selection"}));
    assert_eq!(selected["ids"].as_array().unwrap().len(), 12 + 6);
}

#[test]
fn undoing_the_group_while_inside_it_closes_it() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "context"}))["path"], json!([]));
    assert_eq!(counts(&d), [8, 12, 6, 0, 0]);
}

#[test]
fn a_failed_atomic_batch_restores_the_open_context() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "open_instance", "id": i},
        {"type": "push_pull", "face": "f999", "distance": 1.0}
    ]))
    .unwrap();
    let err = d.execute_batch(commands, true).unwrap_err();
    assert_eq!(err.index, 1);
    assert_eq!(ask(&d, json!({"type": "context"}))["path"], json!([]));
}

#[test]
fn group_commands_reject_bad_ids() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    let invalid = |r: Result<Value, (ErrorKind, Vec<String>)>| r.unwrap_err().0;
    assert_eq!(
        invalid(run(&mut d, json!({"type": "make_group", "ids": []}))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        run(&mut d, json!({"type": "make_group", "ids": ["v0"]})).unwrap_err(),
        (ErrorKind::InvalidInput, vec!["v0".to_string()])
    );
    assert_eq!(
        invalid(run(
            &mut d,
            json!({"type": "make_component", "ids": [i], "name": " "})
        )),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        invalid(run(&mut d, json!({"type": "explode", "ids": ["f8"]}))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        invalid(run(&mut d, json!({"type": "open_instance", "id": "f8"}))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        invalid(run(&mut d, json!({"type": "open_instance", "id": "i999"}))),
        ErrorKind::NotFound
    );
    assert_eq!(
        invalid(run(&mut d, json!({"type": "close_instance"}))),
        ErrorKind::InvalidInput
    );
    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    assert_eq!(
        run(&mut d, json!({"type": "select", "ids": [i]})).unwrap_err(),
        (ErrorKind::NotFound, vec![i.clone()]),
        "the open instance is not in its own context"
    );
}

#[test]
fn instances_filter_by_kind_and_box() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    run(
        &mut d,
        json!({"type": "add_edge", "start": [5, 0, 0], "end": [6, 0, 0]}),
    )
    .unwrap();
    let only = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    assert_eq!(only["instances"].as_array().unwrap().len(), 1);
    assert_eq!(only["edges"], json!([]));
    let inside = ask(
        &d,
        json!({"type": "entities", "within": {"min": [-1, -1, -1], "max": [3, 3, 3]}}),
    );
    assert_eq!(inside["instances"][0]["id"], json!(i));
    assert_eq!(inside["edges"], json!([]));
    let outside = ask(
        &d,
        json!({"type": "entities", "within": {"min": [0, 0, 0], "max": [1, 1, 1]}}),
    );
    assert_eq!(outside["instances"], json!([]));
}

#[test]
fn repeated_ids_count_once() {
    let mut d = doc();
    let i = grouped_cube(&mut d, json!({"type": "make_group"}));
    let out = run(&mut d, json!({"type": "make_group", "ids": [i, i]})).unwrap();
    let outer = created_instance(&out);
    run(&mut d, json!({"type": "explode", "ids": [outer, outer]})).unwrap();
    assert_eq!(counts(&d), [0, 0, 0, 1, 1]);
}
