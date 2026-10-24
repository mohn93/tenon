//! `fillet_edges`: in the open context's own solid, or in an instance's definition.

use std::f64::consts::FRAC_PI_2;

use glam::{DVec2, DVec3};
use kernel::{EntityId, Instance, Model};
use serde_json::{Value, json};
use session::{BatchCommand, Command, CommandError, Document, ErrorKind, Query};

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

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn volume(d: &Document, id: &Value) -> f64 {
    let r = ask(d, json!({"type": "solid", "id": id}));
    assert_eq!(r["solid"], true, "{r}");
    r["volume"].as_f64().unwrap()
}

fn instance(d: &Document, id: &Value) -> Instance {
    let id: EntityId = serde_json::from_value(id.clone()).unwrap();
    let EntityId::Instance(i) = id else {
        panic!("{id} is not an instance")
    };
    d.model().instance(i).unwrap().clone()
}

fn mesh<'a>(d: &'a Document, id: &Value) -> &'a Model {
    let def = instance(d, id).definition;
    &d.scene().definition(def).unwrap().mesh
}

fn add_box(d: &mut Document, size: [f64; 3], placement: &str) -> Value {
    run(
        d,
        json!({"type": "add_box", "corner": [0.0, 0.0, 0.0], "size": size, "as": placement}),
    )["placed"][0]
        .clone()
}

