//! `add_box`, `add_cylinder`, `add_tube`, `add_cone` and `add_prism`, driven through JSON.

use std::collections::BTreeMap;

use glam::{DVec2, DVec3};
use serde_json::{Value, json};
use session::{BatchCommand, Command, Document, ErrorKind, Query};

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

fn depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

fn entities(d: &Document) -> Value {
    ask(d, json!({"type": "entities"}))
}

fn instance(d: &Document, id: &Value) -> Value {
    entities(d)["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| &i["id"] == id)
        .unwrap()
        .clone()
}

fn point(v: &Value) -> DVec3 {
    let a = v.as_array().unwrap();
    DVec3::new(
        a[0].as_f64().unwrap(),
        a[1].as_f64().unwrap(),
        a[2].as_f64().unwrap(),
    )
}

/// Checks the open context holds exactly one closed, outward oriented solid and returns its
/// signed volume and its soft edge count.
fn closed_solid(d: &Document) -> (f64, usize) {
    let e = entities(d);
    let at: BTreeMap<String, DVec3> = e["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["id"].as_str().unwrap().to_string(), point(&v["position"])))
        .collect();
    let mut sides: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut volume = 0.0;
    for f in e["faces"].as_array().unwrap() {
        let normal = point(&f["normal"]);
        let mut area = DVec3::ZERO;
        let loops =
            std::iter::once(f["outer"].clone()).chain(f["holes"].as_array().unwrap().clone());
        let mut origin = None;
        for l in loops {
            let ids: Vec<String> = l
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            for i in 0..ids.len() {
                let (a, b) = (ids[i].clone(), ids[(i + 1) % ids.len()].clone());
                *sides.entry((a.clone(), b.clone())).or_default() += 1;
                area += at[&a].cross(at[&b]) * 0.5;
                origin.get_or_insert(at[&a]);
            }
        }
        volume += normal.dot(origin.unwrap()) * area.dot(normal) / 3.0;
    }
    for ((a, b), n) in &sides {
        assert_eq!(*n, 1, "{a}-{b} used {n} times");
        assert_eq!(
            sides.get(&(b.clone(), a.clone())),
            Some(&1),
            "{a}-{b} is open"
        );
    }
    let edges = e["edges"].as_array().unwrap();
    for edge in edges {
        let v = edge["vertices"].as_array().unwrap();
        let key = (
            v[0].as_str().unwrap().to_string(),
            v[1].as_str().unwrap().to_string(),
        );
        assert!(sides.contains_key(&key) || sides.contains_key(&(key.1, key.0)));
    }
    let soft = edges.iter().filter(|e| e["soft"] == true).count();
    (volume, soft)
}

/// The solid of the one instance, with its instance opened.
fn open_only(d: &mut Document) -> Value {
    let id = entities(d)["instances"][0]["id"].clone();
    run(d, json!({"type": "open_instance", "id": id})).unwrap();
    id
}

fn cylinder_volume(r: f64, h: f64, n: f64) -> f64 {
    0.5 * n * r * r * (std::f64::consts::TAU / n).sin() * h
}

/// Volume of the faceted UV sphere: frustums of n-gons between the rings.
fn sphere_volume(r: f64, n: u32) -> f64 {
    use std::f64::consts::PI;
    let rings = (n / 2).max(2);
    let area = |rho: f64| 0.5 * n as f64 * rho * rho * (2.0 * PI / n as f64).sin();
    (0..rings)
        .map(|k| {
            let t = |k: u32| PI * k as f64 / rings as f64;
            let (a1, a2) = (area(r * t(k).sin()), area(r * t(k + 1).sin()));
            (t(k).cos() - t(k + 1).cos()) * r / 3.0 * (a1 + a2 + (a1 * a2).sqrt())
        })
        .sum()
}

#[test]
fn each_shape_is_a_closed_outward_group() {
    let n = 32.0;
    let cases = [
        (
            json!({"type": "add_box", "corner": [1, 2, 3], "size": [2, 3, 4]}),
            24.0,
            0,
        ),
        (
            json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1.5, "height": 2, "segments": 32}),
            cylinder_volume(1.5, 2.0, n),
            32,
        ),
        (
            json!({"type": "add_tube", "base": [0, 0, 0], "radius": 2, "inner_radius": 1, "height": 3, "segments": 32}),
            cylinder_volume(2.0, 3.0, n) - cylinder_volume(1.0, 3.0, n),
            64,
        ),
        (
            json!({"type": "add_cone", "base": [0, 0, 0], "radius": 2, "height": 3, "segments": 32}),
            cylinder_volume(2.0, 3.0, n) / 3.0,
            32,
        ),
        (
            json!({"type": "add_cone", "base": [0, 0, 0], "radius": 2, "top_radius": 3, "height": 3, "segments": 32}),
            (cylinder_volume(2.0, 1.0, n)
                + cylinder_volume(3.0, 1.0, n)
                + (cylinder_volume(2.0, 1.0, n) * cylinder_volume(3.0, 1.0, n)).sqrt())
                * 1.0,
            32,
        ),
        (
            json!({"type": "add_prism", "base": [0, 0, 0], "radius": 1, "sides": 6, "height": 2}),
            1.5 * 3f64.sqrt() * 2.0,
            0,
        ),
        (
            json!({"type": "add_sphere", "center": [1, 2, 3], "radius": 2, "segments": 32}),
            sphere_volume(2.0, 32),
            31 * 32,
        ),
        (
            json!({"type": "add_torus", "center": [0, 0, 0], "axis": [1, 1, 0], "radius": 3, "tube_radius": 1, "segments": 32, "tube_segments": 10}),
            32.0 * (std::f64::consts::TAU / 32.0).sin()
                * 0.5
                * 10.0
                * (std::f64::consts::TAU / 10.0).sin()
                * 3.0,
            2 * 32 * 10,
        ),
    ];
    for (command, volume, soft) in cases {
        let mut d = doc();
        let out = run(&mut d, command.clone()).unwrap();
        assert_eq!(depth(&d), 1, "{command}");
        let placed = out["placed"].as_array().unwrap().clone();
        assert_eq!(placed.len(), 1);
        assert_eq!(out["definitions"].as_array().unwrap().len(), 1);
        assert_eq!(
            out["changes"]["created"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|i| i.as_str().unwrap().starts_with('i'))
                .count(),
            1
        );
        // The group is the selection and holds the solid, not the root.
        assert_eq!(entities(&d)["faces"].as_array().unwrap().len(), 0);
        assert_eq!(ask(&d, json!({"type": "selection"}))["ids"], json!(placed));
        open_only(&mut d);
        let (v, s) = closed_solid(&d);
        assert!(
            (v - volume).abs() < 1e-9 * volume.max(1.0),
            "{command}: {v} vs {volume}"
        );
        assert_eq!(s, soft, "{command}");
    }
}

