//! Scale, driven through JSON commands.

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

/// Like `run`, but for JSON that may not even parse as a command (NaN factors): a parse
/// failure counts as invalid input, as it does over the API.
fn run_json(d: &mut Document, text: &str) -> Result<Value, ErrorKind> {
    match serde_json::from_str::<Command>(text) {
        Ok(cmd) => d
            .execute(cmd)
            .map(|o| serde_json::to_value(o).unwrap())
            .map_err(|e| e.kind),
        Err(_) => Err(ErrorKind::InvalidInput),
    }
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
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

fn created(out: &Value, prefix: char) -> Vec<Value> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with(prefix))
        .cloned()
        .collect()
}

/// A group of a 1 x 1 x 1 box with its bottom corner at `at`; returns the instance.
fn group_box(d: &mut Document, at: [f64; 3]) -> Value {
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
    let out = run(d, json!({"type": "make_group", "ids": ids})).unwrap();
    created(&out, 'i')[0].clone()
}

fn undo_labels(d: &Document) -> Value {
    ask(d, json!({"type": "history"}))["undo"].clone()
}

#[test]
fn scale_command_scales_geometry_instances_and_guides() {
    let mut d = doc();
    let group = group_box(&mut d, [5.0, 0.0, 0.0]);
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    let rect = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    let guide = created(
        &run(
            &mut d,
            json!({"type": "add_guide_point", "position": [1, 1, 0]}),
        )
        .unwrap(),
        'g',
    )[0]
    .clone();
    let before_positions = positions(&d);
    let before_instance = ask(&d, json!({"type": "entity", "id": group}));
    let before_guide = ask(&d, json!({"type": "entity", "id": guide}));
    let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();

    run(
        &mut d,
        json!({"type": "scale", "ids": [rect, group, guide], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    )
    .unwrap();

    let doubled: Vec<[f64; 3]> = before_positions
        .iter()
        .map(|p| [p[0] * 2.0, p[1], p[2]])
        .collect();
    assert_eq!(positions(&d), doubled);
    let info = ask(&d, json!({"type": "entity", "id": group}));
    let t: Vec<f64> = serde_json::from_value(info["transform"].clone()).unwrap();
    let t0: Vec<f64> = serde_json::from_value(before_instance["transform"].clone()).unwrap();
    assert_eq!(&t[0..3], &[t0[0] * 2.0, t0[1], t0[2]]);
    assert_eq!(&t[3..9], &t0[3..9]);
    assert_eq!(t[9], t0[9] * 2.0);
    assert_eq!(info["bounds"]["min"], json!([10.0, 0.0, 0.0]));
    assert_eq!(info["bounds"]["max"], json!([12.0, 1.0, 1.0]));
    let g = ask(&d, json!({"type": "entity", "id": guide}));
    assert_eq!(g["point"], json!([2.0, 1.0, 0.0]));
    let h = ask(&d, json!({"type": "history"}));
    assert_eq!(
        h["undo_depth"].as_u64().unwrap(),
        depth.as_u64().unwrap() + 1
    );
    assert_eq!(undo_labels(&d)[0], "scale");

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(positions(&d), before_positions);
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": group})),
        before_instance
    );
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": guide})),
        before_guide
    );
}

#[test]
fn scale_refuses_bad_factors() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    let before = positions(&d);
    let bad = |factors: &str| {
        format!(r#"{{"type": "scale", "ids": ["f8"], "origin": [0, 0, 0], "factors": {factors}}}"#)
    };
    for factors in [
        "[0, 1, 1]",
        "[1, 1, 1]",
        "[1e5, 1, 1]",
        "[1e-5, 1, 1]",
        "[-2e4, 1, 1]",
        "[NaN, 1, 1]",
    ] {
        assert_eq!(
            run_json(&mut d, &bad(factors)),
            Err(ErrorKind::InvalidInput),
            "{factors}"
        );
    }
    // Infinity cannot be written in JSON, but a command built in code may carry it.
    assert_eq!(
        d.execute(Command::Scale {
            ids: serde_json::from_value(json!(["f8"])).unwrap(),
            origin: [0.0; 3],
            factors: [f64::INFINITY, 1.0, 1.0],
        })
        .map_err(|e| e.kind)
        .err(),
        Some(ErrorKind::InvalidInput)
    );
    assert_eq!(
        run(
            &mut d,
            json!({"type": "scale", "ids": [], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
        )
        .map_err(|e| e.0),
        Err(ErrorKind::InvalidInput)
    );
    assert_eq!(
        run(
            &mut d,
            json!({"type": "scale", "ids": ["f99"], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
        ),
        Err((ErrorKind::NotFound, vec!["f99".to_string()]))
    );
    assert_eq!(positions(&d), before);
    // A negative factor in range mirrors.
    run(
        &mut d,
        json!({"type": "scale", "ids": ["f8"], "origin": [0, 0, 0], "factors": [-1, 1, 1]}),
    )
    .unwrap();
    assert!(positions(&d).iter().all(|p| (-2.0..=0.0).contains(&p[0])));
}

#[test]
fn scale_a_rotated_group_along_model_axes() {
    let mut d = doc();
    let group = group_box(&mut d, [0.0, 0.0, 0.0]);
    run(
        &mut d,
        json!({"type": "rotate", "ids": [group], "center": [0, 0, 0], "angle": 30}),
    )
    .unwrap();
    let before = ask(&d, json!({"type": "entity", "id": group}))["bounds"].clone();
    run(
        &mut d,
        json!({"type": "scale", "ids": [group], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    )
    .unwrap();
    let after = ask(&d, json!({"type": "entity", "id": group}))["bounds"].clone();
    let x = |b: &Value, k: &str| b[k][0].as_f64().unwrap();
    assert!((x(&after, "max") - 2.0 * x(&before, "max")).abs() < 1e-9);
    assert!((x(&after, "min") - 2.0 * x(&before, "min")).abs() < 1e-9);
    assert_eq!(after["max"][1], before["max"][1]);
}

#[test]
fn an_instance_squeezed_too_flat_is_refused() {
    let mut d = doc();
    let group = group_box(&mut d, [0.0, 0.0, 0.0]);
    run(
        &mut d,
        json!({"type": "scale", "ids": [group], "origin": [0, 0, 0], "factors": [1e-4, 1e-4, 1e-3]}),
    )
    .unwrap();
    let before = ask(&d, json!({"type": "entity", "id": group}));
    assert_eq!(
        run(
            &mut d,
            json!({"type": "scale", "ids": [group], "origin": [0, 0, 0], "factors": [1e-4, 1, 1]}),
        ),
        Err((
            ErrorKind::InvalidInput,
            vec![group.as_str().unwrap().to_string()]
        ))
    );
    assert_eq!(ask(&d, json!({"type": "entity", "id": group})), before);
}
