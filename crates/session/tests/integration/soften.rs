//! Soft and hard edges, driven through JSON commands.

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

fn soft(d: &Document) -> usize {
    ask(d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["soft"] == true)
        .count()
}

/// A 2 x 2 x 1 m box; returns its top face.
fn add_box(d: &mut Document) -> Value {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    let base = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    let down = ask(d, json!({"type": "entity", "id": base}))["normal"][2]
        .as_f64()
        .unwrap()
        < 0.0;
    let distance = if down { -1.0 } else { 1.0 };
    run(
        d,
        json!({"type": "push_pull", "face": base, "distance": distance}),
    )
    .unwrap();
    let top = ask(
        d,
        json!({"type": "entities", "kinds": ["face"], "within": {"min": [-1, -1, 0.9], "max": [3, 3, 1.1]}}),
    );
    top["faces"][0]["id"].clone()
}

#[test]
fn soften_edges_softens_the_edges_of_faces_and_undo_hardens_them() {
    let mut d = doc();
    let top = add_box(&mut d);
    let out = run(
        &mut d,
        json!({"type": "soften_edges", "ids": [top], "soft": true}),
    )
    .unwrap();
    assert_eq!(out["soften"], json!({"changed": 4, "skipped": 0}));
    assert_eq!(out["changes"]["modified"].as_array().unwrap().len(), 4);
    assert_eq!(soft(&d), 4);

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(soft(&d), 0);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(soft(&d), 4);

    let out = run(
        &mut d,
        json!({"type": "soften_edges", "ids": [top], "soft": false}),
    )
    .unwrap();
    assert_eq!(out["soften"], json!({"changed": 4, "skipped": 0}));
    assert_eq!(soft(&d), 0);

    // A lone edge bounds no face: it is skipped, and nothing changes.
    let lone = run(
        &mut d,
        json!({"type": "add_edge", "start": [5, 0, 0], "end": [6, 0, 0]}),
    )
    .unwrap()["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('e'))
        .unwrap()
        .clone();
    let revision = ask(&d, json!({"type": "file"}))["revision"].clone();
    let out = run(
        &mut d,
        json!({"type": "soften_edges", "ids": [lone], "soft": true}),
    )
    .unwrap();
    assert_eq!(out["soften"], json!({"changed": 0, "skipped": 1}));
    assert_eq!(ask(&d, json!({"type": "file"}))["revision"], revision);
}

#[test]
fn soften_by_angle_takes_the_whole_context_or_the_given_entities() {
    let mut d = doc();
    let top = add_box(&mut d);
    let out = run(
        &mut d,
        json!({"type": "soften_by_angle", "max_angle_deg": 45}),
    )
    .unwrap();
    assert_eq!(out["soften"], json!({"changed": 0, "skipped": 12}));
    let out = run(
        &mut d,
        json!({"type": "soften_by_angle", "ids": [top], "max_angle_deg": 90}),
    )
    .unwrap();
    assert_eq!(out["soften"], json!({"changed": 4, "skipped": 0}));
    let out = run(
        &mut d,
        json!({"type": "soften_by_angle", "ids": [], "max_angle_deg": 90}),
    )
    .unwrap();
    assert_eq!(out["soften"], json!({"changed": 8, "skipped": 0}));
    assert_eq!(soft(&d), 12);
    // One undo step each.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(soft(&d), 4);
}

#[test]
fn soften_works_in_the_open_group_only() {
    let mut d = doc();
    add_box(&mut d);
    let all = ask(&d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    let ids: Vec<Value> = ["edges", "faces"]
        .iter()
        .flat_map(|k| all[k].as_array().unwrap().clone())
        .map(|e| e["id"].clone())
        .collect();
    run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    add_box(&mut d);
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let out = run(
        &mut d,
        json!({"type": "soften_by_angle", "max_angle_deg": 90}),
    )
    .unwrap();
    assert_eq!(out["soften"]["changed"], 12);
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    assert_eq!(soft(&d), 0, "the box outside the group stays hard");
}

#[test]
fn bad_soften_input_is_refused() {
    let mut d = doc();
    let top = add_box(&mut d);
    for (command, kind) in [
        (
            json!({"type": "soften_edges", "ids": [], "soft": true}),
            ErrorKind::InvalidInput,
        ),
        (
            json!({"type": "soften_edges", "ids": ["v0"], "soft": true}),
            ErrorKind::InvalidInput,
        ),
        (
            json!({"type": "soften_edges", "ids": ["f999"], "soft": true}),
            ErrorKind::NotFound,
        ),
        (
            json!({"type": "soften_by_angle", "ids": [top], "max_angle_deg": 181}),
            ErrorKind::InvalidInput,
        ),
        (
            json!({"type": "soften_by_angle", "max_angle_deg": -1}),
            ErrorKind::InvalidInput,
        ),
    ] {
        let (k, message) = run(&mut d, command.clone()).unwrap_err();
        assert_eq!(k, kind, "{command}: {message}");
    }
    let guide = run(
        &mut d,
        json!({"type": "add_guide_point", "position": [5, 5, 0]}),
    )
    .unwrap()["changes"]["created"][0]
        .clone();
    let cmd: Command =
        serde_json::from_value(json!({"type": "soften_edges", "ids": [guide], "soft": true}))
            .unwrap();
    let e = d.execute(cmd).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);
    assert_eq!(serde_json::to_value(&e.ids).unwrap(), json!([guide]));
    assert_eq!(soft(&d), 0);
}

#[test]
fn an_atomic_batch_reports_each_soften() {
    let mut d = doc();
    let top = add_box(&mut d);
    let cmds: Vec<Command> = [
        json!({"type": "soften_edges", "ids": [top], "soft": true}),
        json!({"type": "soften_by_angle", "max_angle_deg": 90}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).unwrap())
    .collect();
    let outcomes = d.execute_batch(cmds, true).unwrap();
    let reports: Vec<Value> = outcomes
        .iter()
        .map(|o| serde_json::to_value(o).unwrap()["soften"].clone())
        .collect();
    assert_eq!(
        reports,
        [
            json!({"changed": 4, "skipped": 0}),
            json!({"changed": 8, "skipped": 0})
        ]
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(soft(&d), 0);
}
