//! Guide lines and guide points, driven through JSON commands.

use std::io::Write;
use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, Vec<String>)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.ids.iter().map(|i| i.to_string()).collect()))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

/// The guides of the open context as returned by `entities`.
fn guides(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities", "kinds": ["guide"]}))["guides"]
        .as_array()
        .unwrap()
        .clone()
}

fn undo_depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

/// A guide line along X through (0, -1, 0) (g0) and a guide point at (1, 1, 0) (g1).
fn two_guides(d: &mut Document) {
    let line = run(
        d,
        json!({"type": "add_guide_line", "point": [0, -1, 0], "direction": [3, 0, 0]}),
    )
    .unwrap();
    assert_eq!(line["changes"]["created"], json!(["g0"]));
    let point = run(d, json!({"type": "add_guide_point", "position": [1, 1, 0]})).unwrap();
    assert_eq!(point["changes"]["created"], json!(["g1"]));
}

#[test]
fn guides_are_added_listed_and_undone_one_step_each() {
    let mut d = doc();
    two_guides(&mut d);
    assert_eq!(undo_depth(&d), 2);
    assert_eq!(
        guides(&d),
        vec![
            json!({"id": "g0", "kind": "line", "point": [0.0, -1.0, 0.0], "direction": [1.0, 0.0, 0.0], "tag": null}),
            json!({"id": "g1", "kind": "point", "point": [1.0, 1.0, 0.0], "tag": null}),
        ]
    );
    let one = ask(&d, json!({"type": "entity", "id": "g1"}));
    assert_eq!(one["type"], "guide");
    assert_eq!(one["kind"], "point");
    // Guides are no geometry: no vertices, edges or faces.
    let stats = ask(&d, json!({"type": "stats"}));
    assert_eq!(
        (stats["vertices"].as_u64(), stats["edges"].as_u64()),
        (Some(0), Some(0))
    );

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(guides(&d).len(), 1);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(guides(&d).len(), 2);
}

#[test]
fn bad_guides_are_refused() {
    let mut d = doc();
    for bad in [
        json!({"type": "add_guide_line", "point": [0, 0, 0], "direction": [0, 0, 0]}),
        json!({"type": "add_guide_line", "point": [2e6, 0, 0], "direction": [1, 0, 0]}),
        json!({"type": "add_guide_point", "position": [0, -2e6, 0]}),
    ] {
        assert_eq!(run(&mut d, bad).unwrap_err().0, ErrorKind::InvalidInput);
    }
    assert!(guides(&d).is_empty());
    assert_eq!(undo_depth(&d), 0);
}

#[test]
fn a_direction_of_any_finite_length_is_taken() {
    let mut d = doc();
    for direction in [[1e200, 0.0, 0.0], [0.0, -1e-300, 0.0]] {
        run(
            &mut d,
            json!({"type": "add_guide_line", "point": [0, 0, 0], "direction": direction}),
        )
        .unwrap();
    }
    let all = guides(&d);
    assert_eq!(all[0]["direction"], json!([1.0, 0.0, 0.0]), "{all:?}");
    assert_eq!(all[1]["direction"], json!([0.0, -1.0, 0.0]), "{all:?}");
}

