//! Section planes and the section display settings, driven through JSON commands and
//! queries, and kept in files.

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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-sections-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

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

fn created(out: &Value, prefix: char) -> Vec<String> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .filter(|s| s.starts_with(prefix))
        .collect()
}

fn made(out: &Value, prefix: char) -> String {
    let ids = created(out, prefix);
    assert_eq!(ids.len(), 1, "{out}");
    ids[0].clone()
}

/// Adds a plane through `point` with `normal` and returns its id.
fn plane(d: &mut Document, point: [f64; 3], normal: [f64; 3], activate: bool) -> String {
    let out = run(
        d,
        json!({"type": "add_section_plane", "point": point, "normal": normal, "activate": activate}),
    )
    .unwrap();
    made(&out, 's')
}

fn planes(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "section_planes"}))["planes"]
        .as_array()
        .unwrap()
        .clone()
}

fn active(d: &Document) -> Vec<String> {
    planes(d)
        .iter()
        .filter(|p| p["active"] == true)
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect()
}

fn undo_depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

fn near(a: &Value, b: [f64; 3]) {
    for k in 0..3 {
        let x = a[k].as_f64().unwrap();
        assert!((x - b[k]).abs() < 1e-9, "{a} vs {b:?}");
    }
}

/// A 2 x 2 x 1 box on the ground: its edges and faces.
fn cube(d: &mut Document) -> Vec<String> {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    let face = made(&out, 'f');
    run(d, json!({"type": "push_pull", "face": face, "distance": 1})).unwrap();
    let all = ask(d, json!({"type": "entities"}));
    all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn adding_a_plane_normalizes_the_normal_and_activates_it() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_section_plane", "point": [1, 2, 3], "normal": [0, 0, 5]}),
    )
    .unwrap();
    let s = made(&out, 's');
    let list = planes(&d);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], s);
    assert_eq!(list[0]["active"], true);
    near(&list[0]["point"], [1.0, 2.0, 3.0]);
    near(&list[0]["normal"], [0.0, 0.0, 1.0]);
    let info = ask(&d, json!({"type": "entity", "id": s}));
    assert_eq!(info["id"], s);
    assert_eq!(info["active"], true);
    near(&info["normal"], [0.0, 0.0, 1.0]);
    assert_eq!(undo_depth(&d), 1);
}

#[test]
fn a_plane_can_be_added_inactive() {
    let mut d = doc();
    let a = plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], true);
    let b = plane(&mut d, [0.0; 3], [1.0, 0.0, 0.0], false);
    assert_eq!(active(&d), vec![a.clone()]);
    assert_eq!(ask(&d, json!({"type": "entity", "id": b}))["active"], false);
}

#[test]
fn a_zero_or_far_plane_is_refused() {
    let mut d = doc();
    for bad in [
        json!({"type": "add_section_plane", "point": [0, 0, 0], "normal": [0, 0, 0]}),
        json!({"type": "add_section_plane", "point": [2e7, 0, 0], "normal": [0, 0, 1]}),
    ] {
        let (kind, ..) = run(&mut d, bad).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput);
    }
    let nan = Command::AddSectionPlane {
        point: [0.0; 3],
        normal: [f64::NAN, 0.0, 1.0],
        activate: None,
    };
    assert_eq!(d.execute(nan).unwrap_err().kind, ErrorKind::InvalidInput);
    assert!(planes(&d).is_empty());
    assert_eq!(undo_depth(&d), 0);
}

