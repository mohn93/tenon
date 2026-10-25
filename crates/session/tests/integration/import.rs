//! Importing OBJ and STL, driven through JSON commands.

use std::path::PathBuf;

use glam::DVec2;
use kernel::Triangle;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_POLYGONS, Query};

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
        .join(format!("session-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn write(name: &str, text: &str) -> String {
    let path = scratch(name);
    std::fs::write(&path, text).unwrap();
    path.to_str().unwrap().to_string()
}

/// A 2 x 2 x 1 box with its top painted red, plus a 1 x 1 x 3 tower made a component standing
/// on a turned instance 5 m away: faces in many planes, some painted.
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
        json!({"type": "add_rectangle", "corner": [5, 0, 0], "opposite": [6, 1, 0]}),
    )
    .unwrap();
    let base = ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["id"]
        .clone();
    run(
        d,
        json!({"type": "push_pull", "face": base, "distance": 3.0}),
    )
    .unwrap();
    let all = ask(
        d,
        json!({"type": "entities", "within": {"min": [4, -1, -1], "max": [7, 2, 4]}}),
    );
    let ids: Vec<Value> = ["edges", "faces"]
        .iter()
        .flat_map(|k| all[k].as_array().unwrap().iter().map(|e| e["id"].clone()))
        .collect();
    run(
        d,
        json!({"type": "make_component", "ids": ids, "name": "Tower"}),
    )
    .unwrap();
    let tower = ask(d, json!({"type": "selection"}))["ids"][0].clone();
    run(
        d,
        json!({"type": "rotate", "ids": [tower], "center": [5, 0, 0], "angle": 30}),
    )
    .unwrap();
}

/// Asserts two sets of triangles show the same surface, however it is cut: the same corners
/// (to `tol`) and, for each normal direction, the same area.
fn same_surface(a: &[Triangle], b: &[Triangle], tol: f64) {
    let corners = |tris: &[Triangle]| {
        let mut out: Vec<glam::DVec3> = Vec::new();
        for p in tris.iter().flat_map(|t| t.corners) {
            if !out.iter().any(|q| q.distance(p) < tol) {
                out.push(p);
            }
        }
        out
    };
    let (ca, cb) = (corners(a), corners(b));
    assert_eq!(ca.len(), cb.len());
    assert!(ca.iter().all(|p| cb.iter().any(|q| q.distance(*p) < tol)));
    let areas = |tris: &[Triangle]| {
        let mut out: Vec<(glam::DVec3, f64)> = Vec::new();
        for t in tris {
            let [p, q, r] = t.corners;
            let area = (q - p).cross(r - p).length() / 2.0;
            match out.iter_mut().find(|(n, _)| n.distance(t.normal) < 1e-4) {
                Some((_, s)) => *s += area,
                None => out.push((t.normal, area)),
            }
        }
        out
    };
    let (aa, ab) = (areas(a), areas(b));
    assert_eq!(aa.len(), ab.len());
    for (n, s) in aa {
        let other = ab
            .iter()
            .find(|(m, _)| m.distance(n) < 1e-4)
            .expect("same normals");
        assert!((other.1 - s).abs() < tol, "{n}: {s} vs {}", other.1);
    }
}

fn export_import(name: &str) -> (Document, Document, Value) {
    let mut d = doc();
    model(&mut d);
    let path = scratch(name);
    let path = path.to_str().unwrap();
    run(&mut d, json!({"type": "export", "path": path})).unwrap();
    let mut e = doc();
    let r = run(&mut e, json!({"type": "import", "path": path})).unwrap();
    e.scene().validate().unwrap();
    (d, e, r)
}

