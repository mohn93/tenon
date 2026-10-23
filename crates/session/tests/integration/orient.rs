//! Reverse Faces and Orient Faces, driven through JSON commands.

use glam::DVec2;
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind, Query};

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

fn undo_depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

/// A 2 x 2 x 2 box centred on (1, 1, 1); returns the ids of its six faces.
fn add_box(d: &mut Document) -> Vec<Value> {
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
    let distance = if down { -2.0 } else { 2.0 };
    run(
        d,
        json!({"type": "push_pull", "face": base, "distance": distance}),
    )
    .unwrap();
    ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .collect()
}

/// The faces whose normal points toward the centre of the box.
fn inward(d: &Document, faces: &[Value]) -> Vec<Value> {
    faces
        .iter()
        .filter(|id| {
            let f = ask(d, json!({"type": "entity", "id": id}));
            let n: Vec<f64> = f["normal"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect();
            let outer = f["outer"].as_array().unwrap();
            let mut c = [0.0; 3];
            for v in outer {
                let p = ask(d, json!({"type": "entity", "id": v}))["position"].clone();
                for i in 0..3 {
                    c[i] += p[i].as_f64().unwrap() / outer.len() as f64;
                }
            }
            let to_centre: f64 = (0..3).map(|i| n[i] * (1.0 - c[i])).sum();
            to_centre > 0.0
        })
        .cloned()
        .collect()
}

#[test]
fn reverse_faces_turns_faces_over_in_one_undo_step() {
    let mut d = doc();
    let faces = add_box(&mut d);
    assert!(inward(&d, &faces).is_empty());
    let depth = undo_depth(&d);
    let out = run(
        &mut d,
        json!({"type": "reverse_faces", "ids": [faces[0], faces[1]]}),
    )
    .unwrap();
    assert_eq!(out["changed"], json!([faces[0], faces[1]]));
    assert_eq!(inward(&d, &faces), vec![faces[0].clone(), faces[1].clone()]);
    assert_eq!(undo_depth(&d), depth + 1);
    assert_eq!(
        ask(&d, json!({"type": "history"}))["undo"][0],
        "reverse_faces"
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(inward(&d, &faces).is_empty());
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(inward(&d, &faces).len(), 2);
}

#[test]
fn reverse_faces_ignores_other_entities_and_empty_is_no_step() {
    let mut d = doc();
    let faces = add_box(&mut d);
    let edge = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"][0]["id"].clone();
    let depth = undo_depth(&d);
    let out = run(&mut d, json!({"type": "reverse_faces", "ids": [edge]})).unwrap();
    assert_eq!(out["changed"], json!([]));
    let out = run(&mut d, json!({"type": "reverse_faces", "ids": []})).unwrap();
    assert_eq!(out["changed"], json!([]));
    assert_eq!(undo_depth(&d), depth);
    // A mix reverses the face only.
    let out = run(
        &mut d,
        json!({"type": "reverse_faces", "ids": [edge, faces[2]]}),
    )
    .unwrap();
    assert_eq!(out["changed"], json!([faces[2]]));
    let (kind, _) = run(&mut d, json!({"type": "reverse_faces", "ids": ["f9999"]})).unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
}

#[test]
fn orient_faces_reports_what_it_flipped_and_is_one_undo_step() {
    let mut d = doc();
    let faces = add_box(&mut d);
    run(
        &mut d,
        json!({"type": "reverse_faces", "ids": [faces[1], faces[4]]}),
    )
    .unwrap();
    let depth = undo_depth(&d);
    let out = run(
        &mut d,
        json!({"type": "orient_faces", "id": faces[0], "ids": []}),
    )
    .unwrap();
    assert_eq!(
        out["orient"],
        json!({"flipped": [faces[1], faces[4]], "conflicts": 0})
    );
    assert!(inward(&d, &faces).is_empty());
    assert_eq!(undo_depth(&d), depth + 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(inward(&d, &faces).len(), 2);
}

#[test]
fn orient_faces_with_nothing_to_flip_makes_no_undo_step() {
    let mut d = doc();
    let faces = add_box(&mut d);
    let depth = undo_depth(&d);
    let out = run(
        &mut d,
        json!({"type": "orient_faces", "id": faces[0], "ids": []}),
    )
    .unwrap();
    assert_eq!(out["orient"], json!({"flipped": [], "conflicts": 0}));
    assert_eq!(undo_depth(&d), depth);
}

#[test]
fn orient_faces_rejects_non_faces_and_unknown_ids() {
    let mut d = doc();
    let faces = add_box(&mut d);
    let edge = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"][0]["id"].clone();
    let (kind, _) = run(&mut d, json!({"type": "orient_faces", "id": edge})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, _) = run(
        &mut d,
        json!({"type": "orient_faces", "id": faces[0], "ids": [edge]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, _) = run(&mut d, json!({"type": "orient_faces", "id": "f9999"})).unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
}

#[test]
fn orient_faces_stays_in_the_open_context() {
    let mut d = doc();
    let faces = add_box(&mut d);
    run(&mut d, json!({"type": "select", "ids": faces})).unwrap();
    let out = run(&mut d, json!({"type": "make_group", "ids": faces})).unwrap();
    // Faces inside the group are not in the root context.
    let (kind, _) = run(
        &mut d,
        json!({"type": "orient_faces", "id": faces[0], "ids": []}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    let (kind, _) = run(&mut d, json!({"type": "reverse_faces", "ids": [faces[0]]})).unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    let _ = out;
}

#[test]
fn both_commands_take_references_in_a_batch() {
    let mut d = doc();
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
        json!({"type": "push_pull", "face": "$0:f0", "distance": 2}),
        json!({"type": "reverse_faces", "ids": ["$0:f0"]}),
        json!({"type": "orient_faces", "id": "$0:f0", "ids": ["$1:f*"]}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let outcomes = d.execute_batch(commands, true).unwrap();
    assert_eq!(outcomes[2].changed.as_ref().unwrap().len(), 1);
    // The reversed base is the start: the five other faces turn to match it.
    assert_eq!(outcomes[3].orient.as_ref().unwrap().flipped.len(), 5);
    assert_eq!(outcomes[3].orient.as_ref().unwrap().conflicts, 0);
}
