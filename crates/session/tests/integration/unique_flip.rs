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
    for ids in [Some(json!([a, b])), None] {
        let mut command = json!({"type": "flip", "axis": "red", "frame": "own"});
        if let Some(ids) = ids {
            command["ids"] = ids;
        }
        let err = run(&mut d, command).unwrap_err();
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

#[test]
fn make_unique_of_every_instance_of_a_definition_changes_nothing() {
    let mut d = doc();
    let first = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_component");
    let more = copies(&mut d, std::slice::from_ref(&first), 1);
    let steps = depth(&d);
    let out = run(
        &mut d,
        json!({"type": "make_unique", "ids": [first, more[0]]}),
    )
    .unwrap();
    assert_eq!(out["definitions"], json!([]));
    assert_eq!(depth(&d), steps);
    assert_eq!(instance_count(&d, "Chair"), 2);
}

#[test]
fn own_flip_of_a_sheared_instance_keeps_its_bounds_and_mirrors_its_own_axis() {
    let mut d = doc();
    let g = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_group");
    run(
        &mut d,
        json!({"type": "rotate", "ids": [g], "center": [0, 0, 0], "axis": [0, 0, 1], "angle": 30}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "scale", "ids": [g], "origin": [0, 0, 0], "factors": [3, 1, 1]}),
    )
    .unwrap();
    let before = ask(&d, json!({"type": "entity", "id": g}));
    let t0: Vec<f64> = serde_json::from_value(before["transform"].clone()).unwrap();
    run(
        &mut d,
        json!({"type": "flip", "ids": [g], "axis": "red", "frame": "own"}),
    )
    .unwrap();
    let after = ask(&d, json!({"type": "entity", "id": g}));
    let near = |a: &Value, b: &Value| {
        let (a, b): (Vec<f64>, Vec<f64>) = (
            a.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect(),
            b.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect(),
        );
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-9)
    };
    assert!(near(&after["bounds"]["min"], &before["bounds"]["min"]));
    assert!(near(&after["bounds"]["max"], &before["bounds"]["max"]));
    // Own red column negated, the others as they were.
    let t: Vec<f64> = serde_json::from_value(after["transform"].clone()).unwrap();
    for k in 0..3 {
        assert!((t[k] + t0[k]).abs() < 1e-9, "red column {k}");
    }
    for k in 3..9 {
        assert!((t[k] - t0[k]).abs() < 1e-9, "other columns {k}");
    }
}

#[test]
fn empty_ids_act_on_the_selection_only_when_absent() {
    let mut d = doc();
    let g = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_group");
    let before = ask(&d, json!({"type": "entity", "id": g}));
    // g is selected; a reference that expands to nothing must not reach it.
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_edge", "start": [5, 5, 5], "end": [6, 5, 5]}),
        json!({"type": "flip", "ids": ["$0:i*"], "axis": "red"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
    assert_eq!(ask(&d, json!({"type": "entity", "id": g})), before);
    let steps = depth(&d);
    let out = run(&mut d, json!({"type": "flip", "ids": [], "axis": "red"})).unwrap();
    assert_eq!(out["changed"], json!([]));
    assert_eq!(depth(&d), steps);
    let out = run(&mut d, json!({"type": "make_unique", "ids": []})).unwrap();
    assert_eq!(out["definitions"], json!([]));
    assert_eq!(depth(&d), steps);
    // Absent: the selection.
    run(&mut d, json!({"type": "flip", "axis": "red"})).unwrap();
}

#[test]
fn make_unique_ignores_instances_in_unplaced_definitions() {
    let mut d = doc();
    let first = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_component");
    // A second chair only inside a component that nothing places.
    let more = copies(&mut d, std::slice::from_ref(&first), 1);
    let holder = created(
        &run(
            &mut d,
            json!({"type": "make_component", "ids": [more[0]], "name": "Holder"}),
        )
        .unwrap(),
        'i',
    )[0]
    .clone();
    run(&mut d, json!({"type": "erase", "ids": [holder]})).unwrap();
    assert_eq!(instance_count(&d, "Chair"), 2, "one in the unplaced Holder");
    let out = run(&mut d, json!({"type": "make_unique", "ids": [first]})).unwrap();
    assert_eq!(out["definitions"], json!([]));
}

#[test]
fn flip_of_a_mixed_selection_moves_guides_sections_and_dimensions_too() {
    let mut d = doc();
    let group = wrapped_box(&mut d, [3.0, 3.0, 0.0], "make_group");
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    let face = all_ids(&d, &["face"])[0].clone();
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 1}),
    )
    .unwrap();
    let geometry = all_ids(&d, &["edge", "face"]);
    let guide = created(
        &run(
            &mut d,
            json!({"type": "add_guide_point", "position": [1, 5, 0]}),
        )
        .unwrap(),
        'g',
    )[0]
    .clone();
    let plane = created(
        &run(
            &mut d,
            json!({"type": "add_section_plane", "point": [1, 1, 1], "normal": [1, 0, 0]}),
        )
        .unwrap(),
        's',
    )[0]
    .clone();
    let dim = created(
        &run(
            &mut d,
            json!({"type": "add_dimension", "a": [0, 0, 0], "b": [1, 0, 0], "offset": [0, -1, 0]}),
        )
        .unwrap(),
        'r',
    )[0]
    .clone();
    let mut ids = geometry;
    ids.extend([group.clone(), guide.clone(), plane.clone(), dim.clone()]);
    // The box spans x 0..4 (geometry and the group), so x -> 4 - x.
    run(&mut d, json!({"type": "flip", "ids": ids, "axis": "red"})).unwrap();
    let g = ask(&d, json!({"type": "entity", "id": guide}));
    assert_eq!(g["point"], json!([3.0, 5.0, 0.0]));
    let s = ask(&d, json!({"type": "entity", "id": plane}));
    assert_eq!(s["point"], json!([3.0, 1.0, 1.0]));
    let m = ask(&d, json!({"type": "entity", "id": dim}));
    assert_eq!(m["a"]["point"], json!([4.0, 0.0, 0.0]));
    assert_eq!(m["b"]["point"], json!([3.0, 0.0, 0.0]));
    let i = ask(&d, json!({"type": "entity", "id": group}));
    assert_eq!(i["bounds"]["min"], json!([0.0, 3.0, 0.0]));
    assert_eq!(i["bounds"]["max"], json!([1.0, 4.0, 1.0]));
}

