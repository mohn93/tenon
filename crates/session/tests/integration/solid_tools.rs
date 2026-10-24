//! Solid Tools: `solid_union`, `solid_subtract`, `solid_intersect`, `solid_trim`, `solid_split`
//! and `outer_shell`.

use std::f64::consts::TAU;

use glam::DVec2;
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

fn instances(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities"}))["instances"]
        .as_array()
        .unwrap()
        .clone()
}

/// Instance `id` of the root.
fn instance(d: &Document, id: &Value) -> Instance {
    let id: EntityId = serde_json::from_value(id.clone()).unwrap();
    let EntityId::Instance(i) = id else {
        panic!("{id} is not an instance")
    };
    d.model().instance(i).unwrap().clone()
}

/// The definition mesh of instance `id` of the root.
fn mesh<'a>(d: &'a Document, id: &Value) -> &'a Model {
    let def = instance(d, id).definition;
    &d.scene().definition(def).unwrap().mesh
}

fn add_box(d: &mut Document, corner: [f64; 3], size: [f64; 3]) -> Value {
    run(
        d,
        json!({"type": "add_box", "corner": corner, "size": size}),
    )["placed"][0]
        .clone()
}

fn add_cylinder(
    d: &mut Document,
    base: [f64; 3],
    axis: [f64; 3],
    radius: f64,
    height: f64,
) -> Value {
    run(
        d,
        json!({"type": "add_cylinder", "base": base, "axis": axis, "radius": radius,
               "height": height}),
    )["placed"][0]
        .clone()
}

/// The area of a regular `n`-gon of circumradius `r`.
fn ngon(r: f64, n: u32) -> f64 {
    0.5 * n as f64 * r * r * (TAU / n as f64).sin()
}

#[test]
fn a_cylinder_bores_through_a_box_as_one_undo_step() {
    let mut d = doc();
    let plate = add_box(&mut d, [0.0; 3], [2.0, 2.0, 1.0]);
    let tool = add_cylinder(&mut d, [1.0, 1.0, -1.0], [0.0, 0.0, 1.0], 0.5, 3.0);
    let before = d.model().clone();
    let out = run(
        &mut d,
        json!({"type": "solid_subtract", "target": plate, "tools": [tool]}),
    );
    let placed = out["placed"].as_array().unwrap();
    assert_eq!(placed.len(), 1, "{out}");
    let result = placed[0].clone();
    assert!(out.get("empty").is_none());
    let deleted = out["changes"]["deleted"].as_array().unwrap();
    assert!(deleted.contains(&plate) && deleted.contains(&tool), "{out}");
    assert_eq!(instances(&d).len(), 1);
    assert!((volume(&d, &result) - (4.0 - ngon(0.5, 24))).abs() < 1e-9);
    let selection = ask(&d, json!({"type": "selection"}));
    assert_eq!(selection["ids"], json!([result]), "{selection}");
    let history = ask(&d, json!({"type": "history"}));
    assert_eq!(history["undo"][0], "solid_subtract", "{history}");

    run(&mut d, json!({"type": "undo"}));
    assert_eq!(*d.model(), before);
    assert_eq!(d.scene().definitions().count(), 2);
    run(&mut d, json!({"type": "redo"}));
    assert_eq!(instances(&d).len(), 1);
    d.scene().validate().unwrap();
}

#[test]
fn a_box_pocket_has_the_exact_volume() {
    let mut d = doc();
    let block = add_box(&mut d, [0.0; 3], [4.0, 4.0, 2.0]);
    let tool = add_box(&mut d, [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]);
    let out = run(
        &mut d,
        json!({"type": "solid_subtract", "target": block, "tools": [tool]}),
    );
    let result = out["placed"][0].clone();
    assert!((volume(&d, &result) - 28.0).abs() < 1e-9);
    assert_eq!(mesh(&d, &result).faces().count(), 11);
}

