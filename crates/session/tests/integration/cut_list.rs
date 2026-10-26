//! The `cut_list` query, driven through JSON.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Value {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    serde_json::to_value(d.execute(cmd).unwrap()).unwrap()
}

fn ask(d: &Document, v: Value) -> Value {
    let q: Query = serde_json::from_value(v).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn ask_err(d: &Document, v: Value) -> ErrorKind {
    let q: Query = serde_json::from_value(v).unwrap();
    d.query(&q).unwrap_err().kind
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn size(row: &Value) -> [f64; 3] {
    ["length", "width", "thickness"].map(|k| f(&row[k]))
}

fn close(a: [f64; 3], b: [f64; 3]) {
    for i in 0..3 {
        assert!((a[i] - b[i]).abs() < 1e-9, "{a:?} vs {b:?}");
    }
}

/// A 1 x 0.1 x 0.02 m component named Board at the origin.
fn board(d: &mut Document) -> Value {
    run(
        d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1.0, 0.1, 0.02],
               "as": "component", "name": "Board"}),
    )["placed"][0]
        .clone()
}

fn top(d: &mut Document) -> Value {
    run(
        d,
        json!({"type": "add_box", "corner": [0, 1, 0], "size": [0.6, 1.2, 0.02],
               "as": "group", "name": "Top"}),
    )["placed"][0]
        .clone()
}

fn rows(d: &Document, q: Value) -> Vec<Value> {
    ask(d, q)["rows"].as_array().unwrap().clone()
}

#[test]
fn three_boards_of_one_component_and_a_top_are_two_rows() {
    let mut d = doc();
    let b = board(&mut d);
    run(
        &mut d,
        json!({"type": "copy", "ids": [b], "offset": [0, 0.5, 0], "count": 2}),
    );
    let t = top(&mut d);
    let before = ask(&d, json!({"type": "history"}));
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 2);
    assert_eq!(r[0]["name"], "Top");
    assert_eq!(r[0]["quantity"], 1);
    close(size(&r[0]), [1.2, 0.6, 0.02]);
    assert_eq!(r[0]["ids"], json!([t]));
    assert_eq!(r[1]["name"], "Board");
    assert_eq!(r[1]["quantity"], 3);
    close(size(&r[1]), [1.0, 0.1, 0.02]);
    assert_eq!(r[1]["ids"].as_array().unwrap().len(), 3);
    assert_eq!(r[1]["solid"], true);
    assert!(r[1].get("note").is_none());
    let none = rows(&d, json!({"type": "cut_list", "group_by": "none"}));
    assert_eq!(none.len(), 4);
    assert!(none.iter().all(|row| row["quantity"] == 1));
    assert_eq!(ask(&d, json!({"type": "history"})), before, "a query only");
}

#[test]
fn a_board_turned_thirty_degrees_keeps_its_true_size() {
    let mut d = doc();
    let b = board(&mut d);
    run(
        &mut d,
        json!({"type": "rotate", "ids": [b], "center": [0, 0, 0], "angle": 30}),
    );
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 1);
    close(size(&r[0]), [1.0, 0.1, 0.02]);
}

