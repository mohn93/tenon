//! The `measure` query (distance, angle, round), driven through JSON.

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

fn measure(d: &Document, q: Value) -> Result<Value, (ErrorKind, String)> {
    let mut q = q;
    q["type"] = json!("measure");
    let q: Query = serde_json::from_value(q).expect("valid query json");
    d.query(&q)
        .map(|r| serde_json::to_value(r).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn entities(d: &Document) -> Value {
    let q: Query = serde_json::from_value(json!({"type": "entities"})).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn near(v: &Value, want: f64) -> bool {
    (v.as_f64().unwrap() - want).abs() < 1e-9
}

fn point_near(v: &Value, want: [f64; 3]) -> bool {
    (0..3).all(|i| near(&v[i], want[i]))
}

fn boxed(d: &mut Document, at: [f64; 3], size: [f64; 3]) -> Value {
    run(
        d,
        json!({"type": "add_box", "corner": at, "size": size, "as": "group"}),
    )
    .unwrap()["placed"][0]
        .clone()
}

fn edge(d: &mut Document, a: [f64; 3], b: [f64; 3]) -> Value {
    let out = run(d, json!({"type": "add_edge", "start": a, "end": b})).unwrap();
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('e'))
        .unwrap()
        .clone()
}

#[test]
fn distance_between_two_groups_is_the_gap() {
    let mut d = doc();
    let a = boxed(&mut d, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let b = boxed(&mut d, [3.0, 0.2, 0.2], [1.0, 1.0, 1.0]);
    let r = measure(&d, json!({"distance": [a, b]})).unwrap();
    assert!(near(&r["distance"]["distance"], 2.0), "{r}");
    assert!(near(&r["distance"]["points"][0][0], 1.0));
    assert!(near(&r["distance"]["points"][1][0], 3.0));
    // Touching: zero.
    let c = boxed(&mut d, [1.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    let r = measure(&d, json!({"distance": [a, c]})).unwrap();
    assert!(near(&r["distance"]["distance"], 0.0));
}

#[test]
fn skew_segments_and_vertices() {
    let mut d = doc();
    let e1 = edge(&mut d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
    let e2 = edge(&mut d, [1.0, -1.0, 1.0], [1.0, 1.0, 1.0]);
    let r = measure(&d, json!({"distance": [e1, e2]})).unwrap();
    assert!(near(&r["distance"]["distance"], 1.0), "{r}");
    assert!(point_near(&r["distance"]["points"][0], [1.0, 0.0, 0.0]));
    assert!(point_near(&r["distance"]["points"][1], [1.0, 0.0, 1.0]));
    // Segments whose closest points are end points.
    let e3 = edge(&mut d, [5.0, 5.0, 0.0], [5.0, 8.0, 0.0]);
    let r = measure(&d, json!({"distance": [e1, e3]})).unwrap();
    assert!(
        near(&r["distance"]["distance"], (9.0f64 + 25.0).sqrt()),
        "{r}"
    );
    // Vertex to edge.
    let ents = entities(&d);
    let v = ents["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == e3)
        .unwrap()["vertices"][1]
        .clone();
    let r = measure(&d, json!({"distance": [v, e1]})).unwrap();
    assert!(near(&r["distance"]["distance"], 73.0f64.sqrt()), "{r}");
}

#[test]
fn segment_to_polygon_inside_outside_and_hole() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 4, 0]}),
    )
    .unwrap();
    let face = entities(&d)["faces"][0]["id"].clone();
    // Above the interior.
    let over = edge(&mut d, [1.0, 1.0, 2.0], [3.0, 1.0, 2.0]);
    let r = measure(&d, json!({"distance": [over, face]})).unwrap();
    assert!(near(&r["distance"]["distance"], 2.0), "{r}");
    assert!(near(&r["distance"]["points"][1][2], 0.0));
    // Slanting down through it: zero.
    let through = edge(&mut d, [2.0, 2.0, 1.0], [2.0, 2.0, -1.0]);
    let r = measure(&d, json!({"distance": [through, face]})).unwrap();
    assert!(near(&r["distance"]["distance"], 0.0), "{r}");
    // Beside it, above the plane: nearest point is the polygon's edge.
    let beside = edge(&mut d, [6.0, 1.0, 3.0], [6.0, 3.0, 3.0]);
    let r = measure(&d, json!({"distance": [beside, face]})).unwrap();
    assert!(near(&r["distance"]["distance"], 13.0f64.sqrt()), "{r}");
    assert!(point_near(&r["distance"]["points"][1], [4.0, 1.0, 0.0]));
    // A hole: a circle drawn inside the face.
    run(
        &mut d,
        json!({"type": "add_circle", "center": [2, 2, 0], "radius": 1.0, "segments": 64}),
    )
    .unwrap();
    let ents = entities(&d);
    let holed = ents["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| !f["holes"].as_array().unwrap().is_empty())
        .expect("face with a hole")["id"]
        .clone();
    let r = measure(&d, json!({"distance": [through, holed]})).unwrap();
    assert!(!near(&r["distance"]["distance"], 0.0), "{r}");
    assert!((r["distance"]["distance"].as_f64().unwrap() - 1.0).abs() < 0.01);
}

#[test]
fn nested_paths_use_the_world_placement() {
    let mut d = doc();
    let outer = boxed(&mut d, [0.0, 0.0, 0.0], [4.0, 4.0, 4.0]);
    run(&mut d, json!({"type": "open_instance", "id": outer})).unwrap();
    run(
        &mut d,
        json!({"type": "add_box", "corner": [1, 1, 1], "size": [1, 1, 1], "as": "group"}),
    )
    .unwrap();
    let inner = entities(&d)["instances"][0]["id"].clone();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    run(
        &mut d,
        json!({"type": "set_transform", "id": outer,
               "transform": {"matrix": [2, 0, 0, 0, 2, 0, 0, 0, 2, 10, 0, 0]}}),
    )
    .unwrap();
    // The inner cube now spans x 12..14 (scaled by 2, shifted 10).
    let other = boxed(&mut d, [0.0, 2.0, 2.0], [1.0, 1.0, 1.0]);
    let r = measure(&d, json!({"distance": [[outer, inner], other]})).unwrap();
    assert!(near(&r["distance"]["distance"], 11.0), "{r}");
    // A face inside the nested group: found by path.
    let rows = {
        run(&mut d, json!({"type": "open_instance", "id": outer})).unwrap();
        run(&mut d, json!({"type": "open_instance", "id": inner})).unwrap();
        let f = entities(&d)["faces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["normal"] == json!([-1.0, 0.0, 0.0]))
            .unwrap()["id"]
            .clone();
        run(&mut d, json!({"type": "close_instance"})).unwrap();
        run(&mut d, json!({"type": "close_instance"})).unwrap();
        f
    };
    let r = measure(&d, json!({"distance": [[outer, inner, rows], other]})).unwrap();
    assert!(near(&r["distance"]["distance"], 11.0), "{r}");
    assert!(near(&r["distance"]["points"][0][0], 12.0));
    // A path through something that is not an instance is invalid.
    let e = measure(&d, json!({"distance": [[rows, inner], other]})).unwrap_err();
    assert!(matches!(e.0, ErrorKind::InvalidInput | ErrorKind::NotFound));
}

#[test]
fn angles_between_edges_and_faces() {
    let mut d = doc();
    let e1 = edge(&mut d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
    let e2 = edge(&mut d, [0.0, 0.0, 0.0], [0.0, 3.0, 0.0]);
    let e3 = edge(&mut d, [5.0, 1.0, 0.0], [4.0, 2.0, 0.0]);
    let r = measure(&d, json!({"angle": [e1, e2]})).unwrap();
    assert!(near(&r["angle"]["degrees"], 90.0), "{r}");
    let r = measure(&d, json!({"angle": [e1, e3]})).unwrap();
    assert!(near(&r["angle"]["degrees"], 135.0), "{r}");
    assert!(near(&r["angle"]["acute"], 45.0));
    // Faces of a box.
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1], "as": "geometry"}),
    )
    .unwrap();
    let faces = entities(&d)["faces"].as_array().unwrap().clone();
    let by = |n: [f64; 3]| faces.iter().find(|f| f["normal"] == json!(n)).unwrap()["id"].clone();
    let (top, bottom, side) = (
        by([0.0, 0.0, 1.0]),
        by([0.0, 0.0, -1.0]),
        by([1.0, 0.0, 0.0]),
    );
    let r = measure(&d, json!({"angle": [top, side]})).unwrap();
    assert!(near(&r["angle"]["degrees"], 90.0));
    let r = measure(&d, json!({"angle": [top, bottom]})).unwrap();
    assert!(near(&r["angle"]["degrees"], 180.0));
    assert!(near(&r["angle"]["acute"], 0.0));
    // Mixing kinds is invalid.
    let e1 = edge(&mut d, [5.0, 5.0, 5.0], [6.0, 5.0, 5.0]);
    let e = measure(&d, json!({"angle": [top, e1]})).unwrap_err();
    assert!(matches!(e.0, ErrorKind::InvalidInput | ErrorKind::NotFound));
}

fn cylinder(d: &mut Document, extra: Value) {
    let mut c = json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1.5, "height": 2,
                       "segments": 32, "as": "geometry"});
    for (k, v) in extra.as_object().unwrap() {
        c[k] = v.clone();
    }
    run(d, c).unwrap();
}

