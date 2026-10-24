//! `place_instance` and `set_transform`, driven through JSON commands.

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

fn depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

fn instances(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities", "kinds": ["instance"]}))["instances"]
        .as_array()
        .unwrap()
        .clone()
}

/// Wraps a new square face in a component or group; returns (instance, definition).
fn wrapped(d: &mut Document, kind: &str) -> (Value, Value) {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let ids: Vec<Value> = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i.as_str().unwrap().starts_with(['e', 'f']))
        .cloned()
        .collect();
    let mut c = json!({"type": kind, "ids": ids});
    if kind == "make_component" {
        c["name"] = json!("Tile");
    }
    let out = run(d, c).unwrap();
    let inst = out["placed"][0].clone();
    let def = instances(d).iter().find(|i| i["id"] == inst).unwrap()["definition"].clone();
    (inst, def)
}

fn matrix_of(d: &Document, id: &Value) -> Vec<f64> {
    instances(d).iter().find(|i| &i["id"] == id).unwrap()["transform"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}

#[test]
fn place_with_axes_form_places_a_shared_instance() {
    let mut d = doc();
    let (_, def) = wrapped(&mut d, "make_component");
    let steps = depth(&d);
    let out = run(
        &mut d,
        json!({"type": "place_instance", "definition": def,
               "transform": {"origin": [5, 6, 7], "x_axis": [0, 2, 0], "y_axis": [-2, 0, 0], "z_axis": [0, 0, 2]}}),
    )
    .unwrap();
    let placed = out["placed"][0].clone();
    assert_eq!(out["changes"]["created"], json!([placed]));
    assert_eq!(depth(&d), steps + 1);
    assert_eq!(
        matrix_of(&d, &placed),
        vec![0.0, 2.0, 0.0, -2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 5.0, 6.0, 7.0]
    );
    assert_eq!(instances(&d).len(), 2);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(instances(&d).len(), 1);
}

#[test]
fn defaults_are_identity() {
    let mut d = doc();
    let (_, def) = wrapped(&mut d, "make_component");
    let out = run(
        &mut d,
        json!({"type": "place_instance", "definition": def, "transform": {"origin": [1, 0, 0]}}),
    )
    .unwrap();
    assert_eq!(
        matrix_of(&d, &out["placed"][0]),
        vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0]
    );
}

#[test]
fn query_matrix_round_trips_through_matrix_form() {
    let mut d = doc();
    let (inst, def) = wrapped(&mut d, "make_component");
    run(
        &mut d,
        json!({"type": "set_transform", "id": inst,
               "transform": {"origin": [1, 2, 3], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}}),
    )
    .unwrap();
    let m = matrix_of(&d, &inst);
    let out = run(
        &mut d,
        json!({"type": "place_instance", "definition": def, "transform": {"matrix": m}}),
    )
    .unwrap();
    assert_eq!(matrix_of(&d, &out["placed"][0]), m);
}