#[test]
fn flip_activate_and_deactivate_are_exact_undo_steps() {
    let mut d = doc();
    let a = plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], false);
    let b = plane(&mut d, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], false);
    assert!(active(&d).is_empty());

    run(
        &mut d,
        json!({"type": "activate_section_plane", "plane": a}),
    )
    .unwrap();
    assert_eq!(active(&d), vec![a.clone()]);
    run(
        &mut d,
        json!({"type": "activate_section_plane", "plane": b}),
    )
    .unwrap();
    assert_eq!(active(&d), vec![b.clone()]);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(active(&d), vec![a.clone()]);

    run(&mut d, json!({"type": "flip_section_plane", "plane": a})).unwrap();
    let flipped = ask(&d, json!({"type": "entity", "id": a}));
    near(&flipped["normal"], [0.0, 0.0, -1.0]);
    near(&flipped["point"], [0.0, 0.0, 0.0]);
    assert_eq!(flipped["active"], true, "flipping keeps it active");
    run(&mut d, json!({"type": "undo"})).unwrap();
    near(
        &ask(&d, json!({"type": "entity", "id": a}))["normal"],
        [0.0, 0.0, 1.0],
    );

    let before = undo_depth(&d);
    run(&mut d, json!({"type": "deactivate_section"})).unwrap();
    assert!(active(&d).is_empty());
    assert_eq!(undo_depth(&d), before + 1);
    run(&mut d, json!({"type": "deactivate_section"})).unwrap();
    assert_eq!(undo_depth(&d), before + 1, "nothing active: no step");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(active(&d), vec![a.clone()]);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert!(active(&d).is_empty());

    // Activating the active plane again changes nothing.
    run(
        &mut d,
        json!({"type": "activate_section_plane", "plane": a}),
    )
    .unwrap();
    let depth = undo_depth(&d);
    run(
        &mut d,
        json!({"type": "activate_section_plane", "plane": a}),
    )
    .unwrap();
    assert_eq!(undo_depth(&d), depth);
}

#[test]
fn planes_of_another_context_are_not_found_and_no_plane_is_invalid_input() {
    let mut d = doc();
    let ids = cube(&mut d);
    let root_plane = plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], false);
    let out = run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let i = made(&out, 'i');
    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    for other in [
        json!({"type": "activate_section_plane", "plane": root_plane}),
        json!({"type": "flip_section_plane", "plane": root_plane}),
    ] {
        let (kind, ..) = run(&mut d, other.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::NotFound, "{other}");
    }
    for bad in [
        json!({"type": "activate_section_plane", "plane": "s999"}),
        json!({"type": "activate_section_plane", "plane": "f1"}),
    ] {
        let (kind, ..) = run(&mut d, bad.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{bad}");
    }
}

#[test]
fn erasing_the_active_plane_and_undoing_restores_it_active() {
    let mut d = doc();
    let a = plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], true);
    run(&mut d, json!({"type": "erase", "ids": [a]})).unwrap();
    assert!(planes(&d).is_empty());
    assert!(active(&d).is_empty());
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(active(&d), vec![a.clone()]);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert!(planes(&d).is_empty());
    let (kind, ..) = run(&mut d, json!({"type": "erase", "ids": [a]})).unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
}