#[test]
fn boxes_sharing_a_face_unite_into_one_box() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0; 3], [1.0, 1.0, 1.0]);
    let b = add_box(&mut d, [1.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let out = run(&mut d, json!({"type": "solid_union", "ids": [a, b]}));
    let result = out["placed"][0].clone();
    assert!((volume(&d, &result) - 2.0).abs() < 1e-9);
    let m = mesh(&d, &result);
    assert_eq!(m.faces().count(), 6);
    assert_eq!(m.edges().count(), 12);
}

#[test]
fn crossing_cylinders_intersect() {
    let mut d = doc();
    let a = add_cylinder(&mut d, [0.0, 0.0, -2.0], [0.0, 0.0, 1.0], 1.0, 4.0);
    let b = add_cylinder(&mut d, [-2.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.8, 4.0);
    let (va, vb) = (volume(&d, &a), volume(&d, &b));
    let out = run(&mut d, json!({"type": "solid_intersect", "ids": [a, b]}));
    let common = volume(&d, &out["placed"][0]);
    assert!(common > 0.5 && common < va.min(vb), "{common}");
    // Union and difference agree with it.
    run(&mut d, json!({"type": "undo"}));
    let out = run(&mut d, json!({"type": "solid_union", "ids": [a, b]}));
    let union = volume(&d, &out["placed"][0]);
    assert!((va + vb - union - common).abs() < 1e-9);
}

#[test]
fn a_box_cuts_a_sphere_in_half() {
    let mut d = doc();
    let sphere = run(
        &mut d,
        json!({"type": "add_sphere", "center": [0, 0, 0], "radius": 1}),
    )["placed"][0]
        .clone();
    let whole = volume(&d, &sphere);
    let lid = add_box(&mut d, [-2.0, -2.0, 0.0], [4.0, 4.0, 2.0]);
    let out = run(
        &mut d,
        json!({"type": "solid_subtract", "target": sphere, "tools": [lid]}),
    );
    assert!((volume(&d, &out["placed"][0]) - whole / 2.0).abs() < 1e-9);
}

#[test]
fn twenty_bores_cut_a_plate_quickly() {
    let mut d = doc();
    let plate = add_box(&mut d, [0.0; 3], [10.0, 4.0, 0.5]);
    let tools: Vec<Value> = (0..20)
        .map(|i| {
            let x = 0.7 + (i % 10) as f64 * 0.9;
            let y = if i < 10 { 1.0 } else { 3.0 };
            add_cylinder(&mut d, [x, y, -0.5], [0.0, 0.0, 1.0], 0.3, 2.0)
        })
        .collect();
    let start = std::time::Instant::now();
    let out = run(
        &mut d,
        json!({"type": "solid_subtract", "target": plate, "tools": tools}),
    );
    let took = start.elapsed();
    assert!(took.as_secs_f64() < 2.0, "{took:?}");
    let v = volume(&d, &out["placed"][0]);
    assert!(
        (v - (20.0 - 20.0 * 0.5 * ngon(0.3, 24))).abs() < 1e-9,
        "{v}"
    );
    assert_eq!(instances(&d).len(), 1);
}

#[test]
fn materials_stay_on_their_sides() {
    let mut d = doc();
    let mut material = |name: &str| {
        run(
            &mut d,
            json!({"type": "add_material", "name": name, "color": [90, 90, 100]}),
        )["changes"]["materials"][0]
            .clone()
    };
    let (steel, red) = (material("Steel"), material("Red"));
    let block = run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [4, 4, 2], "material": steel}),
    )["placed"][0]
        .clone();
    let tool = run(
        &mut d,
        json!({"type": "add_box", "corner": [1, 1, 1], "size": [2, 2, 2], "material": red}),
    )["placed"][0]
        .clone();
    let out = run(
        &mut d,
        json!({"type": "solid_subtract", "target": block, "tools": [tool]}),
    );
    let result = out["placed"][0].clone();
    let steel: kernel::MaterialId = serde_json::from_value(steel).unwrap();
    assert_eq!(instance(&d, &result).material, Some(steel));
    let red: kernel::MaterialId = serde_json::from_value(red).unwrap();
    let m = mesh(&d, &result);
    let cut: Vec<_> = m.faces().filter(|(_, f)| f.front == Some(red)).collect();
    assert_eq!(cut.len(), 5);
    // The pocket's floor faces up, out of the block, the tool's material on its front.
    assert!(cut.iter().any(|(_, f)| f.normal.z > 0.99));
    assert!(
        m.faces()
            .filter(|(_, f)| f.front != Some(red))
            .all(|(_, f)| f.front.is_none() && f.back.is_none())
    );
}

