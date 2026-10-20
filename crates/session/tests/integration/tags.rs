//! Tags, driven through JSON commands and queries, and kept in files.

use std::io::Write;
use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind, Query};

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

fn scratch(name: &str) -> PathBuf {
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-tags-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// The file's `document.json`.
fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

/// Rewrites the file's `document.json` through `f`.
fn edit_file(path: &PathBuf, f: impl FnOnce(&mut Value)) {
    let mut document = stored(path);
    f(&mut document);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    zip.finish().unwrap();
}

/// A rectangle on the ground: its face and its four edges.
fn rectangle(d: &mut Document, corner: [f64; 2], opposite: [f64; 2]) -> (String, Vec<String>) {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [corner[0], corner[1], 0],
               "opposite": [opposite[0], opposite[1], 0]}),
    )
    .unwrap();
    let created: Vec<String> = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let face = created
        .iter()
        .find(|id| id.starts_with('f'))
        .unwrap()
        .clone();
    let edges = created
        .into_iter()
        .filter(|id| id.starts_with('e'))
        .collect();
    (face, edges)
}

/// A 2 x 2 square at the origin: its face and its four edges.
fn square(d: &mut Document) -> (String, Vec<String>) {
    rectangle(d, [0.0, 0.0], [2.0, 2.0])
}

/// Adds a tag and returns its id.
fn add(d: &mut Document, v: Value) -> String {
    let mut v = v;
    v["type"] = json!("add_tag");
    let out = run(d, v).unwrap();
    out["changes"]["tags"][0].as_str().unwrap().to_string()
}

fn tags(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "tags"}))["tags"]
        .as_array()
        .unwrap()
        .clone()
}

fn tag_of(d: &Document, id: &str) -> Value {
    ask(d, json!({"type": "entity", "id": id}))["tag"].clone()
}

fn history(d: &Document) -> Value {
    ask(d, json!({"type": "history"}))
}

#[test]
fn add_tag_names_and_colors_new_tags_in_turn() {
    let mut d = doc();
    let first = add(&mut d, json!({}));
    let second = add(&mut d, json!({}));
    let named = add(&mut d, json!({"name": "Walls", "color": [1, 2, 3]}));
    assert!(first.starts_with('t'), "{first}");
    let all = tags(&d);
    assert_eq!(
        all[0],
        json!({"id": first, "name": "Tag", "color": kernel::TAG_COLORS[0], "visible": true, "count": 0})
    );
    assert_eq!(all[1]["name"], "Tag 2");
    assert_eq!(all[1]["id"], json!(second));
    assert_eq!(all[1]["color"], json!(kernel::TAG_COLORS[1]));
    assert_eq!(all[2]["name"], "Walls");
    assert_eq!(all[2]["id"], json!(named));
    assert_eq!(all[2]["color"], json!([1, 2, 3]));
    // The next default color skips none used: the third of the palette.
    add(&mut d, json!({}));
    assert_eq!(tags(&d)[3]["color"], json!(kernel::TAG_COLORS[2]));

    for bad in ["", "   ", "Walls", "Untagged"] {
        let e = run(&mut d, json!({"type": "add_tag", "name": bad})).unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput, "{bad:?}: {e:?}");
    }
    let long = "x".repeat(257);
    let e = run(&mut d, json!({"type": "add_tag", "name": long})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert_eq!(history(&d)["undo"][0], "add_tag");
}

