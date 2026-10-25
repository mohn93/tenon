//! Importing glTF files (`.gltf` and `.glb`): node trees, shared meshes, materials, textures
//! and side files, driven through JSON commands, and round trips through `export`.

use std::path::PathBuf;

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

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gltf")
        .join(name)
        .to_str()
        .unwrap()
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-gltf-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn import(d: &mut Document, path: &str) -> Value {
    let r = run(d, json!({"type": "import", "path": path})).unwrap();
    d.scene().validate().unwrap();
    r["import"].clone()
}

fn definitions(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "definitions"}))["definitions"]
        .as_array()
        .unwrap()
        .clone()
}

fn definition<'a>(defs: &'a [Value], name: &str) -> &'a Value {
    defs.iter()
        .find(|d| d["name"] == name)
        .unwrap_or_else(|| panic!("no definition {name} in {defs:?}"))
}

/// Every corner of the model's triangles, in world coordinates.
fn corners(d: &Document) -> Vec<DVec3> {
    d.scene()
        .triangles()
        .iter()
        .flat_map(|t| t.corners)
        .collect()
}

fn bounds(points: &[DVec3]) -> (DVec3, DVec3) {
    points.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    )
}

#[test]
fn a_gltf_box_comes_in_as_six_faces_with_its_color() {
    let mut d = doc();
    let report = import(&mut d, &fixture("box.gltf"));
    assert_eq!(report["polygons"], 12);
    assert_eq!(report["faces"], 6);
    assert_eq!(report["vertices"], 8);
    assert_eq!(report["skipped"], 0);
    assert_eq!(report["materials"], 1);
    // One node holding a mesh only it holds goes straight into the wrapper group.
    assert_eq!(report.get("definitions"), None);
    let warnings = report["warnings"].as_array().unwrap();
    assert!(warnings.contains(&json!("1 animation was not imported")));
    assert!(warnings.contains(&json!("1 camera was not imported")));
    let defs = definitions(&d);
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0]["name"], "box");
    assert_eq!(defs[0]["kind"], "group");
    // The node's move (2 m along file x) is kept, and file y is up.
    let (lo, hi) = bounds(&corners(&d));
    assert!(lo.distance(DVec3::new(2.0, -1.0, 0.0)) < 1e-6, "{lo}");
    assert!(hi.distance(DVec3::new(3.0, 0.0, 1.0)) < 1e-6, "{hi}");
    let materials = ask(&d, json!({"type": "materials"}))["materials"].clone();
    assert_eq!(materials[0]["name"], "Blue");
    assert_eq!(materials[0]["color"], json!([0, 0, 255]));
    assert_eq!(materials[0]["faces"], 6);
    // A flat-shaded box has no soft edges.
    assert_eq!(soft_edges(&d), 0);
    // One undo step takes it all back.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(definitions(&d).is_empty());
    assert!(
        ask(&d, json!({"type": "materials"}))["materials"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_glb_with_a_texture_lays_the_image_as_its_coordinates_say() {
    let mut d = doc();
    let report = import(&mut d, &fixture("textured.glb"));
    assert_eq!(report["faces"], 1);
    assert_eq!(report["textures"], 1);
    let materials = ask(&d, json!({"type": "materials"}))["materials"].clone();
    let texture = &materials[0]["texture"];
    assert_eq!(texture["width"], 1.0);
    assert_eq!(texture["height"], 1.0);
    assert_eq!(
        (&texture["pixel_width"], &texture["pixel_height"]),
        (&json!(2), &json!(2))
    );
    // The image repeats every meter along x and y from the origin.
    for t in d.scene().triangles() {
        for (c, uv) in t.corners.iter().zip(t.uvs) {
            let d = uv - DVec2::new(c.x, c.y);
            assert!((d - d.round()).length() < 1e-6, "{c} {uv}");
        }
    }
}

#[test]
fn a_mesh_two_nodes_place_becomes_one_component_placed_twice() {
    let mut d = doc();
    let report = import(&mut d, &fixture("table.gltf"));
    assert_eq!(report["definitions"], 2);
    assert_eq!(report["instances"], 3);
    assert_eq!(report["materials"], 2);
    let defs = definitions(&d);
    let leg = definition(&defs, "Leg");
    assert_eq!(leg["kind"], "component");
    assert_eq!(leg["instances"], 2);
    assert_eq!(leg["faces"], 6);
    let table = definition(&defs, "Table");
    assert_eq!(table["kind"], "group");
    assert_eq!(table["faces"], 6);
    assert_eq!(definition(&defs, "table")["instances"], 1);
    // The table node is turned 90 degrees about file y (model z): its 1 m top runs along y.
    let (lo, hi) = bounds(&corners(&d));
    let size = hi - lo;
    assert!(
        (size - DVec3::new(0.5, 1.0, 0.75)).length() < 1e-6,
        "{size}"
    );
}

#[test]
fn side_files_are_read_beside_the_file_and_never_outside_it() {
    let mut d = doc();
    let report = import(&mut d, &fixture("side.gltf"));
    assert_eq!(report["faces"], 6);
    assert_eq!(report["textures"], 1);
    let err = run(
        &mut d,
        json!({"type": "import", "path": fixture("escape.gltf")}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::File);
    assert!(err.1.contains("outside"), "{}", err.1);

    // From bytes (the web build), side files cannot be read; data URIs and .glb can.
    let mut e = doc();
    let bytes = std::fs::read(fixture("side.gltf")).unwrap();
    let err = e
        .import_bytes("side.gltf", bytes, &session::ImportOptions::default())
        .unwrap_err();
    assert!(err.message.contains("memory"), "{}", err.message);
    let bytes = std::fs::read(fixture("textured.glb")).unwrap();
    e.import_bytes("textured.glb", bytes, &session::ImportOptions::default())
        .unwrap();
    assert_eq!(e.scene().triangles().len(), 2);
}

#[test]
fn an_image_outside_the_folder_leaves_the_color_with_a_warning() {
    let mut gltf: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("side.gltf")).unwrap()).unwrap();
    let dir = PathBuf::from(fixture("side.gltf"))
        .parent()
        .unwrap()
        .to_path_buf();
    gltf["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        base64_of(&std::fs::read(dir.join("side.bin")).unwrap())
    ));
    gltf["images"][0]["uri"] = json!("https://example.com/crate.png");
    let path = scratch("remote.gltf");
    std::fs::write(&path, gltf.to_string()).unwrap();
    let mut d = doc();
    let report = import(&mut d, path.to_str().unwrap());
    assert_eq!(report["faces"], 6);
    assert_eq!(report.get("textures"), None);
    let warnings = report["warnings"].to_string();
    assert!(
        warnings.contains("1 texture was not imported"),
        "{warnings}"
    );
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[test]
fn broken_and_hostile_files_are_refused_and_change_nothing() {
    let box_doc: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("box.gltf")).unwrap()).unwrap();
    type Change = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Change, &str)> = vec![
        (
            "version",
            Box::new(|v| v["asset"]["version"] = json!("1.0")),
            "2.0",
        ),
        (
            "draco",
            Box::new(|v| v["extensionsRequired"] = json!(["KHR_draco_mesh_compression"])),
            "KHR_draco_mesh_compression",
        ),
        (
            "count",
            Box::new(|v| v["accessors"][0]["count"] = json!(1000)),
            "past the end",
        ),
        (
            "offset",
            Box::new(|v| v["bufferViews"][0]["byteOffset"] = json!(u64::MAX)),
            "past the end",
        ),
        (
            "view",
            Box::new(|v| v["accessors"][0]["bufferView"] = json!(99)),
            "does not exist",
        ),
        (
            "kind",
            Box::new(|v| v["accessors"][0]["type"] = json!("VEC2")),
            "VEC3",
        ),
        (
            "sparse",
            Box::new(|v| v["accessors"][0]["sparse"] = json!({"count": 1})),
            "sparse",
        ),
        (
            "cycle",
            Box::new(|v| v["nodes"][0]["children"] = json!([0])),
            "places itself",
        ),
        (
            "url",
            Box::new(|v| v["buffers"][0]["uri"] = json!("http://x/y.bin")),
            "URL",
        ),
        (
            "absolute",
            Box::new(|v| v["buffers"][0]["uri"] = json!("/etc/hosts")),
            "absolute",
        ),
        ("json", Box::new(|v| *v = json!("text")), "not a glTF file"),
    ];
    for (name, change, wanted) in cases {
        let mut v = box_doc.clone();
        change(&mut v);
        let path = scratch(&format!("bad-{name}.gltf"));
        std::fs::write(&path, v.to_string()).unwrap();
        let mut d = doc();
        let err = run(
            &mut d,
            json!({"type": "import", "path": path.to_str().unwrap()}),
        )
        .unwrap_err();
        assert_eq!(err.0, ErrorKind::File, "{name}: {}", err.1);
        assert!(err.1.contains(wanted), "{name}: {}", err.1);
        assert!(d.scene().triangles().is_empty(), "{name}");
        assert!(
            ask(&d, json!({"type": "materials"}))["materials"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{name}"
        );
    }
    // A cut-short .glb.
    let mut glb = std::fs::read(fixture("textured.glb")).unwrap();
    glb.truncate(40);
    let path = scratch("short.glb");
    std::fs::write(&path, glb).unwrap();
    let err = run(
        &mut doc(),
        json!({"type": "import", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::File, "{}", err.1);
}

#[test]
fn a_mirrored_node_keeps_its_faces_facing_out() {
    let box_doc: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("box.gltf")).unwrap()).unwrap();
    for (name, scale) in [("plain", [1.0, 1.0, 1.0]), ("mirrored", [-1.0, 1.0, 1.0])] {
        for place in ["group", "context"] {
            let mut v = box_doc.clone();
            v["nodes"][0]["scale"] = json!(scale);
            let path = scratch(&format!("{name}-{place}.gltf"));
            std::fs::write(&path, v.to_string()).unwrap();
            let mut d = doc();
            run(
                &mut d,
                json!({"type": "import", "path": path.to_str().unwrap(), "as": place}),
            )
            .unwrap();
            d.scene().validate().unwrap();
            let triangles = d.scene().triangles();
            let (lo, hi) = bounds(&corners(&d));
            let center = (lo + hi) / 2.0;
            assert!(!triangles.is_empty());
            for t in &triangles {
                let middle = (t.corners[0] + t.corners[1] + t.corners[2]) / 3.0;
                assert!(
                    t.normal.dot(middle - center) > 0.0,
                    "{name} {place}: a face faces in"
                );
            }
        }
    }
}

