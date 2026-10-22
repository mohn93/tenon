//! Library items: saving a component, material or model to a `.skr` file, listing a folder
//! of them and inserting one.

use std::path::{Path, PathBuf};

use glam::DVec2;
use kernel::EntityId;
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn ok(d: &mut Document, v: Value) -> Value {
    run(d, v).unwrap_or_else(|e| panic!("command failed: {e:?}"))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

/// A fresh, empty folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-library-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn created(out: &Value) -> Vec<String> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

fn instance_of(out: &Value) -> String {
    created(out)
        .into_iter()
        .find(|id| id.starts_with('i'))
        .expect("an instance was created")
}

fn everything(d: &Document) -> Vec<EntityId> {
    let m = d.model();
    m.edges()
        .map(|(id, _)| EntityId::Edge(id))
        .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
        .collect()
}

fn rectangle(d: &mut Document, corner: [f64; 3], opposite: [f64; 3]) -> Vec<EntityId> {
    let before = everything(d);
    ok(
        d,
        json!({"type": "add_rectangle", "corner": corner, "opposite": opposite}),
    );
    everything(d)
        .into_iter()
        .filter(|i| !before.contains(i))
        .collect()
}

/// A component "Chair" (a red square on tag "Walls"), turned 30 degrees, stretched along x
/// and moved, in a group at the root; the group is the open context. Returns the instance.
fn turned_component(d: &mut Document) -> String {
    let ids = rectangle(d, [0.0; 3], [1.0, 1.0, 0.0]);
    ok(
        d,
        json!({"type": "add_material", "name": "Red", "color": [200, 0, 0]}),
    );
    ok(d, json!({"type": "add_tag", "name": "Walls"}));
    let red = d.scene().materials().next().unwrap().0.to_string();
    let walls = d.scene().tags().next().unwrap().0.to_string();
    let face: Vec<EntityId> = ids
        .iter()
        .copied()
        .filter(|i| matches!(i, EntityId::Face(_)))
        .collect();
    ok(d, json!({"type": "paint", "ids": face, "material": red}));
    ok(d, json!({"type": "set_tag", "ids": ids, "tag": walls}));
    let chair = instance_of(&ok(
        d,
        json!({"type": "make_component", "ids": ids, "name": "Chair"}),
    ));
    ok(
        d,
        json!({"type": "rotate", "ids": [chair], "center": [0, 0, 0], "angle": 30}),
    );
    ok(
        d,
        json!({"type": "scale", "ids": [chair], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    );
    ok(
        d,
        json!({"type": "move", "ids": [chair], "offset": [3, -2, 1]}),
    );
    let group = instance_of(&ok(d, json!({"type": "make_group", "ids": [chair]})));
    ok(d, json!({"type": "open_instance", "id": group}));
    chair
}

fn instance_info(d: &Document, instance: &str) -> Value {
    let all = ask(d, json!({"type": "entities", "ids": [instance]}));
    all["instances"][0].clone()
}

fn instances(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities", "kinds": ["instance"]}))["instances"]
        .as_array()
        .unwrap()
        .clone()
}

fn extent(info: &Value) -> [f64; 3] {
    let min: Vec<f64> = serde_json::from_value(info["bounds"]["min"].clone()).unwrap();
    let max: Vec<f64> = serde_json::from_value(info["bounds"]["max"].clone()).unwrap();
    [max[0] - min[0], max[1] - min[1], max[2] - min[2]]
}

fn list(dir: &Path) -> Value {
    ask(
        &doc(),
        json!({"type": "library", "folder": dir.to_str().unwrap()}),
    )
}

fn materials(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "materials"}))["materials"]
        .as_array()
        .unwrap()
        .clone()
}

/// A 4 x 2 pixel PNG, its left half red and its right half blue.
fn red_blue_png(dir: &Path) -> PathBuf {
    let path = dir.join("tiles.png");
    image::RgbaImage::from_fn(4, 2, |x, _| {
        if x < 2 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    })
    .save(&path)
    .unwrap();
    path
}