#[test]
fn change_tag_renames_recolors_and_hides_as_undo_steps() {
    let mut d = doc();
    let t = add(&mut d, json!({"name": "Walls"}));
    let saved = d.is_modified();
    assert!(saved);
    let out = run(
        &mut d,
        json!({"type": "change_tag", "tag": t, "name": "Old walls", "color": [9, 9, 9]}),
    )
    .unwrap();
    assert_eq!(out["changes"]["tags"], json!([t]));
    let out = run(
        &mut d,
        json!({"type": "change_tag", "tag": t, "visible": false}),
    )
    .unwrap();
    assert_eq!(out["changes"]["tags"], json!([t]));
    assert_eq!(
        tags(&d)[0],
        json!({"id": t, "name": "Old walls", "color": [9, 9, 9], "visible": false, "count": 0})
    );
    assert_eq!(
        history(&d)["undo"],
        json!(["change_tag", "change_tag", "add_tag"])
    );
    // Hiding again changes nothing and makes no step.
    run(
        &mut d,
        json!({"type": "change_tag", "tag": t, "visible": false}),
    )
    .unwrap();
    assert_eq!(history(&d)["undo_depth"], 3);

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(tags(&d)[0]["visible"], true);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(tags(&d)[0]["name"], "Walls");
    run(&mut d, json!({"type": "redo", "count": 2})).unwrap();
    assert_eq!(tags(&d)[0]["visible"], false);
    assert_eq!(tags(&d)[0]["name"], "Old walls");

    let e = run(
        &mut d,
        json!({"type": "change_tag", "tag": "t999", "visible": true}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    assert!(e.1.contains("t999"), "{}", e.1);
    let other = add(&mut d, json!({"name": "Roof"}));
    let e = run(
        &mut d,
        json!({"type": "change_tag", "tag": other, "name": "Old walls"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
}

#[test]
fn set_tag_takes_edges_faces_instances_and_guides_in_one_step() {
    let mut d = doc();
    let (f, e) = square(&mut d);
    let (other, _) = rectangle(&mut d, [5.0, 0.0], [7.0, 2.0]);
    let group =
        run(&mut d, json!({"type": "make_group", "ids": [other]})).unwrap()["changes"]["created"]
            .as_array()
            .unwrap()
            .iter()
            .find(|id| id.as_str().unwrap().starts_with('i'))
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
    let guide = run(
        &mut d,
        json!({"type": "add_guide_point", "position": [1, 1, 1]}),
    )
    .unwrap()["changes"]["created"][0]
        .as_str()
        .unwrap()
        .to_string();
    let t = add(&mut d, json!({"name": "Walls"}));
    // Untagged entities report a null tag.
    assert_eq!(tag_of(&d, &f), Value::Null);
    assert!(
        ask(&d, json!({"type": "entity", "id": f}))
            .as_object()
            .unwrap()
            .contains_key("tag")
    );

    let ids = json!([e[0], f, group, guide]);
    let out = run(&mut d, json!({"type": "set_tag", "ids": ids, "tag": t})).unwrap();
    assert_eq!(out["changes"]["modified"].as_array().unwrap().len(), 4);
    for id in [&e[0], &f, &group, &guide] {
        assert_eq!(tag_of(&d, id), json!(t), "{id}");
    }
    assert_eq!(tag_of(&d, &e[1]), Value::Null);
    assert_eq!(tags(&d)[0]["count"], 4);
    assert_eq!(history(&d)["undo"][0], "set_tag");
    let entities = ask(&d, json!({"type": "entities", "ids": [f, group]}));
    assert_eq!(entities["faces"][0]["tag"], json!(t));
    assert_eq!(entities["instances"][0]["tag"], json!(t));

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(tag_of(&d, &f), Value::Null);
    assert_eq!(tag_of(&d, &group), Value::Null);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(tag_of(&d, &guide), json!(t));
    // null puts them back on Untagged.
    run(&mut d, json!({"type": "set_tag", "ids": [f], "tag": null})).unwrap();
    assert_eq!(tag_of(&d, &f), Value::Null);

    // Inside the group, the count covers the whole model.
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let inside = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [inside], "tag": t}),
    )
    .unwrap();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    assert_eq!(tags(&d)[0]["count"], 4);

    let depth = history(&d)["undo_depth"].clone();
    let e = run(
        &mut d,
        json!({"type": "set_tag", "ids": [f, "v0"], "tag": t}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert_eq!(e.2, vec!["v0".to_string()]);
    let e = run(&mut d, json!({"type": "set_tag", "ids": [], "tag": t})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    let e = run(
        &mut d,
        json!({"type": "set_tag", "ids": ["f999"], "tag": t}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    let e = run(
        &mut d,
        json!({"type": "set_tag", "ids": [f], "tag": "t999"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    assert!(e.1.contains("t999"), "{}", e.1);
    assert_eq!(history(&d)["undo_depth"], depth);
    assert!(
        serde_json::from_value::<Command>(json!({"type": "set_tag", "ids": [f], "tag": "m3"}))
            .is_err()
    );
}

#[test]
fn delete_tag_moves_entities_in_one_undo_step() {
    for move_to in [false, true] {
        let mut d = doc();
        let (f, e) = square(&mut d);
        let on = [f.clone(), e[0].clone(), e[1].clone()];
        let walls = add(&mut d, json!({"name": "Walls"}));
        let roof = add(&mut d, json!({"name": "Roof"}));
        run(&mut d, json!({"type": "set_tag", "ids": on, "tag": walls})).unwrap();
        let command = if move_to {
            json!({"type": "delete_tag", "tag": walls, "move_to": roof})
        } else {
            json!({"type": "delete_tag", "tag": walls})
        };
        let out = run(&mut d, command).unwrap();
        assert_eq!(out["changes"]["tags"], json!([walls]));
        let expected = if move_to { json!(roof) } else { Value::Null };
        for id in &on {
            assert_eq!(tag_of(&d, id), expected, "{id}");
        }
        let all = tags(&d);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["count"], if move_to { 3 } else { 0 });
        assert_eq!(history(&d)["undo"][0], "delete_tag");

        run(&mut d, json!({"type": "undo"})).unwrap();
        assert_eq!(tags(&d).len(), 2);
        for id in &on {
            assert_eq!(tag_of(&d, id), json!(walls), "{id}");
        }
        assert_eq!(tag_of(&d, &e[2]), Value::Null);
        run(&mut d, json!({"type": "redo"})).unwrap();
        for id in &on {
            assert_eq!(tag_of(&d, id), expected, "{id}");
        }
    }

    let mut d = doc();
    let walls = add(&mut d, json!({"name": "Walls"}));
    let e = run(&mut d, json!({"type": "delete_tag", "tag": "t999"})).unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    let e = run(
        &mut d,
        json!({"type": "delete_tag", "tag": walls, "move_to": "t999"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    assert!(e.1.contains("t999"), "{}", e.1);
    let e = run(
        &mut d,
        json!({"type": "delete_tag", "tag": walls, "move_to": walls}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert_eq!(tags(&d).len(), 1);
}

#[test]
fn copies_and_explodes_keep_tags_through_commands() {
    let mut d = doc();
    let (f, e) = square(&mut d);
    let walls = add(&mut d, json!({"name": "Walls"}));
    let roof = add(&mut d, json!({"name": "Roof"}));
    let mut all = e.clone();
    all.push(f.clone());
    run(&mut d, json!({"type": "set_tag", "ids": all, "tag": walls})).unwrap();
    let copied = run(
        &mut d,
        json!({"type": "copy", "ids": [f], "offset": [5, 0, 0]}),
    )
    .unwrap()["changes"]["created"]
        .clone();
    let copied: Vec<String> = copied
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .filter(|id| !id.starts_with('v'))
        .collect();
    assert_eq!(copied.len(), 5, "{copied:?}");
    for id in &copied {
        assert_eq!(tag_of(&d, id), json!(walls), "{id}");
    }

    // Grouped, the group on Roof, exploded: the geometry keeps Walls.
    let out = run(&mut d, json!({"type": "make_group", "ids": copied})).unwrap();
    let group = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('i'))
        .unwrap()
        .clone();
    assert_eq!(tag_of(&d, group.as_str().unwrap()), Value::Null);
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [group], "tag": roof}),
    )
    .unwrap();
    run(&mut d, json!({"type": "explode", "ids": [group]})).unwrap();
    let entities = ask(&d, json!({"type": "entities", "kinds": ["face"]}));
    let faces = entities["faces"].as_array().unwrap();
    assert_eq!(faces.len(), 2);
    assert!(faces.iter().all(|f| f["tag"] == json!(walls)), "{faces:?}");
}

#[test]
fn tags_are_saved_and_opened_with_their_entities() {
    let mut d = doc();
    let (f, e) = square(&mut d);
    let walls = add(&mut d, json!({"name": "Walls", "color": [10, 20, 30]}));
    let hidden = add(&mut d, json!({"name": "Hidden"}));
    run(
        &mut d,
        json!({"type": "change_tag", "tag": hidden, "visible": false}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [f, e[0]], "tag": walls}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [e[1]], "tag": hidden}),
    )
    .unwrap();
    let path = scratch("tags.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(document["schema_version"], session::file::SCHEMA_VERSION);
    assert_eq!(document["schema_version"], 10);
    let before = tags(&d);

    let mut opened = doc();
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(tags(&opened), before);
    assert_eq!(tags(&opened)[1]["visible"], false);
    assert_eq!(tag_of(&opened, &f), json!(walls));
    assert_eq!(tag_of(&opened, &e[0]), json!(walls));
    assert_eq!(tag_of(&opened, &e[1]), json!(hidden));
    assert_eq!(tag_of(&opened, &e[2]), Value::Null);

    // An entity on a tag the file lacks is refused.
    let broken = scratch("broken-tag.skr");
    std::fs::copy(&path, &broken).unwrap();
    edit_file(&broken, |doc| {
        doc["model"]["root"]["faces"][0]["tag"] = json!(999_999);
    });
    let e = run(
        &mut opened,
        json!({"type": "open", "path": broken.to_str().unwrap(), "discard": true}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::File);
    assert!(e.1.contains("missing tag"), "{}", e.1);
}

#[test]
fn a_model_without_tags_keeps_its_old_schema_and_old_files_open_untagged() {
    let mut d = doc();
    let (f, e) = square(&mut d);
    let path = scratch("untagged.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(
        document["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
    assert!(!document.to_string().contains("tag"), "{document}");

    // A schema 9 file (before tags) opens with no tags and everything Untagged.
    edit_file(&path, |doc| doc["schema_version"] = json!(9));
    let mut opened = doc();
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert!(tags(&opened).is_empty());
    assert_eq!(tag_of(&opened, &f), Value::Null);
    assert_eq!(tag_of(&opened, &e[0]), Value::Null);
}

#[test]
fn set_tag_ids_take_batch_references_and_tag_fields_do_not() {
    let mut d = doc();
    let t = add(&mut d, json!({"name": "Walls"}));
    let commands = vec![
        BatchCommand::parse(
            json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
            0,
        )
        .unwrap(),
        BatchCommand::parse(json!({"type": "set_tag", "ids": ["$0:*"], "tag": t}), 1).unwrap(),
    ];
    d.execute_batch(commands, true).unwrap();
    assert_eq!(tags(&d)[0]["count"], 5);
    assert!(
        BatchCommand::parse(json!({"type": "set_tag", "ids": ["f1"], "tag": "$0:t0"}), 1).is_err()
    );
    assert!(BatchCommand::parse(json!({"type": "delete_tag", "tag": "$0:m0"}), 1).is_err());
}

#[test]
fn follow_me_makes_untagged_geometry_from_a_tagged_profile() {
    let mut d = doc();
    let path = run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [2, 0, 0]}),
    )
    .unwrap()["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('e'))
        .unwrap()
        .clone();
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [0, 1, 1], "normal": [1, 0, 0]}),
    )
    .unwrap();
    let old: Vec<Value> = out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| !id.as_str().unwrap().starts_with('v'))
        .cloned()
        .collect();
    let face = old
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    let walls = add(&mut d, json!({"name": "Walls"}));
    run(&mut d, json!({"type": "set_tag", "ids": old, "tag": walls})).unwrap();
    run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": [path]}),
    )
    .unwrap();
    let all = ask(&d, json!({"type": "entities", "kinds": ["edge", "face"]}));
    let faces = all["faces"].as_array().unwrap();
    assert_eq!(faces.len(), 6);
    assert!(faces.iter().all(|f| f["tag"].is_null()), "{faces:?}");
    // Edges the sweep made are Untagged; the profile's edges that stay keep their tag.
    let edges = all["edges"].as_array().unwrap();
    assert!(edges.iter().any(|e| old.contains(&e["id"])), "{edges:?}");
    assert!(edges.iter().any(|e| !old.contains(&e["id"])), "{edges:?}");
    for e in edges {
        let kept = old.contains(&e["id"]);
        let expected = if kept { json!(walls) } else { Value::Null };
        assert_eq!(e["tag"], expected, "{e}");
    }
}

/// Hides a tag.
fn hide(d: &mut Document, tag: &str) {
    run(
        d,
        json!({"type": "change_tag", "tag": tag, "visible": false}),
    )
    .unwrap();
}

/// Groups `ids` and returns the instance.
fn group(d: &mut Document, ids: Vec<String>) -> String {
    let out = run(d, json!({"type": "make_group", "ids": ids})).unwrap();
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .find(|id| id.starts_with('i'))
        .unwrap()
}

fn export(d: &mut Document, name: &str) -> Result<Value, (ErrorKind, String, Vec<String>)> {
    let path = scratch(name);
    run(d, json!({"type": "export", "path": path.to_str().unwrap()}))
}

#[test]
fn exports_leave_out_what_hidden_tags_hide() {
    let mut d = doc();
    // A loose square, and a square grouped inside a group whose inner face is on a shown tag.
    let (loose, _) = square(&mut d);
    let (inner, inner_edges) = rectangle(&mut d, [5.0, 0.0], [7.0, 2.0]);
    let shown = add(&mut d, json!({"name": "Shown"}));
    let hidden = add(&mut d, json!({"name": "Hidden"}));
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [inner], "tag": shown}),
    )
    .unwrap();
    let mut ids = inner_edges.clone();
    ids.push(inner);
    let g = group(&mut d, ids);
    let triangles = |out: Value| out["export"]["triangles"].as_u64().unwrap();
    assert_eq!(triangles(export(&mut d, "all.stl").unwrap()), 4);
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [g], "tag": hidden}),
    )
    .unwrap();
    hide(&mut d, &hidden);
    for name in ["shown.obj", "shown.stl", "shown.glb", "shown.gltf"] {
        assert_eq!(triangles(export(&mut d, name).unwrap()), 2, "{name}");
    }
    let path = scratch("shown-again.obj");
    run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let obj = std::fs::read_to_string(&path).unwrap();
    assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 2);
    // A face on a hidden tag goes too; with nothing shown there is nothing to export.
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [loose], "tag": hidden}),
    )
    .unwrap();
    let e = export(&mut d, "none.stl").unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(e.1.contains("hidden"), "{}", e.1);
}

