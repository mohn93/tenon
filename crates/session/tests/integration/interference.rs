//! The `interference` and `clearance` queries.

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

fn add_box(d: &mut Document, corner: [f64; 3], size: [f64; 3]) -> Value {
    run(
        d,
        json!({"type": "add_box", "corner": corner, "size": size, "as": "group"}),
    )["placed"][0]
        .clone()
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

#[test]
fn overlapping_boxes_report_their_volume_and_change_nothing() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
    let b = add_box(&mut d, [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]);
    let c = add_box(&mut d, [0.0, 0.0, -1.0], [3.0, 3.0, 0.5]);
    let before = ask(&d, json!({"type": "history"}));
    let r = ask(&d, json!({"type": "interference"}));
    assert_eq!(r["type"], "interference");
    let pairs = r["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!((&pairs[0]["a"], &pairs[0]["b"]), (&a, &b));
    assert!((f(&pairs[0]["volume"]) - 1.0).abs() < 1e-9);
    let p: Vec<f64> = pairs[0]["point"]
        .as_array()
        .unwrap()
        .iter()
        .map(f)
        .collect();
    assert!(p.iter().all(|x| *x > 1.0 && *x < 2.0), "{p:?}");
    assert!(r.get("touching").is_none());
    assert!(r.get("failed").is_none());
    assert_eq!(r["skipped"], json!([]));
    assert_eq!(ask(&d, json!({"type": "history"})), before);
    // Only the parts named.
    let r = ask(&d, json!({"type": "interference", "ids": [a, c]}));
    assert_eq!(r["pairs"], json!([]));
}

#[test]
fn touching_parts_are_listed_only_when_asked() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let b = add_box(&mut d, [1.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let r = ask(&d, json!({"type": "interference"}));
    assert_eq!(r["pairs"], json!([]));
    let r = ask(&d, json!({"type": "interference", "touching": true}));
    let t = r["touching"].as_array().unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!((&t[0]["a"], &t[0]["b"]), (&a, &b));
    assert!((f(&t[0]["point"][0]) - 1.0).abs() < 1e-9);
    let c = ask(&d, json!({"type": "clearance", "a": a, "b": b}));
    assert_eq!(f(&c["pairs"][0]["distance"]), 0.0);
    assert_eq!(c["pairs"][0]["overlap"], false);
}

#[test]
fn a_pin_in_its_bore_has_its_clearance() {
    let mut d = doc();
    let tube = run(
        &mut d,
        json!({"type": "add_tube", "base": [0, 0, 0], "radius": 0.02,
               "inner_radius": 0.0101, "height": 0.03, "as": "group"}),
    )["placed"][0]
        .clone();
    let pin = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [0, 0, -0.01], "radius": 0.01,
               "height": 0.05, "as": "group"}),
    )["placed"][0]
        .clone();
    let r = ask(&d, json!({"type": "interference", "touching": true}));
    assert_eq!(r["pairs"], json!([]));
    assert_eq!(r["touching"], json!([]));
    let c = ask(&d, json!({"type": "clearance", "a": tube, "b": pin}));
    let pair = &c["pairs"][0];
    let expected = 0.0001 * (std::f64::consts::PI / 24.0).cos();
    assert!((f(&pair["distance"]) - expected).abs() < 1e-12, "{pair}");
    assert_eq!(pair["overlap"], false);
    let gap: f64 = (0..3)
        .map(|i| (f(&pair["point_a"][i]) - f(&pair["point_b"][i])).powi(2))
        .sum::<f64>()
        .sqrt();
    assert!((gap - expected).abs() < 1e-12);
}

#[test]
fn a_part_inside_another_interferes_and_has_no_clearance() {
    let mut d = doc();
    let big = add_box(&mut d, [0.0, 0.0, 0.0], [4.0, 4.0, 4.0]);
    let small = add_box(&mut d, [1.0, 1.0, 1.0], [1.0, 2.0, 1.0]);
    let r = ask(&d, json!({"type": "interference"}));
    assert!((f(&r["pairs"][0]["volume"]) - 2.0).abs() < 1e-9);
    let c = ask(&d, json!({"type": "clearance", "ids": [big, small]}));
    assert_eq!(f(&c["pairs"][0]["distance"]), 0.0);
    assert_eq!(c["pairs"][0]["overlap"], true);
}

