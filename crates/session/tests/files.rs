//! New, open and save, renaming and purging definitions, driven through JSON commands.

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

/// A fresh path in the test scratch directory.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("session-files");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// A box made a component "Crate", copied once, plus the view moved.
fn model(d: &mut Document) -> String {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "push_pull", "face": "f8", "distance": 1.0}),
    )
    .unwrap();
    let all = ask(d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    let ids: Vec<Value> = all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect();
    run(
        d,
        json!({"type": "make_component", "ids": ids, "name": "Crate"}),
    )
    .unwrap();
    let crate_ = ask(d, json!({"type": "selection"}))["ids"][0].clone();
    run(
        d,
        json!({"type": "copy", "ids": [crate_], "offset": [3, 0, 0]}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "set_camera", "eye": [10, -10, 8], "target": [1, 1, 0], "fov_y_deg": 40}),
    )
    .unwrap();
    crate_.as_str().unwrap().to_string()
}

#[test]
fn a_saved_model_opens_as_it_was() {
    let path = scratch("round-trip.skr");
    let mut d = doc();
    model(&mut d);
    let entities = ask(&d, json!({"type": "entities"}));
    let definitions = ask(&d, json!({"type": "definitions"}));
    let camera = ask(&d, json!({"type": "camera"}));
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let file = ask(&d, json!({"type": "file"}));
    assert_eq!(file["path"], json!(path.to_str().unwrap()));
    assert_eq!(file["modified"], false);

    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let strip = |mut v: Value| {
        v.as_object_mut().unwrap().remove("revision");
        v
    };
    assert_eq!(strip(ask(&e, json!({"type": "entities"}))), strip(entities));
    assert_eq!(
        strip(ask(&e, json!({"type": "definitions"}))),
        strip(definitions)
    );
    assert_eq!(ask(&e, json!({"type": "camera"})), camera);
    assert_eq!(ask(&e, json!({"type": "history"}))["undo_depth"], 0);
    assert_eq!(ask(&e, json!({"type": "file"}))["modified"], false);
    run(
        &mut e,
        json!({"type": "add_edge", "start": [0, -2, 0], "end": [1, -2, 0]}),
    )
    .unwrap();
    assert_eq!(ask(&e, json!({"type": "file"}))["modified"], true);
    run(&mut e, json!({"type": "save"})).unwrap();
    assert_eq!(ask(&e, json!({"type": "file"}))["modified"], false);
}

#[test]
fn opening_starts_over() {
    let path = scratch("starts-over.skr");
    let mut d = doc();
    let crate_ = model(&mut d);
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    run(&mut d, json!({"type": "open_instance", "id": crate_})).unwrap();
    let before = ask(&d, json!({"type": "file"}))["revision"]
        .as_u64()
        .unwrap();
    run(
        &mut d,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(ask(&d, json!({"type": "context"}))["path"], json!([]));
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([]));
    assert_eq!(
        run(&mut d, json!({"type": "undo"})).unwrap_err().0,
        ErrorKind::NothingToUndo
    );
    let events = ask(&d, json!({"type": "events", "since": before}));
    assert_eq!(
        events["truncated"], true,
        "subscribers must read the model again"
    );
}

#[test]
fn new_starts_an_empty_untitled_model() {
    let path = scratch("before-new.skr");
    let mut d = doc();
    model(&mut d);
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    run(&mut d, json!({"type": "new"})).unwrap();
    let stats = ask(&d, json!({"type": "stats"}));
    assert_eq!(
        [
            &stats["vertices"],
            &stats["instances"],
            &stats["definitions"]
        ],
        [&json!(0), &json!(0), &json!(0)]
    );
    assert_eq!(ask(&d, json!({"type": "file"}))["path"], Value::Null);
    let (kind, _) = run(&mut d, json!({"type": "save"})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "nowhere to save to");
}

#[test]
fn files_that_cannot_be_opened_leave_the_model_alone() {
    let mut d = doc();
    model(&mut d);
    let entities = ask(&d, json!({"type": "entities"}));
    let missing = scratch("missing.skr");
    let (kind, message) = run(
        &mut d,
        json!({"type": "open", "path": missing.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("missing.skr"), "{message}");

    let garbage = scratch("garbage.skr");
    std::fs::write(&garbage, b"not a zip").unwrap();
    let (kind, message) = run(
        &mut d,
        json!({"type": "open", "path": garbage.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("not a sketchup-alt file"), "{message}");

    let newer = scratch("newer.skr");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&newer).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"format": "sketchup-alt", "schema_version": 99}"#)
        .unwrap();
    zip.finish().unwrap();
    let (kind, message) = run(
        &mut d,
        json!({"type": "open", "path": newer.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("newer"), "{message}");

    assert_eq!(ask(&d, json!({"type": "entities"})), entities);
}

#[test]
fn a_file_is_a_zip_with_a_versioned_document() {
    let path = scratch("layout.skr");
    let mut d = doc();
    model(&mut d);
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let doc: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    assert_eq!(doc["format"], session::file::FORMAT);
    assert_eq!(doc["schema_version"], session::file::SCHEMA_VERSION);
    assert_eq!(doc["model"]["definitions"].as_array().unwrap().len(), 1);
}

#[test]
fn file_commands_are_not_allowed_in_atomic_batches() {
    let mut d = doc();
    let commands: Vec<Command> = serde_json::from_value(json!([{"type": "new"}])).unwrap();
    let err = d.execute_batch(commands, true).unwrap_err();
    assert_eq!(err.error.kind, ErrorKind::InvalidInput);
}

#[test]
fn renaming_and_purging_are_undo_steps() {
    let mut d = doc();
    let crate_ = model(&mut d);
    let definition = ask(&d, json!({"type": "entity", "id": crate_}))["definition"].clone();
    run(
        &mut d,
        json!({"type": "rename_definition", "definition": definition, "name": "Box"}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": crate_}))["name"],
        "Box"
    );
    assert_eq!(
        run(
            &mut d,
            json!({"type": "rename_definition", "definition": definition, "name": " "})
        )
        .unwrap_err()
        .0,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        run(
            &mut d,
            json!({"type": "rename_definition", "definition": "d999", "name": "x"})
        )
        .unwrap_err()
        .0,
        ErrorKind::NotFound
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": crate_}))["name"],
        "Crate"
    );

    let instances: Vec<Value> =
        ask(&d, json!({"type": "entities", "kinds": ["instance"]}))["instances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].clone())
            .collect();
    run(&mut d, json!({"type": "erase", "ids": instances})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "stats"}))["definitions"],
        1,
        "the component stays"
    );
    run(&mut d, json!({"type": "purge_definitions"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["definitions"], 0);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["definitions"], 1);
}