#[test]
fn move_rotate_and_scale_carry_planes() {
    let mut d = doc();
    let a = plane(&mut d, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], true);
    run(
        &mut d,
        json!({"type": "move", "ids": [a], "offset": [0, 0, 2]}),
    )
    .unwrap();
    let info = ask(&d, json!({"type": "entity", "id": a}));
    near(&info["point"], [1.0, 0.0, 2.0]);
    assert_eq!(info["active"], true);
    assert_eq!(undo_depth(&d), 2);

    run(
        &mut d,
        json!({"type": "rotate", "ids": [a], "center": [0, 0, 0], "angle": 90}),
    )
    .unwrap();
    let info = ask(&d, json!({"type": "entity", "id": a}));
    near(&info["point"], [0.0, 1.0, 2.0]);
    near(&info["normal"], [0.0, 1.0, 0.0]);

    // A stretch bends the normal as a plane's normal bends.
    let b = plane(&mut d, [0.0; 3], [0.6, 0.8, 0.0], false);
    run(
        &mut d,
        json!({"type": "scale", "ids": [b], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    )
    .unwrap();
    let n = ask(&d, json!({"type": "entity", "id": b}))["normal"].clone();
    let len = (0.3f64 * 0.3 + 0.8 * 0.8).sqrt();
    near(&n, [0.3 / len, 0.8 / len, 0.0]);
    run(&mut d, json!({"type": "undo"})).unwrap();
    near(
        &ask(&d, json!({"type": "entity", "id": b}))["normal"],
        [0.6, 0.8, 0.0],
    );
}

#[test]
fn moving_a_plane_past_the_coordinate_limit_is_refused() {
    let mut d = doc();
    let a = plane(&mut d, [9.0e5, 0.0, 0.0], [1.0, 0.0, 0.0], false);
    for cmd in [
        json!({"type": "move", "ids": [a], "offset": [2.0e5, 0, 0]}),
        json!({"type": "copy", "ids": [a], "offset": [2.0e5, 0, 0]}),
        json!({"type": "scale", "ids": [a], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    ] {
        let (kind, ..) = run(&mut d, cmd.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{cmd}");
    }
    assert_eq!(planes(&d).len(), 1);
}

#[test]
fn copies_of_a_plane_are_inactive() {
    let mut d = doc();
    let a = plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], true);
    let out = run(
        &mut d,
        json!({"type": "copy", "ids": [a], "offset": [0, 0, 1], "count": 2}),
    )
    .unwrap();
    let copies = created(&out, 's');
    assert_eq!(copies.len(), 2);
    assert_eq!(active(&d), vec![a.clone()]);
    near(
        &ask(&d, json!({"type": "entity", "id": copies[1]}))["point"],
        [0.0, 0.0, 2.0],
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(planes(&d).len(), 1);

    // A rotated array copies it as well.
    let out = run(
        &mut d,
        json!({"type": "rotate", "ids": [a], "center": [0, 0, 0], "axis": [1, 0, 0], "angle": 90, "copies": 1}),
    )
    .unwrap();
    assert_eq!(created(&out, 's').len(), 1);
    assert_eq!(planes(&d).len(), 2);
}

#[test]
fn select_takes_planes_and_select_all_lists_them() {
    let mut d = doc();
    let a = plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], true);
    run(&mut d, json!({"type": "select", "ids": [a.clone()]})).unwrap();
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([a]));
    run(&mut d, json!({"type": "select", "ids": [], "all": true})).unwrap();
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([a]));
    let (kind, ..) = run(&mut d, json!({"type": "select", "ids": ["s99"]})).unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    // A hidden tag hides the plane from select all.
    let out = run(&mut d, json!({"type": "add_tag", "name": "Cuts"})).unwrap();
    let tag = out["changes"]["tags"][0].clone();
    run(&mut d, json!({"type": "set_tag", "ids": [a], "tag": tag})).unwrap();
    run(
        &mut d,
        json!({"type": "change_tag", "tag": tag, "visible": false}),
    )
    .unwrap();
    run(&mut d, json!({"type": "select", "ids": [], "all": true})).unwrap();
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([]));
}

#[test]
fn grouping_moves_planes_with_the_geometry_and_explode_brings_them_out_inactive() {
    let mut d = doc();
    let mut ids = cube(&mut d);
    let a = plane(&mut d, [1.0, 1.0, 0.5], [0.0, 0.0, 1.0], true);
    ids.push(a.clone());
    let out = run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let i = made(&out, 'i');
    assert!(planes(&d).is_empty(), "the root keeps none");
    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    assert_eq!(active(&d), vec![a.clone()], "active in the new definition");
    run(&mut d, json!({"type": "close_instance"})).unwrap();

    run(&mut d, json!({"type": "explode", "ids": [i]})).unwrap();
    let list = planes(&d);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["active"], false);
    assert!(active(&d).is_empty());
    near(&list[0]["point"], [1.0, 1.0, 0.5]);
    assert_eq!(
        ask(&d, json!({"type": "selection"}))["ids"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v.as_str().unwrap().starts_with('s'))
            .count(),
        1,
        "what comes out is selected"
    );
}

#[test]
fn planes_alone_cannot_be_grouped() {
    let mut d = doc();
    let a = plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], true);
    let (kind, ..) = run(&mut d, json!({"type": "make_group", "ids": [a]})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}

