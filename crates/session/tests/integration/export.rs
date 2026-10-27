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
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-export-{}", std::process::id()));
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
    let dir = model.parent().unwrap();
    let dotted = dir
        .join("..")
        .join(dir.file_name().unwrap())
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

/// `two_boxes` with the first crate painted with a new material named `name` and `color`.
fn painted_boxes(d: &mut Document, name: &str, color: [u8; 3]) {
    two_boxes(d);
    let out = run(
        d,
        json!({"type": "add_material", "name": name, "color": color}),
    )
    .unwrap();
    let material = out["changes"]["materials"][0].clone();
    let instances = ask(d, json!({"type": "entities", "kinds": ["instance"]}));
    let first = instances["instances"][0]["id"].clone();
    run(
        d,
        json!({"type": "paint", "ids": [first], "material": material}),
    )
    .unwrap();
}

#[test]
fn obj_of_painted_faces_names_a_material_library_beside_it() {
    let mut d = doc();
    two_boxes(&mut d);
    let mtl = scratch("plain-boxes.mtl");
    let obj = String::from_utf8(export(&mut d, "plain-boxes.obj")).unwrap();
    assert!(!obj.contains("mtllib") && !obj.contains("usemtl"));
    assert!(!mtl.exists(), "no library without painted faces");

    let mut d = doc();
    painted_boxes(&mut d, "Red brick", [255, 0, 0]);
    let mtl = scratch("painted-boxes.mtl");
    let obj = String::from_utf8(export(&mut d, "painted-boxes.obj")).unwrap();
    assert!(obj.contains("mtllib painted-boxes.mtl\n"), "{obj}");
    let uses: Vec<&str> = obj.lines().filter(|l| l.starts_with("usemtl")).collect();
    assert_eq!(uses, ["usemtl Red_brick", "usemtl default"]);
    assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 24);
    let library = std::fs::read_to_string(&mtl).unwrap();
    assert!(
        library.contains("newmtl Red_brick\nKd 1.000000 0.000000 0.000000\n"),
        "{library}"
    );
    assert!(library.contains("newmtl default\n"), "{library}");

    // The library must not overwrite the OBJ itself.
    let path = scratch("painted.mtl");
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap(), "format": "obj"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    assert!(!path.exists());
    // Nor when the paths differ only in case (one file on case-blind file systems).
    let upper = scratch("painted-upper.MTL");
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": upper.to_str().unwrap(), "format": "obj"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
    assert!(message.contains("not free"), "{message}");
    assert!(!upper.exists());
}

#[test]
fn gltf_has_one_primitive_and_material_per_color() {
    let mut d = doc();
    painted_boxes(&mut d, "Clay", [128, 64, 255]);
    let doc: Value = serde_json::from_slice(&export(&mut d, "painted.gltf")).unwrap();
    let primitives = doc["meshes"][0]["primitives"].as_array().unwrap();
    assert_eq!(primitives.len(), 2);
    assert_eq!(doc["materials"][0]["name"], "Clay");
    assert_eq!(doc["materials"][1]["name"], "default");
    assert!(doc["materials"][1]["pbrMetallicRoughness"]["baseColorFactor"].is_null());
    // glTF colors are linear: sRGB 128 is about 0.216.
    let factor = doc["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"].clone();
    let f: Vec<f64> = factor
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    assert!(
        (f[0] - 0.2158).abs() < 1e-3 && (f[1] - 0.0513).abs() < 1e-3,
        "{f:?}"
    );
    assert_eq!((f[2], f[3]), (1.0, 1.0));
    assert!(
        doc["materials"][0]["alphaMode"].is_null(),
        "opaque is the default mode"
    );
    for (k, p) in primitives.iter().enumerate() {
        assert_eq!(p["material"], k);
        let pos = &doc["accessors"][p["attributes"]["POSITION"].as_u64().unwrap() as usize];
        assert_eq!(pos["count"], 36);
        assert_eq!(pos["byteOffset"], 36 * 12 * k);
    }
    // The second box (the unpainted one) is the default primitive: x from 3 to 5.
    assert_eq!(doc["accessors"][2]["min"][0], 3.0);
}