/// Whether every face of the open context points away from the middle of its vertices.
fn faces_point_out(d: &Document) -> bool {
    let all = ask(d, json!({"type": "entities", "kinds": ["vertex", "face"]}));
    let at = |id: &Value| -> [f64; 3] {
        let v = all["vertices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == *id)
            .unwrap();
        serde_json::from_value(v["position"].clone()).unwrap()
    };
    let points = positions(d);
    let middle: [f64; 3] = std::array::from_fn(|k| {
        let lo = points.iter().map(|p| p[k]).fold(f64::MAX, f64::min);
        let hi = points.iter().map(|p| p[k]).fold(f64::MIN, f64::max);
        (lo + hi) / 2.0
    });
    all["faces"].as_array().unwrap().iter().all(|f| {
        let n: [f64; 3] = serde_json::from_value(f["normal"].clone()).unwrap();
        let ring: Vec<[f64; 3]> = f["outer"].as_array().unwrap().iter().map(at).collect();
        let c: [f64; 3] =
            std::array::from_fn(|k| ring.iter().map(|p| p[k]).sum::<f64>() / ring.len() as f64);
        (0..3).map(|k| n[k] * (c[k] - middle[k])).sum::<f64>() > 0.0
    })
}

#[test]
fn faces_keep_facing_out_after_flips_at_every_depth() {
    let mut d = doc();
    let inner = wrapped_box(&mut d, [0.0, 0.0, 0.0], "make_group");
    let outer = created(
        &run(&mut d, json!({"type": "make_group", "ids": [inner]})).unwrap(),
        'i',
    )[0]
    .clone();
    // Flipping the outer instance mirrors its transform; its definitions are untouched.
    run(
        &mut d,
        json!({"type": "flip", "ids": [outer], "axis": "red"}),
    )
    .unwrap();
    let t: Vec<f64> = serde_json::from_value(
        ask(&d, json!({"type": "entity", "id": outer}))["transform"].clone(),
    )
    .unwrap();
    let (x, y, z) = (
        glam::DVec3::new(t[0], t[1], t[2]),
        glam::DVec3::new(t[3], t[4], t[5]),
        glam::DVec3::new(t[6], t[7], t[8]),
    );
    assert!(x.cross(y).dot(z) < 0.0);
    run(&mut d, json!({"type": "open_instance", "id": outer})).unwrap();
    let inner_inside =
        ask(&d, json!({"type": "entities", "kinds": ["instance"]}))["instances"][0]["id"].clone();
    // Flip the inner group's instance inside the outer one, then the inner geometry inside it.
    run(
        &mut d,
        json!({"type": "flip", "ids": [inner_inside], "axis": "green"}),
    )
    .unwrap();
    run(&mut d, json!({"type": "open_instance", "id": inner_inside})).unwrap();
    assert!(faces_point_out(&d));
    let ids = all_ids(&d, &["edge", "face"]);
    run(&mut d, json!({"type": "flip", "ids": ids, "axis": "blue"})).unwrap();
    assert!(faces_point_out(&d));
}