#[test]
fn a_component_from_a_turned_instance_in_a_group_reinserts_with_its_shape_materials_and_tags() {
    let dir = folder("component");
    let mut d = doc();
    let chair = turned_component(&mut d);
    let original = instance_info(&d, &chair);
    let revision = d.revision();
    let file = dir.join("chair.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&file), "kind": "component", "item": chair}),
    );
    // Saving is no model change and no undo step.
    assert_eq!(d.revision(), revision);
    assert_eq!(d.undo_label(), Some("make_group"));

    let mut other = doc();
    let out = ok(
        &mut other,
        json!({"type": "insert_library_item", "path": p(&file)}),
    );
    assert_eq!(other.undo_label(), Some("insert"));
    let made = instance_info(&other, &instance_of(&out));
    // Rotation and scale are kept: the matrix's axes are the original's.
    let (a, b) = (
        original["transform"].as_array().unwrap(),
        made["transform"].as_array().unwrap(),
    );
    for k in 0..9 {
        let (x, y) = (a[k].as_f64().unwrap(), b[k].as_f64().unwrap());
        assert!((x - y).abs() < 1e-9, "axis entry {k}: {x} vs {y}");
    }
    for (x, y) in extent(&original).iter().zip(extent(&made)) {
        assert!((x - y).abs() < 1e-9, "{original} vs {made}");
    }
    // The bounding box's minimum corner is at the context's origin.
    for v in made["bounds"]["min"].as_array().unwrap() {
        assert!(v.as_f64().unwrap().abs() < 1e-9, "{made}");
    }
    assert_eq!(materials(&other).len(), 1);
    assert_eq!(materials(&other)[0]["name"], "Red");
    assert_eq!(other.scene().tags().count(), 1);
    assert_eq!(other.selection().len(), 1);
    run(&mut other, json!({"type": "undo"})).unwrap();
    assert!(instances(&other).is_empty());
}

#[test]
fn inserting_at_a_point_puts_the_minimum_corner_there() {
    let dir = folder("at");
    let mut d = doc();
    let chair = turned_component(&mut d);
    let file = dir.join("chair.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&file), "kind": "component", "item": chair}),
    );
    let mut other = doc();
    let out = ok(
        &mut other,
        json!({"type": "insert_library_item", "path": p(&file), "at": [4.0, 5.0, 6.0]}),
    );
    let made = instance_info(&other, &instance_of(&out));
    let min: Vec<f64> = serde_json::from_value(made["bounds"]["min"].clone()).unwrap();
    for (x, want) in min.iter().zip([4.0, 5.0, 6.0]) {
        assert!((x - want).abs() < 1e-9, "{made}");
    }
}

#[test]
fn inserting_a_component_twice_makes_two_definitions_and_brings_materials_and_tags() {
    let dir = folder("twice");
    let mut d = doc();
    let chair = turned_component(&mut d);
    let file = dir.join("chair.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&file), "kind": "component", "item": chair}),
    );
    // Into the model it came from too: a fresh definition each time.
    let before = d.scene().definitions().count();
    for k in 1..=2 {
        ok(
            &mut d,
            json!({"type": "insert_library_item", "path": p(&file)}),
        );
        assert_eq!(d.scene().definitions().count(), before + k);
    }
    let mut other = doc();
    for k in 1..=2 {
        ok(
            &mut other,
            json!({"type": "insert_library_item", "path": p(&file), "at": [k, 0, 0]}),
        );
        assert_eq!(other.scene().definitions().count(), k);
    }
    // Materials and tags merge by name.
    assert_eq!(materials(&other).len(), 1);
    assert_eq!(other.scene().tags().count(), 1);
}

#[test]
fn saving_refuses_groups_missing_items_wrong_kinds_and_bad_paths() {
    let dir = folder("refuse");
    let mut d = doc();
    let ids = rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    let group = instance_of(&ok(&mut d, json!({"type": "make_group", "ids": ids})));
    let file = dir.join("g.skr");
    let save = |d: &mut Document, extra: Value| {
        let mut c = json!({"type": "save_to_library", "path": p(&file)});
        for (k, v) in extra.as_object().unwrap() {
            c[k] = v.clone();
        }
        run(d, c)
    };
    let (kind, message) = save(&mut d, json!({"kind": "component", "item": group})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("component"), "{message}");
    assert!(!file.exists());
    for extra in [
        json!({"kind": "component"}),
        json!({"kind": "component", "item": "i999"}),
        json!({"kind": "material"}),
        json!({"kind": "material", "item": "m999"}),
        json!({"kind": "model", "item": group}),
        json!({"kind": "component", "item": "m1"}),
    ] {
        assert!(save(&mut d, extra.clone()).is_err(), "{extra}");
    }
    // Only .skr files, never a name with control characters.
    let txt = dir.join("g.txt");
    let c = json!({"type": "save_to_library", "path": p(&txt), "kind": "model"});
    assert_eq!(run(&mut d, c).unwrap_err().0, ErrorKind::InvalidInput);
    let c = json!({"type": "save_to_library", "path": p(&file), "kind": "model", "name": "a\nb"});
    assert_eq!(run(&mut d, c).unwrap_err().0, ErrorKind::InvalidInput);
    assert!(!file.exists() && !txt.exists());
}

