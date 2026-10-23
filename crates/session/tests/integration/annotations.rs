//! Dimensions, notes and display units, driven through JSON commands and queries, and kept in
//! files.

use std::io::Write;
use std::path::PathBuf;

use glam::DVec2;
use kernel::EntityId;
use serde_json::{Value, json};
use session::{
    BatchCommand, Command, Document, ErrorKind, LengthUnit, Query, Units, format_length,
};

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

fn entity(d: &Document, id: &str) -> Value {
    ask(d, json!({"type": "entity", "id": id}))
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-annotations-{}", std::process::id()));
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

/// The ids of kind `prefix` an outcome created.
fn created(out: &Value, prefix: char) -> Vec<String> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .filter(|s| s.starts_with(prefix))
        .collect()
}

/// The one id of kind `prefix` an outcome created.
fn made(out: &Value, prefix: char) -> String {
    let ids = created(out, prefix);
    assert_eq!(ids.len(), 1, "{out}");
    ids[0].clone()
}

/// A 2 x 1 rectangle on the ground: its face, edges and vertices, and the edge along y = 0.
struct Rect {
    face: String,
    edges: Vec<String>,
    bottom: String,
}

fn rectangle(d: &mut Document) -> Rect {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 1, 0]}),
    )
    .unwrap();
    let edges = created(&out, 'e');
    let bottom = edges
        .iter()
        .find(|e| {
            let info = entity(d, e);
            info["start"][1] == 0.0 && info["end"][1] == 0.0
        })
        .unwrap()
        .clone();
    Rect {
        face: made(&out, 'f'),
        edges,
        bottom,
    }
}