#[test]
fn a_clip_carries_planes_inactive() {
    let mut d = doc();
    let ids = cube(&mut d);
    let a = plane(&mut d, [1.0, 1.0, 0.5], [0.0, 0.0, 1.0], true);
    let mut with = ids.clone();
    with.push(a.clone());
    let parsed: Vec<kernel::EntityId> = with.iter().map(|s| s.parse().unwrap()).collect();
    let clip = d.clip(&parsed).unwrap();
    run(
        &mut d,
        json!({"type": "paste", "clip": clip, "at": [10, 0, 0]}),
    )
    .unwrap();
    let pasted: Vec<String> = ask(&d, json!({"type": "selection"}))["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .filter(|s| s.starts_with('s'))
        .collect();
    assert_eq!(pasted.len(), 1);
    assert_ne!(pasted[0], a);
    let info = ask(&d, json!({"type": "entity", "id": pasted[0]}));
    assert_eq!(info["active"], false);
    assert_eq!(active(&d), vec![a.clone()]);
    near(&info["normal"], [0.0, 0.0, 1.0]);

    // Pasting beyond the limit is refused whole.
    let one = d.clip(&[pasted[0].parse().unwrap()]).unwrap();
    let far = run(
        &mut d,
        json!({"type": "paste", "clip": one, "at": [2.0e7, 0, 0]}),
    );
    assert_eq!(far.unwrap_err().0, ErrorKind::InvalidInput);
}

#[test]
fn display_settings_default_on_and_are_view_settings() {
    let mut d = doc();
    let shown = ask(&d, json!({"type": "section_display"}));
    assert_eq!(
        (&shown["planes"], &shown["cuts"], &shown["fill"]),
        (&json!(true), &json!(true), &json!(true))
    );
    let revision = ask(&d, json!({"type": "stats"}))["revision"].clone();
    run(
        &mut d,
        json!({"type": "set_section_display", "cuts": false, "fill": false}),
    )
    .unwrap();
    let shown = ask(&d, json!({"type": "section_display"}));
    assert_eq!(
        (&shown["planes"], &shown["cuts"], &shown["fill"]),
        (&json!(true), &json!(false), &json!(false))
    );
    assert_eq!(undo_depth(&d), 0);
    assert_eq!(ask(&d, json!({"type": "stats"}))["revision"], revision);
    run(
        &mut d,
        json!({"type": "set_section_display", "planes": false}),
    )
    .unwrap();
    assert_eq!(ask(&d, json!({"type": "section_display"}))["planes"], false);
    assert!(
        Command::SetSectionDisplay {
            planes: None,
            cuts: None,
            fill: None,
            fill_color: None
        }
        .is_camera()
    );
}

#[test]
fn an_atomic_batch_rolls_the_display_back() {
    let mut d = doc();
    let commands = vec![
        BatchCommand::parse(json!({"type": "set_section_display", "cuts": false}), 0).unwrap(),
        BatchCommand::parse(json!({"type": "erase", "ids": ["e99"]}), 1).unwrap(),
    ];
    d.execute_batch(commands, true).unwrap_err();
    assert_eq!(ask(&d, json!({"type": "section_display"}))["cuts"], true);
}

#[test]
fn batch_references_name_section_planes() {
    let mut d = doc();
    let commands = vec![
        BatchCommand::parse(
            json!({"type": "add_section_plane", "point": [0, 0, 0], "normal": [0, 0, 1], "activate": false}),
            0,
        )
        .unwrap(),
        BatchCommand::parse(
            json!({"type": "flip_section_plane", "plane": "$0:s0"}),
            1,
        )
        .unwrap(),
        BatchCommand::parse(
            json!({"type": "activate_section_plane", "plane": "$0:s0"}),
            2,
        )
        .unwrap(),
        BatchCommand::parse(
            json!({"type": "move", "ids": ["$0:s*"], "offset": [0, 0, 1]}),
            3,
        )
        .unwrap(),
    ];
    d.execute_batch(commands, true).unwrap();
    let list = planes(&d);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["active"], true);
    near(&list[0]["normal"], [0.0, 0.0, -1.0]);
    near(&list[0]["point"], [0.0, 0.0, 1.0]);
    assert_eq!(undo_depth(&d), 1);
}