#[test]
fn guides_are_selected_erased_and_deleted_all_at_once() {
    let mut d = doc();
    two_guides(&mut d);
    run(&mut d, json!({"type": "select", "ids": ["g0", "g1"]})).unwrap();
    let erased = run(&mut d, json!({"type": "erase", "ids": ["g0"]})).unwrap();
    assert_eq!(erased["changes"]["deleted"], json!(["g0"]));
    assert_eq!(
        ask(&d, json!({"type": "selection"}))["ids"],
        json!(["g1"]),
        "an erased guide leaves the selection"
    );
    assert_eq!(
        run(&mut d, json!({"type": "erase", "ids": ["g0"]})).unwrap_err(),
        (ErrorKind::NotFound, vec!["g0".into()])
    );

    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    let depth = undo_depth(&d);
    let deleted = run(&mut d, json!({"type": "delete_guides"})).unwrap();
    assert_eq!(deleted["changes"]["deleted"], json!(["g1"]));
    assert!(guides(&d).is_empty());
    assert_eq!(
        ask(&d, json!({"type": "stats"}))["faces"],
        1,
        "geometry stays"
    );
    assert_eq!(undo_depth(&d), depth + 1);
    // Nothing left to delete: no error and no undo step.
    run(&mut d, json!({"type": "delete_guides"})).unwrap();
    assert_eq!(undo_depth(&d), depth + 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(guides(&d).len(), 1);
}

#[test]
fn guides_move_turn_and_copy_with_the_rest() {
    let mut d = doc();
    two_guides(&mut d);
    let moved = run(
        &mut d,
        json!({"type": "move", "ids": ["g0", "g1"], "offset": [0, 0, 2]}),
    )
    .unwrap();
    assert_eq!(moved["changes"]["modified"], json!(["g0", "g1"]));
    assert_eq!(guides(&d)[0]["point"], json!([0.0, -1.0, 2.0]));
    assert_eq!(guides(&d)[1]["point"], json!([1.0, 1.0, 2.0]));

    run(
        &mut d,
        json!({"type": "rotate", "ids": ["g0"], "center": [0, 0, 0], "angle": 90}),
    )
    .unwrap();
    let g0 = &guides(&d)[0];
    assert_eq!(g0["point"], json!([1.0, 0.0, 2.0]));
    assert_eq!(g0["direction"], json!([0.0, 1.0, 0.0]));

    let copied = run(
        &mut d,
        json!({"type": "copy", "ids": ["g1"], "offset": [1, 0, 0], "count": 2}),
    )
    .unwrap();
    assert_eq!(copied["changes"]["created"].as_array().unwrap().len(), 2);
    let points: Vec<Value> = guides(&d).iter().map(|g| g["point"].clone()).collect();
    assert!(points.contains(&json!([3.0, 1.0, 2.0])), "{points:?}");

    assert_eq!(
        run(
            &mut d,
            json!({"type": "move", "ids": ["g1"], "offset": [1e6, 0, 0]})
        )
        .unwrap_err()
        .0,
        ErrorKind::InvalidInput,
        "a guide cannot leave the coordinate limit"
    );
}

#[test]
fn guides_go_into_groups_and_come_out_when_exploded() {
    let mut d = doc();
    two_guides(&mut d);
    assert_eq!(
        run(&mut d, json!({"type": "make_group", "ids": ["g0"]}))
            .unwrap_err()
            .0,
        ErrorKind::InvalidInput,
        "guides alone make no group"
    );
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(&mut d, json!({"type": "make_group", "ids": [face, "g1"]})).unwrap();
    let group = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    assert_eq!(guides(&d).len(), 1, "only g0 stays at the root");
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    assert_eq!(guides(&d)[0]["id"], "g1", "the guide keeps its id inside");
    run(&mut d, json!({"type": "close_instance"})).unwrap();

    run(&mut d, json!({"type": "explode", "ids": [group]})).unwrap();
    let points: Vec<Value> = guides(&d).iter().map(|g| g["point"].clone()).collect();
    assert_eq!(points.len(), 2);
    assert!(points.contains(&json!([1.0, 1.0, 0.0])));
}

#[test]
fn within_finds_guide_points_but_never_guide_lines() {
    let mut d = doc();
    two_guides(&mut d);
    let found = ask(
        &d,
        json!({"type": "entities", "within": {"min": [-5, -5, -5], "max": [5, 5, 5]}}),
    );
    let ids: Vec<&Value> = found["guides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| &g["id"])
        .collect();
    assert_eq!(ids, vec![&json!("g1")]);
}

/// A fresh path in the test scratch directory.
fn scratch(name: &str) -> PathBuf {
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-guides-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// Rewrites the document inside a saved file.
fn edit_file(path: &PathBuf, f: impl FnOnce(&mut Value)) {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut document: Value =
        serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    f(&mut document);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    zip.finish().unwrap();
}

#[test]
fn guides_are_saved_but_not_exported_and_old_files_still_open() {
    let mut d = doc();
    two_guides(&mut d);
    let path = scratch("guides.skr");
    let path_text = path.to_str().unwrap();
    run(&mut d, json!({"type": "save", "path": path_text})).unwrap();
    let mut e = doc();
    run(&mut e, json!({"type": "open", "path": path_text})).unwrap();
    assert_eq!(guides(&e), guides(&d));

    let obj = scratch("guides.obj");
    assert_eq!(
        run(
            &mut d,
            json!({"type": "export", "path": obj.to_str().unwrap()})
        )
        .unwrap_err()
        .0,
        ErrorKind::InvalidInput,
        "guides are not faces: nothing to export"
    );

    // A version 1 file (before guides) opens.
    edit_file(&path, |doc| {
        doc["schema_version"] = json!(1);
        doc["model"]["root"]
            .as_object_mut()
            .unwrap()
            .remove("guides");
    });
    let mut old = doc();
    run(&mut old, json!({"type": "open", "path": path_text})).unwrap();
    assert!(guides(&old).is_empty());

    // A guide beyond the coordinate limit does not.
    run(&mut d, json!({"type": "save", "path": path_text})).unwrap();
    edit_file(&path, |doc| {
        doc["model"]["root"]["guides"][1]["point"] = json!([2e6, 0, 0]);
    });
    let mut far = doc();
    assert_eq!(
        run(&mut far, json!({"type": "open", "path": path_text}))
            .unwrap_err()
            .0,
        ErrorKind::File
    );
}

/// A group holding a unit square and a guide point at x = 900000, as (doc, group id).
fn group_with_a_far_guide() -> (Document, Value) {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]}),
    )
    .unwrap();
    let added = run(
        &mut d,
        json!({"type": "add_guide_point", "position": [900000, 0, 0]}),
    )
    .unwrap();
    let guide = added["changes"]["created"][0].clone();
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(&mut d, json!({"type": "make_group", "ids": [face, guide]})).unwrap();
    let group = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    (d, group)
}

