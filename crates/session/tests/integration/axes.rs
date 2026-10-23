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
    assert_eq!(session::file::AXES_SCHEMA_VERSION, 15);

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

#[test]
fn a_context_frame_that_throws_an_instance_past_the_limit_is_refused() {
    let mut d = doc();
    let a = crate_at(&mut d, [0.0, 0.0, 0.0]);
    run(
        &mut d,
        json!({"type": "move", "ids": [a], "offset": [900000, 0, 0]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
    let steps = depth(&d);
    let before = positions(&d);
    let err = run(
        &mut d,
        json!({"type": "set_axes", "origin": [950000, 0, 0], "x_axis": [1, 0, 0], "y_axis": [0, 1, 0]}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
    assert_eq!(depth(&d), steps);
    assert_eq!(positions(&d), before);
    // The same frame is fine where the instance stays in range.
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [50000, 0, 0], "x_axis": [1, 0, 0], "y_axis": [0, 1, 0]}),
    )
    .unwrap();
}

#[test]
fn a_context_frame_that_throws_nested_content_past_the_limit_is_refused() {
    let mut d = doc();
    let inner = crate_at(&mut d, [0.0, 0.0, 0.0]);
    let out = run(
        &mut d,
        json!({"type": "make_component", "ids": [inner], "name": "Holder"}),
    )
    .unwrap();
    let holder = created(&out, 'i')[0].clone();
    run(&mut d, json!({"type": "open_instance", "id": holder})).unwrap();
    // A turned frame far away: the nested crate's box corners land beyond the limit.
    let err = run(
        &mut d,
        json!({"type": "set_axes", "origin": [999999, 999999, 0], "x_axis": [1, 1, 0], "y_axis": [-1, 1, 0]}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
}

#[test]
fn zero_axis_vectors_say_so() {
    let mut d = doc();
    for (x, y) in [
        (json!([0, 0, 0]), json!([0, 1, 0])),
        (json!([1e300, 1e300, 0]), json!([0, 1, 0])),
    ] {
        let cmd: Command = serde_json::from_value(
            json!({"type": "set_axes", "origin": [0, 0, 0], "x_axis": x, "y_axis": y}),
        )
        .unwrap();
        let e = d.execute(cmd).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
        assert!(e.message.contains("axis"), "{}", e.message);
        assert!(!e.message.contains("endpoints"), "{}", e.message);
    }
}

#[test]
fn a_file_with_axes_beyond_the_limit_or_not_a_frame_is_refused() {
    let mut d = doc();
    crate_at(&mut d, [0.0, 0.0, 0.0]);
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [1, 2, 3], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}),
    )
    .unwrap();
    let path = scratch("limit.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let edit = |f: &dyn Fn(&mut Value)| {
        let mut document: Value = {
            let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
            serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
        };
        f(&mut document);
        let out = scratch("edited.skr");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
        use std::io::Write;
        zip.start_file("document.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&document).unwrap())
            .unwrap();
        zip.finish().unwrap();
        out
    };
    let opens = |p: &PathBuf| {
        let mut e = doc();
        run(&mut e, json!({"type": "open", "path": p.to_str().unwrap()})).is_ok()
    };
    assert!(opens(&path));
    assert!(!opens(&edit(
        &|doc| doc["model"]["axes"]["origin"] = json!([2000000.0, 0.0, 0.0])
    )));
    assert!(!opens(&edit(
        &|doc| doc["model"]["axes"]["x"] = json!([1.0, 1.0, 0.0])
    )));
}

/// `p` placed by an instance's `transform` (nine matrix entries by column, then the translation).
fn to_world(transform: &Value, p: [f64; 3]) -> [f64; 3] {
    let t: Vec<f64> = serde_json::from_value(transform.clone()).unwrap();
    std::array::from_fn(|k| t[k] * p[0] + t[3 + k] * p[1] + t[6 + k] * p[2] + t[9 + k])
}

#[test]
fn context_axes_keep_guides_dimensions_notes_and_section_planes_in_place() {
    let mut d = doc();
    let a = crate_at(&mut d, [10.0, 0.0, 0.0]);
    run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
    let guide = created(
        &run(
            &mut d,
            json!({"type": "add_guide_point", "position": [10.5, 0.5, 0.5]}),
        )
        .unwrap(),
        'g',
    )[0]
    .clone();
    let plane = created(
        &run(
            &mut d,
            json!({"type": "add_section_plane", "point": [10.5, 0.25, 0.5], "normal": [1, 0, 0]}),
        )
        .unwrap(),
        's',
    )[0]
    .clone();
    let dim = created(
        &run(
            &mut d,
            json!({"type": "add_dimension", "a": [10, 0, 0], "b": [11, 0, 0], "offset": [0, -1, 0]}),
        )
        .unwrap(),
        'r',
    )[0]
    .clone();
    let note = created(
        &run(
            &mut d,
            json!({"type": "add_note", "anchor": [10, 1, 0], "position": [10, 2, 1], "text": "Door"}),
        )
        .unwrap(),
        'n',
    )[0]
    .clone();
    let entity = |d: &Document, id: &Value| ask(d, json!({"type": "entity", "id": id}));
    let points = |d: &Document| -> Vec<[f64; 3]> {
        vec![
            vec3(&entity(d, &guide)["point"]),
            vec3(&entity(d, &plane)["point"]),
            vec3(&entity(d, &dim)["a"]["point"]),
            vec3(&entity(d, &dim)["b"]["point"]),
            vec3(&entity(d, &note)["anchor"]["point"]),
            vec3(&entity(d, &note)["position"]),
        ]
    };
    let before = points(&d);
    let normal = vec3(&entity(&d, &plane)["normal"]);
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [10, 0, 0], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}),
    )
    .unwrap();
    let after = points(&d);
    // The frame's inverse takes (x, y, z) to (y, 10 - x, z).
    for (b, a) in before.iter().zip(&after) {
        assert!(near(*a, [b[1], 10.0 - b[0], b[2]]), "{b:?} -> {a:?}");
    }
    // The plane's normal turned with the frame: x became local -y.
    assert!(
        near(vec3(&entity(&d, &plane)["normal"]), [0.0, -1.0, 0.0]),
        "{normal:?}"
    );
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let transform = ask(&d, json!({"type": "entity", "id": a}))["transform"].clone();
    for (b, a) in before.iter().zip(&after) {
        assert!(near(to_world(&transform, *a), *b), "world {b:?}");
    }
}

