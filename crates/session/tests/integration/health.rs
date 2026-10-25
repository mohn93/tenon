//! The `model_health` query and the `make_solid` command.

use glam::DVec2;
use kernel::{EntityId, Model};
use serde_json::{Value, json};
use session::{Command, CommandError, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn try_run(d: &mut Document, v: Value) -> Result<Value, CommandError> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd).map(|o| serde_json::to_value(o).unwrap())
}

fn run(d: &mut Document, v: Value) -> Value {
    try_run(d, v).unwrap()
}

fn ask(d: &Document, v: Value) -> Value {
    let q: Query = serde_json::from_value(v).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn add_box(d: &mut Document, corner: [f64; 3], placement: &str) -> Value {
    run(
        d,
        json!({"type": "add_box", "corner": corner, "size": [1.0, 2.0, 3.0], "as": placement}),
    )["placed"][0]
        .clone()
}

fn mesh<'a>(d: &'a Document, id: &Value) -> &'a Model {
    let id: EntityId = serde_json::from_value(id.clone()).unwrap();
    let EntityId::Instance(i) = id else {
        panic!("{id} is not an instance")
    };
    let def = d.model().instance(i).unwrap().definition;
    &d.scene().definition(def).unwrap().mesh
}

/// Erases the top face of part `id`'s definition, from inside it.
fn erase_top(d: &mut Document, id: &Value) {
    let top = mesh(d, id)
        .faces()
        .find(|(_, f)| f.normal.z > 0.5)
        .map(|(f, _)| EntityId::Face(f))
        .unwrap();
    run(d, json!({"type": "open_path", "path": [id]}));
    run(d, json!({"type": "erase", "ids": [top]}));
    run(d, json!({"type": "open_path", "path": []}));
}

#[test]
fn health_lists_the_broken_part_with_why_and_not_the_solid_one() {
    let mut d = doc();
    let good = add_box(&mut d, [0.0, 0.0, 0.0], "group");
    let bad = add_box(&mut d, [5.0, 0.0, 0.0], "group");
    erase_top(&mut d, &bad);
    let r = ask(&d, json!({"type": "model_health"}));
    assert_eq!(r["type"], "model_health");
    assert_eq!(r["checked"], 2);
    let parts = r["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 1, "{r}");
    assert_eq!(parts[0]["id"], bad);
    assert_eq!(parts[0]["fixable"], true);
    let p = &parts[0]["problems"][0];
    assert_eq!(p["kind"], "open_edges");
    assert_eq!(p["text"], "4 open edges");
    assert_eq!(p["ids"].as_array().unwrap().len(), 4);
    let r = ask(&d, json!({"type": "model_health", "ids": [good]}));
    assert_eq!(r["parts"], json!([]));
    assert_eq!(r["checked"], 1);
}

#[test]
fn make_solid_fixes_a_component_once_for_every_instance_and_undoes_in_one_step() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0, 0.0, 0.0], "component");
    let b = run(
        &mut d,
        json!({"type": "copy", "ids": [a], "offset": [5.0, 0.0, 0.0]}),
    )["placed"][0]
        .clone();
    erase_top(&mut d, &a);
    let broken = mesh(&d, &a).clone();
    let steps = ask(&d, json!({"type": "history"}))["undo_depth"].clone();
    let out = run(&mut d, json!({"type": "make_solid", "ids": [a, b]}));
    let report = &out["make_solid"];
    let entries = report["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{report}");
    assert_eq!(entries[0]["fixed"], true);
    assert_eq!(entries[0]["holes_filled"], 1);
    assert_eq!(entries[0]["parts"], json!([a, b]));
    for id in [&a, &b] {
        let s = ask(&d, json!({"type": "solid", "id": id}));
        assert_eq!(s["solid"], true, "{s}");
        assert!((s["volume"].as_f64().unwrap() - 6.0).abs() < 1e-9);
    }
    let history = ask(&d, json!({"type": "history"}));
    assert_eq!(history["undo_depth"], steps.as_u64().unwrap() + 1);
    assert_eq!(history["undo"][0], "make_solid", "{history}");
    run(&mut d, json!({"type": "undo"}));
    assert_eq!(mesh(&d, &a), &broken);
}

#[test]
fn make_solid_on_solids_and_unfixable_parts_changes_nothing() {
    let mut d = doc();
    let good = add_box(&mut d, [0.0, 0.0, 0.0], "group");
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [5.0, 0.0, 0.0], "opposite": [6.0, 1.0, 0.0]}),
    );
    let m = d.model();
    let ids: Vec<EntityId> = m
        .edges()
        .map(|(e, _)| EntityId::Edge(e))
        .chain(m.faces().map(|(f, _)| EntityId::Face(f)))
        .collect();
    let lone = run(&mut d, json!({"type": "make_group", "ids": ids}))["placed"][0].clone();
    let revision = ask(&d, json!({"type": "history"}));
    let out = run(&mut d, json!({"type": "make_solid"}));
    let report = &out["make_solid"];
    assert_eq!(report["solid"], 1, "{report}");
    let e = &report["entries"][0];
    assert_eq!(e["parts"], json!([lone]));
    assert_eq!(e["fixed"], false);
    assert!(
        e["left"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["kind"] == "no_volume"),
        "{e}"
    );
    assert_eq!(ask(&d, json!({"type": "history"})), revision);
    let r = ask(&d, json!({"type": "model_health", "ids": [lone, good]}));
    assert_eq!(r["parts"][0]["fixable"], false);
}

#[test]
fn make_solid_and_health_take_only_groups_and_components() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0.0, 0.0, 0.0], "opposite": [1.0, 1.0, 0.0]}),
    );
    let face = d
        .model()
        .faces()
        .next()
        .map(|(f, _)| EntityId::Face(f))
        .unwrap();
    let err = try_run(&mut d, json!({"type": "make_solid", "ids": [face]})).unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    let q: Query =
        serde_json::from_value(json!({"type": "model_health", "ids": ["i999"]})).unwrap();
    assert_eq!(d.query(&q).unwrap_err().kind, ErrorKind::NotFound);
}
