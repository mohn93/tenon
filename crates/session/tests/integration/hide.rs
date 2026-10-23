//! Hide and unhide, driven through JSON commands and queries, and kept in files.

use std::io::Write;
use std::path::PathBuf;

use glam::DVec2;
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
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-hide-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

fn raw(path: &PathBuf) -> String {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("document.json").unwrap(), &mut text).unwrap();
    text
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

fn created(out: &Value, prefix: char) -> Vec<Value> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with(prefix))
        .cloned()
        .collect()
}

/// A 2 x 2 square: returns its face and edges.
fn square(d: &mut Document) -> (Value, Vec<Value>) {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    (created(&out, 'f')[0].clone(), created(&out, 'e'))
}

fn hidden_ids(d: &Document, kind: &str) -> Vec<Value> {
    let key = format!("{kind}s");
    ask(d, json!({"type": "entities", "kinds": [kind]}))[key.as_str()]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["hidden"] == true)
        .map(|e| e["id"].clone())
        .collect()
}

fn undo_depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

#[test]
fn hide_hides_edges_and_faces_independently_and_undo_restores() {
    let mut d = doc();
    let (face, edges) = square(&mut d);
    let out = run(&mut d, json!({"type": "hide", "ids": [face, edges[0]]})).unwrap();
    assert_eq!(out["changed"], json!([edges[0], face]));
    assert_eq!(hidden_ids(&d, "face"), vec![face.clone()]);
    assert_eq!(hidden_ids(&d, "edge"), vec![edges[0].clone()]);
    assert_eq!(
        ask(&d, json!({"type": "history"}))["undo"][0],
        json!("hide")
    );
    // The face is still a face, bounded by all four edges.
    assert_eq!(
        ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(hidden_ids(&d, "face").is_empty() && hidden_ids(&d, "edge").is_empty());
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(hidden_ids(&d, "face"), vec![face]);
}

#[test]
fn doing_nothing_is_a_success_without_an_undo_step() {
    let mut d = doc();
    let (face, _) = square(&mut d);
    let depth = undo_depth(&d);
    // Nothing is hidden yet, so each unhide does nothing; a second hide of the same face
    // (below) does nothing too.
    for c in [
        json!({"type": "unhide", "ids": [face]}),
        json!({"type": "unhide_last"}),
        json!({"type": "unhide_all"}),
    ] {
        let out = run(&mut d, c).unwrap();
        assert_eq!(out["changed"], json!([]), "{out}");
    }
    assert_eq!(undo_depth(&d), depth);
    run(&mut d, json!({"type": "hide", "ids": [face]})).unwrap();
    assert_eq!(undo_depth(&d), depth + 1);
    let out = run(&mut d, json!({"type": "hide", "ids": [face]})).unwrap();
    assert_eq!(out["changed"], json!([]));
    assert_eq!(undo_depth(&d), depth + 1);
}

#[test]
fn vertices_and_unknown_ids_are_refused() {
    let mut d = doc();
    let (_, _) = square(&mut d);
    let v = ask(&d, json!({"type": "entities", "kinds": ["vertex"]}))["vertices"][0]["id"].clone();
    let depth = undo_depth(&d);
    let err = run(&mut d, json!({"type": "hide", "ids": [v]})).unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
    assert_eq!(undo_depth(&d), depth);
    let err = run(&mut d, json!({"type": "hide", "ids": ["e999999"]})).unwrap_err();
    assert_eq!(err.0, ErrorKind::NotFound);
}

#[test]
fn hiding_removes_the_entities_from_the_selection() {
    let mut d = doc();
    let (face, edges) = square(&mut d);
    run(
        &mut d,
        json!({"type": "select", "ids": [face, edges[0], edges[1]]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "hide", "ids": [face, edges[0]]})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "selection"}))["ids"],
        json!([edges[1]])
    );
    // Undoing the hide does not bring the selection back, but unhide selected works on ids.
    run(&mut d, json!({"type": "unhide", "ids": [face]})).unwrap();
    assert_eq!(hidden_ids(&d, "edge"), vec![edges[0].clone()]);
}

