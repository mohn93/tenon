//! Make Unique and Flip Along, driven through JSON commands.

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

fn created(out: &Value, prefix: char) -> Vec<Value> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with(prefix))
        .cloned()
        .collect()
}

fn depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

/// Makes a 1 x 1 x 1 box at `at` and wraps it in `kind` ("make_group" or "make_component");
/// returns the instance.
fn wrapped_box(d: &mut Document, at: [f64; 3], kind: &str) -> Value {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": at, "opposite": [at[0] + 1.0, at[1] + 1.0, at[2]]}),
    )
    .unwrap();
    let face = created(&out, 'f')[0].clone();
    run(d, json!({"type": "push_pull", "face": face, "distance": 1})).unwrap();
    let all = ask(d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    let ids: Vec<Value> = all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect();
    let mut command = json!({"type": kind, "ids": ids});
    if kind == "make_component" {
        command["name"] = json!("Chair");
    }
    created(&run(d, command).unwrap(), 'i')[0].clone()
}

/// The ids of the copies `copy` makes of `ids`, `count` of them.
fn copies(d: &mut Document, ids: &[Value], count: u32) -> Vec<Value> {
    let out = run(
        d,
        json!({"type": "copy", "ids": ids, "offset": [3, 0, 0], "count": count}),
    )
    .unwrap();
    created(&out, 'i')
}

fn definitions(d: &Document) -> Value {
    ask(d, json!({"type": "definitions"}))
}

fn instance_count(d: &Document, name: &str) -> u64 {
    definitions(d)["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .map_or(0, |x| x["instances"].as_u64().unwrap())
}

#[test]
fn make_unique_splits_the_selected_instances_off_a_shared_component() {
    let mut d = doc();
    let first = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_component");
    let more = copies(&mut d, std::slice::from_ref(&first), 2);
    assert_eq!(instance_count(&d, "Chair"), 3);
    let before = definitions(&d);
    let steps = depth(&d);

    let out = run(
        &mut d,
        json!({"type": "make_unique", "ids": [more[0], more[1]]}),
    )
    .unwrap();

    assert_eq!(out["definitions"].as_array().unwrap().len(), 1);
    assert_eq!(instance_count(&d, "Chair"), 1);
    assert_eq!(instance_count(&d, "Chair#2"), 2);
    assert_eq!(depth(&d), steps + 1);
    assert_eq!(
        ask(&d, json!({"type": "history"}))["undo"][0],
        "make_unique"
    );
    let new = out["definitions"][0].as_str().unwrap();
    let info = ask(&d, json!({"type": "entity", "id": more[0]}));
    assert_eq!(info["definition"], new);

    run(&mut d, json!({"type": "undo"})).unwrap();
    let mut restored = definitions(&d);
    restored["revision"] = before["revision"].clone();
    assert_eq!(restored, before);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(instance_count(&d, "Chair#2"), 2);
}

#[test]
fn make_unique_names_count_up_and_skip_trailing_numbers() {
    let mut d = doc();
    let first = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_component");
    let more = copies(&mut d, std::slice::from_ref(&first), 3);
    run(&mut d, json!({"type": "make_unique", "ids": [more[0]]})).unwrap();
    run(&mut d, json!({"type": "make_unique", "ids": [more[1]]})).unwrap();
    assert_eq!(instance_count(&d, "Chair#2"), 1);
    assert_eq!(instance_count(&d, "Chair#3"), 1);
    assert_eq!(instance_count(&d, "Chair"), 2);
    // A copy of Chair#2 does not become Chair#2#N.
    let again = copies(&mut d, std::slice::from_ref(&more[0]), 1);
    run(&mut d, json!({"type": "make_unique", "ids": [again[0]]})).unwrap();
    assert_eq!(instance_count(&d, "Chair#4"), 1);
}

#[test]
fn make_unique_defaults_to_the_selection_and_leaves_groups_and_sole_instances() {
    let mut d = doc();
    let group = wrapped_box(&mut d, [0.0, 5.0, 0.0], "make_group");
    let sole = wrapped_box(&mut d, [0.0, 8.0, 0.0], "make_component");
    let steps = depth(&d);
    run(
        &mut d,
        json!({"type": "select", "ids": [group, sole], "mode": "replace"}),
    )
    .unwrap();
    let out = run(&mut d, json!({"type": "make_unique"})).unwrap();
    assert_eq!(out["definitions"], json!([]));
    assert_eq!(depth(&d), steps);

    let shared = copies(&mut d, std::slice::from_ref(&sole), 1);
    run(
        &mut d,
        json!({"type": "select", "ids": [shared[0]], "mode": "replace"}),
    )
    .unwrap();
    let out = run(&mut d, json!({"type": "make_unique"})).unwrap();
    assert_eq!(out["definitions"].as_array().unwrap().len(), 1);
}

#[test]
fn make_unique_outcome_definitions_are_batch_referable() {
    let mut d = doc();
    let first = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_component");
    let more = copies(&mut d, std::slice::from_ref(&first), 1);
    let commands: Vec<BatchCommand> = [
        json!({"type": "make_unique", "ids": [more[0]]}),
        json!({"type": "rename_definition", "definition": "$0:d0", "name": "Stool"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
    assert_eq!(instance_count(&d, "Stool"), 1);
    assert_eq!(instance_count(&d, "Chair"), 1);
}

/// Positions of the vertices of the open context, sorted.
fn positions(d: &Document) -> Vec<[f64; 3]> {
    let all = ask(d, json!({"type": "entities", "kinds": ["vertex"]}));
    let mut out: Vec<[f64; 3]> = all["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| serde_json::from_value(v["position"].clone()).unwrap())
        .collect();
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out
}

fn all_ids(d: &Document, kinds: &[&str]) -> Vec<Value> {
    let all = ask(d, json!({"type": "entities", "kinds": kinds}));
    ["edges", "faces"]
        .iter()
        .flat_map(|k| all[*k].as_array().cloned().unwrap_or_default())
        .map(|e| e["id"].clone())
        .collect()
}

#[test]
fn flip_mirrors_geometry_about_its_box_center_and_faces_stay_outward() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [1, 0, 0], "opposite": [3, 2, 0]}),
    )
    .unwrap();
    let face = all_ids(&d, &["face"])[0].clone();
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 1}),
    )
    .unwrap();
    let before = positions(&d);
    let ids = all_ids(&d, &["edge", "face"]);
    let steps = depth(&d);

    let out = run(&mut d, json!({"type": "flip", "ids": ids, "axis": "red"})).unwrap();

    // Box spans x 1..3, so x -> 4 - x.
    let mut expect: Vec<[f64; 3]> = before.iter().map(|p| [4.0 - p[0], p[1], p[2]]).collect();
    expect.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(positions(&d), expect);
    assert!(!out["changed"].as_array().unwrap().is_empty());
    assert_eq!(depth(&d), steps + 1);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo"][0], "flip");
    // Every face still faces away from the box center.
    let all = ask(&d, json!({"type": "entities", "kinds": ["vertex", "face"]}));
    let at = |id: &Value| -> [f64; 3] {
        let v = all["vertices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == *id)
            .unwrap();
        serde_json::from_value(v["position"].clone()).unwrap()
    };
    for f in all["faces"].as_array().unwrap() {
        let n: [f64; 3] = serde_json::from_value(f["normal"].clone()).unwrap();
        let ring: Vec<[f64; 3]> = f["outer"].as_array().unwrap().iter().map(at).collect();
        let c: [f64; 3] =
            std::array::from_fn(|k| ring.iter().map(|p| p[k]).sum::<f64>() / ring.len() as f64);
        let out_dir = [c[0] - 2.0, c[1] - 1.0, c[2] - 0.5];
        let dot: f64 = (0..3).map(|k| n[k] * out_dir[k]).sum();
        assert!(dot > 0.0, "face {} points inward", f["id"]);
    }
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(positions(&d), before);
}