/// The id (as JSON) of the edge of `m` from `p` to `q`.
fn edge(m: &Model, p: [f64; 3], q: [f64; 3]) -> Value {
    let (p, q) = (DVec3::from(p), DVec3::from(q));
    let id = m
        .edges()
        .find(|(_, e)| {
            let (a, b) = (m.position(e.v[0]).unwrap(), m.position(e.v[1]).unwrap());
            (a.distance(p) < 1e-9 && b.distance(q) < 1e-9)
                || (a.distance(q) < 1e-9 && b.distance(p) < 1e-9)
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no edge {p} {q}"));
    serde_json::to_value(EntityId::Edge(id)).unwrap()
}

const TOP_FRONT: ([f64; 3], [f64; 3]) = ([0.0, 0.0, 4.0], [2.0, 0.0, 4.0]);

/// What a quarter-turn fillet of `r` in `n` faces removes per meter of edge.
fn quarter_loss(r: f64, n: usize) -> f64 {
    let step = FRAC_PI_2 / n as f64;
    r * r - n as f64 * r * r * step.sin() / 2.0
}

#[test]
fn an_edge_of_the_open_group_is_rounded_as_one_undo_step() {
    let mut d = doc();
    let b = add_box(&mut d, [2.0, 3.0, 4.0], "group");
    run(&mut d, json!({"type": "open_instance", "id": b}));
    let before = d.model().clone();
    let e = edge(d.model(), TOP_FRONT.0, TOP_FRONT.1);
    let out = run(
        &mut d,
        json!({"type": "fillet_edges", "edges": [e], "radius": 0.5}),
    );
    let changes = &out["changes"];
    assert!(changes["deleted"].as_array().unwrap().contains(&e), "{out}");
    // The 8 round faces are made.
    let created = changes["created"].as_array().unwrap();
    let faces = created
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with('f'))
        .count();
    assert!(faces >= 8, "{out}");
    let history = ask(&d, json!({"type": "history"}));
    assert_eq!(history["undo"][0], "fillet_edges", "{history}");
    // The round's edges and the two where it meets top and front are soft.
    let soft = d.model().edges().filter(|(_, e)| e.soft).count();
    assert_eq!(soft, 9);
    run(&mut d, json!({"type": "close_instance"}));
    let rounded = 24.0 - quarter_loss(0.5, 8) * 2.0;
    assert!((volume(&d, &b) - rounded).abs() < 1e-9);

    run(&mut d, json!({"type": "undo"}));
    assert_eq!(*mesh(&d, &b), before);
    run(&mut d, json!({"type": "redo"}));
    assert!((volume(&d, &b) - rounded).abs() < 1e-9);
    d.scene().validate().unwrap();
}

#[test]
fn segments_set_the_faces_per_quarter_turn() {
    let mut d = doc();
    let b = add_box(&mut d, [2.0, 3.0, 4.0], "group");
    let e = edge(mesh(&d, &b), TOP_FRONT.0, TOP_FRONT.1);
    run(
        &mut d,
        json!({"type": "fillet_edges", "instance": b, "edges": [e], "radius": 0.5,
               "segments": 3}),
    );
    assert_eq!(mesh(&d, &b).faces().count(), 6 + 3);
    assert_eq!(mesh(&d, &b).edges().filter(|(_, e)| e.soft).count(), 4);
    assert!((volume(&d, &b) - (24.0 - quarter_loss(0.5, 3) * 2.0)).abs() < 1e-9);
}

#[test]
fn all_twelve_edges_of_a_box_stay_a_closed_solid() {
    let mut d = doc();
    let b = add_box(&mut d, [2.0, 3.0, 4.0], "group");
    let edges: Vec<Value> = mesh(&d, &b)
        .edges()
        .map(|(id, _)| serde_json::to_value(EntityId::Edge(id)).unwrap())
        .collect();
    let r: f64 = 0.25;
    run(
        &mut d,
        json!({"type": "fillet_edges", "instance": b, "edges": edges, "radius": r}),
    );
    let v = volume(&d, &b);
    let sharp = 24.0 - quarter_loss(r, 8) * 4.0 * (9.0 - 6.0 * r);
    let spherical = sharp - 8.0 * r.powi(3) * (1.0 - std::f64::consts::PI / 6.0);
    assert!(v < sharp && v > spherical, "{spherical} < {v} < {sharp}");
}

#[test]
fn a_concave_edge_of_an_l_shape_gains_a_cove() {
    let mut d = doc();
    let a = add_box(&mut d, [4.0, 2.0, 1.0], "group");
    let b = add_box(&mut d, [1.0, 2.0, 3.0], "group");
    let l = run(&mut d, json!({"type": "solid_union", "ids": [a, b]}))["placed"][0].clone();
    let before = volume(&d, &l);
    let e = edge(mesh(&d, &l), [1.0, 0.0, 1.0], [1.0, 2.0, 1.0]);
    run(
        &mut d,
        json!({"type": "fillet_edges", "instance": l, "edges": [e], "radius": 0.25}),
    );
    assert!((volume(&d, &l) - (before + quarter_loss(0.25, 8) * 2.0)).abs() < 1e-9);
    assert_eq!(mesh(&d, &l).edges().filter(|(_, e)| e.soft).count(), 9);
}

#[test]
fn the_instance_form_edits_a_components_definition() {
    let mut d = doc();
    let c = add_box(&mut d, [2.0, 3.0, 4.0], "component");
    let def = instance(&d, &c).definition;
    let other = run(
        &mut d,
        json!({"type": "place_instance", "definition": def,
               "transform": {"origin": [10.0, 0.0, 0.0]}}),
    )["placed"][0]
        .clone();
    let e = edge(mesh(&d, &c), TOP_FRONT.0, TOP_FRONT.1);
    run(
        &mut d,
        json!({"type": "fillet_edges", "instance": c, "edges": [e], "radius": 0.5}),
    );
    let rounded = 24.0 - quarter_loss(0.5, 8) * 2.0;
    assert!((volume(&d, &c) - rounded).abs() < 1e-9);
    assert!((volume(&d, &other) - rounded).abs() < 1e-9);
    run(&mut d, json!({"type": "undo"}));
    assert!((volume(&d, &other) - 24.0).abs() < 1e-9);
}

fn refused(d: &mut Document, command: Value, kind: ErrorKind, text: &str) {
    let revision = d.revision();
    let scene = d.scene().clone();
    let err = try_run(d, command).unwrap_err();
    assert_eq!(err.kind, kind, "{err:?}");
    assert!(err.message.contains(text), "{}", err.message);
    assert_eq!(d.revision(), revision);
    assert_eq!(*d.scene(), scene);
}

#[test]
fn unsuitable_fillets_change_nothing() {
    let mut d = doc();
    let b = add_box(&mut d, [2.0, 3.0, 4.0], "group");
    let e = edge(mesh(&d, &b), TOP_FRONT.0, TOP_FRONT.1);
    let fillet = |radius: f64, segments: Value| {
        json!({"type": "fillet_edges", "instance": b, "edges": [e], "radius": radius,
               "segments": segments})
    };
    refused(
        &mut d,
        fillet(3.0, json!(8)),
        ErrorKind::InvalidInput,
        "radius too large for edge",
    );
    refused(
        &mut d,
        fillet(0.0, json!(8)),
        ErrorKind::InvalidInput,
        "radius",
    );
    for segments in [0, 91] {
        refused(
            &mut d,
            fillet(0.1, json!(segments)),
            ErrorKind::InvalidInput,
            "segments must be 1 to 90",
        );
    }
    refused(
        &mut d,
        json!({"type": "fillet_edges", "instance": b, "edges": [], "radius": 0.1}),
        ErrorKind::InvalidInput,
        "edges",
    );
    refused(
        &mut d,
        json!({"type": "fillet_edges", "instance": b, "edges": ["e999999"], "radius": 0.1}),
        ErrorKind::NotFound,
        "e999999",
    );
    // Edges facing each other across a 1 m face: their cuts would meet.
    let cube = add_box(&mut d, [1.0, 1.0, 1.0], "group");
    let m = mesh(&d, &cube).clone();
    let front = edge(&m, [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]);
    let back = edge(&m, [0.0, 1.0, 1.0], [1.0, 1.0, 1.0]);
    for (kind, size) in [("fillet_edges", "radius"), ("chamfer_edges", "distance")] {
        let revision = d.revision();
        let err = try_run(
            &mut d,
            json!({"type": kind, "instance": cube, "edges": [front, back], size: 0.7}),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert!(
            err.message
                .contains(&format!("{size} too large for edges e")),
            "{}",
            err.message
        );
        let ids = serde_json::to_value(&err.ids).unwrap();
        assert!(ids.as_array().unwrap().contains(&front), "{ids}");
        assert!(ids.as_array().unwrap().contains(&back), "{ids}");
        assert_eq!(d.revision(), revision);
    }
    // Missing ids are listed once.
    let err = try_run(
        &mut d,
        json!({"type": "fillet_edges", "instance": b, "edges": ["e999999", "e999999"],
               "radius": 0.1}),
    )
    .unwrap_err();
    assert_eq!(err.message.matches("e999999").count(), 1, "{}", err.message);
}

#[test]
fn a_rounded_box_rounds_its_top_in_a_second_command() {
    let mut d = doc();
    let b = add_box(&mut d, [2.0, 3.0, 4.0], "group");
    let m = mesh(&d, &b).clone();
    let id = |e| serde_json::to_value(EntityId::Edge(e)).unwrap();
    let upright: Vec<Value> = m
        .edges()
        .filter(|(_, e)| {
            let (p, q) = (m.position(e.v[0]).unwrap(), m.position(e.v[1]).unwrap());
            (p - q).truncate().length() < 1e-9
        })
        .map(|(e, _)| id(e))
        .collect();
    run(
        &mut d,
        json!({"type": "fillet_edges", "instance": b, "edges": upright, "radius": 0.25,
               "segments": 4}),
    );
    let once = volume(&d, &b);
    let m = mesh(&d, &b).clone();
    let top = m
        .faces()
        .find(|(_, f)| f.normal.z > 0.5 && m.position(f.outer[0]).unwrap().z > 3.9)
        .unwrap()
        .0;
    let edges: Vec<Value> = m.face_edges(top).into_iter().map(id).collect();
    assert_eq!(edges.len(), 20);
    run(
        &mut d,
        json!({"type": "fillet_edges", "instance": b, "edges": edges, "radius": 0.2}),
    );
    let removed = once - volume(&d, &b);
    let loss = quarter_loss(0.2, 8);
    assert!(removed > loss * 8.0 && removed < loss * 9.6, "{removed}");
    d.scene().validate().unwrap();

    // A chamfer beside the rounds.
    let m = mesh(&d, &b).clone();
    let bottom = edge(&m, [0.25, 0.0, 0.0], [1.75, 0.0, 0.0]);
    let before = volume(&d, &b);
    run(
        &mut d,
        json!({"type": "chamfer_edges", "instance": b, "edges": [bottom], "distance": 0.1}),
    );
    let v = volume(&d, &b);
    assert!(v < before && v > before - 0.005 * 2.0, "{v}");
}

#[test]
fn a_batch_names_the_instance_and_edges_by_reference() {
    let mut d = doc();
    let commands = vec![
        BatchCommand::parse(
            json!({"type": "add_box", "corner": [5.0, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
            0,
        )
        .unwrap(),
        BatchCommand::parse(
            json!({"type": "fillet_edges", "instance": "$0:i0", "edges": ["$0:e*"],
                   "radius": 0.1, "segments": 2}),
            1,
        )
        .unwrap(),
    ];
    let outcomes = d.execute_batch(commands, true).unwrap();
    let small = outcomes[0].placed.as_ref().unwrap()[0];
    let small = serde_json::to_value(small).unwrap();
    let v = volume(&d, &small);
    let r: f64 = 0.1;
    let sharp = 1.0 - quarter_loss(r, 2) * 4.0 * (3.0 - 6.0 * r);
    assert!(v < sharp && v > sharp - 8.0 * r.powi(3), "{v}");
}