#[test]
fn unhide_all_and_last() {
    let mut d = doc();
    let (face, edges) = square(&mut d);
    run(&mut d, json!({"type": "hide", "ids": [edges[0]]})).unwrap();
    run(&mut d, json!({"type": "hide", "ids": [face, edges[1]]})).unwrap();
    // Last is the most recent hide only.
    let out = run(&mut d, json!({"type": "unhide_last"})).unwrap();
    assert_eq!(out["changed"], json!([edges[1], face]));
    assert_eq!(hidden_ids(&d, "edge"), vec![edges[0].clone()]);
    assert_eq!(
        ask(&d, json!({"type": "history"}))["undo"][0],
        json!("unhide_last")
    );
    // Entities already shown by hand are skipped.
    run(&mut d, json!({"type": "hide", "ids": [face, edges[1]]})).unwrap();
    run(&mut d, json!({"type": "unhide", "ids": [face]})).unwrap();
    let out = run(&mut d, json!({"type": "unhide_last"})).unwrap();
    assert_eq!(out["changed"], json!([edges[1]]));
    // Undo and redo of the hide keep it usable.
    run(&mut d, json!({"type": "undo", "count": 2})).unwrap();
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(hidden_ids(&d, "edge").len(), 2);
    let out = run(&mut d, json!({"type": "unhide_all"})).unwrap();
    assert_eq!(out["changed"].as_array().unwrap().len(), 2);
    assert!(hidden_ids(&d, "edge").is_empty());
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(hidden_ids(&d, "edge").len(), 2);
}

#[test]
fn unhide_last_forgets_on_new_and_open() {
    let mut d = doc();
    let (face, _) = square(&mut d);
    run(&mut d, json!({"type": "hide", "ids": [face]})).unwrap();
    let path = scratch("last.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "open", "path": path.to_str().unwrap(), "discard": true}),
    )
    .unwrap();
    assert_eq!(hidden_ids(&d, "face"), vec![face]);
    let depth = undo_depth(&d);
    run(&mut d, json!({"type": "unhide_last"})).unwrap();
    assert_eq!(undo_depth(&d), depth);
    assert_eq!(hidden_ids(&d, "face").len(), 1);
}

#[test]
fn hide_works_on_the_open_context_and_unhide_all_stays_inside_it() {
    let mut d = doc();
    let (face, edges) = square(&mut d);
    let mut ids = edges.clone();
    ids.push(face.clone());
    let out = run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = created(&out, 'i')[0].clone();
    // A second square at the root.
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [5, 0, 0], "opposite": [7, 2, 0]}),
    )
    .unwrap();
    let root_face = created(&out, 'f')[0].clone();
    run(&mut d, json!({"type": "hide", "ids": [group, root_face]})).unwrap();
    assert_eq!(hidden_ids(&d, "instance"), vec![group.clone()]);
    // The root cannot hide what is inside the group.
    let only_root_faces = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .find(|id| *id != root_face)
        .is_none();
    assert!(only_root_faces);

    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    let inner = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(&mut d, json!({"type": "hide", "ids": [inner]})).unwrap();
    assert_eq!(hidden_ids(&d, "face"), vec![inner.clone()]);
    // Unhide all inside the group leaves the root's hidden entities alone, and last (made in
    // the group) shows nothing outside it.
    run(&mut d, json!({"type": "unhide_all"})).unwrap();
    assert!(hidden_ids(&d, "face").is_empty());
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    assert_eq!(hidden_ids(&d, "instance"), vec![group.clone()]);
    assert_eq!(hidden_ids(&d, "face"), vec![root_face]);
    let depth = undo_depth(&d);
    run(&mut d, json!({"type": "unhide_last"})).unwrap();
    // The last hide was in the group, where nothing it hid is still hidden.
    assert_eq!(undo_depth(&d), depth);
    run(&mut d, json!({"type": "unhide_all"})).unwrap();
    assert!(hidden_ids(&d, "instance").is_empty());
}

