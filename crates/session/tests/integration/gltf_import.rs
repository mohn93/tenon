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

/// The `textured.glb` fixture with its JSON changed by `change`, written to `name`.
fn textured_with(name: &str, change: impl Fn(&mut Value)) -> String {
    let glb = std::fs::read(fixture("textured.glb")).unwrap();
    let word = |at: usize| u32::from_le_bytes(glb[at..at + 4].try_into().unwrap()) as usize;
    let json_len = word(12);
    let mut v: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    change(&mut v);
    let mut json = v.to_string().into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let bin = &glb[20 + json_len..];
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((20 + json.len() + bin.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(bin);
    let path = scratch(name);
    std::fs::write(&path, out).unwrap();
    path.to_str().unwrap().to_string()
}

fn linear(byte: u64) -> f64 {
    let c = byte as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb(linear: f64) -> u64 {
    let s = if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u64
}

#[test]
fn a_textured_materials_color_is_the_images_average_times_the_base_color_factor() {
    let color = |path: &str| {
        let mut d = doc();
        import(&mut d, path);
        let materials = ask(&d, json!({"type": "materials"}))["materials"].clone();
        let c = materials[0]["color"].as_array().unwrap().clone();
        c.iter().map(|x| x.as_u64().unwrap()).collect::<Vec<_>>()
    };
    let plain = color(&fixture("textured.glb"));
    let tinted = color(&textured_with("tinted.glb", |v| {
        v["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] = json!([0.5, 1.0, 0.0, 1.0]);
    }));
    let want = vec![srgb(linear(plain[0]) * 0.5), plain[1], 0];
    assert_eq!(tinted, want, "{plain:?}");
}

/// Where each corner of the model's triangles lies on the image (in repeats, v up, whole
/// repeats aside) with the first material's texture size.
fn image_spots(d: &Document) -> Vec<(DVec3, DVec2)> {
    let materials = ask(d, json!({"type": "materials"}))["materials"].clone();
    let t = &materials[0]["texture"];
    let size = DVec2::new(t["width"].as_f64().unwrap(), t["height"].as_f64().unwrap());
    let mut out = Vec::new();
    for tri in d.scene().triangles() {
        for (c, uv) in tri.corners.iter().zip(tri.uvs) {
            out.push((*c, uv / size));
        }
    }
    out
}

#[test]
fn khr_texture_transform_moves_turns_and_scales_the_coordinates() {
    let (offset, rotation, scale) = ([0.25, 0.1], 0.5f64, [2.0, 2.0]);
    let path = textured_with("transformed.glb", |v| {
        v["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] = json!({
            "KHR_texture_transform": {"offset": offset, "rotation": rotation, "scale": scale}
        });
        v["extensionsUsed"] = json!(["KHR_texture_transform"]);
        v["extensionsRequired"] = json!(["KHR_texture_transform"]);
    });
    let mut d = doc();
    let report = import(&mut d, &path);
    assert!(
        report["warnings"].as_array().is_none_or(Vec::is_empty),
        "{report}"
    );
    let (sin, cos) = rotation.sin_cos();
    for (c, st) in image_spots(&d) {
        // The fixture's coordinates are (x, -y) with v down the image; the transform is
        // offset after rotation after scale, in the file's coordinates.
        let (u, v) = (c.x * scale[0], -c.y * scale[1]);
        let (u, v) = (
            cos * u + sin * v + offset[0],
            -sin * u + cos * v + offset[1],
        );
        let want = DVec2::new(u, -v);
        let d = st - want;
        assert!((d - d.round()).length() < 1e-6, "{c}: {st} not {want}");
    }

    // A rotation with unequal scales shears the image, which a placement cannot: the
    // nearest is laid, and the report says so.
    let path = textured_with("sheared.glb", |v| {
        v["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] = json!({
            "KHR_texture_transform": {"rotation": 0.5, "scale": [1.0, 3.0]}
        });
    });
    let report = import(&mut doc(), &path);
    let warnings = report["warnings"].to_string();
    assert!(warnings.contains("KHR_texture_transform"), "{report}");
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
            "basisu",
            Box::new(|v| v["extensionsRequired"] = json!(["KHR_texture_basisu"])),
            "KHR_texture_basisu",
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

// ---- M20c: vertex colors, texture coordinate sets, compression ----

fn pack_f32(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A `.glb` of `json` with `bin` as its binary chunk, written to `name`.
fn write_glb(name: &str, json: Value, bin: &[u8]) -> String {
    let mut json = json.to_string().into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    let path = scratch(name);
    std::fs::write(&path, out).unwrap();
    path.to_str().unwrap().to_string()
}

/// Two triangles of a unit square in the XZ plane facing up (glTF Y up), six corners.
const SQUARE: [[f32; 3]; 6] = [
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [1.0, 0.0, -1.0],
    [0.0, 0.0, 0.0],
    [1.0, 0.0, -1.0],
    [0.0, 0.0, -1.0],
];

fn material_colors(d: &Document) -> Vec<Vec<u64>> {
    let materials = ask(d, json!({"type": "materials"}))["materials"].clone();
    materials
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            m["color"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_u64().unwrap())
                .collect()
        })
        .collect()
}

/// A glb of the square with `COLOR_0` accessor JSON `color` over `color_bytes`.
fn colored_square(name: &str, color: Value, color_bytes: &[u8], material: Value) -> String {
    let positions = pack_f32(&SQUARE.concat());
    let mut bin = positions.clone();
    bin.extend_from_slice(color_bytes);
    let mut color = color;
    color["bufferView"] = json!(1);
    color["count"] = json!(6);
    write_glb(
        name,
        json!({
            "asset": {"version": "2.0"},
            "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
            "materials": [material],
            "meshes": [{"primitives": [{
                "attributes": {"POSITION": 0, "COLOR_0": 1}, "material": 0}]}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": positions.len()},
                {"buffer": 0, "byteOffset": positions.len(), "byteLength": color_bytes.len()}],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 6, "type": "VEC3"},
                color],
            "buffers": [{"byteLength": bin.len()}],
        }),
        &bin,
    )
}

