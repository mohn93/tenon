//! Styles: the model's list of named looks, choosing one for the view, and keeping them in
//! files and scenes.

use std::path::PathBuf;

use glam::DVec2;
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

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-styles-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

fn add_style(d: &mut Document, params: Value) -> String {
    let mut command = json!({"type": "add_style"});
    command
        .as_object_mut()
        .unwrap()
        .extend(params.as_object().unwrap().clone());
    let out = run(d, command).unwrap();
    let ids = out["changes"]["styles"].as_array().unwrap();
    assert_eq!(ids.len(), 1, "{out}");
    ids[0].as_str().unwrap().to_string()
}

fn styles(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "styles"}))["styles"]
        .as_array()
        .unwrap()
        .clone()
}

fn chosen(d: &Document) -> Value {
    ask(d, json!({"type": "camera"}))
        .get("style")
        .cloned()
        .unwrap_or(Value::Null)
}

fn undo_labels(d: &Document) -> Value {
    ask(d, json!({"type": "history"}))["undo"].clone()
}

#[test]
fn a_model_starts_with_no_styles_and_the_default_look() {
    let d = doc();
    assert!(styles(&d).is_empty());
    assert_eq!(chosen(&d), Value::Null);
    assert!(d.chosen_style().is_none());
    assert!(ask(&d, json!({"type": "camera"})).get("style").is_none());
}

#[test]
fn add_style_makes_a_style_with_default_settings_and_one_undo_step() {
    let mut d = doc();
    let before = d.revision();
    let a = add_style(&mut d, json!({}));
    assert!(a.starts_with('y'), "{a}");
    assert!(d.revision() > before && d.is_modified());
    assert_eq!(undo_labels(&d)[0], "add_style");
    let list = styles(&d);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], a);
    assert_eq!(list[0]["name"], "Style 1");
    let s = &list[0]["settings"];
    assert_eq!(s["edges"]["color_mode"], "fixed");
    assert_eq!(s["edges"]["color"], json!([20, 20, 20]));
    assert_eq!(s["edges"]["show_edges"], true);
    assert_eq!(s["edges"]["profiles"], Value::Null);
    assert_eq!(s["background"], json!([245, 245, 245]));
    assert_eq!(s["sky"], Value::Null);
    let b = add_style(&mut d, json!({"name": "  Blueprint "}));
    assert_eq!(styles(&d)[1]["name"], "Blueprint");
    assert_ne!(a, b);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(styles(&d).len(), 1);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(styles(&d)[1]["id"], b);
}

#[test]
fn add_style_from_another_copies_its_settings() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "settings": {
            "edges": {"profiles": 4.0, "color_mode": "by_axis"},
            "fog": {"start": 1.0, "end": 20.0, "color": "background"}}}),
    )
    .unwrap();
    let b = add_style(&mut d, json!({"name": "B", "from": a}));
    let list = styles(&d);
    assert_eq!(list[1]["id"], b);
    assert_eq!(list[1]["settings"], list[0]["settings"]);
    assert_eq!(list[1]["settings"]["edges"]["profiles"], 4.0);
    let e = run(&mut d, json!({"type": "add_style", "from": "y99999"})).unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
}

#[test]
fn names_must_be_valid_and_distinct() {
    let mut d = doc();
    add_style(&mut d, json!({"name": "Plain"}));
    for bad in ["", "  ", &"x".repeat(101)] {
        let e = run(&mut d, json!({"type": "add_style", "name": bad})).unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput, "{bad:?}");
    }
    let e = run(&mut d, json!({"type": "add_style", "name": "Plain"})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert_eq!(styles(&d).len(), 1);
}