/// The vertex at `p`.
fn vertex_at(d: &Document, p: [f64; 3]) -> String {
    let all = ask(d, json!({"type": "entities", "kinds": ["vertex"]}));
    all["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["position"] == json!(p))
        .unwrap_or_else(|| panic!("no vertex at {p:?}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn units(length: LengthUnit, precision: u8) -> Units {
    Units {
        length,
        precision,
        ..Units::default()
    }
}

#[test]
fn lengths_are_written_in_every_unit_and_precision() {
    use LengthUnit::*;
    let table: &[(f64, Units, &str)] = &[
        // The default matches the Measurements box: 3 decimals, trailing zeros dropped.
        (1.5, Units::default(), "1.5 m"),
        (0.0, Units::default(), "0 m"),
        (-0.0001, Units::default(), "0 m"),
        (-2.25, Units::default(), "-2.25 m"),
        (1.23456, units(Meters, 0), "1 m"),
        (1.23456, units(Meters, 1), "1.2 m"),
        (1.23456, units(Meters, 2), "1.23 m"),
        (1.23456, units(Meters, 4), "1.2346 m"),
        (1.23456, units(Meters, 5), "1.23456 m"),
        (1.234567, units(Meters, 6), "1.234567 m"),
        (1.0, units(Meters, 6), "1 m"),
        (1.5, units(Centimeters, 0), "150 cm"),
        (0.12345, units(Centimeters, 2), "12.35 cm"),
        (-0.12345, units(Centimeters, 1), "-12.3 cm"),
        (0.0, units(Centimeters, 3), "0 cm"),
        (0.0123456, units(Millimeters, 3), "12.346 mm"),
        (0.0123456, units(Millimeters, 0), "12 mm"),
        (-0.0004, units(Millimeters, 0), "0 mm"),
        (0.0254, units(Inches, 2), "1\""),
        (0.3175, units(Inches, 3), "12.5\""),
        (-0.3175, units(Inches, 0), "-12\""),
        (0.0, units(Inches, 4), "0\""),
        (0.0, units(FeetInches, 4), "0\""),
        (-0.0001, units(FeetInches, 0), "0\""),
        (0.3048, units(FeetInches, 3), "1'"),
        (1.6002, units(FeetInches, 0), "5' 3\""),
        (1.6129, units(FeetInches, 1), "5' 3 1/2\""),
        (-1.6129, units(FeetInches, 1), "-5' 3 1/2\""),
        (0.0127, units(FeetInches, 6), "1/2\""),
        (0.0254 * 0.75, units(FeetInches, 2), "3/4\""),
        (0.0254 * (1.0 / 64.0), units(FeetInches, 6), "1/64\""),
        (0.0254 * 0.01, units(FeetInches, 5), "0\""),
        (0.0254 * 3.0625, units(FeetInches, 4), "3 1/16\""),
        // 11 63/64" is exact at 1/64 but rounds up to a foot at coarser fractions.
        (
            0.0254 * (11.0 + 63.0 / 64.0),
            units(FeetInches, 6),
            "11 63/64\"",
        ),
        (0.0254 * (11.0 + 63.0 / 64.0), units(FeetInches, 4), "1'"),
        (0.0254 * 11.999, units(FeetInches, 6), "1'"),
        (0.0254 * 23.999, units(FeetInches, 0), "2'"),
        (0.0254 * 12.5, units(FeetInches, 1), "1' 1/2\""),
    ];
    for (meters, units, expected) in table {
        assert_eq!(
            format_length(*meters, units),
            *expected,
            "{meters} m in {units:?}"
        );
    }
}

#[test]
fn dimensions_measure_between_vertices_points_and_edges() {
    let mut d = doc();
    let r = rectangle(&mut d);
    let v0 = vertex_at(&d, [0.0, 0.0, 0.0]);
    let v2 = vertex_at(&d, [2.0, 1.0, 0.0]);
    // Between two vertices, the offset kept perpendicular to them.
    let out = run(
        &mut d,
        json!({"type": "add_dimension", "a": v0, "b": [2, 0, 0], "offset": [1, -1, 0]}),
    )
    .unwrap();
    let r1 = made(&out, 'r');
    let info = entity(&d, &r1);
    assert_eq!(info["type"], "dimension");
    assert_eq!(info["kind"], "linear");
    assert_eq!(info["a"], json!({"point": [0.0, 0.0, 0.0], "vertex": v0}));
    assert_eq!(info["b"], json!({"point": [2.0, 0.0, 0.0]}));
    assert_eq!(info["offset"], json!([0.0, -1.0, 0.0]));
    assert_eq!(info["line"], json!([[0.0, -1.0, 0.0], [2.0, -1.0, 0.0]]));
    assert_eq!(info["value"], 2.0);
    assert_eq!(info["label"], "2 m");
    assert_eq!(info["tag"], Value::Null);

    // Along an edge: anchored to its two vertices.
    let out = run(
        &mut d,
        json!({"type": "add_edge_dimension", "edge": r.bottom, "offset": [0, -0.5, 0]}),
    )
    .unwrap();
    let r2 = made(&out, 'r');
    let info = entity(&d, &r2);
    assert!(info["a"]["vertex"].is_string() && info["b"]["vertex"].is_string());
    assert_eq!(info["label"], "2 m");

    // Diagonal between two vertices.
    let out = run(
        &mut d,
        json!({"type": "add_dimension", "a": v0, "b": v2, "offset": [0, 0, 1]}),
    )
    .unwrap();
    let diagonal = entity(&d, &made(&out, 'r'));
    assert_eq!(diagonal["label"], "2.236 m");

    // Each is one undo step.
    let history = ask(&d, json!({"type": "history"}));
    assert_eq!(
        history["undo"],
        json!([
            "add_dimension",
            "add_edge_dimension",
            "add_dimension",
            "add_rectangle"
        ])
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    let all = ask(&d, json!({"type": "annotations"}));
    assert_eq!(all["dimensions"].as_array().unwrap().len(), 2);
    assert_eq!(all["dimensions"][0]["id"], json!(r1));
    let _ = r.face;
}

#[test]
fn dimension_refusals_are_clear() {
    let mut d = doc();
    let r = rectangle(&mut d);
    let v0 = vertex_at(&d, [0.0, 0.0, 0.0]);
    let (kind, message, _) = run(
        &mut d,
        json!({"type": "add_dimension", "a": v0, "b": [0, 0, 0], "offset": [0, 1, 0]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("coincide"), "{message}");
    let (kind, _, ids) = run(
        &mut d,
        json!({"type": "add_dimension", "a": "v999", "b": [1, 0, 0], "offset": [0, 1, 0]}),
    )
    .unwrap_err();
    assert_eq!((kind, ids), (ErrorKind::NotFound, vec!["v999".to_string()]));
    let (kind, message, ids) = run(
        &mut d,
        json!({"type": "add_dimension", "a": r.face, "b": [1, 0, 0], "offset": [0, 1, 0]}),
    )
    .unwrap_err();
    assert_eq!((kind, ids), (ErrorKind::InvalidInput, vec![r.face.clone()]));
    assert!(message.contains("vertex id or a point"), "{message}");
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "add_dimension", "a": [0, 0, 0], "b": [2e6, 0, 0], "offset": [0, 1, 0]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    // The line, moved by the offset, must stay within the limit too.
    let (kind, message, _) = run(
        &mut d,
        json!({"type": "add_dimension", "a": [0, 9e5, 0], "b": [1, 9e5, 0], "offset": [0, 2e5, 0]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("within"), "{message}");
    // A radial dimension needs a circle or an arc.
    let (kind, message, ids) = run(
        &mut d,
        json!({"type": "add_radial_dimension", "edge": r.bottom, "leader": [1, 1, 0]}),
    )
    .unwrap_err();
    assert_eq!(
        (kind, ids),
        (ErrorKind::InvalidInput, vec![r.bottom.clone()])
    );
    assert!(message.contains("circle or an arc"), "{message}");
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "add_edge_dimension", "edge": r.face, "offset": [0, 1, 0]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "add_edge_dimension", "edge": "e999", "offset": [0, 1, 0]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
}

#[test]
fn radial_dimensions_follow_their_circle_and_label_radius_or_diameter() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [1, 1, 0], "radius": 1.5}),
    )
    .unwrap();
    let edge = created(&out, 'e')[0].clone();
    let face = made(&out, 'f');
    let out = run(
        &mut d,
        json!({"type": "add_radial_dimension", "edge": edge, "leader": [2, 0, 0]}),
    )
    .unwrap();
    let radius = made(&out, 'r');
    let out = run(
        &mut d,
        json!({"type": "add_radial_dimension", "edge": edge, "diameter": true, "leader": [0, 2, 0]}),
    )
    .unwrap();
    let diameter = made(&out, 'r');
    let info = entity(&d, &radius);
    assert_eq!(info["kind"], "radial");
    assert!(info["curve"].as_str().unwrap().starts_with('c'));
    assert_eq!(info["label"], "R 1.5 m");
    assert_eq!(entity(&d, &diameter)["label"], "Ø 3 m");
    assert_eq!(entity(&d, &diameter)["diameter"], true);

    // The circle is moved: the dimensions follow it.
    run(
        &mut d,
        json!({"type": "move", "ids": [face], "offset": [0, 0, 2]}),
    )
    .unwrap();
    let center = entity(&d, &radius)["center"].clone();
    for (c, want) in center.as_array().unwrap().iter().zip([1.0, 1.0, 2.0]) {
        assert!((c.as_f64().unwrap() - want).abs() < 1e-9, "{center}");
    }
}

#[test]
fn a_non_uniform_scale_of_a_circle_with_its_radial_dimension_scales_the_frozen_circle() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1}),
    )
    .unwrap();
    let edges = created(&out, 'e');
    let out = run(
        &mut d,
        json!({"type": "add_radial_dimension", "edge": edges[0], "leader": [2, 0, 0]}),
    )
    .unwrap();
    let r = made(&out, 'r');
    let mut ids: Vec<Value> = edges.iter().map(|e| json!(e)).collect();
    ids.push(json!(r));
    run(
        &mut d,
        json!({"type": "scale", "ids": ids, "origin": [0, 0, 0], "factors": [2, 1, 1]}),
    )
    .unwrap();
    // The ellipse is no circle: the dimension keeps the circle it had, scaled with it (same
    // area), and its leader is scaled too.
    let info = entity(&d, &r);
    assert!(info.get("curve").is_none(), "{info}");
    let radius = info["radius"].as_f64().unwrap();
    assert!((radius - 2f64.sqrt()).abs() < 1e-9, "{radius}");
    assert_eq!(info["leader"], json!([4.0, 0.0, 0.0]));
    // One undo step puts both back.
    run(&mut d, json!({"type": "undo"})).unwrap();
    let info = entity(&d, &r);
    assert!(info["curve"].is_string(), "{info}");
    assert_eq!(info["radius"], 1.0);
}