#[test]
fn flip_along_green_and_blue_pick_their_axes() {
    for (axis, k) in [("green", 1), ("blue", 2)] {
        let mut d = doc();
        run(
            &mut d,
            json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 3, 0]}),
        )
        .unwrap();
        let face = all_ids(&d, &["face"])[0].clone();
        run(
            &mut d,
            json!({"type": "push_pull", "face": face, "distance": 1}),
        )
        .unwrap();
        // A lopsided vertex, so a flip is visible on every axis.
        let before = positions(&d);
        let ids = all_ids(&d, &["edge", "face"]);
        run(&mut d, json!({"type": "flip", "ids": ids, "axis": axis})).unwrap();
        let mut expect = before.clone();
        let (lo, hi) = (
            before.iter().map(|p| p[k]).fold(f64::MAX, f64::min),
            before.iter().map(|p| p[k]).fold(f64::MIN, f64::max),
        );
        for p in &mut expect {
            p[k] = lo + hi - p[k];
        }
        expect.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(positions(&d), expect, "{axis}");
    }
}

#[test]
fn flip_of_instances_about_the_selection_center_mirrors_their_transforms() {
    let mut d = doc();
    let a = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_group");
    let b = wrapped_box(&mut d, [4.0, 0.0, 0.0], "make_group");
    run(
        &mut d,
        json!({"type": "select", "ids": [a, b], "mode": "replace"}),
    )
    .unwrap();
    run(&mut d, json!({"type": "flip", "axis": "red"})).unwrap();
    // The union spans x 0..5, so the boxes swap places mirrored.
    let ia = ask(&d, json!({"type": "entity", "id": a}));
    let ib = ask(&d, json!({"type": "entity", "id": b}));
    assert_eq!(ia["bounds"]["min"], json!([4.0, 0.0, 0.0]));
    assert_eq!(ia["bounds"]["max"], json!([5.0, 1.0, 1.0]));
    assert_eq!(ib["bounds"]["min"], json!([0.0, 0.0, 0.0]));
    assert_eq!(ib["bounds"]["max"], json!([1.0, 1.0, 1.0]));
}