#[test]
fn a_cylinders_side_cap_and_rim_are_fitted() {
    let mut d = doc();
    cylinder(&mut d, json!({}));
    let ents = entities(&d);
    let side = ents["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"][2].as_f64().unwrap().abs() < 1e-9)
        .unwrap()["id"]
        .clone();
    let r = measure(&d, json!({"round": side})).unwrap();
    let round = &r["round"];
    assert_eq!(round["kind"], "cylinder", "{r}");
    assert!(near(&round["radius"], 1.5));
    assert!(near(&round["length"], 2.0));
    assert!(point_near(&round["axis"], [0.0, 0.0, 1.0]));
    assert!(point_near(&round["center"], [0.0, 0.0, 1.0]));
    assert!(round["max_deviation"].as_f64().unwrap() < 1e-9);
    // The cap is a circle.
    let cap = ents["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"] == json!([0.0, 0.0, 1.0]))
        .unwrap()["id"]
        .clone();
    let r = measure(&d, json!({"round": cap})).unwrap();
    assert_eq!(r["round"]["kind"], "circle");
    assert!(near(&r["round"]["radius"], 1.5));
    assert!(point_near(&r["round"]["center"], [0.0, 0.0, 2.0]));
    assert!(r["round"].get("length").is_none());
    // So is a rim edge.
    let rim = ents["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e.get("curve").is_some() && e["start"][2] == json!(0.0))
        .unwrap()["id"]
        .clone();
    let r = measure(&d, json!({"round": rim})).unwrap();
    assert_eq!(r["round"]["kind"], "circle");
    assert!(near(&r["round"]["radius"], 1.5));
    assert!(point_near(&r["round"]["center"], [0.0, 0.0, 0.0]));
    assert!(point_near(&r["round"]["axis"], [0.0, 0.0, 1.0]));
}