#[test]
fn the_result_takes_the_first_operands_place_and_name() {
    let mut d = doc();
    let a = run(
        &mut d,
        json!({"type": "add_box", "corner": [3, 0, 0], "size": [1, 1, 1], "name": "Base"}),
    )["placed"][0]
        .clone();
    let b = add_box(&mut d, [3.5, 0.5, 0.5], [1.0, 1.0, 1.0]);
    let before = instance(&d, &a);
    let out = run(&mut d, json!({"type": "solid_union", "ids": [a, b]}));
    let after = instance(&d, &out["placed"][0]);
    assert_eq!(after.transform, before.transform);
    let definition = d.scene().definition(after.definition).unwrap();
    assert_eq!(definition.name, "Base");
    assert_eq!(definition.kind, kernel::DefinitionKind::Group);
    assert!((volume(&d, &out["placed"][0]) - (2.0 - 0.125)).abs() < 1e-9);
}

#[test]
fn an_empty_result_erases_the_operands() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0; 3], [1.0, 1.0, 1.0]);
    let b = add_box(&mut d, [5.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let out = run(&mut d, json!({"type": "solid_intersect", "ids": [a, b]}));
    assert_eq!(out["empty"], true, "{out}");
    assert_eq!(out["placed"], json!([]), "{out}");
    assert!(instances(&d).is_empty());
    run(&mut d, json!({"type": "undo"}));
    assert_eq!(instances(&d).len(), 2);

    let big = add_box(&mut d, [-1.0; 3], [3.0, 3.0, 3.0]);
    let out = run(
        &mut d,
        json!({"type": "solid_subtract", "target": a, "tools": [big]}),
    );
    assert_eq!(out["empty"], true, "{out}");
    assert_eq!(instances(&d).len(), 1);
}

#[test]
fn refusals_change_nothing() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0; 3], [1.0, 1.0, 1.0]);
    let b = add_box(&mut d, [1.0, 1.0, 0.0], [1.0, 1.0, 1.0]);
    let open = add_box(&mut d, [5.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    run(&mut d, json!({"type": "open_instance", "id": open}));
    let face = d.model().faces().next().unwrap().0;
    run(
        &mut d,
        json!({"type": "erase", "ids": [format!("f{}", face.0)]}),
    );
    run(&mut d, json!({"type": "close_instance"}));
    let revision = d.revision();
    let cases = [
        (
            json!({"type": "solid_union", "ids": [a, open]}),
            ErrorKind::InvalidInput,
            "is not a solid: edge e",
        ),
        (
            // Meeting only along an edge: the union would not be closed.
            json!({"type": "solid_union", "ids": [a, b]}),
            ErrorKind::InvalidInput,
            "the result would not be a solid",
        ),
        (
            json!({"type": "solid_union", "ids": [a]}),
            ErrorKind::InvalidInput,
            "at least 2",
        ),
        (
            json!({"type": "solid_intersect", "ids": [a, a]}),
            ErrorKind::InvalidInput,
            "named twice",
        ),
        (
            json!({"type": "solid_subtract", "target": a, "tools": []}),
            ErrorKind::InvalidInput,
            "at least 1",
        ),
        (
            json!({"type": "solid_subtract", "target": a, "tools": [b, a]}),
            ErrorKind::InvalidInput,
            "cannot also be a tool",
        ),
        (
            json!({"type": "solid_union", "ids": [a, "i999"]}),
            ErrorKind::NotFound,
            "",
        ),
        (
            json!({"type": "solid_trim", "target": a, "tools": []}),
            ErrorKind::InvalidInput,
            "at least 1",
        ),
        (
            json!({"type": "solid_trim", "target": a, "tools": [b, a]}),
            ErrorKind::InvalidInput,
            "cannot also be a tool",
        ),
        (
            json!({"type": "solid_trim", "target": a, "tools": [open]}),
            ErrorKind::InvalidInput,
            "is not a solid",
        ),
        (
            json!({"type": "solid_split", "a": a, "b": a}),
            ErrorKind::InvalidInput,
            "named twice",
        ),
        (
            json!({"type": "solid_split", "a": a, "b": open}),
            ErrorKind::InvalidInput,
            "is not a solid",
        ),
        (
            json!({"type": "outer_shell", "ids": []}),
            ErrorKind::InvalidInput,
            "at least 1",
        ),
        (
            json!({"type": "outer_shell", "ids": [open]}),
            ErrorKind::InvalidInput,
            "is not a solid",
        ),
        (
            json!({"type": "outer_shell", "ids": [a, b]}),
            ErrorKind::InvalidInput,
            "the result would not be a solid",
        ),
    ];
    for (command, kind, text) in cases {
        let e = try_run(&mut d, command.clone()).unwrap_err();
        assert_eq!(e.kind, kind, "{command} {e:?}");
        assert!(e.message.contains(text), "{command} {e:?}");
        assert_eq!(d.revision(), revision, "{command}");
    }
    let e = try_run(&mut d, json!({"type": "solid_union", "ids": [a, "e1"]}));
    assert!(e.is_err());
    assert_eq!(d.revision(), revision);
}

#[test]
fn a_batch_refers_to_target_and_tools() {
    let mut d = doc();
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [2, 2, 1]}),
        json!({"type": "add_cylinder", "base": [1, 1, -1], "radius": 0.5, "height": 3}),
        json!({"type": "solid_subtract", "target": "$0:i0", "tools": ["$1:i0"]}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let outcomes = d.execute_batch(commands, true).unwrap();
    let placed = outcomes[2].placed.clone().unwrap();
    assert_eq!(placed.len(), 1);
    let id = serde_json::to_value(placed[0]).unwrap();
    assert!((volume(&d, &id) - (4.0 - ngon(0.5, 24))).abs() < 1e-9);
}