#[test]
fn clearance_among_ids_gives_every_pair() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let b = add_box(&mut d, [2.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let c = add_box(&mut d, [0.0, 0.0, 4.0], [1.0, 1.0, 1.0]);
    let r = ask(&d, json!({"type": "clearance", "ids": [a, b, c]}));
    let pairs = r["pairs"].as_array().unwrap();
    let got: Vec<(Value, Value, f64)> = pairs
        .iter()
        .map(|p| (p["a"].clone(), p["b"].clone(), f(&p["distance"])))
        .collect();
    let want_bc = 10.0f64.sqrt();
    assert_eq!(got.len(), 3);
    assert_eq!((&got[0].0, &got[0].1), (&a, &b));
    assert!((got[0].2 - 1.0).abs() < 1e-12);
    assert!((got[1].2 - 3.0).abs() < 1e-12);
    assert!((got[2].2 - want_bc).abs() < 1e-9, "{}", got[2].2);
}

#[test]
fn parts_that_are_not_solids_are_skipped() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let b = add_box(&mut d, [0.5, 0.0, 0.0], [1.0, 1.0, 1.0]);
    run(&mut d, json!({"type": "open_instance", "id": b}));
    let face = d.model().faces().next().unwrap().0;
    run(
        &mut d,
        json!({"type": "erase", "ids": [format!("f{}", face.0)]}),
    );
    run(&mut d, json!({"type": "close_instance"}));
    let r = ask(&d, json!({"type": "interference"}));
    assert_eq!(r["pairs"], json!([]));
    assert_eq!(r["skipped"][0]["id"], b);
    let reason = r["skipped"][0]["reason"].as_str().unwrap();
    assert!(reason.contains("bounds 1 face"), "{reason}");
    let c = ask(&d, json!({"type": "clearance", "a": a, "b": b}));
    assert_eq!(c["pairs"], json!([]));
    assert_eq!(c["skipped"][0]["id"], b);
}