#[test]
fn round_fits_follow_the_placement_and_report_faceting() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1,
               "height": 3, "segments": 16, "as": "group"}),
    )
    .unwrap();
    let g = entities(&d)["instances"][0]["id"].clone();
    // Local z becomes world x; the whole thing is scaled by 2 and moved.
    run(
        &mut d,
        json!({"type": "set_transform", "id": g,
               "transform": {"matrix": [0, 2, 0, 0, 0, 2, 2, 0, 0, 0, 5, 0]}}),
    )
    .unwrap();
    run(&mut d, json!({"type": "open_instance", "id": g})).unwrap();
    let side = entities(&d)["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"][2].as_f64().unwrap().abs() < 1e-9)
        .unwrap()["id"]
        .clone();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let r = measure(&d, json!({"round": [g, side]})).unwrap();
    assert_eq!(r["round"]["kind"], "cylinder", "{r}");
    assert!(near(&r["round"]["radius"], 2.0), "{r}");
    assert!(near(&r["round"]["length"], 6.0));
    assert!(point_near(&r["round"]["axis"], [1.0, 0.0, 0.0]));
    assert!(point_near(&r["round"]["center"], [3.0, 5.0, 0.0]));
    assert!(r["round"]["max_deviation"].as_f64().unwrap() < 1e-9);
    // Non-uniform scale makes the surface elliptical: the deviation says so.
    run(
        &mut d,
        json!({"type": "set_transform", "id": g,
               "transform": {"matrix": [3, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]}}),
    )
    .unwrap();
    let r = measure(&d, json!({"round": [g, side]})).unwrap();
    assert!(r["round"]["max_deviation"].as_f64().unwrap() > 0.1, "{r}");
}

#[test]
fn flat_and_spherical_faces_are_not_round() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_box", "corner": [0, 0, 0], "size": [1, 1, 1], "as": "geometry"}),
    )
    .unwrap();
    let f = entities(&d)["faces"][0]["id"].clone();
    let e = measure(&d, json!({"round": f})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput, "{}", e.1);
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_sphere", "center": [0, 0, 0], "radius": 1, "segments": 16,
               "as": "geometry"}),
    )
    .unwrap();
    let f = entities(&d)["faces"][5]["id"].clone();
    let e = measure(&d, json!({"round": f})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput, "{}", e.1);
}