#[test]
fn change_style_changes_only_the_given_parts_as_one_step() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "settings": {
            "background": [1, 2, 3], "sky": [9, 9, 9],
            "ground": {"color": [4, 4, 4], "opacity": 0.5, "from_below": true}}}),
    )
    .unwrap();
    let steps = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    // Only the edges change; the background, sky and ground stay.
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "settings": {"edges": {"show_edges": false}}}),
    )
    .unwrap();
    let s = styles(&d)[0]["settings"].clone();
    assert_eq!(s["edges"]["show_edges"], false);
    assert_eq!(s["background"], json!([1, 2, 3]));
    assert_eq!(s["sky"], json!([9, 9, 9]));
    assert_eq!(s["ground"]["opacity"], 0.5);
    assert_eq!(
        ask(&d, json!({"type": "history"}))["undo_depth"].as_u64(),
        Some(steps.as_u64().unwrap() + 1)
    );
    // A null turns an optional part off; absent leaves it.
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "settings": {"sky": null, "ground": null}}),
    )
    .unwrap();
    let s = styles(&d)[0]["settings"].clone();
    assert_eq!(s["sky"], Value::Null);
    assert_eq!(s["ground"], Value::Null);
    assert_eq!(s["background"], json!([1, 2, 3]));
    // Rename.
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "name": "Renamed"}),
    )
    .unwrap();
    assert_eq!(styles(&d)[0]["name"], "Renamed");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(styles(&d)[0]["name"], "A");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(styles(&d)[0]["settings"]["sky"], json!([9, 9, 9]));
}

