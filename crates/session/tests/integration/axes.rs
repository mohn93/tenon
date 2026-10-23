//! Model axes and context axes, driven through JSON commands.

use std::path::PathBuf;

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

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-axes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

fn vec3(v: &Value) -> [f64; 3] {
    serde_json::from_value(v.clone()).unwrap()
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9)
}

fn axes(d: &Document) -> Value {
    ask(d, json!({"type": "camera"}))["axes"].clone()
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

/// A 1 x 1 x 1 box at `at`, made a component "Crate"; returns its instance.
fn crate_at(d: &mut Document, at: [f64; 3]) -> Value {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": at, "opposite": [at[0] + 1.0, at[1] + 1.0, at[2]]}),
    )
    .unwrap();
    let face = created(&out, 'f')[0].clone();
    run(d, json!({"type": "push_pull", "face": face, "distance": 1})).unwrap();
    let ids = loose(d);
    let out = run(
        d,
        json!({"type": "make_component", "ids": ids, "name": "Crate"}),
    )
    .unwrap();
    created(&out, 'i')[0].clone()
}

/// Every edge and face of the open context.
fn loose(d: &Document) -> Vec<Value> {
    let all = ask(d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    ["edges", "faces"]
        .iter()
        .flat_map(|k| all[*k].as_array().cloned().unwrap_or_default())
        .map(|e| e["id"].clone())
        .collect()
}

fn bounds(d: &Document, id: &Value) -> Value {
    ask(d, json!({"type": "entity", "id": id}))["bounds"].clone()
}

fn positions(d: &Document) -> Vec<[f64; 3]> {
    let all = ask(d, json!({"type": "entities", "kinds": ["vertex"]}));
    let mut out: Vec<[f64; 3]> = all["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| vec3(&v["position"]))
        .collect();
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out
}

#[test]
fn the_axes_start_at_the_world_axes() {
    let d = doc();
    let a = axes(&d);
    assert_eq!(a["origin"], json!([0.0, 0.0, 0.0]));
    assert_eq!(a["x"], json!([1.0, 0.0, 0.0]));
    assert_eq!(a["y"], json!([0.0, 1.0, 0.0]));
    assert_eq!(a["z"], json!([0.0, 0.0, 1.0]));
}

#[test]
fn set_axes_orthonormalizes_and_is_one_undo_step() {
    let mut d = doc();
    let steps = depth(&d);
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [1, 2, 3], "x_axis": [2, 0, 0], "y_axis": [1, 3, 0]}),
    )
    .unwrap();
    let a = axes(&d);
    assert_eq!(vec3(&a["origin"]), [1.0, 2.0, 3.0]);
    assert!(near(vec3(&a["x"]), [1.0, 0.0, 0.0]));
    assert!(near(vec3(&a["y"]), [0.0, 1.0, 0.0]));
    assert!(near(vec3(&a["z"]), [0.0, 0.0, 1.0]));
    assert_eq!(depth(&d), steps + 1);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo"][0], "set_axes");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(axes(&d)["origin"], json!([0.0, 0.0, 0.0]));
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(vec3(&axes(&d)["origin"]), [1.0, 2.0, 3.0]);
}

#[test]
fn set_axes_refuses_zero_and_parallel_vectors() {
    let mut d = doc();
    for (x, y) in [
        ([0, 0, 0], [0, 1, 0]),
        ([1, 0, 0], [0, 0, 0]),
        ([1, 0, 0], [-2, 0, 0]),
        ([1, 0, 0], [3, 0, 0]),
    ] {
        let err = run(
            &mut d,
            json!({"type": "set_axes", "origin": [0, 0, 0], "x_axis": x, "y_axis": y}),
        )
        .unwrap_err();
        assert_eq!(err.0, ErrorKind::InvalidInput, "{x:?} {y:?}");
    }
    assert_eq!(depth(&d), 0);
}

#[test]
fn reset_axes_restores_the_world_axes_and_does_nothing_when_there() {
    let mut d = doc();
    run(&mut d, json!({"type": "reset_axes"})).unwrap();
    assert_eq!(depth(&d), 0);
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [1, 0, 0], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "reset_axes"})).unwrap();
    assert_eq!(axes(&d)["x"], json!([1.0, 0.0, 0.0]));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo"][0], "reset_axes");
    assert_eq!(depth(&d), 2);
}