#[test]
fn obj_export_then_import_gives_back_the_faces_and_colors() {
    let (d, e, r) = export_import("round.obj");
    same_surface(&d.scene().triangles(), &e.scene().triangles(), 1e-9);
    // The box's 6 faces and the tower's 6 merge back from 24 triangles.
    let report = &r["import"];
    assert_eq!(report["polygons"], 24);
    assert_eq!(report["faces"], 12);
    assert_eq!(report["skipped"], 0);
    assert_eq!(report["vertices"], 16);
    assert_eq!(
        report["materials"], 1,
        "Red comes from the .mtl; default stays default"
    );
    let materials = ask(&e, json!({"type": "materials"}))["materials"].clone();
    assert_eq!(materials.as_array().unwrap().len(), 1);
    assert_eq!(materials[0]["name"], "Red");
    assert_eq!(materials[0]["color"], json!([200, 30, 20]));
    assert_eq!(materials[0]["faces"], 1);
    // One group named after the file, selected, holding everything.
    let instance = report["instance"].clone();
    assert_eq!(
        ask(&e, json!({"type": "selection"}))["ids"],
        json!([instance])
    );
    let defs = ask(&e, json!({"type": "definitions"}))["definitions"].clone();
    assert_eq!(defs[0]["name"], "round");
    assert_eq!(defs[0]["kind"], "group");
    assert_eq!(defs[0]["id"], report["definition"]);
    assert_eq!(defs[0]["faces"], 12);
}

#[test]
fn stl_export_then_import_gives_back_the_faces() {
    let (d, e, r) = export_import("round.stl");
    // STL stores f32 corners: equal to a few micrometers.
    same_surface(&d.scene().triangles(), &e.scene().triangles(), 1e-5);
    assert_eq!(r["import"]["faces"], 12);
    assert_eq!(r["import"]["materials"], 0);
}

#[test]
fn up_axis_and_scale_place_the_file_in_model_coordinates() {
    let tri = "v 0 0 0\nv 1 0 0\nv 0 2 0\nf 1 2 3\n";
    let path = write("tri.obj", tri);
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "import", "path": path, "as": "context"}),
    )
    .unwrap();
    // Y-up: the file's y is the model's z.
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0].clone();
    assert_eq!(face["normal"], json!([0.0, -1.0, 0.0]));
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "import", "path": path, "as": "context", "up": "z", "scale": 0.001}),
    )
    .unwrap();
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0].clone();
    assert_eq!(face["normal"], json!([0.0, 0.0, 1.0]));
    assert!((face["area"].as_f64().unwrap() - 1e-6).abs() < 1e-15);
}

#[test]
fn import_as_component_or_into_the_context_is_one_undo_step() {
    let path = write(
        "quad.obj",
        "v 0 0 0\nv 2 0 0\nv 2 0 -2\nv 0 0 -2\nf 1 2 3 4\n",
    );
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": path, "as": "component"}),
    )
    .unwrap();
    let defs = ask(&d, json!({"type": "definitions"}))["definitions"].clone();
    assert_eq!(
        (defs[0]["kind"].clone(), defs[0]["name"].clone()),
        (json!("component"), json!("quad"))
    );
    assert_eq!(r["import"]["faces"], 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "definitions"}))["definitions"],
        json!([])
    );
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "stats"}))["instances"], 1);

    // Into the context, a square drawn first is split by the imported one it overlaps.
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [1, 1, 0], "opposite": [3, 3, 0]}),
    )
    .unwrap();
    let before = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    let r = run(
        &mut d,
        json!({"type": "import", "path": path, "as": "context"}),
    )
    .unwrap();
    assert!(r["import"].get("instance").is_none());
    let stats = ask(&d, json!({"type": "stats"}));
    assert_eq!(
        (stats["instances"].clone(), stats["definitions"].clone()),
        (json!(0), json!(0))
    );
    assert_eq!(
        stats["faces"], 3,
        "the overlap, and the rest of each square"
    );
    let selected = ask(&d, json!({"type": "selection"}))["ids"]
        .as_array()
        .unwrap()
        .len();
    assert!(selected > 0);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], before);
    assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 1);
}

#[test]
fn importing_inside_an_open_group_places_the_new_group_there() {
    let path = write(
        "small.stl",
        "solid s\nfacet normal 0 0 1\nouter loop\nvertex 0 0 5\nvertex 1 0 5\nvertex 0 1 5\nendloop\nendfacet\nendsolid s\n",
    );
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "make_group", "ids": ["f8"]})).unwrap();
    let outer = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    run(&mut d, json!({"type": "open_instance", "id": outer})).unwrap();
    let r = run(&mut d, json!({"type": "import", "path": path})).unwrap();
    let inner = r["import"]["instance"].clone();
    assert_eq!(
        ask(&d, json!({"type": "entities", "kinds": ["instance"]}))["instances"][0]["id"],
        inner
    );
    let tree = ask(&d, json!({"type": "outliner"}))["instances"].clone();
    assert_eq!(tree[0]["children"][0]["id"], inner);
    assert_eq!(tree[0]["children"][0]["name"], "small");
}