#[test]
fn flip_inside_an_open_group_works_in_its_coordinates() {
    let mut d = doc();
    let g = wrapped_box(&mut d, [10.0, 0.0, 0.0], "make_group");
    run(&mut d, json!({"type": "open_instance", "id": g})).unwrap();
    let before = positions(&d);
    let ids = all_ids(&d, &["edge", "face"]);
    run(&mut d, json!({"type": "flip", "ids": ids, "axis": "red"})).unwrap();
    // The group's own geometry spans x 10..11 in the group's coordinates too.
    let expect: Vec<[f64; 3]> = {
        let mut e: Vec<[f64; 3]> = before.iter().map(|p| [21.0 - p[0], p[1], p[2]]).collect();
        e.sort_by(|a, b| a.partial_cmp(b).unwrap());
        e
    };
    assert_eq!(positions(&d), expect);
}

#[test]
fn own_flip_mirrors_a_rotated_instance_about_its_own_center_along_its_own_axis() {
    let mut d = doc();
    let g = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_group");
    run(
        &mut d,
        json!({"type": "rotate", "ids": [g], "center": [0, 0, 0], "axis": [0, 0, 1], "angle": 90}),
    )
    .unwrap();
    // After the turn the box spans x -1..0, y 0..1, and its own x axis points along +y.
    let before = ask(&d, json!({"type": "entity", "id": g}));
    run(
        &mut d,
        json!({"type": "flip", "ids": [g], "axis": "red", "frame": "own"}),
    )
    .unwrap();
    let after = ask(&d, json!({"type": "entity", "id": g}));
    // The box stays where it was (mirrored about its own center) ...
    assert_eq!(after["bounds"], before["bounds"]);
    // ... but its transform is now mirrored.
    assert_ne!(after["transform"], before["transform"]);
    let t: Vec<f64> = serde_json::from_value(after["transform"].clone()).unwrap();
    // Nine matrix entries by column, then the translation.
    let x = glam::DVec3::new(t[0], t[1], t[2]);
    let y = glam::DVec3::new(t[3], t[4], t[5]);
    let z = glam::DVec3::new(t[6], t[7], t[8]);
    assert!(x.cross(y).dot(z) < 0.0);
}

#[test]
fn own_flip_needs_exactly_one_instance() {
    let mut d = doc();
    let a = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_group");
    let b = wrapped_box(&mut d, [4.0, 0.0, 0.0], "make_group");
    run(
        &mut d,
        json!({"type": "select", "ids": [a, b], "mode": "replace"}),
    )
    .unwrap();
    for ids in [json!([a, b]), json!([])] {
        let err = run(
            &mut d,
            json!({"type": "flip", "ids": ids, "axis": "red", "frame": "own"}),
        )
        .unwrap_err();
        assert_eq!(err.0, ErrorKind::InvalidInput);
    }
    // A selected geometry id is not an instance either.
    run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
    let edges = all_ids(&d, &["edge"]);
    let err = run(
        &mut d,
        json!({"type": "flip", "ids": edges, "axis": "red", "frame": "own"}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
}

#[test]
fn flip_twice_about_the_same_axis_restores_a_mirrored_instance() {
    let mut d = doc();
    let g = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_group");
    let start = ask(&d, json!({"type": "entity", "id": g}));
    for _ in 0..2 {
        run(
            &mut d,
            json!({"type": "flip", "ids": [g], "axis": "green", "frame": "own"}),
        )
        .unwrap();
    }
    let end = ask(&d, json!({"type": "entity", "id": g}));
    assert_eq!(end["bounds"], start["bounds"]);
    let a: Vec<f64> = serde_json::from_value(start["transform"].clone()).unwrap();
    let b: Vec<f64> = serde_json::from_value(end["transform"].clone()).unwrap();
    assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-9));
}

#[test]
fn flip_with_nothing_to_flip_is_an_error() {
    let mut d = doc();
    let err = run(&mut d, json!({"type": "flip", "axis": "red"})).unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
}

#[test]
fn definition_references_fit_only_definition_fields() {
    for (index, v) in [
        json!({"type": "rename_definition", "definition": "$0:i0", "name": "X"}),
        json!({"type": "rename_definition", "definition": "$0:d*", "name": "X"}),
        json!({"type": "erase", "ids": ["$0:d0"]}),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(BatchCommand::parse(v, index + 1).is_err());
    }
}
