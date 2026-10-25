//! Opening, importing, saving and exporting as bytes (the web's file access) gives what the
//! path-based commands give.

use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, ExportFormat, ImportOptions, Query};

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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-bytes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// A box, with the view moved.
fn boxed(d: &mut Document) {
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
    run(
        d,
        json!({"type": "set_camera", "eye": [10, -10, 8], "target": [1, 1, 0], "fov_y_deg": 40}),
    )
    .unwrap();
}

/// What a model holds, for comparing two documents.
fn contents(d: &Document) -> (Value, Value) {
    (
        ask(d, json!({"type": "entities", "kinds": ["edge", "face"]})),
        ask(d, json!({"type": "camera"})),
    )
}

fn path_str(p: &std::path::Path) -> String {
    p.to_str().unwrap().to_string()
}

#[test]
fn save_bytes_is_the_file_save_writes() {
    let mut d = doc();
    boxed(&mut d);
    let path = scratch("saved.skr");
    run(&mut d, json!({"type": "save", "path": path_str(&path)})).unwrap();
    let file = std::fs::read(&path).unwrap();
    let bytes = d.save_bytes().unwrap();
    assert_eq!(bytes, file);
}

#[test]
fn save_bytes_marks_the_model_saved() {
    let mut d = doc();
    boxed(&mut d);
    assert!(d.is_modified());
    d.save_bytes().unwrap();
    assert!(!d.is_modified());
}

#[test]
fn open_bytes_equals_open_of_the_path() {
    let mut d = doc();
    boxed(&mut d);
    let path = scratch("model.skr");
    run(&mut d, json!({"type": "save", "path": path_str(&path)})).unwrap();
    let bytes = std::fs::read(&path).unwrap();

    let mut by_path = doc();
    run(
        &mut by_path,
        json!({"type": "open", "path": path_str(&path)}),
    )
    .unwrap();
    let mut by_bytes = doc();
    let report = by_bytes.open_bytes("model.skr", &bytes, false).unwrap();
    assert!(report.is_none());
    assert_eq!(contents(&by_bytes), contents(&by_path));
    assert!(!by_bytes.is_modified());
    assert!(by_bytes.path().is_none());
    // Saved again, the bytes are the same.
    assert_eq!(by_bytes.save_bytes().unwrap(), bytes);
}

#[test]
fn open_bytes_refuses_unsaved_changes_unless_discarded() {
    let mut d = doc();
    boxed(&mut d);
    let bytes = d.save_bytes().unwrap();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [5, 5, 0], "opposite": [6, 6, 0]}),
    )
    .unwrap();
    let err = d.open_bytes("a.skr", &bytes, false).unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnsavedChanges);
    d.open_bytes("a.skr", &bytes, true).unwrap();
}

#[test]
fn open_bytes_of_something_else_fails_and_changes_nothing() {
    let mut d = doc();
    boxed(&mut d);
    let before = contents(&d);
    let err = d.open_bytes("a.skr", b"not an archive", true).unwrap_err();
    assert_eq!(err.kind, ErrorKind::File);
    assert!(err.message.starts_with("a.skr: "), "{}", err.message);
    assert_eq!(contents(&d), before);
}

#[test]
fn a_skp_opens_from_bytes_as_a_new_model() {
    let mut d = doc();
    boxed(&mut d);
    let exported = d.export_bytes(ExportFormat::Skp, "box", None).unwrap();

    // By extension and by header, with a name that says nothing.
    let mut named = doc();
    let report = named.open_bytes("box.skp", &exported.bytes, false).unwrap();
    let mut sniffed = doc();
    let sniffed_report = sniffed
        .open_bytes("box.dat", &exported.bytes, false)
        .unwrap();
    assert!(report.is_some());
    assert_eq!(report, sniffed_report);

    let path = scratch("box.skp");
    std::fs::write(&path, &exported.bytes).unwrap();
    let mut by_path = doc();
    let out = run(
        &mut by_path,
        json!({"type": "open", "path": path_str(&path)}),
    )
    .unwrap();
    assert_eq!(
        out["import"]["faces"],
        serde_json::to_value(report.as_ref().unwrap().faces).unwrap()
    );
    assert_eq!(contents(&named), contents(&by_path));
    assert!(named.path().is_none() && named.imported_from().is_none());
}

const OBJ: &str = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";

