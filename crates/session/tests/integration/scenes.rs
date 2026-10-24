//! Scenes, driven through JSON commands and queries, and kept in files.

use std::io::Write;
use std::path::PathBuf;

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

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-scenes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// The file's `document.json`.
fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

/// Rewrites the file's `document.json` through `f`.
fn edit_file(path: &PathBuf, f: impl FnOnce(&mut Value)) {
    let mut document = stored(path);
    f(&mut document);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    zip.finish().unwrap();
}

fn add_tag(d: &mut Document, name: &str) -> String {
    let out = run(d, json!({"type": "add_tag", "name": name})).unwrap();
    out["changes"]["tags"][0].as_str().unwrap().to_string()
}

fn set_visible(d: &mut Document, tag: &str, visible: bool) {
    run(
        d,
        json!({"type": "change_tag", "tag": tag, "visible": visible}),
    )
    .unwrap();
}

fn visible(d: &Document) -> Vec<bool> {
    ask(d, json!({"type": "tags"}))["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["visible"].as_bool().unwrap())
        .collect()
}

/// Adds a scene with `params` and returns its id.
fn add_scene(d: &mut Document, params: Value) -> String {
    let mut command = json!({"type": "add_scene"});
    command
        .as_object_mut()
        .unwrap()
        .extend(params.as_object().unwrap().clone());
    let out = run(d, command).unwrap();
    let ids = out["changes"]["scenes"].as_array().unwrap();
    assert_eq!(ids.len(), 1, "{out}");
    ids[0].as_str().unwrap().to_string()
}

fn scenes(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "scenes"}))["scenes"]
        .as_array()
        .unwrap()
        .clone()
}

fn names(d: &Document) -> Vec<String> {
    scenes(d)
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect()
}

fn camera(d: &Document) -> Value {
    ask(d, json!({"type": "camera"}))
}

fn undo_labels(d: &Document) -> Value {
    ask(d, json!({"type": "history"}))["undo"].clone()
}

fn set_camera(d: &mut Document, eye: [f64; 3]) {
    run(
        d,
        json!({"type": "set_camera", "eye": eye, "target": [0, 0, 0]}),
    )
    .unwrap();
}

#[test]
fn add_scene_stores_the_view_and_hidden_tags_and_selects_it() {
    let mut d = doc();
    let walls = add_tag(&mut d, "Walls");
    let roof = add_tag(&mut d, "Roof");
    set_visible(&mut d, &roof, false);
    set_camera(&mut d, [3.0, -9.0, 4.0]);
    run(
        &mut d,
        json!({"type": "set_projection", "projection": "parallel"}),
    )
    .unwrap();
    run(&mut d, json!({"type": "set_lighting", "mode": "night"})).unwrap();
    run(&mut d, json!({"type": "set_color_by_tag", "on": true})).unwrap();
    let before = d.revision();
    let one = add_scene(&mut d, json!({}));
    assert!(one.starts_with('p'), "{one}");
    assert!(d.revision() > before && d.is_modified());
    assert_eq!(undo_labels(&d)[0], "add_scene");
    assert_eq!(
        scenes(&d),
        [json!({"id": one, "name": "Scene 1", "camera": true, "tags": true, "selected": true})]
    );
    let detail = ask(&d, json!({"type": "scene", "scene": one}));
    assert_eq!(detail["name"], "Scene 1");
    assert_eq!(detail["hidden_tags"], json!([roof]));
    let view = &detail["view"];
    assert_eq!(view["eye"], json!([3.0, -9.0, 4.0]));
    assert_eq!(view["target"], json!([0.0, 0.0, 0.0]));
    assert_eq!(view["up"], json!([0.0, 0.0, 1.0]));
    assert!((view["fov_y_deg"].as_f64().unwrap() - 35.0).abs() < 1e-9);
    assert_eq!(view["projection"], "parallel");
    assert_eq!(view["lighting"], "night");
    assert_eq!(view["color_by_tag"], true);

    // Parts not stored are null; default names take the lowest free number.
    let two = add_scene(&mut d, json!({"camera": false}));
    let three = add_scene(&mut d, json!({"tags": false, "name": "Plan"}));
    let four = add_scene(&mut d, json!({}));
    assert_eq!(names(&d), ["Scene 1", "Scene 2", "Plan", "Scene 3"]);
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": two}))["view"],
        Value::Null
    );
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": three}))["hidden_tags"],
        Value::Null
    );
    let selected: Vec<bool> = scenes(&d)
        .iter()
        .map(|s| s["selected"].as_bool().unwrap())
        .collect();
    assert_eq!(selected, [false, false, false, true]);
    assert_eq!(d.selected_scene().unwrap().to_string(), four);
    let _ = walls;

    // Undo takes the scene away, and with it the selection.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(names(&d), ["Scene 1", "Scene 2", "Plan"]);
    assert_eq!(d.selected_scene(), None);
}

