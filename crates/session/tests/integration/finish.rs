//! Material metalness and roughness, driven through JSON commands and queries.

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

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-finish-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

fn add_steel(d: &mut Document) -> String {
    run(
        d,
        json!({"type": "add_material", "name": "Steel", "color": [200, 200, 210],
               "metalness": 1, "roughness": 0.25}),
    )
    .unwrap()["changes"]["materials"][0]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn finish_defaults_are_listed_and_set_by_add_and_change_material() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_material", "name": "Plain", "color": [1, 2, 3]}),
    )
    .unwrap();
    let steel = add_steel(&mut d);
    let listed = materials(&d);
    assert_eq!(listed[0]["metalness"], 0.0, "always present");
    assert_eq!(listed[0]["roughness"], 1.0);
    assert_eq!(listed[1]["metalness"], 1.0);
    assert_eq!(listed[1]["roughness"], 0.25);

    let revision = ask(&d, json!({"type": "history"}))["revision"].clone();
    run(
        &mut d,
        json!({"type": "change_material", "material": steel, "roughness": 0.5}),
    )
    .unwrap();
    assert_eq!(materials(&d)[1]["roughness"], 0.5);
    assert_eq!(materials(&d)[1]["metalness"], 1.0);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[1]["roughness"], 0.25);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(materials(&d)[1]["roughness"], 0.5);
    assert_ne!(ask(&d, json!({"type": "history"}))["revision"], revision);
}

#[test]
fn finish_outside_zero_to_one_is_refused() {
    let mut d = doc();
    let steel = add_steel(&mut d);
    let revision = ask(&d, json!({"type": "history"}))["revision"].clone();
    for field in ["metalness", "roughness"] {
        for bad in [json!(-0.1), json!(1.5)] {
            let (kind, message) = run(
                &mut d,
                json!({"type": "add_material", "color": [0, 0, 0], field: bad}),
            )
            .unwrap_err();
            assert_eq!(kind, ErrorKind::InvalidInput);
            assert!(message.contains(field), "{message}");
            let (kind, _) = run(
                &mut d,
                json!({"type": "change_material", "material": steel, field: bad}),
            )
            .unwrap_err();
            assert_eq!(kind, ErrorKind::InvalidInput);
            // The ranges are checked on reuse too.
            let (kind, _) = run(
                &mut d,
                json!({"type": "add_material", "name": "Steel", "existing": "reuse",
                       field: bad}),
            )
            .unwrap_err();
            assert_eq!(kind, ErrorKind::InvalidInput);
        }
    }
    assert_eq!(ask(&d, json!({"type": "history"}))["revision"], revision);
    assert_eq!(materials(&d).len(), 1);
}

#[test]
fn add_material_update_sets_the_finish_of_an_existing_material() {
    let mut d = doc();
    add_steel(&mut d);
    run(
        &mut d,
        json!({"type": "add_material", "name": "Steel", "existing": "update",
               "metalness": 0.5}),
    )
    .unwrap();
    assert_eq!(materials(&d)[0]["metalness"], 0.5);
    assert_eq!(materials(&d)[0]["roughness"], 0.25);
}

#[test]
fn a_finished_material_is_saved_as_version_19_and_old_files_load_with_defaults() {
    let mut d = doc();
    let steel = add_steel(&mut d);
    let path = scratch("steel.tnn");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(
        document["schema_version"],
        session::file::FINISH_SCHEMA_VERSION
    );
    assert_eq!(document["schema_version"], 19);
    assert_eq!(document["model"]["materials"][0]["metalness"], 1.0);
    assert_eq!(document["model"]["materials"][0]["roughness"], 0.25);
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(materials(&e), materials(&d));

    // Back to the defaults: the file is as before finish existed.
    run(
        &mut d,
        json!({"type": "change_material", "material": steel, "metalness": 0, "roughness": 1}),
    )
    .unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    let document = stored(&path);
    assert_eq!(
        document["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
    let m = &document["model"]["materials"][0];
    assert!(m.get("metalness").is_none() && m.get("roughness").is_none());
    let mut f = doc();
    run(
        &mut f,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(materials(&f)[0]["metalness"], 0.0);
    assert_eq!(materials(&f)[0]["roughness"], 1.0);
}