/// The camera query's direction from the target to the eye and its up.
fn look(d: &Document) -> ([f64; 3], [f64; 3]) {
    let c = ask(d, json!({"type": "camera"}));
    let (eye, target) = (vec3(&c["eye"]), vec3(&c["target"]));
    let from: Vec<f64> = (0..3).map(|k| eye[k] - target[k]).collect();
    let len = from.iter().map(|x| x * x).sum::<f64>().sqrt();
    let dir: [f64; 3] = std::array::from_fn(|k| from[k] / len);
    (dir, vec3(&c["up"]))
}

fn standard_view(d: &mut Document, view: &str) {
    run(d, json!({"type": "standard_view", "view": view})).unwrap();
}

#[test]
fn standard_views_follow_the_model_axes() {
    let mut d = doc();
    // Red along world y, green along world -x, blue still up.
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [5, 5, 0], "x_axis": [0, 1, 0], "y_axis": [-1, 0, 0]}),
    )
    .unwrap();
    standard_view(&mut d, "top");
    let (dir, up) = look(&d);
    assert!(near(dir, [0.0, 0.0, 1.0]), "{dir:?}");
    assert!(near(up, [-1.0, 0.0, 0.0]), "top has green up: {up:?}");
    // Front looks along +green: from the -green side, which is world +x.
    standard_view(&mut d, "front");
    let (dir, up) = look(&d);
    assert!(near(dir, [1.0, 0.0, 0.0]), "{dir:?}");
    assert!(near(up, [0.0, 0.0, 1.0]), "{up:?}");
    standard_view(&mut d, "right");
    assert!(near(look(&d).0, [0.0, 1.0, 0.0]));

    // A blue axis lying along world y: top looks from there, front looks from -green.
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [0, 0, 0], "x_axis": [0, 0, 1], "y_axis": [1, 0, 0]}),
    )
    .unwrap();
    standard_view(&mut d, "top");
    let (dir, up) = look(&d);
    assert!(near(dir, [0.0, 1.0, 0.0]), "{dir:?}");
    assert!(near(up, [1.0, 0.0, 0.0]), "{up:?}");
    standard_view(&mut d, "front");
    let (dir, up) = look(&d);
    assert!(near(dir, [-1.0, 0.0, 0.0]), "{dir:?}");
    assert!(near(up, [0.0, 1.0, 0.0]), "{up:?}");

    // World axes again: the views are the world's.
    run(&mut d, json!({"type": "reset_axes"})).unwrap();
    standard_view(&mut d, "front");
    let (dir, up) = look(&d);
    assert!(near(dir, [0.0, -1.0, 0.0]) && near(up, [0.0, 0.0, 1.0]));
    standard_view(&mut d, "top");
    assert!(near(look(&d).1, [0.0, 1.0, 0.0]));
}