#[test]
fn scene_names_must_be_valid_and_distinct() {
    let mut d = doc();
    let one = add_scene(&mut d, json!({"name": "Front"}));
    for name in ["Front", "", "  ", &"x".repeat(257)] {
        let e = run(&mut d, json!({"type": "add_scene", "name": name})).unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput, "{name:?}: {}", e.1);
    }
    let two = add_scene(&mut d, json!({"name": "front"}));
    let e = run(
        &mut d,
        json!({"type": "change_scene", "scene": two, "name": "Front"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    run(
        &mut d,
        json!({"type": "change_scene", "scene": one, "name": "Elevation"}),
    )
    .unwrap();
    assert_eq!(names(&d), ["Elevation", "front"]);
    assert_eq!(undo_labels(&d)[0], "change_scene");
    // A new default name skips none taken: "Scene 1" is free.
    add_scene(&mut d, json!({}));
    assert_eq!(names(&d)[2], "Scene 1");
    for command in [
        json!({"type": "change_scene", "scene": "p999", "name": "X"}),
        json!({"type": "update_scene", "scene": "p999"}),
        json!({"type": "move_scene", "scene": "p999", "index": 0}),
        json!({"type": "delete_scene", "scene": "p999"}),
        json!({"type": "go_to_scene", "scene": "p999"}),
    ] {
        let e = run(&mut d, command.clone()).unwrap_err();
        assert_eq!(e.0, ErrorKind::NotFound, "{command}");
        assert!(e.1.contains("p999"), "{}", e.1);
    }
    let q: Query = serde_json::from_value(json!({"type": "scene", "scene": "p999"})).unwrap();
    assert_eq!(d.query(&q).unwrap_err().kind, ErrorKind::NotFound);
}

#[test]
fn update_scene_stores_again_what_it_stores_unless_told() {
    let mut d = doc();
    let walls = add_tag(&mut d, "Walls");
    set_camera(&mut d, [1.0, -9.0, 4.0]);
    let s = add_scene(&mut d, json!({"tags": false}));
    set_camera(&mut d, [2.0, -9.0, 4.0]);
    set_visible(&mut d, &walls, false);
    run(&mut d, json!({"type": "update_scene", "scene": s})).unwrap();
    assert_eq!(undo_labels(&d)[0], "update_scene");
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["view"]["eye"], json!([2.0, -9.0, 4.0]));
    assert_eq!(detail["hidden_tags"], Value::Null);
    // Asked to, it stores the tags too, or stops storing the view.
    run(
        &mut d,
        json!({"type": "update_scene", "scene": s, "tags": true, "camera": false}),
    )
    .unwrap();
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["view"], Value::Null);
    assert_eq!(detail["hidden_tags"], json!([walls]));
    assert_eq!(
        scenes(&d)[0],
        json!({"id": s, "name": "Scene 1", "camera": false, "tags": true, "selected": true})
    );
    // Storing what it stores already is no step.
    let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    run(&mut d, json!({"type": "update_scene", "scene": s})).unwrap();
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], depth);
}