#[test]
fn zoom_extents_fits_only_what_is_shown() {
    let mut d = doc();
    square(&mut d);
    let (far, far_edges) = rectangle(&mut d, [100.0, 0.0], [102.0, 2.0]);
    let away = add(&mut d, json!({"name": "Away"}));
    let mut ids = far_edges;
    ids.push(far);
    run(&mut d, json!({"type": "set_tag", "ids": ids, "tag": away})).unwrap();
    let target = |d: &Document| ask(d, json!({"type": "camera"}))["target"].clone();
    run(&mut d, json!({"type": "zoom_extents"})).unwrap();
    assert_eq!(target(&d)[0], json!(51.0), "both squares");
    hide(&mut d, &away);
    run(&mut d, json!({"type": "zoom_extents"})).unwrap();
    assert_eq!(target(&d)[0], json!(1.0), "the shown square only");
    // Everything hidden: as for an empty model, the camera stays.
    let rest: Vec<String> = ask(&d, json!({"type": "entities", "kinds": ["edge", "face"]}))
        .as_object()
        .unwrap()
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    run(&mut d, json!({"type": "set_tag", "ids": rest, "tag": away})).unwrap();
    let before = ask(&d, json!({"type": "camera"}));
    run(&mut d, json!({"type": "zoom_extents"})).unwrap();
    assert_eq!(ask(&d, json!({"type": "camera"})), before);
}