#[test]
fn vertex_colors_paint_each_face_with_its_color() {
    let colors = pack_f32(&[
        1., 0., 0., 1., 0., 0., 1., 0., 0., 0., 1., 0., 0., 1., 0., 0., 1., 0.,
    ]);
    let path = colored_square(
        "colors.glb",
        json!({"componentType": 5126, "type": "VEC3"}),
        &colors,
        json!({"name": "Paint"}),
    );
    let mut d = doc();
    let report = import(&mut d, &path);
    assert_eq!(report["faces"], 2, "{report}");
    let mut got = material_colors(&d);
    got.sort();
    assert_eq!(got, vec![vec![0, 255, 0], vec![255, 0, 0]]);
    let names = ask(&d, json!({"type": "materials"}))["materials"].to_string();
    assert!(names.contains("Paint #ff0000"), "{names}");

    // Normalized unsigned bytes with alpha, the base color factor multiplying: gray 128 is
    // 128/255 in linear light, times 0.5.
    let bytes: Vec<u8> = (0..6).flat_map(|_| [128u8, 128, 128, 255]).collect();
    let path = colored_square(
        "colors-bytes.glb",
        json!({"componentType": 5121, "normalized": true, "type": "VEC4"}),
        &bytes,
        json!({"pbrMetallicRoughness": {"baseColorFactor": [0.5, 1.0, 1.0, 1.0]}}),
    );
    let mut d = doc();
    let report = import(&mut d, &path);
    // The same color on both triangles: one material, one merged face.
    assert_eq!(report["faces"], 1, "{report}");
    let c = material_colors(&d);
    assert_eq!(c.len(), 1);
    let gray = 128.0 / 255.0;
    assert_eq!(
        c[0],
        vec![srgb(0.5 * gray), srgb(gray), srgb(gray)],
        "{c:?}"
    );
}