#[test]
fn guides_and_the_outliner_report_hidden() {
    let mut d = doc();
    let (face, edges) = square(&mut d);
    let mut ids = edges;
    ids.push(face);
    let out = run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = created(&out, 'i')[0].clone();
    let out = run(
        &mut d,
        json!({"type": "add_guide_point", "position": [1, 1, 1]}),
    )
    .unwrap();
    let guide = created(&out, 'g')[0].clone();
    let tree = ask(&d, json!({"type": "outliner"}));
    assert!(tree["instances"][0].get("hidden").is_none());
    run(&mut d, json!({"type": "hide", "ids": [group]})).unwrap();
    let tree = ask(&d, json!({"type": "outliner"}));
    assert_eq!(tree["instances"][0]["hidden"], true);
    assert_eq!(tree["instances"][0]["own_hidden"], true);
    // A hidden tag hides without the own flag.
    run(&mut d, json!({"type": "unhide_all"})).unwrap();
    run(&mut d, json!({"type": "add_tag", "name": "T"})).unwrap();
    let tag = ask(&d, json!({"type": "tags"}))["tags"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["id"]
        .clone();
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [group], "tag": tag}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "change_tag", "tag": tag, "visible": false}),
    )
    .unwrap();
    let tree = ask(&d, json!({"type": "outliner"}));
    assert_eq!(tree["instances"][0]["hidden"], true);
    assert!(tree["instances"][0].get("own_hidden").is_none());
    run(&mut d, json!({"type": "hide", "ids": [guide]})).unwrap();
    assert_eq!(hidden_ids(&d, "guide"), vec![guide]);
}

#[test]
fn the_skr_round_trips_hidden_and_schema_14_is_only_written_when_needed() {
    let mut d = doc();
    let (face, edges) = square(&mut d);
    let plain = scratch("plain.skr");
    run(
        &mut d,
        json!({"type": "save", "path": plain.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&plain)["schema_version"], 5);
    assert!(!raw(&plain).contains("hidden"));

    run(&mut d, json!({"type": "hide", "ids": [face, edges[2]]})).unwrap();
    let path = scratch("hidden.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&path)["schema_version"], 14);

    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(hidden_ids(&e, "face"), vec![face.clone()]);
    assert_eq!(hidden_ids(&e, "edge"), vec![edges[2].clone()]);

    // Showing everything again writes the old schema.
    run(&mut d, json!({"type": "unhide_all"})).unwrap();
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap(), "overwrite": true}),
    )
    .unwrap();
    assert_eq!(stored(&path)["schema_version"], 5);

    // A file from a newer build is refused.
    edit_file(&path, |doc| doc["schema_version"] = json!(15));
    let err = run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap(), "discard": true}),
    )
    .unwrap_err();
    assert!(err.1.contains("newer"), "{}", err.1);
}

#[test]
fn schema_13_is_written_as_before_when_nothing_is_hidden() {
    let mut d = doc();
    square(&mut d);
    run(
        &mut d,
        json!({"type": "add_section_plane", "point": [1, 1, 0], "normal": [1, 0, 0]}),
    )
    .unwrap();
    let path = scratch("sectioned.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&path)["schema_version"], 13);
    assert!(!raw(&path).contains("hidden"));
}

#[test]
fn a_skp_export_keeps_hidden_flags_and_the_import_reads_them_back() {
    let mut d = doc();
    let (face, edges) = square(&mut d);
    let mut ids = edges.clone();
    ids.push(face.clone());
    let out = run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let group = created(&out, 'i')[0].clone();
    // Root geometry beside the group: a second square, a loose edge, two guides.
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [5, 0, 0], "opposite": [7, 2, 0]}),
    )
    .unwrap();
    let root_face = created(&out, 'f')[0].clone();
    let root_edge = created(&out, 'e')[0].clone();
    let out = run(
        &mut d,
        json!({"type": "add_guide_point", "position": [1, 1, 1]}),
    )
    .unwrap();
    let hidden_guide = created(&out, 'g')[0].clone();
    run(
        &mut d,
        json!({"type": "add_guide_line", "point": [0, 0, 0], "direction": [1, 0, 0]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "hide", "ids": [group, root_face, root_edge, hidden_guide]}),
    )
    .unwrap();
    let path = scratch("hidden.skp");
    run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap(), "format": "skp"}),
    )
    .unwrap();

    let mut e = doc();
    let out = run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert!(out.get("import").is_some());
    assert_eq!(hidden_ids(&e, "instance").len(), 1);
    assert_eq!(hidden_ids(&e, "face").len(), 1);
    assert_eq!(hidden_ids(&e, "edge").len(), 1);
    assert_eq!(hidden_ids(&e, "guide").len(), 1);
    // Hidden edges are hidden, not soft.
    let edges = ask(&e, json!({"type": "entities", "kinds": ["edge"]}));
    assert!(
        edges["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x.get("soft").is_none())
    );
    assert_eq!(
        ask(&e, json!({"type": "entities", "kinds": ["guide"]}))["guides"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
