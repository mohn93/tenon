//! New, open and save, renaming and purging definitions, driven through JSON commands.

use std::io::Write;
use std::path::{Path, PathBuf};

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
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-files-{}", std::process::id()));
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
        json!({"type": "open", "path": missing.to_str().unwrap(), "discard": true}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("missing.skr"), "{message}");

    let garbage = scratch("garbage.skr");
    std::fs::write(&garbage, b"not a zip").unwrap();
    let (kind, message) = run(
        &mut d,
        json!({"type": "open", "path": garbage.to_str().unwrap(), "discard": true}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("not a tenon file"), "{message}");

    let newer = scratch("newer.skr");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&newer).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"format": "tenon", "schema_version": 99}"#)
        .unwrap();
    zip.finish().unwrap();
    let (kind, message) = run(
        &mut d,
        json!({"type": "open", "path": newer.to_str().unwrap(), "discard": true}),
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
    assert_eq!(doc["schema_version"], session::file::OPAQUE_SCHEMA_VERSION);
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

#[test]
fn a_failed_save_leaves_no_partial_file_and_the_model_modified() {
    let folder = scratch("failed-save");
    let _ = std::fs::remove_dir_all(&folder);
    let dir = folder.join("a-directory.skr");
    std::fs::create_dir_all(&dir).unwrap();
    let mut d = doc();
    model(&mut d);
    let (kind, message) = run(
        &mut d,
        json!({"type": "save", "path": dir.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("a-directory.skr"), "{message}");
    let left: Vec<_> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, ["a-directory.skr"], "the partial file is gone");
    assert!(dir.is_dir(), "the target is untouched");
    let file = ask(&d, json!({"type": "file"}));
    assert_eq!(file["modified"], true);
    assert_eq!(file["path"], Value::Null);
}

/// Saves `model()` to `name`, then rewrites its document with `f`.
fn edited_file(name: &str, f: impl FnOnce(&mut Value)) -> PathBuf {
    let path = scratch(name);
    let mut d = doc();
    model(&mut d);
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let mut document: Value =
        serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    f(&mut document);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    zip.finish().unwrap();
    path
}

/// Opening `path` in a fresh document fails with a file error containing `expected`.
fn open_fails(path: &Path, expected: &str) {
    let mut d = doc();
    let (kind, message) = run(
        &mut d,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File, "{message}");
    assert!(message.contains(expected), "{message:?} lacks {expected:?}");
}

#[test]
fn files_with_out_of_range_numbers_or_versions_do_not_open() {
    let far = edited_file("far-vertex-x.skr", |d| {
        d["model"]["definitions"][0]["mesh"]["vertices"][0]["position"][0] = json!(2.0e6);
    });
    open_fails(&far, "beyond");
    let far = edited_file("far-vertex-z.skr", |d| {
        d["model"]["definitions"][0]["mesh"]["vertices"][1]["position"][2] = json!(-2.0e6);
    });
    open_fails(&far, "beyond");
    let far = edited_file("far-instance.skr", |d| {
        d["model"]["root"]["instances"][0]["transform"][9] = json!(5.0e6);
    });
    open_fails(&far, "beyond");
    let flat = edited_file("flat-instance.skr", |d| {
        d["model"]["root"]["instances"][0]["transform"] = Value::from(vec![0.0; 12]);
    });
    open_fails(&flat, "not invertible");
    let unversioned = edited_file("unversioned.skr", |d| {
        d.as_object_mut().unwrap().remove("schema_version");
    });
    open_fails(&unversioned, "schema version");
    let zero = edited_file("version-zero.skr", |d| d["schema_version"] = json!(0));
    open_fails(&zero, "schema version");
}

#[test]
fn a_header_is_read_from_a_document_object_only() {
    // The header's two values in an array are no header.
    let listed = edited_file("listed.skr", |d| {
        *d = json!([d["format"].clone(), d["schema_version"].clone()]);
    });
    open_fails(&listed, "not a tenon file");
    let unnamed = edited_file("format-null.skr", |d| d["format"] = Value::Null);
    open_fails(&unnamed, "not a tenon file");
    let unversioned = edited_file("version-null.skr", |d| d["schema_version"] = Value::Null);
    open_fails(&unversioned, "schema version");
    let newer = edited_file("version-newer.skr", |d| {
        d["schema_version"] = json!(1_000_000)
    });
    open_fails(&newer, "newer than this app");
}

#[test]
fn new_and_open_refuse_to_drop_unsaved_changes_unless_told_to() {
    let path = scratch("unsaved.skr");
    let mut d = doc();
    model(&mut d);
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_edge", "start": [0, -2, 0], "end": [1, -2, 0]}),
    )
    .unwrap();
    let entities = ask(&d, json!({"type": "entities"}));
    for command in [
        json!({"type": "new"}),
        json!({"type": "open", "path": path.to_str().unwrap()}),
        json!({"type": "new", "discard": false}),
    ] {
        let (kind, message) = run(&mut d, command).unwrap_err();
        assert_eq!(kind, ErrorKind::UnsavedChanges);
        assert!(message.contains("discard"), "{message}");
    }
    assert_eq!(ask(&d, json!({"type": "entities"})), entities);
    assert_eq!(ask(&d, json!({"type": "file"}))["modified"], true);
    let kind = serde_json::to_value(ErrorKind::UnsavedChanges).unwrap();
    assert_eq!(kind, "unsaved_changes");

    run(
        &mut d,
        json!({"type": "open", "path": path.to_str().unwrap(), "discard": true}),
    )
    .unwrap();
    assert_eq!(ask(&d, json!({"type": "file"}))["modified"], false);
    run(
        &mut d,
        json!({"type": "add_edge", "start": [0, -2, 0], "end": [1, -2, 0]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "new", "discard": true})).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["vertices"], 0);
    // A model without changes needs no discard.
    run(
        &mut d,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
}

#[test]
fn coordinates_and_the_view_reopen_bit_for_bit() {
    let path = scratch("bits.skr");
    let mut d = doc();
    // serde_json reads these back one unit in the last place off without float_roundtrip.
    let start = [180.17933438838418, -930.0397635799367, 474.47532350305437];
    let end = [-452.10277274836835, -420.07796691370356, 0.1 + 0.2];
    run(
        &mut d,
        json!({"type": "add_edge", "start": start, "end": end}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [10.000000000000002, -9.87654321012345, 8.1], "target": start, "fov_y_deg": 41.3}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut opened = doc();
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let bits = |d: &Document| -> Vec<[u64; 3]> {
        d.model()
            .vertices()
            .map(|(_, v)| v.pos.to_array().map(f64::to_bits))
            .collect()
    };
    assert_eq!(bits(&opened), bits(&d));
    assert_eq!(
        bits(&opened),
        vec![start.map(f64::to_bits), end.map(f64::to_bits)]
    );
    let view = |d: &Document| {
        [d.camera.eye, d.camera.target]
            .map(|p| p.to_array().map(f64::to_bits))
            .to_vec()
    };
    assert_eq!(view(&opened), view(&d));
}

#[test]
fn a_model_that_has_used_up_its_ids_is_not_saved() {
    let path = edited_file("last-ids.skr", |d| {
        d["model"]["next_id"] = json!(kernel::MAX_ID);
    });
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    // Saving as it is works: no id at or past the limit was given out.
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let saved = ask(&d, json!({"type": "stats"}));
    run(
        &mut d,
        json!({"type": "add_edge", "start": [0, -5, 0], "end": [1, -5, 0]}),
    )
    .unwrap();
    let (kind, message) = run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("more than a file can hold"), "{message}");
    // The file on disk is the one saved before.
    let mut again = doc();
    run(
        &mut again,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(ask(&again, json!({"type": "stats"})), saved);
    assert_eq!(
        ask(&d, json!({"type": "stats"}))["edges"].as_u64().unwrap(),
        saved["edges"].as_u64().unwrap() + 1
    );
    assert!(d.is_modified());
}

#[cfg(unix)]
#[test]
fn a_link_at_the_partial_name_is_not_written_through() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    )
    .unwrap();
    let victim = scratch("partial-victim.txt");
    std::fs::write(&victim, "keep me").unwrap();
    let target = scratch("partial-target.skr");
    let partial = scratch("partial-target.skr.partial");
    std::os::unix::fs::symlink(&victim, &partial).unwrap();
    run(&mut d, json!({"type": "save", "path": target})).unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
    assert!(
        std::fs::symlink_metadata(&partial).is_ok_and(|m| m.file_type().is_symlink()),
        "the link is the user's: left as it is"
    );
    let mut opened = doc();
    run(&mut opened, json!({"type": "open", "path": target})).unwrap();
}

