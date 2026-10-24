//! Rotate, driven through JSON commands.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, MAX_COPIES, Query};

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

/// Vertices, edges, faces and instances in the open context.
fn counts(d: &Document) -> [u64; 4] {
    let s = ask(d, json!({"type": "stats"}));
    ["vertices", "edges", "faces", "instances"].map(|k| s[k].as_u64().unwrap())
}

/// A 2 x 2 square on an empty model: vertices v0 v1 v3 v5, edges e2 e4 e6 e7, face f8.
fn square(d: &mut Document) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
}

/// A 2 x 2 x 1 box on an empty model; returns every edge and face id.
fn cube(d: &mut Document) -> Vec<Value> {
    square(d);
    run(
        d,
        json!({"type": "push_pull", "face": "f8", "distance": 1.0}),
    )
    .unwrap();
    let all = ask(d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect()
}

fn positions(d: &Document) -> Vec<[f64; 3]> {
    let all = ask(d, json!({"type": "entities", "kinds": ["vertex"]}));
    all["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| serde_json::from_value(v["position"].clone()).unwrap())
        .collect()
}

/// The id and normal of every face.
fn normals(d: &Document) -> Vec<(String, [f64; 3])> {
    let all = ask(d, json!({"type": "entities", "kinds": ["face"]}));
    all["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let id = f["id"].as_str().unwrap().to_string();
            (id, serde_json::from_value(f["normal"].clone()).unwrap())
        })
        .collect()
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9)
}

