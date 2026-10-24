//! M17a: faces along the given normal, zero rotation, named groups, material reuse.

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

fn depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

fn first_face(out: &Value) -> Value {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone()
}

fn bounds(d: &Document) -> (Vec<f64>, Vec<f64>) {
    let v = ask(d, json!({"type": "entities", "kinds": ["vertex"]}));
    let pts: Vec<Vec<f64>> = v["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            p["position"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        })
        .collect();
    let lo = (0..3)
        .map(|k| pts.iter().map(|p| p[k]).fold(f64::MAX, f64::min))
        .collect();
    let hi = (0..3)
        .map(|k| pts.iter().map(|p| p[k]).fold(f64::MIN, f64::max))
        .collect();
    (lo, hi)
}

/// Draws a shape with `normal`, pulls it by +1 and checks the solid grew along `normal`.
fn pulls_along(shape: Value, axis: usize, sign: f64) {
    let mut d = doc();
    let out = run(&mut d, shape.clone()).unwrap();
    run(
        &mut d,
        json!({"type": "push_pull", "face": first_face(&out), "distance": 1.0}),
    )
    .unwrap();
    let (lo, hi) = bounds(&d);
    if sign > 0.0 {
        assert!(
            (hi[axis] - 1.0).abs() < 1e-9 && lo[axis].abs() < 1e-9,
            "{shape}"
        );
    } else {
        assert!(
            (lo[axis] + 1.0).abs() < 1e-9 && hi[axis].abs() < 1e-9,
            "{shape}"
        );
    }
}

#[test]
fn shapes_pull_along_the_normal_they_were_given() {
    for (n, axis, sign) in [
        ([0, 0, -1], 2, -1.0),
        ([0, 0, 1], 2, 1.0),
        ([0, -1, 0], 1, -1.0),
        ([0, 1, 0], 1, 1.0),
        ([-1, 0, 0], 0, -1.0),
        ([1, 0, 0], 0, 1.0),
    ] {
        pulls_along(
            json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "normal": n}),
            axis,
            sign,
        );
        pulls_along(
            json!({"type": "add_polygon", "center": [0, 0, 0], "radius": 1.0, "normal": n}),
            axis,
            sign,
        );
        let opposite = [[0, 1, 1], [1, 0, 1], [1, 1, 0]][axis];
        pulls_along(
            json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": opposite, "normal": n}),
            axis,
            sign,
        );
    }
}

#[test]
fn a_rotated_rectangle_faces_along_its_side_cross_width() {
    // (1,0,0) x (0,1,0) = +z.
    pulls_along(
        json!({"type": "add_rotated_rectangle", "corner": [0, 0, 0], "edge_end": [1, 0, 0], "width_point": [0, 1, 0]}),
        2,
        1.0,
    );
    // (1,0,0) x (0,-1,0) = -z.
    pulls_along(
        json!({"type": "add_rotated_rectangle", "corner": [0, 0, 0], "edge_end": [1, 0, 0], "width_point": [0, -1, 0]}),
        2,
        -1.0,
    );
}

#[test]
fn without_a_normal_the_canonical_orientation_stays() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "push_pull", "face": first_face(&out), "distance": 1.0}),
    )
    .unwrap();
    let (lo, hi) = bounds(&d);
    assert!(lo[2].abs() < 1e-9 && (hi[2] - 1.0).abs() < 1e-9);
}

#[test]
fn a_circle_inside_a_face_keeps_the_outer_orientation() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 4, 0]}),
    )
    .unwrap();
    let before = ask(&d, json!({"type": "entities", "kinds": ["face"]}));
    let outer_normal = before["faces"][0]["normal"].clone();
    let outer_id = before["faces"][0]["id"].clone();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [2, 2, 0], "radius": 1.0, "normal": [0, 0, -1]}),
    )
    .unwrap();
    let after = ask(&d, json!({"type": "entities", "kinds": ["face"]}));
    for f in after["faces"].as_array().unwrap() {
        assert_eq!(f["normal"], outer_normal, "{f}");
    }
    assert!(
        after["faces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["id"] == outer_id)
    );
}