#[test]
fn change_style_with_nothing_to_do_or_a_bad_value_is_refused_or_free() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    add_style(&mut d, json!({"name": "B"}));
    let e = run(&mut d, json!({"type": "change_style", "style": a})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    // The same values change nothing and make no undo step.
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "name": "A", "settings": {"background": [245, 245, 245]}}),
    )
    .unwrap();
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], depth);
    for (bad, why) in [
        (json!({"edges": {"profiles": 1.0}}), "profile width"),
        (json!({"edges": {"profiles": 11.0}}), "profile width"),
        (
            json!({"edges": {"sketchy": {"jitter": 11.0, "extension": 0.0}}}),
            "sketchy jitter",
        ),
        (
            json!({"edges": {"sketchy": {"jitter": 1.0, "extension": 31.0}}}),
            "sketchy extension",
        ),
        (json!({"edges": {"endpoints": 1.0}}), "endpoint length"),
        (json!({"edges": {"endpoints": 31.0}}), "endpoint length"),
        (
            json!({"ground": {"color": [0, 0, 0], "opacity": 1.5, "from_below": false}}),
            "ground opacity",
        ),
        (
            json!({"fog": {"start": 10.0, "end": 5.0, "color": "background"}}),
            "fog",
        ),
        (
            json!({"fog": {"start": -1.0, "end": 5.0, "color": "background"}}),
            "fog",
        ),
    ] {
        let cmd: Command =
            serde_json::from_value(json!({"type": "change_style", "style": a, "settings": bad}))
                .unwrap();
        let e = Document::execute(&mut d, cmd).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput, "{bad}");
        assert!(e.message.contains(why), "{bad}: {}", e.message);
    }
    // A colour channel past 255 is not a colour at all.
    assert!(
        serde_json::from_value::<Command>(
            json!({"type": "change_style", "style": a, "settings": {"background": [0, 0, 256]}})
        )
        .is_err()
    );
    let e = run(
        &mut d,
        json!({"type": "change_style", "style": a, "name": "B"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    let e = run(
        &mut d,
        json!({"type": "change_style", "style": "y99999", "name": "Z"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    assert_eq!(styles(&d)[0]["name"], "A");
}

#[test]
fn set_style_chooses_without_an_undo_step_or_a_change() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    let rev = d.revision();
    let path = scratch("clean.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    assert_eq!(chosen(&d), json!(a));
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], depth);
    assert_eq!(d.revision(), rev);
    assert!(!d.is_modified());
    assert!(d.chosen_style().is_some());
    assert_eq!(*d.chosen_style().unwrap(), kernel::StyleSettings::default());
    run(&mut d, json!({"type": "set_style", "style": null})).unwrap();
    assert_eq!(chosen(&d), Value::Null);
    assert!(d.chosen_style().is_none());
    // Absent is the same as null.
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    run(&mut d, json!({"type": "set_style"})).unwrap();
    assert_eq!(chosen(&d), Value::Null);
    let e = run(&mut d, json!({"type": "set_style", "style": "y99999"})).unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
}

#[test]
fn delete_style_falls_back_the_camera_and_scenes_and_undo_restores_them() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    let b = add_style(&mut d, json!({"name": "B"}));
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    let on_a =
        run(&mut d, json!({"type": "add_scene", "name": "On A"})).unwrap()["changes"]["scenes"][0]
            .clone();
    run(&mut d, json!({"type": "set_style", "style": b})).unwrap();
    let on_b =
        run(&mut d, json!({"type": "add_scene", "name": "On B"})).unwrap()["changes"]["scenes"][0]
            .clone();
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    let scene_style = |d: &Document, id: &Value| {
        ask(d, json!({"type": "scene", "scene": id}))["view"]["style"].clone()
    };
    assert_eq!(scene_style(&d, &on_a), json!(a));
    assert_eq!(scene_style(&d, &on_b), json!(b));
    let out = run(&mut d, json!({"type": "delete_style", "style": a})).unwrap();
    assert_eq!(out["changes"]["styles"], json!([a]));
    assert_eq!(styles(&d).len(), 1);
    assert_eq!(chosen(&d), Value::Null);
    assert!(d.chosen_style().is_none());
    assert_eq!(scene_style(&d, &on_a), Value::Null);
    assert_eq!(scene_style(&d, &on_b), json!(b));
    assert_eq!(undo_labels(&d)[0], "delete_style");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(styles(&d).len(), 2);
    assert_eq!(chosen(&d), json!(a));
    assert_eq!(scene_style(&d, &on_a), json!(a));
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(chosen(&d), Value::Null);
    assert_eq!(scene_style(&d, &on_a), Value::Null);
    let e = run(&mut d, json!({"type": "delete_style", "style": a})).unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
}

#[test]
fn undoing_the_add_of_the_chosen_style_leaves_the_default_look() {
    let mut d = doc();
    let a = add_style(&mut d, json!({}));
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(chosen(&d), Value::Null);
    assert!(d.chosen_style().is_none());
}

#[test]
fn scenes_capture_and_restore_the_chosen_style() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    let one = run(&mut d, json!({"type": "add_scene"})).unwrap()["changes"]["scenes"][0].clone();
    run(&mut d, json!({"type": "set_style", "style": null})).unwrap();
    let two = run(&mut d, json!({"type": "add_scene"})).unwrap()["changes"]["scenes"][0].clone();
    run(&mut d, json!({"type": "go_to_scene", "scene": one})).unwrap();
    assert_eq!(chosen(&d), json!(a));
    run(&mut d, json!({"type": "go_to_scene", "scene": two})).unwrap();
    assert_eq!(chosen(&d), Value::Null);
    // Update stores the current choice again.
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    run(&mut d, json!({"type": "update_scene", "scene": two})).unwrap();
    run(&mut d, json!({"type": "set_style", "style": null})).unwrap();
    run(&mut d, json!({"type": "go_to_scene", "scene": two})).unwrap();
    assert_eq!(chosen(&d), json!(a));
}

#[test]
fn files_without_styles_keep_their_schema_and_bytes() {
    let mut d = doc();
    run(&mut d, json!({"type": "add_scene"})).unwrap();
    let path = scratch("plain.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let doc = stored(&path);
    assert_eq!(doc["schema_version"], 11);
    assert!(doc["model"].get("styles").is_none());
    assert!(doc["camera"].get("style").is_none());
    assert_eq!(session::file::STYLES_SCHEMA_VERSION, 18);
    assert_eq!(session::file::SCHEMA_VERSION, 27);
}

#[test]
fn a_model_with_styles_or_a_chosen_style_is_schema_18_and_reads_back() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "settings": {
            "edges": {"profiles": 3.5, "endpoints": 6.0},
            "fog": {"start": 2.0, "end": 40.0, "color": {"rgb": [1, 2, 3]}}}}),
    )
    .unwrap();
    let path = scratch("styled.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let doc_json = stored(&path);
    assert_eq!(doc_json["schema_version"], 18);
    assert!(doc_json["camera"].get("style").is_none());
    // A chosen style is saved with the camera.
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    run(&mut d, json!({"type": "add_scene"})).unwrap();
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&path)["camera"]["style"], json!(a));
    let mut opened = doc();
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(styles(&opened), styles(&d));
    assert_eq!(chosen(&opened), json!(a));
    // Only a choice, with no style left in the model, cannot happen: a file whose camera
    // chooses a missing style is refused.
    let mut text = stored(&path);
    text["camera"]["style"] = json!("y99999");
    let bad = scratch("bad.skr");
    {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
        zip.start_file("document.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&text).unwrap()).unwrap();
        zip.finish().unwrap();
    }
    let e = run(
        &mut opened,
        json!({"type": "open", "path": bad.to_str().unwrap()}),
    )
    .unwrap_err();
    assert!(e.1.contains("style"), "{}", e.1);
    // New clears the choice.
    run(&mut opened, json!({"type": "new", "discard": true})).unwrap();
    assert_eq!(chosen(&opened), Value::Null);
    assert!(styles(&opened).is_empty());
}

