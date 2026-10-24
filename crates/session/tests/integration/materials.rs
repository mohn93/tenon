//! Materials and painting, driven through JSON commands and queries.

use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String, Vec<String>)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| {
            (
                e.kind,
                e.message,
                e.ids.iter().map(|i| i.to_string()).collect(),
            )
        })
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

/// A 2 x 2 x 1 box: `f8` is its bottom (facing down), `f25` its top.
fn cube(d: &mut Document) {
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
}

/// Adds a material and returns its id.
fn add(d: &mut Document, name: &str, color: [u8; 3]) -> String {
    let out = run(
        d,
        json!({"type": "add_material", "name": name, "color": color}),
    )
    .unwrap();
    out["changes"]["materials"][0].as_str().unwrap().to_string()
}

fn materials(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "materials"}))["materials"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn materials_are_added_renamed_recolored_and_deleted() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_material", "color": [10, 20, 30]}),
    )
    .unwrap();
    let first = out["changes"]["materials"][0].as_str().unwrap().to_string();
    assert!(first.starts_with('m'), "{first}");
    assert!(out["changes"]["created"].as_array().unwrap().is_empty());
    assert_eq!(d.revision(), 1, "an undo step that changes the revision");
    let second = run(&mut d, json!({"type": "add_material", "color": [0, 0, 0]})).unwrap()
        ["changes"]["materials"][0]
        .clone();
    let listed = materials(&d);
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed[0],
        json!({"id": first, "name": "Material", "color": [10, 20, 30], "opacity": 1.0,
            "emission": 0.0, "metalness": 0.0, "roughness": 1.0, "faces": 0,
            "instances": 0})
    );
    assert_eq!(listed[1]["id"], second);
    assert_eq!(listed[1]["name"], "Material 2");

    run(
        &mut d,
        json!({"type": "change_material", "material": first, "name": "Brick", "color": [180, 60, 40]}),
    )
    .unwrap();
    assert_eq!(materials(&d)[0]["name"], "Brick");
    assert_eq!(materials(&d)[0]["color"], json!([180, 60, 40]));
    run(
        &mut d,
        json!({"type": "change_material", "material": first, "color": [1, 2, 3]}),
    )
    .unwrap();
    assert_eq!(materials(&d)[0]["name"], "Brick", "absent fields stay");

    let out = run(
        &mut d,
        json!({"type": "delete_material", "material": first}),
    )
    .unwrap();
    assert_eq!(out["changes"]["materials"], json!([first]));
    assert_eq!(materials(&d).len(), 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[0]["color"], json!([1, 2, 3]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(materials(&d)[0]["name"], "Material");
}

#[test]
fn bad_material_commands_are_refused() {
    let mut d = doc();
    let red = add(&mut d, "Red", [200, 0, 0]);
    let invalid = |r: Result<Value, (ErrorKind, String, Vec<String>)>, why: &str| {
        let (kind, message, _) = r.unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{message}");
        assert!(message.contains(why), "{message:?} lacks {why:?}");
    };
    invalid(
        run(
            &mut d,
            json!({"type": "add_material", "name": "Red", "color": [0, 0, 0]}),
        ),
        "already exists",
    );
    invalid(
        run(
            &mut d,
            json!({"type": "add_material", "name": " ", "color": [0, 0, 0]}),
        ),
        "needs a name",
    );
    invalid(
        run(
            &mut d,
            json!({"type": "add_material", "name": "x".repeat(257), "color": [0, 0, 0]}),
        ),
        "at most 256",
    );
    let (kind, message, _) = run(
        &mut d,
        json!({"type": "change_material", "material": "m999", "name": "X"}),
    )
    .unwrap_err();
    assert_eq!(
        (kind, message.as_str()),
        (ErrorKind::NotFound, "no material m999")
    );
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "delete_material", "material": "m999"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    // Colors are whole numbers 0 to 255, and material ids are "m<n>".
    for bad in [
        json!({"type": "add_material", "color": [256, 0, 0]}),
        json!({"type": "add_material", "color": [0.5, 0, 0]}),
        json!({"type": "add_material", "color": [0, 0]}),
        json!({"type": "delete_material", "material": "d3"}),
    ] {
        assert!(
            serde_json::from_value::<Command>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    assert_eq!(materials(&d).len(), 1);
    assert_eq!(materials(&d)[0]["id"], red);
}

#[test]
fn painting_faces_sets_their_front_and_back_materials() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    let blue = add(&mut d, "Blue", [0, 0, 200]);
    let revision = d.revision();
    let out = run(
        &mut d,
        json!({"type": "paint", "ids": ["f25"], "material": red}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"], json!(["f25"]));
    assert_eq!(d.revision(), revision + 1);
    let top = ask(&d, json!({"type": "entity", "id": "f25"}));
    assert_eq!(top["front_material"], red);
    assert!(top.get("back_material").is_none(), "the default is absent");

    run(
        &mut d,
        json!({"type": "paint", "ids": ["f25", "f8"], "material": blue, "side": "back"}),
    )
    .unwrap();
    let top = ask(&d, json!({"type": "entity", "id": "f25"}));
    assert_eq!(
        (&top["front_material"], &top["back_material"]),
        (&json!(red), &json!(blue))
    );
    run(
        &mut d,
        json!({"type": "paint", "ids": ["f25"], "side": "both"}),
    )
    .unwrap();
    let top = ask(&d, json!({"type": "entity", "id": "f25"}));
    assert!(top.get("front_material").is_none() && top.get("back_material").is_none());
    assert_eq!(materials(&d)[1]["faces"], 1, "f8's back is still blue");

    run(&mut d, json!({"type": "undo"})).unwrap();
    let top = ask(&d, json!({"type": "entity", "id": "f25"}));
    assert_eq!(top["front_material"], red);

    // Painting what is already painted so changes nothing and adds no undo step.
    let revision = d.revision();
    let out = run(
        &mut d,
        json!({"type": "paint", "ids": ["f25"], "material": red}),
    )
    .unwrap();
    assert!(out["changes"]["modified"].as_array().unwrap().is_empty());
    assert_eq!(d.revision(), revision);
}

#[test]
fn bad_paint_commands_are_refused() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    let cases = [
        (
            json!({"type": "paint", "ids": [], "material": red}),
            ErrorKind::InvalidInput,
            vec![],
        ),
        (
            json!({"type": "paint", "ids": ["f8", "e2"], "material": red}),
            ErrorKind::InvalidInput,
            vec!["e2"],
        ),
        (
            json!({"type": "paint", "ids": ["v0"], "material": red}),
            ErrorKind::InvalidInput,
            vec!["v0"],
        ),
        (
            json!({"type": "paint", "ids": ["f999"], "material": red}),
            ErrorKind::NotFound,
            vec!["f999"],
        ),
        (
            json!({"type": "paint", "ids": ["f8"], "material": "m999"}),
            ErrorKind::NotFound,
            vec![],
        ),
    ];
    let revision = d.revision();
    for (command, kind, ids) in cases {
        let (got, message, got_ids) = run(&mut d, command.clone()).unwrap_err();
        assert_eq!(got, kind, "{command}: {message}");
        assert_eq!(got_ids, ids, "{command}");
        if command["ids"] == json!(["v0"]) || command["ids"] == json!(["f8", "e2"]) {
            assert!(message.contains("only faces and instances"), "{message}");
        }
    }
    assert_eq!(d.revision(), revision);
    assert!(
        serde_json::from_value::<Command>(
            json!({"type": "paint", "ids": ["f8"], "material": red, "side": "inside"})
        )
        .is_err()
    );
}

/// The id of the open context's face pointing up at z = 1.
fn top_face(d: &Document) -> Value {
    let faces = ask(d, json!({"type": "entities", "kinds": ["face"]}));
    faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap()["id"]
        .clone()
}

#[test]
fn instances_are_painted_inside_and_out_and_deleting_unpaints() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    let all = ask(&d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    let ids: Vec<Value> = all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect();
    run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    run(
        &mut d,
        json!({"type": "paint", "ids": [group], "material": red, "side": "back"}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": group}))["material"],
        red
    );

    // Inside the group, its faces (with new ids) are painted in the group's own context.
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let top = top_face(&d);
    run(
        &mut d,
        json!({"type": "paint", "ids": [top], "material": red}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let listed = materials(&d);
    assert_eq!(
        (&listed[0]["faces"], &listed[0]["instances"]),
        (&json!(1), &json!(1))
    );

    run(&mut d, json!({"type": "delete_material", "material": red})).unwrap();
    assert!(
        ask(&d, json!({"type": "entity", "id": group}))
            .get("material")
            .is_none()
    );
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    assert!(
        ask(&d, json!({"type": "entity", "id": top}))
            .get("front_material")
            .is_none()
    );
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": group}))["material"],
        red
    );
    assert_eq!(materials(&d)[0]["faces"], 1);
}

#[test]
fn a_failed_atomic_batch_unpaints_and_events_list_materials() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    let commands: Vec<Command> = serde_json::from_value(json!([
        {"type": "paint", "ids": ["f25"], "material": red},
        {"type": "paint", "ids": ["f8"], "material": "m999"}
    ]))
    .unwrap();
    let err = d.execute_batch(commands, true).unwrap_err();
    assert!(err.rolled_back);
    assert!(
        ask(&d, json!({"type": "entity", "id": "f25"}))
            .get("front_material")
            .is_none()
    );
    let events = ask(&d, json!({"type": "events", "since": 0}));
    let last = events["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["changes"]["materials"], json!([red]));
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("session-materials");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn materials_and_paint_are_saved_and_older_files_still_open() {
    let mut d = doc();
    cube(&mut d);
    let red = add(&mut d, "Red", [200, 0, 0]);
    run(
        &mut d,
        json!({"type": "paint", "ids": ["f25"], "material": red, "side": "both"}),
    )
    .unwrap();
    let path = scratch("painted.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(materials(&e), materials(&d));
    assert_eq!(
        ask(&e, json!({"type": "entity", "id": "f25"})),
        ask(&d, json!({"type": "entity", "id": "f25"}))
    );
    // A new material after reopening gets an id never used before.
    let next = add(&mut e, "Blue", [0, 0, 200]);
    assert_ne!(next, red);

    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let document: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    assert_eq!(document["schema_version"], 5);
    assert_eq!(document["model"]["materials"][0]["name"], "Red");
    assert!(
        document["model"]["materials"][0].get("opacity").is_none(),
        "an opaque material saves as before opacity existed"
    );

    // The same model written by version 4, before materials: it opens unpainted.
    let mut old = document.clone();
    old["schema_version"] = json!(4);
    old["model"].as_object_mut().unwrap().remove("materials");
    for f in old["model"]["root"]["faces"].as_array_mut().unwrap() {
        let f = f.as_object_mut().unwrap();
        f.remove("front");
        f.remove("back");
    }
    let old_path = scratch("v4.skr");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&old_path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, &serde_json::to_vec(&old).unwrap()).unwrap();
    zip.finish().unwrap();
    let mut v4 = doc();
    run(
        &mut v4,
        json!({"type": "open", "path": old_path.to_str().unwrap()}),
    )
    .unwrap();
    assert!(materials(&v4).is_empty());
    assert_eq!(ask(&v4, json!({"type": "stats"}))["faces"], 6);

    // A face painted with a material the file lacks does not open.
    let mut broken = document;
    broken["model"]["materials"] = json!([]);
    let broken_path = scratch("broken.skr");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&broken_path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, &serde_json::to_vec(&broken).unwrap()).unwrap();
    zip.finish().unwrap();
    let (kind, message, _) = run(
        &mut doc(),
        json!({"type": "open", "path": broken_path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("missing material"), "{message}");
}

#[test]
fn opacity_is_added_changed_undone_and_listed() {
    let mut d = doc();
    let glass = run(
        &mut d,
        json!({"type": "add_material", "name": "Glass", "color": [180, 220, 240], "opacity": 0.3}),
    )
    .unwrap()["changes"]["materials"][0]
        .as_str()
        .unwrap()
        .to_string();
    let opacity = |d: &Document| materials(d)[0]["opacity"].as_f64().unwrap() as f32;
    assert_eq!(opacity(&d), 0.3);
    let out = run(
        &mut d,
        json!({"type": "change_material", "material": glass, "opacity": 0.6}),
    )
    .unwrap();
    assert_eq!(out["changes"]["materials"], json!([glass]));
    assert_eq!(opacity(&d), 0.6);
    assert_eq!(
        materials(&d)[0]["color"],
        json!([180, 220, 240]),
        "color stays"
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(opacity(&d), 0.3);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(opacity(&d), 0.6);
    // Without opacity a material is opaque.
    add(&mut d, "Brick", [180, 60, 40]);
    assert_eq!(materials(&d)[1]["opacity"], 1.0);
}

#[test]
fn bad_opacities_are_refused() {
    let mut d = doc();
    let red = add(&mut d, "Red", [200, 0, 0]);
    for bad in [json!(-0.5), json!(1.5), json!(1e300)] {
        for cmd in [
            json!({"type": "add_material", "name": "Glass", "color": [0, 0, 0], "opacity": bad}),
            json!({"type": "change_material", "material": red, "opacity": bad}),
        ] {
            let (kind, message, _) = run(&mut d, cmd.clone()).unwrap_err();
            assert_eq!(kind, ErrorKind::InvalidInput, "{cmd}: {message}");
            assert!(message.contains("opacity"), "{message}");
        }
    }
    assert!(
        serde_json::from_value::<Command>(
            json!({"type": "add_material", "color": [0, 0, 0], "opacity": "half"})
        )
        .is_err()
    );
    assert_eq!(materials(&d).len(), 1);
    assert_eq!(materials(&d)[0]["opacity"], 1.0);
}

#[test]
fn opacity_is_saved_as_version_6_and_opaque_models_stay_version_5() {
    let mut d = doc();
    cube(&mut d);
    let glass = run(
        &mut d,
        json!({"type": "add_material", "name": "Glass", "color": [180, 220, 240], "opacity": 0.4}),
    )
    .unwrap()["changes"]["materials"][0]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut d,
        json!({"type": "paint", "ids": ["f25"], "material": glass}),
    )
    .unwrap();
    let path = scratch("glass.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let document: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    assert_eq!(
        document["schema_version"],
        session::file::TRANSLUCENT_SCHEMA_VERSION
    );
    assert_eq!(document["schema_version"], 6);
    assert_eq!(document["model"]["materials"][0]["opacity"], 0.4);
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(materials(&e), materials(&d));

    // Made opaque again, the model saves as version 5 with no opacity.
    run(
        &mut d,
        json!({"type": "change_material", "material": glass, "opacity": 1}),
    )
    .unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let document: Value = serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    assert_eq!(document["schema_version"], 5);
    assert!(document["model"]["materials"][0].get("opacity").is_none());
}