#[test]
fn whole_turns_do_nothing_without_copies() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let face = first_face(&out);
    let steps = depth(&d);
    let revision = d.revision();
    for angle in [0.0, 360.0, -720.0] {
        let out = run(
            &mut d,
            json!({"type": "rotate", "ids": [face], "center": [0, 0, 0], "angle": angle}),
        )
        .unwrap();
        assert!(out["changes"]["created"].is_null() || out["changes"]["created"] == json!([]));
    }
    assert_eq!(depth(&d), steps);
    assert_eq!(d.revision(), revision);
    let (kind, _) = run(
        &mut d,
        json!({"type": "rotate", "ids": [face], "center": [0, 0, 0], "angle": 360, "copies": 1}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, _) = run(
        &mut d,
        json!({"type": "rotate", "ids": ["f9999"], "center": [0, 0, 0], "angle": 0}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
}

#[test]
fn scaling_by_one_does_nothing() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let face = first_face(&out);
    let steps = depth(&d);
    let revision = d.revision();
    run(
        &mut d,
        json!({"type": "scale", "ids": [face], "origin": [0, 0, 0], "factors": [1, 1, 1]}),
    )
    .unwrap();
    assert_eq!(depth(&d), steps);
    assert_eq!(d.revision(), revision);
}

#[test]
fn a_group_takes_its_name() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let ids = json!([first_face(&out)]);
    run(
        &mut d,
        json!({"type": "make_group", "ids": ids, "name": "Hull"}),
    )
    .unwrap();
    let e = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    assert_eq!(e["instances"][0]["name"], "Hull");
    let (kind, _) = run(
        &mut d,
        json!({"type": "make_group", "ids": ["i1"], "name": " "}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}

fn material(name: &str, color: [u8; 3], existing: Option<&str>) -> Value {
    let mut v = json!({"type": "add_material", "name": name, "color": color});
    if let Some(e) = existing {
        v["existing"] = json!(e);
    }
    v
}

fn materials(d: &Document) -> Value {
    ask(d, json!({"type": "materials"}))["materials"].clone()
}

#[test]
fn add_material_on_a_taken_name_fails_by_default() {
    let mut d = doc();
    run(&mut d, material("Brick", [200, 0, 0], None)).unwrap();
    let (kind, _) = run(&mut d, material("Brick", [0, 200, 0], None)).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, _) = run(&mut d, material("Brick", [0, 200, 0], Some("fail"))).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}

#[test]
fn add_material_reuse_returns_the_existing_one_without_a_step() {
    let mut d = doc();
    let first = run(&mut d, material("Brick", [200, 0, 0], None)).unwrap();
    let steps = depth(&d);
    let revision = d.revision();
    let again = run(&mut d, material("Brick", [0, 200, 0], Some("reuse"))).unwrap();
    assert_eq!(again["changes"]["materials"], first["changes"]["materials"]);
    assert_eq!(depth(&d), steps);
    assert_eq!(d.revision(), revision);
    assert_eq!(materials(&d).as_array().unwrap().len(), 1);
    assert_eq!(materials(&d)[0]["color"], json!([200, 0, 0]));
    // A new name is added as usual.
    let other = run(&mut d, material("Slate", [9, 9, 9], Some("reuse"))).unwrap();
    assert_ne!(other["changes"]["materials"], first["changes"]["materials"]);
    assert_eq!(depth(&d), steps + 1);
}

#[test]
fn add_material_update_changes_the_existing_one_in_one_step() {
    let mut d = doc();
    let first = run(&mut d, material("Brick", [200, 0, 0], None)).unwrap();
    let steps = depth(&d);
    let again = run(&mut d, material("Brick", [0, 200, 0], Some("update"))).unwrap();
    assert_eq!(again["changes"]["materials"], first["changes"]["materials"]);
    assert_eq!(depth(&d), steps + 1);
    assert_eq!(materials(&d).as_array().unwrap().len(), 1);
    assert_eq!(materials(&d)[0]["color"], json!([0, 200, 0]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[0]["color"], json!([200, 0, 0]));
    // Nothing different: no step, but the id is still returned.
    let same = run(&mut d, material("Brick", [200, 0, 0], Some("update"))).unwrap();
    assert_eq!(same["changes"]["materials"], first["changes"]["materials"]);
    assert_eq!(depth(&d), steps);
}

#[test]
fn existing_needs_a_name() {
    let mut d = doc();
    for e in ["reuse", "update"] {
        let (kind, _) = run(
            &mut d,
            json!({"type": "add_material", "color": [1, 2, 3], "existing": e}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput);
    }
}

#[test]
fn batch_material_references_resolve_in_every_case() {
    for existing in ["fail", "reuse", "update"] {
        let mut d = doc();
        let first = run(&mut d, material("Brick", [200, 0, 0], None)).unwrap();
        let id = first["changes"]["materials"][0].clone();
        let commands = if existing == "fail" {
            vec![
                material("Fresh", [1, 2, 3], Some(existing)),
                json!({"type": "change_material", "material": "$0:m0", "color": [5, 5, 5]}),
            ]
        } else {
            vec![
                material("Brick", [1, 2, 3], Some(existing)),
                json!({"type": "change_material", "material": "$0:m0", "color": [5, 5, 5]}),
            ]
        };
        let commands: Vec<BatchCommand> = commands
            .into_iter()
            .enumerate()
            .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
            .collect();
        let outcomes = d.execute_batch(commands, true).unwrap();
        if existing != "fail" {
            assert_eq!(
                outcomes[0].changes.materials,
                vec![serde_json::from_value(id).unwrap()]
            );
        }
        let by_name = |n: &str| {
            materials(&d)
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["name"] == n)
                .unwrap()
                .clone()
        };
        let target = if existing == "fail" { "Fresh" } else { "Brick" };
        assert_eq!(by_name(target)["color"], json!([5, 5, 5]), "{existing}");
    }
}