#[test]
fn style_ids_are_batch_references() {
    let mut d = doc();
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_style", "name": "A"}),
        json!({"type": "change_style", "style": "$0:y0", "settings": {"background": [1, 1, 1]}}),
        json!({"type": "add_style", "name": "B", "from": "$0:y0"}),
        json!({"type": "set_style", "style": "$2:y0"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
    let list = styles(&d);
    assert_eq!(list[1]["settings"]["background"], json!([1, 1, 1]));
    assert_eq!(chosen(&d), list[1]["id"]);
    // A reference to a style where an entity goes, or the reverse, is refused.
    assert!(
        BatchCommand::parse(json!({"type": "erase", "ids": ["$0:y0"]}), 1).is_err(),
        "a style is not an entity"
    );
    assert!(
        BatchCommand::parse(json!({"type": "set_style", "style": "$0:f0"}), 1).is_err(),
        "a face is not a style"
    );
}

#[test]
fn an_atomic_batch_that_fails_restores_the_chosen_style() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    let commands: Vec<BatchCommand> = [
        json!({"type": "set_style", "style": a}),
        json!({"type": "delete_style", "style": "y99999"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    assert!(d.execute_batch(commands, true).is_err());
    assert_eq!(chosen(&d), Value::Null);
}

/// A rectangle's edge and face ids.
fn rectangle(d: &mut Document) -> Vec<kernel::EntityId> {
    run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let m = d.model();
    m.edges()
        .map(|(id, _)| kernel::EntityId::Edge(id))
        .chain(m.faces().map(|(id, _)| kernel::EntityId::Face(id)))
        .collect()
}

#[test]
fn copy_and_paste_work_with_a_style_chosen() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    let ids = rectangle(&mut d);
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    let text = d.clip(&ids).unwrap();
    let out = run(
        &mut d,
        json!({"type": "paste", "clip": text, "at": [5, 0, 0]}),
    )
    .unwrap();
    assert!(!out["changes"]["created"].as_array().unwrap().is_empty());
    assert_eq!(chosen(&d), json!(a));
}

#[test]
fn library_items_save_and_insert_with_a_style_chosen() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-styles-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    let ids = rectangle(&mut d);
    let group = run(
        &mut d,
        json!({"type": "make_component", "ids": ids, "name": "Part"}),
    )
    .unwrap()["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i.as_str().unwrap().starts_with('i'))
        .unwrap()
        .clone();
    let mat = run(
        &mut d,
        json!({"type": "add_material", "name": "Red", "color": [255, 0, 0]}),
    )
    .unwrap()["changes"]["materials"][0]
        .clone();
    run(&mut d, json!({"type": "set_style", "style": a})).unwrap();
    let component = dir.join("c.skr");
    let material = dir.join("m.skr");
    let model = dir.join("model.skr");
    for (path, params) in [
        (&component, json!({"kind": "component", "item": group})),
        (&material, json!({"kind": "material", "item": mat})),
        (&model, json!({"kind": "model"})),
    ] {
        let mut c = json!({"type": "save_to_library", "path": path.to_str().unwrap()});
        c.as_object_mut()
            .unwrap()
            .extend(params.as_object().unwrap().clone());
        run(&mut d, c).unwrap();
    }
    // Items made of part of the model carry no style, so they are written as before.
    for path in [&component, &material] {
        let doc = stored(path);
        assert!(doc["camera"].get("style").is_none(), "{path:?}");
        assert!(doc["schema_version"].as_u64().unwrap() < 18, "{path:?}");
        let mut other = self::doc();
        run(
            &mut other,
            json!({"type": "insert_library_item", "path": path.to_str().unwrap()}),
        )
        .unwrap();
    }
    // The whole model keeps its styles and the choice.
    assert_eq!(stored(&model)["camera"]["style"], json!(a));
    let mut other = self::doc();
    run(
        &mut other,
        json!({"type": "open", "path": model.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(chosen(&other), json!(a));
    run(
        &mut doc(),
        json!({"type": "insert_library_item", "path": model.to_str().unwrap()}),
    )
    .unwrap();
}

#[test]
fn ambient_occlusion_is_set_by_change_style_and_defaults_to_off() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    let s = styles(&d)[0]["settings"]["ambient_occlusion"].clone();
    assert_eq!(s, json!({"on": false, "radius": 0.25, "strength": 1.0}));
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "settings": {"ambient_occlusion": {"on": true}}}),
    )
    .unwrap();
    let s = styles(&d)[0]["settings"]["ambient_occlusion"].clone();
    assert_eq!(s, json!({"on": true, "radius": 0.25, "strength": 1.0}));
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "settings": {"ambient_occlusion": {"radius": 2.0, "strength": 0.5}}}),
    )
    .unwrap();
    let s = styles(&d)[0]["settings"]["ambient_occlusion"].clone();
    assert_eq!(s, json!({"on": true, "radius": 2.0, "strength": 0.5}));
    for bad in [
        json!({"radius": 0.0}),
        json!({"radius": 101.0}),
        json!({"strength": -0.1}),
        json!({"strength": 2.5}),
    ] {
        let e = run(
            &mut d,
            json!({"type": "change_style", "style": a, "settings": {"ambient_occlusion": bad}}),
        )
        .unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput, "{bad}");
    }
    run(&mut d, json!({"type": "undo"})).unwrap();
    let s = styles(&d)[0]["settings"]["ambient_occlusion"].clone();
    assert_eq!(s["radius"], 0.25);
}