#[test]
fn change_scene_drops_or_adds_a_part_and_keeps_what_stays_stored() {
    let mut d = doc();
    let walls = add_tag(&mut d, "Walls");
    set_camera(&mut d, [1.0, -9.0, 4.0]);
    let s = add_scene(&mut d, json!({}));
    let stored = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(stored["hidden_tags"], json!([]));
    // Tag visibility changes and the camera moves; then the camera is no longer stored.
    set_visible(&mut d, &walls, false);
    set_camera(&mut d, [2.0, -9.0, 4.0]);
    let depth = |d: &Document| ask(d, json!({"type": "history"}))["undo_depth"].clone();
    let before = depth(&d);
    let out = run(
        &mut d,
        json!({"type": "change_scene", "scene": s, "camera": false}),
    )
    .unwrap();
    assert_eq!(out["changes"]["scenes"], json!([s]));
    assert_eq!(undo_labels(&d)[0], "change_scene");
    assert_eq!(depth(&d), json!(before.as_u64().unwrap() + 1));
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["view"], Value::Null);
    assert_eq!(
        detail["hidden_tags"],
        json!([]),
        "the stored tags are unchanged"
    );
    assert_eq!(detail["name"], "Scene 1");
    // Turned on again, the camera is stored from now; tags true keeps the stored tags.
    run(
        &mut d,
        json!({"type": "change_scene", "scene": s, "camera": true, "tags": true}),
    )
    .unwrap();
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["view"]["eye"], json!([2.0, -9.0, 4.0]));
    assert_eq!(detail["hidden_tags"], json!([]));
    // Camera true on a scene that stores it keeps the stored view.
    set_camera(&mut d, [3.0, -9.0, 4.0]);
    let before = depth(&d);
    run(
        &mut d,
        json!({"type": "change_scene", "scene": s, "camera": true}),
    )
    .unwrap();
    let detail = ask(&d, json!({"type": "scene", "scene": s}));
    assert_eq!(detail["view"]["eye"], json!([2.0, -9.0, 4.0]));
    assert_eq!(depth(&d), before, "nothing changed: no step");
    // Tags dropped, then stored from the current visibility.
    run(
        &mut d,
        json!({"type": "change_scene", "scene": s, "tags": false}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": s}))["hidden_tags"],
        Value::Null
    );
    run(
        &mut d,
        json!({"type": "change_scene", "scene": s, "tags": true}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": s}))["hidden_tags"],
        json!([walls])
    );
    // Undo takes each change back.
    run(&mut d, json!({"type": "undo", "count": 2})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": s}))["hidden_tags"],
        json!([])
    );
    // Something to change is needed.
    let e = run(&mut d, json!({"type": "change_scene", "scene": s})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput, "{}", e.1);
}

#[test]
fn move_scene_reorders_and_refuses_an_index_out_of_range() {
    let mut d = doc();
    let a = add_scene(&mut d, json!({"name": "A"}));
    add_scene(&mut d, json!({"name": "B"}));
    add_scene(&mut d, json!({"name": "C"}));
    let out = run(
        &mut d,
        json!({"type": "move_scene", "scene": a, "index": 1}),
    )
    .unwrap();
    assert_eq!(out["changes"]["scenes"], json!([a]));
    assert_eq!(names(&d), ["B", "A", "C"]);
    assert_eq!(undo_labels(&d)[0], "move_scene");
    let e = run(
        &mut d,
        json!({"type": "move_scene", "scene": a, "index": 3}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(names(&d), ["A", "B", "C"]);
}

#[test]
fn undoing_delete_scene_restores_it_in_place_with_what_it_stored() {
    let mut d = doc();
    let walls = add_tag(&mut d, "Walls");
    add_scene(&mut d, json!({"name": "A"}));
    set_visible(&mut d, &walls, false);
    set_camera(&mut d, [5.0, -9.0, 4.0]);
    let b = add_scene(&mut d, json!({"name": "B"}));
    add_scene(&mut d, json!({"name": "C"}));
    let stored_b = ask(&d, json!({"type": "scene", "scene": b}));
    let out = run(&mut d, json!({"type": "delete_scene", "scene": b})).unwrap();
    assert_eq!(out["changes"]["scenes"], json!([b]));
    assert_eq!(names(&d), ["A", "C"]);
    assert_eq!(undo_labels(&d)[0], "delete_scene");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(names(&d), ["A", "B", "C"]);
    let mut restored = ask(&d, json!({"type": "scene", "scene": b}));
    restored["revision"] = stored_b["revision"].clone();
    assert_eq!(restored, stored_b);
    assert_eq!(restored["hidden_tags"], json!([walls]));
}

#[test]
fn delete_tag_drops_it_from_every_scene_and_undo_puts_it_back() {
    let mut d = doc();
    let walls = add_tag(&mut d, "Walls");
    let roof = add_tag(&mut d, "Roof");
    set_visible(&mut d, &walls, false);
    set_visible(&mut d, &roof, false);
    let a = add_scene(&mut d, json!({}));
    set_visible(&mut d, &roof, true);
    let b = add_scene(&mut d, json!({}));
    let c = add_scene(&mut d, json!({"tags": false}));
    let hidden =
        |d: &Document, s: &str| ask(d, json!({"type": "scene", "scene": s}))["hidden_tags"].clone();
    let out = run(&mut d, json!({"type": "delete_tag", "tag": walls})).unwrap();
    assert_eq!(out["changes"]["tags"], json!([walls]));
    let mut changed: Vec<String> = out["changes"]["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    changed.sort();
    let mut expected = vec![a.clone(), b.clone()];
    expected.sort();
    assert_eq!(changed, expected);
    assert_eq!(hidden(&d, &a), json!([roof]));
    assert_eq!(hidden(&d, &b), json!([]));
    assert_eq!(hidden(&d, &c), Value::Null);
    // One undo step brings the tag back into every scene that hid it.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(hidden(&d, &a), json!([walls, roof]));
    assert_eq!(hidden(&d, &b), json!([walls]));
    assert_eq!(hidden(&d, &c), Value::Null);
}

#[test]
fn go_to_scene_sets_the_view_without_an_undo_step() {
    let mut d = doc();
    set_camera(&mut d, [3.0, -9.0, 4.0]);
    run(
        &mut d,
        json!({"type": "set_projection", "projection": "parallel"}),
    )
    .unwrap();
    run(&mut d, json!({"type": "set_lighting", "mode": "night"})).unwrap();
    run(&mut d, json!({"type": "set_color_by_tag", "on": true})).unwrap();
    let s = add_scene(&mut d, json!({}));
    let saved = camera(&d);
    let first = add_scene(&mut d, json!({"name": "Other"}));
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [0, 0, 10], "target": [0, 0, 0], "up": [1, 0, 0], "projection": "perspective", "fov_y_deg": 50}),
    )
    .unwrap();
    run(&mut d, json!({"type": "set_lighting", "mode": "day"})).unwrap();
    run(&mut d, json!({"type": "set_color_by_tag", "on": false})).unwrap();
    let revision = d.revision();
    let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
    assert_eq!(camera(&d), saved);
    assert_eq!(d.revision(), revision);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], depth);
    assert_eq!(d.selected_scene().unwrap().to_string(), s);
    let _ = first;

    // A straight-down view keeps its screen's up.
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [0, 0, 10], "target": [0, 0, 0], "up": [1, 0, 0]}),
    )
    .unwrap();
    let top = add_scene(&mut d, json!({"name": "Top"}));
    let top_camera = camera(&d);
    run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
    run(&mut d, json!({"type": "go_to_scene", "scene": top})).unwrap();
    assert_eq!(camera(&d), top_camera);
    assert_eq!(camera(&d)["up"], json!([1.0, 0.0, 0.0]));
}