#[test]
fn files_without_planes_or_section_settings_keep_their_schema() {
    let mut d = doc();
    cube(&mut d);
    let path = scratch("plain.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&path)["schema_version"], 5);
    let text = stored(&path).to_string();
    assert!(!text.contains("section"), "{text}");
}

#[test]
fn planes_the_active_one_and_the_display_survive_save_and_open() {
    let mut d = doc();
    cube(&mut d);
    let a = plane(&mut d, [1.0, 1.0, 0.5], [0.0, 0.0, 1.0], true);
    let b = plane(&mut d, [0.0; 3], [1.0, 0.0, 0.0], false);
    run(
        &mut d,
        json!({"type": "set_section_display", "planes": false, "fill": false}),
    )
    .unwrap();
    let path = scratch("sections.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&path)["schema_version"], 13);

    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(planes(&e), planes(&d));
    assert_eq!(active(&e), vec![a.clone()]);
    assert_eq!(ask(&e, json!({"type": "entity", "id": b}))["active"], false);
    let shown = ask(&e, json!({"type": "section_display"}));
    assert_eq!(
        (&shown["planes"], &shown["cuts"], &shown["fill"]),
        (&json!(false), &json!(true), &json!(false))
    );
    // New starts with the defaults.
    run(&mut e, json!({"type": "new", "discard": true})).unwrap();
    assert_eq!(ask(&e, json!({"type": "section_display"}))["planes"], true);
}

#[test]
fn non_default_display_alone_writes_schema_13_and_defaults_are_not_written() {
    let mut d = doc();
    cube(&mut d);
    run(
        &mut d,
        json!({"type": "set_section_display", "cuts": false}),
    )
    .unwrap();
    let path = scratch("display.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let doc_json = stored(&path);
    assert_eq!(doc_json["schema_version"], 13);
    assert_eq!(doc_json["camera"]["section_cuts"], false);
    assert!(doc_json["camera"].get("section_planes").is_none());
    assert!(doc_json["camera"].get("section_fill").is_none());

    run(&mut d, json!({"type": "set_section_display", "cuts": true})).unwrap();
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap(), "overwrite": true}),
    )
    .unwrap();
    assert_eq!(stored(&path)["schema_version"], 5);
}

#[test]
fn a_schema_12_file_opens_with_no_planes_and_default_display() {
    let mut d = doc();
    cube(&mut d);
    run(
        &mut d,
        json!({"type": "add_note", "anchor": [0, 0, 0], "position": [1, 1, 1], "text": "a"}),
    )
    .unwrap();
    let path = scratch("twelve.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&path)["schema_version"], 12);
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert!(planes(&e).is_empty());
    assert_eq!(ask(&e, json!({"type": "section_display"}))["cuts"], true);

    // A newer schema is refused.
    edit_file(&path, |doc| doc["schema_version"] = json!(23));
    let err = run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap(), "discard": true}),
    )
    .unwrap_err();
    assert!(err.1.contains("newer"), "{}", err.1);
}