#[test]
fn color_by_tag_is_a_view_setting_saved_with_the_camera() {
    let mut d = doc();
    square(&mut d);
    let camera = |d: &Document| ask(d, json!({"type": "camera"}));
    assert_eq!(camera(&d)["color_by_tag"], false);
    let revision = d.revision();
    run(&mut d, json!({"type": "set_color_by_tag", "on": true})).unwrap();
    assert_eq!(camera(&d)["color_by_tag"], true);
    assert_eq!(d.revision(), revision, "not an edit");
    assert_eq!(history(&d)["undo"].as_array().unwrap().len(), 1);
    // A failed atomic batch puts it back.
    let batch: Vec<Command> = vec![
        serde_json::from_value(json!({"type": "set_color_by_tag", "on": false})).unwrap(),
        serde_json::from_value(json!({"type": "delete_tag", "tag": "t999"})).unwrap(),
    ];
    assert!(d.execute_batch(batch, true).is_err());
    assert_eq!(camera(&d)["color_by_tag"], true);

    // Saved with the camera, at the tags' schema even without a tag; opened back.
    let path = scratch("color-by-tag.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(document["schema_version"], 10);
    assert_eq!(document["camera"]["color_by_tag"], true);
    let mut opened = doc();
    run(
        &mut opened,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(camera(&opened)["color_by_tag"], true);
    run(&mut opened, json!({"type": "new", "discard": true})).unwrap();
    assert_eq!(camera(&opened)["color_by_tag"], false);

    // Off, nothing is written.
    run(&mut d, json!({"type": "set_color_by_tag", "on": false})).unwrap();
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap(), "overwrite": true}),
    )
    .unwrap();
    assert!(!stored(&path).to_string().contains("color_by_tag"));
}

#[test]
fn the_outliner_marks_what_hidden_tags_hide() {
    let mut d = doc();
    let (f, e) = square(&mut d);
    let mut ids = e;
    ids.push(f);
    let inner = group(&mut d, ids);
    let outer = group(&mut d, vec![inner.clone()]);
    let hidden = add(&mut d, json!({"name": "Hidden"}));
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [outer], "tag": hidden}),
    )
    .unwrap();
    let tree = |d: &Document| ask(d, json!({"type": "outliner"}))["instances"].clone();
    assert!(tree(&d)[0].get("hidden").is_none());
    hide(&mut d, &hidden);
    let t = tree(&d);
    assert_eq!(t[0]["id"], json!(outer));
    assert_eq!(t[0]["hidden"], true);
    assert_eq!(t[0]["children"][0]["id"], json!(inner));
    assert_eq!(
        t[0]["children"][0]["hidden"], true,
        "hidden by the group above"
    );
    // Hiding does not change the selection.
    run(&mut d, json!({"type": "select", "ids": [outer]})).unwrap();
    run(
        &mut d,
        json!({"type": "change_tag", "tag": hidden, "visible": true}),
    )
    .unwrap();
    hide(&mut d, &hidden);
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([outer]));
}