/// A `.skr` archive holding `document.json` with `json` as its text.
fn skr_with(path: &Path, json: &str) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(json.as_bytes()).unwrap();
    zip.finish().unwrap();
}

#[test]
fn open_errors_do_not_quote_the_file() {
    let path = scratch("secret.skr");
    let cases = [
        r#"{"format": "tenon", "schema_version": 9, "camera": "hunter2-SECRET"}"#,
        r#"{"format": "tenon", "schema_version": 9, "hunter2-SECRET": 1}"#,
        r#"{"format": "tenon", "schema_version": 9, "camera": {"eye": [hunter2-SECRET"#,
    ];
    for json in cases {
        skr_with(&path, json);
        let (kind, message) = run(&mut doc(), json!({"type": "open", "path": path})).unwrap_err();
        assert_eq!(kind, ErrorKind::File);
        assert!(!message.contains("hunter2"), "{message}");
        assert!(message.contains("line"), "{message}");
    }
}

#[test]
fn save_does_not_overwrite_another_kind_of_file_unless_told() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    )
    .unwrap();
    let victim = scratch("victim.txt");
    std::fs::write(&victim, "notes").unwrap();
    let (kind, message) = run(&mut d, json!({"type": "save", "path": victim})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("\"overwrite\": true"), "{message}");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "notes");
    // A model file, an empty file and a missing one are fine.
    let model = scratch("model.skr");
    run(&mut d, json!({"type": "save", "path": model})).unwrap();
    run(&mut d, json!({"type": "save", "path": model})).unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    let empty = scratch("empty.skr");
    std::fs::write(&empty, "").unwrap();
    run(&mut d, json!({"type": "save", "path": empty})).unwrap();
    // A zip that is not a model is another kind of file.
    let other = scratch("other.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&other).unwrap());
    zip.start_file("a.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.finish().unwrap();
    assert!(run(&mut d, json!({"type": "save", "path": other})).is_err());
    run(
        &mut d,
        json!({"type": "save", "path": victim, "overwrite": true}),
    )
    .unwrap();
    assert!(std::fs::read(&victim).unwrap().starts_with(b"PK"));
}

#[test]
fn save_refuses_a_folder() {
    let mut d = doc();
    let dir = scratch("a-folder");
    std::fs::create_dir_all(&dir).unwrap();
    let (kind, _) = run(&mut d, json!({"type": "save", "path": dir})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}

#[test]
fn a_user_file_named_like_a_partial_file_is_kept() {
    let mut d = doc();
    model(&mut d);
    let target = scratch("notes.skr");
    let mine = scratch("notes.skr.partial");
    std::fs::write(&mine, "my notes").unwrap();
    run(&mut d, json!({"type": "save", "path": target})).unwrap();
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "my notes");
    let mut opened = doc();
    run(&mut opened, json!({"type": "open", "path": target})).unwrap();
}

#[cfg(unix)]
#[test]
fn a_recovery_copy_is_readable_by_the_user_only() {
    use std::os::unix::fs::PermissionsExt;
    let mut d = doc();
    model(&mut d);
    let copy = scratch("recovery-copy.skr");
    d.write_copy(&copy).unwrap();
    let mode = std::fs::metadata(&copy).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");
}

#[test]
fn a_path_with_a_nul_byte_is_invalid_input() {
    let mut d = doc();
    model(&mut d);
    for c in [
        json!({"type": "save", "path": "a\u{0}b.skr"}),
        json!({"type": "export", "path": "a\u{0}b.stl"}),
        json!({"type": "open", "path": "a\u{0}b.skr", "discard": true}),
        json!({"type": "import", "path": "a\u{0}b.obj"}),
        json!({"type": "add_material", "texture": {"path": "a\u{0}b.png"}}),
    ] {
        let (kind, message) = run(&mut d, c.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{c}");
        assert!(message.contains("NUL"), "{c}: {message}");
    }
}