#[test]
fn standard_views_inside_a_context_follow_the_context_axes() {
    let mut d = doc();
    let a = crate_at(&mut d, [0.0, 0.0, 0.0]);
    run(
        &mut d,
        json!({"type": "rotate", "ids": [a], "center": [0, 0, 0], "axis": [0, 0, 1], "angle": 90}),
    )
    .unwrap();
    // The model axes are moved too; inside the crate they do not count.
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [0, 0, 0], "x_axis": [0, 0, 1], "y_axis": [1, 0, 0]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
    standard_view(&mut d, "top");
    let (dir, up) = look(&d);
    assert!(near(dir, [0.0, 0.0, 1.0]), "{dir:?}");
    // The crate's local y points along world -x.
    assert!(near(up, [-1.0, 0.0, 0.0]), "{up:?}");
    standard_view(&mut d, "right");
    // Its local x points along world y.
    assert!(near(look(&d).0, [0.0, 1.0, 0.0]));
}

#[test]
fn rectangles_on_axis_planes_follow_the_drawing_axes() {
    let mut d = doc();
    let s = std::f64::consts::FRAC_1_SQRT_2;
    run(
        &mut d,
        json!({"type": "set_axes", "origin": [0, 0, 0], "x_axis": [1, 1, 0], "y_axis": [-1, 1, 0]}),
    )
    .unwrap();
    // The sides run along the red and green axes: a square turned 45 degrees.
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 0, 0], "normal": [0, 0, 1]}),
    )
    .unwrap();
    let h = 2.0 * s;
    let expect = [
        [0.0, 0.0, 0.0],
        [h * s, h * s, 0.0],
        [2.0, 0.0, 0.0],
        [h * s, -h * s, 0.0],
    ];
    let got = positions(&d);
    assert!(
        got.len() == 4 && expect.iter().all(|q| got.iter().any(|p| near(*p, *q))),
        "{got:?} vs {expect:?}"
    );
    // Without a normal the corners differ in two drawing coordinates: (1, 1, 0) is along red
    // only, so it is refused (one flat coordinate pair is not enough).
    let mut other = doc();
    run(
        &mut other,
        json!({"type": "set_axes", "origin": [0, 0, 0], "x_axis": [1, 1, 0], "y_axis": [-1, 1, 0]}),
    )
    .unwrap();
    run(
        &mut other,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap_err();
}

#[test]
fn standard_views_inside_a_mirrored_placement_keep_blue_where_the_definition_z_goes() {
    // (flips, rotate first?, expected top direction, expected top up, expected front direction)
    let cases = [
        (
            vec!["red"],
            false,
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
        ),
        (
            vec!["blue"],
            false,
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
        ),
        (
            vec!["red"],
            true,
            [0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ),
    ];
    for (flips, turn, top, up, front) in cases {
        let mut d = doc();
        let a = crate_at(&mut d, [0.0, 0.0, 0.0]);
        if turn {
            run(
                &mut d,
                json!({"type": "rotate", "ids": [a], "center": [0, 0, 0], "axis": [0, 0, 1], "angle": 90}),
            )
            .unwrap();
        }
        for axis in &flips {
            run(
                &mut d,
                json!({"type": "flip", "ids": [a], "axis": axis, "frame": "own"}),
            )
            .unwrap();
        }
        run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
        standard_view(&mut d, "top");
        let (dir, u) = look(&d);
        assert!(near(dir, top), "{flips:?} {turn}: top from {dir:?}");
        assert!(near(u, up), "{flips:?} {turn}: top up {u:?}");
        standard_view(&mut d, "front");
        let (dir, _) = look(&d);
        assert!(near(dir, front), "{flips:?} {turn}: front from {dir:?}");
    }
}