#[test]
fn set_transform_replaces_and_undoes_exactly() {
    let mut d = doc();
    let (inst, _) = wrapped(&mut d, "make_component");
    let before = matrix_of(&d, &inst);
    let steps = depth(&d);
    let out = run(
        &mut d,
        json!({"type": "set_transform", "id": inst,
               "transform": {"matrix": [1, 0, 0, 0, 1, 0, 0, 0, -1, 4, 5, 6]}}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"], json!([inst]));
    assert_eq!(
        matrix_of(&d, &inst),
        vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0, 4.0, 5.0, 6.0]
    );
    assert_eq!(depth(&d), steps + 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(matrix_of(&d, &inst), before);
}

#[test]
fn bad_transforms_are_refused() {
    let mut d = doc();
    let (inst, def) = wrapped(&mut d, "make_component");
    let steps = depth(&d);
    let bad = [
        json!({"x_axis": [1, 0, 0], "y_axis": [2, 0, 0]}),
        json!({"z_axis": [0, 0, 0]}),
        json!({"matrix": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]}),
        json!({"origin": [2e6, 0, 0]}),
        json!({"matrix": [1e300, 0, 0, 0, 1e300, 0, 0, 0, 1e300, 0, 0, 0]}),
        json!({"matrix": [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0]}),
        json!({"matrix": [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0], "origin": [0, 0, 0]}),
    ];
    for t in bad {
        for c in [
            json!({"type": "place_instance", "definition": def, "transform": t}),
            json!({"type": "set_transform", "id": inst, "transform": t}),
        ] {
            // A wrong-length matrix is refused while parsing.
            if let Ok(cmd) = serde_json::from_value::<Command>(c) {
                let e = d.execute(cmd).unwrap_err();
                assert_eq!(e.kind, ErrorKind::InvalidInput, "{t}");
            }
        }
    }
    // NaN cannot be written in JSON, but a Command can hold one.
    let nan = session::Transform {
        origin: Some([f64::NAN, 0.0, 0.0]),
        ..Default::default()
    };
    let e = d
        .execute(Command::SetTransform {
            id: serde_json::from_value(inst.clone()).unwrap(),
            transform: nan,
        })
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert_eq!(depth(&d), steps);
}

#[test]
fn a_mirror_is_allowed() {
    let mut d = doc();
    let (_, def) = wrapped(&mut d, "make_component");
    run(
        &mut d,
        json!({"type": "place_instance", "definition": def, "transform": {"x_axis": [-1, 0, 0]}}),
    )
    .unwrap();
}

#[test]
fn unknown_definition_and_instance_are_not_found() {
    let mut d = doc();
    let (inst, _) = wrapped(&mut d, "make_component");
    let e = run(
        &mut d,
        json!({"type": "place_instance", "definition": "d999"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    let e = run(
        &mut d,
        json!({"type": "set_transform", "id": "i999", "transform": {}}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    let _ = inst;
}

#[test]
fn placing_a_definition_inside_itself_is_refused() {
    let mut d = doc();
    let (inner, inner_def) = wrapped(&mut d, "make_component");
    // Wrap the component in a group, then open the group.
    let out = run(&mut d, json!({"type": "make_group", "ids": [inner]})).unwrap();
    let outer = out["placed"][0].clone();
    let outer_def = instances(&d).iter().find(|i| i["id"] == outer).unwrap()["definition"].clone();
    run(&mut d, json!({"type": "open_instance", "id": outer})).unwrap();
    // Inside the group: the group's own definition, and (through nesting) nothing else bad.
    let e = run(
        &mut d,
        json!({"type": "place_instance", "definition": outer_def}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    // Open the nested component; placing the group (which contains it) there is a cycle.
    let inner_in_group = instances(&d)[0]["id"].clone();
    run(
        &mut d,
        json!({"type": "open_instance", "id": inner_in_group}),
    )
    .unwrap();
    let steps = depth(&d);
    for def in [&outer_def, &inner_def] {
        let e = run(&mut d, json!({"type": "place_instance", "definition": def})).unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput);
    }
    assert_eq!(depth(&d), steps);
    // Back in the group, the component (which does not contain the group) is fine.
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    run(
        &mut d,
        json!({"type": "place_instance", "definition": inner_def, "transform": {"origin": [3, 0, 0]}}),
    )
    .unwrap();
}

#[test]
fn a_group_definition_placed_twice_shows_edits_in_both() {
    let mut d = doc();
    let (g, def) = wrapped(&mut d, "make_group");
    let out = run(
        &mut d,
        json!({"type": "place_instance", "definition": def, "transform": {"origin": [5, 0, 0]}}),
    )
    .unwrap();
    let second = out["placed"][0].clone();
    let defs = ask(&d, json!({"type": "definitions"}));
    let entry = defs["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == def)
        .unwrap();
    assert_eq!(entry["instances"], 2);
    run(&mut d, json!({"type": "open_instance", "id": g})).unwrap();
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 1}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    // Both instances show the extruded box: the second one's bounds are 1 high.
    for id in [g, second] {
        let b = instances(&d).iter().find(|i| i["id"] == id).unwrap()["bounds"].clone();
        let (min, max) = (b["min"][2].as_f64().unwrap(), b["max"][2].as_f64().unwrap());
        assert!((max - min - 1.0).abs() < 1e-9, "{b}");
    }
}

#[test]
fn batch_places_with_a_definition_reference_and_sets_a_placed_transform() {
    let mut d = doc();
    let (_, def) = wrapped(&mut d, "make_component");
    let commands: Vec<BatchCommand> = [
        json!({"type": "place_instance", "definition": def, "transform": {"origin": [2, 0, 0]}}),
        json!({"type": "set_transform", "id": "$0:i0", "transform": {"origin": [9, 0, 0]}}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let out = d.execute_batch(commands, true).unwrap();
    let placed = serde_json::to_value(&out[0]).unwrap()["placed"][0].clone();
    assert_eq!(matrix_of(&d, &placed)[9], 9.0);
}
