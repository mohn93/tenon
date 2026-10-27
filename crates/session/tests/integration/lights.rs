//! Glowing materials and the lighting mode, driven through JSON commands and queries.

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

fn materials(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "materials"}))["materials"]
        .as_array()
        .unwrap()
        .clone()
}

/// A fresh path in the test scratch directory.
fn scratch(name: &str) -> PathBuf {
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-lights-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// The `document.json` saved in a `.tnn` file.
fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

fn add_lamp(d: &mut Document, emission: f64) -> String {
    run(
        d,
        json!({"type": "add_material", "name": "Lamp", "color": [255, 220, 160],
               "emission": emission}),
    )
    .unwrap()["changes"]["materials"][0]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn emission_is_added_changed_undone_and_listed() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_material", "name": "Plain", "color": [10, 20, 30]}),
    )
    .unwrap();
    let lamp = add_lamp(&mut d, 2.5);
    let listed = materials(&d);
    assert_eq!(listed[0]["emission"], 0.0, "always present");
    assert_eq!(listed[1]["emission"], 2.5);

    run(
        &mut d,
        json!({"type": "change_material", "material": lamp, "emission": 10}),
    )
    .unwrap();
    assert_eq!(materials(&d)[1]["emission"], 10.0);
    // Other changes keep it.
    run(
        &mut d,
        json!({"type": "change_material", "material": lamp, "color": [255, 255, 255]}),
    )
    .unwrap();
    assert_eq!(materials(&d)[1]["emission"], 10.0);
    run(&mut d, json!({"type": "undo"})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[1]["emission"], 2.5);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(materials(&d)[1]["emission"], 10.0);
}

#[test]
fn emission_outside_zero_to_ten_is_refused() {
    let mut d = doc();
    let lamp = add_lamp(&mut d, 1.0);
    let revision = ask(&d, json!({"type": "history"}))["revision"].clone();
    for bad in [json!(-1), json!(10.5), json!(1e9)] {
        let (kind, message) = run(
            &mut d,
            json!({"type": "add_material", "name": "Other", "color": [0, 0, 0],
                   "emission": bad}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput);
        assert!(message.contains("emission"), "{message}");
        let (kind, _) = run(
            &mut d,
            json!({"type": "change_material", "material": lamp, "emission": bad}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput);
    }
    assert!(
        serde_json::from_value::<Command>(
            json!({"type": "add_material", "color": [0, 0, 0], "emission": "bright"})
        )
        .is_err()
    );
    assert_eq!(ask(&d, json!({"type": "history"}))["revision"], revision);
    assert_eq!(materials(&d).len(), 1);
}

#[test]
fn glowing_materials_are_saved_as_version_8() {
    let mut d = doc();
    let lamp = add_lamp(&mut d, 3.0);
    let path = scratch("glow.tnn");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(
        document["schema_version"],
        session::file::LIGHTS_SCHEMA_VERSION
    );
    assert_eq!(document["schema_version"], 8);
    assert_eq!(document["model"]["materials"][0]["emission"], 3.0);
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(materials(&e), materials(&d));

    // Without glow the model saves as before emission existed.
    run(
        &mut d,
        json!({"type": "change_material", "material": lamp, "emission": 0}),
    )
    .unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    let document = stored(&path);
    assert_eq!(
        document["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
    assert!(document["model"]["materials"][0].get("emission").is_none());
}

#[test]
fn lighting_is_a_view_setting_reported_with_the_camera() {
    let mut d = doc();
    assert_eq!(ask(&d, json!({"type": "camera"}))["lighting"], "day");
    let history = ask(&d, json!({"type": "history"}));
    let set: Command = serde_json::from_value(json!({"type": "set_lighting", "mode": "night"}))
        .expect("valid command json");
    assert!(set.is_camera() && !set.changes_model());
    d.execute(set).unwrap();
    assert_eq!(ask(&d, json!({"type": "camera"}))["lighting"], "night");
    // Not an undo step, not a change to the model.
    assert_eq!(ask(&d, json!({"type": "history"})), history);
    assert!(!d.is_modified());
    run(&mut d, json!({"type": "set_lighting", "mode": "day"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "camera"}))["lighting"], "day");
    assert!(
        serde_json::from_value::<Command>(json!({"type": "set_lighting", "mode": "dusk"})).is_err()
    );
    assert!(serde_json::from_value::<Command>(json!({"type": "set_lighting"})).is_err());
}

#[test]
fn a_failed_atomic_batch_puts_the_lighting_back() {
    let mut d = doc();
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "set_lighting", "mode": "night"},
        {"type": "erase", "ids": ["e999"]}
    ]))
    .unwrap();
    assert!(d.execute_batch(commands, true).unwrap_err().rolled_back);
    assert_eq!(ask(&d, json!({"type": "camera"}))["lighting"], "day");
}