#[test]
fn import_bytes_equals_import_of_the_path() {
    let path = scratch("quad.obj");
    std::fs::write(&path, OBJ).unwrap();
    let mut by_path = doc();
    let expected = run(
        &mut by_path,
        json!({"type": "import", "path": path_str(&path)}),
    )
    .unwrap();

    let mut by_bytes = doc();
    let outcome = by_bytes
        .import_bytes(
            "quad.obj",
            OBJ.as_bytes().to_vec(),
            &ImportOptions::default(),
        )
        .unwrap();
    let got = serde_json::to_value(&outcome).unwrap();
    assert_eq!(got["import"], expected["import"]);
    assert_eq!(contents(&by_bytes), contents(&by_path));
    // One undo step, as the command makes.
    by_bytes
        .execute(serde_json::from_value(json!({"type": "undo"})).unwrap())
        .unwrap();
    assert_eq!(
        ask(&by_bytes, json!({"type": "entities", "kinds": ["face"]}))["faces"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn import_bytes_reads_no_material_library_and_checks_the_name() {
    let mut d = doc();
    // A library beside the working folder, which a path import of the same text would find.
    let lib = format!("bytes-probe-{}.mtl", std::process::id());
    std::fs::write(&lib, "newmtl red\nKd 1 0 0\n").unwrap();
    let obj = format!("mtllib {lib}\nusemtl red\nv 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\n");
    let outcome = d
        .import_bytes(
            "quad.obj",
            obj.clone().into_bytes(),
            &ImportOptions::default(),
        )
        .unwrap();
    // The same text next to its library does make the material, so the library was findable.
    let beside = std::env::current_dir().unwrap().join("bytes-probe.obj");
    std::fs::write(&beside, &obj).unwrap();
    let mut by_path = doc();
    let found = run(
        &mut by_path,
        json!({"type": "import", "path": path_str(&beside)}),
    );
    let _ = std::fs::remove_file(&beside);
    let _ = std::fs::remove_file(&lib);
    assert_eq!(found.unwrap()["import"]["materials"], 1);
    assert_eq!(
        serde_json::to_value(&outcome).unwrap()["import"]["materials"],
        0
    );
    let err = d
        .import_bytes("model.skr", vec![1], &ImportOptions::default())
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    let err = d
        .import_bytes(
            "quad.txt",
            OBJ.as_bytes().to_vec(),
            &ImportOptions::default(),
        )
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
}

#[test]
fn export_bytes_equals_the_exported_file() {
    let mut d = doc();
    boxed(&mut d);
    for (format, ext) in [
        (ExportFormat::Obj, "obj"),
        (ExportFormat::Stl, "stl"),
        (ExportFormat::Glb, "glb"),
        (ExportFormat::Gltf, "gltf"),
        (ExportFormat::Skp, "skp"),
        (ExportFormat::Svg, "svg"),
        (ExportFormat::ThreeMf, "3mf"),
    ] {
        let path = scratch(&format!("exp.{ext}"));
        let expected = run(&mut d, json!({"type": "export", "path": path_str(&path)})).unwrap();
        let got = d.export_bytes(format, "exp", None).unwrap();
        assert_eq!(got.bytes, std::fs::read(&path).unwrap(), "{ext}");
        assert!(got.side_files.is_empty(), "{ext}");
        assert_eq!(
            serde_json::to_value(&got.report).unwrap(),
            expected["export"],
            "{ext}"
        );
    }
}

#[test]
fn export_bytes_of_a_painted_obj_returns_its_library() {
    let mut d = doc();
    boxed(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_material", "name": "Red", "color": [200, 30, 30]}),
    )
    .unwrap();
    let material = out["changes"]["materials"][0].clone();
    let faces = ask(&d, json!({"type": "entities", "kinds": ["face"]}));
    let ids: Vec<Value> = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .collect();
    run(
        &mut d,
        json!({"type": "paint", "ids": ids, "material": material}),
    )
    .unwrap();
    let path = scratch("paint.obj");
    run(&mut d, json!({"type": "export", "path": path_str(&path)})).unwrap();
    let got = d.export_bytes(ExportFormat::Obj, "paint", None).unwrap();
    assert_eq!(got.bytes, std::fs::read(&path).unwrap());
    assert_eq!(got.side_files.len(), 1);
    assert_eq!(got.side_files[0].0, "paint.mtl");
    assert_eq!(
        got.side_files[0].1,
        std::fs::read(path.with_extension("mtl")).unwrap()
    );
}

#[test]
fn export_bytes_refuses_what_export_refuses() {
    let mut d = doc();
    assert_eq!(
        d.export_bytes(ExportFormat::Obj, "x", None)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    boxed(&mut d);
    assert_eq!(
        d.export_bytes(ExportFormat::Obj, "x", Some(100.0))
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn export_bytes_names_the_library_after_the_stem_and_refuses_paths() {
    let mut d = doc();
    boxed(&mut d);
    assert!(d.export_bytes(ExportFormat::Obj, "", None).is_ok());
    for bad in ["../x", "a/b", "a\\b"] {
        assert_eq!(
            d.export_bytes(ExportFormat::Obj, bad, None)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidInput,
            "{bad}"
        );
    }
}
