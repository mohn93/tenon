//! Exporting OBJ, STL and glTF, driven through JSON commands.

use std::path::PathBuf;

use base64::Engine;
use glam::{DVec2, DVec3};
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
fn a_turned_instance_is_written_turned() {
    let mut d = doc();
    two_boxes(&mut d);
    let instances = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    let second = instances["instances"][1]["id"].clone();
    run(
        &mut d,
        json!({"type": "rotate", "ids": [second], "center": [3, 0, 0], "angle": 45}),
    )
    .unwrap();
    let text = String::from_utf8(export(&mut d, "turned.obj")).unwrap();
    let count = |prefix: &str| text.lines().filter(|l| l.starts_with(prefix)).count();
    assert_eq!(
        count("vn "),
        6 + 4,
        "the turned box's sides face four new ways"
    );
    assert_eq!(count("f "), 24);
    // Y up: (x, z, -y). Every turned side lies flat in the model, at 45 degrees to x.
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let diagonal = text
        .lines()
        .filter_map(|l| l.strip_prefix("vn "))
        .map(|l| {
            l.split(' ')
                .map(|c| c.parse::<f64>().unwrap())
                .collect::<Vec<_>>()
        })
        .filter(|n| (n[0].abs() - half).abs() < 1e-9 && n[1] == 0.0)
        .count();
    assert_eq!(diagonal, 4);
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
    assert_eq!(doc["materials"][0]["doubleSided"], true);
    assert_eq!(doc["meshes"][0]["primitives"][0]["material"], 0);
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
    assert_eq!(doc["materials"][0]["doubleSided"], true);
    assert_eq!(doc["meshes"][0]["primitives"][0]["material"], 0);
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

#[test]
fn obj_has_no_negative_zero() {
    let mut d = doc();
    two_boxes(&mut d);
    let text = String::from_utf8(export(&mut d, "zeros.obj")).unwrap();
    assert!(!text.contains("-0 ") && !text.contains("-0\n"), "{text}");
}

#[test]
fn skr_paths_and_the_open_models_file_are_refused() {
    let mut d = doc();
    two_boxes(&mut d);
    let model = scratch("model.skr");
    run(
        &mut d,
        json!({"type": "save", "path": model.to_str().unwrap()}),
    )
    .unwrap();
    let saved = std::fs::read(&model).unwrap();
    let before = ask(&d, json!({"type": "file"}));
    for path in [model.clone(), scratch("other.SKR")] {
        let (kind, message) = run(
            &mut d,
            json!({"type": "export", "path": path.to_str().unwrap(), "format": "obj"}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    }
    assert_eq!(std::fs::read(&model).unwrap(), saved, "the model is intact");
    assert_eq!(ask(&d, json!({"type": "file"})), before);
}

#[test]
fn the_open_models_file_is_refused_even_without_the_skr_extension() {
    let mut d = doc();
    two_boxes(&mut d);
    let model = scratch("model.dat");
    run(
        &mut d,
        json!({"type": "save", "path": model.to_str().unwrap()}),
    )
    .unwrap();
    let saved = std::fs::read(&model).unwrap();
    let dotted = model
        .parent()
        .unwrap()
        .join("..")
        .join("session-export")
        .join("model.dat");
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": dotted.to_str().unwrap(), "format": "obj"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    assert_eq!(std::fs::read(&model).unwrap(), saved);
}

/// The area and count of the triangles in a binary STL.
fn stl_totals(bytes: &[u8]) -> (usize, f64) {
    let n = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    let f = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as f64;
    let mut area = 0.0;
    for k in 0..n {
        let base = 84 + 50 * k + 12;
        let v = |i: usize| DVec3::new(f(base + 12 * i), f(base + 12 * i + 4), f(base + 12 * i + 8));
        area += (v(1) - v(0)).cross(v(2) - v(0)).length() / 2.0;
    }
    (n, area)
}

#[test]
fn a_face_with_a_hole_in_nested_groups_exports_whole_and_undo_still_works() {
    let mut d = doc();
    // A 4 x 4 plate with a 2 x 2 hole, pulled up 2 m into a hollow tube.
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 4, 0]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [1, 1, 0], "opposite": [3, 3, 0]}),
    )
    .unwrap();
    let faces = ask(&d, json!({"type": "entities", "kinds": ["face"]}));
    let ring = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| (f["area"].as_f64().unwrap() - 12.0).abs() < 1e-9)
        .expect("the plate with a hole")["id"]
        .clone();
    run(
        &mut d,
        json!({"type": "push_pull", "face": ring, "distance": 2.0}),
    )
    .unwrap();
    // Group everything, then group that group: the tube sits two levels down.
    let all = ask(&d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    let ids: Vec<Value> = all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect();
    run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let inner = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    run(&mut d, json!({"type": "make_group", "ids": [inner]})).unwrap();
    let outer = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    run(
        &mut d,
        json!({"type": "move", "ids": [outer], "offset": [10, 0, 0]}),
    )
    .unwrap();
    let last_edit = ask(&d, json!({"type": "file"}));
    let revision = d.revision();

    let (count, area) = stl_totals(&export(&mut d, "tube.stl"));
    // Top and bottom rings 12 + 12, outer walls 32, inner walls 16, the hole's own face 4.
    assert!((area - 76.0).abs() < 1e-3, "{area}");
    assert_eq!(count, 34);

    assert_eq!(d.revision(), revision);
    assert_eq!(ask(&d, json!({"type": "file"})), last_edit);
    run(&mut d, json!({"type": "undo"})).unwrap();
    let top = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    assert_eq!(
        top["instances"][0]["transform"][9], 0.0,
        "undo undid the move, not an export: {top}"
    );
}