#[test]
fn notes_show_their_text_or_their_anchors_coordinates_in_the_display_units() {
    let mut d = doc();
    rectangle(&mut d);
    let v = vertex_at(&d, [2.0, 1.0, 0.0]);
    let out = run(
        &mut d,
        json!({"type": "add_note", "anchor": v, "position": [3, 2, 1]}),
    )
    .unwrap();
    let auto = made(&out, 'n');
    let info = entity(&d, &auto);
    assert_eq!(info["type"], "note");
    assert_eq!(info["kind"], "leader");
    assert!(info.get("text").is_none());
    assert_eq!(info["label"], "(2 m, 1 m, 0 m)");
    assert_eq!(info["anchor"]["vertex"], json!(v));
    assert_eq!(info["position"], json!([3.0, 2.0, 1.0]));

    let out = run(
        &mut d,
        json!({"type": "add_note", "anchor": [0, 0, 0], "position": [-1, -1, 0], "text": "Door"}),
    )
    .unwrap();
    let door = made(&out, 'n');
    assert_eq!(entity(&d, &door)["label"], "Door");

    let out = run(
        &mut d,
        json!({"type": "add_screen_note", "at": [0.1, 0.9], "text": "Plan A\nsecond line"}),
    )
    .unwrap();
    let screen = made(&out, 'n');
    let info = entity(&d, &screen);
    assert_eq!(info["kind"], "screen");
    assert_eq!(info["at"], json!([0.1, 0.9]));
    assert_eq!(info["label"], "Plan A\nsecond line");

    // Units change the automatic text, as one undo step.
    run(
        &mut d,
        json!({"type": "set_units", "length": "centimeters", "precision": 0}),
    )
    .unwrap();
    assert_eq!(entity(&d, &auto)["label"], "(200 cm, 100 cm, 0 cm)");
    assert_eq!(
        ask(&d, json!({"type": "units"})),
        json!({"type": "units", "length": "centimeters", "precision": 0, "length_snap": 0.0,
            "angle_precision": 1, "angle_snap": 15.0})
    );
    run(
        &mut d,
        json!({"type": "undo", "expect": {"label": "set_units"}}),
    )
    .unwrap();
    assert_eq!(entity(&d, &auto)["label"], "(2 m, 1 m, 0 m)");
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(d.units().length, LengthUnit::Centimeters);
    // Setting the same units again makes no step.
    let depth = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    run(&mut d, json!({"type": "set_units", "precision": 0})).unwrap();
    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], depth);

    // Text changes: written, then automatic again; a screen note needs text.
    run(
        &mut d,
        json!({"type": "change_note", "note": auto, "text": "Corner"}),
    )
    .unwrap();
    assert_eq!(entity(&d, &auto)["label"], "Corner");
    run(
        &mut d,
        json!({"type": "change_note", "note": auto, "text": null}),
    )
    .unwrap();
    assert_eq!(entity(&d, &auto)["label"], "(200 cm, 100 cm, 0 cm)");
    let (kind, message, _) =
        run(&mut d, json!({"type": "change_note", "note": screen})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("needs text"), "{message}");
    let (kind, _, ids) = run(
        &mut d,
        json!({"type": "change_note", "note": v, "text": "x"}),
    )
    .unwrap_err();
    assert_eq!((kind, ids), (ErrorKind::InvalidInput, vec![v.clone()]));
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "change_note", "note": "n999", "text": "x"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
}

#[test]
fn note_text_and_units_are_checked() {
    let mut d = doc();
    for text in [
        json!("   "),
        json!("x".repeat(1025)),
        json!("a\n".repeat(65)),
    ] {
        let (kind, _, _) = run(
            &mut d,
            json!({"type": "add_note", "anchor": [0, 0, 0], "position": [1, 1, 1], "text": text}),
        )
        .unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{text}");
    }
    run(
        &mut d,
        json!({"type": "add_note", "anchor": [0, 0, 0], "position": [1, 1, 1], "text": "x".repeat(1024)}),
    )
    .unwrap();
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "add_screen_note", "at": [1.5, 0], "text": "off the view"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "add_note", "anchor": [0, 0, 0], "position": [2e6, 0, 0]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, message, _) = run(&mut d, json!({"type": "set_units", "precision": 7})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("precision"), "{message}");
    let (kind, _, _) = run(&mut d, json!({"type": "set_units"})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(d.units(), Units::default());
}

#[test]
fn an_atomic_batch_that_fails_puts_the_units_back_and_one_that_works_is_one_step() {
    let mut d = doc();
    let batch = vec![
        Command::SetUnits {
            length: Some(LengthUnit::Inches),
            precision: None,
            length_snap: None,
            angle_precision: None,
            angle_snap: None,
        },
        serde_json::from_value(json!({"type": "erase", "ids": ["e999"]})).unwrap(),
    ];
    d.execute_batch(batch, true).unwrap_err();
    assert_eq!(d.units(), Units::default());
    let batch: Vec<Command> = vec![
        Command::SetUnits {
            length: Some(LengthUnit::Inches),
            precision: Some(2),
            length_snap: None,
            angle_precision: None,
            angle_snap: None,
        },
        serde_json::from_value(
            json!({"type": "add_note", "anchor": [0, 0, 0], "position": [1, 1, 1]}),
        )
        .unwrap(),
    ];
    d.execute_batch(batch, true).unwrap();
    assert_eq!(d.units(), units(LengthUnit::Inches, 2));
    assert_eq!(
        ask(&d, json!({"type": "history"}))["undo"],
        json!(["batch"])
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(d.units(), Units::default());
    assert_eq!(ask(&d, json!({"type": "annotations"}))["notes"], json!([]));
}

#[test]
fn batch_references_name_annotations_and_anchor_vertices() {
    let mut d = doc();
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [3, 0, 0]}),
        json!({"type": "add_dimension", "a": "$0:v0", "b": "$0:v1", "offset": [0, 1, 0]}),
        json!({"type": "add_edge_dimension", "edge": "$0:e0", "offset": [0, -1, 0]}),
        json!({"type": "add_note", "anchor": "$0:v1", "position": [4, 1, 0]}),
        json!({"type": "change_note", "note": "$3:n0", "text": "End"}),
        json!({"type": "select", "ids": ["$1:r0", "$3:*"]}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let outcomes = d.execute_batch(commands, false).unwrap();
    let r = outcomes[1].changes.created[0].to_string();
    let n = outcomes[3].changes.created[0].to_string();
    assert_eq!(entity(&d, &r)["label"], "3 m");
    assert_eq!(entity(&d, &n)["label"], "End");
    let selection = ask(&d, json!({"type": "selection"}));
    assert_eq!(selection["ids"], json!([r, n]));
    // A reference to a vertex in an annotation field of another command is not one.
    let err = BatchCommand::parse(
        json!({"type": "add_screen_note", "at": "$0:v0", "text": "x"}),
        1,
    )
    .unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn select_erase_and_tag_take_annotation_ids() {
    let mut d = doc();
    rectangle(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_dimension", "a": [0, 0, 0], "b": [1, 0, 0], "offset": [0, 1, 0]}),
    )
    .unwrap();
    let r = made(&out, 'r');
    let out = run(
        &mut d,
        json!({"type": "add_screen_note", "at": [0.5, 0.5], "text": "Hi"}),
    )
    .unwrap();
    let n = made(&out, 'n');
    run(&mut d, json!({"type": "select", "ids": [r, n]})).unwrap();
    assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!([r, n]));
    run(&mut d, json!({"type": "select", "all": true})).unwrap();
    let all = ask(&d, json!({"type": "selection"}))["ids"].clone();
    assert!(all.as_array().unwrap().contains(&json!(r)), "{all}");
    assert!(all.as_array().unwrap().contains(&json!(n)), "{all}");

    let out = run(&mut d, json!({"type": "add_tag", "name": "Notes"})).unwrap();
    let tag = out["changes"]["tags"][0].clone();
    run(
        &mut d,
        json!({"type": "set_tag", "ids": [r, n], "tag": tag}),
    )
    .unwrap();
    assert_eq!(entity(&d, &r)["tag"], tag);
    assert_eq!(entity(&d, &n)["tag"], tag);
    assert_eq!(ask(&d, json!({"type": "tags"}))["tags"][0]["count"], 2);
    // Hidden, they leave select all.
    run(
        &mut d,
        json!({"type": "change_tag", "tag": tag, "visible": false}),
    )
    .unwrap();
    run(&mut d, json!({"type": "select", "all": true})).unwrap();
    let all = ask(&d, json!({"type": "selection"}))["ids"].clone();
    assert!(!all.as_array().unwrap().contains(&json!(r)), "{all}");

    let out = run(&mut d, json!({"type": "erase", "ids": [r, n]})).unwrap();
    assert_eq!(out["changes"]["deleted"], json!([r, n]));
    assert_eq!(
        ask(&d, json!({"type": "annotations"}))["dimensions"],
        json!([])
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(entity(&d, &r)["label"], "1 m");
    // Paint and soften do not take them.
    let (kind, _, ids) = run(&mut d, json!({"type": "paint", "ids": [r]})).unwrap_err();
    assert_eq!((kind, ids), (ErrorKind::InvalidInput, vec![r.clone()]));
}

#[test]
fn erasing_an_end_vertex_freezes_the_dimension_and_undo_attaches_it_again() {
    // Review Focus 1, through commands.
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [2, 0, 0]}),
    )
    .unwrap();
    let first = made(&out, 'e');
    let out = run(
        &mut d,
        json!({"type": "add_edge", "start": [2, 0, 0], "end": [2, 3, 0]}),
    )
    .unwrap();
    let second = made(&out, 'e');
    let out = run(
        &mut d,
        json!({"type": "add_edge_dimension", "edge": first, "offset": [0, -1, 0]}),
    )
    .unwrap();
    let r = made(&out, 'r');
    let start = entity(&d, &r)["a"].clone();
    // Erasing the first edge removes its vertex at the origin: that end freezes there.
    run(&mut d, json!({"type": "erase", "ids": [first]})).unwrap();
    let info = entity(&d, &r);
    assert_eq!(info["a"], json!({"point": [0.0, 0.0, 0.0]}));
    assert!(info["b"]["vertex"].is_string());
    assert_eq!(info["label"], "2 m");
    // Moving the remaining vertex moves only the attached end.
    let corner = info["b"]["vertex"].clone();
    run(
        &mut d,
        json!({"type": "move", "ids": [corner], "offset": [1, 0, 0]}),
    )
    .unwrap();
    let info = entity(&d, &r);
    assert_eq!(info["a"], json!({"point": [0.0, 0.0, 0.0]}));
    assert_eq!(info["b"]["point"], json!([3.0, 0.0, 0.0]));
    assert_eq!(info["label"], "3 m");
    run(&mut d, json!({"type": "undo", "count": 2})).unwrap();
    assert_eq!(entity(&d, &r)["a"], start);
    let _ = second;
}