#[test]
fn the_outcome_counts_triangles_and_faces_left_out() {
    let mut d = doc();
    two_boxes(&mut d);
    let path = scratch("counted.stl");
    let out = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(out["export"], json!({"triangles": 24, "skipped_faces": 0}));

    // A face whose hole is its own outer loop passes validation but gives no triangles.
    let model = json!({
        "next_id": 20,
        "root": {
            "vertices": [
                {"id": 1, "position": [0, 0, 0]}, {"id": 2, "position": [1, 0, 0]},
                {"id": 3, "position": [1, 1, 0]}, {"id": 4, "position": [0, 1, 0]},
                {"id": 5, "position": [0, 2, 0]}
            ],
            "edges": [
                {"id": 6, "vertices": [1, 2]}, {"id": 7, "vertices": [2, 3]},
                {"id": 8, "vertices": [3, 4]}, {"id": 9, "vertices": [4, 1]},
                {"id": 10, "vertices": [3, 5]}, {"id": 11, "vertices": [5, 4]}
            ],
            "faces": [
                {"id": 12, "outer": [1, 2, 3, 4], "holes": [[1, 4, 3, 2]], "normal": [0, 0, 1]},
                {"id": 13, "outer": [4, 3, 5], "holes": [], "normal": [0, 0, 1]}
            ],
            "instances": []
        },
        "definitions": []
    });
    let skr = scratch("empty-face.skr");
    let document = json!({
        "format": "tenon",
        "schema_version": session::file::SCHEMA_VERSION,
        "camera": {"eye": [5, -5, 5], "target": [0, 0, 0], "fov_y_deg": 45},
        "model": model
    });
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&skr).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, &serde_json::to_vec(&document).unwrap()).unwrap();
    zip.finish().unwrap();
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "open", "path": skr.to_str().unwrap()}),
    )
    .unwrap();
    let path = scratch("left-out.obj");
    let out = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(out["export"], json!({"triangles": 1, "skipped_faces": 1}));
    let obj = std::fs::read_to_string(&path).unwrap();
    assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 1);
}

#[test]
fn a_model_whose_every_face_fails_to_triangulate_says_so() {
    let model = json!({
        "next_id": 20,
        "root": {
            "vertices": [
                {"id": 1, "position": [0, 0, 0]}, {"id": 2, "position": [1, 0, 0]},
                {"id": 3, "position": [1, 1, 0]}, {"id": 4, "position": [0, 1, 0]}
            ],
            "edges": [
                {"id": 6, "vertices": [1, 2]}, {"id": 7, "vertices": [2, 3]},
                {"id": 8, "vertices": [3, 4]}, {"id": 9, "vertices": [4, 1]}
            ],
            "faces": [
                {"id": 12, "outer": [1, 2, 3, 4], "holes": [[1, 4, 3, 2]], "normal": [0, 0, 1]}
            ],
            "instances": []
        },
        "definitions": []
    });
    let skr = scratch("all-left-out.skr");
    let document = json!({
        "format": "tenon",
        "schema_version": session::file::SCHEMA_VERSION,
        "camera": {"eye": [5, -5, 5], "target": [0, 0, 0], "fov_y_deg": 45},
        "model": model
    });
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&skr).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, document.to_string().as_bytes()).unwrap();
    zip.finish().unwrap();
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "open", "path": skr.to_str().unwrap()}),
    )
    .unwrap();
    let path = scratch("all-left-out.obj");
    let (kind, message) = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(
        message.contains("1 faces could not be triangulated"),
        "{message}"
    );
    assert!(!message.contains("no faces"), "{message}");
}

/// `painted_boxes` with the material made see-through.
fn glass_boxes(d: &mut Document) {
    painted_boxes(d, "Glass", [255, 255, 255]);
    let material = ask(d, json!({"type": "materials"}))["materials"][0]["id"].clone();
    run(
        d,
        json!({"type": "change_material", "material": material, "opacity": 0.4}),
    )
    .unwrap();
}

#[test]
fn gltf_of_glass_blends_with_its_opacity_as_alpha() {
    let mut d = doc();
    glass_boxes(&mut d);
    let doc: Value = serde_json::from_slice(&export(&mut d, "glass.gltf")).unwrap();
    let glass = &doc["materials"][0];
    assert_eq!(glass["name"], "Glass");
    assert_eq!(glass["alphaMode"], "BLEND");
    assert_eq!(
        glass["pbrMetallicRoughness"]["baseColorFactor"],
        json!([1.0, 1.0, 1.0, 0.4])
    );
    assert!(doc["materials"][1]["alphaMode"].is_null());
    // A .glb carries the same document.
    let glb = export(&mut d, "glass.glb");
    let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let doc: Value = serde_json::from_slice(&glb[20..20 + len]).unwrap();
    assert_eq!(doc["materials"][0]["alphaMode"], "BLEND");
}

