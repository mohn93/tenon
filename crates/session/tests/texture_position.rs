//! Texture positions on faces, driven through JSON commands and queries.

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

fn faces(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .clone()
}

fn revision(d: &Document) -> Value {
    ask(d, json!({"type": "history"}))["revision"].clone()
}

/// A 2 x 1 m rectangle on the ground; returns its face id.
fn rectangle(d: &mut Document) -> Value {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 1, 0]}),
    )
    .unwrap();
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone()
}

/// A fresh path in the test scratch directory.
fn scratch(name: &str) -> PathBuf {
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-texture-position-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// The `document.json` saved in a `.skr` file.
fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

#[test]
fn position_texture_sets_fields_reports_them_and_is_one_undo_step() {
    let mut d = doc();
    let f = rectangle(&mut d);
    let out = run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "offset": [0.5, 0.25],
               "rotation_deg": -90}),
    )
    .unwrap();
    assert_eq!(out["positioned"], 1);
    assert_eq!(out["changes"]["modified"], json!([f]));
    let face = ask(&d, json!({"type": "entity", "id": f}));
    assert_eq!(
        face["texture_position"],
        json!({"offset": [0.5, 0.25], "rotation_deg": 270.0, "scale": [1.0, 1.0]})
    );
    // Fields left out stay.
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "scale": [2, 0.5]}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": f}))["texture_position"],
        json!({"offset": [0.5, 0.25], "rotation_deg": 270.0, "scale": [2.0, 0.5]})
    );
    // The same again changes nothing and makes no undo step.
    let history = ask(&d, json!({"type": "history"}));
    let out = run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "scale": [2, 0.5]}),
    )
    .unwrap();
    assert_eq!(out["positioned"], 0);
    assert_eq!(ask(&d, json!({"type": "history"})), history);

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": f}))["texture_position"]["scale"],
        json!([1.0, 1.0])
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(
        ask(&d, json!({"type": "entity", "id": f}))
            .get("texture_position")
            .is_none(),
        "absent when the texture lies as projected"
    );
    run(&mut d, json!({"type": "redo"})).unwrap();
    run(&mut d, json!({"type": "redo"})).unwrap();
    // Reset clears it, then applies what is given.
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "reset": true, "rotation_deg": 90}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": f}))["texture_position"],
        json!({"offset": [0.0, 0.0], "rotation_deg": 90.0, "scale": [1.0, 1.0]})
    );
    let out = run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "reset": true}),
    )
    .unwrap();
    assert_eq!(out["positioned"], 1);
    assert!(
        ask(&d, json!({"type": "entity", "id": f}))
            .get("texture_position")
            .is_none()
    );
}

#[test]
fn bad_position_texture_commands_are_refused_and_change_nothing() {
    let mut d = doc();
    let f = rectangle(&mut d);
    let edge = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"][0]["id"].clone();
    let before = revision(&d);
    for (command, kind, words) in [
        (
            json!({"type": "position_texture", "ids": [], "rotation_deg": 10}),
            ErrorKind::InvalidInput,
            "at least one face",
        ),
        (
            json!({"type": "position_texture", "ids": [edge], "rotation_deg": 10}),
            ErrorKind::InvalidInput,
            "only faces",
        ),
        (
            json!({"type": "position_texture", "ids": [f]}),
            ErrorKind::InvalidInput,
            "give offset",
        ),
        (
            json!({"type": "position_texture", "ids": [f], "scale": [0, 1]}),
            ErrorKind::InvalidInput,
            "scale",
        ),
        (
            json!({"type": "position_texture", "ids": [f], "offset": [2e6, 0]}),
            ErrorKind::InvalidInput,
            "offset",
        ),
        (
            json!({"type": "position_texture", "ids": ["f999"], "rotation_deg": 10}),
            ErrorKind::NotFound,
            "no such",
        ),
    ] {
        let (k, message) = run(&mut d, command.clone()).unwrap_err();
        assert_eq!(k, kind, "{command}: {message}");
        assert!(message.contains(words), "{command}: {message}");
    }
    assert!(
        serde_json::from_value::<Command>(
            json!({"type": "position_texture", "ids": [f], "angle": 10})
        )
        .is_err(),
        "unknown fields are refused"
    );
    assert_eq!(revision(&d), before);
}

#[test]
fn split_and_copied_faces_keep_their_texture_position() {
    let mut d = doc();
    let f = rectangle(&mut d);
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "rotation_deg": 45}),
    )
    .unwrap();
    // A line across the face splits it; both pieces keep the placement.
    run(
        &mut d,
        json!({"type": "add_edge", "start": [1, 0, 0], "end": [1, 1, 0]}),
    )
    .unwrap();
    let split = faces(&d);
    assert_eq!(split.len(), 2);
    assert!(
        split
            .iter()
            .all(|f| f["texture_position"]["rotation_deg"] == 45.0)
    );
    let ids: Vec<Value> = split.iter().map(|f| f["id"].clone()).collect();
    run(
        &mut d,
        json!({"type": "copy", "ids": ids, "offset": [0, 3, 0]}),
    )
    .unwrap();
    let copied = faces(&d);
    assert_eq!(copied.len(), 4);
    assert!(
        copied
            .iter()
            .all(|f| f["texture_position"]["rotation_deg"] == 45.0)
    );
    // A new face has none.
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [5, 0, 0], "opposite": [6, 1, 0]}),
    )
    .unwrap();
    assert_eq!(
        faces(&d)
            .iter()
            .filter(|f| f.get("texture_position").is_none())
            .count(),
        1
    );
}

#[test]
fn texture_positions_are_saved_as_version_9_and_older_files_open_without_them() {
    let mut d = doc();
    let f = rectangle(&mut d);
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "offset": [0.1, 0.2],
               "rotation_deg": 30, "scale": [2, 3]}),
    )
    .unwrap();
    let path = scratch("positioned.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(document["schema_version"], session::file::SCHEMA_VERSION);
    assert_eq!(document["schema_version"], 9);
    assert_eq!(
        document["model"]["root"]["faces"][0]["texture_position"],
        json!({"offset": [0.1, 0.2], "rotation_deg": 30.0, "scale": [2.0, 3.0]})
    );
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(faces(&e), faces(&d));

    // A file from before texture positions (schema 5, no field) opens without them.
    let mut old = document.clone();
    old["schema_version"] = json!(5);
    old["model"]["root"]["faces"][0]
        .as_object_mut()
        .unwrap()
        .remove("texture_position");
    let old_path = scratch("old.skr");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&old_path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&old).unwrap()).unwrap();
    zip.finish().unwrap();
    let mut o = doc();
    run(
        &mut o,
        json!({"type": "open", "path": old_path.to_str().unwrap()}),
    )
    .unwrap();
    assert!(
        faces(&o)
            .iter()
            .all(|f| f.get("texture_position").is_none())
    );

    // Without placements the model saves as before they existed.
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "reset": true}),
    )
    .unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    assert_eq!(
        stored(&path)["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
}