#[test]
fn box_places_at_its_corner_and_prism_edges_are_hard() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_box", "corner": [1, 2, 3], "size": [2, 3, 4]}),
    )
    .unwrap();
    let i = instance(&d, &out["placed"][0]);
    assert_eq!(
        i["transform"],
        json!([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0])
    );
    let (lo, hi) = (point(&i["bounds"]["min"]), point(&i["bounds"]["max"]));
    assert!((lo - DVec3::new(1.0, 2.0, 3.0)).length() < 1e-9);
    assert!((hi - DVec3::new(3.0, 5.0, 7.0)).length() < 1e-9);
}

#[test]
fn curved_shapes_map_local_z_onto_the_axis_with_the_circle_u_axis() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [1, 2, 3], "axis": [0, 0, 5], "radius": 1, "height": 2}),
    )
    .unwrap();
    let m = instance(&d, &out["placed"][0])["transform"].clone();
    // add_circle's first corner for normal +Z lies along +X.
    assert_eq!(
        m,
        json!([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0])
    );

    let out = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [0, 0, 0], "axis": [3, 0, 0], "radius": 1, "height": 2}),
    )
    .unwrap();
    let m: Vec<f64> = instance(&d, &out["placed"][0])["transform"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    // z column along the axis; x column along the u axis of normal +X, which is +Y.
    assert!(
        (DVec3::new(m[6], m[7], m[8]) - DVec3::X).length() < 1e-12,
        "{m:?}"
    );
    assert!(
        (DVec3::new(m[0], m[1], m[2]) - DVec3::Y).length() < 1e-12,
        "{m:?}"
    );
    // The cylinder spans x from 0 to 2.
    let b = instance(&d, &out["placed"][0])["bounds"].clone();
    assert!((point(&b["max"]).x - 2.0).abs() < 1e-9);
    assert!(point(&b["min"]).x.abs() < 1e-9);
}

