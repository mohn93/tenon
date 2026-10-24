//! `chamfer_edges`: in the open context's own solid, or in an instance's definition.

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

#[test]
fn an_edge_of_the_open_group_is_chamfered_as_one_undo_step() {
    let mut d = doc();
    let b = add_box(&mut d, [2.0, 3.0, 4.0], "group");
    run(&mut d, json!({"type": "open_instance", "id": b}));
    let before = d.model().clone();
    let e = edge(d.model(), TOP_FRONT.0, TOP_FRONT.1);
    let far = edge(d.model(), [0.0, 3.0, 0.0], [2.0, 3.0, 0.0]);
    let out = run(
        &mut d,
        json!({"type": "chamfer_edges", "edges": [e], "distance": 0.5}),
    );
    let changes = &out["changes"];
    assert!(changes["deleted"].as_array().unwrap().contains(&e), "{out}");
    assert!(!changes["created"].as_array().unwrap().is_empty(), "{out}");
    let history = ask(&d, json!({"type": "history"}));
    assert_eq!(history["undo"][0], "chamfer_edges", "{history}");
    assert!(d.exists(serde_json::from_value(far.clone()).unwrap()));
    run(&mut d, json!({"type": "close_instance"}));
    assert!((volume(&d, &b) - 23.75).abs() < 1e-9);

    run(&mut d, json!({"type": "undo"}));
    assert_eq!(*mesh(&d, &b), before);
    run(&mut d, json!({"type": "redo"}));
    assert!((volume(&d, &b) - 23.75).abs() < 1e-9);
    d.scene().validate().unwrap();
}

#[test]
fn three_edges_at_a_corner_leave_a_closed_solid() {
    let mut d = doc();
    let b = add_box(&mut d, [2.0, 3.0, 4.0], "group");
    let m = mesh(&d, &b);
    let edges = [
        edge(m, [0.0, 0.0, 4.0], [2.0, 0.0, 4.0]),
        edge(m, [0.0, 0.0, 4.0], [0.0, 3.0, 4.0]),
        edge(m, [0.0, 0.0, 0.0], [0.0, 0.0, 4.0]),
    ];
    run(
        &mut d,
        json!({"type": "chamfer_edges", "instance": b, "edges": edges, "distance": 0.5}),
    );
    let v = volume(&d, &b);
    assert!(v > 24.0 - 0.125 * 9.0 && v < 24.0 - 0.125 * 7.5, "{v}");
}