#[test]
fn saving_over_an_existing_item_needs_overwrite_and_never_replaces_the_open_model() {
    let dir = folder("overwrite");
    let mut d = doc();
    rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    let file = dir.join("m.skr");
    let save = |d: &mut Document, overwrite: bool| {
        run(
            d,
            json!({"type": "save_to_library", "path": p(&file), "kind": "model",
                   "overwrite": overwrite}),
        )
    };
    save(&mut d, false).unwrap();
    let first = std::fs::read(&file).unwrap();
    assert_eq!(save(&mut d, false).unwrap_err().0, ErrorKind::InvalidInput);
    assert_eq!(std::fs::read(&file).unwrap(), first);
    save(&mut d, true).unwrap();
    // No temporary files are left beside it.
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    assert_eq!(names.len(), 1);

    // The model's own file, by its path or another spelling of it, is never written.
    let own = dir.join("own.skr");
    ok(&mut d, json!({"type": "save", "path": p(&own)}));
    let before = std::fs::read(&own).unwrap();
    for spelling in [
        own.clone(),
        dir.join("sub/../own.skr"),
        dir.join("./own.skr"),
    ] {
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let c = json!({"type": "save_to_library", "path": p(&spelling), "kind": "model",
                       "overwrite": true});
        let (kind, message) = run(&mut d, c).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{spelling:?}");
        assert!(message.contains("open model"), "{message}");
    }
    assert_eq!(std::fs::read(&own).unwrap(), before);
}

#[test]
fn a_material_item_keeps_its_texture_and_gets_a_free_name_when_another_has_it() {
    let dir = folder("material");
    let png = red_blue_png(&dir);
    let mut d = doc();
    let out = ok(
        &mut d,
        json!({"type": "add_material", "name": "Tiles",
               "texture": {"path": p(&png), "width": 0.5}}),
    );
    let tiles = out["changes"]["materials"][0].as_str().unwrap().to_string();
    let file = dir.join("tiles.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&file), "kind": "material", "item": tiles}),
    );

    // A model with a different material of the same name.
    let mut other = doc();
    ok(
        &mut other,
        json!({"type": "add_material", "name": "Tiles", "color": [1, 2, 3]}),
    );
    let out = ok(
        &mut other,
        json!({"type": "insert_library_item", "path": p(&file)}),
    );
    let mats = materials(&other);
    assert_eq!(mats.len(), 2);
    let added = mats
        .iter()
        .find(|m| m["name"] == "Tiles 2")
        .expect("Tiles 2");
    assert!(added["texture"].is_object(), "{added}");
    assert_eq!(out["material"], added["id"]);
    assert_eq!(other.undo_label(), Some("insert"));
    // Again: the same item is reused, not added a third time.
    let again = ok(
        &mut other,
        json!({"type": "insert_library_item", "path": p(&file)}),
    );
    assert_eq!(materials(&other).len(), 2);
    assert_eq!(again["material"], added["id"]);
    // Into the model it came from: identical, so reused as it is.
    let same = ok(
        &mut d,
        json!({"type": "insert_library_item", "path": p(&file)}),
    );
    assert_eq!(materials(&d).len(), 1);
    assert_eq!(same["material"].as_str().unwrap(), tiles);
    // Into an empty model: added under its own name.
    let mut empty = doc();
    ok(
        &mut empty,
        json!({"type": "insert_library_item", "path": p(&file)}),
    );
    assert_eq!(materials(&empty)[0]["name"], "Tiles");
    assert!(materials(&empty)[0]["texture"].is_object());
}