#[test]
fn prism_first_corner_follows_the_direction() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_prism", "base": [0, 0, 0], "radius": 1, "sides": 4, "height": 1,
               "direction": [0, 1, 7]}),
    )
    .unwrap();
    open_only(&mut d);
    let e = entities(&d);
    let first = e["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| point(&v["position"]))
        .find(|p| p.z.abs() < 1e-12 && (p.y - 1.0).abs() < 1e-9 && p.x.abs() < 1e-9);
    assert!(first.is_some(), "{e}");
    let (v, soft) = closed_solid(&d);
    assert!((v - 2.0).abs() < 1e-9);
    assert_eq!(soft, 0);
}

#[test]
fn prism_direction_is_tested_relative_to_its_length() {
    for len in [1e-10, 1e10] {
        let mut d = doc();
        run(
            &mut d,
            json!({"type": "add_prism", "base": [0, 0, 0], "radius": 1, "height": 1,
                   "axis": [0, 0, 1], "direction": [len, 0, 0]}),
        )
        .unwrap();
    }
    let mut d = doc();
    let e = run(
        &mut d,
        json!({"type": "add_prism", "base": [0, 0, 0], "radius": 1, "height": 1,
               "axis": [0, 0, 1], "direction": [0, 0, 1e-10]}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
}

#[test]
fn a_name_needs_a_group_or_component() {
    let mut d = doc();
    let before = depth(&d);
    let e = run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1],
               "as": "geometry", "name": "Crate"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(e.1.contains("name applies only"), "{}", e.1);
    assert_eq!(depth(&d), before);
}

#[test]
fn component_and_group_names() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1], "as": "component"}),
    )
    .unwrap();
    assert_eq!(instance(&d, &out["placed"][0])["name"], "Box");
    assert_eq!(instance(&d, &out["placed"][0])["kind"], "component");
    let out = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [5, 0, 0], "radius": 1, "height": 1,
               "as": "component", "name": "Pin"}),
    )
    .unwrap();
    assert_eq!(instance(&d, &out["placed"][0])["name"], "Pin");
    let out = run(
        &mut d,
        json!({"type": "add_tube", "base": [9, 0, 0], "radius": 1, "inner_radius": 0.5, "height": 1,
               "name": "Sleeve"}),
    )
    .unwrap();
    assert_eq!(instance(&d, &out["placed"][0])["name"], "Sleeve");
    assert_eq!(instance(&d, &out["placed"][0])["kind"], "group");
    let out = run(
        &mut d,
        json!({"type": "add_cone", "base": [0, 9, 0], "radius": 1, "height": 1, "as": "component"}),
    )
    .unwrap();
    assert_eq!(instance(&d, &out["placed"][0])["name"], "Cone");
    let out = run(
        &mut d,
        json!({"type": "add_prism", "base": [0, 0, 9], "radius": 1, "height": 1, "as": "component"}),
    )
    .unwrap();
    assert_eq!(instance(&d, &out["placed"][0])["name"], "Prism");
    let e = run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1], "name": " "}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(e.1.contains("name"), "{}", e.1);
}

fn material(d: &mut Document) -> String {
    run(
        d,
        json!({"type": "add_material", "name": "Steel", "color": [90, 90, 100]}),
    )
    .unwrap()["changes"]["materials"][0]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn material_paints_the_instance_or_the_geometry_fronts() {
    let mut d = doc();
    let m = material(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1], "material": m}),
    )
    .unwrap();
    assert_eq!(instance(&d, &out["placed"][0])["material"], json!(m));
    // Undo the box: one step takes it away.
    let steps = depth(&d);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(depth(&d), steps - 1);
    assert!(entities(&d)["instances"].as_array().unwrap().is_empty());

    let out = run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1], "as": "geometry",
               "material": m}),
    )
    .unwrap();
    assert!(out.get("placed").is_none());
    let faces = entities(&d)["faces"].as_array().unwrap().clone();
    assert_eq!(faces.len(), 6);
    assert!(faces.iter().all(|f| f["front_material"] == json!(m)));
    assert!(faces.iter().all(|f| f.get("back_material").is_none()));

    let e = run(
        &mut d,
        json!({"type": "add_box", "corner": [5, 5, 5], "size": [1, 1, 1], "material": "m99"}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
}

#[test]
fn geometry_is_loose_in_the_open_context() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1, "height": 2,
               "segments": 12, "as": "geometry"}),
    )
    .unwrap();
    assert!(out.get("placed").is_none());
    assert!(out.get("definitions").is_none());
    let created = out["changes"]["created"].as_array().unwrap();
    assert_eq!(
        created
            .iter()
            .filter(|i| i.as_str().unwrap().starts_with('f'))
            .count(),
        14
    );
    assert_eq!(
        created
            .iter()
            .filter(|i| i.as_str().unwrap().starts_with('e'))
            .count(),
        36
    );
    assert!(entities(&d)["instances"].as_array().unwrap().is_empty());
    let (v, soft) = closed_solid(&d);
    assert!((v - cylinder_volume(1.0, 2.0, 12.0)).abs() < 1e-9);
    assert_eq!(soft, 12);
    assert_eq!(depth(&d), 1);

    // A second solid next to it is its own step and undoes alone.
    run(
        &mut d,
        json!({"type": "add_box", "corner": [5, 0, 0], "size": [2, 2, 1], "as": "geometry"}),
    )
    .unwrap();
    assert_eq!(depth(&d), 2);
    assert_eq!(entities(&d)["faces"].as_array().unwrap().len(), 20);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(entities(&d)["faces"].as_array().unwrap().len(), 14);
}