#[test]
fn mtl_of_glass_gives_dissolve_and_transparency() {
    let mut d = doc();
    glass_boxes(&mut d);
    let mtl = scratch("glass-boxes.mtl");
    export(&mut d, "glass-boxes.obj");
    let library = std::fs::read_to_string(&mtl).unwrap();
    assert!(
        library.contains("newmtl Glass\nKd 1.000000 1.000000 1.000000\nd 0.400000\nTr 0.600000\n"),
        "{library}"
    );
    assert!(library.contains("newmtl default\nKd"), "{library}");
    assert_eq!(
        library.matches("\nd ").count(),
        1,
        "opaque materials have no d"
    );
    // STL has no materials: the same triangles as opaque boxes.
    let glass = export(&mut d, "glass-boxes.stl");
    let mut plain = doc();
    two_boxes(&mut plain);
    assert_eq!(glass, export(&mut plain, "plain-boxes.stl"));
}

#[test]
fn gltf_normals_are_smooth_across_soft_edges() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1, "segments": 12}),
    )
    .unwrap();
    let face = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": -2}),
    )
    .unwrap();
    let doc: Value = serde_json::from_slice(&export(&mut d, "cylinder.gltf")).unwrap();
    let uri = doc["buffers"][0]["uri"].as_str().unwrap();
    let data = uri
        .strip_prefix("data:application/octet-stream;base64,")
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    let floats: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let (positions, normals) = floats.split_at(floats.len() / 2);
    let mut sides = 0;
    let vectors = |xs: &[f32]| -> Vec<DVec3> {
        xs.as_chunks::<3>()
            .0
            .iter()
            .map(|c| DVec3::from_array(c.map(f64::from)))
            .collect()
    };
    for (p, n) in vectors(positions).into_iter().zip(vectors(normals)) {
        if n.y.abs() > 0.999 {
            continue; // a cap
        }
        // A side corner's normal points straight out from the axis (Y up) through it.
        let out = DVec3::new(p.x, 0.0, p.z).normalize();
        assert!(n.distance(out) < 1e-5, "{p} {n}");
        sides += 1;
    }
    assert_eq!(sides, 12 * 6);
}

/// `two_boxes` with the first crate painted with a material whose texture is a 4 x 2 pixel PNG
/// covering 2 x 1 m. Returns the PNG's bytes.
fn textured_boxes(d: &mut Document, png: &str) -> Vec<u8> {
    two_boxes(d);
    let path = scratch(png);
    image::RgbaImage::from_fn(4, 2, |x, _| image::Rgba([60 * x as u8, 0, 200, 255]))
        .save(&path)
        .unwrap();
    let out = run(
        d,
        json!({"type": "add_material", "name": "Tiles",
               "texture": {"path": path.to_str().unwrap(), "width": 2}}),
    )
    .unwrap();
    let material = out["changes"]["materials"][0].clone();
    let instances = ask(d, json!({"type": "entities", "kinds": ["instance"]}));
    let first = instances["instances"][0]["id"].clone();
    run(
        d,
        json!({"type": "paint", "ids": [first], "material": material}),
    )
    .unwrap();
    std::fs::read(&path).unwrap()
}