#[test]
fn a_sheared_board_reads_the_smallest_box_around_it() {
    let mut d = doc();
    let b = board(&mut d);
    run(
        &mut d,
        json!({"type": "rotate", "ids": [b], "center": [0, 0, 0], "angle": 30}),
    );
    run(
        &mut d,
        json!({"type": "scale", "ids": [b], "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    );
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 1);
    let s = size(&r[0]);
    for (got, want) in s.iter().zip([1.874_86, 0.110_94, 0.02]) {
        assert!((got - want).abs() < 1e-4, "{s:?}");
    }
}

#[test]
fn a_scaled_instance_is_its_own_row() {
    let mut d = doc();
    let b = board(&mut d);
    let copy = run(
        &mut d,
        json!({"type": "copy", "ids": [b], "offset": [0, 0.5, 0]}),
    )["placed"][0]
        .clone();
    run(
        &mut d,
        json!({"type": "scale", "ids": [copy], "origin": [0, 0, 0], "factors": [1, 1, 2]}),
    );
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 2);
    assert!(r.iter().all(|row| row["quantity"] == 1));
    let sizes: Vec<[f64; 3]> = r.iter().map(size).collect();
    close(sizes[0], [1.0, 0.1, 0.04]);
    close(sizes[1], [1.0, 0.1, 0.02]);
}

#[test]
fn boards_that_differ_by_float_noise_on_a_rounding_boundary_are_one_row() {
    // 23/32 inch plywood is 182562.5 steps of 0.1 micrometer: noise either side of it must
    // not split identical boards into two rows.
    let mut d = doc();
    let b = run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1.0, 0.1, 0.01825625],
               "as": "component", "name": "Ply"}),
    )["placed"][0]
        .clone();
    let copies = run(
        &mut d,
        json!({"type": "copy", "ids": [b], "offset": [0, 0.5, 0], "count": 2}),
    )["placed"]
        .as_array()
        .unwrap()
        .clone();
    for (copy, z) in copies.iter().zip([1.0 + 1e-12, 1.0 - 1e-12]) {
        run(
            &mut d,
            json!({"type": "scale", "ids": [copy], "origin": [0, 0, 0], "factors": [1, 1, z]}),
        );
    }
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0]["quantity"], 3);
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

#[test]
fn hidden_geometry_inside_a_part_does_not_count_in_its_size() {
    let mut d = doc();
    let b = board(&mut d);
    run(&mut d, json!({"type": "open_instance", "id": b}));
    let out = run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0.02], "end": [0, 0, 5]}),
    );
    let edge = created(&out, 'e')[0].clone();
    run(&mut d, json!({"type": "close_instance"}));
    close(
        size(&rows(&d, json!({"type": "cut_list"}))[0]),
        [5.0, 1.0, 0.1],
    );
    run(&mut d, json!({"type": "open_instance", "id": b}));
    run(&mut d, json!({"type": "hide", "ids": [edge]}));
    run(&mut d, json!({"type": "close_instance"}));
    close(
        size(&rows(&d, json!({"type": "cut_list"}))[0]),
        [1.0, 0.1, 0.02],
    );
}

#[test]
fn a_part_whose_faces_are_all_hidden_is_not_listed() {
    let mut d = doc();
    let b = board(&mut d);
    top(&mut d);
    run(&mut d, json!({"type": "open_instance", "id": b}));
    let faces: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .collect();
    assert_eq!(faces.len(), 6);
    run(&mut d, json!({"type": "hide", "ids": faces[..5]}));
    run(&mut d, json!({"type": "close_instance"}));
    assert_eq!(
        rows(&d, json!({"type": "cut_list"})).len(),
        2,
        "one face still shows"
    );
    run(&mut d, json!({"type": "open_instance", "id": b}));
    run(&mut d, json!({"type": "hide", "ids": [faces[5]]}));
    run(&mut d, json!({"type": "close_instance"}));
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 1);
    assert_eq!(r[0]["name"], "Top");
}

#[test]
fn a_cabinet_lists_its_parts_not_itself() {
    let mut d = doc();
    let side = board(&mut d);
    let copy = run(
        &mut d,
        json!({"type": "copy", "ids": [side], "offset": [0, 0.5, 0]}),
    )["placed"][0]
        .clone();
    let shelf = top(&mut d);
    let cabinet = run(
        &mut d,
        json!({"type": "make_group", "ids": [side, copy, shelf], "name": "Cabinet"}),
    )["placed"][0]
        .clone();
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 2, "{r:?}");
    assert!(r.iter().all(|row| row["name"] != "Cabinet"));
    assert_eq!(r[1]["quantity"], 2);
    assert_eq!(
        r[1]["ids"],
        json!([cabinet]),
        "the cabinet is what is selected"
    );
    // Limiting the ids to another part leaves the cabinet out.
    let other = board(&mut d);
    let only = rows(&d, json!({"type": "cut_list", "ids": [other]}));
    assert_eq!(only.len(), 1);
}

