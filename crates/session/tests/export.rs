//! Exporting OBJ, STL and glTF, driven through JSON commands.

use std::path::PathBuf;

use base64::Engine;
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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("session-export");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// A 2 x 2 x 1 box made a component and copied 3 m along x: 24 triangles, 16 corners.
fn two_boxes(d: &mut Document) {
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
}

fn export(d: &mut Document, name: &str) -> Vec<u8> {
    let path = scratch(name);
    run(d, json!({"type": "export", "path": path.to_str().unwrap()})).unwrap();
    std::fs::read(&path).unwrap()
}

#[test]
fn obj_has_shared_corners_triangles_and_y_up() {
    let mut d = doc();
    two_boxes(&mut d);
    let text = String::from_utf8(export(&mut d, "boxes.obj")).unwrap();
    let count = |prefix: &str| text.lines().filter(|l| l.starts_with(prefix)).count();
    assert_eq!(count("v "), 16);
    assert_eq!(count("vn "), 6, "six face directions");
    assert_eq!(count("f "), 24);
    // The boxes' tops (z = 1) are at y = 1 in a Y-up file; nothing is above it.
    let ys: Vec<f64> = text
        .lines()
        .filter(|l| l.starts_with("v "))
        .map(|l| l.split(' ').nth(2).unwrap().parse().unwrap())
        .collect();
    assert_eq!(ys.iter().cloned().fold(f64::MIN, f64::max), 1.0);
    assert_eq!(ys.iter().cloned().fold(f64::MAX, f64::min), 0.0);
}

#[test]
fn stl_is_binary_with_one_record_per_triangle() {
    let mut d = doc();
    two_boxes(&mut d);
    let bytes = export(&mut d, "boxes.stl");
    assert_eq!(bytes.len(), 84 + 50 * 24);
    assert_eq!(u32::from_le_bytes(bytes[80..84].try_into().unwrap()), 24);
}

#[test]
fn glb_holds_a_document_and_a_buffer() {
    let mut d = doc();
    two_boxes(&mut d);
    let bytes = export(&mut d, "boxes.glb");
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[0..4], b"glTF");
    assert_eq!(word(4), 2);
    assert_eq!(word(8), bytes.len());
    let json_len = word(12);
    assert_eq!(&bytes[16..20], b"JSON");
    let doc: Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
    assert_eq!(doc["asset"]["version"], "2.0");
    assert_eq!(doc["accessors"][0]["count"], 72);
    assert_eq!(doc["accessors"][0]["max"], json!([5.0, 1.0, 0.0]));
    let bin = 20 + json_len;
    assert_eq!(&bytes[bin + 4..bin + 8], b"BIN\0");
    assert_eq!(word(bin), 72 * 12 * 2);
    assert_eq!(json_len % 4, 0);
}

#[test]
fn gltf_embeds_its_buffer() {
    let mut d = doc();
    two_boxes(&mut d);
    let doc: Value = serde_json::from_slice(&export(&mut d, "boxes.gltf")).unwrap();
    let uri = doc["buffers"][0]["uri"].as_str().unwrap();
    let data = uri
        .strip_prefix("data:application/octet-stream;base64,")
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    assert_eq!(
        bytes.len(),
        doc["buffers"][0]["byteLength"].as_u64().unwrap() as usize
    );
    assert_eq!(bytes.len(), 72 * 12 * 2);
}

#[test]
fn the_format_comes_from_the_command_or_the_extension() {
    let mut d = doc();
    two_boxes(&mut d);
    let path = scratch("named.bin");
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains(".obj"), "{message}");
    run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap(), "format": "stl"}),
    )
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap().len(), 84 + 50 * 24);
    let upper = scratch("upper.OBJ");
    run(
        &mut d,
        json!({"type": "export", "path": upper.to_str().unwrap()}),
    )
    .unwrap();
    assert!(std::fs::read_to_string(&upper).unwrap().contains("\nf "));
}

#[test]
fn exporting_changes_nothing_and_needs_faces() {
    let mut d = doc();
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": scratch("empty.obj").to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("no faces"), "{message}");
    two_boxes(&mut d);
    let before = ask(&d, json!({"type": "file"}));
    export(&mut d, "unchanged.obj");
    assert_eq!(
        ask(&d, json!({"type": "file"})),
        before,
        "not an edit, not a save"
    );
    let missing_dir = scratch("no-such-dir").join("x.obj");
    let (kind, _) = run(
        &mut d,
        json!({"type": "export", "path": missing_dir.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
}

#[test]
fn an_open_instance_does_not_change_what_is_exported() {
    let mut d = doc();
    two_boxes(&mut d);
    let root = export(&mut d, "from-root.obj");
    let crate_ =
        ask(&d, json!({"type": "entities", "kinds": ["instance"]}))["instances"][1]["id"].clone();
    run(&mut d, json!({"type": "open_instance", "id": crate_})).unwrap();
    assert_eq!(export(&mut d, "from-inside.obj"), root);
}