/// The `.glb` chunks: the document and the buffer.
fn glb_parts(bytes: &[u8]) -> (Value, Vec<u8>) {
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    let json_len = word(12);
    let doc = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
    let bin = 20 + json_len;
    (doc, bytes[bin + 8..bin + 8 + word(bin)].to_vec())
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

#[test]
fn gltf_embeds_textures_with_texture_coordinates() {
    let mut d = doc();
    let png = textured_boxes(&mut d, "gltf-tiles.png");
    let (doc, buffer) = glb_parts(&export(&mut d, "textured.glb"));
    let tiles = &doc["materials"][0];
    assert_eq!(tiles["name"], "Tiles");
    assert_eq!(
        tiles["pbrMetallicRoughness"]["baseColorTexture"],
        json!({"index": 0})
    );
    assert_eq!(
        tiles["pbrMetallicRoughness"]["baseColorFactor"],
        json!([1.0, 1.0, 1.0, 1.0])
    );
    assert_eq!(doc["textures"], json!([{"sampler": 0, "source": 0}]));
    assert_eq!(doc["samplers"][0]["wrapS"], 10497);
    assert_eq!(doc["samplers"][0]["wrapT"], 10497);
    assert_eq!(doc["images"][0]["mimeType"], "image/png");
    // The image is the file as read.
    let view = &doc["bufferViews"][doc["images"][0]["bufferView"].as_u64().unwrap() as usize];
    let at = view["byteOffset"].as_u64().unwrap() as usize;
    let len = view["byteLength"].as_u64().unwrap() as usize;
    assert_eq!(&buffer[at..at + len], &png[..]);
    // The default material has no texture coordinates.
    let primitives = doc["meshes"][0]["primitives"].as_array().unwrap();
    assert!(primitives[1]["attributes"]["TEXCOORD_0"].is_null());
    assert!(doc["materials"][1]["pbrMetallicRoughness"]["baseColorTexture"].is_null());

    // On the crate's wall facing -y (z = 0 in Y-up) the texture runs along x and up, one copy
    // per 2 x 1 m: s = x / 2 and t = -height, up to whole repeats.
    let accessor = |k: &Value| {
        let a = &doc["accessors"][k.as_u64().unwrap() as usize];
        let view = &doc["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let at =
            (view["byteOffset"].as_u64().unwrap() + a["byteOffset"].as_u64().unwrap()) as usize;
        let width = if a["type"] == "VEC2" { 8 } else { 12 };
        floats(&buffer[at..at + width * a["count"].as_u64().unwrap() as usize])
    };
    let positions = accessor(&primitives[0]["attributes"]["POSITION"]);
    let st = accessor(&primitives[0]["attributes"]["TEXCOORD_0"]);
    assert_eq!(st.len(), 36 * 2);
    let mut wall = 0;
    for (p, s) in positions
        .as_chunks::<9>()
        .0
        .iter()
        .zip(st.as_chunks::<6>().0)
    {
        if [p[2], p[5], p[8]].iter().any(|z| *z != 0.0) {
            continue;
        }
        for k in 0..3 {
            let (x, height) = (p[3 * k], p[3 * k + 1]);
            let (ds, dt) = (s[2 * k] - x / 2.0, s[2 * k + 1] + height);
            assert!((ds - ds.round()).abs() < 1e-6, "{p:?} {s:?}");
            assert!((dt - dt.round()).abs() < 1e-6, "{p:?} {s:?}");
            assert!(
                s[2 * k].abs() <= 2.0 && s[2 * k + 1].abs() <= 2.0,
                "kept small"
            );
        }
        wall += 1;
    }
    assert_eq!(wall, 2);

    // A .gltf carries the image in its embedded buffer the same way.
    let doc: Value = serde_json::from_slice(&export(&mut d, "textured.gltf")).unwrap();
    assert_eq!(doc["images"][0]["mimeType"], "image/png");
    assert!(doc["images"][0]["bufferView"].is_u64());
}

#[test]
fn obj_writes_texture_coordinates_and_images_beside_the_library() {
    let mut d = doc();
    let png = textured_boxes(&mut d, "obj-tiles.png");
    let obj = String::from_utf8(export(&mut d, "tiled-boxes.obj")).unwrap();
    assert!(obj.lines().any(|l| l.starts_with("vt ")), "{obj}");
    let faces: Vec<&str> = obj.lines().filter(|l| l.starts_with("f ")).collect();
    assert_eq!(faces.len(), 24);
    // The textured crate's faces name texture coordinates; the default ones do not.
    assert_eq!(faces.iter().filter(|f| !f.contains("//")).count(), 12);
    let library = std::fs::read_to_string(scratch_kept("tiled-boxes.mtl")).unwrap();
    assert!(
        library.contains("newmtl Tiles\nKd ") && library.contains("map_Kd tiled-boxes_Tiles.png\n"),
        "{library}"
    );
    assert_eq!(
        std::fs::read(scratch_kept("tiled-boxes_Tiles.png")).unwrap(),
        png
    );
}

#[test]
fn obj_texture_images_differ_even_ignoring_case() {
    // "Wood" and "wood" are two materials; on a case-blind file system their images must not
    // share a file.
    let mut d = doc();
    two_boxes(&mut d);
    let instances = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    let mut pngs = Vec::new();
    for (k, name) in ["Wood", "wood"].into_iter().enumerate() {
        let path = scratch(&format!("case-{k}.png"));
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 100 * k as u8, 200, 255]))
            .save(&path)
            .unwrap();
        pngs.push(std::fs::read(&path).unwrap());
        let out = run(
            &mut d,
            json!({"type": "add_material", "name": name,
                   "texture": {"path": path.to_str().unwrap(), "width": 1}}),
        )
        .unwrap();
        let material = out["changes"]["materials"][0].clone();
        let instance = instances["instances"][k]["id"].clone();
        run(
            &mut d,
            json!({"type": "paint", "ids": [instance], "material": material}),
        )
        .unwrap();
    }
    export(&mut d, "case-boxes.obj");
    let library = std::fs::read_to_string(scratch_kept("case-boxes.mtl")).unwrap();
    let maps: Vec<&str> = library
        .lines()
        .filter_map(|l| l.strip_prefix("map_Kd "))
        .collect();
    assert_eq!(maps.len(), 2, "{library}");
    assert_ne!(maps[0].to_lowercase(), maps[1].to_lowercase(), "{library}");
    // Each material's `newmtl` precedes its own image, which holds that material's texture.
    for (k, name) in ["Wood", "wood_2"].into_iter().enumerate() {
        let block = library
            .split("newmtl ")
            .find(|b| b.starts_with(&format!("{name}\n")));
        let file = block
            .and_then(|b| b.lines().find_map(|l| l.strip_prefix("map_Kd ")))
            .unwrap_or_else(|| panic!("{name}: {library}"));
        assert_eq!(
            std::fs::read(scratch_kept(file)).unwrap(),
            pngs[k],
            "{file}"
        );
    }
}