#[test]
fn a_model_item_inserts_the_whole_model_with_new_definitions() {
    let dir = folder("model");
    let mut d = doc();
    let chair = turned_component(&mut d);
    let _ = chair;
    ok(&mut d, json!({"type": "close_instance"}));
    let file = dir.join("house.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&file), "kind": "model", "name": "House"}),
    );
    let mut other = doc();
    let out = ok(
        &mut other,
        json!({"type": "insert_library_item", "path": p(&file), "at": [10, 0, 0]}),
    );
    assert_eq!(instances(&other).len(), 1);
    assert_eq!(other.scene().definitions().count(), 2);
    assert_eq!(other.selection().len(), 1);
    assert!(created(&out).iter().any(|i| i.starts_with('i')));
    // A model item opens as a model too.
    let mut third = doc();
    ok(&mut third, json!({"type": "open", "path": p(&file)}));
    assert_eq!(third.scene().definitions().count(), 2);
    // Listed under the name it was saved with.
    let listed = list(&dir);
    assert_eq!(listed["items"][0]["name"], "House");
    assert_eq!(listed["items"][0]["kind"], "model");
}

#[test]
fn listing_names_kinds_and_sorts_by_name_ignoring_case() {
    let dir = folder("list");
    let mut d = doc();
    let ids = rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    let part = instance_of(&ok(
        &mut d,
        json!({"type": "make_component", "ids": ids, "name": "apple"}),
    ));
    ok(
        &mut d,
        json!({"type": "add_material", "name": "Zinc", "color": [9, 9, 9]}),
    );
    let zinc = d.scene().materials().next().unwrap().0.to_string();
    for (file, extra) in [
        ("b.skr", json!({"kind": "model", "name": "Banana"})),
        ("a.skr", json!({"kind": "component", "item": part})),
        ("c.skr", json!({"kind": "material", "item": zinc})),
    ] {
        let mut c = json!({"type": "save_to_library", "path": p(&dir.join(file))});
        for (k, v) in extra.as_object().unwrap() {
            c[k] = v.clone();
        }
        ok(&mut d, c);
    }
    // A plain model file with no library entry is a model named by its stem.
    ok(
        &mut d,
        json!({"type": "save", "path": p(&dir.join("Plain.SKR"))}),
    );
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    std::fs::create_dir(dir.join("nested.skr")).unwrap();
    let listed = list(&dir);
    let rows: Vec<(String, String)> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["name"].as_str().unwrap().to_string(),
                i["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("apple".into(), "component".into()),
            ("Banana".into(), "model".into()),
            ("Plain".into(), "model".into()),
            ("Zinc".into(), "material".into()),
        ]
    );
    assert!(listed["items"][0]["modified"].as_u64().unwrap() > 0);
    assert!(listed.get("more").is_none_or(|m| m == false));
    assert!(listed["items"][0].get("error").is_none());
}

#[test]
fn listing_skips_links_and_huge_files_and_marks_damaged_ones() {
    let dir = folder("hazards");
    let mut d = doc();
    rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&dir.join("good.skr")), "kind": "model"}),
    );
    std::fs::write(dir.join("damaged.skr"), b"not a zip at all").unwrap();
    std::fs::write(dir.join("empty.skr"), b"").unwrap();
    let huge = std::fs::File::create(dir.join("huge.skr")).unwrap();
    huge.set_len(300 * 1024 * 1024).unwrap();
    drop(huge);
    #[cfg(unix)]
    {
        let elsewhere = folder("hazards-elsewhere");
        std::fs::copy(dir.join("good.skr"), elsewhere.join("far.skr")).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("far.skr"), dir.join("link.skr")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("linkdir.skr")).unwrap();
    }
    let listed = list(&dir);
    let rows: Vec<(String, bool)> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["name"].as_str().unwrap().to_string(),
                i.get("error").is_some(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("damaged".to_string(), true),
            ("empty".to_string(), true),
            ("good".to_string(), false),
        ]
    );
}