#[test]
fn a_textured_material_keeps_its_texture_and_says_vertex_colors_were_left() {
    let path = textured_with("textured-colors.glb", |v| {
        // The normals view (four (0, 1, 0)) read as colors.
        v["accessors"].as_array_mut().unwrap().push(json!(
            {"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3"}));
        v["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(4);
    });
    let mut d = doc();
    let report = import(&mut d, &path);
    assert_eq!(report["textures"], 1, "{report}");
    assert!(
        report["warnings"].to_string().contains("vertex colors"),
        "{report}"
    );
    let materials = ask(&d, json!({"type": "materials"}))["materials"].clone();
    assert_eq!(materials.as_array().unwrap().len(), 1);
    assert!(!materials[0]["texture"].is_null());
}

/// The `textured.glb` fixture with a second coordinate set (u + 0.25, v) added as
/// `TEXCOORD_1`, then changed by `change`.
fn two_sets(name: &str, change: impl Fn(&mut Value)) -> String {
    let glb = std::fs::read(fixture("textured.glb")).unwrap();
    let word = |at: usize| u32::from_le_bytes(glb[at..at + 4].try_into().unwrap()) as usize;
    let json_len = word(12);
    let bin = &glb[20 + json_len + 8..];
    let first: Vec<f32> = bin[96..128]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let second: Vec<f32> = first
        .as_chunks::<2>()
        .0
        .iter()
        .flat_map(|uv| [uv[0] + 0.25, uv[1]])
        .collect();
    let mut bin = bin.to_vec();
    bin.extend_from_slice(&pack_f32(&second));
    let mut v: Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    v["buffers"][0]["byteLength"] = json!(bin.len());
    v["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer": 0, "byteOffset": bin.len() - 32, "byteLength": 32}));
    v["accessors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView": 5, "componentType": 5126, "count": 4, "type": "VEC2"}));
    v["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_1"] = json!(4);
    change(&mut v);
    write_glb(name, v, &bin)
}

/// How far the first material's image sits from the corners' x, in repeats along x, mod 1.
fn x_shift(d: &Document) -> f64 {
    let spots = image_spots(d);
    let shifts: Vec<f64> = spots
        .iter()
        .map(|(c, st)| (st.x - c.x).rem_euclid(1.0))
        .collect();
    let first = shifts[0];
    for s in &shifts {
        let d = (s - first).abs();
        assert!(d < 1e-6 || (1.0 - d) < 1e-6, "{shifts:?}");
    }
    first
}

#[test]
fn a_texture_reads_the_coordinate_set_its_tex_coord_names() {
    let mut d = doc();
    import(&mut d, &two_sets("sets-0.glb", |_| {}));
    assert!(x_shift(&d) < 1e-6 || x_shift(&d) > 1.0 - 1e-6);
    let path = two_sets("sets-1.glb", |v| {
        v["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"] = json!(1);
    });
    let mut d = doc();
    import(&mut d, &path);
    assert!((x_shift(&d) - 0.25).abs() < 1e-6, "{}", x_shift(&d));

    // KHR_texture_transform's texCoord overrides the texture's, and its offset adds.
    let path = two_sets("sets-transform.glb", |v| {
        v["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] = json!({
            "KHR_texture_transform": {"texCoord": 1, "offset": [0.5, 0.0]}});
    });
    let mut d = doc();
    import(&mut d, &path);
    assert!((x_shift(&d) - 0.75).abs() < 1e-6, "{}", x_shift(&d));
}

#[test]
fn a_quarter_turn_with_unequal_scales_is_laid_exactly() {
    let (offset, rotation, scale) = ([0.3, 0.1], std::f64::consts::FRAC_PI_2, [1.0, 3.0]);
    let path = textured_with("quarter.glb", |v| {
        v["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"] = json!({
            "KHR_texture_transform": {"offset": offset, "rotation": rotation, "scale": scale}
        });
    });
    let mut d = doc();
    let report = import(&mut d, &path);
    assert!(
        report["warnings"].as_array().is_none_or(Vec::is_empty),
        "{report}"
    );
    let (sin, cos) = rotation.sin_cos();
    for (c, st) in image_spots(&d) {
        let (u, v) = (c.x * scale[0], -c.y * scale[1]);
        let (u, v) = (
            cos * u + sin * v + offset[0],
            -sin * u + cos * v + offset[1],
        );
        let want = DVec2::new(u, -v);
        let d = st - want;
        assert!((d - d.round()).length() < 1e-6, "{c}: {st} not {want}");
    }
}