#[test]
fn go_to_scene_applies_tag_visibility_as_one_undo_step_only_when_it_changes() {
    let mut d = doc();
    let walls = add_tag(&mut d, "Walls");
    let roof = add_tag(&mut d, "Roof");
    let glass = add_tag(&mut d, "Glass");
    set_visible(&mut d, &walls, false);
    let s = add_scene(&mut d, json!({}));
    assert_eq!(visible(&d), [false, true, true]);
    // Already as stored: no step.
    let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    let out = run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
    assert_eq!(out["changes"].get("tags"), None);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], depth);

    set_visible(&mut d, &walls, true);
    set_visible(&mut d, &roof, false);
    set_visible(&mut d, &glass, false);
    let revision = d.revision();
    let out = run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
    assert_eq!(visible(&d), [false, true, true]);
    assert_eq!(out["changes"]["tags"], json!([walls, roof, glass]));
    assert!(d.revision() > revision);
    assert_eq!(undo_labels(&d)[0], "go_to_scene");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(visible(&d), [true, false, false]);
}

#[test]
fn a_scene_changes_only_what_it_stores() {
    let mut d = doc();
    let walls = add_tag(&mut d, "Walls");
    set_camera(&mut d, [3.0, -9.0, 4.0]);
    let tags_only = add_scene(&mut d, json!({"camera": false}));
    set_visible(&mut d, &walls, false);
    let camera_only = add_scene(&mut d, json!({"tags": false}));
    set_camera(&mut d, [7.0, -9.0, 4.0]);
    run(&mut d, json!({"type": "set_lighting", "mode": "night"})).unwrap();

    // Tags only: the camera and view settings stay.
    let before = camera(&d);
    run(&mut d, json!({"type": "go_to_scene", "scene": tags_only})).unwrap();
    assert_eq!(camera(&d), before);
    assert_eq!(visible(&d), [true]);

    // Camera only: tag visibility stays.
    set_visible(&mut d, &walls, false);
    run(&mut d, json!({"type": "go_to_scene", "scene": camera_only})).unwrap();
    assert_eq!(camera(&d)["eye"], json!([3.0, -9.0, 4.0]));
    assert_eq!(camera(&d)["lighting"], "day");
    assert_eq!(visible(&d), [false]);
}

#[test]
fn a_failed_atomic_batch_undoes_go_to_scene() {
    let mut d = doc();
    let walls = add_tag(&mut d, "Walls");
    set_visible(&mut d, &walls, false);
    let s = add_scene(&mut d, json!({}));
    let other = add_scene(&mut d, json!({"name": "Other"}));
    set_visible(&mut d, &walls, true);
    set_camera(&mut d, [9.0, -9.0, 4.0]);
    let before = camera(&d);
    let commands: Vec<Command> = vec![
        serde_json::from_value(json!({"type": "go_to_scene", "scene": s})).unwrap(),
        serde_json::from_value(json!({"type": "delete_scene", "scene": "p999"})).unwrap(),
    ];
    let e = d.execute_batch(commands, true).unwrap_err();
    assert!(e.rolled_back);
    assert_eq!(camera(&d), before);
    assert_eq!(visible(&d), [true]);
    assert_eq!(d.selected_scene().unwrap().to_string(), other);
    // Done, the batch is one step.
    let commands: Vec<Command> =
        vec![serde_json::from_value(json!({"type": "go_to_scene", "scene": s})).unwrap()];
    d.execute_batch(commands, true).unwrap();
    assert_eq!(visible(&d), [false]);
    assert_eq!(undo_labels(&d)[0], "go_to_scene");
}