#[test]
fn a_file_with_a_plane_past_the_limit_or_a_bad_active_plane_does_not_open() {
    let mut d = doc();
    plane(&mut d, [1.0, 1.0, 1.0], [0.0, 0.0, 1.0], true);
    let path = scratch("bad.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    for (name, edit) in [("far.skr", 0), ("ghost.skr", 1)] {
        let copy = scratch(name);
        std::fs::copy(&path, &copy).unwrap();
        edit_file(&copy, |doc| {
            let root = &mut doc["model"]["root"];
            if edit == 0 {
                root["sections"][0]["point"] = json!([2e7, 0, 0]);
            } else {
                root["active_section"] = json!(9999);
            }
        });
        let mut e = doc();
        let (kind, ..) = run(
            &mut e,
            json!({"type": "open", "path": copy.to_str().unwrap()}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::File, "{name}");
    }
}

#[test]
fn exports_report_the_planes_they_leave_out() {
    let mut d = doc();
    cube(&mut d);
    for (count, said) in [
        (1, "1 section plane was not written"),
        (2, "2 section planes were not written"),
    ] {
        if count == 1 {
            plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], true);
        } else {
            plane(&mut d, [0.0; 3], [1.0, 0.0, 0.0], false);
        }
        for name in ["out.obj", "out.stl", "out.glb", "out.skp"] {
            let path = scratch(&format!("{count}-{name}"));
            let out = run(
                &mut d,
                json!({"type": "export", "path": path.to_str().unwrap()}),
            )
            .unwrap();
            let warnings = out["export"]["warnings"].as_array().unwrap();
            assert!(warnings.contains(&json!(said)), "{name}: {warnings:?}");
        }
    }
}

#[test]
fn a_model_without_planes_exports_no_plane_warning() {
    let mut d = doc();
    cube(&mut d);
    let path = scratch("none.obj");
    let out = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert!(!out["export"]["warnings"].to_string().contains("section"));
}

#[test]
fn flip_and_activate_work_inside_an_open_group() {
    let mut d = doc();
    let ids = cube(&mut d);
    let root_plane = plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], true);
    let out = run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let i = made(&out, 'i');
    run(&mut d, json!({"type": "open_instance", "id": i})).unwrap();
    let inner = plane(&mut d, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], false);
    let other = plane(&mut d, [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], true);
    run(
        &mut d,
        json!({"type": "activate_section_plane", "plane": inner}),
    )
    .unwrap();
    assert_eq!(active(&d), vec![inner.clone()], "one active plane per mesh");
    run(
        &mut d,
        json!({"type": "flip_section_plane", "plane": other}),
    )
    .unwrap();
    near(
        &ask(&d, json!({"type": "entity", "id": other}))["normal"],
        [0.0, -1.0, 0.0],
    );
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    assert_eq!(
        active(&d),
        vec![root_plane],
        "the root's plane is untouched"
    );
}

