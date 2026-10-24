//! M17a: ids in another context are reported with the path to open; make_group and
//! make_component report their definition.

use glam::DVec2;
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn created(out: &Value, prefix: char) -> Vec<Value> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i.as_str().unwrap().starts_with(prefix))
        .cloned()
        .collect()
}

/// A group g1 at the root holding a group g2 that holds a face and a section plane.
/// Returns (g1, g2, face, plane, root face); the open context is the root again.
fn nested(d: &mut Document) -> (Value, Value, Value, Value, Value) {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [10, 0, 0], "opposite": [11, 1, 0]}),
    )
    .unwrap();
    let root_face = created(&out, 'f')[0].clone();
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let ids: Vec<Value> = [created(&out, 'e'), created(&out, 'f')].concat();
    let g1 = run(d, json!({"type": "make_group", "ids": ids})).unwrap()["placed"][0].clone();
    run(d, json!({"type": "open_path", "path": [g1]})).unwrap();
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 5], "opposite": [1, 1, 5]}),
    )
    .unwrap();
    let face = created(&out, 'f')[0].clone();
    let ids: Vec<Value> = [created(&out, 'e'), vec![face.clone()]].concat();
    let g2 = run(d, json!({"type": "make_group", "ids": ids})).unwrap()["placed"][0].clone();
    run(d, json!({"type": "open_path", "path": [g1, g2]})).unwrap();
    // Grouping makes new ids for what it moves: ask what g2 holds.
    let faces = serde_json::to_value(
        d.query(&serde_json::from_value(json!({"type": "entities", "kinds": ["face"]})).unwrap())
            .unwrap(),
    )
    .unwrap();
    let face = faces["faces"][0]["id"].clone();
    let plane = created(
        &run(
            d,
            json!({"type": "add_section_plane", "point": [0, 0, 0], "normal": [0, 0, 1]}),
        )
        .unwrap(),
        's',
    )[0]
    .clone();
    run(d, json!({"type": "open_path", "path": []})).unwrap();
    (g1, g2, face, plane, root_face)
}

fn hint(id: &Value, g1: &Value, g2: &Value) -> String {
    format!(
        "{} is inside {} > {}; open that path first (open_path)",
        id.as_str().unwrap(),
        g1.as_str().unwrap(),
        g2.as_str().unwrap()
    )
}

#[test]
fn erase_move_and_paint_name_the_path() {
    let mut d = doc();
    let (g1, g2, face, _, _) = nested(&mut d);
    let want = hint(&face, &g1, &g2);
    for c in [
        json!({"type": "erase", "ids": [face]}),
        json!({"type": "move", "ids": [face], "offset": [1, 0, 0]}),
        json!({"type": "paint", "ids": [face]}),
    ] {
        let (kind, msg) = run(&mut d, c.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::NotFound, "{c}");
        assert_eq!(msg, want, "{c}");
    }
}

#[test]
fn flip_section_plane_names_the_path() {
    let mut d = doc();
    let (g1, g2, _, plane, _) = nested(&mut d);
    let (kind, msg) = run(
        &mut d,
        json!({"type": "flip_section_plane", "plane": plane}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    assert_eq!(msg, hint(&plane, &g1, &g2));
}

#[test]
fn a_path_one_level_deep_has_one_instance() {
    let mut d = doc();
    let (g1, _, _, _, _) = nested(&mut d);
    // The face of g1's own mesh: the root face of g1's wrapper.
    run(&mut d, json!({"type": "open_path", "path": [g1]})).unwrap();
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [3, 3, 0], "opposite": [4, 4, 0]}),
    )
    .unwrap();
    let f = created(&out, 'f')[0].clone();
    run(&mut d, json!({"type": "open_path", "path": []})).unwrap();
    let (_, msg) = run(&mut d, json!({"type": "erase", "ids": [f]})).unwrap_err();
    assert_eq!(
        msg,
        format!(
            "{} is inside {}; open that path first (open_path)",
            f.as_str().unwrap(),
            g1.as_str().unwrap()
        )
    );
}

#[test]
fn unknown_ids_keep_the_old_message() {
    let mut d = doc();
    nested(&mut d);
    let (kind, msg) = run(&mut d, json!({"type": "erase", "ids": ["f99999"]})).unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    assert_eq!(msg, "no such entities");
}

#[test]
fn batch_reference_to_an_id_in_another_context_names_the_path() {
    let mut d = doc();
    let (g1, g2, _, _, root_face) = nested(&mut d);
    // Open g1 > g2, make a face there and group it; then erase it by reference from the root.
    run(&mut d, json!({"type": "open_path", "path": [g1, g2]})).unwrap();
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_rectangle", "corner": [7, 7, 7], "opposite": [8, 8, 7]}),
        json!({"type": "open_path", "path": []}),
        json!({"type": "erase", "ids": ["$0:f0"]}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let err = d.execute_batch(commands, false).unwrap_err();
    assert_eq!(err.error.kind, ErrorKind::NotFound);
    assert!(
        err.error.message.contains(&format!(
            "is inside {} > {}; open that path first (open_path)",
            g1.as_str().unwrap(),
            g2.as_str().unwrap()
        )),
        "{}",
        err.error.message
    );
    let _ = root_face;
}

#[test]
fn make_group_and_make_component_report_their_definition() {
    let mut d = doc();
    for (kind, name) in [("make_group", None), ("make_component", Some("Tile"))] {
        let out = run(
            &mut d,
            json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
        )
        .unwrap();
        let ids: Vec<Value> = [created(&out, 'e'), created(&out, 'f')].concat();
        let mut c = json!({"type": kind, "ids": ids});
        if let Some(n) = name {
            c["name"] = json!(n);
        }
        let out = run(&mut d, c).unwrap();
        let defs = out["definitions"].as_array().expect("definitions");
        assert_eq!(defs.len(), 1, "{kind}");
        let inst = out["placed"][0].clone();
        let listed = serde_json::to_value(
            d.query(
                &serde_json::from_value(json!({"type": "entities", "kinds": ["instance"]}))
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let def = listed["instances"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == inst)
            .unwrap()["definition"]
            .clone();
        assert_eq!(defs[0], def, "{kind}");
    }
}

#[test]
fn batch_places_the_definition_of_a_component_it_just_made() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let ids: Vec<Value> = [created(&out, 'e'), created(&out, 'f')].concat();
    let commands: Vec<BatchCommand> = [
        json!({"type": "make_component", "ids": ids, "name": "Tile"}),
        json!({"type": "place_instance", "definition": "$0:d0", "transform": {"origin": [5, 0, 0]}}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let out = d.execute_batch(commands, true).unwrap();
    let a = serde_json::to_value(&out[0]).unwrap();
    let b = serde_json::to_value(&out[1]).unwrap();
    assert_eq!(a["definitions"].as_array().unwrap().len(), 1);
    assert_eq!(b["placed"].as_array().unwrap().len(), 1);
}