fn ids_of(out: &Value) -> Vec<Value> {
    out["placed"].as_array().unwrap().clone()
}

#[test]
fn trim_keeps_the_tools() {
    let mut d = doc();
    let block = add_box(&mut d, [0.0; 3], [4.0, 4.0, 2.0]);
    let tool = add_box(&mut d, [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]);
    let before = d.model().clone();
    let out = run(
        &mut d,
        json!({"type": "solid_trim", "target": block, "tools": [tool]}),
    );
    let placed = ids_of(&out);
    assert_eq!(placed.len(), 1, "{out}");
    let deleted = out["changes"]["deleted"].as_array().unwrap();
    assert!(
        deleted.contains(&block) && !deleted.contains(&tool),
        "{out}"
    );
    assert_eq!(instances(&d).len(), 2);
    assert!((volume(&d, &placed[0]) - 28.0).abs() < 1e-9);
    // The tool is as it was, still a solid of its own volume.
    assert!((volume(&d, &tool) - 8.0).abs() < 1e-9);
    let history = ask(&d, json!({"type": "history"}));
    assert_eq!(history["undo"][0], "solid_trim", "{history}");
    run(&mut d, json!({"type": "undo"}));
    assert_eq!(*d.model(), before);
    d.scene().validate().unwrap();

    // A tool that removes everything leaves the tool and erases the target.
    let big = add_box(&mut d, [-1.0; 3], [6.0, 6.0, 4.0]);
    let out = run(
        &mut d,
        json!({"type": "solid_trim", "target": block, "tools": [big]}),
    );
    assert_eq!(out["empty"], true, "{out}");
    assert_eq!(instances(&d).len(), 2);
}

#[test]
fn split_makes_three_pieces_in_order() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0; 3], [2.0, 2.0, 2.0]);
    let b = add_box(&mut d, [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]);
    let before = d.model().clone();
    let out = run(&mut d, json!({"type": "solid_split", "a": a, "b": b}));
    let placed = ids_of(&out);
    assert_eq!(placed.len(), 3, "{out}");
    let volumes: Vec<f64> = placed.iter().map(|p| volume(&d, p)).collect();
    assert!((volumes[0] - 7.0).abs() < 1e-9, "{volumes:?}");
    assert!((volumes[1] - 1.0).abs() < 1e-9, "{volumes:?}");
    assert!((volumes[2] - 7.0).abs() < 1e-9, "{volumes:?}");
    let deleted = out["changes"]["deleted"].as_array().unwrap();
    assert!(deleted.contains(&a) && deleted.contains(&b), "{out}");
    assert_eq!(instances(&d).len(), 3);
    let selection = ask(&d, json!({"type": "selection"}));
    assert_eq!(selection["ids"].as_array().unwrap().len(), 3, "{selection}");
    // They add up to the union.
    let union = run(&mut d, json!({"type": "solid_union", "ids": placed}));
    assert!((volume(&d, &union["placed"][0]) - 15.0).abs() < 1e-9);
    run(&mut d, json!({"type": "undo"}));
    run(&mut d, json!({"type": "undo"}));
    assert_eq!(*d.model(), before);
    d.scene().validate().unwrap();
}