#[test]
fn a_style_with_ambient_occlusion_is_schema_21_and_default_ones_keep_18() {
    let mut d = doc();
    let a = add_style(&mut d, json!({"name": "A"}));
    let path = scratch("ao.skr");
    let save = |d: &mut Document| {
        run(d, json!({"type": "save", "path": path.to_str().unwrap()})).unwrap();
        stored(&path)
    };
    let plain = save(&mut d);
    assert_eq!(plain["schema_version"], 18);
    assert!(
        plain["model"]["styles"][0]["settings"]
            .get("ambient_occlusion")
            .is_none()
    );
    run(
        &mut d,
        json!({"type": "change_style", "style": a, "settings": {"ambient_occlusion": {"on": true, "radius": 1.5}}}),
    )
    .unwrap();
    let occluded = save(&mut d);
    assert_eq!(occluded["schema_version"], 21);
    assert_eq!(
        occluded["schema_version"],
        session::file::AMBIENT_OCCLUSION_SCHEMA_VERSION
    );
    assert_eq!(
        occluded["model"]["styles"][0]["settings"]["ambient_occlusion"],
        json!({"on": true, "radius": 1.5, "strength": 1.0})
    );
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(styles(&e), styles(&d));
    // An older file has no such field and loads with the default.
    let mut text = plain;
    text["model"]["styles"][0]["settings"]
        .as_object_mut()
        .unwrap()
        .remove("ambient_occlusion");
    let old = scratch("old-ao.skr");
    {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&old).unwrap());
        zip.start_file("document.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&text).unwrap()).unwrap();
        zip.finish().unwrap();
    }
    run(
        &mut e,
        json!({"type": "open", "path": old.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(styles(&e)[0]["settings"]["ambient_occlusion"]["on"], false);
}