#[test]
fn moving_rotating_and_scaling_annotations_moves_their_points_not_their_vertices() {
    let mut d = doc();
    rectangle(&mut d);
    let v0 = vertex_at(&d, [0.0, 0.0, 0.0]);
    let out = run(
        &mut d,
        json!({"type": "add_dimension", "a": v0, "b": [0, -2, 0], "offset": [1, 0, 0]}),
    )
    .unwrap();
    let r = made(&out, 'r');
    let out = run(
        &mut d,
        json!({"type": "add_note", "anchor": [5, 5, 0], "position": [6, 6, 0], "text": "x"}),
    )
    .unwrap();
    let n = made(&out, 'n');
    let out = run(
        &mut d,
        json!({"type": "move", "ids": [r, n], "offset": [0, -1, 0]}),
    )
    .unwrap();
    assert_eq!(out["changes"]["modified"], json!([r, n]));
    let info = entity(&d, &r);
    // The vertex end stays with its vertex.
    assert_eq!(info["a"]["point"], json!([0.0, 0.0, 0.0]));
    assert_eq!(info["b"]["point"], json!([0.0, -3.0, 0.0]));
    assert_eq!(entity(&d, &n)["anchor"]["point"], json!([5.0, 4.0, 0.0]));
    assert_eq!(entity(&d, &n)["position"], json!([6.0, 5.0, 0.0]));

    run(
        &mut d,
        json!({"type": "rotate", "ids": [n], "center": [0, 0, 0], "angle": 90}),
    )
    .unwrap();
    let anchor = entity(&d, &n)["anchor"]["point"].clone();
    assert!((anchor[0].as_f64().unwrap() + 4.0).abs() < 1e-9, "{anchor}");
    assert!((anchor[1].as_f64().unwrap() - 5.0).abs() < 1e-9, "{anchor}");

    run(
        &mut d,
        json!({"type": "scale", "ids": [r], "origin": [0, 0, 0], "factors": [1, 2, 1]}),
    )
    .unwrap();
    assert_eq!(entity(&d, &r)["b"]["point"], json!([0.0, -6.0, 0.0]));
    assert_eq!(entity(&d, &r)["label"], "6 m");

    // A move that brings the point end onto the vertex end removes the dimension.
    let out = run(
        &mut d,
        json!({"type": "move", "ids": [r], "offset": [0, 6, 0]}),
    )
    .unwrap();
    assert_eq!(out["changes"]["deleted"], json!([r]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(entity(&d, &r)["label"], "6 m");

    // Points beyond the limit are refused, naming nothing changed.
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "move", "ids": [n], "offset": [2e6, 0, 0]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    let (kind, _, _) = run(
        &mut d,
        json!({"type": "scale", "ids": [r], "origin": [0, 0, 0], "factors": [1, 1e6, 1]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
}

#[test]
fn a_copied_dimension_follows_the_copied_vertices_not_the_originals() {
    // Review Focus 2, through commands.
    let mut d = doc();
    let r = rectangle(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_edge_dimension", "edge": r.bottom, "offset": [0, -1, 0]}),
    )
    .unwrap();
    let dim = made(&out, 'r');
    let out = run(
        &mut d,
        json!({"type": "add_note", "anchor": [9, 9, 0], "position": [9, 10, 0], "text": "far"}),
    )
    .unwrap();
    let note = made(&out, 'n');
    let mut ids: Vec<Value> = r.edges.iter().map(|e| json!(e)).collect();
    ids.extend([json!(r.face), json!(dim), json!(note)]);
    let out = run(
        &mut d,
        json!({"type": "copy", "ids": ids, "offset": [5, 0, 0]}),
    )
    .unwrap();
    let copy = made(&out, 'r');
    let note_copy = made(&out, 'n');
    let info = entity(&d, &copy);
    let copied_start = vertex_at(&d, [5.0, 0.0, 0.0]);
    assert_eq!(info["a"]["vertex"], json!(copied_start), "{info}");
    assert_eq!(info["line"], json!([[5.0, -1.0, 0.0], [7.0, -1.0, 0.0]]));
    assert_eq!(
        entity(&d, &note_copy)["anchor"]["point"],
        json!([14.0, 9.0, 0.0])
    );
    // Moving the original rectangle leaves the copy's dimension where it is.
    let mut originals: Vec<Value> = r.edges.iter().map(|e| json!(e)).collect();
    originals.push(json!(r.face));
    run(
        &mut d,
        json!({"type": "move", "ids": originals, "offset": [0, 0, 3]}),
    )
    .unwrap();
    assert_eq!(
        entity(&d, &copy)["line"],
        json!([[5.0, -1.0, 0.0], [7.0, -1.0, 0.0]])
    );
    assert_eq!(entity(&d, &dim)["a"]["point"][2], 3.0);

    // A dimension copied without its geometry anchors to points.
    let out = run(
        &mut d,
        json!({"type": "copy", "ids": [dim], "offset": [0, 10, 0], "count": 2}),
    )
    .unwrap();
    let copies = created(&out, 'r');
    assert_eq!(copies.len(), 2);
    let last = entity(&d, &copies[1]);
    assert_eq!(last["a"], json!({"point": [0.0, 20.0, 3.0]}));
}

#[test]
fn annotations_group_into_the_definition_and_come_out_when_exploded() {
    // Review Focus 3, the session part.
    let mut d = doc();
    let r = rectangle(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_edge_dimension", "edge": r.bottom, "offset": [0, -1, 0]}),
    )
    .unwrap();
    let dim = made(&out, 'r');
    let out = run(
        &mut d,
        json!({"type": "add_screen_note", "at": [0.2, 0.2], "text": "Title"}),
    )
    .unwrap();
    let screen = made(&out, 'n');
    let mut ids: Vec<Value> = r.edges.iter().map(|e| json!(e)).collect();
    ids.extend([json!(r.face), json!(dim), json!(screen)]);
    let out = run(&mut d, json!({"type": "make_group", "ids": ids})).unwrap();
    let instance = made(&out, 'i');
    // The screen note stays in the root; the dimension went into the group.
    let root = ask(&d, json!({"type": "annotations"}));
    assert_eq!(root["dimensions"], json!([]));
    assert_eq!(root["notes"][0]["id"], json!(screen));
    // Moving the group moves what the dimension measures, in root terms.
    run(
        &mut d,
        json!({"type": "move", "ids": [instance], "offset": [0, 0, 4]}),
    )
    .unwrap();
    run(&mut d, json!({"type": "open_instance", "id": instance})).unwrap();
    let inside = ask(&d, json!({"type": "annotations"}));
    assert_eq!(inside["dimensions"][0]["id"], json!(dim));
    assert!(inside["dimensions"][0]["a"]["vertex"].is_string());
    assert_eq!(inside["dimensions"][0]["label"], "2 m");
    // A note's automatic text is in root coordinates.
    let out = run(
        &mut d,
        json!({"type": "add_note", "anchor": [0, 0, 0], "position": [1, 1, 1]}),
    )
    .unwrap();
    let auto = made(&out, 'n');
    assert_eq!(entity(&d, &auto)["label"], "(0 m, 0 m, 4 m)");
    // Screen notes go only into the root.
    let (kind, message, _) = run(
        &mut d,
        json!({"type": "add_screen_note", "at": [0.5, 0.5], "text": "no"}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("screen notes"), "{message}");
    run(&mut d, json!({"type": "close_instance"})).unwrap();

    // Guides and annotations alone cannot be grouped.
    let (kind, message, _) =
        run(&mut d, json!({"type": "make_group", "ids": [screen]})).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(
        message.contains("section planes alone cannot be grouped"),
        "{message}"
    );

    let out = run(&mut d, json!({"type": "explode", "ids": [instance]})).unwrap();
    let dims = created(&out, 'r');
    assert_eq!(dims.len(), 1);
    let info = entity(&d, &dims[0]);
    assert!(info["a"]["vertex"].is_string());
    assert_eq!(info["a"]["point"][2], 4.0);
    let selection = ask(&d, json!({"type": "selection"}))["ids"].clone();
    assert!(selection.as_array().unwrap().contains(&json!(dims[0])));
}

#[test]
fn clips_carry_annotations_into_a_paste() {
    let mut d = doc();
    let r = rectangle(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_edge_dimension", "edge": r.bottom, "offset": [0, -1, 0]}),
    )
    .unwrap();
    let dim = made(&out, 'r');
    let entity_ids: Vec<EntityId> = r
        .edges
        .iter()
        .chain([&r.face, &dim])
        .map(|s| s.parse().unwrap())
        .collect();
    let clip = d.clip(&entity_ids).unwrap();
    run(
        &mut d,
        json!({"type": "paste", "clip": clip, "at": [10, 0, 0]}),
    )
    .unwrap();
    let selection = ask(&d, json!({"type": "selection"}))["ids"].clone();
    let pasted: Vec<String> = selection
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .filter(|s| s.starts_with('r'))
        .collect();
    assert_eq!(pasted.len(), 1);
    let info = entity(&d, &pasted[0]);
    assert!(info["a"]["vertex"].is_string());
    assert_eq!(info["line"][0][0], 10.0);
}

#[test]
fn files_keep_annotations_units_frozen_anchors_and_screen_notes() {
    // Review Focus 4.
    let mut d = doc();
    let r = rectangle(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_edge_dimension", "edge": r.bottom, "offset": [0, -1, 0]}),
    )
    .unwrap();
    let dim = made(&out, 'r');
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [5, 5, 0], "radius": 1}),
    )
    .unwrap();
    let circle = created(&out, 'e')[0].clone();
    run(
        &mut d,
        json!({"type": "add_radial_dimension", "edge": circle, "diameter": true, "leader": [1, 1, 0]}),
    )
    .unwrap();
    let v = vertex_at(&d, [2.0, 1.0, 0.0]);
    run(
        &mut d,
        json!({"type": "add_note", "anchor": v, "position": [3, 3, 0]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_screen_note", "at": [0.25, 0.75], "text": "Sheet 1"}),
    )
    .unwrap();
    // Freeze one end of the edge dimension: erasing both edges at the origin removes it.
    let left = r
        .edges
        .iter()
        .find(|e| {
            let info = entity(&d, e);
            info["start"][0] == 0.0 && info["end"][0] == 0.0
        })
        .unwrap()
        .clone();
    run(&mut d, json!({"type": "erase", "ids": [r.bottom, left]})).unwrap();
    let info = entity(&d, &dim);
    let frozen = [&info["a"], &info["b"]]
        .iter()
        .filter(|a| a.get("vertex").is_none())
        .count();
    assert_eq!(frozen, 1, "{info}");
    run(
        &mut d,
        json!({"type": "set_units", "length": "feet_inches", "precision": 4}),
    )
    .unwrap();
    let before = ask(&d, json!({"type": "annotations"}));
    let path = scratch("annotated.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(
        document["schema_version"],
        session::file::ANNOTATIONS_SCHEMA_VERSION
    );
    assert_eq!(document["schema_version"], 12);
    assert_eq!(
        document["units"],
        json!({"length": "feet_inches", "precision": 4})
    );

    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let mut after = ask(&e, json!({"type": "annotations"}));
    let mut before = before;
    before["revision"] = Value::Null;
    after["revision"] = Value::Null;
    assert_eq!(after, before);
    assert_eq!(e.units(), units(LengthUnit::FeetInches, 4));
    // A new model starts in the default units.
    run(&mut e, json!({"type": "new", "discard": true})).unwrap();
    assert_eq!(e.units(), Units::default());
}

#[test]
fn files_without_annotations_keep_their_schema_and_open_with_none() {
    let mut d = doc();
    rectangle(&mut d);
    let plain = scratch("plain.skr");
    run(
        &mut d,
        json!({"type": "save", "path": plain.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(
        stored(&plain)["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
    run(&mut d, json!({"type": "add_scene"})).unwrap();
    let scenes = scratch("scenes.skr");
    run(
        &mut d,
        json!({"type": "save", "path": scenes.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&scenes);
    assert_eq!(document["schema_version"], 11);
    assert_eq!(
        document["schema_version"],
        session::file::SCENES_SCHEMA_VERSION
    );
    assert!(document.get("units").is_none());
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": scenes.to_str().unwrap()}),
    )
    .unwrap();
    let all = ask(&e, json!({"type": "annotations"}));
    assert_eq!(
        (&all["dimensions"], &all["notes"]),
        (&json!([]), &json!([]))
    );
    assert_eq!(e.units(), Units::default());

    // Units alone are worth schema 12.
    run(
        &mut e,
        json!({"type": "set_units", "length": "millimeters"}),
    )
    .unwrap();
    let units_only = scratch("units-only.skr");
    run(
        &mut e,
        json!({"type": "save", "path": units_only.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&units_only)["schema_version"], 12);
    // Back to the default, it is not.
    run(&mut e, json!({"type": "undo"})).unwrap();
    run(
        &mut e,
        json!({"type": "save", "path": units_only.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(stored(&units_only)["schema_version"], 11);
}

#[test]
fn files_with_broken_annotations_are_refused_clearly() {
    let mut d = doc();
    rectangle(&mut d);
    let v = vertex_at(&d, [0.0, 0.0, 0.0]);
    run(
        &mut d,
        json!({"type": "add_dimension", "a": v, "b": [0, -3, 0], "offset": [1, 0, 0]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_note", "anchor": [0, 0, 0], "position": [1, 1, 1], "text": "n"}),
    )
    .unwrap();
    let path = scratch("broken.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let open = |path: &PathBuf| {
        let mut e = doc();
        run(
            &mut e,
            json!({"type": "open", "path": path.to_str().unwrap()}),
        )
        .unwrap_err()
    };

    let missing = scratch("missing-vertex.skr");
    std::fs::copy(&path, &missing).unwrap();
    edit_file(&missing, |doc| {
        doc["model"]["root"]["dimensions"][0]["a"] = json!({"vertex": 999});
    });
    let (kind, message, _) = open(&missing);
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("v999"), "{message}");

    let far = scratch("far-note.skr");
    std::fs::copy(&path, &far).unwrap();
    edit_file(&far, |doc| {
        doc["model"]["root"]["notes"][0]["position"] = json!([2e6, 0, 0]);
    });
    let (kind, message, _) = open(&far);
    assert_eq!(kind, ErrorKind::File);
    assert!(
        message.contains("note n") && message.contains("beyond"),
        "{message}"
    );

    let precise = scratch("precise.skr");
    std::fs::copy(&path, &precise).unwrap();
    edit_file(&precise, |doc| {
        doc["units"] = json!({"length": "meters", "precision": 9});
    });
    let (kind, message, _) = open(&precise);
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("precision"), "{message}");

    // A newer schema is refused as before.
    let newer = scratch("newer.skr");
    std::fs::copy(&path, &newer).unwrap();
    edit_file(&newer, |doc| doc["schema_version"] = json!(19));
    let (_, message, _) = open(&newer);
    assert!(message.contains("newer"), "{message}");
}

#[test]
fn exports_report_the_annotations_they_leave_out() {
    let mut d = doc();
    rectangle(&mut d);
    run(
        &mut d,
        json!({"type": "add_dimension", "a": [0, 0, 0], "b": [2, 0, 0], "offset": [0, -1, 0]}),
    )
    .unwrap();
    for text in ["a", "b"] {
        run(
            &mut d,
            json!({"type": "add_note", "anchor": [0, 0, 0], "position": [1, 1, 1], "text": text}),
        )
        .unwrap();
    }
    for name in ["out.obj", "out.stl", "out.glb", "out.skp"] {
        let path = scratch(name);
        let out = run(
            &mut d,
            json!({"type": "export", "path": path.to_str().unwrap()}),
        )
        .unwrap();
        let warnings = out["export"]["warnings"].as_array().unwrap();
        assert!(
            warnings.contains(&json!("1 dimension was not written")),
            "{name}: {warnings:?}"
        );
        assert!(
            warnings.contains(&json!("2 notes were not written")),
            "{name}: {warnings:?}"
        );
    }
}

#[test]
fn a_malformed_anchor_says_what_an_anchor_is() {
    for anchor in [json!("e"), json!([1, 2]), json!({"vertex": 3}), json!(5)] {
        let err = serde_json::from_value::<Command>(
            json!({"type": "add_note", "anchor": anchor, "position": [0, 0, 0]}),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("a vertex id such as \"v3\", or a point [x, y, z]"),
            "{anchor}: {err}"
        );
    }
}

#[test]
fn scaling_a_fixed_circle_past_the_limit_is_refused_and_the_model_still_saves() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 200}),
    )
    .unwrap();
    let edges = created(&out, 'e');
    let out = run(
        &mut d,
        json!({"type": "add_radial_dimension", "edge": edges[0], "leader": [1, 0, 0]}),
    )
    .unwrap();
    let r = made(&out, 'r');
    // Erasing the circle leaves the dimension on a fixed circle of radius 200.
    let ids: Vec<Value> = edges.iter().map(|e| json!(e)).collect();
    run(&mut d, json!({"type": "erase", "ids": ids})).unwrap();
    let before = entity(&d, &r);
    assert!(before.get("curve").is_none(), "{before}");
    // Scaled by 1e4 the circle would reach 2e6 m: refused, nothing changed.
    let revision = d.revision();
    let (kind, message, _) = run(
        &mut d,
        json!({"type": "scale", "ids": [r], "origin": [0, 0, 0], "factors": [1e4, 1e4, 1e4]}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(message.contains("within"), "{message}");
    assert_eq!(d.revision(), revision);
    assert_eq!(entity(&d, &r), before);
    // A smaller scale is fine, and the model saves and opens.
    run(
        &mut d,
        json!({"type": "scale", "ids": [r], "origin": [0, 0, 0], "factors": [100, 100, 100]}),
    )
    .unwrap();
    let path = scratch("scaled-circle.skr");
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
    assert_eq!(entity(&e, &r)["radius"], 20000.0);
    // A file whose fixed circle reaches past the limit is refused.
    edit_file(&path, |doc| {
        doc["model"]["root"]["dimensions"][0]["curve"]["fixed"]["radius"] = json!(2e6);
    });
    let (kind, message, _) = run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("dimension r"), "{message}");
}

#[test]
fn a_fixed_circle_inside_a_group_is_checked_when_the_group_is_scaled() {
    let mut d = doc();
    let r = rectangle(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 200}),
    )
    .unwrap();
    let circle = created(&out, 'e');
    let out = run(
        &mut d,
        json!({"type": "add_radial_dimension", "edge": circle[0], "leader": [1, 0, 0]}),
    )
    .unwrap();
    let dim = made(&out, 'r');
    let ids: Vec<Value> = circle.iter().map(|e| json!(e)).collect();
    run(&mut d, json!({"type": "erase", "ids": ids})).unwrap();
    let out = run(&mut d, json!({"type": "make_group", "ids": [r.face, dim]})).unwrap();
    let group = made(&out, 'i');
    let (kind, _, ids) = run(
        &mut d,
        json!({"type": "scale", "ids": [group], "origin": [0, 0, 0], "factors": [1e4, 1e4, 1]}),
    )
    .unwrap_err();
    assert_eq!((kind, ids), (ErrorKind::InvalidInput, vec![group.clone()]));
}

fn full(length: &str, precision: u8) -> Value {
    json!({"length": length, "precision": precision, "length_snap": 0.0,
        "angle_precision": 1, "angle_snap": 15.0})
}

#[test]
fn outcomes_and_events_carry_the_display_units_they_set() {
    let mut d = doc();
    let start = d.revision();
    let out = run(&mut d, json!({"type": "set_units", "length": "inches"})).unwrap();
    assert_eq!(out["units"], full("inches", 3));
    let out = run(
        &mut d,
        json!({"type": "add_note", "anchor": [0, 0, 0], "position": [1, 1, 1]}),
    )
    .unwrap();
    assert!(out.get("units").is_none(), "{out}");
    let out = run(&mut d, json!({"type": "undo", "count": 2})).unwrap();
    assert_eq!(out["units"], full("meters", 3));
    let out = run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(out["units"], full("inches", 3));
    let events = ask(&d, json!({"type": "events", "since": start}))["events"].clone();
    let set: Vec<Value> = events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.get("units").cloned().unwrap_or(Value::Null))
        .collect();
    assert_eq!(
        set,
        vec![
            full("inches", 3),
            Value::Null,
            Value::Null,
            full("meters", 3),
            full("inches", 3),
        ]
    );
    // In an atomic batch, the command that set them says so.
    let batch: Vec<Command> = vec![
        serde_json::from_value(json!({"type": "set_units", "precision": 1})).unwrap(),
        serde_json::from_value(
            json!({"type": "add_note", "anchor": [0, 0, 0], "position": [1, 1, 1]}),
        )
        .unwrap(),
    ];
    let outcomes = d.execute_batch(batch, true).unwrap();
    assert_eq!(outcomes[0].units, Some(units(LengthUnit::Inches, 1)));
    assert_eq!(outcomes[1].units, None);
}

/// An edge (0,0,0)-(0,1,0) with a dimension from its first vertex to the point [1,0,0].
fn edge_with_point_dimension(d: &mut Document) -> (Vec<Value>, String) {
    let out = run(
        d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [0, 1, 0]}),
    )
    .unwrap();
    let edge = made(&out, 'e');
    let v0 = vertex_at(d, [0.0, 0.0, 0.0]);
    let out = run(
        d,
        json!({"type": "add_dimension", "a": v0, "b": [1, 0, 0], "offset": [0, -1, 0]}),
    )
    .unwrap();
    let r = made(&out, 'r');
    (vec![json!(edge), json!(r)], r)
}

#[test]
fn moving_an_edge_with_its_dimension_onto_the_dimensions_old_point_keeps_the_dimension() {
    let mut d = doc();
    let (ids, r) = edge_with_point_dimension(&mut d);
    run(
        &mut d,
        json!({"type": "move", "ids": ids, "offset": [1, 0, 0]}),
    )
    .unwrap();
    let info = entity(&d, &r);
    assert_eq!(info["value"], 1.0, "{info}");
    assert_eq!(info["a"]["point"], json!([1.0, 0.0, 0.0]));
    assert_eq!(info["b"]["point"], json!([2.0, 0.0, 0.0]));
    run(&mut d, json!({"type": "undo"})).unwrap();
    let info = entity(&d, &r);
    assert_eq!(info["a"]["point"], json!([0.0, 0.0, 0.0]));
    assert_eq!(info["b"]["point"], json!([1.0, 0.0, 0.0]));
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(entity(&d, &r)["a"]["point"], json!([1.0, 0.0, 0.0]));
}

#[test]
fn rotating_an_edge_with_its_dimension_onto_the_dimensions_old_point_keeps_the_dimension() {
    let mut d = doc();
    let (ids, r) = edge_with_point_dimension(&mut d);
    run(
        &mut d,
        json!({"type": "rotate", "ids": ids, "center": [0.5, 0, 0], "axis": [0, 0, 1], "angle": 180}),
    )
    .unwrap();
    let info = entity(&d, &r);
    assert_eq!(info["value"], 1.0, "{info}");
    let a = info["a"]["point"].as_array().unwrap();
    assert!((a[0].as_f64().unwrap() - 1.0).abs() < 1e-9, "{info}");
    run(&mut d, json!({"type": "undo"})).unwrap();
    let info = entity(&d, &r);
    assert_eq!(info["a"]["point"], json!([0.0, 0.0, 0.0]));
    assert_eq!(info["b"]["point"], json!([1.0, 0.0, 0.0]));
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(entity(&d, &r)["value"], 1.0);
}

#[test]
fn a_dimension_frozen_near_the_limit_still_saves_and_opens() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1}),
    )
    .unwrap();
    let edges = created(&out, 'e');
    let out = run(
        &mut d,
        json!({"type": "add_radial_dimension", "edge": edges[0], "leader": [0.5, 0, 0]}),
    )
    .unwrap();
    let r = made(&out, 'r');
    let mut ids: Vec<Value> = edges.iter().map(|e| json!(e)).collect();
    run(
        &mut d,
        json!({"type": "move", "ids": ids, "offset": [0, 0, 999999.5]}),
    )
    .unwrap();
    ids = edges.iter().map(|e| json!(e)).collect();
    run(&mut d, json!({"type": "erase", "ids": ids})).unwrap();
    assert!(entity(&d, &r).get("curve").is_none());
    let path = scratch("frozen-near-limit.skr");
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
    assert_eq!(entity(&e, &r)["radius"], 1.0);
}

#[test]
fn set_units_takes_the_snapping_and_angle_settings_in_one_undo_step() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "set_units", "length": "feet", "precision": 2, "length_snap": 0.0254,
            "angle_precision": 2, "angle_snap": 5}),
    )
    .unwrap();
    let want = Units {
        length: LengthUnit::Feet,
        precision: 2,
        length_snap: 0.0254,
        angle_precision: 2,
        angle_snap: 5.0,
    };
    assert_eq!(d.units(), want);
    assert_eq!(out["units"]["angle_snap"], 5.0);
    assert_eq!(ask(&d, json!({"type": "units"}))["length"], "feet");
    // Absent fields stay.
    run(&mut d, json!({"type": "set_units", "angle_snap": 0})).unwrap();
    assert_eq!(
        d.units(),
        Units {
            angle_snap: 0.0,
            ..want
        }
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(d.units(), want);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(d.units(), Units::default());
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(d.units(), want);
}