#[test]
fn placing_a_group_refuses_to_put_its_guide_beyond_the_coordinate_limit() {
    for command in [
        json!({"type": "move", "offset": [500000, 0, 0]}),
        json!({"type": "copy", "offset": [500000, 0, 0]}),
    ] {
        let (mut d, group) = group_with_a_far_guide();
        let mut command = command;
        command["ids"] = json!([group]);
        let before = ask(&d, json!({"type": "entities"}));
        let depth = undo_depth(&d);
        let err = run(&mut d, command.clone()).unwrap_err();
        assert_eq!(err.0, ErrorKind::InvalidInput, "{command}");
        assert_eq!(err.1, vec![group.as_str().unwrap().to_string()]);
        assert_eq!(ask(&d, json!({"type": "entities"})), before);
        assert_eq!(undo_depth(&d), depth);
    }
}

#[test]
fn rotating_a_group_refuses_to_swing_its_guide_beyond_the_coordinate_limit() {
    let (mut d, group) = group_with_a_far_guide();
    // A quarter turn about a center 900000 on the other side of the origin puts the guide
    // at (-900000, 1.8e6, 0).
    let err = run(
        &mut d,
        json!({"type": "rotate", "ids": [group], "center": [-900000, 0, 0], "axis": [0, 0, 1], "angle": 90}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
    assert_eq!(err.1, vec![group.as_str().unwrap().to_string()]);
}

#[test]
fn a_guide_line_direction_is_only_normalized_not_range_checked() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_guide_line", "point": [0, 1, 0], "direction": [2e6, 0, 0]}),
    )
    .unwrap();
    let line = &guides(&d)[0];
    assert_eq!(line["direction"], json!([1.0, 0.0, 0.0]));
}

#[test]
fn a_non_finite_guide_line_direction_is_refused() {
    let mut d = doc();
    for direction in [[f64::NAN, 0.0, 0.0], [1.0, f64::INFINITY, 0.0]] {
        let err = d
            .execute(Command::AddGuideLine {
                point: [0.0, 0.0, 0.0],
                direction,
            })
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
    }
    assert!(guides(&d).is_empty());
}