#[test]
fn a_concave_edge_of_an_l_shape_gains_material() {
    let mut d = doc();
    let a = add_box(&mut d, [4.0, 2.0, 1.0], "group");
    let b = add_box(&mut d, [1.0, 2.0, 3.0], "group");
    let l = run(&mut d, json!({"type": "solid_union", "ids": [a, b]}))["placed"][0].clone();
    let before = volume(&d, &l);
    let e = edge(mesh(&d, &l), [1.0, 0.0, 1.0], [1.0, 2.0, 1.0]);
    run(
        &mut d,
        json!({"type": "chamfer_edges", "instance": l, "edges": [e], "distance": 0.25}),
    );
    assert!((volume(&d, &l) - (before + 0.0625)).abs() < 1e-9);
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
        json!({"type": "chamfer_edges", "instance": c, "edges": [e], "distance": 0.5}),
    );
    assert!((volume(&d, &c) - 23.75).abs() < 1e-9);
    assert!((volume(&d, &other) - 23.75).abs() < 1e-9);
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
fn unsuitable_edges_change_nothing() {
    let mut d = doc();
    let b = add_box(&mut d, [2.0, 3.0, 4.0], "group");
    let cyl = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [5.0, 0.0, 0.0], "radius": 1.0, "height": 2.0}),
    )["placed"][0]
        .clone();
    let m = mesh(&d, &b).clone();
    let e = edge(&m, TOP_FRONT.0, TOP_FRONT.1);
    let chamfer = |edges: Value, distance: f64| json!({"type": "chamfer_edges", "instance": b, "edges": edges, "distance": distance});
    refused(
        &mut d,
        chamfer(json!([e]), 3.0),
        ErrorKind::InvalidInput,
        "distance too large for edge",
    );
    refused(
        &mut d,
        chamfer(json!([e]), 0.0),
        ErrorKind::InvalidInput,
        "distance",
    );
    refused(
        &mut d,
        chamfer(json!([]), 0.1),
        ErrorKind::InvalidInput,
        "edges",
    );
    refused(
        &mut d,
        chamfer(json!([e, e]), 0.1),
        ErrorKind::InvalidInput,
        "twice",
    );
    let face = serde_json::to_value(EntityId::Face(m.faces().next().unwrap().0)).unwrap();
    refused(
        &mut d,
        chamfer(json!([face]), 0.1),
        ErrorKind::InvalidInput,
        "not an edge",
    );
    refused(
        &mut d,
        chamfer(json!(["e999999"]), 0.1),
        ErrorKind::NotFound,
        "e999999",
    );

    // Curve edges and the faces of a curved surface.
    let cm = mesh(&d, &cyl).clone();
    let curve = cm.edges().find(|(_, e)| e.curve.is_some()).unwrap().0;
    let soft = cm.edges().find(|(_, e)| e.soft).unwrap().0;
    for (id, text) in [(curve, "not straight"), (soft, "curved surface")] {
        let id = serde_json::to_value(EntityId::Edge(id)).unwrap();
        refused(
            &mut d,
            json!({"type": "chamfer_edges", "instance": cyl, "edges": [id], "distance": 0.1}),
            ErrorKind::InvalidInput,
            text,
        );
    }

    // A seam splitting the top face joins tangent faces.
    run(&mut d, json!({"type": "open_instance", "id": b}));
    run(
        &mut d,
        json!({"type": "add_edge", "start": [1.0, 0.0, 4.0], "end": [1.0, 3.0, 4.0]}),
    );
    let seam = edge(d.model(), [1.0, 0.0, 4.0], [1.0, 3.0, 4.0]);
    refused(
        &mut d,
        json!({"type": "chamfer_edges", "edges": [seam], "distance": 0.1}),
        ErrorKind::InvalidInput,
        "tangent",
    );

    // An open mesh is not a solid.
    let top = d.model().faces().find(|(_, f)| f.normal.z > 0.5).unwrap().0;
    run(
        &mut d,
        json!({"type": "erase", "ids": [EntityId::Face(top)]}),
    );
    let e = edge(d.model(), TOP_FRONT.0, [1.0, 0.0, 4.0]);
    refused(
        &mut d,
        json!({"type": "chamfer_edges", "edges": [e], "distance": 0.1}),
        ErrorKind::InvalidInput,
        "not a solid",
    );
    run(&mut d, json!({"type": "close_instance"}));
    refused(
        &mut d,
        json!({"type": "chamfer_edges", "instance": b, "edges": [e], "distance": 0.1}),
        ErrorKind::InvalidInput,
        "not a solid",
    );
}

#[test]
fn a_batch_names_the_instance_and_edges_by_reference() {
    let mut d = doc();
    let b = add_box(&mut d, [2.0, 3.0, 4.0], "group");
    let e = edge(mesh(&d, &b), TOP_FRONT.0, TOP_FRONT.1);
    let commands = vec![
        BatchCommand::parse(
            json!({"type": "add_box", "corner": [5.0, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
            0,
        )
        .unwrap(),
        BatchCommand::parse(
            json!({"type": "chamfer_edges", "instance": "$0:i0", "edges": ["$0:e*"],
                   "distance": 0.1}),
            1,
        )
        .unwrap(),
    ];
    // The box's edges are listed in its outcome: all twelve are chamfered.
    let outcomes = d.execute_batch(commands, true).unwrap();
    let small = outcomes[0].placed.as_ref().unwrap()[0];
    let small = serde_json::to_value(small).unwrap();
    let v = volume(&d, &small);
    let per = 0.1 * 0.1 / 2.0;
    assert!(v > 1.0 - per * 12.0 && v < 1.0 - per * 12.0 * 0.8, "{v}");
    let commands = vec![
        BatchCommand::parse(
            json!({"type": "chamfer_edges", "instance": b, "edges": [e], "distance": 0.5}),
            0,
        )
        .unwrap(),
    ];
    d.execute_batch(commands, true).unwrap();
    assert!((volume(&d, &b) - 23.75).abs() < 1e-9);
}