#[test]
fn night_lighting_is_saved_with_the_camera_as_version_8() {
    let mut d = doc();
    run(&mut d, json!({"type": "set_lighting", "mode": "night"})).unwrap();
    let path = scratch("night.tnn");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(document["schema_version"], 8);
    assert_eq!(document["camera"]["lighting"], "night");

    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(ask(&e, json!({"type": "camera"}))["lighting"], "night");
    // A new model is lit by day.
    run(&mut e, json!({"type": "new"})).unwrap();
    assert_eq!(ask(&e, json!({"type": "camera"}))["lighting"], "day");

    // By day nothing is written: the file is as before lighting existed.
    run(&mut d, json!({"type": "set_lighting", "mode": "day"})).unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    let document = stored(&path);
    assert_eq!(
        document["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
    assert!(document["camera"].get("lighting").is_none());
    // An older file opens lit by day, even into a document lit by night.
    run(&mut e, json!({"type": "set_lighting", "mode": "night"})).unwrap();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(ask(&e, json!({"type": "camera"}))["lighting"], "day");
}

#[test]
fn studio_lighting_is_set_reported_and_not_an_edit() {
    let mut d = doc();
    let history = ask(&d, json!({"type": "history"}));
    run(&mut d, json!({"type": "set_lighting", "mode": "studio"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "camera"}))["lighting"], "studio");
    assert_eq!(ask(&d, json!({"type": "history"})), history);
    assert!(!d.is_modified());
    run(&mut d, json!({"type": "set_lighting", "mode": "night"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "camera"}))["lighting"], "night");
}

#[test]
fn studio_lighting_is_saved_as_version_20_and_only_then() {
    let mut d = doc();
    run(&mut d, json!({"type": "set_lighting", "mode": "studio"})).unwrap();
    let path = scratch("studio.tnn");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(
        document["schema_version"],
        session::file::STUDIO_SCHEMA_VERSION
    );
    assert_eq!(session::file::STUDIO_SCHEMA_VERSION, 20);
    assert_eq!(document["camera"]["lighting"], "studio");

    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(ask(&e, json!({"type": "camera"}))["lighting"], "studio");

    // Night alone keeps the version it always had.
    run(&mut d, json!({"type": "set_lighting", "mode": "night"})).unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    assert_eq!(stored(&path)["schema_version"], 8);
}

#[test]
fn a_scene_stores_studio_lighting_and_the_file_keeps_it() {
    let mut d = doc();
    run(&mut d, json!({"type": "set_lighting", "mode": "studio"})).unwrap();
    let out = run(&mut d, json!({"type": "add_scene"})).unwrap();
    let scene = out["changes"]["scenes"][0].as_str().unwrap().to_string();
    let detail = ask(&d, json!({"type": "scene", "scene": scene}));
    assert_eq!(detail["view"]["lighting"], "studio");

    // Back to day, then the scene brings studio back.
    run(&mut d, json!({"type": "set_lighting", "mode": "day"})).unwrap();
    run(&mut d, json!({"type": "go_to_scene", "scene": scene})).unwrap();
    assert_eq!(ask(&d, json!({"type": "camera"}))["lighting"], "studio");

    // A file whose only studio is in a scene still needs version 20.
    run(&mut d, json!({"type": "set_lighting", "mode": "day"})).unwrap();
    let path = scratch("studio_scene.tnn");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&path)["schema_version"], 20);
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    run(&mut e, json!({"type": "go_to_scene", "scene": scene})).unwrap();
    assert_eq!(ask(&e, json!({"type": "camera"}))["lighting"], "studio");
}