#[test]
fn scenes_are_saved_and_opened_in_order() {
    let mut d = doc();
    let walls = add_tag(&mut d, "Walls");
    set_visible(&mut d, &walls, false);
    set_camera(&mut d, [3.0, -9.0, 4.0]);
    run(
        &mut d,
        json!({"type": "set_projection", "projection": "parallel"}),
    )
    .unwrap();
    add_scene(&mut d, json!({"name": "Zeta"}));
    add_scene(&mut d, json!({"name": "Alpha", "camera": false}));
    add_scene(&mut d, json!({"name": "Mid", "tags": false}));
    let path = scratch("scenes.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(
        stored(&path)["schema_version"],
        session::file::SCENES_SCHEMA_VERSION
    );
    assert_eq!(stored(&path)["schema_version"], 11);
    let details = |d: &Document| -> Vec<Value> {
        scenes(d)
            .iter()
            .map(|s| {
                let mut v = ask(d, json!({"type": "scene", "scene": s["id"]}));
                v.as_object_mut().unwrap().remove("revision");
                v
            })
            .collect()
    };
    let before = details(&d);

    let mut opened = doc();
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(names(&opened), ["Zeta", "Alpha", "Mid"]);
    assert_eq!(details(&opened), before);
    // The selected scene is not saved.
    assert!(scenes(&opened).iter().all(|s| s["selected"] == false));
    // New and open clear it.
    let s = scenes(&opened)[0]["id"].clone();
    run(&mut opened, json!({"type": "go_to_scene", "scene": s})).unwrap();
    assert!(opened.selected_scene().is_some());
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(opened.selected_scene(), None);
    run(&mut opened, json!({"type": "go_to_scene", "scene": s})).unwrap();
    run(&mut opened, json!({"type": "new"})).unwrap();
    assert_eq!(opened.selected_scene(), None);
    assert!(scenes(&opened).is_empty());

    // A scene hiding a tag the file lacks is refused, as is a view beyond the limit.
    for (name, f) in [
        (
            "broken-tag.skr",
            Box::new(|doc: &mut Value| doc["model"]["pages"][0]["tags"] = json!([999_999]))
                as Box<dyn Fn(&mut Value)>,
        ),
        (
            "far.skr",
            Box::new(|doc: &mut Value| {
                doc["model"]["pages"][0]["view"]["eye"] = json!([2e6, 0, 0])
            }),
        ),
    ] {
        let broken = scratch(name);
        std::fs::copy(&path, &broken).unwrap();
        edit_file(&broken, f);
        let e = run(
            &mut opened,
            json!({"type": "open", "path": broken.to_str().unwrap(), "discard": true}),
        )
        .unwrap_err();
        assert_eq!(e.0, ErrorKind::File, "{name}: {}", e.1);
    }
}

#[test]
fn a_model_without_scenes_keeps_its_schema_and_older_files_open_without_scenes() {
    let mut d = doc();
    add_tag(&mut d, "Walls");
    let path = scratch("no-scenes.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(document["schema_version"], 10);
    assert!(document["model"].get("pages").is_none());
    let mut opened = doc();
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert!(scenes(&opened).is_empty());
    // Deleting the last scene goes back to the older schema.
    let s = add_scene(&mut d, json!({}));
    run(&mut d, json!({"type": "delete_scene", "scene": s})).unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    assert_eq!(stored(&path)["schema_version"], 10);
}

#[test]
fn going_to_a_scene_and_updating_it_changes_nothing_for_any_view() {
    // A field of view whose trip through degrees is not exact.
    let inexact = (1..1700)
        .map(|k| f64::from(k) / 10.0 + 1.0)
        .find(|d| d.to_radians().to_degrees().to_radians() != d.to_radians())
        .expect("a field of view that drifts through degrees");
    let mut d = doc();
    let views = [
        json!({"type": "set_camera", "eye": [3, -9, 4], "target": [0, 0, 0], "fov_y_deg": inexact}),
        // Straight down, the screen's up a horizontal direction that is not an axis.
        json!({"type": "set_camera", "eye": [0, 0, 10], "target": [0, 0, 0], "up": [1, 1, 0], "fov_y_deg": inexact}),
    ];
    for (k, view) in views.into_iter().enumerate() {
        run(&mut d, view).unwrap();
        let saved = camera(&d);
        let s = add_scene(&mut d, json!({"name": format!("View {k}")}));
        run(
            &mut d,
            json!({"type": "set_camera", "eye": [5, 5, 5], "target": [0, 0, 0], "fov_y_deg": 60}),
        )
        .unwrap();
        run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
        assert_eq!(camera(&d), saved, "{k}");
        let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
        run(&mut d, json!({"type": "update_scene", "scene": s})).unwrap();
        assert_eq!(
            ask(&d, json!({"type": "history"}))["undo_depth"],
            depth,
            "{k}: update after go_to_scene stored a change"
        );
    }
    let up = camera(&d)["up"].clone();
    let half = std::f64::consts::FRAC_1_SQRT_2;
    assert!((up[0].as_f64().unwrap() - half).abs() < 1e-12, "{up}");
    assert!((up[1].as_f64().unwrap() - half).abs() < 1e-12, "{up}");
    assert_eq!(up[2], 0.0);
}

#[test]
fn go_to_scene_in_an_open_group_places_the_camera_in_model_coordinates() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let ids: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .collect();
    run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    run(
        &mut d,
        json!({"type": "move", "ids": [group], "offset": [10, 0, 0]}),
    )
    .unwrap();
    set_camera(&mut d, [3.0, -9.0, 4.0]);
    let saved = camera(&d);
    let s = add_scene(&mut d, json!({}));
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    set_camera(&mut d, [6.0, -9.0, 4.0]);
    // Inside the group the scene's view is where it was in the model, as set_camera's is.
    run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
    assert_eq!(camera(&d), saved);
    assert_eq!(
        ask(&d, json!({"type": "context"}))["path"],
        json!([group]),
        "the group stays open"
    );
    // A scene added inside the group stores the same model coordinates.
    let inside = add_scene(&mut d, json!({}));
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": inside}))["view"]["eye"],
        json!([3.0, -9.0, 4.0])
    );
}