#[test]
fn bad_ids_are_refused() {
    let mut d = doc();
    let a = add_box(&mut d, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    assert_eq!(
        ask_err(&d, json!({"type": "interference", "ids": ["f1"]})),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask_err(&d, json!({"type": "interference", "ids": ["i999"]})),
        ErrorKind::NotFound
    );
    assert_eq!(
        ask_err(&d, json!({"type": "clearance", "a": a})),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask_err(&d, json!({"type": "clearance", "a": a, "b": a})),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask_err(&d, json!({"type": "clearance", "ids": [a]})),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask_err(&d, json!({"type": "clearance", "a": a, "b": "i999"})),
        ErrorKind::NotFound
    );
}

/// A slider-crank of boxes stacked in Z (crank, rod, piston), the crank turning about Z at
/// the origin with its pin at x = 0.05 and the piston's wrist at x = 0.2; a valve ahead of
/// the piston that it reaches 40 degrees either side of top dead centre. Returns the parts
/// [crank, rod, piston, valve] and the crank's joint.
fn slider_crank(d: &mut Document) -> ([Value; 4], Value) {
    let (r, l, h) = (0.05f64, 0.15f64, 0.02f64);
    let t = 40f64.to_radians();
    let valve_x = r * t.cos() + (l * l - r * r * t.sin().powi(2)).sqrt() + h;
    let crank = add_box(d, [-0.01, -0.01, 0.0], [r + 0.02, 0.02, 0.01]);
    let rod = add_box(d, [r - 0.005, -0.005, 0.01], [l + 0.01, 0.01, 0.01]);
    let piston = add_box(d, [r + l - h, -0.02, 0.02], [2.0 * h, 0.04, 0.03]);
    let valve = add_box(d, [valve_x, -0.01, 0.02], [0.03, 0.02, 0.03]);
    let mut join = |kind: &str, a: &Value, b: &Value, x: f64, axis: [f64; 3]| {
        run(
            d,
            json!({"type": "add_joint", "kind": kind, "a": a, "b": b, "point": [x, 0.0, 0.0], "axis": axis, "reference": [0, 1, 0]}),
        )["changes"]["joints"][0]
            .clone()
    };
    let z = [0.0, 0.0, 1.0];
    let crank_joint = join("revolute", &Value::Null, &crank, 0.0, z);
    join("revolute", &crank, &rod, r, z);
    join("revolute", &rod, &piston, r + l, z);
    join("prismatic", &Value::Null, &piston, r + l, [1.0, 0.0, 0.0]);
    ([crank, rod, piston, valve], crank_joint)
}

#[test]
fn a_sweep_reports_where_the_piston_hits_the_valve_and_changes_nothing() {
    let mut d = doc();
    let ([crank, rod, piston, valve], j) = slider_crank(&mut d);
    let before = ask(&d, json!({"type": "history"}));
    let revision = d.revision();
    let transforms = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    let r = ask(
        &d,
        json!({"type": "sweep_check", "joint": j, "from": 90, "to": 450, "steps": 37}),
    );
    assert_eq!(r["type"], "sweep_check");
    assert_eq!(r["steps"], 37);
    assert!(r.get("stopped").is_none());
    let pairs = r["pairs"].as_array().unwrap();
    let find = |a: &Value, b: &Value| {
        pairs
            .iter()
            .find(|p| (&p["a"], &p["b"]) == (a, b) || (&p["a"], &p["b"]) == (b, a))
            .cloned()
    };
    // Degrees, within one 10 degree step of 320 and 400.
    let hit = find(&piston, &valve).unwrap();
    let (first, last) = (f(&hit["first"]), f(&hit["last"]));
    assert!((320.0..=330.0).contains(&first), "{first}");
    assert!((390.0..=400.0).contains(&last), "{last}");
    assert!(f(&hit["volume"]) > 0.0);
    assert!((f(&hit["volume_at"]) - 360.0).abs() < 1e-9);
    assert_eq!(f(&hit["clearance"]), 0.0);
    // Stacked layers touch: listed with clearance 0 and no first or last.
    let touch = find(&crank, &rod).unwrap();
    assert!(touch.get("first").is_none());
    assert_eq!(f(&touch["clearance"]), 0.0);
    // The crank never comes near the valve.
    assert!(find(&crank, &valve).is_none());
    assert_eq!(ask(&d, json!({"type": "history"})), before);
    assert_eq!(d.revision(), revision);
    assert_eq!(r["revision"], json!(revision));
    assert_eq!(
        ask(&d, json!({"type": "entities", "kinds": ["instance"]})),
        transforms
    );
    // Only the parts named; a larger report distance.
    let r = ask(
        &d,
        json!({"type": "sweep_check", "joint": j, "from": 0, "to": 360, "steps": 5, "ids": [crank, piston], "report_within": 0.1}),
    );
    // Nearest at 180 degrees: the crank's near end (x = 0.01, z up to 0.01) and the piston's
    // back (x = 0.08, z from 0.02).
    let pairs = r["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1);
    let near = 0.07f64.hypot(0.01);
    assert!(
        (f(&pairs[0]["clearance"]) - near).abs() < 1e-9,
        "{}",
        pairs[0]
    );
    assert!((f(&pairs[0]["clearance_at"]) - 180.0).abs() < 1e-9);
}

#[test]
fn a_sweep_stops_at_a_value_the_solver_cannot_meet() {
    let mut d = doc();
    let (_, j) = slider_crank(&mut d);
    run(
        &mut d,
        json!({"type": "change_joint", "joint": j, "limits": [-45, 45]}),
    );
    let r = ask(
        &d,
        json!({"type": "sweep_check", "joint": j, "from": 0, "to": 90, "steps": 10}),
    );
    assert!((f(&r["stopped"]["value"]) - 50.0).abs() < 1e-9, "{r}");
    assert!(r["stopped"]["reason"].as_str().unwrap().contains("limits"));
    assert_eq!(r["steps"], 5);
}

#[test]
fn bad_sweeps_are_refused() {
    let mut d = doc();
    let ([crank, ..], j) = slider_crank(&mut d);
    let base = json!({"type": "sweep_check", "joint": j, "from": 0, "to": 90, "steps": 10});
    let with = |k: &str, v: Value| {
        let mut q = base.clone();
        q[k] = v;
        q
    };
    assert_eq!(
        ask_err(&d, with("steps", json!(1))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask_err(&d, with("steps", json!(10_001))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask_err(&d, with("report_within", json!(-1.0))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask_err(&d, with("slide", json!(0.1))),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        ask_err(&d, with("joint", json!("j9999"))),
        ErrorKind::NotFound
    );
    assert_eq!(
        ask_err(&d, with("ids", json!(["i9999"]))),
        ErrorKind::NotFound
    );
    assert_eq!(
        ask_err(&d, with("ids", json!(["e1"]))),
        ErrorKind::InvalidInput
    );
    let r = ask(&d, with("ids", json!([crank])));
    assert_eq!(r["pairs"], json!([]));
}