fn encode_vertices(bytes: &[u8], stride: usize) -> Vec<u8> {
    use meshopt_rs::vertex::VertexEncodingVersion;
    use meshopt_rs::vertex::buffer::{encode_vertex_buffer, encode_vertex_buffer_bound};
    let count = bytes.len() / stride;
    let mut buf = vec![0u8; encode_vertex_buffer_bound(count, stride)];
    macro_rules! go {
        ($n:literal) => {{
            let v: Vec<[u8; $n]> = bytes
                .chunks_exact($n)
                .map(|c| c.try_into().unwrap())
                .collect();
            encode_vertex_buffer(&mut buf, &v, VertexEncodingVersion::V0).unwrap()
        }};
    }
    let n = match stride {
        4 => go!(4),
        12 => go!(12),
        _ => unreachable!(),
    };
    buf.truncate(n);
    buf
}

/// A glb of one unit square (four corners, six indices) with positions, octahedral normals
/// and indices meshopt compressed; `corrupt` may damage the compressed bytes.
fn meshopt_square(name: &str, corrupt: impl Fn(&mut Vec<u8>)) -> String {
    use meshopt_rs::index::IndexEncodingVersion;
    use meshopt_rs::index::buffer::{encode_index_buffer, encode_index_buffer_bound};
    let positions = pack_f32(&[0., 0., 0., 1., 0., 0., 1., 0., -1., 0., 0., -1.]);
    let normals: Vec<u8> = (0..4).flat_map(|_| [0u8, 0, 127, 0]).collect();
    let indices = [0u32, 1, 2, 0, 2, 3];
    let mut buf = vec![0u8; encode_index_buffer_bound(6, 4)];
    let n = encode_index_buffer(&mut buf, &indices, IndexEncodingVersion::V1).unwrap();
    buf.truncate(n);
    let parts = [
        encode_vertices(&positions, 12),
        encode_vertices(&normals, 4),
        buf,
    ];
    let mut bin = Vec::new();
    let mut at = Vec::new();
    for p in &parts {
        at.push((bin.len(), p.len()));
        bin.extend_from_slice(p);
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
    }
    corrupt(&mut bin);
    let view = |i: usize, count: usize, stride: usize, mode: &str, filter: Option<&str>| {
        let mut ext = json!({"buffer": 0, "byteOffset": at[i].0, "byteLength": at[i].1,
            "byteStride": stride, "count": count, "mode": mode});
        if let Some(f) = filter {
            ext["filter"] = json!(f);
        }
        json!({"buffer": 1, "byteOffset": 0, "byteLength": count * stride,
            "extensions": {"EXT_meshopt_compression": ext}})
    };
    write_glb(
        name,
        json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": ["EXT_meshopt_compression"],
            "extensionsRequired": ["EXT_meshopt_compression"],
            "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{
                "attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2}]}],
            "bufferViews": [
                view(0, 4, 12, "ATTRIBUTES", None),
                view(1, 4, 4, "ATTRIBUTES", Some("OCTAHEDRAL")),
                view(2, 6, 4, "TRIANGLES", None)],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5120, "normalized": true, "count": 4,
                 "type": "VEC3"},
                {"bufferView": 2, "componentType": 5125, "count": 6, "type": "SCALAR"}],
            "buffers": [{"byteLength": bin.len()},
                {"byteLength": 4 * 12 + 4 * 4 + 6 * 4,
                 "extensions": {"EXT_meshopt_compression": {"fallback": true}}}],
        }),
        &bin,
    )
}

#[test]
fn a_meshopt_compressed_glb_imports_as_the_plain_one_does() {
    let mut d = doc();
    let report = import(&mut d, &meshopt_square("meshopt.glb", |_| {}));
    assert_eq!(report["faces"], 1, "{report}");
    assert!(
        report["warnings"].as_array().is_none_or(Vec::is_empty),
        "{report}"
    );
    let points = corners(&d);
    let (lo, hi) = bounds(&points);
    // glTF (x, y, z) is the model's (x, -z, y): the square spans x 0-1, y 0-1 at z 0.
    assert!(
        (lo - DVec3::ZERO).length() < 1e-9 && (hi - DVec3::new(1.0, 1.0, 0.0)).length() < 1e-9,
        "{lo} {hi}"
    );

    // Damaged compressed bytes are a file error, and change nothing.
    let path = meshopt_square("meshopt-bad.glb", |bin| {
        for b in bin.iter_mut().take(20) {
            *b = 0xff;
        }
    });
    let mut d = doc();
    let err = run(&mut d, json!({"type": "import", "path": path})).unwrap_err();
    assert_eq!(err.0, ErrorKind::File, "{}", err.1);
    assert!(err.1.contains("buffer view"), "{}", err.1);
    assert!(d.scene().triangles().is_empty());
}