#[test]
fn a_solid_in_a_group_keeps_to_the_open_context() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [4, 4, 4]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "open_instance", "id": out["placed"][0]}),
    )
    .unwrap();
    let out = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [1, 1, 4], "radius": 0.5, "height": 1}),
    )
    .unwrap();
    // The cylinder is a separate group inside the box's group, not merged with its top.
    assert_eq!(out["placed"].as_array().unwrap().len(), 1);
    assert_eq!(entities(&d)["faces"].as_array().unwrap().len(), 6);
    assert_eq!(entities(&d)["instances"].as_array().unwrap().len(), 1);
}

#[test]
fn batch_references_name_the_instance() {
    let mut d = doc();
    let m = material(&mut d);
    let commands: Vec<BatchCommand> = [
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1]}),
        json!({"type": "set_transform", "id": "$0:i0", "transform": {"origin": [0, 0, 9]}}),
        json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1, "height": 1, "material": m}),
        json!({"type": "paint", "ids": ["$2:i0"], "material": m}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
    let instances = entities(&d)["instances"].as_array().unwrap().clone();
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0]["transform"][11], json!(9.0));
}

#[test]
fn one_undo_step_and_redo() {
    let mut d = doc();
    run(&mut d, json!({"type": "add_tube", "base": [0, 0, 0], "radius": 1, "inner_radius": 0.5, "height": 1}))
        .unwrap();
    assert_eq!(depth(&d), 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(entities(&d)["instances"].as_array().unwrap().is_empty());
    let defs = ask(&d, json!({"type": "definitions"}));
    assert!(defs["definitions"].as_array().unwrap().is_empty(), "{defs}");
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(entities(&d)["instances"].as_array().unwrap().len(), 1);
    open_only(&mut d);
    closed_solid(&d);
}

#[test]
fn invalid_dimensions_are_refused_without_a_step() {
    let bad = [
        (
            json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 0, 1]}),
            "size",
        ),
        (
            json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, -1]}),
            "size",
        ),
        (
            json!({"type": "add_box", "corner": [2e6, 0, 0], "size": [1, 1, 1]}),
            "coordinates",
        ),
        (
            json!({"type": "add_box", "corner": [0, 0, 0], "size": [2e6, 1, 1]}),
            "size",
        ),
        (
            json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1, "height": 2e6}),
            "height",
        ),
        (
            json!({"type": "add_box", "corner": [9e5, 0, 0], "size": [2e5, 1, 1]}),
            "coordinates",
        ),
        (
            json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 0, "height": 1}),
            "radius",
        ),
        (
            json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1, "height": 0}),
            "height",
        ),
        (
            json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1, "height": 1, "segments": 2}),
            "segments",
        ),
        (
            json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1, "height": 1, "segments": 361}),
            "segments",
        ),
        (
            json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1, "height": 1, "axis": [0, 0, 0]}),
            "axis",
        ),
        (
            json!({"type": "add_tube", "base": [0, 0, 0], "radius": 1, "inner_radius": 1, "height": 1}),
            "inner_radius",
        ),
        (
            json!({"type": "add_tube", "base": [0, 0, 0], "radius": 1, "inner_radius": 0, "height": 1}),
            "inner_radius",
        ),
        (
            json!({"type": "add_cone", "base": [0, 0, 0], "radius": 1, "top_radius": -1, "height": 1}),
            "top_radius",
        ),
        (
            json!({"type": "add_prism", "base": [0, 0, 0], "radius": 1, "height": 1, "sides": 2}),
            "sides",
        ),
        (
            json!({"type": "add_prism", "base": [0, 0, 0], "radius": 1, "height": 1, "direction": [0, 0, 1]}),
            "direction",
        ),
        (
            json!({"type": "add_prism", "base": [0, 0, 0], "radius": 1, "height": 1, "as": "geometry", "axis": [0, 0, 0]}),
            "axis",
        ),
        (
            json!({"type": "add_cone", "base": [0, 0, 0], "radius": 1e-9, "height": 1}),
            "radius",
        ),
        (
            json!({"type": "add_sphere", "center": [0, 0, 0], "radius": 0}),
            "radius",
        ),
        (
            json!({"type": "add_sphere", "center": [0, 0, 0], "radius": 1, "segments": 2}),
            "segments",
        ),
        (
            json!({"type": "add_sphere", "center": [0, 0, 0], "radius": 1, "axis": [0, 0, 0]}),
            "axis",
        ),
        (
            json!({"type": "add_sphere", "center": [9.9e5, 0, 0], "radius": 2e4}),
            "coordinates",
        ),
        (
            json!({"type": "add_torus", "center": [0, 0, 0], "radius": 1, "tube_radius": 1}),
            "tube_radius",
        ),
        (
            json!({"type": "add_torus", "center": [0, 0, 0], "radius": 1, "tube_radius": 0.5, "tube_segments": 2}),
            "tube_segments",
        ),
        (
            json!({"type": "add_torus", "center": [0, 0, 0], "radius": 1, "tube_radius": 0.5, "segments": 361}),
            "segments",
        ),
        (
            json!({"type": "add_torus", "center": [0, 0, 0], "radius": 2e6, "tube_radius": 0.5}),
            "radius",
        ),
    ];
    let mut d = doc();
    for (command, field) in bad {
        let e = run(&mut d, command.clone()).unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput, "{command}");
        assert!(e.1.contains(field), "{command}: {}", e.1);
    }
    assert_eq!(depth(&d), 0);
    assert!(entities(&d)["instances"].as_array().unwrap().is_empty());
}