#[test]
fn bad_requests_and_bad_files_change_nothing() {
    let mut d = doc();
    let check = |d: &mut Document, v: Value, kind: ErrorKind, text: &str| {
        let (k, m) = run(d, v.clone()).unwrap_err();
        assert_eq!(k, kind, "{v}: {m}");
        assert!(m.contains(text), "{v}: {m}");
    };
    let ok = write("ok.obj", "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    check(
        &mut d,
        json!({"type": "import", "path": write("m.skr", "")}),
        ErrorKind::InvalidInput,
        "use open",
    );
    check(
        &mut d,
        json!({"type": "import", "path": write("m.dwg", "")}),
        ErrorKind::InvalidInput,
        ".obj, .stl, .skp, .gltf, .glb or .dae",
    );
    for scale in [0.0, -1.0] {
        check(
            &mut d,
            json!({"type": "import", "path": ok, "scale": scale}),
            ErrorKind::InvalidInput,
            "scale",
        );
    }
    check(
        &mut d,
        json!({"type": "import", "path": ok, "scale": 1e7}),
        ErrorKind::File,
        "beyond",
    );
    let missing = scratch("missing.obj");
    check(
        &mut d,
        json!({"type": "import", "path": missing}),
        ErrorKind::File,
        "missing.obj",
    );
    let flat = write("flat.obj", "v 0 0 0\nv 1 0 0\nv 2 0 0\nf 1 2 3\n");
    check(
        &mut d,
        json!({"type": "import", "path": flat}),
        ErrorKind::File,
        "nothing to import",
    );
    let broken = write("broken.obj", "v 0 0 0\nf 1 2 3\n");
    check(
        &mut d,
        json!({"type": "import", "path": broken}),
        ErrorKind::File,
        "line 2",
    );
    // A format given explicitly wins over the extension.
    check(
        &mut d,
        json!({"type": "import", "path": ok, "format": "stl"}),
        ErrorKind::File,
        "STL",
    );
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 0);
    assert_eq!(ask(&d, json!({"type": "stats"}))["revision"], 0);
    assert_eq!(
        ask(&d, json!({"type": "materials"}))["materials"],
        json!([])
    );
}

#[test]
fn a_file_over_the_polygon_limit_is_refused() {
    let n = MAX_IMPORT_POLYGONS as u32 + 1;
    let mut bin = vec![0u8; 80];
    bin.extend_from_slice(&n.to_le_bytes());
    bin.resize(84 + 50 * n as usize, 0);
    let path = scratch("huge.stl");
    std::fs::write(&path, &bin).unwrap();
    let (kind, message) = run(&mut doc(), json!({"type": "import", "path": path})).unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("more than"), "{message}");
}

#[test]
fn obj_materials_reuse_the_models_own_and_skip_missing_libraries() {
    let dir = scratch("mats.obj");
    std::fs::write(
        dir.with_extension("mtl"),
        "newmtl Red\nKd 1 0 0\nnewmtl Blue\nKd 0 0 1\n",
    )
    .unwrap();
    let path = write(
        "mats.obj",
        "mtllib mats.mtl missing.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\n\
         usemtl Red\nf 1 2 3\nusemtl Blue\nf 1 3 4\nusemtl Green\nf 1 2 5\nusemtl default\nf 1 4 5\n",
    );
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_material", "name": "Red", "color": [9, 9, 9]}),
    )
    .unwrap();
    let r = run(&mut d, json!({"type": "import", "path": path, "up": "z"})).unwrap();
    assert_eq!(r["import"]["materials"], 1, "only Blue is new");
    assert_eq!(r["changes"]["materials"].as_array().unwrap().len(), 1);
    let m = ask(&d, json!({"type": "materials"}))["materials"].clone();
    let by_name = |n: &str| {
        m.as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == n)
            .unwrap()
            .clone()
    };
    assert_eq!(
        by_name("Red")["color"],
        json!([9, 9, 9]),
        "the model's Red wins"
    );
    assert_eq!(by_name("Red")["faces"], 1);
    assert_eq!(by_name("Blue")["color"], json!([0, 0, 255]));
    assert_eq!(by_name("Blue")["faces"], 1);
    // Undo takes the added material away with the faces.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "materials"}))["materials"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn import_works_in_an_atomic_batch_and_rolls_back_with_it() {
    let path = write("batch.obj", "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    let mut d = doc();
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "import", "path": path},
        {"type": "erase", "ids": ["f999"]}
    ]))
    .unwrap();
    let e = d.execute_batch(commands, true).unwrap_err();
    assert!(e.rolled_back);
    assert_eq!(ask(&d, json!({"type": "stats"}))["instances"], 0);
    assert_eq!(
        ask(&d, json!({"type": "definitions"}))["definitions"],
        json!([])
    );
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "import", "path": path},
        {"type": "import", "path": path, "as": "component"}
    ]))
    .unwrap();
    let outcomes = d.execute_batch(commands, true).unwrap();
    assert_eq!(outcomes[1].import.as_ref().unwrap().faces, 1);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
    assert_eq!(ask(&d, json!({"type": "stats"}))["instances"], 2);
}

