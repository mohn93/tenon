//! The `solid` query: whether a group or component is a solid, and its volume.

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

fn solid(d: &Document, id: &Value) -> Value {
    let q: Query = serde_json::from_value(json!({"type": "solid", "id": id})).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn add(d: &mut Document, size: [f64; 3], as_: &str) -> Value {
    let out = run(
        d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": size, "as": as_}),
    );
    out["placed"][0].clone()
}

#[test]
fn a_box_group_is_a_solid_with_its_volume() {
    let mut d = doc();
    let id = add(&mut d, [2.0, 3.0, 4.0], "group");
    let r = solid(&d, &id);
    assert_eq!(r["type"], "solid");
    assert_eq!(r["solid"], true);
    assert!((r["volume"].as_f64().unwrap() - 24.0).abs() < 1e-9);
    assert!(r.get("reason").is_none());
}

#[test]
fn a_component_solid_scales_with_its_placement() {
    let mut d = doc();
    let id = add(&mut d, [1.0, 1.0, 1.0], "component");
    run(
        &mut d,
        json!({"type": "set_transform", "id": id,
               "transform": {"matrix": [2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0]}}),
    );
    let r = solid(&d, &id);
    assert!((r["volume"].as_f64().unwrap() - 24.0).abs() < 1e-9);
}

#[test]
fn an_open_group_says_why_it_is_not_a_solid() {
    let mut d = doc();
    let id = add(&mut d, [1.0, 1.0, 1.0], "group");
    run(&mut d, json!({"type": "open_instance", "id": id}));
    let face = d.model().faces().next().unwrap().0;
    run(
        &mut d,
        json!({"type": "erase", "ids": [format!("f{}", face.0)]}),
    );
    run(&mut d, json!({"type": "close_instance"}));
    let r = solid(&d, &id);
    assert_eq!(r["solid"], false);
    assert!(r.get("volume").is_none());
    let reason = r["reason"].as_str().unwrap();
    assert!(
        reason.starts_with("edge e") && reason.ends_with("bounds 1 face"),
        "{reason}"
    );
}

#[test]
fn a_group_holding_a_group_is_not_a_solid() {
    let mut d = doc();
    let outer = add(&mut d, [1.0, 1.0, 1.0], "group");
    run(&mut d, json!({"type": "open_instance", "id": outer}));
    add(&mut d, [1.0, 1.0, 1.0], "group");
    run(&mut d, json!({"type": "close_instance"}));
    let r = solid(&d, &outer);
    assert_eq!(r["solid"], false);
    assert!(
        r["reason"]
            .as_str()
            .unwrap()
            .contains("groups or components")
    );
}

#[test]
fn only_instances_can_be_asked() {
    let mut d = doc();
    add(&mut d, [1.0, 1.0, 1.0], "group");
    for (id, kind) in [
        ("v1", ErrorKind::InvalidInput),
        ("i99", ErrorKind::NotFound),
    ] {
        let q: Query = serde_json::from_value(json!({"type": "solid", "id": id})).unwrap();
        assert_eq!(d.query(&q).unwrap_err().kind, kind, "{id}");
    }
}