#[test]
fn a_quarter_turn_of_a_box_is_exact_and_one_undo_step() {
    let mut d = doc();
    let ids = cube(&mut d);
    let before = positions(&d);
    let out = run(
        &mut d,
        json!({"type": "rotate", "ids": ids, "center": [0, 0, 0], "angle": 90}),
    )
    .unwrap();
    // The two corners on the axis stay; the four sides turn, the top and bottom do not.
    assert_eq!(out["changes"]["modified"].as_array().unwrap().len(), 6 + 4);
    for p in positions(&d) {
        assert!(
            p[0] >= -2.0 && p[0] <= 0.0 && p[1] >= 0.0 && p[1] <= 2.0,
            "{p:?}"
        );
        assert!(p.iter().all(|c| c.fract() == 0.0), "exact: {p:?}");
    }
    // Faces turn with the box: the one that faced +X now faces +Y.
    let turned: Vec<[f64; 3]> = normals(&d).into_iter().map(|(_, n)| n).collect();
    assert!(turned.contains(&[0.0, 1.0, 0.0]) && turned.contains(&[-1.0, 0.0, 0.0]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(positions(&d), before);
}

#[test]
fn a_hinged_lid_tilts_and_stretches_the_sides() {
    let mut d = doc();
    cube(&mut d);
    let (top, _) = normals(&d)
        .into_iter()
        .find(|(_, n)| *n == [0.0, 0.0, 1.0])
        .unwrap();
    run(
        &mut d,
        json!({"type": "rotate", "ids": [top], "center": [0, 0, 1], "axis": [1, 0, 0], "angle": 30}),
    )
    .unwrap();
    let n = normals(&d)
        .into_iter()
        .find(|(id, _)| *id == top)
        .unwrap()
        .1;
    let (s, c) = 30f64.to_radians().sin_cos();
    assert!(near(n, [0.0, -s, c]), "{n:?}");
    assert!(
        positions(&d)
            .into_iter()
            .any(|p| near(p, [2.0, 2.0 * c, 1.0 + 2.0 * s]))
    );
    assert_eq!(counts(&d), [8, 12, 6, 0]);
}

#[test]
fn twisting_the_top_of_a_box_is_refused_naming_a_bent_side() {
    let mut d = doc();
    cube(&mut d);
    let before = positions(&d);
    let (top, _) = normals(&d)
        .into_iter()
        .find(|(_, n)| *n == [0.0, 0.0, 1.0])
        .unwrap();
    let (kind, ids) = run(
        &mut d,
        json!({"type": "rotate", "ids": [top], "center": [1, 1, 1], "angle": 10}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(ids.len(), 1);
    assert!(ids[0].starts_with('f') && ids[0] != top, "{ids:?}");
    assert_eq!(positions(&d), before);
}

#[test]
fn turning_a_corner_within_its_face_reshapes_it() {
    let mut d = doc();
    square(&mut d);
    run(
        &mut d,
        json!({"type": "rotate", "ids": ["v3"], "center": [0, 0, 0], "angle": -20}),
    )
    .unwrap();
    let v = ask(&d, json!({"type": "entity", "id": "v3"}));
    let p: [f64; 3] = serde_json::from_value(v["position"].clone()).unwrap();
    let (s, c) = 20f64.to_radians().sin_cos();
    assert!(near(p, [2.0 * (c + s), 2.0 * (c - s), 0.0]), "{p:?}");
    assert_eq!(counts(&d), [4, 4, 1, 0]);
}

#[test]
fn rotating_about_the_entity_itself_changes_nothing() {
    let mut d = doc();
    square(&mut d);
    let out = run(
        &mut d,
        json!({"type": "rotate", "ids": ["e2"], "center": [0, 0, 0], "axis": [1, 0, 0], "angle": 90}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"], json!([]));
    assert_eq!(d.revision(), 1);
    assert_eq!(
        run(&mut d, json!({"type": "undo"})).unwrap()["changes"]["deleted"]
            .as_array()
            .unwrap()
            .len(),
        9,
        "the only undo step left is the square's"
    );
}

#[test]
fn a_rotated_instance_turns_its_transform_and_bounds() {
    let mut d = doc();
    let ids = cube(&mut d);
    let out = run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('i'))
        .unwrap()
        .clone();
    run(
        &mut d,
        json!({"type": "rotate", "ids": [group], "center": [0, 0, 0], "angle": 90}),
    )
    .unwrap();
    let info = ask(&d, json!({"type": "entity", "id": group}));
    assert_eq!(
        info["transform"],
        json!([0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0])
    );
    assert_eq!(info["bounds"]["min"], json!([-2.0, 0.0, 0.0]));
    assert_eq!(info["bounds"]["max"], json!([0.0, 2.0, 1.0]));
    // Inside, the geometry keeps its own coordinates.
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    assert!(positions(&d).contains(&[2.0, 2.0, 1.0]));
}

#[test]
fn rotated_copies_make_a_radial_array_in_one_undo_step() {
    let mut d = doc();
    square(&mut d);
    let out = run(
        &mut d,
        json!({"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 90, "copies": 3}),
    )
    .unwrap();
    assert!(!out["changes"]["created"].as_array().unwrap().is_empty());
    assert_eq!(counts(&d), [9, 12, 4, 0]);
    assert!(normals(&d).iter().all(|(_, n)| *n == [0.0, 0.0, 1.0]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(counts(&d), [4, 4, 1, 0]);
}

#[test]
fn rotated_copies_of_a_component_place_the_same_definition() {
    let mut d = doc();
    square(&mut d);
    let out = run(
        &mut d,
        json!({"type": "make_component", "ids": ["f8"], "name": "Tile"}),
    )
    .unwrap();
    let tile = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('i'))
        .unwrap()
        .clone();
    run(
        &mut d,
        json!({"type": "rotate", "ids": [tile], "center": [0, 0, 0], "angle": 60, "copies": 5}),
    )
    .unwrap();
    assert_eq!(counts(&d)[3], 6);
    let defs = ask(&d, json!({"type": "definitions"}));
    assert_eq!(defs["definitions"][0]["instances"], 6);
}

#[test]
fn bad_rotations_are_refused() {
    let mut d = doc();
    square(&mut d);
    let rot = |extra: Value| {
        let mut c = json!({"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 45});
        for (k, v) in extra.as_object().unwrap() {
            c[k] = v.clone();
        }
        c
    };
    for bad in [
        rot(json!({"ids": []})),
        rot(json!({"axis": [0, 0, 0]})),
        rot(json!({"angle": 360, "copies": 1})),
        rot(json!({"angle": -720, "copies": 1})),
        rot(json!({"angle": 0, "copies": 1})),
        rot(json!({"angle": -1e-20, "copies": 1})),
        rot(json!({"angle": 1e-15, "copies": 1})),
        rot(json!({"angle": 360.0 - 1e-12, "copies": 1})),
        rot(json!({"angle": -360.0 + 1e-12, "copies": 1})),
        rot(json!({"center": [2e6, 0, 0]})),
        rot(json!({"center": [1e6, 0, 0], "angle": 180})),
        rot(json!({"copies": 0})),
        rot(json!({"copies": MAX_COPIES + 1})),
        rot(json!({"ids": ["v0"], "copies": 1})),
    ] {
        let (kind, _) = run(&mut d, bad.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{bad}");
    }
    let (kind, ids) = run(&mut d, rot(json!({"ids": ["f8", "e99"]}))).unwrap_err();
    assert_eq!((kind, ids), (ErrorKind::NotFound, vec!["e99".to_string()]));
    assert_eq!(counts(&d), [4, 4, 1, 0]);
    assert_eq!(d.revision(), 1);
}

#[test]
fn rotations_run_in_atomic_batches() {
    let mut d = doc();
    square(&mut d);
    let cmds: Vec<Command> = [
        json!({"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 90}),
        json!({"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 90}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).unwrap())
    .collect();
    d.execute_batch(cmds, true).unwrap();
    for p in positions(&d) {
        assert!(p[0] <= 0.0 && p[1] <= 0.0, "{p:?}");
    }
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(positions(&d).contains(&[2.0, 2.0, 0.0]));
}

#[test]
fn the_axis_is_a_direction_of_any_length() {
    let turn = |axis: Option<Value>| {
        let mut d = doc();
        square(&mut d);
        let mut c = json!({"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 30});
        if let Some(a) = axis {
            c["axis"] = a;
        }
        run(&mut d, c).unwrap();
        positions(&d)
    };
    let z = turn(None);
    assert_eq!(turn(Some(json!([0, 0, 2e6]))), z);
    assert_eq!(turn(Some(json!([0, 0, 1e-200]))), z);
}

#[test]
fn vertices_on_the_axis_stay_put_at_any_angle() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [1, 1, 0], "opposite": [1, 3, 2]}),
    )
    .unwrap();
    let before = positions(&d);
    let out = run(
        &mut d,
        json!({"type": "rotate", "ids": ["f8"], "center": [1, 1, 0], "angle": 60}),
    )
    .unwrap();
    // Only the two vertices off the axis move.
    assert_eq!(out["changes"]["modified"].as_array().unwrap().len(), 3);
    let after = positions(&d);
    for b in before.iter().filter(|p| p[0] == 1.0 && p[1] == 1.0) {
        assert!(after.contains(b), "{b:?} drifted");
    }
}

#[test]
fn an_edge_on_the_axis_rotates_to_nothing_about_any_center() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [2, 0, 0]}),
    )
    .unwrap();
    let edge = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"][0]["id"].clone();
    let out = run(
        &mut d,
        json!({"type": "rotate", "ids": [edge], "center": [0.3, 0, 0], "axis": [1, 0, 0], "angle": 60}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"], json!([]));
    assert_eq!(d.revision(), 1);
}

#[test]
fn copies_that_complete_a_turn_are_refused() {
    let mut d = doc();
    square(&mut d);
    let rot = |n: u32| json!({"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 90, "copies": n});
    let (kind, _) = run(&mut d, rot(4)).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(counts(&d), [4, 4, 1, 0]);
    run(&mut d, rot(3)).unwrap();
}

/// The box grouped and turned 30 degrees about Y through the origin; returns the instance.
fn turned_group(d: &mut Document) -> Value {
    let ids = cube(d);
    let out = run(d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('i'))
        .unwrap()
        .clone();
    run(
        d,
        json!({"type": "rotate", "ids": [group], "center": [0, 0, 0], "axis": [0, 1, 0], "angle": 30}),
    )
    .unwrap();
    group
}

#[test]
fn exploding_a_turned_group_puts_its_geometry_where_it_was_shown() {
    let mut d = doc();
    let group = turned_group(&mut d);
    run(&mut d, json!({"type": "explode", "ids": [group]})).unwrap();
    assert_eq!(counts(&d), [8, 12, 6, 0]);
    let (s, c) = 30f64.to_radians().sin_cos();
    // Turning about +Y carries +Z toward +X: the top corner (0, 0, 1) goes to (sin, 0, cos).
    assert!(positions(&d).into_iter().any(|p| near(p, [s, 0.0, c])));
    assert!(normals(&d).into_iter().any(|(_, n)| near(n, [s, 0.0, c])));
}

#[test]
fn a_turned_group_is_saved_and_opened_turned() {
    let mut d = doc();
    let group = turned_group(&mut d);
    let before = ask(&d, json!({"type": "entity", "id": group}));
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-rotate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("turned.skr");
    let path = path.to_str().unwrap();
    run(&mut d, json!({"type": "save", "path": path})).unwrap();
    let mut opened = doc();
    run(&mut opened, json!({"type": "open", "path": path})).unwrap();
    assert_eq!(ask(&opened, json!({"type": "entity", "id": group})), before);
}