/// A path in the scratch directory, left as it is.
fn scratch_kept(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-export-{}", std::process::id()))
        .join(name)
}

/// `painted_boxes` with the material glowing at `emission`.
fn glowing_boxes(d: &mut Document, emission: f64) {
    painted_boxes(d, "Lamp", [255, 255, 255]);
    let material = ask(d, json!({"type": "materials"}))["materials"][0]["id"].clone();
    run(
        d,
        json!({"type": "change_material", "material": material, "emission": emission}),
    )
    .unwrap();
}

#[test]
fn gltf_of_glowing_materials_gives_their_emissive_color_and_strength() {
    let mut d = doc();
    glowing_boxes(&mut d, 0.5);
    let doc: Value = serde_json::from_slice(&export(&mut d, "dim-lamp.gltf")).unwrap();
    assert_eq!(
        doc["materials"][0]["emissiveFactor"],
        json!([0.5, 0.5, 0.5])
    );
    assert!(doc["materials"][0]["extensions"].is_null());
    assert!(doc["extensionsUsed"].is_null());
    assert!(doc["materials"][1]["emissiveFactor"].is_null(), "default");

    let mut d = self::doc();
    glowing_boxes(&mut d, 4.0);
    let (doc, _) = glb_parts(&export(&mut d, "bright-lamp.glb"));
    let lamp = &doc["materials"][0];
    assert_eq!(lamp["emissiveFactor"], json!([1.0, 1.0, 1.0]));
    assert_eq!(
        lamp["extensions"]["KHR_materials_emissive_strength"]["emissiveStrength"],
        4.0
    );
    assert_eq!(
        doc["extensionsUsed"],
        json!(["KHR_materials_emissive_strength"])
    );
}

#[test]
fn a_glowing_texture_is_its_own_emissive_texture() {
    let mut d = doc();
    textured_boxes(&mut d, "glow-tiles.png");
    let tiles = ask(&d, json!({"type": "materials"}))["materials"][0]["id"].clone();
    run(
        &mut d,
        json!({"type": "change_material", "material": tiles, "emission": 2}),
    )
    .unwrap();
    let (doc, _) = glb_parts(&export(&mut d, "glow-tiles.glb"));
    let tiles = &doc["materials"][0];
    assert_eq!(tiles["emissiveTexture"], json!({"index": 0}));
    assert_eq!(tiles["emissiveFactor"], json!([1.0, 1.0, 1.0]));

    export(&mut d, "glow-tiles.obj");
    let library = std::fs::read_to_string(scratch_kept("glow-tiles.mtl")).unwrap();
    assert!(
        library.contains("map_Ke glow-tiles_Tiles.png\n"),
        "{library}"
    );
}