#[test]
fn rotating_a_plane_past_the_coordinate_limit_is_refused() {
    let mut d = doc();
    let a = plane(&mut d, [9.0e5, 0.0, 0.0], [1.0, 0.0, 0.0], false);
    let (kind, ..) = run(
        &mut d,
        json!({"type": "rotate", "ids": [a], "center": [-5.0e5, 0, 0], "angle": 90}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(planes(&d).len(), 1);
}

#[test]
fn geometry_and_a_plane_move_and_erase_together() {
    let mut d = doc();
    let mut ids = cube(&mut d);
    let a = plane(&mut d, [1.0, 1.0, 0.5], [0.0, 0.0, 1.0], true);
    ids.push(a.clone());
    run(
        &mut d,
        json!({"type": "move", "ids": ids, "offset": [0, 0, 3]}),
    )
    .unwrap();
    near(
        &ask(&d, json!({"type": "entity", "id": a}))["point"],
        [1.0, 1.0, 3.5],
    );
    let all = ask(&d, json!({"type": "entities"}));
    assert!(
        all["vertices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["position"][2].as_f64().unwrap() >= 3.0 - 1e-9)
    );
    run(&mut d, json!({"type": "erase", "ids": ids})).unwrap();
    assert!(planes(&d).is_empty());
    assert!(
        ask(&d, json!({"type": "entities"}))["edges"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(planes(&d).len(), 1);
}

#[test]
fn a_component_with_a_plane_placed_twice_counts_it_once_in_export_warnings() {
    let mut d = doc();
    let mut ids = cube(&mut d);
    ids.push(plane(&mut d, [1.0, 1.0, 0.5], [0.0, 0.0, 1.0], true));
    let out = run(
        &mut d,
        json!({"type": "make_component", "ids": ids, "name": "Box"}),
    )
    .unwrap();
    let i = made(&out, 'i');
    run(
        &mut d,
        json!({"type": "copy", "ids": [i], "offset": [5, 0, 0]}),
    )
    .unwrap();
    let path = scratch("twice.obj");
    let out = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let warnings = out["export"]["warnings"].as_array().unwrap();
    assert!(
        warnings.contains(&json!("1 section plane was not written")),
        "{warnings:?}"
    );
}

#[test]
fn select_all_skips_planes_while_they_are_not_shown() {
    let mut d = doc();
    let a = plane(&mut d, [0.0; 3], [0.0, 0.0, 1.0], true);
    run(
        &mut d,
        json!({"type": "set_section_display", "planes": false}),
    )
    .unwrap();
    run(&mut d, json!({"type": "select", "ids": [], "all": true})).unwrap();
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([]));
    run(
        &mut d,
        json!({"type": "set_section_display", "planes": true}),
    )
    .unwrap();
    run(&mut d, json!({"type": "select", "ids": [], "all": true})).unwrap();
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([a]));
}

/// A cube made a group. Returns the group's id.
fn grouped_cube(d: &mut Document) -> String {
    let ids = cube(d);
    let out = run(d, json!({"type": "make_group", "ids": ids})).unwrap();
    made(&out, 'i')
}

fn instance_entry(d: &Document, id: &str) -> Value {
    ask(d, json!({"type": "entities"}))["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == id)
        .unwrap()
        .clone()
}

#[test]
fn section_cut_is_on_by_default_and_absent_from_the_entities_query() {
    let mut d = doc();
    let g = grouped_cube(&mut d);
    assert!(instance_entry(&d, &g).get("section_cut").is_none());
    let out = run(
        &mut d,
        json!({"type": "set_section_cut", "ids": [g], "cut": false}),
    )
    .unwrap();
    assert_eq!(out["changed"], json!([g]));
    assert_eq!(instance_entry(&d, &g)["section_cut"], false);
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": g}))["section_cut"],
        false
    );
    run(
        &mut d,
        json!({"type": "set_section_cut", "ids": [g], "cut": true}),
    )
    .unwrap();
    assert!(instance_entry(&d, &g).get("section_cut").is_none());
}

#[test]
fn set_section_cut_is_one_undo_step_and_skips_instances_already_as_asked() {
    let mut d = doc();
    let g = grouped_cube(&mut d);
    let depth = undo_depth(&d);
    run(
        &mut d,
        json!({"type": "set_section_cut", "ids": [g], "cut": false}),
    )
    .unwrap();
    assert_eq!(undo_depth(&d), depth + 1);
    let out = run(
        &mut d,
        json!({"type": "set_section_cut", "ids": [g], "cut": false}),
    )
    .unwrap();
    assert_eq!(out["changed"], json!([]));
    assert_eq!(undo_depth(&d), depth + 1, "nothing changed, no step");
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(instance_entry(&d, &g).get("section_cut").is_none());
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(instance_entry(&d, &g)["section_cut"], false);
}

#[test]
fn set_section_cut_refuses_what_is_not_an_instance() {
    let mut d = doc();
    grouped_cube(&mut d);
    let plane = plane(&mut d, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], true);
    let (kind, ..) = run(
        &mut d,
        json!({"type": "set_section_cut", "ids": [plane], "cut": false}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, ..) = run(
        &mut d,
        json!({"type": "set_section_cut", "ids": [], "cut": false}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}

#[test]
fn fill_color_is_a_display_setting_saved_only_when_not_dark() {
    let mut d = doc();
    cube(&mut d);
    assert_eq!(
        ask(&d, json!({"type": "section_display"}))["fill_color"],
        "dark"
    );
    let depth = undo_depth(&d);
    run(
        &mut d,
        json!({"type": "set_section_display", "fill_color": "material"}),
    )
    .unwrap();
    assert_eq!(undo_depth(&d), depth, "a view setting, not an undo step");
    assert_eq!(
        ask(&d, json!({"type": "section_display"}))["fill_color"],
        "material"
    );
    let path = scratch("fill-color.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let doc_json = stored(&path);
    assert_eq!(doc_json["schema_version"], 22);
    assert_eq!(doc_json["camera"]["section_fill_color"], "material");
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(
        ask(&e, json!({"type": "section_display"}))["fill_color"],
        "material"
    );

    run(
        &mut d,
        json!({"type": "set_section_display", "fill_color": "dark"}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap(), "overwrite": true}),
    )
    .unwrap();
    let doc_json = stored(&path);
    assert!(doc_json["schema_version"].as_u64().unwrap() < 22);
    assert!(doc_json["camera"].get("section_fill_color").is_none());
    // New starts dark.
    run(&mut e, json!({"type": "new", "discard": true})).unwrap();
    assert_eq!(
        ask(&e, json!({"type": "section_display"}))["fill_color"],
        "dark"
    );
}

#[test]
fn an_instance_left_whole_writes_schema_22_and_round_trips() {
    let mut d = doc();
    let g = grouped_cube(&mut d);
    let path = scratch("whole.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let before = stored(&path);
    assert!(before["schema_version"].as_u64().unwrap() < 22);
    assert!(
        before["model"]["root"]["instances"][0]
            .get("section_cut")
            .is_none()
    );

    run(
        &mut d,
        json!({"type": "set_section_cut", "ids": [g], "cut": false}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap(), "overwrite": true}),
    )
    .unwrap();
    let after = stored(&path);
    assert_eq!(after["schema_version"], 22);
    assert_eq!(after["model"]["root"]["instances"][0]["section_cut"], false);
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(instance_entry(&e, &g)["section_cut"], false);

    // An old file has no field: every instance is cut.
    edit_file(&path, |doc| {
        doc["schema_version"] = json!(21);
        doc["model"]["root"]["instances"][0]
            .as_object_mut()
            .unwrap()
            .remove("section_cut");
    });
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap(), "discard": true}),
    )
    .unwrap();
    assert!(instance_entry(&e, &g).get("section_cut").is_none());
}

#[test]
fn a_group_left_whole_stays_whole_in_the_line_drawing_while_a_sibling_is_cut() {
    use session::drawing::line_drawing;
    let mut d = doc();
    let a = grouped_cube(&mut d);
    // A second cube beside it, grouped too.
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 5, 0], "opposite": [2, 7, 0]}),
    )
    .unwrap();
    let face = made(&out, 'f');
    run(
        &mut d,
        json!({"type": "push_pull", "face": face, "distance": 1}),
    )
    .unwrap();
    let all = ask(&d, json!({"type": "entities"}));
    let ids: Vec<Value> = all["edges"]
        .as_array()
        .unwrap()
        .iter()
        .chain(all["faces"].as_array().unwrap())
        .map(|e| e["id"].clone())
        .collect();
    let out = run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let b = made(&out, 'i');
    plane(&mut d, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], true);
    let extent = |d: &Document| -> (f64, f64) {
        let pts = session::drawing::extent_points(d);
        let x = |sel: fn(f64, f64) -> f64, y_lo: f64, y_hi: f64| {
            pts.iter()
                .filter(|p| p.y >= y_lo && p.y <= y_hi)
                .map(|p| p.x)
                .fold(f64::NAN, sel)
        };
        (x(f64::min, -1.0, 3.0), x(f64::min, 4.0, 8.0))
    };
    let (cut, other) = extent(&d);
    assert!(
        (cut - 1.0).abs() < 1e-6 && (other - 1.0).abs() < 1e-6,
        "{cut} {other}"
    );
    run(
        &mut d,
        json!({"type": "set_section_cut", "ids": [b], "cut": false}),
    )
    .unwrap();
    let (cut, other) = extent(&d);
    assert!((cut - 1.0).abs() < 1e-6, "{a} is still cut: {cut}");
    assert!(other.abs() < 1e-6, "{b} is whole: {other}");
    assert!(line_drawing(&d, &d.camera, d.camera.viewport).is_ok());
}