#[test]
fn split_without_overlap_makes_two_pieces() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0; 3], [1.0, 1.0, 1.0]);
    let b = add_box(&mut d, [5.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
    let out = run(&mut d, json!({"type": "solid_split", "a": a, "b": b}));
    let placed = ids_of(&out);
    assert_eq!(placed.len(), 2, "{out}");
    assert!((volume(&d, &placed[0]) - 1.0).abs() < 1e-9);
    assert!((volume(&d, &placed[1]) - 2.0).abs() < 1e-9);
    assert!(out.get("empty").is_none());
}

#[test]
fn outer_shell_of_overlapping_boxes_is_their_union() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0; 3], [2.0, 2.0, 2.0]);
    let b = add_box(&mut d, [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]);
    let out = run(&mut d, json!({"type": "outer_shell", "ids": [a, b]}));
    let placed = ids_of(&out);
    assert_eq!(placed.len(), 1);
    assert!((volume(&d, &placed[0]) - 15.0).abs() < 1e-9);
    assert_eq!(instances(&d).len(), 1);
    let history = ask(&d, json!({"type": "history"}));
    assert_eq!(history["undo"][0], "outer_shell", "{history}");
}

#[test]
fn outer_shell_removes_an_internal_void() {
    let mut d = doc();
    let block = add_box(&mut d, [0.0; 3], [4.0, 4.0, 4.0]);
    let core = add_box(&mut d, [1.5, 1.5, 1.5], [1.0, 1.0, 1.0]);
    let hollow = run(
        &mut d,
        json!({"type": "solid_subtract", "target": block, "tools": [core]}),
    )["placed"][0]
        .clone();
    assert!((volume(&d, &hollow) - 63.0).abs() < 1e-9);
    assert_eq!(mesh(&d, &hollow).faces().count(), 12);
    let before = d.model().clone();
    let out = run(&mut d, json!({"type": "outer_shell", "ids": [hollow]}));
    let solid = ids_of(&out)[0].clone();
    assert!((volume(&d, &solid) - 64.0).abs() < 1e-9);
    assert_eq!(mesh(&d, &solid).faces().count(), 6);
    run(&mut d, json!({"type": "undo"}));
    assert_eq!(*d.model(), before);
}

#[test]
fn a_batch_refers_to_trim_split_and_shell_operands() {
    let mut d = doc();
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [2, 2, 2]}),
        json!({"type": "add_box", "corner": [1, 1, 1], "size": [2, 2, 2]}),
        json!({"type": "solid_trim", "target": "$0:i0", "tools": ["$1:i0"]}),
        json!({"type": "solid_split", "a": "$2:i0", "b": "$1:i0"}),
        json!({"type": "outer_shell", "ids": ["$3:i0"]}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    let outcomes = d.execute_batch(commands, true).unwrap();
    assert_eq!(outcomes[2].placed.clone().unwrap().len(), 1);
    assert_eq!(outcomes[3].placed.clone().unwrap().len(), 2);
    assert_eq!(outcomes[4].placed.clone().unwrap().len(), 1);
}

#[test]
fn solid_operations_report_the_new_definitions() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0; 3], [2.0, 2.0, 2.0]);
    let b = add_box(&mut d, [1.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
    let out = run(&mut d, json!({"type": "solid_union", "ids": [a, b]}));
    let defs = out["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "{out}");
    let result = out["placed"][0].clone();
    assert_eq!(
        instance(&d, &result).definition.to_string(),
        defs[0].as_str().unwrap()
    );

    // Split places up to three groups, each with its own definition, in `placed` order.
    let c = add_box(&mut d, [10.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
    let e = add_box(&mut d, [11.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
    let out = run(&mut d, json!({"type": "solid_split", "a": c, "b": e}));
    assert_eq!(out["definitions"].as_array().unwrap().len(), 3, "{out}");

    // An empty result names no definition.
    let f = add_box(&mut d, [30.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let g = add_box(&mut d, [40.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let out = run(&mut d, json!({"type": "solid_intersect", "ids": [f, g]}));
    assert_eq!(out["empty"], true);
    assert!(out.get("definitions").is_none(), "{out}");

    // A batch can use `$N:d0`.
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [2, 2, 2]}),
        json!({"type": "add_box", "corner": [1, 0, 0], "size": [2, 2, 2]}),
        json!({"type": "solid_union", "ids": ["$0:i0", "$1:i0"]}),
        json!({"type": "rename_definition", "definition": "$2:d0", "name": "Joined"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
}