#[test]
fn a_small_file_asking_for_millions_of_items_is_refused_before_they_are_read() {
    // A few megabytes of empty objects would be hundreds of megabytes once read.
    let objects = vec!["{}"; 1_100_000].join(",");
    let text = format!(r#"{{"asset":{{"version":"2.0"}},"accessors":[{objects}]}}"#);
    let path = scratch("many.gltf");
    std::fs::write(&path, text).unwrap();
    let err = run(
        &mut doc(),
        json!({"type": "import", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::File, "{}", err.1);
    assert!(err.1.contains("too many items"), "{}", err.1);
    // Values in arrays count too; strings (data URIs) do not.
    let values = vec!["0"; 8_100_000].join(",");
    let text = format!(r#"{{"asset":{{"version":"2.0"}},"extras":[{values}]}}"#);
    std::fs::write(&path, text).unwrap();
    let err = run(
        &mut doc(),
        json!({"type": "import", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert!(err.1.contains("too many items"), "{}", err.1);
    let text = format!(
        r#"{{"asset":{{"version":"2.0"}},"extras":"{}"}}"#,
        ",[{".repeat(3_000_000)
    );
    std::fs::write(&path, text).unwrap();
    // Read through to the end: it holds nothing to import.
    let err = run(
        &mut doc(),
        json!({"type": "import", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert!(err.1.contains("nothing to import"), "{}", err.1);
}

/// A 2 x 2 x 1 box with its top painted red, and a cylinder whose sides are soft.
fn model(d: &mut Document) {
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
    let red = run(
        d,
        json!({"type": "add_material", "name": "Red", "color": [200, 30, 20]}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    let top = ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap()["id"]
        .clone();
    run(d, json!({"type": "paint", "ids": [top], "material": red})).unwrap();
    run(
        d,
        json!({"type": "add_cylinder", "base": [5, 0, 0], "radius": 0.5, "height": 2,
               "segments": 12, "as": "geometry"}),
    )
    .unwrap();
}

/// Soft edges in every mesh of the model, each definition once.
fn soft_edges(d: &Document) -> usize {
    d.scene()
        .meshes()
        .map(|(_, m)| m.edges().filter(|(_, e)| e.soft).count())
        .sum()
}

#[test]
fn export_to_glb_and_gltf_then_import_gives_back_the_faces() {
    for name in ["round.glb", "round.gltf"] {
        let mut d = doc();
        model(&mut d);
        let faces: usize = d.scene().meshes().map(|(_, m)| m.faces().count()).sum();
        let path = scratch(name);
        let path = path.to_str().unwrap();
        run(&mut d, json!({"type": "export", "path": path})).unwrap();
        let mut e = doc();
        let report = import(&mut e, path);
        assert_eq!(report["faces"], faces, "{name}");
        assert_eq!(report["skipped"], 0, "{name}");
        // The cylinder's sides stay soft; the box and the caps stay hard.
        assert_eq!(soft_edges(&e), soft_edges(&d), "{name}");
        assert!(soft_edges(&d) > 0);
        // The same corners, f32 aside.
        let (a, b) = (bounds(&corners(&d)), bounds(&corners(&e)));
        assert!(
            a.0.distance(b.0) < 1e-5 && a.1.distance(b.1) < 1e-5,
            "{name}"
        );
        let materials = ask(&e, json!({"type": "materials"}))["materials"].clone();
        let red = materials
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == "Red")
            .unwrap();
        assert_eq!(red["color"], json!([200, 30, 20]));
        assert_eq!(red["faces"], 1);
    }
}

#[test]
fn textures_come_back_where_they_were_after_a_glb_round_trip() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_box", "corner": [1, 2, 0], "size": [2, 1, 1.5], "as": "geometry"}),
    )
    .unwrap();
    let wood = run(
        &mut d,
        json!({"type": "add_material", "name": "Wood",
               "texture": {"path": fixture("textures/crate wood.png"), "width": 0.5, "height": 0.25}}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    let faces: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .collect();
    run(
        &mut d,
        json!({"type": "paint", "ids": faces, "material": wood}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [faces[0], faces[3]], "rotation_deg": 30,
               "offset": [0.1, 0.05], "scale": [2, 1.5]}),
    )
    .unwrap();
    let path = scratch("textured-round.glb");
    let path = path.to_str().unwrap();
    run(&mut d, json!({"type": "export", "path": path})).unwrap();
    let mut e = doc();
    let report = import(&mut e, path);
    assert_eq!(report["faces"], 6);
    assert_eq!(report["textures"], 1);
    // The size most faces show is the material's; the turned faces keep their placement.
    let materials = ask(&e, json!({"type": "materials"}))["materials"].clone();
    assert_eq!(materials[0]["texture"]["width"], 0.5);
    assert_eq!(materials[0]["texture"]["height"], 0.25);
    let placed = e
        .scene()
        .meshes()
        .flat_map(|(_, m)| m.faces().filter_map(|(_, f)| f.texture).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(placed.len(), 2, "{placed:?}");
    // Each corner shows the same spot of the image (whole repeats aside) as before.
    let spots = |doc: &Document| {
        let materials = ask(doc, json!({"type": "materials"}))["materials"].clone();
        let t = &materials[0]["texture"];
        let size = DVec2::new(t["width"].as_f64().unwrap(), t["height"].as_f64().unwrap());
        let mut out: Vec<(DVec3, DVec2)> = Vec::new();
        for tri in doc.scene().triangles() {
            for (c, uv) in tri.corners.iter().zip(tri.uvs) {
                let st = uv / size;
                out.push((*c, st - st.floor()));
            }
        }
        out
    };
    let (before, after) = (spots(&d), spots(&e));
    for (c, st) in &after {
        let near = before
            .iter()
            .filter(|(p, _)| p.distance(*c) < 1e-5)
            .map(|(_, s)| {
                let d = *s - *st;
                (d - d.round()).length()
            })
            .fold(f64::INFINITY, f64::min);
        assert!(near < 1e-4, "{c}: {st} is {near} off");
    }
}