#[test]
fn degenerate_polygons_are_counted_and_points_weld() {
    // Two triangles sharing an edge, written with their own corners (as STL does) and a
    // hair apart; a sliver; and a repeat.
    let path = write(
        "weld.obj",
        "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 0 0.0000001\nv 1 1 0\nv 0 1 0\nv 0.5 0.5 0\n\
         f 1 2 3\nf 4 5 6\nf 1 7 3\nf 3 2 1\n",
    );
    let mut d = doc();
    let r = run(&mut d, json!({"type": "import", "path": path, "up": "z"})).unwrap();
    assert_eq!(r["import"]["polygons"], 4);
    assert_eq!(r["import"]["skipped"], 2);
    assert_eq!(
        r["import"]["faces"], 1,
        "the two triangles merge into a square"
    );
    assert_eq!(r["import"]["vertices"], 4);
}

#[test]
fn import_appears_in_the_command_schema() {
    let schema = serde_json::to_value(schemars::schema_for!(Command))
        .unwrap()
        .to_string();
    assert!(schema.contains("merge_coplanar"));
    assert!(schema.contains("\"as\""));
}

#[test]
fn a_big_file_goes_into_the_context_only_as_a_group() {
    // A row of separate triangles, one more than the context takes.
    let n = MAX_CONTEXT_IMPORT_FACES + 1;
    let mut text = String::new();
    for k in 0..n {
        let x = 2 * k;
        text += &format!("v {x} 0 0\nv {} 0 0\nv {x} 1 0\nf -3 -2 -1\n", x + 1);
    }
    let path = write("row.obj", &text);
    let mut d = doc();
    let (kind, message) = run(
        &mut d,
        json!({"type": "import", "path": path, "as": "context"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("as a group"), "{message}");
    assert_eq!(ask(&d, json!({"type": "stats"}))["revision"], 0);
    assert_eq!(
        ask(&d, json!({"type": "definitions"}))["definitions"],
        json!([])
    );
    let r = run(&mut d, json!({"type": "import", "path": path})).unwrap();
    assert_eq!(r["import"]["faces"], n);
}

#[test]
fn only_regular_files_are_read_and_absolute_libraries_are_ignored() {
    let mut d = doc();
    let dir = scratch("a_directory.obj");
    std::fs::create_dir_all(&dir).unwrap();
    let (kind, message) = run(
        &mut d,
        json!({"type": "import", "path": dir.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("regular file"), "{message}");

    // A library naming a directory, and one by absolute path, are not read.
    let abs = scratch("abs.mtl");
    std::fs::write(&abs, "newmtl Red\nKd 1 0 0\n").unwrap();
    let text = format!(
        "mtllib . {}\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl Red\nf 1 2 3\n",
        abs.to_str().unwrap()
    );
    let path = write("libs.obj", &text);
    let r = run(&mut d, json!({"type": "import", "path": path})).unwrap();
    assert_eq!(r["import"]["materials"], 0);
    assert_eq!(r["import"]["faces"], 1);
}

#[test]
fn a_face_over_the_corner_limit_is_refused() {
    let mut text = String::from("v 0 0 0\n");
    text += &format!("f{}\n", " 1".repeat(session::MAX_IMPORT_POINTS + 1));
    let path = write("corners.obj", &text);
    let mut d = doc();
    let (kind, message) = run(&mut d, json!({"type": "import", "path": path})).unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("corners"), "{message}");
    let mut stl = String::from("solid t\nouter loop\n");
    stl += &"vertex 0 0 0\n".repeat(session::MAX_IMPORT_POINTS + 1);
    let path = write("corners.stl", &stl);
    let (kind, message) = run(&mut d, json!({"type": "import", "path": path})).unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("more than"), "{message}");
    assert_eq!(ask(&d, json!({"type": "stats"}))["revision"], 0);
}

#[test]
fn colors_survive_an_export_to_a_name_with_spaces() {
    let mut d = doc();
    model(&mut d);
    let path = scratch("my model.obj");
    run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut e = doc();
    let r = run(
        &mut e,
        json!({"type": "import", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(r["import"]["materials"], 1);
    let m = ask(&e, json!({"type": "materials"}))["materials"].clone();
    assert_eq!(m[0]["name"], "Red");
    assert_eq!(m[0]["color"], json!([200, 30, 20]));
    assert_eq!(m[0]["faces"], 1);
}

#[test]
fn a_big_library_and_many_material_names_import_fast() {
    // A 50,000-entry library; the OBJ names 1,000 of them on distinct triangles and 20,000
    // more names the library lacks.
    let library = 50_000;
    let mtl: String = (0..library)
        .map(|i| format!("newmtl m{i}\nKd 0.5 0.5 0.5\n"))
        .collect();
    write("many.mtl", &mtl);
    let mut obj = String::from("mtllib many.mtl\n");
    for i in 0..1_000 {
        let x = 3.0 * i as f64;
        obj += &format!("v {x} 0 0\nv {} 0 0\nv {x} 1 0\n", x + 1.0);
    }
    for i in 0..1_000 {
        obj += &format!("usemtl m{i}\nf {} {} {}\n", 3 * i + 1, 3 * i + 2, 3 * i + 3);
    }
    for i in 0..20_000 {
        obj += &format!("usemtl nolib{i}\nf 1 2 3\n");
    }
    let path = write("many.obj", &obj);
    let mut d = doc();
    let start = std::time::Instant::now();
    let r = run(&mut d, json!({"type": "import", "path": path, "up": "z"})).unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(r["import"]["faces"], 1_000);
    assert_eq!(r["import"]["skipped"], 20_000);
    assert_eq!(r["import"]["materials"], 1_000);
    d.scene().validate().unwrap();
}

#[test]
fn materials_only_skipped_polygons_use_are_not_created() {
    write(
        "skip.mtl",
        "newmtl Used\nKd 1 0 0\nnewmtl Unused\nKd 0 1 0\n",
    );
    let path = write(
        "skip.obj",
        "mtllib skip.mtl\nv 0 0 0\nv 1 0 0\nv 2 0 0\nv 0 1 0\n\
         usemtl Unused\nf 1 2 3\nusemtl Used\nf 1 2 4\n",
    );
    let mut d = doc();
    let r = run(&mut d, json!({"type": "import", "path": path, "up": "z"})).unwrap();
    assert_eq!(r["import"]["skipped"], 1);
    assert_eq!(r["import"]["materials"], 1);
    let m = ask(&d, json!({"type": "materials"}))["materials"].clone();
    assert_eq!(m.as_array().unwrap().len(), 1);
    assert_eq!(m[0]["name"], "Used");
    // Undo leaves no material behind.
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(
        ask(&d, json!({"type": "materials"}))["materials"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn more_new_materials_than_the_limit_is_a_file_error() {
    let names = session::MAX_IMPORT_MATERIALS + 1;
    // Distinct triangles so every name builds a face.
    let mut obj = String::from("mtllib cap.mtl\n");
    for i in 0..names {
        let x = 3.0 * i as f64;
        obj += &format!("v {x} 0 0\nv {} 0 0\nv {x} 1 0\n", x + 1.0);
    }
    for i in 0..names {
        obj += &format!("usemtl m{i}\nf {} {} {}\n", 3 * i + 1, 3 * i + 2, 3 * i + 3);
    }
    let mtl: String = (0..names)
        .map(|i| format!("newmtl m{i}\nKd 0.5 0.5 0.5\n"))
        .collect();
    write("cap.mtl", &mtl);
    let path = write("cap.obj", &obj);
    let mut d = doc();
    let (kind, message) = run(&mut d, json!({"type": "import", "path": path})).unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("1024"), "{message}");
    assert_eq!(ask(&d, json!({"type": "stats"}))["revision"], 0);
    assert!(
        ask(&d, json!({"type": "materials"}))["materials"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Exactly at the limit imports.
    let ok = names - 1;
    let obj_ok: String = obj
        .lines()
        .filter(|l| !l.starts_with("usemtl") && !l.starts_with("f "))
        .map(|l| format!("{l}\n"))
        .chain(
            (0..ok).map(|i| format!("usemtl m{i}\nf {} {} {}\n", 3 * i + 1, 3 * i + 2, 3 * i + 3)),
        )
        .collect();
    let path = write("cap.obj", &obj_ok);
    let r = run(&mut d, json!({"type": "import", "path": path})).unwrap();
    assert_eq!(r["import"]["materials"], ok);
}

#[test]
fn only_the_exact_name_default_is_the_default_material() {
    write("case.mtl", "newmtl Default\nKd 0 0 1\n");
    let path = write(
        "case.obj",
        "mtllib case.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\n\
         usemtl Default\nf 1 2 3\nusemtl default\nf 2 4 3\n",
    );
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": path, "up": "z", "merge_coplanar": false}),
    )
    .unwrap();
    assert_eq!(r["import"]["materials"], 1);
    let m = ask(&d, json!({"type": "materials"}))["materials"].clone();
    assert_eq!(m[0]["name"], "Default");
    assert_eq!(m[0]["faces"], 1);
}

#[test]
fn a_bent_polygon_is_cut_into_triangles_even_when_merging_is_off() {
    // Not flat: a quad with one corner lifted becomes two triangles, never one face.
    let path = write(
        "bent.obj",
        "v 0 0 0\nv 1 0 0\nv 1 1 0.5\nv 0 1 0\nf 1 2 3 4\n",
    );
    let mut d = doc();
    let r = run(
        &mut d,
        json!({"type": "import", "path": path, "up": "z", "merge_coplanar": false}),
    )
    .unwrap();
    assert_eq!(r["import"]["faces"], 2);
    d.scene().validate().unwrap();
}

#[test]
fn imported_geometry_survives_later_edits() {
    let path = write(
        "cube.obj",
        "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
         f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n",
    );
    let mut d = doc();
    let r = run(&mut d, json!({"type": "import", "path": path, "up": "z"})).unwrap();
    assert_eq!(r["import"]["faces"], 6);
    let instance = r["import"]["instance"].clone();
    run(&mut d, json!({"type": "open_instance", "id": instance})).unwrap();
    let top = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap()["id"]
        .clone();
    run(
        &mut d,
        json!({"type": "push_pull", "face": top, "distance": 0.5}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    run(&mut d, json!({"type": "explode", "ids": [instance]})).unwrap();
    d.scene().validate().unwrap();
    let faces = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"].clone();
    assert_eq!(faces.as_array().unwrap().len(), 6);
}

#[test]
fn import_errors_do_not_quote_the_file() {
    let cases: [(&str, &str); 4] = [
        ("secret1.obj", "v 0 0 0\nv hunter2-SECRET 0 0\n"),
        (
            "secret2.obj",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 hunter2-SECRET\n",
        ),
        (
            "secret3.stl",
            "solid x\nfacet normal 0 0 1\nouter loop\nvertex hunter2-SECRET 0 0\n",
        ),
        ("secret4.obj", "hunter2-SECRET\nv nan 0 0\n"),
    ];
    for (name, text) in cases {
        let path = scratch(name);
        std::fs::write(&path, text).unwrap();
        let (_, message) = run(&mut doc(), json!({"type": "import", "path": path})).unwrap_err();
        assert!(!message.contains("hunter2"), "{name}: {message}");
        assert!(
            message.contains("line ") || message.contains("facet "),
            "{message}"
        );
    }
}