#[test]
fn set_units_refuses_settings_out_of_range_and_changes_nothing() {
    let mut d = doc();
    for bad in [
        json!({"length_snap": -0.1}),
        json!({"length_snap": 1000.5}),
        json!({"angle_precision": 4}),
        json!({"angle_snap": 91}),
        json!({"angle_snap": -1}),
        json!({"precision": 3, "angle_snap": 120}),
    ] {
        let mut command = json!({"type": "set_units"});
        command
            .as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        let (kind, message, _) = run(&mut d, command).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{bad}");
        assert!(
            message.contains("length_snap") || message.contains("angle"),
            "{message}"
        );
    }
    assert_eq!(d.units(), Units::default());
}

fn save_units(d: &mut Document, name: &str) -> PathBuf {
    let path = scratch(name);
    run(d, json!({"type": "save", "path": path.to_str().unwrap()})).unwrap();
    path
}

#[test]
fn schema_17_is_written_only_for_the_new_unit_settings() {
    let mut d = doc();
    rectangle(&mut d);
    let plain = save_units(&mut d, "units-plain.skr");
    assert_eq!(
        stored(&plain)["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
    assert!(stored(&plain).get("units").is_none());
    // Non-default units an older build reads keep schema 12 and the old two fields.
    run(
        &mut d,
        json!({"type": "set_units", "length": "inches", "precision": 2}),
    )
    .unwrap();
    let old = save_units(&mut d, "units-old.skr");
    assert_eq!(stored(&old)["schema_version"], 12);
    assert_eq!(
        stored(&old)["units"],
        json!({"length": "inches", "precision": 2})
    );
    // Each new setting needs 17, and reads back.
    for (set, expect) in [
        (
            json!({"length": "feet", "precision": 3}),
            json!({"length": "feet", "precision": 3}),
        ),
        (
            json!({"length": "meters", "length_snap": 0.05}),
            json!({"length": "meters", "precision": 2, "length_snap": 0.05}),
        ),
        (
            json!({"length_snap": 0, "angle_precision": 0}),
            json!({"length": "meters", "precision": 2, "angle_precision": 0}),
        ),
        (
            json!({"angle_precision": 1, "angle_snap": 0}),
            json!({"length": "meters", "precision": 2, "angle_snap": 0.0}),
        ),
    ] {
        let mut d = doc();
        rectangle(&mut d);
        let mut command = json!({"type": "set_units", "precision": 2});
        command
            .as_object_mut()
            .unwrap()
            .extend(set.as_object().unwrap().clone());
        run(&mut d, command).unwrap();
        let path = save_units(&mut d, "units-17.skr");
        let file = stored(&path);
        assert_eq!(file["schema_version"], 17, "{set}");
        let mut expect = expect;
        if expect.get("precision").is_none() {
            expect["precision"] = json!(2);
        }
        assert_eq!(file["units"], expect, "{set}");
        let mut e = doc();
        run(
            &mut e,
            json!({"type": "open", "path": path.to_str().unwrap()}),
        )
        .unwrap();
        assert_eq!(e.units(), d.units());
    }
}

#[test]
fn files_with_missing_new_unit_fields_open_with_the_defaults_and_bad_ones_are_refused() {
    let mut d = doc();
    rectangle(&mut d);
    run(
        &mut d,
        json!({"type": "set_units", "length": "centimeters", "precision": 1}),
    )
    .unwrap();
    let path = save_units(&mut d, "units-missing.skr");
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(e.units().angle_snap, 15.0);
    assert_eq!(e.units().angle_precision, 1);
    assert_eq!(e.units().length_snap, 0.0);
    edit_file(&path, |v| v["units"]["angle_snap"] = json!(200.0));
    let (kind, message, _) = run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::File);
    assert!(message.contains("angle_snap"), "{message}");
}

#[test]
fn the_api_always_reports_every_unit_field() {
    let mut d = doc();
    let all = |v: &Value| {
        for k in [
            "length",
            "precision",
            "length_snap",
            "angle_precision",
            "angle_snap",
        ] {
            assert!(v.get(k).is_some(), "{k} missing in {v}");
        }
    };
    let q = ask(&d, json!({"type": "units"}));
    all(&q);
    assert_eq!(q["angle_snap"], 15.0);
    assert_eq!(q["angle_precision"], 1);
    assert_eq!(q["length_snap"], 0.0);
    let out = run(&mut d, json!({"type": "set_units", "length": "inches"})).unwrap();
    all(&out["units"]);
    let out = run(&mut d, json!({"type": "undo"})).unwrap();
    all(&out["units"]);
    let (events, _) = d.events_since(0);
    let with_units: Vec<_> = events.iter().filter_map(|e| e.units).collect();
    assert!(!with_units.is_empty());
    for u in with_units {
        all(&serde_json::to_value(u).unwrap());
    }
}