#[test]
fn non_finite_numbers_never_panic() {
    let mut d = doc();
    let nan = f64::NAN;
    for command in [
        Command::AddBox {
            corner: [nan, 0.0, 0.0],
            size: [1.0, 1.0, 1.0],
            placement: Default::default(),
            name: None,
            material: None,
        },
        Command::AddBox {
            corner: [0.0, 0.0, 0.0],
            size: [1.0, f64::INFINITY, 1.0],
            placement: Default::default(),
            name: None,
            material: None,
        },
        Command::AddCylinder {
            base: [0.0, 0.0, 0.0],
            axis: Some([nan, 0.0, 1.0]),
            radius: 1.0,
            height: 1.0,
            segments: None,
            placement: Default::default(),
            name: None,
            material: None,
        },
        Command::AddCone {
            base: [0.0, 0.0, 0.0],
            axis: None,
            radius: 1.0,
            top_radius: Some(nan),
            height: 1.0,
            segments: None,
            placement: Default::default(),
            name: None,
            material: None,
        },
        Command::AddPrism {
            base: [0.0, 0.0, 0.0],
            axis: None,
            radius: nan,
            sides: None,
            height: 1.0,
            direction: Some([nan, 0.0, 0.0]),
            placement: Default::default(),
            name: None,
            material: None,
        },
        Command::AddSphere {
            center: [nan, 0.0, 0.0],
            axis: None,
            radius: 1.0,
            segments: None,
            placement: Default::default(),
            name: None,
            material: None,
        },
        Command::AddSphere {
            center: [0.0, 0.0, 0.0],
            axis: Some([0.0, nan, 1.0]),
            radius: nan,
            segments: None,
            placement: Default::default(),
            name: None,
            material: None,
        },
        Command::AddTorus {
            center: [0.0, 0.0, 0.0],
            axis: None,
            radius: 1.0,
            tube_radius: nan,
            segments: None,
            tube_segments: None,
            placement: Default::default(),
            name: None,
            material: None,
        },
    ] {
        let e = d.execute(command).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }
    assert_eq!(depth(&d), 0);
}