#[test]
fn a_camera_beyond_the_coordinate_limit_is_not_stored() {
    // Window navigation can take the camera past the limit; no scene command stores it.
    let mut d = doc();
    let kept = add_scene(&mut d, json!({}));
    let unstored = add_scene(&mut d, json!({"camera": false}));
    let revision = ask(&d, json!({"type": "scenes"}))["revision"].clone();
    d.camera.eye = glam::DVec3::new(2.0e6, 0.0, 0.0);
    for command in [
        json!({"type": "add_scene"}),
        json!({"type": "update_scene", "scene": kept}),
        json!({"type": "change_scene", "scene": unstored, "camera": true}),
    ] {
        let (kind, message) = run(&mut d, command.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{command}");
        assert!(message.contains("beyond +/-1000000 m"), "{message}");
    }
    assert_eq!(names(&d), ["Scene 1", "Scene 2"]);
    assert_eq!(ask(&d, json!({"type": "scenes"}))["revision"], revision);
    // What does not store the camera still works.
    run(&mut d, json!({"type": "add_scene", "camera": false})).unwrap();
    run(
        &mut d,
        json!({"type": "update_scene", "scene": kept, "camera": false}),
    )
    .unwrap();

    // Back near the model, a scene stores the view, saves and opens again.
    set_camera(&mut d, [10.0, -10.0, 5.0]);
    run(
        &mut d,
        json!({"type": "update_scene", "scene": kept, "camera": true}),
    )
    .unwrap();
    let path = scratch("far-camera.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut back = doc();
    run(
        &mut back,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(names(&back), ["Scene 1", "Scene 2", "Scene 3"]);
    let view = &ask(&back, json!({"type": "scene", "scene": kept}))["view"];
    assert_eq!(view["eye"], json!([10.0, -10.0, 5.0]));
}

/// Two groups of one square each, at x = 0 and x = 5; returns their ids.
fn two_groups(d: &mut Document) -> (String, String) {
    let mut ids = Vec::new();
    for x in [0.0, 5.0] {
        run(
            d,
            json!({"type": "add_rectangle", "corner": [x, 0, 0], "opposite": [x + 1.0, 1, 0]}),
        )
        .unwrap();
        let faces = ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
            .as_array()
            .unwrap()
            .clone();
        let face = faces.last().unwrap()["id"].clone();
        run(d, json!({"type": "make_group", "ids": [face]})).unwrap();
        ids.push(
            ask(d, json!({"type": "selection"}))["ids"][0]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    (ids[0].clone(), ids[1].clone())
}

/// The x of an instance's translation.
fn x_of(d: &Document, id: &str) -> f64 {
    ask(d, json!({"type": "entity", "id": id}))["transform"][9]
        .as_f64()
        .unwrap_or_else(|| {
            panic!(
                "no transform in {}",
                ask(d, json!({"type": "entity", "id": id}))
            )
        })
}

fn shift(d: &mut Document, id: &str, dx: f64) {
    run(
        d,
        json!({"type": "move", "ids": [id], "offset": [dx, 0, 0]}),
    )
    .unwrap();
}

#[test]
fn a_scene_stores_instance_positions_and_going_to_it_sets_them_in_one_undo_step() {
    let mut d = doc();
    let (a, b) = two_groups(&mut d);
    let s = add_scene(&mut d, json!({"name": "Open", "positions": [a]}));
    assert_eq!(scenes(&d)[0]["positions"], json!([a]));
    assert_eq!(
        ask(&d, json!({"type": "scene", "scene": s}))["positions"],
        json!([a])
    );
    shift(&mut d, &a, 3.0);
    shift(&mut d, &b, 3.0);
    let steps = undo_labels(&d).as_array().unwrap().len();
    run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
    assert_eq!(
        (x_of(&d, &a), x_of(&d, &b)),
        (0.0, 3.0),
        "only the stored one goes back"
    );
    let labels = undo_labels(&d);
    assert_eq!(labels.as_array().unwrap().len(), steps + 1);
    assert_eq!(labels[0], "Go to scene Open");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!((x_of(&d, &a), x_of(&d, &b)), (3.0, 3.0));
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(x_of(&d, &a), 0.0);
}

#[test]
fn going_to_a_scene_without_positions_adds_no_undo_step_and_moves_nothing() {
    let mut d = doc();
    let (a, _) = two_groups(&mut d);
    let s = add_scene(&mut d, json!({}));
    shift(&mut d, &a, 3.0);
    let before = undo_labels(&d);
    run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
    assert_eq!(undo_labels(&d), before);
    assert_eq!(x_of(&d, &a), 3.0);
    assert!(scenes(&d)[0].get("positions").is_none());
}

#[test]
fn positions_true_stores_every_root_instance_and_update_stores_them_again() {
    let mut d = doc();
    let (a, b) = two_groups(&mut d);
    let s = add_scene(&mut d, json!({"positions": true}));
    assert_eq!(scenes(&d)[0]["positions"], json!([a, b]));
    shift(&mut d, &a, 2.0);
    // update_scene without positions stores the same set again.
    run(&mut d, json!({"type": "update_scene", "scene": s})).unwrap();
    shift(&mut d, &a, 4.0);
    run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
    assert_eq!(x_of(&d, &a), 2.0);
    // false stops storing.
    run(
        &mut d,
        json!({"type": "change_scene", "scene": s, "positions": false}),
    )
    .unwrap();
    assert!(scenes(&d)[0].get("positions").is_none());
    // change_scene true stores the set again from the current transforms.
    run(
        &mut d,
        json!({"type": "change_scene", "scene": s, "positions": [b]}),
    )
    .unwrap();
    assert_eq!(scenes(&d)[0]["positions"], json!([b]));
    run(
        &mut d,
        json!({"type": "change_scene", "scene": s, "positions": true}),
    )
    .unwrap();
    assert_eq!(scenes(&d)[0]["positions"], json!([b]));
}

#[test]
fn positions_must_be_instances_of_the_root() {
    let mut d = doc();
    let (a, _) = two_groups(&mut d);
    for bad in [json!(["i999"]), json!(["e1"]), json!([])] {
        let (kind, _) = run(&mut d, json!({"type": "add_scene", "positions": bad})).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput);
    }
    assert!(scenes(&d).is_empty(), "a refused command stores nothing");
    // An instance inside a group is not an instance of the root: put one inside, then try.
    run(&mut d, json!({"type": "open_instance", "id": a})).unwrap();
    let inner = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 1], "opposite": [1, 1, 1]}),
    )
    .unwrap();
    let _ = inner;
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][1]["id"].clone();
    run(&mut d, json!({"type": "make_group", "ids": [face]})).unwrap();
    let nested = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let (kind, _) = run(&mut d, json!({"type": "add_scene", "positions": [nested]})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}