/// A glb of the square's two triangles (six corners) with `KHR_draco_mesh_compression`:
/// positions and red-then-green byte colors, read through a color accessor of `color_kind`.
fn draco_square(name: &str, damage: bool, color_kind: &str) -> String {
    use draco_core::{
        DataType, EncoderBuffer, EncoderOptions, GeometryAttributeType, Mesh, MeshEncoder,
        PointAttribute, PointIndex,
    };
    let mut mesh = Mesh::new();
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        6,
    );
    for (i, p) in SQUARE.iter().enumerate() {
        position.buffer_mut().write(i * 12, &pack_f32(p));
    }
    let position_id = mesh.add_attribute(position);
    let mut color = PointAttribute::new();
    color.init(GeometryAttributeType::Color, 3, DataType::Uint8, true, 6);
    for i in 0..6 {
        let rgb: [u8; 3] = if i < 3 { [255, 0, 0] } else { [0, 255, 0] };
        color.buffer_mut().write(i * 3, &rgb);
    }
    let color_id = mesh.add_attribute(color);
    for f in [[0, 1, 2], [3, 4, 5]] {
        mesh.add_face(f.map(PointIndex));
    }
    let (position_uid, color_uid) = (
        mesh.attribute(position_id).unique_id(),
        mesh.attribute(color_id).unique_id(),
    );
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut out = EncoderBuffer::new();
    encoder.encode(&EncoderOptions::new(), &mut out).unwrap();
    let mut bin = out.data().to_vec();
    if damage {
        let n = bin.len();
        for b in &mut bin[n / 3..] {
            *b = 0xA5;
        }
    }
    write_glb(
        name,
        json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": ["KHR_draco_mesh_compression"],
            "extensionsRequired": ["KHR_draco_mesh_compression"],
            "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{
                "attributes": {"POSITION": 0, "COLOR_0": 1}, "indices": 2,
                "extensions": {"KHR_draco_mesh_compression": {"bufferView": 0,
                    "attributes": {"POSITION": position_uid, "COLOR_0": color_uid}}}}]}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": bin.len()}],
            "accessors": [
                {"componentType": 5126, "count": 6, "type": "VEC3"},
                {"componentType": 5121, "normalized": true, "count": 6, "type": color_kind},
                {"componentType": 5125, "count": 6, "type": "SCALAR"}],
            "buffers": [{"byteLength": bin.len()}],
        }),
        &bin,
    )
}

#[test]
fn a_draco_compressed_glb_imports_with_its_colors() {
    let mut d = doc();
    let report = import(&mut d, &draco_square("draco.glb", false, "VEC3"));
    assert_eq!(report["faces"], 2, "{report}");
    let mut got = material_colors(&d);
    got.sort();
    assert_eq!(got, vec![vec![0, 255, 0], vec![255, 0, 0]]);
    let (lo, hi) = bounds(&corners(&d));
    assert!(
        (lo - DVec3::ZERO).length() < 1e-3 && (hi - DVec3::new(1.0, 1.0, 0.0)).length() < 1e-3,
        "{lo} {hi}"
    );

    let mut d = doc();
    let err = run(
        &mut d,
        json!({"type": "import", "path": draco_square("draco-bad.glb", true, "VEC3")}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::File, "{}", err.1);
    assert!(err.1.contains("Draco"), "{}", err.1);
    assert!(d.scene().triangles().is_empty());

    // A color accessor that is not VEC3 or VEC4 leaves the colors out: one plain square.
    for kind in ["SCALAR", "VEC2"] {
        let mut d = doc();
        let report = import(
            &mut d,
            &draco_square(&format!("draco-{kind}.glb"), false, kind),
        );
        assert_eq!(report["faces"], 1, "{report}");
        assert!(material_colors(&d).is_empty(), "{kind}");
    }
}
