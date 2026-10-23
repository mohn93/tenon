//! X-ray and Hidden Geometry: view settings saved with the camera and scenes.

use std::path::{Path, PathBuf};

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Value {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    serde_json::to_value(d.execute(cmd).unwrap()).unwrap()
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn camera(d: &Document) -> Value {
    ask(d, json!({"type": "camera"}))
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-view-flags-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

fn square(d: &mut Document) -> Vec<Value> {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    out["changes"]["created"].as_array().unwrap().clone()
}

fn save(d: &mut Document, path: &Path) {
    run(
        d,
        json!({"type": "save", "path": path.to_str().unwrap(), "overwrite": true}),
    );
}

#[test]
fn both_are_off_by_default_and_view_settings_not_edits() {
    let mut d = doc();
    square(&mut d);
    assert_eq!(camera(&d)["x_ray"], false);
    assert_eq!(camera(&d)["hidden_geometry"], false);
    let (revision, depth) = (
        d.revision(),
        ask(&d, json!({"type": "history"}))["undo_depth"].clone(),
    );
    run(&mut d, json!({"type": "set_x_ray", "on": true}));
    run(&mut d, json!({"type": "set_hidden_geometry", "on": true}));
    assert_eq!(camera(&d)["x_ray"], true);
    assert_eq!(camera(&d)["hidden_geometry"], true);
    assert_eq!(d.revision(), revision, "not an edit");
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], depth);
    // A failed atomic batch puts them back.
    let batch: Vec<Command> = vec![
        serde_json::from_value(json!({"type": "set_x_ray", "on": false})).unwrap(),
        serde_json::from_value(json!({"type": "set_hidden_geometry", "on": false})).unwrap(),
        serde_json::from_value(json!({"type": "delete_tag", "tag": "t999"})).unwrap(),
    ];
    assert!(d.execute_batch(batch, true).is_err());
    assert_eq!(camera(&d)["x_ray"], true);
    assert_eq!(camera(&d)["hidden_geometry"], true);
    // New starts with both off.
    run(&mut d, json!({"type": "new", "discard": true}));
    assert_eq!(camera(&d)["x_ray"], false);
    assert_eq!(camera(&d)["hidden_geometry"], false);
}

#[test]
fn they_are_saved_with_the_camera_at_schema_14_and_only_when_on() {
    let mut d = doc();
    square(&mut d);
    let path = scratch("flags.skr");
    save(&mut d, &path);
    let plain = stored(&path);
    assert_eq!(plain["schema_version"], 5);
    assert!(!plain.to_string().contains("x_ray"));
    assert!(!plain.to_string().contains("hidden_geometry"));
    run(&mut d, json!({"type": "set_x_ray", "on": true}));
    save(&mut d, &path);
    let document = stored(&path);
    assert_eq!(document["schema_version"], 14);
    assert_eq!(document["camera"]["x_ray"], true);
    assert!(document["camera"].get("hidden_geometry").is_none());
    let mut opened = doc();
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    );
    assert_eq!(camera(&opened)["x_ray"], true);
    assert_eq!(camera(&opened)["hidden_geometry"], false);
    run(&mut d, json!({"type": "set_x_ray", "on": false}));
    run(&mut d, json!({"type": "set_hidden_geometry", "on": true}));
    save(&mut d, &path);
    assert_eq!(stored(&path)["camera"]["hidden_geometry"], true);
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap(), "discard": true}),
    );
    assert_eq!(camera(&opened)["hidden_geometry"], true);
    assert_eq!(camera(&opened)["x_ray"], false);
}

#[test]
fn a_scene_stores_both_and_going_to_it_restores_them() {
    let mut d = doc();
    square(&mut d);
    run(&mut d, json!({"type": "set_x_ray", "on": true}));
    run(&mut d, json!({"type": "set_hidden_geometry", "on": true}));
    let out = run(&mut d, json!({"type": "add_scene", "name": "Glass"}));
    let scene = out["changes"]["scenes"][0].clone();
    let view = ask(&d, json!({"type": "scene", "scene": scene}))["view"].clone();
    assert_eq!(view["x_ray"], true);
    assert_eq!(view["hidden_geometry"], true);
    run(&mut d, json!({"type": "set_x_ray", "on": false}));
    run(&mut d, json!({"type": "set_hidden_geometry", "on": false}));
    run(&mut d, json!({"type": "go_to_scene", "scene": scene}));
    assert_eq!(camera(&d)["x_ray"], true);
    assert_eq!(camera(&d)["hidden_geometry"], true);
    // A scene with a view flag needs schema 14 even when nothing is hidden, and keeps it.
    run(&mut d, json!({"type": "set_x_ray", "on": false}));
    run(&mut d, json!({"type": "set_hidden_geometry", "on": false}));
    let path = scratch("scene-flags.skr");
    save(&mut d, &path);
    assert_eq!(stored(&path)["schema_version"], 14);
    let mut opened = doc();
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    );
    let id = ask(&opened, json!({"type": "scenes"}))["scenes"][0]["id"].clone();
    let view = ask(&opened, json!({"type": "scene", "scene": id}))["view"].clone();
    assert_eq!(view["x_ray"], true);
    assert_eq!(view["hidden_geometry"], true);
}

#[test]
fn hidden_entities_can_be_selected_while_hidden_geometry_is_on() {
    let mut d = doc();
    let created = square(&mut d);
    let face = created
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    run(&mut d, json!({"type": "hide", "ids": [face]}));
    let selected = |d: &Document| ask(d, json!({"type": "selection"}))["ids"].clone();
    run(&mut d, json!({"type": "select", "ids": [face]}));
    assert_eq!(selected(&d), json!([]), "hidden: not selectable");
    run(&mut d, json!({"type": "set_hidden_geometry", "on": true}));
    run(&mut d, json!({"type": "select", "ids": [face]}));
    assert_eq!(selected(&d), json!([face]));
    // Hiding again keeps a selected hidden entity while the setting is on.
    run(&mut d, json!({"type": "unhide", "ids": [face]}));
    run(&mut d, json!({"type": "hide", "ids": [face]}));
    assert_eq!(selected(&d), json!([face]));
    // Turned off, it leaves the selection.
    run(&mut d, json!({"type": "set_hidden_geometry", "on": false}));
    assert_eq!(selected(&d), json!([]));
}

#[test]
fn going_to_a_scene_with_hidden_geometry_off_prunes_the_selection() {
    let mut d = doc();
    let created = square(&mut d);
    let face = created
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    let out = run(&mut d, json!({"type": "add_scene", "name": "Plain"}));
    let plain = out["changes"]["scenes"][0].clone();
    run(&mut d, json!({"type": "set_hidden_geometry", "on": true}));
    run(&mut d, json!({"type": "hide", "ids": [face]}));
    run(&mut d, json!({"type": "select", "ids": [face]}));
    let selected = |d: &Document| ask(d, json!({"type": "selection"}))["ids"].clone();
    assert_eq!(selected(&d), json!([face]));
    // The scene changes no tag, so it is no undo step, but the flag goes off.
    run(&mut d, json!({"type": "go_to_scene", "scene": plain}));
    assert_eq!(camera(&d)["hidden_geometry"], false);
    assert_eq!(selected(&d), json!([]));
}