#[test]
fn a_part_that_is_not_solid_is_listed_with_a_note() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [0.5, 0.25, 0]}),
    );
    let faces = ask(&d, json!({"type": "entities"}));
    let ids: Vec<Value> = faces["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .collect();
    let edges: Vec<Value> = faces["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].clone())
        .collect();
    let all = [ids, edges].concat();
    run(
        &mut d,
        json!({"type": "make_group", "ids": all, "name": "Sheet"}),
    );
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 1);
    assert_eq!(r[0]["name"], "Sheet");
    assert_eq!(r[0]["solid"], false);
    assert!(r[0]["note"].as_str().unwrap().contains("bounds 1 face"));
    close(size(&r[0]), [0.5, 0.25, 0.0]);
}

#[test]
fn rows_are_ordered_by_material_then_size_and_name_the_material() {
    let mut d = doc();
    let oak = run(
        &mut d,
        json!({"type": "add_material", "name": "Oak", "color": [200, 160, 100]}),
    )["changes"]["materials"][0]
        .clone();
    let ash = run(
        &mut d,
        json!({"type": "add_material", "name": "ash", "color": [230, 220, 200]}),
    )["changes"]["materials"][0]
        .clone();
    let mk = |d: &mut Document, name: &str, size: [f64; 3], material: Option<&Value>| {
        let mut c = json!({"type": "add_box", "corner": [0, 0, 0], "size": size,
                           "as": "group", "name": name});
        if let Some(m) = material {
            c["material"] = m.clone();
        }
        run(d, c);
    };
    mk(&mut d, "Plain", [3.0, 1.0, 0.1], None);
    mk(&mut d, "Leg", [0.1, 0.1, 0.7], Some(&oak));
    mk(&mut d, "Rail", [0.9, 0.1, 0.05], Some(&oak));
    mk(&mut d, "Back", [0.8, 0.5, 0.01], Some(&ash));
    let r = rows(&d, json!({"type": "cut_list"}));
    let names: Vec<&str> = r.iter().map(|row| row["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Back", "Rail", "Leg", "Plain"]);
    assert_eq!(r[0]["material"], "ash");
    assert_eq!(r[1]["material"], "Oak");
    assert!(r[3].get("material").is_none());
    close(size(&r[2]), [0.7, 0.1, 0.1]);
}

#[test]
fn bad_ids_are_refused() {
    let d = doc();
    assert_eq!(
        ask_err(&d, json!({"type": "cut_list", "ids": ["f1"]})),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask_err(&d, json!({"type": "cut_list", "ids": ["i999"]})),
        ErrorKind::NotFound
    );
    assert_eq!(rows(&d, json!({"type": "cut_list"})), Vec::<Value>::new());
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-cutlist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn the_csv_export_escapes_names_and_follows_the_display_units() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1.2, 0.0254, 0.019],
               "as": "group", "name": "Rail, \"long\""}),
    );
    run(
        &mut d,
        json!({"type": "set_units", "length": "millimeters", "precision": 1}),
    );
    let path = scratch("list.csv");
    let out = run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    );
    assert_eq!(out["export"]["triangles"], 0);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        text,
        "Name,Material,Length (mm),Width (mm),Thickness (mm),Quantity,Solid,Note\r\n\
         \"Rail, \"\"long\"\"\",,1200,25.4,19,1,yes,\r\n"
    );
    // Another unit changes the header and the numbers; the same file may be replaced.
    run(
        &mut d,
        json!({"type": "set_units", "length": "inches", "precision": 2}),
    );
    run(
        &mut d,
        json!({"type": "export", "path": path.to_str().unwrap()}),
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("Name,Material,Length (in),Width (in),Thickness (in),"));
    assert!(text.contains(",47.24,1,0.75,1,yes,"), "{text}");
    // A file that is not a cut list is not replaced without `overwrite`.
    std::fs::write(&path, "hello").unwrap();
    let cmd: Command =
        serde_json::from_value(json!({"type": "export", "path": path.to_str().unwrap()})).unwrap();
    assert!(d.execute(cmd).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
}