#[test]
fn listing_two_thousand_files_is_bounded_and_fast() {
    let dir = folder("many");
    let mut d = doc();
    rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    let seed = dir.join("seed.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&seed), "kind": "model"}),
    );
    for k in 0..2000 {
        std::fs::copy(&seed, dir.join(format!("item-{k:04}.skr"))).unwrap();
    }
    let started = std::time::Instant::now();
    let listed = list(&dir);
    assert!(started.elapsed().as_secs() < 20);
    assert_eq!(listed["items"].as_array().unwrap().len(), 1000);
    assert_eq!(listed["more"], true);
    let names: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn listing_a_file_or_a_missing_folder_is_an_error() {
    let dir = folder("notdir");
    std::fs::write(dir.join("a.skr"), b"x").unwrap();
    let d = doc();
    for path in [dir.join("a.skr"), dir.join("missing")] {
        let q: Query =
            serde_json::from_value(json!({"type": "library", "folder": p(&path)})).unwrap();
        assert!(d.query(&q).is_err(), "{path:?}");
    }
}

#[test]
fn inserting_refuses_damaged_files_other_files_and_oversized_ones() {
    let dir = folder("insert-refuse");
    let mut d = doc();
    std::fs::write(dir.join("damaged.skr"), b"nope").unwrap();
    std::fs::create_dir(dir.join("folder.skr")).unwrap();
    for name in ["damaged.skr", "folder.skr", "missing.skr"] {
        let r = run(
            &mut d,
            json!({"type": "insert_library_item", "path": p(&dir.join(name))}),
        );
        assert!(r.is_err(), "{name}");
    }
    assert_eq!(d.revision(), 0);
    let huge = std::fs::File::create(dir.join("huge.skr")).unwrap();
    huge.set_len(300 * 1024 * 1024).unwrap();
    drop(huge);
    let r = run(
        &mut d,
        json!({"type": "insert_library_item", "path": p(&dir.join("huge.skr"))}),
    );
    assert_eq!(r.unwrap_err().0, ErrorKind::InvalidInput);
}

#[test]
fn save_to_library_takes_batch_references_to_an_earlier_commands_ids() {
    let dir = folder("batch");
    let mut d = doc();
    let file = dir.join("red.skr");
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_material", "name": "Red", "color": [200, 0, 0]}),
        json!({"type": "save_to_library", "path": p(&file), "kind": "material",
               "item": "$0:m0"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, false).unwrap();
    assert_eq!(list(&dir)["items"][0]["kind"], "material");
    // Not allowed in an atomic batch, as save is not.
    let commands = vec![
        BatchCommand::parse(
            json!({"type": "save_to_library", "path": p(&file), "kind": "model",
                   "overwrite": true}),
            0,
        )
        .unwrap(),
    ];
    assert!(d.execute_batch(commands, true).is_err());
}

/// The world positions of the geometry of the instance `info` (an `entities` row of the open
/// context), through the context's transform.
fn world_geometry(d: &Document, info: &Value) -> Vec<glam::DVec3> {
    let t: Vec<f64> = serde_json::from_value(info["transform"].clone()).unwrap();
    let local = glam::DAffine3::from_mat3_translation(
        glam::DMat3::from_cols_slice(&t[..9]),
        glam::DVec3::from_slice(&t[9..]),
    );
    let to_world = d.context_transform() * local;
    let definition = d
        .model()
        .instances()
        .find(|(i, _)| EntityId::Instance(*i).to_string() == info["id"].as_str().unwrap());
    let definition = definition.unwrap().1.definition;
    d.scene()
        .definition(definition)
        .unwrap()
        .mesh
        .vertices()
        .map(|(_, v)| to_world.transform_point3(v.pos))
        .collect()
}

fn box_of(points: &[glam::DVec3]) -> (glam::DVec3, glam::DVec3) {
    let lo = points.iter().copied().reduce(glam::DVec3::min).unwrap();
    let hi = points.iter().copied().reduce(glam::DVec3::max).unwrap();
    (lo, hi)
}

#[test]
fn inserting_inside_a_turned_scaled_group_keeps_world_orientation_and_size() {
    let dir = folder("turned-context");
    let mut d = doc();
    let ids = rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    let group = instance_of(&ok(&mut d, json!({"type": "make_group", "ids": ids})));
    ok(
        &mut d,
        json!({"type": "rotate", "ids": [group], "center": [0, 0, 0], "angle": 30}),
    );
    ok(
        &mut d,
        json!({"type": "scale", "ids": [group], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    );
    ok(
        &mut d,
        json!({"type": "move", "ids": [group], "offset": [3, -2, 1]}),
    );
    ok(&mut d, json!({"type": "open_instance", "id": group}));
    let inner = rectangle(&mut d, [5.0, 5.0, 0.0], [6.0, 6.0, 0.0]);
    let part = instance_of(&ok(
        &mut d,
        json!({"type": "make_component", "ids": inner, "name": "Tile"}),
    ));
    let file = dir.join("tile.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&file), "kind": "component", "item": part}),
    );
    let (lo, hi) = box_of(&world_geometry(&d, &instance_info(&d, &part)));
    let want = hi - lo;
    let context = d.context_transform();
    for at in [None, Some([0.5, 0.25, 0.0])] {
        let mut c = json!({"type": "insert_library_item", "path": p(&file)});
        let point = at.map_or(context.translation, |a| {
            context.transform_point3(glam::DVec3::from(a))
        });
        if let Some(a) = at {
            c["at"] = json!(a);
        }
        let out = ok(&mut d, c);
        let made = instance_info(&d, &instance_of(&out));
        let (mlo, mhi) = box_of(&world_geometry(&d, &made));
        for k in 0..3 {
            assert!(((mhi - mlo)[k] - want[k]).abs() < 1e-9, "{at:?} {made}");
            assert!((mlo[k] - point[k]).abs() < 1e-9, "{at:?}: {mlo} vs {point}");
        }
    }
}

#[test]
fn a_big_library_entry_is_ignored_and_a_file_with_too_many_entries_is_marked() {
    use std::io::{Read, Write};
    let dir = folder("entries");
    let mut d = doc();
    rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    let good = dir.join("good.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&good), "kind": "model", "name": "Named"}),
    );
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&good).unwrap()).unwrap();
    let mut entries = Vec::new();
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).unwrap();
        let mut b = Vec::new();
        e.read_to_end(&mut b).unwrap();
        entries.push((e.name().unwrap().to_string(), b));
    }
    let write = |path: &Path, entries: &[(String, Vec<u8>)]| {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, b) in entries {
            z.start_file(n.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    };
    let big = dir.join("big.skr");
    let mut padded = entries.clone();
    for (n, b) in &mut padded {
        if n == "library.json" {
            b.extend(vec![b' '; 20 * 1024]);
        }
    }
    write(&big, &padded);
    let many = dir.join("many.skr");
    let mut crowded: Vec<_> = entries
        .iter()
        .filter(|(n, _)| n != "library.json")
        .cloned()
        .collect();
    for k in 0..10_001 {
        crowded.push((format!("extra-{k}"), Vec::new()));
    }
    write(&many, &crowded);
    let listed = list(&dir);
    let rows: Vec<(String, bool)> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["name"].as_str().unwrap().to_string(),
                i.get("error").is_some(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("big".to_string(), false),
            ("many".to_string(), true),
            ("Named".to_string(), false)
        ]
    );
    assert_eq!(listed["items"][0]["kind"], "model");
    // The oversized entry does not stop it being inserted as a model.
    let mut other = doc();
    ok(
        &mut other,
        json!({"type": "insert_library_item", "path": p(&big)}),
    );
}