#[test]
fn erasing_an_instance_drops_it_from_scenes_in_the_same_undo_step() {
    let mut d = doc();
    let (a, b) = two_groups(&mut d);
    let both = add_scene(&mut d, json!({"name": "Both", "positions": true}));
    add_scene(&mut d, json!({"name": "OnlyA", "positions": [a]}));
    let steps = undo_labels(&d).as_array().unwrap().len();
    run(&mut d, json!({"type": "erase", "ids": [a]})).unwrap();
    assert_eq!(undo_labels(&d).as_array().unwrap().len(), steps + 1);
    let all = scenes(&d);
    assert_eq!(all[0]["positions"], json!([b]));
    assert!(all[1].get("positions").is_none());
    // Going to a scene still works and moves what is left.
    shift(&mut d, &b, 2.0);
    run(&mut d, json!({"type": "go_to_scene", "scene": both})).unwrap();
    assert_eq!(x_of(&d, &b), 0.0);
}

#[test]
fn undoing_an_erase_restores_the_stored_positions() {
    let mut d = doc();
    let (a, b) = two_groups(&mut d);
    add_scene(&mut d, json!({"positions": true}));
    run(&mut d, json!({"type": "erase", "ids": [a]})).unwrap();
    assert_eq!(scenes(&d)[0]["positions"], json!([b]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(scenes(&d)[0]["positions"], json!([a, b]));
}

#[test]
fn scene_positions_are_saved_and_older_files_open() {
    let mut d = doc();
    let (a, b) = two_groups(&mut d);
    add_scene(&mut d, json!({"name": "P", "positions": [b]}));
    shift(&mut d, &b, 1.5);
    let path = scratch("positions.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(
        stored(&path)["schema_version"],
        session::file::SCENE_POSITIONS_SCHEMA_VERSION
    );
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(scenes(&e)[0]["positions"], json!([b]));
    let s = scenes(&e)[0]["id"].as_str().unwrap().to_string();
    run(&mut e, json!({"type": "go_to_scene", "scene": s})).unwrap();
    assert_eq!(x_of(&e, &b), 0.0);
    let _ = a;

    // A scene without positions keeps the old schema number and writes no positions.
    let mut plain = doc();
    two_groups(&mut plain);
    add_scene(&mut plain, json!({}));
    let old = scratch("no-positions.skr");
    run(
        &mut plain,
        json!({"type": "save", "path": old.to_str().unwrap()}),
    )
    .unwrap();
    assert!(
        stored(&old)["schema_version"].as_u64().unwrap()
            < session::file::SCENE_POSITIONS_SCHEMA_VERSION as u64
    );
    assert!(stored(&old)["model"]["pages"][0].get("positions").is_none());
}

#[test]
fn set_scene_transition_is_validated_one_undo_step_and_no_op_when_unchanged() {
    let mut d = doc();
    assert_eq!(
        ask(&d, json!({"type": "scene_transition"})),
        json!({"type": "scene_transition", "on": true, "seconds": 1.0})
    );
    let steps = undo_labels(&d).as_array().unwrap().len();
    run(
        &mut d,
        json!({"type": "set_scene_transition", "seconds": 2.5}),
    )
    .unwrap();
    assert_eq!(ask(&d, json!({"type": "scene_transition"}))["seconds"], 2.5);
    assert_eq!(ask(&d, json!({"type": "scene_transition"}))["on"], true);
    let labels = undo_labels(&d);
    assert_eq!(labels.as_array().unwrap().len(), steps + 1);
    assert_eq!(labels[0], "set_scene_transition");
    run(&mut d, json!({"type": "set_scene_transition", "on": false})).unwrap();
    let revision = d.revision();
    run(&mut d, json!({"type": "set_scene_transition", "on": false})).unwrap();
    assert_eq!(d.revision(), revision, "nothing to change");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "scene_transition"}))["on"], true);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "scene_transition"}))["seconds"], 1.0);
    for bad in [json!(-1.0), json!(10.5)] {
        let (kind, _) = run(
            &mut d,
            json!({"type": "set_scene_transition", "seconds": bad}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput);
    }
    let (kind, _) = run(&mut d, json!({"type": "set_scene_transition"})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "needs on or seconds");
}