#[test]
fn bad_requests_are_rejected() {
    let mut d = doc();
    let a = boxed(&mut d, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    for q in [
        json!({}),
        json!({"distance": [a, a], "angle": [a, a]}),
        json!({"distance": [a, a], "round": a}),
    ] {
        let e = measure(&d, q).unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput);
    }
    let e = measure(&d, json!({"distance": [a, "f99"]})).unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    let e = measure(&d, json!({"distance": [a, "i99"]})).unwrap_err();
    assert_eq!(e.0, ErrorKind::NotFound);
    let e = measure(&d, json!({"distance": [a, "r1"]})).unwrap_err();
    assert!(matches!(e.0, ErrorKind::InvalidInput | ErrorKind::NotFound));
    let e = measure(&d, json!({"distance": [a, []]})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
}

#[test]
fn a_bore_in_a_subtracted_plate_is_a_cylinder() {
    let mut d = doc();
    let plate = boxed(&mut d, [0.0, 0.0, 0.0], [4.0, 4.0, 1.0]);
    let tool = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [2, 2, -1], "radius": 0.75, "height": 3,
               "segments": 48, "as": "group"}),
    )
    .unwrap()["placed"][0]
        .clone();
    let out = run(
        &mut d,
        json!({"type": "solid_subtract", "target": plate, "tools": [tool]}),
    )
    .unwrap();
    let part = out["placed"][0].clone();
    run(&mut d, json!({"type": "open_instance", "id": part})).unwrap();
    let ents = entities(&d);
    let wall = ents["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| {
            f["normal"][2].as_f64().unwrap().abs() < 1e-9 && f["area"].as_f64().unwrap() < 1.0
        })
        .expect("a bore wall")["id"]
        .clone();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let r = measure(&d, json!({"round": [part, wall]})).unwrap();
    assert_eq!(r["round"]["kind"], "cylinder", "{r}");
    assert!(near(&r["round"]["radius"], 0.75), "{r}");
    assert!(near(&r["round"]["length"], 1.0), "{r}");
    assert!(point_near(&r["round"]["center"], [2.0, 2.0, 0.5]), "{r}");
    assert!(r["round"]["max_deviation"].as_f64().unwrap() < 1e-9);
}

#[test]
fn a_split_circle_edge_does_not_bias_the_fit() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [3, -2, 0], "radius": 1, "segments": 12}),
    )
    .unwrap();
    let ents = entities(&d);
    let first = ents["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e.get("curve").is_some())
        .unwrap()
        .clone();
    let (a, b) = (&first["start"], &first["end"]);
    let mid: Vec<f64> = (0..3)
        .map(|i| (a[i].as_f64().unwrap() + b[i].as_f64().unwrap()) / 2.0)
        .collect();
    // An edge from the middle of a side outward splits that side in two.
    let out = [mid[0] * 2.0 - 3.0, mid[1] * 2.0 + 2.0, 0.0];
    edge(&mut d, [mid[0], mid[1], mid[2]], out);
    let ents = entities(&d);
    let rim: Vec<&Value> = ents["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e.get("curve").is_some())
        .collect();
    assert_eq!(rim.len(), 13, "the split side is two edges");
    for e in rim {
        let r = measure(&d, json!({"round": e["id"]})).unwrap();
        assert_eq!(r["round"]["kind"], "circle");
        assert!(near(&r["round"]["radius"], 1.0), "{r}");
        assert!(point_near(&r["round"]["center"], [3.0, -2.0, 0.0]), "{r}");
        assert_eq!(r["round"]["samples"], 12, "{r}");
    }
}

#[test]
fn a_cross_bore_does_not_bias_the_cylinder_fit() {
    let mut d = doc();
    let shaft = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [0, 0, 0], "radius": 1, "height": 4,
               "segments": 24, "as": "group"}),
    )
    .unwrap()["placed"][0]
        .clone();
    let tool = run(
        &mut d,
        json!({"type": "add_cylinder", "base": [-2, 0, 2], "axis": [1, 0, 0], "radius": 0.3,
               "height": 4, "segments": 16, "as": "group"}),
    )
    .unwrap()["placed"][0]
        .clone();
    let out = run(
        &mut d,
        json!({"type": "solid_subtract", "target": shaft, "tools": [tool]}),
    )
    .unwrap();
    let part = out["placed"][0].clone();
    run(&mut d, json!({"type": "open_instance", "id": part})).unwrap();
    let ents = entities(&d);
    // A facet of the shaft's side facing along y, away from the bore.
    let side = ents["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["normal"][1].as_f64().unwrap() > 0.99)
        .expect("a side facet")["id"]
        .clone();
    run(&mut d, json!({"type": "close_instance"})).unwrap();
    let r = measure(&d, json!({"round": [part, side]})).unwrap();
    assert_eq!(r["round"]["kind"], "cylinder", "{r}");
    assert!(near(&r["round"]["radius"], 1.0), "{r}");
    assert!(near(&r["round"]["length"], 4.0), "{r}");
    assert!(point_near(&r["round"]["center"], [0.0, 0.0, 2.0]), "{r}");
    assert!(r["round"]["max_deviation"].as_f64().unwrap() < 1e-9, "{r}");
}