#[test]
fn the_open_contexts_cut_list_is_written_as_the_window_shows_it() {
    let mut d = doc();
    let shelf = top(&mut d);
    let cabinet = run(
        &mut d,
        json!({"type": "make_group", "ids": [shelf], "name": "Cabinet"}),
    )["placed"][0]
        .clone();
    board(&mut d);
    run(&mut d, json!({"type": "open_instance", "id": cabinet}));
    let text = d.cut_list_text().unwrap();
    assert!(text.contains("\r\nTop,"), "{text}");
    assert!(!text.contains("Board"), "only the open context: {text}");
    let path = scratch("context.csv");
    std::fs::write(&path, "not a cut list").unwrap();
    d.write_cut_list(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    // The export command still writes the whole model.
    let whole = scratch("whole.csv");
    run(
        &mut d,
        json!({"type": "export", "path": whole.to_str().unwrap()}),
    );
    assert!(std::fs::read_to_string(&whole).unwrap().contains("Board"));
}

#[test]
fn instances_that_differ_only_in_the_shape_of_their_placement_are_sized_apart() {
    // One board as drawn, one mirrored, one stretched then turned (no shear) and one turned
    // then stretched (sheared): the sizes found for one placement are not reused for another.
    let mut d = doc();
    let a = board(&mut d);
    let copy = |d: &mut Document, y: f64| {
        run(d, json!({"type": "copy", "ids": [a], "offset": [0, y, 0]}))["placed"][0].clone()
    };
    let mirrored = copy(&mut d, 1.0);
    run(
        &mut d,
        json!({"type": "scale", "ids": [mirrored], "origin": [0, 1, 0], "factors": [-1, 1, 1]}),
    );
    let plain = copy(&mut d, 2.0);
    run(
        &mut d,
        json!({"type": "scale", "ids": [plain], "origin": [0, 2, 0], "factors": [2, 1, 1]}),
    );
    run(
        &mut d,
        json!({"type": "rotate", "ids": [plain], "center": [0, 2, 0], "angle": 30}),
    );
    let sheared = copy(&mut d, 3.0);
    run(
        &mut d,
        json!({"type": "rotate", "ids": [sheared], "center": [0, 3, 0], "angle": 30}),
    );
    run(
        &mut d,
        json!({"type": "scale", "ids": [sheared], "origin": [0, 3, 0], "factors": [2, 1, 1]}),
    );
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 3, "{r:?}");
    close(size(&r[0]), [2.0, 0.1, 0.02]);
    assert_eq!(r[0]["ids"], json!([plain]));
    let s = size(&r[1]);
    for (got, want) in s.iter().zip([1.874_86, 0.110_94, 0.02]) {
        assert!((got - want).abs() < 1e-4, "{s:?}");
    }
    assert_eq!(r[1]["ids"], json!([sheared]));
    close(size(&r[2]), [1.0, 0.1, 0.02]);
    assert_eq!(r[2]["quantity"], 2);
}

#[test]
fn a_dowel_reads_its_diameter_not_the_width_across_its_flats() {
    // The sides of a circle are not taken for the sides of the part: a 24-sided cylinder
    // read along its flats would be 1.7% smaller than the circle drawn.
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 0.015, "height": 0.5,
               "segments": 24, "as": "component"}),
    );
    let r = rows(&d, json!({"type": "cut_list"}));
    assert_eq!(r.len(), 1);
    close(size(&r[0]), [0.5, 0.03, 0.03]);
}