#[test]
fn the_scene_transition_is_saved_and_the_schema_is_20_only_when_it_is_not_the_default() {
    let mut d = doc();
    two_groups(&mut d);
    let plain = scratch("transition-default.skr");
    run(
        &mut d,
        json!({"type": "save", "path": plain.to_str().unwrap()}),
    )
    .unwrap();
    assert!(
        stored(&plain)["schema_version"].as_u64().unwrap()
            < session::file::SCENE_POSITIONS_SCHEMA_VERSION as u64
    );
    assert!(stored(&plain)["model"].get("transition").is_none());

    run(
        &mut d,
        json!({"type": "set_scene_transition", "on": false, "seconds": 3.0}),
    )
    .unwrap();
    let path = scratch("transition.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(
        stored(&path)["schema_version"],
        session::file::TRANSITION_SCHEMA_VERSION
    );
    assert_eq!(session::file::TRANSITION_SCHEMA_VERSION, 24);
    assert_eq!(session::file::SCHEMA_VERSION, 27);
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(
        ask(&e, json!({"type": "scene_transition"})),
        json!({"type": "scene_transition", "on": false, "seconds": 3.0})
    );
    // An older file opens with the default.
    let mut f = doc();
    run(
        &mut f,
        json!({"type": "open", "path": plain.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(ask(&f, json!({"type": "scene_transition"}))["on"], true);
}

#[test]
fn a_motion_to_a_scene_runs_from_the_current_state_to_what_going_there_makes() {
    let mut d = doc();
    let (a, b) = two_groups(&mut d);
    run(&mut d, json!({"type": "standard_view", "view": "front"})).unwrap();
    let s = add_scene(&mut d, json!({"name": "End", "positions": [a]}));
    shift(&mut d, &a, 4.0);
    shift(&mut d, &b, 4.0);
    run(&mut d, json!({"type": "standard_view", "view": "top"})).unwrap();
    let id: kernel::PageId = serde_json::from_value(json!(s)).unwrap();
    let revision = d.revision();
    let before = d.camera.clone();
    let motion = d.motion_between(None, id).unwrap();
    assert_eq!(d.revision(), revision, "does not change the document");
    assert_eq!(d.camera, before);
    assert_eq!(motion.camera_at(0.0), before);
    // Only the stored instance moves, from where it is to the stored place.
    assert_eq!(motion.moves.len(), 1);
    let (_, from, to) = motion.moves[0];
    assert_eq!((from.translation.x, to.translation.x), (4.0, 0.0));
    let half = motion.scene_at(d.scene(), 0.5);
    let mid = half.root().instance(motion.moves[0].0).unwrap().transform;
    assert_eq!(mid.translation.x, 2.0);
    assert_eq!(
        d.scene()
            .root()
            .instance(motion.moves[0].0)
            .unwrap()
            .transform
            .translation
            .x,
        4.0
    );
    // Going there makes the camera the motion ends at.
    run(&mut d, json!({"type": "go_to_scene", "scene": s})).unwrap();
    let mut end = motion.camera_at(1.0);
    end.viewport = d.camera.viewport;
    assert_eq!(end, d.camera);
}