#[test]
fn the_skr_extension_is_matched_ignoring_case_when_saving_and_listing() {
    let dir = folder("case");
    let mut d = doc();
    rectangle(&mut d, [0.0; 3], [1.0, 1.0, 0.0]);
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&dir.join("Big.SKR")), "kind": "model"}),
    );
    assert_eq!(list(&dir)["items"][0]["name"], "Big");
}

#[test]
fn an_items_outline_is_its_edges_and_its_corner_a_material_has_none() {
    let dir = folder("outline");
    let mut d = doc();
    rectangle(&mut d, [1.0, 1.0, 0.0], [3.0, 4.0, 0.0]);
    let model = dir.join("plate.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&model), "kind": "model"}),
    );
    let outline = session::library::outline(&model).unwrap();
    assert_eq!(outline.base, glam::DVec3::new(1.0, 1.0, 0.0));
    assert_eq!(outline.segments.len(), 4);
    let red = ok(
        &mut d,
        json!({"type": "add_material", "name": "Red", "color": [200, 30, 30]}),
    );
    let material = dir.join("red.skr");
    ok(
        &mut d,
        json!({"type": "save_to_library", "path": p(&material), "kind": "material",
               "item": red["changes"]["materials"][0]}),
    );
    let err = session::library::outline(&material).unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    let err = session::library::outline(&dir.join("none.skr")).unwrap_err();
    assert_eq!(err.kind, ErrorKind::File);
    std::fs::remove_dir_all(dir).unwrap();
}