#[test]
fn mtl_of_glowing_materials_gives_their_emissive_color() {
    let mut d = doc();
    glowing_boxes(&mut d, 2.5);
    export(&mut d, "lamp-boxes.obj");
    let library = std::fs::read_to_string(scratch_kept("lamp-boxes.mtl")).unwrap();
    assert!(
        library.contains(
            "newmtl Lamp\nKd 1.000000 1.000000 1.000000\nKe 2.500000 2.500000 2.500000\n"
        ),
        "{library}"
    );
    assert_eq!(library.matches("Ke ").count(), 1, "only the lamp glows");
    assert!(!library.contains("map_Ke"), "{library}");
}

/// A 2 x 1 m rectangle on the ground painted with a 2 x 1 m texture, its texture moved and
/// turned a quarter. Returns the face as the kernel holds it.
fn positioned_tile(d: &mut Document) -> kernel::Face {
    let path = scratch("positioned-tile.png");
    image::RgbaImage::from_fn(4, 2, |x, _| image::Rgba([60 * x as u8, 0, 200, 255]))
        .save(&path)
        .unwrap();
    let out = run(
        d,
        json!({"type": "add_material", "name": "Tiles",
               "texture": {"path": path.to_str().unwrap(), "width": 2}}),
    )
    .unwrap();
    let material = out["changes"]["materials"][0].clone();
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 1, 0]}),
    )
    .unwrap();
    let face = ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(
        d,
        json!({"type": "paint", "ids": [face], "material": material}),
    )
    .unwrap();
    run(
        d,
        json!({"type": "position_texture", "ids": [face], "offset": [0.5, 0.25],
               "rotation_deg": 90}),
    )
    .unwrap();
    d.model().faces().next().unwrap().1.clone()
}

#[test]
fn obj_writes_placed_texture_coordinates() {
    let mut d = doc();
    let face = positioned_tile(&mut d);
    let obj = String::from_utf8(export(&mut d, "positioned.obj")).unwrap();
    let numbers = |prefix: &str| -> Vec<Vec<f64>> {
        obj.lines()
            .filter_map(|l| l.strip_prefix(prefix))
            .map(|l| l.split(' ').map(|x| x.parse().unwrap()).collect())
            .collect()
    };
    let (v, vt) = (numbers("v "), numbers("vt "));
    let mut corners = 0;
    for f in obj.lines().filter_map(|l| l.strip_prefix("f ")) {
        for corner in f.split(' ') {
            let ids: Vec<usize> = corner.split('/').map(|k| k.parse().unwrap()).collect();
            // Y-up back to the model's Z-up.
            let p = &v[ids[0] - 1];
            let model = DVec3::new(p[0], -p[2], p[1]);
            let expected = face.texture_position(model) / DVec2::new(2.0, 1.0);
            let st = &vt[ids[1] - 1];
            assert!(
                (st[0] - expected.x).abs() < 1e-9 && (st[1] - expected.y).abs() < 1e-9,
                "{model} {st:?} {expected}"
            );
            corners += 1;
        }
    }
    assert_eq!(corners, 6);
    // Turned a quarter: along the ground's x the texture's height changes, not its width.
    assert!(
        (face.texture_position(DVec3::new(1.0, 0.0, 0.0)) - face.texture_position(DVec3::ZERO))
            .abs_diff_eq(DVec2::new(0.0, -1.0), 1e-12)
    );
}