#[test]
fn moved_axes_are_saved_in_schema_15_and_older_files_are_unchanged() {
    let mut d = doc();
    crate_at(&mut d, [0.0, 0.0, 0.0]);
    let plain = scratch("plain.skr");
    run(
        &mut d,
        json!({"type": "save", "path": plain.to_str().unwrap()}),
    )
    .unwrap();
    let version = |path: &PathBuf| -> Value {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let doc: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
        doc
    };
    let before = version(&plain);
    assert_eq!(
        before["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
    assert!(before["model"].get("axes").is_none());

    run(
        &mut d,
        json!({"type": "set_axes", "origin": [1, 2, 3], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}),
    )
    .unwrap();
    let moved = scratch("moved.skr");
    run(
        &mut d,
        json!({"type": "save", "path": moved.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(version(&moved)["schema_version"], 15);
    assert_eq!(session::file::SCHEMA_VERSION, 15);

    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": moved.to_str().unwrap()}),
    )
    .unwrap();
    let a = axes(&e);
    assert_eq!(vec3(&a["origin"]), [1.0, 2.0, 3.0]);
    assert!(near(vec3(&a["x"]), [0.0, 1.0, 0.0]));
    assert!(near(vec3(&a["y"]), [-1.0, 0.0, 0.0]));
    run(
        &mut e,
        json!({"type": "open", "path": plain.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(axes(&e)["x"], json!([1.0, 0.0, 0.0]));
}

#[test]
fn context_axes_move_the_definition_and_nothing_moves_in_the_world() {
    let mut d = doc();
    let a = crate_at(&mut d, [10.0, 0.0, 0.0]);
    let out = run(
        &mut d,
        json!({"type": "copy", "ids": [a], "offset": [0, 5, 0]}),
    )
    .unwrap();
    let b = created(&out, 'i')[0].clone();
    let (ba, bb) = (bounds(&d, &a), bounds(&d, &b));
    run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
    let context = ask(&d, json!({"type": "context"}));
    let local_before = positions(&d);
    let steps = depth(&d);

    // The crate's own axes move to its corner, turned a quarter about z.
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [10, 0, 0], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}),
    )
    .unwrap();
    assert_eq!(depth(&d), steps + 1);
    assert_eq!(ask(&d, json!({"type": "context"})), context);
    // Local vertices moved by A^-1: (x, y) -> (y, 10 - x).
    let mut expect: Vec<[f64; 3]> = local_before
        .iter()
        .map(|p| [p[1], 10.0 - p[0], p[2]])
        .collect();
    expect.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let moved = positions(&d);
    assert!(
        moved.iter().zip(&expect).all(|(p, q)| near(*p, *q)),
        "{moved:?} vs {expect:?}"
    );
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    for (id, before) in [(&a, &ba), (&b, &bb)] {
        let now = bounds(&d, id);
        for key in ["min", "max"] {
            assert!(near(vec3(&now[key]), vec3(&before[key])), "{id} {key}");
        }
    }
    // The drawing axes of the root are untouched.
    assert_eq!(axes(&d)["origin"], json!([0.0, 0.0, 0.0]));

    // Exact undo.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(bounds(&d, &a), ba);
    run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
    assert_eq!(positions(&d), local_before);
}

#[test]
fn context_axes_keep_nested_instances_where_they_are_even_when_placed_twice() {
    let mut d = doc();
    let inner = crate_at(&mut d, [0.0, 0.0, 0.0]);
    let out = run(
        &mut d,
        json!({"type": "copy", "ids": [inner], "offset": [3, 0, 0]}),
    )
    .unwrap();
    let inner2 = created(&out, 'i')[0].clone();
    // Wrap both crates in a component placed twice.
    let out = run(
        &mut d,
        json!({"type": "make_component", "ids": [inner, inner2], "name": "Pair"}),
    )
    .unwrap();
    let pair = created(&out, 'i')[0].clone();
    let out = run(
        &mut d,
        json!({"type": "copy", "ids": [pair], "offset": [0, 10, 0]}),
    )
    .unwrap();
    let pair2 = created(&out, 'i')[0].clone();
    let before = (bounds(&d, &pair), bounds(&d, &pair2));
    // Open the pair, then one crate in it, and move that crate's axes.
    run(&mut d, json!({"type": "open_instance", "id": pair})).unwrap();
    let crates = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    let crate_id = crates["instances"][0]["id"].clone();
    let crate_bounds = bounds(&d, &crate_id);
    run(&mut d, json!({"type": "open_instance", "id": crate_id})).unwrap();
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [0.5, 0.5, 0], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    // The crate still fills the same place in the pair (quarter turns keep boxes exact).
    assert_eq!(bounds(&d, &crate_id)["min"][2], crate_bounds["min"][2]);
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    for (id, b) in [(&pair, &before.0), (&pair2, &before.1)] {
        let now = bounds(&d, id);
        for key in ["min", "max"] {
            assert!(near(vec3(&now[key]), vec3(&b[key])), "{id} {key}");
        }
    }
}

#[test]
fn reset_axes_inside_a_context_does_nothing() {
    let mut d = doc();
    let a = crate_at(&mut d, [0.0, 0.0, 0.0]);
    run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
    let steps = depth(&d);
    run(&mut d, json!({"type": "reset_axes"})).unwrap();
    assert_eq!(depth(&d), steps);
}

#[test]
fn flip_follows_the_model_axes() {
    let mut d = doc();
    // A box x 0..2, y 0..4; the red axis is turned to point along world y.
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 4, 0]}),
    )
    .unwrap();
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 1}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [0, 0, 0], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}),
    )
    .unwrap();
    let before = positions(&d);
    let ids = loose(&d);
    run(&mut d, json!({"type": "flip", "ids": ids, "axis": "red"})).unwrap();
    let mut expect: Vec<[f64; 3]> = before.iter().map(|p| [p[0], 4.0 - p[1], p[2]]).collect();
    expect.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let now = positions(&d);
    assert!(
        now.iter().zip(&expect).all(|(p, q)| near(*p, *q)),
        "{now:?}"
    );
}