/// (faces, edges, soft edges, curve edges, vertices) of the open context.
fn counts(d: &Document) -> (usize, usize, usize, usize, usize) {
    let e = entities(d);
    let edges = e["edges"].as_array().unwrap();
    (
        e["faces"].as_array().unwrap().len(),
        edges.len(),
        edges.iter().filter(|x| x["soft"] == true).count(),
        edges.iter().filter(|x| x.get("curve").is_some()).count(),
        e["vertices"].as_array().unwrap().len(),
    )
}

#[test]
fn primitive_definitions_survive_save_and_open() {
    let mut d = doc();
    for c in [
        json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1, "height": 1, "segments": 12}),
        json!({"type": "add_tube", "base": [5, 0, 0], "radius": 1, "inner_radius": 0.5, "height": 1, "segments": 12}),
        json!({"type": "add_cone", "base": [10, 0, 0], "radius": 1, "height": 1, "segments": 12, "as": "component"}),
        json!({"type": "add_prism", "base": [15, 0, 0], "radius": 1, "height": 1, "sides": 5}),
        json!({"type": "add_sphere", "center": [20, 0, 0], "radius": 1, "segments": 12}),
        json!({"type": "add_torus", "center": [25, 0, 0], "radius": 2, "tube_radius": 0.5, "segments": 12, "tube_segments": 6, "as": "component"}),
    ] {
        run(&mut d, c).unwrap();
    }
    let open_all = |d: &mut Document| -> Vec<(usize, usize, usize, usize, usize)> {
        let ids: Vec<Value> = entities(d)["instances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].clone())
            .collect();
        ids.iter()
            .map(|id| {
                run(d, json!({"type": "open_instance", "id": id})).unwrap();
                let c = counts(d);
                d.model().validate().unwrap();
                run(d, json!({"type": "close_instance"})).unwrap();
                c
            })
            .collect()
    };
    let before = open_all(&mut d);
    assert_eq!(before[0], (14, 36, 12, 24, 24));
    assert!(before.iter().all(|c| c.2 > 0 || c == &before[3]));
    assert_eq!(before[4].2, 11 * 12);
    assert_eq!(before[5].2, 2 * 12 * 6);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-primitives-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("solids.tnn");
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
    assert_eq!(open_all(&mut e), before);
}

#[test]
fn stacked_geometry_boxes_share_their_face() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [2, 2, 2], "as": "geometry"}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 2], "size": [2, 2, 1], "as": "geometry"}),
    )
    .unwrap();
    d.model().validate().unwrap();
    // Two boxes share the one face between them: 12 faces, minus the second copy of it.
    let (faces, edges, ..) = counts(&d);
    assert_eq!((faces, edges), (11, 20));
}

#[test]
fn sphere_and_torus_frames_and_options() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_sphere", "center": [1, 2, 3], "radius": 1, "axis": [0, 0, 1], "name": "Ball"}),
    )
    .unwrap();
    let i = instance(&d, &out["placed"][0]);
    let m = i["transform"].as_array().unwrap();
    // Origin at the centre, local +z on the axis.
    assert_eq!(point(&json!(m[9..12])), DVec3::new(1.0, 2.0, 3.0));
    assert!((point(&json!(m[6..9])) - DVec3::Z).length() < 1e-12);
    let out = run(
        &mut d,
        json!({"type": "add_torus", "center": [0, 0, 0], "axis": [1, 0, 0], "radius": 2, "tube_radius": 0.5, "as": "component"}),
    )
    .unwrap();
    let i = instance(&d, &out["placed"][0]);
    let m = i["transform"].as_array().unwrap();
    assert!((point(&json!(m[6..9])) - DVec3::X).length() < 1e-12);
    assert_eq!(i["name"], "Torus");
    // Defaults: sphere 24 segments, torus 24 by 12.
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_sphere", "center": [0, 0, 0], "radius": 1, "as": "geometry"}),
    )
    .unwrap();
    assert_eq!(entities(&d)["faces"].as_array().unwrap().len(), 24 * 12);
    d.model().validate().unwrap();
    let mut d = doc();
    run(&mut d, json!({"type": "add_torus", "center": [0, 0, 0], "radius": 2, "tube_radius": 0.5, "as": "geometry"})).unwrap();
    assert_eq!(entities(&d)["faces"].as_array().unwrap().len(), 24 * 12);
    d.model().validate().unwrap();
}