#[test]
fn gltf_writes_placed_texture_coordinates() {
    let mut d = doc();
    let face = positioned_tile(&mut d);
    let (doc, buffer) = glb_parts(&export(&mut d, "positioned.glb"));
    let accessor = |k: &Value| {
        let a = &doc["accessors"][k.as_u64().unwrap() as usize];
        let view = &doc["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let at =
            (view["byteOffset"].as_u64().unwrap() + a["byteOffset"].as_u64().unwrap()) as usize;
        let width = if a["type"] == "VEC2" { 8 } else { 12 };
        floats(&buffer[at..at + width * a["count"].as_u64().unwrap() as usize])
    };
    let primitive = &doc["meshes"][0]["primitives"][0];
    let positions = accessor(&primitive["attributes"]["POSITION"]);
    let st = accessor(&primitive["attributes"]["TEXCOORD_0"]);
    assert_eq!(st.len(), 6 * 2);
    for (p, s) in positions
        .as_chunks::<3>()
        .0
        .iter()
        .zip(st.as_chunks::<2>().0)
    {
        let model = DVec3::new(f64::from(p[0]), -f64::from(p[2]), f64::from(p[1]));
        let placed = face.texture_position(model) / DVec2::new(2.0, 1.0);
        // v counted down from the image's top, up to whole repeats.
        let (ds, dt) = (f64::from(s[0]) - placed.x, f64::from(s[1]) + placed.y);
        assert!((ds - ds.round()).abs() < 1e-6, "{model} {s:?} {placed}");
        assert!((dt - dt.round()).abs() < 1e-6, "{model} {s:?} {placed}");
    }
}

#[test]
fn export_does_not_overwrite_another_kind_of_file_unless_told() {
    let mut d = doc();
    painted_boxes(&mut d, "Red", [255, 0, 0]);
    for (name, format) in [
        ("v.txt", "stl"),
        ("v2.txt", "obj"),
        ("v3.txt", "glb"),
        ("v4.txt", "gltf"),
    ] {
        let victim = scratch(name);
        let _ = std::fs::remove_file(victim.with_extension("mtl"));
        std::fs::write(&victim, "notes").unwrap();
        let (kind, _) = run(
            &mut d,
            json!({"type": "export", "path": victim, "format": format}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{format}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "notes");
        run(
            &mut d,
            json!({"type": "export", "path": victim, "format": format, "overwrite": true}),
        )
        .unwrap();
        // Its own format is replaced freely.
        run(
            &mut d,
            json!({"type": "export", "path": victim, "format": format}),
        )
        .unwrap();
    }
}

#[test]
fn an_obj_export_checks_its_library_before_writing_anything() {
    let mut d = doc();
    painted_boxes(&mut d, "Red", [255, 0, 0]);
    let obj = scratch("lib-guard.obj");
    let mtl = scratch("lib-guard.mtl");
    std::fs::write(&mtl, "notes").unwrap();
    let (kind, message) = run(&mut d, json!({"type": "export", "path": obj})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("lib-guard.mtl"), "{message}");
    assert!(!obj.exists());
    assert_eq!(std::fs::read_to_string(&mtl).unwrap(), "notes");
    run(
        &mut d,
        json!({"type": "export", "path": obj, "overwrite": true}),
    )
    .unwrap();
    run(&mut d, json!({"type": "export", "path": obj})).unwrap();
}

#[test]
fn stl_files_of_either_kind_are_recognised() {
    let mut d = doc();
    two_boxes(&mut d);
    let ascii = scratch("ascii.stl");
    std::fs::write(&ascii, "  SOLID x\nendsolid x\n").unwrap();
    run(&mut d, json!({"type": "export", "path": ascii})).unwrap();
    let binary = scratch("binary.stl");
    let mut bytes = vec![0u8; 80];
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 50]);
    std::fs::write(&binary, &bytes).unwrap();
    run(&mut d, json!({"type": "export", "path": binary})).unwrap();
    let wrong = scratch("wrong.stl");
    bytes.push(0);
    std::fs::write(&wrong, &bytes).unwrap();
    assert!(run(&mut d, json!({"type": "export", "path": wrong})).is_err());
}

#[cfg(unix)]
#[test]
fn an_obj_export_replaces_a_library_link_and_leaves_its_target() {
    let mut d = doc();
    painted_boxes(&mut d, "Red", [255, 0, 0]);
    let obj = scratch("lib-link.obj");
    let mtl = scratch("lib-link.mtl");
    let outside = scratch("elsewhere/outside.txt");
    std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
    std::fs::write(&outside, "keep me").unwrap();
    std::os::unix::fs::symlink(&outside, &mtl).unwrap();
    run(
        &mut d,
        json!({"type": "export", "path": obj, "overwrite": true}),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep me");
    let meta = std::fs::symlink_metadata(&mtl).unwrap();
    assert!(meta.is_file() && !meta.file_type().is_symlink());
    assert!(
        std::fs::read_to_string(&mtl)
            .unwrap()
            .contains("newmtl Red")
    );
}
